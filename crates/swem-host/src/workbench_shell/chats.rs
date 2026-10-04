//! Chats as a person and a door use them: start one, read one, say
//! something in one, stop what is running, answer what waits.
//!
//! Who a message is for is decided here by one pure function,
//! [`recipients`]; carrying a message to an agent is `runtime`'s.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{WorkbenchShellError, WorkbenchShellState};
use crate::{
    CHANNEL_WORKBENCH, Chat, ChatEvent, Delivery, Message, Participant, ParticipantKind, Question,
    RoutingError,
};

/// What is asked for when a chat is started.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StartChatBody {
    /// What the person calls it; empty, it is known by its first words.
    #[serde(default)]
    pub title: String,
    /// The profiles of the agents in it.
    pub agents: Vec<String>,
}

/// What is said.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Saying {
    #[serde(default)]
    pub text: String,
    /// What was put beside the words, as content blocks of the protocol:
    /// an image, a link to a file, a file itself.
    #[serde(default)]
    pub blocks: Vec<Value>,
    /// What was handed over with it, by the id the content store gave.
    #[serde(default)]
    pub content_refs: Vec<String>,
    /// What an App said the person is looking at, given with the words.
    #[serde(default)]
    pub context: Option<super::model_context::BindModelContextBody>,
    /// The sender's own name for this message, so that saying it again
    /// after a lost answer is not a second message.
    #[serde(default)]
    pub client_ref: Option<String>,
    /// Said for the record, for nobody to answer: what was said in a group
    /// on a messenger's side without speaking to the bot.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub for_the_record: bool,
}

/// A stretch of a chat with what the chat owes and waits for.
#[derive(Clone, Debug, Serialize)]
pub struct ChatPage {
    pub chat: Chat,
    pub events: Vec<ChatEvent>,
    pub messages: Vec<Message>,
    /// Whether there is more before this stretch.
    pub more: bool,
    pub deliveries: Vec<Delivery>,
    pub questions: Vec<Question>,
}

/// What came of saying something.
#[derive(Clone, Debug, Serialize)]
pub struct SaidInChat {
    pub message: Message,
    pub deliveries: Vec<Delivery>,
}

/// One a message is for, and why.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Recipient {
    pub agent_id: String,
    pub why: String,
}

/// Who a message is for.
///
/// One agent in a chat answers everything a person says there. Several
/// answer when they are named. What an agent says is for the agents it
/// names and nobody else, so two agents do not answer each other for ever
/// by being in the same room.
pub(crate) fn recipients(chat: &Chat, sender: &Participant, named: &[String]) -> Vec<Recipient> {
    let agents: Vec<&Participant> = chat
        .members
        .iter()
        .filter(|member| {
            member.kind == ParticipantKind::Agent
                && !member.retired
                && member.participant_id != sender.participant_id
        })
        .collect();
    let by_name = |agents: &[&Participant]| -> Vec<Recipient> {
        agents
            .iter()
            .filter(|agent| {
                named
                    .iter()
                    .any(|handle| handle.eq_ignore_ascii_case(&agent.handle))
            })
            .map(|agent| Recipient {
                agent_id: agent.participant_id.clone(),
                why: format!("named by @{}", sender.handle),
            })
            .collect()
    };
    if sender.kind == ParticipantKind::Agent {
        return by_name(&agents);
    }
    match agents.as_slice() {
        [only] => vec![Recipient {
            agent_id: only.participant_id.clone(),
            why: "the only agent in the chat".to_owned(),
        }],
        several => {
            let named = by_name(several);
            // Where the chat says agents always answer, what names nobody
            // is for all of them; what names somebody is for those named.
            if named.is_empty() && chat.answer_rule == "always" {
                several
                    .iter()
                    .map(|agent| Recipient {
                        agent_id: agent.participant_id.clone(),
                        why: "agents in this chat answer whatever is said".to_owned(),
                    })
                    .collect()
            } else {
                named
            }
        }
    }
}

