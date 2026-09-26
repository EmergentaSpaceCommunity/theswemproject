use std::fs;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::McpServer;
use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use swem_host::{
    AgentDistributionBuildPlan, AgentDistributionBuildSpec, AgentDistributionInventory,
    BackendProbe, BackendProbeStatus, DistributionBuildAuthorization, ManagedProvisioningPolicy,
    NativeSessionOptions, NativeSessionStart, PersonalAgentProfile, PersonalAgentProfileStore,
    PodmanImageBootstrapPlan, PodmanImageBootstrapSpec, PodmanOrphanReconciliationPlan,
    PodmanProvisioningOptions, ProvisioningAuthorization, ProvisioningDisposition,
    ProvisioningPlan, Readiness, agent_distribution_build_plan,
    apply_agent_distribution_build_plan, apply_podman_image_bootstrap_plan,
    apply_podman_orphan_reconciliation_plan, apply_podman_provisioning_plan, install_plan,
    podman_image_bootstrap_plan, podman_orphan_reconciliation_plan, podman_provisioning_plan,
    probe_podman, probe_podman_endpoints, run_native_session, verify_discovered_agent,
};

#[derive(Debug, Parser)]
#[command(name = "swem", version, about = "SWEM reference meta-harness")]
struct Cli {
    /// Omitted, the product starts: the Workbench with the projects this
    /// machine already has.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Answer an editor that speaks ACP, on this process's stdio. The editor
    /// starts this; a person points their editor at `swem acp --profile <id>`
    /// and works with the agent that profile runs.
    Acp(Acp),
    Agents(Agents),
    Environments(Environments),
    Mcp(Mcp),
    Profiles(Profiles),
    Workbench(Workbench),
}

#[derive(Debug, Args)]
struct Workbench {
    #[command(subcommand)]
    command: Option<WorkbenchCommand>,
}

#[derive(Debug, Subcommand)]
enum WorkbenchCommand {
    /// Serve the generic Workbench shell: profile selection, an interactive
    /// native driver and the durable route-event projection over HTTP. The
    /// shell knows no agent, Cycle or domain schema.
    Serve {
        /// Profile inventory directory (see `swem profiles`).
        #[arg(long)]
        inventory: Option<PathBuf>,
        /// Durable routing ledger file.
        #[arg(long)]
        ledger: Option<PathBuf>,
        /// Port to bind on 127.0.0.1 (0 picks an ephemeral port).
        #[arg(long, default_value_t = 0)]
        port: u16,
        /// Port of the App sandbox origin on 127.0.0.1 (0 picks an ephemeral
        /// port); fix it to forward both origins to a remote machine.
        #[arg(long, default_value_t = 0)]
        sandbox_port: u16,
        /// Deadline for each individual ACP operation; idle time between
        /// interactive turns is deliberately unbounded.
        #[arg(long, default_value_t = 600)]
        operation_timeout_secs: u64,
        /// JSON file containing one standard ACP `McpServer` declaration;
        /// repeat to resolve every profile attachment by its server name.
        #[arg(long)]
        mcp_server: Vec<PathBuf>,
        /// Directory holding the esbuild output `apps-bridge.js` (see
        /// crates/swem-host/web/apps-host). Absent = the Apps panel is
        /// disabled and tools stay available through the ordinary path.
        #[arg(long)]
        apps_bundle: Option<PathBuf>,
        /// Print the URL without opening it in the system browser.
        #[arg(long)]
        no_open: bool,
        /// The ACP registry index this SWEM reads when it resolves an agent
        /// install plan. Defaults to the public CDN; name a mirror, an
        /// internal copy or a `file://` path when this machine cannot reach
        /// it. The plan is still shown, confirmed and receipted the same way.
        #[arg(long)]
        acp_registry: Option<String>,
        /// The image a profile that runs in a container runs its agent in,
        /// pinned as `sha256:<id>` or `name@sha256:<id>`. The image must run
        /// an ACP agent at `/usr/local/bin/swem-agent`. Also read from
        /// `SWEM_CONTAINER_IMAGE`.
        #[arg(long)]
        container_image: Option<String>,
    },
}

#[derive(Debug, Args)]
struct Acp {
    /// Which profile's agent the editor is talking to.
    #[arg(long)]
    profile: String,
    #[arg(long)]
    inventory: Option<PathBuf>,
    #[arg(long)]
    ledger: Option<PathBuf>,
    #[arg(long, default_value_t = 600)]
    operation_timeout_secs: u64,
    /// The image a profile that runs in a container runs its agent in; see
    /// the same option on `workbench serve`.
    #[arg(long)]
    container_image: Option<String>,
}

#[derive(Debug, Args)]
struct Profiles {
    #[command(subcommand)]
    command: ProfilesCommand,
}

#[derive(Debug, Subcommand)]
enum ProfilesCommand {
    /// Persist one canonical profile from an exact JSON document.
    Create {
        #[arg(long)]
        inventory: PathBuf,
        #[arg(long)]
        input: PathBuf,
    },
    /// List all valid profiles in stable id order.
    List {
        #[arg(long)]
        inventory: PathBuf,
    },
    /// Resolve one exact profile for this invocation; no global current state.
    Select {
        profile_id: String,
        #[arg(long)]
        inventory: PathBuf,
    },
}

#[derive(Debug, Args)]
struct Environments {
    #[command(subcommand)]
    command: EnvironmentsCommand,
}

