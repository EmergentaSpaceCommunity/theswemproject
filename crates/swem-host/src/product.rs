//! The product, assembled from a data root in a page of code.
//!
//! Everything the Workbench is, before a door is opened onto it: the data
//! root and its named places, the profiles, the route ledger, the projects
//! and MCP servers this machine declared, the agents it can discover, the
//! Store, and the resolver that turns a profile into a running agent. One
//! builder, because there are several doors onto the same product - the
//! Workbench over HTTP, the editor door over stdio, an application that
//! embeds the harness - and a second assembly would be a second product
//! wearing the same name. What differs between the doors stays with each
//! door: the Workbench mints a session secret and runs the clock, the editor
//! door does neither.
//!
//! Nothing here is process-global: two products assembled in one process
//! read different roots, registries and catalogs.

mod at_an_address;
mod built_in;
mod resolver;

pub use at_an_address::{Closed, ServeAt, ServedAt};
pub use built_in::{Answering, BuiltIn};

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::McpServer;

use crate::workbench_shell::{Catalog, WorkbenchShellHandle};
use crate::{AgentDiscovery, Readiness, WorkbenchAgentOption, WorkbenchShellState};

/// Where the agent is inside the image a container environment runs it in,
/// by convention, so a person needs to say only which image.
pub const AGENT_IN_THE_IMAGE: &str = "/usr/local/bin/swem-agent";

/// The environment variable that names the container image when the
/// builder was not told one.
pub const CONTAINER_IMAGE_VAR: &str = "SWEM_CONTAINER_IMAGE";

/// The directory a product keeps everything under, and the places in it by
/// name, so an embedder and the product binary agree on the layout without
/// spelling a path twice.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataRoot {
    root: PathBuf,
}

impl DataRoot {
    /// A product at `root`; the directory is made when the product is
    /// assembled.
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Where this machine keeps the product: `%LOCALAPPDATA%\SWEM\workbench`,
    /// `$XDG_DATA_HOME/swem/workbench`, else `~/.local/share/swem/workbench`.
    ///
    /// # Errors
    ///
    /// None of the three variables is set.
    pub fn for_this_machine() -> Result<Self, String> {
        if let Some(root) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
            return Ok(Self::at(PathBuf::from(root).join("SWEM").join("workbench")));
        }
        if let Some(root) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            return Ok(Self::at(PathBuf::from(root).join("swem").join("workbench")));
        }
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| "LOCALAPPDATA, XDG_DATA_HOME and HOME are unavailable".to_owned())?;
        Ok(Self::at(
            PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("swem")
                .join("workbench"),
        ))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.root
    }

    /// The personal agent profiles.
    #[must_use]
    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles")
    }

    /// The route ledger.
    #[must_use]
    pub fn routes(&self) -> PathBuf {
        self.root.join("routes.jsonl")
    }

    /// Where this product installs things: agents, tools, servers, skills.
    #[must_use]
    pub fn installed(&self) -> PathBuf {
        self.root.join("installed")
    }

    /// The Store's indexes and the registry's cached copy.
    #[must_use]
    pub fn indexes(&self) -> PathBuf {
        self.root.join("indexes")
    }

    /// The MCP servers a person declared from the product.
    #[must_use]
    pub fn mcp_servers(&self) -> PathBuf {
        self.root.join("mcp-servers")
    }

    /// The places a model is served from.
    #[must_use]
    pub fn model_providers(&self) -> PathBuf {
        self.root.join("model-providers")
    }

    /// The keys of providers, their owner's alone.
    #[must_use]
    pub fn keys(&self) -> PathBuf {
        self.root.join("keys")
    }

    /// Where prepared environments live, shared across projects.
    #[must_use]
    pub fn environments(&self) -> PathBuf {
        self.root.join("environments")
    }

    /// Workspaces the product makes for profiles that have none.
    #[must_use]
    pub fn workspaces(&self) -> PathBuf {
        self.root.join("workspaces")
    }

    /// Agent homes the product makes for profiles.
    #[must_use]
    pub fn agent_homes(&self) -> PathBuf {
        self.root.join("agent-homes")
    }

    /// The look of the system's keeper when it has nothing to do here,
    /// found without putting the product together; nothing when there is
    /// something to do.
    #[must_use]
    pub fn nothing_to_keep(&self) -> Option<crate::KeeperLook> {
        crate::workbench_shell::nothing_for_a_keeper_to_do(
            &self.routes(),
            &self.time(),
            crate::KEPT_BY_THE_SYSTEM,
        )
    }

    /// What was removed and is kept aside: the profiles of agents that
    /// were removed.
    #[must_use]
    pub fn removed(&self) -> PathBuf {
        self.root.join("removed")
    }

    /// Who may come in to a Workbench served at an address.
    #[must_use]
    pub fn access(&self) -> PathBuf {
        self.root.join("access")
    }

    /// Who keeps time for whom, and when a keeper looked last.
    #[must_use]
    pub fn time(&self) -> PathBuf {
        self.root.join("time")
    }

    /// Where the clock before the ledger's kept its schedules: they are
    /// brought into the ledger and the directory is set aside.
    #[must_use]
    pub fn schedules(&self) -> PathBuf {
        self.root.join("schedules")
    }

    /// Agents a person declared beyond the built-in catalogue.
    #[must_use]
    pub fn agents(&self) -> PathBuf {
        self.root.join("agents")
    }

    /// Where products before 2026-09-25 installed agents: a sibling of the
    /// data root (`<data home>/swem/agents`) rather than inside it.
    #[must_use]
    pub fn legacy_agents_home(&self) -> Option<PathBuf> {
        self.root.parent().map(|parent| parent.join("agents"))
    }
}

