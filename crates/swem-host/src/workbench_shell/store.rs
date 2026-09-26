//! The store: one list of what a person can install, from the indexes this
//! product reads.
//!
//! Two kinds of index feed it. The ACP registry lists agents; the product
//! fetches it live and keeps the last good copy under the indexes directory,
//! so a machine that cannot reach it today still sees what it saw. A catalog
//! is a document a person adds by URL (`swem:catalog@0.1`): MCP servers and
//! skills, each with a distribution the one installer already knows - an npm
//! package run through node, a native archive per platform with a digest, or
//! a whole archive with a digest for a skill. Nothing here hosts a registry;
//! every index is consumed, never served.
//!
//! Installing goes the road every install here goes: the exact plan is
//! shown, consented to by its id, applied, receipted. What differs by kind is
//! what happens after: an agent is rediscovered so the product offers it, a
//! server is declared in the MCP catalogue for a profile to attach, a skill
//! waits under the install root for a profile to take a copy.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{DeclareMcpServerBody, WorkbenchShellError, WorkbenchShellState, which_node};
use crate::{
    InstallKind, InstallPlan, InstallReceipt, RegistryDistribution, SKILL_FILE, fetch_url,
    load_receipts, registry_platform,
};

/// The document a catalog is.
pub const CATALOG_SCHEMA: &str = "swem:catalog@0.1";
/// The file an added catalog is kept in, with where it came from.
pub const INDEX_SCHEMA: &str = "swem:index@0.1";
/// The last good copy of the ACP registry, under the indexes directory.
pub const ACP_REGISTRY_CACHE: &str = "acp-registry.json";

/// An npm package run through node, as the ACP registry spells it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NpxDistribution {
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

/// A whole archive with a digest: how a skill is distributed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveDistribution {
    pub url: String,
    pub sha256: String,
}

/// How one catalog entry is distributed; the shape the ACP registry uses,
/// plus `archive` for a skill.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogDistribution {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npx: Option<NpxDistribution>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<BTreeMap<String, BinaryDistribution>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<ArchiveDistribution>,
}

/// One thing a catalog offers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogEntry {
    pub kind: InstallKind,
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
    /// Read a catalog document, refusing what the store cannot use.
    ///
    /// # Errors
    ///
    /// Not a catalog, the wrong schema, an entry of a kind a catalog does
    /// not list, or a server or skill with no distribution it could have.
    pub fn parse(bytes: &[u8]) -> Result<Self, WorkbenchShellError> {
        parse_catalog(bytes)
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
pub struct StoreEntry {
    pub kind: InstallKind,
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    /// The index it came from, by name.
    pub index: String,
    /// Whether this machine can install it: a distribution it can run.
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
}

/// One entry named for a plan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StorePlanBody {
    pub kind: InstallKind,
    pub id: String,
}

/// One entry installed against the plan the person saw.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StoreInstallBody {
    pub kind: InstallKind,
    pub id: String,
    pub plan_id: String,
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

pub(super) struct StoreHome {
    indexes: PathBuf,
    /// The catalogs the distribution ships, listed before the added ones.
    builtin: Vec<Catalog>,
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The file name a catalog is kept under: its name, lowered, with runs of
/// anything else as one dash.
fn slug_of(name: &str) -> Result<String, WorkbenchShellError> {
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
        return Err(WorkbenchShellError::Invalid(format!(
            "a catalog's name must give it a file name: {name:?}"
        )));
    }
    Ok(slug)
}

fn valid_slug(slug: &str) -> Result<(), WorkbenchShellError> {
    if slug.is_empty()
        || slug.len() > 64
        || !slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(WorkbenchShellError::Invalid(format!(
            "not the name of an index: {slug:?}"
        )));
    }
    Ok(())
}

fn valid_entry_id(id: &str) -> Result<(), WorkbenchShellError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(WorkbenchShellError::Invalid(format!(
            "an entry's id holds letters, digits, '-', '_' and '.': {id:?}"
        )));
    }
    Ok(())
}

