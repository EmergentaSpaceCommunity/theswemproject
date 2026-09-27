//! A chat has participants and holds sessions (ADR-0007).

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::json;
use swem_host::{
    AttachmentBinding, AttachmentTransport, CHANNEL_AGENT, CHANNEL_WORKBENCH, DeliveryState,
    ParticipantKind, QuestionState, RoutingLedger, SessionRouteBinding, SurfaceEventSource,
};

fn fixture_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-chat-ledger-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

fn binding(workspace: &Path, route: &str, profile: &str, session: &str) -> SessionRouteBinding {
    SessionRouteBinding::new(
        route,
        "agent",
        profile,
        session,
        "environment",
        workspace,
        vec![AttachmentBinding::new(
            "notes-profile",
            "notes",
            AttachmentTransport::Stdio,
        )],
    )
    .expect("fixture binding")
}

/// A ledger as it was before chats: one route, two turns, the line about who
/// wrote appended to what they wrote, and a replay of the first turn.
fn ledger_of_routes(path: &Path, workspace: &Path) {
    let connection = Connection::open(path).expect("open v1");
    connection
        .execute_batch(
            "CREATE TABLE ledger_meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             INSERT INTO ledger_meta(key, value) VALUES ('schema_version', 1);
             CREATE TABLE routes (
               route_id TEXT PRIMARY KEY, agent_id TEXT NOT NULL, agent_profile_id TEXT NOT NULL,
               native_session_id TEXT NOT NULL, environment_profile_id TEXT NOT NULL,
               workspace TEXT NOT NULL, attachments_json TEXT NOT NULL);
             CREATE TABLE events (
               sequence INTEGER PRIMARY KEY, route_id TEXT NOT NULL REFERENCES routes(route_id),
               event_id TEXT NOT NULL, kind TEXT NOT NULL, source TEXT NOT NULL,
               payload_json TEXT NOT NULL, UNIQUE(route_id, event_id));
             CREATE INDEX events_route_sequence ON events(route_id, sequence);
             CREATE TABLE surface_cursors (
               route_id TEXT NOT NULL REFERENCES routes(route_id), surface_id TEXT NOT NULL,
               cursor INTEGER NOT NULL, PRIMARY KEY(route_id, surface_id));",
        )
        .expect("v1 schema");
    connection
        .execute(
            "INSERT INTO routes VALUES ('route-1', 'agent', 'ada', 'native-1', 'environment', ?1, '[]')",
            [workspace.to_str().expect("workspace")],
        )
        .expect("route");
    let chunk = |text: &str| {
        json!({ "sessionId": "native-1", "update": {
            "sessionUpdate": "agent_message_chunk", "content": { "type": "text", "text": text } } })
    };
    let events = [
        (
            "host/turn_written",
            "\"host\"",
            json!({ "correspondent": { "surface": "workbench-browser" },
            "text": "Hello there\n[Someone, writing from workbench-browser]" }),
        ),
        (
            "host/prompt_submitted",
            "\"host\"",
            json!({ "session_id": "native-1", "turn": 0, "content": [
            { "type": "text", "text": "Hello there" },
            { "type": "text", "text": "[Someone, writing from workbench-browser]" } ] }),
        ),
        ("acp/session_update", "\"native_live\"", chunk("Hello!")),
        (
            "acp/session_update",
            "\"native_live\"",
            chunk(" What shall we do?"),
        ),
        (
            "acp/prompt_response",
            "\"native_live\"",
            json!({ "turn": 0, "stop_reason": "end_turn" }),
        ),
        (
            "acp/session_update",
            "\"native_replay\"",
            chunk("Hello! What shall we do?"),
        ),
        (
            "host/turn_written",
            "\"host\"",
            json!({ "correspondent": { "surface": "workbench-browser" },
            "text": "[a list]\n[Someone, writing from workbench-browser]" }),
        ),
        (
            "host/prompt_submitted",
            "\"host\"",
            json!({ "session_id": "native-1", "turn": 1, "content": [
            { "type": "text", "text": "[a list]" },
            { "type": "text", "text": "[Someone, writing from workbench-browser]" } ] }),
        ),
        ("acp/session_update", "\"native_live\"", chunk("Done.")),
        (
            "acp/prompt_response",
            "\"native_live\"",
            json!({ "turn": 1, "stop_reason": "end_turn" }),
        ),
    ];
    for (index, (kind, source, payload)) in events.iter().enumerate() {
        connection
            .execute(
                "INSERT INTO events(route_id, event_id, kind, source, payload_json)
                 VALUES ('route-1', ?1, ?2, ?3, ?4)",
                rusqlite::params![format!("e{index}"), kind, source, payload.to_string()],
            )
            .expect("event");
    }
    connection
        .execute(
            "INSERT INTO surface_cursors VALUES ('route-1', 'workbench-browser', 10)",
            [],
        )
        .expect("cursor");
}