#[derive(Debug, Subcommand)]
enum EnvironmentsCommand {
    /// Read-only discovery of environment backends and their observed readiness.
    Probe,
    /// Preview exact Podman machine operations and host effects without mutation.
    PlanPodman {
        #[arg(long, default_value = "swem")]
        machine_name: String,
        #[arg(long, default_value_t = 2)]
        cpus: u16,
        #[arg(long, default_value_t = 2048)]
        memory_mib: u64,
        #[arg(long, default_value_t = 20)]
        disk_gib: u64,
        #[arg(long)]
        rootful: bool,
        /// Opt into Podman's WSL-wide user-mode networking side effect.
        #[arg(long)]
        user_mode_networking: bool,
        /// Mark this preview for a pre-authorized managed host policy.
        #[arg(long)]
        managed: bool,
        /// Optional file receiving the exact preview JSON later passed to apply-podman.
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Apply an unchanged preview, then prove readiness with a fresh probe.
    ApplyPodman {
        /// JSON file written from `plan-podman --output`.
        #[arg(long)]
        plan: PathBuf,
        /// Exact plan id shown in a confirm preview. Omit only for managed plans.
        #[arg(long, conflicts_with = "managed_policy")]
        confirm_plan: Option<String>,
        /// Host-owned JSON policy authorizing this backend, machine and resource envelope.
        #[arg(long, conflicts_with = "confirm_plan")]
        managed_policy: Option<PathBuf>,
    },
    /// Preview acquisition of one fully-qualified digest-pinned Linux image.
    PlanPodmanImage {
        /// Exact endpoint id from `environments probe`; inferred only when one
        /// ready Podman endpoint exists.
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        reference: String,
        #[arg(long, default_value = "linux/amd64")]
        platform: String,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Apply an unchanged image preview and verify its resolved local identity.
    ApplyPodmanImage {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        confirm_plan: String,
    },
    /// Preview exact SWEM-owned containers absent from active lease inventory.
    PlanPodmanReconciliation {
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long = "active-instance")]
        active_instances: Vec<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Remove only candidates from an unchanged reconciliation preview.
    ApplyPodmanReconciliation {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        confirm_plan: String,
    },
}

#[derive(Debug, Args)]
struct Agents {
    #[command(subcommand)]
    command: AgentsCommand,
}

#[derive(Debug, Subcommand)]
enum AgentsCommand {
    /// Discover curated coding agents on the current OS without launching them.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Resolve a consent-required install plan; does not install anything.
    PlanInstall { agent_id: String },
    /// Install the registry's exact typed npm or verified native distribution
    /// under the product's install root. Any id the registry lists may be
    /// named, not only the ones this build describes. Requires --consent.
    Install {
        agent_id: String,
        /// Explicit consent to download and install the pinned distribution.
        #[arg(long)]
        consent: bool,
        /// Exact content id printed by `plan-install` or the consent preview.
        #[arg(long)]
        confirm_plan: Option<String>,
    },
    /// Preview a locked OCI distribution build without pulling or building.
    PlanDistributionImage {
        /// JSON build specification naming the exact context, lock and adapter.
        #[arg(long)]
        spec: PathBuf,
        /// Exact endpoint id; inferred only when one ready Podman endpoint exists.
        #[arg(long)]
        endpoint: Option<String>,
        #[arg(long)]
        output: Option<PathBuf>,
    },
    /// Apply an unchanged distribution build preview and record its inspection.
    ApplyDistributionImage {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        confirm_plan: String,
        #[arg(long)]
        inventory: PathBuf,
    },
    /// List verified local distribution receipts in stable id order.
    ListDistributions {
        #[arg(long)]
        inventory: PathBuf,
    },
    /// Explicitly launch an installed ACP agent and verify its v1 handshake.
    Verify { agent_id: String },
    /// Run one or more turns in a native ACP agent without enabling SWEM Cycle.
    /// Tool permissions are denied unless a later explicit host policy grants them.
    Session {
        agent_id: String,
        /// Existing workspace directory bound to the ACP session.
        #[arg(long)]
        workspace: PathBuf,
        /// UTF-8 prompt file; repeat to send multiple turns in the same session.
        #[arg(long, required = true)]
        prompt_file: Vec<PathBuf>,
        /// Load this native ACP session and receive replayed history before prompting.
        #[arg(long, conflicts_with = "resume_session")]
        load_session: Option<String>,
        /// Resume this native ACP session without replaying its history.
        #[arg(long, conflicts_with = "load_session")]
        resume_session: Option<String>,
        /// JSONL protocol transcript (MCP environment values are never logged).
        #[arg(long)]
        transcript: PathBuf,
        /// JSON file containing one standard ACP `McpServer` declaration;
        /// repeat to attach multiple arbitrary servers without enabling Cycle.
        #[arg(long)]
        mcp_server: Vec<PathBuf>,
        /// Seconds to wait for the complete multi-turn session.
        #[arg(long, default_value_t = 600)]
        timeout_secs: u64,
    },
}

#[derive(Debug, Args)]
struct Mcp {
    #[command(subcommand)]
    command: McpCommand,
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    /// Internal byte-transparent stdio wrapper used by Workbench Apps.
    #[command(hide = true)]
    ObserveStdio {
        #[arg(long)]
        server_name: String,
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = Cli::parse()
        .command
        .unwrap_or(Command::Workbench(Workbench { command: None }));
    match command {
        Command::Agents(agents) => run_agents(agents).await,
        Command::Environments(environments) => run_environments(environments),
        Command::Mcp(mcp) => run_mcp(mcp).await,
        Command::Profiles(profiles) => run_profiles(profiles),
        Command::Acp(acp) => run_acp(acp).await,
        Command::Workbench(workbench) => run_workbench(workbench).await,
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one linear product startup keeps paths, discovery, resolver and server ownership auditable"
)]
async fn run_workbench(workbench: Workbench) -> Result<()> {
    match workbench.command.unwrap_or(WorkbenchCommand::Serve {
        inventory: None,
        ledger: None,
        port: 0,
        sandbox_port: 0,
        operation_timeout_secs: 600,
        mcp_server: Vec::new(),
        apps_bundle: None,
        no_open: false,
        acp_registry: None,
        container_image: None,
    }) {
        WorkbenchCommand::Serve {
            inventory,
            ledger,
            port,
            sandbox_port,
            operation_timeout_secs,
            mcp_server,
            apps_bundle,
            no_open,
            acp_registry,
            container_image,
        } => {
            if let Some(index) = acp_registry.as_deref() {
                swem_host::set_acp_registry_index(index).map_err(anyhow::Error::msg)?;
            }
            let (state, data_root) = assemble_product(
                inventory,
                ledger,
                operation_timeout_secs,
                &mcp_server,
                container_image,
            )?;
            let token = swem_host::mint_session_token().map_err(anyhow::Error::msg)?;
            state.set_session_token(token.clone());
            // The clock. A standing instruction runs where the product runs,
            // so it starts with the product and stops with it, and it holds
            // the same state the page does - a scheduled turn is a turn like
            // any other, on the same lanes and in the same record.
            let state = std::sync::Arc::new(state);
            state
                .enable_schedules(&data_root.join("schedules"))
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let handle = swem_host::serve_workbench_http_with_apps_at(
                state,
                ([127, 0, 0, 1], port).into(),
                ([127, 0, 0, 1], sandbox_port).into(),
                apps_bundle,
            )
            .await
            .map_err(anyhow::Error::msg)?;
            // The address carries this run's secret. Loopback is not a
            // boundary: every program on this machine can reach the port, and
            // behind it are a live terminal and a profile's secrets. Whoever
            // can read what this printed is who may work this Workbench.
            let url = format!(
                "http://127.0.0.1:{}/?token={token}",
                handle.local_addr.port()
            );
            println!("SWEM Workbench: {url}");
            println!(
                "App sandbox origin: http://127.0.0.1:{}",
                handle.sandbox_addr.port()
            );
            if !no_open && let Err(error) = open_system_browser(&url) {
                eprintln!("could not open the system browser: {error}");
            }
            std::future::pending::<()>().await;
            Ok(())
        }
    }
}

/// Where the agent is inside the image this product runs it in.
///
/// One path, by convention, so a person needs to say only which image. Any
/// image that puts an ACP agent there can be named with `--container-image`;
/// `scripts/build-agent-container-image.sh` builds one from what is on this
/// machine, which is how a machine that cannot reach a registry still has one.
const AGENT_IN_THE_IMAGE: &str = "/usr/local/bin/swem-agent";

/// The uid:gid that owns `path`, as a container user.
///
/// # Errors
///
/// Returns a sentence a person can act on when the owner is root: a container
/// environment exists to keep an agent out of the machine, so running it as
/// the machine's root would give away the thing it was chosen for.
#[cfg(unix)]
fn owner_of(path: &std::path::Path) -> Result<String, String> {
    use std::os::unix::fs::MetadataExt as _;
    let owner = std::fs::metadata(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    if owner.uid() == 0 || owner.gid() == 0 {
        return Err(format!(
            "this profile runs in a container, and {} belongs to root. A container environment \
             will not run an agent as the machine's root; run SWEM as the person who owns the \
             work, or give this profile a directory they own",
            path.display()
        ));
    }
    Ok(format!("{}:{}", owner.uid(), owner.gid()))
}

#[cfg(not(unix))]
fn owner_of(_path: &std::path::Path) -> Result<String, String> {
    Err("a container environment is a Linux container, and this is not a Linux host".into())
}

/// The profile's agent, in a container on this machine.
///
/// The container is created here rather than when the session starts: the
/// harness takes a connection that already names one exact lease and the
/// transport bound to it, so that it can refuse a session whose environment
/// drifted from the profile instead of discovering the drift halfway through
/// a turn.
fn in_a_container(
    profile: &swem_host::PersonalAgentProfile,
    image: Option<&str>,
    mcp_servers: Vec<McpServer>,
    providers_root: &std::path::Path,
) -> Result<swem_host::ResolvedAgentConnection, String> {
    let image = image.ok_or_else(|| {
        format!(
            "this profile runs in a container, and this product was not told which image to run \
             an agent in. Start it with --container-image sha256:<id> (or SWEM_CONTAINER_IMAGE), \
             naming an image that runs an ACP agent at {AGENT_IN_THE_IMAGE}"
        )
    })?;
    let probe = swem_host::probe_podman_endpoints()
        .into_iter()
        .find(|probe| probe.status == swem_host::BackendProbeStatus::Ready)
        .ok_or_else(|| {
            let probe = swem_host::probe_podman();
            let said = probe
                .diagnostics
                .first()
                .cloned()
                .unwrap_or_else(|| "no Podman endpoint answered".to_owned());
            format!("this profile runs in a container, and this machine cannot: {said}")
        })?;

    let mut requirements = swem_host::EnvironmentRequirements::new(&profile.workspace);
    requirements.cpu_limit = Some(2);
    requirements.memory_mib = Some(2_048);
    requirements.required_guarantees.extend([
        swem_host::EnvironmentGuarantee::AgentProcessIsolation,
        swem_host::EnvironmentGuarantee::FilesystemIsolation,
        swem_host::EnvironmentGuarantee::NetworkDenyByDefault,
        swem_host::EnvironmentGuarantee::ResourceLimits,
    ]);

    let mut spec = swem_host::PodmanContainerSpec::deny_network(
        image,
        swem_host::PodmanWorkspaceBinding {
            service_source: profile.workspace.display().to_string(),
            container_target: "/workspace".into(),
        },
        AGENT_IN_THE_IMAGE,
    );
    // The agent keeps whatever it keeps between sessions, and it keeps it on
    // the person's disk rather than inside a container that will be removed.
    spec.agent_home = Some(swem_host::PodmanWorkspaceBinding {
        service_source: profile.agent_home.display().to_string(),
        container_target: "/home/swem".into(),
    });
    // Who the agent is inside the container: whoever owns the directory it
    // works in. A container user that is not that owner sees the person's own
    // workspace as read-only, which surfaces as "Permission denied" on a path
    // that plainly exists - and a container run as root is refused outright,
    // because an agent kept out of the machine should not be its root.
    spec.user = owner_of(&profile.workspace)?;
    // The model and the provider's address, as plain variables of the agent
    // process - the same ones a direct launch gets. The files (the role, the
    // skills) were already written into the workspace, which the container
    // mounts; only the variables need carrying across. A key never goes this
    // way: keys are runtime secrets.
    let provider = match &profile.model_provider {
        Some(id) => Some(
            swem_host::workbench_shell::ModelProviderBook::open(providers_root)
                .and_then(|book| book.get(id))
                .map_err(|error| error.to_string())?,
        ),
        None => None,
    };
    spec.environment =
        swem_host::agent_setup::materialise_profile(profile, provider.as_ref())?.environment;

    let lease_id = format!(
        "workbench-container-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    );
    let lease = swem_host::prepare_podman_lease(&probe, lease_id, &requirements, &spec)
        .map_err(|error| format!("this profile's container could not be prepared: {error}"))?;
    let transport = swem_host::EnvironmentTransport::podman_endpoint(&lease, &probe)
        .map_err(|error| format!("this profile's container could not be reached: {error}"))?;

    Ok(swem_host::ResolvedAgentConnection {
        launch: swem_host::LaunchCommand {
            executable: AGENT_IN_THE_IMAGE.into(),
            args: Vec::new(),
            integration: swem_host::IntegrationKind::DirectAcp,
        },
        agent_executable: std::path::PathBuf::from(AGENT_IN_THE_IMAGE),
        mcp_servers,
        environment: swem_host::ResolvedAgentEnvironment::Prepared {
            environment_profile_id: profile.environment_profile_id.clone(),
            lease: Box::new(lease),
            transport: Box::new(transport),
        },
    })
}

/// Everything the product is, before a door is opened onto it: the data root,
/// the profiles, the route ledger, the projects and MCP servers this machine
/// has declared, the agents it can discover, and the resolver that turns a
/// profile into a running agent.
///
/// It is one function because there are now two doors onto the same product -
/// the Workbench over HTTP and the editor door over stdio - and a second
/// assembly would be a second product wearing the same name. What differs
/// between the doors stays with each door: the Workbench mints a session
/// secret and runs the clock, the editor door does neither.
#[allow(
    clippy::too_many_lines,
    reason = "one linear product startup keeps paths, discovery, resolver and server ownership auditable"
)]
fn assemble_product(
    inventory: Option<PathBuf>,
    ledger: Option<PathBuf>,
    operation_timeout_secs: u64,
    mcp_server: &[PathBuf],
    container_image: Option<String>,
) -> Result<(swem_host::WorkbenchShellState, PathBuf)> {
    let data_root = workbench_data_root()?;
    fs::create_dir_all(&data_root)
        .with_context(|| format!("create Workbench data root {}", data_root.display()))?;
    let inventory = inventory.unwrap_or_else(|| data_root.join("profiles"));
    let ledger = ledger.unwrap_or_else(|| data_root.join("routes.jsonl"));
    // Connection-local MCP declarations, keyed by their ACP names.
    // Shared with the host so a project created from the product is
    // attachable by an agent session without a restart.
    let declarations: std::sync::Arc<
        std::sync::Mutex<std::collections::BTreeMap<String, McpServer>>,
    > = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let projects_root = data_root.join("projects");
    // Every project this product made before: its declaration lives
    // beside it, so there is no index to fall out of step.
    for entry in fs::read_dir(&projects_root).into_iter().flatten().flatten() {
        let manifest = entry.path().join("project.json");
        if !manifest.is_file() {
            continue;
        }
        let bytes = fs::read(&manifest).with_context(|| format!("read {}", manifest.display()))?;
        let server: McpServer = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse {}", manifest.display()))?;
        let name = match &server {
            McpServer::Stdio(stdio) => stdio.name.clone(),
            McpServer::Http(http) => http.name.clone(),
            McpServer::Sse(sse) => sse.name.clone(),
            other => bail!(
                "unsupported MCP transport in {}: {other:?}",
                manifest.display()
            ),
        };
        declarations
            .lock()
            .map_err(|_| anyhow::anyhow!("declaration registry poisoned"))?
            .insert(name, server);
    }
    for path in mcp_server {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let server: McpServer = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse ACP MCP server declaration {}", path.display()))?;
        let name = match &server {
            McpServer::Stdio(stdio) => stdio.name.clone(),
            McpServer::Http(http) => http.name.clone(),
            McpServer::Sse(sse) => sse.name.clone(),
            other => bail!("unsupported MCP transport in declaration: {other:?}"),
        };
        declarations
            .lock()
            .map_err(|_| anyhow::anyhow!("declaration registry poisoned"))?
            .insert(name, server);
    }
    let declared_agents = declared_agents()?;
    let installed_root = installed_root()?;
    let discovered = swem_host::discover_agents_with(declared_agents.clone(), &installed_root);
    // The same declarations feed the Project space, which dials them
    // on its own - no profile, resolver or environment takes part.
    let project_declarations: Vec<McpServer> = declarations
        .lock()
        .map_err(|_| anyhow::anyhow!("declaration registry poisoned"))?
        .values()
        .cloned()
        .collect();
    let resolver_declarations = std::sync::Arc::clone(&declarations);
    let resolver_providers = data_root.join("model-providers");
    // Which image a container environment runs an agent in. A setting rather
    // than something the product decides, for the same reason the agent
    // registry is one: a person may have their own, a mirror, or one built on
    // the machine itself where no registry can be reached.
    let container_image = container_image.or_else(|| {
        std::env::var("SWEM_CONTAINER_IMAGE")
            .ok()
            .filter(|value| !value.trim().is_empty())
    });
    let state = swem_host::WorkbenchShellState::open_with_environment(
        &inventory,
        &ledger,
        std::time::Duration::from_secs(operation_timeout_secs),
        move |profile| {
            // The profile names where its agent runs, and the catalogue is
            // asked before anything is built: a profile naming an environment
            // this product does not have is refused with the name in the
            // message rather than quietly started here.
            // Matched exhaustively on purpose: a new row in that catalogue
            // stops this compiling until the resolver says what it does with
            // it, instead of running it here by default.
            let backend = swem_host::environment_backend(&profile.environment_profile_id)?;
            let discovery =
                swem_host::discover_agents_with(declared_agents.clone(), &installed_root)
                    .into_iter()
                    .find(|agent| agent.id == profile.agent_id)
                    .ok_or_else(|| format!("unknown agent: {}", profile.agent_id))?;
            if discovery.readiness == Readiness::Absent {
                return Err(format!("agent is not installed: {}", profile.agent_id));
            }
            let launch = discovery
                .launch
                .clone()
                .ok_or_else(|| "discovered agent has no launch command".to_owned())?;
            let agent_executable = discovery
                .executable_path
                .clone()
                .ok_or_else(|| "discovered agent has no executable path".to_owned())?;
            let mut mcp_servers = Vec::new();
            let declared = resolver_declarations
                .lock()
                .map_err(|_| "declaration registry poisoned".to_owned())?;
            for attachment in &profile.attachments {
                let server = declared.get(&attachment.server_name).ok_or_else(|| {
                    format!("no declared project named {}", attachment.server_name)
                })?;
                mcp_servers.push(server.clone());
            }
            drop(declared);
            match backend {
                swem_host::EnvironmentBackend::ThisMachine => {
                    Ok(swem_host::ResolvedDirectAgentConnection {
                        launch,
                        agent_executable,
                        mcp_servers,
                    }
                    .into())
                }
                // The container is prepared here, before the session exists,
                // because that is what the harness validates against: a
                // connection arrives with one exact lease and one transport
                // bound to it, or it does not arrive.
                swem_host::EnvironmentBackend::InAContainer => in_a_container(
                    profile,
                    container_image.as_deref(),
                    mcp_servers,
                    &resolver_providers,
                ),
            }
        },
    )
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    state
        .enable_local_onboarding(
            onboarding_options(discovered),
            &data_root.join("workspaces"),
            &data_root.join("agent-homes"),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    // Discovery reads the agents directory, so what a person installs
    // from the product is there the next time it is asked. Without
    // this the shell answers from the list it was given at startup and
    // calls a freshly installed agent unavailable until a restart.
    state.set_agent_discovery(|| {
        let declared = crate::declared_agents().unwrap_or_default();
        let installed_root = crate::installed_root().unwrap_or_default();
        onboarding_options(swem_host::discover_agents_with(declared, &installed_root))
    });
    state
        .enable_installs(data_root.join("installed"))
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    // The store reads the registry and the catalogs a person adds; both
    // are kept under the data root so a machine that cannot reach them
    // today still sees what it saw.
    // The Store lists what this distribution carries before anything added,
    // the Cycle among them: the one list says what this product is as well
    // as what it can be given.
    let shipped = swem_host::Catalog::parse(include_bytes!("default-catalog.json"))
        .map_err(|error| anyhow::anyhow!("the shipped catalog: {error}"))?;
    state
        .enable_store(&data_root.join("indexes"), vec![shipped])
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    state
        .enable_projects(project_declarations)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    // The MCP servers a person declares from the product. They land in
    // the same declaration map the projects use, so an agent attaches
    // either kind by name - but they are not projects, so they are
    // loaded after the Project space has taken its own list.
    state
        .enable_mcp_catalogue(
            &data_root.join("mcp-servers"),
            std::sync::Arc::clone(&declarations),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    // The places a model is served from, set up once and named by profiles.
    // The product ships a few; a person's own live beside them as documents.
    state
        .enable_model_providers(&data_root.join("model-providers"))
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    // A project is served by the Cycle, which is not this binary: it is the
    // `swem-cycle` hub installed from the Store, beside this binary, or on
    // PATH, and it answers `serve --projects … [--plugins …] --tools …
    // --environment-root …`. Without one, the product runs as an agent
    // harness alone and says so when a project is asked for.
    match cycle_server(&data_root) {
        Some(command) => {
            // The hub: one server over the projects directory, which every
            // project's own server is started from. Files, not values, so a
            // tool installed or a secret added later is read at the next
            // close without a restart.
            let hub = agent_client_protocol::schema::v1::McpServerStdio::new("swem-cycle", command)
                .args(vec![
                    "serve".to_owned(),
                    "--projects".to_owned(),
                    projects_root.display().to_string(),
                    "--plugins".to_owned(),
                    data_root.join("plugins").display().to_string(),
                    "--tools".to_owned(),
                    swem_host::tools_file(&data_root.join("installed"))
                        .display()
                        .to_string(),
                    "--environment-root".to_owned(),
                    data_root.join("environments").display().to_string(),
                ]);
            state
                .enable_project_creation(swem_host::ProjectFactory {
                    root: projects_root,
                    hub,
                    plugins: vec![data_root.join("plugins")],
                    packages_home: data_root.join("plugins"),
                    attachments: std::sync::Arc::clone(&declarations),
                })
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        }
        None => eprintln!(
            "no Cycle server on this machine: projects are not created here until `swem-cycle` \
             is installed from the Store, put beside this binary, or put on PATH"
        ),
    }
    // The Apps bridge ships inside this binary, so the observer the
    // App relay needs is always configured.
    state.set_mcp_observer_command(
        std::env::current_exe()?,
        vec!["mcp".into(), "observe-stdio".into()],
    );
    Ok((state, data_root))
}

/// The editor door. The same product as the Workbench, answered over stdio to
/// whatever started this process.
///
/// Nothing is printed on stdout: an editor is speaking JSON-RPC on it, and a
/// line of ours would be a protocol error. What a person needs to read goes to
/// stderr, which is where an editor shows an agent's output.
async fn run_acp(acp: Acp) -> Result<()> {
    let (state, _data_root) = assemble_product(
        acp.inventory,
        acp.ledger,
        acp.operation_timeout_secs,
        &[],
        acp.container_image,
    )?;
    // No session secret and no clock: this door serves no HTTP for a secret to
    // guard, and a standing instruction belongs to the product a person left
    // running, not to an editor that happens to be open.
    let state = std::sync::Arc::new(state);
    eprintln!("SWEM is answering as the agent of profile {}", acp.profile);
    swem_host::serve_editor_door(state, acp.profile)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
}

fn workbench_data_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(root).join("SWEM").join("workbench"));
    }
    if let Some(root) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(root).join("swem").join("workbench"));
    }
    let home = std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .context("LOCALAPPDATA, XDG_DATA_HOME and HOME are unavailable")?;
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("swem")
        .join("workbench"))
}

