//! Minimal Cycle-independent ACP fixture.
//!
//! It deliberately does not import `swem-core`, `swem-runtime`, or the host
//! crate. Besides lifecycle/recovery, one explicit fixture prompt uses the
//! official MCP SDK to prove that a caller-supplied attachment is executable.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio as ProcessStdio;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{
    AgentCapabilities, AuthEnvVar, AuthMethod, AuthMethodEnvVar, AuthenticateRequest,
    AuthenticateResponse, CancelNotification, CloseSessionRequest, CloseSessionResponse,
    CompleteElicitationNotification, ConfigOptionUpdate, ContentBlock, ContentChunk,
    CreateElicitationRequest, CurrentModeUpdate, ElicitationAction, ElicitationFormMode,
    ElicitationRequestScope, ElicitationSchema, ElicitationSessionScope, ElicitationUrlMode,
    EmbeddedResourceResource, EnumOption, InitializeRequest, InitializeResponse,
    ListSessionsRequest, ListSessionsResponse, LoadSessionRequest, LoadSessionResponse, McpServer,
    MultiSelectPropertySchema, NewSessionRequest, NewSessionResponse, PermissionOption,
    PermissionOptionKind, PromptCapabilities, PromptRequest, PromptResponse, RequestId,
    RequestPermissionOutcome, RequestPermissionRequest, ResourceLink, ResumeSessionRequest,
    ResumeSessionResponse, SessionCapabilities, SessionCloseCapabilities, SessionConfigOption,
    SessionConfigOptionCategory, SessionConfigOptionValue, SessionConfigSelectOption, SessionId,
    SessionInfo, SessionListCapabilities, SessionMode, SessionModeId, SessionModeState,
    SessionNotification, SessionResumeCapabilities, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse, SetSessionModeRequest, SetSessionModeResponse, StopReason,
    StringPropertySchema, TextContent, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate,
    ToolCallUpdateFields, ToolKind,
};
use agent_client_protocol::{Agent, Client, ConnectionTo, Error, Stdio};
use base64::Engine as _;
use rmcp::ServiceExt as _;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities, ClientInfo,
    ClientRequest, GetTaskParams, Implementation, JsonObject, Request, TaskPayload,
};
use rmcp::service::PeerRequestOptions;
use rmcp::transport::TokioChildProcess;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::watch;
use url::Url;

const STATE_FILE: &str = ".swem-echo-agent-session.json";
/// One file per session, beside the pointer to the newest one. A real agent
/// keeps every session it was asked to keep; this fixture used to hold exactly
/// one per workspace, so a second session silently destroyed the first and no
/// gate noticed until a person tried to go back to one.
const SESSIONS_DIR: &str = ".swem-echo-agent-sessions";
const CONTENT_OBSERVATION_FILE: &str = ".swem-content-observation.json";
const CONTENT_MATRIX_MARKER: &str = "SWEM_CONTENT_MATRIX";
const WORKSPACE_LINK_MARKER: &str = "SWEM_WORKSPACE_LINK";
const WORKSPACE_LINK_BYTES: &[u8] = b"\0SWEM_LINKED_OUTPUT\xff\x10\x80\x7f";
const PARTIAL_ARTIFACT_FAILURE_MARKER: &str = "SWEM_PARTIAL_ARTIFACT_FAILURE";
const PARTIAL_MESSAGE_BYTES: &[u8] = b"\0SWEM_PARTIAL_MESSAGE\xff\x11";
const PARTIAL_TOOL_BYTES: &[u8] = b"\0SWEM_PARTIAL_TOOL\xfe\x12";
const CONFIG_UPDATE_MARKER: &str = "SWEM_CONFIG_UPDATE";

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EchoConfiguration {
    mode: String,
    model: String,
    thought_level: String,
    brave: bool,
}

