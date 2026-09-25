//! Durable, non-secret configuration for one personal native agent.
//!
//! Profiles belong to the meta-harness inventory. They select existing supply,
//! environment, permission, attachment and credential bindings, but never own
//! ACP conversation state, an active environment lease, connection-local MCP
//! commands or credential material.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::McpServer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AttachmentBinding, BackendProbe, BackendProbeStatus, EnvironmentError, EnvironmentRequirements,
    EnvironmentTransport, IntegrationKind, LaunchCommand, NativeSessionOptions,
    NativeSessionOutcome, PodmanContainerSpec, PodmanNetworkPolicy, PodmanRuntimeSecret,
    PodmanWorkspaceBinding, SupplyError, cleanup_podman_lease, prepare_podman_lease,
    run_native_session,
};

pub const PERSONAL_AGENT_PROFILE_SCHEMA: &str = "swem:personal-agent-profile@0.1";
pub const AGENT_DISTRIBUTION_RECEIPT_SCHEMA: &str = "swem:agent-distribution-receipt@0.1";

/// Verified identities needed to launch one installed adapter artifact.
///
/// The receipt deliberately distinguishes the frozen npm tree from the OCI
/// artifact that contains it. Equal inputs do not claim byte-identical image
/// builds unless a separate build attestation proves that stronger property.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDistributionReceipt {
    pub schema: String,
    pub distribution_id: String,
    pub registry_id: String,
    pub publisher: String,
    pub package: String,
    pub version: String,
    pub package_integrity: String,
    pub lockfile_sha256: String,
    pub platform: String,
    pub resolved_image: String,
    pub agent_executable: String,
    #[serde(default)]
    pub agent_args: Vec<String>,
}

impl AgentDistributionReceipt {
    /// Validate that every identity required by active launch is explicit.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an incomplete or weakly identified receipt.
    pub fn validate(&self) -> Result<(), ProfileError> {
        if self.schema != AGENT_DISTRIBUTION_RECEIPT_SCHEMA {
            return Err(ProfileError::InvalidDistribution(format!(
                "unsupported schema {}",
                self.schema
            )));
        }
        for (field, value) in [
            ("distribution_id", self.distribution_id.as_str()),
            ("registry_id", self.registry_id.as_str()),
            ("publisher", self.publisher.as_str()),
            ("package", self.package.as_str()),
            ("version", self.version.as_str()),
            ("package_integrity", self.package_integrity.as_str()),
            ("platform", self.platform.as_str()),
            ("agent_executable", self.agent_executable.as_str()),
        ] {
            require_non_empty(field, value).map_err(ProfileError::InvalidDistribution)?;
        }
        validate_id("distribution_id", &self.distribution_id)
            .map_err(ProfileError::InvalidDistribution)?;
        if !self.package_integrity.starts_with("sha512-") {
            return Err(ProfileError::InvalidDistribution(
                "package_integrity must be an npm sha512 SRI value".into(),
            ));
        }
        validate_sha256("lockfile_sha256", &self.lockfile_sha256)
            .map_err(ProfileError::InvalidDistribution)?;
        validate_sha256("resolved_image", &self.resolved_image)
            .map_err(ProfileError::InvalidDistribution)?;
        if !self.agent_executable.starts_with('/') {
            return Err(ProfileError::InvalidDistribution(
                "agent_executable must be an absolute agent-visible path".into(),
            ));
        }
        Ok(())
    }

    /// Hash the exact committed lockfile bytes for a distribution receipt.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] when the lockfile cannot be read.
    pub fn lockfile_digest(path: &Path) -> Result<String, ProfileError> {
        let bytes = fs::read(path).map_err(|error| ProfileError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
    }
}

/// A credential source resolved only at active launch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialSourceRef {
    EnvironmentVariable { name: String },
}

/// Non-secret mapping from a host credential source to an agent-visible name.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CredentialBindingRef {
    pub binding_id: String,
    pub source: CredentialSourceRef,
    pub target_environment: String,
}

impl CredentialBindingRef {
    fn validate(&self) -> Result<(), ProfileError> {
        validate_id("credential binding id", &self.binding_id)
            .map_err(ProfileError::InvalidProfile)?;
        let CredentialSourceRef::EnvironmentVariable { name } = &self.source;
        validate_environment_name("credential source environment", name)
            .map_err(ProfileError::InvalidProfile)?;
        validate_environment_name("credential target environment", &self.target_environment)
            .map_err(ProfileError::InvalidProfile)
    }
}

/// One skill a person wrote for their agent: a name, when to use it, and the
/// instructions. Written into the agent's skills folder as `SKILL.md` before
/// a session, in the shape every agent that reads skills reads.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSkill {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: String,
}

/// What a person sets an agent up with beyond where it runs and what it
/// reaches: which model provider and model it answers from, the role it is
/// given, and the skills it carries. Every field has a default, so a profile
/// written before these existed reads as one with none of them set.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSetup {
    /// A model provider's id, from the host's book of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    /// The model, by the id the provider lists it under.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The role, as the person wrote it: the system-prompt addendum written
    /// into the agent's instruction file. Kept verbatim, trailing newline and
    /// all, because canonical form must be reachable from what was stored.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_skills: Vec<AgentSkill>,
}

