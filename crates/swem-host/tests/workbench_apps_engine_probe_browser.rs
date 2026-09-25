//! Engine-probe browser gate: what a sandboxed MCP App can use on this host,
//! measured inside the REAL generic Workbench sandbox (official `AppBridge`,
//! different-origin proxy, Rust-resolved CSP header, srcdoc view) and
//! persisted through an App-only tool. The oracle is the receipt file the
//! fixture server wrote, never the page's prose. One fact is required (an
//! `AudioContext` runs after a user gesture); the rest are recorded as the
//! measured substrate the domain surfaces build on, and copied to
//! `tmp/evidence/apps-engine-probe-0.71/facts.json`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use swem_host::{
    AttachmentBinding, AttachmentTransport, IntegrationKind, LaunchCommand, PersonalAgentProfile,
    PersonalAgentProfileStore, ResolvedDirectAgentConnection, WorkbenchShellState,
    serve_workbench_http_with_apps,
};

fn browser_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SWEM_BROWSER") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path);
    }
    [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn node_available() -> bool {
    std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// `npm ci` (once) + esbuild the official `AppBridge` bundle into `out_dir`.
/// `npm`/`npx`, launched the way the running platform can launch them.
///
/// On Windows these are batch shims that only a shell resolves; everywhere else
/// `cmd` does not exist, so spawning it fails with `NotFound` and this gate -
/// which is `#[ignore]`d, and therefore silent - never runs at all. The same
/// platform bug lived in the strict lint until `40153cc`; `tests/web_suite.rs`
/// already carries this shape.
fn node_tool(program: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        let mut command = std::process::Command::new("cmd");
        command.arg("/C").arg(program);
        command
    }
    #[cfg(not(windows))]
    std::process::Command::new(program)
}

fn build_apps_bundle(out_dir: &Path) {
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/apps-host");
    if !web.join("node_modules").is_dir() {
        let install = node_tool("npm")
            .args(["ci"])
            .current_dir(&web)
            .output()
            .expect("run npm ci");
        assert!(
            install.status.success(),
            "npm ci failed:\n{}",
            String::from_utf8_lossy(&install.stderr)
        );
    }
    let outfile = format!("--outfile={}", out_dir.join("apps-bridge.js").display());
    let bundle = node_tool("npx")
        .args([
            "esbuild",
            "apps-bridge.entry.mjs",
            "--bundle",
            "--format=iife",
            &outfile,
        ])
        .current_dir(&web)
        .output()
        .expect("run esbuild");
    assert!(
        bundle.status.success(),
        "esbuild failed:\n{}",
        String::from_utf8_lossy(&bundle.stderr)
    );
}

/// Open one probe App of the fixture in the real Workbench, let it measure
/// and acknowledge, and return its facts with the workspace the shell gave
/// the fixture (an upload lands there).
#[allow(
    clippy::too_many_lines,
    reason = "one browser transaction keeps the fixture wiring and the receipt oracle together"
)]
async fn probe_facts(app_uri: &str, label: &str) -> (serde_json::Value, PathBuf, PathBuf) {
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(node_available(), "put Node.js on PATH");
    let root = std::env::temp_dir().join(format!(
        "swem-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let bundle_dir = root.join("bundle");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    fs::create_dir_all(&bundle_dir).expect("create bundle dir");
    build_apps_bundle(&bundle_dir);
    let probe_receipt = root.join("engine-probe-facts.json");

    PersonalAgentProfileStore::open(&inventory)
        .expect("open profile inventory")
        .create(
            &PersonalAgentProfile::new(
                "apps-probe",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                vec![AttachmentBinding::new(
                    "notes-attachment",
                    "notes",
                    AttachmentTransport::Stdio,
                )],
                Vec::new(),
            )
            .expect("build fixture profile"),
        )
        .expect("persist fixture profile");
    let root_for_resolver = root.clone();
    let receipt_for_resolver = probe_receipt.clone();
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_secs(30),
        move |_profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![McpServer::Stdio(
                    McpServerStdio::new(
                        "notes",
                        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                    )
                    .args(vec![
                        "--receipt".to_owned(),
                        root_for_resolver
                            .join("notes-receipt.json")
                            .display()
                            .to_string(),
                        "--poison".to_owned(),
                        root_for_resolver
                            .join("poison-receipt.json")
                            .display()
                            .to_string(),
                        "--probe-receipt".to_owned(),
                        receipt_for_resolver.display().to_string(),
                    ]),
                )],
            })
        },
    )
    .expect("open shell state");
    let state = Arc::new(state);
    let handle = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle_dir),
    )
    .await
    .expect("bind shell + sandbox listeners");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/workbench_apps_engine_probe_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new("node")
            .arg(&driver)
            .arg(&browser)
            .arg(&url)
            .arg(app_uri)
            .output(),
    )
    .await
    .expect("browser drive timed out")
    .expect("node failed to start");
    assert!(
        output.status.success(),
        "driver failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facts: serde_json::Value = serde_json::from_slice(
        &fs::read(&probe_receipt).expect("the probe App acknowledged its facts"),
    )
    .expect("facts JSON");
    handle.shutdown().await;
    drop(state);
    (facts, workspace, root)
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome, Node.js and npm"]
async fn a_real_sandboxed_app_reports_the_engine_substrate_it_can_use() {
    let (facts, workspace, root) =
        probe_facts("ui://apps-fixture/engine-probe", "engine-probe").await;
    // Required by the composition workbench: decoded audio plays after a
    // user gesture (0.71), and the affordances the host grants by declared
    // metadata (0.73) hold inside the real sandbox: WebAssembly instantiates
    // under the App CSP, a worklet module loads from a served `ui://` script
    // resource on the sandbox origin, a re-exported callback can be placed in
    // a wasm table, and a declared permission reaches the View's policy.
    // The rest stays a measurement (blob: sources and isolation are not
    // granted; the facts say so).
    assert_eq!(facts["audio_context"], "running", "facts: {facts}");
    assert_eq!(facts["wasm"], "instantiated", "facts: {facts}");
    assert_eq!(
        facts["audio_worklet_served"], "registered",
        "facts: {facts}"
    );
    assert_eq!(facts["table_set_reexport"], "ok", "facts: {facts}");
    assert_eq!(
        facts["permissions_policy_microphone"], true,
        "facts: {facts}"
    );
    // The grant is not sufficient for the View itself: a sandboxed srcdoc
    // document has an opaque origin, the browser refuses `getUserMedia` to
    // one whatever the policy says, and will not transfer a track into one.
    // The sandbox proxy, on its real origin with the same declared
    // permission, opens the (fake) microphone, runs the App's served worklet
    // on it and hands the worklet's port into the View, where the input's
    // peak arrives with the proxy's clock; and the View put bytes back into
    // its server's workspace through the upload route on that origin - the
    // two halves a recording stands on.
    assert!(
        facts["microphone_stream"]
            .as_str()
            .is_some_and(|state| state == "live" || state.contains("SecurityError")),
        "facts: {facts}"
    );
    assert_eq!(facts["sandbox_user_media"], "live", "facts: {facts}");
    assert!(
        facts["sandbox_media_devices"].as_u64().unwrap_or(0) >= 1,
        "the proxy lists no input: {facts}"
    );
    assert_eq!(facts["sandbox_context_rate"], 48000, "facts: {facts}");
    assert!(
        facts["sandbox_capture_peak"].as_f64().unwrap_or(0.0) > 0.001,
        "the proxy's capture carries no signal: {facts}"
    );
    assert!(
        facts["sandbox_clock_ticks"].as_u64().unwrap_or(0) >= 2,
        "the proxy's clock never ticked: {facts}"
    );
    assert_eq!(facts["self_upload"], 200, "facts: {facts}");
    assert_eq!(facts["self_upload_bytes"], 70000, "facts: {facts}");
    let uploaded = facts["self_upload_path"]
        .as_str()
        .expect("the upload route answered a workspace path");
    assert!(uploaded.starts_with("uploads/u-"), "{uploaded}");
    let bytes = fs::read(workspace.join(uploaded)).expect("the upload landed in the workspace");
    assert_eq!(bytes.len(), 70000);
    assert!(
        bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| usize::from(*byte) == index & 0xff),
        "the bytes are the ones the View sent"
    );
    for key in [
        "audio_worklet_blob",
        "worker_blob",
        "shared_array_buffer",
        "cross_origin_isolated",
        "webgpu",
        "web_midi",
        "origin",
        "base_origin",
    ] {
        assert!(facts.get(key).is_some(), "probe recorded no {key}: {facts}");
    }
    let evidence = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tmp/evidence/plugin-0.73");
    fs::create_dir_all(&evidence).expect("create evidence dir");
    fs::write(
        evidence.join("facts.json"),
        serde_json::to_vec_pretty(&facts).expect("encode facts"),
    )
    .expect("write evidence");
    eprintln!("engine probe facts: {facts}");

    let _ = fs::remove_dir_all(&root);
}

