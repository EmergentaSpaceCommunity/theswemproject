//! Channels: how people reach an agent from a messenger.
//!
//! A channel is a package of kind `swem/channel@1` (`swem_sdk::channel`): a
//! program the harness starts and keeps, and talks to through the tools of
//! that shape. The harness names no messenger. What it does: it pulls what
//! arrived and says it in the chat the messenger's chat is bound to, as the
//! participant the messenger's person is bound to; and it carries what an
//! agent says in that chat back, as it is written.
//!
//! A channel a person added is a document under `<root>/channels/<id>.json`
//! (who runs it, which agent answers in it, what guests may do) and a key
//! kept with every other key as `channel-<id>`. One runner per channel, as
//! the timekeeper is one loop.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use swem_sdk::channel::{self, Bot, ChatKind, Inbound, Person};

use super::chats::Saying;
use super::{WorkbenchShellError, WorkbenchShellState};
use crate::chat_ledger::{ParticipantKind, channel_of, now_ms};
use crate::workbench_apps::{AppAttachmentEntry, call_tool_of, discover_server};

/// The document a channel is.
pub const CHANNEL_SCHEMA: &str = "swem:channel@0.1";

/// What a stranger may do with the bot.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestPolicy {
    /// The bot answers its owner only; a stranger is told so, once.
    #[default]
    Nobody,
    /// A stranger is let in only when the owner joins them to a chat from
    /// the page.
    ByInvitation,
    /// A stranger gets a chat of their own with the bot's agent.
    Anyone,
}

/// A channel a person added, as kept on disk. Never the key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChannelDocument {
    #[serde(default = "channel_schema")]
    pub schema: String,
    pub id: String,
    /// What the person called it.
    pub name: String,
    /// The channel package that runs it: the id of its catalog entry, or
    /// `program` when it is a program of the person's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    /// A program of the person's own that answers the channel shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// The agent that answers in this channel's chats, by profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default)]
    pub guests: GuestPolicy,
    /// How what the messenger has gets here: asked for, or delivered to
    /// a door of this Workbench, which needs it served at an address.
    #[serde(default)]
    pub reach: Reach,
    /// Where the page the bot opens inside the messenger is hosted, when
    /// not by this Workbench: a static copy of `web/mini-app` anywhere
    /// with HTTPS. The bot's button then carries this Workbench's address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_at: Option<String>,
    #[serde(default)]
    pub settings: channel::Settings,
    /// The code the owner says to the bot once, to be known there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pairing_code: Option<String>,
    /// Who the bot was, when it was looked at last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot: Option<Bot>,
    pub added_ms: i64,
}

/// How a channel receives what its messenger has.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// The channel asks the messenger; works anywhere.
    #[default]
    Pull,
    /// The messenger delivers to `/api/channels/<id>/receive`, when this
    /// Workbench is served at an address.
    Door,
}

fn channel_schema() -> String {
    CHANNEL_SCHEMA.to_owned()
}

/// A channel as the page shows it: the document, and how it is doing.
#[derive(Clone, Debug, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "four standings the page shows as four words; no two are one state"
)]
pub struct ChannelShown {
    #[serde(flatten)]
    pub document: ChannelDocument,
    /// Whether its program runs now.
    pub running: bool,
    /// Whether a key was given.
    pub keyed: bool,
    /// Whether the owner was bound to somebody on the messenger's side.
    pub paired: bool,
    /// What is wrong with it, in words; empty when nothing is.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub said: String,
    /// Everybody the bot has met but the owner, and who of them may speak.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub people: Vec<GuestSeen>,
    /// Whether a door can be offered: this Workbench is served at an address.
    pub door_offered: bool,
    /// The door the messenger was given, when the channel is reached at one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<String>,
}

/// What the page may change about a channel once it is added.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChangeChannelBody {
    #[serde(default)]
    pub reach: Option<Reach>,
    /// Where the page inside the messenger is hosted; empty for here.
    #[serde(default)]
    pub app_at: Option<String>,
    #[serde(default)]
    pub guests: Option<GuestPolicy>,
    #[serde(default)]
    pub agent: Option<String>,
}

/// Somebody who opened the page inside the messenger, as the door learns
/// them at the exchange: who they are here, and the bot's agent.
pub(super) struct MessengerPerson {
    pub participant: crate::Participant,
    /// The messenger's id for them.
    pub external_id: String,
    pub is_owner: bool,
    /// The bot's agent, by profile and as a participant.
    pub agent_profile: String,
    pub agent_id: String,
}

/// Who opened a channel's Mini App, and what is theirs here.
struct AppPerson {
    participant: crate::Participant,
    /// The owner sees what the agent put out; a guest sees their chat only.
    is_owner: bool,
    /// Whether the agent takes a turn on what they send.
    may_speak: bool,
    chat_id: String,
    /// The bot's agent, by profile.
    agent: String,
    bot: Bot,
}

/// Somebody a bot has met, and whether the agent takes a turn on what they
/// say; the owner is not listed here.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct GuestSeen {
    pub participant_id: String,
    pub name: String,
    pub may_speak: bool,
    /// Whether they have written to the bot alone.
    pub alone: bool,
}

/// What can run a channel here: a package the Store installed, or one that
/// came with the product.
#[derive(Clone, Debug, Serialize)]
pub struct ChannelPackage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    pub bundled: bool,
}

/// What the Channels page reads.
#[derive(Clone, Debug, Serialize)]
pub struct ChannelsStanding {
    pub channels: Vec<ChannelShown>,
    pub packages: Vec<ChannelPackage>,
    /// Where the product hosts the page a bot opens inside the messenger,
    /// if it does: what a bot is told when the person names no host.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_hosted_at: Option<String>,
}

/// What a person gives to add a channel.
#[derive(Clone, Debug, Deserialize)]
pub struct AddChannelBody {
    pub name: String,
    #[serde(default)]
    pub package: Option<String>,
    #[serde(default)]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub guests: GuestPolicy,
    #[serde(default)]
    pub settings: channel::Settings,
    /// Where the page inside the messenger is hosted, when not here.
    #[serde(default)]
    pub app_at: Option<String>,
    /// The bot's secret; kept with the keys, never in the document.
    #[serde(default)]
    pub key: Option<String>,
}

/// What a channel's runner holds while it runs.
struct Running {
    entry: Arc<AppAttachmentEntry>,
    stop: Arc<tokio::sync::Notify>,
    said: Arc<Mutex<String>>,
}

/// The channels of one Workbench.
pub(crate) struct Channels {
    root: PathBuf,
    running: Mutex<BTreeMap<String, Running>>,
}

impl Channels {
    fn document_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.json"))
    }

    fn home_of(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    fn read(&self, id: &str) -> Result<ChannelDocument, WorkbenchShellError> {
        let path = self.document_path(id);
        let bytes = std::fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                WorkbenchShellError::NotFound(format!("no channel called {id}"))
            } else {
                WorkbenchShellError::Failed(format!("read {}: {error}", path.display()))
            }
        })?;
        serde_json::from_slice(&bytes)
            .map_err(|error| WorkbenchShellError::Failed(format!("{}: {error}", path.display())))
    }

    fn write(&self, document: &ChannelDocument) -> Result<(), WorkbenchShellError> {
        std::fs::create_dir_all(&self.root)
            .map_err(|error| WorkbenchShellError::Failed(format!("make channels: {error}")))?;
        let path = self.document_path(&document.id);
        let bytes = serde_json::to_vec_pretty(document)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        crate::closed::write(&path, &bytes)
            .map_err(|error| WorkbenchShellError::Failed(format!("{}: {error}", path.display())))
    }

    fn all(&self) -> Result<Vec<ChannelDocument>, WorkbenchShellError> {
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(Vec::new());
        };
        let mut documents = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) {
                documents.push(self.read(id)?);
            }
        }
        documents.sort_by_key(|document| document.added_ms);
        Ok(documents)
    }
}

/// The name a channel's key is kept under: beside the model providers' keys.
#[must_use]
pub fn key_name_of(id: &str) -> String {
    format!("channel-{id}")
}

fn pairing_code() -> Result<String, WorkbenchShellError> {
    // Six digits are enough: the code is said to one bot, by one person,
    // once; a wrong code is just a message.
    let secret = crate::mint_session_token().map_err(WorkbenchShellError::Failed)?;
    let digits: String = secret
        .bytes()
        .map(|byte| char::from(b'0' + byte % 10))
        .take(6)
        .collect();
    Ok(digits)
}

impl WorkbenchShellState {
    /// Keep channels under this directory, and start every channel that
    /// was added.
    ///
    /// # Errors
    ///
    /// Channels are enabled already, or the directory cannot be read.
    pub fn enable_channels(self: &Arc<Self>, root: &Path) -> Result<(), WorkbenchShellError> {
        self.channels
            .set(Channels {
                root: root.to_path_buf(),
                running: Mutex::new(BTreeMap::new()),
            })
            .map_err(|_| WorkbenchShellError::Conflict("channels are enabled already".into()))?;
        for document in self.channels()?.all()? {
            let state = Arc::clone(self);
            tokio::spawn(async move {
                let _ = state.start_channel(&document.id).await;
            });
        }
        Ok(())
    }