/// Durable host configuration of a personal native agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PersonalAgentProfile {
    pub schema: String,
    pub profile_id: String,
    /// How many times a person has amended this exact profile. Identity is the
    /// id; the revision is what changed under it, so a session opened against
    /// an older configuration can say so instead of failing as drift.
    #[serde(default)]
    pub revision: u64,
    pub agent_id: String,
    pub distribution_ref: String,
    pub environment_profile_id: String,
    pub permission_profile_id: String,
    pub workspace: PathBuf,
    pub agent_home: PathBuf,
    #[serde(default)]
    pub attachments: Vec<AttachmentBinding>,
    #[serde(default)]
    pub credential_bindings: Vec<CredentialBindingRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_skills: Vec<AgentSkill>,
}

impl PersonalAgentProfile {
    /// The profile with its setup replaced: the model provider, the model,
    /// the role and the skills. Everything else stays. Skills are sorted by
    /// name and a repeated or unusable name is refused, because a name
    /// becomes a folder.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for a blank or unsafe provider, model or
    /// skill name, or a skill named twice.
    pub fn with_setup(mut self, setup: AgentSetup) -> Result<Self, ProfileError> {
        let mut setup = setup;
        if let Some(provider) = &setup.model_provider {
            validate_id("model_provider", provider).map_err(ProfileError::InvalidProfile)?;
        }
        if let Some(model) = &setup.model {
            require_non_empty("model", model).map_err(ProfileError::InvalidProfile)?;
            if model.chars().any(char::is_whitespace) {
                return Err(ProfileError::InvalidProfile(
                    "a model id holds no whitespace".into(),
                ));
            }
        }
        setup
            .agent_skills
            .sort_by(|left, right| left.name.cmp(&right.name));
        let mut names = BTreeSet::new();
        for skill in &setup.agent_skills {
            validate_id("skill name", &skill.name).map_err(ProfileError::InvalidProfile)?;
            if !names.insert(skill.name.as_str()) {
                return Err(ProfileError::InvalidProfile(format!(
                    "duplicate skill name {}",
                    skill.name
                )));
            }
        }
        self.model_provider = setup.model_provider;
        self.model = setup.model;
        self.role = setup.role;
        self.agent_skills = setup.agent_skills;
        Ok(self)
    }

    /// The setup this profile carries, as one value a form edits whole.
    #[must_use]
    pub fn setup(&self) -> AgentSetup {
        AgentSetup {
            model_provider: self.model_provider.clone(),
            model: self.model.clone(),
            role: self.role.clone(),
            agent_skills: self.agent_skills.clone(),
        }
    }

    /// Canonicalize a minimal create-only profile.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for ambiguous identity, nonexistent paths or
    /// duplicate attachment/credential bindings.
    #[allow(
        clippy::too_many_arguments,
        reason = "the constructor exposes each independent host binding"
    )]
    pub fn new(
        profile_id: impl Into<String>,
        agent_id: impl Into<String>,
        distribution_ref: impl Into<String>,
        environment_profile_id: impl Into<String>,
        permission_profile_id: impl Into<String>,
        workspace: &Path,
        agent_home: &Path,
        mut attachments: Vec<AttachmentBinding>,
        mut credential_bindings: Vec<CredentialBindingRef>,
    ) -> Result<Self, ProfileError> {
        let profile_id = profile_id.into();
        let agent_id = agent_id.into();
        let distribution_ref = distribution_ref.into();
        let environment_profile_id = environment_profile_id.into();
        let permission_profile_id = permission_profile_id.into();
        for (field, value) in [
            ("profile_id", profile_id.as_str()),
            ("agent_id", agent_id.as_str()),
            ("distribution_ref", distribution_ref.as_str()),
            ("environment_profile_id", environment_profile_id.as_str()),
            ("permission_profile_id", permission_profile_id.as_str()),
        ] {
            validate_id(field, value).map_err(ProfileError::InvalidProfile)?;
        }
        let workspace = canonical_directory("workspace", workspace)?;
        let agent_home = canonical_directory("agent_home", agent_home)?;
        if workspace == agent_home {
            return Err(ProfileError::InvalidProfile(
                "workspace and native agent home must be distinct".into(),
            ));
        }
        attachments.sort();
        let mut server_names = BTreeSet::new();
        let mut attachment_profiles = BTreeSet::new();
        for attachment in &attachments {
            validate_id("attachment profile id", &attachment.profile_id)
                .map_err(ProfileError::InvalidProfile)?;
            require_non_empty("attachment server name", &attachment.server_name)
                .map_err(ProfileError::InvalidProfile)?;
            if !server_names.insert(attachment.server_name.as_str()) {
                return Err(ProfileError::InvalidProfile(format!(
                    "duplicate MCP server name {}",
                    attachment.server_name
                )));
            }
            if !attachment_profiles.insert(attachment.profile_id.as_str()) {
                return Err(ProfileError::InvalidProfile(format!(
                    "duplicate attachment profile id {}",
                    attachment.profile_id
                )));
            }
        }
        credential_bindings.sort_by(|left, right| left.binding_id.cmp(&right.binding_id));
        let mut credential_ids = BTreeSet::new();
        let mut target_names = BTreeSet::new();
        for binding in &credential_bindings {
            binding.validate()?;
            if !credential_ids.insert(binding.binding_id.as_str()) {
                return Err(ProfileError::InvalidProfile(format!(
                    "duplicate credential binding id {}",
                    binding.binding_id
                )));
            }
            if !target_names.insert(binding.target_environment.as_str()) {
                return Err(ProfileError::InvalidProfile(format!(
                    "duplicate credential target {}",
                    binding.target_environment
                )));
            }
        }
        Ok(Self {
            schema: PERSONAL_AGENT_PROFILE_SCHEMA.into(),
            profile_id,
            revision: 0,
            agent_id,
            distribution_ref,
            environment_profile_id,
            permission_profile_id,
            workspace,
            agent_home,
            attachments,
            credential_bindings,
            model_provider: None,
            model: None,
            role: String::new(),
            agent_skills: Vec::new(),
        })
    }

    fn validate_loaded(&self, expected_id: &str) -> Result<(), ProfileError> {
        if self.schema != PERSONAL_AGENT_PROFILE_SCHEMA {
            return Err(ProfileError::InvalidProfile(format!(
                "unsupported schema {}",
                self.schema
            )));
        }
        if self.profile_id != expected_id {
            return Err(ProfileError::InvalidProfile(format!(
                "profile id {} does not match inventory key {expected_id}",
                self.profile_id
            )));
        }
        let mut normalized = Self::new(
            self.profile_id.clone(),
            self.agent_id.clone(),
            self.distribution_ref.clone(),
            self.environment_profile_id.clone(),
            self.permission_profile_id.clone(),
            &self.workspace,
            &self.agent_home,
            self.attachments.clone(),
            self.credential_bindings.clone(),
        )?
        .with_setup(self.setup())?;
        normalized.revision = self.revision;
        if normalized == *self {
            Ok(())
        } else {
            Err(ProfileError::InvalidProfile(
                "stored profile is not in canonical form".into(),
            ))
        }
    }
}

