//! The Store for several hosts: kinds by name, takers, requirements,
//! updates and removal, below any page.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use swem_store::{
    Catalog, CatalogEntry, InstallPlan, InstallReceipt, Kind, KindWords, Store, StoreError, Taker,
};

fn root(label: &str) -> PathBuf {
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

/// A tarball of one folder holding `plugin.json` with the given text, and
/// its digest.
fn archive(at: &Path, name: &str, manifest: &str) -> (String, String) {
    use std::io::Write as _;
    let path = at.join(format!("{name}.tar.gz"));
    let file = std::fs::File::create(&path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    let mut tar = tar::Builder::new(encoder);
    let bytes = manifest.as_bytes();
    let mut header = tar::Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    tar.append_data(&mut header, format!("{name}/plugin.json"), bytes)
        .unwrap();
    tar.into_inner().unwrap().finish().unwrap().flush().unwrap();
    let digest = {
        use sha2::Digest as _;
        format!("{:x}", sha2::Sha256::digest(std::fs::read(&path).unwrap()))
    };
    (format!("file://{}", path.display()), digest)
}

/// A host that takes packages of its own kind: a tree with a `plugin.json`
/// whose name is the package's id.
struct Packages {
    kind: Kind,
    taken: Mutex<Vec<String>>,
    removed: Mutex<Vec<String>>,
}

impl Taker for Packages {
    fn kind(&self) -> Kind {
        self.kind.clone()
    }
    fn words(&self) -> KindWords {
        KindWords {
            one: "a package".into(),
            many: "Packages".into(),
            after_install: "The hub has it.".into(),
        }
    }
    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        entry
            .distribution
            .archive
            .as_ref()
            .map(|_| ())
            .ok_or_else(|| "a package is an archive".to_owned())
    }
    fn check(&self, staged: &Path, plan: &InstallPlan) -> Result<(), String> {
        let tree = staged.join("tree");
        let manifest = std::fs::read_dir(&tree)
            .ok()
            .and_then(|entries| entries.flatten().next())
            .map(|entry| entry.path().join("plugin.json"))
            .and_then(|path| std::fs::read_to_string(path).ok())
            .ok_or_else(|| "no plugin.json in the tree".to_owned())?;
        let manifest: serde_json::Value =
            serde_json::from_str(&manifest).map_err(|e| e.to_string())?;
        if manifest["name"] != plan.registry_id {
            return Err(format!(
                "the manifest names {}, the catalog {}",
                manifest["name"], plan.registry_id
            ));
        }
        Ok(())
    }
    fn after_install(&self, receipt: &InstallReceipt) -> Result<(), String> {
        self.taken
            .lock()
            .unwrap()
            .push(format!("{}@{}", receipt.registry_id, receipt.version));
        Ok(())
    }
    fn before_remove(&self, receipt: &InstallReceipt) -> Result<(), String> {
        self.removed
            .lock()
            .unwrap()
            .push(receipt.registry_id.clone());
        Ok(())
    }
}

fn catalog(entries: &str) -> Catalog {
    Catalog::parse(
        format!(r#"{{"schema":"swem:catalog@0.2","name":"Things","entries":[{entries}]}}"#)
            .as_bytes(),
    )
    .unwrap()
}

#[test]
fn a_package_of_another_hosts_kind_is_taken_by_that_host_and_by_nobody_else() {
    let root = root("taken");
    let kind = Kind::parse("example/package@1").unwrap();
    let (url, digest) = archive(&root, "hello", r#"{"name":"hello","version":"0.1.0"}"#);
    let mut store = Store::open(&root.join("indexes"), &root.join("installed")).unwrap();
    store.ship(catalog(&format!(
        r#"{{"kind":"example/package@1","id":"hello","name":"Hello","version":"0.1.0",
            "distribution":{{"archive":{{"url":"{url}","sha256":"{digest}"}}}}}}"#
    )));

    // Nobody takes it: listed, said so, not installable.
    let view = store.view().unwrap();
    let hello = view.entries.iter().find(|e| e.id == "hello").unwrap();
    assert!(!hello.installable && !hello.taken);
    assert!(hello.reason.as_deref().unwrap().contains("does not have"));
    assert!(matches!(
        store.plan(&kind, "hello"),
        Err(StoreError::Invalid(_))
    ));

    // Somebody takes it: planned, checked, installed, handed over.
    let taker = Arc::new(Packages {
        kind: kind.clone(),
        taken: Mutex::default(),
        removed: Mutex::default(),
    });
    store.taken_by(taker.clone());
    let view = store.view().unwrap();
    assert_eq!(view.kinds.len(), 1);
    assert_eq!(view.kinds[0].many, "Packages");
    let hello = view.entries.iter().find(|e| e.id == "hello").unwrap();
    assert!(hello.installable && hello.taken);
    let planned = store.plan(&kind, "hello").unwrap();
    assert!(planned.also.is_empty());
    let receipts = store
        .install(&kind, "hello", &planned.plan.plan_id, None)
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(receipts[0].file.as_ref().unwrap().is_dir());
    assert!(
        receipts[0].file.as_ref().unwrap().starts_with(
            root.join("installed")
                .join("example.package-1")
                .join("hello")
        )
    );
    assert_eq!(taker.taken.lock().unwrap().as_slice(), ["hello@0.1.0"]);
    assert_eq!(store.receipts(&kind).len(), 1);
    assert!(
        store
            .view()
            .unwrap()
            .entries
            .iter()
            .find(|e| e.id == "hello")
            .unwrap()
            .installed
            == Some("0.1.0".into())
    );

    // Removed, after the taker had its say; nothing of it is left.
    store.remove(&kind, "hello").unwrap();
    assert_eq!(taker.removed.lock().unwrap().as_slice(), ["hello"]);
    assert!(store.receipts(&kind).is_empty());
    assert!(
        !root
            .join("installed")
            .join("example.package-1")
            .join("hello")
            .exists()
    );
}

#[test]
fn a_check_that_refuses_leaves_nothing_behind() {
    let root = root("refused");
    let kind = Kind::parse("example/package@1").unwrap();
    // The manifest names another package than the catalog says.
    let (url, digest) = archive(&root, "hello", r#"{"name":"not-hello","version":"0.1.0"}"#);
    let mut store = Store::open(&root.join("indexes"), &root.join("installed")).unwrap();
    store.ship(catalog(&format!(
        r#"{{"kind":"example/package@1","id":"hello","name":"Hello","version":"0.1.0",
            "distribution":{{"archive":{{"url":"{url}","sha256":"{digest}"}}}}}}"#
    )));
    store.taken_by(Arc::new(Packages {
        kind: kind.clone(),
        taken: Mutex::default(),
        removed: Mutex::default(),
    }));
    let planned = store.plan(&kind, "hello").unwrap();
    let refused = store
        .install(&kind, "hello", &planned.plan.plan_id, None)
        .unwrap_err();
    assert!(
        refused.to_string().contains("the manifest names"),
        "{refused}"
    );
    assert!(store.receipts(&kind).is_empty());
    let home = root
        .join("installed")
        .join("example.package-1")
        .join("hello");
    assert!(
        !home.exists() || std::fs::read_dir(&home).unwrap().next().is_none(),
        "something was left under {}",
        home.display()
    );
}

