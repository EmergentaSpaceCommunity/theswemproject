//! The Apps of the servers this host declared, outside any agent session.
//!
//! A server that marks one of its App resources as its home is a space on
//! the Workbench; opening the space opens that App, which reads its server
//! and calls its tools through the same relay and the same gates a session's
//! App goes through. No agent connection takes part.
//!
//! One Apps client per declared server, dialled the first time somebody
//! lists the spaces or opens one, and kept. A request for one server never
//! waits on the dial of another.
use std::collections::BTreeMap;

use tokio::sync::OnceCell;

use super::*;
use crate::workbench_apps::AppAttachmentEntry;

/// One declared server and, once somebody needed its Apps, the dialled
/// attachment. The cell shares one dial between concurrent requests for
/// this server and never makes a request for another wait.
struct AppSlot {
    declaration: McpServer,
    cell: OnceCell<Arc<AppAttachmentEntry>>,
}

/// One open App of a space, by handle. It holds the attachment it was
/// opened on, so a request for it never goes through a list.
struct SpaceOpenApp {
    entry: Arc<AppAttachmentEntry>,
    uri: String,
    /// The View's HTML and the CSP the host resolved for it, kept so the
    /// sandbox origin can serve the View as a document of its own when the
    /// App asked for a real origin.
    html: String,
    csp: String,
    isolated: bool,
}

#[derive(Default)]
struct SpaceOpen {
    map: BTreeMap<String, SpaceOpenApp>,
    next_app: u64,
}

/// The servers whose Apps this host may dial, one slot per declaration, and the
/// Apps a person has open. `slots` is locked only to find or add a slot,
/// never across an `.await`; `open` is locked to look a handle up and is
/// put down before anything is asked of the server.
pub(super) struct ServerApps {
    slots: std::sync::Mutex<Vec<Arc<AppSlot>>>,
    open: tokio::sync::Mutex<SpaceOpen>,
}

impl ServerApps {
    pub(super) fn new() -> Self {
        Self {
            slots: std::sync::Mutex::new(Vec::new()),
            open: tokio::sync::Mutex::new(SpaceOpen {
                map: BTreeMap::new(),
                next_app: 1,
            }),
        }
    }

    /// Take in every declaration not yet known, dialling none. Only adds,
    /// so a server declared while another's App is open leaves that App's
    /// client where it is.
    fn refresh(&self, declarations: &[McpServer]) {
        let mut slots = lock(&self.slots);
        for declaration in declarations {
            let name = match declaration {
                McpServer::Stdio(server) => &server.name,
                McpServer::Http(server) => &server.name,
                McpServer::Sse(server) => &server.name,
                _ => continue,
            };
            if slots
                .iter()
                .any(|slot| slot_name(&slot.declaration) == Some(name.as_str()))
            {
                continue;
            }
            slots.push(Arc::new(AppSlot {
                declaration: declaration.clone(),
                cell: OnceCell::new(),
            }));
        }
    }

    /// The dialled attachment of one server: this call's dial when it is
    /// the first to ask, the shared one when another is under way, the kept
    /// one after. A stdio server that could not be started is not kept, so
    /// the next open tries again.
    async fn entry(&self, server: &str) -> Result<Arc<AppAttachmentEntry>, WorkbenchShellError> {
        let slot = lock(&self.slots)
            .iter()
            .find(|slot| slot_name(&slot.declaration) == Some(server))
            .cloned()
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown server".into()))?;
        let entry = slot
            .cell
            .get_or_try_init(|| async {
                let entry = workbench_apps::discover_server(
                    server.to_owned(),
                    &slot.declaration,
                    // A space's server works where it works; this host
                    // knows no directory of its own to hand it bytes in.
                    None,
                )
                .await;
                if matches!(slot.declaration, McpServer::Stdio(_)) && entry.relay_client().is_err()
                {
                    return Err(WorkbenchShellError::Failed(format!(
                        "the server {server} could not be started"
                    )));
                }
                Ok(Arc::new(entry))
            })
            .await?;
        Ok(Arc::clone(entry))
    }

