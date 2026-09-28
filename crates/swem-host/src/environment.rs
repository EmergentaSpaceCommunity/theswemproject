//! Runtime environment discovery, compatibility and provisioning contracts.
//!
//! This module belongs to the meta-harness host. It is deliberately independent
//! from SWEM Cycle records and from native ACP session identity: an agent profile
//! asks for guarantees, a backend proves which guarantees it can enforce, and a
//! concrete lease binds the resulting environment to one canonical workspace.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use agent_client_protocol::{ByteStreams, ConnectTo, Role};
use percent_encoding::percent_decode_str;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::AsyncReadExt as _;
use tokio_util::compat::{TokioAsyncReadCompatExt as _, TokioAsyncWriteCompatExt as _};
use url::Url;

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

/// A positive property which a backend must enforce, not merely advertise.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentGuarantee {
    CanonicalWorkspaceBinding,
    PersistentWorkspace,
    AgentProcessIsolation,
    /// Non-allowlisted ambient variables, including credential variables, are
    /// absent. This says nothing about credentials reachable through the host
    /// filesystem and therefore is deliberately weaker than host isolation.
    AmbientEnvironmentFiltering,
    FilesystemIsolation,
    ProcessTreeCleanup,
    ResourceLimits,
    NetworkDenyByDefault,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspacePersistence {
    #[default]
    Session,
    Persistent,
}

/// Backend-neutral requirements supplied by an agent launch profile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentRequirements {
    pub workspace: PathBuf,
    #[serde(default)]
    pub persistence: WorkspacePersistence,
    #[serde(default)]
    pub required_guarantees: BTreeSet<EnvironmentGuarantee>,
    pub cpu_limit: Option<u16>,
    pub memory_mib: Option<u64>,
    pub disk_gib: Option<u64>,
}

impl EnvironmentRequirements {
    /// Build requirements around an existing canonicalizable workspace.
    #[must_use]
    pub fn new(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            persistence: WorkspacePersistence::Session,
            required_guarantees: BTreeSet::from([EnvironmentGuarantee::CanonicalWorkspaceBinding]),
            cpu_limit: None,
            memory_mib: None,
            disk_gib: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendProbeStatus {
    Absent,
    InstalledNotReady,
    Ready,
    Incompatible,
}

/// Read-only evidence about an environment backend.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackendProbe {
    pub backend_id: String,
    /// Stable identity of one concrete endpoint. Multiple endpoints may use the
    /// same backend adapter without sharing storage, lifecycle or cleanup scope.
    pub endpoint_id: String,
    pub status: BackendProbeStatus,
    pub executable: Option<PathBuf>,
    /// Connection-local argv inserted before every backend command. This keeps
    /// transports such as an already-running WSL distribution explicit without
    /// generating shell wrappers or changing Podman's command language.
    #[serde(default)]
    pub command_prefix: Vec<String>,
    pub version: Option<String>,
    pub topology: Value,
    /// Predicates which this backend can attempt to enforce for a concrete
    /// launch. These are compatibility capabilities, not lease evidence.
    pub available_guarantees: BTreeSet<EnvironmentGuarantee>,
    pub limitations: Vec<String>,
    pub diagnostics: Vec<String>,
}

impl BackendProbe {
    /// A direct host process is useful, but is not an isolation boundary.
    #[must_use]
    pub fn direct() -> Self {
        Self {
            backend_id: "direct-process".into(),
            endpoint_id: "direct-process:host".into(),
            status: BackendProbeStatus::Ready,
            executable: None,
            command_prefix: Vec::new(),
            version: None,
            topology: serde_json::json!({"kind": "host_process"}),
            available_guarantees: BTreeSet::from([
                EnvironmentGuarantee::CanonicalWorkspaceBinding,
                EnvironmentGuarantee::AmbientEnvironmentFiltering,
                EnvironmentGuarantee::ProcessTreeCleanup,
            ]),
            limitations: vec![
                "agent process is not contained from the host".into(),
                "the agent can still read host files permitted to the current OS principal".into(),
                "resource and network limits are not enforced".into(),
            ],
            diagnostics: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningDisposition {
    /// Never mutate this host; return remediation only.
    Never,
    /// Apply after one explicit, informed confirmation of the displayed plan.
    Confirm,
    /// Apply automatically under a previously stored host policy.
    Managed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProvisioningStep {
    Run {
        executable: PathBuf,
        args: Vec<String>,
        description: String,
    },
    VerifyBackend,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent host-impact flags are serialized for a consent preview, not control state"
)]
pub struct ProvisioningEffects {
    pub creates_virtual_machine: bool,
    /// Machine initialization may download its OS image before any agent image
    /// is acquired. This is a distinct network/storage effect.
    pub may_download_machine_image: bool,
    pub requires_privilege_elevation: bool,
    pub may_require_restart: bool,
    pub cpu_count: u16,
    pub memory_mib: u64,
    pub disk_gib: u64,
    pub rootful: bool,
    pub user_mode_networking: bool,
    /// On Windows/WSL, enabling user-mode networking changes the networking
    /// distribution shared by all running WSL distributions.
    pub changes_shared_wsl_networking: bool,
    /// Podman Machine can apply provider/configuration default host mounts.
    /// They are machine exposure, not container workspace authorization.
    pub uses_provider_default_host_mounts: bool,
}

/// A serializable preview. Commands are typed argv; no remote shell is stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProvisioningPlan {
    pub plan_id: String,
    pub backend_id: String,
    pub backend_executable: PathBuf,
    #[serde(default)]
    pub backend_command_prefix: Vec<String>,
    pub provider: String,
    pub source: String,
    pub disposition: ProvisioningDisposition,
    pub steps: Vec<ProvisioningStep>,
    pub effects: ProvisioningEffects,
    pub blockers: Vec<String>,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanProvisioningOptions {
    pub machine_name: String,
    pub cpus: u16,
    pub memory_mib: u64,
    pub disk_gib: u64,
    pub rootful: bool,
    pub user_mode_networking: bool,
    pub disposition: ProvisioningDisposition,
}

impl Default for PodmanProvisioningOptions {
    fn default() -> Self {
        Self {
            machine_name: "swem".into(),
            cpus: 2,
            memory_mib: 2_048,
            disk_gib: 20,
            rootful: false,
            user_mode_networking: false,
            disposition: ProvisioningDisposition::Confirm,
        }
    }
}

/// One explicit registry acquisition request. It belongs to environment
/// bootstrap inventory, never to SWEM Cycle or lease guarantees.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanImageBootstrapSpec {
    pub reference: String,
    pub platform: String,
    pub disposition: ProvisioningDisposition,
}

/// Preview of one content-pinned image acquisition. Absence can authorize a
/// pull, while presence still requires low-level inspection before a receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanImageBootstrapPlan {
    pub plan_id: String,
    pub backend_executable: PathBuf,
    pub backend_endpoint_id: String,
    #[serde(default)]
    pub backend_command_prefix: Vec<String>,
    pub source_reference: String,
    pub platform: String,
    pub disposition: ProvisioningDisposition,
    pub pull_required: bool,
    pub steps: Vec<ProvisioningStep>,
    pub blockers: Vec<String>,
}

/// Verified local image identity. `resolved_image` is safe to pass to the
/// no-pull container preparation path; the source digest remains provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanImageBootstrapReceipt {
    pub plan_id: String,
    pub source_reference: String,
    pub resolved_image: String,
    pub image_digest: String,
    pub platform: String,
    pub size_bytes: u64,
}

/// One exact container carrying both SWEM ownership labels but absent from the
/// caller's durable active-lease inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanOrphanCandidate {
    pub container_id: String,
    pub container_name: String,
    pub lease_id: String,
    pub status: String,
}

/// Read-only reconciliation preview. Label matches alone never authorize
/// deletion: name derivation, full container identity and active inventory are
/// all included in the immutable plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanOrphanReconciliationPlan {
    pub plan_id: String,
    pub backend_executable: PathBuf,
    pub backend_endpoint_id: String,
    #[serde(default)]
    pub backend_command_prefix: Vec<String>,
    pub disposition: ProvisioningDisposition,
    pub active_instance_ids: BTreeSet<String>,
    pub candidates: Vec<PodmanOrphanCandidate>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanOrphanReconciliationReceipt {
    pub plan_id: String,
    pub removed: Vec<PodmanOrphanCandidate>,
    pub cleanup_evidence: Vec<EnvironmentGuaranteeEvidence>,
}

/// Host-owned authority for unattended provisioning. It is configuration, not
/// a project record, and constrains rather than merely labels a managed plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedProvisioningPolicy {
    pub policy_id: String,
    pub backend_id: String,
    pub allowed_machine_names: BTreeSet<String>,
    pub max_cpus: u16,
    pub max_memory_mib: u64,
    pub max_disk_gib: u64,
    pub allow_rootful: bool,
    pub allow_user_mode_networking: bool,
}

/// Authority supplied at apply time; a plan's disposition is never authority.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProvisioningAuthorization {
    confirmed_plan_id: Option<String>,
    managed_policy: Option<ManagedProvisioningPolicy>,
}

impl ProvisioningAuthorization {
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn confirmed(plan_id: impl Into<String>) -> Self {
        Self {
            confirmed_plan_id: Some(plan_id.into()),
            managed_policy: None,
        }
    }

    #[must_use]
    pub fn managed(policy: ManagedProvisioningPolicy) -> Self {
        Self {
            confirmed_plan_id: None,
            managed_policy: Some(policy),
        }
    }
}

/// How a concrete environment guarantee was established for this lease.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentEvidenceKind {
    /// The host constructed the launch through an enforcing primitive whose
    /// conformance is covered by executable fixtures.
    EnforcedConfiguration,
    /// The backend's own low-level state was inspected after preparation.
    BackendInspection,
    /// A challenge executed inside the prepared environment observed the
    /// predicate directly.
    RuntimeChallenge,
    /// A terminal observation proved revoke or cleanup behaviour.
    CleanupObservation,
}

/// One typed receipt for one positive predicate. `details` must contain only
/// non-secret enforcement metadata suitable for host inventory and transcripts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentGuaranteeEvidence {
    pub kind: EnvironmentEvidenceKind,
    pub source: String,
    pub details: Value,
}

/// Concrete runtime binding. It is host inventory/evidence, never a Cycle record.
/// The evidence map is the sole source of positive guarantees; a second set
/// would permit claims and receipts to drift apart.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentLease {
    pub lease_id: String,
    pub backend_id: String,
    pub endpoint_id: String,
    pub instance_id: String,
    /// Canonical path owned by the client/host. This is the stable project
    /// identity used for authorization and durable host inventory.
    pub workspace: PathBuf,
    /// The same binding as addressed inside the agent's runtime namespace.
    /// ACP cwd/session operations must use this view, never assume that the
    /// client and agent share path syntax or topology.
    pub agent_workspace: PathBuf,
    pub evidence: BTreeMap<EnvironmentGuarantee, EnvironmentGuaranteeEvidence>,
    pub cleanup_required: bool,
}

/// A workspace path as seen by the Podman service and its container.  On a
/// remote client (including Podman Machine on Windows/macOS) `service_source`
/// is deliberately not inferred from the client-side path: Podman mounts from
/// the server/VM, not necessarily from the client filesystem.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanWorkspaceBinding {
    pub service_source: String,
    pub container_target: String,
}

/// Network authority for one active container. This is deliberately concrete
/// runtime configuration rather than an ordinal "isolation level".
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PodmanNetworkPolicy {
    /// A private network namespace containing loopback only.
    #[default]
    DenyAll,
    /// A private namespace with outbound connectivity and no published ports.
    /// Provider/domain allowlisting, when required, belongs to a stronger
    /// backend or an explicit egress proxy; this policy must not claim it.
    PrivateEgress,
}

impl PodmanNetworkPolicy {
    fn podman_mode(self) -> &'static str {
        match self {
            Self::DenyAll => "none",
            Self::PrivateEgress => "private",
        }
    }
}

/// Opaque reference to a secret already materialized in the selected Podman
/// endpoint. The value is absent by construction. Podman exposes it only to
/// the active process under `target_environment` and masks it in inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanRuntimeSecret {
    pub opaque_reference: String,
    pub target_environment: String,
}

/// Connection-local launch configuration for one prepared Podman instance.
///
/// Images must already exist in local Podman storage and be content-pinned.
/// Agent credentials and arbitrary environment values are intentionally absent:
/// bootstrap/image resolution and secret delivery are separate effects, never
/// a surprise side effect of the first ACP prompt or values leaked through CLI
/// arguments.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PodmanContainerSpec {
    pub image: String,
    pub workspace: PodmanWorkspaceBinding,
    /// Optional persistent native-agent home, separate from clean project
    /// output. The target is fixed to `/home/swem`; only its service source is
    /// selected per connection.
    #[serde(default)]
    pub agent_home: Option<PodmanWorkspaceBinding>,
    /// Active-phase credential bindings. Only opaque Podman references and
    /// environment variable names are serializable here, never secret values.
    #[serde(default)]
    pub runtime_secrets: Vec<PodmanRuntimeSecret>,
    /// Plain variables for the agent process that are not secrets: the model
    /// it should answer from, the address its provider is served at. Each is
    /// emitted as `--env` and checked back against the container exactly, so
    /// the container's environment stays the fixed baseline plus these and
    /// nothing else.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub network: PodmanNetworkPolicy,
    pub agent_executable: String,
    #[serde(default)]
    pub agent_args: Vec<String>,
    pub user: String,
    pub cpu_limit: u16,
    pub memory_mib: u64,
    pub pids_limit: u32,
}

impl PodmanContainerSpec {
    /// A conservative deny-network baseline for an already provisioned image.
    #[must_use]
    pub fn deny_network(
        image: impl Into<String>,
        workspace: PodmanWorkspaceBinding,
        agent_executable: impl Into<String>,
    ) -> Self {
        Self {
            image: image.into(),
            workspace,
            agent_home: None,
            runtime_secrets: Vec::new(),
            environment: BTreeMap::new(),
            network: PodmanNetworkPolicy::DenyAll,
            agent_executable: agent_executable.into(),
            agent_args: Vec::new(),
            user: "1000:1000".into(),
            cpu_limit: 2,
            memory_mib: 2_048,
            pids_limit: 256,
        }
    }
}

impl EnvironmentLease {
    /// Return exactly the predicates backed by concrete lease evidence.
    #[must_use]
    pub fn guarantees(&self) -> BTreeSet<EnvironmentGuarantee> {
        self.evidence.keys().copied().collect()
    }

    /// Resolve one standard ACP `file:` URI from the agent-visible workspace
    /// namespace to the corresponding host workspace candidate.
    ///
    /// Canonical containment and regular-file checks remain the content
    /// store's responsibility. This method only applies the exact namespace
    /// binding proven by this lease; it never fetches a URI or guesses a
    /// service path for an unrelated remote environment.
    ///
    /// # Errors
    ///
    /// Returns [`EnvironmentError`] when the URI is not a local file, the
    /// lease lacks workspace-binding evidence, or the agent path escapes its
    /// declared workspace.
    pub fn resolve_workspace_file_uri(&self, uri: &str) -> Result<PathBuf, EnvironmentError> {
        if !self
            .evidence
            .contains_key(&EnvironmentGuarantee::CanonicalWorkspaceBinding)
        {
            return Err(EnvironmentError::WorkspaceResourceUnavailable(
                "lease has no canonical workspace binding evidence".into(),
            ));
        }
        let url = Url::parse(uri).map_err(|_| {
            EnvironmentError::WorkspaceResourceUnavailable("resource URI is invalid".into())
        })?;
        if url.scheme() != "file"
            || url.host_str().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(EnvironmentError::WorkspaceResourceUnavailable(
                "resource is not a local file URI".into(),
            ));
        }
        if self.agent_workspace == self.workspace {
            return url.to_file_path().map_err(|()| {
                EnvironmentError::WorkspaceResourceUnavailable(
                    "resource URI is not a host file path".into(),
                )
            });
        }
        let decoded = percent_decode_str(url.path()).decode_utf8().map_err(|_| {
            EnvironmentError::WorkspaceResourceUnavailable(
                "agent file URI path is not UTF-8".into(),
            )
        })?;
        if decoded.contains('\0') {
            return Err(EnvironmentError::WorkspaceResourceUnavailable(
                "agent file URI path contains NUL".into(),
            ));
        }
        let agent_path = Path::new(decoded.as_ref());
        let relative = agent_path
            .strip_prefix(&self.agent_workspace)
            .map_err(|_| {
                EnvironmentError::WorkspaceResourceUnavailable(
                    "resource is outside the agent workspace".into(),
                )
            })?;
        if relative
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
        {
            return Err(EnvironmentError::WorkspaceResourceUnavailable(
                "resource path is not a normal workspace-relative path".into(),
            ));
        }
        Ok(self.workspace.join(relative))
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningApplyStatus {
    AlreadyReady,
    Applied,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProvisioningReceipt {
    pub plan_id: String,
    pub status: ProvisioningApplyStatus,
    pub executed_steps: usize,
    pub final_probe: BackendProbe,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum EnvironmentError {
    #[error("workspace path must be an existing absolute directory: {0}")]
    InvalidWorkspace(String),
    #[error("agent executable must be an existing absolute file: {0}")]
    InvalidExecutable(String),
    #[error("environment lease cannot launch this transport: {0}")]
    UnsupportedLease(String),
    #[error("backend {backend_id} cannot enforce required guarantees: {missing:?}")]
    MissingGuarantees {
        backend_id: String,
        missing: Vec<EnvironmentGuarantee>,
    },
    #[error("backend is not ready: {0}")]
    BackendNotReady(String),
    #[error("provisioning plan is blocked: {0:?}")]
    ProvisioningBlocked(Vec<String>),
    #[error("provisioning is disabled by host policy")]
    ProvisioningDisabled,
    #[error("provisioning needs confirmation of plan {0}")]
    ConfirmationRequired(String),
    #[error("provisioning plan identity changed")]
    PlanIdentityMismatch,
    #[error("managed provisioning needs a matching host-owned policy")]
    ManagedPolicyRequired,
    #[error("managed provisioning policy rejected the plan: {0}")]
    ManagedPolicyRejected(String),
    #[error("command failed: {0}")]
    CommandFailed(String),
    #[error("invalid Podman container specification: {0}")]
    InvalidContainerSpec(String),
    #[error("Podman returned an invalid container identity: {0}")]
    InvalidContainerIdentity(String),
    #[error("prepared Podman container failed inspection: {0}")]
    ContainerInspectionFailed(String),
    #[error("environment cannot resolve workspace resource: {0}")]
    WorkspaceResourceUnavailable(String),
    #[error("invalid Podman image bootstrap specification: {0}")]
    InvalidImageBootstrapSpec(String),
    #[error("Podman image bootstrap failed inspection: {0}")]
    ImageInspectionFailed(String),
}

/// Resolve only exact compatible probes; callers choose priority explicitly.
/// There is intentionally no automatic fallback to the direct host backend.
///
/// # Errors
///
/// Returns [`EnvironmentError`] for an invalid workspace, no ready backend, or
/// a ready backend which cannot enforce every required guarantee.
pub fn resolve_backend<'a>(
    requirements: &EnvironmentRequirements,
    probes: impl IntoIterator<Item = &'a BackendProbe>,
) -> Result<&'a BackendProbe, EnvironmentError> {
    canonical_workspace(&requirements.workspace)?;
    let mut last_missing = None;
    for probe in probes {
        if probe.status != BackendProbeStatus::Ready {
            continue;
        }
        let missing = requirements
            .required_guarantees
            .difference(&probe.available_guarantees)
            .copied()
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return Ok(probe);
        }
        last_missing = Some((probe.backend_id.clone(), missing));
    }
    if let Some((backend_id, missing)) = last_missing {
        Err(EnvironmentError::MissingGuarantees {
            backend_id,
            missing,
        })
    } else {
        Err(EnvironmentError::BackendNotReady(
            "no ready environment backend was supplied".into(),
        ))
    }
}

