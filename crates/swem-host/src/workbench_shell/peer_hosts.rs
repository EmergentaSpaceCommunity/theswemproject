//! The hosts of one person at the door: this host's face, "Add a host",
//! "Forget", what one host answers another over the link, and the page of
//! one host asking another through it (ADR-0019).

use std::path::Path;
use std::sync::Arc;

use http_body_util::BodyExt as _;
use hyper::{Method, Request, Response, StatusCode};
use iroh::{EndpointId, RelayMode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    AskedBody, ShellBody, WorkbenchShellError, WorkbenchShellState, error_response, json_result,
    read_json, respond_json,
};
use crate::peers::{Came, Meeting, Peers, PeersError, ThisHost, Told, serve_connection};
use crate::{CameBy, May, Principal};

/// Who came over the link, set by the link's own listener: nothing a caller
/// can send.
#[derive(Clone, Debug)]
pub(super) struct AsHost(pub Came);

impl From<PeersError> for WorkbenchShellError {
    fn from(error: PeersError) -> Self {
        match error {
            PeersError::Invalid(said) => Self::Invalid(said),
            PeersError::Refused(said) => Self::Forbidden(said),
            PeersError::Link(said) => Self::Failed(said),
            PeersError::Io { .. } | PeersError::Book(_) => Self::Failed(error.to_string()),
        }
    }
}

#[derive(Deserialize)]
struct AddHostBody {
    address: String,
    word: String,
}

#[derive(Deserialize)]
struct CallItBody {
    name: String,
}

impl WorkbenchShellState {
    /// This host among the person's hosts: its key under `root`, made at
    /// the first start, its endpoint bound, and its peers answered from
    /// now on. What this host is, for printing where it was started.
    ///
    /// # Errors
    ///
    /// The key or the book cannot be opened, or the endpoint cannot be bound.
    pub async fn enable_hosts(
        self: &Arc<Self>,
        root: &Path,
        relay: RelayMode,
    ) -> Result<ThisHost, WorkbenchShellError> {
        let peers = Arc::new(Peers::open(root, relay)?);
        if let Ok(owner) = self.owner_name().await {
            peers.owned_by(&owner);
        }
        let endpoint = peers.bind().await?;
        self.peers
            .set(Arc::clone(&peers))
            .map_err(|_| WorkbenchShellError::Conflict("hosts are enabled already".into()))?;
        let state = Arc::clone(self);
        let this_host = peers.this_host(false)?;
        tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let state = Arc::clone(&state);
                let peers = Arc::clone(&peers);
                tokio::spawn(async move {
                    let Ok(accepting) = incoming.accept() else {
                        return;
                    };
                    let Ok(connection) = accepting.await else {
                        return;
                    };
                    let host = connection.remote_id();
                    let came = match peers.trusted(&host) {
                        Ok(Some(name)) => {
                            peers.met_again(&host, &name);
                            Came::Trusted { host, name }
                        }
                        _ => Came::Unknown { host },
                    };
                    serve_connection(connection, came, move |came, request| {
                        let state = Arc::clone(&state);
                        async move {
                            let mut request = super::asked(request);
                            request.extensions_mut().insert(AsHost(came));
                            Box::pin(super::route_shell(&state, request)).await
                        }
                    })
                    .await;
                });
            }
        });
        Ok(this_host)
    }

    fn peers(&self) -> Result<&Arc<Peers>, WorkbenchShellError> {
        self.peers.get().ok_or_else(|| {
            WorkbenchShellError::Conflict("this Workbench has no host of its own here".into())
        })
    }

    /// Whether hosts are enabled here at all: the page shows nothing of
    /// them otherwise.
    #[must_use]
    pub fn has_a_host(&self) -> bool {
        self.peers.get().is_some()
    }

    /// Who came over the link, when the request did.
    pub(super) fn came_as_host<Body>(request: &Request<Body>) -> Option<Came> {
        request
            .extensions()
            .get::<AsHost>()
            .map(|came| came.0.clone())
    }

    /// The principal a trusted host is: the owner, by that host.
    pub(super) fn host_principal(came: &Came) -> Option<Principal> {
        match came {
            Came::Trusted { host, name } => Some(Principal::owner_by(CameBy::Host {
                host_id: host.to_string(),
                name: name.clone(),
            })),
            Came::Unknown { .. } => None,
        }
    }

    /// This host and the person's others, as the page shows them. The
    /// owner is shown the word to add this host by.
    ///
    /// # Errors
    ///
    /// Hosts are not enabled, or the book cannot be read.
    pub fn hosts_standing(&self, with_a_word: bool) -> Result<Value, WorkbenchShellError> {
        let peers = self.peers()?;
        Ok(json!({
            "this": peers.this_host(with_a_word)?,
            "hosts": peers.hosts()?,
        }))
    }

    /// Add a host: reach it at `address`, give it `word`, vouch for it.
    ///
    /// # Errors
    ///
    /// The address is not one, the host is out of reach, or it refuses.
    pub async fn add_host(&self, address: &str, word: &str) -> Result<Value, WorkbenchShellError> {
        let shown = self.peers()?.introduce(address, word).await?;
        Ok(serde_json::to_value(shown).unwrap_or(Value::Null))
    }

    /// Forget a host: it comes in no more, and the others are told.
    ///
    /// # Errors
    ///
    /// It is not one of the person's hosts.
    pub async fn forget_host(&self, host_id: &str) -> Result<Value, WorkbenchShellError> {
        let host: EndpointId = host_id
            .parse()
            .map_err(|_| WorkbenchShellError::NotFound("no such host".into()))?;
        let peers = self.peers()?;
        if peers.trusted(&host)?.is_none() {
            return Err(WorkbenchShellError::NotFound(
                "that is not one of your hosts".into(),
            ));
        }
        // The host is told first, best effort, so that its page stops
        // listing this one; then it is forgotten here, told or not.
        let request = Request::builder()
            .method(Method::POST)
            .uri("/api/peers/forgotten")
            .header("content-type", "application/json")
            .body(http_body_util::Full::new(hyper::body::Bytes::from(
                json!({"host": peers.id().to_string()}).to_string(),
            )))
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), peers.ask(&host, request))
            .await;
        let name = peers.forget(&host)?;
        Ok(json!({"host_id": host_id, "name": name, "forgotten": true}))
    }

    /// Ask another host of the person's over the link, as the page asks
    /// this one: the same door, the same words.
    ///
    /// # Errors
    ///
    /// The host is not one of the person's, is out of reach, or did not
    /// answer in HTTP.
    pub async fn ask_host(
        &self,
        host_id: &str,
        method: Method,
        path: &str,
        content_type: Option<&str>,
        body: hyper::body::Bytes,
    ) -> Result<Response<ShellBody>, WorkbenchShellError> {
        let host: EndpointId = host_id
            .parse()
            .map_err(|_| WorkbenchShellError::NotFound("no such host".into()))?;
        let mut request = Request::builder().method(method).uri(path);
        if let Some(content_type) = content_type {
            request = request.header("content-type", content_type);
        }
        let request = request
            .body(http_body_util::Full::new(body))
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let answered = self.peers()?.ask(&host, request).await?;
        // What the other host said, as it said it: its page, its script
        // and its stream of events pass through whole, so that its
        // Workbench is drawn under this one's address. Only what belongs
        // to the one link is dropped.
        let (parts, body) = answered.into_parts();
        let mut response = Response::builder().status(parts.status);
        for (name, value) in &parts.headers {
            if !matches!(
                name.as_str(),
                "connection" | "transfer-encoding" | "content-length" | "keep-alive"
            ) {
                response = response.header(name, value);
            }
        }
        response
            .body(body.map_err(std::io::Error::other).boxed())
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }
}

