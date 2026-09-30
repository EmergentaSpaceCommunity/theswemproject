//! A provider's key, given once and handed to every agent that stands on
//! the provider.
//!
//! The key is kept by [`KeyStore`], beside the providers and not inside any
//! agent. When an agent's engine starts on this machine it is handed the key
//! of the provider the agent answers from, under the variable that kind of
//! key is read from, and then whatever the agent has of its own over it: an
//! agent with a key of its own keeps using it.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{
    ModelProviderView, PersonalAgentProfile, WorkbenchShellError, WorkbenchShellState,
    profile_error,
};
use crate::{KeyError, KeyHeld, KeyStore};

/// A provider as Providers shows it: what it is, whether it has its key,
/// and which agents answer from it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ModelProviderStanding {
    #[serde(flatten)]
    pub provider: ModelProviderView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<KeyHeld>,
    /// The profiles of the agents that answer from it.
    pub used_by: Vec<String>,
}

/// What a person gives when they give a key.
#[derive(Clone, Deserialize)]
pub struct GiveKeyBody {
    pub value: String,
    /// The variable an engine reads it from, for a kind of key that names
    /// none of its own.
    #[serde(default)]
    pub variable: Option<String>,
}

fn key_refusal(error: KeyError) -> WorkbenchShellError {
    match error {
        KeyError::Invalid(said) => WorkbenchShellError::Invalid(said),
        other => WorkbenchShellError::Failed(other.to_string()),
    }
}

impl WorkbenchShellState {
    /// Keep the keys of providers in `root`, and move there what agents
    /// held of their provider's key.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory cannot be made its owner's alone,
    /// and when keys are enabled twice.
    pub fn enable_provider_keys(&self, root: &Path) -> Result<(), WorkbenchShellError> {
        let keys = KeyStore::open(root).map_err(key_refusal)?;
        self.provider_keys
            .set(keys)
            .map_err(|_| WorkbenchShellError::Conflict("keys are already enabled".into()))?;
        self.settle_keys()
    }

    /// Keep the keys of providers with a keeper a product supplied, named
    /// under `keys/`.
    ///
    /// # Errors
    ///
    /// Keys are enabled already.
    pub fn enable_provider_keys_kept_by(
        &self,
        keeper: crate::Keeper,
    ) -> Result<(), WorkbenchShellError> {
        self.provider_keys
            .set(KeyStore::kept_by(keeper, "keys/"))
            .map_err(|_| WorkbenchShellError::Conflict("keys are already enabled".into()))?;
        self.settle_keys()
    }

    fn keys(&self) -> Result<&KeyStore, WorkbenchShellError> {
        self.provider_keys.get().ok_or_else(|| {
            WorkbenchShellError::NotFound(
                "this host keeps no keys; enable them with a data root".into(),
            )
        })
    }

