use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionValue, SessionModeState,
};
use swem_host::{
    LaunchCommand, NativeSessionControl, NativeSessionOptions, Readiness, discover_agents,
    run_native_session, verify_discovered_agent,
};

const CONFIG_UPDATE_MARKER: &str = "SWEM_CONFIG_UPDATE";

fn fixture_root(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("fixture clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "swem-session-controls-{label}-{}-{nonce}",
        std::process::id()
    ))
}

async fn cleanup_live_workspace(root: PathBuf) {
    // Some ACP adapters release their Windows workspace handle just after the
    // stdio transport closes. Cleanup is not protocol evidence, so do not turn
    // a successful live round-trip into a false negative on that race.
    let mut cleanup_error = None;
    for _ in 0..10 {
        match fs::remove_dir_all(&root) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => {
                cleanup_error = Some(error);
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
    if let Some(error) = cleanup_error {
        eprintln!("live workspace cleanup deferred: {error}");
    }
}

fn fixture_launch(arguments: Vec<String>) -> (LaunchCommand, PathBuf) {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    (
        LaunchCommand {
            executable: executable.display().to_string(),
            args: arguments,
            integration: swem_host::IntegrationKind::AcpAdapter,
        },
        executable,
    )
}

fn ids(options: &[SessionConfigOption]) -> Vec<&str> {
    options.iter().map(|option| option.id.0.as_ref()).collect()
}

fn select_current<'a>(options: &'a [SessionConfigOption], id: &str) -> &'a str {
    let option = options
        .iter()
        .find(|option| option.id.0.as_ref() == id)
        .unwrap_or_else(|| panic!("missing config option {id}"));
    let SessionConfigKind::Select(select) = &option.kind else {
        panic!("config option {id} is not select");
    };
    select.current_value.0.as_ref()
}

fn current_value(option: &SessionConfigOption) -> SessionConfigOptionValue {
    match &option.kind {
        SessionConfigKind::Select(select) => {
            SessionConfigOptionValue::value_id(select.current_value.clone())
        }
        SessionConfigKind::Boolean(boolean) => {
            SessionConfigOptionValue::boolean(boolean.current_value)
        }
        _ => panic!("live agent exposed an unsupported config option kind"),
    }
}

fn legacy_current(modes: &SessionModeState) -> &str {
    modes.current_mode_id.0.as_ref()
}

