//! A channel carries a chat both ways: what arrives on a messenger's side is
//! said here by the one it was bound to, and what the agent answers is
//! carried back as it is written. The messenger is the channel fixture - two
//! directories - and the agent is the echo fixture. No bot, no network.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use swem_host::{
    AddChannelBody, AttachmentBinding, AttachmentTransport, GuestPolicy, IntegrationKind, Kind,
    LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore, ResolvedDirectAgentConnection,
    StoreInstallBody, StorePlanBody, WorkbenchShellState, registry_platform,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-channels-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

/// One agent of the echo engine, with keys and channels enabled, as the
/// product enables them.
fn shell_with_one_agent(root: &Path) -> Arc<WorkbenchShellState> {
    let inventory = root.join("inventory");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    let workspace = root.join("coder");
    let agent_home = root.join("coder-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    store
        .create(
            &PersonalAgentProfile::new(
                "coder",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                swem_host::ASK_EVERY_TIME,
                &workspace,
                &agent_home,
                vec![AttachmentBinding::new(
                    "echo-attachment",
                    "echo",
                    AttachmentTransport::Stdio,
                )],
                Vec::new(),
            )
            .expect("build fixture profile"),
        )
        .expect("persist fixture profile");
    let receipt = root.join("mcp-receipt.json");
    let state = WorkbenchShellState::open(
        &inventory,
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        move |_profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            let mcp = agent_client_protocol::schema::v1::McpServer::Stdio(
                agent_client_protocol::schema::v1::McpServerStdio::new(
                    "echo",
                    PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-echo")),
                )
                .args(vec!["--receipt".into(), receipt.display().to_string()]),
            );
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![mcp],
            })
        },
    )
    .expect("open shell state");
    let state = Arc::new(state);
    state
        .enable_provider_keys_kept_by(Arc::new(
            swem_host::InFiles::at(&root.join("secrets")).expect("a place for keys"),
        ))
        .expect("keys");
    state
        .enable_channels(&root.join("channels"))
        .expect("channels");
    state
}

/// Drop an event into the fixture's inbox, as the messenger would deliver it.
fn arrives(home: &Path, name: &str, event: &Value) {
    let inbox = home.join("inbox");
    fs::create_dir_all(&inbox).expect("inbox");
    fs::write(
        inbox.join(format!("{name}.json")),
        serde_json::to_vec(event).expect("json"),
    )
    .expect("drop the event");
}

