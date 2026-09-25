use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    ElicitationAcceptAction, ElicitationAction, ElicitationCapabilities, ElicitationContentValue,
    ElicitationFormCapabilities, ElicitationMode, ElicitationUrlCapabilities, EnvVariable,
    McpServer, McpServerStdio, PermissionOptionKind,
};
use serde_json::Value;
use swem_host::{
    EnvironmentEvidenceKind, EnvironmentGuarantee, EnvironmentGuaranteeEvidence, EnvironmentLease,
    IntegrationKind, LaunchCommand, NativePermissionDecisionSource, NativePermissionProvenance,
    NativeSessionControl, NativeSessionOptions, NativeSessionPhase, NativeSessionStart,
    NativeTurnControlOutcome, Readiness, SessionPermissionPolicy, SupplyError, discover_agents,
    run_native_session, verify_discovered_agent,
};

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one transaction keeps invalid attempts, the exact agent receipt and the redaction oracle together"
)]
async fn native_acp_elicitation_keeps_values_connection_local_and_returns_exact_content() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-elicitation-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create elicitation workspace");
    let transcript = root.join("transcript.jsonl");
    let receipt = root.join(".swem-form-elicitation-receipt.json");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.transcript_path = Some(transcript.clone());
    options.control = Some(control.clone());
    options.elicitation_capabilities = Some(
        ElicitationCapabilities::new()
            .form(ElicitationFormCapabilities::new())
            .url(ElicitationUrlCapabilities::new()),
    );
    let run_root = root.clone();
    let run_fixture = fixture.clone();
    let task = tokio::spawn(async move {
        run_native_session(
            &launch,
            &run_fixture,
            &run_root,
            &[serde_json::json!({"fixture": "elicitation-form-v0.1"}).to_string()],
            &options,
        )
        .await
    });

    let offered = tokio::time::timeout(Duration::from_secs(2), control.next_elicitation_request())
        .await
        .expect("fixture offered elicitation")
        .expect("elicitation lane remained open");
    let recovered =
        tokio::time::timeout(Duration::from_secs(1), control.elicitation_request_after(0))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(recovered, offered, "unanswered form survives observer loss");
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            control.elicitation_request_after(offered.sequence)
        )
        .await
        .is_err(),
        "rendered form must not be offered repeatedly to one attachment"
    );
    let ElicitationMode::Form(form) = &offered.request.mode else {
        panic!("fixture offered non-form elicitation");
    };
    assert_eq!(form.requested_schema.properties.len(), 6);
    assert_eq!(
        form.requested_schema.required.as_deref(),
        Some(&["strategy".into(), "iterations".into(), "stems".into()][..])
    );
    let invalid = BTreeMap::from([(
        "strategy".into(),
        ElicitationContentValue::from("not-advertised"),
    )]);
    let error = control
        .answer_elicitation(
            offered.sequence,
            ElicitationAction::Accept(ElicitationAcceptAction::new().content(invalid)),
        )
        .expect_err("invalid enum and missing required values must be refused");
    assert!(matches!(error, SupplyError::Protocol(_)));
    let exact = BTreeMap::from([
        ("strategy".into(), ElicitationContentValue::from("bold")),
        ("iterations".into(), ElicitationContentValue::from(3_i64)),
        ("gain_db".into(), ElicitationContentValue::from(-1.25_f64)),
        ("normalize".into(), ElicitationContentValue::from(true)),
        (
            "stems".into(),
            ElicitationContentValue::from(vec!["voice", "fx"]),
        ),
    ]);
    let mut invalid_format = exact.clone();
    invalid_format.insert(
        "contact".into(),
        ElicitationContentValue::from("not-an-email"),
    );
    control
        .answer_elicitation(
            offered.sequence,
            ElicitationAction::Accept(ElicitationAcceptAction::new().content(invalid_format)),
        )
        .expect_err("mature JSON Schema format validation must reject malformed email");
    let mut injected = exact.clone();
    injected.insert(
        "undeclared".into(),
        ElicitationContentValue::from("must-not-cross"),
    );
    control
        .answer_elicitation(
            offered.sequence,
            ElicitationAction::Accept(ElicitationAcceptAction::new().content(injected)),
        )
        .expect_err("an HTTP caller cannot add fields the ACP form did not declare");
    control
        .answer_elicitation(
            offered.sequence,
            ElicitationAction::Accept(ElicitationAcceptAction::new().content(exact)),
        )
        .expect("answer exact elicitation");
    let outcome = task
        .await
        .expect("elicitation task join")
        .expect("elicitation transaction");
    assert_eq!(outcome.turns[0].reply_text, r#"{"action":"accept"}"#);

    let witnessed: Value =
        serde_json::from_slice(&fs::read(receipt).expect("read agent-side receipt"))
            .expect("receipt JSON");
    assert_eq!(witnessed["action"], "accept");
    assert_eq!(witnessed["content"]["strategy"], "bold");
    assert_eq!(witnessed["content"]["iterations"], 3);
    assert_eq!(witnessed["content"]["gain_db"], -1.25);
    assert_eq!(witnessed["content"]["normalize"], true);
    assert_eq!(
        witnessed["content"]["stems"],
        serde_json::json!(["voice", "fx"])
    );
    let transcript = fs::read_to_string(transcript).expect("read elicitation transcript");
    assert!(transcript.contains("\"kind\":\"elicitation/create\""));
    assert!(!transcript.contains("bold"));
    assert!(!transcript.contains("voice"));
    assert!(!transcript.contains("Review and submit"));

    fs::remove_dir_all(root).expect("remove elicitation workspace");
}

