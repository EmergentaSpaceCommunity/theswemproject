//! Native workspace-bound ACP sessions and the optional SWEM Cycle seam.
//!
//! The generic session lifecycle knows ACP and caller-supplied MCP attachments,
//! but nothing about Cycle records. [`run_agent_session`] is the compatibility
//! composition that attaches the SWEM MCP server and its journal. Nothing here
//! interprets an agent reply as truth; callers verify the relevant external
//! state.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs::{self, File};
use std::future::Future;
use std::io::{BufWriter, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AuthenticateRequest, BooleanConfigOptionCapabilities, CancelNotification, ClientCapabilities,
    ClientSessionCapabilities, CloseSessionRequest, CompleteElicitationNotification, ContentBlock,
    CreateElicitationRequest, CreateElicitationResponse, CreateTerminalRequest,
    CreateTerminalResponse, ElicitationAcceptAction, ElicitationAction, ElicitationCapabilities,
    ElicitationMode, ElicitationScope, EmbeddedResourceResource, ErrorCode, FileSystemCapabilities,
    Implementation, InitializeRequest, KillTerminalRequest, KillTerminalResponse,
    ListSessionsRequest, LoadSessionRequest, McpServer, McpServerStdio, NewSessionRequest,
    PermissionOption, PermissionOptionId, PermissionOptionKind, PromptCapabilities, PromptRequest,
    ReadTextFileRequest, ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionValue, SessionConfigOptionsCapabilities, SessionConfigSelectOptions,
    SessionInfo, SessionModeId, SessionModeState, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionModeRequest, StopReason, TerminalExitStatus,
    TerminalOutputRequest, TerminalOutputResponse, TextContent, ToolCallContent, ToolCallId,
    ToolCallUpdate, ToolCallUpdateFields, ToolKind, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Agent, Client, ConnectionTo, DynConnectTo};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    EnvironmentGuaranteeEvidence, EnvironmentLease, EnvironmentTransport, LaunchCommand,
    NativeOutputProjection, NativeSessionEvent, SupplyError, SurfaceEventSource,
};

/// Name under which the SWEM MCP server is injected; agents expose its tools
/// as `mcp__<name>__<tool>` (Kimi Code) or an equivalent prefixed form.
pub const SWEM_MCP_SERVER_NAME: &str = "swem";

/// Host-owned permission policy for one seam session.
///
/// Allowed: tool calls that name one of the MCP servers this session attached,
/// and read-only workspace tools when `allow_read_only_tools` is set.
/// Rejected: every other effect (shell, file writes outside the authoring
/// workspace, MCP servers this session did not attach, network fetches). The
/// agent can therefore produce output only through what it was attached to.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SeamPermissionPolicy {
    /// The MCP servers whose tools this session may call without asking. It
    /// is a set rather than one name because a profile attaches several -
    /// its project's Cycle and whatever else the person declared - and a
    /// policy that knew only one of them would refuse the rest of the work
    /// the person attached them for.
    pub mcp_server_names: Vec<String>,
    pub allow_read_only_tools: bool,
    /// The agent's authoring workspace (`realization.typ`, "Свобода coding
    /// agent"): file edits whose every location lies under this absolute
    /// directory are allowed; authored files enter SWEM later through
    /// `put_rust_realization{workspace_dir}`. Shell stays rejected.
    pub authoring_workspace: Option<PathBuf>,
}

impl Default for SeamPermissionPolicy {
    fn default() -> Self {
        Self {
            mcp_server_names: vec![SWEM_MCP_SERVER_NAME.into()],
            allow_read_only_tools: true,
            authoring_workspace: None,
        }
    }
}

/// Absolute paths a tool call touches: ACP `locations` first, then the
/// conventional raw-input keys of file tools.
fn touched_paths(request: &RequestPermissionRequest) -> Vec<PathBuf> {
    let fields = &request.tool_call.fields;
    let mut paths = fields
        .locations
        .iter()
        .flatten()
        .map(|location| location.path.clone())
        .collect::<Vec<_>>();
    if let Some(input) = &fields.raw_input {
        for key in ["file_path", "path", "notebook_path", "old_path", "new_path"] {
            if let Some(path) = input.get(key).and_then(Value::as_str) {
                paths.push(PathBuf::from(path));
            }
        }
    }
    paths
}

fn inside(workspace: &Path, path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return false;
    }
    let Ok(workspace) = workspace.canonicalize() else {
        return false;
    };

    // Edits commonly target a file that does not exist yet. Resolve the
    // nearest existing ancestor so a symlink/junction in the authored path
    // cannot turn a lexically in-workspace filename into an outside write.
    let mut existing = path;
    while !existing.exists() {
        let Some(parent) = existing.parent() else {
            return false;
        };
        existing = parent;
    }
    existing
        .canonicalize()
        .is_ok_and(|resolved| resolved.starts_with(workspace))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PermissionDecision {
    pub tool_call_id: String,
    pub title: Option<String>,
    pub kind: Option<String>,
    /// ACP runtime permission outcome only. It is not Cycle semantic
    /// authority, MCP authentication, or proof that an effect occurred.
    pub allowed: bool,
    pub reason: String,
    pub selected_option: Option<String>,
    /// Whether this request's exact `toolCallId` had already appeared in a
    /// `session/update` for the active turn. Both variants remain
    /// agent-reported evidence, never authenticated MCP server identity.
    pub provenance: NativePermissionProvenance,
    pub decision_source: NativePermissionDecisionSource,
}

/// Correlation available to a surface when an ACP agent asks for permission.
///
/// ACP v1 only recommends a preceding tool-call update, so an uncorrelated
/// request is still displayable. Neither variant proves which MCP server (if
/// any) will receive the resulting operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePermissionProvenance {
    CorrelatedAgentReport,
    UncorrelatedAgentReport,
    /// The host raised this question itself, and the words in it are the
    /// host's own, taken from the request it is about to carry out. Nothing
    /// here was reported by the agent, so nothing here needs correlating.
    HostCallback,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativePermissionDecisionSource {
    DenyAllPolicy,
    LegacySeamPolicy,
    Surface,
    ProtocolGuard,
}

/// One unmodified ACP permission payload offered to a Workbench or channel.
///
/// `sequence` is local, ephemeral routing state. `tool_call` and `options` are
/// the official ACP v1 values; the host does not normalize them into a second
/// permission protocol.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NativePermissionRequest {
    pub sequence: u64,
    pub session_id: String,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
    pub provenance: NativePermissionProvenance,
}

#[derive(Debug)]
struct PendingNativePermission {
    option_ids: BTreeSet<String>,
    request: NativePermissionRequest,
    response: oneshot::Sender<Option<String>>,
}

/// What an agent asked a surface to do with one file, on its way out to the
/// surface that can actually do it.
///
/// This is not `Serialize`, and deliberately: `content` is the text of a
/// person's file, and every other request that crosses this host has a
/// redacted wire value taken of it somewhere. A descriptor that names the
/// method and the path is what belongs in the record; the text belongs only
/// in the live exchange. [`Self::descriptor`] is the only way to write one
/// down.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeFileCall {
    pub session_id: String,
    /// The exact ACP method: `fs/read_text_file` or `fs/write_text_file`.
    pub method: String,
    /// An absolute path, already checked against this connection's boundary.
    /// A path that reaches a surface is one the host was willing to forward.
    pub path: PathBuf,
    /// Reads only: the 1-based first line, and how many lines to take.
    pub line: Option<u32>,
    pub limit: Option<u32>,
    /// Writes only: the text to put in the file.
    pub content: Option<String>,
}

impl NativeFileCall {
    /// What this call looks like in the record: which file, asked for how,
    /// and never with what in it.
    #[must_use]
    pub fn descriptor(&self) -> Value {
        serde_json::json!({
            "session_id": self.session_id,
            "method": self.method,
            "path": self.path.display().to_string(),
            "line": self.line,
            "limit": self.limit,
        })
    }
}

/// One file call offered to the surface that owns this connection.
///
/// `sequence` is local, ephemeral routing state, exactly as a permission's
/// is: a surface answers by it, and cancelling the turn drains every one
/// still pending.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeFileRequest {
    pub sequence: u64,
    pub call: NativeFileCall,
}

/// What a surface did with a file call.
#[derive(Clone, Debug, PartialEq)]
pub enum NativeFileAnswer {
    /// The text a read returned.
    Text(String),
    /// A write happened.
    Written,
    /// The surface could not do it, in its own words. The agent hears this as
    /// an error on its own callback, which is what ACP leaves it able to act
    /// on inside the same turn.
    Refused(String),
}

#[derive(Debug)]
struct PendingNativeFile {
    request: NativeFileRequest,
    response: oneshot::Sender<Option<NativeFileAnswer>>,
}

/// What this connection may ask the surface behind it to do with files.
///
/// Both halves are needed and neither is optional on its own: the
/// capabilities are the exact methods the surface said it can carry out, and
/// the boundary is the directory a path must be inside. The boundary is not
/// the surface's to name - it comes from the profile, the same way the
/// working directory does - so a surface cannot widen it by asking.
#[derive(Clone, Debug)]
pub struct FileCallbacks {
    pub capabilities: FileSystemCapabilities,
    pub boundary: PathBuf,
}

/// What this connection may run, and in whose environment.
///
/// This is not the shape [`FileCallbacks`] has, and the difference is the
/// point. ACP puts both `fs/*` and `terminal/*` on the client because the
/// client owns the environment they name, but for files that environment can
/// be an editor's buffer, which nothing else can read, so the host asks the
/// surface. Nothing equivalent holds for a command: what a command needs is
/// the place the agent's work lives, and the harness owns it - a real pty in
/// the profile's workspace, with the profile's agent home and vault
/// (`workbench_shell/terminal.rs`). A browser tab owns no process at all, and
/// an editor's terminal is somewhere else entirely. So the host carries these
/// out itself.
///
/// Setting this is what advertises ACP's `terminal` capability - one boolean
/// for all five methods, as the protocol defines it. Leaving it `None`
/// refuses all five, which stays the default.
/// Whether a command has to be put to a person before it runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AskBeforeRunning {
    /// The person has already said this agent may act inside its workspace
    /// without being asked each time, and a command inside it is one of those
    /// acts.
    No,
    /// Every command is put to them first, and runs only on a yes. This is
    /// what the profile that asks every time means, and the reason it cannot
    /// simply be left refused: a client callback never reaches the person the
    /// way a tool call does, so the host has to ask on its own account.
    EveryTime,
}

#[derive(Clone)]
pub struct TerminalCallbacks {
    /// The terminals of this host: the same ones the person's terminal panel
    /// lists, so a command the agent runs is a command the person can watch.
    pub terminals: Arc<crate::workbench_shell::Terminals>,
    /// Whose environment to run in. Its workspace is also the boundary: a
    /// working directory outside it is refused before any process starts.
    pub profile: crate::PersonalAgentProfile,
    /// The profile's secrets, resolved into the environment the same way the
    /// agent process itself was started.
    pub secrets: BTreeMap<String, String>,
    /// Whether each command is put to the person first.
    pub ask: AskBeforeRunning,
}

impl fmt::Debug for TerminalCallbacks {
    /// The secrets are never formatted, and neither are their names: this is
    /// the same material the session options keep out of their own `Debug`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminalCallbacks")
            .field("profile_id", &self.profile.profile_id)
            .field("workspace", &self.profile.workspace)
            .field("ask", &self.ask)
            .finish_non_exhaustive()
    }
}

/// One official ACP v1 elicitation offered to the surface that owns this
/// connection. The request is deliberately ephemeral: form values and URL
/// targets are returned to the agent over ACP but never copied into the
/// transcript or durable routing ledger.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NativeElicitationRequest {
    pub sequence: u64,
    pub request: CreateElicitationRequest,
}

#[derive(Debug)]
struct PendingNativeElicitation {
    request: CreateElicitationRequest,
    response: oneshot::Sender<ElicitationAction>,
}

/// Permission behavior selected by the caller of a native session.
///
/// `DenyAll` is the safe, Cycle-independent default. `SwemSeam` exists for the
/// compatibility composition and must not be mistaken for authenticated MCP
/// provenance: ACP v1 currently supplies only the adapter-reported tool title.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionPermissionPolicy {
    #[default]
    DenyAll,
    SwemSeam {
        policy: SeamPermissionPolicy,
    },
    /// Forward each request to the caller-owned [`NativeSessionControl`].
    /// The exact option selected by that surface is returned to the agent.
    Surface,
}

impl SessionPermissionPolicy {
    fn decide(&self, request: &RequestPermissionRequest) -> PermissionDecision {
        match self {
            Self::SwemSeam { policy } => policy.decide(request),
            Self::Surface => PermissionDecision {
                tool_call_id: request.tool_call.tool_call_id.0.to_string(),
                title: request.tool_call.fields.title.clone(),
                kind: request.tool_call.fields.kind.as_ref().map(wire_name),
                allowed: false,
                reason: "surface permission policy requires a native session control".into(),
                selected_option: None,
                provenance: NativePermissionProvenance::UncorrelatedAgentReport,
                decision_source: NativePermissionDecisionSource::ProtocolGuard,
            },
            Self::DenyAll => {
                let selected_option = request
                    .options
                    .iter()
                    .find(|option| {
                        matches!(
                            option.kind,
                            PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways
                        )
                    })
                    .map(|option| option.option_id.0.to_string());
                PermissionDecision {
                    tool_call_id: request.tool_call.tool_call_id.0.to_string(),
                    title: request.tool_call.fields.title.clone(),
                    kind: request.tool_call.fields.kind.as_ref().map(wire_name),
                    allowed: false,
                    reason: "native session has no authority grant for this tool call".into(),
                    selected_option,
                    provenance: NativePermissionProvenance::UncorrelatedAgentReport,
                    decision_source: NativePermissionDecisionSource::DenyAllPolicy,
                }
            }
        }
    }
}

