use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::{
    AGENT_DISTRIBUTION_RECEIPT_SCHEMA, AgentDistributionReceipt, AttachmentBinding,
    AttachmentTransport, CredentialBindingRef, CredentialSourceRef, PERSONAL_AGENT_PROFILE_SCHEMA,
    PersonalAgentProfile, PersonalAgentProfileStore, ProfileError,
};

#[test]
fn profile_survives_store_reopen_without_connection_or_secret_state() {
    let fixture = FixtureDirectory::new("profile-roundtrip");
    let workspace = fixture.create_directory("workspace");
    let agent_home = fixture.create_directory("agent-home");
    let inventory = fixture.create_directory("inventory");
    let profile = PersonalAgentProfile::new(
        "claude-main",
        "claude-code",
        "claude-agent-acp-linux-amd64-0.70.0",
        "podman-private-egress",
        "interactive-surface",
        &workspace,
        &agent_home,
        vec![AttachmentBinding::new(
            "echo-local",
            "echo-capability",
            AttachmentTransport::Stdio,
        )],
        vec![CredentialBindingRef {
            binding_id: "claude-subscription".into(),
            source: CredentialSourceRef::EnvironmentVariable {
                name: "CLAUDE_CODE_OAUTH_TOKEN".into(),
            },
            target_environment: "CLAUDE_CODE_OAUTH_TOKEN".into(),
        }],
    )
    .expect("build canonical personal-agent profile");
    assert_eq!(profile.schema, PERSONAL_AGENT_PROFILE_SCHEMA);

    let first_process = PersonalAgentProfileStore::open(&inventory).expect("open inventory");
    let path = first_process.create(&profile).expect("create profile once");
    drop(first_process);

    let durable_bytes = fs::read_to_string(&path).expect("read durable profile");
    assert!(!durable_bytes.contains("secret-value"));
    assert!(!durable_bytes.contains("agent_args"));
    assert!(!durable_bytes.contains("native_session_id"));
    assert!(!durable_bytes.contains("lease_id"));
    assert!(!durable_bytes.contains("mcp-live-echo.mjs"));

    let second_process = PersonalAgentProfileStore::open(&inventory).expect("reopen inventory");
    assert_eq!(
        second_process
            .load("claude-main")
            .expect("load after cold process boundary"),
        profile
    );
    assert!(matches!(
        second_process.create(&profile),
        Err(ProfileError::ProfileExists(id)) if id == "claude-main"
    ));
}

#[test]
fn profile_rejects_ambiguous_bindings_and_path_escape_ids() {
    let fixture = FixtureDirectory::new("profile-negative");
    let workspace = fixture.create_directory("workspace");
    let agent_home = fixture.create_directory("agent-home");
    let duplicate = PersonalAgentProfile::new(
        "../escape",
        "claude-code",
        "distribution",
        "environment",
        "permission",
        &workspace,
        &agent_home,
        vec![],
        vec![],
    );
    assert!(matches!(duplicate, Err(ProfileError::InvalidProfile(_))));

    let duplicate_targets = PersonalAgentProfile::new(
        "safe-profile",
        "claude-code",
        "distribution",
        "environment",
        "permission",
        &workspace,
        &agent_home,
        vec![],
        vec![
            CredentialBindingRef {
                binding_id: "first".into(),
                source: CredentialSourceRef::EnvironmentVariable {
                    name: "FIRST_TOKEN".into(),
                },
                target_environment: "CLAUDE_CODE_OAUTH_TOKEN".into(),
            },
            CredentialBindingRef {
                binding_id: "second".into(),
                source: CredentialSourceRef::EnvironmentVariable {
                    name: "SECOND_TOKEN".into(),
                },
                target_environment: "CLAUDE_CODE_OAUTH_TOKEN".into(),
            },
        ],
    );
    assert!(matches!(
        duplicate_targets,
        Err(ProfileError::InvalidProfile(_))
    ));
}