/// The product before a door is opened onto it.
pub struct Product {
    root: DataRoot,
    profiles: Option<PathBuf>,
    routes: Option<PathBuf>,
    operation_timeout: Duration,
    shipped: Vec<Catalog>,
    acp_registry: Option<String>,
    declarations: Vec<McpServer>,
    container_image: Option<String>,
    agent_in_image: String,
    mcp_observer: Option<(PathBuf, Vec<String>)>,
    time_tools: Option<(PathBuf, Vec<String>)>,
    keep_time: Option<(PathBuf, Vec<String>)>,
    called: Option<String>,
    owned_by: Option<String>,
    servers_for: Option<Arc<ServersFor>>,
    secrets: Option<crate::Keeper>,
    takers: Vec<Arc<dyn crate::Taker>>,
}

/// What a product says an agent is handed beside what its profile names:
/// resolved when the agent's session opens, for that agent.
type ServersFor = dyn Fn(&crate::PersonalAgentProfile) -> Vec<McpServer> + Send + Sync;

impl Product {
    /// A product over `root`, with the defaults a person's machine gets: a
    /// ten-minute operation timeout, the public agent registry, no shipped
    /// catalog, no container image, no server declared.
    #[must_use]
    pub fn at(root: DataRoot) -> Self {
        Self {
            root,
            profiles: None,
            routes: None,
            operation_timeout: Duration::from_mins(10),
            shipped: Vec::new(),
            acp_registry: None,
            declarations: Vec::new(),
            container_image: None,
            agent_in_image: AGENT_IN_THE_IMAGE.to_owned(),
            mcp_observer: None,
            time_tools: None,
            keep_time: None,
            called: None,
            owned_by: None,
            servers_for: None,
            secrets: None,
            takers: Vec::new(),
        }
    }

    /// A kind of package this product takes from the Store, beside the
    /// harness's own: what it is called, what it accepts, how a candidate
    /// is checked, what happens once one is installed.
    #[must_use]
    pub fn takes(mut self, taker: Arc<dyn crate::Taker>) -> Self {
        self.takers.push(taker);
        self
    }

    /// Who keeps what a person gives: the keys of providers, what a
    /// profile is launched with, the values of declared servers. Without
    /// it they are files under the data root, closed to others; a product
    /// that has a place of its own for such things supplies it here and
    /// the harness keeps nothing of them anywhere else.
    #[must_use]
    pub fn secrets_kept_by(mut self, keeper: crate::Keeper) -> Self {
        self.secrets = Some(keeper);
        self
    }

