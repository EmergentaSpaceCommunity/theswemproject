//! Explicit, receipt-producing acquisition of native-agent OCI distributions.
//!
//! Planning is read-only. Applying a plan revalidates every declared input,
//! stages only those files, builds without pulling base images, inspects the
//! resulting image, and records an immutable host-inventory receipt. Active
//! profile launch consumes the receipt and never builds an image itself.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{
    AGENT_DISTRIBUTION_RECEIPT_SCHEMA, AgentDistributionReceipt, BackendProbe, BackendProbeStatus,
    ProvisioningDisposition,
};

pub const AGENT_DISTRIBUTION_BUILD_PLAN_SCHEMA: &str = "swem:agent-distribution-build-plan@0.1";
pub const AGENT_DISTRIBUTION_BUILD_RECEIPT_SCHEMA: &str =
    "swem:agent-distribution-build-receipt@0.1";

/// Build-time network authority. It applies only while constructing the image.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DistributionBuildNetwork {
    DenyAll,
    PrivateEgress,
}

impl DistributionBuildNetwork {
    fn podman_mode(self) -> &'static str {
        match self {
            Self::DenyAll => "none",
            Self::PrivateEgress => "private",
        }
    }
}

/// One exact local file admitted to the isolated build context.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DistributionBuildInput {
    pub relative_path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
}

/// Caller intent for one locked adapter image build.
///
/// The current profile is deliberately narrow: it covers the locked npm ACP
/// distribution already used by H0. Other registry distribution forms should
/// add sibling planners only after a real adapter requires them.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDistributionBuildSpec {
    pub distribution_id: String,
    pub registry_id: String,
    pub publisher: String,
    pub package: String,
    pub version: String,
    pub package_integrity: String,
    pub platform: String,
    pub context_root: PathBuf,
    pub containerfile: PathBuf,
    pub lockfile: PathBuf,
    pub context_files: Vec<PathBuf>,
    pub agent_executable: String,
    pub agent_args: Vec<String>,
    pub network: DistributionBuildNetwork,
    pub disposition: ProvisioningDisposition,
}

/// Read-only, confirmation-bound preview of one adapter image build.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDistributionBuildPlan {
    pub schema: String,
    pub plan_id: String,
    pub backend_executable: PathBuf,
    pub backend_endpoint_id: String,
    #[serde(default)]
    pub backend_command_prefix: Vec<String>,
    pub builder_version: String,
    pub distribution_id: String,
    pub registry_id: String,
    pub publisher: String,
    pub package: String,
    pub version: String,
    pub package_integrity: String,
    pub platform: String,
    pub context_root: PathBuf,
    pub containerfile: PathBuf,
    pub lockfile: PathBuf,
    pub context_files: Vec<DistributionBuildInput>,
    pub context_sha256: String,
    pub lockfile_sha256: String,
    pub base_images: Vec<String>,
    pub image_tag: String,
    pub agent_executable: String,
    #[serde(default)]
    pub agent_args: Vec<String>,
    pub network: DistributionBuildNetwork,
    pub disposition: ProvisioningDisposition,
    pub command_preview: Vec<String>,
    pub blockers: Vec<String>,
}

/// Post-build inspection receipt stored in host inventory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDistributionBuildReceipt {
    pub schema: String,
    pub plan_id: String,
    pub context_sha256: String,
    pub backend_endpoint_id: String,
    pub builder_version: String,
    pub image_size_bytes: u64,
    pub distribution: AgentDistributionReceipt,
}

impl AgentDistributionBuildReceipt {
    /// Validate the receipt without consulting mutable backend state.
    ///
    /// # Errors
    ///
    /// Returns [`DistributionBuildError`] when the envelope or inner launch
    /// receipt is malformed.
    pub fn validate(&self) -> Result<(), DistributionBuildError> {
        if self.schema != AGENT_DISTRIBUTION_BUILD_RECEIPT_SCHEMA {
            return Err(DistributionBuildError::InvalidReceipt(format!(
                "unsupported schema {}",
                self.schema
            )));
        }
        validate_sha256("plan_id", &self.plan_id)?;
        validate_sha256("context_sha256", &self.context_sha256)?;
        require_non_empty("backend_endpoint_id", &self.backend_endpoint_id)?;
        require_non_empty("builder_version", &self.builder_version)?;
        if self.image_size_bytes == 0 {
            return Err(DistributionBuildError::InvalidReceipt(
                "image_size_bytes must be positive".into(),
            ));
        }
        self.distribution
            .validate()
            .map_err(|error| DistributionBuildError::InvalidReceipt(error.to_string()))
    }
}

/// Idempotent apply result. Reuse always includes a fresh image inspection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentDistributionApplyOutcome {
    pub reused: bool,
    pub receipt: AgentDistributionBuildReceipt,
}

/// Explicit authority for one exact supply plan.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DistributionBuildAuthorization {
    confirmed_plan_id: Option<String>,
}

impl DistributionBuildAuthorization {
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn confirmed(plan_id: impl Into<String>) -> Self {
        Self {
            confirmed_plan_id: Some(plan_id.into()),
        }
    }
}

/// Create-only host inventory for verified distribution build receipts.
#[derive(Clone, Debug)]
pub struct AgentDistributionInventory {
    root: PathBuf,
}

impl AgentDistributionInventory {
    /// Open or create the exact inventory root.
    ///
    /// # Errors
    ///
    /// Returns [`DistributionBuildError`] when the directory is unavailable.
    pub fn open(root: &Path) -> Result<Self, DistributionBuildError> {
        fs::create_dir_all(root).map_err(|error| io_error(root, error))?;
        Ok(Self {
            root: fs::canonicalize(root).map_err(|error| io_error(root, error))?,
        })
    }

