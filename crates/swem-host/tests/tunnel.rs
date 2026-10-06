//! A tunnel: an address from outside, for a while - below any page. The
//! tunnel fixture answers `open` with the URL it is handed, so what is
//! reached "through the tunnel" is the harness's own gate on loopback, and
//! what the gate answers and refuses is what a stranger at the public
//! address would get.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use swem_host::{Kind, StoreInstallBody, StorePlanBody, WorkbenchShellState, registry_platform};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-tunnel-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn sha256_of(path: &Path) -> String {
    use sha2::Digest as _;
    format!(
        "{:x}",
        sha2::Sha256::digest(std::fs::read(path).expect("read the file"))
    )
}

/// A Workbench with the Store enabled and the tunnel fixture installed from
/// a file catalog, as one bare executable: the road a tool takes.
fn shell_with_the_tunnel_fixture(root: &Path) -> Arc<WorkbenchShellState> {
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_| panic!("no agent connection is resolved here"),
    )
    .expect("open shell state");
    let state = Arc::new(state);
    state
        .enable_installs(root.join("installed"))
        .expect("installs");
    state
        .enable_tunnels(&root.join("tunnels"))
        .expect("tunnels");
    state
        .enable_channels(&root.join("channels"))
        .expect("channels");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-tunnel-fixture"));
    let platform = registry_platform().expect("a platform this test runs on");
    let catalog = swem_host::Catalog::parse(
        format!(
            r#"{{"schema":"swem:catalog@0.2","name":"Tunnels for the test","entries":[
              {{"kind":"swem/tunnel@1","id":"nowhere","name":"A tunnel to nowhere","version":"0.1.0",
                "distribution":{{"binary":{{"{platform}":{{"archive":"file://{}","sha256":"{}","cmd":"./swem-tunnel-nowhere"}}}}}}}}]}}"#,
            fixture.display(),
            sha256_of(&fixture)
        )
        .as_bytes(),
    )
    .expect("a catalog");
    state
        .enable_store(&root.join("indexes"), vec![catalog])
        .expect("the store");
    state
}

fn install_the_fixture(state: &Arc<WorkbenchShellState>) {
    let kind = Kind::parse("swem/tunnel@1").expect("a kind");
    let planned = state
        .store_plan(&StorePlanBody {
            kind: kind.clone(),
            id: "nowhere".into(),
        })
        .expect("a plan");
    state
        .store_install(&StoreInstallBody {
            kind,
            id: "nowhere".into(),
            plan_id: planned.plan.plan_id,
        })
        .expect("installed");
}

/// One HTTP/1.1 request to the gate, by hand.
fn ask_the_gate(origin: &str, method: &str, path: &str) -> (u16, String) {
    let host = origin.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(host).expect("the gate listens");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .expect("write the request");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read the answer");
    let status: u16 = answer
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .expect("a status");
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_owned())
        .unwrap_or_default();
    (status, body)
}

