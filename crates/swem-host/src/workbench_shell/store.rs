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
    /// Nothing of it is installed, or its kind is not removed from here.
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
