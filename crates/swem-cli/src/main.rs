use std::fs;
use std::path::{Path, PathBuf};

use agent_client_protocol::schema::v1::{McpServer, McpServerStdio};
use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};
use swem_host::product::{DataRoot, Product};
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
    Time(Time),
    Workbench(Workbench),
}

#[derive(Debug, Args)]
struct Time {
    #[command(subcommand)]
    command: TimeCommand,
}

#[derive(Debug, Subcommand)]
enum TimeCommand {
    /// Keep time once and leave: say what is due to the agents whose time
    /// the system's scheduler keeps, have it answered, and stop. The
    /// system's scheduler starts this when it was turned on under
    /// Providers, Time; a Workbench that is open keeps time itself, and
    /// this leaves at once.
    Keep {
        /// Deadline for each individual ACP operation.
        #[arg(long, default_value_t = 600)]
        operation_timeout_secs: u64,
    },
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
        /// Serve at this address, for whoever comes from elsewhere:
        /// `https://workbench.example.org`. Whoever comes signs in with a
        /// passkey. Needs `--tls-cert` and `--tls-key`, or `--behind-proxy`.
        /// `http://localhost:<port>` tries sign-in on this machine.
        #[arg(long)]
        at: Option<String>,
        /// Where to listen when served at an address. With a certificate
        /// the default is every interface, at the port of the address; with
        /// a proxy in front it is this machine alone, at `--port`.
        #[arg(long, requires = "at")]
        listen: Option<std::net::SocketAddr>,
        /// The certificate the address is served with, and what stands
        /// before it, as one PEM file.
        #[arg(long, requires_all = ["at", "tls_key"])]
        tls_cert: Option<PathBuf>,
        /// The key of that certificate, as a PEM file.
        #[arg(long, requires_all = ["at", "tls_cert"])]
        tls_key: Option<PathBuf>,
        /// A proxy of yours stands in front on this machine and speaks TLS;
        /// the Workbench listens on this machine alone.
        #[arg(long, requires = "at", conflicts_with = "tls_cert")]
        behind_proxy: bool,
        /// Where a browser finds what is drawn for Apps: an address of its
        /// own. With a certificate the default is the same name at the
        /// next port.
        #[arg(long, requires = "at")]
        apps_at: Option<String>,
        /// Where what is drawn for Apps is listened for.
        #[arg(long, requires = "at")]
        apps_listen: Option<std::net::SocketAddr>,
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
    /// An agent's own schedules, served to its session over stdio. The
    /// Workbench starts this; it is handed to an engine with a session.
    #[command(hide = true)]
    TimeTools {
        #[arg(long)]
        ledger: PathBuf,
        #[arg(long)]
        agent: String,
        #[arg(long)]
        chat: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let command = Cli::parse()
        .command
        .unwrap_or(Command::Workbench(Workbench { command: None }));
    match command {
        Command::Agents(agents) => run_agents(agents).await,
        Command::Time(time) => run_time(time).await,
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
        at: None,
        listen: None,
        tls_cert: None,
        tls_key: None,
        behind_proxy: false,
        apps_at: None,
        apps_listen: None,
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
            at,
            listen,
            tls_cert,
            tls_key,
            behind_proxy,
            apps_at,
            apps_listen,
            acp_registry,
            container_image,
        } => {
            if let Some(address) = at {
                let asked = asked_to_serve_at(
                    &address,
                    AskedAt {
                        port,
                        sandbox_port,
                        listen,
                        certificate: tls_cert.zip(tls_key),
                        behind_proxy,
                        apps_at,
                        apps_listen,
                        apps_bundle,
                    },
                )?;
                let here = asked.closed == swem_host::product::Closed::ThisMachine;
                let served = product(
                    inventory,
                    ledger,
                    operation_timeout_secs,
                    &mcp_server,
                    acp_registry,
                    container_image,
                )?
                .assemble()
                .map_err(anyhow::Error::msg)?
                .serve_at(asked.clone())
                .await
                .map_err(anyhow::Error::msg)?;
                println!("SWEM Workbench: {}", served.address);
                println!(
                    "Apps are drawn at: {}",
                    asked.apps_address.origin().ascii_serialization()
                );
                println!(
                    "It listens on {} and, for Apps, on {}.",
                    served.handle.local_addr, served.handle.sandbox_addr
                );
                if let Some(until) = served.certificate_good_until {
                    println!(
                        "Its certificate is good for {} more days.",
                        until
                            .duration_since(std::time::SystemTime::now())
                            .map_or(0, |left| left.as_secs() / 86_400)
                    );
                }
                match &served.word {
                    Some(word) => println!(
                        "This Workbench belongs to nobody yet. Open the address and give it \
                         this word, which is used once:\n\n    {word}\n"
                    ),
                    None => println!("Whoever comes signs in with a device that was registered."),
                }
                if here
                    && !no_open
                    && let Err(error) = open_system_browser(&served.address)
                {
                    eprintln!("could not open the system browser: {error}");
                }
                std::future::pending::<()>().await;
                return Ok(());
            }
            let served = product(
                inventory,
                ledger,
                operation_timeout_secs,
                &mcp_server,
                acp_registry,
                container_image,
            )?
            .assemble()
            .map_err(anyhow::Error::msg)?
            .serve(
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
            println!("SWEM Workbench: {}", served.url);
            println!(
                "App sandbox origin: http://127.0.0.1:{}",
                served.handle.sandbox_addr.port()
            );
            if !no_open && let Err(error) = open_system_browser(&served.url) {
                eprintln!("could not open the system browser: {error}");
            }
            std::future::pending::<()>().await;
            Ok(())
        }
    }
}

/// What the command was told about serving at an address.
struct AskedAt {
    port: u16,
    sandbox_port: u16,
    listen: Option<std::net::SocketAddr>,
    certificate: Option<(PathBuf, PathBuf)>,
    behind_proxy: bool,
    apps_at: Option<String>,
    apps_listen: Option<std::net::SocketAddr>,
    apps_bundle: Option<PathBuf>,
}

/// What is served, from what the command was told, with what it was not
/// told filled in. Whether that may be served at all is the product's to
/// say, and it says it when it is asked to serve.
fn asked_to_serve_at(address: &str, asked: AskedAt) -> Result<swem_host::product::ServeAt> {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use swem_host::product::{Closed, ServeAt};

    let read = |what: &str, address: &str| {
        url::Url::parse(address).map_err(|error| {
            anyhow!("{what} is given as https://workbench.example.org; {address} is not an address: {error}")
        })
    };
    let address = read("The address", address)?;
    let closed = match (asked.certificate, asked.behind_proxy) {
        (Some((chain, key)), _) => Closed::Certificate { chain, key },
        (None, true) => Closed::ProxyInFront,
        (None, false) => Closed::ThisMachine,
    };
    let with_certificate = matches!(closed, Closed::Certificate { .. });
    let listen = asked.listen.unwrap_or_else(|| {
        if with_certificate {
            SocketAddr::new(
                IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                address.port_or_known_default().unwrap_or(443),
            )
        } else if closed == Closed::ThisMachine {
            SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                address.port_or_known_default().unwrap_or(asked.port),
            )
        } else {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), asked.port)
        }
    });
    let next = |port: u16| port.checked_add(1).unwrap_or(port - 1);
    let apps_address = match asked.apps_at {
        Some(apps) => read("Where Apps are drawn", &apps)?,
        None if closed == Closed::ProxyInFront => bail!(
            "With a proxy in front, say where Apps are drawn: --apps-at https://apps.<name>, a \
             second name your proxy sends to where Apps are listened for (--apps-listen)."
        ),
        None => {
            let mut apps = address.clone();
            let port = if asked.sandbox_port == 0 {
                next(address.port_or_known_default().unwrap_or(listen.port()))
            } else {
                asked.sandbox_port
            };
            apps.set_port(Some(port))
                .map_err(|()| anyhow!("{address} cannot be given a port"))?;
            apps
        }
    };
    let apps_listen = asked.apps_listen.unwrap_or_else(|| {
        let port = if with_certificate || closed == Closed::ThisMachine {
            apps_address
                .port_or_known_default()
                .unwrap_or(next(listen.port()))
        } else if asked.sandbox_port == 0 {
            next(listen.port())
        } else {
            asked.sandbox_port
        };
        SocketAddr::new(listen.ip(), port)
    });
    Ok(ServeAt {
        address,
        listen,
        closed,
        apps_address,
        apps_listen,
        apps_bundle: asked.apps_bundle,
    })
}

