//! Three-process live fixture for a durable Claude personal-agent profile.
//!
//! Each invocation loads host inventory afresh, materializes a short-lived
//! credential, prepares a new OCI lease and delegates conversation recovery to
//! the native ACP adapter. It is intentionally a product-gate fixture, not a
//! second agent loop or a Claude-specific session engine.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio, PermissionOptionKind};
use serde_json::Value;
use swem_host::{
    AGENT_DISTRIBUTION_RECEIPT_SCHEMA, AgentDistributionReceipt, AttachmentBinding,
    AttachmentTransport, BackendProbe, BackendProbeStatus, CredentialBindingRef,
    CredentialSourceRef, EnvironmentGuarantee, EnvironmentRequirements, NativeSessionControl,
    NativeSessionOptions, NativeSessionOutcome, NativeSessionStart, PersonalAgentProfile,
    PersonalAgentProfileStore, PodmanNetworkPolicy, PodmanWorkspaceBinding, ResolvedMcpAttachment,
    ResolvedPodmanAgentConnection, RoutingLedger, SessionPermissionPolicy, SessionRouteBinding,
    WorkspacePersistence, probe_podman, run_personal_agent_in_podman,
};

const PROFILE_ID: &str = "claude-main";
const ROUTE_ID: &str = "claude-main-route";
const DISTRIBUTION_ID: &str = "claude-agent-acp-linux-amd64-0.70.0";
const ENVIRONMENT_PROFILE_ID: &str = "podman-private-egress-v1";
const PERMISSION_PROFILE_ID: &str = "interactive-surface-v1";
const ATTACHMENT_PROFILE_ID: &str = "echo-capability-v1";
const CREDENTIAL_BINDING_ID: &str = "claude-subscription";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let stage = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or("missing fixture stage")?;
    let inventory_root = absolute(arguments.next().ok_or("missing inventory root")?)?;
    let ledger_path = absolute(arguments.next().ok_or("missing route ledger")?)?;
    let output_path = absolute(arguments.next().ok_or("missing output path")?)?;
    if arguments.next().is_some() {
        return Err("unexpected fixture arguments".into());
    }

    match stage.as_str() {
        "first" => first_process(&inventory_root, &ledger_path, &output_path).await?,
        "second" => second_process(&inventory_root, &ledger_path, &output_path).await?,
        "cancel" => cancel_process(&inventory_root, &ledger_path, &output_path).await?,
        _ => return Err(format!("unknown fixture stage: {stage}").into()),
    }
    Ok(())
}

async fn first_process(
    inventory_root: &Path,
    ledger_path: &Path,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = required_directory("SWEM_LIVE_PODMAN_WORKSPACE")?;
    let agent_home = required_directory("SWEM_LIVE_PODMAN_AGENT_HOME")?;
    let profile = PersonalAgentProfile::new(
        PROFILE_ID,
        "claude-code",
        DISTRIBUTION_ID,
        ENVIRONMENT_PROFILE_ID,
        PERMISSION_PROFILE_ID,
        &workspace,
        &agent_home,
        vec![AttachmentBinding::new(
            ATTACHMENT_PROFILE_ID,
            "echo-capability",
            AttachmentTransport::Stdio,
        )],
        vec![CredentialBindingRef {
            binding_id: CREDENTIAL_BINDING_ID.into(),
            source: CredentialSourceRef::EnvironmentVariable {
                name: "SWEM_LIVE_CLAUDE_CREDENTIAL".into(),
            },
            target_environment: "CLAUDE_CODE_OAUTH_TOKEN".into(),
        }],
    )?;
    let store = PersonalAgentProfileStore::open(inventory_root)?;
    let profile_path = store.create(&profile)?;
    drop(store);

    let nonce = format!("swem-profile-{}", std::process::id());
    let transcript = workspace.join("profile-first-transcript.jsonl");
    let run = run_profile_session(
        inventory_root,
        NativeSessionStart::New,
        format!(
            "Call the MCP tool `echo` from server `echo-capability` exactly once with nonce `{nonce}`. Do not use shell, filesystem, or another tool. Then report the exact structured result."
        ),
        &transcript,
        false,
    )
    .await?;
    let receipt: Value = serde_json::from_slice(&fs::read(workspace.join("mcp-receipt.json"))?)?;
    if receipt["nonce"] != nonce || receipt["server"] != "swem-live-echo" {
        return Err("external MCP receipt did not match requested nonce".into());
    }
    let profile = PersonalAgentProfileStore::open(inventory_root)?.load(PROFILE_ID)?;
    let binding = SessionRouteBinding::new(
        ROUTE_ID,
        &profile.agent_id,
        &profile.profile_id,
        &run.outcome.session_id,
        &profile.environment_profile_id,
        &profile.workspace,
        profile.attachments.clone(),
    )?;
    let mut ledger = RoutingLedger::open(ledger_path)?;
    ledger.bind_route(&binding)?;
    write_result(
        output_path,
        &serde_json::json!({
            "stage": "first",
            "profile_path": profile_path,
            "profile": profile,
            "distribution": run.distribution,
            "session_id": run.outcome.session_id,
            "start": run.outcome.start,
            "mcp_receipt": receipt,
            "credential_cleanup": run.credential_cleanup,
            "environment_cleanup": run.outcome.environment_terminal_evidence,
            "transcript": transcript,
        }),
    )
}