    /// Servers an agent is handed because the product says so, resolved
    /// for that agent when its session opens: the product's own server
    /// with what that agent may reach, say. A profile's own attachments
    /// come first; a server named the same as one of them is not added
    /// twice.
    #[must_use]
    pub fn servers_for<Resolve>(mut self, resolve: Resolve) -> Self
    where
        Resolve: Fn(&crate::PersonalAgentProfile) -> Vec<McpServer> + Send + Sync + 'static,
    {
        self.servers_for = Some(Arc::new(resolve));
        self
    }

    /// What the product is called on the page, when it is not SWEM.
    #[must_use]
    pub fn called(mut self, name: impl Into<String>) -> Self {
        self.called = Some(name.into());
        self
    }

    /// Whose this Workbench is, by the name the product knows them by.
    /// Without it the owner is named after the system's user, which is
    /// right on a computer of one's own and wrong on a server of many.
    #[must_use]
    pub fn owned_by(mut self, name: impl Into<String>) -> Self {
        self.owned_by = Some(name.into());
        self
    }

    /// Keep the profiles somewhere other than `<root>/profiles`.
    #[must_use]
    pub fn profiles_at(mut self, path: impl Into<PathBuf>) -> Self {
        self.profiles = Some(path.into());
        self
    }

    /// Keep the route ledger somewhere other than `<root>/routes.jsonl`.
    #[must_use]
    pub fn routes_at(mut self, path: impl Into<PathBuf>) -> Self {
        self.routes = Some(path.into());
        self
    }

    /// How long one agent operation may take.
    #[must_use]
    pub fn operation_timeout(mut self, timeout: Duration) -> Self {
        self.operation_timeout = timeout;
        self
    }

    /// A catalog this product carries before anything is added: what the
    /// Store lists first.
    #[must_use]
    pub fn shipped_catalog(mut self, catalog: Catalog) -> Self {
        self.shipped.push(catalog);
        self
    }

    /// The ACP registry index this product reads, instead of the public one
    /// or whatever the environment names.
    #[must_use]
    pub fn acp_registry(mut self, index: impl Into<String>) -> Self {
        self.acp_registry = Some(index.into());
        self
    }

    /// An MCP server declared for this run, attachable by name.
    #[must_use]
    pub fn declare(mut self, server: McpServer) -> Self {
        self.declarations.push(server);
        self
    }

    /// The image a container environment runs an agent in; without it the
    /// variable `SWEM_CONTAINER_IMAGE` is read, and without that a container
    /// profile is refused with the flag named.
    #[must_use]
    pub fn container_image(mut self, image: Option<String>) -> Self {
        self.container_image = image;
        self
    }

    /// Where the agent is inside that image.
    #[must_use]
    pub fn agent_in_image(mut self, path: impl Into<String>) -> Self {
        self.agent_in_image = path.into();
        self
    }

    /// The command the App relay observes servers through: an executable
    /// that runs the observer over its standard streams.
    #[must_use]
    pub fn mcp_observer(mut self, executable: PathBuf, args: Vec<String>) -> Self {
        self.mcp_observer = Some((executable, args));
        self
    }

    /// The command that serves an agent its own schedules: an executable
    /// that runs [`crate::serve_time_tools`] over its standard streams when
    /// given these arguments and then `--ledger`, `--agent` and `--chat`.
    #[must_use]
    pub fn time_tools(mut self, executable: PathBuf, args: Vec<String>) -> Self {
        self.time_tools = Some((executable, args));
        self
    }

    /// The command the system's scheduler starts when it keeps time for
    /// this product: an executable that, given these arguments, assembles
    /// the same product and calls [`Assembled::keep_time_once`].
    #[must_use]
    pub fn keep_time(mut self, executable: PathBuf, args: Vec<String>) -> Self {
        self.keep_time = Some((executable, args));
        self
    }

