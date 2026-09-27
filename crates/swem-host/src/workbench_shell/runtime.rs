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
    ContentBlock, ElicitationAction, ElicitationMode, TextContent,
};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::chats::{ledger_refusal, recipients};
use super::{
    ChatPlace, Closing, Opening, ShellConnectionMode, WorkbenchShellError, WorkbenchShellState,
};
use crate::{
    Chat, ChatSession, Delivery, DeliveryState, Message, NativeSessionControl, NativeSessionPhase,
    NativeTurnControlOutcome, Participant, ParticipantKind, Question, Said, Speaker, Trust, Turn,
};

/// How long an agent's sessions are kept open after its last turn.
const IDLE: Duration = Duration::from_secs(600);

/// How much of what was said before a fresh session is given in its first
/// turn; the whole of it is handed over as a file beside.
const HISTORY: crate::Fitting = crate::Fitting {
    count: 400,
    each: 4_000,
    all: 48_000,
};

/// Where a question is answered.
#[derive(Clone, Debug)]
struct Asked {
    connection_id: String,
    delivery_id: String,
    sequence: u64,
    /// The options of a permission question, by id; none for a form or a
    /// link.
    options: BTreeMap<String, String>,
    form: bool,
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
    /// The questions that can be answered, by id.
    asked: tokio::sync::Mutex<BTreeMap<String, Asked>>,
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
}

/// How a turn that was given ended.
enum Given {
    Done,
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

impl WorkbenchShellState {
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
        let running = self
            .with_ledger(|ledger| ledger.agents_running())
            .await
            .map_err(ledger_refusal)?;
        let mut left = Vec::new();
        for agent_id in running {
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
        let owed = self
            .with_ledger(move |ledger| {
                ledger.settle_what_was_running(&left)?;
                ledger.agents_owed()
            })
            .await
            .map_err(ledger_refusal)?;
        for agent_id in owed {
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
            tokio::spawn(async move { state.work(agent_id, wake).await });
        }
    }

    async fn work(self: Arc<Self>, agent_id: String, wake: Arc<Notify>) {
        loop {
            match self.next_turn(&agent_id).await {
                Ok(Some((delivery, turn_lock))) => {
                    self.carry_out(&delivery).await;
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
                    let owed = self
                        .with_ledger(|ledger| ledger.agents_owed())
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
        let owed = self
            .with_ledger(|ledger| ledger.agents_owed())
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
            .with_ledger(move |ledger| ledger.take_next_delivery(&agent))
            .await
            .map_err(ledger_refusal)?;
        Ok(delivery.map(|delivery| (delivery, turn_lock)))
    }

    /// Give the message and record how it ended. Nothing that goes wrong
    /// here is lost: it is how the delivery ended, in words.
    async fn carry_out(self: &Arc<Self>, delivery: &Delivery) {
        let (state, outcome) = match self.give(delivery).await {
            Ok(Given::Done) => (DeliveryState::Done, None),
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
            .retain(|_, asked| asked.delivery_id != delivery.delivery_id);
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
            })
        })
        .await
        .map_err(ledger_refusal)
    }