async fn run_time(time: Time) -> Result<()> {
    match time.command {
        TimeCommand::Keep {
            operation_timeout_secs,
        } => {
            // Most looks find nothing: that is told from the ledger alone.
            let root = DataRoot::for_this_machine().map_err(anyhow::Error::msg)?;
            if root.nothing_to_keep().is_some() {
                return Ok(());
            }
            let look = product(None, None, operation_timeout_secs, &[], None, None)?
                .assemble()
                .map_err(anyhow::Error::msg)?
                .keep_time_once()
                .await
                .map_err(anyhow::Error::msg)?;
            if look.said > 0 {
                println!("said {} on time", look.said);
            }
            Ok(())
        }
    }
}

/// The product this binary is: the harness crate's builder over this
/// machine's data root, with what this distribution adds - its shipped
/// catalog, the observer command, and the Cycle hub beside it when there is
/// one. One function because there are two doors onto the same product - the
/// Workbench over HTTP and the editor door over stdio - and a second assembly
/// would be a second product wearing the same name.
fn product(
    inventory: Option<PathBuf>,
    ledger: Option<PathBuf>,
    operation_timeout_secs: u64,
    mcp_server: &[PathBuf],
    acp_registry: Option<String>,
    container_image: Option<String>,
) -> Result<Product> {
    let root = DataRoot::for_this_machine().map_err(anyhow::Error::msg)?;
    let shipped = swem_host::Catalog::parse(include_bytes!("default-catalog.json"))
        .map_err(|error| anyhow::anyhow!("the shipped catalog: {error}"))?;
    let mut product = Product::at(root.clone())
        .operation_timeout(std::time::Duration::from_secs(operation_timeout_secs))
        .shipped_catalog(shipped)
        .container_image(container_image)
        // The Apps bridge ships inside this binary, so the observer the
        // App relay needs is always configured.
        .mcp_observer(
            std::env::current_exe()?,
            vec!["mcp".into(), "observe-stdio".into()],
        )
        // And so does what serves an agent its own schedules.
        .time_tools(
            std::env::current_exe()?,
            vec!["mcp".into(), "time-tools".into()],
        )
        // And what the system's scheduler starts to keep time once.
        .keep_time(std::env::current_exe()?, vec!["time".into(), "keep".into()]);
    // Projects are served by the Cycle, which is not this binary: it is the
    // `swem-cycle` hub installed from the Store, beside this binary, or on
    // PATH. When there is one it is a server of this product like any other:
    // its home App is its space on the Workbench and an agent attaches it by
    // name. Without one the product is an agent harness alone.
    if let Some(hub) = cycle_hub(&root) {
        product = product.declare(hub);
    }
    if let Some(inventory) = inventory {
        product = product.profiles_at(inventory);
    }
    if let Some(ledger) = ledger {
        product = product.routes_at(ledger);
    }
    if let Some(index) = acp_registry {
        product = product.acp_registry(index);
    }
    for path in mcp_server {
        let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
        let server: McpServer = serde_json::from_slice(&bytes)
            .with_context(|| format!("parse ACP MCP server declaration {}", path.display()))?;
        product = product.declare(server);
    }
    Ok(product)
}

