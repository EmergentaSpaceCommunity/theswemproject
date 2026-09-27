//! Arbitrary third-party MCP Apps fixture for the generic Apps-host gate.
//!
//! The server owns no SWEM Cycle state and models a domain SWEM never heard
//! of (a note board). It exposes one MCP App resource
//! (`ui://apps-fixture/notes`, `text/html;profile=mcp-app`) and terminal-state
//! fixture tools in addition to the ordinary note tools:
//! `save_note` (visible to model and app; writes an external receipt so the
//! host gate is proven by a file, never by prose),
//! `acknowledge_observed_call` (App-only browser oracle) and `model_only_probe`
//! (visibility `["model"]` only; if it ever executes it writes a POISON
//! receipt - the absence of that file is the durable witness that the host
//! refused an app-originated call instead of getting lucky).
//!
//! `--hostile` serves a hostile App at the same URI which attempts undeclared
//! and model-only tool calls, an external fetch, a `window.top` escape and
//! malformed messages; every attempt must be observably blocked by the host.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use rmcp::handler::server::tool::ToolCallContext;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorData,
    Implementation, JsonObject, ListResourcesResult, ListToolsResult, MetaObject,
    PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    Resource, ResourceContents, ServerCapabilities, ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::json;

const NOTES_RESOURCE: &str = "ui://apps-fixture/notes";
/// Listed only with `--engine-probe`: a View that measures what the host's
/// real sandbox lets an App use (WASM, `AudioWorklet`, workers, audio, GPU,
/// MIDI, isolation) and persists the facts through an App-only tool.
const PROBE_RESOURCE: &str = "ui://apps-fixture/engine-probe";
/// Listed only with `--engine-probe`: a worklet script the probe View loads
/// from the sandbox origin (a served `ui://` resource, never `blob:`).
const PROBE_WORKLET_RESOURCE: &str = "ui://apps-fixture/probe-worklet.js";
const PROBE_WORKLET_JS: &str = "registerProcessor('swem-probe-served', class extends AudioWorkletProcessor { process() { return true; } });\n\
registerProcessor('swem-probe-capture', class extends AudioWorkletProcessor { process(inputs) { const input = inputs[0] && inputs[0][0]; let peak = 0; if (input) { for (const value of input) { const magnitude = value < 0 ? -value : value; if (magnitude > peak) peak = magnitude; } } this.port.postMessage({kind: 'peak', peak}); return true; } });\n";
/// Listed only with `--engine-probe`: a View that asks the host for a real
/// origin (`_meta.ui.origin: "isolated"`) and measures what that buys -
/// storage, isolation, workers and worklets from files under its own path,
/// wasm by media type, the microphone from the View itself.
const ISOLATED_PROBE_RESOURCE: &str = "ui://apps-fixture/isolated-probe";
/// A module worker the isolated probe loads as `./probe-worker.js`.
const PROBE_WORKER_RESOURCE: &str = "ui://apps-fixture/probe-worker.js";
const PROBE_WORKER_JS: &str = "self.onmessage = (event) => { self.postMessage({echo: event.data, isolated: Boolean(self.crossOriginIsolated), shared_array_buffer: typeof SharedArrayBuffer === 'function'}); };\n";
/// A wasm module the isolated probe fetches as `./probe.wasm`: the smallest
/// valid module (magic and version, no sections), served as a blob.
const PROBE_WASM_RESOURCE: &str = "ui://apps-fixture/probe.wasm";
const PROBE_WASM_BASE64: &str = "AGFzbQEAAAA=";
const WASM_MIME: &str = "application/wasm";
const SCRIPT_MIME: &str = "text/javascript";
const APP_MIME: &str = "text/html;profile=mcp-app";
const NOTES_HTML: &str = include_str!("swem_mcp_apps_fixture/notes.html");
const HOSTILE_HTML: &str = include_str!("swem_mcp_apps_fixture/notes-hostile.html");
const PROBE_HTML: &str = include_str!("swem_mcp_apps_fixture/engine-probe.html");
const ISOLATED_PROBE_HTML: &str = include_str!("swem_mcp_apps_fixture/isolated-probe.html");

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct SaveNoteRequest {
    nonce: String,
    text: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct DelayedNoteRequest {
    nonce: String,
    text: String,
    delay_ms: u64,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct SaveNoteResponse {
    saved: bool,
    nonce: String,
    note_count: usize,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct ProbeRequest {
    nonce: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct ProbeResponse {
    executed: bool,
    nonce: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct AcknowledgeObservedRequest {
    input_nonce: String,
    input_text: String,
    result_nonce: String,
    result_count: usize,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct AcknowledgeObservedResponse {
    acknowledged: bool,
}

/// The probe's measurements as one JSON-encoded string: App-only tools are
/// offered by the generic shell as flat structured forms, so their inputs
/// stay primitive like every other fixture receipt tool.
#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct EngineProbeRequest {
    facts_json: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct AcknowledgeTerminalRequest {
    input_nonce: String,
    terminal: String,
    reason: Option<String>,
}

fn save_note_meta() -> MetaObject {
    let mut meta = JsonObject::new();
    meta.insert(
        "ui".into(),
        json!({"resourceUri": NOTES_RESOURCE, "visibility": ["model", "app"]}),
    );
    MetaObject(meta)
}

fn model_only_meta() -> MetaObject {
    let mut meta = JsonObject::new();
    meta.insert("ui".into(), json!({"visibility": ["model"]}));
    MetaObject(meta)
}

fn app_only_meta() -> MetaObject {
    let mut meta = JsonObject::new();
    meta.insert("ui".into(), json!({"visibility": ["app"]}));
    MetaObject(meta)
}

#[derive(Debug)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one fixture mirrors its command line flag by flag"
)]
struct NotesServer {
    receipt: PathBuf,
    poison: PathBuf,
    observed_receipt: Option<PathBuf>,
    terminal_receipt: Option<PathBuf>,
    delay_release: Option<PathBuf>,
    probe_receipt: Option<PathBuf>,
    hostile: bool,
    omit_resource_listing: bool,
    engine_probe: bool,
    /// `--home`: the notes View is this server's home App, marked on the
    /// resource so a host shows it as a space of its own.
    home: bool,
    /// `--late-tool <file>`: once the file exists the server declares one
    /// more tool, `late_note` - a server that gains a tool while it runs,
    /// as a hub does when a package is installed through it.
    late_tool: Option<PathBuf>,
    notes: Mutex<Vec<(String, String)>>,
    tool_router: ToolRouter<Self>,
}

/// The tool a `--late-tool` server declares once its file exists.
const LATE_TOOL: &str = "late_note";

impl NotesServer {
    #[allow(
        clippy::too_many_arguments,
        reason = "one fixture constructor mirrors its command line flag by flag"
    )]
    fn new(
        receipt: PathBuf,
        poison: PathBuf,
        observed_receipt: Option<PathBuf>,
        terminal_receipt: Option<PathBuf>,
        delay_release: Option<PathBuf>,
        probe_receipt: Option<PathBuf>,
        hostile: bool,
        omit_resource_listing: bool,
        home: bool,
        late_tool: Option<PathBuf>,
    ) -> Self {
        Self {
            receipt,
            poison,
            observed_receipt,
            terminal_receipt,
            delay_release,
            engine_probe: probe_receipt.is_some(),
            probe_receipt,
            hostile,
            omit_resource_listing,
            home,
            late_tool,
            notes: Mutex::new(Vec::new()),
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl NotesServer {
    /// Save one note and persist an out-of-band invocation receipt.
    #[tool(
        description = "Save a note to the board and record the exact nonce in an external receipt",
        meta = save_note_meta()
    )]
    fn save_note(
        &self,
        Parameters(SaveNoteRequest { nonce, text }): Parameters<SaveNoteRequest>,
    ) -> Result<Json<SaveNoteResponse>, String> {
        let note_count = {
            let mut notes = self.notes.lock().map_err(|error| error.to_string())?;
            notes.push((nonce.clone(), text.clone()));
            notes.len()
        };
        let receipt = json!({
            "tool": "save_note",
            "nonce": nonce,
            "text": text,
            "note_count": note_count,
        });
        let bytes = serde_json::to_vec(&receipt).map_err(|error| error.to_string())?;
        fs::write(&self.receipt, bytes).map_err(|error| error.to_string())?;
        Ok(Json(SaveNoteResponse {
            saved: true,
            nonce,
            note_count,
        }))
    }

    /// Return a standards-shaped MCP tool execution error. The JSON-RPC
    /// request succeeds and its `CallToolResult.isError` remains visible to
    /// both the native agent and the App.
    #[tool(
        description = "Fail one note operation with an actionable tool execution error",
        meta = save_note_meta()
    )]
    fn fail_note(
        &self,
        Parameters(SaveNoteRequest { nonce, .. }): Parameters<SaveNoteRequest>,
    ) -> Result<Json<SaveNoteResponse>, String> {
        if self.receipt.as_os_str().is_empty() {
            return Err("fixture receipt path is not configured".to_owned());
        }
        Err(format!("fixture rejected note {nonce}"))
    }

    /// Complete after a bounded delay. Browser teardown during this call must
    /// not cancel it because the original tools/call belongs to the agent.
    #[tool(
        description = "Save a note after a bounded delay for App teardown race conformance",
        meta = save_note_meta()
    )]
    async fn delayed_note(
        &self,
        Parameters(DelayedNoteRequest {
            nonce,
            text,
            delay_ms,
        }): Parameters<DelayedNoteRequest>,
    ) -> Result<Json<SaveNoteResponse>, String> {
        if let Some(release) = &self.delay_release {
            // Say that the call has arrived before waiting for the release.
            // A test that wants to time a second request against a call in
            // flight otherwise has to guess when this one started.
            let waiting = release.with_extension("waiting");
            fs::write(&waiting, nonce.as_bytes()).map_err(|error| error.to_string())?;
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                while !release.is_file() {
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .map_err(|_| "fixture release receipt did not arrive".to_owned())?;
        } else {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms.min(5_000))).await;
        }
        let receipt = json!({
            "tool": "delayed_note",
            "nonce": nonce,
            "text": text,
            "note_count": 1,
        });
        fs::write(
            &self.receipt,
            serde_json::to_vec(&receipt).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(Json(SaveNoteResponse {
            saved: true,
            nonce,
            note_count: 1,
        }))
    }

    /// Stay in flight until the MCP client sends `notifications/cancelled`.
    /// The start receipt proves the request reached the server before the
    /// native client, rather than Workbench, cancelled it.
    #[tool(
        description = "Start a note operation that waits for native MCP cancellation",
        meta = save_note_meta()
    )]
    async fn cancellable_note(
        &self,
        Parameters(SaveNoteRequest { nonce, text }): Parameters<SaveNoteRequest>,
    ) -> Result<Json<SaveNoteResponse>, String> {
        let started = json!({
            "tool": "cancellable_note",
            "nonce": nonce,
            "text": text,
            "state": "started",
        });
        fs::write(
            &self.receipt,
            serde_json::to_vec(&started).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        std::future::pending::<()>().await;
        unreachable!("the cancellable fixture only terminates by request cancellation")
    }

    /// Model-only tool: executing it from an App means the host gate failed.
    #[tool(
        description = "Diagnostic probe visible to the model only; never callable from an App",
        meta = model_only_meta()
    )]
    fn model_only_probe(
        &self,
        Parameters(ProbeRequest { nonce }): Parameters<ProbeRequest>,
    ) -> Result<Json<ProbeResponse>, String> {
        let poison = json!({"tool": "model_only_probe", "nonce": nonce});
        let bytes = serde_json::to_vec(&poison).map_err(|error| error.to_string())?;
        fs::write(&self.poison, bytes).map_err(|error| error.to_string())?;
        Ok(Json(ProbeResponse {
            executed: true,
            nonce,
        }))
    }

    /// Browser oracle for the host-delivered tool input and result. This tool
    /// is App-only, so the native model cannot manufacture the receipt.
    #[tool(
        description = "Acknowledge the exact agent-call input and result delivered to this App",
        meta = app_only_meta()
    )]
    fn acknowledge_observed_call(
        &self,
        Parameters(request): Parameters<AcknowledgeObservedRequest>,
    ) -> Result<Json<AcknowledgeObservedResponse>, String> {
        let path = self
            .observed_receipt
            .as_ref()
            .ok_or_else(|| "observed-call receipt is not configured".to_owned())?;
        let bytes = serde_json::to_vec(&request).map_err(|error| error.to_string())?;
        fs::write(path, bytes).map_err(|error| error.to_string())?;
        Ok(Json(AcknowledgeObservedResponse { acknowledged: true }))
    }

    /// Browser oracle for terminal notifications delivered by `AppBridge`. The
    /// JSONL receipt is separate from model/tool state and can only be reached
    /// through this App-visible tool.
    #[tool(
        description = "Acknowledge an exact error or cancellation notification delivered to this App",
        meta = app_only_meta()
    )]
    fn acknowledge_terminal(
        &self,
        Parameters(request): Parameters<AcknowledgeTerminalRequest>,
    ) -> Result<Json<AcknowledgeObservedResponse>, String> {
        let path = self
            .terminal_receipt
            .as_ref()
            .ok_or_else(|| "terminal receipt is not configured".to_owned())?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|error| error.to_string())?;
        serde_json::to_writer(&mut file, &request).map_err(|error| error.to_string())?;
        file.write_all(b"\n").map_err(|error| error.to_string())?;
        Ok(Json(AcknowledgeObservedResponse { acknowledged: true }))
    }

    /// Persist what the engine-probe App measured inside the host's sandbox.
    /// App-only: the facts come from the View that ran in the real iframe.
    #[tool(
        description = "Acknowledge the engine capabilities the probe App measured",
        meta = app_only_meta()
    )]
    fn acknowledge_engine_probe(
        &self,
        Parameters(request): Parameters<EngineProbeRequest>,
    ) -> Result<Json<AcknowledgeObservedResponse>, String> {
        let path = self
            .probe_receipt
            .as_ref()
            .ok_or_else(|| "engine probe receipt is not configured".to_owned())?;
        let facts: serde_json::Value =
            serde_json::from_str(&request.facts_json).map_err(|error| error.to_string())?;
        let bytes = serde_json::to_vec_pretty(&facts).map_err(|error| error.to_string())?;
        fs::write(path, bytes).map_err(|error| error.to_string())?;
        Ok(Json(AcknowledgeObservedResponse { acknowledged: true }))
    }
}

