//! Byte-transparent observation of one already resolved MCP stdio attachment.
//!
//! This is not an MCP router. It never chooses a server, renames a tool or
//! answers an MCP request. The original JSON-RPC lines cross unchanged while a
//! best-effort side channel reports exact `tools/call` request/response pairs.
//! Losing that side channel must not take native tool access away from the
//! agent.

use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::tcp::OwnedWriteHalf;

pub(crate) const OBSERVER_ENDPOINT_ENV: &str = "SWEM_MCP_OBSERVER_ENDPOINT";
pub(crate) const OBSERVER_TOKEN_ENV: &str = "SWEM_MCP_OBSERVER_TOKEN";
pub(crate) const OBSERVER_SERVER_ENV: &str = "SWEM_MCP_OBSERVER_SERVER";
pub(crate) const OBSERVER_PROTOCOL: &str = "swem.mcp-observer/0.1";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ObserverHello {
    pub(crate) protocol: String,
    pub(crate) token: String,
    pub(crate) server: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct McpObservationEvent {
    pub(crate) phase: String,
    pub(crate) server: String,
    pub(crate) request_id: Value,
    pub(crate) tool: String,
    pub(crate) arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cancellation_reason: Option<String>,
    pub(crate) wire_sha256: String,
}

pub struct StdioObserverConfig {
    pub server_name: String,
    pub command: PathBuf,
    pub command_args: Vec<String>,
}

enum ObservationSink {
    File(File),
    Ephemeral(OwnedWriteHalf),
    Disabled,
}

impl ObservationSink {
    async fn append(&mut self, value: &McpObservationEvent) -> Result<(), String> {
        match self {
            Self::File(file) => {
                serde_json::to_writer(&mut *file, value).map_err(|error| error.to_string())?;
                file.write_all(b"\n").map_err(|error| error.to_string())?;
                file.flush().map_err(|error| error.to_string())
            }
            Self::Ephemeral(stream) => {
                let mut bytes = serde_json::to_vec(value).map_err(|error| error.to_string())?;
                bytes.push(b'\n');
                if stream.write_all(&bytes).await.is_err() || stream.flush().await.is_err() {
                    // GenUI observation is optional. A dead Workbench must not
                    // remove the native MCP capability from the agent.
                    *self = Self::Disabled;
                }
                Ok(())
            }
            Self::Disabled => Ok(()),
        }
    }
}

fn request_key(id: &Value) -> Option<String> {
    if id.is_string() || matches!(id, Value::Number(number) if number.is_i64() || number.is_u64()) {
        serde_json::to_string(id).ok()
    } else {
        None
    }
}

fn is_response(message: &Value) -> bool {
    message.get("method").is_none()
        && (message.get("result").is_some() || message.get("error").is_some())
}

fn wire_sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn create_evidence(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| format!("open evidence: {error}"))
}

