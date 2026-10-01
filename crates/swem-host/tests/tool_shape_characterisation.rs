//! Controlled separation between MCP protocol conformance and empirical
//! agent/model argument realisation. The hermetic test proves the fixture and
//! exact-wire oracle. The ignored live test records observations for one exact
//! native agent stack; it does not turn them into a global compatibility list.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    ContentBlock, McpServer, McpServerStdio, PermissionOptionKind, SessionConfigKind,
    SessionConfigOptionValue, TextContent,
};
use rmcp::ServiceExt as _;
use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::transport::TokioChildProcess;
use serde_json::{Value, json};
use swem_host::{
    NativePermissionDecisionSource, NativeSessionControl, NativeSessionOptions,
    SessionPermissionPolicy, discover_agents, run_interactive_native_session,
    verify_discovered_agent,
};

const SERVER: &str = "tool-shapes";
const FLAT: &str = "shape_flat_closed";
const NESTED: &str = "shape_nested_closed";
const UNION: &str = "shape_discriminated_union";
const VALUE: &str = "shape_unconstrained_value";
const LIVE_MODEL: &str = "xiaomi-token-plan-sgp/mimo-v2.5";

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-tool-shapes-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create tool-shape fixture root");
    root
}

fn observer_declaration(receipts: &Path, evidence: &Path) -> McpServer {
    McpServer::Stdio(
        McpServerStdio::new(
            SERVER,
            PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        )
        .args(vec![
            "--server-name".into(),
            SERVER.into(),
            "--evidence".into(),
            evidence.display().to_string(),
            "--".into(),
            env!("CARGO_BIN_EXE_swem-mcp-tool-shape-fixture").into(),
            "--receipts".into(),
            receipts.display().to_string(),
        ]),
    )
}

async fn connect_through_observer(
    receipts: &Path,
    evidence: &Path,
) -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    let McpServer::Stdio(server) = observer_declaration(receipts, evidence) else {
        unreachable!("observer declaration is stdio")
    };
    let mut command = tokio::process::Command::new(server.command);
    command.args(server.args);
    let transport = TokioChildProcess::new(command).expect("spawn observed shape fixture");
    ().serve(transport)
        .await
        .expect("initialize shape fixture through observer")
}

fn samples(prefix: &str) -> Vec<(&'static str, Value)> {
    vec![
        (
            FLAT,
            json!({"nonce": format!("{prefix}-flat"), "enabled": true, "count": 3}),
        ),
        (
            NESTED,
            json!({
                "nonce": format!("{prefix}-nested"),
                "timeline": {
                    "title": "Two-beat cut",
                    "clips": [
                        {"asset": "intro-shot", "start_ms": 0, "duration_ms": 1200, "role": "opening"},
                        {"asset": "outro-shot", "start_ms": 1200, "duration_ms": 800, "role": "closing"}
                    ]
                }
            }),
        ),
        (
            UNION,
            json!({
                "nonce": format!("{prefix}-union"),
                "operation": {"kind": "trim", "start_ms": 250, "end_ms": 1750}
            }),
        ),
        (
            VALUE,
            json!({
                "nonce": format!("{prefix}-value"),
                "value": {"segments": [{"name": "intro", "gain": 0.8}], "mix": {"limiter": true}}
            }),
        ),
    ]
}

fn object(value: &Value) -> JsonObject {
    JsonObject::from_iter(value.as_object().expect("sample is object").clone())
}

fn jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse JSONL line"))
        .collect()
}

fn observation_status(attempts: usize, accepted: usize, exact_receipt: bool) -> &'static str {
    if exact_receipt {
        "supported"
    } else if accepted > 0 {
        "accepted_argument_mismatch"
    } else if attempts > 0 {
        "rejected"
    } else {
        "not_attempted"
    }
}

#[test]
fn observation_status_separates_protocol_rejection_from_semantic_mismatch() {
    assert_eq!(observation_status(1, 1, true), "supported");
    assert_eq!(
        observation_status(1, 1, false),
        "accepted_argument_mismatch"
    );
    assert_eq!(observation_status(3, 0, false), "rejected");
    assert_eq!(observation_status(0, 0, false), "not_attempted");
}