impl NotesServer {
    fn late_tool_arrived(&self) -> bool {
        self.late_tool.as_ref().is_some_and(|path| path.is_file())
    }
}

impl ServerHandler for NotesServer {
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut tools = self.tool_router.list_all();
        if self.late_tool_arrived() {
            tools.push(Tool::new(
                LATE_TOOL,
                "A tool this server declared after it was dialled",
                std::sync::Arc::new(
                    json!({"type": "object", "properties": {}})
                        .as_object()
                        .cloned()
                        .unwrap_or_default(),
                ),
            ));
        }
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        if request.name == LATE_TOOL {
            return Ok(CallToolResponse::Complete(if self.late_tool_arrived() {
                CallToolResult::success(vec![ContentBlock::text("the late tool answered")])
            } else {
                CallToolResult::error(vec![ContentBlock::text("no such tool yet")])
            }));
        }
        let call = ToolCallContext::new(self, request, context);
        self.tool_router.call(call).await
    }

    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new(
            "swem-mcp-apps-fixture",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions("Cycle-independent note board with one MCP App view")
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        if self.omit_resource_listing {
            // Tool metadata is still the normative Apps discovery source.
            return Ok(ListResourcesResult::with_all_items(Vec::new()));
        }
        // Deliberately NO `_meta.ui` at listing level: the app metadata lives
        // on the read content item only, exercising the host's spec duty to
        // check both locations with content winning.
        let mut resource = Resource::new(NOTES_RESOURCE, "notes");
        resource.mime_type = Some(APP_MIME.into());
        resource.description = Some("Note board MCP App view".into());
        if self.home {
            let mut meta = JsonObject::new();
            meta.insert("swem/home".into(), json!(true));
            resource.meta = Some(MetaObject(meta));
        }
        let mut resources = vec![resource];
        if self.engine_probe {
            let mut probe = Resource::new(PROBE_RESOURCE, "engine-probe");
            probe.mime_type = Some(APP_MIME.into());
            probe.description = Some("Engine capability probe MCP App view".into());
            resources.push(probe);
            let mut worklet = Resource::new(PROBE_WORKLET_RESOURCE, "probe-worklet");
            worklet.mime_type = Some(SCRIPT_MIME.into());
            worklet.description =
                Some("AudioWorklet module the probe loads from the sandbox origin".into());
            resources.push(worklet);
            let mut isolated = Resource::new(ISOLATED_PROBE_RESOURCE, "isolated-probe");
            isolated.mime_type = Some(APP_MIME.into());
            isolated.description = Some("Isolated-origin probe MCP App view".into());
            resources.push(isolated);
            let mut worker = Resource::new(PROBE_WORKER_RESOURCE, "probe-worker");
            worker.mime_type = Some(SCRIPT_MIME.into());
            worker.description =
                Some("Module worker the isolated probe loads by relative URL".into());
            resources.push(worker);
            let mut wasm = Resource::new(PROBE_WASM_RESOURCE, "probe-wasm");
            wasm.mime_type = Some(WASM_MIME.into());
            wasm.description =
                Some("A wasm module the isolated probe fetches by relative URL".into());
            resources.push(wasm);
        }
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        if request.uri == PROBE_WORKLET_RESOURCE && self.engine_probe {
            return Ok(
                ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: request.uri,
                    mime_type: Some(SCRIPT_MIME.into()),
                    text: PROBE_WORKLET_JS.to_owned(),
                    meta: None,
                }])
                .into(),
            );
        }
        if request.uri == PROBE_WORKER_RESOURCE && self.engine_probe {
            return Ok(
                ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                    uri: request.uri,
                    mime_type: Some(SCRIPT_MIME.into()),
                    text: PROBE_WORKER_JS.to_owned(),
                    meta: None,
                }])
                .into(),
            );
        }
        if request.uri == PROBE_WASM_RESOURCE && self.engine_probe {
            return Ok(
                ReadResourceResult::new(vec![ResourceContents::BlobResourceContents {
                    uri: request.uri,
                    mime_type: Some(WASM_MIME.into()),
                    blob: PROBE_WASM_BASE64.to_owned(),
                    meta: None,
                }])
                .into(),
            );
        }
        let probe = request.uri == PROBE_RESOURCE && self.engine_probe;
        let isolated = request.uri == ISOLATED_PROBE_RESOURCE && self.engine_probe;
        if request.uri != NOTES_RESOURCE && !probe && !isolated {
            return Err(ErrorData::resource_not_found(
                format!("unknown resource: {}", request.uri),
                None,
            ));
        }
        let mut meta = JsonObject::new();
        if isolated {
            // The View asks for a real origin, a permission and a CSP block
            // whose only origin is its own (files under its path).
            meta.insert(
                "ui".into(),
                json!({
                    "prefersBorder": true,
                    "origin": "isolated",
                    "permissions": {"microphone": {}},
                    "csp": {"connectDomains": []}
                }),
            );
        } else if probe {
            // The probe declares one permission so the gate can measure that
            // declared permissions reach the View's Permissions Policy, and a
            // CSP block with no foreign origin so its own sandbox origin is
            // reachable by `fetch` (the upload route lives there).
            meta.insert(
                "ui".into(),
                json!({
                    "prefersBorder": true,
                    "permissions": {"microphone": {}},
                    "csp": {"connectDomains": []}
                }),
            );
        } else {
            meta.insert("ui".into(), json!({"prefersBorder": true}));
        }
        let html = if isolated {
            ISOLATED_PROBE_HTML
        } else if probe {
            PROBE_HTML
        } else if self.hostile {
            HOSTILE_HTML
        } else {
            NOTES_HTML
        };
        Ok(
            ReadResourceResult::new(vec![ResourceContents::TextResourceContents {
                uri: request.uri,
                mime_type: Some(APP_MIME.into()),
                text: html.to_owned(),
                meta: Some(MetaObject(meta)),
            }])
            .into(),
        )
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let path_after = |flag: &str| {
        arguments
            .windows(2)
            .find(|pair| pair[0] == flag)
            .map(|pair| PathBuf::from(&pair[1]))
    };
    let receipt = path_after("--receipt")
        .ok_or("usage: swem-mcp-apps-fixture --receipt <abs> --poison <abs> [--hostile]")?;
    let poison = path_after("--poison")
        .ok_or("usage: swem-mcp-apps-fixture --receipt <abs> --poison <abs> [--hostile]")?;
    let observed_receipt = path_after("--observed-receipt");
    let terminal_receipt = path_after("--terminal-receipt");
    let delay_release = path_after("--delay-release");
    let probe_receipt = path_after("--probe-receipt");
    if probe_receipt
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err("probe receipt path must be absolute".into());
    }
    if !receipt.is_absolute() || !poison.is_absolute() {
        return Err("receipt and poison paths must be absolute".into());
    }
    let hostile = arguments.iter().any(|argument| argument == "--hostile");
    let home = arguments.iter().any(|argument| argument == "--home");
    let late_tool = arguments
        .iter()
        .position(|argument| argument == "--late-tool")
        .and_then(|index| arguments.get(index + 1))
        .map(PathBuf::from);
    let omit_resource_listing = arguments
        .iter()
        .any(|argument| argument == "--omit-resource-listing");
    if observed_receipt
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err("observed receipt path must be absolute".into());
    }
    if terminal_receipt
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err("terminal receipt path must be absolute".into());
    }
    if delay_release
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err("delay release path must be absolute".into());
    }
    NotesServer::new(
        receipt,
        poison,
        observed_receipt,
        terminal_receipt,
        delay_release,
        probe_receipt,
        hostile,
        omit_resource_listing,
        home,
        late_tool,
    )
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}