    /// The variable a provider's key is handed over under.
    fn key_variable(
        provider: &ModelProviderView,
        named: Option<&str>,
    ) -> Result<String, WorkbenchShellError> {
        if let Some(variable) = &provider.key_env {
            return Ok(variable.clone());
        }
        named
            .map(str::trim)
            .filter(|variable| !variable.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                WorkbenchShellError::Invalid(format!(
                    "{} reads its key from a variable you name; say which",
                    provider.provider.name
                ))
            })
    }

    fn provider_view(&self, id: &str) -> Result<ModelProviderView, WorkbenchShellError> {
        self.model_providers()?
            .into_iter()
            .find(|provider| provider.provider.id == id)
            .ok_or_else(|| WorkbenchShellError::NotFound(format!("no model provider named {id}")))
    }

    /// Every provider with whether it has its key and who answers from it,
    /// and what keeps the keys.
    ///
    /// # Errors
    ///
    /// Not found when providers are not enabled; failed when a key's file
    /// is there and cannot be read.
    pub fn providers_standing(
        &self,
    ) -> Result<(Vec<ModelProviderStanding>, Option<String>), WorkbenchShellError> {
        let profiles = self.profiles()?;
        let keys = self.provider_keys.get();
        let standing = self
            .model_providers()?
            .into_iter()
            .map(|provider| {
                let key = match keys {
                    Some(keys) => keys.held(&provider.provider.id).map_err(key_refusal)?,
                    None => None,
                };
                let used_by = profiles
                    .iter()
                    .filter(|profile| {
                        profile.model_provider.as_deref() == Some(provider.provider.id.as_str())
                    })
                    .map(|profile| profile.profile_id.clone())
                    .collect();
                Ok(ModelProviderStanding {
                    provider,
                    key,
                    used_by,
                })
            })
            .collect::<Result<Vec<_>, WorkbenchShellError>>()?;
        Ok((standing, keys.map(KeyStore::kept_by_words)))
    }

    /// Give a provider its key.
    ///
    /// # Errors
    ///
    /// Not found for a provider nobody declared; refuses blanks, and a kind
    /// of key that names no variable when none is given.
    pub fn give_provider_key(
        &self,
        id: &str,
        body: &GiveKeyBody,
    ) -> Result<KeyHeld, WorkbenchShellError> {
        let provider = self.provider_view(id)?;
        let variable = Self::key_variable(&provider, body.variable.as_deref())?;
        self.keys()?
            .give(id, &variable, &body.value)
            .map_err(key_refusal)
    }

    /// Take a provider's key away.
    ///
    /// # Errors
    ///
    /// Not found when keys are not enabled; failed when the file cannot be
    /// removed.
    pub fn take_provider_key(&self, id: &str) -> Result<(), WorkbenchShellError> {
        self.keys()?.take(id).map_err(key_refusal)
    }

    /// The keys an agent's engine and its terminals are handed on this
    /// machine: its provider's, then its own over it.
    ///
    /// # Errors
    ///
    /// Fails closed when a key that is there cannot be read: an engine
    /// started without the key it was promised fails further away.
    pub(super) fn keys_of(
        &self,
        profile: &PersonalAgentProfile,
    ) -> Result<BTreeMap<String, String>, WorkbenchShellError> {
        let mut handed = BTreeMap::new();
        if let (Some(keys), Some(provider)) = (self.provider_keys.get(), &profile.model_provider)
            && let Some(key) = keys.key(provider).map_err(key_refusal)?
        {
            handed.insert(key.variable, key.value);
        }
        handed.extend(
            self.inventory
                .secrets(&profile.profile_id)
                .map_err(profile_error)?,
        );
        Ok(handed)
    }

    /// Move to each provider the key an agent held for it.
    ///
    /// An agent's secret is its provider's key when it is of the kind the
    /// provider takes and under the variable that kind is read from. It
    /// moves when the provider has no key or has the same one; an agent
    /// whose key differs keeps its own. An agent that names no provider
    /// keeps everything: there is nowhere to move it.
    fn settle_keys(&self) -> Result<(), WorkbenchShellError> {
        let Some(keys) = self.provider_keys.get() else {
            return Ok(());
        };
        if self.model_providers.get().is_none() {
            return Ok(());
        }
        for profile in self.profiles()? {
            let Some(provider_id) = &profile.model_provider else {
                continue;
            };
            let Ok(provider) = self.provider_view(provider_id) else {
                continue;
            };
            let Some(variable) = &provider.key_env else {
                continue;
            };
            let own = self
                .inventory
                .secret_entries(&profile.profile_id)
                .map_err(profile_error)?;
            if !own
                .iter()
                .any(|entry| &entry.name == variable && entry.type_id == provider.provider.key_type)
            {
                continue;
            }
            let Some(value) = self
                .inventory
                .secrets(&profile.profile_id)
                .map_err(profile_error)?
                .remove(variable)
            else {
                continue;
            };
            match keys.key(provider_id).map_err(key_refusal)? {
                Some(held) if held.value != value => continue,
                Some(_) => {}
                None => {
                    keys.give(provider_id, variable, &value)
                        .map_err(key_refusal)?;
                }
            }
            self.inventory
                .remove_secret(&profile.profile_id, variable)
                .map_err(profile_error)?;
        }
        Ok(())
    }
}