    fn channels(&self) -> Result<&Channels, WorkbenchShellError> {
        self.channels
            .get()
            .ok_or_else(|| WorkbenchShellError::Conflict("channels are not enabled here".into()))
    }

    /// The channels a person added, as the page shows them.
    ///
    /// # Errors
    ///
    /// Channels are not enabled, or a document cannot be read.
    pub async fn channels_shown(
        self: &Arc<Self>,
    ) -> Result<Vec<ChannelShown>, WorkbenchShellError> {
        let channels = self.channels()?;
        let mut shown = Vec::new();
        for document in channels.all()? {
            shown.push(self.channel_shown(document).await?);
        }
        Ok(shown)
    }

    async fn channel_shown(
        self: &Arc<Self>,
        document: ChannelDocument,
    ) -> Result<ChannelShown, WorkbenchShellError> {
        let channels = self.channels()?;
        let (running, said) = {
            let running = channels
                .running
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match running.get(&document.id) {
                Some(run) => (
                    true,
                    run.said
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone(),
                ),
                None => (false, String::new()),
            }
        };
        let keyed = self
            .provider_keys
            .get()
            .and_then(|keys| keys.held(&key_name_of(&document.id)).ok().flatten())
            .is_some();
        let id = document.id.clone();
        let (paired, people) = self
            .with_ledger(move |ledger| {
                let owner = ledger.owner()?;
                let mut paired = false;
                let mut guests = Vec::new();
                for identity in ledger.identities_at(&id)? {
                    if identity.participant_id == owner.participant_id {
                        paired = true;
                        continue;
                    }
                    let Ok(participant) = ledger.participant(&identity.participant_id) else {
                        continue;
                    };
                    if participant.retired {
                        continue;
                    }
                    guests.push(GuestSeen {
                        participant_id: identity.participant_id,
                        name: participant.name,
                        may_speak: identity.may_speak,
                        alone: identity.direct_chat.is_some(),
                    });
                }
                Ok((paired, guests))
            })
            .await
            .unwrap_or((false, Vec::new()));
        let door = (document.reach == Reach::Door)
            .then(|| self.door_of(&document.id))
            .flatten();
        Ok(ChannelShown {
            document,
            running,
            keyed,
            paired,
            said,
            people,
            door_offered: self.served_origin().is_some(),
            door,
        })
    }

    /// The address of the page a bot opens inside the messenger, for this
    /// Workbench at `origin`: the host the person named for the bot, else
    /// the product's own copy, else the Workbench's page itself.
    pub(super) fn app_url_for(&self, document: &ChannelDocument, origin: &str) -> String {
        match document
            .app_at
            .clone()
            .or_else(|| self.app_hosted_at().map(str::to_owned))
        {
            // Hosted elsewhere: the page is told where this Workbench is and
            // which channel.
            Some(at) => format!("{at}/?at={origin}&channel={}", document.id),
            None => format!("{origin}/channels/{}/app", document.id),
        }
    }

    /// Where a messenger delivers for a channel, when this Workbench is
    /// served at an address.
    fn door_of(&self, id: &str) -> Option<String> {
        self.served_origin()
            .map(|origin| format!("{origin}/api/channels/{id}/receive"))
    }

    /// Change how a channel is reached, who may write to it or who answers;
    /// the channel is started again with the change.
    ///
    /// # Errors
    ///
    /// No such channel; a door asked for where none can be offered.
    pub async fn change_channel(
        self: &Arc<Self>,
        id: &str,
        body: ChangeChannelBody,
    ) -> Result<ChannelShown, WorkbenchShellError> {
        let channels = self.channels()?;
        let mut document = channels.read(id)?;
        if let Some(reach) = body.reach {
            if reach == Reach::Door && self.served_origin().is_none() {
                return Err(WorkbenchShellError::Invalid(
                    "a door needs this Workbench served at an address: see docs/serving.md".into(),
                ));
            }
            document.reach = reach;
        }
        if let Some(guests) = body.guests {
            document.guests = guests;
        }
        if let Some(app_at) = body.app_at {
            let app_at = app_at.trim().trim_end_matches('/').to_owned();
            document.app_at = (!app_at.is_empty()).then_some(app_at);
        }
        if let Some(agent) = body.agent {
            document.agent = (!agent.trim().is_empty()).then(|| agent.trim().to_owned());
        }
        channels.write(&document)?;
        let started = self.start_channel(id).await;
        let mut shown = self.channel_shown(channels.read(id)?).await?;
        if let Err(error) = started {
            shown.said = error.to_string();
        }
        Ok(shown)
    }

    /// Who opened the page inside the messenger, by the messenger's
    /// signature: the channel package says who, the ledger says who that is
    /// here. Somebody the bot never met has nothing here.
    ///
    /// # Errors
    ///
    /// The channel does not run or does not answer `verify_app`; the data
    /// is not the messenger's; the person is not known here; no agent
    /// answers for the bot yet.
    pub(super) async fn messenger_person(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
    ) -> Result<MessengerPerson, WorkbenchShellError> {
        let channels = self.channels()?;
        let document = channels.read(id)?;
        let entry = channels
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .map(|run| Arc::clone(&run.entry))
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no channel {id} runs")))?;
        let person: Person = call_tool_of(
            &entry,
            channel::VERIFY_APP,
            json!({ "init_data": init_data }),
        )
        .await
        .and_then(|answer| serde_json::from_value(answer).map_err(|error| error.to_string()))
        .map_err(WorkbenchShellError::Forbidden)?;
        let agent_profile = document
            .agent
            .clone()
            .ok_or_else(|| WorkbenchShellError::Invalid("no agent answers here yet".into()))?;
        let (channel, external, profile) =
            (id.to_owned(), person.id.clone(), agent_profile.clone());
        let found = self
            .with_ledger(move |ledger| {
                let Some(participant) = ledger.participant_of_identity(&channel, &external)? else {
                    return Ok(None);
                };
                let owner = ledger.owner()?;
                let is_owner = participant.participant_id == owner.participant_id;
                let agent = ledger.agent_of_profile(&profile)?;
                Ok(Some((participant, is_owner, agent.participant_id)))
            })
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let Some((participant, is_owner, agent_id)) = found else {
            return Err(WorkbenchShellError::Forbidden(
                "write to the bot first; then this page knows you".into(),
            ));
        };
        Ok(MessengerPerson {
            participant,
            external_id: person.id,
            is_owner,
            agent_profile,
            agent_id,
        })
    }

    /// Who opened a channel's Mini App, by the messenger's signature, and
    /// the chat they have through the bot.
    ///
    /// # Errors
    ///
    /// The channel does not run or cannot verify; the data is not the
    /// messenger's; the person is not known here or has no chat yet.
    async fn app_person(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
    ) -> Result<AppPerson, WorkbenchShellError> {
        let channels = self.channels()?;
        let document = channels.read(id)?;
        let entry = channels
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .map(|run| Arc::clone(&run.entry))
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no channel {id} runs")))?;
        let person: Person = call_tool_of(
            &entry,
            channel::VERIFY_APP,
            json!({ "init_data": init_data }),
        )
        .await
        .and_then(|answer| serde_json::from_value(answer).map_err(|error| error.to_string()))
        .map_err(WorkbenchShellError::Forbidden)?;
        let (channel, external) = (id.to_owned(), person.id.clone());
        // The chat the page is about: the one they have with the bot alone,
        // else a chat through this bot they are in. Somebody in none of them
        // has nothing here; nobody gets more than their own chats.
        let found = self
            .with_ledger(move |ledger| {
                let Some(participant) = ledger.participant_of_identity(&channel, &external)? else {
                    return Ok(None);
                };
                let is_owner = participant.kind == ParticipantKind::Person;
                let may_speak = is_owner
                    || ledger
                        .identity_of(&channel, &external)?
                        .is_some_and(|identity| identity.may_speak);
                if let Some(direct) =
                    ledger.direct_chat_of(&channel, &participant.participant_id)?
                    && let Some(chat_id) = ledger.chat_of_channel_chat(&channel, &direct)?
                {
                    return Ok(Some((participant, is_owner, may_speak, chat_id)));
                }
                for bound in ledger.chats_of_channel(&channel)? {
                    let chat = ledger.chat(&bound.chat_id)?;
                    if chat.members.iter().any(|member| {
                        member.participant_id == participant.participant_id && !member.retired
                    }) {
                        return Ok(Some((participant, is_owner, may_speak, bound.chat_id)));
                    }
                }
                Ok(None)
            })
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let Some((participant, is_owner, may_speak, chat_id)) = found else {
            return Err(WorkbenchShellError::Forbidden(
                "write to the bot first; then this page knows you".into(),
            ));
        };
        let agent = document
            .agent
            .clone()
            .ok_or_else(|| WorkbenchShellError::Invalid("no agent answers here yet".into()))?;
        Ok(AppPerson {
            participant,
            is_owner,
            may_speak,
            chat_id,
            agent,
            bot: document.bot.clone().unwrap_or_default(),
        })
    }

