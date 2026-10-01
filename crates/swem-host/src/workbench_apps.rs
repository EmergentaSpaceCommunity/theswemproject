//! Generic MCP Apps discovery and relay for the Workbench shell (gate H1).
//!
//! The host opens its own stdio MCP client per profile attachment and proxies
//! View<->server over that connection. The agent's MCP children stay
//! agent-owned, so this is deliberately an independent connection/process:
//! arbitrary server state is NOT assumed to be shared with the agent-side
//! connection. A shared attachment broker remains a separate HC0 boundary.
//! The host discovers Apps the standard
//! SEP-1865 way (tool `_meta.ui.resourceUri` plus `ui://` resources with the
//! `text/html;profile=mcp-app` MIME) and relays bare JSON-RPC from the App
//! iframe with every security decision made HERE, in Rust: method allowlist
//! (`tools/call`, `tools/list`, `resources/read` - never any `ui/*`), tool
//! visibility (`"app"` required), and cross-server isolation by construction
//! (one app id holds one server's client). Nothing is copied into a host
//! registry: discovery results are live projections of the attached server.

use base64::Engine as _;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hyper::Uri;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use rmcp::ServiceExt as _;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientConfig,
    ElicitRequestParams, ElicitResult, ElicitationAction, GetTaskParams, Implementation,
    InputRequest, JsonObject, ProtocolVersion, ReadResourceRequestParams, TaskPayload,
};
use rmcp::service::{RoleClient, RunningService, RxJsonRpcMessage, TxJsonRpcMessage};
use rmcp::transport::{TokioChildProcess, Transport};
use serde::Serialize;
use serde_json::{Value, json};

use crate::ResolvedMcpAttachment;
use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};

#[cfg(windows)]
use process_wrap::tokio::JobObject;
#[cfg(unix)]
use process_wrap::tokio::ProcessGroup;

/// MIME type mandated by the MCP Apps extension for HTML views.
pub const MCP_APP_MIME: &str = "text/html;profile=mcp-app";

/// Spec default CSP when the App declares no `_meta.ui.csp`, plus the one
/// host decision the spec's declaration format cannot express: the CSP3
/// keyword `'wasm-unsafe-eval'`. `'unsafe-inline'` already lets an App run
/// arbitrary JavaScript, so compiling WebAssembly adds no capability; without
/// the keyword no App can instantiate a content-addressed wasm artifact (the
/// 0.71 engine probe measured exactly that). `blob:` and `worker-src` stay
/// absent here: worklet scripts are served `ui://` resources on the sandbox
/// origin, which `'self'` already covers. An App on a real origin
/// (`_meta.ui.origin: "isolated"`) additionally gets `worker-src 'self'
/// blob:` - see [`ISOLATED_APP_CSP_EXTRA`].
const DEFAULT_APP_CSP: &str = "default-src 'none'; \
     script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data:; media-src 'self' data:; \
     connect-src 'none'; object-src 'none'";
/// What a View on its own real origin gets on top: it may embed frames of
/// its own origin, and run workers it makes itself. A mature audio or data
/// library drains a `SharedArrayBuffer` ring or decodes off the main thread
/// on a worker built from a `blob:` URL of its own code; on an isolated
/// origin that worker is the App's own code on the App's own origin, so no
/// capability leaves the sandbox. An opaque `srcdoc` View has no origin a
/// blob could belong to and keeps the default policy.
const ISOLATED_APP_CSP_EXTRA: &str = "frame-src 'self'; worker-src 'self' blob:";
/// MIME of a script resource an App may load into a worklet or worker.
pub const SCRIPT_RESOURCE_MIME: &str = "text/javascript";

/// One tool of an attached server, as the discovery panel shows it.
#[derive(Clone, Debug, Serialize)]
pub struct DiscoveredAppTool {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub resource_uri: Option<String>,
    pub visibility: Vec<String>,
}

/// One standard form elicitation waiting for an operator response. The opaque
/// MRTR request state and original tool arguments never leave the Rust host.
#[derive(Clone, Debug, Serialize)]
pub struct PendingElicitationView {
    pub interaction_id: String,
    pub server_name: String,
    pub tool: String,
    pub connection_scope: &'static str,
    pub message: String,
    pub requested_schema: Value,
}

struct PendingInteraction {
    view: PendingElicitationView,
    request: CallToolRequestParams,
    response_key: String,
    request_state: Option<String>,
}

/// One `ui://` App resource of an attached server.
#[derive(Clone, Debug, Serialize)]
pub struct DiscoveredAppResource {
    pub uri: String,
    pub mime: String,
    pub description: Option<String>,
    /// The server's home App: the surface the host shows as a space of its
    /// own, opened without a tool call, marked `_meta["swem/home"]: true`
    /// on the resource (a vendor key inside the specification's metadata).
    pub home: bool,
}

/// Discovery projection of one attachment for the shell page.
#[derive(Clone, Debug, Serialize)]
pub struct AppAttachmentView {
    pub server_name: String,
    pub transport_supported: bool,
    /// Honest process/connection scope of the current stdio Apps adapter.
    pub connection_scope: &'static str,
    pub tools: Vec<DiscoveredAppTool>,
    pub apps: Vec<DiscoveredAppResource>,
}

pub(crate) struct AppAttachmentEntry {
    pub server_name: String,
    /// Shared, because a relay must be able to hold the client while the
    /// apps lock is back open: a tool that runs for ten seconds otherwise
    /// blocks every other request of the same App, the project envelope's
    /// own `resources/read` among them.
    ///
    /// Private to this module so `take_relay_client` stays the only way out
    /// of it: reaching the client is what puts the lock down.
    client: Option<Arc<HostClient>>,
    exit: Option<ManagedStdioExit>,
    pub tools: Vec<DiscoveredAppTool>,
    /// Tools the server declared after it was dialled, as the relay found
    /// them when an App called one: a server may gain tools while it runs
    /// (a hub installs a package), and `tools` is what it listed at the
    /// start.
    late_tools: std::sync::Mutex<Vec<DiscoveredAppTool>>,
    pub apps: Vec<DiscoveredAppResource>,
    /// Where bytes an App of this server uploads land: the server's own
    /// workspace, as the host that declared the server knows it. `None`
    /// when the host serves this server without a workspace of its own.
    pub upload_root: Option<PathBuf>,
}

