use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    ContentBlock, SessionConfigKind, SessionConfigOptionValue, TextContent,
};
use serde_json::Value;
use swem_host::{
    IntegrationKind, LaunchCommand, NativeSessionControl, NativeSessionOptions, NativeSessionPhase,
    NativeSessionTermination, Readiness, SupplyError, discover_agents,
    run_interactive_native_session, verify_discovered_agent,
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
    root
}

fn text(value: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text(TextContent::new(value))]
}

async fn wait_until_idle(control: &NativeSessionControl) {
    tokio::time::timeout(Duration::from_mins(1), control.wait_until_ready())
        .await
        .expect("interactive session did not become idle")
        .expect("interactive session finished before idle");
}

#[tokio::test]
async fn interactive_driver_accepts_late_turns_and_disconnects_without_closing_session() {
    let root = fixture_workspace("interactive-session");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(10));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_executable = executable.clone();
    let run_launch = launch.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&run_launch, &run_executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    let configuration = control
        .set_config_option("model", SessionConfigOptionValue::value_id("quality"))
        .await
        .expect("change native configuration while idle");
    let model = configuration
        .iter()
        .find(|option| option.id.0.as_ref() == "model")
        .expect("model option");
    let SessionConfigKind::Select(model) = &model.kind else {
        panic!("model option must be select");
    };
    assert_eq!(model.current_value.0.as_ref(), "quality");
    let first = control
        .submit_prompt(text("late first"))
        .await
        .expect("first dynamic turn");
    let second = control
        .submit_prompt(text("late second"))
        .await
        .expect("second dynamic turn");
    control.disconnect().await.expect("graceful disconnect");
    let outcome = task
        .await
        .expect("interactive runner task")
        .expect("interactive session outcome");

    let first_reply: Value = serde_json::from_str(&first.reply_text).expect("first reply JSON");
    let second_reply: Value = serde_json::from_str(&second.reply_text).expect("second reply JSON");
    assert_eq!(first_reply["session_id"], outcome.session_id);
    assert_eq!(second_reply["session_id"], outcome.session_id);
    assert_eq!(first_reply["turn"], 1);
    assert_eq!(second_reply["turn"], 2);
    assert_eq!(outcome.turns, vec![first, second]);
    assert_eq!(outcome.termination, NativeSessionTermination::Disconnected);

    fs::remove_dir_all(root).expect("remove interactive fixture");
}

#[tokio::test]
async fn interactive_cancel_finishes_one_turn_and_keeps_the_connection_usable() {
    let root = fixture_workspace("interactive-cancel");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(10));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_executable = executable.clone();
    let run_launch = launch.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&run_launch, &run_executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    let prompt_control = control.clone();
    let prompt = tokio::spawn(async move {
        prompt_control
            .submit_prompt(text(r#"{"fixture":"cancel-v0.1"}"#))
            .await
    });
    let active = tokio::time::timeout(Duration::from_secs(2), control.wait_for_active_turn())
        .await
        .expect("active turn wait")
        .expect("active turn");
    assert_eq!(control.cancel_active_turn(), Some(active));
    let cancelled = prompt
        .await
        .expect("cancel prompt task")
        .expect("cancelled prompt outcome");
    assert_eq!(cancelled.stop_reason, "cancelled");
    let next = control
        .submit_prompt(text("after cancellation"))
        .await
        .expect("next turn after cancellation");
    assert!(next.reply_text.contains("after cancellation"));
    control.disconnect().await.expect("disconnect after cancel");
    let outcome = task
        .await
        .expect("cancel runner task")
        .expect("cancel session outcome");
    assert_eq!(outcome.turns, vec![cancelled, next]);

    fs::remove_dir_all(root).expect("remove cancel fixture");
}

#[tokio::test]
async fn batch_control_cannot_masquerade_as_an_interactive_driver() {
    let control = NativeSessionControl::new();
    let error = control
        .submit_prompt(text("never queued"))
        .await
        .expect_err("unattached prompt lane must fail synchronously");
    assert!(matches!(error, SupplyError::Protocol(message) if message.contains("not attached")));
}

#[tokio::test]
async fn interactive_close_is_capability_gated_and_distinct_from_disconnect() {
    let root = fixture_workspace("interactive-close");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut events = control
        .take_surface_events()
        .expect("take surface event lane");
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(10));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_executable = executable.clone();
    let run_launch = launch.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&run_launch, &run_executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    control
        .submit_prompt(text("close me"))
        .await
        .expect("turn before close");
    control.close_session().await.expect("ACP session close");
    let outcome = task
        .await
        .expect("close runner task")
        .expect("close session outcome");
    assert_eq!(outcome.termination, NativeSessionTermination::Closed);

    let mut kinds = Vec::new();
    while let Ok(event) = events.try_recv() {
        kinds.push(event.kind);
    }
    let close = kinds
        .iter()
        .position(|kind| kind == "acp/session_close")
        .expect("close surface event");
    let terminal = kinds
        .iter()
        .position(|kind| kind == "host/session_terminal")
        .expect("terminal surface event");
    assert!(close < terminal);

    fs::remove_dir_all(root).expect("remove close fixture");
}

