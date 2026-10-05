//! Connection-local ingress and lifecycle state for agent-initiated MCP Apps.
//!
//! Raw tool arguments/results live only in this runtime object. The routing
//! ledger receives redacted identities and digests, never the payload.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
use base64::Engine as _;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncBufReadExt as _;
use tokio::sync::{Mutex, Notify, mpsc};

use crate::mcp_observer::{
    McpObservationEvent, OBSERVER_ENDPOINT_ENV, OBSERVER_PROTOCOL, OBSERVER_SERVER_ENV,
    OBSERVER_TOKEN_ENV, ObserverHello,
};
use crate::{RoutingLedger, SurfaceEventSource};

const MAX_OBSERVER_LINE: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub(crate) struct ObserverCommand {
    pub executable: PathBuf,
    pub prefix_args: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservedAppCall {
    pub observation_id: String,
    pub cursor: u64,
    pub server_name: String,
    pub tool: String,
    pub resource_uri: String,
    pub arguments: Value,
    pub status: &'static str,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub cancellation_reason: Option<String>,
}

struct ObservationState {
    calls: Vec<ObservedAppCall>,
    pending: BTreeMap<(String, String), String>,
    next_cursor: u64,
}

struct AuthenticatedEvent {
    stream_server: String,
    event: McpObservationEvent,
}

pub(crate) struct ObservationIngress {
    endpoint: std::net::SocketAddr,
    tokens: BTreeMap<String, String>,
    receiver: mpsc::Receiver<AuthenticatedEvent>,
    accept_task: AbortOnDrop,
}

struct AbortOnDrop(Option<tokio::task::JoinHandle<()>>);

impl AbortOnDrop {
    fn take(&mut self) -> tokio::task::JoinHandle<()> {
        self.0.take().expect("observer accept task is present")
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(task) = self.0.take() {
            task.abort();
        }
    }
}

pub(crate) struct ObservationRuntime {
    state: Arc<Mutex<ObservationState>>,
    notify: Arc<Notify>,
    accept_task: tokio::task::JoinHandle<()>,
    projection_task: tokio::task::JoinHandle<()>,
}

fn random_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

async fn read_line(
    reader: &mut tokio::io::BufReader<tokio::net::tcp::OwnedReadHalf>,
) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let read = reader
        .read_until(b'\n', &mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    if read == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_OBSERVER_LINE {
        return Err("MCP observation line exceeds the connection-local limit".into());
    }
    Ok(Some(bytes))
}

async fn accept_observer(
    stream: tokio::net::TcpStream,
    authorizations: Arc<Mutex<BTreeMap<String, String>>>,
    sender: mpsc::Sender<AuthenticatedEvent>,
) -> Result<(), String> {
    let (read, _write) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read);
    let hello = read_line(&mut reader)
        .await?
        .ok_or_else(|| "observer disconnected before authentication".to_owned())?;
    let hello: ObserverHello = serde_json::from_slice(&hello).map_err(|error| error.to_string())?;
    if hello.protocol != OBSERVER_PROTOCOL {
        return Err("observer protocol mismatch".into());
    }
    let expected_server = authorizations
        .lock()
        .await
        .remove(&hello.token)
        .ok_or_else(|| "unknown or reused observer credential".to_owned())?;
    if expected_server != hello.server {
        return Err("observer credential is bound to another attachment".into());
    }
    while let Some(bytes) = read_line(&mut reader).await? {
        let event: McpObservationEvent =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if event.server != expected_server {
            return Err("observer event changed its authenticated attachment".into());
        }
        if sender
            .send(AuthenticatedEvent {
                stream_server: expected_server.clone(),
                event,
            })
            .await
            .is_err()
        {
            break;
        }
    }
    Ok(())
}

