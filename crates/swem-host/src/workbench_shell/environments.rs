//! Environments as Providers shows them: the ways this host runs an agent,
//! and what this machine is.
//!
//! An environment is how an agent is run on this host - as is, on the
//! machine itself, or sealed in a container - not a place of its own; the
//! host stays its home. What is offered comes from what was found. The
//! machine is looked at when the product starts and whenever a person asks;
//! a container is offered only where one can be started, and where it cannot
//! the page says why.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{WorkbenchShellError, WorkbenchShellState};
use crate::host::{look_at_this_machine, read_look, write_look};
use crate::{EnvironmentProfileOption, IN_A_CONTAINER, MachineLook};

/// How long a look may take before it is given up on: an engine that hangs
/// must not leave the page waiting for what it will never be told.
const LOOK_WITHIN: Duration = Duration::from_secs(45);

/// One way this host runs an agent, with whether it can today.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnvironmentOffered {
    #[serde(flatten)]
    pub option: EnvironmentProfileOption,
    /// What kind of environment it is, in a word or two: as is, a container.
    pub kind: String,
    /// What runs it, as it was found: the machine, or the engine on it.
    pub machine: String,
    pub an_agent_gets: String,
    /// The profiles of the agents run in it.
    pub used_by: Vec<String>,
    /// Whether an agent can be run in it today. A machine nobody looked at
    /// promises nothing and refuses nothing.
    pub available: bool,
    /// Why it cannot be chosen today; empty when it can.
    pub why_not: String,
}

/// What a person chooses for the machine containers run in. What is left
/// out is what Podman is usually given.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MachineWanted {
    #[serde(default)]
    pub cpus: Option<u16>,
    #[serde(default)]
    pub memory_mib: Option<u64>,
    #[serde(default)]
    pub disk_gib: Option<u64>,
}

/// One thing that is done to set containers up, and how it went.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetUpStep {
    /// What is done, in words.
    pub what: String,
    /// The command that does it, as it is run.
    pub command: String,
    /// `waiting`, `running`, `done` or `failed`.
    pub state: String,
    /// What the command said, when it said something worth reading.
    pub said: String,
}

/// Setting containers up on this machine: what would be done, or what is
/// being done, or how it ended.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetUp {
    /// The plan a person agrees to, by its exact id.
    pub plan_id: String,
    pub steps: Vec<SetUpStep>,
    /// What it does to this computer, in words.
    pub effects: Vec<String>,
    /// Why it cannot be done as asked; empty when it can.
    pub blockers: Vec<String>,
    /// `offered`, `running`, `done` or `failed`.
    pub state: String,
    /// How it ended, in words.
    pub said: String,
    /// What a person can do about a failure, when the product knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// The name of a machine Podman already has, or the one this product makes.
fn machine_named(probe: &crate::BackendProbe) -> String {
    probe.topology["machines"]
        .as_array()
        .and_then(|machines| machines.first())
        .and_then(|machine| machine["Name"].as_str())
        .map_or_else(
            || "swem".to_owned(),
            |name| name.trim_end_matches('*').to_owned(),
        )
}