/// Filesystem inventory for personal-agent profiles.
///
/// Creation stays create-only: an id is an identity and is never taken over by
/// a second `create`. Amending that same identity is a separate operation with
/// its own revision check, because a person configures an agent over time and
/// a new id for every change would strand the sessions already bound to it.
#[derive(Clone, Debug)]
pub struct PersonalAgentProfileStore {
    root: PathBuf,
}

impl PersonalAgentProfileStore {
    /// Open an inventory directory, creating only that exact directory.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] if the directory cannot be created/canonicalized.
    pub fn open(root: &Path) -> Result<Self, ProfileError> {
        fs::create_dir_all(root).map_err(|error| ProfileError::Io {
            path: root.to_path_buf(),
            message: error.to_string(),
        })?;
        let root = fs::canonicalize(root).map_err(|error| ProfileError::Io {
            path: root.to_path_buf(),
            message: error.to_string(),
        })?;
        Ok(Self { root })
    }

    /// Persist one profile without overwriting an existing identity.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::ProfileExists`] on overwrite and removes an
    /// incomplete temporary file after any failed write.
    pub fn create(&self, profile: &PersonalAgentProfile) -> Result<PathBuf, ProfileError> {
        profile.validate_loaded(&profile.profile_id)?;
        let directory = self.root.join(&profile.profile_id);
        fs::create_dir(&directory).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                ProfileError::ProfileExists(profile.profile_id.clone())
            } else {
                ProfileError::Io {
                    path: directory.clone(),
                    message: error.to_string(),
                }
            }
        })?;
        let target = directory.join("profile.json");
        let temporary = directory.join(format!("profile.{}.tmp", std::process::id()));
        let result = (|| {
            let bytes = serde_json::to_vec_pretty(profile)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| ProfileError::Io {
                    path: temporary.clone(),
                    message: error.to_string(),
                })?;
            file.write_all(&bytes).map_err(|error| ProfileError::Io {
                path: temporary.clone(),
                message: error.to_string(),
            })?;
            file.sync_all().map_err(|error| ProfileError::Io {
                path: temporary.clone(),
                message: error.to_string(),
            })?;
            fs::rename(&temporary, &target).map_err(|error| ProfileError::Io {
                path: target.clone(),
                message: error.to_string(),
            })?;
            Ok(target.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_dir(&directory);
        }
        result
    }

    /// Amend one profile that already exists, keeping its identity.
    ///
    /// The id is the identity and never changes here; everything a person can
    /// configure - where the agent works, which MCP servers it attaches, which
    /// host credentials it carries - does. `wanted.revision` must be the
    /// revision the person edited, so two surfaces editing at once cannot
    /// silently overwrite one another.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError::ProfileNotFound`] for an unknown id,
    /// [`ProfileError::ProfileChanged`] when the stored revision moved on, and
    /// the ordinary validation errors for an incoherent profile.
    pub fn amend(
        &self,
        wanted: &PersonalAgentProfile,
    ) -> Result<PersonalAgentProfile, ProfileError> {
        let existing = self.load(&wanted.profile_id)?;
        if existing.revision != wanted.revision {
            return Err(ProfileError::ProfileChanged {
                profile_id: wanted.profile_id.clone(),
                current: existing.revision,
                edited: wanted.revision,
            });
        }
        let mut amended = wanted.clone();
        amended.revision = existing.revision.saturating_add(1);
        amended.validate_loaded(&amended.profile_id)?;
        let directory = self.root.join(&amended.profile_id);
        let target = directory.join("profile.json");
        let temporary = directory.join(format!("profile.amend.{}.tmp", std::process::id()));
        let result = (|| {
            let bytes = serde_json::to_vec_pretty(&amended)?;
            let mut file = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| ProfileError::Io {
                    path: temporary.clone(),
                    message: error.to_string(),
                })?;
            file.write_all(&bytes).map_err(|error| ProfileError::Io {
                path: temporary.clone(),
                message: error.to_string(),
            })?;
            file.sync_all().map_err(|error| ProfileError::Io {
                path: temporary.clone(),
                message: error.to_string(),
            })?;
            fs::rename(&temporary, &target).map_err(|error| ProfileError::Io {
                path: target.clone(),
                message: error.to_string(),
            })
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(amended)
    }

    /// Reload and revalidate one exact profile after a host restart.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an unsafe id, absent/malformed profile or
    /// path/binding drift.
    pub fn load(&self, profile_id: &str) -> Result<PersonalAgentProfile, ProfileError> {
        validate_id("profile_id", profile_id).map_err(ProfileError::InvalidProfile)?;
        let path = self.root.join(profile_id).join("profile.json");
        let mut file = File::open(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ProfileError::ProfileNotFound(profile_id.into())
            } else {
                ProfileError::Io {
                    path: path.clone(),
                    message: error.to_string(),
                }
            }
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| ProfileError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
        let profile: PersonalAgentProfile = serde_json::from_slice(&bytes)?;
        profile.validate_loaded(profile_id)?;
        Ok(profile)
    }

    /// Return every valid profile in stable profile-id order.
    ///
    /// # Errors
    ///
    /// Fails closed when the inventory contains an unexpected filesystem entry
    /// or any profile cannot be loaded and revalidated.
    pub fn list(&self) -> Result<Vec<PersonalAgentProfile>, ProfileError> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| ProfileError::Io {
            path: self.root.clone(),
            message: error.to_string(),
        })? {
            let entry = entry.map_err(|error| ProfileError::Io {
                path: self.root.clone(),
                message: error.to_string(),
            })?;
            let file_type = entry.file_type().map_err(|error| ProfileError::Io {
                path: entry.path(),
                message: error.to_string(),
            })?;
            if !file_type.is_dir() || file_type.is_symlink() {
                return Err(ProfileError::InvalidProfile(format!(
                    "unexpected entry in profile inventory: {}",
                    entry.path().display()
                )));
            }
            let id = entry.file_name().into_string().map_err(|_| {
                ProfileError::InvalidProfile("profile inventory id is not UTF-8".into())
            })?;
            validate_id("profile_id", &id).map_err(ProfileError::InvalidProfile)?;
            ids.push(id);
        }
        ids.sort();
        ids.into_iter().map(|id| self.load(&id)).collect()
    }

    /// Resolve an exact profile for an invocation without creating mutable
    /// global "current profile" state.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] under the same conditions as [`Self::load`].
    pub fn select(&self, profile_id: &str) -> Result<PersonalAgentProfile, ProfileError> {
        self.load(profile_id)
    }

    fn secrets_path(&self, profile_id: &str) -> Result<PathBuf, ProfileError> {
        validate_id("profile_id", profile_id).map_err(ProfileError::InvalidProfile)?;
        let directory = self.root.join(profile_id);
        if !directory.join("profile.json").is_file() {
            return Err(ProfileError::ProfileNotFound(profile_id.into()));
        }
        Ok(directory.join("secrets.json"))
    }

    /// What a person told this profile, never the values: one entry per
    /// environment variable, with the typed reason it exists.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an unknown profile or an unreadable file.
    pub fn secret_entries(&self, profile_id: &str) -> Result<Vec<SecretEntry>, ProfileError> {
        Ok(self
            .stored_secrets(profile_id)?
            .into_iter()
            .map(|(name, stored)| SecretEntry {
                name,
                type_id: stored.type_id,
                label: stored.label,
            })
            .collect())
    }

    /// The secrets a person gave this profile, by environment variable name.
    ///
    /// These are what an agent reads at start - an API key, an OAuth token,
    /// an endpoint - and what an ACP `env_var` authentication method asks
    /// for. A person types each once; it is handed to the agent process
    /// under the variable's name and never written anywhere the ledger or a
    /// transcript can reach. They live beside the profile rather than in it
    /// because the profile is a durable identity that may be listed, copied
    /// and compared; a secret is none of those things.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an unknown profile or an unreadable file.
    pub fn secrets(&self, profile_id: &str) -> Result<BTreeMap<String, String>, ProfileError> {
        Ok(self
            .stored_secrets(profile_id)?
            .into_iter()
            .map(|(name, stored)| (name, stored.value))
            .collect())
    }

    fn stored_secrets(
        &self,
        profile_id: &str,
    ) -> Result<BTreeMap<String, StoredSecret>, ProfileError> {
        let path = self.secrets_path(profile_id)?;
        match fs::read(&path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(ProfileError::Io {
                path,
                message: error.to_string(),
            }),
        }
    }

    /// Store one secret under its environment variable name, replacing any
    /// previous value.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an unknown profile, an unknown type, a
    /// name that is not an environment variable name, or a failed write.
    pub fn set_secret(
        &self,
        profile_id: &str,
        type_id: &str,
        label: &str,
        name: &str,
        value: &str,
    ) -> Result<(), ProfileError> {
        if !secret_types().iter().any(|kind| kind.type_id == type_id) {
            return Err(ProfileError::InvalidProfile(format!(
                "unknown secret type {type_id:?}"
            )));
        }
        // Blanks are not a value. A key of spaces is handed to the agent as
        // an environment variable of spaces, which fails further away and
        // less legibly than not having it at all. The value is stored as
        // typed; only the emptiness is judged on the trimmed text.
        if value.trim().is_empty() {
            return Err(ProfileError::InvalidProfile(
                "a secret needs a value, not blanks".into(),
            ));
        }
        let mut secrets = self.stored_secrets(profile_id)?;
        secrets.insert(
            valid_variable_name(name)?.to_owned(),
            StoredSecret {
                type_id: type_id.to_owned(),
                label: label.to_owned(),
                value: value.to_owned(),
            },
        );
        self.write_secrets(profile_id, &secrets)
    }

    /// Forget one secret.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] for an unknown profile or a failed write.
    pub fn remove_secret(&self, profile_id: &str, name: &str) -> Result<(), ProfileError> {
        let mut secrets = self.stored_secrets(profile_id)?;
        secrets.remove(valid_variable_name(name)?);
        self.write_secrets(profile_id, &secrets)
    }

    fn write_secrets(
        &self,
        profile_id: &str,
        secrets: &BTreeMap<String, StoredSecret>,
    ) -> Result<(), ProfileError> {
        let path = self.secrets_path(profile_id)?;
        let bytes = serde_json::to_vec_pretty(secrets)?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let io = |error: std::io::Error| ProfileError::Io {
            path: path.clone(),
            message: error.to_string(),
        };
        {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            // Owner-only from the first byte: a secret must never spend even
            // a moment world-readable between create and chmod.
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt as _;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary).map_err(io)?;
            file.write_all(&bytes).map_err(io)?;
            file.sync_all().map_err(io)?;
        }
        fs::rename(&temporary, &path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            io(error)
        })
    }
}