impl Default for EchoConfiguration {
    fn default() -> Self {
        Self {
            mode: "ask".into(),
            model: "fast".into(),
            thought_level: "low".into(),
            brave: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EchoSession {
    id: SessionId,
    mcp_names: Vec<String>,
    /// Attachment launch data belongs to the active ACP connection. In
    /// particular, MCP env values must never enter durable agent state.
    #[serde(skip, default)]
    mcp_servers: Vec<McpServer>,
    turn: usize,
    cwd: PathBuf,
    history: Vec<EchoTurn>,
    #[serde(default)]
    configuration: EchoConfiguration,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct EchoTurn {
    prompt: String,
    reply: String,
}

#[derive(Clone, Debug, Default)]
struct EchoState {
    /// Set once `authenticate` succeeded; `session/new` is refused until then
    /// when `--require-auth` names a variable.
    authenticated: Arc<Mutex<bool>>,
    session: Arc<Mutex<Option<EchoSession>>>,
    client_boolean_config: Arc<Mutex<bool>>,
    client_form_elicitation: Arc<Mutex<bool>>,
    client_url_elicitation: Arc<Mutex<bool>>,
    active_cancellations: Arc<Mutex<BTreeMap<String, watch::Sender<bool>>>>,
    pending_cancellations: Arc<Mutex<BTreeSet<String>>>,
}

#[derive(Clone, Copy)]
enum CancelFixture {
    Confirm,
    CompleteRace,
    ProtocolError,
    Ignore,
}

enum EchoPermissionOutcome {
    Allowed(ToolCallId),
    Rejected(Value),
    Cancelled,
}

#[derive(Clone, Copy)]
struct ConfigFixture {
    surface: ConfigSurface,
    boolean_option: BooleanOption,
    config_ids: ConfigIds,
}

#[derive(Clone, Copy)]
enum ConfigSurface {
    ConfigOptions,
    LegacyModes,
}

#[derive(Clone, Copy)]
enum BooleanOption {
    Negotiated(bool),
    ForceAdvertise,
}

#[derive(Clone, Copy)]
enum ConfigIds {
    Unique,
    Duplicate,
}

#[tokio::main]
#[allow(
    clippy::too_many_lines,
    reason = "linear fixture protocol keeps initialize/new/load/resume/prompt behavior visible in one place"
)]
async fn main() -> Result<(), Error> {
    let arguments = std::env::args().collect::<Vec<_>>();
    if arguments
        .get(1)
        .is_some_and(|argument| argument == "--swem-descendant")
    {
        let marker = arguments
            .get(2)
            .ok_or_else(|| internal("descendant marker path is missing"))?;
        run_descendant(Path::new(marker));
    }
    if let Some(index) = arguments
        .iter()
        .position(|argument| argument == "--spawn-descendant")
    {
        let marker = arguments
            .get(index + 1)
            .ok_or_else(|| internal("descendant marker path is missing"))?;
        let executable = std::env::current_exe()
            .map_err(|error| internal(format!("resolve fixture executable: {error}")))?;
        let child = std::process::Command::new(executable)
            .arg("--swem-descendant")
            .arg(marker)
            .stdin(ProcessStdio::null())
            .stdout(ProcessStdio::null())
            .stderr(ProcessStdio::null())
            .spawn()
            .map_err(|error| internal(format!("spawn fixture descendant: {error}")))?;
        fs::write(format!("{marker}.pid"), child.id().to_string())
            .map_err(|error| internal(format!("write descendant pid: {error}")))?;
    }
    let recovery_advertised = !arguments
        .iter()
        .any(|argument| argument == "--hide-recovery");
    let rich_content_advertised = !arguments
        .iter()
        .any(|argument| argument == "--baseline-content-only");
    let legacy_modes_only = arguments
        .iter()
        .any(|argument| argument == "--legacy-modes-only");
    let violate_boolean_capability = arguments
        .iter()
        .any(|argument| argument == "--violate-boolean-capability");
    let duplicate_config_id = arguments
        .iter()
        .any(|argument| argument == "--duplicate-config-id");
    let close_advertised = !arguments.iter().any(|argument| argument == "--hide-close");
    let initialize_elicitation_receipt = arguments
        .iter()
        .position(|argument| argument == "--initialize-elicitation-receipt")
        .and_then(|index| arguments.get(index + 1))
        .map(PathBuf::from);
    // An agent that needs a key. It advertises one `env_var` authentication
    // method naming the variable, refuses `session/new` until `authenticate`
    // has been called, and `authenticate` itself succeeds only when the
    // variable reached THIS process non-empty - which is how a test proves
    // the key a person typed in the Workbench arrived where the agent reads
    // it, and never merely that a button was pressed.
    // An agent that says no at the door, in its own words. A refusal here is
    // the one failure the host cannot narrate for itself - the reason belongs
    // to the agent - so a fixture that can produce one on demand is what lets
    // a test read the sentence a person actually gets.
    let refuse_initialize = arguments
        .iter()
        .position(|argument| argument == "--refuse-initialize")
        .and_then(|index| arguments.get(index + 1))
        .cloned();
    let required_auth_variable = arguments
        .iter()
        .position(|argument| argument == "--require-auth")
        .and_then(|index| arguments.get(index + 1))
        .cloned();
    let state = EchoState::default();
    let initialize_writer = state.clone();
    let authenticate_writer = state.clone();
    let auth_variable_for_initialize = required_auth_variable.clone();
    let auth_variable_for_authenticate = required_auth_variable.clone();
    let auth_required_for_new = required_auth_variable.is_some();
    let new_session_writer = state.clone();
    let list_session_reader = state.clone();
    let load_session_writer = state.clone();
    let resume_session_writer = state.clone();
    let config_session_writer = state.clone();
    let mode_session_writer = state.clone();
    let close_session_writer = state.clone();
    let cancellation_writer = state.clone();
    let session_reader = state;
    Agent
        .builder()
        .name("swem-echo-agent")
        .on_receive_request(
            async move |request: InitializeRequest, responder, connection: ConnectionTo<Client>| {
                if let Some(reason) = refuse_initialize.clone() {
                    return Err(internal(reason));
                }
                let boolean_supported = request
                    .client_capabilities
                    .session
                    .as_ref()
                    .and_then(|session| session.config_options.as_ref())
                    .and_then(|options| options.boolean.as_ref())
                    .is_some();
                *initialize_writer
                    .client_boolean_config
                    .lock()
                    .map_err(|_| internal("fixture capability lock poisoned"))? = boolean_supported;
                let elicitation = request.client_capabilities.elicitation.as_ref();
                *initialize_writer
                    .client_form_elicitation
                    .lock()
                    .map_err(|_| internal("fixture capability lock poisoned"))? = elicitation
                    .and_then(|capabilities| capabilities.form.as_ref())
                    .is_some();
                *initialize_writer
                    .client_url_elicitation
                    .lock()
                    .map_err(|_| internal("fixture capability lock poisoned"))? = elicitation
                    .and_then(|capabilities| capabilities.url.as_ref())
                    .is_some();
                let mut session_capabilities = SessionCapabilities::new();
                if close_advertised {
                    session_capabilities =
                        session_capabilities.close(SessionCloseCapabilities::new());
                }
                if recovery_advertised {
                    session_capabilities = session_capabilities
                        .list(SessionListCapabilities::new())
                        .resume(SessionResumeCapabilities::new());
                }
                let mut capabilities = AgentCapabilities::new()
                    .load_session(recovery_advertised)
                    .session_capabilities(session_capabilities);
                if rich_content_advertised {
                    capabilities = capabilities.prompt_capabilities(
                        PromptCapabilities::new()
                            .image(true)
                            .audio(true)
                            .embedded_context(true),
                    );
                }
                let mut response = InitializeResponse::new(request.protocol_version)
                    .agent_capabilities(capabilities);
                if let Some(variable) = auth_variable_for_initialize.clone() {
                    response = response.auth_methods(vec![AuthMethod::EnvVar(
                        AuthMethodEnvVar::new(
                            "api-key",
                            "API key",
                            vec![AuthEnvVar::new(variable).label("Fixture API key")],
                        )
                        .description("A key this fixture reads from its environment."),
                    )]);
                }
                if let Some(receipt) = initialize_elicitation_receipt.clone() {
                    if !*initialize_writer
                        .client_form_elicitation
                        .lock()
                        .map_err(|_| internal("fixture capability lock poisoned"))?
                    {
                        return Err(internal(
                            "fixture client did not advertise initialize form elicitation",
                        ));
                    }
                    let background = connection.clone();
                    connection.spawn(async move {
                        let request = CreateElicitationRequest::new(
                            ElicitationFormMode::new(
                                ElicitationRequestScope::new(RequestId::Str(
                                    "initialize-fixture".into(),
                                )),
                                ElicitationSchema::new().string("workspace_label", true),
                            ),
                            "Name this connection before the native session is created.",
                        );
                        let answer = background.send_request(request).block_task().await?;
                        let encoded = serde_json::to_vec(&answer).map_err(|error| {
                            internal(format!("encode initialize elicitation response: {error}"))
                        })?;
                        fs::write(receipt, encoded).map_err(|error| {
                            internal(format!("write initialize elicitation receipt: {error}"))
                        })?;
                        responder.respond(response)
                    })?;
                    return Ok(());
                }
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: AuthenticateRequest, responder, _connection| {
                let Some(variable) = auth_variable_for_authenticate.clone() else {
                    return Err(internal("fixture advertises no authentication method"));
                };
                if request.method_id.0.as_ref() != "api-key" {
                    return Err(Error::invalid_params());
                }
                if !std::env::var(&variable).is_ok_and(|value| !value.is_empty()) {
                    return Err(Error::auth_required());
                }
                *authenticate_writer
                    .authenticated
                    .lock()
                    .map_err(|_| internal("fixture auth lock poisoned"))? = true;
                responder.respond(AuthenticateResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ListSessionsRequest, responder, _connection| {
                // Every session of that workspace, not just the last one: a
                // person who opened three of them must be able to reconnect to
                // any of the three.
                let sessions = match request.cwd.as_deref() {
                    Some(cwd) => sessions_in(cwd)?,
                    None => list_session_reader
                        .session
                        .lock()
                        .map_err(|_| internal("fixture session lock poisoned"))?
                        .clone()
                        .into_iter()
                        .collect(),
                };
                let sessions = sessions
                    .into_iter()
                    .filter(|session| {
                        request
                            .cwd
                            .as_ref()
                            .is_none_or(|requested| requested == &session.cwd)
                    })
                    .map(|session| SessionInfo::new(session.id, session.cwd))
                    .collect();
                responder.respond(ListSessionsResponse::new(sessions))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                if !request.cwd.is_absolute() {
                    return Err(internal("fixture cwd is not absolute"));
                }
                if auth_required_for_new
                    && !*new_session_writer
                        .authenticated
                        .lock()
                        .map_err(|_| internal("fixture auth lock poisoned"))?
                {
                    return Err(Error::auth_required());
                }
                let id = SessionId::new(format!(
                    "echo-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| internal("fixture clock before epoch"))?
                        .as_nanos()
                ));
                let mcp_names = request.mcp_servers.iter().map(mcp_name).collect();
                let session = EchoSession {
                    id: id.clone(),
                    mcp_names,
                    mcp_servers: request.mcp_servers,
                    turn: 0,
                    cwd: request.cwd,
                    history: Vec::new(),
                    configuration: EchoConfiguration::default(),
                };
                let response = session_response(
                    NewSessionResponse::new(id),
                    &session.configuration,
                    ConfigFixture {
                        surface: if legacy_modes_only {
                            ConfigSurface::LegacyModes
                        } else {
                            ConfigSurface::ConfigOptions
                        },
                        boolean_option: if violate_boolean_capability {
                            BooleanOption::ForceAdvertise
                        } else {
                            BooleanOption::Negotiated(boolean_capability(&new_session_writer)?)
                        },
                        config_ids: if duplicate_config_id {
                            ConfigIds::Duplicate
                        } else {
                            ConfigIds::Unique
                        },
                    },
                );
                persist(&session)?;
                *new_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))? = Some(session);
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: LoadSessionRequest,
                        responder,
                        connection: ConnectionTo<Client>| {
                let session = restore(&request.cwd, &request.session_id, request.mcp_servers)?;
                for turn in &session.history {
                    send_chunk(
                        &connection,
                        session.id.clone(),
                        SessionUpdate::UserMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(turn.prompt.clone()),
                        ))),
                    )?;
                    send_chunk(
                        &connection,
                        session.id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new(turn.reply.clone()),
                        ))),
                    )?;
                }
                *load_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))? = Some(session);
                let session = load_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?
                    .clone()
                    .ok_or_else(|| internal("fixture load state disappeared"))?;
                responder.respond(load_response(
                    &session.configuration,
                    ConfigFixture {
                        surface: if legacy_modes_only {
                            ConfigSurface::LegacyModes
                        } else {
                            ConfigSurface::ConfigOptions
                        },
                        boolean_option: if violate_boolean_capability {
                            BooleanOption::ForceAdvertise
                        } else {
                            BooleanOption::Negotiated(boolean_capability(&load_session_writer)?)
                        },
                        config_ids: if duplicate_config_id {
                            ConfigIds::Duplicate
                        } else {
                            ConfigIds::Unique
                        },
                    },
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: ResumeSessionRequest, responder, _connection| {
                let session = restore(&request.cwd, &request.session_id, request.mcp_servers)?;
                *resume_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))? = Some(session);
                let session = resume_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?
                    .clone()
                    .ok_or_else(|| internal("fixture resume state disappeared"))?;
                responder.respond(resume_response(
                    &session.configuration,
                    ConfigFixture {
                        surface: if legacy_modes_only {
                            ConfigSurface::LegacyModes
                        } else {
                            ConfigSurface::ConfigOptions
                        },
                        boolean_option: if violate_boolean_capability {
                            BooleanOption::ForceAdvertise
                        } else {
                            BooleanOption::Negotiated(boolean_capability(&resume_session_writer)?)
                        },
                        config_ids: if duplicate_config_id {
                            ConfigIds::Duplicate
                        } else {
                            ConfigIds::Unique
                        },
                    },
                ))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionConfigOptionRequest, responder, _connection| {
                let boolean_supported = boolean_capability(&config_session_writer)?;
                let mut guard = config_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?;
                let session = guard
                    .as_mut()
                    .ok_or_else(|| internal("config option before session/new"))?;
                if request.session_id != session.id {
                    return Err(internal("config option session id does not match"));
                }
                apply_echo_config(
                    &mut session.configuration,
                    request.config_id.0.as_ref(),
                    &request.value,
                    boolean_supported,
                )?;
                persist(session)?;
                let options = echo_config_options(
                    &session.configuration,
                    boolean_supported || violate_boolean_capability,
                    duplicate_config_id,
                );
                responder.respond(SetSessionConfigOptionResponse::new(options))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: SetSessionModeRequest,
                        responder,
                        connection: ConnectionTo<Client>| {
                let mut guard = mode_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?;
                let session = guard
                    .as_mut()
                    .ok_or_else(|| internal("set mode before session/new"))?;
                if request.session_id != session.id {
                    return Err(internal("set mode session id does not match"));
                }
                if !matches!(request.mode_id.0.as_ref(), "ask" | "code") {
                    return Err(internal("unknown fixture legacy mode"));
                }
                session.configuration.mode = request.mode_id.0.to_string();
                persist(session)?;
                connection.send_notification(SessionNotification::new(
                    request.session_id,
                    SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(request.mode_id)),
                ))?;
                responder.respond(SetSessionModeResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: CloseSessionRequest, responder, _connection| {
                let mut session = close_session_writer
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?;
                let active = session
                    .as_ref()
                    .ok_or_else(|| internal("close before session/new"))?;
                if request.session_id != active.id {
                    return Err(internal("close session id does not match"));
                }
                *session = None;
                responder.respond(CloseSessionResponse::new())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection: ConnectionTo<Client>| {
                let prompt = request
                    .prompt
                    .iter()
                    .find_map(|block| match block {
                        ContentBlock::Text(text) => Some(text.text.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| internal("fixture prompt has no text"))?;
                let snapshot = session_reader
                    .session
                    .lock()
                    .map_err(|_| internal("fixture session lock poisoned"))?
                    .clone()
                    .ok_or_else(|| internal("prompt before session/new"))?;
                if request.session_id != snapshot.id {
                    return Err(internal("prompt session id does not match"));
                }
                if prompt.contains(PARTIAL_ARTIFACT_FAILURE_MARKER) {
                    let message_path = snapshot.cwd.join("partial-message-output.bin");
                    let tool_path = snapshot.cwd.join("partial-tool-output.bin");
                    fs::write(&message_path, PARTIAL_MESSAGE_BYTES)
                        .map_err(|error| internal(format!("write partial message: {error}")))?;
                    fs::write(&tool_path, PARTIAL_TOOL_BYTES)
                        .map_err(|error| internal(format!("write partial tool output: {error}")))?;
                    let message_uri = Url::from_file_path(&message_path)
                        .map_err(|()| internal("partial message path is not a file URI"))?
                        .to_string();
                    let tool_uri = Url::from_file_path(&tool_path)
                        .map_err(|()| internal("partial tool path is not a file URI"))?
                        .to_string();
                    send_chunk(
                        &connection,
                        request.session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(
                            ContentBlock::ResourceLink(
                                ResourceLink::new("partial-message-output.bin", message_uri)
                                    .mime_type("application/octet-stream")
                                    .size(i64::try_from(PARTIAL_MESSAGE_BYTES.len()).map_err(
                                        |error| internal(format!("partial message size: {error}")),
                                    )?),
                            ),
                        )),
                    )?;
                    send_chunk(
                        &connection,
                        request.session_id.clone(),
                        SessionUpdate::ToolCall(
                            ToolCall::new("partial-tool-call", "Produce partial tool output")
                                .kind(ToolKind::Execute)
                                .status(ToolCallStatus::Completed)
                                .content(vec![
                                    ContentBlock::ResourceLink(
                                        ResourceLink::new("partial-tool-output.bin", tool_uri)
                                            .mime_type("application/octet-stream")
                                            .size(
                                                i64::try_from(PARTIAL_TOOL_BYTES.len()).map_err(
                                                    |error| {
                                                        internal(format!(
                                                            "partial tool size: {error}"
                                                        ))
                                                    },
                                                )?,
                                            ),
                                    )
                                    .into(),
                                ]),
                        ),
                    )?;
                    return Err(internal("intentional prompt failure after partial output"));
                }
                if prompt.contains(WORKSPACE_LINK_MARKER) {
                    let linked_path = snapshot.cwd.join("linked-output.bin");
                    fs::write(&linked_path, WORKSPACE_LINK_BYTES)
                        .map_err(|error| internal(format!("write linked output: {error}")))?;
                    let outside_path = snapshot
                        .cwd
                        .parent()
                        .ok_or_else(|| internal("fixture workspace has no parent"))?
                        .join("outside-linked-output.bin");
                    fs::write(&outside_path, b"OUTSIDE_WORKSPACE_MUST_NOT_BE_ADOPTED")
                        .map_err(|error| internal(format!("write outside output: {error}")))?;
                    let linked_uri = Url::from_file_path(&linked_path)
                        .map_err(|()| internal("linked output path is not a file URI"))?
                        .to_string();
                    let outside_uri = Url::from_file_path(&outside_path)
                        .map_err(|()| internal("outside output path is not a file URI"))?
                        .to_string();
                    for block in [
                        ContentBlock::Text(TextContent::new(WORKSPACE_LINK_MARKER)),
                        ContentBlock::ResourceLink(
                            ResourceLink::new("linked-output.bin", linked_uri)
                                .mime_type("application/octet-stream")
                                .size(i64::try_from(WORKSPACE_LINK_BYTES.len()).map_err(
                                    |error| {
                                        internal(format!("linked output size conversion: {error}"))
                                    },
                                )?),
                        ),
                        ContentBlock::ResourceLink(
                            ResourceLink::new("outside-linked-output.bin", outside_uri)
                                .mime_type("application/octet-stream"),
                        ),
                        ContentBlock::ResourceLink(
                            ResourceLink::new(
                                "remote-output.bin",
                                "https://example.invalid/remote-output.bin",
                            )
                            .mime_type("application/octet-stream"),
                        ),
                    ] {
                        send_chunk(
                            &connection,
                            request.session_id.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(block)),
                        )?;
                    }
                    {
                        let mut guard = session_reader
                            .session
                            .lock()
                            .map_err(|_| internal("fixture session lock poisoned"))?;
                        let session = guard
                            .as_mut()
                            .ok_or_else(|| internal("prompt before session/new"))?;
                        session.turn += 1;
                        session.history.push(EchoTurn {
                            prompt,
                            reply: WORKSPACE_LINK_MARKER.into(),
                        });
                        persist(session)?;
                    }
                    return responder.respond(PromptResponse::new(StopReason::EndTurn));
                }
                if prompt.contains(CONTENT_MATRIX_MARKER) {
                    observe_content_matrix(&snapshot.cwd, &request.prompt)?;
                    for block in request.prompt.clone() {
                        send_chunk(
                            &connection,
                            request.session_id.clone(),
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(block)),
                        )?;
                    }
                    {
                        let mut guard = session_reader
                            .session
                            .lock()
                            .map_err(|_| internal("fixture session lock poisoned"))?;
                        let session = guard
                            .as_mut()
                            .ok_or_else(|| internal("prompt before session/new"))?;
                        session.turn += 1;
                        session.history.push(EchoTurn {
                            prompt,
                            reply: CONTENT_MATRIX_MARKER.into(),
                        });
                        persist(session)?;
                    }
                    return responder.respond(PromptResponse::new(StopReason::EndTurn));
                }
                if prompt.contains(CONFIG_UPDATE_MARKER) {
                    let boolean_supported = boolean_capability(&session_reader)?;
                    let options = {
                        let mut guard = session_reader
                            .session
                            .lock()
                            .map_err(|_| internal("fixture session lock poisoned"))?;
                        let session = guard
                            .as_mut()
                            .ok_or_else(|| internal("prompt before session/new"))?;
                        session.configuration.mode = "code".into();
                        persist(session)?;
                        echo_config_options(
                            &session.configuration,
                            boolean_supported || violate_boolean_capability,
                            duplicate_config_id,
                        )
                    };
                    if !legacy_modes_only {
                        send_chunk(
                            &connection,
                            request.session_id.clone(),
                            SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(options)),
                        )?;
                    }
                    send_chunk(
                        &connection,
                        request.session_id.clone(),
                        SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new("code")),
                    )?;
                }
                if let Some(mode) = elicitation_fixture(&prompt) {
                    let form_supported = *session_reader
                        .client_form_elicitation
                        .lock()
                        .map_err(|_| internal("fixture capability lock poisoned"))?;
                    let url_supported = *session_reader
                        .client_url_elicitation
                        .lock()
                        .map_err(|_| internal("fixture capability lock poisoned"))?;
                    if matches!(mode, FixtureElicitation::Form) && !form_supported
                        || matches!(mode, FixtureElicitation::Url) && !url_supported
                    {
                        return Err(internal(
                            "fixture client did not advertise elicitation mode",
                        ));
                    }
                    let background = connection.clone();
                    let background_state = session_reader.clone();
                    let session_id = request.session_id.clone();
                    let receipt = snapshot.cwd.join(match mode {
                        FixtureElicitation::Form => ".swem-form-elicitation-receipt.json",
                        FixtureElicitation::Url => ".swem-url-elicitation-receipt.json",
                    });
                    connection.spawn(async move {
                        let elicitation = fixture_elicitation_request(mode, session_id.clone());
                        let response = background.send_request(elicitation).block_task().await?;
                        let raw_response = serde_json::to_vec(&response).map_err(|error| {
                            internal(format!("encode elicitation response: {error}"))
                        })?;
                        fs::write(receipt, raw_response).map_err(|error| {
                            internal(format!("write elicitation receipt: {error}"))
                        })?;
                        let action = match response.action {
                            ElicitationAction::Accept(_) => "accept",
                            ElicitationAction::Decline => "decline",
                            ElicitationAction::Cancel => "cancel",
                            ElicitationAction::Other(_) => "other",
                            _ => "unknown",
                        };
                        let complete_url =
                            matches!(mode, FixtureElicitation::Url) && action == "accept";
                        let reply = serde_json::json!({"action": action}).to_string();
                        {
                            let mut guard = background_state
                                .session
                                .lock()
                                .map_err(|_| internal("fixture session lock poisoned"))?;
                            let session = guard
                                .as_mut()
                                .ok_or_else(|| internal("prompt before session/new"))?;
                            session.turn += 1;
                            session.history.push(EchoTurn {
                                prompt,
                                reply: reply.clone(),
                            });
                            persist(session)?;
                        }
                        send_chunk(
                            &background,
                            session_id,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(reply)),
                            )),
                        )?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))?;
                        if complete_url {
                            // Exercise the protocol's asynchronous lifecycle:
                            // completion is connection-bound and can arrive
                            // after the originating prompt has already ended.
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            background.send_notification(CompleteElicitationNotification::new(
                                "fixture-oauth-1",
                            ))?;
                        }
                        Ok(())
                    })?;
                    return Ok(());
                }
                if let Some(fixture) = cancel_fixture(&prompt) {
                    let key = request.session_id.0.to_string();
                    let (cancel_tx, mut cancel_rx) = watch::channel(false);
                    session_reader
                        .active_cancellations
                        .lock()
                        .map_err(|_| internal("fixture cancellation lock poisoned"))?
                        .insert(key.clone(), cancel_tx.clone());
                    let was_pending = session_reader
                        .pending_cancellations
                        .lock()
                        .map_err(|_| internal("fixture cancellation lock poisoned"))?
                        .remove(&key);
                    if was_pending {
                        cancel_tx.send_replace(true);
                    }
                    let cancellation_state = session_reader.clone();
                    connection.spawn(async move {
                        if !*cancel_rx.borrow() {
                            cancel_rx
                                .changed()
                                .await
                                .map_err(|_| internal("fixture cancellation signal closed"))?;
                        }
                        cancellation_state
                            .active_cancellations
                            .lock()
                            .map_err(|_| internal("fixture cancellation lock poisoned"))?
                            .remove(&key);
                        cancellation_state
                            .pending_cancellations
                            .lock()
                            .map_err(|_| internal("fixture cancellation lock poisoned"))?
                            .remove(&key);
                        match fixture {
                            CancelFixture::Confirm => {
                                responder.respond(PromptResponse::new(StopReason::Cancelled))
                            }
                            CancelFixture::CompleteRace => {
                                responder.respond(PromptResponse::new(StopReason::EndTurn))
                            }
                            CancelFixture::ProtocolError => responder
                                .respond_with_internal_error("fixture violated ACP cancellation"),
                            CancelFixture::Ignore => {
                                std::future::pending::<()>().await;
                                Ok(())
                            }
                        }
                    })?;
                    return Ok(());
                }
                if serde_json::from_str::<Value>(&prompt)
                    .ok()
                    .is_some_and(|value| {
                        matches!(
                            value["fixture"].as_str(),
                            Some("mcp-echo-permission-v0.1" | "mcp-call-permission-v0.1")
                        )
                    })
                {
                    // A prompt that issues a client request must not await that
                    // response inside the JSON-RPC request dispatcher. Keep the
                    // dispatcher free to receive the permission response.
                    let background = connection.clone();
                    let background_state = session_reader.clone();
                    let session_id = request.session_id.clone();
                    connection.spawn(async move {
                        let (mcp_result, permission_cancelled) =
                            call_requested_mcp(&background, &session_id, &snapshot, &prompt)
                                .await?;
                        if permission_cancelled {
                            return responder.respond(PromptResponse::new(StopReason::Cancelled));
                        }
                        let reply = {
                            let mut guard = background_state
                                .session
                                .lock()
                                .map_err(|_| internal("fixture session lock poisoned"))?;
                            let session = guard
                                .as_mut()
                                .ok_or_else(|| internal("prompt before session/new"))?;
                            session.turn += 1;
                            let reply = serde_json::json!({
                                "session_id": session.id.0.as_ref(),
                                "turn": session.turn,
                                "prompt": prompt,
                                "mcp_names": session.mcp_names,
                                "mcp_result": mcp_result,
                                "environment": environment_observation(),
                            })
                            .to_string();
                            session.history.push(EchoTurn {
                                prompt,
                                reply: reply.clone(),
                            });
                            persist(session)?;
                            reply
                        };
                        send_chunk(
                            &background,
                            session_id,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                ContentBlock::Text(TextContent::new(reply)),
                            )),
                        )?;
                        responder.respond(PromptResponse::new(StopReason::EndTurn))
                    })?;
                    return Ok(());
                }
                let (mcp_result, permission_cancelled) =
                    call_requested_mcp(&connection, &request.session_id, &snapshot, &prompt)
                        .await?;
                if permission_cancelled {
                    return responder.respond(PromptResponse::new(StopReason::Cancelled));
                }
                let reply = {
                    let mut guard = session_reader
                        .session
                        .lock()
                        .map_err(|_| internal("fixture session lock poisoned"))?;
                    let session = guard
                        .as_mut()
                        .ok_or_else(|| internal("prompt before session/new"))?;
                    session.turn += 1;
                    let reply = serde_json::json!({
                        "session_id": session.id.0.as_ref(),
                        "turn": session.turn,
                        "prompt": prompt,
                        "mcp_names": session.mcp_names,
                        "mcp_result": mcp_result,
                        "environment": environment_observation(),
                        // What the host set through `session/set_config_option`,
                        // as this fixture now holds it - so a test can see a
                        // profile's model arrive by the native road.
                        "configuration": {
                            "mode": session.configuration.mode,
                            "model": session.configuration.model,
                            "thought_level": session.configuration.thought_level,
                        },
                    })
                    .to_string();
                    session.history.push(EchoTurn {
                        prompt,
                        reply: reply.clone(),
                    });
                    persist(session)?;
                    reply
                };
                send_chunk(
                    &connection,
                    request.session_id,
                    SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                        TextContent::new(reply),
                    ))),
                )?;
                responder.respond(PromptResponse::new(StopReason::EndTurn))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |notification: CancelNotification, _connection| {
                let key = notification.session_id.0.to_string();
                let active = cancellation_writer
                    .active_cancellations
                    .lock()
                    .map_err(|_| internal("fixture cancellation lock poisoned"))?
                    .get(&key)
                    .cloned();
                if let Some(cancel) = active {
                    cancel.send_replace(true);
                } else {
                    cancellation_writer
                        .pending_cancellations
                        .lock()
                        .map_err(|_| internal("fixture cancellation lock poisoned"))?
                        .insert(key);
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
}

fn environment_observation() -> Value {
    let probe_name = std::env::var("SWEM_PROBE_VARIABLE_NAME").ok();
    // The variables a profile's model and provider arrive under, by name,
    // when they are set: what the host put into this process, seen from
    // inside it. Never a key.
    let model_environment: serde_json::Map<String, Value> = [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_BASE_URL",
        "OPENCODE_MODEL",
        "OPENAI_BASE_URL",
        "GEMINI_MODEL",
    ]
    .into_iter()
    .filter_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| (name.to_owned(), Value::String(value)))
    })
    .collect();
    serde_json::json!({
        "cwd": std::env::current_dir().ok().map(|path| path.display().to_string()),
        "explicit_value": std::env::var("SWEM_ALLOWED_TEST").ok(),
        "ambient_probe_name": probe_name,
        "ambient_probe_present": std::env::var("SWEM_PROBE_VARIABLE_NAME")
            .ok()
            .is_some_and(|name| std::env::var_os(name).is_some()),
        "model_environment": model_environment,
    })
}

