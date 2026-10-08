//! Hosts of one person (ADR-0019): a host that is already the person's adds
//! a new one by its address and the word it showed; from then on each asks
//! the other over the link as the page asks it, until one is forgotten.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hyper::Method;
use serde_json::Value;
use swem_host::{
    PersonalAgentProfile, PersonalAgentProfileStore, THIS_MACHINE, WorkbenchShellState,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-peer-hosts-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

/// A host with one agent called `agent`, its link bound with no relay and
/// nothing looked up: two hosts on one machine reach each other directly.
async fn a_host(label: &str, agent: &str) -> (Arc<WorkbenchShellState>, PathBuf) {
    let root = fixture_root(label);
    let workspace = root.join("workspace");
    let home = root.join("home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&home).expect("create agent home");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(
            &PersonalAgentProfile::new(
                agent,
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
    let state = Arc::new(
        WorkbenchShellState::open(
            &root.join("inventory"),
            &root.join("routes.sqlite3"),
            Duration::from_secs(20),
            |_profile| Err("no engine is started here".to_owned()),
        )
        .expect("open shell state"),
    );
    state
        .enable_hosts(&root.join("hosts"), iroh::RelayMode::Disabled)
        .await
        .expect("this host has a key");
    (state, root)
}

async fn body_of(response: hyper::Response<swem_host::ShellBody>) -> (u16, Value) {
    use http_body_util::BodyExt as _;
    let status = response.status().as_u16();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn a_host_is_added_by_its_address_and_word_and_asked_through_the_link() {
    let (laptop, _) = a_host("laptop", "ada").await;
    let (server, _) = a_host("server", "bob").await;

    // Each has a face of its own from its first start.
    let laptop_face = laptop.hosts_standing(false).expect("laptop's face");
    let server_face = server
        .hosts_standing(true)
        .expect("server's face, with a word");
    assert_ne!(
        laptop_face["this"]["host_id"],
        server_face["this"]["host_id"]
    );
    assert_eq!(
        laptop_face["this"]["fingerprint"].as_str().map(str::len),
        Some(14)
    );
    assert!(
        laptop_face["this"]["word"].is_null(),
        "no word unless asked for"
    );
    let word = server_face["this"]["word"]
        .as_str()
        .expect("the server's word")
        .to_owned();
    let address = server_face["this"]["address"]
        .as_str()
        .expect("the server's address")
        .to_owned();
    assert_eq!(laptop_face["hosts"].as_array().map(Vec::len), Some(0));

    // A wrong word is refused, and nothing is kept.
    let refused = laptop.add_host(&address, "nope-nope-nope").await;
    assert!(refused.is_err(), "a wrong word: {refused:?}");
    assert_eq!(
        laptop.hosts_standing(false).expect("hosts")["hosts"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );

    // The person, on the laptop's page, adds the server by its address and word.
    let added = laptop
        .add_host(&address, &word)
        .await
        .expect("the server is added");
    assert_eq!(added["host_id"], server_face["this"]["host_id"]);
    assert_eq!(added["vouched_by"], laptop_face["this"]["host_id"]);
    // Both list each other now; the word is spent.
    let laptop_hosts = laptop.hosts_standing(false).expect("hosts")["hosts"].clone();
    let server_hosts = server.hosts_standing(false).expect("hosts")["hosts"].clone();
    assert_eq!(laptop_hosts[0]["host_id"], server_face["this"]["host_id"]);
    assert_eq!(server_hosts[0]["host_id"], laptop_face["this"]["host_id"]);
    assert!(
        laptop.add_host(&address, &word).await.is_err(),
        "a word is used once"
    );

    // The laptop's page asks the server through the link, as it asks the laptop.
    let server_id = server_face["this"]["host_id"].as_str().expect("id");
    let (status, people) = body_of(
        laptop
            .ask_host(
                server_id,
                Method::GET,
                "/api/people",
                None,
                hyper::body::Bytes::new(),
            )
            .await
            .expect("the server answers"),
    )
    .await;
    assert_eq!(status, 200, "{people}");
    // The server's own page comes through the same link, whole, so that
    // the laptop's page draws it under its own address.
    let page = laptop
        .ask_host(server_id, Method::GET, "/", None, hyper::body::Bytes::new())
        .await
        .expect("the server's page");
    assert_eq!(page.status(), 200);
    let content_type = page
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    assert!(content_type.starts_with("text/html"), "{content_type}");
    let html = {
        use http_body_util::BodyExt as _;
        String::from_utf8_lossy(&page.into_body().collect().await.expect("html").to_bytes())
            .into_owned()
    };
    assert!(html.contains("workbench.js"), "the page names its script");
    let names: Vec<&str> = people["participants"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|one| one["name"].as_str())
        .collect();
    assert!(
        names.contains(&"bob"),
        "the server's agent, through the link: {people}"
    );
    assert!(!names.contains(&"ada"), "not the laptop's own: {people}");

    // And the other way round: the server reaches the laptop too.
    let laptop_id = laptop_face["this"]["host_id"].as_str().expect("id");
    let (status, people) = body_of(
        server
            .ask_host(
                laptop_id,
                Method::GET,
                "/api/people",
                None,
                hyper::body::Bytes::new(),
            )
            .await
            .expect("the laptop answers"),
    )
    .await;
    assert_eq!(status, 200, "{people}");

    // Forgotten on the laptop: the server is told, and neither reaches the other.
    let forgotten = laptop.forget_host(server_id).await.expect("forgotten");
    assert_eq!(forgotten["forgotten"], true);
    assert_eq!(
        laptop.hosts_standing(false).expect("hosts")["hosts"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(
        server.hosts_standing(false).expect("hosts")["hosts"]
            .as_array()
            .map(Vec::len),
        Some(0),
        "the server was told"
    );
    assert!(
        laptop
            .ask_host(
                server_id,
                Method::GET,
                "/api/people",
                None,
                hyper::body::Bytes::new()
            )
            .await
            .is_err(),
        "a forgotten host is not asked"
    );
}

#[tokio::test]
async fn a_third_host_is_told_of_the_others_and_knows_them() {
    let (a, _) = a_host("a", "ada").await;
    let (b, _) = a_host("b", "bob").await;
    let (c, _) = a_host("c", "cyd").await;
    let b_face = b.hosts_standing(true).expect("b");
    a.add_host(
        b_face["this"]["address"].as_str().expect("address"),
        b_face["this"]["word"].as_str().expect("word"),
    )
    .await
    .expect("b added from a");
    let c_face = c.hosts_standing(true).expect("c");
    a.add_host(
        c_face["this"]["address"].as_str().expect("address"),
        c_face["this"]["word"].as_str().expect("word"),
    )
    .await
    .expect("c added from a");
    // c was told of b when it met a; b was told of c by a.
    let c_hosts = c.hosts_standing(false).expect("c's hosts")["hosts"].clone();
    let b_hosts = b.hosts_standing(false).expect("b's hosts")["hosts"].clone();
    assert_eq!(c_hosts.as_array().map(Vec::len), Some(2), "{c_hosts}");
    assert_eq!(b_hosts.as_array().map(Vec::len), Some(2), "{b_hosts}");
    // So b reaches c directly, on a's word.
    let c_id = c_face["this"]["host_id"].as_str().expect("id");
    let (status, _) = body_of(
        b.ask_host(
            c_id,
            Method::GET,
            "/api/people",
            None,
            hyper::body::Bytes::new(),
        )
        .await
        .expect("c answers b"),
    )
    .await;
    assert_eq!(status, 200);
}
