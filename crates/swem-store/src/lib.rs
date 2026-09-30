//! The Store: one list of what can be installed, from the indexes a host
//! reads, and one road to install it, for as many hosts as take what it
//! lists.
//!
//! A **kind** names what a package is for. The Store knows kinds by name
//! only; a **host** registers, for each kind it takes, the words for it,
//! what it accepts, how a candidate is checked before it is promised, and
//! what happens after it is installed ([`Taker`]). The harness takes agents,
//! MCP servers and skills; an installed server that says it takes a kind
//! takes packages for itself; a product the harness is built into takes
//! kinds of its own. A kind nobody on an installation takes is listed as
//! for something this installation does not have, and is not installable.
//!
//! Every install goes one road: the exact plan is shown, consented to by
//! its id, staged, checked by its taker, applied by one rename, receipted
//! under `<installed>/<kind>/<id>/<version>/`. What a package requires is
//! planned with it, and one consent covers the closure. Nothing here hosts a
//! registry: every index is consumed, never served.

mod install;
pub mod shape;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use shape::{SHAPE_SCHEMA, Shape, ToolListed, ToolShape};

pub use install::{
    INSTALL_RECEIPT_SCHEMA, INSTALLATION_MANIFEST, InstallReceipt, RegistryDistribution,
    SKILL_FILE, adopt_legacy_agents, all_receipts, fetch_url, install, load_receipts, newer,
    registry_platform, resolve_registry_install_plan, resolve_registry_install_plan_from_bytes,
};

/// The document a catalog is. `@0.1` is read as a subset of `@0.2`.
pub const CATALOG_SCHEMA: &str = "swem:catalog@0.2";
/// The document a catalog was before kinds were open.
pub const CATALOG_SCHEMA_0_1: &str = "swem:catalog@0.1";
/// The file an added catalog is kept in, with where it came from.
pub const INDEX_SCHEMA: &str = "swem:index@0.1";
/// The last good copy of the ACP registry, under the indexes directory.
pub const ACP_REGISTRY_CACHE: &str = "acp-registry.json";

/// What the Store answers when something is wrong.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum StoreError {
    /// What was asked is not something the Store can do, and why.
    #[error("{0}")]
    Invalid(String),
    /// Nothing of that name is there.
    #[error("not found: {0}")]
    NotFound(String),
    /// It cannot be done now; what was asked stays as it was.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Something under the Store failed.
    #[error("{0}")]
    Failed(String),
    #[error("installation requires explicit consent: {0}")]
    ConsentRequired(String),
    #[error("registry entry has no supported typed distribution: {0}")]
    UnsupportedDistribution(String),
    #[error("{0}")]
    Protocol(String),
    #[error("{0}")]
    Serialization(String),
    #[error("executable was not found: {0}")]
    MissingExecutable(String),
    #[error("unknown agent: {0}")]
    UnknownAgent(String),
}

/// What a package is for. The four built-in kinds keep the words they had
/// on the wire (`agent`, `tool`, `server`, `skill`); a kind of another
/// host's is named the way MCP names an extension, in reverse-DNS form
/// with a version, as `swem.cycle/package@1`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Kind(Cow<'static, str>);

impl Kind {
    /// An ACP agent: launched for a session.
    pub const AGENT: Self = Self(Cow::Borrowed("agent"));
    /// A tool a package declared its adapters need.
    pub const TOOL: Self = Self(Cow::Borrowed("tool"));
    /// An MCP server: declared for a profile to attach.
    pub const SERVER: Self = Self(Cow::Borrowed("server"));
    /// A skill: a `SKILL.md` a profile takes a copy of.
    pub const SKILL: Self = Self(Cow::Borrowed("skill"));

    /// The kinds the harness has always had, in the order receipts are
    /// listed.
    pub const BUILT_IN: [Self; 4] = [Self::AGENT, Self::TOOL, Self::SERVER, Self::SKILL];

    /// A kind by its name on the wire.
    ///
    /// # Errors
    ///
    /// A name that is not one: empty, longer than 96, or with characters
    /// other than letters, digits, `.`, `-`, `_`, `/` and `@`.
    pub fn parse(name: &str) -> Result<Self, StoreError> {
        if name.is_empty()
            || name.len() > 96
            || !name.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/' | b'@')
            })
            || name.starts_with('.')
        {
            return Err(StoreError::Invalid(format!(
                "not the name of a kind: {name:?}"
            )));
        }
        Ok(Self::BUILT_IN
            .into_iter()
            .find(|kind| kind.as_str() == name)
            .unwrap_or_else(|| Self(Cow::Owned(name.to_owned()))))
    }

    /// The kind's name on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether it is one of the harness's own.
    #[must_use]
    pub fn is_built_in(&self) -> bool {
        Self::BUILT_IN.contains(self)
    }

    /// The directory under the install root this kind lands in: the old
    /// names for the built-in kinds, and the kind's name with `/` and `@`
    /// made plain for the others.
    #[must_use]
    pub fn directory(&self) -> String {
        match self.as_str() {
            "agent" => "agents".into(),
            "tool" => "tools".into(),
            "server" => "servers".into(),
            "skill" => "skills".into(),
            other => other.replace('/', ".").replace('@', "-"),
        }
    }

    /// Every kind that has a directory under the install root, the
    /// built-in ones first.
    #[must_use]
    pub fn at(installed_root: &Path) -> Vec<Self> {
        let mut kinds = Self::BUILT_IN.to_vec();
        let known: Vec<String> = kinds.iter().map(Self::directory).collect();
        if let Ok(entries) = std::fs::read_dir(installed_root) {
            let mut others = entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
                .filter(|name| !known.contains(name) && !name.starts_with('.'))
                .map(|name| Self(Cow::Owned(name.replacen('.', "/", 1).replacen('-', "@", 1))))
                .collect::<Vec<_>>();
            others.sort();
            kinds.extend(others);
        }
        kinds
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Default for Kind {
    fn default() -> Self {
        Self::AGENT
    }
}

impl Serialize for Kind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Kind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::parse(&name).map_err(serde::de::Error::custom)
    }
}

