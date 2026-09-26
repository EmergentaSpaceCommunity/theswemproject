//! The profile's agent in a container on this machine, prepared before the
//! session starts so the harness takes a connection that already names one
//! exact lease and the transport bound to it.

use std::path::Path;

use agent_client_protocol::schema::v1::McpServer;

use crate::{
    EnvironmentGuarantee, EnvironmentRequirements, EnvironmentTransport, IntegrationKind,
    LaunchCommand, PersonalAgentProfile, PodmanContainerSpec, PodmanWorkspaceBinding,
    ResolvedAgentConnection, ResolvedAgentEnvironment,
};

/// The uid:gid that owns `path`, as a container user.
///
/// A container environment exists to keep an agent out of the machine, so
/// running it as the machine's root would give away the thing it was chosen
/// for; the owner being root is refused with a sentence a person can act on.
#[cfg(unix)]
fn owner_of(path: &Path) -> Result<String, String> {
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
fn owner_of(_path: &Path) -> Result<String, String> {
    Err("a container environment is a Linux container, and this is not a Linux host".into())
}

/// The profile's agent, in a container on this machine.
///
/// # Errors
///
/// No image was named, no Podman endpoint answers, the workspace belongs to
/// root, the model provider cannot be read, or the container cannot be
/// prepared or reached - each in a sentence naming what to do.
pub(super) fn in_a_container(
    profile: &PersonalAgentProfile,
    image: Option<&str>,
    agent_in_image: &str,
    mcp_servers: Vec<McpServer>,
    providers_root: &Path,
) -> Result<ResolvedAgentConnection, String> {
    let image = image.ok_or_else(|| {
        format!(
            "this profile runs in a container, and this product was not told which image to run \
             an agent in. Start it with --container-image sha256:<id> (or SWEM_CONTAINER_IMAGE), \
             naming an image that runs an ACP agent at {agent_in_image}"
        )
    })?;
    let probe = crate::probe_podman_endpoints()
        .into_iter()
        .find(|probe| probe.status == crate::BackendProbeStatus::Ready)
        .ok_or_else(|| {
            let probe = crate::probe_podman();
            let said = probe
                .diagnostics
                .first()
                .cloned()
                .unwrap_or_else(|| "no Podman endpoint answered".to_owned());
            format!("this profile runs in a container, and this machine cannot: {said}")
        })?;

    let mut requirements = EnvironmentRequirements::new(&profile.workspace);
    requirements.cpu_limit = Some(2);
    requirements.memory_mib = Some(2_048);
    requirements.required_guarantees.extend([
        EnvironmentGuarantee::AgentProcessIsolation,
        EnvironmentGuarantee::FilesystemIsolation,
        EnvironmentGuarantee::NetworkDenyByDefault,
        EnvironmentGuarantee::ResourceLimits,
    ]);

    let mut spec = PodmanContainerSpec::deny_network(
        image,
        PodmanWorkspaceBinding {
            service_source: profile.workspace.display().to_string(),
            container_target: "/workspace".into(),
        },
        agent_in_image,
    );
    // The agent keeps whatever it keeps between sessions, and it keeps it on
    // the person's disk rather than inside a container that will be removed.
    spec.agent_home = Some(PodmanWorkspaceBinding {
        service_source: profile.agent_home.display().to_string(),
        container_target: "/home/swem".into(),
    });
    // Who the agent is inside the container: whoever owns the directory it
    // works in. A container user that is not that owner sees the person's own
    // workspace as read-only, which surfaces as "Permission denied" on a path
    // that plainly exists - and a container run as root is refused outright.
    spec.user = owner_of(&profile.workspace)?;
    // The model and the provider's address, as plain variables of the agent
    // process - the same ones a direct launch gets. A key never goes this
    // way: keys are runtime secrets.
    let provider = match &profile.model_provider {
        Some(id) => Some(
            crate::workbench_shell::ModelProviderBook::open(providers_root)
                .and_then(|book| book.get(id))
                .map_err(|error| error.to_string())?,
        ),
        None => None,
    };
    spec.environment =
        crate::agent_setup::materialise_profile(profile, provider.as_ref())?.environment;

    let lease_id = format!(
        "workbench-container-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    );
    let lease = crate::prepare_podman_lease(&probe, lease_id, &requirements, &spec)
        .map_err(|error| format!("this profile's container could not be prepared: {error}"))?;
    let transport = EnvironmentTransport::podman_endpoint(&lease, &probe)
        .map_err(|error| format!("this profile's container could not be reached: {error}"))?;

    Ok(ResolvedAgentConnection {
        launch: LaunchCommand {
            executable: agent_in_image.into(),
            args: Vec::new(),
            integration: IntegrationKind::DirectAcp,
        },
        agent_executable: std::path::PathBuf::from(agent_in_image),
        mcp_servers,
        environment: ResolvedAgentEnvironment::Prepared {
            environment_profile_id: profile.environment_profile_id.clone(),
            lease: Box::new(lease),
            transport: Box::new(transport),
        },
    })
}
