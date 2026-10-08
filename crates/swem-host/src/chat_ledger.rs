//! Who is in a chat and who said what: the part of the ledger a person means
//! by "a conversation".
//!
//! A route is one agent's one native session. A chat is what a person opens:
//! it has members, every message in it has one sender, and it holds a session
//! per agent that can be replaced without the chat ending. See ADR-0007.
//!
//! Nothing here is given back to an engine as a conversation to continue.
//! What an engine is told about who spoke is made from these records by
//! `envelope`, one block per turn.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{RoutingError, RoutingLedger, SessionRouteBinding, SurfaceEventSource};

pub(crate) const CHATS_SCHEMA: &str = "
  CREATE TABLE participants (
    participant_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE COLLATE NOCASE,
    name TEXT NOT NULL,
    colour TEXT,
    profile_id TEXT UNIQUE,
    made_by TEXT REFERENCES participants(participant_id),
    retired_ms INTEGER,
    created_ms INTEGER NOT NULL
  );
  CREATE TABLE chats (
    chat_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    created_by TEXT NOT NULL REFERENCES participants(participant_id),
    created_ms INTEGER,
    answer_rule TEXT NOT NULL DEFAULT 'named',
    reply_limit INTEGER NOT NULL DEFAULT 4,
    agent_replies INTEGER NOT NULL DEFAULT 0
  );
  CREATE TABLE chat_members (
    chat_id TEXT NOT NULL REFERENCES chats(chat_id),
    participant_id TEXT NOT NULL REFERENCES participants(participant_id),
    joined_ms INTEGER,
    left_ms INTEGER,
    given_through INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(chat_id, participant_id)
  );
  CREATE TABLE sessions (
    session_id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL REFERENCES chats(chat_id),
    agent_id TEXT NOT NULL REFERENCES participants(participant_id),
    route_id TEXT NOT NULL UNIQUE REFERENCES routes(route_id),
    state TEXT NOT NULL,
    began_ms INTEGER,
    ended_why TEXT
  );
  CREATE UNIQUE INDEX sessions_current
    ON sessions(chat_id, agent_id) WHERE state = 'current';";

pub(crate) const MESSAGES_SCHEMA: &str = "
  CREATE TABLE messages (
    message_id TEXT PRIMARY KEY,
    sequence INTEGER NOT NULL UNIQUE REFERENCES events(sequence),
    chat_id TEXT NOT NULL REFERENCES chats(chat_id),
    sender_id TEXT NOT NULL REFERENCES participants(participant_id),
    channel TEXT NOT NULL,
    channel_ref TEXT,
    content_json TEXT NOT NULL,
    text TEXT NOT NULL,
    named_json TEXT NOT NULL DEFAULT '[]',
    answers TEXT REFERENCES messages(message_id),
    created_ms INTEGER
  );
  CREATE UNIQUE INDEX messages_channel_ref
    ON messages(chat_id, channel, channel_ref) WHERE channel_ref IS NOT NULL;
  CREATE INDEX messages_chat_sequence ON messages(chat_id, sequence);
  CREATE TABLE deliveries (
    delivery_id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL REFERENCES chats(chat_id),
    agent_id TEXT NOT NULL REFERENCES participants(participant_id),
    message_id TEXT NOT NULL REFERENCES messages(message_id),
    state TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    started_ms INTEGER,
    ended_ms INTEGER,
    outcome TEXT,
    UNIQUE(agent_id, message_id)
  );
  CREATE INDEX deliveries_agent_state ON deliveries(agent_id, state, created_ms);
  CREATE TABLE questions (
    question_id TEXT PRIMARY KEY,
    chat_id TEXT NOT NULL REFERENCES chats(chat_id),
    agent_id TEXT NOT NULL REFERENCES participants(participant_id),
    delivery_id TEXT REFERENCES deliveries(delivery_id),
    kind TEXT NOT NULL,
    asked_json TEXT NOT NULL,
    asked_ms INTEGER NOT NULL,
    wait_until_ms INTEGER,
    state TEXT NOT NULL,
    answer_json TEXT,
    answered_by TEXT REFERENCES participants(participant_id),
    answered_ms INTEGER
  );
  CREATE INDEX questions_state ON questions(state, chat_id);";

/// Who a participant is on a messenger's side, and which chats there are
/// which chats here. Made when missing, beside the rest: the schema version
/// does not move for it.
pub(crate) const CHANNELS_SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS identities (
    channel_id TEXT NOT NULL,
    external_id TEXT NOT NULL,
    participant_id TEXT NOT NULL REFERENCES participants(participant_id),
    name TEXT NOT NULL,
    bound_ms INTEGER NOT NULL,
    direct_chat TEXT,
    may_speak INTEGER NOT NULL DEFAULT 0,
    told_ms INTEGER,
    bot INTEGER NOT NULL DEFAULT 0,
    may_command INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(channel_id, external_id)
  );
  CREATE INDEX IF NOT EXISTS identities_participant ON identities(participant_id);
  CREATE TABLE IF NOT EXISTS channel_chats (
    channel_id TEXT NOT NULL,
    external_chat TEXT NOT NULL,
    chat_id TEXT NOT NULL REFERENCES chats(chat_id),
    bound_ms INTEGER NOT NULL,
    PRIMARY KEY(channel_id, external_chat)
  );
  CREATE INDEX IF NOT EXISTS channel_chats_chat ON channel_chats(chat_id);";

const IDENTITY_COLUMNS: &str = "channel_id, external_id, participant_id, name, bound_ms, direct_chat, may_speak, told_ms, bot, may_command";

fn identity_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    Ok(Identity {
        channel_id: row.get(0)?,
        external_id: row.get(1)?,
        participant_id: row.get(2)?,
        name: row.get(3)?,
        bound_ms: row.get(4)?,
        direct_chat: row.get(5)?,
        may_speak: row.get::<_, i32>(6)? != 0,
        told_ms: row.get(7)?,
        bot: row.get::<_, i32>(8)? != 0,
        may_command: row.get::<_, i32>(9)? != 0,
    })
}

/// Columns of `identities` that came later, with how they are added to a
/// ledger whose table predates them.
pub(crate) const IDENTITY_COLUMNS_ADDED: [(&str, &str); 5] = [
    ("direct_chat", "TEXT"),
    ("may_speak", "INTEGER NOT NULL DEFAULT 0"),
    ("told_ms", "INTEGER"),
    ("bot", "INTEGER NOT NULL DEFAULT 0"),
    ("may_command", "INTEGER NOT NULL DEFAULT 0"),
];

/// The channel a message came through when it was typed into the Workbench.
pub const CHANNEL_WORKBENCH: &str = "workbench";
/// The channel of a message an editor sent through the editor door.
pub const CHANNEL_EDITOR: &str = "editor";
/// The channel of a message that arrived on time.
pub const CHANNEL_SCHEDULE: &str = "schedule";
/// The channel of what an agent said in its own turn.
pub const CHANNEL_AGENT: &str = "agent";

/// The channel of a message that came through a channel a person added: a
/// messenger. The channel's id follows the colon.
#[must_use]
pub fn channel_of(channel_id: &str) -> String {
    format!("channel:{channel_id}")
}

/// Who a participant is on a messenger's side.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Identity {
    pub channel_id: String,
    pub external_id: String,
    pub participant_id: String,
    /// What the messenger called them when they were bound.
    pub name: String,
    pub bound_ms: i64,
    /// The chat on the messenger's side where they and the bot talk alone,
    /// when known: where a guest is reached once they are let into a chat.
    pub direct_chat: Option<String>,
    /// Whether the agent takes a turn on what they say: the owner always;
    /// a guest when the owner allowed it, or the channel's default did.
    pub may_speak: bool,
    /// When a guest who may not speak was told so, if ever: once is enough.
    pub told_ms: Option<i64>,
    /// The messenger says they are a bot: what they say to the agent is
    /// answered on a budget of turns since a person last spoke in the chat.
    pub bot: bool,
    /// A guest the owner lets reset and compact the agent's session: their
    /// `/clear` and `/compact` reach the agent as typed, as the owner's do.
    /// In a group the session is everybody's, so this is the owner's to
    /// give.
    pub may_command: bool,
}

/// A chat on a messenger's side that is a chat here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChannelChat {
    pub channel_id: String,
    pub external_chat: String,
    pub chat_id: String,
    pub bound_ms: i64,
}

/// Who can say something in a chat.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantKind {
    Person,
    Agent,
    Guest,
    Schedule,
}