/// Installed from the Store as one bare executable, a tunnel opens at the
/// gate; the gate answers the app and nothing else; closed, it is gone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tunnel_opens_the_app_and_nothing_else() {
    let root = fixture_root("gate");
    let state = shell_with_the_tunnel_fixture(&root);
    let before = state.reach_standing().expect("standing");
    assert!(before.packages.is_empty() && before.tunnel.is_none() && before.served_at.is_none());
    let nothing = state.open_tunnel(None).await.unwrap_err();
    assert!(nothing.to_string().contains("install one"), "{nothing}");

    install_the_fixture(&state);
    let standing = state.reach_standing().expect("standing");
    assert_eq!(standing.packages.len(), 1);
    assert_eq!(standing.packages[0].id, "nowhere");
    assert!(!standing.packages[0].bundled);

    let opened = state.open_tunnel(None).await.expect("the tunnel opens");
    assert_eq!(opened.package, "nowhere");
    assert!(
        opened.origin.starts_with("http://127.0.0.1:"),
        "{}",
        opened.origin
    );
    // Opened again, it is the same one.
    let again = state.open_tunnel(None).await.expect("still open");
    assert_eq!(again.origin, opened.origin);
    // A second address, for the sandbox the Apps are drawn in: the package
    // stood twice, and the sandbox's own router answers there.
    let sandbox = opened
        .sandbox_origin
        .clone()
        .expect("the sandbox's address through the tunnel");
    assert_ne!(sandbox, opened.origin);
    let (status, body) = ask_the_gate(&sandbox, "GET", "/sandbox?csp=");
    assert_eq!(status, 200, "{body}");
    let (status, _) = ask_the_gate(&sandbox, "GET", "/api/chats");
    assert_eq!(status, 404, "the sandbox's address answers no API");

    // The Workbench's own page is what the gate serves, for somebody who
    // came through a messenger to open; behind it, without a session of
    // theirs, nothing - and no word of sign-in, the run's secret, a knock
    // or a channel's webhook door.
    for path in ["/", "/workbench.js", "/api/access"] {
        let (status, body) = ask_the_gate(&opened.origin, "GET", path);
        assert_eq!(status, 200, "GET {path} through the gate");
        if path == "/" {
            assert!(body.contains("<title>SWEM Workbench</title>"), "the page");
        }
    }
    // No such bot: the exchange refuses, with nothing of a session.
    let (status, _) = ask_the_gate(&opened.origin, "POST", "/api/access/by-channel/some-bot");
    assert_ne!(status, 200);
    for (method, path) in [
        ("GET", "/api/chats"),
        ("POST", "/api/time/due"),
        ("GET", "/api/stream"),
        ("GET", "/api/profiles"),
        ("POST", "/api/channels/some-bot/receive"),
        ("POST", "/api/access/register/begin"),
        ("POST", "/api/access/sign-in/begin"),
        ("GET", "/api/reach"),
    ] {
        let (status, body) = ask_the_gate(&opened.origin, method, path);
        assert_eq!(status, 403, "{method} {path} through the gate: {body}");
        assert!(
            !body.contains("sign"),
            "{method} {path} says something: {body}"
        );
    }

    state.close_tunnel().await;
    assert!(state.reach_standing().expect("standing").tunnel.is_none());
    assert!(
        std::net::TcpStream::connect(opened.origin.trim_start_matches("http://")).is_err(),
        "the gate still listens after the tunnel closed"
    );
    assert!(
        std::net::TcpStream::connect(sandbox.trim_start_matches("http://")).is_err(),
        "the sandbox's listener still listens after the tunnel closed"
    );
    let calls = std::fs::read_to_string(root.join("tunnels").join("nowhere").join("calls.jsonl"))
        .expect("the fixture wrote its calls");
    assert!(
        calls.contains("\"tool\":\"open\"") && calls.contains("\"tool\":\"close\""),
        "{calls}"
    );
}

/// Unused for a while, a tunnel closes by itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tunnel_nobody_uses_closes_by_itself() {
    let root = fixture_root("idle");
    let state = shell_with_the_tunnel_fixture(&root);
    install_the_fixture(&state);
    let opened = Box::pin(state.open_tunnel_for(Some("nowhere"), Duration::from_millis(1_500)))
        .await
        .expect("the tunnel opens");
    assert!(state.reach_standing().expect("standing").tunnel.is_some());
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    assert!(
        state.reach_standing().expect("standing").tunnel.is_none(),
        "the tunnel stayed open with nobody using it"
    );
    assert!(std::net::TcpStream::connect(opened.origin.trim_start_matches("http://")).is_err());
}

