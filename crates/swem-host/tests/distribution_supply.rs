use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::{
    AgentDistributionBuildSpec, AgentDistributionInventory, BackendProbeStatus,
    DistributionBuildAuthorization, DistributionBuildNetwork, ProvisioningDisposition,
    agent_distribution_build_plan, apply_agent_distribution_build_plan, probe_podman_endpoints,
};

#[test]
#[ignore = "requires a ready Podman endpoint and the pre-acquired digest-pinned Node base image"]
fn live_locked_distribution_build_is_inspected_recorded_and_reused() {
    let requested = std::env::var("SWEM_LIVE_PODMAN_ENDPOINT").ok();
    let probe = probe_podman_endpoints()
        .into_iter()
        .find(|probe| {
            probe.status == BackendProbeStatus::Ready
                && requested
                    .as_deref()
                    .is_none_or(|endpoint| probe.endpoint_id == endpoint)
        })
        .expect("ready Podman endpoint");
    let context_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .canonicalize()
        .expect("canonical committed build context");
    let spec = AgentDistributionBuildSpec {
        distribution_id: "claude-agent-acp-linux-amd64-0.70.0-product-supply".into(),
        registry_id: "claude-acp".into(),
        publisher: "agentclientprotocol".into(),
        package: "@agentclientprotocol/claude-agent-acp".into(),
        version: "0.70.0".into(),
        package_integrity: "sha512-Psqj6fhV4pQ8IM480zpJ+xGiMMIqNLxlsTj5Mzn+T8KSURCVNJdl0ktcqLMjgHJC/QnOvDdDkFf3xTW9VIV9aQ==".into(),
        platform: "linux/amd64".into(),
        context_root,
        containerfile: "claude-agent-acp.Containerfile".into(),
        lockfile: "package-lock.json".into(),
        context_files: vec![
            "claude-agent-acp.Containerfile".into(),
            "package.json".into(),
            "package-lock.json".into(),
        ],
        agent_executable: "/usr/local/bin/node".into(),
        agent_args: vec![
            "/opt/swem/agent/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js"
                .into(),
        ],
        network: DistributionBuildNetwork::PrivateEgress,
        disposition: ProvisioningDisposition::Confirm,
    };
    let plan = agent_distribution_build_plan(&probe, &spec).expect("read-only build preview");
    assert!(
        plan.blockers.is_empty(),
        "base image must be acquired separately: {:?}",
        plan.blockers
    );
    assert!(
        plan.command_preview
            .iter()
            .any(|value| value == "--pull=never")
    );
    assert!(plan.command_preview.iter().any(|value| {
        value.ends_with("<staged-context>\\claude-agent-acp.Containerfile")
            || value.ends_with("<staged-context>/claude-agent-acp.Containerfile")
    }));

    let inventory_root = temporary_directory("distribution-inventory");
    let inventory = AgentDistributionInventory::open(&inventory_root).expect("open inventory");
    let authorization = DistributionBuildAuthorization::confirmed(plan.plan_id.clone());
    let first = apply_agent_distribution_build_plan(&plan, &authorization, &inventory)
        .expect("build and inspect locked distribution");
    assert!(!first.reused);
    assert_eq!(first.receipt.plan_id, plan.plan_id);
    assert_eq!(first.receipt.context_sha256, plan.context_sha256);

    let second = apply_agent_distribution_build_plan(&plan, &authorization, &inventory)
        .expect("reinspect and reuse exact distribution");
    assert!(second.reused);
    assert_eq!(second.receipt, first.receipt);
    assert_eq!(
        inventory.list().expect("list immutable receipts"),
        vec![first.receipt]
    );

    fs::remove_dir_all(&inventory_root).expect("remove exact temporary inventory");
}

fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("swem-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir(&root).expect("create exact temporary directory");
    root
}
