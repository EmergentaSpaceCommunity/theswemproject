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
