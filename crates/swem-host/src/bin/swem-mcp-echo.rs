//! Independent MCP capability fixture for Meta-harness contract tests.
//!
//! The server owns no SWEM Cycle state. Its single tool returns a caller
//! nonce and writes an external receipt so tests never treat agent prose as
//! evidence that a tool was actually invoked.

use std::fs;
use std::path::PathBuf;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct EchoRequest {
    nonce: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct EchoResponse {
    nonce: String,
    server: String,
}

#[derive(Clone, Debug)]
struct EchoServer {
    receipt: PathBuf,
    tool_router: ToolRouter<Self>,
}

impl EchoServer {
    fn new(receipt: PathBuf) -> Self {
        Self {
            receipt,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl EchoServer {
    /// Return the exact nonce and persist an out-of-band invocation receipt.
    #[tool(description = "Echo an opaque nonce and record that the MCP tool executed")]
    fn echo(
        &self,
        Parameters(EchoRequest { nonce }): Parameters<EchoRequest>,
    ) -> Result<Json<EchoResponse>, String> {
        let response = EchoResponse {
            nonce,
            server: "swem-mcp-echo".into(),
        };
        let bytes = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        fs::write(&self.receipt, bytes).map_err(|error| error.to_string())?;
        Ok(Json(response))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for EchoServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-mcp-echo",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Cycle-independent echo capability fixture")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let receipt = arguments
        .windows(2)
        .find(|pair| pair[0] == "--receipt")
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or("usage: swem-mcp-echo --receipt <absolute-path>")?;
    if !receipt.is_absolute() {
        return Err("receipt path must be absolute".into());
    }
    EchoServer::new(receipt)
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
