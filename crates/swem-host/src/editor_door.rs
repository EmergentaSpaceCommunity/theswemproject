//! The second door onto the same product: an editor, over stdio.
//!
//! The Workbench is a page over HTTP. This is the same harness answering an
//! editor that speaks ACP — Zed, a `JetBrains` plugin, Neovim — so a person can
//! work with the agent SWEM manages without leaving where they write code, and
//! the conversation lands in the same record as the one in the page.
//!
//! SWEM is an ACP client everywhere else: it drives agents. Here it is the
//! agent, and the turn it is asked for goes to the profile's real agent
//! underneath. What the editor gets is the profile's agent, working in the
//! profile's directory, under the profile's permission mode, with the profile's
//! attached servers.
//!
//! # This door is not a way around the other one
//!
//! The Workbench door has a lock: a secret in the address, and a refusal for a
//! request that states a foreign origin. This door has neither and cannot: the
//! editor starts SWEM as its own child process, so whoever started the editor
//! is already who this runs as, and there is no request to check. That makes it
//! worth saying exactly what it does *not* give away:
//!
//! - **The profile's boundary is the profile's, not the editor's.** ACP hands a
//!   `cwd` with every new session. This door does not take it: the session runs
//!   in the profile's own working directory, and an editor asking for another
//!   one is told so rather than obeyed. Taking it would mean an editor could
//!   point a profile that works alone at any directory on the machine and have
//!   the harness enforce a boundary around it — which is the boundary gone.
//! - **No secret leaves through it.** This door reads a profile to run it. It
//!   never serves a profile's secrets, never serves the Workbench's own
//!   session secret, and offers nothing of the `/api` surface.
//! - **The turn says where it came from.** Every turn carries a correspondent
//!   naming this surface and the editor that wrote it, so the record can tell
//!   an editor's turn from a person's in the page.
//! - **What needs a person is asked of the person who is there.** A profile
//!   that asks every time asks the editor, over ACP's own permission request,
//!   and the person answers where they are working. Holding the question for
//!   the page instead would mean an agent stops at its first question and the
//!   only surface that can release it is the one nobody has open.
//!
//! # One conversation, not one per launch
//!
//! The id this door gives an editor is the lane's own id in the record, not a
//! handle that dies with the process. An editor that kept it — which is what
//! ACP clients do — can hand it back with `session/load` and get the same
//! conversation: what was said is replayed to it, and what it says next lands
//! on the same lane the page is reading. The alternative is what this door did
//! before: a new conversation every time the editor starts, and a record that
//! grows a lane per launch with nothing tying them together.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, CancelNotification, CloseSessionRequest, CloseSessionResponse, ContentBlock,
    ContentChunk, FileSystemCapabilities, Implementation, InitializeRequest, InitializeResponse,
    ListSessionsRequest, ListSessionsResponse, LoadSessionRequest, LoadSessionResponse,
    NewSessionRequest, NewSessionResponse, PermissionOptionKind, PromptRequest, PromptResponse,
    ReadTextFileRequest, RequestPermissionOutcome, RequestPermissionRequest, SessionCapabilities,
    SessionCloseCapabilities, SessionId, SessionInfo, SessionListCapabilities, SessionNotification,
    SessionUpdate, StopReason, TextContent, WriteTextFileRequest,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};

use crate::workbench_shell::{ShellConnectionMode, WorkbenchShellState};
use crate::{NativeFileAnswer, NativeFileCall};

/// The surface name an editor's turns are recorded under. One name for every
/// editor: what distinguishes them is the correspondent's author, the way two
/// people in one channel are two authors on one surface.
pub const EDITOR_SURFACE: &str = "editor";

/// How long the update pump waits on the ledger before looking again.
const PUMP_WAIT: Duration = Duration::from_millis(250);
/// How many of a turn's updates to carry at once.
const PUMP_BATCH: usize = 64;

