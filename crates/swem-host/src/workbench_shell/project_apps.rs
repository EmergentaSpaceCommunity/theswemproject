//! Project-owned App handles are independent of native agent connections.
//! Reads stay on the resources-only Project client; an explicitly opened App
//! uses the existing MCP Apps discovery, sandbox and server-scoped relay.
//!
//! One Apps client per project, dialled the first time somebody opens an
//! App of that project or acts on it, and never by listing. Until
//! 2026-09-21 this module dialled every declared project the moment any
//! project's Apps were asked for - one child process each, on top of the
//! one the project's envelope reader already ran - and rebuilt all of them
//! whenever a project was declared. A person with two projects paid 57.6 s
//! to open the Apps of one.
use std::collections::BTreeMap;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use tokio::sync::OnceCell;

use super::*;
use crate::workbench_apps::{AppAttachmentEntry, AppAttachmentView};

/// One declared project server and, once somebody needed its Apps, the
/// dialled attachment. The cell shares one dial between concurrent
/// requests for this server and never makes a request for another wait.
struct AppSlot {
    declaration: McpServer,
    /// Where bytes an App of this server uploads land: the server's own
    /// workspace, as the factory that made the project knows it.
    upload_root: Option<PathBuf>,
    cell: OnceCell<Arc<AppAttachmentEntry>>,
}

/// One open App of a project, by handle. It holds the attachment it was
/// opened on, so a request for it never goes through a list.
struct ProjectOpenApp {
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
struct ProjectOpen {
    map: BTreeMap<String, ProjectOpenApp>,
    next_app: u64,
}

/// The project Apps this host may dial, one slot per declaration, and the
/// Apps a person has open. `slots` is locked only to find or add a slot,
/// never across an `.await`; `open` is locked to look a handle up and is
/// put down before anything is asked of the server.
pub(super) struct ProjectApps {
    slots: std::sync::Mutex<Vec<Arc<AppSlot>>>,
    open: tokio::sync::Mutex<ProjectOpen>,
}

impl ProjectApps {
    pub(super) fn new() -> Self {
        Self {
            slots: std::sync::Mutex::new(Vec::new()),
            open: tokio::sync::Mutex::new(ProjectOpen {
                map: BTreeMap::new(),
                next_app: 1,
            }),
        }
    }