/// The Cycle server this machine has: the newest one installed from the
/// Store under `installed/servers/swem-cycle/`, else `swem-cycle` on PATH.
fn cycle_server(data_root: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "swem-cycle.exe"
    } else {
        "swem-cycle"
    };
    // A distribution is a directory: the hub beside this binary comes first.
    if let Ok(executable) = std::env::current_exe()
        && let Some(beside) = executable.parent().map(|directory| directory.join(name))
        && beside.is_file()
    {
        return Some(beside);
    }
    let installed =
        swem_host::load_receipts(&data_root.join("installed"), swem_host::InstallKind::Server);
    if let Some(receipt) = installed.get("swem-cycle")
        && let Some(executable) = &receipt.executable
    {
        return Some(executable.clone());
    }
    std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn open_system_browser(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("explorer.exe");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url).spawn().map(|_| ())
}

fn run_profiles(profiles: Profiles) -> Result<()> {
    match profiles.command {
        ProfilesCommand::Create { inventory, input } => {
            let profile: PersonalAgentProfile = serde_json::from_slice(
                &fs::read(&input)
                    .with_context(|| format!("read profile input {}", input.display()))?,
            )?;
            let store = PersonalAgentProfileStore::open(&inventory)?;
            let path = store.create(&profile)?;
            println!("{}", path.display());
            Ok(())
        }
        ProfilesCommand::List { inventory } => {
            let profiles = PersonalAgentProfileStore::open(&inventory)?.list()?;
            println!("{}", serde_json::to_string_pretty(&profiles)?);
            Ok(())
        }
        ProfilesCommand::Select {
            profile_id,
            inventory,
        } => {
            let profile = PersonalAgentProfileStore::open(&inventory)?.select(&profile_id)?;
            println!("{}", serde_json::to_string_pretty(&profile)?);
            Ok(())
        }
    }
}