/// The upstream child transport cleans up from `Drop` in a detached task.
/// Wrap it without changing its wire protocol so connection teardown can await
/// the real child-process boundary instead of merely awaiting the MCP service.
struct ManagedStdioTransport {
    inner: Option<TokioChildProcess>,
    completion: tokio::sync::watch::Sender<Option<Result<(), String>>>,
}

pub(crate) struct ManagedStdioExit {
    completion: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
}

pub(crate) type HostClient = RunningService<RoleClient, ClientConfig>;

/// Spawn one independent host-side stdio MCP client for a declared server:
/// the child is contained (kill-on-drop, Job Object / process group), the
/// transport is managed so teardown can await the real child boundary, and
/// the client speaks the 2026-07-28 profile. `None` when the process cannot
/// be started or refuses the handshake (fail closed, child already reaped).
pub(crate) async fn spawn_host_client(
    stdio: &McpServerStdio,
) -> Option<(HostClient, ManagedStdioExit)> {
    let mut command = tokio::process::Command::new(&stdio.command);
    command.args(&stdio.args);
    for variable in &stdio.env {
        command.env(&variable.name, &variable.value);
    }
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(windows)]
    command.wrap(JobObject);
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    let transport = TokioChildProcess::new(command).ok()?;
    let (transport, exit) = managed_stdio(transport);
    let info = ClientConfig::new(
        // Tasks, because a person's own call to a long tool should leave the
        // same record an agent's does. Without this the server takes the
        // synchronous path for everything the Workbench asks, the operation
        // is never recorded, and a person who pressed a button sees nothing
        // until it returns. The relay drives the task to its end and
        // returns the tool's own result, so nothing above it changes.
        ClientCapabilities::builder()
            .enable_elicitation()
            .enable_tasks()
            .build(),
        Implementation::new("swem-workbench", env!("CARGO_PKG_VERSION")),
    )
    .with_protocol_version(ProtocolVersion::V_2026_07_28);
    if let Ok(client) = info.serve(transport).await {
        Some((client, exit))
    } else {
        let _ = exit.wait().await;
        None
    }
}

/// Start a server once, list its tools, and stop it: what a host that takes
/// a kind of server checks a candidate's tools against its
/// [`swem_store::Shape`] with. The server is started as a declared server
/// is; nothing of it is kept.
///
/// # Errors
///
/// The program could not be started, refused the handshake, or did not
/// list its tools.
pub async fn tools_listed_by(
    stdio: &McpServerStdio,
) -> Result<Vec<swem_store::ToolListed>, String> {
    let (client, exit) = spawn_host_client(stdio).await.ok_or_else(|| {
        format!(
            "{} could not be started as a server",
            stdio.command.display()
        )
    })?;
    let listed = client.list_all_tools().await;
    let _ = client.cancel().await;
    let _ = exit.wait().await;
    Ok(listed
        .map_err(|error| {
            format!(
                "{} did not list its tools: {error}",
                stdio.command.display()
            )
        })?
        .iter()
        .map(|tool| swem_store::ToolListed {
            name: tool.name.to_string(),
            input_schema: Value::Object((*tool.input_schema).clone()),
        })
        .collect())
}

/// [`tools_listed_by`] from code that is not async: a taker's check runs
/// on a blocking thread of the runtime the product serves on, or on no
/// runtime at all.
///
/// # Errors
///
/// As [`tools_listed_by`].
pub fn tools_listed_by_blocking(
    stdio: &McpServerStdio,
) -> Result<Vec<swem_store::ToolListed>, String> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(tools_listed_by(stdio))),
        Err(_) => tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?
            .block_on(tools_listed_by(stdio)),
    }
}

fn managed_stdio(transport: TokioChildProcess) -> (ManagedStdioTransport, ManagedStdioExit) {
    let (sender, receiver) = tokio::sync::watch::channel(None);
    (
        ManagedStdioTransport {
            inner: Some(transport),
            completion: sender,
        },
        ManagedStdioExit {
            completion: receiver,
        },
    )
}

fn record_stdio_completion(
    completion: &tokio::sync::watch::Sender<Option<Result<(), String>>>,
    result: &std::io::Result<()>,
) {
    let recorded = match result {
        Ok(()) => Ok(()),
        Err(error) => Err(error.to_string()),
    };
    completion.send_replace(Some(recorded));
}

impl ManagedStdioExit {
    pub(crate) async fn wait(mut self) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(result) = self.completion.borrow().clone() {
                    return result;
                }
                self.completion
                    .changed()
                    .await
                    .map_err(|_| "stdio child cleanup ended without a receipt".to_owned())?;
            }
        })
        .await
        .map_err(|_| "stdio child cleanup exceeded five seconds".to_owned())?
    }
}

impl Transport<RoleClient> for ManagedStdioTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: TxJsonRpcMessage<RoleClient>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.inner
            .as_mut()
            .expect("MCP service cannot send after transport close")
            .send(item)
    }

    fn receive(&mut self) -> impl Future<Output = Option<RxJsonRpcMessage<RoleClient>>> + Send {
        self.inner
            .as_mut()
            .expect("MCP service cannot receive after transport close")
            .receive()
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let inner = self.inner.take();
        let completion = self.completion.clone();
        async move {
            let result = match inner {
                Some(mut transport) => transport.graceful_shutdown().await,
                None => Ok(()),
            };
            record_stdio_completion(&completion, &result);
            result
        }
    }
}

impl Drop for ManagedStdioTransport {
    fn drop(&mut self) {
        let Some(mut transport) = self.inner.take() else {
            return;
        };
        let completion = self.completion.clone();
        tokio::spawn(async move {
            let result = transport.graceful_shutdown().await;
            record_stdio_completion(&completion, &result);
        });
    }
}

pub(crate) struct OpenApp {
    pub entry_index: usize,
    pub uri: String,
    /// The View's HTML and the CSP the host resolved for it, kept so the
    /// sandbox origin can serve the View as a document of its own when the
    /// App asked for a real origin.
    pub html: String,
    pub csp: String,
    pub isolated: bool,
}

/// Lazily created per-connection Apps state: host-side clients, discovery
/// projections and the open-app registry.
pub(crate) struct ConnectionApps {
    pub entries: Vec<AppAttachmentEntry>,
    pub open: BTreeMap<String, OpenApp>,
    pub next_app: u64,
    pub next_event: u64,
    pending: BTreeMap<String, PendingInteraction>,
    next_interaction: u64,
}