async fn second_process(
    inventory_root: &Path,
    ledger_path: &Path,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let profile = PersonalAgentProfileStore::open(inventory_root)?.load(PROFILE_ID)?;
    let ledger = RoutingLedger::open(ledger_path)?;
    let stored = ledger.route(ROUTE_ID)?;
    let expected = SessionRouteBinding::new(
        ROUTE_ID,
        &profile.agent_id,
        &profile.profile_id,
        &stored.native_session_id,
        &profile.environment_profile_id,
        &profile.workspace,
        profile.attachments.clone(),
    )?;
    ledger.require_identity(&expected.identity())?;
    drop(ledger);

    let transcript = profile.workspace.join("profile-resume-transcript.jsonl");
    let run = run_profile_session(
        inventory_root,
        NativeSessionStart::Resume {
            session_id: stored.native_session_id.clone(),
        },
        "Without calling any tool, state the nonce from the preceding turn exactly. It was not supplied anywhere in this prompt."
            .into(),
        &transcript,
        false,
    )
    .await?;
    if run.outcome.session_id != stored.native_session_id || run.outcome.replayed_updates != 0 {
        return Err("native resume changed identity or replayed history".into());
    }
    write_result(
        output_path,
        &serde_json::json!({
            "stage": "second",
            "profile": profile,
            "distribution": run.distribution,
            "session_id": run.outcome.session_id,
            "start": run.outcome.start,
            "reply": run.outcome.turns[0].reply_text,
            "replayed_updates": run.outcome.replayed_updates,
            "native_session_listed": run.outcome.native_session_listed,
            "credential_cleanup": run.credential_cleanup,
            "environment_cleanup": run.outcome.environment_terminal_evidence,
            "transcript": transcript,
        }),
    )
}

async fn cancel_process(
    inventory_root: &Path,
    ledger_path: &Path,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let profile = PersonalAgentProfileStore::open(inventory_root)?.load(PROFILE_ID)?;
    let stored = RoutingLedger::open(ledger_path)?.route(ROUTE_ID)?;
    let transcript = profile.workspace.join("profile-cancel-transcript.jsonl");
    let run = run_profile_session(
        inventory_root,
        NativeSessionStart::Resume {
            session_id: stored.native_session_id,
        },
        "Work continuously for several minutes. Do not finish early and do not call any tool."
            .into(),
        &transcript,
        true,
    )
    .await?;
    let turn = run
        .outcome
        .turns
        .first()
        .ok_or("cancel outcome has no turn")?;
    if turn.stop_reason != "cancelled" {
        return Err(format!(
            "native adapter returned {}, not cancelled",
            turn.stop_reason
        )
        .into());
    }
    write_result(
        output_path,
        &serde_json::json!({
            "stage": "cancel",
            "profile": profile,
            "distribution": run.distribution,
            "session_id": run.outcome.session_id,
            "start": run.outcome.start,
            "stop_reason": turn.stop_reason,
            "control_outcome": turn.control_outcome,
            "credential_cleanup": run.credential_cleanup,
            "environment_cleanup": run.outcome.environment_terminal_evidence,
            "transcript": transcript,
        }),
    )
}

struct ProfileRun {
    distribution: AgentDistributionReceipt,
    outcome: NativeSessionOutcome,
    credential_cleanup: swem_host::CredentialCleanupReceipt,
}

