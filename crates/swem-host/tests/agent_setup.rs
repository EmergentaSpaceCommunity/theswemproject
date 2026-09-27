//! The profile's setup, put where its agent reads it: the role in the
//! instruction file the agent's layout names, skills as folders, the model
//! in the variable the agent reads - written idempotently, and never over a
//! person's own text or their own configuration.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use swem_host::agent_setup::{
    SKILL_MARKER, SYSTEM_MARKER, USER_MARKER, materialise_profile, write_materialised,
};
use swem_host::workbench_shell::{
    MODEL_PROVIDER_SCHEMA, ModelChoice, ModelProvider, ModelProviderOrigin,
};
use swem_host::{AgentSetup, AgentSkill, PersonalAgentProfile};

fn scratch(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("swem-agent-setup-{name}-{stamp}"));
    std::fs::create_dir_all(root.join("workspace")).expect("workspace");
    std::fs::create_dir_all(root.join("home")).expect("home");
    root
}

fn profile(root: &Path, agent_id: &str, setup: AgentSetup) -> PersonalAgentProfile {
    PersonalAgentProfile::new(
        "ada",
        agent_id,
        "direct-host-distribution",
        "direct-host-environment",
        "ask-every-time",
        &root.join("workspace"),
        &root.join("home"),
        Vec::new(),
        Vec::new(),
    )
    .expect("profile")
    .with_setup(setup)
    .expect("setup")
}

fn provider(base_url: Option<&str>) -> ModelProvider {
    ModelProvider {
        schema: MODEL_PROVIDER_SCHEMA.to_owned(),
        id: "local".into(),
        name: "Local".into(),
        base_url: base_url.map(str::to_owned),
        key_type: "generic_env_var".into(),
        models: vec![ModelChoice {
            id: "quality".into(),
            name: "Quality".into(),
        }],
        origin: ModelProviderOrigin::Declared,
    }
}

#[test]
fn claude_code_gets_its_role_in_claude_md_its_skills_as_folders_and_its_model_in_the_environment() {
    let root = scratch("claude");
    let profile = profile(
        &root,
        "claude-code",
        AgentSetup {
            model_provider: Some("local".into()),
            model: Some("quality".into()),
            role: "You are Ada. Be brief.\n".into(),
            agent_skills: vec![AgentSkill {
                name: "review".into(),
                description: "when asked to review".into(),
                body: "Read first.".into(),
            }],
        },
    );
    let materialised = materialise_profile(
        &profile,
        Some(&provider(Some("http://127.0.0.1:8000/v1"))),
        None,
    )
    .expect("materialise");
    write_materialised(&materialised).expect("write");

    let workspace = root.join("workspace");
    let instructions = std::fs::read_to_string(workspace.join("CLAUDE.md")).expect("CLAUDE.md");
    assert!(instructions.starts_with(SYSTEM_MARKER));
    assert!(instructions.contains("You are Ada. Be brief."));
    assert!(instructions.contains("`ada`"));
    assert!(instructions.trim_end().ends_with(USER_MARKER));
    let skill = std::fs::read_to_string(workspace.join(".claude/skills/review/SKILL.md"))
        .expect("SKILL.md");
    assert!(skill.starts_with("---\nname: review\ndescription: when asked to review\n---"));
    assert!(skill.contains("Read first."));
    assert!(
        workspace
            .join(".claude/skills/review")
            .join(SKILL_MARKER)
            .is_file()
    );
    assert_eq!(
        materialised
            .environment
            .get("ANTHROPIC_MODEL")
            .map(String::as_str),
        Some("quality")
    );
    assert_eq!(
        materialised
            .environment
            .get("ANTHROPIC_BASE_URL")
            .map(String::as_str),
        Some("http://127.0.0.1:8000/v1")
    );
    assert!(
        materialised
            .notes
            .iter()
            .any(|note| note.contains("CLAUDE.md"))
    );
    assert!(
        !workspace.join("opencode.json").exists(),
        "not opencode's file"
    );

    // Written again, nothing changes; the person's own text below the
    // marker survives a changed role.
    let mut personal = instructions.clone();
    personal.push_str("# My notes\nkeep these\n");
    std::fs::write(workspace.join("CLAUDE.md"), &personal).expect("their text");
    let edited = profile
        .clone()
        .with_setup(AgentSetup {
            role: "You are Ada, again.".into(),
            ..profile.setup()
        })
        .expect("edit");
    let again = materialise_profile(&edited, None, None).expect("again");
    write_materialised(&again).expect("write again");
    let rewritten = std::fs::read_to_string(workspace.join("CLAUDE.md")).expect("CLAUDE.md");
    assert!(rewritten.contains("You are Ada, again."));
    assert!(!rewritten.contains("Be brief."));
    assert!(rewritten.ends_with("# My notes\nkeep these\n"));
    let once_more = materialise_profile(&edited, None, None).expect("once more");
    write_materialised(&once_more).expect("write once more");
    assert_eq!(
        std::fs::read_to_string(workspace.join("CLAUDE.md")).expect("stable"),
        rewritten
    );
}

