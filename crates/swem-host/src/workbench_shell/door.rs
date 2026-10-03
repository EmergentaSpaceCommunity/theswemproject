//! The door of a Workbench: who asks, found before anything is answered.
//!
//! On the machine a person sits at, the door is the secret of the run and
//! the page's own origin. Served at an address it is sign-in: a device that
//! holds a passkey, a program that holds a token, a code to come back with.
//! There is one place where a request is let in, and it is here; what is
//! behind it is a terminal and a person's keys.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use http_body_util::BodyExt as _;
use hyper::header::HeaderValue;
use hyper::{HeaderMap, Method, Request, Response, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    AskedBody, ShellBody, WorkbenchShellState, carries_the_secret, from_the_workbenchs_own_page,
    read_json, respond_json,
};
use crate::{Access, AccessError, CameBy, May, Principal};

/// Where a request came from, as the listener saw it.
#[derive(Clone, Copy, Debug)]
pub(super) struct CameFrom(pub SocketAddr);

/// Who asks, as the product the harness is built into said when it handed
/// the request over. Only that product's own code can say it: it travels
/// with the request inside the process and is nothing a caller can send.
#[derive(Clone, Debug)]
pub(crate) struct SaidBy(pub Principal);

/// How the way to a Workbench served at an address is kept closed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Way {
    /// A certificate of its own.
    Certificate,
    /// The person's proxy stands in front, on this machine.
    ProxyInFront,
    /// `localhost`, which a browser takes for a safe place.
    ThisMachine,
}

impl Way {
    fn said(self) -> &'static str {
        match self {
            Self::Certificate => "certificate",
            Self::ProxyInFront => "proxy",
            Self::ThisMachine => "this-machine",
        }
    }
}

/// A Workbench served at an address, as the door knows it.
pub(super) struct ServedAt {
    access: Arc<Access>,
    way: Way,
    /// Where Apps are drawn, when they are.
    apps: Option<String>,
    /// The certificate the Workbench holds, when it holds one.
    certificate: Option<PathBuf>,
}

/// Until when the certificate in this file is good. Read when it is asked,
/// because a certificate is renewed while the Workbench runs.
#[must_use]
pub fn certificate_good_until(chain: &Path) -> Option<SystemTime> {
    use tokio_rustls::rustls::pki_types::CertificateDer;
    use tokio_rustls::rustls::pki_types::pem::PemObject as _;

    let first = CertificateDer::pem_file_iter(chain).ok()?.next()?.ok()?;
    let (_, read) = x509_parser::parse_x509_certificate(first.as_ref()).ok()?;
    let seconds = u64::try_from(read.validity().not_after.timestamp()).ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds))
}

/// A refusal, as it is answered.
type Refused = Box<Response<ShellBody>>;

/// A session is kept this long by the browser; the book ends it sooner
/// when nobody came with it.
const A_SESSION_IS_KEPT_S: u64 = 30 * 24 * 60 * 60;

#[derive(Deserialize)]
struct RegisterBody {
    word: String,
    name: String,
}