/// Everything the harness asked the channel to do so far.
fn calls(home: &Path) -> Vec<Value> {
    fs::read_to_string(home.join("outbox").join("calls.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

async fn until<T>(what: &str, mut look: impl FnMut() -> Option<T>) -> T {
    let began = std::time::Instant::now();
    loop {
        if let Some(found) = look() {
            return found;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "waited thirty seconds for {what}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn person(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "username": name.to_lowercase() })
}

fn message(chat: &str, who: &Value, text: &str, reference: &str) -> Value {
    json!({
        "kind": "message",
        "chat": { "id": chat, "kind": "direct" },
        "person": who,
        "text": text,
        "reference": reference
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one channel: pairing, a turn, a duplicate, a stranger, removal"
)]
async fn the_owner_pairs_with_a_code_talks_to_their_agent_and_a_stranger_gets_one_line() {
    let root = fixture_root("owner");
    let state = shell_with_one_agent(&root);

    // A channel run by a program of one's own: the fixture. It answers as a
    // channel, the harness looks at who the bot is, and shows the code.
    let shown = state
        .add_channel(AddChannelBody {
            name: "The fixture bot".into(),
            package: None,
            program: Some(env!("CARGO_BIN_EXE_swem-channel-fixture").into()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings::default(),
            key: Some("not-a-real-token".into()),
        })
        .await
        .expect("the channel is added");
    assert!(shown.running, "{}", shown.said);
    assert!(shown.keyed);
    assert!(!shown.paired);
    let bot = shown.document.bot.clone().expect("the bot was looked at");
    assert_eq!(bot.username, "fixture_bot");
    let code = shown.document.pairing_code.clone().expect("a code to say");
    let channel_id = shown.document.id.clone();
    let home = root.join("channels").join(&channel_id);

    // The owner says the code to the bot: known from then on.
    arrives(
        &home,
        "001",
        &message("dm-ada", &person("u-1", "Ada"), &code, "upd-1"),
    );
    let began = std::time::Instant::now();
    loop {
        let shown = state.channels_shown().await.expect("channels");
        if shown
            .iter()
            .any(|one| one.document.id == channel_id && one.paired)
        {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "waited thirty seconds for the owner to be paired"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let said = until("the welcome", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "send" && call["arguments"]["chat"] == "dm-ada")
    })
    .await;
    assert!(
        said["arguments"]["markdown"]
            .as_str()
            .unwrap_or_default()
            .contains("known here"),
        "{said}"
    );

    // The owner asks for something: a chat here, their words in it from the
    // channel, the agent's answer streamed back and finished.
    arrives(
        &home,
        "002",
        &message(
            "dm-ada",
            &person("u-1", "Ada"),
            "/status and then look",
            "upd-2",
        ),
    );
    let finished = until("the agent's answer to reach the messenger", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "stream_end" && call["arguments"]["chat"] == "dm-ada")
    })
    .await;
    let answer = finished["arguments"]["markdown"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    assert!(!answer.trim().is_empty(), "{finished}");
    assert!(
        calls(&home)
            .into_iter()
            .any(|call| call["tool"] == "stream_begin"),
        "the turn was not streamed as it was written"
    );
    // The chat is a chat here, with the owner's words marked as from the
    // channel and the agent's answer after them.
    let chats = state.chats().await.expect("chats");
    assert_eq!(chats.len(), 1, "{chats:?}");
    let chat = &chats[0];
    assert_eq!(chat.title, "Fixture", "named after the bot");
    let page = state
        .chat_page(&chat.chat_id, None, 100)
        .await
        .expect("the chat's page");
    let first = &page.messages[0];
    assert_eq!(first.text, "/status and then look");
    assert_eq!(first.channel, format!("channel:{channel_id}"));
    assert_eq!(first.channel_ref.as_deref(), Some("upd-2"));
    assert!(page.messages.len() >= 2, "the agent answered in the chat");

    // The same thing delivered twice is said once.
    arrives(
        &home,
        "003",
        &message(
            "dm-ada",
            &person("u-1", "Ada"),
            "/status and then look",
            "upd-2",
        ),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    let page = state
        .chat_page(&chat.chat_id, None, 100)
        .await
        .expect("the chat's page");
    assert_eq!(
        page.messages
            .iter()
            .filter(|message| message.channel_ref.as_deref() == Some("upd-2"))
            .count(),
        1
    );

    // A stranger gets one polite line and nothing else: no chat, no agent.
    arrives(
        &home,
        "004",
        &message("dm-bo", &person("u-2", "Bo"), "hello?", "upd-3"),
    );
    let refused = until("the stranger's line", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "send" && call["arguments"]["chat"] == "dm-bo")
    })
    .await;
    assert!(
        refused["arguments"]["markdown"]
            .as_str()
            .unwrap_or_default()
            .contains("allow you to speak"),
        "{refused}"
    );
    let chats = state.chats().await.expect("chats");
    assert_eq!(chats.len(), 1, "a stranger opened no chat");
    // Writing again, the stranger is not told again: once is enough.
    arrives(
        &home,
        "005",
        &message("dm-bo", &person("u-2", "Bo"), "hello??", "upd-4"),
    );
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        calls(&home)
            .iter()
            .filter(|call| call["tool"] == "send" && call["arguments"]["chat"] == "dm-bo")
            .count(),
        1,
        "the stranger was told twice"
    );

    // Removed: the program stops, the key is forgotten, the chat stays.
    state
        .remove_channel(&channel_id)
        .await
        .expect("the channel is removed");
    assert!(state.channels_shown().await.expect("channels").is_empty());
    assert_eq!(state.chats().await.expect("chats").len(), 1);
}

/// A program that does not answer as a channel is refused in words, before
/// anything is bound to it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_program_that_is_not_a_channel_is_refused_in_words() {
    let root = fixture_root("not-a-channel");
    let state = shell_with_one_agent(&root);
    let shown = state
        .add_channel(AddChannelBody {
            name: "Not a channel".into(),
            package: None,
            // The taker fixture: an MCP server, with other tools.
            program: Some(env!("CARGO_BIN_EXE_swem-mcp-taker-fixture").into()),
            args: vec!["--home".into(), root.join("taker").display().to_string()],
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings::default(),
            key: None,
        })
        .await
        .expect("added, but not running");
    assert!(!shown.running);
    assert!(
        shown.said.contains("does not answer as a channel") && shown.said.contains("`pull`"),
        "{}",
        shown.said
    );
}

/// The Telegram channel beside this crate's binaries, when it was built
/// (`cargo build -p swem-channel-telegram`; the gate does).
fn telegram_channel() -> Option<PathBuf> {
    let beside = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let program = beside
        .parent()?
        .join(format!("swem-channel-telegram{suffix}"));
    program.is_file().then_some(program)
}

/// One HTTP/1.1 request to the Bot API fixture's side door, by hand: the
/// test needs nothing more than a line and a body.
fn fixture_call(address: &str, method: &str, path: &str, body: Option<&Value>) -> Value {
    use std::io::{Read as _, Write as _};
    let host = address.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(host).expect("the fixture listens");
    let body = body.map(Value::to_string).unwrap_or_default();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write the request");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read the answer");
    let (_, json) = answer.split_once("\r\n\r\n").expect("a body");
    serde_json::from_str::<Value>(json).expect("json")["result"].clone()
}

fn sent(address: &str) -> Vec<Value> {
    fixture_call(address, "GET", "/_fixture/sent", None)
        .as_array()
        .cloned()
        .unwrap_or_default()
}

fn telegram_update(from: (i64, &str), chat: i64, text: &str) -> Value {
    json!({
        "message": {
            "message_id": 1,
            "from": { "id": from.0, "is_bot": false, "first_name": from.1, "username": from.1.to_lowercase() },
            "chat": { "id": chat, "type": "private", "first_name": from.1 },
            "date": 0,
            "text": text
        }
    })
}

/// The Telegram channel, against a Bot API that is a fixture: the bot is
/// looked at, the owner pairs, a message becomes a chat, the agent's answer
/// is drafted while it is written and sent when done, and `/stop` stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_telegram_channel_carries_a_chat_through_the_bot_api() {
    let Some(program) = telegram_channel() else {
        eprintln!(
            "skipped: build the Telegram channel first: cargo build -p swem-channel-telegram"
        );
        return;
    };
    let root = fixture_root("telegram");
    // The Bot API fixture, on loopback, printing its address.
    let mut api = std::process::Command::new(env!("CARGO_BIN_EXE_swem-telegram-api-fixture"))
        .arg("--files")
        .arg(root.join("files"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let address = {
        use std::io::BufRead as _;
        let stdout = api.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        lines
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };
    let state = shell_with_one_agent(&root);
    let shown = state
        .add_channel(AddChannelBody {
            name: "Telegram".into(),
            package: None,
            program: Some(program.display().to_string()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings {
                api_root: Some(address.clone()),
                door: None,
            },
            key: Some("123456:fixture".into()),
        })
        .await
        .expect("the channel is added");
    assert!(shown.running, "{}", shown.said);
    let bot = shown.document.bot.clone().expect("the bot was looked at");
    assert_eq!(bot.username, "swem_fixture_bot");
    let code = shown.document.pairing_code.clone().expect("a code");
    let channel_id = shown.document.id.clone();

    // The owner says the code; then asks for something.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, &code)),
    );
    let welcomed = until("the welcome", || {
        sent(&address).into_iter().find(|call| {
            call["method"] == "sendMessage"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("known here"))
        })
    })
    .await;
    assert_eq!(welcomed["body"]["chat_id"], "7");
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, "look at this and say")),
    );
    let answered = until("the agent's answer on Telegram", || {
        let calls = sent(&address);
        let drafted = calls
            .iter()
            .any(|call| call["method"] == "sendMessageDraft");
        calls
            .into_iter()
            .filter(|call| call["method"] == "sendMessage")
            .find(|call| {
                call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("session_id"))
            })
            .filter(|_| drafted)
    })
    .await;
    assert_eq!(answered["body"]["parse_mode"], "HTML");
    let chats = state.chats().await.expect("chats");
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0].title, "Fixture bot");
    let page = state
        .chat_page(&chats[0].chat_id, None, 100)
        .await
        .expect("the page");
    assert_eq!(page.messages[0].channel, format!("channel:{channel_id}"));
    assert_eq!(page.messages[0].text, "look at this and say");

    state.remove_channel(&channel_id).await.expect("removed");
    let _ = api.kill();
    let _ = api.wait();
}

