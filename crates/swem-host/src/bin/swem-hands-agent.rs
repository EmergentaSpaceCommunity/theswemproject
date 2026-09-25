//! A fixture agent with hands: it calls one tool on the project a person
//! attached to it, and says what came back.
//!
//! This is not intelligence and does not pretend to be. Everything a real
//! agent would decide - which tool, with what arguments - arrives in the
//! prompt as JSON, and the agent does exactly that and reports the result.
//! What it exists to prove is the seam a person cannot see and cannot test by
//! hand: that attaching a project to an agent in the Workbench really puts
//! that project in the agent's own hands, over its own MCP connection, and
//! that what it writes lands in the same journal the person's own edits land
//! in.
//!
//! A prompt is `{"tool": "...", "arguments": {...}}`, optionally with
//! `"server"` naming which attachment to call it on (the first one otherwise).
//! Anything else comes back as a sentence saying what this fixture accepts.
//!
//! It answers `session/resume`, so a conversation can be picked up after the
//! process that started it is gone. It keeps no memory of what was said: a
//! real agent that advertises this remembers, and what a resume against this
//! fixture proves is the harness's half - one lane, one id, and the record
//! replayed - and never the agent's.
//!
//! Files come in two kinds, and the difference is the point of them. `"read"`
//! and `"write"` open a path on the disk, which is what an agent with hands
//! and no protocol does. `"client_read"` and `"client_write"` ask the client
//! for the same file over ACP, which is what an agent does when the client is
//! an editor: the file the person is looking at is in that editor's buffer,
//! unsaved edits and all, and the disk has a different one.
//! `"client_read_anywhere"` names a path exactly as given, boundary and all -
//! a fixture that only ever asked for paths it had checked itself could never
//! show that the harness checks them too.
//!
//! `"run"` asks the client to run a command, which is what an agent does
//! when the client owns the environment the command belongs in - here, the
//! profile's own. `"keep": true` leaves it running and reports the terminal's
//! id, so a person can watch it rather than only read what it said.
//!
//! A prompt may also carry `"ask"`: what the agent wants permission for,
//! in a person's words. The agent then asks before doing anything, the way a
//! real one asks before it touches something, and does the work only if the
//! answer is yes. That is how a walk can prove where the question is shown -
//! a surface that never sees it leaves the agent waiting forever.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, CreateTerminalRequest, InitializeRequest,
    InitializeResponse, McpServer, McpServerStdio, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, PromptRequest, PromptResponse, ReadTextFileRequest,
    ReleaseTerminalRequest, RequestPermissionOutcome, RequestPermissionRequest,
    ResumeSessionRequest, ResumeSessionResponse, SessionCapabilities, SessionId,
    SessionNotification, SessionResumeCapabilities, SessionUpdate, StopReason,
    TerminalOutputRequest, TextContent, ToolCallId, ToolCallUpdate, ToolCallUpdateFields,
    WaitForTerminalExitRequest, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};
use rmcp::ServiceExt as _;
use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Session {
    id: SessionId,
    cwd: PathBuf,
    attachments: Vec<McpServerStdio>,
}

#[derive(Clone, Debug, Default)]
struct State {
    session: Arc<Mutex<Option<Session>>>,
}

