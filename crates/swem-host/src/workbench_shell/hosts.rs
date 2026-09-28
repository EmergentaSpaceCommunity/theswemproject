//! Hosts as Providers shows them: what this machine is, and where on it an
//! agent may live.
//!
//! What is offered comes from what was found. The machine is looked at when
//! the product starts and whenever a person asks; a container is offered
//! only where one can be started, and where it cannot the page says why.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{WorkbenchShellError, WorkbenchShellState};
use crate::host::{look_at_this_machine, read_look, write_look};
use crate::{EnvironmentProfileOption, IN_A_CONTAINER, MachineLook, THIS_MACHINE};

/// How long a look may take before it is given up on: an engine that hangs
/// must not leave the page waiting for what it will never be told.
const LOOK_WITHIN: Duration = Duration::from_secs(45);

/// A place an agent may live, as Providers lists it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HostStanding {
    /// The id an agent names it by.
    pub id: String,
    pub name: String,
    /// What kind of host it is, in a word or two.
    pub kind: String,
    /// The machine behind it, as it was found.
    pub machine: String,
    pub an_agent_gets: String,
    /// The profiles of the agents that live on it.
    pub used_by: Vec<String>,
    pub ready: bool,
    /// How it stands, in words.
    pub said: String,
}

/// Where an agent may run, with whether it can today.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentOffered {
    #[serde(flatten)]
    pub option: EnvironmentProfileOption,
    pub available: bool,
    /// Why it cannot be chosen today; empty when it can.
    pub why_not: String,
}

fn sized(bytes: u64) -> String {
    const GIGABYTE: u64 = 1 << 30;
    format!("{} GB", (bytes + GIGABYTE / 2) / GIGABYTE)
}

impl WorkbenchShellState {
    /// Keep what is found of this machine under `data_root`, beginning with
    /// what was kept there last time.
    ///
    /// # Errors
    ///
    /// Refuses being enabled twice.
    pub fn enable_machine_look(&self, data_root: &Path) -> Result<(), WorkbenchShellError> {
        self.machine_root.set(data_root.to_owned()).map_err(|_| {
            WorkbenchShellError::Conflict("the machine is already looked at".into())
        })?;
        if let (Some(kept), Ok(mut look)) = (read_look(data_root), self.machine_look.write()) {
            *look = Some(kept);
        }
        Ok(())
    }

    /// What was last found of this machine; nothing when it was never
    /// looked at.
    #[must_use]
    pub fn machine(&self) -> Option<MachineLook> {
        self.machine_look.read().ok().and_then(|look| look.clone())
    }

    /// Look at this machine now and keep what was found.
    ///
    /// # Errors
    ///
    /// Not found when the look is not enabled; failed when the machine
    /// could not be looked at in time.
    pub async fn look_at_the_machine(&self) -> Result<MachineLook, WorkbenchShellError> {
        let root =
            self.machine_root.get().cloned().ok_or_else(|| {
                WorkbenchShellError::NotFound("this host looks at no machine".into())
            })?;
        let looking = tokio::task::spawn_blocking(move || {
            let look = look_at_this_machine(&root);
            // What was found is worth saying whether or not it could be kept.
            let _ = write_look(&root, &look);
            look
        });
        let look = tokio::time::timeout(LOOK_WITHIN, looking)
            .await
            .map_err(|_| {
                WorkbenchShellError::Failed(
                    "This computer could not be looked at in time; a container engine on it \
                     does not answer."
                        .into(),
                )
            })?
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if let Ok(mut kept) = self.machine_look.write() {
            *kept = Some(look.clone());
        }
        Ok(look)
    }

    /// Look at this machine beside everything else, as the product does
    /// when it starts: nobody waits for it.
    pub fn look_at_the_machine_meanwhile(self: &Arc<Self>) {
        if self.machine_root.get().is_none() {
            return;
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
            if let Err(error) = state.look_at_the_machine().await {
                eprintln!("SWEM could not look at this machine: {error}");
            }
        });
    }

    /// Where an agent may run, with whether each can be chosen today.
    /// A machine nobody looked at promises nothing and refuses nothing.
    #[must_use]
    pub fn environments_offered(&self) -> Vec<EnvironmentOffered> {
        let look = self.machine();
        crate::environment_profiles()
            .into_iter()
            .map(|option| {
                let why_not = match &look {
                    Some(look) if option.environment_profile_id == IN_A_CONTAINER => {
                        look.why_no_container().unwrap_or_default()
                    }
                    _ => String::new(),
                };
                EnvironmentOffered {
                    option,
                    available: why_not.is_empty(),
                    why_not,
                }
            })
            .collect()
    }

    /// Refuse a place an agent cannot live in today, in the words the look
    /// found, while the person is looking at the form.
    pub(super) fn check_environment(&self, chosen: &str) -> Result<(), WorkbenchShellError> {
        match self
            .environments_offered()
            .into_iter()
            .find(|offered| offered.option.environment_profile_id == chosen)
        {
            Some(offered) if !offered.available => Err(WorkbenchShellError::Invalid(format!(
                "An agent cannot be put in a container here today. {}",
                offered.why_not
            ))),
            _ => Ok(()),
        }
    }

    /// The hosts of this product: this machine directly, and containers on
    /// it.
    ///
    /// # Errors
    ///
    /// Fails when the agents cannot be listed.
    pub fn hosts(&self) -> Result<Vec<HostStanding>, WorkbenchShellError> {
        let profiles = self.profiles()?;
        let look = self.machine();
        let living_on = |id: &str| -> Vec<String> {
            profiles
                .iter()
                .filter(|profile| profile.environment_profile_id == id)
                .map(|profile| profile.profile_id.clone())
                .collect()
        };
        let machine = look.as_ref().map_or_else(String::new, |look| {
            format!(
                "{} · {} cores · {}",
                look.system,
                look.processors,
                sized(look.memory_bytes)
            )
        });
        let podman = look.as_ref().map(|look| &look.podman);
        Ok(vec![
            HostStanding {
                id: THIS_MACHINE.into(),
                name: "This machine, directly".into(),
                kind: "Built in".into(),
                machine,
                an_agent_gets: "Its own folder, not sealed".into(),
                used_by: living_on(THIS_MACHINE),
                ready: true,
                said: "Ready".into(),
            },
            HostStanding {
                id: IN_A_CONTAINER.into(),
                name: "Containers here".into(),
                kind: "Podman".into(),
                machine: podman
                    .and_then(|podman| podman.version.as_ref())
                    .map_or_else(String::new, |version| format!("Podman {version}")),
                an_agent_gets: "A sealed container, with no network yet".into(),
                used_by: living_on(IN_A_CONTAINER),
                ready: look
                    .as_ref()
                    .is_some_and(MachineLook::a_container_can_start),
                said: match &look {
                    None => "Not looked at yet".into(),
                    Some(look) => look
                        .why_no_container()
                        .unwrap_or_else(|| "Ready".to_owned()),
                },
            },
        ])
    }
}
