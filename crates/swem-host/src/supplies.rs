//! What a realization needs besides code, kept by the host.
//!
//! A package declares the tools its adapters run - exact archives per
//! platform, digested - and the host installs them from that declaration by
//! a plan the person confirms, into a directory of its own, and hands the
//! Cycle a file naming each installed tool's path. A project's nodes may
//! require secrets of types packages declare; the host keeps their values in
//! a vault file beside the project and hands the Cycle that file. Nothing a
//! package needs is ever something a person installs on their machine by
//! hand, and no value of a secret enters a record: the Cycle reads both
//! files again at every close.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{InstallKind, InstallPlan, RegistryDistribution, SupplyError, install, load_receipts};

/// The file the Cycle reads for tool paths: `{"uv": "<path>"}`.
pub const TOOLS_FILE: &str = "tools.json";
/// The file the Cycle reads for a project's secret values, beside the
/// project's declaration: `{"vault://project/<name>/<type>/<n>": "<value>"}`.
pub const PROJECT_VAULT_FILE: &str = "secrets.json";

/// This machine, in the vocabulary a package's `tools[].artifacts` uses.
#[must_use]
pub fn current_platform() -> String {
    let arch = std::env::consts::ARCH;
    match std::env::consts::OS {
        "windows" => format!("{arch}-pc-windows-msvc"),
        "macos" => format!("{arch}-apple-darwin"),
        "linux" => format!("{arch}-unknown-linux-gnu"),
        other => format!("{arch}-unknown-{other}"),
    }
}

/// One tool a loaded package declares, as the person sees it before
/// consenting: what would be fetched, from where, with which digest, and
/// whether it is already installed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ToolView {
    pub name: String,
    pub module: String,
    pub version: String,
    pub platform: String,
    /// Exact identity of the plan: what the install consents to.
    pub plan_id: String,
    pub url: String,
    pub sha256: String,
    pub entry: String,
    /// The installed executable, when this version is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<PathBuf>,
}

fn plan_identity(
    name: &str,
    version: &str,
    platform: &str,
    url: &str,
    sha256: &str,
    entry: &str,
) -> String {
    let mut digest = Sha256::new();
    for field in [name, version, platform, url, sha256, entry] {
        digest.update(field.as_bytes());
        digest.update(b"\n");
    }
    format!("sha256:{:x}", digest.finalize())
}

/// The same, from declarations gathered from more than one place.
///
/// A process fixes its assembly once, so a package installed into a running
/// host never joins that host's own vocabulary - and its tools have to be
/// read from what is installed instead. The host merges the two and asks for
/// views of the result.
#[must_use]
pub fn tool_views_of(tools: &[crate::SupplyTool], installed_root: &Path) -> Vec<ToolView> {
    let platform = current_platform();
    let installed = load_receipts(installed_root, InstallKind::Tool);
    tools
        .iter()
        .filter_map(|tool| {
            let artifact = tool.artifacts.get(&platform)?;
            let executable = installed
                .get(&tool.name)
                .filter(|installation| installation.version == tool.version)
                .and_then(|installation| installation.executable.clone())
                .filter(|path| path.is_file());
            Some(ToolView {
                name: tool.name.clone(),
                module: tool.module.clone(),
                version: tool.version.clone(),
                platform: platform.clone(),
                plan_id: plan_identity(
                    &tool.name,
                    &tool.version,
                    &platform,
                    &artifact.url,
                    &artifact.sha256,
                    &artifact.entry,
                ),
                url: artifact.url.clone(),
                sha256: artifact.sha256.clone(),
                entry: artifact.entry.clone(),
                executable,
            })
        })
        .collect()
}

/// Install one declared tool against the exact plan the person saw, then
/// rewrite the tools file the Cycle reads. Idempotent: an installation that
/// matches the plan is reused.
///
/// # Errors
///
/// Returns [`SupplyError`] when the plan does not match, the download or
/// digest fails, or the tools file cannot be written.
pub fn install_tool(
    view: &ToolView,
    plan_id: &str,
    installed_root: &Path,
) -> Result<PathBuf, SupplyError> {
    if view.plan_id != plan_id {
        return Err(SupplyError::Protocol(format!(
            "the install plan changed since it was shown ({} now); read it again",
            view.plan_id
        )));
    }
    let plan = InstallPlan {
        plan_id: view.plan_id.clone(),
        kind: InstallKind::Tool,
        agent_id: view.name.clone(),
        registry_id: view.name.clone(),
        registry_index: format!("package:{}", view.module),
        name: view.name.clone(),
        version: view.version.clone(),
        distribution: RegistryDistribution::Binary {
            platform: view.platform.clone(),
            archive: view.url.clone(),
            sha256: view.sha256.clone(),
            command: view.entry.clone(),
            args: Vec::new(),
        },
        requires_explicit_consent: true,
        executes_remote_shell: false,
    };
    let receipt = install(&plan, true, installed_root, None)?;
    let executable = receipt
        .executable
        .ok_or_else(|| SupplyError::Protocol("the tool installed without an executable".into()))?;
    write_tools_file(installed_root)?;
    Ok(executable)
}