/// What a plan installs, exactly.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstallPlan {
    pub plan_id: String,
    /// What the plan installs; an agent unless the plan says otherwise.
    #[serde(default)]
    pub kind: Kind,
    pub agent_id: String,
    pub registry_id: String,
    pub registry_index: String,
    pub name: String,
    pub version: String,
    pub distribution: RegistryDistribution,
    pub requires_explicit_consent: bool,
    pub executes_remote_shell: bool,
    /// What it requires, as the catalog said.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
}

/// An npm package run through node, as the ACP registry spells it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NpxDistribution {
    pub package: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// A Python package run through `uv`, as the ACP registry spells it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct UvxDistribution {
    /// The package, with its version pinned as `name==1.2.3`.
    pub package: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// A native archive for one platform, as the ACP registry spells it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryDistribution {
    pub archive: String,
    pub sha256: String,
    pub cmd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// A whole archive with a digest: how a skill, or a package of another
/// host's, is distributed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveDistribution {
    pub url: String,
    pub sha256: String,
}

/// How one catalog entry is distributed; the shape the ACP registry uses,
/// plus `archive` for a whole tree.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogDistribution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx: Option<NpxDistribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uvx: Option<UvxDistribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<BTreeMap<String, BinaryDistribution>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveDistribution>,
}

impl CatalogDistribution {
    /// Whether anything at all is named.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.npx.is_none() && self.uvx.is_none() && self.binary.is_none() && self.archive.is_none()
    }
}

/// What a package requires to be there: another package, by kind and id,
/// and a version it needs where that matters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Requirement {
    pub kind: Kind,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// A kind an installed server takes packages of, and the tools it takes
/// them with. The arguments are templates: `{tree}` is the staged package
/// directory, `{plan_id}` what the plan tool answered, `{id}` and
/// `{version}` the package's. The defaults are what the Cycle's hub takes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Takes {
    pub kind: Kind,
    /// The tool that plans a package from a staged directory and answers
    /// its plan id.
    pub plan: String,
    /// The tool that installs a planned package by its id.
    pub install: String,
    /// The tool that removes an installed package, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove: Option<String>,
    #[serde(default = "plan_arguments")]
    pub plan_arguments: serde_json::Value,
    /// The field of what the plan tool answered that holds the plan id.
    #[serde(default = "plan_id_field")]
    pub answers: String,
    #[serde(default = "install_arguments")]
    pub install_arguments: serde_json::Value,
    #[serde(default = "remove_arguments")]
    pub remove_arguments: serde_json::Value,
    /// What the kind is called, one and many, and what to say after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub one: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub many: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_install: Option<String>,
}

fn plan_arguments() -> serde_json::Value {
    serde_json::json!({"source": {"kind": "directory", "path": "{tree}"}})
}

fn plan_id_field() -> String {
    "plan_id".into()
}

fn install_arguments() -> serde_json::Value {
    serde_json::json!({"plan_id": "{plan_id}"})
}

fn remove_arguments() -> serde_json::Value {
    serde_json::json!({"id": "{id}"})
}

/// Every `{name}` in the strings of a value, filled in.
fn filled(template: &serde_json::Value, with: &[(&str, &str)]) -> serde_json::Value {
    match template {
        serde_json::Value::String(text) => {
            let mut text = text.clone();
            for (name, value) in with {
                text = text.replace(&format!("{{{name}}}"), value);
            }
            serde_json::Value::String(text)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(|item| filled(item, with)).collect())
        }
        serde_json::Value::Object(fields) => serde_json::Value::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), filled(value, with)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// How a host reaches a server that takes packages: whether the server is
/// there, and a call of one of its tools.
pub trait Through: Send + Sync {
    /// Whether the server of that name is declared here and can be called.
    fn is_there(&self, server: &str) -> bool;

    /// Call a tool of the server, and answer what it answered as one value.
    ///
    /// # Errors
    ///
    /// The server could not be reached, or refused.
    fn call(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String>;
}

/// A server that takes a kind, as the Store hands packages to it.
struct Delegated {
    server: String,
    takes: Takes,
    through: Arc<dyn Through>,
}

impl Delegated {
    /// The package's own directory in a tree: the tree itself when its files
    /// are at the top, else the one folder in it.
    fn package_in(tree: &Path) -> PathBuf {
        let mut entries = std::fs::read_dir(tree)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        if entries.len() == 1 && entries[0].is_dir() {
            entries.remove(0)
        } else {
            tree.to_path_buf()
        }
    }

    /// Ask the server to plan the package in `tree`, and answer the plan id
    /// it gave. The server reads the tree where it is: a plan made of a
    /// staged tree says whether the package would be taken; a plan made of
    /// the tree where it was put is the one installed.
    fn plan_with_the_server(&self, tree: &Path, id: &str, version: &str) -> Result<String, String> {
        let package = Self::package_in(tree).to_string_lossy().into_owned();
        let with = [("tree", package.as_str()), ("id", id), ("version", version)];
        let answered = self.through.call(
            &self.server,
            &self.takes.plan,
            filled(&self.takes.plan_arguments, &with),
        )?;
        let plan_id = answered
            .get(&self.takes.answers)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                format!(
                    "{} answered no {} for the package",
                    self.server, self.takes.answers
                )
            })?
            .to_owned();
        if let Some(named) = answered.get("id").and_then(serde_json::Value::as_str)
            && named != id
        {
            return Err(format!(
                "the package calls itself {named}; the catalog lists it as {id}"
            ));
        }
        Ok(plan_id)
    }
}

