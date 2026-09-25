//! The places a model is served from, which a person sets up once and every
//! profile draws on.
//!
//! A model provider is not a publication provider (`swem-core`'s
//! `swem.provider:*`, which is where a delivery goes) and not an environment
//! (where an agent runs): it is the endpoint an agent's model answers at, the
//! kind of key that opens it, and the models it is known to serve. The key
//! itself is never here - it stays in the profile's own secrets under the
//! kind this provider names, so a provider can be listed, copied and shown
//! without ever carrying a value.
//!
//! The product ships a few (`model-providers.json` beside `agent-secrets.json`,
//! each bound to a kind of key the secret catalogue already knows). A person
//! adds their own as a document under `<data root>/model-providers/<id>.json`,
//! and a document with a shipped id overrides the shipped entry - that is how
//! a person corrects a base URL or a model list without a new release.
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::*;

pub const MODEL_PROVIDER_SCHEMA: &str = "swem:model-provider@0.1";

/// Whether the product shipped this provider or a person declared it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelProviderOrigin {
    BuiltIn,
    #[default]
    Declared,
}

/// One model a provider is known to serve.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelChoice {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// A model provider, as stored.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelProvider {
    #[serde(default = "model_provider_schema")]
    pub schema: String,
    pub id: String,
    pub name: String,
    /// An OpenAI-compatible (or vendor) endpoint. Absent means the vendor's
    /// own default, which the agent knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// A `type_id` of the secret catalogue (`agent-secrets.json`): which kind
    /// of key opens this provider, and so which variable it is injected under.
    pub key_type: String,
    #[serde(default)]
    pub models: Vec<ModelChoice>,
    #[serde(default)]
    pub origin: ModelProviderOrigin,
}

fn model_provider_schema() -> String {
    MODEL_PROVIDER_SCHEMA.to_owned()
}

/// A provider as a person sees it: the record plus what its key kind means.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelProviderView {
    #[serde(flatten)]
    pub provider: ModelProvider,
    /// The variable the key is injected under, from the secret catalogue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_env: Option<String>,
    pub key_label: String,
}

/// What a person fills in to declare a provider.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeclareModelProviderBody {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    pub key_type: String,
    #[serde(default)]
    pub models: Vec<ModelChoice>,
}

/// The book of providers: the shipped ones, overlaid by the documents in one
/// directory.
#[derive(Clone, Debug)]
pub struct ModelProviderBook {
    root: PathBuf,
}