/// Create a truthful direct-process lease for low-assurance profiles.
///
/// # Errors
///
/// Returns [`EnvironmentError`] when the workspace is invalid or requirements
/// ask for guarantees which the host process cannot enforce.
pub fn direct_environment_lease(
    lease_id: impl Into<String>,
    requirements: &EnvironmentRequirements,
) -> Result<EnvironmentLease, EnvironmentError> {
    let direct = BackendProbe::direct();
    resolve_backend(requirements, [&direct])?;
    Ok(EnvironmentLease {
        lease_id: lease_id.into(),
        backend_id: direct.backend_id,
        endpoint_id: direct.endpoint_id,
        instance_id: "host".into(),
        workspace: canonical_workspace(&requirements.workspace)?,
        agent_workspace: canonical_workspace(&requirements.workspace)?,
        evidence: direct_environment_evidence(),
        cleanup_required: true,
    })
}

fn direct_environment_evidence() -> BTreeMap<EnvironmentGuarantee, EnvironmentGuaranteeEvidence> {
    let source = format!("swem-host/direct-process@{}", env!("CARGO_PKG_VERSION"));
    BTreeMap::from([
        (
            EnvironmentGuarantee::CanonicalWorkspaceBinding,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::EnforcedConfiguration,
                source: source.clone(),
                details: serde_json::json!({"mechanism": "canonical_current_dir"}),
            },
        ),
        (
            EnvironmentGuarantee::AmbientEnvironmentFiltering,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::EnforcedConfiguration,
                source: source.clone(),
                details: serde_json::json!({
                    "mechanism": "env_clear_then_allowlist",
                    "allowlist_names": direct_bootstrap_environment().into_keys().collect::<Vec<_>>(),
                }),
            },
        ),
        (
            EnvironmentGuarantee::ProcessTreeCleanup,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::EnforcedConfiguration,
                source,
                details: serde_json::json!({
                    "mechanism": if cfg!(windows) {
                        "process_wrap_windows_job_object"
                    } else {
                        "process_wrap_unix_process_group"
                    },
                    "kill_on_drop": true,
                }),
            },
        ),
    ])
}

/// ACP byte transport backed by one environment lease.
///
/// This is not a second ACP implementation: framing and JSON-RPC remain in the
/// official SDK's [`ByteStreams`]. The type owns only process launch policy and
/// lifecycle containment. Future OCI/remote launchers can construct the same
/// ACP stream boundary without changing native session semantics.
#[derive(Clone)]
pub struct EnvironmentTransport {
    executable: PathBuf,
    args: Vec<String>,
    workspace: PathBuf,
    agent_workspace: PathBuf,
    environment: BTreeMap<String, String>,
    lease_id: String,
    backend_id: String,
    endpoint_id: String,
    instance_id: String,
    cleanup: Option<PodmanCleanup>,
    terminal: Arc<TransportTerminal>,
}

#[derive(Clone, Debug)]
struct PodmanCleanup {
    executable: PathBuf,
    command_prefix: Vec<String>,
    instance_id: String,
    environment: BTreeMap<String, String>,
    terminal: Arc<TransportTerminal>,
}

#[derive(Debug)]
struct TransportTerminal {
    result: Mutex<Option<Result<EnvironmentGuaranteeEvidence, String>>>,
    notify: tokio::sync::Notify,
}

impl TransportTerminal {
    fn new() -> Self {
        Self {
            result: Mutex::new(None),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn record(&self, result: Result<EnvironmentGuaranteeEvidence, String>) {
        if let Ok(mut stored) = self.result.lock()
            && stored.is_none()
        {
            *stored = Some(result);
            self.notify.notify_waiters();
        }
    }

    fn current(&self) -> Option<Result<EnvironmentGuaranteeEvidence, String>> {
        self.result.lock().ok().and_then(|result| result.clone())
    }
}

impl std::fmt::Debug for EnvironmentTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EnvironmentTransport")
            .field("executable", &self.executable)
            .field("args", &self.args)
            .field("workspace", &self.workspace)
            .field("agent_workspace", &self.agent_workspace)
            .field("lease_id", &self.lease_id)
            .field("backend_id", &self.backend_id)
            .field("endpoint_id", &self.endpoint_id)
            .field("instance_id", &self.instance_id)
            .field("cleanup_required", &self.cleanup.is_some())
            .field(
                "has_terminal_evidence",
                &self
                    .terminal
                    .result
                    .lock()
                    .is_ok_and(|result| result.is_some()),
            )
            .field(
                "environment_names",
                &self.environment.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl EnvironmentTransport {
    /// Build the enforced direct-process transport for a concrete lease.
    /// Explicit values are added after a small platform bootstrap allowlist;
    /// every other ambient host variable is removed before spawn.
    ///
    /// # Errors
    ///
    /// Returns [`EnvironmentError`] when the lease is not the direct backend,
    /// its workspace/guarantees are inconsistent, or the executable is not an
    /// existing absolute file.
    pub fn direct(
        lease: &EnvironmentLease,
        executable: &Path,
        args: Vec<String>,
        explicit_environment: BTreeMap<String, String>,
    ) -> Result<Self, EnvironmentError> {
        if lease.backend_id != "direct-process" {
            return Err(EnvironmentError::UnsupportedLease(format!(
                "backend {} needs its own launcher",
                lease.backend_id
            )));
        }
        let workspace = canonical_workspace(&lease.workspace)?;
        if workspace != lease.workspace {
            return Err(EnvironmentError::UnsupportedLease(
                "lease workspace is not canonical".into(),
            ));
        }
        if lease.agent_workspace != lease.workspace {
            return Err(EnvironmentError::UnsupportedLease(
                "direct lease agent workspace differs from its host workspace".into(),
            ));
        }
        let required = BTreeSet::from([
            EnvironmentGuarantee::CanonicalWorkspaceBinding,
            EnvironmentGuarantee::AmbientEnvironmentFiltering,
            EnvironmentGuarantee::ProcessTreeCleanup,
        ]);
        let missing = required
            .difference(&lease.guarantees())
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(EnvironmentError::MissingGuarantees {
                backend_id: lease.backend_id.clone(),
                missing,
            });
        }
        if !executable.is_absolute() || !executable.is_file() {
            return Err(EnvironmentError::InvalidExecutable(
                executable.display().to_string(),
            ));
        }
        let mut environment = direct_bootstrap_environment();
        environment.extend(browser_selection_environment());
        environment.extend(explicit_environment);
        Ok(Self {
            executable: executable.to_path_buf(),
            args,
            workspace,
            agent_workspace: lease.agent_workspace.clone(),
            environment,
            lease_id: lease.lease_id.clone(),
            backend_id: lease.backend_id.clone(),
            endpoint_id: lease.endpoint_id.clone(),
            instance_id: lease.instance_id.clone(),
            cleanup: None,
            terminal: Arc::new(TransportTerminal::new()),
        })
    }

    /// Attach ACP stdio to an already created and inspected Podman instance.
    /// The agent command was fixed at `podman create`; this launcher supplies no
    /// second command and owns force-removal by the exact container ID.
    ///
    /// # Errors
    ///
    /// Returns [`EnvironmentError`] for a non-Podman/unproved lease or an
    /// invalid Podman executable.
    pub fn podman(
        lease: &EnvironmentLease,
        podman_executable: &Path,
    ) -> Result<Self, EnvironmentError> {
        Self::podman_with_command(lease, podman_executable, &[], "podman:native")
    }

    /// Attach through the exact command transport discovered for this lease.
    /// Prefix argv is host-local connection state (for example
    /// `wsl --distribution Ubuntu -- podman`), not a shell fragment.
    ///
    /// # Errors
    ///
    /// Returns [`EnvironmentError`] when the probe is not the endpoint which
    /// prepared the lease or when it is not a usable Podman command.
    pub fn podman_endpoint(
        lease: &EnvironmentLease,
        probe: &BackendProbe,
    ) -> Result<Self, EnvironmentError> {
        if probe.backend_id != "podman" || lease.endpoint_id != probe.endpoint_id {
            return Err(EnvironmentError::UnsupportedLease(format!(
                "lease endpoint {} differs from Podman endpoint {}",
                lease.endpoint_id, probe.endpoint_id
            )));
        }
        let executable = probe
            .executable
            .as_deref()
            .ok_or_else(|| EnvironmentError::BackendNotReady("podman executable missing".into()))?;
        Self::podman_with_command(lease, executable, &probe.command_prefix, &probe.endpoint_id)
    }

    fn podman_with_command(
        lease: &EnvironmentLease,
        podman_executable: &Path,
        command_prefix: &[String],
        endpoint_id: &str,
    ) -> Result<Self, EnvironmentError> {
        if lease.backend_id != "podman" {
            return Err(EnvironmentError::UnsupportedLease(format!(
                "backend {} is not a Podman instance",
                lease.backend_id
            )));
        }
        if lease.endpoint_id != endpoint_id {
            return Err(EnvironmentError::UnsupportedLease(format!(
                "lease endpoint {} differs from command endpoint {endpoint_id}",
                lease.endpoint_id
            )));
        }
        if !podman_executable.is_absolute() || !podman_executable.is_file() {
            return Err(EnvironmentError::InvalidExecutable(
                podman_executable.display().to_string(),
            ));
        }
        if !is_sha256_hex(&lease.instance_id) {
            return Err(EnvironmentError::InvalidContainerIdentity(
                lease.instance_id.clone(),
            ));
        }
        let required = BTreeSet::from([
            EnvironmentGuarantee::CanonicalWorkspaceBinding,
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuarantee::AmbientEnvironmentFiltering,
            EnvironmentGuarantee::FilesystemIsolation,
            EnvironmentGuarantee::ResourceLimits,
        ]);
        let missing = required
            .difference(&lease.guarantees())
            .copied()
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            return Err(EnvironmentError::MissingGuarantees {
                backend_id: lease.backend_id.clone(),
                missing,
            });
        }
        let workspace = canonical_workspace(&lease.workspace)?;
        let environment = podman_invocation_environment();
        let terminal = Arc::new(TransportTerminal::new());
        Ok(Self {
            executable: podman_executable.to_path_buf(),
            args: prefixed_backend_args(
                command_prefix,
                &[
                    "start".into(),
                    "--interactive".into(),
                    "--attach".into(),
                    lease.instance_id.clone(),
                ],
            ),
            workspace,
            agent_workspace: lease.agent_workspace.clone(),
            environment: environment.clone(),
            lease_id: lease.lease_id.clone(),
            backend_id: lease.backend_id.clone(),
            endpoint_id: lease.endpoint_id.clone(),
            instance_id: lease.instance_id.clone(),
            cleanup: Some(PodmanCleanup {
                executable: podman_executable.to_path_buf(),
                command_prefix: command_prefix.to_vec(),
                instance_id: lease.instance_id.clone(),
                environment,
                terminal: Arc::clone(&terminal),
            }),
            terminal,
        })
    }

    /// Prove that a connection-local transport belongs to the supplied lease.
    #[must_use]
    pub fn matches_lease(&self, lease: &EnvironmentLease) -> bool {
        self.lease_id == lease.lease_id
            && self.backend_id == lease.backend_id
            && self.endpoint_id == lease.endpoint_id
            && self.instance_id == lease.instance_id
            && self.workspace == lease.workspace
            && self.agent_workspace == lease.agent_workspace
    }

    /// Revoke a prepared environment when a caller rejects the connection
    /// before ACP transport launch. Direct transports have no prepared
    /// instance and return `None`.
    ///
    /// # Errors
    ///
    /// Returns [`EnvironmentError`] when exact Podman removal cannot be
    /// observed. Calling this more than once is safe because removal uses the
    /// prepared instance id with `--ignore`.
    pub async fn abort_prepared(
        &self,
    ) -> Result<Option<EnvironmentGuaranteeEvidence>, EnvironmentError> {
        let Some(cleanup) = self.cleanup.clone() else {
            return Ok(None);
        };
        let result = tokio::task::spawn_blocking(move || {
            let result = cleanup_podman_after_transport_drop(&cleanup);
            cleanup.terminal.record(result.clone());
            result
        })
        .await
        .map_err(|error| EnvironmentError::CommandFailed(error.to_string()))?;
        result.map(Some).map_err(EnvironmentError::CommandFailed)
    }

    /// Terminal lifecycle evidence becomes available only after the transport
    /// observed exact instance removal. It is never pre-populated from policy.
    #[must_use]
    pub fn terminal_evidence(&self) -> Option<EnvironmentGuaranteeEvidence> {
        self.terminal.current().and_then(Result::ok)
    }

    /// Wait until exact cleanup has either been observed or failed. This closes
    /// the lifecycle race where the ACP client callback finishes before the
    /// transport future has completed its environment teardown.
    ///
    /// # Errors
    ///
    /// Returns [`EnvironmentError`] on cleanup failure or timeout.
    pub async fn wait_for_terminal_evidence(
        &self,
        timeout: std::time::Duration,
    ) -> Result<EnvironmentGuaranteeEvidence, EnvironmentError> {
        let wait = async {
            loop {
                let notified = self.terminal.notify.notified();
                if let Some(result) = self.terminal.current() {
                    return result.map_err(EnvironmentError::CommandFailed);
                }
                notified.await;
            }
        };
        tokio::time::timeout(timeout, wait).await.map_err(|_| {
            EnvironmentError::CommandFailed(
                "timed out waiting for terminal environment cleanup evidence".into(),
            )
        })?
    }

    fn spawn(self) -> Result<SpawnedEnvironmentProcess, agent_client_protocol::Error> {
        let mut command = tokio::process::Command::new(&self.executable);
        command
            .args(&self.args)
            .current_dir(&self.workspace)
            .env_clear()
            .envs(&self.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut command = CommandWrap::from(command);
        command.wrap(KillOnDrop);
        #[cfg(windows)]
        command.wrap(JobObject);
        #[cfg(unix)]
        command.wrap(ProcessGroup::leader());

        let mut child = command.spawn().map_err(|error| {
            agent_client_protocol::Error::internal_error().data(error.to_string())
        })?;
        let stdin = child.stdin().take().ok_or_else(|| {
            agent_client_protocol::Error::internal_error().data("agent stdin was not piped")
        })?;
        let stdout = child.stdout().take().ok_or_else(|| {
            agent_client_protocol::Error::internal_error().data("agent stdout was not piped")
        })?;
        let stderr = child.stderr().take().ok_or_else(|| {
            agent_client_protocol::Error::internal_error().data("agent stderr was not piped")
        })?;
        Ok(SpawnedEnvironmentProcess {
            stdin,
            stdout,
            stderr,
            child: ChildTreeGuard(Some(child)),
            cleanup: self.cleanup.map(PodmanCleanupGuard::new),
        })
    }
}

impl<R: Role> ConnectTo<R> for EnvironmentTransport {
    async fn connect_to(
        self,
        client: impl ConnectTo<R::Counterpart>,
    ) -> Result<(), agent_client_protocol::Error> {
        let SpawnedEnvironmentProcess {
            stdin,
            stdout,
            stderr,
            mut child,
            mut cleanup,
        } = self.spawn()?;
        let stderr_task = tokio::spawn(drain_stderr_tail(stderr));
        let streams = ByteStreams::new(stdin.compat_write(), stdout.compat());
        let mut protocol = Box::pin(ConnectTo::<R>::connect_to(streams, client));

        let first = tokio::select! {
            result = &mut protocol => TransportCompletion::Protocol(result),
            status = child.wait() => TransportCompletion::Child(status),
        };

        let protocol_result = match first {
            TransportCompletion::Protocol(result) => {
                child.terminate();
                let _ = child.wait().await;
                let _ = stderr_task.await;
                result
            }
            TransportCompletion::Child(status) => {
                let status = status.map_err(|error| {
                    agent_client_protocol::Error::internal_error().data(error.to_string())
                })?;
                let protocol_result =
                    tokio::time::timeout(std::time::Duration::from_secs(2), &mut protocol)
                        .await
                        .map_err(|_| {
                            agent_client_protocol::Error::internal_error()
                                .data("agent exited but ACP transport did not close")
                        })?;
                let stderr = stderr_task.await.unwrap_or_default();
                if status.success() {
                    protocol_result
                } else {
                    Err(
                        agent_client_protocol::Error::internal_error().data(serde_json::json!({
                            "message": "agent process exited unsuccessfully",
                            "status": status.to_string(),
                            "stderr_tail": stderr,
                        })),
                    )
                }
            }
        };
        let cleanup_result = match cleanup.as_mut() {
            Some(cleanup) => cleanup.cleanup().await,
            None => Ok(()),
        };
        match (protocol_result, cleanup_result) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(protocol), Ok(())) => Err(protocol),
            (Ok(()), Err(cleanup)) => Err(cleanup),
            (Err(protocol), Err(cleanup)) => Err(agent_client_protocol::Error::internal_error()
                .data(serde_json::json!({
                    "message": "ACP transport and environment cleanup both failed",
                    "protocol": protocol.to_string(),
                    "cleanup": cleanup.to_string(),
                }))),
        }
    }
}

enum TransportCompletion {
    Protocol(Result<(), agent_client_protocol::Error>),
    Child(std::io::Result<std::process::ExitStatus>),
}

struct SpawnedEnvironmentProcess {
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
    child: ChildTreeGuard,
    cleanup: Option<PodmanCleanupGuard>,
}

struct ChildTreeGuard(Option<Box<dyn ChildWrapper>>);

impl ChildTreeGuard {
    fn terminate(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.start_kill();
        }
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.0
            .as_mut()
            .expect("child guard is never disarmed")
            .wait()
            .await
    }
}

impl Drop for ChildTreeGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

struct PodmanCleanupGuard {
    cleanup: PodmanCleanup,
    completed: bool,
}

impl PodmanCleanupGuard {
    fn new(cleanup: PodmanCleanup) -> Self {
        Self {
            cleanup,
            completed: false,
        }
    }

