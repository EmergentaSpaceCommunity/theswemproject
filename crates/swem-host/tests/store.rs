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
    AddIndexBody, Kind, McpServerOrigin, StoreInstallBody, StorePlanBody, WorkbenchShellState,
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
    state
        .set_acp_registry_index(&file_url(&registry))
        .expect("the registry is named once");
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
            kind: Kind::SERVER,
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
    assert_eq!(far.kind, Kind::AGENT);
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
    assert_eq!(echo_entry.kind, Kind::SERVER);
    assert!(echo_entry.installable);
    assert_eq!(echo_entry.needs, ["ECHO_TOKEN"]);
    assert_eq!(echo_entry.installed, None);
    assert_eq!(echo_entry.index, "This Test's Catalog");
    assert!(
        view.entries
            .iter()
            .any(|entry| entry.id == "shout" && entry.kind == Kind::SKILL)
    );

    // The server: planned, installed against the plan, declared for a profile.
    let plan = state
        .store_plan(&StorePlanBody {
            kind: Kind::SERVER,
            id: "echo".into(),
        })
        .unwrap();
    assert_eq!(plan.plan.kind, Kind::SERVER);
    let moved = state
        .store_install(&StoreInstallBody {
            kind: Kind::SERVER,
            id: "echo".into(),
            plan_id: "sha256:not-this-one".into(),
        })
        .unwrap_err();
    assert!(moved.to_string().contains("read it again"), "{moved}");
    let receipt = state
        .store_install(&StoreInstallBody {
            kind: Kind::SERVER,
            id: "echo".into(),
            plan_id: plan.plan.plan_id.clone(),
        })
        .unwrap();
    let receipt = receipt
        .last()
        .cloned()
        .expect("the receipt of the entry itself");
    assert_eq!(receipt.kind, Kind::SERVER);
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

    // A value the person gave the declared server survives an install of
    // it again: an update is not a reason to type a key again.
    state
        .declare_mcp_server(&swem_host::DeclareMcpServerBody {
            name: "echo".into(),
            transport: "stdio".into(),
            command: executable.display().to_string(),
            args: Vec::new(),
            env: vec![swem_host::NamedValue {
                name: "ECHO_KEY".into(),
                value: "given-once".into(),
            }],
            url: String::new(),
            headers: Vec::new(),
        })
        .unwrap();
    state
        .store_install(&StoreInstallBody {
            kind: Kind::SERVER,
            id: "echo".into(),
            plan_id: plan.plan.plan_id.clone(),
        })
        .unwrap();
    let echo_again = state
        .mcp_servers()
        .unwrap()
        .into_iter()
        .find(|server| server.name == "echo")
        .unwrap();
    assert_eq!(echo_again.env_names, vec!["ECHO_KEY".to_owned()]);

    // The skill: installed as a tree, read by its front matter.
    let plan = state
        .store_plan(&StorePlanBody {
            kind: Kind::SKILL,
            id: "shout".into(),
        })
        .unwrap();
    let receipt = state
        .store_install(&StoreInstallBody {
            kind: Kind::SKILL,
            id: "shout".into(),
            plan_id: plan.plan.plan_id,
        })
        .unwrap();
    let receipt = receipt
        .last()
        .cloned()
        .expect("the receipt of the entry itself");
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
            .map(|r| (r.kind.clone(), r.registry_id.as_str()))
            .collect::<Vec<_>>(),
        [(Kind::SERVER, "echo"), (Kind::SKILL, "shout")]
    );

    // Forgetting the catalog leaves what was installed from it installed.
    state.forget_index("this-test-s-catalog").unwrap();
    let view = state.store().unwrap();
    assert_eq!(view.indexes.len(), 1);
    assert!(!view.entries.iter().any(|entry| entry.id == "echo"));
    assert_eq!(state.installs().unwrap().len(), 2);
}

/// A declared server that says it takes a kind of package takes it: the
/// Store fetches, checks and stages, the server plans, installs and
/// removes, through the host, as the Cycle's hub does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one product: a catalog, a plan, an install, a removal"
)]
async fn a_declared_server_takes_packages_of_its_kind_through_the_store() {
    use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
    use swem_host::product::{DataRoot, Product};

    let root = fresh_root("takes");
    let home = root.join("taker-home");
    let package = root.join("supply").join("hello");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(
        package.join("plugin.json"),
        r#"{"name":"hello","version":"0.1.0"}"#,
    )
    .unwrap();
    let archive = tar_gz(&root.join("hello.tar.gz"), &root.join("supply"), "hello");
    let catalog = swem_host::Catalog::parse(
        format!(
            r#"{{"schema":"swem:catalog@0.2","name":"With a taker","entries":[
                {{"kind":"server","id":"taker","name":"The taker","version":"1","bundled":true,
                  "takes":[{{"kind":"example/package@1","plan":"plan_package","install":"install_package",
                             "remove":"remove_package","one":"an example package","many":"Example packages",
                             "after_install":"The taker has it."}}]}},
                {{"kind":"example/package@1","id":"hello","name":"Hello","version":"0.1.0",
                  "distribution":{{"archive":{{"url":"{}","sha256":"{}"}}}},
                  "requires":[{{"kind":"server","id":"taker"}}]}}
            ]}}"#,
            file_url(&archive),
            sha256_of(&archive)
        )
        .as_bytes(),
    )
    .unwrap();
    let product = Product::at(DataRoot::at(root.join("data")))
        .shipped_catalog(catalog)
        .declare(McpServer::Stdio(
            McpServerStdio::new("taker", env!("CARGO_BIN_EXE_swem-mcp-taker-fixture"))
                .args(vec!["--home".to_owned(), home.display().to_string()]),
        ))
        .assemble()
        .expect("the product assembles");
    let state = product.state;

    let kind = swem_host::Kind::parse("example/package@1").unwrap();
    let view = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        move || state.store()
    })
    .await
    .unwrap()
    .unwrap();
    assert!(
        view.kinds
            .iter()
            .any(|shown| shown.kind == kind && shown.many == "Example packages"),
        "{:?}",
        view.kinds
    );
    let hello = view
        .entries
        .iter()
        .find(|entry| entry.id == "hello")
        .unwrap();
    assert!(hello.installable && hello.taken, "{hello:?}");

    let planned = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let kind = kind.clone();
        move || {
            state.store_plan(&StorePlanBody {
                kind,
                id: "hello".into(),
            })
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(planned.also.is_empty(), "the taker is bundled and declared");
    let receipts = tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let kind = kind.clone();
        let plan_id = planned.plan.plan_id.clone();
        move || {
            state.store_install(&StoreInstallBody {
                kind,
                id: "hello".into(),
                plan_id,
            })
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(
        home.join("hello").join("plugin.json").is_file(),
        "the taker did not get the package"
    );

    tokio::task::spawn_blocking({
        let state = Arc::clone(&state);
        let kind = kind.clone();
        move || {
            state.store_remove(&swem_host::StoreRemoveBody {
                kind,
                id: "hello".into(),
            })
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(!home.join("hello").exists(), "the taker kept the package");
    let _ = state.shutdown_server_apps().await;
}
