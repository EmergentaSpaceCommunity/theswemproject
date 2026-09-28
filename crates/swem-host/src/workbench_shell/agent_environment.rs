//! Configuring an agent: what it can reach, where it works, and a prompt.
//!
//! Access alone is not configuration. Until a person can say *which* MCP
//! servers this agent attaches - their own project's Cycle among them - the
//! agent starts blind, and binding a project to a session refuses every time
//! because the profile attaches nothing. These are the operations that make
//! the profile a thing a person owns rather than a row created once at
//! first run.
use super::*;
use crate::{AttachmentBinding, AttachmentTransport};

/// What a person changed about one profile. Everything is optional: a field
/// left out is left alone.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AmendProfileBody {
    /// The revision the person edited, so a second surface cannot be
    /// overwritten silently.
    #[serde(default)]
    pub revision: u64,
    /// Where the agent works. Created if it does not exist yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// The agent's own home directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_home: Option<String>,
    /// The MCP servers this agent attaches, by declared name. Replaces the
    /// whole set, because that is what the form shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<String>>,
    /// How this agent's permission requests are answered, by the id of one of
    /// the choices this host offers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    /// Where this agent runs, by the id of one of the environments this host
    /// offers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// The model provider, the model, the role and the skills. Replaces the
    /// whole setup, because that is what the form shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<crate::AgentSetup>,
}

/// What a person sent to a terminal.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct TerminalInputBody {
    /// Plain text, which is what typing produces.
    #[serde(default)]
    pub text: String,
    /// Standard base64, for the bytes a key produces that text cannot carry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<String>,
}

/// A new shape for the terminal window.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub struct TerminalSizeBody {
    pub cols: u16,
    pub rows: u16,
}

impl WorkbenchShellState {
    /// Keep the MCP servers a person declares in `root`, sharing the same
    /// declaration map the projects use so an attachment resolves the same way
    /// whatever kind of server it names.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory or any declaration in it is unusable,
    /// and when the catalogue is enabled twice.
    pub fn enable_mcp_catalogue(
        &self,
        root: &Path,
        declared: std::sync::Arc<std::sync::Mutex<BTreeMap<String, McpServer>>>,
    ) -> Result<(), WorkbenchShellError> {
        let catalogue = McpCatalogue::open(root, declared)?;
        self.mcp_catalogue
            .set(catalogue)
            .map_err(|_| WorkbenchShellError::Conflict("MCP catalogue is already enabled".into()))
    }

    fn catalogue(&self) -> Result<&McpCatalogue, WorkbenchShellError> {
        self.mcp_catalogue.get().ok_or_else(|| {
            WorkbenchShellError::NotFound(
                "this host keeps no MCP catalogue; its servers are declared where it was started"
                    .into(),
            )
        })
    }

    /// Every MCP server an agent here can attach, project and declared alike.
    ///
    /// # Errors
    ///
    /// Fails when no catalogue is enabled or it cannot be read.
    pub fn mcp_servers(&self) -> Result<Vec<McpServerView>, WorkbenchShellError> {
        self.catalogue()?.list()
    }

    /// Declare a server, or correct one already declared here.
    ///
    /// # Errors
    ///
    /// Refuses an unusable declaration and a name a project already holds.
    pub fn declare_mcp_server(
        &self,
        body: &DeclareMcpServerBody,
    ) -> Result<McpServerView, WorkbenchShellError> {
        self.catalogue()?.declare(body)
    }

