//! A Telegram bot as a channel of the SWEM Workbench.
//!
//! The harness starts this program with the bot's token in `SWEM_CHANNEL_KEY`,
//! a directory in `SWEM_CHANNEL_HOME` and settings in `SWEM_CHANNEL_SETTINGS`
//! (`api_root` for a local Bot API server or a fixture), and talks to it as
//! an MCP server of the channel shape (`swem_sdk::channel`). This program
//! polls Telegram for updates and hands them over through `pull`; what the
//! harness sends goes out as the Bot API takes it: an agent's turn as a
//! message draft while it is written (`sendMessageDraft`, Bot API 9.5) and a
//! message when it is done, Markdown as the HTML Telegram renders, a
//! question as buttons, a file as a document.

mod api;
mod text;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use swem_sdk::channel::{self, Bot, ChatKind, ChatRef, FileRef, Inbound, Person};
use tokio::sync::Mutex;

use api::Api;

/// How often a draft is updated at most: Telegram rate-limits edits, and a
/// person reads nothing faster.
const DRAFT_EVERY: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Nothing {}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct PullParams {
    wait_s: u64,
}

/// A delivery the harness's door took in, as it came.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct ReceiveParams {
    /// The request's headers, names in lower case.
    #[serde(default)]
    headers: BTreeMap<String, String>,
    /// The request's body, as text.
    #[serde(default)]
    body: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct SendParams {
    chat: String,
    markdown: String,
    #[serde(default)]
    reply_to: Option<String>,
    /// A button under the message that opens a page of the Workbench's
    /// inside the messenger (a Mini App), with the person's signed identity.
    #[serde(default)]
    app: Option<AppButton>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct AppButton {
    label: String,
    url: String,
}

/// Where the Workbench's page inside the messenger is, to be one tap away.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct SetAppParams {
    url: String,
}