/// The Cycle hub, when this machine has one, and how it is started:
/// `swem-cycle serve` over directories of this product's data root. Where
/// they are is this product's to say - the harness keeps no place for
/// projects - and they are files, not values, so a tool installed or a secret
/// added later is read at the next close without a restart.
fn cycle_hub(root: &DataRoot) -> Option<McpServer> {
    let command = cycle_server(root)?;
    let under = |name: &str| root.path().join(name).display().to_string();
    Some(McpServer::Stdio(
        McpServerStdio::new("swem-cycle", command).args(vec![
            "serve".to_owned(),
            "--projects".to_owned(),
            under("projects"),
            "--plugins".to_owned(),
            under("plugins"),
            "--packages-home".to_owned(),
            under("plugins"),
            "--tools".to_owned(),
            root.installed()
                .join("tools")
                .join("tools.json")
                .display()
                .to_string(),
            "--environment-root".to_owned(),
            root.environments().display().to_string(),
        ]),
    ))
}

/// The editor door. The same product as the Workbench, answered over stdio to
/// whatever started this process.
///
/// Nothing is printed on stdout: an editor is speaking JSON-RPC on it, and a
/// line of ours would be a protocol error. What a person needs to read goes to
/// stderr, which is where an editor shows an agent's output.
async fn run_acp(acp: Acp) -> Result<()> {
    let assembled = product(
        acp.inventory,
        acp.ledger,
        acp.operation_timeout_secs,
        &[],
        None,
        acp.container_image,
    )?
    .assemble()
    .map_err(anyhow::Error::msg)?;
    // No session secret and no clock: this door serves no HTTP for a secret to
    // guard, and a standing instruction belongs to the product a person left
    // running, not to an editor that happens to be open.
    eprintln!("SWEM is answering as the agent of profile {}", acp.profile);
    assembled
        .editor_door(acp.profile)
        .await
        .map_err(anyhow::Error::msg)
}

fn workbench_data_root() -> Result<PathBuf> {
    DataRoot::for_this_machine()
        .map(|root| root.path().to_path_buf())
        .map_err(anyhow::Error::msg)
}

/// The Cycle server this machine has: the newest one installed from the
/// Store under `installed/servers/swem-cycle/`, else `swem-cycle` on PATH.
fn cycle_server(root: &DataRoot) -> Option<PathBuf> {
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
    let installed = swem_host::load_receipts(&root.installed(), swem_host::InstallKind::Server);
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
        McpCommand::TimeTools {
            ledger,
            agent,
            chat,
        } => swem_host::serve_time_tools(ledger, agent, chat)
            .await
            .map_err(anyhow::Error::msg),
    }
}

/// The agents this person declared, on top of the ones this build knows
/// about: a declaration says where an ACP agent lives, so an agent the
/// product never heard of is still theirs to use.
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