#[test]
fn profile_inventory_lists_stably_and_selects_without_global_state() {
    let fixture = FixtureDirectory::new("profile-list-select");
    let workspace = fixture.create_directory("workspace");
    let agent_home = fixture.create_directory("agent-home");
    let inventory = fixture.create_directory("inventory");
    let store = PersonalAgentProfileStore::open(&inventory).expect("open inventory");
    for id in ["zeta", "alpha"] {
        let profile = PersonalAgentProfile::new(
            id,
            "claude-code",
            "claude-agent-acp-linux-amd64-0.70.0",
            "podman-private-egress",
            "interactive-surface",
            &workspace,
            &agent_home,
            vec![],
            vec![],
        )
        .expect("build canonical profile");
        store.create(&profile).expect("create profile");
    }

    let listed = store.list().expect("list valid inventory");
    assert_eq!(
        listed
            .iter()
            .map(|profile| profile.profile_id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(
        store
            .select("zeta")
            .expect("select exact profile")
            .profile_id,
        "zeta"
    );
    assert!(matches!(
        store.select("missing"),
        Err(ProfileError::ProfileNotFound(id)) if id == "missing"
    ));

    fs::write(inventory.join("unexpected-file"), b"drift").expect("create inventory drift");
    assert!(matches!(store.list(), Err(ProfileError::InvalidProfile(_))));
}

#[test]
fn distribution_receipt_pins_registry_integrity_lock_and_resolved_image() {
    let lockfile =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/package-lock.json");
    let lockfile_sha256 =
        AgentDistributionReceipt::lockfile_digest(&lockfile).expect("hash committed lockfile");
    let receipt = AgentDistributionReceipt {
        schema: AGENT_DISTRIBUTION_RECEIPT_SCHEMA.into(),
        distribution_id: "claude-agent-acp-linux-amd64-0.70.0".into(),
        registry_id: "claude-acp".into(),
        publisher: "agentclientprotocol".into(),
        package: "@agentclientprotocol/claude-agent-acp".into(),
        version: "0.70.0".into(),
        package_integrity: "sha512-Psqj6fhV4pQ8IM480zpJ+xGiMMIqNLxlsTj5Mzn+T8KSURCVNJdl0ktcqLMjgHJC/QnOvDdDkFf3xTW9VIV9aQ==".into(),
        lockfile_sha256,
        platform: "linux/amd64".into(),
        resolved_image:
            "sha256:ca2f6b0d25785fcb78a695d2f0b00d48e7f2b92a9d6ef820cbc352f33a4a8825"
                .into(),
        agent_executable: "/usr/local/bin/node".into(),
        agent_args: vec![
            "/opt/swem/agent/node_modules/@agentclientprotocol/claude-agent-acp/dist/index.js"
                .into(),
        ],
    };
    receipt
        .validate()
        .expect("validate exact distribution receipt");

    let mut weak = receipt;
    weak.package_integrity = "sha256:not-npm-sri".into();
    assert!(matches!(
        weak.validate(),
        Err(ProfileError::InvalidDistribution(_))
    ));
}

#[test]
#[ignore = "requires the locked Claude ACP image, ready Podman endpoint and explicit live credential"]
fn live_profile_cold_restart_resume_and_cancel_are_native_and_secret_free() {
    let workspace = PathBuf::from(
        std::env::var_os("SWEM_LIVE_PODMAN_WORKSPACE")
            .expect("SWEM_LIVE_PODMAN_WORKSPACE must be set"),
    );
    let agent_home = PathBuf::from(
        std::env::var_os("SWEM_LIVE_PODMAN_AGENT_HOME")
            .expect("SWEM_LIVE_PODMAN_AGENT_HOME must be set"),
    );
    let credential = std::env::var("SWEM_LIVE_CLAUDE_CREDENTIAL")
        .expect("SWEM_LIVE_CLAUDE_CREDENTIAL must be set only for this live test");
    assert!(!credential.is_empty());
    fs::create_dir_all(&workspace).expect("create live profile workspace");
    fs::create_dir_all(&agent_home).expect("create live native-agent home");
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mcp-live-echo.mjs"),
        workspace.join("mcp-live-echo.mjs"),
    )
    .expect("copy independent arbitrary MCP oracle");
    let workspace = workspace.canonicalize().expect("canonical live workspace");
    let inventory = workspace.join("harness-inventory");
    let ledger = workspace.join("harness-routes.sqlite3");
    let first_path = workspace.join("profile-stage-first.json");
    let second_path = workspace.join("profile-stage-second.json");
    let cancel_path = workspace.join("profile-stage-cancel.json");

    run_profile_stage("first", &inventory, &ledger, &first_path);
    run_profile_stage("second", &inventory, &ledger, &second_path);
    run_profile_stage("cancel", &inventory, &ledger, &cancel_path);

    let first: serde_json::Value =
        serde_json::from_slice(&fs::read(&first_path).expect("read first process result"))
            .expect("parse first process result");
    let second: serde_json::Value =
        serde_json::from_slice(&fs::read(&second_path).expect("read second process result"))
            .expect("parse second process result");
    let cancel: serde_json::Value =
        serde_json::from_slice(&fs::read(&cancel_path).expect("read cancel process result"))
            .expect("parse cancel process result");
    assert_eq!(first["session_id"], second["session_id"]);
    assert_eq!(first["session_id"], cancel["session_id"]);
    assert_eq!(second["replayed_updates"], 0);
    let nonce = first["mcp_receipt"]["nonce"]
        .as_str()
        .expect("first MCP receipt has nonce");
    assert!(
        second["reply"]
            .as_str()
            .is_some_and(|reply| reply.contains(nonce)),
        "cold host process did not recover native conversation state"
    );
    assert_eq!(cancel["stop_reason"], "cancelled");
    assert_eq!(cancel["control_outcome"], "cancelled");
    for result in [&first, &second, &cancel] {
        assert_eq!(result["credential_cleanup"]["terminally_absent"], true);
        assert!(result["environment_cleanup"].is_object());
        assert_eq!(
            result["distribution"]["lockfile_sha256"],
            first["distribution"]["lockfile_sha256"]
        );
        assert_eq!(
            result["distribution"]["resolved_image"],
            first["distribution"]["resolved_image"]
        );
    }

    let profile_path = inventory.join("claude-main/profile.json");
    for path in [
        profile_path,
        first_path,
        second_path,
        cancel_path,
        workspace.join("profile-first-transcript.jsonl"),
        workspace.join("profile-resume-transcript.jsonl"),
        workspace.join("profile-cancel-transcript.jsonl"),
    ] {
        let bytes = fs::read(&path)
            .unwrap_or_else(|error| panic!("read durable surface {}: {error}", path.display()));
        assert!(
            !String::from_utf8_lossy(&bytes).contains(&credential),
            "credential leaked into durable surface {}",
            path.display()
        );
    }
    let profile_bytes = fs::read_to_string(inventory.join("claude-main/profile.json"))
        .expect("read persisted personal-agent profile");
    assert!(!profile_bytes.contains("agent_args"));
    assert!(!profile_bytes.contains("native_session_id"));
    assert!(!profile_bytes.contains("lease_id"));
    assert!(!profile_bytes.contains("mcp-live-echo.mjs"));
}

fn run_profile_stage(
    stage: &str,
    inventory: &std::path::Path,
    ledger: &std::path::Path,
    output: &std::path::Path,
) {
    let status = Command::new(env!("CARGO_BIN_EXE_swem-personal-agent-fixture"))
        .args([stage])
        .arg(inventory)
        .arg(ledger)
        .arg(output)
        .status()
        .unwrap_or_else(|error| panic!("spawn {stage} host process: {error}"));
    assert!(
        status.success(),
        "{stage} host process failed with {status}"
    );
}

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("swem-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir(&root).expect("create exact fixture root");
        Self(root)
    }

    fn create_directory(&self, name: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::create_dir(&path).expect("create fixture child");
        path.canonicalize().expect("canonical fixture child")
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        assert!(
            self.0.starts_with(std::env::temp_dir()),
            "fixture cleanup escaped the OS temporary directory"
        );
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The setup a person gives an agent - a model provider, a model, a role,
/// skills - survives the store like everything else, in canonical form: the
/// role verbatim (its trailing newline included, because canonical form has
/// to be reachable from what was stored), the skills sorted by name.
#[test]
fn profile_setup_round_trips_verbatim_and_in_canonical_form() {
    let fixture = FixtureDirectory::new("profile-setup");
    let workspace = fixture.create_directory("workspace");
    let agent_home = fixture.create_directory("agent-home");
    let inventory = fixture.create_directory("inventory");
    let profile = PersonalAgentProfile::new(
        "ada",
        "claude-code",
        "direct-host-distribution",
        "direct-host-environment",
        "ask-every-time",
        &workspace,
        &agent_home,
        Vec::new(),
        Vec::new(),
    )
    .expect("build")
    .with_setup(swem_host::AgentSetup {
        model_provider: Some("anthropic".into()),
        model: Some("claude-sonnet-5".into()),
        role: "You are Ada.\nBe brief.\n".into(),
        agent_skills: vec![
            swem_host::AgentSkill {
                name: "zebra".into(),
                description: "last".into(),
                body: "stripes".into(),
            },
            swem_host::AgentSkill {
                name: "apple".into(),
                description: "first".into(),
                body: "crunch".into(),
            },
        ],
    })
    .expect("setup");
    assert_eq!(
        profile.agent_skills[0].name, "apple",
        "skills are sorted by name"
    );
    assert_eq!(
        profile.role, "You are Ada.\nBe brief.\n",
        "the role is kept verbatim"
    );

    let store = PersonalAgentProfileStore::open(&inventory).expect("open");
    store.create(&profile).expect("create");
    let loaded = PersonalAgentProfileStore::open(&inventory)
        .expect("reopen")
        .load("ada")
        .expect("load");
    assert_eq!(loaded, profile);
    assert_eq!(loaded.setup().model.as_deref(), Some("claude-sonnet-5"));

    // Amending the setup alone keeps everything else and bumps the revision.
    let mut edited = loaded
        .clone()
        .with_setup(swem_host::AgentSetup {
            role: "You are Ada, again.".into(),
            ..loaded.setup()
        })
        .expect("edit");
    edited.revision = loaded.revision;
    let amended = store.amend(&edited).expect("amend");
    assert_eq!(amended.revision, loaded.revision + 1);
    assert_eq!(amended.role, "You are Ada, again.");
    assert_eq!(amended.agent_skills.len(), 2);
}

/// A skill's name becomes a folder, so a name that cannot be one, or one
/// used twice, is refused before anything is written.
#[test]
fn profile_setup_refuses_unusable_or_repeated_skill_names() {
    let fixture = FixtureDirectory::new("profile-setup-refused");
    let workspace = fixture.create_directory("workspace");
    let agent_home = fixture.create_directory("agent-home");
    let base = PersonalAgentProfile::new(
        "ada",
        "claude-code",
        "direct-host-distribution",
        "direct-host-environment",
        "ask-every-time",
        &workspace,
        &agent_home,
        Vec::new(),
        Vec::new(),
    )
    .expect("build");
    let skill = |name: &str| swem_host::AgentSkill {
        name: name.into(),
        description: String::new(),
        body: String::new(),
    };
    assert!(matches!(
        base.clone().with_setup(swem_host::AgentSetup {
            agent_skills: vec![skill("../escape")],
            ..Default::default()
        }),
        Err(ProfileError::InvalidProfile(_))
    ));
    assert!(matches!(
        base.clone().with_setup(swem_host::AgentSetup {
            agent_skills: vec![skill("twice"), skill("twice")],
            ..Default::default()
        }),
        Err(ProfileError::InvalidProfile(_))
    ));
    assert!(matches!(
        base.with_setup(swem_host::AgentSetup {
            model: Some("a model with spaces".into()),
            ..Default::default()
        }),
        Err(ProfileError::InvalidProfile(_))
    ));
}

/// A profile written before the setup existed - no role, no model, no
/// skills in its JSON - still loads, and reads as one with none of them set.
#[test]
fn a_profile_written_before_the_setup_existed_still_loads() {
    let fixture = FixtureDirectory::new("profile-old");
    let workspace = fixture.create_directory("workspace");
    let agent_home = fixture.create_directory("agent-home");
    let inventory = fixture.create_directory("inventory");
    let directory = inventory.join("old");
    fs::create_dir_all(&directory).expect("profile dir");
    fs::write(
        directory.join("profile.json"),
        serde_json::json!({
            "schema": PERSONAL_AGENT_PROFILE_SCHEMA,
            "profile_id": "old",
            "revision": 3,
            "agent_id": "hands",
            "distribution_ref": "direct-host-distribution",
            "environment_profile_id": "direct-host-environment",
            "permission_profile_id": "ask-every-time",
            "workspace": workspace,
            "agent_home": agent_home,
            "attachments": [],
            "credential_bindings": []
        })
        .to_string(),
    )
    .expect("write old profile");
    let loaded = PersonalAgentProfileStore::open(&inventory)
        .expect("open")
        .load("old")
        .expect("an old profile loads");
    assert_eq!(loaded.revision, 3);
    assert_eq!(loaded.setup(), swem_host::AgentSetup::default());
}