#[derive(Deserialize)]
struct FinishBody {
    ceremony: String,
    answered: Value,
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

#[derive(Deserialize)]
struct TokenBody {
    name: String,
    may: Vec<May>,
}

fn status_of(error: &AccessError) -> StatusCode {
    match error {
        AccessError::Invalid(_) => StatusCode::BAD_REQUEST,
        AccessError::Refused(_) => StatusCode::FORBIDDEN,
        AccessError::TooManyTries { .. } => StatusCode::TOO_MANY_REQUESTS,
        AccessError::Io { .. } | AccessError::Book(_) => StatusCode::BAD_GATEWAY,
    }
}

fn refusal(error: &AccessError) -> Response<ShellBody> {
    respond_json(status_of(error), &json!({ "error": error.to_string() }))
}

fn answered<T: serde::Serialize>(result: Result<T, AccessError>) -> Response<ShellBody> {
    match result {
        Ok(value) => respond_json(
            StatusCode::OK,
            &serde_json::to_value(value).unwrap_or(Value::Null),
        ),
        Err(error) => refusal(&error),
    }
}

async fn body_of<T: serde::de::DeserializeOwned>(
    request: Request<AskedBody>,
) -> Result<T, Refused> {
    let value = read_json(request)
        .await
        .map_err(|error| Box::new(super::error_response(&error)))?;
    serde_json::from_value(value).map_err(|error| {
        Box::new(respond_json(
            StatusCode::BAD_REQUEST,
            &json!({ "error": error.to_string() }),
        ))
    })
}

fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(hyper::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(key, value)| (key.trim() == name).then_some(value.trim()))
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(hyper::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

impl ServedAt {
    fn over_tls(&self) -> bool {
        self.access.address().scheme() == "https"
    }

    /// The cookie a session is kept in. Over TLS its name binds it to this
    /// name and to TLS; on `localhost` it is named by the port, as the
    /// secret of a run is, so two Workbenches do not take each other's.
    fn cookie_name(&self) -> String {
        if self.over_tls() {
            return match self.access.address().port() {
                Some(port) => format!("__Host-swem_signed_in_{port}"),
                None => "__Host-swem_signed_in".into(),
            };
        }
        format!(
            "swem_signed_in_{}",
            self.access.address().port_or_known_default().unwrap_or(0)
        )
    }

    fn kept(&self, session: &str, for_seconds: u64) -> Option<HeaderValue> {
        let secure = if self.over_tls() { "; Secure" } else { "" };
        HeaderValue::from_str(&format!(
            "{}={session}; Path=/; Max-Age={for_seconds}; SameSite=Strict; HttpOnly{secure}",
            self.cookie_name()
        ))
        .ok()
    }

    /// Whether a request that states where it came from came from the page
    /// served at this address. One that states nothing is a program, and a
    /// program comes in by what it holds.
    fn is_its_own_page(&self, headers: &HeaderMap) -> bool {
        let Some(origin) = headers.get("origin") else {
            return true;
        };
        let address = self.access.address().origin().ascii_serialization();
        origin.to_str().is_ok_and(|origin| origin == address)
    }

    fn who(&self, headers: &HeaderMap, from: &str) -> Result<Option<Principal>, AccessError> {
        if let Some(session) = cookie(headers, &self.cookie_name())
            && let Some(who) = self.access.of_session(session)?
        {
            return Ok(Some(who));
        }
        match bearer(headers) {
            Some(token) => self.access.of_token(token, from),
            None => Ok(None),
        }
    }
}

/// What anybody may ask: whether there is a door, and to be let through it;
/// and a messenger's delivery to a channel's door, which the channel
/// verifies itself.
fn open_to_anybody(method: &Method, segments: &[&str]) -> bool {
    matches!(
        (method, segments),
        (&Method::GET, ["api", "access"])
            | (
                &Method::POST,
                ["api", "access", "register" | "sign-in", "begin" | "finish"]
                    | ["api", "access", "come-back" | "sign-out"]
                    | ["api", "channels", _, "receive"]
            )
    )
}

/// What a channel's Mini App asks, from wherever it is hosted: its page,
/// and what it does on behalf of whoever the messenger says opened it.
pub(super) fn open_to_an_app(method: &Method, segments: &[&str]) -> bool {
    matches!(
        (method, segments),
        (
            &Method::GET | &Method::POST | &Method::OPTIONS,
            ["api", "channels", _, "app"] | ["api", "channels", _, "app", ..]
        )
    )
}

/// What somebody who came with a code may ask: a word for a device, so
/// that they have a device again.
fn open_to_a_code(method: &Method, segments: &[&str]) -> bool {
    matches!(
        (method, segments),
        (&Method::POST, ["api", "access", "devices", "word"])
    )
}

/// What a token may ask that says it may say what is due.
fn open_to_a_knock(method: &Method, segments: &[&str]) -> bool {
    matches!((method, segments), (&Method::POST, ["api", "time", "due"]))
}

impl WorkbenchShellState {
    /// Serve this Workbench at an address: from now on the door is sign-in.
    pub fn serve_at(
        &self,
        access: Arc<Access>,
        way: Way,
        apps: Option<String>,
        certificate: Option<PathBuf>,
    ) {
        let _ = self.served_at.set(ServedAt {
            access,
            way,
            apps,
            certificate,
        });
    }

    /// Where a request came from, in the words of what was done. Behind a
    /// proxy on this machine it is what the proxy says; the proxy is the
    /// person's own and nobody else reaches the listener.
    pub(super) fn came_from<Body>(&self, request: &Request<Body>) -> String {
        let peer = request
            .extensions()
            .get::<CameFrom>()
            .map(|from| from.0.ip());
        if self
            .served_at
            .get()
            .is_some_and(|served| served.way == Way::ProxyInFront)
            && peer.is_none_or(|peer| peer.is_loopback())
            && let Some(said) = request
                .headers()
                .get("x-forwarded-for")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.rsplit(',').next())
                .and_then(|last| last.trim().parse::<IpAddr>().ok())
        {
            return said.to_string();
        }
        peer.map_or_else(|| "somewhere".to_owned(), |peer| peer.to_string())
    }

    /// Let a request to `/api` in, or refuse it. Who asks is what comes
    /// back; nobody comes back only for what anybody may ask.
    pub(super) fn let_in(
        &self,
        method: &Method,
        segments: &[&str],
        headers: &HeaderMap,
        from: &str,
        said: Option<Principal>,
    ) -> Result<Option<Principal>, Refused> {
        // Nothing about the route: a caller that is not let in learns
        // neither which routes exist nor what they wanted.
        let forbidden = || {
            Box::new(respond_json(
                StatusCode::FORBIDDEN,
                &json!({"error": "forbidden"}),
            ))
        };
        if self.built_under.get().is_some() {
            // Who a person is, the product said. Which page asks is still
            // asked here: another site's page is no more this Workbench's
            // under a product's roof than under its own.
            return match said {
                Some(who) if from_the_workbenchs_own_page(headers) => Ok(Some(who)),
                _ => Err(forbidden()),
            };
        }
        // A Mini App of a channel's, hosted anywhere, says who opened it
        // with the messenger's signature; which page asks is not the
        // question there, and nobody is let in by this alone.
        if self.served_at.get().is_some() && open_to_an_app(method, segments) {
            return Ok(None);
        }
        let Some(served) = self.served_at.get() else {
            if !from_the_workbenchs_own_page(headers)
                || self
                    .session_token()
                    .is_some_and(|token| !carries_the_secret(headers, token))
            {
                return Err(forbidden());
            }
            return Ok(Some(Principal::of_this_run()));
        };
        if !served.is_its_own_page(headers) {
            let _ = served
                .access
                .refused("Refused: a page that is not this Workbench's", from);
            return Err(forbidden());
        }
        let who = served
            .who(headers, from)
            .map_err(|error| Box::new(refusal(&error)))?;
        if open_to_anybody(method, segments) {
            return Ok(who);
        }
        let Some(who) = who else {
            return Err(Box::new(respond_json(
                StatusCode::UNAUTHORIZED,
                &json!({"error": "sign in"}),
            )));
        };
        let may = who.may(May::Everything)
            || (matches!(who.by, CameBy::Code) && open_to_a_code(method, segments))
            || (who.may(May::SayWhatIsDue) && open_to_a_knock(method, segments));
        if may {
            return Ok(Some(who));
        }
        let _ = served.access.refused(
            &format!("Refused: {} asked for what it may not", who.named()),
            from,
        );
        Err(forbidden())
    }

    /// The name of the person this Workbench is: what a passkey shows for
    /// whom it lets in.
    ///
    /// # Errors
    ///
    /// The ledger cannot be read.
    pub async fn owner_name(&self) -> Result<String, super::WorkbenchShellError> {
        self.with_ledger(|ledger| ledger.owner().map(|owner| owner.name))
            .await
            .map_err(|error| super::WorkbenchShellError::Failed(error.to_string()))
    }

    /// Name the person this Workbench is as a product knows them.
    ///
    /// # Errors
    ///
    /// The ledger cannot be written, or the name is one no chat can carry.
    pub async fn name_the_owner(&self, name: &str) -> Result<(), super::WorkbenchShellError> {
        let name = name.trim().to_owned();
        self.with_ledger(move |ledger| {
            let owner = ledger.owner()?;
            if owner.name == name {
                return Ok(());
            }
            let handle = crate::handle_from(&name);
            ledger
                .name_participant(&owner.participant_id, Some(&name), Some(&handle), None)
                .map(|_| ())
        })
        .await
        .map_err(|error| super::WorkbenchShellError::Failed(error.to_string()))
    }

    /// Build this Workbench into a product: it is drawn under `under` on
    /// that product's server, and who asks is what the product says.
    pub fn build_into(&self, under: &str) {
        let under = under.trim_end_matches('/');
        let under = if under.is_empty() || under.starts_with('/') {
            under.to_owned()
        } else {
            format!("/{under}")
        };
        let _ = self.built_under.set(under);
    }

    /// Call the product by its own name on the page.
    pub fn call_it(&self, name: &str) {
        let _ = self.called.set(name.trim().to_owned());
    }

    /// What is asked for, with the path the Workbench is drawn under taken
    /// off. What is asked for elsewhere is not the Workbench's to answer,
    /// and the path without its last stroke is sent on to the one with it,
    /// so that what the page names beside itself is found.
    pub(super) fn under(&self, path: &str) -> Result<String, Refused> {
        let Some(under) = self.built_under.get().filter(|under| !under.is_empty()) else {
            return Ok(path.to_owned());
        };
        match path.strip_prefix(under.as_str()) {
            Some("") => Err(Box::new(
                Response::builder()
                    .status(StatusCode::PERMANENT_REDIRECT)
                    .header(hyper::header::LOCATION, format!("{under}/"))
                    .body(
                        http_body_util::Full::new(hyper::body::Bytes::new())
                            .map_err(|never| match never {})
                            .boxed(),
                    )
                    .expect("a redirect"),
            )),
            Some(rest) if rest.starts_with('/') => Ok(rest.to_owned()),
            _ => Err(Box::new(respond_json(
                StatusCode::NOT_FOUND,
                &json!({"error": "not found"}),
            ))),
        }
    }

    /// Who the product said asks, when the harness is built into one.
    /// Built in, a request nobody vouched for is answered by nothing: not
    /// by a route, not by the page.
    pub(super) fn said_by<Body>(
        &self,
        request: &Request<Body>,
    ) -> Result<Option<Principal>, Refused> {
        let said = request
            .extensions()
            .get::<SaidBy>()
            .map(|said| said.0.clone());
        if self.built_under.get().is_some() && said.is_none() {
            return Err(Box::new(respond_json(
                StatusCode::FORBIDDEN,
                &json!({"error": "forbidden"}),
            )));
        }
        Ok(said)
    }

    /// Where a scheduler outside knocks, when this Workbench is served at
    /// an address, and how many tokens that may knock were made.
    pub(super) fn knocked_on(&self) -> (Option<String>, usize) {
        let Some(served) = self.served_at.get() else {
            return (None, 0);
        };
        let knocking = served.access.tokens().map_or(0, |tokens| {
            tokens
                .iter()
                .filter(|token| token.may.contains(&May::SayWhatIsDue))
                .count()
        });
        (
            Some(format!(
                "{}/api/time/due",
                served.access.address().origin().ascii_serialization()
            )),
            knocking,
        )
    }

    /// Whether this Workbench is served at an address, and so opens by
    /// sign-in and not by the secret of a run.
    pub(super) fn is_served_at_an_address(&self) -> bool {
        self.served_at.get().is_some()
    }

    /// The origin this Workbench is served at, when it is: what a door for
    /// a messenger's deliveries is made of.
    pub(super) fn served_origin(&self) -> Option<String> {
        self.served_at
            .get()
            .map(|served| served.access.address().origin().ascii_serialization())
    }

    /// Whether what is answered goes over TLS, so that a browser is told
    /// to come over TLS from now on.
    pub(super) fn is_served_over_tls(&self) -> bool {
        self.served_at.get().is_some_and(ServedAt::over_tls)
    }

    fn door_standing(&self, who: Option<&Principal>) -> Value {
        let called = self.called.get().map_or("SWEM", String::as_str);
        match self.served_at.get() {
            None => json!({"at": null, "claimed": true, "who": who, "called": called}),
            Some(served) => json!({
                "at": served.access.address().origin().ascii_serialization(),
                "name": served.access.address().host_str(),
                "claimed": served.access.claimed().unwrap_or(true),
                "who": who,
                "called": called,
            }),
        }
    }

    fn access_shown(served: &ServedAt, who: &Principal) -> Result<Value, AccessError> {
        let this = match &who.by {
            CameBy::Device { device_id, .. } => Some(device_id.as_str()),
            _ => None,
        };
        let devices = served
            .access
            .devices()?
            .into_iter()
            .map(|device| {
                let here = this == Some(device.device_id.as_str());
                let mut shown = serde_json::to_value(device).unwrap_or(Value::Null);
                shown["this"] = Value::Bool(here);
                shown
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "at": served.access.address().origin().ascii_serialization(),
            "way": served.way.said(),
            "apps_at": served.apps,
            "certificate_good_until_ms": served
                .certificate
                .as_deref()
                .and_then(certificate_good_until)
                .and_then(|until| until.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX)),
            "devices": devices,
            "tokens": served.access.tokens()?,
            "codes": served.access.codes_left()?,
            "happened": served.access.happened(50)?,
        }))
    }
}