    /// Forget a declared server, naming the profiles that would lose it.
    ///
    /// # Errors
    ///
    /// Refuses an unknown name, a project, and a server an agent still
    /// attaches - detaching is the person's decision, not a side effect.
    pub fn forget_mcp_server(&self, name: &str) -> Result<Value, WorkbenchShellError> {
        let attached: Vec<String> = self
            .profiles()?
            .into_iter()
            .filter(|profile| {
                profile
                    .attachments
                    .iter()
                    .any(|attachment| attachment.server_name == name)
            })
            .map(|profile| profile.profile_id)
            .collect();
        if !attached.is_empty() {
            return Err(WorkbenchShellError::Conflict(format!(
                "{name} is still attached by {}; detach it there first",
                attached.join(", ")
            )));
        }
        self.catalogue()?.forget(name)?;
        Ok(json!({ "forgotten": name }))
    }

    /// Change what one profile is, keeping its identity and its sessions.
    ///
    /// # Errors
    ///
    /// Not found for an unknown profile, conflict when the profile moved on
    /// under the editor, invalid for a directory that cannot be used or an
    /// attachment naming a server this host does not declare.
    pub fn amend_profile(
        &self,
        profile_id: &str,
        body: &AmendProfileBody,
    ) -> Result<PersonalAgentProfile, WorkbenchShellError> {
        let existing = self.inventory.load(profile_id).map_err(profile_error)?;
        let workspace = match &body.workspace {
            Some(path) => usable_directory("workspace", path)?,
            None => existing.workspace.clone(),
        };
        let agent_home = match &body.agent_home {
            Some(path) => usable_directory("agent home", path)?,
            None => existing.agent_home.clone(),
        };
        let attachments = match &body.attachments {
            Some(names) => {
                let declared = self.declared_servers()?;
                let mut bindings = Vec::with_capacity(names.len());
                for name in names {
                    // One the profile already holds stays as it is even when
                    // its server is not declared here any more: a person
                    // changing the others is not asked to repair this one
                    // first, and leaving its name out is how it is detached.
                    let Some(server) = declared.get(name.as_str()) else {
                        if let Some(held) = existing
                            .attachments
                            .iter()
                            .find(|attachment| &attachment.server_name == name)
                        {
                            bindings.push(held.clone());
                            continue;
                        }
                        return Err(WorkbenchShellError::Invalid(format!(
                            "this host declares no MCP server named {name}"
                        )));
                    };
                    let transport = match server {
                        McpServer::Stdio(_) => AttachmentTransport::Stdio,
                        McpServer::Http(_) => AttachmentTransport::Http,
                        McpServer::Sse(_) => AttachmentTransport::Sse,
                        other => {
                            return Err(WorkbenchShellError::Invalid(format!(
                                "this host cannot dial {name}: {other:?}"
                            )));
                        }
                    };
                    bindings.push(AttachmentBinding::new(
                        format!("{profile_id}-{name}"),
                        name.clone(),
                        transport,
                    ));
                }
                bindings
            }
            None => existing.attachments.clone(),
        };
        // A choice this host does not offer is refused here rather than at the
        // start of a session, so a person learns it while they are looking at
        // the form.
        let permissions = match &body.permissions {
            Some(chosen) => {
                crate::permission_policy(chosen, &workspace, Vec::new())
                    .map_err(WorkbenchShellError::Invalid)?;
                chosen.clone()
            }
            None => existing.permission_profile_id.clone(),
        };
        // Same rule as the permission choice: an environment this host does
        // not have is refused while the person is looking at the form, not at
        // the start of a session.
        let environment = match &body.environment {
            Some(chosen) => {
                crate::environment_backend(chosen).map_err(WorkbenchShellError::Invalid)?;
                chosen.clone()
            }
            None => existing.environment_profile_id.clone(),
        };
        let wanted = PersonalAgentProfile::new(
            existing.profile_id.clone(),
            existing.agent_id.clone(),
            existing.distribution_ref.clone(),
            environment,
            permissions,
            &workspace,
            &agent_home,
            attachments,
            existing.credential_bindings.clone(),
        )
        .map_err(profile_error)?;
        // The setup is replaced whole, like the attachments; a provider or a
        // model this host cannot vouch for is refused at the form.
        let setup = body.setup.clone().unwrap_or_else(|| existing.setup());
        self.check_setup(&setup)?;
        let mut wanted = wanted.with_setup(setup).map_err(profile_error)?;
        wanted.revision = body.revision;
        self.inventory.amend(&wanted).map_err(|error| match error {
            ProfileError::ProfileChanged { .. } => WorkbenchShellError::Conflict(error.to_string()),
            other => profile_error(other),
        })
    }

