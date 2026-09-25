//! Manual debug harness for the Apps host: serves the exact browser-gate
//! fixture on a fixed port until the process is killed. Never asserts.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use swem_host::{
    AttachmentBinding, AttachmentTransport, IntegrationKind, LaunchCommand, PersonalAgentProfile,
    PersonalAgentProfileStore, ResolvedDirectAgentConnection, WorkbenchShellState,
    serve_workbench_http_with_apps,
};

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

#[tokio::test]
#[ignore = "manual debug server: set SWEM_APPS_DEBUG and kill the process to stop"]
#[allow(
    clippy::too_many_lines,
    reason = "one linear manual harness mirrors the browser-gate fixture setup"
)]
async fn serve_the_apps_shell_for_manual_debugging() {
    if std::env::var("SWEM_APPS_DEBUG").is_err() {
        eprintln!("skipped: set SWEM_APPS_DEBUG to run the debug server");
        return;
    }
    let root = std::env::temp_dir().join("swem-apps-debug");
    let _ = fs::remove_dir_all(&root);
    let inventory = root.join("inventory");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let bundle_dir = root.join("bundle");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    fs::create_dir_all(&bundle_dir).unwrap();
    let web = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("web/apps-host");
    let outfile = format!("--outfile={}", bundle_dir.join("apps-bridge.js").display());
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
        .unwrap();
    assert!(
        bundle.status.success(),
        "{}",
        String::from_utf8_lossy(&bundle.stderr)
    );
    let store = PersonalAgentProfileStore::open(&inventory).unwrap();
    for profile_id in ["apps-main", "apps-hostile"] {
        store
            .create(
                &PersonalAgentProfile::new(
                    profile_id,
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
                .unwrap(),
            )
            .unwrap();
    }
    let root_for_resolver = root.clone();
    let state = WorkbenchShellState::open(
        &inventory,
        &root.join("routes.sqlite3"),
        Duration::from_secs(30),
        move |profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            let hostile = profile.profile_id == "apps-hostile";
            let mut notes_args = vec![
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
            ];
            if hostile {
                notes_args.push("--hostile".to_owned());
            }
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
                    .args(notes_args),
                )],
            })
        },
    )
    .unwrap();
    let handle = serve_workbench_http_with_apps(
        Arc::new(state),
        ([127, 0, 0, 1], 8788).into(),
        Some(bundle_dir),
    )
    .await
    .unwrap();
    eprintln!(
        "apps debug shell: http://127.0.0.1:{}/ (sandbox http://127.0.0.1:{})",
        handle.local_addr.port(),
        handle.sandbox_addr.port()
    );
    std::future::pending::<()>().await;
}