impl ModelProviderBook {
    /// Open the directory a person's providers live in.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory cannot be made or a document in it is
    /// not a provider - a book that silently dropped one would leave a
    /// profile naming a provider that is not there.
    pub fn open(root: &Path) -> Result<Self, WorkbenchShellError> {
        std::fs::create_dir_all(root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let root = std::fs::canonicalize(root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let book = Self { root };
        book.stored()?;
        Ok(book)
    }

    fn path_of(&self, id: &str) -> Result<PathBuf, WorkbenchShellError> {
        crate::profile::validate_id("model provider id", id)
            .map_err(WorkbenchShellError::Invalid)?;
        Ok(self.root.join(format!("{id}.json")))
    }

    fn stored(&self) -> Result<BTreeMap<String, ModelProvider>, WorkbenchShellError> {
        let mut found = BTreeMap::new();
        for entry in std::fs::read_dir(&self.root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        {
            let entry = entry.map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            let mut provider: ModelProvider = serde_json::from_slice(&bytes).map_err(|error| {
                WorkbenchShellError::Failed(format!(
                    "{} is not a model provider: {error}",
                    path.display()
                ))
            })?;
            let stem = path
                .file_stem()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or_default();
            if stem != provider.id {
                return Err(WorkbenchShellError::Failed(format!(
                    "{} declares the provider {}, which does not match its file name",
                    path.display(),
                    provider.id
                )));
            }
            validate_provider(&provider)?;
            provider.origin = ModelProviderOrigin::Declared;
            found.insert(provider.id.clone(), provider);
        }
        Ok(found)
    }

    /// Every provider a profile may name, shipped and declared, by id.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory is unreadable.
    pub fn list(&self) -> Result<Vec<ModelProviderView>, WorkbenchShellError> {
        let mut all: BTreeMap<String, ModelProvider> = built_in()
            .iter()
            .map(|provider| (provider.id.clone(), provider.clone()))
            .collect();
        for (id, provider) in self.stored()? {
            all.insert(id, provider);
        }
        Ok(all.into_values().map(view_of).collect())
    }

    /// One provider by id, declared or shipped.
    ///
    /// # Errors
    ///
    /// Not found for an id nobody declared and the product does not ship.
    pub fn get(&self, id: &str) -> Result<ModelProvider, WorkbenchShellError> {
        let path = self.path_of(id)?;
        if path.is_file() {
            let stored = self.stored()?;
            if let Some(provider) = stored.get(id) {
                return Ok(provider.clone());
            }
        }
        built_in()
            .iter()
            .find(|provider| provider.id == id)
            .cloned()
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no model provider named {id}")))
    }

    /// Declare a provider, or correct one - a shipped one included.
    ///
    /// # Errors
    ///
    /// Refuses an unusable id, a kind of key the catalogue does not know, an
    /// address that is not http(s), and a model list with a blank or repeated
    /// id.
    pub fn declare(
        &self,
        body: &DeclareModelProviderBody,
    ) -> Result<ModelProviderView, WorkbenchShellError> {
        let id = body.id.trim().to_owned();
        let path = self.path_of(&id)?;
        let base_url = body.base_url.trim();
        let provider = ModelProvider {
            schema: MODEL_PROVIDER_SCHEMA.to_owned(),
            id,
            name: body.name.trim().to_owned(),
            base_url: (!base_url.is_empty()).then(|| base_url.to_owned()),
            key_type: body.key_type.trim().to_owned(),
            models: body
                .models
                .iter()
                .map(|choice| ModelChoice {
                    id: choice.id.trim().to_owned(),
                    name: choice.name.trim().to_owned(),
                })
                .filter(|choice| !choice.id.is_empty())
                .collect(),
            origin: ModelProviderOrigin::Declared,
        };
        validate_provider(&provider)?;
        let bytes = serde_json::to_vec_pretty(&provider)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let temporary = self
            .root
            .join(format!(".{}.{}.tmp", provider.id, std::process::id()));
        std::fs::write(&temporary, &bytes)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        std::fs::rename(&temporary, &path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            WorkbenchShellError::Failed(error.to_string())
        })?;
        Ok(view_of(provider))
    }

    /// Forget a provider a person declared. A shipped one comes back as
    /// shipped; it cannot be removed, only corrected.
    ///
    /// # Errors
    ///
    /// Not found for an id nobody declared; invalid for a shipped id that was
    /// never overridden, since there is nothing here to forget.
    pub fn forget(&self, id: &str) -> Result<(), WorkbenchShellError> {
        let path = self.path_of(id)?;
        if !path.is_file() {
            if built_in().iter().any(|provider| provider.id == id) {
                return Err(WorkbenchShellError::Invalid(format!(
                    "{id} is shipped with the product; declare it again to change it"
                )));
            }
            return Err(WorkbenchShellError::NotFound(format!(
                "no declared model provider named {id}"
            )));
        }
        std::fs::remove_file(&path).map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }
}

/// What a person's key for this provider is called, and the variable it goes
/// under, from the one catalogue of key kinds.
fn view_of(provider: ModelProvider) -> ModelProviderView {
    let kind = crate::profile::secret_types()
        .iter()
        .find(|kind| kind.type_id == provider.key_type);
    ModelProviderView {
        key_env: kind.and_then(|kind| kind.env_var).map(str::to_owned),
        key_label: kind.map_or_else(|| provider.key_type.clone(), |kind| kind.label.to_owned()),
        provider,
    }
}

fn validate_provider(provider: &ModelProvider) -> Result<(), WorkbenchShellError> {
    if provider.schema != MODEL_PROVIDER_SCHEMA {
        return Err(WorkbenchShellError::Invalid(format!(
            "unsupported model provider schema {}",
            provider.schema
        )));
    }
    crate::profile::validate_id("model provider id", &provider.id)
        .map_err(WorkbenchShellError::Invalid)?;
    if provider.name.trim().is_empty() {
        return Err(WorkbenchShellError::Invalid(
            "a model provider needs a name".into(),
        ));
    }
    if let Some(url) = &provider.base_url
        && !url.starts_with("http://")
        && !url.starts_with("https://")
    {
        return Err(WorkbenchShellError::Invalid(
            "a model provider's address is an http(s) URL".into(),
        ));
    }
    if !crate::profile::secret_types()
        .iter()
        .any(|kind| kind.type_id == provider.key_type)
    {
        return Err(WorkbenchShellError::Invalid(format!(
            "{} is not a kind of key this product knows",
            provider.key_type
        )));
    }
    let mut seen = BTreeSet::new();
    for choice in &provider.models {
        if choice.id.trim().is_empty() {
            return Err(WorkbenchShellError::Invalid("a model needs an id".into()));
        }
        if !seen.insert(choice.id.as_str()) {
            return Err(WorkbenchShellError::Invalid(format!(
                "the model {} is listed twice",
                choice.id
            )));
        }
    }
    Ok(())
}

/// The providers the product ships, read once.
///
/// # Panics
///
/// Panics when the embedded catalogue is not the JSON this build was made
/// with - a build defect, never a runtime condition.
pub fn built_in() -> &'static [ModelProvider] {
    static SHIPPED: std::sync::OnceLock<Vec<ModelProvider>> = std::sync::OnceLock::new();
    SHIPPED.get_or_init(|| {
        let mut shipped: Vec<ModelProvider> =
            serde_json::from_str(include_str!("../model-providers.json"))
                .expect("the shipped model providers are valid JSON");
        for provider in &mut shipped {
            provider.origin = ModelProviderOrigin::BuiltIn;
            validate_provider(provider).expect("a shipped model provider is valid");
        }
        shipped
    })
}

impl WorkbenchShellState {
    /// Keep the model providers a person declares in `root`, over the ones
    /// the product ships.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory or any document in it is unusable,
    /// and when the book is enabled twice.
    pub fn enable_model_providers(&self, root: &Path) -> Result<(), WorkbenchShellError> {
        let book = ModelProviderBook::open(root)?;
        self.model_providers.set(book).map_err(|_| {
            WorkbenchShellError::Conflict("model providers are already enabled".into())
        })
    }

