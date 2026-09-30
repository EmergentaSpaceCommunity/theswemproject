//! The MCP servers a person can give an agent.
//!
//! An agent that cannot reach anything is not configured. Two kinds of server
//! end up in the same place here: the ones the product itself declared when
//! it was assembled, and the servers a person declares by hand or installs
//! from the Store, which are kept as ACP `McpServer` documents in the
//! catalogue directory. Both are attachable by name from a profile, and the name is the
//! only thing a profile stores - so a declaration can be corrected without
//! touching every profile that uses it.
//!
//! Secret material is never returned from here. A declaration may carry
//! environment values (an API key an MCP server needs); the views below carry
//! the *names* of those variables and never their values.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::{
    EnvVariable, HttpHeader, McpServer, McpServerHttp, McpServerSse, McpServerStdio,
};
use serde::{Deserialize, Serialize};

use super::{WorkbenchShellError, declaration_name};

/// Where a declaration came from, which decides whether it can be removed
/// here: what the product declared is the product's to take away.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpServerOrigin {
    /// A server the product declared when it was assembled.
    Product,
    /// A server a person declared in the catalogue.
    Catalogue,
}

/// One attachable MCP server, as a person sees it. Never carries a secret.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpServerView {
    pub name: String,
    pub transport: String,
    pub origin: McpServerOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// The environment variable names this declaration sets, without values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// The header names this declaration sends, without values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub header_names: Vec<String>,
}

/// One name/value pair a person typed for a declaration.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct NamedValue {
    pub name: String,
    #[serde(default)]
    pub value: String,
}

/// What a person fills in to declare a server.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeclareMcpServerBody {
    pub name: String,
    /// `stdio`, `http` or `sse`.
    pub transport: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<NamedValue>,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: Vec<NamedValue>,
}

/// The catalogue: a directory of ACP `McpServer` documents, loaded into the
/// same declaration map the product's own servers are in, so an attachment
/// resolves the same way whatever the server is.
#[derive(Clone, Debug)]
pub struct McpCatalogue {
    /// Who keeps the declarations: they hold what a person gave - the key
    /// in a header, the token in an environment - so they are kept as a
    /// key is.
    keeper: crate::Keeper,
    /// What the declarations are named under at the keeper.
    under: String,
    declared: std::sync::Arc<std::sync::Mutex<BTreeMap<String, McpServer>>>,
}

impl McpCatalogue {
    /// Open the catalogue as files under `root`, closed to others, and load
    /// every declaration in it.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory cannot be made or a document in it is
    /// not a dialable ACP declaration - a catalogue that silently drops a
    /// server would leave a profile attached to nothing.
    pub fn open(
        root: &Path,
        declared: std::sync::Arc<std::sync::Mutex<BTreeMap<String, McpServer>>>,
    ) -> Result<Self, WorkbenchShellError> {
        let keeper = crate::InFiles::at(root).map_err(|error| {
            WorkbenchShellError::Failed(format!(
                "the declared servers at {} could not be kept: {error}",
                root.display()
            ))
        })?;
        // What an earlier version left open is closed here.
        for entry in std::fs::read_dir(root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        {
            let path = entry
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
                .path();
            if path.is_file() {
                crate::closed::file(&path)
                    .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            }
        }
        Self::kept_by(std::sync::Arc::new(keeper), "", declared)
    }

    /// The catalogue kept by `keeper`, named under `under` (`mcp-servers/`,
    /// or nothing), with every declaration loaded.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub fn kept_by(
        keeper: crate::Keeper,
        under: &str,
        declared: std::sync::Arc<std::sync::Mutex<BTreeMap<String, McpServer>>>,
    ) -> Result<Self, WorkbenchShellError> {
        let catalogue = Self {
            keeper,
            under: under.to_owned(),
            declared,
        };
        for (name, server) in catalogue.stored()? {
            catalogue.insert_declared(&name, server)?;
        }
        Ok(catalogue)
    }

    /// The declaration map this catalogue shares with the rest of the host.
    #[must_use]
    pub fn declared(&self) -> std::sync::Arc<std::sync::Mutex<BTreeMap<String, McpServer>>> {
        std::sync::Arc::clone(&self.declared)
    }

