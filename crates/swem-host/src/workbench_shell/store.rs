//! The Store as the Workbench has it: the library's Store
//! ([`swem_store::Store`]) over this product's indexes and install root,
//! with the harness as one of its hosts.
//!
//! The harness takes three kinds: an agent from the ACP registry, which
//! goes its own road and is rediscovered so the product offers it; an MCP
//! server from a catalog, declared once installed for a profile to attach,
//! keeping the values a person gave it before; and a skill, which waits
//! under the install root for a profile to take a copy. A product the
//! harness is built into takes kinds of its own the same way.
use std::path::Path;

use agent_client_protocol::schema::v1::EnvVariable;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{
    DeclareMcpServerBody, McpCatalogue, WorkbenchShellError, WorkbenchShellState, which_node,
};
use crate::{InstallReceipt, RegistryDistribution, SKILL_FILE};
pub use swem_store::{
    ACP_REGISTRY_CACHE, ArchiveDistribution, BinaryDistribution, CATALOG_SCHEMA, Catalog,
    CatalogDistribution, CatalogEntry, INDEX_SCHEMA, IndexFile, Kind, KindShown, KindWords,
    NpxDistribution, Planned, RegistryStatus, Requirement, Source, Store, StoreEntry, StoreError,
    StoreIndexView, StoreView, Taker, Takes, UvxDistribution, read_skill,
};

/// One entry named for a plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StorePlanBody {
    pub kind: Kind,
    pub id: String,
}

/// One entry installed against the plan the person saw.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreInstallBody {
    pub kind: Kind,
    pub id: String,
    pub plan_id: String,
}

/// One installed entry, to be removed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreRemoveBody {
    pub kind: Kind,
    pub id: String,
}

/// A catalog added by URL.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AddIndexBody {
    pub url: String,
}

/// An installed skill, read from its `SKILL.md`: what a profile takes a
/// copy of.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstalledSkill {
    pub id: String,
    pub version: String,
    pub name: String,
    pub description: String,
    pub body: String,
}

/// What a store refusal is to the page.
pub(super) fn store_refusal(error: StoreError) -> WorkbenchShellError {
    match error {
        StoreError::Invalid(words)
        | StoreError::UnsupportedDistribution(words)
        | StoreError::ConsentRequired(words) => WorkbenchShellError::Invalid(words),
        StoreError::NotFound(words) | StoreError::UnknownAgent(words) => {
            WorkbenchShellError::NotFound(words)
        }
        StoreError::Conflict(words) => WorkbenchShellError::Conflict(words),
        StoreError::Failed(words)
        | StoreError::Protocol(words)
        | StoreError::Serialization(words)
        | StoreError::MissingExecutable(words) => WorkbenchShellError::Failed(words),
    }
}

/// How the Store reaches a declared server that takes packages: through
/// the host, once the host is whole. The Store is made before the host is
/// shared, so the way to it is filled in after.
struct ServersHere {
    host: Arc<std::sync::OnceLock<std::sync::Weak<WorkbenchShellState>>>,
}

impl ServersHere {
    fn host(&self) -> Option<Arc<WorkbenchShellState>> {
        self.host.get().and_then(std::sync::Weak::upgrade)
    }
}

impl swem_store::Through for ServersHere {
    fn is_there(&self, server: &str) -> bool {
        self.host().is_some_and(|host| {
            host.mcp_catalogue.get().is_some_and(|catalogue| {
                catalogue
                    .declared()
                    .lock()
                    .is_ok_and(|declared| declared.contains_key(server))
            })
        })
    }

    fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let host = self
            .host()
            .ok_or_else(|| "the host is not there any more".to_owned())?;
        // The Store works on a blocking thread; the server is reached on
        // the runtime that thread belongs to.
        let handle = tokio::runtime::Handle::try_current()
            .map_err(|_| "the Store is not on a runtime".to_owned())?;
        tokio::task::block_in_place(|| {
            handle.block_on(host.call_declared_server(server, tool, arguments))
        })
        .map_err(|error| error.to_string())
    }
}

/// Agents: listed from the registry, installed by the harness's own road.
struct Agents;

impl Taker for Agents {
    fn kind(&self) -> Kind {
        Kind::AGENT
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: "an agent".into(),
            many: "Agents".into(),
            after_install: "It is offered in the Agent space; make one yours there.".into(),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        let for_this_machine = entry
            .distribution
            .binary
            .as_ref()
            .zip(swem_store::registry_platform())
            .is_some_and(|(binaries, platform)| binaries.contains_key(platform));
        if for_this_machine || entry.distribution.npx.is_some() || entry.distribution.uvx.is_some()
        {
            Ok(())
        } else {
            Err("no distribution for this machine".into())
        }
    }

