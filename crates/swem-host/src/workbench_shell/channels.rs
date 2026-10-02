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

fn channel_schema() -> String {
    CHANNEL_SCHEMA.to_owned()
}

/// A channel as the page shows it: the document, and how it is doing.
#[derive(Clone, Debug, Serialize)]
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
        let paired = self
            .with_ledger(move |ledger| {
                let owner = ledger.owner()?;
                Ok(ledger
                    .identities_of(&owner.participant_id)?
                    .iter()
                    .any(|identity| identity.channel_id == id))
            })
            .await
            .unwrap_or(false);
        Ok(ChannelShown {
            document,
            running,
            keyed,
            paired,
            said,
        })
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
            if let Some(entry) = Arc::into_inner(run.entry) {
                let _ = entry.shutdown().await;
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
        let mut env = vec![
            EnvVariable::new(channel::HOME_VARIABLE, home.display().to_string()),
            EnvVariable::new(
                channel::SETTINGS_VARIABLE,
                serde_json::to_string(&document.settings).unwrap_or_else(|_| "{}".into()),
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
            ledger.bind_identity(&id, &external_id, &owner.participant_id, &name)?;
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
}

/// How long a `pull` may wait before answering with nothing.
const PULL_WAIT: u64 = 20;
/// How often the ledger is looked at for what to carry.
const CARRY_EVERY: Duration = Duration::from_millis(200);
/// How many events are read at once.
const AT_ONCE: usize = 256;

/// What an agent has said so far in a turn, by (chat, agent).
type Streams = BTreeMap<(String, String), String>;

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
                reference,
            } => {
                self.message_arrived(chat, person, text, files, reference)
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
                    self.greet(&chat.id).await;
                    Ok(())
                } else {
                    // A command the harness does not know is words.
                    let text = format!("/{name} {args}").trim().to_owned();
                    self.message_arrived(chat, person, text, Vec::new(), reference)
                        .await
                }
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

    async fn guest_for(&self, person: &Person) -> Result<crate::Participant, WorkbenchShellError> {
        let (id, external, name) = (self.id.clone(), person.id.clone(), person.name.clone());
        self.state
            .with_ledger(move |ledger| ledger.guest_of_identity(&id, &external, &name))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Who speaks for somebody on the messenger's side, by the channel's
    /// policy: the participant they are bound to, a guest, or nobody - in
    /// which case they were told why.
    async fn speaker_for(
        &self,
        document: &ChannelDocument,
        chat: &channel::ChatRef,
        person: &Person,
        text: &str,
    ) -> Result<Option<(crate::Participant, bool)>, WorkbenchShellError> {
        let known = self.participant_of(person).await?;
        // The pairing code, said once, binds the owner.
        if known.is_none()
            && let Some(code) = &document.pairing_code
            && text.trim() == code
        {
            let (id, external, name) = (self.id.clone(), person.id.clone(), person.name.clone());
            self.state
                .with_ledger(move |ledger| {
                    let owner = ledger.owner()?;
                    ledger.bind_identity(&id, &external, &owner.participant_id, &name)?;
                    Ok(owner)
                })
                .await
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            self.tell(&chat.id, "You are known here. Say what you want done.")
                .await;
            return Ok(None);
        }
        if let Some(participant) = known {
            let is_owner = participant.kind == ParticipantKind::Person;
            return Ok(Some((participant, is_owner)));
        }
        match document.guests {
            GuestPolicy::Nobody => {
                self.tell(&chat.id, "This bot answers its owner only.")
                    .await;
                Ok(None)
            }
            GuestPolicy::ByInvitation => {
                // Known only once the owner joins them to a chat; until then
                // a guest with nothing to say into.
                let guest = self.guest_for(person).await?;
                if self.bound_chat(&chat.id).await?.is_none() {
                    self.tell(&chat.id, "Somebody has to invite you to a chat first.")
                        .await;
                    return Ok(None);
                }
                Ok(Some((guest, false)))
            }
            GuestPolicy::Anyone => Ok(Some((self.guest_for(person).await?, false))),
        }
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
        reference: String,
    ) -> Result<(), WorkbenchShellError> {
        let document = self.state.channels()?.read(&self.id)?;
        let Some((speaker, is_owner)) = self.speaker_for(&document, &chat, &person, &text).await?
        else {
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
        let content_refs = self.kept_files(files).await;
        self.state
            .say_in_chat_as(
                &chat_id,
                Some(speaker.participant_id),
                &channel_of(&self.id),
                Saying {
                    text,
                    blocks: Vec::new(),
                    content_refs,
                    context: None,
                    client_ref: Some(reference),
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
        let title = if !chat.title.trim().is_empty() {
            chat.title.trim().to_owned()
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
                let Ok(Some(external_chat)) = self.external_chat_of(&chat_id).await else {
                    continue;
                };
                match message {
                    // A message is placed on an event of its own kind: a
                    // person's on `chat/message`, an agent's on the event that
                    // ended its turn. Whatever the kind, a message is carried.
                    Some(message) => {
                        self.carry_message(&chat_id, &external_chat, &message, &mut streams)
                            .await;
                    }
                    None => {
                        self.carry_event(&chat_id, &external_chat, &event, &mut streams)
                            .await;
                    }
                }
            }
        }
    }

    async fn carry_message(
        &self,
        chat_id: &str,
        external_chat: &str,
        message: &crate::Message,
        streams: &mut Streams,
    ) {
        if message.channel == channel_of(&self.id) {
            // It came from this very messenger.
            return;
        }
        if message.channel == crate::CHANNEL_AGENT {
            let key = (chat_id.to_owned(), message.sender_id.clone());
            let stream = format!("{chat_id}:{}", message.sender_id);
            if streams.remove(&key).is_some() {
                let _ = self
                    .call(
                        channel::STREAM_END,
                        json!({ "chat": external_chat, "stream": stream, "markdown": message.text }),
                    )
                    .await;
            } else {
                self.tell(external_chat, &message.text).await;
            }
            return;
        }
        // Somebody said it elsewhere - the page, the editor, a schedule:
        // shown with their name.
        let markdown = match self.name_of(&message.sender_id).await {
            Some(name) => format!("**{name}:** {}", message.text),
            None => message.text.clone(),
        };
        self.tell(external_chat, &markdown).await;
    }

    async fn carry_event(
        &self,
        chat_id: &str,
        external_chat: &str,
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
                let stream = format!("{chat_id}:{agent_id}");
                let begun = streams.contains_key(&key);
                let said = streams.entry(key).or_default();
                said.push_str(text);
                let said = said.clone();
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
            "chat/delivery" => {
                // A turn that ended without words - stopped, failed - closes
                // its stream with what there was.
                let state = event.payload.get("state").and_then(Value::as_str);
                if matches!(state, Some("stopped" | "failed" | "interrupted"))
                    && let Some(agent_id) = event.payload.get("agent_id").and_then(Value::as_str)
                    && let Some(said) = streams.remove(&(chat_id.to_owned(), agent_id.to_owned()))
                {
                    let stream = format!("{chat_id}:{agent_id}");
                    let _ = self
                        .call(
                            channel::STREAM_END,
                            json!({ "chat": external_chat, "stream": stream, "markdown": said, "stopped": true }),
                        )
                        .await;
                }
            }
            "chat/question" => {
                let Some(question_id) = event.payload.get("question_id").and_then(Value::as_str)
                else {
                    return;
                };
                let Some(options) = event.payload.get("options") else {
                    return;
                };
                let title = event
                    .payload
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("The agent asks")
                    .to_owned();
                let _ = self
                    .call(
                        channel::ASK,
                        json!({ "chat": external_chat, "question": question_id, "title": title, "options": options }),
                    )
                    .await;
            }
            _ => {}
        }
    }

    async fn external_chat_of(&self, chat_id: &str) -> Result<Option<String>, WorkbenchShellError> {
        let (id, chat) = (self.id.clone(), chat_id.to_owned());
        self.state
            .with_ledger(move |ledger| {
                Ok(ledger
                    .channel_chats_of(&chat)?
                    .into_iter()
                    .find(|bound| bound.channel_id == id)
                    .map(|bound| bound.external_chat))
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