    /// The declarations this host can attach, by ACP name.
    fn declared_servers(&self) -> Result<BTreeMap<String, McpServer>, WorkbenchShellError> {
        let shared = self
            .mcp_catalogue
            .get()
            .map(McpCatalogue::declared)
            .ok_or_else(|| {
                WorkbenchShellError::NotFound("this host declares no MCP servers to attach".into())
            })?;
        let declared = shared
            .lock()
            .map_err(|_| WorkbenchShellError::Failed("declaration registry poisoned".into()))?
            .clone();
        Ok(declared)
    }

    /// Open a terminal in one profile's environment.
    ///
    /// # Errors
    ///
    /// Not found for an unknown profile; fails when the program cannot start.
    pub async fn open_terminal(
        &self,
        body: &OpenTerminalBody,
    ) -> Result<TerminalView, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(&body.profile_id)
            .map_err(profile_error)?;
        let secrets = self.keys_of(&profile)?;
        self.terminals
            .open(&profile, secrets, body, TerminalOpener::Person)
            .await
    }

    /// Every open terminal.
    pub async fn terminals(&self) -> Vec<TerminalView> {
        self.terminals.list().await
    }

    /// The output of one terminal from `after`, waiting for the next byte.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal.
    pub async fn terminal_output(
        &self,
        terminal_id: &str,
        after: u64,
        wait: Duration,
    ) -> Result<TerminalOutput, WorkbenchShellError> {
        self.terminals.output(terminal_id, after, wait).await
    }

    /// Send exactly what a person typed.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal, invalid for malformed bytes,
    /// conflict once the process ended.
    pub async fn terminal_input(
        &self,
        terminal_id: &str,
        body: &TerminalInputBody,
    ) -> Result<Value, WorkbenchShellError> {
        let bytes = match &body.bytes {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|error| WorkbenchShellError::Invalid(error.to_string()))?,
            None => body.text.clone().into_bytes(),
        };
        self.terminals.input(terminal_id, &bytes).await?;
        Ok(json!({ "sent": bytes.len() }))
    }

    /// Tell the process its window changed shape.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal.
    pub async fn resize_terminal(
        &self,
        terminal_id: &str,
        size: TerminalSizeBody,
    ) -> Result<TerminalView, WorkbenchShellError> {
        self.terminals
            .resize(terminal_id, size.cols, size.rows)
            .await
    }

    /// End one terminal.
    ///
    /// # Errors
    ///
    /// Not found for an unknown terminal.
    pub async fn close_terminal(&self, terminal_id: &str) -> Result<Value, WorkbenchShellError> {
        self.terminals.close(terminal_id).await?;
        Ok(json!({ "closed": terminal_id }))
    }

    /// End every terminal this host has open.
    pub async fn close_terminals(&self) {
        self.terminals.close_all().await;
    }
}

/// A directory a person named: absolute, and made if it is not there yet.
fn usable_directory(field: &str, path: &str) -> Result<PathBuf, WorkbenchShellError> {
    let path = PathBuf::from(path.trim());
    if !path.is_absolute() {
        return Err(WorkbenchShellError::Invalid(format!(
            "the {field} needs a full path, starting at /"
        )));
    }
    std::fs::create_dir_all(&path).map_err(|error| {
        WorkbenchShellError::Invalid(format!("{field} {}: {error}", path.display()))
    })?;
    std::fs::canonicalize(&path)
        .map_err(|error| WorkbenchShellError::Invalid(format!("{field}: {error}")))
}