    /// The environment values a declared stdio server has, by name, for
    /// declaring it again with them kept. Nothing for a server that is not
    /// declared or is not a stdio one.
    #[must_use]
    pub fn env_of(&self, name: &str) -> Vec<NamedValue> {
        let Ok(declared) = self.declared.lock() else {
            return Vec::new();
        };
        match declared.get(name) {
            Some(McpServer::Stdio(stdio)) => stdio
                .env
                .iter()
                .map(|variable| NamedValue {
                    name: variable.name.clone(),
                    value: variable.value.clone(),
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    fn name_of(&self, name: &str) -> Result<String, WorkbenchShellError> {
        validate_server_name(name)?;
        Ok(format!("{}{name}.json", self.under))
    }

    fn stored(&self) -> Result<BTreeMap<String, McpServer>, WorkbenchShellError> {
        let mut found = BTreeMap::new();
        let kept = self
            .keeper
            .list(&self.under)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        for document in kept {
            let Some(stem) = document
                .strip_prefix(&self.under)
                .and_then(|file| file.strip_suffix(".json"))
            else {
                continue;
            };
            let bytes = self
                .keeper
                .read(&document)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
                .unwrap_or_default();
            let server: McpServer = serde_json::from_slice(&bytes).map_err(|error| {
                WorkbenchShellError::Failed(format!("{document} is not a declaration: {error}"))
            })?;
            let name = declaration_name(&server)?;
            if stem != name {
                return Err(WorkbenchShellError::Failed(format!(
                    "{document} declares the server {name}, which does not match its name"
                )));
            }
            found.insert(name, server);
        }
        Ok(found)
    }

    fn insert_declared(&self, name: &str, server: McpServer) -> Result<(), WorkbenchShellError> {
        self.declared
            .lock()
            .map_err(|_| WorkbenchShellError::Failed("declaration registry poisoned".into()))?
            .insert(name.to_owned(), server);
        Ok(())
    }

    /// Every attachable server, the catalogue's and the product's alike, by name.
    ///
    /// # Errors
    ///
    /// Fails closed when the registry is poisoned or the catalogue is
    /// unreadable.
    pub fn list(&self) -> Result<Vec<McpServerView>, WorkbenchShellError> {
        let mine = self.stored()?;
        let declared = self
            .declared
            .lock()
            .map_err(|_| WorkbenchShellError::Failed("declaration registry poisoned".into()))?
            .clone();
        Ok(declared
            .into_iter()
            .map(|(name, server)| {
                let origin = if mine.contains_key(&name) {
                    McpServerOrigin::Catalogue
                } else {
                    McpServerOrigin::Product
                };
                view_of(&name, &server, origin)
            })
            .collect())
    }

    /// Declare a server, or correct a declaration a person already made.
    ///
    /// # Errors
    ///
    /// Refuses an unusable name, a transport this host cannot dial, a missing
    /// command or url, and taking over the name of a server the product declared.
    pub fn declare(
        &self,
        body: &DeclareMcpServerBody,
    ) -> Result<McpServerView, WorkbenchShellError> {
        let name = body.name.trim().to_owned();
        let document = self.name_of(&name)?;
        let kept = self
            .keeper
            .read(&document)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            .is_some();
        if !kept {
            let taken = self
                .declared
                .lock()
                .map_err(|_| WorkbenchShellError::Failed("declaration registry poisoned".into()))?
                .contains_key(&name);
            if taken {
                return Err(WorkbenchShellError::Conflict(format!(
                    "{name} is already the name of a server this product declares; give this one another name"
                )));
            }
        }
        let server = build_declaration(&name, body)?;
        let bytes = serde_json::to_vec_pretty(&server)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.keeper
            .write(&document, &bytes)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.insert_declared(&name, server.clone())?;
        Ok(view_of(&name, &server, McpServerOrigin::Catalogue))
    }

    /// Forget one declaration a person made here.
    ///
    /// # Errors
    ///
    /// Refuses an unknown name and a server the product declared (which is
    /// the product's to take away).
    pub fn forget(&self, name: &str) -> Result<(), WorkbenchShellError> {
        let document = self.name_of(name)?;
        let kept = self
            .keeper
            .read(&document)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
            .is_some();
        if !kept {
            return Err(WorkbenchShellError::NotFound(format!(
                "no declared MCP server named {name} in this catalogue"
            )));
        }
        self.keeper
            .remove(&document)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        self.declared
            .lock()
            .map_err(|_| WorkbenchShellError::Failed("declaration registry poisoned".into()))?
            .remove(name);
        Ok(())
    }
}

fn view_of(name: &str, server: &McpServer, origin: McpServerOrigin) -> McpServerView {
    match server {
        McpServer::Stdio(stdio) => McpServerView {
            name: name.to_owned(),
            transport: "stdio".into(),
            origin,
            command: Some(stdio.command.display().to_string()),
            args: stdio.args.clone(),
            env_names: stdio
                .env
                .iter()
                .map(|variable| variable.name.clone())
                .collect(),
            url: None,
            header_names: Vec::new(),
        },
        McpServer::Http(http) => McpServerView {
            name: name.to_owned(),
            transport: "http".into(),
            origin,
            command: None,
            args: Vec::new(),
            env_names: Vec::new(),
            url: Some(http.url.clone()),
            header_names: http
                .headers
                .iter()
                .map(|header| header.name.clone())
                .collect(),
        },
        McpServer::Sse(sse) => McpServerView {
            name: name.to_owned(),
            transport: "sse".into(),
            origin,
            command: None,
            args: Vec::new(),
            env_names: Vec::new(),
            url: Some(sse.url.clone()),
            header_names: sse
                .headers
                .iter()
                .map(|header| header.name.clone())
                .collect(),
        },
        _ => McpServerView {
            name: name.to_owned(),
            transport: "unsupported".into(),
            origin,
            command: None,
            args: Vec::new(),
            env_names: Vec::new(),
            url: None,
            header_names: Vec::new(),
        },
    }
}

fn build_declaration(
    name: &str,
    body: &DeclareMcpServerBody,
) -> Result<McpServer, WorkbenchShellError> {
    match body.transport.trim() {
        "stdio" => {
            let command = body.command.trim();
            if command.is_empty() {
                return Err(WorkbenchShellError::Invalid(
                    "a server started here needs the command that starts it".into(),
                ));
            }
            let mut env = Vec::new();
            for pair in &body.env {
                let variable = pair.name.trim();
                if variable.is_empty() {
                    continue;
                }
                env.push(EnvVariable::new(variable, pair.value.clone()));
            }
            Ok(McpServer::Stdio(
                McpServerStdio::new(name, PathBuf::from(command))
                    .args(
                        body.args
                            .iter()
                            .map(|argument| argument.trim().to_owned())
                            .filter(|argument| !argument.is_empty())
                            .collect(),
                    )
                    .env(env),
            ))
        }
        transport @ ("http" | "sse") => {
            let url = body.url.trim();
            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Err(WorkbenchShellError::Invalid(
                    "a server reached over the network needs an http(s) address".into(),
                ));
            }
            let headers = body
                .headers
                .iter()
                .filter(|pair| !pair.name.trim().is_empty())
                .map(|pair| HttpHeader::new(pair.name.trim(), pair.value.clone()))
                .collect();
            if transport == "http" {
                Ok(McpServer::Http(
                    McpServerHttp::new(name, url).headers(headers),
                ))
            } else {
                Ok(McpServer::Sse(
                    McpServerSse::new(name, url).headers(headers),
                ))
            }
        }
        other => Err(WorkbenchShellError::Invalid(format!(
            "unknown transport {other}: this host dials stdio, http and sse"
        ))),
    }
}

/// A name that is safe as both an ACP server name and a file name.
fn validate_server_name(name: &str) -> Result<(), WorkbenchShellError> {
    if name.is_empty() || name.len() > 64 {
        return Err(WorkbenchShellError::Invalid(
            "a server name is 1-64 characters".into(),
        ));
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(WorkbenchShellError::Invalid(
            "a server name holds letters, digits, '-' and '_'".into(),
        ));
    }
    Ok(())
}