    /// Everything the product is, opened and enabled, with no door yet.
    ///
    /// # Errors
    ///
    /// The data root cannot be made, a declaration cannot be read, or the
    /// shell refuses a piece of its configuration - each in a sentence.
    #[allow(
        clippy::too_many_lines,
        reason = "one linear product assembly keeps paths, discovery, resolver and server ownership auditable"
    )]
    pub fn assemble(self) -> Result<Assembled, String> {
        let root = self.root;
        std::fs::create_dir_all(root.path())
            .map_err(|error| format!("create the data root {}: {error}", root.path().display()))?;
        // Under it are chats, keys and what agents wrote: nobody else who
        // has an account on this machine enters it.
        crate::closed::directory(root.path()).map_err(|error| {
            format!(
                "close the data root {} to others: {error}",
                root.path().display()
            )
        })?;
        let profiles = self.profiles.unwrap_or_else(|| root.profiles());
        let routes = self.routes.unwrap_or_else(|| root.routes());
        // The MCP declarations of this product, keyed by their ACP names and
        // shared with the shell, so a server declared while the product runs
        // is attachable by an agent session without a restart.
        let declarations: Arc<Mutex<BTreeMap<String, McpServer>>> = Arc::default();
        {
            let mut held = declarations
                .lock()
                .map_err(|_| "declaration registry poisoned")?;
            for server in self.declarations {
                held.insert(declaration_name(&server)?, server);
            }
        }
        let installed_root = adopted_installed_root(&root)?;
        let agents_root = root.agents();
        let declared_agents = crate::declared_agents(&agents_root)?;
        let discovered = crate::discover_agents_with(declared_agents.clone(), &installed_root);
        let container_image = self.container_image.or_else(|| {
            std::env::var(CONTAINER_IMAGE_VAR)
                .ok()
                .filter(|value| !value.trim().is_empty())
        });
        let agent_in_image = self.agent_in_image;
        let resolver_declarations = Arc::clone(&declarations);
        let resolver_servers_for = self.servers_for.clone();
        let resolver_providers = root.model_providers();
        let resolver_installed = installed_root.clone();
        let mut state = WorkbenchShellState::open_with_environment(
            &profiles,
            &routes,
            self.operation_timeout,
            move |profile| {
                // The profile names where its agent runs, and the catalogue is
                // asked before anything is built: a profile naming an
                // environment this product does not have is refused with the
                // name in the message rather than quietly started here.
                let backend = crate::environment_backend(&profile.environment_profile_id)?;
                let discovery =
                    crate::discover_agents_with(declared_agents.clone(), &resolver_installed)
                        .into_iter()
                        .find(|agent| agent.id == profile.agent_id)
                        .ok_or_else(|| format!("unknown agent: {}", profile.agent_id))?;
                if discovery.readiness == Readiness::Absent {
                    return Err(format!("agent is not installed: {}", profile.agent_id));
                }
                let launch = discovery
                    .launch
                    .clone()
                    .ok_or_else(|| "discovered agent has no launch command".to_owned())?;
                let agent_executable = discovery
                    .executable_path
                    .clone()
                    .ok_or_else(|| "discovered agent has no executable path".to_owned())?;
                let mut mcp_servers = Vec::new();
                let declared = resolver_declarations
                    .lock()
                    .map_err(|_| "declaration registry poisoned".to_owned())?;
                // An attachment whose server is not declared here is left
                // out, and the shell says so: the agent reaches less, and
                // the person can still talk to it and detach what is gone.
                for attachment in &profile.attachments {
                    if let Some(server) = declared.get(&attachment.server_name) {
                        mcp_servers.push(server.clone());
                    }
                }
                drop(declared);
                if let Some(servers_for) = &resolver_servers_for {
                    for server in servers_for(profile) {
                        let name = declaration_name(&server)?;
                        let named_already = mcp_servers
                            .iter()
                            .any(|had| declaration_name(had).is_ok_and(|had| had == name));
                        if !named_already {
                            mcp_servers.push(server);
                        }
                    }
                }
                match backend {
                    crate::EnvironmentBackend::ThisMachine => {
                        Ok(crate::ResolvedDirectAgentConnection {
                            launch,
                            agent_executable,
                            mcp_servers,
                        }
                        .into())
                    }
                    // The container is prepared here, before the session
                    // exists, because that is what the harness validates
                    // against: a connection arrives with one exact lease and
                    // one transport bound to it, or it does not arrive.
                    crate::EnvironmentBackend::InAContainer => resolver::in_a_container(
                        profile,
                        container_image.as_deref(),
                        &agent_in_image,
                        mcp_servers,
                        &resolver_providers,
                    ),
                }
            },
        )
        .map_err(|error| error.to_string())?;
        if let Some(index) = self.acp_registry {
            state
                .set_acp_registry_index(&index)
                .map_err(|error| error.to_string())?;
        }
        state
            .enable_local_onboarding(
                onboarding_options(discovered),
                &root.workspaces(),
                &root.agent_homes(),
            )
            .map_err(|error| error.to_string())?;
        // Discovery reads the agents directory, so what a person installs
        // from the product is there the next time it is asked. Without this
        // the shell answers from the list it was given at startup and calls a
        // freshly installed agent unavailable until a restart.
        let rediscover_root = root.clone();
        state.set_agent_discovery(move || {
            let declared = crate::declared_agents(&rediscover_root.agents()).unwrap_or_default();
            let installed = adopted_installed_root(&rediscover_root).unwrap_or_default();
            onboarding_options(crate::discover_agents_with(declared, &installed))
        });
        state
            .enable_installs(installed_root.clone())
            .map_err(|error| error.to_string())?;
        // The Store reads the registry and the catalogs a person adds; both
        // are kept under the data root so a machine that cannot reach them
        // today still sees what it saw. What the product ships is listed
        // before anything added: the one list says what this product is as
        // well as what it can be given.
        state
            .enable_store_with(&root.indexes(), self.shipped, self.takers)
            .map_err(|error| error.to_string())?;
        // The MCP servers a person declares from the product land in the same
        // declaration map as the ones the product itself declared, so an
        // agent attaches either kind by name.
        match &self.secrets {
            Some(keeper) => {
                state.keep_secrets_with(Arc::clone(keeper));
                state
                    .enable_mcp_catalogue_kept_by(Arc::clone(keeper), Arc::clone(&declarations))
                    .map_err(|error| error.to_string())?;
            }
            None => state
                .enable_mcp_catalogue(&root.mcp_servers(), Arc::clone(&declarations))
                .map_err(|error| error.to_string())?,
        }
        state
            .enable_model_providers(&root.model_providers())
            .map_err(|error| error.to_string())?;
        state
            .enable_machine_look(root.path())
            .map_err(|error| error.to_string())?;
        // After the providers: a key an agent held for its provider moves
        // to the provider, and that needs both.
        match &self.secrets {
            Some(keeper) => state
                .enable_provider_keys_kept_by(Arc::clone(keeper))
                .map_err(|error| error.to_string())?,
            None => state
                .enable_provider_keys(&root.keys())
                .map_err(|error| error.to_string())?,
        }
        if let Some((executable, args)) = self.mcp_observer {
            state.set_mcp_observer_command(executable, args);
        }
        if let Some((executable, args)) = self.time_tools {
            state.set_time_tools_command(executable, args);
        }
        state
            .enable_removal(&root.removed())
            .map_err(|error| error.to_string())?;
        state
            .enable_keepers(&root.time())
            .map_err(|error| error.to_string())?;
        if let Some((executable, args)) = self.keep_time {
            state.set_keep_time_command(executable, args);
        }
        if let Some(called) = &self.called {
            state.call_it(called);
        }
        let state = Arc::new(state);
        state.store_lives_in();
        Ok(Assembled {
            state,
            root,
            owned_by: self.owned_by,
        })
    }
}

