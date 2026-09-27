//! A server that declares a home App is a space of its own.
//!
//! The Apps fixture with `--home` marks its notes View as the server's home
//! App; declared in the MCP catalogue, the host lists it under `/api/spaces`
//! and opens it without a tool call. The same fixture without the marker is
//! an ordinary server: no space.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use swem_host::{DeclareMcpServerBody, WorkbenchShellState};

fn fresh_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-spaces-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn shell_with_fixture(root: &std::path::Path, name: &str, home: bool) -> Arc<WorkbenchShellState> {
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
    let mut args = vec![
        "--receipt".to_owned(),
        root.join(format!("{name}-receipt.json"))
            .display()
            .to_string(),
        "--poison".to_owned(),
        root.join(format!("{name}-poison.json"))
            .display()
            .to_string(),
    ];
    if home {
        args.push("--home".to_owned());
    }
    args.push("--late-tool".to_owned());
    args.push(root.join(format!("{name}-late-tool")).display().to_string());
    state
        .declare_mcp_server(&DeclareMcpServerBody {
            name: name.to_owned(),
            transport: "stdio".into(),
            command: env!("CARGO_BIN_EXE_swem-mcp-apps-fixture").to_owned(),
            args,
            env: Vec::new(),
            url: String::new(),
            headers: Vec::new(),
        })
        .expect("declare the fixture");
    Arc::new(state)
}

#[tokio::test]
async fn a_server_with_a_home_app_is_a_space_and_one_without_is_not() {
    let root = fresh_root("spaces");
    let state = shell_with_fixture(&root, "notes", true);
    let spaces = state.spaces().await.expect("the spaces list");
    assert_eq!(spaces.len(), 1, "{spaces:?}");
    assert_eq!(spaces[0].server, "notes");
    assert_eq!(spaces[0].uri, "ui://apps-fixture/notes");
    assert_eq!(
        spaces[0].description.as_deref(),
        Some("Note board MCP App view")
    );

    // Opening the space hands the page the App, as a project's App is handed.
    let opened = state
        .space_open("notes", "ui://apps-fixture/notes")
        .await
        .expect("the space opens");
    assert_eq!(opened.server_name, "notes");
    assert!(
        opened.html.contains("<html") || opened.html.contains("<!doctype"),
        "{}",
        &opened.html[..80.min(opened.html.len())]
    );
    let refused = state
        .space_open("notes", "ui://apps-fixture/nothing")
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("declares no home App"),
        "{refused}"
    );
    state.project_app_close(&opened.app_id).await.ok();

    // The same server without the marker is an ordinary server: no space.
    let plain = shell_with_fixture(&fresh_root("plain"), "notes", false);
    assert!(plain.spaces().await.unwrap().is_empty());
    let refused = plain
        .space_open("notes", "ui://apps-fixture/notes")
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("declares no home App"),
        "{refused}"
    );
}

/// A server may gain a tool while it runs - a hub installs a package - and
/// the gate is on what the server declares, not on when the host looked.
#[tokio::test]
async fn an_app_may_call_a_tool_its_server_declared_after_it_was_dialled() {
    let root = fresh_root("late-tool");
    let state = shell_with_fixture(&root, "notes", true);
    let opened = state
        .space_open("notes", "ui://apps-fixture/notes")
        .await
        .expect("the space opens");
    let call = |id: u64| {
        serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "late_note", "arguments": {}}
        })
    };
    // Not declared yet: asked of the server once more, and refused.
    let before = state
        .project_app_rpc(&opened.app_id, call(1))
        .await
        .expect("the relay answers");
    assert_eq!(before["error"]["code"], -32602, "{before}");
    assert!(
        before["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("late_note"),
        "{before}"
    );

    // The server declares it; the same call goes through.
    std::fs::write(root.join("notes-late-tool"), b"arrived").expect("the tool arrives");
    let after = state
        .project_app_rpc(&opened.app_id, call(2))
        .await
        .expect("the relay answers");
    assert!(after.get("error").is_none(), "{after}");
    assert_eq!(
        after["result"]["content"][0]["text"], "the late tool answered",
        "{after}"
    );
    state.project_app_close(&opened.app_id).await.ok();
}