impl Taker for Delegated {
    fn kind(&self) -> Kind {
        self.takes.kind.clone()
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: self
                .takes
                .one
                .clone()
                .unwrap_or_else(|| format!("a package for {}", self.server)),
            many: self
                .takes
                .many
                .clone()
                .unwrap_or_else(|| format!("Packages for {}", self.server)),
            after_install: self
                .takes
                .after_install
                .clone()
                .unwrap_or_else(|| format!("{} has it.", self.server)),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        if entry.distribution.archive.is_none() {
            return Err(format!("a package for {} is an archive", self.server));
        }
        Ok(())
    }

    fn check(&self, staged: &Path, plan: &InstallPlan) -> Result<(), String> {
        self.plan_with_the_server(&staged.join("tree"), &plan.registry_id, &plan.version)
            .map(|_| ())
    }

    fn after_install(&self, receipt: &InstallReceipt) -> Result<(), String> {
        let tree = receipt
            .file
            .as_deref()
            .ok_or_else(|| "the receipt names no tree".to_owned())?;
        let plan_id = self.plan_with_the_server(tree, &receipt.registry_id, &receipt.version)?;
        let with = [
            ("plan_id", plan_id.as_str()),
            ("id", receipt.registry_id.as_str()),
            ("version", receipt.version.as_str()),
        ];
        self.through
            .call(
                &self.server,
                &self.takes.install,
                filled(&self.takes.install_arguments, &with),
            )
            .map(|_| ())
    }

    fn removable(&self) -> bool {
        self.takes.remove.is_some()
    }

    fn before_remove(&self, receipt: &InstallReceipt) -> Result<(), String> {
        let Some(remove) = &self.takes.remove else {
            return Err(format!("{} does not remove what it took", self.server));
        };
        let with = [
            ("id", receipt.registry_id.as_str()),
            ("version", receipt.version.as_str()),
        ];
        self.through
            .call(
                &self.server,
                remove,
                filled(&self.takes.remove_arguments, &with),
            )
            .map(|_| ())
    }
}

/// One thing a catalog offers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogEntry {
    pub kind: Kind,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub distribution: CatalogDistribution,
    /// The environment variable names a server needs given values; listed
    /// to the person, never filled in here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    /// The distribution carries it: nothing to fetch, listed so the store
    /// says what this product already is.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bundled: bool,
    /// What it requires to be installed as well.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    /// The kinds this entry, once installed, takes packages of.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub takes: Vec<Takes>,
}

/// A catalog document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Catalog {
    pub schema: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub entries: Vec<CatalogEntry>,
}

impl Catalog {
    /// Read a catalog document, refusing what no Store could use. What a
    /// particular host takes is asked when the catalog is added to one.
    ///
    /// # Errors
    ///
    /// Not a catalog, the wrong schema, an entry with no id or with a
    /// distribution nothing could apply.
    pub fn parse(bytes: &[u8]) -> Result<Self, StoreError> {
        let catalog: Self = serde_json::from_slice(bytes)
            .map_err(|error| StoreError::Invalid(format!("not a catalog: {error}")))?;
        if catalog.schema != CATALOG_SCHEMA && catalog.schema != CATALOG_SCHEMA_0_1 {
            return Err(StoreError::Invalid(format!(
                "a catalog says {CATALOG_SCHEMA}; this one says {}",
                catalog.schema
            )));
        }
        if catalog.name.trim().is_empty() {
            return Err(StoreError::Invalid("a catalog has a name".into()));
        }
        for entry in &catalog.entries {
            valid_entry_id(&entry.id)?;
            if !entry.bundled && entry.distribution.is_empty() {
                return Err(StoreError::Invalid(format!(
                    "{} {} has no distribution and did not come with the product",
                    entry.kind, entry.id
                )));
            }
        }
        Ok(catalog)
    }
}

/// An added catalog as kept on disk: the document and where it came from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndexFile {
    pub schema: String,
    pub url: String,
    pub fetched_at: u64,
    pub catalog: Catalog,
}

/// One index the store reads, as listed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreIndexView {
    pub slug: String,
    pub name: String,
    pub url: String,
    pub entries: usize,
    /// Shipped with this product rather than added by the person; it is not
    /// forgotten.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub builtin: bool,
}

/// One entry of the store, with what this machine has of it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "each is a fact of the row a page reads on the wire, not a mode"
)]
pub struct StoreEntry {
    pub kind: Kind,
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// The index it came from, by name.
    pub index: String,
    /// Whether this machine can install it: a distribution it can run, a
    /// host that takes its kind.
    pub installable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Environment variable names a server needs given values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs: Vec<String>,
    /// The installed version, when one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<String>,
    /// Carried by this product: neither installable nor installed, present.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bundled: bool,
    /// Whether somebody on this installation takes its kind.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taken: bool,
    /// What it requires, as the catalog said.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<Requirement>,
    /// The index has a newer version than the one installed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub newer: bool,
    /// Whether what is installed of it can be removed here.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removable: bool,
}

/// A kind, as a page names it: taken here or not.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KindShown {
    pub kind: Kind,
    pub one: String,
    pub many: String,
    /// What to tell a person once one is installed.
    pub after_install: String,
}

/// How the ACP registry was read for this listing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RegistryStatus {
    pub url: String,
    pub agents: usize,
    /// `live`, `cached` or `unavailable`.
    pub read: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The store as a page reads it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreView {
    pub registry: RegistryStatus,
    pub indexes: Vec<StoreIndexView>,
    pub entries: Vec<StoreEntry>,
    /// The kinds somebody here takes, in the order they were registered.
    #[serde(default)]
    pub kinds: Vec<KindShown>,
}

/// One entry planned, with what it requires planned before it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Planned {
    #[serde(flatten)]
    pub plan: InstallPlan,
    /// What is installed first because the entry requires it and it is
    /// not there yet, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<InstallPlan>,
}

/// Words for a kind, as its taker says them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KindWords {
    pub one: String,
    pub many: String,
    pub after_install: String,
}

