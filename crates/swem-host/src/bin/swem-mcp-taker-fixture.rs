//! A server that takes packages of a kind of its own, as the Cycle's hub
//! does: `plan_package` reads a package's manifest from a directory and
//! answers a plan, `install_package` copies the planned package home, and
//! `remove_package` removes it. A fixture: it stands in for the hub where
//! the harness proves that a server takes packages through the Store.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Source {
    kind: String,
    path: PathBuf,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct PlanParams {
    source: Source,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
#[allow(
    clippy::struct_field_names,
    reason = "the field is named as the hub names it on the wire"
)]
struct Plan {
    plan_id: String,
    id: String,
    version: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct InstallParams {
    plan_id: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct RemoveParams {
    id: String,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct Taken {
    id: String,
}

#[derive(Debug)]
struct Taker {
    home: PathBuf,
    /// What was planned and not yet installed, by plan id: where it is and
    /// what it is called.
    /// A plan id names a tree by its contents, so the same package planned
    /// again from another directory answers the same id: the last plan wins.
    planned: Mutex<BTreeMap<String, (PathBuf, String)>>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl Taker {
    #[tool(
        description = "Read a package from a directory and answer the plan installing it consents to"
    )]
    fn plan_package(
        &self,
        Parameters(PlanParams { source }): Parameters<PlanParams>,
    ) -> Result<Json<Plan>, String> {
        if source.kind != "directory" {
            return Err(format!(
                "this fixture takes a directory, not {}",
                source.kind
            ));
        }
        let manifest: serde_json::Value = serde_json::from_slice(
            &fs::read(source.path.join("plugin.json"))
                .map_err(|error| format!("no plugin.json in {}: {error}", source.path.display()))?,
        )
        .map_err(|error| error.to_string())?;
        let id = manifest["name"]
            .as_str()
            .ok_or("the manifest names no package")?
            .to_owned();
        let version = manifest["version"].as_str().unwrap_or("0").to_owned();
        let plan_id = format!("sha256-fixture-{id}-{version}");
        self.planned
            .lock()
            .map_err(|_| "poisoned")?
            .insert(plan_id.clone(), (source.path.clone(), id.clone()));
        Ok(Json(Plan {
            plan_id,
            id,
            version,
        }))
    }

    #[tool(description = "Install the package planned under this id")]
    fn install_package(
        &self,
        Parameters(InstallParams { plan_id }): Parameters<InstallParams>,
    ) -> Result<Json<Taken>, String> {
        let (from, id) = {
            self.planned
                .lock()
                .map_err(|_| "poisoned")?
                .remove(&plan_id)
                .ok_or_else(|| format!("nothing was planned under {plan_id}"))?
        };
        let into = self.home.join(&id);
        let _ = fs::remove_dir_all(&into);
        fs::create_dir_all(&into).map_err(|error| format!("make {}: {error}", into.display()))?;
        for entry in fs::read_dir(&from)
            .map_err(|error| format!("read {}: {error}", from.display()))?
            .flatten()
        {
            if entry.path().is_file() {
                fs::copy(entry.path(), into.join(entry.file_name()))
                    .map_err(|error| format!("copy {}: {error}", entry.path().display()))?;
            }
        }
        Ok(Json(Taken { id }))
    }

    #[tool(description = "Remove a package installed through this server")]
    fn remove_package(
        &self,
        Parameters(RemoveParams { id }): Parameters<RemoveParams>,
    ) -> Result<Json<Taken>, String> {
        let at = self.home.join(&id);
        fs::remove_dir_all(&at).map_err(|error| format!("remove {}: {error}", at.display()))?;
        Ok(Json(Taken { id }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Taker {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "swem-mcp-taker-fixture",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("A server that takes packages of its own kind; a fixture")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = std::env::args().collect::<Vec<_>>();
    let home = arguments
        .windows(2)
        .find(|pair| pair[0] == "--home")
        .map(|pair| PathBuf::from(&pair[1]))
        .ok_or("usage: swem-mcp-taker-fixture --home <absolute-path>")?;
    fs::create_dir_all(&home)?;
    Taker {
        home,
        planned: Mutex::new(BTreeMap::new()),
        tool_router: Taker::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await?
    .waiting()
    .await?;
    Ok(())
}