    /// What a channel's Mini App shows: who you are here, the chat's last
    /// words, and what the agent put out.
    ///
    /// # Errors
    ///
    /// As [`Self::app_person`].
    pub async fn app_standing(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
    ) -> Result<Value, WorkbenchShellError> {
        let who = self.app_person(id, init_data).await?;
        self.touch_tunnel();
        let page = self.chat_page(&who.chat_id, None, 50).await?;
        let workspace = self
            .inventory
            .select(&who.agent)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?
            .workspace;
        // What the agent put out is the owner's to take, not a guest's.
        let files: Vec<crate::HandedFile> = if who.is_owner {
            crate::workbench_files::list(&workspace)
                .await?
                .into_iter()
                .filter(|file| file.area == crate::workbench_files::OUTBOX)
                .collect()
        } else {
            Vec::new()
        };
        let names: BTreeMap<String, String> = page
            .chat
            .members
            .iter()
            .map(|member| (member.participant_id.clone(), member.name.clone()))
            .collect();
        Ok(json!({
            "you": { "participant_id": who.participant.participant_id, "name": who.participant.name },
            "owner": who.is_owner,
            "bot": who.bot,
            "agent": who.agent,
            "chat": { "chat_id": who.chat_id, "title": page.chat.title },
            "messages": page.messages.iter().map(|message| json!({
                "by": names.get(&message.sender_id).cloned().unwrap_or_default(),
                "text": message.text,
                "at_ms": message.created_ms,
            })).collect::<Vec<_>>(),
            "files": files,
        }))
    }

    /// A file of any size, from the Mini App into the chat: kept as content
    /// once the person is known, and said in the chat with their words, so
    /// it reaches the agent's inbox as a file handed over on the page does.
    ///
    /// # Errors
    ///
    /// As [`Self::app_person`]; or the file cannot be kept.
    pub async fn app_upload<F>(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
        words: &str,
        ingest: F,
    ) -> Result<Value, WorkbenchShellError>
    where
        F: Future<Output = Result<crate::WorkbenchContentDescriptor, WorkbenchShellError>>,
    {
        // Who asks is known before a byte is kept.
        let who = self.app_person(id, init_data).await?;
        self.touch_tunnel();
        let descriptor = ingest.await?;
        let name = descriptor.name.clone();
        let said = self
            .say_in_chat_as(
                &who.chat_id,
                Some(who.participant.participant_id),
                &channel_of(id),
                Saying {
                    text: if words.trim().is_empty() {
                        format!("Sent {name} through the app.")
                    } else {
                        words.to_owned()
                    },
                    blocks: Vec::new(),
                    content_refs: vec![descriptor.descriptor_id.clone()],
                    context: None,
                    client_ref: None,
                    for_the_record: !who.may_speak,
                },
            )
            .await?;
        Ok(json!({
            "message_id": said.message.message_id,
            "name": name,
            "byte_length": descriptor.byte_length,
        }))
    }

    /// A question the agent asks, for the page inside the messenger to draw:
    /// for the owner, who answers it.
    ///
    /// # Errors
    ///
    /// As [`Self::app_person`]; somebody who is not the owner; a question
    /// no longer waiting.
    pub async fn app_question(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
        question_id: &str,
    ) -> Result<Value, WorkbenchShellError> {
        let who = self.app_person(id, init_data).await?;
        self.touch_tunnel();
        if !who.is_owner {
            return Err(WorkbenchShellError::Forbidden(
                "the agent's questions are its owner's to answer".into(),
            ));
        }
        self.question_in_full(question_id).await
    }

    /// The owner's answer to a question, from the page inside the messenger.
    ///
    /// # Errors
    ///
    /// As [`Self::app_question`]; an answer the question does not take.
    pub async fn app_answer(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
        question_id: &str,
        answer: Value,
    ) -> Result<Value, WorkbenchShellError> {
        let who = self.app_person(id, init_data).await?;
        self.touch_tunnel();
        if !who.is_owner {
            return Err(WorkbenchShellError::Forbidden(
                "the agent's questions are its owner's to answer".into(),
            ));
        }
        self.answer_in_chat_by(question_id, answer, Some(who.participant.participant_id))
            .await
            .and_then(|answered| {
                serde_json::to_value(answered)
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
            })
    }

    /// Words said in the person's chat from the page inside the messenger:
    /// as theirs, and answered as anything they say is.
    ///
    /// # Errors
    ///
    /// As [`Self::app_person`]; nothing to say.
    pub async fn app_say(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
        text: &str,
    ) -> Result<Value, WorkbenchShellError> {
        let who = self.app_person(id, init_data).await?;
        self.touch_tunnel();
        let said = self
            .say_in_chat_as(
                &who.chat_id,
                Some(who.participant.participant_id),
                &channel_of(id),
                Saying {
                    text: text.to_owned(),
                    blocks: Vec::new(),
                    content_refs: Vec::new(),
                    context: None,
                    client_ref: None,
                    for_the_record: !who.may_speak,
                },
            )
            .await?;
        Ok(json!({ "message_id": said.message.message_id }))
    }

    /// One file the agent put out, for the Mini App to show or save.
    ///
    /// # Errors
    ///
    /// As [`Self::app_person`]; or no such file.
    pub async fn app_file(
        self: &Arc<Self>,
        id: &str,
        init_data: &str,
        name: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let who = self.app_person(id, init_data).await?;
        self.touch_tunnel();
        if !who.is_owner {
            return Err(WorkbenchShellError::Forbidden(
                "what the agent put out is its owner's".into(),
            ));
        }
        let workspace = self
            .inventory
            .select(&who.agent)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?
            .workspace;
        crate::workbench_files::read(&workspace, crate::workbench_files::OUTBOX, name).await
    }

    /// A delivery at a channel's door, handed to the channel as it came.
    ///
    /// # Errors
    ///
    /// No such channel or one not running; or the channel refused it.
    pub async fn receive_at_door(
        self: &Arc<Self>,
        id: &str,
        headers: BTreeMap<String, String>,
        body: String,
    ) -> Result<(), WorkbenchShellError> {
        let channels = self.channels()?;
        let entry = channels
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .map(|run| Arc::clone(&run.entry))
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no channel {id} runs")))?;
        call_tool_of(
            &entry,
            channel::RECEIVE,
            json!({ "headers": headers, "body": body }),
        )
        .await
        .map(|_| ())
        .map_err(WorkbenchShellError::Invalid)
    }