fn in_words(plan: &crate::ProvisioningPlan) -> SetUp {
    let steps = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            crate::ProvisioningStep::Run {
                executable, args, ..
            } => Some(SetUpStep {
                what: if args.iter().any(|arg| arg == "init") {
                    "Make a Linux machine for containers, and download its system".to_owned()
                } else {
                    "Start the machine".to_owned()
                },
                command: std::iter::once(
                    executable
                        .file_name()
                        .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
                )
                .chain(args.iter().cloned())
                .collect::<Vec<_>>()
                .join(" "),
                state: "waiting".into(),
                said: String::new(),
            }),
            crate::ProvisioningStep::VerifyBackend => None,
        })
        .collect();
    let effects = &plan.effects;
    let mut said = Vec::new();
    if effects.creates_virtual_machine {
        said.push(format!(
            "A virtual machine is made on this computer: {} processors, {} of memory, up to {} GB of disk.",
            effects.cpu_count,
            sized(effects.memory_mib.saturating_mul(1_048_576)),
            effects.disk_gib
        ));
    }
    if effects.may_download_machine_image {
        said.push("Its system is downloaded, several hundred megabytes.".to_owned());
    }
    if effects.uses_provider_default_host_mounts {
        said.push(
            "Podman lets the machine see your home folder. An agent in a container is given \
             its own folder only."
                .to_owned(),
        );
    }
    if effects.requires_privilege_elevation {
        said.push("Your password is asked for.".to_owned());
    }
    if effects.may_require_restart {
        said.push("This computer may have to be restarted.".to_owned());
    }
    SetUp {
        plan_id: plan.plan_id.clone(),
        steps,
        effects: said,
        blockers: plan.blockers.clone(),
        state: "offered".into(),
        said: String::new(),
        hint: None,
    }
}

/// The least free disk a machine for containers is made on: its system
/// unpacked, and room for an image of an agent.
const LEAST_FREE_DISK: u64 = 5 * 1_073_741_824;

/// What this computer cannot give a machine of that size, and what a
/// person should know before agreeing.
fn what_the_machine_says(
    look: &crate::MachineLook,
    effects: &crate::ProvisioningEffects,
) -> (Vec<String>, Vec<String>) {
    let (mut blockers, mut notes) = (Vec::new(), Vec::new());
    if usize::from(effects.cpu_count) > look.processors {
        blockers.push(format!(
            "This computer has {} processors; the machine cannot be given {}.",
            look.processors, effects.cpu_count
        ));
    }
    let memory = effects.memory_mib.saturating_mul(1_048_576);
    if memory >= look.memory_bytes {
        blockers.push(format!(
            "This computer has {} of memory; the machine cannot be given {}.",
            sized(look.memory_bytes),
            sized(memory)
        ));
    } else if memory > look.memory_free_bytes {
        notes.push(format!(
            "Only {} of memory are free now: while the machine runs, this computer will be slower.",
            sized(look.memory_free_bytes)
        ));
    }
    if let Some(free) = look.disk_free_bytes {
        if free < LEAST_FREE_DISK {
            blockers.push(format!(
                "Only {} are free on this disk; making the machine takes {}.",
                sized(free),
                sized(LEAST_FREE_DISK)
            ));
        } else if free < effects.disk_gib.saturating_mul(1_073_741_824) {
            notes.push(format!(
                "Only {} are free on this disk. The machine takes what it uses, not all {} GB \
                 at once, and cannot grow past what is free.",
                sized(free),
                effects.disk_gib
            ));
        }
    }
    (blockers, notes)
}

