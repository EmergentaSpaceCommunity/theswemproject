//! A program is started as a header says, and what the header holds is in
//! nobody's list of processes.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use swem_runner::{ExecHeader, ProbeAnswer, ProbeRequest};

const RUNNER: &str = env!("CARGO_BIN_EXE_swem-runner");

fn header(argv: &[&str]) -> ExecHeader {
    ExecHeader {
        argv: argv.iter().map(|part| (*part).to_owned()).collect(),
        cwd: None,
        env: BTreeMap::new(),
        keep: None,
    }
}

#[test]
fn what_follows_the_header_is_the_programs_own() {
    let mut runner = Command::new(RUNNER)
        .arg("exec")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the runner");
    // The header and what follows it arrive at once: nothing of what
    // follows may be read with the header.
    let mut sent = header(&["/bin/cat"]).as_a_line();
    let follows = b"{\"jsonrpc\":\"2.0\",\"id\":1}\nsecond line\n\x00\xffbytes";
    sent.extend_from_slice(follows);
    runner
        .stdin
        .take()
        .expect("its input")
        .write_all(&sent)
        .expect("send");
    let output = runner.wait_with_output().expect("its end");
    assert!(output.status.success());
    assert_eq!(output.stdout, follows);
}

#[test]
fn a_key_is_given_and_is_in_no_list_of_processes() {
    let folder = std::env::temp_dir().join(format!(
        "swem-runner-exec-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&folder).expect("a folder");
    // Made here, so that nothing else on this machine holds it by chance.
    let key = format!(
        "not-a-key-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    );
    let key = key.as_str();
    let mut started = header(&[
        "/bin/sh",
        "-c",
        "echo \"$THE_KEY in $(pwd) kept:${KEPT:-no} dropped:${DROPPED:-no}\"; sleep 3",
    ]);
    started.cwd = Some(folder.clone());
    started.env.insert("THE_KEY".into(), key.into());
    started.keep = Some(vec!["PATH".into(), "KEPT".into()]);
    assert!(
        !format!("{started:?}").contains(key),
        "a header is not printed with what it holds"
    );

    let mut runner = Command::new(RUNNER)
        .arg("exec")
        .env("KEPT", "yes")
        .env("DROPPED", "yes")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the runner");
    let pid = runner.id();
    runner
        .stdin
        .take()
        .expect("its input")
        .write_all(&started.as_a_line())
        .expect("send the header");
    let mut out = runner.stdout.take().expect("its output");
    let mut line = Vec::new();
    let mut byte = [0_u8; 1];
    while out.read(&mut byte).expect("read") == 1 && byte[0] != b'\n' {
        line.push(byte[0]);
    }
    let line = String::from_utf8(line).expect("text");
    let folder_as_it_is = std::fs::canonicalize(&folder).expect("the folder");
    assert_eq!(
        line,
        format!("{key} in {} kept:yes dropped:no", folder_as_it_is.display())
    );
    // The program is the process the runner was: one process, and the key
    // is in neither's command line.
    let listed = Command::new("ps")
        .args(["-o", "args=", "-p", &pid.to_string()])
        .output()
        .expect("list the process");
    let listed = String::from_utf8_lossy(&listed.stdout);
    assert!(listed.contains("/bin/sh"), "{listed}");
    assert!(!listed.contains(key), "{listed}");
    let everything = Command::new("ps")
        .args(["-A", "-o", "args="])
        .output()
        .expect("list every process");
    assert!(!String::from_utf8_lossy(&everything.stdout).contains(key));
    runner.kill().expect("stop it");
    let _ = runner.wait();
    std::fs::remove_dir_all(folder).expect("remove the folder");
}

#[test]
fn a_header_in_a_file_is_read_once_and_removed() {
    let file = std::env::temp_dir().join(format!(
        "swem-runner-header-{}-{}.json",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    std::fs::write(&file, header(&["/bin/echo", "started"]).as_a_line()).expect("the header");
    let output = Command::new(RUNNER)
        .args(["exec", "--header"])
        .arg(&file)
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "started\n");
    assert!(!file.exists(), "the header was left where it was");

    // What cannot be started is said, and nothing is started.
    let nothing = Command::new(RUNNER)
        .arg("exec")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut runner| {
            runner
                .stdin
                .take()
                .expect("its input")
                .write_all(&header(&["/no/such/program"]).as_a_line())?;
            runner.wait_with_output()
        })
        .expect("run");
    assert_eq!(nothing.status.code(), Some(127));
    assert!(String::from_utf8_lossy(&nothing.stderr).contains("/no/such/program"));
}

#[test]
fn the_machine_is_looked_at_from_inside() {
    let workspace = std::env::temp_dir();
    let mut runner = Command::new(RUNNER)
        .arg("probe")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start the runner");
    runner
        .stdin
        .take()
        .expect("its input")
        .write_all(
            &serde_json::to_vec(&ProbeRequest {
                workspace: Some(workspace),
                programs: vec!["sh".into(), "no-such-program-here".into(), "../sh".into()],
            })
            .expect("a request"),
        )
        .expect("ask");
    let output = runner.wait_with_output().expect("its answer");
    let found: ProbeAnswer = serde_json::from_slice(&output.stdout).expect("an answer");
    assert_eq!(found.system, std::env::consts::OS);
    assert_eq!(found.workspace_writable, Some(true));
    assert_eq!(found.workspace_owned, Some(true));
    assert!(found.workspace_free_bytes.is_some_and(|free| free > 0));
    assert!(found.user.is_some());
    assert!(found.processors >= 1);
    assert!(found.programs[0].path.is_some());
    assert_eq!(found.programs[1].path, None);
    assert_eq!(found.programs[2].path, None, "a path is not a name");
}

#[test]
fn idle_stays_until_it_is_told_to_end() {
    let mut runner = Command::new(RUNNER)
        .arg("idle")
        .stdin(Stdio::null())
        .spawn()
        .expect("start the runner");
    let began = Instant::now();
    while began.elapsed() < Duration::from_millis(600) {
        assert!(
            runner.try_wait().expect("ask").is_none(),
            "it left by itself"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    runner.kill().expect("tell it to end");
    assert!(!runner.wait().expect("its end").success());
}
