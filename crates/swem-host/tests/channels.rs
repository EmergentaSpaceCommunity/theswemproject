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
    AddChannelBody, AttachmentBinding, AttachmentTransport, GuestPolicy, IntegrationKind,
    LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore, ResolvedDirectAgentConnection,
    WorkbenchShellState,
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
            .contains("owner only"),
        "{refused}"
    );
    let chats = state.chats().await.expect("chats");
    assert_eq!(chats.len(), 1, "a stranger opened no chat");

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
