//! A provider's key is given once and every agent that answers from the
//! provider is handed it; a key an agent held for its provider moves there.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
use swem_host::{
    AgentSetup, IntegrationKind, KeyStore, LaunchCommand, PersonalAgentProfile,
    PersonalAgentProfileStore, ResolvedDirectAgentConnection, ShellConnectionMode,
    WorkbenchShellError, WorkbenchShellState,
    workbench_shell::{DeclareModelProviderBody, GiveKeyBody},
};

fn fixture_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "swem-provider-keys-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("create fixture root");
    root
}

/// An agent on the echo engine, answering from `provider` when one is named.
fn agent(root: &Path, id: &str, provider: Option<&str>) {
    let workspace = root.join(format!("workspace-{id}"));
    let home = root.join(format!("home-{id}"));
    fs::create_dir_all(&workspace).expect("create workspace");
    fs::create_dir_all(&home).expect("create agent home");
    let profile = PersonalAgentProfile::new(
        id,
        "swem-echo-agent",
        "echo-fixture-distribution",
        "direct-fixture-environment",
        swem_host::ASK_EVERY_TIME,
        &workspace,
        &home,
        Vec::new(),
        Vec::new(),
    )
    .expect("profile")
    .with_setup(AgentSetup {
        model_provider: provider.map(str::to_owned),
        ..Default::default()
    })
    .expect("setup");
    PersonalAgentProfileStore::open(&root.join("inventory"))
        .expect("open inventory")
        .create(&profile)
        .expect("persist profile");
}

/// A Workbench over the fixture's agents; keys are enabled by the test, at
/// the moment it means a product to start.
fn shell(root: &Path) -> Arc<WorkbenchShellState> {
    let state = WorkbenchShellState::open(
        &root.join("inventory"),
        &root.join("routes.sqlite3"),
        Duration::from_secs(20),
        |_profile| {
            let executable = PathBuf::from(env!("CARGO_BIN_EXE_swem-echo-agent"));
            Ok(ResolvedDirectAgentConnection {
                launch: LaunchCommand {
                    executable: executable.display().to_string(),
                    args: Vec::new(),
                    integration: IntegrationKind::DirectAcp,
                },
                agent_executable: executable,
                mcp_servers: Vec::new(),
            })
        },
    )
    .expect("open shell state");
    state
        .enable_model_providers(&root.join("model-providers"))
        .expect("enable providers");
    Arc::new(state)
}

/// Whether the engine of an agent was handed the variable it was told to
/// look for. The echo engine says whether, never what.
async fn handed(state: &Arc<WorkbenchShellState>, profile: &str) -> bool {
    let (connection, _route, _session) = tokio::time::timeout(
        Duration::from_secs(60),
        state.open_connection(profile, ShellConnectionMode::New, None),
    )
    .await
    .expect("opened in time")
    .expect("opens");
    let reply = state
        .submit_prompt(
            &connection,
            vec![ContentBlock::Text(TextContent::new("what did you get"))],
        )
        .await
        .expect("prompt");
    state.disconnect(&connection).await.expect("disconnect");
    let seen: serde_json::Value =
        serde_json::from_str(&reply.reply_text).expect("the echo answers JSON");
    assert!(
        !reply.reply_text.contains("sk-desk"),
        "the engine repeated a key: {}",
        reply.reply_text
    );
    seen["environment"]["ambient_probe_present"] == true
}

