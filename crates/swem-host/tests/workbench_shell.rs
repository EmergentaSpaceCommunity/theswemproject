//! Component gate for the generic Workbench shell state: the interactive
//! driver and the durable route projector run together over one control, and
//! every shell operation is a projection of ledger reads or driver commands.
//! The fixture profile is the arbitrary echo agent - no Claude, no Cycle.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{ContentBlock, PermissionOptionKind, TextContent};
use swem_host::{
    AttachmentBinding, AttachmentTransport, CredentialBindingRef, CredentialSourceRef,
    IntegrationKind, LaunchCommand, NativeSessionTermination, PersonalAgentProfile,
    PersonalAgentProfileStore, ResolvedDirectAgentConnection, RoutingLedger, ShellConnectionMode,
    WorkbenchAgentOption, WorkbenchShellError, WorkbenchShellState, credential_environment,
};

/// A direct connection hands the agent exactly the profile's credential
/// bindings, read from this process under their declared target names, and
/// fails closed when a bound source is absent - the shell never launches an
/// agent that would silently run unauthenticated.
#[test]
fn direct_connections_carry_profile_credential_bindings_or_fail_closed() {
    let root = fixture_root("credentials");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    let binding = |id: &str, source: &str, target: &str| CredentialBindingRef {
        binding_id: id.into(),
        source: CredentialSourceRef::EnvironmentVariable {
            name: source.into(),
        },
        target_environment: target.into(),
    };
    let profile = |credentials: Vec<CredentialBindingRef>| {
        PersonalAgentProfile::new(
            "bound",
            "echo-fixture",
            "direct-host-distribution",
            "direct-host-environment",
            "surface-permissions",
            &workspace,
            &agent_home,
            Vec::new(),
            credentials,
        )
        .expect("profile with credential bindings")
    };
    // PATH exists in every host process; the agent sees it under the target.
    let carried = credential_environment(
        &profile(vec![binding("token", "PATH", "SWEM_TEST_AGENT_TOKEN")]),
        BTreeMap::new(),
    )
    .expect("bound source present");
    assert_eq!(
        carried.get("SWEM_TEST_AGENT_TOKEN"),
        std::env::var("PATH").ok().as_ref()
    );
    assert_eq!(carried.len(), 1);
    assert!(
        credential_environment(&profile(Vec::new()), BTreeMap::new())
            .unwrap()
            .is_empty()
    );
    // A secret the person gave the profile reaches the agent under its own
    // name, and a host-side binding for the same name is the operator's
    // decision and wins over it.
    let typed = credential_environment(
        &profile(vec![binding("token", "PATH", "SWEM_TEST_AGENT_TOKEN")]),
        BTreeMap::from([
            ("SWEM_TEST_AGENT_TOKEN".to_owned(), "typed".to_owned()),
            ("SWEM_TEST_OTHER_KEY".to_owned(), "kept".to_owned()),
        ]),
    )
    .expect("bound source present");
    assert_eq!(
        typed.get("SWEM_TEST_AGENT_TOKEN"),
        std::env::var("PATH").ok().as_ref()
    );
    assert_eq!(
        typed.get("SWEM_TEST_OTHER_KEY").map(String::as_str),
        Some("kept")
    );
    let missing = credential_environment(
        &profile(vec![binding(
            "token",
            "SWEM_TEST_SOURCE_THAT_DOES_NOT_EXIST_0_72",
            "SWEM_TEST_AGENT_TOKEN",
        )]),
        BTreeMap::new(),
    )
    .unwrap_err();
    assert!(
        matches!(missing, WorkbenchShellError::Failed(_)),
        "{missing}"
    );
    assert!(missing.to_string().contains("token"));
    fs::remove_dir_all(&root).ok();
}

/// A runner that fails before the agent process exists (here: the resolved
/// executable is not a file) must surface as an open error, never as a
/// connection that stays "connecting" forever.
#[tokio::test]
async fn an_unlaunchable_agent_fails_the_open_instead_of_hanging_it() {
    let root = fixture_root("unlaunchable");
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    PersonalAgentProfileStore::open(&inventory)
        .unwrap()
        .create(
            &PersonalAgentProfile::new(
                "ghost",
                "ghost-agent",
                "direct-host-distribution",
                "direct-host-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();
    let missing = root.join("missing-agent.exe");
    let state =
        WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(30), move |_| {
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: missing.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: missing.clone(),
                mcp_servers: Vec::new(),
            })
        })
        .unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        state.open_connection("ghost", ShellConnectionMode::New, None),
    )
    .await
    .expect("the open must return, not hang");
    let error = outcome.expect_err("an unlaunchable agent cannot open");
    assert!(
        error.to_string().contains("missing-agent"),
        "the failure names the executable: {error}"
    );
    assert!(state.active_connections().await.is_empty());
    fs::remove_dir_all(&root).ok();
}