    async fn cleanup(&mut self) -> Result<(), agent_client_protocol::Error> {
        let result = async {
            let remove = self
                .command(&["rm", "--force", "--ignore", &self.cleanup.instance_id])
                .output()
                .await
                .map_err(|error| error.to_string())?;
            if !remove.status.success() {
                return Err(format!(
                    "podman rm: {}",
                    String::from_utf8_lossy(&remove.stderr).trim()
                ));
            }
            let exists = self
                .command(&["container", "exists", &self.cleanup.instance_id])
                .output()
                .await
                .map_err(|error| error.to_string())?;
            terminal_cleanup_evidence(&self.cleanup.instance_id, &exists)
        }
        .await;
        self.cleanup.terminal.record(result.clone());
        if result.is_ok() {
            self.completed = true;
        }
        result.map(|_| ()).map_err(acp_environment_error)
    }

    fn command(&self, args: &[&str]) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(&self.cleanup.executable);
        let operation = args.iter().map(|argument| (*argument).to_owned());
        command
            .args(self.cleanup.command_prefix.iter())
            .args(operation)
            .env_clear()
            .envs(&self.cleanup.environment)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

impl Drop for PodmanCleanupGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        let cleanup = self.cleanup.clone();
        let _ = std::thread::Builder::new()
            .name("swem-podman-cleanup".into())
            .spawn(move || {
                let result = cleanup_podman_after_transport_drop(&cleanup);
                cleanup.terminal.record(result);
            });
    }
}

fn cleanup_podman_after_transport_drop(
    cleanup: &PodmanCleanup,
) -> Result<EnvironmentGuaranteeEvidence, String> {
    let mut remove = Command::new(&cleanup.executable);
    let remove = remove
        .args(&cleanup.command_prefix)
        .args(["rm", "--force", "--ignore", &cleanup.instance_id])
        .env_clear()
        .envs(&cleanup.environment)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    if !remove.status.success() {
        return Err(format!(
            "podman rm: {}",
            String::from_utf8_lossy(&remove.stderr).trim()
        ));
    }
    let mut exists = Command::new(&cleanup.executable);
    let exists = exists
        .args(&cleanup.command_prefix)
        .args(["container", "exists", &cleanup.instance_id])
        .env_clear()
        .envs(&cleanup.environment)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| error.to_string())?;
    terminal_cleanup_evidence(&cleanup.instance_id, &exists)
}

fn terminal_cleanup_evidence(
    instance_id: &str,
    exists: &Output,
) -> Result<EnvironmentGuaranteeEvidence, String> {
    match exists.status.code() {
        Some(1) => Ok(EnvironmentGuaranteeEvidence {
            kind: EnvironmentEvidenceKind::CleanupObservation,
            source: "podman/container-exists".into(),
            details: serde_json::json!({"container_id": instance_id, "exists": false}),
        }),
        Some(0) => Err("container still exists after force removal".into()),
        code => Err(format!(
            "podman container exists returned {code:?}: {}",
            String::from_utf8_lossy(&exists.stderr).trim()
        )),
    }
}

fn acp_environment_error(message: impl Into<String>) -> agent_client_protocol::Error {
    agent_client_protocol::Error::internal_error().data(message.into())
}

async fn drain_stderr_tail(mut stderr: tokio::process::ChildStderr) -> String {
    const LIMIT: usize = 64 * 1024;
    let mut tail = VecDeque::with_capacity(LIMIT);
    let mut buffer = [0_u8; 8 * 1024];
    while let Ok(read) = stderr.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        tail.extend(&buffer[..read]);
        while tail.len() > LIMIT {
            tail.pop_front();
        }
    }
    String::from_utf8_lossy(&tail.into_iter().collect::<Vec<_>>()).into_owned()
}

fn direct_bootstrap_environment() -> BTreeMap<String, String> {
    #[cfg(windows)]
    const NAMES: &[&str] = &[
        "SystemRoot",
        "WINDIR",
        "ComSpec",
        "PATHEXT",
        "PATH",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "APPDATA",
        "LOCALAPPDATA",
    ];
    #[cfg(not(windows))]
    const NAMES: &[&str] = &[
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "TERM",
    ];

    NAMES
        .iter()
        .filter_map(|name| env::var(name).ok().map(|value| ((*name).into(), value)))
        .collect()
}

/// Ambient variables which select the browser the music engine renders in.
///
/// The same argument as the Podman names below, one layer further out. A host
/// started with a browser named explicitly has *resolved* that question; an
/// agent it launches, and the Cycle that agent attaches, must resolve it the
/// same way. Without them the filtered bootstrap leaves only `PATH`, so a
/// browser installed anywhere else is invisible below this line - and the
/// failure is silent in the worst way: every root that needs no subprocess
/// closes normally and the one that needs the engine is simply not resolved
///.
///
/// Carried only into a direct launch. A container has its own filesystem, so
/// a path from this host would name a browser that is not there, and the
/// agent inside it must discover its own.
const BROWSER_SELECTION_NAMES: &[&str] = &["SWEM_BROWSER", "SWEM_BROWSER_NO_SANDBOX"];

fn browser_selection_environment() -> BTreeMap<String, String> {
    BROWSER_SELECTION_NAMES
        .iter()
        .filter_map(|name| env::var(name).ok().map(|value| ((*name).into(), value)))
        .collect()
}

/// Ambient variables which select the Podman engine/connection. Discovery and
/// lease preparation run `podman` with the inherited host environment; the
/// transport's `start`/`rm` invocations must resolve the same engine, so these
/// names are carried across the bootstrap allowlist. Values are connection
/// state, never recorded.
const PODMAN_ENGINE_SELECTION_NAMES: &[&str] = &[
    "CONTAINER_HOST",
    "CONTAINER_CONNECTION",
    "CONTAINER_SSHKEY",
    "CONTAINERS_CONF",
    "CONTAINERS_STORAGE_CONF",
    "DOCKER_HOST",
    "REGISTRY_AUTH_FILE",
    "XDG_RUNTIME_DIR",
];

fn podman_invocation_environment() -> BTreeMap<String, String> {
    let mut environment = direct_bootstrap_environment();
    environment.extend(
        PODMAN_ENGINE_SELECTION_NAMES
            .iter()
            .filter_map(|name| env::var(name).ok().map(|value| ((*name).into(), value))),
    );
    environment
}

fn canonical_workspace(workspace: &Path) -> Result<PathBuf, EnvironmentError> {
    if !workspace.is_absolute() || !workspace.is_dir() {
        return Err(EnvironmentError::InvalidWorkspace(
            workspace.display().to_string(),
        ));
    }
    workspace
        .canonicalize()
        .map_err(|_| EnvironmentError::InvalidWorkspace(workspace.display().to_string()))
}

/// Discover Podman without creating a machine, connection, image or container.
#[must_use]
pub fn probe_podman() -> BackendProbe {
    probe_podman_with(find_executable("podman"), &SystemCommandRunner)
}

fn probe_podman_with(executable: Option<PathBuf>, runner: &dyn CommandRunner) -> BackendProbe {
    probe_podman_endpoint_with(
        executable,
        Vec::new(),
        "podman:native".into(),
        serde_json::json!({"kind": "native_podman"}),
        true,
        runner,
    )
}

/// Discover independently usable Podman endpoints without starting a stopped
/// WSL distribution. The native client is always returned; on Windows, already
/// running distributions with a local Podman engine appear as additional
/// endpoints rather than being disguised as Podman Machines.
#[must_use]
pub fn probe_podman_endpoints() -> Vec<BackendProbe> {
    let native = probe_podman();
    #[cfg(windows)]
    let managed_machine_distributions = podman_machine_wsl_distributions(&native);
    // Only the Windows arm below pushes, so the binding is immutable everywhere
    // else. Stating that here keeps one strict-lint result on every host instead
    // of a gate that passes only where WSL exists.
    #[cfg_attr(
        not(windows),
        expect(unused_mut, reason = "extended only by the Windows WSL arm")
    )]
    let mut probes = vec![native];
    #[cfg(windows)]
    if let Some(wsl) = find_executable("wsl") {
        for distribution in running_wsl_distributions(&wsl)
            .into_iter()
            .filter(|name| !managed_machine_distributions.contains(&name.to_ascii_lowercase()))
        {
            let prefix = vec![
                "--distribution".into(),
                distribution.clone(),
                "--".into(),
                "podman".into(),
            ];
            probes.push(probe_podman_endpoint_with(
                Some(wsl.clone()),
                prefix,
                format!("podman:wsl:{distribution}"),
                serde_json::json!({
                    "kind": "wsl_local_podman",
                    "distribution": distribution,
                    "discovery": "running_distributions_only",
                }),
                false,
                &SystemCommandRunner,
            ));
        }
    }
    probes
}

#[cfg(any(windows, test))]
fn podman_machine_wsl_distributions(probe: &BackendProbe) -> BTreeSet<String> {
    probe
        .topology
        .get("machines")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|machine| get_any_value(machine, &["Name", "name"]).and_then(Value::as_str))
        .map(|name| format!("podman-{name}").to_ascii_lowercase())
        .collect()
}

fn probe_podman_endpoint_with(
    executable: Option<PathBuf>,
    command_prefix: Vec<String>,
    endpoint_id: String,
    endpoint_topology: Value,
    inspect_machine_topology: bool,
    runner: &dyn CommandRunner,
) -> BackendProbe {
    let Some(executable) = executable else {
        return BackendProbe {
            backend_id: "podman".into(),
            endpoint_id,
            status: BackendProbeStatus::Absent,
            executable: None,
            command_prefix,
            version: None,
            topology: endpoint_topology,
            available_guarantees: BTreeSet::new(),
            limitations: Vec::new(),
            diagnostics: vec!["podman executable was not found".into()],
        };
    };
    let run = |args: &[String]| {
        let mut command = command_prefix.clone();
        command.extend_from_slice(args);
        runner.output(&executable, &command)
    };
    let version = run(&["--version".into()])
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
    let engine_info = run(&["info".into(), "--format".into(), "json".into()]);
    let (engine_json, engine_error) = parse_json_output(engine_info);
    let engine_ready = engine_error.is_none() && engine_json.as_object().is_some();
    let machines = inspect_machine_topology.then(|| {
        run(&[
            "machine".into(),
            "list".into(),
            "--format".into(),
            "json".into(),
        ])
    });
    let connections = inspect_machine_topology.then(|| {
        run(&[
            "system".into(),
            "connection".into(),
            "list".into(),
            "--format".into(),
            "json".into(),
        ])
    });
    let machine_info = inspect_machine_topology.then(|| {
        run(&[
            "machine".into(),
            "info".into(),
            "--format".into(),
            "json".into(),
        ])
    });
    let (machine_json, machine_error) = machines.map_or((Value::Null, None), parse_json_output);
    let (connection_json, connection_error) =
        connections.map_or((Value::Null, None), parse_json_output);
    let (machine_info_json, machine_info_error) =
        machine_info.map_or((Value::Null, None), parse_json_output);
    let status = if version.is_none() {
        BackendProbeStatus::Incompatible
    } else if engine_ready {
        BackendProbeStatus::Ready
    } else {
        BackendProbeStatus::InstalledNotReady
    };
    let diagnostics = [
        engine_error,
        machine_error,
        connection_error,
        machine_info_error,
    ]
    .into_iter()
    .flatten()
    .collect();
    let available_guarantees = (status == BackendProbeStatus::Ready)
        .then(podman_available_guarantees)
        .unwrap_or_default();
    BackendProbe {
        backend_id: "podman".into(),
        endpoint_id,
        status,
        executable: Some(executable),
        command_prefix,
        version,
        topology: serde_json::json!({
            "endpoint": endpoint_topology,
            "engine_info": engine_json,
            "machines": machine_json,
            "connections": connection_json,
            "machine_info": machine_info_json,
        }),
        // These are operations a ready engine can attempt. They become positive
        // lease claims only after create + low-level inspection succeeds.
        available_guarantees,
        limitations: vec![
            "engine readiness is capability discovery, not lease enforcement evidence".into(),
            "network deny-by-default is a per-lease policy and is not proved by machine readiness"
                .into(),
            "the Podman engine socket must never be mounted into an agent container".into(),
        ],
        diagnostics,
    }
}

