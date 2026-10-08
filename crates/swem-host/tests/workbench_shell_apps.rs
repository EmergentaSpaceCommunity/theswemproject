//! Component gate for the generic Apps host (H1, headless half): discovery
//! over real host-side stdio clients, the open-app payload, the Rust relay
//! gate (visibility, undeclared, cross-server, ui/*), the poison-receipt
//! absence, reconnect identity and the durable ledger decisions. No browser,
//! no npm, no Cycle.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use serde_json::{Value, json};
use swem_host::{
    AttachmentBinding, AttachmentTransport, IntegrationKind, LaunchCommand, PersonalAgentProfile,
    PersonalAgentProfileStore, ResolvedDirectAgentConnection, RoutingLedger, ShellConnectionMode,
    WorkbenchShellState, serve_workbench_http_with_apps,
};

const NOTES_URI: &str = "ui://apps-fixture/notes";

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-shell-apps-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

/// Shell over one profile with TWO attachments: the notes Apps fixture and
/// the plain echo server (the cross-server candidate).
fn shell_over_fixtures(
    root: &Path,
    hostile: bool,
    omit_resource_listing: bool,
) -> (Arc<WorkbenchShellState>, PathBuf) {
    shell_over_fixtures_with(root, hostile, omit_resource_listing, Vec::new())
}

fn shell_over_fixtures_with(
    root: &Path,
    hostile: bool,
    omit_resource_listing: bool,
    extra_notes_args: Vec<String>,
) -> (Arc<WorkbenchShellState>, PathBuf) {
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    let profile = PersonalAgentProfile::new(
        "apps-main",
        "swem-echo-agent",
        "echo-fixture-distribution",
        "direct-fixture-environment",
        "surface-permissions",
        &workspace,
        &agent_home,
        vec![
            AttachmentBinding::new("notes-attachment", "notes", AttachmentTransport::Stdio),
            AttachmentBinding::new("echo-attachment", "echo", AttachmentTransport::Stdio),
        ],
        Vec::new(),
    )
    .expect("build fixture profile");
    store.create(&profile).expect("persist fixture profile");
    let root_for_resolver = root.to_path_buf();
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_secs(20),
        move |_profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
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
            if omit_resource_listing {
                notes_args.push("--omit-resource-listing".to_owned());
            }
            notes_args.extend(extra_notes_args.iter().cloned());
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![
                    McpServer::Stdio(
                        McpServerStdio::new(
                            "notes",
                            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                        )
                        .args(notes_args),
                    ),
                    McpServer::Stdio(
                        McpServerStdio::new(
                            "echo",
                            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo")),
                        )
                        .args(vec![
                            "--receipt".into(),
                            root_for_resolver
                                .join("echo-receipt.json")
                                .display()
                                .to_string(),
                        ]),
                    ),
                ],
            })
        },
    )
    .expect("open shell state");
    (Arc::new(state), ledger)
}

/// One raw HTTP/1.1 GET against an address; returns (status line, headers, body).
async fn raw_get(address: std::net::SocketAddr, path: &str) -> (String, String, String) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: sandbox\r\nConnection: close\r\n\r\n").as_bytes(),
        )
        .await
        .expect("send request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read response");
    let (head, body) = response.split_once("\r\n\r\n").expect("headers");
    let (status, headers) = head.split_once("\r\n").unwrap_or((head, ""));
    (
        status.to_owned(),
        headers.to_ascii_lowercase(),
        body.to_owned(),
    )
}

/// One raw HTTP request on the sandbox origin with a body, as a fetch from
/// the View would send it; answers status, lowercased headers and body.
async fn raw_request(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &[u8],
) -> (String, String, Vec<u8>) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: sandbox\r\nOrigin: null\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.expect("send head");
    stream.write_all(body).await.expect("send body");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("headers");
    let head = String::from_utf8_lossy(&response[..split]).into_owned();
    let (status, headers) = head.split_once("\r\n").unwrap_or((head.as_str(), ""));
    (
        status.to_owned(),
        headers.to_ascii_lowercase(),
        response[split + 4..].to_vec(),
    )
}

