//! Contract test for the arbitrary MCP Apps fixture: the server declares its
//! App the way the SEP-1865 extension does (ui:// resource, mcp-app MIME,
//! `_meta.ui` on tools and on the read content item), the tool effect is
//! proven by an external receipt, and the hostile mode serves different HTML
//! at the same URI. No SWEM Cycle code is loaded anywhere.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use rmcp::ServiceExt as _;
use rmcp::model::{CallToolRequestParams, JsonObject, ReadResourceRequestParams};
use rmcp::transport::TokioChildProcess;
use serde_json::Value;

const NOTES_URI: &str = "ui://apps-fixture/notes";
const APP_MIME: &str = "text/html;profile=mcp-app";

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-apps-fixture-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

async fn connect(
    receipt: &PathBuf,
    poison: &PathBuf,
    hostile: bool,
) -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture"));
    command
        .arg("--receipt")
        .arg(receipt)
        .arg("--poison")
        .arg(poison);
    if hostile {
        command.arg("--hostile");
    }
    let transport = TokioChildProcess::new(command).expect("spawn apps fixture");
    ().serve(transport).await.expect("initialize MCP client")
}

async fn connect_through_observer(
    receipt: &PathBuf,
    poison: &PathBuf,
    evidence: &PathBuf,
) -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture"));
    command
        .arg("--server-name")
        .arg("notes")
        .arg("--evidence")
        .arg(evidence)
        .arg("--")
        .arg(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture"))
        .arg("--receipt")
        .arg(receipt)
        .arg("--poison")
        .arg(poison);
    let transport = TokioChildProcess::new(command).expect("spawn observed apps fixture");
    ().serve(transport)
        .await
        .expect("initialize MCP client through observer")
}