impl SeamPermissionPolicy {
    #[must_use]
    pub fn decide(&self, request: &RequestPermissionRequest) -> PermissionDecision {
        let fields = &request.tool_call.fields;
        let title = fields.title.clone();
        let kind = fields.kind.as_ref().map(wire_name);
        // ACP v1 exposes the adapter-reported title, not an authenticated MCP
        // server identity. Match the complete conventional title prefix only;
        // never search arbitrary raw input, where an attacker can plant the
        // string and obtain an unrelated permission.
        let reported_title = title.as_deref().unwrap_or_default().to_ascii_lowercase();
        let reported_own_tool = self.mcp_server_names.iter().find(|name| {
            let own_prefix = format!("mcp__{}__", name.to_ascii_lowercase());
            reported_title
                .strip_prefix(&own_prefix)
                .is_some_and(|tool| !tool.is_empty() && !tool.contains("__"))
        });
        let (allowed, reason) = if let Some(name) = reported_own_tool {
            (
                true,
                format!("agent-reported title matches the attached `{name}` MCP convention"),
            )
        } else if reported_title.starts_with("mcp__") {
            (false, "tool call names a foreign MCP server".into())
        } else if self.allow_read_only_tools
            && matches!(fields.kind, Some(ToolKind::Read | ToolKind::Search))
        {
            (true, "read-only workspace tool".into())
        } else if let (Some(workspace), Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move)) =
            (&self.authoring_workspace, &fields.kind)
        {
            let paths = touched_paths(request);
            if !paths.is_empty() && paths.iter().all(|path| inside(workspace, path)) {
                (true, "file edit inside the authoring workspace".into())
            } else {
                (
                    false,
                    "file edit outside the authoring workspace (or with no stated location)".into(),
                )
            }
        } else {
            (false, "effectful tool outside the SWEM seam".into())
        };
        let wanted = if allowed {
            [
                PermissionOptionKind::AllowOnce,
                PermissionOptionKind::AllowAlways,
            ]
        } else {
            [
                PermissionOptionKind::RejectOnce,
                PermissionOptionKind::RejectAlways,
            ]
        };
        let selected_option = wanted.iter().find_map(|wanted| {
            request
                .options
                .iter()
                .find(|option| option.kind == *wanted)
                .map(|option| option.option_id.0.to_string())
        });
        PermissionDecision {
            tool_call_id: request.tool_call.tool_call_id.0.to_string(),
            title,
            kind,
            allowed,
            reason,
            selected_option,
            provenance: NativePermissionProvenance::UncorrelatedAgentReport,
            decision_source: NativePermissionDecisionSource::LegacySeamPolicy,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentSessionOptions {
    pub timeout: Duration,
    pub grant_local_execution: bool,
    /// Grant the `channel.publish` capability to the injected MCP server.
    /// Capability only: each publish request still needs an authorization
    /// decision recorded by an authority outside the seam.
    pub grant_publish: bool,
    pub journal_dir: Option<PathBuf>,
    pub transcript_path: Option<PathBuf>,
    pub permission_policy: SeamPermissionPolicy,
    /// Inject the SWEM MCP server into the session. `false` (the default)
    /// is the generic session: the native agent alone in the workspace. A
    /// Cycle composition opts in explicitly - the generic agent session no
    /// longer carries the Cycle by default (gate C0 delete-first rule).
    pub inject_mcp: bool,
    /// The package directories the injected Cycle loads. A domain arrives as
    /// a package now, so a session whose Cycle is not pointed at them serves
    /// the kernel's tools and no domain's.
    pub plugins: Vec<PathBuf>,
}

impl AgentSessionOptions {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            grant_local_execution: false,
            grant_publish: false,
            journal_dir: None,
            transcript_path: None,
            permission_policy: SeamPermissionPolicy::default(),
            inject_mcp: false,
            plugins: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolCallSummary {
    pub tool_call_id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentSessionOutcome {
    pub protocol_version: String,
    pub session_id: String,
    pub agent_info: Option<Value>,
    pub stop_reason: String,
    pub reply_text: String,
    pub tool_calls: Vec<ToolCallSummary>,
    pub permission_decisions: Vec<PermissionDecision>,
    pub wall_clock_ms: u64,
    pub journal_dir: Option<PathBuf>,
    pub transcript_path: Option<PathBuf>,
}

/// Configuration for a Cycle-independent native ACP session.
#[derive(Clone)]
pub struct NativeSessionOptions {
    /// Maximum duration of one ACP request. Idle time between interactive
    /// turns is deliberately outside this deadline.
    pub operation_timeout: Duration,
    /// Optional deadline for the complete transport connection. Batch callers
    /// retain the historical bounded behavior; a Workbench-style interactive
    /// connection uses `None` and ends only through disconnect, close or
    /// failure.
    pub connection_timeout: Option<Duration>,
    pub transcript_path: Option<PathBuf>,
    pub mcp_servers: Vec<McpServer>,
    pub permission_policy: SessionPermissionPolicy,
    pub start: NativeSessionStart,
    /// Advertise exact ACP support for boolean session config options.
    /// Select options require no capability flag and remain available either
    /// way. This is transport capability negotiation, not a SWEM model flag.
    pub boolean_config_options: bool,
    /// Exact ACP v1 elicitation modes this client surface can render. `None`
    /// advertises no support; an empty capability object also advertises no
    /// modes, exactly as ACP specifies.
    pub elicitation_capabilities: Option<ElicitationCapabilities>,
    /// What the surface behind this connection can do with files, and where.
    /// `None` advertises no `fs/*` at all and the two callbacks are refused,
    /// which is right for a surface that owns no filesystem - a browser tab
    /// is one. An editor is the other case and the reason this exists.
    pub file_callbacks: Option<FileCallbacks>,
    /// Whose environment this connection may run commands in. `None`
    /// advertises no `terminal/*` at all and all five are refused, which is
    /// right wherever nobody has agreed that this agent may run commands on
    /// its own.
    pub terminal_callbacks: Option<TerminalCallbacks>,
    /// Optional surface-owned control lane. It is ephemeral and carries no
    /// conversation state; the native agent remains the session authority.
    pub control: Option<NativeSessionControl>,
    /// Optional lease selected by the environment fabric. The current native
    /// transport accepts only the truthful direct-process lease; isolated
    /// backends must provide their own enforced transport before use.
    pub environment_lease: Option<EnvironmentLease>,
    /// Connection-local transport for a non-direct prepared environment. It
    /// must match the exact lease identity; commands and backend connection
    /// details do not become durable session state.
    pub environment_transport: Option<EnvironmentTransport>,
    /// Explicit values added to the child process. With an environment lease,
    /// these are added after the direct transport clears ambient values to its
    /// platform bootstrap allowlist. Without a lease the compatibility launcher
    /// still inherits the host. Only names are written to the transcript.
    pub process_environment_overrides: BTreeMap<String, String>,
    /// The authentication method the person chose, by the id the agent
    /// advertised. `None` takes the first advertised method, which is right
    /// for an agent that offers one method needing no input and was the only
    /// behaviour before a person could choose.
    pub auth_method_id: Option<String>,
}

impl fmt::Debug for NativeSessionOptions {
    /// Environment override values may carry credential material; only their
    /// names are ever formatted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSessionOptions")
            .field("operation_timeout", &self.operation_timeout)
            .field("connection_timeout", &self.connection_timeout)
            .field("transcript_path", &self.transcript_path)
            .field("mcp_server_count", &self.mcp_servers.len())
            .field("permission_policy", &self.permission_policy)
            .field("start", &self.start)
            .field("boolean_config_options", &self.boolean_config_options)
            .field("elicitation_capabilities", &self.elicitation_capabilities)
            .field("file_callbacks", &self.file_callbacks)
            .field("terminal_callbacks", &self.terminal_callbacks)
            .field("control", &self.control.is_some())
            .field("environment_lease", &self.environment_lease)
            .field("environment_transport", &self.environment_transport)
            .field("auth_method_id", &self.auth_method_id)
            .field(
                "process_environment_override_names",
                &self
                    .process_environment_overrides
                    .keys()
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

impl NativeSessionOptions {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self {
            operation_timeout: timeout,
            connection_timeout: Some(timeout),
            transcript_path: None,
            mcp_servers: Vec::new(),
            permission_policy: SessionPermissionPolicy::DenyAll,
            start: NativeSessionStart::New,
            boolean_config_options: true,
            elicitation_capabilities: None,
            file_callbacks: None,
            terminal_callbacks: None,
            control: None,
            auth_method_id: None,
            environment_lease: None,
            environment_transport: None,
            process_environment_overrides: BTreeMap::new(),
        }
    }

    /// Configuration for a long-lived interactive ACP connection.
    ///
    /// Individual initialize/setup/config/prompt/close operations remain
    /// bounded by `operation_timeout`; idle connection lifetime does not.
    #[must_use]
    pub fn interactive(operation_timeout: Duration) -> Self {
        Self {
            connection_timeout: None,
            ..Self::new(operation_timeout)
        }
    }
}

/// Observable phase of one ephemeral native-session connection.
///
/// This is control-plane state for a UI/channel caller, not a durable session
/// record. In particular, it cannot be used to reconstruct or replay a native
/// agent conversation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum NativeSessionPhase {
    Connecting,
    Idle {
        session_id: String,
    },
    Prompting {
        session_id: String,
        turn_index: usize,
    },
    Finished,
}

/// Identity of the active turn for which a cancellation signal was accepted.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NativeActiveTurn {
    pub session_id: String,
    pub turn_index: usize,
}

#[derive(Clone, Debug)]
struct NativeCancelSignal {
    sequence: u64,
    turn: NativeActiveTurn,
}

enum NativeSessionCommand {
    SetConfigOption {
        config_id: String,
        value: SessionConfigOptionValue,
        response: oneshot::Sender<Result<Vec<SessionConfigOption>, SupplyError>>,
    },
    SetLegacyMode {
        mode_id: String,
        response: oneshot::Sender<Result<SessionModeState, SupplyError>>,
    },
    Configuration {
        response: oneshot::Sender<SessionConfiguration>,
    },
}

/// What the agent offers to configure on this session, as it stands now:
/// its `configOptions` (a model, a mode, a thought level, whatever it
/// declared) and, for an agent that predates them, its modes.
#[derive(Clone, Debug, Default, Serialize)]
pub struct SessionConfiguration {
    pub config_options: Option<Vec<SessionConfigOption>>,
    pub legacy_modes: Option<SessionModeState>,
}

enum NativeSessionDriverCommand {
    Prompt {
        content: Vec<ContentBlock>,
        response: oneshot::Sender<Result<NativeTurnOutcome, SupplyError>>,
    },
    Disconnect {
        response: oneshot::Sender<Result<(), SupplyError>>,
    },
    Close {
        response: oneshot::Sender<Result<(), SupplyError>>,
    },
}

struct PendingNativePrompt {
    content: Vec<ContentBlock>,
    descriptors: Vec<Value>,
    response: Option<oneshot::Sender<Result<NativeTurnOutcome, SupplyError>>>,
}

#[derive(Debug)]
struct NativeSessionControlInner {
    phase: Mutex<NativeSessionPhase>,
    phase_tx: watch::Sender<NativeSessionPhase>,
    cancel_tx: watch::Sender<Option<NativeCancelSignal>>,
    next_sequence: Mutex<u64>,
    command_tx: mpsc::UnboundedSender<NativeSessionCommand>,
    command_rx: Mutex<Option<mpsc::UnboundedReceiver<NativeSessionCommand>>>,
    driver_tx: mpsc::Sender<NativeSessionDriverCommand>,
    driver_rx: Mutex<Option<mpsc::Receiver<NativeSessionDriverCommand>>>,
    driver_attached: AtomicBool,
    permission_tx: mpsc::UnboundedSender<NativePermissionRequest>,
    permission_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<NativePermissionRequest>>,
    pending_permissions: Mutex<BTreeMap<u64, PendingNativePermission>>,
    elicitation_tx: mpsc::UnboundedSender<NativeElicitationRequest>,
    elicitation_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<NativeElicitationRequest>>,
    pending_elicitations: Mutex<BTreeMap<u64, PendingNativeElicitation>>,
    open_url_elicitations: Mutex<BTreeSet<String>>,
    file_tx: mpsc::UnboundedSender<NativeFileRequest>,
    file_rx: tokio::sync::Mutex<mpsc::UnboundedReceiver<NativeFileRequest>>,
    pending_files: Mutex<BTreeMap<u64, PendingNativeFile>>,
    surface_event_tx: mpsc::UnboundedSender<NativeSessionEvent>,
    surface_event_rx: Mutex<Option<mpsc::UnboundedReceiver<NativeSessionEvent>>>,
    surface_events_enabled: AtomicBool,
    /// What the agent said it can take in a turn, once it has said it. A
    /// surface reads this to offer content the agent can actually use rather
    /// than to find out by having a turn refused.
    prompt_capabilities: Mutex<Option<PromptCapabilities>>,
}

/// Surface-neutral handle for signalling an in-flight ACP prompt turn.
///
/// Clones may be held by a Workbench, channel adapter or another host task.
/// Cancellation is accepted only while a prompt is active, so a late user
/// action can never cancel a later turn by accident.
#[derive(Clone, Debug)]
pub struct NativeSessionControl {
    inner: Arc<NativeSessionControlInner>,
}

impl Default for NativeSessionControl {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeSessionControl {
    #[must_use]
    pub fn new() -> Self {
        let initial = NativeSessionPhase::Connecting;
        let (phase_tx, _) = watch::channel(initial.clone());
        let (cancel_tx, _) = watch::channel(None);
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        // User turns are intentionally bounded. A slow or permission-blocked
        // native agent must exert backpressure on every attached surface.
        let (driver_tx, driver_rx) = mpsc::channel(8);
        let (permission_tx, permission_rx) = mpsc::unbounded_channel();
        let (elicitation_tx, elicitation_rx) = mpsc::unbounded_channel();
        let (file_tx, file_rx) = mpsc::unbounded_channel();
        let (surface_event_tx, surface_event_rx) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(NativeSessionControlInner {
                phase: Mutex::new(initial),
                phase_tx,
                cancel_tx,
                next_sequence: Mutex::new(0),
                command_tx,
                command_rx: Mutex::new(Some(command_rx)),
                driver_tx,
                driver_rx: Mutex::new(Some(driver_rx)),
                driver_attached: AtomicBool::new(false),
                permission_tx,
                permission_rx: tokio::sync::Mutex::new(permission_rx),
                pending_permissions: Mutex::new(BTreeMap::new()),
                elicitation_tx,
                elicitation_rx: tokio::sync::Mutex::new(elicitation_rx),
                pending_elicitations: Mutex::new(BTreeMap::new()),
                open_url_elicitations: Mutex::new(BTreeSet::new()),
                file_tx,
                file_rx: tokio::sync::Mutex::new(file_rx),
                pending_files: Mutex::new(BTreeMap::new()),
                surface_event_tx,
                surface_event_rx: Mutex::new(Some(surface_event_rx)),
                surface_events_enabled: AtomicBool::new(false),
                prompt_capabilities: Mutex::new(None),
            }),
        }
    }

    /// What this connection's agent advertised it can take in a turn, or
    /// `None` before the handshake has answered.
    #[must_use]
    pub fn prompt_capabilities(&self) -> Option<PromptCapabilities> {
        self.inner
            .prompt_capabilities
            .lock()
            .ok()
            .and_then(|capabilities| capabilities.clone())
    }

    fn remember_prompt_capabilities(&self, capabilities: &PromptCapabilities) {
        if let Ok(mut held) = self.inner.prompt_capabilities.lock() {
            *held = Some(capabilities.clone());
        }
    }

    /// Take the ordered surface projection for this one ACP connection.
    ///
    /// The lane has one consumer by design. Fan-out and durable per-surface
    /// cursors belong to [`crate::RoutingLedger`], not to the native session.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when another projector already owns the lane.
    pub fn take_surface_events(
        &self,
    ) -> Result<mpsc::UnboundedReceiver<NativeSessionEvent>, SupplyError> {
        let receiver = self
            .inner
            .surface_event_rx
            .lock()
            .map_err(|_| SupplyError::Poisoned)?
            .take()
            .ok_or_else(|| {
                SupplyError::Protocol(
                    "native session surface events cannot have two direct consumers".into(),
                )
            })?;
        self.inner
            .surface_events_enabled
            .store(true, Ordering::Release);
        Ok(receiver)
    }

    fn publish_surface_event(
        &self,
        kind: impl Into<String>,
        source: SurfaceEventSource,
        payload: Value,
    ) {
        if !self.inner.surface_events_enabled.load(Ordering::Acquire) {
            return;
        }
        let _ = self
            .inner
            .surface_event_tx
            .send(NativeSessionEvent::new(kind, source, payload));
    }

    fn publish_surface_event_with_output(
        &self,
        kind: impl Into<String>,
        source: SurfaceEventSource,
        payload: Value,
        output_projection: Vec<NativeOutputProjection>,
    ) {
        if !self.inner.surface_events_enabled.load(Ordering::Acquire) {
            return;
        }
        let event = NativeSessionEvent::new(kind, source, payload)
            .with_output_projection(output_projection);
        let _ = self.inner.surface_event_tx.send(event);
    }

    #[must_use]
    pub fn phase(&self) -> NativeSessionPhase {
        self.inner
            .phase
            .lock()
            .map_or(NativeSessionPhase::Finished, |phase| phase.clone())
    }

    /// Wait until ACP setup has produced a native session identity.
    ///
    /// This observes ephemeral connection state only. A caller that also owns
    /// the runner task should use its result for the exact startup failure.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the connection finishes before becoming
    /// usable.
    pub async fn wait_until_ready(&self) -> Result<String, SupplyError> {
        let mut receiver = self.inner.phase_tx.subscribe();
        loop {
            match receiver.borrow().clone() {
                NativeSessionPhase::Idle { session_id }
                | NativeSessionPhase::Prompting { session_id, .. } => return Ok(session_id),
                NativeSessionPhase::Finished => {
                    return Err(SupplyError::Protocol(
                        "native session finished before becoming ready".into(),
                    ));
                }
                NativeSessionPhase::Connecting => {}
            }
            receiver.changed().await.map_err(|_| {
                SupplyError::Protocol("native session readiness lane is closed".into())
            })?;
        }
    }

    /// Wait until a prompt is active, or return `None` if the connection ends.
    pub async fn wait_for_active_turn(&self) -> Option<NativeActiveTurn> {
        let mut receiver = self.inner.phase_tx.subscribe();
        loop {
            match receiver.borrow().clone() {
                NativeSessionPhase::Prompting {
                    session_id,
                    turn_index,
                } => {
                    return Some(NativeActiveTurn {
                        session_id,
                        turn_index,
                    });
                }
                NativeSessionPhase::Finished => return None,
                NativeSessionPhase::Connecting | NativeSessionPhase::Idle { .. } => {}
            }
            if receiver.changed().await.is_err() {
                return None;
            }
        }
    }

    /// Request cancellation of the currently active turn.
    ///
    /// `None` means there is no active turn. The signal is deliberately not
    /// queued, preventing a stale click or channel event from cancelling the
    /// next prompt.
    #[must_use]
    pub fn cancel_active_turn(&self) -> Option<NativeActiveTurn> {
        let phase = self.inner.phase.lock().ok()?;
        let NativeSessionPhase::Prompting {
            session_id,
            turn_index,
        } = &*phase
        else {
            return None;
        };
        let turn = NativeActiveTurn {
            session_id: session_id.clone(),
            turn_index: *turn_index,
        };
        let mut sequence = self.inner.next_sequence.lock().ok()?;
        *sequence = sequence.saturating_add(1);
        self.inner.cancel_tx.send_replace(Some(NativeCancelSignal {
            sequence: *sequence,
            turn: turn.clone(),
        }));
        self.publish_surface_event(
            "host/cancel_requested",
            SurfaceEventSource::Host,
            serde_json::to_value(&turn).unwrap_or(Value::Null),
        );
        self.cancel_pending_permissions();
        self.cancel_pending_elicitations();
        self.cancel_pending_files();
        Some(turn)
    }

    /// Receive the next ACP permission request offered by an active session.
    ///
    /// The returned request contains the agent's exact option ids. A surface
    /// must answer it with [`Self::select_permission`]. Cancelling the whole
    /// active turn resolves every pending request with ACP `cancelled`; a
    /// permission-only rejection must select an agent-provided reject option.
    /// `None` means the session finished or the request lane closed.
    pub async fn next_permission_request(&self) -> Option<NativePermissionRequest> {
        let mut permissions = self.inner.permission_rx.lock().await;
        let mut phase = self.inner.phase_tx.subscribe();
        loop {
            if matches!(*phase.borrow(), NativeSessionPhase::Finished) {
                return None;
            }
            tokio::select! {
                request = permissions.recv() => {
                    let request = request?;
                    let still_pending = self
                        .inner
                        .pending_permissions
                        .lock()
                        .is_ok_and(|pending| pending.contains_key(&request.sequence));
                    if still_pending {
                        return Some(request);
                    }
                }
                changed = phase.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                }
            }
        }
    }

    /// Read an outstanding request after a surface-local cursor, including one
    /// already observed by a disconnected surface. This is live state, not
    /// conversation replay. A new attachment starts with zero.
    pub async fn permission_request_after(&self, after: u64) -> Option<NativePermissionRequest> {
        let pending = self
            .inner
            .pending_permissions
            .lock()
            .ok()?
            .values()
            .find(|pending| pending.request.sequence > after)
            .map(|pending| pending.request.clone());
        if pending.is_some() {
            return pending;
        }
        loop {
            let request = self.next_permission_request().await?;
            if request.sequence > after {
                return Some(request);
            }
        }
    }

    /// Select one exact option advertised by the agent for this request.
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a stale sequence or an option id that was
    /// not present in that request. Invalid selections do not consume the
    /// pending request, allowing a surface to correct its response.
    pub fn select_permission(
        &self,
        sequence: u64,
        option_id: impl AsRef<str>,
    ) -> Result<(), SupplyError> {
        let option_id = option_id.as_ref();
        let mut pending = self
            .inner
            .pending_permissions
            .lock()
            .map_err(|_| SupplyError::Poisoned)?;
        let request = pending.get(&sequence).ok_or_else(|| {
            SupplyError::Protocol(format!(
                "permission request {sequence} is no longer pending"
            ))
        })?;
        if !request.option_ids.contains(option_id) {
            return Err(SupplyError::Protocol(format!(
                "permission option `{option_id}` was not offered for request {sequence}"
            )));
        }
        let request = pending.remove(&sequence).ok_or(SupplyError::Poisoned)?;
        request
            .response
            .send(Some(option_id.to_owned()))
            .map_err(|_| {
                SupplyError::Protocol(format!(
                    "permission request {sequence} ended before the response"
                ))
            })
    }

    /// Put one command to the surface and say whether it may run.
    ///
    /// It goes out on the same lane an agent's own permission request goes
    /// out on, and for a plain reason: that lane is already what a person
    /// answers - the page draws it, the editor door forwards it over ACP's
    /// own `session/request_permission`, and the record keeps the decision.
    /// A second way to ask would be a second place to look.
    ///
    /// What it is not is an agent's report: the words are the host's, taken
    /// from the request it is about to carry out, and the provenance says so.
    ///
    /// The lane is recoverable by cursor, so a question raised while no tab
    /// is open is not lost: it waits there until somebody answers it or the
    /// turn ends. No answer means no - the host never answers for the person,
    /// and a turn nobody comes back to ends on its own deadline.
    pub(crate) async fn ask_to_run(&self, session_id: &str, command_line: &str) -> bool {
        const ALLOW: &str = "run-once";
        const REJECT: &str = "do-not-run";
        let mut fields = ToolCallUpdateFields::default();
        fields.title = Some(format!("Run {command_line}"));
        fields.kind = Some(ToolKind::Execute);
        let selected = self
            .offer_permission(
                session_id.to_owned(),
                ToolCallUpdate::new(ToolCallId::new("terminal/create"), fields),
                vec![
                    PermissionOption::new(
                        PermissionOptionId::new(ALLOW),
                        "Run it",
                        PermissionOptionKind::AllowOnce,
                    ),
                    PermissionOption::new(
                        PermissionOptionId::new(REJECT),
                        "Not this time",
                        PermissionOptionKind::RejectOnce,
                    ),
                ],
                NativePermissionProvenance::HostCallback,
            )
            .await;
        selected.as_deref() == Some(ALLOW)
    }

    async fn offer_permission(
        &self,
        session_id: String,
        tool_call: ToolCallUpdate,
        options: Vec<PermissionOption>,
        provenance: NativePermissionProvenance,
    ) -> Option<String> {
        let response = {
            // Shared phase locking makes publishing a request atomic with
            // turn cancellation: either it is inserted first and cancellation
            // drains it, or it observes that the turn is no longer active.
            let phase = self.inner.phase.lock().ok()?;
            if !matches!(
                &*phase,
                NativeSessionPhase::Prompting { session_id: active, .. } if active == &session_id
            ) {
                return None;
            }
            let mut sequence = self.inner.next_sequence.lock().ok()?;
            *sequence = sequence.saturating_add(1);
            let request_sequence = *sequence;
            let option_ids = options
                .iter()
                .map(|option| option.option_id.0.to_string())
                .collect();
            let (response, received) = oneshot::channel();
            let request = NativePermissionRequest {
                sequence: request_sequence,
                session_id,
                tool_call,
                options,
                provenance,
            };
            self.inner.pending_permissions.lock().ok()?.insert(
                request_sequence,
                PendingNativePermission {
                    option_ids,
                    request: request.clone(),
                    response,
                },
            );
            if self.inner.permission_tx.send(request.clone()).is_err() {
                self.inner
                    .pending_permissions
                    .lock()
                    .ok()?
                    .remove(&request_sequence);
                return None;
            }
            self.publish_surface_event(
                "acp/session_request_permission",
                SurfaceEventSource::NativeLive,
                redacted_wire_value(&request),
            );
            received
        };
        response.await.unwrap_or(None)
    }

    fn cancel_pending_permissions(&self) {
        if let Ok(mut pending) = self.inner.pending_permissions.lock() {
            for (_, request) in std::mem::take(&mut *pending) {
                let _ = request.response.send(None);
            }
        }
    }

    /// Receive the next file call offered by an active session.
    ///
    /// The path in it has already been checked against the connection's
    /// boundary, so a surface reading this lane is being asked for something
    /// the host was willing to ask for. A surface must answer with
    /// [`Self::answer_file_request`]; cancelling the turn resolves every
    /// pending call as unanswered. `None` means the session finished or the
    /// lane closed.
    pub async fn next_file_request(&self) -> Option<NativeFileRequest> {
        let mut files = self.inner.file_rx.lock().await;
        let mut phase = self.inner.phase_tx.subscribe();
        loop {
            if matches!(*phase.borrow(), NativeSessionPhase::Finished) {
                return None;
            }
            tokio::select! {
                request = files.recv() => {
                    let request = request?;
                    let still_pending = self
                        .inner
                        .pending_files
                        .lock()
                        .is_ok_and(|pending| pending.contains_key(&request.sequence));
                    if still_pending {
                        return Some(request);
                    }
                }
                changed = phase.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                }
            }
        }
    }

    /// Read an outstanding file call after a surface-local cursor, including
    /// one a disconnected surface already saw. This is live state, not
    /// replay: a reconnecting surface starts at zero and is handed whatever
    /// the agent is still waiting on.
    pub async fn file_request_after(&self, after: u64) -> Option<NativeFileRequest> {
        let pending = self
            .inner
            .pending_files
            .lock()
            .ok()?
            .values()
            .find(|pending| pending.request.sequence > after)
            .map(|pending| pending.request.clone());
        if pending.is_some() {
            return pending;
        }
        loop {
            let request = self.next_file_request().await?;
            if request.sequence > after {
                return Some(request);
            }
        }
    }

    /// Tell the session what the surface did with one file call.
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a sequence that is no longer pending, or
    /// for an answer that does not fit the method that was asked - a read
    /// answered with `Written` is a surface answering a question nobody put,
    /// and the agent must not be told the file says that.
    pub fn answer_file_request(
        &self,
        sequence: u64,
        answer: NativeFileAnswer,
    ) -> Result<(), SupplyError> {
        let mut pending = self
            .inner
            .pending_files
            .lock()
            .map_err(|_| SupplyError::Poisoned)?;
        let request = pending.get(&sequence).ok_or_else(|| {
            SupplyError::Protocol(format!("file request {sequence} is no longer pending"))
        })?;
        let fits = matches!(
            (&answer, request.request.call.method.as_str()),
            (NativeFileAnswer::Text(_), "fs/read_text_file")
                | (NativeFileAnswer::Written, "fs/write_text_file")
                | (NativeFileAnswer::Refused(_), _)
        );
        if !fits {
            return Err(SupplyError::Protocol(format!(
                "that answer does not fit `{}`, which file request {sequence} asked for",
                request.request.call.method
            )));
        }
        let request = pending.remove(&sequence).ok_or(SupplyError::Poisoned)?;
        request.response.send(Some(answer)).map_err(|_| {
            SupplyError::Protocol(format!("file request {sequence} ended before the answer"))
        })
    }

    async fn offer_file_request(&self, call: NativeFileCall) -> Option<NativeFileAnswer> {
        let response = {
            // The same atomicity the permission lane has: either the call is
            // inserted while the turn is still active and cancellation drains
            // it, or it sees that the turn is over and never goes out.
            let phase = self.inner.phase.lock().ok()?;
            if !matches!(
                &*phase,
                NativeSessionPhase::Prompting { session_id: active, .. }
                    if active == &call.session_id
            ) {
                return None;
            }
            let mut sequence = self.inner.next_sequence.lock().ok()?;
            *sequence = sequence.saturating_add(1);
            let request_sequence = *sequence;
            let (response, received) = oneshot::channel();
            let descriptor = call.descriptor();
            let request = NativeFileRequest {
                sequence: request_sequence,
                call,
            };
            self.inner.pending_files.lock().ok()?.insert(
                request_sequence,
                PendingNativeFile {
                    request: request.clone(),
                    response,
                },
            );
            if self.inner.file_tx.send(request).is_err() {
                self.inner
                    .pending_files
                    .lock()
                    .ok()?
                    .remove(&request_sequence);
                return None;
            }
            self.publish_surface_event(
                "acp/fs_request",
                SurfaceEventSource::NativeLive,
                descriptor,
            );
            received
        };
        response.await.unwrap_or(None)
    }

    fn cancel_pending_files(&self) {
        if let Ok(mut pending) = self.inner.pending_files.lock() {
            for (_, request) in std::mem::take(&mut *pending) {
                let _ = request.response.send(None);
            }
        }
    }

    /// Recover an unanswered elicitation for a new surface without journaling
    /// its form schema or URL. The cursor is local to that surface attachment.
    pub async fn elicitation_request_after(&self, after: u64) -> Option<NativeElicitationRequest> {
        let pending = self
            .inner
            .pending_elicitations
            .lock()
            .ok()?
            .iter()
            .find(|(sequence, _)| **sequence > after)
            .map(|(sequence, pending)| NativeElicitationRequest {
                sequence: *sequence,
                request: pending.request.clone(),
            });
        if pending.is_some() {
            return pending;
        }
        loop {
            let request = self.next_elicitation_request().await?;
            if request.sequence > after {
                return Some(request);
            }
        }
    }

    /// Receive the next official ACP elicitation for this connection.
    ///
    /// The request remains bound to this control and must be answered with
    /// [`Self::answer_elicitation`]. Unknown future modes are not offered as a
    /// known UI; the protocol handler returns `cancel` for them.
    pub async fn next_elicitation_request(&self) -> Option<NativeElicitationRequest> {
        let mut elicitations = self.inner.elicitation_rx.lock().await;
        let mut phase = self.inner.phase_tx.subscribe();
        loop {
            if matches!(*phase.borrow(), NativeSessionPhase::Finished) {
                return None;
            }
            tokio::select! {
                request = elicitations.recv() => {
                    let request = request?;
                    let still_pending = self
                        .inner
                        .pending_elicitations
                        .lock()
                        .is_ok_and(|pending| pending.contains_key(&request.sequence));
                    if still_pending {
                        return Some(request);
                    }
                }
                changed = phase.changed() => {
                    if changed.is_err() {
                        return None;
                    }
                }
            }
        }
    }

    /// Answer one connection-bound ACP elicitation with an official action.
    /// Raw accepted content crosses only the pending oneshot into the ACP
    /// responder; it is never published as a surface event.
    ///
    /// # Errors
    ///
    /// Returns a protocol error for a stale sequence, an extension action, or
    /// content attached to an accepted URL interaction.
    pub fn answer_elicitation(
        &self,
        sequence: u64,
        action: ElicitationAction,
    ) -> Result<(), SupplyError> {
        let mut pending = self
            .inner
            .pending_elicitations
            .lock()
            .map_err(|_| SupplyError::Poisoned)?;
        let request = pending.get(&sequence).ok_or_else(|| {
            SupplyError::Protocol(format!(
                "elicitation request {sequence} is no longer pending"
            ))
        })?;
        match (&request.request.mode, &action) {
            (ElicitationMode::Other(_), _) | (_, ElicitationAction::Other(_)) => {
                return Err(SupplyError::Protocol(
                    "unknown ACP elicitation modes and actions cannot be rendered as known UI"
                        .into(),
                ));
            }
            (
                ElicitationMode::Url(_),
                ElicitationAction::Accept(ElicitationAcceptAction {
                    content: Some(_), ..
                }),
            ) => {
                return Err(SupplyError::Protocol(
                    "ACP URL elicitation accept must not return form content".into(),
                ));
            }
            _ => {}
        }
        validate_elicitation_action(&request.request, &action)?;
        let request = pending.remove(&sequence).ok_or(SupplyError::Poisoned)?;
        request.response.send(action).map_err(|_| {
            SupplyError::Protocol(format!(
                "elicitation request {sequence} ended before the response"
            ))
        })
    }

    async fn offer_elicitation(&self, request: CreateElicitationRequest) -> ElicitationAction {
        if matches!(request.mode, ElicitationMode::Other(_)) {
            return ElicitationAction::Cancel;
        }
        let response = {
            let Ok(phase) = self.inner.phase.lock() else {
                return ElicitationAction::Cancel;
            };
            if matches!(*phase, NativeSessionPhase::Finished) {
                return ElicitationAction::Cancel;
            }
            if let ElicitationScope::Session(scope) = request.scope()
                && !matches!(
                    &*phase,
                    NativeSessionPhase::Idle { session_id }
                        | NativeSessionPhase::Prompting { session_id, .. }
                        if session_id == scope.session_id.0.as_ref()
                )
            {
                return ElicitationAction::Cancel;
            }
            let url_id = match &request.mode {
                ElicitationMode::Url(url) => Some(url.elicitation_id.0.to_string()),
                _ => None,
            };
            if url_id.as_ref().is_some_and(|id| {
                self.inner
                    .open_url_elicitations
                    .lock()
                    .map_or(true, |open| open.contains(id))
            }) {
                // URL elicitation IDs must stay unique while outstanding on
                // this exact Agent-Client connection.
                return ElicitationAction::Cancel;
            }
            let Ok(mut sequence) = self.inner.next_sequence.lock() else {
                return ElicitationAction::Cancel;
            };
            *sequence = sequence.saturating_add(1);
            let request_sequence = *sequence;
            let (response, received) = oneshot::channel();
            if self
                .inner
                .pending_elicitations
                .lock()
                .map_or(true, |mut pending| {
                    if url_id.as_ref().is_some_and(|id| {
                        pending.values().any(|pending| {
                            matches!(
                                &pending.request.mode,
                                ElicitationMode::Url(url) if url.elicitation_id.0.as_ref() == id
                            )
                        })
                    }) {
                        return true;
                    }
                    pending.insert(
                        request_sequence,
                        PendingNativeElicitation {
                            request: request.clone(),
                            response,
                        },
                    );
                    false
                })
            {
                return ElicitationAction::Cancel;
            }
            let offered = NativeElicitationRequest {
                sequence: request_sequence,
                request,
            };
            if self.inner.elicitation_tx.send(offered.clone()).is_err() {
                if let Ok(mut pending) = self.inner.pending_elicitations.lock() {
                    pending.remove(&request_sequence);
                }
                return ElicitationAction::Cancel;
            }
            self.publish_surface_event(
                "acp/elicitation_create",
                SurfaceEventSource::NativeLive,
                redacted_elicitation_descriptor(&offered),
            );
            (received, url_id)
        };
        let action = response.0.await.unwrap_or(ElicitationAction::Cancel);
        if matches!(action, ElicitationAction::Accept(_))
            && let Some(url_id) = response.1
            && let Ok(mut open) = self.inner.open_url_elicitations.lock()
        {
            open.insert(url_id);
        }
        action
    }

    fn complete_url_elicitation(
        &self,
        notification: &CompleteElicitationNotification,
    ) -> Option<Value> {
        let elicitation_id = notification.elicitation_id.0.as_ref();
        let removed = self
            .inner
            .open_url_elicitations
            .lock()
            .ok()
            .is_some_and(|mut open| open.remove(elicitation_id));
        if !removed {
            return None;
        }
        let descriptor = serde_json::json!({
            "elicitation_id_sha256": digest_bytes(elicitation_id.as_bytes()),
        });
        self.publish_surface_event(
            "acp/elicitation_complete",
            SurfaceEventSource::NativeLive,
            descriptor.clone(),
        );
        Some(descriptor)
    }

    fn cancel_pending_elicitations(&self) {
        if let Ok(mut pending) = self.inner.pending_elicitations.lock() {
            for (_, request) in std::mem::take(&mut *pending) {
                let _ = request.response.send(ElicitationAction::Cancel);
            }
        }
    }

    fn clear_open_url_elicitations(&self) {
        if let Ok(mut open) = self.inner.open_url_elicitations.lock() {
            open.clear();
        }
    }

    /// Set one exact ACP session configuration option.
    ///
    /// The id and value are not normalized into SWEM-specific model or mode
    /// enums. The returned vector is the complete ordered state supplied by
    /// the agent, including any dependent changes.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when the session has finished, the command
    /// lane is closed, the selection does not match the negotiated state, or
    /// the agent rejects or fails to acknowledge the standard ACP request.
    pub async fn set_config_option(
        &self,
        config_id: impl Into<String>,
        value: SessionConfigOptionValue,
    ) -> Result<Vec<SessionConfigOption>, SupplyError> {
        if matches!(self.phase(), NativeSessionPhase::Finished) {
            return Err(SupplyError::Protocol(
                "native session configuration is no longer available".into(),
            ));
        }
        let (response, received) = oneshot::channel();
        self.inner
            .command_tx
            .send(NativeSessionCommand::SetConfigOption {
                config_id: config_id.into(),
                value,
                response,
            })
            .map_err(|_| {
                SupplyError::Protocol("native session configuration lane is closed".into())
            })?;
        received.await.map_err(|_| {
            SupplyError::Protocol(
                "native session ended before configuration was acknowledged".into(),
            )
        })?
    }

    /// What the agent offers to configure on this session right now. A
    /// finished session, or one whose lane has closed, answers nothing at
    /// all rather than a stale copy.
    pub async fn configuration(&self) -> SessionConfiguration {
        if matches!(self.phase(), NativeSessionPhase::Finished) {
            return SessionConfiguration::default();
        }
        let (response, received) = oneshot::channel();
        if self
            .inner
            .command_tx
            .send(NativeSessionCommand::Configuration { response })
            .is_err()
        {
            return SessionConfiguration::default();
        }
        received.await.unwrap_or_default()
    }

    /// Use the transitional ACP `session/set_mode` method.
    ///
    /// This is accepted only when the agent did not provide preferred
    /// `configOptions`; it is never used to shadow a category=`mode` option.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when preferred `configOptions` are present,
    /// the session has finished, the requested legacy mode is unavailable, or
    /// the agent rejects or fails to acknowledge the standard ACP request.
    pub async fn set_legacy_mode(
        &self,
        mode_id: impl Into<String>,
    ) -> Result<SessionModeState, SupplyError> {
        if matches!(self.phase(), NativeSessionPhase::Finished) {
            return Err(SupplyError::Protocol(
                "native session mode control is no longer available".into(),
            ));
        }
        let (response, received) = oneshot::channel();
        self.inner
            .command_tx
            .send(NativeSessionCommand::SetLegacyMode {
                mode_id: mode_id.into(),
                response,
            })
            .map_err(|_| SupplyError::Protocol("native session control lane is closed".into()))?;
        received.await.map_err(|_| {
            SupplyError::Protocol("native session ended before mode was acknowledged".into())
        })?
    }

    /// Submit the next exact ACP content turn to an interactive connection.
    ///
    /// The bounded lane provides backpressure; it stores neither prior turns
    /// nor a host-authored conversation. The result is only the terminal
    /// observation for this submission.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when this control is attached to a batch
    /// runner, the connection has ended, or the native turn fails.
    pub async fn submit_prompt(
        &self,
        content: Vec<ContentBlock>,
    ) -> Result<NativeTurnOutcome, SupplyError> {
        if !self.inner.driver_attached.load(Ordering::Acquire) {
            return Err(SupplyError::Protocol(
                "native session control is not attached to an interactive driver".into(),
            ));
        }
        if matches!(self.phase(), NativeSessionPhase::Finished) {
            return Err(SupplyError::Protocol(
                "native session prompt lane is no longer available".into(),
            ));
        }
        let (response, received) = oneshot::channel();
        self.inner
            .driver_tx
            .send(NativeSessionDriverCommand::Prompt { content, response })
            .await
            .map_err(|_| SupplyError::Protocol("native session prompt lane is closed".into()))?;
        received.await.map_err(|_| {
            SupplyError::Protocol("native session ended before the prompt completed".into())
        })?
    }

    /// Gracefully end this ACP connection while preserving the native session.
    ///
    /// The request is ordered after any already active or queued turn. Urgent
    /// interruption remains the separate [`Self::cancel_active_turn`] action.
    /// This is deliberately distinct from [`Self::close_session`].
    ///
    /// # Errors
    ///
    /// Returns a protocol error when no interactive driver owns this control.
    pub async fn disconnect(&self) -> Result<(), SupplyError> {
        self.send_driver_termination(false).await
    }

    /// Close the native ACP session and release agent-owned session resources.
    ///
    /// This uses stable `session/close` only when the agent advertised
    /// `sessionCapabilities.close`. Unsupported agents fail without silently
    /// degrading to a transport disconnect.
    ///
    /// # Errors
    ///
    /// Returns a protocol error when close is unsupported or the request fails.
    pub async fn close_session(&self) -> Result<(), SupplyError> {
        self.send_driver_termination(true).await
    }

    async fn send_driver_termination(&self, close: bool) -> Result<(), SupplyError> {
        if !self.inner.driver_attached.load(Ordering::Acquire) {
            return Err(SupplyError::Protocol(
                "native session control is not attached to an interactive driver".into(),
            ));
        }
        if matches!(self.phase(), NativeSessionPhase::Finished) {
            return Err(SupplyError::Protocol(
                "native session driver is no longer available".into(),
            ));
        }
        let (response, received) = oneshot::channel();
        let command = if close {
            NativeSessionDriverCommand::Close { response }
        } else {
            NativeSessionDriverCommand::Disconnect { response }
        };
        self.inner
            .driver_tx
            .send(command)
            .await
            .map_err(|_| SupplyError::Protocol("native session driver lane is closed".into()))?;
        received.await.map_err(|_| {
            SupplyError::Protocol("native session ended before termination was acknowledged".into())
        })?
    }

    fn set_phase(&self, phase: &NativeSessionPhase) {
        if let Ok(mut current) = self.inner.phase.lock() {
            current.clone_from(phase);
            self.inner.phase_tx.send_replace(phase.clone());
            self.publish_surface_event(
                "host/session_phase",
                SurfaceEventSource::Host,
                serde_json::to_value(phase).unwrap_or(Value::Null),
            );
            if matches!(*current, NativeSessionPhase::Finished) {
                self.cancel_pending_permissions();
                self.cancel_pending_elicitations();
                self.cancel_pending_files();
                self.clear_open_url_elicitations();
            }
        }
    }

    fn subscribe_cancellation(&self) -> watch::Receiver<Option<NativeCancelSignal>> {
        self.inner.cancel_tx.subscribe()
    }

    fn take_commands(&self) -> Result<mpsc::UnboundedReceiver<NativeSessionCommand>, SupplyError> {
        self.inner
            .command_rx
            .lock()
            .map_err(|_| SupplyError::Poisoned)?
            .take()
            .ok_or_else(|| {
                SupplyError::Protocol(
                    "native session control cannot be attached to two sessions".into(),
                )
            })
    }

    fn take_driver_commands(
        &self,
    ) -> Result<mpsc::Receiver<NativeSessionDriverCommand>, SupplyError> {
        let receiver = self
            .inner
            .driver_rx
            .lock()
            .map_err(|_| SupplyError::Poisoned)?
            .take()
            .ok_or_else(|| {
                SupplyError::Protocol(
                    "native session driver cannot be attached to two sessions".into(),
                )
            })?;
        self.inner.driver_attached.store(true, Ordering::Release);
        Ok(receiver)
    }

    /// Atomically close one active turn and observe whether a cancellation was
    /// accepted before the close. `cancel_active_turn` takes the same phase
    /// lock, so no accepted signal can fall between observation and `Idle`.
    fn finish_turn(&self, turn: &NativeActiveTurn, baseline_sequence: u64) -> bool {
        let Ok(mut phase) = self.inner.phase.lock() else {
            return false;
        };
        let requested = self
            .inner
            .cancel_tx
            .borrow()
            .as_ref()
            .is_some_and(|signal| signal.sequence > baseline_sequence && signal.turn == *turn);
        *phase = NativeSessionPhase::Idle {
            session_id: turn.session_id.clone(),
        };
        self.inner.phase_tx.send_replace(phase.clone());
        self.publish_surface_event(
            "host/session_phase",
            SurfaceEventSource::Host,
            serde_json::to_value(&*phase).unwrap_or(Value::Null),
        );
        self.cancel_pending_permissions();
        self.cancel_pending_elicitations();
        self.cancel_pending_files();
        requested
    }
}

/// How the host opens the native ACP session after initialization.
///
/// Recovery remains the native agent's responsibility. `Load` asks the agent
/// to replay its conversation as ACP updates, while `Resume` deliberately does
/// not. The host never emulates either operation by re-sending old prompts.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NativeSessionStart {
    #[default]
    New,
    Load {
        session_id: String,
    },
    Resume {
        session_id: String,
    },
}