/// The twin of the blob route: bytes INTO an open App's server workspace.
/// An upload lands under a host-chosen name in `uploads/`, is answered as a
/// workspace-relative path the App can name to the server's own ingest, is
/// preflighted for the opaque-origin View, is removable by that path only,
/// and reaches nothing for an unknown App or connection.
#[tokio::test]
async fn the_sandbox_origin_takes_uploads_into_the_open_apps_workspace() {
    let root = fixture_root("uploads");
    let receipt = root.join("probe-receipt.json");
    let (state, _ledger) = shell_over_fixtures_with(
        &root,
        false,
        false,
        vec![
            "--engine-probe".to_owned(),
            "--probe-receipt".to_owned(),
            receipt.display().to_string(),
        ],
    );
    let bundle = root.join("bundle");
    fs::create_dir_all(&bundle).expect("create empty Apps bundle directory");
    let server = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle),
    )
    .await
    .expect("enable the Apps surface");
    let (connection, _route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open connection");
    let opened = state
        .app_open(&connection, "notes", "ui://apps-fixture/engine-probe")
        .await
        .expect("open the probe App");
    // The App's declared CSP block admits its own origin for fetches.
    assert!(opened.csp.contains("connect-src 'self'"), "{}", opened.csp);
    let path = format!("/apps/{connection}/{}/upload", opened.app_id);

    let (status, headers, _) = raw_request(server.sandbox_addr, "OPTIONS", &path, b"").await;
    assert!(status.contains("204"), "{status}");
    assert!(
        headers.contains("access-control-allow-origin: *"),
        "{headers}"
    );
    assert!(
        headers.contains("access-control-allow-methods: post, delete, options"),
        "{headers}"
    );

    let payload: Vec<u8> = (0..100_000u32).map(|index| (index % 251) as u8).collect();
    let (status, headers, body) = raw_request(server.sandbox_addr, "POST", &path, &payload).await;
    assert!(
        status.contains("200"),
        "{status} {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        headers.contains("access-control-allow-origin: *"),
        "{headers}"
    );
    let answer: Value = serde_json::from_slice(&body).expect("json answer");
    let named = answer["workspace_path"].as_str().expect("a workspace path");
    assert!(named.starts_with("uploads/u-"), "{named}");
    assert_eq!(answer["byte_length"], 100_000);
    let landed = root.join("workspace").join(named);
    assert_eq!(fs::read(&landed).expect("the upload landed"), payload);

    // Unknown App, unknown connection: nothing lands.
    for refused in [
        path.replace(&opened.app_id, "a999"),
        path.replace(&connection, "nope"),
    ] {
        let (status, _, _) = raw_request(server.sandbox_addr, "POST", &refused, b"x").await;
        assert!(status.contains("404"), "{refused}: {status}");
    }
    // GET is not an upload.
    let (status, _, _) = raw_get(server.sandbox_addr, &path).await;
    assert!(status.contains("404"), "{status}");

    // Removal only by the answered path; a path outside `uploads/` is refused.
    let (status, _, _) = raw_request(
        server.sandbox_addr,
        "DELETE",
        &format!("{path}?path=uploads%2F..%2Fnotes-receipt.json"),
        b"",
    )
    .await;
    assert!(status.contains("404"), "{status}");
    let encoded = named.replace('/', "%2F");
    let (status, _, _) = raw_request(
        server.sandbox_addr,
        "DELETE",
        &format!("{path}?path={encoded}"),
        b"",
    )
    .await;
    assert!(status.contains("204"), "{status}");
    assert!(!landed.exists(), "the upload was removed by its own path");

    server.shutdown().await;
    let _ = state.disconnect(&connection).await;
    fs::remove_dir_all(&root).ok();
}

