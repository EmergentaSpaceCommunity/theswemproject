//! What a page asks about chats, and the one stream it follows them by.
//!
//! A page holds one connection that stays open: `GET /api/stream`. On it
//! comes everything that happens in every chat, in the one order the ledger
//! keeps, each event under its place in that order. A page that lost the
//! connection comes back with the place it had reached (`Last-Event-ID`, as
//! a browser sends it by itself, or `?cursor=`) and is given what it missed.
//! A page that comes with no place is given the state whole first.
//!
//! Everything else a page does with a chat is a short request beside it.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream;
use http_body_util::{BodyExt as _, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::{Method, Request, Response, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::chats::ledger_refusal;
use super::{
    AskedBody, Saying, ShellBody, StartChatBody, WorkbenchShellError, WorkbenchShellState,
    error_response, json_result, query_param, read_json, respond_json,
};
use crate::{Principal, RoutingLedger, Scope};

/// How often the ledger is asked whether something happened. Another
/// process may write to it, so nothing in this one can say when.
const LOOK: Duration = Duration::from_millis(120);
/// How often a quiet stream says it is still there.
const STILL_HERE: Duration = Duration::from_secs(15);
/// The most events sent for one look.
const AT_ONCE: usize = 500;

#[derive(Deserialize)]
struct StopBody {
    #[serde(default)]
    agent_id: Option<String>,
}

/// What is changed about a chat: what it is called, its rules. What is
/// left out stays.
#[derive(Deserialize)]
struct ChangeChatBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    answer_rule: Option<String>,
    #[serde(default)]
    reply_limit: Option<u32>,
}

#[derive(Deserialize)]
struct BringBody {
    /// The profile of the agent that is brought in.
    #[serde(default)]
    agent: Option<String>,
    /// Or the guest that is let in.
    #[serde(default)]
    guest: Option<String>,
}

/// What somebody is called from now on; what is left out stays.
#[derive(Deserialize)]
struct NameBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    handle: Option<String>,
    #[serde(default)]
    colour: Option<String>,
}

fn frame(event: &str, place: Option<u64>, data: &Value) -> Bytes {
    use std::fmt::Write as _;
    let mut text = String::new();
    if let Some(place) = place {
        let _ = writeln!(text, "id: {place}");
    }
    let _ = write!(text, "event: {event}\ndata: {data}\n\n");
    Bytes::from(text)
}

async fn body_of<T: serde::de::DeserializeOwned>(
    request: Request<AskedBody>,
) -> Result<T, WorkbenchShellError> {
    serde_json::from_value(read_json(request).await?)
        .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
}

