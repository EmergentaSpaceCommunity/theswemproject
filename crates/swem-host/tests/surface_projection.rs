use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    ContentBlock, ImageContent, McpServer, McpServerStdio, PermissionOptionKind, ResourceLink,
    TextContent,
};
use base64::Engine as _;
use swem_host::{
    IntegrationKind, LaunchCommand, NativeSessionControl, NativeSessionOptions, NativeSessionStart,
    Readiness, RoutingLedger, SessionPermissionPolicy, SessionRouteBinding, SurfaceEventSource,
    discover_agents, project_native_session_events, run_native_session,
    run_native_session_with_content, verify_discovered_agent,
};

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "swem-surface-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}

fn fixture_launch() -> (LaunchCommand, PathBuf) {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    (
        LaunchCommand {
            executable: executable.display().to_string(),
            args: Vec::new(),
            integration: IntegrationKind::DirectAcp,
        },
        executable,
    )
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one linear external-process transaction keeps wire order, permission effect and two cursor oracles visible"
)]
async fn native_stream_projects_once_and_offline_surfaces_resume_by_cursor() {
    let root = fixture_root("ordered");
    fs::create_dir_all(&root).expect("create surface workspace");
    let ledger_path = root.join("routing.sqlite3");
    let receipt = root.join("mcp-receipt.json");
    let mcp_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let (launch, executable) = fixture_launch();
    let control = NativeSessionControl::new();
    let surface_events = control
        .take_surface_events()
        .expect("opt into one surface projection before launch");
    let image_payload = base64::engine::general_purpose::STANDARD.encode(b"surface-image");
    let first_prompt = vec![
        ContentBlock::Text(TextContent::new("SWEM_CONTENT_MATRIX")),
        ContentBlock::ResourceLink(
            ResourceLink::new("surface-artifact", "file:///workspace/artifact.txt")
                .mime_type("text/plain")
                .size(17),
        ),
        ContentBlock::Image(
            ImageContent::new(image_payload.clone(), "image/png")
                .uri("file:///workspace/image.png"),
        ),
    ];
    let second_prompt = vec![ContentBlock::Text(TextContent::new(
        serde_json::json!({
            "fixture": "mcp-echo-permission-v0.1",
            "server": "echo",
            "nonce": "surface-nonce"
        })
        .to_string(),
    ))];
    let mut options = NativeSessionOptions::new(Duration::from_secs(20));
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo", &mcp_fixture)
            .args(vec!["--receipt".into(), receipt.display().to_string()]),
    )];

    let run_root = root.clone();
    let task = tokio::spawn(async move {
        run_native_session_with_content(
            &launch,
            &executable,
            &run_root,
            &[first_prompt, second_prompt],
            &options,
        )
        .await
    });

    let active = tokio::time::timeout(Duration::from_secs(10), control.wait_for_active_turn())
        .await
        .expect("session reached a prompt")
        .expect("session remained active");
    let binding = SessionRouteBinding::new(
        "route-surface",
        "echo-agent",
        "fixture-profile",
        &active.session_id,
        "direct-fixture",
        &root,
        Vec::new(),
    )
    .expect("bind exact native session");
    RoutingLedger::open(&ledger_path)
        .expect("open routing ledger")
        .bind_route(&binding)
        .expect("persist route binding");
    let projection = project_native_session_events(
        &ledger_path,
        &binding.route_id,
        "connection-1",
        surface_events,
    )
    .expect("attach one durable projector");

    let permission =
        tokio::time::timeout(Duration::from_secs(10), control.next_permission_request())
            .await
            .expect("fixture requested permission")
            .expect("permission lane remained open");
    let allow_once = permission
        .options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
        .expect("fixture offered allow_once")
        .option_id
        .0
        .to_string();
    control
        .select_permission(permission.sequence, allow_once)
        .expect("surface selected exact ACP option");

    let outcome = task
        .await
        .expect("native task joined")
        .expect("native session completed");
    assert_eq!(outcome.session_id, active.session_id);
    assert_eq!(outcome.turns.len(), 2);
    assert!(receipt.is_file(), "permission-gated MCP effect did not run");
    let projected = projection.finish().await.expect("projection completed");

    let mut ledger = RoutingLedger::open(&ledger_path).expect("reopen durable ledger");
    let batch = ledger
        .events_for_surface(&binding.route_id, "workbench-a", 200)
        .expect("read ordered surface events");
    assert_eq!(
        projected,
        batch.events.len() as u64,
        "every projected event is durable and nothing else reached the route"
    );
    let kinds = batch
        .events
        .iter()
        .map(|event| event.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(kinds.first(), Some(&"host/session_connecting"));
    assert_eq!(kinds.last(), Some(&"host/session_terminal"));
    for required in [
        "acp/initialize",
        "acp/session_new",
        "host/prompt_submitted",
        "acp/session_update",
        "acp/session_request_permission",
        "host/permission_decision",
        "acp/prompt_response",
    ] {
        assert!(
            kinds.contains(&required),
            "surface stream omitted {required}"
        );
    }
    let permission_position = kinds
        .iter()
        .position(|kind| *kind == "acp/session_request_permission")
        .expect("permission event position");
    let decision_position = kinds
        .iter()
        .position(|kind| *kind == "host/permission_decision")
        .expect("decision event position");
    assert!(permission_position < decision_position);
    assert!(
        batch
            .events
            .iter()
            .filter(|event| event.kind == "acp/session_update")
            .all(|event| event.source == SurfaceEventSource::NativeLive)
    );
    let durable_json = serde_json::to_string(&batch.events).expect("serialize event oracle");
    assert!(durable_json.contains("file:///workspace/artifact.txt"));
    assert!(durable_json.contains("dataDescriptor"));
    assert!(
        !durable_json.contains(&image_payload),
        "durable surface events must not retain base64 bodies"
    );

    let head = batch.next_cursor();
    ledger
        .acknowledge_surface(&binding.route_id, "workbench-a", head)
        .expect("ack workbench cursor");
    assert!(
        ledger
            .events_for_surface(&binding.route_id, "workbench-a", 10)
            .expect("read after ack")
            .events
            .is_empty()
    );
    let offline = ledger
        .events_for_surface(&binding.route_id, "telegram-offline", 200)
        .expect("offline surface catches up independently");
    assert_eq!(offline.events, batch.events);
}

