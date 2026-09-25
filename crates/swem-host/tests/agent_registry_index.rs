//! A person whose machine cannot reach the public registry can still install
//! an agent.
//!
//! The index used to be a constant, so "the registry is unreachable" meant
//! "no agent can be installed here" - for a person behind a corporate proxy,
//! on a disconnected network, or reading a mirror their organisation keeps,
//! exactly as much as for a gate in a container. Naming the index changes
//! nothing else: the plan is still resolved from the index, carries its own
//! content id, and is what the install has to be confirmed against.

use std::fs;
use std::path::PathBuf;

use swem_host::{ACP_REGISTRY_INDEX, RegistryDistribution, resolve_registry_install_plan};

fn index_file(body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "swem-acp-index-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::write(&path, body).expect("write the index");
    path
}

#[test]
fn a_mirror_a_person_names_resolves_the_same_shape_of_plan() {
    let path = index_file(
        r#"{"agents":[{"id":"claude-acp","name":"Claude Code (ACP adapter)","version":"0.79.0",
             "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp","args":[]}}}]}"#,
    );
    let plan = resolve_registry_install_plan(
        "claude-code",
        "claude-acp",
        &format!("file://{}", path.display()),
    )
    .expect("a named index resolves a plan");

    assert_eq!(plan.agent_id, "claude-code");
    assert_eq!(plan.version, "0.79.0");
    assert!(
        plan.requires_explicit_consent,
        "naming the index does not make the install silent"
    );
    assert!(
        !plan.executes_remote_shell,
        "naming the index does not let the index run a shell"
    );
    let RegistryDistribution::Npx { package, .. } = &plan.distribution else {
        panic!("the npx distribution of the index: {:?}", plan.distribution);
    };
    assert_eq!(package, "@agentclientprotocol/claude-agent-acp");

    // The plan's content id covers the index it came from, so consent given
    // for a plan read from one index is not consent for the same agent read
    // from another.
    let elsewhere = index_file(
        r#"{"agents":[{"id":"claude-acp","name":"Claude Code (ACP adapter)","version":"0.79.0",
             "distribution":{"npx":{"package":"@agentclientprotocol/claude-agent-acp","args":[]}}}]}"#,
    );
    let same_bytes = resolve_registry_install_plan(
        "claude-code",
        "claude-acp",
        &format!("file://{}", elsewhere.display()),
    )
    .expect("the other index resolves too");
    assert_ne!(
        plan.plan_id, same_bytes.plan_id,
        "the same agent from a different index is a different plan to consent to"
    );

    fs::remove_file(path).ok();
    fs::remove_file(elsewhere).ok();
}

/// And the default is still the public one, so naming an index is a choice a
/// person makes rather than something that happened to them.
#[test]
fn the_default_index_is_the_public_registry() {
    assert_eq!(
        swem_host::acp_registry_index(),
        ACP_REGISTRY_INDEX,
        "with nothing named, SWEM reads the registry it always read"
    );
}