/// A program that is not a tunnel is refused in words, and nothing stays
/// open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_that_is_not_a_tunnel_is_refused_in_words() {
    let root = fixture_root("refused");
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_| panic!("no agent connection is resolved here"),
    )
    .expect("open shell state");
    let state = Arc::new(state);
    state
        .enable_installs(root.join("installed"))
        .expect("installs");
    state
        .enable_tunnels(&root.join("tunnels"))
        .expect("tunnels");
    // The channel fixture is an MCP server, but not a tunnel.
    let not_a_tunnel = PathBuf::from(env!("CARGO_BIN_EXE_swem-channel-fixture"));
    let platform = registry_platform().expect("a platform this test runs on");
    let catalog = swem_host::Catalog::parse(
        format!(
            r#"{{"schema":"swem:catalog@0.2","name":"Not tunnels","entries":[
              {{"kind":"swem/tunnel@1","id":"channel","name":"A channel, not a tunnel","version":"0.1.0",
                "distribution":{{"binary":{{"{platform}":{{"archive":"file://{}","sha256":"{}","cmd":"./swem-tunnel-channel"}}}}}}}}]}}"#,
            not_a_tunnel.display(),
            sha256_of(&not_a_tunnel)
        )
        .as_bytes(),
    )
    .expect("a catalog");
    state
        .enable_store(&root.join("indexes"), vec![catalog])
        .expect("the store");
    let kind = Kind::parse("swem/tunnel@1").expect("a kind");
    let planned = state
        .store_plan(&StorePlanBody {
            kind: kind.clone(),
            id: "channel".into(),
        })
        .expect("a plan");
    let refused = state
        .store_install(&StoreInstallBody {
            kind,
            id: "channel".into(),
            plan_id: planned.plan.plan_id,
        })
        .unwrap_err();
    assert!(refused.to_string().contains("is not a tunnel"), "{refused}");
    assert!(
        state
            .reach_standing()
            .expect("standing")
            .packages
            .is_empty()
    );
}

/// What a tunnel requires and is not installed is said, with where it
/// comes from; one consent installs it and opens the tunnel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_tunnel_needs_is_installed_with_one_consent() {
    let root = fixture_root("needs");
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_| panic!("no agent connection is resolved here"),
    )
    .expect("open shell state");
    let state = Arc::new(state);
    state
        .enable_installs(root.join("installed"))
        .expect("installs");
    state
        .enable_tunnels(&root.join("tunnels"))
        .expect("tunnels");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-tunnel-fixture"));
    // A tool the tunnel needs: any program will do for the Store.
    let tool = PathBuf::from(env!("CARGO_BIN_EXE_swem-channel-fixture"));
    let platform = registry_platform().expect("a platform this test runs on");
    let catalog = swem_host::Catalog::parse(
        format!(
            r#"{{"schema":"swem:catalog@0.2","name":"Tunnels for the test","entries":[
              {{"kind":"tool","id":"the-tool","name":"The tool","version":"1.0.0",
                "distribution":{{"binary":{{"{platform}":{{"archive":"file://{}","sha256":"{}","cmd":"./the-tool"}}}}}}}},
              {{"kind":"swem/tunnel@1","id":"nowhere","name":"A tunnel to nowhere","version":"0.1.0",
                "requires":[{{"kind":"tool","id":"the-tool"}}],
                "distribution":{{"binary":{{"{platform}":{{"archive":"file://{}","sha256":"{}","cmd":"./swem-tunnel-nowhere"}}}}}}}}]}}"#,
            tool.display(),
            sha256_of(&tool),
            fixture.display(),
            sha256_of(&fixture)
        )
        .as_bytes(),
    )
    .expect("a catalog");
    state
        .enable_store(&root.join("indexes"), vec![catalog])
        .expect("the store");
    install_the_fixture(&state);
    assert!(state.reach_standing().expect("standing").needs.is_empty());
    // The tool is taken away: the tunnel needs it again, and says so.
    state
        .store_remove(&swem_host::StoreRemoveBody {
            kind: Kind::TOOL,
            id: "the-tool".into(),
        })
        .expect("the tool removed");
    let needs = state.reach_standing().expect("standing").needs;
    assert_eq!(needs.len(), 1, "{needs:?}");
    assert_eq!(needs[0].id, "the-tool");
    assert_eq!(needs[0].version, "1.0.0");
    // One consent: installed, then the tunnel opens.
    let opened = state
        .install_and_open_tunnel()
        .await
        .expect("installed and opened");
    assert!(opened.origin.starts_with("http://127.0.0.1:"));
    assert!(state.reach_standing().expect("standing").needs.is_empty());
    state.close_tunnel().await;
}
