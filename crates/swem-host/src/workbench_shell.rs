//! Minimal generic Workbench browser shell (implementation.typ, "Следующая
//! автономная итерация" before H1).
//!
//! The shell is a thin product boundary over three already-proven seams:
//! profile selection ([`PersonalAgentProfileStore`]), the interactive driver
//! ([`run_interactive_native_session`] + [`NativeSessionControl`]) and the
//! durable surface projection ([`RoutingLedger`] cursors fed by
//! [`project_native_session_events`]). Every HTTP endpoint is a projection of
//! a ledger read or a driver command; there is no second ACP JSON-RPC wire,
//! no second conversation history and no knowledge of any agent, Cycle or
//! domain schema. The browser never carries the native session id - reconnect
//! is route binding plus `session/load`/`session/resume`, host-side.

use std::collections::{BTreeMap, BTreeSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    ContentBlock, ElicitationAction as AcpElicitationAction, ElicitationCapabilities,
    ElicitationFormCapabilities, ElicitationUrlCapabilities, EmbeddedResourceResource,
    FileSystemCapabilities, McpServer, McpServerStdio,
};
use base64::Engine as _;
use futures_util::TryStreamExt as _;
use http_body_util::{BodyExt as _, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::header::HeaderValue;
use hyper::{Method, Request, Response, StatusCode};
use rmcp::model::{ElicitationAction, JsonObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncSeekExt as _, AsyncWriteExt as _, SeekFrom};
use tokio_util::io::ReaderStream;

use crate::routing::{NativeOutputProjectionRequest, project_native_session_events_with_output};
use crate::workbench_apps::{self, ConnectionApps, OpenApp, OpenedApp, RelayRefusal};
#[path = "workbench_shell/agent_environment.rs"]
mod agent_environment;
mod door;
mod hosts;
pub(crate) use door::SaidBy;
pub use door::{Way, certificate_good_until};
#[path = "workbench_shell/mcp_servers.rs"]
mod mcp_servers;
pub use hosts::{EnvironmentOffered, HostStanding, MachineWanted, SetUp, SetUpStep};
mod channels;
mod model_providers;
mod provider_keys;
pub use model_providers::{
    DeclareModelProviderBody, MODEL_PROVIDER_SCHEMA, ModelChoice, ModelProvider, ModelProviderBook,
    ModelProviderOrigin, ModelProviderView,
};
pub use provider_keys::{GiveKeyBody, ModelProviderStanding};
#[path = "workbench_shell/server_apps.rs"]
mod server_apps;
pub use server_apps::SpaceView;
#[path = "workbench_shell/model_context.rs"]
mod model_context;
pub use model_context::{BindModelContextBody, ModelContext, ModelContextBlock};
#[path = "workbench_shell/chats.rs"]
mod chats;
#[path = "workbench_shell/runtime.rs"]
mod runtime;
#[path = "workbench_shell/stream.rs"]
mod stream;
pub use chats::{ChatPage, SaidInChat, Saying, StartChatBody};
#[path = "workbench_shell/files.rs"]
mod files;
pub use files::{ChangeTreeBody, FileOpened, SaveFileBody, SavedOrNot};
#[path = "workbench_shell/looks.rs"]
mod looks;
pub use looks::{EngineLooked, LOOK_INSIDE_SCHEMA, LookInside};
#[path = "workbench_shell/removal.rs"]
mod removal;
pub use removal::AgentRemoved;
#[path = "workbench_shell/keepers.rs"]
mod keepers;
mod store;
mod tunnel;
pub use channels::{
    AddChannelBody, CHANNEL_SCHEMA, ChangeChannelBody, ChannelDocument, ChannelPackage,
    ChannelShown, ChannelsStanding, GuestPolicy, GuestWaiting, Reach,
};
pub use keepers::{ChooseKeeperBody, KeeperShown, TurnOnBody};
pub use tunnel::{IDLE as TUNNEL_IDLE, ReachStanding, TunnelPackage, TunnelShown};
#[path = "workbench_shell/timekeeper.rs"]
mod timekeeper;
pub use timekeeper::{KeeperStanding, NewScheduleBody, ScheduleShown, nothing_for_a_keeper_to_do};
#[path = "workbench_shell/terminal.rs"]
mod terminal;
use crate::workbench_content::{WorkbenchContentSource, WorkbenchContentStore};
use crate::workbench_observation::{
    ObservationIngress, ObservationRuntime, ObservedAppCall, ObserverCommand,
};
use crate::{
    CredentialSourceRef, EnvironmentLease, EnvironmentRequirements, EnvironmentTransport,
    LaunchCommand, NativeOutputProjection, NativeSessionControl, NativeSessionOptions,
    NativeSessionOutcome, NativeSessionStart, NativeTurnOutcome, PersonalAgentProfile,
    PersonalAgentProfileStore, ProfileError, Readiness, ResolvedMcpAttachment, RoutingLedger,
    SessionRouteBinding, SupplyError, SurfaceEventBatch, SurfaceEventSource,
    direct_environment_lease, run_interactive_native_session,
};
pub use agent_environment::{AmendProfileBody, TerminalInputBody, TerminalSizeBody};
pub use mcp_servers::{
    DeclareMcpServerBody, McpCatalogue, McpServerOrigin, McpServerView, NamedValue,
};
pub use store::{
    ACP_REGISTRY_CACHE, AddIndexBody, ArchiveDistribution, BinaryDistribution, CATALOG_SCHEMA,
    Catalog, CatalogDistribution, CatalogEntry, INDEX_SCHEMA, IndexFile, InstalledSkill, KindShown,
    KindWords, NpxDistribution, Planned, RegistryStatus, Requirement, StoreEntry, StoreIndexView,
    StoreInstallBody, StorePlanBody, StoreRemoveBody, StoreView, Taker, Takes, UvxDistribution,
};
pub(crate) use terminal::working_directory;
pub use terminal::{
    ExitReport, OpenTerminalBody, TerminalOpener, TerminalOutput, TerminalView, Terminals,
};

/// A config option's value as a person's form sends it: a boolean for a
/// toggle, a string for a choice, or the wire shape itself.
fn config_value_of(
    value: &Value,
) -> Result<agent_client_protocol::schema::v1::SessionConfigOptionValue, WorkbenchShellError> {
    use agent_client_protocol::schema::v1::{SessionConfigOptionValue, SessionConfigValueId};
    match value {
        Value::Bool(value) => Ok(SessionConfigOptionValue::Boolean { value: *value }),
        Value::String(value) => Ok(SessionConfigOptionValue::ValueId {
            value: SessionConfigValueId::new(value.as_str()),
        }),
        other => serde_json::from_value(other.clone()).map_err(|error| {
            WorkbenchShellError::Invalid(format!("not a config option value: {error}"))
        }),
    }
}

/// Offer the profile's model to the agent through `session/set_config_option`
/// when the agent lists it, and say what came of it.
///
/// A value the agent did not advertise is never sent: the control refuses it
/// first, and a refusal on the wire would end the session. A model the agent
/// does not offer is not a failure of the open - the agent runs with its own,
/// and the sentence says so.
async fn apply_profile_model(control: &crate::NativeSessionControl, model: &str) -> String {
    use agent_client_protocol::schema::v1::{
        SessionConfigKind, SessionConfigOptionCategory, SessionConfigOptionValue,
        SessionConfigSelectOptions, SessionConfigValueId,
    };
    let configuration = control.configuration().await;
    let Some(options) = configuration.config_options else {
        return "this agent offers no model choice on the session; it runs with its own".into();
    };
    let wanted = model.rsplit('/').next().unwrap_or(model);
    let listed = options.iter().find(|option| {
        matches!(option.category, Some(SessionConfigOptionCategory::Model))
            && match &option.kind {
                SessionConfigKind::Select(select) => {
                    let choices: Vec<&str> = match &select.options {
                        SessionConfigSelectOptions::Ungrouped(choices) => choices
                            .iter()
                            .map(|choice| choice.value.0.as_ref())
                            .collect(),
                        SessionConfigSelectOptions::Grouped(groups) => groups
                            .iter()
                            .flat_map(|group| group.options.iter())
                            .map(|choice| choice.value.0.as_ref())
                            .collect(),
                        _ => Vec::new(),
                    };
                    choices.contains(&model) || choices.contains(&wanted)
                }
                _ => false,
            }
    });
    let Some(option) = listed else {
        return format!("this agent does not offer the model {model}; it runs with its own");
    };
    let value = match &option.kind {
        SessionConfigKind::Select(select) => match &select.options {
            SessionConfigSelectOptions::Ungrouped(choices)
                if choices
                    .iter()
                    .any(|choice| choice.value.0.as_ref() == model) =>
            {
                model
            }
            SessionConfigSelectOptions::Grouped(groups)
                if groups
                    .iter()
                    .flat_map(|group| group.options.iter())
                    .any(|choice| choice.value.0.as_ref() == model) =>
            {
                model
            }
            _ => wanted,
        },
        _ => wanted,
    };
    match control
        .set_config_option(
            option.id.0.as_ref(),
            SessionConfigOptionValue::ValueId {
                value: SessionConfigValueId::new(value),
            },
        )
        .await
    {
        Ok(_) => format!("model {value} set on the session"),
        Err(error) => format!("the agent refused the model {value}: {error}"),
    }
}

/// The process environment a direct connection adds to the filtered bootstrap
/// set: the secrets a person gave the profile (what an `env_var`
/// authentication method asks for) plus the profile's credential bindings,
/// each read from its source variable of THIS host process and handed to the
/// agent under its declared target name. The prepared (container) path materializes the same bindings
/// as runtime secrets; the direct path had no equivalent, so a profile that
/// needs a token could never authenticate through the shell. Values are never
/// recorded: the session ledger keeps override names only.
///
/// # Errors
///
/// Fails closed when a bound source variable is absent or empty.
pub fn credential_environment(
    profile: &PersonalAgentProfile,
    secrets: BTreeMap<String, String>,
) -> Result<BTreeMap<String, String>, WorkbenchShellError> {
    // What the person typed into the profile comes first; a host-side
    // binding for the same name is the operator's decision and wins.
    let mut environment = secrets;
    for binding in &profile.credential_bindings {
        let CredentialSourceRef::EnvironmentVariable { name } = &binding.source;
        let value = std::env::var(name).map_err(|_| {
            WorkbenchShellError::Failed(format!(
                "credential binding {} needs the host environment variable {name}",
                binding.binding_id
            ))
        })?;
        if value.is_empty() {
            return Err(WorkbenchShellError::Failed(format!(
                "credential binding {} found the host environment variable {name} empty",
                binding.binding_id
            )));
        }
        environment.insert(binding.target_environment.clone(), value);
    }
    Ok(environment)
}

/// The static shell page served at `/`. It knows profile/route/driver/event
/// vocabulary only; agents, Cycle, Telegram and domain schemas never appear.
const SHELL_HTML: &str = include_str!("workbench_shell/shell.html");
/// The shared view kit: one stylesheet the shell and every domain App render
/// from. It lives at the repository root rather than inside this crate because
/// the host and the domain modules are peers that both consume it.
const KIT_CSS: &str = include_str!("../../../web/view-kit/kit.css");
/// SWEM's own values for the host-style vocabulary, in both themes. The shell
/// is a host, so these are what it sends across the sandbox; an App inlines the
/// same file so it has values before any host context arrives.
const PALETTE_CSS: &str = include_str!("../../../web/view-kit/palette.css");

/// The production-built Workbench shell is compiled into the Rust binary.
/// Node is a source-build dependency, never a product runtime dependency.
/// How the Workbench's own page lays the kit's shapes out.
const WORKBENCH_CSS: &str = include_str!("workbench_shell/workbench.css");
const WORKBENCH_JS: &str = include_str!("../web/apps-host/dist/workbench.js");
/// A channel's Mini App: the page a bot opens inside the messenger.
const MINI_APP_HTML: &str = include_str!("../../../web/mini-app/index.html");

/// Connection-local launch material for a profile resolved to a direct
/// process. The CLI resolves through agent discovery; tests resolve to fixture
/// binaries. The durable profile never stores commands, so this value exists
/// only for the lifetime of one open connection.
#[derive(Clone, Debug)]
pub struct ResolvedDirectAgentConnection {
    pub launch: LaunchCommand,
    pub agent_executable: PathBuf,
    /// Connection-local MCP declarations. Every durable profile attachment
    /// must be resolved by `server_name`; the shell fails closed otherwise.
    pub mcp_servers: Vec<McpServer>,
}

/// Environment selected by the product resolver for one Workbench connection.
/// A prepared environment is already bound to one exact lease and transport;
/// Workbench must neither reconstruct nor weaken either value.
#[derive(Clone, Debug)]
pub enum ResolvedAgentEnvironment {
    Direct,
    Prepared {
        environment_profile_id: String,
        lease: Box<EnvironmentLease>,
        transport: Box<EnvironmentTransport>,
    },
}

/// Connection-local native agent launch after supply, environment and MCP
/// profile resolution. It contains no durable session state or secret value.
#[derive(Clone, Debug)]
pub struct ResolvedAgentConnection {
    pub launch: LaunchCommand,
    pub agent_executable: PathBuf,
    pub mcp_servers: Vec<McpServer>,
    pub environment: ResolvedAgentEnvironment,
}

impl From<ResolvedDirectAgentConnection> for ResolvedAgentConnection {
    fn from(connection: ResolvedDirectAgentConnection) -> Self {
        Self {
            launch: connection.launch,
            agent_executable: connection.agent_executable,
            mcp_servers: connection.mcp_servers,
            environment: ResolvedAgentEnvironment::Direct,
        }
    }
}

/// How the shell reports a failed operation to its HTTP layer and tests.
#[derive(Debug, thiserror::Error)]
pub enum WorkbenchShellError {
    /// Unknown profile, connection or route.
    #[error("not found: {0}")]
    NotFound(String),
    /// The operation is well-formed but refused (unsupported close, binding
    /// drift, invalid permission option). The connection stays usable.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Malformed input.
    #[error("invalid request: {0}")]
    Invalid(String),
    /// Whoever asks is not who may: a signature that is not the
    /// messenger's, a person not known here.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// The underlying session/ledger failed.
    #[error("{0}")]
    Failed(String),
}

impl WorkbenchShellError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Invalid(_) => StatusCode::BAD_REQUEST,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::Failed(_) => StatusCode::BAD_GATEWAY,
        }
    }
}

type Resolver =
    dyn Fn(&PersonalAgentProfile) -> Result<ResolvedAgentConnection, String> + Send + Sync;

#[derive(Default)]
struct ArtifactTurnProjection {
    capture: WorkbenchArtifactCapture,
    seen: BTreeSet<String>,
    complete: bool,
}

#[derive(Default)]
struct ArtifactProjectionState {
    turns: BTreeMap<usize, ArtifactTurnProjection>,
    closed: bool,
}

#[derive(Clone)]
struct WorkbenchArtifactProjector {
    content: WorkbenchContentStore,
    ledger_path: PathBuf,
    route_id: String,
    agent_id: String,
    environment_lease: EnvironmentLease,
    event_namespace: String,
    next_event: Arc<AtomicU64>,
    state: Arc<tokio::sync::Mutex<ArtifactProjectionState>>,
    changed: Arc<tokio::sync::Notify>,
}

impl WorkbenchArtifactProjector {
    async fn run(
        self,
        mut input: tokio::sync::mpsc::UnboundedReceiver<NativeOutputProjectionRequest>,
    ) {
        while let Some(request) = input.recv().await {
            match request.projection.clone() {
                NativeOutputProjection::Content {
                    turn_index,
                    ordinal,
                    content,
                } => self.project_content(turn_index, ordinal, &content).await,
                NativeOutputProjection::TurnComplete { turn_index } => {
                    self.state
                        .lock()
                        .await
                        .turns
                        .entry(turn_index)
                        .or_default()
                        .complete = true;
                    self.changed.notify_waiters();
                }
            }
            request.acknowledge();
        }
        self.state.lock().await.closed = true;
        self.changed.notify_waiters();
    }

    async fn wait_turn(&self, turn_index: usize) -> WorkbenchArtifactCapture {
        loop {
            let changed = self.changed.notified();
            {
                let mut state = self.state.lock().await;
                if state
                    .turns
                    .get(&turn_index)
                    .is_some_and(|turn| turn.complete)
                {
                    return state
                        .turns
                        .remove(&turn_index)
                        .map_or_else(WorkbenchArtifactCapture::default, |turn| turn.capture);
                }
                if state.closed {
                    let mut capture = state
                        .turns
                        .remove(&turn_index)
                        .map_or_else(WorkbenchArtifactCapture::default, |turn| turn.capture);
                    capture.issues.push(WorkbenchArtifactIssue::new(
                        format!("turn-{turn_index}"),
                        None,
                        "artifact_projection_incomplete",
                    ));
                    return capture;
                }
            }
            changed.await;
        }
    }

    async fn project_content(&self, turn_index: usize, ordinal: usize, block: &ContentBlock) {
        let Some(captured) = self.capture_content_block(ordinal, block).await else {
            return;
        };
        match captured {
            Ok(descriptor) => {
                let is_new = self
                    .state
                    .lock()
                    .await
                    .turns
                    .entry(turn_index)
                    .or_default()
                    .seen
                    .insert(descriptor.descriptor_id.clone());
                if !is_new {
                    return;
                }
                let projected = self
                    .record_artifact_event(
                        "host/artifact_available",
                        turn_index,
                        ordinal,
                        &descriptor,
                    )
                    .await;
                let mut state = self.state.lock().await;
                let turn = state.turns.entry(turn_index).or_default();
                if projected.is_err() {
                    turn.capture.issues.push(WorkbenchArtifactIssue::new(
                        descriptor.name.clone(),
                        Some(descriptor.media_type.clone()),
                        "route_projection_failed",
                    ));
                }
                turn.capture.artifacts.push(descriptor);
            }
            Err(issue) => {
                let _ = self
                    .record_artifact_event("host/artifact_unavailable", turn_index, ordinal, &issue)
                    .await;
                self.state
                    .lock()
                    .await
                    .turns
                    .entry(turn_index)
                    .or_default()
                    .capture
                    .issues
                    .push(issue);
            }
        }
    }

    async fn capture_content_block(
        &self,
        index: usize,
        block: &ContentBlock,
    ) -> Option<CapturedArtifact> {
        match block {
            ContentBlock::Image(image) => {
                let fallback = format!("agent-image-{}", index + 1);
                let name = image.uri.as_deref().map_or_else(
                    || fallback.clone(),
                    |uri| content_name_from_uri(uri, &fallback),
                );
                Some(
                    self.capture_encoded_content(&image.data, name, image.mime_type.clone())
                        .await,
                )
            }
            ContentBlock::Audio(audio) => Some(
                self.capture_encoded_content(
                    &audio.data,
                    format!("agent-audio-{}", index + 1),
                    audio.mime_type.clone(),
                )
                .await,
            ),
            ContentBlock::ResourceLink(resource) => Some(
                match self
                    .environment_lease
                    .resolve_workspace_file_uri(&resource.uri)
                {
                    // The inbox holds what a person handed over. An agent
                    // naming one of those files is quoting the person, not
                    // producing an artifact, and recording it as agent output
                    // would put the person's own file in the record under the
                    // agent's name. Everything else in the workspace is the
                    // agent's to hand back.
                    Ok(requested)
                        if requested.starts_with(
                            self.environment_lease
                                .workspace
                                .join(crate::workbench_files::INBOX),
                        ) =>
                    {
                        return None;
                    }
                    Ok(requested) => self
                        .content
                        .ingest_workspace_link(
                            resource,
                            &self.environment_lease.workspace,
                            &requested,
                            self.agent_id.clone(),
                        )
                        .await
                        .map_err(|error| {
                            let reason = match error {
                                WorkbenchShellError::Invalid(_)
                                | WorkbenchShellError::NotFound(_)
                                | WorkbenchShellError::Forbidden(_) => "resource_link_unavailable",
                                WorkbenchShellError::Conflict(_)
                                | WorkbenchShellError::Failed(_) => "artifact_store_failed",
                            };
                            WorkbenchArtifactIssue::new(
                                resource.name.clone(),
                                resource.mime_type.clone(),
                                reason,
                            )
                        }),
                    Err(_) => Err(WorkbenchArtifactIssue::new(
                        resource.name.clone(),
                        resource.mime_type.clone(),
                        "resource_link_unavailable",
                    )),
                },
            ),
            ContentBlock::Resource(resource) => match &resource.resource {
                EmbeddedResourceResource::TextResourceContents(text) => {
                    let name =
                        content_name_from_uri(&text.uri, &format!("agent-resource-{}", index + 1));
                    let media_type = text
                        .mime_type
                        .clone()
                        .unwrap_or_else(|| "text/plain;charset=utf-8".into());
                    Some(
                        self.capture_content_bytes(text.text.as_bytes(), name, media_type)
                            .await,
                    )
                }
                EmbeddedResourceResource::BlobResourceContents(blob) => {
                    let name =
                        content_name_from_uri(&blob.uri, &format!("agent-resource-{}", index + 1));
                    let media_type = blob
                        .mime_type
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into());
                    Some(
                        self.capture_encoded_content(&blob.blob, name, media_type)
                            .await,
                    )
                }
                _ => None,
            },
            _ => None,
        }
    }

    async fn capture_encoded_content(
        &self,
        encoded: &str,
        name: String,
        media_type: String,
    ) -> CapturedArtifact {
        let bytes = decode_standard_base64(encoded).map_err(|_| {
            WorkbenchArtifactIssue::new(
                name.clone(),
                Some(media_type.clone()),
                "invalid_inline_content",
            )
        })?;
        self.capture_content_bytes(&bytes, name, media_type).await
    }

    async fn capture_content_bytes(
        &self,
        bytes: &[u8],
        name: String,
        media_type: String,
    ) -> CapturedArtifact {
        self.content
            .ingest_bytes(
                bytes,
                name.clone(),
                media_type.clone(),
                WorkbenchContentSource::AgentOutput,
                self.agent_id.clone(),
            )
            .await
            .map_err(|_| {
                WorkbenchArtifactIssue::new(name, Some(media_type), "artifact_store_failed")
            })
    }

    async fn record_artifact_event(
        &self,
        kind: &'static str,
        turn_index: usize,
        ordinal: usize,
        payload: &impl Serialize,
    ) -> Result<(), WorkbenchShellError> {
        let event = self.next_event.fetch_add(1, Ordering::SeqCst);
        let route = self.route_id.clone();
        let event_id = format!("{}:artifact:{event}", self.event_namespace);
        let mut payload = serde_json::to_value(payload)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if let Value::Object(object) = &mut payload {
            object.insert("turn_index".into(), json!(turn_index));
            object.insert("content_ordinal".into(), json!(ordinal));
        }
        let ledger_path = self.ledger_path.clone();
        tokio::task::spawn_blocking(move || {
            let mut ledger = RoutingLedger::open(&ledger_path)?;
            ledger.append_event(&route, &event_id, kind, SurfaceEventSource::Host, &payload)
        })
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        .map(|_| ())
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }
}