#[tokio::test]
async fn session_rejects_an_environment_without_an_enforced_transport() {
    let workspace = std::env::current_dir()
        .expect("current workspace")
        .canonicalize()
        .expect("canonical workspace");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_secs(2));
    options.environment_lease = Some(EnvironmentLease {
        lease_id: "unproved-container".into(),
        backend_id: "podman".into(),
        endpoint_id: "podman:native".into(),
        instance_id: "swem".into(),
        workspace: workspace.clone(),
        agent_workspace: PathBuf::from("/workspace"),
        evidence: BTreeMap::from([(
            EnvironmentGuarantee::AgentProcessIsolation,
            EnvironmentGuaranteeEvidence {
                kind: EnvironmentEvidenceKind::RuntimeChallenge,
                source: "fixture".into(),
                details: serde_json::json!({}),
            },
        )]),
        cleanup_required: true,
    });
    let error = run_native_session(
        &launch,
        &fixture,
        &workspace,
        &["must not reach the agent".into()],
        &options,
    )
    .await
    .expect_err("unenforced backend must fail before process launch");
    assert!(
        matches!(error, SupplyError::Protocol(message) if message.contains("needs an exact connection-local transport"))
    );
}

#[tokio::test]
async fn generic_session_keeps_turn_identity_and_accepts_arbitrary_mcp_declarations() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-session-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create native session workspace");
    let transcript = root.join("transcript.jsonl");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.transcript_path = Some(transcript.clone());
    options.mcp_servers = vec![
        McpServer::Stdio(McpServerStdio::new("notes", "unused-notes-server")),
        McpServer::Stdio(McpServerStdio::new("calendar", "unused-calendar-server")),
    ];

    let outcome = run_native_session(
        &launch,
        &fixture,
        &root,
        &["first".into(), "second".into()],
        &options,
    )
    .await
    .expect("run generic native session");

    assert!(outcome.session_id.starts_with("echo-"));
    assert_eq!(outcome.start, NativeSessionStart::New);
    assert_eq!(outcome.replayed_updates, 0);
    assert_eq!(outcome.child_environment, "inherited");
    assert_eq!(outcome.turns.len(), 2);
    let first: Value = serde_json::from_str(&outcome.turns[0].reply_text).expect("first JSON");
    let second: Value = serde_json::from_str(&outcome.turns[1].reply_text).expect("second JSON");
    assert_eq!(first["session_id"], outcome.session_id);
    assert_eq!(first["turn"], 1);
    assert_eq!(first["prompt"], "first");
    assert_eq!(first["mcp_names"], serde_json::json!(["notes", "calendar"]));
    assert_eq!(second["session_id"], outcome.session_id);
    assert_eq!(second["turn"], 2);
    assert_eq!(second["prompt"], "second");
    assert!(outcome.permission_decisions.is_empty());

    let transcript = fs::read_to_string(&transcript).expect("read transcript");
    assert_eq!(transcript.matches("\"kind\":\"session/new\"").count(), 1);
    assert_eq!(transcript.matches("\"kind\":\"session/prompt\"").count(), 2);
    assert!(!transcript.contains("journal_dir"));
    assert!(!transcript.contains("create_model_revision"));

    fs::remove_dir_all(root).expect("remove native session fixture");
}