fn valid_variable_name(name: &str) -> Result<&str, ProfileError> {
    let plausible = !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && !name.as_bytes()[0].is_ascii_digit();
    if plausible {
        Ok(name)
    } else {
        Err(ProfileError::InvalidProfile(format!(
            "secret name {name:?} is not an environment variable name"
        )))
    }
}

/// One stored secret: the typed reason it exists, and the value.
#[derive(Clone, Deserialize, Serialize)]
struct StoredSecret {
    type_id: String,
    label: String,
    value: String,
}

/// The value never reaches a log through `{:?}`, whoever formats it.
impl std::fmt::Debug for StoredSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredSecret")
            .field("type_id", &self.type_id)
            .field("label", &self.label)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// A secret as a surface may list it: everything but the value.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SecretEntry {
    /// The environment variable the agent reads it from.
    pub name: String,
    pub type_id: String,
    pub label: String,
}

/// A kind of secret a person can give an agent, and the environment
/// variable an agent conventionally reads it from.
///
/// The catalogue is the product's, not any one agent's: an agent's own
/// `env_var` authentication method may name a variable outside it, and
/// `generic_env_var` covers that. Adding a kind here is a product change,
/// never a per-agent one.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct SecretType {
    pub type_id: &'static str,
    pub label: &'static str,
    /// The conventional variable; `None` means the person names it.
    pub env_var: Option<&'static str>,
    pub hint: &'static str,
}