    /// The connection the agent is live on in the chat, opened when it has
    /// none, and whether its session there is a fresh one.
    async fn connection_in_chat(
        &self,
        prepared: &Prepared,
        profile_id: &str,
    ) -> Result<(String, bool), WorkbenchShellError> {
        let key = (
            prepared.chat.chat_id.clone(),
            prepared.agent.participant_id.clone(),
        );
        let known = self.chat_runtime.live.lock().await.get(&key).cloned();
        if let Some(connection_id) = known {
            let alive = self
                .connection(&connection_id)
                .await
                .is_ok_and(|connection| {
                    !matches!(connection.control.phase(), NativeSessionPhase::Finished)
                });
            if alive {
                return Ok((connection_id, false));
            }
            self.chat_runtime.live.lock().await.remove(&key);
        }
        let opening = |mode, route_id, why: &str| Opening {
            profile_id: profile_id.to_owned(),
            mode,
            route_id,
            requested_connection_id: None,
            auth_method_id: None,
            file_callbacks: None,
            place: Some(ChatPlace {
                chat_id: prepared.chat.chat_id.clone(),
                why_the_last_ended: why.to_owned(),
            }),
        };
        let continued = match &prepared.session {
            Some(session) => Some(
                self.open_as(opening(
                    ShellConnectionMode::Resume,
                    Some(session.route_id.clone()),
                    "",
                ))
                .await,
            ),
            None => None,
        };
        let ((connection_id, _route, _session), fresh) = match continued {
            Some(Ok(opened)) => (opened, false),
            // The engine cannot go on with the session this chat holds. The
            // chat goes on in a fresh one; if that cannot be opened either,
            // why it cannot is what is said.
            Some(Err(refusal)) => (
                self.open_as(opening(
                    ShellConnectionMode::New,
                    None,
                    &refusal.to_string(),
                ))
                .await?,
                true,
            ),
            None => (
                self.open_as(opening(ShellConnectionMode::New, None, ""))
                    .await?,
                true,
            ),
        };
        self.chat_runtime
            .live
            .lock()
            .await
            .insert(key, connection_id.clone());
        Ok((connection_id, fresh))
    }

