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
    Saying, ShellBody, StartChatBody, WorkbenchShellError, WorkbenchShellState, error_response,
    json_result, query_param, read_json,
};
use crate::RoutingLedger;

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

#[derive(Deserialize)]
struct RenameBody {
    title: String,
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
    request: Request<hyper::body::Incoming>,
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
        let people = self.chat_people().await?;
        self.with_ledger(move |ledger| {
            // The place first: what happens while the rest is read is sent
            // again after it, and being told twice changes nothing.
            let head = ledger.head()?;
            let owner = ledger.owner()?;
            Ok(json!({
                "head": head,
                "owner": people["owner"],
                "participants": people["participants"],
                "chats": ledger.chats_of(&owner.participant_id)?,
                "deliveries": ledger.deliveries_open(None)?,
                "questions": ledger.questions_waiting(None)?,
            }))
        })
        .await
        .map_err(ledger_refusal)
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

    /// The stream of everything that happens, from a place or from now.
    async fn stream_from(&self, place: Option<u64>) -> Response<ShellBody> {
        let (frames, sent) = tokio::sync::mpsc::channel::<Bytes>(64);
        let opening = match place {
            // A place past the end of the record is a place in another
            // record: the page is told to forget what it has.
            Some(place) => match self.with_ledger(|ledger| ledger.head()).await {
                Ok(head) if place <= head => Ok((place, None)),
                Ok(_) => self
                    .chats_now()
                    .await
                    .map(|now| (now["head"].as_u64().unwrap_or(0), Some(("reset", now)))),
                Err(error) => Err(ledger_refusal(error)),
            },
            None => self
                .chats_now()
                .await
                .map(|now| (now["head"].as_u64().unwrap_or(0), Some(("state", now)))),
        };
        let (mut place, first) = match opening {
            Ok(opening) => opening,
            Err(error) => return error_response(&error),
        };
        let path = self.ledger_path.clone();
        tokio::spawn(async move {
            if let Some((event, now)) = first
                && frames.send(frame(event, Some(place), &now)).await.is_err()
            {
                return;
            }
            let mut quiet = tokio::time::Instant::now();
            // One connection to the ledger for as long as the page listens.
            let mut kept: Option<RoutingLedger> = None;
            loop {
                let ledger_path = path.clone();
                let ledger = kept.take();
                let looked = tokio::task::spawn_blocking(move || {
                    let ledger = match ledger {
                        Some(ledger) => ledger,
                        None => RoutingLedger::open(&ledger_path)?,
                    };
                    let happened = ledger.happened_with_what_was_said(place, AT_ONCE)?;
                    Ok::<_, crate::RoutingError>((ledger, happened))
                })
                .await;
                let Ok(Ok((ledger, happened))) = looked else {
                    // The ledger could not be read this time; the page
                    // keeps its place and is given the rest when it can be.
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                };
                kept = Some(ledger);
                let caught_up = happened.len() < AT_ONCE;
                for (event, message) in happened {
                    place = event.sequence;
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
        });
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

/// Answer what a page asks about chats, or hand the request back when it
/// asks about something else.
pub(super) async fn route_chats(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    query: Option<&str>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<ShellBody>, Request<hyper::body::Incoming>> {
    Ok(match (method, segments) {
        (&Method::GET, ["api", "stream"]) => {
            let place = request
                .headers()
                .get("last-event-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
                .or_else(|| query_param(query, "cursor"))
                .and_then(|place| place.parse::<u64>().ok());
            state.stream_from(place).await
        }
        (&Method::GET, ["api", "people"]) => json_result(state.chat_people().await),
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
            json_result(state.chat_page(chat_id, before, limit).await)
        }
        (&Method::PATCH, ["api", "chats", chat_id]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<RenameBody>(request).await {
                Ok(body) => json_result(state.rename_chat(&chat_id, body.title).await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "chats", chat_id, "messages"]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<Saying>(request).await {
                Ok(saying) => json_result(state.say_in_chat(&chat_id, saying).await),
                Err(error) => error_response(&error),
            }
        }
        (&Method::POST, ["api", "chats", chat_id, "stop"]) => {
            let chat_id = (*chat_id).to_owned();
            match body_of::<StopBody>(request).await {
                Ok(body) => {
                    json_result(state.stop_in_chat(&chat_id, body.agent_id.as_deref()).await)
                }
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
        (&Method::GET, ["api", "questions", question_id]) => {
            json_result(state.question_in_full(question_id).await)
        }
        (&Method::POST, ["api", "questions", question_id, "answer"]) => {
            let question_id = (*question_id).to_owned();
            match read_json(request).await {
                Ok(answer) => json_result(state.answer_in_chat(&question_id, answer).await),
                Err(error) => error_response(&error),
            }
        }
        _ => return Err(request),
    })
}
