//! What a package is to the Workbench: the source a person names, the plan
//! they consent to, and the package as installed. Reading a manifest, loading
//! a package and installing one belong to the product root, which hands
//! the host a `ProductSupply`; the host keeps only the values a page shows
//! and a receipt names.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The receipt written beside a package's manifest: where it came from and
/// the plan it was installed against. A dotfile, so it cannot collide with
/// anything the package ships and the loader never reads it.
pub const PACKAGE_RECEIPT_FILE: &str = ".swem-install.json";

/// Where a staged package waits between the plan and the consent. It holds
/// no manifest of its own, so `load_packages` walks past it.
pub const PACKAGE_STAGING_DIR: &str = ".staging";

/// Where a package would come from, as the person names it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageSource {
    /// A directory on this machine. Copied, not linked: an installed package
    /// does not change under the Cycle because someone edited a tree
    /// elsewhere.
    Directory { path: PathBuf },
    /// An archive fetched over the network and checked against the digest
    /// the person named. `.tar.gz`, `.tgz` or `.zip`.
    Archive { url: String, sha256: String },
}

impl PackageSource {
    /// The source as one line, for the receipt and for the person.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Directory { path } => path.display().to_string(),
            Self::Archive { url, .. } => url.clone(),
        }
    }
}

/// What a package declares, as the person reads it before consenting. Counts
/// where a count is the whole of the meaning, names where the name is.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageDeclares {
    /// `component` or `none`. A package on disk cannot carry native code.
    pub behaviour: String,
    /// The node types a revision may use once this package is loaded.
    pub node_types: Vec<String>,
    /// Adapters the Cycle would spawn, by id - processes, so they are named.
    pub adapters: Vec<String>,
    /// Tools it would then want installed, by name.
    pub tools: Vec<String>,
    pub secret_types: Vec<String>,
    pub language_profiles: Vec<String>,
    pub predicates: Vec<String>,
    pub resources: usize,
    pub prompts: usize,
    pub realizations: usize,
    /// Runs of the Cycle's own tools it brings, and how many of those start a
    /// project rather than act on one.
    pub recipes: usize,
    pub seeds: usize,
    /// The tool that opens a slot of this kind, when the package is a domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<String>,
}

/// The plan a person confirms: what would be installed, from where, and what
/// it declares. Staged and loaded already, so every refusal a package can
/// earn has been earned before this is shown.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackagePlan {
    /// Exact identity of this plan: what the install consents to.
    pub plan_id: String,
    pub id: String,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub source: PackageSource,
    /// The digest of the staged tree, whatever the source was.
    pub sha256: String,
    pub declares: PackageDeclares,
    /// The version already installed under this id, when this would replace
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
}

/// One package installed in the plugins directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackageView {
    pub id: String,
    /// Whose package this is, which is what decides whether a person can
    /// replace it from the page.
    #[serde(default)]
    pub home: PackageHome,
    pub version: String,
    pub title: String,
    pub summary: String,
    pub directory: PathBuf,
    pub declares: PackageDeclares,
    /// From the receipt; absent for a package put here by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<PackageSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_at: Option<u64>,
    /// Whether this package loads. A tree the loader refuses is still shown,
    /// with why, rather than being silently missing from the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

/// Where a package was read from.
///
/// A host reads packages from more than one place: the `plugins` directory
/// beside the running executable, which is what a distribution ships, and
/// `<data root>/plugins`, which is what this person installed. Only the second
/// is theirs to replace.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageHome {
    /// Came with the product, or was pointed at when it was started.
    #[default]
    Product,
    /// Installed by this person, in their own data root.
    Yours,
}