fn run_descendant(marker: &Path) -> ! {
    let mut counter = 0_u64;
    loop {
        counter = counter.wrapping_add(1);
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(marker)
        {
            let _ = write!(file, "{counter}");
            let _ = file.flush();
        }
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
}

fn boolean_capability(state: &EchoState) -> Result<bool, Error> {
    state
        .client_boolean_config
        .lock()
        .map(|supported| *supported)
        .map_err(|_| internal("fixture capability lock poisoned"))
}

fn legacy_modes(configuration: &EchoConfiguration) -> SessionModeState {
    SessionModeState::new(
        SessionModeId::new(configuration.mode.clone()),
        vec![
            SessionMode::new("ask", "Ask"),
            SessionMode::new("code", "Code"),
        ],
    )
}

fn echo_config_options(
    configuration: &EchoConfiguration,
    include_boolean: bool,
    duplicate_config_id: bool,
) -> Vec<SessionConfigOption> {
    let thought_options = if configuration.model == "quality" {
        vec![
            SessionConfigSelectOption::new("medium", "Medium"),
            SessionConfigSelectOption::new("high", "High"),
        ]
    } else {
        vec![
            SessionConfigSelectOption::new("low", "Low"),
            SessionConfigSelectOption::new("medium", "Medium"),
        ]
    };
    let mut options = vec![
        SessionConfigOption::select(
            "mode",
            "Session Mode",
            configuration.mode.clone(),
            vec![
                SessionConfigSelectOption::new("ask", "Ask"),
                SessionConfigSelectOption::new("code", "Code"),
            ],
        )
        .category(SessionConfigOptionCategory::Mode),
        SessionConfigOption::select(
            "model",
            "Model",
            configuration.model.clone(),
            vec![
                SessionConfigSelectOption::new("fast", "Fast"),
                SessionConfigSelectOption::new("quality", "Quality"),
            ],
        )
        .category(SessionConfigOptionCategory::Model),
        SessionConfigOption::select(
            "thought_level",
            "Thought Level",
            configuration.thought_level.clone(),
            thought_options,
        )
        .category(SessionConfigOptionCategory::ThoughtLevel),
    ];
    if include_boolean {
        options.push(
            SessionConfigOption::boolean("brave", "Brave", configuration.brave)
                .category(SessionConfigOptionCategory::Other("_fixture".into())),
        );
    }
    if duplicate_config_id {
        options.push(SessionConfigOption::select(
            "model",
            "Duplicate Model",
            "fast",
            vec![SessionConfigSelectOption::new("fast", "Fast")],
        ));
    }
    options
}

fn session_response(
    response: NewSessionResponse,
    configuration: &EchoConfiguration,
    fixture: ConfigFixture,
) -> NewSessionResponse {
    let response = response.modes(legacy_modes(configuration));
    if matches!(fixture.surface, ConfigSurface::LegacyModes) {
        response
    } else {
        response.config_options(echo_config_options(
            configuration,
            matches!(
                fixture.boolean_option,
                BooleanOption::Negotiated(true) | BooleanOption::ForceAdvertise
            ),
            matches!(fixture.config_ids, ConfigIds::Duplicate),
        ))
    }
}

fn load_response(configuration: &EchoConfiguration, fixture: ConfigFixture) -> LoadSessionResponse {
    let response = LoadSessionResponse::new().modes(legacy_modes(configuration));
    if matches!(fixture.surface, ConfigSurface::LegacyModes) {
        response
    } else {
        response.config_options(echo_config_options(
            configuration,
            matches!(
                fixture.boolean_option,
                BooleanOption::Negotiated(true) | BooleanOption::ForceAdvertise
            ),
            matches!(fixture.config_ids, ConfigIds::Duplicate),
        ))
    }
}

fn resume_response(
    configuration: &EchoConfiguration,
    fixture: ConfigFixture,
) -> ResumeSessionResponse {
    let response = ResumeSessionResponse::new().modes(legacy_modes(configuration));
    if matches!(fixture.surface, ConfigSurface::LegacyModes) {
        response
    } else {
        response.config_options(echo_config_options(
            configuration,
            matches!(
                fixture.boolean_option,
                BooleanOption::Negotiated(true) | BooleanOption::ForceAdvertise
            ),
            matches!(fixture.config_ids, ConfigIds::Duplicate),
        ))
    }
}

fn apply_echo_config(
    configuration: &mut EchoConfiguration,
    config_id: &str,
    value: &SessionConfigOptionValue,
    boolean_supported: bool,
) -> Result<(), Error> {
    match (config_id, value) {
        ("mode", SessionConfigOptionValue::ValueId { value })
            if matches!(value.0.as_ref(), "ask" | "code") =>
        {
            configuration.mode = value.0.to_string();
        }
        ("model", SessionConfigOptionValue::ValueId { value })
            if matches!(value.0.as_ref(), "fast" | "quality") =>
        {
            configuration.model = value.0.to_string();
            configuration.thought_level = if value.0.as_ref() == "quality" {
                "high".into()
            } else {
                "low".into()
            };
        }
        ("thought_level", SessionConfigOptionValue::ValueId { value }) => {
            let allowed = if configuration.model == "quality" {
                matches!(value.0.as_ref(), "medium" | "high")
            } else {
                matches!(value.0.as_ref(), "low" | "medium")
            };
            if !allowed {
                return Err(internal("thought level is unavailable for current model"));
            }
            configuration.thought_level = value.0.to_string();
        }
        ("brave", SessionConfigOptionValue::Boolean { value }) if boolean_supported => {
            configuration.brave = *value;
        }
        _ => return Err(internal("invalid fixture config option selection")),
    }
    Ok(())
}

fn observe_content_matrix(cwd: &Path, blocks: &[ContentBlock]) -> Result<(), Error> {
    let mut observations = Vec::with_capacity(blocks.len());
    for (index, block) in blocks.iter().enumerate() {
        let observation = match block {
            ContentBlock::Text(text) => serde_json::json!({
                "type": "text",
                "text": text.text,
            }),
            ContentBlock::Image(image) => {
                let bytes = decode_fixture_base64(&image.data, "image")?;
                let name = format!("content-{index}-image.bin");
                fs::write(cwd.join(&name), &bytes)
                    .map_err(|error| internal(format!("write image observation: {error}")))?;
                serde_json::json!({
                    "type": "image",
                    "mime_type": image.mime_type,
                    "uri": image.uri,
                    "byte_length": bytes.len(),
                    "oracle_file": name,
                })
            }
            ContentBlock::Audio(audio) => {
                let bytes = decode_fixture_base64(&audio.data, "audio")?;
                let name = format!("content-{index}-audio.bin");
                fs::write(cwd.join(&name), &bytes)
                    .map_err(|error| internal(format!("write audio observation: {error}")))?;
                serde_json::json!({
                    "type": "audio",
                    "mime_type": audio.mime_type,
                    "byte_length": bytes.len(),
                    "oracle_file": name,
                })
            }
            ContentBlock::ResourceLink(resource) => serde_json::json!({
                "type": "resource_link",
                "name": resource.name,
                "uri": resource.uri,
                "mime_type": resource.mime_type,
                "size": resource.size,
            }),
            ContentBlock::Resource(resource) => match &resource.resource {
                EmbeddedResourceResource::TextResourceContents(text) => {
                    let name = format!("content-{index}-resource.txt");
                    fs::write(cwd.join(&name), text.text.as_bytes()).map_err(|error| {
                        internal(format!("write text resource observation: {error}"))
                    })?;
                    serde_json::json!({
                        "type": "resource",
                        "representation": "text",
                        "uri": text.uri,
                        "mime_type": text.mime_type,
                        "byte_length": text.text.len(),
                        "oracle_file": name,
                    })
                }
                EmbeddedResourceResource::BlobResourceContents(blob) => {
                    let bytes = decode_fixture_base64(&blob.blob, "resource_blob")?;
                    let name = format!("content-{index}-resource.bin");
                    fs::write(cwd.join(&name), &bytes).map_err(|error| {
                        internal(format!("write blob resource observation: {error}"))
                    })?;
                    serde_json::json!({
                        "type": "resource",
                        "representation": "blob",
                        "uri": blob.uri,
                        "mime_type": blob.mime_type,
                        "byte_length": bytes.len(),
                        "oracle_file": name,
                    })
                }
                _ => return Err(internal("unknown embedded resource representation")),
            },
            _ => return Err(internal("unknown ACP content block variant")),
        };
        observations.push(observation);
    }
    let bytes = serde_json::to_vec_pretty(&observations)
        .map_err(|error| internal(format!("serialize content observation: {error}")))?;
    fs::write(cwd.join(CONTENT_OBSERVATION_FILE), bytes)
        .map_err(|error| internal(format!("write content observation: {error}")))
}

fn decode_fixture_base64(data: &str, kind: &str) -> Result<Vec<u8>, Error> {
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| internal(format!("decode {kind} base64: {error}")))
}

