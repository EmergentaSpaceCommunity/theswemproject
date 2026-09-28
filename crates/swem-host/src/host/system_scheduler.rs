//! The system's own scheduler as a keeper of time.
//!
//! It starts the product every minute; the product looks at what is due,
//! says it and leaves, or leaves at once when a Workbench keeps time. The
//! job holds the command, where the data is and the search path the
//! engines are found on. It holds no key.
//!
//! How it stands is read from the system each time, never from a record
//! of ours: a job somebody removed by hand is off.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// How often the system starts the product to look. A schedule is not to
/// the second, and a look that finds nothing opens the ledger and leaves.
pub const LOOKS_EVERY_SECONDS: u32 = 60;

/// What the system is asked to start.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemJob {
    /// What the system knows the job by; one per data root.
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    /// What the product is started with: where its data is and the search
    /// path. Never a key.
    pub environment: Vec<(String, String)>,
    /// Where what it says goes.
    pub log: PathBuf,
}

/// Which scheduler this system has.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemScheduler {
    Launchd,
    Systemd,
    /// None this product knows how to ask.
    None,
}

impl SystemScheduler {
    #[must_use]
    pub fn of_this_system() -> Self {
        if cfg!(target_os = "macos") {
            Self::Launchd
        } else if cfg!(target_os = "linux")
            && crate::environment::find_executable("systemctl").is_some()
        {
            Self::Systemd
        } else {
            Self::None
        }
    }

    /// What a person knows it by.
    #[must_use]
    pub fn called(self) -> &'static str {
        match self {
            Self::Launchd => "launchd",
            Self::Systemd => "systemd",
            Self::None => "",
        }
    }
}

/// How the system's scheduler stands for one data root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SystemSchedulerStanding {
    pub scheduler: SystemScheduler,
    /// The system holds the job and starts it.
    pub on: bool,
    /// What the job starts, when the system holds one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<PathBuf>,
    /// What is wrong with it, in words; empty when nothing is.
    pub said: String,
}

/// The name of the job for a data root: two products on one machine keep
/// their own time.
#[must_use]
pub fn job_name(data_root: &Path) -> String {
    let digest = Sha256::digest(data_root.to_string_lossy().as_bytes());
    let short = digest
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .concat();
    format!("swem.timekeeper.{short}")
}

