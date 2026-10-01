use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio, PermissionOptionKind};
use serde_json::Value;
use swem_host::{
    BackendProbe, BackendProbeStatus, CredentialBindingRef, CredentialSourceRef,
    EnvironmentGuarantee, EnvironmentRequirements, EnvironmentTransport, IntegrationKind,
    LaunchCommand, NativeSessionControl, NativeSessionOptions, NativeSessionStart,
    PodmanContainerSpec, PodmanCredentialLease, PodmanNetworkPolicy, PodmanWorkspaceBinding,
    SessionPermissionPolicy, WorkspacePersistence, direct_environment_lease, prepare_podman_lease,
    probe_podman, run_native_session,
};

fn fixture_workspace(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture workspace");
    root.canonicalize().expect("canonical fixture workspace")
}

fn fixture_launch(args: Vec<String>) -> (LaunchCommand, PathBuf) {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    (
        LaunchCommand {
            executable: executable.display().to_string(),
            args,
            integration: IntegrationKind::DirectAcp,
        },
        executable,
    )
}

fn ambient_probe_name() -> String {
    #[cfg(windows)]
    const ALLOWED: &[&str] = &[
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
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
    const ALLOWED: &[&str] = &[
        "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "LC_ALL", "TERM",
    ];

    let candidates = std::env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .filter(|name| {
            !ALLOWED
                .iter()
                .any(|allowed| name.eq_ignore_ascii_case(allowed))
                && !name.starts_with("SWEM_")
        })
        .collect::<Vec<_>>();
    candidates
        .iter()
        .find(|name| {
            let upper = name.to_ascii_uppercase();
            ["TOKEN", "KEY", "SECRET", "PASSWORD"]
                .iter()
                .any(|marker| upper.contains(marker))
        })
        .cloned()
        .or_else(|| candidates.into_iter().next())
        .expect("test process must have one ambient variable outside the bootstrap allowlist")
}

#[tokio::test]
async fn direct_transport_clears_ambient_environment_and_binds_cwd() {
    let workspace = fixture_workspace("environment-transport");
    let transcript = workspace.join("transcript.jsonl");
    let (launch, executable) = fixture_launch(Vec::new());
    let requirements = EnvironmentRequirements::new(&workspace);
    let lease = direct_environment_lease("direct-fixture", &requirements).expect("direct lease");
    let probe_name = ambient_probe_name();
    assert!(std::env::var_os(&probe_name).is_some());

    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.environment_lease = Some(lease);
    options.transcript_path = Some(transcript.clone());
    options.process_environment_overrides = BTreeMap::from([
        ("SWEM_ALLOWED_TEST".into(), "visible-only-in-child".into()),
        ("SWEM_PROBE_VARIABLE_NAME".into(), probe_name.clone()),
    ]);
    let outcome = run_native_session(
        &launch,
        &executable,
        &workspace,
        &["observe environment".into()],
        &options,
    )
    .await
    .expect("run session through direct environment transport");

    assert_eq!(
        outcome.child_environment,
        "allowlisted_with_explicit_values"
    );
    let reply: Value = serde_json::from_str(&outcome.turns[0].reply_text).expect("fixture JSON");
    assert_eq!(
        reply["environment"]["explicit_value"],
        "visible-only-in-child"
    );
    assert_eq!(reply["environment"]["ambient_probe_name"], probe_name);
    assert_eq!(reply["environment"]["ambient_probe_present"], false);
    let observed_cwd = PathBuf::from(
        reply["environment"]["cwd"]
            .as_str()
            .expect("fixture reports cwd"),
    )
    .canonicalize()
    .expect("reported cwd is canonicalizable");
    assert_eq!(observed_cwd, workspace);

    let transcript = fs::read_to_string(&transcript).expect("read transcript");
    let launch_record = transcript
        .lines()
        .find(|line| line.contains("\"kind\":\"launch\""))
        .expect("transcript contains launch record");
    assert!(launch_record.contains("SWEM_ALLOWED_TEST"));
    assert!(!launch_record.contains("visible-only-in-child"));
    assert!(launch_record.contains("environment_guarantee_evidence"));
    assert!(launch_record.contains("enforced_configuration"));
    assert!(!launch_record.contains("filesystem_isolation"));
    remove_fixture_directory(&workspace).await;
}

#[tokio::test]
async fn direct_transport_terminates_the_spawned_process_tree() {
    let workspace = fixture_workspace("process-tree");
    let heartbeat = workspace.join("descendant-heartbeat.txt");
    let (launch, executable) = fixture_launch(vec![
        "--spawn-descendant".into(),
        heartbeat.display().to_string(),
    ]);
    let lease = direct_environment_lease("tree-fixture", &EnvironmentRequirements::new(&workspace))
        .expect("direct lease");
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.environment_lease = Some(lease);

    run_native_session(
        &launch,
        &executable,
        &workspace,
        &["finish normally".into()],
        &options,
    )
    .await
    .expect("run session with descendant");

    wait_for_file(&heartbeat).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let stopped_at = fs::read_to_string(&heartbeat).expect("read stopped heartbeat");
    tokio::time::sleep(Duration::from_millis(350)).await;
    assert_eq!(
        fs::read_to_string(&heartbeat).expect("read heartbeat again"),
        stopped_at,
        "descendant kept running after the ACP transport closed"
    );
    remove_fixture_directory(&workspace).await;
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end fixture keeps create, ACP attach, terminal evidence and external absence in one scenario"
)]
async fn prepared_podman_transport_uses_the_same_acp_path_and_removes_exact_instance() {
    let workspace = fixture_workspace("podman-transport");
    let podman = PathBuf::from(env!("CARGO_BIN_EXE_swem-podman-fixture"));
    let fixture_agent = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let probe = BackendProbe {
        backend_id: "podman".into(),
        endpoint_id: "podman:native".into(),
        status: BackendProbeStatus::Ready,
        executable: Some(podman.clone()),
        command_prefix: Vec::new(),
        version: Some("fixture-only".into()),
        topology: serde_json::json!({"kind": "hermetic_cli_double"}),
        available_guarantees: [
            EnvironmentGuarantee::CanonicalWorkspaceBinding,
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuarantee::AmbientEnvironmentFiltering,
            EnvironmentGuarantee::FilesystemIsolation,
            EnvironmentGuarantee::ProcessTreeCleanup,
            EnvironmentGuarantee::ResourceLimits,
            EnvironmentGuarantee::NetworkDenyByDefault,
        ]
        .into_iter()
        .collect(),
        limitations: vec!["not real isolation evidence".into()],
        diagnostics: Vec::new(),
    };
    let mut requirements = EnvironmentRequirements::new(&workspace);
    requirements.cpu_limit = Some(2);
    requirements.memory_mib = Some(2_048);
    requirements.required_guarantees.extend([
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::AmbientEnvironmentFiltering,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuarantee::ResourceLimits,
        EnvironmentGuarantee::NetworkDenyByDefault,
    ]);
    let mut spec = PodmanContainerSpec::deny_network(
        "example.invalid/swem/fixture@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        PodmanWorkspaceBinding {
            service_source: "/fixture/workspace".into(),
            container_target: "/workspace".into(),
        },
        "/opt/swem/fixture-agent",
    );
    spec.agent_args = vec![
        "--fixture-host-agent".into(),
        fixture_agent.display().to_string(),
    ];
    let lease_id = format!(
        "podman-acp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    );
    let mut lease = prepare_podman_lease(&probe, &lease_id, &requirements, &spec)
        .expect("prepare fixture Podman instance");
    // The CLI double launches the fixture as a Windows process and therefore
    // cannot dereference the real OCI-visible `/workspace` address. The live
    // Podman tests below exercise the cross-namespace address unchanged.
    lease.agent_workspace = workspace.clone();
    let cleanup_promise = lease
        .evidence
        .get(&EnvironmentGuarantee::ProcessTreeCleanup)
        .expect("active lease names its enforced cleanup protocol");
    assert_eq!(
        cleanup_promise.kind,
        swem_host::EnvironmentEvidenceKind::EnforcedConfiguration,
        "terminal cleanup observation must not be claimed by an active lease"
    );
    assert_eq!(
        cleanup_promise.details["terminal_receipt_observed_at_lease_time"],
        false
    );
    let instance_id = lease.instance_id.clone();
    let transport =
        EnvironmentTransport::podman(&lease, &podman).expect("bind exact Podman transport");
    let logical_launch = LaunchCommand {
        executable: spec.agent_executable.clone(),
        args: spec.agent_args.clone(),
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.environment_lease = Some(lease);
    options.environment_transport = Some(transport);

    let outcome = run_native_session(
        &logical_launch,
        &fixture_agent,
        &workspace,
        &["same ACP path".into()],
        &options,
    )
    .await
    .expect("run ACP through prepared Podman fixture");
    let reply: Value =
        serde_json::from_str(&outcome.turns[0].reply_text).expect("fixture reply JSON");
    assert_eq!(reply["prompt"], "same ACP path");
    assert_eq!(
        outcome.environment_lease_id.as_deref(),
        Some(lease_id.as_str())
    );
    assert_eq!(
        outcome
            .environment_terminal_evidence
            .as_ref()
            .map(|evidence| evidence.kind),
        Some(swem_host::EnvironmentEvidenceKind::CleanupObservation)
    );

    let exists = std::process::Command::new(&podman)
        .args(["container", "exists", &instance_id])
        .status()
        .expect("query fixture container state");
    assert_eq!(exists.code(), Some(1), "exact instance survived ACP close");
    remove_fixture_directory(&workspace).await;
}

#[tokio::test]
async fn podman_transport_timeout_still_waits_for_terminal_cleanup() {
    let workspace = fixture_workspace("podman-timeout");
    let podman = PathBuf::from(env!("CARGO_BIN_EXE_swem-podman-fixture"));
    let fixture_agent = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let probe = BackendProbe {
        backend_id: "podman".into(),
        endpoint_id: "podman:native".into(),
        status: BackendProbeStatus::Ready,
        executable: Some(podman.clone()),
        command_prefix: Vec::new(),
        version: Some("fixture-only".into()),
        topology: serde_json::json!({"kind": "hermetic_cli_double"}),
        available_guarantees: [
            EnvironmentGuarantee::CanonicalWorkspaceBinding,
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuarantee::AmbientEnvironmentFiltering,
            EnvironmentGuarantee::FilesystemIsolation,
            EnvironmentGuarantee::ProcessTreeCleanup,
            EnvironmentGuarantee::ResourceLimits,
            EnvironmentGuarantee::NetworkDenyByDefault,
        ]
        .into_iter()
        .collect(),
        limitations: vec!["not real isolation evidence".into()],
        diagnostics: Vec::new(),
    };
    let mut requirements = EnvironmentRequirements::new(&workspace);
    requirements.cpu_limit = Some(2);
    requirements.memory_mib = Some(2_048);
    requirements.required_guarantees.extend([
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::AmbientEnvironmentFiltering,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuarantee::ResourceLimits,
        EnvironmentGuarantee::NetworkDenyByDefault,
    ]);
    let mut spec = PodmanContainerSpec::deny_network(
        "example.invalid/swem/fixture@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        PodmanWorkspaceBinding {
            service_source: "/fixture/workspace".into(),
            container_target: "/workspace".into(),
        },
        "/opt/swem/fixture-agent",
    );
    spec.agent_args = vec![
        "--fixture-host-agent".into(),
        fixture_agent.display().to_string(),
    ];
    let lease_id = format!(
        "podman-timeout-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    );
    let mut lease = prepare_podman_lease(&probe, &lease_id, &requirements, &spec)
        .expect("prepare timeout fixture instance");
    // See the successful hermetic transport test above: this double runs on
    // the client OS, whereas a real OCI agent receives `/workspace`.
    lease.agent_workspace = workspace.clone();
    let instance_id = lease.instance_id.clone();
    let transport = EnvironmentTransport::podman(&lease, &podman).expect("bind timeout transport");
    let logical_launch = LaunchCommand {
        executable: spec.agent_executable.clone(),
        args: spec.agent_args.clone(),
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_millis(300));
    options.environment_lease = Some(lease);
    options.environment_transport = Some(transport);

    let error = run_native_session(
        &logical_launch,
        &fixture_agent,
        &workspace,
        &[serde_json::json!({"fixture": "cancel-ignore-v0.1"}).to_string()],
        &options,
    )
    .await
    .expect_err("ignored prompt must hit host containment timeout");
    assert!(matches!(error, swem_host::SupplyError::SessionTimeout));
    let exists = std::process::Command::new(&podman)
        .args(["container", "exists", &instance_id])
        .status()
        .expect("query timed-out fixture state");
    assert_eq!(
        exists.code(),
        Some(1),
        "timed-out ACP transport left its exact instance behind"
    );
    remove_fixture_directory(&workspace).await;
}

#[tokio::test]
#[ignore = "requires a ready native Podman endpoint, pinned Python image and explicit service path"]
#[allow(
    clippy::too_many_lines,
    reason = "one live acceptance scenario keeps lease evidence, runtime challenge and terminal cleanup together"
)]
async fn live_podman_lease_proves_workspace_acp_runtime_and_terminal_cleanup() {
    let image = std::env::var("SWEM_LIVE_PODMAN_IMAGE")
        .expect("SWEM_LIVE_PODMAN_IMAGE must be a resolved sha256 image ID");
    let service_source = std::env::var("SWEM_LIVE_PODMAN_SERVICE_SOURCE")
        .expect("SWEM_LIVE_PODMAN_SERVICE_SOURCE must name the same workspace in the service VM");
    let workspace = std::env::var_os("SWEM_LIVE_PODMAN_WORKSPACE")
        .map(PathBuf::from)
        .expect("SWEM_LIVE_PODMAN_WORKSPACE must be an absolute client path");
    fs::create_dir_all(&workspace).expect("create live workspace");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/acp-live-fixture.py"),
        workspace.join("acp-live-fixture.py"),
    )
    .expect("copy ACP fixture into live workspace");
    let challenge = "SWEM_H0_2G_HOST_BYTES_2026_08_28\n";
    fs::write(workspace.join("host-challenge.txt"), challenge).expect("write host challenge bytes");
    let workspace = workspace.canonicalize().expect("canonical live workspace");

    let probe = probe_podman();
    assert_eq!(
        probe.status,
        BackendProbeStatus::Ready,
        "native Podman is not ready"
    );
    let mut requirements = EnvironmentRequirements::new(&workspace);
    requirements.cpu_limit = Some(1);
    requirements.memory_mib = Some(256);
    requirements.required_guarantees.extend([
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::AmbientEnvironmentFiltering,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuarantee::ResourceLimits,
        EnvironmentGuarantee::NetworkDenyByDefault,
    ]);
    let mut spec = PodmanContainerSpec::deny_network(
        image,
        PodmanWorkspaceBinding {
            service_source,
            container_target: "/workspace".into(),
        },
        "/usr/local/bin/python3",
    );
    spec.agent_args = vec!["/workspace/acp-live-fixture.py".into()];
    spec.cpu_limit = 1;
    spec.memory_mib = 256;
    spec.pids_limit = 64;
    let lease_id = format!(
        "live-podman-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    );
    let lease = prepare_podman_lease(&probe, &lease_id, &requirements, &spec)
        .expect("prepare real inspected Podman lease");
    assert_eq!(lease.endpoint_id, probe.endpoint_id);
    assert_eq!(
        lease
            .evidence
            .get(&EnvironmentGuarantee::ProcessTreeCleanup)
            .map(|evidence| evidence.kind),
        Some(swem_host::EnvironmentEvidenceKind::EnforcedConfiguration),
        "the active lease names its cleanup protocol; the observation arrives only at terminal"
    );
    let instance_id = lease.instance_id.clone();
    let transport = EnvironmentTransport::podman_endpoint(&lease, &probe)
        .expect("bind ACP to exact Podman endpoint");
    let logical_launch = LaunchCommand {
        executable: spec.agent_executable.clone(),
        args: spec.agent_args.clone(),
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_secs(20));
    options.environment_lease = Some(lease);
    options.environment_transport = Some(transport);
    let outcome = run_native_session(
        &logical_launch,
        Path::new("C:/logical/container/agent"),
        &workspace,
        &["SWEM_H0_2G_ACP_PROMPT".into()],
        &options,
    )
    .await
    .expect("run official ACP ByteStreams through real Podman stdio");
    assert_live_podman_observation(&outcome.turns[0].reply_text, &workspace, challenge);
    assert_eq!(
        outcome
            .environment_terminal_evidence
            .as_ref()
            .map(|evidence| evidence.kind),
        Some(swem_host::EnvironmentEvidenceKind::CleanupObservation)
    );
    let exists = std::process::Command::new(probe.executable.as_ref().expect("Podman executable"))
        .args(["container", "exists", &instance_id])
        .status()
        .expect("query real terminal container state");
    assert_eq!(
        exists.code(),
        Some(1),
        "real container survived terminal cleanup"
    );
}

