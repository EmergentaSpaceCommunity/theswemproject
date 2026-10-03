//! A tunnel through Cloudflare's quick tunnel: `cloudflared tunnel --url
//! <local>` stands at a random `https://<words>.trycloudflare.com` address
//! with no account and no key, for as long as it runs. This program answers
//! the tunnel shape (`swem_sdk::tunnel`) over MCP and drives `cloudflared`
//! for the harness, which names no vendor.
//!
//! `cloudflared` itself is a tool the Store installs; its path is handed in
//! as `SWEM_TOOL_CLOUDFLARED`. Failing that, one on `PATH` is used - a
//! person who put it there themselves.
//!
//! Cloudflare says a quick tunnel is for testing and development: no
//! uptime promise, two hundred requests in flight, a hostname that changes
//! every run. That is what a tunnel for a while is.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::Deserialize;
use swem_sdk::tunnel::{self, Standing};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::sync::Mutex;

/// How long `cloudflared` is given to say its address.
const SAYS_ITS_ADDRESS_WITHIN: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Nothing {}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct OpenParams {
    url: String,
}

/// The tunnel that is open: the program, and where it stands.
struct Open {
    program: tokio::process::Child,
    standing: Standing,
}

struct Tunnel {
    open: Mutex<Option<Open>>,
    tool_router: ToolRouter<Self>,
}

/// The address in a line `cloudflared` prints, if the line carries one:
/// `https://<words>.trycloudflare.com`, boxed in its log line.
fn address_in(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|letter: char| letter.is_whitespace() || letter == '|')
        .unwrap_or(rest.len());
    let url = &rest[..end];
    url.ends_with(".trycloudflare.com")
        .then(|| url.trim_end_matches('/').to_owned())
}

#[tool_router]
impl Tunnel {
    #[tool(description = "Stand at an address from outside for a local URL")]
    async fn open(
        &self,
        Parameters(params): Parameters<OpenParams>,
    ) -> Result<Json<Standing>, String> {
        let mut slot = self.open.lock().await;
        if let Some(open) = slot.as_ref() {
            return Ok(Json(open.standing.clone()));
        }
        // Looked for when asked, not when started: the tool may be
        // installed after this program is, and the words then say so.
        let cloudflared = cloudflared()?;
        let mut program = tokio::process::Command::new(&cloudflared)
            .args(["tunnel", "--no-autoupdate", "--url", &params.url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| format!("cloudflared could not be started: {error}"))?;
        let stderr = program.stderr.take().ok_or("cloudflared's output")?;
        let mut lines = BufReader::new(stderr).lines();
        let found = tokio::time::timeout(SAYS_ITS_ADDRESS_WITHIN, async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(address) = address_in(&line) {
                    return Some(address);
                }
            }
            None
        })
        .await;
        let origin = match found {
            Ok(Some(origin)) => origin,
            Ok(None) => {
                let _ = program.kill().await;
                return Err("cloudflared ended without saying an address".to_owned());
            }
            Err(_) => {
                let _ = program.kill().await;
                return Err("cloudflared said no address within thirty seconds".to_owned());
            }
        };
        // The rest of its log is read and dropped, so the pipe never fills.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let standing = Standing {
            origin,
            url: params.url,
            said:
                "a Cloudflare quick tunnel: no account, no uptime promise, a new address each time"
                    .into(),
        };
        *slot = Some(Open {
            program,
            standing: standing.clone(),
        });
        Ok(Json(standing))
    }

    #[tool(description = "Leave the address")]
    async fn close(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Standing>, String> {
        if let Some(mut open) = self.open.lock().await.take() {
            let _ = open.program.kill().await;
        }
        Ok(Json(Standing::default()))
    }

    #[tool(description = "Where the tunnel stands, if anywhere")]
    async fn look(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Standing>, String> {
        Ok(Json(
            self.open
                .lock()
                .await
                .as_ref()
                .map(|open| open.standing.clone())
                .unwrap_or_default(),
        ))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Tunnel {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-tunnel-cloudflare",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "An address from outside for a while, through Cloudflare's quick tunnel",
            )
    }
}

/// Where `cloudflared` is: handed in by the Store, else on `PATH`.
fn cloudflared() -> Result<PathBuf, String> {
    if let Some(handed) = std::env::var_os(tunnel::tool_variable("cloudflared")) {
        let path = PathBuf::from(handed);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!("{} is not a file", path.display()));
    }
    let name = if cfg!(windows) {
        "cloudflared.exe"
    } else {
        "cloudflared"
    };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            "cloudflared is not installed: install the tool from the Store, or put it on PATH"
                .to_owned()
        })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    Tunnel {
        open: Mutex::new(None),
        tool_router: Tunnel::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_read_from_the_log_line_cloudflared_prints() {
        let line = "2026-01-01T00:00:00Z INF |  https://quiet-words-here.trycloudflare.com                                   |";
        assert_eq!(
            address_in(line).as_deref(),
            Some("https://quiet-words-here.trycloudflare.com")
        );
        assert_eq!(
            address_in(
                "2026-01-01T00:00:00Z INF Requesting new quick Tunnel on trycloudflare.com..."
            ),
            None
        );
        assert_eq!(
            address_in("INF Visit https://developers.cloudflare.com/ to learn more"),
            None
        );
    }
}
