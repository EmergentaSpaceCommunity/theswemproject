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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use swem_sdk::*;

pub use install::{
    adopt_legacy_agents, all_receipts, fetch_url, install, load_receipts, newer, registry_platform,
    resolve_registry_install_plan, resolve_registry_install_plan_from_bytes,
};

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

impl From<Invalid> for StoreError {
    fn from(Invalid(words): Invalid) -> Self {
        Self::Invalid(words)
    }
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
    /// Whether what is installed of it can be removed here: its kind is
    /// removed from here, and nothing installed or bundled requires it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removable: bool,
    /// What still requires it, installed or bundled, each as `kind id`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_by: Vec<String>,
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

/// What is being planned on one walk of the requirements.
#[derive(Default)]
struct Closure {
    /// Requirements planned so far, in the order they are installed.
    also: Vec<InstallPlan>,
    /// Each planned requirement: who first required it, at which version.
    planned: BTreeMap<(Kind, String), (String, String)>,
    /// The entries being walked, top first: a requirement found here is a
    /// circle.
    path: Vec<String>,
}

fn label(kind: &Kind, id: &str) -> String {
    format!("{kind} {id}")
}

/// The id of a plan with what it requires: the entry's own plan with the
/// plans of its whole closure, in order, so that consent is to all of it.
fn closure_id(plan: &InstallPlan, also: &[InstallPlan]) -> String {
    if also.is_empty() {
        return plan.plan_id.clone();
    }
    let mut hasher = Sha256::new();
    hasher.update(plan.plan_id.as_bytes());
    for required in also {
        hasher.update(b"\n");
        hasher.update(required.plan_id.as_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
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
        let required_by = if installed.is_some() {
            self.required_by(&entry.kind, &entry.id)
        } else {
            Vec::new()
        };
        let removable_here = installed.is_some()
            && required_by.is_empty()
            && taker.as_ref().is_some_and(|taker| taker.removable());
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
            required_by,
        }
    }

    /// What requires `id` of `kind` on this installation: every installed
    /// package whose receipt names it, and every bundled entry that does,
    /// each as `kind id`.
    #[must_use]
    pub fn required_by(&self, kind: &Kind, id: &str) -> Vec<String> {
        let needs_it = |requires: &[Requirement]| {
            requires
                .iter()
                .any(|required| &required.kind == kind && required.id == id)
        };
        let installed = all_receipts(&self.installed)
            .into_iter()
            .filter(|receipt| needs_it(&receipt.requires))
            .map(|receipt| label(&receipt.kind, &receipt.registry_id));
        let bundled = self
            .shipped
            .iter()
            .flat_map(|catalog| catalog.entries.iter())
            .filter(|entry| entry.bundled && needs_it(&entry.requires))
            .map(|entry| label(&entry.kind, &entry.id));
        let mut found: Vec<String> = installed.chain(bundled).collect();
        found.sort();
        found.dedup();
        found
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

    /// The exact plan installing one entry would apply, with everything it
    /// requires, and everything that requires, planned before it where that
    /// is not installed yet: requirements before what requires them, each
    /// once. The plan's id is the digest of the whole closure, so a
    /// requirement that moved since the plan was shown moves the id.
    ///
    /// # Errors
    ///
    /// An entry no index lists, a kind nobody here takes, a distribution
    /// this machine cannot run, a requirement nothing lists, a requirement
    /// two entries want at versions that do not meet, or entries that
    /// require each other in a circle.
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
        let mut closure = Closure::default();
        closure.path.push(label(&entry.kind, &entry.id));
        self.plan_requirements(&entry, &mut closure)?;
        let plan_id = closure_id(&plan, &closure.also);
        Ok(Planned {
            plan: InstallPlan { plan_id, ..plan },
            also: closure.also,
        })
    }

    /// Plan what `entry` requires, depth first, into `closure`: a
    /// requirement that is installed or bundled at a version that satisfies
    /// is left as it is; one already planned on another path is checked
    /// against this path's version and not planned twice; one on the path
    /// being walked is a circle.
    fn plan_requirements(
        &self,
        entry: &CatalogEntry,
        closure: &mut Closure,
    ) -> Result<(), StoreError> {
        for required in &entry.requires {
            let key = (required.kind.clone(), required.id.clone());
            let wanted = required.version.as_deref();
            if closure.path.contains(&label(&required.kind, &required.id)) {
                return Err(StoreError::Invalid(format!(
                    "{} require each other in a circle",
                    closure
                        .path
                        .iter()
                        .map(String::as_str)
                        .chain(std::iter::once(
                            label(&required.kind, &required.id).as_str()
                        ))
                        .collect::<Vec<_>>()
                        .join(" → ")
                )));
            }
            if let Some((by, version)) = closure.planned.get(&key) {
                if !satisfies(version, wanted) {
                    return Err(StoreError::Invalid(format!(
                        "{} {} requires {} {} {}, and {by} requires it at {version}: they do not meet",
                        entry.kind,
                        entry.id,
                        required.kind,
                        required.id,
                        wanted.unwrap_or_default()
                    )));
                }
                continue;
            }
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
            if bundled || had.as_deref().is_some_and(|had| satisfies(had, wanted)) {
                continue;
            }
            let (found, found_url) = self.entry(&required.kind, &required.id).map_err(|_| {
                StoreError::Invalid(format!(
                    "{} {} requires {} {}, which no index here lists",
                    entry.kind, entry.id, required.kind, required.id
                ))
            })?;
            if !satisfies(&found.version, wanted) {
                return Err(StoreError::Invalid(format!(
                    "{} {} requires {} {} {}, and what is listed is {}",
                    entry.kind,
                    entry.id,
                    required.kind,
                    required.id,
                    wanted.unwrap_or_default(),
                    found.version
                )));
            }
            if let Some(why) = self.why_not(&found) {
                return Err(StoreError::Invalid(format!(
                    "{} {} requires {} {}, which cannot be installed here: {why}",
                    entry.kind, entry.id, required.kind, required.id
                )));
            }
            closure
                .planned
                .insert(key, (label(&entry.kind, &entry.id), found.version.clone()));
            closure.path.push(label(&found.kind, &found.id));
            self.plan_requirements(&found, closure)?;
            closure.path.pop();
            closure.also.push(plan_of(&found, &found_url)?);
        }
        Ok(())
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
        // Consent was to the closure; what is written beside the entry is
        // the entry's own plan, so that it still matches once what it
        // required is installed and planned no more.
        let (entry, index_url) = self.entry(kind, id)?;
        let own = plan_of(&entry, &index_url)?;
        let mut receipts = Vec::new();
        for plan in planned.also.iter().chain(std::iter::once(&own)) {
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
    /// Nothing of it is installed, something installed or bundled still
    /// requires it, its taker does not allow it, or the files will not go.
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
        let required_by = self.required_by(kind, id);
        if !required_by.is_empty() {
            return Err(StoreError::Invalid(format!(
                "{kind} {id} is still required by {}; remove that first",
                required_by.join(", ")
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
