//! A message into a chat reaches its agents (ADR-0007): each agent works
//! one turn at a time, several agents work at once, a question waits in the
//! ledger, and a chat goes on when its engine's session is gone. The agents
//! are the echo fixture - no Claude, no Cycle.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use swem_host::{
    AttachmentBinding, AttachmentTransport, ChatPage, DeliveryState, IntegrationKind,
    LaunchCommand, PersonalAgentProfile, PersonalAgentProfileStore, QuestionState,
    ResolvedDirectAgentConnection, RoutingLedger, Saying, StartChatBody, WorkbenchShellError,
    WorkbenchShellState,
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-chats-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

/// Two agents of the echo engine, each in a working directory of its own.
fn shell_of_two(root: &Path) -> (Arc<WorkbenchShellState>, PathBuf) {
    shell_with_deadline(root, Duration::from_secs(20))
}

fn shell_with_deadline(root: &Path, deadline: Duration) -> (Arc<WorkbenchShellState>, PathBuf) {
    let inventory = root.join("inventory");
    let ledger = root.join("routes.sqlite3");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open profile inventory");
    for profile_id in ["coder", "reviewer"] {
        let workspace = root.join(profile_id);
        let agent_home = root.join(format!("{profile_id}-home"));
        fs::create_dir_all(&workspace).expect("create workspace");
        fs::create_dir_all(&agent_home).expect("create agent home");
        store
            .create(
                &PersonalAgentProfile::new(
                    profile_id,
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
    }
    let receipt = root.join("mcp-receipt.json");
    let state = WorkbenchShellState::open(&inventory, &ledger, deadline, move |_profile| {
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
    })
    .expect("open shell state");
    (Arc::new(state), ledger)
}

fn saying(text: &str) -> Saying {
    Saying {
        text: text.to_owned(),
        ..Saying::default()
    }
}

/// Read the chat until it shows what is waited for, as a page would.
async fn until(
    state: &WorkbenchShellState,
    chat_id: &str,
    what: &str,
    shows: impl Fn(&ChatPage) -> bool,
) -> ChatPage {
    let began = std::time::Instant::now();
    loop {
        let page = state
            .chat_page(chat_id, None, 500)
            .await
            .expect("read the chat");
        if shows(&page) {
            return page;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "waited thirty seconds for {what}; the chat shows {} messages, {} deliveries, {} questions",
            page.messages.len(),
            page.deliveries.len(),
            page.questions.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn settled(messages: usize) -> impl Fn(&ChatPage) -> bool {
    move |page| page.deliveries.is_empty() && page.messages.len() >= messages
}

/// What the engine was given in its turns in a chat, in order.
fn prompts(page: &ChatPage) -> Vec<Vec<String>> {
    page.events
        .iter()
        .filter(|event| event.kind == "host/prompt_submitted")
        .map(|event| {
            event.payload["content"]
                .as_array()
                .expect("content blocks")
                .iter()
                .map(|block| block["text"].as_str().unwrap_or("").to_owned())
                .collect()
        })
        .collect()
}

fn reply(page: &ChatPage, index: usize) -> Value {
    serde_json::from_str(&page.messages[index].text).expect("the fixture answers in JSON")
}

#[tokio::test]
async fn a_message_into_a_chat_is_answered_and_each_has_its_sender() {
    let root = fixture_root("answered");
    let (state, _ledger) = shell_of_two(&root);
    let people = state.chat_people().await.expect("people");
    let owner = people["owner"]["participant_id"]
        .as_str()
        .expect("owner")
        .to_owned();
    assert_eq!(
        people["participants"].as_array().expect("everybody").len(),
        3
    );

    let chat = state
        .start_chat(StartChatBody {
            title: String::new(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("start a chat");
    let coder = chat.members[1].participant_id.clone();
    assert!(
        matches!(
            state
                .start_chat(StartChatBody {
                    title: String::new(),
                    agents: vec!["nobody".into()]
                })
                .await,
            Err(WorkbenchShellError::NotFound(_))
        ),
        "an agent is a profile"
    );

    let said = state
        .say_in_chat(&chat.chat_id, saying("/status and then look"))
        .await
        .expect("say");
    assert_eq!(said.message.sender_id, owner);
    assert_eq!(said.deliveries.len(), 1);
    assert_eq!(said.deliveries[0].state, DeliveryState::Queued);
    let page = until(&state, &chat.chat_id, "the answer", settled(2)).await;
    assert_eq!(page.chat.title, "/status and then look");
    assert_eq!(
        page.messages
            .iter()
            .map(|message| (message.sender_id.as_str(), message.channel.as_str()))
            .collect::<Vec<_>>(),
        vec![(owner.as_str(), "workbench"), (coder.as_str(), "agent")]
    );
    assert_eq!(reply(&page, 1)["prompt"], "/status and then look");
    assert_eq!(reply(&page, 1)["turn"], 1);

    // The person's words are the turn, as typed; the host's block ends it.
    let given = prompts(&page);
    assert_eq!(given.len(), 1);
    assert_eq!(given[0][0], "/status and then look");
    let block = given[0].last().expect("the block");
    assert!(block.starts_with("<swem:turn k=\""), "{block}");
    assert!(block.contains("to: \"coder\""));
    assert!(block.contains("why: \"the only agent in the chat\""));
    assert!(block.contains("\"trust\":\"principal\""));
    assert!(block.contains("\"said_above\":true"));
    assert!(
        !block.contains("\"said\":"),
        "a principal's words are not wrapped"
    );
    assert!(!block.contains("swem:earlier"));

    // The agent was told who it is and how it is told who speaks.
    let instructions =
        fs::read_to_string(root.join("coder/AGENTS.md")).expect("the instruction file");
    assert!(instructions.contains("You are **coder** (`@coder`), an agent of"));
    assert!(instructions.contains("## Who is speaking"));

    // The chat goes on in the same session of the engine.
    state
        .say_in_chat(&chat.chat_id, saying("and again"))
        .await
        .expect("say again");
    let page = until(&state, &chat.chat_id, "the second answer", settled(4)).await;
    assert_eq!(reply(&page, 3)["turn"], 2);
    assert_eq!(reply(&page, 3)["session_id"], reply(&page, 1)["session_id"]);
    // Its own answer is not told back to it.
    assert!(
        !prompts(&page)[1]
            .last()
            .expect("the block")
            .contains("swem:earlier")
    );

    state.let_go_of(None).await;
    assert!(state.active_connections().await.is_empty());
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn a_message_is_said_once_and_nothing_is_not_a_message() {
    let root = fixture_root("once");
    let (state, _ledger) = shell_of_two(&root);
    let chat = state
        .start_chat(StartChatBody {
            title: "Once".into(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("start a chat");
    // Said again with the same name of the sender's, it is the same message.
    let named = Saying {
        client_ref: Some("m-1".into()),
        ..saying("once")
    };
    let first = state
        .say_in_chat(&chat.chat_id, named.clone())
        .await
        .expect("say");
    let again = state
        .say_in_chat(&chat.chat_id, named)
        .await
        .expect("say again");
    assert_eq!(first.message.message_id, again.message.message_id);
    assert_eq!(
        first.deliveries[0].delivery_id,
        again.deliveries[0].delivery_id
    );
    until(&state, &chat.chat_id, "the third answer", settled(2)).await;

    assert!(matches!(
        state.say_in_chat(&chat.chat_id, saying("  ")).await,
        Err(WorkbenchShellError::Invalid(_))
    ));
    assert!(matches!(
        state.say_in_chat("c_none", saying("hello")).await,
        Err(WorkbenchShellError::NotFound(_))
    ));
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn two_agents_work_at_once_and_a_question_waits_for_whoever_comes_back() {
    let root = fixture_root("at-once");
    let (state, ledger) = shell_of_two(&root);
    let start = |agent: &str| StartChatBody {
        title: format!("with {agent}"),
        agents: vec![agent.to_owned()],
    };
    let asking = state.start_chat(start("coder")).await.expect("chat");
    let other = state.start_chat(start("reviewer")).await.expect("chat");

    // The first agent asks before it runs something, and waits.
    let wants = json!({ "fixture": "mcp-echo-permission-v0.1", "server": "echo", "nonce": "kept" });
    let owed = state
        .say_in_chat(&asking.chat_id, saying(&wants.to_string()))
        .await
        .expect("say");
    let page = until(&state, &asking.chat_id, "the question", |page| {
        !page.questions.is_empty()
    })
    .await;
    assert_eq!(page.deliveries.len(), 1);
    assert_eq!(page.deliveries[0].state, DeliveryState::Running);
    let question = page.questions[0].clone();
    assert_eq!(question.kind, "permission");
    assert_eq!(question.state, QuestionState::Waiting);
    assert_eq!(
        question.delivery_id.as_deref(),
        Some(owed.deliveries[0].delivery_id.as_str())
    );
    let options = question.asked["options"].as_array().expect("options");
    let allow = options
        .iter()
        .find(|option| option["kind"] == "allow_once")
        .expect("the agent offered to be allowed once")["optionId"]
        .as_str()
        .expect("an id")
        .to_owned();

    // Meanwhile the second agent answers in its own chat.
    state
        .say_in_chat(&other.chat_id, saying("while the other waits"))
        .await
        .expect("say");
    let answered = until(
        &state,
        &other.chat_id,
        "the other agent's answer",
        settled(2),
    )
    .await;
    assert_eq!(reply(&answered, 1)["prompt"], "while the other waits");

    // Nobody is looking; whoever comes back reads the question in the chat.
    let came_back = state
        .chat_page(&asking.chat_id, None, 500)
        .await
        .expect("read the chat");
    assert_eq!(came_back.questions.len(), 1);
    assert_eq!(came_back.questions[0].question_id, question.question_id);
    assert!(matches!(
        state
            .answer_in_chat(&question.question_id, json!({ "option": "not-offered" }))
            .await,
        Err(WorkbenchShellError::Conflict(_))
    ));
    let kept = state
        .answer_in_chat(&question.question_id, json!({ "option": allow }))
        .await
        .expect("answer");
    assert_eq!(kept.state, QuestionState::Answered);
    assert_eq!(kept.answer.as_ref().expect("the answer")["option"], allow);
    let page = until(&state, &asking.chat_id, "the turn to finish", settled(2)).await;
    assert!(page.questions.is_empty());
    assert!(page.messages[1].text.contains("kept"));
    assert!(
        root.join("mcp-receipt.json").is_file(),
        "what was allowed was done"
    );
    assert!(matches!(
        state
            .answer_in_chat(&question.question_id, json!({ "option": allow }))
            .await,
        Err(WorkbenchShellError::Conflict(_))
    ));
    assert_eq!(
        RoutingLedger::open(&ledger)
            .expect("ledger")
            .delivery(&owed.deliveries[0].delivery_id)
            .expect("delivery")
            .state,
        DeliveryState::Done
    );
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn a_turn_does_not_run_out_while_its_question_waits_for_a_person() {
    let root = fixture_root("deadline");
    let (state, ledger) = shell_with_deadline(&root, Duration::from_secs(2));
    let chat = state
        .start_chat(StartChatBody {
            title: "Deadline".into(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("chat");
    let wants = json!({ "fixture": "mcp-echo-permission-v0.1", "server": "echo", "nonce": "late" });
    let owed = state
        .say_in_chat(&chat.chat_id, saying(&wants.to_string()))
        .await
        .expect("say");
    let page = until(&state, &chat.chat_id, "the question", |page| {
        !page.questions.is_empty()
    })
    .await;
    // The person takes longer to answer than a turn is given.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let option = page.questions[0].asked["options"]
        .as_array()
        .expect("options")
        .iter()
        .find(|option| option["kind"] == "allow_once")
        .expect("allow once")["optionId"]
        .as_str()
        .expect("an id")
        .to_owned();
    state
        .answer_in_chat(&page.questions[0].question_id, json!({ "option": option }))
        .await
        .expect("the question still waits");
    let page = until(&state, &chat.chat_id, "the turn to finish", settled(2)).await;
    assert!(page.messages[1].text.contains("late"));
    assert_eq!(
        RoutingLedger::open(&ledger)
            .expect("ledger")
            .delivery(&owed.deliveries[0].delivery_id)
            .expect("delivery")
            .state,
        DeliveryState::Done
    );
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn a_turn_is_stopped_and_what_waited_behind_it_is_not_begun() {
    let root = fixture_root("stopped");
    let (state, ledger) = shell_of_two(&root);
    let chat = state
        .start_chat(StartChatBody {
            title: "Stop".into(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("chat");
    let long = state
        .say_in_chat(
            &chat.chat_id,
            saying(&json!({ "fixture": "cancel-v0.1" }).to_string()),
        )
        .await
        .expect("say");
    let behind = state
        .say_in_chat(&chat.chat_id, saying("after that"))
        .await
        .expect("say");
    until(&state, &chat.chat_id, "the turn to begin", |page| {
        page.deliveries
            .iter()
            .any(|delivery| delivery.state == DeliveryState::Running)
            && page
                .events
                .iter()
                .any(|event| event.kind == "host/prompt_submitted")
    })
    .await;
    let stopped = state.stop_in_chat(&chat.chat_id, None).await.expect("stop");
    assert_eq!(stopped["not_begun"], 1);
    assert_eq!(stopped["stopped"].as_array().expect("agents").len(), 1);
    let page = until(
        &state,
        &chat.chat_id,
        "the chat to come to rest",
        settled(2),
    )
    .await;
    assert_eq!(page.messages.len(), 2, "nothing was answered");
    let mut ledger = RoutingLedger::open(&ledger).expect("ledger");
    for owed in [&long, &behind] {
        assert_eq!(
            ledger
                .delivery(&owed.deliveries[0].delivery_id)
                .expect("delivery")
                .state,
            DeliveryState::Stopped
        );
    }
    // The chat is not over: the next message is answered.
    state
        .say_in_chat(&chat.chat_id, saying("go on"))
        .await
        .expect("say");
    let page = until(&state, &chat.chat_id, "the answer", settled(4)).await;
    assert_eq!(reply(&page, 3)["prompt"], "go on");
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn a_chat_goes_on_when_the_engine_no_longer_has_its_session() {
    let root = fixture_root("fresh");
    let (state, ledger) = shell_of_two(&root);
    let chat = state
        .start_chat(StartChatBody {
            title: "Release notes".into(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("chat");
    let coder = chat.members[1].participant_id.clone();
    state
        .say_in_chat(&chat.chat_id, saying("draft the notes"))
        .await
        .expect("say");
    let page = until(&state, &chat.chat_id, "the answer", settled(2)).await;
    let first_session = reply(&page, 1)["session_id"].clone();
    let first = RoutingLedger::open(&ledger)
        .expect("ledger")
        .current_session(&chat.chat_id, &coder)
        .expect("session")
        .expect("the chat holds one");

    // The Workbench lets go; the engine forgets.
    state.let_go_of(None).await;
    fs::remove_dir_all(root.join("coder/.swem-echo-agent-sessions"))
        .expect("the engine's sessions");
    let _ = fs::remove_file(root.join("coder/.swem-echo-agent-session.json"));

    state
        .say_in_chat(&chat.chat_id, saying("now shorten them"))
        .await
        .expect("say");
    let page = until(
        &state,
        &chat.chat_id,
        "the answer of a fresh session",
        settled(4),
    )
    .await;
    assert_eq!(reply(&page, 3)["prompt"], "now shorten them");
    assert_eq!(reply(&page, 3)["turn"], 1, "the engine began again");
    assert_ne!(reply(&page, 3)["session_id"], first_session);

    let ledger = RoutingLedger::open(&ledger).expect("ledger");
    let second = ledger
        .current_session(&chat.chat_id, &coder)
        .expect("session")
        .expect("the chat holds one");
    assert_ne!(second.route_id, first.route_id);
    assert!(
        !ledger
            .session_of_route(&first.route_id)
            .expect("the earlier session")
            .expect("is kept")
            .current
    );
    assert_eq!(ledger.chats_of(&chat.created_by).expect("chats").len(), 1);

    // The fresh session was given what was said before, and where the rest is.
    let given = prompts(&page);
    let block = given
        .last()
        .expect("a turn")
        .last()
        .expect("the block")
        .clone();
    assert!(block.contains("<swem:earlier k="), "{block}");
    assert!(block.contains("\"said\":\"draft the notes\""));
    assert!(
        block.contains("\"from\":\"coder\""),
        "its own words too: it remembers nothing"
    );
    let transcript = block
        .lines()
        .find_map(|line| line.strip_prefix("transcript: "))
        .expect("the block says where the whole of it is");
    let transcript: String = serde_json::from_str(transcript).expect("a path");
    let whole = fs::read_to_string(&transcript).expect("the file is there");
    assert!(whole.starts_with("# Release notes\n"));
    assert!(whole.contains("draft the notes"));
    assert!(
        Path::new(&transcript).starts_with(root.join("coder").canonicalize().expect("workspace"))
    );
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// A short request of a page, answered in JSON.
async fn ask(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    body: &Value,
) -> (u16, Value) {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let body = if body.is_null() {
        String::new()
    } else {
        body.to_string()
    };
    let mut stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("connect");
    stream
        .write_all(
            format!(
                "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\
                 Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .expect("ask");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).await.expect("answer");
    let status = answer
        .split_whitespace()
        .nth(1)
        .expect("a status")
        .parse()
        .expect("a number");
    let (_, said) = answer.split_once("\r\n\r\n").expect("a body");
    (status, serde_json::from_str(said).expect("JSON"))
}

/// The stream a page follows, read frame by frame.
struct Following {
    stream: tokio::net::TcpStream,
    read: String,
}

impl Following {
    async fn from(address: std::net::SocketAddr, place: Option<u64>) -> Self {
        use tokio::io::AsyncWriteExt as _;
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect");
        let place = place.map_or_else(String::new, |place| format!("Last-Event-ID: {place}\r\n"));
        stream
            .write_all(
                format!("GET /api/stream HTTP/1.1\r\nHost: 127.0.0.1\r\n{place}\r\n").as_bytes(),
            )
            .await
            .expect("ask for the stream");
        Self {
            stream,
            read: String::new(),
        }
    }

    /// The next frame: its name, its place and what it says.
    async fn next(&mut self) -> (String, u64, Value) {
        use tokio::io::AsyncReadExt as _;
        loop {
            // A frame is `id`, `event` and `data`, each on a line of its own.
            if let Some(at) = self.read.find("data: ")
                && let Some(end) = self.read[at..].find('\n')
            {
                let before = self.read[..at].to_owned();
                let data = self.read[at + 6..at + end].to_owned();
                self.read.drain(..at + end);
                let line = |name: &str| {
                    before
                        .lines()
                        .rev()
                        .find_map(|line| line.strip_prefix(name))
                        .unwrap_or("")
                        .to_owned()
                };
                return (
                    line("event: "),
                    line("id: ").parse().expect("a place"),
                    serde_json::from_str(&data).expect("JSON"),
                );
            }
            let mut bytes = [0_u8; 8192];
            let count = tokio::time::timeout(Duration::from_secs(20), self.stream.read(&mut bytes))
                .await
                .expect("the stream went quiet for twenty seconds")
                .expect("read the stream");
            assert!(count > 0, "the stream ended");
            self.read
                .push_str(&String::from_utf8_lossy(&bytes[..count]));
        }
    }
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one page from opening to coming back, in the order a person does it"
)]
async fn a_page_follows_every_chat_by_one_stream_and_comes_back_to_its_place() {
    let root = fixture_root("stream");
    let (state, _ledger) = shell_of_two(&root);
    let handle = swem_host::serve_workbench_http(Arc::clone(&state), ([127, 0, 0, 1], 0).into())
        .await
        .expect("serve");
    let address = handle.local_addr;

    // A page that comes with no place is given the state whole.
    let mut page = Following::from(address, None).await;
    let (name, began, now) = page.next().await;
    assert_eq!(name, "state");
    assert_eq!(now["head"], began);
    assert_eq!(now["owner"]["kind"], "person");
    assert_eq!(now["participants"].as_array().expect("everybody").len(), 3);
    assert_eq!(now["chats"], json!([]));

    let (status, chat) = ask(
        address,
        "POST",
        "/api/chats",
        &json!({ "agents": ["coder"] }),
    )
    .await;
    assert_eq!(status, 200, "{chat}");
    let chat_id = chat["chat_id"].as_str().expect("a chat").to_owned();
    let (status, refused) = ask(address, "POST", "/api/chats", &json!({ "agents": [] })).await;
    assert_eq!(status, 400, "{refused}");
    let (status, said) = ask(
        address,
        "POST",
        &format!("/api/chats/{chat_id}/messages"),
        &json!({ "text": "hello", "client_ref": "m-1" }),
    )
    .await;
    assert_eq!(status, 200, "{said}");
    assert_eq!(said["deliveries"][0]["state"], "queued");

    // Everything arrives on the one stream, in the one order, the messages
    // with who sent them.
    let mut kinds = Vec::new();
    let mut messages = Vec::new();
    let mut place = began;
    loop {
        let (name, at, event) = page.next().await;
        assert_eq!(name, "event");
        assert!(at > place, "places only grow");
        place = at;
        assert_eq!(event["chat_id"], chat_id.as_str());
        let kind = event["kind"].as_str().expect("a kind").to_owned();
        if let Some(message) = event.get("message") {
            messages.push((
                message["sender_id"].as_str().expect("a sender").to_owned(),
                message["channel"].as_str().expect("a channel").to_owned(),
            ));
        }
        let done = kind == "chat/delivery" && event["payload"]["state"] == "done";
        kinds.push(kind);
        if done {
            break;
        }
    }
    assert_eq!(
        kinds[..3],
        ["chat/started", "chat/message", "chat/delivery"]
    );
    assert!(kinds.iter().any(|kind| kind == "acp/session_update"));
    let owner = now["owner"]["participant_id"].as_str().expect("owner");
    let coder = chat["members"][1]["participant_id"]
        .as_str()
        .expect("agent");
    assert_eq!(
        messages,
        vec![
            (owner.to_owned(), "workbench".to_owned()),
            (coder.to_owned(), "agent".to_owned())
        ]
    );

    // The chat is read whole by a short request beside the stream.
    let (status, read) = ask(
        address,
        "GET",
        &format!("/api/chats/{chat_id}"),
        &Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(read["chat"]["title"], "hello");
    assert_eq!(read["messages"].as_array().expect("messages").len(), 2);
    assert_eq!(read["deliveries"], json!([]));
    let (status, renamed) = ask(
        address,
        "PATCH",
        &format!("/api/chats/{chat_id}"),
        &json!({ "title": "Greetings" }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(renamed["title"], "Greetings");
    let (status, missing) = ask(address, "GET", "/api/chats/c_none", &Value::Null).await;
    assert_eq!(status, 404, "{missing}");

    // A page that comes back with its place is given what it missed and
    // nothing it has.
    drop(page);
    let mut back = Following::from(address, Some(place)).await;
    let (name, at, event) = back.next().await;
    assert_eq!(name, "event");
    assert!(at > place);
    assert_eq!(event["kind"], "chat/renamed");
    assert_eq!(event["payload"]["title"], "Greetings");

    // A place the record never reached is a place in another record.
    let mut lost = Following::from(address, Some(place + 1_000_000)).await;
    let (name, _, now) = lost.next().await;
    assert_eq!(name, "reset");
    assert_eq!(now["chats"][0]["title"], "Greetings");

    state.let_go_of(None).await;
    drop(handle);
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// The moment a schedule is next due, for a test that takes it up then.
async fn next_due(state: &WorkbenchShellState, schedule_id: &str) -> u64 {
    let (schedules, _) = state.schedules_shown(None).await.expect("schedules");
    schedules
        .iter()
        .find(|shown| shown.schedule.schedule_id == schedule_id)
        .and_then(|shown| shown.schedule.next_due_ms)
        .expect("the schedule is due some time")
}

#[tokio::test]
async fn what_the_old_clock_kept_is_said_on_time_by_the_schedule() {
    let root = fixture_root("clock");
    let (state, _ledger) = shell_of_two(&root);
    // What the clock before this one kept, as it kept it.
    let kept = root.join("schedules");
    fs::create_dir_all(&kept).expect("the old directory");
    fs::write(
        kept.join("morning.json"),
        json!({
            "schema": "swem.workbench-schedule.v1",
            "schedule_id": "morning",
            "profile_id": "coder",
            "say": "how are we doing",
            "every_minutes": 60,
            "enabled": true,
        })
        .to_string(),
    )
    .expect("an old schedule");
    state.keep_time(&kept).await.expect("keep time");
    assert!(
        !kept.exists(),
        "the old schedules were left where they were"
    );
    assert!(root.join("schedules.v1").join("morning.json").is_file());

    let (schedules, runs) = state.schedules_shown(None).await.expect("schedules");
    assert!(runs.is_empty());
    assert_eq!(schedules.len(), 1);
    let schedule = &schedules[0];
    assert_eq!(schedule.schedule.say, "how are we doing");
    assert_eq!(schedule.schedule.when_in_words, "Every hour");
    assert!(schedule.schedule.enabled);
    assert_eq!(schedule.chat_title.as_deref(), Some("morning"));
    let standing = state.keeper().await.expect("who keeps time");
    assert!(standing.keeping);
    assert_eq!(standing.used_by, ["coder"]);

    // When it is due it is said, by the schedule, and answered.
    let due = state
        .claim_what_is_due(next_due(&state, &schedule.schedule.schedule_id).await)
        .await;
    assert_eq!(due.len(), 1);
    let chat_id = schedule.schedule.chat_id.clone().expect("its chat");
    state.say_what_is_due(due[0].clone()).await;
    let page = until(&state, &chat_id, "the answer", settled(2)).await;
    let clock = page
        .chat
        .members
        .iter()
        .find(|member| member.kind == swem_host::ParticipantKind::Schedule)
        .expect("the schedule is in the chat it speaks in");
    assert_eq!(clock.participant_id, schedule.schedule.speaker_id);
    assert_eq!(clock.name, "Every hour");
    assert_eq!(
        clock.made_by.as_deref(),
        Some(page.chat.created_by.as_str())
    );
    assert_eq!(page.messages[0].sender_id, clock.participant_id);
    assert_eq!(page.messages[0].channel, "schedule");
    assert_eq!(reply(&page, 1)["prompt"], "how are we doing");

    // It speaks with the trust of who made it, and is said to be a schedule.
    let given = prompts(&page);
    assert_eq!(given[0][0], "how are we doing");
    let block = given[0].last().expect("the block");
    assert!(block.contains("via: \"schedule\""));
    assert!(
        block.contains("\"name\":\"Every hour\",\"kind\":\"schedule\",\"trust\":\"principal\""),
        "{block}"
    );

    let (_, runs) = state.schedules_shown(None).await.expect("schedules");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].state, swem_host::RunState::Answered);
    assert_eq!(runs[0].chat_id.as_deref(), Some(chat_id.as_str()));
    assert!(!runs[0].late);

    // Off, it is not due whatever the time.
    state
        .change_schedule(
            &schedule.schedule.schedule_id,
            swem_host::TimedMessageChange {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .expect("turn it off");
    assert!(state.claim_what_is_due(u64::MAX / 4).await.is_empty());
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: said, asked, left alone, refused in time, ended"
)]
async fn what_an_agent_asks_with_nobody_there_is_refused_for_it_in_time() {
    let root = fixture_root("unattended");
    let (state, _ledger) = shell_of_two(&root);
    state
        .keep_time(&root.join("schedules"))
        .await
        .expect("keep time");
    let agent = state.chat_people().await.expect("people")["participants"]
        .as_array()
        .expect("participants")
        .iter()
        .find(|one| one["profile_id"] == "coder")
        .expect("the agent")["participant_id"]
        .as_str()
        .expect("an id")
        .to_owned();
    // A new chat each time: the schedule names none.
    let wants =
        json!({ "fixture": "mcp-echo-permission-v0.1", "server": "echo", "nonce": "unattended" });
    let made = state
        .make_schedule(swem_host::workbench_shell::NewScheduleBody {
            agent_id: agent,
            chat_id: None,
            say: wants.to_string(),
            when: swem_host::When::Every { minutes: 30 },
        })
        .await
        .expect("make a schedule");
    assert_eq!(made.chat_title, None);
    let at = next_due(&state, &made.schedule.schedule_id).await;
    let due = state.claim_what_is_due(at).await;
    assert_eq!(due.len(), 1);
    let saying = tokio::spawn({
        let state = Arc::clone(&state);
        let due = due[0].clone();
        async move { state.say_what_is_due(due).await }
    });

    // The agent asks before it does something, and nobody is there.
    let began = std::time::Instant::now();
    let chat_id = loop {
        let (_, runs) = state.schedules_shown(None).await.expect("schedules");
        if let Some(chat_id) = runs.first().and_then(|run| run.chat_id.clone()) {
            break chat_id;
        }
        assert!(
            began.elapsed() < Duration::from_secs(30),
            "it was never said"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let page = until(&state, &chat_id, "the question", |page| {
        !page.questions.is_empty()
    })
    .await;
    let question = page.questions[0].clone();

    // While it is due again the run before it has not ended: passed over.
    assert!(state.claim_what_is_due(at + 30 * 60_000).await.is_empty());

    // A person has half an hour. Before it is over nothing is answered.
    // It was asked now, whatever moment the run was taken up at.
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_millis(),
    )
    .expect("a time that fits");
    state.refuse_what_nobody_answered(now + 29 * 60_000).await;
    let still = state
        .chat_page(&chat_id, None, 500)
        .await
        .expect("read the chat");
    assert_eq!(still.questions.len(), 1, "it was refused before its time");

    // After it, the schedule refuses for them, in the engine's own words,
    // and the turn ends.
    state.refuse_what_nobody_answered(u64::MAX / 4).await;
    saying.await.expect("the run ended");
    let page = until(&state, &chat_id, "the turn to finish", settled(2)).await;
    assert!(page.questions.is_empty());
    assert!(
        !root.join("mcp-receipt.json").is_file(),
        "what nobody allowed was done"
    );
    let answered = page
        .events
        .iter()
        .rev()
        .find(|event| event.kind == "chat/question")
        .expect("the question moved");
    assert_eq!(answered.payload["question_id"], question.question_id);
    assert_eq!(answered.payload["state"], "answered");
    assert_eq!(answered.payload["answered_by"], made.schedule.speaker_id);
    let refused = question.asked["options"]
        .as_array()
        .expect("options")
        .iter()
        .find(|option| {
            option["kind"]
                .as_str()
                .is_some_and(|kind| kind.starts_with("reject"))
        })
        .expect("the engine offered a way to refuse")["optionId"]
        .clone();
    assert_eq!(answered.payload["answer"]["option"], refused);

    let (_, runs) = state.schedules_shown(None).await.expect("schedules");
    assert_eq!(
        runs.iter().map(|run| run.state).collect::<Vec<_>>(),
        [swem_host::RunState::Skipped, swem_host::RunState::Answered]
    );
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

/// Ask the agent to call one of its schedule tools, and read what the tool
/// answered.
async fn through_its_tools(
    state: &Arc<WorkbenchShellState>,
    chat_id: &str,
    said_before: usize,
    tool: &str,
    arguments: Value,
) -> Value {
    let asks = json!({
        "fixture": "mcp-call-v0.1", "server": "swem-time", "nonce": tool,
        "tool": tool, "arguments": arguments,
    });
    state
        .say_in_chat(chat_id, saying(&asks.to_string()))
        .await
        .expect("say");
    let page = until(
        state,
        chat_id,
        "the tool's answer",
        settled(said_before + 2),
    )
    .await;
    reply(&page, said_before + 1)["mcp_result"].clone()
}

#[tokio::test]
#[allow(
    clippy::too_many_lines,
    reason = "one walk: made, refused, told of, not its own, paused"
)]
async fn an_agent_makes_its_own_schedule_in_the_chat_it_was_asked_in() {
    let root = fixture_root("its-own");
    let (state, _ledger) = shell_of_two(&root);
    state.set_time_tools_command(
        PathBuf::from(env!("CARGO_BIN_EXE_swem-time-tools")),
        Vec::new(),
    );
    let chat = state
        .start_chat(StartChatBody {
            title: "Release 0.9".into(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("chat");
    let agent = chat
        .members
        .iter()
        .find(|member| member.kind == swem_host::ParticipantKind::Agent)
        .expect("the agent")
        .participant_id
        .clone();

    // Asked to check back, it makes a schedule: its own, in this chat.
    let made = through_its_tools(
        &state,
        &chat.chat_id,
        0,
        "make_schedule",
        json!({ "say": "Look for a new release branch and say so", "every_minutes": 30 }),
    )
    .await;
    assert_eq!(made["when"], "Every 30 minutes", "{made}");
    assert_eq!(made["made_by_you"], true);
    let (schedules, _) = state
        .schedules_shown(Some(agent.clone()))
        .await
        .expect("schedules");
    assert_eq!(schedules.len(), 1);
    assert_eq!(schedules[0].schedule.made_by, agent);
    assert_eq!(
        schedules[0].schedule.chat_id.as_deref(),
        Some(chat.chat_id.as_str())
    );
    assert_eq!(schedules[0].chat_title.as_deref(), Some("Release 0.9"));
    let its_own = schedules[0].schedule.schedule_id.clone();

    // Too often for an agent, and it is told so.
    let refused = through_its_tools(
        &state,
        &chat.chat_id,
        2,
        "make_schedule",
        json!({ "say": "Again", "every_minutes": 1 }),
    )
    .await;
    assert!(
        refused.to_string().contains("at least 5 minutes"),
        "{refused}"
    );

    // What a person made is theirs: the agent is told of it and cannot
    // remove it. What it made itself it pauses.
    let theirs = state
        .make_schedule(swem_host::workbench_shell::NewScheduleBody {
            agent_id: agent.clone(),
            chat_id: Some(chat.chat_id.clone()),
            say: "Check the changelog".into(),
            when: swem_host::When::Every { minutes: 60 },
        })
        .await
        .expect("a person's schedule");
    let listed = through_its_tools(&state, &chat.chat_id, 4, "list_schedules", json!({})).await;
    let listed = listed["schedules"].as_array().expect("schedules").clone();
    assert_eq!(
        listed
            .iter()
            .map(|one| (
                one["say"].as_str().unwrap_or(""),
                one["made_by_you"] == true
            ))
            .collect::<Vec<_>>(),
        [
            ("Look for a new release branch and say so", true),
            ("Check the changelog", false)
        ]
    );
    let refused = through_its_tools(
        &state,
        &chat.chat_id,
        6,
        "remove_schedule",
        json!({ "schedule_id": theirs.schedule.schedule_id }),
    )
    .await;
    assert!(
        refused.to_string().contains("made by a person"),
        "{refused}"
    );
    let paused = through_its_tools(
        &state,
        &chat.chat_id,
        8,
        "pause_schedule",
        json!({ "schedule_id": its_own, "paused": true }),
    )
    .await;
    assert_eq!(paused["paused"], true, "{paused}");
    let (schedules, _) = state.schedules_shown(Some(agent)).await.expect("schedules");
    assert_eq!(
        schedules
            .iter()
            .map(|shown| shown.schedule.enabled)
            .collect::<Vec<_>>(),
        [false, true]
    );
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}

#[tokio::test]
async fn what_an_app_said_the_person_is_looking_at_goes_with_their_words() {
    let root = fixture_root("context");
    let (state, _ledger) = shell_of_two(&root);
    let chat = state
        .start_chat(StartChatBody {
            title: "Context".into(),
            agents: vec!["coder".into()],
        })
        .await
        .expect("chat");
    let looking_at = |server: &str| {
        serde_json::from_value::<Saying>(json!({
            "text": "what is this?",
            "context": { "server_name": server, "content": [
                { "type": "text", "text": "Project notes, revision 4d6eb1e" },
                { "type": "resource_link", "uri": "notes://revisions/4d6eb1e", "name": "revision" } ] }
        }))
        .expect("what a page sends")
    };
    state
        .say_in_chat(&chat.chat_id, looking_at("echo"))
        .await
        .expect("say");
    let page = until(&state, &chat.chat_id, "the answer", settled(2)).await;
    // The message is the person's words; what they were looking at is
    // kept with it and is not their words.
    assert_eq!(page.messages[0].text, "what is this?");
    assert_eq!(page.messages[0].content[1]["type"], "context");
    let given = &prompts(&page)[0];
    assert_eq!(
        given[..2],
        [
            "what is this?".to_owned(),
            "Project notes, revision 4d6eb1e".to_owned()
        ]
    );
    assert!(
        given
            .last()
            .expect("the block")
            .starts_with("<swem:turn k=\"")
    );
    let links: Vec<&str> = page
        .events
        .iter()
        .filter(|event| event.kind == "host/prompt_submitted")
        .flat_map(|event| event.payload["content"].as_array().expect("blocks").iter())
        .filter_map(|block| block["uri"].as_str())
        .collect();
    assert_eq!(links, vec!["notes://revisions/4d6eb1e"]);

    // Said by an App of a server this agent does not attach, the agent
    // could not follow it: it is kept with the message and not given.
    state
        .say_in_chat(&chat.chat_id, looking_at("somebody-elses"))
        .await
        .expect("say");
    let page = until(&state, &chat.chat_id, "the second answer", settled(4)).await;
    let given = &prompts(&page)[1];
    assert_eq!(given.len(), 2, "{given:?}");
    assert_eq!(given[0], "what is this?");
    state.let_go_of(None).await;
    fs::remove_dir_all(root).expect("remove fixture root");
}