/// Every installed tool by name, from the installation manifests.
#[must_use]
pub fn installed_tools(installed_root: &Path) -> BTreeMap<String, PathBuf> {
    load_receipts(installed_root, InstallKind::Tool)
        .into_iter()
        .filter_map(|(name, installation)| {
            installation
                .executable
                .filter(|path| path.is_file())
                .map(|path| (name, path))
        })
        .collect()
}

/// The file the Cycle reads tool paths from: beside the installed tools,
/// under the install root.
#[must_use]
pub fn tools_file(installed_root: &Path) -> PathBuf {
    installed_root
        .join(InstallKind::Tool.directory())
        .join(TOOLS_FILE)
}

/// Rewrite the file the Cycle reads for tool paths from what is installed.
///
/// # Errors
///
/// Returns [`SupplyError`] when the file cannot be written.
pub fn write_tools_file(installed_root: &Path) -> Result<PathBuf, SupplyError> {
    let path = tools_file(installed_root);
    let tools_home = path.parent().unwrap_or(installed_root);
    std::fs::create_dir_all(tools_home).map_err(|error| {
        SupplyError::Protocol(format!("create {}: {error}", tools_home.display()))
    })?;
    let tools: BTreeMap<String, String> = installed_tools(installed_root)
        .into_iter()
        .map(|(name, executable)| (name, executable.display().to_string()))
        .collect();
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&tools)
            .map_err(|error| SupplyError::Serialization(error.to_string()))?,
    )
    .map_err(|error| SupplyError::Protocol(format!("write {}: {error}", path.display())))?;
    Ok(path)
}

/// One secret a project holds, as the Workbench lists it: the entry and its
/// type, never the value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSecretEntry {
    pub vault_ref: String,
    pub type_id: String,
}

/// What a project's vault holds and may hold.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProjectSecrets {
    /// The types loaded packages declare, from the vocabulary.
    pub types: Vec<crate::SupplySecretType>,
    pub entries: Vec<ProjectSecretEntry>,
}

fn read_vault(file: &Path) -> Result<BTreeMap<String, String>, SupplyError> {
    if !file.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(file)
        .map_err(|error| SupplyError::Protocol(format!("read {}: {error}", file.display())))?;
    serde_json::from_str(&text).map_err(|error| SupplyError::Serialization(error.to_string()))
}

/// The entries of a project's vault file, without values.
///
/// # Errors
///
/// Returns [`SupplyError`] when the file exists and cannot be read.
pub fn project_secret_entries(file: &Path) -> Result<Vec<ProjectSecretEntry>, SupplyError> {
    Ok(read_vault(file)?
        .into_keys()
        .map(|vault_ref| {
            // vault://project/<name>/<type>/<n>
            let type_id = vault_ref.rsplit('/').nth(1).unwrap_or_default().to_owned();
            ProjectSecretEntry { vault_ref, type_id }
        })
        .collect())
}

/// Store one value of one type in a project's vault file and return the
/// entry that names it. Each value is a new entry; a binding that named an
/// older entry keeps naming it.
///
/// # Errors
///
/// Returns [`SupplyError`] when the file cannot be read or written.
pub fn set_project_secret(
    file: &Path,
    project: &str,
    type_id: &str,
    value: &str,
) -> Result<ProjectSecretEntry, SupplyError> {
    let mut vault = read_vault(file)?;
    let prefix = format!("vault://project/{project}/{type_id}/");
    let next = vault
        .keys()
        .filter_map(|key| key.strip_prefix(&prefix))
        .filter_map(|n| n.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        + 1;
    let vault_ref = format!("{prefix}{next}");
    vault.insert(vault_ref.clone(), value.to_owned());
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            SupplyError::Protocol(format!("create {}: {error}", parent.display()))
        })?;
    }
    std::fs::write(
        file,
        serde_json::to_vec_pretty(&vault)
            .map_err(|error| SupplyError::Serialization(error.to_string()))?,
    )
    .map_err(|error| SupplyError::Protocol(format!("write {}: {error}", file.display())))?;
    Ok(ProjectSecretEntry {
        vault_ref,
        type_id: type_id.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_vault_numbers_entries_and_lists_them_without_values() {
        let directory = std::env::temp_dir().join(format!("swem-vault-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let file = directory.join(PROJECT_VAULT_FILE);
        let first = set_project_secret(&file, "teaser", "api-key", "one").unwrap();
        let second = set_project_secret(&file, "teaser", "api-key", "two").unwrap();
        assert_eq!(first.vault_ref, "vault://project/teaser/api-key/1");
        assert_eq!(second.vault_ref, "vault://project/teaser/api-key/2");
        let listed = project_secret_entries(&file).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|entry| entry.type_id == "api-key"));
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("\"one\"") && text.contains("\"two\""));
        assert!(!serde_json::to_string(&listed).unwrap().contains("one"));
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_platform_is_spelled_as_a_target_triple() {
        let platform = current_platform();
        assert!(platform.contains('-'), "{platform}");
        assert!(platform.starts_with(std::env::consts::ARCH));
    }
}
