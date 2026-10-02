//! Browser gate for the generic Apps host (H1): a REAL headless browser
//! opens the shell, the official `AppBridge` bundle mounts the discovered App
//! through the different-origin sandbox proxy, real viewport clicks inside
//! the sandboxed App produce a tool call whose external receipt is the
//! oracle, the hostile App is refused on every attempt (poison receipt
//! absent, shell alive), resume preserves the resource identity, and the
//! App-disabled leg completes the same semantic operation through the
//! ordinary agent tool path. Oracles: routing ledger, receipt files and
//! driver-reported identities - never pixels.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{ContentBlock, McpServer, McpServerStdio, TextContent};
use swem_host::{
    AttachmentBinding, AttachmentTransport, CredentialBindingRef, CredentialSourceRef,
    IntegrationKind, LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore,
    ResolvedDirectAgentConnection, RoutingLedger, ShellConnectionMode, WorkbenchShellState,
    serve_workbench_http_with_apps,
};

/// The credential a live profile binds for its agent: the variable the host
/// process holds (from the gitignored `.env.local`) under the name the agent
/// reads. The durable profile stores names only.
fn live_credential_bindings(agent_id: &str) -> Vec<CredentialBindingRef> {
    let name = match agent_id {
        "claude-code" => "CLAUDE_CODE_OAUTH_TOKEN",
        "opencode" => "OPENCODE_CONFIG_CONTENT",
        _ => return Vec::new(),
    };
    vec![CredentialBindingRef {
        binding_id: format!("{agent_id}-credential"),
        source: CredentialSourceRef::EnvironmentVariable { name: name.into() },
        target_environment: name.into(),
    }]
}

const NOTES_URI: &str = "ui://apps-fixture/notes";

fn browser_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SWEM_BROWSER") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    // The places a Chromium-family browser lives on the three platforms a
    // person runs this on; the same list as the product gate's.
    [
        "/opt/pw-browsers/chromium",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// `npm ci` (once) + esbuild the official `AppBridge` bundle into `out_dir`.
/// The committed lockfile is the provenance; the bundle is never committed.
/// `npm`/`npx`, launched the way the running platform can launch them.
///
/// On Windows these are batch shims that only a shell resolves; everywhere else
/// `cmd` does not exist, so spawning it fails with `NotFound` and this gate -
/// which is `#[ignore]`d, and therefore silent - never runs at all. The same
/// platform bug lived in the strict lint until `40153cc`; `tests/web_suite.rs`
/// already carries this shape.
fn node_tool(program: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("cmd");
        command.arg("/C").arg(program);
        command
    }
    #[cfg(not(windows))]
    std::process::Command::new(program)
}

fn build_apps_bundle(out_dir: &Path) {
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/apps-host");
    if !web.join("node_modules").is_dir() {
        let install = node_tool("npm")
            .args(["ci"])
            .current_dir(&web)
            .output()
            .expect("run npm ci");
        assert!(
            install.status.success(),
            "npm ci failed:\n{}",
            String::from_utf8_lossy(&install.stderr)
        );
    }
    let outfile = format!("--outfile={}", out_dir.join("apps-bridge.js").display());
    let bundle = node_tool("npx")
        .args([
            "esbuild",
            "apps-bridge.entry.mjs",
            "--bundle",
            "--format=iife",
            &outfile,
        ])
        .current_dir(&web)
        .output()
        .expect("run esbuild");
    assert!(
        bundle.status.success(),
        "esbuild failed:\n{}",
        String::from_utf8_lossy(&bundle.stderr)
    );
}

/// The session of the engine a chat of the walk is in, for the oracles
/// that read the session's own events.
fn route_of(ledger: &Path, report: &serde_json::Value, which: &str) -> String {
    let chat = report[format!("{which}_chat")].as_str().expect("a chat");
    let agent = report[format!("{which}_agent")].as_str().expect("an agent");
    RoutingLedger::open(ledger)
        .expect("open ledger")
        .current_session(chat, agent)
        .expect("read the session")
        .expect("the chat holds a session")
        .route_id
}

fn events_of(ledger: &Path, route_id: &str) -> Vec<(String, serde_json::Value)> {
    RoutingLedger::open(ledger)
        .expect("reopen routing ledger")
        .events_for_surface(route_id, "oracle", 5000)
        .expect("read the full ordered lane")
        .events
        .into_iter()
        .map(|event| (event.kind, event.payload))
        .collect()
}