#[tokio::test]
async fn the_fixture_declares_its_app_the_standard_way_and_proves_the_tool_by_receipt() {
    let root = fixture_root("contract");
    let receipt = root.join("notes-receipt.json");
    let poison = root.join("poison-receipt.json");
    let client = connect(&receipt, &poison, false).await;

    // Tool discovery: save_note links the App through _meta.ui.resourceUri
    // and is visible to model and app; the probe is model-only.
    let tools = client.list_all_tools().await.expect("list tools");
    let save_note = tools
        .iter()
        .find(|tool| tool.name == "save_note")
        .expect("save_note declared");
    let save_meta = save_note.meta.as_ref().expect("save_note _meta");
    assert_eq!(save_meta.0["ui"]["resourceUri"], NOTES_URI);
    assert_eq!(
        save_meta.0["ui"]["visibility"],
        serde_json::json!(["model", "app"])
    );
    let probe = tools
        .iter()
        .find(|tool| tool.name == "model_only_probe")
        .expect("model_only_probe declared");
    let probe_meta = probe.meta.as_ref().expect("probe _meta");
    assert_eq!(
        probe_meta.0["ui"]["visibility"],
        serde_json::json!(["model"])
    );
    assert!(probe_meta.0["ui"].get("resourceUri").is_none());

    // Resource discovery: ui:// + mcp-app MIME; listing level deliberately
    // carries no _meta.ui (the content item is the authority) - only the
    // host's own keys beside it: where the App works.
    let resources = client.list_all_resources().await.expect("list resources");
    let notes = resources
        .iter()
        .find(|resource| resource.uri == NOTES_URI)
        .expect("notes App resource listed");
    assert_eq!(notes.mime_type.as_deref(), Some(APP_MIME));
    let listing_meta = notes.meta.as_ref().expect("the listing's own keys");
    assert!(listing_meta.0.get("ui").is_none());
    assert_eq!(
        listing_meta.0["swem/platforms"],
        serde_json::json!(["web", "desktop", "mobile"])
    );

    // Read: HTML document with content-level _meta.ui.
    let read = client
        .read_resource(ReadResourceRequestParams::new(NOTES_URI))
        .await
        .expect("read notes App");
    let rmcp::model::ResourceContents::TextResourceContents {
        mime_type,
        text,
        meta,
        ..
    } = &read.contents[0]
    else {
        panic!("App resource must be text");
    };
    assert_eq!(mime_type.as_deref(), Some(APP_MIME));
    assert!(text.contains("<!DOCTYPE html>"));
    assert!(text.contains("ui/initialize"));
    let content_meta = meta.as_ref().expect("content-level _meta.ui");
    assert_eq!(content_meta.0["ui"]["prefersBorder"], true);

    // The tool effect is a file, never prose.
    let response =
        client
            .call_tool(CallToolRequestParams::new("save_note").with_arguments(
                JsonObject::from_iter(BTreeMap::from_iter([
                    ("nonce".to_owned(), Value::String("fixture-nonce".into())),
                    ("text".to_owned(), Value::String("hello board".into())),
                ])),
            ))
            .await
            .expect("call save_note");
    let structured = response.structured_content.expect("structured content");
    assert_eq!(structured["nonce"], "fixture-nonce");
    assert_eq!(structured["note_count"], 1);
    let written: Value =
        serde_json::from_slice(&fs::read(&receipt).expect("read receipt")).expect("parse receipt");
    assert_eq!(written["nonce"], "fixture-nonce");
    assert_eq!(written["text"], "hello board");
    assert!(!poison.exists(), "the probe must not have run");

    client.cancel().await.expect("shut fixture down");
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn the_hostile_mode_serves_different_html_at_the_same_uri() {
    let root = fixture_root("hostile");
    let receipt = root.join("notes-receipt.json");
    let poison = root.join("poison-receipt.json");

    let benign = connect(&receipt, &poison, false).await;
    let benign_read = benign
        .read_resource(ReadResourceRequestParams::new(NOTES_URI))
        .await
        .expect("read benign App");
    benign.cancel().await.expect("shut benign down");

    let hostile = connect(&receipt, &poison, true).await;
    let hostile_read = hostile
        .read_resource(ReadResourceRequestParams::new(NOTES_URI))
        .await
        .expect("read hostile App");
    hostile.cancel().await.expect("shut hostile down");

    let text_of = |response: &rmcp::model::ReadResourceResult| -> String {
        let rmcp::model::ResourceContents::TextResourceContents { text, .. } =
            &response.contents[0]
        else {
            panic!("App resource must be text");
        };
        text.clone()
    };
    let benign_text = text_of(&benign_read);
    let hostile_text = text_of(&hostile_read);
    assert_ne!(benign_text, hostile_text);
    assert!(hostile_text.contains("model_only_probe"));
    assert!(hostile_text.contains("no_such_tool"));

    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn the_attachment_observer_preserves_mcp_and_witnesses_the_exact_transaction() {
    let root = fixture_root("observer");
    let receipt = root.join("notes-receipt.json");
    let poison = root.join("poison-receipt.json");
    let evidence = root.join("observer.jsonl");
    let client = connect_through_observer(&receipt, &poison, &evidence).await;
    let response =
        client
            .call_tool(CallToolRequestParams::new("save_note").with_arguments(
                JsonObject::from_iter(BTreeMap::from_iter([
                    (
                        "nonce".to_owned(),
                        Value::String("observer-transaction-0.54".into()),
                    ),
                    (
                        "text".to_owned(),
                        Value::String("exact MCP result survives".into()),
                    ),
                ])),
            ))
            .await
            .expect("call save_note through observer");
    let structured = response
        .structured_content
        .as_ref()
        .expect("client retains structured MCP result");
    assert_eq!(structured["nonce"], "observer-transaction-0.54");
    assert_eq!(structured["note_count"], 1);
    let client_result = serde_json::to_value(&response).expect("serialize client MCP result");
    client.cancel().await.expect("shut observed fixture down");

    let records = fs::read_to_string(&evidence)
        .expect("read observer evidence")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse observer evidence line"))
        .collect::<Vec<_>>();
    let request = records
        .iter()
        .find(|record| record["phase"] == "request")
        .expect("observed tools/call request");
    assert_eq!(request["server"], "notes");
    assert_eq!(request["tool"], "save_note");
    assert_eq!(request["arguments"]["nonce"], "observer-transaction-0.54");
    assert!(
        request["wire_sha256"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let response = records
        .iter()
        .find(|record| record["phase"] == "response")
        .expect("observed tools/call response");
    assert_eq!(response["server"], "notes");
    assert_eq!(response["tool"], "save_note");
    assert_eq!(
        response["result"], client_result,
        "observer evidence and client must receive the same MCP result"
    );
    assert_eq!(
        response["result"]["structuredContent"]["nonce"],
        "observer-transaction-0.54"
    );
    assert!(response["error"].is_null());
    assert!(
        response["wire_sha256"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );
    let written: Value =
        serde_json::from_slice(&fs::read(&receipt).expect("read receipt")).expect("parse receipt");
    assert_eq!(written["nonce"], "observer-transaction-0.54");
    assert_eq!(written["text"], "exact MCP result survives");
    assert!(!poison.exists(), "observer must not invoke another tool");

    fs::remove_dir_all(root).expect("remove fixture root");
}
