//! What the product supplies to the Workbench about the domain side, without
//! the host knowing the domain side.
//!
//! The Workbench shows what packages declare - the tools an adapter needs
//! installed, the kinds of secret a project may hold, the recipes a person can
//! run - and installs a package from a source. Reading a manifest and loading
//! a package are the Cycle's business (`swem-plugin`); the host used
//! to link the Cycle for exactly that and nothing else. Now the host holds a
//! [`ProductSupply`] the product root hands it, and the product root
//! (`swem-cli`, which links the Cycle) is the one that reads manifests. A host
//! without a supply - a test over an agent alone - lists nothing and refuses
//! an install, which is the truth of that host.
//!
//! Everything here is a plain value: no manifest type, no loader type, no
//! word of any domain. A test that scans the host for domain words
//! (`genericity.rs`) stays green by construction.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PackageHome, PackagePlan, PackageSource, PackageView, SupplyError};

/// One artifact of a declared tool, for one platform.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupplyArtifact {
    pub url: String,
    pub sha256: String,
    pub entry: String,
}

/// One tool a package declares its adapters need: exact archives per
/// platform, digested.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupplyTool {
    pub name: String,
    pub module: String,
    pub version: String,
    pub artifacts: BTreeMap<String, SupplyArtifact>,
}

/// One kind of secret a package declares a project may hold.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupplySecretType {
    pub type_id: String,
    pub module: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_var: Option<String>,
    #[serde(default)]
    pub hint: String,
}

/// One step of a recipe, as the person reads it: what it does and which of
/// the project's tools it calls. Its arguments are the package's business and
/// come from [`ProductSupply::recipe_step_arguments`] when the step runs.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupplyRecipeStep {
    pub note: String,
    pub tool: String,
}

/// One recipe a package brings: an ordered list of the project's own tool
/// calls.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SupplyRecipe {
    pub module: String,
    pub name: String,
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub starts_a_project: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_example: Option<String>,
    pub steps: Vec<SupplyRecipeStep>,
}

/// The product root's answers about packages. `packages_home` is where
/// this person's own packages are installed (`<data root>/plugins`), or
/// `None` on a host that creates no projects and so has no such place; the
/// packages the distribution ships are the supply's own knowledge and are
/// answered either way.
pub trait ProductSupply: Send + Sync {
    /// Every tool every package declares, shipped and installed alike.
    fn tools(&self, packages_home: Option<&Path>) -> Vec<SupplyTool>;
    /// Every kind of secret every package declares.
    fn secret_types(&self, packages_home: Option<&Path>) -> Vec<SupplySecretType>;
    /// Every recipe every package brings.
    fn recipes(&self, packages_home: Option<&Path>) -> Vec<SupplyRecipe>;
    /// The arguments of one step of one recipe, with references to earlier
    /// steps' results replaced. `step` counts from one.
    ///
    /// # Errors
    ///
    /// A sentence when the recipe or the step is unknown, or a reference
    /// names a result that is not there.
    fn recipe_step_arguments(
        &self,
        packages_home: Option<&Path>,
        recipe: &str,
        step: usize,
        results: &[Value],
    ) -> Result<Value, String>;
    /// Every package under one directory, whether or not it loads.
    fn package_views(&self, directory: &Path, home: PackageHome) -> Vec<PackageView>;
    /// Read a source and answer the plan installing it would consent to.
    ///
    /// # Errors
    ///
    /// The supply's own refusal: an unreadable source, a digest that does not
    /// match, a package the loader refuses.
    fn plan_package(
        &self,
        source: &PackageSource,
        packages_home: &Path,
    ) -> Result<PackagePlan, SupplyError>;
    /// Install the package staged under a plan.
    ///
    /// # Errors
    ///
    /// The supply's own refusal, a plan nothing is staged under among them.
    fn install_package(
        &self,
        plan_id: &str,
        packages_home: &Path,
    ) -> Result<PackageView, SupplyError>;
}

/// A host with no product root behind it: an agent harness alone. It
/// declares nothing and installs nothing, and says so.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoSupply;

impl ProductSupply for NoSupply {
    fn tools(&self, _packages_home: Option<&Path>) -> Vec<SupplyTool> {
        Vec::new()
    }

    fn secret_types(&self, _packages_home: Option<&Path>) -> Vec<SupplySecretType> {
        Vec::new()
    }

    fn recipes(&self, _packages_home: Option<&Path>) -> Vec<SupplyRecipe> {
        Vec::new()
    }

    fn recipe_step_arguments(
        &self,
        _packages_home: Option<&Path>,
        recipe: &str,
        _step: usize,
        _results: &[Value],
    ) -> Result<Value, String> {
        Err(format!(
            "this host runs no packages, so it knows no recipe {recipe}"
        ))
    }

    fn package_views(&self, _directory: &Path, _home: PackageHome) -> Vec<PackageView> {
        Vec::new()
    }

    fn plan_package(
        &self,
        _source: &PackageSource,
        _packages_home: &Path,
    ) -> Result<PackagePlan, SupplyError> {
        Err(SupplyError::Protocol(
            "this host runs no packages; start the product to install one".into(),
        ))
    }

    fn install_package(
        &self,
        _plan_id: &str,
        _packages_home: &Path,
    ) -> Result<PackageView, SupplyError> {
        Err(SupplyError::Protocol(
            "this host runs no packages; start the product to install one".into(),
        ))
    }
}