#[test]
fn what_a_package_requires_is_installed_first_in_one_plan() {
    let root = root("requires");
    let kind = Kind::parse("example/package@1").unwrap();
    let (base_url, base_digest) = archive(&root, "base", r#"{"name":"base","version":"1.0.0"}"#);
    let (top_url, top_digest) = archive(&root, "top", r#"{"name":"top","version":"2.0.0"}"#);
    let mut store = Store::open(&root.join("indexes"), &root.join("installed")).unwrap();
    store.ship(catalog(&format!(
        r#"{{"kind":"example/package@1","id":"base","name":"Base","version":"1.0.0",
            "distribution":{{"archive":{{"url":"{base_url}","sha256":"{base_digest}"}}}}}},
           {{"kind":"example/package@1","id":"top","name":"Top","version":"2.0.0",
            "distribution":{{"archive":{{"url":"{top_url}","sha256":"{top_digest}"}}}},
            "requires":[{{"kind":"example/package@1","id":"base","version":"^1"}}]}}"#
    )));
    let taker = Arc::new(Packages {
        kind: kind.clone(),
        taken: Mutex::default(),
        removed: Mutex::default(),
    });
    store.taken_by(taker.clone());
    let planned = store.plan(&kind, "top").unwrap();
    assert_eq!(planned.also.len(), 1);
    assert_eq!(planned.also[0].registry_id, "base");
    let receipts = store
        .install(&kind, "top", &planned.plan.plan_id, None)
        .unwrap();
    assert_eq!(
        receipts
            .iter()
            .map(|r| r.registry_id.as_str())
            .collect::<Vec<_>>(),
        ["base", "top"]
    );
    assert_eq!(
        taker.taken.lock().unwrap().as_slice(),
        ["base@1.0.0", "top@2.0.0"]
    );
    // Planned again, the requirement is there and is not planned twice.
    assert!(store.plan(&kind, "top").unwrap().also.is_empty());

    // A requirement nothing lists is refused in words.
    let mut lonely = Store::open(&root.join("indexes2"), &root.join("installed2")).unwrap();
    lonely.ship(catalog(&format!(
        r#"{{"kind":"example/package@1","id":"top","name":"Top","version":"2.0.0",
            "distribution":{{"archive":{{"url":"{top_url}","sha256":"{top_digest}"}}}},
            "requires":[{{"kind":"example/package@1","id":"base"}}]}}"#
    )));
    lonely.taken_by(taker);
    let refused = lonely.plan(&kind, "top").unwrap_err();
    assert!(
        refused.to_string().contains("no index here lists"),
        "{refused}"
    );
}