/// The affordances the host grants by declared metadata (0.73): every App
/// CSP carries `'wasm-unsafe-eval'`; a script resource the App's server
/// LISTS with the script MIME is served on the sandbox origin, addressed by
/// the connection and App handles, with the CORS grant an opaque-origin View
/// needs; nothing else of the server is reachable that way; and the App's
/// declared permissions ride in the open payload.
#[tokio::test]
async fn the_sandbox_origin_serves_only_listed_script_resources_of_an_open_app() {
    let root = fixture_root("script-resources");
    let receipt = root.join("probe-receipt.json");
    let (state, _ledger) = shell_over_fixtures_with(
        &root,
        false,
        false,
        vec![
            "--engine-probe".to_owned(),
            "--probe-receipt".to_owned(),
            receipt.display().to_string(),
        ],
    );
    let bundle = root.join("bundle");
    fs::create_dir_all(&bundle).expect("create empty Apps bundle directory");
    let server = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle),
    )
    .await
    .expect("enable the Apps surface");
    let (connection, _route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open connection");
    let probe_uri = "ui://apps-fixture/engine-probe";
    let worklet_uri = "ui://apps-fixture/probe-worklet.js";
    let opened = state
        .app_open(&connection, "notes", probe_uri)
        .await
        .expect("open the probe App");
    assert_eq!(opened.connection_id, connection);
    assert!(
        opened
            .csp
            .contains("script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'"),
        "{}",
        opened.csp
    );
    assert_eq!(opened.permissions, json!({"microphone": {}}));

    // The listed script resource, through the state and through the origin.
    let script = state
        .app_script_resource(&connection, &opened.app_id, worklet_uri)
        .await
        .expect("listed script resource");
    assert!(script.contains("registerProcessor('swem-probe-served'"));
    let path = format!(
        "/apps/{connection}/{}/resources?uri={}",
        opened.app_id,
        percent_encoding::utf8_percent_encode(worklet_uri, percent_encoding::NON_ALPHANUMERIC)
    );
    let (status, headers, body) = raw_get(server.sandbox_addr, &path).await;
    assert!(status.contains("200"), "{status}");
    assert!(
        headers.contains("content-type: text/javascript"),
        "{headers}"
    );
    assert!(
        headers.contains("access-control-allow-origin: *"),
        "{headers}"
    );
    assert!(headers.contains("cache-control: no-store"), "{headers}");
    assert_eq!(body, script);

    // Not a script resource (the App document itself), an unknown App, an
    // unknown connection, a missing uri: all 404 on the origin, no bytes.
    let app_path = format!(
        "/apps/{connection}/{}/resources?uri={}",
        opened.app_id,
        percent_encoding::utf8_percent_encode(probe_uri, percent_encoding::NON_ALPHANUMERIC)
    );
    for refused in [
        app_path,
        path.replace(&opened.app_id, "a999"),
        path.replace(&connection, "nope"),
        format!("/apps/{connection}/{}/resources", opened.app_id),
    ] {
        let (status, _, _) = raw_get(server.sandbox_addr, &refused).await;
        assert!(status.contains("404"), "{refused}: {status}");
    }
    assert!(
        state
            .app_script_resource(&connection, &opened.app_id, probe_uri)
            .await
            .is_err(),
        "an App document is not a script resource"
    );
    let (status, _, _) = raw_get(server.sandbox_addr, "/api/profiles").await;
    assert!(status.contains("404"), "no API on the sandbox origin");

    server.shutdown().await;
    let _ = state.disconnect(&connection).await;
    fs::remove_dir_all(&root).ok();
}

fn app_events(ledger: &Path, route_id: &str) -> Vec<(String, Value)> {
    RoutingLedger::open(ledger)
        .expect("reopen ledger")
        .events_for_surface(route_id, "oracle", 5000)
        .expect("read the ordered lane")
        .events
        .into_iter()
        .filter(|event| event.kind.starts_with("host/app_"))
        .map(|event| (event.kind, event.payload))
        .collect()
}