#[tokio::test]
async fn native_agent_actually_invokes_an_arbitrary_mcp_without_cycle() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-mcp-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create arbitrary MCP workspace");
    let receipt = root.join("echo-receipt.json");
    let agent_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let mcp_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let launch = LaunchCommand {
        executable: agent_fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.mcp_servers = vec![
        McpServer::Stdio(
            McpServerStdio::new("echo-capability", &mcp_fixture)
                .args(vec!["--receipt".into(), receipt.display().to_string()])
                .env(vec![EnvVariable::new(
                    "SWEM_FIXTURE_SECRET",
                    "must-not-enter-agent-state",
                )]),
        ),
        McpServer::Stdio(McpServerStdio::new("unused", "must-not-be-started")),
    ];
    let nonce = "capability-fabric-7f5e0e94";
    let prompt = serde_json::json!({
        "fixture": "mcp-echo-v0.1",
        "server": "echo-capability",
        "nonce": nonce,
    })
    .to_string();

    let outcome = run_native_session(&launch, &agent_fixture, &root, &[prompt], &options)
        .await
        .expect("invoke arbitrary MCP through native agent");

    let reply: Value =
        serde_json::from_str(&outcome.turns[0].reply_text).expect("fixture reply JSON");
    assert_eq!(reply["mcp_result"]["nonce"], nonce);
    assert_eq!(reply["mcp_result"]["server"], "swem-mcp-echo");
    let witnessed: Value =
        serde_json::from_slice(&fs::read(&receipt).expect("MCP receipt")).expect("receipt JSON");
    assert_eq!(witnessed["nonce"], nonce);
    assert_eq!(witnessed["server"], "swem-mcp-echo");
    assert!(outcome.permission_decisions.is_empty());
    assert!(
        !root.join("journal").exists(),
        "Cycle journal must not exist"
    );
    assert!(
        outcome.turns[0].reply_text.contains(nonce),
        "agent result and external receipt must agree"
    );
    let agent_state = fs::read_to_string(root.join(".swem-echo-agent-session.json"))
        .expect("read persisted agent state");
    assert!(agent_state.contains("echo-capability"));
    assert!(!agent_state.contains("must-not-enter-agent-state"));
    assert!(!agent_state.contains("SWEM_FIXTURE_SECRET"));

    fs::remove_dir_all(root).expect("remove arbitrary MCP fixture");
}

#[tokio::test]
async fn surface_permission_preserves_acp_payload_and_gates_the_mcp_effect() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-permission-allow-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create permission workspace");
    let receipt = root.join("allow-receipt.json");
    let agent_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let mcp_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let launch = LaunchCommand {
        executable: agent_fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo-capability", &mcp_fixture)
            .args(vec!["--receipt".into(), receipt.display().to_string()]),
    )];
    let nonce = "surface-allow-97b1";
    let prompt = serde_json::json!({
        "fixture": "mcp-echo-permission-v0.1",
        "server": "echo-capability",
        "nonce": nonce,
        "report_tool_call": true,
    })
    .to_string();
    let run_root = root.clone();
    let run_agent = agent_fixture.clone();
    let task = tokio::spawn(async move {
        run_native_session(&launch, &run_agent, &run_root, &[prompt], &options).await
    });

    let permission =
        tokio::time::timeout(Duration::from_secs(2), control.next_permission_request())
            .await
            .expect("permission surfaced")
            .expect("permission lane remained open");
    assert_eq!(
        permission.provenance,
        NativePermissionProvenance::CorrelatedAgentReport
    );
    assert_eq!(
        permission.tool_call.tool_call_id.0.as_ref(),
        format!("echo-{nonce}")
    );
    assert_eq!(
        permission.tool_call.fields.title.as_deref(),
        Some("mcp__echo-capability__echo")
    );
    assert_eq!(
        permission.tool_call.fields.raw_input.as_ref().unwrap()["nonce"],
        nonce
    );
    assert_eq!(
        permission
            .options
            .iter()
            .map(|option| option.option_id.0.as_ref())
            .collect::<Vec<_>>(),
        ["allow-once", "reject-once"]
    );
    assert!(
        matches!(
            control.select_permission(permission.sequence, "invented-option"),
            Err(SupplyError::Protocol(_))
        ),
        "the surface cannot manufacture an ACP option"
    );
    control
        .select_permission(permission.sequence, "allow-once")
        .expect("select exact allow option");

    let outcome = task
        .await
        .expect("permission task join")
        .expect("permission-gated native session");
    let witnessed: Value =
        serde_json::from_slice(&fs::read(&receipt).expect("MCP receipt")).expect("receipt JSON");
    assert_eq!(witnessed["nonce"], nonce);
    assert_eq!(outcome.permission_decisions.len(), 1);
    let decision = &outcome.permission_decisions[0];
    assert!(decision.allowed);
    assert_eq!(decision.selected_option.as_deref(), Some("allow-once"));
    assert_eq!(
        decision.provenance,
        NativePermissionProvenance::CorrelatedAgentReport
    );
    assert_eq!(
        decision.decision_source,
        NativePermissionDecisionSource::Surface
    );

    fs::remove_dir_all(root).expect("remove permission workspace");
}

