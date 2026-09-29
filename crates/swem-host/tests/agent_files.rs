//! An agent's files as the page reads and changes them: the folder it
//! works in and nothing beside it, and a file saved only while it is what
//! was opened.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swem_host::workbench_shell::{ChangeTreeBody, SaveFileBody};
use swem_host::{
    PersonalAgentProfile, PersonalAgentProfileStore, THIS_MACHINE, WorkbenchShellError,
    WorkbenchShellState,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-agent-files-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn an_agent_that_has_worked(root: &std::path::Path) -> WorkbenchShellState {
    let workspace = root.join("workspace");
    let home = root.join("home");
    fs::create_dir_all(workspace.join("src")).expect("create workspace");
    fs::create_dir_all(&home).expect("create agent home");
    fs::write(workspace.join("src").join("main.rs"), "fn main() {}\n").expect("a file");
    fs::write(workspace.join("picture.bin"), [0_u8, 159, 146, 150]).expect("a file");
    fs::write(root.join("beside.txt"), "not the agent's").expect("what is beside");
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
    WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_profile| Err("no engine is started here".to_owned()),
    )
    .expect("open shell state")
}

#[tokio::test]
async fn a_person_reads_the_tree_and_saves_what_they_changed() {
    let root = fixture_root("saved");
    let state = an_agent_that_has_worked(&root);

    let top = state.agent_tree("ada", "").await.expect("the tree");
    let names: Vec<&str> = top.iter().map(|entry| entry.name.as_str()).collect();
    assert_eq!(names, ["src", "picture.bin"]);
    let inside = state.agent_tree("ada", "src").await.expect("a folder");
    assert_eq!(inside[0].name, "main.rs");

    let opened = state
        .agent_file("ada", "src/main.rs")
        .await
        .expect("open the file");
    assert_eq!(opened.text.as_deref(), Some("fn main() {}\n"));
    // What is not text is opened as what it is, not as text to edit.
    let picture = state.agent_file("ada", "picture.bin").await.expect("open");
    assert_eq!(picture.text, None);
    assert_eq!(picture.byte_length, 4);
    let (bytes, served_as) = state
        .agent_file_as_it_is("ada", "picture.bin")
        .await
        .expect("its bytes");
    assert_eq!(bytes, [0_u8, 159, 146, 150]);
    assert_eq!(served_as, "application/octet-stream");

    let saved = state
        .save_agent_file(
            "ada",
            SaveFileBody {
                path: "src/main.rs".into(),
                text: "fn main() { println!(\"hi\"); }\n".into(),
                bytes: None,
                was: Some(opened.sha256.clone()),
                over: false,
            },
        )
        .await
        .expect("save");
    assert!(saved.saved);

    // The agent rewrote it meanwhile: the person is told and nothing is
    // written until they choose.
    fs::write(
        root.join("workspace/src/main.rs"),
        "fn main() { agent(); }\n",
    )
    .expect("the agent writes");
    let mine = SaveFileBody {
        path: "src/main.rs".into(),
        text: "fn main() { mine(); }\n".into(),
        bytes: None,
        was: saved.sha256.clone(),
        over: false,
    };
    let told = state
        .save_agent_file("ada", mine.clone())
        .await
        .expect("an answer, not a failure");
    assert!(!told.saved);
    assert!(
        told.said.contains("changed since it was read"),
        "{}",
        told.said
    );
    assert_eq!(
        fs::read_to_string(root.join("workspace/src/main.rs")).expect("read"),
        "fn main() { agent(); }\n"
    );
    let chosen = state
        .save_agent_file("ada", SaveFileBody { over: true, ..mine })
        .await
        .expect("save over it");
    assert!(chosen.saved);
    assert_eq!(
        fs::read_to_string(root.join("workspace/src/main.rs")).expect("read"),
        "fn main() { mine(); }\n"
    );
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn nothing_beside_the_folder_it_works_in_is_reached() {
    let root = fixture_root("beside");
    let state = an_agent_that_has_worked(&root);
    for climbing in ["../beside.txt", "src/../../beside.txt", "/etc/hosts"] {
        assert!(
            matches!(
                state.agent_file("ada", climbing).await,
                Err(WorkbenchShellError::Invalid(_))
            ),
            "{climbing}"
        );
        assert!(
            matches!(
                state
                    .save_agent_file(
                        "ada",
                        SaveFileBody {
                            path: climbing.into(),
                            text: "taken".into(),
                            bytes: None,
                            was: None,
                            over: true,
                        },
                    )
                    .await,
                Err(WorkbenchShellError::Invalid(_))
            ),
            "{climbing}"
        );
    }
    assert!(matches!(
        state.agent_tree("ada", "..").await,
        Err(WorkbenchShellError::Invalid(_))
    ));
    assert!(matches!(
        state.agent_tree("nobody", "").await,
        Err(WorkbenchShellError::NotFound(_))
    ));
    assert_eq!(
        fs::read_to_string(root.join("beside.txt")).expect("still there"),
        "not the agent's"
    );
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn a_person_makes_renames_and_removes() {
    let root = fixture_root("tree");
    let state = an_agent_that_has_worked(&root);
    state
        .change_agent_tree(
            "ada",
            ChangeTreeBody::MakeFolder {
                path: "docs".into(),
            },
        )
        .await
        .expect("make a folder");
    let made = state
        .save_agent_file(
            "ada",
            SaveFileBody {
                path: "docs/notes.md".into(),
                text: String::new(),
                bytes: None,
                was: None,
                over: false,
            },
        )
        .await
        .expect("make a file");
    assert!(made.saved);
    // A name that is taken is not written over by a new file.
    assert!(matches!(
        state
            .save_agent_file(
                "ada",
                SaveFileBody {
                    path: "docs/notes.md".into(),
                    text: "other".into(),
                    bytes: None,
                    was: None,
                    over: false,
                },
            )
            .await,
        Err(WorkbenchShellError::Conflict(_))
    ));
    // What a person brings from their own machine is written as it is.
    let brought = state
        .save_agent_file(
            "ada",
            SaveFileBody {
                path: "docs/logo.bin".into(),
                text: String::new(),
                bytes: Some("AJ+Slg==".into()),
                was: None,
                over: false,
            },
        )
        .await
        .expect("bring a file");
    assert!(brought.saved);
    assert_eq!(
        fs::read(root.join("workspace/docs/logo.bin")).expect("read"),
        [0_u8, 159, 146, 150]
    );
    state
        .change_agent_tree(
            "ada",
            ChangeTreeBody::Rename {
                from: "docs/notes.md".into(),
                to: "docs/plan.md".into(),
            },
        )
        .await
        .expect("rename");
    assert!(matches!(
        state
            .change_agent_tree(
                "ada",
                ChangeTreeBody::Remove {
                    path: "docs".into(),
                    with_all: false,
                },
            )
            .await,
        Err(WorkbenchShellError::Conflict(_))
    ));
    state
        .change_agent_tree(
            "ada",
            ChangeTreeBody::Remove {
                path: "docs".into(),
                with_all: true,
            },
        )
        .await
        .expect("remove with what it holds");
    assert!(!root.join("workspace/docs").exists());
    fs::remove_dir_all(root).expect("remove fixture root");
}