fn run_environments(environments: Environments) -> Result<()> {
    match environments.command {
        EnvironmentsCommand::Probe => {
            let mut probes = vec![swem_host::BackendProbe::direct()];
            probes.extend(probe_podman_endpoints());
            println!("{}", serde_json::to_string_pretty(&probes)?);
            Ok(())
        }
        EnvironmentsCommand::PlanPodman {
            machine_name,
            cpus,
            memory_mib,
            disk_gib,
            rootful,
            user_mode_networking,
            managed,
            output,
        } => {
            let probe = probe_podman();
            let plan = podman_provisioning_plan(
                &probe,
                &PodmanProvisioningOptions {
                    machine_name,
                    cpus,
                    memory_mib,
                    disk_gib,
                    rootful,
                    user_mode_networking,
                    disposition: if managed {
                        ProvisioningDisposition::Managed
                    } else {
                        ProvisioningDisposition::Confirm
                    },
                },
            )?;
            let json = serde_json::to_vec_pretty(&plan)?;
            if let Some(path) = output {
                fs::write(&path, &json)
                    .with_context(|| format!("write provisioning plan {}", path.display()))?;
            }
            println!("{}", String::from_utf8(json)?);
            Ok(())
        }
        EnvironmentsCommand::ApplyPodman {
            plan,
            confirm_plan,
            managed_policy,
        } => {
            let plan: ProvisioningPlan = serde_json::from_slice(
                &fs::read(&plan)
                    .with_context(|| format!("read provisioning plan {}", plan.display()))?,
            )?;
            let authorization = if let Some(plan_id) = confirm_plan {
                ProvisioningAuthorization::confirmed(plan_id)
            } else if let Some(path) = managed_policy {
                let policy: ManagedProvisioningPolicy = serde_json::from_slice(
                    &fs::read(&path)
                        .with_context(|| format!("read managed policy {}", path.display()))?,
                )?;
                ProvisioningAuthorization::managed(policy)
            } else {
                ProvisioningAuthorization::none()
            };
            let receipt = apply_podman_provisioning_plan(&plan, &authorization)?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(())
        }
        EnvironmentsCommand::PlanPodmanImage {
            endpoint,
            reference,
            platform,
            output,
        } => plan_podman_image(endpoint.as_deref(), reference, platform, output),
        EnvironmentsCommand::ApplyPodmanImage { plan, confirm_plan } => {
            apply_podman_image(&plan, confirm_plan)
        }
        EnvironmentsCommand::PlanPodmanReconciliation {
            endpoint,
            active_instances,
            output,
        } => plan_podman_reconciliation(endpoint.as_deref(), active_instances, output),
        EnvironmentsCommand::ApplyPodmanReconciliation { plan, confirm_plan } => {
            apply_podman_reconciliation(&plan, confirm_plan)
        }
    }
}

