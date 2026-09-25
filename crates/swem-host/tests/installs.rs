//! One install root, one receipt schema, and the agents an earlier product
//! left outside it.
//!
//! Everything a person installs from the product lands under
//! `<data root>/installed/<kind>/<id>/<version>/` with one receipt. Three
//! claims here: an agent installed under that root is discovered and
//! launchable even when this build's catalogue never heard of it; the receipt
//! an earlier product wrote into its own `swem/agents` directory is brought
//! under the root with its paths rewritten, so nothing a person installed is
//! lost; and the receipts of every kind read as one list.

use std::path::{Path, PathBuf};

use swem_host::{
    INSTALL_RECEIPT_SCHEMA, INSTALLATION_MANIFEST, InstallKind, InstallReceipt, Readiness,
    adopt_legacy_agents, all_receipts, discover_agents_with, load_receipts,
};

fn fresh_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-installs-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A receipt whose launch path is a real file, as an install leaves one.
fn write_receipt(directory: &Path, receipt: &InstallReceipt) {
    std::fs::create_dir_all(directory).unwrap();
    std::fs::write(
        directory.join(INSTALLATION_MANIFEST),
        serde_json::to_vec_pretty(receipt).unwrap(),
    )
    .unwrap();
}

#[test]
fn an_agent_installed_from_the_registry_is_discovered_without_a_catalogue_entry() {
    let root = fresh_root("unknown-agent");
    let installed = root.join("installed");
    let home = installed.join("agents").join("kilo-gate").join("2.0.0");
    let executable = home.join("kilo");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(&executable, b"#!/bin/sh\nexit 0\n").unwrap();
    write_receipt(
        &home,
        &InstallReceipt {
            schema: INSTALL_RECEIPT_SCHEMA.into(),
            kind: InstallKind::Agent,
            plan_id: "sha256:plan".into(),
            source: "file:///registry.json".into(),
            registry_id: "kilo-gate".into(),
            name: "Kilo (gate)".into(),
            version: "2.0.0".into(),
            platform: Some("linux-x86_64".into()),
            package: None,
            archive: Some("https://example.invalid/kilo.tar.gz".into()),
            sha256: Some("a".repeat(64)),
            command: Some("./kilo".into()),
            entry_script: None,
            executable: Some(executable.clone()),
            file: None,
            args: vec!["acp".into()],
            discovery_method: "registry-binary-sha256-exact-entry".into(),
            installed_at: 1,
        },
    );

    let discovered = discover_agents_with(Vec::new(), &installed);
    let kilo = discovered
        .iter()
        .find(|agent| agent.id == "kilo-gate")
        .expect("an installed agent the catalogue does not name is still discovered");
    assert_eq!(kilo.name, "Kilo (gate)");
    assert_eq!(kilo.readiness, Readiness::InstalledUnverified);
    assert_eq!(kilo.registry_id.as_deref(), Some("kilo-gate"));
    let launch = kilo.launch.as_ref().expect("launchable from its receipt");
    assert_eq!(launch.executable, executable.display().to_string());
    assert_eq!(launch.args, ["acp"]);
    // The catalogue's own entries are still there, absent as they are here.
    assert!(discovered.iter().any(|agent| agent.id == "opencode"));
}

#[test]
fn an_earlier_products_agents_are_brought_under_the_install_root_with_their_paths() {
    let root = fresh_root("legacy");
    // What a product before the one root left: `<data home>/swem/agents`,
    // receipts without a schema or a kind, absolute paths into that home.
    let legacy = root.join("swem").join("agents");
    let version = legacy.join("claude-acp").join("0.81.0");
    let entry = version
        .join("node_modules")
        .join("claude-agent-acp")
        .join("dist")
        .join("index.js");
    std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
    std::fs::write(&entry, b"// the agent\n").unwrap();
    std::fs::write(
        version.join(INSTALLATION_MANIFEST),
        serde_json::to_vec_pretty(&serde_json::json!({
            "registry_id": "claude-acp",
            "name": "Claude Agent",
            "version": "0.81.0",
            "package": "@agentclientprotocol/claude-agent-acp@0.81.0",
            "entry_script": entry,
            "args": [],
            "discovery_method": "registry-npx-npm-install-ignore-scripts"
        }))
        .unwrap(),
    )
    .unwrap();
    // A stray file in the legacy home is not an agent and is left alone.
    std::fs::write(legacy.join("notes.txt"), b"not an agent").unwrap();

    let installed = root.join("swem").join("workbench").join("installed");
    assert_eq!(adopt_legacy_agents(&legacy, &installed).unwrap(), 1);

    let receipts = load_receipts(&installed, InstallKind::Agent);
    let claude = receipts
        .get("claude-acp")
        .expect("the adopted agent reads from the install root");
    assert_eq!(claude.schema, INSTALL_RECEIPT_SCHEMA);
    assert_eq!(claude.kind, InstallKind::Agent);
    assert_eq!(claude.version, "0.81.0");
    let moved_entry = claude
        .entry_script
        .as_ref()
        .expect("the launch path survived the move");
    assert!(
        moved_entry.starts_with(&installed),
        "the receipt still points into the old home: {}",
        moved_entry.display()
    );
    assert!(moved_entry.is_file(), "{}", moved_entry.display());
    assert!(!version.exists(), "the old directory was moved, not copied");
    assert!(
        legacy.join("notes.txt").is_file(),
        "a file that is not an agent stays where it was"
    );

    // Adopting again moves nothing and changes nothing.
    assert_eq!(adopt_legacy_agents(&legacy, &installed).unwrap(), 0);
    assert_eq!(load_receipts(&installed, InstallKind::Agent).len(), 1);
}

#[test]
fn every_kind_reads_as_one_list_of_receipts() {
    let root = fresh_root("all-kinds");
    let installed = root.join("installed");
    for (kind, id, file) in [
        (InstallKind::Agent, "opencode", "opencode"),
        (InstallKind::Tool, "ffmpeg", "ffmpeg"),
    ] {
        let home = installed.join(kind.directory()).join(id).join("1.0.0");
        std::fs::create_dir_all(&home).unwrap();
        let executable = home.join(file);
        std::fs::write(&executable, b"#!/bin/sh\n").unwrap();
        write_receipt(
            &home,
            &InstallReceipt {
                schema: INSTALL_RECEIPT_SCHEMA.into(),
                kind,
                plan_id: format!("sha256:{id}"),
                source: String::new(),
                registry_id: id.into(),
                name: id.into(),
                version: "1.0.0".into(),
                platform: None,
                package: None,
                archive: Some("https://example.invalid/a.tar.gz".into()),
                sha256: Some("b".repeat(64)),
                command: Some(format!("./{file}")),
                entry_script: None,
                executable: Some(executable),
                file: None,
                args: Vec::new(),
                discovery_method: "registry-binary-sha256-exact-entry".into(),
                installed_at: 2,
            },
        );
    }
    let receipts = all_receipts(&installed);
    let kinds: Vec<(InstallKind, &str)> = receipts
        .iter()
        .map(|receipt| (receipt.kind, receipt.registry_id.as_str()))
        .collect();
    assert_eq!(
        kinds,
        [
            (InstallKind::Agent, "opencode"),
            (InstallKind::Tool, "ffmpeg")
        ]
    );
    assert!(receipts.iter().all(|r| r.schema == INSTALL_RECEIPT_SCHEMA));
}
