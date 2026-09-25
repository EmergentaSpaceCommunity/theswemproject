//! Browser gate for the generic Workbench shell: a REAL headless browser
//! drives the shell page with real Input events through the whole paper
//! acceptance list on the arbitrary echo fixture profile - late second
//! prompt, ordered stream visible, interactive permission allow with an
//! independent MCP receipt, active-turn cancel, disconnect + resume with
//! cursor continuity and no re-delivery of acknowledged events, and the
//! fail-closed unadvertised close. The oracle is the routing ledger and the
//! out-of-band receipt file, never the pixels; the driver reports only the
//! identities it observed.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use serde_json::Value;
use swem_host::{
    AttachmentBinding, AttachmentTransport, CredentialBindingRef, CredentialSourceRef,
    IntegrationKind, LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore, Readiness,
    ResolvedDirectAgentConnection, RoutingLedger, WorkbenchAgentOption, WorkbenchShellState,
    discover_agents, serve_workbench_http,
};

/// The credential a live profile binds for its agent: the variable the host
/// process holds (from the gitignored `.env.local`) under the name the agent
/// reads. The durable profile stores names only.
fn live_credential_bindings(agent_id: &str) -> Vec<CredentialBindingRef> {
    let name = match agent_id {
        "claude-code" => "CLAUDE_CODE_OAUTH_TOKEN",
        "opencode" => "OPENCODE_CONFIG_CONTENT",
        _ => return Vec::new(),
    };
    vec![CredentialBindingRef {
        binding_id: format!("{agent_id}-credential"),
        source: CredentialSourceRef::EnvironmentVariable { name: name.into() },
        target_environment: name.into(),
    }]
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome and Node.js"]
#[allow(
    clippy::too_many_lines,
    reason = "one browser transaction keeps first-run interaction and its external ledger/profile oracles together"
)]
async fn a_real_browser_completes_product_first_run_and_resumes_after_reload() {
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(
        node_available(),
        "put Node.js on PATH before running this ignored test"
    );
    let root = std::env::temp_dir().join(format!(
        "swem-product-browser-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let state =
        WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(30), |profile| {
            if profile.agent_id != "swem-echo-agent" {
                return Err(format!("unexpected agent {}", profile.agent_id));
            }
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: Vec::new(),
            })
        })
        .expect("open empty product state");
    state
        .enable_local_onboarding(
            vec![WorkbenchAgentOption {
                agent_id: "swem-echo-agent".into(),
                name: "Fixture Agent".into(),
                readiness: Readiness::InstalledUnverified,
                available: true,
            }],
            &root.join("workspaces"),
            &root.join("agent-homes"),
        )
        .expect("enable first-run onboarding");
    let handle = serve_workbench_http(Arc::new(state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("bind product shell server");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());
    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workbench_shell_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new("node")
            .arg(&driver)
            .arg(&browser)
            .arg(&url)
            .arg("--onboarding")
            .arg("product-first-run-marker")
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
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver reported route"),
    )
    .expect("parse driver report");
    let route = report["main_route"].as_str().expect("product route");
    let artifact = &report["artifact_proof"];
    let expected_attachment = b"attachment-evidence";
    assert_eq!(artifact["full_status"], 200);
    assert_eq!(
        artifact["full_bytes"],
        serde_json::json!(expected_attachment)
    );
    assert_eq!(artifact["partial_status"], 206);
    assert_eq!(artifact["partial_range"], "bytes 0-9/19");
    assert_eq!(
        artifact["partial_bytes"],
        serde_json::json!(&expected_attachment[..10])
    );
    assert!(
        artifact["representation_digest"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha-256=:"))
    );
    let linked = &report["linked_artifact_proof"];
    let linked_bytes = b"\0SWEM_LINKED_OUTPUT\xff\x10\x80\x7f";
    assert_eq!(linked["full_status"], 200);
    assert_eq!(linked["full_bytes"], serde_json::json!(linked_bytes));
    assert_eq!(linked["unavailable_count"], 2);
    let events = kinds_of(&ledger, route);
    assert_eq!(
        events
            .iter()
            .filter(|(kind, _, _)| kind == "host/prompt_submitted")
            .count(),
        6
    );

    // The composer's five ACP resource fields, which no walk had typed into.
    // The ledger's submitted payload is what the agent was actually given.
    let fields = &report["resource_fields"];
    let said = |key: &str| fields[key].as_str().expect("driver field").to_owned();
    let submitted = events
        .iter()
        .filter(|(kind, _, _)| kind == "host/prompt_submitted")
        .map(|(_, _, payload)| payload.clone())
        .collect::<Vec<_>>();
    let blocks_of = |text: &str| -> Vec<Value> {
        submitted
            .iter()
            .find(|payload| payload.to_string().contains(text))
            .unwrap_or_else(|| panic!("no submitted turn carrying {text}: {submitted:?}"))["content"]
            .as_array()
            .expect("the turn's content")
            .clone()
    };
    let filled = blocks_of("SWEM_RESOURCE_FIELDS");
    let link = filled
        .iter()
        .find(|block| block["type"] == "resource_link")
        .unwrap_or_else(|| panic!("the resource link the person filled never left: {filled:?}"));
    assert_eq!(link["uri"], said("link_uri"));
    assert_eq!(link["name"], said("link_name"));
    assert_eq!(link["mimeType"], "text/plain");
    let embedded = filled
        .iter()
        .find(|block| block["type"] == "resource")
        .unwrap_or_else(|| panic!("the embedded resource never left: {filled:?}"));
    assert_eq!(embedded["resource"]["uri"], said("embed_uri"));
    // The body itself is journalled by digest, so what is compared is its
    // length: the whole of what the person typed, not a truncation.
    assert_eq!(
        embedded["resource"]["textDescriptor"]["byte_length"],
        Value::from(said("embed_body").len())
    );

    // And it stopped there. The five fields used to keep their values after a
    // send, so one resource link rode every later turn uninvited.
    let after = blocks_of("SWEM_AFTER_RESOURCE_FIELDS");
    assert!(
        after.iter().all(|block| block["type"] == "text"),
        "a resource the person sent once rode the next turn as well: {after:?}"
    );

    // A body typed with no URI to hang it on is answered, not swallowed.
    let orphan_answer = said("orphan_answer");
    assert!(
        orphan_answer.contains("URI"),
        "a body without its URI was not answered in words: {orphan_answer}"
    );
    assert!(
        submitted
            .iter()
            .all(|payload| !payload.to_string().contains(&said("orphan_body"))),
        "a body the product refused was sent anyway: {submitted:?}"
    );

    // Who wrote it. ACP carries no author, so a turn a person typed would be
    // anonymous unless the product says otherwise - and it says it twice: to
    // the agent, as a line appended to the turn, and to the lane, as an event
    // the agent cannot alter. Nothing had ever typed into that field, so
    // every turn of every walk before this one went out as "Someone".
    let writing_as = report["writing_as"]
        .as_str()
        .expect("the name the walk wrote under");
    let named: Vec<&Value> = events
        .iter()
        .filter(|(kind, _, _)| kind == "host/turn_written")
        .map(|(_, _, payload)| payload)
        .collect();
    let said_who = named
        .iter()
        .position(|payload| payload["correspondent"]["author"] == writing_as)
        .unwrap_or_else(|| panic!("the lane never names the person who wrote: {named:?}"));
    // Before the name was typed the lane says nobody said, and the agent was
    // told that same word rather than a guess at one.
    assert!(
        named[..said_who].iter().all(|payload| {
            payload["correspondent"]["author"].is_null()
                && payload["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("[Someone, writing from "))
        }),
        "a turn written before anyone said who they were came out named: {named:?}"
    );
    // And from the moment it was typed every turn carries it, the ones after
    // the reload included - which is the person not typing it again.
    assert!(
        named[said_who..]
            .iter()
            .all(|payload| payload["correspondent"]["author"] == writing_as),
        "the name stopped being carried partway through: {named:?}"
    );
    assert!(
        named.len() - said_who >= 2,
        "the name was not carried past the turn it was typed in: {named:?}"
    );
    // The lane is one half; this is the other. The agent is told inside the
    // turn, because a reader of the transcript must see what the agent saw.
    let provenance = format!("[{writing_as}, writing from ");
    let carried = events
        .iter()
        .filter(|(kind, _, payload)| {
            kind == "host/prompt_submitted" && payload.to_string().contains(&provenance)
        })
        .count();
    assert_eq!(
        carried,
        named.len() - said_who,
        "the agent was told who is writing on a different set of turns than the lane records"
    );

    assert!(events.iter().any(|(kind, _, payload)| {
        kind == "host/prompt_submitted"
            && payload.to_string().contains("textDescriptor")
            && !payload.to_string().contains("attachment-evidence")
    }));
    assert!(events.iter().any(|(kind, _, payload)| {
        kind == "host/artifact_available"
            && payload["descriptor_id"] == artifact["descriptor_id"]
            && payload["byte_length"] == expected_attachment.len()
            && payload["source"] == "agent_output"
    }));
    assert!(events.iter().any(|(kind, _, payload)| {
        kind == "host/artifact_available"
            && payload["descriptor_id"] == linked["descriptor_id"]
            && payload["name"] == "linked-output.bin"
            && payload["byte_length"] == linked_bytes.len()
    }));
    assert_eq!(
        events
            .iter()
            .filter(|(kind, _, _)| kind == "host/artifact_available")
            .count(),
        2,
        "progressive capture must not duplicate terminal turn content"
    );
    assert_eq!(
        events
            .iter()
            .filter(|(kind, _, _)| kind == "host/artifact_unavailable")
            .count(),
        2
    );
    assert!(
        events
            .iter()
            .any(|(kind, _, _)| kind == "acp/session_resume")
    );
    let profile = PersonalAgentProfileStore::open(&inventory)
        .expect("reopen inventory")
        .load("swem-echo-agent")
        .expect("load browser-created profile");
    assert!(
        profile.attachments.is_empty(),
        "Cycle/MCP was implicitly enabled"
    );
    assert!(profile.credential_bindings.is_empty());

    drop(handle);
    fs::remove_dir_all(root).expect("remove fixture root");
}

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

fn kinds_of(ledger: &Path, route_id: &str) -> Vec<(String, String, serde_json::Value)> {
    RoutingLedger::open(ledger)
        .expect("reopen routing ledger")
        .events_for_surface(route_id, "oracle", 5000)
        .expect("read the full ordered lane")
        .events
        .into_iter()
        .map(|event| (event.kind, event.event_id, event.payload))
        .collect()
}

#[tokio::test]
#[ignore = "live browser test: requires Edge/Chrome and Node.js"]
#[allow(
    clippy::too_many_lines,
    reason = "one browser acceptance scenario keeps the fixture profiles, the driver run and every ledger oracle together"
)]
async fn a_real_browser_runs_the_whole_shell_acceptance_on_the_fixture_profile() {
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(
        node_available(),
        "put Node.js on PATH before running this ignored test"
    );

    let root = std::env::temp_dir().join(format!(
        "swem-shell-browser-{}-{}",
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
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let receipt = root.join("mcp-receipt.json");

    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    for profile_id in ["echo-main", "echo-nocls"] {
        let profile = PersonalAgentProfile::new(
            profile_id,
            "swem-echo-agent",
            "echo-fixture-distribution",
            "direct-fixture-environment",
            "surface-permissions",
            &workspace,
            &agent_home,
            vec![AttachmentBinding::new(
                "echo-attachment",
                "echo",
                AttachmentTransport::Stdio,
            )],
            Vec::new(),
        )
        .expect("build fixture profile");
        store.create(&profile).expect("persist fixture profile");
    }
    let receipt_for_resolver = receipt.clone();
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_secs(30),
        move |profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            let mut args = Vec::new();
            if profile.profile_id == "echo-nocls" {
                args.push("--hide-close".to_owned());
            }
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args,
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![McpServer::Stdio(
                    McpServerStdio::new("echo", PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo")))
                        .args(vec![
                            "--receipt".into(),
                            receipt_for_resolver.display().to_string(),
                        ]),
                )],
            })
        },
    )
    .expect("open shell state");
    let handle = serve_workbench_http(Arc::new(state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("bind shell server");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());

    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workbench_shell_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new("node")
            .arg(&driver)
            .arg(&browser)
            .arg(&url)
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
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver reported its identities"),
    )
    .expect("parse driver report");
    let main_route = report["main_route"].as_str().expect("main route id");
    let nocls_route = report["nocls_route"].as_str().expect("nocls route id");
    assert_ne!(main_route, nocls_route);

    // Ledger oracle for the main route: binding identity, permission ordering,
    // a cancelled turn, the native resume, two distinct connection namespaces
    // and two terminal events (one per connection).
    let binding = RoutingLedger::open(&ledger)
        .expect("reopen ledger")
        .route(main_route)
        .expect("main route binding");
    assert_eq!(binding.agent_profile_id, "echo-main");
    assert_eq!(binding.agent_id, "swem-echo-agent");
    let events = kinds_of(&ledger, main_route);
    let position = |kind: &str| {
        events
            .iter()
            .position(|(event_kind, _, _)| event_kind == kind)
            .unwrap_or_else(|| panic!("missing {kind} on the durable lane"))
    };
    assert!(position("acp/session_request_permission") < position("host/permission_decision"));
    position("acp/session_resume");
    // Load is the other way back into a conversation: rather than reattaching
    // to a session the agent still holds, the agent is asked to hand it back.
    // The rail has offered it since the rail existed and no walk had pressed
    // it, so nothing said this product can do it.
    position("acp/session_load");
    assert!(
        events
            .iter()
            .any(|(kind, _, payload)| kind == "acp/prompt_response"
                && payload.to_string().contains("cancelled")),
        "no cancelled prompt response on the durable lane"
    );
    let namespaces: std::collections::BTreeSet<&str> = events
        .iter()
        .filter_map(|(_, event_id, _)| event_id.split(':').next())
        .collect();
    // Three connections, one lane: the one the conversation started on, the
    // one that resumed it, and the one that loaded it back.
    assert_eq!(
        namespaces.len(),
        3,
        "expected exactly three connection namespaces on the main route"
    );
    // One per connection that ended: the first, the resumed one, the loaded
    // one.
    assert_eq!(
        events
            .iter()
            .filter(|(kind, _, _)| kind == "host/session_terminal")
            .count(),
        3
    );

    // The browser surface acknowledged what it rendered; the cursor is real.
    let cursor = RoutingLedger::open(&ledger)
        .expect("reopen ledger")
        .surface_cursor(main_route, "workbench-browser")
        .expect("browser surface cursor");
    assert!(cursor > 0, "the browser surface never acknowledged");

    // Independent MCP effect: the receipt carries the browser-typed nonce.
    let receipt_value: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt).expect("read MCP receipt"))
            .expect("parse MCP receipt");
    assert_eq!(receipt_value["nonce"], "browser-nonce");

    // The close-hiding route: the refused close never reached the wire and the
    // connection ended only through the later disconnect.
    let nocls_events = kinds_of(&ledger, nocls_route);
    assert!(
        nocls_events
            .iter()
            .all(|(kind, _, _)| kind != "acp/session_close"),
        "a refused close must not reach the ACP wire"
    );
    assert!(
        nocls_events
            .iter()
            .any(|(kind, _, _)| kind == "host/session_disconnect")
    );
    assert!(
        nocls_events
            .iter()
            .any(|(kind, _, _)| kind == "host/session_terminal")
    );

    drop(handle);
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[ignore = "live browser + agent test: requires Edge/Chrome, Node.js, SWEM_NATIVE_AGENT=claude-code and agent credentials"]
#[allow(
    clippy::too_many_lines,
    reason = "one live scenario keeps the profile, the driver run and the ledger oracle together"
)]
async fn the_same_shell_serves_a_live_claude_profile_without_changing_a_line() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=claude-code before running this ignored test");
    assert_eq!(agent_id, "claude-code");
    let browser = browser_path()
        .expect("install Edge/Chrome or set SWEM_BROWSER before running this ignored test");
    assert!(node_available(), "put Node.js on PATH");

    let root = std::env::temp_dir().join(format!(
        "swem-shell-live-{}-{}",
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
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    store
        .create(
            &PersonalAgentProfile::new(
                "claude-main",
                agent_id.clone(),
                "direct-host-distribution",
                "direct-host-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                live_credential_bindings(&agent_id),
            )
            .expect("build live profile"),
        )
        .expect("persist live profile");
    let state = WorkbenchShellState::open(
        &inventory,
        &ledger,
        Duration::from_secs(120),
        move |profile| {
            let discovery = discover_agents()
                .into_iter()
                .find(|agent| agent.id == profile.agent_id)
                .ok_or_else(|| format!("unknown agent: {}", profile.agent_id))?;
            if discovery.readiness == Readiness::Absent {
                return Err(format!("agent is not installed: {}", profile.agent_id));
            }
            Ok(ResolvedDirectAgentConnection {
                launch: discovery
                    .launch
                    .clone()
                    .ok_or_else(|| "discovered agent has no launch command".to_owned())?,
                agent_executable: discovery
                    .executable_path
                    .clone()
                    .ok_or_else(|| "discovered agent has no executable path".to_owned())?,
                mcp_servers: Vec::new(),
            })
        },
    )
    .expect("open shell state");
    let handle = serve_workbench_http(Arc::new(state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("bind shell server");
    let url = format!("http://127.0.0.1:{}/", handle.local_addr.port());

    let marker = "SWEM_SHELL_LIVE_2026_08_29";
    let prompt = format!("Reply with exactly the text {marker} and nothing else.");
    let driver =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/workbench_shell_cdp_driver.mjs");
    let output = tokio::time::timeout(
        Duration::from_secs(300),
        tokio::process::Command::new("node")
            .arg(&driver)
            .arg(&browser)
            .arg(&url)
            .arg("--simple")
            .arg(&prompt)
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
    let report: serde_json::Value = serde_json::from_str(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .expect("driver reported its identities"),
    )
    .expect("parse driver report");
    let route = report["main_route"].as_str().expect("live route id");

    let events = kinds_of(&ledger, route);
    assert!(
        events
            .iter()
            .any(|(kind, _, payload)| kind == "host/prompt_submitted"
                && payload.to_string().contains(marker))
    );
    assert!(
        events.iter().any(|(kind, _, payload)| matches!(
            kind.as_str(),
            "acp/session_update" | "acp/prompt_response"
        ) && payload.to_string().contains(marker)),
        "the live reply with the marker never reached the durable lane"
    );
    assert!(
        events
            .iter()
            .any(|(kind, _, _)| kind == "host/session_terminal")
    );
    let cursor = RoutingLedger::open(&ledger)
        .expect("reopen ledger")
        .surface_cursor(route, "workbench-browser")
        .expect("browser surface cursor");
    assert!(cursor > 0);

    drop(handle);
    fs::remove_dir_all(root).expect("remove fixture root");
}