fn plan_podman_image(
    endpoint: Option<&str>,
    reference: String,
    platform: String,
    output: Option<PathBuf>,
) -> Result<()> {
    let probe = select_podman_endpoint(endpoint)?;
    let plan = podman_image_bootstrap_plan(
        &probe,
        &PodmanImageBootstrapSpec {
            reference,
            platform,
            disposition: ProvisioningDisposition::Confirm,
        },
    )?;
    print_and_optionally_write_plan(&plan, output, "image bootstrap")
}

fn apply_podman_image(plan_path: &Path, confirm_plan: String) -> Result<()> {
    let plan: PodmanImageBootstrapPlan = read_plan(plan_path, "image bootstrap")?;
    let receipt = apply_podman_image_bootstrap_plan(
        &plan,
        &ProvisioningAuthorization::confirmed(confirm_plan),
    )?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}

fn plan_podman_reconciliation(
    endpoint: Option<&str>,
    active_instances: Vec<String>,
    output: Option<PathBuf>,
) -> Result<()> {
    let probe = select_podman_endpoint(endpoint)?;
    let plan = podman_orphan_reconciliation_plan(&probe, active_instances.into_iter().collect())?;
    print_and_optionally_write_plan(&plan, output, "reconciliation")
}

fn apply_podman_reconciliation(plan_path: &Path, confirm_plan: String) -> Result<()> {
    let plan: PodmanOrphanReconciliationPlan = read_plan(plan_path, "reconciliation")?;
    let receipt = apply_podman_orphan_reconciliation_plan(
        &plan,
        &ProvisioningAuthorization::confirmed(confirm_plan),
    )?;
    println!("{}", serde_json::to_string_pretty(&receipt)?);
    Ok(())
}