/// Answer what is asked about hosts, or hand the request back.
#[allow(clippy::too_many_lines)]
pub(super) async fn route_hosts(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    query: Option<&str>,
    request: Request<AskedBody>,
    who: Option<&Principal>,
) -> Result<Response<ShellBody>, Request<AskedBody>> {
    // `/api/peers/...` is what the page asks about the person's hosts (the
    // places an agent lives are `/api/hosts`); `/peers/<id>/...`
    // is another host's own page, drawn under this one's address.
    let a_page = segments.len() >= 2 && segments[0] == "peers";
    if !a_page && (segments.len() < 2 || segments[0] != "api" || segments[1] != "peers") {
        return Err(request);
    }
    if a_page {
        let owner = who.is_some_and(|who| who.may(May::Everything));
        if !owner {
            return Ok(respond_json(
                StatusCode::FORBIDDEN,
                &json!({"error": "forbidden"}),
            ));
        }
        // The page is drawn under `/peers/<id>/`, with the stroke, so that
        // what it names beside itself is found beside it.
        if segments.len() == 2 && !request.uri().path().ends_with('/') {
            return Ok(Response::builder()
                .status(StatusCode::PERMANENT_REDIRECT)
                .header(
                    hyper::header::LOCATION,
                    format!("{}/", request.uri().path()),
                )
                .body(
                    http_body_util::Full::new(hyper::body::Bytes::new())
                        .map_err(std::io::Error::other)
                        .boxed(),
                )
                .expect("a redirect"));
        }
        return Ok(
            through_the_link(state, method, segments[1], &segments[2..], query, request).await,
        );
    }
    let came = WorkbenchShellState::came_as_host(&request);
    // What one host says to another over the link, and nobody else.
    match (method, &segments[2..], came) {
        (&Method::POST, ["meet"], Some(Came::Unknown { host })) => {
            let peers = match state.peers() {
                Ok(peers) => peers,
                Err(error) => return Ok(error_response(&error)),
            };
            let meeting = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<Meeting>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(meeting) => meeting,
                Err(error) => return Ok(error_response(&error)),
            };
            // The word the owner was shown on this host's page, or, while
            // nobody owns this served host yet, the word it printed at its
            // first start (ADR-0019: a host is added from a page that is
            // the person's, by a word only they were given).
            if let Err(error) = peers.use_the_word(&meeting.word) {
                let by_the_first_word = state.served_at.get().map(|served| {
                    served
                        .access
                        .use_the_first_word(&meeting.word, "another host")
                });
                if !matches!(by_the_first_word, Some(Ok(()))) {
                    return Ok(error_response(&WorkbenchShellError::from(error)));
                }
            }
            return Ok(json_result(
                peers
                    .met(&host, &meeting)
                    .map_err(WorkbenchShellError::from),
            ));
        }
        (&Method::POST, ["told"], Some(Came::Trusted { host, .. })) => {
            let peers = match state.peers() {
                Ok(peers) => peers,
                Err(error) => return Ok(error_response(&error)),
            };
            let told = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<Told>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(told) => told,
                Err(error) => return Ok(error_response(&error)),
            };
            return Ok(json_result(
                peers
                    .told(&host, &told)
                    .map(|()| json!({"told": true}))
                    .map_err(WorkbenchShellError::from),
            ));
        }
        (&Method::POST, ["forgotten"], Some(Came::Trusted { host, .. })) => {
            let peers = match state.peers() {
                Ok(peers) => peers,
                Err(error) => return Ok(error_response(&error)),
            };
            return Ok(json_result(
                peers
                    .forget(&host)
                    .map(|name| json!({"forgotten": name}))
                    .map_err(WorkbenchShellError::from),
            ));
        }
        (_, _, Some(Came::Unknown { .. })) => {
            return Ok(respond_json(
                StatusCode::FORBIDDEN,
                &json!({"error": "this host does not know you"}),
            ));
        }
        _ => {}
    }
    // The person, on a page: the owner alone.
    let owner = who.is_some_and(|who| who.may(May::Everything));
    if !owner {
        return Ok(respond_json(
            StatusCode::FORBIDDEN,
            &json!({"error": "forbidden"}),
        ));
    }
    match (method, &segments[2..]) {
        (&Method::GET, []) => Ok(json_result(state.hosts_standing(true))),
        (&Method::POST, []) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<AddHostBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return Ok(error_response(&error)),
            };
            Ok(json_result(state.add_host(&body.address, &body.word).await))
        }
        (&Method::PATCH, ["this"]) => {
            let body = match read_json(request).await.and_then(|value| {
                serde_json::from_value::<CallItBody>(value)
                    .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))
            }) {
                Ok(body) => body,
                Err(error) => return Ok(error_response(&error)),
            };
            let called = state
                .peers()
                .and_then(|peers| peers.call_it(&body.name).map_err(WorkbenchShellError::from));
            match called {
                Ok(()) => Ok(json_result(state.hosts_standing(false))),
                Err(error) => Ok(error_response(&error)),
            }
        }
        (&Method::DELETE, [host_id]) => Ok(json_result(state.forget_host(host_id).await)),
        (_, [host_id, rest @ ..]) if !rest.is_empty() => {
            // The page asks another host through this one: the same door.
            Ok(through_the_link(state, method, host_id, rest, query, request).await)
        }
        _ => Ok(respond_json(
            StatusCode::NOT_FOUND,
            &json!({"error": "no such route"}),
        )),
    }
}

/// What the page asks of another host, carried over the link and answered
/// as that host answered: the same door, the same words.
async fn through_the_link(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    host_id: &str,
    rest: &[&str],
    query: Option<&str>,
    request: Request<AskedBody>,
) -> Response<ShellBody> {
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned);
    let bytes = match request.into_body().collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => return error_response(&WorkbenchShellError::Invalid(error.to_string())),
    };
    let path = match query {
        Some(query) => format!("/{}?{query}", rest.join("/")),
        None => format!("/{}", rest.join("/")),
    };
    match state
        .ask_host(
            host_id,
            method.clone(),
            &path,
            content_type.as_deref(),
            bytes,
        )
        .await
    {
        Ok(response) => response,
        Err(error) => error_response(&error),
    }
}
