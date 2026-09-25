//! Minimal ACP fixture for a cross-namespace workspace link.
//!
//! The hermetic Podman CLI double starts this host process in the inspected
//! bind source while ACP intentionally addresses the same directory as
//! `/workspace`. This binary is protocol/lifecycle evidence, not an isolation
//! fixture and not product code.

use std::fs;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, ResourceLink, SessionId,
    SessionNotification, SessionUpdate, StopReason,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};

const MARKER: &str = "SWEM_PODMAN_WORKSPACE_LINK";
const OUTPUT: &str = "podman-linked-output.bin";
const OUTPUT_BYTES: &[u8] = b"\0SWEM_PODMAN_LINKED_OUTPUT\xff\x21\x80";

#[tokio::main]
async fn main() -> Result<(), Error> {
    let session = Arc::new(Mutex::new(None::<SessionId>));
    let session_writer = Arc::clone(&session);
    let session_reader = session;
    Agent
        .builder()
        .name("swem-workspace-link-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                if request.cwd != std::path::Path::new("/workspace") {
                    return Err(internal("fixture expected ACP cwd /workspace"));
                }
                let id = SessionId::new("podman-workspace-link-session");
                *session_writer
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))? = Some(id.clone());
                responder.respond(NewSessionResponse::new(id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                let active = session_reader
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?
                    .clone()
                    .ok_or_else(|| internal("prompt before session/new"))?;
                if request.session_id != active {
                    return Err(internal("prompt session id differs from active session"));
                }
                let marker = request.prompt.iter().any(|block| match block {
                    ContentBlock::Text(text) => text.text.contains(MARKER),
                    _ => false,
                });
                if !marker {
                    return Err(internal("fixture prompt marker is missing"));
                }
                fs::write(OUTPUT, OUTPUT_BYTES)
                    .map_err(|error| internal(format!("write workspace output: {error}")))?;
                for resource in [
                    ResourceLink::new(OUTPUT, format!("file:///workspace/{OUTPUT}"))
                        .mime_type("application/octet-stream")
                        .size(i64::try_from(OUTPUT_BYTES.len()).map_err(|error| {
                            internal(format!("workspace output size conversion: {error}"))
                        })?),
                    ResourceLink::new(
                        "escaped-output.bin",
                        "file:///workspace/%2e%2e/escaped-output.bin",
                    )
                    .mime_type("application/octet-stream"),
                    ResourceLink::new("outside-output.bin", "file:///outside-output.bin")
                        .mime_type("application/octet-stream"),
                ] {
                    connection.send_notification(SessionNotification::new(
                        request.session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(
                            ContentBlock::ResourceLink(resource),
                        )),
                    ))?;
                }
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

fn internal(message: impl Into<String>) -> Error {
    Error::internal_error().data(message.into())
}