#[test]
fn opencode_gets_agents_md_a_project_config_and_a_prefixed_model_variable() {
    let root = scratch("opencode");
    let profile = profile(
        &root,
        "opencode",
        AgentSetup {
            model_provider: Some("local".into()),
            model: Some("quality".into()),
            role: "Terse.".into(),
            agent_skills: Vec::new(),
        },
    );
    let materialised = materialise_profile(
        &profile,
        Some(&provider(Some("http://127.0.0.1:8000/v1"))),
        None,
    )
    .expect("materialise");
    write_materialised(&materialised).expect("write");
    let workspace = root.join("workspace");
    assert!(workspace.join("AGENTS.md").is_file());
    assert_eq!(
        materialised
            .environment
            .get("OPENCODE_MODEL")
            .map(String::as_str),
        Some("local/quality")
    );
    let config: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(workspace.join("opencode.json")).expect("opencode.json"),
    )
    .expect("json");
    assert_eq!(config["swem"], true);
    assert_eq!(config["model"], "local/quality");
    assert_eq!(
        config["provider"]["local"]["options"]["baseURL"],
        "http://127.0.0.1:8000/v1"
    );
    assert_eq!(
        config["provider"]["local"]["models"]["quality"]["name"],
        "Quality"
    );
}

#[test]
fn a_persons_own_opencode_config_is_left_alone_and_said_so() {
    let root = scratch("opencode-theirs");
    let workspace = root.join("workspace");
    std::fs::write(
        workspace.join("opencode.json"),
        r#"{"model": "theirs/model"}"#,
    )
    .expect("theirs");
    let profile = profile(
        &root,
        "opencode",
        AgentSetup {
            model: Some("quality".into()),
            ..Default::default()
        },
    );
    let materialised = materialise_profile(&profile, None, None).expect("materialise");
    write_materialised(&materialised).expect("write");
    assert_eq!(
        std::fs::read_to_string(workspace.join("opencode.json")).expect("theirs still"),
        r#"{"model": "theirs/model"}"#
    );
    assert!(
        materialised
            .notes
            .iter()
            .any(|note| note.contains("its own opencode.json"))
    );
}

#[test]
fn a_skill_taken_off_the_profile_leaves_the_folder_but_a_persons_own_folder_stays() {
    let root = scratch("skills");
    let skill = |name: &str| AgentSkill {
        name: name.into(),
        description: String::new(),
        body: "x".into(),
    };
    let two = profile(
        &root,
        "claude-code",
        AgentSetup {
            agent_skills: vec![skill("one"), skill("two")],
            ..Default::default()
        },
    );
    write_materialised(&materialise_profile(&two, None, None).expect("two")).expect("write two");
    let skills = root.join("workspace/.claude/skills");
    std::fs::create_dir_all(skills.join("theirs")).expect("their folder");
    std::fs::write(skills.join("theirs/SKILL.md"), "handmade").expect("their skill");
    assert!(skills.join("two").is_dir());

    let one = two
        .clone()
        .with_setup(AgentSetup {
            agent_skills: vec![skill("one")],
            ..two.setup()
        })
        .expect("one");
    write_materialised(&materialise_profile(&one, None, None).expect("one")).expect("write one");
    assert!(skills.join("one").is_dir());
    assert!(
        !skills.join("two").exists(),
        "the skill taken off the profile is gone"
    );
    assert!(
        skills.join("theirs/SKILL.md").is_file(),
        "a folder without the marker is never touched"
    );
}

#[test]
fn a_declared_agent_gets_agents_md_and_no_model_variable() {
    let root = scratch("declared");
    let profile = profile(
        &root,
        "hands",
        AgentSetup {
            model: Some("quality".into()),
            role: "Hands only.".into(),
            ..Default::default()
        },
    );
    let materialised = materialise_profile(&profile, None, None).expect("materialise");
    write_materialised(&materialised).expect("write");
    assert!(
        std::fs::read_to_string(root.join("workspace/AGENTS.md"))
            .expect("AGENTS.md")
            .contains("Hands only.")
    );
    assert!(materialised.environment.is_empty());
    assert!(
        materialised
            .notes
            .iter()
            .any(|note| note.contains("offered in the session"))
    );
}

#[test]
fn an_agent_is_told_who_it_is_whose_it_is_and_how_it_is_told_who_speaks() {
    let root = scratch("standing");
    let profile = profile(
        &root,
        "claude-code",
        AgentSetup {
            role: "Review what you are shown.".into(),
            ..Default::default()
        },
    );
    let standing = swem_host::agent_setup::Standing {
        name: "Reviewer".into(),
        handle: "reviewer".into(),
        principal_name: "Ada".into(),
        principal_handle: "ada".into(),
    };
    let materialised = materialise_profile(&profile, None, Some(&standing)).expect("materialise");
    write_materialised(&materialised).expect("write");
    let instructions =
        std::fs::read_to_string(root.join("workspace/CLAUDE.md")).expect("CLAUDE.md");
    assert!(instructions.starts_with(&format!(
        "{SYSTEM_MARKER}\nYou are **Reviewer** (`@reviewer`), an agent of Ada (`@ada`) in SWEM."
    )));
    for part in [
        "## Who is speaking",
        "<swem:turn k=\"…\">",
        "`principal` is Ada",
        "A turn that ends with no such block was written by Ada.",
        "## Where you work",
        "## Your role\n\nReview what you are shown.",
    ] {
        assert!(instructions.contains(part), "the instructions say: {part}");
    }
    assert!(instructions.trim_end().ends_with(USER_MARKER));

    // Renamed, the agent is told its new name; nothing of the old one stays.
    let renamed = swem_host::agent_setup::Standing {
        name: "Second reader".into(),
        handle: "second".into(),
        ..standing
    };
    write_materialised(&materialise_profile(&profile, None, Some(&renamed)).expect("again"))
        .expect("write again");
    let instructions =
        std::fs::read_to_string(root.join("workspace/CLAUDE.md")).expect("CLAUDE.md");
    assert!(instructions.contains("**Second reader** (`@second`)"));
    assert!(!instructions.contains("reviewer"));
}