    fn after_install(&self, _receipt: &InstallReceipt) -> Result<(), String> {
        // An agent goes the harness's own road, which rediscovers it.
        Ok(())
    }

    fn removable(&self) -> bool {
        false
    }
}

/// MCP servers: declared once installed, for a profile to attach.
struct Servers {
    catalogue: Arc<std::sync::OnceLock<McpCatalogue>>,
}

impl Taker for Servers {
    fn kind(&self) -> Kind {
        Kind::SERVER
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: "an MCP server".into(),
            many: "MCP servers".into(),
            after_install: "It is declared; attach it to an agent under Settings, Tools.".into(),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        if entry.distribution.archive.is_some()
            && entry.distribution.npx.is_none()
            && entry.distribution.binary.is_none()
            && entry.distribution.uvx.is_none()
        {
            return Err("a server is a program: an npx, uvx or binary distribution".into());
        }
        Ok(())
    }

    fn after_install(&self, receipt: &InstallReceipt) -> Result<(), String> {
        let catalogue = self
            .catalogue
            .get()
            .ok_or_else(|| "this host keeps no MCP catalogue".to_owned())?;
        // Declared with what launches it. The values a person gave the
        // server before are kept: an update is not a reason to type a key
        // again.
        let (command, args) = if let Some(entry_script) = &receipt.entry_script {
            let mut args = vec![entry_script.display().to_string()];
            args.extend(receipt.args.iter().cloned());
            ("node".to_owned(), args)
        } else {
            (
                receipt
                    .executable
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_default(),
                receipt.args.clone(),
            )
        };
        let env = catalogue.env_of(&receipt.registry_id);
        catalogue
            .declare(&DeclareMcpServerBody {
                name: receipt.registry_id.clone(),
                transport: "stdio".into(),
                command,
                args,
                env,
                url: String::new(),
                headers: Vec::new(),
            })
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn before_remove(&self, receipt: &InstallReceipt) -> Result<(), String> {
        if let Some(catalogue) = self.catalogue.get() {
            // A declaration that names files about to go is forgotten with
            // them; one the product itself made stays.
            let _ = catalogue.forget(&receipt.registry_id);
        }
        Ok(())
    }
}

/// Skills: a `SKILL.md` a profile takes a copy of.
struct Skills;

impl Taker for Skills {
    fn kind(&self) -> Kind {
        Kind::SKILL
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: "a skill".into(),
            many: "Skills".into(),
            after_install: "Give it to an agent under Settings, Skills.".into(),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        if entry.distribution.archive.is_none() {
            return Err("a skill is an archive holding a SKILL.md".into());
        }
        Ok(())
    }

    fn after_install(&self, _receipt: &InstallReceipt) -> Result<(), String> {
        Ok(())
    }
}

/// Channels: a program that answers the channel shape, run by the harness
/// for a bot a person adds under Providers, Channels.
struct Channels;

impl Taker for Channels {
    fn kind(&self) -> Kind {
        Kind::parse(swem_sdk::channel::CHANNEL_KIND).expect("the channel kind is a kind")
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: "a channel".into(),
            many: "Channels".into(),
            after_install: "Add a bot that runs on it under Providers, Channels.".into(),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        let distribution = &entry.distribution;
        if distribution.npx.is_none()
            && distribution.uvx.is_none()
            && distribution.binary.is_none()
            && distribution.archive.is_none()
        {
            return Err(
                "a channel is a program: an npm package, a Python package, a binary or an archive"
                    .into(),
            );
        }
        Ok(())
    }

    /// Started once from where it was staged, its tools listed and compared
    /// with the channel shape; refused, nothing lands.
    fn check(&self, staged: &Path, plan: &swem_sdk::InstallPlan) -> Result<(), String> {
        answers_the_shape(staged, plan, &swem_sdk::channel::shape(), "a channel")
    }

    fn after_install(&self, _receipt: &InstallReceipt) -> Result<(), String> {
        Ok(())
    }
}

/// Tunnels: a program that answers the tunnel shape, run by the harness to
/// stand at an address from outside for a while (ADR-0015).
struct Tunnels;

impl Taker for Tunnels {
    fn kind(&self) -> Kind {
        Kind::parse(swem_sdk::tunnel::TUNNEL_KIND).expect("the tunnel kind is a kind")
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: "a tunnel".into(),
            many: "Tunnels".into(),
            after_install: "Open it under Providers, Channels, or with /app to a bot.".into(),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        let distribution = &entry.distribution;
        if distribution.npx.is_none()
            && distribution.uvx.is_none()
            && distribution.binary.is_none()
            && distribution.archive.is_none()
        {
            return Err(
                "a tunnel is a program: an npm package, a Python package, a binary or an archive"
                    .into(),
            );
        }
        Ok(())
    }