    /// Allow somebody the bot has met to speak to the agent, or take it
    /// back. Allowed, they get a chat of their own with the agent when they
    /// write to the bot alone, and their word in a group the bot is in.
    ///
    /// # Errors
    ///
    /// No such channel; nobody of that id met through it.
    pub async fn allow_guest_at(
        self: &Arc<Self>,
        id: &str,
        guest: &str,
        may_speak: bool,
    ) -> Result<(), WorkbenchShellError> {
        self.channels()?.read(id)?;
        let (channel, guest) = (id.to_owned(), guest.to_owned());
        self.with_ledger(move |ledger| {
            if !ledger
                .identities_of(&guest)?
                .iter()
                .any(|identity| identity.channel_id == channel)
            {
                return Err(crate::RoutingError::InvalidBinding(format!(
                    "nobody {guest} was met through this channel"
                )));
            }
            ledger.let_speak(&channel, &guest, may_speak)
        })
        .await
        .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))
    }

    /// Channels as the page shows them: each one, and what can run one.
    ///
    /// # Errors
    ///
    /// As [`Self::channels_shown`].
    pub async fn channels_standing(
        self: &Arc<Self>,
    ) -> Result<ChannelsStanding, WorkbenchShellError> {
        let channels = self.channels_shown().await?;
        let mut packages: Vec<ChannelPackage> = Vec::new();
        // What the Store installed of the kind, and what came with the
        // product beside the binary.
        if let Ok(installed) = self.installed_root()
            && let Ok(kind) = swem_sdk::Kind::parse(channel::CHANNEL_KIND)
        {
            for (id, receipt) in swem_store::load_receipts(installed, &kind) {
                packages.push(ChannelPackage {
                    id,
                    name: receipt.name,
                    version: receipt.version,
                    bundled: false,
                });
            }
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(dir) = exe.parent()
            && let Ok(entries) = std::fs::read_dir(dir)
        {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                let stem = name.strip_suffix(".exe").unwrap_or(&name).to_owned();
                // A program, not what a build leaves beside one (`.d`, `.pdb`).
                if let Some(package) = stem.strip_prefix("swem-channel-")
                    && !package.is_empty()
                    && !package.contains('.')
                    && package != "fixture"
                    && entry.path().is_file()
                    && !packages.iter().any(|known| known.id == package)
                {
                    packages.push(ChannelPackage {
                        id: package.to_owned(),
                        name: package.to_owned(),
                        version: String::new(),
                        bundled: true,
                    });
                }
            }
        }
        packages.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(ChannelsStanding {
            channels,
            packages,
            app_hosted_at: self.app_hosted_at().map(str::to_owned),
        })
    }

    /// Where the product hosts the page a bot opens inside the messenger.
    pub fn set_app_hosted_at(&self, address: &str) {
        let _ = self
            .app_hosted_at
            .set(address.trim().trim_end_matches('/').to_owned());
    }

    pub(super) fn app_hosted_at(&self) -> Option<&str> {
        self.app_hosted_at
            .get()
            .map(String::as_str)
            .filter(|address| !address.is_empty())
    }

    /// Add a channel: its document, its key, a pairing code; then start it.
    ///
    /// # Errors
    ///
    /// The name is empty, neither a package nor a program is named, the
    /// key cannot be kept, or the channel cannot be started.
    pub async fn add_channel(
        self: &Arc<Self>,
        body: AddChannelBody,
    ) -> Result<ChannelShown, WorkbenchShellError> {
        let channels = self.channels()?;
        let name = body.name.trim();
        if name.is_empty() {
            return Err(WorkbenchShellError::Invalid("a channel has a name".into()));
        }
        if body.package.is_none() && body.program.is_none() {
            return Err(WorkbenchShellError::Invalid(
                "a channel is run by a package from the Store or a program of your own".into(),
            ));
        }
        let id = crate::chat_ledger::new_id("ch")
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if let Some(key) = body.key.as_deref().filter(|key| !key.trim().is_empty()) {
            self.provider_keys
                .get()
                .ok_or_else(|| WorkbenchShellError::Conflict("keys are not enabled here".into()))?
                .give(&key_name_of(&id), channel::KEY_VARIABLE, key)
                .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?;
        }
        let document = ChannelDocument {
            schema: CHANNEL_SCHEMA.into(),
            id: id.clone(),
            name: name.to_owned(),
            package: body.package,
            program: body.program,
            args: body.args,
            agent: body.agent,
            guests: body.guests,
            reach: Reach::Pull,
            app_at: body
                .app_at
                .map(|at| at.trim().trim_end_matches('/').to_owned())
                .filter(|at| !at.is_empty()),
            settings: body.settings,
            pairing_code: Some(pairing_code()?),
            bot: None,
            added_ms: now_ms(),
        };
        channels.write(&document)?;
        let started = self.start_channel(&id).await;
        let mut shown = self.channel_shown(channels.read(&id)?).await?;
        if let Err(error) = started {
            shown.said = error.to_string();
        }
        Ok(shown)
    }

    /// Remove a channel: stop it, forget its key and its document. The chats
    /// it was bound to stay, as chats.
    ///
    /// # Errors
    ///
    /// No such channel.
    pub async fn remove_channel(self: &Arc<Self>, id: &str) -> Result<(), WorkbenchShellError> {
        let channels = self.channels()?;
        channels.read(id)?;
        self.stop_channel(id).await;
        if let Some(keys) = self.provider_keys.get() {
            let _ = keys.take(&key_name_of(id));
        }
        let _ = std::fs::remove_file(channels.document_path(id));
        let _ = std::fs::remove_dir_all(channels.home_of(id));
        Ok(())
    }

    /// Stop a channel's program, if it runs.
    pub async fn stop_channel(self: &Arc<Self>, id: &str) {
        let Ok(channels) = self.channels() else {
            return;
        };
        let taken = channels
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        if let Some(run) = taken {
            run.stop.notify_waiters();
            // The two loops let go of the program once they are told; the
            // program is ended by whoever holds it last, and this is where
            // that is waited for, so a removed bot stops polling at once
            // rather than when its loops happen to end.
            let mut entry = run.entry;
            for _ in 0..50 {
                match Arc::try_unwrap(entry) {
                    Ok(alone) => {
                        let _ = alone.shutdown().await;
                        return;
                    }
                    Err(shared) => entry = shared,
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }

    /// The program a channel runs: the installed package's, or the person's
    /// own.
    fn channel_program(
        &self,
        document: &ChannelDocument,
    ) -> Result<McpServerStdio, WorkbenchShellError> {
        let program = if let Some(program) = &document.program {
            PathBuf::from(program)
        } else if let Some(package) = &document.package {
            let kind = swem_sdk::Kind::parse(channel::CHANNEL_KIND)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            let installed = self.installed_root()?;
            let receipts = swem_store::load_receipts(installed, &kind);
            if let Some(path) = receipts
                .get(package)
                .and_then(|receipt| receipt.launch_path())
            {
                path.to_path_buf()
            } else {
                // A channel that came with the product is a program beside
                // this one.
                std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(Path::to_path_buf))
                    .map(|dir| {
                        dir.join(if cfg!(windows) {
                            format!("swem-channel-{package}.exe")
                        } else {
                            format!("swem-channel-{package}")
                        })
                    })
                    .filter(|path| path.is_file())
                    .ok_or_else(|| {
                        WorkbenchShellError::NotFound(format!(
                            "the channel package {package} is not installed here"
                        ))
                    })?
            }
        } else {
            return Err(WorkbenchShellError::Invalid(
                "a channel is run by a package or a program".into(),
            ));
        };
        let channels = self.channels()?;
        let home = channels.home_of(&document.id);
        std::fs::create_dir_all(&home).map_err(|error| {
            WorkbenchShellError::Failed(format!("make {}: {error}", home.display()))
        })?;
        // The door is this Workbench's, made of where it is served; the
        // document keeps only that a door is wanted.
        let settings = channel::Settings {
            door: (document.reach == Reach::Door)
                .then(|| self.door_of(&document.id))
                .flatten(),
            ..document.settings.clone()
        };
        let mut env = vec![
            EnvVariable::new(channel::HOME_VARIABLE, home.display().to_string()),
            EnvVariable::new(
                channel::SETTINGS_VARIABLE,
                serde_json::to_string(&settings).unwrap_or_else(|_| "{}".into()),
            ),
        ];
        if let Some(key) = self
            .provider_keys
            .get()
            .and_then(|keys| keys.key(&key_name_of(&document.id)).ok().flatten())
        {
            env.push(EnvVariable::new(channel::KEY_VARIABLE, key.value));
        }
        Ok(
            McpServerStdio::new(document.id.clone(), program.display().to_string())
                .args(document.args.clone())
                .env(env),
        )
    }

    /// Start a channel's program and its runner: look at who the bot is,
    /// then pull what arrives and carry what is said.
    ///
    /// # Errors
    ///
    /// No such channel, no program for it, or the program does not answer
    /// as a channel.
    pub async fn start_channel(self: &Arc<Self>, id: &str) -> Result<(), WorkbenchShellError> {
        let channels = self.channels()?;
        let mut document = channels.read(id)?;
        self.stop_channel(id).await;
        let stdio = self.channel_program(&document)?;
        let entry = discover_server(id.to_owned(), &McpServer::Stdio(stdio), None).await;
        let listed: Vec<swem_sdk::ToolListed> = entry
            .tools
            .iter()
            .map(|tool| swem_sdk::ToolListed {
                name: tool.name.clone(),
                input_schema: tool.input_schema.clone(),
            })
            .collect();
        if let Err(short) = channel::shape().check(&listed) {
            let _ = entry.shutdown().await;
            return Err(WorkbenchShellError::Invalid(format!(
                "{} does not answer as a channel: it {short}",
                document.name
            )));
        }
        let entry = Arc::new(entry);
        let bot: Bot = call_tool_of(&entry, channel::LOOK, json!({}))
            .await
            .and_then(|answer| serde_json::from_value(answer).map_err(|error| error.to_string()))
            .map_err(|error| {
                WorkbenchShellError::Failed(format!("{}: look: {error}", document.name))
            })?;
        document.bot = Some(bot);
        channels.write(&document)?;
        let stop = Arc::new(tokio::sync::Notify::new());
        let said = Arc::new(Mutex::new(String::new()));
        channels
            .running
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                id.to_owned(),
                Running {
                    entry: Arc::clone(&entry),
                    stop: Arc::clone(&stop),
                    said: Arc::clone(&said),
                },
            );
        let runner = Runner {
            state: Arc::clone(self),
            id: id.to_owned(),
            entry,
            said,
            origins: Arc::new(Mutex::new(BTreeMap::new())),
        };
        let pulled = runner.clone();
        let stopped = Arc::clone(&stop);
        tokio::spawn(async move {
            tokio::select! {
                () = pulled.pull_forever() => {}
                () = stopped.notified() => {}
            }
        });
        tokio::spawn(async move {
            tokio::select! {
                () = runner.carry_forever() => {}
                () = stop.notified() => {}
            }
        });
        Ok(())
    }

    /// Pair the owner with somebody on a messenger's side by hand, from the
    /// page: for a messenger that has no way to say the code, or to undo a
    /// wrong pairing.
    ///
    /// # Errors
    ///
    /// No such channel, or the ledger cannot be written.
    pub async fn pair_channel_owner(
        self: &Arc<Self>,
        id: &str,
        external_id: &str,
        name: &str,
    ) -> Result<(), WorkbenchShellError> {
        self.channels()?.read(id)?;
        let (id, external_id, name) = (id.to_owned(), external_id.to_owned(), name.to_owned());
        self.with_ledger(move |ledger| {
            let owner = ledger.owner()?;
            ledger.bind_identity(&id, &external_id, &owner.participant_id, &name, None)?;
            Ok(())
        })
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }
}