struct WorkbenchConnection {
    control: NativeSessionControl,
    route_id: String,
    artifact_projector: WorkbenchArtifactProjector,
    runner: tokio::sync::Mutex<
        Option<tokio::task::JoinHandle<Result<NativeSessionOutcome, SupplyError>>>,
    >,
    projection: tokio::sync::Mutex<Option<crate::NativeRouteProjection>>,
    artifact_projection: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Durable attachment identity paired with the connection-local MCP
    /// declaration - the Apps host dials these with ITS OWN clients; the
    /// agent's MCP children stay agent-owned.
    attachments: Vec<ResolvedMcpAttachment>,
    /// Lazily discovered Apps state (host-side clients, open registry).
    apps: tokio::sync::Mutex<Option<ConnectionApps>>,
    /// Exact agent-initiated App payloads. This state is never persisted and
    /// is destroyed with the connection.
    observation: tokio::sync::Mutex<Option<ObservationRuntime>>,
    /// Idempotent observation-to-view binding. A retried long poll must not
    /// create a second App instance for the same native tool call.
    observed_apps: tokio::sync::Mutex<BTreeMap<String, OpenedApp>>,
    context_events: AtomicU64,
    /// What the profile's setup came to on this connection, in sentences:
    /// where the role went, which variable the model took, whether the agent
    /// offered the model natively. Read off the connection's status.
    setup_notes: tokio::sync::Mutex<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ObservedAppOpen {
    pub opened: OpenedApp,
    pub observation: ObservedAppCall,
}

/// One host-discovered agent shown by the first-run Workbench. Discovery is
/// read-only; `available` only means that this host can attempt a native ACP
/// launch. The handshake performed while opening a connection remains the
/// authority for readiness.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkbenchAgentOption {
    pub agent_id: String,
    pub name: String,
    pub readiness: Readiness,
    pub available: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkbenchOnboarding {
    pub enabled: bool,
    pub agents: Vec<WorkbenchAgentOption>,
}

struct LocalOnboarding {
    /// What this host knows is installed. Behind a lock because installing an
    /// agent changes the answer while the product is running, and a person who
    /// just pressed Install should not have to restart to use what they
    /// installed.
    agents: std::sync::RwLock<BTreeMap<String, WorkbenchAgentOption>>,
    workspaces_root: PathBuf,
    agent_homes_root: PathBuf,
}

/// How a host finds out what is installed now.
///
/// Discovery is the host's, not the shell's - it probes executables and reads
/// receipts - so the shell holds it as a function and calls it when the answer
/// can have changed.
type RediscoverAgents = Box<dyn Fn() -> Vec<WorkbenchAgentOption> + Send + Sync>;

/// The official MCP Apps bridge this product was built against, committed in
/// the repository and pinned by the bundle-freshness gate.
const APPS_BRIDGE: &str = include_str!("../web/apps-host/dist/apps-bridge.js");

/// The node runtime an npx distribution needs, if the host has one.
fn which_node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["node.exe", "node"]
    } else {
        &["node"]
    };
    std::env::split_paths(&path)
        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

/// The declaration's ACP name, refusing a transport this host cannot dial.
fn declaration_name(declaration: &McpServer) -> Result<String, WorkbenchShellError> {
    match declaration {
        McpServer::Stdio(stdio) => Ok(stdio.name.clone()),
        McpServer::Http(http) => Ok(http.name.clone()),
        McpServer::Sse(sse) => Ok(sse.name.clone()),
        other => Err(WorkbenchShellError::Invalid(format!(
            "unsupported MCP declaration transport: {other:?}"
        ))),
    }
}

/// Requested connection start, before the ledger supplies the session id.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ShellConnectionMode {
    New,
    Load,
    Resume,
}

/// The state behind every shell endpoint. Component tests drive these methods
/// directly; the HTTP layer only serializes them.
pub struct WorkbenchShellState {
    inventory: PersonalAgentProfileStore,
    ledger_path: PathBuf,
    operation_timeout: Duration,
    resolver: Box<Resolver>,
    connections: tokio::sync::Mutex<BTreeMap<String, Arc<WorkbenchConnection>>>,
    next_connection: AtomicU64,
    content: WorkbenchContentStore,
    /// `(sandbox_url, sandbox_origin)` once the serve layer bound the
    /// second-origin sandbox listener; `None` in headless/component use.
    sandbox: std::sync::OnceLock<(String, String)>,
    /// The secret a caller must hold to work this Workbench, when the product
    /// minted one. Absent = this shell asks for none, which is what an
    /// in-process test or an embedder gets; the product always mints one.
    session_token: std::sync::OnceLock<String>,
    /// The address this Workbench is served at and who may come in there,
    /// when it is served at one. Then the door is sign-in, not the secret
    /// of a run.
    served_at: std::sync::OnceLock<door::ServedAt>,
    /// The path this Workbench is drawn under on the server of a product
    /// the harness is built into, when it is built into one. Then who asks
    /// is what that product says.
    built_under: std::sync::OnceLock<String>,
    /// What the product is called on the page, when it is not SWEM.
    called: std::sync::OnceLock<String>,
    /// Directory holding the esbuild output `apps-bridge.js`; absent = the
    /// Apps panel stays disabled (the honest App-disabled mode).
    apps_bundle: std::sync::OnceLock<PathBuf>,
    /// Product-internal command used only to wrap App-linked stdio servers.
    mcp_observer: std::sync::OnceLock<ObserverCommand>,
    /// Optional product onboarding. Tests and embedders may omit it and retain
    /// the original read-only inventory shell.
    onboarding: std::sync::OnceLock<LocalOnboarding>,
    /// How to ask the host what is installed now; see `set_agent_discovery`.
    rediscover: std::sync::OnceLock<RediscoverAgents>,
    /// The ACP registry index this product reads, when its builder named
    /// one; else the crate's default for this process.
    acp_registry_index: std::sync::OnceLock<String>,
    /// The Apps of the servers this host declared, outside any agent
    /// session: one slot per declaration, dialled when a space of that
    /// server is listed or opened, refreshed by name. A server declared
    /// while another's App is open is found because the next lookup reads
    /// the declarations, and nothing built earlier is thrown away for it.
    server_apps: server_apps::ServerApps,
    /// The MCP servers a person declared for their agents, and the directory
    /// they are kept in. Absent in tests and embedders that declare their own.
    mcp_catalogue: Arc<std::sync::OnceLock<McpCatalogue>>,
    /// The model providers a profile may name, shipped and declared.
    model_providers: std::sync::OnceLock<ModelProviderBook>,
    /// Where what is found of this machine is kept, and what was found.
    machine_root: std::sync::OnceLock<PathBuf>,
    machine_look: std::sync::RwLock<Option<crate::MachineLook>>,
    /// The plan to set containers up that was offered, and how it goes.
    container_setup: std::sync::Mutex<Option<(crate::ProvisioningPlan, SetUp)>>,
    /// The keys of providers, given once for every agent.
    provider_keys: std::sync::OnceLock<crate::KeyStore>,
    /// Where what this product installs lands (`<data root>/installed`), when
    /// installing is enabled; a host alone installs nothing.
    installed_root: std::sync::OnceLock<PathBuf>,
    /// The indexes the store reads, when a store is enabled.
    store: std::sync::OnceLock<store::Store>,
    /// The way back from the Store to the host it lives in, for servers
    /// that take packages; filled in once the host is shared.
    host_of_the_store: Arc<std::sync::OnceLock<std::sync::Weak<WorkbenchShellState>>>,
    /// The standing instructions a clock runs, and the directory they are
    /// kept in. Absent until a host enables them.
    /// The command that serves an agent's own schedules to its session.
    time_tools: std::sync::OnceLock<(PathBuf, Vec<String>)>,
    /// The keeper's lock, held for as long as this process keeps time.
    timekeeper: std::sync::OnceLock<std::fs::File>,
    /// Where what is removed is kept aside.
    removed_root: std::sync::OnceLock<PathBuf>,
    /// Who keeps time for whom, as it was chosen.
    keepers: std::sync::OnceLock<crate::Keepers>,
    channels: std::sync::OnceLock<channels::Channels>,
    /// Where the product hosts a copy of the page a bot opens inside the
    /// messenger, if it does.
    app_hosted_at: std::sync::OnceLock<String>,
    /// A tunnel that is open, and where tunnels keep their files.
    tunnel: std::sync::Mutex<Option<tunnel::Tunnel>>,
    tunnel_root: std::sync::OnceLock<PathBuf>,
    /// The command the system's scheduler starts to keep time once.
    keep_time_command: std::sync::OnceLock<(PathBuf, Vec<String>)>,
    /// The terminals open on this host. A terminal runs in the environment of
    /// one profile, which is how an agent that signs in at a prompt is signed
    /// in at all - and, since a session may open one over ACP, how an agent
    /// runs a command a person can watch. Shared, because a session holds it
    /// for as long as it is connected.
    terminals: Arc<Terminals>,
    /// What the chats of this Workbench are doing now: which agents are at
    /// work, which connection a chat's agent is live on, what waits for an
    /// answer. The ledger is the record; this is the process's own hands.
    chat_runtime: runtime::ChatRuntime,
}

/// Where a session that is being opened belongs: a chat that already exists.
#[derive(Clone, Debug)]
pub(crate) struct ChatPlace {
    pub chat_id: String,
    /// Why the session the agent had there ended, when it had one.
    pub why_the_last_ended: String,
}

/// Everything that is said when a connection is asked for.
pub(crate) struct Opening {
    pub profile_id: String,
    pub mode: ShellConnectionMode,
    pub route_id: Option<String>,
    pub requested_connection_id: Option<String>,
    pub auth_method_id: Option<String>,
    pub file_callbacks: Option<FileSystemCapabilities>,
    pub place: Option<ChatPlace>,
}