#[tokio::test]
async fn an_uncorrelated_agent_title_never_becomes_automatic_authority() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-permission-reject-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create reject workspace");
    let receipt = root.join("must-not-exist.json");
    let agent_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let mcp_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let launch = LaunchCommand {
        executable: agent_fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo-capability", &mcp_fixture)
            .args(vec!["--receipt".into(), receipt.display().to_string()]),
    )];
    let prompt = serde_json::json!({
        "fixture": "mcp-echo-permission-v0.1",
        "server": "echo-capability",
        "nonce": "surface-reject-482a",
        "report_tool_call": false,
        "title": "mcp__trusted-payments__transfer_everything",
    })
    .to_string();
    let run_root = root.clone();
    let run_agent = agent_fixture.clone();
    let task = tokio::spawn(async move {
        run_native_session(&launch, &run_agent, &run_root, &[prompt], &options).await
    });

    let permission =
        tokio::time::timeout(Duration::from_secs(2), control.next_permission_request())
            .await
            .expect("uncorrelated permission surfaced")
            .expect("permission lane remained open");
    assert_eq!(
        permission.provenance,
        NativePermissionProvenance::UncorrelatedAgentReport
    );
    assert_eq!(
        permission.tool_call.fields.title.as_deref(),
        Some("mcp__trusted-payments__transfer_everything")
    );
    control
        .select_permission(permission.sequence, "reject-once")
        .expect("reject exact permission option");
    let outcome = task
        .await
        .expect("reject task join")
        .expect("agent handles rejection");

    assert!(!receipt.exists(), "rejected MCP side effect must be absent");
    assert_eq!(outcome.permission_decisions.len(), 1);
    assert!(!outcome.permission_decisions[0].allowed);
    assert_eq!(
        outcome.permission_decisions[0].provenance,
        NativePermissionProvenance::UncorrelatedAgentReport
    );

    fs::remove_dir_all(root).expect("remove reject workspace");
}

#[tokio::test]
async fn cancelling_a_turn_resolves_its_pending_permission_as_cancelled() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-permission-cancel-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create permission cancellation workspace");
    let receipt = root.join("must-not-exist.json");
    let transcript = root.join("transcript.jsonl");
    let agent_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let mcp_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let launch = LaunchCommand {
        executable: agent_fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.transcript_path = Some(transcript.clone());
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo-capability", &mcp_fixture)
            .args(vec!["--receipt".into(), receipt.display().to_string()]),
    )];
    let prompt = serde_json::json!({
        "fixture": "mcp-echo-permission-v0.1",
        "server": "echo-capability",
        "nonce": "surface-cancel-c9f0",
    })
    .to_string();
    let run_root = root.clone();
    let run_agent = agent_fixture.clone();
    let task = tokio::spawn(async move {
        run_native_session(&launch, &run_agent, &run_root, &[prompt], &options).await
    });

    let permission =
        tokio::time::timeout(Duration::from_secs(2), control.next_permission_request())
            .await
            .expect("pending permission surfaced")
            .expect("permission lane remained open");
    assert!(control.cancel_active_turn().is_some());
    assert!(
        matches!(
            control.select_permission(permission.sequence, "allow-once"),
            Err(SupplyError::Protocol(_))
        ),
        "cancelled permission must not be revivable"
    );
    let outcome = task
        .await
        .expect("cancel task join")
        .expect("cancel permission turn");

    assert!(
        !receipt.exists(),
        "cancelled MCP side effect must be absent"
    );
    assert_eq!(outcome.turns[0].stop_reason, "cancelled");
    assert_eq!(
        outcome.turns[0].control_outcome,
        NativeTurnControlOutcome::Cancelled
    );
    assert_eq!(outcome.permission_decisions.len(), 1);
    assert_eq!(outcome.permission_decisions[0].selected_option, None);
    assert!(
        fs::read_to_string(transcript)
            .expect("read permission transcript")
            .contains("surface cancelled the ACP permission request")
    );

    fs::remove_dir_all(root).expect("remove permission cancellation workspace");
}