    /// Load one exact receipt by immutable distribution identity.
    ///
    /// # Errors
    ///
    /// Returns [`DistributionBuildError`] for absent, malformed or drifted
    /// inventory data.
    pub fn load(
        &self,
        distribution_id: &str,
    ) -> Result<AgentDistributionBuildReceipt, DistributionBuildError> {
        validate_id("distribution_id", distribution_id)?;
        let path = self.receipt_path(distribution_id);
        let mut file = File::open(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                DistributionBuildError::ReceiptNotFound(distribution_id.into())
            } else {
                io_error(&path, error)
            }
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|error| io_error(&path, error))?;
        let receipt: AgentDistributionBuildReceipt = serde_json::from_slice(&bytes)?;
        receipt.validate()?;
        if receipt.distribution.distribution_id != distribution_id {
            return Err(DistributionBuildError::InvalidReceipt(format!(
                "receipt distribution {} does not match inventory key {distribution_id}",
                receipt.distribution.distribution_id
            )));
        }
        Ok(receipt)
    }

    /// Return every valid receipt in stable distribution-id order.
    ///
    /// # Errors
    ///
    /// Fails closed on malformed entries instead of silently making them
    /// launchable or hiding inventory drift.
    pub fn list(&self) -> Result<Vec<AgentDistributionBuildReceipt>, DistributionBuildError> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| io_error(&self.root, error))? {
            let entry = entry.map_err(|error| io_error(&self.root, error))?;
            let file_type = entry
                .file_type()
                .map_err(|error| io_error(&entry.path(), error))?;
            if !file_type.is_dir() || file_type.is_symlink() {
                return Err(DistributionBuildError::InvalidReceipt(format!(
                    "unexpected entry in distribution inventory: {}",
                    entry.path().display()
                )));
            }
            let id = entry.file_name().into_string().map_err(|_| {
                DistributionBuildError::InvalidReceipt(
                    "distribution inventory id is not UTF-8".into(),
                )
            })?;
            validate_id("distribution_id", &id)?;
            ids.push(id);
        }
        ids.sort();
        ids.into_iter().map(|id| self.load(&id)).collect()
    }

    fn record(
        &self,
        receipt: &AgentDistributionBuildReceipt,
    ) -> Result<AgentDistributionBuildReceipt, DistributionBuildError> {
        receipt.validate()?;
        let id = &receipt.distribution.distribution_id;
        let directory = self.root.join(id);
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = self.load(id)?;
                return if existing == *receipt {
                    Ok(existing)
                } else {
                    Err(DistributionBuildError::InventoryConflict(id.clone()))
                };
            }
            Err(error) => return Err(io_error(&directory, error)),
        }
        let temporary = directory.join(format!("receipt.{}.tmp", std::process::id()));
        let target = directory.join("receipt.json");
        let write_result = (|| {
            let bytes = serde_json::to_vec_pretty(receipt)?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|error| io_error(&temporary, error))?;
            file.write_all(&bytes)
                .map_err(|error| io_error(&temporary, error))?;
            file.sync_all()
                .map_err(|error| io_error(&temporary, error))?;
            fs::rename(&temporary, &target).map_err(|error| io_error(&target, error))?;
            Ok(receipt.clone())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
            let _ = fs::remove_dir(&directory);
        }
        write_result
    }

    fn receipt_path(&self, distribution_id: &str) -> PathBuf {
        self.root.join(distribution_id).join("receipt.json")
    }
}

/// Produce a read-only, hash-bound build preview.
///
/// # Errors
///
/// Returns [`DistributionBuildError`] for an unready endpoint, undeclared or
/// unsafe context input, lock/integrity mismatch, or unsupported Containerfile.
pub fn agent_distribution_build_plan(
    probe: &BackendProbe,
    spec: &AgentDistributionBuildSpec,
) -> Result<AgentDistributionBuildPlan, DistributionBuildError> {
    agent_distribution_build_plan_with(probe, spec, &SystemDistributionCommandRunner)
}

fn agent_distribution_build_plan_with(
    probe: &BackendProbe,
    spec: &AgentDistributionBuildSpec,
    runner: &dyn DistributionCommandRunner,
) -> Result<AgentDistributionBuildPlan, DistributionBuildError> {
    if probe.backend_id != "podman" || probe.status != BackendProbeStatus::Ready {
        return Err(DistributionBuildError::BackendNotReady);
    }
    let executable = probe
        .executable
        .clone()
        .ok_or(DistributionBuildError::BackendNotReady)?;
    let builder_version = probe
        .version
        .clone()
        .ok_or_else(|| DistributionBuildError::InvalidSpec("builder version is missing".into()))?;
    validate_spec_scalars(spec)?;
    let context_root = canonical_directory("context_root", &spec.context_root)?;
    let context_files = collect_context_inputs(&context_root, &spec.context_files)?;
    require_declared_file(&context_files, &spec.containerfile, "containerfile")?;
    require_declared_file(&context_files, &spec.lockfile, "lockfile")?;
    let containerfile_bytes = read_declared_file(&context_root, &spec.containerfile)?;
    let base_images = parse_pinned_base_images(&containerfile_bytes)?;
    validate_npm_lock(spec, &context_root)?;
    let lockfile_sha256 = file_digest(&context_root.join(&spec.lockfile))?;
    let context_sha256 = context_digest(&context_files);
    let mut blockers = Vec::new();
    for image in &base_images {
        let output = runner.output(
            &executable,
            &prefixed_args(
                &probe.command_prefix,
                &["image".into(), "exists".into(), image.clone()],
            ),
        )?;
        match output.code {
            Some(0) => {}
            Some(1) => blockers.push(format!(
                "base image {image} is absent; acquire it with a separate image bootstrap plan"
            )),
            code => blockers.push(format!(
                "base image probe for {image} returned unexpected exit code {code:?}"
            )),
        }
    }
    let plan_id = build_plan_identity(
        probe,
        spec,
        &context_root,
        &context_files,
        &context_sha256,
        &lockfile_sha256,
        &base_images,
        &builder_version,
    );
    let image_tag = format!(
        "localhost/swem/{}:{}",
        spec.distribution_id,
        &plan_id["sha256:".len().."sha256:".len() + 16]
    );
    let staged_context = Path::new("<staged-context>");
    let command_preview = build_command_args(
        &probe.command_prefix,
        &plan_id,
        &context_sha256,
        &spec.distribution_id,
        &spec.platform,
        spec.network,
        &image_tag,
        staged_context,
        &staged_context.join(&spec.containerfile),
        Path::new("<staged-context>/image-id"),
    );
    Ok(AgentDistributionBuildPlan {
        schema: AGENT_DISTRIBUTION_BUILD_PLAN_SCHEMA.into(),
        plan_id,
        backend_executable: executable,
        backend_endpoint_id: probe.endpoint_id.clone(),
        backend_command_prefix: probe.command_prefix.clone(),
        builder_version,
        distribution_id: spec.distribution_id.clone(),
        registry_id: spec.registry_id.clone(),
        publisher: spec.publisher.clone(),
        package: spec.package.clone(),
        version: spec.version.clone(),
        package_integrity: spec.package_integrity.clone(),
        platform: spec.platform.clone(),
        context_root,
        containerfile: spec.containerfile.clone(),
        lockfile: spec.lockfile.clone(),
        context_files,
        context_sha256,
        lockfile_sha256,
        base_images,
        image_tag,
        agent_executable: spec.agent_executable.clone(),
        agent_args: spec.agent_args.clone(),
        network: spec.network,
        disposition: spec.disposition,
        command_preview,
        blockers,
    })
}

