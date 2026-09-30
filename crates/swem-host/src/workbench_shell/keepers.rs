//! Who keeps time, as Providers shows it and as a person changes it.
//!
//! The running product keeps time while it runs. The system's own
//! scheduler is turned on and off from here: it starts the product to look
//! at what is due when the Workbench is closed. A scheduler outside knocks
//! on a Workbench served at an address, with a token that may do nothing
//! else; it is added by making that token.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{WorkbenchShellError, WorkbenchShellState};
use crate::host::system_scheduler::{self, SystemJob};
use crate::{KEPT_BY_SWEM, KEPT_BY_THE_SYSTEM, KeeperLook, Keepers};

/// One who keeps time, as Providers shows it.
#[derive(Clone, Debug, Serialize)]
pub struct KeeperShown {
    /// `swem`, `system` or `outside`.
    pub id: String,
    /// Whether it can be used on this system today.
    pub available: bool,
    /// Whether it keeps time now: the running product does when it holds
    /// the keeper's lock, the system's scheduler when it holds the job.
    pub on: bool,
    /// Agents nothing was chosen for have this one.
    pub default: bool,
    /// The profiles of the agents whose time it keeps.
    pub used_by: Vec<String>,
    /// What the system's scheduler is called here.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub called: String,
    /// When it looked last, for one that looks and leaves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_look: Option<KeeperLook>,
    /// What is wrong with it, in words; empty when nothing is.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub said: String,
    /// Where a scheduler outside knocks, when this Workbench can be
    /// knocked on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knocks_at: Option<String>,
}

/// What a person chooses when the system's scheduler is turned on.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct TurnOnBody {
    /// Let it keep time for every agent nothing else was chosen for.
    #[serde(default)]
    pub make_default: bool,
}

/// Who keeps an agent's time, as a person chose; nothing follows the
/// default.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ChooseKeeperBody {
    #[serde(default)]
    pub keeper: Option<String>,
}

fn refused(error: &crate::KeepersError) -> WorkbenchShellError {
    match error {
        crate::KeepersError::Unknown(_) => WorkbenchShellError::Invalid(error.to_string()),
        crate::KeepersError::Io { .. } => WorkbenchShellError::Failed(error.to_string()),
    }
}

impl WorkbenchShellState {
    /// Keep the choices of who keeps time under `root`.
    ///
    /// # Errors
    ///
    /// Refuses being enabled twice.
    pub fn enable_keepers(&self, root: &Path) -> Result<(), WorkbenchShellError> {
        self.keepers
            .set(Keepers::at(root))
            .map_err(|_| WorkbenchShellError::Conflict("who keeps time is already kept".into()))
    }

    /// The command the system's scheduler starts: an executable that keeps
    /// time once over this product's data and leaves.
    pub fn set_keep_time_command(&self, executable: std::path::PathBuf, args: Vec<String>) {
        let _ = self.keep_time_command.set((executable, args));
    }

    fn keepers_kept(&self) -> Result<&Keepers, WorkbenchShellError> {
        self.keepers.get().ok_or_else(|| {
            WorkbenchShellError::NotFound("who keeps time is not kept by this product".into())
        })
    }

    /// The job the system is given for this product's data.
    fn the_systems_job(&self) -> Result<SystemJob, WorkbenchShellError> {
        let keepers = self.keepers_kept()?;
        let (program, args) = self.keep_time_command.get().cloned().ok_or_else(|| {
            WorkbenchShellError::NotFound("this product has no command that keeps time once".into())
        })?;
        Ok(SystemJob {
            name: system_scheduler::job_name(keepers.root()),
            program,
            args,
            environment: system_scheduler::carried_environment(),
            log: keepers.root().join("system.log"),
        })
    }