#[tokio::test]
async fn arbitrary_mcp_request_fails_closed_when_attachment_is_missing() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-mcp-missing-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create missing MCP workspace");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let prompt = serde_json::json!({
        "fixture": "mcp-echo-v0.1",
        "server": "not-attached",
        "nonce": "must-not-succeed",
    })
    .to_string();

    let error = run_native_session(
        &launch,
        &fixture,
        &root,
        &[prompt],
        &NativeSessionOptions::new(Duration::from_secs(10)),
    )
    .await
    .expect_err("missing MCP attachment must fail closed");
    assert!(matches!(error, SupplyError::Protocol(_)));

    fs::remove_dir_all(root).expect("remove missing MCP fixture");
}

#[tokio::test]
async fn external_control_cancels_only_the_active_native_turn() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-cancel-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create cancellation workspace");
    let transcript = root.join("transcript.jsonl");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    assert_eq!(
        control.cancel_active_turn(),
        None,
        "cancellation must never queue before a prompt"
    );
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.transcript_path = Some(transcript.clone());
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_fixture = fixture.clone();
    let task = tokio::spawn(async move {
        run_native_session(
            &launch,
            &run_fixture,
            &run_root,
            &[serde_json::json!({ "fixture": "cancel-v0.1" }).to_string()],
            &options,
        )
        .await
    });

    let active = tokio::time::timeout(Duration::from_secs(2), control.wait_for_active_turn())
        .await
        .expect("fixture reached active prompt")
        .expect("session did not finish before cancellation");
    assert_eq!(control.cancel_active_turn(), Some(active));
    let outcome = task
        .await
        .expect("cancellation task join")
        .expect("cancel native turn");

    assert_eq!(outcome.turns.len(), 1);
    assert_eq!(outcome.turns[0].stop_reason, "cancelled");
    assert_eq!(
        outcome.turns[0].control_outcome,
        NativeTurnControlOutcome::Cancelled
    );
    assert!(outcome.turns[0].reply_text.is_empty());
    assert_eq!(control.phase(), NativeSessionPhase::Finished);
    assert_eq!(control.cancel_active_turn(), None);
    let transcript = fs::read_to_string(transcript).expect("read cancellation transcript");
    assert_eq!(transcript.matches("\"kind\":\"session/cancel\"").count(), 1);
    assert!(transcript.contains("\"control_outcome\":\"cancelled\""));

    fs::remove_dir_all(root).expect("remove cancellation fixture");
}

#[tokio::test]
async fn completion_racing_cancel_is_not_misreported_as_cancelled() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-cancel-race-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create cancellation race workspace");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_fixture = fixture.clone();
    let task = tokio::spawn(async move {
        run_native_session(
            &launch,
            &run_fixture,
            &run_root,
            &[serde_json::json!({ "fixture": "cancel-race-v0.1" }).to_string()],
            &options,
        )
        .await
    });

    let active = tokio::time::timeout(Duration::from_secs(2), control.wait_for_active_turn())
        .await
        .expect("fixture reached active prompt")
        .expect("session did not finish before cancellation race");
    assert_eq!(control.cancel_active_turn(), Some(active));
    let outcome = task
        .await
        .expect("cancellation race task join")
        .expect("complete cancellation race");

    assert_eq!(outcome.turns[0].stop_reason, "end_turn");
    assert_eq!(
        outcome.turns[0].control_outcome,
        NativeTurnControlOutcome::CompletedBeforeCancel
    );
    assert_ne!(outcome.turns[0].stop_reason, "cancelled");

    fs::remove_dir_all(root).expect("remove cancellation race fixture");
}