/// What a person changed about a profile since a chat began with it, in
/// words.
fn configuration_differences(
    stored: &SessionRouteBinding,
    wanted: &SessionRouteBinding,
) -> Vec<String> {
    let mut differences = Vec::new();
    if stored.workspace != wanted.workspace {
        differences.push("it works somewhere else now".to_owned());
    }
    if stored.environment_profile_id != wanted.environment_profile_id {
        differences.push("it runs in another environment now".to_owned());
    }
    let names = |binding: &SessionRouteBinding| {
        binding
            .attachments
            .iter()
            .map(|attachment| attachment.server_name.clone())
            .collect::<BTreeSet<_>>()
    };
    let (before, now) = (names(stored), names(wanted));
    if before != now {
        let added: Vec<&String> = now.difference(&before).collect();
        let removed: Vec<&String> = before.difference(&now).collect();
        if !added.is_empty() {
            differences.push(format!(
                "it now reaches {}",
                added
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !removed.is_empty() {
            differences.push(format!(
                "it no longer reaches {}",
                removed
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    differences
}

/// The sentence for an agent that no longer has the session a chat led to.
///
/// The driver says it in the protocol's words; a person is told what it means
/// for them. What was said stays in the record either way.
fn cannot_go_on(failure: &str) -> Option<String> {
    [
        "reconnect target was absent from ACP session/list",
        "agent did not advertise ACP loadSession",
        "agent did not advertise ACP sessionCapabilities.resume",
    ]
    .iter()
    .any(|marker| failure.contains(marker))
    .then(|| {
        "the agent no longer has this conversation on its side, so it cannot go on from where it stopped. \
         What was said is kept here; start a new chat to continue the work"
            .to_owned()
    })
}

/// A name for a set of sentences, the same for the same sentences.
fn said_once(sentences: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for sentence in sentences {
        hasher.update(sentence.as_bytes());
        hasher.update([0]);
    }
    hasher
        .finalize()
        .iter()
        .take(8)
        .fold(String::new(), |mut name, byte| {
            use std::fmt::Write as _;
            let _ = write!(name, "{byte:02x}");
            name
        })
}

fn nanos_now() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos())
}

fn profile_error(error: ProfileError) -> WorkbenchShellError {
    match error {
        ProfileError::ProfileNotFound(_) => WorkbenchShellError::NotFound(error.to_string()),
        ProfileError::InvalidProfile(_) => WorkbenchShellError::Invalid(error.to_string()),
        other => WorkbenchShellError::Failed(other.to_string()),
    }
}

async fn abort_resolved_environment(
    environment: &ResolvedAgentEnvironment,
    primary: WorkbenchShellError,
) -> WorkbenchShellError {
    let ResolvedAgentEnvironment::Prepared { transport, .. } = environment else {
        return primary;
    };
    match transport.abort_prepared().await {
        Ok(_) => primary,
        Err(cleanup) => WorkbenchShellError::Failed(format!(
            "{primary}; prepared environment cleanup also failed: {cleanup}"
        )),
    }
}

impl WorkbenchShellState {
    /// Open the shell over an existing profile inventory and route ledger.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the inventory cannot be opened.
    pub fn open(
        inventory_root: &Path,
        ledger_path: &Path,
        operation_timeout: Duration,
        resolver: impl Fn(&PersonalAgentProfile) -> Result<ResolvedDirectAgentConnection, String>
        + Send
        + Sync
        + 'static,
    ) -> Result<Self, WorkbenchShellError> {
        Self::open_with_environment(
            inventory_root,
            ledger_path,
            operation_timeout,
            move |profile| resolver(profile).map(Into::into),
        )
    }

    /// Open the shell with a resolver that may supply either the enforced
    /// direct process path or an already prepared exact environment transport.
    /// Environment preparation and secret materialization remain resolver
    /// responsibilities; Workbench validates identity and owns terminal use.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the inventory cannot be opened.
    pub fn open_with_environment(
        inventory_root: &Path,
        ledger_path: &Path,
        operation_timeout: Duration,
        resolver: impl Fn(&PersonalAgentProfile) -> Result<ResolvedAgentConnection, String>
        + Send
        + Sync
        + 'static,
    ) -> Result<Self, WorkbenchShellError> {
        let inventory = PersonalAgentProfileStore::open(inventory_root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let content = WorkbenchContentStore::open(&ledger_path.with_extension("content"))?;
        Ok(Self {
            inventory,
            ledger_path: ledger_path.to_path_buf(),
            operation_timeout,
            resolver: Box::new(resolver),
            connections: tokio::sync::Mutex::new(BTreeMap::new()),
            next_connection: AtomicU64::new(1),
            content,
            sandbox: std::sync::OnceLock::new(),
            session_token: std::sync::OnceLock::new(),
            served_at: std::sync::OnceLock::new(),
            built_under: std::sync::OnceLock::new(),
            called: std::sync::OnceLock::new(),
            apps_bundle: std::sync::OnceLock::new(),
            mcp_observer: std::sync::OnceLock::new(),
            onboarding: std::sync::OnceLock::new(),
            rediscover: std::sync::OnceLock::new(),
            acp_registry_index: std::sync::OnceLock::new(),
            server_apps: server_apps::ServerApps::new(),
            mcp_catalogue: Arc::new(std::sync::OnceLock::new()),
            model_providers: std::sync::OnceLock::new(),
            provider_keys: std::sync::OnceLock::new(),
            machine_root: std::sync::OnceLock::new(),
            machine_look: std::sync::RwLock::new(None),
            container_setup: std::sync::Mutex::new(None),
            installed_root: std::sync::OnceLock::new(),
            store: std::sync::OnceLock::new(),
            host_of_the_store: Arc::new(std::sync::OnceLock::new()),
            time_tools: std::sync::OnceLock::new(),
            timekeeper: std::sync::OnceLock::new(),
            removed_root: std::sync::OnceLock::new(),
            keepers: std::sync::OnceLock::new(),
            channels: std::sync::OnceLock::new(),
            app_hosted_at: std::sync::OnceLock::new(),
            tunnel: std::sync::Mutex::new(None),
            tunnel_root: std::sync::OnceLock::new(),
            keep_time_command: std::sync::OnceLock::new(),
            terminals: Arc::new(Terminals::default()),
            chat_runtime: runtime::ChatRuntime::default(),
        })
    }

    /// What installing `agent_id` would fetch, so the person consents to an
    /// exact plan instead of to a word.
    ///
    /// # Errors
    ///
    /// Returns the supply diagnostic when the registry cannot be resolved.
    pub fn agent_install_plan(
        &self,
        agent_id: &str,
    ) -> Result<crate::InstallPlan, WorkbenchShellError> {
        crate::install_plan_at(agent_id, &self.acp_registry_index()).map_err(|error| {
            // The registry is a network dependency of this machine, not a
            // fault in the request: say which host, so the person knows
            // whether to fix their network or install the agent themselves.
            WorkbenchShellError::Invalid(format!(
                "the agent registry ({}) could not be read: {error}. Point {} at one this machine can reach, or install the agent yourself and it appears here.",
                self.acp_registry_index(),
                crate::ACP_REGISTRY_INDEX_VAR
            ))
        })
    }

    /// Name the ACP registry index this product reads. A builder names it;
    /// a product that was not told one reads the crate's default.
    ///
    /// # Errors
    ///
    /// Conflict when a different index was already named.
    pub fn set_acp_registry_index(&self, index: &str) -> Result<(), WorkbenchShellError> {
        let index = index.trim().to_owned();
        if index.is_empty() {
            return Err(WorkbenchShellError::Invalid(
                "the agent registry index cannot be empty".into(),
            ));
        }
        match self.acp_registry_index.set(index.clone()) {
            Ok(()) => Ok(()),
            Err(_) if self.acp_registry_index.get() == Some(&index) => Ok(()),
            Err(_) => Err(WorkbenchShellError::Conflict(format!(
                "this product already reads the agent registry at {}",
                self.acp_registry_index.get().map_or("", String::as_str)
            ))),
        }
    }

    /// The ACP registry index this product reads.
    #[must_use]
    pub fn acp_registry_index(&self) -> String {
        self.acp_registry_index
            .get()
            .cloned()
            .unwrap_or_else(crate::acp_registry_index)
    }

    /// Install the agent the person chose, against the exact plan they saw.
    ///
    /// The click is the consent, and `plan_id` is what it consented to: a plan
    /// that moved since it was shown is refused rather than silently applied.
    ///
    /// # Errors
    ///
    /// Refuses a plan mismatch, a missing node runtime for an npx
    /// distribution, or any supply failure.
    pub fn install_agent(
        &self,
        agent_id: &str,
        plan_id: &str,
    ) -> Result<crate::InstallReceipt, WorkbenchShellError> {
        let plan = self.agent_install_plan(agent_id)?;
        if plan.plan_id != plan_id {
            return Err(WorkbenchShellError::Conflict(format!(
                "the install plan changed since it was shown ({} now); read it again",
                plan.plan_id
            )));
        }
        let node = if matches!(plan.distribution, crate::RegistryDistribution::Npx { .. }) {
            Some(which_node().ok_or_else(|| {
                WorkbenchShellError::Invalid(
                    "this distribution runs through npx and node is not on PATH".into(),
                )
            })?)
        } else {
            None
        };
        let installation = crate::install(&plan, true, self.installed_root()?, node.as_deref())
            .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?;
        // What is installed just changed, and the list this shell answers
        // from was made when the product started. Without this a person
        // presses Install, the install succeeds, and the product goes on
        // telling them the agent is not available on this host until they
        // restart it.
        self.rediscover_agents();
        Ok(installation)
    }

    /// Let this host install things: agents from the registry, tools packages
    /// declare. `root` is `<data root>/installed`; one place for all of it.
    ///
    /// # Errors
    ///
    /// Refuses a second root.
    pub fn enable_installs(&self, root: PathBuf) -> Result<(), WorkbenchShellError> {
        self.installed_root
            .set(root)
            .map_err(|_| WorkbenchShellError::Conflict("installs already enabled".into()))
    }

    fn installed_root(&self) -> Result<&Path, WorkbenchShellError> {
        self.installed_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| WorkbenchShellError::NotFound("this host installs nothing".into()))
    }

    /// Everything this product has installed, of every kind, by receipt.
    ///
    /// # Errors
    ///
    /// Not found when this host installs nothing.
    pub fn installs(&self) -> Result<Vec<crate::InstallReceipt>, WorkbenchShellError> {
        Ok(crate::all_receipts(self.installed_root()?))
    }

    /// Terminate the project clients and await their child boundaries.
    ///
    /// # Errors
    ///
    /// Returns the first cleanup failure.
    pub async fn shutdown_server_apps(&self) -> Result<(), String> {
        self.close_terminals().await;
        self.server_apps.shutdown().await
    }

    /// Ids of the open native connections (a test oracle for "no agent").
    pub async fn active_connections(&self) -> Vec<String> {
        self.connections.lock().await.keys().cloned().collect()
    }

    async fn append_context_event(
        &self,
        connection: &WorkbenchConnection,
        event: u64,
        kind: &str,
        payload: Value,
    ) -> Result<(), WorkbenchShellError> {
        let route = connection.route_id.clone();
        // In the connection's own namespace, like every other event on this
        // route. A route's lane is read back by asking which connection each
        // event belongs to, and an event id of another shape is a connection
        // nobody opened - which is what `a_real_browser_runs_the_whole_shell_
        // acceptance_on_the_fixture_profile` counts and refuses.
        let event_id = format!(
            "{}:host:{event}",
            connection.artifact_projector.event_namespace
        );
        let kind = kind.to_owned();
        self.with_ledger(move |ledger| {
            ledger.append_event(&route, &event_id, &kind, SurfaceEventSource::Host, &payload)
        })
        .await
        .map(|_| ())
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Enable explicit first-run creation of local direct-host profiles.
    /// This does not install, verify or start an agent and stores no secret.
    ///
    /// # Errors
    ///
    /// Returns an error when the roots cannot be created/canonicalized or the
    /// configuration was already installed on this state.
    pub fn enable_local_onboarding(
        &self,
        agents: Vec<WorkbenchAgentOption>,
        workspaces_root: &Path,
        agent_homes_root: &Path,
    ) -> Result<(), WorkbenchShellError> {
        let mut indexed_agents = BTreeMap::new();
        for agent in agents {
            crate::profile::validate_id("onboarding agent id", &agent.agent_id)
                .map_err(WorkbenchShellError::Invalid)?;
            let agent_id = agent.agent_id.clone();
            if indexed_agents.insert(agent_id.clone(), agent).is_some() {
                return Err(WorkbenchShellError::Invalid(format!(
                    "duplicate onboarding agent {agent_id}"
                )));
            }
        }
        std::fs::create_dir_all(workspaces_root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        std::fs::create_dir_all(agent_homes_root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let workspaces_root = std::fs::canonicalize(workspaces_root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let agent_homes_root = std::fs::canonicalize(agent_homes_root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.onboarding
            .set(LocalOnboarding {
                agents: std::sync::RwLock::new(indexed_agents),
                workspaces_root,
                agent_homes_root,
            })
            .map_err(|_| WorkbenchShellError::Conflict("onboarding is already configured".into()))
    }

    /// Tell this shell how to find out what is installed, so it can ask again
    /// after an install instead of answering from what was true at startup.
    ///
    /// Optional: a shell without it still works and still lists what it was
    /// given, which is what the in-process gates rely on.
    pub fn set_agent_discovery<F>(&self, rediscover: F)
    where
        F: Fn() -> Vec<WorkbenchAgentOption> + Send + Sync + 'static,
    {
        let _ = self.rediscover.set(Box::new(rediscover));
    }

    /// Ask discovery again and replace what this shell lists.
    ///
    /// Silent when no discovery was configured: there is nothing to ask.
    fn rediscover_agents(&self) {
        let (Some(onboarding), Some(rediscover)) = (self.onboarding.get(), self.rediscover.get())
        else {
            return;
        };
        let found = rediscover();
        let mut indexed = BTreeMap::new();
        for agent in found {
            if crate::profile::validate_id("onboarding agent id", &agent.agent_id).is_err() {
                continue;
            }
            indexed.insert(agent.agent_id.clone(), agent);
        }
        if let Ok(mut agents) = onboarding.agents.write() {
            *agents = indexed;
        }
    }

    #[must_use]
    pub fn onboarding(&self) -> WorkbenchOnboarding {
        self.onboarding.get().map_or(
            WorkbenchOnboarding {
                enabled: false,
                agents: Vec::new(),
            },
            |onboarding| WorkbenchOnboarding {
                enabled: true,
                agents: onboarding
                    .agents
                    .read()
                    .map(|agents| agents.values().cloned().collect())
                    .unwrap_or_default(),
            },
        )
    }

    /// Create the minimal durable profile selected in first-run UI. This is a
    /// create-only mutation and deliberately attaches neither Cycle nor MCP.
    ///
    /// A configured agent can hold more than one profile. `profile_id` is the
    /// name the person gives this one; without it the profile is named after
    /// the agent, which is what first run does when there is nothing to tell
    /// apart yet. The workspace and the native home are the profile's, not the
    /// agent's, so two people working the same installed agent do not write
    /// into each other's files.
    ///
    /// # Errors
    ///
    /// Fails for disabled onboarding, unknown/unavailable agents, unsafe
    /// paths, or an existing profile identity.
    pub fn create_local_profile(
        &self,
        agent_id: &str,
        profile_id: Option<&str>,
        setup: Option<crate::AgentSetup>,
    ) -> Result<PersonalAgentProfile, WorkbenchShellError> {
        let onboarding = self
            .onboarding
            .get()
            .ok_or_else(|| WorkbenchShellError::NotFound("local onboarding is disabled".into()))?;
        let available = onboarding
            .agents
            .read()
            .ok()
            .and_then(|agents| agents.get(agent_id).map(|agent| agent.available))
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown agent {agent_id}")))?;
        if !available {
            return Err(WorkbenchShellError::Conflict(format!(
                "agent {agent_id} is not available on this host"
            )));
        }
        let profile_id = profile_id.unwrap_or(agent_id);
        crate::profile::validate_id("profile_id", profile_id)
            .map_err(WorkbenchShellError::Invalid)?;
        let workspace = onboarding.workspaces_root.join(profile_id);
        let agent_home = onboarding.agent_homes_root.join(profile_id);
        std::fs::create_dir_all(&workspace)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        std::fs::create_dir_all(&agent_home)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let workspace = std::fs::canonicalize(&workspace)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let agent_home = std::fs::canonicalize(&agent_home)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if !workspace.starts_with(&onboarding.workspaces_root)
            || !agent_home.starts_with(&onboarding.agent_homes_root)
        {
            return Err(WorkbenchShellError::Conflict(
                "local profile directory escaped its configured root".into(),
            ));
        }
        let profile = PersonalAgentProfile::new(
            profile_id,
            agent_id,
            "direct-host-distribution",
            crate::THIS_MACHINE,
            crate::ASK_EVERY_TIME,
            &workspace,
            &agent_home,
            Vec::new(),
            Vec::new(),
        )
        .and_then(|profile| match setup {
            Some(setup) => profile.with_setup(setup),
            None => Ok(profile),
        })
        .map_err(|error| match error {
            ProfileError::InvalidProfile(_) => WorkbenchShellError::Invalid(error.to_string()),
            _ => WorkbenchShellError::Failed(error.to_string()),
        })?;
        self.check_setup(&profile.setup())?;
        self.inventory
            .create(&profile)
            .map_err(|error| match error {
                ProfileError::ProfileExists(_) => WorkbenchShellError::Conflict(error.to_string()),
                _ => WorkbenchShellError::Failed(error.to_string()),
            })?;
        Ok(profile)
    }

    /// List the durable profiles (non-secret by construction).
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the inventory fails closed.
    /// The servers an agent's session is handed, by name: what its profile
    /// names and is declared, and what the product this harness is built
    /// into resolves for it.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile or an agent
    /// that cannot be resolved.
    pub fn servers_of(&self, profile_id: &str) -> Result<Vec<String>, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let connection = (self.resolver)(&profile).map_err(WorkbenchShellError::Failed)?;
        Ok(connection
            .mcp_servers
            .iter()
            .filter_map(|server| match server {
                McpServer::Stdio(stdio) => Some(stdio.name.clone()),
                McpServer::Http(http) => Some(http.name.clone()),
                McpServer::Sse(sse) => Some(sse.name.clone()),
                _ => None,
            })
            .collect())
    }

    /// What the profile's agent advertises at `initialize`, authentication
    /// methods included, without opening a session. The surface asks this
    /// before offering to start, so a person is asked for a key up front
    /// rather than told about it by a failed connection.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile, an agent that
    /// cannot be launched, or a handshake that fails or times out.
    pub async fn profile_handshake(
        &self,
        profile_id: &str,
    ) -> Result<crate::AcpHandshake, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let connection = (self.resolver)(&profile).map_err(WorkbenchShellError::Failed)?;
        let expected = crate::builtin_catalog()
            .into_iter()
            .find(|entry| entry.id == profile.agent_id)
            .and_then(|entry| entry.expected_agent_name);
        // Ask the agent where the agent is. A prepared environment's launch
        // command names a path inside it, so starting that path on this
        // machine asks nothing and fails with "no such file" on a path that
        // plainly exists in the environment the profile chose.
        let handshake = match &connection.environment {
            ResolvedAgentEnvironment::Direct => {
                crate::verify_launch(
                    &connection.launch,
                    &connection.agent_executable,
                    self.operation_timeout,
                    expected,
                )
                .await
            }
            ResolvedAgentEnvironment::Prepared { transport, .. } => {
                crate::verify_transport((**transport).clone(), self.operation_timeout, expected)
                    .await
            }
        };
        // A prepared environment was leased for this handshake alone and is
        // released the same way an aborted open releases it - whether the
        // handshake answered or failed, because a lease released only on
        // success is a container left behind by every failure.
        let _ = abort_resolved_environment(
            &connection.environment,
            WorkbenchShellError::Failed("handshake only".into()),
        )
        .await;
        handshake.map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// What a profile holds and may hold; values never leave the host.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile.
    pub fn profile_secrets(&self, profile_id: &str) -> Result<ProfileSecrets, WorkbenchShellError> {
        let secrets = self
            .inventory
            .secret_entries(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        Ok(ProfileSecrets {
            types: crate::secret_types(),
            secrets,
        })
    }

    /// Store one secret of a profile.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile, an unknown
    /// type, or a name that is not an environment variable name.
    pub fn set_profile_secret(
        &self,
        profile_id: &str,
        type_id: &str,
        label: &str,
        name: Option<&str>,
        value: &str,
    ) -> Result<ProfileSecrets, WorkbenchShellError> {
        let kind = crate::secret_types()
            .iter()
            .find(|kind| kind.type_id == type_id)
            .ok_or_else(|| {
                WorkbenchShellError::Invalid(format!("unknown secret type {type_id:?}"))
            })?;
        let name = match (kind.env_var, name) {
            (_, Some(given)) if !given.is_empty() => given,
            (Some(conventional), _) => conventional,
            (None, _) => {
                return Err(WorkbenchShellError::Invalid(
                    "an environment variable needs a name".into(),
                ));
            }
        };
        let label = if label.is_empty() { kind.label } else { label };
        self.inventory
            .set_secret(profile_id, type_id, label, name, value)
            .map_err(profile_error)?;
        self.profile_secrets(profile_id)
    }

    /// Forget one secret of a profile.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile.
    pub fn remove_profile_secret(
        &self,
        profile_id: &str,
        name: &str,
    ) -> Result<ProfileSecrets, WorkbenchShellError> {
        self.inventory
            .remove_secret(profile_id, name)
            .map_err(profile_error)?;
        self.profile_secrets(profile_id)
    }

    /// What this profile's agent has been handed and what it has handed back:
    /// the files in `inbox` and `outbox` inside its own workspace.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile or a workspace
    /// that exists and cannot be read.
    pub async fn profile_files(
        &self,
        profile_id: &str,
    ) -> Result<Vec<crate::HandedFile>, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        crate::workbench_files::list(&profile.workspace).await
    }

    /// One of those files, with the media type to serve it as.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown profile, an area that is
    /// neither directory, a name that is a path, or a file that is not there.
    pub async fn profile_file(
        &self,
        profile_id: &str,
        area: &str,
        name: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        crate::workbench_files::read(&profile.workspace, area, name).await
    }

    /// Every durable profile of this inventory.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the inventory cannot be listed.
    pub fn profiles(&self) -> Result<Vec<PersonalAgentProfile>, WorkbenchShellError> {
        self.inventory
            .list()
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Open a new/load/resume connection for one exact profile and attach the
    /// durable projector. Returns `(connection_id, route_id, session_id)`.
    ///
    /// # Errors
    ///
    /// Fails closed on unknown profile/route, binding drift, an unresolvable
    /// attachment, or a session that never reached ready.
    #[allow(
        clippy::too_many_lines,
        reason = "one linear open transaction keeps resolve, start, bind and projection ordering auditable"
    )]
    pub async fn open_connection(
        &self,
        profile_id: &str,
        mode: ShellConnectionMode,
        route_id: Option<String>,
    ) -> Result<(String, String, String), WorkbenchShellError> {
        self.open_connection_with_id(profile_id, mode, route_id, None, None, None)
            .await
    }

    /// Open while honoring an optional surface-generated ephemeral connection
    /// handle. This lets the surface poll request-scoped ACP elicitation during
    /// authentication, before the native session id exists.
    ///
    /// `file_callbacks` names the exact ACP `fs/*` methods the surface opening
    /// this connection can carry out. A surface that owns no files - the
    /// Workbench's browser tab - passes `None` and the agent is told so at the
    /// handshake. What a surface never names is the directory those calls are
    /// bounded to: that is the profile's own workspace, taken here, so asking
    /// cannot widen it.
    ///
    /// # Errors
    ///
    /// Has the same fail-closed conditions as [`Self::open_connection`], plus
    /// malformed or already-active requested handles.
    pub async fn open_connection_with_id(
        &self,
        profile_id: &str,
        mode: ShellConnectionMode,
        route_id: Option<String>,
        requested_connection_id: Option<String>,
        auth_method_id: Option<String>,
        file_callbacks: Option<FileSystemCapabilities>,
    ) -> Result<(String, String, String), WorkbenchShellError> {
        self.open_as(Opening {
            profile_id: profile_id.to_owned(),
            mode,
            route_id,
            requested_connection_id,
            auth_method_id,
            file_callbacks,
            place: None,
        })
        .await
    }

    /// Open a connection, in a chat when one is named: a new session there
    /// becomes the agent's current one in that chat.
    #[allow(
        clippy::too_many_lines,
        reason = "one linear open transaction keeps resolve, early control registration, start, bind and projection ordering auditable"
    )]
    pub(crate) async fn open_as(
        &self,
        opening: Opening,
    ) -> Result<(String, String, String), WorkbenchShellError> {
        let Opening {
            profile_id,
            mode,
            route_id,
            requested_connection_id,
            auth_method_id,
            file_callbacks,
            place,
        } = opening;
        // The chat the session is asked for, for what is handed to it.
        let asked_in = place.as_ref().map(|place| place.chat_id.clone());
        let profile_id = profile_id.as_str();
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let mut setup_changed = Vec::new();
        let (route_id, start) = match mode {
            // A new session gets a new route, always. A route is bound to the
            // exact native session it was opened with, so reusing one for a
            // second session could only ever end as binding drift - which is
            // what a person met the second time they ever pressed Start.
            ShellConnectionMode::New => {
                if let Some(named) = route_id {
                    let known = self
                        .with_ledger({
                            let named = named.clone();
                            move |ledger| ledger.route(&named)
                        })
                        .await;
                    if known.is_ok() {
                        return Err(WorkbenchShellError::Conflict(format!(
                            "session {named} already exists: resume it, or start a new one without naming it"
                        )));
                    }
                    (named, NativeSessionStart::New)
                } else {
                    (
                        format!("route-{}-{}", std::process::id(), nanos_now()),
                        NativeSessionStart::New,
                    )
                }
            }
            ShellConnectionMode::Load | ShellConnectionMode::Resume => {
                let route_id = route_id.ok_or_else(|| {
                    WorkbenchShellError::Invalid("load/resume requires route_id".into())
                })?;
                let stored = self
                    .with_ledger({
                        let route_id = route_id.clone();
                        move |ledger| ledger.route(&route_id)
                    })
                    .await
                    .map_err(|error| match error {
                        crate::RoutingError::RouteNotFound(route) => {
                            WorkbenchShellError::NotFound(format!("unknown route {route}"))
                        }
                        other => WorkbenchShellError::Failed(other.to_string()),
                    })?;
                // A chat is its agent's and leads to one native session;
                // that is what is proved here. Where the agent works, where
                // it runs and what it attaches are how the chat began. A
                // person changes them and goes on talking, so a difference
                // there is a thing to record, never a reason to refuse.
                let identity = crate::RouteIdentity {
                    route_id: route_id.clone(),
                    agent_id: profile.agent_id.clone(),
                    agent_profile_id: profile.profile_id.clone(),
                    native_session_id: stored.native_session_id.clone(),
                };
                self.with_ledger(move |ledger| ledger.require_identity(&identity))
                    .await
                    .map_err(|_| {
                        WorkbenchShellError::Conflict(format!(
                            "this chat belongs to another agent, not to {}",
                            profile.profile_id
                        ))
                    })?;
                let current = SessionRouteBinding::new(
                    route_id.clone(),
                    profile.agent_id.clone(),
                    profile.profile_id.clone(),
                    stored.native_session_id.clone(),
                    profile.environment_profile_id.clone(),
                    &profile.workspace,
                    profile.attachments.clone(),
                )
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
                setup_changed = configuration_differences(&stored, &current);
                let session_id = stored.native_session_id;
                (
                    route_id,
                    match mode {
                        ShellConnectionMode::Load => NativeSessionStart::Load { session_id },
                        _ => NativeSessionStart::Continue { session_id },
                    },
                )
            }
        };
        let connection_number = self.next_connection.fetch_add(1, Ordering::SeqCst);
        // A connection's id is in the address of every App it opens, on the
        // sandbox origin, where an App View has an opaque origin and so
        // cannot be told apart from any other page by where it came from.
        // While the id was `c1`, `c2`, a page that found that port could name
        // an open App's upload and blob routes by guessing. The counter stays
        // for reading a log in order; what makes the address a capability is
        // what follows it.
        let connection_id = if let Some(requested) = requested_connection_id {
            requested
        } else {
            // Refuse rather than fall back to the guessable name: a door
            // that quietly opens when the lock cannot be made is the lock
            // nobody knows is missing.
            let secret = mint_session_token().map_err(|error| {
                WorkbenchShellError::Failed(format!(
                    "this machine would not give random bytes for a connection address: {error}"
                ))
            })?;
            format!("c{connection_number}-{}", &secret[..22])
        };
        if connection_id.is_empty()
            || connection_id.len() > 96
            || !connection_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(WorkbenchShellError::Invalid(
                "connection_id must contain 1-96 ASCII letters, digits, '-' or '_'".into(),
            ));
        }
        if self.connections.lock().await.contains_key(&connection_id) {
            return Err(WorkbenchShellError::Conflict(format!(
                "connection {connection_id} is already active"
            )));
        }
        let connection = (self.resolver)(&profile).map_err(WorkbenchShellError::Failed)?;
        // The profile's setup - its role, its skills, its model - goes where
        // the agent reads it before the agent starts: files in the working
        // directory, which both a direct process and a container see, and
        // variables for the direct process below. Written at every open, so
        // a role edited between two sessions reaches the second one.
        let setup = {
            let provider = match &profile.model_provider {
                Some(id) => Some(self.model_provider_book()?.get(id)?),
                None => None,
            };
            let standing = self.standing_of(&profile.profile_id).await?;
            let materialised = crate::agent_setup::materialise_profile(
                &profile,
                provider.as_ref(),
                Some(&standing),
            )
            .map_err(WorkbenchShellError::Failed)?;
            crate::agent_setup::write_materialised(&materialised)
                .map_err(WorkbenchShellError::Failed)?;
            materialised
        };
        let (environment_lease, environment_transport) = match &connection.environment {
            ResolvedAgentEnvironment::Direct => {
                let lease = direct_environment_lease(
                    format!("workbench-direct-{}-{}", std::process::id(), nanos_now()),
                    &EnvironmentRequirements::new(&profile.workspace),
                )
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
                (lease, None)
            }
            ResolvedAgentEnvironment::Prepared {
                environment_profile_id,
                lease,
                transport,
            } => {
                let mismatch = if environment_profile_id != &profile.environment_profile_id {
                    Some("resolved environment profile differs from the durable profile")
                } else if lease.backend_id == "direct-process" {
                    Some("prepared environment cannot use the direct-process backend")
                } else if lease.workspace != profile.workspace {
                    Some("resolved environment workspace differs from the durable profile")
                } else if !transport.matches_lease(lease) {
                    Some("resolved environment transport identity differs from its lease")
                } else {
                    None
                };
                if let Some(message) = mismatch {
                    return Err(abort_resolved_environment(
                        &connection.environment,
                        WorkbenchShellError::Conflict(message.into()),
                    )
                    .await);
                }
                (lease.as_ref().clone(), Some(transport.as_ref().clone()))
            }
        };
        // Pair every durable attachment with its connection-local
        // declaration. One that has none - a server that was forgotten, or
        // that another computer had - is left out and said: the agent reaches
        // less than its profile names, never more, and a missing server is no
        // reason for a person to be unable to talk to their agent at all.
        let mut resolved_attachments = Vec::with_capacity(profile.attachments.len());
        let mut unavailable = Vec::new();
        for attachment in &profile.attachments {
            let server = connection.mcp_servers.iter().find(|server| match server {
                McpServer::Stdio(stdio) => stdio.name == attachment.server_name,
                McpServer::Http(http) => http.name == attachment.server_name,
                McpServer::Sse(sse) => sse.name == attachment.server_name,
                _ => false,
            });
            let Some(server) = server else {
                unavailable.push(attachment.server_name.clone());
                continue;
            };
            resolved_attachments.push(ResolvedMcpAttachment {
                binding: attachment.clone(),
                server: server.clone(),
            });
        }
        let mut apps = None;
        let mut observation = None;
        let mut app_links = BTreeMap::new();
        let mut session_mcp_servers = connection.mcp_servers.clone();
        if Self::apps_enabled()
            && let Some(observer_command) = self.mcp_observer.get()
        {
            let discovered =
                workbench_apps::discover(&resolved_attachments, Some(&environment_lease.workspace))
                    .await;
            app_links = discovered.model_app_links();
            let observed_servers = app_links
                .keys()
                .map(|(server, _tool)| server.clone())
                .collect::<BTreeSet<_>>();
            if !observed_servers.is_empty() {
                let ingress = match ObservationIngress::bind(observed_servers.clone()).await {
                    Ok(ingress) => ingress,
                    Err(error) => {
                        return Err(abort_resolved_environment(
                            &connection.environment,
                            WorkbenchShellError::Failed(error),
                        )
                        .await);
                    }
                };
                for server in &mut session_mcp_servers {
                    if let McpServer::Stdio(stdio) = server
                        && observed_servers.contains(&stdio.name)
                    {
                        *server = match ingress.wrap(stdio, observer_command) {
                            Ok(server) => server,
                            Err(error) => {
                                return Err(abort_resolved_environment(
                                    &connection.environment,
                                    WorkbenchShellError::Failed(error),
                                )
                                .await);
                            }
                        };
                    }
                }
                observation = Some(ingress);
            }
            apps = Some(discovered);
        }
        let control = NativeSessionControl::new();
        let mut surface_events = match control.take_surface_events() {
            Ok(events) => events,
            Err(error) => {
                return Err(abort_resolved_environment(
                    &connection.environment,
                    WorkbenchShellError::Failed(error.to_string()),
                )
                .await);
            }
        };
        let mut options = NativeSessionOptions::interactive(self.operation_timeout);
        options.control = Some(control.clone());
        // How this agent's requests are answered is the person's choice, kept
        // on the profile. It used to be Surface for everybody, which is right
        // while somebody is watching and useless when nobody is: an agent left
        // alone stopped at its first question and waited. The boundary a
        // choice names is built from the profile's own workspace and
        // attachments, so picking one cannot widen it.
        let permission_policy = crate::permission_policy(
            &profile.permission_profile_id,
            &profile.workspace,
            resolved_attachments
                .iter()
                .map(|attachment| attachment.binding.server_name.clone())
                .collect(),
        );
        options.permission_policy = match permission_policy {
            Ok(policy) => policy,
            Err(error) => {
                return Err(abort_resolved_environment(
                    &connection.environment,
                    WorkbenchShellError::Invalid(error),
                )
                .await);
            }
        };
        options.elicitation_capabilities = Some(
            ElicitationCapabilities::new()
                .form(ElicitationFormCapabilities::new())
                .url(ElicitationUrlCapabilities::new()),
        );
        // The two halves of a file callback: what the surface said it can do,
        // and where. Only the first half came from the surface.
        options.file_callbacks = file_callbacks.map(|capabilities| crate::FileCallbacks {
            capabilities,
            boundary: profile.workspace.clone(),
        });
        // Its own schedules, in the chat the session is opened for. The
        // command runs where the engine runs, so only on this machine.
        if let (Some((executable, prefix)), Some(chat_id)) = (self.time_tools.get(), &asked_in)
            && matches!(connection.environment, ResolvedAgentEnvironment::Direct)
        {
            let profile_id = profile.profile_id.clone();
            let agent = self
                .with_ledger(move |ledger| ledger.agent_of_profile(&profile_id))
                .await;
            if let Ok(agent) = agent {
                let mut args = prefix.clone();
                args.extend([
                    "--ledger".to_owned(),
                    self.ledger_path.display().to_string(),
                    "--agent".to_owned(),
                    agent.participant_id,
                    "--chat".to_owned(),
                    chat_id.clone(),
                ]);
                session_mcp_servers.push(McpServer::Stdio(
                    McpServerStdio::new(crate::TIME_TOOLS, executable.clone()).args(args),
                ));
            }
        }
        options.mcp_servers = session_mcp_servers;
        options.start = start;
        let workspace = profile.workspace.clone();
        let launch = connection.launch.clone();
        let agent_executable = connection.agent_executable.clone();
        options.environment_lease = Some(environment_lease.clone());
        // A prepared environment carries its credentials as runtime secrets
        // and refuses process overrides; the direct process gets the profile's
        // bindings here, on top of the filtered bootstrap environment.
        if matches!(connection.environment, ResolvedAgentEnvironment::Direct) {
            let secrets = self.keys_of(&profile)?;
            // The model and the provider's address first, the credentials
            // over them: a key is never shadowed by a setup variable.
            let mut overrides = setup.environment.clone();
            overrides.extend(credential_environment(&profile, secrets.clone())?);
            options.process_environment_overrides = overrides;
            // Whether this agent may run a command on its own is the same
            // choice as whether it may edit a file on its own, and the person
            // already made it: the profile that works alone inside its
            // workspace runs commands there without being asked, and the one
            // that asks every time is asked about each command before it
            // starts, on the same lane its tool calls are asked on.
            //
            // Only a direct environment. A session prepared elsewhere - a
            // container - must not have its commands run on this machine, and
            // that is the same line `process_environment_overrides` draws
            // right above.
            let ask = if profile.permission_profile_id
                == crate::permission_profile::INSIDE_ITS_WORKSPACE
            {
                crate::AskBeforeRunning::No
            } else {
                crate::AskBeforeRunning::EveryTime
            };
            options.terminal_callbacks = Some(crate::TerminalCallbacks {
                terminals: Arc::clone(&self.terminals),
                profile: profile.clone(),
                secrets,
                ask,
            });
        }
        options.auth_method_id = auth_method_id;
        options.environment_transport = environment_transport;
        let namespace = format!("conn-{connection_number}-{}", nanos_now());
        let observation_namespace = format!("observer-{connection_number}-{}", nanos_now());
        let observation = observation.map(|ingress| {
            ingress.start(
                app_links,
                self.ledger_path.clone(),
                route_id.clone(),
                observation_namespace,
            )
        });
        let artifact_projector = WorkbenchArtifactProjector {
            content: self.content.clone(),
            ledger_path: self.ledger_path.clone(),
            route_id: route_id.clone(),
            agent_id: profile.agent_id.clone(),
            environment_lease,
            event_namespace: namespace.clone(),
            next_event: Arc::new(AtomicU64::new(1)),
            state: Arc::new(tokio::sync::Mutex::new(ArtifactProjectionState::default())),
            changed: Arc::new(tokio::sync::Notify::new()),
        };
        let registered = {
            let mut connections = self.connections.lock().await;
            if connections.contains_key(&connection_id) {
                let error = WorkbenchShellError::Conflict(format!(
                    "connection {connection_id} is already active"
                ));
                drop(connections);
                return Err(abort_resolved_environment(&connection.environment, error).await);
            }
            let runner = tokio::spawn(async move {
                run_interactive_native_session(&launch, &agent_executable, &workspace, &options)
                    .await
            });
            let registered = Arc::new(WorkbenchConnection {
                control,
                route_id: route_id.clone(),
                artifact_projector,
                runner: tokio::sync::Mutex::new(Some(runner)),
                projection: tokio::sync::Mutex::new(None),
                artifact_projection: tokio::sync::Mutex::new(None),
                attachments: resolved_attachments,
                apps: tokio::sync::Mutex::new(apps),
                observation: tokio::sync::Mutex::new(observation),
                observed_apps: tokio::sync::Mutex::new(BTreeMap::new()),
                context_events: AtomicU64::new(1),
                setup_notes: tokio::sync::Mutex::new(
                    setup
                        .notes
                        .iter()
                        .cloned()
                        .chain(unavailable.iter().map(|name| {
                            format!(
                                "{name} is attached but not set up on this computer; the agent works without it"
                            )
                        }))
                        .collect(),
                ),
            });
            connections.insert(connection_id.clone(), Arc::clone(&registered));
            registered
        };
        let Ok(session_id) = registered.control.wait_until_ready().await else {
            // The control was registered before initialize specifically so a
            // browser could answer request-scoped elicitation. Once startup
            // fails, the ordinary finish path drains every child and removes
            // that provisional handle.
            let failure = match self.finish_connection(&connection_id, &registered).await {
                Ok(_) => "session finished before ready".to_owned(),
                Err(error) => error.to_string(),
            };
            // Nothing had read this lane yet: the projector below starts only
            // once a session exists, so a handshake the agent refuses used to
            // be dropped here together with the receiver, and the person was
            // left with "session finished before ready" over an empty route
            //. Whatever the handshake managed to say is still queued,
            // so it is read out now and carried in the failure instead.
            return Err(WorkbenchShellError::Failed(
                match handshake_refusal(&mut surface_events) {
                    Some(reason) => format!("{failure}: {reason}"),
                    None => cannot_go_on(&failure).unwrap_or(failure),
                },
            ));
        };
        // The model by the native road too: an agent that offers a model
        // choice on the session gets the profile's, when it lists it. Only a
        // value the agent advertised is ever sent - the control refuses any
        // other before the wire, and a refusal on the wire would end the
        // session. What happened is a sentence on the connection's status.
        if let Some(model) = profile.model.as_deref().filter(|model| !model.is_empty()) {
            let note = apply_profile_model(&registered.control, model).await;
            registered.setup_notes.lock().await.push(note);
        }
        let binding = match SessionRouteBinding::new(
            route_id.clone(),
            profile.agent_id.clone(),
            profile.profile_id.clone(),
            session_id.clone(),
            profile.environment_profile_id.clone(),
            &profile.workspace,
            profile.attachments.clone(),
        ) {
            Ok(binding) => binding,
            Err(error) => {
                let _ = registered.control.disconnect().await;
                let _ = self.finish_connection(&connection_id, &registered).await;
                return Err(WorkbenchShellError::Failed(error.to_string()));
            }
        };
        if let Err(error) = self
            .with_ledger(move |ledger| match place {
                Some(place) => ledger
                    .bind_route_in_chat(&binding, &place.chat_id, &place.why_the_last_ended)
                    .map(|_| ()),
                None => ledger.bind_route(&binding),
            })
            .await
        {
            let _ = registered.control.disconnect().await;
            let _ = self.finish_connection(&connection_id, &registered).await;
            return Err(WorkbenchShellError::Conflict(error.to_string()));
        }
        // What is different from how the chat began, and what the agent was
        // meant to reach and cannot: each said once per chat. The id is made
        // from what is said, so opening the chat again appends nothing.
        for (kind, said) in [
            ("host/setup_changed", &setup_changed),
            ("host/attachment_unavailable", &unavailable),
        ] {
            if said.is_empty() {
                continue;
            }
            let route = route_id.clone();
            let event_id = format!("{kind}:{}", said_once(said));
            let payload = json!({ "in_words": said });
            if let Err(error) = self
                .with_ledger(move |ledger| {
                    ledger.append_event(&route, &event_id, kind, SurfaceEventSource::Host, &payload)
                })
                .await
            {
                let _ = registered.control.disconnect().await;
                let _ = self.finish_connection(&connection_id, &registered).await;
                return Err(WorkbenchShellError::Failed(error.to_string()));
            }
        }
        let (output_tx, output_rx) = tokio::sync::mpsc::unbounded_channel();
        let projection = match project_native_session_events_with_output(
            &self.ledger_path,
            route_id.clone(),
            namespace,
            surface_events,
            output_tx,
        ) {
            Ok(projection) => projection,
            Err(error) => {
                let _ = registered.control.disconnect().await;
                let _ = self.finish_connection(&connection_id, &registered).await;
                return Err(WorkbenchShellError::Failed(error.to_string()));
            }
        };
        *registered.projection.lock().await = Some(projection);
        let artifact_projector = registered.artifact_projector.clone();
        *registered.artifact_projection.lock().await = Some(tokio::spawn(async move {
            artifact_projector.run(output_rx).await;
        }));
        Ok((connection_id, route_id, session_id))
    }

    async fn connection(
        &self,
        connection_id: &str,
    ) -> Result<Arc<WorkbenchConnection>, WorkbenchShellError> {
        self.connections
            .lock()
            .await
            .get(connection_id)
            .cloned()
            .ok_or_else(|| {
                WorkbenchShellError::NotFound(format!("unknown connection {connection_id}"))
            })
    }

    /// Who a profile's agent is in chats and whose it is, as the ledger
    /// knows them; both are made the first time they are asked for.
    async fn standing_of(
        &self,
        profile_id: &str,
    ) -> Result<crate::agent_setup::Standing, WorkbenchShellError> {
        let path = self.ledger_path.clone();
        let profile_id = profile_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut ledger = RoutingLedger::open(&path)?;
            let principal = ledger.owner()?;
            let agent = ledger.agent_of_profile(&profile_id)?;
            Ok::<_, crate::RoutingError>(crate::agent_setup::Standing {
                name: agent.name,
                handle: agent.handle,
                principal_name: principal.name,
                principal_handle: principal.handle,
            })
        })
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Submit the next exact ACP content turn and return its outcome.
    ///
    /// # Errors
    ///
    /// Propagates the driver failure (capability violation, timeout, finished
    /// connection) without retrying or rewriting content.
    pub async fn submit_prompt(
        &self,
        connection_id: &str,
        content: Vec<ContentBlock>,
    ) -> Result<NativeTurnOutcome, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .submit_prompt(content)
            .await
            .map_err(|error| match error {
                // Content the agent never said it could take: the turn is
                // rejected and the session is untouched, so this is a refusal
                // the caller can answer, not a failure it must recover from.
                SupplyError::PromptRefused(refusal) => WorkbenchShellError::Conflict(refusal),
                other => WorkbenchShellError::Failed(other.to_string()),
            })
    }

    async fn submit_workbench_prompt(
        &self,
        connection_id: &str,
        mut content: Vec<ContentBlock>,
        content_refs: Vec<String>,
        block: String,
    ) -> Result<WorkbenchPromptResponse, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let workspace = connection
            .artifact_projector
            .environment_lease
            .workspace
            .clone();
        for descriptor_id in content_refs {
            // A file a person hands over reaches the agent twice, because
            // agents differ in what they can take. As an ACP content block,
            // for one that reads the turn; and as a real file in the inbox
            // directory inside its own workspace, for one that only opens
            // paths. The turn names the path, so neither kind has to guess.
            let descriptor = self.content.load(&descriptor_id).await?;
            let bytes = self.content.bytes(&descriptor).await?;
            match crate::workbench_files::hand_over(&workspace, &descriptor.name, &bytes).await {
                Ok(path) => content.push(ContentBlock::ResourceLink(
                    agent_client_protocol::schema::v1::ResourceLink::new(
                        descriptor.name.clone(),
                        format!("file://{}", path.display()),
                    )
                    .mime_type(descriptor.media_type.clone()),
                )),
                // The inbox is a convenience, not the transport. A workspace
                // that cannot be written still gets the content block, and
                // the person is not stopped from talking to their agent.
                Err(refusal) => {
                    self.append_context_event(
                        &connection,
                        connection.context_events.fetch_add(1, Ordering::Relaxed),
                        "host/inbox_refused",
                        json!({ "name": descriptor.name, "why": refusal.to_string() }),
                    )
                    .await?;
                }
            }
            // The bytes themselves, for an agent that takes embedded content.
            // One that does not has the file in its inbox and the path in
            // this turn, so the attachment still reaches it - and refusing
            // the whole turn over a block the agent never needed would be the
            // product telling a person their own file is the problem.
            let embedded = self.content.content_block(&descriptor_id).await?;
            let takeable = connection
                .control
                .prompt_capabilities()
                .is_none_or(|capabilities| crate::agent_takes(&embedded, &capabilities));
            if takeable {
                content.push(embedded);
            } else {
                self.append_context_event(
                    &connection,
                    connection.context_events.fetch_add(1, Ordering::Relaxed),
                    "host/attachment_by_path",
                    json!({
                        "name": descriptor.name,
                        "why": "the agent takes no embedded content, so it was handed the file's path",
                    }),
                )
                .await?;
            }
        }
        // The host's block is the last thing of the turn, after what was
        // written and whatever rides along with it.
        content.push(ContentBlock::Text(
            agent_client_protocol::schema::v1::TextContent::new(block),
        ));
        let outcome = match connection.control.submit_prompt(content).await {
            Ok(outcome) => outcome,
            // Content the agent cannot take is this turn's problem, not the
            // session's: nothing was sent, the driver is still waiting for the
            // next command, and the person can say the same thing another way.
            // Ending the connection here used to await a runner that will
            // never finish, so attaching a file to an agent without embedded
            // context hung the turn for good instead of refusing it.
            Err(SupplyError::PromptRefused(refusal)) => {
                return Err(WorkbenchShellError::Conflict(refusal));
            }
            Err(error) => {
                let failure = error.to_string();
                // A failed ACP prompt terminates this runner. Drain both
                // projections so partial rich output is durable before the
                // HTTP error is returned and remove the dead connection.
                let _ = self.finish_connection(connection_id, &connection).await;
                return Err(WorkbenchShellError::Failed(failure));
            }
        };
        let capture = connection
            .artifact_projector
            .wait_turn(outcome.turn_index)
            .await;
        Ok(WorkbenchPromptResponse {
            stop_reason: outcome.stop_reason,
            control_outcome: outcome.control_outcome,
            reply_text: outcome.reply_text,
            artifacts: capture.artifacts,
            artifact_issues: capture.issues,
        })
    }

    /// Request cancellation of the active turn, if one is active right now.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection.
    pub async fn cancel(&self, connection_id: &str) -> Result<Value, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        Ok(match connection.control.cancel_active_turn() {
            Some(turn) => json!({
                "active_turn": {"session_id": turn.session_id, "turn_index": turn.turn_index},
            }),
            None => json!({ "active_turn": null }),
        })
    }

    /// Gracefully disconnect: the agent-owned session stays loadable/resumable.
    /// Awaits the runner and the projector so the terminal event is durable.
    ///
    /// # Errors
    ///
    /// Returns the session failure when the connection ended in an error.
    pub async fn disconnect(
        &self,
        connection_id: &str,
    ) -> Result<NativeSessionOutcome, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .disconnect()
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.finish_connection(connection_id, &connection).await
    }

    /// Capability-gated `session/close`. An unadvertised close is refused and
    /// the connection stays registered and usable - it never degrades to a
    /// silent disconnect.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] when the agent does not
    /// advertise close.
    pub async fn close(
        &self,
        connection_id: &str,
    ) -> Result<NativeSessionOutcome, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .close_session()
            .await
            .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))?;
        self.finish_connection(connection_id, &connection).await
    }

    async fn finish_connection(
        &self,
        connection_id: &str,
        connection: &WorkbenchConnection,
    ) -> Result<NativeSessionOutcome, WorkbenchShellError> {
        let runner = connection.runner.lock().await.take();
        let runner_result = match runner {
            Some(task) => task.await,
            None => {
                return Err(WorkbenchShellError::Conflict(
                    "connection already finished".into(),
                ));
            }
        };
        let projection_result = match connection.projection.lock().await.take() {
            Some(projection) => projection.finish().await,
            None => Ok(0),
        };
        let artifact_result = match connection.artifact_projection.lock().await.take() {
            Some(projection) => projection.await,
            None => Ok(()),
        };
        if let Some(observation) = connection.observation.lock().await.take() {
            observation.shutdown().await;
        }
        let apps_result = match connection.apps.lock().await.take() {
            Some(apps) => apps.shutdown().await,
            None => Ok(()),
        };
        self.connections.lock().await.remove(connection_id);
        let outcome = runner_result
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        projection_result.map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        artifact_result.map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        apps_result.map_err(WorkbenchShellError::Failed)?;
        Ok(outcome)
    }

    /// Long-poll the next unmodified ACP permission request. `None` after the
    /// wait means no request arrived (or the session finished).
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection.
    pub async fn next_permission(
        &self,
        connection_id: &str,
        wait: Duration,
    ) -> Result<Option<crate::NativePermissionRequest>, WorkbenchShellError> {
        self.permission_after(connection_id, 0, wait).await
    }

    /// Recover an unanswered permission after this attachment's cursor.
    ///
    /// # Errors
    /// Returns not-found for an unknown connection.
    pub async fn permission_after(
        &self,
        connection_id: &str,
        after: u64,
        wait: Duration,
    ) -> Result<Option<crate::NativePermissionRequest>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        Ok(
            tokio::time::timeout(wait, connection.control.permission_request_after(after))
                .await
                .unwrap_or_default(),
        )
    }

    /// Answer one pending permission request with an exact agent-offered
    /// option id.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for an invalid sequence or
    /// an option the agent did not offer (the request stays pending).
    pub async fn select_permission(
        &self,
        connection_id: &str,
        sequence: u64,
        option_id: &str,
    ) -> Result<(), WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .select_permission(sequence, option_id)
            .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))
    }

    /// Recover an unanswered file call after this attachment's cursor.
    ///
    /// The path in what comes back has already been checked against the
    /// profile's workspace by the host: a surface is never handed a path it
    /// would have to refuse for the boundary's sake.
    ///
    /// # Errors
    /// Returns not-found for an unknown connection.
    pub async fn file_request_after(
        &self,
        connection_id: &str,
        after: u64,
        wait: Duration,
    ) -> Result<Option<crate::NativeFileRequest>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        Ok(
            tokio::time::timeout(wait, connection.control.file_request_after(after))
                .await
                .unwrap_or_default(),
        )
    }

    /// Tell one pending file call what the surface did with it.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::Conflict`] for a sequence that is no
    /// longer pending, or an answer that does not fit the method asked for.
    pub async fn answer_file_request(
        &self,
        connection_id: &str,
        sequence: u64,
        answer: crate::NativeFileAnswer,
    ) -> Result<(), WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .answer_file_request(sequence, answer)
            .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))
    }

    /// Long-poll the next connection-bound ACP elicitation. The exact request
    /// is ephemeral and is not reconstructed from the routing ledger.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection.
    pub async fn next_elicitation(
        &self,
        connection_id: &str,
        wait: Duration,
    ) -> Result<Option<crate::NativeElicitationRequest>, WorkbenchShellError> {
        self.elicitation_after(connection_id, 0, wait).await
    }

    /// Recover an unanswered elicitation after this attachment's cursor.
    ///
    /// # Errors
    /// Returns not-found for an unknown connection.
    pub async fn elicitation_after(
        &self,
        connection_id: &str,
        after: u64,
        wait: Duration,
    ) -> Result<Option<crate::NativeElicitationRequest>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        Ok(
            tokio::time::timeout(wait, connection.control.elicitation_request_after(after))
                .await
                .unwrap_or_default(),
        )
    }

    /// Return an official accept/decline/cancel action to the requesting ACP
    /// agent. Accepted form values remain connection-local.
    ///
    /// # Errors
    ///
    /// Returns a conflict for a stale sequence or schema-invalid response and
    /// not-found for an unknown connection.
    pub async fn answer_elicitation(
        &self,
        connection_id: &str,
        sequence: u64,
        action: AcpElicitationAction,
    ) -> Result<(), WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .answer_elicitation(sequence, action)
            .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))
    }

    /// Report the connection phase and identity without exposing the native
    /// session id to the browser.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection.
    pub async fn connection_status(
        &self,
        connection_id: &str,
    ) -> Result<Value, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let phase = connection.control.phase();
        let setup = connection.setup_notes.lock().await.clone();
        Ok(json!({
            "connection_id": connection_id,
            "route_id": connection.route_id,
            "phase": phase,
            "setup": setup,
        }))
    }

    /// What the agent offers to configure on this session, and its modes.
    ///
    /// # Errors
    ///
    /// Not found for an unknown connection.
    pub async fn connection_options(
        &self,
        connection_id: &str,
    ) -> Result<crate::SessionConfiguration, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        Ok(connection.control.configuration().await)
    }

    /// Choose one of the agent's config options on this session - its model,
    /// its mode, whatever it declared.
    ///
    /// # Errors
    ///
    /// Not found for an unknown connection; conflict, with the control's own
    /// sentence, when the option or the value is not one the agent offers.
    /// The connection stays open either way.
    pub async fn set_connection_option(
        &self,
        connection_id: &str,
        config_id: &str,
        value: &Value,
    ) -> Result<Vec<agent_client_protocol::schema::v1::SessionConfigOption>, WorkbenchShellError>
    {
        let connection = self.connection(connection_id).await?;
        let value = config_value_of(value)?;
        connection
            .control
            .set_config_option(config_id, value)
            .await
            .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))
    }

    /// Choose one of the agent's transitional modes on this session.
    ///
    /// # Errors
    ///
    /// As [`Self::set_connection_option`].
    pub async fn set_connection_mode(
        &self,
        connection_id: &str,
        mode_id: &str,
    ) -> Result<agent_client_protocol::schema::v1::SessionModeState, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        connection
            .control
            .set_legacy_mode(mode_id)
            .await
            .map_err(|error| WorkbenchShellError::Conflict(error.to_string()))
    }

    /// Long-poll the durable ledger for the next delivery batch after this
    /// surface's cursor. Reading never advances the cursor.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown route.
    pub async fn events(
        &self,
        route_id: &str,
        surface_id: &str,
        limit: usize,
        wait: Duration,
    ) -> Result<SurfaceEventBatch, WorkbenchShellError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let route = route_id.to_owned();
            let surface = surface_id.to_owned();
            let batch = self
                .with_ledger(move |ledger| ledger.events_for_surface(&route, &surface, limit))
                .await
                .map_err(|error| match error {
                    crate::RoutingError::RouteNotFound(route) => {
                        WorkbenchShellError::NotFound(format!("unknown route {route}"))
                    }
                    other => WorkbenchShellError::Failed(other.to_string()),
                })?;
            if !batch.events.is_empty() || tokio::time::Instant::now() >= deadline {
                return Ok(batch);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Acknowledge this surface's cursor after the events were rendered.
    ///
    /// # Errors
    ///
    /// Regression/beyond-head acknowledgements are refused by the ledger.
    pub async fn acknowledge(
        &self,
        route_id: &str,
        surface_id: &str,
        cursor: u64,
    ) -> Result<(), WorkbenchShellError> {
        let route = route_id.to_owned();
        let surface = surface_id.to_owned();
        self.with_ledger(move |ledger| ledger.acknowledge_surface(&route, &surface, cursor))
            .await
            .map_err(|error| match error {
                crate::RoutingError::RouteNotFound(route) => {
                    WorkbenchShellError::NotFound(format!("unknown route {route}"))
                }
                other => WorkbenchShellError::Conflict(other.to_string()),
            })
    }

    /// Discover the Apps of this connection's attachments (lazily dialing
    /// host-side stdio clients on first use) and project them for the panel.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection.
    pub async fn apps_list(
        &self,
        connection_id: &str,
    ) -> Result<Vec<crate::AppAttachmentView>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let mut apps = connection.apps.lock().await;
        if apps.is_none() {
            *apps = Some(
                workbench_apps::discover(
                    &connection.attachments,
                    Some(&connection.artifact_projector.environment_lease.workspace),
                )
                .await,
            );
        }
        Ok(apps.as_ref().map(ConnectionApps::views).unwrap_or_default())
    }

    /// Start one App-only structured fallback on the explicitly independent
    /// Workbench MCP connection. Success means the server returned a real MCP
    /// `input_required` form; ordinary completed tool calls are refused.
    ///
    /// # Errors
    ///
    /// Returns an explicit connection, discovery, visibility or MCP protocol
    /// error; a completed ordinary tool is never accepted as an interaction.
    pub async fn start_app_interaction(
        &self,
        connection_id: &str,
        server_name: &str,
        tool: &str,
        arguments: JsonObject,
    ) -> Result<crate::workbench_apps::PendingElicitationView, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let mut apps = connection.apps.lock().await;
        if apps.is_none() {
            *apps = Some(
                workbench_apps::discover(
                    &connection.attachments,
                    Some(&connection.artifact_projector.environment_lease.workspace),
                )
                .await,
            );
        }
        let state = apps.as_mut().ok_or_else(|| {
            WorkbenchShellError::Conflict("no Apps were discovered on this connection".into())
        })?;
        let view = state
            .start_interaction(server_name, tool, arguments)
            .await
            .map_err(WorkbenchShellError::Conflict)?;
        let event = state.next_event;
        state.next_event += 1;
        drop(apps);
        self.append_app_event(
            &connection,
            connection_id,
            event,
            "host/app_elicitation_requested",
            json!({
                "server": view.server_name,
                "tool": view.tool,
                "interaction": view.interaction_id,
                "connection_scope": view.connection_scope,
            }),
        )
        .await?;
        Ok(view)
    }

    /// Return one exact accept/decline/cancel action to the MCP server and
    /// drive the second MRTR call. Raw form content remains connection-local.
    ///
    /// # Errors
    ///
    /// Returns an explicit error for an unknown/consumed interaction, invalid
    /// action payload, transport failure or unsupported additional MRTR round.
    pub async fn answer_app_interaction(
        &self,
        connection_id: &str,
        interaction_id: &str,
        action: ElicitationAction,
        content: Option<Value>,
    ) -> Result<rmcp::model::CallToolResult, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let mut apps = connection.apps.lock().await;
        let state = apps.as_mut().ok_or_else(|| {
            WorkbenchShellError::Conflict("no Apps were discovered on this connection".into())
        })?;
        let result = state
            .answer_interaction(interaction_id, action.clone(), content)
            .await
            .map_err(WorkbenchShellError::Conflict)?;
        let event = state.next_event;
        state.next_event += 1;
        drop(apps);
        self.append_app_event(
            &connection,
            connection_id,
            event,
            "host/app_elicitation_answered",
            json!({
                "interaction": interaction_id,
                "action": action,
                "completed": true,
            }),
        )
        .await?;
        Ok(result)
    }

    /// Long-poll the next exact agent-initiated App call and open its declared
    /// resource on the existing independent host-side App connection.
    ///
    /// Raw input/result comes only from connection-local observation memory.
    /// The ledger contains redacted descriptors written by the observer
    /// projector, not this payload.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection and
    /// propagates App resource discovery/read failures.
    pub async fn next_observed_app(
        &self,
        connection_id: &str,
        after: u64,
        wait: Duration,
    ) -> Result<Option<ObservedAppOpen>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let observation = connection.observation.lock().await;
        let Some(runtime) = observation.as_ref() else {
            return Ok(None);
        };
        let Some(call) = runtime.next(after, wait).await else {
            return Ok(None);
        };
        drop(observation);
        let mut observed_apps = connection.observed_apps.lock().await;
        let opened = if let Some(opened) = observed_apps.get(&call.observation_id) {
            opened.clone()
        } else {
            let opened = self
                .app_open(connection_id, &call.server_name, &call.resource_uri)
                .await?;
            observed_apps.insert(call.observation_id.clone(), opened.clone());
            opened
        };
        Ok(Some(ObservedAppOpen {
            opened,
            observation: call,
        }))
    }

    /// Await the terminal result of one already opened observed App call.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection.
    pub async fn observed_app_status(
        &self,
        connection_id: &str,
        observation_id: &str,
        wait: Duration,
    ) -> Result<Option<ObservedAppCall>, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let observation = connection.observation.lock().await;
        let Some(runtime) = observation.as_ref() else {
            return Ok(None);
        };
        Ok(runtime.get_terminal(observation_id, wait).await)
    }

    /// Open one declared App: read its resource (content-level `_meta.ui`
    /// wins), resolve CSP/permissions in Rust and register the app id.
    ///
    /// # Errors
    ///
    /// Fails closed on unknown connection/server, an undeclared URI, or a
    /// resource that is not a spec-shaped App document.
    pub async fn app_open(
        &self,
        connection_id: &str,
        server_name: &str,
        uri: &str,
    ) -> Result<OpenedApp, WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let mut apps = connection.apps.lock().await;
        if apps.is_none() {
            *apps = Some(
                workbench_apps::discover(
                    &connection.attachments,
                    Some(&connection.artifact_projector.environment_lease.workspace),
                )
                .await,
            );
        }
        let Some(state) = apps.as_mut() else {
            return Err(WorkbenchShellError::Failed(
                "apps state disappeared after initialization".into(),
            ));
        };
        let entry_index = state
            .entries
            .iter()
            .position(|entry| entry.server_name == server_name)
            .ok_or_else(|| {
                WorkbenchShellError::NotFound(format!("unknown attachment {server_name}"))
            })?;
        let workbench_apps::AppRead {
            html,
            csp,
            permissions,
            prefers_border,
            isolated,
        } = workbench_apps::read_app(&state.entries[entry_index], uri)
            .await
            .map_err(WorkbenchShellError::Conflict)?;
        let app_id = format!("a{}", state.next_app);
        state.next_app += 1;
        state.open.insert(
            app_id.clone(),
            OpenApp {
                entry_index,
                uri: uri.to_owned(),
                html: html.clone(),
                csp: csp.clone(),
                isolated,
            },
        );
        let event = state.next_event;
        state.next_event += 1;
        drop(apps);
        self.append_app_event(
            &connection,
            connection_id,
            event,
            "host/app_opened",
            json!({
                "server": server_name,
                "uri": uri,
                "csp": csp,
                "permissions": permissions,
            }),
        )
        .await?;
        let (sandbox_url, sandbox_origin) =
            self.sandbox.get().map_or((None, None), |(url, origin)| {
                (Some(url.clone()), Some(origin.clone()))
            });
        let view_url = sandbox_origin
            .as_ref()
            .filter(|_| isolated)
            .map(|origin| format!("{origin}/apps/{connection_id}/{app_id}/view/"));
        Ok(OpenedApp {
            app_id,
            connection_id: connection_id.to_owned(),
            server_name: server_name.to_owned(),
            uri: uri.to_owned(),
            html,
            csp,
            permissions,
            prefers_border,
            sandbox_url,
            sandbox_origin,
            isolated,
            view_url,
        })
    }

    /// The View of an open App that asked for a real origin, as the document
    /// the sandbox origin serves at `/apps/{connection}/{app}/view/`, with
    /// the CSP the host resolved for it.
    ///
    /// # Errors
    ///
    /// Refuses unknown connections and App handles, and an App that did not
    /// ask for a real origin (its View is a `srcdoc` document).
    pub async fn app_view(
        &self,
        connection_id: &str,
        app_id: &str,
    ) -> Result<(String, String), WorkbenchShellError> {
        if connection_id == server_apps::SPACE_CONNECTION {
            return self.space_app_view(app_id).await;
        }
        let connection = self.connection(connection_id).await?;
        let apps = connection.apps.lock().await;
        let Some(state) = apps.as_ref() else {
            return Err(WorkbenchShellError::NotFound(format!(
                "connection {connection_id} has no open Apps"
            )));
        };
        let open = state
            .open
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown App {app_id}")))?;
        if !open.isolated {
            return Err(WorkbenchShellError::NotFound(format!(
                "App {app_id} is not served as a document"
            )));
        }
        Ok((open.html.clone(), open.csp.clone()))
    }

    /// One file under an open App's View path: the server's resource whose
    /// uri is the View's uri with its last segment replaced by `path`, text
    /// or blob, with the media type the server lists.
    ///
    /// # Errors
    ///
    /// Refuses unknown connections and App handles, a climbing or empty
    /// path, and anything the server does not list.
    pub async fn app_file_resource(
        &self,
        connection_id: &str,
        app_id: &str,
        path: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        if connection_id == server_apps::SPACE_CONNECTION {
            return self.space_app_file(app_id, path).await;
        }
        let connection = self.connection(connection_id).await?;
        let apps = connection.apps.lock().await;
        let Some(state) = apps.as_ref() else {
            return Err(WorkbenchShellError::NotFound(format!(
                "connection {connection_id} has no open Apps"
            )));
        };
        let open = state
            .open
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown App {app_id}")))?;
        let entry = state
            .entries
            .get(open.entry_index)
            .ok_or_else(|| WorkbenchShellError::Failed("open App lost its attachment".into()))?;
        let uri = workbench_apps::sibling_uri(&open.uri, path)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no file at {path}")))?;
        workbench_apps::read_file_resource(entry, &uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// A script resource of an open App's server, for the sandbox origin to
    /// serve to that App (worklet and worker scripts cannot be `blob:` under
    /// the App CSP and must come from a URL the CSP allows). Only resources
    /// the server lists with the script MIME qualify; the App must be open.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for an unknown connection or
    /// App and [`WorkbenchShellError::Conflict`] when the server does not
    /// list the URI as a script resource or the read disagrees.
    pub async fn app_script_resource(
        &self,
        connection_id: &str,
        app_id: &str,
        uri: &str,
    ) -> Result<String, WorkbenchShellError> {
        if connection_id == server_apps::SPACE_CONNECTION {
            return self.space_app_script(app_id, uri).await;
        }
        let connection = self.connection(connection_id).await?;
        let apps = connection.apps.lock().await;
        let Some(state) = apps.as_ref() else {
            return Err(WorkbenchShellError::NotFound(format!(
                "connection {connection_id} has no open Apps"
            )));
        };
        let open = state
            .open
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown App {app_id}")))?;
        let entry = state
            .entries
            .get(open.entry_index)
            .ok_or_else(|| WorkbenchShellError::Failed("open App lost its attachment".into()))?;
        workbench_apps::read_script_resource(entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// The exact bytes of one blob resource of an open App's server, with the
    /// media type the server gave them.
    ///
    /// # Errors
    ///
    /// Refuses unknown connections and App handles, and anything the server
    /// does not serve as a blob.
    pub async fn app_blob_resource(
        &self,
        connection_id: &str,
        app_id: &str,
        uri: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        if connection_id == server_apps::SPACE_CONNECTION {
            return self.space_app_blob(app_id, uri).await;
        }
        let connection = self.connection(connection_id).await?;
        let apps = connection.apps.lock().await;
        let Some(state) = apps.as_ref() else {
            return Err(WorkbenchShellError::NotFound(format!(
                "connection {connection_id} has no open Apps"
            )));
        };
        let open = state
            .open
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown App {app_id}")))?;
        let entry = state
            .entries
            .get(open.entry_index)
            .ok_or_else(|| WorkbenchShellError::Failed("open App lost its attachment".into()))?;
        workbench_apps::read_blob_resource(entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// The workspace an open App's server works in, where bytes the App
    /// uploads land. The host that declared the server knows it; the App
    /// never names a path, only a handle.
    ///
    /// # Errors
    ///
    /// Refuses unknown connections and App handles, and a server the host
    /// serves without a workspace.
    pub async fn app_upload_root(
        &self,
        connection_id: &str,
        app_id: &str,
    ) -> Result<PathBuf, WorkbenchShellError> {
        let root = if connection_id == server_apps::SPACE_CONNECTION {
            self.space_app_upload_root(app_id).await?
        } else {
            let connection = self.connection(connection_id).await?;
            let apps = connection.apps.lock().await;
            let Some(state) = apps.as_ref() else {
                return Err(WorkbenchShellError::NotFound(format!(
                    "connection {connection_id} has no open Apps"
                )));
            };
            let open = state
                .open
                .get(app_id)
                .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown App {app_id}")))?;
            state
                .entries
                .get(open.entry_index)
                .ok_or_else(|| WorkbenchShellError::Failed("open App lost its attachment".into()))?
                .upload_root
                .clone()
        };
        root.ok_or_else(|| {
            WorkbenchShellError::NotFound(format!(
                "the server of App {app_id} has no workspace to upload into"
            ))
        })
    }

    /// Relay one bare JSON-RPC message from an open App. The method allowlist
    /// and the tool visibility gate live HERE; `ui/*` never reaches the
    /// server; cross-server calls are impossible because the app id maps to
    /// exactly one server's client. A message without an integer/string id is
    /// a notification and is deliberately not forwarded.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for unknown connection/app
    /// and [`WorkbenchShellError::Invalid`] for a non-JSON-RPC body; relay
    /// refusals are JSON-RPC errors in the returned value, not `Err`.
    #[allow(
        clippy::too_many_lines,
        reason = "one linear relay transaction keeps parse, gate, execute and ledger ordering auditable"
    )]
    pub async fn app_rpc(
        &self,
        connection_id: &str,
        app_id: &str,
        message: Value,
    ) -> Result<Value, WorkbenchShellError> {
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(WorkbenchShellError::Invalid(
                "relay accepts JSON-RPC 2.0 messages".into(),
            ));
        }
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            return Err(WorkbenchShellError::Invalid(
                "relay accepts requests and notifications, not responses".into(),
            ));
        };
        let method = method.to_owned();
        // Integer or string ids are requests; anything else (including the
        // float-id trick that parses as a notification) is not forwarded.
        let id = message.get("id").cloned().filter(|id| {
            id.is_string()
                || matches!(id, Value::Number(number) if number.is_i64() || number.is_u64())
        });
        let connection = self.connection(connection_id).await?;
        let mut apps = connection.apps.lock().await;
        let state = apps.as_mut().ok_or_else(|| {
            WorkbenchShellError::Conflict("no apps were discovered on this connection".into())
        })?;
        let open = state
            .open
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown app {app_id}")))?;
        let entry_index = open.entry_index;
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let tool_name = params
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(id) = id else {
            return Ok(Value::Null);
        };
        let decision =
            workbench_apps::allow_relay(&state.entries[entry_index], &method, tool_name.as_deref());
        let server_name = state.entries[entry_index].server_name.clone();
        if let Err(refusal) = decision {
            let event = state.next_event;
            state.next_event += 1;
            drop(apps);
            self.append_app_event(
                &connection,
                connection_id,
                event,
                "host/app_tool_call",
                json!({
                    "server": server_name,
                    "method": method,
                    "tool": tool_name,
                    "decision": "refused",
                    "reason": refusal.message(),
                }),
            )
            .await?;
            let code = match refusal {
                RelayRefusal::MethodNotAllowed(_) => -32601,
                RelayRefusal::ToolNotDeclared(_) | RelayRefusal::ToolNotAppVisible(_) => -32602,
            };
            return Ok(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": code, "message": refusal.message()},
            }));
        }
        let ledger_event = (method == "tools/call").then(|| {
            let event = state.next_event;
            state.next_event += 1;
            event
        });
        // Taking the client puts the lock down; the event number above is
        // still reserved under it, so the ledger order does not move.
        let outcome = match workbench_apps::take_relay_client(apps, entry_index) {
            Ok(client) => workbench_apps::execute_relay(&client, &method, &params).await,
            Err(refusal) => Err(refusal),
        };
        if let Some(event) = ledger_event {
            self.append_app_event(
                &connection,
                connection_id,
                event,
                "host/app_tool_call",
                json!({
                    "server": server_name,
                    "method": method,
                    "tool": tool_name,
                    "decision": "allowed",
                }),
            )
            .await?;
        }
        Ok(match outcome {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(message) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32000, "message": message},
            }),
        })
    }

    /// Close one open App after its `ui/resource-teardown` request resolved.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError::NotFound`] for unknown connection/app.
    pub async fn app_close(
        &self,
        connection_id: &str,
        app_id: &str,
    ) -> Result<(), WorkbenchShellError> {
        let connection = self.connection(connection_id).await?;
        let mut apps = connection.apps.lock().await;
        let state = apps.as_mut().ok_or_else(|| {
            WorkbenchShellError::Conflict("no apps were discovered on this connection".into())
        })?;
        let open = state
            .open
            .remove(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("unknown app {app_id}")))?;
        let server_name = state.entries[open.entry_index].server_name.clone();
        let event = state.next_event;
        state.next_event += 1;
        drop(apps);
        connection
            .observed_apps
            .lock()
            .await
            .retain(|_, opened| opened.app_id != app_id);
        self.append_app_event(
            &connection,
            connection_id,
            event,
            "host/app_closed",
            json!({"server": server_name, "uri": open.uri}),
        )
        .await
    }

    /// Record the sandbox listener address once the serve layer bound it.
    pub(crate) fn set_sandbox(&self, url: String, origin: String) {
        let _ = self.sandbox.set((url, origin));
    }

    /// Mint nothing: record the secret the product minted for this run.
    ///
    /// A Workbench that is handed one asks every caller for it; one that is
    /// not asks nobody, which is the shape a test serving the shell in
    /// process has always had.
    pub fn set_session_token(&self, token: String) {
        let _ = self.session_token.set(token);
    }

    /// The secret this run asks for, if it asks for one.
    fn session_token(&self) -> Option<&str> {
        self.session_token.get().map(String::as_str)
    }

    /// Record the bundle directory holding `apps-bridge.js`.
    pub(crate) fn set_apps_bundle(&self, bundle: PathBuf) {
        let _ = self.apps_bundle.set(bundle);
    }

    /// Apps are part of the product: the bridge is embedded, so the panel is
    /// live rather than waiting for a second install step.
    pub(crate) fn apps_enabled() -> bool {
        !APPS_BRIDGE.is_empty()
    }

    /// The command that serves an agent's own schedules to its session:
    /// an executable and what it is given before the ledger, the agent and
    /// the chat. Without it an agent makes no schedules.
    pub fn set_time_tools_command(&self, executable: PathBuf, prefix_args: Vec<String>) {
        let _ = self.time_tools.set((executable, prefix_args));
    }

    /// Configure the product-internal observer command. The command is inert
    /// unless the Apps bundle is also enabled and a stdio tool declares a UI.
    pub fn set_mcp_observer_command(&self, executable: PathBuf, prefix_args: Vec<String>) {
        let _ = self.mcp_observer.set(ObserverCommand {
            executable,
            prefix_args,
        });
    }

    async fn append_app_event(
        &self,
        connection: &WorkbenchConnection,
        connection_id: &str,
        event: u64,
        kind: &str,
        payload: Value,
    ) -> Result<(), WorkbenchShellError> {
        let route = connection.route_id.clone();
        let event_id = format!("app-{connection_id}-{event}");
        let kind = kind.to_owned();
        self.with_ledger(move |ledger| {
            ledger.append_event(&route, &event_id, &kind, SurfaceEventSource::Host, &payload)
        })
        .await
        .map(|_| ())
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    async fn with_ledger<T, F>(&self, operation: F) -> Result<T, crate::RoutingError>
    where
        T: Send + 'static,
        F: FnOnce(&mut RoutingLedger) -> Result<T, crate::RoutingError> + Send + 'static,
    {
        let path = self.ledger_path.clone();
        tokio::task::spawn_blocking(move || {
            let mut ledger = RoutingLedger::open(&path)?;
            operation(&mut ledger)
        })
        .await
        .map_err(|error| crate::RoutingError::Projection(error.to_string()))?
    }
}

