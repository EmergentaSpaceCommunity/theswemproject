//! A product that is not SWEM, with the harness built into it.
//!
//! It has a server of its own, two people, and a way in of its own: each
//! person is given an address that lets them in. Under `/people/<name>/agents`
//! each has a Workbench over a data root of their own, called by this
//! product's name. Nothing of the harness listens; this product's server
//! hears every request, decides who asks, and hands the request over for
//! whom it let in.
//!
//! `cargo run -p swem-host --example built_in` prints the addresses. The
//! root is `SWEM_EXAMPLE_ROOT` when set, else a directory under the temp dir;
//! `SWEM_EXAMPLE_SERVER` names an MCP server of the product's own, given to
//! every agent for its owner.

use std::convert::Infallible;
use std::sync::Arc;

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use swem_host::ShellBody;
use swem_host::product::{Answering, BuiltIn, DataRoot, Product};

const CALLED: &str = "Example";

struct Person {
    name: &'static str,
    /// What lets this person in. A product has its own way in; this one's
    /// is as small as a way in can be.
    key: String,
    under: String,
    agents: Answering,
}

fn said(status: StatusCode, words: &str) -> Response<ShellBody> {
    Response::builder()
        .status(status)
        .header("content-type", "text/html;charset=utf-8")
        .body(
            Full::new(Bytes::from(format!(
                "<!doctype html><title>{CALLED}</title><body style=\"font:16px system-ui;margin:48px\">\
                 <h1>{CALLED}</h1><p>{words}</p></body>"
            )))
            .map_err(|never| match never {})
            .boxed(),
        )
        .expect("a page")
}

fn came_with(request: &Request<Incoming>) -> Option<&str> {
    request
        .headers()
        .get_all(hyper::header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.split_once('='))
        .find_map(|(name, value)| (name.trim() == "example_person").then_some(value.trim()))
}

async fn answer(people: Arc<Vec<Person>>, request: Request<Incoming>) -> Response<ShellBody> {
    let path = request.uri().path().to_owned();
    if let Some(key) = path.strip_prefix("/enter/") {
        let Some(person) = people.iter().find(|person| person.key == key) else {
            return said(StatusCode::FORBIDDEN, "Nobody comes in with that.");
        };
        return Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(hyper::header::LOCATION, format!("{}/", person.under))
            .header(
                hyper::header::SET_COOKIE,
                format!("example_person={key}; Path=/; SameSite=Strict; HttpOnly"),
            )
            .body(
                Full::new(Bytes::new())
                    .map_err(|never| match never {})
                    .boxed(),
            )
            .expect("a redirect");
    }
    if let Some(person) = people.iter().find(|person| {
        path.strip_prefix(person.under.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }) {
        // Who asks is settled here, by this product, before the harness
        // hears of the request.
        if came_with(&request) != Some(person.key.as_str()) {
            return said(
                StatusCode::FORBIDDEN,
                &format!(
                    "These are {}'s agents. Come in with the address you were given.",
                    person.name
                ),
            );
        }
        return person.agents.answer(request).await;
    }
    if request.method() == Method::GET && path == "/" {
        return said(
            StatusCode::OK,
            "A product with agents built into it. Come in with the address you were given.",
        );
    }
    said(StatusCode::NOT_FOUND, "There is nothing here.")
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let root = std::env::var_os("SWEM_EXAMPLE_ROOT").map_or_else(
        || std::env::temp_dir().join("swem-built-in"),
        std::path::PathBuf::from,
    );
    let mut people = Vec::new();
    for name in ["Ada", "Bo"] {
        let under = format!("/people/{}/agents", name.to_lowercase());
        let mut product = Product::at(DataRoot::at(root.join(name.to_lowercase())))
            .called(CALLED)
            .owned_by(name);
        // The product's own server, resolved for each agent as its session
        // opens: what this agent may reach of the product is settled here,
        // by the product, and never by the agent's profile.
        if let Some(server) = std::env::var_os("SWEM_EXAMPLE_SERVER") {
            let owner = name.to_lowercase();
            product = product.servers_for(move |profile| {
                vec![McpServer::Stdio(
                    McpServerStdio::new("example", server.to_string_lossy().into_owned()).args(
                        vec![
                            "--for".to_owned(),
                            format!("{owner}/{}", profile.profile_id),
                        ],
                    ),
                )]
            });
        }
        let agents = product
            .assemble()?
            .built_in(BuiltIn {
                under: under.clone(),
            })
            .await?;
        people.push(Person {
            name,
            key: swem_host::mint_session_token()?,
            under,
            agents,
        });
    }
    let port = std::env::var("SWEM_EXAMPLE_PORT")
        .ok()
        .and_then(|port| port.parse::<u16>().ok())
        .unwrap_or(0);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    println!("{CALLED}: http://{address}/");
    for person in &people {
        println!(
            "{} comes in at: http://{address}/enter/{}",
            person.name, person.key
        );
    }
    let people = Arc::new(people);
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            continue;
        };
        let people = Arc::clone(&people);
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |request| {
                let people = Arc::clone(&people);
                async move { Ok::<_, Infallible>(answer(people, request).await) }
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
        });
    }
}