/// The kinds of secret an agent's profile may hold, as data: the catalogue
/// is `agent-secrets.json` beside this file, read once. A kind is a product
/// fact, never a per-agent one; a package's own secret types come through
/// the Cycle's vocabulary and live in a project's vault, not here.
///
/// # Panics
///
/// Panics when the embedded catalogue is not the JSON this build was made
/// with - a build defect, never a runtime condition.
#[must_use]
pub fn secret_types() -> &'static [SecretType] {
    static TYPES: std::sync::OnceLock<&'static [SecretType]> = std::sync::OnceLock::new();
    TYPES.get_or_init(|| {
        #[derive(serde::Deserialize)]
        struct Declared {
            type_id: String,
            label: String,
            #[serde(default)]
            env_var: Option<String>,
            #[serde(default)]
            hint: String,
        }
        let declared: Vec<Declared> = serde_json::from_str(include_str!("agent-secrets.json"))
            .expect("the agent secret catalogue is valid JSON");
        let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
        let types: Vec<SecretType> = declared
            .into_iter()
            .map(|kind| SecretType {
                type_id: leak(kind.type_id),
                label: leak(kind.label),
                env_var: kind.env_var.map(leak),
                hint: leak(kind.hint),
            })
            .collect();
        Box::leak(types.into_boxed_slice())
    })
}

/// One connection-local MCP declaration resolving a durable attachment identity.
#[derive(Clone, Debug)]
pub struct ResolvedMcpAttachment {
    pub binding: AttachmentBinding,
    pub server: McpServer,
}

/// Concrete inputs used to assemble one profile on one ready Podman endpoint.
///
/// Service paths, ACP commands and MCP connection data deliberately live here,
/// not in [`PersonalAgentProfile`]. A different host/provider resolver may
/// produce another connection for the same durable profile.
#[derive(Clone, Debug)]
pub struct ResolvedPodmanAgentConnection {
    pub distribution: AgentDistributionReceipt,
    pub environment_profile_id: String,
    pub permission_profile_id: String,
    pub probe: BackendProbe,
    pub requirements: EnvironmentRequirements,
    pub workspace: PodmanWorkspaceBinding,
    pub agent_home: PodmanWorkspaceBinding,
    pub network: PodmanNetworkPolicy,
    pub cpu_limit: u16,
    pub memory_mib: u64,
    pub pids_limit: u32,
    pub attachments: Vec<ResolvedMcpAttachment>,
}

