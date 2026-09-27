//! Durable state owned by the meta-harness rather than by an ACP agent.
//!
//! The ledger records only routing bindings, projected events and per-surface
//! acknowledgement cursors. It is never used to rebuild an agent conversation:
//! native recovery remains `session/load` or `session/resume`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::{NativeSessionEvent, SurfaceEventSource};

const SCHEMA_VERSION: i64 = 2;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentTransport {
    Stdio,
    Http,
    Sse,
}

/// One session of one agent profile, as a person picks it from a list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSummary {
    pub route_id: String,
    pub agent_id: String,
    /// The lane's last sequence, absent when nothing was ever recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sequence: Option<u64>,
    /// What the lane last carried, for a sentence a person can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_kind: Option<String>,
    pub events: u64,
    /// The first thing said on this lane, which is how a person recognises a
    /// conversation. A route id is not: every surface that lists sessions was
    /// showing `route-6919-1789833696034060189` and calling it a name. Absent
    /// on a lane nobody has written to yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_with: Option<String>,
}

/// How much of the first turn a surface shows as a conversation's name.
const OPENING_WORDS: usize = 72;

/// The first thing said on a lane, short enough to be a name.
///
/// The harness appends a provenance line to what a person wrote, so the words
/// they meant are the first line; the rest of the turn is the conversation,
/// not its name.
fn opening_words(turn_written: &str) -> Option<String> {
    let text = serde_json::from_str::<Value>(turn_written)
        .ok()?
        .get("text")?
        .as_str()?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?
        .to_owned();
    if text.chars().count() <= OPENING_WORDS {
        return Some(text);
    }
    let cut: String = text.chars().take(OPENING_WORDS).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// Stable, non-secret identity of an MCP attachment configuration.
///
/// Commands, arguments, URLs, headers and environment values intentionally do
/// not live here. `profile_id` resolves those values from the active connection
/// profile when the session is reconnected.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AttachmentBinding {
    pub profile_id: String,
    pub server_name: String,
    pub transport: AttachmentTransport,
}

impl AttachmentBinding {
    #[must_use]
    pub fn new(
        profile_id: impl Into<String>,
        server_name: impl Into<String>,
        transport: AttachmentTransport,
    ) -> Self {
        Self {
            profile_id: profile_id.into(),
            server_name: server_name.into(),
            transport,
        }
    }
}

/// Exact host-owned information needed to route back to an agent-owned session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionRouteBinding {
    pub route_id: String,
    pub agent_id: String,
    pub agent_profile_id: String,
    pub native_session_id: String,
    pub environment_profile_id: String,
    pub workspace: PathBuf,
    pub attachments: Vec<AttachmentBinding>,
}

impl SessionRouteBinding {
    /// Construct a canonical binding. Attachment order is not identity, but
    /// duplicate ACP server names are rejected because reconnect would be
    /// ambiguous.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when an identity is empty, the workspace is not
    /// an existing absolute directory, or attachment identity is ambiguous.
    pub fn new(
        route_id: impl Into<String>,
        agent_id: impl Into<String>,
        agent_profile_id: impl Into<String>,
        native_session_id: impl Into<String>,
        environment_profile_id: impl Into<String>,
        workspace: &Path,
        mut attachments: Vec<AttachmentBinding>,
    ) -> Result<Self, RoutingError> {
        let route_id = route_id.into();
        let agent_id = agent_id.into();
        let agent_profile_id = agent_profile_id.into();
        let native_session_id = native_session_id.into();
        let environment_profile_id = environment_profile_id.into();
        for (field, value) in [
            ("route_id", route_id.as_str()),
            ("agent_id", agent_id.as_str()),
            ("agent_profile_id", agent_profile_id.as_str()),
            ("native_session_id", native_session_id.as_str()),
            ("environment_profile_id", environment_profile_id.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(RoutingError::InvalidBinding(format!(
                    "{field} must not be empty"
                )));
            }
        }
        if !workspace.is_absolute() || !workspace.is_dir() {
            return Err(RoutingError::InvalidBinding(format!(
                "workspace must be an existing absolute directory: {}",
                workspace.display()
            )));
        }
        let workspace = fs::canonicalize(workspace).map_err(|error| {
            RoutingError::InvalidBinding(format!(
                "canonicalize workspace {}: {error}",
                workspace.display()
            ))
        })?;
        for attachment in &attachments {
            if attachment.profile_id.trim().is_empty() || attachment.server_name.trim().is_empty() {
                return Err(RoutingError::InvalidBinding(
                    "attachment profile_id and server_name must not be empty".into(),
                ));
            }
        }
        attachments.sort();
        let mut names = BTreeSet::new();
        if let Some(duplicate) = attachments
            .iter()
            .find(|attachment| !names.insert(&attachment.server_name))
        {
            return Err(RoutingError::InvalidBinding(format!(
                "duplicate MCP server name: {}",
                duplicate.server_name
            )));
        }
        Ok(Self {
            route_id,
            agent_id,
            agent_profile_id,
            native_session_id,
            environment_profile_id,
            workspace,
            attachments,
        })
    }
}