/// One conversation this door is holding for the editor connected to it.
#[derive(Clone)]
struct Open {
    connection: String,
    route: String,
    /// This editor session's own cursor on the route, so its updates are read
    /// at its own pace and not at the page's.
    surface: String,
}

#[derive(Clone, Default)]
struct Door {
    /// Every conversation this editor has open, by the id it knows each one
    /// by. An editor is one process with several windows in it, and ACP says
    /// a client may hold as many sessions on a connection as it likes; a door
    /// that kept only the last one answered a prompt about the first with
    /// "this prompt names no session this door opened".
    open: Arc<Mutex<BTreeMap<String, Open>>>,
    /// What the editor called itself at `initialize`. The author of every turn
    /// that comes through here.
    client: Arc<Mutex<Option<String>>>,
    /// What the editor said at `initialize` it can do with files. Nothing
    /// until it says so: an agent underneath is told it may read the person's
    /// files only when the editor holding them offered to be asked.
    files: Arc<Mutex<FileSystemCapabilities>>,
}

impl Door {
    /// Remember a conversation under the id the editor will name it by.
    fn remember(&self, session: &SessionId, open: Open) -> Result<(), Error> {
        self.open
            .lock()
            .map_err(|_| refuse("the editor door lost its lock"))?
            .insert(session.0.to_string(), open);
        Ok(())
    }

    /// The conversation an editor's request names, or nothing.
    fn find(&self, session: &SessionId) -> Result<Option<Open>, Error> {
        Ok(self
            .open
            .lock()
            .map_err(|_| refuse("the editor door lost its lock"))?
            .get(session.0.as_ref())
            .cloned())
    }

    /// What this editor offered to do with files, for a connection about to
    /// be opened. Nothing offered is `None`, which is the refusal every other
    /// door of this host gives: a connection is never told the surface behind
    /// it has files it never claimed.
    fn file_callbacks(&self) -> Result<Option<FileSystemCapabilities>, Error> {
        let offered = self
            .files
            .lock()
            .map_err(|_| refuse("the editor door lost its lock"))?
            .clone();
        Ok((offered.read_text_file || offered.write_text_file).then_some(offered))
    }

    /// Stop holding a conversation, because the editor said it is done with
    /// it. The lane stays in the record and can be loaded again.
    fn forget(&self, session: &SessionId) -> Result<Option<Open>, Error> {
        Ok(self
            .open
            .lock()
            .map_err(|_| refuse("the editor door lost its lock"))?
            .remove(session.0.as_ref()))
    }
}

fn refuse(message: impl Into<String>) -> Error {
    Error::internal_error().data(serde_json::json!({ "reason": message.into() }))
}

