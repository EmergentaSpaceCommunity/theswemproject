//! A Telegram Bot API that is nobody's: enough of it for a channel to run
//! against, on loopback, with a side door for a test. A fixture.
//!
//! Methods: `getMe`, `getUpdates` (long-polls a queue), `sendMessage`,
//! `sendMessageDraft`, `editMessageText`, `sendChatAction`, `sendDocument`,
//! `answerCallbackQuery`, `getFile` and the file download; everything else
//! answers `ok: false, description: "method not found"`. Any token is
//! accepted, and is the bot's name.
//!
//! The side door, for a test: `POST /_fixture/updates` queues an update (a
//! Telegram `Update` object, or `{"message": ..}` to be numbered here);
//! `GET /_fixture/sent` lists every method call made, in order, as
//! `{method, body}`; `POST /_fixture/reset` forgets both. A file to be
//! fetched is put under `<files>/<file_id>` with `--files <dir>`.
//!
//! It prints its address on one line of stdout: `http://127.0.0.1:<port>`.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use serde_json::{Value, json};

struct Fixture {
    updates: Mutex<VecDeque<Value>>,
    sent: Mutex<Vec<Value>>,
    next_update: Mutex<i64>,
    next_message: Mutex<i64>,
    files: PathBuf,
}

/// One text field of a multipart body, by its name.
fn multipart_field(text: &str, name: &str) -> Option<String> {
    let marker = format!("name=\"{name}\"");
    let (_, rest) = text.split_once(&marker)?;
    let (_, rest) = rest.split_once("\r\n\r\n")?;
    let (value, _) = rest.split_once("\r\n")?;
    Some(value.to_owned())
}

fn reply(status: StatusCode, body: &Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .expect("a response")
}

// Callers hand over a value they are done with; `json!` only reads it.
#[allow(clippy::needless_pass_by_value)]
fn ok(result: Value) -> Response<Full<Bytes>> {
    reply(StatusCode::OK, &json!({ "ok": true, "result": result }))
}

fn refused(description: &str) -> Response<Full<Bytes>> {
    reply(
        StatusCode::BAD_REQUEST,
        &json!({ "ok": false, "description": description }),
    )
}

impl Fixture {
    fn record(&self, method: &str, body: &Value) {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(json!({ "method": method, "body": body }));
    }

    fn message_id(&self) -> i64 {
        let mut next = self
            .next_message
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *next += 1;
        *next
    }