fn read_plan<T: serde::de::DeserializeOwned>(path: &Path, kind: &str) -> Result<T> {
    serde_json::from_slice(
        &fs::read(path).with_context(|| format!("read {kind} plan {}", path.display()))?,
    )
    .map_err(Into::into)
}

fn print_and_optionally_write_plan<T: serde::Serialize>(
    plan: &T,
    output: Option<PathBuf>,
    kind: &str,
) -> Result<()> {
    let json = serde_json::to_vec_pretty(plan)?;
    if let Some(path) = output {
        fs::write(&path, &json).with_context(|| format!("write {kind} plan {}", path.display()))?;
    }
    println!("{}", String::from_utf8(json)?);
    Ok(())
}

fn select_podman_endpoint(requested: Option<&str>) -> Result<BackendProbe> {
    let endpoints = probe_podman_endpoints();
    if let Some(endpoint_id) = requested {
        return endpoints
            .into_iter()
            .find(|probe| probe.endpoint_id == endpoint_id)
            .with_context(|| format!("Podman endpoint {endpoint_id} was not discovered"));
    }
    let mut ready = endpoints
        .into_iter()
        .filter(|probe| probe.status == BackendProbeStatus::Ready);
    let Some(selected) = ready.next() else {
        bail!("no ready Podman endpoint; inspect `swem environments probe`");
    };
    if let Some(other) = ready.next() {
        bail!(
            "multiple ready Podman endpoints (including {} and {}); pass --endpoint",
            selected.endpoint_id,
            other.endpoint_id
        );
    }
    Ok(selected)
}