/// A guest by invitation: somebody who writes to the bot is told to wait;
/// once the owner lets them into a chat from the page, what they write to
/// the bot is said in that chat, the owner sees it on their own side with
/// the guest's name, and the agent's answer reaches both; taken out, the
/// guest is told to wait again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one guest: told to wait, let in, reached, taken out"
)]
async fn a_guest_is_let_into_a_chat_and_reached_there() {
    let Some(program) = telegram_channel() else {
        eprintln!(
            "skipped: build the Telegram channel first: cargo build -p swem-channel-telegram"
        );
        return;
    };
    let root = fixture_root("telegram-guest");
    let mut api = std::process::Command::new(env!("CARGO_BIN_EXE_swem-telegram-api-fixture"))
        .arg("--files")
        .arg(root.join("files"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let address = {
        use std::io::BufRead as _;
        let stdout = api.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        lines
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };
    let state = shell_with_one_agent(&root);
    let shown = state
        .add_channel(AddChannelBody {
            name: "Telegram".into(),
            package: None,
            program: Some(program.display().to_string()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings {
                api_root: Some(address.clone()),
                door: None,
            },
            key: Some("123456:fixture".into()),
        })
        .await
        .expect("the channel is added");
    assert!(shown.running, "{}", shown.said);
    let code = shown.document.pairing_code.clone().expect("a code");
    let channel_id = shown.document.id.clone();

    // The owner pairs and opens the bot's chat with a first message.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, &code)),
    );
    until("the welcome", || {
        sent(&address)
            .into_iter()
            .find(|call| call["method"] == "sendMessage" && call["body"]["chat_id"] == "7")
    })
    .await;
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, "say something please")),
    );
    until("the agent's answer to the owner", || {
        sent(&address).into_iter().find(|call| {
            call["method"] == "sendMessage"
                && call["body"]["chat_id"] == "7"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("session_id"))
        })
    })
    .await;

    // A stranger writes: told to wait, nothing opened.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((9, "Bob"), 9, "hello?")),
    );
    let told = until("the guest is told to wait", || {
        sent(&address)
            .into_iter()
            .find(|call| call["method"] == "sendMessage" && call["body"]["chat_id"] == "9")
    })
    .await;
    assert!(
        told["body"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("allow you to speak")),
        "{told}"
    );
    let chats = state.chats().await.expect("chats");
    assert_eq!(chats.len(), 1, "the guest opened a chat");
    let chat_id = chats[0].chat_id.clone();

    // The owner lets the guest in, from the page's "Add someone".
    let people = state.chat_people().await.expect("people");
    let guest = people["participants"]
        .as_array()
        .expect("participants")
        .iter()
        .find(|one| one["name"] == "Bob")
        .expect("the guest is known")["participant_id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let chat = state
        .let_guest_into_chat(&chat_id, &guest)
        .await
        .expect("the guest is let in");
    assert!(
        chat.members
            .iter()
            .any(|member| member.participant_id == guest)
    );

    // What the guest writes to the bot is said in the chat, the owner sees
    // it on their side with the guest's name, and the agent answers both.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((9, "Bob"), 9, "a word for the guest")),
    );
    let owner_saw = until("the owner sees the guest's words", || {
        sent(&address).into_iter().find(|call| {
            call["method"] == "sendMessage"
                && call["body"]["chat_id"] == "7"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("Bob") && text.contains("for the guest"))
        })
    })
    .await;
    assert_eq!(owner_saw["body"]["parse_mode"], "HTML");
    until("the agent's answer reaches the guest", || {
        sent(&address).into_iter().find(|call| {
            call["method"] == "sendMessage"
                && call["body"]["chat_id"] == "9"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("session_id"))
        })
    })
    .await;
    // And the guest's own words were not sent back to the guest with
    // their name on them, as they were to the owner.
    assert!(
        !sent(&address).into_iter().any(|call| {
            call["method"] == "sendMessage"
                && call["body"]["chat_id"] == "9"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("<b>Bob:</b>"))
        }),
        "the guest's words came back to them"
    );
    let page = state
        .chat_page(&chat_id, None, 100)
        .await
        .expect("the page");
    assert!(
        page.messages
            .iter()
            .any(|message| message.sender_id == guest && message.text == "a word for the guest"),
        "the guest's words are not in the chat as theirs"
    );

    // Taken out and forbidden, the guest is not answered - and not told
    // again either: once was enough.
    state
        .take_out_of_chat(&chat_id, &guest)
        .await
        .expect("the guest is taken out");
    state
        .allow_guest_at(&channel_id, &guest, false)
        .await
        .expect("forbidden");
    let before = sent(&address).len();
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((9, "Bob"), 9, "still there?")),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let since: Vec<Value> = sent(&address).into_iter().skip(before).collect();
    assert!(
        since.iter().all(|call| call["method"] == "getUpdates"),
        "a forbidden guest got an answer: {since:?}"
    );

    state.remove_channel(&channel_id).await.expect("removed");
    let _ = api.kill();
    let _ = api.wait();
}