#[test]
fn the_newest_of_several_versions_is_the_newest_and_not_the_last_by_text() {
    let root = root("versions");
    let kind = Kind::parse("example/package@1").unwrap();
    let mut store = Store::open(&root.join("indexes"), &root.join("installed")).unwrap();
    let taker = Arc::new(Packages {
        kind: kind.clone(),
        taken: Mutex::default(),
        removed: Mutex::default(),
    });
    store.taken_by(taker);
    for version in ["0.9.0", "0.10.0"] {
        let (url, digest) = archive(
            &root,
            &format!("hello-{version}"),
            &format!(r#"{{"name":"hello","version":"{version}"}}"#),
        );
        let mut one = Store::open(&root.join("indexes"), &root.join("installed")).unwrap();
        one.ship(catalog(&format!(
            r#"{{"kind":"example/package@1","id":"hello","name":"Hello","version":"{version}",
                "distribution":{{"archive":{{"url":"{url}","sha256":"{digest}"}}}}}}"#
        )));
        one.taken_by(Arc::new(Packages {
            kind: kind.clone(),
            taken: Mutex::default(),
            removed: Mutex::default(),
        }));
        let planned = one.plan(&kind, "hello").unwrap();
        one.install(&kind, "hello", &planned.plan.plan_id, None)
            .unwrap();
    }
    assert_eq!(store.receipts(&kind)["hello"].version, "0.10.0");
    // An index with a newer version says so.
    let (url, digest) = archive(&root, "hello-1", r#"{"name":"hello","version":"1.0.0"}"#);
    store.ship(catalog(&format!(
        r#"{{"kind":"example/package@1","id":"hello","name":"Hello","version":"1.0.0",
            "distribution":{{"archive":{{"url":"{url}","sha256":"{digest}"}}}}}}"#
    )));
    let hello = store.view().unwrap();
    let hello = hello.entries.iter().find(|e| e.id == "hello").unwrap();
    assert!(hello.newer);
    assert_eq!(hello.installed.as_deref(), Some("0.10.0"));
    assert!(hello.removable);
}

/// A server reached through the host, as a test has it: it plans by
/// reading the package's manifest, installs by copying the tree home, and
/// remembers what it was asked.
#[derive(Default)]
struct Hub {
    home: PathBuf,
    asked: Mutex<Vec<String>>,
}

impl swem_store::Through for Hub {
    fn is_there(&self, server: &str) -> bool {
        server == "hub"
    }
    fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        assert_eq!(server, "hub");
        self.asked
            .lock()
            .unwrap()
            .push(format!("{tool} {arguments}"));
        match tool {
            "plan_package" => {
                let path = arguments["source"]["path"].as_str().ok_or("no path")?;
                let manifest: serde_json::Value = serde_json::from_str(
                    &std::fs::read_to_string(Path::new(path).join("plugin.json"))
                        .map_err(|e| e.to_string())?,
                )
                .map_err(|e| e.to_string())?;
                std::fs::create_dir_all(self.home.join(".staging")).unwrap();
                std::fs::copy(
                    Path::new(path).join("plugin.json"),
                    self.home.join(".staging").join("plugin.json"),
                )
                .unwrap();
                Ok(
                    serde_json::json!({"plan_id": "sha256-hub", "id": manifest["name"], "version": manifest["version"]}),
                )
            }
            "install_package" => {
                assert_eq!(arguments["plan_id"], "sha256-hub");
                let manifest: serde_json::Value = serde_json::from_str(
                    &std::fs::read_to_string(self.home.join(".staging").join("plugin.json"))
                        .unwrap(),
                )
                .unwrap();
                let into = self.home.join(manifest["name"].as_str().unwrap());
                std::fs::create_dir_all(&into).unwrap();
                std::fs::rename(
                    self.home.join(".staging").join("plugin.json"),
                    into.join("plugin.json"),
                )
                .unwrap();
                Ok(serde_json::json!({"id": manifest["name"]}))
            }
            "remove_package" => {
                let id = arguments["id"].as_str().ok_or("no id")?;
                std::fs::remove_dir_all(self.home.join(id)).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({"id": id}))
            }
            other => Err(format!("no tool {other}")),
        }
    }
}

