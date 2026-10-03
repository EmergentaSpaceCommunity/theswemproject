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

/// HMAC-SHA256, as Telegram signs a Mini App's `initData`: the key is
/// `HMAC_SHA256(key = "WebAppData", message = bot token)`, the signed text
/// every field but `hash`, sorted, one per line as `name=value`.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    const BLOCK: usize = 64;
    let mut key_block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let inner: Vec<u8> = key_block.iter().map(|byte| byte ^ 0x36).collect();
    let outer: Vec<u8> = key_block.iter().map(|byte| byte ^ 0x5c).collect();
    let mut hasher = Sha256::new();
    hasher.update(&inner);
    hasher.update(message);
    let inner_hash = hasher.finalize();
    let mut hasher = Sha256::new();
    hasher.update(&outer);
    hasher.update(inner_hash);
    hasher.finalize().into()
}

/// Bytes as lower-case hex.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// One field of a query string, decoded.
fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' if at + 2 < bytes.len() => {
                let hex = &text[at + 1..at + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    out.push(byte);
                    at += 2;
                } else {
                    out.push(b'%');
                }
            }
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl Api {
    /// The user behind a Mini App's `initData`, when the messenger signed
    /// it for this bot and not longer than a day ago.
    ///
    /// # Errors
    ///
    /// The data is not signed for this bot, is too old, or names no user.
    pub fn verify_init_data(&self, init_data: &str) -> Result<Value, String> {
        let mut fields: Vec<(String, String)> = init_data
            .split('&')
            .filter(|pair| !pair.is_empty())
            .map(|pair| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (decoded(name), decoded(value))
            })
            .collect();
        let hash = fields
            .iter()
            .find(|(name, _)| name == "hash")
            .map(|(_, value)| value.clone())
            .ok_or("initData carries no hash")?;
        fields.retain(|(name, _)| name != "hash");
        fields.sort();
        let signed = fields
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        let secret = hmac_sha256(b"WebAppData", self.token.as_bytes());
        let expected = hex(&hmac_sha256(&secret, signed.as_bytes()));
        if expected != hash.to_ascii_lowercase() {
            return Err("initData is not signed for this bot".to_owned());
        }
        let auth_date: u64 = fields
            .iter()
            .find(|(name, _)| name == "auth_date")
            .and_then(|(_, value)| value.parse().ok())
            .ok_or("initData carries no auth_date")?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or_default();
        if now.saturating_sub(auth_date) > 24 * 60 * 60 {
            return Err("initData is older than a day".to_owned());
        }
        let user = fields
            .iter()
            .find(|(name, _)| name == "user")
            .and_then(|(_, value)| serde_json::from_str::<Value>(value).ok())
            .ok_or("initData names no user")?;
        Ok(user)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_data_is_verified_the_way_telegram_signs_it() {
        let api = Api::new("http://127.0.0.1:1", "123456:token").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let user = r#"{"id":7,"first_name":"Ada","username":"ada"}"#;
        let signed = format!("auth_date={now}\nuser={user}");
        let secret = hmac_sha256(b"WebAppData", b"123456:token");
        let hash = hex(&hmac_sha256(&secret, signed.as_bytes()));
        let encoded_user = user
            .replace('{', "%7B")
            .replace('}', "%7D")
            .replace('"', "%22")
            .replace(':', "%3A")
            .replace(',', "%2C");
        let init_data = format!("user={encoded_user}&auth_date={now}&hash={hash}");
        let found = api.verify_init_data(&init_data).unwrap();
        assert_eq!(found["id"], 7);
        assert!(
            api.verify_init_data(&init_data.replace(&hash, "00"))
                .is_err()
        );
        assert!(
            api.verify_init_data(&format!("user={encoded_user}&auth_date=1&hash={hash}"))
                .is_err()
        );
    }

    #[test]
    fn a_query_value_is_decoded() {
        assert_eq!(decoded("a%20b+c%7B"), "a b c{");
        assert_eq!(decoded("%zz"), "%zz");
    }
}
