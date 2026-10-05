//! A channel that is nobody's messenger: the shape of `swem/channel@1`
//! over two directories. What a test drops into the inbox as JSON events is
//! what `pull` answers; every tool the harness calls is written to the
//! outbox, one JSON line each, so a test reads what the harness sent. A
//! fixture: it stands in for a messenger where the harness proves that a
//! channel carries a chat both ways.
//!
//! The directories come from the environment the harness gives a channel
//! (`SWEM_CHANNEL_HOME`), or from `--home`; the inbox is `<home>/inbox`,
//! the outbox `<home>/outbox/calls.jsonl`.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Nothing {}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct PullParams {
    wait_s: u64,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct SendParams {
    chat: String,
    markdown: String,
    #[serde(default)]
    reply_to: Option<String>,
    /// A button to a page inside the messenger, when the harness sends one.
    #[serde(default)]
    app: Option<Value>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct StreamBeginParams {
    chat: String,
    stream: String,
    #[serde(default)]
    reply_to: Option<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct StreamParams {
    chat: String,
    stream: String,
    markdown: String,
    #[serde(default)]
    stopped: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct AskParams {
    chat: String,
    question: String,
    title: String,
    options: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct SendFileParams {
    chat: String,
    path: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct FetchFileParams {
    file: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct VerifyAppParams {
    init_data: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct TakeBackParams {
    chat: String,
    reference: String,
    markdown: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct Sent {
    reference: String,
}

/// HMAC-SHA256, for the fixture's own way of signing who opened its page.
fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    let mut block = [0_u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&sha2::Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner: Vec<u8> = block.iter().map(|byte| byte ^ 0x36).collect();
    let outer: Vec<u8> = block.iter().map(|byte| byte ^ 0x5c).collect();
    let mut hasher = sha2::Sha256::new();
    hasher.update(&inner);
    hasher.update(message);
    let inner_hash = hasher.finalize();
    let mut hasher = sha2::Sha256::new();
    hasher.update(&outer);
    hasher.update(inner_hash);
    hasher.finalize().to_vec()
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

#[derive(Debug)]
struct Channel {
    inbox: PathBuf,
    outbox: PathBuf,
    sent: Mutex<u64>,
    /// The bot's key, as the harness hands it: what the fixture's page
    /// data is signed with.
    key: String,
    tool_router: ToolRouter<Self>,
}

impl Channel {
    fn record(&self, tool: &str, arguments: &Value) -> Result<Sent, String> {
        let mut sent = self.sent.lock().map_err(|_| "poisoned")?;
        *sent += 1;
        let reference = format!("sent-{}", *sent);
        let line = json!({ "tool": tool, "arguments": arguments, "reference": reference });
        fs::create_dir_all(self.outbox.parent().unwrap_or(&self.outbox))
            .map_err(|error| error.to_string())?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.outbox)
            .map_err(|error| format!("open {}: {error}", self.outbox.display()))?;
        writeln!(file, "{line}").map_err(|error| error.to_string())?;
        Ok(Sent { reference })
    }

    /// Everything dropped into the inbox, in name order, taken out.
    fn take_inbox(&self) -> Vec<Value> {
        let Ok(entries) = fs::read_dir(&self.inbox) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .collect();
        files.sort();
        let mut events = Vec::new();
        for file in files {
            if let Ok(bytes) = fs::read(&file)
                && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
            {
                match value {
                    Value::Array(several) => events.extend(several),
                    one => events.push(one),
                }
            }
            let _ = fs::remove_file(&file);
        }
        events
    }
}

#[tool_router]
impl Channel {
    #[tool(description = "Who the bot is")]
    fn look(&self, Parameters(Nothing {}): Parameters<Nothing>) -> Json<Value> {
        // The outbox records the look like every other call.
        let _ = self.record("look", &json!({}));
        Json(json!({
            "id": "bot-1",
            "username": "fixture_bot",
            "name": "Fixture",
            "app_data_fragment": "appData"
        }))
    }

    #[tool(description = "Take back a button the bot sent")]
    fn take_back(
        &self,
        Parameters(params): Parameters<TakeBackParams>,
    ) -> Result<Json<Sent>, String> {
        self.record(
            "take_back",
            &json!({ "chat": params.chat, "reference": params.reference, "markdown": params.markdown }),
        )
        .map(Json)
    }

    /// The fixture's page data is `user=<json>&hash=<hex>`, the hash an
    /// HMAC-SHA256 of `user=<json>` under the bot's key - the shape of a
    /// messenger's signed data, and nobody's messenger.
    #[tool(description = "Who opened the page inside the messenger, from signed data")]
    fn verify_app(
        &self,
        Parameters(params): Parameters<VerifyAppParams>,
    ) -> Result<Json<Value>, String> {
        let mut user = None;
        let mut hash = None;
        for pair in params.init_data.split('&') {
            match pair.split_once('=') {
                Some(("user", value)) => user = Some(value),
                Some(("hash", value)) => hash = Some(value),
                _ => {}
            }
        }
        let (Some(user), Some(hash)) = (user, hash) else {
            return Err("the data names no user, or carries no hash".into());
        };
        let expected = hex(&hmac_sha256(
            self.key.as_bytes(),
            format!("user={user}").as_bytes(),
        ));
        if expected != hash {
            return Err("the data is not signed for this bot".into());
        }
        let person: Value = serde_json::from_str(user).map_err(|error| error.to_string())?;
        self.record("verify_app", &json!({ "id": person["id"] }))?;
        Ok(Json(json!({
            "id": person["id"].as_str().map_or_else(|| person["id"].to_string(), str::to_owned),
            "name": person["name"].as_str().unwrap_or_default(),
            "username": person["username"].as_str().unwrap_or_default(),
        })))
    }

    #[tool(description = "The next inbound events, waiting up to wait_s for one")]
    async fn pull(
        &self,
        Parameters(PullParams { wait_s }): Parameters<PullParams>,
    ) -> Result<Json<Value>, String> {
        let deadline = Instant::now() + Duration::from_secs(wait_s.min(30));
        loop {
            let events = self.take_inbox();
            if !events.is_empty() || Instant::now() >= deadline {
                return Ok(Json(json!({ "events": events })));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    #[tool(description = "One message to a chat")]
    fn send(&self, Parameters(params): Parameters<SendParams>) -> Result<Json<Sent>, String> {
        self.record(
            "send",
            &json!({ "chat": params.chat, "markdown": params.markdown, "reply_to": params.reply_to, "app": params.app }),
        )
        .map(Json)
    }

    #[tool(description = "A turn begins to be written")]
    fn stream_begin(
        &self,
        Parameters(params): Parameters<StreamBeginParams>,
    ) -> Result<Json<Sent>, String> {
        self.record(
            "stream_begin",
            &json!({ "chat": params.chat, "stream": params.stream, "reply_to": params.reply_to }),
        )
        .map(Json)
    }

    #[tool(description = "What was written so far")]
    fn stream_update(
        &self,
        Parameters(params): Parameters<StreamParams>,
    ) -> Result<Json<Sent>, String> {
        self.record(
            "stream_update",
            &json!({ "chat": params.chat, "stream": params.stream, "markdown": params.markdown }),
        )
        .map(Json)
    }

    #[tool(description = "The turn is written, or was stopped")]
    fn stream_end(
        &self,
        Parameters(params): Parameters<StreamParams>,
    ) -> Result<Json<Sent>, String> {
        self.record(
            "stream_end",
            &json!({ "chat": params.chat, "stream": params.stream, "markdown": params.markdown, "stopped": params.stopped }),
        )
        .map(Json)
    }

    #[tool(description = "A question with options")]
    fn ask(&self, Parameters(params): Parameters<AskParams>) -> Result<Json<Sent>, String> {
        self.record(
            "ask",
            &json!({ "chat": params.chat, "question": params.question, "title": params.title, "options": params.options }),
        )
        .map(Json)
    }

    #[tool(description = "A file to a chat")]
    fn send_file(
        &self,
        Parameters(params): Parameters<SendFileParams>,
    ) -> Result<Json<Sent>, String> {
        self.record(
            "send_file",
            &json!({ "chat": params.chat, "path": params.path, "name": params.name }),
        )
        .map(Json)
    }

    #[tool(description = "A file that came with a message, fetched into the channel's home")]
    fn fetch_file(
        &self,
        Parameters(params): Parameters<FetchFileParams>,
    ) -> Result<Json<Value>, String> {
        // The fixture's files are the inbox's neighbours: `file` names a
        // file under `<home>/files`.
        let home = self.inbox.parent().unwrap_or(&self.inbox).to_path_buf();
        let path = home.join("files").join(&params.file);
        if !path.is_file() {
            return Err(format!("no file {}", params.file));
        }
        self.record("fetch_file", &json!({ "file": params.file }))?;
        Ok(Json(
            json!({ "path": path.display().to_string(), "name": params.file }),
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Channel {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-channel-fixture",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("A channel over two directories; a fixture")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let home = arguments
        .windows(2)
        .find(|pair| pair[0] == "--home")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| std::env::var_os("SWEM_CHANNEL_HOME").map(PathBuf::from))
        // Started by nobody in particular (a shape check), it lives in the
        // temporary directory.
        .unwrap_or_else(|| std::env::temp_dir().join("swem-channel-fixture"));
    let inbox = home.join("inbox");
    fs::create_dir_all(&inbox)?;
    fs::create_dir_all(home.join("outbox"))?;
    Channel {
        inbox,
        outbox: home.join("outbox").join("calls.jsonl"),
        sent: Mutex::new(0),
        key: std::env::var("SWEM_CHANNEL_KEY").unwrap_or_default(),
        tool_router: Channel::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}