fn fixture_prompt(value: &Value) -> Vec<agent_client_protocol::schema::v1::ContentBlock> {
    vec![agent_client_protocol::schema::v1::ContentBlock::Text(
        agent_client_protocol::schema::v1::TextContent::new(value.to_string()),
    )]
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one transaction proves native ownership, exact projection, idempotent open, redaction and teardown"
)]
async fn native_agent_call_is_projected_to_the_declared_app_without_owning_the_call() {
    let root = fixture_root("agent-initiated");
    let bundle = root.join("bundle");
    fs::create_dir_all(&bundle).expect("create empty Apps bundle directory");
    let (state, ledger) = shell_over_fixtures(&root, false, false);
    state.set_mcp_observer_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        Vec::new(),
    );
    let server = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle),
    )
    .await
    .expect("enable the Apps surface");

    let (connection, route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open observed native connection");
    let nonce = "agent-initiated-nonce-0.55";
    let text = "the native agent still owns this MCP call";
    let turn = state
        .submit_prompt(
            &connection,
            vec![agent_client_protocol::schema::v1::ContentBlock::Text(
                agent_client_protocol::schema::v1::TextContent::new(
                    json!({
                        "fixture": "mcp-apps-note-v0.1",
                        "server": "notes",
                        "nonce": nonce,
                        "text": text,
                    })
                    .to_string(),
                ),
            )],
        )
        .await
        .expect("native agent calls its MCP attachment");
    assert!(turn.reply_text.contains(nonce));

    let observed = state
        .next_observed_app(&connection, 0, Duration::from_secs(5))
        .await
        .expect("query observed App call")
        .expect("agent call projected to an App");
    assert_eq!(observed.opened.server_name, "notes");
    assert_eq!(observed.opened.uri, NOTES_URI);
    assert_eq!(observed.observation.tool, "save_note");
    assert_eq!(observed.observation.arguments["nonce"], nonce);
    assert_eq!(observed.observation.arguments["text"], text);
    let repeated = state
        .next_observed_app(&connection, 0, Duration::ZERO)
        .await
        .expect("retry observed App call")
        .expect("same observation is still visible before cursor advance");
    assert_eq!(
        repeated.opened.app_id, observed.opened.app_id,
        "a retried long poll must not create a second App instance"
    );
    let terminal = state
        .observed_app_status(
            &connection,
            &observed.observation.observation_id,
            Duration::from_secs(5),
        )
        .await
        .expect("query terminal observed App call")
        .expect("observation remains connection-local");
    assert_eq!(terminal.status, "completed");
    assert_eq!(
        terminal.result.as_ref().expect("exact CallToolResult")["structuredContent"]["nonce"],
        nonce
    );

    let receipt: Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt.json")).expect("read agent-side receipt"),
    )
    .expect("parse agent-side receipt");
    assert_eq!(receipt["nonce"], nonce);
    assert_eq!(receipt["text"], text);

    state
        .app_close(&connection, &observed.opened.app_id)
        .await
        .expect("close projected App");
    let observation_id = observed.observation.observation_id.clone();
    state.disconnect(&connection).await.expect("disconnect");
    assert!(
        state
            .observed_app_status(&connection, &observation_id, Duration::ZERO)
            .await
            .is_err(),
        "disconnect must destroy connection-local raw observation state"
    );

    let descriptors = app_events(&ledger, &route)
        .into_iter()
        .filter(|(kind, _)| kind == "host/app_tool_observed")
        .collect::<Vec<_>>();
    assert_eq!(descriptors.len(), 2);
    let durable = serde_json::to_string(&descriptors).expect("serialize durable descriptors");
    assert!(durable.contains("arguments_digest"));
    assert!(durable.contains("result_digest"));
    assert!(!durable.contains(nonce));
    assert!(!durable.contains(text));

    drop(server);
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "three isolated fixture connections prove every terminal state and the App teardown race without conflating ownership"
)]
async fn observed_app_terminal_states_preserve_native_call_ownership() {
    let root = fixture_root("agent-terminal-states");
    let bundle = root.join("bundle");
    fs::create_dir_all(&bundle).expect("create empty Apps bundle directory");
    let (state, ledger) = shell_over_fixtures(&root, false, false);
    state.set_mcp_observer_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        Vec::new(),
    );
    let server = serve_workbench_http_with_apps(
        Arc::clone(&state),
        ([127, 0, 0, 1], 0).into(),
        Some(bundle),
    )
    .await
    .expect("enable the Apps surface");
    let (error_connection, error_route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open observed native connection");

    let error_nonce = "terminal-tool-error-0.57";
    state
        .submit_prompt(
            &error_connection,
            fixture_prompt(&json!({
                "fixture": "mcp-apps-error-v0.1",
                "server": "notes",
                "nonce": error_nonce,
                "text": "the server returns CallToolResult.isError",
            })),
        )
        .await
        .expect("native agent receives the tool execution error");
    let error_call = state
        .next_observed_app(&error_connection, 0, Duration::from_secs(5))
        .await
        .expect("query error observation")
        .expect("tool error was observed");
    let error_terminal = state
        .observed_app_status(
            &error_connection,
            &error_call.observation.observation_id,
            Duration::from_secs(5),
        )
        .await
        .expect("query tool error terminal")
        .expect("tool error observation retained");
    assert_eq!(error_terminal.status, "tool_error");
    assert_eq!(
        error_terminal
            .result
            .as_ref()
            .and_then(|result| result["isError"].as_bool()),
        Some(true),
        "a standards-shaped tool error remains an exact CallToolResult"
    );
    assert!(error_terminal.error.is_none());
    state
        .app_close(&error_connection, &error_call.opened.app_id)
        .await
        .expect("close error App");
    state
        .disconnect(&error_connection)
        .await
        .expect("disconnect error fixture");

    let (cancel_connection, cancel_route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open cancellation fixture connection");

    let cancel_nonce = "terminal-cancel-0.57";
    let cancelled_turn = state
        .submit_prompt(
            &cancel_connection,
            fixture_prompt(&json!({
                "fixture": "mcp-apps-cancel-v0.1",
                "server": "notes",
                "nonce": cancel_nonce,
                "text": "the native MCP client owns cancellation",
            })),
        )
        .await
        .expect("native agent completes its cancelled turn");
    assert_eq!(cancelled_turn.stop_reason, "cancelled");
    let cancelled_call = state
        .next_observed_app(&cancel_connection, 0, Duration::from_secs(5))
        .await
        .expect("query cancellation observation")
        .expect("cancellation was observed");
    let cancelled_terminal = state
        .observed_app_status(
            &cancel_connection,
            &cancelled_call.observation.observation_id,
            Duration::from_secs(5),
        )
        .await
        .expect("query cancelled terminal")
        .expect("cancelled observation retained");
    assert_eq!(cancelled_terminal.status, "cancelled");
    assert_eq!(
        cancelled_terminal.cancellation_reason.as_deref(),
        Some("native agent cancelled fixture call")
    );
    assert!(cancelled_terminal.result.is_none());
    assert!(cancelled_terminal.error.is_none());
    let started: Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt.json")).expect("read cancellation start receipt"),
    )
    .expect("parse cancellation start receipt");
    assert_eq!(started["state"], "started");
    state
        .app_close(&cancel_connection, &cancelled_call.opened.app_id)
        .await
        .expect("close cancelled App");
    state
        .disconnect(&cancel_connection)
        .await
        .expect("disconnect cancellation fixture");

    let (delay_connection, delay_route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open teardown-race fixture connection");

    let delayed_nonce = "terminal-teardown-race-0.57";
    let delayed_state = Arc::clone(&state);
    let delayed_connection = delay_connection.clone();
    let delayed_turn = tokio::spawn(async move {
        delayed_state
            .submit_prompt(
                &delayed_connection,
                fixture_prompt(&json!({
                    "fixture": "mcp-apps-delay-v0.1",
                    "server": "notes",
                    "nonce": delayed_nonce,
                    "text": "closing the View cannot cancel this call",
                    "delay_ms": 1_000,
                })),
            )
            .await
    });
    let delayed_call = state
        .next_observed_app(&delay_connection, 0, Duration::from_secs(5))
        .await
        .expect("query delayed observation")
        .expect("delayed call was observed before completion");
    assert_eq!(delayed_call.observation.status, "pending");
    state
        .app_close(&delay_connection, &delayed_call.opened.app_id)
        .await
        .expect("tear down the View while the native call is pending");
    let completed_turn = delayed_turn
        .await
        .expect("join delayed native turn")
        .expect("native call survives App teardown");
    assert_eq!(completed_turn.stop_reason, "end_turn");
    let delayed_terminal = state
        .observed_app_status(
            &delay_connection,
            &delayed_call.observation.observation_id,
            Duration::from_secs(5),
        )
        .await
        .expect("query delayed terminal")
        .expect("delayed observation retained after App teardown");
    assert_eq!(delayed_terminal.status, "completed");
    let completed: Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt.json")).expect("read delayed completion receipt"),
    )
    .expect("parse delayed completion receipt");
    assert_eq!(completed["tool"], "delayed_note");
    assert_eq!(completed["nonce"], delayed_nonce);

    state
        .disconnect(&delay_connection)
        .await
        .expect("disconnect teardown-race fixture");
    let descriptors = [error_route, cancel_route, delay_route]
        .into_iter()
        .flat_map(|route| app_events(&ledger, &route))
        .filter(|(kind, _)| kind == "host/app_tool_observed")
        .collect::<Vec<_>>();
    assert_eq!(descriptors.len(), 6, "three request/terminal pairs");
    let durable = serde_json::to_string(&descriptors).expect("serialize terminal descriptors");
    for raw in [
        error_nonce,
        cancel_nonce,
        delayed_nonce,
        "native agent cancelled fixture call",
    ] {
        assert!(
            !durable.contains(raw),
            "raw observation leaked into the ledger: {raw}"
        );
    }
    assert!(durable.contains("tool_error"));
    assert!(durable.contains("cancelled"));
    assert!(durable.contains("completed"));

    drop(server);
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one linear gate transaction keeps discovery, open, allowed and every refusal over one ledger visible"
)]
async fn discovery_open_and_the_relay_gate_are_host_side_decisions() {
    let root = fixture_root("gate");
    let (state, ledger) = shell_over_fixtures(&root, false, false);
    let receipt = root.join("notes-receipt.json");
    let poison = root.join("poison-receipt.json");

    let (connection, route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open shell connection");

    // Discovery is a live projection of both attachments.
    let attachments = state.apps_list(&connection).await.expect("discover apps");
    let notes = attachments
        .iter()
        .find(|view| view.server_name == "notes")
        .expect("notes attachment discovered");
    assert!(notes.transport_supported);
    assert_eq!(notes.connection_scope, "independent_host_connection");
    let save_note = notes
        .tools
        .iter()
        .find(|tool| tool.name == "save_note")
        .expect("save_note discovered");
    assert_eq!(save_note.resource_uri.as_deref(), Some(NOTES_URI));
    assert_eq!(save_note.visibility, vec!["model", "app"]);
    let probe = notes
        .tools
        .iter()
        .find(|tool| tool.name == "model_only_probe")
        .expect("probe discovered");
    assert_eq!(probe.visibility, vec!["model"]);
    assert_eq!(notes.apps.len(), 1);
    assert_eq!(notes.apps[0].uri, NOTES_URI);
    // The listing's title and where the App works are kept: the fixture
    // declares all three platforms on its notes App.
    assert_eq!(notes.apps[0].title.as_deref(), Some("Notes"));
    assert_eq!(notes.apps[0].platforms, ["web", "desktop", "mobile"]);
    assert!(notes.apps[0].works_on("mobile"));
    let echo = attachments
        .iter()
        .find(|view| view.server_name == "echo")
        .expect("echo attachment discovered");
    assert!(echo.apps.is_empty(), "echo declares no Apps");

    // Open resolves content-level metadata and the spec-default CSP.
    let opened = state
        .app_open(&connection, "notes", NOTES_URI)
        .await
        .expect("open the notes App");
    assert!(opened.html.contains("<!DOCTYPE html>"));
    assert!(opened.csp.contains("connect-src 'none'"));
    assert!(opened.prefers_border);
    assert_eq!(opened.permissions, json!({}));
    assert!(opened.sandbox_url.is_none(), "component use has no sandbox");

    // Allowed relay call: integer id echoed, receipt written, ledger allowed.
    let allowed = state
        .app_rpc(
            &connection,
            &opened.app_id,
            json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {
                "name": "save_note",
                "arguments": {"nonce": "gate-nonce", "text": "from the gate"},
            }}),
        )
        .await
        .expect("relay save_note");
    assert_eq!(allowed["id"], 7);
    assert_eq!(
        allowed["result"]["structuredContent"]["nonce"],
        "gate-nonce"
    );
    let written: Value =
        serde_json::from_slice(&fs::read(&receipt).expect("read receipt")).expect("parse receipt");
    assert_eq!(written["nonce"], "gate-nonce");

    // Refusals: model-only, undeclared, cross-server, ui/* - each a JSON-RPC
    // error with the id echoed, none reaching any server.
    for (id, method, params, code) in [
        (
            11,
            "tools/call",
            json!({"name": "model_only_probe", "arguments": {"nonce": "poison"}}),
            -32602,
        ),
        (12, "tools/call", json!({"name": "no_such_tool"}), -32602),
        (
            13,
            "tools/call",
            json!({"name": "echo", "arguments": {"nonce": "cross"}}),
            -32602,
        ),
        (
            14,
            "ui/open-link",
            json!({"url": "https://example.com"}),
            -32601,
        ),
    ] {
        let refused = state
            .app_rpc(
                &connection,
                &opened.app_id,
                json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
            )
            .await
            .expect("relay returns a JSON-RPC error, not a transport failure");
        assert_eq!(refused["id"], id);
        assert_eq!(refused["error"]["code"], code, "{method}");
    }
    assert!(!poison.exists(), "the model-only probe must not have run");
    let echo_receipt = root.join("echo-receipt.json");
    assert!(
        !echo_receipt.exists(),
        "the cross-server tool must not have run"
    );

    // A float id parses as a notification and is not forwarded.
    let notification = state
        .app_rpc(
            &connection,
            &opened.app_id,
            json!({"jsonrpc": "2.0", "id": 2.71, "method": "tools/call", "params": {
                "name": "save_note", "arguments": {"nonce": "float", "text": "x"},
            }}),
        )
        .await
        .expect("notification accepted");
    assert_eq!(notification, Value::Null);
    let unchanged: Value =
        serde_json::from_slice(&fs::read(&receipt).expect("read receipt")).expect("parse receipt");
    assert_eq!(
        unchanged["nonce"], "gate-nonce",
        "notification must not run"
    );

    // Close, then disconnect; the durable lane carries the decisions.
    state
        .app_close(&connection, &opened.app_id)
        .await
        .expect("close the App");
    state.disconnect(&connection).await.expect("disconnect");

    let events = app_events(&ledger, &route);
    assert_eq!(events[0].0, "host/app_opened");
    assert_eq!(events[0].1["uri"], NOTES_URI);
    let decisions: Vec<&str> = events
        .iter()
        .filter(|(kind, _)| kind == "host/app_tool_call")
        .map(|(_, payload)| payload["decision"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        decisions,
        vec!["allowed", "refused", "refused", "refused", "refused"]
    );
    assert_eq!(
        events.last().map(|(kind, _)| kind.as_str()),
        Some("host/app_closed")
    );

    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn reconnect_rediscovers_the_identical_app_identity() {
    let root = fixture_root("reconnect");
    let (state, ledger) = shell_over_fixtures(&root, false, false);

    let (connection, route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open shell connection");
    let opened = state
        .app_open(&connection, "notes", NOTES_URI)
        .await
        .expect("open the notes App");
    let first_identity = (opened.server_name.clone(), opened.uri.clone());
    state.disconnect(&connection).await.expect("disconnect");

    let (resumed, resumed_route, _session) = state
        .open_connection(
            "apps-main",
            ShellConnectionMode::Resume,
            Some(route.clone()),
        )
        .await
        .expect("resume the route");
    assert_eq!(resumed_route, route);
    let attachments = state.apps_list(&resumed).await.expect("re-discover apps");
    let notes = attachments
        .iter()
        .find(|view| view.server_name == "notes")
        .expect("notes attachment re-discovered");
    assert_eq!(notes.apps[0].uri, first_identity.1);
    let reopened = state
        .app_open(&resumed, "notes", NOTES_URI)
        .await
        .expect("re-open the notes App");
    assert_eq!(
        (reopened.server_name, reopened.uri),
        first_identity,
        "reconnect must preserve the resource identity"
    );
    state
        .disconnect(&resumed)
        .await
        .expect("disconnect resumed");

    let opened_events: Vec<Value> = app_events(&ledger, &route)
        .into_iter()
        .filter(|(kind, _)| kind == "host/app_opened")
        .map(|(_, payload)| payload)
        .collect();
    assert_eq!(opened_events.len(), 2);
    assert_eq!(opened_events[0]["server"], opened_events[1]["server"]);
    assert_eq!(opened_events[0]["uri"], opened_events[1]["uri"]);

    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn tool_linked_app_opens_when_resources_list_omits_it() {
    let root = fixture_root("tool-linked-only");
    let (state, _ledger) = shell_over_fixtures(&root, false, true);
    let (connection, _route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open shell connection");

    let attachments = state.apps_list(&connection).await.expect("discover apps");
    let notes = attachments
        .iter()
        .find(|view| view.server_name == "notes")
        .expect("notes attachment discovered from tool metadata");
    assert_eq!(notes.apps.len(), 1);
    assert_eq!(notes.apps[0].uri, NOTES_URI);
    assert!(
        notes.apps[0].description.is_none(),
        "an omitted listing must not fabricate resource metadata"
    );
    // Nor a place it works: unlisted, an App is for the web and the desktop.
    assert!(notes.apps[0].title.is_none());
    assert_eq!(notes.apps[0].platforms, ["web", "desktop"]);
    assert!(!notes.apps[0].works_on("mobile"));
    let opened = state
        .app_open(&connection, "notes", NOTES_URI)
        .await
        .expect("resources/read opens the tool-linked App");
    assert_eq!(opened.uri, NOTES_URI);

    state.disconnect(&connection).await.expect("disconnect");
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[test]
fn the_generic_host_hardcodes_no_server_id_or_domain_schema() {
    let host_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for relative in [
        "src/workbench_apps.rs",
        "src/workbench_shell.rs",
        "src/workbench_shell/server_apps.rs",
        "src/workbench_shell/model_context.rs",
        "src/workbench_shell/shell.html",
        "web/apps-host/src/main.tsx",
    ] {
        let source = fs::read_to_string(host_dir.join(relative)).expect("read host source");
        for needle in [
            "apps-fixture",
            "ui://swem",
            "save_note",
            "composition",
            "swem_domain_music",
        ] {
            assert!(!source.contains(needle), "{relative} hardcodes {needle}");
        }
    }
}

/// One App's long tool call is not the queue for its own next request.
///
/// A person presses a step that runs for seconds; while it runs the App must
/// still be able to read what the project now says about it. The relay takes
/// the apps lock to find the App's client, so the lock has to be down again
/// before the call is awaited. Here `delayed_note` is held open on a release
/// file that does not exist yet, and a `resources/read` of the same App has
/// to be answered while it waits.
#[tokio::test]
async fn a_long_tool_call_does_not_queue_the_same_app_s_next_request() {
    let root = fixture_root("relay-concurrency");
    let release = root.join("release.json");
    let (state, _ledger) = shell_over_fixtures_with(
        &root,
        false,
        false,
        vec!["--delay-release".to_owned(), release.display().to_string()],
    );
    let (connection, _route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open connection");
    let opened = state
        .app_open(&connection, "notes", NOTES_URI)
        .await
        .expect("open the notes App");

    let calling = tokio::spawn({
        let state = Arc::clone(&state);
        let connection = connection.clone();
        let app_id = opened.app_id.clone();
        async move {
            state
                .app_rpc(
                    &connection,
                    &app_id,
                    json!({"jsonrpc": "2.0", "id": 21, "method": "tools/call", "params": {
                        "name": "delayed_note",
                        "arguments": {
                            "nonce": "held-open",
                            "text": "the lock is down while this runs",
                            "delay_ms": 0,
                        },
                    }}),
                )
                .await
        }
    });
    // The fixture says when the call reached it, so the read below is timed
    // against a call that is certainly in flight rather than a guess.
    let waiting = release.with_extension("waiting");
    tokio::time::timeout(Duration::from_secs(20), async {
        while !waiting.is_file() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the delayed call reached the fixture");

    let read = tokio::time::timeout(
        Duration::from_secs(10),
        state.app_rpc(
            &connection,
            &opened.app_id,
            json!({"jsonrpc": "2.0", "id": 22, "method": "resources/read",
                "params": {"uri": NOTES_URI}}),
        ),
    )
    .await
    .expect("the App is answered while its own long call is still running")
    .expect("relay resources/read");
    assert_eq!(read["id"], 22);
    assert!(read.get("error").is_none(), "{read}");
    assert!(
        !release.is_file(),
        "the read must land before the long call is released"
    );

    fs::write(&release, b"go").expect("release the delayed note");
    let done = tokio::time::timeout(Duration::from_secs(20), calling)
        .await
        .expect("the released call finishes")
        .expect("join the delayed call")
        .expect("relay delayed_note");
    assert_eq!(done["id"], 21);
    assert_eq!(done["result"]["structuredContent"]["saved"], true, "{done}");
    let receipt: Value = serde_json::from_slice(
        &fs::read(root.join("notes-receipt.json")).expect("read the fixture receipt"),
    )
    .expect("parse the fixture receipt");
    assert_eq!(receipt["nonce"], "held-open");

    state.disconnect(&connection).await.expect("disconnect");
    fs::remove_dir_all(&root).ok();
}

/// A server's tool that asks before it answers (`input_required`, the
/// 2026-07-28 shape) reaches the page as a question and takes the answer
/// back. The host's client must negotiate that version for the server to
/// be allowed to ask at all: with the older handshake the library refuses
/// the first call.
#[tokio::test]
async fn a_tool_that_asks_first_is_asked_on_the_page_and_answered() {
    let root = fixture_root("asks");
    let (state, _ledger) = shell_over_fixtures(&root, false, false);
    let (connection, _route, _session) = state
        .open_connection("apps-main", ShellConnectionMode::New, None)
        .await
        .expect("open shell connection");

    let asked = state
        .start_app_interaction(
            &connection,
            "notes",
            "ask_title",
            rmcp::model::JsonObject::new(),
        )
        .await
        .expect("the server asks");
    assert_eq!(asked.message, "What is the note called?");
    assert_eq!(asked.server_name, "notes");
    assert_eq!(asked.requested_schema["required"], json!(["title"]));

    let answered = state
        .answer_app_interaction(
            &connection,
            &asked.interaction_id,
            rmcp::model::ElicitationAction::Accept,
            Some(json!({"title": "Rain"})),
        )
        .await
        .expect("the answer completes the call");
    let said = serde_json::to_value(&answered).expect("result as json");
    assert_eq!(said["content"][0]["text"], "the note is called Rain");
    assert_ne!(said["isError"], json!(true));

    // The interaction is spent: a second answer is refused, not replayed.
    let again = state
        .answer_app_interaction(
            &connection,
            &asked.interaction_id,
            rmcp::model::ElicitationAction::Accept,
            Some(json!({"title": "Snow"})),
        )
        .await;
    assert!(again.is_err());
}