/// An agent that refuses the handshake must say so through the Workbench.
///
/// A walk once failed with an agent complaining that the host
/// advertised no form capability, and the Workbench answered with a sentence
/// that named neither the complaint nor the claim. The handshake events were
/// published, but nothing had started reading the lane yet, so they went out
/// with the receiver. This opens a connection to an agent that says no and
/// reads the sentence a person gets.
#[tokio::test]
async fn a_refused_handshake_reaches_the_person_with_the_agents_reason() {
    let root = fixture_root("refused-handshake");
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&agent_home).unwrap();
    PersonalAgentProfileStore::open(&inventory)
        .unwrap()
        .create(
            &PersonalAgentProfile::new(
                "refuser",
                "swem-echo-agent",
                "direct-host-distribution",
                "direct-host-environment",
                "surface-permissions",
                &workspace,
                &agent_home,
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();
    let state =
        WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(30), move |_| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: vec![
                        "--refuse-initialize".into(),
                        "fixture client did not advertise initialize form elicitation".into(),
                    ],
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: Vec::new(),
            })
        })
        .unwrap();
    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        state.open_connection("refuser", ShellConnectionMode::New, None),
    )
    .await
    .expect("the open must return, not hang");
    let error = outcome
        .expect_err("an agent that refuses the handshake cannot open")
        .to_string();
    assert!(
        error.contains("did not advertise initialize form elicitation"),
        "the agent's own reason must survive to the person: {error}"
    );
    // The half only this host can testify to. The Workbench always advertises
    // form elicitation here, so an agent claiming otherwise is contradicted on
    // the same line rather than in a later investigation.
    assert!(
        error.contains("this host advertised") && error.contains("form"),
        "the failure must also say what was advertised: {error}"
    );
    assert!(state.active_connections().await.is_empty());
    fs::remove_dir_all(&root).ok();
}

/// The whole shell path with a bound credential: a profile whose binding
/// reads a host variable opens a real connection on the direct environment
/// (the echo fixture agent), and the same profile fails closed before any
/// launch when the bound source is missing.
#[tokio::test]
async fn a_direct_connection_opens_with_a_bound_credential_and_fails_closed_without_it() {
    let root = fixture_root("bound-connection");
    let (state, _ledger) = shell_over_fixtures(&root);
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    let binding = |source: &str| CredentialBindingRef {
        binding_id: "fixture-token".into(),
        source: CredentialSourceRef::EnvironmentVariable {
            name: source.into(),
        },
        target_environment: "SWEM_FIXTURE_TOKEN".into(),
    };
    for (profile_id, source) in [
        ("echo-bound", "PATH"),
        ("echo-unbound", "SWEM_TEST_SOURCE_THAT_DOES_NOT_EXIST_0_72"),
    ] {
        let profile = PersonalAgentProfile::new(
            profile_id,
            "echo-fixture",
            "direct-host-distribution",
            "direct-host-environment",
            "surface-permissions",
            &workspace,
            &agent_home,
            Vec::new(),
            vec![binding(source)],
        )
        .expect("profile with a credential binding");
        PersonalAgentProfileStore::open(&root.join("inventory"))
            .expect("open inventory")
            .create(&profile)
            .expect("persist bound profile");
    }
    let opened = tokio::time::timeout(
        Duration::from_mins(1),
        state.open_connection("echo-bound", ShellConnectionMode::New, None),
    )
    .await
    .expect("bound connection opened in time")
    .expect("bound profile opens on the direct environment");
    state
        .disconnect(&opened.0)
        .await
        .expect("disconnect bound connection");
    let refused = state
        .open_connection("echo-unbound", ShellConnectionMode::New, None)
        .await
        .unwrap_err();
    assert!(
        matches!(refused, WorkbenchShellError::Failed(_)),
        "{refused}"
    );
    assert!(refused.to_string().contains("fixture-token"), "{refused}");
    fs::remove_dir_all(&root).ok();
}
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-shell-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create shell fixture root");
    root
}

fn text(value: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text(TextContent::new(value))]
}

/// Persist the two fixture profiles (`echo-main`, `echo-nocls`) and build the
/// shell state whose resolver maps them onto the echo fixture binaries.
fn shell_over_fixtures(root: &std::path::Path) -> (Arc<WorkbenchShellState>, PathBuf) {
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let workspace = root.join("workspace");
    let agent_home = root.join("agent-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    let receipt = root.join("mcp-receipt.json");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    // `echo-plain` is an agent that advertises no rich content at all: the
    // ordinary case for a coding agent, and the one where a person attaching
    // a file used to hang the turn.
    for profile_id in ["echo-main", "echo-nocls", "echo-alone", "echo-plain"] {
        let profile = PersonalAgentProfile::new(
            profile_id,
            "swem-echo-agent",
            "echo-fixture-distribution",
            "direct-fixture-environment",
            // `echo-alone` is the person who left their agent to work by
            // itself; every other fixture profile is somebody watching.
            if profile_id == "echo-alone" {
                swem_host::INSIDE_ITS_WORKSPACE
            } else {
                swem_host::ASK_EVERY_TIME
            },
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
        Duration::from_secs(20),
        move |profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            let mut args = Vec::new();
            if profile.profile_id == "echo-nocls" {
                args.push("--hide-close".to_owned());
            }
            if profile.profile_id == "echo-plain" {
                args.push("--baseline-content-only".to_owned());
            }
            let mcp = agent_client_protocol::schema::v1::McpServer::Stdio(
                agent_client_protocol::schema::v1::McpServerStdio::new(
                    "echo",
                    PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo")),
                )
                .args(vec![
                    "--receipt".into(),
                    receipt_for_resolver.display().to_string(),
                ]),
            );
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args,
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![mcp],
            })
        },
    )
    .expect("open shell state");
    (Arc::new(state), ledger)
}

