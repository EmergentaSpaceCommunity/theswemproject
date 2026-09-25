//! Adversarial ACP peer: probes unadvertised client callbacks, not an agent adapter.
//!
//! Turn one asks the client for every `fs/*` and `terminal/*` operation it never
//! advertised and reports what came back over the wire, so the evidence is the
//! host's own transcript rather than a file this fixture wrote into the host's
//! workspace. Turn two runs the agent's own OS tool afterwards: refusing a
//! client callback must not disable what the agent can already do for itself.
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio as ChildStdio,
    sync::{Arc, Mutex},
};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, CreateTerminalRequest, InitializeRequest, InitializeResponse,
    KillTerminalRequest, NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse,
    ReadTextFileRequest, ReleaseTerminalRequest, SessionNotification, SessionUpdate, StopReason,
    TerminalOutputRequest, TextContent, WaitForTerminalExitRequest, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};
use serde_json::{Value, json};

/// Prompt that asks for the refused callbacks.
const PROBE_TURN: &str = "probe callback authority";
/// Prompt that exercises the agent's own tool after the refusals.
const NATIVE_TURN: &str = "run native tool";
/// Prompt that asks for one named file, whatever it is and wherever it is.
/// The rest of the prompt is the absolute path, because the point of this
/// turn is a path the fixture was told to name rather than one it chose: a
/// connection whose client does carry `fs/*` still has a boundary, and it is
/// the host's to enforce, not this peer's to respect.
const BEYOND_TURN: &str = "probe the path ";
/// Prompt that asks for a command to actually be run. The rest of the prompt
/// is the command line, so the host - not this peer - decides what runs.
const RUN_TURN: &str = "run in a terminal ";
/// Prompt that asks for a terminal in one named directory, wherever it is.
/// Same reason as `BEYOND_TURN`: the boundary is the host's to enforce.
const RUN_AT_TURN: &str = "run a terminal in ";
/// Prompt that asks for a command to be started and then stopped. ACP keeps
/// killing and releasing apart, and this is why: an agent that stopped a build
/// still wants what the build said before it stopped.
const STOP_TURN: &str = "start and stop ";

fn failure(_: impl std::fmt::Display) -> Error {
    Error::internal_error()
}

/// One probe outcome, as data rather than a panic: a refused callback and an
/// answered one must be distinguishable in the report.
fn outcome<T: serde::Serialize>(answer: Result<T, Error>) -> Value {
    match answer {
        Ok(response) => json!({ "ok": serde_json::to_value(response).unwrap_or(Value::Null) }),
        Err(error) => json!({ "err": serde_json::to_value(error).unwrap_or(Value::Null) }),
    }
}

