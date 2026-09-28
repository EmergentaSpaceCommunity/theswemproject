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

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
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

/// The keys of this data root.
#[derive(Clone, Debug)]
pub struct KeyStore {
    root: PathBuf,
}

impl KeyStore {
    /// Open the directory keys are kept in, making it for its owner alone.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError`] when the directory cannot be made or closed to
    /// everybody else.
    pub fn open(root: &Path) -> Result<Self, KeyError> {
        let io = |error: std::io::Error| KeyError::Io {
            path: root.to_owned(),
            message: error.to_string(),
        };
        fs::create_dir_all(root).map_err(io)?;
        // Closed every time it is opened, not only when it is made: a
        // directory somebody opened up is closed again before a key is read.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        Ok(Self {
            root: fs::canonicalize(root).map_err(io)?,
        })
    }

    /// What keeps the keys, in words a page can say.
    #[must_use]
    pub fn kept_by(&self) -> &'static str {
        "a file on this computer that only you can read"
    }

    fn path_of(&self, provider: &str) -> Result<PathBuf, KeyError> {
        crate::profile::validate_id("provider", provider).map_err(KeyError::Invalid)?;
        Ok(self.root.join(format!("{provider}.json")))
    }

    fn stored(&self, provider: &str) -> Result<Option<StoredKey>, KeyError> {
        let path = self.path_of(provider)?;
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(KeyError::Io {
                    path,
                    message: error.to_string(),
                });
            }
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
        let path = self.path_of(provider)?;
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
            path: path.clone(),
            message: error.to_string(),
        })?;
        let io = |error: std::io::Error| KeyError::Io {
            path: path.clone(),
            message: error.to_string(),
        };
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        {
            let mut options = OpenOptions::new();
            options.write(true).create(true).truncate(true);
            // Its owner's alone from the first byte.
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
        let path = self.path_of(provider)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(KeyError::Io {
                path,
                message: error.to_string(),
            }),
        }
    }
}