fn surface_kinds(ledger: &std::path::Path, route_id: &str) -> Vec<String> {
    RoutingLedger::open(ledger)
        .expect("reopen ledger")
        .events_for_surface(route_id, "oracle", 1000)
        .expect("read ordered events")
        .events
        .into_iter()
        .map(|event| event.kind)
        .collect()
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one turn of probing, and the two kinds of refusal it earns, read in order"
)]
async fn callback_refusals_reach_the_route_ledger_without_a_transcript() {
    let root = fixture_root("callback-ledger");
    let workspace = root.join("workspace");
    let home = root.join("agent-home");
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&home).unwrap();
    fs::write(workspace.join("sentinel.txt"), b"private fixture content").unwrap();
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    PersonalAgentProfileStore::open(&inventory)
        .unwrap()
        .create(
            &PersonalAgentProfile::new(
                "callback",
                "callback-fixture",
                "fixture",
                "direct-fixture",
                "surface-permissions",
                &workspace,
                &home,
                Vec::new(),
                Vec::new(),
            )
            .unwrap(),
        )
        .unwrap();
    let state = WorkbenchShellState::open(&inventory, &ledger, Duration::from_secs(15), |_| {
        let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-callback-agent"));
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
    .unwrap();
    let state = Arc::new(state);
    let (connection, route, _) = state
        .open_connection("callback", ShellConnectionMode::New, None)
        .await
        .unwrap();
    // Somebody in the person's chair. This profile asks before a command
    // runs, and the probe asks for one: without an answer the turn would sit
    // on its own deadline, which is what a person's closed tab does to it.
    let answering = tokio::spawn({
        let state = Arc::clone(&state);
        let connection = connection.clone();
        async move {
            let question = state
                .next_permission(&connection, Duration::from_secs(10))
                .await
                .expect("read the question")
                .expect("a command is put to the person");
            assert_eq!(
                question.provenance,
                swem_host::NativePermissionProvenance::HostCallback
            );
            state
                .select_permission(&connection, question.sequence, "do-not-run")
                .await
                .expect("answer it");
            question.tool_call.fields.title.clone().unwrap_or_default()
        }
    });
    state
        .submit_prompt(&connection, text("probe callback authority"))
        .await
        .unwrap();
    let asked = answering.await.expect("the person answered");
    assert!(
        asked.starts_with("Run cmd /C echo callback"),
        "the question does not say what would run: {asked}"
    );
    let outcome = state.disconnect(&connection).await.unwrap();
    assert!(outcome.transcript_path.is_none());
    drop(state);
    let events = RoutingLedger::open(&ledger)
        .unwrap()
        .events_for_surface(&route, "fresh-reader", 1000)
        .unwrap()
        .events;
    // Files are what this connection never advertised: it opened without a
    // surface to carry them, so both are refused as methods the agent may
    // not call.
    let refused: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "host/callback_refused")
        .collect();
    let expected = ["fs/read_text_file", "fs/write_text_file"];
    assert_eq!(refused.len(), expected.len());
    for (event, method) in refused.iter().zip(expected) {
        assert_eq!(event.source, swem_host::SurfaceEventSource::Host);
        assert_eq!(event.payload, serde_json::json!({"method": method}));
    }
    // Terminals the host carries itself, so they are not refused as a method
    // - the person was asked about the command and said no, and the four
    // that follow name a terminal that is not this connection's. Nothing
    // started, and the record says which of the two reasons each one was.
    let terminals: Vec<_> = events
        .iter()
        .filter(|event| event.kind == "host/terminal_callback")
        .collect();
    let attempted = [
        ("terminal/create", "not allowed"),
        ("terminal/output", "refused"),
        ("terminal/wait_for_exit", "refused"),
        ("terminal/kill", "refused"),
        ("terminal/release", "refused"),
    ];
    assert_eq!(terminals.len(), attempted.len());
    for (event, (method, outcome)) in terminals.iter().zip(attempted) {
        assert_eq!(event.source, swem_host::SurfaceEventSource::Host);
        assert_eq!(event.payload["method"], serde_json::json!(method));
        assert_eq!(event.payload["outcome"], serde_json::json!(outcome));
    }
    assert_eq!(
        fs::read(workspace.join("sentinel.txt")).unwrap(),
        b"private fixture content"
    );
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one walk of local onboarding, in the order a person meets it"
)]
fn local_onboarding_is_explicit_create_only_and_cycle_free() {
    let root = fixture_root("onboarding");
    let inventory = root.join("inventory");
    let state = WorkbenchShellState::open(
        &inventory,
        &root.join("routes.sqlite3"),
        Duration::from_secs(5),
        |_| Err("resolver must not run during onboarding".into()),
    )
    .expect("open empty shell state");
    let escaped = state.enable_local_onboarding(
        vec![WorkbenchAgentOption {
            agent_id: "../escape".into(),
            name: "Unsafe".into(),
            readiness: swem_host::Readiness::InstalledUnverified,
            available: true,
        }],
        &root.join("workspaces"),
        &root.join("agent-homes"),
    );
    assert!(matches!(escaped, Err(WorkbenchShellError::Invalid(_))));
    assert!(
        !root.join("workspaces").exists(),
        "invalid id mutated the filesystem"
    );
    state
        .enable_local_onboarding(
            vec![
                WorkbenchAgentOption {
                    agent_id: "echo-ready".into(),
                    name: "Echo Ready".into(),
                    readiness: swem_host::Readiness::InstalledUnverified,
                    available: true,
                },
                WorkbenchAgentOption {
                    agent_id: "echo-absent".into(),
                    name: "Echo Absent".into(),
                    readiness: swem_host::Readiness::Absent,
                    available: false,
                },
            ],
            &root.join("workspaces"),
            &root.join("agent-homes"),
        )
        .expect("enable local onboarding");

    let view = state.onboarding();
    assert!(view.enabled);
    assert_eq!(view.agents.len(), 2);
    assert!(matches!(
        state.create_local_profile("echo-absent", None, None),
        Err(WorkbenchShellError::Conflict(_))
    ));
    assert!(matches!(
        state.create_local_profile("invented", None, None),
        Err(WorkbenchShellError::NotFound(_))
    ));

    let profile = state
        .create_local_profile("echo-ready", None, None)
        .expect("create selected local profile");
    assert_eq!(profile.profile_id, "echo-ready");
    assert_eq!(profile.agent_id, "echo-ready");
    // A created profile names an environment this host actually offers, and
    // the id is the one every profile written before the catalogue existed
    // already carries, so nothing has to be migrated.
    assert_eq!(profile.environment_profile_id, swem_host::THIS_MACHINE);
    assert!(
        swem_host::environment_profiles()
            .iter()
            .any(|place| place.environment_profile_id == profile.environment_profile_id),
        "a new profile must run somewhere this host offers"
    );
    assert!(
        profile.attachments.is_empty(),
        "Cycle/MCP was attached implicitly"
    );
    assert!(
        profile.credential_bindings.is_empty(),
        "credentials leaked into profile"
    );
    assert!(
        profile.workspace.starts_with(
            fs::canonicalize(root.join("workspaces")).expect("canonical workspace root")
        )
    );
    assert!(
        profile
            .agent_home
            .starts_with(fs::canonicalize(root.join("agent-homes")).expect("canonical home root"))
    );
    assert!(matches!(
        state.create_local_profile("echo-ready", None, None),
        Err(WorkbenchShellError::Conflict(_))
    ));
    assert_eq!(
        state.profiles().expect("list profiles"),
        vec![profile.clone()]
    );

    // One installed agent, two people. A named profile of an agent that
    // already has one is not a conflict: it is a second person, and what
    // separates them is a workspace and a native home of their own.
    let second = state
        .create_local_profile("echo-ready", Some("ada"), None)
        .expect("a second profile of the same agent");
    assert_eq!(second.profile_id, "ada");
    assert_eq!(second.agent_id, "echo-ready");
    assert_ne!(second.workspace, profile.workspace);
    assert_ne!(second.agent_home, profile.agent_home);
    assert!(second.workspace.ends_with("ada"));
    assert!(second.agent_home.ends_with("ada"));
    assert!(matches!(
        state.create_local_profile("echo-ready", Some("ada"), None),
        Err(WorkbenchShellError::Conflict(_))
    ));
    // A name is an identity, not a path: one that would leave the roots is
    // refused before a directory is made for it.
    for escape in ["../elsewhere", "", "a/b"] {
        assert!(
            matches!(
                state.create_local_profile("echo-ready", Some(escape), None),
                Err(WorkbenchShellError::Invalid(_))
            ),
            "a profile named {escape:?} was accepted"
        );
    }
    assert_eq!(
        state
            .profiles()
            .expect("list profiles")
            .into_iter()
            .map(|entry| entry.profile_id)
            .collect::<Vec<_>>(),
        vec!["ada".to_owned(), "echo-ready".to_owned()]
    );

    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn interactive_driver_and_durable_projector_share_one_connection() {
    let root = fixture_root("union");
    let (state, ledger) = shell_over_fixtures(&root);

    let (connection, route, session) = state
        .open_connection("echo-main", ShellConnectionMode::New, None)
        .await
        .expect("open new shell connection");
    assert!(!session.is_empty());
    let first = state
        .submit_prompt(&connection, text("late first"))
        .await
        .expect("first late prompt");
    let second = state
        .submit_prompt(&connection, text("late second"))
        .await
        .expect("second late prompt");
    assert_ne!(first.reply_text, second.reply_text);
    let outcome = state
        .disconnect(&connection)
        .await
        .expect("graceful disconnect");
    assert_eq!(outcome.termination, NativeSessionTermination::Disconnected);
    assert_eq!(outcome.turns.len(), 2);

    let kinds = surface_kinds(&ledger, &route);
    assert_eq!(
        kinds.first().map(String::as_str),
        Some("host/session_connecting")
    );
    assert_eq!(
        kinds.last().map(String::as_str),
        Some("host/session_terminal")
    );
    for required in [
        "acp/initialize",
        "acp/session_new",
        "host/prompt_submitted",
        "acp/session_update",
        "acp/prompt_response",
        "host/session_disconnect",
    ] {
        assert!(kinds.contains(&required.to_owned()), "missing {required}");
    }
    // Both driver turns are visible on the durable lane.
    assert_eq!(
        kinds
            .iter()
            .filter(|kind| *kind == "host/prompt_submitted")
            .count(),
        2
    );

    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn surface_permission_travels_through_the_shell_and_leaves_an_external_receipt() {
    let root = fixture_root("permission");
    let (state, ledger) = shell_over_fixtures(&root);
    let receipt = root.join("mcp-receipt.json");

    let (connection, route, _session) = state
        .open_connection("echo-main", ShellConnectionMode::New, None)
        .await
        .expect("open new shell connection");
    let prompt_state = Arc::clone(&state);
    let prompt_connection = connection.clone();
    let prompt = tokio::spawn(async move {
        prompt_state
            .submit_prompt(
                &prompt_connection,
                text(
                    &serde_json::json!({
                        "fixture": "mcp-echo-permission-v0.1",
                        "server": "echo",
                        "nonce": "shell-nonce"
                    })
                    .to_string(),
                ),
            )
            .await
    });
    let request = state
        .next_permission(&connection, Duration::from_secs(10))
        .await
        .expect("permission lane")
        .expect("fixture requested permission");
    let allow = request
        .options
        .iter()
        .find(|option| option.kind == PermissionOptionKind::AllowOnce)
        .expect("agent offered allow_once");
    let recovered = state
        .next_permission(&connection, Duration::from_secs(1))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered, request,
        "new observer sees the same pending request"
    );
    assert!(
        state
            .permission_after(&connection, request.sequence, Duration::from_millis(20))
            .await
            .unwrap()
            .is_none(),
        "one observer must not spin on a rendered request"
    );
    // An invented option id is refused and the request stays answerable.
    let invented = state
        .select_permission(&connection, request.sequence, "not-an-option")
        .await;
    assert!(matches!(invented, Err(WorkbenchShellError::Conflict(_))));
    state
        .select_permission(&connection, request.sequence, allow.option_id.0.as_ref())
        .await
        .expect("select exact agent-offered option");
    assert!(
        matches!(
            state
                .select_permission(&connection, request.sequence, allow.option_id.0.as_ref())
                .await,
            Err(WorkbenchShellError::Conflict(_))
        ),
        "only one observer may answer"
    );
    let turn = prompt
        .await
        .expect("prompt task")
        .expect("permitted turn completed");
    assert!(turn.reply_text.contains("shell-nonce"));
    assert!(receipt.is_file(), "external MCP receipt was not written");
    state.disconnect(&connection).await.expect("disconnect");

    let kinds = surface_kinds(&ledger, &route);
    let request_position = kinds
        .iter()
        .position(|kind| kind == "acp/session_request_permission")
        .expect("permission request on the durable lane");
    let decision_position = kinds
        .iter()
        .position(|kind| kind == "host/permission_decision")
        .expect("permission decision on the durable lane");
    assert!(request_position < decision_position);

    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one linear lifecycle transaction keeps cancel, resume continuity and the two close branches over one ledger visible"
)]
async fn cancel_close_and_resume_stay_fail_closed_and_replay_nothing() {
    let root = fixture_root("lifecycle");
    let (state, ledger) = shell_over_fixtures(&root);

    // Unknown route fails closed before any process is launched.
    let unknown = state
        .open_connection(
            "echo-main",
            ShellConnectionMode::Resume,
            Some("route-that-never-existed".into()),
        )
        .await;
    assert!(matches!(unknown, Err(WorkbenchShellError::NotFound(_))));

    // Cancel an active turn through the shell.
    let (connection, route, first_session) = state
        .open_connection("echo-main", ShellConnectionMode::New, None)
        .await
        .expect("open new shell connection");
    let cancel_state = Arc::clone(&state);
    let cancel_connection = connection.clone();
    let blocked = tokio::spawn(async move {
        cancel_state
            .submit_prompt(&cancel_connection, text(r#"{"fixture":"cancel-v0.1"}"#))
            .await
    });
    let cancelled = loop {
        let value = state.cancel(&connection).await.expect("cancel endpoint");
        if !value["active_turn"].is_null() {
            break value;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(cancelled["active_turn"]["session_id"], first_session);
    let turn = blocked
        .await
        .expect("cancel prompt task")
        .expect("cancelled turn still returns its outcome");
    assert_eq!(
        serde_json::to_value(turn.control_outcome).expect("serialize control outcome"),
        serde_json::json!("cancelled")
    );
    state.disconnect(&connection).await.expect("disconnect");

    // Consume the whole lane as the browser surface, acknowledging the head.
    let batch = state
        .events(&route, "workbench-browser", 1000, Duration::from_secs(1))
        .await
        .expect("read the first connection's lane");
    assert!(!batch.events.is_empty());
    state
        .acknowledge(&route, "workbench-browser", batch.next_cursor())
        .await
        .expect("acknowledge rendered events");

    // Resume the same route: same native session, zero replay, and the acked
    // surface receives only the new connection's events.
    let (resumed, resumed_route, resumed_session) = state
        .open_connection(
            "echo-main",
            ShellConnectionMode::Resume,
            Some(route.clone()),
        )
        .await
        .expect("resume over the stored route binding");
    assert_eq!(resumed_route, route);
    assert_eq!(resumed_session, first_session);
    let outcome = state
        .disconnect(&resumed)
        .await
        .expect("disconnect resumed");
    assert_eq!(outcome.replayed_updates, 0);
    let continuation = state
        .events(&route, "workbench-browser", 1000, Duration::from_secs(1))
        .await
        .expect("read the resumed connection's lane");
    assert!(
        continuation.after_cursor >= batch.next_cursor(),
        "cursor continuity was lost"
    );
    assert!(
        continuation
            .events
            .iter()
            .all(|event| event.sequence > batch.next_cursor()),
        "acked events were replayed to the surface"
    );
    assert!(
        continuation
            .events
            .iter()
            .any(|event| event.kind == "acp/session_resume"),
        "resume is not visible on the durable lane"
    );

    // Advertised close is capability-gated: the hiding agent refuses, the
    // connection stays usable, and disconnect still works afterwards.
    let (nocls, _route, _session) = state
        .open_connection("echo-nocls", ShellConnectionMode::New, None)
        .await
        .expect("open close-hiding connection");
    let refused = state.close(&nocls).await;
    assert!(matches!(refused, Err(WorkbenchShellError::Conflict(_))));
    state
        .submit_prompt(&nocls, text("still usable"))
        .await
        .expect("connection survived the refused close");
    state.disconnect(&nocls).await.expect("disconnect nocls");

    // The advertising agent closes for real.
    let (closing, _route, _session) = state
        .open_connection("echo-main", ShellConnectionMode::New, None)
        .await
        .expect("open closable connection");
    let closed = state.close(&closing).await.expect("advertised close");
    assert_eq!(closed.termination, NativeSessionTermination::Closed);

    drop(surface_kinds(&ledger, &route));
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn the_sandbox_origin_is_a_second_port_and_echoes_the_csp_as_a_real_header() {
    let root = fixture_root("sandbox");
    let (state, _ledger) = shell_over_fixtures(&root);
    let handle = swem_host::serve_workbench_http(state, ([127, 0, 0, 1], 0).into())
        .await
        .expect("bind shell + sandbox listeners");
    assert_ne!(
        handle.local_addr.port(),
        handle.sandbox_addr.port(),
        "the sandbox must be a different origin (different port)"
    );

    let mut workbench_asset = tokio::net::TcpStream::connect(handle.local_addr)
        .await
        .expect("connect to the Workbench origin");
    workbench_asset
        .write_all(b"GET /workbench.js HTTP/1.1\r\nHost: workbench\r\nConnection: close\r\n\r\n")
        .await
        .expect("request embedded Workbench asset");
    let mut asset_response = String::new();
    workbench_asset
        .read_to_string(&mut asset_response)
        .await
        .expect("read embedded Workbench asset");
    assert!(asset_response.starts_with("HTTP/1.1 200"));
    assert!(
        asset_response.contains("content-type: application/javascript;charset=utf-8"),
        "the Workbench asset needs an executable JavaScript content type"
    );
    assert!(
        asset_response.contains("react-19"),
        "the Rust host did not serve the production Workbench build"
    );

    let mut stream = tokio::net::TcpStream::connect(handle.sandbox_addr)
        .await
        .expect("connect to the sandbox origin");
    // `%20` decodes to a space; the embedded CRLF must be stripped so header
    // injection through the query parameter is impossible.
    stream
        .write_all(
            b"GET /sandbox?csp=default-src%20'none'%3B%0d%0aX-Injected:1 HTTP/1.1\r\nHost: sandbox\r\nConnection: close\r\n\r\n",
        )
        .await
        .expect("send sandbox request");
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .await
        .expect("read sandbox response");
    let headers = response
        .split("\r\n\r\n")
        .next()
        .expect("response has headers");
    assert!(headers.contains("content-security-policy: default-src 'none';X-Injected:1"));
    assert!(!headers.contains("\r\nX-Injected"), "header injection");
    assert!(response.contains("sandbox-proxy-ready"));

    // The API surface does not exist on the sandbox origin.
    let mut api_probe = tokio::net::TcpStream::connect(handle.sandbox_addr)
        .await
        .expect("connect again");
    api_probe
        .write_all(b"GET /api/profiles HTTP/1.1\r\nHost: sandbox\r\nConnection: close\r\n\r\n")
        .await
        .expect("send api probe");
    let mut api_response = String::new();
    api_probe
        .read_to_string(&mut api_response)
        .await
        .expect("read api probe");
    assert!(api_response.starts_with("HTTP/1.1 404"));

    drop(handle);
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// An agent left to work by itself answers its own permission requests,
/// inside the boundary the person picked, and never stops on a question
/// nobody is there to answer.
///
/// This is the other half of the permission surface. Everything above proves
/// a request reaches a person and comes back; none of it is any use when
/// nobody is watching, which is the case a person running an agent overnight
/// actually has. Before this, the Workbench answered every request the same
/// way and an agent alone waited until its session timed out.
///
/// Both directions are claimed, because a boundary that only ever allows is
/// not a boundary: a tool of the server this profile attached goes through
/// without asking and its effect is on disk, and a tool of a server it did
/// not attach is refused without asking and leaves nothing behind.
#[tokio::test]
async fn an_agent_left_alone_answers_inside_its_boundary_and_refuses_outside_it() {
    let root = fixture_root("alone");
    let (state, ledger) = shell_over_fixtures(&root);
    let receipt = root.join("mcp-receipt.json");

    let (connection, route, _session) = state
        .open_connection("echo-alone", ShellConnectionMode::New, None)
        .await
        .expect("open a connection for the agent working alone");

    // A tool of the server this profile attached. Nobody answers anything.
    let turn = state
        .submit_prompt(
            &connection,
            text(
                &serde_json::json!({
                    "fixture": "mcp-echo-permission-v0.1",
                    "server": "echo",
                    "nonce": "alone-inside"
                })
                .to_string(),
            ),
        )
        .await
        .expect("the turn finished with nobody to answer a question");
    assert!(turn.reply_text.contains("alone-inside"));
    assert!(
        receipt.is_file(),
        "the attached server's tool did not actually run"
    );
    assert!(
        state
            .next_permission(&connection, Duration::from_millis(200))
            .await
            .expect("permission lane")
            .is_none(),
        "the person was asked, which is the thing this choice exists to avoid"
    );

    // A tool that names a server this profile never attached. Still nobody is
    // asked - it is refused.
    std::fs::remove_file(&receipt).expect("clear the receipt between the two halves");
    let refused = state
        .submit_prompt(
            &connection,
            text(
                &serde_json::json!({
                    "fixture": "mcp-echo-permission-v0.1",
                    "server": "echo",
                    "title": "mcp__somewhere_else__write",
                    "nonce": "alone-outside"
                })
                .to_string(),
            ),
        )
        .await;
    assert!(
        state
            .next_permission(&connection, Duration::from_millis(200))
            .await
            .expect("permission lane")
            .is_none(),
        "a refusal must be a refusal, not a question nobody will answer"
    );
    assert!(
        !receipt.is_file(),
        "the refused call ran anyway: {refused:?}"
    );

    state.disconnect(&connection).await.expect("disconnect");
    let kinds = surface_kinds(&ledger, &route);
    assert!(
        kinds.contains(&"host/permission_decision".to_owned()),
        "the decisions the host made for the person are not on the durable lane: {kinds:?}"
    );
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// A connection's address is a capability, so it cannot be guessed.
///
/// The id is in the address of every App that connection opens, on the
/// sandbox origin, where an App View has an opaque origin and so cannot be
/// told apart from any other page by where it came from. While the ids were
/// `c1` and `c2`, a page that found that port could name an open App's
/// upload and blob routes without being given anything.
#[tokio::test]
async fn two_connections_get_addresses_nobody_can_guess() {
    let root = fixture_root("guessable");
    let (state, _ledger) = shell_over_fixtures(&root);
    let (first, _, _) = state
        .open_connection("echo-main", ShellConnectionMode::New, None)
        .await
        .unwrap();
    let (second, _, _) = state
        .open_connection("echo-main", ShellConnectionMode::New, None)
        .await
        .unwrap();
    for (ordinal, id) in [("c1", &first), ("c2", &second)] {
        assert_ne!(id.as_str(), ordinal, "the address is the ordinal itself");
        assert!(
            id.len() > 20,
            "{id} is short enough to walk through: an address that is a capability is not a count"
        );
    }
    assert_ne!(first, second, "two connections share one address");
    // The counter is still there, for reading a log in order.
    assert!(first.starts_with("c1-") && second.starts_with("c2-"));
    // A caller that names its own connection still gets what it named: a
    // test or an embedder addresses a connection it chose.
    let (named, _, _) = state
        .open_connection_with_id(
            "echo-main",
            ShellConnectionMode::New,
            None,
            Some("mine".into()),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(named, "mine");
    std::fs::remove_dir_all(&root).ok();
}

/// Content the agent cannot take is refused, and the session survives it.
///
/// This was a hang, not a refusal: the shell treated every failed prompt as a
/// dead connection and awaited a runner that had not stopped, because nothing
/// had been sent to the agent at all. A person attaching a file to an agent
/// without embedded context therefore watched the turn spin for good. The
/// walk below is the whole fact: the turn is refused with a reason, and the
/// very next turn on the same connection works.
#[tokio::test]
async fn content_the_agent_cannot_take_is_refused_and_the_session_survives() {
    let root = fixture_root("refused-content");
    let (state, _ledger) = shell_over_fixtures(&root);
    let (connection, _route, _session) = state
        .open_connection("echo-plain", ShellConnectionMode::New, None)
        .await
        .unwrap();

    // This fixture advertises no embedded context, so an embedded
    // resource is content it never said it could read.
    let embedded = vec![agent_client_protocol::schema::v1::ContentBlock::Resource(
        agent_client_protocol::schema::v1::EmbeddedResource::new(
            agent_client_protocol::schema::v1::EmbeddedResourceResource::TextResourceContents(
                agent_client_protocol::schema::v1::TextResourceContents::new(
                    "a note",
                    "ni:///sha-256;abc",
                ),
            ),
        ),
    )];
    let refusal = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        state.submit_prompt(&connection, embedded),
    )
    .await
    .expect("a refusal is an answer, not a wait");
    assert!(
        matches!(refusal, Err(WorkbenchShellError::Conflict(ref why)) if why.contains("capability")),
        "refused content did not come back as a conflict: {refusal:?}"
    );

    // And the connection is exactly as it was.
    state
        .submit_prompt(&connection, text("hello"))
        .await
        .expect("the session is still usable after refusing content");
    state.close(&connection).await.expect("close the session");
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// A person hands a file over and the agent finds it by path, with nothing
/// negotiated. The product walk proves the whole journey through the page;
/// this proves the host's half - that the inbox is written, that the turn
/// carries the path, and that a name which is a path is refused.
#[tokio::test]
async fn a_handed_file_lands_in_the_inbox_and_the_turn_names_its_path() {
    let root = fixture_root("inbox");
    let workspace = root.join("workspace");
    let landed = swem_host::hand_over_for_tests(&workspace, "notes.txt", b"a note")
        .await
        .expect("hand the file over");
    assert_eq!(landed, workspace.join("inbox").join("notes.txt"));

    // The same name with different bytes is a second file, not an overwrite.
    let again = swem_host::hand_over_for_tests(&workspace, "notes.txt", b"another note")
        .await
        .expect("hand a second file over");
    assert_eq!(again, workspace.join("inbox").join("notes-1.txt"));
    // The same name with the same bytes is the same file.
    assert_eq!(
        swem_host::hand_over_for_tests(&workspace, "notes.txt", b"a note")
            .await
            .expect("hand the first file over again"),
        landed
    );

    let listed = swem_host::list_handed_files_for_tests(&workspace)
        .await
        .expect("list what is there");
    assert_eq!(
        listed
            .iter()
            .map(|file| (file.area.as_str(), file.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("inbox", "notes-1.txt"), ("inbox", "notes.txt")]
    );

    for refused in ["../escaped.txt", "a/b.txt", "", "..", "."] {
        assert!(
            swem_host::hand_over_for_tests(&workspace, refused, b"x")
                .await
                .is_err(),
            "a file named {refused:?} was accepted"
        );
    }
    assert!(
        swem_host::read_handed_file_for_tests(&workspace, "outbox", "notes.txt")
            .await
            .is_err(),
        "a file was served from the wrong directory"
    );
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// A profile's setup reaches its agent by both roads: the role in the
/// instruction file the agent's layout names, the model in the variable the
/// agent reads at launch, and - when the agent offers a model choice on the
/// session - through `session/set_config_option` too. The echo fixture
/// stands in for Claude Code here (the resolver launches it whatever the
/// agent id says), advertises `fast` and `quality`, and reports what it got.
#[tokio::test]
async fn a_profiles_role_and_model_reach_the_agent_by_file_variable_and_session() {
    let root = fixture_root("setup-reaches");
    let (state, _ledger) = shell_over_fixtures(&root);
    let workspace = root.join("workspace");
    let profile = PersonalAgentProfile::new(
        "echo-ada",
        "claude-code",
        "direct-host-distribution",
        "direct-host-environment",
        "surface-permissions",
        &workspace,
        &root.join("agent-home"),
        Vec::new(),
        Vec::new(),
    )
    .expect("profile")
    .with_setup(swem_host::AgentSetup {
        model: Some("quality".into()),
        role: "You are Ada.".into(),
        ..Default::default()
    })
    .expect("setup");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(&profile)
        .expect("persist profile");

    let (connection, _route, _session) = tokio::time::timeout(
        Duration::from_mins(1),
        state.open_connection("echo-ada", ShellConnectionMode::New, None),
    )
    .await
    .expect("opened in time")
    .expect("opens");
    let instructions = fs::read_to_string(workspace.join("CLAUDE.md"))
        .expect("the role was written for Claude Code");
    assert!(instructions.contains("You are Ada."), "{instructions}");
    let status = state.connection_status(&connection).await.expect("status");
    let notes = status["setup"].as_array().expect("setup notes").clone();
    assert!(
        notes
            .iter()
            .any(|note| note.as_str().is_some_and(|note| note.contains("CLAUDE.md"))),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note.as_str() == Some("model quality set on the session")),
        "{notes:?}"
    );

    let reply = state
        .submit_prompt(&connection, text("what did you get"))
        .await
        .expect("prompt");
    let seen: serde_json::Value =
        serde_json::from_str(&reply.reply_text).expect("the echo answers JSON");
    assert_eq!(
        seen["environment"]["model_environment"]["ANTHROPIC_MODEL"],
        "quality"
    );
    assert_eq!(seen["configuration"]["model"], "quality");

    let options = state
        .connection_options(&connection)
        .await
        .expect("options");
    let listed = options
        .config_options
        .expect("the echo offers config options");
    assert!(listed.iter().any(|option| option.id.0.as_ref() == "model"));
    // A value the agent does not offer is refused with its sentence, and the
    // session is still there to answer.
    let refused = state
        .set_connection_option(&connection, "model", &serde_json::json!("imaginary"))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, WorkbenchShellError::Conflict(_)),
        "{refused}"
    );
    state
        .set_connection_option(&connection, "model", &serde_json::json!("fast"))
        .await
        .expect("a listed value is taken");
    let again = state
        .submit_prompt(&connection, text("and now"))
        .await
        .expect("prompt after the change");
    let seen: serde_json::Value = serde_json::from_str(&again.reply_text).expect("JSON");
    assert_eq!(seen["configuration"]["model"], "fast");
    state.disconnect(&connection).await.expect("disconnect");
    fs::remove_dir_all(&root).ok();
}
