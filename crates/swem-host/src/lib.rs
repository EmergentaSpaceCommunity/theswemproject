//! Host-side agent supply and session boundaries.
//!
//! Discovery is intentionally read-only: locating an executable does not run it
//! and never upgrades readiness to `HandshakeReady`. Only an explicit verify
//! (one ACP `initialize`) can promote readiness, and only when the agent's
//! self-reported identity matches the catalog expectation; a binary with the
//! right name but another product behind it is `IdentityMismatch`. Install
//! execution belongs to a later explicit-consent boundary.

use std::env;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{Implementation, InitializeRequest};
use agent_client_protocol::{AcpAgent, AcpAgentConfig, Client, ConnectTo};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub mod agent_setup;
mod chat_ledger;
mod chat_work;
mod distribution;
mod editor_door;
mod envelope;
mod environment;
mod environment_profile;
mod install;
pub mod mcp_observer;
mod permission_profile;
pub mod product;
mod profile;
mod routing;
mod session;
mod surface;
pub mod workbench_apps;
mod workbench_content;
mod workbench_files;
mod workbench_observation;
pub mod workbench_shell;

pub use chat_ledger::*;
pub use chat_work::*;
pub use distribution::*;
pub use editor_door::*;
pub use envelope::{
    Fitting, Said, Speaker, Trust, Turn, envelope, fitted, fitted_within, sealed,
    standing_explanation,
};
pub use environment::*;
pub use environment_profile::*;
pub use install::*;
pub use permission_profile::*;
pub use profile::*;
pub use routing::*;
pub use session::agent_takes;
pub use session::*;
pub(crate) use surface::NativeOutputProjection;
pub use surface::{NativeSessionEvent, SurfaceEventSource};
pub use workbench_apps::{
    AppAttachmentView, DiscoveredAppResource, DiscoveredAppTool, MCP_APP_MIME, OpenedApp,
    RelayRefusal,
};
pub use workbench_content::{
    WorkbenchContentDescriptor, WorkbenchContentLifecycle, WorkbenchContentSource,
};
#[doc(hidden)]
pub use workbench_files::hand_over as hand_over_for_tests;
#[doc(hidden)]
pub use workbench_files::list as list_handed_files_for_tests;
#[doc(hidden)]
pub use workbench_files::read as read_handed_file_for_tests;
pub use workbench_files::{HandedFile, INBOX, OUTBOX};
pub use workbench_observation::ObservedAppCall;
pub use workbench_shell::{
    ACP_REGISTRY_CACHE, AddIndexBody, AmendProfileBody, ArchiveDistribution, BinaryDistribution,
    BindModelContextBody, CATALOG_SCHEMA, Catalog, CatalogDistribution, CatalogEntry, ChatPage,
    DeclareMcpServerBody, INDEX_SCHEMA, IndexFile, InstalledSkill, McpServerOrigin, McpServerView,
    ModelContext, ModelContextBlock, NamedValue, NpxDistribution, ObservedAppOpen,
    OpenTerminalBody, RegistryStatus, ResolvedAgentConnection, ResolvedAgentEnvironment,
    ResolvedDirectAgentConnection, SCHEDULE_SURFACE, SaidInChat, Saying, Schedule, ScheduleBook,
    SetScheduleBody, ShellConnectionMode, StartChatBody, StoreEntry, StoreIndexView,
    StoreInstallBody, StorePlanBody, StoreView, TerminalInputBody, TerminalOutput,
    TerminalSizeBody, TerminalView, WorkbenchAgentOption, WorkbenchOnboarding, WorkbenchShellError,
    WorkbenchShellHandle, WorkbenchShellState, credential_environment, mint_session_token,
    serve_workbench_http, serve_workbench_http_with_apps, serve_workbench_http_with_apps_at,
};

pub const ACP_REGISTRY_INDEX: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";

/// The environment variable that names a different ACP registry index.
pub const ACP_REGISTRY_INDEX_VAR: &str = "SWEM_ACP_REGISTRY_INDEX";