/// A wake is a look at the clock: a Workbench with nobody keeping time
/// (a server woken by a message) says what was due as soon as something
/// arrives through a channel.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: a bot paired, something due, nobody keeping time, a message, the due said"
)]
async fn a_message_through_a_channel_has_what_was_due_said() {
    let Some(program) = telegram_channel() else {
        eprintln!(
            "skipped: build the Telegram channel first: cargo build -p swem-channel-telegram"
        );
        return;
    };
    let root = fixture_root("telegram-wake");
    let mut api = std::process::Command::new(env!("CARGO_BIN_EXE_swem-telegram-api-fixture"))
        .arg("--files")
        .arg(root.join("files"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let address = {
        use std::io::BufRead as _;
        let stdout = api.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        lines
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };
    let state = shell_with_one_agent(&root);
    state
        .enable_keepers(&root.join("time"))
        .expect("keep who keeps time");
    let shown = state
        .add_channel(AddChannelBody {
            name: "Telegram".into(),
            package: None,
            program: Some(program.display().to_string()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings {
                api_root: Some(address.clone()),
                door: None,
            },
            key: Some("123456:fixture".into()),
        })
        .await
        .expect("the channel is added");
    let code = shown.document.pairing_code.clone().expect("a code");
    let channel_id = shown.document.id.clone();
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, &code)),
    );
    until("the welcome", || {
        sent(&address)
            .into_iter()
            .find(|call| call["method"] == "sendMessage" && call["body"]["chat_id"] == "7")
    })
    .await;

    // Something due in a moment, with nobody running to say it.
    let people = state.chat_people().await.expect("people");
    let agent = people["participants"]
        .as_array()
        .expect("participants")
        .iter()
        .find(|one| one["profile_id"] == "coder")
        .expect("the agent")["participant_id"]
        .as_str()
        .expect("an id")
        .to_owned();
    let due_at = u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis(),
    )
    .unwrap_or_default()
        + 1_000;
    state
        .make_schedule(swem_host::workbench_shell::NewScheduleBody {
            agent_id: agent,
            chat_id: None,
            say: "the morning summary, please".into(),
            when: swem_host::When::Once { at_ms: due_at },
        })
        .await
        .expect("make a schedule");
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let said_so_far = |state: &Arc<WorkbenchShellState>| {
        let state = Arc::clone(state);
        async move {
            let mut found = false;
            for chat in state.chats().await.unwrap_or_default() {
                let page = state
                    .chat_page(&chat.chat_id, None, 100)
                    .await
                    .expect("page");
                found |= page
                    .messages
                    .iter()
                    .any(|message| message.text.contains("the morning summary"));
            }
            found
        }
    };
    assert!(
        !said_so_far(&state).await,
        "nobody keeps time here, yet what was due was said before anything arrived"
    );

    // A message arrives: the Workbench looks at the clock on the way.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, "good morning")),
    );
    let began = std::time::Instant::now();
    loop {
        if said_so_far(&state).await {
            break;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "what was due was not said when a message arrived"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    state.remove_channel(&channel_id).await.expect("removed");
    let _ = api.kill();
    let _ = api.wait();
}

/// In a group, the agent takes a turn only when the bot is spoken to - by
/// name or by a reply to it. What else is said there is in the chat for
/// the record, and nobody is greeted or refused over it. Another bot there
/// is met as a bot, speaks when allowed, and is answered on a budget of
/// turns that a person's word starts again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one group: named, not spoken to, a stranger, a reply, a bot"
)]
async fn in_a_group_the_agent_answers_only_when_spoken_to() {
    let Some(program) = telegram_channel() else {
        eprintln!(
            "skipped: build the Telegram channel first: cargo build -p swem-channel-telegram"
        );
        return;
    };
    let root = fixture_root("telegram-group");
    let mut api = std::process::Command::new(env!("CARGO_BIN_EXE_swem-telegram-api-fixture"))
        .arg("--files")
        .arg(root.join("files"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let address = {
        use std::io::BufRead as _;
        let stdout = api.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        lines
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };
    let state = shell_with_one_agent(&root);
    let shown = state
        .add_channel(AddChannelBody {
            name: "Telegram".into(),
            package: None,
            program: Some(program.display().to_string()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings {
                api_root: Some(address.clone()),
                door: None,
            },
            key: Some("123456:fixture".into()),
        })
        .await
        .expect("the channel is added");
    let code = shown.document.pairing_code.clone().expect("a code");
    let channel_id = shown.document.id.clone();
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, &code)),
    );
    until("the welcome", || {
        sent(&address)
            .into_iter()
            .find(|call| call["method"] == "sendMessage" && call["body"]["chat_id"] == "7")
    })
    .await;

    let in_group = |from: (i64, &str), text: &str, reply_to_bot: bool| {
        let mut update = json!({
            "message": {
                "message_id": 1,
                "from": { "id": from.0, "is_bot": false, "first_name": from.1, "username": from.1.to_lowercase() },
                "chat": { "id": -100, "type": "supergroup", "title": "Studio" },
                "date": 0,
                "text": text
            }
        });
        if reply_to_bot {
            update["message"]["reply_to_message"] = json!({
                "message_id": 0,
                "from": { "id": 1000, "is_bot": true, "username": "swem_fixture_bot" }
            });
        }
        update
    };
    let answers = |address: &str| {
        sent(address)
            .into_iter()
            .filter(|call| {
                call["method"] == "sendMessage"
                    && call["body"]["chat_id"] == "-100"
                    && call["body"]["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("session_id"))
            })
            .count()
    };

    // Spoken to by name: a turn.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group(
            (7, "Ada"),
            "@swem_fixture_bot /status for the group",
            false,
        )),
    );
    until("the agent answers in the group", || {
        (answers(&address) >= 1).then_some(())
    })
    .await;

    // Not spoken to: for the record, no turn, nobody told anything.
    let before = sent(&address).len();
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group((7, "Ada"), "just talking among ourselves", false)),
    );
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group((9, "Bob"), "me too, not to the bot", false)),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    let since: Vec<Value> = sent(&address).into_iter().skip(before).collect();
    assert!(
        since.iter().all(|call| call["method"] == "getUpdates"),
        "the bot acted on what was not said to it: {since:?}"
    );
    let chats = state.chats().await.expect("chats");
    let group = chats
        .iter()
        .find(|chat| chat.title == "Studio")
        .expect("the group's chat");
    let page = state
        .chat_page(&group.chat_id, None, 100)
        .await
        .expect("the page");
    assert!(
        page.messages
            .iter()
            .any(|message| message.text == "just talking among ourselves"),
        "what the owner said in the group is not in the chat"
    );
    // Somebody else in the group is met: a person in the chat, their words
    // there as theirs, and no right to make the agent speak.
    let bob = page
        .chat
        .members
        .iter()
        .find(|member| member.name == "Bob")
        .expect("Bob is in the group's chat");
    assert!(
        page.messages
            .iter()
            .any(|message| message.text == "me too, not to the bot"
                && message.sender_id == bob.participant_id),
        "Bob's words are not in the chat as his"
    );
    let shown = state.channels_shown().await.expect("channels");
    let met = &shown[0].people;
    assert!(
        met.iter().any(|one| one.name == "Bob" && !one.may_speak),
        "Bob is not listed as met and silent: {met:?}"
    );

    // A reply to the bot: a turn.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group((7, "Ada"), "and this one too, by reply", true)),
    );
    until("the agent answers the reply", || {
        (answers(&address) >= 2).then_some(())
    })
    .await;

    // Bob speaks to the bot: he may not, so nothing - not even a line in the
    // group. Allowed by the owner, he is answered.
    let bob_id = page
        .chat
        .members
        .iter()
        .find(|member| member.name == "Bob")
        .expect("Bob")
        .participant_id
        .clone();
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group(
            (9, "Bob"),
            "@swem_fixture_bot /status please",
            false,
        )),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        answers(&address),
        2,
        "the agent answered somebody who may not speak"
    );
    state
        .allow_guest_at(&channel_id, &bob_id, true)
        .await
        .expect("allowed");
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group(
            (9, "Bob"),
            "@swem_fixture_bot /status now",
            false,
        )),
    );
    until("the agent answers Bob once allowed", || {
        (answers(&address) >= 3).then_some(())
    })
    .await;

    // Another bot in the group is met as what it is, and may not speak
    // until allowed, like anybody else.
    let from_a_bot = |text: &str| {
        let mut update = in_group((2000, "Echo"), text, false);
        update["message"]["from"]["is_bot"] = json!(true);
        update
    };
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&from_a_bot("@swem_fixture_bot say something")),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        answers(&address),
        3,
        "the agent answered a bot nobody allowed"
    );
    let shown = state.channels_shown().await.expect("channels");
    let echo = shown[0]
        .people
        .iter()
        .find(|one| one.name == "Echo")
        .expect("the bot is listed among those met");
    assert!(echo.bot && !echo.may_speak, "{echo:?}");
    state
        .allow_guest_at(&channel_id, &echo.participant_id, true)
        .await
        .expect("allowed");
    // Allowed, it is answered: on a budget of turns since a person last
    // spoke, so that two bots do not keep the agent talking for ever.
    // Ada's reply and Bob's words cost two turns; two more are the budget.
    for turn in 4..=5 {
        fixture_call(
            &address,
            "POST",
            "/_fixture/updates",
            Some(&from_a_bot("@swem_fixture_bot say something more")),
        );
        until("the agent answers the bot", || {
            (answers(&address) >= turn).then_some(())
        })
        .await;
    }
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&from_a_bot("@swem_fixture_bot and again")),
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert_eq!(
        answers(&address),
        5,
        "the agent went on answering a bot past the budget"
    );
    let page = state
        .chat_page(&group.chat_id, None, 100)
        .await
        .expect("the page");
    assert!(
        page.messages
            .iter()
            .any(|message| message.text == "and again"),
        "what the bot said past the budget is not in the chat for the record"
    );
    // A person's word starts the budget again.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&in_group((7, "Ada"), "@swem_fixture_bot carry on", false)),
    );
    until("the agent answers Ada", || {
        (answers(&address) >= 6).then_some(())
    })
    .await;
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&from_a_bot("@swem_fixture_bot once more")),
    );
    until("the agent answers the bot again", || {
        (answers(&address) >= 7).then_some(())
    })
    .await;

    // The chat follows the group: somebody added to it is in the chat,
    // somebody who leaves is out of it, and what they said stays.
    let joined = |who: (i64, &str)| {
        json!({
            "message": {
                "message_id": 1,
                "from": { "id": 7, "is_bot": false, "first_name": "Ada", "username": "ada" },
                "chat": { "id": -100, "type": "supergroup", "title": "Studio" },
                "date": 0,
                "new_chat_members": [
                    { "id": who.0, "is_bot": false, "first_name": who.1, "username": who.1.to_lowercase() }
                ]
            }
        })
    };
    let left = |who: (i64, &str)| {
        json!({
            "message": {
                "message_id": 1,
                "from": { "id": who.0, "is_bot": false, "first_name": who.1, "username": who.1.to_lowercase() },
                "chat": { "id": -100, "type": "supergroup", "title": "Studio" },
                "date": 0,
                "left_chat_member": { "id": who.0, "is_bot": false, "first_name": who.1, "username": who.1.to_lowercase() }
            }
        })
    };
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&joined((11, "Cy"))),
    );
    let members_named = |name: &str| {
        let chat_id = group.chat_id.clone();
        let state = state.clone();
        let name = name.to_owned();
        async move {
            state
                .chat_page(&chat_id, None, 100)
                .await
                .expect("the page")
                .chat
                .members
                .iter()
                .any(|member| member.name == name)
        }
    };
    let mut tries = 0;
    while !members_named("Cy").await {
        tries += 1;
        assert!(
            tries < 50,
            "Cy was added to the group and is not in the chat"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&left((9, "Bob"))),
    );
    let mut tries = 0;
    while members_named("Bob").await {
        tries += 1;
        assert!(tries < 50, "Bob left the group and is still in the chat");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let page = state
        .chat_page(&group.chat_id, None, 100)
        .await
        .expect("the page");
    assert!(
        page.messages
            .iter()
            .any(|message| message.text == "me too, not to the bot"),
        "what Bob said is gone with him"
    );

    state.remove_channel(&channel_id).await.expect("removed");
    let _ = api.kill();
    let _ = api.wait();
}

