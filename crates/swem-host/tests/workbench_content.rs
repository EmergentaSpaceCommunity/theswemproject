//! Domain-neutral Workbench content plane component gate. A real HTTP server
//! streams opaque bytes in, a native ACP fixture receives them unchanged, its
//! rich output is captured as another descriptor, and HTTP full/range reads
//! remain byte-exact after the agent connection has ended.

use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use swem_host::{
    BackendProbe, BackendProbeStatus, EnvironmentGuarantee, EnvironmentRequirements,
    EnvironmentTransport, IntegrationKind, LaunchCommand, PersonalAgentProfile,
    PersonalAgentProfileStore, PodmanContainerSpec, PodmanWorkspaceBinding,
    ResolvedAgentConnection, ResolvedAgentEnvironment, ResolvedDirectAgentConnection,
    RoutingLedger, WorkbenchShellState, prepare_podman_lease, serve_workbench_http,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

struct HttpResponse {
    status: u16,
    headers: std::collections::BTreeMap<String, String>,
    body: Vec<u8>,
}

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

fn fixture_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-workbench-content-{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
    ));
    fs::create_dir_all(&root).expect("create content fixture root");
    root
}

fn open_state(root: &Path) -> WorkbenchShellState {
    WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: Vec::new(),
            })
        },
    )
    .expect("open Workbench state")
}

async fn request(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect to Workbench");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.expect("write head");
    stream.write_all(body).await.expect("write body");
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.expect("read response");
    let boundary = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("HTTP response head terminator");
    let head = std::str::from_utf8(&bytes[..boundary]).expect("ASCII response head");
    let mut lines = head.lines();
    let status = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse()
        .expect("numeric status");
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    HttpResponse {
        status,
        headers,
        body: bytes[boundary + 4..].to_vec(),
    }
}