#[cfg(windows)]
fn running_wsl_distributions(wsl: &Path) -> Vec<String> {
    let Ok(output) = Command::new(wsl)
        .args(["--list", "--running", "--quiet"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    decode_windows_command_text(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
        .map(str::to_owned)
        .collect()
}

#[cfg(windows)]
fn decode_windows_command_text(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes.chunks_exact(2).any(|pair| pair[1] == 0) {
        let words = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&words)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn podman_available_guarantees() -> BTreeSet<EnvironmentGuarantee> {
    BTreeSet::from([
        EnvironmentGuarantee::CanonicalWorkspaceBinding,
        EnvironmentGuarantee::PersistentWorkspace,
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::AmbientEnvironmentFiltering,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuarantee::ResourceLimits,
        EnvironmentGuarantee::NetworkDenyByDefault,
    ])
}

fn parse_json_output(result: Result<Output, String>) -> (Value, Option<String>) {
    match result {
        Ok(output) if output.status.success() => serde_json::from_slice(&output.stdout)
            .map_or_else(
                |error| (Value::Array(vec![]), Some(error.to_string())),
                |value| (value, None),
            ),
        Ok(output) => (
            Value::Array(vec![]),
            Some(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
        ),
        Err(error) => (Value::Array(vec![]), Some(error)),
    }
}

fn machine_is_running(machine: &Value) -> bool {
    ["Running", "running", "IsRunning", "isRunning"]
        .iter()
        .find_map(|key| machine.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn named_machine_state(probe: &BackendProbe, name: &str) -> (bool, bool) {
    probe
        .topology
        .get("machines")
        .and_then(Value::as_array)
        .and_then(|machines| {
            machines.iter().find(|machine| {
                ["Name", "name"]
                    .iter()
                    .find_map(|key| machine.get(key))
                    .and_then(Value::as_str)
                    == Some(name)
            })
        })
        .map_or((false, false), |machine| {
            (true, machine_is_running(machine))
        })
}

fn named_connection_exists(probe: &BackendProbe, name: &str) -> bool {
    probe
        .topology
        .get("connections")
        .and_then(Value::as_array)
        .is_some_and(|connections| {
            connections.iter().any(|connection| {
                ["Name", "name"]
                    .iter()
                    .find_map(|key| connection.get(key))
                    .and_then(Value::as_str)
                    .is_some_and(|candidate| candidate == name || candidate.contains(name))
            })
        })
}

fn machine_provider(probe: &BackendProbe) -> String {
    probe
        .topology
        .pointer("/machine_info/Host/VMType")
        .or_else(|| probe.topology.pointer("/machine_info/host/vmType"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned()
}

/// Create a stopped, content-pinned Podman container and turn only verified
/// low-level configuration into lease evidence.  This operation never pulls an
/// image and never infers a server-side mount path from a remote client path.
///
/// # Errors
///
/// Returns [`EnvironmentError`] when the engine is not ready, the specification
/// is unsafe/ambiguous, creation fails, or inspection differs from the requested
/// policy. A container created before a later failure is removed by exact
/// generated identity, even when Podman's stdout cannot be trusted as an ID.
pub fn prepare_podman_lease(
    probe: &BackendProbe,
    lease_id: impl Into<String>,
    requirements: &EnvironmentRequirements,
    spec: &PodmanContainerSpec,
) -> Result<EnvironmentLease, EnvironmentError> {
    prepare_podman_lease_with(
        probe,
        lease_id.into(),
        requirements,
        spec,
        &SystemCommandRunner,
    )
}

fn prepare_podman_lease_with(
    probe: &BackendProbe,
    lease_id: String,
    requirements: &EnvironmentRequirements,
    spec: &PodmanContainerSpec,
    runner: &dyn CommandRunner,
) -> Result<EnvironmentLease, EnvironmentError> {
    if probe.backend_id != "podman" {
        return Err(EnvironmentError::BackendNotReady(probe.backend_id.clone()));
    }
    resolve_backend(requirements, [probe])?;
    validate_podman_spec(&lease_id, requirements, spec)?;
    let container_name = podman_container_name(&lease_id);
    let executable = probe
        .executable
        .as_deref()
        .ok_or_else(|| EnvironmentError::BackendNotReady("podman executable missing".into()))?;
    let create = run_backend_command(
        runner,
        executable,
        &probe.command_prefix,
        &podman_create_args(&lease_id, spec)?,
    )
    .map_err(EnvironmentError::CommandFailed)?;
    if !create.status.success() {
        return Err(EnvironmentError::CommandFailed(format!(
            "podman create: {}",
            String::from_utf8_lossy(&create.stderr).trim()
        )));
    }
    let instance_id = String::from_utf8_lossy(&create.stdout).trim().to_owned();
    if !is_sha256_hex(&instance_id) {
        let _ = cleanup_podman_instance_with(
            executable,
            &probe.command_prefix,
            &container_name,
            runner,
        );
        return Err(EnvironmentError::InvalidContainerIdentity(instance_id));
    }

    let inspected = run_backend_command(
        runner,
        executable,
        &probe.command_prefix,
        &["container".into(), "inspect".into(), instance_id.clone()],
    );
    let evidence = match inspected {
        Ok(output) if output.status.success() => serde_json::from_slice(&output.stdout)
            .map_err(|error| EnvironmentError::ContainerInspectionFailed(error.to_string()))
            .and_then(|value| {
                verify_podman_inspection(
                    &value,
                    &instance_id,
                    &container_name,
                    &lease_id,
                    requirements,
                    spec,
                )
            }),
        Ok(output) => Err(EnvironmentError::ContainerInspectionFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        )),
        Err(error) => Err(EnvironmentError::ContainerInspectionFailed(error)),
    };
    let evidence = match evidence {
        Ok(evidence) => evidence,
        Err(error) => {
            let _ = cleanup_podman_instance_with(
                executable,
                &probe.command_prefix,
                &instance_id,
                runner,
            );
            return Err(error);
        }
    };
    let missing_after_inspection = requirements
        .required_guarantees
        .difference(&evidence.keys().copied().collect())
        .copied()
        .collect::<Vec<_>>();
    if !missing_after_inspection.is_empty() {
        let _ =
            cleanup_podman_instance_with(executable, &probe.command_prefix, &instance_id, runner);
        return Err(EnvironmentError::MissingGuarantees {
            backend_id: "podman".into(),
            missing: missing_after_inspection,
        });
    }

    Ok(EnvironmentLease {
        lease_id,
        backend_id: "podman".into(),
        endpoint_id: probe.endpoint_id.clone(),
        instance_id,
        workspace: canonical_workspace(&requirements.workspace)?,
        agent_workspace: PathBuf::from(&spec.workspace.container_target),
        evidence,
        cleanup_required: true,
    })
}

fn validate_podman_spec(
    lease_id: &str,
    requirements: &EnvironmentRequirements,
    spec: &PodmanContainerSpec,
) -> Result<(), EnvironmentError> {
    if lease_id.is_empty()
        || lease_id.len() > 128
        || lease_id
            .chars()
            .any(|character| character.is_control() || character == ',')
    {
        return Err(EnvironmentError::InvalidContainerSpec(
            "lease id is not safe for an exact label".into(),
        ));
    }
    if !is_pinned_image(&spec.image) {
        return Err(EnvironmentError::InvalidContainerSpec(
            "image must be a sha256 image ID or a name pinned with @sha256:<64 hex>".into(),
        ));
    }
    validate_podman_bindings(spec)?;
    validate_podman_runtime_secrets(spec)?;
    validate_podman_process(requirements, spec)
}

fn validate_podman_bindings(spec: &PodmanContainerSpec) -> Result<(), EnvironmentError> {
    for (name, value) in [
        ("service workspace", spec.workspace.service_source.as_str()),
        (
            "container workspace",
            spec.workspace.container_target.as_str(),
        ),
        ("agent executable", spec.agent_executable.as_str()),
    ] {
        if !value.starts_with('/')
            || value
                .chars()
                .any(|character| character.is_control() || character == ',')
        {
            return Err(EnvironmentError::InvalidContainerSpec(format!(
                "{name} must be an absolute Linux path without control characters or commas"
            )));
        }
    }
    if let Some(agent_home) = &spec.agent_home {
        for (name, value) in [
            ("service agent home", agent_home.service_source.as_str()),
            ("container agent home", agent_home.container_target.as_str()),
        ] {
            if !value.starts_with('/')
                || value
                    .chars()
                    .any(|character| character.is_control() || character == ',')
            {
                return Err(EnvironmentError::InvalidContainerSpec(format!(
                    "{name} must be an absolute Linux path without control characters or commas"
                )));
            }
        }
        if agent_home.container_target != "/home/swem" {
            return Err(EnvironmentError::InvalidContainerSpec(
                "persistent agent home target must be exactly /home/swem".into(),
            ));
        }
        if agent_home.service_source == spec.workspace.service_source
            || agent_home.container_target == spec.workspace.container_target
        {
            return Err(EnvironmentError::InvalidContainerSpec(
                "agent home and project workspace must be distinct bindings".into(),
            ));
        }
    }
    Ok(())
}

fn validate_podman_runtime_secrets(spec: &PodmanContainerSpec) -> Result<(), EnvironmentError> {
    if spec.runtime_secrets.len() > 16 {
        return Err(EnvironmentError::InvalidContainerSpec(
            "at most 16 active runtime secret references are supported".into(),
        ));
    }
    let mut secret_references = BTreeSet::new();
    let mut secret_targets = BTreeSet::new();
    for binding in &spec.runtime_secrets {
        if !is_safe_podman_reference(&binding.opaque_reference) {
            return Err(EnvironmentError::InvalidContainerSpec(
                "runtime secret reference must contain only ASCII letters, digits, '.', '_' or '-'"
                    .into(),
            ));
        }
        if !is_safe_environment_name(&binding.target_environment) {
            return Err(EnvironmentError::InvalidContainerSpec(
                "runtime secret target must be a portable environment variable name".into(),
            ));
        }
        if !secret_references.insert(&binding.opaque_reference)
            || !secret_targets.insert(&binding.target_environment)
        {
            return Err(EnvironmentError::InvalidContainerSpec(
                "runtime secret references and targets must be unique".into(),
            ));
        }
    }
    Ok(())
}

fn validate_podman_process(
    requirements: &EnvironmentRequirements,
    spec: &PodmanContainerSpec,
) -> Result<(), EnvironmentError> {
    if spec.network == PodmanNetworkPolicy::PrivateEgress
        && requirements
            .required_guarantees
            .contains(&EnvironmentGuarantee::NetworkDenyByDefault)
    {
        return Err(EnvironmentError::InvalidContainerSpec(
            "private egress cannot satisfy network-deny-by-default".into(),
        ));
    }
    if spec.agent_args.iter().any(|argument| {
        argument.contains('\0') || argument.contains('\n') || argument.contains('\r')
    }) {
        return Err(EnvironmentError::InvalidContainerSpec(
            "agent arguments contain control characters".into(),
        ));
    }
    let Some((uid, gid)) = spec.user.split_once(':') else {
        return Err(EnvironmentError::InvalidContainerSpec(
            "user must be a numeric non-root uid:gid".into(),
        ));
    };
    if uid.parse::<u32>().ok().is_none_or(|uid| uid == 0)
        || gid.parse::<u32>().ok().is_none_or(|gid| gid == 0)
    {
        return Err(EnvironmentError::InvalidContainerSpec(
            "user must be a numeric non-root uid:gid".into(),
        ));
    }
    if spec.cpu_limit == 0 || spec.memory_mib == 0 || spec.pids_limit == 0 {
        return Err(EnvironmentError::InvalidContainerSpec(
            "CPU, memory and PID limits must be positive".into(),
        ));
    }
    if requirements
        .cpu_limit
        .is_some_and(|limit| limit != spec.cpu_limit)
        || requirements
            .memory_mib
            .is_some_and(|limit| limit != spec.memory_mib)
    {
        return Err(EnvironmentError::InvalidContainerSpec(
            "container resource limits differ from resolved requirements".into(),
        ));
    }
    if requirements.disk_gib.is_some() {
        return Err(EnvironmentError::InvalidContainerSpec(
            "per-container disk quota is not enforced by this Podman slice".into(),
        ));
    }
    Ok(())
}

fn is_safe_podman_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_safe_environment_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && value.len() <= 128
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_pinned_image(image: &str) -> bool {
    image.strip_prefix("sha256:").is_some_and(is_hex_digest)
        || image
            .rsplit_once("@sha256:")
            .is_some_and(|(name, digest)| !name.is_empty() && is_hex_digest(digest))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_hex_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn podman_container_name(lease_id: &str) -> String {
    format!("swem-{:x}", Sha256::digest(lease_id.as_bytes()))
}

fn podman_create_args(
    lease_id: &str,
    spec: &PodmanContainerSpec,
) -> Result<Vec<String>, EnvironmentError> {
    let entrypoint = serde_json::to_string(&[&spec.agent_executable])
        .map_err(|error| EnvironmentError::InvalidContainerSpec(error.to_string()))?;
    let mount = format!(
        "type=bind,src={},dst={},rw=true,nodev,nosuid",
        spec.workspace.service_source, spec.workspace.container_target
    );
    let agent_home_mount = spec.agent_home.as_ref().map(|binding| {
        format!(
            "type=bind,src={},dst={},rw=true,nodev,nosuid",
            binding.service_source, binding.container_target
        )
    });
    let mut args = vec![
        "create".into(),
        "--pull=never".into(),
        "--name".into(),
        podman_container_name(lease_id),
        "--label".into(),
        format!("io.swem.lease-id={lease_id}"),
        "--label".into(),
        "io.swem.managed=true".into(),
        "--interactive".into(),
        "--attach=stdin".into(),
        "--attach=stdout".into(),
        "--attach=stderr".into(),
        format!("--network={}", spec.network.podman_mode()),
        "--no-hosts".into(),
        // Podman copies this machine's proxy variables into every container
        // it creates unless told not to. On a machine behind a proxy - a
        // corporate laptop, this workspace's own container - that silently
        // breaks the filtering this lease promises, and the container is then
        // refused by its own inspection. The environment a container gets is
        // the one the spec names, and nothing from the machine around it.
        "--http-proxy=false".into(),
        "--ipc=private".into(),
        "--pid=private".into(),
        "--cgroupns=private".into(),
        "--cap-drop=all".into(),
        "--security-opt=no-new-privileges".into(),
        "--read-only".into(),
        "--read-only-tmpfs=true".into(),
        "--image-volume=ignore".into(),
        "--restart=no".into(),
        "--pids-limit".into(),
        spec.pids_limit.to_string(),
        "--memory".into(),
        format!("{}m", spec.memory_mib),
        "--cpus".into(),
        spec.cpu_limit.to_string(),
        "--user".into(),
        spec.user.clone(),
        "--workdir".into(),
        spec.workspace.container_target.clone(),
        "--mount".into(),
        mount,
        "--unsetenv-all".into(),
        "--env".into(),
        "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".into(),
        "--env".into(),
        "HOME=/home/swem".into(),
        "--env".into(),
        "LANG=C.UTF-8".into(),
    ];
    if let Some(agent_home_mount) = agent_home_mount {
        args.extend(["--mount".into(), agent_home_mount]);
    }
    for (name, value) in &spec.environment {
        args.extend(["--env".into(), format!("{name}={value}")]);
    }
    for binding in &spec.runtime_secrets {
        args.extend([
            "--secret".into(),
            format!(
                "{},type=env,target={}",
                binding.opaque_reference, binding.target_environment
            ),
        ]);
    }
    args.extend(["--entrypoint".into(), entrypoint, spec.image.clone()]);
    args.extend(spec.agent_args.iter().cloned());
    Ok(args)
}

#[allow(
    clippy::too_many_lines,
    reason = "one fail-closed comparison keeps the inspected Podman policy and its evidence derivation adjacent"
)]
fn verify_podman_inspection(
    value: &Value,
    instance_id: &str,
    container_name: &str,
    lease_id: &str,
    requirements: &EnvironmentRequirements,
    spec: &PodmanContainerSpec,
) -> Result<BTreeMap<EnvironmentGuarantee, EnvironmentGuaranteeEvidence>, EnvironmentError> {
    let container = value
        .as_array()
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first())
        .ok_or_else(|| inspection_error("inspect must contain exactly one container"))?;
    require_string_value(container, &["Id", "ID"], instance_id, "container id")?;
    let inspected_name = get_any_value(container, &["Name"])
        .and_then(Value::as_str)
        .ok_or_else(|| inspection_error("container name is missing"))?;
    if inspected_name != container_name && inspected_name != format!("/{container_name}") {
        return Err(inspection_error("container name differs"));
    }
    let state = require_object(container, &["State"], "state")?;
    let status = get_any(state, &["Status", "status"])
        .and_then(Value::as_str)
        .ok_or_else(|| inspection_error("state status is missing"))?;
    if !matches!(status, "created" | "configured" | "initialized")
        || get_any(state, &["Running", "running"])
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        return Err(inspection_error(
            "container is not stopped before ACP attach",
        ));
    }
    let config = require_object(container, &["Config"], "config")?;
    let host = require_object(container, &["HostConfig"], "host config")?;
    require_string(config, &["User"], &spec.user, "non-root user")?;
    require_string(
        config,
        &["WorkingDir"],
        &spec.workspace.container_target,
        "working directory",
    )?;
    require_bool(config, &["Tty"], false, "TTY")?;
    require_bool(config, &["OpenStdin"], true, "interactive stdin")?;
    // Podman 5.8 records all Config.Attach* fields as false for `create` even
    // when the exact CreateCommand contains --attach flags. OpenStdin is stable
    // prepared-state evidence; bidirectional stdio is proved only by the ACP
    // runtime challenge through `start --interactive --attach`.
    verify_entrypoint(config, spec)?;
    verify_image(container, spec)?;
    verify_labels(config, lease_id)?;
    verify_environment(config, spec)?;
    verify_runtime_secret_references(config, spec)?;

    verify_network(host, spec.network)?;
    require_string(host, &["IpcMode"], "private", "IPC namespace")?;
    require_string(host, &["PidMode"], "private", "PID namespace")?;
    require_string(host, &["CgroupMode"], "private", "cgroup namespace")?;
    require_bool(host, &["ReadonlyRootfs"], true, "read-only rootfs")?;
    require_integer(
        host,
        &["Memory"],
        spec.memory_mib.saturating_mul(1_048_576),
        "memory limit",
    )?;
    require_integer(
        host,
        &["NanoCpus", "NanoCPUs"],
        u64::from(spec.cpu_limit).saturating_mul(1_000_000_000),
        "CPU limit",
    )?;
    require_integer(
        host,
        &["PidsLimit"],
        u64::from(spec.pids_limit),
        "PID limit",
    )?;
    verify_security(container, host)?;
    verify_mounts(container, spec)?;

    let source = "podman/container-inspect".to_owned();
    let mut evidence = BTreeMap::from([
        (
            EnvironmentGuarantee::CanonicalWorkspaceBinding,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::BackendInspection,
                source: source.clone(),
                details: serde_json::json!({
                    "client_workspace": canonical_workspace(&requirements.workspace)?,
                    "service_source": spec.workspace.service_source,
                    "container_target": spec.workspace.container_target,
                    "read_write": true,
                }),
            },
        ),
        (
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::BackendInspection,
                source: source.clone(),
                details: serde_json::json!({
                    "user": spec.user,
                    "pid_namespace": "private",
                    "ipc_namespace": "private",
                    "cgroup_namespace": "private",
                    "capabilities": "dropped_all",
                    "no_new_privileges": true,
                }),
            },
        ),
        (
            EnvironmentGuarantee::AmbientEnvironmentFiltering,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::BackendInspection,
                source: source.clone(),
                details: serde_json::json!({
                    "mechanism": "unsetenv_all_then_fixed_non_secret_baseline",
                    "names": ["PATH", "HOME", "LANG"],
                    "runtime_secret_targets": spec.runtime_secrets.iter()
                        .map(|binding| binding.target_environment.as_str())
                        .collect::<Vec<_>>(),
                    "runtime_secret_values_recorded": false,
                }),
            },
        ),
        (
            EnvironmentGuarantee::FilesystemIsolation,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::BackendInspection,
                source: source.clone(),
                details: serde_json::json!({
                    "rootfs": "read_only",
                    "workspace_mount": spec.workspace.container_target,
                    "agent_home_mount": spec.agent_home.as_ref()
                        .map(|binding| binding.container_target.as_str()),
                    "engine_socket_mounted": false,
                }),
            },
        ),
        (
            EnvironmentGuarantee::ResourceLimits,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::BackendInspection,
                source: source.clone(),
                details: serde_json::json!({
                    "cpus": spec.cpu_limit,
                    "memory_mib": spec.memory_mib,
                    "pids": spec.pids_limit,
                }),
            },
        ),
    ]);
    evidence.insert(
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuaranteeEvidence {
            // The container is created with --restart=no and the host owns a
            // mandatory terminal removal protocol; the actual observation
            // (`rm --force` then `container exists -> 1`) is a separate
            // cleanup receipt, never claimed here in advance.
            kind: EnvironmentEvidenceKind::EnforcedConfiguration,
            source: "host/terminal-cleanup-protocol".into(),
            details: serde_json::json!({
                "restart_policy": "no",
                "cleanup_required": true,
                "terminal_receipt": "podman rm --force; container exists -> 1",
                "terminal_receipt_observed_at_lease_time": false,
            }),
        },
    );
    if spec.network == PodmanNetworkPolicy::DenyAll {
        evidence.insert(
            EnvironmentGuarantee::NetworkDenyByDefault,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::BackendInspection,
                source,
                details: serde_json::json!({"network_mode": "none", "published_ports": 0}),
            },
        );
    }
    if requirements.persistence == WorkspacePersistence::Persistent {
        evidence.insert(
            EnvironmentGuarantee::PersistentWorkspace,
            EnvironmentGuaranteeEvidence {
                // Persistence follows from the bind-mount configuration; no
                // backend inspection of durability was performed.
                kind: EnvironmentEvidenceKind::EnforcedConfiguration,
                source: "host/workspace-bind-mount".into(),
                details: serde_json::json!({"mechanism": "external_bind_mount"}),
            },
        );
    }
    Ok(evidence)
}

fn verify_entrypoint(
    config: &serde_json::Map<String, Value>,
    spec: &PodmanContainerSpec,
) -> Result<(), EnvironmentError> {
    let entrypoint = get_any(config, &["Entrypoint"])
        .ok_or_else(|| inspection_error("entrypoint is missing"))?;
    let entrypoint_matches = entrypoint.as_str() == Some(spec.agent_executable.as_str())
        || entrypoint.as_array().is_some_and(|items| {
            items.len() == 1 && items[0].as_str() == Some(spec.agent_executable.as_str())
        });
    if !entrypoint_matches {
        return Err(inspection_error(
            "agent entrypoint differs from prepared command",
        ));
    }
    let command = get_any(config, &["Cmd"]);
    let actual = command
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    let expected = spec
        .agent_args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if actual != expected {
        return Err(inspection_error(
            "agent arguments differ from prepared command",
        ));
    }
    Ok(())
}

fn verify_network(
    host: &serde_json::Map<String, Value>,
    policy: PodmanNetworkPolicy,
) -> Result<(), EnvironmentError> {
    let mode = get_any(host, &["NetworkMode"])
        .and_then(Value::as_str)
        .ok_or_else(|| inspection_error("network mode is missing"))?;
    let accepted = match policy {
        PodmanNetworkPolicy::DenyAll => mode == "none",
        // `private` is a Podman CLI policy alias. Rootless Podman 5 resolves it
        // to `pasta`; rootful endpoints normally resolve it to `bridge`.
        // Inspect the concrete namespace driver instead of requiring the alias
        // to survive serialization, while rejecting host/container sharing.
        PodmanNetworkPolicy::PrivateEgress => {
            matches!(mode, "private" | "pasta" | "slirp4netns" | "bridge")
        }
    };
    if !accepted {
        return Err(inspection_error("network mode differs"));
    }
    if get_any(host, &["PortBindings"]).is_some_and(|bindings| {
        !bindings.is_null() && !bindings.as_object().is_some_and(serde_json::Map::is_empty)
    }) {
        return Err(inspection_error(
            "prepared agent container publishes host ports",
        ));
    }
    Ok(())
}

fn verify_image(container: &Value, spec: &PodmanContainerSpec) -> Result<(), EnvironmentError> {
    let expected_digest = spec
        .image
        .rsplit_once('@')
        .map_or(spec.image.as_str(), |(_, digest)| digest);
    let matches = ["ImageDigest", "Image"]
        .iter()
        .filter_map(|key| container.get(key).and_then(Value::as_str))
        .any(|actual| {
            actual == spec.image
                || actual == expected_digest
                || actual.strip_prefix("sha256:").unwrap_or(actual)
                    == expected_digest
                        .strip_prefix("sha256:")
                        .unwrap_or(expected_digest)
        });
    matches
        .then_some(())
        .ok_or_else(|| inspection_error("image digest differs from pinned image"))
}

fn verify_labels(
    config: &serde_json::Map<String, Value>,
    lease_id: &str,
) -> Result<(), EnvironmentError> {
    let labels = get_any(config, &["Labels"])
        .and_then(Value::as_object)
        .ok_or_else(|| inspection_error("SWEM ownership labels are missing"))?;
    if labels.get("io.swem.lease-id").and_then(Value::as_str) != Some(lease_id)
        || labels.get("io.swem.managed").and_then(Value::as_str) != Some("true")
    {
        return Err(inspection_error("SWEM ownership labels differ"));
    }
    Ok(())
}