/// One channel's loops.
#[derive(Clone)]
struct Runner {
    state: Arc<WorkbenchShellState>,
    id: String,
    entry: Arc<AppAttachmentEntry>,
    said: Arc<Mutex<String>>,
    /// Where the last messages that came through this channel were said,
    /// by their reference: so a message is not carried back to the chat it
    /// was said in.
    origins: Arc<Mutex<BTreeMap<String, String>>>,
}

/// The question the bot asks its owner before fetching what a tunnel
/// needs; not one of the agent's.
const INSTALL_FOR_A_TUNNEL: &str = "swem:install-for-a-tunnel";

/// How long a `pull` may wait before answering with nothing.
const PULL_WAIT: u64 = 20;
/// How often the ledger is looked at for what to carry.
const CARRY_EVERY: Duration = Duration::from_millis(200);
/// How many events are read at once.
const AT_ONCE: usize = 256;

/// An agent's turn as it is written, by (chat, agent).
struct Turn {
    /// What it has said so far.
    said: String,
    /// Whether a stream was begun on the messenger's side for it.
    begun: bool,
    /// When it began, so what it put in its outbox meanwhile is known.
    began_ms: i64,
}

type Streams = BTreeMap<(String, String), Turn>;

/// How many origins are remembered.
const ORIGINS_KEPT: usize = 512;

impl Runner {
    fn say(&self, words: impl Into<String>) {
        *self
            .said
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = words.into();
    }

