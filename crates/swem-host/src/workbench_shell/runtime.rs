//! Carrying messages to agents.
//!
//! A message said in a chat is owed to the agents it is for (`chats`). Here
//! the debt is paid: one worker per agent takes what the agent is owed, one
//! message at a time, finds or opens the agent's session in that chat, gives
//! the turn with the host's block at its end, and records how it ended.
//!
//! - **One turn at a time per agent**, across processes: a worker holds the
//!   agent's turn lock, a file, while it claims and carries out a delivery.
//!   The editor door is another process over the same ledger.
//! - **A chat outlives its engine's session.** When the engine cannot go on
//!   with the session the chat holds, a fresh one is opened in the same chat
//!   and the first turn gives it what was said before.
//! - **A question waits in the ledger.** What an agent asks in its turn is
//!   kept there and answered from there; the connection only carries the
//!   answer back.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{
    ContentBlock, ElicitationAction, ElicitationMode, FileSystemCapabilities,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::chats::{ledger_refusal, recipients};
use super::{ChatPlace, Opening, ShellConnectionMode, WorkbenchShellError, WorkbenchShellState};
use crate::{
    Chat, ChatSession, Delivery, DeliveryState, Message, NativeSessionControl, NativeSessionPhase,
    NativeTurnControlOutcome, Participant, ParticipantKind, Question, QuestionState, Said, Speaker,
    Trust, Turn,
};

/// How often the ledger is asked whether a question has its answer.
const LOOK: Duration = Duration::from_millis(150);

/// How long an agent's sessions are kept open after its last turn.
const IDLE: Duration = Duration::from_mins(10);

/// How much of what was said before a fresh session is given in its first
/// turn; the whole of it is handed over as a file beside.
const HISTORY: crate::Fitting = crate::Fitting {
    count: 400,
    each: 4_000,
    all: 48_000,
};

/// Where a form or a link is answered: in the session that asked, in this
/// process. What is typed into a form is never written down, so it cannot
/// be answered through the ledger as a permission is.
#[derive(Clone, Debug)]
struct Asked {
    connection_id: String,
    delivery_id: Option<String>,
    sequence: u64,
}

/// Whose a question is: the chat and the agent it was asked in, and the
/// delivery it was asked for, when it was asked in a turn.
#[derive(Clone, Debug)]
struct AskedIn {
    chat: String,
    agent: String,
    delivery: Option<String>,
}

impl AskedIn {
    fn turn(delivery: &Delivery) -> Self {
        Self {
            chat: delivery.chat_id.clone(),
            agent: delivery.agent_id.clone(),
            delivery: Some(delivery.delivery_id.clone()),
        }
    }
}

/// What runs now.
#[derive(Clone, Debug)]
struct Running {
    chat: String,
    agent: String,
    connection: String,
}

/// What the chats of this process are doing.
#[derive(Default)]
pub(super) struct ChatRuntime {
    /// The agents that have a worker.
    at_work: std::sync::Mutex<BTreeSet<String>>,
    /// What wakes an agent's worker.
    wake: std::sync::Mutex<BTreeMap<String, Arc<Notify>>>,
    /// The connection an agent is live on in a chat, by chat and agent.
    live: tokio::sync::Mutex<BTreeMap<(String, String), String>>,
    /// The deliveries that run, by id.
    running: tokio::sync::Mutex<BTreeMap<String, Running>>,
    /// The forms and links that can be answered here, by question.
    asked: tokio::sync::Mutex<BTreeMap<String, Asked>>,
    /// The editor this process is the door of, when it is one.
    editor: std::sync::OnceLock<Editor>,
    /// Held while a session is being opened.
    opening: tokio::sync::Mutex<()>,
    /// Which revision of the agent's setup each live session was opened
    /// with, by connection.
    set_up_as: tokio::sync::Mutex<BTreeMap<String, u64>>,
}

/// An editor's door: what it says is carried out here, with the files the
/// editor offered.
struct Editor {
    door: String,
    files: std::sync::Mutex<Option<FileSystemCapabilities>>,
}

/// Everything a turn is made of, read from the ledger at once.
struct Prepared {
    message: Message,
    chat: Chat,
    agent: Participant,
    sender: Participant,
    /// Whose trust the sender speaks with: itself, or who made it.
    speaks_for: Participant,
    owner: Participant,
    session: Option<ChatSession>,
    given_through: u64,
    /// What was said in the chat before the message, oldest first.
    before: Vec<(Message, Participant)>,
    /// The sender is a guest the owner lets reset and compact the session.
    may_command: bool,
}

/// A `/clear` or `/compact`: what a guest who may command says as typed.
fn is_a_session_command(text: &str) -> bool {
    matches!(text.split_whitespace().next(), Some("/clear" | "/compact"))
}

/// How a turn that was given ended.
enum Given {
    /// By itself, for the reason the engine gave.
    Done(String),
    Stopped,
}

fn trust_of(participant: &Participant, owner: &Participant) -> Trust {
    match participant.kind {
        ParticipantKind::Person if participant.participant_id == owner.participant_id => {
            Trust::Principal
        }
        ParticipantKind::Agent => Trust::Participant,
        _ => Trust::Guest,
    }
}

fn speaker(participant: &Participant, trust: Trust) -> Speaker {
    Speaker {
        handle: participant.handle.clone(),
        name: participant.name.clone(),
        kind: participant.kind,
        trust,
    }
}

/// A chat as a file an agent can read: everything said, oldest first.
fn transcript(chat: &Chat, before: &[(Message, Participant)]) -> String {
    use std::fmt::Write as _;
    let mut text = format!("# {}\n", chat.title);
    for (message, sender) in before {
        let _ = write!(
            text,
            "\n## {} (@{})\n\n{}\n",
            sender.name, sender.handle, message.text
        );
    }
    text
}