#[tokio::test]
#[ignore = "requires the content-pinned Claude ACP image, a ready Podman endpoint and an explicit live credential"]
#[allow(
    clippy::too_many_lines,
    reason = "one live acceptance scenario must keep secret materialization, two exact OCI leases, native resume and external MCP evidence together"
)]
async fn live_claude_acp_in_oci_keeps_cycle_optional_and_native_state_persistent() {
    let image = std::env::var("SWEM_LIVE_CLAUDE_ACP_IMAGE")
        .expect("SWEM_LIVE_CLAUDE_ACP_IMAGE must be a resolved sha256 image ID");
    let workspace_source = std::env::var("SWEM_LIVE_PODMAN_SERVICE_SOURCE")
        .expect("SWEM_LIVE_PODMAN_SERVICE_SOURCE must name the workspace in the service VM");
    let home_source = std::env::var("SWEM_LIVE_PODMAN_AGENT_HOME_SERVICE_SOURCE").expect(
        "SWEM_LIVE_PODMAN_AGENT_HOME_SERVICE_SOURCE must name native agent state in the service VM",
    );
    let workspace = std::env::var_os("SWEM_LIVE_PODMAN_WORKSPACE")
        .map(PathBuf::from)
        .expect("SWEM_LIVE_PODMAN_WORKSPACE must be an absolute client path");
    let agent_home = std::env::var_os("SWEM_LIVE_PODMAN_AGENT_HOME")
        .map(PathBuf::from)
        .expect("SWEM_LIVE_PODMAN_AGENT_HOME must be an absolute client path");
    let credential = std::env::var("SWEM_LIVE_CLAUDE_CREDENTIAL")
        .expect("SWEM_LIVE_CLAUDE_CREDENTIAL must be set only for this explicit test");
    assert!(!credential.is_empty(), "live credential must not be empty");

    fs::create_dir_all(&workspace).expect("create live Claude workspace");
    fs::create_dir_all(&agent_home).expect("create persistent live Claude home");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp-live-echo.mjs"),
        workspace.join("mcp-live-echo.mjs"),
    )
    .expect("copy arbitrary non-Cycle MCP fixture");
    let workspace = workspace.canonicalize().expect("canonical live workspace");

    let probe = probe_podman();
    assert_eq!(
        probe.status,
        BackendProbeStatus::Ready,
        "native Podman endpoint is not ready"
    );
    let credential_binding = CredentialBindingRef {
        binding_id: "claude-live-subscription".into(),
        source: CredentialSourceRef::EnvironmentVariable {
            name: "SWEM_LIVE_CLAUDE_CREDENTIAL".into(),
        },
        target_environment: "CLAUDE_CODE_OAUTH_TOKEN".into(),
    };
    let secret = PodmanCredentialLease::materialize(&probe, &credential_binding)
        .expect("materialize short-lived Podman credential from profile binding");

    let mut requirements = EnvironmentRequirements::new(&workspace);
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
    let mut spec = PodmanContainerSpec::deny_network(
        image,
        PodmanWorkspaceBinding {
            service_source: workspace_source,
            container_target: "/workspace".into(),
        },
        "/usr/local/bin/node",
    );
    spec.agent_home = Some(PodmanWorkspaceBinding {
        service_source: home_source,
        container_target: "/home/swem".into(),
    });
    spec.runtime_secrets = vec![secret.runtime_secret()];
    spec.network = PodmanNetworkPolicy::PrivateEgress;
    spec.agent_args = vec![
        "/opt/swem/agent/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js".into(),
    ];
    spec.cpu_limit = 1;
    spec.memory_mib = 768;
    spec.pids_limit = 256;

    let nonce = format!("swem-h0-native-{}", std::process::id());
    let first_transcript = workspace.join("claude-first-transcript.jsonl");
    let first = run_live_claude_lease(
        &probe,
        &requirements,
        &spec,
        &workspace,
        NativeSessionStart::New,
        format!(
            "Call the MCP tool `echo` from server `echo-capability` exactly once with nonce `{nonce}`. Do not use shell, filesystem, or any other tool. Then report the exact structured result."
        ),
        &first_transcript,
    )
    .await;
    assert!(
        first.agent_info.is_some(),
        "ACP initialize exposed no agentInfo"
    );
    assert!(
        first.turns[0]
            .tool_calls
            .iter()
            .any(|call| call.title.contains("echo")),
        "native adapter exposed no arbitrary MCP tool call"
    );
    let receipt: Value = serde_json::from_slice(
        &fs::read(workspace.join("mcp-receipt.json")).expect("read external MCP receipt"),
    )
    .expect("parse external MCP receipt");
    assert_eq!(receipt["nonce"], nonce);
    assert_eq!(receipt["server"], "swem-live-echo");
    assert!(
        !fs::read_to_string(&first_transcript)
            .expect("read first transcript")
            .contains(&credential),
        "credential leaked into the ACP transcript"
    );

    let second_transcript = workspace.join("claude-resume-transcript.jsonl");
    let resumed = run_live_claude_lease(
        &probe,
        &requirements,
        &spec,
        &workspace,
        NativeSessionStart::Resume {
            session_id: first.session_id.clone(),
        },
        "Without calling any tool, state the nonce from the preceding turn exactly. It was not supplied anywhere in this prompt."
            .into(),
        &second_transcript,
    )
    .await;
    assert_eq!(resumed.session_id, first.session_id);
    assert_eq!(
        resumed.replayed_updates, 0,
        "resume must not emulate replay"
    );
    assert!(
        resumed.turns[0].reply_text.contains(&nonce),
        "native resumed session did not recover prior context"
    );
    assert!(
        !fs::read_to_string(&second_transcript)
            .expect("read resume transcript")
            .contains(&credential),
        "credential leaked into the resumed ACP transcript"
    );
    let cleanup = secret
        .remove()
        .expect("remove exact short-lived credential after terminal leases");
    assert!(cleanup.terminally_absent);
}