async fn run_mcp(mcp: Mcp) -> Result<()> {
    match mcp.command {
        McpCommand::ObserveStdio {
            server_name,
            command,
        } => {
            let mut command = command.into_iter();
            let executable = PathBuf::from(command.next().expect("clap requires a child command"));
            swem_host::mcp_observer::run_ephemeral_stdio_observer(
                swem_host::mcp_observer::StdioObserverConfig {
                    server_name,
                    command: executable,
                    command_args: command.collect(),
                },
            )
            .await
            .map_err(anyhow::Error::msg)
        }
    }
}

/// The agents this person declared, on top of the ones this build knows
/// about: a declaration says where an ACP agent lives, so an agent the
/// product never heard of is still theirs to use.
/// What the onboarding surface lists, from what discovery found. An agent is
/// offered only when it is installed and the host can actually launch it.
fn onboarding_options(
    discovered: Vec<swem_host::AgentDiscovery>,
) -> Vec<swem_host::WorkbenchAgentOption> {
    discovered
        .into_iter()
        .map(|agent| swem_host::WorkbenchAgentOption {
            agent_id: agent.id,
            name: agent.name,
            readiness: agent.readiness,
            available: matches!(
                agent.readiness,
                Readiness::InstalledUnverified | Readiness::HandshakeReady
            ) && agent.launch.is_some()
                && agent.executable_path.is_some(),
        })
        .collect()
}

fn declared_agents() -> Result<Vec<swem_host::AgentCatalogEntry>> {
    swem_host::declared_agents(&workbench_data_root()?.join("agents"))
        .map_err(|error| anyhow::anyhow!(error))
}

/// Where this product installs things (`<data root>/installed`). The first
/// call on a machine an earlier product installed agents on - into its own
/// directory beside the data root - brings those under this root, so there
/// is one place to look and nothing a person installed is lost.
fn installed_root() -> Result<PathBuf> {
    let root = workbench_data_root()?.join("installed");
    if let Some(legacy) = legacy_agents_home()
        && legacy.is_dir()
    {
        swem_host::adopt_legacy_agents(&legacy, &root)
            .map_err(|error| anyhow::anyhow!("adopt earlier agent installs: {error}"))?;
    }
    Ok(root)
}

/// Where products before 2026-09-25 installed agents: `<data home>/swem/agents`,
/// a sibling of the data root rather than inside it.
fn legacy_agents_home() -> Option<PathBuf> {
    if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(local).join("swem").join("agents"));
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(data).join("swem").join("agents"));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".local/share/swem/agents"))
}

/// Every agent this product can start: the built-in catalogue and the ones
/// a person declared, found on PATH or under the install root.
fn discovered_agents() -> Result<Vec<swem_host::AgentDiscovery>> {
    Ok(swem_host::discover_agents_with(
        declared_agents()?,
        &installed_root()?,
    ))
}

/// Every agent this host can start, as a table or as JSON.
fn list_agents(json: bool) -> Result<()> {
    // The agents this person declared, on top of the ones this build
    // knows about.
    let discovered = discovered_agents()?;
    if json {
        println!("{}", serde_json::to_string_pretty(&discovered)?);
    } else {
        println!("{:<14} {:<22} {:<22} EXECUTABLE", "ID", "NAME", "READINESS");
        for agent in discovered {
            let path = agent
                .executable_path
                .map_or_else(|| "-".into(), |path| path.display().to_string());
            println!(
                "{:<14} {:<22} {:<22} {}",
                agent.id,
                agent.name,
                format!("{:?}", agent.readiness),
                path
            );
        }
    }
    Ok(())
}