impl ObservationIngress {
    pub async fn bind(servers: BTreeSet<String>) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| error.to_string())?;
        let endpoint = listener.local_addr().map_err(|error| error.to_string())?;
        let mut tokens = BTreeMap::new();
        let mut authorizations = BTreeMap::new();
        for server in servers {
            let token = random_token()?;
            authorizations.insert(token.clone(), server.clone());
            tokens.insert(server, token);
        }
        let authorizations = Arc::new(Mutex::new(authorizations));
        let (sender, receiver) = mpsc::channel(128);
        let accept_task = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((stream, peer)) = accepted else { break };
                        if !peer.ip().is_loopback() {
                            continue;
                        }
                        let authorizations = Arc::clone(&authorizations);
                        let sender = sender.clone();
                        connections.spawn(async move {
                            let _ = accept_observer(stream, authorizations, sender).await;
                        });
                    }
                    completed = connections.join_next(), if !connections.is_empty() => {
                        let _ = completed;
                    }
                }
            }
            // Dropping the JoinSet aborts every authenticated reader, so a
            // disconnected Workbench cannot retain a side-channel socket.
        });
        Ok(Self {
            endpoint,
            tokens,
            receiver,
            accept_task: AbortOnDrop(Some(accept_task)),
        })
    }

    pub fn wrap(
        &self,
        server: &McpServerStdio,
        command: &ObserverCommand,
    ) -> Result<McpServer, String> {
        let token = self
            .tokens
            .get(&server.name)
            .ok_or_else(|| format!("no observation credential for {}", server.name))?;
        if server.env.iter().any(|variable| {
            matches!(
                variable.name.as_str(),
                OBSERVER_ENDPOINT_ENV | OBSERVER_TOKEN_ENV | OBSERVER_SERVER_ENV
            )
        }) {
            return Err(format!(
                "attachment {} uses a reserved observer environment name",
                server.name
            ));
        }
        let child = server
            .command
            .to_str()
            .ok_or_else(|| format!("attachment {} command is not valid UTF-8", server.name))?;
        let mut args = command.prefix_args.clone();
        args.extend([
            "--server-name".to_owned(),
            server.name.clone(),
            "--".to_owned(),
            child.to_owned(),
        ]);
        args.extend(server.args.clone());
        let mut environment = server.env.clone();
        environment.extend([
            EnvVariable::new(OBSERVER_ENDPOINT_ENV, self.endpoint.to_string()),
            EnvVariable::new(OBSERVER_TOKEN_ENV, token.clone()),
            EnvVariable::new(OBSERVER_SERVER_ENV, server.name.clone()),
        ]);
        let mut wrapped = McpServerStdio::new(&server.name, &command.executable)
            .args(args)
            .env(environment);
        wrapped.meta.clone_from(&server.meta);
        Ok(McpServer::Stdio(wrapped))
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one projection task keeps request/response state, redaction and ordered ledger descriptors in one lifecycle"
    )]
    pub fn start(
        mut self,
        app_links: BTreeMap<(String, String), String>,
        ledger_path: PathBuf,
        route_id: String,
        event_namespace: String,
    ) -> ObservationRuntime {
        let state = Arc::new(Mutex::new(ObservationState {
            calls: Vec::new(),
            pending: BTreeMap::new(),
            next_cursor: 1,
        }));
        let notify = Arc::new(Notify::new());
        let projection_state = Arc::clone(&state);
        let projection_notify = Arc::clone(&notify);
        let mut receiver = self.receiver;
        let projection_task = tokio::spawn(async move {
            while let Some(authenticated) = receiver.recv().await {
                let event = authenticated.event;
                let link_key = (authenticated.stream_server.clone(), event.tool.clone());
                let Some(resource_uri) = app_links.get(&link_key) else {
                    continue;
                };
                let request_key = match serde_json::to_string(&event.request_id) {
                    Ok(key) => (authenticated.stream_server.clone(), key),
                    Err(_) => continue,
                };
                if event.phase == "request" {
                    let mut locked = projection_state.lock().await;
                    if locked.pending.contains_key(&request_key) {
                        continue;
                    }
                    let cursor = locked.next_cursor;
                    locked.next_cursor += 1;
                    let observation_id = format!("o{cursor}");
                    locked.pending.insert(request_key, observation_id.clone());
                    locked.calls.push(ObservedAppCall {
                        observation_id: observation_id.clone(),
                        cursor,
                        server_name: authenticated.stream_server.clone(),
                        tool: event.tool.clone(),
                        resource_uri: resource_uri.clone(),
                        arguments: event.arguments.clone(),
                        status: "pending",
                        result: None,
                        error: None,
                        cancellation_reason: None,
                    });
                    drop(locked);
                    append_descriptor(
                        ledger_path.clone(),
                        route_id.clone(),
                        format!("{event_namespace}-{cursor}-request"),
                        json!({
                            "observation_id": observation_id,
                            // Where the call stands among the calls of this
                            // session, for a page to ask for this one.
                            "cursor": cursor,
                            "server": authenticated.stream_server,
                            "tool": event.tool,
                            // The App the tool brings, for whoever offers it.
                            "resource_uri": resource_uri,
                            "phase": "request",
                            "request_id_digest": digest_json(&event.request_id),
                            "arguments_digest": digest_json(&event.arguments),
                            "wire_digest": event.wire_sha256,
                        }),
                    )
                    .await;
                    projection_notify.notify_waiters();
                } else if matches!(event.phase.as_str(), "response" | "cancelled") {
                    let mut locked = projection_state.lock().await;
                    let Some(observation_id) = locked.pending.remove(&request_key) else {
                        continue;
                    };
                    let Some(call) = locked
                        .calls
                        .iter_mut()
                        .find(|call| call.observation_id == observation_id)
                    else {
                        continue;
                    };
                    call.result = event.result.clone();
                    call.error = event.error.clone();
                    call.cancellation_reason = event.cancellation_reason.clone();
                    call.status = observation_status(&event);
                    let cursor = call.cursor;
                    let status = call.status;
                    let phase = event.phase.clone();
                    drop(locked);
                    append_descriptor(
                        ledger_path.clone(),
                        route_id.clone(),
                        format!("{event_namespace}-{cursor}-{phase}"),
                        json!({
                            "observation_id": observation_id,
                            "server": authenticated.stream_server,
                            "tool": event.tool,
                            "phase": phase,
                            "status": status,
                            "result_digest": event.result.as_ref().map(digest_json),
                            "error_digest": event.error.as_ref().map(digest_json),
                            "cancellation_reason_digest": event.cancellation_reason
                                .as_ref()
                                .map(|reason| digest_json(&Value::String(reason.clone()))),
                            "wire_digest": event.wire_sha256,
                        }),
                    )
                    .await;
                    projection_notify.notify_waiters();
                }
            }
        });
        ObservationRuntime {
            state,
            notify,
            accept_task: self.accept_task.take(),
            projection_task,
        }
    }
}

