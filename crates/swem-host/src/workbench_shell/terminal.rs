//! A real terminal in the product, running in one profile's environment.
//!
//! An agent is not configured until a person can sign it in, and many agents
//! are signed in at a prompt - a device-code flow, a browser handshake, a
//! `login` subcommand. Telling a person to open a second terminal, find the
//! right working directory and export the right variables by hand is telling
//! them the product does not do its job. So the product has a terminal, and it
//! runs where the agent runs: the profile's workspace as the working
//! directory, the profile's agent home as `HOME`, the profile's typed vault
//! and credential bindings in the environment.
//!
//! The bytes travel over the same long poll the rest of the shell uses rather
//! than a second protocol: output is a sequence of chunks, a reader waits for
//! the next one, and input is a POST. On a loopback socket that is a keystroke
//! round trip, and it keeps one transport, one gate and one test harness.
//!
//! Nothing here is recorded. A terminal is a live thing between a person and a
//! process; its bytes are not journal events, and a secret typed at a prompt
//! must not become a record.
use std::collections::BTreeMap;
use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use portable_pty::{CommandBuilder, MasterPty, PtySize, native_pty_system};
use serde::{Deserialize, Serialize};

use super::{PersonalAgentProfile, WorkbenchShellError, credential_environment, nanos_now};

/// The shell a person gets when they name none: the one their environment
/// names, else the platform's own. A literal `/bin/sh` opened nothing on
/// Windows and the terminal printed a `CreateProcessW` failure instead.
fn default_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        return shell;
    }
    if cfg!(windows) {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into())
    } else {
        "/bin/sh".into()
    }
}

/// How much output one terminal keeps for a reader that fell behind, when
/// nobody asked for less. Enough for a screen's worth of history many times
/// over; a person who was away longer than that is told bytes were dropped
/// rather than shown a hole. ACP lets an agent ask for a smaller window
/// (`outputByteLimit`), and a terminal opened that way keeps exactly that.
const OUTPUT_LIMIT: usize = 512 * 1024;

/// Who opened a terminal. A person reading the list needs to tell their own
/// shell from the one their agent started while they were not looking.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalOpener {
    Person,
    Agent,
}

impl TerminalOpener {
    const fn word(self) -> &'static str {
        match self {
            Self::Person => "person",
            Self::Agent => "agent",
        }
    }
}

/// How a terminal's process ended, in both readings at once.
///
/// `words` is the one a person gets, and is the only one that ever reached a
/// screen. The other two are what ACP asks a client for: an agent that ran a
/// build decides what to do next from the code, and "exited with code 2"
/// parsed back out of a sentence would be a second source of truth for the
/// same fact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitReport {
    pub words: String,
    pub code: Option<u32>,
    pub signal: Option<String>,
}

/// One terminal, as a person sees it in the list.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalView {
    pub terminal_id: String,
    pub profile_id: String,
    /// What is running: the shell (or command) this terminal started.
    pub command: String,
    /// What it was given. Here and not in the record: this is the live list a
    /// person reads to see what their agent is running, and they can read the
    /// terminal itself anyway; the record is the thing that outlives the
    /// moment, so it keeps the program alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// `person` or `agent`.
    pub opened_by: String,
    pub cols: u16,
    pub rows: u16,
    /// Present once the process ended, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<String>,
}

/// One run of output, with the sequence a reader continues from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalOutput {
    /// The sequence to ask for next.
    pub next: u64,
    /// Standard base64 of the bytes between the requested sequence and `next`.
    pub bytes: String,
    /// True when output older than the requested sequence was already dropped.
    pub dropped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<String>,
}

struct OutputBuffer {
    /// The sequence of the first byte still held.
    first: u64,
    /// The sequence just past the last byte held.
    next: u64,
    /// How much is kept before the oldest bytes are dropped.
    limit: usize,
    bytes: std::collections::VecDeque<u8>,
}

impl OutputBuffer {
    fn new(limit: usize) -> Self {
        Self {
            first: 0,
            next: 0,
            limit,
            bytes: std::collections::VecDeque::new(),
        }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.bytes.extend(chunk.iter().copied());
        self.next += chunk.len() as u64;
        while self.bytes.len() > self.limit {
            let overflow = self.bytes.len() - OUTPUT_LIMIT;
            self.bytes.drain(..overflow);
            self.first += overflow as u64;
        }
    }