fn verify_environment(
    config: &serde_json::Map<String, Value>,
    spec: &PodmanContainerSpec,
) -> Result<(), EnvironmentError> {
    let entries = get_any(config, &["Env"])
        .and_then(Value::as_array)
        .ok_or_else(|| inspection_error("container environment is missing"))?;
    let allowed = ["PATH", "HOME", "LANG", "HOSTNAME", "container"];
    let mut observed = BTreeSet::new();
    for entry in entries.iter().filter_map(Value::as_str) {
        let (name, value) = entry.split_once('=').unwrap_or((entry, ""));
        // The spec's own plain variables are allowed with exactly the value
        // the spec gave them; anything else is a variable nobody declared.
        match spec.environment.get(name) {
            Some(declared) if declared == value => {}
            Some(_) => {
                return Err(inspection_error(format!(
                    "container environment variable {name} does not hold the declared value"
                )));
            }
            None if allowed.contains(&name) => {}
            None => {
                return Err(inspection_error(format!(
                    "unexpected container environment variable {name}"
                )));
            }
        }
        observed.insert(name);
    }
    if let Some(missing) = spec
        .environment
        .keys()
        .find(|name| !observed.contains(name.as_str()))
    {
        return Err(inspection_error(format!(
            "declared container environment variable {missing} is missing"
        )));
    }
    if !["PATH", "HOME", "LANG"]
        .iter()
        .all(|name| observed.contains(name))
    {
        return Err(inspection_error(
            "fixed container environment baseline is incomplete",
        ));
    }
    Ok(())
}

fn verify_runtime_secret_references(
    config: &serde_json::Map<String, Value>,
    spec: &PodmanContainerSpec,
) -> Result<(), EnvironmentError> {
    let Some(command) = get_any(config, &["CreateCommand"]).and_then(Value::as_array) else {
        return spec
            .runtime_secrets
            .is_empty()
            .then_some(())
            .ok_or_else(|| inspection_error("runtime secret references are not inspectable"));
    };
    let command = command.iter().filter_map(Value::as_str).collect::<Vec<_>>();
    let mut observed = Vec::new();
    let mut index = 0;
    while index < command.len() {
        if command[index] == "--secret" {
            let value = command
                .get(index + 1)
                .ok_or_else(|| inspection_error("--secret has no opaque reference"))?;
            observed.push((*value).to_owned());
            index += 2;
            continue;
        }
        if let Some(value) = command[index].strip_prefix("--secret=") {
            observed.push(value.to_owned());
        }
        index += 1;
    }
    let expected = spec
        .runtime_secrets
        .iter()
        .map(|binding| {
            format!(
                "{},type=env,target={}",
                binding.opaque_reference, binding.target_environment
            )
        })
        .collect::<Vec<_>>();
    if observed != expected {
        return Err(inspection_error(
            "runtime secret reference inventory differs from the prepared specification",
        ));
    }
    Ok(())
}

fn verify_security(
    container: &Value,
    host: &serde_json::Map<String, Value>,
) -> Result<(), EnvironmentError> {
    let cap_drop = get_any(host, &["CapDrop"])
        .and_then(Value::as_array)
        .ok_or_else(|| inspection_error("capability drop set is missing"))?;
    let literal_all = cap_drop
        .iter()
        .filter_map(Value::as_str)
        .any(|capability| matches!(capability.to_ascii_lowercase().as_str(), "all" | "cap_all"));
    let create_requested_all = get_any_value(container, &["Config"])
        .and_then(|config| get_any_value(config, &["CreateCommand"]))
        .and_then(Value::as_array)
        .is_some_and(|command| {
            command
                .iter()
                .filter_map(Value::as_str)
                .any(|argument| argument == "--cap-drop=all")
        });
    // Podman 5.8 expands `all` to the engine's complete default capability
    // inventory in HostConfig.CapDrop instead of preserving the literal token.
    if cap_drop.is_empty() || (!literal_all && !create_requested_all) {
        return Err(inspection_error(
            "all Linux capabilities were not configured to be dropped",
        ));
    }
    if get_any(host, &["CapAdd"])
        .and_then(Value::as_array)
        .is_some_and(|caps| !caps.is_empty())
    {
        return Err(inspection_error("Linux capabilities were added"));
    }
    if get_any_value(container, &["EffectiveCaps"])
        .and_then(Value::as_array)
        .is_some_and(|caps| !caps.is_empty())
    {
        return Err(inspection_error(
            "effective Linux capabilities are not empty",
        ));
    }
    let security = get_any(host, &["SecurityOpt"])
        .and_then(Value::as_array)
        .ok_or_else(|| inspection_error("security options are missing"))?;
    if !security
        .iter()
        .filter_map(Value::as_str)
        .any(|option| option.starts_with("no-new-privileges"))
    {
        return Err(inspection_error("no-new-privileges is missing"));
    }
    Ok(())
}