/// A host's part in the Store: who takes packages of a kind.
pub trait Taker: Send + Sync {
    /// The kind taken.
    fn kind(&self) -> Kind;

    /// The words for the kind on a page.
    fn words(&self) -> KindWords;

    /// Whether an entry of this kind, as a catalog lists it, is one the
    /// taker could take: its distribution is of a shape the taker runs.
    ///
    /// # Errors
    ///
    /// Why not, in words for the person.
    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String>;

    /// Look at what was staged before it is promised: a manifest read, a
    /// server's tools listed against a shape. Refused, nothing lands.
    ///
    /// # Errors
    ///
    /// Why not, in words for the person.
    fn check(&self, _staged: &Path, _plan: &InstallPlan) -> Result<(), String> {
        Ok(())
    }

    /// What happens once it landed: a server declared, an agent
    /// rediscovered, a package handed to the hub.
    ///
    /// # Errors
    ///
    /// What went wrong after the install, which stays installed.
    fn after_install(&self, receipt: &InstallReceipt) -> Result<(), String>;

    /// Whether what is installed of this kind may be removed from the page.
    fn removable(&self) -> bool {
        true
    }

    /// What happens before an installed one is removed.
    ///
    /// # Errors
    ///
    /// Why it is not removed.
    fn before_remove(&self, _receipt: &InstallReceipt) -> Result<(), String> {
        Ok(())
    }
}

/// An index the Store reads that is not a catalog file: the ACP registry.
pub trait Source: Send + Sync {
    /// What the index is called on the page.
    fn name(&self) -> String;
    /// Where it is read from.
    fn url(&self) -> String;
    /// Its entries, and how it was read.
    fn read(&self) -> (Vec<CatalogEntry>, SourceRead);
}

/// How a source was read.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceRead {
    /// `live`, `cached` or `unavailable`.
    pub read: String,
    pub error: Option<String>,
}

/// The ACP registry as a source: fetched live and kept as the last good
/// copy under the indexes directory.
pub struct AcpRegistry {
    url: String,
    cache: PathBuf,
}

impl AcpRegistry {
    #[must_use]
    pub fn new(url: impl Into<String>, indexes: &Path) -> Self {
        Self {
            url: url.into(),
            cache: indexes.join(ACP_REGISTRY_CACHE),
        }
    }

    /// The registry's bytes: fetched now and kept, or the copy kept before.
    fn bytes(&self) -> (Vec<u8>, SourceRead) {
        match fetch_url(&self.url) {
            Ok(bytes) => {
                let _ = std::fs::write(&self.cache, &bytes);
                (
                    bytes,
                    SourceRead {
                        read: "live".into(),
                        error: None,
                    },
                )
            }
            Err(error) => match std::fs::read(&self.cache) {
                Ok(bytes) => (
                    bytes,
                    SourceRead {
                        read: "cached".into(),
                        error: Some(error.to_string()),
                    },
                ),
                Err(_) => (
                    Vec::new(),
                    SourceRead {
                        read: "unavailable".into(),
                        error: Some(error.to_string()),
                    },
                ),
            },
        }
    }
}

impl Source for AcpRegistry {
    fn name(&self) -> String {
        "ACP registry".into()
    }

    fn url(&self) -> String {
        self.url.clone()
    }

    fn read(&self) -> (Vec<CatalogEntry>, SourceRead) {
        let (bytes, how) = self.bytes();
        if bytes.is_empty() {
            return (Vec::new(), how);
        }
        let Ok(index) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            return (
                Vec::new(),
                SourceRead {
                    read: "unavailable".into(),
                    error: Some("the registry is not a document this Store reads".into()),
                },
            );
        };
        let mut entries = Vec::new();
        for agent in index["agents"].as_array().into_iter().flatten() {
            let Some(id) = agent["id"].as_str() else {
                continue;
            };
            let distribution: CatalogDistribution =
                serde_json::from_value(agent["distribution"].clone()).unwrap_or_default();
            entries.push(CatalogEntry {
                kind: Kind::AGENT,
                id: id.to_owned(),
                name: agent["name"].as_str().unwrap_or(id).to_owned(),
                description: agent["description"].as_str().unwrap_or_default().to_owned(),
                version: agent["version"].as_str().unwrap_or_default().to_owned(),
                distribution,
                env: Vec::new(),
                bundled: false,
                requires: Vec::new(),
                takes: Vec::new(),
            });
        }
        (entries, how)
    }
}

/// The Store of one installation.
pub struct Store {
    indexes: PathBuf,
    installed: PathBuf,
    /// The catalogs the distribution ships, listed before the added ones.
    shipped: Vec<Catalog>,
    sources: Vec<Box<dyn Source>>,
    takers: Vec<Arc<dyn Taker>>,
    /// How a server that takes a kind is reached, when the host lets
    /// servers take.
    through: Option<Arc<dyn Through>>,
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The file name a catalog is kept under: its name, lowered, with runs of
/// anything else as one dash.
///
/// # Errors
///
/// A name that gives no file name.
pub fn slug_of(name: &str) -> Result<String, StoreError> {
    let mut slug = String::new();
    let mut dash = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
            dash = false;
        } else if !dash && !slug.is_empty() {
            slug.push('-');
            dash = true;
        }
    }
    let slug = slug.trim_end_matches('-').to_owned();
    if slug.is_empty() || slug.len() > 64 {
        return Err(StoreError::Invalid(format!(
            "a catalog's name must give it a file name: {name:?}"
        )));
    }
    Ok(slug)
}

fn valid_slug(slug: &str) -> Result<(), StoreError> {
    if slug.is_empty()
        || slug.len() > 64
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(StoreError::Invalid(format!(
            "not the name of an index: {slug:?}"
        )));
    }
    Ok(())
}

fn valid_entry_id(id: &str) -> Result<(), StoreError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(StoreError::Invalid(format!(
            "an entry's id holds letters, digits, '-', '_' and '.': {id:?}"
        )));
    }
    Ok(())
}