#[test]
fn native_surface_lane_refuses_two_direct_consumers() {
    let control = NativeSessionControl::new();
    let _first = control
        .take_surface_events()
        .expect("first surface projector owns lane");
    let second = control
        .take_surface_events()
        .expect_err("second direct consumer must be rejected");
    assert!(second.to_string().contains("two direct consumers"));
}

#[tokio::test]
async fn losing_the_observer_does_not_own_permission_or_interrupt_the_effect() {
    let root = fixture_root("lost-observer");
    fs::create_dir_all(&root).unwrap();
    let receipt = root.join("mcp-receipt.json");
    let (launch, executable) = fixture_launch();
    let control = NativeSessionControl::new();
    let observer = control.take_surface_events().unwrap();
    let mut options = NativeSessionOptions::new(Duration::from_secs(20));
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo", env!("CARGO_BIN_EXE_swem-mcp-echo"))
            .args(vec!["--receipt".into(), receipt.display().to_string()]),
    )];
    let task = tokio::spawn(async move {
        run_native_session(
            &launch,
            &executable,
            &root,
            &[serde_json::json!({
                "fixture":"mcp-echo-permission-v0.1", "server":"echo", "nonce":"lost-observer"
            })
            .to_string()],
            &options,
        )
        .await
    });
    let permission =
        tokio::time::timeout(Duration::from_secs(10), control.next_permission_request())
            .await
            .unwrap()
            .unwrap();
    assert!(!receipt.exists(), "permission has not been granted");
    drop(observer);
    assert!(
        !task.is_finished(),
        "observer loss must not terminate the native operation"
    );
    let option = permission
        .options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
        .unwrap()
        .option_id
        .0
        .to_string();
    // The surviving runtime/control owner, not the vanished viewer, grants the exact request.
    control
        .select_permission(permission.sequence, option)
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(outcome.turns.len(), 1);
    assert!(
        receipt.is_file(),
        "independent MCP effect must finish after observer loss"
    );
    let evidence: serde_json::Value = serde_json::from_slice(&fs::read(receipt).unwrap()).unwrap();
    assert_eq!(evidence["nonce"], "lost-observer");
}

#[tokio::test]
async fn protocol_failure_is_terminal_surface_state_not_a_false_finished_chat() {
    let root = fixture_root("failed");
    fs::create_dir_all(&root).expect("create failure workspace");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: vec!["--baseline-content-only".into()],
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut events = control
        .take_surface_events()
        .expect("claim failure event lane");
    let prompt = vec![ContentBlock::Image(ImageContent::new(
        base64::engine::general_purpose::STANDARD.encode(b"unsupported"),
        "image/png",
    ))];
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.control = Some(control);
    let error = run_native_session_with_content(&launch, &executable, &root, &[prompt], &options)
        .await
        .expect_err("unadvertised image capability must fail");
    assert!(error.to_string().contains("prompt capability"));

    let mut observed = Vec::new();
    while let Ok(event) = events.try_recv() {
        observed.push(event);
    }
    assert_eq!(
        observed.last().map(|event| event.kind.as_str()),
        Some("host/session_terminal")
    );
    assert_eq!(
        observed.last().expect("terminal event").payload["status"],
        "failed"
    );
    assert!(
        !observed.iter().any(|event| event.kind == "acp/session_new"),
        "content capability rejection must happen before session creation"
    );
}