    pub(super) fn model_provider_book(&self) -> Result<&ModelProviderBook, WorkbenchShellError> {
        self.model_providers.get().ok_or_else(|| {
            WorkbenchShellError::NotFound(
                "this host keeps no model providers; enable them with a data root".into(),
            )
        })
    }

    /// Every provider a profile may name.
    ///
    /// # Errors
    ///
    /// Not found when the book is not enabled; failed when it is unreadable.
    pub fn model_providers(&self) -> Result<Vec<ModelProviderView>, WorkbenchShellError> {
        self.model_provider_book()?.list()
    }

    /// Declare or correct one provider.
    ///
    /// # Errors
    ///
    /// As [`ModelProviderBook::declare`].
    pub fn declare_model_provider(
        &self,
        body: &DeclareModelProviderBody,
    ) -> Result<ModelProviderView, WorkbenchShellError> {
        self.model_provider_book()?.declare(body)
    }

    /// Forget one declared provider.
    ///
    /// # Errors
    ///
    /// As [`ModelProviderBook::forget`].
    pub fn forget_model_provider(&self, id: &str) -> Result<(), WorkbenchShellError> {
        self.model_provider_book()?.forget(id)
    }

    /// Refuse a profile setup naming a provider this host does not have, or a
    /// model that provider does not list - while the person is looking at the
    /// form, not at the start of a session. A provider with no model list
    /// takes any model id, because nobody can enumerate it here.
    pub(super) fn check_setup(&self, setup: &crate::AgentSetup) -> Result<(), WorkbenchShellError> {
        let Some(provider_id) = &setup.model_provider else {
            return Ok(());
        };
        let provider = self.model_provider_book()?.get(provider_id)?;
        if let Some(model) = &setup.model
            && !provider.models.is_empty()
            && !provider.models.iter().any(|choice| &choice.id == model)
        {
            return Err(WorkbenchShellError::Invalid(format!(
                "{} does not list a model {model}; pick one it lists or declare the provider with it",
                provider.name
            )));
        }
        Ok(())
    }
}