/// The distribution of one catalog entry this machine can apply.
///
/// # Errors
///
/// Why none can be: it came with the product, or nothing is for this
/// platform.
pub fn distribution_of(entry: &CatalogEntry) -> Result<RegistryDistribution, String> {
    if entry.bundled {
        return Err("it came with this product; there is nothing to fetch".to_owned());
    }
    if let Some(archive) = &entry.distribution.archive {
        return Ok(RegistryDistribution::Archive {
            url: archive.url.clone(),
            sha256: archive.sha256.to_ascii_lowercase(),
        });
    }
    let platform = registry_platform().ok_or_else(|| {
        format!(
            "no distribution for {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    if let Some(binary) = entry
        .distribution
        .binary
        .as_ref()
        .and_then(|binaries| binaries.get(platform))
    {
        return Ok(RegistryDistribution::Binary {
            platform: platform.into(),
            archive: binary.archive.clone(),
            sha256: binary.sha256.to_ascii_lowercase(),
            command: binary.cmd.clone(),
            args: binary.args.clone(),
        });
    }
    if let Some(npx) = &entry.distribution.npx {
        return Ok(RegistryDistribution::Npx {
            package: npx.package.clone(),
            args: npx.args.clone(),
        });
    }
    if let Some(uvx) = &entry.distribution.uvx {
        return Ok(RegistryDistribution::Uvx {
            package: uvx.package.clone(),
            args: uvx.args.clone(),
        });
    }
    Err(format!("no distribution for {platform}"))
}

/// The exact plan for one catalog entry: its identity is the digest of
/// everything the install would act on.
///
/// # Errors
///
/// No distribution this machine can apply.
pub fn plan_of(entry: &CatalogEntry, index_url: &str) -> Result<InstallPlan, StoreError> {
    let distribution = distribution_of(entry).map_err(StoreError::Invalid)?;
    let plan_bytes = serde_json::to_vec(&(
        entry.kind.as_str(),
        &entry.id,
        index_url,
        &entry.name,
        &entry.version,
        &distribution,
        &entry.requires,
    ))
    .map_err(|error| StoreError::Failed(error.to_string()))?;
    Ok(InstallPlan {
        plan_id: format!("sha256:{:x}", Sha256::digest(plan_bytes)),
        kind: entry.kind.clone(),
        agent_id: entry.id.clone(),
        registry_id: entry.id.clone(),
        registry_index: index_url.to_owned(),
        name: entry.name.clone(),
        version: entry.version.clone(),
        distribution,
        requires_explicit_consent: true,
        executes_remote_shell: false,
        requires: entry.requires.clone(),
    })
}

/// Whether an installed version satisfies what is required of it.
fn satisfies(installed: &str, required: Option<&str>) -> bool {
    let Some(required) = required else {
        return true;
    };
    match (
        semver::VersionReq::parse(required),
        semver::Version::parse(installed),
    ) {
        (Ok(required), Ok(installed)) => required.matches(&installed),
        _ => installed == required,
    }
}

impl Store {
    /// The Store over `indexes` (the catalogs a person added) and
    /// `installed` (what was installed), both made if absent.
    ///
    /// # Errors
    ///
    /// A directory cannot be made.
    pub fn open(indexes: &Path, installed: &Path) -> Result<Self, StoreError> {
        for directory in [indexes, installed] {
            std::fs::create_dir_all(directory)
                .map_err(|error| StoreError::Failed(format!("{}: {error}", directory.display())))?;
        }
        Ok(Self {
            indexes: indexes.to_path_buf(),
            installed: installed.to_path_buf(),
            shipped: Vec::new(),
            sources: Vec::new(),
            takers: Vec::new(),
            through: None,
        })
    }

    /// A catalog the distribution ships: listed first, never forgotten.
    pub fn ship(&mut self, catalog: Catalog) {
        self.shipped.push(catalog);
    }

    /// An index that is not a catalog file.
    pub fn read_from(&mut self, source: impl Source + 'static) {
        self.sources.push(Box::new(source));
    }

    /// Somebody who takes packages of a kind.
    pub fn taken_by(&mut self, taker: Arc<dyn Taker>) {
        self.takers.retain(|had| had.kind() != taker.kind());
        self.takers.push(taker);
    }

    /// Where what is installed lands.
    #[must_use]
    pub fn installed_root(&self) -> &Path {
        &self.installed
    }

    /// Let servers that say they take a kind take it, reached `through`
    /// the host.
    pub fn servers_take_through(&mut self, through: Arc<dyn Through>) {
        self.through = Some(through);
    }

    /// Who takes a kind here, if anybody: a taker the host registered, or a
    /// server that is there and whose catalog entry says it takes the kind.
    #[must_use]
    pub fn taker(&self, kind: &Kind) -> Option<Arc<dyn Taker>> {
        if let Some(taker) = self.takers.iter().find(|taker| &taker.kind() == kind) {
            return Some(Arc::clone(taker));
        }
        let through = self.through.as_ref()?;
        let added = self.index_files().unwrap_or_default();
        let listed = self
            .shipped
            .iter()
            .chain(added.iter().map(|(_, index)| &index.catalog));
        for entry in listed.flat_map(|catalog| catalog.entries.iter()) {
            if let Some(takes) = entry.takes.iter().find(|takes| &takes.kind == kind)
                && through.is_there(&entry.id)
            {
                return Some(Arc::new(Delegated {
                    server: entry.id.clone(),
                    takes: takes.clone(),
                    through: Arc::clone(through),
                }));
            }
        }
        None
    }

    /// Every receipt of one kind, newest version of each.
    #[must_use]
    pub fn receipts(&self, kind: &Kind) -> BTreeMap<String, InstallReceipt> {
        load_receipts(&self.installed, kind)
    }

    fn index_files(&self) -> Result<Vec<(String, IndexFile)>, StoreError> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.indexes)
            .map_err(|error| StoreError::Failed(error.to_string()))?
            .flatten()
        {
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || path.file_name().and_then(|value| value.to_str()) == Some(ACP_REGISTRY_CACHE)
            {
                continue;
            }
            let Some(slug) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            let bytes =
                std::fs::read(&path).map_err(|error| StoreError::Failed(error.to_string()))?;
            let index: IndexFile = serde_json::from_slice(&bytes).map_err(|error| {
                StoreError::Failed(format!("{} is not an index: {error}", path.display()))
            })?;
            found.push((slug.to_owned(), index));
        }
        found.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(found)
    }

    /// Why an entry cannot be installed here, or nothing.
    fn why_not(&self, entry: &CatalogEntry) -> Option<String> {
        if entry.bundled {
            return None;
        }
        let Some(taker) = self.taker(&entry.kind) else {
            return Some(format!(
                "for something this installation does not have ({})",
                entry.kind
            ));
        };
        if let Err(why) = taker.accepts(entry) {
            return Some(why);
        }
        distribution_of(entry).err()
    }

    /// Everything the indexes offer, with what is installed of it.
    ///
    /// # Errors
    ///
    /// An index cannot be read.
    pub fn view(&self) -> Result<StoreView, StoreError> {
        let mut installed: BTreeMap<Kind, BTreeMap<String, InstallReceipt>> = BTreeMap::new();
        let mut version_of = |kind: &Kind, id: &str| -> Option<String> {
            installed
                .entry(kind.clone())
                .or_insert_with(|| load_receipts(&self.installed, kind))
                .get(id)
                .map(|receipt| receipt.version.clone())
        };
        let mut entries = Vec::new();
        let mut registry = RegistryStatus {
            url: String::new(),
            agents: 0,
            read: "unavailable".into(),
            error: None,
        };
        for source in &self.sources {
            let (found, how) = source.read();
            registry = RegistryStatus {
                url: source.url(),
                agents: found.len(),
                read: how.read,
                error: how.error,
            };
            for entry in found {
                let reason = self.why_not(&entry);
                let installed = version_of(&entry.kind, &entry.id);
                entries.push(self.shown(&entry, &source.name(), reason, installed));
            }
        }
        let mut indexes = Vec::new();
        let shipped = self.shipped.iter().map(|catalog| {
            (
                slug_of(&catalog.name).unwrap_or_default(),
                String::new(),
                catalog,
                true,
            )
        });
        let added = self.index_files()?;
        let listed = shipped.chain(
            added
                .iter()
                .map(|(slug, index)| (slug.clone(), index.url.clone(), &index.catalog, false)),
        );
        for (slug, url, catalog, builtin) in listed {
            indexes.push(StoreIndexView {
                slug,
                name: catalog.name.clone(),
                url,
                entries: catalog.entries.len(),
                builtin,
            });
            for entry in &catalog.entries {
                let reason = self.why_not(entry);
                let installed = version_of(&entry.kind, &entry.id);
                entries.push(self.shown(entry, &catalog.name, reason, installed));
            }
        }
        Ok(StoreView {
            registry,
            indexes,
            entries,
            kinds: self.kinds_shown(),
        })
    }

    /// The kinds taken here, the host's own first, then what servers that
    /// are there take.
    fn kinds_shown(&self) -> Vec<KindShown> {
        let mut shown: Vec<KindShown> = self
            .takers
            .iter()
            .map(|taker| {
                let words = taker.words();
                KindShown {
                    kind: taker.kind(),
                    one: words.one,
                    many: words.many,
                    after_install: words.after_install,
                }
            })
            .collect();
        if let Some(through) = &self.through {
            let added = self.index_files().unwrap_or_default();
            let listed = self
                .shipped
                .iter()
                .chain(added.iter().map(|(_, index)| &index.catalog));
            for entry in listed.flat_map(|catalog| catalog.entries.iter()) {
                for takes in &entry.takes {
                    if shown.iter().any(|had| had.kind == takes.kind)
                        || !through.is_there(&entry.id)
                    {
                        continue;
                    }
                    let words = Delegated {
                        server: entry.id.clone(),
                        takes: takes.clone(),
                        through: Arc::clone(through),
                    }
                    .words();
                    shown.push(KindShown {
                        kind: takes.kind.clone(),
                        one: words.one,
                        many: words.many,
                        after_install: words.after_install,
                    });
                }
            }
        }
        shown
    }

    fn shown(
        &self,
        entry: &CatalogEntry,
        index: &str,
        reason: Option<String>,
        installed: Option<String>,
    ) -> StoreEntry {
        let newer = installed.as_deref().is_some_and(|had| {
            match (
                semver::Version::parse(had),
                semver::Version::parse(&entry.version),
            ) {
                (Ok(had), Ok(offered)) => offered > had,
                _ => had != entry.version,
            }
        });
        let taker = self.taker(&entry.kind);
        let removable_here =
            installed.is_some() && taker.as_ref().is_some_and(|taker| taker.removable());
        StoreEntry {
            kind: entry.kind.clone(),
            id: entry.id.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
            version: entry.version.clone(),
            index: index.to_owned(),
            installable: reason.is_none() && !entry.bundled,
            reason,
            needs: entry.env.clone(),
            removable: removable_here,
            installed,
            bundled: entry.bundled,
            taken: taker.is_some(),
            requires: entry.requires.clone(),
            newer,
        }
    }

    /// The catalog entry `id` of `kind`, with the URL of the index it is in.
    ///
    /// # Errors
    ///
    /// No index lists it.
    pub fn entry(&self, kind: &Kind, id: &str) -> Result<(CatalogEntry, String), StoreError> {
        if let Some(entry) = self
            .shipped
            .iter()
            .flat_map(|catalog| catalog.entries.iter())
            .find(|entry| &entry.kind == kind && entry.id == id)
        {
            return Ok((entry.clone(), String::new()));
        }
        for (_, index) in self.index_files()? {
            if let Some(entry) = index
                .catalog
                .entries
                .iter()
                .find(|entry| &entry.kind == kind && entry.id == id)
            {
                return Ok((entry.clone(), index.url));
            }
        }
        for source in &self.sources {
            let (entries, _) = source.read();
            if let Some(entry) = entries
                .into_iter()
                .find(|entry| &entry.kind == kind && entry.id == id)
            {
                return Ok((entry, source.url()));
            }
        }
        Err(StoreError::NotFound(format!(
            "no index lists a {kind} named {id}"
        )))
    }

    /// The exact plan installing one entry would apply, with what it
    /// requires planned before it where that is not installed yet.
    ///
    /// # Errors
    ///
    /// An entry no index lists, a kind nobody here takes, a distribution
    /// this machine cannot run, or a requirement nothing lists.
    pub fn plan(&self, kind: &Kind, id: &str) -> Result<Planned, StoreError> {
        let (entry, index_url) = self.entry(kind, id)?;
        if let Some(why) = self.why_not(&entry) {
            return Err(StoreError::Invalid(why));
        }
        if entry.bundled {
            return Err(StoreError::Invalid(
                "it came with this product; there is nothing to fetch".into(),
            ));
        }
        let plan = plan_of(&entry, &index_url)?;
        let mut also = Vec::new();
        for required in &entry.requires {
            let had = self
                .receipts(&required.kind)
                .get(&required.id)
                .map(|receipt| receipt.version.clone());
            let bundled = self
                .shipped
                .iter()
                .flat_map(|catalog| catalog.entries.iter())
                .any(|entry| {
                    entry.kind == required.kind && entry.id == required.id && entry.bundled
                });
            if bundled
                || had
                    .as_deref()
                    .is_some_and(|had| satisfies(had, required.version.as_deref()))
            {
                continue;
            }
            let (found, found_url) = self.entry(&required.kind, &required.id).map_err(|_| {
                StoreError::Invalid(format!(
                    "{} {} requires {} {}, which no index here lists",
                    entry.kind, entry.id, required.kind, required.id
                ))
            })?;
            if !satisfies(&found.version, required.version.as_deref()) {
                return Err(StoreError::Invalid(format!(
                    "{} {} requires {} {} {}, and what is listed is {}",
                    entry.kind,
                    entry.id,
                    required.kind,
                    required.id,
                    required.version.as_deref().unwrap_or_default(),
                    found.version
                )));
            }
            if let Some(why) = self.why_not(&found) {
                return Err(StoreError::Invalid(format!(
                    "{} {} requires {} {}, which cannot be installed here: {why}",
                    entry.kind, entry.id, required.kind, required.id
                )));
            }
            also.push(plan_of(&found, &found_url)?);
        }
        Ok(Planned { plan, also })
    }

    /// Install one entry against the plan the person saw, what it requires
    /// first, and hand each to its taker. The receipts, in the order they
    /// landed.
    ///
    /// # Errors
    ///
    /// A moved plan, a taker's refusal, or the install's own failure.
    pub fn install(
        &self,
        kind: &Kind,
        id: &str,
        plan_id: &str,
        node: Option<&Path>,
    ) -> Result<Vec<InstallReceipt>, StoreError> {
        let planned = self.plan(kind, id)?;
        if planned.plan.plan_id != plan_id {
            return Err(StoreError::Conflict(format!(
                "the install plan changed since it was shown ({} now); read it again",
                planned.plan.plan_id
            )));
        }
        let mut receipts = Vec::new();
        for plan in planned.also.iter().chain(std::iter::once(&planned.plan)) {
            let taker = self
                .taker(&plan.kind)
                .ok_or_else(|| StoreError::Invalid(format!("nobody here takes {}", plan.kind)))?;
            let check = |staged: &Path| taker.check(staged, plan).map_err(StoreError::Invalid);
            let receipt = install::install_checked(plan, true, &self.installed, node, &check)?;
            taker.after_install(&receipt).map_err(StoreError::Failed)?;
            receipts.push(receipt);
        }
        Ok(receipts)
    }

    /// Remove what was installed of `id` of `kind`, every version, after
    /// its taker had its say.
    ///
    /// # Errors
    ///
    /// Nothing of it is installed, its taker does not allow it, or the
    /// files will not go.
    pub fn remove(&self, kind: &Kind, id: &str) -> Result<InstallReceipt, StoreError> {
        let receipt = self
            .receipts(kind)
            .remove(id)
            .ok_or_else(|| StoreError::NotFound(format!("nothing of {kind} {id} is installed")))?;
        let taker = self
            .taker(kind)
            .ok_or_else(|| StoreError::Invalid(format!("nobody here takes {kind}")))?;
        if !taker.removable() {
            return Err(StoreError::Invalid(format!(
                "a {kind} is not removed from here"
            )));
        }
        taker.before_remove(&receipt).map_err(StoreError::Invalid)?;
        valid_entry_id(id)?;
        let directory = self.installed.join(kind.directory()).join(id);
        std::fs::remove_dir_all(&directory)
            .map_err(|error| StoreError::Failed(format!("{}: {error}", directory.display())))?;
        Ok(receipt)
    }

    /// Add a catalog by URL: fetched now, kept under the indexes directory
    /// with where it came from.
    ///
    /// # Errors
    ///
    /// Refuses a URL that does not answer with a catalog.
    pub fn add_index(&self, url: &str) -> Result<StoreIndexView, StoreError> {
        let url = url.trim();
        if url.is_empty() {
            return Err(StoreError::Invalid("an index has a URL".into()));
        }
        let bytes = fetch_url(url).map_err(|error| StoreError::Invalid(error.to_string()))?;
        let catalog = Catalog::parse(&bytes)?;
        let slug = slug_of(&catalog.name)?;
        let index = IndexFile {
            schema: INDEX_SCHEMA.into(),
            url: url.to_owned(),
            fetched_at: now_seconds(),
            catalog,
        };
        let path = self.indexes.join(format!("{slug}.json"));
        let temporary = self
            .indexes
            .join(format!(".{slug}.{}.tmp", std::process::id()));
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&index)
                .map_err(|error| StoreError::Failed(error.to_string()))?,
        )
        .map_err(|error| StoreError::Failed(error.to_string()))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            StoreError::Failed(error.to_string())
        })?;
        Ok(StoreIndexView {
            slug,
            name: index.catalog.name,
            url: index.url,
            entries: index.catalog.entries.len(),
            builtin: false,
        })
    }

    /// Forget one added catalog. What was installed from it stays installed.
    ///
    /// # Errors
    ///
    /// Refuses an unknown index and one the product ships.
    pub fn forget_index(&self, slug: &str) -> Result<(), StoreError> {
        valid_slug(slug)?;
        if self
            .shipped
            .iter()
            .any(|catalog| slug_of(&catalog.name).ok().as_deref() == Some(slug))
        {
            return Err(StoreError::Invalid(format!(
                "{slug} came with this product and is not forgotten"
            )));
        }
        let path = self.indexes.join(format!("{slug}.json"));
        if !path.is_file() {
            return Err(StoreError::NotFound(format!("no index named {slug}")));
        }
        std::fs::remove_file(&path).map_err(|error| StoreError::Failed(error.to_string()))
    }
}