/// Read a catalog document and refuse what the store cannot use.
fn parse_catalog(bytes: &[u8]) -> Result<Catalog, WorkbenchShellError> {
    let catalog: Catalog = serde_json::from_slice(bytes)
        .map_err(|error| WorkbenchShellError::Invalid(format!("not a catalog: {error}")))?;
    if catalog.schema != CATALOG_SCHEMA {
        return Err(WorkbenchShellError::Invalid(format!(
            "a catalog says {CATALOG_SCHEMA}; this one says {}",
            catalog.schema
        )));
    }
    if catalog.name.trim().is_empty() {
        return Err(WorkbenchShellError::Invalid("a catalog has a name".into()));
    }
    for entry in &catalog.entries {
        valid_entry_id(&entry.id)?;
        match entry.kind {
            InstallKind::Server if entry.bundled => {}
            InstallKind::Server => {
                if entry.distribution.npx.is_none() && entry.distribution.binary.is_none() {
                    return Err(WorkbenchShellError::Invalid(format!(
                        "server {} has neither an npx nor a binary distribution",
                        entry.id
                    )));
                }
            }
            InstallKind::Skill => {
                if entry.distribution.archive.is_none() {
                    return Err(WorkbenchShellError::Invalid(format!(
                        "skill {} has no archive distribution",
                        entry.id
                    )));
                }
            }
            InstallKind::Agent | InstallKind::Tool => {
                return Err(WorkbenchShellError::Invalid(format!(
                    "a catalog lists servers and skills; {} is {}",
                    entry.id,
                    entry.kind.as_str()
                )));
            }
        }
    }
    Ok(catalog)
}

