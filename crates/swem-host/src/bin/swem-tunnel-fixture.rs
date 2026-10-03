//! A tunnel that goes nowhere: the shape of `swem/tunnel@1`, answering
//! `open` with the very URL it is handed, so what a test opens "through the
//! tunnel" is the harness's own gate on loopback. A fixture: it stands in
//! for a tunnel vendor where the harness proves that a tunnel is opened,
//! used, and closed. Every call is written to `<home>/calls.jsonl`, one
//! JSON line each, so a test reads what the harness asked.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use swem_sdk::tunnel::Standing;

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Nothing {}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct OpenParams {
    url: String,
}

struct Tunnel {
    calls: PathBuf,
    standing: Mutex<Standing>,
    tool_router: ToolRouter<Self>,
}

impl Tunnel {
    fn record(&self, tool: &str, params: &Value) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.calls)
        {
            let _ = writeln!(file, "{}", json!({ "tool": tool, "params": params }));
        }
    }
}

#[tool_router]
impl Tunnel {
    #[tool(description = "Stand at an address from outside for a local URL")]
    async fn open(
        &self,
        Parameters(params): Parameters<OpenParams>,
    ) -> Result<Json<Standing>, String> {
        self.record("open", &json!({ "url": params.url }));
        let standing = Standing {
            origin: params.url.trim_end_matches('/').to_owned(),
            url: params.url,
            said: "a fixture: the address is the URL itself".into(),
        };
        *self
            .standing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = standing.clone();
        Ok(Json(standing))
    }

    #[tool(description = "Leave the address")]
    async fn close(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Standing>, String> {
        self.record("close", &json!({}));
        *self
            .standing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Standing::default();
        Ok(Json(Standing::default()))
    }

    #[tool(description = "Where the tunnel stands, if anywhere")]
    async fn look(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Standing>, String> {
        self.record("look", &json!({}));
        Ok(Json(
            self.standing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tunnel {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-tunnel-fixture",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("A tunnel that goes nowhere; a fixture")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let home = arguments
        .windows(2)
        .find(|pair| pair[0] == "--home")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| std::env::var_os(swem_sdk::tunnel::HOME_VARIABLE).map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&home)?;
    Tunnel {
        calls: home.join("calls.jsonl"),
        standing: Mutex::new(Standing::default()),
        tool_router: Tunnel::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}