async fn run_profile_session(
    inventory_root: &Path,
    start: NativeSessionStart,
    prompt: String,
    transcript: &Path,
    cancel: bool,
) -> Result<ProfileRun, Box<dyn std::error::Error>> {
    let profile = PersonalAgentProfileStore::open(inventory_root)?.load(PROFILE_ID)?;
    let probe = probe_podman();
    if probe.status != BackendProbeStatus::Ready {
        return Err(format!("Podman endpoint is not ready: {:?}", probe.status).into());
    }
    let distribution = distribution_receipt()?;
    let connection = resolved_connection(&profile, distribution, probe)?;
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_mins(4));
    options.start = start;
    options.transcript_path = Some(transcript.to_path_buf());
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    let task = tokio::spawn(async move {
        run_personal_agent_in_podman(&profile, &connection, &[prompt], options).await
    });
    let permission_control = control.clone();
    let permission_task = tokio::spawn(async move {
        while let Some(request) = permission_control.next_permission_request().await {
            let allow_once = request
                .options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::AllowOnce)
                .map(|option| option.option_id.0.to_string());
            match allow_once {
                Some(option) => permission_control.select_permission(request.sequence, option)?,
                None => {
                    return Err(swem_host::SupplyError::Protocol(
                        "permission request omitted allow_once".into(),
                    ));
                }
            }
        }
        Ok::<(), swem_host::SupplyError>(())
    });
    if cancel {
        let active = control
            .wait_for_active_turn()
            .await
            .ok_or("session finished before cancel control became active")?;
        if control.cancel_active_turn().as_ref() != Some(&active) {
            return Err("cancel signal did not target the observed active turn".into());
        }
    }
    let outcome = task.await??;
    permission_task.await??;
    if outcome.session.environment_terminal_evidence.is_none() {
        return Err("session returned before terminal environment cleanup".into());
    }
    let credential_cleanup = outcome
        .credential_cleanup
        .into_iter()
        .next()
        .ok_or("profile run returned no credential cleanup receipt")?;
    Ok(ProfileRun {
        distribution: outcome.distribution,
        outcome: outcome.session,
        credential_cleanup,
    })
}

fn resolved_connection(
    profile: &PersonalAgentProfile,
    distribution: AgentDistributionReceipt,
    probe: BackendProbe,
) -> Result<ResolvedPodmanAgentConnection, Box<dyn std::error::Error>> {
    let workspace_source = std::env::var("SWEM_LIVE_PODMAN_SERVICE_SOURCE")?;
    let home_source = std::env::var("SWEM_LIVE_PODMAN_AGENT_HOME_SERVICE_SOURCE")?;
    let mut requirements = EnvironmentRequirements::new(&profile.workspace);
    requirements.persistence = WorkspacePersistence::Persistent;
    requirements.cpu_limit = Some(1);
    requirements.memory_mib = Some(768);
    requirements.required_guarantees.extend([
        EnvironmentGuarantee::PersistentWorkspace,
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::AmbientEnvironmentFiltering,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuarantee::ResourceLimits,
    ]);
    Ok(ResolvedPodmanAgentConnection {
        distribution,
        environment_profile_id: ENVIRONMENT_PROFILE_ID.into(),
        permission_profile_id: PERMISSION_PROFILE_ID.into(),
        probe,
        requirements,
        workspace: PodmanWorkspaceBinding {
            service_source: workspace_source,
            container_target: "/workspace".into(),
        },
        agent_home: PodmanWorkspaceBinding {
            service_source: home_source,
            container_target: "/home/swem".into(),
        },
        network: PodmanNetworkPolicy::PrivateEgress,
        cpu_limit: 1,
        memory_mib: 768,
        pids_limit: 256,
        attachments: vec![ResolvedMcpAttachment {
            binding: AttachmentBinding::new(
                ATTACHMENT_PROFILE_ID,
                "echo-capability",
                AttachmentTransport::Stdio,
            ),
            server: McpServer::Stdio(
                McpServerStdio::new("echo-capability", "/usr/local/bin/node").args(vec![
                    "/workspace/mcp-live-echo.mjs".into(),
                    "--receipt".into(),
                    "/workspace/mcp-receipt.json".into(),
                ]),
            ),
        }],
    })
}

fn distribution_receipt() -> Result<AgentDistributionReceipt, Box<dyn std::error::Error>> {
    let lockfile =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/package-lock.json");
    Ok(AgentDistributionReceipt {
        schema: AGENT_DISTRIBUTION_RECEIPT_SCHEMA.into(),
        distribution_id: DISTRIBUTION_ID.into(),
        registry_id: "claude-acp".into(),
        publisher: "agentclientprotocol".into(),
        package: "@agentclientprotocol/claude-agent-acp".into(),
        version: "0.70.0".into(),
        package_integrity: "sha512-Psqj6fhV4pQ8IM480zpJ+xGiMMIqNLxlsTj5Mzn+T8KSURCVNJdl0ktcqLMjgHJC/QnOvDdDkFf3xTW9VIV9aQ==".into(),
        lockfile_sha256: AgentDistributionReceipt::lockfile_digest(&lockfile)?,
        platform: "linux/amd64".into(),
        resolved_image: std::env::var("SWEM_LIVE_CLAUDE_ACP_IMAGE")?,
        agent_executable: "/usr/local/bin/node".into(),
        agent_args: vec![
            "/opt/swem/agent/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js".into(),
        ],
    })
}

fn required_directory(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(std::env::var_os(name).ok_or_else(|| format!("{name} is unset"))?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute").into());
    }
    fs::create_dir_all(&path)?;
    Ok(path.canonicalize()?)
}

fn absolute(value: std::ffi::OsString) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(format!("path must be absolute: {}", path.display()).into());
    }
    Ok(path)
}

fn write_result(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