/// Product-owned outcome of one profile launch without copying native history.
#[derive(Debug)]
pub struct PersonalAgentRunOutcome {
    pub session: NativeSessionOutcome,
    pub distribution: AgentDistributionReceipt,
    pub credential_cleanup: Vec<CredentialCleanupReceipt>,
}

/// Assemble and run one durable profile through the existing native ACP seam.
///
/// # Errors
///
/// Returns [`PersonalAgentRunError`] when a durable identity does not match the
/// connection resolver, credential/lease preparation fails, the native session
/// fails, or terminal credential cleanup cannot be proven.
#[allow(
    clippy::too_many_lines,
    reason = "one linear lease transaction keeps secret creation, ACP execution and terminal cleanup auditable"
)]
pub async fn run_personal_agent_in_podman(
    profile: &PersonalAgentProfile,
    connection: &ResolvedPodmanAgentConnection,
    prompts: &[String],
    mut options: NativeSessionOptions,
) -> Result<PersonalAgentRunOutcome, PersonalAgentRunError> {
    validate_resolved_connection(profile, connection)?;
    if options.environment_lease.is_some()
        || options.environment_transport.is_some()
        || !options.process_environment_overrides.is_empty()
        || !options.mcp_servers.is_empty()
    {
        return Err(ProfileError::InvalidProfile(
            "profile runner owns environment transport, process values and resolved MCP declarations"
                .into(),
        )
        .into());
    }
    let mut credentials = Vec::with_capacity(profile.credential_bindings.len());
    for binding in &profile.credential_bindings {
        match PodmanCredentialLease::materialize(&connection.probe, binding) {
            Ok(credential) => credentials.push(credential),
            Err(error) => {
                let cleanup = cleanup_credentials(credentials);
                return match cleanup {
                    Ok(_) => Err(error.into()),
                    Err(cleanup) => Err(PersonalAgentRunError::CleanupAfterFailure {
                        primary: error.to_string(),
                        cleanup: cleanup.to_string(),
                    }),
                };
            }
        }
    }
    let mut spec = PodmanContainerSpec::deny_network(
        connection.distribution.resolved_image.clone(),
        connection.workspace.clone(),
        connection.distribution.agent_executable.clone(),
    );
    spec.agent_home = Some(connection.agent_home.clone());
    spec.runtime_secrets = credentials
        .iter()
        .map(PodmanCredentialLease::runtime_secret)
        .collect();
    spec.network = connection.network;
    spec.agent_args
        .clone_from(&connection.distribution.agent_args);
    spec.cpu_limit = connection.cpu_limit;
    spec.memory_mib = connection.memory_mib;
    spec.pids_limit = connection.pids_limit;
    let lease_id = format!(
        "profile-{}-{}-{}",
        sanitize_podman_name(&profile.profile_id),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ProfileError::InvalidProfile(error.to_string()))?
            .as_nanos()
    );
    let lease = match prepare_podman_lease(
        &connection.probe,
        &lease_id,
        &connection.requirements,
        &spec,
    ) {
        Ok(lease) => lease,
        Err(error) => {
            return fail_after_cleanup(error.to_string(), credentials);
        }
    };
    let transport = match EnvironmentTransport::podman_endpoint(&lease, &connection.probe) {
        Ok(transport) => transport,
        Err(error) => {
            let primary = match cleanup_podman_lease(&connection.probe, &lease) {
                Ok(_) => error.to_string(),
                Err(cleanup) => {
                    format!("{error}; prepared environment cleanup also failed: {cleanup}")
                }
            };
            return fail_after_cleanup(primary, credentials);
        }
    };
    // Recovery handle for the failure paths below: if the session errors
    // before the transport ever spawned, the guard inside the transport never
    // armed and the prepared container would otherwise outlive this call.
    // `podman rm --force --ignore` is idempotent, so running it after a
    // guard-cleaned session is safe.
    let lease_for_recovery = lease.clone();
    options.environment_lease = Some(lease);
    options.environment_transport = Some(transport);
    options.mcp_servers = connection
        .attachments
        .iter()
        .map(|attachment| attachment.server.clone())
        .collect();
    let launch = LaunchCommand {
        executable: connection.distribution.agent_executable.clone(),
        args: connection.distribution.agent_args.clone(),
        integration: IntegrationKind::AcpAdapter,
    };
    // With a container transport the executable parameter is transcript
    // identity, not a spawn path: record the real container-side executable
    // instead of a fabricated placeholder.
    let session = run_native_session(
        &launch,
        Path::new(&connection.distribution.agent_executable),
        &profile.workspace,
        prompts,
        &options,
    )
    .await;
    let cleanup = cleanup_credentials(credentials);
    if session.is_err() {
        // Best-effort container recovery for failures before the transport
        // guard armed; a failure here is reported, never masked as success.
        if let Err(error) = cleanup_podman_lease(&connection.probe, &lease_for_recovery) {
            let primary = session
                .as_ref()
                .err()
                .map_or_else(String::new, ToString::to_string);
            return Err(PersonalAgentRunError::CleanupAfterFailure {
                primary,
                cleanup: error.to_string(),
            });
        }
    }
    match (session, cleanup) {
        (Ok(session), Ok(credential_cleanup)) => Ok(PersonalAgentRunOutcome {
            session,
            distribution: connection.distribution.clone(),
            credential_cleanup,
        }),
        (Err(primary), Ok(_)) => Err(primary.into()),
        (Ok(_), Err(cleanup)) => Err(cleanup.into()),
        (Err(primary), Err(cleanup)) => Err(PersonalAgentRunError::CleanupAfterFailure {
            primary: primary.to_string(),
            cleanup: cleanup.to_string(),
        }),
    }
}

