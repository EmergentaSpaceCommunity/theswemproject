//! Surface-neutral events emitted by a native ACP connection.
//!
//! These values are delivery projections for a Workbench or channel. They are
//! never fed back to the agent and never reconstruct its conversation. The
//! payload preserves the official ACP shape except that binary bodies are
//! replaced with descriptors before an event is emitted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use agent_client_protocol::schema::v1::ContentBlock;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceEventSource {
    NativeLive,
    NativeReplay,
    Host,
}

/// One ordered event observed on a single native ACP connection.
///
/// Ordering is established by the channel that carries these events. Durable
/// route sequence and idempotency identity are assigned only when a projector
/// commits the event to a [`crate::RoutingLedger`].
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NativeSessionEvent {
    pub kind: String,
    pub source: SurfaceEventSource,
    pub payload: Value,
    /// Connection-local raw output that may be projected into the Workbench
    /// content store. It is deliberately absent from serialization: durable
    /// route events contain only the redacted ACP payload above.
    #[serde(skip)]
    pub(crate) output_projection: Vec<NativeOutputProjection>,
}

/// A transient observation carried in the same ordered lane as the durable
/// ACP event that caused it. Routing commits the redacted source event before
/// forwarding this value to any optional Workbench projector.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NativeOutputProjection {
    Content {
        turn_index: usize,
        ordinal: usize,
        content: Box<ContentBlock>,
    },
    TurnComplete {
        turn_index: usize,
    },
}

impl NativeSessionEvent {
    #[must_use]
    pub fn new(kind: impl Into<String>, source: SurfaceEventSource, payload: Value) -> Self {
        Self {
            kind: kind.into(),
            source,
            payload,
            output_projection: Vec::new(),
        }
    }

    pub(crate) fn with_output_projection(
        mut self,
        output_projection: Vec<NativeOutputProjection>,
    ) -> Self {
        self.output_projection = output_projection;
        self
    }
}

/// Who wrote a turn, and from where.
///
/// The Workbench had no such notion: a session belonged to a profile, and the
/// one who wrote was a constant. While one person writes from one browser that
/// is invisible. The moment a channel and a clock write into the same session,
/// the same conversation carries turns from several people and from nobody,
/// and the record cannot tell them apart.
///
/// This is not bookkeeping. An agent left to work alone acts on what it is
/// told; if it cannot tell who told it, there can be no list of who may tell
/// it anything. Every rule a channel needs - who is allowed, what a group
/// message means, whether one agent may drive another - stands on this one
/// value.
///
/// ACP has no field for an author, so provenance either travels in the
/// content, in the open, or it does not exist. It travels in the content.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Correspondent {
    /// Where the turn came from: the Workbench page, a channel, the clock.
    pub surface: String,
    /// Who wrote it, in the words that surface knows them by. Absent is
    /// honest - nobody said - and is what the Workbench sends until a person
    /// names themselves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Who it was written to, when the surface carries that: a channel
    /// message may be addressed to one agent among several in a room.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addressed_to: Option<String>,
}

/// The longest a name may be. Long enough for a person, a handle or a room;
/// short enough that a provenance line stays one line.
const NAME_LIMIT: usize = 96;

impl Correspondent {
    /// Check the names before they are written anywhere.
    ///
    /// The provenance line goes into the content the agent reads, so a name
    /// that could carry a line break could write a second line of its own and
    /// claim to be someone else. Control characters are refused for that
    /// reason and not for tidiness.
    ///
    /// # Errors
    ///
    /// Returns the reason a name cannot be used, naming the field.
    pub fn validate(&self) -> Result<(), String> {
        check("surface", Some(&self.surface))?;
        check("author", self.author.as_deref())?;
        check("addressed_to", self.addressed_to.as_deref())
    }

    /// The line the agent reads above what was written.
    ///
    /// Plain prose, because the agent is a language model and this is the
    /// same thing a person would say: who is talking, and where from.
    #[must_use]
    pub fn provenance_line(&self) -> String {
        let who = self.author.as_deref().unwrap_or("Someone");
        match self.addressed_to.as_deref() {
            Some(addressed) => {
                format!("[{who}, writing to {addressed} from {}]", self.surface)
            }
            None => format!("[{who}, writing from {}]", self.surface),
        }
    }
}

fn check(field: &str, value: Option<&str>) -> Result<(), String> {
    let Some(value) = value else { return Ok(()) };
    if value.trim().is_empty() {
        return Err(format!("{field} is empty"));
    }
    if value.chars().count() > NAME_LIMIT {
        return Err(format!("{field} is longer than {NAME_LIMIT} characters"));
    }
    if value.chars().any(char::is_control) {
        return Err(format!(
            "{field} carries a control character, and a name that can break a line can write a line of its own"
        ));
    }
    Ok(())
}
