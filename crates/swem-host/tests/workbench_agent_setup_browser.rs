//! Setting an agent up, in a real browser, and the agent getting it.
//!
//! What a person does here: declare where a model is served from, pick that
//! provider and one of its models for their profile, write the agent's role,
//! add a skill; start a session and see the model on the strip over the
//! conversation; read in the agent's own reply that the model reached it by
//! the variable at launch and by the session's own option; reload the page
//! and come back to the session. Then the product is started again over the
//! same directory and everything set up is still theirs.
//!
//! The echo fixture stands in for Claude Code: the resolver launches it
//! whatever the profile's agent id says, so the profile can say `claude-code`
//! and get Claude Code's layout - `CLAUDE.md`, `.claude/skills` - on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use swem_host::{
    AgentSetup, IntegrationKind, LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore,
    ResolvedDirectAgentConnection, WorkbenchShellState, serve_workbench_http,
};

fn browser_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SWEM_BROWSER") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    [
        "/usr/bin/microsoft-edge",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|candidate| candidate.is_file())
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn fixture_root(label: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("swem-{label}-{}-{stamp}", std::process::id()));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

/// One run of the product over `root`: served, driven by a person, shut down.
async fn walk(root: &Path, browser: &Path, again: bool) -> Value {
    let inventory = root.join("profiles");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    for directory in [&workspace, &agent_home] {
        fs::create_dir_all(directory).expect("create fixture directory");
    }
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    if store.list().expect("list profiles").is_empty() {
        // Exactly what one click on an agent creates: a profile with the
        // agent's own defaults and nothing set up yet.
        let profile = PersonalAgentProfile::new(
            "ada",
            "claude-code",
            "direct-host-distribution",
            "direct-host-environment",
            "surface-permissions",
            &workspace,
            &agent_home,
            Vec::new(),
            Vec::new(),
        )
        .expect("build fixture profile");
        assert_eq!(profile.setup(), AgentSetup::default());
        store.create(&profile).expect("persist fixture profile");
    }
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_secs(30),
        move |_profile| {
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
    .expect("open shell state");
    state
        .enable_model_providers(&root.join("model-providers"))
        .expect("enable the model providers");
    let state = Arc::new(state);
    let handle = serve_workbench_http(Arc::clone(&state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("bind shell server");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());

    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_agent_setup_cdp_driver.mjs");
    let mut command = tokio::process::Command::new("node");
    command.arg(&driver).arg(&url);
    command.env("SWEM_BROWSER", browser);
    if again {
        command.arg("--again");
    }
    let output = tokio::time::timeout(Duration::from_secs(240), command.output())
        .await
        .expect("browser drive timed out")
        .expect("node failed to start");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "driver failed:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{stdout}");
    let report: Value = serde_json::from_str(
        stdout
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
async fn a_person_gives_their_agent_a_model_a_role_and_a_skill_and_the_agent_gets_them() {
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(
        node_available(),
        "put Node.js on PATH before running this ignored test"
    );
    let root = fixture_root("agent-setup");

    let report = walk(&root, &browser, false).await;
    assert_eq!(report["model_variable"], "quality");
    assert_eq!(report["session_model"], "quality");

    // The profile is now the person's: the setup they gave it, on disk.
    let store = PersonalAgentProfileStore::open(&root.join("profiles")).expect("reopen inventory");
    let profile = store.load("ada").expect("load the profile");
    assert!(profile.revision >= 3, "{profile:?}");
    assert_eq!(profile.model_provider.as_deref(), Some("local"));
    assert_eq!(profile.model.as_deref(), Some("quality"));
    assert!(profile.role.contains("You are Ada."), "{profile:?}");
    assert_eq!(profile.agent_skills.len(), 1);
    assert_eq!(profile.agent_skills[0].name, "review");

    // The provider is a document a person can read and correct.
    let declared = root.join("model-providers").join("local.json");
    let provider: Value =
        serde_json::from_slice(&fs::read(&declared).expect("the provider was kept")).expect("json");
    assert_eq!(provider["base_url"], "http://127.0.0.1:8000/v1");
    assert_eq!(provider["key_type"], "generic_env_var");

    // And the agent's working directory carries what Claude Code reads.
    let workspace = root.join("workspace");
    let instructions = fs::read_to_string(workspace.join("CLAUDE.md")).expect("CLAUDE.md written");
    assert!(instructions.contains("You are Ada."), "{instructions}");
    assert!(
        workspace.join(".claude/skills/review/SKILL.md").is_file(),
        "the skill was not written as a folder"
    );

    // The product started again over the same directory: still theirs.
    let again = walk(&root, &browser, true).await;
    assert_eq!(again["again"], true);
    assert_eq!(again["model"], "quality");

    fs::remove_dir_all(&root).ok();
}