#[tokio::test]
async fn fixture_preserves_four_schema_shapes_and_exact_call_outcomes() {
    let root = fixture_root("contract");
    let receipts = root.join("receipts.jsonl");
    let evidence = root.join("wire.jsonl");
    let client = connect_through_observer(&receipts, &evidence).await;

    let tools = client
        .list_all_tools()
        .await
        .expect("list controlled tools");
    assert_eq!(tools.len(), 4);
    let by_name = tools
        .iter()
        .map(|tool| {
            (
                tool.name.as_ref(),
                Value::Object((*tool.input_schema).clone()),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for name in [FLAT, NESTED, UNION, VALUE] {
        assert_eq!(by_name[name]["type"], "object", "{name}");
        assert_eq!(
            by_name[name]["$schema"], "https://json-schema.org/draft/2020-12/schema",
            "{name}"
        );
        assert_eq!(by_name[name]["additionalProperties"], false, "{name}");
    }
    assert!(by_name[NESTED]["properties"]["timeline"]["properties"]["clips"]["items"]
        ["properties"]["role"]["enum"]
        .is_array());
    assert_eq!(
        by_name[UNION]["properties"]["operation"]["oneOf"]
            .as_array()
            .expect("oneOf preserved")
            .len(),
        2
    );
    assert_eq!(by_name[VALUE]["properties"]["value"], json!({}));

    let expected = samples("hermetic");
    for (name, arguments) in &expected {
        let result = client
            .call_tool(CallToolRequestParams::new(*name).with_arguments(object(arguments)))
            .await
            .unwrap_or_else(|error| panic!("valid {name} call failed: {error}"));
        assert_eq!(result.structured_content.unwrap()["accepted"], true);
    }
    let invalid = json!({"nonce": "hermetic-invalid", "enabled": true, "count": 3, "extra": 1});
    assert!(
        client
            .call_tool(CallToolRequestParams::new(FLAT).with_arguments(object(&invalid)))
            .await
            .is_err(),
        "fixture must reject an unknown property"
    );
    client.cancel().await.expect("close controlled fixture");

    let receipts = jsonl(&receipts);
    assert_eq!(receipts.len(), expected.len());
    for (name, arguments) in &expected {
        assert!(
            receipts
                .iter()
                .any(|receipt| receipt["tool"] == *name && receipt["arguments"] == *arguments),
            "missing exact receipt for {name}"
        );
    }
    let wire = jsonl(&evidence);
    assert_eq!(
        wire.iter().filter(|row| row["phase"] == "request").count(),
        expected.len() + 1
    );
    assert!(wire.iter().any(|row| {
        row["phase"] == "response"
            && row["tool"] == FLAT
            && row["arguments"] == invalid
            && row["error"]["code"] == -32602
    }));
    fs::remove_dir_all(root).expect("remove tool-shape fixture root");
}

#[tokio::test]
#[ignore = "live characterisation: requires SWEM_NATIVE_AGENT=opencode and its lawful provider credential"]
#[allow(
    clippy::too_many_lines,
    reason = "one live scenario keeps exact ACP model selection, wire responses and effect receipts in one auditable transaction"
)]
async fn live_agent_model_stack_records_tool_shape_observations_without_global_labels() {
    let agent_id = std::env::var("SWEM_NATIVE_AGENT")
        .expect("set SWEM_NATIVE_AGENT=opencode for this explicit live observation");
    assert_eq!(
        agent_id, "opencode",
        "this host currently authorizes OpenCode"
    );
    let root = fixture_root("live");
    let receipts = root.join("receipts.jsonl");
    let evidence = root.join("wire.jsonl");
    let transcript = root.join("acp.jsonl");
    let discovery = discover_agents()
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .expect("OpenCode is absent from discovery");
    let handshake = verify_discovered_agent(&discovery, Duration::from_secs(30))
        .await
        .expect("OpenCode ACP initialize handshake");
    assert_eq!(handshake.identity_matched, Some(true));
    let launch = discovery.launch.clone().expect("OpenCode launch command");
    let executable = discovery
        .executable_path
        .clone()
        .expect("OpenCode executable path");

    let expected = samples("live-056");
    let prompts = expected
        .iter()
        .map(|(name, arguments)| {
            format!(
                "Call the `{name}` tool on the attached `{SERVER}` MCP server exactly once with exactly these JSON arguments: {arguments}. Do not substitute another tool. Then reply with exactly OBSERVED."
            )
        })
        .collect::<Vec<_>>();
    let control = NativeSessionControl::new();
    let mut options = NativeSessionOptions::interactive(Duration::from_mins(6));
    options.control = Some(control.clone());
    options.permission_policy = SessionPermissionPolicy::Surface;
    options.transcript_path = Some(transcript.clone());
    options.mcp_servers = vec![observer_declaration(&receipts, &evidence)];
    let task = tokio::spawn({
        let launch = launch.clone();
        let executable = executable.clone();
        let root = root.clone();
        async move { run_interactive_native_session(&launch, &executable, &root, &options).await }
    });
    let permission_control = control.clone();
    let permission_task = tokio::spawn(async move {
        while let Some(request) = permission_control.next_permission_request().await {
            let allow = request
                .options
                .iter()
                .find(|option| option.kind == PermissionOptionKind::AllowOnce)
                .expect("agent omitted allow_once")
                .option_id
                .0
                .to_string();
            permission_control
                .select_permission(request.sequence, &allow)
                .expect("select exact allow_once option");
        }
    });
    tokio::time::timeout(Duration::from_mins(1), control.wait_until_ready())
        .await
        .expect("OpenCode did not reach idle")
        .expect("OpenCode finished before configuration");
    let configured = control
        .set_config_option("model", SessionConfigOptionValue::value_id(LIVE_MODEL))
        .await
        .expect("select exact live model through ACP");
    let model = configured
        .iter()
        .find(|option| option.id.0.as_ref() == "model")
        .expect("agent omitted model option after selection");
    let SessionConfigKind::Select(model) = &model.kind else {
        panic!("agent model option is not select")
    };
    assert_eq!(model.current_value.0.as_ref(), LIVE_MODEL);
    for prompt in &prompts {
        control
            .submit_prompt(vec![ContentBlock::Text(TextContent::new(prompt.clone()))])
            .await
            .expect("live tool-shape turn");
    }
    control
        .disconnect()
        .await
        .expect("disconnect live observation");
    let outcome = task
        .await
        .expect("join live characterisation")
        .expect("live characterisation session");
    permission_task.await.expect("join permission responder");
    assert_eq!(outcome.turns.len(), expected.len());
    assert!(
        outcome.permission_decisions.iter().all(|decision| {
            decision.allowed && decision.decision_source == NativePermissionDecisionSource::Surface
        }),
        "all live tool permissions must be explicit surface decisions"
    );

    let wire = jsonl(&evidence);
    let receipts = if receipts.exists() {
        jsonl(&receipts)
    } else {
        Vec::new()
    };
    let mut observations = Vec::new();
    for (name, arguments) in &expected {
        let attempts = wire
            .iter()
            .filter(|row| row["phase"] == "request" && row["tool"] == *name)
            .collect::<Vec<_>>();
        let responses = wire
            .iter()
            .filter(|row| row["phase"] == "response" && row["tool"] == *name)
            .collect::<Vec<_>>();
        let exact_receipt = receipts
            .iter()
            .any(|row| row["tool"] == *name && row["arguments"] == *arguments);
        let accepted_receipts = receipts
            .iter()
            .filter(|row| row["tool"] == *name)
            .collect::<Vec<_>>();
        let status = observation_status(attempts.len(), accepted_receipts.len(), exact_receipt);
        observations.push(json!({
            "tool": name,
            "status": status,
            "expected_arguments": arguments,
            "attempts": attempts,
            "responses": responses,
            "accepted_receipts": accepted_receipts,
            "exact_receipt": exact_receipt
        }));
    }
    assert_eq!(
        observations[0]["status"], "supported",
        "flat control must prove this exact stack reached the fixture"
    );
    assert_eq!(
        observations
            .iter()
            .map(|row| row["tool"].as_str().unwrap())
            .collect::<BTreeSet<_>>()
            .len(),
        4
    );

    let selected_config = outcome
        .config_options
        .as_ref()
        .into_iter()
        .flatten()
        .map(|option| {
            let value = serde_json::to_value(option).expect("serialize selected ACP option");
            json!({
                "id": value["id"],
                "category": value["category"],
                "current_value": value["currentValue"]
            })
        })
        .collect::<Vec<_>>();
    let report = json!({
        "kind": "swem.agent_tool_shape_observation",
        "schema_version": 1,
        "scope": "one exact run; not a global agent capability label",
        "agent_catalog_id": agent_id,
        "verified_handshake": handshake,
        "agent_info": outcome.agent_info,
        "selected_config": selected_config,
        "mcp_server": "swem-mcp-tool-shape-fixture",
        "mcp_schema_draft": "2020-12",
        "session_id": outcome.session_id,
        "observations": observations,
        "oracle": {
            "wire_evidence": evidence,
            "handler_receipts": root.join("receipts.jsonl"),
            "acp_transcript": transcript
        }
    });
    let report_path = root.join("report.json");
    fs::write(
        &report_path,
        serde_json::to_vec_pretty(&report).expect("serialize live report"),
    )
    .expect("write live report");
    println!("tool-shape report: {}", report_path.display());
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    // Keep live evidence for manual and paper audit. The path is unique and
    // contains no credential values; raw prompts contain only fixture data.
}