/// HMAC-SHA256, as the messenger signs what a page inside it is opened
/// with.
fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    let mut block = [0_u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&sha2::Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let inner: Vec<u8> = block.iter().map(|byte| byte ^ 0x36).collect();
    let outer: Vec<u8> = block.iter().map(|byte| byte ^ 0x5c).collect();
    let mut hasher = sha2::Sha256::new();
    hasher.update(&inner);
    hasher.update(message);
    let inner_hash = hasher.finalize();
    let mut hasher = sha2::Sha256::new();
    hasher.update(&outer);
    hasher.update(inner_hash);
    hasher.finalize().to_vec()
}

/// What the messenger would hand the page for somebody, signed for a bot.
fn signed_init_data(user: &Value, token: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let user = user.to_string();
    let check = format!("auth_date={now}\nuser={user}");
    let secret = hmac_sha256(b"WebAppData", token.as_bytes());
    let hash =
        hmac_sha256(&secret, check.as_bytes())
            .iter()
            .fold(String::new(), |mut hex, byte| {
                use std::fmt::Write as _;
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    let encoded = user.bytes().fold(String::new(), |mut out, byte| {
        use std::fmt::Write as _;
        if byte.is_ascii_alphanumeric() {
            out.push(byte as char);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
        out
    });
    format!("user={encoded}&auth_date={now}&hash={hash}")
}

/// The agent asks for a form: the question reaches the messenger in words,
/// and the owner answers it from the page inside the messenger - fields,
/// or declining and saying something else. A guest cannot answer it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one question: asked, read, refused to a guest, answered"
)]
async fn the_agents_form_is_answered_from_the_page_inside_the_messenger() {
    let Some(program) = telegram_channel() else {
        eprintln!(
            "skipped: build the Telegram channel first: cargo build -p swem-channel-telegram"
        );
        return;
    };
    let root = fixture_root("telegram-form");
    let mut api = std::process::Command::new(env!("CARGO_BIN_EXE_swem-telegram-api-fixture"))
        .arg("--files")
        .arg(root.join("files"))
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("start the Bot API fixture");
    let address = {
        use std::io::BufRead as _;
        let stdout = api.stdout.take().expect("stdout");
        let mut lines = std::io::BufReader::new(stdout).lines();
        lines
            .next()
            .expect("the fixture prints its address")
            .expect("a line")
    };
    let state = shell_with_one_agent(&root);
    with_the_tunnel_fixture(&root, &state);
    let token = "123456:fixture";
    let shown = state
        .add_channel(AddChannelBody {
            name: "Telegram".into(),
            package: None,
            program: Some(program.display().to_string()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Anyone,
            settings: swem_sdk::channel::Settings {
                api_root: Some(address.clone()),
                door: None,
            },
            key: Some(token.into()),
        })
        .await
        .expect("the channel is added");
    let code = shown.document.pairing_code.clone().expect("a code");
    let channel_id = shown.document.id.clone();
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((7, "Ada"), 7, &code)),
    );
    until("the welcome", || {
        sent(&address)
            .into_iter()
            .find(|call| call["method"] == "sendMessage" && call["body"]["chat_id"] == "7")
    })
    .await;

    // The page is reached through a tunnel, opened before the agent asks:
    // a question waits for an answer only so long.
    let origin = state.open_tunnel(None).await.expect("a tunnel").origin;

    // The owner asks the agent for something it answers with a form.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update(
            (7, "Ada"),
            7,
            &json!({"fixture": "elicitation-form-v0.1"}).to_string(),
        )),
    );
    let asked = until("the question reaches the messenger", || {
        sent(&address).into_iter().find(|call| {
            call["method"] == "sendMessage"
                && call["body"]["chat_id"] == "7"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("The agent asks"))
        })
    })
    .await;
    assert!(
        asked["body"]["text"]
            .as_str()
            .is_some_and(|text| text.contains("composition")),
        "the question's words are not the agent's: {asked}"
    );
    let question_id = until("the question in the ledger", || {
        let state = Arc::clone(&state);
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            handle.block_on(async {
                let chats = state.chats().await.ok()?;
                for chat in chats {
                    let page = state.chat_page(&chat.chat_id, None, 100).await.ok()?;
                    if let Some(question) = page.questions.first() {
                        return Some(question.question_id.clone());
                    }
                }
                None
            })
        })
        .join()
        .ok()
        .flatten()
    })
    .await;

    // Somebody else who opens the page cannot answer the owner's agent.
    fixture_call(
        &address,
        "POST",
        "/_fixture/updates",
        Some(&telegram_update((9, "Bob"), 9, "hi")),
    );
    // Bob is met (the agent, busy with the owner's question, answers him
    // later).
    until("Bob is met", || {
        let state = Arc::clone(&state);
        let handle = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            handle.block_on(async {
                let shown = state.channels_shown().await.ok()?;
                shown
                    .first()?
                    .people
                    .iter()
                    .any(|one| one.name == "Bob")
                    .then_some(())
            })
        })
        .join()
        .ok()
        .flatten()
    })
    .await;
    // The bot's button says what to open and under which name the
    // messenger signs who opened it.
    let button = until("the Answer button", || {
        sent(&address).into_iter().find_map(|call| {
            call["body"]["reply_markup"]["inline_keyboard"][0][0]["web_app"]["url"]
                .as_str()
                .map(str::to_owned)
        })
    })
    .await;
    assert!(
        button.starts_with(&format!("{origin}/?open=question&channel={channel_id}"))
            && button.contains("&signed=tgWebAppData")
            && button.contains(&format!("&question={question_id}")),
        "{button}"
    );
    let exchange = |data: &str| {
        ask_the_gate(
            &origin,
            "POST",
            &format!("/api/access/by-channel/{channel_id}"),
            None,
            Some(&json!({ "init_data": data }).to_string()),
        )
    };
    let (status, bob, body) = exchange(&signed_init_data(
        &json!({"id": 9, "first_name": "Bob"}),
        token,
    ));
    assert_eq!(status, 200, "{body}");
    let bob = bob.expect("a cookie for Bob");
    let (status, _, refused) = ask_the_gate(
        &origin,
        "GET",
        &format!("/api/questions/{question_id}"),
        Some(&bob),
        None,
    );
    assert_eq!(status, 403, "{refused}");
    assert!(refused.contains("owner"), "{refused}");

    // The owner reads it, with its fields, and answers it.
    let (status, ada, body) = exchange(&signed_init_data(
        &json!({"id": 7, "first_name": "Ada"}),
        token,
    ));
    assert_eq!(status, 200, "{body}");
    let ada = ada.expect("a cookie for Ada");
    let (status, _, read) = ask_the_gate(
        &origin,
        "GET",
        &format!("/api/questions/{question_id}"),
        Some(&ada),
        None,
    );
    assert_eq!(status, 200, "{read}");
    let read: Value = serde_json::from_str(&read).expect("json");
    assert_eq!(read["mode"], "form", "{read}");
    assert!(
        read["requestedSchema"]["properties"]["strategy"].is_object(),
        "{read}"
    );
    let (status, _, body) = ask_the_gate(
        &origin,
        "POST",
        &format!("/api/questions/{question_id}/answer"),
        Some(&ada),
        Some(
            &json!({"action": "accept", "content": {
                "strategy": "bold", "iterations": 2, "stems": ["voice"]
            }})
            .to_string(),
        ),
    );
    if status != 200 {
        // A question that can no longer be answered was ended by its turn:
        // how every turn ended is what explains the refusal.
        let mut turns = Vec::new();
        for chat in state.chats().await.expect("chats") {
            let page = state
                .chat_page(&chat.chat_id, None, 100)
                .await
                .expect("the chat's page");
            turns.push(serde_json::to_string(&page.deliveries).unwrap_or_default());
        }
        panic!("{body}\n{}", turns.join("\n"));
    }
    // The agent went on: its answer reaches the messenger.
    until("the agent says it was accepted", || {
        sent(&address).into_iter().find(|call| {
            call["method"] == "sendMessage"
                && call["body"]["chat_id"] == "7"
                && call["body"]["text"]
                    .as_str()
                    .is_some_and(|text| text.contains("accept"))
        })
    })
    .await;

    // And words said from the page are said in the chat, as the owner's.
    let ada_chat = state.chats().await.expect("chats")[0].chat_id.clone();
    let (status, _, body) = ask_the_gate(
        &origin,
        "POST",
        &format!("/api/chats/{ada_chat}/messages"),
        Some(&ada),
        Some(&json!({ "text": "and one more thing" }).to_string()),
    );
    assert_eq!(status, 200, "{body}");
    // The tunnel closed, the bot takes its button back.
    state.close_tunnel().await;
    until("the button taken back", || {
        sent(&address)
            .into_iter()
            .find(|call| call["method"] == "editMessageText" && call["body"]["chat_id"] == "7")
    })
    .await;

    state.remove_channel(&channel_id).await.expect("removed");
    let _ = api.kill();
    let _ = api.wait();
}