    fn check(&self, staged: &Path, plan: &swem_sdk::InstallPlan) -> Result<(), String> {
        answers_the_shape(staged, plan, &swem_sdk::tunnel::shape(), "a tunnel")
    }

    fn after_install(&self, _receipt: &InstallReceipt) -> Result<(), String> {
        Ok(())
    }
}

/// Tools: a program another package `requires` and is handed the path of
/// when it starts (`swem_sdk::tunnel::tool_variable`). Checked by its
/// digest, as the Store checks everything, and by nothing else: a tool
/// answers no shape of ours.
struct Tools;

impl Taker for Tools {
    fn kind(&self) -> Kind {
        Kind::TOOL
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: "a tool".into(),
            many: "Tools".into(),
            after_install: "On hand for the packages that need it.".into(),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        let distribution = &entry.distribution;
        if distribution.binary.is_none() && distribution.archive.is_none() {
            return Err("a tool is a binary or an archive".into());
        }
        Ok(())
    }

    fn after_install(&self, _receipt: &InstallReceipt) -> Result<(), String> {
        Ok(())
    }
}

/// A staged program started once, its tools listed and compared with a
/// shape; refused in words when short. An npm or Python package has no
/// program before the install finishes and is checked when first started.
fn answers_the_shape(
    staged: &Path,
    plan: &swem_sdk::InstallPlan,
    shape: &swem_sdk::Shape,
    what: &str,
) -> Result<(), String> {
    let Some(program) = program_in(staged) else {
        return Ok(());
    };
    let stdio = agent_client_protocol::schema::v1::McpServerStdio::new(
        plan.registry_id.clone(),
        program.display().to_string(),
    );
    let listed = crate::workbench_apps::tools_listed_by_blocking(&stdio)?;
    shape
        .check(&listed)
        .map_err(|short| format!("{} is not {what}: it {short}", plan.name))
}

impl WorkbenchShellState {
    /// The tools on hand, each under `SWEM_TOOL_<ID>`, for the environment
    /// of every package process this host starts: a tunnel, a channel.
    /// What a package requires it finds there, as a program finds a
    /// database under `DATABASE_URL`, without knowing SWEM.
    pub(super) fn tools_in_environment(&self) -> Vec<EnvVariable> {
        let Ok(installed) = self.installed_root() else {
            return Vec::new();
        };
        swem_store::load_receipts(installed, &swem_sdk::Kind::TOOL)
            .into_iter()
            .filter_map(|(id, receipt)| {
                receipt.launch_path().map(|path| {
                    EnvVariable::new(
                        swem_sdk::tunnel::tool_variable(&id),
                        path.display().to_string(),
                    )
                })
            })
            .collect()
    }
}

/// The one program a staged binary or archive tree holds, if it is one.
fn program_in(staged: &Path) -> Option<std::path::PathBuf> {
    let tree = staged.join("tree");
    let mut files: Vec<std::path::PathBuf> =
        std::fs::read_dir(if tree.is_dir() { &tree } else { staged })
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path.file_name()
                        != Some(std::ffi::OsStr::new(swem_sdk::INSTALLATION_MANIFEST))
            })
            .collect();
    files.retain(|path| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            path.metadata()
                .is_ok_and(|meta| meta.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            path.extension().is_some_and(|ext| ext == "exe")
        }
    });
    (files.len() == 1).then(|| files.remove(0))
}

impl WorkbenchShellState {
    /// Read the store from `indexes`: the catalogs a person added and the
    /// last good copy of the registry, after the catalogs the distribution
    /// ships (`builtin`), with the harness's own takers.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the directories cannot be made,
    /// installs are not enabled, or the store is enabled already.
    pub fn enable_store(
        &self,
        indexes: &Path,
        builtin: Vec<Catalog>,
    ) -> Result<(), WorkbenchShellError> {
        self.enable_store_with(indexes, builtin, Vec::new())
    }

    /// As [`Self::enable_store`], with takers of a product's own.
    ///
    /// # Errors
    ///
    /// As [`Self::enable_store`].
    pub fn enable_store_with(
        &self,
        indexes: &Path,
        builtin: Vec<Catalog>,
        takers: Vec<Arc<dyn Taker>>,
    ) -> Result<(), WorkbenchShellError> {
        let mut store = Store::open(indexes, self.installed_root()?).map_err(store_refusal)?;
        for catalog in builtin {
            store.ship(catalog);
        }
        store.read_from(swem_store::AcpRegistry::new(
            self.acp_registry_index(),
            indexes,
        ));
        store.taken_by(Arc::new(Agents));
        store.taken_by(Arc::new(Servers {
            catalogue: Arc::clone(&self.mcp_catalogue),
        }));
        store.taken_by(Arc::new(Skills));
        store.taken_by(Arc::new(Channels));
        store.taken_by(Arc::new(Tunnels));
        store.taken_by(Arc::new(Tools));
        for taker in takers {
            store.taken_by(taker);
        }
        store.servers_take_through(Arc::new(ServersHere {
            host: Arc::clone(&self.host_of_the_store),
        }));
        self.store
            .set(store)
            .map_err(|_| WorkbenchShellError::Conflict("the store is already enabled".into()))
    }