    async fn call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        call_tool_of(&self.entry, tool, arguments).await
    }

    /// Pull what arrives, for as long as the channel runs.
    async fn pull_forever(self) {
        loop {
            let pulled = self
                .call(channel::PULL, json!({ "wait_s": PULL_WAIT }))
                .await;
            let events = match pulled {
                Ok(answer) => serde_json::from_value::<channel::Pulled>(answer)
                    .map(|pulled| pulled.events)
                    .unwrap_or_default(),
                Err(error) => {
                    self.say(format!("pull: {error}"));
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };
            for event in events {
                if let Err(error) = self.arrived(event).await {
                    self.say(error.to_string());
                }
            }
        }
    }

    /// One thing that happened on the messenger's side.
    async fn arrived(&self, event: Inbound) -> Result<(), WorkbenchShellError> {
        // Anything arriving is a reason to look at the clock: a harness
        // woken by a message says what was due meanwhile.
        {
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                let _ = state.look_at_a_knock().await;
            });
        }
        match event {
            Inbound::Message {
                chat,
                person,
                text,
                files,
                reply_to: _,
                addressed,
                reference,
            } => {
                self.message_arrived(chat, person, text, files, addressed, reference)
                    .await
            }
            Inbound::Command {
                chat,
                person,
                name,
                args,
                reference,
            } => {
                if name == "start" {
                    // A hello is for somebody alone with the bot; in a group
                    // it is nobody's business.
                    if chat.kind == ChatKind::Direct
                        && self.participant_of(&person).await?.is_none()
                    {
                        self.greet(&chat.id).await;
                    }
                    Ok(())
                } else if name == "app" || name == "status" {
                    // Only for whoever may speak to the agent; the rest are
                    // met, told once alone, and left alone in a group.
                    let document = self.state.channels()?.read(&self.id)?;
                    let Some((_, is_owner)) =
                        self.speaker_for(&document, &chat, &person, "").await?
                    else {
                        return Ok(());
                    };
                    if name == "app" {
                        self.offer_the_app(&chat.id, is_owner).await;
                    } else {
                        let words = self.status_words(&chat).await;
                        self.tell(&chat.id, &words).await;
                    }
                    Ok(())
                } else {
                    // A command the harness does not know is words.
                    let text = format!("/{name} {args}").trim().to_owned();
                    self.message_arrived(chat, person, text, Vec::new(), true, reference)
                        .await
                }
            }
            Inbound::Answered {
                chat,
                person,
                question,
                option,
                reference: _,
            } if question == INSTALL_FOR_A_TUNNEL => {
                self.install_for_a_tunnel(&chat, &person, &option).await
            }
            Inbound::Answered {
                chat: _,
                person,
                question,
                option,
                reference: _,
            } => {
                let by = self.participant_of(&person).await?;
                self.state
                    .answer_in_chat_by(
                        &question,
                        json!({ "option": option }),
                        by.map(|p| p.participant_id),
                    )
                    .await
                    .map(|_| ())
            }
            Inbound::Stopped { chat, .. } => {
                let bound = self.bound_chat(&chat.id).await?;
                if let Some(chat_id) = bound {
                    self.state.stop_in_chat(&chat_id, None).await.map(|_| ())
                } else {
                    Ok(())
                }
            }
            Inbound::Joined { .. } | Inbound::Left { .. } => Ok(()),
        }
    }

    /// The Workbench's page inside the messenger, offered as a button when
    /// the Workbench has an address; said to need one otherwise.
    /// The Workbench's page inside the messenger, offered as a button: at
    /// the served address, else through a tunnel - opened here if a package
    /// can - else said to need one.
    async fn offer_the_app(&self, external_chat: &str, may_open_a_tunnel: bool) {
        let origin = match self.state.app_origin() {
            Some(origin) => origin,
            // Only the owner opens a door onto their Workbench.
            None if !may_open_a_tunnel => {
                self.tell(
                    external_chat,
                    "The Workbench's page is not reachable right now; its owner can open it.",
                )
                .await;
                return;
            }
            None => match self.state.open_tunnel(None).await {
                Ok(tunnel) => tunnel.origin,
                // The tunnel needs something fetched first: one tap of the
                // owner's is the consent, with what and from where said.
                Err(_) if !self.missing_for_a_tunnel().is_empty() => {
                    let needs = self.missing_for_a_tunnel();
                    let words = needs
                        .iter()
                        .map(|one| format!("{} {} (from {})", one.name, one.version, one.from))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = self
                        .call(
                            channel::ASK,
                            json!({
                                "chat": external_chat,
                                "question": INSTALL_FOR_A_TUNNEL,
                                "title": format!("To open the Workbench here from this computer, a tunnel needs {words}. Install it and open?"),
                                "options": [
                                    { "id": "install", "label": "Install and open" },
                                    { "id": "not-now", "label": "Not now" },
                                ],
                            }),
                        )
                        .await;
                    return;
                }
                Err(error) => {
                    self.tell(
                        external_chat,
                        &format!(
                            "The Workbench's page opens here once the Workbench has an address \
                             from outside: served at one, or through a tunnel from the Store. \
                             Right now: {error}"
                        ),
                    )
                    .await;
                    return;
                }
            },
        };
        // The host the person named for this bot, else the product's own
        // copy, else the Workbench's page itself.
        let url = match self
            .state
            .channels()
            .and_then(|channels| channels.read(&self.id))
        {
            Ok(document) => self.state.app_url_for(&document, &origin),
            Err(_) => format!("{origin}/channels/{}/app", self.id),
        };
        let _ = self
            .call(
                channel::SEND,
                json!({
                    "chat": external_chat,
                    "markdown": "The Workbench, here: a file of any size to the agent, what it put out, the chat.",
                    "app": { "label": "Open", "url": url },
                }),
            )
            .await;
    }

    /// The owner's tap on "Install and open": fetch what the tunnel needs,
    /// open it, offer the page. Anybody else's tap does nothing.
    async fn install_for_a_tunnel(
        &self,
        chat: &channel::ChatRef,
        person: &Person,
        option: &str,
    ) -> Result<(), WorkbenchShellError> {
        let is_owner = self
            .participant_of(person)
            .await?
            .is_some_and(|one| one.kind == ParticipantKind::Person);
        if !is_owner || option != "install" {
            return Ok(());
        }
        self.tell(&chat.id, "Installing, then opening - a moment.")
            .await;
        match self.state.install_and_open_tunnel().await {
            Ok(_) => self.offer_the_app(&chat.id, true).await,
            Err(error) => {
                self.tell(&chat.id, &format!("It could not be done: {error}"))
                    .await;
            }
        }
        Ok(())
    }

    /// What the tunnel that would open needs installed first.
    fn missing_for_a_tunnel(&self) -> Vec<super::tunnel::TunnelNeeds> {
        self.state
            .tunnel_package_chosen(None)
            .map(|package| self.state.tunnel_needs(&package))
            .unwrap_or_default()
    }

    async fn greet(&self, external_chat: &str) {
        self.tell(
            external_chat,
            "Hello. Say the code from your Workbench to be known here.",
        )
        .await;
    }

    async fn participant_of(
        &self,
        person: &Person,
    ) -> Result<Option<crate::Participant>, WorkbenchShellError> {
        let (id, external) = (self.id.clone(), person.id.clone());
        self.state
            .with_ledger(move |ledger| ledger.participant_of_identity(&id, &external))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    async fn bound_chat(&self, external_chat: &str) -> Result<Option<String>, WorkbenchShellError> {
        let (id, external) = (self.id.clone(), external_chat.to_owned());
        self.state
            .with_ledger(move |ledger| ledger.chat_of_channel_chat(&id, &external))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    async fn tell(&self, external_chat: &str, words: &str) {
        let _ = self
            .call(
                channel::SEND,
                json!({ "chat": external_chat, "markdown": words }),
            )
            .await;
    }

    /// The guest somebody is here, met now if never before: their identity
    /// bound, where they write to the bot alone kept, and whether they may
    /// speak set from the channel's default for the new. In a group they
    /// are in the chat from the first word, so the page shows who is in
    /// the room and the agent is told who speaks.
    async fn guest_for(
        &self,
        document: &ChannelDocument,
        person: &Person,
        chat: &channel::ChatRef,
    ) -> Result<(crate::Participant, crate::Identity), WorkbenchShellError> {
        let (id, external, name) = (self.id.clone(), person.id.clone(), person.name.clone());
        let direct = (chat.kind == ChatKind::Direct).then(|| chat.id.clone());
        let may_speak_by_default = document.guests == GuestPolicy::Anyone;
        let (guest, identity) = self
            .state
            .with_ledger(move |ledger| {
                let guest = ledger.guest_of_identity(
                    &id,
                    &external,
                    &name,
                    direct.as_deref(),
                    may_speak_by_default,
                )?;
                let identity = ledger
                    .identity_of(&id, &external)?
                    .ok_or_else(|| crate::RoutingError::InvalidBinding("bound just now".into()))?;
                Ok((guest, identity))
            })
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if chat.kind != ChatKind::Direct
            && let Some(chat_id) = self.bound_chat(&chat.id).await?
        {
            let guest_id = guest.participant_id.clone();
            self.state
                .with_ledger(move |ledger| ledger.join_chat(&chat_id, &guest_id))
                .await
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        }
        Ok((guest, identity))
    }

    /// Who speaks for somebody on the messenger's side, and whether the
    /// agent takes a turn on it: the owner always; a guest when allowed.
    /// A guest who may not speak is left alone in a group and told once,
    /// alone with the bot. `None` means nobody speaks.
    async fn speaker_for(
        &self,
        document: &ChannelDocument,
        chat: &channel::ChatRef,
        person: &Person,
        text: &str,
    ) -> Result<Option<(crate::Participant, bool)>, WorkbenchShellError> {
        let known = self.participant_of(person).await?;
        // The pairing code, said once, binds the owner - also somebody who
        // wrote before saying it and was taken for a guest meanwhile.
        if !known
            .as_ref()
            .is_some_and(|participant| participant.kind == ParticipantKind::Person)
            && let Some(code) = &document.pairing_code
            && text.trim() == code
        {
            let (id, external, name) = (self.id.clone(), person.id.clone(), person.name.clone());
            let direct = (chat.kind == ChatKind::Direct).then(|| chat.id.clone());
            self.state
                .with_ledger(move |ledger| {
                    let owner = ledger.owner()?;
                    ledger.bind_identity(
                        &id,
                        &external,
                        &owner.participant_id,
                        &name,
                        direct.as_deref(),
                    )?;
                    Ok(owner)
                })
                .await
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            self.tell(&chat.id, "You are known here. Say what you want done.")
                .await;
            return Ok(None);
        }
        if let Some(participant) = &known
            && participant.kind == ParticipantKind::Person
        {
            return Ok(Some((participant.clone(), true)));
        }
        let (guest, identity) = self.guest_for(document, person, chat).await?;
        if identity.may_speak {
            return Ok(Some((guest, false)));
        }
        // May not speak: silence in a group; one line, once, alone.
        if chat.kind == ChatKind::Direct && identity.told_ms.is_none() {
            self.tell(
                &chat.id,
                "Hello. The owner of this bot has to allow you to speak to it first.",
            )
            .await;
            let (id, external) = (self.id.clone(), person.id.clone());
            let _ = self
                .state
                .with_ledger(move |ledger| ledger.told(&id, &external))
                .await;
        }
        Ok(None)
    }

    /// Files that came with a message, fetched and kept, as content the
    /// message refers to.
    async fn kept_files(&self, files: Vec<channel::FileRef>) -> Vec<String> {
        let mut content_refs = Vec::new();
        for file in files {
            let fetched = self
                .call(channel::FETCH_FILE, json!({ "file": file.file }))
                .await
                .and_then(|answer| {
                    serde_json::from_value::<channel::Fetched>(answer)
                        .map_err(|error| error.to_string())
                });
            let fetched = match fetched {
                Ok(fetched) => fetched,
                Err(error) => {
                    self.say(format!("a file could not be fetched: {error}"));
                    continue;
                }
            };
            let name = if fetched.name.is_empty() {
                file.name.clone()
            } else {
                fetched.name.clone()
            };
            let bytes = match tokio::fs::read(&fetched.path).await {
                Ok(bytes) => bytes,
                Err(error) => {
                    self.say(format!("a fetched file could not be read: {error}"));
                    continue;
                }
            };
            let media_type = if file.mime.is_empty() {
                "application/octet-stream".to_owned()
            } else {
                file.mime.clone()
            };
            match self
                .state
                .content
                .ingest_bytes(
                    &bytes,
                    name,
                    media_type,
                    crate::WorkbenchContentSource::UserUpload,
                    channel_of(&self.id),
                )
                .await
            {
                Ok(descriptor) => content_refs.push(descriptor.descriptor_id),
                Err(error) => self.say(format!("a file could not be kept: {error}")),
            }
        }
        content_refs
    }

    async fn message_arrived(
        &self,
        chat: channel::ChatRef,
        person: Person,
        text: String,
        files: Vec<channel::FileRef>,
        addressed: bool,
        reference: String,
    ) -> Result<(), WorkbenchShellError> {
        let document = self.state.channels()?.read(&self.id)?;
        // In a group, what is not spoken to the bot is said in the chat for
        // the record - by somebody already in it - and nobody answers; it
        // is no reason to greet, refuse or let anybody in.
        if !addressed && chat.kind != ChatKind::Direct {
            let Some(chat_id) = self.bound_chat(&chat.id).await? else {
                return Ok(());
            };
            let speaker = match self.participant_of(&person).await? {
                Some(participant) if participant.kind == ParticipantKind::Person => participant,
                _ => self.guest_for(&document, &person, &chat).await?.0,
            };
            return self
                .say_through(
                    &chat_id,
                    &chat,
                    speaker.participant_id,
                    text,
                    files,
                    reference,
                    true,
                )
                .await;
        }
        let Some((speaker, is_owner)) = self.speaker_for(&document, &chat, &person, &text).await?
        else {
            // Somebody who may not speak, in a group: heard, for the record.
            if chat.kind != ChatKind::Direct
                && let Some(chat_id) = self.bound_chat(&chat.id).await?
                && let Some(guest) = self.participant_of(&person).await?
                && guest.kind != ParticipantKind::Person
            {
                return self
                    .say_through(
                        &chat_id,
                        &chat,
                        guest.participant_id,
                        text,
                        files,
                        reference,
                        true,
                    )
                    .await;
            }
            return Ok(());
        };
        let chat_id = if let Some(chat_id) = self.bound_chat(&chat.id).await? {
            chat_id
        } else {
            let Some(agent) = document.agent.clone() else {
                self.tell(
                    &chat.id,
                    "No agent answers here yet; choose one on the Workbench.",
                )
                .await;
                return Ok(());
            };
            self.open_chat(&chat, &speaker, is_owner, &agent, &document)
                .await?
        };
        self.say_through(
            &chat_id,
            &chat,
            speaker.participant_id,
            text,
            files,
            reference,
            false,
        )
        .await
    }

    /// What arrived, said in the chat here as the speaker's, with its files
    /// kept and where it came from remembered.
    #[allow(
        clippy::too_many_arguments,
        reason = "one message, all of what came with it"
    )]
    async fn say_through(
        &self,
        chat_id: &str,
        chat: &channel::ChatRef,
        speaker_id: String,
        text: String,
        files: Vec<channel::FileRef>,
        reference: String,
        for_the_record: bool,
    ) -> Result<(), WorkbenchShellError> {
        let content_refs = self.kept_files(files).await;
        {
            let mut origins = self
                .origins
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            origins.insert(reference.clone(), chat.id.clone());
            while origins.len() > ORIGINS_KEPT {
                let first = origins.keys().next().cloned();
                if let Some(first) = first {
                    origins.remove(&first);
                }
            }
        }
        self.state
            .say_in_chat_as(
                chat_id,
                Some(speaker_id),
                &channel_of(&self.id),
                Saying {
                    text,
                    blocks: Vec::new(),
                    content_refs,
                    context: None,
                    client_ref: Some(reference),
                    for_the_record,
                },
            )
            .await
            .map(|_| ())
    }

    /// A chat here for a chat on the messenger's side: the bot's agent and
    /// the speaker, named after the messenger's chat or the bot.
    async fn open_chat(
        &self,
        chat: &channel::ChatRef,
        speaker: &crate::Participant,
        is_owner: bool,
        agent_profile: &str,
        document: &ChannelDocument,
    ) -> Result<String, WorkbenchShellError> {
        let title = if chat.kind == ChatKind::Topic {
            // A forum topic: the group's name and the topic's number, which
            // is what the messenger gives; the person renames it on the page.
            let thread = chat.id.rsplit_once(':').map_or("", |(_, thread)| thread);
            let group = if chat.title.trim().is_empty() {
                document.name.as_str()
            } else {
                chat.title.trim()
            };
            format!("{group} · topic {thread}")
        } else if !chat.title.trim().is_empty() {
            chat.title.trim().to_owned()
        } else if chat.kind == ChatKind::Direct && !is_owner {
            // Somebody else alone with the bot: their chat, by their name.
            speaker.name.clone()
        } else if chat.kind == ChatKind::Direct {
            match &document.bot {
                Some(bot) if !bot.name.is_empty() => bot.name.clone(),
                _ => document.name.clone(),
            }
        } else {
            document.name.clone()
        };
        let (id, external, speaker_id, profile) = (
            self.id.clone(),
            chat.id.clone(),
            speaker.participant_id.clone(),
            agent_profile.to_owned(),
        );
        self.state
            .with_ledger(move |ledger| {
                let owner = ledger.owner()?;
                let agent = ledger.agent_of_profile(&profile)?;
                let mut members = vec![agent.participant_id.clone()];
                if !is_owner {
                    members.push(speaker_id.clone());
                }
                let chat = ledger.start_chat(&title, &owner.participant_id, &members)?;
                ledger.bind_channel_chat(&id, &external, &chat.chat_id)?;
                Ok(chat.chat_id)
            })
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Carry what is said in the chats this channel is bound to, as it is
    /// said: a message from somebody else, an agent's turn as it is written.
    async fn carry_forever(self) {
        let mut place = self
            .state
            .with_ledger(|ledger| ledger.head())
            .await
            .unwrap_or(0);
        // What an agent has said so far in a turn, by (chat, agent).
        let mut streams: Streams = BTreeMap::new();
        loop {
            tokio::time::sleep(CARRY_EVERY).await;
            let from = place;
            let Ok(happened) = self
                .state
                .with_ledger(move |ledger| ledger.happened_with_what_was_said(from, AT_ONCE))
                .await
            else {
                continue;
            };
            for (event, message) in happened {
                place = event.sequence;
                let Some(chat_id) = event.chat_id.clone() else {
                    continue;
                };
                // A chat here may be reached from several places on the
                // messenger's side: the group it is, and the direct chats of
                // guests let into it.
                let Ok(external_chats) = self.external_chats_of(&chat_id).await else {
                    continue;
                };
                if external_chats.is_empty() {
                    continue;
                }
                match message {
                    // A message is placed on an event of its own kind: a
                    // person's on `chat/message`, an agent's on the event that
                    // ended its turn. Whatever the kind, a message is carried.
                    Some(message) => {
                        self.carry_message(&chat_id, &external_chats, &message, &mut streams)
                            .await;
                    }
                    None => {
                        self.carry_event(&chat_id, &external_chats, &event, &mut streams)
                            .await;
                    }
                }
            }
        }
    }

    async fn carry_message(
        &self,
        chat_id: &str,
        external_chats: &[String],
        message: &crate::Message,
        streams: &mut Streams,
    ) {
        if message.channel == crate::CHANNEL_AGENT {
            let key = (chat_id.to_owned(), message.sender_id.clone());
            let turn = streams.remove(&key);
            for external_chat in external_chats {
                let stream = format!("{chat_id}:{}:{external_chat}", message.sender_id);
                if turn.as_ref().is_some_and(|turn| turn.begun) {
                    let _ = self
                        .call(
                            channel::STREAM_END,
                            json!({ "chat": external_chat, "stream": stream, "markdown": message.text }),
                        )
                        .await;
                } else {
                    self.tell(external_chat, &message.text).await;
                }
            }
            // What the agent put in its outbox during the turn goes along.
            if let Some(turn) = turn {
                self.send_what_was_put_out(&message.sender_id, turn.began_ms, external_chats)
                    .await;
            }
            return;
        }
        // Somebody said it elsewhere - the page, the editor, a schedule, or
        // another place on the messenger's side: shown with their name,
        // everywhere but where it was said.
        let markdown = match self.name_of(&message.sender_id).await {
            Some(name) => format!("**{name}:** {}", message.text),
            None => message.text.clone(),
        };
        let said_from = if message.channel == channel_of(&self.id) {
            message.channel_ref.as_ref().and_then(|reference| {
                self.origins
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(reference)
                    .cloned()
            })
        } else {
            None
        };
        for external_chat in external_chats {
            if said_from.as_deref() == Some(external_chat.as_str()) {
                continue;
            }
            self.tell(external_chat, &markdown).await;
        }
    }

    async fn carry_event(
        &self,
        chat_id: &str,
        external_chats: &[String],
        event: &crate::ChatEvent,
        streams: &mut Streams,
    ) {
        match event.kind.as_str() {
            "acp/session_update" => {
                let Some(agent_id) = event.agent_id.clone() else {
                    return;
                };
                let update = event.payload.get("update");
                if update
                    .and_then(|u| u.get("sessionUpdate"))
                    .and_then(Value::as_str)
                    != Some("agent_message_chunk")
                {
                    return;
                }
                let Some(text) = update
                    .and_then(|u| u.get("content"))
                    .and_then(|c| c.get("text"))
                    .and_then(Value::as_str)
                else {
                    return;
                };
                let key = (chat_id.to_owned(), agent_id.clone());
                let turn = streams.entry(key).or_insert_with(|| Turn {
                    said: String::new(),
                    begun: false,
                    began_ms: event
                        .at_ms
                        .and_then(|at| i64::try_from(at).ok())
                        .unwrap_or_else(now_ms),
                });
                let begun = std::mem::replace(&mut turn.begun, true);
                turn.said.push_str(text);
                let said = turn.said.clone();
                for external_chat in external_chats {
                    let stream = format!("{chat_id}:{agent_id}:{external_chat}");
                    if !begun {
                        let _ = self
                            .call(
                                channel::STREAM_BEGIN,
                                json!({ "chat": external_chat, "stream": stream }),
                            )
                            .await;
                    }
                    let _ = self
                        .call(
                            channel::STREAM_UPDATE,
                            json!({ "chat": external_chat, "stream": stream, "markdown": said }),
                        )
                        .await;
                }
            }
            "chat/delivery" => {
                let state = event.payload.get("state").and_then(Value::as_str);
                let Some(agent_id) = event.payload.get("agent_id").and_then(Value::as_str) else {
                    return;
                };
                // A turn begins: from now on what the agent puts in its
                // outbox is the turn's, and the messenger's side sees it
                // begin - typing, and a mark on the message being answered
                // when it came from there.
                if state == Some("running") {
                    self.turn_begins(chat_id, agent_id, &event.payload, external_chats)
                        .await;
                    streams.insert(
                        (chat_id.to_owned(), agent_id.to_owned()),
                        Turn {
                            said: String::new(),
                            begun: true,
                            began_ms: event
                                .at_ms
                                .and_then(|at| i64::try_from(at).ok())
                                .unwrap_or_else(now_ms),
                        },
                    );
                    return;
                }
                // A turn that ended without words - stopped, failed - closes
                // its stream with what there was.
                if matches!(state, Some("stopped" | "failed" | "interrupted"))
                    && let Some(turn) = streams.remove(&(chat_id.to_owned(), agent_id.to_owned()))
                    && turn.begun
                {
                    for external_chat in external_chats {
                        let stream = format!("{chat_id}:{agent_id}:{external_chat}");
                        let _ = self
                            .call(
                                channel::STREAM_END,
                                json!({ "chat": external_chat, "stream": stream, "markdown": turn.said, "stopped": true }),
                            )
                            .await;
                    }
                }
            }
            "chat/question" => self.carry_question(&event.payload, external_chats).await,
            _ => {}
        }
    }

    /// A form or a link the agent asks for: in words, with a button that
    /// opens the page inside the messenger at the question, where its
    /// fields are drawn - with a way to decline it, or to say something
    /// else instead.
    async fn carry_a_form(&self, question_id: &str, external_chats: &[String]) {
        let message = self
            .state
            .question_in_full(question_id)
            .await
            .ok()
            .and_then(|asked| {
                asked
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "The agent asks for something.".to_owned());
        let origin = match self.state.app_origin() {
            Some(origin) => Some(origin),
            None => self
                .state
                .open_tunnel(None)
                .await
                .ok()
                .map(|tunnel| tunnel.origin),
        };
        let document = self.state.channels().and_then(|c| c.read(&self.id)).ok();
        for external_chat in external_chats {
            if let (Some(origin), Some(document)) = (&origin, &document) {
                let url = self.state.app_url_for(document, origin);
                let joiner = if url.contains('?') { '&' } else { '?' };
                let _ = self
                    .call(
                        channel::SEND,
                        json!({
                            "chat": external_chat,
                            "markdown": format!("**The agent asks:** {message}"),
                            "app": { "label": "Answer", "url": format!("{url}{joiner}question={question_id}") },
                        }),
                    )
                    .await;
            } else {
                self.tell(
                    external_chat,
                    &format!(
                        "**The agent asks:** {message}\n\nAnswer it on the Workbench's page, or send /app once it has an address."
                    ),
                )
                .await;
            }
        }
    }

    /// A question the agent asks, as buttons where the messenger has them:
    /// a permission's options. What only the page can answer - a form, a
    /// link - is said to be so.
    async fn carry_question(&self, payload: &Value, external_chats: &[String]) {
        if payload.get("state").and_then(Value::as_str) != Some("waiting") {
            return;
        }
        let Some(question_id) = payload.get("question_id").and_then(Value::as_str) else {
            return;
        };
        let asked = payload.get("asked").cloned().unwrap_or(Value::Null);
        let kind = payload.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind != "permission" {
            self.carry_a_form(question_id, external_chats).await;
            return;
        }
        let options: Vec<Value> = asked
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| {
                        let id = option.get("optionId").and_then(Value::as_str)?;
                        let label = option.get("name").and_then(Value::as_str).unwrap_or(id);
                        Some(json!({ "id": id, "label": label }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if options.is_empty() {
            return;
        }
        let title = asked
            .get("title")
            .and_then(Value::as_str)
            .filter(|title| !title.trim().is_empty())
            .map_or_else(
                || "The agent asks: may it?".to_owned(),
                |title| format!("The agent asks: may it?\n\n`{title}`"),
            );
        for external_chat in external_chats {
            let _ = self
                .call(
                    channel::ASK,
                    json!({
                        "chat": external_chat,
                        "question": question_id,
                        "title": title,
                        "options": options,
                    }),
                )
                .await;
        }
    }

    /// What the agent put in its outbox since its turn began, sent along.
    /// A file the messenger cannot take is said to be too large.
    async fn send_what_was_put_out(
        &self,
        agent_id: &str,
        since_ms: i64,
        external_chats: &[String],
    ) {
        let Some(workspace) = self.workspace_of_agent(agent_id).await else {
            return;
        };
        let Ok(files) = crate::workbench_files::list(&workspace).await else {
            return;
        };
        for file in files {
            if file.area != crate::workbench_files::OUTBOX {
                continue;
            }
            let path = workspace
                .join(crate::workbench_files::OUTBOX)
                .join(&file.name);
            let changed_ms = tokio::fs::metadata(&path)
                .await
                .ok()
                .and_then(|meta| meta.modified().ok())
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |since| {
                    i64::try_from(since.as_millis()).unwrap_or(i64::MAX)
                });
            // A second's grace: a file system keeps seconds, a ledger
            // keeps milliseconds.
            if changed_ms + 1_000 < since_ms {
                continue;
            }
            for external_chat in external_chats {
                if let Err(error) = self
                    .call(
                        channel::SEND_FILE,
                        json!({ "chat": external_chat, "path": path.display().to_string(), "name": file.name }),
                    )
                    .await
                {
                    self.tell(
                        external_chat,
                        &format!("_{} could not be sent here: {error}_", file.name),
                    )
                    .await;
                    if error.contains("too_large") {
                        self.offer_the_app(external_chat, true).await;
                    }
                }
            }
        }
    }

    /// A turn begins on the messenger's side: typing, and a mark on the
    /// message being answered when it came from there.
    async fn turn_begins(
        &self,
        chat_id: &str,
        agent_id: &str,
        payload: &Value,
        external_chats: &[String],
    ) {
        let answering = match payload.get("message_id").and_then(Value::as_str) {
            Some(message_id) => self.reference_of(message_id).await,
            None => None,
        };
        for external_chat in external_chats {
            let stream = format!("{chat_id}:{agent_id}:{external_chat}");
            let _ = self
                .call(
                    channel::STREAM_BEGIN,
                    json!({ "chat": external_chat, "stream": stream, "reply_to": answering }),
                )
                .await;
        }
    }

    /// The messenger's reference of a message, when it came through this
    /// channel.
    async fn reference_of(&self, message_id: &str) -> Option<String> {
        let id = message_id.to_owned();
        let message = self
            .state
            .with_ledger(move |ledger| ledger.message(&id))
            .await
            .ok()?;
        (message.channel == channel_of(&self.id))
            .then_some(message.channel_ref)
            .flatten()
    }

    /// `/status`: how things stand here, in a few lines.
    async fn status_words(&self, chat: &channel::ChatRef) -> String {
        let document = match self
            .state
            .channels()
            .and_then(|channels| channels.read(&self.id))
        {
            Ok(document) => document,
            Err(error) => return format!("Nothing is known: {error}"),
        };
        let agent = document
            .agent
            .clone()
            .unwrap_or_else(|| "nobody".to_owned());
        let reach = match document.bot.as_ref().map(|bot| bot.reach.as_str()) {
            Some("door") => "delivered to the Workbench's door".to_owned(),
            Some(other) if !other.is_empty() && other != "pull" => other.to_owned(),
            _ => "asked from the messenger".to_owned(),
        };
        let Ok(Some(chat_id)) = self.bound_chat(&chat.id).await else {
            return format!(
                "**{agent}** answers here. No chat yet: say something.\nUpdates are {reach}."
            );
        };
        let Ok(page) = self.state.chat_page(&chat_id, None, 20).await else {
            return format!("**{agent}** answers here.");
        };
        let working = page
            .deliveries
            .iter()
            .filter(|delivery| delivery.state == crate::DeliveryState::Running)
            .count();
        let waiting = page
            .deliveries
            .iter()
            .filter(|delivery| delivery.state == crate::DeliveryState::Queued)
            .count();
        let asking = page
            .questions
            .iter()
            .filter(|question| question.state == crate::QuestionState::Waiting)
            .count();
        let doing = if asking > 0 {
            "waiting for an answer to its question".to_owned()
        } else if working > 0 {
            "working now".to_owned()
        } else if waiting > 0 {
            format!("{waiting} message(s) in line")
        } else {
            "idle".to_owned()
        };
        let members = page
            .chat
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "**{agent}**: {doing}.\nChat **{}** with {members}; {} messages.\nUpdates are {reach}.",
            page.chat.title,
            page.messages.len()
        )
    }

    async fn workspace_of_agent(&self, agent_id: &str) -> Option<PathBuf> {
        let id = agent_id.to_owned();
        let profile = self
            .state
            .with_ledger(move |ledger| ledger.participant(&id))
            .await
            .ok()?
            .profile_id?;
        self.state
            .inventory
            .select(&profile)
            .ok()
            .map(|profile| profile.workspace)
    }

    async fn external_chats_of(&self, chat_id: &str) -> Result<Vec<String>, WorkbenchShellError> {
        let (id, chat) = (self.id.clone(), chat_id.to_owned());
        self.state
            .with_ledger(move |ledger| {
                Ok(ledger
                    .channel_chats_of(&chat)?
                    .into_iter()
                    .filter(|bound| bound.channel_id == id)
                    .map(|bound| bound.external_chat)
                    .collect())
            })
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    async fn name_of(&self, participant_id: &str) -> Option<String> {
        let id = participant_id.to_owned();
        self.state
            .with_ledger(move |ledger| ledger.participant(&id))
            .await
            .ok()
            .map(|participant| participant.name)
    }
}