/// The tunnel fixture, installed from a file catalog as one bare executable,
/// so that a tunnel opens at the gate: what the page inside the messenger
/// is reached through from a laptop.
fn with_the_tunnel_fixture(root: &Path, state: &Arc<WorkbenchShellState>) {
    fn sha256_of(path: &Path) -> String {
        use sha2::Digest as _;
        format!(
            "{:x}",
            sha2::Sha256::digest(fs::read(path).expect("read the file"))
        )
    }
    state
        .enable_installs(root.join("installed"))
        .expect("installs");
    state
        .enable_tunnels(&root.join("tunnels"))
        .expect("tunnels");
    let fixture = PathBuf::from(env!("CARGO_BIN_EXE_swem-tunnel-fixture"));
    let platform = registry_platform().expect("a platform this test runs on");
    let catalog = swem_host::Catalog::parse(
        format!(
            r#"{{"schema":"swem:catalog@0.2","name":"Tunnels for the test","entries":[
              {{"kind":"swem/tunnel@1","id":"nowhere","name":"A tunnel to nowhere","version":"0.1.0",
                "distribution":{{"binary":{{"{platform}":{{"archive":"file://{}","sha256":"{}","cmd":"./swem-tunnel-nowhere"}}}}}}}}]}}"#,
            fixture.display(),
            sha256_of(&fixture)
        )
        .as_bytes(),
    )
    .expect("a catalog");
    state
        .enable_store(&root.join("indexes"), vec![catalog])
        .expect("the store");
    let kind = Kind::parse("swem/tunnel@1").expect("a kind");
    let planned = state
        .store_plan(&StorePlanBody {
            kind: kind.clone(),
            id: "nowhere".into(),
        })
        .expect("a plan");
    state
        .store_install(&StoreInstallBody {
            kind,
            id: "nowhere".into(),
            plan_id: planned.plan.plan_id,
        })
        .expect("installed");
}

/// What the fixture channel takes as the messenger's signed data on who
/// opened its page: `user=<json>&hash=<hmac under the bot's key>`.
fn fixture_app_data(user: &Value, key: &str) -> String {
    let user = user.to_string();
    let hash = hmac_sha256(key.as_bytes(), format!("user={user}").as_bytes())
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    format!("user={user}&hash={hash}")
}

/// One HTTP/1.1 request to the gate, by hand, with a cookie and a body:
/// the status, the cookie set if one was, and the body.
fn ask_the_gate(
    origin: &str,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Option<&str>,
) -> (u16, Option<String>, String) {
    use std::io::{Read as _, Write as _};
    let host = origin.trim_start_matches("http://");
    let mut stream = std::net::TcpStream::connect(host).expect("the gate listens");
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .expect("a timeout");
    let body = body.unwrap_or_default();
    let cookie = cookie.map_or(String::new(), |cookie| format!("Cookie: {cookie}\r\n"));
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{cookie}Connection: close\r\n\r\n{body}",
        body.len()
    )
    .expect("write the request");
    let mut answer = String::new();
    let _ = stream.read_to_string(&mut answer);
    let status: u16 = answer
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status in {answer:?}"));
    let (head, body) = answer.split_once("\r\n\r\n").expect("a body");
    let set = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("set-cookie").then(|| {
            value
                .trim()
                .split(';')
                .next()
                .unwrap_or_default()
                .to_owned()
        })
    });
    (status, set, body.to_owned())
}

/// The state whole, as a page through a tunnel asks for it: the fixture
/// tunnel holds a stream back until it ends, as a vendor's edge does, so
/// a page asks instead of listening.
fn state_asked_for(origin: &str, cookie: &str) -> Value {
    let (status, _, body) = ask_the_gate(origin, "GET", "/api/now", Some(cookie), None);
    assert_eq!(status, 200, "{body}");
    serde_json::from_str(&body).expect("the state is json")
}

