//! A real browser: a server that declares a home App appears as a space on
//! the switcher, and opening it mounts the App.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use swem_host::{DeclareMcpServerBody, WorkbenchShellState, serve_workbench_http_with_apps};

fn browser_path() -> Option<PathBuf> {
    std::env::var("SWEM_BROWSER")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

#[tokio::test]
#[ignore = "live browser test: requires Chrome and Node.js"]
async fn a_real_browser_opens_a_servers_home_app_as_a_space() {
    let browser = browser_path().expect("set SWEM_BROWSER before running this ignored test");
    let root = std::env::temp_dir().join(format!(
        "swem-spaces-browser-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_| panic!("no agent connection is resolved here"),
    )
    .expect("open shell state");
    state
        .enable_mcp_catalogue(&root.join("mcp-servers"), Arc::default())
        .expect("the catalogue");
    state
        .declare_mcp_server(&DeclareMcpServerBody {
            name: "notes".into(),
            transport: "stdio".into(),
            command: env!("CARGO_BIN_EXE_swem-mcp-apps-fixture").to_owned(),
            args: vec![
                "--receipt".into(),
                root.join("receipt.json").display().to_string(),
                "--poison".into(),
                root.join("poison.json").display().to_string(),
                "--home".into(),
            ],
            env: Vec::new(),
            url: String::new(),
            headers: Vec::new(),
        })
        .expect("declare the fixture");
    let state = Arc::new(state);
    let handle = serve_workbench_http_with_apps(Arc::clone(&state), ([127, 0, 0, 1], 0).into(), None)
        .await
        .expect("serve");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workbench_shell_spaces_cdp_driver.mjs");
    let output = tokio::process::Command::new("node")
        .arg(driver)
        .arg(&browser)
        .arg(&url)
        .output()
        .await
        .expect("run the driver");
    let stdout = String::from_utf8_lossy(&output.stdout);
    println!("{stdout}");
    assert!(
        output.status.success(),
        "driver failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("spaces OK"), "the driver did not reach the end");
    state.project_app_close("p1").await.ok();
    std::fs::remove_dir_all(&root).ok();
}