/// A running shell server bound to a local address. Dropping it stops the
/// server tasks; open connections keep their own runner tasks.
pub struct WorkbenchShellHandle {
    pub local_addr: SocketAddr,
    /// The second-origin sandbox listener (origin = scheme+host+port).
    pub sandbox_addr: SocketAddr,
    state: Arc<WorkbenchShellState>,
    task: Option<tokio::task::JoinHandle<()>>,
    sandbox_task: Option<tokio::task::JoinHandle<()>>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    sandbox_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl std::fmt::Debug for WorkbenchShellHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkbenchShellHandle")
            .field("local_addr", &self.local_addr)
            .field("sandbox_addr", &self.sandbox_addr)
            .finish_non_exhaustive()
    }
}

impl WorkbenchShellHandle {
    /// Stop both listeners and every accepted HTTP connection owned by them,
    /// then terminate the Project space's host-side clients and await their
    /// child boundaries. Native agent connections remain owned by
    /// [`WorkbenchShellState`] and must be disconnected or closed through
    /// their normal lifecycle first.
    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(shutdown) = self.sandbox_shutdown.take() {
            let _ = shutdown.send(());
        }
        for task in [&mut self.task, &mut self.sandbox_task] {
            if let Some(task) = task.take() {
                let _ = task.await;
            }
        }
        let _ = self.state.shutdown_server_apps().await;
    }
}