/// What makes a route the route it is: whose it is and which native session
/// it leads to.
///
/// Where the agent works, where it runs and what it attaches are how a chat
/// began, not what it is. A person changes them and goes on talking; the
/// binding keeps what they were, and only these four are proved on a retry
/// or a return.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteIdentity {
    pub route_id: String,
    pub agent_id: String,
    pub agent_profile_id: String,
    pub native_session_id: String,
}

impl SessionRouteBinding {
    /// The part of this binding that is proved when a route is met again.
    #[must_use]
    pub fn identity(&self) -> RouteIdentity {
        RouteIdentity {
            route_id: self.route_id.clone(),
            agent_id: self.agent_id.clone(),
            agent_profile_id: self.agent_profile_id.clone(),
            native_session_id: self.native_session_id.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SurfaceEvent {
    pub sequence: u64,
    pub event_id: String,
    pub kind: String,
    pub source: SurfaceEventSource,
    /// A delivery projection may contain message content. It is evidence for a
    /// surface, never input to ACP recovery or prompt replay.
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SurfaceEventBatch {
    pub after_cursor: u64,
    pub events: Vec<SurfaceEvent>,
}

impl SurfaceEventBatch {
    #[must_use]
    pub fn next_cursor(&self) -> u64 {
        self.events
            .last()
            .map_or(self.after_cursor, |event| event.sequence)
    }
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("routing database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("routing serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("invalid routing binding: {0}")]
    InvalidBinding(String),
    #[error("unknown session route: {0}")]
    RouteNotFound(String),
    #[error("route binding drift for {route_id}")]
    BindingDrift { route_id: String },
    #[error("event id {event_id} was reused with different content on route {route_id}")]
    EventCollision { route_id: String, event_id: String },
    #[error("surface cursor regressed from {current} to {requested}")]
    CursorRegression { current: u64, requested: u64 },
    #[error("surface cursor {requested} is beyond route head {head}")]
    CursorBeyondHead { requested: u64, head: u64 },
    #[error("cursor value does not fit SQLite integer: {0}")]
    CursorOverflow(u64),
    #[error("native route projection failed: {0}")]
    Projection(String),
}

/// Background projection of one ACP connection into an already-bound route.
///
/// The connection namespace is caller-owned and must be unique for the route.
/// A retry with the same namespace is idempotent; reusing it for different
/// event content fails through the ledger's ordinary collision check.
pub struct NativeRouteProjection {
    task: tokio::task::JoinHandle<Result<u64, RoutingError>>,
}

pub(crate) struct NativeOutputProjectionRequest {
    pub projection: crate::NativeOutputProjection,
    acknowledged: tokio::sync::oneshot::Sender<()>,
}

impl NativeOutputProjectionRequest {
    pub fn acknowledge(self) {
        let _ = self.acknowledged.send(());
    }
}

impl NativeRouteProjection {
    /// Wait until the native session emits its terminal surface event.
    ///
    /// # Errors
    ///
    /// Returns the first durable projection error or a task join failure.
    pub async fn finish(self) -> Result<u64, RoutingError> {
        self.task
            .await
            .map_err(|error| RoutingError::Projection(error.to_string()))?
    }
}

/// Attach one previously claimed native surface-event lane to a durable route.
///
/// This is a projection only: events are never replayed into ACP. The route
/// must already be bound to the native session before this function is called.
///
/// # Errors
///
/// Returns an error when the route or namespace is empty. Claim the receiver
/// with [`NativeSessionControl::take_surface_events`](crate::NativeSessionControl::take_surface_events)
/// before launch so early handshake events are not lost.
pub fn project_native_session_events(
    ledger_path: &Path,
    route_id: impl Into<String>,
    connection_namespace: impl Into<String>,
    events: tokio::sync::mpsc::UnboundedReceiver<NativeSessionEvent>,
) -> Result<NativeRouteProjection, RoutingError> {
    project_native_session_events_inner(
        ledger_path,
        route_id.into(),
        connection_namespace.into(),
        events,
        None,
    )
}

pub(crate) fn project_native_session_events_with_output(
    ledger_path: &Path,
    route_id: impl Into<String>,
    connection_namespace: impl Into<String>,
    events: tokio::sync::mpsc::UnboundedReceiver<NativeSessionEvent>,
    output: tokio::sync::mpsc::UnboundedSender<NativeOutputProjectionRequest>,
) -> Result<NativeRouteProjection, RoutingError> {
    project_native_session_events_inner(
        ledger_path,
        route_id.into(),
        connection_namespace.into(),
        events,
        Some(output),
    )
}

fn project_native_session_events_inner(
    ledger_path: &Path,
    route_id: String,
    connection_namespace: String,
    mut events: tokio::sync::mpsc::UnboundedReceiver<NativeSessionEvent>,
    output: Option<tokio::sync::mpsc::UnboundedSender<NativeOutputProjectionRequest>>,
) -> Result<NativeRouteProjection, RoutingError> {
    if route_id.trim().is_empty() || connection_namespace.trim().is_empty() {
        return Err(RoutingError::InvalidBinding(
            "route id and connection namespace must not be empty".into(),
        ));
    }
    let ledger_path = ledger_path.to_path_buf();
    let task = tokio::task::spawn_blocking(move || {
        let mut ledger = RoutingLedger::open(&ledger_path)?;
        ledger.ensure_route(&route_id)?;
        let mut projected = 0_u64;
        while let Some(NativeSessionEvent {
            kind,
            source,
            payload,
            output_projection,
        }) = events.blocking_recv()
        {
            projected = projected.saturating_add(1);
            let event_id = format!("{connection_namespace}:{projected}");
            ledger.append_event(&route_id, &event_id, &kind, source, &payload)?;
            if let Some(output) = &output {
                for item in output_projection {
                    let (acknowledged, receipt) = tokio::sync::oneshot::channel();
                    // Workbench projection is optional observation. Losing its
                    // receiver must not terminate or mutate the native route.
                    if output
                        .send(NativeOutputProjectionRequest {
                            projection: item,
                            acknowledged,
                        })
                        .is_ok()
                    {
                        let _ = receipt.blocking_recv();
                    }
                }
            }
            if kind == "host/session_terminal" {
                break;
            }
        }
        Ok(projected)
    });
    Ok(NativeRouteProjection { task })
}

const ROUTES_SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS routes (
    route_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    agent_profile_id TEXT NOT NULL,
    native_session_id TEXT NOT NULL,
    environment_profile_id TEXT NOT NULL,
    workspace TEXT NOT NULL,
    attachments_json TEXT NOT NULL
  );
  CREATE TABLE IF NOT EXISTS surface_cursors (
    route_id TEXT NOT NULL REFERENCES routes(route_id),
    surface_id TEXT NOT NULL,
    cursor INTEGER NOT NULL,
    PRIMARY KEY(route_id, surface_id)
  );";

/// One sequence orders everything that happens: what an engine said on a
/// route, and what was said in a chat by anyone. An event of a route also
/// names its chat, so one cursor follows a chat whatever session it is in.
const EVENTS_SCHEMA: &str = "
  CREATE TABLE events (
    sequence INTEGER PRIMARY KEY,
    route_id TEXT REFERENCES routes(route_id),
    chat_id TEXT REFERENCES chats(chat_id),
    event_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    source TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    at_ms INTEGER,
    CHECK (route_id IS NOT NULL OR chat_id IS NOT NULL)
  );
  CREATE UNIQUE INDEX events_route_event
    ON events(route_id, event_id) WHERE route_id IS NOT NULL;
  CREATE UNIQUE INDEX events_chat_event
    ON events(chat_id, event_id) WHERE route_id IS NULL;
  CREATE INDEX events_route_sequence ON events(route_id, sequence);
  CREATE INDEX events_chat_sequence ON events(chat_id, sequence);";

/// `SQLite` implementation of the minimal host-owned routing ledger.
///
/// `SQLite` is an implementation choice, not a public interchange format. WAL,
/// foreign keys and FULL synchronous commits are enabled so independent host
/// processes observe committed route/event/cursor transitions after a crash.
pub struct RoutingLedger {
    pub(crate) connection: Connection,
}

impl RoutingLedger {
    /// Open or create a routing ledger.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for filesystem, `SQLite` or schema failures.
    pub fn open(path: &Path) -> Result<Self, RoutingError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                RoutingError::InvalidBinding(format!(
                    "create routing directory {}: {error}",
                    parent.display()
                ))
            })?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             CREATE TABLE IF NOT EXISTS ledger_meta (
               key TEXT PRIMARY KEY,
               value INTEGER NOT NULL
             );",
        )?;
        let mut ledger = Self { connection };
        match ledger.stored_version()? {
            Some(version) if version == SCHEMA_VERSION => {}
            None => ledger.create_schema()?,
            Some(1) => {
                crate::chat_ledger::keep_a_copy(&ledger.connection, path)?;
                ledger.migrate_from_routes()?;
            }
            Some(version) => {
                return Err(RoutingError::InvalidBinding(format!(
                    "unsupported routing schema version {version}"
                )));
            }
        }
        Ok(ledger)
    }