async fn run_live_claude_lease(
    probe: &BackendProbe,
    requirements: &EnvironmentRequirements,
    spec: &PodmanContainerSpec,
    workspace: &Path,
    start: NativeSessionStart,
    prompt: String,
    transcript: &Path,
) -> swem_host::NativeSessionOutcome {
    let lease_id = format!(
        "live-claude-acp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    );
    let lease = prepare_podman_lease(probe, &lease_id, requirements, spec)
        .expect("prepare inspected Claude ACP Podman lease");
    assert!(
        !lease
            .guarantees()
            .contains(&EnvironmentGuarantee::NetworkDenyByDefault),
        "private egress was mislabeled as network denial"
    );
    let transport = EnvironmentTransport::podman_endpoint(&lease, probe)
        .expect("bind Claude ACP to the exact Podman endpoint");
    let logical_launch = LaunchCommand {
        executable: spec.agent_executable.clone(),
        args: spec.agent_args.clone(),
        integration: IntegrationKind::AcpAdapter,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_mins(4));
    options.start = start;
    options.environment_lease = Some(lease);
    options.environment_transport = Some(transport);
    options.transcript_path = Some(transcript.to_path_buf());
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo-capability", "/usr/local/bin/node").args(vec![
            "/workspace/mcp-live-echo.mjs".into(),
            "--receipt".into(),
            "/workspace/mcp-receipt.json".into(),
        ]),
    )];

    let run_workspace = workspace.to_path_buf();
    let run_launch = logical_launch.clone();
    let task = tokio::spawn(async move {
        run_native_session(
            &run_launch,
            Path::new("C:/logical/container/claude-agent-acp"),
            &run_workspace,
            &[prompt],
            &options,
        )
        .await
    });
    let permission_control = control.clone();
    let permission_task = tokio::spawn(async move {
        while let Some(request) = permission_control.next_permission_request().await {
            let allow_once = request
                .options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::AllowOnce)
                .expect("Claude ACP permission omitted allow_once")
                .option_id
                .0
                .to_string();
            permission_control
                .select_permission(request.sequence, &allow_once)
                .expect("select exact Claude ACP allow_once option");
        }
    });
    let outcome = task
        .await
        .expect("join live Claude ACP task")
        .unwrap_or_else(|error| panic!("live Claude ACP lease failed: {error}"));
    permission_task
        .await
        .expect("join live permission responder");
    assert_eq!(
        outcome
            .environment_terminal_evidence
            .as_ref()
            .map(|evidence| evidence.kind),
        Some(swem_host::EnvironmentEvidenceKind::CleanupObservation),
        "successful turn returned before exact container cleanup"
    );
    outcome
}

