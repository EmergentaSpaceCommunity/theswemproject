//! Who keeps time, and for which agents.
//!
//! Time is never kept inside an agent's machine, so somebody outside it
//! keeps it, and who that is differs by what it can do. The running product
//! keeps time while it runs. The system's own scheduler starts the product
//! when the Workbench is closed. What is kept here is only the choice: the
//! keeper agents have when nothing was chosen for them, the agents somebody
//! chose one for, and when the system's keeper looked last.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The running product.
pub const KEPT_BY_SWEM: &str = "swem";
/// The system's own scheduler, which starts the product.
pub const KEPT_BY_THE_SYSTEM: &str = "system";
/// A scheduler outside, which knocks: it tells a Workbench served at an
/// address to look. It is chosen for no agent; whoever knocks has the
/// Workbench look for every one.
pub const KEPT_FROM_OUTSIDE: &str = "outside";

pub const KEEPERS_SCHEMA: &str = "swem:timekeepers@0.1";
pub const KEEPER_LOOK_SCHEMA: &str = "swem:timekeeper-look@0.1";

#[derive(Debug, thiserror::Error)]
pub enum KeepersError {
    #[error("{0} is not somebody who keeps time")]
    Unknown(String),
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct Chosen {
    schema: String,
    /// Who keeps time for an agent nothing was chosen for.
    default: String,
    /// The agents somebody chose a keeper for, by their profiles.
    #[serde(default)]
    agents: BTreeMap<String, String>,
}

impl Default for Chosen {
    fn default() -> Self {
        Self {
            schema: KEEPERS_SCHEMA.into(),
            default: KEPT_BY_SWEM.into(),
            agents: BTreeMap::new(),
        }
    }
}

/// One look of a keeper that starts the product, looks and leaves.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct KeeperLook {
    pub schema: String,
    pub looked_ms: u64,
    /// How many messages were said.
    pub said: usize,
    /// The running product kept time, so there was nothing to do.
    pub kept_elsewhere: bool,
    /// When it last said something, whichever look that was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said_last_ms: Option<u64>,
}

/// The choices of who keeps time, under `<data root>/time`.
#[derive(Clone, Debug)]
pub struct Keepers {
    root: PathBuf,
}

fn known(keeper: &str) -> Result<(), KeepersError> {
    if [KEPT_BY_SWEM, KEPT_BY_THE_SYSTEM].contains(&keeper) {
        Ok(())
    } else {
        Err(KeepersError::Unknown(keeper.to_owned()))
    }
}

impl Keepers {
    #[must_use]
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn chosen_at(&self) -> PathBuf {
        self.root.join("keepers.json")
    }

    fn look_at(&self, keeper: &str) -> PathBuf {
        self.root.join(format!("{keeper}-look.json"))
    }

    fn chosen(&self) -> Chosen {
        fs::read(self.chosen_at())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Chosen>(&bytes).ok())
            .filter(|chosen| known(&chosen.default).is_ok())
            .unwrap_or_default()
    }

    fn keep(&self, path: &Path, bytes: &[u8]) -> Result<(), KeepersError> {
        let failed = |path: &Path, error: std::io::Error| KeepersError::Io {
            path: path.to_owned(),
            message: error.to_string(),
        };
        fs::create_dir_all(&self.root).map_err(|error| failed(&self.root, error))?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temporary, bytes).map_err(|error| failed(&temporary, error))?;
        fs::rename(&temporary, path).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            failed(path, error)
        })
    }

    fn keep_chosen(&self, chosen: &Chosen) -> Result<(), KeepersError> {
        let bytes = serde_json::to_vec_pretty(chosen).unwrap_or_default();
        self.keep(&self.chosen_at(), &bytes)
    }

    /// Who keeps time for an agent nothing was chosen for.
    #[must_use]
    pub fn default_keeper(&self) -> String {
        self.chosen().default
    }

    /// # Errors
    ///
    /// Refuses somebody who keeps no time, and a file that cannot be kept.
    pub fn make_default(&self, keeper: &str) -> Result<(), KeepersError> {
        known(keeper)?;
        let mut chosen = self.chosen();
        keeper.clone_into(&mut chosen.default);
        self.keep_chosen(&chosen)
    }

    /// What was chosen for an agent; nothing when it follows the default.
    #[must_use]
    pub fn chosen_for(&self, profile_id: &str) -> Option<String> {
        self.chosen().agents.get(profile_id).cloned()
    }

    /// Who keeps an agent's time.
    #[must_use]
    pub fn keeper_of(&self, profile_id: &str) -> String {
        let chosen = self.chosen();
        chosen
            .agents
            .get(profile_id)
            .filter(|keeper| known(keeper).is_ok())
            .cloned()
            .unwrap_or(chosen.default)
    }

    /// Choose who keeps an agent's time; nothing lets it follow the
    /// default.
    ///
    /// # Errors
    ///
    /// Refuses somebody who keeps no time, and a file that cannot be kept.
    pub fn choose_for(&self, profile_id: &str, keeper: Option<&str>) -> Result<(), KeepersError> {
        let mut chosen = self.chosen();
        match keeper {
            Some(keeper) => {
                known(keeper)?;
                chosen
                    .agents
                    .insert(profile_id.to_owned(), keeper.to_owned());
            }
            None => {
                chosen.agents.remove(profile_id);
            }
        }
        self.keep_chosen(&chosen)
    }

    /// Write down that a keeper looked. What is kept says as well when it
    /// last said something.
    ///
    /// # Errors
    ///
    /// The note cannot be kept.
    pub fn looked(&self, keeper: &str, look: &KeeperLook) -> Result<(), KeepersError> {
        // A look that found nothing does not forget the one that did.
        let look = KeeperLook {
            said_last_ms: if look.said > 0 {
                Some(look.looked_ms)
            } else {
                self.last_look(keeper).and_then(|last| last.said_last_ms)
            },
            ..look.clone()
        };
        let bytes = serde_json::to_vec_pretty(&look).unwrap_or_default();
        self.keep(&self.look_at(keeper), &bytes)
    }

    /// When a keeper looked last, and what it found to say.
    #[must_use]
    pub fn last_look(&self, keeper: &str) -> Option<KeeperLook> {
        fs::read(self.look_at(keeper))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }
}
