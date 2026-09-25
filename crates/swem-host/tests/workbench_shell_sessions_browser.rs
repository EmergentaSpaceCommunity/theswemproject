//! A person's sessions, in a real browser: start one, start another, come
//! back to the first, and find them all again after the product restarts.
//!
//! The second session is why this gate exists. A route is bound to the exact
//! native session it was opened with, and the page used to hand the host the
//! route the previous session had bound - so the second time a person ever
//! pressed Start, the host refused with "route binding drift" and the agent
//! was unusable from then on. Nothing in the suite noticed, because every
//! browser gate opened exactly one session.
//!
//! The oracles are the routing ledger (two distinct routes for one profile,
//! each with its own lane) and what the page shows when a session is picked up
//! again (its own words, not the other session's).

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swem_host::{
    IntegrationKind, LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore,
    ResolvedDirectAgentConnection, RoutingLedger, WorkbenchShellState, serve_workbench_http,
};

fn browser_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SWEM_BROWSER") {
        return Some(PathBuf::from(path));
    }
    [
        "/usr/bin/microsoft-edge",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|candidate| candidate.exists())
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// One run of the product over `root`: the shell, served, driven, shut down.
async fn walk(
    root: &std::path::Path,
    browser: &std::path::Path,
    come_back_to: Option<&str>,
) -> serde_json::Value {
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    for directory in [&workspace, &agent_home] {
        fs::create_dir_all(directory).expect("create fixture directory");
    }
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    if store.list().expect("list profiles").is_empty() {
        let profile = PersonalAgentProfile::new(
            "echo-sessions",
            "swem-echo-agent",
            "echo-fixture-distribution",
            "direct-fixture-environment",
            "surface-permissions",
            &workspace,
            &agent_home,
            Vec::new(),
            Vec::new(),
        )
        .expect("build fixture profile");
        store.create(&profile).expect("persist fixture profile");
    }
    let state =
        WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(30), |profile| {
            if profile.agent_id != "swem-echo-agent" {
                return Err(format!("unexpected agent {}", profile.agent_id));
            }
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
        })
        .expect("open shell state");
    let handle = serve_workbench_http(Arc::new(state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("bind shell server");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());

    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_shell_sessions_cdp_driver.mjs");
    let mut command = tokio::process::Command::new("node");
    command.arg(&driver).arg(browser).arg(&url);
    if let Some(route) = come_back_to {
        command.arg("--reopen").arg(route);
    }
    let output = tokio::time::timeout(Duration::from_secs(180), command.output())
        .await
        .expect("browser drive timed out")
        .expect("node failed to start");
    assert!(
        output.status.success(),
        "driver failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("the driver reported what it saw"),
    )
    .expect("parse the driver report");
    drop(handle);
    report
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome and Node.js"]
async fn a_person_keeps_more_than_one_session_and_comes_back_to_them() {
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(
        node_available(),
        "put Node.js on PATH before running this ignored test"
    );
    let root = std::env::temp_dir().join(format!(
        "swem-sessions-browser-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));

    let report = walk(&root, &browser, None).await;
    let first = report["first"].as_str().expect("the first route");
    let second = report["second"].as_str().expect("the second route");
    assert_ne!(first, second, "the second session reused the first's route");

    // The ledger's own answer: one profile, two sessions, each with its own
    // lane, and the words are where they were said.
    let ledger = RoutingLedger::open(&root.join("routes.sqlite3")).expect("reopen ledger");
    let sessions = ledger
        .sessions_of_profile("echo-sessions")
        .expect("read the profile's sessions");
    let routes: Vec<&str> = sessions
        .iter()
        .map(|session| session.route_id.as_str())
        .collect();
    assert_eq!(routes.len(), 2, "{sessions:?}");
    assert!(routes.contains(&first) && routes.contains(&second));
    for session in &sessions {
        assert!(
            session.events > 0,
            "a session with an empty lane: {session:?}"
        );
    }
    let said_in = |route: &str| -> String {
        ledger
            .history(route, 500)
            .expect("read the lane")
            .into_iter()
            .map(|event| event.payload.to_string())
            .collect::<String>()
    };
    assert!(said_in(first).contains("the first thing"));
    assert!(!said_in(first).contains("the second thing"));
    assert!(said_in(second).contains("the second thing"));

    // What the page recovered when the first session was picked up again is
    // that session's own conversation.
    let recovered = report["recovered"].as_array().expect("the recovered words");
    let recovered: Vec<&str> = recovered
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert!(
        recovered
            .iter()
            .any(|text| text.contains("the first thing")),
        "{recovered:?}"
    );

    // A second run of the product over the same directory finds both.
    let again = walk(&root, &browser, Some(first)).await;
    assert_eq!(again["reopened"], true);
    let listed = again["sessions"].as_array().expect("the listed sessions");
    assert_eq!(listed.len(), 2, "{listed:?}");

    fs::remove_dir_all(&root).ok();
}
