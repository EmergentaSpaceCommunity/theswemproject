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

/// The channel a message came through when it was typed into the Workbench.
pub const CHANNEL_WORKBENCH: &str = "workbench";
/// The channel of a message an editor sent through the editor door.
pub const CHANNEL_EDITOR: &str = "editor";
/// The channel of a message that arrived on time.
pub const CHANNEL_SCHEDULE: &str = "schedule";
/// The channel of what an agent said in its own turn.
pub const CHANNEL_AGENT: &str = "agent";

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

const MESSAGE_COLUMNS: &str =
    "message_id, sequence, chat_id, sender_id, channel, content_json, text, named_json, created_ms";

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
