//! Executable two-process fixture for host-owned routing continuity.
//!
//! Each invocation is one short-lived host process. The ACP echo agent is also
//! a separate process and owns the conversation state in the workspace.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use serde_json::Value;
use swem_host::{
    AttachmentBinding, AttachmentTransport, IntegrationKind, LaunchCommand, NativeSessionOptions,
    NativeSessionStart, RoutingLedger, SessionRouteBinding, SurfaceEventSource, run_native_session,
};

const ROUTE_ID: &str = "two-surface-route";
const SURFACE_A: &str = "surface-a";
const SURFACE_B: &str = "surface-b";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let stage = arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .ok_or("missing fixture stage")?;
    let workspace = absolute(arguments.next().ok_or("missing workspace")?)?;
    let ledger_path = absolute(arguments.next().ok_or("missing ledger path")?)?;
    let output_path = absolute(arguments.next().ok_or("missing output path")?)?;
    if arguments.next().is_some() {
        return Err("unexpected fixture arguments".into());
    }

    match stage.as_str() {
        "first" => first_process(&workspace, &ledger_path, &output_path).await?,
        "second" => second_process(&workspace, &ledger_path, &output_path).await?,
        "crash-after-append" => {
            crash_after_append(&workspace, &ledger_path)?;
            std::process::exit(86);
        }
        _ => return Err(format!("unknown fixture stage: {stage}").into()),
    }
    Ok(())
}

async fn first_process(
    workspace: &Path,
    ledger_path: &Path,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (launch, agent, mcp_servers, attachments) = fixture_connection()?;
    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.mcp_servers = mcp_servers;
    let outcome = run_native_session(
        &launch,
        &agent,
        workspace,
        &["first surface turn".into()],
        &options,
    )
    .await?;
    let binding = SessionRouteBinding::new(
        ROUTE_ID,
        "swem-echo-agent",
        "fixture-agent-profile-v1",
        &outcome.session_id,
        "local-fixture-environment-v1",
        workspace,
        attachments,
    )?;
    let mut ledger = RoutingLedger::open(ledger_path)?;
    ledger.bind_route(&binding)?;
    let sequence = append_turn(&mut ledger, &outcome, 0)?;
    let surface_a = ledger.events_for_surface(ROUTE_ID, SURFACE_A, 16)?;
    ledger.acknowledge_surface(ROUTE_ID, SURFACE_A, surface_a.next_cursor())?;
    let surface_b = ledger.events_for_surface(ROUTE_ID, SURFACE_B, 16)?;

    write_json(
        output_path,
        &serde_json::json!({
            "stage": "first",
            "session_id": outcome.session_id,
            "turn": parse_turn(&outcome.turns[0].reply_text)?,
            "sequence": sequence,
            "surface_a_events": surface_a.events.len(),
            "surface_a_cursor": ledger.surface_cursor(ROUTE_ID, SURFACE_A)?,
            "surface_b_events": surface_b.events.len(),
            "surface_b_cursor": ledger.surface_cursor(ROUTE_ID, SURFACE_B)?,
            "replayed_updates": outcome.replayed_updates,
            "native_session_listed": outcome.native_session_listed,
        }),
    )
}

