//! A tunnel: an address from outside, for a while (ADR-0015).
//!
//! A Workbench that is not served at an address has none the outside can
//! reach, and the page a bot opens inside the messenger needs one. A tunnel
//! is a package of a fixed shape (`swem_sdk::tunnel`), run by the harness as
//! a channel is, which stands at a public address for a local URL. The local
//! URL is the gate: a second loopback listener that answers the page and its
//! API and nothing else (`door::route_the_gate`). The tunnel's origin lives
//! here in memory and nowhere else; it closes after a while unused, or when
//! the person closes it. A served Workbench has an origin already and
//! refuses to open one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::v1::{EnvVariable, McpServer, McpServerStdio};
use serde::Serialize;
use serde_json::json;
use swem_sdk::tunnel;

use super::{WorkbenchShellError, WorkbenchShellState};
use crate::chat_ledger::now_ms;
use crate::workbench_apps::{AppAttachmentEntry, call_tool_of, discover_server};

/// How long a tunnel stays open with nobody using it.
pub const IDLE: Duration = Duration::from_mins(30);

/// A tunnel that is open.
pub(super) struct Tunnel {
    origin: String,
    package: String,
    opened_ms: i64,
    entry: Arc<AppAttachmentEntry>,
    /// The gate: told, it stops listening.
    gate: Option<tokio::sync::oneshot::Sender<()>>,
    /// Notified whenever somebody known uses the tunnel.
    touched: Arc<tokio::sync::Notify>,
}

/// A tunnel as the page shows it.
#[derive(Clone, Debug, Serialize)]
pub struct TunnelShown {
    pub package: String,
    pub origin: String,
    pub opened_ms: i64,
}

/// What can open a tunnel here: a package the Store installed, or one that
/// came with the product.
#[derive(Clone, Debug, Serialize)]
pub struct TunnelPackage {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    pub bundled: bool,
}

/// How this Workbench is reached from outside, as the page shows it.
#[derive(Clone, Debug, Serialize)]
pub struct ReachStanding {
    /// Served at an address: that address.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub served_at: Option<String>,
    /// A tunnel that is open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<TunnelShown>,
    /// What could open one.
    pub packages: Vec<TunnelPackage>,
}

impl WorkbenchShellState {
    /// Where a channel's page is answered from outside: the served address,
    /// else the tunnel that is open. The one question channels ask; the
    /// webhook door keeps asking `served_origin`.
    pub(super) fn app_origin(&self) -> Option<String> {
        self.served_origin().or_else(|| {
            self.tunnel
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_ref()
                .map(|tunnel| tunnel.origin.clone())
        })
    }

    /// Somebody known used the tunnel: it stays open a while longer.
    pub(super) fn touch_tunnel(&self) {
        if let Some(tunnel) = self
            .tunnel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
        {
            tunnel.touched.notify_one();
        }
    }