impl Drop for WorkbenchShellHandle {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(shutdown) = self.sandbox_shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

/// What is answered.
pub type ShellBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;

/// What is asked, whoever heard it: the harness's own listener, or the
/// server of a product the harness is built into.
pub type AskedBody = http_body_util::combinators::UnsyncBoxBody<Bytes, std::io::Error>;

/// A request as the harness takes it, from a request as somebody's server
/// heard it.
pub fn asked<Body>(request: Request<Body>) -> Request<AskedBody>
where
    Body: hyper::body::Body<Data = Bytes> + Send + 'static,
    Body::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    request.map(|body| body.map_err(std::io::Error::other).boxed_unsync())
}

fn respond(status: StatusCode, content_type: &str, body: String) -> Response<ShellBody> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        .body(
            Full::new(Bytes::from(body))
                .map_err(infallible_to_io)
                .boxed(),
        )
        .expect("static response")
}

/// Bytes as they are, with the type the caller decided on. Used for a file
/// out of the agent's outbox, whose content type comes from its name rather
/// than from anything the agent said.
fn respond_bytes(status: StatusCode, content_type: &str, body: Vec<u8>) -> Response<ShellBody> {
    Response::builder()
        .status(status)
        .header("content-type", content_type)
        // The page opens these in a tab. A file the agent wrote is not the
        // Workbench's own script, so it is never run as one.
        .header("x-content-type-options", "nosniff")
        .header("content-security-policy", "sandbox")
        .body(
            Full::new(Bytes::from(body))
                .map_err(infallible_to_io)
                .boxed(),
        )
        .expect("static response")
}

