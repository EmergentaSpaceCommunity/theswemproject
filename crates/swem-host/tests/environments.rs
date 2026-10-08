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
    let environments = state.environments().expect("environments");
    assert!(environments.iter().all(|one| one.available));
    assert_eq!(
        environments
            .iter()
            .map(|one| (
                one.option.environment_profile_id.as_str(),
                one.used_by.len()
            ))
            .collect::<Vec<_>>(),
        [(THIS_MACHINE, 1), (IN_A_CONTAINER, 0)]
    );
    assert_eq!(environments[1].machine, "");
    state.enable_machine_look(&root).expect("enable the look");

    let look = state.look_at_the_machine().await.expect("look");
    assert_eq!(state.machine(), Some(look.clone()));
    let offered = state.environments().expect("environments");
    let container = offered
        .iter()
        .find(|one| one.option.environment_profile_id == IN_A_CONTAINER)
        .expect("a container is listed");
    assert_eq!(container.available, look.a_container_can_start());
    assert_eq!(container.why_not.is_empty(), look.a_container_can_start());
    assert!(offered[0].available && offered[0].machine.contains(&look.system));

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

#[cfg(unix)]
fn a_plan_that_runs(steps: &[(&str, &str)]) -> swem_host::ProvisioningPlan {
    swem_host::ProvisioningPlan {
        plan_id: "sha256:the-plan-offered".into(),
        backend_id: "podman".into(),
        backend_executable: PathBuf::from("/bin/sh"),
        backend_command_prefix: Vec::new(),
        provider: "qemu".into(),
        source: "test".into(),
        disposition: swem_host::ProvisioningDisposition::Confirm,
        steps: steps
            .iter()
            .map(|(about, script)| swem_host::ProvisioningStep::Run {
                executable: PathBuf::from("/bin/sh"),
                args: vec![
                    "-c".into(),
                    (*script).into(),
                    "machine".into(),
                    (*about).into(),
                ],
                description: (*about).into(),
            })
            .chain([swem_host::ProvisioningStep::VerifyBackend])
            .collect(),
        effects: swem_host::ProvisioningEffects {
            creates_virtual_machine: true,
            may_download_machine_image: true,
            requires_privilege_elevation: false,
            may_require_restart: false,
            cpu_count: 2,
            memory_mib: 2048,
            disk_gib: 20,
            rootful: false,
            user_mode_networking: false,
            changes_shared_wsl_networking: false,
            uses_provider_default_host_mounts: true,
        },
        blockers: Vec::new(),
        idempotency_key: "test".into(),
    }
}

#[cfg(unix)]
async fn how_it_ended(state: &WorkbenchShellState) -> swem_host::workbench_shell::SetUp {
    for _ in 0..600 {
        let run = state.setting_up().expect("a set-up is kept");
        if run.state != "running" {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the set-up never ended");
}

/// Setting containers up is agreed to as it was offered, done step by step
/// where a person can see it, and a failure is said in the words of what
/// failed.
#[cfg(unix)]
#[tokio::test]
async fn containers_are_set_up_by_the_plan_a_person_agreed_to() {
    let root = fixture_root("set-up");
    let state = std::sync::Arc::new(
        WorkbenchShellState::open(
            &root.join("inventory"),
            &root.join("routes.sqlite3"),
            Duration::from_secs(20),
            |_profile| Err("no engine is started here".to_owned()),
        )
        .expect("open shell state"),
    );
    state.enable_machine_look(&root).expect("enable the look");

    // Nothing was offered: there is nothing to agree to.
    assert!(matches!(
        state.set_containers_up("sha256:the-plan-offered"),
        Err(WorkbenchShellError::Conflict(_))
    ));

    let offered = state
        .offer_containers(a_plan_that_runs(&[
            ("init", "echo made"),
            (
                "start",
                "echo 'qemu-system-x86_64: Library not loaded' >&2; exit 1",
            ),
        ]))
        .expect("offer the plan");
    assert_eq!(offered.state, "offered");
    assert_eq!(offered.steps.len(), 2, "the look afterwards is not a step");
    assert!(offered.steps.iter().all(|step| step.state == "waiting"));
    assert!(offered.steps[0].what.starts_with("Make a Linux machine"));
    assert_eq!(offered.steps[1].what, "Start the machine");
    assert!(
        offered
            .effects
            .iter()
            .any(|effect| effect.contains("2 processors")),
        "{:?}",
        offered.effects
    );

    // Another plan than the one offered is not agreed to.
    assert!(matches!(
        state.set_containers_up("sha256:another"),
        Err(WorkbenchShellError::Conflict(_))
    ));
    assert_eq!(state.setting_up().expect("kept").state, "offered");

    let begun = state
        .set_containers_up(&offered.plan_id)
        .expect("agree to the plan");
    assert_eq!(begun.state, "running");
    // While it runs, neither another plan nor a second agreement is taken.
    assert!(matches!(
        state.set_containers_up(&offered.plan_id),
        Err(WorkbenchShellError::Conflict(_))
    ));

    let ended = how_it_ended(&state).await;
    assert_eq!(ended.state, "failed");
    assert_eq!(ended.steps[0].state, "done");
    assert_eq!(ended.steps[0].said, "made");
    assert_eq!(ended.steps[1].state, "failed");
    assert!(ended.said.contains("Library not loaded"), "{}", ended.said);
    assert!(ended.hint.is_some(), "what to do about it is said");
    assert!(
        state.machine().is_some(),
        "the machine was looked at again afterwards"
    );

    // A plan that cannot be done as asked is refused in its own words.
    let mut blocked = a_plan_that_runs(&[("init", "echo made")]);
    blocked.blockers = vec!["requested 900 GiB machine disk exceeds 80 GiB free".into()];
    let offered = state.offer_containers(blocked).expect("offer it");
    assert!(matches!(
        state.set_containers_up(&offered.plan_id),
        Err(WorkbenchShellError::Conflict(why)) if why.contains("900 GiB")
    ));
}