fn listed_session_id(sessions: &[SessionInfo], reconnect_id: &str) -> bool {
    sessions
        .iter()
        .any(|session| session.session_id.0.as_ref() == reconnect_id)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NativeTurnOutcome {
    /// Zero-based turn identity on this native ACP session.
    pub turn_index: usize,
    /// Concatenated text blocks retained for compatibility with text-only
    /// callers. It is not a serialization of non-text ACP content.
    pub prompt: String,
    /// Exact content blocks sent in the ACP `session/prompt` request.
    pub prompt_content: Vec<ContentBlock>,
    pub stop_reason: String,
    pub control_outcome: NativeTurnControlOutcome,
    pub reply_text: String,
    /// Exact agent message chunks observed for this turn, in wire order.
    pub reply_content: Vec<ContentBlock>,
    pub tool_calls: Vec<ToolCallSummary>,
}

/// Host-observed relationship between a surface cancellation signal and the
/// terminal ACP prompt response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeTurnControlOutcome {
    Completed,
    CompletedBeforeCancel,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NativeSessionOutcome {
    pub protocol_version: String,
    pub session_id: String,
    pub start: NativeSessionStart,
    /// Number of `session/update` notifications observed while `session/load`
    /// was in flight. It is evidence of agent-owned replay, not host memory.
    pub replayed_updates: usize,
    /// `Some(true)` means the reconnect target was reconciled through standard
    /// ACP `session/list` before load/resume. `None` means the optional
    /// capability was not advertised or this was a new session.
    pub native_session_listed: Option<bool>,
    pub agent_info: Option<Value>,
    /// Preferred ACP session configuration, preserving the agent's ordering.
    /// `None` means the agent did not expose `configOptions`; an empty vector
    /// is a distinct, explicitly supplied state.
    pub config_options: Option<Vec<SessionConfigOption>>,
    /// Transitional ACP modes state. It is preserved for fallback and
    /// diagnostics, but ignored for control when `config_options` is present.
    pub legacy_modes: Option<SessionModeState>,
    pub config_option_updates: usize,
    pub legacy_mode_updates: usize,
    pub turns: Vec<NativeTurnOutcome>,
    pub termination: NativeSessionTermination,
    pub permission_decisions: Vec<PermissionDecision>,
    pub wall_clock_ms: u64,
    pub transcript_path: Option<PathBuf>,
    /// The official ACP process launcher currently inherits the host process
    /// environment. Record that fact instead of claiming sanitization.
    pub child_environment: String,
    /// Host-owned environment lease used by this launch, if supplied.
    pub environment_lease_id: Option<String>,
    /// Observation produced only after a disposable environment was removed.
    /// Absence is honest for direct/compatibility transports or failed cleanup.
    pub environment_terminal_evidence: Option<EnvironmentGuaranteeEvidence>,
}

/// Why the host stopped holding the native ACP connection.
///
/// A disconnect preserves the agent-owned session for load/resume. `Closed`
/// is emitted only after a successful standard `session/close` response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeSessionTermination {
    Disconnected,
    Closed,
}