fn infallible_to_io(value: Infallible) -> std::io::Error {
    match value {}
}

fn decode_standard_base64(value: &str) -> Result<Vec<u8>, WorkbenchShellError> {
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| WorkbenchShellError::Invalid("ACP binary content is not valid base64".into()))
}

fn content_name_from_uri(uri: &str, fallback: &str) -> String {
    uri.rsplit(['/', ':'])
        .next()
        .map(percent_decode)
        .filter(|name| !name.is_empty() && name.len() <= 512 && !name.chars().any(char::is_control))
        .unwrap_or_else(|| fallback.to_owned())
}

fn respond_json(status: StatusCode, value: &Value) -> Response<ShellBody> {
    respond(status, "application/json", value.to_string())
}

fn error_response(error: &WorkbenchShellError) -> Response<ShellBody> {
    respond_json(error.status(), &json!({ "error": error.to_string() }))
}

fn json_result<T: serde::Serialize>(result: Result<T, WorkbenchShellError>) -> Response<ShellBody> {
    match result.and_then(|value| {
        serde_json::to_value(value).map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }) {
        Ok(value) => respond_json(StatusCode::OK, &value),
        Err(error) => error_response(&error),
    }
}

/// [`json_result`] for work that computes rather than waits.
///
/// A handler that does its work synchronously holds the runtime thread it is
/// polled on for the whole of it, and nothing can take that task away: it
/// never yields. On 2026-09-21 one handler that loaded WebAssembly components
/// took 13-22 s on this machine, and an unrelated request measured beside it
/// went from 39 ms to 21 s. From the page that is every press being ignored
/// for twenty seconds.
///
/// That handler has left the host, but the shape is what allowed it: any
/// handler that thinks instead of waiting can do this again. So the computing
/// ones are handed to the blocking
/// pool, where a thread may be occupied without costing the runtime one, and
/// the waiting ones are left exactly where they are - they already yield, and
/// moving them would only add a hop.
async fn blocking_json<T, F>(work: F) -> Response<ShellBody>
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce() -> Result<T, WorkbenchShellError> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => json_result(result),
        Err(error) => error_response(&WorkbenchShellError::Failed(error.to_string())),
    }
}

fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

/// The page a bot opens inside the messenger, with the channel written in.
fn mini_app_page(channel_id: &str) -> Response<ShellBody> {
    let safe: String = channel_id
        .chars()
        .filter(|letter| letter.is_ascii_alphanumeric() || *letter == '_' || *letter == '-')
        .collect();
    respond(
        StatusCode::OK,
        "text/html;charset=utf-8",
        MINI_APP_HTML
            .replace("__PALETTE_CSS__", PALETTE_CSS)
            .replace("__KIT_CSS__", KIT_CSS)
            .replace("__CHANNEL_ID__", &safe),
    )
}

/// The Mini App's API, answered the same from the main listener and from
/// the gate a tunnel points at: who opened it is said by the messenger's
/// signature, which the channel checks on every call. `Err` for a route
/// that is not the app's.
async fn route_the_app(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    query: Option<&str>,
    request: Request<AskedBody>,
) -> Result<Response<ShellBody>, Request<AskedBody>> {
    let response = match (method, segments) {
        (&Method::OPTIONS, ["api", "channels", _, "app"] | ["api", "channels", _, "app", ..]) => {
            for_an_app(respond(StatusCode::NO_CONTENT, "text/plain", String::new()))
        }
        (&Method::GET, ["api", "channels", channel_id, "app"]) => {
            let (channel_id, init_data) = ((*channel_id).to_owned(), app_data_of(&request));
            for_an_app(json_result(
                state.app_standing(&channel_id, &init_data).await,
            ))
        }
        (&Method::POST, ["api", "channels", channel_id, "app", "files"]) => {
            let (channel_id, init_data) = ((*channel_id).to_owned(), app_data_of(&request));
            let words = query_param(query, "words")
                .map(|value| percent_decode(&value))
                .unwrap_or_default();
            let ingest = upload_content(state, request, query);
            for_an_app(json_result(
                state
                    .app_upload(&channel_id, &init_data, &words, ingest)
                    .await,
            ))
        }
        (&Method::GET, ["api", "channels", channel_id, "app", "files", name]) => {
            let (channel_id, init_data) = ((*channel_id).to_owned(), app_data_of(&request));
            let name = percent_decode(name);
            for_an_app(match state.app_file(&channel_id, &init_data, &name).await {
                Ok((bytes, media_type)) => respond_bytes(StatusCode::OK, &media_type, bytes),
                Err(error) => error_response(&error),
            })
        }
        _ => return Err(request),
    };
    Ok(response)
}

/// What a Mini App sends to be known: the messenger's signed data, in a
/// header of its own.
fn app_data_of(request: &Request<AskedBody>) -> String {
    request
        .headers()
        .get("x-swem-app-data")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// An answer to a Mini App, which may be hosted anywhere: any origin may
/// ask, carrying the messenger's signature and nothing of a session.
fn for_an_app(mut response: Response<ShellBody>) -> Response<ShellBody> {
    let headers = response.headers_mut();
    headers.insert("access-control-allow-origin", HeaderValue::from_static("*"));
    headers.insert(
        "access-control-allow-headers",
        HeaderValue::from_static("x-swem-app-data, content-type"),
    );
    headers.insert(
        "access-control-allow-methods",
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    response
}

/// A request's body as text, for what is handed on as it came.
async fn body_text_of(request: Request<AskedBody>) -> Result<String, WorkbenchShellError> {
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?
        .to_bytes();
    String::from_utf8(body.to_vec())
        .map_err(|error| WorkbenchShellError::Invalid(format!("not text: {error}")))
}

async fn read_json(request: Request<AskedBody>) -> Result<Value, WorkbenchShellError> {
    let body = request
        .into_body()
        .collect()
        .await
        .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?
        .to_bytes();
    serde_json::from_slice(&body).map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
}

async fn upload_content(
    state: &WorkbenchShellState,
    request: Request<AskedBody>,
    query: Option<&str>,
) -> Result<crate::WorkbenchContentDescriptor, WorkbenchShellError> {
    let name = query_param(query, "name")
        .map(|value| percent_decode(&value))
        .ok_or_else(|| WorkbenchShellError::Invalid("name query parameter is required".into()))?;
    let media_type = request
        .headers()
        .get(hyper::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_owned();
    state
        .content
        .ingest_http(name, media_type, request.into_body())
        .await
}

#[derive(Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64,
}

fn parse_byte_range(value: &str, length: u64) -> Option<ByteRange> {
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') || length == 0 {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix: u64 = end.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some(ByteRange {
            start: length.saturating_sub(suffix),
            end: length - 1,
        });
    }
    let start: u64 = start.parse().ok()?;
    if start >= length {
        return None;
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>().ok()?.min(length - 1)
    };
    (end >= start).then_some(ByteRange { start, end })
}

fn disposition_value(name: &str, download: bool) -> String {
    let fallback = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let encoded = name
        .as_bytes()
        .iter()
        .map(|byte| match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' => {
                char::from(*byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect::<String>();
    format!(
        "{}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}",
        if download { "attachment" } else { "inline" }
    )
}

fn representation_digest_header(content_digest: &str) -> Result<String, WorkbenchShellError> {
    let hex = content_digest
        .strip_prefix("sha256:")
        .ok_or_else(|| WorkbenchShellError::Failed("unsupported content digest".into()))?;
    if hex.len() != 64 {
        return Err(WorkbenchShellError::Failed("invalid content digest".into()));
    }
    let mut bytes = Vec::with_capacity(32);
    for pair in hex.as_bytes().chunks_exact(2) {
        let pair = std::str::from_utf8(pair)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        bytes.push(
            u8::from_str_radix(pair, 16)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?,
        );
    }
    Ok(format!(
        "sha-256=:{}:",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

async fn serve_content(
    state: &WorkbenchShellState,
    request: Request<AskedBody>,
    descriptor_id: &str,
    download: bool,
) -> Result<Response<ShellBody>, WorkbenchShellError> {
    let descriptor = state.content.load(descriptor_id).await?;
    let requested_range = request
        .headers()
        .get(hyper::header::RANGE)
        .and_then(|value| value.to_str().ok());
    let range = match requested_range {
        Some(value) => match parse_byte_range(value, descriptor.byte_length) {
            Some(range) => Some(range),
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(
                        "content-range",
                        format!("bytes */{}", descriptor.byte_length),
                    )
                    .header("accept-ranges", "bytes")
                    .body(Full::new(Bytes::new()).map_err(infallible_to_io).boxed())
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()));
            }
        },
        None => None,
    };
    let (status, start, length, content_range) =
        range.map_or((StatusCode::OK, 0, descriptor.byte_length, None), |range| {
            (
                StatusCode::PARTIAL_CONTENT,
                range.start,
                range.end - range.start + 1,
                Some(format!(
                    "bytes {}-{}/{}",
                    range.start, range.end, descriptor.byte_length
                )),
            )
        });
    let mut builder = Response::builder()
        .status(status)
        .header(hyper::header::CONTENT_TYPE, &descriptor.media_type)
        .header(hyper::header::CONTENT_LENGTH, length)
        .header("accept-ranges", "bytes")
        .header(
            "repr-digest",
            representation_digest_header(&descriptor.content_digest)?,
        )
        .header("etag", format!("\"{}\"", descriptor.content_digest))
        .header(
            hyper::header::CONTENT_DISPOSITION,
            disposition_value(&descriptor.name, download),
        );
    if let Some(content_range) = content_range {
        builder = builder.header(hyper::header::CONTENT_RANGE, content_range);
    }
    if request.method() == Method::HEAD {
        return builder
            .body(Full::new(Bytes::new()).map_err(infallible_to_io).boxed())
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()));
    }
    let mut file = state.content.blob_file(&descriptor).await?;
    file.seek(SeekFrom::Start(start))
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    let stream = ReaderStream::new(file.take(length)).map_ok(Frame::data);
    builder
        .body(StreamBody::new(stream).boxed())
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
}

#[derive(Deserialize)]
struct ProfileSecretBody {
    type_id: String,
    #[serde(default)]
    label: String,
    /// The variable name; required only for `generic_env_var`, otherwise the
    /// type's conventional one is used.
    #[serde(default)]
    name: Option<String>,
    value: String,
}

/// What a surface needs to offer a person a key: the kinds it may be, and
/// the ones this profile already holds (names, never values).
#[derive(Serialize)]
pub struct ProfileSecrets {
    pub types: &'static [crate::SecretType],
    pub secrets: Vec<crate::SecretEntry>,
}

#[derive(Deserialize)]
struct InstallAgentBody {
    agent_id: String,
    /// The exact plan the person was shown and clicked.
    plan_id: String,
}

#[derive(Deserialize)]
struct CreateLocalProfileBody {
    agent_id: String,
    /// The name this profile goes by. Absent on first run, where there is only
    /// one and the agent's own name will do.
    #[serde(default)]
    profile_id: Option<String>,
    /// The model provider, the model, the role and the skills, when the
    /// person filled them in with the name. Absent means the agent's own
    /// defaults, which is what one click on an agent gives.
    #[serde(default)]
    setup: Option<crate::AgentSetup>,
}

#[derive(Serialize)]
struct WorkbenchPromptResponse {
    stop_reason: String,
    control_outcome: crate::NativeTurnControlOutcome,
    reply_text: String,
    artifacts: Vec<crate::WorkbenchContentDescriptor>,
    artifact_issues: Vec<WorkbenchArtifactIssue>,
}

#[derive(Clone, Default)]
struct WorkbenchArtifactCapture {
    artifacts: Vec<crate::WorkbenchContentDescriptor>,
    issues: Vec<WorkbenchArtifactIssue>,
}

type CapturedArtifact = Result<crate::WorkbenchContentDescriptor, WorkbenchArtifactIssue>;

/// A projection failure is deliberately separate from the agent turn result.
/// It contains no raw bytes or URI and is safe for the durable route.
#[derive(Clone, Debug, Serialize)]
struct WorkbenchArtifactIssue {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    media_type: Option<String>,
    reason: String,
}

impl WorkbenchArtifactIssue {
    fn new(name: String, media_type: Option<String>, reason: impl Into<String>) -> Self {
        Self {
            name,
            media_type,
            reason: reason.into(),
        }
    }
}

#[derive(Deserialize)]
struct StartAppInteractionBody {
    server_name: String,
    tool: String,
    #[serde(default)]
    arguments: JsonObject,
}

#[derive(Deserialize)]
struct AnswerAppInteractionBody {
    action: ElicitationAction,
    #[serde(default)]
    content: Option<Value>,
}

#[allow(
    clippy::too_many_lines,
    reason = "one flat routing match keeps the whole HTTP projection auditable in one place"
)]
/// A fresh secret for one run of the Workbench.
///
/// # Errors
///
/// When the operating system will not give this process random bytes.
pub fn mint_session_token() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
}

/// The cookie this Workbench keeps its secret in, named by the port so two
/// Workbenches on one machine do not overwrite each other's: cookies are
/// scoped by host and path, never by port.
fn session_cookie_name(headers: &hyper::HeaderMap) -> String {
    let port = headers
        .get(hyper::header::HOST)
        .and_then(|host| host.to_str().ok())
        .and_then(|host| host.rsplit_once(':'))
        .map_or_else(|| "0".to_owned(), |(_, port)| port.to_owned());
    format!("swem_session_{port}")
}

/// Whether this request carries the secret this run asks for.
///
/// Refusing a page on another site is not the whole door: every program on
/// this machine can reach loopback too, and a terminal and a profile's
/// secrets are behind it. So the product mints a secret per run, hands it to
/// the person in the address it prints, and asks for it here. A caller may
/// carry it in the cookie the opened page was given, or state it outright -
/// a person who wants their own script to work this Workbench has the
/// address, and the secret is in it.
fn carries_the_secret(headers: &hyper::HeaderMap, expected: &str) -> bool {
    if headers
        .get("x-swem-session")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == expected)
    {
        return true;
    }
    let name = session_cookie_name(headers);
    headers
        .get_all(hyper::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .any(|(key, value)| key.trim() == name && value.trim() == expected)
}