/// Apply one unchanged plan and atomically record its inspected image receipt.
///
/// # Errors
///
/// Returns [`DistributionBuildError`] when confirmation is absent, inputs or
/// plan identity changed, Podman fails, inspection disagrees, or inventory
/// contains another receipt for the same immutable distribution id.
pub fn apply_agent_distribution_build_plan(
    plan: &AgentDistributionBuildPlan,
    authorization: &DistributionBuildAuthorization,
    inventory: &AgentDistributionInventory,
) -> Result<AgentDistributionApplyOutcome, DistributionBuildError> {
    apply_agent_distribution_build_plan_with(
        plan,
        authorization,
        inventory,
        &SystemDistributionCommandRunner,
    )
}

fn apply_agent_distribution_build_plan_with(
    plan: &AgentDistributionBuildPlan,
    authorization: &DistributionBuildAuthorization,
    inventory: &AgentDistributionInventory,
    runner: &dyn DistributionCommandRunner,
) -> Result<AgentDistributionApplyOutcome, DistributionBuildError> {
    validate_plan(plan)?;
    if !plan.blockers.is_empty() {
        return Err(DistributionBuildError::Blocked(plan.blockers.clone()));
    }
    match plan.disposition {
        ProvisioningDisposition::Never => return Err(DistributionBuildError::Disabled),
        ProvisioningDisposition::Confirm => {
            if authorization.confirmed_plan_id.as_deref() != Some(plan.plan_id.as_str()) {
                return Err(DistributionBuildError::ConfirmationRequired(
                    plan.plan_id.clone(),
                ));
            }
        }
        ProvisioningDisposition::Managed => {
            return Err(DistributionBuildError::ManagedPolicyRequired);
        }
    }
    revalidate_context(plan)?;
    match inventory.load(&plan.distribution_id) {
        Ok(existing) => {
            if existing.plan_id != plan.plan_id || existing.context_sha256 != plan.context_sha256 {
                return Err(DistributionBuildError::InventoryConflict(
                    plan.distribution_id.clone(),
                ));
            }
            inspect_receipt(plan, &existing, runner)?;
            return Ok(AgentDistributionApplyOutcome {
                reused: true,
                receipt: existing,
            });
        }
        Err(DistributionBuildError::ReceiptNotFound(_)) => {}
        Err(error) => return Err(error),
    }

    let staging = StagedContext::create(plan)?;
    let containerfile = staging.root.join(&plan.containerfile);
    let iidfile = staging.root.join(".swem-image-id");
    let args = build_command_args(
        &plan.backend_command_prefix,
        &plan.plan_id,
        &plan.context_sha256,
        &plan.distribution_id,
        &plan.platform,
        plan.network,
        &plan.image_tag,
        &staging.root,
        &containerfile,
        &iidfile,
    );
    let output = runner.output(&plan.backend_executable, &args)?;
    if output.code != Some(0) {
        return Err(DistributionBuildError::CommandFailed(format!(
            "podman build failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let resolved_image = fs::read_to_string(&iidfile)
        .map_err(|error| io_error(&iidfile, error))?
        .trim()
        .to_owned();
    validate_sha256("built image id", &resolved_image)?;
    let inspection = inspect_image(plan, &resolved_image, runner)?;
    let receipt = AgentDistributionBuildReceipt {
        schema: AGENT_DISTRIBUTION_BUILD_RECEIPT_SCHEMA.into(),
        plan_id: plan.plan_id.clone(),
        context_sha256: plan.context_sha256.clone(),
        backend_endpoint_id: plan.backend_endpoint_id.clone(),
        builder_version: plan.builder_version.clone(),
        image_size_bytes: inspection.size_bytes,
        distribution: AgentDistributionReceipt {
            schema: AGENT_DISTRIBUTION_RECEIPT_SCHEMA.into(),
            distribution_id: plan.distribution_id.clone(),
            registry_id: plan.registry_id.clone(),
            publisher: plan.publisher.clone(),
            package: plan.package.clone(),
            version: plan.version.clone(),
            package_integrity: plan.package_integrity.clone(),
            lockfile_sha256: plan.lockfile_sha256.clone(),
            platform: plan.platform.clone(),
            resolved_image,
            agent_executable: plan.agent_executable.clone(),
            agent_args: plan.agent_args.clone(),
        },
    };
    let receipt = inventory.record(&receipt)?;
    Ok(AgentDistributionApplyOutcome {
        reused: false,
        receipt,
    })
}

fn inspect_receipt(
    plan: &AgentDistributionBuildPlan,
    receipt: &AgentDistributionBuildReceipt,
    runner: &dyn DistributionCommandRunner,
) -> Result<(), DistributionBuildError> {
    receipt.validate()?;
    let inspection = inspect_image(plan, &receipt.distribution.resolved_image, runner)?;
    if inspection.size_bytes != receipt.image_size_bytes {
        return Err(DistributionBuildError::InspectionFailed(
            "image size differs from inventory receipt".into(),
        ));
    }
    Ok(())
}

struct ImageInspection {
    size_bytes: u64,
}

fn inspect_image(
    plan: &AgentDistributionBuildPlan,
    resolved_image: &str,
    runner: &dyn DistributionCommandRunner,
) -> Result<ImageInspection, DistributionBuildError> {
    let output = runner.output(
        &plan.backend_executable,
        &prefixed_args(
            &plan.backend_command_prefix,
            &["image".into(), "inspect".into(), resolved_image.into()],
        ),
    )?;
    if output.code != Some(0) {
        return Err(DistributionBuildError::CommandFailed(format!(
            "podman image inspect failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let image = value
        .as_array()
        .filter(|images| images.len() == 1)
        .and_then(|images| images.first())
        .ok_or_else(|| {
            DistributionBuildError::InspectionFailed("inspect must return exactly one image".into())
        })?;
    let raw_id = image
        .get("Id")
        .or_else(|| image.get("ID"))
        .and_then(Value::as_str)
        .ok_or_else(|| DistributionBuildError::InspectionFailed("image id missing".into()))?;
    let normalized_id = if raw_id.starts_with("sha256:") {
        raw_id.to_owned()
    } else {
        format!("sha256:{raw_id}")
    };
    if normalized_id != resolved_image {
        return Err(DistributionBuildError::InspectionFailed(
            "inspected image id differs from iidfile/receipt".into(),
        ));
    }
    let (os, architecture) = plan
        .platform
        .split_once('/')
        .ok_or_else(|| DistributionBuildError::InvalidPlan("platform is invalid".into()))?;
    require_json_string(image, &["Os", "OS"], os, "image OS")?;
    require_json_string(image, &["Architecture"], architecture, "image architecture")?;
    let labels = image
        .get("Labels")
        .or_else(|| image.pointer("/Config/Labels"))
        .and_then(Value::as_object)
        .ok_or_else(|| DistributionBuildError::InspectionFailed("image labels missing".into()))?;
    for (key, expected) in [
        ("io.swem.distribution.plan", plan.plan_id.as_str()),
        ("io.swem.distribution.context", plan.context_sha256.as_str()),
        ("io.swem.distribution.id", plan.distribution_id.as_str()),
    ] {
        if labels.get(key).and_then(Value::as_str) != Some(expected) {
            return Err(DistributionBuildError::InspectionFailed(format!(
                "image label {key} does not match the confirmed plan"
            )));
        }
    }
    let size_bytes = image
        .get("Size")
        .and_then(Value::as_u64)
        .ok_or_else(|| DistributionBuildError::InspectionFailed("image size missing".into()))?;
    if size_bytes == 0 {
        return Err(DistributionBuildError::InspectionFailed(
            "image size must be positive".into(),
        ));
    }
    Ok(ImageInspection { size_bytes })
}

fn validate_plan(plan: &AgentDistributionBuildPlan) -> Result<(), DistributionBuildError> {
    if plan.schema != AGENT_DISTRIBUTION_BUILD_PLAN_SCHEMA {
        return Err(DistributionBuildError::InvalidPlan(format!(
            "unsupported schema {}",
            plan.schema
        )));
    }
    validate_sha256("plan_id", &plan.plan_id)?;
    validate_sha256("context_sha256", &plan.context_sha256)?;
    validate_sha256("lockfile_sha256", &plan.lockfile_sha256)?;
    let expected_tag = format!(
        "localhost/swem/{}:{}",
        plan.distribution_id,
        &plan.plan_id["sha256:".len().."sha256:".len() + 16]
    );
    if plan.image_tag != expected_tag {
        return Err(DistributionBuildError::InvalidPlan(
            "image tag does not derive from plan identity".into(),
        ));
    }
    let staged_context = Path::new("<staged-context>");
    let preview = build_command_args(
        &plan.backend_command_prefix,
        &plan.plan_id,
        &plan.context_sha256,
        &plan.distribution_id,
        &plan.platform,
        plan.network,
        &plan.image_tag,
        staged_context,
        &staged_context.join(&plan.containerfile),
        Path::new("<staged-context>/image-id"),
    );
    if preview != plan.command_preview {
        return Err(DistributionBuildError::InvalidPlan(
            "command preview was modified".into(),
        ));
    }
    let identity = build_plan_identity_from_plan(plan);
    if identity != plan.plan_id {
        return Err(DistributionBuildError::PlanIdentityMismatch);
    }
    Ok(())
}

fn revalidate_context(plan: &AgentDistributionBuildPlan) -> Result<(), DistributionBuildError> {
    let current = collect_context_inputs(
        &plan.context_root,
        &plan
            .context_files
            .iter()
            .map(|input| input.relative_path.clone())
            .collect::<Vec<_>>(),
    )?;
    if current != plan.context_files || context_digest(&current) != plan.context_sha256 {
        return Err(DistributionBuildError::ContextChanged);
    }
    if file_digest(&plan.context_root.join(&plan.lockfile))? != plan.lockfile_sha256 {
        return Err(DistributionBuildError::ContextChanged);
    }
    Ok(())
}

fn validate_spec_scalars(spec: &AgentDistributionBuildSpec) -> Result<(), DistributionBuildError> {
    validate_id("distribution_id", &spec.distribution_id)?;
    for (field, value) in [
        ("registry_id", spec.registry_id.as_str()),
        ("publisher", spec.publisher.as_str()),
        ("package", spec.package.as_str()),
        ("version", spec.version.as_str()),
        ("agent_executable", spec.agent_executable.as_str()),
    ] {
        require_non_empty(field, value)?;
    }
    if !spec.package_integrity.starts_with("sha512-") {
        return Err(DistributionBuildError::InvalidSpec(
            "package_integrity must be an npm sha512 SRI value".into(),
        ));
    }
    validate_platform(&spec.platform)?;
    if !spec.agent_executable.starts_with('/') {
        return Err(DistributionBuildError::InvalidSpec(
            "agent_executable must be an absolute agent-visible path".into(),
        ));
    }
    validate_relative_path(&spec.containerfile)?;
    validate_relative_path(&spec.lockfile)
}

fn validate_npm_lock(
    spec: &AgentDistributionBuildSpec,
    context_root: &Path,
) -> Result<(), DistributionBuildError> {
    let path = context_root.join(&spec.lockfile);
    let value: Value =
        serde_json::from_slice(&fs::read(&path).map_err(|error| io_error(&path, error))?)?;
    let package_key = format!("node_modules/{}", spec.package);
    let package = value
        .pointer(&format!("/packages/{}", escape_json_pointer(&package_key)))
        .ok_or_else(|| {
            DistributionBuildError::InvalidSpec(format!(
                "lockfile has no exact package entry {package_key}"
            ))
        })?;
    if package.get("version").and_then(Value::as_str) != Some(spec.version.as_str())
        || package.get("integrity").and_then(Value::as_str) != Some(spec.package_integrity.as_str())
    {
        return Err(DistributionBuildError::InvalidSpec(
            "lockfile package version/integrity does not match distribution spec".into(),
        ));
    }
    let root_dependency = value
        .pointer(&format!(
            "/packages//dependencies/{}",
            escape_json_pointer(&spec.package)
        ))
        .and_then(Value::as_str);
    if root_dependency != Some(spec.version.as_str()) {
        return Err(DistributionBuildError::InvalidSpec(
            "lockfile root dependency is not pinned to the exact adapter version".into(),
        ));
    }
    Ok(())
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn collect_context_inputs(
    context_root: &Path,
    relative_paths: &[PathBuf],
) -> Result<Vec<DistributionBuildInput>, DistributionBuildError> {
    if relative_paths.is_empty() {
        return Err(DistributionBuildError::InvalidSpec(
            "at least one build context file must be declared".into(),
        ));
    }
    let canonical_root = canonical_directory("context_root", context_root)?;
    let mut seen = BTreeSet::new();
    let mut inputs = Vec::with_capacity(relative_paths.len());
    for relative in relative_paths {
        validate_relative_path(relative)?;
        if !seen.insert(relative.clone()) {
            return Err(DistributionBuildError::InvalidSpec(format!(
                "duplicate context path {}",
                relative.display()
            )));
        }
        let source = canonical_root.join(relative);
        let metadata = fs::symlink_metadata(&source).map_err(|error| io_error(&source, error))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(DistributionBuildError::InvalidSpec(format!(
                "context input must be a regular non-symlink file: {}",
                relative.display()
            )));
        }
        let canonical_source =
            fs::canonicalize(&source).map_err(|error| io_error(&source, error))?;
        if !canonical_source.starts_with(&canonical_root) {
            return Err(DistributionBuildError::InvalidSpec(format!(
                "context input escapes root: {}",
                relative.display()
            )));
        }
        inputs.push(DistributionBuildInput {
            relative_path: relative.clone(),
            sha256: file_digest(&canonical_source)?,
            size_bytes: metadata.len(),
        });
    }
    inputs.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(inputs)
}

fn require_declared_file(
    inputs: &[DistributionBuildInput],
    path: &Path,
    kind: &str,
) -> Result<(), DistributionBuildError> {
    if inputs.iter().any(|input| input.relative_path == path) {
        Ok(())
    } else {
        Err(DistributionBuildError::InvalidSpec(format!(
            "{kind} {} is not in declared context files",
            path.display()
        )))
    }
}

fn read_declared_file(
    context_root: &Path,
    relative: &Path,
) -> Result<Vec<u8>, DistributionBuildError> {
    let path = context_root.join(relative);
    fs::read(&path).map_err(|error| io_error(&path, error))
}

fn parse_pinned_base_images(bytes: &[u8]) -> Result<Vec<String>, DistributionBuildError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|error| DistributionBuildError::InvalidSpec(error.to_string()))?;
    let mut images = Vec::new();
    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        if !parts
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("FROM"))
        {
            continue;
        }
        let mut image = parts.next().ok_or_else(|| {
            DistributionBuildError::InvalidSpec("Containerfile FROM lacks image".into())
        })?;
        if image.starts_with("--platform=") {
            image = parts.next().ok_or_else(|| {
                DistributionBuildError::InvalidSpec("Containerfile FROM lacks image".into())
            })?;
        }
        if image.eq_ignore_ascii_case("scratch") {
            continue;
        }
        validate_fully_qualified_digest(image)?;
        images.push(image.to_owned());
    }
    if images.is_empty() {
        return Err(DistributionBuildError::InvalidSpec(
            "Containerfile must declare at least one digest-pinned non-scratch base image".into(),
        ));
    }
    images.sort();
    images.dedup();
    Ok(images)
}

fn validate_fully_qualified_digest(reference: &str) -> Result<(), DistributionBuildError> {
    let Some((name, digest)) = reference.rsplit_once("@sha256:") else {
        return Err(DistributionBuildError::InvalidSpec(format!(
            "base image must be digest-pinned: {reference}"
        )));
    };
    let registry = name.split('/').next().unwrap_or_default();
    if !name.contains('/')
        || !(registry.contains('.') || registry.contains(':') || registry == "localhost")
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(DistributionBuildError::InvalidSpec(format!(
            "base image must be fully-qualified and sha256-pinned: {reference}"
        )));
    }
    Ok(())
}

fn context_digest(inputs: &[DistributionBuildInput]) -> String {
    let bytes = serde_json::to_vec(inputs).expect("serializable context inputs");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[derive(Serialize)]
struct DistributionBuildIdentity<'a> {
    schema: &'static str,
    backend_executable: Option<&'a Path>,
    backend_endpoint_id: &'a str,
    backend_command_prefix: &'a [String],
    builder_version: &'a str,
    distribution_id: &'a str,
    registry_id: &'a str,
    publisher: &'a str,
    package: &'a str,
    version: &'a str,
    package_integrity: &'a str,
    platform: &'a str,
    context_root: &'a Path,
    containerfile: &'a Path,
    lockfile: &'a Path,
    context_files: &'a [DistributionBuildInput],
    context_sha256: &'a str,
    lockfile_sha256: &'a str,
    base_images: &'a [String],
    agent_executable: &'a str,
    agent_args: &'a [String],
    network: DistributionBuildNetwork,
    disposition: ProvisioningDisposition,
}

#[allow(clippy::too_many_arguments)]
fn build_plan_identity(
    probe: &BackendProbe,
    spec: &AgentDistributionBuildSpec,
    context_root: &Path,
    context_files: &[DistributionBuildInput],
    context_sha256: &str,
    lockfile_sha256: &str,
    base_images: &[String],
    builder_version: &str,
) -> String {
    let bytes = serde_json::to_vec(&DistributionBuildIdentity {
        schema: AGENT_DISTRIBUTION_BUILD_PLAN_SCHEMA,
        backend_executable: probe.executable.as_deref(),
        backend_endpoint_id: &probe.endpoint_id,
        backend_command_prefix: &probe.command_prefix,
        builder_version,
        distribution_id: &spec.distribution_id,
        registry_id: &spec.registry_id,
        publisher: &spec.publisher,
        package: &spec.package,
        version: &spec.version,
        package_integrity: &spec.package_integrity,
        platform: &spec.platform,
        context_root,
        containerfile: &spec.containerfile,
        lockfile: &spec.lockfile,
        context_files,
        context_sha256,
        lockfile_sha256,
        base_images,
        agent_executable: &spec.agent_executable,
        agent_args: &spec.agent_args,
        network: spec.network,
        disposition: spec.disposition,
    })
    .expect("serializable distribution build identity");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn build_plan_identity_from_plan(plan: &AgentDistributionBuildPlan) -> String {
    let bytes = serde_json::to_vec(&DistributionBuildIdentity {
        schema: AGENT_DISTRIBUTION_BUILD_PLAN_SCHEMA,
        backend_executable: Some(&plan.backend_executable),
        backend_endpoint_id: &plan.backend_endpoint_id,
        backend_command_prefix: &plan.backend_command_prefix,
        builder_version: &plan.builder_version,
        distribution_id: &plan.distribution_id,
        registry_id: &plan.registry_id,
        publisher: &plan.publisher,
        package: &plan.package,
        version: &plan.version,
        package_integrity: &plan.package_integrity,
        platform: &plan.platform,
        context_root: &plan.context_root,
        containerfile: &plan.containerfile,
        lockfile: &plan.lockfile,
        context_files: &plan.context_files,
        context_sha256: &plan.context_sha256,
        lockfile_sha256: &plan.lockfile_sha256,
        base_images: &plan.base_images,
        agent_executable: &plan.agent_executable,
        agent_args: &plan.agent_args,
        network: plan.network,
        disposition: plan.disposition,
    })
    .expect("serializable distribution build identity");
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[allow(clippy::too_many_arguments)]
fn build_command_args(
    prefix: &[String],
    plan_id: &str,
    context_sha256: &str,
    distribution_id: &str,
    platform: &str,
    network: DistributionBuildNetwork,
    image_tag: &str,
    context: &Path,
    containerfile: &Path,
    iidfile: &Path,
) -> Vec<String> {
    prefixed_args(
        prefix,
        &[
            "build".into(),
            "--pull=never".into(),
            "--format=oci".into(),
            "--platform".into(),
            platform.into(),
            "--network".into(),
            network.podman_mode().into(),
            "--label".into(),
            format!("io.swem.distribution.plan={plan_id}"),
            "--label".into(),
            format!("io.swem.distribution.context={context_sha256}"),
            "--label".into(),
            format!("io.swem.distribution.id={distribution_id}"),
            "--tag".into(),
            image_tag.into(),
            "--iidfile".into(),
            iidfile.display().to_string(),
            "--file".into(),
            containerfile.display().to_string(),
            context.display().to_string(),
        ],
    )
}

fn prefixed_args(prefix: &[String], suffix: &[String]) -> Vec<String> {
    prefix.iter().chain(suffix).cloned().collect()
}

struct StagedContext {
    root: PathBuf,
}

impl StagedContext {
    fn create(plan: &AgentDistributionBuildPlan) -> Result<Self, DistributionBuildError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| DistributionBuildError::InvalidPlan(error.to_string()))?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "swem-distribution-build-{}-{}-{nonce}",
            plan.distribution_id,
            std::process::id()
        ));
        fs::create_dir(&root).map_err(|error| io_error(&root, error))?;
        for input in &plan.context_files {
            let source = plan.context_root.join(&input.relative_path);
            let target = root.join(&input.relative_path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|error| io_error(parent, error))?;
            }
            fs::copy(&source, &target).map_err(|error| io_error(&target, error))?;
        }
        Ok(Self { root })
    }
}