impl AppAttachmentEntry {
    /// The discovery projection of this one attachment.
    pub(crate) fn view(&self) -> AppAttachmentView {
        AppAttachmentView {
            server_name: self.server_name.clone(),
            transport_supported: self.client.is_some(),
            connection_scope: "independent_host_connection",
            tools: self.tools.clone(),
            apps: self.apps.clone(),
        }
    }

    /// The client a relay runs over, shared: the caller holds it for as long
    /// as its call takes and nothing else waits on that.
    ///
    /// # Errors
    ///
    /// The sentence for an attachment with no supported transport.
    pub(crate) fn relay_client(&self) -> Result<Arc<HostClient>, String> {
        self.client
            .clone()
            .ok_or_else(|| format!("attachment {} has no supported transport", self.server_name))
    }

    /// Cancel the client and await the real child-process boundary.
    ///
    /// Only the last holder can cancel: a relay still in flight owns the
    /// client too, and taking the service out from under it would fail its
    /// call. When that happens the client shuts down on the relay's own
    /// last drop and the wait below still ends on the real child boundary.
    pub(crate) async fn shutdown(self) -> Result<(), String> {
        let mut first_error = None;
        if let Some(client) = self.client.and_then(Arc::into_inner)
            && let Err(error) = client.cancel().await
        {
            first_error = Some(error.to_string());
        }
        if let Some(exit) = self.exit
            && let Err(error) = exit.wait().await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl ConnectionApps {
    pub fn views(&self) -> Vec<AppAttachmentView> {
        self.entries.iter().map(AppAttachmentEntry::view).collect()
    }

    pub async fn shutdown(self) -> Result<(), String> {
        let mut first_error = None;
        for entry in self.entries {
            if let Err(error) = entry.shutdown().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Exact model-visible tool to App-resource links discovered on this
    /// connection. This is the only allowlist the transport observer uses;
    /// unrelated MCP calls never enter the `GenUI` correlation state.
    pub fn model_app_links(&self) -> BTreeMap<(String, String), String> {
        self.entries
            .iter()
            .flat_map(|entry| {
                entry.tools.iter().filter_map(move |tool| {
                    let uri = tool.resource_uri.as_ref()?;
                    tool.visibility
                        .iter()
                        .any(|visibility| visibility == "model")
                        .then(|| ((entry.server_name.clone(), tool.name.clone()), uri.clone()))
                })
            })
            .collect()
    }

    /// Begin one server-declared App-only structured fallback. The first MCP
    /// call must return one form elicitation; completed ordinary tools and
    /// multi-request workflows are refused rather than reinterpreted.
    pub async fn start_interaction(
        &mut self,
        server_name: &str,
        tool_name: &str,
        arguments: JsonObject,
    ) -> Result<PendingElicitationView, String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.server_name == server_name)
            .ok_or_else(|| format!("unknown attachment {server_name}"))?;
        let tool = entry
            .tools
            .iter()
            .find(|tool| tool.name == tool_name)
            .ok_or_else(|| format!("server {server_name} declares no tool {tool_name}"))?;
        if !tool.visibility.iter().any(|value| value == "app")
            || tool.visibility.iter().any(|value| value == "model")
            || tool.resource_uri.is_none()
        {
            return Err(format!(
                "tool {server_name}/{tool_name} is not an App-only structured fallback"
            ));
        }
        let client = entry
            .client
            .as_ref()
            .ok_or_else(|| format!("attachment {server_name} has no supported transport"))?;
        let request = CallToolRequestParams::new(tool_name.to_owned()).with_arguments(arguments);
        let required = match client
            .call_tool_once(request.clone())
            .await
            .map_err(|error| error.to_string())?
        {
            CallToolResponse::InputRequired(required) => required,
            CallToolResponse::Complete(_) => {
                return Err("structured fallback completed without server elicitation".into());
            }
            _ => return Err("structured fallback returned an unsupported MCP result type".into()),
        };
        let requests = required
            .input_requests
            .ok_or_else(|| "input_required contains no inputRequests".to_owned())?;
        if requests.len() != 1 {
            return Err("Workbench v0.1 accepts exactly one input request per interaction".into());
        }
        let (response_key, input_request) = requests
            .into_iter()
            .next()
            .expect("one input request was checked");
        let InputRequest::Elicitation(elicitation) = input_request else {
            return Err("Workbench v0.1 structured fallback accepts only elicitation".into());
        };
        let ElicitRequestParams::FormElicitationParams {
            message,
            requested_schema,
            ..
        } = elicitation.params
        else {
            return Err("Workbench v0.1 does not put URL elicitation in a form".into());
        };
        let interaction_id = format!("i{}", self.next_interaction);
        self.next_interaction += 1;
        let view = PendingElicitationView {
            interaction_id: interaction_id.clone(),
            server_name: server_name.to_owned(),
            tool: tool_name.to_owned(),
            connection_scope: "independent_host_connection",
            message,
            requested_schema: serde_json::to_value(requested_schema)
                .map_err(|error| error.to_string())?,
        };
        self.pending.insert(
            interaction_id,
            PendingInteraction {
                view: view.clone(),
                request,
                response_key,
                request_state: required.request_state,
            },
        );
        Ok(view)
    }

    /// Complete one pending MRTR round with the exact MCP elicitation action.
    /// The interaction is consumed before the second call, so retries cannot
    /// accidentally duplicate a successful mutation.
    pub async fn answer_interaction(
        &mut self,
        interaction_id: &str,
        action: ElicitationAction,
        content: Option<Value>,
    ) -> Result<CallToolResult, String> {
        let pending = self
            .pending
            .remove(interaction_id)
            .ok_or_else(|| format!("unknown or completed interaction {interaction_id}"))?;
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.server_name == pending.view.server_name)
            .ok_or_else(|| format!("attachment {} disappeared", pending.view.server_name))?;
        let client = entry.client.as_ref().ok_or_else(|| {
            format!(
                "attachment {} has no supported transport",
                pending.view.server_name
            )
        })?;
        if action != ElicitationAction::Accept && content.is_some() {
            return Err("decline/cancel elicitation must not carry content".into());
        }
        let result = match content {
            Some(content) => ElicitResult::new(action).with_content(content),
            None => ElicitResult::new(action),
        };
        let mut responses = BTreeMap::new();
        responses.insert(
            pending.response_key,
            serde_json::to_value(result).map_err(|error| error.to_string())?,
        );
        let mut request = pending.request;
        request.input_responses = Some(responses);
        request.request_state = pending.request_state;
        match client
            .call_tool_once(request)
            .await
            .map_err(|error| error.to_string())?
        {
            CallToolResponse::Complete(result) => Ok(result),
            CallToolResponse::InputRequired(_) => {
                Err("Workbench v0.1 does not yet accept a second MRTR round".into())
            }
            _ => Err("structured fallback returned an unsupported MCP result type".into()),
        }
    }
}

/// The vendor marker on an App resource's `_meta` that names it the server's
/// home App. Inside the specification's own metadata, so a host that does not
/// know it sees an ordinary App; it goes when app-only hosts are standardised.
pub const HOME_APP_MARKER: &str = "swem/home";

/// Read the MCP Apps `ui` block from a tool/resource `_meta` value: the
/// nested `"ui"` object is current; the flat `"ui/resourceUri"` key is the
/// deprecated pre-GA form and stays readable.
fn ui_meta(meta: Option<&rmcp::model::MetaObject>) -> (Option<Value>, Option<String>) {
    let Some(meta) = meta else {
        return (None, None);
    };
    let nested = meta.0.get("ui").cloned();
    let flat = meta
        .0
        .get("ui/resourceUri")
        .and_then(Value::as_str)
        .map(str::to_owned);
    (nested, flat)
}

fn tool_view(tool: &rmcp::model::Tool) -> DiscoveredAppTool {
    let (nested, deprecated_flat) = ui_meta(tool.meta.as_ref());
    let resource_uri = nested
        .as_ref()
        .and_then(|ui| ui.get("resourceUri"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or(deprecated_flat);
    let visibility = nested
        .as_ref()
        .and_then(|ui| ui.get("visibility"))
        .and_then(Value::as_array)
        .map_or_else(
            // Spec default when a tool declares no visibility.
            || vec!["model".to_owned(), "app".to_owned()],
            |values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            },
        );
    DiscoveredAppTool {
        name: tool.name.to_string(),
        title: tool.title.clone(),
        description: tool.description.as_deref().map(str::to_owned),
        input_schema: Value::Object((*tool.input_schema).clone()),
        resource_uri,
        visibility,
    }
}

fn discovered_apps(
    tools: &[DiscoveredAppTool],
    listed: impl IntoIterator<Item = rmcp::model::Resource>,
) -> Vec<DiscoveredAppResource> {
    let mut apps = BTreeMap::<String, DiscoveredAppResource>::new();
    for tool in tools {
        let Some(uri) = tool
            .resource_uri
            .as_ref()
            .filter(|uri| uri.starts_with("ui://"))
        else {
            continue;
        };
        // Tool metadata is the primary discovery path. UI-only resources may
        // legally be omitted from resources/list; resources/read validates
        // the exact MIME and URI when the App is opened.
        apps.entry(uri.clone()).or_insert(DiscoveredAppResource {
            uri: uri.clone(),
            mime: MCP_APP_MIME.to_owned(),
            description: None,
            home: false,
        });
    }
    for resource in listed {
        if resource.uri.starts_with("ui://") && resource.mime_type.as_deref() == Some(MCP_APP_MIME)
        {
            let home = resource
                .meta
                .as_ref()
                .and_then(|meta| meta.0.get(HOME_APP_MARKER))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            apps.insert(
                resource.uri.clone(),
                DiscoveredAppResource {
                    uri: resource.uri,
                    mime: MCP_APP_MIME.to_owned(),
                    description: resource.description,
                    home,
                },
            );
        }
    }
    apps.into_values().collect()
}

/// Spawn independent host-side clients for every stdio attachment and project
/// their Apps. This does not claim state identity with the agent-owned MCP
/// process. Non-stdio transports are listed but unsupported (fail closed).
pub(crate) async fn discover(
    attachments: &[ResolvedMcpAttachment],
    upload_root: Option<&Path>,
) -> ConnectionApps {
    let declarations: Vec<_> = attachments
        .iter()
        .map(|attachment| {
            (
                attachment.binding.server_name.clone(),
                &attachment.server,
                upload_root.map(Path::to_path_buf),
            )
        })
        .collect();
    discover_named_servers(&declarations).await
}

async fn discover_named_servers(
    declarations: &[(String, &McpServer, Option<PathBuf>)],
) -> ConnectionApps {
    let mut entries = Vec::with_capacity(declarations.len());
    for (name, declaration, upload_root) in declarations {
        entries.push(discover_server(name.clone(), declaration, upload_root.clone()).await);
    }
    ConnectionApps {
        entries,
        open: BTreeMap::new(),
        next_app: 1,
        next_event: 1,
        pending: BTreeMap::new(),
        next_interaction: 1,
    }
}

/// Dial one declared server for its Apps: its tools, its `ui://` resources
/// and the client a relay runs over. A non-stdio transport is listed but
/// unsupported; a stdio process that cannot be started or refuses the
/// handshake is listed the same way (fail closed), which the caller may
/// choose not to keep.
pub(crate) async fn discover_server(
    server_name: String,
    declaration: &McpServer,
    upload_root: Option<PathBuf>,
) -> AppAttachmentEntry {
    let McpServer::Stdio(stdio) = declaration else {
        return AppAttachmentEntry {
            server_name,
            client: None,
            exit: None,
            tools: Vec::new(),
            late_tools: std::sync::Mutex::default(),
            apps: Vec::new(),
            upload_root,
        };
    };
    let (client, exit) = match spawn_host_client(stdio).await {
        Some((client, exit)) => (Some(Arc::new(client)), Some(exit)),
        None => (None, None),
    };
    let (tools, apps) = match &client {
        Some(client) => {
            let tools: Vec<DiscoveredAppTool> = client
                .list_all_tools()
                .await
                .unwrap_or_default()
                .iter()
                .map(tool_view)
                .collect();
            let listed = client.list_all_resources().await.unwrap_or_default();
            let apps = discovered_apps(&tools, listed);
            (tools, apps)
        }
        None => (Vec::new(), Vec::new()),
    };
    AppAttachmentEntry {
        server_name,
        client,
        exit,
        tools,
        late_tools: std::sync::Mutex::default(),
        apps,
        upload_root,
    }
}

/// The opened App payload handed to the shell page.
#[derive(Clone, Debug, Serialize)]
pub struct OpenedApp {
    pub app_id: String,
    /// Host scope for sandbox script routing, not a native session id.
    /// `project` selects the independent Project Apps registry; other values
    /// select a Workbench connection. Together with `app_id` this addresses
    /// the App's served script resources on the sandbox origin.
    pub connection_id: String,
    pub server_name: String,
    pub uri: String,
    pub html: String,
    pub csp: String,
    pub permissions: Value,
    pub prefers_border: bool,
    /// Where the different-origin sandbox proxy is served, when the serve
    /// layer bound one; `None` in headless/component use.
    pub sandbox_url: Option<String>,
    pub sandbox_origin: Option<String>,
    /// The App asked for a real origin (`_meta.ui.origin: "isolated"`): its
    /// View is a document of the sandbox origin at `view_url`, with storage,
    /// workers and cross-origin isolation, instead of an opaque `srcdoc`.
    pub isolated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_url: Option<String>,
}

/// One App resource as read and resolved: the View's HTML and the metadata
/// the host enforces for it.
pub(crate) struct AppRead {
    pub html: String,
    pub csp: String,
    pub permissions: Value,
    pub prefers_border: bool,
    /// `_meta.ui.origin == "isolated"`, see [`OpenedApp::isolated`].
    pub isolated: bool,
}

/// Read one App resource and resolve its effective metadata: the content-item
/// `_meta.ui` wins over the listing entry per the spec.
///
/// # Errors
///
/// Returns a message when the resource is unknown, not an App, or unreadable.
pub(crate) async fn read_app(entry: &AppAttachmentEntry, uri: &str) -> Result<AppRead, String> {
    if !uri.starts_with("ui://") {
        return Err(format!("not an App resource uri: {uri}"));
    }
    if !entry.apps.iter().any(|app| app.uri == uri) {
        return Err(format!(
            "server {} declares no App {uri}",
            entry.server_name
        ));
    }
    let client = entry.client.as_ref().ok_or_else(|| {
        format!(
            "attachment {} has no supported transport",
            entry.server_name
        )
    })?;
    let read = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .map_err(|error| error.to_string())?;
    let rmcp::model::ResourceContents::TextResourceContents {
        uri: returned_uri,
        mime_type,
        text,
        meta,
    } = read
        .contents
        .first()
        .ok_or_else(|| "App resource returned no content".to_owned())?
    else {
        return Err("App resource must be a text HTML document".into());
    };
    if returned_uri != uri {
        return Err(format!(
            "App resources/read returned {returned_uri} for requested {uri}"
        ));
    }
    if mime_type.as_deref() != Some(MCP_APP_MIME) {
        return Err(format!(
            "App content MIME must be {MCP_APP_MIME}, got {mime_type:?}"
        ));
    }
    let (content_ui, _) = ui_meta(meta.as_ref());
    let isolated = content_ui
        .as_ref()
        .and_then(|ui| ui.get("origin"))
        .and_then(Value::as_str)
        == Some("isolated");
    let csp = resolve_csp(content_ui.as_ref(), isolated)?;
    let permissions = content_ui
        .as_ref()
        .and_then(|ui| ui.get("permissions"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let prefers_border = content_ui
        .as_ref()
        .and_then(|ui| ui.get("prefersBorder"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(AppRead {
        html: text.clone(),
        csp,
        permissions,
        prefers_border,
        isolated,
    })
}

/// The uri of a file next to a View: the View's uri with its last segment
/// replaced by `path`. A path that climbs (`..`) or is empty names nothing.
pub(crate) fn sibling_uri(view_uri: &str, path: &str) -> Option<String> {
    if path.is_empty()
        || path
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    let base = view_uri.rsplit_once('/').map_or(view_uri, |(base, _)| base);
    Some(format!("{base}/{path}"))
}

/// The bytes of one listed resource of an App's server - text or blob -
/// with the media type the server lists it under: what the sandbox origin
/// serves under a View's own path when the View is a document of that
/// origin (its scripts, workers, worklets, wasm and styles by relative URL).
///
/// # Errors
///
/// Returns a diagnostic when the attachment has no transport, the server
/// does not list the uri, the read fails or answers for another uri.
pub(crate) async fn read_file_resource(
    entry: &AppAttachmentEntry,
    uri: &str,
) -> Result<(Vec<u8>, String), String> {
    if !uri.starts_with("ui://") {
        return Err(format!("not a ui resource uri: {uri}"));
    }
    let client = entry.client.as_ref().ok_or_else(|| {
        format!(
            "attachment {} has no supported transport",
            entry.server_name
        )
    })?;
    let listed = client
        .list_all_resources()
        .await
        .map_err(|error| error.to_string())?;
    let Some(media_type) = listed
        .iter()
        .find(|resource| resource.uri == uri)
        .and_then(|resource| resource.mime_type.clone())
    else {
        return Err(format!(
            "server {} lists no resource {uri} with a media type",
            entry.server_name
        ));
    };
    let read = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .map_err(|error| error.to_string())?;
    match read.contents.first() {
        Some(rmcp::model::ResourceContents::TextResourceContents {
            uri: returned_uri,
            text,
            ..
        }) if returned_uri == uri => Ok((text.clone().into_bytes(), media_type)),
        Some(rmcp::model::ResourceContents::BlobResourceContents {
            uri: returned_uri,
            blob,
            ..
        }) if returned_uri == uri => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(blob.as_bytes())
                .map_err(|error| format!("resource {uri} carries malformed base64: {error}"))?;
            Ok((bytes, media_type))
        }
        Some(_) => Err(format!(
            "resources/read answered for another uri than {uri}"
        )),
        None => Err(format!("resource {uri} returned no content")),
    }
}

/// Read one script resource of an App's server for delivery on the sandbox
/// origin: the server must LIST the `ui://` URI with the script MIME (an App
/// cannot fetch arbitrary resources of its server this way), and the read
/// must return text under the same URI and MIME. The bytes are the server's,
/// verbatim; the host adds no policy beyond this allowlist.
///
/// # Errors
///
/// Returns a message when the URI is not a listed script resource, the
/// transport is unsupported, or the read disagrees with the listing.
pub(crate) async fn read_script_resource(
    entry: &AppAttachmentEntry,
    uri: &str,
) -> Result<String, String> {
    if !uri.starts_with("ui://") {
        return Err(format!("not a ui resource uri: {uri}"));
    }
    let client = entry.client.as_ref().ok_or_else(|| {
        format!(
            "attachment {} has no supported transport",
            entry.server_name
        )
    })?;
    let listed = client
        .list_all_resources()
        .await
        .map_err(|error| error.to_string())?;
    if !listed.iter().any(|resource| {
        resource.uri == uri && resource.mime_type.as_deref() == Some(SCRIPT_RESOURCE_MIME)
    }) {
        return Err(format!(
            "server {} lists no {SCRIPT_RESOURCE_MIME} resource {uri}",
            entry.server_name
        ));
    }
    let read = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .map_err(|error| error.to_string())?;
    let rmcp::model::ResourceContents::TextResourceContents {
        uri: returned_uri,
        mime_type,
        text,
        ..
    } = read
        .contents
        .first()
        .ok_or_else(|| "script resource returned no content".to_owned())?
    else {
        return Err("script resource must be text".into());
    };
    if returned_uri != uri {
        return Err(format!(
            "resources/read returned {returned_uri} for requested {uri}"
        ));
    }
    if mime_type.as_deref() != Some(SCRIPT_RESOURCE_MIME) {
        return Err(format!(
            "script resource MIME must be {SCRIPT_RESOURCE_MIME}, got {mime_type:?}"
        ));
    }
    Ok(text.clone())
}

/// The exact bytes of one blob resource the server serves, for delivery on
/// the sandbox origin.
///
/// This widens nothing: the relay already permits `resources/read` for any
/// URI, so an App can already have these bytes - base64 inside a JSON-RPC
/// message, held in memory several times over by every peer on the way. A
/// ten-minute WAV is why that is not good enough, and why the same read is
/// also reachable as an ordinary HTTP response the browser can stream and
/// cache. The server decides what it will serve, exactly as before.
///
/// # Errors
///
/// Returns a diagnostic when the attachment has no transport, the read
/// fails, the server answers for another URI, or the content is not a blob.
pub(crate) async fn read_blob_resource(
    entry: &AppAttachmentEntry,
    uri: &str,
) -> Result<(Vec<u8>, String), String> {
    let client = entry.client.as_ref().ok_or_else(|| {
        format!(
            "attachment {} has no supported transport",
            entry.server_name
        )
    })?;
    let read = client
        .read_resource(ReadResourceRequestParams::new(uri))
        .await
        .map_err(|error| error.to_string())?;
    let rmcp::model::ResourceContents::BlobResourceContents {
        uri: returned_uri,
        mime_type,
        blob,
        ..
    } = read
        .contents
        .first()
        .ok_or_else(|| "resource returned no content".to_owned())?
    else {
        return Err(format!("resource {uri} is not a blob"));
    };
    if returned_uri != uri {
        return Err(format!(
            "resources/read returned {returned_uri} for requested {uri}"
        ));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob.as_bytes())
        .map_err(|error| format!("resource {uri} carries malformed base64: {error}"))?;
    Ok((
        bytes,
        mime_type
            .clone()
            .unwrap_or_else(|| "application/octet-stream".to_owned()),
    ))
}

/// Build the effective CSP from `_meta.ui.csp` per the spec's construction
/// formula, or the spec default block when nothing is declared. Undeclared
/// domains stay blocked; the host never loosens.
fn validated_csp_sources(csp: &Value, key: &str) -> Result<Vec<String>, String> {
    let Some(value) = csp.get(key) else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("MCP App CSP {key} must be an array of origins"))?;
    values
        .iter()
        .map(|value| {
            let source = value
                .as_str()
                .ok_or_else(|| format!("MCP App CSP {key} contains a non-string origin"))?;
            if !source.is_ascii()
                || source.is_empty()
                || source.bytes().any(|byte| {
                    byte.is_ascii_whitespace()
                        || byte.is_ascii_control()
                        || matches!(byte, b';' | b'\'' | b'\"')
                })
            {
                return Err(format!("MCP App CSP {key} contains an invalid origin"));
            }
            let uri = source
                .parse::<Uri>()
                .map_err(|_| format!("MCP App CSP {key} contains an invalid origin"))?;
            let scheme = uri
                .scheme_str()
                .ok_or_else(|| format!("MCP App CSP {key} origin has no scheme"))?;
            let allowed_scheme = match key {
                "connectDomains" => matches!(scheme, "http" | "https" | "ws" | "wss"),
                _ => matches!(scheme, "http" | "https"),
            };
            if !allowed_scheme {
                return Err(format!(
                    "MCP App CSP {key} uses unsupported scheme {scheme}"
                ));
            }
            let authority = uri
                .authority()
                .ok_or_else(|| format!("MCP App CSP {key} origin has no authority"))?;
            if authority.as_str().contains('@') {
                return Err(format!("MCP App CSP {key} origin contains user info"));
            }
            let host = authority.host();
            if host.is_empty()
                || host.strip_prefix("*.").is_some_and(str::is_empty)
                || host.matches('*').count() > usize::from(host.starts_with("*."))
            {
                return Err(format!("MCP App CSP {key} contains an invalid wildcard"));
            }
            if uri
                .path_and_query()
                .is_some_and(|path| path.as_str() != "/")
            {
                return Err(format!(
                    "MCP App CSP {key} must contain origins, not URL paths"
                ));
            }
            Ok(source.to_owned())
        })
        .collect()
}

///
/// An App served as a document of its own origin (`isolated`) is framed by
/// the sandbox proxy under this same policy, so `frame-src` admits `'self'`
/// for it: the proxy may frame the View, and the View its own origin's
/// documents, nothing foreign.
fn resolve_csp(ui: Option<&Value>, isolated: bool) -> Result<String, String> {
    let Some(csp) = ui
        .and_then(|ui| ui.get("csp"))
        .filter(|csp| csp.is_object())
    else {
        return Ok(if isolated {
            format!("{DEFAULT_APP_CSP}; {ISOLATED_APP_CSP_EXTRA}")
        } else {
            DEFAULT_APP_CSP.to_owned()
        });
    };
    let resource_domains = validated_csp_sources(csp, "connectDomains")?.join(" ");
    let asset_domains = validated_csp_sources(csp, "resourceDomains")?.join(" ");
    let frame_domains = validated_csp_sources(csp, "frameDomains")?.join(" ");
    let base_domains = validated_csp_sources(csp, "baseUriDomains")?.join(" ");
    let with = |base: &str, extra: &str| {
        if extra.is_empty() {
            base.to_owned()
        } else {
            format!("{base} {extra}")
        }
    };
    Ok(format!(
        "default-src 'none'; script-src {script}; style-src {style}; img-src {img}; \
         media-src {media}; font-src {font}; connect-src {connect}; frame-src {frame}; \
         base-uri {base}; object-src 'none'{worker}",
        script = with("'self' 'unsafe-inline' 'wasm-unsafe-eval'", &asset_domains),
        style = with("'self' 'unsafe-inline'", &asset_domains),
        img = with("'self' data:", &asset_domains),
        media = with("'self' data:", &asset_domains),
        font = with("'self'", &asset_domains),
        // A declared CSP block always admits the App's own origin: that is
        // where the host serves its script resources, its blobs and takes
        // its uploads. Only the default (undeclared) policy keeps
        // `connect-src 'none'`.
        connect = with("'self'", &resource_domains),
        frame = match (isolated, frame_domains.is_empty()) {
            (false, true) => "'none'".to_owned(),
            (false, false) => frame_domains.clone(),
            (true, true) => "'self'".to_owned(),
            (true, false) => format!("'self' {frame_domains}"),
        },
        worker = if isolated {
            "; worker-src 'self' blob:"
        } else {
            ""
        },
        base = if base_domains.is_empty() {
            "'self'".to_owned()
        } else {
            base_domains.clone()
        },
    ))
}

/// Why the relay refused an App-originated message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RelayRefusal {
    MethodNotAllowed(String),
    ToolNotDeclared(String),
    ToolNotAppVisible(String),
}

impl RelayRefusal {
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::MethodNotAllowed(method) => {
                format!("method {method} is never forwarded to the server")
            }
            Self::ToolNotDeclared(tool) => {
                format!("tool {tool} is not declared by this App's server")
            }
            Self::ToolNotAppVisible(tool) => {
                format!("tool {tool} is not visible to Apps")
            }
        }
    }
}

/// The host's relay gate: which App-originated JSON-RPC request may reach the
/// server. Cross-server calls are impossible by construction (one app id maps
/// to one server's client), so the gate only reasons about THIS server.
pub(crate) fn allow_relay(
    entry: &AppAttachmentEntry,
    method: &str,
    tool_name: Option<&str>,
) -> Result<(), RelayRefusal> {
    match method {
        "tools/list" | "resources/read" => Ok(()),
        "tools/call" => {
            let tool_name = tool_name.unwrap_or_default();
            let late = entry
                .late_tools
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let tool = entry
                .tools
                .iter()
                .chain(late.iter())
                .find(|tool| tool.name == tool_name)
                .ok_or_else(|| RelayRefusal::ToolNotDeclared(tool_name.to_owned()))?;
            if tool.visibility.iter().any(|entry| entry == "app") {
                Ok(())
            } else {
                Err(RelayRefusal::ToolNotAppVisible(tool_name.to_owned()))
            }
        }
        other => Err(RelayRefusal::MethodNotAllowed(other.to_owned())),
    }
}

/// The same gate, for a server that may have gained tools since it was
/// dialled. A tool the host has not seen is asked of the server once more
/// before it is refused: what the server lists now is what it declares, and
/// the gate is on what the server declares, not on when the host looked.
pub(crate) async fn allow_relay_now(
    entry: &AppAttachmentEntry,
    method: &str,
    tool_name: Option<&str>,
) -> Result<(), RelayRefusal> {
    let refusal = match allow_relay(entry, method, tool_name) {
        Err(RelayRefusal::ToolNotDeclared(name)) => RelayRefusal::ToolNotDeclared(name),
        decided => return decided,
    };
    let Ok(client) = entry.relay_client() else {
        return Err(refusal);
    };
    let Ok(listed) = client.list_all_tools().await else {
        return Err(refusal);
    };
    let arrived: Vec<DiscoveredAppTool> = listed
        .iter()
        .map(tool_view)
        .filter(|tool| !entry.tools.iter().any(|known| known.name == tool.name))
        .collect();
    *entry
        .late_tools
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = arrived;
    allow_relay(entry, method, tool_name)
}

/// How long the relay waits on one long operation before giving up. A render
/// is seconds and an unlucky one is minutes; an hour is the point past which
/// something is wrong rather than slow.
const TASK_DEADLINE: Duration = Duration::from_hours(1);

/// Call a tool and answer with its result, whether the server ran it there and
/// then or made a task of it.
///
/// The Workbench declares the tasks capability, so a long tool comes back as a
/// task handle rather than a result. That is the point: the server writes the
/// Call one tool of an attachment's server, for the host's own reasons
/// rather than an agent's, and answer what it answered as one value: its
/// structured content, or its text read as JSON, or its text.
///
/// # Errors
///
/// The server could not be reached, or said it failed.
pub(crate) async fn call_tool_of(
    entry: &AppAttachmentEntry,
    tool: &str,
    arguments: Value,
) -> Result<Value, String> {
    let client = entry.relay_client()?;
    let arguments = match arguments {
        Value::Object(fields) => fields,
        Value::Null => serde_json::Map::new(),
        other => return Err(format!("a tool's arguments are an object, not {other}")),
    };
    let request = CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
    let result = call_through_task(&client, request).await?;
    let text = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    if result.is_error == Some(true) {
        return Err(if text.is_empty() {
            format!("{tool} failed")
        } else {
            text
        });
    }
    if let Some(structured) = result.structured_content {
        return Ok(structured);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// operation's start record before the body runs, so what a person started is
/// in the project's records while it is still running, and survives a reload.
/// The waiting itself belongs here rather than on the page - a surface that
/// polled would be a second client of the task surface, with its own timeout
/// and its own idea of an ending, over a record it can already read.
async fn call_through_task(
    client: &RunningService<RoleClient, ClientConfig>,
    request: CallToolRequestParams,
) -> Result<CallToolResult, String> {
    let created = match client
        .call_tool_once(request)
        .await
        .map_err(|error| error.to_string())?
    {
        CallToolResponse::Complete(result) => return Ok(result),
        CallToolResponse::Task(created) => created,
        // The App's own elicitation is the two-step structured fallback, which
        // calls the server itself; a round arriving here has nobody to answer
        // it, and pretending otherwise would run the tool twice.
        CallToolResponse::InputRequired(_) => {
            return Err("this tool asks for input, which the Workbench answers through the App's structured fallback rather than here".into());
        }
        other => return Err(format!("unsupported MCP result type: {other:?}")),
    };
    let task_id = created.task.task_id.clone();
    let interval = Duration::from_millis(created.task.poll_interval_ms.unwrap_or(250));
    let deadline = std::time::Instant::now() + TASK_DEADLINE;
    loop {
        tokio::time::sleep(interval).await;
        let task = client
            .peer()
            .get_task(GetTaskParams::new(task_id.clone()))
            .await
            .map_err(|error| error.to_string())?
            .task;
        if task.status().is_terminal() {
            return match task.payload {
                TaskPayload::Completed { result } => {
                    serde_json::from_value::<CallToolResult>(Value::Object(result))
                        .map_err(|error| error.to_string())
                }
                // A failed or cancelled task has an end record of its own, so
                // the person is not left with only this sentence.
                other => Err(format!("the operation ended without a result: {other:?}")),
            };
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "the operation is still running after {} seconds",
                TASK_DEADLINE.as_secs()
            ));
        }
    }
}

/// Take one App's client out of the apps lock and put the lock down.
///
/// The guard is consumed on purpose, and this is the only way to reach a
/// client: a caller physically cannot still be holding the lock when it
/// awaits the relay. Held across that await, one App's ten-second tool is
/// the queue for every other request of that App - its own read of the
/// project envelope among them, which is how the App would learn the tool
/// had started at all.
///
/// # Errors
///
/// Returns the sentence for a vanished attachment or one with no supported
/// transport.
pub(crate) fn take_relay_client(
    apps: tokio::sync::MutexGuard<'_, Option<ConnectionApps>>,
    entry_index: usize,
) -> Result<Arc<HostClient>, String> {
    let taken = apps
        .as_ref()
        .and_then(|state| state.entries.get(entry_index))
        .ok_or_else(|| "the App's attachment is no longer open".to_owned())
        .and_then(AppAttachmentEntry::relay_client);
    drop(apps);
    taken
}

/// Execute one allowed relay request against a client and return the
/// JSON-RPC `result` value verbatim.
///
/// # Errors
///
/// Returns the server/client error message.
pub(crate) async fn execute_relay(
    client: &HostClient,
    method: &str,
    params: &Value,
) -> Result<Value, String> {
    match method {
        "tools/list" => {
            let tools = client
                .list_all_tools()
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "tools": tools }))
        }
        "resources/read" => {
            let uri = params
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| "resources/read needs a uri".to_owned())?;
            let read = client
                .read_resource(ReadResourceRequestParams::new(uri))
                .await
                .map_err(|error| error.to_string())?;
            serde_json::to_value(&read).map_err(|error| error.to_string())
        }
        "tools/call" => {
            let name = params
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "tools/call needs a name".to_owned())?;
            let arguments = params
                .get("arguments")
                .and_then(Value::as_object)
                .cloned()
                .map_or_else(JsonObject::new, JsonObject::from_iter);
            let result = call_through_task(
                client,
                CallToolRequestParams::new(name.to_owned()).with_arguments(arguments),
            )
            .await?;
            serde_json::to_value(&result).map_err(|error| error.to_string())
        }
        other => Err(format!("method {other} is not relayable")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_csp_is_the_spec_block_and_declared_domains_extend_it() {
        assert_eq!(resolve_csp(None, false).unwrap(), DEFAULT_APP_CSP);
        assert_eq!(
            resolve_csp(Some(&json!({"prefersBorder": true})), false).unwrap(),
            DEFAULT_APP_CSP
        );
        let declared = resolve_csp(
            Some(&json!({
                "csp": {
                    "connectDomains": ["https://api.example.com", "wss://events.example.com"],
                    "resourceDomains": ["https://*.example.net"]
                }
            })),
            false,
        )
        .unwrap();
        assert!(declared.contains("connect-src 'self' https://api.example.com"));
        assert!(declared.contains("wss://events.example.com"));
        assert!(declared.contains("https://*.example.net"));
        assert!(declared.contains("object-src 'none'"));
        assert!(declared.contains("frame-src 'none'"));
        // A declared block with no foreign origin still admits the App's own
        // origin: script resources, blobs and uploads live there.
        let own = resolve_csp(Some(&json!({"csp": {"connectDomains": []}})), false).unwrap();
        assert!(own.contains("connect-src 'self';"), "{own}");
        assert!(!own.contains("connect-src 'none'"), "{own}");
        // An opaque View runs no blob workers; a View on its own origin may.
        assert!(!own.contains("worker-src"), "{own}");
        let isolated = resolve_csp(None, true).unwrap();
        assert!(isolated.contains("frame-src 'self'"), "{isolated}");
        assert!(isolated.contains("worker-src 'self' blob:"), "{isolated}");
        let isolated_declared =
            resolve_csp(Some(&json!({"csp": {"connectDomains": []}})), true).unwrap();
        assert!(
            isolated_declared.ends_with("object-src 'none'; worker-src 'self' blob:"),
            "{isolated_declared}"
        );
    }

    #[test]
    fn csp_refuses_directive_injection_and_non_origin_urls() {
        for source in [
            "https://safe.example;script-src *",
            "https://safe.example/path",
            "data:text/javascript,alert(1)",
            "https://user@safe.example",
        ] {
            let result = resolve_csp(
                Some(&json!({
                    "csp": {"resourceDomains": [source]}
                })),
                false,
            );
            assert!(result.is_err(), "accepted invalid CSP source {source}");
        }
    }

    #[test]
    fn the_relay_gate_reasons_about_visibility_and_methods() {
        let entry = AppAttachmentEntry {
            server_name: "board".into(),
            client: None,
            exit: None,
            tools: vec![
                DiscoveredAppTool {
                    name: "both_visible".into(),
                    title: None,
                    description: None,
                    input_schema: json!({"type": "object"}),
                    resource_uri: Some("ui://example/board".into()),
                    visibility: vec!["model".into(), "app".into()],
                },
                DiscoveredAppTool {
                    name: "model_only".into(),
                    title: None,
                    description: None,
                    input_schema: json!({"type": "object"}),
                    resource_uri: None,
                    visibility: vec!["model".into()],
                },
            ],
            late_tools: std::sync::Mutex::default(),
            apps: Vec::new(),
            upload_root: None,
        };
        assert!(allow_relay(&entry, "tools/call", Some("both_visible")).is_ok());
        assert_eq!(
            allow_relay(&entry, "tools/call", Some("model_only")),
            Err(RelayRefusal::ToolNotAppVisible("model_only".into()))
        );
        assert_eq!(
            allow_relay(&entry, "tools/call", Some("undeclared")),
            Err(RelayRefusal::ToolNotDeclared("undeclared".into()))
        );
        assert_eq!(
            allow_relay(&entry, "ui/open-link", None),
            Err(RelayRefusal::MethodNotAllowed("ui/open-link".into()))
        );
        assert!(allow_relay(&entry, "tools/list", None).is_ok());
        assert!(allow_relay(&entry, "resources/read", None).is_ok());
    }
}