fn json_body(response: &HttpResponse) -> Value {
    serde_json::from_slice(&response.body).expect("JSON response")
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one transaction keeps upload, ACP handoff, output capture, range and restart oracles together"
)]
async fn bytes_survive_http_acp_output_range_disconnect_and_restart() {
    const LINKED_BYTES: &[u8] = b"\0SWEM_LINKED_OUTPUT\xff\x10\x80\x7f";
    let root = fixture_root();
    let inventory = root.join("inventory");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let profiles = PersonalAgentProfileStore::open(&inventory).expect("open inventory");
    profiles
        .create(
            &PersonalAgentProfile::new(
                "content-agent",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .expect("create content profile"),
        )
        .expect("persist content profile");
    let state = Arc::new(open_state(&root));
    let handle = serve_workbench_http(Arc::clone(&state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("serve Workbench");
    let address = handle.local_addr;
    let bytes = (0_u16..4096)
        .map(|value| u8::try_from(value % 251).expect("bounded byte"))
        .collect::<Vec<_>>();
    let upload = request(
        address,
        "POST",
        "/api/content?name=opaque-sample.bin",
        &[("Content-Type", "application/octet-stream")],
        &bytes,
    )
    .await;
    assert_eq!(upload.status, 200);
    let uploaded = json_body(&upload);
    assert_eq!(uploaded["byte_length"], bytes.len());
    assert_eq!(uploaded["source"], "user_upload");
    let descriptor_id = uploaded["descriptor_id"]
        .as_str()
        .expect("uploaded descriptor id");

    let opened = request(
        address,
        "POST",
        "/api/connections",
        &[("Content-Type", "application/json")],
        serde_json::to_string(&json!({
            "profile_id": "content-agent",
            "mode": "new",
        }))
        .expect("encode open")
        .as_bytes(),
    )
    .await;
    assert_eq!(opened.status, 200);
    let opened = json_body(&opened);
    let connection = opened["connection_id"].as_str().expect("connection id");
    let route = opened["route_id"].as_str().expect("route id");
    let prompt_json = serde_json::to_vec(&json!({
        "content": [{"type": "text", "text": "SWEM_CONTENT_MATRIX"}],
        "content_refs": [descriptor_id],
    }))
    .expect("encode prompt");
    let prompt = request(
        address,
        "POST",
        &format!("/api/connections/{connection}/prompt"),
        &[("Content-Type", "application/json")],
        &prompt_json,
    )
    .await;
    assert_eq!(prompt.status, 200);
    let prompt_text = String::from_utf8_lossy(&prompt.body);
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    assert!(
        !prompt_text.contains(&encoded),
        "HTTP result leaked the blob body"
    );
    let prompt = json_body(&prompt);
    let artifact = prompt["artifacts"]
        .as_array()
        .and_then(|artifacts| artifacts.first())
        .expect("captured agent artifact");
    assert_eq!(artifact["source"], "agent_output");
    assert_eq!(artifact["byte_length"], bytes.len());
    let artifact_id = artifact["descriptor_id"]
        .as_str()
        .expect("agent artifact descriptor id")
        .to_owned();
    // The agent names its oracle after the block's position, and the position
    // moved when a handed file started riding as a path as well as bytes: the
    // turn now carries the inbox link beside the blob. What this test is about
    // is that the bytes arrived whole, so it finds the oracle rather than
    // pinning where in the turn the blob sat.
    let oracle = fs::read_dir(&workspace)
        .expect("read the agent's working directory")
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("-resource.bin"))
        })
        .expect("agent byte oracle");
    assert_eq!(fs::read(&oracle).expect("read the byte oracle"), bytes);

    let linked_prompt = request(
        address,
        "POST",
        &format!("/api/connections/{connection}/prompt"),
        &[("Content-Type", "application/json")],
        br#"{"content":[{"type":"text","text":"SWEM_WORKSPACE_LINK"}]}"#,
    )
    .await;
    assert_eq!(linked_prompt.status, 200);
    let linked_prompt = json_body(&linked_prompt);
    assert_eq!(linked_prompt["stop_reason"], "end_turn");
    assert_eq!(linked_prompt["reply_text"], "SWEM_WORKSPACE_LINK");
    let linked_artifacts = linked_prompt["artifacts"]
        .as_array()
        .expect("linked artifacts");
    assert_eq!(linked_artifacts.len(), 1);
    let linked = &linked_artifacts[0];
    assert_eq!(linked["name"], "linked-output.bin");
    assert_eq!(linked["byte_length"], LINKED_BYTES.len());
    let linked_id = linked["descriptor_id"]
        .as_str()
        .expect("linked descriptor id")
        .to_owned();
    let issues = linked_prompt["artifact_issues"]
        .as_array()
        .expect("linked artifact issues");
    assert_eq!(issues.len(), 2);
    assert!(
        issues
            .iter()
            .all(|issue| issue["reason"] == "resource_link_unavailable")
    );
    assert_eq!(
        fs::read(workspace.join("linked-output.bin")).expect("fixture linked output"),
        LINKED_BYTES
    );
    assert_eq!(
        fs::read(root.join("outside-linked-output.bin")).expect("fixture outside output"),
        b"OUTSIDE_WORKSPACE_MUST_NOT_BE_ADOPTED"
    );
    fs::remove_file(workspace.join("linked-output.bin")).expect("remove source after adoption");
    let adopted = request(
        address,
        "GET",
        &format!("/api/content/{linked_id}"),
        &[],
        &[],
    )
    .await;
    assert_eq!(adopted.status, 200);
    assert_eq!(adopted.body, LINKED_BYTES);

    let partial = request(
        address,
        "GET",
        &format!("/api/content/{artifact_id}"),
        &[("Range", "bytes=37-111")],
        &[],
    )
    .await;
    assert_eq!(partial.status, 206);
    assert_eq!(partial.headers["content-range"], "bytes 37-111/4096");
    assert_eq!(partial.body, bytes[37..=111]);
    let expected_digest = format!(
        "sha-256=:{}:",
        base64::engine::general_purpose::STANDARD.encode(Sha256::digest(&bytes))
    );
    assert_eq!(partial.headers["repr-digest"], expected_digest);

    let disconnected = request(
        address,
        "POST",
        &format!("/api/connections/{connection}/disconnect"),
        &[("Content-Type", "application/json")],
        b"{}",
    )
    .await;
    assert_eq!(disconnected.status, 200);
    let events = RoutingLedger::open(&root.join("routes.sqlite3"))
        .expect("reopen ledger")
        .events_for_surface(route, "content-oracle", 1000)
        .expect("read route events")
        .events;
    assert!(events.iter().any(|event| {
        event.kind == "host/artifact_available" && event.payload["descriptor_id"] == artifact_id
    }));
    assert!(events.iter().any(|event| {
        event.kind == "host/artifact_available" && event.payload["descriptor_id"] == linked_id
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "host/artifact_unavailable")
            .count(),
        2
    );
    let ledger_text = serde_json::to_string(&events).expect("serialize event audit");
    assert!(
        !ledger_text.contains(&encoded),
        "ledger leaked the blob body"
    );

    handle.shutdown().await;
    drop(state);
    let restarted = Arc::new(open_state(&root));
    let restarted_handle = serve_workbench_http(restarted, ([127, 0, 0, 1], 0).into())
        .await
        .expect("restart Workbench");
    let full = request(
        restarted_handle.local_addr,
        "GET",
        &format!("/api/content/{linked_id}?download=1"),
        &[],
        &[],
    )
    .await;
    assert_eq!(full.status, 200);
    assert_eq!(full.body, LINKED_BYTES);
    assert!(full.headers["content-disposition"].starts_with("attachment;"));
    restarted_handle.shutdown().await;
    fs::remove_dir_all(root).expect("remove content fixture root");
}

fn prepared_podman_connection(
    workspace: &Path,
    environment_profile_id: &str,
) -> (ResolvedAgentConnection, PathBuf, String) {
    let podman = PathBuf::from(env!("CARGO_BIN_EXE_swem-podman-fixture"));
    let fixture_agent = PathBuf::from(env!("CARGO_BIN_EXE_swem-workspace-link-agent"));
    let guarantees = [
        EnvironmentGuarantee::CanonicalWorkspaceBinding,
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::AmbientEnvironmentFiltering,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::ProcessTreeCleanup,
        EnvironmentGuarantee::ResourceLimits,
        EnvironmentGuarantee::NetworkDenyByDefault,
    ]
    .into_iter()
    .collect();
    let probe = BackendProbe {
        backend_id: "podman".into(),
        endpoint_id: "podman:native".into(),
        status: BackendProbeStatus::Ready,
        executable: Some(podman.clone()),
        command_prefix: Vec::new(),
        version: Some("fixture-only".into()),
        topology: json!({"kind": "hermetic_cli_double"}),
        available_guarantees: guarantees,
        limitations: vec!["not real isolation evidence".into()],
        diagnostics: Vec::new(),
    };
    let mut requirements = EnvironmentRequirements::new(workspace);
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
    let logical_agent = "/opt/swem/workspace-link-agent";
    let mut spec = PodmanContainerSpec::deny_network(
        "example.invalid/swem/workbench@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        PodmanWorkspaceBinding {
            service_source: "/fixture/workspace".into(),
            container_target: "/workspace".into(),
        },
        logical_agent,
    );
    spec.agent_args = vec![
        "--fixture-host-agent".into(),
        fixture_agent.display().to_string(),
        "--fixture-host-workspace".into(),
        workspace.display().to_string(),
    ];
    let lease_id = format!(
        "workbench-podman-{}-{}",
        std::process::id(),
        NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
    );
    let lease = prepare_podman_lease(&probe, &lease_id, &requirements, &spec)
        .expect("prepare Podman lease");
    let instance_id = lease.instance_id.clone();
    let transport =
        EnvironmentTransport::podman(&lease, &podman).expect("bind exact Podman transport");
    (
        ResolvedAgentConnection {
            launch: LaunchCommand {
                executable: logical_agent.into(),
                args: spec.agent_args,
                integration: IntegrationKind::DirectAcp,
            },
            agent_executable: fixture_agent,
            mcp_servers: Vec::new(),
            environment: ResolvedAgentEnvironment::Prepared {
                environment_profile_id: environment_profile_id.into(),
                lease: Box::new(lease),
                transport: Box::new(transport),
            },
        },
        podman,
        instance_id,
    )
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one transaction proves prepared environment identity, ACP namespace mapping, CAS durability and exact cleanup"
)]
async fn prepared_podman_workbench_maps_only_its_bound_workspace_links() {
    const OUTPUT_BYTES: &[u8] = b"\0SWEM_PODMAN_LINKED_OUTPUT\xff\x21\x80";
    let root = fixture_root();
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let environment_profile_id = "podman-workbench-fixture";
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                "podman-content-agent",
                "swem-workspace-link-agent",
                "workspace-link-fixture-distribution",
                environment_profile_id,
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .expect("create prepared profile"),
        )
        .expect("persist prepared profile");
    let (resolved, podman, instance_id) =
        prepared_podman_connection(&workspace, environment_profile_id);
    let state = Arc::new(
        WorkbenchShellState::open_with_environment(
            &root.join("inventory"),
            &root.join("routes.sqlite3"),
            Duration::from_secs(20),
            move |_| Ok(resolved.clone()),
        )
        .expect("open prepared Workbench state"),
    );
    let handle = serve_workbench_http(Arc::clone(&state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("serve prepared Workbench");
    let opened = request(
        handle.local_addr,
        "POST",
        "/api/connections",
        &[("Content-Type", "application/json")],
        br#"{"profile_id":"podman-content-agent","mode":"new"}"#,
    )
    .await;
    assert_eq!(
        opened.status,
        200,
        "{}",
        String::from_utf8_lossy(&opened.body)
    );
    let opened = json_body(&opened);
    let connection = opened["connection_id"].as_str().expect("connection id");
    let route = opened["route_id"].as_str().expect("route id");
    let prompt = request(
        handle.local_addr,
        "POST",
        &format!("/api/connections/{connection}/prompt"),
        &[("Content-Type", "application/json")],
        br#"{"content":[{"type":"text","text":"SWEM_PODMAN_WORKSPACE_LINK"}]}"#,
    )
    .await;
    assert_eq!(
        prompt.status,
        200,
        "{}",
        String::from_utf8_lossy(&prompt.body)
    );
    let prompt = json_body(&prompt);
    let artifacts = prompt["artifacts"].as_array().expect("artifact list");
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["name"], "podman-linked-output.bin");
    assert_eq!(artifacts[0]["byte_length"], OUTPUT_BYTES.len());
    let descriptor_id = artifacts[0]["descriptor_id"]
        .as_str()
        .expect("descriptor id")
        .to_owned();
    let issues = prompt["artifact_issues"].as_array().expect("issue list");
    assert_eq!(issues.len(), 2);
    assert!(
        issues
            .iter()
            .all(|issue| issue["reason"] == "resource_link_unavailable")
    );
    assert_eq!(
        fs::read(workspace.join("podman-linked-output.bin")).expect("host bind output"),
        OUTPUT_BYTES
    );
    let disconnected = request(
        handle.local_addr,
        "POST",
        &format!("/api/connections/{connection}/disconnect"),
        &[("Content-Type", "application/json")],
        b"{}",
    )
    .await;
    assert_eq!(disconnected.status, 200);
    let exists = std::process::Command::new(&podman)
        .args(["container", "exists", &instance_id])
        .status()
        .expect("query exact fixture instance");
    assert_eq!(
        exists.code(),
        Some(1),
        "prepared instance survived disconnect"
    );
    let events = RoutingLedger::open(&root.join("routes.sqlite3"))
        .expect("open route ledger")
        .events_for_surface(route, "podman-content-oracle", 1000)
        .expect("read route events")
        .events;
    assert!(events.iter().any(|event| {
        event.kind == "acp/session_update"
            && event
                .payload
                .to_string()
                .contains("file:///workspace/podman-linked-output.bin")
    }));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "host/artifact_unavailable")
            .count(),
        2
    );
    fs::remove_file(workspace.join("podman-linked-output.bin")).expect("remove adopted source");
    handle.shutdown().await;
    drop(state);
    let restarted = Arc::new(open_state(&root));
    let restarted_handle = serve_workbench_http(restarted, ([127, 0, 0, 1], 0).into())
        .await
        .expect("restart Workbench");
    let recovered = request(
        restarted_handle.local_addr,
        "GET",
        &format!("/api/content/{descriptor_id}"),
        &[],
        &[],
    )
    .await;
    assert_eq!(recovered.status, 200);
    assert_eq!(recovered.body, OUTPUT_BYTES);
    restarted_handle.shutdown().await;
    fs::remove_dir_all(root).expect("remove prepared content fixture root");
}