    fn since(&self, after: u64) -> (Vec<u8>, bool) {
        let dropped = after < self.first;
        let from = after.max(self.first);
        let skip = usize::try_from(from - self.first).unwrap_or(usize::MAX);
        (self.bytes.iter().skip(skip).copied().collect(), dropped)
    }

    /// Everything still held, and whether anything older was already dropped.
    fn all(&self) -> (Vec<u8>, bool) {
        (self.bytes.iter().copied().collect(), self.first > 0)
    }
}

struct TerminalSession {
    terminal_id: String,
    profile_id: String,
    command: String,
    args: Vec<String>,
    opened_by: TerminalOpener,
    size: Mutex<(u16, u16)>,
    /// The pty's own handle, behind a lock only so the session is shareable:
    /// `MasterPty` is `Send` but not `Sync`.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn std::io::Write + Send>>,
    output: Mutex<OutputBuffer>,
    exit: Mutex<Option<ExitReport>>,
    changed: tokio::sync::Notify,
    child: Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl TerminalSession {
    fn ended(&self) -> Option<ExitReport> {
        self.exit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn view(&self) -> TerminalView {
        let (cols, rows) = *self
            .size
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        TerminalView {
            terminal_id: self.terminal_id.clone(),
            profile_id: self.profile_id.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            opened_by: self.opened_by.word().to_owned(),
            cols,
            rows,
            exit: self.ended().map(|report| report.words),
        }
    }
}

/// Every terminal this host has open.
#[derive(Default)]
pub struct Terminals {
    open: tokio::sync::Mutex<BTreeMap<String, Arc<TerminalSession>>>,
    next: AtomicU64,
}

impl std::fmt::Debug for Terminals {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Terminals")
    }
}

/// What a person asks for when they open a terminal.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenTerminalBody {
    pub profile_id: String,
    /// The program to run; empty means the person's login shell.
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_cols")]
    pub cols: u16,
    #[serde(default = "default_rows")]
    pub rows: u16,
    /// Where to run, when it is not the profile's workspace. Wherever it
    /// comes from it is checked against that workspace before anything
    /// starts: this is the one place the rule is written, so neither door can
    /// move it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Added to the environment after the profile's own bindings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// How much output to keep, when less than the default is wanted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_byte_limit: Option<u64>,
}

const fn default_cols() -> u16 {
    80
}
const fn default_rows() -> u16 {
    24
}

/// Read one pty until it closes, then say how its process ended.
///
/// This is a thread rather than a task because the read is blocking; what it
/// hands the async side is finished chunks and a wake.
fn pump_output(session: &TerminalSession, reader: &mut dyn Read) {
    let mut buffer = [0_u8; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                session
                    .output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(&buffer[..read]);
                session.changed.notify_waiters();
            }
        }
    }
    let status = {
        let mut child = session
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        child.as_mut().map(|child| child.wait())
    };
    let report = match status {
        Some(Ok(status)) if status.success() => ExitReport {
            words: "finished".to_owned(),
            code: Some(status.exit_code()),
            signal: None,
        },
        Some(Ok(status)) => {
            let signal = status.signal().map(str::to_owned);
            ExitReport {
                words: signal.as_ref().map_or_else(
                    || format!("exited with code {}", status.exit_code()),
                    |signal| format!("stopped by {signal}"),
                ),
                // ACP: a process the signal ended has no exit code.
                code: signal.is_none().then(|| status.exit_code()),
                signal,
            }
        }
        Some(Err(error)) => ExitReport {
            words: format!("ended: {error}"),
            code: None,
            signal: None,
        },
        None => ExitReport {
            words: "ended".to_owned(),
            code: None,
            signal: None,
        },
    };
    *session
        .exit
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(report);
    session.changed.notify_waiters();
}

