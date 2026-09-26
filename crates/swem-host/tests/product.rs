//! The product, assembled from a data root by whoever embeds the harness.
//!
//! Two products in one process read what each was given, not what the
//! process was told; and the example that embeds the Workbench in a page of
//! code opens a door a browser can reach.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use swem_host::product::{DataRoot, Product};

fn fresh_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-product-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[tokio::test]
async fn two_products_in_one_process_read_different_registries() {
    let first = Product::at(DataRoot::at(fresh_root("first")))
        .acp_registry("http://127.0.0.1:1/first/index.json")
        .assemble()
        .expect("the first product assembles");
    let second = Product::at(DataRoot::at(fresh_root("second")))
        .acp_registry("http://127.0.0.1:1/second/index.json")
        .assemble()
        .expect("the second product assembles");
    let third = Product::at(DataRoot::at(fresh_root("third")))
        .assemble()
        .expect("a product told no registry assembles");
    assert_eq!(
        first.state.acp_registry_index(),
        "http://127.0.0.1:1/first/index.json"
    );
    assert_eq!(
        second.state.acp_registry_index(),
        "http://127.0.0.1:1/second/index.json"
    );
    assert_eq!(
        third.state.acp_registry_index(),
        swem_host::acp_registry_index()
    );
    // The Store says which registry it read, or tried to: each product its own.
    let first_store = first.state.store().expect("the first store answers");
    let second_store = second.state.store().expect("the second store answers");
    assert_eq!(
        first_store.registry.url,
        "http://127.0.0.1:1/first/index.json"
    );
    assert_eq!(
        second_store.registry.url,
        "http://127.0.0.1:1/second/index.json"
    );
    // The layout is the data root's, named once.
    assert!(first.root.installed().starts_with(first.root.path()));
    assert!(first.root.path().join("indexes").is_dir());
}

/// The example built beside this crate's own binaries by `cargo test`.
fn embed_example() -> PathBuf {
    let beside = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    beside
        .parent()
        .expect("a binary has a directory")
        .join("examples")
        .join(format!("embed{suffix}"))
}

#[test]
fn an_embedder_serves_a_workbench_from_a_page_of_code() {
    let example = embed_example();
    assert!(
        example.is_file(),
        "cargo test builds the example beside the binaries: {}",
        example.display()
    );
    let root = fresh_root("embed");
    let mut child = Command::new(&example)
        .env("SWEM_EMBED_ROOT", &root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start the embed example");
    let stdout = child.stdout.take().expect("the example's stdout");
    let mut lines = BufReader::new(stdout).lines();
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut url = None;
    while Instant::now() < deadline {
        let Some(Ok(line)) = lines.next() else { break };
        if let Some(found) = line.strip_prefix("SWEM Workbench: ") {
            url = Some(found.trim().to_owned());
            break;
        }
    }
    let url = url.unwrap_or_else(|| {
        let _ = child.kill();
        panic!("the example printed no Workbench address")
    });

    // The door answers a browser's first request: the page, at the address
    // with this run's secret.
    let (host_and_port, query) = url
        .trim_start_matches("http://")
        .split_once("/?")
        .expect("an address with a query");
    let mut stream = std::net::TcpStream::connect(host_and_port).expect("reach the door");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    write!(
        stream,
        "GET /?{query} HTTP/1.1\r\nHost: {host_and_port}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    let _ = child.kill();
    let _ = child.wait();
    assert!(answer.starts_with("HTTP/1.1 200"), "{answer}");
    assert!(
        answer.contains("<html"),
        "the door serves the page: {answer}"
    );
    // And the product wrote its layout under the root it was given.
    assert!(
        root.join("profiles").is_dir() || root.join("indexes").is_dir(),
        "{}",
        root.display()
    );
}