/// The environment a job is given, from the one this process has: where
/// the data is, and the search path. Nothing else is carried over.
#[must_use]
pub fn carried_environment() -> Vec<(String, String)> {
    ["PATH", "XDG_DATA_HOME", "LOCALAPPDATA"]
        .into_iter()
        .filter_map(|name| {
            std::env::var(name)
                .ok()
                .filter(|value| !value.is_empty())
                .map(|value| (name.to_owned(), value))
        })
        .collect()
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The job as launchd reads it.
#[must_use]
pub fn launchd_plist(job: &SystemJob) -> String {
    let program = std::iter::once(job.program.to_string_lossy().into_owned())
        .chain(job.args.iter().cloned())
        .map(|value| format!("    <string>{}</string>\n", xml(&value)))
        .collect::<Vec<_>>()
        .concat();
    let environment = job
        .environment
        .iter()
        .map(|(name, value)| {
            format!(
                "    <key>{}</key>\n    <string>{}</string>\n",
                xml(name),
                xml(value)
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let log = xml(&job.log.to_string_lossy());
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n  \
         <key>Label</key>\n  <string>{name}</string>\n  \
         <key>ProgramArguments</key>\n  <array>\n{program}  </array>\n  \
         <key>EnvironmentVariables</key>\n  <dict>\n{environment}  </dict>\n  \
         <key>StartInterval</key>\n  <integer>{LOOKS_EVERY_SECONDS}</integer>\n  \
         <key>RunAtLoad</key>\n  <true/>\n  \
         <key>ProcessType</key>\n  <string>Background</string>\n  \
         <key>StandardOutPath</key>\n  <string>{log}</string>\n  \
         <key>StandardErrorPath</key>\n  <string>{log}</string>\n\
         </dict>\n</plist>\n",
        name = xml(&job.name),
    )
}

fn quoted(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The job as systemd reads it: what is started, and when.
#[must_use]
pub fn systemd_units(job: &SystemJob) -> (String, String) {
    let command: Vec<String> = std::iter::once(job.program.to_string_lossy().into_owned())
        .chain(job.args.iter().cloned())
        .map(|part| quoted(&part))
        .collect();
    let environment = job
        .environment
        .iter()
        .map(|(name, value)| format!("Environment={}\n", quoted(&format!("{name}={value}"))))
        .collect::<Vec<_>>()
        .concat();
    let service = format!(
        "[Unit]\nDescription=SWEM keeps time while the Workbench is closed\n\n\
         [Service]\nType=oneshot\nExecStart={}\n{environment}",
        command.join(" ")
    );
    let timer = format!(
        "[Unit]\nDescription=SWEM looks at what is due\n\n\
         [Timer]\nOnBootSec={LOOKS_EVERY_SECONDS}\nOnUnitInactiveSec={LOOKS_EVERY_SECONDS}\n\
         AccuracySec=1\n\n[Install]\nWantedBy=timers.target\n"
    );
    (service, timer)
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "this system does not say where your home is".to_owned())
}

fn launchd_file(name: &str) -> Result<PathBuf, String> {
    Ok(home()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{name}.plist")))
}

fn systemd_files(name: &str) -> Result<(PathBuf, PathBuf), String> {
    let units = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|root| !root.is_empty())
        .map_or(home()?.join(".config"), PathBuf::from)
        .join("systemd")
        .join("user");
    Ok((
        units.join(format!("{name}.service")),
        units.join(format!("{name}.timer")),
    ))
}

/// The part of launchd a person's own jobs live in.
#[cfg(unix)]
fn launchd_domain() -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;
    let home = home()?;
    let owner = fs::metadata(&home)
        .map_err(|error| format!("{}: {error}", home.display()))?
        .uid();
    Ok(format!("gui/{owner}"))
}

#[cfg(not(unix))]
fn launchd_domain() -> Result<String, String> {
    Err("launchd is not on this system".to_owned())
}

/// Run one of the system's own commands and say, when it failed, what it
/// said.
fn ask(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{program}: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let said = [&output.stderr, &output.stdout]
            .into_iter()
            .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
            .find(|said| !said.is_empty())
            .unwrap_or_else(|| format!("{program} {} failed", args.join(" ")));
        Err(said)
    }
}

fn put(path: &Path, text: &str) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("{}: {error}", path.display());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(failed)?;
    }
    fs::write(path, text).map_err(failed)
}