/// Whether this request to `/api` came from the Workbench's own page.
///
/// The Workbench listens on loopback, and loopback is not a boundary a
/// browser respects: a page on any site a person visits can call
/// `http://127.0.0.1:<port>/api/...`, and because the routes parse a body
/// whatever its content type claims, a plain cross-site form reaches them
/// with no preflight to stop it. Behind that door are a live terminal and a
/// profile's secrets.
///
/// A browser states where a request came from, and a page cannot lie about
/// it: `Origin` is set by the browser, so the rule is that a stated origin
/// must be this server's own. Requests that state none - `curl`, an MCP
/// client, the product's own tools - are not browser cross-site requests and
/// are not what this refuses; the lock that answers "which program on this
/// machine" is a different one.
///
/// `Origin: null` - an opaque origin, which is what a sandboxed frame sends -
/// is stated and is not this page, so it is refused.
fn from_the_workbenchs_own_page(headers: &hyper::HeaderMap) -> bool {
    let Some(origin) = headers.get("origin") else {
        return true;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    let Some(stated) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    // The authority the browser was told to reach, rather than an address
    // recorded at startup: a person may open `localhost` or `127.0.0.1`, and
    // either way the page's own requests state the one they opened.
    headers
        .get(hyper::header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| host == stated)
}

#[allow(
    clippy::too_many_lines,
    reason = "one match over the shell's whole route table; splitting it would hide which routes exist"
)]
pub(crate) async fn route_shell(
    state: &Arc<WorkbenchShellState>,
    request: Request<AskedBody>,
) -> Response<ShellBody> {
    let method = request.method().clone();
    let query = request.uri().query().map(str::to_owned);
    // Built into a product, the Workbench is drawn under a path of that
    // product's server, and nothing of it is anywhere else.
    let path = match state.under(request.uri().path()) {
        Ok(path) => path,
        Err(refused) => return *refused,
    };
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    // Who asks is found here, before anything is answered, and nowhere else.
    let from = state.came_from(&request);
    let said = match state.said_by(&request) {
        Ok(said) => said,
        Err(refused) => return *refused,
    };
    let who = if segments.first() == Some(&"api") {
        match state.let_in(&method, &segments, request.headers(), &from, said) {
            Ok(who) => who,
            Err(refused) => return *refused,
        }
    } else {
        None
    };
    let request =
        match door::route_door(state, &method, &segments, request, who.as_ref(), &from).await {
            Ok(response) => return response,
            Err(request) => request,
        };
    let request = match Box::pin(stream::route_chats(
        state,
        &method,
        &segments,
        query.as_deref(),
        request,
    ))
    .await
    {
        Ok(response) => return response,
        Err(request) => request,
    };
    match (&method, segments.as_slice()) {
        // A channel's Mini App: one static page, served here so a bot has
        // somewhere to point by default; hosted anywhere else just as well.
        (&Method::GET, ["channels", channel_id, "app"]) => {
            if !state.is_served_at_an_address() {
                return respond(
                    StatusCode::NOT_FOUND,
                    "text/plain;charset=utf-8",
                    "A channel's app is served where the Workbench has an address.\n".to_owned(),
                );
            }
            mini_app_page(channel_id)
        }
        (&Method::GET, []) => {
            // Opening the address the product printed is how a person hands
            // the page the run's secret: the address carries it once, the
            // document gives it to the browser to keep, and the page's own
            // calls carry it from then on without ever reading it.
            let offered = query.as_deref().and_then(|query| {
                query_param(Some(query), "token").map(|token| percent_decode(&token))
            });
            let handover = match state.session_token() {
                None => None,
                // Served at an address, the page is what anybody gets and
                // says nothing; what is behind it opens by sign-in.
                Some(_) if state.is_served_at_an_address() => None,
                Some(token) if offered.as_deref() == Some(token) => Some(format!(
                    "{}={token}; Path=/; SameSite=Strict; HttpOnly",
                    session_cookie_name(request.headers())
                )),
                Some(token) if carries_the_secret(request.headers(), token) => None,
                Some(_) => {
                    return respond(
                        StatusCode::FORBIDDEN,
                        "text/plain;charset=utf-8",
                        "This Workbench is opened at the address it printed when it started, \
                         which carries this run's secret.\n"
                            .to_owned(),
                    );
                }
            };
            // The shell is cross-origin isolated so a View that asked for a
            // real origin can be: an isolated top-level document is what
            // lets a frame under it use SharedArrayBuffer. `credentialless`
            // keeps a foreign App's own no-cors loads working (sent without
            // credentials) instead of blocking them.
            let mut response = respond(
                StatusCode::OK,
                "text/html;charset=utf-8",
                SHELL_HTML
                    .replace("__PALETTE_CSS__", PALETTE_CSS)
                    .replace("__KIT_CSS__", KIT_CSS)
                    .replace("__WORKBENCH_CSS__", WORKBENCH_CSS),
            );
            let headers = response.headers_mut();
            headers.insert(
                "cross-origin-opener-policy",
                hyper::header::HeaderValue::from_static("same-origin"),
            );
            headers.insert(
                "cross-origin-embedder-policy",
                hyper::header::HeaderValue::from_static("credentialless"),
            );
            if let Some(handover) = handover
                && let Ok(value) = hyper::header::HeaderValue::from_str(&handover)
            {
                headers.insert(hyper::header::SET_COOKIE, value);
            }
            response
        }
        (&Method::GET, ["workbench.js"]) => respond(
            StatusCode::OK,
            "application/javascript;charset=utf-8",
            WORKBENCH_JS.to_owned(),
        ),
        (&Method::GET, ["api", "profiles"]) => json_result(state.profiles()),
        // Configuring the agent: what it reaches, where it works, and a
        // prompt in its own environment.
        (&Method::PATCH, ["api", "profiles", profile_id]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<AmendProfileBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.amend_profile(profile_id, &body))
        }
        // The servers an agent's session is handed: what it attaches and
        // what the product this harness is built into gives it.
        (&Method::GET, ["api", "profiles", profile_id, "servers"]) => {
            json_result(state.servers_of(profile_id).and_then(|servers| {
                let profile = state
                    .inventory
                    .select(profile_id)
                    .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
                let attached: BTreeSet<&str> = profile
                    .attachments
                    .iter()
                    .map(|attachment| attachment.server_name.as_str())
                    .collect();
                let given: Vec<&String> = servers
                    .iter()
                    .filter(|name| !attached.contains(name.as_str()))
                    .collect();
                Ok(json!({"servers": servers, "given": given}))
            }))
        }
        (&Method::GET, ["api", "permission-profiles"]) => json_result(Ok(json!({
            "profiles": crate::permission_profiles(),
        }))),
        (&Method::GET, ["api", "environments"]) => json_result(Ok(json!({
            "environments": state.environments_offered(),
        }))),
        // What this machine is and where on it an agent may live.
        (&Method::GET, ["api", "hosts"]) => json_result(state.hosts().map(|hosts| {
            json!({
                "machine": state.machine(),
                "hosts": hosts,
                "setting_up": state.setting_up(),
            })
        })),
        // Setting containers up here: what would be done, and doing it
        // once a person agreed to exactly that.
        (&Method::POST, ["api", "hosts", "this-machine", "containers", "plan"]) => {
            let wanted = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<MachineWanted>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(wanted) => wanted,
                Err(error) => return error_response(&error),
            };
            json_result(state.plan_containers(wanted).await)
        }
        (&Method::POST, ["api", "hosts", "this-machine", "containers", "set-up"]) => {
            let plan_id = match read_json(request).await {
                Ok(body) => body["plan_id"].as_str().unwrap_or_default().to_owned(),
                Err(error) => return error_response(&error),
            };
            json_result(state.set_containers_up(&plan_id))
        }
        (&Method::POST, ["api", "hosts", "this-machine", "look"]) => {
            match state.look_at_the_machine().await {
                Ok(_) => json_result(
                    state
                        .hosts()
                        .map(|hosts| json!({ "machine": state.machine(), "hosts": hosts })),
                ),
                Err(error) => error_response(&error),
            }
        }
        (&Method::GET, ["api", "mcp-servers"]) => json_result(
            state
                .mcp_servers()
                .map(|servers| json!({ "servers": servers })),
        ),
        (&Method::POST, ["api", "mcp-servers"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<DeclareMcpServerBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.declare_mcp_server(&body))
        }
        (&Method::DELETE, ["api", "mcp-servers", name]) => {
            json_result(state.forget_mcp_server(name))
        }
        // The places a model is served from, set up once for every profile.
        (&Method::GET, ["api", "model-providers"]) => {
            json_result(state.providers_standing().map(|(providers, kept_by)| {
                json!({
                    "providers": providers,
                    "kept_by": kept_by,
                    // What a provider can be opened by, for whoever adds one.
                    "key_kinds": crate::secret_types(),
                })
            }))
        }
        // A provider's key: given once, never read back.
        (&Method::PUT, ["api", "model-providers", id, "key"]) => {
            let id = (*id).to_owned();
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<GiveKeyBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.give_provider_key(&id, &body))
        }
        (&Method::DELETE, ["api", "model-providers", id, "key"]) => {
            json_result(state.take_provider_key(id).map(|()| json!({ "taken": id })))
        }
        (&Method::POST, ["api", "model-providers"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<DeclareModelProviderBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.declare_model_provider(&body))
        }
        (&Method::DELETE, ["api", "model-providers", id]) => json_result(
            state
                .forget_model_provider(id)
                .map(|()| json!({ "forgotten": id })),
        ),
        // What the agent offers to configure on a session - its model, its
        // mode - and choosing among it. A choice the agent refuses is a 409
        // with the agent's sentence; the connection stays.
        (&Method::GET, ["api", "connections", cid, "options"]) => {
            json_result(state.connection_options(cid).await)
        }
        (&Method::POST, ["api", "connections", cid, "options"]) => {
            #[derive(Deserialize)]
            struct SetOptionBody {
                config_id: String,
                value: Value,
            }
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<SetOptionBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(
                state
                    .set_connection_option(cid, &body.config_id, &body.value)
                    .await
                    .map(|options| json!({ "config_options": options })),
            )
        }
        (&Method::POST, ["api", "connections", cid, "mode"]) => {
            #[derive(Deserialize)]
            struct SetModeBody {
                mode_id: String,
            }
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<SetModeBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(
                state
                    .set_connection_mode(cid, &body.mode_id)
                    .await
                    .map(|modes| json!({ "modes": modes })),
            )
        }
        (&Method::GET, ["api", "terminals"]) => {
            let terminals = state.terminals().await;
            json_result(Ok(json!({ "terminals": terminals })))
        }
        (&Method::POST, ["api", "terminals"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<OpenTerminalBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.open_terminal(&body).await)
        }
        (&Method::GET, ["api", "terminals", terminal_id, "output"]) => {
            let after = query_param(query.as_deref(), "after")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0_u64);
            let wait = query_param(query.as_deref(), "wait_ms")
                .and_then(|value| value.parse().ok())
                .map_or(Duration::from_secs(20), Duration::from_millis);
            json_result(state.terminal_output(terminal_id, after, wait).await)
        }
        (&Method::POST, ["api", "terminals", terminal_id, "input"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<TerminalInputBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.terminal_input(terminal_id, &body).await)
        }
        (&Method::POST, ["api", "terminals", terminal_id, "size"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<TerminalSizeBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.resize_terminal(terminal_id, body).await)
        }
        (&Method::DELETE, ["api", "terminals", terminal_id]) => {
            json_result(state.close_terminal(terminal_id).await)
        }
        (&Method::GET, ["api", "onboarding"]) => json_result(Ok(state.onboarding())),
        (&Method::GET, ["api", "agents", agent_id, "install-plan"]) => {
            json_result(state.agent_install_plan(agent_id))
        }
        // Schedules: messages that arrive on time.
        (&Method::GET, ["api", "schedules"]) => json_result(
            state
                .schedules_shown(query_param(query.as_deref(), "agent"))
                .await
                .map(|(schedules, runs)| json!({ "schedules": schedules, "runs": runs })),
        ),
        (&Method::POST, ["api", "schedules"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<NewScheduleBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.make_schedule(body).await)
        }
        (&Method::PATCH, ["api", "schedules", schedule_id]) => {
            let schedule_id = (*schedule_id).to_owned();
            let change = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<crate::TimedMessageChange>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(change) => change,
                Err(error) => return error_response(&error),
            };
            json_result(state.change_schedule(&schedule_id, change).await)
        }
        (&Method::DELETE, ["api", "schedules", schedule_id]) => json_result(
            state
                .forget_schedule(schedule_id)
                .await
                .map(|()| json!({ "forgotten": schedule_id })),
        ),
        // Channels: how people reach an agent from a messenger.
        (&Method::GET, ["api", "channels"]) => json_result(state.channels_standing().await),
        (&Method::POST, ["api", "channels"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<AddChannelBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.add_channel(body).await)
        }
        (&Method::DELETE, ["api", "channels", channel_id]) => {
            let channel_id = (*channel_id).to_owned();
            json_result(
                state
                    .remove_channel(&channel_id)
                    .await
                    .map(|()| json!({ "removed": channel_id })),
            )
        }
        (&Method::POST, ["api", "channels", channel_id, "start"]) => {
            let channel_id = (*channel_id).to_owned();
            match state.start_channel(&channel_id).await {
                Ok(()) => json_result(state.channels_standing().await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::PATCH, ["api", "channels", channel_id]) => {
            let channel_id = (*channel_id).to_owned();
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<ChangeChannelBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.change_channel(&channel_id, body).await)
        }
        // A messenger delivers at a channel's door; the channel verifies
        // it. Open to anybody when served at an address, as the door is.
        (&Method::POST, ["api", "channels", channel_id, "receive"]) => {
            let channel_id = (*channel_id).to_owned();
            let headers: BTreeMap<String, String> = request
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .to_str()
                        .ok()
                        .map(|value| (name.as_str().to_ascii_lowercase(), value.to_owned()))
                })
                .collect();
            let body = match body_text_of(request).await {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            match state.receive_at_door(&channel_id, headers, body).await {
                Ok(()) => json_result(Ok(json!({ "received": true }))),
                Err(error) => error_response(&error),
            }
        }
        // The channel's Mini App, from inside the messenger: who opened it
        // is said by the messenger's signature, which the channel checks.
        (
            &Method::OPTIONS | &Method::GET | &Method::POST,
            ["api", "channels", _, "app"] | ["api", "channels", _, "app", ..],
        ) => match route_the_app(state, &method, &segments, query.as_deref(), request).await {
            Ok(response) => response,
            Err(_) => respond_json(StatusCode::NOT_FOUND, &json!({ "error": "not found" })),
        },
        // How this Workbench is reached from outside, and a tunnel opened
        // or closed by hand.
        (&Method::GET, ["api", "reach"]) => json_result(state.reach_standing()),
        (&Method::POST, ["api", "reach", "tunnel"]) => {
            let package = match read_json(request).await {
                Ok(body) => body
                    .get("package")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                Err(_) => None,
            };
            match state.open_tunnel(package.as_deref()).await {
                Ok(_) => json_result(state.reach_standing()),
                Err(error) => error_response(&error),
            }
        }
        (&Method::DELETE, ["api", "reach", "tunnel"]) => {
            state.close_tunnel().await;
            json_result(state.reach_standing())
        }
        // A guest who wrote to the bot is let into the chat the owner has
        // with it.
        (&Method::POST, ["api", "channels", channel_id, "guests", guest, "let-in"]) => {
            let (channel_id, guest) = ((*channel_id).to_owned(), (*guest).to_owned());
            match state.let_guest_in_at(&channel_id, &guest).await {
                Ok(_) => json_result(state.channels_standing().await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::GET, ["api", "time"]) => json_result(state.keeper().await),
        // A scheduler outside knocks: it is time to look. It carries
        // nothing and is told how many messages were taken up.
        (&Method::POST, ["api", "time", "due"]) => json_result(state.look_at_a_knock().await),
        // Who keeps time: the system's scheduler is turned on and off, a
        // keeper is made the one agents have, and one is chosen for an
        // agent.
        (&Method::POST, ["api", "time", "keepers", "system", "on"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<TurnOnBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            match state.turn_the_system_on(body).await {
                Ok(()) => json_result(state.keeper().await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "time", "keepers", "system", "off"]) => {
            match state.turn_the_system_off().await {
                Ok(()) => json_result(state.keeper().await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "time", "keepers", keeper, "default"]) => {
            match state.make_default_keeper(keeper).await {
                Ok(()) => json_result(state.keeper().await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::PUT, ["api", "time", "agents", profile_id]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<ChooseKeeperBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            match state.choose_keeper(profile_id, body).await {
                Ok(()) => json_result(state.keeper().await),
                Err(error) => error_response(&error),
            }
        }
        // An agent's files: the folder it works in as a tree, a file as
        // it is opened, saved and kept, and what a person does to the tree.
        (&Method::GET, ["api", "profiles", profile_id, "tree"]) => {
            let dir = query_param(query.as_deref(), "dir").unwrap_or_default();
            json_result(
                state
                    .agent_tree(profile_id, &percent_decode(&dir))
                    .await
                    .map(|entries| json!({ "entries": entries })),
            )
        }
        (&Method::POST, ["api", "profiles", profile_id, "tree"]) => {
            let profile_id = (*profile_id).to_owned();
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<ChangeTreeBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(
                state
                    .change_agent_tree(&profile_id, body)
                    .await
                    .map(|()| json!({ "done": true })),
            )
        }
        (&Method::GET, ["api", "profiles", profile_id, "file"]) => {
            let path = percent_decode(&query_param(query.as_deref(), "path").unwrap_or_default());
            if query_param(query.as_deref(), "as").as_deref() == Some("it-is") {
                match state.agent_file_as_it_is(profile_id, &path).await {
                    Ok((bytes, media_type)) => respond_bytes(StatusCode::OK, &media_type, bytes),
                    Err(error) => error_response(&error),
                }
            } else {
                json_result(state.agent_file(profile_id, &path).await)
            }
        }
        (&Method::PUT, ["api", "profiles", profile_id, "file"]) => {
            let profile_id = (*profile_id).to_owned();
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<SaveFileBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.save_agent_file(&profile_id, body).await)
        }
        (&Method::GET, ["api", "profiles", profile_id, "files"]) => {
            json_result(state.profile_files(profile_id).await)
        }
        (&Method::GET, ["api", "profiles", profile_id, "files", area, name]) => {
            let name = percent_decode(name);
            match state.profile_file(profile_id, area, &name).await {
                Ok((bytes, media_type)) => respond_bytes(StatusCode::OK, &media_type, bytes),
                Err(error) => error_response(&error),
            }
        }
        // What an agent has where it lives: as it was found last, and
        // looked at now.
        (&Method::GET, ["api", "profiles", profile_id, "look"]) => json_result(
            state
                .look_inside_kept(profile_id)
                .map(|look| json!({ "look": look })),
        ),
        (&Method::POST, ["api", "profiles", profile_id, "look"]) => json_result(
            state
                .look_inside(profile_id)
                .await
                .map(|look| json!({ "look": look })),
        ),
        (&Method::GET, ["api", "profiles", profile_id, "handshake"]) => {
            json_result(state.profile_handshake(profile_id).await)
        }
        (&Method::GET, ["api", "profiles", profile_id, "secrets"]) => {
            json_result(state.profile_secrets(profile_id))
        }
        (&Method::PUT, ["api", "profiles", profile_id, "secrets"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<ProfileSecretBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.set_profile_secret(
                profile_id,
                &body.type_id,
                &body.label,
                body.name.as_deref(),
                &body.value,
            ))
        }
        // An agent is removed: what it said and wrote stays.
        (&Method::DELETE, ["api", "profiles", profile_id]) => {
            json_result(state.remove_agent(profile_id).await)
        }
        (&Method::DELETE, ["api", "profiles", profile_id, "secrets", name]) => {
            json_result(state.remove_profile_secret(profile_id, name))
        }
        // Installing the agent the person already chose, from the product
        // instead of a second terminal.
        (&Method::POST, ["api", "agents", "install"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<InstallAgentBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            let state = Arc::clone(state);
            blocking_json(move || state.install_agent(&body.agent_id, &body.plan_id)).await
        }
        // What packages bring and projects require: tools installed from
        // declarations by a plan the person confirms, and a project's vault.
        // Everything installed here, of every kind, one receipt each.
        (&Method::GET, ["api", "installs"]) => json_result(state.installs()),
        // The store: what the indexes offer, the plan for one entry, the
        // install against it, and the catalogs a person adds by URL.
        (&Method::GET, ["api", "store"]) => {
            let state = Arc::clone(state);
            blocking_json(move || state.store()).await
        }
        (&Method::POST, ["api", "store", "plan"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<StorePlanBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            let state = Arc::clone(state);
            blocking_json(move || state.store_plan(&body)).await
        }
        (&Method::POST, ["api", "store", "install"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<StoreInstallBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            let state = Arc::clone(state);
            blocking_json(move || {
                state
                    .store_install(&body)
                    .map(|receipts| json!({"installed": receipts}))
            })
            .await
        }
        (&Method::POST, ["api", "store", "remove"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<StoreRemoveBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            let state = Arc::clone(state);
            blocking_json(move || state.store_remove(&body)).await
        }
        (&Method::POST, ["api", "store", "indexes"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<AddIndexBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            let state = Arc::clone(state);
            blocking_json(move || state.add_index(&body)).await
        }
        (&Method::DELETE, ["api", "store", "indexes", slug]) => json_result(
            state
                .forget_index(slug)
                .map(|()| json!({ "forgotten": slug })),
        ),
        // The skills installed here, as a profile takes a copy of one.
        (&Method::GET, ["api", "skills"]) => json_result(state.installed_skills()),
        (&Method::GET, ["api", "spaces"]) => json_result(state.spaces().await),
        (&Method::POST, ["api", "spaces", server, "open"]) => match read_json(request).await {
            Ok(body) => match body.get("uri").and_then(Value::as_str) {
                Some(uri) => json_result(state.space_open(server, uri).await),
                None => error_response(&WorkbenchShellError::Invalid("App uri required".into())),
            },
            Err(error) => error_response(&error),
        },
        // An App of a space: its relay and its close. No agent session
        // takes part, so these are not under a connection.
        (&Method::POST, ["api", "space-apps", app, "rpc"]) => match read_json(request).await {
            Ok(body) => json_result(state.space_app_rpc(app, body).await),
            Err(error) => error_response(&error),
        },
        (&Method::POST, ["api", "space-apps", app, "close"]) => {
            json_result(state.space_app_close(app).await)
        }
        (&Method::POST, ["api", "content"]) => {
            json_result(upload_content(state, request, query.as_deref()).await)
        }
        (&Method::GET, ["api", "content", descriptor_id, "metadata"]) => {
            json_result(state.content.load(descriptor_id).await)
        }
        (&Method::GET | &Method::HEAD, ["api", "content", descriptor_id]) => {
            let download = query_param(query.as_deref(), "download").as_deref() == Some("1");
            match serve_content(state, request, descriptor_id, download).await {
                Ok(response) => response,
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "profiles", "local"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<CreateLocalProfileBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.create_local_profile(
                &body.agent_id,
                body.profile_id.as_deref(),
                body.setup,
            ))
        }
        (&Method::GET, ["apps-bridge.js"]) => match state.apps_bundle.get() {
            Some(bundle) => match std::fs::read_to_string(bundle.join("apps-bridge.js")) {
                Ok(bundled) => respond(StatusCode::OK, "application/javascript", bundled),
                Err(error) => respond(
                    StatusCode::NOT_FOUND,
                    "text/plain",
                    format!("apps bundle unreadable: {error}"),
                ),
            },
            // The product ships the official AppBridge it was built against,
            // so a domain App opens without a flag and without a second
            // install step. `--apps-bundle` still overrides it for development.
            None if WorkbenchShellState::apps_enabled() => respond(
                StatusCode::OK,
                "application/javascript",
                APPS_BRIDGE.to_owned(),
            ),
            // No bundle configured: the Apps panel stays disabled - the
            // honest App-disabled mode, not an error.
            None => respond(
                StatusCode::NOT_FOUND,
                "text/plain",
                "apps bundle not configured".into(),
            ),
        },
        (&Method::GET, ["api", "connections", connection_id, "apps"]) => {
            json_result(state.apps_list(connection_id).await)
        }
        (&Method::POST, ["api", "connections", connection_id, "apps", "interactions"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<StartAppInteractionBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(
                state
                    .start_app_interaction(
                        connection_id,
                        &body.server_name,
                        &body.tool,
                        body.arguments,
                    )
                    .await,
            )
        }
        (
            &Method::POST,
            [
                "api",
                "connections",
                connection_id,
                "apps",
                "interactions",
                interaction_id,
            ],
        ) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<AnswerAppInteractionBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(
                state
                    .answer_app_interaction(
                        connection_id,
                        interaction_id,
                        body.action,
                        body.content,
                    )
                    .await,
            )
        }
        (
            &Method::GET,
            [
                "api",
                "connections",
                connection_id,
                "apps",
                "observations",
                "next",
            ],
        ) => {
            let after = query_param(query.as_deref(), "after")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let wait = query_param(query.as_deref(), "wait_ms")
                .and_then(|value| value.parse().ok())
                .map_or(Duration::from_secs(25), Duration::from_millis);
            json_result(state.next_observed_app(connection_id, after, wait).await)
        }
        (
            &Method::GET,
            [
                "api",
                "connections",
                connection_id,
                "apps",
                "observations",
                observation_id,
            ],
        ) => {
            let wait = query_param(query.as_deref(), "wait_ms")
                .and_then(|value| value.parse().ok())
                .map_or(Duration::from_secs(25), Duration::from_millis);
            json_result(
                state
                    .observed_app_status(connection_id, observation_id, wait)
                    .await,
            )
        }
        (&Method::POST, ["api", "connections", connection_id, "apps", "open"]) => {
            let connection_id = (*connection_id).to_owned();
            let body = match read_json(request).await {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            let (Some(server_name), Some(uri)) = (
                body.get("server_name").and_then(Value::as_str),
                body.get("uri").and_then(Value::as_str),
            ) else {
                return error_response(&WorkbenchShellError::Invalid(
                    "open needs server_name and uri".into(),
                ));
            };
            json_result(state.app_open(&connection_id, server_name, uri).await)
        }
        (&Method::POST, ["api", "connections", connection_id, "apps", app_id, "rpc"]) => {
            let connection_id = (*connection_id).to_owned();
            let app_id = (*app_id).to_owned();
            let body = match read_json(request).await {
                Ok(body) => body,
                Err(error) => return error_response(&error),
            };
            json_result(state.app_rpc(&connection_id, &app_id, body).await)
        }
        (&Method::POST, ["api", "connections", connection_id, "apps", app_id, "close"]) => {
            json_result(
                state
                    .app_close(connection_id, app_id)
                    .await
                    .map(|()| json!({ "closed": true })),
            )
        }
        _ => respond(StatusCode::NOT_FOUND, "text/plain", "not found".into()),
    }
}

/// The spec-exact sandbox proxy page, served from the SECOND origin only.
const SANDBOX_HTML: &str = include_str!("../web/apps-host/sandbox.html");

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or("");
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    decoded.push(byte);
                    index += 3;
                } else {
                    decoded.push(bytes[index]);
                    index += 1;
                }
            }
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Bytes INTO the server's workspace, the twin of the blob route: an App
/// records or imports something and hands it to its server as a file under
/// a host-chosen name, then names that file to the server's own tool. The
/// View has an opaque origin, so a non-simple request is preflighted and
/// both need the CORS grant.
async fn route_upload(
    state: &WorkbenchShellState,
    connection_id: &str,
    app_id: &str,
    request: Request<AskedBody>,
) -> Response<ShellBody> {
    match *request.method() {
        Method::OPTIONS => Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("access-control-allow-origin", "*")
            .header("access-control-allow-methods", "POST, DELETE, OPTIONS")
            .header("access-control-allow-headers", "content-type")
            .header("access-control-max-age", "600")
            .body(Full::new(Bytes::new()).map_err(infallible_to_io).boxed())
            .expect("preflight response"),
        Method::POST => match state.app_upload_root(connection_id, app_id).await {
            Ok(root) => match receive_upload(&root, request.into_body()).await {
                Ok((workspace_path, byte_length)) => cors_json(
                    StatusCode::OK,
                    &json!({"workspace_path": workspace_path, "byte_length": byte_length}),
                ),
                Err(UploadRefusal::TooLarge) => cors_json(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    &json!({"error": format!("an upload is at most {UPLOAD_LIMIT} bytes")}),
                ),
                Err(UploadRefusal::Failed(error)) => {
                    cors_json(StatusCode::INTERNAL_SERVER_ERROR, &json!({"error": error}))
                }
            },
            Err(_) => cors_json(StatusCode::NOT_FOUND, &json!({"error": "not found"})),
        },
        Method::DELETE => {
            let named =
                query_param(request.uri().query(), "path").map(|value| percent_decode(&value));
            match (state.app_upload_root(connection_id, app_id).await, named) {
                (Ok(root), Some(named)) => match remove_upload(&root, &named).await {
                    Ok(()) => Response::builder()
                        .status(StatusCode::NO_CONTENT)
                        .header("access-control-allow-origin", "*")
                        .body(Full::new(Bytes::new()).map_err(infallible_to_io).boxed())
                        .expect("delete response"),
                    Err(error) => cors_json(StatusCode::NOT_FOUND, &json!({"error": error})),
                },
                _ => cors_json(StatusCode::NOT_FOUND, &json!({"error": "not found"})),
            }
        }
        _ => respond(StatusCode::NOT_FOUND, "text/plain", "not found".into()),
    }
}

/// A View served as a document of the sandbox origin, and the files under
/// its path. The trailing slash is the View's: relative URLs in it resolve
/// to `view/<file>`.
async fn route_app_view(
    state: &WorkbenchShellState,
    connection_id: &str,
    app_id: &str,
    rest: &[&str],
) -> Response<ShellBody> {
    if rest.is_empty() || rest == [""] {
        return match state.app_view(connection_id, app_id).await {
            Ok((html, csp)) => {
                let mut builder = Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "text/html;charset=utf-8")
                    .header("cross-origin-embedder-policy", "credentialless")
                    .header("cross-origin-resource-policy", "cross-origin")
                    .header("cache-control", "no-store");
                if !csp.is_empty() {
                    builder = builder.header("content-security-policy", csp);
                }
                builder
                    .body(
                        Full::new(Bytes::from(html))
                            .map_err(infallible_to_io)
                            .boxed(),
                    )
                    .expect("view response")
            }
            Err(_) => respond(StatusCode::NOT_FOUND, "text/plain", "not found".into()),
        };
    }
    let file = percent_decode(&rest.join("/"));
    match state.app_file_resource(connection_id, app_id, &file).await {
        // A dedicated worker's script must carry the embedder policy of the
        // document that spawns it, or the browser refuses the worker.
        Ok((bytes, media_type)) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", media_type)
            .header("cross-origin-embedder-policy", "credentialless")
            .header("cache-control", "no-store")
            .body(
                Full::new(Bytes::from(bytes))
                    .map_err(infallible_to_io)
                    .boxed(),
            )
            .expect("view file response"),
        Err(_) => respond(StatusCode::NOT_FOUND, "text/plain", "not found".into()),
    }
}

/// The sandbox origin serves the proxy page, with the Rust-resolved per-app
/// CSP echoed back as a REAL response header (header injection impossible:
/// control characters are stripped), and the script resources an open App's
/// server lists (`/apps/{connection}/{app}/resources?uri=ui://...`), so a
/// worklet module can be loaded from a URL the App CSP's `'self'` covers.
/// `/api` does not exist on this origin.
async fn route_sandbox(
    state: &WorkbenchShellState,
    request: Request<AskedBody>,
) -> Response<ShellBody> {
    let path = request.uri().path().to_owned();
    let segments = path.trim_start_matches('/').split('/').collect::<Vec<_>>();
    if let ["apps", connection_id, app_id, "upload"] = segments.as_slice() {
        let (connection_id, app_id) = ((*connection_id).to_owned(), (*app_id).to_owned());
        return route_upload(state, &connection_id, &app_id, request).await;
    }
    if request.method() != Method::GET {
        return respond(StatusCode::NOT_FOUND, "text/plain", "not found".into());
    }
    if let ["apps", connection_id, app_id, "resources"] = segments.as_slice() {
        let Some(uri) =
            query_param(request.uri().query(), "uri").map(|value| percent_decode(&value))
        else {
            return respond(StatusCode::NOT_FOUND, "text/plain", "not found".into());
        };
        return match state.app_script_resource(connection_id, app_id, &uri).await {
            Ok(text) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", workbench_apps::SCRIPT_RESOURCE_MIME)
                // The App View has an opaque origin: a module-script fetch
                // arrives with `Origin: null` and needs an explicit grant.
                .header("access-control-allow-origin", "*")
                .header("cache-control", "no-store")
                .body(
                    Full::new(Bytes::from(text))
                        .map_err(infallible_to_io)
                        .boxed(),
                )
                .expect("script resource response"),
            Err(_) => respond(StatusCode::NOT_FOUND, "text/plain", "not found".into()),
        };
    }
    if let ["apps", connection_id, app_id, "blob"] = segments.as_slice() {
        let Some(uri) =
            query_param(request.uri().query(), "uri").map(|value| percent_decode(&value))
        else {
            return respond(StatusCode::NOT_FOUND, "text/plain", "not found".into());
        };
        return match state.app_blob_resource(connection_id, app_id, &uri).await {
            Ok((bytes, media_type)) => Response::builder()
                .status(StatusCode::OK)
                .header("content-type", media_type)
                // The View has an opaque origin, so a fetch arrives with
                // `Origin: null` and needs an explicit grant.
                .header("access-control-allow-origin", "*")
                // Content-addressed: these bytes cannot become other bytes.
                .header("cache-control", "public, max-age=31536000, immutable")
                .body(
                    Full::new(Bytes::from(bytes))
                        .map_err(infallible_to_io)
                        .boxed(),
                )
                .expect("blob resource response"),
            Err(_) => respond(StatusCode::NOT_FOUND, "text/plain", "not found".into()),
        };
    }
    if let ["apps", connection_id, app_id, "view", rest @ ..] = segments.as_slice() {
        return route_app_view(state, connection_id, app_id, rest).await;
    }
    if path != "/sandbox" {
        return respond(StatusCode::NOT_FOUND, "text/plain", "not found".into());
    }
    let csp = query_param(request.uri().query(), "csp")
        .map(|value| percent_decode(&value))
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    // The proxy document is embedded by the isolated shell, so it carries
    // the embedder policy itself and says it may be embedded cross-origin.
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/html;charset=utf-8")
        .header("cross-origin-embedder-policy", "credentialless")
        .header("cross-origin-resource-policy", "cross-origin");
    if !csp.is_empty() {
        builder = builder.header("content-security-policy", csp);
    }
    builder
        .body(
            Full::new(Bytes::from(SANDBOX_HTML.to_owned()))
                .map_err(infallible_to_io)
                .boxed(),
        )
        .expect("static sandbox response")
}

/// The most an App may upload in one request: the same ceiling the Cycle's
/// own workspace-path ingest accepts, so a file that lands here can always
/// be named to the server.
pub const UPLOAD_LIMIT: u64 = 256 * 1024 * 1024;

/// Where uploads land inside a server's workspace; a name under it is the
/// only path an App ever learns.
const UPLOAD_DIRECTORY: &str = "uploads";

enum UploadRefusal {
    TooLarge,
    Failed(String),
}

static UPLOAD_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Stream a request body into a fresh file under the workspace's upload
/// directory, refusing past the limit. Answers the workspace-relative path
/// (portable, forward slashes) and the byte count.
async fn receive_upload(root: &Path, mut body: AskedBody) -> Result<(String, u64), UploadRefusal> {
    let directory = root.join(UPLOAD_DIRECTORY);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| UploadRefusal::Failed(error.to_string()))?;
    let name = format!(
        "u-{}-{:x}",
        nanos_now() / 1_000_000,
        UPLOAD_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let target = directory.join(&name);
    let mut file = tokio::fs::File::create(&target)
        .await
        .map_err(|error| UploadRefusal::Failed(error.to_string()))?;
    let mut received: u64 = 0;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|error| UploadRefusal::Failed(error.to_string()))?;
        let Ok(data) = frame.into_data() else {
            continue;
        };
        received += data.len() as u64;
        if received > UPLOAD_LIMIT {
            drop(file);
            let _ = tokio::fs::remove_file(&target).await;
            return Err(UploadRefusal::TooLarge);
        }
        file.write_all(&data)
            .await
            .map_err(|error| UploadRefusal::Failed(error.to_string()))?;
    }
    file.flush()
        .await
        .map_err(|error| UploadRefusal::Failed(error.to_string()))?;
    Ok((format!("{UPLOAD_DIRECTORY}/{name}"), received))
}

/// Remove one upload by the path the upload route answered - and nothing
/// else: the path must be a plain name under the upload directory.
async fn remove_upload(root: &Path, named: &str) -> Result<(), String> {
    let Some(name) = named.strip_prefix(&format!("{UPLOAD_DIRECTORY}/")) else {
        return Err("not an upload path".into());
    };
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || !name.starts_with("u-")
    {
        return Err("not an upload path".into());
    }
    tokio::fs::remove_file(root.join(UPLOAD_DIRECTORY).join(name))
        .await
        .map_err(|error| error.to_string())
}

fn cors_json(status: StatusCode, value: &Value) -> Response<ShellBody> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("access-control-allow-origin", "*")
        .body(
            Full::new(Bytes::from(value.to_string()))
                .map_err(infallible_to_io)
                .boxed(),
        )
        .expect("json response")
}

/// Serve the generic Workbench shell over HTTP on `bind` (port 0 for an
/// ephemeral port; the handle carries the real address).
///
/// # Errors
///
/// Returns an error when the address cannot be bound.
pub async fn serve_workbench_http(
    state: Arc<WorkbenchShellState>,
    bind: SocketAddr,
) -> Result<WorkbenchShellHandle, String> {
    serve_workbench_http_with_apps(state, bind, None).await
}

/// Serve the shell plus the Apps host surfaces: a SECOND ephemeral listener
/// on 127.0.0.1 is the sandbox origin (origin = scheme+host+port, so two
/// ports are two origins), and `apps_bundle` names the directory holding the
/// esbuild output `apps-bridge.js` (absent = the Apps panel stays disabled -
/// the honest App-disabled mode).
///
/// # Errors
///
/// Returns an error when an address cannot be bound.
pub async fn serve_workbench_http_with_apps(
    state: Arc<WorkbenchShellState>,
    bind: SocketAddr,
    apps_bundle: Option<PathBuf>,
) -> Result<WorkbenchShellHandle, String> {
    serve_workbench_http_with_apps_at(
        state,
        bind,
        (std::net::Ipv4Addr::LOCALHOST, 0).into(),
        apps_bundle,
    )
    .await
}

/// As [`serve_workbench_http_with_apps`], with the sandbox origin bound at
/// `sandbox_bind` (port 0 for an ephemeral port). A fixed port lets an
/// operator forward both origins to a remote machine; the origin stays a
/// loopback one.
///
/// # Errors
///
/// Returns an error when an address cannot be bound.
pub async fn serve_workbench_http_with_apps_at(
    state: Arc<WorkbenchShellState>,
    bind: SocketAddr,
    sandbox_bind: SocketAddr,
    apps_bundle: Option<PathBuf>,
) -> Result<WorkbenchShellHandle, String> {
    serve_workbench(
        state,
        Listening {
            bind,
            sandbox_bind,
            sandbox_at: None,
            tls: None,
            apps_bundle,
        },
    )
    .await
}

/// How a Workbench listens.
pub struct Listening {
    pub bind: SocketAddr,
    /// Where what is drawn for Apps is listened for.
    pub sandbox_bind: SocketAddr,
    /// The origin a browser finds that at. On the machine a person sits at
    /// it is the listener itself; served at an address it is an address.
    pub sandbox_at: Option<String>,
    /// TLS, when the Workbench keeps the way to it closed by itself.
    pub tls: Option<Arc<tokio_rustls::rustls::ServerConfig>>,
    pub apps_bundle: Option<PathBuf>,
}

/// A handshake that does not finish in this long is let go of.
const A_HANDSHAKE: Duration = Duration::from_secs(10);

/// Accept connections until told to stop, and answer each request with
/// `route`. Where the request came from travels with it.
async fn accept_until_told<Route, Answer>(
    listener: tokio::net::TcpListener,
    tls: Option<tokio_rustls::TlsAcceptor>,
    mut told: tokio::sync::oneshot::Receiver<()>,
    route: Route,
) where
    Route: Fn(Request<AskedBody>) -> Answer + Clone + Send + 'static,
    Answer: Future<Output = Response<ShellBody>> + Send,
{
    let mut connections = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut told => break,
            accepted = listener.accept() => {
                let Ok((stream, from)) = accepted else { continue };
                let route = route.clone();
                let tls = tls.clone();
                connections.spawn(async move {
                    let service = hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                        let mut request = asked(request);
                        request.extensions_mut().insert(door::CameFrom(from));
                        let route = route.clone();
                        async move { Ok::<_, Infallible>(route(request).await) }
                    });
                    let served = hyper::server::conn::http1::Builder::new();
                    match tls {
                        None => {
                            let _ = served
                                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                                .await;
                        }
                        Some(tls) => {
                            let Ok(Ok(stream)) =
                                tokio::time::timeout(A_HANDSHAKE, tls.accept(stream)).await
                            else {
                                return;
                            };
                            let _ = served
                                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                                .await;
                        }
                    }
                });
            }
            completed = connections.join_next(), if !connections.is_empty() => {
                let _ = completed;
            }
        }
    }
    connections.shutdown().await;
}