fn state_path(cwd: &Path) -> PathBuf {
    cwd.join(STATE_FILE)
}

/// Where one session lives. The id is content of our own making
/// (`echo-<pid>-<nanos>`), and anything outside that shape is refused rather
/// than turned into a path.
fn session_path(cwd: &Path, id: &SessionId) -> Result<PathBuf, Error> {
    let name = id.0.as_ref();
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(internal("fixture session id is not a plain name"));
    }
    Ok(cwd.join(SESSIONS_DIR).join(format!("{name}.json")))
}

fn persist(session: &EchoSession) -> Result<(), Error> {
    let bytes = serde_json::to_vec(session)
        .map_err(|error| internal(format!("serialize fixture state: {error}")))?;
    let path = session_path(&session.cwd, &session.id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| internal(format!("create fixture session directory: {error}")))?;
    }
    fs::write(&path, &bytes)
        .map_err(|error| internal(format!("write fixture session: {error}")))?;
    // The newest session, for `session/list` and for anything that still asks
    // the workspace what it last did.
    fs::write(state_path(&session.cwd), bytes)
        .map_err(|error| internal(format!("write fixture state: {error}")))
}

fn restore(
    cwd: &Path,
    requested_id: &SessionId,
    mcp_servers: Vec<McpServer>,
) -> Result<EchoSession, Error> {
    if !cwd.is_absolute() {
        return Err(internal("fixture cwd is not absolute"));
    }
    let path = session_path(cwd, requested_id)?;
    let bytes = fs::read(&path).or_else(|_| {
        // A session written before this fixture kept them apart.
        fs::read(state_path(cwd))
            .map_err(|error| internal(format!("read fixture session: {error}")))
    })?;
    let mut session: EchoSession = serde_json::from_slice(&bytes)
        .map_err(|error| internal(format!("parse fixture state: {error}")))?;
    if session.id != *requested_id || session.cwd != cwd {
        return Err(internal("fixture session id or cwd does not match"));
    }
    session.mcp_names = mcp_servers.iter().map(mcp_name).collect();
    session.mcp_servers = mcp_servers;
    persist(&session)?;
    Ok(session)
}