    /// Close every open App and shut down every dialled attachment; a slot
    /// nobody opened has no child and costs nothing here.
    pub(super) async fn shutdown(&self) -> Result<(), String> {
        self.open.lock().await.map.clear();
        let slots = std::mem::take(&mut *lock(&self.slots));
        let mut first_error = None;
        for slot in slots {
            let Some(slot) = Arc::into_inner(slot) else {
                continue;
            };
            let Some(entry) = slot.cell.into_inner().and_then(Arc::into_inner) else {
                continue;
            };
            if let Err(error) = entry.shutdown().await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// The open App behind a handle: its attachment and what was opened.
    async fn opened(
        &self,
        app_id: &str,
    ) -> Result<(Arc<AppAttachmentEntry>, String, bool), WorkbenchShellError> {
        let open = self.open.lock().await;
        let app = open
            .map
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown App".into()))?;
        Ok((Arc::clone(&app.entry), app.uri.clone(), app.isolated))
    }
}

fn slot_name(declaration: &McpServer) -> Option<&str> {
    match declaration {
        McpServer::Stdio(server) => Some(&server.name),
        McpServer::Http(server) => Some(&server.name),
        McpServer::Sse(server) => Some(&server.name),
        _ => None,
    }
}

/// A poisoned lock here means a thread panicked between two reads of a
/// list; the list is still a list.
fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A server's home App, as the host lists it: a space of its own.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SpaceView {
    pub server: String,
    pub uri: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl WorkbenchShellState {
    /// Every server this host declared, as one list of declarations.
    fn every_declaration(&self) -> Vec<McpServer> {
        self.mcp_catalogue
            .get()
            .and_then(|catalogue| {
                catalogue
                    .declared()
                    .lock()
                    .ok()
                    .map(|declared| declared.values().cloned().collect())
            })
            .unwrap_or_default()
    }

    /// The Apps client of a declared server, dialled once and kept.
    pub(super) async fn declared_app_entry(
        &self,
        server: &str,
    ) -> Result<Arc<AppAttachmentEntry>, WorkbenchShellError> {
        self.server_apps.refresh(&self.every_declaration());
        self.server_apps.entry(server).await
    }

    /// Call one tool of a declared server, for the Store: a server that
    /// takes packages is asked to plan, install and remove them.
    ///
    /// # Errors
    ///
    /// No such server, or the server could not be reached or refused.
    pub async fn call_declared_server(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, WorkbenchShellError> {
        let entry = self.declared_app_entry(server).await?;
        workbench_apps::call_tool_of(&entry, tool, arguments)
            .await
            .map_err(WorkbenchShellError::Failed)
    }

    /// Every space a declared server offers: its home App, read off the
    /// server's own resource listing. Each server is dialled once per run
    /// and kept; one that does not answer in time is left out of this
    /// listing, not waited for by the others.
    ///
    /// # Errors
    ///
    /// Never fails today; a server that cannot be reached is simply not a
    /// space.
    pub async fn spaces(&self) -> Result<Vec<SpaceView>, WorkbenchShellError> {
        let names: Vec<String> = self
            .every_declaration()
            .iter()
            .filter_map(|declaration| slot_name(declaration).map(str::to_owned))
            .collect();
        let mut spaces = Vec::new();
        for name in names {
            let dialled = tokio::time::timeout(
                std::time::Duration::from_secs(15),
                self.declared_app_entry(&name),
            )
            .await;
            let Ok(Ok(entry)) = dialled else {
                continue;
            };
            for app in entry.view().apps.into_iter().filter(|app| app.home) {
                spaces.push(SpaceView {
                    server: name.clone(),
                    uri: app.uri,
                    name: name.clone(),
                    description: app.description,
                });
            }
        }
        spaces.sort_by(|left, right| left.server.cmp(&right.server));
        Ok(spaces)
    }

    /// Open a declared server's home App as a space: read the App and hand
    /// it to the page as any App is handed, without a tool
    /// call - the App reads the server through the relay from there.
    ///
    /// # Errors
    ///
    /// Not found for a server that declares no such home App; the App's own
    /// refusal otherwise.
    pub async fn space_open(
        &self,
        server: &str,
        uri: &str,
    ) -> Result<OpenedApp, WorkbenchShellError> {
        let entry = self.declared_app_entry(server).await?;
        if !entry
            .view()
            .apps
            .iter()
            .any(|app| app.uri == uri && app.home)
        {
            return Err(WorkbenchShellError::NotFound(format!(
                "{server} declares no home App at {uri}"
            )));
        }
        self.open_app_of(entry, server, uri).await
    }

    /// Open one App of a dialled server and hand it to the page: the View's
    /// HTML and CSP, its handle for the relay, and its own address when it
    /// asked for an origin of its own.
    async fn open_app_of(
        &self,
        entry: Arc<AppAttachmentEntry>,
        server: &str,
        uri: &str,
    ) -> Result<OpenedApp, WorkbenchShellError> {
        let workbench_apps::AppRead {
            html,
            csp,
            permissions,
            prefers_border,
            isolated,
        } = workbench_apps::read_app(&entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)?;
        let app_id = {
            let mut open = self.server_apps.open.lock().await;
            let app_id = format!("s{}", open.next_app);
            open.next_app += 1;
            open.map.insert(
                app_id.clone(),
                SpaceOpenApp {
                    entry,
                    uri: uri.into(),
                    html: html.clone(),
                    csp: csp.clone(),
                    isolated,
                },
            );
            app_id
        };
        let (sandbox_url, sandbox_origin) =
            self.sandbox.get().map_or((None, None), |(url, origin)| {
                (Some(url.clone()), Some(origin.clone()))
            });
        let view_url = sandbox_origin
            .as_ref()
            .filter(|_| isolated)
            .map(|origin| view_address(origin, &app_id));
        Ok(OpenedApp {
            app_id,
            connection_id: SPACE_CONNECTION.into(),
            server_name: server.into(),
            uri: uri.into(),
            html,
            csp,
            permissions,
            prefers_border,
            sandbox_url,
            sandbox_origin,
            isolated,
            view_url,
        })
    }

    /// The View of a space's App that asked for a real origin, as a document.
    pub(super) async fn space_app_view(
        &self,
        app_id: &str,
    ) -> Result<(String, String), WorkbenchShellError> {
        let open = self.server_apps.open.lock().await;
        let app = open
            .map
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown App".into()))?;
        if !app.isolated {
            return Err(WorkbenchShellError::NotFound(
                "this App is not served as a document".into(),
            ));
        }
        Ok((app.html.clone(), app.csp.clone()))
    }

    /// One file under a space App's View path, by the server's listing.
    pub(super) async fn space_app_file(
        &self,
        app_id: &str,
        path: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let (entry, view_uri, _) = self.server_apps.opened(app_id).await?;
        let uri = workbench_apps::sibling_uri(&view_uri, path)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no file at {path}")))?;
        workbench_apps::read_file_resource(&entry, &uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// Relay through the same method and tool-visibility gates as session Apps.
    /// # Errors
    /// Refuses malformed requests and unknown App handles.
    pub async fn space_app_rpc(
        &self,
        app_id: &str,
        message: Value,
    ) -> Result<Value, WorkbenchShellError> {
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(WorkbenchShellError::Invalid(
                "relay accepts JSON-RPC 2.0 messages".into(),
            ));
        }
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| WorkbenchShellError::Invalid("relay requires a method".into()))?;
        let (entry, _, _) = self.server_apps.opened(app_id).await?;
        let Some(id) = message
            .get("id")
            .filter(|id| id.is_string() || id.is_i64() || id.is_u64())
        else {
            return Ok(Value::Null);
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if let Err(refusal) = workbench_apps::allow_relay_now(
            &entry,
            method,
            params.get("name").and_then(Value::as_str),
        )
        .await
        {
            let code = match refusal {
                RelayRefusal::MethodNotAllowed(_) => -32601,
                _ => -32602,
            };
            return Ok(
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":code, "message":refusal.message()}}),
            );
        }
        // No lock is held here: the attachment is shared, so a long tool
        // waits on nothing and nothing waits on it - an App of the same
        // server reads while the action runs.
        let outcome = match entry.relay_client() {
            Ok(client) => workbench_apps::execute_relay(&client, method, &params).await,
            Err(refusal) => Err(refusal),
        };
        Ok(match outcome {
            Ok(result) => json!({"jsonrpc":"2.0", "id":id, "result":result}),
            Err(error) => {
                json!({"jsonrpc":"2.0", "id":id, "error":{"code":-32000,"message":error}})
            }
        })
    }

    pub(super) async fn space_app_script(
        &self,
        app_id: &str,
        uri: &str,
    ) -> Result<String, WorkbenchShellError> {
        let (entry, _, _) = self.server_apps.opened(app_id).await?;
        workbench_apps::read_script_resource(&entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// The bytes of one blob resource of a space's server, for an App opened
    /// without an agent.
    pub(super) async fn space_app_blob(
        &self,
        app_id: &str,
        uri: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let (entry, _, _) = self.server_apps.opened(app_id).await?;
        workbench_apps::read_blob_resource(&entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// Where an open space App's uploads land, when the host that declared
    /// the server knows a place.
    pub(super) async fn space_app_upload_root(
        &self,
        app_id: &str,
    ) -> Result<Option<PathBuf>, WorkbenchShellError> {
        let (entry, _, _) = self.server_apps.opened(app_id).await?;
        Ok(entry.upload_root.clone())
    }

    /// Close the App's handle; what the server keeps, it keeps.
    /// # Errors
    /// Refuses unknown handles. An in-flight relay finishes before removal.
    pub async fn space_app_close(&self, app_id: &str) -> Result<(), WorkbenchShellError> {
        self.server_apps
            .open
            .lock()
            .await
            .map
            .remove(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown App".into()))?;
        Ok(())
    }
}

/// The scope of a space's App in the sandbox origin's addresses, where a
/// session's App has its connection id.
pub(super) const SPACE_CONNECTION: &str = "space";

/// Where an isolated App's View is served, for one open.
fn view_address(origin: &str, app_id: &str) -> String {
    format!("{origin}/apps/{SPACE_CONNECTION}/{app_id}/view/")
}

#[cfg(test)]
mod tests {
    use super::view_address;

    #[test]
    fn an_app_of_a_space_is_served_under_its_own_handle() {
        assert_eq!(
            view_address("http://127.0.0.1:9000", "s4"),
            "http://127.0.0.1:9000/apps/space/s4/view/"
        );
    }
}