fn validate_resolved_connection(
    profile: &PersonalAgentProfile,
    connection: &ResolvedPodmanAgentConnection,
) -> Result<(), PersonalAgentRunError> {
    profile.validate_loaded(&profile.profile_id)?;
    connection.distribution.validate()?;
    if profile.distribution_ref != connection.distribution.distribution_id {
        return Err(ProfileError::InvalidProfile(
            "resolved distribution does not match profile reference".into(),
        )
        .into());
    }
    if profile.environment_profile_id != connection.environment_profile_id
        || profile.permission_profile_id != connection.permission_profile_id
    {
        return Err(ProfileError::InvalidProfile(
            "resolved environment or permission profile identity drifted".into(),
        )
        .into());
    }
    let resolved_workspace =
        canonical_directory("connection workspace", &connection.requirements.workspace)?;
    if resolved_workspace != profile.workspace {
        return Err(ProfileError::InvalidProfile(
            "connection requirements are bound to another workspace".into(),
        )
        .into());
    }
    if connection.probe.backend_id != "podman"
        || connection.probe.status != BackendProbeStatus::Ready
    {
        return Err(ProfileError::InvalidProfile(
            "profile connection requires an exact ready Podman backend probe".into(),
        )
        .into());
    }
    let mut resolved = connection
        .attachments
        .iter()
        .map(|attachment| attachment.binding.clone())
        .collect::<Vec<_>>();
    resolved.sort();
    if resolved != profile.attachments {
        return Err(ProfileError::InvalidProfile(
            "resolved MCP attachment identities do not match the durable profile".into(),
        )
        .into());
    }
    for attachment in &connection.attachments {
        let wire = serde_json::to_value(&attachment.server).map_err(ProfileError::from)?;
        if wire.get("name").and_then(serde_json::Value::as_str)
            != Some(attachment.binding.server_name.as_str())
        {
            return Err(ProfileError::InvalidProfile(format!(
                "resolved MCP server does not preserve name {}",
                attachment.binding.server_name
            ))
            .into());
        }
    }
    Ok(())
}

fn cleanup_credentials(
    credentials: Vec<PodmanCredentialLease>,
) -> Result<Vec<CredentialCleanupReceipt>, ProfileError> {
    let mut receipts = Vec::with_capacity(credentials.len());
    for credential in credentials {
        receipts.push(credential.remove()?);
    }
    Ok(receipts)
}

fn fail_after_cleanup<T>(
    primary: String,
    credentials: Vec<PodmanCredentialLease>,
) -> Result<T, PersonalAgentRunError> {
    match cleanup_credentials(credentials) {
        Ok(_) => Err(PersonalAgentRunError::EnvironmentMessage(primary)),
        Err(cleanup) => Err(PersonalAgentRunError::CleanupAfterFailure {
            primary,
            cleanup: cleanup.to_string(),
        }),
    }
}