#[tokio::test]
async fn exact_config_control_preserves_order_dependent_state_and_agent_updates() {
    let root = fixture_root("exact");
    fs::create_dir_all(&root).expect("create fixture root");
    let transcript = root.join("transcript.jsonl");
    let (launch, executable) = fixture_launch(Vec::new());
    let control = NativeSessionControl::new();
    let selection = {
        let control = control.clone();
        tokio::spawn(async move {
            control
                .set_config_option("model", SessionConfigOptionValue::value_id("quality"))
                .await
        })
    };
    tokio::task::yield_now().await;

    let mut options = NativeSessionOptions::new(Duration::from_secs(15));
    options.transcript_path = Some(transcript.clone());
    options.control = Some(control);
    let outcome = run_native_session(
        &launch,
        &executable,
        &root,
        &[CONFIG_UPDATE_MARKER.into()],
        &options,
    )
    .await
    .expect("run config fixture");
    let selected = selection
        .await
        .expect("join config selection")
        .expect("set config option");
    assert_eq!(ids(&selected), ["mode", "model", "thought_level", "brave"]);
    assert_eq!(select_current(&selected, "model"), "quality");
    assert_eq!(select_current(&selected, "thought_level"), "high");

    let final_options = outcome.config_options.expect("preferred config options");
    assert_eq!(
        ids(&final_options),
        ["mode", "model", "thought_level", "brave"]
    );
    assert_eq!(select_current(&final_options, "mode"), "code");
    assert_eq!(select_current(&final_options, "model"), "quality");
    assert_eq!(select_current(&final_options, "thought_level"), "high");
    assert_eq!(outcome.config_option_updates, 1);
    assert_eq!(outcome.legacy_mode_updates, 1);

    let kinds = fs::read_to_string(&transcript)
        .expect("read transcript")
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("parse transcript line")["kind"]
                .as_str()
                .expect("transcript kind")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let position = |kind: &str| {
        kinds
            .iter()
            .position(|candidate| candidate == kind)
            .unwrap_or_else(|| panic!("missing transcript event {kind}: {kinds:?}"))
    };
    assert!(position("session/new") < position("session/set_config_option"));
    assert!(position("session/set_config_option") < position("session/prompt"));
    assert!(position("session/prompt") < position("session/update"));
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn boolean_options_require_exact_client_capability_and_invalid_state_fails_closed() {
    let root = fixture_root("boolean");
    fs::create_dir_all(&root).expect("create fixture root");
    let (launch, executable) = fixture_launch(Vec::new());
    let control = NativeSessionControl::new();
    let selection = {
        let control = control.clone();
        tokio::spawn(async move {
            control
                .set_config_option("brave", SessionConfigOptionValue::boolean(true))
                .await
        })
    };
    tokio::task::yield_now().await;
    let mut options = NativeSessionOptions::new(Duration::from_secs(15));
    options.boolean_config_options = false;
    options.control = Some(control);
    let outcome = run_native_session(&launch, &executable, &root, &["baseline".into()], &options)
        .await
        .expect("run select-only client");
    let error = selection
        .await
        .expect("join boolean selection")
        .expect_err("unadvertised boolean selection must fail");
    assert!(error.to_string().contains("did not expose"));
    assert_eq!(
        ids(outcome.config_options.as_deref().expect("select options")),
        ["mode", "model", "thought_level"]
    );

    let (violating, violating_executable) =
        fixture_launch(vec!["--violate-boolean-capability".into()]);
    let error = run_native_session(
        &violating,
        &violating_executable,
        &root,
        &["must not prompt".into()],
        &options_without_control(false),
    )
    .await
    .expect_err("agent capability violation must fail closed");
    assert!(
        error
            .to_string()
            .contains("without exact client capability")
    );
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn duplicate_config_ids_are_rejected_before_prompt() {
    let root = fixture_root("duplicate");
    fs::create_dir_all(&root).expect("create fixture root");
    let (launch, executable) = fixture_launch(vec!["--duplicate-config-id".into()]);
    let error = run_native_session(
        &launch,
        &executable,
        &root,
        &["must not prompt".into()],
        &NativeSessionOptions::new(Duration::from_secs(15)),
    )
    .await
    .expect_err("duplicate ids must fail closed");
    assert!(error.to_string().contains("non-empty and unique"));
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn legacy_modes_are_a_fallback_and_never_shadow_config_options() {
    let root = fixture_root("legacy");
    fs::create_dir_all(&root).expect("create fixture root");
    let (launch, executable) = fixture_launch(vec!["--legacy-modes-only".into()]);
    let control = NativeSessionControl::new();
    let selection = {
        let control = control.clone();
        tokio::spawn(async move { control.set_legacy_mode("code").await })
    };
    tokio::task::yield_now().await;
    let mut options = NativeSessionOptions::new(Duration::from_secs(15));
    options.control = Some(control);
    let outcome = run_native_session(&launch, &executable, &root, &["legacy".into()], &options)
        .await
        .expect("run legacy fixture");
    let selected = selection
        .await
        .expect("join legacy selection")
        .expect("set legacy mode");
    assert_eq!(legacy_current(&selected), "code");
    assert!(outcome.config_options.is_none());
    assert_eq!(
        legacy_current(outcome.legacy_modes.as_ref().expect("legacy modes")),
        "code"
    );
    assert_eq!(outcome.legacy_mode_updates, 1);
    fs::remove_dir_all(root).expect("remove fixture root");
}

fn options_without_control(boolean_config_options: bool) -> NativeSessionOptions {
    let mut options = NativeSessionOptions::new(Duration::from_secs(15));
    options.boolean_config_options = boolean_config_options;
    options
}

#[tokio::test]
#[ignore = "live ACP session controls: requires SWEM_NATIVE_AGENT and agent credentials"]
async fn live_agent_controls_are_exercised_or_reported_explicitly_unsupported() {
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
    let root = fixture_root(&format!("live-{agent_id}"));
    fs::create_dir_all(&root).expect("create live workspace");

    let first = run_native_session(
        &launch,
        &executable,
        &root,
        &["Reply with SWEM_CONFIG_PROBE_OK.".into()],
        &NativeSessionOptions::new(Duration::from_secs(180)),
    )
    .await
    .expect("probe live session controls");

    if let Some(config_options) = first
        .config_options
        .as_ref()
        .filter(|state| !state.is_empty())
    {
        let option = config_options.first().expect("non-empty config state");
        let control = NativeSessionControl::new();
        let selection = {
            let control = control.clone();
            let id = option.id.0.to_string();
            let value = current_value(option);
            tokio::spawn(async move { control.set_config_option(id, value).await })
        };
        tokio::task::yield_now().await;
        let mut options = NativeSessionOptions::new(Duration::from_secs(180));
        options.control = Some(control);
        run_native_session(
            &launch,
            &executable,
            &root,
            &["Reply with SWEM_CONFIG_SET_OK.".into()],
            &options,
        )
        .await
        .expect("run live config selection");
        let complete = selection
            .await
            .expect("join live config selection")
            .expect("set live config option");
        eprintln!(
            "live ACP session config evidence: agent={agent_id}, kind=config_options, count={}",
            complete.len()
        );
    } else if let Some(modes) = &first.legacy_modes {
        let control = NativeSessionControl::new();
        let selection = {
            let control = control.clone();
            let current = modes.current_mode_id.0.to_string();
            tokio::spawn(async move { control.set_legacy_mode(current).await })
        };
        tokio::task::yield_now().await;
        let mut options = NativeSessionOptions::new(Duration::from_secs(180));
        options.control = Some(control);
        run_native_session(
            &launch,
            &executable,
            &root,
            &["Reply with SWEM_LEGACY_MODE_SET_OK.".into()],
            &options,
        )
        .await
        .expect("run live legacy mode selection");
        selection
            .await
            .expect("join live legacy selection")
            .expect("set live legacy mode");
        eprintln!("live ACP session config evidence: agent={agent_id}, kind=legacy_modes");
    } else {
        eprintln!(
            "live ACP session config evidence: agent={agent_id}, kind=explicitly_unsupported"
        );
    }
    cleanup_live_workspace(root).await;
}