fn verify_mounts(container: &Value, spec: &PodmanContainerSpec) -> Result<(), EnvironmentError> {
    let mounts = get_any_value(container, &["Mounts"])
        .and_then(Value::as_array)
        .ok_or_else(|| inspection_error("mount inventory is missing"))?;
    for mount in mounts {
        let source = get_any_value(mount, &["Source", "source"])
            .and_then(Value::as_str)
            .unwrap_or_default();
        let destination = get_any_value(mount, &["Destination", "destination"])
            .and_then(Value::as_str)
            .unwrap_or_default();
        if source.ends_with("podman.sock")
            || source.ends_with("docker.sock")
            || destination.ends_with("podman.sock")
            || destination.ends_with("docker.sock")
        {
            return Err(inspection_error("container engine socket is mounted"));
        }
    }
    let workspace = mounts
        .iter()
        .find(|mount| {
            get_any_value(mount, &["Destination", "destination"]).and_then(Value::as_str)
                == Some(spec.workspace.container_target.as_str())
        })
        .ok_or_else(|| inspection_error("workspace mount is missing"))?;
    require_string_value(
        workspace,
        &["Source", "source"],
        &spec.workspace.service_source,
        "workspace source",
    )?;
    require_bool_value(workspace, &["RW", "rw"], true, "workspace write mode")?;
    if let Some(binding) = &spec.agent_home {
        let agent_home = mounts
            .iter()
            .find(|mount| {
                get_any_value(mount, &["Destination", "destination"]).and_then(Value::as_str)
                    == Some(binding.container_target.as_str())
            })
            .ok_or_else(|| inspection_error("persistent agent home mount is missing"))?;
        require_string_value(
            agent_home,
            &["Source", "source"],
            &binding.service_source,
            "agent home source",
        )?;
        require_bool_value(agent_home, &["RW", "rw"], true, "agent home write mode")?;
    }
    let allowed_bind_targets = [
        Some(spec.workspace.container_target.as_str()),
        spec.agent_home
            .as_ref()
            .map(|binding| binding.container_target.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect::<BTreeSet<_>>();
    for mount in mounts {
        let mount_type = get_any_value(mount, &["Type", "type"])
            .and_then(Value::as_str)
            .unwrap_or_default();
        let destination = get_any_value(mount, &["Destination", "destination"])
            .and_then(Value::as_str)
            .unwrap_or_default();
        if mount_type == "bind" && !allowed_bind_targets.contains(destination) {
            return Err(inspection_error(format!(
                "unexpected bind mount target {destination}"
            )));
        }
    }
    Ok(())
}

fn get_any<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn get_any_value<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    value.as_object().and_then(|object| get_any(object, keys))
}

fn require_object<'a>(
    value: &'a Value,
    keys: &[&str],
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, EnvironmentError> {
    get_any_value(value, keys)
        .and_then(Value::as_object)
        .ok_or_else(|| inspection_error(format!("{label} is missing")))
}

fn require_string(
    value: &serde_json::Map<String, Value>,
    keys: &[&str],
    expected: &str,
    label: &str,
) -> Result<(), EnvironmentError> {
    (get_any(value, keys).and_then(Value::as_str) == Some(expected))
        .then_some(())
        .ok_or_else(|| inspection_error(format!("{label} differs")))
}

fn require_string_value(
    value: &Value,
    keys: &[&str],
    expected: &str,
    label: &str,
) -> Result<(), EnvironmentError> {
    (get_any_value(value, keys).and_then(Value::as_str) == Some(expected))
        .then_some(())
        .ok_or_else(|| inspection_error(format!("{label} differs")))
}

fn require_bool(
    value: &serde_json::Map<String, Value>,
    keys: &[&str],
    expected: bool,
    label: &str,
) -> Result<(), EnvironmentError> {
    (get_any(value, keys).and_then(Value::as_bool) == Some(expected))
        .then_some(())
        .ok_or_else(|| inspection_error(format!("{label} differs")))
}

fn require_bool_value(
    value: &Value,
    keys: &[&str],
    expected: bool,
    label: &str,
) -> Result<(), EnvironmentError> {
    (get_any_value(value, keys).and_then(Value::as_bool) == Some(expected))
        .then_some(())
        .ok_or_else(|| inspection_error(format!("{label} differs")))
}

fn require_integer(
    value: &serde_json::Map<String, Value>,
    keys: &[&str],
    expected: u64,
    label: &str,
) -> Result<(), EnvironmentError> {
    (get_any(value, keys).and_then(Value::as_u64) == Some(expected))
        .then_some(())
        .ok_or_else(|| inspection_error(format!("{label} differs")))
}

fn inspection_error(message: impl Into<String>) -> EnvironmentError {
    EnvironmentError::ContainerInspectionFailed(message.into())
}

/// Remove one exact SWEM-owned Podman instance and prove absence from storage.
///
/// # Errors
///
/// Returns [`EnvironmentError`] if force removal fails or Podman reports a
/// storage error/existing container after removal.
pub fn cleanup_podman_lease(
    probe: &BackendProbe,
    lease: &EnvironmentLease,
) -> Result<EnvironmentGuaranteeEvidence, EnvironmentError> {
    if probe.backend_id != "podman" || lease.backend_id != "podman" {
        return Err(EnvironmentError::UnsupportedLease(
            "cleanup requires a Podman probe and Podman lease".into(),
        ));
    }
    let executable = probe
        .executable
        .as_deref()
        .ok_or_else(|| EnvironmentError::BackendNotReady("podman executable missing".into()))?;
    if lease.endpoint_id != probe.endpoint_id {
        return Err(EnvironmentError::UnsupportedLease(format!(
            "lease endpoint {} differs from probe endpoint {}",
            lease.endpoint_id, probe.endpoint_id
        )));
    }
    cleanup_podman_instance_with(
        executable,
        &probe.command_prefix,
        &lease.instance_id,
        &SystemCommandRunner,
    )
}

fn cleanup_podman_instance_with(
    executable: &Path,
    command_prefix: &[String],
    instance_id: &str,
    runner: &dyn CommandRunner,
) -> Result<EnvironmentGuaranteeEvidence, EnvironmentError> {
    let remove = run_backend_command(
        runner,
        executable,
        command_prefix,
        &[
            "rm".into(),
            "--force".into(),
            "--ignore".into(),
            instance_id.into(),
        ],
    )
    .map_err(EnvironmentError::CommandFailed)?;
    if !remove.status.success() {
        return Err(EnvironmentError::CommandFailed(format!(
            "podman rm: {}",
            String::from_utf8_lossy(&remove.stderr).trim()
        )));
    }
    let exists = run_backend_command(
        runner,
        executable,
        command_prefix,
        &["container".into(), "exists".into(), instance_id.into()],
    )
    .map_err(EnvironmentError::CommandFailed)?;
    match exists.status.code() {
        Some(1) => Ok(EnvironmentGuaranteeEvidence {
            kind: EnvironmentEvidenceKind::CleanupObservation,
            source: "podman/container-exists".into(),
            details: serde_json::json!({"container_id": instance_id, "exists": false}),
        }),
        Some(0) => Err(inspection_error(
            "container still exists after force removal",
        )),
        code => Err(EnvironmentError::CommandFailed(format!(
            "podman container exists returned {code:?}: {}",
            String::from_utf8_lossy(&exists.stderr).trim()
        ))),
    }
}

/// Preview exact SWEM-owned containers which are absent from durable active
/// lease inventory. Discovery is read-only and never treats a label match as
/// sufficient ownership proof.
///
/// # Errors
///
/// Returns [`EnvironmentError`] when the endpoint is not ready or its inventory
/// cannot be read as Podman JSON.
pub fn podman_orphan_reconciliation_plan(
    probe: &BackendProbe,
    active_instance_ids: BTreeSet<String>,
) -> Result<PodmanOrphanReconciliationPlan, EnvironmentError> {
    podman_orphan_reconciliation_plan_with(probe, active_instance_ids, &SystemCommandRunner)
}

fn podman_orphan_reconciliation_plan_with(
    probe: &BackendProbe,
    active_instance_ids: BTreeSet<String>,
    runner: &dyn CommandRunner,
) -> Result<PodmanOrphanReconciliationPlan, EnvironmentError> {
    if probe.backend_id != "podman" || probe.status != BackendProbeStatus::Ready {
        return Err(EnvironmentError::BackendNotReady("podman".into()));
    }
    if active_instance_ids.iter().any(|id| !is_sha256_hex(id)) {
        return Err(EnvironmentError::InvalidContainerIdentity(
            "active lease inventory contains a non-full container id".into(),
        ));
    }
    let executable = probe
        .executable
        .clone()
        .ok_or_else(|| EnvironmentError::BackendNotReady("podman executable missing".into()))?;
    let output = run_backend_command(
        runner,
        &executable,
        &probe.command_prefix,
        &[
            "ps".into(),
            "--all".into(),
            "--filter".into(),
            "label=io.swem.managed=true".into(),
            "--no-trunc".into(),
            "--format".into(),
            "json".into(),
        ],
    )
    .map_err(EnvironmentError::CommandFailed)?;
    if !output.status.success() {
        return Err(EnvironmentError::CommandFailed(format!(
            "podman ps: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let inventory: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| EnvironmentError::ContainerInspectionFailed(error.to_string()))?;
    let entries = inventory.as_array().ok_or_else(|| {
        EnvironmentError::ContainerInspectionFailed("podman ps must return an array".into())
    })?;
    let mut candidates = Vec::new();
    let mut blockers = Vec::new();
    for entry in entries {
        match verified_orphan_candidate(entry, &active_instance_ids) {
            Ok(Some(candidate)) => candidates.push(candidate),
            Ok(None) => {}
            Err(error) => blockers.push(error),
        }
    }
    candidates.sort_by(|left, right| left.container_id.cmp(&right.container_id));
    let mut plan = PodmanOrphanReconciliationPlan {
        plan_id: String::new(),
        backend_executable: executable,
        backend_endpoint_id: probe.endpoint_id.clone(),
        backend_command_prefix: probe.command_prefix.clone(),
        disposition: ProvisioningDisposition::Confirm,
        active_instance_ids,
        candidates,
        blockers,
    };
    plan.plan_id = orphan_reconciliation_plan_identity(&plan);
    Ok(plan)
}

fn verified_orphan_candidate(
    entry: &Value,
    active_instance_ids: &BTreeSet<String>,
) -> Result<Option<PodmanOrphanCandidate>, String> {
    let id = get_any_value(entry, &["Id", "ID"])
        .and_then(Value::as_str)
        .ok_or_else(|| "managed inventory entry has no container id".to_owned())?;
    if !is_sha256_hex(id) {
        return Err(format!("managed inventory entry has non-full id {id}"));
    }
    let labels = get_any_value(entry, &["Labels", "labels"])
        .and_then(Value::as_object)
        .ok_or_else(|| format!("managed container {id} has no label map"))?;
    if labels.get("io.swem.managed").and_then(Value::as_str) != Some("true") {
        return Err(format!("filtered container {id} lacks exact managed label"));
    }
    let lease_id = labels
        .get("io.swem.lease-id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("managed container {id} lacks lease label"))?;
    let name = get_any_value(entry, &["Name", "Names"])
        .and_then(|value| {
            value.as_str().or_else(|| {
                value
                    .as_array()
                    .and_then(|names| names.first())
                    .and_then(Value::as_str)
            })
        })
        .ok_or_else(|| format!("managed container {id} has no name"))?;
    let name = name.trim_start_matches('/');
    if name != podman_container_name(lease_id) {
        return Err(format!(
            "managed container {id} name is inconsistent with its lease label"
        ));
    }
    if active_instance_ids.contains(id) {
        return Ok(None);
    }
    let status = get_any_value(entry, &["State", "Status"])
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    Ok(Some(PodmanOrphanCandidate {
        container_id: id.to_owned(),
        container_name: name.to_owned(),
        lease_id: lease_id.to_owned(),
        status,
    }))
}

/// Named identity payload: adding a field changes the shape and therefore
/// invalidates previously minted plan ids, unlike a positional tuple.
#[derive(Serialize)]
struct OrphanReconciliationIdentity<'a> {
    backend_executable: &'a Path,
    backend_endpoint_id: &'a str,
    backend_command_prefix: &'a [String],
    disposition: ProvisioningDisposition,
    active_instance_ids: &'a BTreeSet<String>,
    candidates: &'a [PodmanOrphanCandidate],
    blockers: &'a [String],
}

fn orphan_reconciliation_plan_identity(plan: &PodmanOrphanReconciliationPlan) -> String {
    let identity = serde_json::to_vec(&OrphanReconciliationIdentity {
        backend_executable: &plan.backend_executable,
        backend_endpoint_id: &plan.backend_endpoint_id,
        backend_command_prefix: &plan.backend_command_prefix,
        disposition: plan.disposition,
        active_instance_ids: &plan.active_instance_ids,
        candidates: &plan.candidates,
        blockers: &plan.blockers,
    })
    .expect("serializable orphan reconciliation identity");
    format!("sha256:{:x}", Sha256::digest(identity))
}

/// Apply an unchanged orphan preview and prove terminal absence for every exact
/// candidate id.
///
/// # Errors
///
/// Returns [`EnvironmentError`] for changed/blocked/unauthorized plans or any
/// cleanup operation which does not end in `container exists -> 1`.
pub fn apply_podman_orphan_reconciliation_plan(
    plan: &PodmanOrphanReconciliationPlan,
    authorization: &ProvisioningAuthorization,
) -> Result<PodmanOrphanReconciliationReceipt, EnvironmentError> {
    if orphan_reconciliation_plan_identity(plan) != plan.plan_id {
        return Err(EnvironmentError::PlanIdentityMismatch);
    }
    apply_podman_orphan_reconciliation_plan_with(plan, authorization, &SystemCommandRunner)
}

fn apply_podman_orphan_reconciliation_plan_with(
    plan: &PodmanOrphanReconciliationPlan,
    authorization: &ProvisioningAuthorization,
    runner: &dyn CommandRunner,
) -> Result<PodmanOrphanReconciliationReceipt, EnvironmentError> {
    if !plan.blockers.is_empty() {
        return Err(EnvironmentError::ProvisioningBlocked(plan.blockers.clone()));
    }
    match plan.disposition {
        ProvisioningDisposition::Confirm => {
            if authorization.confirmed_plan_id.as_deref() != Some(plan.plan_id.as_str()) {
                return Err(EnvironmentError::ConfirmationRequired(plan.plan_id.clone()));
            }
        }
        ProvisioningDisposition::Never => return Err(EnvironmentError::ProvisioningDisabled),
        ProvisioningDisposition::Managed => return Err(EnvironmentError::ManagedPolicyRequired),
    }
    let mut evidence = Vec::with_capacity(plan.candidates.len());
    for candidate in &plan.candidates {
        evidence.push(cleanup_podman_instance_with(
            &plan.backend_executable,
            &plan.backend_command_prefix,
            &candidate.container_id,
            runner,
        )?);
    }
    Ok(PodmanOrphanReconciliationReceipt {
        plan_id: plan.plan_id.clone(),
        removed: plan.candidates.clone(),
        cleanup_evidence: evidence,
    })
}

/// Build a read-only preview for acquiring one exact registry image.
///
/// # Errors
///
/// Returns [`EnvironmentError`] when Podman is not ready or the source is not a
/// fully-qualified digest reference with an explicit Linux platform.
pub fn podman_image_bootstrap_plan(
    probe: &BackendProbe,
    spec: &PodmanImageBootstrapSpec,
) -> Result<PodmanImageBootstrapPlan, EnvironmentError> {
    podman_image_bootstrap_plan_with(probe, spec, &SystemCommandRunner)
}

fn podman_image_bootstrap_plan_with(
    probe: &BackendProbe,
    spec: &PodmanImageBootstrapSpec,
    runner: &dyn CommandRunner,
) -> Result<PodmanImageBootstrapPlan, EnvironmentError> {
    if probe.backend_id != "podman" || probe.status != BackendProbeStatus::Ready {
        return Err(EnvironmentError::BackendNotReady("podman".into()));
    }
    validate_image_bootstrap_spec(spec)?;
    let executable = probe
        .executable
        .clone()
        .ok_or_else(|| EnvironmentError::BackendNotReady("podman executable missing".into()))?;
    let exists = run_backend_command(
        runner,
        &executable,
        &probe.command_prefix,
        &["image".into(), "exists".into(), spec.reference.clone()],
    )
    .map_err(EnvironmentError::CommandFailed)?;
    let (pull_required, blockers) = match exists.status.code() {
        Some(0) => (false, Vec::new()),
        Some(1) => (true, Vec::new()),
        Some(125) => (
            false,
            vec!["Podman could not access local image storage".into()],
        ),
        code => (
            false,
            vec![format!(
                "podman image exists returned unexpected exit code {code:?}"
            )],
        ),
    };
    let mut steps = Vec::new();
    if pull_required {
        steps.push(ProvisioningStep::Run {
            executable: executable.clone(),
            args: prefixed_backend_args(
                &probe.command_prefix,
                &[
                    "pull".into(),
                    "--quiet".into(),
                    "--policy=always".into(),
                    "--tls-verify=true".into(),
                    "--platform".into(),
                    spec.platform.clone(),
                    spec.reference.clone(),
                ],
            ),
            description: format!("acquire pinned image {}", spec.reference),
        });
    }
    steps.push(ProvisioningStep::Run {
        executable: executable.clone(),
        args: prefixed_backend_args(
            &probe.command_prefix,
            &["image".into(), "inspect".into(), spec.reference.clone()],
        ),
        description: format!("inspect pinned image {}", spec.reference),
    });
    let mut plan = PodmanImageBootstrapPlan {
        plan_id: String::new(),
        backend_executable: executable,
        backend_endpoint_id: probe.endpoint_id.clone(),
        backend_command_prefix: probe.command_prefix.clone(),
        source_reference: spec.reference.clone(),
        platform: spec.platform.clone(),
        disposition: spec.disposition,
        pull_required,
        steps,
        blockers,
    };
    plan.plan_id = image_bootstrap_plan_identity(&plan);
    Ok(plan)
}

fn validate_image_bootstrap_spec(spec: &PodmanImageBootstrapSpec) -> Result<(), EnvironmentError> {
    let Some((name, digest)) = spec.reference.rsplit_once("@sha256:") else {
        return Err(EnvironmentError::InvalidImageBootstrapSpec(
            "source must be a registry name pinned with @sha256:<64 hex>".into(),
        ));
    };
    let registry = name.split('/').next().unwrap_or_default();
    if name.contains(char::is_whitespace)
        || name.contains(|character: char| character.is_control())
        || !name.contains('/')
        || !(registry.contains('.') || registry.contains(':') || registry == "localhost")
        || !is_hex_digest(digest)
    {
        return Err(EnvironmentError::InvalidImageBootstrapSpec(
            "source must be a fully-qualified registry reference pinned by sha256 digest".into(),
        ));
    }
    let Some((os, architecture)) = spec.platform.split_once('/') else {
        return Err(EnvironmentError::InvalidImageBootstrapSpec(
            "platform must be linux/<architecture>".into(),
        ));
    };
    if os != "linux"
        || architecture.is_empty()
        || architecture
            .chars()
            .any(|character| !character.is_ascii_alphanumeric() && character != '_')
    {
        return Err(EnvironmentError::InvalidImageBootstrapSpec(
            "platform must be an explicit Linux OS/architecture pair".into(),
        ));
    }
    Ok(())
}

/// Named identity payload: adding a field changes the shape and therefore
/// invalidates previously minted plan ids, unlike a positional tuple.
#[derive(Serialize)]
struct ImageBootstrapIdentity<'a> {
    backend_executable: &'a Path,
    backend_endpoint_id: &'a str,
    backend_command_prefix: &'a [String],
    source_reference: &'a str,
    platform: &'a str,
    disposition: ProvisioningDisposition,
    pull_required: bool,
    steps: &'a [ProvisioningStep],
    blockers: &'a [String],
}

fn image_bootstrap_plan_identity(plan: &PodmanImageBootstrapPlan) -> String {
    let identity = serde_json::to_vec(&ImageBootstrapIdentity {
        backend_executable: &plan.backend_executable,
        backend_endpoint_id: &plan.backend_endpoint_id,
        backend_command_prefix: &plan.backend_command_prefix,
        source_reference: &plan.source_reference,
        platform: &plan.platform,
        disposition: plan.disposition,
        pull_required: plan.pull_required,
        steps: &plan.steps,
        blockers: &plan.blockers,
    })
    .expect("serializable image bootstrap identity");
    format!("sha256:{:x}", Sha256::digest(identity))
}

/// Apply an unchanged image bootstrap preview and derive identity only from a
/// successful post-acquisition inspect.
///
/// # Errors
///
/// Returns [`EnvironmentError`] for a changed/blocked/unauthorized plan, a pull
/// failure, or image identity/platform mismatch.
pub fn apply_podman_image_bootstrap_plan(
    plan: &PodmanImageBootstrapPlan,
    authorization: &ProvisioningAuthorization,
) -> Result<PodmanImageBootstrapReceipt, EnvironmentError> {
    if image_bootstrap_plan_identity(plan) != plan.plan_id {
        return Err(EnvironmentError::PlanIdentityMismatch);
    }
    apply_podman_image_bootstrap_plan_with(plan, authorization, &SystemCommandRunner)
}

fn apply_podman_image_bootstrap_plan_with(
    plan: &PodmanImageBootstrapPlan,
    authorization: &ProvisioningAuthorization,
    runner: &dyn CommandRunner,
) -> Result<PodmanImageBootstrapReceipt, EnvironmentError> {
    if !plan.blockers.is_empty() {
        return Err(EnvironmentError::ProvisioningBlocked(plan.blockers.clone()));
    }
    match plan.disposition {
        ProvisioningDisposition::Never => return Err(EnvironmentError::ProvisioningDisabled),
        ProvisioningDisposition::Confirm => match authorization.confirmed_plan_id.as_deref() {
            None => return Err(EnvironmentError::ConfirmationRequired(plan.plan_id.clone())),
            Some(plan_id) if plan_id != plan.plan_id => {
                return Err(EnvironmentError::PlanIdentityMismatch);
            }
            Some(_) => {}
        },
        ProvisioningDisposition::Managed => return Err(EnvironmentError::ManagedPolicyRequired),
    }
    let mut inspection = None;
    for step in &plan.steps {
        let ProvisioningStep::Run {
            executable, args, ..
        } = step
        else {
            return Err(EnvironmentError::PlanIdentityMismatch);
        };
        let output = runner
            .output(executable, args)
            .map_err(EnvironmentError::CommandFailed)?;
        if !output.status.success() {
            return Err(EnvironmentError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            ));
        }
        if backend_args_start_with(args, &plan.backend_command_prefix, &["image", "inspect"]) {
            inspection = Some(output.stdout);
        }
    }
    let inspection = inspection.ok_or(EnvironmentError::PlanIdentityMismatch)?;
    verify_image_bootstrap_inspection(plan, &inspection)
}

fn verify_image_bootstrap_inspection(
    plan: &PodmanImageBootstrapPlan,
    bytes: &[u8],
) -> Result<PodmanImageBootstrapReceipt, EnvironmentError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|error| EnvironmentError::ImageInspectionFailed(error.to_string()))?;
    let image = value
        .as_array()
        .filter(|images| images.len() == 1)
        .and_then(|images| images.first())
        .ok_or_else(|| {
            EnvironmentError::ImageInspectionFailed("inspect must contain exactly one image".into())
        })?;
    let raw_id = get_any_value(image, &["Id", "ID"])
        .and_then(Value::as_str)
        .ok_or_else(|| EnvironmentError::ImageInspectionFailed("image ID is missing".into()))?;
    let id = raw_id.strip_prefix("sha256:").unwrap_or(raw_id);
    if !is_sha256_hex(id) {
        return Err(EnvironmentError::ImageInspectionFailed(
            "image ID is not a full sha256 identity".into(),
        ));
    }
    let expected_digest = plan
        .source_reference
        .rsplit_once('@')
        .map(|(_, digest)| digest)
        .ok_or_else(|| {
            EnvironmentError::ImageInspectionFailed("source digest is missing".into())
        })?;
    let image_digest = get_any_value(image, &["Digest"])
        .and_then(Value::as_str)
        .ok_or_else(|| EnvironmentError::ImageInspectionFailed("image digest is missing".into()))?;
    let repo_digests = get_any_value(image, &["RepoDigests"])
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    if image_digest != expected_digest
        && !repo_digests
            .iter()
            .any(|digest| *digest == plan.source_reference)
    {
        return Err(EnvironmentError::ImageInspectionFailed(
            "inspected image is not bound to the requested source digest".into(),
        ));
    }
    let (expected_os, expected_architecture) = plan
        .platform
        .split_once('/')
        .ok_or_else(|| EnvironmentError::ImageInspectionFailed("platform is invalid".into()))?;
    require_string_value(image, &["Os", "OS"], expected_os, "image OS")
        .map_err(|error| EnvironmentError::ImageInspectionFailed(error.to_string()))?;
    require_string_value(
        image,
        &["Architecture"],
        expected_architecture,
        "image architecture",
    )
    .map_err(|error| EnvironmentError::ImageInspectionFailed(error.to_string()))?;
    let size_bytes = get_any_value(image, &["Size"])
        .and_then(Value::as_u64)
        .ok_or_else(|| EnvironmentError::ImageInspectionFailed("image size is missing".into()))?;
    Ok(PodmanImageBootstrapReceipt {
        plan_id: plan.plan_id.clone(),
        source_reference: plan.source_reference.clone(),
        resolved_image: format!("sha256:{id}"),
        image_digest: image_digest.to_owned(),
        platform: plan.platform.clone(),
        size_bytes,
    })
}

/// Construct the exact Podman machine operations and host-impact preview.
///
/// # Errors
///
/// Returns [`EnvironmentError`] when the probe does not describe an installed
/// Podman backend. Failed prerequisite/resource checks become plan blockers.
#[allow(
    clippy::too_many_lines,
    reason = "the preview keeps each host effect adjacent to the exact argv and blocker it explains"
)]
pub fn podman_provisioning_plan(
    probe: &BackendProbe,
    options: &PodmanProvisioningOptions,
) -> Result<ProvisioningPlan, EnvironmentError> {
    if probe.backend_id != "podman" {
        return Err(EnvironmentError::BackendNotReady(probe.backend_id.clone()));
    }
    let executable = probe
        .executable
        .clone()
        .ok_or_else(|| EnvironmentError::BackendNotReady("podman is absent".into()))?;
    let backend_executable = executable.clone();
    let mut blockers = Vec::new();
    if probe.status == BackendProbeStatus::Incompatible {
        blockers.push("the discovered Podman CLI did not return a usable version".into());
    }
    if options.cpus == 0 || options.memory_mib < 512 || options.disk_gib < 1 {
        blockers
            .push("machine resources must be at least 1 CPU, 512 MiB RAM and 1 GiB disk".into());
    }
    if cfg!(windows) && !wsl_is_ready() {
        blockers.push(
            "WSL status is not ready; Podman machine needs a configured Linux VM provider".into(),
        );
    }
    if let Some(free) = free_space_bytes(&executable)
        && free < options.disk_gib.saturating_mul(1_073_741_824)
    {
        blockers.push(format!(
            "requested {} GiB machine disk exceeds {} GiB free on the Podman drive",
            options.disk_gib,
            free / 1_073_741_824
        ));
    }
    if let Some(total) = total_memory_bytes()
        && total < options.memory_mib.saturating_mul(1_048_576)
    {
        blockers.push(format!(
            "requested {} MiB machine memory exceeds {} MiB physical host memory",
            options.memory_mib,
            total / 1_048_576
        ));
    }
    let mut args = vec![
        "machine".into(),
        "init".into(),
        "--cpus".into(),
        options.cpus.to_string(),
        "--memory".into(),
        options.memory_mib.to_string(),
        "--disk-size".into(),
        options.disk_gib.to_string(),
        format!("--rootful={}", options.rootful),
    ];
    if cfg!(windows) {
        args.push(format!(
            "--user-mode-networking={}",
            options.user_mode_networking
        ));
    }
    args.push(options.machine_name.clone());
    let (machine_exists, machine_running) = named_machine_state(probe, &options.machine_name);
    let named_ready = machine_running && named_connection_exists(probe, &options.machine_name);
    let steps = if named_ready {
        vec![ProvisioningStep::VerifyBackend]
    } else if machine_exists {
        vec![
            ProvisioningStep::Run {
                executable,
                args: vec![
                    "machine".into(),
                    "start".into(),
                    options.machine_name.clone(),
                ],
                description: format!("start existing Podman machine {}", options.machine_name),
            },
            ProvisioningStep::VerifyBackend,
        ]
    } else {
        vec![
            ProvisioningStep::Run {
                executable: executable.clone(),
                args,
                description: format!("create Podman machine {}", options.machine_name),
            },
            ProvisioningStep::Run {
                executable,
                args: vec![
                    "machine".into(),
                    "start".into(),
                    options.machine_name.clone(),
                ],
                description: format!("start Podman machine {}", options.machine_name),
            },
            ProvisioningStep::VerifyBackend,
        ]
    };
    let effects = ProvisioningEffects {
        creates_virtual_machine: !machine_exists,
        may_download_machine_image: !machine_exists,
        requires_privilege_elevation: false,
        may_require_restart: cfg!(windows),
        cpu_count: options.cpus,
        memory_mib: options.memory_mib,
        disk_gib: options.disk_gib,
        rootful: options.rootful,
        user_mode_networking: options.user_mode_networking,
        changes_shared_wsl_networking: !machine_exists
            && cfg!(windows)
            && machine_provider(probe) == "wsl"
            && options.user_mode_networking,
        uses_provider_default_host_mounts: !machine_exists,
    };
    let idempotency_key = format!("podman-machine:{}", options.machine_name);
    let mut plan = ProvisioningPlan {
        plan_id: String::new(),
        backend_id: "podman".into(),
        backend_executable,
        backend_command_prefix: probe.command_prefix.clone(),
        provider: machine_provider(probe),
        source: "locally discovered Podman CLI; official machine init/start commands".into(),
        disposition: options.disposition,
        steps,
        effects,
        blockers,
        idempotency_key,
    };
    plan.plan_id = provisioning_plan_identity(&plan);
    Ok(plan)
}

/// Named identity payload: adding a field changes the shape and therefore
/// invalidates previously minted plan ids, unlike a positional tuple.
#[derive(Serialize)]
struct ProvisioningIdentity<'a> {
    backend_id: &'a str,
    backend_executable: &'a Path,
    backend_command_prefix: &'a [String],
    provider: &'a str,
    source: &'a str,
    disposition: ProvisioningDisposition,
    steps: &'a [ProvisioningStep],
    effects: &'a ProvisioningEffects,
    blockers: &'a [String],
    idempotency_key: &'a str,
}

fn provisioning_plan_identity(plan: &ProvisioningPlan) -> String {
    let identity_payload = serde_json::to_vec(&ProvisioningIdentity {
        backend_id: &plan.backend_id,
        backend_executable: &plan.backend_executable,
        backend_command_prefix: &plan.backend_command_prefix,
        provider: &plan.provider,
        source: &plan.source,
        disposition: plan.disposition,
        steps: &plan.steps,
        effects: &plan.effects,
        blockers: &plan.blockers,
        idempotency_key: &plan.idempotency_key,
    })
    .expect("serializable provisioning identity");
    format!("sha256:{:x}", Sha256::digest(identity_payload))
}

/// Execute an unchanged preview after authority has been established.
///
/// # Errors
///
/// Returns [`EnvironmentError`] for a changed, blocked or unauthorized plan, a
/// failed command, or a final probe which does not prove the named machine.
pub fn apply_podman_provisioning_plan(
    plan: &ProvisioningPlan,
    authorization: &ProvisioningAuthorization,
) -> Result<ProvisioningReceipt, EnvironmentError> {
    if provisioning_plan_identity(plan) != plan.plan_id {
        return Err(EnvironmentError::PlanIdentityMismatch);
    }
    apply_podman_provisioning_plan_with(plan, authorization, &SystemCommandRunner)
}

