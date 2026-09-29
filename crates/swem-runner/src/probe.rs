//! A look at the machine the runner is in.
//!
//! Nothing is offered that was not looked at. This is the look from
//! inside: which system, who the agent is here, whether the folder it is
//! to work in can be written, and which of the programs asked about are
//! here.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

pub const RUNNER_PROBE_SCHEMA: &str = "swem:runner-probe@0.1";

/// What the product asks to be looked at.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProbeRequest {
    /// The folder the agent is to work in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    /// The programs asked about, by the names they are started by.
    #[serde(default)]
    pub programs: Vec<String>,
}

/// A program as it was found.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProgramFound {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// The first line of what it says of its version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

/// The machine as it was found from inside.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ProbeAnswer {
    pub schema: String,
    pub runner: String,
    pub system: String,
    pub architecture: String,
    /// The C library programs here are linked against, when it is told:
    /// `musl` or `glibc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub libc: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<PathBuf>,
    pub processors: usize,
    /// Whether the folder the agent is to work in is there and can be
    /// written; nothing when none was asked about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_writable: Option<bool>,
    pub programs: Vec<ProgramFound>,
}

fn said(program: &Path, given: &[&str]) -> Option<String> {
    let output = Command::new(program)
        .args(given)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let text = if output.stdout.is_empty() {
        output.stderr
    } else {
        output.stdout
    };
    String::from_utf8_lossy(&text)
        .lines()
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
}

fn found(name: &str) -> Option<PathBuf> {
    // A name, not a path: nothing is looked for outside the search path.
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|folder| folder.join(name))
        .find(|candidate| candidate.is_file())
}

fn libc() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let musl = ["/lib", "/usr/lib"].iter().any(|folder| {
        std::fs::read_dir(folder).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|entry| entry.file_name().to_string_lossy().starts_with("ld-musl-"))
        })
    });
    Some(if musl { "musl" } else { "glibc" }.to_owned())
}

fn writable(folder: &Path) -> bool {
    if !folder.is_dir() {
        return false;
    }
    let trial = folder.join(format!(".swem-look-{}", std::process::id()));
    let could = std::fs::write(&trial, b"").is_ok();
    let _ = std::fs::remove_file(&trial);
    could
}

/// Look at the machine.
#[must_use]
pub fn look(request: &ProbeRequest) -> ProbeAnswer {
    ProbeAnswer {
        schema: RUNNER_PROBE_SCHEMA.into(),
        runner: env!("CARGO_PKG_VERSION").into(),
        system: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        libc: libc(),
        user: found("id").and_then(|id| said(&id, &["-un"])),
        home: std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from),
        processors: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        workspace_writable: request.workspace.as_deref().map(writable),
        programs: request
            .programs
            .iter()
            .map(|name| {
                let path = found(name);
                ProgramFound {
                    name: name.clone(),
                    version: path.as_deref().and_then(|path| said(path, &["--version"])),
                    path,
                }
            })
            .collect(),
    }
}