#[tokio::test]
async fn load_replay_is_labeled_without_becoming_host_conversation_memory() {
    let root = fixture_root("replay");
    fs::create_dir_all(&root).expect("create replay workspace");
    let (launch, executable) = fixture_launch();
    let first = run_native_session_with_content(
        &launch,
        &executable,
        &root,
        &[vec![ContentBlock::Text(TextContent::new("first"))]],
        &NativeSessionOptions::new(Duration::from_secs(10)),
    )
    .await
    .expect("create native session");

    let control = NativeSessionControl::new();
    let mut events = control
        .take_surface_events()
        .expect("claim replay event lane");
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.start = NativeSessionStart::Load {
        session_id: first.session_id.clone(),
    };
    options.control = Some(control);
    let loaded = run_native_session_with_content(
        &launch,
        &executable,
        &root,
        &[vec![ContentBlock::Text(TextContent::new("second"))]],
        &options,
    )
    .await
    .expect("load native session");
    assert_eq!(loaded.replayed_updates, 2);

    let mut updates = Vec::new();
    while let Ok(event) = events.try_recv() {
        if event.kind == "acp/session_update" {
            updates.push(event.source);
        }
    }
    assert_eq!(
        updates
            .iter()
            .filter(|source| **source == SurfaceEventSource::NativeReplay)
            .count(),
        2
    );
    assert!(updates.contains(&SurfaceEventSource::NativeLive));
}

#[tokio::test]
#[ignore = "live Claude surface projection: requires SWEM_NATIVE_AGENT=claude-code and its credential"]
async fn live_claude_updates_reach_the_same_durable_surface_contract() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=claude-code for this explicit live test");
    assert_eq!(agent_id, "claude-code", "this host authorizes only Claude");
    let discovery = discover_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .expect("Claude catalog entry");
    assert!(
        matches!(
            discovery.readiness,
            Readiness::InstalledUnverified | Readiness::HandshakeReady
        ),
        "Claude ACP adapter is unavailable: {:?}",
        discovery.readiness
    );
    let verified = verify_discovered_agent(&discovery, Duration::from_secs(30))
        .await
        .expect("verify installed Claude ACP adapter");
    assert_eq!(verified.readiness, Readiness::HandshakeReady);
    let launch = discovery.launch.expect("verified launch command");
    let executable = discovery
        .executable_path
        .expect("verified Claude executable path");
    let root = fixture_root("live-claude");
    fs::create_dir_all(&root).expect("create live Claude workspace");
    let ledger_path = root.join("routing.sqlite3");
    let control = NativeSessionControl::new();
    let surface_events = control
        .take_surface_events()
        .expect("claim live surface lane before launch");
    let mut options = NativeSessionOptions::new(Duration::from_mins(3));
    options.control = Some(control.clone());
    let prompt = "Reply with the exact marker SWEM_SURFACE_LIVE_043 and nothing else.".to_owned();
    let run_root = root.clone();
    let task = tokio::spawn(async move {
        run_native_session(&launch, &executable, &run_root, &[prompt], &options).await
    });
    let active = tokio::time::timeout(Duration::from_mins(2), control.wait_for_active_turn())
        .await
        .expect("Claude reached an active turn")
        .expect("Claude connection stayed active");
    let binding = SessionRouteBinding::new(
        "route-live-claude",
        "claude-code",
        "live-local-profile",
        &active.session_id,
        "direct-live",
        &root,
        Vec::new(),
    )
    .expect("bind live Claude route");
    RoutingLedger::open(&ledger_path)
        .expect("open live route ledger")
        .bind_route(&binding)
        .expect("persist live route");
    let projection = project_native_session_events(
        &ledger_path,
        &binding.route_id,
        "live-connection-1",
        surface_events,
    )
    .expect("project live Claude events");
    let outcome = task
        .await
        .expect("join live Claude task")
        .expect("complete live Claude turn");
    assert!(
        outcome.turns[0]
            .reply_text
            .contains("SWEM_SURFACE_LIVE_043")
    );
    projection.finish().await.expect("finish live projection");
    let events = RoutingLedger::open(&ledger_path)
        .expect("reopen live route")
        .events_for_surface(&binding.route_id, "live-workbench", 200)
        .expect("read live Claude surface events")
        .events;
    assert!(
        events
            .iter()
            .any(|event| event.kind == "acp/session_update")
    );
    assert_eq!(
        events.last().map(|event| event.kind.as_str()),
        Some("host/session_terminal")
    );
}