fn apply_podman_provisioning_plan_with(
    plan: &ProvisioningPlan,
    authorization: &ProvisioningAuthorization,
    runner: &dyn CommandRunner,
) -> Result<ProvisioningReceipt, EnvironmentError> {
    if !plan.blockers.is_empty() {
        return Err(EnvironmentError::ProvisioningBlocked(plan.blockers.clone()));
    }
    match plan.disposition {
        ProvisioningDisposition::Never => return Err(EnvironmentError::ProvisioningDisabled),
        ProvisioningDisposition::Confirm => match authorization.confirmed_plan_id.as_deref() {
            None => return Err(EnvironmentError::ConfirmationRequired(plan.plan_id.clone())),
            Some(identity) if identity != plan.plan_id => {
                return Err(EnvironmentError::PlanIdentityMismatch);
            }
            Some(_) => {}
        },
        ProvisioningDisposition::Managed => validate_managed_policy(
            plan,
            authorization
                .managed_policy
                .as_ref()
                .ok_or(EnvironmentError::ManagedPolicyRequired)?,
        )?,
    }
    let executable = Some(plan.backend_executable.clone());
    if !plan.backend_command_prefix.is_empty() {
        return Err(EnvironmentError::ManagedPolicyRejected(
            "Podman Machine provisioning is only valid for the native Podman client".into(),
        ));
    }
    let initial = probe_podman_with(executable.clone(), runner);
    let machine_name = plan
        .idempotency_key
        .strip_prefix("podman-machine:")
        .ok_or(EnvironmentError::PlanIdentityMismatch)?;
    let (_, initial_running) = named_machine_state(&initial, machine_name);
    if initial_running && named_connection_exists(&initial, machine_name) {
        return Ok(ProvisioningReceipt {
            plan_id: plan.plan_id.clone(),
            status: ProvisioningApplyStatus::AlreadyReady,
            executed_steps: 0,
            final_probe: initial,
        });
    }
    let mut executed_steps = 0;
    for step in &plan.steps {
        match step {
            ProvisioningStep::Run {
                executable, args, ..
            } => {
                let output = runner
                    .output(executable, args)
                    .map_err(EnvironmentError::CommandFailed)?;
                if !output.status.success() {
                    return Err(EnvironmentError::CommandFailed(
                        String::from_utf8_lossy(&output.stderr).trim().to_owned(),
                    ));
                }
                executed_steps += 1;
            }
            ProvisioningStep::VerifyBackend => {}
        }
    }
    let final_probe = probe_podman_with(executable, runner);
    let (_, final_running) = named_machine_state(&final_probe, machine_name);
    if final_probe.status != BackendProbeStatus::Ready
        || !final_running
        || !named_connection_exists(&final_probe, machine_name)
    {
        return Err(EnvironmentError::BackendNotReady(format!(
            "Podman remained {:?} after provisioning: {:?}",
            final_probe.status, final_probe.diagnostics
        )));
    }
    Ok(ProvisioningReceipt {
        plan_id: plan.plan_id.clone(),
        status: ProvisioningApplyStatus::Applied,
        executed_steps,
        final_probe,
    })
}

fn validate_managed_policy(
    plan: &ProvisioningPlan,
    policy: &ManagedProvisioningPolicy,
) -> Result<(), EnvironmentError> {
    if policy.backend_id != plan.backend_id {
        return Err(EnvironmentError::ManagedPolicyRejected(format!(
            "policy {} allows backend {}, not {}",
            policy.policy_id, policy.backend_id, plan.backend_id
        )));
    }
    let machine_name = plan
        .idempotency_key
        .strip_prefix("podman-machine:")
        .ok_or(EnvironmentError::PlanIdentityMismatch)?;
    if !policy.allowed_machine_names.contains(machine_name) {
        return Err(EnvironmentError::ManagedPolicyRejected(format!(
            "machine {machine_name} is outside policy {}",
            policy.policy_id
        )));
    }
    let effects = &plan.effects;
    if effects.cpu_count > policy.max_cpus
        || effects.memory_mib > policy.max_memory_mib
        || effects.disk_gib > policy.max_disk_gib
    {
        return Err(EnvironmentError::ManagedPolicyRejected(format!(
            "resource request exceeds policy {} ceilings",
            policy.policy_id
        )));
    }
    if effects.rootful && !policy.allow_rootful {
        return Err(EnvironmentError::ManagedPolicyRejected(format!(
            "rootful machine is forbidden by policy {}",
            policy.policy_id
        )));
    }
    if effects.user_mode_networking && !policy.allow_user_mode_networking {
        return Err(EnvironmentError::ManagedPolicyRejected(format!(
            "user-mode networking is forbidden by policy {}",
            policy.policy_id
        )));
    }
    Ok(())
}

trait CommandRunner {
    fn output(&self, executable: &Path, args: &[String]) -> Result<Output, String>;
}

fn run_backend_command(
    runner: &dyn CommandRunner,
    executable: &Path,
    command_prefix: &[String],
    args: &[String],
) -> Result<Output, String> {
    let command = prefixed_backend_args(command_prefix, args);
    runner.output(executable, &command)
}

fn prefixed_backend_args(command_prefix: &[String], args: &[String]) -> Vec<String> {
    let mut command = Vec::with_capacity(command_prefix.len() + args.len());
    command.extend_from_slice(command_prefix);
    command.extend_from_slice(args);
    command
}

fn backend_args_start_with(args: &[String], prefix: &[String], operation: &[&str]) -> bool {
    args.get(prefix.len()..).is_some_and(|tail| {
        tail.len() >= operation.len()
            && tail
                .iter()
                .take(operation.len())
                .map(String::as_str)
                .eq(operation.iter().copied())
    })
}

struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn output(&self, executable: &Path, args: &[String]) -> Result<Output, String> {
        Command::new(executable)
            .args(args)
            .output()
            .map_err(|error| format!("{}: {error}", executable.display()))
    }
}

pub(crate) fn find_executable(name: &str) -> Option<PathBuf> {
    let candidates = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), name.into()]
    } else {
        vec![name.into()]
    };
    env::var_os("PATH")
        .into_iter()
        .flat_map(|path| env::split_paths(&path).collect::<Vec<_>>())
        .flat_map(|directory| {
            candidates
                .iter()
                .map(move |candidate| directory.join(candidate))
        })
        .find(|candidate| candidate.is_file())
        .or_else(|| {
            cfg!(windows)
                .then(|| {
                    PathBuf::from(r"C:\Program Files\RedHat\Podman").join(format!("{name}.exe"))
                })
                .filter(|candidate| candidate.is_file())
        })
}