/// Who an agent's answer goes back to without being named: the agent that
/// asked it by name. An agent that asks another is told what it answered;
/// what it says next goes on only to who it names, so two agents do not
/// answer each other for ever by having spoken once.
pub(crate) fn asker(
    chat: &Chat,
    agent: &Participant,
    asked_by: &Participant,
    asked_named: &[String],
) -> Option<Recipient> {
    let asked_it = asked_by.kind == ParticipantKind::Agent
        && asked_by.participant_id != agent.participant_id
        && asked_named
            .iter()
            .any(|handle| handle.eq_ignore_ascii_case(&agent.handle));
    let still_here = chat
        .members
        .iter()
        .any(|member| member.participant_id == asked_by.participant_id && !member.retired);
    (asked_it && still_here).then(|| Recipient {
        agent_id: asked_by.participant_id.clone(),
        why: format!("answered by @{}", agent.handle),
    })
}

/// A refusal of the ledger as the shell says it.
pub(super) fn ledger_refusal(error: RoutingError) -> WorkbenchShellError {
    match error {
        RoutingError::InvalidBinding(said)
            if said.starts_with("there is no ") || said.starts_with("nobody is ") =>
        {
            WorkbenchShellError::NotFound(said)
        }
        RoutingError::InvalidBinding(said) => WorkbenchShellError::Invalid(said),
        RoutingError::RouteNotFound(route) => {
            WorkbenchShellError::NotFound(format!("unknown route {route}"))
        }
        other => WorkbenchShellError::Failed(other.to_string()),
    }
}

