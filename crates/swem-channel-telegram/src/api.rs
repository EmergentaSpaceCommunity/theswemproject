//! The Telegram Bot API, as much of it as a channel needs.
//!
//! One client over one bot token against one API root - the public one, a
//! local Bot API server, or a fixture. Every method is `POST /bot<token>/<method>`
//! with a JSON body and answers `{ok, result | description}`; a file is
//! fetched from `/file/bot<token>/<path>`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};

/// The public Bot API.
pub const PUBLIC_API_ROOT: &str = "https://api.telegram.org";

/// What the public Bot API lets a bot fetch, in bytes.
pub const FETCH_LIMIT: u64 = 20 * 1024 * 1024;
/// What the public Bot API lets a bot send, in bytes.
pub const SEND_LIMIT: u64 = 50 * 1024 * 1024;
/// The longest text one message carries.
pub const TEXT_LIMIT: usize = 4096;

#[derive(Clone)]
pub struct Api {
    client: reqwest::Client,
    root: String,
    token: String,
}

#[derive(Debug, Deserialize)]
struct Answer {
    ok: bool,
    #[serde(default)]
    result: Value,
    #[serde(default)]
    description: String,
}

impl Api {
    /// A client for the bot `token` at `root` (no trailing slash).
    ///
    /// # Errors
    ///
    /// The HTTP client could not be built.
    pub fn new(root: &str, token: &str) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(70))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client,
            root: root.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
        })
    }

    fn method_url(&self, method: &str) -> String {
        format!("{}/bot{}/{method}", self.root, self.token)
    }

    /// Call a method with a JSON body and answer its result.
    ///
    /// # Errors
    ///
    /// The call failed, or Telegram said `ok: false` (with its words).
    pub async fn call(&self, method: &str, body: Value) -> Result<Value, String> {
        let response = self
            .client
            .post(self.method_url(method))
            .json(&body)
            .send()
            .await
            .map_err(|error| format!("{method}: {error}"))?;
        let status = response.status();
        let answer: Answer = response
            .json()
            .await
            .map_err(|error| format!("{method}: not a Bot API answer ({status}): {error}"))?;
        if !answer.ok {
            return Err(format!("{method}: {}", answer.description));
        }
        Ok(answer.result)
    }

    /// Upload a file as a document.
    ///
    /// # Errors
    ///
    /// The file cannot be read, is over the limit, or the call failed.
    pub async fn send_document(
        &self,
        chat_id: &str,
        thread: Option<i64>,
        path: &Path,
        name: &str,
    ) -> Result<Value, String> {
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if bytes.len() as u64 > SEND_LIMIT {
            return Err(format!(
                "too_large: the Bot API takes files up to {SEND_LIMIT} bytes"
            ));
        }
        let part = reqwest::multipart::Part::bytes(bytes).file_name(name.to_owned());
        let mut form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_owned())
            .part("document", part);
        if let Some(thread) = thread {
            form = form.text("message_thread_id", thread.to_string());
        }
        let response = self
            .client
            .post(self.method_url("sendDocument"))
            .multipart(form)
            .send()
            .await
            .map_err(|error| format!("sendDocument: {error}"))?;
        let answer: Answer = response
            .json()
            .await
            .map_err(|error| format!("sendDocument: not a Bot API answer: {error}"))?;
        if !answer.ok {
            return Err(format!("sendDocument: {}", answer.description));
        }
        Ok(answer.result)
    }

    /// Fetch a file by its id into `into`, named `name`.
    ///
    /// # Errors
    ///
    /// The file is over the limit, unknown, or cannot be written.
    pub async fn fetch_file(
        &self,
        file_id: &str,
        into: &Path,
        name: &str,
    ) -> Result<PathBuf, String> {
        let file = self.call("getFile", json!({ "file_id": file_id })).await?;
        if let Some(size) = file.get("file_size").and_then(Value::as_u64)
            && size > FETCH_LIMIT
        {
            return Err(format!(
                "too_large: the Bot API gives files up to {FETCH_LIMIT} bytes"
            ));
        }
        let file_path = file
            .get("file_path")
            .and_then(Value::as_str)
            .ok_or_else(|| "getFile: no file_path".to_owned())?;
        let url = format!("{}/file/bot{}/{file_path}", self.root, self.token);
        let bytes = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|error| format!("fetch: {error}"))?
            .bytes()
            .await
            .map_err(|error| format!("fetch: {error}"))?;
        tokio::fs::create_dir_all(into)
            .await
            .map_err(|error| format!("make {}: {error}", into.display()))?;
        let safe = name
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let safe = if safe.is_empty() {
            "file".to_owned()
        } else {
            safe
        };
        let path = into.join(format!("{}-{safe}", short(file_id)));
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|error| format!("write {}: {error}", path.display()))?;
        Ok(path)
    }
}

/// The first few characters of an id, for a file name.
fn short(id: &str) -> String {
    id.chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect()
}

/// A chat on Telegram's side, as the channel spells it to the harness:
/// `<chat_id>` or `<chat_id>:<thread_id>` for a forum topic.
#[must_use]
pub fn chat_ref(chat_id: i64, thread: Option<i64>) -> String {
    match thread {
        Some(thread) => format!("{chat_id}:{thread}"),
        None => chat_id.to_string(),
    }
}

/// The two halves of a spelled chat.
#[must_use]
pub fn chat_parts(spelled: &str) -> (String, Option<i64>) {
    match spelled.split_once(':') {
        Some((chat, thread)) => (chat.to_owned(), thread.parse().ok()),
        None => (spelled.to_owned(), None),
    }
}