fn came_in(served: &ServedAt, came: &crate::CameIn) -> Response<ShellBody> {
    let mut response = respond_json(
        StatusCode::OK,
        &json!({"who": came.principal, "codes": came.codes}),
    );
    if let Some(kept) = served.kept(&came.session, A_SESSION_IS_KEPT_S) {
        response
            .headers_mut()
            .insert(hyper::header::SET_COOKIE, kept);
    }
    response
}

/// Answer what is asked of the door, or hand the request back when it asks
/// about something else. Whoever reaches this was let in by
/// [`WorkbenchShellState::let_in`].
#[allow(
    clippy::too_many_lines,
    reason = "one match over everything the door answers, so that what it answers is read in one place"
)]
pub(super) async fn route_door(
    state: &Arc<WorkbenchShellState>,
    method: &Method,
    segments: &[&str],
    request: Request<AskedBody>,
    who: Option<&Principal>,
    from: &str,
) -> Result<Response<ShellBody>, Request<AskedBody>> {
    if !matches!(segments, ["api", "access", ..]) {
        return Err(request);
    }
    if matches!((method, segments), (&Method::GET, ["api", "access"])) {
        return Ok(respond_json(StatusCode::OK, &state.door_standing(who)));
    }
    let Some(served) = state.served_at.get() else {
        // On the machine a person sits at there is nobody to let in.
        return Ok(respond_json(
            StatusCode::CONFLICT,
            &json!({"error": "This Workbench is served on this computer alone. \
                     Start it at an address to let devices and programs in."}),
        ));
    };
    let access = &served.access;
    let signed_in = || -> Result<Principal, Refused> {
        who.cloned().ok_or_else(|| {
            Box::new(respond_json(
                StatusCode::UNAUTHORIZED,
                &json!({"error": "sign in"}),
            ))
        })
    };
    Ok(match (method, segments) {
        (&Method::POST, ["api", "access", "register", "begin"]) => {
            match body_of::<RegisterBody>(request).await {
                Ok(body) => answered(access.begin_registering(&body.word, &body.name, from)),
                Err(response) => *response,
            }
        }
        (&Method::POST, ["api", "access", "register", "finish"]) => {
            match body_of::<FinishBody>(request).await {
                Ok(body) => match access.finish_registering(&body.ceremony, &body.answered, from) {
                    Ok(came) => came_in(served, &came),
                    Err(error) => refusal(&error),
                },
                Err(response) => *response,
            }
        }
        (&Method::POST, ["api", "access", "sign-in", "begin"]) => {
            answered(access.begin_signing_in())
        }
        (&Method::POST, ["api", "access", "sign-in", "finish"]) => {
            match body_of::<FinishBody>(request).await {
                Ok(body) => match access.finish_signing_in(&body.ceremony, &body.answered, from) {
                    Ok(came) => came_in(served, &came),
                    Err(error) => refusal(&error),
                },
                Err(response) => *response,
            }
        }
        (&Method::POST, ["api", "access", "come-back"]) => {
            match body_of::<CodeBody>(request).await {
                Ok(body) => match access.come_back(&body.code, from) {
                    Ok(came) => came_in(served, &came),
                    Err(error) => refusal(&error),
                },
                Err(response) => *response,
            }
        }
        (&Method::POST, ["api", "access", "sign-out"]) => {
            if let Some(session) = cookie(request.headers(), &served.cookie_name()) {
                let _ = access.end_session(session, from);
            }
            let mut response = respond_json(StatusCode::OK, &json!({"who": null}));
            if let Some(forgotten) = served.kept("", 0) {
                response
                    .headers_mut()
                    .insert(hyper::header::SET_COOKIE, forgotten);
            }
            response
        }
        (&Method::GET, ["api", "access", "standing"]) => match signed_in() {
            Ok(who) => answered(WorkbenchShellState::access_shown(served, &who)),
            Err(response) => *response,
        },
        (&Method::POST, ["api", "access", "devices", "word"]) => match signed_in() {
            Ok(who) => answered(
                access
                    .word_for_a_device(&who, from)
                    .map(|word| json!({"word": word, "good_for_minutes": 10})),
            ),
            Err(response) => *response,
        },
        (&Method::DELETE, ["api", "access", "devices", device_id]) => match signed_in() {
            Ok(who) => answered(
                access
                    .take_away(device_id, &who, from)
                    .map(|()| json!({"taken_away": device_id})),
            ),
            Err(response) => *response,
        },
        (&Method::POST, ["api", "access", "tokens"]) => match signed_in() {
            Ok(who) => match body_of::<TokenBody>(request).await {
                Ok(body) => answered(
                    access
                        .make_token(&body.name, &body.may, &who, from)
                        .map(|(token, opens)| json!({"token": token, "opens": opens})),
                ),
                Err(response) => *response,
            },
            Err(response) => *response,
        },
        (&Method::DELETE, ["api", "access", "tokens", token_id]) => match signed_in() {
            Ok(who) => answered(
                access
                    .withdraw(token_id, &who, from)
                    .map(|()| json!({"withdrawn": token_id})),
            ),
            Err(response) => *response,
        },
        (&Method::POST, ["api", "access", "codes"]) => match signed_in() {
            Ok(who) => answered(
                access
                    .new_codes(&who, from)
                    .map(|codes| json!({"codes": codes})),
            ),
            Err(response) => *response,
        },
        _ => respond_json(StatusCode::NOT_FOUND, &json!({"error": "not found"})),
    })
}