/// Why a turn is given to its agent, as the agent is told: it was named,
/// it is the only one, or an agent it asked by name has answered.
fn why_it_is_given(prepared: &Prepared) -> String {
    recipients(&prepared.chat, &prepared.sender, &prepared.message.named)
        .into_iter()
        .find(|recipient| recipient.agent_id == prepared.agent.participant_id)
        .map_or_else(
            || {
                if prepared.sender.kind == ParticipantKind::Agent {
                    format!("@{} answers what you asked it", prepared.sender.handle)
                } else {
                    "this message is for you".to_owned()
                }
            },
            |found| found.why,
        )
}

impl WorkbenchShellState {
    /// Make this process an editor's door: it carries out what that editor
    /// says and nothing else. Returns the door's name, which begins the
    /// name of every message said through it.
    ///
    /// # Errors
    ///
    /// When this machine gives no random bytes, and when this process is a
    /// door already.
    pub fn carry_for_an_editor(&self) -> Result<String, WorkbenchShellError> {
        let door = crate::new_id("door")
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.chat_runtime
            .editor
            .set(Editor {
                door: door.clone(),
                files: std::sync::Mutex::new(None),
            })
            .map_err(|_| WorkbenchShellError::Conflict("this process is a door already".into()))?;
        Ok(door)
    }

    /// What the editor behind this door offered to do with files.
    pub fn editor_offers_files(&self, offered: Option<FileSystemCapabilities>) {
        if let Some(editor) = self.chat_runtime.editor.get()
            && let Ok(mut files) = editor.files.lock()
        {
            *files = offered;
        }
    }

    fn door(&self) -> Option<String> {
        self.chat_runtime
            .editor
            .get()
            .map(|editor| editor.door.clone())
    }

    /// The connection a delivery runs on, while it runs here.
    pub async fn connection_of(&self, delivery_id: &str) -> Option<String> {
        self.chat_runtime
            .running
            .lock()
            .await
            .get(delivery_id)
            .map(|running| running.connection.clone())
    }