#[test]
fn a_ledger_of_routes_becomes_a_ledger_of_chats() {
    let root = fixture_root("migration");
    let path = root.join("routes.jsonl");
    ledger_of_routes(&path, &root);

    let mut ledger = RoutingLedger::open(&path).expect("open and migrate");
    assert!(
        root.join("routes.jsonl.v1").is_file(),
        "the ledger is copied beside itself before it is changed"
    );

    let owner = ledger.owner().expect("the owner");
    assert_eq!(owner.kind, ParticipantKind::Person);
    let chats = ledger.chats_of(&owner.participant_id).expect("chats");
    assert_eq!(chats.len(), 1);
    let chat = &chats[0];
    assert_eq!(
        chat.title, "Hello there",
        "a chat nobody named is known by its first words"
    );
    assert_eq!(chat.members.len(), 2);
    let agent = chat
        .members
        .iter()
        .find(|member| member.kind == ParticipantKind::Agent)
        .expect("the agent is a member");
    assert_eq!(agent.profile_id.as_deref(), Some("ada"));
    assert_eq!(agent.handle, "ada");

    let said = ledger.said_after(&chat.chat_id, 0, 100).expect("messages");
    let lines: Vec<(&str, &str, &str)> = said
        .iter()
        .map(|message| {
            (
                if message.sender_id == owner.participant_id {
                    "owner"
                } else {
                    "agent"
                },
                message.channel.as_str(),
                message.text.as_str(),
            )
        })
        .collect();
    assert_eq!(
        lines,
        vec![
            ("owner", CHANNEL_WORKBENCH, "Hello there"),
            ("agent", CHANNEL_AGENT, "Hello! What shall we do?"),
            // What a person wrote in brackets is theirs; only the host's own
            // line about who wrote is taken off.
            ("owner", CHANNEL_WORKBENCH, "[a list]"),
            ("agent", CHANNEL_AGENT, "Done."),
        ]
    );

    let (events, messages, more) = ledger.timeline(&chat.chat_id, None, 100).expect("timeline");
    assert_eq!(events.len(), 10);
    assert!(
        events
            .iter()
            .all(|event| event.chat_id.as_deref() == Some(chat.chat_id.as_str()))
    );
    assert!(
        events
            .iter()
            .all(|event| event.agent_id.as_deref() == Some(agent.participant_id.as_str()))
    );
    assert_eq!(messages.len(), 4);
    assert!(!more);

    // The route is still what it was, and its surface is where it was.
    assert_eq!(
        ledger.route("route-1").expect("route").native_session_id,
        "native-1"
    );
    assert_eq!(
        ledger
            .surface_cursor("route-1", "workbench-browser")
            .expect("cursor"),
        10
    );
    // Opening it again changes nothing.
    drop(ledger);
    let ledger = RoutingLedger::open(&path).expect("open again");
    assert_eq!(
        ledger.chats_of(&owner.participant_id).expect("chats").len(),
        1
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn a_message_has_a_sender_and_a_place_in_the_one_order() {
    let root = fixture_root("messages");
    let mut ledger = RoutingLedger::open(&root.join("routes.jsonl")).expect("open");
    let owner = ledger.owner().expect("owner");
    let ada = ledger.agent_of_profile("ada").expect("ada");
    let builder = ledger.agent_of_profile("Builder Two").expect("builder");
    assert_eq!(builder.handle, "builder-two");
    assert_ne!(ada.colour, builder.colour);

    let chat = ledger
        .start_chat(
            "Release",
            &owner.participant_id,
            &[ada.participant_id.clone(), builder.participant_id.clone()],
        )
        .expect("chat");
    assert_eq!(chat.members.len(), 3);

    let first = ledger
        .say(
            &chat.chat_id,
            &owner.participant_id,
            CHANNEL_WORKBENCH,
            Some("client-1"),
            &json!([{ "type": "text", "text": "@ada draft it, `@builder` wait" }]),
        )
        .expect("say");
    assert_eq!(first.named, vec!["ada".to_owned()]);
    let again = ledger
        .say(
            &chat.chat_id,
            &owner.participant_id,
            CHANNEL_WORKBENCH,
            Some("client-1"),
            &json!([{ "type": "text", "text": "@ada draft it, `@builder` wait" }]),
        )
        .expect("say again");
    assert_eq!(
        again.message_id, first.message_id,
        "the same message is not said twice"
    );

    let stranger = ledger.agent_of_profile("scout").expect("scout");
    assert!(
        ledger
            .say(
                &chat.chat_id,
                &stranger.participant_id,
                CHANNEL_AGENT,
                None,
                &json!([{ "type": "text", "text": "hello" }]),
            )
            .is_err(),
        "one who is not in a chat says nothing in it"
    );

    fs::remove_dir_all(root).expect("remove fixture");
}

struct ChatWithAda {
    root: PathBuf,
    ledger: RoutingLedger,
    ada: swem_host::Participant,
    chat: swem_host::Chat,
    first: swem_host::Message,
}

fn a_chat_with_ada(name: &str) -> ChatWithAda {
    let root = fixture_root(name);
    let mut ledger = RoutingLedger::open(&root.join("routes.jsonl")).expect("open");
    let owner = ledger.owner().expect("owner");
    let ada = ledger.agent_of_profile("ada").expect("ada");
    let chat = ledger
        .start_chat(
            "Release",
            &owner.participant_id,
            std::slice::from_ref(&ada.participant_id),
        )
        .expect("chat");
    let first = ledger
        .say(
            &chat.chat_id,
            &owner.participant_id,
            CHANNEL_WORKBENCH,
            None,
            &json!([{ "type": "text", "text": "@ada draft it" }]),
        )
        .expect("say");
    ChatWithAda {
        root,
        ledger,
        ada,
        chat,
        first,
    }
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one turn from the message to the answer, in the order it happens"
)]
fn what_an_engine_says_in_a_session_is_the_chats() {
    let ChatWithAda {
        root,
        mut ledger,
        ada,
        chat,
        first,
    } = a_chat_with_ada("sessions");
    // A session of the agent in this chat: what its engine says is the chat's.
    let route = binding(&root, "route-a", "ada", "native-a");
    let session = ledger
        .bind_route_in_chat(&route, &chat.chat_id, "")
        .expect("session");
    assert_eq!(session.agent_id, ada.participant_id);
    assert_eq!(
        ledger
            .bind_route_in_chat(&route, &chat.chat_id, "")
            .expect("met again")
            .session_id,
        session.session_id
    );
    // The message is given to the agent: the prompt carries the host's block,
    // so it is not kept a second time. What the agent answers is kept when
    // its turn ends, whole, as the agent's.
    let mut append =
        |id: &str, kind: &str, source: SurfaceEventSource, payload: serde_json::Value| {
            ledger
                .append_event("route-a", id, kind, source, &payload)
                .expect("event")
        };
    append(
        "e1",
        "host/prompt_submitted",
        SurfaceEventSource::Host,
        json!({ "turn": 0, "content": [
            { "type": "text", "text": "@ada draft it" },
            { "type": "text", "text": "<swem:turn k=\"9f2c4e1ab07d\">\n</swem:turn k=\"9f2c4e1ab07d\">" } ] }),
    );
    let chunk = |text: &str| {
        json!({ "update": { "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": text } } })
    };
    append(
        "e2",
        "acp/session_update",
        SurfaceEventSource::NativeLive,
        chunk("Draf"),
    );
    append(
        "e3",
        "acp/session_update",
        SurfaceEventSource::NativeLive,
        chunk("ted."),
    );
    let place = append(
        "e4",
        "acp/prompt_response",
        SurfaceEventSource::NativeLive,
        json!({ "turn": 0, "stop_reason": "end_turn" }),
    );
    assert!(place > first.sequence);
    let said = ledger.said_after(&chat.chat_id, 0, 10).expect("messages");
    assert_eq!(
        said.iter()
            .map(|message| (
                message.sender_id.as_str(),
                message.text.as_str(),
                message.sequence
            ))
            .collect::<Vec<_>>(),
        vec![
            (chat.created_by.as_str(), "@ada draft it", first.sequence),
            (ada.participant_id.as_str(), "Drafted.", place),
        ]
    );
    assert!(said[1].created_ms.is_some());

    let happened = ledger.happened_after(0, 100).expect("everything");
    assert_eq!(
        happened
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        vec![
            "chat/started",
            "chat/message",
            "host/prompt_submitted",
            "acp/session_update",
            "acp/session_update",
            "acp/prompt_response"
        ]
    );
    assert!(happened[..2].iter().all(|event| event.agent_id.is_none()));
    assert!(
        happened[2..]
            .iter()
            .all(|event| event.agent_id.as_deref() == Some(ada.participant_id.as_str()))
    );
    assert!(happened.iter().all(|event| event.at_ms.is_some()));
    assert_eq!(ledger.head().expect("head"), place);

    assert_eq!(
        ledger
            .given_through(&chat.chat_id, &ada.participant_id, Some(place))
            .expect("given"),
        place
    );
    assert_eq!(
        ledger
            .given_through(&chat.chat_id, &ada.participant_id, Some(1))
            .expect("given"),
        place
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn a_chat_outlives_the_session_an_agent_had_in_it() {
    let ChatWithAda {
        root,
        mut ledger,
        ada,
        chat,
        ..
    } = a_chat_with_ada("outlives");
    let route = binding(&root, "route-a", "ada", "native-a");
    let session = ledger
        .bind_route_in_chat(&route, &chat.chat_id, "")
        .expect("session");
    // The engine's session is replaced; the chat goes on.
    let fresh = binding(&root, "route-b", "ada", "native-b");
    let replaced = ledger
        .bind_route_in_chat(&fresh, &chat.chat_id, "its engine no longer had it")
        .expect("a fresh session in the same chat");
    assert_ne!(replaced.session_id, session.session_id);
    assert_eq!(
        ledger
            .current_session(&chat.chat_id, &ada.participant_id)
            .expect("current")
            .expect("a session")
            .route_id,
        "route-b"
    );
    assert!(
        !ledger
            .session_of_route("route-a")
            .expect("the earlier one")
            .expect("is kept")
            .current
    );
    // A route bound with no chat named gets a chat of its own.
    let alone = binding(&root, "route-c", "builder-profile", "native-c");
    ledger.bind_route(&alone).expect("bind alone");
    let own = ledger
        .session_of_route("route-c")
        .expect("session")
        .expect("it has one");
    assert_ne!(own.chat_id, chat.chat_id);
    assert!(
        ledger
            .bind_route_in_chat(&alone, &chat.chat_id, "")
            .is_err(),
        "a route is a session of one chat"
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "one walk through every state of a delivery and a question, in order"
)]
fn what_a_chat_owes_and_waits_for_outlives_the_page_and_the_workbench() {
    let ChatWithAda {
        root,
        mut ledger,
        ada,
        chat,
        first,
    } = a_chat_with_ada("work");
    let agents = std::slice::from_ref(&ada.participant_id);

    let owed = ledger.deliver(&first.message_id, agents).expect("deliver");
    assert_eq!(owed.len(), 1);
    assert_eq!(owed[0].state, DeliveryState::Queued);
    assert_eq!(
        ledger.deliver(&first.message_id, agents).expect("again")[0].delivery_id,
        owed[0].delivery_id,
        "owed twice, it is the same debt"
    );
    assert!(
        ledger
            .deliver(&first.message_id, std::slice::from_ref(&chat.created_by))
            .is_err(),
        "a message is owed to agents of its chat"
    );
    assert_eq!(ledger.agents_owed(None).expect("owed"), agents.to_vec());

    // A second message waits its turn behind the first.
    let second = ledger
        .say(
            &chat.chat_id,
            &chat.created_by,
            CHANNEL_WORKBENCH,
            None,
            &json!([{ "type": "text", "text": "and then publish" }]),
        )
        .expect("say");
    ledger.deliver(&second.message_id, agents).expect("deliver");
    let running = ledger
        .take_next_delivery(&ada.participant_id, None)
        .expect("take")
        .expect("the first");
    assert_eq!(running.message_id, first.message_id);
    assert_eq!(running.state, DeliveryState::Running);
    assert!(running.started_ms.is_some());

    // The agent asks; the question waits in the ledger.
    let asked = json!({ "title": "Run cargo publish", "options": [
        { "id": "run-once", "name": "Run it" }, { "id": "do-not-run", "name": "Not this time" } ] });
    let question = ledger
        .ask(
            &chat.chat_id,
            &ada.participant_id,
            Some(&running.delivery_id),
            "permission",
            &asked,
        )
        .expect("ask");
    drop(ledger);
    let mut ledger = RoutingLedger::open(&root.join("routes.jsonl")).expect("the page came back");
    let waiting = ledger
        .questions_waiting(Some(&chat.chat_id))
        .expect("waiting");
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].asked, asked);
    let answered = ledger
        .answer_question(
            &question.question_id,
            &json!({ "option": "run-once" }),
            &chat.created_by,
        )
        .expect("answer");
    assert_eq!(answered.state, QuestionState::Answered);
    assert!(
        ledger
            .answer_question(
                &question.question_id,
                &json!({ "option": "do-not-run" }),
                &chat.created_by
            )
            .is_err(),
        "a question is answered once"
    );

    // A second question is left waiting when the Workbench stops.
    let left = ledger
        .ask(
            &chat.chat_id,
            &ada.participant_id,
            Some(&running.delivery_id),
            "permission",
            &asked,
        )
        .expect("ask again");
    assert_eq!(ledger.agents_running().expect("running"), agents.to_vec());
    assert_eq!(ledger.settle_what_was_running(&[]).expect("settle"), 0);
    assert_eq!(ledger.settle_what_was_running(agents).expect("settle"), 1);
    assert_eq!(
        ledger
            .delivery(&running.delivery_id)
            .expect("delivery")
            .state,
        DeliveryState::Interrupted
    );
    assert_eq!(
        ledger.question(&left.question_id).expect("question").state,
        QuestionState::Lapsed
    );
    assert!(ledger.questions_waiting(None).expect("waiting").is_empty());
    // What was queued is still owed, and ends as it ends.
    let next = ledger
        .take_next_delivery(&ada.participant_id, None)
        .expect("take")
        .expect("the second");
    assert_eq!(next.message_id, second.message_id);
    let done = ledger
        .end_delivery(&next.delivery_id, DeliveryState::Done, None)
        .expect("end");
    assert!(done.ended_ms.is_some());
    assert_eq!(
        ledger
            .end_delivery(&next.delivery_id, DeliveryState::Failed, Some("late"))
            .expect("ended already")
            .state,
        DeliveryState::Done
    );
    assert!(
        ledger
            .take_next_delivery(&ada.participant_id, None)
            .expect("take")
            .is_none()
    );
    assert!(ledger.deliveries_open(None).expect("open").is_empty());

    // Every move is an event of the chat, in the one order.
    let kinds: Vec<String> = ledger
        .happened_after(first.sequence, 100)
        .expect("events")
        .into_iter()
        .map(|event| {
            format!(
                "{} {}",
                event.kind,
                event.payload["state"].as_str().unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "chat/delivery queued",
            "chat/message ",
            "chat/delivery queued",
            "chat/delivery running",
            "chat/question waiting",
            "chat/question answered",
            "chat/question waiting",
            "chat/question lapsed",
            "chat/delivery interrupted",
            "chat/delivery running",
            "chat/delivery done",
        ]
    );
    fs::remove_dir_all(root).expect("remove fixture");
}