fn assert_live_podman_observation(reply_text: &str, workspace: &Path, challenge: &str) {
    let observation: Value = serde_json::from_str(reply_text).expect("parse fixture observation");
    assert_eq!(observation["uid"], 1000);
    assert_eq!(observation["gid"], 1000);
    assert_eq!(observation["cwd"], "/workspace");
    assert_eq!(observation["host_challenge"], challenge);
    assert_eq!(observation["prompt"], "SWEM_H0_2G_ACP_PROMPT");
    assert_eq!(observation["interfaces"], serde_json::json!(["lo"]));
    assert_eq!(observation["cap_eff"], "0000000000000000");
    assert_eq!(observation["cap_bnd"], "0000000000000000");
    assert!(
        observation["network"]
            .as_str()
            .is_some_and(|value| value.starts_with("denied:"))
    );
    assert_eq!(
        observation["environment_names"],
        serde_json::json!(["HOME", "HOSTNAME", "LANG", "PATH"])
    );
    assert_eq!(
        fs::read_to_string(workspace.join("container-write.txt"))
            .expect("read container-produced bytes"),
        format!("container-observed:{challenge}")
    );
}

async fn wait_for_file(path: &Path) {
    for _ in 0..50 {
        if path.is_file() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!("fixture descendant did not create {}", path.display());
}

async fn remove_fixture_directory(path: &Path) {
    let mut last_error = None;
    for _ in 0..25 {
        match fs::remove_dir_all(path) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!(
        "remove fixture directory {}: {}",
        path.display(),
        last_error.expect("at least one removal attempt failed")
    );
}