/// The product, assembled; a door is opened onto it from here.
pub struct Assembled {
    pub state: Arc<WorkbenchShellState>,
    pub root: DataRoot,
    owned_by: Option<String>,
}

/// A Workbench served over HTTP: its handle, its address with this run's
/// secret, and the secret alone.
pub struct Served {
    pub handle: WorkbenchShellHandle,
    /// The address a browser opens, carrying the run's secret. Loopback is
    /// not a boundary: whoever can read this may work this Workbench.
    pub url: String,
    pub token: String,
}

impl Assembled {
    /// Name the owner as the product knows them, when it said.
    pub(crate) async fn name_the_owner(&self) -> Result<(), String> {
        match &self.owned_by {
            Some(name) => self
                .state
                .name_the_owner(name)
                .await
                .map_err(|error| error.to_string()),
            None => Ok(()),
        }
    }

    /// Open the Workbench door: mint this run's secret, start the clock of
    /// standing instructions, and serve the page and its App sandbox.
    ///
    /// # Errors
    ///
    /// The secret cannot be minted, the schedules cannot be opened, or a
    /// listener cannot be bound.
    pub async fn serve(
        self,
        bind: SocketAddr,
        sandbox_bind: SocketAddr,
        apps_bundle: Option<PathBuf>,
    ) -> Result<Served, String> {
        self.name_the_owner().await?;
        let token = crate::mint_session_token()?;
        self.state.set_session_token(token.clone());
        // The clock. A standing instruction runs where the product runs, so
        // it starts with the product and stops with it, and it holds the
        // same state the page does.
        self.state
            .keep_time(&self.root.schedules())
            .await
            .map_err(|error| error.to_string())?;
        // What this machine has is looked at while the door opens.
        self.state.look_at_the_machine_meanwhile();
        // What the chats were left owing when the product last stopped.
        self.state
            .take_up_chats()
            .await
            .map_err(|error| error.to_string())?;
        let handle = crate::serve_workbench_http_with_apps_at(
            Arc::clone(&self.state),
            bind,
            sandbox_bind,
            apps_bundle,
        )
        .await?;
        let url = format!(
            "http://127.0.0.1:{}/?token={token}",
            handle.local_addr.port()
        );
        Ok(Served { handle, url, token })
    }