#[tokio::test]
async fn rejected_prepared_workbench_connection_removes_its_exact_instance() {
    let root = fixture_root();
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                "rejected-podman-agent",
                "swem-workspace-link-agent",
                "workspace-link-fixture-distribution",
                "expected-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .expect("create rejected profile"),
        )
        .expect("persist rejected profile");
    let (resolved, podman, instance_id) =
        prepared_podman_connection(&workspace, "wrong-environment");
    let state = WorkbenchShellState::open_with_environment(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        move |_| Ok(resolved.clone()),
    )
    .expect("open rejecting Workbench state");
    let error = state
        .open_connection(
            "rejected-podman-agent",
            swem_host::ShellConnectionMode::New,
            None,
        )
        .await
        .expect_err("mismatched environment must fail closed");
    assert!(error.to_string().contains("environment profile differs"));
    let exists = std::process::Command::new(&podman)
        .args(["container", "exists", &instance_id])
        .status()
        .expect("query rejected fixture instance");
    assert_eq!(exists.code(), Some(1), "rejected instance survived cleanup");
    fs::remove_dir_all(root).expect("remove rejected fixture root");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one failure oracle keeps ACP ordering, adoption and restart recovery together"
)]
async fn partial_rich_output_survives_a_failed_prompt_turn() {
    const MESSAGE_BYTES: &[u8] = b"\0SWEM_PARTIAL_MESSAGE\xff\x11";
    const TOOL_BYTES: &[u8] = b"\0SWEM_PARTIAL_TOOL\xfe\x12";
    let root = fixture_root();
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                "partial-agent",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .expect("create partial profile"),
        )
        .expect("persist partial profile");

    let state = Arc::new(open_state(&root));
    let handle = serve_workbench_http(Arc::clone(&state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("serve Workbench");
    let opened = request(
        handle.local_addr,
        "POST",
        "/api/connections",
        &[("Content-Type", "application/json")],
        br#"{"profile_id":"partial-agent","mode":"new"}"#,
    )
    .await;
    assert_eq!(opened.status, 200);
    let opened = json_body(&opened);
    let connection = opened["connection_id"].as_str().expect("connection id");
    let route = opened["route_id"].as_str().expect("route id");

    let failed = request(
        handle.local_addr,
        "POST",
        &format!("/api/connections/{connection}/prompt"),
        &[("Content-Type", "application/json")],
        br#"{"content":[{"type":"text","text":"SWEM_PARTIAL_ARTIFACT_FAILURE"}]}"#,
    )
    .await;
    assert_eq!(failed.status, 502);

    let events = RoutingLedger::open(&root.join("routes.sqlite3"))
        .expect("reopen ledger")
        .events_for_surface(route, "partial-oracle", 1000)
        .expect("read partial route")
        .events;
    let message_update = events
        .iter()
        .position(|event| {
            event.kind == "acp/session_update"
                && event
                    .payload
                    .to_string()
                    .contains("partial-message-output.bin")
        })
        .expect("message source update");
    let tool_update = events
        .iter()
        .position(|event| {
            event.kind == "acp/session_update"
                && event
                    .payload
                    .to_string()
                    .contains("partial-tool-output.bin")
        })
        .expect("tool source update");
    let terminal = events
        .iter()
        .position(|event| {
            event.kind == "host/session_terminal" && event.payload["status"] == "failed"
        })
        .expect("failed terminal event");
    let artifacts = events
        .iter()
        .enumerate()
        .filter(|(_, event)| event.kind == "host/artifact_available")
        .collect::<Vec<_>>();
    assert_eq!(
        artifacts.len(),
        2,
        "each partial artifact is projected once"
    );
    let message_artifact = artifacts
        .iter()
        .find(|(_, event)| event.payload["name"] == "partial-message-output.bin")
        .expect("message artifact");
    let tool_artifact = artifacts
        .iter()
        .find(|(_, event)| event.payload["name"] == "partial-tool-output.bin")
        .expect("tool artifact");
    assert!(message_update < message_artifact.0 && message_artifact.0 < terminal);
    assert!(tool_update < tool_artifact.0 && tool_artifact.0 < terminal);
    assert_eq!(message_artifact.1.payload["turn_index"], 0);
    assert_eq!(tool_artifact.1.payload["turn_index"], 0);
    let message_id = message_artifact.1.payload["descriptor_id"]
        .as_str()
        .expect("message descriptor")
        .to_owned();
    let tool_id = tool_artifact.1.payload["descriptor_id"]
        .as_str()
        .expect("tool descriptor")
        .to_owned();
    assert!(
        !serde_json::to_string(&events)
            .expect("serialize route")
            .contains(&base64::engine::general_purpose::STANDARD.encode(MESSAGE_BYTES)),
        "durable route retained a raw artifact body"
    );
    assert_eq!(
        request(
            handle.local_addr,
            "POST",
            &format!("/api/connections/{connection}/disconnect"),
            &[("Content-Type", "application/json")],
            b"{}",
        )
        .await
        .status,
        404,
        "failed runner was removed from the active connection registry"
    );

    fs::remove_file(workspace.join("partial-message-output.bin"))
        .expect("remove adopted message source");
    fs::remove_file(workspace.join("partial-tool-output.bin")).expect("remove adopted tool source");
    handle.shutdown().await;
    drop(state);

    let restarted = Arc::new(open_state(&root));
    let restarted_handle = serve_workbench_http(restarted, ([127, 0, 0, 1], 0).into())
        .await
        .expect("restart Workbench");
    let message = request(
        restarted_handle.local_addr,
        "GET",
        &format!("/api/content/{message_id}"),
        &[],
        &[],
    )
    .await;
    assert_eq!(message.status, 200);
    assert_eq!(message.body, MESSAGE_BYTES);
    let tool = request(
        restarted_handle.local_addr,
        "GET",
        &format!("/api/content/{tool_id}"),
        &[("Range", "bytes=1-8")],
        &[],
    )
    .await;
    assert_eq!(tool.status, 206);
    assert_eq!(tool.body, TOOL_BYTES[1..=8]);
    restarted_handle.shutdown().await;
    fs::remove_dir_all(root).expect("remove partial fixture root");
}