fn prompt_text(request: &PromptRequest) -> String {
    request
        .prompt
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

/// Both `fs/*` methods, asked about one exact path.
async fn probe_files(
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
    path: &Path,
) -> Value {
    let session = request.session_id.clone();
    json!({
        "fs/read_text_file": outcome(
            connection
                .send_request(ReadTextFileRequest::new(session.clone(), path.to_path_buf()))
                .block_task()
                .await,
        ),
        "fs/write_text_file": outcome(
            connection
                .send_request(WriteTextFileRequest::new(
                    session,
                    path.to_path_buf(),
                    "overwritten",
                ))
                .block_task()
                .await,
        ),
    })
}

/// The shell this platform has, since the command comes as one line.
fn shell_command(line: &str) -> (String, Vec<String>) {
    if cfg!(windows) {
        ("cmd".to_owned(), vec!["/C".to_owned(), line.to_owned()])
    } else {
        ("/bin/sh".to_owned(), vec!["-c".to_owned(), line.to_owned()])
    }
}

/// Run one command through the client and report every step of it: an agent
/// that gets a terminal id and nothing else has learned nothing.
async fn run_command(
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
    line: &str,
    cwd: &Path,
) -> Value {
    let session = request.session_id.clone();
    let (command, args) = shell_command(line);
    let created = connection
        .send_request(
            CreateTerminalRequest::new(session.clone(), command)
                .args(args)
                .cwd(cwd.to_path_buf()),
        )
        .block_task()
        .await;
    let Ok(created) = created else {
        return json!({ "terminal/create": outcome(created) });
    };
    let terminal = created.terminal_id.clone();
    let waited = connection
        .send_request(WaitForTerminalExitRequest::new(
            session.clone(),
            terminal.clone(),
        ))
        .block_task()
        .await;
    let read = connection
        .send_request(TerminalOutputRequest::new(
            session.clone(),
            terminal.clone(),
        ))
        .block_task()
        .await;
    let released = connection
        .send_request(ReleaseTerminalRequest::new(session, terminal.clone()))
        .block_task()
        .await;
    json!({
        "terminal_id": terminal.0.to_string(),
        "terminal/create": outcome(Ok::<_, Error>(created)),
        "terminal/wait_for_exit": outcome(waited),
        "terminal/output": outcome(read),
        "terminal/release": outcome(released),
    })
}

/// Start one command, stop it, and report what is left: the exit it was given
/// and everything it said before it was stopped.
async fn stop_command(
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
    line: &str,
    cwd: &Path,
) -> Value {
    let session = request.session_id.clone();
    let (command, args) = shell_command(line);
    let created = connection
        .send_request(
            CreateTerminalRequest::new(session.clone(), command)
                .args(args)
                .cwd(cwd.to_path_buf()),
        )
        .block_task()
        .await;
    let Ok(created) = created else {
        return json!({ "terminal/create": outcome(created) });
    };
    let terminal = created.terminal_id.clone();
    // Stop it once it has said something, the way an agent watching a build
    // stops it: killing before the first byte would prove nothing about what
    // survives the stopping.
    let mut spoke = false;
    for _ in 0..100 {
        let read = connection
            .send_request(TerminalOutputRequest::new(
                session.clone(),
                terminal.clone(),
            ))
            .block_task()
            .await;
        if read.is_ok_and(|read| !read.output.is_empty()) {
            spoke = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let killed = connection
        .send_request(KillTerminalRequest::new(session.clone(), terminal.clone()))
        .block_task()
        .await;
    let waited = connection
        .send_request(WaitForTerminalExitRequest::new(
            session.clone(),
            terminal.clone(),
        ))
        .block_task()
        .await;
    let read = connection
        .send_request(TerminalOutputRequest::new(
            session.clone(),
            terminal.clone(),
        ))
        .block_task()
        .await;
    let released = connection
        .send_request(ReleaseTerminalRequest::new(session, terminal))
        .block_task()
        .await;
    json!({
        "spoke_before_the_stop": spoke,
        "terminal/kill": outcome(killed),
        "terminal/wait_for_exit": outcome(waited),
        "terminal/output": outcome(read),
        "terminal/release": outcome(released),
    })
}

/// What one turn reports, chosen by what its prompt asked for.
async fn turn_report(
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
    root: &Path,
    caps: Value,
) -> Result<Value, Error> {
    Ok(match prompt_text(request).as_str() {
        PROBE_TURN => {
            json!({
                "capabilities": caps,
                "callbacks": probe_callbacks(connection, request, root).await,
            })
        }
        NATIVE_TURN => {
            let executable = std::env::current_exe().map_err(failure)?;
            // The agent's own tool must not inherit the ACP
            // pipes: anything it writes would corrupt the wire.
            let status = tokio::process::Command::new(executable)
                .arg("--marker")
                .arg(root.join("native-marker"))
                .stdin(ChildStdio::null())
                .stdout(ChildStdio::null())
                .stderr(ChildStdio::null())
                .kill_on_drop(true)
                .status()
                .await
                .map_err(failure)?;
            json!({ "native_tool_exit_success": status.success() })
        }
        line if line.starts_with(RUN_TURN) => {
            json!({
                "capabilities": caps,
                "terminal": run_command(
                    connection,
                    request,
                    &line[RUN_TURN.len()..],
                    root,
                ).await,
            })
        }
        line if line.starts_with(STOP_TURN) => {
            json!({
                "terminal": stop_command(
                    connection,
                    request,
                    &line[STOP_TURN.len()..],
                    root,
                ).await,
            })
        }
        line if line.starts_with(RUN_AT_TURN) => {
            let cwd = PathBuf::from(&line[RUN_AT_TURN.len()..]);
            json!({
                "cwd": cwd.display().to_string(),
                "terminal": run_command(
                    connection,
                    request,
                    // Leaves a mark if it ever runs, so a
                    // refusal that came too late is visible.
                    "echo ran > ran-here",
                    &cwd,
                ).await,
            })
        }
        beyond if beyond.starts_with(BEYOND_TURN) => {
            let path = PathBuf::from(&beyond[BEYOND_TURN.len()..]);
            json!({
                "path": path.display().to_string(),
                "callbacks": probe_files(connection, request, &path).await,
            })
        }
        other => json!({ "unexpected_prompt": other }),
    })
}

/// Every unadvertised callback, asked in one turn. A terminal id is invented on
/// purpose: a client that refuses the method never looks at it, and a client
/// that does not refuse must not be able to hide behind a bad argument.
async fn probe_callbacks(
    connection: &ConnectionTo<Client>,
    request: &PromptRequest,
    root: &Path,
) -> Value {
    let session = request.session_id.clone();
    let terminal = "callback-audit-terminal";
    let files = probe_files(connection, request, &root.join("sentinel.txt")).await;
    let mut report = json!({
        "terminal/create": outcome(
            connection
                .send_request(
                    CreateTerminalRequest::new(session.clone(), "cmd")
                        .args(vec!["/C".into(), "echo callback".into()])
                        .cwd(root.to_path_buf()),
                )
                .block_task()
                .await,
        ),
        "terminal/output": outcome(
            connection
                .send_request(TerminalOutputRequest::new(session.clone(), terminal))
                .block_task()
                .await,
        ),
        "terminal/wait_for_exit": outcome(
            connection
                .send_request(WaitForTerminalExitRequest::new(session.clone(), terminal))
                .block_task()
                .await,
        ),
        "terminal/kill": outcome(
            connection
                .send_request(KillTerminalRequest::new(session.clone(), terminal))
                .block_task()
                .await,
        ),
        "terminal/release": outcome(
            connection
                .send_request(ReleaseTerminalRequest::new(session, terminal))
                .block_task()
                .await,
        ),
    });
    if let (Some(report), Some(files)) = (report.as_object_mut(), files.as_object()) {
        for (method, answer) in files {
            report.insert(method.clone(), answer.clone());
        }
    }
    report
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--marker") {
        fs::write(args.get(2).ok_or("missing marker path")?, b"executed")?;
        return Ok(());
    }
    let capabilities = Arc::new(Mutex::new(Value::Null));
    let trace = args.get(1).cloned();
    let captured = Arc::clone(&capabilities);
    let workspace = Arc::new(Mutex::new(PathBuf::new()));
    let bound = Arc::clone(&workspace);
    Agent
        .builder()
        .on_receive_request(
            async move |request: InitializeRequest, reply, _| {
                *captured.lock().map_err(failure)? =
                    serde_json::to_value(request.client_capabilities).map_err(failure)?;
                reply.respond(InitializeResponse::new(ProtocolVersion::V1))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, reply, _| {
                *bound.lock().map_err(failure)? = request.cwd;
                reply.respond(NewSessionResponse::new("callback-audit"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, reply, cx| {
                let root = workspace.lock().map_err(failure)?.clone();
                let caps = capabilities.lock().map_err(failure)?.clone();
                let connection: ConnectionTo<Client> = cx.clone();
                // The turn's work leaves the dispatcher free to receive the
                // client's answers; a reply that never arrives is the failure
                // this fixture exists to expose, so nothing here may block it.
                cx.spawn(async move {
                    let report = turn_report(&connection, &request, &root, caps).await?;
                    connection.send_notification(SessionNotification::new(
                        request.session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(report.to_string()),
                        ))),
                    ))?;
                    reply.respond(PromptResponse::new(StopReason::EndTurn))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new().with_debug(move |line, _| {
            if let Some(path) = &trace
                && let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path)
            {
                let _ = writeln!(file, "{line}");
            }
        }))
        .await?;
    Ok(())
}