    /// Tell the Store the host it lives in, once the host is shared: from
    /// then on servers that take packages can be reached.
    pub fn store_lives_in(self: &Arc<Self>) {
        let _ = self.host_of_the_store.set(Arc::downgrade(self));
    }

    fn store_home(&self) -> Result<&Store, WorkbenchShellError> {
        self.store
            .get()
            .ok_or_else(|| WorkbenchShellError::NotFound("this host has no store".into()))
    }

    /// Everything the indexes offer, with what is installed of it.
    ///
    /// # Errors
    ///
    /// Fails when this host has no store or an index cannot be read.
    pub fn store(&self) -> Result<StoreView, WorkbenchShellError> {
        self.store_home()?.view().map_err(store_refusal)
    }

    /// The exact plan installing one store entry would apply, with what it
    /// requires first.
    ///
    /// # Errors
    ///
    /// Refuses an entry no index lists, a kind nobody here takes, and a
    /// distribution this machine cannot run.
    pub fn store_plan(&self, body: &StorePlanBody) -> Result<Planned, WorkbenchShellError> {
        if body.kind == Kind::AGENT {
            return self.agent_install_plan(&body.id).map(|plan| Planned {
                plan,
                also: Vec::new(),
            });
        }
        self.store_home()?
            .plan(&body.kind, &body.id)
            .map_err(store_refusal)
    }

    /// Install one store entry against the plan the person saw, what it
    /// requires first, and put each where its kind is used from. The
    /// receipts, the entry's last.
    ///
    /// # Errors
    ///
    /// Refuses a moved plan and any install failure.
    pub fn store_install(
        &self,
        body: &StoreInstallBody,
    ) -> Result<Vec<InstallReceipt>, WorkbenchShellError> {
        if body.kind == Kind::AGENT {
            return self
                .install_agent(&body.id, &body.plan_id)
                .map(|receipt| vec![receipt]);
        }
        let store = self.store_home()?;
        let planned = store.plan(&body.kind, &body.id).map_err(store_refusal)?;
        let through_node = planned
            .also
            .iter()
            .chain(std::iter::once(&planned.plan))
            .any(|plan| matches!(plan.distribution, RegistryDistribution::Npx { .. }));
        let node = if through_node {
            Some(which_node().ok_or_else(|| {
                WorkbenchShellError::Invalid(
                    "this distribution runs through npx and node is not on PATH".into(),
                )
            })?)
        } else {
            None
        };
        store
            .install(&body.kind, &body.id, &body.plan_id, node.as_deref())
            .map_err(store_refusal)
    }

    /// Remove what was installed of one entry, after its taker had its say.
    ///
    /// # Errors
    ///
    /// Nothing of it is installed, something still requires it, or its kind
    /// is not removed from here.
    pub fn store_remove(
        &self,
        body: &StoreRemoveBody,
    ) -> Result<InstallReceipt, WorkbenchShellError> {
        self.store_home()?
            .remove(&body.kind, &body.id)
            .map_err(store_refusal)
    }

    /// Add a catalog by URL: fetched now, kept under the indexes directory
    /// with where it came from.
    ///
    /// # Errors
    ///
    /// Refuses a URL that does not answer with a catalog.
    pub fn add_index(&self, body: &AddIndexBody) -> Result<StoreIndexView, WorkbenchShellError> {
        self.store_home()?
            .add_index(&body.url)
            .map_err(store_refusal)
    }

    /// Forget one added catalog. What was installed from it stays installed.
    ///
    /// # Errors
    ///
    /// Refuses an unknown index.
    pub fn forget_index(&self, slug: &str) -> Result<(), WorkbenchShellError> {
        self.store_home()?.forget_index(slug).map_err(store_refusal)
    }

    /// Every installed skill, read from its `SKILL.md`.
    ///
    /// # Errors
    ///
    /// Fails when this host installs nothing.
    pub fn installed_skills(&self) -> Result<Vec<InstalledSkill>, WorkbenchShellError> {
        let mut skills = Vec::new();
        for (id, receipt) in swem_store::load_receipts(self.installed_root()?, &Kind::SKILL) {
            let Some(file) = &receipt.file else {
                continue;
            };
            if file.file_name().and_then(|name| name.to_str()) != Some(SKILL_FILE) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(file) else {
                continue;
            };
            let (name, description, body) = read_skill(&id, &text);
            skills.push(InstalledSkill {
                id,
                version: receipt.version,
                name,
                description,
                body,
            });
        }
        Ok(skills)
    }
}
