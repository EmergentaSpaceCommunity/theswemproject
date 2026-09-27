//! A real browser: what an App of a server's space says a person is looking
//! at (`ui/update-model-context`) is given to the agent with the next turn,
//! shown in the Agent space, and let go of from there. The host draws no
//! Project space of its own. Oracles: the driver's report and the route's
//! ledger - never pixels.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use swem_host::{
    AttachmentBinding, AttachmentTransport, DeclareMcpServerBody, IntegrationKind, LaunchCommand,
    PersonalAgentProfile, PersonalAgentProfileStore, ResolvedDirectAgentConnection, RoutingLedger,
    WorkbenchShellState, serve_workbench_http_with_apps,
};

fn browser_path() -> Option<PathBuf> {
    std::env::var("SWEM_BROWSER")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn notes_args(root: &Path, who: &str) -> Vec<String> {
    vec![
        "--receipt".to_owned(),
        root.join(format!("{who}-receipt.json"))
            .display()
            .to_string(),
        "--poison".to_owned(),
        root.join(format!("{who}-poison.json"))
            .display()
            .to_string(),
        "--home".to_owned(),
    ]
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

/// A shell with one profile of the echo agent that attaches `notes`, and
/// `notes` declared in the catalogue with a home App: the session's server
/// and the space's are the same declaration by name.
fn shell_with_a_profile_and_the_servers_space(root: &Path, ledger: &Path) -> WorkbenchShellState {
    let inventory = root.join("inventory");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&agent_home).unwrap();

    PersonalAgentProfileStore::open(&inventory)
        .expect("open profile inventory")
        .create(
            &PersonalAgentProfile::new(
                "context-main",
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
            .expect("build the profile"),
        )
        .expect("keep the profile");

    let root_for_resolver = root.to_path_buf();
    let state = WorkbenchShellState::open(&inventory, ledger, Duration::from_secs(30), move |_| {
        let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
        Ok(ResolvedDirectAgentConnection {
            launch: LaunchCommand {
                executable: executable.display().to_string(),
                args: Vec::new(),
                integration: IntegrationKind::DirectAcp,
            },
            agent_executable: executable,
            mcp_servers: vec![McpServer::Stdio(
                McpServerStdio::new(
                    "notes",
                    PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                )
                .args(notes_args(&root_for_resolver, "session")),
            )],
        })
    })
    .expect("open shell state");
    state
        .enable_mcp_catalogue(&root.join("mcp-servers"), Arc::default())
        .expect("the catalogue");
    state
        .declare_mcp_server(&DeclareMcpServerBody {
            name: "notes".into(),
            transport: "stdio".into(),
            command: env!("CARGO_BIN_EXE_swem-mcp-apps-fixture").to_owned(),
            args: notes_args(root, "space"),
            env: Vec::new(),
            url: String::new(),
            headers: Vec::new(),
        })
        .expect("declare the fixture");
    state
}

#[tokio::test]
#[ignore = "live browser test: requires Chrome and Node.js"]
async fn a_real_browser_gives_the_agent_what_an_app_says_a_person_is_looking_at() {
    let browser = browser_path().expect("set SWEM_BROWSER before running this ignored test");
    let root = std::env::temp_dir().join(format!(
        "swem-context-browser-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let ledger = root.join("routes.sqlite3");
    let state = shell_with_a_profile_and_the_servers_space(&root, &ledger);
    let state = Arc::new(state);
    let handle =
        serve_workbench_http_with_apps(Arc::clone(&state), ([127, 0, 0, 1], 0).into(), None)
            .await
            .expect("serve");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_shell_context_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(240),
        tokio::process::Command::new("node")
            .arg(driver)
            .arg(&browser)
            .arg(&url)
            .output(),
    )
    .await
    .expect("the browser drive timed out")
    .expect("run the driver");
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "driver failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_str(stdout.lines().last().expect("the driver reported"))
            .expect("parse the driver's report");
    // What the person saw the agent was given: the sentence and the link.
    assert_eq!(report["blocks"][0][0], "text");
    assert_eq!(report["blocks"][1][0], "resource_link");
    assert_eq!(report["blocks"][1][1], "the note board");

    // What the route kept: the context was bound from the server, a turn
    // was written after it, and it was let go of.
    let events = events_of(&ledger, report["route"].as_str().expect("the route"));
    let kinds: Vec<&str> = events.iter().map(|(kind, _)| kind.as_str()).collect();
    let bound = kinds
        .iter()
        .position(|kind| *kind == "host/agent_context_bound")
        .unwrap_or_else(|| panic!("the context was never bound: {kinds:?}"));
    assert_eq!(
        events[bound].1["server_name"], "notes",
        "{:?}",
        events[bound]
    );
    assert_eq!(
        events[bound].1["content"][1]["uri"],
        "ui://apps-fixture/notes"
    );
    let written = kinds
        .iter()
        .rposition(|kind| *kind == "host/turn_written")
        .unwrap_or_else(|| panic!("no turn was written: {kinds:?}"));
    let cleared = kinds
        .iter()
        .position(|kind| *kind == "host/agent_context_cleared")
        .unwrap_or_else(|| panic!("the context was never let go of: {kinds:?}"));
    assert!(bound < written && written < cleared, "{kinds:?}");
    // The turn the agent was handed carried the link the App gave.
    assert!(
        events
            .iter()
            .any(|(kind, payload)| kind != "host/agent_context_bound"
                && payload.to_string().contains("ui://apps-fixture/notes")),
        "no event of the turn carries the link: {kinds:?}"
    );
    handle.shutdown().await;
    std::fs::remove_dir_all(&root).ok();
}