impl ParticipantKind {
    fn of(word: &str) -> Result<Self, RoutingError> {
        match word {
            "person" => Ok(Self::Person),
            "agent" => Ok(Self::Agent),
            "guest" => Ok(Self::Guest),
            "schedule" => Ok(Self::Schedule),
            other => Err(RoutingError::InvalidBinding(format!(
                "unknown kind of participant: {other}"
            ))),
        }
    }
}

/// One who can say something: a person, an agent, a guest or a schedule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Participant {
    pub participant_id: String,
    pub kind: ParticipantKind,
    /// How it is named in a chat: `@handle`.
    pub handle: String,
    pub name: String,
    /// A role of the palette, never a colour of its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    /// The profile an agent is made of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    /// Who made it, for a schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made_by: Option<String>,
    /// Its profile is gone; what it said stays.
    pub retired: bool,
}

/// A chat, as a person picks it from a list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Chat {
    pub chat_id: String,
    /// What a person called it, or its first words when they called it nothing.
    pub title: String,
    pub created_by: String,
    pub answer_rule: String,
    pub reply_limit: u32,
    pub agent_replies: u32,
    pub members: Vec<Participant>,
    /// Where the chat's record ends, absent when nothing was ever recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_at_ms: Option<u64>,
}

/// What one participant said.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Message {
    pub message_id: String,
    /// Its place in the one order everything has.
    pub sequence: u64,
    pub chat_id: String,
    pub sender_id: String,
    pub channel: String,
    /// The channel's own name for the message: what an editor called
    /// itself, the window a schedule claimed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_ref: Option<String>,
    pub content: Value,
    pub text: String,
    /// The handles named in it.
    pub named: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_ms: Option<u64>,
}

/// An agent's session in a chat: which route is the engine's memory of it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChatSession {
    pub session_id: String,
    pub chat_id: String,
    pub agent_id: String,
    pub route_id: String,
    pub current: bool,
}

/// One thing that happened, for a page that follows everything with one cursor.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ChatEvent {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    /// The agent whose session this happened in, for what an engine said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub kind: String,
    pub source: SurfaceEventSource,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_ms: Option<u64>,
}

/// Milliseconds since the epoch, as the ledger keeps time.
#[must_use]
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

const CROCKFORD: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

/// An id of one kind for everything the ledger names: a prefix, then
/// forty-eight bits of time and eighty of chance, so ids of one kind sort by
/// when they were made and two made in the same millisecond still differ.
///
/// # Errors
///
/// When this machine gives no random bytes.
pub fn new_id(prefix: &str) -> Result<String, RoutingError> {
    let mut chance = [0_u8; 10];
    getrandom::fill(&mut chance)
        .map_err(|error| RoutingError::Projection(format!("no random bytes: {error}")))?;
    let time = u64::try_from(now_ms()).unwrap_or(0);
    let mut bits: u128 = u128::from(time & 0xffff_ffff_ffff);
    for byte in chance {
        bits = (bits << 8) | u128::from(byte);
    }
    let mut id = String::with_capacity(prefix.len() + 27);
    id.push_str(prefix);
    id.push('_');
    for position in (0..26).rev() {
        let digit = usize::try_from((bits >> (position * 5)) & 31).unwrap_or(0);
        id.push(char::from(CROCKFORD[digit]));
    }
    Ok(id)
}

/// A handle made from a name somebody already has: lower case, and what a
/// handle cannot hold turned into a dash.
#[must_use]
pub fn handle_from(name: &str) -> String {
    let mut handle = String::new();
    for character in name.chars() {
        let character = character.to_ascii_lowercase();
        let kept = character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || (matches!(character, '-' | '_') && !handle.is_empty());
        if kept {
            handle.push(character);
        } else if !handle.is_empty() && !handle.ends_with('-') {
            handle.push('-');
        }
        if handle.len() >= 32 {
            break;
        }
    }
    let handle = handle.trim_end_matches(['-', '_']).to_owned();
    if handle.is_empty() {
        "someone".to_owned()
    } else {
        handle
    }
}

/// Whether a handle is one a chat can carry.
#[must_use]
pub fn is_handle(handle: &str) -> bool {
    let mut bytes = handle.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && handle.len() <= 32
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

/// The handles a text names, in the order it names them, each once. A handle
/// inside a code span or a code block names nobody.
#[must_use]
pub fn handles_named(text: &str) -> Vec<String> {
    let mut named: Vec<String> = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if in_block {
            continue;
        }
        let mut in_span = false;
        let characters: Vec<char> = line.chars().collect();
        for (index, character) in characters.iter().enumerate() {
            if *character == '`' {
                in_span = !in_span;
            } else if *character == '@'
                && !in_span
                && (index == 0 || !characters[index - 1].is_alphanumeric())
            {
                let handle: String = characters[index + 1..]
                    .iter()
                    .take_while(|next| next.is_ascii_alphanumeric() || matches!(next, '-' | '_'))
                    .collect::<String>()
                    .to_ascii_lowercase();
                let handle = handle.trim_end_matches(['-', '_']).to_owned();
                if is_handle(&handle) && !named.contains(&handle) {
                    named.push(handle);
                }
            }
        }
    }
    named
}

/// The words a chat is known by when nobody named it: the first line of what
/// was first said, short enough to be a name.
fn opening_words(text: &str) -> String {
    const WORDS: usize = 72;
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.chars().count() <= WORDS {
        return line.to_owned();
    }
    let cut: String = line.chars().take(WORDS).collect();
    format!("{}…", cut.trim_end())
}

/// Copy the ledger beside itself before it is changed, once.
pub(crate) fn keep_a_copy(connection: &Connection, path: &Path) -> Result<(), RoutingError> {
    let mut copy = path.as_os_str().to_owned();
    copy.push(".v1");
    let copy = std::path::PathBuf::from(copy);
    if copy.exists() {
        return Ok(());
    }
    // What is still in the write-ahead log goes into the file first, so the
    // copy is the whole ledger and not the part that had been checkpointed.
    connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    std::fs::copy(path, &copy).map_err(|error| {
        RoutingError::InvalidBinding(format!(
            "the ledger could not be copied to {} before it is changed: {error}",
            copy.display()
        ))
    })?;
    Ok(())
}

/// An event of a chat that belongs to no route: the host's own record of
/// what happened to the chat.
pub(crate) fn chat_event_in(
    transaction: &Transaction<'_>,
    chat_id: &str,
    kind: &str,
    payload: &Value,
) -> Result<i64, RoutingError> {
    transaction.execute(
        "INSERT INTO events(route_id, chat_id, event_id, kind, source, payload_json, at_ms)
         VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            chat_id,
            new_id("e")?,
            kind,
            serde_json::to_string(&SurfaceEventSource::Host)?,
            serde_json::to_string(payload)?,
            now_ms()
        ],
    )?;
    Ok(transaction.last_insert_rowid())
}

fn text_of_blocks(content: &Value) -> String {
    content
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| {
                    (block.get("type").and_then(Value::as_str) == Some("text"))
                        .then(|| block.get("text").and_then(Value::as_str))
                        .flatten()
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn owner_in(transaction: &Transaction<'_>) -> Result<String, RoutingError> {
    if let Some(owner) = transaction
        .query_row(
            "SELECT participant_id FROM participants
             WHERE kind = 'person' ORDER BY created_ms, participant_id LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(owner);
    }
    let name = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "you".to_owned());
    let owner = new_id("p")?;
    transaction.execute(
        "INSERT INTO participants(participant_id, kind, handle, name, created_ms)
         VALUES (?1, 'person', ?2, ?3, ?4)",
        params![
            owner,
            free_handle(transaction, &handle_from(&name))?,
            name,
            now_ms()
        ],
    )?;
    Ok(owner)
}

/// [`free_handle`], for the tables that stand beside these.
pub(crate) fn free_handle_in(
    transaction: &Transaction<'_>,
    wanted: &str,
) -> Result<String, RoutingError> {
    free_handle(transaction, wanted)
}

fn free_handle(transaction: &Transaction<'_>, wanted: &str) -> Result<String, RoutingError> {
    let mut handle = wanted.to_owned();
    let mut attempt = 1_u32;
    loop {
        let taken: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM participants WHERE handle = ?1)",
            [&handle],
            |row| row.get(0),
        )?;
        if !taken {
            return Ok(handle);
        }
        attempt += 1;
        let suffix = format!("-{attempt}");
        let keep = 32_usize.saturating_sub(suffix.len());
        handle = format!("{}{suffix}", &wanted[..wanted.len().min(keep)]);
    }
}