#[test]
fn a_server_that_says_it_takes_a_kind_takes_packages_through_the_host() {
    let root = root("through");
    let kind = Kind::parse("swem.cycle/package@1").unwrap();
    let (url, digest) = archive(&root, "hello", r#"{"name":"hello","version":"0.1.0"}"#);
    let (wrong_url, wrong_digest) =
        archive(&root, "other", r#"{"name":"not-other","version":"0.1.0"}"#);
    let mut store = Store::open(&root.join("indexes"), &root.join("installed")).unwrap();
    store.ship(catalog(&format!(
        r#"{{"kind":"server","id":"hub","name":"The hub","version":"1","bundled":true,
            "takes":[{{"kind":"swem.cycle/package@1","plan":"plan_package","install":"install_package",
                      "remove":"remove_package","one":"a Cycle package","many":"Cycle packages"}}]}},
           {{"kind":"swem.cycle/package@1","id":"hello","name":"Hello","version":"0.1.0",
            "distribution":{{"archive":{{"url":"{url}","sha256":"{digest}"}}}},
            "requires":[{{"kind":"server","id":"hub"}}]}},
           {{"kind":"swem.cycle/package@1","id":"other","name":"Other","version":"0.1.0",
            "distribution":{{"archive":{{"url":"{wrong_url}","sha256":"{wrong_digest}"}}}}}}"#
    )));
    // Nobody takes it until a server that is there says so.
    assert!(store.taker(&kind).is_none());
    let hub = Arc::new(Hub {
        home: root.join("hub-home"),
        asked: Mutex::default(),
    });
    store.servers_take_through(hub.clone());
    let view = store.view().unwrap();
    assert!(
        view.kinds
            .iter()
            .any(|shown| shown.kind == kind && shown.many == "Cycle packages")
    );
    let hello = view.entries.iter().find(|e| e.id == "hello").unwrap();
    assert!(hello.installable && hello.taken);

    // Planned with what it requires: the hub is bundled, so nothing more.
    let planned = store.plan(&kind, "hello").unwrap();
    assert!(planned.also.is_empty());
    let receipts = store
        .install(&kind, "hello", &planned.plan.plan_id, None)
        .unwrap();
    assert_eq!(receipts.len(), 1);
    assert!(
        root.join("hub-home")
            .join("hello")
            .join("plugin.json")
            .is_file()
    );
    // Planned by the hub twice: of the staged tree, so a package it refuses
    // is never committed, and of the tree where it was put, which is the
    // one the hub installs from.
    let asked = hub.asked.lock().unwrap().clone();
    assert!(
        asked[0].starts_with("plan_package {\"source\":{\"kind\":\"directory\",\"path\":\"")
            && asked[0].contains(".staging-"),
        "{asked:?}"
    );
    assert!(
        asked[1].starts_with("plan_package {\"source\":{\"kind\":\"directory\",\"path\":\"")
            && asked[1].contains("/hello/0.1.0/tree/hello")
            && !asked[1].contains(".staging-"),
        "{asked:?}"
    );
    assert!(
        asked[2].starts_with("install_package {\"plan_id\":\"sha256-hub\"}"),
        "{asked:?}"
    );

    // A package that calls itself something else is refused by the hub's
    // own plan, and nothing lands anywhere.
    let planned = store.plan(&kind, "other").unwrap();
    let refused = store
        .install(&kind, "other", &planned.plan.plan_id, None)
        .unwrap_err();
    assert!(refused.to_string().contains("calls itself"), "{refused}");
    assert!(!root.join("hub-home").join("not-other").exists());
    assert!(!store.receipts(&kind).contains_key("other"));

    // Removed through the hub.
    store.remove(&kind, "hello").unwrap();
    assert!(!root.join("hub-home").join("hello").exists());
    assert!(
        hub.asked
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .starts_with("remove_package {\"id\":\"hello\"}")
    );
}