    /// The turn's block, and what stands above it.
    async fn turn_of(
        &self,
        prepared: &Prepared,
        fresh: bool,
        workspace: &std::path::Path,
    ) -> Result<(Vec<ContentBlock>, Vec<String>, String), WorkbenchShellError> {
        let trust = trust_of(&prepared.speaks_for, &prepared.owner);
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
        let why = recipients(&prepared.chat, &prepared.sender, &prepared.message.named)
            .into_iter()
            .find(|recipient| recipient.agent_id == prepared.agent.participant_id)
            .map_or_else(|| "this message is for you".to_owned(), |found| found.why);
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
            match block.get("type").and_then(Value::as_str) {
                // A principal's words are the turn, as they were typed.
                // Anybody else's are in the block, as data.
                Some("text") if trust == Trust::Principal => {
                    if let Some(text) = block.get("text").and_then(Value::as_str) {
                        above.push(ContentBlock::Text(TextContent::new(text)));
                    }
                }
                Some("attachment") => {
                    if let Some(id) = block.get("content_ref").and_then(Value::as_str) {
                        handed_over.push(id.to_owned());
                    }
                }
                _ => {}
            }
        }
        let typed = if trust == Trust::Principal {
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
        let (connection_id, fresh) = self.connection_in_chat(&prepared, &profile_id).await?;
        let connection = self.connection(&connection_id).await?;
        let (above, handed_over, block) =
            self.turn_of(&prepared, fresh, &profile.workspace).await?;
        self.chat_runtime.running.lock().await.insert(
            delivery.delivery_id.clone(),
            Running {
                chat: delivery.chat_id.clone(),
                agent: delivery.agent_id.clone(),
                connection: connection_id.clone(),
            },
        );
        let keepers = [
            tokio::spawn(Arc::clone(self).keep_permission_questions(
                connection_id.clone(),
                connection.control.clone(),
                delivery.clone(),
            )),
            tokio::spawn(Arc::clone(self).keep_form_questions(
                connection_id.clone(),
                connection.control.clone(),
                delivery.clone(),
            )),
        ];
        let answered = self
            .submit_workbench_prompt(&connection_id, above, handed_over, Closing::Block(block))
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
                Given::Done
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
                let agents: Vec<String> = recipients(&chat, &agent, &answer.named)
                    .into_iter()
                    .map(|recipient| recipient.agent_id)
                    .collect();
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

    async fn keep_permission_questions(
        self: Arc<Self>,
        connection_id: String,
        control: NativeSessionControl,
        delivery: Delivery,
    ) {
        let mut after = 0;
        while let Some(request) = control.permission_request_after(after).await {
            after = request.sequence;
            let tool_call = serde_json::to_value(&request.tool_call).unwrap_or(Value::Null);
            let options: Vec<Value> = request
                .options
                .iter()
                .map(|option| serde_json::to_value(option).unwrap_or(Value::Null))
                .collect();
            let asked = json!({
                "title": tool_call.get("title"),
                "tool_kind": tool_call.get("kind"),
                "tool_call_id": tool_call.get("toolCallId"),
                "asked_by": request.provenance,
                "options": options,
            });
            let kept = Asked {
                connection_id: connection_id.clone(),
                delivery_id: delivery.delivery_id.clone(),
                sequence: request.sequence,
                options: request
                    .options
                    .iter()
                    .map(|option| (option.option_id.0.to_string(), option.name.clone()))
                    .collect(),
                form: false,
            };
            self.keep_question(&delivery, "permission", asked, kept)
                .await;
        }
    }

    async fn keep_form_questions(
        self: Arc<Self>,
        connection_id: String,
        control: NativeSessionControl,
        delivery: Delivery,
    ) {
        let mut after = 0;
        while let Some(request) = control.elicitation_request_after(after).await {
            after = request.sequence;
            // What a form asks and where a link leads are the engine's and
            // may be secret; the ledger keeps that something is asked, and
            // the question is read from the session while it waits.
            let kind = match &request.request.mode {
                ElicitationMode::Url(_) => "link",
                _ => "form",
            };
            let kept = Asked {
                connection_id: connection_id.clone(),
                delivery_id: delivery.delivery_id.clone(),
                sequence: request.sequence,
                options: BTreeMap::new(),
                form: true,
            };
            self.keep_question(&delivery, kind, json!({}), kept).await;
        }
    }

    async fn keep_question(
        &self,
        delivery: &Delivery,
        kind: &'static str,
        asked: Value,
        kept: Asked,
    ) {
        let (chat_id, agent_id, delivery_id) = (
            delivery.chat_id.clone(),
            delivery.agent_id.clone(),
            delivery.delivery_id.clone(),
        );
        let question = self
            .with_ledger(move |ledger| {
                ledger.ask(&chat_id, &agent_id, Some(&delivery_id), kind, &asked)
            })
            .await;
        if let Ok(question) = question {
            self.chat_runtime
                .asked
                .lock()
                .await
                .insert(question.question_id, kept);
        }
    }

    /// What a form asks or where a link leads, read from the session that
    /// asked, while the question waits.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for a question that can no
    /// longer be answered.
    pub async fn question_in_full(&self, question_id: &str) -> Result<Value, WorkbenchShellError> {
        let asked = self.asked(question_id).await?;
        let connection = self.connection(&asked.connection_id).await?;
        if !asked.form {
            return Ok(json!({ "options": asked.options }));
        }
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
        let asked = self
            .chat_runtime
            .asked
            .lock()
            .await
            .get(question_id)
            .cloned();
        if let Some(asked) = asked {
            return Ok(asked);
        }
        let id = question_id.to_owned();
        self.with_ledger(move |ledger| {
            ledger.question(&id)?;
            ledger.lapse_question(&id)
        })
        .await
        .map_err(ledger_refusal)?;
        Err(no_longer(question_id))
    }

    /// Answer a question that waits: `{"option": id}` for a permission, the
    /// protocol's own action for a form or a link.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for a question that can no
    /// longer be answered and for an answer that was not offered.
    pub async fn answer_in_chat(
        &self,
        question_id: &str,
        answer: Value,
    ) -> Result<Question, WorkbenchShellError> {
        let asked = self.asked(question_id).await?;
        let connection = self.connection(&asked.connection_id).await?;
        let kept = if asked.form {
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
            // What was typed into a form stays between the person and the
            // engine.
            json!({ "action": word })
        } else {
            let option = answer
                .get("option")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    WorkbenchShellError::Invalid("an answer names the option chosen".into())
                })?;
            connection
                .control
                .select_permission(asked.sequence, option)
                .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))?;
            json!({ "option": option, "name": asked.options.get(option) })
        };
        self.chat_runtime.asked.lock().await.remove(question_id);
        let id = question_id.to_owned();
        self.with_ledger(move |ledger| {
            let owner = ledger.owner()?;
            ledger.answer_question(&id, &kept, &owner.participant_id)
        })
        .await
        .map_err(ledger_refusal)
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
