//! A tunnel that goes nowhere: the shape of `swem/tunnel@1`, standing at a
//! loopback address of its own for the URL it is handed and passing every
//! request through - whole. A proxy in between holds a response back until
//! it ends, as a vendor's edge does through a quick tunnel (measured on
//! 2026-10-05: a stream through one never reached the browser), so what a
//! test reaches "through the tunnel" behaves as the real road does: a
//! stream hangs, a page has to ask instead. A fixture: it stands in for a
//! tunnel vendor where the harness proves that a tunnel is opened, used,
//! and closed. Every call is written to `<home>/calls.jsonl`, one JSON line
//! each, so a test reads what the harness asked.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;

use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};
use swem_sdk::tunnel::Standing;

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Nothing {}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct OpenParams {
    url: String,
}

struct Tunnel {
    calls: PathBuf,
    standing: Mutex<Standing>,
    /// Told, the proxy stops listening.
    told: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    tool_router: ToolRouter<Self>,
}

/// One request through, whole: the body collected, sent on to the target,
/// and the whole answer collected before a byte of it goes back. What never
/// ends never goes back - which is the point.
async fn through(
    target: String,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Box<dyn std::error::Error + Send + Sync>> {
    let (parts, body) = request.into_parts();
    let body = body.collect().await?.to_bytes();
    let host = target.trim_start_matches("http://").trim_end_matches('/');
    let stream = tokio::net::TcpStream::connect(host).await?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await?;
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let mut forwarded = Request::builder()
        .method(parts.method)
        .uri(parts.uri.path_and_query().map_or("/", |path| path.as_str()));
    // Every header as it came, the `Host` the browser named included: a
    // vendor's edge passes it on, and the gate compares it with `Origin`.
    for (name, value) in &parts.headers {
        forwarded = forwarded.header(name, value);
    }
    let answer = sender
        .send_request(forwarded.body(Full::new(body))?)
        .await?;
    let (parts, body) = answer.into_parts();
    let body = body.collect().await?.to_bytes();
    let mut response = Response::builder().status(parts.status);
    for (name, value) in &parts.headers {
        if name != hyper::header::TRANSFER_ENCODING && name != hyper::header::CONTENT_LENGTH {
            response = response.header(name, value);
        }
    }
    Ok(response.body(Full::new(body))?)
}

/// Stand at a loopback address of the fixture's own and pass everything
/// through to the target, whole, until told to stop.
async fn stand(
    listener: tokio::net::TcpListener,
    target: String,
    mut told: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            _ = &mut told => break,
            accepted = listener.accept() => {
                let Ok((stream, _)) = accepted else { continue };
                let target = target.clone();
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |request| through(target.clone(), request));
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await;
                });
            }
        }
    }
}

impl Tunnel {
    fn record(&self, tool: &str, params: &Value) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.calls)
        {
            // Two instances of this program (the gate's and the sandbox's)
            // write one file at once: a line is written whole, in one call,
            // so their lines never run into each other.
            let line = format!("{}\n", json!({ "tool": tool, "params": params }));
            let _ = file.write_all(line.as_bytes());
        }
    }
}

#[tool_router]
impl Tunnel {
    #[tool(description = "Stand at an address from outside for a local URL")]
    async fn open(
        &self,
        Parameters(params): Parameters<OpenParams>,
    ) -> Result<Json<Standing>, String> {
        self.record("open", &json!({ "url": params.url }));
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| error.to_string())?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        let (told, told_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(stand(listener, params.url.clone(), told_rx));
        *self
            .told
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(told);
        let standing = Standing {
            origin: format!("http://127.0.0.1:{port}"),
            url: params.url,
            said: "a fixture: an address on this machine, every answer held back until it ends"
                .into(),
        };
        *self
            .standing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = standing.clone();
        Ok(Json(standing))
    }

    #[tool(description = "Leave the address")]
    async fn close(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Standing>, String> {
        self.record("close", &json!({}));
        if let Some(told) = self
            .told
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = told.send(());
        }
        *self
            .standing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Standing::default();
        Ok(Json(Standing::default()))
    }

    #[tool(description = "Where the tunnel stands, if anywhere")]
    async fn look(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Standing>, String> {
        self.record("look", &json!({}));
        Ok(Json(
            self.standing
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tunnel {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-tunnel-fixture",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("A tunnel that goes nowhere; a fixture")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let home = arguments
        .windows(2)
        .find(|pair| pair[0] == "--home")
        .map(|pair| PathBuf::from(&pair[1]))
        .or_else(|| std::env::var_os(swem_sdk::tunnel::HOME_VARIABLE).map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&home)?;
    Tunnel {
        calls: home.join("calls.jsonl"),
        standing: Mutex::new(Standing::default()),
        told: Mutex::new(None),
        tool_router: Tunnel::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}
