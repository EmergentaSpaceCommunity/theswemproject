//! An agent that is removed is nobody to write to, and nothing it said or
//! wrote is destroyed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swem_host::{
    IntegrationKind, LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore,
    ResolvedDirectAgentConnection, Saying, StartChatBody, THIS_MACHINE, WorkbenchShellError,
    WorkbenchShellState,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-removal-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn a_profile(root: &Path, profile_id: &str) {
    let workspace = root.join(profile_id);
    let home = root.join(format!("{profile_id}-home"));
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&home).expect("create agent home");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                profile_id,
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
}

fn a_product(root: &Path) -> Arc<WorkbenchShellState> {
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_profile| {
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
        .enable_removal(&root.join("removed"))
        .expect("keep what is removed");
    Arc::new(state)
}

async fn agents(state: &WorkbenchShellState) -> Vec<serde_json::Value> {
    state.chat_people().await.expect("people")["participants"]
        .as_array()
        .expect("participants")
        .iter()
        .filter(|one| one["kind"] == "agent")
        .cloned()
        .collect()
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: said, scheduled, removed, what stays, the name given again"
)]
async fn an_agent_is_removed_and_what_it_said_stays() {
    let root = fixture_root("removed");
    a_profile(&root, "scribe");
    a_profile(&root, "reviewer");
    let state = a_product(&root);
    let known = agents(&state).await;
    let id_of = |profile: &str| {
        known
            .iter()
            .find(|one| one["profile_id"] == profile)
            .expect("the agent")["participant_id"]
            .as_str()
            .expect("an id")
            .to_owned()
    };
    let scribe = id_of("scribe");

    // It says something in a chat of several, holds a key and has a
    // schedule.
    let chat = state
        .start_chat(StartChatBody {
            title: "Release".into(),
            agents: vec!["scribe".into(), "reviewer".into()],
        })
        .await
        .expect("start a chat");
    state
        .say_in_chat(
            &chat.chat_id,
            Saying {
                text: "@scribe say it".into(),
                ..Saying::default()
            },
        )
        .await
        .expect("say");
    let began = std::time::Instant::now();
    loop {
        let page = state
            .chat_page(&chat.chat_id, None, 100)
            .await
            .expect("the chat");
        if page.messages.len() >= 2 && page.deliveries.is_empty() {
            break;
        }
        assert!(began.elapsed() < Duration::from_secs(30), "never answered");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    state
        .make_schedule(swem_host::workbench_shell::NewScheduleBody {
            agent_id: scribe.clone(),
            chat_id: Some(chat.chat_id.clone()),
            say: "check back".into(),
            when: swem_host::When::Every { minutes: 30 },
        })
        .await
        .expect("make a schedule");
    fs::write(root.join("scribe").join("notes.md"), "what it wrote").expect("its file");

    let removed = state.remove_agent("scribe").await.expect("remove it");
    assert_eq!(removed.schedules, 1);
    assert_eq!(
        removed.workspace,
        fs::canonicalize(root.join("scribe")).expect("its folder")
    );

    // It is nobody to write to.
    let left = agents(&state).await;
    let gone = left
        .iter()
        .find(|one| one["participant_id"] == scribe.as_str())
        .expect("it is still who said what it said");
    assert_eq!(gone["retired"], true);
    assert!(gone["profile_id"].is_null());
    assert_ne!(gone["handle"], "scribe");
    assert!(
        state
            .profiles()
            .expect("profiles")
            .iter()
            .all(|profile| profile.profile_id != "scribe")
    );
    assert!(matches!(
        state.remove_agent("scribe").await,
        Err(WorkbenchShellError::NotFound(_))
    ));
    let (schedules, _) = state.schedules_shown(None).await.expect("schedules");
    assert!(schedules.is_empty());

    // What it said and wrote stays.
    let page = state
        .chat_page(&chat.chat_id, None, 100)
        .await
        .expect("the chat");
    assert!(
        page.messages
            .iter()
            .any(|message| message.sender_id == scribe)
    );
    assert_eq!(
        fs::read_to_string(root.join("scribe").join("notes.md")).expect("its file"),
        "what it wrote"
    );
    // Naming it does not reach it: the agent that is left alone in the
    // chat answers, as the only one there.
    let said = state
        .say_in_chat(
            &chat.chat_id,
            Saying {
                text: "@scribe are you there".into(),
                ..Saying::default()
            },
        )
        .await
        .expect("say");
    assert!(
        said.deliveries
            .iter()
            .all(|delivery| delivery.agent_id != scribe)
    );

    // Its profile is set aside without what it held, and its name can be
    // given again: to a new agent, with no chats.
    let aside: Vec<PathBuf> = fs::read_dir(root.join("removed").join("profiles"))
        .expect("what was removed")
        .flatten()
        .map(|entry| entry.path())
        .collect();
    assert_eq!(aside.len(), 1);
    assert!(aside[0].join("profile.json").is_file());
    assert!(!aside[0].join("secrets.json").exists());
    a_profile(&root, "scribe");
    let again = agents(&state).await;
    let new = again
        .iter()
        .find(|one| one["profile_id"] == "scribe")
        .expect("the new agent");
    assert_ne!(new["participant_id"], scribe.as_str());
    assert_eq!(new["handle"], "scribe");
    assert_eq!(new["retired"], false);

    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}