    async fn method(&self, method: &str, body: Value) -> Response<Full<Bytes>> {
        self.record(method, &body);
        match method {
            "getMe" => ok(json!({
                "id": 1000,
                "is_bot": true,
                "first_name": "Fixture bot",
                "username": "swem_fixture_bot"
            })),
            "getUpdates" => {
                let timeout = body
                    .get("timeout")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    .min(30);
                let offset = body.get("offset").and_then(Value::as_i64);
                let deadline = Instant::now() + Duration::from_secs(timeout);
                loop {
                    let ready: Vec<Value> = {
                        let mut updates = self
                            .updates
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        // Confirmed updates (before the offset) are gone.
                        if let Some(offset) = offset {
                            updates.retain(|update| {
                                update.get("update_id").and_then(Value::as_i64) >= Some(offset)
                            });
                        }
                        updates.iter().cloned().collect()
                    };
                    if !ready.is_empty() || Instant::now() >= deadline {
                        return ok(Value::Array(ready));
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
            "sendMessage" | "sendMessageDraft" | "editMessageText" => {
                if body.get("chat_id").is_none() {
                    return refused("Bad Request: chat_id is empty");
                }
                let id = if method == "editMessageText" {
                    body.get("message_id").and_then(Value::as_i64).unwrap_or(0)
                } else {
                    self.message_id()
                };
                ok(json!({
                    "message_id": id,
                    "chat": { "id": body.get("chat_id") },
                    "text": body.get("text")
                }))
            }
            "sendChatAction" | "answerCallbackQuery" | "setWebhook" | "deleteWebhook" => {
                ok(Value::Bool(true))
            }
            "getFile" => {
                let file_id = body
                    .get("file_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let path = self.files.join(file_id);
                match std::fs::metadata(&path) {
                    Ok(meta) => ok(json!({
                        "file_id": file_id,
                        "file_unique_id": file_id,
                        "file_size": meta.len(),
                        "file_path": file_id
                    })),
                    Err(_) => refused("Bad Request: file not found"),
                }
            }
            _ => reply(
                StatusCode::NOT_FOUND,
                &json!({ "ok": false, "description": "Not Found: method not found" }),
            ),
        }
    }

    async fn answer(self: Arc<Self>, request: Request<Incoming>) -> Response<Full<Bytes>> {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let content_type = request
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        let bytes = request
            .into_body()
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes)
            .unwrap_or_default();
        // The side door.
        if path == "/_fixture/updates" && method == Method::POST {
            let mut update: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            if update.get("update_id").is_none() {
                let mut next = self
                    .next_update
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                *next += 1;
                update["update_id"] = json!(*next);
            }
            self.updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push_back(update.clone());
            return ok(update);
        }
        if path == "/_fixture/sent" && method == Method::GET {
            let sent = self
                .sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            return ok(Value::Array(sent));
        }
        if path == "/_fixture/reset" && method == Method::POST {
            self.updates
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
            self.sent
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
            return ok(Value::Bool(true));
        }
        // A file download: /file/bot<token>/<file_path>.
        if let Some(rest) = path.strip_prefix("/file/bot")
            && let Some((_, file_path)) = rest.split_once('/')
        {
            return match std::fs::read(self.files.join(file_path)) {
                Ok(bytes) => Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/octet-stream")
                    .body(Full::new(Bytes::from(bytes)))
                    .expect("a response"),
                Err(_) => refused("file not found"),
            };
        }
        // A method: /bot<token>/<method>.
        let Some(rest) = path.strip_prefix("/bot") else {
            return reply(
                StatusCode::NOT_FOUND,
                &json!({ "ok": false, "description": "Not Found" }),
            );
        };
        let Some((_, name)) = rest.split_once('/') else {
            return refused("no method");
        };
        let body: Value = if content_type.starts_with("multipart/") {
            // A document upload: what matters to a test is that it was sent,
            // where, and under what name.
            let text = String::from_utf8_lossy(&bytes);
            let mut body = json!({ "multipart": true, "bytes": bytes.len() });
            for field in ["chat_id", "message_thread_id", "caption"] {
                if let Some(value) = multipart_field(&text, field) {
                    body[field] = json!(value);
                }
            }
            if let Some(rest) = text.split_once("filename=\"").map(|(_, rest)| rest)
                && let Some((name, _)) = rest.split_once('"')
            {
                body["file_name"] = json!(name);
            }
            body
        } else if bytes.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(&bytes).unwrap_or(Value::Null)
        };
        if name == "sendDocument" {
            self.record(name, &body);
            let id = self.message_id();
            return ok(json!({ "message_id": id, "document": { "file_id": format!("doc-{id}") } }));
        }
        self.method(name, body).await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let files = arguments
        .windows(2)
        .find(|pair| pair[0] == "--files")
        .map_or_else(
            || std::env::temp_dir().join("swem-telegram-api-fixture-files"),
            |pair| PathBuf::from(&pair[1]),
        );
    std::fs::create_dir_all(&files)?;
    let fixture = Arc::new(Fixture {
        updates: Mutex::new(VecDeque::new()),
        sent: Mutex::new(Vec::new()),
        next_update: Mutex::new(0),
        next_message: Mutex::new(0),
        files,
    });
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let address = listener.local_addr()?;
    println!("http://{address}");
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let fixture = Arc::clone(&fixture);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let fixture = Arc::clone(&fixture);
                async move { Ok::<_, Infallible>(fixture.answer(request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}
