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
//! - **The message says where it came from.** What is written here is said
//!   in the chat through the editor's channel, so the record can tell what a
//!   person wrote in their editor from what they wrote in the page.
//! - **What needs a person is asked of the person who is there.** A profile
//!   that asks every time asks the editor, over ACP's own permission request,
//!   and the person answers where they are working. Holding the question for
//!   the page instead would mean an agent stops at its first question and the
//!   only surface that can release it is the one nobody has open.
//!
//! # One conversation, not one per launch
//!
//! The id this door gives an editor is the chat's own id in the record, not a
//! handle that dies with the process, and not the id of a session of the
//! engine, which the chat outlives. An editor that kept it — which is what
//! ACP clients do — can hand it back with `session/load` and get the same
//! conversation: what was said is replayed to it, and what it says next is
//! said in the same chat the page is reading.
//!
//! # The door carries out what it said
//!
//! What an editor says is a message in a chat like any other, owed to the
//! chat's agent like any other. It is this process that pays the debt, not
//! the Workbench beside it: the editor has the file the person is looking
//! at, and is where the person answers what the agent asks. The agent's turn
//! lock is what keeps the two processes from giving one agent two turns.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
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

use crate::workbench_shell::{Saying, StartChatBody, WorkbenchShellState};
use crate::{
    CHANNEL_EDITOR, Chat, DeliveryState, NativeFileAnswer, NativeFileCall, ParticipantKind,
    SurfaceEventSource,
};

/// The channel what an editor says comes through. One name for every editor.
pub const EDITOR_SURFACE: &str = CHANNEL_EDITOR;

/// How long the update pump waits on the ledger before looking again.
const PUMP_WAIT: Duration = Duration::from_millis(250);
/// How many of a turn's updates to carry at once.
const PUMP_BATCH: usize = 64;

/// One conversation this door is holding for the editor connected to it.
#[derive(Clone)]
struct Open {
    chat: String,
    /// How far this editor has been told what happened in the chat.
    told_through: Arc<AtomicU64>,
}

#[derive(Clone, Default)]
struct Door {
    /// Every conversation this editor has open, by the id it knows each one
    /// by. An editor is one process with several windows in it, and ACP says
    /// a client may hold as many sessions on a connection as it likes; a door
    /// that kept only the last one answered a prompt about the first with
    /// "this prompt names no session this door opened".
    open: Arc<Mutex<BTreeMap<String, Open>>>,
    /// What the editor called itself at `initialize`. Part of the name of
    /// every message that comes through here.
    client: Arc<Mutex<Option<String>>>,
    /// How many messages came through here.
    said: Arc<AtomicU64>,
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