#[tokio::test]
async fn unsupported_close_does_not_silently_disconnect_the_native_session() {
    let root = fixture_workspace("interactive-close-unsupported");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: vec!["--hide-close".into()],
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(10));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_executable = executable.clone();
    let run_launch = launch.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&run_launch, &run_executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    let error = control
        .close_session()
        .await
        .expect_err("unadvertised close must fail");
    assert!(
        matches!(error, SupplyError::Protocol(message) if message.contains("sessionCapabilities.close"))
    );
    let turn = control
        .submit_prompt(text("still connected"))
        .await
        .expect("session remains usable after rejected close");
    assert!(turn.reply_text.contains("still connected"));
    control
        .disconnect()
        .await
        .expect("disconnect after rejection");
    let outcome = task
        .await
        .expect("unsupported close runner task")
        .expect("unsupported close session outcome");
    assert_eq!(outcome.termination, NativeSessionTermination::Disconnected);

    fs::remove_dir_all(root).expect("remove unsupported close fixture");
}

#[tokio::test]
async fn interactive_idle_time_is_not_a_connection_deadline() {
    let root = fixture_workspace("interactive-idle-lifetime");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(1));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_executable = executable.clone();
    let run_launch = launch.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&run_launch, &run_executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    tokio::time::sleep(Duration::from_millis(1_250)).await;
    assert!(matches!(control.phase(), NativeSessionPhase::Idle { .. }));
    let turn = control
        .submit_prompt(text("after idle longer than operation timeout"))
        .await
        .expect("idle time must not expire the connection");
    assert!(
        turn.reply_text
            .contains("after idle longer than operation timeout")
    );
    control.disconnect().await.expect("disconnect after idle");
    let outcome = task
        .await
        .expect("idle runner task")
        .expect("idle session outcome");
    assert_eq!(outcome.turns, vec![turn]);

    fs::remove_dir_all(root).expect("remove idle lifetime fixture");
}

#[tokio::test]
async fn interactive_prompt_has_its_own_operation_deadline() {
    let root = fixture_workspace("interactive-operation-timeout");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let control = NativeSessionControl::new();
    let mut events = control
        .take_surface_events()
        .expect("take timeout surface lane");
    let mut options = NativeSessionOptions::interactive(Duration::from_millis(500));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let run_executable = executable.clone();
    let run_launch = launch.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&run_launch, &run_executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    let prompt_error = control
        .submit_prompt(text(r#"{"fixture":"cancel-ignore-v0.1"}"#))
        .await
        .expect_err("ignored prompt must hit its operation deadline");
    assert_eq!(
        prompt_error,
        SupplyError::OperationTimeout {
            operation: "session/prompt".into()
        }
    );
    let session_error = task
        .await
        .expect("timeout runner task")
        .expect_err("operation timeout must fail the connection");
    assert_eq!(session_error, prompt_error);
    assert_eq!(control.phase(), NativeSessionPhase::Finished);

    let mut terminal = None;
    while let Ok(event) = events.try_recv() {
        if event.kind == "host/session_terminal" {
            terminal = Some(event.payload);
        }
    }
    assert_eq!(
        terminal.as_ref().and_then(|value| value["status"].as_str()),
        Some("failed")
    );
    assert!(
        terminal
            .as_ref()
            .and_then(|value| value["error"].as_str())
            .is_some_and(|error| error.contains("session/prompt"))
    );

    fs::remove_dir_all(root).expect("remove operation timeout fixture");
}

#[tokio::test]
#[ignore = "live interactive ACP: requires SWEM_NATIVE_AGENT=claude-code and its lawful credential"]
async fn live_claude_accepts_two_turns_submitted_after_connection_start() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=claude-code for this explicit live test");
    assert_eq!(
        agent_id, "claude-code",
        "only Claude is authorized on this host"
    );
    let discovery = discover_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .expect("Claude catalog entry");
    assert!(matches!(
        discovery.readiness,
        Readiness::InstalledUnverified | Readiness::HandshakeReady
    ));
    let verified = verify_discovered_agent(&discovery, Duration::from_secs(30))
        .await
        .expect("verify installed Claude ACP adapter");
    assert_eq!(verified.readiness, Readiness::HandshakeReady);
    let launch = discovery.launch.expect("verified Claude launch command");
    let executable = discovery
        .executable_path
        .expect("verified Claude executable path");
    let root = fixture_workspace("interactive-live-claude");
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::interactive(Duration::from_mins(3));
    options.control = Some(control.clone());
    let run_root = root.clone();
    let task = tokio::spawn(async move {
        run_interactive_native_session(&launch, &executable, &run_root, &options).await
    });

    wait_until_idle(&control).await;
    let first = control
        .submit_prompt(text(
            "Reply with the exact marker SWEM_INTERACTIVE_LIVE_044_A and nothing else.",
        ))
        .await
        .expect("first live dynamic turn");
    let second = control
        .submit_prompt(text(
            "Reply with the exact marker SWEM_INTERACTIVE_LIVE_044_B and nothing else.",
        ))
        .await
        .expect("second live dynamic turn");
    control.disconnect().await.expect("live disconnect");
    let outcome = task
        .await
        .expect("live interactive runner task")
        .expect("live interactive outcome");
    assert!(first.reply_text.contains("SWEM_INTERACTIVE_LIVE_044_A"));
    assert!(second.reply_text.contains("SWEM_INTERACTIVE_LIVE_044_B"));
    assert_eq!(outcome.turns.len(), 2);
    assert_eq!(outcome.termination, NativeSessionTermination::Disconnected);

    for _ in 0..10 {
        match fs::remove_dir_all(&root) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}
