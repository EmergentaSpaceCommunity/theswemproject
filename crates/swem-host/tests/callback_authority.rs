use agent_client_protocol::schema::v1::FileSystemCapabilities;
use serde_json::Value;
use std::collections::BTreeMap;
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use swem_host::workbench_shell::Terminals;
use swem_host::{
    AskBeforeRunning, FileCallbacks, INSIDE_ITS_WORKSPACE, IntegrationKind, LaunchCommand,
    NativeFileAnswer, NativeSessionControl, NativeSessionOptions, PersonalAgentProfile,
    TerminalCallbacks, run_native_session,
};

/// Every ACP client callback this host does not advertise.
const REFUSED: [&str; 7] = [
    "fs/read_text_file",
    "fs/write_text_file",
    "terminal/create",
    "terminal/output",
    "terminal/wait_for_exit",
    "terminal/kill",
    "terminal/release",
];
/// Bytes a leaked filesystem callback would have returned.
const SENTINEL: &[u8] = b"synthetic private editor contents";

#[tokio::test]
async fn unadvertised_client_callbacks_fail_without_host_effects_or_disabling_native_tools() {
    let root = std::env::temp_dir().join(format!(
        "swem-callback-audit-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&root).unwrap();
    fs::write(root.join("sentinel.txt"), SENTINEL).unwrap();
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-callback-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: vec![root.join("agent-wire.jsonl").display().to_string()],
        integration: IntegrationKind::DirectAcp,
    };
    // Per-operation deadlines only: a stalled callback must be reported as the
    // exact operation that stalled, not as an anonymous connection timeout.
    // The outer bound belongs to the test, not to a new product option.
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(15));
    options.transcript_path = Some(root.join("host-trace.jsonl"));
    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        run_native_session(
            &launch,
            &executable,
            &root,
            &["probe callback authority".into(), "run native tool".into()],
            &options,
        ),
    )
    .await
    .expect("the native session never terminated")
    .unwrap();

    let report: Value = serde_json::from_str(&outcome.turns[0].reply_text)
        .expect("the probe turn reported no callback outcomes");
    assert_eq!(outcome.turns[0].stop_reason, "end_turn");
    assert_ne!(report["capabilities"]["terminal"], true);
    for capability in ["readTextFile", "writeTextFile"] {
        assert_ne!(report["capabilities"]["fs"][capability], true);
    }
    for method in REFUSED {
        assert_eq!(
            report["callbacks"][method]["err"]["code"], -32601,
            "{method}: explicit method-not-found, not silence and not false success"
        );
    }

    // No host effect: the refused write left the bytes alone, and the refused
    // read never handed them to the agent.
    assert_eq!(fs::read(root.join("sentinel.txt")).unwrap(), SENTINEL);
    let sentinel = std::str::from_utf8(SENTINEL).unwrap();
    assert!(!outcome.turns[0].reply_text.contains(sentinel));
    let transcript = fs::read_to_string(root.join("host-trace.jsonl")).unwrap();
    assert!(!transcript.contains(sentinel));

    // Each refusal is a recorded host decision, not an absence.
    for method in REFUSED {
        assert!(
            transcript.contains(&format!(r#""method":"{method}""#)),
            "{method}: no host/callback_refused record"
        );
    }

    // The agent's own tool still works, in the same session, after the refusals.
    assert_eq!(outcome.turns[1].stop_reason, "end_turn");
    let native: Value = serde_json::from_str(&outcome.turns[1].reply_text).unwrap();
    assert_eq!(native["native_tool_exit_success"], true);
    assert_eq!(fs::read(root.join("native-marker")).unwrap(), b"executed");
}

/// What the surface hands back when it is asked to read. It is deliberately
/// not what is on disk: a read that came from the file would prove nothing
/// about where the answer came from.
const IN_THE_EDITOR: &str = "what the person has on their screen, not yet saved";
/// A file outside the workspace this connection is bounded to.
const ELSEWHERE: &[u8] = b"a file this connection has no business with";

/// A connection whose surface does carry `fs/*` still has a boundary, and it
/// is the host's.
///
/// The test above is the default: a client with no filesystem of its own -
/// the Workbench's browser tab - and every callback refused. This is the other
/// case, the one the editor door opens: the surface says it can read and write
/// text files, so the agent underneath is told so and its callbacks are
/// carried out to it. Two things have to hold at once for that to be safe, and
/// they are what this asserts:
///
/// - the answer comes from the surface and not from the disk, which is the
///   whole reason ACP puts these methods on the client: the file a person has
///   open is in their editor's buffer, and a host that read the path itself
///   would hand the agent a different file from the one on the screen;
/// - a path outside the connection's workspace is refused by the host, before
///   the surface is asked at all. The surface never names that boundary, so it
///   cannot widen it by asking.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one session, then everything it is evidence for: what the agent was told, what it got back, what the disk still says, what the surface was asked, and what the record kept"
)]
async fn advertised_file_callbacks_reach_the_surface_and_stop_at_the_boundary() {
    let root = std::env::temp_dir().join(format!(
        "swem-callback-files-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let outside = root.with_extension("outside");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&outside).unwrap();
    fs::write(root.join("sentinel.txt"), SENTINEL).unwrap();
    let beyond = outside.join("beyond.txt");
    fs::write(&beyond, ELSEWHERE).unwrap();

    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-callback-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: vec![root.join("agent-wire.jsonl").display().to_string()],
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::new(Duration::from_secs(15));
    options.transcript_path = Some(root.join("host-trace.jsonl"));
    options.file_callbacks = Some(FileCallbacks {
        capabilities: FileSystemCapabilities::new()
            .read_text_file(true)
            .write_text_file(true),
        boundary: root.clone(),
    });
    let control = NativeSessionControl::new();
    options.control = Some(control.clone());

    // The surface, played by this test: it answers out of its own head, the
    // way an editor answers out of its buffer, and keeps a note of every path
    // it was actually asked about.
    let asked: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let noted = Arc::clone(&asked);
    let surface = control.clone();
    let answering = tokio::spawn(async move {
        let mut after = 0;
        while let Some(request) = surface.file_request_after(after).await {
            let answer = if request.call.method == "fs/read_text_file" {
                NativeFileAnswer::Text(IN_THE_EDITOR.to_owned())
            } else {
                NativeFileAnswer::Written
            };
            noted.lock().unwrap().push((
                request.call.method.clone(),
                request.call.path.display().to_string(),
            ));
            if surface
                .answer_file_request(request.sequence, answer)
                .is_err()
            {
                return;
            }
            after = request.sequence;
        }
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(60),
        run_native_session(
            &launch,
            &executable,
            &root,
            &[
                "probe callback authority".into(),
                format!("probe the path {}", beyond.display()),
            ],
            &options,
        ),
    )
    .await
    .expect("the native session never terminated")
    .unwrap();
    answering.abort();

    // The agent was told it may ask, because the surface said it can answer.
    let inside: Value = serde_json::from_str(&outcome.turns[0].reply_text).unwrap();
    for capability in ["readTextFile", "writeTextFile"] {
        assert_eq!(
            inside["capabilities"]["fs"][capability], true,
            "the agent was not told the surface carries {capability}"
        );
    }

    // And what it got back is the surface's answer, not the file's bytes.
    assert_eq!(
        inside["callbacks"]["fs/read_text_file"]["ok"]["content"], IN_THE_EDITOR,
        "the read did not come from the surface: {inside}"
    );
    assert!(
        inside["callbacks"]["fs/write_text_file"]["ok"].is_object(),
        "the write was not carried out: {inside}"
    );
    assert_eq!(
        fs::read(root.join("sentinel.txt")).unwrap(),
        SENTINEL,
        "the host wrote the file itself instead of asking the surface to"
    );

    // The path outside the workspace is refused as a bad argument - which is
    // what it is, on a connection that does carry the method - and the reason
    // says which boundary it left.
    let out: Value = serde_json::from_str(&outcome.turns[1].reply_text).unwrap();
    for method in ["fs/read_text_file", "fs/write_text_file"] {
        assert_eq!(
            out["callbacks"][method]["err"]["code"], -32602,
            "{method}: a path outside the workspace was not refused: {out}"
        );
        assert_eq!(
            out["callbacks"][method]["err"]["data"]["reason"],
            "the path is outside the workspace this session works in",
            "{method}: refused without saying why: {out}"
        );
    }
    assert_eq!(fs::read(&beyond).unwrap(), ELSEWHERE);

    // The surface was never asked about it. This is the part that matters: a
    // boundary enforced by asking politely is not a boundary.
    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![
            (
                "fs/read_text_file".to_owned(),
                root.join("sentinel.txt").display().to_string()
            ),
            (
                "fs/write_text_file".to_owned(),
                root.join("sentinel.txt").display().to_string()
            ),
        ],
        "the surface saw a path the host should have stopped"
    );

    // The record says which file was touched and how it went, and never what
    // was in it - on the way out or on the way back. The claim is about the
    // host's own records and no wider: an agent that reads a file and then
    // says what it read out loud has put those words in the conversation, and
    // the conversation is recorded. What the host must not do is copy the
    // text there a second time, where nobody chose to put it.
    let transcript = fs::read_to_string(root.join("host-trace.jsonl")).unwrap();
    let files: Vec<Value> = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["kind"] == "host/file_callback")
        .collect();
    let touched: Vec<(&str, &str)> = files
        .iter()
        .filter_map(|record| {
            Some((
                record["payload"]["method"].as_str()?,
                record["payload"]["outcome"].as_str()?,
            ))
        })
        .collect();
    assert_eq!(
        touched,
        vec![
            ("fs/read_text_file", "read"),
            ("fs/write_text_file", "written")
        ],
        "the record does not say which callbacks were carried out and how they went: {files:?}"
    );
    for record in &files {
        let written = record.to_string();
        assert!(
            written.contains(&root.join("sentinel.txt").display().to_string()),
            "a file callback record does not say which file: {written}"
        );
        assert!(
            !written.contains(IN_THE_EDITOR) && !written.contains("overwritten"),
            "the record kept the text of a person's file: {written}"
        );
    }

    fs::remove_dir_all(&root).ok();
    fs::remove_dir_all(&outside).ok();
}

/// A connection whose agent may run commands runs them in the profile's own
/// environment, and no further.
///
/// This is the third case, and it is not the shape the two above have. `fs/*`
/// goes to the surface because an editor's buffer holds a file the disk does
/// not. Nothing equivalent holds for a command: what a command needs is the
/// place the agent's work lives, and the harness owns it - the same pty, in
/// the same profile's environment, that a person opens from the terminal
/// panel. So the host carries these out itself, and what this asserts is that
/// it ran them in that environment and refused a working directory outside it
/// before starting anything at all.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one session, then everything it is evidence for: what the agent was told, what came back, where the command actually ran, what the refused one left behind, and what the record kept"
)]
async fn advertised_terminal_callbacks_run_in_the_profiles_environment_and_stop_at_the_boundary() {
    let root = std::env::temp_dir().join(format!(
        "swem-callback-terminal-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    let home = root.join("home");
    let outside = root.join("outside");
    for directory in [&root, &workspace, &home, &outside] {
        fs::create_dir_all(directory).unwrap();
    }
    let profile = PersonalAgentProfile::new(
        "terminal-profile",
        "fixture-agent",
        "fixture",
        "direct",
        INSIDE_ITS_WORKSPACE,
        &workspace,
        &home,
        Vec::new(),
        Vec::new(),
    )
    .expect("the profile is well-formed");
    let terminals = Arc::new(Terminals::default());

    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-callback-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: vec![root.join("agent-wire.jsonl").display().to_string()],
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(30));
    options.transcript_path = Some(root.join("host-trace.jsonl"));
    options.terminal_callbacks = Some(TerminalCallbacks {
        terminals: Arc::clone(&terminals),
        profile: profile.clone(),
        secrets: BTreeMap::new(),
        ask: AskBeforeRunning::No,
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        run_native_session(
            &launch,
            &executable,
            &workspace,
            &[
                // `pwd` says where it ran; `$HOME` says whose environment it
                // ran in. Both have to be the profile's, or the command ran
                // somewhere that is not the agent's work.
                "run in a terminal printf %s \"$HOME\" > home.txt; pwd".into(),
                // Says something, then waits to be stopped. What it said has
                // to survive the stopping.
                "start and stop echo before the stop; sleep 30".into(),
                format!("run a terminal in {}", outside.display()),
            ],
            &options,
        ),
    )
    .await
    .expect("the native session never terminated")
    .unwrap();

    // The agent was told it may ask.
    let ran: Value = serde_json::from_str(&outcome.turns[0].reply_text).unwrap();
    assert_eq!(
        ran["capabilities"]["terminal"], true,
        "the agent was not told this client carries terminals: {ran}"
    );

    // It ran, it ended the way a command that worked ends, and what it said
    // came back.
    let terminal = &ran["terminal"];
    assert!(
        terminal["terminal/create"]["ok"].is_object(),
        "the terminal was not created: {terminal}"
    );
    assert_eq!(
        terminal["terminal/wait_for_exit"]["ok"]["exitCode"], 0,
        "the command did not end successfully: {terminal}"
    );
    let said = terminal["terminal/output"]["ok"]["output"]
        .as_str()
        .unwrap_or_default();
    let where_it_ran = fs::canonicalize(&workspace).unwrap();
    assert!(
        said.contains(&where_it_ran.display().to_string()),
        "the command did not run in the profile's workspace: {said:?}"
    );
    assert!(
        terminal["terminal/release"]["ok"].is_object(),
        "the terminal was not released: {terminal}"
    );

    // And it ran in the profile's environment, not this process's.
    assert_eq!(
        fs::read_to_string(workspace.join("home.txt")).unwrap(),
        fs::canonicalize(&home).unwrap().display().to_string(),
        "the command did not get the profile's agent home"
    );

    // A command the agent stopped is still a command the agent can read: ACP
    // keeps killing and releasing apart, and so does the host.
    let stopped: Value = serde_json::from_str(&outcome.turns[1].reply_text).unwrap();
    let stopped = &stopped["terminal"];
    assert_eq!(
        stopped["spoke_before_the_stop"], true,
        "the command never said anything, so the stop proves nothing: {stopped}"
    );
    assert!(
        stopped["terminal/kill"]["ok"].is_object(),
        "the command was not stopped: {stopped}"
    );
    assert_eq!(
        // portable-pty hangs a terminal up rather than killing its process,
        // which is what closing a terminal window does; the name comes back
        // as the platform's own.
        stopped["terminal/wait_for_exit"]["ok"]["signal"],
        "Hangup",
        "a stopped command did not say it was stopped: {stopped}"
    );
    assert!(
        stopped["terminal/output"]["ok"]["output"]
            .as_str()
            .unwrap_or_default()
            .contains("before the stop"),
        "what the command said did not survive the stopping: {stopped}"
    );

    // A working directory outside the workspace is refused as a bad argument,
    // the reason says which boundary it left, and nothing ran: the refusal is
    // the host's, before the process.
    let beyond: Value = serde_json::from_str(&outcome.turns[2].reply_text).unwrap();
    assert_eq!(
        beyond["terminal"]["terminal/create"]["err"]["code"], -32602,
        "a terminal outside the workspace was not refused: {beyond}"
    );
    assert_eq!(
        beyond["terminal"]["terminal/create"]["err"]["data"]["reason"],
        "the path is outside the workspace this session works in",
        "refused without saying why: {beyond}"
    );
    assert!(
        !outside.join("ran-here").exists(),
        "the refused command ran anyway"
    );

    // Nothing is left holding a pty afterwards, and the record says what was
    // run and how it went - the command line, because a person reading it
    // later needs to know what their agent did and `/bin/sh` answers nothing
    // - without keeping a byte of what the command said back.
    assert!(terminals.list().await.is_empty());
    let transcript = fs::read_to_string(root.join("host-trace.jsonl")).unwrap();
    let records: Vec<Value> = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["kind"] == "host/terminal_callback")
        .collect();
    let all: Vec<(&str, &str)> = records
        .iter()
        .filter_map(|record| {
            Some((
                record["payload"]["method"].as_str()?,
                record["payload"]["outcome"].as_str()?,
            ))
        })
        .collect();
    // Reads are left out of the sequence because the agent chooses how often
    // to look; that it looked at all is asserted on its own below.
    let went: Vec<(&str, &str)> = all
        .iter()
        .copied()
        .filter(|(method, _)| *method != "terminal/output")
        .collect();
    assert!(
        all.iter()
            .filter(|entry| *entry == &("terminal/output", "read"))
            .count()
            >= 2,
        "the record does not say the agent read what its terminals said: {all:?}"
    );
    assert_eq!(
        went,
        vec![
            ("terminal/create", "created"),
            ("terminal/wait_for_exit", "finished"),
            ("terminal/release", "released"),
            ("terminal/create", "created"),
            ("terminal/kill", "killed"),
            ("terminal/wait_for_exit", "stopped by Hangup"),
            ("terminal/release", "released"),
            ("terminal/create", "refused"),
        ],
        "the record does not say which terminal callbacks were carried out and how they went: {records:?}"
    );
    let created = records
        .iter()
        .find(|record| record["payload"]["outcome"] == "created")
        .expect("a terminal was created");
    assert!(
        created["payload"]["command"]
            .as_str()
            .unwrap_or_default()
            .contains("pwd"),
        "the record does not say what was run: {created}"
    );
    for record in &records {
        let written = record.to_string();
        assert!(
            !written.contains(&where_it_ran.display().to_string()),
            "the record kept what the command said: {written}"
        );
    }

    fs::remove_dir_all(&root).ok();
}

/// The profile that asks every time is asked about a command too, and the
/// command runs only on a yes.
///
/// This is the other half of the terminal, and the reason it is not simply a
/// flag: a client callback never reaches the person the way a tool call does,
/// so a host that carried one out would be answering for them. The question
/// goes out on the same lane an agent's own permission request goes out on -
/// the one the page draws and the editor door forwards - carrying the command
/// line, because nothing less would let anyone answer it. What this asserts is
/// the pair: a no leaves nothing behind, and a yes runs the same command.
#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one session with both answers in it, then everything each answer is evidence for: what came back to the agent, what the workspace holds, and what the person was actually asked"
)]
async fn a_command_is_put_to_the_person_first_and_runs_only_on_a_yes() {
    let root = std::env::temp_dir().join(format!(
        "swem-callback-asked-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let workspace = root.join("workspace");
    let home = root.join("home");
    for directory in [&root, &workspace, &home] {
        fs::create_dir_all(directory).unwrap();
    }
    let profile = PersonalAgentProfile::new(
        "asking-profile",
        "fixture-agent",
        "fixture",
        "direct",
        "surface-permissions",
        &workspace,
        &home,
        Vec::new(),
        Vec::new(),
    )
    .expect("the profile is well-formed");

    let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-callback-agent"));
    let launch = LaunchCommand {
        executable: executable.display().to_string(),
        args: vec![root.join("agent-wire.jsonl").display().to_string()],
        integration: IntegrationKind::DirectAcp,
    };
    let mut options = NativeSessionOptions::interactive(Duration::from_secs(30));
    options.transcript_path = Some(root.join("host-trace.jsonl"));
    options.terminal_callbacks = Some(TerminalCallbacks {
        terminals: Arc::new(Terminals::default()),
        profile,
        secrets: BTreeMap::new(),
        ask: AskBeforeRunning::EveryTime,
    });
    let control = NativeSessionControl::new();
    options.control = Some(control.clone());

    // The person, played by this test: no to the first command, yes to the
    // second, and a note of every question they were actually asked.
    let asked: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let noted = Arc::clone(&asked);
    let answering = control.clone();
    let person = tokio::spawn(async move {
        let mut after = 0;
        while let Some(request) = answering.permission_request_after(after).await {
            let title = request.tool_call.fields.title.clone().unwrap_or_default();
            let yes = !noted.lock().unwrap().is_empty();
            noted.lock().unwrap().push((
                title,
                serde_json::to_value(request.provenance)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .unwrap_or_default(),
            ));
            let option = if yes { "run-once" } else { "do-not-run" };
            if answering
                .select_permission(request.sequence, option)
                .is_err()
            {
                return;
            }
            after = request.sequence;
        }
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(120),
        run_native_session(
            &launch,
            &executable,
            &workspace,
            &[
                "run in a terminal echo refused > refused.txt".into(),
                "run in a terminal echo allowed > allowed.txt".into(),
            ],
            &options,
        ),
    )
    .await
    .expect("the native session never terminated")
    .unwrap();
    person.abort();

    // A no is a refusal the agent can act on, and nothing ran.
    let refused: Value = serde_json::from_str(&outcome.turns[0].reply_text).unwrap();
    assert_eq!(
        refused["terminal"]["terminal/create"]["err"]["data"]["reason"],
        "this command was not allowed to run",
        "a command nobody allowed was not refused, or refused without saying why: {refused}"
    );
    assert!(
        !workspace.join("refused.txt").exists(),
        "the command ran although the answer was no"
    );

    // A yes runs the same command.
    let allowed: Value = serde_json::from_str(&outcome.turns[1].reply_text).unwrap();
    assert!(
        allowed["terminal"]["terminal/create"]["ok"].is_object(),
        "the allowed command did not run: {allowed}"
    );
    assert!(
        workspace.join("allowed.txt").is_file(),
        "the allowed command left nothing behind"
    );

    // Both questions carried the command line, because that is the only thing
    // that makes them answerable, and both said the words were the host's.
    let asked = asked.lock().unwrap().clone();
    assert_eq!(
        asked,
        vec![
            (
                "Run /bin/sh -c echo refused > refused.txt".to_owned(),
                "host_callback".to_owned()
            ),
            (
                "Run /bin/sh -c echo allowed > allowed.txt".to_owned(),
                "host_callback".to_owned()
            ),
        ],
        "the person was not asked what would run, or was told an agent asked"
    );

    fs::remove_dir_all(&root).ok();
}