#[derive(Default)]
struct TurnLedger {
    prompt: String,
    prompt_content: Vec<ContentBlock>,
    reply_text: String,
    reply_content: Vec<ContentBlock>,
    tool_calls: Vec<ToolCallSummary>,
    stop_reason: Option<String>,
    control_outcome: Option<NativeTurnControlOutcome>,
    projected_content_count: usize,
}

#[derive(Default)]
struct SessionLedger {
    started: Option<Instant>,
    transcript: Option<BufWriter<File>>,
    turns: Vec<TurnLedger>,
    active_turn: Option<usize>,
    permission_decisions: Vec<PermissionDecision>,
    agent_info: Option<Value>,
    session_id: Option<String>,
    config_options: Option<Vec<SessionConfigOption>>,
    legacy_modes: Option<SessionModeState>,
    config_option_updates: usize,
    legacy_mode_updates: usize,
    boolean_config_options: bool,
    replaying: bool,
    replayed_updates: usize,
    native_session_listed: Option<bool>,
    termination: Option<NativeSessionTermination>,
}

impl SessionLedger {
    fn record(&mut self, kind: &str, payload: &Value) {
        let Some(transcript) = self.transcript.as_mut() else {
            return;
        };
        let elapsed_ms = self
            .started
            .map_or(0, |started| started.elapsed().as_millis());
        let line = serde_json::json!({
            "t_ms": elapsed_ms,
            "kind": kind,
            "payload": payload,
        });
        let _ = writeln!(transcript, "{line}");
        let _ = transcript.flush();
    }

    fn note_update(
        &mut self,
        notification: &SessionNotification,
    ) -> Result<(), agent_client_protocol::Error> {
        if self.replaying {
            self.replayed_updates += 1;
        }
        match &notification.update {
            SessionUpdate::ConfigOptionUpdate(update) => {
                validate_session_config_options(
                    &update.config_options,
                    self.boolean_config_options,
                )?;
                self.config_options = Some(update.config_options.clone());
                self.config_option_updates = self.config_option_updates.saturating_add(1);
            }
            SessionUpdate::CurrentModeUpdate(update) => {
                let modes = self.legacy_modes.as_mut().ok_or_else(|| {
                    protocol_error("agent sent current_mode_update without initial modes")
                })?;
                if !modes
                    .available_modes
                    .iter()
                    .any(|mode| mode.id == update.current_mode_id)
                {
                    return Err(protocol_error(
                        "agent selected a legacy mode absent from availableModes",
                    ));
                }
                modes.current_mode_id = update.current_mode_id.clone();
                self.legacy_mode_updates = self.legacy_mode_updates.saturating_add(1);
            }
            SessionUpdate::AgentMessageChunk(chunk) => {
                let Some(turn) = self.active_turn.and_then(|index| self.turns.get_mut(index))
                else {
                    return Ok(());
                };
                if let ContentBlock::Text(text) = &chunk.content {
                    turn.reply_text.push_str(&text.text);
                }
                turn.reply_content.push(chunk.content.clone());
            }
            SessionUpdate::ToolCall(call) => {
                let Some(turn) = self.active_turn.and_then(|index| self.turns.get_mut(index))
                else {
                    return Ok(());
                };
                turn.tool_calls.push(ToolCallSummary {
                    tool_call_id: call.tool_call_id.0.to_string(),
                    title: call.title.clone(),
                    kind: wire_name(&call.kind),
                    status: wire_name(&call.status),
                });
            }
            SessionUpdate::ToolCallUpdate(update) => {
                let Some(turn) = self.active_turn.and_then(|index| self.turns.get_mut(index))
                else {
                    return Ok(());
                };
                let id = update.tool_call_id.0.to_string();
                if let Some(existing) = turn
                    .tool_calls
                    .iter_mut()
                    .find(|call| call.tool_call_id == id)
                {
                    if let Some(status) = &update.fields.status {
                        existing.status = wire_name(status);
                    }
                    if let Some(title) = &update.fields.title {
                        existing.title.clone_from(title);
                    }
                } else {
                    turn.tool_calls.push(ToolCallSummary {
                        tool_call_id: id,
                        title: update.fields.title.clone().unwrap_or_default(),
                        kind: update
                            .fields
                            .kind
                            .as_ref()
                            .map_or_else(|| "other".into(), wire_name),
                        status: update
                            .fields
                            .status
                            .as_ref()
                            .map_or_else(|| "pending".into(), wire_name),
                    });
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn output_projection(
        &mut self,
        notification: &SessionNotification,
        source: SurfaceEventSource,
    ) -> Vec<NativeOutputProjection> {
        if source != SurfaceEventSource::NativeLive {
            return Vec::new();
        }
        let Some(turn_index) = self.active_turn else {
            return Vec::new();
        };
        let Some(turn) = self.turns.get_mut(turn_index) else {
            return Vec::new();
        };
        let content = match &notification.update {
            SessionUpdate::AgentMessageChunk(chunk) => vec![chunk.content.clone()],
            SessionUpdate::ToolCall(call) => tool_content_blocks(&call.content),
            SessionUpdate::ToolCallUpdate(update) => update
                .fields
                .content
                .as_deref()
                .map_or_else(Vec::new, tool_content_blocks),
            _ => Vec::new(),
        };
        content
            .into_iter()
            .map(|content| {
                let ordinal = turn.projected_content_count;
                turn.projected_content_count = turn.projected_content_count.saturating_add(1);
                NativeOutputProjection::Content {
                    turn_index,
                    ordinal,
                    content: Box::new(content),
                }
            })
            .collect()
    }

    fn preempt_active_tool_calls(&mut self) -> Vec<String> {
        let Some(turn) = self.active_turn.and_then(|index| self.turns.get_mut(index)) else {
            return Vec::new();
        };
        let mut cancelled = Vec::new();
        for call in &mut turn.tool_calls {
            if !matches!(call.status.as_str(), "completed" | "failed" | "cancelled") {
                call.status = "cancelled".into();
                cancelled.push(call.tool_call_id.clone());
            }
        }
        cancelled
    }

    fn permission_provenance(
        &self,
        request: &RequestPermissionRequest,
    ) -> Result<NativePermissionProvenance, String> {
        let requested_session = request.session_id.0.as_ref();
        if self.session_id.as_deref() != Some(requested_session) {
            return Err(format!(
                "permission request names session `{requested_session}` instead of the active session"
            ));
        }
        let turn = self
            .active_turn
            .and_then(|index| self.turns.get(index))
            .ok_or_else(|| "permission request arrived outside an active prompt turn".to_owned())?;
        let tool_call_id = request.tool_call.tool_call_id.0.as_ref();
        Ok(
            if turn
                .tool_calls
                .iter()
                .any(|call| call.tool_call_id == tool_call_id)
            {
                NativePermissionProvenance::CorrelatedAgentReport
            } else {
                NativePermissionProvenance::UncorrelatedAgentReport
            },
        )
    }
}

fn tool_content_blocks(content: &[ToolCallContent]) -> Vec<ContentBlock> {
    content
        .iter()
        .filter_map(|item| match item {
            ToolCallContent::Content(content) => Some(content.content.clone()),
            _ => None,
        })
        .collect()
}

fn turn_outcome(turn_index: usize, turn: &TurnLedger) -> Result<NativeTurnOutcome, SupplyError> {
    Ok(NativeTurnOutcome {
        turn_index,
        prompt: turn.prompt.clone(),
        prompt_content: turn.prompt_content.clone(),
        stop_reason: turn
            .stop_reason
            .clone()
            .ok_or(SupplyError::MissingTransactionResult)?,
        control_outcome: turn
            .control_outcome
            .ok_or(SupplyError::MissingTransactionResult)?,
        reply_text: turn.reply_text.clone(),
        reply_content: turn.reply_content.clone(),
        tool_calls: turn.tool_calls.clone(),
    })
}

fn guarded_permission_decision(
    request: &RequestPermissionRequest,
    reason: String,
) -> PermissionDecision {
    PermissionDecision {
        tool_call_id: request.tool_call.tool_call_id.0.to_string(),
        title: request.tool_call.fields.title.clone(),
        kind: request.tool_call.fields.kind.as_ref().map(wire_name),
        allowed: false,
        reason,
        selected_option: None,
        provenance: NativePermissionProvenance::UncorrelatedAgentReport,
        decision_source: NativePermissionDecisionSource::ProtocolGuard,
    }
}

fn surface_permission_decision(
    request: &RequestPermissionRequest,
    provenance: NativePermissionProvenance,
    selected_option: Option<String>,
) -> PermissionDecision {
    let selected_kind = selected_option.as_deref().and_then(|selected| {
        request
            .options
            .iter()
            .find(|option| option.option_id.0.as_ref() == selected)
            .map(|option| option.kind)
    });
    let allowed = matches!(
        selected_kind,
        Some(PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways)
    );
    let reason = match selected_kind {
        Some(PermissionOptionKind::AllowOnce) => "surface selected ACP allow_once",
        Some(PermissionOptionKind::AllowAlways) => "surface selected ACP allow_always",
        Some(PermissionOptionKind::RejectOnce) => "surface selected ACP reject_once",
        Some(PermissionOptionKind::RejectAlways) => "surface selected ACP reject_always",
        None => "surface cancelled the ACP permission request",
        _ => "surface selected an unsupported ACP permission option kind",
    };
    PermissionDecision {
        tool_call_id: request.tool_call.tool_call_id.0.to_string(),
        title: request.tool_call.fields.title.clone(),
        kind: request.tool_call.fields.kind.as_ref().map(wire_name),
        allowed,
        reason: reason.into(),
        selected_option,
        provenance,
        decision_source: NativePermissionDecisionSource::Surface,
    }
}

type SharedLedger = Arc<Mutex<SessionLedger>>;

/// Wire spelling of an ACP enum (`end_turn`, `in_progress`, ...), never Rust's `Debug`.
fn wire_name<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(Value::String(name)) => name,
        Ok(other) => other.to_string(),
        Err(_) => "unknown".into(),
    }
}

fn lock(
    ledger: &SharedLedger,
) -> Result<std::sync::MutexGuard<'_, SessionLedger>, agent_client_protocol::Error> {
    ledger
        .lock()
        .map_err(|_| agent_client_protocol::Error::internal_error())
}

fn protocol_error(message: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error()
        .data(serde_json::json!({ "message": message.into() }))
}

/// What a path outside the connection's workspace is told, in one wording,
/// so the record and the agent say the same thing about the same refusal.
/// What an agent is told when the person said no to a command, in one
/// wording, so the record and the agent say the same thing about it.
const NOT_ALLOWED_TO_RUN: &str = "this command was not allowed to run";

pub(crate) const OUTSIDE_THE_BOUNDARY: &str =
    "the path is outside the workspace this session works in";

/// Record a callback this connection did not carry out.
///
/// Every refusal is a decision with a reason, and both doors read it: the
/// ledger keeps it as evidence, and a watching surface sees it happen.
fn record_callback_refusal(
    ledger: &SharedLedger,
    control: Option<&NativeSessionControl>,
    method: &str,
    reason: Option<&str>,
) -> Result<(), agent_client_protocol::Error> {
    let decision = match reason {
        Some(reason) => serde_json::json!({ "method": method, "reason": reason }),
        None => serde_json::json!({ "method": method }),
    };
    lock(ledger)?.record("host/callback_refused", &decision);
    if let Some(control) = control {
        control.publish_surface_event("host/callback_refused", SurfaceEventSource::Host, decision);
    }
    Ok(())
}

/// Record that a file callback was carried out to a surface, and how it came
/// back.
///
/// The file's own text is never here, on the way out or on the way back. What
/// the record is for is that the agent touched this path and whether it got
/// what it asked for; the content belongs to the live exchange and to the
/// file.
fn record_file_callback(
    ledger: &SharedLedger,
    method: &str,
    path: &Path,
    answer: Option<&NativeFileAnswer>,
) -> Result<(), agent_client_protocol::Error> {
    let outcome = match answer {
        Some(NativeFileAnswer::Text(_)) => "read",
        Some(NativeFileAnswer::Written) => "written",
        Some(NativeFileAnswer::Refused(_)) => "refused",
        None => "unanswered",
    };
    lock(ledger)?.record(
        "host/file_callback",
        &serde_json::json!({
            "method": method,
            "path": path.display().to_string(),
            "outcome": outcome,
        }),
    );
    Ok(())
}

/// Record that a terminal callback was carried out, and how it went.
///
/// What a terminal carries is never recorded - that is the rule the panel's
/// own module states, and an agent's terminal is the same terminal. What is
/// recorded is the command line, because the whole point of this capability
/// is that a person can find out what their agent did, and `/bin/sh` answers
/// nothing. That is also what the question asks when the profile asks, and a
/// record that kept less than the question would be a second story about the
/// same command.
fn record_terminal_callback(
    ledger: &SharedLedger,
    control: Option<&NativeSessionControl>,
    method: &str,
    terminal_id: Option<&str>,
    command_line: Option<&str>,
    outcome: &str,
) -> Result<(), agent_client_protocol::Error> {
    let mut decision = serde_json::json!({ "method": method, "outcome": outcome });
    if let Some(entry) = decision.as_object_mut() {
        if let Some(terminal_id) = terminal_id {
            entry.insert("terminal_id".into(), terminal_id.into());
        }
        if let Some(command_line) = command_line {
            entry.insert("command".into(), command_line.into());
        }
    }
    lock(ledger)?.record("host/terminal_callback", &decision);
    if let Some(control) = control {
        control.publish_surface_event("host/terminal_callback", SurfaceEventSource::Host, decision);
    }
    Ok(())
}

/// What the agent hears when a terminal callback could not be carried out.
///
/// A working directory outside the workspace and a terminal that is not this
/// connection's are both bad arguments, and the agent can act on that in its
/// own turn; anything else is the host's failure to say so.
fn terminal_callback_error(
    method: &str,
    error: &crate::workbench_shell::WorkbenchShellError,
) -> agent_client_protocol::Error {
    use crate::workbench_shell::WorkbenchShellError;
    let (base, reason) = match error {
        WorkbenchShellError::Invalid(reason) | WorkbenchShellError::NotFound(reason) => {
            (agent_client_protocol::Error::invalid_params(), reason)
        }
        WorkbenchShellError::Conflict(reason) | WorkbenchShellError::Failed(reason) => {
            (agent_client_protocol::Error::internal_error(), reason)
        }
    };
    base.data(serde_json::json!({ "method": method, "reason": reason }))
}

/// One command, as a person reads it: the program and what it was given.
fn command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program)
        .chain(args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// How a terminal ended, in ACP's own reading.
fn exit_status(report: &crate::workbench_shell::ExitReport) -> TerminalExitStatus {
    TerminalExitStatus::new()
        .exit_code(report.code)
        .signal(report.signal.clone())
}

fn unadvertised_callback_error(method: &str) -> agent_client_protocol::Error {
    agent_client_protocol::Error::method_not_found().data(serde_json::json!({ "method": method }))
}

fn outside_the_boundary_error(method: &str, path: &Path) -> agent_client_protocol::Error {
    agent_client_protocol::Error::invalid_params().data(serde_json::json!({
        "method": method,
        "path": path.display().to_string(),
        "reason": OUTSIDE_THE_BOUNDARY,
    }))
}

/// What the agent hears when a file callback did not come back with what it
/// asked for: an error on its own callback, inside its own turn, so it can
/// act on the file not being read rather than wait out a deadline.
fn file_callback_error(
    method: &str,
    answer: Option<&NativeFileAnswer>,
) -> agent_client_protocol::Error {
    let reason = match answer {
        Some(NativeFileAnswer::Refused(reason)) => reason.clone(),
        Some(_) => "the surface answered something other than this method asked for".to_owned(),
        None => "no surface carried this out".to_owned(),
    };
    agent_client_protocol::Error::internal_error()
        .data(serde_json::json!({ "method": method, "reason": reason }))
}

/// True when this exact path is inside the boundary with no way out of it.
///
/// Canonicalizing the path itself is not enough, because a file a write names
/// does not have to exist yet, and `canonicalize` of a missing path fails. So
/// what is resolved is the nearest ancestor that does exist - symlinks and
/// all - and what is left over must be plain names. A leftover `..` is
/// refused rather than folded away: a component that does not exist cannot be
/// canonicalized, and a lexical answer about where a symlink would have gone
/// is a guess. Refusing is the guess that cannot widen the boundary.
pub(crate) fn inside_boundary(path: &Path, boundary: &Path) -> bool {
    if !path.is_absolute() {
        return false;
    }
    let Ok(boundary) = fs::canonicalize(boundary) else {
        return false;
    };
    let mut existing = path.to_path_buf();
    let mut unresolved: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(resolved) = fs::canonicalize(&existing) {
            return resolved.starts_with(&boundary)
                && unresolved
                    .iter()
                    .all(|name| name != ".." && name != "." && !name.is_empty());
        }
        let Some(name) = existing.file_name().map(std::ffi::OsStr::to_os_string) else {
            return false;
        };
        unresolved.push(name);
        if !existing.pop() {
            return false;
        }
    }
}

const OPERATION_TIMEOUT_MARKER: &str = "swem.operation_timeout";

fn operation_timeout_error(operation: &'static str) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(serde_json::json!({
        "swem_error": OPERATION_TIMEOUT_MARKER,
        "operation": operation,
    }))
}

fn timed_out_operation(error: &agent_client_protocol::Error) -> Option<String> {
    let data = error.data.as_ref()?;
    if data.get("swem_error")?.as_str()? != OPERATION_TIMEOUT_MARKER {
        return None;
    }
    data.get("operation")?.as_str().map(str::to_owned)
}

fn supply_error_from_acp(error: &agent_client_protocol::Error) -> SupplyError {
    timed_out_operation(error).map_or_else(
        || SupplyError::Protocol(error.to_string()),
        |operation| SupplyError::OperationTimeout { operation },
    )
}

async fn await_acp_operation<T, F>(
    operation: &'static str,
    timeout: Duration,
    future: F,
) -> Result<T, agent_client_protocol::Error>
where
    F: Future<Output = Result<T, agent_client_protocol::Error>>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| operation_timeout_error(operation))?
}

fn select_contains(
    options: &SessionConfigSelectOptions,
    wanted: &agent_client_protocol::schema::v1::SessionConfigValueId,
) -> bool {
    match options {
        SessionConfigSelectOptions::Ungrouped(options) => {
            options.iter().any(|option| option.value == *wanted)
        }
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| &group.options)
            .any(|option| option.value == *wanted),
        _ => false,
    }
}

fn select_values_are_valid(options: &SessionConfigSelectOptions) -> bool {
    let values: Vec<&str> = match options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .map(|option| option.value.0.as_ref())
            .collect(),
        SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| &group.options)
            .map(|option| option.value.0.as_ref())
            .collect(),
        _ => return false,
    };
    let mut unique = BTreeSet::new();
    !values.is_empty()
        && values
            .into_iter()
            .all(|value| !value.is_empty() && unique.insert(value))
}