impl Drop for StagedContext {
    fn drop(&mut self) {
        if self.root.starts_with(std::env::temp_dir())
            && self.root.file_name().is_some_and(|name| {
                name.to_string_lossy()
                    .starts_with("swem-distribution-build-")
            })
        {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

fn canonical_directory(field: &str, path: &Path) -> Result<PathBuf, DistributionBuildError> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(DistributionBuildError::InvalidSpec(format!(
            "{field} must be an existing absolute directory: {}",
            path.display()
        )));
    }
    fs::canonicalize(path).map_err(|error| io_error(path, error))
}

fn validate_relative_path(path: &Path) -> Result<(), DistributionBuildError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(DistributionBuildError::InvalidSpec(format!(
            "build context path must be a normalized relative path: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_platform(platform: &str) -> Result<(), DistributionBuildError> {
    let Some((os, architecture)) = platform.split_once('/') else {
        return Err(DistributionBuildError::InvalidSpec(
            "platform must be linux/<architecture>".into(),
        ));
    };
    if os != "linux"
        || architecture.is_empty()
        || !architecture
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(DistributionBuildError::InvalidSpec(
            "platform must be an explicit Linux OS/architecture pair".into(),
        ));
    }
    Ok(())
}

fn validate_id(field: &str, value: &str) -> Result<(), DistributionBuildError> {
    require_non_empty(field, value)?;
    if value.len() > 128
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(DistributionBuildError::InvalidSpec(format!(
            "{field} must use 1..128 ASCII letters, digits, dot, dash or underscore"
        )));
    }
    Ok(())
}

fn require_non_empty(field: &str, value: &str) -> Result<(), DistributionBuildError> {
    if value.trim().is_empty() {
        Err(DistributionBuildError::InvalidSpec(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn validate_sha256(field: &str, value: &str) -> Result<(), DistributionBuildError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(DistributionBuildError::InvalidReceipt(format!(
            "{field} must be a sha256 digest"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DistributionBuildError::InvalidReceipt(format!(
            "{field} must contain 64 hexadecimal digits"
        )));
    }
    Ok(())
}

fn file_digest(path: &Path) -> Result<String, DistributionBuildError> {
    let bytes = fs::read(path).map_err(|error| io_error(path, error))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn require_json_string(
    value: &Value,
    keys: &[&str],
    expected: &str,
    field: &str,
) -> Result<(), DistributionBuildError> {
    let observed = keys
        .iter()
        .find_map(|key| value.get(*key))
        .and_then(Value::as_str);
    if observed == Some(expected) {
        Ok(())
    } else {
        Err(DistributionBuildError::InspectionFailed(format!(
            "{field} was {observed:?}, expected {expected}"
        )))
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err closures transfer each concrete error into this uniform constructor"
)]
fn io_error(path: &Path, error: impl ToString) -> DistributionBuildError {
    DistributionBuildError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    }
}

struct DistributionCommandOutput {
    code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait DistributionCommandRunner {
    fn output(
        &self,
        executable: &Path,
        args: &[String],
    ) -> Result<DistributionCommandOutput, DistributionBuildError>;
}

struct SystemDistributionCommandRunner;

impl DistributionCommandRunner for SystemDistributionCommandRunner {
    fn output(
        &self,
        executable: &Path,
        args: &[String],
    ) -> Result<DistributionCommandOutput, DistributionBuildError> {
        let output = Command::new(executable)
            .args(args)
            .output()
            .map_err(|error| DistributionBuildError::CommandFailed(error.to_string()))?;
        Ok(DistributionCommandOutput {
            code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[derive(Debug, Error)]
pub enum DistributionBuildError {
    #[error("distribution build backend is not a ready Podman endpoint")]
    BackendNotReady,
    #[error("invalid distribution build specification: {0}")]
    InvalidSpec(String),
    #[error("invalid distribution build plan: {0}")]
    InvalidPlan(String),
    #[error("distribution build plan identity changed")]
    PlanIdentityMismatch,
    #[error("distribution build context changed after preview")]
    ContextChanged,
    #[error("distribution build is blocked: {0:?}")]
    Blocked(Vec<String>),
    #[error("distribution build is disabled by host policy")]
    Disabled,
    #[error("distribution build needs confirmation of plan {0}")]
    ConfirmationRequired(String),
    #[error("managed distribution build needs a separate host-owned policy")]
    ManagedPolicyRequired,
    #[error("distribution build command failed: {0}")]
    CommandFailed(String),
    #[error("distribution image inspection failed: {0}")]
    InspectionFailed(String),
    #[error("invalid distribution receipt: {0}")]
    InvalidReceipt(String),
    #[error("distribution receipt not found: {0}")]
    ReceiptNotFound(String),
    #[error("distribution inventory already contains a different receipt: {0}")]
    InventoryConflict(String),
    #[error("distribution inventory I/O failed at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("distribution JSON failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

    struct MockRunner {
        outputs: Mutex<VecDeque<DistributionCommandOutput>>,
    }

    impl MockRunner {
        fn new(outputs: Vec<DistributionCommandOutput>) -> Self {
            Self {
                outputs: Mutex::new(outputs.into()),
            }
        }
    }

    impl DistributionCommandRunner for MockRunner {
        fn output(
            &self,
            _executable: &Path,
            _args: &[String],
        ) -> Result<DistributionCommandOutput, DistributionBuildError> {
            self.outputs
                .lock()
                .expect("mock runner lock")
                .pop_front()
                .ok_or_else(|| DistributionBuildError::CommandFailed("unexpected command".into()))
        }
    }

    #[test]
    fn base_parser_rejects_tag_and_accepts_fully_pinned_images() {
        assert!(parse_pinned_base_images(b"FROM node:22\n").is_err());
        let digest = "a".repeat(64);
        let parsed = parse_pinned_base_images(
            format!("FROM docker.io/library/node@sha256:{digest}\n").as_bytes(),
        )
        .expect("parse exact base");
        assert_eq!(parsed.len(), 1);
    }

    #[test]
    fn plan_reports_absent_base_without_mutating_backend() {
        let fixture = Fixture::new();
        let spec = fixture.spec();
        let runner = MockRunner::new(vec![DistributionCommandOutput {
            code: Some(1),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }]);
        let plan = agent_distribution_build_plan_with(&Fixture::probe(), &spec, &runner)
            .expect("build preview");
        assert_eq!(plan.blockers.len(), 1);
        assert!(plan.command_preview.iter().any(|arg| arg == "--pull=never"));
        assert_eq!(plan.context_files.len(), 3);
    }

    #[test]
    fn apply_requires_exact_confirmation_and_unchanged_context() {
        let fixture = Fixture::new();
        let spec = fixture.spec();
        let runner = MockRunner::new(vec![DistributionCommandOutput {
            code: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }]);
        let plan = agent_distribution_build_plan_with(&Fixture::probe(), &spec, &runner)
            .expect("build unblocked preview");
        let inventory = AgentDistributionInventory::open(&fixture.root.join("inventory"))
            .expect("open exact inventory");
        assert!(matches!(
            apply_agent_distribution_build_plan_with(
                &plan,
                &DistributionBuildAuthorization::none(),
                &inventory,
                &runner,
            ),
            Err(DistributionBuildError::ConfirmationRequired(id)) if id == plan.plan_id
        ));

        fs::write(
            fixture.root.join("package.json"),
            r#"{"dependencies":{"@scope/agent":"9.9.9"}}"#,
        )
        .expect("mutate planned context");
        assert!(matches!(
            apply_agent_distribution_build_plan_with(
                &plan,
                &DistributionBuildAuthorization::confirmed(plan.plan_id.clone()),
                &inventory,
                &runner,
            ),
            Err(DistributionBuildError::ContextChanged)
        ));
    }

    #[test]
    fn apply_fails_closed_on_corrupt_existing_inventory() {
        let fixture = Fixture::new();
        let spec = fixture.spec();
        let runner = MockRunner::new(vec![DistributionCommandOutput {
            code: Some(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        }]);
        let plan = agent_distribution_build_plan_with(&Fixture::probe(), &spec, &runner)
            .expect("build unblocked preview");
        let inventory_root = fixture.root.join("inventory");
        let inventory =
            AgentDistributionInventory::open(&inventory_root).expect("open exact inventory");
        let corrupt_directory = inventory_root.join(&plan.distribution_id);
        fs::create_dir(&corrupt_directory).expect("create corrupt inventory key");
        fs::write(corrupt_directory.join("receipt.json"), b"not-json")
            .expect("write corrupt receipt");

        assert!(matches!(
            apply_agent_distribution_build_plan_with(
                &plan,
                &DistributionBuildAuthorization::confirmed(plan.plan_id.clone()),
                &inventory,
                &runner,
            ),
            Err(DistributionBuildError::Serialization(_))
        ));
    }

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "swem-distribution-unit-{}-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos(),
                NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("create fixture");
            let digest = "a".repeat(64);
            fs::write(
                root.join("Containerfile"),
                format!("FROM docker.io/library/node@sha256:{digest}\n"),
            )
            .expect("write Containerfile");
            fs::write(
                root.join("package.json"),
                r#"{"dependencies":{"@scope/agent":"1.2.3"}}"#,
            )
            .expect("write manifest");
            fs::write(
                root.join("package-lock.json"),
                r#"{"packages":{"":{"dependencies":{"@scope/agent":"1.2.3"}},"node_modules/@scope/agent":{"version":"1.2.3","integrity":"sha512-fixture"}}}"#,
            )
            .expect("write lock");
            Self { root }
        }

        fn probe() -> BackendProbe {
            BackendProbe {
                backend_id: "podman".into(),
                endpoint_id: "podman:test".into(),
                status: BackendProbeStatus::Ready,
                executable: Some(PathBuf::from("podman")),
                command_prefix: vec!["--connection".into(), "test".into()],
                version: Some("5.8.3".into()),
                topology: serde_json::json!({}),
                available_guarantees: BTreeSet::new(),
                limitations: vec![],
                diagnostics: vec![],
            }
        }

        fn spec(&self) -> AgentDistributionBuildSpec {
            AgentDistributionBuildSpec {
                distribution_id: "fixture-1.2.3".into(),
                registry_id: "fixture".into(),
                publisher: "fixture".into(),
                package: "@scope/agent".into(),
                version: "1.2.3".into(),
                package_integrity: "sha512-fixture".into(),
                platform: "linux/amd64".into(),
                context_root: self.root.canonicalize().expect("canonical fixture"),
                containerfile: "Containerfile".into(),
                lockfile: "package-lock.json".into(),
                context_files: vec![
                    "Containerfile".into(),
                    "package.json".into(),
                    "package-lock.json".into(),
                ],
                agent_executable: "/usr/bin/node".into(),
                agent_args: vec!["/agent/index.js".into()],
                network: DistributionBuildNetwork::PrivateEgress,
                disposition: ProvisioningDisposition::Confirm,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