#[tokio::test]
async fn cancellation_protocol_failure_and_transport_timeout_remain_distinct() {
    for (fixture_name, expected) in [
        ("cancel-error-v0.1", "protocol"),
        ("cancel-ignore-v0.1", "timeout"),
    ] {
        let root = std::env::temp_dir().join(format!(
            "swem-native-cancel-{expected}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create cancellation failure workspace");
        let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
        let launch = LaunchCommand {
            executable: fixture.display().to_string(),
            args: Vec::new(),
            integration: IntegrationKind::DirectAcp,
        };
        let control = NativeSessionControl::new();
        let mut options = NativeSessionOptions::new(Duration::from_secs(1));
        options.control = Some(control.clone());
        let run_root = root.clone();
        let run_fixture = fixture.clone();
        let prompt = serde_json::json!({ "fixture": fixture_name }).to_string();
        let task = tokio::spawn(async move {
            run_native_session(&launch, &run_fixture, &run_root, &[prompt], &options).await
        });

        tokio::time::timeout(Duration::from_secs(2), control.wait_for_active_turn())
            .await
            .expect("fixture reached active prompt")
            .expect("session did not finish before cancellation failure");
        assert!(control.cancel_active_turn().is_some());
        let error = task
            .await
            .expect("cancellation failure task join")
            .expect_err("non-compliant fixture must not produce a turn outcome");
        match expected {
            "protocol" => assert!(matches!(error, SupplyError::Protocol(_))),
            "timeout" => assert_eq!(error, SupplyError::SessionTimeout),
            _ => unreachable!(),
        }
        assert_eq!(control.phase(), NativeSessionPhase::Finished);
        fs::remove_dir_all(root).expect("remove cancellation failure fixture");
    }
}

#[tokio::test]
#[ignore = "live native-agent MCP invocation: requires SWEM_NATIVE_AGENT and credentials"]
#[allow(
    clippy::too_many_lines,
    reason = "one live transaction keeps handshake, optional permission routing and the out-of-band MCP receipt auditable"
)]
async fn live_native_agent_invokes_arbitrary_mcp_without_cycle() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=<catalog id> for this explicit live test");
    let discovery = discover_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .unwrap_or_else(|| panic!("unknown catalog agent {agent_id}"));
    assert_ne!(discovery.readiness, Readiness::Absent, "agent is absent");
    let handshake = verify_discovered_agent(&discovery, Duration::from_secs(30))
        .await
        .expect("verify live ACP agent");
    assert_eq!(handshake.readiness, Readiness::HandshakeReady);
    let launch = discovery
        .launch
        .as_ref()
        .expect("live agent launch command");
    let executable = discovery
        .executable_path
        .as_ref()
        .expect("live agent executable");

    let root = std::env::temp_dir().join(format!(
        "swem-live-native-mcp-{agent_id}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create live MCP workspace");
    let receipt = root.join("echo-receipt.json");
    let transcript = root.join("transcript.jsonl");
    let mcp_fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let nonce = format!("live-{agent_id}-9c2f70e1");
    let mut options = NativeSessionOptions::new(Duration::from_secs(180));
    options.transcript_path = Some(transcript.clone());
    let control = NativeSessionControl::new();
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.mcp_servers = vec![McpServer::Stdio(
        McpServerStdio::new("echo", &mcp_fixture)
            .args(vec!["--receipt".into(), receipt.display().to_string()]),
    )];
    let prompt = format!(
        "Call the MCP tool `echo` from the attached server `echo` exactly once with nonce `{nonce}`. Do not use shell or file tools. Report its structured result."
    );

    let run_root = root.clone();
    let launch = launch.clone();
    let executable = executable.clone();
    let task = tokio::spawn(async move {
        run_native_session(&launch, &executable, &run_root, &[prompt], &options).await
    });
    let permission =
        tokio::time::timeout(Duration::from_secs(120), control.next_permission_request())
            .await
            .expect("live permission observation timed out");
    if let Some(permission) = permission {
        let allow_once = permission
            .options
            .iter()
            .find(|option| option.kind == PermissionOptionKind::AllowOnce)
            .expect("live agent offered allow_once")
            .option_id
            .0
            .to_string();
        control
            .select_permission(permission.sequence, &allow_once)
            .expect("select live allow_once");
    }
    let outcome = task
        .await
        .expect("live MCP task join")
        .unwrap_or_else(|error| {
            panic!(
                "live arbitrary MCP invocation failed: {error}; transcript: {}",
                transcript.display()
            )
        });
    let witnessed: Value = serde_json::from_slice(&fs::read(&receipt).unwrap_or_else(|error| {
        panic!(
            "agent completed without external MCP receipt ({error}); transcript: {}",
            transcript.display()
        )
    }))
    .expect("live MCP receipt JSON");
    assert_eq!(witnessed["nonce"], nonce);
    assert_eq!(witnessed["server"], "swem-mcp-echo");
    assert_eq!(outcome.turns.len(), 1);
    assert!(
        outcome
            .permission_decisions
            .iter()
            .all(|decision| decision.decision_source == NativePermissionDecisionSource::Surface)
    );
    assert!(
        outcome.turns[0]
            .tool_calls
            .iter()
            .any(|call| call.title == "mcp__echo__echo" || call.title.contains("echo")),
        "adapter exposed no echo tool call: {:?}; transcript: {}",
        outcome.turns[0].tool_calls,
        transcript.display()
    );
    eprintln!("live MCP evidence: {}", root.display());
}