    /// Keep time once and leave, as the system's scheduler has it done:
    /// what is due for the agents it keeps time for is said and answered,
    /// and nothing is left running. A Workbench that is open keeps time
    /// itself, and this leaves at once.
    ///
    /// # Errors
    ///
    /// The schedules or the ledger cannot be read.
    pub async fn keep_time_once(self) -> Result<crate::KeeperLook, String> {
        self.state
            .keep_time_once(&self.root.schedules(), crate::KEPT_BY_THE_SYSTEM)
            .await
            .map_err(|error| error.to_string())
    }

    /// Open the editor door: answer as the agent of `profile` over this
    /// process's standard streams, with no session secret and no clock.
    ///
    /// # Errors
    ///
    /// The door's own failure, in its words.
    pub async fn editor_door(self, profile: String) -> Result<(), String> {
        crate::serve_editor_door(self.state, profile)
            .await
            .map_err(|error| error.to_string())
    }
}

fn declaration_name(server: &McpServer) -> Result<String, String> {
    match server {
        McpServer::Stdio(stdio) => Ok(stdio.name.clone()),
        McpServer::Http(http) => Ok(http.name.clone()),
        McpServer::Sse(sse) => Ok(sse.name.clone()),
        other => Err(format!(
            "unsupported MCP transport in declaration: {other:?}"
        )),
    }
}

/// Where this product installs things. The first call on a machine an
/// earlier product installed agents on - into its own directory beside the
/// data root - brings those under this root, so there is one place to look
/// and nothing a person installed is lost.
fn adopted_installed_root(root: &DataRoot) -> Result<PathBuf, String> {
    let installed = root.installed();
    if let Some(legacy) = root.legacy_agents_home()
        && legacy.is_dir()
    {
        crate::adopt_legacy_agents(&legacy, &installed)
            .map_err(|error| format!("adopt earlier agent installs: {error}"))?;
    }
    Ok(installed)
}

/// What the onboarding surface lists, from what discovery found. An agent is
/// offered only when it is installed and the host can actually launch it.
fn onboarding_options(discovered: Vec<AgentDiscovery>) -> Vec<WorkbenchAgentOption> {
    discovered
        .into_iter()
        .map(|agent| WorkbenchAgentOption {
            agent_id: agent.id,
            name: agent.name,
            readiness: agent.readiness,
            available: matches!(
                agent.readiness,
                Readiness::InstalledUnverified | Readiness::HandshakeReady
            ) && agent.launch.is_some()
                && agent.executable_path.is_some(),
        })
        .collect()
}

impl std::fmt::Debug for Product {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Product")
            .field("root", &self.root)
            .field("acp_registry", &self.acp_registry)
            .finish_non_exhaustive()
    }
}