/// Serve one editor on this process's stdio until it goes away.
///
/// # Errors
///
/// Returns the protocol error when the editor's side of the connection fails.
/// A refusal inside a request (an unknown profile, a session that was never
/// opened) is answered to the editor rather than ending the connection.
#[allow(
    clippy::too_many_lines,
    reason = "one function per door: initialize, new, prompt and cancel read as the protocol they answer"
)]
pub async fn serve_editor_door(
    state: Arc<WorkbenchShellState>,
    profile_id: String,
) -> Result<(), Error> {
    let door = Door::default();
    let initializing = door.clone();
    let opening = door.clone();
    let loading = door.clone();
    let closing = door.clone();
    let closing_state = Arc::clone(&state);
    let listing_state = Arc::clone(&state);
    let listing_profile = profile_id.clone();
    let prompting = door.clone();
    let cancelling = door.clone();
    let open_state = Arc::clone(&state);
    let load_state = Arc::clone(&state);
    let prompt_state = Arc::clone(&state);
    let cancel_state = Arc::clone(&state);
    let open_profile = profile_id.clone();
    let load_profile = profile_id.clone();

    Agent
        .builder()
        .name("swem")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                if let Some(name) = request.client_info.as_ref().map(|info| info.name.clone()) {
                    *initializing
                        .client
                        .lock()
                        .map_err(|_| refuse("the editor door lost its lock"))? = Some(name);
                }
                // The editor's own `fs/*` capability, kept for the
                // connections this door opens after it. It is the editor that
                // has the file the person is looking at - unsaved buffer and
                // all - so this is the one door of this host where an agent's
                // file callback has somewhere true to go.
                *initializing
                    .files
                    .lock()
                    .map_err(|_| refuse("the editor door lost its lock"))? =
                    request.client_capabilities.fs.clone();
                // Deliberately modest: this door claims no rich content of its
                // own. What the profile's agent can take is the profile's
                // agent's business, and the harness already refuses content an
                // agent never said it could take.
                responder.respond(
                    InitializeResponse::new(ProtocolVersion::V1)
                        // What this door claims, and no more: an editor may
                        // hand back a session id it kept and get that
                        // conversation rather than a new one, and it may ask
                        // what conversations there are when it kept none.
                        .agent_capabilities(
                            AgentCapabilities::new()
                                .load_session(true)
                                .session_capabilities(
                                    SessionCapabilities::new()
                                        .list(SessionListCapabilities::new())
                                        .close(SessionCloseCapabilities::new()),
                                ),
                        )
                        // What the editor shows the person it is talking to.
                        // It is SWEM, not the agent underneath: the profile
                        // chooses that, and it can change without the editor
                        // reconnecting.
                        .agent_info(Implementation::new("swem", env!("CARGO_PKG_VERSION"))),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, connection: ConnectionTo<Client>| {
                let (connection_id, route_id, _) = open_state
                    .open_connection_with_id(
                        &open_profile,
                        ShellConnectionMode::New,
                        None,
                        None,
                        None,
                        opening.file_callbacks()?,
                    )
                    .await
                    .map_err(|error| refuse(error.to_string()))?;
                // The lane's id, not this process's handle on it. An editor
                // keeps what `session/new` returns and offers it back on its
                // next launch; a handle that dies with the process would make
                // every launch a new conversation.
                let session_id = SessionId::new(route_id.clone());
                let surface_id = format!("{EDITOR_SURFACE}:{connection_id}");
                let workspace = workspace_of(&open_state, &open_profile)?;
                opening.remember(
                    &session_id,
                    Open {
                        connection: connection_id,
                        route: route_id,
                        surface: surface_id,
                    },
                )?;
                // The editor asked for a directory. It does not get to choose
                // one: the profile's boundary is what the harness enforces, so
                // the person is told where their agent actually is instead of
                // the harness quietly widening it.
                if elsewhere(&request.cwd, &workspace) {
                    connection.send_notification(boundary_note(
                        &session_id,
                        &workspace,
                        &request.cwd,
                    ))?;
                }
                responder.respond(NewSessionResponse::new(session_id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CloseSessionRequest, responder, _connection| {
                // A window closing is not the end of a conversation: the lane
                // stays in the record and loads again. What ends is this
                // door's hold on it, and with it the agent process it was
                // running - which, before this, stayed alive until the whole
                // editor exited, one per window a person had ever opened.
                let Some(open) = closing.forget(&request.session_id)? else {
                    return Err(refuse("this close names no session this door opened"));
                };
                closing_state
                    .disconnect(&open.connection)
                    .await
                    .map_err(|error| refuse(error.to_string()))?;
                responder.respond(CloseSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ListSessionsRequest, responder, _connection| {
                let workspace = workspace_of(&listing_state, &listing_profile)?;
                // Every conversation of this profile lives in the profile's
                // own directory, so a filter naming another one matches
                // nothing. That is the same answer this door gives a
                // `session/new` that claims a directory: the editor's
                // directory is not where this agent works.
                if request
                    .cwd
                    .is_some_and(|asked| elsewhere(&asked, &workspace))
                {
                    return responder.respond(ListSessionsResponse::new(Vec::new()));
                }
                let sessions = listing_state
                    .profile_sessions(&listing_profile)
                    .await
                    .map_err(|error| refuse(error.to_string()))?
                    .into_iter()
                    .map(|session| {
                        let info = SessionInfo::new(session.route_id, workspace.clone());
                        // What was said first, which is how a person finds the
                        // conversation they mean. ACP's `updatedAt` is left
                        // unset rather than invented: the ledger records the
                        // order of what happened and not the hour of it.
                        match session.opened_with {
                            Some(title) => info.title(title),
                            None => info,
                        }
                    })
                    .collect();
                responder.respond(ListSessionsResponse::new(sessions))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest,
                        responder,
                        connection: ConnectionTo<Client>| {
                // The id the editor kept is the lane's id, so continuing is
                // resuming that lane - the same thing the clock does when a
                // schedule fires again, and the same thing the page does when
                // a person opens a session they had.
                let route_id = request.session_id.0.to_string();
                let (connection_id, route_id, _) = load_state
                    .open_connection_with_id(
                        &load_profile,
                        ShellConnectionMode::Resume,
                        Some(route_id),
                        None,
                        None,
                        loading.file_callbacks()?,
                    )
                    .await
                    .map_err(|error| refuse(error.to_string()))?;
                let surface_id = format!("{EDITOR_SURFACE}:{connection_id}");
                let workspace = workspace_of(&load_state, &load_profile)?;
                loading.remember(
                    &request.session_id,
                    Open {
                        connection: connection_id,
                        route: route_id.clone(),
                        surface: surface_id.clone(),
                    },
                )?;
                if elsewhere(&request.cwd, &workspace) {
                    connection.send_notification(boundary_note(
                        &request.session_id,
                        &workspace,
                        &request.cwd,
                    ))?;
                }
                // ACP's own answer to "what was said here": the agent replays
                // the conversation as updates before it responds, so the
                // editor draws the history it would have had if it had never
                // been closed.
                replay(
                    &load_state,
                    &connection,
                    &request.session_id,
                    &route_id,
                    &surface_id,
                )
                .await?;
                responder.respond(LoadSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                let open = prompting
                    .find(&request.session_id)?
                    .ok_or_else(|| refuse("this prompt names no session this door opened"))?;
                let Open {
                    connection: connection_id,
                    route: route_id,
                    surface: surface_id,
                } = open;
                let author = prompting
                    .client
                    .lock()
                    .map_err(|_| refuse("the editor door lost its lock"))?
                    .clone();

                // The turn's updates, carried out to the editor while the turn
                // is still running. Without this the editor sits silent for as
                // long as the agent takes.
                let pump_state = Arc::clone(&prompt_state);
                let pump_connection = connection.clone();
                let pump_session = request.session_id.clone();
                let pump = tokio::spawn(async move {
                    carry_updates(
                        pump_state,
                        pump_connection,
                        pump_session,
                        route_id,
                        surface_id,
                    )
                    .await;
                });

                // And the questions the agent stops on. A profile that asks
                // every time is the default, so without this the first thing
                // an agent asks for ends the editor's turn in silence: the
                // question waits for a surface the person does not have open.
                let asking_state = Arc::clone(&prompt_state);
                let asking_connection = connection.clone();
                let asking_session = request.session_id.clone();
                let asking_connection_id = connection_id.clone();
                let asking = tokio::spawn(async move {
                    carry_questions(
                        asking_state,
                        asking_connection,
                        asking_session,
                        asking_connection_id,
                    )
                    .await;
                });

                // And the files the agent reaches for. Same reason and same
                // shape as the questions: the editor is where the file is,
                // and a callback answered from a request handler would stop
                // this connection dispatching the answer it is waiting for.
                let reading_state = Arc::clone(&prompt_state);
                let reading_connection = connection.clone();
                let reading_session = request.session_id.clone();
                let reading_connection_id = connection_id.clone();
                let reading = tokio::spawn(async move {
                    carry_files(
                        reading_state,
                        reading_connection,
                        reading_session,
                        reading_connection_id,
                    )
                    .await;
                });

                // The turn runs in a task of its own, and this handler
                // returns at once. Waiting for the turn here would stop this
                // connection from reading the editor's own messages while it
                // ran - and the answer to a permission request is one of
                // them, so an agent that asked anything would never hear the
                // reply and the turn would never end.
                let turn_state = Arc::clone(&prompt_state);
                connection.spawn(async move {
                    let outcome = turn_state
                        .submit_prompt_from(
                            &connection_id,
                            request.prompt.clone(),
                            Some(crate::Correspondent {
                                surface: EDITOR_SURFACE.to_owned(),
                                author,
                                addressed_to: None,
                            }),
                        )
                        .await;
                    // One last look before the pump stops, so the end of the
                    // turn is not the one batch that never arrives.
                    tokio::time::sleep(PUMP_WAIT * 2).await;
                    pump.abort();
                    asking.abort();
                    reading.abort();

                    let outcome = outcome.map_err(|error| refuse(error.to_string()))?;
                    responder.respond(PromptResponse::new(stop_reason(&outcome)))
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _cx| {
                let connection_id = cancelling
                    .find(&notification.session_id)?
                    .map(|open| open.connection);
                if let Some(connection_id) = connection_id {
                    cancel_state
                        .cancel(&connection_id)
                        .await
                        .map_err(|error| refuse(error.to_string()))?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
}

/// Where this profile's agent works. Both doors of a session need it: the
/// editor names a directory of its own and is told about this one instead.
fn workspace_of(state: &Arc<WorkbenchShellState>, profile_id: &str) -> Result<PathBuf, Error> {
    state
        .profiles()
        .map_err(|error| refuse(error.to_string()))?
        .into_iter()
        .find(|profile| profile.profile_id == profile_id)
        .map(|profile| profile.workspace)
        .ok_or_else(|| refuse(format!("unknown profile {profile_id}")))
}

/// What the person is told when their editor opened one directory and their
/// profile works in another. It is said, not obeyed.
fn boundary_note(
    session_id: &SessionId,
    workspace: &std::path::Path,
    asked: &std::path::Path,
) -> SessionNotification {
    SessionNotification::new(
        session_id.clone(),
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            format!(
                "This agent works in {}, which is where SWEM keeps its files. \
                 The directory your editor opened ({}) is not what it can reach.",
                workspace.display(),
                asked.display(),
            ),
        )))),
    )
}

/// How much of a lane a loading editor is given. Enough that a person reads
/// what they were doing; a lane longer than this is a lane whose beginning is
/// not what they came back for.
const REPLAY_LIMIT: usize = 500;

/// Replay a lane to an editor that has just loaded it, and leave its cursor
/// past what it has seen.
///
/// Both kinds of thing said on a lane are replayed, because both are what the
/// conversation was: the agent's own recorded updates, and the turns people
/// wrote into it — from this editor, from the page, or from the clock. Without
/// the second kind an editor would redraw the agent talking to nobody.
///
/// The cursor moves to the end afterwards. This surface is new (its id carries
/// this connection), so the live pump would otherwise start at zero and send
/// the whole conversation a second time, on top of the replay.
async fn replay(
    state: &Arc<WorkbenchShellState>,
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    route_id: &str,
    surface_id: &str,
) -> Result<(), Error> {
    let events = state
        .route_history(route_id, REPLAY_LIMIT)
        .await
        .map_err(|error| refuse(error.to_string()))?;
    let mut last = 0;
    for event in &events {
        last = event.sequence;
        let update = match event.kind.as_str() {
            "acp/session_update" => {
                serde_json::from_value::<SessionNotification>(event.payload.clone())
                    .ok()
                    .map(|mut update| {
                        update.session_id = session_id.clone();
                        update
                    })
            }
            // A turn someone wrote. The provenance line the harness appended
            // is part of the text, so the editor sees who wrote it for the
            // same reason the agent did.
            "host/turn_written" => event
                .payload
                .get("text")
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(|text| {
                    SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(text.to_owned()),
                        ))),
                    )
                }),
            _ => None,
        };
        if let Some(update) = update {
            connection.send_notification(update)?;
        }
    }
    if last > 0 {
        state
            .acknowledge(route_id, surface_id, last)
            .await
            .map_err(|error| refuse(error.to_string()))?;
    }
    Ok(())
}

/// True when the editor opened somewhere the profile's agent cannot reach.
fn elsewhere(asked: &std::path::Path, workspace: &std::path::Path) -> bool {
    let resolve = |path: &std::path::Path| -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    };
    !resolve(asked).starts_with(resolve(workspace))
}

/// Read this editor session's own events off the route and send each session
/// update on to the editor, until the task is dropped.
///
/// The events are the agent's own ACP updates, recorded on the lane as they
/// happened, so what the editor sees is what the page would have seen — the
/// same turn, read at this surface's own pace.
async fn carry_updates(
    state: Arc<WorkbenchShellState>,
    connection: ConnectionTo<Client>,
    session_id: SessionId,
    route_id: String,
    surface_id: String,
) {
    loop {
        let Ok(batch) = state
            .events(&route_id, &surface_id, PUMP_BATCH, PUMP_WAIT)
            .await
        else {
            return;
        };
        let cursor = batch.next_cursor();
        for event in &batch.events {
            if event.kind != "acp/session_update" {
                continue;
            }
            let Ok(mut update) =
                serde_json::from_value::<SessionNotification>(event.payload.clone())
            else {
                continue;
            };
            // The lane's session id is the agent's own. The editor knows this
            // conversation by the id this door gave it.
            update.session_id = session_id.clone();
            if connection.send_notification(update).is_err() {
                return;
            }
        }
        if cursor > 0
            && state
                .acknowledge(&route_id, &surface_id, cursor)
                .await
                .is_err()
        {
            return;
        }
    }
}

/// Ask the editor what the agent stopped on, and tell the session what the
/// person answered, until the task is dropped.
///
/// The editor is the surface the person is looking at, so this is where ACP's
/// own `session/request_permission` belongs: the same question the page shows,
/// asked of whoever is actually there.
async fn carry_questions(
    state: Arc<WorkbenchShellState>,
    connection: ConnectionTo<Client>,
    session_id: SessionId,
    connection_id: String,
) {
    let mut answered = 0;
    loop {
        let Ok(question) = state
            .permission_after(&connection_id, answered, PUMP_WAIT)
            .await
        else {
            return;
        };
        let Some(question) = question else {
            continue;
        };
        let options = question.options.clone();
        let asked = connection
            .send_request(RequestPermissionRequest::new(
                session_id.clone(),
                question.tool_call.clone(),
                options.clone(),
            ))
            // Safe here and nowhere else in this file: this runs in a task of
            // its own, not in a request handler, so waiting for the person's
            // answer does not stop the connection from dispatching.
            .block_task()
            .await;
        // An editor that cannot be asked is not an editor that said yes. The
        // safe answer is the agent's own refusal, so the turn ends with the
        // thing not done rather than waiting on a surface nobody has open -
        // and if the agent offered no way to refuse, the question is left for
        // the page, where the person can still answer it.
        let chosen = match asked.map(|answer| answer.outcome) {
            Ok(RequestPermissionOutcome::Selected(selected)) => selected.option_id.0.to_string(),
            Ok(RequestPermissionOutcome::Cancelled) => {
                let _ = state.cancel(&connection_id).await;
                return;
            }
            _ => match refusal(&options) {
                Some(refusal) => refusal,
                None => return,
            },
        };
        if state
            .select_permission(&connection_id, question.sequence, &chosen)
            .await
            .is_err()
        {
            return;
        }
        answered = question.sequence;
    }
}

/// Ask the editor for the files the agent reaches for, and tell the session
/// what came back, until the task is dropped.
///
/// This is the one door of this host where `fs/*` has somewhere true to go.
/// ACP puts those methods on the client because they are operations in the
/// client's environment, and the Workbench's client is a browser tab that has
/// none. An editor's does: the file the person is looking at is in its
/// buffer, unsaved edits and all, and reading the same path off the disk
/// would hand the agent a different file from the one on the screen.
///
/// The paths are already the profile's: the host checked every one against
/// the workspace before offering it here, so what this carries out is only
/// what the harness was willing to ask for in the first place.
async fn carry_files(
    state: Arc<WorkbenchShellState>,
    connection: ConnectionTo<Client>,
    session_id: SessionId,
    connection_id: String,
) {
    let mut answered = 0;
    loop {
        let Ok(asked) = state
            .file_request_after(&connection_id, answered, PUMP_WAIT)
            .await
        else {
            return;
        };
        let Some(asked) = asked else {
            continue;
        };
        let answer = ask_editor(&connection, &session_id, &asked.call).await;
        if state
            .answer_file_request(&connection_id, asked.sequence, answer)
            .await
            .is_err()
        {
            return;
        }
        answered = asked.sequence;
    }
}

/// One file call, put to the editor and brought back.
///
/// An editor that cannot be reached, or that says no, is not an editor that
/// handed over a file: the agent is told so and its turn goes on knowing the
/// file was not read, which is something it can act on. Silence is what it
/// would get instead, and silence costs it the whole turn.
async fn ask_editor(
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    call: &NativeFileCall,
) -> NativeFileAnswer {
    match call.method.as_str() {
        "fs/read_text_file" => {
            let mut request = ReadTextFileRequest::new(session_id.clone(), call.path.clone());
            request.line = call.line;
            request.limit = call.limit;
            // Safe here for the same reason the questions are: this runs in a
            // task of its own, not in a request handler, so waiting for the
            // editor does not stop the connection dispatching its answer.
            match connection.send_request(request).block_task().await {
                Ok(answer) => NativeFileAnswer::Text(answer.content),
                Err(error) => NativeFileAnswer::Refused(error.to_string()),
            }
        }
        "fs/write_text_file" => {
            // A write with nothing to write is refused rather than turned
            // into an empty file. The host always carries the content; if it
            // ever does not, the person keeps their file.
            let Some(content) = call.content.clone() else {
                return NativeFileAnswer::Refused(
                    "a write reached this door with no content to write".to_owned(),
                );
            };
            let request = WriteTextFileRequest::new(session_id.clone(), call.path.clone(), content);
            match connection.send_request(request).block_task().await {
                Ok(_) => NativeFileAnswer::Written,
                Err(error) => NativeFileAnswer::Refused(error.to_string()),
            }
        }
        other => NativeFileAnswer::Refused(format!("this door does not carry `{other}`")),
    }
}

/// The agent's own way of saying no, when it offered one.
fn refusal(options: &[agent_client_protocol::schema::v1::PermissionOption]) -> Option<String> {
    options
        .iter()
        .find(|option| {
            matches!(
                option.kind,
                PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
            )
        })
        .map(|option| option.option_id.0.to_string())
}

/// How the turn ended, in the editor's vocabulary.
fn stop_reason(outcome: &crate::NativeTurnOutcome) -> StopReason {
    match outcome.stop_reason.as_str() {
        "cancelled" => StopReason::Cancelled,
        "max_tokens" => StopReason::MaxTokens,
        "max_turn_requests" => StopReason::MaxTurnRequests,
        "refusal" => StopReason::Refusal,
        _ => StopReason::EndTurn,
    }
}

#[cfg(test)]
mod tests {
    use super::elsewhere;

    #[test]
    fn the_editors_own_directory_does_not_move_the_profiles_boundary() {
        let workspace = std::path::Path::new("/tmp/swem-profile-workspace");
        assert!(
            elsewhere(std::path::Path::new("/etc"), workspace),
            "an editor opened elsewhere must be told so"
        );
        assert!(
            !elsewhere(workspace, workspace),
            "the profile's own directory is not elsewhere"
        );
        assert!(
            !elsewhere(&workspace.join("inside"), workspace),
            "a directory inside the workspace is not elsewhere"
        );
    }
}