async fn run_agents(agents: Agents) -> Result<()> {
    match agents.command {
        AgentsCommand::List { json } => list_agents(json),
        AgentsCommand::PlanInstall { agent_id } => {
            let plan = install_plan(&agent_id)?;
            println!("{}", serde_json::to_string_pretty(&plan)?);
            Ok(())
        }
        AgentsCommand::Install {
            agent_id,
            consent,
            confirm_plan,
        } => install_catalog_agent(&agent_id, consent, confirm_plan.as_deref()),
        AgentsCommand::PlanDistributionImage {
            spec,
            endpoint,
            output,
        } => plan_distribution_image(&spec, endpoint.as_deref(), output),
        AgentsCommand::ApplyDistributionImage {
            plan,
            confirm_plan,
            inventory,
        } => apply_distribution_image(&plan, confirm_plan, &inventory),
        AgentsCommand::ListDistributions { inventory } => list_distributions(&inventory),
        AgentsCommand::Verify { agent_id } => {
            let discovery = discovered_agents()?
                .into_iter()
                .find(|agent| agent.id == agent_id)
                .with_context(|| format!("unknown agent: {agent_id}"))?;
            if discovery.readiness == Readiness::Absent {
                bail!("agent is not installed: {agent_id}");
            }
            let handshake =
                verify_discovered_agent(&discovery, std::time::Duration::from_secs(20)).await?;
            println!("{}", serde_json::to_string_pretty(&handshake)?);
            Ok(())
        }
        AgentsCommand::Session {
            agent_id,
            workspace,
            prompt_file,
            load_session,
            resume_session,
            transcript,
            mcp_server,
            timeout_secs,
        } => {
            run_native_agent_session(
                &agent_id,
                &workspace,
                &prompt_file,
                load_session.as_deref(),
                resume_session.as_deref(),
                &transcript,
                &mcp_server,
                timeout_secs,
            )
            .await
        }
    }
}

fn install_catalog_agent(agent_id: &str, consent: bool, confirm_plan: Option<&str>) -> Result<()> {
    let plan = install_plan(agent_id)?;
    if !consent {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        bail!(
            "re-run with --consent --confirm-plan {} to install {}",
            plan.plan_id,
            plan.registry_id
        );
    }
    if confirm_plan != Some(plan.plan_id.as_str()) {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        bail!("install confirmation does not match the exact current plan_id");
    }
    let node = if matches!(
        plan.distribution,
        swem_host::RegistryDistribution::Npx { .. }
    ) {
        Some(which_node().context("node is required for npx distributions")?)
    } else {
        None
    };
    let installation = swem_host::install(&plan, true, &installed_root()?, node.as_deref())?;
    println!("{}", serde_json::to_string_pretty(&installation)?);
    Ok(())
}

fn plan_distribution_image(
    spec_path: &Path,
    endpoint: Option<&str>,
    output: Option<PathBuf>,
) -> Result<()> {
    let spec: AgentDistributionBuildSpec = serde_json::from_slice(
        &fs::read(spec_path)
            .with_context(|| format!("read distribution spec {}", spec_path.display()))?,
    )?;
    let probe = select_podman_endpoint(endpoint)?;
    let plan = agent_distribution_build_plan(&probe, &spec)?;
    print_and_optionally_write_plan(&plan, output, "distribution build")
}

fn apply_distribution_image(
    plan_path: &Path,
    confirm_plan: String,
    inventory_path: &Path,
) -> Result<()> {
    let plan: AgentDistributionBuildPlan = read_plan(plan_path, "distribution build")?;
    let inventory = AgentDistributionInventory::open(inventory_path)?;
    let outcome = apply_agent_distribution_build_plan(
        &plan,
        &DistributionBuildAuthorization::confirmed(confirm_plan),
        &inventory,
    )?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

fn list_distributions(inventory_path: &Path) -> Result<()> {
    let receipts = AgentDistributionInventory::open(inventory_path)?.list()?;
    println!("{}", serde_json::to_string_pretty(&receipts)?);
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "direct CLI boundary; a second options object would add no semantic boundary"
)]
async fn run_native_agent_session(
    agent_id: &str,
    workspace: &Path,
    prompt_files: &[PathBuf],
    load_session: Option<&str>,
    resume_session: Option<&str>,
    transcript: &Path,
    mcp_server_files: &[PathBuf],
    timeout_secs: u64,
) -> Result<()> {
    let discovery = discovered_agents()?
        .into_iter()
        .find(|agent| agent.id == agent_id)
        .with_context(|| format!("unknown agent: {agent_id}"))?;
    if discovery.readiness == Readiness::Absent {
        bail!("agent is not installed: {agent_id}");
    }
    let handshake = verify_discovered_agent(&discovery, std::time::Duration::from_secs(20)).await?;
    if handshake.readiness != Readiness::HandshakeReady {
        bail!(
            "agent {agent_id} is not handshake-ready: {:?} (reported {:?})",
            handshake.readiness,
            handshake.agent_name
        );
    }
    let launch = discovery
        .launch
        .as_ref()
        .context("discovered agent has no launch command")?;
    let executable = discovery
        .executable_path
        .as_ref()
        .context("discovered agent has no executable path")?;
    let prompts = prompt_files
        .iter()
        .map(|path| fs::read_to_string(path).with_context(|| format!("read {}", path.display())))
        .collect::<Result<Vec<_>>>()?;
    let mut options = NativeSessionOptions::new(std::time::Duration::from_secs(timeout_secs));
    options.transcript_path = Some(std::path::absolute(transcript)?);
    options.mcp_servers = mcp_server_files
        .iter()
        .map(|path| {
            let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_slice::<McpServer>(&bytes)
                .with_context(|| format!("parse ACP MCP server declaration {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    options.start = match (load_session, resume_session) {
        (Some(session_id), None) => NativeSessionStart::Load {
            session_id: session_id.to_owned(),
        },
        (None, Some(session_id)) => NativeSessionStart::Resume {
            session_id: session_id.to_owned(),
        },
        (None, None) => NativeSessionStart::New,
        (Some(_), Some(_)) => unreachable!("clap rejects conflicting session modes"),
    };
    let outcome = run_native_session(
        launch,
        executable,
        &std::path::absolute(workspace)?,
        &prompts,
        &options,
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&outcome)?);
    Ok(())
}

fn which_node() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["node.exe", "node"]
    } else {
        &["node"]
    };
    std::env::split_paths(&path)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| candidate.is_file())
}