/// Where a terminal runs. The profile's workspace, or a directory inside it.
///
/// The check is here rather than at either door because both doors reach it:
/// a person opens a terminal from the page, and an agent asks for one over
/// ACP. Neither gets to name the boundary, so neither can widen it.
///
/// The ACP door also reads it before it asks the person anything, so a
/// command this rule would refuse is never put to them.
pub(crate) fn working_directory(
    profile: &PersonalAgentProfile,
    asked: Option<&Path>,
) -> Result<PathBuf, WorkbenchShellError> {
    let Some(asked) = asked else {
        return Ok(profile.workspace.clone());
    };
    if asked == profile.workspace || crate::session::inside_boundary(asked, &profile.workspace) {
        return Ok(asked.to_path_buf());
    }
    Err(WorkbenchShellError::Invalid(
        crate::session::OUTSIDE_THE_BOUNDARY.to_owned(),
    ))
}

impl Terminals {
    async fn session(
        &self,
        terminal_id: &str,
    ) -> Result<Arc<TerminalSession>, WorkbenchShellError> {
        self.open
            .lock()
            .await
            .get(terminal_id)
            .cloned()
            .ok_or_else(|| {
                WorkbenchShellError::NotFound(format!("no terminal named {terminal_id}"))
            })
    }

    /// Every open terminal, oldest first.
    pub async fn list(&self) -> Vec<TerminalView> {
        self.open
            .lock()
            .await
            .values()
            .map(|session| session.view())
            .collect()
    }

    /// Start one terminal in the environment of one profile.
    ///
    /// # Errors
    ///
    /// Fails when the pty cannot be opened or the program cannot be started.
    pub async fn open(
        &self,
        profile: &PersonalAgentProfile,
        secrets: BTreeMap<String, String>,
        body: &OpenTerminalBody,
        opened_by: TerminalOpener,
    ) -> Result<TerminalView, WorkbenchShellError> {
        let cwd = working_directory(profile, body.cwd.as_deref())?;
        let cols = body.cols.clamp(20, 500);
        let rows = body.rows.clamp(5, 200);
        let pty = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let program = if body.command.trim().is_empty() {
            default_shell()
        } else {
            body.command.trim().to_owned()
        };
        let mut command = CommandBuilder::new(&program);
        for argument in &body.args {
            command.arg(argument);
        }
        command.cwd(&cwd);
        // The agent's own home, so a sign-in performed here is a sign-in the
        // agent finds when it starts.
        command.env("HOME", profile.agent_home.as_os_str());
        command.env("TERM", "xterm-256color");
        for (name, value) in credential_environment(profile, secrets)? {
            command.env(name, value);
        }
        for (name, value) in &body.env {
            command.env(name, value);
        }
        let child = pty.slave.spawn_command(command).map_err(|error| {
            WorkbenchShellError::Failed(format!("{program} did not start: {error}"))
        })?;
        drop(pty.slave);
        let writer = pty
            .master
            .take_writer()
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let mut reader = pty
            .master
            .try_clone_reader()
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let limit = body
            .output_byte_limit
            .and_then(|limit| usize::try_from(limit).ok())
            .map_or(OUTPUT_LIMIT, |limit| limit.clamp(1024, OUTPUT_LIMIT));
        let terminal_id = format!(
            "t{}-{}",
            self.next.fetch_add(1, Ordering::SeqCst),
            nanos_now()
        );
        let session = Arc::new(TerminalSession {
            terminal_id: terminal_id.clone(),
            profile_id: profile.profile_id.clone(),
            command: program,
            args: body.args.clone(),
            opened_by,
            size: Mutex::new((cols, rows)),
            master: Mutex::new(pty.master),
            writer: Mutex::new(writer),
            output: Mutex::new(OutputBuffer::new(limit)),
            exit: Mutex::new(None),
            changed: tokio::sync::Notify::new(),
            child: Mutex::new(Some(child)),
        });
        // The pty read is blocking, so it lives on its own thread and hands
        // finished chunks to the async side by waking the waiters.
        let pump = Arc::clone(&session);
        std::thread::spawn(move || pump_output(&pump, &mut reader));
        let view = session.view();
        self.open.lock().await.insert(terminal_id, session);
        Ok(view)
    }