fn observation_status(event: &McpObservationEvent) -> &'static str {
    if event.phase == "cancelled" {
        "cancelled"
    } else if event.error.is_some() {
        "protocol_error"
    } else if event
        .result
        .as_ref()
        .and_then(|result| result.get("isError"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "tool_error"
    } else {
        "completed"
    }
}

fn digest_json(value: &Value) -> String {
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(bytes))
}

async fn append_descriptor(path: PathBuf, route: String, event_id: String, payload: Value) {
    let written = tokio::task::spawn_blocking(move || {
        let mut ledger = RoutingLedger::open(&path)?;
        ledger.append_event(
            &route,
            &event_id,
            "host/app_tool_observed",
            SurfaceEventSource::Host,
            &payload,
        )?;
        Ok::<_, crate::RoutingError>(())
    })
    .await;
    match written {
        Err(error) => eprintln!("SWEM App observation descriptor task failed: {error}"),
        Ok(Err(error)) => eprintln!("SWEM App observation descriptor write failed: {error}"),
        Ok(Ok(())) => {}
    }
}

impl ObservationRuntime {
    pub async fn next(&self, after: u64, wait: Duration) -> Option<ObservedAppCall> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            if let Some(call) = self
                .state
                .lock()
                .await
                .calls
                .iter()
                .find(|call| call.cursor > after)
                .cloned()
            {
                return Some(call);
            }
            let notified = self.notify.notified();
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() || tokio::time::timeout(remaining, notified).await.is_err() {
                return None;
            }
        }
    }

    pub async fn get_terminal(&self, id: &str, wait: Duration) -> Option<ObservedAppCall> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let call = self
                .state
                .lock()
                .await
                .calls
                .iter()
                .find(|call| call.observation_id == id)
                .cloned()?;
            if call.status != "pending" {
                return Some(call);
            }
            let notified = self.notify.notified();
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() || tokio::time::timeout(remaining, notified).await.is_err() {
                return Some(call);
            }
        }
    }

    pub async fn shutdown(self) {
        self.accept_task.abort();
        self.projection_task.abort();
        let _ = self.accept_task.await;
        let _ = self.projection_task.await;
        let mut state = self.state.lock().await;
        state.calls.clear();
        state.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(phase: &str, result: Option<Value>, error: Option<Value>) -> McpObservationEvent {
        McpObservationEvent {
            phase: phase.into(),
            server: "fixture".into(),
            request_id: json!(1),
            tool: "fixture_tool".into(),
            arguments: json!({}),
            result,
            error,
            cancellation_reason: None,
            wire_sha256: "sha256:fixture".into(),
        }
    }

    #[test]
    fn terminal_status_preserves_the_four_distinct_mcp_outcomes() {
        assert_eq!(
            observation_status(&event("response", Some(json!({"content": []})), None)),
            "completed"
        );
        assert_eq!(
            observation_status(&event(
                "response",
                Some(json!({"content": [], "isError": true})),
                None,
            )),
            "tool_error"
        );
        assert_eq!(
            observation_status(&event(
                "response",
                None,
                Some(json!({"code": -32603, "message": "server failed"})),
            )),
            "protocol_error"
        );
        assert_eq!(
            observation_status(&event("cancelled", None, None)),
            "cancelled"
        );
    }
}