fn taken_away(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// Give the system the job. A job of the same name is replaced.
///
/// # Errors
///
/// What the system said when it would not take it, and a system whose
/// scheduler this product does not know how to ask.
pub fn turn_on(job: &SystemJob) -> Result<(), String> {
    match SystemScheduler::of_this_system() {
        SystemScheduler::Launchd => {
            let file = launchd_file(&job.name)?;
            let domain = launchd_domain()?;
            // One that is there under this name is taken out first; that
            // there was none is not a failure.
            let _ = ask("launchctl", &["bootout", &format!("{domain}/{}", job.name)]);
            put(&file, &launchd_plist(job))?;
            ask(
                "launchctl",
                &["bootstrap", &domain, &file.to_string_lossy()],
            )
            .map(|_| ())
        }
        SystemScheduler::Systemd => {
            let (service, timer) = systemd_files(&job.name)?;
            let (what, when) = systemd_units(job);
            put(&service, &what)?;
            put(&timer, &when)?;
            ask("systemctl", &["--user", "daemon-reload"])?;
            ask(
                "systemctl",
                &["--user", "enable", "--now", &format!("{}.timer", job.name)],
            )
            .map(|_| ())
        }
        SystemScheduler::None => {
            Err("this product does not know how to ask this system's scheduler".to_owned())
        }
    }
}

/// Take the job away from the system; nothing of it is left.
///
/// # Errors
///
/// A file of the job that cannot be removed.
pub fn turn_off(name: &str) -> Result<(), String> {
    match SystemScheduler::of_this_system() {
        SystemScheduler::Launchd => {
            if let Ok(domain) = launchd_domain() {
                let _ = ask("launchctl", &["bootout", &format!("{domain}/{name}")]);
            }
            taken_away(&launchd_file(name)?)
        }
        SystemScheduler::Systemd => {
            let _ = ask(
                "systemctl",
                &["--user", "disable", "--now", &format!("{name}.timer")],
            );
            let (service, timer) = systemd_files(name)?;
            taken_away(&timer)?;
            taken_away(&service)?;
            let _ = ask("systemctl", &["--user", "daemon-reload"]);
            Ok(())
        }
        SystemScheduler::None => Ok(()),
    }
}

/// The program a job of launchd starts, read from what launchd was given.
fn program_in(plist: &str) -> Option<PathBuf> {
    let after = plist.split("<key>ProgramArguments</key>").nth(1)?;
    let first = after.split("<string>").nth(1)?.split("</string>").next()?;
    Some(PathBuf::from(
        first
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    ))
}

/// The program a service of systemd starts.
fn program_of(service: &str) -> Option<PathBuf> {
    let line = service
        .lines()
        .find_map(|line| line.strip_prefix("ExecStart="))?;
    let first = line.strip_prefix('"')?.split('"').next()?;
    Some(PathBuf::from(first))
}

/// How the job stands, as the system has it now.
#[must_use]
pub fn standing(name: &str) -> SystemSchedulerStanding {
    let scheduler = SystemScheduler::of_this_system();
    let (held, program) = match scheduler {
        SystemScheduler::Launchd => {
            let held = launchd_domain()
                .and_then(|domain| ask("launchctl", &["print", &format!("{domain}/{name}")]))
                .is_ok();
            let program = launchd_file(name)
                .ok()
                .and_then(|file| fs::read_to_string(file).ok())
                .and_then(|plist| program_in(&plist));
            (held, program)
        }
        SystemScheduler::Systemd => {
            let held = ask(
                "systemctl",
                &["--user", "is-active", &format!("{name}.timer")],
            )
            .is_ok();
            let program = systemd_files(name)
                .ok()
                .and_then(|(service, _)| fs::read_to_string(service).ok())
                .and_then(|service| program_of(&service));
            (held, program)
        }
        SystemScheduler::None => (false, None),
    };
    let said = match (&program, held) {
        (Some(program), true) if !program.is_file() => {
            "It starts a SWEM that is no longer where it was. Turn it on again from this one."
                .to_owned()
        }
        _ => String::new(),
    };
    SystemSchedulerStanding {
        scheduler,
        on: held,
        program: program.filter(|_| held),
        said,
    }
}

#[cfg(test)]
mod tests {
    use super::{SystemJob, job_name, launchd_plist, program_in, program_of, systemd_units};
    use std::path::{Path, PathBuf};

    fn a_job() -> SystemJob {
        SystemJob {
            name: job_name(Path::new("/data/of/a person")),
            program: PathBuf::from("/opt/swem & co/swem"),
            args: vec!["time".into(), "keep".into()],
            environment: vec![
                ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                ("XDG_DATA_HOME".into(), "/data/of/a person".into()),
            ],
            log: PathBuf::from("/data/of/a person/time/system.log"),
        }
    }

    #[test]
    fn a_data_root_has_a_job_of_its_own() {
        let name = job_name(Path::new("/data/one"));
        assert!(name.starts_with("swem.timekeeper."));
        assert_eq!(name, job_name(Path::new("/data/one")));
        assert_ne!(name, job_name(Path::new("/data/two")));
    }

    #[test]
    fn launchd_is_given_the_command_and_no_more_than_it_needs() {
        let job = a_job();
        let plist = launchd_plist(&job);
        assert!(plist.contains(&format!("<string>{}</string>", job.name)));
        assert!(plist.contains("<string>/opt/swem &amp; co/swem</string>"));
        assert!(plist.contains("<string>time</string>\n    <string>keep</string>"));
        assert!(plist.contains("<key>StartInterval</key>\n  <integer>60</integer>"));
        assert!(plist.contains("<key>XDG_DATA_HOME</key>"));
        assert_eq!(program_in(&plist), Some(job.program));
    }

    #[test]
    fn systemd_is_given_a_service_and_a_timer() {
        let job = a_job();
        let (service, timer) = systemd_units(&job);
        assert!(service.contains("ExecStart=\"/opt/swem & co/swem\" \"time\" \"keep\""));
        assert!(service.contains("Environment=\"XDG_DATA_HOME=/data/of/a person\""));
        assert!(service.contains("Type=oneshot"));
        assert!(timer.contains("OnUnitInactiveSec=60"));
        assert_eq!(program_of(&service), Some(job.program));
    }
}