    fn tunnel_shown(&self) -> Option<TunnelShown> {
        self.tunnel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|tunnel| TunnelShown {
                package: tunnel.package.clone(),
                origin: tunnel.origin.clone(),
                opened_ms: tunnel.opened_ms,
            })
    }

    /// How this Workbench is reached from outside, and what could open a
    /// tunnel.
    ///
    /// # Errors
    ///
    /// Never, in practice: an install root that cannot be read lists no
    /// packages.
    pub fn reach_standing(&self) -> Result<ReachStanding, WorkbenchShellError> {
        Ok(ReachStanding {
            served_at: self.served_origin(),
            tunnel: self.tunnel_shown(),
            packages: self.tunnel_packages(),
        })
    }

    /// What can open a tunnel: the Store's receipts of the kind, and what
    /// came with the product beside the binary.
    fn tunnel_packages(&self) -> Vec<TunnelPackage> {
        let mut packages: Vec<TunnelPackage> = Vec::new();
        if let Ok(installed) = self.installed_root()
            && let Ok(kind) = swem_sdk::Kind::parse(tunnel::TUNNEL_KIND)
        {
            for (id, receipt) in swem_store::load_receipts(installed, &kind) {
                packages.push(TunnelPackage {
                    id,
                    name: receipt.name,
                    version: receipt.version,
                    bundled: false,
                });
            }
        }
        for (package, _) in beside_the_binary("swem-tunnel-") {
            if !packages.iter().any(|known| known.id == package) {
                packages.push(TunnelPackage {
                    id: package.clone(),
                    name: package,
                    version: String::new(),
                    bundled: true,
                });
            }
        }
        packages.sort_by(|a, b| a.id.cmp(&b.id));
        packages
    }

    /// The program of a tunnel package: the Store's receipt, or the one
    /// beside the binary.
    fn tunnel_program(&self, package: &str) -> Result<PathBuf, WorkbenchShellError> {
        if let Ok(installed) = self.installed_root()
            && let Ok(kind) = swem_sdk::Kind::parse(tunnel::TUNNEL_KIND)
            && let Some(receipt) = swem_store::load_receipts(installed, &kind).remove(package)
            && let Some(path) = receipt.launch_path()
        {
            return Ok(path.to_path_buf());
        }
        beside_the_binary("swem-tunnel-")
            .into_iter()
            .find(|(id, _)| id == package)
            .map(|(_, path)| path)
            .ok_or_else(|| {
                WorkbenchShellError::NotFound(format!(
                    "the tunnel package {package} is not installed here"
                ))
            })
    }

    /// The tools on hand, by id, for a package that drives one.
    fn tools_on_hand(&self) -> BTreeMap<String, PathBuf> {
        let Ok(installed) = self.installed_root() else {
            return BTreeMap::new();
        };
        swem_store::load_receipts(installed, &swem_sdk::Kind::TOOL)
            .into_iter()
            .filter_map(|(id, receipt)| receipt.launch_path().map(|path| (id, path.to_path_buf())))
            .collect()
    }

    /// Open a tunnel with a package, or with the one package there is:
    /// the gate is bound, the package started and checked against the
    /// tunnel shape, told the gate's URL, and the origin it answers is kept.
    /// Open already, the tunnel that is open is answered.
    ///
    /// # Errors
    ///
    /// Served at an address (there is an origin already); no package, or
    /// none of that name; a program that is not a tunnel; a package that
    /// cannot stand at an address, in its words.
    pub async fn open_tunnel(
        self: &Arc<Self>,
        package: Option<&str>,
    ) -> Result<TunnelShown, WorkbenchShellError> {
        self.open_tunnel_for(package, IDLE).await
    }

    /// [`Self::open_tunnel`] with how long it stays open unused; a test
    /// asks for a shorter while.
    ///
    /// # Errors
    ///
    /// As [`Self::open_tunnel`].
    pub async fn open_tunnel_for(
        self: &Arc<Self>,
        package: Option<&str>,
        idle: Duration,
    ) -> Result<TunnelShown, WorkbenchShellError> {
        if let Some(address) = self.served_origin() {
            return Err(WorkbenchShellError::Conflict(format!(
                "this Workbench is served at {address}; a tunnel is for one without an address"
            )));
        }
        if let Some(open) = self.tunnel_shown() {
            return Ok(open);
        }
        let package = self.tunnel_package_chosen(package)?;
        let stdio = self.tunnel_stdio(&package)?;

        // The gate first, so the package is told a URL that answers.
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| WorkbenchShellError::Failed(format!("bind the gate: {error}")))?;
        let port = listener
            .local_addr()
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            .port();
        let (told, told_rx) = tokio::sync::oneshot::channel();
        let state = Arc::clone(self);
        tokio::spawn(super::accept_until_told(
            listener,
            None,
            told_rx,
            move |request| {
                let state = Arc::clone(&state);
                async move { super::door::route_the_gate(&state, request).await }
            },
        ));
        let (entry, origin) = match standing_at(&package, stdio, port).await {
            Ok(stood) => stood,
            Err(error) => {
                let _ = told.send(());
                return Err(error);
            }
        };

        let touched = Arc::new(tokio::sync::Notify::new());
        let shown = TunnelShown {
            package: package.clone(),
            origin: origin.clone(),
            opened_ms: now_ms(),
        };
        *self
            .tunnel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Tunnel {
            origin,
            package,
            opened_ms: shown.opened_ms,
            entry,
            gate: Some(told),
            touched: Arc::clone(&touched),
        });
        // Unused for a while, it closes: the house pattern of a session
        // that is let go when nobody is there.
        let state = Arc::clone(self);
        tokio::spawn(async move {
            while tokio::time::timeout(idle, touched.notified()).await.is_ok() {}
            state.close_tunnel().await;
        });
        Ok(shown)
    }

    /// The package to open a tunnel with: the one named, else the one the
    /// person installed, else the one that came with the product; several
    /// installed, the person names one.
    fn tunnel_package_chosen(&self, named: Option<&str>) -> Result<String, WorkbenchShellError> {
        let packages = self.tunnel_packages();
        if let Some(named) = named {
            return packages
                .iter()
                .find(|known| known.id == named)
                .map(|known| known.id.clone())
                .ok_or_else(|| {
                    WorkbenchShellError::NotFound(format!("no tunnel package {named} here"))
                });
        }
        let installed: Vec<&TunnelPackage> =
            packages.iter().filter(|known| !known.bundled).collect();
        let all: Vec<&TunnelPackage> = packages.iter().collect();
        match (installed.as_slice(), all.as_slice()) {
            ([one], _) | ([], [one]) => Ok(one.id.clone()),
            ([], []) => Err(WorkbenchShellError::NotFound(
                "nothing here can open a tunnel: install one from the Store".into(),
            )),
            _ => Err(WorkbenchShellError::Invalid(
                "several packages could open a tunnel; name one".into(),
            )),
        }
    }

    /// How a tunnel package is started: its program, a home of its own,
    /// and the tools on hand in its environment.
    fn tunnel_stdio(&self, package: &str) -> Result<McpServerStdio, WorkbenchShellError> {
        let program = self.tunnel_program(package)?;
        let home = self.data_root_for_tunnels()?.join(package);
        std::fs::create_dir_all(&home).map_err(|error| {
            WorkbenchShellError::Failed(format!("make {}: {error}", home.display()))
        })?;
        let mut env = vec![EnvVariable::new(
            tunnel::HOME_VARIABLE,
            home.display().to_string(),
        )];
        for (id, path) in self.tools_on_hand() {
            env.push(EnvVariable::new(
                tunnel::tool_variable(&id),
                path.display().to_string(),
            ));
        }
        Ok(
            McpServerStdio::new(format!("tunnel-{package}"), program.display().to_string())
                .env(env),
        )
    }

    /// Close the tunnel that is open: the package is told, its program and
    /// the gate are stopped. Nothing is open: nothing happens.
    pub async fn close_tunnel(&self) {
        let taken = self
            .tunnel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        let Some(mut tunnel) = taken else {
            return;
        };
        let _ = call_tool_of(&tunnel.entry, tunnel::CLOSE, json!({})).await;
        if let Some(told) = tunnel.gate.take() {
            let _ = told.send(());
        }
        if let Some(alone) = Arc::into_inner(tunnel.entry) {
            let _ = alone.shutdown().await;
        }
    }

    /// Where tunnels keep their files, under the data root.
    ///
    /// # Errors
    ///
    /// Enabled twice.
    pub fn enable_tunnels(&self, root: &Path) -> Result<(), WorkbenchShellError> {
        self.tunnel_root
            .set(root.to_path_buf())
            .map_err(|_| WorkbenchShellError::Conflict("tunnels are enabled already".into()))
    }

    fn data_root_for_tunnels(&self) -> Result<PathBuf, WorkbenchShellError> {
        self.tunnel_root
            .get()
            .cloned()
            .ok_or_else(|| WorkbenchShellError::Conflict("tunnels are not enabled here".into()))
    }
}

