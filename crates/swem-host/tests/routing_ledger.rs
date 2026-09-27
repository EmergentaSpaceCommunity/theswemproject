use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;
use swem_host::{
    AttachmentBinding, AttachmentTransport, RoutingError, RoutingLedger, SessionRouteBinding,
    SurfaceEventSource,
};

#[test]
fn two_host_processes_route_two_surfaces_to_one_native_session() {
    let root = fixture_root("reconnect");
    fs::create_dir_all(&root).expect("create reconnect root");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create reconnect workspace");
    let ledger = root.join("routing.sqlite3");
    let first_output = root.join("first.json");
    let second_output = root.join("second.json");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-routing-fixture"));

    run_stage(&fixture, "first", &workspace, &ledger, &first_output);
    run_stage(&fixture, "second", &workspace, &ledger, &second_output);

    let first = read_json(&first_output);
    let second = read_json(&second_output);
    assert_eq!(first["session_id"], second["session_id"]);
    assert_eq!(first["turn"], 1);
    assert_eq!(second["turn"], 2, "agent-owned state did not continue");
    assert_eq!(first["replayed_updates"], 0);
    assert!(first["native_session_listed"].is_null());
    assert_eq!(
        second["replayed_updates"], 0,
        "resume must not replay history"
    );
    assert_eq!(second["native_session_listed"], true);
    assert_eq!(first["surface_a_events"], 1);
    assert_eq!(first["surface_b_events"], 1);
    assert_eq!(second["surface_a_events"], 1);
    assert_eq!(second["surface_b_events"], 2);
    assert_eq!(second["surface_a_event_ids"].as_array().unwrap().len(), 1);
    assert_eq!(second["surface_b_event_ids"].as_array().unwrap().len(), 2);
    assert_eq!(second["surface_a_cursor"], second["surface_b_cursor"]);
    assert_eq!(second["start"]["kind"], "resume");

    let agent_state = fs::read_to_string(workspace.join(".swem-echo-agent-session.json"))
        .expect("read agent-owned state");
    assert!(agent_state.contains("first surface turn"));
    assert!(agent_state.contains("second surface turn"));
    fs::remove_dir_all(root).expect("remove reconnect fixture");
}

#[test]
fn committed_event_survives_process_death_before_surface_ack() {
    let root = fixture_root("crash");
    fs::create_dir_all(&root).expect("create crash root");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).expect("create crash workspace");
    let ledger_path = root.join("routing.sqlite3");
    let unused_output = root.join("unused.json");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-routing-fixture"));

    let status = Command::new(&fixture)
        .args([
            "crash-after-append",
            workspace.to_str().unwrap(),
            ledger_path.to_str().unwrap(),
            unused_output.to_str().unwrap(),
        ])
        .status()
        .expect("run crash fixture");
    assert_eq!(status.code(), Some(86));

    let mut ledger = RoutingLedger::open(&ledger_path).expect("reopen after process death");
    let batch = ledger
        .events_for_surface("crash-route", "late-surface", 8)
        .expect("read committed event");
    assert_eq!(batch.after_cursor, 0);
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].event_id, "committed-before-crash");
    ledger
        .acknowledge_surface("crash-route", "late-surface", batch.next_cursor())
        .expect("ack after recovery");
    assert!(
        ledger
            .events_for_surface("crash-route", "late-surface", 8)
            .expect("read after ack")
            .events
            .is_empty()
    );

    drop(ledger);
    fs::remove_dir_all(root).expect("remove crash fixture");
}