#[derive(Debug, Error)]
pub enum PersonalAgentRunError {
    #[error(transparent)]
    Profile(#[from] ProfileError),
    #[error(transparent)]
    Environment(#[from] EnvironmentError),
    #[error("environment preparation failed: {0}")]
    EnvironmentMessage(String),
    #[error(transparent)]
    Supply(#[from] SupplyError),
    #[error("profile run failed ({primary}); credential cleanup also failed ({cleanup})")]
    CleanupAfterFailure { primary: String, cleanup: String },
}

/// Active Podman secret corresponding to one non-secret profile binding.
pub struct PodmanCredentialLease {
    executable: PathBuf,
    command_prefix: Vec<String>,
    binding_id: String,
    secret_name: String,
    target_environment: String,
    removed: bool,
}

impl std::fmt::Debug for PodmanCredentialLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PodmanCredentialLease")
            .field("binding_id", &self.binding_id)
            .field("secret_name", &self.secret_name)
            .field("target_environment", &self.target_environment)
            .field("removed", &self.removed)
            .finish_non_exhaustive()
    }
}

impl PodmanCredentialLease {
    /// Materialize a profile credential through Podman's secret stdin.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] when the backend is not the exact ready Podman
    /// endpoint, the source is unavailable, or Podman cannot create/inspect the
    /// short-lived secret. The value is never placed in argv or returned.
    pub fn materialize(
        probe: &BackendProbe,
        binding: &CredentialBindingRef,
    ) -> Result<Self, ProfileError> {
        binding.validate()?;
        if probe.backend_id != "podman" || probe.status != BackendProbeStatus::Ready {
            return Err(ProfileError::Credential(format!(
                "backend {} is not a ready Podman endpoint",
                probe.backend_id
            )));
        }
        let executable = probe.executable.clone().ok_or_else(|| {
            ProfileError::Credential("ready Podman endpoint has no executable".into())
        })?;
        let CredentialSourceRef::EnvironmentVariable { name } = &binding.source;
        let value = std::env::var(name).map_err(|_| {
            ProfileError::Credential(format!(
                "credential source environment {name} is unavailable"
            ))
        })?;
        if value.is_empty() {
            return Err(ProfileError::Credential(format!(
                "credential source environment {name} is empty"
            )));
        }
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| ProfileError::Credential(error.to_string()))?
            .as_nanos();
        let secret_name = format!(
            "swem-{}-{}-{nonce}",
            sanitize_podman_name(&binding.binding_id),
            std::process::id()
        );
        let mut command = Command::new(&executable);
        command
            .args(&probe.command_prefix)
            .args(["secret", "create", &secret_name, "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| ProfileError::Credential(format!("podman secret create: {error}")))?;
        child
            .stdin
            .take()
            .ok_or_else(|| ProfileError::Credential("podman secret stdin unavailable".into()))?
            .write_all(value.as_bytes())
            .map_err(|error| ProfileError::Credential(format!("podman secret stdin: {error}")))?;
        drop(value);
        let output = child
            .wait_with_output()
            .map_err(|error| ProfileError::Credential(format!("podman secret create: {error}")))?;
        if !output.status.success() {
            return Err(ProfileError::Credential(format!(
                "podman secret create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let lease = Self {
            executable,
            command_prefix: probe.command_prefix.clone(),
            binding_id: binding.binding_id.clone(),
            secret_name,
            target_environment: binding.target_environment.clone(),
            removed: false,
        };
        lease.require_exists(true)?;
        Ok(lease)
    }

    #[must_use]
    pub fn runtime_secret(&self) -> PodmanRuntimeSecret {
        PodmanRuntimeSecret {
            opaque_reference: self.secret_name.clone(),
            target_environment: self.target_environment.clone(),
        }
    }

    /// Remove the exact secret and observe terminal absence.
    ///
    /// # Errors
    ///
    /// Returns [`ProfileError`] when removal or the terminal existence oracle
    /// fails. The drop guard still makes a best-effort retry.
    pub fn remove(mut self) -> Result<CredentialCleanupReceipt, ProfileError> {
        self.remove_inner()?;
        Ok(CredentialCleanupReceipt {
            binding_id: self.binding_id.clone(),
            target_environment: self.target_environment.clone(),
            terminally_absent: true,
        })
    }

    fn remove_inner(&mut self) -> Result<(), ProfileError> {
        let output = Command::new(&self.executable)
            .args(&self.command_prefix)
            .args(["secret", "rm", "--ignore", &self.secret_name])
            .output()
            .map_err(|error| ProfileError::Credential(format!("podman secret rm: {error}")))?;
        if !output.status.success() {
            return Err(ProfileError::Credential(format!(
                "podman secret rm failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        self.require_exists(false)?;
        self.removed = true;
        Ok(())
    }

    fn require_exists(&self, expected: bool) -> Result<(), ProfileError> {
        let status = Command::new(&self.executable)
            .args(&self.command_prefix)
            .args(["secret", "exists", &self.secret_name])
            .status()
            .map_err(|error| ProfileError::Credential(format!("podman secret exists: {error}")))?;
        let observed = match status.code() {
            Some(0) => true,
            Some(1) => false,
            code => {
                return Err(ProfileError::Credential(format!(
                    "podman secret exists returned {code:?}"
                )));
            }
        };
        if observed == expected {
            Ok(())
        } else {
            Err(ProfileError::Credential(format!(
                "podman secret existence was {observed}, expected {expected}"
            )))
        }
    }
}

impl Drop for PodmanCredentialLease {
    fn drop(&mut self) {
        if !self.removed {
            let _ = self.remove_inner();
        }
    }
}

/// Non-secret terminal evidence emitted after a credential lease is removed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CredentialCleanupReceipt {
    pub binding_id: String,
    pub target_environment: String,
    pub terminally_absent: bool,
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("invalid personal-agent profile: {0}")]
    InvalidProfile(String),
    #[error("invalid agent distribution receipt: {0}")]
    InvalidDistribution(String),
    #[error("personal-agent profile already exists: {0}")]
    ProfileExists(String),
    #[error("personal-agent profile not found: {0}")]
    ProfileNotFound(String),
    #[error(
        "personal-agent profile {profile_id} changed while you were editing it: it is now at revision {current}, you edited revision {edited}"
    )]
    ProfileChanged {
        profile_id: String,
        current: u64,
        edited: u64,
    },
    #[error("profile inventory I/O failed at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("profile serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("credential materialization failed: {0}")]
    Credential(String),
}

/// Write a file so that a reader sees either the old bytes or the new ones:
/// a temporary beside it, synced, then renamed over it.
///
/// # Errors
///
/// Returns the I/O error of the step that failed; the temporary is removed.
pub(crate) fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temporary = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn canonical_directory(field: &str, path: &Path) -> Result<PathBuf, ProfileError> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(ProfileError::InvalidProfile(format!(
            "{field} must be an existing absolute directory: {}",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|error| ProfileError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn require_non_empty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_id(field: &str, value: &str) -> Result<(), String> {
    require_non_empty(field, value)?;
    if value.len() > 128
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "{field} must use 1..128 ASCII letters, digits, dot, dash or underscore"
        ));
    }
    Ok(())
}

fn validate_environment_name(field: &str, value: &str) -> Result<(), String> {
    require_non_empty(field, value)?;
    let mut bytes = value.bytes();
    if !bytes
        .next()
        .is_some_and(|byte| byte == b'_' || byte.is_ascii_alphabetic())
        || !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
    {
        return Err(format!(
            "{field} is not a portable environment variable name"
        ));
    }
    Ok(())
}

fn validate_sha256(field: &str, value: &str) -> Result<(), String> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(format!("{field} must be a sha256: digest"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("{field} must contain 64 hexadecimal digits"));
    }
    Ok(())
}

fn sanitize_podman_name(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}