#[tokio::test]
#[ignore = "live native-agent cancellation: requires SWEM_NATIVE_AGENT and credentials"]
async fn live_native_agent_confirms_active_turn_cancellation() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=<catalog id> for this explicit live test");
    let discovery = discover_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .unwrap_or_else(|| panic!("unknown catalog agent {agent_id}"));
    assert_ne!(discovery.readiness, Readiness::Absent, "agent is absent");
    let handshake = verify_discovered_agent(&discovery, Duration::from_secs(30))
        .await
        .expect("verify live ACP agent");
    assert_eq!(handshake.readiness, Readiness::HandshakeReady);
    let launch = discovery.launch.expect("live agent launch command");
    let executable = discovery
        .executable_path
        .expect("live agent executable path");
    let root = std::env::temp_dir().join(format!(
        "swem-live-native-cancel-{agent_id}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create live cancellation workspace");
    let transcript = root.join("transcript.jsonl");
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::new(Duration::from_secs(60));
    options.transcript_path = Some(transcript.clone());
    options.control = Some(control.clone());
    let run_root = root.clone();
    let task = tokio::spawn(async move {
        run_native_session(
            &launch,
            &executable,
            &run_root,
            &["Produce a very long, detailed analysis of every integer from 1 through 10000. Do not use tools.".into()],
            &options,
        )
        .await
    });

    let active = tokio::time::timeout(Duration::from_secs(30), control.wait_for_active_turn())
        .await
        .expect("live agent reached active prompt")
        .expect("live session finished before cancellation");
    assert_eq!(control.cancel_active_turn(), Some(active));
    let outcome = task
        .await
        .expect("live cancellation task join")
        .unwrap_or_else(|error| {
            panic!(
                "live native cancellation failed: {error}; transcript: {}",
                transcript.display()
            )
        });
    assert_eq!(outcome.turns.len(), 1);
    assert_eq!(outcome.turns[0].stop_reason, "cancelled");
    assert_eq!(
        outcome.turns[0].control_outcome,
        NativeTurnControlOutcome::Cancelled
    );
    eprintln!("live cancellation evidence: {}", root.display());
}

#[tokio::test]
async fn native_agent_owns_recovery_across_independent_host_connections() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-recovery-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create recovery workspace");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };

    let first = run_native_session(
        &launch,
        &fixture,
        &root,
        &["first process".into()],
        &NativeSessionOptions::new(Duration::from_secs(10)),
    )
    .await
    .expect("create native session");

    let mut load_options = NativeSessionOptions::new(Duration::from_secs(10));
    load_options.start = NativeSessionStart::Load {
        session_id: first.session_id.clone(),
    };
    let loaded = run_native_session(
        &launch,
        &fixture,
        &root,
        &["second process".into()],
        &load_options,
    )
    .await
    .expect("load native session in a fresh agent process");
    assert_eq!(loaded.session_id, first.session_id);
    assert_eq!(loaded.start, load_options.start);
    assert_eq!(loaded.replayed_updates, 2, "one prior user/agent pair");
    let loaded_reply: Value =
        serde_json::from_str(&loaded.turns[0].reply_text).expect("loaded reply JSON");
    assert_eq!(loaded_reply["turn"], 2);
    assert_eq!(loaded_reply["prompt"], "second process");

    let mut resume_options = NativeSessionOptions::new(Duration::from_secs(10));
    resume_options.start = NativeSessionStart::Resume {
        session_id: first.session_id.clone(),
    };
    let resumed = run_native_session(
        &launch,
        &fixture,
        &root,
        &["third process".into()],
        &resume_options,
    )
    .await
    .expect("resume native session in another fresh agent process");
    assert_eq!(resumed.session_id, first.session_id);
    assert_eq!(resumed.start, resume_options.start);
    assert_eq!(
        resumed.replayed_updates, 0,
        "resume must not replay history"
    );
    let resumed_reply: Value =
        serde_json::from_str(&resumed.turns[0].reply_text).expect("resumed reply JSON");
    assert_eq!(resumed_reply["turn"], 3);
    assert_eq!(resumed_reply["prompt"], "third process");

    fs::remove_dir_all(root).expect("remove native recovery fixture");
}