    /// Stop holding a conversation, because the editor said it is done with
    /// it. The chat stays in the record and can be loaded again.
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
    // This process carries out what its editor says, and only that.
    let name = state
        .carry_for_an_editor()
        .map_err(|error| refuse(error.to_string()))?;
    state
        .take_up_chats()
        .await
        .map_err(|error| refuse(error.to_string()))?;
    let door = Door::default();
    let initializing = door.clone();
    let initial_state = Arc::clone(&state);
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
                // The editor's own `fs/*` capability, kept for the sessions
                // this door opens after it. It is the editor that has the
                // file the person is looking at - unsaved buffer and all -
                // so this is the one door of this host where an agent's
                // file callback has somewhere true to go. Nothing offered
                // is nothing claimed.
                let offered: FileSystemCapabilities = request.client_capabilities.fs.clone();
                initial_state.editor_offers_files(
                    (offered.read_text_file || offered.write_text_file).then_some(offered),
                );
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
                // A chat, and no engine yet: the engine is started by the
                // first thing said. The id is the chat's, so an editor that
                // keeps it comes back to the conversation whatever session
                // of the engine the chat is in by then.
                let chat = open_state
                    .start_chat(StartChatBody {
                        title: String::new(),
                        agents: vec![open_profile.clone()],
                    })
                    .await
                    .map_err(|error| refuse(error.to_string()))?;
                let session_id = SessionId::new(chat.chat_id.clone());
                let workspace = workspace_of(&open_state, &open_profile)?;
                opening.remember(
                    &session_id,
                    Open {
                        told_through: Arc::new(AtomicU64::new(chat.last_sequence.unwrap_or(0))),
                        chat: chat.chat_id,
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
                // A window closing is not the end of a conversation: the
                // chat stays in the record and loads again. What ends is
                // this door's hold on it, and with it the agent process it
                // was running - which otherwise stays alive for every
                // window a person had ever opened.
                let Some(open) = closing.forget(&request.session_id)? else {
                    return Err(refuse("this close names no session this door opened"));
                };
                closing_state.let_go_of_chat(&open.chat).await;
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
                    .chats()
                    .await
                    .map_err(|error| refuse(error.to_string()))?
                    .into_iter()
                    .filter(|chat| has_agent(chat, &listing_profile))
                    .map(|chat| {
                        let info = SessionInfo::new(chat.chat_id, workspace.clone());
                        // What it is called, or what was said first, which
                        // is how a person finds the conversation they mean.
                        if chat.title.is_empty() {
                            info
                        } else {
                            info.title(chat.title)
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
                // The id the editor kept is the chat's - or, from before
                // there were chats, the id of the engine's session the
                // chat holds. No engine is started by looking.
                let chat = load_state
                    .chat_named(request.session_id.0.as_ref())
                    .await
                    .map_err(|error| refuse(error.to_string()))?;
                if !has_agent(&chat, &load_profile) {
                    return Err(refuse(format!(
                        "this conversation belongs to another agent, not to {load_profile}"
                    )));
                }
                let workspace = workspace_of(&load_state, &load_profile)?;
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
                let told_through =
                    replay(&load_state, &connection, &request.session_id, &chat).await?;
                loading.remember(
                    &request.session_id,
                    Open {
                        chat: chat.chat_id,
                        told_through: Arc::new(AtomicU64::new(told_through)),
                    },
                )?;
                responder.respond(LoadSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                let open = prompting
                    .find(&request.session_id)?
                    .ok_or_else(|| refuse("this prompt names no session this door opened"))?;
                let editor = prompting
                    .client
                    .lock()
                    .map_err(|_| refuse("the editor door lost its lock"))?
                    .clone()
                    .unwrap_or_default();
                let blocks = request
                    .prompt
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| refuse(error.to_string()))?;
                // Said in the chat, as the person this Workbench is, through
                // the editor's channel. The name of the message begins with
                // this door's, which is what makes the debt this door's to
                // pay.
                let said = prompt_state
                    .say_in_chat_as(
                        &open.chat,
                        None,
                        CHANNEL_EDITOR,
                        Saying {
                            text: String::new(),
                            blocks,
                            content_refs: Vec::new(),
                            client_ref: Some(format!(
                                "{name}:{}:{editor}",
                                prompting.said.fetch_add(1, Ordering::SeqCst)
                            )),
                        },
                    )
                    .await
                    .map_err(|error| refuse(error.to_string()))?;
                let Some(owed) = said.deliveries.first().cloned() else {
                    return Err(refuse(
                        "there are several agents in this chat; name the one this is for",
                    ));
                };
                open.told_through
                    .fetch_max(said.message.sequence, Ordering::SeqCst);

                // What happens in the chat, carried out to the editor while
                // the turn is still running. Without this the editor sits
                // silent for as long as the agent takes.
                let pump = tokio::spawn(carry_updates(
                    Arc::clone(&prompt_state),
                    connection.clone(),
                    request.session_id.clone(),
                    open.clone(),
                ));
                // And the questions the agent stops on, and the files it
                // reaches for: the editor is where the person is and where
                // the file is. Both wait in tasks of their own, because an
                // answer awaited in a request handler would stop this
                // connection dispatching the answer it is waiting for.
                let asking = tokio::spawn(carry_questions(
                    Arc::clone(&prompt_state),
                    connection.clone(),
                    request.session_id.clone(),
                    owed.delivery_id.clone(),
                ));
                let reading = tokio::spawn(carry_files(
                    Arc::clone(&prompt_state),
                    connection.clone(),
                    request.session_id.clone(),
                    owed.delivery_id.clone(),
                ));

                // The turn is followed in a task of its own, and this
                // handler returns at once. Waiting for the turn here would
                // stop this connection from reading the editor's own
                // messages while it ran - and the answer to a permission
                // request is one of them.
                let turn_state = Arc::clone(&prompt_state);
                let last_connection = connection.clone();
                let session_id = request.session_id.clone();
                connection.spawn(async move {
                    let ended = turn_state.ended(&owed.delivery_id).await;
                    pump.abort();
                    asking.abort();
                    reading.abort();
                    // One last look, so the end of the turn is not the one
                    // batch that never arrives.
                    carry_what_happened(&turn_state, &last_connection, &session_id, &open).await;
                    let ended = ended.map_err(|error| refuse(error.to_string()))?;
                    match ended.state {
                        DeliveryState::Done => responder.respond(PromptResponse::new(stop_reason(
                            ended.outcome.as_deref().unwrap_or("end_turn"),
                        ))),
                        DeliveryState::Stopped => {
                            responder.respond(PromptResponse::new(StopReason::Cancelled))
                        }
                        _ => Err(refuse(ended.outcome.unwrap_or_else(|| {
                            "the turn did not come to its end".to_owned()
                        }))),
                    }
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _cx| {
                if let Some(open) = cancelling.find(&notification.session_id)? {
                    cancel_state
                        .stop_in_chat(&open.chat, None)
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

/// Whether the agent of a profile is in a chat.
fn has_agent(chat: &Chat, profile_id: &str) -> bool {
    chat.members.iter().any(|member| {
        member.kind == ParticipantKind::Agent && member.profile_id.as_deref() == Some(profile_id)
    })
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

/// How much of a chat a loading editor is given. Enough that a person reads
/// what they were doing; a chat longer than this is one whose beginning is
/// not what they came back for.
const REPLAY_LIMIT: usize = 500;

/// Replay a chat to an editor that has just loaded it, and say how far it
/// has been told.
///
/// Both kinds of thing said in a chat are replayed, because both are what
/// the conversation was: the agents' own recorded updates, and what people
/// and the clock said - from this editor, from the page, or on time.
/// Without the second kind an editor would redraw the agent talking to
/// nobody. What an engine replayed when its session was loaded is the same
/// words a second time and is left out.
async fn replay(
    state: &Arc<WorkbenchShellState>,
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    chat: &Chat,
) -> Result<u64, Error> {
    let page = state
        .chat_page(&chat.chat_id, None, REPLAY_LIMIT)
        .await
        .map_err(|error| refuse(error.to_string()))?;
    let agents: Vec<&str> = page
        .chat
        .members
        .iter()
        .filter(|member| member.kind == ParticipantKind::Agent)
        .map(|member| member.participant_id.as_str())
        .collect();
    let mut told_through = 0;
    for event in &page.events {
        told_through = event.sequence;
        let said = page
            .messages
            .iter()
            .find(|message| message.sequence == event.sequence)
            .filter(|message| !agents.contains(&message.sender_id.as_str()))
            .filter(|message| !message.text.trim().is_empty());
        let update = if let Some(said) = said {
            Some(SessionNotification::new(
                session_id.clone(),
                SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                    TextContent::new(said.text.clone()),
                ))),
            ))
        } else {
            update_of(event, session_id)
        };
        if let Some(update) = update {
            connection.send_notification(update)?;
        }
    }
    Ok(told_through)
}

/// The engine's own update an event of a chat is, addressed to the
/// conversation as the editor knows it.
fn update_of(event: &crate::ChatEvent, session_id: &SessionId) -> Option<SessionNotification> {
    if event.kind != "acp/session_update" || event.source != SurfaceEventSource::NativeLive {
        return None;
    }
    let mut update = serde_json::from_value::<SessionNotification>(event.payload.clone()).ok()?;
    // The event's session id is the engine's own. The editor knows this
    // conversation by the id this door gave it.
    update.session_id = session_id.clone();
    Some(update)
}

/// True when the editor opened somewhere the profile's agent cannot reach.
fn elsewhere(asked: &std::path::Path, workspace: &std::path::Path) -> bool {
    let resolve = |path: &std::path::Path| -> PathBuf {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    };
    !resolve(asked).starts_with(resolve(workspace))
}

/// Tell the editor what happened in the chat since it was last told.
/// Returns whether it could be told.
async fn carry_what_happened(
    state: &Arc<WorkbenchShellState>,
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    open: &Open,
) -> bool {
    loop {
        let after = open.told_through.load(Ordering::SeqCst);
        let Ok(happened) = state.happened_in_chat(&open.chat, after, PUMP_BATCH).await else {
            return false;
        };
        let Some(last) = happened.last() else {
            return true;
        };
        let last = last.sequence;
        for event in &happened {
            if let Some(update) = update_of(event, session_id)
                && connection.send_notification(update).is_err()
            {
                return false;
            }
        }
        open.told_through.fetch_max(last, Ordering::SeqCst);
        if happened.len() < PUMP_BATCH {
            return true;
        }
    }
}

/// Carry what happens in the chat to the editor, until the task is dropped.
///
/// The events are the engine's own ACP updates, recorded as they happened,
/// so what the editor sees is what the page sees - the same turn, read at
/// this editor's own pace.
async fn carry_updates(
    state: Arc<WorkbenchShellState>,
    connection: ConnectionTo<Client>,
    session_id: SessionId,
    open: Open,
) {
    while carry_what_happened(&state, &connection, &session_id, &open).await {
        tokio::time::sleep(PUMP_WAIT).await;
    }
}

/// The connection a delivery runs on, once it runs; nothing when it ended
/// before it ran.
async fn connection_of(state: &Arc<WorkbenchShellState>, delivery_id: &str) -> Option<String> {
    loop {
        if let Some(connection_id) = state.connection_of(delivery_id).await {
            return Some(connection_id);
        }
        tokio::time::sleep(PUMP_WAIT).await;
    }
}

/// Ask the editor what the agent stopped on, and tell the session what the
/// person answered, until the task is dropped.
///
/// The editor is the surface the person is looking at, so this is where ACP's
/// own `session/request_permission` belongs: the same question the page shows,
/// asked of whoever is actually there. The question is kept in the chat as
/// well, so whoever answers first has answered.
async fn carry_questions(
    state: Arc<WorkbenchShellState>,
    connection: ConnectionTo<Client>,
    session_id: SessionId,
    delivery_id: String,
) {
    let Some(connection_id) = connection_of(&state, &delivery_id).await else {
        return;
    };
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
        // Somebody may have answered in the page meanwhile; then it is
        // answered, and this answer is nobody's.
        let _ = state
            .answer_where_it_runs(&delivery_id, question.sequence, &chosen)
            .await;
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
    delivery_id: String,
) {
    let Some(connection_id) = connection_of(&state, &delivery_id).await else {
        return;
    };
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
fn stop_reason(said: &str) -> StopReason {
    match said {
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
