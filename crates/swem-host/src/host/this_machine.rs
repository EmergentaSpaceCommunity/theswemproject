//! The machine the Workbench runs on: what it is, what it has, and whether
//! a container can be started on it.
//!
//! The look starts nothing and changes nothing: it asks the system what it
//! is and asks each container engine whether it answers.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::environment::{BackendProbe, BackendProbeStatus, find_executable, probe_podman};

/// How long an engine is given to say whether it is there.
const ANSWER_WITHIN: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStanding {
    /// The program is not on this machine.
    NotFound,
    /// The program is here and its engine does not answer.
    NotReady,
    /// A container can be started with it.
    Ready,
}

/// One container engine, as it was found.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContainerEngineLook {
    pub standing: EngineStanding,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// How it stands, in words a page can say.
    pub said: String,
}

impl ContainerEngineLook {
    fn not_found() -> Self {
        Self {
            standing: EngineStanding::NotFound,
            version: None,
            said: "Not found".into(),
        }
    }
}

/// What was found when this machine was looked at.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MachineLook {
    pub looked_ms: u64,
    /// The system and its version, as it names itself.
    pub system: String,
    pub architecture: String,
    pub processors: usize,
    pub memory_bytes: u64,
    pub memory_free_bytes: u64,
    /// Free where the Workbench keeps its data; nothing when the system
    /// did not say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_free_bytes: Option<u64>,
    pub podman: ContainerEngineLook,
    pub docker: ContainerEngineLook,
}

impl MachineLook {
    /// Whether an agent can be put in a container here today. Podman is the
    /// engine the harness runs agents with; Docker is looked at and said,
    /// and runs nothing yet.
    #[must_use]
    pub fn a_container_can_start(&self) -> bool {
        self.podman.standing == EngineStanding::Ready
    }

    /// Why an agent cannot be put in a container here, in words; nothing
    /// when it can.
    #[must_use]
    pub fn why_no_container(&self) -> Option<String> {
        match self.podman.standing {
            EngineStanding::Ready => None,
            EngineStanding::NotFound => Some("Podman is not on this computer.".into()),
            EngineStanding::NotReady => Some(format!("Podman is here: {}.", self.podman.said)),
        }
    }
}

fn version_in(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|word| word.starts_with(|first: char| first.is_ascii_digit()))
        .map(|word| word.trim_end_matches(',').to_owned())
}

/// How Podman stands, from the harness's own probe of it.
fn podman_found(probe: &BackendProbe) -> ContainerEngineLook {
    let version = probe.version.as_deref().and_then(version_in);
    match probe.status {
        BackendProbeStatus::Absent => ContainerEngineLook::not_found(),
        BackendProbeStatus::Ready => ContainerEngineLook {
            standing: EngineStanding::Ready,
            said: "Ready".into(),
            version,
        },
        BackendProbeStatus::Incompatible => ContainerEngineLook {
            standing: EngineStanding::NotReady,
            said: "it does not say its version".into(),
            version,
        },
        BackendProbeStatus::InstalledNotReady => {
            let machines = probe.topology["machines"].as_array();
            let said = match machines {
                Some(machines) if machines.is_empty() => "no machine is set up",
                Some(machines)
                    if !machines
                        .iter()
                        .any(|machine| machine["Running"].as_bool() == Some(true)) =>
                {
                    "its machine is stopped"
                }
                // Where Podman needs a machine and would not list any, it
                // has none.
                None if cfg!(any(target_os = "macos", windows)) => "no machine is set up",
                _ => "its engine does not answer",
            };
            ContainerEngineLook {
                standing: EngineStanding::NotReady,
                said: said.into(),
                version,
            }
        }
    }
}