/// Serve the Workbench as `listening` says.
///
/// # Errors
///
/// Returns an error when an address cannot be bound.
pub async fn serve_workbench(
    state: Arc<WorkbenchShellState>,
    listening: Listening,
) -> Result<WorkbenchShellHandle, String> {
    let listener = tokio::net::TcpListener::bind(listening.bind)
        .await
        .map_err(|error| format!("{}: {error}", listening.bind))?;
    let local_addr = listener.local_addr().map_err(|error| error.to_string())?;
    let sandbox_listener = tokio::net::TcpListener::bind(listening.sandbox_bind)
        .await
        .map_err(|error| format!("{}: {error}", listening.sandbox_bind))?;
    let sandbox_addr = sandbox_listener
        .local_addr()
        .map_err(|error| error.to_string())?;
    let sandbox_at = listening
        .sandbox_at
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", sandbox_addr.port()));
    state.set_sandbox(format!("{sandbox_at}/sandbox"), sandbox_at);
    if let Some(bundle) = listening.apps_bundle {
        state.set_apps_bundle(bundle);
    }
    let tls = listening.tls.map(tokio_rustls::TlsAcceptor::from);
    let (sandbox_shutdown, sandbox_told) = tokio::sync::oneshot::channel();
    let sandbox_state = Arc::clone(&state);
    let sandbox_task = tokio::spawn(accept_until_told(
        sandbox_listener,
        tls.clone(),
        sandbox_told,
        move |request| {
            let state = Arc::clone(&sandbox_state);
            async move { route_sandbox(&state, request).await }
        },
    ));
    let (shutdown, told) = tokio::sync::oneshot::channel();
    let shell_state = Arc::clone(&state);
    let task = tokio::spawn(accept_until_told(listener, tls, told, move |request| {
        let state = Arc::clone(&shell_state);
        async move {
            let mut response = Box::pin(route_shell(&state, request)).await;
            if state.is_served_over_tls() {
                // A browser that came once over TLS comes over TLS from
                // now on, whatever address it is given.
                response.headers_mut().insert(
                    "strict-transport-security",
                    hyper::header::HeaderValue::from_static("max-age=31536000"),
                );
            }
            response
        }
    }));
    Ok(WorkbenchShellHandle {
        local_addr,
        sandbox_addr,
        state,
        task: Some(task),
        sandbox_task: Some(sandbox_task),
        shutdown: Some(shutdown),
        sandbox_shutdown: Some(sandbox_shutdown),
    })
}

/// Read a refused handshake out of a lane nothing has projected yet.
///
/// The words come from two events the session publishes around `initialize`:
/// what this host advertised, and what the agent answered. An agent that
/// refuses because "the client did not advertise X" is contradicting the
/// first, so both halves belong in the one sentence the person reads.
fn handshake_refusal(
    events: &mut tokio::sync::mpsc::UnboundedReceiver<crate::NativeSessionEvent>,
) -> Option<String> {
    let mut advertised = None;
    let mut refusal = None;
    while let Ok(event) = events.try_recv() {
        match event.kind.as_str() {
            "acp/initialize_requested" => advertised = Some(event.payload),
            "acp/initialize_refused" => {
                refusal = event
                    .payload
                    .get("error")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
            }
            _ => {}
        }
    }
    let refusal = refusal?;
    Some(match advertised {
        Some(advertised) => format!(
            "the agent refused the handshake ({refusal}); this host \
             advertised {advertised}"
        ),
        None => format!("the agent refused the handshake ({refusal})"),
    })
}

#[cfg(test)]
mod tests {
    use hyper::HeaderMap;

    use super::from_the_workbenchs_own_page;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for (name, value) in pairs {
            headers.insert(
                hyper::header::HeaderName::from_bytes(name.as_bytes()).expect("a header name"),
                value.parse().expect("a header value"),
            );
        }
        headers
    }

    #[test]
    fn the_workbenchs_own_page_is_let_in_by_whichever_name_it_was_opened_under() {
        assert!(from_the_workbenchs_own_page(&headers(&[
            ("host", "127.0.0.1:8080"),
            ("origin", "http://127.0.0.1:8080"),
        ])));
        assert!(from_the_workbenchs_own_page(&headers(&[
            ("host", "localhost:8080"),
            ("origin", "http://localhost:8080"),
        ])));
    }

    #[test]
    fn a_page_on_another_site_is_refused_however_it_states_itself() {
        for origin in [
            "http://not-the-workbench.example",
            // The same name on another port is another origin.
            "http://127.0.0.1:8081",
            // An opaque origin: what a sandboxed frame sends. It is stated,
            // and it is not this page.
            "null",
            // A scheme this server does not speak, and a stated origin that
            // is not an origin at all.
            "file://",
            "127.0.0.1:8080",
        ] {
            assert!(
                !from_the_workbenchs_own_page(&headers(&[
                    ("host", "127.0.0.1:8080"),
                    ("origin", origin),
                ])),
                "{origin} was let in"
            );
        }
    }

    #[test]
    fn a_request_that_states_no_origin_is_not_what_this_refuses() {
        // `curl`, an MCP client, the product's own tools. They are not
        // browser cross-site requests; which program on this machine may
        // call is a different lock.
        assert!(from_the_workbenchs_own_page(&headers(&[(
            "host",
            "127.0.0.1:8080"
        )])));
    }
}