#[test]
fn binding_events_and_cursors_fail_closed() {
    let root = fixture_root("invariants");
    fs::create_dir_all(&root).expect("create invariant workspace");
    let mut ledger = RoutingLedger::open(&root.join("routing.sqlite3")).expect("open ledger");
    let route_binding = binding(&root, "agent-profile");
    ledger.bind_route(&route_binding).expect("bind route");
    ledger
        .bind_route(&route_binding)
        .expect("idempotent route bind");

    let drift = binding(&root, "other-agent-profile");
    assert!(matches!(
        ledger.require_identity(&drift.identity()),
        Err(RoutingError::BindingDrift { .. })
    ));
    assert!(matches!(
        ledger.bind_route(&drift),
        Err(RoutingError::BindingDrift { .. })
    ));

    let payload = serde_json::json!({"projection": "one"});
    let sequence = ledger
        .append_event(
            "route",
            "event-1",
            "projection",
            SurfaceEventSource::NativeLive,
            &payload,
        )
        .expect("append event");
    assert_eq!(
        ledger
            .append_event(
                "route",
                "event-1",
                "projection",
                SurfaceEventSource::NativeLive,
                &payload,
            )
            .expect("idempotent append"),
        sequence
    );
    assert!(matches!(
        ledger.append_event(
            "route",
            "event-1",
            "projection",
            SurfaceEventSource::NativeLive,
            &serde_json::json!({"projection": "different"}),
        ),
        Err(RoutingError::EventCollision { .. })
    ));

    ledger
        .acknowledge_surface("route", "surface", sequence)
        .expect("ack head");
    assert!(matches!(
        ledger.acknowledge_surface("route", "surface", sequence - 1),
        Err(RoutingError::CursorRegression { .. })
    ));
    assert!(matches!(
        ledger.acknowledge_surface("route", "surface", sequence + 1),
        Err(RoutingError::CursorBeyondHead { .. })
    ));

    drop(ledger);
    let reopened = RoutingLedger::open(&root.join("routing.sqlite3")).expect("reopen ledger");
    assert_eq!(
        reopened
            .surface_cursor("route", "surface")
            .expect("durable cursor"),
        sequence
    );
    assert_eq!(
        reopened.route("route").expect("durable route"),
        route_binding
    );

    drop(reopened);
    fs::remove_dir_all(root).expect("remove invariant fixture");
}

#[test]
fn concurrent_connections_preserve_one_ordered_route_log() {
    let root = fixture_root("concurrent");
    fs::create_dir_all(&root).expect("create concurrent workspace");
    let ledger_path = root.join("routing.sqlite3");
    let mut creator = RoutingLedger::open(&ledger_path).expect("open creator ledger");
    creator
        .bind_route(&binding(&root, "agent-profile"))
        .expect("bind concurrent route");
    drop(creator);

    let writers = (0..4)
        .map(|writer| {
            let ledger_path = ledger_path.clone();
            thread::spawn(move || {
                let mut ledger = RoutingLedger::open(&ledger_path).expect("open writer ledger");
                for event in 0..25 {
                    let event_id = format!("writer-{writer}-event-{event}");
                    ledger
                        .append_event(
                            "route",
                            &event_id,
                            "concurrent_observation",
                            SurfaceEventSource::Host,
                            &serde_json::json!({"writer": writer, "event": event}),
                        )
                        .expect("append concurrent event");
                }
            })
        })
        .collect::<Vec<_>>();
    for writer in writers {
        writer.join().expect("join concurrent writer");
    }

    let ledger = RoutingLedger::open(&ledger_path).expect("open reader ledger");
    let batch = ledger
        .events_for_surface("route", "reader", 200)
        .expect("query ordered log");
    assert_eq!(batch.events.len(), 100);
    assert!(
        batch
            .events
            .windows(2)
            .all(|events| events[0].sequence < events[1].sequence)
    );
    let identities = batch
        .events
        .iter()
        .map(|event| event.event_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(identities.len(), 100);

    drop(ledger);
    fs::remove_dir_all(root).expect("remove concurrent fixture");
}

fn binding(workspace: &Path, agent_profile: &str) -> SessionRouteBinding {
    SessionRouteBinding::new(
        "route",
        "agent",
        agent_profile,
        "native-session",
        "environment",
        workspace,
        vec![AttachmentBinding::new(
            "notes-profile",
            "notes",
            AttachmentTransport::Stdio,
        )],
    )
    .expect("fixture binding")
}

fn run_stage(fixture: &Path, stage: &str, workspace: &Path, ledger: &Path, output: &Path) {
    let status = Command::new(fixture)
        .args([
            stage,
            workspace.to_str().unwrap(),
            ledger.to_str().unwrap(),
            output.to_str().unwrap(),
        ])
        .status()
        .expect("run routing fixture stage");
    assert!(status.success(), "routing fixture stage {stage} failed");
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read fixture output"))
        .expect("parse fixture output")
}

fn fixture_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "swem-routing-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ))
}