    /// Who keeps time, each as it stands now, and what was chosen for
    /// which agent.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the profiles cannot be read.
    pub async fn keepers_shown(
        &self,
    ) -> Result<(Vec<KeeperShown>, BTreeMap<String, String>), WorkbenchShellError> {
        let Some(keepers) = self.keepers.get().cloned() else {
            return Ok((Vec::new(), BTreeMap::new()));
        };
        let profiles: Vec<String> = self
            .profiles()?
            .into_iter()
            .map(|profile| profile.profile_id)
            .collect();
        let name = system_scheduler::job_name(keepers.root());
        let standing = tokio::task::spawn_blocking(move || system_scheduler::standing(&name))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let default = keepers.default_keeper();
        let kept_by = |keeper: &str| -> Vec<String> {
            profiles
                .iter()
                .filter(|profile_id| keepers.keeper_of(profile_id) == keeper)
                .cloned()
                .collect()
        };
        let chosen = profiles
            .iter()
            .filter_map(|profile_id| {
                keepers
                    .chosen_for(profile_id)
                    .map(|keeper| (profile_id.clone(), keeper))
            })
            .collect();
        let can_be_started = self.keep_time_command.get().is_some();
        let (knocks_at, knocking) = self.knocked_on();
        let shown = vec![
            KeeperShown {
                id: KEPT_BY_SWEM.into(),
                available: true,
                on: self.timekeeper.get().is_some(),
                default: default == KEPT_BY_SWEM,
                used_by: kept_by(KEPT_BY_SWEM),
                called: String::new(),
                last_look: None,
                said: String::new(),
                knocks_at: None,
            },
            KeeperShown {
                id: KEPT_BY_THE_SYSTEM.into(),
                available: can_be_started
                    && standing.scheduler != system_scheduler::SystemScheduler::None,
                on: standing.on,
                default: default == KEPT_BY_THE_SYSTEM,
                used_by: kept_by(KEPT_BY_THE_SYSTEM),
                called: standing.scheduler.called().to_owned(),
                last_look: keepers.last_look(KEPT_BY_THE_SYSTEM),
                said: standing.said,
                knocks_at: None,
            },
            KeeperShown {
                id: crate::KEPT_FROM_OUTSIDE.into(),
                available: knocks_at.is_some(),
                on: knocking > 0,
                default: false,
                used_by: Vec::new(),
                called: String::new(),
                last_look: keepers.last_look(crate::KEPT_FROM_OUTSIDE),
                said: String::new(),
                knocks_at,
            },
        ];
        Ok((shown, chosen))
    }

    /// Give the system's scheduler the job of starting this product to
    /// look at what is due.
    ///
    /// # Errors
    ///
    /// What the system said when it would not take the job.
    pub async fn turn_the_system_on(&self, body: TurnOnBody) -> Result<(), WorkbenchShellError> {
        let job = self.the_systems_job()?;
        tokio::task::spawn_blocking(move || system_scheduler::turn_on(&job))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            .map_err(WorkbenchShellError::Failed)?;
        if body.make_default {
            self.keepers_kept()?
                .make_default(KEPT_BY_THE_SYSTEM)
                .map_err(|error| refused(&error))?;
        }
        Ok(())
    }

    /// Take the job away from the system. The agents it kept time for are
    /// the running product's again.
    ///
    /// # Errors
    ///
    /// A file of the job that cannot be removed.
    pub async fn turn_the_system_off(&self) -> Result<(), WorkbenchShellError> {
        let keepers = self.keepers_kept()?.clone();
        let name = system_scheduler::job_name(keepers.root());
        tokio::task::spawn_blocking(move || system_scheduler::turn_off(&name))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            .map_err(WorkbenchShellError::Failed)?;
        if keepers.default_keeper() == KEPT_BY_THE_SYSTEM {
            keepers
                .make_default(KEPT_BY_SWEM)
                .map_err(|error| refused(&error))?;
        }
        for profile in self.profiles()? {
            if keepers.chosen_for(&profile.profile_id).as_deref() == Some(KEPT_BY_THE_SYSTEM) {
                keepers
                    .choose_for(&profile.profile_id, None)
                    .map_err(|error| refused(&error))?;
            }
        }
        Ok(())
    }

    /// Let a keeper keep time for every agent nothing else was chosen for.
    ///
    /// # Errors
    ///
    /// Refuses one that keeps no time, and one that is off.
    pub async fn make_default_keeper(&self, keeper: &str) -> Result<(), WorkbenchShellError> {
        self.on_or_refused(keeper).await?;
        self.keepers_kept()?
            .make_default(keeper)
            .map_err(|error| refused(&error))
    }

    /// Choose who keeps an agent's time; nothing lets it follow the
    /// default.
    ///
    /// # Errors
    ///
    /// Refuses an agent that is not there, one that keeps no time, and one
    /// that is off.
    pub async fn choose_keeper(
        &self,
        profile_id: &str,
        body: ChooseKeeperBody,
    ) -> Result<(), WorkbenchShellError> {
        if !self
            .profiles()?
            .iter()
            .any(|profile| profile.profile_id == profile_id)
        {
            return Err(WorkbenchShellError::NotFound(format!(
                "no agent {profile_id}"
            )));
        }
        if let Some(keeper) = &body.keeper {
            self.on_or_refused(keeper).await?;
        }
        self.keepers_kept()?
            .choose_for(profile_id, body.keeper.as_deref())
            .map_err(|error| refused(&error))
    }

    /// Nobody is given to a keeper that keeps no time.
    async fn on_or_refused(&self, keeper: &str) -> Result<(), WorkbenchShellError> {
        if keeper != KEPT_BY_THE_SYSTEM {
            return Ok(());
        }
        let name = system_scheduler::job_name(self.keepers_kept()?.root());
        let standing = tokio::task::spawn_blocking(move || system_scheduler::standing(&name))
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if standing.on {
            Ok(())
        } else {
            Err(WorkbenchShellError::Conflict(
                "the system's scheduler is off; turn it on first".into(),
            ))
        }
    }
}