impl WorkbenchShellState {
    /// Everything a page draws before anything happens: who there is, the
    /// chats, what they owe and wait for, and the place this was true at.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn chats_now(&self) -> Result<Value, WorkbenchShellError> {
        self.chats_now_within(&Scope::Everything).await
    }

    /// [`Self::chats_now`] as far as a scope reaches: somebody who came
    /// through a messenger is shown the chats within it, the people in
    /// them, and what those chats owe and wait for - nothing else.
    pub(super) async fn chats_now_within(
        &self,
        scope: &Scope,
    ) -> Result<Value, WorkbenchShellError> {
        let people = self.chat_people().await?;
        let scope = scope.clone();
        self.with_ledger(move |ledger| {
            // The place first: what happens while the rest is read is sent
            // again after it, and being told twice changes nothing.
            let head = ledger.head()?;
            let owner = ledger.owner()?;
            let (whose, you) = match &scope {
                Scope::Everything => (owner.participant_id.clone(), None),
                Scope::Within { participant_id, .. } => {
                    (participant_id.clone(), Some(participant_id.clone()))
                }
            };
            let chats: Vec<crate::Chat> = ledger
                .chats_of(&whose)?
                .into_iter()
                .filter(|chat| scope.admits(chat))
                .collect();
            let admitted: BTreeSet<&str> = chats.iter().map(|chat| chat.chat_id.as_str()).collect();
            let deliveries: Vec<crate::Delivery> = ledger
                .deliveries_open(None)?
                .into_iter()
                .filter(|delivery| admitted.contains(delivery.chat_id.as_str()))
                .collect();
            let questions: Vec<crate::Question> = ledger
                .questions_waiting(None)?
                .into_iter()
                .filter(|question| admitted.contains(question.chat_id.as_str()))
                .collect();
            let (owner, participants) = match &scope {
                Scope::Everything => (people["owner"].clone(), people["participants"].clone()),
                Scope::Within { .. } => {
                    // The people in those chats, and nobody else.
                    let seen: BTreeSet<&str> = chats
                        .iter()
                        .flat_map(|chat| chat.members.iter())
                        .map(|member| member.participant_id.as_str())
                        .collect();
                    let participants: Vec<Value> = people["participants"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|one| {
                            one["participant_id"]
                                .as_str()
                                .is_some_and(|id| seen.contains(id))
                        })
                        .cloned()
                        .collect();
                    let owner = if seen.contains(owner.participant_id.as_str()) {
                        people["owner"].clone()
                    } else {
                        Value::Null
                    };
                    (owner, Value::Array(participants))
                }
            };
            Ok(json!({
                "head": head,
                "owner": owner,
                "you": you,
                "participants": participants,
                "chats": chats,
                "deliveries": deliveries,
                "questions": questions,
            }))
        })
        .await
        .map_err(ledger_refusal)
    }

    /// The chats a scope reaches, by id, as the ledger has them now.
    fn admitted_now(ledger: &RoutingLedger, scope: &Scope) -> Option<BTreeSet<String>> {
        let Scope::Within { participant_id, .. } = scope else {
            return None;
        };
        Some(
            ledger
                .chats_of(participant_id)
                .unwrap_or_default()
                .into_iter()
                .filter(|chat| scope.admits(chat))
                .map(|chat| chat.chat_id)
                .collect(),
        )
    }

    /// Call a chat something else.
    ///
    /// # Errors
    ///
    /// Refuses a chat that does not exist and a name that is not one line.
    pub async fn rename_chat(
        &self,
        chat_id: &str,
        title: String,
    ) -> Result<crate::Chat, WorkbenchShellError> {
        let chat_id = chat_id.to_owned();
        self.with_ledger(move |ledger| ledger.rename_chat(&chat_id, &title))
            .await
            .map_err(ledger_refusal)
    }

    /// Change what a participant is called: its name, the handle it is
    /// named by in a chat, its colour. An agent is told its new name with
    /// the next session it starts.
    ///
    /// # Errors
    ///
    /// Refuses somebody else's handle, a handle a chat cannot carry and a
    /// name that is not one line.
    pub async fn name_participant(
        &self,
        participant_id: &str,
        name: Option<String>,
        handle: Option<String>,
        colour: Option<String>,
    ) -> Result<crate::Participant, WorkbenchShellError> {
        let participant_id = participant_id.to_owned();
        self.with_ledger(move |ledger| {
            ledger.participant(&participant_id)?;
            ledger.name_participant(
                &participant_id,
                name.as_deref(),
                handle.as_deref(),
                colour.as_deref(),
            )
        })
        .await
        .map_err(ledger_refusal)
    }

    async fn change_chat(
        &self,
        chat_id: &str,
        change: ChangeChatBody,
    ) -> Result<crate::Chat, WorkbenchShellError> {
        let mut chat = self.chat_named(chat_id).await?;
        if let Some(title) = change.title {
            chat = self.rename_chat(chat_id, title).await?;
        }
        if change.answer_rule.is_some() || change.reply_limit.is_some() {
            chat = self
                .rule_chat(chat_id, change.answer_rule, change.reply_limit)
                .await?;
        }
        Ok(chat)
    }

    /// The stream of everything that happens, from a place or from now -
    /// as far as the scope reaches: somebody who came through a messenger
    /// hears their chats and nothing else.
    async fn stream_from(&self, place: Option<u64>, scope: Scope) -> Response<ShellBody> {
        let (frames, sent) = tokio::sync::mpsc::channel::<Bytes>(64);
        let opening = match place {
            // A place past the end of the record is a place in another
            // record: the page is told to forget what it has.
            Some(place) => match self.with_ledger(|ledger| ledger.head()).await {
                Ok(head) if place <= head => Ok((place, None)),
                Ok(_) => self
                    .chats_now_within(&scope)
                    .await
                    .map(|now| (now["head"].as_u64().unwrap_or(0), Some(("reset", now)))),
                Err(error) => Err(ledger_refusal(error)),
            },
            None => self
                .chats_now_within(&scope)
                .await
                .map(|now| (now["head"].as_u64().unwrap_or(0), Some(("state", now)))),
        };
        let (place, first) = match opening {
            Ok(opening) => opening,
            Err(error) => return error_response(&error),
        };
        tokio::spawn(follow(
            self.ledger_path.clone(),
            frames,
            place,
            first,
            scope,
        ));
        let frames = stream::unfold(sent, |mut sent| async move {
            sent.recv()
                .await
                .map(|bytes| (Ok::<_, std::io::Error>(Frame::data(bytes)), sent))
        });
        Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-store")
            // A proxy in between must not hold the stream back to fill a
            // buffer.
            .header("x-accel-buffering", "no")
            .body(StreamBody::new(frames).boxed())
            .expect("static response")
    }
}