/// The ACP registry index a product reads unless its builder named one: the
/// public CDN, or whatever [`ACP_REGISTRY_INDEX_VAR`] names.
///
/// The index was a constant, which meant a person whose machine cannot reach
/// that one host could not install an agent at all - behind a corporate proxy,
/// on a disconnected network, or reading an internal mirror. Naming the index
/// changes nothing else about installing: the plan is still resolved from the
/// index, shown in full, confirmed by the person, applied against the exact
/// `plan_id` they saw, and receipted, and a binary distribution still has to
/// match the sha256 the index gives. What it does change is that the trust in
/// the index is now the person's to place, which is the same bargain as naming
/// a package mirror to any other package manager.
#[must_use]
pub fn acp_registry_index() -> String {
    std::env::var(ACP_REGISTRY_INDEX_VAR)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| ACP_REGISTRY_INDEX.to_owned())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    InteractiveSession,
    DelegatedMcpCapability,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationKind {
    DirectAcp,
    AcpAdapter,
    NativeAdapterRequired,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    Absent,
    InstalledUnverified,
    AdapterRequired,
    /// The launched binary answered `initialize` as a different product than
    /// the catalog entry names. Never ready: the launch would run the wrong agent.
    IdentityMismatch,
    HandshakeReady,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LaunchCommand {
    pub executable: String,
    pub args: Vec<String>,
    pub integration: IntegrationKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentCatalogEntry {
    pub id: String,
    pub name: String,
    pub registry_id: Option<String>,
    pub roles: Vec<AgentRole>,
    pub probes: Vec<LaunchCommand>,
    /// `agentInfo.name` the product reports in `initialize`; `None` means no
    /// live evidence exists yet and verify cannot claim identity.
    pub expected_agent_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDiscovery {
    pub id: String,
    pub name: String,
    pub readiness: Readiness,
    pub executable_path: Option<PathBuf>,
    pub launch: Option<LaunchCommand>,
    pub registry_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstallPlan {
    pub plan_id: String,
    /// What the plan installs; an agent unless the plan says otherwise.
    #[serde(default)]
    pub kind: InstallKind,
    pub agent_id: String,
    pub registry_id: String,
    pub registry_index: String,
    pub name: String,
    pub version: String,
    pub distribution: RegistryDistribution,
    pub requires_explicit_consent: bool,
    pub executes_remote_shell: bool,
}

/// Typed result of one ACP v1 `initialize` against a discovered agent.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AcpHandshake {
    pub protocol_version: String,
    pub agent_name: Option<String>,
    pub agent_version: Option<String>,
    pub auth_method_ids: Vec<String>,
    /// The advertised methods as the agent sent them (`{type, id, name,
    /// description, vars?, args?, link?}`), so a surface can ask a person for
    /// exactly what a method needs rather than only name it.
    #[serde(default)]
    pub auth_methods: Vec<Value>,
    pub agent_capabilities: Value,
    pub expected_agent_name: Option<String>,
    /// `Some(true)` only when the catalog expectation exists and matches.
    pub identity_matched: Option<bool>,
    pub readiness: Readiness,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRevisionIdentity {
    pub schema: String,
    pub digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelRevisionTransaction {
    pub protocol_version: String,
    pub session_id: String,
    pub record: ModelRevisionIdentity,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AgentJsonTransaction {
    pub protocol_version: String,
    pub session_id: String,
    pub response: Value,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum SupplyError {
    #[error("unknown agent: {0}")]
    UnknownAgent(String),
    #[error("agent has no registry-managed distribution: {0}")]
    UnmanagedAgent(String),
    #[error("agent is not launchable through ACP: {0}")]
    NotAcpLaunchable(String),
    #[error("agent executable was not discovered: {0}")]
    MissingExecutable(String),
    #[error("ACP handshake timed out")]
    HandshakeTimeout,
    #[error("ACP session timed out")]
    SessionTimeout,
    #[error("ACP operation `{operation}` timed out")]
    OperationTimeout { operation: String },
    #[error("ACP protocol error: {0}")]
    Protocol(String),
    /// The content of a turn was refused before anything was sent to the
    /// agent - it asked for a capability the agent never advertised. The
    /// session is untouched and stays usable; only this turn is rejected.
    /// Distinct from [`Self::Protocol`] because a caller must not tear a live
    /// connection down over content it can simply send differently.
    #[error("the agent cannot take this turn's content: {0}")]
    PromptRefused(String),
    #[error("ACP initialize response could not be serialized: {0}")]
    Serialization(String),
    #[error("ACP handshake result lock was poisoned")]
    Poisoned,
    #[error("ACP handshake completed without an initialize response")]
    MissingInitializeResponse,
    #[error("workspace path must be an existing absolute directory: {0}")]
    InvalidWorkspace(String),
    #[error("ACP/MCP transaction completed without a result")]
    MissingTransactionResult,
    #[error("agent requires authentication before a session can start: {0}")]
    AuthenticationRequired(String),
    #[error("installation requires explicit consent: {0}")]
    ConsentRequired(String),
    #[error("registry entry has no supported typed distribution: {0}")]
    UnsupportedDistribution(String),
}

#[must_use]
pub fn builtin_catalog() -> Vec<AgentCatalogEntry> {
    vec![
        AgentCatalogEntry {
            id: "codex".into(),
            name: "Codex".into(),
            registry_id: Some("codex-acp".into()),
            roles: vec![
                AgentRole::InteractiveSession,
                AgentRole::DelegatedMcpCapability,
            ],
            probes: vec![
                LaunchCommand {
                    executable: "codex-acp".into(),
                    args: vec![],
                    integration: IntegrationKind::AcpAdapter,
                },
                LaunchCommand {
                    executable: "codex".into(),
                    args: vec![],
                    integration: IntegrationKind::NativeAdapterRequired,
                },
            ],
            expected_agent_name: None,
        },
        AgentCatalogEntry {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            registry_id: Some("claude-acp".into()),
            roles: vec![AgentRole::InteractiveSession],
            probes: vec![
                LaunchCommand {
                    executable: "claude-agent-acp".into(),
                    args: vec![],
                    integration: IntegrationKind::AcpAdapter,
                },
                LaunchCommand {
                    executable: "claude".into(),
                    args: vec![],
                    integration: IntegrationKind::NativeAdapterRequired,
                },
            ],
            // Observed from the registry distribution 0.70.0 `initialize`.
            expected_agent_name: Some("@agentclientprotocol/claude-agent-acp".into()),
        },
        AgentCatalogEntry {
            id: "opencode".into(),
            name: "OpenCode".into(),
            registry_id: Some("opencode".into()),
            roles: vec![AgentRole::InteractiveSession],
            probes: vec![LaunchCommand {
                executable: "opencode".into(),
                args: vec!["acp".into()],
                integration: IntegrationKind::DirectAcp,
            }],
            // Observed from official ACP Registry binary 1.18.25 `initialize`.
            expected_agent_name: Some("OpenCode".into()),
        },
        AgentCatalogEntry {
            id: "gemini".into(),
            name: "Gemini CLI".into(),
            registry_id: Some("gemini".into()),
            roles: vec![AgentRole::InteractiveSession],
            probes: vec![LaunchCommand {
                executable: "gemini".into(),
                args: vec!["--acp".into()],
                integration: IntegrationKind::DirectAcp,
            }],
            expected_agent_name: None,
        },
        // Two products share the `kimi` binary name and the `acp` subcommand.
        // The registry entry `kimi` is Kimi CLI (MoonshotAI/kimi-cli); the
        // separately distributed Kimi Code CLI (moonshotai/kimi-code) is not
        // registry-managed. Discovery cannot tell them apart; verify must.
        AgentCatalogEntry {
            id: "kimi-code".into(),
            name: "Kimi Code CLI".into(),
            registry_id: None,
            roles: vec![AgentRole::InteractiveSession],
            probes: vec![LaunchCommand {
                executable: "kimi".into(),
                args: vec!["acp".into()],
                integration: IntegrationKind::DirectAcp,
            }],
            expected_agent_name: Some("Kimi Code CLI".into()),
        },
        AgentCatalogEntry {
            id: "kimi".into(),
            name: "Kimi CLI".into(),
            registry_id: Some("kimi".into()),
            roles: vec![AgentRole::InteractiveSession],
            probes: vec![LaunchCommand {
                executable: "kimi".into(),
                args: vec!["acp".into()],
                integration: IntegrationKind::DirectAcp,
            }],
            expected_agent_name: Some("Kimi CLI".into()),
        },
    ]
}

/// The built-in catalogue found on PATH alone: what a machine has without
/// this product having installed anything. The product's own discovery,
/// which also reads the install root and the agents a person declared, is
/// [`discover_agents_with`].
#[must_use]
pub fn discover_agents() -> Vec<AgentDiscovery> {
    discover_with(&builtin_catalog(), find_on_path)
}

/// Discovery over the built-in catalogue plus the agents a person declared
/// themselves.
///
/// The built-in list is what this product knows about; it is not what exists.
/// An ACP agent a person has and this build has never heard of was simply
/// unusable, which is the wrong answer for a product whose whole subject is
/// working with an agent.
#[must_use]
pub fn discover_agents_with(
    declared: Vec<AgentCatalogEntry>,
    installed_root: &Path,
) -> Vec<AgentDiscovery> {
    let installations = load_receipts(installed_root, InstallKind::Agent);
    let mut catalog = builtin_catalog();
    for entry in declared {
        // A declaration replaces a built-in entry of the same id: the person
        // saying where their agent is beats the product's guess.
        catalog.retain(|known| known.id != entry.id);
        catalog.push(entry);
    }
    discover_with_installations(&catalog, find_on_path, &installations)
}

/// One agent a person declared, as the document they wrote it in.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeclaredAgent {
    /// The id this agent is known by here (letters, digits, `-`, `_`).
    pub id: String,
    /// What a person calls it.
    pub name: String,
    /// The command that starts it, speaking ACP over stdio.
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

impl DeclaredAgent {
    /// The catalogue entry this declaration is, or the reason it is not one.
    ///
    /// # Errors
    ///
    /// Returns a sentence for an unusable id or an empty command.
    pub fn into_entry(self) -> Result<AgentCatalogEntry, String> {
        if self.id.is_empty()
            || self.id.len() > 64
            || !self
                .id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(format!(
                "{:?} is not an agent id: 1-64 letters, digits, '-' and '_'",
                self.id
            ));
        }
        if self.command.trim().is_empty() {
            return Err(format!("agent {} declares no command to start", self.id));
        }
        Ok(AgentCatalogEntry {
            name: if self.name.trim().is_empty() {
                self.id.clone()
            } else {
                self.name
            },
            id: self.id,
            registry_id: None,
            roles: vec![AgentRole::InteractiveSession],
            probes: vec![LaunchCommand {
                executable: self.command.trim().to_owned(),
                args: self.args,
                integration: IntegrationKind::DirectAcp,
            }],
            expected_agent_name: None,
        })
    }
}

/// Every agent declared in `root` (`<root>/*.json`), in id order.
///
/// # Errors
///
/// Fails closed on an unreadable directory or a malformed declaration: an
/// agent a person declared and this product silently dropped is worse than a
/// refusal that names the file.
pub fn declared_agents(root: &Path) -> Result<Vec<AgentCatalogEntry>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json") {
            continue;
        }
        let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let declared: DeclaredAgent = serde_json::from_slice(&bytes)
            .map_err(|error| format!("{} is not an agent declaration: {error}", path.display()))?;
        found.push(declared.into_entry()?);
    }
    found.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(found)
}

pub fn discover_with<F>(catalog: &[AgentCatalogEntry], locate: F) -> Vec<AgentDiscovery>
where
    F: FnMut(&str) -> Option<PathBuf>,
{
    discover_with_installations(catalog, locate, &std::collections::BTreeMap::new())
}

/// Discover catalog entries against `locate` (PATH-style lookup) and the
/// host-owned registry installations. A registry installation wins over a
/// PATH binary because its version is exact; readiness still stops at
/// `InstalledUnverified` until `verify` runs.
pub fn discover_with_installations<F>(
    catalog: &[AgentCatalogEntry],
    mut locate: F,
    installations: &std::collections::BTreeMap<String, InstallReceipt>,
) -> Vec<AgentDiscovery>
where
    F: FnMut(&str) -> Option<PathBuf>,
{
    let mut discovered: Vec<AgentDiscovery> = catalog
        .iter()
        .map(|entry| {
            let installed = entry
                .registry_id
                .as_deref()
                .and_then(|id| installations.get(id))
                .and_then(|installation| {
                    let integration = entry
                        .probes
                        .iter()
                        .map(|probe| probe.integration)
                        .find(|integration| *integration != IntegrationKind::NativeAdapterRequired)
                        .unwrap_or(IntegrationKind::AcpAdapter);
                    if let Some(entry_script) = &installation.entry_script {
                        let node = locate("node")?;
                        let mut args = vec![entry_script.display().to_string()];
                        args.extend(installation.args.iter().cloned());
                        Some((
                            LaunchCommand {
                                executable: "node".into(),
                                args,
                                integration,
                            },
                            node,
                        ))
                    } else {
                        let executable = installation.executable.as_ref()?.clone();
                        Some((
                            LaunchCommand {
                                executable: executable.display().to_string(),
                                args: installation.args.clone(),
                                integration,
                            },
                            executable,
                        ))
                    }
                });
            let found = installed.or_else(|| {
                entry
                    .probes
                    .iter()
                    .find_map(|probe| locate(&probe.executable).map(|path| (probe.clone(), path)))
            });
            match found {
                Some((launch, path)) => AgentDiscovery {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    readiness: if launch.integration == IntegrationKind::NativeAdapterRequired {
                        Readiness::AdapterRequired
                    } else {
                        Readiness::InstalledUnverified
                    },
                    executable_path: Some(path),
                    launch: Some(launch),
                    registry_id: entry.registry_id.clone(),
                },
                None => AgentDiscovery {
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    readiness: Readiness::Absent,
                    executable_path: None,
                    launch: None,
                    registry_id: entry.registry_id.clone(),
                },
            }
        })
        .collect();
    // An agent installed from the registry that no catalogue entry names -
    // the registry lists many more than this build knows - is still an agent
    // this machine has.
    for (registry_id, receipt) in installations {
        if discovered
            .iter()
            .any(|agent| agent.registry_id.as_deref() == Some(registry_id))
        {
            continue;
        }
        if let Some(agent) = discovery_of_receipt(registry_id, receipt, &mut locate) {
            discovered.push(agent);
        }
    }
    discovered
}

/// An installed agent known only by its receipt: launched from the receipt,
/// named by the registry; the registry's own distribution speaks ACP
/// directly. None when its launch path needs a node this machine lacks.
fn discovery_of_receipt<F>(
    registry_id: &str,
    receipt: &InstallReceipt,
    locate: &mut F,
) -> Option<AgentDiscovery>
where
    F: FnMut(&str) -> Option<PathBuf>,
{
    let (launch, path) = if let Some(entry_script) = &receipt.entry_script {
        let node = locate("node")?;
        let mut args = vec![entry_script.display().to_string()];
        args.extend(receipt.args.iter().cloned());
        (
            LaunchCommand {
                executable: "node".into(),
                args,
                integration: IntegrationKind::DirectAcp,
            },
            node,
        )
    } else {
        let executable = receipt.executable.clone()?;
        (
            LaunchCommand {
                executable: executable.display().to_string(),
                args: receipt.args.clone(),
                integration: IntegrationKind::DirectAcp,
            },
            executable,
        )
    };
    Some(AgentDiscovery {
        id: registry_id.to_owned(),
        name: receipt.name.clone(),
        readiness: Readiness::InstalledUnverified,
        executable_path: Some(path),
        launch: Some(launch),
        registry_id: Some(registry_id.to_owned()),
    })
}

/// Resolve a registry-backed install plan without downloading or executing it.
///
/// `agent_id` is one this build's catalogue knows, which names its registry
/// entry; or, when the catalogue does not know it, the registry's own id -
/// the registry lists many more agents than the catalogue describes, and a
/// person may install any of them. Such an agent is then known here by that
/// registry id.
///
/// # Errors
///
/// Returns [`SupplyError::UnmanagedAgent`] if a catalogue entry has no
/// registry distribution, or the registry's own refusal when the id is
/// absent from it.
pub fn install_plan(agent_id: &str) -> Result<InstallPlan, SupplyError> {
    install_plan_at(agent_id, &acp_registry_index())
}

/// The plan installing `agent_id` from the registry at `registry_index`.
///
/// # Errors
///
/// The agent is not in the catalogue or the registry, has no managed
/// distribution, or the registry cannot be read.
pub fn install_plan_at(agent_id: &str, registry_index: &str) -> Result<InstallPlan, SupplyError> {
    let registry_id = match builtin_catalog()
        .into_iter()
        .find(|entry| entry.id == agent_id)
    {
        Some(entry) => entry
            .registry_id
            .ok_or_else(|| SupplyError::UnmanagedAgent(agent_id.into()))?,
        None => agent_id.to_owned(),
    };
    resolve_registry_install_plan(agent_id, &registry_id, registry_index)
}

/// Launch an explicitly selected discovered agent and verify the stable ACP v1
/// initialize handshake. This function executes the discovered binary; callers
/// must obtain user consent before invoking it.
///
/// # Errors
///
/// Returns [`SupplyError`] when the discovery is not ACP-launchable, process
/// launch/protocol negotiation fails, or the timeout expires.
pub async fn verify_discovered_agent(
    discovery: &AgentDiscovery,
    timeout: Duration,
) -> Result<AcpHandshake, SupplyError> {
    let launch = discovery
        .launch
        .as_ref()
        .ok_or_else(|| SupplyError::MissingExecutable(discovery.id.clone()))?;
    if launch.integration == IntegrationKind::NativeAdapterRequired {
        return Err(SupplyError::NotAcpLaunchable(discovery.id.clone()));
    }
    let executable = discovery
        .executable_path
        .as_ref()
        .ok_or_else(|| SupplyError::MissingExecutable(discovery.id.clone()))?;
    let expected_agent_name = builtin_catalog()
        .into_iter()
        .find(|entry| entry.id == discovery.id)
        .and_then(|entry| entry.expected_agent_name);
    let config = AcpAgentConfig::new(executable).args(launch.args.clone());
    verify_transport(AcpAgent::new(config), timeout, expected_agent_name).await
}

/// Run the smallest complete external-agent transaction: initialize stable ACP
/// v1, bind an absolute workspace, inject a standalone SWEM MCP stdio server,
/// and ask the agent to create a validated immutable `ModelRevision` through it.
///
/// Both executables are launched. Callers must bind this operation to explicit
/// user intent and an allowed workspace before invoking it.
///
/// # Errors
///
/// Returns [`SupplyError`] for invalid workspace binding, protocol/process
/// failure, timeout, or a malformed transaction result.
pub async fn run_model_revision_transaction(
    agent_executable: &Path,
    mcp_executable: &Path,
    workspace: &Path,
    revision: &impl Serialize,
    timeout: Duration,
) -> Result<ModelRevisionTransaction, SupplyError> {
    // The revision is the kernel's record; the host carries it as a value
    // and does not know its type. Whoever calls this validated it.
    let prompt = serde_json::to_string(revision)
        .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    // No packages: a model revision is the kernel's own record, and this
    // transaction asks for nothing a domain owns.
    let transaction = run_agent_json_transaction(
        agent_executable,
        mcp_executable,
        workspace,
        &prompt,
        timeout,
        &[],
    )
    .await?;
    let record = serde_json::from_value(transaction.response)
        .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    Ok(ModelRevisionTransaction {
        protocol_version: transaction.protocol_version,
        session_id: transaction.session_id,
        record,
    })
}

/// Run a workspace-bound native ACP turn with the SWEM MCP server injected and
/// require the agent's complete response to be one JSON value.
///
/// This is the protocol-neutral host seam used by vertical transaction tests;
/// it does not prescribe an agent loop or a SWEM domain workflow.
///
/// # Errors
///
/// Returns [`SupplyError`] for an invalid workspace, protocol/process failure,
/// timeout, missing result, or a non-JSON agent response.
pub async fn run_agent_json_transaction(
    agent_executable: &Path,
    mcp_executable: &Path,
    workspace: &Path,
    prompt: &str,
    timeout: Duration,
    plugins: &[PathBuf],
) -> Result<AgentJsonTransaction, SupplyError> {
    run_agent_json_transaction_with_mcp_policy(
        agent_executable,
        mcp_executable,
        workspace,
        prompt,
        timeout,
        false,
        plugins,
    )
    .await
}

/// Run the same ACP/MCP transaction while explicitly granting the experimental
/// unsandboxed local Cargo capability to the child MCP server.
///
/// The capability remains absent from ordinary sessions. Callers must obtain
/// authority for local code execution and bind an isolated workspace before
/// invoking this function.
///
/// # Errors
///
/// Returns [`SupplyError`] under the same conditions as
/// [`run_agent_json_transaction`].
pub async fn run_agent_json_transaction_with_local_execution(
    agent_executable: &Path,
    mcp_executable: &Path,
    workspace: &Path,
    prompt: &str,
    timeout: Duration,
    plugins: &[PathBuf],
) -> Result<AgentJsonTransaction, SupplyError> {
    run_agent_json_transaction_with_mcp_policy(
        agent_executable,
        mcp_executable,
        workspace,
        prompt,
        timeout,
        true,
        plugins,
    )
    .await
}

async fn run_agent_json_transaction_with_mcp_policy(
    agent_executable: &Path,
    mcp_executable: &Path,
    workspace: &Path,
    prompt: &str,
    timeout: Duration,
    grant_local_execution: bool,
    plugins: &[PathBuf],
) -> Result<AgentJsonTransaction, SupplyError> {
    let mut options = AgentSessionOptions::new(timeout);
    // This helper IS the Cycle composition: it opts in to the seam.
    options.inject_mcp = true;
    options.grant_local_execution = grant_local_execution;
    // And it names the packages that Cycle loads: a domain arrives as a
    // package, so a transaction that names none speaks no domain.
    options.plugins = plugins.to_vec();
    run_agent_json_transaction_with_options(
        agent_executable,
        mcp_executable,
        workspace,
        prompt,
        &options,
    )
    .await
}

/// Run one JSON transaction with explicit session options (grants, journal,
/// transcript): the entry used when a transaction must resume over a journal
/// written by an earlier one.
///
/// # Errors
///
/// Returns [`SupplyError`] under the same conditions as
/// [`run_agent_json_transaction`].
pub async fn run_agent_json_transaction_with_options(
    agent_executable: &Path,
    mcp_executable: &Path,
    workspace: &Path,
    prompt: &str,
    options: &AgentSessionOptions,
) -> Result<AgentJsonTransaction, SupplyError> {
    let launch = LaunchCommand {
        executable: agent_executable.display().to_string(),
        args: vec![],
        integration: IntegrationKind::DirectAcp,
    };
    let outcome = run_agent_session(
        &launch,
        agent_executable,
        mcp_executable,
        workspace,
        prompt,
        options,
    )
    .await?;
    let response = serde_json::from_str(&outcome.reply_text)
        .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    Ok(AgentJsonTransaction {
        protocol_version: outcome.protocol_version,
        session_id: outcome.session_id,
        response,
    })
}

/// Initialize against a resolved launch and report what the agent advertised,
/// authentication methods included. This is what a surface calls before it
/// offers to start a session: the person sees what the agent will ask for
/// instead of finding out from a failed connection.
///
/// # Errors
///
/// Returns [`SupplyError`] when the launch is not ACP-launchable, the process
/// or handshake fails, or the timeout expires.
pub async fn verify_launch(
    launch: &LaunchCommand,
    agent_executable: &Path,
    timeout: Duration,
    expected_agent_name: Option<String>,
) -> Result<AcpHandshake, SupplyError> {
    if launch.integration == IntegrationKind::NativeAdapterRequired {
        return Err(SupplyError::NotAcpLaunchable(launch.executable.clone()));
    }
    let config = AcpAgentConfig::new(agent_executable).args(launch.args.clone());
    verify_transport(AcpAgent::new(config), timeout, expected_agent_name).await
}

async fn verify_transport(
    transport: impl ConnectTo<Client> + 'static,
    timeout: Duration,
    expected_agent_name: Option<String>,
) -> Result<AcpHandshake, SupplyError> {
    let response_slot = Arc::new(Mutex::new(None));
    let writer = Arc::clone(&response_slot);
    let connection =
        Client
            .builder()
            .name("swem-host")
            .connect_with(transport, async move |connection| {
                let request = InitializeRequest::new(ProtocolVersion::V1)
                    .client_info(Implementation::new("swem-host", env!("CARGO_PKG_VERSION")));
                let response = connection.send_request(request).block_task().await?;
                *writer
                    .lock()
                    .map_err(|_| agent_client_protocol::Error::internal_error())? = Some(response);
                Ok(())
            });

    tokio::time::timeout(timeout, connection)
        .await
        .map_err(|_| SupplyError::HandshakeTimeout)?
        .map_err(|error| SupplyError::Protocol(error.to_string()))?;

    let response = response_slot
        .lock()
        .map_err(|_| SupplyError::Poisoned)?
        .take()
        .ok_or(SupplyError::MissingInitializeResponse)?;
    let agent_name = response.agent_info.as_ref().map(|info| info.name.clone());
    let agent_version = response
        .agent_info
        .as_ref()
        .map(|info| info.version.clone());
    let auth_methods: Vec<Value> = response
        .auth_methods
        .iter()
        .map(|method| serde_json::to_value(method).unwrap_or(Value::Null))
        .collect();
    let auth_method_ids = auth_methods
        .iter()
        .map(|value| {
            value
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_default()
        })
        .collect();
    let agent_capabilities = serde_json::to_value(&response.agent_capabilities)
        .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    let identity_matched = expected_agent_name
        .as_deref()
        .map(|expected| agent_name.as_deref() == Some(expected));
    let readiness = match identity_matched {
        Some(false) => Readiness::IdentityMismatch,
        Some(true) | None => Readiness::HandshakeReady,
    };
    Ok(AcpHandshake {
        protocol_version: "1".into(),
        agent_name,
        agent_version,
        auth_method_ids,
        auth_methods,
        agent_capabilities,
        expected_agent_name,
        identity_matched,
        readiness,
    })
}

fn find_on_path(executable: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    let candidates = executable_candidates(executable);
    env::split_paths(&path)
        .flat_map(|directory| candidates.iter().map(move |name| directory.join(name)))
        .find(|candidate| candidate.is_file())
}

fn executable_candidates(executable: &str) -> Vec<String> {
    if Path::new(executable).extension().is_some() {
        return vec![executable.into()];
    }
    if cfg!(windows) {
        let extensions = env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        let mut names = vec![executable.into()];
        names.extend(
            extensions
                .into_iter()
                .map(|extension| format!("{executable}{extension}")),
        );
        names
    } else {
        vec![executable.into()]
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn direct_acp_probe_wins_over_native_binary() {
        let paths = HashMap::from([
            ("codex-acp", PathBuf::from("/managed/codex-acp")),
            ("codex", PathBuf::from("/usr/bin/codex")),
        ]);
        let discovered = discover_with(&builtin_catalog(), |name| paths.get(name).cloned());
        let codex = discovered.iter().find(|agent| agent.id == "codex").unwrap();
        assert_eq!(codex.readiness, Readiness::InstalledUnverified);
        assert_eq!(
            codex.launch.as_ref().unwrap().integration,
            IntegrationKind::AcpAdapter
        );
    }

    #[test]
    fn native_binary_does_not_claim_acp_readiness() {
        let discovered = discover_with(&builtin_catalog(), |name| {
            (name == "claude").then(|| PathBuf::from("/usr/bin/claude"))
        });
        let claude = discovered
            .iter()
            .find(|agent| agent.id == "claude-code")
            .unwrap();
        assert_eq!(claude.readiness, Readiness::AdapterRequired);
    }

    #[test]
    fn registry_native_install_launches_without_node() {
        let executable = std::env::current_exe().unwrap();
        let installation = InstallReceipt {
            schema: INSTALL_RECEIPT_SCHEMA.into(),
            kind: InstallKind::Agent,
            plan_id: String::new(),
            source: String::new(),
            registry_id: "opencode".into(),
            name: "OpenCode".into(),
            version: "1.2.3".into(),
            platform: Some("windows-x86_64".into()),
            package: None,
            archive: Some("https://example.invalid/opencode.zip".into()),
            sha256: Some("a".repeat(64)),
            command: Some("./opencode.exe".into()),
            entry_script: None,
            executable: Some(executable.clone()),
            file: None,
            args: vec!["acp".into()],
            discovery_method: "registry-binary-sha256-exact-entry".into(),
            installed_at: 0,
        };
        let installations = std::collections::BTreeMap::from([("opencode".into(), installation)]);
        let discovered = discover_with_installations(&builtin_catalog(), |_| None, &installations);
        let opencode = discovered
            .iter()
            .find(|agent| agent.id == "opencode")
            .unwrap();
        assert_eq!(opencode.executable_path.as_ref(), Some(&executable));
        assert_eq!(opencode.launch.as_ref().unwrap().args, ["acp"]);
        assert_eq!(
            opencode.launch.as_ref().unwrap().integration,
            IntegrationKind::DirectAcp
        );
    }

    #[test]
    fn install_plan_is_registry_backed_and_never_remote_shell() {
        let plan = resolve_registry_install_plan_from_bytes(
            "opencode",
            "opencode",
            ACP_REGISTRY_INDEX,
            br#"{"agents":[{"id":"opencode","name":"OpenCode","version":"1.2.3","distribution":{"binary":{"windows-x86_64":{"archive":"https://example.invalid/opencode.zip","cmd":"./opencode.exe","args":["acp"],"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}}]}"#,
            "windows-x86_64",
        )
        .unwrap();
        assert_eq!(plan.registry_id, "opencode");
        assert_eq!(plan.version, "1.2.3");
        assert!(matches!(
            plan.distribution,
            RegistryDistribution::Binary { .. }
        ));
        assert!(plan.requires_explicit_consent);
        assert!(!plan.executes_remote_shell);
    }

    #[tokio::test]
    async fn official_sdk_handshake_promotes_only_after_initialize() {
        use agent_client_protocol::Agent;
        use agent_client_protocol::schema::v1::{
            AgentCapabilities, InitializeRequest, InitializeResponse,
        };

        let agent = Agent.builder().name("fixture-agent").on_receive_request(
            async |initialize: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(initialize.protocol_version)
                        .agent_info(Implementation::new("fixture-agent", "1.0.0"))
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        );

        let handshake = verify_transport(agent, Duration::from_secs(2), None)
            .await
            .unwrap();
        assert_eq!(handshake.protocol_version, "1");
        assert_eq!(handshake.agent_name.as_deref(), Some("fixture-agent"));
        assert_eq!(handshake.identity_matched, None);
        assert_eq!(handshake.readiness, Readiness::HandshakeReady);
    }

    #[tokio::test]
    async fn handshake_with_wrong_product_identity_is_never_ready() {
        use agent_client_protocol::Agent;
        use agent_client_protocol::schema::v1::{
            AgentCapabilities, InitializeRequest, InitializeResponse,
        };

        let agent = Agent.builder().name("other-product").on_receive_request(
            async |initialize: InitializeRequest, responder, _connection| {
                responder.respond(
                    InitializeResponse::new(initialize.protocol_version)
                        .agent_info(Implementation::new("Other Product", "9.9.9"))
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        );

        let handshake = verify_transport(agent, Duration::from_secs(2), Some("Kimi CLI".into()))
            .await
            .unwrap();
        assert_eq!(handshake.identity_matched, Some(false));
        assert_eq!(handshake.readiness, Readiness::IdentityMismatch);
    }

    #[test]
    fn same_binary_name_discovers_both_kimi_products_unverified() {
        let discovered = discover_with(&builtin_catalog(), |name| {
            (name == "kimi").then(|| PathBuf::from("/home/user/.kimi-code/bin/kimi"))
        });
        for id in ["kimi-code", "kimi"] {
            let agent = discovered.iter().find(|agent| agent.id == id).unwrap();
            assert_eq!(agent.readiness, Readiness::InstalledUnverified, "{id}");
            assert_eq!(agent.launch.as_ref().unwrap().args, vec!["acp".to_owned()]);
        }
        assert_eq!(
            install_plan("kimi-code").unwrap_err(),
            SupplyError::UnmanagedAgent("kimi-code".into())
        );
        // Which registry entry the managed product maps to is a fact of the
        // catalog, so it is read from the catalog. Calling `install_plan` here
        // would resolve the official registry over the network from a default
        // unit test; the live resolution is its own opt-in gate below.
        assert_eq!(
            builtin_catalog()
                .into_iter()
                .find(|entry| entry.id == "kimi")
                .and_then(|entry| entry.registry_id)
                .as_deref(),
            Some("kimi")
        );
    }

    /// The one claim the hermetic test above cannot make: that the catalog's
    /// `registry_id` actually resolves against the official ACP Agent Registry.
    #[test]
    #[ignore = "live registry test: requires network access to the official ACP Agent Registry"]
    fn the_managed_kimi_product_resolves_against_the_official_registry() {
        assert_eq!(install_plan("kimi").unwrap().registry_id, "kimi");
    }
}