/// The name and description of a skill from its `SKILL.md` front matter,
/// and the body after it. A file without front matter is all body.
#[must_use]
pub fn read_skill(id: &str, text: &str) -> (String, String, String) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (id.to_owned(), String::new(), text.trim_end().to_owned());
    };
    let Some((front, body)) = rest.split_once("\n---\n") else {
        return (id.to_owned(), String::new(), text.trim_end().to_owned());
    };
    let mut name = id.to_owned();
    let mut description = String::new();
    for line in front.lines() {
        if let Some(value) = line.strip_prefix("name:") {
            value.trim().clone_into(&mut name);
        } else if let Some(value) = line.strip_prefix("description:") {
            value.trim().clone_into(&mut description);
        }
    }
    (name, description, body.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kind_is_a_name_on_the_wire_and_a_directory_on_disk() {
        assert_eq!(Kind::parse("server").unwrap(), Kind::SERVER);
        assert_eq!(Kind::SERVER.directory(), "servers");
        let cycle = Kind::parse("swem.cycle/package@1").unwrap();
        assert!(!cycle.is_built_in());
        assert_eq!(cycle.directory(), "swem.cycle.package-1");
        assert!(Kind::parse("").is_err());
        assert!(Kind::parse("../x").is_err());
        assert!(Kind::parse("with space").is_err());
        let json = serde_json::to_string(&cycle).unwrap();
        assert_eq!(json, "\"swem.cycle/package@1\"");
        assert_eq!(serde_json::from_str::<Kind>(&json).unwrap(), cycle);
    }

    #[test]
    fn a_catalog_is_read_in_both_schemas_and_a_wrong_one_refused() {
        let old = br#"{"schema":"swem:catalog@0.1","name":"My Tools","entries":[
            {"kind":"server","id":"echo","name":"Echo","version":"1","distribution":{"npx":{"package":"echo@1"}}},
            {"kind":"skill","id":"shout","name":"Shout","version":"1","distribution":{"archive":{"url":"https://x/s.tar.gz","sha256":"AB"}}}
        ]}"#;
        let catalog = Catalog::parse(old).unwrap();
        assert_eq!(catalog.entries.len(), 2);
        assert_eq!(slug_of(&catalog.name).unwrap(), "my-tools");
        // A digest is a digest however it was typed.
        assert!(matches!(
            distribution_of(&catalog.entries[1]).unwrap(),
            RegistryDistribution::Archive { sha256, .. } if sha256 == "ab"
        ));
        let new = br#"{"schema":"swem:catalog@0.2","name":"Cycle","entries":[
            {"kind":"swem.cycle/package@1","id":"hello-node","name":"Hello","version":"0.1.0",
             "distribution":{"archive":{"url":"https://x/h.tar.gz","sha256":"cd"}},
             "requires":[{"kind":"server","id":"swem-cycle"}]},
            {"kind":"server","id":"swem-cycle","name":"Cycle","version":"1","bundled":true,
             "takes":[{"kind":"swem.cycle/package@1","plan":"plan_package","install":"install_package"}]}
        ]}"#;
        let catalog = Catalog::parse(new).unwrap();
        assert_eq!(catalog.entries[0].requires[0].id, "swem-cycle");
        assert_eq!(catalog.entries[1].takes[0].plan, "plan_package");
        let wrong_schema = br#"{"schema":"other","name":"x","entries":[]}"#;
        assert!(Catalog::parse(wrong_schema).is_err());
        let nothing_to_fetch = br#"{"schema":"swem:catalog@0.2","name":"x","entries":[{"kind":"server","id":"x","name":"x","version":"1"}]}"#;
        assert!(Catalog::parse(nothing_to_fetch).is_err());
    }

    #[test]
    fn a_skill_file_is_read_by_its_front_matter() {
        let (name, description, body) = read_skill(
            "shout",
            "---\nname: Shout\ndescription: when asked loudly\n---\n\nSHOUT.\n",
        );
        assert_eq!(name, "Shout");
        assert_eq!(description, "when asked loudly");
        assert_eq!(body, "SHOUT.");
    }

    #[test]
    fn a_requirement_is_satisfied_by_a_version_in_its_range() {
        assert!(satisfies("1.2.3", None));
        assert!(satisfies("1.2.3", Some("^1.0")));
        assert!(!satisfies("2.0.0", Some("^1.0")));
        assert!(satisfies("0.1.0-dev", Some("0.1.0-dev")));
    }
}