/// Every session this workspace holds, oldest first.
fn sessions_in(cwd: &Path) -> Result<Vec<EchoSession>, Error> {
    if !cwd.is_absolute() {
        return Err(internal("fixture list cwd is not absolute"));
    }
    let directory = cwd.join(SESSIONS_DIR);
    let mut sessions = Vec::new();
    if let Ok(entries) = fs::read_dir(&directory) {
        let mut paths: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect();
        paths.sort();
        for path in paths {
            let bytes = fs::read(&path)
                .map_err(|error| internal(format!("read fixture session: {error}")))?;
            let session: EchoSession = serde_json::from_slice(&bytes)
                .map_err(|error| internal(format!("parse fixture session: {error}")))?;
            sessions.push(session);
        }
    }
    if sessions.is_empty() {
        // A workspace written before this fixture kept sessions apart.
        sessions.extend(restore_for_list(cwd)?);
    }
    Ok(sessions)
}

fn restore_for_list(cwd: &Path) -> Result<Option<EchoSession>, Error> {
    if !cwd.is_absolute() {
        return Err(internal("fixture list cwd is not absolute"));
    }
    let path = state_path(cwd);
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| internal(format!("read fixture state: {error}")))?;
    let session: EchoSession = serde_json::from_slice(&bytes)
        .map_err(|error| internal(format!("parse fixture state: {error}")))?;
    Ok(Some(session))
}

