//! A channel: how people reach an agent from a messenger.
//!
//! A channel is a package of kind [`CHANNEL_KIND`]: an MCP server program the
//! harness starts and keeps, with the tools below. The harness names no
//! messenger; a channel package carries one messenger's protocol, and
//! answers in these terms. The harness checks a channel's tools against
//! [`shape`] when the package is installed and before the channel is
//! started.
//!
//! Direction: the harness calls, the channel answers. What arrives from the
//! messenger is handed over by [`PULL`]: the harness asks, the channel gives
//! what it has or waits a while. A channel never reaches into the harness.
//!
//! What the harness gives a channel when it starts it is in its
//! environment: [`KEY_VARIABLE`] (the messenger's secret for the bot),
//! [`HOME_VARIABLE`] (a directory of the channel's own, for files it fetches),
//! [`SETTINGS_VARIABLE`] (the channel's settings as one JSON document, as the
//! person set them on the page).

use serde::{Deserialize, Serialize};

use crate::shape::Shape;

/// The kind a channel package is.
pub const CHANNEL_KIND: &str = "swem/channel@1";

/// The variable a channel reads its bot's secret from.
pub const KEY_VARIABLE: &str = "SWEM_CHANNEL_KEY";
/// The variable naming the directory a channel may write into.
pub const HOME_VARIABLE: &str = "SWEM_CHANNEL_HOME";
/// The variable carrying the channel's settings, one JSON object.
pub const SETTINGS_VARIABLE: &str = "SWEM_CHANNEL_SETTINGS";

/// Who the bot is: `look`.
pub const LOOK: &str = "look";
/// The next inbound events: `pull`.
pub const PULL: &str = "pull";
/// One message: `send`.
pub const SEND: &str = "send";
/// A turn as it is written: `stream_begin`, `stream_update`, `stream_end`.
pub const STREAM_BEGIN: &str = "stream_begin";
pub const STREAM_UPDATE: &str = "stream_update";
pub const STREAM_END: &str = "stream_end";
/// A question with options: `ask`.
pub const ASK: &str = "ask";
/// A file out: `send_file`; a file in: `fetch_file`.
pub const SEND_FILE: &str = "send_file";
pub const FETCH_FILE: &str = "fetch_file";
/// A delivery the harness's door forwarded: `receive`. Not in the shape: a
/// channel that answers it can be reached at a door, one that does not
/// pulls.
pub const RECEIVE: &str = "receive";

/// The shape every channel answers to, as a document.
pub const CHANNEL_SHAPE: &str = r#"{
  "schema": "swem:shape@0.1",
  "id": "swem/channel",
  "tools": [
    {"name": "look", "inputSchema": {"type": "object", "properties": {}}},
    {"name": "pull", "inputSchema": {"type": "object",
      "properties": {"wait_s": {"type": "integer"}}, "required": ["wait_s"]}},
    {"name": "send", "inputSchema": {"type": "object",
      "properties": {"chat": {"type": "string"}, "markdown": {"type": "string"}, "reply_to": {"type": "string"}},
      "required": ["chat", "markdown"]}},
    {"name": "stream_begin", "inputSchema": {"type": "object",
      "properties": {"chat": {"type": "string"}, "stream": {"type": "string"}, "reply_to": {"type": "string"}},
      "required": ["chat", "stream"]}},
    {"name": "stream_update", "inputSchema": {"type": "object",
      "properties": {"chat": {"type": "string"}, "stream": {"type": "string"}, "markdown": {"type": "string"}},
      "required": ["chat", "stream", "markdown"]}},
    {"name": "stream_end", "inputSchema": {"type": "object",
      "properties": {"chat": {"type": "string"}, "stream": {"type": "string"}, "markdown": {"type": "string"}, "stopped": {"type": "boolean"}},
      "required": ["chat", "stream", "markdown"]}},
    {"name": "ask", "inputSchema": {"type": "object",
      "properties": {"chat": {"type": "string"}, "question": {"type": "string"}, "title": {"type": "string"},
                     "options": {"type": "array"}},
      "required": ["chat", "question", "title", "options"]}},
    {"name": "send_file", "inputSchema": {"type": "object",
      "properties": {"chat": {"type": "string"}, "path": {"type": "string"}, "name": {"type": "string"}},
      "required": ["chat", "path", "name"]}},
    {"name": "fetch_file", "inputSchema": {"type": "object",
      "properties": {"file": {"type": "string"}}, "required": ["file"]}}
  ]
}"#;

/// The shape every channel answers to.
///
/// # Panics
///
/// Never: the document is this crate's own and is checked by its tests.
#[must_use]
pub fn shape() -> Shape {
    Shape::parse(CHANNEL_SHAPE.as_bytes()).expect("the channel shape is a shape")
}

/// Who the bot is, as `look` answers.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Bot {
    /// The messenger's id for the bot.
    pub id: String,
    /// What people write to reach it, without any sigil.
    #[serde(default)]
    pub username: String,
    /// What the bot is called.
    #[serde(default)]
    pub name: String,
    /// How the channel receives what the messenger has: `pull` when it asks
    /// the messenger, `door` when the messenger delivers to the door it was
    /// given in [`Settings::door`], or why the door could not be used.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reach: String,
}