const AGENT_COLOURS: [&str; 4] = ["info", "success", "warning", "danger"];

fn agent_in(transaction: &Transaction<'_>, profile_id: &str) -> Result<String, RoutingError> {
    if let Some(agent) = transaction
        .query_row(
            "SELECT participant_id FROM participants WHERE profile_id = ?1",
            [profile_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    {
        return Ok(agent);
    }
    let agents: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM participants WHERE kind = 'agent'",
        [],
        |row| row.get(0),
    )?;
    let colour = AGENT_COLOURS[usize::try_from(agents).unwrap_or(0) % AGENT_COLOURS.len()];
    let agent = new_id("p")?;
    transaction.execute(
        "INSERT INTO participants(participant_id, kind, handle, name, colour, profile_id, created_ms)
         VALUES (?1, 'agent', ?2, ?3, ?4, ?5, ?6)",
        params![
            agent,
            free_handle(transaction, &handle_from(profile_id))?,
            profile_id,
            colour,
            profile_id,
            now_ms()
        ],
    )?;
    Ok(agent)
}

fn chat_in(
    transaction: &Transaction<'_>,
    title: &str,
    created_by: &str,
    members: &[&str],
    created_ms: Option<i64>,
) -> Result<String, RoutingError> {
    let chat = new_id("c")?;
    transaction.execute(
        "INSERT INTO chats(chat_id, title, created_by, created_ms) VALUES (?1, ?2, ?3, ?4)",
        params![chat, title, created_by, created_ms],
    )?;
    for member in members {
        transaction.execute(
            "INSERT OR IGNORE INTO chat_members(chat_id, participant_id, joined_ms)
             VALUES (?1, ?2, ?3)",
            params![chat, member, created_ms],
        )?;
    }
    Ok(chat)
}

fn session_in(
    transaction: &Transaction<'_>,
    chat_id: &str,
    agent_id: &str,
    route_id: &str,
    began_ms: Option<i64>,
    why_the_last_ended: &str,
) -> Result<String, RoutingError> {
    transaction.execute(
        "UPDATE sessions SET state = 'superseded', ended_why = ?3
         WHERE chat_id = ?1 AND agent_id = ?2 AND state = 'current'",
        params![chat_id, agent_id, why_the_last_ended],
    )?;
    let session = new_id("s")?;
    transaction.execute(
        "INSERT INTO sessions(session_id, chat_id, agent_id, route_id, state, began_ms)
         VALUES (?1, ?2, ?3, ?4, 'current', ?5)",
        params![session, chat_id, agent_id, route_id, began_ms],
    )?;
    transaction.execute(
        "UPDATE events SET chat_id = ?1 WHERE route_id = ?2 AND chat_id IS NULL",
        params![chat_id, route_id],
    )?;
    Ok(session)
}

/// Every route that has no chat yet becomes one: a chat of the owner and the
/// route's agent, with what was said read out of what was recorded.
pub(crate) fn chats_from_routes(transaction: &Transaction<'_>) -> Result<(), RoutingError> {
    let routes: Vec<(String, String)> = {
        let mut statement = transaction.prepare(
            "SELECT route_id, agent_profile_id FROM routes
             WHERE route_id NOT IN (SELECT route_id FROM sessions)
             ORDER BY rowid",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };
    if routes.is_empty() {
        return Ok(());
    }
    let owner = owner_in(transaction)?;
    for (route_id, profile_id) in routes {
        let agent = agent_in(transaction, &profile_id)?;
        let chat = chat_in(transaction, "", &owner, &[&owner, &agent], None)?;
        session_in(transaction, &chat, &agent, &route_id, None, "")?;
        spoken_on_route(transaction, &chat, &route_id)?;
    }
    Ok(())
}

/// What was said on a route, read out of its events into messages.
fn spoken_on_route(
    transaction: &Transaction<'_>,
    chat_id: &str,
    route_id: &str,
) -> Result<(), RoutingError> {
    let events: Vec<(i64, String, String, String)> = {
        let mut statement = transaction.prepare(
            "SELECT sequence, kind, source, payload_json FROM events
             WHERE route_id = ?1 AND kind IN ('host/prompt_submitted', 'acp/prompt_response')
             ORDER BY sequence",
        )?;
        let rows = statement.query_map([route_id], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })?;
        rows.collect::<Result<_, _>>()?
    };
    for (sequence, kind, source, payload) in events {
        spoken_by_event(
            transaction,
            &Happened {
                route_id,
                chat_id,
                sequence,
                kind: &kind,
                source: &source,
                payload: &serde_json::from_str(&payload)?,
                at_ms: None,
            },
        )?;
    }
    Ok(())
}

/// How the block the host ends a turn with begins. A prompt that carries it
/// is a message of the chat being given to an agent, and was kept when it
/// was said.
pub(crate) const BLOCK_OPENS: &str = "<swem:turn k=\"";

/// An event of a route, as the one who keeps messages reads it.
pub(crate) struct Happened<'a> {
    pub route_id: &'a str,
    pub chat_id: &'a str,
    pub sequence: i64,
    pub kind: &'a str,
    /// As the ledger stores it: a JSON string.
    pub source: &'a str,
    pub payload: &'a Value,
    pub at_ms: Option<i64>,
}

/// What an event of a route says was said, kept as a message at the event's
/// own place.
///
/// A person's message is the prompt that was submitted, without the line the
/// host used to append about who wrote it. An agent's message is what it said
/// in one turn, whole, kept when the turn ends. What an engine replayed when
/// a session was loaded is the same words a second time and is left out.
pub(crate) fn spoken_by_event(
    transaction: &Transaction<'_>,
    happened: &Happened<'_>,
) -> Result<(), RoutingError> {
    if happened.source.contains("native_replay") {
        return Ok(());
    }
    // Where this turn began: after the last prompt or the last answer.
    let since: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(sequence), 0) FROM events
         WHERE route_id = ?1 AND sequence < ?2
           AND kind IN ('host/prompt_submitted', 'acp/prompt_response')",
        params![happened.route_id, happened.sequence],
        |row| row.get(0),
    )?;
    match happened.kind {
        "host/prompt_submitted" => a_persons_prompt(transaction, happened, since),
        "acp/prompt_response" => an_agents_turn(transaction, happened, since),
        _ => Ok(()),
    }
}

fn a_persons_prompt(
    transaction: &Transaction<'_>,
    happened: &Happened<'_>,
    since: i64,
) -> Result<(), RoutingError> {
    let mut blocks = happened
        .payload
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let last = blocks
        .last()
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if last.starts_with(BLOCK_OPENS) {
        return Ok(());
    }
    let writer: Option<Value> = transaction
        .query_row(
            "SELECT payload_json FROM events
             WHERE route_id = ?1 AND kind = 'host/turn_written'
               AND sequence > ?2 AND sequence < ?3
             ORDER BY sequence DESC LIMIT 1",
            params![happened.route_id, since, happened.sequence],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|payload| serde_json::from_str::<Value>(&payload))
        .transpose()?
        .and_then(|payload| payload.get("correspondent").cloned());
    let channel = match writer
        .as_ref()
        .and_then(|writer| writer.get("surface"))
        .and_then(Value::as_str)
    {
        Some("schedule") => CHANNEL_SCHEDULE,
        Some("editor") => CHANNEL_EDITOR,
        _ => CHANNEL_WORKBENCH,
    };
    // The line about who wrote it was the last block, and was the host's,
    // not the person's.
    if writer.is_some() && last.starts_with('[') && last.ends_with(']') {
        blocks.pop();
    }
    let owner = owner_in(transaction)?;
    message_in(
        transaction,
        &Said {
            chat_id: happened.chat_id,
            sender_id: &owner,
            channel,
            channel_ref: None,
            content: &Value::Array(blocks),
            sequence: happened.sequence,
            created_ms: happened.at_ms,
        },
    )?;
    Ok(())
}