fn internal(message: impl Into<String>) -> Error {
    Error::internal_error().data(json!({ "reason": message.into() }))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    serve().await?;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "one function per agent: initialize, new, resume and prompt read as the protocol they answer"
)]
async fn serve() -> Result<(), Error> {
    let state = State::default();
    let writer = state.clone();
    let resumer = state.clone();
    let reader = state;
    Agent
        .builder()
        .name("swem-hands-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(request.protocol_version).agent_capabilities(
                        // A conversation that can be picked up again. A real
                        // agent that advertises this remembers what was said;
                        // this one keeps no memory of its own, and says so in
                        // its own documentation, so what a resume here proves
                        // is the harness's half of it and not the agent's.
                        AgentCapabilities::new().session_capabilities(
                            SessionCapabilities::new().resume(SessionResumeCapabilities::new()),
                        ),
                    ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                if !request.cwd.is_absolute() {
                    return Err(internal("session cwd is not absolute"));
                }
                // Whatever the person attached: this agent has no opinion about
                // which servers it should be given.
                let attachments: Vec<McpServerStdio> = request
                    .mcp_servers
                    .iter()
                    .filter_map(|server| match server {
                        McpServer::Stdio(stdio) => Some(stdio.clone()),
                        _ => None,
                    })
                    .collect();
                let id = SessionId::new(format!("hands-{}", std::process::id()));
                *writer
                    .session
                    .lock()
                    .map_err(|_| internal("session lock poisoned"))? = Some(Session {
                    id: id.clone(),
                    cwd: request.cwd,
                    attachments,
                });
                responder.respond(NewSessionResponse::new(id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ResumeSessionRequest, responder, _connection| {
                if !request.cwd.is_absolute() {
                    return Err(internal("session cwd is not absolute"));
                }
                // The id belongs to the conversation, not to the process that
                // first served it: a resume arrives at a new process, and the
                // one thing it must not do is hand back an id of its own and
                // leave the caller talking to a different session than the one
                // it named.
                let attachments: Vec<McpServerStdio> = request
                    .mcp_servers
                    .iter()
                    .filter_map(|server| match server {
                        McpServer::Stdio(stdio) => Some(stdio.clone()),
                        _ => None,
                    })
                    .collect();
                *resumer
                    .session
                    .lock()
                    .map_err(|_| internal("session lock poisoned"))? = Some(Session {
                    id: request.session_id.clone(),
                    cwd: request.cwd,
                    attachments,
                });
                responder.respond(ResumeSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                let session = reader
                    .session
                    .lock()
                    .map_err(|_| internal("session lock poisoned"))?
                    .clone()
                    .ok_or_else(|| internal("prompt received before session/new"))?;
                if request.session_id != session.id {
                    return Err(internal("prompt is not bound to the open session"));
                }
                let text = request
                    .prompt
                    .iter()
                    .find_map(|block| match block {
                        ContentBlock::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .unwrap_or_default();
                // The work runs in a task of its own rather than in this
                // handler. An agent that asks for permission waits for the
                // answer, and waiting here would stop this connection from
                // reading the very message the answer arrives in - the
                // question would be answered and the agent would never hear
                // it. Doing the work outside costs nothing when nothing is
                // asked.
                let working = connection.clone();
                let session_id = request.session_id.clone();
                connection.spawn(async move {
                    let answer = act(&session, &working, &text).await;
                    working.send_notification(SessionNotification::new(
                        session_id,
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(answer),
                        ))),
                    ))?;
                    responder.respond(PromptResponse::new(StopReason::EndTurn))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

/// Do what the prompt says, and say what happened - including when it could
/// not be done, because a silent agent is the worst kind.
async fn act(session: &Session, connection: &ConnectionTo<Client>, text: &str) -> String {
    let Ok(prompt) = serde_json::from_str::<Value>(text) else {
        return format!(
            "this fixture takes a tool call as JSON: {{\"tool\": \"...\", \"arguments\": {{...}}}}; it attaches {}",
            names(session)
        );
    };
    // Asked before anything is done, not after: a permission that arrives
    // once the file is written is not a permission.
    if let Some(about) = prompt.get("ask").and_then(Value::as_str) {
        match ask_first(connection, session, about).await {
            Ok(true) => {}
            Ok(false) => return format!("not done: the answer to {about} was no"),
            Err(reason) => return format!("could not ask about {about}: {reason}"),
        }
    }
    // Files, the way an agent that speaks no protocol for them does it: open
    // a path under the directory it was started in. `read` takes a path,
    // `write` takes a path and the text to put there. Both are refused
    // outside the session's own cwd, because that is the boundary the product
    // gives this agent and a fixture that ignored it would prove nothing.
    if let Some(path) = prompt.get("read").and_then(Value::as_str) {
        return match under_cwd(session, path) {
            Ok(path) => std::fs::read_to_string(&path).map_or_else(
                |error| format!("could not read {}: {error}", path.display()),
                |text| format!("read {}: {text}", path.display()),
            ),
            Err(refusal) => refusal,
        };
    }
    if let Some(path) = prompt.get("write").and_then(Value::as_str) {
        let body = prompt.get("text").and_then(Value::as_str).unwrap_or("");
        return match under_cwd(session, path) {
            Ok(path) => {
                if let Some(parent) = path.parent()
                    && let Err(error) = std::fs::create_dir_all(parent)
                {
                    return format!("could not make {}: {error}", parent.display());
                }
                std::fs::write(&path, body).map_or_else(
                    |error| format!("could not write {}: {error}", path.display()),
                    |()| format!("wrote {}", path.display()),
                )
            }
            Err(refusal) => refusal,
        };
    }
    // The same two things, asked of the client instead of done on the disk.
    if let Some(path) = prompt.get("client_read").and_then(Value::as_str) {
        return match under_cwd(session, path) {
            Ok(path) => read_through_the_client(connection, session, &path).await,
            Err(refusal) => refusal,
        };
    }
    if let Some(path) = prompt.get("client_write").and_then(Value::as_str) {
        let body = prompt.get("text").and_then(Value::as_str).unwrap_or("");
        return match under_cwd(session, path) {
            Ok(path) => write_through_the_client(connection, session, &path, body).await,
            Err(refusal) => refusal,
        };
    }
    if let Some(path) = prompt.get("client_read_anywhere").and_then(Value::as_str) {
        return read_through_the_client(connection, session, std::path::Path::new(path)).await;
    }
    // A command, asked of the client over ACP rather than started by this
    // process. That is what an agent does when the client owns the
    // environment the command belongs in, and it is why a person can watch
    // it: the terminal it runs in is one of theirs. `"keep": true` leaves it
    // running and reports its id instead of waiting for it, which is the
    // case where watching means anything.
    if let Some(line) = prompt.get("run").and_then(Value::as_str) {
        let keep = prompt.get("keep").and_then(Value::as_bool).unwrap_or(false);
        return run_through_the_client(connection, session, line, keep).await;
    }
    let Some(tool) = prompt.get("tool").and_then(Value::as_str) else {
        return "the prompt names no tool".to_owned();
    };
    let wanted = prompt.get("server").and_then(Value::as_str);
    let Some(server) = session
        .attachments
        .iter()
        .find(|candidate| wanted.is_none_or(|wanted| candidate.name == wanted))
    else {
        return format!(
            "nothing is attached to this session{}; attach the project to the agent first",
            wanted.map_or(String::new(), |wanted| format!(" under the name {wanted}"))
        );
    };
    let arguments = prompt
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    match call(server, &session.cwd, tool, arguments).await {
        Ok(result) => result,
        Err(reason) => format!("{tool} did not run: {reason}"),
    }
}

/// Why a client refused, in the client's own words when it gave any.
///
/// ACP puts the message of a standard error code in `message` ("Invalid
/// params") and the reason for this particular one in `data`. An agent that
/// reported only the first would be telling a person nothing they can act on.
fn why(error: &Error) -> String {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("reason"))
        .and_then(Value::as_str)
        .map_or_else(|| error.message.clone(), str::to_owned)
}

/// Run one command through the client, in whatever environment the client
/// keeps for this session.
///
/// With `keep`, the command is started and left alone: what comes back is the
/// terminal's id, so whoever is watching can find it. Otherwise this waits
/// for it to end and says what it said.
async fn run_through_the_client(
    connection: &ConnectionTo<Client>,
    session: &Session,
    line: &str,
    keep: bool,
) -> String {
    let (command, args) = if cfg!(windows) {
        ("cmd".to_owned(), vec!["/C".to_owned(), line.to_owned()])
    } else {
        ("/bin/sh".to_owned(), vec!["-c".to_owned(), line.to_owned()])
    };
    let created = connection
        .send_request(
            CreateTerminalRequest::new(session.id.clone(), command)
                .args(args)
                .cwd(session.cwd.clone()),
        )
        .block_task()
        .await;
    let terminal = match created {
        Ok(created) => created.terminal_id,
        Err(error) => return format!("could not run {line}: {}", why(&error)),
    };
    if keep {
        return format!("running {line} in terminal {}", terminal.0);
    }
    if let Err(error) = connection
        .send_request(WaitForTerminalExitRequest::new(
            session.id.clone(),
            terminal.clone(),
        ))
        .block_task()
        .await
    {
        return format!("could not wait for {line}: {}", why(&error));
    }
    let said = match connection
        .send_request(TerminalOutputRequest::new(
            session.id.clone(),
            terminal.clone(),
        ))
        .block_task()
        .await
    {
        Ok(read) => read.output,
        Err(error) => return format!("could not read {line}: {}", why(&error)),
    };
    let _ = connection
        .send_request(ReleaseTerminalRequest::new(session.id.clone(), terminal))
        .block_task()
        .await;
    format!("ran {line}: {said}")
}

/// Ask whoever is on the other end of this connection, and say whether they
/// said yes. The two options are the ones every agent offers for a one-off:
/// this time only, or not this time.
async fn ask_first(
    connection: &ConnectionTo<Client>,
    session: &Session,
    about: &str,
) -> Result<bool, String> {
    let mut fields = ToolCallUpdateFields::default();
    fields.title = Some(about.to_owned());
    let answer = connection
        .send_request(RequestPermissionRequest::new(
            session.id.clone(),
            ToolCallUpdate::new(ToolCallId::new(format!("hands-{about}")), fields),
            vec![
                PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce),
                PermissionOption::new(
                    "reject-once",
                    "Reject once",
                    PermissionOptionKind::RejectOnce,
                ),
            ],
        ))
        .block_task()
        .await
        .map_err(|error| error.to_string())?;
    Ok(match answer.outcome {
        RequestPermissionOutcome::Selected(selected) => &*selected.option_id.0 == "allow-once",
        _ => false,
    })
}

/// Ask the client for a file, and say what it said - including its refusal
/// in full, because a walk cannot tell one refusal from another without the
/// reason.
async fn read_through_the_client(
    connection: &ConnectionTo<Client>,
    session: &Session,
    path: &std::path::Path,
) -> String {
    match connection
        .send_request(ReadTextFileRequest::new(
            session.id.clone(),
            path.to_path_buf(),
        ))
        .block_task()
        .await
    {
        Ok(answer) => format!("the client read {}: {}", path.display(), answer.content),
        Err(error) => format!(
            "the client did not read {}: {}",
            path.display(),
            said(&error)
        ),
    }
}

/// Ask the client to put text in a file, and say what it said.
async fn write_through_the_client(
    connection: &ConnectionTo<Client>,
    session: &Session,
    path: &std::path::Path,
    body: &str,
) -> String {
    match connection
        .send_request(WriteTextFileRequest::new(
            session.id.clone(),
            path.to_path_buf(),
            body.to_owned(),
        ))
        .block_task()
        .await
    {
        Ok(_) => format!("the client wrote {}", path.display()),
        Err(error) => format!(
            "the client did not write {}: {}",
            path.display(),
            said(&error)
        ),
    }
}

/// An error with its data still on it. `Display` keeps the sentence and drops
/// the reason, and the reason is the whole of what a refusal means.
fn said(error: &Error) -> String {
    serde_json::to_string(error).unwrap_or_else(|_| error.to_string())
}

/// `requested` joined to the session's own directory, refused if it would
/// leave it. Checked before anything is created, so a refused write makes no
/// directory either.
fn under_cwd(session: &Session, requested: &str) -> Result<PathBuf, String> {
    let path = session.cwd.join(requested);
    if requested.is_empty()
        || path.components().any(|component| {
            matches!(component, std::path::Component::ParentDir)
                || matches!(component, std::path::Component::Prefix(_))
        })
        || !path.starts_with(&session.cwd)
    {
        return Err(format!(
            "{requested} is outside {}, which is where this agent works",
            session.cwd.display()
        ));
    }
    Ok(path)
}

fn names(session: &Session) -> String {
    if session.attachments.is_empty() {
        return "nothing".to_owned();
    }
    session
        .attachments
        .iter()
        .map(|server| server.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// One tool call over the agent's own MCP connection to the attachment.
async fn call(
    server: &McpServerStdio,
    cwd: &std::path::Path,
    tool: &str,
    arguments: JsonObject,
) -> Result<String, String> {
    let mut command = tokio::process::Command::new(&server.command);
    command.args(&server.args).current_dir(cwd);
    for variable in &server.env {
        command.env(&variable.name, &variable.value);
    }
    let transport = TokioChildProcess::new(command).map_err(|error| error.to_string())?;
    let client = ().serve(transport).await.map_err(|error| error.to_string())?;
    let result = client
        .call_tool(CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments))
        .await
        .map_err(|error| error.to_string())?;
    let said = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join(" ");
    client.cancel().await.ok();
    if result.is_error.unwrap_or(false) {
        return Err(said);
    }
    Ok(said)
}