impl WorkbenchShellState {
    /// The person this Workbench is, and everybody it knows.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn chat_people(&self) -> Result<Value, WorkbenchShellError> {
        // Every profile is an agent a person can talk to, whether or not
        // it has been talked to yet.
        let profiles: Vec<String> = self
            .profiles()?
            .into_iter()
            .map(|profile| profile.profile_id)
            .collect();
        self.with_ledger(move |ledger| {
            let owner = ledger.owner()?;
            for profile_id in &profiles {
                ledger.agent_of_profile(profile_id)?;
            }
            Ok(json!({ "owner": owner, "participants": ledger.participants()? }))
        })
        .await
        .map_err(ledger_refusal)
    }

    /// The chats of the person this Workbench is, the one that moved last
    /// first.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn chats(&self) -> Result<Vec<Chat>, WorkbenchShellError> {
        self.with_ledger(|ledger| {
            let owner = ledger.owner()?;
            ledger.chats_of(&owner.participant_id)
        })
        .await
        .map_err(ledger_refusal)
    }

    /// Start a chat between the person and the agents they name.
    ///
    /// # Errors
    ///
    /// Refuses a chat with no agent and an agent that is no profile.
    pub async fn start_chat(&self, body: StartChatBody) -> Result<Chat, WorkbenchShellError> {
        if body.agents.is_empty() {
            return Err(WorkbenchShellError::Invalid(
                "a chat is started with at least one agent".into(),
            ));
        }
        if body.title.chars().count() > 160 || body.title.chars().any(char::is_control) {
            return Err(WorkbenchShellError::Invalid(
                "a chat's name is one line of at most a hundred and sixty characters".into(),
            ));
        }
        for profile_id in &body.agents {
            self.inventory
                .select(profile_id)
                .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        }
        self.with_ledger(move |ledger| {
            let owner = ledger.owner()?;
            let mut agents = Vec::new();
            for profile_id in &body.agents {
                agents.push(ledger.agent_of_profile(profile_id)?.participant_id);
            }
            ledger.start_chat(&body.title, &owner.participant_id, &agents)
        })
        .await
        .map_err(ledger_refusal)
    }

    /// Bring an agent into a chat, by its profile.
    ///
    /// # Errors
    ///
    /// Not found for a profile or a chat there is not.
    pub async fn bring_into_chat(
        &self,
        chat_id: &str,
        profile_id: &str,
    ) -> Result<Chat, WorkbenchShellError> {
        self.inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let (chat_id, profile_id) = (chat_id.to_owned(), profile_id.to_owned());
        self.with_ledger(move |ledger| {
            ledger.chat(&chat_id)?;
            let agent = ledger.agent_of_profile(&profile_id)?;
            ledger.join_chat(&chat_id, &agent.participant_id)
        })
        .await
        .map_err(ledger_refusal)
    }

    /// Let a guest into a chat: somebody who wrote to a bot of yours and
    /// was told to wait. From then on what they write to the bot alone is
    /// said in this chat, and what is said there reaches them.
    ///
    /// # Errors
    ///
    /// Not found for a chat there is not; invalid for somebody who is not a
    /// guest.
    pub async fn let_guest_into_chat(
        &self,
        chat_id: &str,
        participant_id: &str,
    ) -> Result<Chat, WorkbenchShellError> {
        let (chat_id, guest_id) = (chat_id.to_owned(), participant_id.to_owned());
        self.with_ledger(move |ledger| {
            ledger.chat(&chat_id)?;
            let guest = ledger.participant(&guest_id)?;
            if guest.kind != crate::ParticipantKind::Guest {
                return Err(crate::RoutingError::InvalidBinding(format!(
                    "{} is not a guest",
                    guest.name
                )));
            }
            let chat = ledger.join_chat(&chat_id, &guest_id)?;
            // Let into a chat, they may speak, and what they write to the
            // bot alone is said here.
            for identity in ledger.identities_of(&guest_id)? {
                ledger.let_speak(&identity.channel_id, &guest_id, true)?;
                if let Some(direct) = identity.direct_chat {
                    ledger.bind_channel_chat(&identity.channel_id, &direct, &chat_id)?;
                }
            }
            Ok(chat)
        })
        .await
        .map_err(ledger_refusal)
    }

    /// Take somebody out of a chat. A session the agent had open for the
    /// chat is let go of.
    ///
    /// # Errors
    ///
    /// Refuses the one who started the chat and somebody who is not in it.
    pub async fn take_out_of_chat(
        &self,
        chat_id: &str,
        participant_id: &str,
    ) -> Result<Chat, WorkbenchShellError> {
        let (chat, participant) = (chat_id.to_owned(), participant_id.to_owned());
        let left = self
            .with_ledger(move |ledger| {
                let left = ledger.leave_chat(&chat, &participant)?;
                // A guest taken out is not reached from here any more.
                for identity in ledger.identities_of(&participant)? {
                    if let Some(direct) = identity.direct_chat
                        && ledger
                            .chat_of_channel_chat(&identity.channel_id, &direct)?
                            .as_deref()
                            == Some(chat.as_str())
                    {
                        ledger.unbind_channel_chat(&identity.channel_id, &direct)?;
                    }
                }
                Ok(left)
            })
            .await
            .map_err(ledger_refusal)?;
        self.let_go_of_in(chat_id, participant_id).await;
        Ok(left)
    }

    /// Set a chat's rules.
    ///
    /// # Errors
    ///
    /// Refuses a rule there is not and a limit outside one to twenty.
    pub async fn rule_chat(
        &self,
        chat_id: &str,
        answer_rule: Option<String>,
        reply_limit: Option<u32>,
    ) -> Result<Chat, WorkbenchShellError> {
        let chat_id = chat_id.to_owned();
        self.with_ledger(move |ledger| {
            ledger.rule_chat(&chat_id, answer_rule.as_deref(), reply_limit)
        })
        .await
        .map_err(ledger_refusal)
    }

    /// The chat somebody names, by its own id or by the id of a session of
    /// an engine it holds: an editor that kept the id of a conversation from
    /// before there were chats comes back to the chat it became.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] when the id names neither.
    pub async fn chat_named(&self, id: &str) -> Result<Chat, WorkbenchShellError> {
        let id = id.to_owned();
        self.with_ledger(move |ledger| match ledger.session_of_route(&id)? {
            Some(session) => ledger.chat(&session.chat_id),
            None => ledger.chat(&id),
        })
        .await
        .map_err(ledger_refusal)
    }

    /// What happened in a chat after a place, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn happened_in_chat(
        &self,
        chat_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<ChatEvent>, WorkbenchShellError> {
        let chat_id = chat_id.to_owned();
        self.with_ledger(move |ledger| ledger.happened_in_chat_after(&chat_id, after, limit))
            .await
            .map_err(ledger_refusal)
    }

    /// A stretch of a chat ending before a place, or at its end, with what
    /// the chat owes and waits for now.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for a chat that does not
    /// exist.
    pub async fn chat_page(
        &self,
        chat_id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<ChatPage, WorkbenchShellError> {
        let chat_id = chat_id.to_owned();
        self.with_ledger(move |ledger| {
            let chat = ledger.chat(&chat_id)?;
            let (events, messages, more) = ledger.timeline(&chat_id, before, limit)?;
            Ok(ChatPage {
                chat,
                events,
                messages,
                more,
                deliveries: ledger.deliveries_open(Some(&chat_id))?,
                questions: ledger.questions_waiting(Some(&chat_id))?,
            })
        })
        .await
        .map_err(ledger_refusal)
    }

    /// Say something in a chat as the person this Workbench is, through the
    /// page.
    ///
    /// # Errors
    ///
    /// As [`Self::say_in_chat_as`].
    pub async fn say_in_chat(
        self: &Arc<Self>,
        chat_id: &str,
        saying: Saying,
    ) -> Result<SaidInChat, WorkbenchShellError> {
        self.say_in_chat_as(chat_id, None, CHANNEL_WORKBENCH, saying)
            .await
    }

    /// Say something in a chat. The message is kept, owed to the agents it
    /// is for, and those agents are set to work; the answer is not waited
    /// for.
    ///
    /// `sender` is a participant of the chat; absent, it is the person this
    /// Workbench is.
    ///
    /// # Errors
    ///
    /// Refuses an empty message, an unknown chat, a sender who is not in
    /// the chat, and content that was never handed over.
    pub async fn say_in_chat_as(
        self: &Arc<Self>,
        chat_id: &str,
        sender: Option<String>,
        channel: &str,
        saying: Saying,
    ) -> Result<SaidInChat, WorkbenchShellError> {
        if saying.text.trim().is_empty()
            && saying.content_refs.is_empty()
            && saying.blocks.is_empty()
        {
            return Err(WorkbenchShellError::Invalid(
                "there is nothing to say".into(),
            ));
        }
        let mut content = Vec::new();
        if !saying.text.is_empty() {
            content.push(json!({ "type": "text", "text": saying.text }));
        }
        for block in &saying.blocks {
            serde_json::from_value::<agent_client_protocol::schema::v1::ContentBlock>(
                block.clone(),
            )
            .map_err(|error| {
                WorkbenchShellError::Invalid(format!(
                    "what was put beside the words is not content of the protocol: {error}"
                ))
            })?;
            content.push(block.clone());
        }
        if let Some(context) = &saying.context {
            super::model_context::validate(context)?;
            content.push(json!({
                "type": "context",
                "server_name": context.server_name,
                "content": context.content,
            }));
        }
        for descriptor_id in &saying.content_refs {
            let descriptor = self.content.load(descriptor_id).await?;
            content.push(json!({
                "type": "attachment",
                "content_ref": descriptor.descriptor_id,
                "name": descriptor.name,
                "media_type": descriptor.media_type,
                "byte_length": descriptor.byte_length,
            }));
        }
        let content = Value::Array(content);
        let chat_id = chat_id.to_owned();
        let channel = channel.to_owned();
        let said = self
            .with_ledger(move |ledger| {
                let chat = ledger.chat(&chat_id)?;
                let sender = match sender {
                    Some(sender) => ledger.participant(&sender)?,
                    None => ledger.owner()?,
                };
                let message = ledger.say(
                    &chat_id,
                    &sender.participant_id,
                    &channel,
                    saying.client_ref.as_deref(),
                    &content,
                )?;
                let agents: Vec<String> = if saying.for_the_record {
                    Vec::new()
                } else {
                    recipients(&chat, &sender, &message.named)
                        .into_iter()
                        .map(|recipient| recipient.agent_id)
                        .collect()
                };
                let deliveries = ledger.deliver(&message.message_id, &agents)?;
                Ok(SaidInChat {
                    message,
                    deliveries,
                })
            })
            .await
            .map_err(ledger_refusal)?;
        for delivery in &said.deliveries {
            self.set_to_work(&delivery.agent_id);
        }
        Ok(said)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn participant(handle: &str, kind: ParticipantKind) -> Participant {
        Participant {
            participant_id: format!("p_{handle}"),
            kind,
            handle: handle.to_owned(),
            name: handle.to_owned(),
            colour: None,
            profile_id: None,
            made_by: None,
            retired: false,
        }
    }

    fn chat(members: Vec<Participant>) -> Chat {
        Chat {
            chat_id: "c_1".into(),
            title: "Release".into(),
            created_by: "p_ada".into(),
            answer_rule: "named".into(),
            reply_limit: 4,
            agent_replies: 0,
            members,
            last_sequence: None,
            last_at_ms: None,
        }
    }

    fn named(handles: &[&str]) -> Vec<String> {
        handles.iter().map(|handle| (*handle).to_owned()).collect()
    }

    fn ids(recipients: &[Recipient]) -> Vec<&str> {
        recipients
            .iter()
            .map(|recipient| recipient.agent_id.as_str())
            .collect()
    }

    #[test]
    fn where_agents_always_answer_what_names_nobody_is_for_all_of_them() {
        let ada = participant("ada", ParticipantKind::Person);
        let coder = participant("coder", ParticipantKind::Agent);
        let reviewer = participant("reviewer", ParticipantKind::Agent);
        let mut chat = chat(vec![ada.clone(), coder.clone(), reviewer]);
        chat.answer_rule = "always".into();
        let for_whom = recipients(&chat, &ada, &[]);
        assert_eq!(ids(&for_whom), vec!["p_coder", "p_reviewer"]);
        assert_eq!(
            for_whom[0].why,
            "agents in this chat answer whatever is said"
        );
        // Naming somebody still means them alone.
        assert_eq!(
            ids(&recipients(&chat, &ada, &named(&["reviewer"]))),
            vec!["p_reviewer"]
        );
        // And what an agent says is for who it names, whatever the rule:
        // two agents do not answer each other for being in one room.
        assert!(recipients(&chat, &coder, &[]).is_empty());
    }

    #[test]
    fn an_agent_that_asked_by_name_is_told_the_answer_and_no_more() {
        let ada = participant("ada", ParticipantKind::Person);
        let coder = participant("coder", ParticipantKind::Agent);
        let reviewer = participant("reviewer", ParticipantKind::Agent);
        let chat = chat(vec![ada.clone(), coder.clone(), reviewer.clone()]);
        // The coder asked the reviewer by name: the answer goes back.
        let back = asker(&chat, &reviewer, &coder, &named(&["Reviewer"])).expect("the asker");
        assert_eq!(back.agent_id, "p_coder");
        assert_eq!(back.why, "answered by @reviewer");
        // What the coder says to that was not asked for by name: it goes
        // on only to who it names.
        assert!(asker(&chat, &coder, &reviewer, &[]).is_none());
        // A person is not an agent to be given a turn, and an agent does
        // not answer itself.
        assert!(asker(&chat, &reviewer, &ada, &named(&["reviewer"])).is_none());
        assert!(asker(&chat, &coder, &coder, &named(&["coder"])).is_none());
        // One who left the chat is not told.
        let without = super::tests::chat(vec![ada, reviewer.clone()]);
        assert!(asker(&without, &reviewer, &coder, &named(&["reviewer"])).is_none());
    }

    #[test]
    fn one_agent_answers_everything_a_person_says() {
        let ada = participant("ada", ParticipantKind::Person);
        let coder = participant("coder", ParticipantKind::Agent);
        let chat = chat(vec![ada.clone(), coder]);
        let for_whom = recipients(&chat, &ada, &[]);
        assert_eq!(ids(&for_whom), vec!["p_coder"]);
        assert_eq!(for_whom[0].why, "the only agent in the chat");
    }

    #[test]
    fn several_agents_answer_when_they_are_named() {
        let ada = participant("ada", ParticipantKind::Person);
        let coder = participant("coder", ParticipantKind::Agent);
        let reviewer = participant("reviewer", ParticipantKind::Agent);
        let mut gone = participant("gone", ParticipantKind::Agent);
        gone.retired = true;
        let chat = chat(vec![ada.clone(), coder.clone(), reviewer, gone]);
        assert!(recipients(&chat, &ada, &[]).is_empty());
        let for_whom = recipients(&chat, &ada, &named(&["Reviewer", "gone", "nobody"]));
        assert_eq!(ids(&for_whom), vec!["p_reviewer"]);
        assert_eq!(for_whom[0].why, "named by @ada");
        assert_eq!(
            ids(&recipients(&chat, &ada, &named(&["coder", "reviewer"]))),
            vec!["p_coder", "p_reviewer"]
        );
        // What an agent says is for those it names, never for itself.
        assert!(recipients(&chat, &coder, &[]).is_empty());
        assert_eq!(
            ids(&recipients(&chat, &coder, &named(&["coder", "reviewer"]))),
            vec!["p_reviewer"]
        );
    }

    #[test]
    fn an_agent_alone_with_another_is_not_answered_unless_it_names_it() {
        let ada = participant("ada", ParticipantKind::Person);
        let coder = participant("coder", ParticipantKind::Agent);
        let reviewer = participant("reviewer", ParticipantKind::Agent);
        let chat = chat(vec![ada, coder.clone(), reviewer]);
        assert!(recipients(&chat, &coder, &[]).is_empty());
    }
}