#[tokio::test]
async fn a_key_given_once_reaches_every_agent_that_answers_from_the_provider() {
    let root = fixture_root("once");
    for (id, provider) in [("ada", Some("desk")), ("bob", Some("desk")), ("cy", None)] {
        agent(&root, id, provider);
    }
    // The provider is declared before the agents that name it are read.
    let state = shell(&root);
    state
        .declare_model_provider(&DeclareModelProviderBody {
            id: "desk".into(),
            name: "The desk".into(),
            base_url: "http://127.0.0.1:9/v1".into(),
            key_type: "generic_env_var".into(),
            models: Vec::new(),
        })
        .expect("declare the provider");
    state
        .enable_provider_keys(&root.join("keys"))
        .expect("enable keys");
    // Each agent is told which variable to look for; that is all it has of
    // its own.
    let inventory = PersonalAgentProfileStore::open(&root.join("inventory")).expect("inventory");
    for id in ["ada", "bob", "cy"] {
        inventory
            .set_secret(
                id,
                "generic_env_var",
                "what to look for",
                "SWEM_PROBE_VARIABLE_NAME",
                "SWEM_DESK_KEY",
            )
            .expect("tell it what to look for");
    }

    assert!(
        !handed(&state, "ada").await,
        "a key nobody gave reached an engine"
    );

    // A kind of key that names no variable is given with one.
    let unnamed = state
        .give_provider_key(
            "desk",
            &GiveKeyBody {
                value: "sk-desk".into(),
                variable: None,
            },
        )
        .expect_err("no variable was named");
    assert!(
        matches!(unnamed, WorkbenchShellError::Invalid(_)),
        "{unnamed}"
    );
    state
        .give_provider_key(
            "desk",
            &GiveKeyBody {
                value: "sk-desk".into(),
                variable: Some("SWEM_DESK_KEY".into()),
            },
        )
        .expect("give the key");

    assert!(handed(&state, "ada").await, "the first agent has no key");
    assert!(handed(&state, "bob").await, "the second agent has no key");
    assert!(
        !handed(&state, "cy").await,
        "an agent that answers from elsewhere was handed the key"
    );

    let (standing, kept_by) = state.providers_standing().expect("standing");
    assert!(kept_by.is_some_and(|said| said.contains("only you")));
    let desk = standing
        .iter()
        .find(|one| one.provider.provider.id == "desk")
        .expect("the provider is listed");
    assert_eq!(desk.used_by, ["ada", "bob"]);
    assert_eq!(
        desk.key.as_ref().map(|key| key.variable.as_str()),
        Some("SWEM_DESK_KEY")
    );
    assert!(
        !serde_json::to_string(&standing)
            .expect("serialize")
            .contains("sk-desk"),
        "what a page is told holds a key"
    );

    state.take_provider_key("desk").expect("take the key away");
    assert!(
        !handed(&state, "ada").await,
        "a key taken away still reached an engine"
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
fn a_key_an_agent_held_for_its_provider_moves_to_the_provider() {
    let root = fixture_root("moves");
    for (id, provider) in [
        ("ada", Some("anthropic")),
        ("bob", Some("anthropic")),
        ("cy", Some("anthropic")),
        ("dee", None),
    ] {
        agent(&root, id, provider);
    }
    let inventory = PersonalAgentProfileStore::open(&root.join("inventory")).expect("inventory");
    let key = |id: &str, value: &str| {
        inventory
            .set_secret(
                id,
                "anthropic_api_key",
                "Anthropic",
                "ANTHROPIC_API_KEY",
                value,
            )
            .expect("an agent's own key");
    };
    key("ada", "sk-shared");
    key("bob", "sk-shared");
    key("cy", "sk-its-own");
    key("dee", "sk-nowhere-to-go");
    // Something else an agent holds is not its provider's key.
    inventory
        .set_secret(
            "ada",
            "generic_env_var",
            "a server's token",
            "NOTES_TOKEN",
            "t",
        )
        .expect("another secret");

    let state = shell(&root);
    state
        .enable_provider_keys(&root.join("keys"))
        .expect("enable keys");

    let held = KeyStore::open(&root.join("keys"))
        .expect("open the keys")
        .key("anthropic")
        .expect("read")
        .expect("the provider has its key");
    assert_eq!(
        (held.variable.as_str(), held.value.as_str()),
        ("ANTHROPIC_API_KEY", "sk-shared")
    );
    let names = |id: &str| -> Vec<String> {
        inventory
            .secret_entries(id)
            .expect("entries")
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    };
    assert_eq!(names("ada"), ["NOTES_TOKEN"], "the key stayed on the agent");
    assert!(names("bob").is_empty(), "the same key stayed on the agent");
    assert_eq!(
        names("cy"),
        ["ANTHROPIC_API_KEY"],
        "an agent's different key was taken from it"
    );
    assert_eq!(
        inventory.secrets("cy").expect("secrets")["ANTHROPIC_API_KEY"],
        "sk-its-own"
    );
    assert_eq!(
        names("dee"),
        ["ANTHROPIC_API_KEY"],
        "an agent with no provider lost its key"
    );
    fs::remove_dir_all(&root).ok();
}