/// The distribution of one catalog entry this machine can apply.
fn distribution_of(entry: &CatalogEntry) -> Result<RegistryDistribution, String> {
    if entry.bundled {
        return Err("it came with this product; there is nothing to fetch".to_owned());
    }
    if entry.kind == InstallKind::Skill {
        return entry
            .distribution
            .archive
            .as_ref()
            .map(|archive| RegistryDistribution::Archive {
                url: archive.url.clone(),
                sha256: archive.sha256.clone(),
            })
            .ok_or_else(|| "no archive distribution".to_owned());
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
    Err(format!("no distribution for {platform}"))
}

/// The exact plan for one catalog entry: its identity is the digest of
/// everything the install would act on.
fn plan_of(entry: &CatalogEntry, index_url: &str) -> Result<InstallPlan, WorkbenchShellError> {
    let distribution = distribution_of(entry).map_err(WorkbenchShellError::Invalid)?;
    let plan_bytes = serde_json::to_vec(&(
        entry.kind.as_str(),
        &entry.id,
        index_url,
        &entry.name,
        &entry.version,
        &distribution,
    ))
    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
    Ok(InstallPlan {
        plan_id: format!("sha256:{:x}", Sha256::digest(plan_bytes)),
        kind: entry.kind,
        agent_id: entry.id.clone(),
        registry_id: entry.id.clone(),
        registry_index: index_url.to_owned(),
        name: entry.name.clone(),
        version: entry.version.clone(),
        distribution,
        requires_explicit_consent: true,
        executes_remote_shell: false,
    })
}

/// The name and description of a skill from its `SKILL.md` front matter,
/// and the body after it. A file without front matter is all body.
fn read_skill(id: &str, text: &str) -> (String, String, String) {
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

impl WorkbenchShellState {
    /// Read the store from `indexes`: the catalogs a person added and the
    /// last good copy of the registry, after the catalogs the distribution
    /// ships (`builtin`).
    ///
    /// # Errors
    ///
    /// Refuses a second enabling, and a directory that cannot be made.
    pub fn enable_store(
        &self,
        indexes: &Path,
        builtin: Vec<Catalog>,
    ) -> Result<(), WorkbenchShellError> {
        std::fs::create_dir_all(indexes)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.store
            .set(StoreHome {
                indexes: indexes.to_path_buf(),
                builtin,
            })
            .map_err(|_| WorkbenchShellError::Conflict("the store is already enabled".into()))
    }

    fn store_home(&self) -> Result<&StoreHome, WorkbenchShellError> {
        self.store
            .get()
            .ok_or_else(|| WorkbenchShellError::NotFound("this host has no store".into()))
    }

    fn index_files(&self) -> Result<Vec<(String, IndexFile)>, WorkbenchShellError> {
        let home = self.store_home()?;
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&home.indexes)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
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
            let bytes = std::fs::read(&path)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            let index: IndexFile = serde_json::from_slice(&bytes).map_err(|error| {
                WorkbenchShellError::Failed(format!("{} is not an index: {error}", path.display()))
            })?;
            found.push((slug.to_owned(), index));
        }
        found.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(found)
    }

    /// The registry's bytes: fetched now and kept, or the copy kept before.
    fn registry_bytes(&self) -> Result<(Vec<u8>, String, Option<String>), WorkbenchShellError> {
        let home = self.store_home()?;
        let cache = home.indexes.join(ACP_REGISTRY_CACHE);
        match fetch_url(&self.acp_registry_index()) {
            Ok(bytes) => {
                let _ = std::fs::write(&cache, &bytes);
                Ok((bytes, "live".into(), None))
            }
            Err(error) => match std::fs::read(&cache) {
                Ok(bytes) => Ok((bytes, "cached".into(), Some(error.to_string()))),
                Err(_) => Ok((Vec::new(), "unavailable".into(), Some(error.to_string()))),
            },
        }
    }

    /// Everything the indexes offer, with what is installed of it.
    ///
    /// # Errors
    ///
    /// Fails when this host has no store or an index cannot be read.
    pub fn store(&self) -> Result<StoreView, WorkbenchShellError> {
        let installed_root = self.installed_root()?.to_path_buf();
        let installed: BTreeMap<InstallKind, BTreeMap<String, InstallReceipt>> = InstallKind::ALL
            .into_iter()
            .map(|kind| (kind, load_receipts(&installed_root, kind)))
            .collect();
        let version_of = |kind: InstallKind, id: &str| -> Option<String> {
            installed
                .get(&kind)
                .and_then(|receipts| receipts.get(id))
                .map(|receipt| receipt.version.clone())
        };
        let mut entries = Vec::new();

        let (bytes, read, error) = self.registry_bytes()?;
        let platform = registry_platform();
        let mut agents = 0;
        if !bytes.is_empty() {
            let index: serde_json::Value = serde_json::from_slice(&bytes)
                .map_err(|error| WorkbenchShellError::Failed(format!("registry: {error}")))?;
            for agent in index["agents"].as_array().into_iter().flatten() {
                let Some(id) = agent["id"].as_str() else {
                    continue;
                };
                agents += 1;
                let has_binary = platform.is_some_and(|platform| {
                    agent["distribution"]["binary"].get(platform).is_some()
                });
                let has_npx = agent["distribution"].get("npx").is_some();
                entries.push(StoreEntry {
                    kind: InstallKind::Agent,
                    id: id.to_owned(),
                    name: agent["name"].as_str().unwrap_or(id).to_owned(),
                    description: agent["description"].as_str().unwrap_or_default().to_owned(),
                    version: agent["version"].as_str().unwrap_or_default().to_owned(),
                    index: "ACP registry".into(),
                    installable: has_binary || has_npx,
                    reason: (!(has_binary || has_npx))
                        .then(|| "no distribution for this machine".to_owned()),
                    needs: Vec::new(),
                    installed: version_of(InstallKind::Agent, id),
                    bundled: false,
                });
            }
        }

        let mut indexes = Vec::new();
        let home = self.store_home()?;
        let shipped = home.builtin.iter().map(|catalog| {
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
                let applicable = distribution_of(entry);
                entries.push(StoreEntry {
                    kind: entry.kind,
                    id: entry.id.clone(),
                    name: entry.name.clone(),
                    description: entry.description.clone(),
                    version: entry.version.clone(),
                    index: catalog.name.clone(),
                    installable: applicable.is_ok(),
                    reason: if entry.bundled {
                        None
                    } else {
                        applicable.err()
                    },
                    needs: entry.env.clone(),
                    installed: version_of(entry.kind, &entry.id),
                    bundled: entry.bundled,
                });
            }
        }
        Ok(StoreView {
            registry: RegistryStatus {
                url: self.acp_registry_index(),
                agents,
                read,
                error,
            },
            indexes,
            entries,
        })
    }

    /// The catalog entry `id` of `kind`, with the URL of the index it is in.
    fn catalog_entry(
        &self,
        kind: InstallKind,
        id: &str,
    ) -> Result<(CatalogEntry, String), WorkbenchShellError> {
        if let Some(entry) = self
            .store_home()?
            .builtin
            .iter()
            .flat_map(|catalog| catalog.entries.iter())
            .find(|entry| entry.kind == kind && entry.id == id)
        {
            return Ok((entry.clone(), String::new()));
        }
        for (_, index) in self.index_files()? {
            if let Some(entry) = index
                .catalog
                .entries
                .iter()
                .find(|entry| entry.kind == kind && entry.id == id)
            {
                return Ok((entry.clone(), index.url));
            }
        }
        Err(WorkbenchShellError::NotFound(format!(
            "no index lists a {} named {id}",
            kind.as_str()
        )))
    }

    /// The exact plan installing one store entry would apply.
    ///
    /// # Errors
    ///
    /// Refuses an entry no index lists, a kind the store does not install,
    /// and a distribution this machine cannot run.
    pub fn store_plan(&self, body: &StorePlanBody) -> Result<InstallPlan, WorkbenchShellError> {
        match body.kind {
            InstallKind::Agent => self.agent_install_plan(&body.id),
            InstallKind::Server | InstallKind::Skill => {
                let (entry, index_url) = self.catalog_entry(body.kind, &body.id)?;
                plan_of(&entry, &index_url)
            }
            InstallKind::Tool => Err(WorkbenchShellError::Invalid(
                "a tool is installed from the package that declares it".into(),
            )),
        }
    }

    /// Install one store entry against the plan the person saw, and put it
    /// where its kind is used from.
    ///
    /// # Errors
    ///
    /// Refuses a moved plan and any install failure.
    pub fn store_install(
        &self,
        body: &StoreInstallBody,
    ) -> Result<InstallReceipt, WorkbenchShellError> {
        if body.kind == InstallKind::Agent {
            return self.install_agent(&body.id, &body.plan_id);
        }
        let plan = self.store_plan(&StorePlanBody {
            kind: body.kind,
            id: body.id.clone(),
        })?;
        if plan.plan_id != body.plan_id {
            return Err(WorkbenchShellError::Conflict(format!(
                "the install plan changed since it was shown ({} now); read it again",
                plan.plan_id
            )));
        }
        let node = if matches!(plan.distribution, RegistryDistribution::Npx { .. }) {
            Some(which_node().ok_or_else(|| {
                WorkbenchShellError::Invalid(
                    "this distribution runs through npx and node is not on PATH".into(),
                )
            })?)
        } else {
            None
        };
        let receipt = crate::install(&plan, true, self.installed_root()?, node.as_deref())
            .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?;
        if body.kind == InstallKind::Server {
            // Declared with what launches it and no values: the names a
            // server needs are shown beside it, and a person gives them where
            // every declared server's are given.
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
            self.declare_mcp_server(&DeclareMcpServerBody {
                name: receipt.registry_id.clone(),
                transport: "stdio".into(),
                command,
                args,
                env: Vec::new(),
                url: String::new(),
                headers: Vec::new(),
            })?;
        }
        Ok(receipt)
    }

    /// Add a catalog by URL: fetched now, kept under the indexes directory
    /// with where it came from.
    ///
    /// # Errors
    ///
    /// Refuses a URL that does not answer with a catalog.
    pub fn add_index(&self, body: &AddIndexBody) -> Result<StoreIndexView, WorkbenchShellError> {
        let home = self.store_home()?;
        let url = body.url.trim();
        if url.is_empty() {
            return Err(WorkbenchShellError::Invalid("an index has a URL".into()));
        }
        let bytes =
            fetch_url(url).map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?;
        let catalog = parse_catalog(&bytes)?;
        let slug = slug_of(&catalog.name)?;
        let index = IndexFile {
            schema: INDEX_SCHEMA.into(),
            url: url.to_owned(),
            fetched_at: now_seconds(),
            catalog,
        };
        let path = home.indexes.join(format!("{slug}.json"));
        let temporary = home
            .indexes
            .join(format!(".{slug}.{}.tmp", std::process::id()));
        std::fs::write(
            &temporary,
            serde_json::to_vec_pretty(&index)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?,
        )
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            WorkbenchShellError::Failed(error.to_string())
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
    /// Refuses an unknown index.
    pub fn forget_index(&self, slug: &str) -> Result<(), WorkbenchShellError> {
        valid_slug(slug)?;
        let home = self.store_home()?;
        if home
            .builtin
            .iter()
            .any(|catalog| slug_of(&catalog.name).ok().as_deref() == Some(slug))
        {
            return Err(WorkbenchShellError::Invalid(format!(
                "{slug} came with this product and is not forgotten"
            )));
        }
        let path = home.indexes.join(format!("{slug}.json"));
        if !path.is_file() {
            return Err(WorkbenchShellError::NotFound(format!(
                "no index named {slug}"
            )));
        }
        std::fs::remove_file(&path).map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Every installed skill, read from its `SKILL.md`.
    ///
    /// # Errors
    ///
    /// Fails when this host installs nothing.
    pub fn installed_skills(&self) -> Result<Vec<InstalledSkill>, WorkbenchShellError> {
        let mut skills = Vec::new();
        for (id, receipt) in load_receipts(self.installed_root()?, InstallKind::Skill) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalog_is_read_and_a_wrong_one_refused() {
        let good = br#"{"schema":"swem:catalog@0.1","name":"My Tools","entries":[
            {"kind":"server","id":"echo","name":"Echo","version":"1","distribution":{"npx":{"package":"echo@1"}}},
            {"kind":"skill","id":"shout","name":"Shout","version":"1","distribution":{"archive":{"url":"https://x/s.tar.gz","sha256":"ab"}}}
        ]}"#;
        let catalog = parse_catalog(good).unwrap();
        assert_eq!(catalog.entries.len(), 2);
        assert_eq!(slug_of(&catalog.name).unwrap(), "my-tools");
        let bad = br#"{"schema":"swem:catalog@0.1","name":"x","entries":[{"kind":"agent","id":"a","name":"a","version":"1","distribution":{}}]}"#;
        assert!(parse_catalog(bad).is_err());
        let wrong_schema = br#"{"schema":"other","name":"x","entries":[]}"#;
        assert!(parse_catalog(wrong_schema).is_err());
        let bundled = br#"{"schema":"swem:catalog@0.1","name":"SWEM","entries":[{"kind":"server","id":"swem-cycle","name":"Cycle","version":"1","bundled":true}]}"#;
        let shipped = parse_catalog(bundled).unwrap();
        assert!(shipped.entries[0].bundled);
        assert!(distribution_of(&shipped.entries[0]).is_err());
        let not_bundled = br#"{"schema":"swem:catalog@0.1","name":"SWEM","entries":[{"kind":"server","id":"x","name":"x","version":"1"}]}"#;
        assert!(parse_catalog(not_bundled).is_err());
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
        let (name, description, body) = read_skill("plain", "just words\n");
        assert_eq!(
            (name.as_str(), description.as_str(), body.as_str()),
            ("plain", "", "just words")
        );
    }
}
