//! What an agent has where it lives is what was found when its machine
//! was looked at from inside.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swem_host::{
    IntegrationKind, LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore,
    ResolvedDirectAgentConnection, THIS_MACHINE, WorkbenchShellError, WorkbenchShellState,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-looks-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn an_agent_on(root: &Path, engine: PathBuf) -> WorkbenchShellState {
    let workspace = root.join("workspace");
    let home = root.join("home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&home).expect("create agent home");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                "ada",
                "swem-echo-agent",
                "echo-fixture-distribution",
                THIS_MACHINE,
                swem_host::ASK_EVERY_TIME,
                &workspace,
                &home,
                Vec::new(),
                Vec::new(),
            )
            .expect("profile"),
        )
        .expect("persist profile");
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        move |_profile| {
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: engine.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: engine.clone(),
                mcp_servers: Vec::new(),
            })
        },
    )
    .expect("open shell state");
    state.enable_machine_look(root).expect("enable the look");
    state
}

#[tokio::test]
async fn an_agents_machine_is_looked_at_from_inside_and_the_look_is_kept() {
    let root = fixture_root("inside");
    let state = an_agent_on(&root, PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent")));

    // Nobody looked: nothing is said.
    assert_eq!(state.look_inside_kept("ada").expect("the agent"), None);

    let look = state.look_inside("ada").await.expect("look inside");
    assert!(look.engine.starts, "{}", look.engine.said);
    assert!(look.engine.started_in_ms.is_some());
    assert_eq!(
        look.engine.signed_in,
        Some(true),
        "the engine opened a session: {}",
        look.engine.said
    );
    let machine = look.machine.clone().expect("the machine from inside");
    assert_eq!(machine.system, std::env::consts::OS);
    assert_eq!(machine.workspace_writable, Some(true));
    assert!(machine.programs.iter().any(|program| program.name == "git"));
    // This machine was not looked at for containers: none is promised.
    assert!(!look.containers);
    assert!(!look.containers_said.is_empty());

    // What was found is what is shown until somebody looks again.
    assert_eq!(
        state.look_inside_kept("ada").expect("the agent"),
        Some(look)
    );
    assert!(root.join("hosts/looks/ada.json").is_file());

    assert!(matches!(
        state.look_inside("nobody").await,
        Err(WorkbenchShellError::NotFound(_))
    ));
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn an_engine_that_does_not_start_is_what_was_found() {
    let root = fixture_root("absent");
    let state = an_agent_on(&root, root.join("no-such-engine"));
    let look = state.look_inside("ada").await.expect("look inside");
    assert!(!look.engine.starts);
    assert!(!look.engine.said.is_empty());
    assert_eq!(look.engine.signed_in, None);
    // The machine is looked at all the same.
    assert!(look.machine.is_some());
    fs::remove_dir_all(root).expect("remove fixture root");
}
