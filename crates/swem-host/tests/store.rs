//! The store over the shell's own state: a catalog added by URL, the one
//! list of what the indexes offer, and an install of each kind landing where
//! its kind is used from.
//!
//! The registry is a file the test writes and names, so the listing is
//! hermetic; the catalog is another, with a server that is this workspace's
//! own MCP fixture packaged as a native archive and a skill that is a folder
//! with a `SKILL.md` in a tar.gz.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sha2::Digest as _;
use swem_host::{
    AddIndexBody, InstallKind, McpServerOrigin, StoreInstallBody, StorePlanBody,
    WorkbenchShellState, set_acp_registry_index,
};

fn fresh_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-store-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn sha256_of(path: &Path) -> String {
    format!("{:x}", sha2::Sha256::digest(std::fs::read(path).unwrap()))
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

/// A tar.gz of `directory`'s entry `name`, made the way a release is.
fn tar_gz(into: &Path, directory: &Path, name: &str) -> PathBuf {
    let status = Command::new("tar")
        .arg("-czf")
        .arg(into)
        .arg("-C")
        .arg(directory)
        .arg(name)
        .status()
        .expect("run tar");
    assert!(status.success(), "tar failed");
    into.to_path_buf()
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one state: the registry, a catalog, two installs, the listing after"
)]
fn a_catalog_is_added_its_entries_listed_and_a_server_and_a_skill_install_where_they_are_used() {
    let Some(platform) = swem_host::registry_platform() else {
        eprintln!("skipped: no registry platform for this machine");
        return;
    };
    let root = fresh_root("catalog");
    let supply = root.join("supply");
    std::fs::create_dir_all(&supply).unwrap();

    // The registry: one agent, distributed nowhere this machine can run.
    let registry = supply.join("registry.json");
    std::fs::write(
        &registry,
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": "1.0.0",
            "agents": [{
                "id": "far-agent", "name": "Far Agent", "version": "3.0.0",
                "description": "an agent for another platform",
                "distribution": {"binary": {"nowhere-x86_64": {"archive": "https://x/a.tar.gz", "sha256": "00", "cmd": "a"}}}
            }],
            "extensions": []
        }))
        .unwrap(),
    )
    .unwrap();
    set_acp_registry_index(&file_url(&registry)).expect("the registry is named once");

    // The catalog: a server that is the MCP fixture, and a skill.
    let echo = PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo"));
    let echo_archive = tar_gz(
        &supply.join("echo.tar.gz"),
        echo.parent().unwrap(),
        echo.file_name().unwrap().to_str().unwrap(),
    );
    let skill_dir = supply.join("shout");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: Shout\ndescription: when asked to be loud\n---\n\nAnswer in capitals.\n",
    )
    .unwrap();
    let skill_archive = tar_gz(&supply.join("shout.tar.gz"), &supply, "shout");
    let catalog = supply.join("catalog.json");
    std::fs::write(
        &catalog,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema": "swem:catalog@0.1",
            "name": "This Test's Catalog",
            "entries": [
                {"kind": "server", "id": "echo", "name": "Echo", "version": "0.1.0",
                 "description": "echoes a nonce", "env": ["ECHO_TOKEN"],
                 "distribution": {"binary": {platform: {
                     "archive": file_url(&echo_archive), "sha256": sha256_of(&echo_archive),
                     "cmd": echo.file_name().unwrap().to_str().unwrap()}}}},
                {"kind": "skill", "id": "shout", "name": "Shout", "version": "1.0.0",
                 "distribution": {"archive": {"url": file_url(&skill_archive), "sha256": sha256_of(&skill_archive)}}}
            ]
        }))
        .unwrap(),
    )
    .unwrap();

    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_| panic!("no agent connection is resolved here"),
    )
    .expect("open shell state");
    let installed = root.join("installed");
    state.enable_installs(installed.clone()).unwrap();
    let shipped = swem_host::Catalog::parse(
        br#"{"schema":"swem:catalog@0.1","name":"This Product","entries":[
            {"kind":"server","id":"own-cycle","name":"Own Cycle","version":"9","bundled":true}]}"#,
    )
    .unwrap();
    state
        .enable_store(&root.join("indexes"), vec![shipped])
        .unwrap();
    let declared = Arc::new(Mutex::new(BTreeMap::new()));
    state
        .enable_mcp_catalogue(&root.join("mcp-servers"), Arc::clone(&declared))
        .unwrap();

    // Before any catalog is added: the registry, read live from the file,
    // and the catalog the product ships, whose entry is neither installable
    // nor installed - it is what the product already is.
    let view = state.store().unwrap();
    assert_eq!(view.registry.read, "live");
    assert_eq!(view.registry.agents, 1);
    assert_eq!(view.indexes.len(), 1);
    assert!(view.indexes[0].builtin);
    assert_eq!(view.indexes[0].slug, "this-product");
    let own = view
        .entries
        .iter()
        .find(|entry| entry.id == "own-cycle")
        .expect("the shipped entry is listed");
    assert!(own.bundled && !own.installable && own.reason.is_none());
    let refused = state
        .store_plan(&StorePlanBody {
            kind: InstallKind::Server,
            id: "own-cycle".into(),
        })
        .unwrap_err();
    assert!(
        refused.to_string().contains("nothing to fetch"),
        "{refused}"
    );
    let kept = state.forget_index("this-product").unwrap_err();
    assert!(kept.to_string().contains("not forgotten"), "{kept}");
    let far = view
        .entries
        .iter()
        .find(|entry| entry.id == "far-agent")
        .unwrap();
    assert_eq!(far.kind, InstallKind::Agent);
    assert!(!far.installable);
    assert!(far.reason.as_deref().unwrap().contains("this machine"));

    // The catalog is added by URL and kept with where it came from.
    let index = state
        .add_index(&AddIndexBody {
            url: file_url(&catalog),
        })
        .unwrap();
    assert_eq!(index.slug, "this-test-s-catalog");
    assert_eq!(index.entries, 2);
    assert!(
        root.join("indexes")
            .join("this-test-s-catalog.json")
            .is_file()
    );
    let refused = state
        .add_index(&AddIndexBody {
            url: file_url(&registry),
        })
        .unwrap_err();
    assert!(refused.to_string().contains("not a catalog"), "{refused}");

    let view = state.store().unwrap();
    let echo_entry = view
        .entries
        .iter()
        .find(|entry| entry.id == "echo")
        .unwrap();
    assert_eq!(echo_entry.kind, InstallKind::Server);
    assert!(echo_entry.installable);
    assert_eq!(echo_entry.needs, ["ECHO_TOKEN"]);
    assert_eq!(echo_entry.installed, None);
    assert_eq!(echo_entry.index, "This Test's Catalog");
    assert!(
        view.entries
            .iter()
            .any(|entry| entry.id == "shout" && entry.kind == InstallKind::Skill)
    );

    // The server: planned, installed against the plan, declared for a profile.
    let plan = state
        .store_plan(&StorePlanBody {
            kind: InstallKind::Server,
            id: "echo".into(),
        })
        .unwrap();
    assert_eq!(plan.kind, InstallKind::Server);
    let moved = state
        .store_install(&StoreInstallBody {
            kind: InstallKind::Server,
            id: "echo".into(),
            plan_id: "sha256:not-this-one".into(),
        })
        .unwrap_err();
    assert!(moved.to_string().contains("read it again"), "{moved}");
    let receipt = state
        .store_install(&StoreInstallBody {
            kind: InstallKind::Server,
            id: "echo".into(),
            plan_id: plan.plan_id.clone(),
        })
        .unwrap();
    assert_eq!(receipt.kind, InstallKind::Server);
    let executable = receipt.executable.clone().unwrap();
    assert!(executable.starts_with(installed.join("servers").join("echo")));
    assert!(executable.is_file());
    let servers = state.mcp_servers().unwrap();
    let declared_echo = servers
        .iter()
        .find(|server| server.name == "echo")
        .expect("the installed server is declared");
    assert_eq!(declared_echo.origin, McpServerOrigin::Catalogue);
    assert_eq!(declared_echo.transport, "stdio");
    assert_eq!(
        declared_echo.command.as_deref(),
        Some(executable.display().to_string().as_str())
    );

    // The skill: installed as a tree, read by its front matter.
    let plan = state
        .store_plan(&StorePlanBody {
            kind: InstallKind::Skill,
            id: "shout".into(),
        })
        .unwrap();
    let receipt = state
        .store_install(&StoreInstallBody {
            kind: InstallKind::Skill,
            id: "shout".into(),
            plan_id: plan.plan_id,
        })
        .unwrap();
    let file = receipt
        .file
        .clone()
        .expect("a skill's receipt names its file");
    assert!(
        file.ends_with("SKILL.md") && file.is_file(),
        "{}",
        file.display()
    );
    let skills = state.installed_skills().unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "shout");
    assert_eq!(skills[0].name, "Shout");
    assert_eq!(skills[0].description, "when asked to be loud");
    assert_eq!(skills[0].body, "Answer in capitals.");

    // The listing now says both are installed, and every receipt is one list.
    let view = state.store().unwrap();
    assert_eq!(
        view.entries
            .iter()
            .find(|entry| entry.id == "echo")
            .unwrap()
            .installed
            .as_deref(),
        Some("0.1.0")
    );
    assert_eq!(
        view.entries
            .iter()
            .find(|entry| entry.id == "shout")
            .unwrap()
            .installed
            .as_deref(),
        Some("1.0.0")
    );
    let receipts = state.installs().unwrap();
    assert_eq!(
        receipts
            .iter()
            .map(|r| (r.kind, r.registry_id.as_str()))
            .collect::<Vec<_>>(),
        [(InstallKind::Server, "echo"), (InstallKind::Skill, "shout")]
    );

    // Forgetting the catalog leaves what was installed from it installed.
    state.forget_index("this-test-s-catalog").unwrap();
    let view = state.store().unwrap();
    assert_eq!(view.indexes.len(), 1);
    assert!(!view.entries.iter().any(|entry| entry.id == "echo"));
    assert_eq!(state.installs().unwrap().len(), 2);
}
