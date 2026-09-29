//! Starting a program in an agent's machine.
//!
//! What is started, where and with which variables is read from a header,
//! never from the command line: a key given to an engine is in nobody's
//! list of processes. The header is one line on standard input, and what
//! follows it on standard input is the program's own; or it is a file,
//! read once and removed, for a program that is given a terminal.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

pub const RUNNER_EXEC_SCHEMA: &str = "swem:runner-exec@0.1";

/// A header is a line; one longer than this is not a header.
const LONGEST_HEADER: usize = 1024 * 1024;

/// What is started.
#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecHeader {
    /// The program and what it is given.
    pub argv: Vec<String>,
    /// Where it is started; where the runner is when nothing is said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// The variables it is given, a key among them.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// The variables of the machine it keeps; all of them when nothing is
    /// said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep: Option<Vec<String>>,
}

// What a header holds is not printed: a key is among it.
impl std::fmt::Debug for ExecHeader {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExecHeader")
            .field("argv", &self.argv)
            .field("cwd", &self.cwd)
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("keep", &self.keep)
            .finish()
    }
}

impl ExecHeader {
    /// The header as it is sent: one line.
    ///
    /// # Panics
    ///
    /// Never: a header is strings.
    #[must_use]
    pub fn as_a_line(&self) -> Vec<u8> {
        let mut line = serde_json::to_vec(self).expect("a header is strings");
        line.push(b'\n');
        line
    }

    /// The program, ready to be started as the header says.
    ///
    /// # Errors
    ///
    /// A header that names no program.
    pub fn command(&self) -> Result<Command, String> {
        let (program, given) = self
            .argv
            .split_first()
            .ok_or_else(|| "the header names no program".to_owned())?;
        let mut command = Command::new(program);
        command.args(given);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        if let Some(keep) = &self.keep {
            command.env_clear();
            for name in keep {
                if let Some(value) = std::env::var_os(name) {
                    command.env(name, value);
                }
            }
        }
        command.envs(&self.env);
        Ok(command)
    }
}

/// Read the header's line from a stream and nothing after it: what
/// follows is the program's. The stream is read a byte at a time for that.
///
/// # Errors
///
/// A stream that ends before the line does, a line that is too long, and
/// one that is not a header.
pub fn header_from(stream: &mut impl Read) -> Result<ExecHeader, String> {
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return Err("what is started was not said".into()),
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => line.push(byte[0]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(format!("what is started cannot be read: {error}")),
        }
        if line.len() > LONGEST_HEADER {
            return Err("what is started is said in too many words".into());
        }
    }
    serde_json::from_slice(&line)
        .map_err(|error| format!("what is started cannot be read: {error}"))
}

/// Read the header from a file and remove the file: it is read once.
///
/// # Errors
///
/// A file that is not there or is not a header.
pub fn header_at(path: &Path) -> Result<ExecHeader, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()));
    // Removed whether or not it could be read: a key may be in it.
    let _ = std::fs::remove_file(path);
    serde_json::from_slice(&bytes?)
        .map_err(|error| format!("what is started cannot be read: {error}"))
}

/// Become the program. On a system where a process can be replaced this
/// returns only when it could not be; elsewhere the program is started
/// and waited for, and what it ended with is returned.
///
/// # Errors
///
/// A header that names no program, and a program that cannot be started.
pub fn become_it(header: &ExecHeader) -> Result<i32, String> {
    let mut command = header.command()?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(format!("{}: {error}", header.argv[0]))
    }
    #[cfg(not(unix))]
    {
        let status = command
            .status()
            .map_err(|error| format!("{}: {error}", header.argv[0]))?;
        Ok(status.code().unwrap_or(1))
    }
}