fn send_chunk(
    connection: &ConnectionTo<Client>,
    session_id: SessionId,
    update: SessionUpdate,
) -> Result<(), Error> {
    connection.send_notification(SessionNotification::new(session_id, update))
}

fn mcp_name(server: &McpServer) -> String {
    match server {
        McpServer::Stdio(server) => server.name.clone(),
        McpServer::Http(server) => server.name.clone(),
        McpServer::Sse(server) => server.name.clone(),
        _ => "unknown".into(),
    }
}

#[derive(Clone, Copy)]
enum FixtureElicitation {
    Form,
    Url,
}

fn elicitation_fixture(prompt: &str) -> Option<FixtureElicitation> {
    match serde_json::from_str::<Value>(prompt)
        .ok()?
        .get("fixture")?
        .as_str()?
    {
        "elicitation-form-v0.1" => Some(FixtureElicitation::Form),
        "elicitation-url-v0.1" => Some(FixtureElicitation::Url),
        _ => None,
    }
}

fn fixture_elicitation_request(
    mode: FixtureElicitation,
    session_id: SessionId,
) -> CreateElicitationRequest {
    let scope = ElicitationSessionScope::new(session_id).tool_call_id("fixture-elicitation");
    match mode {
        FixtureElicitation::Form => CreateElicitationRequest::new(
            ElicitationFormMode::new(
                scope,
                ElicitationSchema::new()
                    .title("Composition review")
                    .description("Flat primitive ACP form fixture")
                    .property(
                        "strategy",
                        StringPropertySchema::new().one_of(vec![
                            EnumOption::new("conservative", "Conservative"),
                            EnumOption::new("bold", "Bold"),
                        ]),
                        true,
                    )
                    .integer("iterations", 1, 8, true)
                    .number("gain_db", -24.0, 12.0, false)
                    .boolean("normalize", false)
                    .property(
                        "contact",
                        StringPropertySchema::email().pattern(r".+@example\.com"),
                        false,
                    )
                    .property(
                        "stems",
                        MultiSelectPropertySchema::titled(vec![
                            EnumOption::new("voice", "Voice"),
                            EnumOption::new("music", "Music"),
                            EnumOption::new("fx", "Effects"),
                        ])
                        .min_items(1)
                        .max_items(3),
                        true,
                    ),
            ),
            "Review and submit the non-sensitive composition choices.",
        ),
        FixtureElicitation::Url => CreateElicitationRequest::new(
            ElicitationUrlMode::new(
                scope,
                "fixture-oauth-1",
                "https://example.invalid/connect?elicitationId=fixture-oauth-1",
            ),
            "Authorize the fixture through an out-of-band page.",
        ),
    }
}