/// An App that asks for a real origin (`_meta.ui.origin: "isolated"`) is
/// served as a document of the sandbox origin: it is cross-origin isolated
/// (the shell and the proxy carry the policies), owns storage, loads a
/// module worker, a worklet and a wasm module from files under its own path
/// with the media types the server lists, holds the microphone itself, and
/// still speaks the Apps protocol to the host through the proxy.
#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome, Node.js and npm"]
async fn an_app_that_asks_for_a_real_origin_gets_isolation_storage_and_its_files() {
    let (facts, _workspace, root) =
        probe_facts("ui://apps-fixture/isolated-probe", "isolated-probe").await;
    assert_eq!(facts["ui_initialized"], true, "facts: {facts}");
    let origin = facts["origin"].as_str().unwrap_or_default();
    assert!(origin.starts_with("http://127.0.0.1:"), "facts: {facts}");
    let base = facts["base_uri"].as_str().unwrap_or_default();
    assert!(base.ends_with("/view/"), "facts: {facts}");
    assert_eq!(facts["cross_origin_isolated"], true, "facts: {facts}");
    assert_eq!(facts["shared_array_buffer"], true, "facts: {facts}");
    assert_eq!(facts["opfs"], "ok", "facts: {facts}");
    assert_eq!(facts["local_storage"], "ok", "facts: {facts}");
    assert_eq!(facts["worker_served"]["echo"], "ping", "facts: {facts}");
    assert_eq!(facts["worker_served"]["isolated"], true, "facts: {facts}");
    // A worker the page builds from a blob of its own code: what the music
    // engine's recording ring reader is (it was once found refused).
    assert_eq!(facts["worker_blob"]["echo"], "ping", "facts: {facts}");
    assert_eq!(facts["worker_blob"]["isolated"], true, "facts: {facts}");
    assert_eq!(facts["audio_context"], "running", "facts: {facts}");
    assert_eq!(facts["worklet_served"], "registered", "facts: {facts}");
    assert_eq!(
        facts["wasm_content_type"], "application/wasm",
        "facts: {facts}"
    );
    assert_eq!(facts["wasm_served"], "compiled", "facts: {facts}");
    assert_eq!(facts["microphone_stream"], "live", "facts: {facts}");
    eprintln!("isolated probe facts: {facts}");
    let _ = fs::remove_dir_all(&root);
}