#[tokio::test]
async fn recovery_is_never_attempted_without_the_negotiated_capability() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-capability-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create capability workspace");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let first = run_native_session(
        &launch,
        &fixture,
        &root,
        &["accepted first turn".into()],
        &NativeSessionOptions::new(Duration::from_secs(10)),
    )
    .await
    .expect("create native session");

    let hidden = LaunchCommand {
        args: vec!["--hide-recovery".into()],
        ..launch.clone()
    };
    for start in [
        NativeSessionStart::Load {
            session_id: first.session_id.clone(),
        },
        NativeSessionStart::Resume {
            session_id: first.session_id.clone(),
        },
    ] {
        let mut options = NativeSessionOptions::new(Duration::from_secs(10));
        options.start = start;
        let error = run_native_session(
            &hidden,
            &fixture,
            &root,
            &["must never reach the agent".into()],
            &options,
        )
        .await
        .expect_err("unadvertised recovery must fail closed");
        assert!(matches!(error, SupplyError::Protocol(_)));
    }

    let mut resume = NativeSessionOptions::new(Duration::from_secs(10));
    resume.start = NativeSessionStart::Resume {
        session_id: first.session_id.clone(),
    };
    let final_turn = run_native_session(
        &launch,
        &fixture,
        &root,
        &["accepted second turn".into()],
        &resume,
    )
    .await
    .expect("resume after rejected attempts");
    let reply: Value =
        serde_json::from_str(&final_turn.turns[0].reply_text).expect("final reply JSON");
    assert_eq!(reply["turn"], 2, "rejected starts sent no hidden prompts");

    fs::remove_dir_all(root).expect("remove capability fixture");
}

/// A refused handshake must leave both halves of the story behind.
///
/// A walk once read `events=[]` after an agent answered `initialize` with "the
/// client did not advertise form elicitation". The route recorded only the
/// *answer*, and a refusal never reaches that line, so the one fact that
/// would settle the question — what the host actually advertised — was
/// nowhere. This drives the fixture into exactly that refusal and asks the
/// transcript and the surface lane what they kept.
#[tokio::test]
async fn a_refused_initialize_records_what_the_host_advertised_and_why_it_was_refused() {
    let root = std::env::temp_dir().join(format!(
        "swem-native-initialize-refused-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create refusal workspace");
    let transcript = root.join("transcript.jsonl");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: fixture.display().to_string(),
        args: vec![
            "--initialize-elicitation-receipt".into(),
            root.join("initialize-receipt.json").display().to_string(),
        ],
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    // The lane has to be owned before the handshake, or the events the
    // handshake publishes are dropped on the floor by design.
    let mut events = control.take_surface_events().expect("own the surface lane");
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.transcript_path = Some(transcript.clone());
    options.control = Some(control.clone());
    // No elicitation advertised: this is the whole point of the fixture flag.
    options.elicitation_capabilities = None;

    let error = run_native_session(&launch, &fixture, &root, &["hello".to_string()], &options)
        .await
        .expect_err("the fixture refuses a client that advertised no form elicitation");
    let complaint = error.to_string();
    assert!(
        complaint.contains("elicitation"),
        "the agent's own words should survive to the caller: {complaint}"
    );

    let lines: Vec<Value> = fs::read_to_string(&transcript)
        .expect("transcript written")
        .lines()
        .map(|line| serde_json::from_str(line).expect("transcript line is JSON"))
        .collect();
    let kinds: Vec<&str> = lines
        .iter()
        .filter_map(|line| line.get("kind").and_then(Value::as_str))
        .collect();
    assert!(
        kinds.contains(&"initialize_requested"),
        "the claim must be written down before it is made: {kinds:?}"
    );
    assert!(
        kinds.contains(&"initialize_refused"),
        "the refusal must be written down: {kinds:?}"
    );
    assert!(
        !kinds.contains(&"initialize"),
        "there was no successful handshake to record: {kinds:?}"
    );

    let requested = lines
        .iter()
        .find(|line| line.get("kind").and_then(Value::as_str) == Some("initialize_requested"))
        .and_then(|line| line.get("payload"))
        .expect("the claim carries a payload");
    assert!(
        requested.get("elicitation").is_none(),
        "this run advertised no elicitation, and the record must say so rather than \
         repeating what the code usually sends: {requested}"
    );
    let refused = lines
        .iter()
        .find(|line| line.get("kind").and_then(Value::as_str) == Some("initialize_refused"))
        .and_then(|line| line.get("payload"))
        .expect("the refusal carries a payload");
    assert_eq!(
        refused.get("advertised"),
        Some(requested),
        "the refusal repeats the same claim, so one line settles the question"
    );
    assert!(
        refused
            .get("error")
            .and_then(Value::as_str)
            .is_some_and(|reason| reason.contains("elicitation")),
        "the refusal keeps the agent's reason: {refused}"
    );

    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event.kind);
    }
    assert!(
        seen.iter().any(|kind| kind == "acp/initialize_requested"),
        "a surface watching this connection sees the claim: {seen:?}"
    );
    assert!(
        seen.iter().any(|kind| kind == "acp/initialize_refused"),
        "a surface watching this connection sees the refusal: {seen:?}"
    );

    let _ = fs::remove_dir_all(&root);
}
