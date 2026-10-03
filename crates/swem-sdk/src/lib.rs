//! The vocabulary of the SWEM Store, for whoever extends it: what a package
//! is for (a kind), what a catalog says, what a plan and a receipt are, what
//! a host registers to take a kind (a taker), how a server says it takes a
//! kind, and the shape of tools a host checks a server against.
//!
//! Nothing here installs anything. The Store that reads indexes, plans,
//! fetches, stages and receipts is `swem-store`, and the harness that hosts
//! it is `swem-host`; both are AGPL. This crate is Apache-2.0 so that a host,
//! a taker, a catalog tool or a package can be written against it under any
//! licence, as `LICENSE-EXCEPTION.md` in the repository says.

#![forbid(unsafe_code)]

pub mod channel;
pub mod shape;
pub mod tunnel;

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use shape::{SHAPE_SCHEMA, Shape, ToolListed, ToolShape};

/// What was asked is not something the Store could use, and why.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct Invalid(pub String);

/// The document a catalog is. `@0.1` is read as a subset of `@0.2`.
pub const CATALOG_SCHEMA: &str = "swem:catalog@0.2";
/// The document a catalog was before kinds were open.
pub const CATALOG_SCHEMA_0_1: &str = "swem:catalog@0.1";

/// The receipt an installation leaves beside what it installed.
pub const INSTALLATION_MANIFEST: &str = "installation.json";

/// The schema every receipt written now names. A receipt without it was
/// written before the kinds were one installer; it is read as an agent's.
pub const INSTALL_RECEIPT_SCHEMA: &str = "swem:install-receipt@0.1";

/// The file installing a skill must find in its archive.
pub const SKILL_FILE: &str = "SKILL.md";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegistryDistribution {
    Npx {
        package: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Binary {
        platform: String,
        archive: String,
        sha256: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// A whole tree, digest-checked, with nothing to launch: how a skill
    /// arrives, and a package of another host's.
    Archive { url: String, sha256: String },
    /// A Python package installed by `uv` into its own environment.
    Uvx {
        package: String,
        #[serde(default)]
        args: Vec<String>,
    },
}

fn default_schema() -> String {
    INSTALL_RECEIPT_SCHEMA.to_owned()
}

/// The receipt of one installation: what was installed, from which plan,
/// and how to launch it. Exactly one launch path is present: `entry_script`
/// (an npm package, run by node) or `executable` (a native archive's entry).
///
/// The fields with defaults were added when the kinds became one installer;
/// a receipt written before then still reads, as an agent's with no plan id.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InstallReceipt {
    #[serde(default = "default_schema")]
    pub schema: String,
    #[serde(default)]
    pub kind: Kind,
    /// The exact plan this installation consented to.
    #[serde(default)]
    pub plan_id: String,
    /// Where the plan came from: the registry index, or `package:<module>`
    /// for a tool a package declared.
    #[serde(default)]
    pub source: String,
    pub registry_id: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_script: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
    /// The installed document itself, for a kind that is one rather than a
    /// program: a skill's `SKILL.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    pub args: Vec<String>,
    pub discovery_method: String,
    /// Seconds since the epoch when the receipt was written; zero for one
    /// written before receipts said.
    #[serde(default)]
    pub installed_at: u64,
}

impl InstallReceipt {
    /// What the installation is: the one of entry script, executable and
    /// file that is present, when it is a file on disk.
    #[must_use]
    pub fn launch_path(&self) -> Option<&Path> {
        let mut present = [&self.entry_script, &self.executable, &self.file]
            .into_iter()
            .flatten();
        match (present.next(), present.next()) {
            (Some(path), None) if path.is_file() => Some(path),
            // A whole tree, for a kind that is one.
            (Some(path), None) if path.is_dir() && !self.kind.is_built_in() => Some(path),
            _ => None,
        }
    }

    /// Whether this receipt is of exactly that plan.
    #[must_use]
    pub fn matches_plan(&self, plan: &InstallPlan) -> bool {
        if self.registry_id != plan.registry_id
            || self.name != plan.name
            || self.version != plan.version
            || self.args != plan.distribution.args()
            || (!self.plan_id.is_empty() && self.plan_id != plan.plan_id)
        {
            return false;
        }
        match &plan.distribution {
            RegistryDistribution::Npx { package, .. } => {
                self.package.as_deref() == Some(package)
                    && self.platform.is_none()
                    && self.archive.is_none()
                    && self.sha256.is_none()
                    && self.command.is_none()
                    && self.entry_script.is_some()
                    && self.executable.is_none()
            }
            RegistryDistribution::Binary {
                platform,
                archive,
                sha256,
                command,
                ..
            } => {
                self.package.is_none()
                    && self.platform.as_deref() == Some(platform)
                    && self.archive.as_deref() == Some(archive)
                    && self.sha256.as_deref() == Some(sha256)
                    && self.command.as_deref() == Some(command)
                    && self.entry_script.is_none()
                    && self.executable.is_some()
            }
            RegistryDistribution::Archive { url, sha256 } => {
                self.package.is_none()
                    && self.archive.as_deref() == Some(url)
                    && self.sha256.as_deref() == Some(sha256)
                    && self.command.is_none()
                    && self.entry_script.is_none()
                    && self.executable.is_none()
                    && self.file.is_some()
            }
            RegistryDistribution::Uvx { package, .. } => {
                self.package.as_deref() == Some(package)
                    && self.archive.is_none()
                    && self.entry_script.is_none()
                    && self.executable.is_some()
            }
        }
    }
}

impl RegistryDistribution {
    /// The arguments the launched program is given.
    #[must_use]
    pub fn args(&self) -> &[String] {
        match self {
            Self::Npx { args, .. } | Self::Binary { args, .. } | Self::Uvx { args, .. } => args,
            Self::Archive { .. } => &[],
        }
    }

    /// The platform a binary distribution is for.
    #[must_use]
    pub fn platform(&self) -> Option<&str> {
        match self {
            Self::Npx { .. } | Self::Archive { .. } | Self::Uvx { .. } => None,
            Self::Binary { platform, .. } => Some(platform),
        }
    }
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
    pub fn parse(name: &str) -> Result<Self, Invalid> {
        if name.is_empty()
            || name.len() > 96
            || !name.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/' | b'@')
            })
            || name.starts_with('.')
        {
            return Err(Invalid(format!("not the name of a kind: {name:?}")));
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
    pub fn parse(bytes: &[u8]) -> Result<Self, Invalid> {
        let catalog: Self = serde_json::from_slice(bytes)
            .map_err(|error| Invalid(format!("not a catalog: {error}")))?;
        if catalog.schema != CATALOG_SCHEMA && catalog.schema != CATALOG_SCHEMA_0_1 {
            return Err(Invalid(format!(
                "a catalog says {CATALOG_SCHEMA}; this one says {}",
                catalog.schema
            )));
        }
        if catalog.name.trim().is_empty() {
            return Err(Invalid("a catalog has a name".into()));
        }
        for entry in &catalog.entries {
            valid_entry_id(&entry.id)?;
            if !entry.bundled && entry.distribution.is_empty() {
                return Err(Invalid(format!(
                    "{} {} has no distribution and did not come with the product",
                    entry.kind, entry.id
                )));
            }
        }
        Ok(catalog)
    }
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

/// Whether an id is one a catalog entry may have.
///
/// # Errors
///
/// It is not.
pub fn valid_entry_id(id: &str) -> Result<(), Invalid> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Invalid(format!(
            "an entry's id holds letters, digits, '-', '_' and '.': {id:?}"
        )));
    }
    Ok(())
}