fn an_agents_turn(
    transaction: &Transaction<'_>,
    happened: &Happened<'_>,
    since: i64,
) -> Result<(), RoutingError> {
    let chunks: Vec<String> = {
        let mut statement = transaction.prepare(
            "SELECT payload_json FROM events
             WHERE route_id = ?1 AND kind = 'acp/session_update'
               AND source NOT LIKE '%native_replay%'
               AND sequence > ?2 AND sequence < ?3
             ORDER BY sequence",
        )?;
        let rows = statement.query_map(
            params![happened.route_id, since, happened.sequence],
            |row| row.get(0),
        )?;
        rows.collect::<Result<_, _>>()?
    };
    let mut said = String::new();
    for chunk in chunks {
        let chunk: Value = serde_json::from_str(&chunk)?;
        let update = chunk.get("update");
        if update
            .and_then(|update| update.get("sessionUpdate"))
            .and_then(Value::as_str)
            == Some("agent_message_chunk")
            && let Some(text) = update
                .and_then(|update| update.get("content"))
                .and_then(|content| content.get("text"))
                .and_then(Value::as_str)
        {
            said.push_str(text);
        }
    }
    if said.trim().is_empty() {
        return Ok(());
    }
    let agent: String = transaction.query_row(
        "SELECT agent_id FROM sessions WHERE route_id = ?1",
        [happened.route_id],
        |row| row.get(0),
    )?;
    message_in(
        transaction,
        &Said {
            chat_id: happened.chat_id,
            sender_id: &agent,
            channel: CHANNEL_AGENT,
            channel_ref: None,
            content: &json!([{ "type": "text", "text": said }]),
            sequence: happened.sequence,
            created_ms: happened.at_ms,
        },
    )?;
    Ok(())
}

struct Said<'a> {
    chat_id: &'a str,
    sender_id: &'a str,
    channel: &'a str,
    channel_ref: Option<&'a str>,
    content: &'a Value,
    sequence: i64,
    created_ms: Option<i64>,
}

fn message_in(transaction: &Transaction<'_>, said: &Said<'_>) -> Result<String, RoutingError> {
    let text = text_of_blocks(said.content);
    let message = new_id("m")?;
    transaction.execute(
        "INSERT INTO messages(message_id, sequence, chat_id, sender_id, channel, channel_ref,
                              content_json, text, named_json, created_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            message,
            said.sequence,
            said.chat_id,
            said.sender_id,
            said.channel,
            said.channel_ref,
            serde_json::to_string(said.content)?,
            text,
            serde_json::to_string(&handles_named(&text))?,
            said.created_ms
        ],
    )?;
    Ok(message)
}

fn participant_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Participant, String)> {
    let kind: String = row.get(1)?;
    Ok((
        Participant {
            participant_id: row.get(0)?,
            kind: ParticipantKind::Person,
            handle: row.get(2)?,
            name: row.get(3)?,
            colour: row.get(4)?,
            profile_id: row.get(5)?,
            made_by: row.get(6)?,
            retired: row.get::<_, Option<i64>>(7)?.is_some(),
        },
        kind,
    ))
}

const PARTICIPANT_COLUMNS: &str =
    "participant_id, kind, handle, name, colour, profile_id, made_by, retired_ms";

fn message_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Message, String, String)> {
    Ok((
        Message {
            message_id: row.get(0)?,
            sequence: u64::try_from(row.get::<_, i64>(1)?).unwrap_or(0),
            chat_id: row.get(2)?,
            sender_id: row.get(3)?,
            channel: row.get(4)?,
            channel_ref: row.get(9)?,
            content: Value::Null,
            text: row.get(6)?,
            named: Vec::new(),
            created_ms: row
                .get::<_, Option<i64>>(8)?
                .and_then(|at| u64::try_from(at).ok()),
        },
        row.get(5)?,
        row.get(7)?,
    ))
}

const MESSAGE_COLUMNS: &str = "message_id, sequence, chat_id, sender_id, channel, content_json, text, named_json, \
     created_ms, channel_ref";