    /// Take in every declaration not yet known, dialling none. Only adds,
    /// so a project declared while another's App is open leaves that App's
    /// client where it is.
    fn refresh(&self, declarations: &[McpServer], upload_root: impl Fn(&str) -> Option<PathBuf>) {
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
                upload_root: upload_root(name),
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
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown Project server".into()))?;
        let entry = slot
            .cell
            .get_or_try_init(|| async {
                let entry = workbench_apps::discover_server(
                    server.to_owned(),
                    &slot.declaration,
                    slot.upload_root.clone(),
                )
                .await;
                if matches!(slot.declaration, McpServer::Stdio(_)) && entry.relay_client().is_err()
                {
                    return Err(WorkbenchShellError::Failed(format!(
                        "the project server {server} could not be started"
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
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown Project App".into()))?;
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

impl WorkbenchShellState {
    /// The dialled Apps attachment of one project, dialling it now if nobody
    /// has. Only this project.
    pub(super) async fn project_app_entry(
        &self,
        server: &str,
    ) -> Result<Arc<AppAttachmentEntry>, WorkbenchShellError> {
        let declarations = self
            .project_declarations
            .lock()
            .map(|held| held.clone())
            .unwrap_or_default();
        // A project's server works in the workspace the factory made for it;
        // an App of that server uploads into the same place.
        let factory_root = self
            .project_factory
            .get()
            .map(|factory| factory.root.clone());
        self.project_apps.refresh(&declarations, |name| {
            factory_root
                .as_ref()
                .map(|root| root.join(name).join("workspace"))
                .filter(|workspace| workspace.is_dir())
        });
        self.project_apps.entry(server).await
    }

    /// Discover domain Apps without launching an agent.
    /// # Errors
    /// Refuses an unavailable or nonpersistent Project source.
    pub async fn project_apps_list(
        &self,
        server: &str,
    ) -> Result<Vec<AppAttachmentView>, WorkbenchShellError> {
        // The one server asked about, and nothing else: until 2026-09-21 this
        // took the whole project list, which dialled every project to say
        // whether this one keeps its records.
        let available = match self.project_source(server).await {
            Ok(entry) => entry.view().available == Some(true),
            Err(WorkbenchShellError::NotFound(_)) => false,
            Err(error) => return Err(error),
        };
        if !available {
            return Err(WorkbenchShellError::Conflict(
                "Apps require an available persistent Project source".into(),
            ));
        }
        Ok(vec![self.project_app_entry(server).await?.view()])
    }

    /// Open a declared domain surface, without creating or claiming an agent session.
    ///
    /// `slot` is the project slot the person opened it from, when they opened
    /// it from one. Two slots of the same kind declare the same `ui://`
    /// resource, so the resource cannot say which line of work is on screen;
    /// this open can, because it is one per open.
    /// # Errors
    /// Refuses unknown resources and invalid MCP Apps metadata.
    pub async fn project_app_open(
        &self,
        server: &str,
        uri: &str,
        slot: Option<&str>,
    ) -> Result<OpenedApp, WorkbenchShellError> {
        self.project_apps_list(server).await?;
        let entry = self.project_app_entry(server).await?;
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
            let mut open = self.project_apps.open.lock().await;
            let app_id = format!("p{}", open.next_app);
            open.next_app += 1;
            open.map.insert(
                app_id.clone(),
                ProjectOpenApp {
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
            .map(|origin| view_address(origin, &app_id, slot));
        Ok(OpenedApp {
            app_id,
            connection_id: "project".into(),
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

    /// The View of a Project App that asked for a real origin, as a document.
    pub(super) async fn project_app_view(
        &self,
        app_id: &str,
    ) -> Result<(String, String), WorkbenchShellError> {
        let open = self.project_apps.open.lock().await;
        let app = open
            .map
            .get(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown Project App".into()))?;
        if !app.isolated {
            return Err(WorkbenchShellError::NotFound(
                "Project App is not served as a document".into(),
            ));
        }
        Ok((app.html.clone(), app.csp.clone()))
    }

    /// One file under a Project App's View path, by the server's listing.
    pub(super) async fn project_app_file(
        &self,
        app_id: &str,
        path: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let (entry, view_uri, _) = self.project_apps.opened(app_id).await?;
        let uri = workbench_apps::sibling_uri(&view_uri, path)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no file at {path}")))?;
        workbench_apps::read_file_resource(&entry, &uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// Relay through the same method and tool-visibility gates as session Apps.
    /// # Errors
    /// Refuses malformed requests and unknown App handles.
    pub async fn project_app_rpc(
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
        let (entry, _, _) = self.project_apps.opened(app_id).await?;
        let Some(id) = message
            .get("id")
            .filter(|id| id.is_string() || id.is_i64() || id.is_u64())
        else {
            return Ok(Value::Null);
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if let Err(refusal) =
            workbench_apps::allow_relay(&entry, method, params.get("name").and_then(Value::as_str))
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

    pub(super) async fn project_app_script(
        &self,
        app_id: &str,
        uri: &str,
    ) -> Result<String, WorkbenchShellError> {
        let (entry, _, _) = self.project_apps.opened(app_id).await?;
        workbench_apps::read_script_resource(&entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// The bytes of one blob resource of a project's server, for an App the
    /// Project space opened without an agent.
    pub(super) async fn project_app_blob(
        &self,
        app_id: &str,
        uri: &str,
    ) -> Result<(Vec<u8>, String), WorkbenchShellError> {
        let (entry, _, _) = self.project_apps.opened(app_id).await?;
        workbench_apps::read_blob_resource(&entry, uri)
            .await
            .map_err(WorkbenchShellError::Conflict)
    }

    /// The workspace an open Project App's server works in, when the host
    /// that declared the server knows one.
    pub(super) async fn project_app_upload_root(
        &self,
        app_id: &str,
    ) -> Result<Option<PathBuf>, WorkbenchShellError> {
        let (entry, _, _) = self.project_apps.opened(app_id).await?;
        Ok(entry.upload_root.clone())
    }

    /// Close observation only; durable domain state remains in the server journal.
    /// # Errors
    /// Refuses unknown handles. An in-flight relay finishes before removal.
    pub async fn project_app_close(&self, app_id: &str) -> Result<(), WorkbenchShellError> {
        self.project_apps
            .open
            .lock()
            .await
            .map
            .remove(app_id)
            .ok_or_else(|| WorkbenchShellError::NotFound("Unknown Project App".into()))?;
        Ok(())
    }
}

/// Where an isolated App's View is served, for one open.
///
/// The slot travels as part of that address. Two slots of the same kind
/// declare the same `ui://` resource, so the resource cannot say which line of
/// work is on screen; this address can, because there is one per open, and the
/// page reads its own `?slot=` without anything else on the road learning a new
/// word. An App opened outside a slot carries none, and its page keeps the
/// behaviour it had before a project could hold two of anything.
fn view_address(origin: &str, app_id: &str, slot: Option<&str>) -> String {
    let base = format!("{origin}/apps/project/{app_id}/view/");
    slot.map_or(base.clone(), |slot| {
        format!(
            "{base}?slot={}",
            utf8_percent_encode(slot, NON_ALPHANUMERIC)
        )
    })
}

#[cfg(test)]
mod tests {
    use super::view_address;

    /// Two slots of the same kind are two different surfaces, and the address
    /// of each says which one it is.
    #[test]
    fn two_slots_of_one_kind_open_at_two_addresses() {
        let first = view_address("http://127.0.0.1:9000", "p1", Some("score"));
        let second = view_address("http://127.0.0.1:9000", "p2", Some("titles"));
        assert_eq!(
            first,
            "http://127.0.0.1:9000/apps/project/p1/view/?slot=score"
        );
        assert_eq!(
            second,
            "http://127.0.0.1:9000/apps/project/p2/view/?slot=titles"
        );
        assert_ne!(first, second);
    }

    /// A person names their own slots, so the name is carried as data and
    /// never as more address.
    #[test]
    fn a_slot_named_by_a_person_stays_one_query_value() {
        let address = view_address("http://127.0.0.1:9000", "p3", Some("лид &/?#=2"));
        let (path, query) = address.split_once('?').expect("one query");
        assert_eq!(path, "http://127.0.0.1:9000/apps/project/p3/view/");
        assert_eq!(query.matches('?').count(), 0);
        assert_eq!(query.matches('=').count(), 1);
        assert!(!query.contains('#'), "{query}");
        assert!(!query.contains('&'), "{query}");
        assert!(!query.contains(' '), "{query}");
    }

    /// An App opened outside a slot is the App this product had before, at the
    /// address it had before.
    #[test]
    fn an_app_opened_outside_a_slot_says_nothing_about_slots() {
        assert_eq!(
            view_address("http://127.0.0.1:9000", "p4", None),
            "http://127.0.0.1:9000/apps/project/p4/view/"
        );
    }
}