    /// Take up what the chats were left owing. A turn the ledger has as
    /// running while nobody holds that agent's turn was interrupted when a
    /// Workbench stopped, and is said to have been; one that another
    /// process holds is that process's. Every agent that is owed something
    /// is set to work.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn take_up_chats(self: &Arc<Self>) -> Result<(), WorkbenchShellError> {
        self.take_up_chats_of(None).await
    }

    /// Take up what the chats were left owing to these agents and leave
    /// the others' to whoever opens the Workbench; everybody's when none
    /// is named.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn take_up_chats_of(
        self: &Arc<Self>,
        agents: Option<Vec<String>>,
    ) -> Result<(), WorkbenchShellError> {
        let of_these = |agent_id: &String| {
            agents
                .as_ref()
                .is_none_or(|agents| agents.contains(agent_id))
        };
        let running = self
            .with_ledger(|ledger| ledger.agents_running())
            .await
            .map_err(ledger_refusal)?;
        let mut left = Vec::new();
        for agent_id in running.into_iter().filter(of_these) {
            let path = self.turn_lock_path(&agent_id);
            let held_by_nobody = tokio::task::spawn_blocking(move || {
                std::fs::File::open(&path).map_or(true, |file| file.try_lock().is_ok())
            })
            .await
            .unwrap_or(false);
            if held_by_nobody {
                left.push(agent_id);
            }
        }
        let door = self.door();
        let owed = self
            .with_ledger(move |ledger| {
                ledger.settle_what_was_running(&left)?;
                ledger.agents_owed(door.as_deref())
            })
            .await
            .map_err(ledger_refusal)?;
        for agent_id in owed.into_iter().filter(of_these) {
            self.set_to_work(&agent_id);
        }
        Ok(())
    }

    /// Wake the agent's worker, or start one.
    pub(super) fn set_to_work(self: &Arc<Self>, agent_id: &str) {
        let wake = {
            let Ok(mut wakes) = self.chat_runtime.wake.lock() else {
                return;
            };
            Arc::clone(wakes.entry(agent_id.to_owned()).or_default())
        };
        wake.notify_one();
        let begins = self
            .chat_runtime
            .at_work
            .lock()
            .is_ok_and(|mut at_work| at_work.insert(agent_id.to_owned()));
        if begins {
            let state = Arc::clone(self);
            let agent_id = agent_id.to_owned();
            tokio::spawn(Box::pin(state.work(agent_id, wake)));
        }
    }

    async fn work(self: Arc<Self>, agent_id: String, wake: Arc<Notify>) {
        loop {
            match self.next_turn(&agent_id).await {
                Ok(Some((delivery, turn_lock))) => {
                    Box::pin(self.carry_out(&delivery)).await;
                    drop(turn_lock);
                }
                Ok(None) => {
                    if tokio::time::timeout(IDLE, wake.notified()).await.is_ok() {
                        continue;
                    }
                    if let Ok(mut at_work) = self.chat_runtime.at_work.lock() {
                        at_work.remove(&agent_id);
                    }
                    self.let_go_of(Some(&agent_id)).await;
                    // Something may have been owed between the last look and
                    // the leaving.
                    let door = self.door();
                    let owed = self
                        .with_ledger(move |ledger| ledger.agents_owed(door.as_deref()))
                        .await
                        .is_ok_and(|owed| owed.contains(&agent_id));
                    if owed {
                        self.set_to_work(&agent_id);
                    }
                    return;
                }
                // The ledger could not be read or the lock not taken: not a
                // reason to stop paying what is owed, and not one to spin.
                Err(_) => tokio::time::sleep(Duration::from_secs(2)).await,
            }
        }
    }

    fn turn_lock_path(&self, agent_id: &str) -> PathBuf {
        self.ledger_path
            .with_extension("turns")
            .join(format!("{agent_id}.lock"))
    }

    /// The next message the agent is owed, claimed, with the agent's turn
    /// lock held. Nothing owed, nothing is locked.
    async fn next_turn(
        &self,
        agent_id: &str,
    ) -> Result<Option<(Delivery, std::fs::File)>, WorkbenchShellError> {
        let door = self.door();
        let owed = self
            .with_ledger({
                let door = door.clone();
                move |ledger| ledger.agents_owed(door.as_deref())
            })
            .await
            .map_err(ledger_refusal)?;
        if !owed.iter().any(|owed| owed == agent_id) {
            return Ok(None);
        }
        let path = self.turn_lock_path(agent_id);
        let turn_lock = tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let file = std::fs::File::options()
                .create(true)
                .truncate(false)
                .write(true)
                .open(&path)?;
            // Waits while another process has the agent's turn.
            file.lock()?;
            Ok::<_, std::io::Error>(file)
        })
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        .map_err(|error| {
            WorkbenchShellError::Failed(format!("the agent's turn could not be taken: {error}"))
        })?;
        let agent = agent_id.to_owned();
        let delivery = self
            .with_ledger(move |ledger| {
                // The turn is held here, so a turn the ledger still has as
                // running is one whose process is gone.
                ledger.settle_what_was_running(std::slice::from_ref(&agent))?;
                ledger.take_next_delivery(&agent, door.as_deref())
            })
            .await
            .map_err(ledger_refusal)?;
        Ok(delivery.map(|delivery| (delivery, turn_lock)))
    }

    /// Give the message and record how it ended. Nothing that goes wrong
    /// here is lost: it is how the delivery ended, in words.
    async fn carry_out(self: &Arc<Self>, delivery: &Delivery) {
        let (state, outcome) = match Box::pin(self.give(delivery)).await {
            // How a turn ended is said when it is not the ordinary end.
            Ok(Given::Done(reason)) => (
                DeliveryState::Done,
                (reason != "end_turn").then_some(reason),
            ),
            Ok(Given::Stopped) => (DeliveryState::Stopped, None),
            Err(refusal) => (DeliveryState::Failed, Some(refusal.to_string())),
        };
        self.chat_runtime
            .running
            .lock()
            .await
            .remove(&delivery.delivery_id);
        self.chat_runtime
            .asked
            .lock()
            .await
            .retain(|_, asked| asked.delivery_id.as_deref() != Some(&delivery.delivery_id));
        let delivery_id = delivery.delivery_id.clone();
        let _ = self
            .with_ledger(move |ledger| ledger.end_delivery(&delivery_id, state, outcome.as_deref()))
            .await;
    }

    async fn prepared(&self, delivery: &Delivery) -> Result<Prepared, WorkbenchShellError> {
        let delivery = delivery.clone();
        self.with_ledger(move |ledger| {
            let message = ledger.message(&delivery.message_id)?;
            let chat = ledger.chat(&delivery.chat_id)?;
            let agent = ledger.participant(&delivery.agent_id)?;
            let sender = ledger.participant(&message.sender_id)?;
            let speaks_for = match (&sender.kind, &sender.made_by) {
                (ParticipantKind::Schedule, Some(maker)) => ledger.participant(maker)?,
                _ => sender.clone(),
            };
            let owner = ledger.owner()?;
            let may_command = sender.kind == ParticipantKind::Guest
                && ledger
                    .identities_of(&sender.participant_id)?
                    .iter()
                    .any(|identity| identity.may_command);
            let session = ledger.current_session(&delivery.chat_id, &delivery.agent_id)?;
            let given_through =
                ledger.given_through(&delivery.chat_id, &delivery.agent_id, None)?;
            let mut before = Vec::new();
            let mut senders: BTreeMap<String, Participant> = BTreeMap::new();
            let mut after = 0;
            'pages: loop {
                let page = ledger.said_after(&delivery.chat_id, after, 500)?;
                if page.is_empty() {
                    break;
                }
                for said in page {
                    after = said.sequence;
                    if said.sequence >= message.sequence {
                        break 'pages;
                    }
                    let sender = if let Some(sender) = senders.get(&said.sender_id) {
                        sender.clone()
                    } else {
                        let sender = ledger.participant(&said.sender_id)?;
                        senders.insert(said.sender_id.clone(), sender.clone());
                        sender
                    };
                    before.push((said, sender));
                }
            }
            Ok(Prepared {
                message,
                chat,
                agent,
                sender,
                speaks_for,
                owner,
                session,
                given_through,
                before,
                may_command,
            })
        })
        .await
        .map_err(ledger_refusal)
    }

    /// The session an agent has in a chat, for a page that wants what only
    /// a live session has: what the engine lets a person choose, and the
    /// Apps its servers bring. With `open`, a session that is not live is
    /// opened - the engine starts; without, nothing is started and a
    /// session that is not live is said not to be.
    ///
    /// # Errors
    ///
    /// Refuses a chat or an agent that does not exist, an agent that is
    /// not in the chat, and whatever refuses the session when it is opened.
    pub async fn session_in_chat(
        self: &Arc<Self>,
        chat_id: &str,
        agent_id: &str,
        open: bool,
    ) -> Result<Value, WorkbenchShellError> {
        let key = (chat_id.to_owned(), agent_id.to_owned());
        let live = self.chat_runtime.live.lock().await.get(&key).cloned();
        if let Some(connection_id) = live
            && let Ok(connection) = self.connection(&connection_id).await
            && !matches!(connection.control.phase(), NativeSessionPhase::Finished)
        {
            return Ok(json!({ "connection_id": connection_id }));
        }
        if !open {
            return Ok(json!({ "connection_id": null }));
        }
        let (chat, agent) = key.clone();
        let (chat, agent, session) = self
            .with_ledger(move |ledger| {
                let found = ledger.chat(&chat)?;
                let who = ledger.participant(&agent)?;
                let session = ledger.current_session(&chat, &agent)?;
                Ok((found, who, session))
            })
            .await
            .map_err(ledger_refusal)?;
        if !chat
            .members
            .iter()
            .any(|member| member.participant_id == agent.participant_id)
        {
            return Err(WorkbenchShellError::Invalid(format!(
                "@{} is not in this chat",
                agent.handle
            )));
        }
        let profile_id = agent.profile_id.clone().ok_or_else(|| {
            WorkbenchShellError::Conflict(format!(
                "@{} has no agent behind it any more",
                agent.handle
            ))
        })?;
        let asked_in = AskedIn {
            chat: chat.chat_id.clone(),
            agent: agent.participant_id.clone(),
            delivery: None,
        };
        let (connection_id, _fresh) = self
            .connection_of_session(
                &chat.chat_id,
                &agent.participant_id,
                session.as_ref(),
                &profile_id,
                &asked_in,
            )
            .await?;
        // Somebody lets go of it when the agent has been idle.
        self.set_to_work(&agent.participant_id);
        Ok(json!({ "connection_id": connection_id }))
    }

    /// The connection the agent is live on in the chat, opened when it has
    /// none, and whether its session there is a fresh one.
    async fn connection_in_chat(
        self: &Arc<Self>,
        prepared: &Prepared,
        profile_id: &str,
        asked_in: &AskedIn,
    ) -> Result<(String, bool), WorkbenchShellError> {
        self.connection_of_session(
            &prepared.chat.chat_id,
            &prepared.agent.participant_id,
            prepared.session.as_ref(),
            profile_id,
            asked_in,
        )
        .await
    }

    /// Open a session and hear what its engine asks while it is opened.
    async fn open_heard(
        self: &Arc<Self>,
        mut opening: Opening,
        asked_in: &AskedIn,
    ) -> Result<String, WorkbenchShellError> {
        let connection_id = format!(
            "chat-{}",
            crate::new_id("o").map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        );
        opening.requested_connection_id = Some(connection_id.clone());
        let heard = tokio::spawn(
            Arc::clone(self).keep_questions_of_opening(connection_id.clone(), asked_in.clone()),
        );
        let opened = self.open_as(opening).await;
        heard.abort();
        if opened.is_err() {
            // What it asked and was not answered can no longer be.
            let unanswered: Vec<String> = {
                let mut asked = self.chat_runtime.asked.lock().await;
                let gone: Vec<String> = asked
                    .iter()
                    .filter(|(_, asked)| asked.connection_id == connection_id)
                    .map(|(question_id, _)| question_id.clone())
                    .collect();
                asked.retain(|_, asked| asked.connection_id != connection_id);
                gone
            };
            for question_id in unanswered {
                let _ = self
                    .with_ledger(move |ledger| ledger.lapse_question(&question_id))
                    .await;
            }
        }
        opened.map(|(connection_id, _route, _session)| connection_id)
    }

    async fn connection_of_session(
        self: &Arc<Self>,
        chat_id: &str,
        agent_id: &str,
        session: Option<&ChatSession>,
        profile_id: &str,
        asked_in: &AskedIn,
    ) -> Result<(String, bool), WorkbenchShellError> {
        // One session is opened at a time, so a page asking for an agent's
        // session and a worker about to give it a message open one between
        // them, not one each.
        let _opening = self.chat_runtime.opening.lock().await;
        let revision = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?
            .revision;
        let key = (chat_id.to_owned(), agent_id.to_owned());
        let known = self.chat_runtime.live.lock().await.get(&key).cloned();
        if let Some(connection_id) = known {
            let alive = self
                .connection(&connection_id)
                .await
                .is_ok_and(|connection| {
                    !matches!(connection.control.phase(), NativeSessionPhase::Finished)
                });
            // A session is opened with the agent's setup as it was then.
            // Set up differently since, the agent goes on with what it is
            // set up with now: the session is let go of and opened again.
            let as_opened = self
                .chat_runtime
                .set_up_as
                .lock()
                .await
                .get(&connection_id)
                .copied();
            if alive && as_opened == Some(revision) {
                return Ok((connection_id, false));
            }
            self.chat_runtime.live.lock().await.remove(&key);
            self.chat_runtime
                .set_up_as
                .lock()
                .await
                .remove(&connection_id);
            if alive {
                let _ = self.disconnect(&connection_id).await;
            }
        }
        // An editor has the files the person is looking at; a page has none.
        let files = self
            .chat_runtime
            .editor
            .get()
            .and_then(|editor| editor.files.lock().ok().and_then(|files| files.clone()));
        let opening = |mode, route_id, why: &str| Opening {
            profile_id: profile_id.to_owned(),
            mode,
            route_id,
            requested_connection_id: None,
            auth_method_id: None,
            file_callbacks: files.clone(),
            place: Some(ChatPlace {
                chat_id: chat_id.to_owned(),
                why_the_last_ended: why.to_owned(),
            }),
        };
        let continued = match session {
            Some(session) => Some(
                self.open_heard(
                    opening(
                        ShellConnectionMode::Resume,
                        Some(session.route_id.clone()),
                        "",
                    ),
                    asked_in,
                )
                .await,
            ),
            None => None,
        };
        let (connection_id, fresh) = match continued {
            Some(Ok(opened)) => (opened, false),
            // The engine cannot go on with the session this chat holds. The
            // chat goes on in a fresh one; if that cannot be opened either,
            // why it cannot is what is said.
            Some(Err(refusal)) => (
                self.open_heard(
                    opening(ShellConnectionMode::New, None, &refusal.to_string()),
                    asked_in,
                )
                .await?,
                true,
            ),
            None => (
                self.open_heard(opening(ShellConnectionMode::New, None, ""), asked_in)
                    .await?,
                true,
            ),
        };
        self.chat_runtime
            .live
            .lock()
            .await
            .insert(key, connection_id.clone());
        self.chat_runtime
            .set_up_as
            .lock()
            .await
            .insert(connection_id.clone(), revision);
        Ok((connection_id, fresh))
    }

    /// The turn's block, and what stands above it.
    async fn turn_of(
        &self,
        prepared: &Prepared,
        fresh: bool,
        workspace: &std::path::Path,
        attached: &BTreeSet<String>,
    ) -> Result<(Vec<ContentBlock>, Vec<String>, String), WorkbenchShellError> {
        let trust = trust_of(&prepared.speaks_for, &prepared.owner);
        // The owner's words are the turn as typed. So is a session command
        // from a guest the owner lets give one; anything else a guest says
        // is data in the block.
        let as_typed = trust == Trust::Principal
            || (prepared.may_command && is_a_session_command(&prepared.message.text));
        let said = |message: &Message, sender: &Participant| Said {
            from: speaker(sender, trust_of(sender, &prepared.owner)),
            at_ms: message.created_ms,
            text: message.text.clone(),
        };
        // A session that goes on remembers what it was given and what it
        // said; it is told what others said since. A fresh one remembers
        // nothing and is told everything.
        let earlier: Vec<Said> = prepared
            .before
            .iter()
            .filter(|(message, sender)| {
                fresh
                    || (message.sequence > prepared.given_through
                        && sender.participant_id != prepared.agent.participant_id)
            })
            .map(|(message, sender)| said(message, sender))
            .collect();
        let (earlier, left_out) = if fresh {
            crate::fitted_within(earlier, &HISTORY)
        } else {
            crate::fitted(earlier)
        };
        let transcript = if fresh && !prepared.before.is_empty() {
            let text = transcript(&prepared.chat, &prepared.before);
            crate::workbench_files::hand_over(workspace, "chat-so-far.md", text.as_bytes())
                .await
                .ok()
                .map(|path| path.display().to_string())
        } else {
            None
        };
        let why = why_it_is_given(prepared);
        let agents = prepared
            .chat
            .members
            .iter()
            .filter(|member| member.kind == ParticipantKind::Agent && !member.retired)
            .count();
        let turn = Turn {
            to: prepared.agent.handle.clone(),
            why,
            chat_title: prepared.chat.title.clone(),
            with: prepared
                .chat
                .members
                .iter()
                .map(|member| member.handle.clone())
                .collect(),
            via: prepared.message.channel.clone(),
            earlier,
            left_out,
            transcript,
            now: Said {
                from: speaker(&prepared.sender, trust),
                at_ms: prepared.message.created_ms,
                text: prepared.message.text.clone(),
            },
            replies_left: (agents > 1).then(|| {
                prepared
                    .chat
                    .reply_limit
                    .saturating_sub(prepared.chat.agent_replies)
            }),
        };
        let mut above = Vec::new();
        let mut handed_over = Vec::new();
        for block in prepared.message.content.as_array().into_iter().flatten() {
            let kind = block.get("type").and_then(Value::as_str);
            if kind == Some("attachment") {
                if let Some(id) = block.get("content_ref").and_then(Value::as_str) {
                    handed_over.push(id.to_owned());
                }
            } else if kind == Some("context") {
                // What an App said the person was looking at, for an agent
                // that attaches the App's server and so can follow it.
                if trust == Trust::Principal
                    && let Ok(context) =
                        serde_json::from_value::<super::ModelContext>(block.clone())
                    && attached.contains(&context.server_name)
                {
                    above.extend(super::model_context::context_content(&context));
                }
            } else if as_typed
                && let Ok(typed) = serde_json::from_value::<ContentBlock>(block.clone())
            {
                // A principal's words are the turn, as they were typed, and
                // so is what they put beside them. Anybody else's words are
                // in the block, as data.
                above.push(typed);
            }
        }
        let typed = if as_typed {
            prepared.message.text.as_str()
        } else {
            ""
        };
        let block = crate::sealed(&turn, typed).map_err(WorkbenchShellError::Failed)?;
        Ok((above, handed_over, block))
    }

    async fn give(self: &Arc<Self>, delivery: &Delivery) -> Result<Given, WorkbenchShellError> {
        let prepared = self.prepared(delivery).await?;
        let profile_id = prepared.agent.profile_id.clone().ok_or_else(|| {
            WorkbenchShellError::Conflict(format!(
                "@{} has no agent behind it any more",
                prepared.agent.handle
            ))
        })?;
        let profile = self
            .inventory
            .select(&profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let (connection_id, fresh) = self
            .connection_in_chat(&prepared, &profile_id, &AskedIn::turn(delivery))
            .await?;
        let connection = self.connection(&connection_id).await?;
        let attached: BTreeSet<String> = profile
            .attachments
            .iter()
            .map(|attachment| attachment.server_name.clone())
            .collect();
        let (above, handed_over, block) = self
            .turn_of(&prepared, fresh, &profile.workspace, &attached)
            .await?;
        self.chat_runtime.running.lock().await.insert(
            delivery.delivery_id.clone(),
            Running {
                chat: delivery.chat_id.clone(),
                agent: delivery.agent_id.clone(),
                connection: connection_id.clone(),
            },
        );
        let keepers = [
            tokio::spawn(
                Arc::clone(self)
                    .keep_permission_questions(connection.control.clone(), delivery.clone()),
            ),
            tokio::spawn(Arc::clone(self).keep_form_questions(
                connection_id.clone(),
                connection.control.clone(),
                AskedIn::turn(delivery),
            )),
        ];
        let answered = self
            .submit_workbench_prompt(&connection_id, above, handed_over, block)
            .await;
        for keeper in keepers {
            keeper.abort();
        }
        let answered = match answered {
            Ok(answered) => answered,
            Err(refusal) => {
                // A turn that failed took its connection with it; one that
                // was refused left it as it was.
                if self.connection(&connection_id).await.is_err() {
                    self.chat_runtime
                        .live
                        .lock()
                        .await
                        .remove(&(delivery.chat_id.clone(), delivery.agent_id.clone()));
                }
                return Err(refusal);
            }
        };
        let (chat_id, agent_id, through) = (
            delivery.chat_id.clone(),
            delivery.agent_id.clone(),
            prepared.message.sequence,
        );
        self.with_ledger(move |ledger| ledger.given_through(&chat_id, &agent_id, Some(through)))
            .await
            .map_err(ledger_refusal)?;
        self.pass_on_what_was_answered(&prepared).await?;
        Ok(
            if answered.control_outcome == NativeTurnControlOutcome::Cancelled {
                Given::Stopped
            } else {
                Given::Done(answered.stop_reason)
            },
        )
    }

    /// What the agent answered is for the agents it named, while the chat
    /// still allows agents to answer each other.
    async fn pass_on_what_was_answered(
        self: &Arc<Self>,
        prepared: &Prepared,
    ) -> Result<(), WorkbenchShellError> {
        let (chat_id, agent_id, after) = (
            prepared.chat.chat_id.clone(),
            prepared.agent.participant_id.clone(),
            prepared.message.sequence,
        );
        let (asked_by, asked_named) = (prepared.sender.clone(), prepared.message.named.clone());
        let owed = self
            .with_ledger(move |ledger| {
                let chat = ledger.chat(&chat_id)?;
                let agent = ledger.participant(&agent_id)?;
                let Some(answer) = ledger
                    .said_after(&chat_id, after, 500)?
                    .into_iter()
                    .rev()
                    .find(|said| said.sender_id == agent_id)
                else {
                    return Ok(Vec::new());
                };
                let mut agents: Vec<String> = recipients(&chat, &agent, &answer.named)
                    .into_iter()
                    .map(|recipient| recipient.agent_id)
                    .collect();
                // And for the agent that asked it by name.
                if let Some(back) = super::chats::asker(&chat, &agent, &asked_by, &asked_named)
                    && !agents.contains(&back.agent_id)
                {
                    agents.push(back.agent_id);
                }
                if agents.is_empty() {
                    return Ok(Vec::new());
                }
                if !ledger.count_agent_reply(&chat_id, &answer.message_id)? {
                    return Ok(Vec::new());
                }
                ledger.deliver(&answer.message_id, &agents)
            })
            .await
            .map_err(ledger_refusal)?;
        for delivery in owed {
            self.set_to_work(&delivery.agent_id);
        }
        Ok(())
    }

    /// Keep what the agent asks before it does something, and carry the
    /// answer back when the ledger has one. The ledger is where a question
    /// and its answer meet, because who answers may be another process.
    async fn keep_permission_questions(
        self: Arc<Self>,
        control: NativeSessionControl,
        delivery: Delivery,
    ) {
        let mut after = 0;
        let mut waiting = tokio::task::JoinSet::new();
        while let Some(request) = control.permission_request_after(after).await {
            after = request.sequence;
            let tool_call = serde_json::to_value(&request.tool_call).unwrap_or(Value::Null);
            let options: Vec<Value> = request
                .options
                .iter()
                .map(|option| serde_json::to_value(option).unwrap_or(Value::Null))
                .collect();
            let asked = json!({
                "sequence": request.sequence,
                "title": tool_call.get("title"),
                "tool_kind": tool_call.get("kind"),
                "tool_call_id": tool_call.get("toolCallId"),
                "asked_by": request.provenance,
                "options": options,
            });
            let Some(question) = self
                .keep_question(&AskedIn::turn(&delivery), "permission", asked)
                .await
            else {
                continue;
            };
            waiting.spawn(Arc::clone(&self).carry_answer_back(
                control.clone(),
                question.question_id,
                request.sequence,
            ));
        }
    }

    async fn carry_answer_back(
        self: Arc<Self>,
        control: NativeSessionControl,
        question_id: String,
        sequence: u64,
    ) {
        loop {
            tokio::time::sleep(LOOK).await;
            let id = question_id.clone();
            let Ok(question) = self.with_ledger(move |ledger| ledger.question(&id)).await else {
                continue;
            };
            match question.state {
                QuestionState::Waiting => {}
                QuestionState::Answered => {
                    if let Some(option) = question
                        .answer
                        .as_ref()
                        .and_then(|answer| answer.get("option"))
                        .and_then(Value::as_str)
                    {
                        let _ = control.select_permission(sequence, option);
                    }
                    return;
                }
                QuestionState::Lapsed => return,
            }
        }
    }

    async fn keep_form_questions(
        self: Arc<Self>,
        connection_id: String,
        control: NativeSessionControl,
        asked_in: AskedIn,
    ) {
        let mut after = 0;
        while let Some(request) = control.elicitation_request_after(after).await {
            after = request.sequence;
            // Kept already, by whoever listened while the session was
            // being opened.
            let known = self.chat_runtime.asked.lock().await.values().any(|asked| {
                asked.connection_id == connection_id && asked.sequence == request.sequence
            });
            if known {
                continue;
            }
            // What a form asks and where a link leads are the engine's and
            // may be secret; the ledger keeps that something is asked, and
            // the question is read from the session while it waits.
            let kind = match &request.request.mode {
                ElicitationMode::Url(_) => "link",
                _ => "form",
            };
            if let Some(question) = self.keep_question(&asked_in, kind, json!({})).await {
                self.chat_runtime.asked.lock().await.insert(
                    question.question_id,
                    Asked {
                        connection_id: connection_id.clone(),
                        delivery_id: asked_in.delivery.clone(),
                        sequence: request.sequence,
                    },
                );
            }
        }
    }

    /// Listen for what an engine asks while its session is being opened:
    /// an engine may ask before there is a session at all, to be signed in
    /// or to be told where it works, and would wait for ever unheard.
    async fn keep_questions_of_opening(self: Arc<Self>, connection_id: String, asked_in: AskedIn) {
        let control = loop {
            if let Ok(connection) = self.connection(&connection_id).await {
                break connection.control.clone();
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        };
        self.keep_form_questions(connection_id, control, asked_in)
            .await;
    }

    async fn keep_question(
        &self,
        asked_in: &AskedIn,
        kind: &'static str,
        asked: Value,
    ) -> Option<Question> {
        let asked_in = asked_in.clone();
        self.with_ledger(move |ledger| {
            ledger.ask(
                &asked_in.chat,
                &asked_in.agent,
                asked_in.delivery.as_deref(),
                kind,
                &asked,
            )
        })
        .await
        .ok()
    }

    /// What a form asks or where a link leads, read from the session that
    /// asked, while the question waits.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for a question that is not
    /// a form or a link waiting in this process.
    pub async fn question_in_full(&self, question_id: &str) -> Result<Value, WorkbenchShellError> {
        let asked = self.asked(question_id).await?;
        let connection = self.connection(&asked.connection_id).await?;
        let waiting = tokio::time::timeout(
            Duration::from_millis(200),
            connection
                .control
                .elicitation_request_after(asked.sequence.saturating_sub(1)),
        )
        .await
        .ok()
        .flatten()
        .filter(|request| request.sequence == asked.sequence)
        .ok_or_else(|| no_longer(question_id))?;
        serde_json::to_value(&waiting.request)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    async fn asked(&self, question_id: &str) -> Result<Asked, WorkbenchShellError> {
        self.chat_runtime
            .asked
            .lock()
            .await
            .get(question_id)
            .cloned()
            .ok_or_else(|| {
                WorkbenchShellError::Conflict(format!(
                    "question {question_id} is answered where its agent runs, while it waits"
                ))
            })
    }

    /// Answer a question that waits: `{"option": id}` for a permission, the
    /// protocol's own action for a form or a link.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for a question that no
    /// longer waits and for an answer that was not offered.
    pub async fn answer_in_chat(
        &self,
        question_id: &str,
        answer: Value,
    ) -> Result<Question, WorkbenchShellError> {
        self.answer_in_chat_by(question_id, answer, None).await
    }

    /// [`Self::answer_in_chat`], answered by somebody other than the person
    /// the Workbench is: a schedule, for a question nobody came to.
    pub(super) async fn answer_in_chat_by(
        &self,
        question_id: &str,
        answer: Value,
        by: Option<String>,
    ) -> Result<Question, WorkbenchShellError> {
        let id = question_id.to_owned();
        let question = self
            .with_ledger(move |ledger| ledger.question(&id))
            .await
            .map_err(ledger_refusal)?;
        if question.state != QuestionState::Waiting {
            return Err(no_longer(question_id));
        }
        let kept = if question.kind == "permission" {
            let option = answer
                .get("option")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkbenchShellError::Invalid("an answer names the option chosen".into())
                })?;
            let offered = question
                .asked
                .get("options")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|offered| offered.get("optionId").and_then(Value::as_str) == Some(option))
                .ok_or_else(|| {
                    WorkbenchShellError::Conflict(format!(
                        "`{option}` was not offered for question {question_id}"
                    ))
                })?;
            json!({ "option": option, "name": offered.get("name") })
        } else {
            let asked = self.asked(question_id).await?;
            let connection = self.connection(&asked.connection_id).await?;
            let action: ElicitationAction = serde_json::from_value(answer)
                .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?;
            let word = match &action {
                ElicitationAction::Accept(_) => "accept",
                ElicitationAction::Decline => "decline",
                _ => "cancel",
            };
            connection
                .control
                .answer_elicitation(asked.sequence, action)
                .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))?;
            self.chat_runtime.asked.lock().await.remove(question_id);
            // What was typed into a form stays between the person and the
            // engine.
            json!({ "action": word })
        };
        let id = question_id.to_owned();
        self.with_ledger(move |ledger| {
            let by = match by {
                Some(by) => by,
                None => ledger.owner()?.participant_id,
            };
            ledger.answer_question(&id, &kept, &by)
        })
        .await
        .map_err(|error| match error {
            // Somebody answered between the look and the answer.
            crate::RoutingError::InvalidBinding(said) if said.contains("cannot be answered") => {
                WorkbenchShellError::Conflict(said)
            }
            other => ledger_refusal(other),
        })
    }

    /// Answer what an agent asked, from where its turn runs: the session
    /// is told at once, and the question the ledger keeps for it is kept
    /// as answered. An editor answers this way; it is where the person is.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for a question that no
    /// longer waits and for an option that was not offered.
    pub async fn answer_where_it_runs(
        &self,
        delivery_id: &str,
        sequence: u64,
        option: &str,
    ) -> Result<(), WorkbenchShellError> {
        let connection_id = self
            .connection_of(delivery_id)
            .await
            .ok_or_else(|| no_longer(delivery_id))?;
        self.select_permission(&connection_id, sequence, option)
            .await?;
        // The question is written down a moment after it is asked.
        for _ in 0..20 {
            let (id, chosen) = (delivery_id.to_owned(), option.to_owned());
            let kept = self
                .with_ledger(move |ledger| {
                    let Some(question) =
                        ledger.questions_waiting(None)?.into_iter().find(|asked| {
                            asked.delivery_id.as_deref() == Some(id.as_str())
                                && asked.asked.get("sequence").and_then(Value::as_u64)
                                    == Some(sequence)
                        })
                    else {
                        return Ok(false);
                    };
                    let owner = ledger.owner()?;
                    ledger.answer_question(
                        &question.question_id,
                        &json!({ "option": chosen }),
                        &owner.participant_id,
                    )?;
                    Ok(true)
                })
                .await
                .unwrap_or(true);
            if kept {
                break;
            }
            tokio::time::sleep(LOOK).await;
        }
        Ok(())
    }

    /// Stop what an agent is doing in a chat, or what every agent is: the
    /// turn that runs is cancelled and what waited behind it is not begun.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be written.
    pub async fn stop_in_chat(
        &self,
        chat_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Value, WorkbenchShellError> {
        let (chat, agent) = (chat_id.to_owned(), agent_id.map(str::to_owned));
        let not_begun = self
            .with_ledger(move |ledger| ledger.stop_queued(&chat, agent.as_deref()))
            .await
            .map_err(ledger_refusal)?;
        let running: Vec<Running> = self
            .chat_runtime
            .running
            .lock()
            .await
            .values()
            .filter(|running| {
                running.chat == chat_id && agent_id.is_none_or(|agent_id| running.agent == agent_id)
            })
            .cloned()
            .collect();
        let mut stopped = Vec::new();
        for running in running {
            if let Ok(connection) = self.connection(&running.connection).await
                && connection.control.cancel_active_turn().is_some()
            {
                stopped.push(running.agent);
            }
        }
        Ok(json!({ "stopped": stopped, "not_begun": not_begun.len() }))
    }

    /// Stop what an agent is doing, in whichever chat it is doing it.
    pub async fn stop_agent(&self, agent_id: &str) {
        let running: Vec<Running> = self
            .chat_runtime
            .running
            .lock()
            .await
            .values()
            .filter(|running| running.agent == agent_id)
            .cloned()
            .collect();
        for running in running {
            if let Ok(connection) = self.connection(&running.connection).await {
                let _ = connection.control.cancel_active_turn();
            }
        }
    }

    /// Close the session this process holds open for one agent in one
    /// chat, when it holds one.
    pub async fn let_go_of_in(&self, chat_id: &str, agent_id: &str) {
        let gone = self
            .chat_runtime
            .live
            .lock()
            .await
            .remove(&(chat_id.to_owned(), agent_id.to_owned()));
        if let Some(connection_id) = gone {
            let _ = self.disconnect(&connection_id).await;
        }
    }

    /// Close the sessions this process holds open for one chat.
    pub async fn let_go_of_chat(&self, chat_id: &str) {
        let gone: Vec<String> = {
            let mut live = self.chat_runtime.live.lock().await;
            let keys: Vec<(String, String)> = live
                .keys()
                .filter(|(chat, _)| chat == chat_id)
                .cloned()
                .collect();
            keys.iter().filter_map(|key| live.remove(key)).collect()
        };
        for connection_id in gone {
            let _ = self.disconnect(&connection_id).await;
        }
    }

    /// Wait until a delivery has ended, and say how. It ends by itself, by
    /// being stopped, or by the turn's own deadline; a question that waits
    /// holds it open, as it holds the turn.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for a delivery that does not exist.
    pub async fn ended(&self, delivery_id: &str) -> Result<Delivery, WorkbenchShellError> {
        loop {
            let id = delivery_id.to_owned();
            let delivery = self
                .with_ledger(move |ledger| ledger.delivery(&id))
                .await
                .map_err(ledger_refusal)?;
            if delivery.state.ended() {
                return Ok(delivery);
            }
            tokio::time::sleep(LOOK).await;
        }
    }

    /// Close the sessions this process holds open for chats: an agent's, or
    /// everybody's. The engines keep them; the chats go on when somebody
    /// writes.
    pub async fn let_go_of(&self, agent_id: Option<&str>) {
        let gone: Vec<String> = {
            let mut live = self.chat_runtime.live.lock().await;
            let keys: Vec<(String, String)> = live
                .keys()
                .filter(|(_, agent)| agent_id.is_none_or(|agent_id| agent == agent_id))
                .cloned()
                .collect();
            keys.iter().filter_map(|key| live.remove(key)).collect()
        };
        for connection_id in gone {
            let _ = self.disconnect(&connection_id).await;
        }
    }
}

fn no_longer(question_id: &str) -> WorkbenchShellError {
    WorkbenchShellError::Conflict(format!(
        "question {question_id} can no longer be answered: the turn that asked it is over"
    ))
}
