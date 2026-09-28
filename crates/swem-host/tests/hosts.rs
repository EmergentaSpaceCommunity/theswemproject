//! What this machine offers is what was found when it was looked at.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swem_host::{
    AmendProfileBody, IN_A_CONTAINER, PersonalAgentProfile, PersonalAgentProfileStore,
    THIS_MACHINE, WorkbenchShellError, WorkbenchShellState,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-hosts-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

#[tokio::test]
async fn where_an_agent_may_live_is_offered_from_what_was_found() {
    let root = fixture_root("offered");
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
        |_profile| Err("no engine is started here".to_owned()),
    )
    .expect("open shell state");

    // Nobody looked: nothing is promised and nothing is refused.
    assert_eq!(state.machine(), None);
    assert!(state.environments_offered().iter().all(|one| one.available));
    state.enable_machine_look(&root).expect("enable the look");
    let hosts = state.hosts().expect("hosts");
    assert_eq!(
        hosts
            .iter()
            .map(|host| (host.id.as_str(), host.used_by.len()))
            .collect::<Vec<_>>(),
        [(THIS_MACHINE, 1), (IN_A_CONTAINER, 0)]
    );
    assert_eq!(hosts[1].said, "Not looked at yet");

    let look = state.look_at_the_machine().await.expect("look");
    assert_eq!(state.machine(), Some(look.clone()));
    let offered = state.environments_offered();
    let container = offered
        .iter()
        .find(|one| one.option.environment_profile_id == IN_A_CONTAINER)
        .expect("a container is listed");
    assert_eq!(container.available, look.a_container_can_start());
    assert_eq!(container.why_not.is_empty(), look.a_container_can_start());
    let hosts = state.hosts().expect("hosts");
    assert!(hosts[0].ready && hosts[0].machine.contains(&look.system));
    assert_eq!(hosts[1].ready, look.a_container_can_start());

    // Moving an agent where it cannot live is refused in the look's words;
    // saying again where it already lives asks for nothing.
    let moved = state.amend_profile(
        "ada",
        &AmendProfileBody {
            revision: 0,
            environment: Some(IN_A_CONTAINER.into()),
            ..Default::default()
        },
    );
    if look.a_container_can_start() {
        moved.expect("a container can be started here");
    } else {
        let refused = moved.expect_err("no container can be started here");
        assert!(
            matches!(&refused, WorkbenchShellError::Invalid(said) if said.contains(&container.why_not)),
            "{refused}"
        );
        state
            .amend_profile(
                "ada",
                &AmendProfileBody {
                    revision: 0,
                    environment: Some(THIS_MACHINE.into()),
                    ..Default::default()
                },
            )
            .expect("it stays where it lives");
    }

    // The next start says what was found before it looks again.
    let again = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_profile| Err("no engine is started here".to_owned()),
    )
    .expect("open shell state again");
    again.enable_machine_look(&root).expect("enable the look");
    assert_eq!(again.machine(), Some(look));
    fs::remove_dir_all(&root).ok();
}