fn validate_session_config_options(
    options: &[SessionConfigOption],
    boolean_supported: bool,
) -> Result<(), agent_client_protocol::Error> {
    let mut ids = BTreeSet::new();
    for option in options {
        let id = option.id.0.as_ref();
        if id.is_empty() || !ids.insert(id) {
            return Err(protocol_error(
                "ACP configOptions ids must be non-empty and unique",
            ));
        }
        match &option.kind {
            SessionConfigKind::Select(select) => {
                if !select_values_are_valid(&select.options)
                    || !select_contains(&select.options, &select.current_value)
                {
                    return Err(protocol_error(
                        "ACP select config option has invalid choices or currentValue",
                    ));
                }
            }
            SessionConfigKind::Boolean(_) if !boolean_supported => {
                return Err(protocol_error(
                    "agent sent a boolean config option without exact client capability",
                ));
            }
            SessionConfigKind::Boolean(_) => {}
            _ => {
                return Err(protocol_error(
                    "unsupported ACP session config option kind reached the host",
                ));
            }
        }
    }
    Ok(())
}

fn validate_session_modes(modes: &SessionModeState) -> Result<(), agent_client_protocol::Error> {
    let mut ids = BTreeSet::new();
    for mode in &modes.available_modes {
        let id = mode.id.0.as_ref();
        if id.is_empty() || !ids.insert(id) {
            return Err(protocol_error(
                "ACP legacy mode ids must be non-empty and unique",
            ));
        }
    }
    if modes
        .available_modes
        .iter()
        .any(|mode| mode.id == modes.current_mode_id)
    {
        Ok(())
    } else {
        Err(protocol_error(
            "ACP legacy currentModeId is absent from availableModes",
        ))
    }
}

fn replace_initial_configuration(
    ledger: &SharedLedger,
    config_options: Option<Vec<SessionConfigOption>>,
    legacy_modes: Option<SessionModeState>,
) -> Result<(), agent_client_protocol::Error> {
    let mut state = lock(ledger)?;
    if let Some(options) = config_options.as_deref() {
        validate_session_config_options(options, state.boolean_config_options)?;
    }
    if let Some(modes) = &legacy_modes {
        validate_session_modes(modes)?;
    }
    state.config_options = config_options;
    state.legacy_modes = legacy_modes;
    Ok(())
}

fn validate_config_selection(
    options: Option<&[SessionConfigOption]>,
    config_id: &str,
    value: &SessionConfigOptionValue,
) -> Result<(), SupplyError> {
    let options = options.ok_or_else(|| {
        SupplyError::Protocol("agent did not expose ACP session configOptions".into())
    })?;
    let option = options
        .iter()
        .find(|option| option.id.0.as_ref() == config_id)
        .ok_or_else(|| {
            SupplyError::Protocol(format!(
                "agent did not expose ACP session config option `{config_id}`"
            ))
        })?;
    let valid = match (&option.kind, value) {
        (SessionConfigKind::Select(select), SessionConfigOptionValue::ValueId { value }) => {
            select_contains(&select.options, value)
        }
        (SessionConfigKind::Boolean(_), SessionConfigOptionValue::Boolean { .. }) => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(SupplyError::Protocol(format!(
            "value does not match ACP session config option `{config_id}`"
        )))
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the two ACP configuration command variants share one ordered connection dispatcher"
)]
async fn apply_native_session_command(
    connection: &ConnectionTo<Agent>,
    session_id: &agent_client_protocol::schema::v1::SessionId,
    command: NativeSessionCommand,
    ledger: &SharedLedger,
    operation_timeout: Duration,
) -> Result<(), agent_client_protocol::Error> {
    match command {
        NativeSessionCommand::SetConfigOption {
            config_id,
            value,
            response,
        } => {
            let validation = {
                let state = lock(ledger)?;
                validate_config_selection(state.config_options.as_deref(), &config_id, &value)
            };
            if let Err(error) = validation {
                let _ = response.send(Err(error));
                return Ok(());
            }
            let request =
                SetSessionConfigOptionRequest::new(session_id.clone(), config_id.clone(), value);
            let result = await_acp_operation(
                "session/set_config_option",
                operation_timeout,
                connection.send_request(request).block_task(),
            )
            .await;
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    let _ = response.send(Err(supply_error_from_acp(&error)));
                    return Err(error);
                }
            };
            let mut state = lock(ledger)?;
            if let Err(error) = validate_session_config_options(
                &result.config_options,
                state.boolean_config_options,
            ) {
                let _ = response.send(Err(SupplyError::Protocol(error.to_string())));
                return Err(error);
            }
            state.config_options = Some(result.config_options.clone());
            state.record(
                "session/set_config_option",
                &serde_json::json!({
                    "config_id": config_id,
                    "response": result,
                }),
            );
            let _ = response.send(Ok(result.config_options));
        }
        NativeSessionCommand::Configuration { response } => {
            let state = lock(ledger)?;
            let _ = response.send(SessionConfiguration {
                config_options: state.config_options.clone(),
                legacy_modes: state.legacy_modes.clone(),
            });
        }
        NativeSessionCommand::SetLegacyMode { mode_id, response } => {
            let selected = {
                let state = lock(ledger)?;
                if state.config_options.is_some() {
                    Err(SupplyError::Protocol(
                        "ACP configOptions supersede legacy session modes".into(),
                    ))
                } else if state.legacy_modes.as_ref().is_some_and(|modes| {
                    modes
                        .available_modes
                        .iter()
                        .any(|mode| mode.id.0.as_ref() == mode_id)
                }) {
                    Ok(())
                } else {
                    Err(SupplyError::Protocol(format!(
                        "agent did not expose legacy ACP mode `{mode_id}`"
                    )))
                }
            };
            if let Err(error) = selected {
                let _ = response.send(Err(error));
                return Ok(());
            }
            let result = await_acp_operation(
                "session/set_mode",
                operation_timeout,
                connection
                    .send_request(SetSessionModeRequest::new(
                        session_id.clone(),
                        SessionModeId::new(mode_id.clone()),
                    ))
                    .block_task(),
            )
            .await;
            if let Err(error) = result {
                let _ = response.send(Err(supply_error_from_acp(&error)));
                return Err(error);
            }
            let mut state = lock(ledger)?;
            let modes = state
                .legacy_modes
                .as_mut()
                .ok_or_else(|| protocol_error("legacy mode state disappeared"))?;
            modes.current_mode_id = SessionModeId::new(mode_id.clone());
            let complete = modes.clone();
            state.record(
                "session/set_mode",
                &serde_json::json!({ "mode_id": mode_id }),
            );
            let _ = response.send(Ok(complete));
        }
    }
    Ok(())
}

async fn next_native_session_command(
    commands: &mut Option<mpsc::UnboundedReceiver<NativeSessionCommand>>,
) -> Option<NativeSessionCommand> {
    match commands {
        Some(commands) => commands.recv().await,
        None => std::future::pending().await,
    }
}

/// Build the stdio MCP server declaration injected into the agent session.
#[must_use]
pub fn swem_mcp_server(
    mcp_executable: &Path,
    workspace: &Path,
    grant_local_execution: bool,
    grant_publish: bool,
    journal_dir: Option<&Path>,
    plugins: &[PathBuf],
) -> McpServer {
    let mut args = vec![
        "--mcp".to_owned(),
        "--workspace".to_owned(),
        workspace.display().to_string(),
    ];
    // The packages this host loaded. The Cycle in the session is a separate
    // process with its own assembly, so a package it is not pointed at is a
    // package it does not have.
    for directory in plugins {
        args.push("--plugins".to_owned());
        args.push(directory.display().to_string());
    }
    if grant_local_execution {
        args.push("--allow-local-execution".to_owned());
    }
    if grant_publish {
        args.push("--allow-publish".to_owned());
    }
    if let Some(journal) = journal_dir {
        args.push("--journal".to_owned());
        args.push(journal.display().to_string());
    }
    McpServer::Stdio(McpServerStdio::new(SWEM_MCP_SERVER_NAME, mcp_executable).args(args))
}

fn content_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn redacted_wire_value<T: Serialize>(value: &T) -> Value {
    let mut value = serde_json::to_value(value).unwrap_or(Value::Null);
    redact_binary_payloads(&mut value);
    value
}

fn redacted_elicitation_descriptor(offered: &NativeElicitationRequest) -> Value {
    let (mode, scope, schema) = match &offered.request.mode {
        ElicitationMode::Form(form) => (
            "form",
            &form.scope,
            Some(serde_json::json!({
                "property_names": form.requested_schema.properties.keys().collect::<Vec<_>>(),
                "required": form.requested_schema.required,
            })),
        ),
        ElicitationMode::Url(url) => ("url", &url.scope, None),
        ElicitationMode::Other(other) => (other.mode.as_str(), &other.scope, None),
        _ => ("unknown", offered.request.scope(), None),
    };
    let scope = match scope {
        ElicitationScope::Session(scope) => serde_json::json!({
            "kind": "session",
            "session_id": scope.session_id.0.as_ref(),
            "tool_call_id": scope.tool_call_id.as_ref().map(|id| id.0.as_ref()),
        }),
        ElicitationScope::Request(scope) => serde_json::json!({
            "kind": "request",
            "request_id": &scope.request_id,
        }),
        _ => serde_json::json!({"kind": "unknown"}),
    };
    serde_json::json!({
        "sequence": offered.sequence,
        "mode": mode,
        "scope": scope,
        "message_length": offered.request.message.len(),
        "message_sha256": digest_bytes(offered.request.message.as_bytes()),
        "schema": schema,
    })
}

fn validate_elicitation_action(
    request: &CreateElicitationRequest,
    action: &ElicitationAction,
) -> Result<(), SupplyError> {
    let ElicitationAction::Accept(accepted) = action else {
        return Ok(());
    };
    let ElicitationMode::Form(form) = &request.mode else {
        return Ok(());
    };
    let mut schema = serde_json::to_value(&form.requested_schema)
        .map_err(|error| SupplyError::Protocol(format!("invalid ACP form schema: {error}")))?;
    // The ACP form can only submit fields the agent declared. JSON Schema's
    // default is permissive, so make this transport invariant explicit rather
    // than allowing a forged HTTP body to smuggle extra connection-local data.
    schema
        .as_object_mut()
        .ok_or_else(|| SupplyError::Protocol("ACP form schema is not an object".into()))?
        .insert("additionalProperties".into(), Value::Bool(false));
    let empty = BTreeMap::new();
    let instance = serde_json::to_value(accepted.content.as_ref().unwrap_or(&empty))
        .map_err(|error| SupplyError::Protocol(format!("invalid ACP form content: {error}")))?;
    let validator = jsonschema::draft202012::options()
        .should_validate_formats(true)
        .build(&schema)
        .map_err(|error| SupplyError::Protocol(format!("invalid ACP form schema: {error}")))?;
    validator.validate(&instance).map_err(|error| {
        SupplyError::Protocol(format!(
            "ACP elicitation response does not satisfy its schema: {error}"
        ))
    })
}

fn redact_binary_payloads(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                redact_binary_payloads(value);
            }
        }
        Value::Object(object) => {
            let content_type = object.get("type").and_then(Value::as_str);
            if matches!(content_type, Some("image" | "audio")) {
                redact_base64_field(object, "data");
            }
            if object.contains_key("uri") && object.contains_key("blob") {
                redact_base64_field(object, "blob");
            }
            if object.contains_key("uri") && object.contains_key("text") {
                redact_text_field(object);
            }
            for value in object.values_mut() {
                redact_binary_payloads(value);
            }
        }
        _ => {}
    }
}

fn redact_text_field(object: &mut serde_json::Map<String, Value>) {
    let Some(Value::String(text)) = object.get("text") else {
        return;
    };
    let descriptor = serde_json::json!({
        "encoding": "utf-8",
        "byte_length": text.len(),
        "sha256": digest_bytes(text.as_bytes()),
    });
    object.remove("text");
    object.insert("textDescriptor".into(), descriptor);
}

fn redact_base64_field(object: &mut serde_json::Map<String, Value>, key: &str) {
    let Some(Value::String(encoded)) = object.get(key) else {
        return;
    };
    let descriptor = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(bytes) => serde_json::json!({
            "encoding": "base64",
            "byte_length": bytes.len(),
            "sha256": digest_bytes(&bytes),
        }),
        Err(_) => serde_json::json!({
            "encoding": "invalid_base64",
            "encoded_length": encoded.len(),
            "encoded_sha256": digest_bytes(encoded.as_bytes()),
        }),
    };
    object.remove(key);
    object.insert(format!("{key}Descriptor"), descriptor);
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn decode_base64(data: &str, content_type: &str) -> Result<Vec<u8>, agent_client_protocol::Error> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| {
            agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                "message": "ACP binary content is not valid standard base64",
                "content_type": content_type,
                "error": error.to_string(),
            }))
        })
}

fn content_descriptors(
    blocks: &[ContentBlock],
) -> Result<Vec<Value>, agent_client_protocol::Error> {
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text(text) => Ok(serde_json::json!({
                "type": "text",
                "text": text.text,
                "byte_length": text.text.len(),
                "sha256": digest_bytes(text.text.as_bytes()),
            })),
            ContentBlock::Image(image) => {
                let bytes = decode_base64(&image.data, "image")?;
                Ok(serde_json::json!({
                    "type": "image",
                    "mime_type": image.mime_type,
                    "uri": image.uri,
                    "byte_length": bytes.len(),
                    "sha256": digest_bytes(&bytes),
                }))
            }
            ContentBlock::Audio(audio) => {
                let bytes = decode_base64(&audio.data, "audio")?;
                Ok(serde_json::json!({
                    "type": "audio",
                    "mime_type": audio.mime_type,
                    "byte_length": bytes.len(),
                    "sha256": digest_bytes(&bytes),
                }))
            }
            ContentBlock::ResourceLink(resource) => Ok(serde_json::json!({
                "type": "resource_link",
                "name": resource.name,
                "uri": resource.uri,
                "mime_type": resource.mime_type,
                "size": resource.size,
            })),
            ContentBlock::Resource(resource) => match &resource.resource {
                EmbeddedResourceResource::TextResourceContents(text) => Ok(serde_json::json!({
                    "type": "resource",
                    "representation": "text",
                    "uri": text.uri,
                    "mime_type": text.mime_type,
                    "byte_length": text.text.len(),
                    "sha256": digest_bytes(text.text.as_bytes()),
                })),
                EmbeddedResourceResource::BlobResourceContents(blob) => {
                    let bytes = decode_base64(&blob.blob, "resource_blob")?;
                    Ok(serde_json::json!({
                        "type": "resource",
                        "representation": "blob",
                        "uri": blob.uri,
                        "mime_type": blob.mime_type,
                        "byte_length": bytes.len(),
                        "sha256": digest_bytes(&bytes),
                    }))
                }
                _ => Err(
                    agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                        "message": "unknown ACP embedded resource representation",
                    })),
                ),
            },
            _ => Err(
                agent_client_protocol::Error::invalid_params().data(serde_json::json!({
                    "message": "unknown ACP content block variant",
                })),
            ),
        })
        .collect()
}

/// Whether an agent that advertised `capabilities` can take this block. Text
/// and a resource link are the two every agent takes, which is why a file
/// handed over by path reaches an agent that can take nothing else.
#[must_use]
pub fn agent_takes(block: &ContentBlock, capabilities: &PromptCapabilities) -> bool {
    match block {
        ContentBlock::Text(_) | ContentBlock::ResourceLink(_) => true,
        ContentBlock::Image(_) => capabilities.image,
        ContentBlock::Audio(_) => capabilities.audio,
        ContentBlock::Resource(_) => capabilities.embedded_context,
        _ => false,
    }
}

fn validate_prompt_content(
    blocks: &[ContentBlock],
    capabilities: &PromptCapabilities,
) -> Result<Vec<Value>, agent_client_protocol::Error> {
    if blocks.is_empty() {
        return Err(agent_client_protocol::Error::invalid_params()
            .data(serde_json::json!({ "message": "ACP prompt content is empty" })));
    }
    for block in blocks {
        if !agent_takes(block, capabilities) {
            let content_type = match block {
                ContentBlock::Image(_) => "image",
                ContentBlock::Audio(_) => "audio",
                ContentBlock::Resource(_) => "resource",
                _ => "unknown",
            };
            return Err(agent_client_protocol::Error::invalid_params().data(
                serde_json::json!({
                    "message": "agent did not advertise the ACP prompt capability required by content",
                    "content_type": content_type,
                }),
            ));
        }
    }
    content_descriptors(blocks)
}

/// Compatibility wrapper for text-only native ACP prompts.
///
/// # Errors
///
/// Returns [`SupplyError`] under the same validation, protocol, process and
/// timeout conditions as [`run_native_session_with_content`].
pub async fn run_native_session(
    agent: &LaunchCommand,
    agent_executable: &Path,
    workspace: &Path,
    prompts: &[String],
    options: &NativeSessionOptions,
) -> Result<NativeSessionOutcome, SupplyError> {
    let content = prompts
        .iter()
        .map(|prompt| vec![ContentBlock::Text(TextContent::new(prompt))])
        .collect::<Vec<_>>();
    run_native_session_with_content(agent, agent_executable, workspace, &content, options).await
}

/// Run one or more content-preserving turns in one native ACP session.
///
/// The caller supplies ordinary ACP MCP declarations. This function does not
/// know which, if any, server is SWEM Cycle. The official process transport
/// currently inherits the host environment, which is reported in the outcome.
///
/// # Errors
///
/// Returns [`SupplyError`] for invalid input, authentication demands,
/// protocol/process failure or timeout.
#[allow(
    clippy::too_many_lines,
    reason = "one linear ACP turn: initialize, session/new, prompt, evidence; splitting would hide the protocol order"
)]
pub async fn run_native_session_with_content(
    agent: &LaunchCommand,
    agent_executable: &Path,
    workspace: &Path,
    prompts: &[Vec<ContentBlock>],
    options: &NativeSessionOptions,
) -> Result<NativeSessionOutcome, SupplyError> {
    run_native_session_core(agent, agent_executable, workspace, prompts, options, false).await
}

/// Hold one native ACP connection open for caller-supplied interactive turns.
///
/// The caller submits exact [`ContentBlock`] arrays through the required
/// [`NativeSessionControl`]. The host retains only bounded in-flight commands
/// and observable evidence; native session history remains agent-owned.
/// The future completes after [`NativeSessionControl::disconnect`], successful
/// [`NativeSessionControl::close_session`], or failure.
///
/// # Errors
///
/// Returns [`SupplyError`] under the same validation, protocol, process and
/// timeout conditions as [`run_native_session_with_content`], and when no
/// control handle was supplied.
pub async fn run_interactive_native_session(
    agent: &LaunchCommand,
    agent_executable: &Path,
    workspace: &Path,
    options: &NativeSessionOptions,
) -> Result<NativeSessionOutcome, SupplyError> {
    run_native_session_core(agent, agent_executable, workspace, &[], options, true).await
}

/// Every exit of the core, including a failure before the agent process
/// exists (bad workspace, lease mismatch, an executable that is not a file),
/// leaves the control's phase `Finished`. Without this, a runner that failed
/// before `initialize` kept the phase `Connecting` and every `wait_until_ready`
/// caller - the Workbench shell's open path among them - waited forever
/// instead of receiving the failure.
async fn run_native_session_core(
    agent: &LaunchCommand,
    agent_executable: &Path,
    workspace: &Path,
    prompts: &[Vec<ContentBlock>],
    options: &NativeSessionOptions,
    interactive: bool,
) -> Result<NativeSessionOutcome, SupplyError> {
    let result = run_native_session_core_inner(
        agent,
        agent_executable,
        workspace,
        prompts,
        options,
        interactive,
    )
    .await;
    if result.is_err()
        && let Some(control) = &options.control
        && !matches!(control.phase(), NativeSessionPhase::Finished)
    {
        control.set_phase(&NativeSessionPhase::Finished);
    }
    result
}

