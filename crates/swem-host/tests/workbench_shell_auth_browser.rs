//! Browser gate for signing an agent in: a person, on the page, gets an agent
//! that needs a key from "cannot start" to "answered", without leaving the
//! Agent space and without seeing a payload.
//!
//! The fixture agent advertises an `env_var` method and refuses `session/new`
//! until the variable it named is set. The oracles are the vault file on disk
//! (names only, owner-readable only) and the routing ledger (the authenticate
//! step recorded by method id, the secret's value nowhere), never the pixels.

use std::fs;
use std::path::{Path, PathBuf};
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

fn kinds_of(ledger: &Path, route_id: &str) -> Vec<(String, serde_json::Value)> {
    RoutingLedger::open(ledger)
        .expect("reopen routing ledger")
        .events_for_surface(route_id, "oracle", 5000)
        .expect("read the full ordered lane")
        .events
        .into_iter()
        .map(|event| (event.kind, event.payload))
        .collect()
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome and Node.js"]
#[allow(
    clippy::too_many_lines,
    reason = "one sign-in scenario keeps the profile, the driver run, the vault and the ledger oracles together"
)]
async fn a_person_signs_an_agent_in_from_the_page_and_talks_to_it() {
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(
        node_available(),
        "put Node.js on PATH before running this ignored test"
    );
    let root = std::env::temp_dir().join(format!(
        "swem-auth-browser-{}-{}",
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

    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    let profile = PersonalAgentProfile::new(
        "echo-auth",
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

    let state =
        WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(30), |profile| {
            if profile.agent_id != "swem-echo-agent" {
                return Err(format!("unexpected agent {}", profile.agent_id));
            }
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: vec!["--require-auth".into(), "FIXTURE_KEY".into()],
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

    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workbench_shell_auth_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(180),
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
            .expect("driver reported what it saw"),
    )
    .expect("parse driver report");
    let route = report["route"].as_str().expect("route id");
    // The fixture echoes what it was asked, so the reply carries the prompt.
    assert!(
        report["reply"]
            .as_str()
            .is_some_and(|reply| reply.contains("Привет")),
        "the agent's reply must reach the conversation: {}",
        report["reply"]
    );
    assert!(
        report["access_before"]
            .as_str()
            .is_some_and(|text| text.contains("one way to sign in")),
        "the page must say what the agent will ask for before anything starts: {}",
        report["access_before"]
    );

    // The vault holds the name and kind, never the value; only the owner
    // can read the file.
    let entries = store.secret_entries("echo-auth").expect("read vault");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "FIXTURE_KEY");
    assert_eq!(entries[0].type_id, "generic_env_var");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let vault_file = inventory.join("echo-auth").join("secrets.json");
        let mode = fs::metadata(&vault_file)
            .expect("vault file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "vault mode {mode:o}");
    }
    let profile_file =
        fs::read_to_string(inventory.join("echo-auth").join("profile.json")).expect("profile file");
    assert!(
        !profile_file.contains("fixture-secret-value"),
        "the profile must not carry a secret's value"
    );

    // The ledger records that authentication happened and by which method;
    // the value the person typed is in no event.
    let events = kinds_of(&ledger, route);
    let authenticate = events
        .iter()
        .find(|(kind, _)| kind == "acp/authenticate")
        .expect("the authenticate step is on the lane");
    assert_eq!(authenticate.1["method_id"], "api-key");
    let ledger_bytes = fs::read(&ledger).expect("read ledger bytes");
    assert!(
        !ledger_bytes
            .windows("fixture-secret-value".len())
            .any(|window| window == b"fixture-secret-value"),
        "the secret's value reached the ledger"
    );
    assert!(events.iter().any(|(kind, _)| kind == "acp/prompt_response"));

    drop(handle);
    fs::remove_dir_all(root).expect("remove fixture root");
}
