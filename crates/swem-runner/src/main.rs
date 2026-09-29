//! The runner as a program: one request on standard input, its answer on
//! standard output. Nothing about the request is read from the command
//! line, so nothing of an agent's appears in a list of processes.

use std::io::{Read, Write};
use std::process::ExitCode;

use swem_runner::{FsAnswer, FsRequest, Why};

const USAGE: &str = "usage: swem-runner fs < request.json\n       swem-runner --version";

fn fs() -> ExitCode {
    let mut asked = String::new();
    let answer = match std::io::stdin().read_to_string(&mut asked) {
        Ok(_) => match serde_json::from_str::<FsRequest>(&asked) {
            Ok(request) => swem_runner::answer(&request),
            Err(error) => FsAnswer::Refused {
                why: Why::Failed,
                said: format!("what was asked cannot be read: {error}"),
                now: None,
            },
        },
        Err(error) => FsAnswer::Refused {
            why: Why::Failed,
            said: format!("what was asked cannot be read: {error}"),
            now: None,
        },
    };
    let said = serde_json::to_vec(&answer).unwrap_or_default();
    let mut out = std::io::stdout().lock();
    if out.write_all(&said).and_then(|()| out.flush()).is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("fs") => fs(),
        Some("--version") => {
            println!("swem-runner {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