/// Follow the ledger from a place for as long as the page listens: the
/// first frame (the state whole, or a reset), then every event within the
/// scope's reach as it happens, and a word now and then while nothing does.
async fn follow(
    path: std::path::PathBuf,
    frames: tokio::sync::mpsc::Sender<Bytes>,
    mut place: u64,
    first: Option<(&'static str, Value)>,
    scope: Scope,
) {
    if let Some((event, now)) = first
        && frames.send(frame(event, Some(place), &now)).await.is_err()
    {
        return;
    }
    let mut quiet = tokio::time::Instant::now();
    // One connection to the ledger for as long as the page listens.
    let mut kept: Option<RoutingLedger> = None;
    // The chats the scope reaches, when it does not reach them all;
    // looked at again when somebody joined or left one.
    let mut admitted: Option<BTreeSet<String>> = None;
    loop {
        let ledger_path = path.clone();
        let ledger = kept.take();
        let scope_now = scope.clone();
        let mut reach = admitted.take();
        let looked = tokio::task::spawn_blocking(move || {
            let ledger = match ledger {
                Some(ledger) => ledger,
                None => RoutingLedger::open(&ledger_path)?,
            };
            let happened = ledger.happened_with_what_was_said(place, AT_ONCE)?;
            let membership_moved = happened.iter().any(|(event, _)| {
                matches!(
                    event.kind.as_str(),
                    "chat/joined" | "chat/left" | "chat/started" | "chat/retired"
                )
            });
            if reach.is_none() || membership_moved {
                reach = WorkbenchShellState::admitted_now(&ledger, &scope_now);
            }
            Ok::<_, crate::RoutingError>((ledger, happened, reach))
        })
        .await;
        let Ok(Ok((ledger, happened, reach))) = looked else {
            // The ledger could not be read this time; the page
            // keeps its place and is given the rest when it can be.
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        kept = Some(ledger);
        admitted = reach;
        let caught_up = happened.len() < AT_ONCE;
        for (event, message) in happened {
            place = event.sequence;
            // Outside the scope's reach: passed over, the place
            // kept.
            if let Some(admitted) = &admitted
                && !event
                    .chat_id
                    .as_deref()
                    .is_some_and(|chat| admitted.contains(chat))
            {
                continue;
            }
            let mut data = serde_json::to_value(&event).unwrap_or(Value::Null);
            if let (Some(message), Value::Object(data)) = (message, &mut data) {
                data.insert(
                    "message".into(),
                    serde_json::to_value(message).unwrap_or(Value::Null),
                );
            }
            if frames
                .send(frame("event", Some(place), &data))
                .await
                .is_err()
            {
                return;
            }
            quiet = tokio::time::Instant::now();
        }
        if !caught_up {
            continue;
        }
        if quiet.elapsed() >= STILL_HERE {
            if frames
                .send(Bytes::from_static(b": still here\n\n"))
                .await
                .is_err()
            {
                return;
            }
            quiet = tokio::time::Instant::now();
        }
        tokio::select! {
            () = frames.closed() => return,
            () = tokio::time::sleep(LOOK) => {}
        }
    }
}

/// Answer what a page asks about chats, or hand the request back when it
/// asks about something else.
/// Somebody brought into a chat: an agent by its profile, or a guest.
async fn bring_in(
    state: &Arc<WorkbenchShellState>,
    chat_id: &str,
    body: BringBody,
) -> Result<crate::Chat, WorkbenchShellError> {
    match body {
        BringBody {
            agent: Some(agent), ..
        } => state.bring_into_chat(chat_id, &agent).await,
        BringBody {
            guest: Some(guest), ..
        } => state.let_guest_into_chat(chat_id, &guest).await,
        BringBody { .. } => Err(WorkbenchShellError::Invalid(
            "name the agent or the guest to bring in".into(),
        )),
    }
}

/// A chat that is within the scope's reach, or the refusal.
async fn chat_within(
    state: &WorkbenchShellState,
    scope: &Scope,
    chat_id: &str,
) -> Result<crate::Chat, WorkbenchShellError> {
    let chat = state.chat_named(chat_id).await?;
    if scope.admits(&chat) {
        Ok(chat)
    } else {
        Err(WorkbenchShellError::Forbidden("not a chat of yours".into()))
    }
}

/// A question that is the scope's to answer: the owner's, in a chat within
/// reach.
async fn question_within(
    state: &WorkbenchShellState,
    scope: &Scope,
    question_id: &str,
) -> Result<(), WorkbenchShellError> {
    if matches!(scope, Scope::Everything) {
        return Ok(());
    }
    if !scope.is_the_owner() {
        return Err(WorkbenchShellError::Forbidden(
            "the agent's questions are its owner's to answer".into(),
        ));
    }
    let id = question_id.to_owned();
    let question = state
        .with_ledger(move |ledger| ledger.question(&id))
        .await
        .map_err(ledger_refusal)?;
    chat_within(state, scope, &question.chat_id)
        .await
        .map(|_| ())
}

/// Something said in a chat: by the owner as the page says it, or by
/// somebody who came through a messenger - as themself, through the
/// messenger's channel, for the record when the agent is not theirs to
/// speak to.
async fn say_within(
    state: &Arc<WorkbenchShellState>,
    scope: &Scope,
    who: Option<&Principal>,
    chat_id: &str,
    saying: Saying,
) -> Response<ShellBody> {
    match (scope, who) {
        (Scope::Everything, _) => json_result(state.say_in_chat(chat_id, saying).await),
        (
            Scope::Within {
                participant_id,
                may_speak,
                ..
            },
            Some(Principal {
                by: crate::CameBy::Messenger { channel_id, .. },
                ..
            }),
        ) => match chat_within(state, scope, chat_id).await {
            Ok(chat) => json_result(
                state
                    .say_in_chat_as(
                        &chat.chat_id,
                        Some(participant_id.clone()),
                        &crate::chat_ledger::channel_of(channel_id),
                        Saying {
                            for_the_record: saying.for_the_record || !may_speak,
                            ..saying
                        },
                    )
                    .await,
            ),
            Err(error) => error_response(&error),
        },
        _ => respond_json(StatusCode::FORBIDDEN, &json!({ "error": "forbidden" })),
    }
}

/// What a page asks about the agent's questions: one in full, and the
/// answer to it - the owner's to give, within reach.
async fn route_questions(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    request: Request<AskedBody>,
    scope: &Scope,
    who: Option<&Principal>,
) -> Response<ShellBody> {
    match (method, segments) {
        (&Method::GET, ["api", "questions", question_id]) => {
            match question_within(state, scope, question_id).await {
                Ok(()) => json_result(state.question_in_full(question_id).await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "questions", question_id, "answer"]) => {
            let question_id = (*question_id).to_owned();
            match read_json(request).await {
                Ok(answer) => match question_within(state, scope, &question_id).await {
                    Ok(()) => json_result(
                        state
                            .answer_in_chat_by(
                                &question_id,
                                answer,
                                who.and_then(|who| who.participant.clone()),
                            )
                            .await,
                    ),
                    Err(error) => error_response(&error),
                },
                Err(error) => error_response(&error),
            }
        }
        _ => respond_json(StatusCode::NOT_FOUND, &json!({ "error": "not found" })),
    }
}

/// What a page asks about people: who there is (as far as the scope
/// reaches), what somebody is called, an agent put to sleep.
async fn route_people(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    request: Request<AskedBody>,
    scope: &Scope,
) -> Response<ShellBody> {
    match (method, segments) {
        (&Method::GET, ["api", "people"]) => match scope {
            Scope::Everything => json_result(state.chat_people().await),
            Scope::Within { .. } => json_result(
                state
                    .chats_now_within(scope)
                    .await
                    .map(|now| json!({ "owner": now["owner"], "you": now["you"], "participants": now["participants"] })),
            ),
        },
        (&Method::PATCH, ["api", "people", participant_id]) => {
            let participant_id = (*participant_id).to_owned();
            match body_of::<NameBody>(request).await {
                Ok(body) => json_result(
                    state
                        .name_participant(&participant_id, body.name, body.handle, body.colour)
                        .await,
                ),
                Err(error) => error_response(&error),
            }
        }
        // Put an agent to sleep: the sessions held open for it are let go
        // of. The engines keep them; it wakes with the next thing said.
        (&Method::POST, ["api", "people", agent_id, "sleep"]) => {
            state.let_go_of(Some(agent_id)).await;
            json_result(Ok(json!({ "asleep": true })))
        }
        _ => respond_json(StatusCode::NOT_FOUND, &json!({ "error": "not found" })),
    }
}

pub(super) async fn route_chats(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    query: Option<&str>,
    request: Request<AskedBody>,
    who: Option<&Principal>,
) -> Result<Response<ShellBody>, Request<AskedBody>> {
    if !matches!(
        segments,
        ["api", "stream"] | ["api", "people" | "chats" | "questions", ..]
    ) {
        return Err(request);
    }
    // What this principal reaches: everything, or what is theirs within
    // the bot's agent.
    let scope = match state.scope_of(who).await {
        Ok(scope) => scope,
        Err(error) => return Ok(error_response(&error)),
    };
    Ok(match (method, segments) {
        (&Method::GET, ["api", "stream"]) => {
            let place = request
                .headers()
                .get("last-event-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .or_else(|| query_param(query, "cursor"))
                .and_then(|place| place.parse::<u64>().ok());
            state.stream_from(place, scope).await
        }
        (_, ["api", "people", ..]) => route_people(state, method, segments, request, &scope).await,
        (&Method::GET, ["api", "chats"]) => json_result(state.chats().await),
        (&Method::POST, ["api", "chats"]) => match body_of::<StartChatBody>(request).await {
            Ok(body) => json_result(state.start_chat(body).await),
            Err(error) => error_response(&error),
        },
        (&Method::GET, ["api", "chats", chat_id]) => {
            let before = query_param(query, "before").and_then(|before| before.parse().ok());
            let limit = query_param(query, "limit")
                .and_then(|limit| limit.parse::<usize>().ok())
                .unwrap_or(400)
                .clamp(1, 2000);
            match chat_within(state, &scope, chat_id).await {
                Ok(chat) => json_result(state.chat_page(&chat.chat_id, before, limit).await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::PATCH, ["api", "chats", chat_id]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<ChangeChatBody>(request).await {
                Ok(body) => json_result(state.change_chat(&chat_id, body).await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "chats", chat_id, "members"]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<BringBody>(request).await {
                Ok(body) => json_result(bring_in(state, &chat_id, body).await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::DELETE, ["api", "chats", chat_id, "members", participant_id]) => {
            json_result(state.take_out_of_chat(chat_id, participant_id).await)
        }
        (&Method::POST, ["api", "chats", chat_id, "messages"]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<Saying>(request).await {
                Ok(saying) => say_within(state, &scope, who, &chat_id, saying).await,
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "chats", chat_id, "stop"]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<StopBody>(request).await {
                Ok(body) => match chat_within(state, &scope, &chat_id).await {
                    Ok(chat) => json_result(
                        state
                            .stop_in_chat(&chat.chat_id, body.agent_id.as_deref())
                            .await,
                    ),
                    Err(error) => error_response(&error),
                },
                Err(error) => error_response(&error),
            }
        }
        (
            &Method::GET | &Method::POST,
            ["api", "chats", chat_id, "agents", agent_id, "session"],
        ) => json_result(
            state
                .session_in_chat(chat_id, agent_id, method == Method::POST)
                .await,
        ),
        (&Method::GET | &Method::POST, ["api", "questions", ..]) => {
            route_questions(state, method, segments, request, &scope, who).await
        }
        _ => return Err(request),
    })
}