/// The package started, checked against the tunnel shape, and told the
/// gate's URL: its program and the origin it stands at.
async fn standing_at(
    package: &str,
    stdio: McpServerStdio,
    port: u16,
) -> Result<(Arc<AppAttachmentEntry>, String), WorkbenchShellError> {
    let entry = discover_server(format!("tunnel-{package}"), &McpServer::Stdio(stdio), None).await;
    let listed: Vec<swem_sdk::ToolListed> = entry
        .tools
        .iter()
        .map(|tool| swem_sdk::ToolListed {
            name: tool.name.clone(),
            input_schema: tool.input_schema.clone(),
        })
        .collect();
    if let Err(short) = tunnel::shape().check(&listed) {
        let _ = entry.shutdown().await;
        return Err(WorkbenchShellError::Invalid(format!(
            "{package} does not answer as a tunnel: it {short}"
        )));
    }
    let entry = Arc::new(entry);
    let standing: tunnel::Standing = call_tool_of(
        &entry,
        tunnel::OPEN,
        json!({ "url": format!("http://127.0.0.1:{port}") }),
    )
    .await
    .and_then(|answer| serde_json::from_value(answer).map_err(|error| error.to_string()))
    .map_err(|error| WorkbenchShellError::Failed(format!("{package}: open: {error}")))?;
    if standing.origin.trim().is_empty() {
        if let Some(alone) = Arc::into_inner(entry) {
            let _ = alone.shutdown().await;
        }
        return Err(WorkbenchShellError::Failed(format!(
            "{package} could not stand at an address: {}",
            if standing.said.is_empty() {
                "it said nothing"
            } else {
                standing.said.as_str()
            }
        )));
    }
    Ok((
        entry,
        standing.origin.trim().trim_end_matches('/').to_owned(),
    ))
}

/// Programs of a family beside the binary (`swem-tunnel-*`), as `(id, path)`;
/// not what a build leaves there, and not a fixture.
fn beside_the_binary(prefix: &str) -> Vec<(String, PathBuf)> {
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stem = name.strip_suffix(".exe").unwrap_or(&name).to_owned();
        if let Some(package) = stem.strip_prefix(prefix)
            && !package.is_empty()
            && !package.contains('.')
            && package != "fixture"
            && entry.path().is_file()
        {
            found.push((package.to_owned(), entry.path()));
        }
    }
    found
}