    fn stored_version(&self) -> Result<Option<i64>, RoutingError> {
        Ok(self
            .connection
            .query_row(
                "SELECT value FROM ledger_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// A ledger nobody has written to yet.
    fn create_schema(&mut self) -> Result<(), RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: Option<i64> = transaction
            .query_row(
                "SELECT value FROM ledger_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if stored.is_none() {
            transaction.execute_batch(ROUTES_SCHEMA)?;
            transaction.execute_batch(crate::chat_ledger::CHATS_SCHEMA)?;
            transaction.execute_batch(EVENTS_SCHEMA)?;
            transaction.execute_batch(crate::chat_ledger::MESSAGES_SCHEMA)?;
            transaction.execute(
                "INSERT INTO ledger_meta(key, value) VALUES ('schema_version', ?1)",
                [SCHEMA_VERSION],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// A ledger of routes becomes a ledger of chats: every route a chat of the
    /// owner and that route's agent, with what was said read out of what was
    /// recorded. One transaction; the file was copied beside itself first.
    fn migrate_from_routes(&mut self) -> Result<(), RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let stored: i64 = transaction.query_row(
            "SELECT value FROM ledger_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )?;
        // Another process may have done it between the read and the lock.
        if stored == 1 {
            transaction.execute_batch(crate::chat_ledger::CHATS_SCHEMA)?;
            transaction.execute_batch(
                "ALTER TABLE events RENAME TO events_v1;
                 DROP INDEX IF EXISTS events_route_sequence;",
            )?;
            transaction.execute_batch(EVENTS_SCHEMA)?;
            transaction.execute_batch(
                "INSERT INTO events(sequence, route_id, event_id, kind, source, payload_json)
                   SELECT sequence, route_id, event_id, kind, source, payload_json
                   FROM events_v1 ORDER BY sequence;
                 DROP TABLE events_v1;",
            )?;
            transaction.execute_batch(crate::chat_ledger::MESSAGES_SCHEMA)?;
            crate::chat_ledger::chats_from_routes(&transaction)?;
            transaction.execute(
                "UPDATE ledger_meta SET value = ?1 WHERE key = 'schema_version'",
                [SCHEMA_VERSION],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    /// Insert a route binding, or prove a retry names the same route.
    ///
    /// The row that is kept is the first one: it is how the chat began. A
    /// later bind with another workspace, environment or attachments is the
    /// same route met again after its agent was changed, and passes.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::BindingDrift`] if the route id already names
    /// another agent, profile or native session.
    pub fn bind_route(&mut self, binding: &SessionRouteBinding) -> Result<(), RoutingError> {
        let attachments = serde_json::to_string(&binding.attachments)?;
        let workspace = binding.workspace.to_str().ok_or_else(|| {
            RoutingError::InvalidBinding("workspace is not valid Unicode for ACP/SQLite".into())
        })?;
        self.connection.execute(
            "INSERT OR IGNORE INTO routes(
               route_id, agent_id, agent_profile_id, native_session_id,
               environment_profile_id, workspace, attachments_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                binding.route_id,
                binding.agent_id,
                binding.agent_profile_id,
                binding.native_session_id,
                binding.environment_profile_id,
                workspace,
                attachments,
            ],
        )?;
        self.require_identity(&binding.identity())?;
        self.chat_of_route(binding)?;
        Ok(())
    }

    /// Bind a route as its agent's session in a chat that already exists.
    /// The session the agent had there, if any, becomes an earlier one, with
    /// why it ended. Met again, the same route in the same chat passes.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::BindingDrift`] as [`Self::bind_route`] does,
    /// and [`RoutingError::InvalidBinding`] for a route that is already a
    /// session of another chat.
    pub fn bind_route_in_chat(
        &mut self,
        binding: &SessionRouteBinding,
        chat_id: &str,
        why_the_last_ended: &str,
    ) -> Result<crate::ChatSession, RoutingError> {
        let attachments = serde_json::to_string(&binding.attachments)?;
        let workspace = binding.workspace.to_str().ok_or_else(|| {
            RoutingError::InvalidBinding("workspace is not valid Unicode for ACP/SQLite".into())
        })?;
        self.connection.execute(
            "INSERT OR IGNORE INTO routes(
               route_id, agent_id, agent_profile_id, native_session_id,
               environment_profile_id, workspace, attachments_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                binding.route_id,
                binding.agent_id,
                binding.agent_profile_id,
                binding.native_session_id,
                binding.environment_profile_id,
                workspace,
                attachments,
            ],
        )?;
        self.require_identity(&binding.identity())?;
        if let Some(session) = self.session_of_route(&binding.route_id)? {
            return if session.chat_id == chat_id {
                Ok(session)
            } else {
                Err(RoutingError::InvalidBinding(format!(
                    "route {} is a session of another chat",
                    binding.route_id
                )))
            };
        }
        let agent = self.agent_of_profile(&binding.agent_profile_id)?;
        self.begin_session(
            chat_id,
            &agent.participant_id,
            &binding.route_id,
            why_the_last_ended,
        )
    }

    /// Read one exact route.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::RouteNotFound`] when no route exists.
    pub fn route(&self, route_id: &str) -> Result<SessionRouteBinding, RoutingError> {
        let row = self
            .connection
            .query_row(
                "SELECT agent_id, agent_profile_id, native_session_id,
                        environment_profile_id, workspace, attachments_json
                 FROM routes WHERE route_id = ?1",
                [route_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| RoutingError::RouteNotFound(route_id.to_owned()))?;
        Ok(SessionRouteBinding {
            route_id: route_id.to_owned(),
            agent_id: row.0,
            agent_profile_id: row.1,
            native_session_id: row.2,
            environment_profile_id: row.3,
            workspace: PathBuf::from(row.4),
            attachments: serde_json::from_str(&row.5)?,
        })
    }

    /// Every session this profile has ever opened, newest first, with what the
    /// lane last carried.
    ///
    /// Without this a person had no way to see their own sessions: the browser
    /// remembered exactly one route id in local storage and the ledger could
    /// only be asked about a route whose id you already knew.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn sessions_of_profile(
        &self,
        agent_profile_id: &str,
    ) -> Result<Vec<SessionSummary>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT r.route_id, r.agent_id,
                    (SELECT MAX(e.sequence) FROM events e WHERE e.route_id = r.route_id),
                    (SELECT e.kind FROM events e WHERE e.route_id = r.route_id
                     ORDER BY e.sequence DESC LIMIT 1),
                    (SELECT COUNT(*) FROM events e WHERE e.route_id = r.route_id),
                    (SELECT e.payload_json FROM events e WHERE e.route_id = r.route_id
                     AND e.kind = 'host/turn_written' ORDER BY e.sequence ASC LIMIT 1)
             FROM routes r
             WHERE r.agent_profile_id = ?1",
        )?;
        let rows = statement.query_map([agent_profile_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows {
            let (route_id, agent_id, last_sequence, last_kind, events, first_turn) = row?;
            sessions.push(SessionSummary {
                route_id,
                agent_id,
                last_sequence: match last_sequence {
                    Some(value) => Some(sqlite_to_cursor(value)?),
                    None => None,
                },
                last_kind,
                events: u64::try_from(events).unwrap_or(0),
                opened_with: first_turn.as_deref().and_then(opening_words),
            });
        }
        // Newest first: the lane that moved last is the one a person means.
        sessions.sort_by(|left, right| {
            right
                .last_sequence
                .cmp(&left.last_sequence)
                .then_with(|| right.route_id.cmp(&left.route_id))
        });
        Ok(sessions)
    }

    /// Read a route's lane from the beginning, for a surface that is showing
    /// what was said rather than following what is being said.
    ///
    /// This is deliberately not `events_for_surface`: that one starts after the
    /// surface's durable cursor, so a browser that acknowledged its batches can
    /// never see them again, and a resumed session opened on an empty page.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown route or invalid stored data.
    pub fn history(&self, route_id: &str, limit: usize) -> Result<Vec<SurfaceEvent>, RoutingError> {
        if limit == 0 {
            return Err(RoutingError::InvalidBinding(
                "history limit must be positive".into(),
            ));
        }
        self.ensure_route(route_id)?;
        let limit = i64::try_from(limit).map_err(|_| {
            RoutingError::InvalidBinding("history limit exceeds SQLite integer".into())
        })?;
        let mut statement = self.connection.prepare(
            "SELECT sequence, event_id, kind, source, payload_json
             FROM events WHERE route_id = ?1 ORDER BY sequence ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![route_id, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, event_id, kind, source, payload) = row?;
            events.push(SurfaceEvent {
                sequence: sqlite_to_cursor(sequence)?,
                event_id,
                kind,
                source: serde_json::from_str(&source)?,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(events)
    }

    /// Prove a route met again is the one that was stored.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::BindingDrift`] when the stored route belongs to
    /// another agent or profile, or leads to another native session.
    pub fn require_identity(&self, expected: &RouteIdentity) -> Result<(), RoutingError> {
        let actual = self.route(&expected.route_id)?;
        if actual.identity() == *expected {
            Ok(())
        } else {
            Err(RoutingError::BindingDrift {
                route_id: expected.route_id.clone(),
            })
        }
    }

    /// Atomically append a projected event. Retrying the same `event_id` with
    /// identical content returns its original sequence.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::EventCollision`] for a non-idempotent retry.
    pub fn append_event(
        &mut self,
        route_id: &str,
        event_id: &str,
        kind: &str,
        source: SurfaceEventSource,
        payload: &Value,
    ) -> Result<u64, RoutingError> {
        if event_id.trim().is_empty() || kind.trim().is_empty() {
            return Err(RoutingError::InvalidBinding(
                "event_id and kind must not be empty".into(),
            ));
        }
        let source = serde_json::to_string(&source)?;
        let payload = serde_json::to_string(payload)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let route_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM routes WHERE route_id = ?1)",
            [route_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !route_exists {
            return Err(RoutingError::RouteNotFound(route_id.to_owned()));
        }
        let existing: Option<(i64, String, String, String)> = transaction
            .query_row(
                "SELECT sequence, kind, source, payload_json
                 FROM events WHERE route_id = ?1 AND event_id = ?2",
                params![route_id, event_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if let Some((sequence, stored_kind, stored_source, stored_payload)) = existing {
            if stored_kind != kind || stored_source != source || stored_payload != payload {
                return Err(RoutingError::EventCollision {
                    route_id: route_id.to_owned(),
                    event_id: event_id.to_owned(),
                });
            }
            transaction.commit()?;
            return sqlite_to_cursor(sequence);
        }
        let chat_id: Option<String> = transaction
            .query_row(
                "SELECT chat_id FROM sessions WHERE route_id = ?1",
                [route_id],
                |row| row.get(0),
            )
            .optional()?;
        transaction.execute(
            "INSERT INTO events(route_id, chat_id, event_id, kind, source, payload_json, at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                route_id,
                chat_id,
                event_id,
                kind,
                source,
                payload,
                crate::chat_ledger::now_ms()
            ],
        )?;
        let sequence = transaction.last_insert_rowid();
        transaction.commit()?;
        sqlite_to_cursor(sequence)
    }

    /// Return the next delivery batch after this surface's durable cursor.
    /// Reading does not advance the cursor; only explicit acknowledgement does.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown route or invalid stored data.
    pub fn events_for_surface(
        &self,
        route_id: &str,
        surface_id: &str,
        limit: usize,
    ) -> Result<SurfaceEventBatch, RoutingError> {
        if surface_id.trim().is_empty() || limit == 0 {
            return Err(RoutingError::InvalidBinding(
                "surface_id must not be empty and limit must be positive".into(),
            ));
        }
        self.ensure_route(route_id)?;
        let after_cursor = self.surface_cursor(route_id, surface_id)?;
        let limit = i64::try_from(limit).map_err(|_| {
            RoutingError::InvalidBinding("surface event limit exceeds SQLite integer".into())
        })?;
        let mut statement = self.connection.prepare(
            "SELECT sequence, event_id, kind, source, payload_json
             FROM events
             WHERE route_id = ?1 AND sequence > ?2
             ORDER BY sequence ASC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![route_id, cursor_to_sqlite(after_cursor)?, limit],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?;
        let mut events = Vec::new();
        for row in rows {
            let (sequence, event_id, kind, source, payload) = row?;
            events.push(SurfaceEvent {
                sequence: sqlite_to_cursor(sequence)?,
                event_id,
                kind,
                source: serde_json::from_str(&source)?,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(SurfaceEventBatch {
            after_cursor,
            events,
        })
    }

    /// Persist a monotonic acknowledgement at or below the committed route head.
    ///
    /// # Errors
    ///
    /// Returns an explicit regression/beyond-head error rather than silently
    /// moving a surface to an unverifiable position.
    pub fn acknowledge_surface(
        &mut self,
        route_id: &str,
        surface_id: &str,
        cursor: u64,
    ) -> Result<(), RoutingError> {
        if surface_id.trim().is_empty() {
            return Err(RoutingError::InvalidBinding(
                "surface_id must not be empty".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let head = transaction.query_row(
            "SELECT COALESCE(MAX(sequence), 0) FROM events WHERE route_id = ?1",
            [route_id],
            |row| row.get::<_, i64>(0),
        )?;
        let route_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM routes WHERE route_id = ?1)",
            [route_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !route_exists {
            return Err(RoutingError::RouteNotFound(route_id.to_owned()));
        }
        let current = transaction
            .query_row(
                "SELECT cursor FROM surface_cursors
                 WHERE route_id = ?1 AND surface_id = ?2",
                params![route_id, surface_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map_or(Ok(0), sqlite_to_cursor)?;
        if cursor < current {
            return Err(RoutingError::CursorRegression {
                current,
                requested: cursor,
            });
        }
        let head = sqlite_to_cursor(head)?;
        if cursor > head {
            return Err(RoutingError::CursorBeyondHead {
                requested: cursor,
                head,
            });
        }
        transaction.execute(
            "INSERT INTO surface_cursors(route_id, surface_id, cursor)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(route_id, surface_id) DO UPDATE SET cursor = excluded.cursor",
            params![route_id, surface_id, cursor_to_sqlite(cursor)?],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Read a surface acknowledgement, defaulting to zero for a new surface.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown route or database failure.
    pub fn surface_cursor(&self, route_id: &str, surface_id: &str) -> Result<u64, RoutingError> {
        if surface_id.trim().is_empty() {
            return Err(RoutingError::InvalidBinding(
                "surface_id must not be empty".into(),
            ));
        }
        self.ensure_route(route_id)?;
        self.connection
            .query_row(
                "SELECT cursor FROM surface_cursors
                 WHERE route_id = ?1 AND surface_id = ?2",
                params![route_id, surface_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .map_or(Ok(0), sqlite_to_cursor)
    }

    fn ensure_route(&self, route_id: &str) -> Result<(), RoutingError> {
        let exists = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM routes WHERE route_id = ?1)",
            [route_id],
            |row| row.get::<_, bool>(0),
        )?;
        if exists {
            Ok(())
        } else {
            Err(RoutingError::RouteNotFound(route_id.to_owned()))
        }
    }
}

fn cursor_to_sqlite(cursor: u64) -> Result<i64, RoutingError> {
    i64::try_from(cursor).map_err(|_| RoutingError::CursorOverflow(cursor))
}

fn sqlite_to_cursor(cursor: i64) -> Result<u64, RoutingError> {
    u64::try_from(cursor)
        .map_err(|_| RoutingError::InvalidBinding(format!("negative SQLite cursor: {cursor}")))
}