/// Somebody on the messenger's side.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Person {
    /// The messenger's id for them; stable, unlike a username.
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
}

/// A file that came with a message, as the messenger names it; fetched
/// with `fetch_file`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct FileRef {
    pub file: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
}

/// What `pull` answers: events in the order they happened.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Pulled {
    #[serde(default)]
    pub events: Vec<Inbound>,
}

/// One thing that happened on the messenger's side.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Inbound {
    /// Somebody wrote in a chat. `reference` is the messenger's own id for
    /// the update, which the harness keeps so the same thing said twice is
    /// said once.
    Message {
        chat: ChatRef,
        person: Person,
        #[serde(default)]
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<FileRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        /// Whether the bot was spoken to: always in a direct chat; in a
        /// group, by naming it or replying to it. What is not addressed to
        /// the bot is said in the chat here for the record, and the agent
        /// takes no turn on it. Absent, taken as addressed.
        #[serde(default = "addressed_by_default")]
        addressed: bool,
        reference: String,
    },
    /// Somebody gave the bot a command (`/start`, `/stop`, ...).
    Command {
        chat: ChatRef,
        person: Person,
        name: String,
        #[serde(default)]
        args: String,
        reference: String,
    },
    /// Somebody chose an option of a question the harness asked.
    Answered {
        chat: ChatRef,
        person: Person,
        question: String,
        option: String,
        reference: String,
    },
    /// Somebody stopped a stream from the messenger.
    Stopped {
        chat: ChatRef,
        stream: String,
        reference: String,
    },
    /// Somebody joined a chat the bot is in, or was added to it.
    Joined {
        chat: ChatRef,
        person: Person,
        reference: String,
    },
    /// Somebody left.
    Left {
        chat: ChatRef,
        person: Person,
        reference: String,
    },
}

fn addressed_by_default() -> bool {
    true
}

impl Inbound {
    /// The messenger's reference for the event.
    #[must_use]
    pub fn reference(&self) -> &str {
        match self {
            Self::Message { reference, .. }
            | Self::Command { reference, .. }
            | Self::Answered { reference, .. }
            | Self::Stopped { reference, .. }
            | Self::Joined { reference, .. }
            | Self::Left { reference, .. } => reference,
        }
    }

    /// The chat the event happened in.
    #[must_use]
    pub fn chat(&self) -> &ChatRef {
        match self {
            Self::Message { chat, .. }
            | Self::Command { chat, .. }
            | Self::Answered { chat, .. }
            | Self::Stopped { chat, .. }
            | Self::Joined { chat, .. }
            | Self::Left { chat, .. } => chat,
        }
    }
}

/// A chat on the messenger's side: direct messages with one person, a
/// group, or a topic in a group.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct ChatRef {
    /// The messenger's id for the chat, with the topic when there is one,
    /// as the channel spells it; opaque to the harness.
    pub id: String,
    #[serde(default)]
    pub kind: ChatKind,
    /// What the chat is called on the messenger's side, if anything.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
}

#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ChatKind {
    /// One person and the bot.
    #[default]
    Direct,
    /// Several people and the bot.
    Group,
    /// A topic inside a group.
    Topic,
}

/// One option of a question, as `ask` takes them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Option_ {
    pub id: String,
    pub label: String,
}

/// What `send`, `send_file` and the stream tools answer: the messenger's
/// reference for what was sent, when it has one.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Sent {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reference: String,
}

/// What `fetch_file` answers: where the file now is, under the channel's
/// home.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Fetched {
    pub path: String,
    #[serde(default)]
    pub name: String,
}

/// The settings every channel has, handed in [`SETTINGS_VARIABLE`] beside
/// whatever the package's own settings are.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, schemars::JsonSchema)]
pub struct Settings {
    /// Where the messenger's API is, when not its public one: a local Bot
    /// API server, or a fixture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_root: Option<String>,
    /// Where deliveries may be pushed to (a webhook), when the harness has
    /// an address; absent, the channel pulls. What arrives at the door is
    /// handed to the channel's [`RECEIVE`] tool as it came, headers and
    /// body, for the channel to verify.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shape_is_a_shape_and_names_every_tool() {
        let shape = shape();
        let names: Vec<&str> = shape.tools.iter().map(|tool| tool.name.as_str()).collect();
        for tool in [
            LOOK,
            PULL,
            SEND,
            STREAM_BEGIN,
            STREAM_UPDATE,
            STREAM_END,
            ASK,
            SEND_FILE,
            FETCH_FILE,
        ] {
            assert!(names.contains(&tool), "{tool} is not in the shape");
        }
    }

    #[test]
    fn an_inbound_event_reads_and_writes_as_tagged_json() {
        let event = Inbound::Message {
            chat: ChatRef {
                id: "12".into(),
                kind: ChatKind::Direct,
                title: String::new(),
            },
            person: Person {
                id: "7".into(),
                name: "Ada".into(),
                username: "ada".into(),
            },
            text: "hello".into(),
            files: vec![],
            reply_to: None,
            addressed: true,
            reference: "u1".into(),
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["kind"], "message");
        assert_eq!(json["chat"]["kind"], "direct");
        let back: Inbound = serde_json::from_value(json).unwrap();
        assert_eq!(back, event);
        assert_eq!(back.reference(), "u1");
    }
}