fn free_space_bytes(path: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        let drive = path.components().next()?.as_os_str().to_string_lossy();
        let drive = drive.trim_end_matches(['\\', ':']);
        let script = format!("[Console]::Write((Get-PSDrive -Name '{drive}').Free)");
        let output = Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .ok()?;
        output.status.success().then_some(())?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

fn wsl_is_ready() -> bool {
    #[cfg(windows)]
    {
        let Some(wsl) = find_executable("wsl") else {
            return false;
        };
        Command::new(wsl)
            .arg("--status")
            .output()
            .is_ok_and(|output| output.status.success())
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn total_memory_bytes() -> Option<u64> {
    #[cfg(windows)]
    {
        let output = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[Console]::Write((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory)",
            ])
            .output()
            .ok()?;
        output.status.success().then_some(())?;
        String::from_utf8_lossy(&output.stdout).trim().parse().ok()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
        ready: Mutex<bool>,
        inspection: Mutex<Option<Value>>,
        container_exists: Mutex<bool>,
        create_stdout: Mutex<Option<Vec<u8>>>,
        image_exists: Mutex<bool>,
        image_inspection: Mutex<Option<Value>>,
        orphan_inventory: Mutex<Option<Value>>,
    }

    impl CommandRunner for FakeRunner {
        fn output(&self, _executable: &Path, args: &[String]) -> Result<Output, String> {
            self.calls.lock().unwrap().push(args.to_vec());
            let operation = args
                .iter()
                .rposition(|argument| argument == "podman")
                .and_then(|index| args.get(index + 1..))
                .unwrap_or(args);
            let text = operation.join(" ");
            if text.ends_with("--version") {
                return Ok(success(b"podman version 5.6.0\n"));
            }
            if text.ends_with("info --format json") {
                return Ok(if *self.ready.lock().unwrap() {
                    success(br#"{"host":{"security":{"rootless":true}}}"#)
                } else {
                    exit_output(125)
                });
            }
            if text.contains("machine list") {
                let ready = *self.ready.lock().unwrap();
                return Ok(success(if ready {
                    br#"[{"Name":"swem","Running":true}]"#
                } else {
                    b"[]"
                }));
            }
            if text.contains("connection list") {
                let ready = *self.ready.lock().unwrap();
                return Ok(success(if ready {
                    br#"[{"Name":"swem"}]"#
                } else {
                    b"[]"
                }));
            }
            if text.contains("machine info") {
                return Ok(success(br#"{"Host":{"VMType":"wsl"}}"#));
            }
            if text.starts_with("machine start") {
                *self.ready.lock().unwrap() = true;
            }
            if text.starts_with("create ") {
                *self.container_exists.lock().unwrap() = true;
                let stdout = self
                    .create_stdout
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| {
                        b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\n"
                            .to_vec()
                    });
                return Ok(success(&stdout));
            }
            if text.starts_with("container inspect ") {
                let value = self
                    .inspection
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                return Ok(success(&serde_json::to_vec(&value).unwrap()));
            }
            if text.starts_with("rm --force --ignore ") {
                *self.container_exists.lock().unwrap() = false;
                return Ok(success(b""));
            }
            if text.starts_with("container exists ") {
                return Ok(exit_output(i32::from(
                    !*self.container_exists.lock().unwrap(),
                )));
            }
            if text.starts_with("image exists ") {
                return Ok(exit_output(i32::from(!*self.image_exists.lock().unwrap())));
            }
            if text.starts_with("pull ") {
                *self.image_exists.lock().unwrap() = true;
                return Ok(success(
                    b"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
                ));
            }
            if text.starts_with("image inspect ") {
                let value = self
                    .image_inspection
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                return Ok(success(&serde_json::to_vec(&value).unwrap()));
            }
            if text.starts_with("ps --all --filter label=io.swem.managed=true") {
                let value = self
                    .orphan_inventory
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| Value::Array(Vec::new()));
                return Ok(success(&serde_json::to_vec(&value).unwrap()));
            }
            Ok(success(b""))
        }
    }

    #[cfg(unix)]
    fn success(stdout: &[u8]) -> Output {
        use std::os::unix::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.to_vec(),
            stderr: vec![],
        }
    }

    #[cfg(unix)]
    fn exit_output(code: i32) -> Output {
        use std::os::unix::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(code << 8),
            stdout: vec![],
            stderr: vec![],
        }
    }

    #[cfg(windows)]
    fn success(stdout: &[u8]) -> Output {
        use std::os::windows::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.to_vec(),
            stderr: vec![],
        }
    }

    #[cfg(windows)]
    fn exit_output(code: i32) -> Output {
        use std::os::windows::process::ExitStatusExt;
        Output {
            status: std::process::ExitStatus::from_raw(code.cast_unsigned()),
            stdout: vec![],
            stderr: vec![],
        }
    }

    #[test]
    fn direct_backend_never_satisfies_isolation_by_fallback() {
        let root = std::env::current_dir().unwrap();
        let mut requirements = EnvironmentRequirements::new(root);
        requirements
            .required_guarantees
            .insert(EnvironmentGuarantee::AgentProcessIsolation);
        let direct = BackendProbe::direct();
        assert_eq!(
            resolve_backend(&requirements, [&direct]),
            Err(EnvironmentError::MissingGuarantees {
                backend_id: "direct-process".into(),
                missing: vec![EnvironmentGuarantee::AgentProcessIsolation],
            })
        );
    }

    #[test]
    fn direct_lease_claims_are_exactly_its_evidence_keys() {
        let root = std::env::current_dir().unwrap();
        let lease =
            direct_environment_lease("direct-evidence", &EnvironmentRequirements::new(root))
                .expect("direct lease");
        assert_eq!(
            lease.guarantees(),
            BTreeSet::from([
                EnvironmentGuarantee::CanonicalWorkspaceBinding,
                EnvironmentGuarantee::AmbientEnvironmentFiltering,
                EnvironmentGuarantee::ProcessTreeCleanup,
            ])
        );
        assert!(lease.evidence.values().all(|evidence| {
            evidence.kind == EnvironmentEvidenceKind::EnforcedConfiguration
                && evidence.source.starts_with("swem-host/direct-process@")
        }));
        let serialized = serde_json::to_value(&lease).expect("serialize lease");
        assert!(serialized.get("evidence").is_some());
        assert!(serialized.get("guarantees").is_none());
    }

    #[test]
    fn workspace_file_uri_resolution_uses_only_the_proven_namespace_binding() {
        let root = std::env::current_dir()
            .unwrap()
            .canonicalize()
            .expect("canonical fixture workspace");
        let direct = direct_environment_lease("direct-uri", &EnvironmentRequirements::new(&root))
            .expect("direct lease");
        let host_file = root.join("nested file.bin");
        let host_uri = Url::from_file_path(&host_file)
            .expect("host file URI")
            .to_string();
        let host_uri_path = Url::parse(&host_uri)
            .expect("parse host file URI")
            .to_file_path()
            .expect("host URI path");
        assert_eq!(
            direct
                .resolve_workspace_file_uri(&host_uri)
                .expect("resolve direct URI"),
            host_uri_path
        );

        let mut namespaced = direct;
        namespaced.agent_workspace = PathBuf::from("/workspace");
        assert_eq!(
            namespaced
                .resolve_workspace_file_uri("file:///workspace/sub/a%20b.bin")
                .expect("resolve agent namespace URI"),
            root.join("sub").join("a b.bin")
        );
        for unavailable in [
            "file:///outside/output.bin",
            "file:///workspace/%2e%2e/output.bin",
            "https://example.invalid/workspace/output.bin",
            "file://example.invalid/workspace/output.bin",
            "file:///workspace/output.bin?revision=1",
        ] {
            assert!(
                matches!(
                    namespaced.resolve_workspace_file_uri(unavailable),
                    Err(EnvironmentError::WorkspaceResourceUnavailable(_))
                ),
                "unexpectedly resolved {unavailable}"
            );
        }
    }

    #[test]
    fn podman_probe_distinguishes_installed_from_ready() {
        let runner = FakeRunner::default();
        let executable = PathBuf::from("podman");
        let installed = probe_podman_with(Some(executable.clone()), &runner);
        assert_eq!(installed.status, BackendProbeStatus::InstalledNotReady);
        *runner.ready.lock().unwrap() = true;
        let ready = probe_podman_with(Some(executable), &runner);
        assert_eq!(ready.status, BackendProbeStatus::Ready);
        assert!(
            ready
                .available_guarantees
                .contains(&EnvironmentGuarantee::NetworkDenyByDefault)
        );
        assert!(
            ready
                .limitations
                .iter()
                .any(|limitation| limitation.contains("not lease enforcement evidence"))
        );
    }

    #[test]
    fn local_engine_readiness_does_not_require_machine_topology() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let prefix = vec![
            "--distribution".into(),
            "Ubuntu".into(),
            "--".into(),
            "podman".into(),
        ];
        let probe = probe_podman_endpoint_with(
            Some(PathBuf::from("wsl.exe")),
            prefix.clone(),
            "podman:wsl:Ubuntu".into(),
            serde_json::json!({"kind": "wsl_local_podman"}),
            false,
            &runner,
        );
        assert_eq!(probe.status, BackendProbeStatus::Ready);
        assert_eq!(probe.endpoint_id, "podman:wsl:Ubuntu");
        assert_eq!(probe.command_prefix, prefix);
        assert!(probe.topology["machines"].is_null());
        assert!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|args| { !args.windows(2).any(|pair| pair == ["machine", "list"]) })
        );
    }

    #[test]
    fn managed_podman_machine_distribution_is_not_a_second_endpoint() {
        let mut probe = BackendProbe::direct();
        probe.topology = serde_json::json!({
            "machines": [
                {"Name": "swem", "Running": true},
                {"name": "Research", "running": false}
            ]
        });
        assert_eq!(
            podman_machine_wsl_distributions(&probe),
            BTreeSet::from(["podman-research".into(), "podman-swem".into()])
        );
    }

    fn podman_spec() -> PodmanContainerSpec {
        PodmanContainerSpec {
            image: "example.invalid/swem/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            workspace: PodmanWorkspaceBinding {
                service_source: "/mnt/c/work/swem".into(),
                container_target: "/workspace".into(),
            },
            agent_home: None,
            runtime_secrets: Vec::new(),
            environment: BTreeMap::new(),
            network: PodmanNetworkPolicy::DenyAll,
            agent_executable: "/opt/swem/bin/fixture-agent".into(),
            agent_args: vec!["acp".into()],
            user: "1000:1000".into(),
            cpu_limit: 2,
            memory_mib: 2_048,
            pids_limit: 256,
        }
    }

    fn valid_podman_inspection(spec: &PodmanContainerSpec, lease_id: &str) -> Value {
        let mut mounts = vec![serde_json::json!({
            "Type": "bind",
            "Source": spec.workspace.service_source,
            "Destination": spec.workspace.container_target,
            "RW": true,
            "Options": ["nodev", "nosuid"]
        })];
        if let Some(agent_home) = &spec.agent_home {
            mounts.push(serde_json::json!({
                "Type": "bind",
                "Source": agent_home.service_source,
                "Destination": agent_home.container_target,
                "RW": true,
                "Options": ["nodev", "nosuid"]
            }));
        }
        let mut create_command = vec!["podman".to_owned()];
        create_command.extend(podman_create_args(lease_id, spec).expect("valid fixture create"));
        serde_json::json!([{
            "Id": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "Name": podman_container_name(lease_id),
            "Image": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "ImageDigest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "State": {"Status": "created", "Running": false},
            "EffectiveCaps": [],
            "Config": {
                "User": spec.user,
                "AttachStdin": true,
                "AttachStdout": true,
                "AttachStderr": true,
                "Tty": false,
                "OpenStdin": true,
                "Env": [
                    "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
                    "HOME=/home/swem",
                    "LANG=C.UTF-8",
                    "HOSTNAME=0123456789ab",
                    "container=podman"
                ],
                "CreateCommand": create_command,
                "Cmd": spec.agent_args,
                "WorkingDir": spec.workspace.container_target,
                "Entrypoint": [spec.agent_executable],
                "Labels": {
                    "io.swem.lease-id": lease_id,
                    "io.swem.managed": "true"
                }
            },
            "HostConfig": {
                "NetworkMode": spec.network.podman_mode(),
                "PortBindings": {},
                "IpcMode": "private",
                "PidMode": "private",
                "CgroupMode": "private",
                "ReadonlyRootfs": true,
                "Memory": 2_147_483_648_u64,
                "NanoCpus": 2_000_000_000_u64,
                "PidsLimit": 256,
                "CapDrop": ["CAP_ALL"],
                "SecurityOpt": ["no-new-privileges"]
            },
            "Mounts": mounts
        }])
    }

    fn image_bootstrap_spec() -> PodmanImageBootstrapSpec {
        PodmanImageBootstrapSpec {
            reference: "registry.example/swem/acp-fixture@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            platform: "linux/amd64".into(),
            disposition: ProvisioningDisposition::Confirm,
        }
    }

    fn valid_image_inspection(spec: &PodmanImageBootstrapSpec) -> Value {
        serde_json::json!([{
            "Id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "Digest": "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "RepoDigests": [spec.reference],
            "Os": "linux",
            "Architecture": "amd64",
            "Size": 12_345_u64
        }])
    }

    #[test]
    fn prepared_podman_lease_is_derived_from_inspection_and_cleans_by_exact_id() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let spec = podman_spec();
        let lease_id = "isolated-session";
        *runner.inspection.lock().unwrap() = Some(valid_podman_inspection(&spec, lease_id));
        let root = std::env::current_dir().unwrap();
        let mut requirements = EnvironmentRequirements::new(root);
        requirements.cpu_limit = Some(2);
        requirements.memory_mib = Some(2_048);
        requirements.required_guarantees.extend([
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuarantee::FilesystemIsolation,
            EnvironmentGuarantee::ResourceLimits,
            EnvironmentGuarantee::NetworkDenyByDefault,
        ]);

        let lease =
            prepare_podman_lease_with(&probe, lease_id.into(), &requirements, &spec, &runner)
                .expect("prepare inspected Podman lease");
        assert_eq!(lease.instance_id.len(), 64);
        assert_eq!(lease.agent_workspace, PathBuf::from("/workspace"));
        assert_eq!(
            lease.workspace,
            requirements.workspace.canonicalize().unwrap()
        );
        assert_eq!(lease.guarantees(), lease.evidence.keys().copied().collect());
        assert_eq!(
            lease.evidence[&EnvironmentGuarantee::NetworkDenyByDefault].kind,
            EnvironmentEvidenceKind::BackendInspection
        );
        let create = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|args| args.first().is_some_and(|arg| arg == "create"))
            .cloned()
            .expect("create call");
        assert!(create.contains(&"--pull=never".into()));
        assert!(create.contains(&"--network=none".into()));
        assert!(create.contains(&"--unsetenv-all".into()));
        assert!(!create.iter().any(|argument| argument == "--privileged"));

        let cleanup =
            cleanup_podman_instance_with(Path::new("podman"), &[], &lease.instance_id, &runner)
                .expect("exact cleanup");
        assert_eq!(cleanup.kind, EnvironmentEvidenceKind::CleanupObservation);
        assert_eq!(cleanup.details["exists"], false);
    }

    #[test]
    fn online_agent_lease_keeps_home_secret_and_network_authority_explicit() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let mut spec = podman_spec();
        spec.agent_home = Some(PodmanWorkspaceBinding {
            service_source: "/mnt/c/swem/agents/claude/home".into(),
            container_target: "/home/swem".into(),
        });
        spec.runtime_secrets = vec![PodmanRuntimeSecret {
            opaque_reference: "swem-claude-token-7f4a".into(),
            target_environment: "CLAUDE_CODE_OAUTH_TOKEN".into(),
        }];
        spec.network = PodmanNetworkPolicy::PrivateEgress;
        let lease_id = "online-native-agent";
        let mut inspection = valid_podman_inspection(&spec, lease_id);
        inspection[0]["HostConfig"]["NetworkMode"] = Value::String("pasta".into());
        *runner.inspection.lock().unwrap() = Some(inspection);
        let root = std::env::current_dir().unwrap();
        let mut requirements = EnvironmentRequirements::new(root);
        requirements.persistence = WorkspacePersistence::Persistent;
        requirements.cpu_limit = Some(2);
        requirements.memory_mib = Some(2_048);
        requirements.required_guarantees.extend([
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuarantee::FilesystemIsolation,
            EnvironmentGuarantee::ResourceLimits,
            EnvironmentGuarantee::PersistentWorkspace,
        ]);

        let lease =
            prepare_podman_lease_with(&probe, lease_id.into(), &requirements, &spec, &runner)
                .expect("prepare online native-agent lease");
        assert!(
            !lease
                .guarantees()
                .contains(&EnvironmentGuarantee::NetworkDenyByDefault),
            "private egress must never be promoted to deny-by-default"
        );
        assert!(
            lease
                .guarantees()
                .contains(&EnvironmentGuarantee::PersistentWorkspace)
        );
        let create = runner
            .calls
            .lock()
            .unwrap()
            .iter()
            .find(|args| args.first().is_some_and(|arg| arg == "create"))
            .cloned()
            .expect("create call");
        assert!(create.contains(&"--network=private".into()));
        assert!(create.windows(2).any(|pair| {
            pair == [
                "--secret",
                "swem-claude-token-7f4a,type=env,target=CLAUDE_CODE_OAUTH_TOKEN",
            ]
        }));
        assert!(create.iter().any(|argument| {
            argument
                == "type=bind,src=/mnt/c/swem/agents/claude/home,dst=/home/swem,rw=true,nodev,nosuid"
        }));
        let serialized = serde_json::to_value(&spec).expect("serialize connection-local spec");
        assert_eq!(
            serialized["runtime_secrets"][0]["opaque_reference"],
            "swem-claude-token-7f4a"
        );
        assert!(serialized["runtime_secrets"][0].get("value").is_none());
    }

    #[test]
    fn inspection_mismatch_fails_closed_and_removes_created_container() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let spec = podman_spec();
        let mut inspection = valid_podman_inspection(&spec, "tampered");
        inspection[0]["HostConfig"]["NetworkMode"] = Value::String("bridge".into());
        *runner.inspection.lock().unwrap() = Some(inspection);
        let requirements = EnvironmentRequirements::new(std::env::current_dir().unwrap());
        let error =
            prepare_podman_lease_with(&probe, "tampered".into(), &requirements, &spec, &runner)
                .expect_err("network mismatch must block lease");
        assert!(matches!(
            error,
            EnvironmentError::ContainerInspectionFailed(_)
        ));
        assert!(!*runner.container_exists.lock().unwrap());
        assert!(
            runner
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|args| args.first().is_some_and(|arg| arg == "rm"))
        );
    }

    #[test]
    fn private_egress_never_accepts_published_host_ports() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let mut spec = podman_spec();
        spec.network = PodmanNetworkPolicy::PrivateEgress;
        let mut inspection = valid_podman_inspection(&spec, "published-port");
        inspection[0]["HostConfig"]["NetworkMode"] = Value::String("pasta".into());
        inspection[0]["HostConfig"]["PortBindings"] = serde_json::json!({
            "8080/tcp": [{"HostIp": "127.0.0.1", "HostPort": "8080"}]
        });
        *runner.inspection.lock().unwrap() = Some(inspection);
        let root = std::env::current_dir().unwrap();
        let requirements = EnvironmentRequirements::new(root);
        let error = prepare_podman_lease_with(
            &probe,
            "published-port".into(),
            &requirements,
            &spec,
            &runner,
        )
        .expect_err("published port must fail closed");
        assert!(matches!(
            error,
            EnvironmentError::ContainerInspectionFailed(message)
                if message.contains("publishes host ports")
        ));
    }

    #[test]
    fn malformed_create_identity_cleans_by_predeclared_exact_name() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        *runner.create_stdout.lock().unwrap() = Some(b"not-a-container-id\n".to_vec());
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let requirements = EnvironmentRequirements::new(std::env::current_dir().unwrap());
        let error = prepare_podman_lease_with(
            &probe,
            "malformed-output".into(),
            &requirements,
            &podman_spec(),
            &runner,
        )
        .expect_err("malformed create output must fail closed");
        assert!(matches!(
            error,
            EnvironmentError::InvalidContainerIdentity(_)
        ));
        assert!(!*runner.container_exists.lock().unwrap());
        let expected_name = podman_container_name("malformed-output");
        assert!(runner.calls.lock().unwrap().iter().any(|args| {
            args.first().is_some_and(|arg| arg == "rm") && args.last() == Some(&expected_name)
        }));
    }

    #[test]
    fn image_bootstrap_preserves_exact_endpoint_command_prefix() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let prefix = vec![
            "--distribution".into(),
            "Ubuntu".into(),
            "--".into(),
            "podman".into(),
        ];
        let probe = probe_podman_endpoint_with(
            Some(PathBuf::from("wsl.exe")),
            prefix.clone(),
            "podman:wsl:Ubuntu".into(),
            serde_json::json!({"kind": "wsl_local_podman"}),
            false,
            &runner,
        );
        let spec = image_bootstrap_spec();
        *runner.image_inspection.lock().unwrap() = Some(valid_image_inspection(&spec));
        let plan = podman_image_bootstrap_plan_with(&probe, &spec, &runner)
            .expect("preview prefixed endpoint image");
        assert_eq!(plan.backend_endpoint_id, probe.endpoint_id);
        assert_eq!(plan.backend_command_prefix, prefix);
        assert!(plan.steps.iter().all(|step| match step {
            ProvisioningStep::Run { args, .. } => args.starts_with(&prefix),
            ProvisioningStep::VerifyBackend => false,
        }));
        let receipt = apply_podman_image_bootstrap_plan_with(
            &plan,
            &ProvisioningAuthorization::confirmed(&plan.plan_id),
            &runner,
        )
        .expect("apply prefixed endpoint image");
        assert_eq!(receipt.source_reference, spec.reference);
    }

    #[test]
    fn runtime_create_rejects_tags_disk_quotas_and_implicit_client_paths() {
        let root = std::env::current_dir().unwrap();
        let mut requirements = EnvironmentRequirements::new(root);
        let mut spec = podman_spec();
        spec.image = "example.invalid/swem/agent:latest".into();
        assert!(matches!(
            validate_podman_spec("lease", &requirements, &spec),
            Err(EnvironmentError::InvalidContainerSpec(_))
        ));
        spec = podman_spec();
        spec.workspace.service_source = r"C:\work\swem".into();
        assert!(matches!(
            validate_podman_spec("lease", &requirements, &spec),
            Err(EnvironmentError::InvalidContainerSpec(_))
        ));
        spec = podman_spec();
        requirements.disk_gib = Some(10);
        assert!(matches!(
            validate_podman_spec("lease", &requirements, &spec),
            Err(EnvironmentError::InvalidContainerSpec(_))
        ));
    }

    #[test]
    fn pinned_image_bootstrap_requires_exact_authority_and_post_pull_inspection() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let spec = image_bootstrap_spec();
        *runner.image_inspection.lock().unwrap() = Some(valid_image_inspection(&spec));
        let plan = podman_image_bootstrap_plan_with(&probe, &spec, &runner)
            .expect("preview missing pinned image");
        assert!(plan.pull_required);
        assert!(plan.steps.iter().any(|step| matches!(
            step,
            ProvisioningStep::Run { args, .. }
                if args.contains(&"--policy=always".into())
                    && args.contains(&"--tls-verify=true".into())
                    && args.contains(&spec.reference)
        )));
        assert!(matches!(
            apply_podman_image_bootstrap_plan_with(
                &plan,
                &ProvisioningAuthorization::none(),
                &runner
            ),
            Err(EnvironmentError::ConfirmationRequired(_))
        ));
        let receipt = apply_podman_image_bootstrap_plan_with(
            &plan,
            &ProvisioningAuthorization::confirmed(&plan.plan_id),
            &runner,
        )
        .expect("authorized pull and inspect");
        assert_eq!(
            receipt.resolved_image,
            "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
        assert_eq!(receipt.source_reference, spec.reference);
    }

    #[test]
    fn existing_image_skips_network_and_digest_mismatch_fails_closed() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        *runner.image_exists.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let spec = image_bootstrap_spec();
        let mut inspection = valid_image_inspection(&spec);
        inspection[0]["Digest"] = Value::String(
            "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
        );
        inspection[0]["RepoDigests"] = Value::Array(Vec::new());
        *runner.image_inspection.lock().unwrap() = Some(inspection);
        let plan = podman_image_bootstrap_plan_with(&probe, &spec, &runner)
            .expect("preview existing image");
        assert!(!plan.pull_required);
        assert!(!plan.steps.iter().any(|step| matches!(
            step,
            ProvisioningStep::Run { args, .. }
                if args.first().is_some_and(|argument| argument == "pull")
        )));
        let error = apply_podman_image_bootstrap_plan_with(
            &plan,
            &ProvisioningAuthorization::confirmed(&plan.plan_id),
            &runner,
        )
        .expect_err("digest mismatch must not become a receipt");
        assert!(matches!(error, EnvironmentError::ImageInspectionFailed(_)));
    }

    #[test]
    fn orphan_reconciliation_requires_exact_ownership_inventory_and_confirmation() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        *runner.container_exists.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let lease_id = "crashed-host-lease";
        let container_id = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        *runner.orphan_inventory.lock().unwrap() = Some(serde_json::json!([{
            "Id": container_id,
            "Names": [podman_container_name(lease_id)],
            "State": "running",
            "Labels": {
                "io.swem.managed": "true",
                "io.swem.lease-id": lease_id
            }
        }]));
        let plan = podman_orphan_reconciliation_plan_with(&probe, BTreeSet::new(), &runner)
            .expect("preview exact orphan");
        assert!(plan.blockers.is_empty());
        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].container_id, container_id);
        assert!(matches!(
            apply_podman_orphan_reconciliation_plan_with(
                &plan,
                &ProvisioningAuthorization::none(),
                &runner
            ),
            Err(EnvironmentError::ConfirmationRequired(_))
        ));
        let receipt = apply_podman_orphan_reconciliation_plan_with(
            &plan,
            &ProvisioningAuthorization::confirmed(&plan.plan_id),
            &runner,
        )
        .expect("remove exact orphan");
        assert_eq!(receipt.removed, plan.candidates);
        assert_eq!(receipt.cleanup_evidence.len(), 1);

        let malformed = FakeRunner::default();
        *malformed.ready.lock().unwrap() = true;
        let malformed_probe = probe_podman_with(Some(PathBuf::from("podman")), &malformed);
        *malformed.orphan_inventory.lock().unwrap() = Some(serde_json::json!([{
            "Id": container_id,
            "Names": ["lookalike"],
            "Labels": {
                "io.swem.managed": "true",
                "io.swem.lease-id": lease_id
            }
        }]));
        let blocked =
            podman_orphan_reconciliation_plan_with(&malformed_probe, BTreeSet::new(), &malformed)
                .expect("preview malformed managed inventory");
        assert!(blocked.candidates.is_empty());
        assert_eq!(blocked.blockers.len(), 1);
    }

    #[test]
    fn provisioning_requires_confirmation_of_the_exact_preview() {
        let runner = FakeRunner::default();
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let options = PodmanProvisioningOptions {
            disk_gib: 1,
            ..PodmanProvisioningOptions::default()
        };
        let mut plan = podman_provisioning_plan(&probe, &options).unwrap();
        // Unit tests do not depend on the developer host's disk/WSL state.
        plan.blockers.clear();
        plan.plan_id = provisioning_plan_identity(&plan);
        assert!(matches!(
            apply_podman_provisioning_plan_with(&plan, &ProvisioningAuthorization::none(), &runner),
            Err(EnvironmentError::ConfirmationRequired(_))
        ));
        assert_eq!(
            apply_podman_provisioning_plan_with(
                &plan,
                &ProvisioningAuthorization::confirmed("sha256:other"),
                &runner
            ),
            Err(EnvironmentError::PlanIdentityMismatch)
        );
        let receipt = apply_podman_provisioning_plan_with(
            &plan,
            &ProvisioningAuthorization::confirmed(plan.plan_id.clone()),
            &runner,
        )
        .unwrap();
        assert_eq!(receipt.status, ProvisioningApplyStatus::Applied);
        assert_eq!(receipt.executed_steps, 2);
        assert_eq!(receipt.final_probe.status, BackendProbeStatus::Ready);
    }

    #[test]
    fn managed_apply_is_idempotent_when_backend_is_already_ready() {
        let runner = FakeRunner::default();
        *runner.ready.lock().unwrap() = true;
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let options = PodmanProvisioningOptions {
            disposition: ProvisioningDisposition::Managed,
            disk_gib: 1,
            ..PodmanProvisioningOptions::default()
        };
        let mut plan = podman_provisioning_plan(&probe, &options).unwrap();
        plan.blockers.clear();
        plan.plan_id = provisioning_plan_identity(&plan);
        let policy = ManagedProvisioningPolicy {
            policy_id: "desktop-managed".into(),
            backend_id: "podman".into(),
            allowed_machine_names: BTreeSet::from(["swem".into()]),
            max_cpus: 2,
            max_memory_mib: 2_048,
            max_disk_gib: 1,
            allow_rootful: false,
            allow_user_mode_networking: cfg!(windows),
        };
        let receipt = apply_podman_provisioning_plan_with(
            &plan,
            &ProvisioningAuthorization::managed(policy),
            &runner,
        )
        .unwrap();
        assert_eq!(receipt.status, ProvisioningApplyStatus::AlreadyReady);
        assert_eq!(receipt.executed_steps, 0);
    }

    #[test]
    fn managed_label_without_policy_is_not_authority() {
        let runner = FakeRunner::default();
        let probe = probe_podman_with(Some(PathBuf::from("podman")), &runner);
        let options = PodmanProvisioningOptions {
            disposition: ProvisioningDisposition::Managed,
            disk_gib: 1,
            ..PodmanProvisioningOptions::default()
        };
        let mut plan = podman_provisioning_plan(&probe, &options).unwrap();
        plan.blockers.clear();
        plan.plan_id = provisioning_plan_identity(&plan);
        assert_eq!(
            apply_podman_provisioning_plan_with(&plan, &ProvisioningAuthorization::none(), &runner),
            Err(EnvironmentError::ManagedPolicyRequired)
        );
    }

    #[test]
    fn podman_invocations_share_one_engine_selection_environment() {
        let invocation = podman_invocation_environment();
        let bootstrap = direct_bootstrap_environment();
        for (name, value) in &bootstrap {
            assert_eq!(
                invocation.get(name),
                Some(value),
                "transport dropped bootstrap variable {name}"
            );
        }
        for name in PODMAN_ENGINE_SELECTION_NAMES {
            match env::var(name) {
                Ok(ambient) => assert_eq!(
                    invocation.get(*name).map(String::as_str),
                    Some(ambient.as_str()),
                    "engine selection variable {name} differs from what discovery saw"
                ),
                Err(_) => assert!(
                    !invocation.contains_key(*name) || bootstrap.contains_key(*name),
                    "engine selection variable {name} was invented"
                ),
            }
        }
    }

    /// The model and the provider's address ride into the container as plain
    /// `--env` variables, and the check afterwards holds the container to the
    /// baseline plus exactly those: a different value or a missing one is a
    /// container that is not the one the spec described.
    #[test]
    fn declared_plain_variables_are_emitted_and_checked_exactly() {
        let mut spec = PodmanContainerSpec::deny_network(
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            PodmanWorkspaceBinding {
                service_source: "/tmp/w".into(),
                container_target: "/workspace".into(),
            },
            "/usr/local/bin/agent",
        );
        spec.user = "1000:1000".into();
        spec.environment
            .insert("ANTHROPIC_MODEL".into(), "quality".into());
        let args = podman_create_args("lease-1", &spec).expect("valid create");
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--env", "ANTHROPIC_MODEL=quality"]),
            "{args:?}"
        );

        let config = |model: Option<&str>| {
            let mut env = vec![
                "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_owned(),
                "HOME=/home/swem".to_owned(),
                "LANG=C.UTF-8".to_owned(),
                "HOSTNAME=0123456789ab".to_owned(),
                "container=podman".to_owned(),
            ];
            if let Some(model) = model {
                env.push(format!("ANTHROPIC_MODEL={model}"));
            }
            let value = serde_json::json!({ "Env": env });
            value.as_object().cloned().expect("object")
        };
        verify_environment(&config(Some("quality")), &spec).expect("the declared value passes");
        assert!(verify_environment(&config(Some("fast")), &spec).is_err());
        assert!(verify_environment(&config(None), &spec).is_err());
        let mut stranger = config(Some("quality"));
        stranger["Env"]
            .as_array_mut()
            .expect("array")
            .push(serde_json::json!("SOMETHING_ELSE=1"));
        assert!(verify_environment(&stranger, &spec).is_err());
    }
}