/// The end of what a command said: enough to read why it failed.
fn the_end_of(output: &std::process::Output) -> String {
    let said = [&output.stderr, &output.stdout]
        .into_iter()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
        .find(|said| !said.is_empty())
        .unwrap_or_default();
    let lines: Vec<&str> = said.lines().rev().take(6).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

const THE_INSTALLER_BRINGS_IT: &str = "Podman is here without everything it runs its machine \
    with. Podman's own installer puts that in place: run it, then look again.";

/// What a person can do about a failure the product recognises.
fn hint_for(said: &str) -> Option<String> {
    let said = said.to_lowercase();
    ["qemu", "library not loaded", "vfkit", "gvproxy"]
        .iter()
        .any(|known| said.contains(known))
        .then(|| THE_INSTALLER_BRINGS_IT.to_owned())
}

/// What Podman is here without, when its machine is run by a program that
/// is looked for apart from it and that program does not start.
fn what_podman_lacks(plan: &crate::ProvisioningPlan) -> Option<String> {
    let runs = plan
        .steps
        .iter()
        .any(|step| matches!(step, crate::ProvisioningStep::Run { .. }));
    if !runs || plan.provider != "qemu" {
        return None;
    }
    let qemu = format!("qemu-system-{}", std::env::consts::ARCH);
    let starts = crate::environment::find_executable(&qemu).is_some_and(|found| {
        std::process::Command::new(found)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output()
            .is_ok_and(|output| output.status.success())
    });
    (!starts).then(|| {
        "Podman runs its machine with QEMU, and QEMU does not start on this computer.".to_owned()
    })
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

    /// The environments this host offers, each with what runs it, who is
    /// run in it and whether it can be chosen today.
    ///
    /// # Errors
    ///
    /// Fails when the agents cannot be listed.
    pub fn environments(&self) -> Result<Vec<EnvironmentOffered>, WorkbenchShellError> {
        let profiles = self.profiles()?;
        let look = self.machine();
        let run_in = |id: &str| -> Vec<String> {
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
        let podman = look
            .as_ref()
            .and_then(|look| look.podman.version.as_ref())
            .map_or_else(String::new, |version| format!("Podman {version}"));
        Ok(crate::environment_profiles()
            .into_iter()
            .map(|option| {
                let in_a_container = option.environment_profile_id == IN_A_CONTAINER;
                let why_not = match &look {
                    Some(look) if in_a_container => look.why_no_container().unwrap_or_default(),
                    _ => String::new(),
                };
                let used_by = run_in(&option.environment_profile_id);
                EnvironmentOffered {
                    option,
                    kind: if in_a_container { "Container" } else { "As is" }.into(),
                    machine: if in_a_container {
                        podman.clone()
                    } else {
                        machine.clone()
                    },
                    an_agent_gets: if in_a_container {
                        "A sealed container, with no network yet"
                    } else {
                        "Its own folder, not sealed"
                    }
                    .into(),
                    used_by,
                    available: why_not.is_empty(),
                    why_not,
                }
            })
            .collect())
    }

    /// Refuse an environment an agent cannot be run in today, in the words
    /// the look found, while the person is looking at the form.
    pub(super) fn check_environment(&self, chosen: &str) -> Result<(), WorkbenchShellError> {
        match self
            .environments()?
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

    /// What setting containers up on this machine would do. The plan is
    /// kept until it is agreed to by its id, or another is asked for.
    ///
    /// # Errors
    ///
    /// Not found when Podman is not on this machine; conflict while a
    /// set-up is running.
    pub async fn plan_containers(
        &self,
        wanted: MachineWanted,
    ) -> Result<SetUp, WorkbenchShellError> {
        if self.setting_up().is_some_and(|run| run.state == "running") {
            return Err(WorkbenchShellError::Conflict(
                "containers are being set up already".into(),
            ));
        }
        let (plan, lacks) = tokio::task::spawn_blocking(move || {
            let probe = crate::probe_podman();
            let defaults = crate::PodmanProvisioningOptions::default();
            let plan = crate::podman_provisioning_plan(
                &probe,
                &crate::PodmanProvisioningOptions {
                    machine_name: machine_named(&probe),
                    cpus: wanted.cpus.unwrap_or(defaults.cpus),
                    memory_mib: wanted.memory_mib.unwrap_or(defaults.memory_mib),
                    disk_gib: wanted.disk_gib.unwrap_or(defaults.disk_gib),
                    ..defaults
                },
            )?;
            let lacks = what_podman_lacks(&plan);
            Ok::<_, crate::EnvironmentError>((plan, lacks))
        })
        .await
        .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        .map_err(|error| match error {
            crate::EnvironmentError::BackendNotReady(_) => {
                WorkbenchShellError::NotFound("Podman is not on this computer".into())
            }
            other => WorkbenchShellError::Failed(other.to_string()),
        })?;
        let effects = plan.effects.clone();
        self.offer_containers(plan)?;
        // Podman does not say what this computer cannot give; the look does.
        let (mut blockers, notes) = self
            .machine()
            .filter(|_| effects.creates_virtual_machine)
            .map_or_else(Default::default, |look| {
                what_the_machine_says(&look, &effects)
            });
        self.set_up_moved(|shown| {
            if let Some(lacks) = lacks {
                // Nothing is downloaded for a machine that cannot start.
                blockers.insert(0, lacks);
                shown.hint = Some(THE_INSTALLER_BRINGS_IT.to_owned());
            }
            shown.blockers.extend(blockers);
            shown.effects.extend(notes);
        });
        self.setting_up()
            .ok_or_else(|| WorkbenchShellError::Failed("the plan was lost".into()))
    }

    /// Offer a plan to set containers up: it is kept until it is agreed to
    /// by its id, or another is offered.
    ///
    /// # Errors
    ///
    /// Conflict while a set-up is running.
    pub fn offer_containers(
        &self,
        plan: crate::ProvisioningPlan,
    ) -> Result<SetUp, WorkbenchShellError> {
        let shown = in_words(&plan);
        let mut kept = self
            .container_setup
            .lock()
            .map_err(|_| WorkbenchShellError::Failed("the plan was lost".into()))?;
        if kept.as_ref().is_some_and(|(_, run)| run.state == "running") {
            return Err(WorkbenchShellError::Conflict(
                "containers are being set up already".into(),
            ));
        }
        *kept = Some((plan, shown.clone()));
        Ok(shown)
    }

    /// The set-up that was offered, is running or has ended.
    #[must_use]
    pub fn setting_up(&self) -> Option<SetUp> {
        self.container_setup
            .lock()
            .ok()
            .and_then(|kept| kept.as_ref().map(|(_, shown)| shown.clone()))
    }

    fn set_up_moved(&self, change: impl FnOnce(&mut SetUp)) {
        if let Ok(mut kept) = self.container_setup.lock()
            && let Some((_, shown)) = kept.as_mut()
        {
            change(shown);
        }
    }

    /// Do what the plan a person agreed to says. It answers at once; how
    /// it goes is read from [`Self::setting_up`].
    ///
    /// # Errors
    ///
    /// Conflict for a plan that is not the one offered, one that cannot be
    /// done as asked, and a set-up that is running already.
    pub fn set_containers_up(
        self: &Arc<Self>,
        plan_id: &str,
    ) -> Result<SetUp, WorkbenchShellError> {
        let plan = {
            let mut kept = self
                .container_setup
                .lock()
                .map_err(|_| WorkbenchShellError::Failed("the plan was lost".into()))?;
            let Some((plan, shown)) = kept.as_mut() else {
                return Err(WorkbenchShellError::Conflict(
                    "nothing was offered to agree to".into(),
                ));
            };
            if plan.plan_id != plan_id || shown.state != "offered" {
                return Err(WorkbenchShellError::Conflict(
                    "that is not the plan that was offered; read it again".into(),
                ));
            }
            if !shown.blockers.is_empty() {
                return Err(WorkbenchShellError::Conflict(shown.blockers.join("; ")));
            }
            shown.state = "running".into();
            plan.clone()
        };
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut at = 0;
            let mut failed = None;
            for step in &plan.steps {
                let crate::ProvisioningStep::Run {
                    executable, args, ..
                } = step
                else {
                    continue;
                };
                state.set_up_moved(|shown| shown.steps[at].state = "running".into());
                let (executable, args) = (executable.clone(), args.clone());
                let ran = tokio::task::spawn_blocking(move || {
                    std::process::Command::new(&executable)
                        .args(&args)
                        .stdin(std::process::Stdio::null())
                        .output()
                })
                .await;
                let said = match ran {
                    Ok(Ok(output)) if output.status.success() => Ok(the_end_of(&output)),
                    Ok(Ok(output)) => Err(the_end_of(&output)),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(error) => Err(error.to_string()),
                };
                let went_well = said.is_ok();
                let said = said.unwrap_or_else(|said| said);
                state.set_up_moved(|shown| {
                    shown.steps[at].state = if went_well { "done" } else { "failed" }.into();
                    shown.steps[at].said.clone_from(&said);
                });
                if !went_well {
                    failed = Some(said);
                    break;
                }
                at += 1;
            }
            // Whatever the commands said, what counts is what is found.
            let found = state.look_at_the_machine().await;
            let can = found
                .as_ref()
                .is_ok_and(crate::MachineLook::a_container_can_start);
            let why_not = failed.or_else(|| {
                (!can).then(|| match &found {
                    Ok(look) => look.why_no_container().unwrap_or_default(),
                    Err(error) => error.to_string(),
                })
            });
            state.set_up_moved(|shown| match why_not {
                None => {
                    shown.state = "done".into();
                    shown.said = "Containers can be started on this computer now.".into();
                }
                Some(why) => {
                    shown.state = "failed".into();
                    shown.hint = hint_for(&why);
                    shown.said = why;
                }
            });
        });
        self.setting_up()
            .ok_or_else(|| WorkbenchShellError::Failed("the plan was lost".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::{hint_for, what_the_machine_says};
    use crate::{ContainerEngineLook, EngineStanding, MachineLook, ProvisioningEffects};

    const GIGABYTE: u64 = 1 << 30;

    fn a_machine(memory_free: u64, disk_free: u64) -> MachineLook {
        let engine = ContainerEngineLook {
            standing: EngineStanding::NotReady,
            version: None,
            said: "no machine is set up".into(),
        };
        MachineLook {
            looked_ms: 0,
            system: "macOS".into(),
            architecture: "x86_64".into(),
            processors: 4,
            memory_bytes: 8 * GIGABYTE,
            memory_free_bytes: memory_free,
            disk_free_bytes: Some(disk_free),
            podman: engine.clone(),
            docker: engine,
        }
    }

    fn a_machine_of(cpus: u16, memory_gib: u64, disk_gib: u64) -> ProvisioningEffects {
        ProvisioningEffects {
            creates_virtual_machine: true,
            may_download_machine_image: true,
            requires_privilege_elevation: false,
            may_require_restart: false,
            cpu_count: cpus,
            memory_mib: memory_gib * 1024,
            disk_gib,
            rootful: false,
            user_mode_networking: false,
            changes_shared_wsl_networking: false,
            uses_provider_default_host_mounts: true,
        }
    }

    #[test]
    fn a_machine_is_not_promised_what_this_computer_lacks() {
        let roomy = a_machine(6 * GIGABYTE, 200 * GIGABYTE);
        assert_eq!(
            what_the_machine_says(&roomy, &a_machine_of(2, 2, 20)),
            (Vec::new(), Vec::new())
        );

        let (blockers, _) = what_the_machine_says(&roomy, &a_machine_of(8, 8, 20));
        assert_eq!(blockers.len(), 2, "{blockers:?}");
        assert!(blockers[0].contains("4 processors"));
        assert!(blockers[1].contains("8 GB of memory"));

        // A disk smaller than the ceiling is said, not refused: the machine
        // takes what it uses.
        let (blockers, notes) =
            what_the_machine_says(&a_machine(GIGABYTE, 10 * GIGABYTE), &a_machine_of(2, 2, 20));
        assert!(blockers.is_empty(), "{blockers:?}");
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].contains("1 GB of memory are free"));
        assert!(notes[1].contains("10 GB are free"));

        let (blockers, _) = what_the_machine_says(
            &a_machine(6 * GIGABYTE, 3 * GIGABYTE),
            &a_machine_of(2, 2, 20),
        );
        assert!(blockers[0].contains("Only 3 GB are free"), "{blockers:?}");
    }

    #[test]
    fn what_to_do_is_said_for_a_failure_that_is_known() {
        assert!(
            hint_for("Error: qemu exited unexpectedly, dyld: Library not loaded: /opt/podman")
                .is_some()
        );
        assert!(hint_for("Error: machine swem: VM already exists").is_none());
    }
}