async fn ephemeral_sink() -> ObservationSink {
    let (Ok(endpoint), Ok(token), Ok(server)) = (
        std::env::var(OBSERVER_ENDPOINT_ENV),
        std::env::var(OBSERVER_TOKEN_ENV),
        std::env::var(OBSERVER_SERVER_ENV),
    ) else {
        return ObservationSink::Disabled;
    };
    let Ok(address) = endpoint.parse::<std::net::SocketAddr>() else {
        return ObservationSink::Disabled;
    };
    if !address.ip().is_loopback() {
        return ObservationSink::Disabled;
    }
    let Ok(Ok(stream)) = tokio::time::timeout(
        Duration::from_secs(2),
        tokio::net::TcpStream::connect(address),
    )
    .await
    else {
        return ObservationSink::Disabled;
    };
    let (_read, mut write) = stream.into_split();
    let hello = ObserverHello {
        protocol: OBSERVER_PROTOCOL.to_owned(),
        token,
        server,
    };
    let Ok(mut bytes) = serde_json::to_vec(&hello) else {
        return ObservationSink::Disabled;
    };
    bytes.push(b'\n');
    if write.write_all(&bytes).await.is_err() || write.flush().await.is_err() {
        ObservationSink::Disabled
    } else {
        ObservationSink::Ephemeral(write)
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one duplex relay loop keeps byte forwarding and request/response observation ordering adjacent"
)]
async fn observe(config: StdioObserverConfig, mut sink: ObservationSink) -> Result<(), String> {
    let mut child = tokio::process::Command::new(&config.command)
        .args(&config.command_args)
        .env_remove(OBSERVER_ENDPOINT_ENV)
        .env_remove(OBSERVER_TOKEN_ENV)
        .env_remove(OBSERVER_SERVER_ENV)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|error| format!("spawn observed MCP server: {error}"))?;
    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| "observed MCP server has no stdin".to_owned())?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| "observed MCP server has no stdout".to_owned())?;
    let mut client = BufReader::new(tokio::io::stdin());
    let mut server = BufReader::new(child_stdout);
    let mut child_stdin = child_stdin;
    let mut client_stdout = tokio::io::stdout();
    let mut client_open = true;
    let mut pending = BTreeMap::<String, (Value, String, Value)>::new();

    loop {
        let mut client_line = Vec::new();
        let mut server_line = Vec::new();
        tokio::select! {
            read = client.read_until(b'\n', &mut client_line), if client_open => {
                let read = read.map_err(|error| format!("read client MCP: {error}"))?;
                if read == 0 {
                    client_open = false;
                    child_stdin.shutdown().await.map_err(|error| error.to_string())?;
                    continue;
                }
                if let Ok(message) = serde_json::from_slice::<Value>(&client_line)
                    && message["method"] == "tools/call"
                    && let Some(id) = message.get("id")
                    && let Some(key) = request_key(id)
                    && let Some(tool) = message["params"]["name"].as_str()
                {
                    let arguments = message["params"]
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    pending.insert(key, (id.clone(), tool.to_owned(), arguments.clone()));
                    sink.append(&McpObservationEvent {
                        phase: "request".into(),
                        server: config.server_name.clone(),
                        request_id: id.clone(),
                        tool: tool.to_owned(),
                        arguments,
                        result: None,
                        error: None,
                        cancellation_reason: None,
                        wire_sha256: wire_sha256(&client_line),
                    }).await?;
                } else if let Ok(message) = serde_json::from_slice::<Value>(&client_line)
                    && message["method"] == "notifications/cancelled"
                    && let Some(id) = message["params"].get("requestId")
                    && let Some(key) = request_key(id)
                    && let Some((request_id, tool, arguments)) = pending.remove(&key)
                {
                    sink.append(&McpObservationEvent {
                        phase: "cancelled".into(),
                        server: config.server_name.clone(),
                        request_id,
                        tool,
                        arguments,
                        result: None,
                        error: None,
                        cancellation_reason: message["params"]["reason"]
                            .as_str()
                            .map(str::to_owned),
                        wire_sha256: wire_sha256(&client_line),
                    }).await?;
                }
                child_stdin
                    .write_all(&client_line)
                    .await
                    .map_err(|error| format!("forward client MCP: {error}"))?;
                child_stdin.flush().await.map_err(|error| error.to_string())?;
            }
            read = server.read_until(b'\n', &mut server_line) => {
                let read = read.map_err(|error| format!("read server MCP: {error}"))?;
                if read == 0 {
                    break;
                }
                if let Ok(message) = serde_json::from_slice::<Value>(&server_line)
                    && is_response(&message)
                    && let Some(id) = message.get("id")
                    && let Some(key) = request_key(id)
                    && let Some((request_id, tool, arguments)) = pending.remove(&key)
                {
                    sink.append(&McpObservationEvent {
                        phase: "response".into(),
                        server: config.server_name.clone(),
                        request_id,
                        tool,
                        arguments,
                        result: message.get("result").cloned(),
                        error: message.get("error").cloned(),
                        cancellation_reason: None,
                        wire_sha256: wire_sha256(&server_line),
                    }).await?;
                }
                client_stdout
                    .write_all(&server_line)
                    .await
                    .map_err(|error| format!("forward server MCP: {error}"))?;
                client_stdout.flush().await.map_err(|error| error.to_string())?;
            }
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|error| format!("wait for observed MCP server: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("observed MCP server exited with {status}"))
    }
}

/// Run the product observer. Missing or invalid side-channel configuration
/// disables observation but leaves the native MCP child operational.
///
/// # Errors
///
/// Returns an error when the child cannot be spawned, the native MCP stream
/// cannot be forwarded, or the child exits unsuccessfully.
pub async fn run_ephemeral_stdio_observer(config: StdioObserverConfig) -> Result<(), String> {
    let sink = ephemeral_sink().await;
    observe(config, sink).await
}

/// Run the same relay with an exact create-only JSONL evidence sink. This is
/// used only by protocol fixtures; product Workbench never persists raw data.
///
/// # Errors
///
/// Returns an error when the evidence file cannot be created, the child or
/// stream fails, or the child exits unsuccessfully.
pub async fn run_evidence_stdio_observer(
    config: StdioObserverConfig,
    evidence: &Path,
) -> Result<(), String> {
    let sink = ObservationSink::File(create_evidence(evidence)?);
    observe(config, sink).await
}