async fn second_process(
    workspace: &Path,
    ledger_path: &Path,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (launch, agent, mcp_servers, attachments) = fixture_connection()?;
    let mut ledger = RoutingLedger::open(ledger_path)?;
    let stored = ledger.route(ROUTE_ID)?;
    let expected = SessionRouteBinding::new(
        ROUTE_ID,
        "swem-echo-agent",
        "fixture-agent-profile-v1",
        &stored.native_session_id,
        "local-fixture-environment-v1",
        workspace,
        attachments,
    )?;
    ledger.require_identity(&expected.identity())?;

    let mut options = NativeSessionOptions::new(Duration::from_secs(10));
    options.start = NativeSessionStart::Resume {
        session_id: stored.native_session_id.clone(),
    };
    options.mcp_servers = mcp_servers;
    let outcome = run_native_session(
        &launch,
        &agent,
        workspace,
        &["second surface turn".into()],
        &options,
    )
    .await?;
    append_turn(&mut ledger, &outcome, 0)?;

    let surface_a = ledger.events_for_surface(ROUTE_ID, SURFACE_A, 16)?;
    let surface_b = ledger.events_for_surface(ROUTE_ID, SURFACE_B, 16)?;
    ledger.acknowledge_surface(ROUTE_ID, SURFACE_A, surface_a.next_cursor())?;
    ledger.acknowledge_surface(ROUTE_ID, SURFACE_B, surface_b.next_cursor())?;

    write_json(
        output_path,
        &serde_json::json!({
            "stage": "second",
            "session_id": outcome.session_id,
            "turn": parse_turn(&outcome.turns[0].reply_text)?,
            "start": outcome.start,
            "replayed_updates": outcome.replayed_updates,
            "native_session_listed": outcome.native_session_listed,
            "surface_a_events": surface_a.events.len(),
            "surface_a_event_ids": surface_a.events.iter().map(|event| &event.event_id).collect::<Vec<_>>(),
            "surface_a_cursor": ledger.surface_cursor(ROUTE_ID, SURFACE_A)?,
            "surface_b_events": surface_b.events.len(),
            "surface_b_event_ids": surface_b.events.iter().map(|event| &event.event_id).collect::<Vec<_>>(),
            "surface_b_cursor": ledger.surface_cursor(ROUTE_ID, SURFACE_B)?,
        }),
    )
}

fn crash_after_append(
    workspace: &Path,
    ledger_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let binding = SessionRouteBinding::new(
        "crash-route",
        "fixture-agent",
        "fixture-profile",
        "fixture-native-session",
        "fixture-environment",
        workspace,
        Vec::new(),
    )?;
    let mut ledger = RoutingLedger::open(ledger_path)?;
    ledger.bind_route(&binding)?;
    ledger.append_event(
        "crash-route",
        "committed-before-crash",
        "host_observation",
        SurfaceEventSource::Host,
        &serde_json::json!({ "committed": true }),
    )?;
    Ok(())
}

fn fixture_connection() -> Result<FixtureConnection, Box<dyn std::error::Error>> {
    let current = std::env::current_exe()?;
    let directory = current.parent().ok_or("fixture binary has no parent")?;
    let agent = directory.join(format!("swem-echo-agent{}", std::env::consts::EXE_SUFFIX));
    if !agent.is_file() {
        return Err(format!("echo agent not found at {}", agent.display()).into());
    }
    let launch = LaunchCommand {
        executable: agent.display().to_string(),
        args: Vec::new(),
        integration: IntegrationKind::DirectAcp,
    };
    let mcp_servers = vec![McpServer::Stdio(McpServerStdio::new(
        "fixture-notes",
        "unused-fixture-server",
    ))];
    let attachments = vec![AttachmentBinding::new(
        "fixture-notes-profile-v1",
        "fixture-notes",
        AttachmentTransport::Stdio,
    )];
    Ok((launch, agent, mcp_servers, attachments))
}

type FixtureConnection = (
    LaunchCommand,
    PathBuf,
    Vec<McpServer>,
    Vec<AttachmentBinding>,
);

fn append_turn(
    ledger: &mut RoutingLedger,
    outcome: &swem_host::NativeSessionOutcome,
    turn_index: usize,
) -> Result<u64, Box<dyn std::error::Error>> {
    let turn = outcome
        .turns
        .get(turn_index)
        .ok_or("fixture outcome has no turn")?;
    let turn_number = parse_turn(&turn.reply_text)?;
    let event_id = format!("native:{}:turn:{turn_number}:terminal", outcome.session_id);
    Ok(ledger.append_event(
        ROUTE_ID,
        &event_id,
        "native_turn_finished",
        SurfaceEventSource::NativeLive,
        &serde_json::json!({
            "native_session_id": outcome.session_id,
            "turn": turn_number,
            "stop_reason": turn.stop_reason,
            "reply_text": turn.reply_text,
        }),
    )?)
}

fn parse_turn(reply: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let value: Value = serde_json::from_str(reply)?;
    value["turn"]
        .as_u64()
        .ok_or_else(|| "reply has no turn".into())
}

fn absolute(path: std::ffi::OsString) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(format!("fixture path must be absolute: {}", path.display()).into());
    }
    Ok(path)
}

fn write_json(path: &Path, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
