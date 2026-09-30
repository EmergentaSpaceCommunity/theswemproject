//! A product that is not SWEM, with the harness built into it.
//!
//! It has a server of its own, two people, and a way in of its own: each
//! person is given an address that lets them in. Under `/people/<name>/agents`
//! each has a Workbench over a data root of their own, called by this
//! product's name. Nothing of the harness listens; this product's server
//! hears every request, decides who asks, and hands the request over for
//! whom it let in.
//!
//! It also takes a kind of package of its own through the Store: a
//! **helper**, an MCP server with the tools this product calls. A helper is
//! an archive whose tree has a `helper.json` naming the program; before one
//! is promised the product starts it once, lists its tools and compares
//! them with the shape it calls, and a helper that does not answer that
//! shape is refused in words. A helper taken is given to every agent of
//! the person who took it, beside the product's own server.
//!
//! `cargo run -p swem-host --example built_in` prints the addresses. The
//! root is `SWEM_EXAMPLE_ROOT` when set, else a directory under the temp dir;
//! `SWEM_EXAMPLE_SERVER` names an MCP server of the product's own, given to
//! every agent for its owner.

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use http_body_util::{BodyExt as _, Full};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use swem_host::product::{Answering, BuiltIn, DataRoot, Product};
use swem_host::{
    CatalogEntry, InstallPlan, InstallReceipt, Kind, KindWords, Shape, ShellBody, Taker,
};

const CALLED: &str = "Example";

/// The kind of package this product takes, named as a product names its
/// own: reverse-DNS, with the version of the kind.
const HELPER: &str = "example/helper@1";

/// What this product calls of a helper. A product writes its shape once,
/// as a document like this one, and checks every candidate against it.
const HELPER_SHAPE: &str = r#"{
  "schema": "swem:shape@0.1",
  "id": "example/helper",
  "tools": [
    {"name": "echo", "inputSchema": {"type": "object", "properties": {"nonce": {"type": "string"}}, "required": ["nonce"]}}
  ]
}"#;

/// How a helper's tree names its program: `helper.json` at the top of the
/// tree, `{"program": "...", "args": [...]}`, the program absolute or
/// relative to the tree.
#[derive(serde::Deserialize)]
struct HelperManifest {
    program: String,
    #[serde(default)]
    args: Vec<String>,
}

/// This product's taker of helpers: what a product the harness is built
/// into registers for a kind of its own.
struct Helpers {
    shape: Shape,
    /// The helpers taken, as servers to give every agent: read from the
    /// receipts at start, kept as helpers are taken and removed.
    given: Mutex<Vec<(String, McpServerStdio)>>,
}

impl Helpers {
    fn at(installed: &Path) -> Result<Arc<Self>, String> {
        let helpers = Self {
            shape: Shape::parse(HELPER_SHAPE.as_bytes())?,
            given: Mutex::new(Vec::new()),
        };
        for receipt in swem_host::load_receipts(installed, &helpers.kind()).into_values() {
            if let Some(tree) = receipt.file.as_deref()
                && let Ok(server) = Self::server_in(tree, &receipt.registry_id)
            {
                helpers
                    .given
                    .lock()
                    .expect("the helpers")
                    .push((receipt.registry_id, server));
            }
        }
        Ok(Arc::new(helpers))
    }

    /// The tree a helper is: the tree itself when `helper.json` is at its
    /// top, else the one folder in it.
    fn server_in(tree: &Path, id: &str) -> Result<McpServerStdio, String> {
        let mut at = tree.to_path_buf();
        if !at.join("helper.json").is_file() {
            let mut entries = std::fs::read_dir(&at)
                .map_err(|error| error.to_string())?
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect::<Vec<_>>();
            if entries.len() == 1 {
                at = entries.remove(0);
            }
        }
        let manifest: HelperManifest = serde_json::from_slice(
            &std::fs::read(at.join("helper.json"))
                .map_err(|_| "a helper has a helper.json at the top of its tree".to_owned())?,
        )
        .map_err(|error| format!("helper.json: {error}"))?;
        let program = PathBuf::from(&manifest.program);
        let program = if program.is_absolute() {
            program
        } else {
            at.join(program)
        };
        Ok(McpServerStdio::new(id, program.display().to_string()).args(manifest.args))
    }

    fn servers(&self) -> Vec<McpServer> {
        self.given
            .lock()
            .expect("the helpers")
            .iter()
            .map(|(_, server)| McpServer::Stdio(server.clone()))
            .collect()
    }
}

impl Taker for Helpers {
    fn kind(&self) -> Kind {
        Kind::parse(HELPER).expect("a kind")
    }

    fn words(&self) -> KindWords {
        KindWords {
            one: format!("a helper for {CALLED}"),
            many: "Helpers".into(),
            after_install: format!("Every agent of yours has it, beside {CALLED}'s own server."),
        }
    }

    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        if entry.distribution.archive.is_none() {
            return Err("a helper is an archive with a helper.json in it".into());
        }
        Ok(())
    }

    /// Started once from where it was staged, its tools listed and compared
    /// with the shape; refused, nothing lands.
    fn check(&self, staged: &Path, plan: &InstallPlan) -> Result<(), String> {
        let server = Self::server_in(&staged.join("tree"), &plan.registry_id)?;
        let listed = swem_host::tools_listed_by_blocking(&server)?;
        self.shape
            .check(&listed)
            .map_err(|short| format!("{} is not a helper for {CALLED}: it {short}", plan.name))
    }

    fn after_install(&self, receipt: &InstallReceipt) -> Result<(), String> {
        let tree = receipt
            .file
            .as_deref()
            .ok_or_else(|| "the receipt names no tree".to_owned())?;
        let server = Self::server_in(tree, &receipt.registry_id)?;
        let mut given = self.given.lock().expect("the helpers");
        given.retain(|(id, _)| id != &receipt.registry_id);
        given.push((receipt.registry_id.clone(), server));
        Ok(())
    }

    fn removable(&self) -> bool {
        true
    }

    fn before_remove(&self, receipt: &InstallReceipt) -> Result<(), String> {
        self.given
            .lock()
            .expect("the helpers")
            .retain(|(id, _)| id != &receipt.registry_id);
        Ok(())
    }
}

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
        let data_root = DataRoot::at(root.join(name.to_lowercase()));
        // This product takes helpers: a kind of its own, checked against
        // the shape it calls, listed in this person's Store under its words.
        let helpers = Helpers::at(&data_root.installed())?;
        let mut product = Product::at(data_root)
            .called(CALLED)
            .owned_by(name)
            .takes(Arc::clone(&helpers) as Arc<dyn Taker>);
        // The product's own server, resolved for each agent as its session
        // opens: what this agent may reach of the product is settled here,
        // by the product, and never by the agent's profile. The helpers
        // this person took come with it.
        let own = std::env::var_os("SWEM_EXAMPLE_SERVER");
        let owner = name.to_lowercase();
        product = product.servers_for(move |profile| {
            let mut servers = Vec::new();
            if let Some(server) = &own {
                servers.push(McpServer::Stdio(
                    McpServerStdio::new("example", server.to_string_lossy().into_owned()).args(
                        vec![
                            "--for".to_owned(),
                            format!("{owner}/{}", profile.profile_id),
                        ],
                    ),
                ));
            }
            servers.extend(helpers.servers());
            servers
        });
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