/// Somebody who opens the page inside the messenger is known as themself:
/// the owner's session reaches what this bot's agent is and nothing of the
/// Workbench's setting up; a guest's reaches their own chats only; the same
/// signed data is the same session, a forged one is nobody; closing the
/// tunnel puts everybody out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "one walk over one door: the owner, a guest, what each reaches, the tunnel closed"
)]
async fn somebody_from_the_messenger_is_known_and_reaches_only_what_is_theirs() {
    let root = fixture_root("messenger-page");
    let state = shell_with_one_agent(&root);
    with_the_tunnel_fixture(&root, &state);
    let key = "fixture-key";
    let shown = state
        .add_channel(AddChannelBody {
            name: "The fixture bot".into(),
            package: None,
            program: Some(env!("CARGO_BIN_EXE_swem-channel-fixture").into()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings::default(),
            key: Some(key.into()),
        })
        .await
        .expect("the channel is added");
    assert!(shown.running, "{}", shown.said);
    let code = shown.document.pairing_code.clone().expect("a code to say");
    let channel_id = shown.document.id.clone();
    let home = root.join("channels").join(&channel_id);

    // The owner pairs and opens the bot's chat; Bob writes and is met,
    // silent.
    arrives(
        &home,
        "001",
        &message("dm-ada", &person("u-1", "Ada"), &code, "upd-1"),
    );
    until("the welcome", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "send" && call["arguments"]["chat"] == "dm-ada")
    })
    .await;
    arrives(
        &home,
        "002",
        &message("dm-ada", &person("u-1", "Ada"), "hello there", "upd-2"),
    );
    until("the agent's answer to the owner", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "stream_end" && call["arguments"]["chat"] == "dm-ada")
    })
    .await;
    arrives(
        &home,
        "003",
        &message("dm-bob", &person("u-2", "Bob"), "hi", "upd-3"),
    );
    until("Bob is told", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "send" && call["arguments"]["chat"] == "dm-bob")
    })
    .await;
    let ada_chat = state.chats().await.expect("chats")[0].chat_id.clone();
    let people = state.chat_people().await.expect("people");
    let owner_id = people["owner"]["participant_id"]
        .as_str()
        .expect("the owner")
        .to_owned();
    let bob_id = state.channels_shown().await.expect("channels")[0]
        .people
        .iter()
        .find(|one| one.name == "Bob")
        .expect("Bob was met")
        .participant_id
        .clone();

    // A tunnel: the gate is where the page is reached from a laptop.
    let origin = state.open_tunnel(None).await.expect("a tunnel").origin;
    let exchange = |data: &str| {
        ask_the_gate(
            &origin,
            "POST",
            &format!("/api/access/by-channel/{channel_id}"),
            None,
            Some(&json!({ "init_data": data }).to_string()),
        )
    };

    // The owner comes in by the messenger's signature: a session for a
    // while; the same data again is the same session; a forged one is
    // nobody.
    let ada_data = fixture_app_data(&json!({"id": "u-1", "name": "Ada"}), key);
    let (status, cookie, body) = exchange(&ada_data);
    assert_eq!(status, 200, "{body}");
    let ada = cookie.expect("a cookie for Ada");
    assert!(ada.starts_with("swem_in_messenger="), "{ada}");
    let who: Value = serde_json::from_str(&body).expect("json");
    assert_eq!(who["who"]["by"], "messenger", "{who}");
    assert_eq!(who["who"]["participant"], owner_id, "{who}");
    let (status, again, _) = exchange(&ada_data);
    assert_eq!(status, 200);
    assert_eq!(
        again.as_deref(),
        Some(ada.as_str()),
        "the same data, the same session"
    );
    let (status, none, _) = exchange(&fixture_app_data(
        &json!({"id": "u-1", "name": "Ada"}),
        "another-key",
    ));
    assert_eq!(status, 403);
    assert!(none.is_none());

    // The owner's session: this bot's agent and the chats it is in, its
    // files to read; nothing of the Workbench's setting up.
    let (status, _, standing) = ask_the_gate(&origin, "GET", "/api/access", Some(&ada), None);
    assert_eq!(status, 200);
    let standing: Value = serde_json::from_str(&standing).expect("json");
    assert_eq!(standing["messenger"]["agent"], "coder", "{standing}");
    assert_eq!(standing["messenger"]["owner"], true, "{standing}");
    assert_eq!(standing["gate"], true, "{standing}");
    for (method, path, expected) in [
        ("GET", format!("/api/chats/{ada_chat}"), 200),
        ("GET", "/api/people".to_owned(), 200),
        ("GET", "/api/profiles/coder/tree".to_owned(), 200),
        ("GET", "/api/profiles/coder/files".to_owned(), 200),
        // The Apps of a connection are the owner's within reach: the route
        // is open, and a connection nobody has is not found.
        ("GET", "/api/connections/nobody/apps".to_owned(), 404),
        ("GET", "/api/profiles".to_owned(), 403),
        ("GET", "/api/reach".to_owned(), 403),
        ("GET", "/api/access/standing".to_owned(), 403),
        ("GET", "/api/chats".to_owned(), 403),
        ("PUT", "/api/profiles/coder/file".to_owned(), 403),
        ("POST", "/api/time/due".to_owned(), 403),
        ("POST", "/api/reach/tunnel".to_owned(), 403),
    ] {
        let (status, _, body) = ask_the_gate(&origin, method, &path, Some(&ada), None);
        assert_eq!(status, expected, "{method} {path} as the owner: {body}");
    }
    // Nobody: the page and its script, the door's standing; nothing else -
    // not the passkey ceremonies, not a channel's webhook door, and no
    // word of sign-in.
    for (method, path, expected) in [
        ("GET", "/", 200),
        ("GET", "/workbench.js", 200),
        ("GET", "/api/access", 200),
        ("GET", "/api/chats", 403),
        ("GET", "/api/stream", 403),
        ("POST", "/api/access/register/begin", 403),
        ("POST", "/api/access/sign-in/begin", 403),
        ("POST", "/api/channels/some-bot/receive", 403),
        ("POST", "/api/time/due", 403),
    ] {
        let (status, _, body) = ask_the_gate(&origin, method, path, None, None);
        assert_eq!(status, expected, "{method} {path} as nobody: {body}");
        assert!(
            !body.contains("sign in"),
            "{method} {path} says something: {body}"
        );
    }
    let ada_state = state_asked_for(&origin, &ada);
    assert_eq!(ada_state["you"], owner_id);
    assert_eq!(
        ada_state["chats"].as_array().map(Vec::len),
        Some(1),
        "{ada_state}"
    );

    // Bob: met, silent, with no chat of his own yet - he reaches nothing of
    // the owner's. Then allowed, his chat opens, and he reaches that.
    let bob_data = fixture_app_data(&json!({"id": "u-2", "name": "Bob"}), key);
    let (status, cookie, body) = exchange(&bob_data);
    assert_eq!(status, 200, "{body}");
    let bob = cookie.expect("a cookie for Bob");
    assert_ne!(bob, ada);
    for (method, path) in [
        ("GET", format!("/api/chats/{ada_chat}")),
        ("GET", "/api/profiles/coder/tree".to_owned()),
        ("GET", "/api/questions/q-1".to_owned()),
        ("GET", "/api/connections/nobody/apps".to_owned()),
        ("POST", format!("/api/chats/{ada_chat}/messages")),
    ] {
        let (status, _, body) = ask_the_gate(
            &origin,
            method,
            &path,
            Some(&bob),
            Some(&json!({ "text": "mine?" }).to_string()),
        );
        assert_eq!(status, 403, "{method} {path} as Bob: {body}");
    }
    let bob_state = state_asked_for(&origin, &bob);
    assert_eq!(bob_state["you"], bob_id);
    assert_eq!(
        bob_state["chats"].as_array().map(Vec::len),
        Some(0),
        "{bob_state}"
    );
    assert!(bob_state["owner"].is_null(), "{bob_state}");
    let (status, _, people) = ask_the_gate(&origin, "GET", "/api/people", Some(&bob), None);
    assert_eq!(status, 200);
    assert!(
        !people.contains("Ada"),
        "Bob is told of the owner: {people}"
    );
    // What happened, asked for - the way a page asks through a tunnel:
    // nothing of the owner's chat reaches Bob.
    let (status, _, happened) =
        ask_the_gate(&origin, "GET", "/api/happened?after=0", Some(&bob), None);
    assert_eq!(status, 200, "{happened}");
    let happened: Value = serde_json::from_str(&happened).expect("json");
    assert!(
        happened["events"]
            .as_array()
            .is_some_and(|events| events.iter().all(|event| event["chat_id"] != ada_chat)),
        "{happened}"
    );

    state
        .allow_guest_at(&channel_id, &bob_id, true)
        .await
        .expect("allowed");
    arrives(
        &home,
        "004",
        &message("dm-bob", &person("u-2", "Bob"), "now I may", "upd-4"),
    );
    until("the agent's answer to Bob", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "stream_end" && call["arguments"]["chat"] == "dm-bob")
    })
    .await;
    let bob_state = state_asked_for(&origin, &bob);
    let bob_chats = bob_state["chats"].as_array().cloned().unwrap_or_default();
    assert_eq!(bob_chats.len(), 1, "{bob_state}");
    let bob_chat = bob_chats[0]["chat_id"].as_str().expect("a chat").to_owned();
    assert_ne!(bob_chat, ada_chat);
    let (status, _, body) = ask_the_gate(
        &origin,
        "GET",
        &format!("/api/chats/{bob_chat}"),
        Some(&bob),
        None,
    );
    assert_eq!(status, 200, "{body}");
    // Said from the page as Bob, through the messenger's channel, and
    // answered as anything he says is.
    let ended = calls(&home)
        .into_iter()
        .filter(|call| call["tool"] == "stream_end" && call["arguments"]["chat"] == "dm-bob")
        .count();
    let (status, _, body) = ask_the_gate(
        &origin,
        "POST",
        &format!("/api/chats/{bob_chat}/messages"),
        Some(&bob),
        Some(&json!({ "text": "from the page" }).to_string()),
    );
    assert_eq!(status, 200, "{body}");
    until("the agent's answer to what Bob said from the page", || {
        (calls(&home)
            .into_iter()
            .filter(|call| call["tool"] == "stream_end" && call["arguments"]["chat"] == "dm-bob")
            .count()
            > ended)
            .then_some(())
    })
    .await;
    let page = state
        .chat_page(&bob_chat, None, 100)
        .await
        .expect("Bob's page");
    let said = page
        .messages
        .iter()
        .find(|message| message.text == "from the page")
        .expect("said in the chat");
    assert_eq!(said.sender_id, bob_id);
    assert_eq!(said.channel, format!("channel:{channel_id}"));
    // Forbidden again, what he says from the page is for the record: no turn.
    state
        .allow_guest_at(&channel_id, &bob_id, false)
        .await
        .expect("forbidden");
    let begun = calls(&home)
        .into_iter()
        .filter(|call| call["tool"] == "stream_begin" && call["arguments"]["chat"] == "dm-bob")
        .count();
    let (status, _, body) = ask_the_gate(
        &origin,
        "POST",
        &format!("/api/chats/{bob_chat}/messages"),
        Some(&bob),
        Some(&json!({ "text": "for the record" }).to_string()),
    );
    assert_eq!(status, 200, "{body}");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        calls(&home)
            .into_iter()
            .filter(|call| call["tool"] == "stream_begin" && call["arguments"]["chat"] == "dm-bob")
            .count(),
        begun,
        "a forbidden guest made the agent speak"
    );

    // The tunnel closed, everybody is out: a new tunnel does not know the
    // old cookie.
    state.close_tunnel().await;
    let origin = state
        .open_tunnel(None)
        .await
        .expect("a tunnel again")
        .origin;
    let (status, _, _) = ask_the_gate(
        &origin,
        "GET",
        &format!("/api/chats/{ada_chat}"),
        Some(&ada),
        None,
    );
    assert_eq!(status, 403, "the old session outlived the tunnel");
    state.close_tunnel().await;
    state.remove_channel(&channel_id).await.expect("removed");
}