/// What a program printed, when it ended by itself in time and well.
fn answered(program: &Path, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let began = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = child.wait_with_output().ok()?;
                return status
                    .success()
                    .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned());
            }
            Ok(None) if began.elapsed() < ANSWER_WITHIN => {
                std::thread::sleep(Duration::from_millis(50));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn docker_found() -> ContainerEngineLook {
    let Some(docker) = find_executable("docker") else {
        return ContainerEngineLook::not_found();
    };
    let version = answered(&docker, &["--version"])
        .as_deref()
        .and_then(version_in);
    match answered(&docker, &["version", "--format", "{{.Server.Version}}"]) {
        Some(server) if !server.is_empty() => ContainerEngineLook {
            standing: EngineStanding::Ready,
            said: "Ready".into(),
            version: Some(server),
        },
        _ => ContainerEngineLook {
            standing: EngineStanding::NotReady,
            said: "its engine does not answer".into(),
            version,
        },
    }
}

/// Free space on the disk a path lives on: the disk mounted nearest to it.
fn free_at(path: &Path) -> Option<u64> {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    sysinfo::Disks::new_with_refreshed_list()
        .list()
        .iter()
        .filter(|disk| path.starts_with(disk.mount_point()))
        .max_by_key(|disk| disk.mount_point().components().count())
        .map(sysinfo::Disk::available_space)
}

/// Look at this machine. It takes as long as the engines take to answer,
/// so it is called from where waiting is allowed.
#[must_use]
pub fn look_at_this_machine(data_root: &Path) -> MachineLook {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    // The name a person knows the system by ("macOS 12.7"), and only
    // without one the kernel's.
    let named = sysinfo::System::long_os_version()
        .filter(|named| !named.trim().is_empty())
        .unwrap_or_else(|| {
            [sysinfo::System::name(), sysinfo::System::os_version()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
        });
    MachineLook {
        looked_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            }),
        system: if named.is_empty() {
            std::env::consts::OS.to_owned()
        } else {
            named
        },
        architecture: std::env::consts::ARCH.to_owned(),
        processors: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        memory_bytes: system.total_memory(),
        memory_free_bytes: system.available_memory(),
        disk_free_bytes: free_at(data_root),
        podman: podman_found(&probe_podman()),
        docker: docker_found(),
    }
}

fn kept_at(data_root: &Path) -> PathBuf {
    data_root.join("hosts").join("this-machine.json")
}

/// The look that was kept, when there is one that can be read.
#[must_use]
pub fn read_look(data_root: &Path) -> Option<MachineLook> {
    serde_json::from_slice(&std::fs::read(kept_at(data_root)).ok()?).ok()
}

/// Keep a look, so the next start can say what was found before it looks
/// again.
///
/// # Errors
///
/// Returns the I/O error of the step that failed.
pub fn write_look(data_root: &Path, look: &MachineLook) -> std::io::Result<()> {
    let path = kept_at(data_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::profile::write_atomically(&path, &serde_json::to_vec_pretty(look)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(status: BackendProbeStatus, machines: &serde_json::Value) -> BackendProbe {
        BackendProbe {
            status,
            version: Some("podman version 4.9.5".into()),
            topology: serde_json::json!({ "machines": machines }),
            ..BackendProbe::direct()
        }
    }

    #[test]
    fn podman_is_said_as_a_person_would_say_it() {
        let ready = podman_found(&probe(BackendProbeStatus::Ready, &serde_json::json!([])));
        assert_eq!(
            (ready.standing, ready.version.as_deref()),
            (EngineStanding::Ready, Some("4.9.5"))
        );
        let none = podman_found(&probe(
            BackendProbeStatus::InstalledNotReady,
            &serde_json::json!([]),
        ));
        assert_eq!(
            (none.standing, none.said.as_str()),
            (EngineStanding::NotReady, "no machine is set up")
        );
        let stopped = podman_found(&probe(
            BackendProbeStatus::InstalledNotReady,
            &serde_json::json!([{ "Name": "podman-machine-default", "Running": false }]),
        ));
        assert_eq!(stopped.said, "its machine is stopped");
        let absent = podman_found(&probe(BackendProbeStatus::Absent, &serde_json::Value::Null));
        assert_eq!(absent.standing, EngineStanding::NotFound);
    }

    #[test]
    fn this_machine_says_what_it_is_and_a_look_is_kept() {
        let root = std::env::temp_dir().join(format!("swem-look-{}", std::process::id()));
        std::fs::create_dir_all(&root).expect("create a root");
        let look = look_at_this_machine(&root);
        assert!(!look.system.is_empty() && !look.architecture.is_empty());
        assert!(look.processors >= 1);
        assert!(look.memory_bytes > 0 && look.memory_free_bytes <= look.memory_bytes);
        assert_eq!(
            look.a_container_can_start(),
            look.why_no_container().is_none()
        );
        assert_eq!(read_look(&root), None);
        write_look(&root, &look).expect("keep the look");
        assert_eq!(read_look(&root), Some(look));
        std::fs::remove_dir_all(root).ok();
    }
}