    /// Output from `after` onwards, waiting up to `wait` for the next byte.
    ///
    /// # Errors
    ///
    /// Not found for a terminal that was closed or never existed.
    pub async fn output(
        &self,
        terminal_id: &str,
        after: u64,
        wait: Duration,
    ) -> Result<TerminalOutput, WorkbenchShellError> {
        let session = self.session(terminal_id).await?;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let waiting = session.changed.notified();
            {
                let output = session
                    .output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let exit = session.ended().map(|report| report.words);
                if output.next > after || exit.is_some() {
                    let (bytes, dropped) = output.since(after);
                    return Ok(TerminalOutput {
                        next: output.next,
                        bytes: BASE64.encode(bytes),
                        dropped,
                        exit,
                    });
                }
            }
            if tokio::time::timeout_at(deadline, waiting).await.is_err() {
                let output = session
                    .output
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                return Ok(TerminalOutput {
                    next: output.next,
                    bytes: String::new(),
                    dropped: after < output.first,
                    exit: None,
                });
            }
        }
    }

    /// What a person typed, as exact bytes.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal, conflict once the process ended.
    pub async fn input(&self, terminal_id: &str, bytes: &[u8]) -> Result<(), WorkbenchShellError> {
        let session = self.session(terminal_id).await?;
        if let Some(report) = session.ended() {
            return Err(WorkbenchShellError::Conflict(format!(
                "this terminal {}; open another one",
                report.words
            )));
        }
        let mut writer = session
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        writer
            .write_all(bytes)
            .and_then(|()| writer.flush())
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Tell the process the window changed shape.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal.
    pub async fn resize(
        &self,
        terminal_id: &str,
        cols: u16,
        rows: u16,
    ) -> Result<TerminalView, WorkbenchShellError> {
        let session = self.session(terminal_id).await?;
        let cols = cols.clamp(20, 500);
        let rows = rows.clamp(5, 200);
        session
            .master
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        *session
            .size
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = (cols, rows);
        Ok(session.view())
    }

    /// Everything one terminal has said so far, its truncation flag, and how
    /// it ended if it has. This is the whole retained window rather than a
    /// continuation, because that is what ACP's `terminal/output` means: an
    /// agent asks what has happened, not what has happened since.
    ///
    /// # Errors
    ///
    /// Not found for a terminal that was released or never existed.
    pub async fn snapshot(
        &self,
        terminal_id: &str,
    ) -> Result<(String, bool, Option<ExitReport>), WorkbenchShellError> {
        let session = self.session(terminal_id).await?;
        let (bytes, truncated) = session
            .output
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .all();
        Ok((
            String::from_utf8_lossy(&bytes).into_owned(),
            truncated,
            session.ended(),
        ))
    }

    /// Wait until the process ends, however long that takes.
    ///
    /// There is no deadline here on purpose: the caller asked how it ended,
    /// and a made-up answer at some arbitrary moment would be worse than
    /// waiting. Whoever wants it to stop sooner kills it.
    ///
    /// # Errors
    ///
    /// Not found for a terminal that was released or never existed.
    pub async fn wait_for_exit(
        &self,
        terminal_id: &str,
    ) -> Result<ExitReport, WorkbenchShellError> {
        let session = self.session(terminal_id).await?;
        loop {
            let waiting = session.changed.notified();
            if let Some(report) = session.ended() {
                return Ok(report);
            }
            waiting.await;
        }
    }

    /// End the process but keep the terminal, so what it said can still be
    /// read. ACP separates killing from releasing for exactly this reason: an
    /// agent that killed a build still wants the output that made it decide.
    ///
    /// # Errors
    ///
    /// Not found for a terminal that was released or never existed.
    pub async fn kill(&self, terminal_id: &str) -> Result<(), WorkbenchShellError> {
        let session = self.session(terminal_id).await?;
        if let Some(child) = session
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            let _ = child.kill();
        }
        session.changed.notify_waiters();
        Ok(())
    }

    /// End one terminal and forget it.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal.
    pub async fn close(&self, terminal_id: &str) -> Result<(), WorkbenchShellError> {
        let session = self.open.lock().await.remove(terminal_id).ok_or_else(|| {
            WorkbenchShellError::NotFound(format!("no terminal named {terminal_id}"))
        })?;
        if let Some(child) = session
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_mut()
        {
            let _ = child.kill();
        }
        session.changed.notify_waiters();
        Ok(())
    }

    /// End every terminal, at shutdown.
    pub async fn close_all(&self) {
        let sessions: Vec<_> = self.open.lock().await.values().cloned().collect();
        self.open.lock().await.clear();
        for session in sessions {
            if let Some(child) = session
                .child
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_mut()
            {
                let _ = child.kill();
            }
            session.changed.notify_waiters();
        }
    }
}