async fn remove_browser_fixture_root(root: &Path) {
    // Windows can keep a just-exited process' current-directory handle alive
    // for a few scheduler ticks after the process-tree wait completed. This is
    // test-harness cleanup, not transaction evidence: require eventual exact
    // removal and fail if the handle is persistent.
    let mut last_error = None;
    for _ in 0..25 {
        match fs::remove_dir_all(root) {
            Ok(()) => return,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => last_error = Some(error),
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
    panic!(
        "remove browser fixture root {}: {}",
        root.display(),
        last_error.expect("at least one removal attempt failed")
    );
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome and Node.js"]
#[allow(
    clippy::too_many_lines,
    reason = "one browser transaction keeps form, URL consent and all independent receipts together"
)]
async fn a_real_browser_reviews_native_acp_form_and_url_elicitation() {
    let Some(browser) = browser_path() else {
        eprintln!("skipped: no browser on this machine (set SWEM_BROWSER to one)");
        return;
    };
    assert!(node_available(), "put Node.js on PATH");
    let root = std::env::temp_dir().join(format!(
        "swem-elicitation-browser-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    PersonalAgentProfileStore::open(&inventory)
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                "elicitation-agent",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .expect("build fixture profile"),
        )
        .expect("persist fixture profile");
    let state = WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(30), {
        let initialize_receipt = root.join("initialize-elicitation-receipt.json");
        move |_profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: vec![
                        "--initialize-elicitation-receipt".into(),
                        initialize_receipt.display().to_string(),
                    ],
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: Vec::new(),
            })
        }
    })
    .expect("open shell state");
    let state = Arc::new(state);
    let handle =
        serve_workbench_http_with_apps(Arc::clone(&state), ([127, 0, 0, 1], 0).into(), None)
            .await
            .expect("bind Workbench");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_shell_elicitation_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(90),
        tokio::process::Command::new("node")
            .arg(driver)
            .arg(browser)
            .arg(url)
            .output(),
    )
    .await
    .expect("browser drive timed out")
    .expect("node failed to start");
    assert!(
        output.status.success(),
        "driver failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver report"),
    )
    .expect("parse driver report");
    assert!(
        report["form_context"]
            .as_str()
            .is_some_and(|value| value.contains("elicitation-agent asks")),
        "the form does not say who asks: {report}"
    );
    assert!(
        report["url_context"]
            .as_str()
            .is_some_and(|value| value.contains("https://example.invalid/connect")
                && value.contains("at example.invalid")),
        "the link does not say where it leads: {report}"
    );
    assert_eq!(
        report["pages_after"].as_u64(),
        report["pages_before"].as_u64().map(|n| n + 1)
    );
    state.let_go_of(None).await;
    let initialize: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("initialize-elicitation-receipt.json"))
            .expect("read initialize elicitation receipt"),
    )
    .expect("initialize elicitation receipt JSON");
    assert_eq!(initialize["action"], "accept");
    assert_eq!(
        initialize["content"]["workspace_label"],
        "browser-pre-session"
    );
    let form: serde_json::Value = serde_json::from_slice(
        &fs::read(workspace.join(".swem-form-elicitation-receipt.json"))
            .expect("read form receipt"),
    )
    .expect("form receipt JSON");
    assert_eq!(form["action"], "accept");
    assert_eq!(form["content"]["strategy"], "bold");
    assert_eq!(form["content"]["iterations"], 3);
    assert_eq!(form["content"]["stems"], serde_json::json!(["voice", "fx"]));
    let url_receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(workspace.join(".swem-url-elicitation-receipt.json")).expect("read URL receipt"),
    )
    .expect("URL receipt JSON");
    assert_eq!(url_receipt, serde_json::json!({"action": "accept"}));
    // Everything the chat's record holds: the engine's own events and what
    // the host wrote about the questions and their answers.
    let chat = report["chat"].as_str().expect("the chat");
    let (events, _, _) = RoutingLedger::open(&ledger)
        .expect("open ledger")
        .timeline(chat, None, 5000)
        .expect("read the chat");
    let durable = serde_json::to_string(&events).expect("encode events");
    assert!(
        durable.contains("acp/elicitation_complete"),
        "known URL completion was not projected"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.kind == "chat/question"
                && event.payload["state"] == "answered"
                && event.payload["answer"]["action"] == "accept")
            .count(),
        3,
        "three questions were asked and answered: before the session, the form, the link"
    );
    for forbidden in [
        "bold",
        "voice",
        "example.invalid",
        "fixture-oauth-1",
        "Review and submit the non-sensitive",
        "browser-pre-session",
    ] {
        assert!(
            !durable.contains(forbidden),
            "the chat's record leaked {forbidden}"
        );
    }

    handle.shutdown().await;
    drop(state);
    remove_browser_fixture_root(&root).await;
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome, Node.js and npm (first run fetches the committed lockfile's packages)"]
#[allow(
    clippy::too_many_lines,
    reason = "one browser acceptance scenario keeps the bundle build, the driver run and every durable oracle together"
)]
async fn a_real_browser_drives_the_generic_apps_host_through_the_whole_acceptance() {
    let Some(browser) = browser_path() else {
        eprintln!("skipped: no browser on this machine (set SWEM_BROWSER to one)");
        return;
    };
    assert!(node_available(), "put Node.js on PATH");

    let root = std::env::temp_dir().join(format!(
        "swem-apps-browser-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let bundle_dir = root.join("bundle");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    build_apps_bundle(&bundle_dir);

    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    for profile_id in ["apps-main", "apps-hostile"] {
        store
            .create(
                &PersonalAgentProfile::new(
                    profile_id,
                    "swem-echo-agent",
                    "echo-fixture-distribution",
                    "direct-fixture-environment",
                    "surface-permissions",
                    &workspace,
                    &agent_home,
                    vec![
                        AttachmentBinding::new(
                            "notes-attachment",
                            "notes",
                            AttachmentTransport::Stdio,
                        ),
                        AttachmentBinding::new(
                            "echo-attachment",
                            "echo",
                            AttachmentTransport::Stdio,
                        ),
                    ],
                    Vec::new(),
                )
                .expect("build fixture profile"),
            )
            .expect("persist fixture profile");
    }
    let root_for_resolver = root.clone();
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_secs(30),
        move |profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            let hostile = profile.profile_id == "apps-hostile";
            let suffix = if hostile { "hostile" } else { "main" };
            let mut notes_args = vec![
                "--receipt".to_owned(),
                root_for_resolver
                    .join(format!("notes-receipt-{suffix}.json"))
                    .display()
                    .to_string(),
                "--poison".to_owned(),
                root_for_resolver
                    .join(format!("poison-receipt-{suffix}.json"))
                    .display()
                    .to_string(),
            ];
            if hostile {
                notes_args.push("--hostile".to_owned());
            }
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![
                    McpServer::Stdio(
                        McpServerStdio::new(
                            "notes",
                            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                        )
                        .args(notes_args),
                    ),
                    McpServer::Stdio(
                        McpServerStdio::new(
                            "echo",
                            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo")),
                        )
                        .args(vec![
                            "--receipt".into(),
                            root_for_resolver
                                .join("echo-receipt.json")
                                .display()
                                .to_string(),
                        ]),
                    ),
                ],
            })
        },
    )
    .expect("open shell state");
    let state = Arc::new(state);
    let handle = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle_dir),
    )
    .await
    .expect("bind shell + sandbox listeners");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());

    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workbench_shell_apps_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_mins(4),
        tokio::process::Command::new("node")
            .arg(&driver)
            .arg(&browser)
            .arg(&url)
            .output(),
    )
    .await
    .expect("browser drive timed out")
    .expect("node failed to start");
    assert!(
        output.status.success(),
        "driver failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver reported its identities"),
    )
    .expect("parse driver report");
    assert_eq!(report["app_server"], "notes");
    assert_eq!(report["app_uri"], NOTES_URI);
    let main_route = &route_of(&ledger, &report, "main");
    let hostile_route = &route_of(&ledger, &report, "hostile");

    // The browser click inside the sandboxed App really ran the tool.
    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt-main.json")).expect("read browser receipt"),
    )
    .expect("parse browser receipt");
    assert_eq!(receipt["nonce"], "browser-app-nonce");
    assert_eq!(receipt["text"], "clicked inside a real sandboxed App");

    // Durable decisions on the main route: two opens with the identical
    // identity (before and after resume), one allowed save_note, two closes.
    let main_events = events_of(&ledger, main_route);
    let opened: Vec<&serde_json::Value> = main_events
        .iter()
        .filter(|(kind, _)| kind == "host/app_opened")
        .map(|(_, payload)| payload)
        .collect();
    assert_eq!(
        opened.len(),
        2,
        "app_opened events: {opened:?}; all main events: {:?}",
        main_events.iter().map(|(kind, _)| kind).collect::<Vec<_>>()
    );
    assert!(opened.iter().all(|payload| payload["uri"] == NOTES_URI));
    assert!(
        main_events.iter().any(|(kind, payload)| {
            kind == "host/app_tool_call"
                && payload["decision"] == "allowed"
                && payload["tool"] == "save_note"
        }),
        "the allowed tool call is not on the durable lane"
    );
    assert_eq!(
        main_events
            .iter()
            .filter(|(kind, _)| kind == "host/app_closed")
            .count(),
        2
    );
    assert!(
        main_events
            .iter()
            .any(|(kind, _)| kind == "acp/session_resume"),
        "resume is not visible on the durable lane"
    );

    // The hostile App: every attempt refused, poison and cross-server
    // receipts absent, and the shell completed an ordinary prompt afterwards.
    let hostile_events = events_of(&ledger, hostile_route);
    // Three refusals reach the Rust relay (model-only, undeclared,
    // cross-server); the hostile ui/* attempt is intercepted by the official
    // bridge before the relay - the Rust ui/* gate is component-proven.
    let refused: Vec<String> = hostile_events
        .iter()
        .filter(|(kind, payload)| kind == "host/app_tool_call" && payload["decision"] == "refused")
        .map(|(_, payload)| payload["tool"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        refused.len() >= 3,
        "expected at least 3 refusals, saw {refused:?}"
    );
    for tool in ["model_only_probe", "no_such_tool", "echo"] {
        assert!(
            refused.iter().any(|name| name == tool),
            "missing refusal for {tool}: {refused:?}"
        );
    }
    assert!(
        !root.join("poison-receipt-hostile.json").exists(),
        "the model-only probe executed - the host gate failed"
    );
    assert!(
        !root.join("echo-receipt.json").exists(),
        "the cross-server tool executed - the host gate failed"
    );
    assert!(
        hostile_events.iter().any(|(kind, payload)| {
            kind == "host/prompt_submitted"
                && payload
                    .to_string()
                    .contains("still alive after the hostile app")
        }),
        "the shell did not survive the hostile app"
    );

    // App-disabled mode: the SAME semantic operation through the ordinary
    // agent tool path (no bridge, no iframe), receipt of the same schema.
    let (connection, _route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open the app-disabled connection");
    let turn = state
        .submit_prompt(
            &connection,
            vec![ContentBlock::Text(TextContent::new(
                serde_json::json!({
                    "fixture": "mcp-apps-note-v0.1",
                    "server": "notes",
                    "nonce": "headless-note-nonce",
                    "text": "same operation without the App",
                })
                .to_string(),
            ))],
        )
        .await
        .expect("app-disabled turn");
    assert!(turn.reply_text.contains("headless-note-nonce"));
    state.disconnect(&connection).await.expect("disconnect");
    let disabled_receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt-main.json")).expect("read app-disabled receipt"),
    )
    .expect("parse app-disabled receipt");
    assert_eq!(disabled_receipt["nonce"], "headless-note-nonce");
    assert_eq!(disabled_receipt["tool"], "save_note");

    handle.shutdown().await;
    drop(state);
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome, Node.js and npm"]
#[allow(
    clippy::too_many_lines,
    reason = "the browser, agent receipt, App acknowledgement and redacted ledger form one acceptance proof"
)]
async fn a_real_browser_receives_the_native_agent_call_input_and_result() {
    let Some(browser) = browser_path() else {
        eprintln!("skipped: no browser on this machine (set SWEM_BROWSER to one)");
        return;
    };
    assert!(node_available(), "put Node.js on PATH");
    let root = std::env::temp_dir().join(format!(
        "swem-observed-app-browser-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let bundle_dir = root.join("bundle");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    build_apps_bundle(&bundle_dir);

    PersonalAgentProfileStore::open(&inventory)
        .expect("open profile inventory")
        .create(
            &PersonalAgentProfile::new(
                "apps-main",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                vec![AttachmentBinding::new(
                    "notes-attachment",
                    "notes",
                    AttachmentTransport::Stdio,
                )],
                Vec::new(),
            )
            .expect("build fixture profile"),
        )
        .expect("persist fixture profile");
    let root_for_resolver = root.clone();
    let state = Arc::new(
        WorkbenchShellState::open(
            &inventory,
            &ledger,
            Duration::from_secs(30),
            move |_profile| {
                let agent = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
                Ok(ResolvedDirectAgentConnection {
                    launch: LaunchCommand {
                        executable: agent.display().to_string(),
                        args: Vec::new(),
                        integration: IntegrationKind::DirectAcp,
                    },
                    agent_executable: agent,
                    mcp_servers: vec![McpServer::Stdio(
                        McpServerStdio::new(
                            "notes",
                            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                        )
                        .args(vec![
                            "--receipt".into(),
                            root_for_resolver
                                .join("agent-receipt.json")
                                .display()
                                .to_string(),
                            "--poison".into(),
                            root_for_resolver.join("poison.json").display().to_string(),
                            "--observed-receipt".into(),
                            root_for_resolver.join("app-ack.json").display().to_string(),
                        ]),
                    )],
                })
            },
        )
        .expect("open shell state"),
    );
    state.set_mcp_observer_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        Vec::new(),
    );
    let handle = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle_dir),
    )
    .await
    .expect("bind observed Apps shell");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_shell_apps_observed_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_mins(3),
        tokio::process::Command::new("node")
            .arg(driver)
            .arg(browser)
            .arg(url)
            .output(),
    )
    .await
    .expect("observed App browser drive timed out")
    .expect("start browser driver");
    assert!(
        output.status.success(),
        "driver failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver report"),
    )
    .expect("parse driver report");
    let route = &route_of(&ledger, &report, "main");
    assert_eq!(report["app_server"], "notes");

    let agent_receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("agent-receipt.json")).expect("read native MCP receipt"),
    )
    .expect("parse native MCP receipt");
    let app_ack: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("app-ack.json")).expect("read App acknowledgement"),
    )
    .expect("parse App acknowledgement");
    assert_eq!(agent_receipt["nonce"], "browser-agent-nonce-0.55");
    assert_eq!(app_ack["input_nonce"], agent_receipt["nonce"]);
    assert_eq!(app_ack["input_text"], agent_receipt["text"]);
    assert_eq!(app_ack["result_nonce"], agent_receipt["nonce"]);
    assert_eq!(app_ack["result_count"], agent_receipt["note_count"]);

    let events = events_of(&ledger, route);
    assert_eq!(
        events
            .iter()
            .filter(|(kind, _)| kind == "host/app_tool_observed")
            .count(),
        2
    );
    assert!(events.iter().any(|(kind, payload)| {
        kind == "host/app_tool_call"
            && payload["decision"] == "allowed"
            && payload["tool"] == "acknowledge_observed_call"
    }));
    let observation_descriptors = events
        .iter()
        .filter(|(kind, _)| kind == "host/app_tool_observed")
        .collect::<Vec<_>>();
    let durable =
        serde_json::to_string(&observation_descriptors).expect("serialize durable descriptors");
    assert!(!durable.contains("browser-agent-nonce-0.55"));
    assert!(!durable.contains("exact native call projected through AppBridge"));
    assert!(!root.join("poison.json").exists());

    handle.shutdown().await;
    drop(state);
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome, Node.js and npm"]
#[allow(
    clippy::too_many_lines,
    reason = "the official AppBridge notifications, App-only receipts, native side effect and ledger ordering form one terminal-state proof"
)]
async fn a_real_browser_preserves_terminal_states_and_cannot_cancel_on_teardown() {
    let Some(browser) = browser_path() else {
        eprintln!("skipped: no browser on this machine (set SWEM_BROWSER to one)");
        return;
    };
    assert!(node_available(), "put Node.js on PATH");
    let root = std::env::temp_dir().join(format!(
        "swem-terminal-app-browser-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let bundle_dir = root.join("bundle");
    let delay_release = root.join("delay-release");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    build_apps_bundle(&bundle_dir);

    PersonalAgentProfileStore::open(&inventory)
        .expect("open profile inventory")
        .create(
            &PersonalAgentProfile::new(
                "apps-main",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                vec![AttachmentBinding::new(
                    "notes-attachment",
                    "notes",
                    AttachmentTransport::Stdio,
                )],
                Vec::new(),
            )
            .expect("build fixture profile"),
        )
        .expect("persist fixture profile");
    let root_for_resolver = root.clone();
    let state = Arc::new(
        WorkbenchShellState::open(
            &inventory,
            &ledger,
            Duration::from_secs(30),
            move |_profile| {
                let agent = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
                Ok(ResolvedDirectAgentConnection {
                    launch: LaunchCommand {
                        executable: agent.display().to_string(),
                        args: Vec::new(),
                        integration: IntegrationKind::DirectAcp,
                    },
                    agent_executable: agent,
                    mcp_servers: vec![McpServer::Stdio(
                        McpServerStdio::new(
                            "notes",
                            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                        )
                        .args(vec![
                            "--receipt".into(),
                            root_for_resolver
                                .join("agent-receipt.json")
                                .display()
                                .to_string(),
                            "--poison".into(),
                            root_for_resolver.join("poison.json").display().to_string(),
                            "--terminal-receipt".into(),
                            root_for_resolver
                                .join("terminal-receipts.jsonl")
                                .display()
                                .to_string(),
                            "--delay-release".into(),
                            root_for_resolver
                                .join("delay-release")
                                .display()
                                .to_string(),
                        ]),
                    )],
                })
            },
        )
        .expect("open shell state"),
    );
    state.set_mcp_observer_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        Vec::new(),
    );
    let handle = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle_dir),
    )
    .await
    .expect("bind terminal Apps shell");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_shell_apps_terminal_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_mins(3),
        tokio::process::Command::new("node")
            .arg(driver)
            .arg(browser)
            .arg(url)
            .arg(&delay_release)
            .output(),
    )
    .await
    .expect("terminal App browser drive timed out")
    .expect("start terminal App browser driver");
    assert!(
        output.status.success(),
        "driver failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver report"),
    )
    .expect("parse driver report");
    let error_route = &route_of(&ledger, &report, "error");
    let cancel_route = &route_of(&ledger, &report, "cancel");
    let delay_route = &route_of(&ledger, &report, "delay");

    let terminal_receipts = fs::read_to_string(root.join("terminal-receipts.jsonl"))
        .expect("read App-only terminal receipts")
        .lines()
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line).expect("parse terminal receipt")
        })
        .collect::<Vec<_>>();
    assert_eq!(terminal_receipts.len(), 2);
    assert_eq!(
        terminal_receipts[0]["input_nonce"],
        "browser-tool-error-0.57"
    );
    assert_eq!(terminal_receipts[0]["terminal"], "tool_error");
    assert_eq!(terminal_receipts[1]["input_nonce"], "browser-cancel-0.57");
    assert_eq!(terminal_receipts[1]["terminal"], "cancelled");
    assert_eq!(
        terminal_receipts[1]["reason"],
        "native agent cancelled fixture call"
    );

    let error_events = events_of(&ledger, error_route);
    let cancel_events = events_of(&ledger, cancel_route);
    let delay_events = events_of(&ledger, delay_route);
    assert!(error_events.iter().any(|(kind, payload)| {
        kind == "host/app_tool_observed" && payload["status"] == "tool_error"
    }));
    assert!(cancel_events.iter().any(|(kind, payload)| {
        kind == "host/app_tool_observed" && payload["status"] == "cancelled"
    }));
    for events in [&error_events, &cancel_events] {
        assert!(events.iter().any(|(kind, payload)| {
            kind == "host/app_tool_call"
                && payload["decision"] == "allowed"
                && payload["tool"] == "acknowledge_terminal"
        }));
    }
    let closed = delay_events
        .iter()
        .position(|(kind, _)| kind == "host/app_closed")
        .expect("pending App was closed");
    let completed = delay_events
        .iter()
        .position(|(kind, payload)| {
            kind == "host/app_tool_observed" && payload["status"] == "completed"
        })
        .expect("native call completed after View teardown");
    assert!(
        closed < completed,
        "the terminal response preceded the asserted teardown race"
    );
    assert!(!delay_events.iter().any(|(kind, payload)| {
        kind == "host/app_tool_observed" && payload["status"] == "cancelled"
    }));
    let delayed_receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("agent-receipt.json")).expect("read delayed native receipt"),
    )
    .expect("parse delayed native receipt");
    assert_eq!(delayed_receipt["tool"], "delayed_note");
    assert_eq!(delayed_receipt["nonce"], "browser-teardown-race-0.57");
    assert!(!root.join("poison.json").exists());

    let observation_descriptors = error_events
        .iter()
        .chain(cancel_events.iter())
        .chain(delay_events.iter())
        .filter(|(kind, _)| kind == "host/app_tool_observed")
        .collect::<Vec<_>>();
    let durable = serde_json::to_string(&observation_descriptors)
        .expect("serialize terminal observation descriptors");
    for raw in [
        "browser-tool-error-0.57",
        "browser-cancel-0.57",
        "browser-teardown-race-0.57",
        "native agent cancelled fixture call",
    ] {
        assert!(
            !durable.contains(raw),
            "raw terminal payload leaked into ledger: {raw}"
        );
    }

    handle.shutdown().await;
    drop(state);
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[ignore = "live agent test: requires SWEM_NATIVE_AGENT and its lawful credential"]
#[allow(
    clippy::too_many_lines,
    reason = "one live scenario keeps the profile, the permission surface and the receipt oracle together"
)]
async fn the_apps_host_discovers_apps_on_a_live_native_profile_and_the_agent_uses_the_same_tool() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=<catalog id> before running this ignored test");

    let root = std::env::temp_dir().join(format!(
        "swem-apps-live-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let bundle_dir = root.join("bundle");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    fs::create_dir_all(&bundle_dir).expect("create Apps bundle marker");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    store
        .create(
            &PersonalAgentProfile::new(
                "native-apps",
                agent_id.clone(),
                "direct-host-distribution",
                "direct-host-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                vec![AttachmentBinding::new(
                    "notes-attachment",
                    "notes",
                    AttachmentTransport::Stdio,
                )],
                live_credential_bindings(&agent_id),
            )
            .expect("build live profile"),
        )
        .expect("persist live profile");
    let root_for_resolver = root.clone();
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_mins(2),
        move |profile| {
            let discovery = swem_host::discover_agents()
                .into_iter()
                .find(|agent| agent.id == profile.agent_id)
                .ok_or_else(|| format!("unknown agent: {}", profile.agent_id))?;
            Ok(ResolvedDirectAgentConnection {
                launch: discovery
                    .launch
                    .clone()
                    .ok_or_else(|| "discovered agent has no launch command".to_owned())?,
                agent_executable: discovery
                    .executable_path
                    .clone()
                    .ok_or_else(|| "discovered agent has no executable path".to_owned())?,
                mcp_servers: vec![McpServer::Stdio(
                    McpServerStdio::new(
                        "notes",
                        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                    )
                    .args(vec![
                        "--receipt".into(),
                        root_for_resolver
                            .join("notes-receipt-live.json")
                            .display()
                            .to_string(),
                        "--poison".into(),
                        root_for_resolver
                            .join("poison-receipt-live.json")
                            .display()
                            .to_string(),
                    ]),
                )],
            })
        },
    )
    .expect("open shell state");
    let state = Arc::new(state);
    state.set_mcp_observer_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        Vec::new(),
    );
    let handle = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle_dir),
    )
    .await
    .expect("enable the live Apps observation surface");

    let (connection, route, _session) = state
        .open_connection("native-apps", ShellConnectionMode::New, None)
        .await
        .expect("open the live connection");

    // Host-side discovery is agent-independent: the SAME generic host dials
    // the attachment with its own client while Claude holds the session.
    let attachments = state.apps_list(&connection).await.expect("discover apps");
    let notes = attachments
        .iter()
        .find(|view| view.server_name == "notes")
        .expect("notes attachment discovered on the live profile");
    assert_eq!(notes.apps[0].uri, NOTES_URI);

    // The surface answers the native agent's permission requests with the exact
    // agent-offered allow option, exactly like a human on the panel.
    let permission_state = Arc::clone(&state);
    let permission_connection = connection.clone();
    let allower = tokio::spawn(async move {
        loop {
            let Ok(Some(request)) = permission_state
                .next_permission(&permission_connection, Duration::from_mins(1))
                .await
            else {
                break;
            };
            let allow = request
                .options
                .iter()
                .find(|option| {
                    option.option_id.0.contains("allow")
                        || option.name.to_lowercase().contains("allow")
                })
                .map(|option| option.option_id.0.to_string());
            let Some(option_id) = allow else { break };
            let _ = permission_state
                .select_permission(&permission_connection, request.sequence, &option_id)
                .await;
        }
    });

    let marker = "SWEM_APPS_LIVE_2026_08_29";
    let turn = state
        .submit_prompt(
            &connection,
            vec![ContentBlock::Text(TextContent::new(format!(
                "Call the save_note tool of the attached notes MCP server exactly once, \
                 with arguments nonce=\"{marker}\" and text=\"saved by the live agent\". \
                 Then reply with exactly DONE."
            )))],
        )
        .await
        .expect("live agent turn");
    assert!(
        turn.reply_text.contains("DONE"),
        "unexpected reply: {}",
        turn.reply_text
    );
    let observed = state
        .next_observed_app(&connection, 0, Duration::from_secs(10))
        .await
        .expect("read live agent observation")
        .expect("live agent call linked to its declared App");
    assert_eq!(observed.observation.arguments["nonce"], marker);
    assert_eq!(
        observed.observation.arguments["text"],
        "saved by the live agent"
    );
    let observed_app_id = observed.opened.app_id.clone();
    let terminal_observed = state
        .observed_app_status(
            &connection,
            &observed.observation.observation_id,
            Duration::from_secs(10),
        )
        .await
        .expect("read terminal live observation")
        .expect("live observation retained until disconnect");
    assert_eq!(terminal_observed.status, "completed");
    assert_eq!(
        terminal_observed
            .result
            .as_ref()
            .expect("original CallToolResult")["structuredContent"]["nonce"],
        marker
    );
    let audit = state
        .events(&route, "agent-app-correlation-audit", 512, Duration::ZERO)
        .await
        .expect("read projected ACP events for correlation audit");
    if agent_id == "opencode" {
        let updates = audit
            .events
            .iter()
            .filter(|event| event.kind == "acp/session_update")
            .filter_map(|event| event.payload.get("update"))
            .collect::<Vec<_>>();
        let opened_call = updates
            .iter()
            .find(|update| {
                update["sessionUpdate"] == "tool_call" && update["title"] == "notes_save_note"
            })
            .expect("OpenCode projected the model-initiated App tool call");
        let tool_call_id = opened_call["toolCallId"]
            .as_str()
            .expect("OpenCode tool call id");
        assert_eq!(
            opened_call["rawInput"],
            serde_json::json!({}),
            "the first ACP report can precede complete arguments"
        );
        let input_update = updates
            .iter()
            .find(|update| {
                update["sessionUpdate"] == "tool_call_update"
                    && update["toolCallId"] == tool_call_id
                    && update["status"] == "in_progress"
            })
            .expect("OpenCode projected complete tool input");
        assert_eq!(input_update["rawInput"]["nonce"], marker);
        assert_eq!(input_update["rawInput"]["text"], "saved by the live agent");
        let completed = updates
            .iter()
            .find(|update| {
                update["sessionUpdate"] == "tool_call_update"
                    && update["toolCallId"] == tool_call_id
                    && update["status"] == "completed"
            })
            .expect("OpenCode projected the completed tool call");
        let adapter_output = completed["rawOutput"]["output"]
            .as_str()
            .expect("OpenCode adapter output wrapper");
        let adapter_output: serde_json::Value =
            serde_json::from_str(adapter_output).expect("parse wrapped MCP result text");
        assert_eq!(adapter_output["saved"], true);
        assert_eq!(adapter_output["nonce"], marker);
        assert!(
            completed["rawOutput"].get("structuredContent").is_none(),
            "ACP report is not the original MCP CallToolResult"
        );
        assert!(
            opened_call.get("_meta").is_none() && completed.get("_meta").is_none(),
            "this OpenCode report unexpectedly gained machine-readable MCP provenance"
        );
    }
    state
        .app_close(&connection, &observed_app_id)
        .await
        .expect("close projected live App");
    state.disconnect(&connection).await.expect("disconnect");
    allower.abort();

    let receipt: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt-live.json")).expect("read live receipt"),
    )
    .expect("parse live receipt");
    assert_eq!(receipt["nonce"], marker);
    assert!(
        !root.join("poison-receipt-live.json").exists(),
        "the model-only probe must not have run"
    );

    let observed_events = events_of(&ledger, &route)
        .into_iter()
        .filter(|(kind, _)| kind == "host/app_tool_observed")
        .collect::<Vec<_>>();
    assert_eq!(observed_events.len(), 2);
    let durable = serde_json::to_string(&observed_events).expect("serialize live descriptors");
    assert!(!durable.contains(marker));

    handle.shutdown().await;
    drop(state);
    fs::remove_dir_all(root).expect("remove fixture root");
}