#[allow(
    clippy::too_many_lines,
    reason = "one linear ACP connection: initialize, session setup, interactive turns and evidence"
)]
async fn run_native_session_core_inner(
    agent: &LaunchCommand,
    agent_executable: &Path,
    workspace: &Path,
    prompts: &[Vec<ContentBlock>],
    options: &NativeSessionOptions,
    interactive: bool,
) -> Result<NativeSessionOutcome, SupplyError> {
    if !workspace.is_absolute() || !workspace.is_dir() {
        return Err(SupplyError::InvalidWorkspace(
            workspace.display().to_string(),
        ));
    }
    if prompts.is_empty() && !interactive {
        return Err(SupplyError::Protocol(
            "native session needs at least one prompt turn".into(),
        ));
    }
    if interactive && options.control.is_none() {
        return Err(SupplyError::Protocol(
            "interactive native session requires NativeSessionControl".into(),
        ));
    }
    if options
        .elicitation_capabilities
        .as_ref()
        .is_some_and(|caps| caps.form.is_some() || caps.url.is_some())
        && options.control.is_none()
    {
        return Err(SupplyError::Protocol(
            "advertised ACP elicitation requires NativeSessionControl".into(),
        ));
    }
    let canonical_workspace = workspace
        .canonicalize()
        .map_err(|_| SupplyError::InvalidWorkspace(workspace.display().to_string()))?;
    if let Some(lease) = &options.environment_lease {
        if lease.workspace != canonical_workspace {
            return Err(SupplyError::Protocol(format!(
                "environment lease {} is bound to {}, not {}",
                lease.lease_id,
                lease.workspace.display(),
                canonical_workspace.display()
            )));
        }
        if lease.backend_id != "direct-process" {
            let transport = options.environment_transport.as_ref().ok_or_else(|| {
                SupplyError::Protocol(format!(
                    "environment backend {} needs an exact connection-local transport",
                    lease.backend_id
                ))
            })?;
            if !transport.matches_lease(lease) {
                return Err(SupplyError::Protocol(
                    "environment transport identity differs from its lease".into(),
                ));
            }
            if !options.process_environment_overrides.is_empty() {
                return Err(SupplyError::Protocol(
                    "container environment is fixed before inspection; session overrides are not allowed"
                        .into(),
                ));
            }
        }
    } else if options.environment_transport.is_some() {
        return Err(SupplyError::Protocol(
            "environment transport requires its exact lease".into(),
        ));
    }
    let agent_workspace = options.environment_lease.as_ref().map_or_else(
        || canonical_workspace.clone(),
        |lease| lease.agent_workspace.clone(),
    );
    if matches!(options.permission_policy, SessionPermissionPolicy::Surface)
        && options.control.is_none()
    {
        return Err(SupplyError::Protocol(
            "surface permission policy requires NativeSessionControl".into(),
        ));
    }
    if let Some(control) = &options.control {
        control.publish_surface_event(
            "host/session_connecting",
            SurfaceEventSource::Host,
            serde_json::json!({
                "start": options.start,
                "workspace": canonical_workspace,
                "mcp_server_count": options.mcp_servers.len(),
            }),
        );
    }
    let transcript = match &options.transcript_path {
        Some(path) => {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|error| SupplyError::Protocol(format!("transcript dir: {error}")))?;
            }
            Some(BufWriter::new(File::create(path).map_err(|error| {
                SupplyError::Protocol(format!("transcript file: {error}"))
            })?))
        }
        None => None,
    };
    let started = Instant::now();
    let ledger: SharedLedger = Arc::new(Mutex::new(SessionLedger {
        started: Some(started),
        transcript,
        boolean_config_options: options.boolean_config_options,
        ..SessionLedger::default()
    }));
    let child_environment = match (
        options
            .environment_lease
            .as_ref()
            .map(|lease| lease.backend_id.as_str()),
        options.process_environment_overrides.is_empty(),
    ) {
        (Some("direct-process"), true) => "allowlisted",
        (Some("direct-process"), false) => "allowlisted_with_explicit_values",
        // Overrides on a non-direct lease were already rejected above.
        (Some(_), _) => "inspected_environment_baseline",
        (None, true) => "inherited",
        (None, false) => "inherited_with_explicit_overrides",
    };
    lock(&ledger).map_err(|_| SupplyError::Poisoned)?.record(
        "launch",
        &serde_json::json!({
            "agent": agent,
            "agent_executable": agent_executable.display().to_string(),
            "workspace": workspace.display().to_string(),
            "agent_workspace": agent_workspace.display().to_string(),
            "mcp_server_count": options.mcp_servers.len(),
            "permission_policy": options.permission_policy,
            "boolean_config_options": options.boolean_config_options,
            "child_environment": child_environment,
            "child_environment_override_names": options.process_environment_overrides.keys().collect::<Vec<_>>(),
            "environment_lease_id": options.environment_lease.as_ref().map(|lease| lease.lease_id.as_str()),
            "environment_guarantee_evidence": options.environment_lease.as_ref().map(|lease| &lease.evidence),
        }),
    );

    let policy = options.permission_policy.clone();
    let notification_ledger = Arc::clone(&ledger);
    let elicitation_completion_ledger = Arc::clone(&ledger);
    let permission_ledger = Arc::clone(&ledger);
    let elicitation_ledger = Arc::clone(&ledger);
    let main_ledger = Arc::clone(&ledger);
    let mcp_servers = options.mcp_servers.clone();
    let session_start = options.start.clone();
    let outcome_start = options.start.clone();
    let session_control = options.control.clone();
    let notification_control = options.control.clone();
    let elicitation_completion_control = options.control.clone();
    let permission_control = options.control.clone();
    let elicitation_control = options.control.clone();
    let result_control = options.control.clone();
    let boolean_config_options = options.boolean_config_options;
    let elicitation_capabilities = options.elicitation_capabilities.clone();
    let operation_timeout = options.operation_timeout;
    let auth_method_id = options.auth_method_id.clone();
    let connection_timeout = options.connection_timeout;
    let workspace = agent_workspace;
    let prompts = prompts.to_vec();
    let transport = match &options.environment_lease {
        Some(lease) if lease.backend_id == "direct-process" => DynConnectTo::<Client>::new(
            EnvironmentTransport::direct(
                lease,
                agent_executable,
                agent.args.clone(),
                options.process_environment_overrides.clone(),
            )
            .map_err(|error| SupplyError::Protocol(error.to_string()))?,
        ),
        Some(_) => {
            let environment_transport = options.environment_transport.clone().ok_or_else(|| {
                SupplyError::Protocol(
                    "non-direct environment transport disappeared after validation".into(),
                )
            })?;
            DynConnectTo::<Client>::new(environment_transport)
        }
        None => DynConnectTo::<Client>::new(AcpAgent::new(
            AcpAgentConfig::new(agent_executable)
                .args(agent.args.clone())
                .envs(options.process_environment_overrides.clone()),
        )),
    };
    // Claim ephemeral command receivers only after every fallible local
    // transport preparation step. A rejected launch must not leave a control
    // looking attached to a runner that never existed.
    let mut session_commands = options
        .control
        .as_ref()
        .map(NativeSessionControl::take_commands)
        .transpose()?;
    let mut driver_commands = if interactive {
        Some(
            options
                .control
                .as_ref()
                .ok_or_else(|| {
                    SupplyError::Protocol("interactive native session control disappeared".into())
                })?
                .take_driver_commands()?,
        )
    } else {
        None
    };

    // Clones for the two `fs/*` handlers: each one either forwards or
    // refuses, and it is the connection's own `file_callbacks` that decides
    // which.
    let read_file_ledger = Arc::clone(&ledger);
    let write_file_ledger = Arc::clone(&ledger);
    let read_file_control = options.control.clone();
    let write_file_control = options.control.clone();
    let read_file_callbacks = options.file_callbacks.clone();
    let write_file_callbacks = options.file_callbacks.clone();
    // One clone of each per handler: five methods, all reading the same
    // `terminal_callbacks`.
    let create_terminal_ledger = Arc::clone(&ledger);
    let output_terminal_ledger = Arc::clone(&ledger);
    let wait_terminal_ledger = Arc::clone(&ledger);
    let kill_terminal_ledger = Arc::clone(&ledger);
    let release_terminal_ledger = Arc::clone(&ledger);
    let create_terminal_control = options.control.clone();
    let output_terminal_control = options.control.clone();
    let wait_terminal_control = options.control.clone();
    let kill_terminal_control = options.control.clone();
    let release_terminal_control = options.control.clone();
    let terminal_callbacks = options.terminal_callbacks.clone().map(Arc::new);
    let create_terminal_callbacks = terminal_callbacks.clone();
    let output_terminal_callbacks = terminal_callbacks.clone();
    let wait_terminal_callbacks = terminal_callbacks.clone();
    let kill_terminal_callbacks = terminal_callbacks.clone();
    let release_terminal_callbacks = terminal_callbacks.clone();
    let terminal_capability = terminal_callbacks.is_some();
    let file_capabilities = options
        .file_callbacks
        .as_ref()
        .map(|callbacks| callbacks.capabilities.clone());

    let connection = Client
        .builder()
        .name("swem-host")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                let mut ledger = lock(&notification_ledger)?;
                let source = if ledger.replaying {
                    SurfaceEventSource::NativeReplay
                } else {
                    SurfaceEventSource::NativeLive
                };
                let output_projection = ledger.output_projection(&notification, source);
                ledger.note_update(&notification)?;
                let payload = redacted_wire_value(&notification);
                ledger.record("session/update", &payload);
                drop(ledger);
                if let Some(control) = &notification_control {
                    control.publish_surface_event_with_output(
                        "acp/session_update",
                        source,
                        payload,
                        output_projection,
                    );
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_notification(
            async move |notification: CompleteElicitationNotification, _cx| {
                let Some(descriptor) = elicitation_completion_control
                    .as_ref()
                    .and_then(|control| control.complete_url_elicitation(&notification))
                else {
                    // ACP requires unknown and already-completed IDs to be
                    // ignored; they must not become audit events either.
                    return Ok(());
                };
                lock(&elicitation_completion_ledger)?
                    .record("elicitation/complete", &descriptor);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _connection| {
                let provenance = {
                    let ledger = lock(&permission_ledger)?;
                    ledger.permission_provenance(&request)
                };
                let decision = match provenance {
                    Err(reason) => guarded_permission_decision(&request, reason),
                    Ok(provenance) => match &policy {
                        SessionPermissionPolicy::Surface => {
                            let selected = match &permission_control {
                                Some(control) => {
                                    control
                                        .offer_permission(
                                            request.session_id.0.to_string(),
                                            request.tool_call.clone(),
                                            request.options.clone(),
                                            provenance,
                                        )
                                        .await
                                }
                                None => None,
                            };
                            surface_permission_decision(&request, provenance, selected)
                        }
                        SessionPermissionPolicy::DenyAll
                        | SessionPermissionPolicy::SwemSeam { .. } => {
                            let mut decision = policy.decide(&request);
                            decision.provenance = provenance;
                            decision
                        }
                    },
                };
                let outcome = match decision.selected_option.clone() {
                    Some(option_id) => RequestPermissionOutcome::Selected(
                        SelectedPermissionOutcome::new(PermissionOptionId::new(option_id.as_str())),
                    ),
                    None => RequestPermissionOutcome::Cancelled,
                };
                {
                    let mut ledger = lock(&permission_ledger)?;
                    ledger.record(
                        "session/request_permission",
                        &serde_json::json!({
                            "request": redacted_wire_value(&request),
                            "decision": decision,
                        }),
                    );
                    ledger.permission_decisions.push(decision);
                }
                if let Some(control) = &permission_control {
                    control.publish_surface_event(
                        "host/permission_decision",
                        SurfaceEventSource::Host,
                        serde_json::json!({
                            "session_id": request.session_id.0.as_ref(),
                            "tool_call_id": request.tool_call.tool_call_id.0.as_ref(),
                            "outcome": outcome,
                        }),
                    );
                }
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CreateElicitationRequest, responder, _connection| {
                let mut descriptor = redacted_elicitation_descriptor(&NativeElicitationRequest {
                    sequence: 0,
                    request: request.clone(),
                });
                descriptor
                    .as_object_mut()
                    .expect("elicitation descriptor is an object")
                    .remove("sequence");
                let action = match &elicitation_control {
                    Some(control) => control.offer_elicitation(request).await,
                    None => ElicitationAction::Cancel,
                };
                let action_name = match &action {
                    ElicitationAction::Accept(_) => "accept",
                    ElicitationAction::Decline => "decline",
                    ElicitationAction::Cancel => "cancel",
                    ElicitationAction::Other(_) => "other",
                    _ => "unknown",
                };
                // Accepted form content and the URL itself remain in the live
                // ACP exchange. Durable evidence records only the request
                // shape, scope and terminal action.
                lock(&elicitation_ledger)?.record(
                    "elicitation/create",
                    &serde_json::json!({
                        "request": descriptor,
                        "action": action_name,
                    }),
                );
                responder.respond(CreateElicitationResponse::new(action))
            },
            agent_client_protocol::on_receive_request!(),
        )
        // ACP `fs/*` and `terminal/*` are operations in the CLIENT's
        // environment, so a connection whose client owns no such environment
        // advertises none of them. Refusing them is its own job: an isolated
        // audit (`experiments/acp-lifecycle`, `callback_dispatch`) measured
        // that the SDK leaves an unadvertised client method unanswered, and
        // an agent that gets silence stalls its whole turn until a deadline
        // instead of learning the boundary. Each refusal is a recorded host
        // decision, not an absence.
        //
        // `fs/*` has a second case. The Workbench's client is a browser tab:
        // it owns no files, so the refusal is the whole story there and stays
        // the default. An editor's client is an editor - it has exactly the
        // environment the callback names, and the file the person has open is
        // nowhere else - so when the surface says it can carry these out, they
        // go to it. What the host keeps either way is the boundary: a path
        // outside this connection's workspace is refused here, by the host,
        // and never reaches the surface. The surface does not get to name that
        // boundary, so it cannot widen it by asking.
        .on_receive_request(
            {
                let ledger = read_file_ledger;
                let control = read_file_control;
                let callbacks = read_file_callbacks;
                async move |request: ReadTextFileRequest, responder, _| {
                    const METHOD: &str = "fs/read_text_file";
                    let forwarding = callbacks
                        .as_ref()
                        .filter(|callbacks| callbacks.capabilities.read_text_file)
                        .zip(control.as_ref());
                    let Some((callbacks, lane)) = forwarding else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    if !inside_boundary(&request.path, &callbacks.boundary) {
                        record_callback_refusal(
                            &ledger,
                            control.as_ref(),
                            METHOD,
                            Some(OUTSIDE_THE_BOUNDARY),
                        )?;
                        return responder
                            .respond_with_error(outside_the_boundary_error(METHOD, &request.path));
                    }
                    let answer = lane
                        .offer_file_request(NativeFileCall {
                            session_id: request.session_id.0.to_string(),
                            method: METHOD.to_owned(),
                            path: request.path.clone(),
                            line: request.line,
                            limit: request.limit,
                            content: None,
                        })
                        .await;
                    record_file_callback(&ledger, METHOD, &request.path, answer.as_ref())?;
                    match answer {
                        Some(NativeFileAnswer::Text(content)) => {
                            responder.respond(ReadTextFileResponse::new(content))
                        }
                        other => responder
                            .respond_with_error(file_callback_error(METHOD, other.as_ref())),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let ledger = write_file_ledger;
                let control = write_file_control;
                let callbacks = write_file_callbacks;
                async move |request: WriteTextFileRequest, responder, _| {
                    const METHOD: &str = "fs/write_text_file";
                    let forwarding = callbacks
                        .as_ref()
                        .filter(|callbacks| callbacks.capabilities.write_text_file)
                        .zip(control.as_ref());
                    let Some((callbacks, lane)) = forwarding else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    if !inside_boundary(&request.path, &callbacks.boundary) {
                        record_callback_refusal(
                            &ledger,
                            control.as_ref(),
                            METHOD,
                            Some(OUTSIDE_THE_BOUNDARY),
                        )?;
                        return responder
                            .respond_with_error(outside_the_boundary_error(METHOD, &request.path));
                    }
                    let answer = lane
                        .offer_file_request(NativeFileCall {
                            session_id: request.session_id.0.to_string(),
                            method: METHOD.to_owned(),
                            path: request.path.clone(),
                            line: None,
                            limit: None,
                            content: Some(request.content.clone()),
                        })
                        .await;
                    record_file_callback(&ledger, METHOD, &request.path, answer.as_ref())?;
                    match answer {
                        Some(NativeFileAnswer::Written) => {
                            responder.respond(WriteTextFileResponse::new())
                        }
                        other => responder
                            .respond_with_error(file_callback_error(METHOD, other.as_ref())),
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        // The five `terminal/*`, each one either carried out in the profile's
        // environment or refused, exactly as this connection's
        // `terminal_callbacks` decides. Refusing is still the default, and
        // still says so on the wire: the refused method is recorded and the
        // agent hears method-not-found in its own turn rather than waiting
        // out a deadline. What changes when they are set is that the command
        // runs where the agent's work lives and shows up in the terminal
        // panel the person already has.
        .on_receive_request(
            {
                let ledger = create_terminal_ledger;
                let control = create_terminal_control;
                let callbacks = create_terminal_callbacks;
                async move |request: CreateTerminalRequest, responder, _| {
                    const METHOD: &str = "terminal/create";
                    let Some(callbacks) = callbacks.as_ref() else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    // The boundary is read before the person is: a command
                    // this rule refuses would be refused whatever they
                    // answered, and a question whose answer changes nothing
                    // teaches them the wrong thing about what a yes means.
                    if let Err(error) = crate::workbench_shell::working_directory(
                        &callbacks.profile,
                        request.cwd.as_deref(),
                    ) {
                        record_terminal_callback(
                            &ledger,
                            control.as_ref(),
                            METHOD,
                            None,
                            Some(&command_line(&request.command, &request.args)),
                            "refused",
                        )?;
                        return responder
                            .respond_with_error(terminal_callback_error(METHOD, &error));
                    }
                    // Asked before anything is started, and only where the
                    // person chose to be asked. The command line goes in the
                    // question because nothing else would let them answer it.
                    if callbacks.ask == AskBeforeRunning::EveryTime {
                        let line = command_line(&request.command, &request.args);
                        let allowed = match control.as_ref() {
                            Some(control) => {
                                control
                                    .ask_to_run(&request.session_id.0, &line)
                                    .await
                            }
                            None => false,
                        };
                        if !allowed {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                None,
                                Some(&line),
                                "not allowed",
                            )?;
                            return responder.respond_with_error(
                                agent_client_protocol::Error::invalid_params().data(
                                    serde_json::json!({
                                        "method": METHOD,
                                        "reason": NOT_ALLOWED_TO_RUN,
                                    }),
                                ),
                            );
                        }
                    }
                    let body = crate::workbench_shell::OpenTerminalBody {
                        profile_id: callbacks.profile.profile_id.clone(),
                        command: request.command.clone(),
                        args: request.args.clone(),
                        cols: 80,
                        rows: 24,
                        cwd: request.cwd.clone(),
                        env: request
                            .env
                            .iter()
                            .map(|variable| (variable.name.clone(), variable.value.clone()))
                            .collect(),
                        output_byte_limit: request.output_byte_limit,
                    };
                    match callbacks
                        .terminals
                        .open(
                            &callbacks.profile,
                            callbacks.secrets.clone(),
                            &body,
                            crate::workbench_shell::TerminalOpener::Agent,
                        )
                        .await
                    {
                        Ok(view) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&view.terminal_id),
                                Some(&command_line(&view.command, &view.args)),
                                "created",
                            )?;
                            responder.respond(CreateTerminalResponse::new(view.terminal_id))
                        }
                        Err(error) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                None,
                                Some(&command_line(&request.command, &request.args)),
                                "refused",
                            )?;
                            responder.respond_with_error(terminal_callback_error(METHOD, &error))
                        }
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let ledger = output_terminal_ledger;
                let control = output_terminal_control;
                let callbacks = output_terminal_callbacks;
                async move |request: TerminalOutputRequest, responder, _| {
                    const METHOD: &str = "terminal/output";
                    let Some(callbacks) = callbacks.as_ref() else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    let terminal_id = request.terminal_id.0.to_string();
                    match callbacks.terminals.snapshot(&terminal_id).await {
                        Ok((output, truncated, exit)) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&terminal_id),
                                None,
                                "read",
                            )?;
                            responder.respond(
                                TerminalOutputResponse::new(output, truncated)
                                    .exit_status(exit.as_ref().map(exit_status)),
                            )
                        }
                        Err(error) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&terminal_id),
                                None,
                                "refused",
                            )?;
                            responder.respond_with_error(terminal_callback_error(METHOD, &error))
                        }
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let handler_ledger = wait_terminal_ledger;
                let handler_control = wait_terminal_control;
                let handler_callbacks = wait_terminal_callbacks;
                async move |request: WaitForTerminalExitRequest, responder, connection| {
                    const METHOD: &str = "terminal/wait_for_exit";
                    // This handler is called once per request and hands its
                    // work to a task, so each call takes its own clones.
                    let ledger = Arc::clone(&handler_ledger);
                    let control = handler_control.clone();
                    let Some(callbacks) = handler_callbacks.clone() else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    // A build runs for minutes and this method is allowed to
                    // wait all of them. Waiting in the handler would stop the
                    // same connection from reading anything else the agent
                    // says, its own `terminal/kill` included, so the wait
                    // happens in a task of the connection.
                    connection.spawn(async move {
                        let terminal_id = request.terminal_id.0.to_string();
                        match callbacks.terminals.wait_for_exit(&terminal_id).await {
                            Ok(report) => {
                                record_terminal_callback(
                                    &ledger,
                                    control.as_ref(),
                                    METHOD,
                                    Some(&terminal_id),
                                    None,
                                    &report.words,
                                )?;
                                responder.respond(
                                    WaitForTerminalExitResponse::new(exit_status(&report)),
                                )
                            }
                            Err(error) => {
                                record_terminal_callback(
                                    &ledger,
                                    control.as_ref(),
                                    METHOD,
                                    Some(&terminal_id),
                                    None,
                                    "refused",
                                )?;
                                responder
                                    .respond_with_error(terminal_callback_error(METHOD, &error))
                            }
                        }
                    })?;
                    Ok(())
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let ledger = kill_terminal_ledger;
                let control = kill_terminal_control;
                let callbacks = kill_terminal_callbacks;
                async move |request: KillTerminalRequest, responder, _| {
                    const METHOD: &str = "terminal/kill";
                    let Some(callbacks) = callbacks.as_ref() else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    let terminal_id = request.terminal_id.0.to_string();
                    match callbacks.terminals.kill(&terminal_id).await {
                        Ok(()) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&terminal_id),
                                None,
                                "killed",
                            )?;
                            responder.respond(KillTerminalResponse::new())
                        }
                        Err(error) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&terminal_id),
                                None,
                                "refused",
                            )?;
                            responder.respond_with_error(terminal_callback_error(METHOD, &error))
                        }
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            {
                let ledger = release_terminal_ledger;
                let control = release_terminal_control;
                let callbacks = release_terminal_callbacks;
                async move |request: ReleaseTerminalRequest, responder, _| {
                    const METHOD: &str = "terminal/release";
                    let Some(callbacks) = callbacks.as_ref() else {
                        record_callback_refusal(&ledger, control.as_ref(), METHOD, None)?;
                        return responder.respond_with_error(unadvertised_callback_error(METHOD));
                    };
                    let terminal_id = request.terminal_id.0.to_string();
                    match callbacks.terminals.close(&terminal_id).await {
                        Ok(()) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&terminal_id),
                                None,
                                "released",
                            )?;
                            responder.respond(ReleaseTerminalResponse::new())
                        }
                        Err(error) => {
                            record_terminal_callback(
                                &ledger,
                                control.as_ref(),
                                METHOD,
                                Some(&terminal_id),
                                None,
                                "refused",
                            )?;
                            responder.respond_with_error(terminal_callback_error(METHOD, &error))
                        }
                    }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |connection| {
            let mut client_capabilities = if boolean_config_options {
                ClientCapabilities::new().session(
                    ClientSessionCapabilities::new().config_options(
                        SessionConfigOptionsCapabilities::new()
                            .boolean(BooleanConfigOptionCapabilities::new()),
                    ),
                )
            } else {
                ClientCapabilities::new()
            };
            if let Some(elicitation) = elicitation_capabilities {
                client_capabilities = client_capabilities.elicitation(elicitation);
            }
            // Only what the surface behind this connection actually said it
            // can do. An agent told the host can read files and then refused
            // every read has learned nothing except not to trust the
            // handshake.
            if let Some(files) = file_capabilities {
                client_capabilities = client_capabilities.fs(files);
            }
            // One boolean for all five, as ACP defines it: a client that
            // carries any `terminal/*` carries all of them.
            if terminal_capability {
                client_capabilities = client_capabilities.terminal(true);
            }
            // What the host claimed, recorded before it is claimed.
            //
            // Only the answer was ever recorded, and a handshake the agent
            // refuses never reaches that line: `?` carries the error out and
            // the route keeps nothing at all. So an agent that says "the
            // client did not advertise X" leaves a person reading "Could not
            // reach the agent" over an empty record, with no way to see
            // whether the host advertised X or not. The claim is the
            // one half only the host can testify to, so it is written down
            // first, and the refusal is written down too.
            let claimed = serde_json::to_value(&client_capabilities).unwrap_or(Value::Null);
            if let Some(control) = &session_control {
                control.publish_surface_event(
                    "acp/initialize_requested",
                    SurfaceEventSource::NativeLive,
                    claimed.clone(),
                );
            }
            {
                let mut ledger = lock(&main_ledger)?;
                ledger.record("initialize_requested", &claimed);
            }
            let initialize = match await_acp_operation(
                "initialize",
                operation_timeout,
                connection
                    .send_request(
                    InitializeRequest::new(ProtocolVersion::V1)
                        .client_capabilities(client_capabilities)
                        .client_info(Implementation::new("swem-host", env!("CARGO_PKG_VERSION"))),
                )
                    .block_task(),
            )
            .await
            {
                Ok(initialize) => initialize,
                Err(error) => {
                    let refusal = json!({
                        "advertised": claimed,
                        "error": error.to_string(),
                    });
                    if let Some(control) = &session_control {
                        control.publish_surface_event(
                            "acp/initialize_refused",
                            SurfaceEventSource::NativeLive,
                            refusal.clone(),
                        );
                    }
                    {
                        let mut ledger = lock(&main_ledger)?;
                        ledger.record("initialize_refused", &refusal);
                    }
                    return Err(error);
                }
            };
            if let Some(control) = &session_control {
                control.remember_prompt_capabilities(
                    &initialize.agent_capabilities.prompt_capabilities,
                );
                control.publish_surface_event(
                    "acp/initialize",
                    SurfaceEventSource::NativeLive,
                    redacted_wire_value(&initialize),
                );
            }
            {
                let mut ledger = lock(&main_ledger)?;
                ledger.agent_info = initialize
                    .agent_info
                    .as_ref()
                    .map(|info| serde_json::to_value(info).unwrap_or(Value::Null));
                ledger.record(
                    "initialize",
                    &serde_json::to_value(&initialize).unwrap_or(Value::Null),
                );
            }
            let prompt_descriptors = prompts
                .iter()
                .map(|prompt| {
                    validate_prompt_content(
                        prompt,
                        &initialize.agent_capabilities.prompt_capabilities,
                    )
                })
                .collect::<Result<Vec<_>, _>>()?;
            // ACP: when the agent advertises auth methods, the client authenticates
            // with one of them before opening a session. The person's choice
            // wins; without one, the first advertised method is used, which is
            // what an agent offering a single input-free method expects. What
            // an `env_var` method needs has already reached the process
            // environment through the profile's secrets; the interactive
            // terminal login is never run by the host.
            let chosen = match &auth_method_id {
                Some(wanted) => {
                    let found = initialize
                        .auth_methods
                        .iter()
                        .find(|method| method.id().0.as_ref() == wanted.as_str());
                    if found.is_none() {
                        return Err(agent_client_protocol::Error::invalid_params().data(
                            serde_json::json!({
                                "reason": format!(
                                    "the agent advertises no authentication method {wanted}"
                                )
                            }),
                        ));
                    }
                    found
                }
                None => initialize.auth_methods.first(),
            };
            if let Some(method) = chosen {
                let method_id = method.id().clone();
                let authenticated = await_acp_operation(
                    "authenticate",
                    operation_timeout,
                    connection
                        .send_request(AuthenticateRequest::new(method_id.clone()))
                        .block_task(),
                )
                .await?;
                if let Some(control) = &session_control {
                    control.publish_surface_event(
                        "acp/authenticate",
                        SurfaceEventSource::NativeLive,
                        serde_json::json!({ "method_id": method_id.0.as_ref() }),
                    );
                }
                let mut ledger = lock(&main_ledger)?;
                ledger.record(
                    "authenticate",
                    &serde_json::json!({
                        "method_id": method_id.0.as_ref(),
                        "response": serde_json::to_value(&authenticated).unwrap_or(Value::Null),
                    }),
                );
            }
            let reconnect_id = match &session_start {
                NativeSessionStart::New => None,
                NativeSessionStart::Load { session_id }
                | NativeSessionStart::Resume { session_id } => Some(session_id.clone()),
            };
            if let Some(reconnect_id) = reconnect_id
                && initialize
                    .agent_capabilities
                    .session_capabilities
                    .list
                    .is_some()
            {
                let mut cursor = None;
                let mut seen_cursors = BTreeSet::new();
                let mut found = false;
                loop {
                    let response = await_acp_operation(
                        "session/list",
                        operation_timeout,
                        connection
                            .send_request(
                            ListSessionsRequest::new()
                                .cwd(workspace.clone())
                                .cursor(cursor.clone()),
                        )
                            .block_task(),
                    )
                    .await?;
                    // The request is already filtered by the exact absolute
                    // `cwd`. ACP defines session/list as discovery and leaves
                    // path spelling platform-native; in particular, Windows
                    // agents may return `C:\\...` after receiving the
                    // equivalent verbatim `\\\\?\\C:\\...` path. The opaque
                    // session id is the reconciliation identity. Resume/load
                    // remains the authoritative cwd check.
                    found |= listed_session_id(&response.sessions, &reconnect_id);
                    {
                        let mut ledger = lock(&main_ledger)?;
                        ledger.record(
                            "session/list",
                            &serde_json::to_value(&response).unwrap_or(Value::Null),
                        );
                    }
                    if let Some(control) = &session_control {
                        control.publish_surface_event(
                            "acp/session_list",
                            SurfaceEventSource::NativeLive,
                            redacted_wire_value(&response),
                        );
                    }
                    let Some(next) = response.next_cursor else {
                        break;
                    };
                    if !seen_cursors.insert(next.clone()) {
                        return Err(agent_client_protocol::Error::internal_error().data(
                            serde_json::json!({
                                "message": "agent repeated an ACP session/list cursor",
                                "cursor": next,
                            }),
                        ));
                    }
                    cursor = Some(next);
                }
                if !found {
                    return Err(agent_client_protocol::Error::internal_error().data(
                        serde_json::json!({
                            "message": "reconnect target was absent from ACP session/list",
                            "session_id": reconnect_id,
                            "workspace": workspace,
                        }),
                    ));
                }
                lock(&main_ledger)?.native_session_listed = Some(true);
            }
            let session_id = match session_start {
                NativeSessionStart::New => {
                    let session = await_acp_operation(
                        "session/new",
                        operation_timeout,
                        connection
                            .send_request(
                            NewSessionRequest::new(workspace.clone()).mcp_servers(mcp_servers),
                        )
                            .block_task(),
                    )
                    .await?;
                    let session_id = session.session_id.clone();
                    replace_initial_configuration(
                        &main_ledger,
                        session.config_options.clone(),
                        session.modes.clone(),
                    )?;
                    let mut ledger = lock(&main_ledger)?;
                    ledger.record(
                        "session/new",
                        &serde_json::to_value(&session).unwrap_or(Value::Null),
                    );
                    drop(ledger);
                    if let Some(control) = &session_control {
                        control.publish_surface_event(
                            "acp/session_new",
                            SurfaceEventSource::NativeLive,
                            redacted_wire_value(&session),
                        );
                    }
                    session_id
                }
                NativeSessionStart::Load { session_id } => {
                    if !initialize.agent_capabilities.load_session {
                        return Err(agent_client_protocol::Error::internal_error().data(
                            serde_json::json!({
                                "message": "agent did not advertise ACP loadSession",
                                "session_id": session_id,
                            }),
                        ));
                    }
                    let session_id = agent_client_protocol::schema::v1::SessionId::new(session_id);
                    {
                        let mut ledger = lock(&main_ledger)?;
                        ledger.replaying = true;
                        ledger.record(
                            "session/load/request",
                            &serde_json::json!({ "session_id": session_id.0.as_ref() }),
                        );
                    }
                    let response = await_acp_operation(
                        "session/load",
                        operation_timeout,
                        connection
                            .send_request(
                            LoadSessionRequest::new(session_id.clone(), workspace.clone())
                                .mcp_servers(mcp_servers),
                        )
                            .block_task(),
                    )
                    .await?;
                    replace_initial_configuration(
                        &main_ledger,
                        response.config_options.clone(),
                        response.modes.clone(),
                    )?;
                    let mut ledger = lock(&main_ledger)?;
                    ledger.replaying = false;
                    ledger.record(
                        "session/load",
                        &serde_json::to_value(&response).unwrap_or(Value::Null),
                    );
                    drop(ledger);
                    if let Some(control) = &session_control {
                        control.publish_surface_event(
                            "acp/session_load",
                            SurfaceEventSource::NativeLive,
                            redacted_wire_value(&response),
                        );
                    }
                    session_id
                }
                NativeSessionStart::Resume { session_id } => {
                    if initialize
                        .agent_capabilities
                        .session_capabilities
                        .resume
                        .is_none()
                    {
                        return Err(agent_client_protocol::Error::internal_error().data(
                            serde_json::json!({
                                "message": "agent did not advertise ACP sessionCapabilities.resume",
                                "session_id": session_id,
                            }),
                        ));
                    }
                    let session_id = agent_client_protocol::schema::v1::SessionId::new(session_id);
                    let response = await_acp_operation(
                        "session/resume",
                        operation_timeout,
                        connection
                            .send_request(
                            ResumeSessionRequest::new(session_id.clone(), workspace.clone())
                                .mcp_servers(mcp_servers),
                        )
                            .block_task(),
                    )
                    .await?;
                    replace_initial_configuration(
                        &main_ledger,
                        response.config_options.clone(),
                        response.modes.clone(),
                    )?;
                    let mut ledger = lock(&main_ledger)?;
                    ledger.record(
                        "session/resume",
                        &serde_json::to_value(&response).unwrap_or(Value::Null),
                    );
                    drop(ledger);
                    if let Some(control) = &session_control {
                        control.publish_surface_event(
                            "acp/session_resume",
                            SurfaceEventSource::NativeLive,
                            redacted_wire_value(&response),
                        );
                    }
                    session_id
                }
            };
            lock(&main_ledger)?.session_id = Some(session_id.0.to_string());
            if let Some(control) = &session_control {
                control.set_phase(&NativeSessionPhase::Idle {
                    session_id: session_id.0.to_string(),
                });
            }
            let mut cancellation = session_control
                .as_ref()
                .map(NativeSessionControl::subscribe_cancellation);
            let supports_close = initialize
                .agent_capabilities
                .session_capabilities
                .close
                .is_some();
            let mut pending_prompts = prompts
                .into_iter()
                .zip(prompt_descriptors)
                .map(|(content, descriptors)| PendingNativePrompt {
                    content,
                    descriptors,
                    response: None,
                })
                .collect::<VecDeque<_>>();
            'connection: loop {
                let pending = if let Some(pending) = pending_prompts.pop_front() {
                    pending
                } else if interactive {
                    loop {
                        let command = tokio::select! {
                            command = async {
                                match driver_commands.as_mut() {
                                    Some(commands) => commands.recv().await,
                                    None => None,
                                }
                            } => command,
                            command = next_native_session_command(&mut session_commands) => {
                                if let Some(command) = command {
                                    apply_native_session_command(
                                        &connection,
                                        &session_id,
                                        command,
                                        &main_ledger,
                                        operation_timeout,
                                    ).await?;
                                } else {
                                    session_commands = None;
                                }
                                continue;
                            }
                        };
                        match command {
                            Some(NativeSessionDriverCommand::Prompt { content, response }) => {
                                match validate_prompt_content(
                                    &content,
                                    &initialize.agent_capabilities.prompt_capabilities,
                                ) {
                                    Ok(descriptors) => {
                                        break PendingNativePrompt {
                                            content,
                                            descriptors,
                                            response: Some(response),
                                        };
                                    }
                                    // Nothing has been sent to the agent, and
                                    // the driver stays on this loop waiting for
                                    // the next command. Saying so exactly is
                                    // what keeps a caller from ending a session
                                    // that is perfectly alive.
                                    Err(error) => {
                                        let _ = response.send(Err(SupplyError::PromptRefused(
                                            error.to_string(),
                                        )));
                                    }
                                }
                            }
                            Some(NativeSessionDriverCommand::Disconnect { response }) => {
                                lock(&main_ledger)?.termination =
                                    Some(NativeSessionTermination::Disconnected);
                                if let Some(control) = &session_control {
                                    control.publish_surface_event(
                                        "host/session_disconnect",
                                        SurfaceEventSource::Host,
                                        serde_json::json!({
                                            "session_id": session_id.0.as_ref(),
                                        }),
                                    );
                                }
                                let _ = response.send(Ok(()));
                                break 'connection;
                            }
                            Some(NativeSessionDriverCommand::Close { response }) => {
                                if !supports_close {
                                    let _ = response.send(Err(SupplyError::Protocol(
                                        "agent did not advertise ACP sessionCapabilities.close"
                                            .into(),
                                    )));
                                    continue;
                                }
                                let result = await_acp_operation(
                                    "session/close",
                                    operation_timeout,
                                    connection
                                        .send_request(CloseSessionRequest::new(session_id.clone()))
                                        .block_task(),
                                )
                                .await;
                                match result {
                                    Ok(result) => {
                                        let payload = redacted_wire_value(&result);
                                        let mut ledger = lock(&main_ledger)?;
                                        ledger.record("session/close", &payload);
                                        ledger.termination =
                                            Some(NativeSessionTermination::Closed);
                                        drop(ledger);
                                        if let Some(control) = &session_control {
                                            control.publish_surface_event(
                                                "acp/session_close",
                                                SurfaceEventSource::NativeLive,
                                                payload,
                                            );
                                        }
                                        let _ = response.send(Ok(()));
                                        break 'connection;
                                    }
                                    Err(error) => {
                                        let _ = response.send(Err(supply_error_from_acp(&error)));
                                        return Err(error);
                                    }
                                }
                            }
                            None => {
                                lock(&main_ledger)?.termination =
                                    Some(NativeSessionTermination::Disconnected);
                                break 'connection;
                            }
                        }
                    }
                } else {
                    lock(&main_ledger)?.termination =
                        Some(NativeSessionTermination::Disconnected);
                    break;
                };
                if let Some(commands) = session_commands.as_mut() {
                    loop {
                        match commands.try_recv() {
                            Ok(command) => {
                                apply_native_session_command(
                                    &connection,
                                    &session_id,
                                    command,
                                    &main_ledger,
                                    operation_timeout,
                                )
                                .await?;
                            }
                            Err(mpsc::error::TryRecvError::Empty) => break,
                            Err(mpsc::error::TryRecvError::Disconnected) => {
                                session_commands = None;
                                break;
                            }
                        }
                    }
                }
                let prompt = pending.content;
                let mut prompt_result_sender = pending.response;
                let prompt_text = content_text(&prompt);
                let turn_index = {
                    let mut ledger = lock(&main_ledger)?;
                    let turn_index = ledger.turns.len();
                    ledger.turns.push(TurnLedger {
                        prompt: prompt_text,
                        prompt_content: prompt.clone(),
                        ..TurnLedger::default()
                    });
                    ledger.active_turn = Some(turn_index);
                    ledger.record(
                        "session/prompt",
                        &serde_json::json!({ "turn": turn_index, "content": pending.descriptors }),
                    );
                    if let Some(control) = &session_control {
                        control.publish_surface_event(
                            "host/prompt_submitted",
                            SurfaceEventSource::Host,
                            serde_json::json!({
                                "session_id": session_id.0.as_ref(),
                                "turn": turn_index,
                                "content": redacted_wire_value(&prompt),
                            }),
                        );
                    }
                    turn_index
                };
                // `send_request` publishes the prompt before the control phase
                // becomes Prompting. A surface can therefore never send
                // session/cancel ahead of the request it intends to cancel.
                let response = connection
                    .send_request(PromptRequest::new(
                        session_id.clone(),
                        prompt,
                    ))
                    .block_task();
                let active_turn = NativeActiveTurn {
                    session_id: session_id.0.to_string(),
                    turn_index,
                };
                let baseline_sequence = cancellation
                    .as_mut()
                    .and_then(|receiver| receiver.borrow_and_update().clone())
                    .map_or(0, |signal| signal.sequence);
                if let Some(control) = &session_control {
                    control.set_phase(&NativeSessionPhase::Prompting {
                        session_id: active_turn.session_id.clone(),
                        turn_index,
                    });
                }
                tokio::pin!(response);
                let mut cancel_sent = false;
                let operation_deadline = tokio::time::sleep(operation_timeout);
                tokio::pin!(operation_deadline);
                let response = if let Some(receiver) = cancellation.as_mut() {
                    loop {
                        tokio::select! {
                            biased;
                            result = &mut response => break result?,
                            () = &mut operation_deadline => {
                                if let Some(sender) = prompt_result_sender.take() {
                                    let _ = sender.send(Err(SupplyError::OperationTimeout {
                                        operation: "session/prompt".into(),
                                    }));
                                }
                                return Err(operation_timeout_error("session/prompt"));
                            }
                            changed = receiver.changed(), if !cancel_sent => {
                                changed.map_err(|_| agent_client_protocol::Error::internal_error())?;
                                let signal = receiver.borrow_and_update().clone();
                                let matching = signal.as_ref().is_some_and(|signal| {
                                    signal.sequence > baseline_sequence && signal.turn == active_turn
                                });
                                if matching {
                                    connection.send_notification(CancelNotification::new(session_id.clone()))?;
                                    cancel_sent = true;
                                    let mut ledger = lock(&main_ledger)?;
                                    let preempted_tool_calls = ledger.preempt_active_tool_calls();
                                    ledger.record(
                                        "session/cancel",
                                        &serde_json::json!({
                                            "turn": turn_index,
                                            "session_id": session_id.0.as_ref(),
                                            "preempted_tool_calls": preempted_tool_calls,
                                        }),
                                    );
                                }
                            }
                            command = next_native_session_command(&mut session_commands) => {
                                if let Some(command) = command {
                                    apply_native_session_command(
                                        &connection,
                                        &session_id,
                                        command,
                                        &main_ledger,
                                        operation_timeout,
                                    ).await?;
                                } else {
                                    session_commands = None;
                                }
                            }
                        }
                    }
                } else {
                    await_acp_operation("session/prompt", operation_timeout, response).await?
                };
                let cancel_requested = session_control.as_ref().is_some_and(|control| {
                    control.finish_turn(&active_turn, baseline_sequence)
                });
                let mut ledger = lock(&main_ledger)?;
                let stop_reason = wire_name(&response.stop_reason);
                let control_outcome = if response.stop_reason == StopReason::Cancelled {
                    NativeTurnControlOutcome::Cancelled
                } else if cancel_requested || cancel_sent {
                    NativeTurnControlOutcome::CompletedBeforeCancel
                } else {
                    NativeTurnControlOutcome::Completed
                };
                ledger.record(
                    "stop",
                    &serde_json::json!({
                        "turn": turn_index,
                        "stop_reason": stop_reason,
                        "control_outcome": control_outcome,
                    }),
                );
                ledger.turns[turn_index].stop_reason = Some(stop_reason.clone());
                ledger.turns[turn_index].control_outcome = Some(control_outcome);
                ledger.active_turn = None;
                drop(ledger);
                if let Some(control) = &session_control {
                    control.publish_surface_event_with_output(
                        "acp/prompt_response",
                        SurfaceEventSource::NativeLive,
                        serde_json::json!({
                            "session_id": session_id.0.as_ref(),
                            "turn": turn_index,
                            "stop_reason": stop_reason,
                            "control_outcome": control_outcome,
                        }),
                        vec![NativeOutputProjection::TurnComplete { turn_index }],
                    );
                }
                if let Some(response) = prompt_result_sender {
                    let completed = {
                        let ledger = lock(&main_ledger)?;
                        turn_outcome(turn_index, &ledger.turns[turn_index])
                    };
                    let _ = response.send(completed);
                }
            }
            Ok(())
        });

    let result = match connection_timeout {
        Some(timeout) => tokio::time::timeout(timeout, connection)
            .await
            .map_err(|_| ()),
        None => Ok(connection.await),
    };
    let mut outcome = match result {
        Err(()) => Err(SupplyError::SessionTimeout),
        Ok(Err(error)) if error.code == ErrorCode::AuthRequired => {
            Err(SupplyError::AuthenticationRequired(error.message))
        }
        Ok(Err(error)) if timed_out_operation(&error).is_some() => {
            Err(supply_error_from_acp(&error))
        }
        Ok(Err(error)) => Err(SupplyError::Protocol(error.to_string())),
        Ok(Ok(())) => Ok(()),
    };
    let environment_terminal_evidence = match options.environment_transport.as_ref() {
        Some(transport) => match transport
            .wait_for_terminal_evidence(std::time::Duration::from_secs(5))
            .await
        {
            Ok(evidence) => Some(evidence),
            Err(error) => {
                if outcome.is_ok() {
                    outcome = Err(SupplyError::Protocol(error.to_string()));
                }
                None
            }
        },
        None => None,
    };
    let mut ledger = ledger.lock().map_err(|_| SupplyError::Poisoned)?;
    if let Some(evidence) = &environment_terminal_evidence {
        ledger.record(
            "environment/terminal",
            &serde_json::to_value(evidence).unwrap_or(Value::Null),
        );
    }
    if let Err(error) = &outcome {
        ledger.record("error", &serde_json::json!({ "error": error.to_string() }));
    }
    let assembled: Result<NativeSessionOutcome, SupplyError> = (|| {
        outcome?;
        let session_id = ledger
            .session_id
            .take()
            .ok_or(SupplyError::MissingTransactionResult)?;
        let turns = ledger
            .turns
            .iter()
            .enumerate()
            .map(|(turn_index, turn)| turn_outcome(turn_index, turn))
            .collect::<Result<Vec<_>, SupplyError>>()?;
        let wall_clock_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(NativeSessionOutcome {
            protocol_version: "1".into(),
            session_id,
            start: outcome_start,
            replayed_updates: ledger.replayed_updates,
            native_session_listed: ledger.native_session_listed,
            agent_info: ledger.agent_info.take(),
            config_options: ledger.config_options.take(),
            legacy_modes: ledger.legacy_modes.take(),
            config_option_updates: ledger.config_option_updates,
            legacy_mode_updates: ledger.legacy_mode_updates,
            turns,
            termination: ledger
                .termination
                .ok_or(SupplyError::MissingTransactionResult)?,
            permission_decisions: std::mem::take(&mut ledger.permission_decisions),
            wall_clock_ms,
            transcript_path: options.transcript_path.clone(),
            child_environment: child_environment.into(),
            environment_lease_id: options
                .environment_lease
                .as_ref()
                .map(|lease| lease.lease_id.clone()),
            environment_terminal_evidence,
        })
    })();
    drop(ledger);
    if let Some(control) = &result_control {
        control.set_phase(&NativeSessionPhase::Finished);
        let payload = match &assembled {
            Ok(outcome) => serde_json::json!({
                "status": "completed",
                "session_id": outcome.session_id,
                "turn_count": outcome.turns.len(),
                "termination": outcome.termination,
                "wall_clock_ms": outcome.wall_clock_ms,
            }),
            Err(error) => serde_json::json!({
                "status": "failed",
                "error": error.to_string(),
            }),
        };
        control.publish_surface_event("host/session_terminal", SurfaceEventSource::Host, payload);
    }
    assembled
}