fn cancel_fixture(prompt: &str) -> Option<CancelFixture> {
    let request: Value = serde_json::from_str(prompt).ok()?;
    match request["fixture"].as_str()? {
        "cancel-v0.1" => Some(CancelFixture::Confirm),
        "cancel-race-v0.1" => Some(CancelFixture::CompleteRace),
        "cancel-error-v0.1" => Some(CancelFixture::ProtocolError),
        "cancel-ignore-v0.1" => Some(CancelFixture::Ignore),
        _ => None,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one fixture transaction keeps generic tool selection, permission correlation and the exact MCP call visible"
)]
async fn call_requested_mcp(
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    session: &EchoSession,
    prompt: &str,
) -> Result<(Option<Value>, bool), Error> {
    let Ok(request): Result<Value, _> = serde_json::from_str(prompt) else {
        return Ok((None, false));
    };
    let fixture = request["fixture"].as_str().unwrap_or_default();
    if !matches!(
        fixture,
        "mcp-echo-v0.1"
            | "mcp-echo-permission-v0.1"
            | "mcp-apps-note-v0.1"
            | "mcp-apps-error-v0.1"
            | "mcp-apps-cancel-v0.1"
            | "mcp-apps-delay-v0.1"
            | "mcp-call-v0.1"
            | "mcp-call-permission-v0.1"
            | "mcp-task-v0.1"
    ) {
        return Ok((None, false));
    }
    let server_name = request["server"]
        .as_str()
        .ok_or_else(|| internal("mcp echo fixture needs server"))?;
    let nonce = request["nonce"]
        .as_str()
        .ok_or_else(|| internal("mcp echo fixture needs nonce"))?;
    let server = session
        .mcp_servers
        .iter()
        .find_map(|server| match server {
            McpServer::Stdio(server) if server.name == server_name => Some(server),
            _ => None,
        })
        .ok_or_else(|| internal(format!("no stdio MCP attachment named {server_name}")))?;

    // The apps-note mode is the App-disabled leg of the MCP Apps gate: the
    // identical semantic operation through the ordinary headless tool path.
    let (tool_name, arguments) = if matches!(
        fixture,
        "mcp-call-v0.1" | "mcp-call-permission-v0.1" | "mcp-task-v0.1"
    ) {
        let tool = request["tool"]
            .as_str()
            .ok_or_else(|| internal("generic MCP fixture needs tool"))?
            .to_owned();
        let arguments = request["arguments"]
            .as_object()
            .cloned()
            .ok_or_else(|| internal("generic MCP fixture needs object arguments"))?;
        (tool, JsonObject::from_iter(arguments))
    } else if matches!(
        fixture,
        "mcp-apps-note-v0.1"
            | "mcp-apps-error-v0.1"
            | "mcp-apps-cancel-v0.1"
            | "mcp-apps-delay-v0.1"
    ) {
        let text = request["text"]
            .as_str()
            .ok_or_else(|| internal("apps note fixture needs text"))?;
        let tool = match fixture {
            "mcp-apps-error-v0.1" => "fail_note",
            "mcp-apps-cancel-v0.1" => "cancellable_note",
            "mcp-apps-delay-v0.1" => "delayed_note",
            _ => "save_note",
        };
        let mut values = BTreeMap::from_iter([
            ("nonce".to_owned(), Value::String(nonce.to_owned())),
            ("text".to_owned(), Value::String(text.to_owned())),
        ]);
        if fixture == "mcp-apps-delay-v0.1" {
            values.insert(
                "delay_ms".to_owned(),
                Value::from(request["delay_ms"].as_u64().unwrap_or(3_000)),
            );
        }
        (tool.to_owned(), JsonObject::from_iter(values))
    } else {
        (
            "echo".to_owned(),
            JsonObject::from_iter(BTreeMap::from_iter([(
                "nonce".to_owned(),
                Value::String(nonce.to_owned()),
            )])),
        )
    };
    let permitted_call = if matches!(
        fixture,
        "mcp-echo-permission-v0.1" | "mcp-call-permission-v0.1"
    ) {
        match request_echo_permission(
            connection,
            session_id,
            &request,
            server_name,
            &tool_name,
            nonce,
            &arguments,
        )
        .await?
        {
            EchoPermissionOutcome::Allowed(tool_call_id) => Some(tool_call_id),
            EchoPermissionOutcome::Rejected(result) => return Ok((Some(result), false)),
            EchoPermissionOutcome::Cancelled => return Ok((None, true)),
        }
    } else {
        None
    };

    let mut command = tokio::process::Command::new(&server.command);
    command.args(&server.args);
    for variable in &server.env {
        command.env(&variable.name, &variable.value);
    }
    let transport = TokioChildProcess::new(command).map_err(|error| internal(error.to_string()))?;
    if fixture == "mcp-task-v0.1" {
        let result = call_as_task(transport, tool_name, arguments).await?;
        return Ok((Some(result), false));
    }
    let client = ().serve(transport).await.map_err(|error| internal(error.to_string()))?;
    if fixture == "mcp-apps-cancel-v0.1" {
        let handle = client
            .send_cancellable_request(
                ClientRequest::CallToolRequest(Request::new(
                    CallToolRequestParams::new(tool_name).with_arguments(arguments),
                )),
                PeerRequestOptions::no_options(),
            )
            .await
            .map_err(|error| internal(error.to_string()))?;
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        handle
            .cancel(Some("native agent cancelled fixture call".into()))
            .await
            .map_err(|error| internal(error.to_string()))?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        client
            .cancel()
            .await
            .map_err(|error| internal(error.to_string()))?;
        return Ok((None, true));
    }
    let response = client
        .call_tool(CallToolRequestParams::new(tool_name).with_arguments(arguments))
        .await
        .map_err(|error| internal(error.to_string()))?;
    let result = response
        .structured_content
        .clone()
        .unwrap_or_else(|| serde_json::to_value(&response).unwrap_or(Value::Null));
    client
        .cancel()
        .await
        .map_err(|error| internal(error.to_string()))?;
    if let Some(tool_call_id) = permitted_call {
        let mut completed = ToolCallUpdateFields::default();
        completed.status = Some(ToolCallStatus::Completed);
        completed.raw_output = Some(result.clone());
        send_chunk(
            connection,
            session_id.clone(),
            SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(tool_call_id, completed)),
        )?;
    }
    Ok((Some(result), false))
}

