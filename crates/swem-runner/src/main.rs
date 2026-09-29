//! The runner as a program. What is asked is read from standard input or
//! from a file named on the command line, never from the command line
//! itself, so nothing of an agent's appears in a list of processes.

use std::io::{Read, Write};
use std::path::Path;
use std::process::ExitCode;

use swem_runner::{FsAnswer, FsRequest, ProbeRequest, Why};

const USAGE: &str = "usage: swem-runner fs < request.json
       swem-runner probe < request.json
       swem-runner exec [--header <file>]
       swem-runner idle
       swem-runner --version";

fn say(answer: &impl serde::Serialize) -> ExitCode {
    let said = serde_json::to_vec(answer).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    if out.write_all(&said).and_then(|()| out.flush()).is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn asked() -> Result<String, String> {
    let mut asked = String::new();
    std::io::stdin()
        .read_to_string(&mut asked)
        .map(|_| asked)
        .map_err(|error| error.to_string())
}

fn fs() -> ExitCode {
    let answer = match asked().and_then(|asked| {
        serde_json::from_str::<FsRequest>(&asked).map_err(|error| error.to_string())
    }) {
        Ok(request) => swem_runner::answer(&request),
        Err(error) => FsAnswer::Refused {
            why: Why::Failed,
            said: format!("what was asked cannot be read: {error}"),
            now: None,
        },
    };
    say(&answer)
}

fn probe() -> ExitCode {
    // Nothing asked is a look at the machine alone.
    let request = asked()
        .ok()
        .filter(|asked| !asked.trim().is_empty())
        .map_or_else(
            || Ok(ProbeRequest::default()),
            |asked| serde_json::from_str::<ProbeRequest>(&asked),
        );
    match request {
        Ok(request) => say(&swem_runner::look(&request)),
        Err(error) => {
            eprintln!("swem-runner: what was asked cannot be read: {error}");
            ExitCode::from(2)
        }
    }
}

/// The standard input as it is, with nothing read ahead: what follows the
/// header is the program's.
#[cfg(unix)]
fn input() -> Result<std::fs::File, String> {
    use std::os::fd::AsFd;
    std::io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map(std::fs::File::from)
        .map_err(|error| error.to_string())
}

#[cfg(windows)]
fn input() -> Result<std::fs::File, String> {
    use std::os::windows::io::AsHandle;
    std::io::stdin()
        .as_handle()
        .try_clone_to_owned()
        .map(std::fs::File::from)
        .map_err(|error| error.to_string())
}

fn exec(header_file: Option<&str>) -> ExitCode {
    let header = match header_file {
        Some(path) => swem_runner::header_at(Path::new(path)),
        None => input().and_then(|mut input| swem_runner::header_from(&mut input)),
    };
    match header.and_then(|header| swem_runner::become_it(&header)) {
        Ok(ended) => ExitCode::from(u8::try_from(ended).unwrap_or(1)),
        Err(error) => {
            eprintln!("swem-runner: {error}");
            ExitCode::from(127)
        }
    }
}

/// Stay, doing nothing: what a machine of an agent's own runs while no
/// engine does. It ends when it is told to.
fn idle() -> ExitCode {
    loop {
        std::thread::park();
    }
}

fn main() -> ExitCode {
    let given: Vec<String> = std::env::args().skip(1).collect();
    match given.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        ["fs"] => fs(),
        ["probe"] => probe(),
        ["exec"] => exec(None),
        ["exec", "--header", path] => exec(Some(path)),
        ["idle"] => idle(),
        ["--version"] => {
            println!("swem-runner {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