/// One agent of the echo engine with the Apps fixture attached as `notes`,
/// observed, so that a tool it calls is seen to bring an App; `undeclared`
/// leaves the notes App silent about where it works.
fn shell_with_an_app(root: &Path, undeclared: bool) -> Arc<WorkbenchShellState> {
    let inventory = root.join("inventory");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    let workspace = root.join("coder");
    let agent_home = root.join("coder-home");
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&agent_home).expect("create agent home");
    store
        .create(
            &PersonalAgentProfile::new(
                "coder",
                "swem-echo-agent",
                "echo-fixture-distribution",
                "direct-fixture-environment",
                swem_host::ASK_EVERY_TIME,
                &workspace,
                &agent_home,
                vec![AttachmentBinding::new(
                    "notes-attachment",
                    "notes",
                    AttachmentTransport::Stdio,
                )],
                Vec::new(),
            )
            .expect("build fixture profile"),
        )
        .expect("persist fixture profile");
    let supply = root.to_path_buf();
    let state = WorkbenchShellState::open(
        &inventory,
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        move |_profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            let mut args = vec![
                "--receipt".to_owned(),
                supply.join("notes-receipt.json").display().to_string(),
                "--poison".to_owned(),
                supply.join("notes-poison.json").display().to_string(),
            ];
            if undeclared {
                args.push("--undeclared".to_owned());
            }
            let notes = agent_client_protocol::schema::v1::McpServer::Stdio(
                agent_client_protocol::schema::v1::McpServerStdio::new(
                    "notes",
                    PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-apps-fixture")),
                )
                .args(args),
            );
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: vec![notes],
            })
        },
    )
    .expect("open shell state");
    let state = Arc::new(state);
    state.set_mcp_observer_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-mcp-observer-fixture")),
        Vec::new(),
    );
    state
        .enable_provider_keys_kept_by(Arc::new(
            swem_host::InFiles::at(&root.join("secrets")).expect("a place for keys"),
        ))
        .expect("keys");
    state
        .enable_channels(&root.join("channels"))
        .expect("channels");
    state
}

/// The bot offers the page's Apps tab for an App a tool brought only when
/// the App declared it works on a phone; an App that declared nothing is
/// said to be for the Workbench, with no button.
async fn an_app_the_agent_brought_is_offered_where_it_works(undeclared: bool) {
    let root = fixture_root(if undeclared {
        "app-undeclared"
    } else {
        "app-declared"
    });
    let state = shell_with_an_app(&root, undeclared);
    with_the_tunnel_fixture(&root, &state);
    let shown = state
        .add_channel(AddChannelBody {
            name: "The fixture bot".into(),
            package: None,
            program: Some(env!("CARGO_BIN_EXE_swem-channel-fixture").into()),
            args: Vec::new(),
            agent: Some("coder".into()),
            guests: GuestPolicy::Nobody,
            settings: swem_sdk::channel::Settings::default(),
            key: Some("fixture-key".into()),
        })
        .await
        .expect("the channel is added");
    assert!(shown.running, "{}", shown.said);
    let code = shown.document.pairing_code.clone().expect("a code to say");
    let channel_id = shown.document.id.clone();
    let home = root.join("channels").join(&channel_id);
    arrives(
        &home,
        "001",
        &message("dm-ada", &person("u-1", "Ada"), &code, "upd-1"),
    );
    until("the welcome", || {
        calls(&home)
            .into_iter()
            .find(|call| call["tool"] == "send" && call["arguments"]["chat"] == "dm-ada")
    })
    .await;
    // An address from outside, so a button could be sent.
    state.open_tunnel(None).await.expect("a tunnel");
    // The owner asks for something a tool answers with an App.
    arrives(
        &home,
        "002",
        &message(
            "dm-ada",
            &person("u-1", "Ada"),
            &json!({"fixture": "mcp-apps-note-v0.1", "server": "notes", "nonce": "n-1", "text": "a note"}).to_string(),
            "upd-2",
        ),
    );
    let said = until("the bot speaks of the App", || {
        calls(&home).into_iter().find(|call| {
            call["tool"] == "send"
                && call["arguments"]["chat"] == "dm-ada"
                && call["arguments"]["markdown"]
                    .as_str()
                    .is_some_and(|text| text.contains("brought an App"))
        })
    })
    .await;
    let words = said["arguments"]["markdown"].as_str().unwrap_or_default();
    if undeclared {
        assert!(words.contains("for the Workbench"), "{said}");
        assert!(
            said["arguments"]["app"].is_null(),
            "a button for an App that works elsewhere: {said}"
        );
    } else {
        assert!(words.contains("Notes"), "the App by its title: {said}");
        let url = said["arguments"]["app"]["url"].as_str().unwrap_or_default();
        assert!(
            url.contains("open=apps") && url.contains("&chat="),
            "{said}"
        );
    }
    state.close_tunnel().await;
    state.remove_channel(&channel_id).await.expect("removed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_app_declared_for_a_phone_is_offered_by_the_bot() {
    an_app_the_agent_brought_is_offered_where_it_works(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_app_that_declared_nothing_is_said_to_be_for_the_workbench() {
    an_app_the_agent_brought_is_offered_where_it_works(true).await;
}