impl RoutingLedger {
    fn participants_where(
        &self,
        condition: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Participant>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {PARTICIPANT_COLUMNS} FROM participants {condition}"
        ))?;
        let rows = statement.query_map(values, participant_of)?;
        let mut participants = Vec::new();
        for row in rows {
            let (mut participant, kind) = row?;
            participant.kind = ParticipantKind::of(&kind)?;
            participants.push(participant);
        }
        Ok(participants)
    }

    /// The person this Workbench is; made the first time anybody asks.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn owner(&mut self) -> Result<Participant, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner = owner_in(&transaction)?;
        transaction.commit()?;
        self.participant(&owner)
    }

    /// The participant an agent's profile is, made the first time it is met.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn agent_of_profile(&mut self, profile_id: &str) -> Result<Participant, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let agent = agent_in(&transaction, profile_id)?;
        transaction.commit()?;
        self.participant(&agent)
    }

    /// The participant a schedule is, made the first time it is met: it
    /// speaks with the trust of whoever made it.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a maker nobody is.
    pub fn schedule_participant(
        &mut self,
        name: &str,
        made_by: &str,
    ) -> Result<Participant, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let known: Option<String> = transaction
            .query_row(
                "SELECT participant_id FROM participants
                 WHERE kind = 'schedule' AND name = ?1 AND made_by = ?2",
                params![name, made_by],
                |row| row.get(0),
            )
            .optional()?;
        let schedule = if let Some(known) = known {
            known
        } else {
            let schedule = new_id("p")?;
            transaction.execute(
                "INSERT INTO participants(participant_id, kind, handle, name, made_by, created_ms)
                 VALUES (?1, 'schedule', ?2, ?3, ?4, ?5)",
                params![
                    schedule,
                    free_handle(&transaction, &handle_from(name))?,
                    name,
                    made_by,
                    now_ms()
                ],
            )?;
            schedule
        };
        transaction.commit()?;
        self.participant(&schedule)
    }

    /// Who somebody on a messenger's side is here, if they were bound.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn participant_of_identity(
        &self,
        channel_id: &str,
        external_id: &str,
    ) -> Result<Option<Participant>, RoutingError> {
        let known: Option<String> = self
            .connection
            .query_row(
                "SELECT participant_id FROM identities WHERE channel_id = ?1 AND external_id = ?2",
                params![channel_id, external_id],
                |row| row.get(0),
            )
            .optional()?;
        known.map(|id| self.participant(&id)).transpose()
    }

    /// The identities a participant has on messengers' sides.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn identities_of(&self, participant_id: &str) -> Result<Vec<Identity>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM identities WHERE participant_id = ?1 ORDER BY bound_ms"
        ))?;
        let rows = statement.query_map([participant_id], identity_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Who somebody on a messenger's side is here, with how they stand.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn identity_of(
        &self,
        channel_id: &str,
        external_id: &str,
    ) -> Result<Option<Identity>, RoutingError> {
        self.connection
            .query_row(
                &format!("SELECT {IDENTITY_COLUMNS} FROM identities WHERE channel_id = ?1 AND external_id = ?2"),
                params![channel_id, external_id],
                identity_row,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Everybody a channel has met, the owner first.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn identities_at(&self, channel_id: &str) -> Result<Vec<Identity>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {IDENTITY_COLUMNS} FROM identities WHERE channel_id = ?1 ORDER BY bound_ms"
        ))?;
        let rows = statement.query_map([channel_id], identity_row)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Whether a guest may speak to the agent through a channel, as the
    /// owner decides.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub fn let_speak(
        &mut self,
        channel_id: &str,
        participant_id: &str,
        may_speak: bool,
    ) -> Result<(), RoutingError> {
        self.connection.execute(
            "UPDATE identities SET may_speak = ?3 WHERE channel_id = ?1 AND participant_id = ?2",
            params![channel_id, participant_id, i32::from(may_speak)],
        )?;
        Ok(())
    }

    /// Whether a guest may reset and compact the agent's session.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub fn let_command(
        &mut self,
        channel_id: &str,
        participant_id: &str,
        may_command: bool,
    ) -> Result<(), RoutingError> {
        self.connection.execute(
            "UPDATE identities SET may_command = ?3 WHERE channel_id = ?1 AND participant_id = ?2",
            params![channel_id, participant_id, i32::from(may_command)],
        )?;
        Ok(())
    }

    /// A guest who may not speak was told so now.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub fn told(&mut self, channel_id: &str, external_id: &str) -> Result<(), RoutingError> {
        self.connection.execute(
            "UPDATE identities SET told_ms = ?3 WHERE channel_id = ?1 AND external_id = ?2",
            params![channel_id, external_id, now_ms()],
        )?;
        Ok(())
    }

    /// Where a guest is reached alone on a messenger's side: the direct
    /// chat of their identity on that channel, when it is known.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn direct_chat_of(
        &self,
        channel_id: &str,
        participant_id: &str,
    ) -> Result<Option<String>, RoutingError> {
        let found: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT direct_chat FROM identities
                 WHERE channel_id = ?1 AND participant_id = ?2 AND direct_chat IS NOT NULL
                 ORDER BY bound_ms DESC LIMIT 1",
                params![channel_id, participant_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(found.flatten())
    }

    /// Bind somebody on a messenger's side to a participant here: the owner
    /// who gave the pairing code, or a guest. An identity bound before is
    /// bound again to this participant.
    ///
    /// # Errors
    ///
    /// The participant is unknown, or the ledger could not be written.
    pub fn bind_identity(
        &mut self,
        channel_id: &str,
        external_id: &str,
        participant_id: &str,
        name: &str,
        direct_chat: Option<&str>,
    ) -> Result<Identity, RoutingError> {
        self.participant(participant_id)?;
        let bound_ms = now_ms();
        self.connection.execute(
            "INSERT INTO identities(channel_id, external_id, participant_id, name, bound_ms, direct_chat, may_speak)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1)
             ON CONFLICT(channel_id, external_id)
             DO UPDATE SET participant_id = excluded.participant_id, name = excluded.name,
                           bound_ms = excluded.bound_ms, may_speak = 1,
                           direct_chat = COALESCE(excluded.direct_chat, identities.direct_chat)",
            params![channel_id, external_id, participant_id, name, bound_ms, direct_chat],
        )?;
        Ok(Identity {
            channel_id: channel_id.to_owned(),
            external_id: external_id.to_owned(),
            participant_id: participant_id.to_owned(),
            name: name.to_owned(),
            bound_ms,
            direct_chat: direct_chat.map(str::to_owned),
            may_speak: true,
            told_ms: None,
            bot: false,
            may_command: true,
        })
    }

    /// The guest somebody on a messenger's side is here: the one they were
    /// bound to, else a new guest named after them, bound now.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub fn guest_of_identity(
        &mut self,
        channel_id: &str,
        external_id: &str,
        name: &str,
        direct_chat: Option<&str>,
        may_speak: bool,
        bot: bool,
    ) -> Result<Participant, RoutingError> {
        if let Some(known) = self.participant_of_identity(channel_id, external_id)? {
            if let Some(direct_chat) = direct_chat {
                self.connection.execute(
                    "UPDATE identities SET direct_chat = ?3
                     WHERE channel_id = ?1 AND external_id = ?2",
                    params![channel_id, external_id, direct_chat],
                )?;
            }
            return Ok(known);
        }
        let name = if name.trim().is_empty() {
            "Someone"
        } else {
            name.trim()
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let guest = new_id("p")?;
        transaction.execute(
            "INSERT INTO participants(participant_id, kind, handle, name, created_ms)
             VALUES (?1, 'guest', ?2, ?3, ?4)",
            params![
                guest,
                free_handle(&transaction, &handle_from(name))?,
                name,
                now_ms()
            ],
        )?;
        transaction.execute(
            "INSERT INTO identities(channel_id, external_id, participant_id, name, bound_ms, direct_chat, may_speak, bot)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![channel_id, external_id, guest, name, now_ms(), direct_chat, i32::from(may_speak), i32::from(bot)],
        )?;
        transaction.commit()?;
        self.participant(&guest)
    }

    /// How many turns agents have taken in a chat since a person last said
    /// something there: what a bot's words are answered against, so that
    /// two bots do not keep each other's agents talking for ever.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn agent_turns_since_a_person(&self, chat_id: &str) -> Result<usize, RoutingError> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM messages
             WHERE chat_id = ?1 AND channel = ?2 AND sequence > COALESCE(
               (SELECT MAX(m.sequence) FROM messages m
                JOIN participants p ON p.participant_id = m.sender_id
                WHERE m.chat_id = ?1 AND (p.kind = 'person' OR (p.kind = 'guest'
                  AND NOT EXISTS (SELECT 1 FROM identities i
                                  WHERE i.participant_id = m.sender_id AND i.bot = 1)))),
               0)",
            params![chat_id, CHANNEL_AGENT],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// The chat here that a chat on a messenger's side is, if it was bound.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn chat_of_channel_chat(
        &self,
        channel_id: &str,
        external_chat: &str,
    ) -> Result<Option<String>, RoutingError> {
        self.connection
            .query_row(
                "SELECT chat_id FROM channel_chats WHERE channel_id = ?1 AND external_chat = ?2",
                params![channel_id, external_chat],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
    }

    /// Where a chat here is on messengers' sides.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn channel_chats_of(&self, chat_id: &str) -> Result<Vec<ChannelChat>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT channel_id, external_chat, chat_id, bound_ms
             FROM channel_chats WHERE chat_id = ?1 ORDER BY bound_ms",
        )?;
        let rows = statement.query_map([chat_id], |row| {
            Ok(ChannelChat {
                channel_id: row.get(0)?,
                external_chat: row.get(1)?,
                chat_id: row.get(2)?,
                bound_ms: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Every chat a channel is bound to.
    ///
    /// # Errors
    ///
    /// The ledger could not be read.
    pub fn chats_of_channel(&self, channel_id: &str) -> Result<Vec<ChannelChat>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT channel_id, external_chat, chat_id, bound_ms
             FROM channel_chats WHERE channel_id = ?1 ORDER BY bound_ms",
        )?;
        let rows = statement.query_map([channel_id], |row| {
            Ok(ChannelChat {
                channel_id: row.get(0)?,
                external_chat: row.get(1)?,
                chat_id: row.get(2)?,
                bound_ms: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Bind a chat on a messenger's side to a chat here.
    ///
    /// # Errors
    ///
    /// The chat is unknown, or the ledger could not be written.
    pub fn bind_channel_chat(
        &mut self,
        channel_id: &str,
        external_chat: &str,
        chat_id: &str,
    ) -> Result<ChannelChat, RoutingError> {
        self.chat(chat_id)?;
        let bound_ms = now_ms();
        self.connection.execute(
            "INSERT INTO channel_chats(channel_id, external_chat, chat_id, bound_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(channel_id, external_chat)
             DO UPDATE SET chat_id = excluded.chat_id, bound_ms = excluded.bound_ms",
            params![channel_id, external_chat, chat_id, bound_ms],
        )?;
        Ok(ChannelChat {
            channel_id: channel_id.to_owned(),
            external_chat: external_chat.to_owned(),
            chat_id: chat_id.to_owned(),
            bound_ms,
        })
    }

    /// Unbind a chat on a messenger's side from whatever chat here it was.
    ///
    /// # Errors
    ///
    /// The ledger could not be written.
    pub fn unbind_channel_chat(
        &mut self,
        channel_id: &str,
        external_chat: &str,
    ) -> Result<(), RoutingError> {
        self.connection.execute(
            "DELETE FROM channel_chats WHERE channel_id = ?1 AND external_chat = ?2",
            params![channel_id, external_chat],
        )?;
        Ok(())
    }

    /// Bring a participant into a chat. One who is in it already stays as
    /// they were; one who had left is in it again.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown chat or participant.
    pub fn join_chat(&mut self, chat_id: &str, participant_id: &str) -> Result<Chat, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let joined = transaction.execute(
            "INSERT INTO chat_members(chat_id, participant_id, joined_ms) VALUES (?1, ?2, ?3)
             ON CONFLICT(chat_id, participant_id) DO UPDATE SET left_ms = NULL
             WHERE left_ms IS NOT NULL",
            params![chat_id, participant_id, now_ms()],
        )?;
        if joined > 0 {
            chat_event_in(
                &transaction,
                chat_id,
                "chat/joined",
                &json!({ "participant_id": participant_id }),
            )?;
        }
        transaction.commit()?;
        self.chat(chat_id)
    }

    /// Take a participant out of a chat. What they said stays; what they
    /// were owed and had not begun is not begun. The one who started the
    /// chat stays in it.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for a chat that does not
    /// exist, for somebody who is not in it, and for the one who started it.
    pub fn leave_chat(
        &mut self,
        chat_id: &str,
        participant_id: &str,
    ) -> Result<Chat, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let started_by: Option<String> = transaction
            .query_row(
                "SELECT created_by FROM chats WHERE chat_id = ?1",
                [chat_id],
                |row| row.get(0),
            )
            .optional()?;
        match started_by {
            None => {
                return Err(RoutingError::InvalidBinding(format!(
                    "there is no chat {chat_id}"
                )));
            }
            Some(started_by) if started_by == participant_id => {
                return Err(RoutingError::InvalidBinding(
                    "whoever started a chat stays in it".into(),
                ));
            }
            Some(_) => {}
        }
        let left = transaction.execute(
            "UPDATE chat_members SET left_ms = ?3
             WHERE chat_id = ?1 AND participant_id = ?2 AND left_ms IS NULL",
            params![chat_id, participant_id, now_ms()],
        )?;
        if left == 0 {
            return Err(RoutingError::InvalidBinding(
                "nobody is in the chat by that id".into(),
            ));
        }
        transaction.execute(
            "UPDATE deliveries SET state = 'stopped', ended_ms = ?3,
                                   outcome = 'it was taken out of the chat'
             WHERE chat_id = ?1 AND agent_id = ?2 AND state = 'queued'",
            params![chat_id, participant_id, now_ms()],
        )?;
        chat_event_in(
            &transaction,
            chat_id,
            "chat/left",
            &json!({ "participant_id": participant_id }),
        )?;
        transaction.commit()?;
        self.chat(chat_id)
    }

    /// Retire an agent: it is in no list of who can be written to, what
    /// waited for it is not begun, and its profile and its handle are let
    /// go of, so that a new agent can be given either. What it said stays
    /// in its chats under its name.
    ///
    /// # Errors
    ///
    /// Refuses somebody who is not an agent, and one retired already.
    pub fn retire_agent(&mut self, participant_id: &str) -> Result<Participant, RoutingError> {
        let agent = self.participant(participant_id)?;
        if agent.kind != ParticipantKind::Agent || agent.retired {
            return Err(RoutingError::InvalidBinding(format!(
                "{} is not an agent that can be removed",
                agent.name
            )));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let kept: String = agent.handle.chars().take(24).collect();
        let handle = free_handle(&transaction, &format!("{kept}-removed"))?;
        transaction.execute(
            "UPDATE participants SET retired_ms = ?2, profile_id = NULL, handle = ?3
             WHERE participant_id = ?1",
            params![participant_id, now_ms(), handle],
        )?;
        transaction.execute(
            "UPDATE deliveries SET state = 'stopped', ended_ms = ?2,
                                   outcome = 'its agent was removed'
             WHERE agent_id = ?1 AND state = 'queued'",
            params![participant_id, now_ms()],
        )?;
        let chats: Vec<String> = {
            let mut statement = transaction
                .prepare("SELECT chat_id FROM chat_members WHERE participant_id = ?1")?;
            statement
                .query_map([participant_id], |row| row.get(0))?
                .collect::<Result<_, _>>()?
        };
        for chat_id in chats {
            chat_event_in(
                &transaction,
                &chat_id,
                "chat/retired",
                &json!({ "participant_id": participant_id }),
            )?;
        }
        transaction.commit()?;
        self.participant(participant_id)
    }

    /// Set a chat's rules: whether agents answer only when they are named
    /// (`named`) or whatever a person says (`always`), and after how many
    /// replies of agents to each other the chain waits for a person. What
    /// is left out stays.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for a chat that does not
    /// exist, a rule there is not, and a limit outside one to twenty.
    pub fn rule_chat(
        &mut self,
        chat_id: &str,
        answer_rule: Option<&str>,
        reply_limit: Option<u32>,
    ) -> Result<Chat, RoutingError> {
        if let Some(rule) = answer_rule
            && !matches!(rule, "named" | "always")
        {
            return Err(RoutingError::InvalidBinding(format!(
                "agents answer when named or always, not {rule:?}"
            )));
        }
        if let Some(limit) = reply_limit
            && !(1..=20).contains(&limit)
        {
            return Err(RoutingError::InvalidBinding(
                "a chain waits for a person after one to twenty replies".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let ruled = transaction.execute(
            "UPDATE chats SET answer_rule = COALESCE(?2, answer_rule),
                              reply_limit = COALESCE(?3, reply_limit)
             WHERE chat_id = ?1",
            params![chat_id, answer_rule, reply_limit],
        )?;
        if ruled == 0 {
            return Err(RoutingError::InvalidBinding(format!(
                "there is no chat {chat_id}"
            )));
        }
        chat_event_in(
            &transaction,
            chat_id,
            "chat/ruled",
            &json!({ "answer_rule": answer_rule, "reply_limit": reply_limit }),
        )?;
        transaction.commit()?;
        self.chat(chat_id)
    }

    /// One participant by id.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for an id nobody has.
    pub fn participant(&self, participant_id: &str) -> Result<Participant, RoutingError> {
        self.participants_where("WHERE participant_id = ?1", &[&participant_id])?
            .into_iter()
            .next()
            .ok_or_else(|| RoutingError::InvalidBinding(format!("nobody is {participant_id}")))
    }

    /// Everybody the ledger knows, people first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn participants(&self) -> Result<Vec<Participant>, RoutingError> {
        self.participants_where(
            "ORDER BY CASE kind WHEN 'person' THEN 0 WHEN 'agent' THEN 1 ELSE 2 END,
                      created_ms, participant_id",
            &[],
        )
    }

    /// Change what a participant is called.
    ///
    /// # Errors
    ///
    /// Refuses a handle a chat cannot carry and one somebody else has.
    pub fn name_participant(
        &mut self,
        participant_id: &str,
        name: Option<&str>,
        handle: Option<&str>,
        colour: Option<&str>,
    ) -> Result<Participant, RoutingError> {
        if let Some(handle) = handle {
            if !is_handle(handle) {
                return Err(RoutingError::InvalidBinding(format!(
                    "a handle is lower-case letters, digits, dashes and underscores, thirty-two at most, and begins with a letter or a digit: {handle}"
                )));
            }
            let taken: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM participants
                               WHERE handle = ?1 AND participant_id != ?2)",
                params![handle, participant_id],
                |row| row.get(0),
            )?;
            if taken {
                return Err(RoutingError::InvalidBinding(format!(
                    "@{handle} is somebody else's handle"
                )));
            }
        }
        if let Some(name) = name
            && (name.trim().is_empty()
                || name.chars().count() > 96
                || name.chars().any(char::is_control))
        {
            return Err(RoutingError::InvalidBinding(
                "a name is one line of at most ninety-six characters".into(),
            ));
        }
        self.connection.execute(
            "UPDATE participants SET name = COALESCE(?2, name), handle = COALESCE(?3, handle),
                                     colour = COALESCE(?4, colour)
             WHERE participant_id = ?1",
            params![participant_id, name.map(str::trim), handle, colour],
        )?;
        self.participant(participant_id)
    }

    /// Start a chat between the one who starts it and those they name.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a member nobody is.
    pub fn start_chat(
        &mut self,
        title: &str,
        created_by: &str,
        members: &[String],
    ) -> Result<Chat, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut everyone: Vec<&str> = vec![created_by];
        for member in members {
            if !everyone.contains(&member.as_str()) {
                everyone.push(member);
            }
        }
        let chat = chat_in(
            &transaction,
            title.trim(),
            created_by,
            &everyone,
            Some(now_ms()),
        )?;
        chat_event_in(
            &transaction,
            &chat,
            "chat/started",
            &json!({ "created_by": created_by, "members": everyone }),
        )?;
        transaction.commit()?;
        self.chat(&chat)
    }

    /// Call a chat something else. Called nothing, it is known by its first
    /// words again.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for a chat that does not
    /// exist and for a name that is not one line.
    pub fn rename_chat(&mut self, chat_id: &str, title: &str) -> Result<Chat, RoutingError> {
        let title = title.trim();
        if title.chars().count() > 160 || title.chars().any(char::is_control) {
            return Err(RoutingError::InvalidBinding(
                "a chat's name is one line of at most a hundred and sixty characters".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let renamed = transaction.execute(
            "UPDATE chats SET title = ?2 WHERE chat_id = ?1",
            params![chat_id, title],
        )?;
        if renamed == 0 {
            return Err(RoutingError::InvalidBinding(format!(
                "there is no chat {chat_id}"
            )));
        }
        chat_event_in(
            &transaction,
            chat_id,
            "chat/renamed",
            &json!({ "title": title }),
        )?;
        transaction.commit()?;
        self.chat(chat_id)
    }

    /// The chat a route belongs to; a route that has none gets one, of the
    /// owner and the route's agent.
    pub(crate) fn chat_of_route(
        &mut self,
        binding: &SessionRouteBinding,
    ) -> Result<String, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let chat = if let Some(chat) = transaction
            .query_row(
                "SELECT chat_id FROM sessions WHERE route_id = ?1",
                [&binding.route_id],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            chat
        } else {
            let owner = owner_in(&transaction)?;
            let agent = agent_in(&transaction, &binding.agent_profile_id)?;
            let chat = chat_in(&transaction, "", &owner, &[&owner, &agent], Some(now_ms()))?;
            session_in(
                &transaction,
                &chat,
                &agent,
                &binding.route_id,
                Some(now_ms()),
                "",
            )?;
            chat
        };
        transaction.commit()?;
        Ok(chat)
    }

    /// Make a route the agent's current session in a chat. The one it had
    /// there, if any, is kept as an earlier one, with why it ended.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown chat, agent or route, and for
    /// a route that is already a session somewhere.
    pub fn begin_session(
        &mut self,
        chat_id: &str,
        agent_id: &str,
        route_id: &str,
        why_the_last_ended: &str,
    ) -> Result<ChatSession, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT OR IGNORE INTO chat_members(chat_id, participant_id, joined_ms)
             VALUES (?1, ?2, ?3)",
            params![chat_id, agent_id, now_ms()],
        )?;
        let session = session_in(
            &transaction,
            chat_id,
            agent_id,
            route_id,
            Some(now_ms()),
            why_the_last_ended,
        )?;
        transaction.commit()?;
        Ok(ChatSession {
            session_id: session,
            chat_id: chat_id.to_owned(),
            agent_id: agent_id.to_owned(),
            route_id: route_id.to_owned(),
            current: true,
        })
    }

    /// The session an agent is in, in a chat, if it has one.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn current_session(
        &self,
        chat_id: &str,
        agent_id: &str,
    ) -> Result<Option<ChatSession>, RoutingError> {
        Ok(self
            .connection
            .query_row(
                "SELECT session_id, route_id FROM sessions
                 WHERE chat_id = ?1 AND agent_id = ?2 AND state = 'current'",
                params![chat_id, agent_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .map(|(session_id, route_id)| ChatSession {
                session_id,
                chat_id: chat_id.to_owned(),
                agent_id: agent_id.to_owned(),
                route_id,
                current: true,
            }))
    }

    /// The chat and the agent a route is a session of.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn session_of_route(&self, route_id: &str) -> Result<Option<ChatSession>, RoutingError> {
        Ok(self
            .connection
            .query_row(
                "SELECT session_id, chat_id, agent_id, state FROM sessions WHERE route_id = ?1",
                [route_id],
                |row| {
                    Ok(ChatSession {
                        session_id: row.get(0)?,
                        chat_id: row.get(1)?,
                        agent_id: row.get(2)?,
                        route_id: route_id.to_owned(),
                        current: row.get::<_, String>(3)? == "current",
                    })
                },
            )
            .optional()?)
    }

    /// One chat, with who is in it.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for a chat that does not exist.
    pub fn chat(&self, chat_id: &str) -> Result<Chat, RoutingError> {
        self.chats_where("WHERE c.chat_id = ?1", &[&chat_id])?
            .into_iter()
            .next()
            .ok_or_else(|| RoutingError::InvalidBinding(format!("there is no chat {chat_id}")))
    }

    /// The chats a participant is in, the one that moved last first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn chats_of(&self, participant_id: &str) -> Result<Vec<Chat>, RoutingError> {
        let mut chats = self.chats_where(
            "WHERE c.chat_id IN (SELECT chat_id FROM chat_members
                                 WHERE participant_id = ?1 AND left_ms IS NULL)",
            &[&participant_id],
        )?;
        chats.sort_by(|left, right| {
            right
                .last_sequence
                .cmp(&left.last_sequence)
                .then_with(|| right.chat_id.cmp(&left.chat_id))
        });
        Ok(chats)
    }

    fn chats_where(
        &self,
        condition: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Chat>, RoutingError> {
        type Row = (
            String,
            String,
            String,
            String,
            i64,
            i64,
            Option<i64>,
            Option<i64>,
            Option<String>,
        );
        let mut statement = self.connection.prepare(&format!(
            "SELECT c.chat_id, c.title, c.created_by, c.answer_rule, c.reply_limit, c.agent_replies,
                    (SELECT MAX(e.sequence) FROM events e WHERE e.chat_id = c.chat_id),
                    (SELECT e.at_ms FROM events e WHERE e.chat_id = c.chat_id
                     ORDER BY e.sequence DESC LIMIT 1),
                    (SELECT m.text FROM messages m WHERE m.chat_id = c.chat_id
                     ORDER BY m.sequence ASC LIMIT 1)
             FROM chats c {condition}"
        ))?;
        let rows = statement.query_map(values, |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })?;
        let rows: Vec<Row> = rows.collect::<Result<_, _>>()?;
        let mut chats = Vec::new();
        for (chat_id, title, created_by, answer_rule, limit, replies, last, at, first) in rows {
            let members = self.participants_where(
                "WHERE participant_id IN (SELECT participant_id FROM chat_members
                                          WHERE chat_id = ?1 AND left_ms IS NULL)
                 ORDER BY CASE kind WHEN 'person' THEN 0 WHEN 'agent' THEN 1 ELSE 2 END,
                          created_ms, participant_id",
                &[&chat_id],
            )?;
            chats.push(Chat {
                title: if title.is_empty() {
                    first.as_deref().map(opening_words).unwrap_or_default()
                } else {
                    title
                },
                chat_id,
                created_by,
                answer_rule,
                reply_limit: u32::try_from(limit).unwrap_or(4),
                agent_replies: u32::try_from(replies).unwrap_or(0),
                members,
                last_sequence: last.and_then(|last| u64::try_from(last).ok()),
                last_at_ms: at.and_then(|at| u64::try_from(at).ok()),
            });
        }
        Ok(chats)
    }

    /// Say something in a chat. The message takes its place in the one order
    /// by an event of its own. A message a channel has already brought, by
    /// the channel's own reference, is the same message and is not said twice.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown chat or a sender who is not in
    /// it.
    pub fn say(
        &mut self,
        chat_id: &str,
        sender_id: &str,
        channel: &str,
        channel_ref: Option<&str>,
        content: &Value,
    ) -> Result<Message, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let member: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM chat_members
                           WHERE chat_id = ?1 AND participant_id = ?2 AND left_ms IS NULL)",
            params![chat_id, sender_id],
            |row| row.get(0),
        )?;
        if !member {
            return Err(RoutingError::InvalidBinding(format!(
                "{sender_id} is not in chat {chat_id}"
            )));
        }
        if let Some(reference) = channel_ref
            && let Some(said) = transaction
                .query_row(
                    "SELECT message_id FROM messages
                     WHERE chat_id = ?1 AND channel = ?2 AND channel_ref = ?3",
                    params![chat_id, channel, reference],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
        {
            transaction.commit()?;
            return self.message(&said);
        }
        // A person has written: agents may answer each other again.
        transaction.execute(
            "UPDATE chats SET agent_replies = 0
             WHERE chat_id = ?1 AND NOT EXISTS (
               SELECT 1 FROM participants WHERE participant_id = ?2 AND kind = 'agent')",
            params![chat_id, sender_id],
        )?;
        let now = now_ms();
        let event_id = new_id("e")?;
        transaction.execute(
            "INSERT INTO events(route_id, chat_id, event_id, kind, source, payload_json, at_ms)
             VALUES (NULL, ?1, ?2, 'chat/message', ?3, '{}', ?4)",
            params![
                chat_id,
                event_id,
                serde_json::to_string(&SurfaceEventSource::Host)?,
                now
            ],
        )?;
        let sequence = transaction.last_insert_rowid();
        let message = message_in(
            &transaction,
            &Said {
                chat_id,
                sender_id,
                channel,
                channel_ref,
                content,
                sequence,
                created_ms: Some(now),
            },
        )?;
        transaction.execute(
            "UPDATE events SET payload_json = ?2 WHERE sequence = ?1",
            params![
                sequence,
                serde_json::to_string(&json!({ "message_id": message, "sender_id": sender_id }))?
            ],
        )?;
        transaction.commit()?;
        self.message(&message)
    }

    /// One message by id.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for a message that does not exist.
    pub fn message(&self, message_id: &str) -> Result<Message, RoutingError> {
        self.messages_where("WHERE message_id = ?1", &[&message_id])?
            .into_iter()
            .next()
            .ok_or_else(|| {
                RoutingError::InvalidBinding(format!("there is no message {message_id}"))
            })
    }

    fn messages_where(
        &self,
        condition: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Message>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {MESSAGE_COLUMNS} FROM messages {condition}"
        ))?;
        let rows = statement.query_map(values, message_of)?;
        let mut messages = Vec::new();
        for row in rows {
            let (mut message, content, named) = row?;
            message.content = serde_json::from_str(&content)?;
            message.named = serde_json::from_str(&named)?;
            messages.push(message);
        }
        Ok(messages)
    }

    /// What was said in a chat after a place, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn said_after(
        &self,
        chat_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<Message>, RoutingError> {
        let after = i64::try_from(after).map_err(|_| RoutingError::CursorOverflow(after))?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.messages_where(
            "WHERE chat_id = ?1 AND sequence > ?2 ORDER BY sequence ASC LIMIT ?3",
            &[&chat_id, &after, &limit],
        )
    }

    /// A page of a chat, ending before a place (or at its end): the events
    /// and the messages of that stretch, oldest first, and whether there is
    /// more before it.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a chat that does not exist.
    pub fn timeline(
        &self,
        chat_id: &str,
        before: Option<u64>,
        limit: usize,
    ) -> Result<(Vec<ChatEvent>, Vec<Message>, bool), RoutingError> {
        self.chat(chat_id)?;
        let before = match before {
            Some(before) => {
                i64::try_from(before).map_err(|_| RoutingError::CursorOverflow(before))?
            }
            None => i64::MAX,
        };
        let limit = limit.max(1);
        let wanted = i64::try_from(limit).unwrap_or(i64::MAX).saturating_add(1);
        let mut events = self.events_where(
            "WHERE e.chat_id = ?1 AND e.sequence < ?2 ORDER BY e.sequence DESC LIMIT ?3",
            &[&chat_id, &before, &wanted],
        )?;
        let more = events.len() > limit;
        events.truncate(limit);
        events.reverse();
        let from = events.first().map_or(0, |event| event.sequence);
        let from = i64::try_from(from).unwrap_or(0);
        let messages = self.messages_where(
            "WHERE chat_id = ?1 AND sequence >= ?2 AND sequence < ?3 ORDER BY sequence ASC",
            &[&chat_id, &from, &before],
        )?;
        Ok((events, messages, more))
    }

    /// What happened in one chat after a place, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn happened_in_chat_after(
        &self,
        chat_id: &str,
        after: u64,
        limit: usize,
    ) -> Result<Vec<ChatEvent>, RoutingError> {
        let after = i64::try_from(after).map_err(|_| RoutingError::CursorOverflow(after))?;
        let limit = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        self.events_where(
            "WHERE e.chat_id = ?1 AND e.sequence > ?2 ORDER BY e.sequence ASC LIMIT ?3",
            &[&chat_id, &after, &limit],
        )
    }

    /// Everything that happened after a place, in every chat, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn happened_after(&self, after: u64, limit: usize) -> Result<Vec<ChatEvent>, RoutingError> {
        let after = i64::try_from(after).map_err(|_| RoutingError::CursorOverflow(after))?;
        let limit = i64::try_from(limit.max(1)).unwrap_or(i64::MAX);
        self.events_where(
            "WHERE e.sequence > ?1 ORDER BY e.sequence ASC LIMIT ?2",
            &[&after, &limit],
        )
    }

    /// Everything that happened after a place, each with the message it
    /// is the place of, when it is the place of one.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn happened_with_what_was_said(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<(ChatEvent, Option<Message>)>, RoutingError> {
        let events = self.happened_after(after, limit)?;
        let Some(last) = events.last() else {
            return Ok(Vec::new());
        };
        let from = i64::try_from(after).map_err(|_| RoutingError::CursorOverflow(after))?;
        let through = i64::try_from(last.sequence)
            .map_err(|_| RoutingError::CursorOverflow(last.sequence))?;
        let mut said: BTreeMap<u64, Message> = self
            .messages_where("WHERE sequence > ?1 AND sequence <= ?2", &[&from, &through])?
            .into_iter()
            .map(|message| (message.sequence, message))
            .collect();
        Ok(events
            .into_iter()
            .map(|event| {
                let message = said.remove(&event.sequence);
                (event, message)
            })
            .collect())
    }

    /// Where the record ends.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn head(&self) -> Result<u64, RoutingError> {
        let head: i64 = self.connection.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events",
            [],
            |row| row.get(0),
        )?;
        Ok(u64::try_from(head).unwrap_or(0))
    }

    fn events_where(
        &self,
        condition: &str,
        values: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<ChatEvent>, RoutingError> {
        type Row = (
            i64,
            Option<String>,
            Option<String>,
            String,
            String,
            String,
            Option<i64>,
        );
        let mut statement = self.connection.prepare(&format!(
            "SELECT e.sequence, e.chat_id, s.agent_id, e.kind, e.source, e.payload_json, e.at_ms
             FROM events e LEFT JOIN sessions s ON s.route_id = e.route_id {condition}"
        ))?;
        let rows = statement.query_map(values, |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            ))
        })?;
        let rows: Vec<Row> = rows.collect::<Result<_, _>>()?;
        let mut events = Vec::new();
        for (sequence, chat_id, agent_id, kind, source, payload, at_ms) in rows {
            events.push(ChatEvent {
                sequence: u64::try_from(sequence).unwrap_or(0),
                chat_id,
                agent_id,
                kind,
                source: serde_json::from_str(&source)?,
                payload: serde_json::from_str(&payload)?,
                at_ms: at_ms.and_then(|at| u64::try_from(at).ok()),
            });
        }
        Ok(events)
    }

    /// How far an agent has been given a chat; moved there when a place is
    /// named, never back.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn given_through(
        &mut self,
        chat_id: &str,
        participant_id: &str,
        through: Option<u64>,
    ) -> Result<u64, RoutingError> {
        if let Some(through) = through {
            let through =
                i64::try_from(through).map_err(|_| RoutingError::CursorOverflow(through))?;
            self.connection.execute(
                "UPDATE chat_members SET given_through = MAX(given_through, ?3)
                 WHERE chat_id = ?1 AND participant_id = ?2",
                params![chat_id, participant_id, through],
            )?;
        }
        let given: Option<i64> = self
            .connection
            .query_row(
                "SELECT given_through FROM chat_members
                 WHERE chat_id = ?1 AND participant_id = ?2",
                params![chat_id, participant_id],
                |row| row.get(0),
            )
            .optional()?;
        Ok(given
            .and_then(|given| u64::try_from(given).ok())
            .unwrap_or(0))
    }

    /// The agents of a chat, by handle.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a chat that does not exist.
    pub fn agents_in(&self, chat_id: &str) -> Result<BTreeMap<String, Participant>, RoutingError> {
        Ok(self
            .chat(chat_id)?
            .members
            .into_iter()
            .filter(|member| member.kind == ParticipantKind::Agent && !member.retired)
            .map(|member| (member.handle.clone(), member))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{handle_from, handles_named, is_handle, new_id};

    #[test]
    fn an_id_is_its_prefix_and_twenty_six_characters_and_two_differ() {
        let first = new_id("p").expect("an id");
        let second = new_id("p").expect("an id");
        assert!(first.starts_with("p_"));
        assert_eq!(first.len(), 28);
        assert_ne!(first, second);
        assert!(
            first[2..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
        );
    }

    #[test]
    fn a_handle_is_made_from_a_name_somebody_has() {
        assert_eq!(handle_from("claude-code"), "claude-code");
        assert_eq!(handle_from("Ada Lovelace"), "ada-lovelace");
        assert_eq!(handle_from("my.agent_2"), "my-agent_2");
        assert_eq!(handle_from("  "), "someone");
        assert!(is_handle(&handle_from("Строитель 7")));
    }

    #[test]
    fn a_handle_in_code_names_nobody() {
        assert_eq!(
            handles_named("@ada look, and @builder too. again @ada"),
            vec!["ada".to_owned(), "builder".to_owned()]
        );
        assert_eq!(
            handles_named("mail me at someone@example.com"),
            Vec::<String>::new()
        );
        assert_eq!(handles_named("run `@ada` here"), Vec::<String>::new());
        assert_eq!(
            handles_named("```\n@ada\n```\nbut @scout, yes"),
            vec!["scout".to_owned()]
        );
        assert_eq!(handles_named("@Ada."), vec!["ada".to_owned()]);
    }
}
