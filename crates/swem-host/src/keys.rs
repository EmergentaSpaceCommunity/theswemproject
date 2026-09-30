//! The keys of providers, given once.
//!
//! A key belongs to the provider it opens, not to an agent that uses the
//! provider: a person gives it once and every agent standing on that
//! provider is handed it when it starts. It is kept in one directory only
//! its owner can enter, one file per provider only its owner can read, and
//! it is never written anywhere else - not into a profile, not into the
//! ledger, not into a command line.
//!
//! What keeps a key is said to the person in words ([`KeyStore::kept_by`]),
//! so that the day it is the system's keychain the page says so too.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The document one provider's key is kept in.
pub const PROVIDER_KEY_SCHEMA: &str = "swem:provider-key@0.1";

#[derive(Debug, Error)]
pub enum KeyError {
    #[error("{0}")]
    Invalid(String),
    #[error("the keys could not be reached at {path}: {message}")]
    Io { path: PathBuf, message: String },
    #[error("{path} is not a key this product wrote: {message}")]
    Unreadable { path: PathBuf, message: String },
}

#[derive(Clone, Deserialize, Serialize)]
struct StoredKey {
    schema: String,
    provider: String,
    /// The environment variable an engine reads the key from.
    variable: String,
    value: String,
    given_ms: u64,
}

/// A key with its value, for the process that is about to start an engine.
#[derive(Clone)]
pub struct ProviderKey {
    pub variable: String,
    pub value: String,
}

impl std::fmt::Debug for ProviderKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderKey")
            .field("variable", &self.variable)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// What a page may know of a key: that it is there, under which variable
/// an engine is handed it, and since when. Never the value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeyHeld {
    pub variable: String,
    pub given_ms: u64,
}

/// The keys of this product, wherever its keeper keeps them.
#[derive(Clone, Debug)]
pub struct KeyStore {
    keeper: crate::Keeper,
    /// What the keys' documents are named under.
    under: String,
}

impl KeyStore {
    /// Keys as files under `root`, made for their owner alone.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the directory cannot be made or closed to
    /// everybody else.
    pub fn open(root: &Path) -> Result<Self, KeyError> {
        let keeper = crate::InFiles::at(root).map_err(|error| KeyError::Io {
            path: root.to_owned(),
            message: error.to_string(),
        })?;
        Ok(Self::kept_by(Arc::new(keeper), ""))
    }

    /// Keys kept by `keeper`, named under `under` (`keys/`, or nothing).
    #[must_use]
    pub fn kept_by(keeper: crate::Keeper, under: &str) -> Self {
        Self {
            keeper,
            under: under.to_owned(),
        }
    }

    /// What keeps the keys, in words a page can say.
    #[must_use]
    pub fn kept_by_words(&self) -> String {
        self.keeper.kept_by()
    }

    fn name_of(&self, provider: &str) -> Result<String, KeyError> {
        crate::profile::validate_id("provider", provider).map_err(KeyError::Invalid)?;
        Ok(format!("{}{provider}.json", self.under))
    }

    fn stored(&self, provider: &str) -> Result<Option<StoredKey>, KeyError> {
        let name = self.name_of(provider)?;
        let path = PathBuf::from(&name);
        let Some(bytes) = self.keeper.read(&name).map_err(|error| KeyError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?
        else {
            return Ok(None);
        };
        let stored: StoredKey =
            serde_json::from_slice(&bytes).map_err(|error| KeyError::Unreadable {
                path: path.clone(),
                // The position of the fault, never the text around it.
                message: format!("line {}, column {}", error.line(), error.column()),
            })?;
        if stored.schema != PROVIDER_KEY_SCHEMA || stored.provider != provider {
            return Err(KeyError::Unreadable {
                path,
                message: "it names another schema or another provider".into(),
            });
        }
        Ok(Some(stored))
    }

    /// Whether a provider has its key, and since when.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the key's file is there and cannot be read.
    pub fn held(&self, provider: &str) -> Result<Option<KeyHeld>, KeyError> {
        Ok(self.stored(provider)?.map(|stored| KeyHeld {
            variable: stored.variable,
            given_ms: stored.given_ms,
        }))
    }

    /// The key itself, for whoever starts an engine with it.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the key's file is there and cannot be read.
    pub fn key(&self, provider: &str) -> Result<Option<ProviderKey>, KeyError> {
        Ok(self.stored(provider)?.map(|stored| ProviderKey {
            variable: stored.variable,
            value: stored.value,
        }))
    }

    /// Give a provider its key, in place of the one it had.
    ///
    /// # Errors
    ///
    /// Refuses blanks for a key and a variable that is not the name of one.
    pub fn give(&self, provider: &str, variable: &str, value: &str) -> Result<KeyHeld, KeyError> {
        let name = self.name_of(provider)?;
        // Blanks are not a key: handed to an engine they fail further away
        // and less legibly than having none.
        if value.trim().is_empty() {
            return Err(KeyError::Invalid("a key is not blanks".into()));
        }
        let named = !variable.is_empty()
            && !variable.starts_with(|first: char| first.is_ascii_digit())
            && variable
                .chars()
                .all(|one| one.is_ascii_alphanumeric() || one == '_');
        if !named {
            return Err(KeyError::Invalid(format!(
                "{variable:?} is not the name of an environment variable"
            )));
        }
        let stored = StoredKey {
            schema: PROVIDER_KEY_SCHEMA.into(),
            provider: provider.to_owned(),
            variable: variable.to_owned(),
            value: value.to_owned(),
            given_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| {
                    u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
                }),
        };
        let bytes = serde_json::to_vec_pretty(&stored).map_err(|error| KeyError::Unreadable {
            path: PathBuf::from(&name),
            message: error.to_string(),
        })?;
        self.keeper
            .write(&name, &bytes)
            .map_err(|error| KeyError::Io {
                path: PathBuf::from(&name),
                message: error.to_string(),
            })?;
        Ok(KeyHeld {
            variable: stored.variable,
            given_ms: stored.given_ms,
        })
    }

    /// Take a provider's key away. Taking away what is not there is done.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the file is there and cannot be removed.
    pub fn take(&self, provider: &str) -> Result<(), KeyError> {
        let name = self.name_of(provider)?;
        self.keeper.remove(&name).map_err(|error| KeyError::Io {
            path: PathBuf::from(name),
            message: error.to_string(),
        })
    }
}