/// What a Mini App sends to be known: the messenger's signed `initData`.
#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct VerifyAppParams {
    init_data: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct StreamBeginParams {
    chat: String,
    stream: String,
    #[serde(default)]
    reply_to: Option<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct StreamParams {
    chat: String,
    stream: String,
    markdown: String,
    #[serde(default)]
    stopped: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct AskParams {
    chat: String,
    question: String,
    title: String,
    options: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct SendFileParams {
    chat: String,
    path: String,
    name: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct FetchFileParams {
    file: String,
}

/// A turn being written on Telegram's side.
struct Stream {
    chat: String,
    /// A draft Telegram shows while the turn is written (Bot API 9.5), or
    /// a message of our own that is edited where drafts are not had.
    how: StreamHow,
    last_sent: Option<Instant>,
    last_text: String,
    /// The message being answered, which wears a reaction while it is.
    answering: Option<i64>,
}

enum StreamHow {
    Draft { draft_id: u32 },
    Edits { message_id: Option<i64> },
    Undecided,
}

struct Channel {
    api: Api,
    home: PathBuf,
    bot: Mutex<Option<Bot>>,
    /// What arrived and was not pulled yet.
    arrived: Arc<Mutex<VecDeque<Inbound>>>,
    streams: Arc<Mutex<BTreeMap<String, Stream>>>,
    /// A question's options by the short token a button carries.
    buttons: Mutex<BTreeMap<String, (String, String)>>,
    /// The messenger's message id behind each update reference pulled, so
    /// a turn can be seen to answer it (a reaction, a reply).
    messages: Mutex<BTreeMap<String, i64>>,
    drafts: Mutex<u32>,
    /// How what the messenger has gets here: pulled, or delivered to the
    /// door the harness gave; and the secret a delivery must carry.
    reach: Mutex<String>,
    door_secret: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl Channel {
    async fn bot_username(&self) -> String {
        self.bot
            .lock()
            .await
            .as_ref()
            .map(|bot| bot.username.clone())
            .unwrap_or_default()
    }

    /// Everything the harness needs to know about one update.
    async fn inbound_of(&self, update: &Value) -> Option<Inbound> {
        let reference = update.get("update_id")?.to_string();
        if let Some(message) = update.get("message").or_else(|| update.get("channel_post")) {
            return self.inbound_of_message(message, &reference).await;
        }
        if let Some(query) = update.get("callback_query") {
            let data = query
                .get("data")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let chat = query
                .get("message")
                .and_then(|m| m.get("chat"))
                .map(chat_of)
                .unwrap_or_default();
            let person = person_of(query.get("from"));
            let _ = self
                .api
                .call(
                    "answerCallbackQuery",
                    json!({ "callback_query_id": query.get("id") }),
                )
                .await;
            if let Some(token) = data.strip_prefix("a:")
                && let Some((question, option)) = self.buttons.lock().await.remove(token)
            {
                return Some(Inbound::Answered {
                    chat,
                    person,
                    question,
                    option,
                    reference,
                });
            }
            return None;
        }
        None
    }

    async fn inbound_of_message(&self, message: &Value, reference: &str) -> Option<Inbound> {
        let chat = chat_of(message.get("chat")?);
        let thread = message
            .get("is_topic_message")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then(|| message.get("message_thread_id").and_then(Value::as_i64))
            .flatten();
        let chat = ChatRef {
            id: api::chat_ref(message["chat"]["id"].as_i64().unwrap_or_default(), thread),
            kind: if thread.is_some() {
                ChatKind::Topic
            } else {
                chat.kind
            },
            title: chat.title,
        };
        let person = person_of(message.get("from"));
        if let Some(joined) = message.get("new_chat_members").and_then(Value::as_array) {
            let who = joined.first().map_or(person, |m| person_of(Some(m)));
            return Some(Inbound::Joined {
                chat,
                person: who,
                reference: reference.to_owned(),
            });
        }
        if let Some(left) = message.get("left_chat_member") {
            return Some(Inbound::Left {
                chat,
                person: person_of(Some(left)),
                reference: reference.to_owned(),
            });
        }
        if let Some(message_id) = message.get("message_id").and_then(Value::as_i64) {
            let mut messages = self.messages.lock().await;
            messages.insert(reference.to_owned(), message_id);
            while messages.len() > 512 {
                let first = messages.keys().next().cloned();
                if let Some(first) = first {
                    messages.remove(&first);
                }
            }
        }
        let text = message
            .get("text")
            .or_else(|| message.get("caption"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let reply_to = message
            .get("reply_to_message")
            .and_then(|r| r.get("message_id"))
            .map(Value::to_string);
        let files = files_of(message);
        // A command: `/start`, `/stop`, `/anything args` - with the bot's
        // name after it in a group.
        if let Some(rest) = text.strip_prefix('/')
            && files.is_empty()
        {
            let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
            let name = name
                .split_once('@')
                .map_or(name, |(name, _)| name)
                .to_owned();
            if name == "stop" {
                let mut streams = self.streams.lock().await;
                if let Some((stream_id, _)) = streams.iter().find(|(_, s)| s.chat == chat.id) {
                    let stream_id = stream_id.clone();
                    streams.remove(&stream_id);
                    return Some(Inbound::Stopped {
                        chat,
                        stream: stream_id,
                        reference: reference.to_owned(),
                    });
                }
            }
            return Some(Inbound::Command {
                chat,
                person,
                name,
                args: args.trim().to_owned(),
                reference: reference.to_owned(),
            });
        }
        // In a group the bot is spoken to by name or by a reply to it; the
        // name is not words. What is not spoken to the bot is still said in
        // the chat for the record, marked as not addressed.
        let username = self.bot_username().await;
        let (addressed, text) = spoken_to(&username, message, &chat, text);
        if text.is_empty() && files.is_empty() {
            return None;
        }
        Some(Inbound::Message {
            chat,
            person,
            text,
            files,
            reply_to,
            addressed,
            reference: reference.to_owned(),
        })
    }

    /// Poll Telegram for updates, for as long as the program runs.
    async fn poll_forever(self: Arc<Self>) {
        let mut offset: Option<i64> = None;
        loop {
            let mut body = json!({ "timeout": 25, "allowed_updates": ["message", "channel_post", "callback_query"] });
            if let Some(offset) = offset {
                body["offset"] = json!(offset);
            }
            match self.api.call("getUpdates", body).await {
                Ok(Value::Array(updates)) => {
                    for update in updates {
                        if let Some(id) = update.get("update_id").and_then(Value::as_i64) {
                            offset = Some(id + 1);
                        }
                        if let Some(inbound) = self.inbound_of(&update).await {
                            self.arrived.lock().await.push_back(inbound);
                        }
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    eprintln!("swem-channel-telegram: {error}");
                    tokio::time::sleep(Duration::from_secs(3)).await;
                }
            }
        }
    }

    async fn send_html(
        &self,
        chat: &str,
        html: &str,
        reply_to: Option<&str>,
        reply_markup: Option<Value>,
    ) -> Result<Value, String> {
        let (chat_id, thread) = api::chat_parts(chat);
        let mut last = Value::Null;
        let pieces = text::chunks(html, api::TEXT_LIMIT);
        let count = pieces.len();
        for (index, piece) in pieces.into_iter().enumerate() {
            let mut body = json!({ "chat_id": chat_id, "text": piece, "parse_mode": "HTML" });
            if let Some(thread) = thread {
                body["message_thread_id"] = json!(thread);
            }
            if index == 0
                && let Some(reply_to) = reply_to.and_then(|r| r.parse::<i64>().ok())
            {
                body["reply_parameters"] = json!({ "message_id": reply_to });
            }
            if index + 1 == count
                && let Some(markup) = &reply_markup
            {
                body["reply_markup"] = markup.clone();
            }
            last = match self.api.call("sendMessage", body.clone()).await {
                Ok(sent) => sent,
                Err(error) if error.contains("can't parse") => {
                    // HTML Telegram refuses is sent as text, so nothing is
                    // lost over a tag.
                    body["text"] = json!(plain(&piece));
                    body.as_object_mut().map(|b| b.remove("parse_mode"));
                    self.api.call("sendMessage", body).await?
                }
                Err(error) => return Err(error),
            };
        }
        Ok(last)
    }

    async fn draft(&self, stream: &mut Stream, markdown: &str, final_: bool) -> Result<(), String> {
        let html = text::html(markdown);
        let (chat_id, thread) = api::chat_parts(&stream.chat);
        if matches!(stream.how, StreamHow::Undecided) {
            let mut drafts = self.drafts.lock().await;
            *drafts += 1;
            let draft_id = *drafts;
            let mut body = json!({ "chat_id": chat_id, "draft_id": draft_id, "text": html, "parse_mode": "HTML" });
            if let Some(thread) = thread {
                body["message_thread_id"] = json!(thread);
            }
            match self.api.call("sendMessageDraft", body).await {
                Ok(_) => {
                    stream.how = StreamHow::Draft { draft_id };
                    stream.last_sent = Some(Instant::now());
                    stream.last_text = html;
                    return Ok(());
                }
                Err(_) => stream.how = StreamHow::Edits { message_id: None },
            }
        }
        if final_ {
            return Ok(());
        }
        match &mut stream.how {
            StreamHow::Draft { draft_id } => {
                let mut body = json!({ "chat_id": chat_id, "draft_id": *draft_id, "text": html, "parse_mode": "HTML" });
                if let Some(thread) = thread {
                    body["message_thread_id"] = json!(thread);
                }
                self.api.call("sendMessageDraft", body).await?;
            }
            StreamHow::Edits { message_id } => {
                let shown: String = text::chunks(&html, api::TEXT_LIMIT)
                    .into_iter()
                    .next()
                    .unwrap_or_default();
                match message_id {
                    None => {
                        let sent = self.send_html(&stream.chat, &shown, None, None).await?;
                        *message_id = sent.get("message_id").and_then(Value::as_i64);
                    }
                    Some(id) => {
                        let _ = self
                            .api
                            .call(
                                "editMessageText",
                                json!({ "chat_id": chat_id, "message_id": *id, "text": shown, "parse_mode": "HTML" }),
                            )
                            .await;
                    }
                }
            }
            StreamHow::Undecided => {}
        }
        stream.last_sent = Some(Instant::now());
        stream.last_text = html;
        Ok(())
    }
}

fn chat_of(chat: &Value) -> ChatRef {
    let kind = match chat.get("type").and_then(Value::as_str) {
        Some("private") => ChatKind::Direct,
        _ => ChatKind::Group,
    };
    let title = chat
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_default();
    ChatRef {
        id: chat.get("id").map(Value::to_string).unwrap_or_default(),
        kind,
        title,
    }
}

/// Whether the bot was spoken to, and the words without its name: always in
/// a direct chat; in a group, by naming it or replying to it.
fn spoken_to(username: &str, message: &Value, chat: &ChatRef, text: String) -> (bool, String) {
    let named = !username.is_empty()
        && text
            .to_ascii_lowercase()
            .contains(&format!("@{}", username.to_ascii_lowercase()));
    let replied_to_the_bot = message
        .get("reply_to_message")
        .and_then(|r| r.get("from"))
        .is_some_and(|from| {
            from.get("is_bot").and_then(Value::as_bool) == Some(true)
                && from
                    .get("username")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.eq_ignore_ascii_case(username))
        });
    let addressed = chat.kind == ChatKind::Direct || named || replied_to_the_bot;
    let text = if named {
        let mention = format!("@{}", username.to_ascii_lowercase());
        let mut cleaned = String::with_capacity(text.len());
        let mut rest = text.as_str();
        while let Some(at) = rest.to_ascii_lowercase().find(&mention) {
            cleaned.push_str(&rest[..at]);
            rest = &rest[at + mention.len()..];
        }
        cleaned.push_str(rest);
        cleaned.trim().to_owned()
    } else {
        text
    };
    (addressed, text)
}

fn person_of(from: Option<&Value>) -> Person {
    let Some(from) = from else {
        return Person::default();
    };
    let first = from
        .get("first_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let last = from
        .get("last_name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Person {
        id: from.get("id").map(Value::to_string).unwrap_or_default(),
        name: format!("{first} {last}").trim().to_owned(),
        username: from
            .get("username")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}

fn files_of(message: &Value) -> Vec<FileRef> {
    let mut files = Vec::new();
    for key in ["document", "audio", "video", "voice", "animation"] {
        if let Some(file) = message.get(key) {
            files.push(FileRef {
                file: file
                    .get("file_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                name: file
                    .get("file_name")
                    .and_then(Value::as_str)
                    .map_or_else(|| format!("{key}.bin"), str::to_owned),
                size: file
                    .get("file_size")
                    .and_then(Value::as_u64)
                    .unwrap_or_default(),
                mime: file
                    .get("mime_type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
    }
    // A photo comes in sizes; the last is the largest.
    if let Some(sizes) = message.get("photo").and_then(Value::as_array)
        && let Some(largest) = sizes.last()
    {
        files.push(FileRef {
            file: largest
                .get("file_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            name: "photo.jpg".into(),
            size: largest
                .get("file_size")
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            mime: "image/jpeg".into(),
        });
    }
    files
}

/// HTML back to text, for when Telegram refuses the markup.
fn plain(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

#[tool_router]
impl Channel {
    #[tool(description = "Who the bot is")]
    async fn look(&self, Parameters(Nothing {}): Parameters<Nothing>) -> Result<Json<Bot>, String> {
        let me = self.api.call("getMe", json!({})).await?;
        let bot = Bot {
            id: me.get("id").map(Value::to_string).unwrap_or_default(),
            username: me
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            name: me
                .get("first_name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            reach: self.reach.lock().await.clone(),
        };
        *self.bot.lock().await = Some(bot.clone());
        Ok(Json(bot))
    }

    #[tool(description = "A delivery the harness's door took in: verified here, then pulled")]
    async fn receive(
        &self,
        Parameters(params): Parameters<ReceiveParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let Some(secret) = &self.door_secret else {
            return Err("this bot is not reached at a door".to_owned());
        };
        let carried = params
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("x-telegram-bot-api-secret-token"))
            .map(|(_, value)| value.as_str())
            .unwrap_or_default();
        if carried != secret {
            return Err(
                "not from the messenger: the secret is not the one the door was given".to_owned(),
            );
        }
        let update: Value = serde_json::from_str(&params.body)
            .map_err(|error| format!("not an update: {error}"))?;
        if let Some(inbound) = self.inbound_of(&update).await {
            self.arrived.lock().await.push_back(inbound);
        }
        Ok(Json(channel::Sent::default()))
    }

    #[tool(description = "The next inbound events, waiting up to wait_s for one")]
    async fn pull(
        &self,
        Parameters(PullParams { wait_s }): Parameters<PullParams>,
    ) -> Result<Json<channel::Pulled>, String> {
        let deadline = Instant::now() + Duration::from_secs(wait_s.min(30));
        loop {
            let events: Vec<Inbound> = self.arrived.lock().await.drain(..).collect();
            if !events.is_empty() || Instant::now() >= deadline {
                return Ok(Json(channel::Pulled { events }));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }

    #[tool(description = "One message to a chat, Markdown")]
    async fn send(
        &self,
        Parameters(params): Parameters<SendParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let markup = params.app.as_ref().map(|app| {
            json!({ "inline_keyboard": [[{ "text": app.label, "web_app": { "url": app.url } }]] })
        });
        let sent = self
            .send_html(
                &params.chat,
                &text::html(&params.markdown),
                params.reply_to.as_deref(),
                markup,
            )
            .await?;
        Ok(Json(channel::Sent {
            reference: sent
                .get("message_id")
                .map(Value::to_string)
                .unwrap_or_default(),
        }))
    }

    #[tool(
        description = "The Workbench's page inside the messenger, one tap away: the bot's menu button"
    )]
    async fn set_app(
        &self,
        Parameters(params): Parameters<SetAppParams>,
    ) -> Result<Json<channel::Sent>, String> {
        self.api
            .call(
                "setChatMenuButton",
                json!({ "menu_button": { "type": "web_app", "text": "Workbench", "web_app": { "url": params.url } } }),
            )
            .await?;
        Ok(Json(channel::Sent::default()))
    }

    #[tool(description = "Who opened a Mini App: the person behind signed initData, or a refusal")]
    async fn verify_app(
        &self,
        Parameters(params): Parameters<VerifyAppParams>,
    ) -> Result<Json<Person>, String> {
        let user = self.api.verify_init_data(&params.init_data)?;
        Ok(Json(person_of(Some(&user))))
    }

    #[tool(description = "A turn begins to be written")]
    async fn stream_begin(
        &self,
        Parameters(params): Parameters<StreamBeginParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let (chat_id, thread) = api::chat_parts(&params.chat);
        // The message being answered wears eyes while it is: a sign that
        // outlives the typing status, which the messenger drops after a
        // few seconds and sometimes never shows.
        let answering = match params.reply_to.as_deref() {
            Some(reference) => self.messages.lock().await.get(reference).copied(),
            None => None,
        };
        if let Some(message_id) = answering {
            let _ = self
                .api
                .call(
                    "setMessageReaction",
                    json!({ "chat_id": chat_id, "message_id": message_id,
                            "reaction": [{ "type": "emoji", "emoji": "👀" }] }),
                )
                .await;
        }
        self.streams.lock().await.insert(
            params.stream.clone(),
            Stream {
                chat: params.chat,
                how: StreamHow::Undecided,
                last_sent: None,
                last_text: String::new(),
                answering,
            },
        );
        // Typing, kept up while the turn is being written: the messenger
        // shows it for a few seconds at a time.
        let api = self.api.clone();
        let streams = Arc::clone(&self.streams);
        let stream = params.stream;
        tokio::spawn(async move {
            loop {
                if !streams.lock().await.contains_key(&stream) {
                    break;
                }
                let mut body = json!({ "chat_id": chat_id, "action": "typing" });
                if let Some(thread) = thread {
                    body["message_thread_id"] = json!(thread);
                }
                let _ = api.call("sendChatAction", body).await;
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });
        Ok(Json(channel::Sent::default()))
    }

    #[tool(description = "What was written so far")]
    async fn stream_update(
        &self,
        Parameters(params): Parameters<StreamParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let mut streams = self.streams.lock().await;
        let Some(stream) = streams.get_mut(&params.stream) else {
            return Ok(Json(channel::Sent::default()));
        };
        if stream
            .last_sent
            .is_some_and(|sent| sent.elapsed() < DRAFT_EVERY)
        {
            // The next update carries all of it; nothing is lost by waiting.
            return Ok(Json(channel::Sent::default()));
        }
        self.draft(stream, &params.markdown, false).await?;
        Ok(Json(channel::Sent::default()))
    }

    #[tool(description = "The turn is written, or was stopped")]
    async fn stream_end(
        &self,
        Parameters(params): Parameters<StreamParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let stream = self.streams.lock().await.remove(&params.stream);
        if let Some(message_id) = stream.as_ref().and_then(|s| s.answering) {
            let (chat_id, _) = api::chat_parts(&params.chat);
            let _ = self
                .api
                .call(
                    "setMessageReaction",
                    json!({ "chat_id": chat_id, "message_id": message_id, "reaction": [] }),
                )
                .await;
        }
        let markdown = if params.stopped == Some(true) {
            format!("{}\n\n_(stopped)_", params.markdown)
        } else {
            params.markdown
        };
        let html = text::html(&markdown);
        if let Some(StreamHow::Edits {
            message_id: Some(id),
        }) = stream.map(|s| s.how)
        {
            {
                let (chat_id, _) = api::chat_parts(&params.chat);
                let pieces = text::chunks(&html, api::TEXT_LIMIT);
                let first = pieces.first().cloned().unwrap_or_default();
                let _ = self
                    .api
                    .call(
                        "editMessageText",
                        json!({ "chat_id": chat_id, "message_id": id, "text": first, "parse_mode": "HTML" }),
                    )
                    .await;
                for piece in pieces.into_iter().skip(1) {
                    self.send_html(&params.chat, &piece, None, None).await?;
                }
                Ok(Json(channel::Sent {
                    reference: id.to_string(),
                }))
            }
        } else {
            let sent = self.send_html(&params.chat, &html, None, None).await?;
            Ok(Json(channel::Sent {
                reference: sent
                    .get("message_id")
                    .map(Value::to_string)
                    .unwrap_or_default(),
            }))
        }
    }

    #[tool(description = "A question with options, as buttons")]
    async fn ask(
        &self,
        Parameters(params): Parameters<AskParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let mut rows = Vec::new();
        {
            let mut buttons = self.buttons.lock().await;
            for option in &params.options {
                let id = option
                    .get("id")
                    .or_else(|| option.get("optionId"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let label = option
                    .get("label")
                    .or_else(|| option.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_owned();
                let token = format!("{}", buttons.len() + 1);
                buttons.insert(token.clone(), (params.question.clone(), id));
                rows.push(vec![
                    json!({ "text": label, "callback_data": format!("a:{token}") }),
                ]);
            }
        }
        let sent = self
            .send_html(
                &params.chat,
                &text::html(&params.title),
                None,
                Some(json!({ "inline_keyboard": rows })),
            )
            .await?;
        Ok(Json(channel::Sent {
            reference: sent
                .get("message_id")
                .map(Value::to_string)
                .unwrap_or_default(),
        }))
    }

    #[tool(description = "A file to a chat")]
    async fn send_file(
        &self,
        Parameters(params): Parameters<SendFileParams>,
    ) -> Result<Json<channel::Sent>, String> {
        let (chat_id, thread) = api::chat_parts(&params.chat);
        let sent = self
            .api
            .send_document(&chat_id, thread, &PathBuf::from(&params.path), &params.name)
            .await?;
        Ok(Json(channel::Sent {
            reference: sent
                .get("message_id")
                .map(Value::to_string)
                .unwrap_or_default(),
        }))
    }

    #[tool(description = "A file that came with a message, fetched into the channel's home")]
    async fn fetch_file(
        &self,
        Parameters(params): Parameters<FetchFileParams>,
    ) -> Result<Json<channel::Fetched>, String> {
        let path = self
            .api
            .fetch_file(&params.file, &self.home.join("files"), "file")
            .await?;
        Ok(Json(channel::Fetched {
            path: path.display().to_string(),
            name: String::new(),
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Channel {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-channel-telegram",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("A Telegram bot as a channel of the SWEM Workbench")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let token = std::env::var(channel::KEY_VARIABLE)
        .ok()
        .filter(|token| !token.trim().is_empty())
        .ok_or("the bot's token is given in SWEM_CHANNEL_KEY")?;
    let home =
        std::env::var_os(channel::HOME_VARIABLE).map_or_else(std::env::temp_dir, PathBuf::from);
    let settings: channel::Settings = std::env::var(channel::SETTINGS_VARIABLE)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default();
    let root = settings
        .api_root
        .filter(|root| !root.trim().is_empty())
        .unwrap_or_else(|| api::PUBLIC_API_ROOT.to_owned());
    let door = settings.door.filter(|door| !door.trim().is_empty());
    let door_secret = door.as_ref().map(|_| {
        use std::fmt::Write as _;
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("the system gives random bytes");
        bytes.iter().fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
    });
    let channel = Arc::new(Channel {
        api: Api::new(&root, &token)?,
        home,
        bot: Mutex::new(None),
        arrived: Arc::new(Mutex::new(VecDeque::new())),
        streams: Arc::new(Mutex::new(BTreeMap::new())),
        buttons: Mutex::new(BTreeMap::new()),
        messages: Mutex::new(BTreeMap::new()),
        drafts: Mutex::new(0),
        reach: Mutex::new("pull".to_owned()),
        door_secret,
        tool_router: Channel::tool_router(),
    });
    // What the bot answers to, offered in the messenger's command menu.
    let _ = channel
        .api
        .call(
            "setMyCommands",
            json!({ "commands": [
                { "command": "status", "description": "How things stand" },
                { "command": "stop", "description": "Stop the agent's turn" },
                { "command": "app", "description": "The Workbench, here" },
            ] }),
        )
        .await;
    // At a door the messenger delivers; otherwise the messenger is asked,
    // and a webhook left from before is taken down first, since Telegram
    // answers no `getUpdates` while one is set.
    let delivered = match (&door, &channel.door_secret) {
        (Some(door), Some(secret)) => channel
            .api
            .call(
                "setWebhook",
                json!({
                    "url": door,
                    "secret_token": secret,
                    "allowed_updates": ["message", "channel_post", "callback_query"],
                }),
            )
            .await
            .map(|_| ()),
        _ => Err(String::new()),
    };
    match delivered {
        Ok(()) => "door".clone_into(&mut *channel.reach.lock().await),
        Err(refusal) => {
            if door.is_some() {
                eprintln!("swem-channel-telegram: the door could not be given: {refusal}");
                *channel.reach.lock().await =
                    format!("pull: the messenger refused the door: {refusal}");
            }
            let _ = channel.api.call("deleteWebhook", json!({})).await;
            tokio::spawn(Arc::clone(&channel).poll_forever());
        }
    }
    channel
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