/// The task leg of the generic MCP fixture: a client that declares the tasks
/// extension calls the tool once and, when the server answers with a task
/// handle, polls `tasks/get` at the server's interval until a terminal
/// status. The reply carries the tool's ordinary result plus the handle's
/// history (`task: null` when the server answered synchronously).
async fn call_as_task(
    transport: TokioChildProcess,
    tool_name: String,
    arguments: JsonObject,
) -> Result<Value, Error> {
    let client = ClientInfo::new(
        ClientCapabilities::builder().enable_tasks().build(),
        Implementation::new("swem-echo-agent", env!("CARGO_PKG_VERSION")),
    )
    .serve(transport)
    .await
    .map_err(|error| internal(error.to_string()))?;
    let response = client
        .call_tool_once(CallToolRequestParams::new(tool_name).with_arguments(arguments))
        .await
        .map_err(|error| internal(error.to_string()))?;
    let (tool_result, task) = match response {
        CallToolResponse::Complete(result) => (result, Value::Null),
        CallToolResponse::Task(created) => {
            let task_id = created.task.task_id.clone();
            let interval = created.task.poll_interval_ms.unwrap_or(250);
            let mut statuses = vec![serde_json::to_value(created.task.status).unwrap_or_default()];
            let mut polls = 0u64;
            let result = loop {
                tokio::time::sleep(std::time::Duration::from_millis(interval)).await;
                let detailed = client
                    .peer()
                    .get_task(GetTaskParams::new(task_id.clone()))
                    .await
                    .map_err(|error| internal(error.to_string()))?
                    .task;
                polls += 1;
                let status = detailed.status();
                let name = serde_json::to_value(status).unwrap_or_default();
                if statuses.last() != Some(&name) {
                    statuses.push(name);
                }
                if status.is_terminal() {
                    break match detailed.payload {
                        TaskPayload::Completed { result } => {
                            serde_json::from_value::<CallToolResult>(Value::Object(result))
                                .map_err(|error| internal(error.to_string()))?
                        }
                        other => {
                            return Err(internal(format!(
                                "long operation {task_id} ended without a result: {other:?}"
                            )));
                        }
                    };
                }
                if polls > 2_400 {
                    return Err(internal(format!(
                        "long operation {task_id} never reached a terminal status"
                    )));
                }
            };
            (
                result,
                json!({"task_id": task_id, "statuses": statuses, "polls": polls}),
            )
        }
        other => {
            return Err(internal(format!(
                "task fixture answers neither input requests nor other responses: {other:?}"
            )));
        }
    };
    let mcp_result = tool_result
        .structured_content
        .clone()
        .unwrap_or_else(|| serde_json::to_value(&tool_result).unwrap_or(Value::Null));
    client
        .cancel()
        .await
        .map_err(|error| internal(error.to_string()))?;
    Ok(json!({"mcp_result": mcp_result, "task": task}))
}

async fn request_echo_permission(
    connection: &ConnectionTo<Client>,
    session_id: &SessionId,
    request: &Value,
    server_name: &str,
    tool_name: &str,
    nonce: &str,
    arguments: &JsonObject,
) -> Result<EchoPermissionOutcome, Error> {
    let tool_call_id = ToolCallId::new(format!("echo-{nonce}"));
    let title = request["title"]
        .as_str()
        .map_or_else(|| format!("mcp__{server_name}__{tool_name}"), str::to_owned);
    let raw_input = Value::Object(arguments.clone());
    let mut fields = ToolCallUpdateFields::default();
    fields.title = Some(title.clone());
    fields.kind = Some(ToolKind::Other);
    fields.status = Some(ToolCallStatus::Pending);
    fields.raw_input = Some(raw_input.clone());
    if request["report_tool_call"].as_bool().unwrap_or(true) {
        send_chunk(
            connection,
            session_id.clone(),
            SessionUpdate::ToolCall(
                ToolCall::new(tool_call_id.clone(), title)
                    .kind(ToolKind::Other)
                    .status(ToolCallStatus::Pending)
                    .raw_input(raw_input),
            ),
        )?;
    }
    let permission = connection
        .send_request(RequestPermissionRequest::new(
            session_id.clone(),
            ToolCallUpdate::new(tool_call_id.clone(), fields),
            vec![
                PermissionOption::new("allow-once", "Allow once", PermissionOptionKind::AllowOnce),
                PermissionOption::new(
                    "reject-once",
                    "Reject once",
                    PermissionOptionKind::RejectOnce,
                ),
            ],
        ))
        .block_task()
        .await?;
    let selected = match permission.outcome {
        RequestPermissionOutcome::Selected(selected) => selected.option_id.0.to_string(),
        RequestPermissionOutcome::Cancelled => return Ok(EchoPermissionOutcome::Cancelled),
        _ => {
            return Ok(EchoPermissionOutcome::Rejected(
                serde_json::json!({ "permission": "unsupported" }),
            ));
        }
    };
    if selected != "allow-once" {
        return Ok(EchoPermissionOutcome::Rejected(serde_json::json!({
            "permission": "rejected",
            "selected_option": selected,
        })));
    }
    let mut running = ToolCallUpdateFields::default();
    running.status = Some(ToolCallStatus::InProgress);
    send_chunk(
        connection,
        session_id.clone(),
        SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(tool_call_id.clone(), running)),
    )?;
    Ok(EchoPermissionOutcome::Allowed(tool_call_id))
}

fn internal(message: impl Into<String>) -> Error {
    Error::internal_error().data(serde_json::json!({"message": message.into()}))
}
