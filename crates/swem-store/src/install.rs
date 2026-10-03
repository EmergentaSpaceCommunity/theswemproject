//! What a person installs into the product, and the receipt it leaves.
//!
//! One installer for everything the Workbench fetches onto this machine: an
//! agent from the ACP registry, a tool a package declares, and (with the
//! store) an MCP server or a skill from a catalog. Every
//! kind goes the same road - a plan the person reads and consents to by its
//! exact id, a staged fetch checked against the digest the plan named, one
//! rename into place, one receipt - and lands under one root,
//! `<data root>/installed/<kind>/<id>/<version>/`, so there is one place to
//! look for what this product has and one receipt schema to read it by.
//!
//! Supported distributions are script-disabled npm packages and
//! digest-verified native archives; neither path executes a remote manifest
//! as shell code.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    INSTALL_RECEIPT_SCHEMA, INSTALLATION_MANIFEST, InstallPlan, InstallReceipt, Kind,
    RegistryDistribution, SKILL_FILE, StoreError as SupplyError,
};

/// Which of two receipts of one thing is newer: by version where both
/// versions are versions, else by when each was installed, else by the
/// version as text.
#[must_use]
pub fn newer(left: &InstallReceipt, right: &InstallReceipt) -> std::cmp::Ordering {
    match (
        semver::Version::parse(&left.version),
        semver::Version::parse(&right.version),
    ) {
        (Ok(left), Ok(right)) => left.cmp(&right),
        _ if left.installed_at != right.installed_at => left.installed_at.cmp(&right.installed_at),
        _ => left.version.cmp(&right.version),
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Every receipt of one kind under the install root, by registry id: the
/// newest version of each, and only one whose launch path is a file.
#[must_use]
pub fn load_receipts(
    installed_root: &Path,
    kind: &Kind,
) -> std::collections::BTreeMap<String, InstallReceipt> {
    let mut found = std::collections::BTreeMap::new();
    let Ok(entries) = fs::read_dir(installed_root.join(kind.directory())) else {
        return found;
    };
    for entry_dir in entries.flatten() {
        let Ok(versions) = fs::read_dir(entry_dir.path()) else {
            continue;
        };
        let mut receipts = versions
            .flatten()
            .map(|version| version.path().join(INSTALLATION_MANIFEST))
            .filter(|path| path.is_file())
            .filter_map(|path| {
                let bytes = fs::read(&path).ok()?;
                serde_json::from_slice::<InstallReceipt>(&bytes).ok()
            })
            .filter(|receipt| receipt.launch_path().is_some())
            .collect::<Vec<_>>();
        // The newest: by version where versions are versions, else by
        // when it was installed. `0.10.0` is newer than `0.9.0`.
        receipts.sort_by(newer);
        if let Some(receipt) = receipts.pop() {
            found.insert(receipt.registry_id.clone(), receipt);
        }
    }
    found
}

/// Every receipt of every kind under the install root: the built-in kinds
/// in their order, then whatever other kinds have a directory there.
#[must_use]
pub fn all_receipts(installed_root: &Path) -> Vec<InstallReceipt> {
    Kind::at(installed_root)
        .iter()
        .flat_map(|kind| load_receipts(installed_root, kind).into_values())
        .collect()
}

/// Fetch one URL's bytes, as the registry and the catalogs are fetched:
/// `curl` with a minute's patience, `file://` included.
///
/// # Errors
///
/// The fetch's own failure, in curl's words.
pub fn fetch_url(url: &str) -> Result<Vec<u8>, SupplyError> {
    let output = Command::new("curl")
        .args(["-fsSL", "--max-time", "60", url])
        .output()
        .map_err(|error| SupplyError::Protocol(format!("curl: {error}")))?;
    if !output.status.success() {
        return Err(SupplyError::Protocol(format!(
            "fetch of {url} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

/// Bring the agents an earlier product installed into its own directory
/// (`<data home>/swem/agents`, outside the data root) under the install
/// root, rewriting the paths their receipts point at. Each is moved once;
/// one already under the root is left alone. Answers how many moved.
///
/// # Errors
///
/// A move or a rewrite that fails names the directory; what was moved
/// before it stays moved, since each is complete on its own.
pub fn adopt_legacy_agents(
    legacy_home: &Path,
    installed_root: &Path,
) -> Result<usize, SupplyError> {
    let Ok(entries) = fs::read_dir(legacy_home) else {
        return Ok(0);
    };
    let agents = installed_root.join(Kind::AGENT.directory());
    let mut moved = 0;
    for agent_dir in entries.flatten() {
        let from = agent_dir.path();
        if !from.is_dir() {
            continue;
        }
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = agents.join(name);
        if to.exists() {
            continue;
        }
        fs::create_dir_all(&agents).map_err(|error| {
            SupplyError::Protocol(format!("create {}: {error}", agents.display()))
        })?;
        fs::rename(&from, &to).map_err(|error| {
            SupplyError::Protocol(format!(
                "move {} to {}: {error}",
                from.display(),
                to.display()
            ))
        })?;
        for version in fs::read_dir(&to).into_iter().flatten().flatten() {
            let manifest = version.path().join(INSTALLATION_MANIFEST);
            let Ok(bytes) = fs::read(&manifest) else {
                continue;
            };
            let Ok(mut receipt) = serde_json::from_slice::<InstallReceipt>(&bytes) else {
                continue;
            };
            let rewrite = |path: &PathBuf| -> PathBuf {
                path.strip_prefix(&from)
                    .map_or_else(|_| path.clone(), |rest| to.join(rest))
            };
            receipt.entry_script = receipt.entry_script.as_ref().map(rewrite);
            receipt.executable = receipt.executable.as_ref().map(rewrite);
            INSTALL_RECEIPT_SCHEMA.clone_into(&mut receipt.schema);
            receipt.kind = Kind::AGENT;
            fs::write(
                &manifest,
                serde_json::to_vec_pretty(&receipt)
                    .map_err(|error| SupplyError::Serialization(error.to_string()))?,
            )
            .map_err(|error| {
                SupplyError::Protocol(format!("rewrite {}: {error}", manifest.display()))
            })?;
        }
        moved += 1;
    }
    // The old directory served its purpose; an empty one left behind would
    // read as a second place to look.
    let _ = fs::remove_dir(legacy_home);
    Ok(moved)
}

#[must_use]
pub fn registry_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("windows", "aarch64") => Some("windows-aarch64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        ("macos", "aarch64") => Some("darwin-aarch64"),
        _ => None,
    }
}

/// Fetch the official registry index and resolve one exact platform plan.
///
/// # Errors
///
/// Returns [`SupplyError`] if the index cannot be fetched or parsed, the agent
/// is absent, or its current platform distribution is unsupported or invalid.
pub fn resolve_registry_install_plan(
    agent_id: &str,
    registry_id: &str,
    registry_index: &str,
) -> Result<InstallPlan, SupplyError> {
    let bytes = fetch_url(registry_index)
        .map_err(|error| SupplyError::Protocol(format!("registry fetch failed: {error}")))?;
    let platform = registry_platform().ok_or_else(|| {
        SupplyError::UnsupportedDistribution(format!(
            "{registry_id} on {}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    })?;
    resolve_registry_install_plan_from_bytes(
        agent_id,
        registry_id,
        registry_index,
        &bytes,
        platform,
    )
}

/// Resolve one exact platform plan from already fetched registry bytes.
///
/// # Errors
///
/// Returns [`SupplyError`] for malformed registry data, an absent entry, or an
/// unsupported/invalid distribution. This function performs no I/O.
pub fn resolve_registry_install_plan_from_bytes(
    agent_id: &str,
    registry_id: &str,
    registry_index: &str,
    bytes: &[u8],
    platform: &str,
) -> Result<InstallPlan, SupplyError> {
    let index: Value = serde_json::from_slice(bytes)
        .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    let entry = index["agents"]
        .as_array()
        .and_then(|agents| agents.iter().find(|agent| agent["id"] == registry_id))
        .ok_or_else(|| SupplyError::UnknownAgent(registry_id.into()))?;
    let name = entry["name"].as_str().unwrap_or(registry_id).to_owned();
    let version = required_string(entry, "version")?;
    let distribution = if let Some(binary) = entry["distribution"]["binary"].get(platform) {
        let archive = required_string(binary, "archive")?;
        let sha256 = required_string(binary, "sha256")?;
        if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(SupplyError::Protocol(
                "registry sha256 is not 64 hex characters".into(),
            ));
        }
        RegistryDistribution::Binary {
            platform: platform.into(),
            archive,
            sha256: sha256.to_ascii_lowercase(),
            command: required_string(binary, "cmd")?,
            args: string_array(binary.get("args"))?,
        }
    } else if let Some(npx) = entry["distribution"].get("npx") {
        RegistryDistribution::Npx {
            package: required_string(npx, "package")?,
            args: string_array(npx.get("args"))?,
        }
    } else {
        return Err(SupplyError::UnsupportedDistribution(format!(
            "{registry_id} on {platform}"
        )));
    };
    let plan_bytes = serde_json::to_vec(&(
        agent_id,
        registry_id,
        registry_index,
        &name,
        &version,
        &distribution,
        true,
        false,
    ))
    .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    Ok(InstallPlan {
        plan_id: format!("sha256:{:x}", Sha256::digest(plan_bytes)),
        kind: Kind::AGENT,
        agent_id: agent_id.into(),
        registry_id: registry_id.into(),
        registry_index: registry_index.into(),
        name,
        version,
        distribution,
        requires_explicit_consent: true,
        executes_remote_shell: false,
        requires: Vec::new(),
    })
}

fn required_string(value: &Value, field: &str) -> Result<String, SupplyError> {
    value[field]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| SupplyError::Protocol(format!("registry entry lacks {field}")))
}

fn string_array(value: Option<&Value>) -> Result<Vec<String>, SupplyError> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value.clone())
        .map_err(|error| SupplyError::Serialization(error.to_string()))
}

fn npm_cli(node: &Path) -> Result<PathBuf, SupplyError> {
    let node_dir = node
        .parent()
        .ok_or_else(|| SupplyError::MissingExecutable("node".into()))?;
    [
        node_dir.join("node_modules/npm/bin/npm-cli.js"),
        node_dir.join("../lib/node_modules/npm/bin/npm-cli.js"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .ok_or_else(|| SupplyError::MissingExecutable("npm-cli.js".into()))
}

/// Apply a previously resolved plan without resolving the registry again.
/// What it installs lands under `<installed_root>/<kind>/<id>/<version>/`.
///
/// # Errors
///
/// Returns [`SupplyError`] when consent is absent, a dependency/download is
/// unavailable, integrity or archive checks fail, or the target is inconsistent.
pub fn install(
    plan: &InstallPlan,
    consent: bool,
    installed_root: &Path,
    node: Option<&Path>,
) -> Result<InstallReceipt, SupplyError> {
    install_checked(plan, consent, installed_root, node, &|_| Ok(()))
}

/// As [`install`], with `check` asked about what was staged before it is
/// moved into place: whoever takes this kind looks at it, and a refusal
/// leaves nothing behind.
///
/// # Errors
///
/// As [`install`], and the check's own refusal.
pub fn install_checked(
    plan: &InstallPlan,
    consent: bool,
    installed_root: &Path,
    node: Option<&Path>,
    check: &dyn Fn(&Path) -> Result<(), SupplyError>,
) -> Result<InstallReceipt, SupplyError> {
    if !consent {
        return Err(SupplyError::ConsentRequired(plan.agent_id.clone()));
    }
    let target = installed_root
        .join(plan.kind.directory())
        .join(&plan.registry_id)
        .join(&plan.version);
    let manifest_path = target.join(INSTALLATION_MANIFEST);
    if manifest_path.is_file() {
        let installation: InstallReceipt = serde_json::from_slice(
            &fs::read(&manifest_path)
                .map_err(|error| SupplyError::Protocol(format!("read manifest: {error}")))?,
        )
        .map_err(|error| SupplyError::Serialization(error.to_string()))?;
        if installation.matches_plan(plan) && installation.launch_path().is_some() {
            return Ok(installation);
        }
        return Err(SupplyError::Protocol(format!(
            "installation target exists with a non-matching manifest: {}",
            target.display()
        )));
    }
    if target.exists() {
        return Err(SupplyError::Protocol(format!(
            "installation target already exists: {}",
            target.display()
        )));
    }
    let parent = target
        .parent()
        .ok_or_else(|| SupplyError::Protocol("installation target has no parent".into()))?;
    fs::create_dir_all(parent)
        .map_err(|error| SupplyError::Protocol(format!("create {}: {error}", parent.display())))?;
    let staging = parent.join(format!(".{}.staging-{}", plan.version, std::process::id()));
    if staging.exists() {
        return Err(SupplyError::Protocol(format!(
            "staging target already exists: {}",
            staging.display()
        )));
    }
    fs::create_dir(&staging)
        .map_err(|error| SupplyError::Protocol(format!("create {}: {error}", staging.display())))?;
    let result = install_into_staging(plan, &staging, node).and_then(|mut installation| {
        check(&staging)?;
        if let Some(path) = &installation.entry_script {
            installation.entry_script =
                Some(target.join(path.strip_prefix(&staging).map_err(|error| {
                    SupplyError::Protocol(format!("staged entry path: {error}"))
                })?));
        }
        if let Some(path) = &installation.executable {
            installation.executable =
                Some(target.join(path.strip_prefix(&staging).map_err(|error| {
                    SupplyError::Protocol(format!("staged executable path: {error}"))
                })?));
        }
        if let Some(path) = &installation.file {
            installation.file =
                Some(target.join(path.strip_prefix(&staging).map_err(|error| {
                    SupplyError::Protocol(format!("staged file path: {error}"))
                })?));
        }
        fs::write(
            staging.join(INSTALLATION_MANIFEST),
            serde_json::to_vec_pretty(&installation)
                .map_err(|error| SupplyError::Serialization(error.to_string()))?,
        )
        .map_err(|error| SupplyError::Protocol(format!("write manifest: {error}")))?;
        fs::rename(&staging, &target).map_err(|error| {
            SupplyError::Protocol(format!(
                "commit installation {} -> {}: {error}",
                staging.display(),
                target.display()
            ))
        })?;
        Ok(installation)
    });
    if result.is_err() && staging.exists() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn install_into_staging(
    plan: &InstallPlan,
    staging: &Path,
    node: Option<&Path>,
) -> Result<InstallReceipt, SupplyError> {
    match &plan.distribution {
        RegistryDistribution::Npx { package, args } => {
            install_npx(plan, staging, node, package, args)
        }
        RegistryDistribution::Binary {
            archive,
            sha256,
            command,
            args,
            ..
        } => install_binary(plan, staging, archive, sha256, command, args),
        RegistryDistribution::Archive { url, sha256 } => {
            install_archive(plan, staging, url, sha256)
        }
        RegistryDistribution::Uvx { package, args } => install_uvx(plan, staging, package, args),
    }
}

/// Fetch one archive into `into`, checked against the digest the plan named.
fn fetch_archive(archive_url: &str, expected_sha256: &str, into: &Path) -> Result<(), SupplyError> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "300",
            "--output",
            &into.display().to_string(),
            archive_url,
        ])
        .output()
        .map_err(|error| SupplyError::Protocol(format!("curl: {error}")))?;
    if !output.status.success() {
        return Err(SupplyError::Protocol(format!(
            "archive download failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let actual_sha256 = sha256_file(into)?;
    if actual_sha256 != expected_sha256 {
        return Err(SupplyError::Protocol(format!(
            "archive sha256 mismatch: expected {expected_sha256}, got {actual_sha256}"
        )));
    }
    Ok(())
}

/// How a distribution's bytes are packed: a zip, a gzipped tar, or not at
/// all - one bare executable, as some tools are published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Packed {
    Zip,
    TarGz,
    Bare,
}

/// What the archive's address ends like.
fn archive_format(archive_url: &str) -> Packed {
    let url_path = Path::new(archive_url);
    let extension = url_path.extension().and_then(|value| value.to_str());
    let is_zip = extension.is_some_and(|value| value.eq_ignore_ascii_case("zip"));
    let is_tgz = extension.is_some_and(|value| value.eq_ignore_ascii_case("tgz"));
    let is_tar_gz = extension.is_some_and(|value| value.eq_ignore_ascii_case("gz"))
        && url_path
            .file_stem()
            .and_then(|value| Path::new(value).extension())
            .and_then(|value| value.to_str())
            .is_some_and(|value| value.eq_ignore_ascii_case("tar"));
    if is_zip {
        Packed::Zip
    } else if is_tar_gz || is_tgz {
        Packed::TarGz
    } else {
        Packed::Bare
    }
}
/// A whole tree from an archive: what a skill is. The tree is unpacked under
/// `tree/` in staging and the receipt's `file` is its `SKILL.md`, at the top
/// or inside the one directory the archive holds.
fn install_archive(
    plan: &InstallPlan,
    staging: &Path,
    archive_url: &str,
    expected_sha256: &str,
) -> Result<InstallReceipt, SupplyError> {
    let archive_path = staging.join("distribution.archive");
    fetch_archive(archive_url, expected_sha256, &archive_path)?;
    let tree = staging.join("tree");
    fs::create_dir_all(&tree)
        .map_err(|error| SupplyError::Protocol(format!("create tree directory: {error}")))?;
    match archive_format(archive_url) {
        Packed::Zip => extract_zip_tree(&archive_path, &tree)?,
        Packed::TarGz => extract_tar_gz_tree(&archive_path, &tree)?,
        Packed::Bare => {
            return Err(SupplyError::UnsupportedDistribution(format!(
                "a tree comes as a zip or a tar.gz, not as one file: {archive_url}"
            )));
        }
    }
    fs::remove_file(&archive_path)
        .map_err(|error| SupplyError::Protocol(format!("remove staged archive: {error}")))?;
    // A skill is its file; a tree of another kind is the tree, and whoever
    // takes it reads it as they read it.
    let file = if plan.kind == Kind::SKILL {
        skill_file_in(&tree)?
    } else {
        tree.clone()
    };
    Ok(InstallReceipt {
        schema: INSTALL_RECEIPT_SCHEMA.to_owned(),
        kind: plan.kind.clone(),
        plan_id: plan.plan_id.clone(),
        source: plan.registry_index.clone(),
        registry_id: plan.registry_id.clone(),
        name: plan.name.clone(),
        version: plan.version.clone(),
        platform: None,
        package: None,
        archive: Some(archive_url.into()),
        sha256: Some(expected_sha256.into()),
        command: None,
        entry_script: None,
        executable: None,
        file: Some(file),
        args: Vec::new(),
        discovery_method: "archive-sha256-tree".into(),
        installed_at: now_seconds(),
    })
}

/// The `SKILL.md` of an unpacked skill: at the top of the tree, or one
/// directory down when the archive was made of a folder.
fn skill_file_in(tree: &Path) -> Result<PathBuf, SupplyError> {
    let direct = tree.join(SKILL_FILE);
    if direct.is_file() {
        return Ok(direct);
    }
    let mut directories = fs::read_dir(tree)
        .map_err(|error| SupplyError::Protocol(format!("read tree: {error}")))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir());
    if let (Some(only), None) = (directories.next(), directories.next()) {
        let nested = only.join(SKILL_FILE);
        if nested.is_file() {
            return Ok(nested);
        }
    }
    Err(SupplyError::Protocol(format!(
        "the archive holds no {SKILL_FILE} at its top or in its one folder"
    )))
}

fn tree_target(into: &Path, name: &Path) -> Result<PathBuf, SupplyError> {
    if name
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SupplyError::Protocol(format!(
            "archive entry is not a safe relative path: {}",
            name.display()
        )));
    }
    Ok(into.join(name))
}

fn extract_tar_gz_tree(archive_path: &Path, into: &Path) -> Result<(), SupplyError> {
    let file = File::open(archive_path)
        .map_err(|error| SupplyError::Protocol(format!("open tar.gz: {error}")))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    for entry in archive
        .entries()
        .map_err(|error| SupplyError::Protocol(format!("read tar.gz: {error}")))?
    {
        let mut entry =
            entry.map_err(|error| SupplyError::Protocol(format!("read tar entry: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| SupplyError::Protocol(format!("entry path: {error}")))?
            .into_owned();
        let target = tree_target(into, &path)?;
        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| SupplyError::Protocol(format!("create directory: {error}")))?;
            continue;
        }
        if !entry.header().entry_type().is_file() {
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| SupplyError::Protocol(format!("create directory: {error}")))?;
        }
        let mut out = File::create(&target)
            .map_err(|error| SupplyError::Protocol(format!("write entry: {error}")))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|error| SupplyError::Protocol(format!("write entry: {error}")))?;
    }
    Ok(())
}

fn extract_zip_tree(archive_path: &Path, into: &Path) -> Result<(), SupplyError> {
    let file = File::open(archive_path)
        .map_err(|error| SupplyError::Protocol(format!("open zip: {error}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| SupplyError::Protocol(format!("parse zip: {error}")))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| SupplyError::Protocol(format!("read zip entry: {error}")))?;
        let Some(name) = entry.enclosed_name() else {
            return Err(SupplyError::Protocol(format!(
                "zip contains unsafe path: {}",
                entry.name()
            )));
        };
        let target = tree_target(into, &name)?;
        if entry.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| SupplyError::Protocol(format!("create directory: {error}")))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| SupplyError::Protocol(format!("create directory: {error}")))?;
        }
        let mut out = File::create(&target)
            .map_err(|error| SupplyError::Protocol(format!("write entry: {error}")))?;
        std::io::copy(&mut entry, &mut out)
            .map_err(|error| SupplyError::Protocol(format!("write entry: {error}")))?;
    }
    Ok(())
}

fn install_npx(
    plan: &InstallPlan,
    staging: &Path,
    node: Option<&Path>,
    package: &str,
    args: &[String],
) -> Result<InstallReceipt, SupplyError> {
    let node = node.ok_or_else(|| SupplyError::MissingExecutable("node".into()))?;
    let npm = npm_cli(node)?;
    let status = Command::new(node)
        .arg(&npm)
        .args([
            "install",
            "--prefix",
            &staging.display().to_string(),
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
            "--loglevel",
            "error",
            package,
        ])
        .status()
        .map_err(|error| SupplyError::Protocol(format!("npm: {error}")))?;
    if !status.success() {
        return Err(SupplyError::Protocol(format!(
            "npm install failed: {status}"
        )));
    }
    let package_name = package
        .rsplit_once('@')
        .filter(|(name, _)| !name.is_empty())
        .map_or(package, |(name, _)| name);
    let package_dir = staging.join("node_modules").join(package_name);
    let manifest: Value = serde_json::from_slice(
        &fs::read(package_dir.join("package.json"))
            .map_err(|error| SupplyError::Protocol(format!("package.json: {error}")))?,
    )
    .map_err(|error| SupplyError::Serialization(error.to_string()))?;
    let bin = match &manifest["bin"] {
        Value::String(path) => path.clone(),
        Value::Object(map) => map
            .values()
            .next()
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| SupplyError::Protocol("package declares no bin".into()))?,
        _ => return Err(SupplyError::Protocol("package declares no bin".into())),
    };
    let entry_script = package_dir.join(bin);
    if !entry_script.is_file() {
        return Err(SupplyError::Protocol(format!(
            "entry script missing: {}",
            entry_script.display()
        )));
    }
    Ok(InstallReceipt {
        schema: INSTALL_RECEIPT_SCHEMA.to_owned(),
        kind: plan.kind.clone(),
        plan_id: plan.plan_id.clone(),
        source: plan.registry_index.clone(),
        registry_id: plan.registry_id.clone(),
        name: plan.name.clone(),
        version: plan.version.clone(),
        platform: None,
        package: Some(package.into()),
        archive: None,
        sha256: None,
        command: None,
        entry_script: Some(entry_script),
        executable: None,
        file: None,
        args: args.to_vec(),
        discovery_method: "registry-npx-npm-install-ignore-scripts".into(),
        installed_at: now_seconds(),
    })
}

/// Install a Python package into an environment of its own under the
/// staging directory, with `uv`, and name the program it put on its bin
/// path. The package is pinned as `name==version` by the catalog.
fn install_uvx(
    plan: &InstallPlan,
    staging: &Path,
    package: &str,
    args: &[String],
) -> Result<InstallReceipt, SupplyError> {
    if package.trim().is_empty() || package.starts_with('-') {
        return Err(SupplyError::UnsupportedDistribution(format!(
            "not a Python package: {package:?}"
        )));
    }
    let tools = staging.join("tools");
    let bin = staging.join("bin");
    let output = Command::new("uv")
        .args(["tool", "install", "--quiet", package])
        .env("UV_TOOL_DIR", &tools)
        .env("UV_TOOL_BIN_DIR", &bin)
        .output()
        .map_err(|_| SupplyError::MissingExecutable("uv".into()))?;
    if !output.status.success() {
        return Err(SupplyError::Protocol(format!(
            "uv tool install {package}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let name = package
        .split(['=', '@', '[', '>', '<', '~'])
        .next()
        .unwrap_or(package)
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    let mut programs = fs::read_dir(&bin)
        .map_err(|error| SupplyError::Protocol(format!("read {}: {error}", bin.display())))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    programs.sort();
    let executable = programs
        .iter()
        .find(|path| {
            path.file_name()
                .and_then(|file| file.to_str())
                .is_some_and(|file| file.to_ascii_lowercase().replace('_', "-") == name)
        })
        .or_else(|| (programs.len() == 1).then(|| &programs[0]))
        .cloned()
        .ok_or_else(|| {
            SupplyError::Protocol(format!(
                "uv put {} programs on the path of {package} and none is called {name}",
                programs.len()
            ))
        })?;
    Ok(InstallReceipt {
        schema: INSTALL_RECEIPT_SCHEMA.to_owned(),
        kind: plan.kind.clone(),
        plan_id: plan.plan_id.clone(),
        source: plan.registry_index.clone(),
        registry_id: plan.registry_id.clone(),
        name: plan.name.clone(),
        version: plan.version.clone(),
        platform: None,
        package: Some(package.into()),
        archive: None,
        sha256: None,
        command: None,
        entry_script: None,
        executable: Some(executable),
        file: None,
        args: args.to_vec(),
        discovery_method: "uv-tool-install-pinned".into(),
        installed_at: now_seconds(),
    })
}

fn install_binary(
    plan: &InstallPlan,
    staging: &Path,
    archive_url: &str,
    expected_sha256: &str,
    command: &str,
    args: &[String],
) -> Result<InstallReceipt, SupplyError> {
    let archive_path = staging.join("distribution.archive");
    fetch_archive(archive_url, expected_sha256, &archive_path)?;
    let relative_command = safe_relative_command(command)?;
    let executable = staging.join(&relative_command);
    if let Some(parent) = executable.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| SupplyError::Protocol(format!("create command directory: {error}")))?;
    }
    let packed = archive_format(archive_url);
    match packed {
        Packed::Zip => extract_zip_entry(&archive_path, &relative_command, &executable)?,
        Packed::TarGz => extract_tar_gz_entry(&archive_path, &relative_command, &executable)?,
        // One bare executable, checked by its digest above: it lands as the
        // command it is named.
        Packed::Bare => fs::rename(&archive_path, &executable)
            .map_err(|error| SupplyError::Protocol(format!("place the executable: {error}")))?,
    }
    if packed != Packed::Bare {
        fs::remove_file(&archive_path)
            .map_err(|error| SupplyError::Protocol(format!("remove staged archive: {error}")))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .map_err(|error| SupplyError::Protocol(format!("set executable mode: {error}")))?;
    }
    Ok(InstallReceipt {
        schema: INSTALL_RECEIPT_SCHEMA.to_owned(),
        kind: plan.kind.clone(),
        plan_id: plan.plan_id.clone(),
        source: plan.registry_index.clone(),
        registry_id: plan.registry_id.clone(),
        name: plan.name.clone(),
        version: plan.version.clone(),
        platform: plan.distribution.platform().map(str::to_owned),
        package: None,
        archive: Some(archive_url.into()),
        sha256: Some(expected_sha256.into()),
        command: Some(command.into()),
        entry_script: None,
        executable: Some(executable),
        file: None,
        args: args.to_vec(),
        discovery_method: if packed == Packed::Bare {
            "registry-binary-sha256-bare".into()
        } else {
            "registry-binary-sha256-exact-entry".into()
        },
        installed_at: now_seconds(),
    })
}

fn sha256_file(path: &Path) -> Result<String, SupplyError> {
    let mut file = File::open(path)
        .map_err(|error| SupplyError::Protocol(format!("open archive: {error}")))?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| SupplyError::Protocol(format!("read archive: {error}")))?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn safe_relative_command(command: &str) -> Result<PathBuf, SupplyError> {
    let path = Path::new(command.strip_prefix("./").unwrap_or(command));
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(SupplyError::Protocol(format!(
            "registry command is not a safe relative path: {command}"
        )));
    }
    Ok(path.to_owned())
}

fn archive_name(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn extract_zip_entry(
    archive_path: &Path,
    relative_command: &Path,
    executable: &Path,
) -> Result<(), SupplyError> {
    let file = File::open(archive_path)
        .map_err(|error| SupplyError::Protocol(format!("open zip: {error}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| SupplyError::Protocol(format!("parse zip: {error}")))?;
    let expected = archive_name(relative_command);
    let mut match_index = None;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| SupplyError::Protocol(format!("read zip entry: {error}")))?;
        let enclosed = entry.enclosed_name().ok_or_else(|| {
            SupplyError::Protocol(format!("zip contains unsafe path: {}", entry.name()))
        })?;
        if archive_name(&enclosed) == expected
            && (entry.is_dir()
                || entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170_000 == 0o120_000)
                || match_index.replace(index).is_some())
        {
            return Err(SupplyError::Protocol(
                "zip command entry is ambiguous or not a regular file".into(),
            ));
        }
    }
    let index = match_index.ok_or_else(|| {
        SupplyError::Protocol(format!("declared command is absent from zip: {expected}"))
    })?;
    let mut entry = archive
        .by_index(index)
        .map_err(|error| SupplyError::Protocol(format!("read zip command: {error}")))?;
    let mut output = File::create(executable)
        .map_err(|error| SupplyError::Protocol(format!("create executable: {error}")))?;
    std::io::copy(&mut entry, &mut output)
        .map_err(|error| SupplyError::Protocol(format!("extract executable: {error}")))?;
    output
        .flush()
        .map_err(|error| SupplyError::Protocol(format!("flush executable: {error}")))
}

fn extract_tar_gz_entry(
    archive_path: &Path,
    relative_command: &Path,
    executable: &Path,
) -> Result<(), SupplyError> {
    let file = File::open(archive_path)
        .map_err(|error| SupplyError::Protocol(format!("open tar.gz: {error}")))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let expected = archive_name(relative_command);
    let mut found = false;
    for entry in archive
        .entries()
        .map_err(|error| SupplyError::Protocol(format!("read tar.gz: {error}")))?
    {
        let mut entry =
            entry.map_err(|error| SupplyError::Protocol(format!("read tar entry: {error}")))?;
        let path = entry
            .path()
            .map_err(|error| SupplyError::Protocol(format!("tar path: {error}")))?;
        if archive_name(&path) != expected {
            continue;
        }
        if found || !entry.header().entry_type().is_file() {
            return Err(SupplyError::Protocol(
                "tar command entry is ambiguous or not a regular file".into(),
            ));
        }
        let mut output = File::create(executable)
            .map_err(|error| SupplyError::Protocol(format!("create executable: {error}")))?;
        std::io::copy(&mut entry, &mut output)
            .map_err(|error| SupplyError::Protocol(format!("extract executable: {error}")))?;
        output
            .flush()
            .map_err(|error| SupplyError::Protocol(format!("flush executable: {error}")))?;
        found = true;
    }
    if !found {
        return Err(SupplyError::Protocol(format!(
            "declared command is absent from tar.gz: {expected}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_plan_pins_all_execution_inputs() {
        let plan = resolve_registry_install_plan_from_bytes(
            "opencode",
            "opencode",
            "https://registry.invalid/index.json",
            br#"{"agents":[{"id":"opencode","name":"OpenCode","version":"1.18.25","distribution":{"binary":{"windows-x86_64":{"archive":"https://example.invalid/opencode.zip","cmd":"./opencode.exe","args":["acp"],"sha256":"831e213e5f454d6e8b26f0fb24c7b3d42b40e47d73d154672a9192702eb08416"}}}}]}"#,
            "windows-x86_64",
        )
        .unwrap();
        assert_eq!(plan.version, "1.18.25");
        assert_eq!(plan.name, "OpenCode");
        assert!(plan.plan_id.starts_with("sha256:"));
        assert!(matches!(
            plan.distribution,
            RegistryDistribution::Binary {
                platform,
                archive,
                command,
                args,
                ..
            } if platform == "windows-x86_64"
                && archive == "https://example.invalid/opencode.zip"
                && command == "./opencode.exe"
                && args == ["acp"]
        ));
    }

    #[test]
    fn binary_plan_rejects_invalid_digest() {
        let error = resolve_registry_install_plan_from_bytes(
            "agent",
            "agent",
            "https://registry.invalid/index.json",
            br#"{"agents":[{"id":"agent","version":"1","distribution":{"binary":{"linux-x86_64":{"archive":"https://example.invalid/a.tar.gz","cmd":"./agent","sha256":"short"}}}}]}"#,
            "linux-x86_64",
        )
        .unwrap_err();
        assert!(matches!(error, SupplyError::Protocol(_)));
    }

    #[test]
    fn command_path_cannot_escape_installation() {
        assert!(safe_relative_command("../agent").is_err());
        assert!(safe_relative_command("/agent").is_err());
        assert_eq!(
            safe_relative_command("./bin/agent").unwrap(),
            Path::new("bin/agent")
        );
    }
}