/// Compatibility composition: attach SWEM Cycle to one native ACP turn.
///
/// The generic lifecycle remains usable without this server, journal, or
/// permission convention; Cycle is one MCP capability selected by the caller.
///
/// # Errors
///
/// Returns [`SupplyError`] for invalid journal/workspace paths or any native
/// session failure.
pub async fn run_agent_session(
    agent: &LaunchCommand,
    agent_executable: &Path,
    mcp_executable: &Path,
    workspace: &Path,
    prompt: &str,
    options: &AgentSessionOptions,
) -> Result<AgentSessionOutcome, SupplyError> {
    if let Some(journal) = &options.journal_dir {
        if !journal.is_absolute() {
            return Err(SupplyError::InvalidWorkspace(journal.display().to_string()));
        }
        fs::create_dir_all(journal)
            .map_err(|error| SupplyError::Protocol(format!("journal dir: {error}")))?;
    }
    let mut native = NativeSessionOptions::new(options.timeout);
    native.transcript_path.clone_from(&options.transcript_path);
    if options.inject_mcp {
        native.permission_policy = SessionPermissionPolicy::SwemSeam {
            policy: options.permission_policy.clone(),
        };
        native.mcp_servers.push(swem_mcp_server(
            mcp_executable,
            workspace,
            options.grant_local_execution,
            options.grant_publish,
            options.journal_dir.as_deref(),
            &options.plugins,
        ));
    }
    let mut outcome = run_native_session(
        agent,
        agent_executable,
        workspace,
        &[prompt.to_owned()],
        &native,
    )
    .await?;
    let turn = outcome
        .turns
        .pop()
        .ok_or(SupplyError::MissingTransactionResult)?;
    Ok(AgentSessionOutcome {
        protocol_version: outcome.protocol_version,
        session_id: outcome.session_id,
        agent_info: outcome.agent_info,
        stop_reason: turn.stop_reason,
        reply_text: turn.reply_text,
        tool_calls: turn.tool_calls,
        permission_decisions: outcome.permission_decisions,
        wall_clock_ms: outcome.wall_clock_ms,
        journal_dir: options.journal_dir.clone(),
        transcript_path: outcome.transcript_path,
    })
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::v1::{
        NewSessionResponse, PermissionOption, SessionId, ToolCallId, ToolCallUpdate,
        ToolCallUpdateFields,
    };

    use super::*;

    /// The boundary a forwarded `fs/*` call is held to.
    ///
    /// Every case here is a way out of a directory that a lexical check would
    /// have let through, or a way in that a strict one would have refused. The
    /// walks cannot reach most of them: a real editor asks for the file it has
    /// open, not for `../../etc/shadow`.
    #[test]
    fn a_file_callback_stays_inside_the_boundary() {
        let root = std::env::temp_dir().join(format!(
            "swem-boundary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_nanos())
        ));
        let workspace = root.join("workspace");
        let elsewhere = root.join("elsewhere");
        fs::create_dir_all(workspace.join("inside")).expect("make the workspace");
        fs::create_dir_all(&elsewhere).expect("make the directory outside it");
        fs::write(workspace.join("open.txt"), "a file the person has open").expect("write it");
        fs::write(elsewhere.join("theirs.txt"), "somebody else's").expect("write that too");

        assert!(inside_boundary(&workspace.join("open.txt"), &workspace));
        assert!(
            inside_boundary(&workspace.join("inside").join("new.txt"), &workspace),
            "a file a write names does not have to exist yet"
        );
        assert!(
            inside_boundary(&workspace, &workspace),
            "the directory itself"
        );
        assert!(
            !inside_boundary(&elsewhere.join("theirs.txt"), &workspace),
            "a file outside is outside"
        );
        assert!(
            !inside_boundary(&workspace.join("..").join("elsewhere"), &workspace),
            "climbing out is climbing out even when every step exists"
        );
        assert!(
            inside_boundary(
                &workspace.join("inside").join("..").join("missing.txt"),
                &workspace
            ),
            "a `..` the filesystem itself resolved is resolved: `inside` exists, so this is\
             the workspace, truthfully"
        );
        assert!(
            !inside_boundary(
                &workspace
                    .join("not-made-yet")
                    .join("..")
                    .join("..")
                    .join("elsewhere")
                    .join("theirs.txt"),
                &workspace
            ),
            "a `..` past a directory that does not exist cannot be resolved, so it is refused\
             rather than folded away"
        );
        assert!(
            !inside_boundary(Path::new("open.txt"), &workspace),
            "ACP paths are absolute; a relative one is not a path this host can place"
        );
        // A neighbour whose name begins with the workspace's. A `starts_with`
        // on the spelling would say yes; the one on components says no.
        let neighbour = root.join("workspace-of-somebody-else");
        fs::create_dir_all(&neighbour).expect("make the neighbour");
        fs::write(neighbour.join("theirs.txt"), "not ours").expect("write in it");
        assert!(!inside_boundary(&neighbour.join("theirs.txt"), &workspace));

        #[cfg(unix)]
        {
            // A link inside the workspace that points out of it. This is the
            // case canonicalizing exists for: the path is inside by every
            // reading of its text.
            let door = workspace.join("out");
            std::os::unix::fs::symlink(&elsewhere, &door).expect("make the link");
            assert!(
                !inside_boundary(&door.join("theirs.txt"), &workspace),
                "a symlink out of the workspace is a way out of the workspace"
            );
            assert!(
                !inside_boundary(&door.join("not-yet.txt"), &workspace),
                "and it is still a way out when the file at the end does not exist"
            );
        }

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn session_list_reconciliation_uses_the_opaque_id_not_path_spelling() {
        let sessions = vec![SessionInfo::new(
            SessionId::new("session-1"),
            PathBuf::from(r"C:\workspace\project"),
        )];
        assert!(listed_session_id(&sessions, "session-1"));
        assert!(!listed_session_id(&sessions, "session-2"));
    }

    #[test]
    fn transcript_redaction_is_limited_to_acp_content_payload_shapes() {
        let value = serde_json::json!({
            "tool_result": { "data": "ordinary structured value" },
            "image": { "type": "image", "mimeType": "image/png", "data": "AAE=" },
            "resource": { "uri": "swem://blob", "blob": "AgM=" },
            "text_resource": { "uri": "file:///context.txt", "text": "private context" },
        });
        let redacted = redacted_wire_value(&value);
        assert_eq!(redacted["tool_result"]["data"], "ordinary structured value");
        assert!(redacted["image"].get("data").is_none());
        assert_eq!(redacted["image"]["dataDescriptor"]["byte_length"], 2);
        assert!(redacted["resource"].get("blob").is_none());
        assert_eq!(redacted["resource"]["blobDescriptor"]["byte_length"], 2);
        assert!(redacted["text_resource"].get("text").is_none());
        assert_eq!(
            redacted["text_resource"]["textDescriptor"]["byte_length"],
            15
        );
    }

    #[test]
    fn permission_request_transcript_payload_carries_no_binary_bodies() {
        let mut fields = ToolCallUpdateFields::default();
        fields.raw_input = Some(serde_json::json!({
            "attachment": { "type": "image", "mimeType": "image/png", "data": "AAE=" },
        }));
        let request = RequestPermissionRequest::new(
            SessionId::new("session-1"),
            ToolCallUpdate::new(ToolCallId::new("call-1"), fields),
            vec![PermissionOption::new(
                PermissionOptionId::new("allow"),
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );
        let payload = redacted_wire_value(&request);
        let serialized = payload.to_string();
        assert!(
            !serialized.contains("AAE="),
            "raw_input base64 leaked into the transcript payload"
        );
        assert_eq!(
            payload["toolCall"]["rawInput"]["attachment"]["dataDescriptor"]["byte_length"],
            2
        );
    }

    #[test]
    fn official_schema_ignores_unknown_config_types_without_losing_known_order() {
        let response: NewSessionResponse = serde_json::from_value(serde_json::json!({
            "sessionId": "s",
            "configOptions": [
                {
                    "id": "model",
                    "name": "Model",
                    "category": "model",
                    "type": "select",
                    "currentValue": "fast",
                    "options": [{"value": "fast", "name": "Fast"}]
                },
                {
                    "id": "future",
                    "name": "Future",
                    "type": "slider",
                    "currentValue": 7
                },
                {
                    "id": "custom",
                    "name": "Custom",
                    "category": "_vendor_hint",
                    "type": "select",
                    "currentValue": "on",
                    "options": [{"value": "on", "name": "On"}]
                }
            ]
        }))
        .expect("official ACP tolerant config parsing");
        let options = response.config_options.expect("known config options");
        assert_eq!(
            options
                .iter()
                .map(|option| option.id.0.as_ref())
                .collect::<Vec<_>>(),
            ["model", "custom"]
        );
        validate_session_config_options(&options, false).expect("known options remain valid");
    }

    fn request(title: &str, kind: Option<ToolKind>, raw_input: Value) -> RequestPermissionRequest {
        let mut fields = ToolCallUpdateFields::default();
        fields.title = Some(title.into());
        fields.kind = kind;
        fields.raw_input = Some(raw_input);
        RequestPermissionRequest::new(
            SessionId::new("s"),
            ToolCallUpdate::new(ToolCallId::new("call-1"), fields),
            vec![
                PermissionOption::new("allow-once", "Allow", PermissionOptionKind::AllowOnce),
                PermissionOption::new("reject-once", "Reject", PermissionOptionKind::RejectOnce),
            ],
        )
    }

    #[test]
    fn seam_policy_allows_only_swem_mcp_and_read_only_tools() {
        let policy = SeamPermissionPolicy::default();
        let swem = policy.decide(&request(
            "mcp__swem__create_model_revision",
            Some(ToolKind::Other),
            serde_json::json!({}),
        ));
        assert!(swem.allowed);
        assert_eq!(swem.selected_option.as_deref(), Some("allow-once"));

        let bash = policy.decide(&request(
            "Bash",
            Some(ToolKind::Execute),
            serde_json::json!({"command": "cargo run"}),
        ));
        assert!(!bash.allowed);
        assert_eq!(bash.selected_option.as_deref(), Some("reject-once"));

        let read = policy.decide(&request(
            "Read",
            Some(ToolKind::Read),
            serde_json::json!({}),
        ));
        assert!(read.allowed);

        let edit = policy.decide(&request(
            "Write",
            Some(ToolKind::Edit),
            serde_json::json!({"file_path": "C:/ws/src/lib.rs"}),
        ));
        assert!(!edit.allowed, "no authoring workspace: edits are rejected");

        let foreign = policy.decide(&request(
            "mcp__github__create_issue",
            Some(ToolKind::Read),
            serde_json::json!({}),
        ));
        assert!(
            !foreign.allowed,
            "foreign MCP servers are rejected even when read-like"
        );

        let planted = policy.decide(&request(
            "Bash",
            Some(ToolKind::Execute),
            serde_json::json!({"command": "echo mcp__swem__create_model_revision"}),
        ));
        assert!(
            !planted.allowed,
            "an MCP-looking string in raw input is not a server identity"
        );

        let prefixed = policy.decide(&request(
            "not_mcp__swem__create_model_revision",
            Some(ToolKind::Execute),
            serde_json::json!({}),
        ));
        assert!(!prefixed.allowed, "the title prefix must be exact");
    }

    #[test]
    fn cancellation_preempts_only_unfinished_calls_of_the_active_turn() {
        let call = |id: &str, status: &str| ToolCallSummary {
            tool_call_id: id.into(),
            title: id.into(),
            kind: "other".into(),
            status: status.into(),
        };
        let mut ledger = SessionLedger {
            turns: vec![TurnLedger {
                tool_calls: vec![
                    call("pending", "pending"),
                    call("running", "in_progress"),
                    call("done", "completed"),
                    call("failed", "failed"),
                ],
                ..TurnLedger::default()
            }],
            active_turn: Some(0),
            ..SessionLedger::default()
        };

        assert_eq!(
            ledger.preempt_active_tool_calls(),
            vec!["pending".to_owned(), "running".to_owned()]
        );
        assert_eq!(ledger.turns[0].tool_calls[0].status, "cancelled");
        assert_eq!(ledger.turns[0].tool_calls[1].status, "cancelled");
        assert_eq!(ledger.turns[0].tool_calls[2].status, "completed");
        assert_eq!(ledger.turns[0].tool_calls[3].status, "failed");
    }

    #[test]
    fn authoring_workspace_admits_edits_inside_it_and_nothing_else() {
        let root = std::env::temp_dir().join(format!(
            "swem-session-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        let workspace = root.join("agent");
        fs::create_dir_all(workspace.join("src")).expect("create authoring workspace");
        let policy = SeamPermissionPolicy {
            authoring_workspace: Some(workspace.clone()),
            ..SeamPermissionPolicy::default()
        };
        let inside = policy.decide(&request(
            "Write",
            Some(ToolKind::Edit),
            serde_json::json!({"file_path": workspace.join("src/invoice.rs")}),
        ));
        assert!(inside.allowed, "{inside:?}");
        let outside = policy.decide(&request(
            "Write",
            Some(ToolKind::Edit),
            serde_json::json!({"file_path": root.join("other/x.rs")}),
        ));
        assert!(!outside.allowed);
        let escaping = policy.decide(&request(
            "Edit",
            Some(ToolKind::Edit),
            serde_json::json!({"file_path": workspace.join("../secrets.txt")}),
        ));
        assert!(!escaping.allowed);
        let unlocated = policy.decide(&request(
            "Edit",
            Some(ToolKind::Edit),
            serde_json::json!({}),
        ));
        assert!(!unlocated.allowed);
        let shell = policy.decide(&request(
            "Bash",
            Some(ToolKind::Execute),
            serde_json::json!({"command": "cargo build"}),
        ));
        assert!(!shell.allowed, "shell stays outside the seam");
        fs::remove_dir_all(root).expect("remove authoring workspace fixture");
    }

    #[cfg(unix)]
    #[test]
    fn authoring_workspace_resolves_symlink_ancestors() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("swem-session-symlink-{}", std::process::id()));
        let workspace = root.join("agent");
        let outside = root.join("outside");
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&outside).expect("create outside directory");
        symlink(&outside, workspace.join("escape")).expect("create fixture symlink");

        let policy = SeamPermissionPolicy {
            authoring_workspace: Some(workspace.clone()),
            ..SeamPermissionPolicy::default()
        };
        let decision = policy.decide(&request(
            "Write",
            Some(ToolKind::Edit),
            serde_json::json!({"file_path": workspace.join("escape/new.rs")}),
        ));
        assert!(
            !decision.allowed,
            "symlink ancestor must not escape workspace"
        );
        fs::remove_dir_all(root).expect("remove symlink fixture");
    }
}