#[cfg(test)]
mod tests {
    use hyper::Method;

    use super::{open_to_a_code, open_to_a_knock, open_to_anybody};

    #[test]
    fn anybody_may_ask_for_the_door_and_for_nothing_behind_it() {
        for (method, path) in [
            (Method::GET, "api/access"),
            (Method::POST, "api/access/register/begin"),
            (Method::POST, "api/access/sign-in/finish"),
            (Method::POST, "api/access/come-back"),
        ] {
            let segments: Vec<&str> = path.split('/').collect();
            assert!(open_to_anybody(&method, &segments), "{path}");
        }
        for (method, path) in [
            (Method::GET, "api/access/standing"),
            (Method::POST, "api/access/tokens"),
            (Method::POST, "api/access/devices/word"),
            (Method::POST, "api/access/codes"),
            (Method::GET, "api/profiles"),
            (Method::GET, "api/stream"),
            (Method::POST, "api/time/due"),
            (Method::POST, "api/access/register/begin/more"),
        ] {
            let segments: Vec<&str> = path.split('/').collect();
            assert!(!open_to_anybody(&method, &segments), "{path}");
        }
    }

    #[test]
    fn a_code_and_a_knock_open_one_thing_each() {
        assert!(open_to_a_code(
            &Method::POST,
            &["api", "access", "devices", "word"]
        ));
        assert!(!open_to_a_code(&Method::POST, &["api", "access", "tokens"]));
        assert!(open_to_a_knock(&Method::POST, &["api", "time", "due"]));
        assert!(!open_to_a_knock(&Method::GET, &["api", "time", "due"]));
        assert!(!open_to_a_knock(&Method::POST, &["api", "schedules"]));
    }
}
