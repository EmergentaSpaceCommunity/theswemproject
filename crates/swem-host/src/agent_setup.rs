//! What a profile says about its agent, put where the agent reads it.
//!
//! A role, a model and skills are the profile's; the agent learns them the
//! way it learns anything - from its instruction file, its skills folder and
//! its environment - because the protocol carries none of them: `session/new`
//! takes a directory and MCP servers, and a model is chosen afterwards only
//! if the agent offers the choice. So before a session opens, the profile is
//! written into the agent's working directory in the agent's own layout, and
//! the model is put into the process environment under the variable that
//! agent reads.
//!
//! Only the working directory is written, never the agent's home. When the
//! agent runs on this machine it gets this machine's `HOME`, not the
//! profile's `agent_home`; when it runs in a container the working directory
//! is the one mount both sides share. A file under the working directory
//! reaches the agent either way, and a fixture that reads its own directory
//! can prove it arrived.
//!
//! The instruction file is the person's as much as SWEM's: only the span
//! between the two markers is SWEM's to rewrite, and the person's own text
//! below it is kept verbatim. A skill folder is removed only when it carries
//! the marker file this module wrote, so a skill the person put there by hand
//! is never touched.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::profile::{AgentSkill, PersonalAgentProfile};
use crate::workbench_shell::ModelProvider;

/// Opens the span this module rewrites on every session start.
pub const SYSTEM_MARKER: &str = "<!-- swem:system -->";
/// Closes that span; everything after it is the person's.
pub const USER_MARKER: &str = "<!-- swem:user -->";
/// The file a skill folder carries when this module wrote it.
pub const SKILL_MARKER: &str = ".swem-skill";
/// The key an `opencode.json` carries when this module wrote it.
const OPENCODE_OWNED: &str = "swem";

/// Who an agent is in chats and whose it is: the names the instructions use
/// when they explain how the agent is told who speaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Standing {
    pub name: String,
    pub handle: String,
    pub principal_name: String,
    pub principal_handle: String,
}

/// What one profile amounts to on disk and in the process environment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Materialised {
    /// Files to write, absolute, with their whole content.
    pub files: Vec<(PathBuf, String)>,
    /// Non-secret variables for the agent process: the model, the address.
    pub environment: BTreeMap<String, String>,
    /// The skills folder to tidy and the skill names that belong in it, so a
    /// skill taken off the profile leaves the folder too.
    pub skills: Option<(PathBuf, BTreeSet<String>)>,
    /// What was decided, in words a person can read on the session's status.
    pub notes: Vec<String>,
}

/// How one kind of agent reads its instructions, skills and model.
struct Layout {
    instructions: &'static str,
    skills: Option<&'static str>,
    model_env: Option<&'static str>,
    /// Whether the model variable takes `provider/model` (opencode) or the
    /// bare model id (everyone else).
    model_with_provider: bool,
    base_url_env: Option<&'static str>,
    opencode_config: bool,
}

fn layout_of(agent_id: &str) -> Layout {
    match agent_id {
        "claude-code" => Layout {
            instructions: "CLAUDE.md",
            skills: Some(".claude/skills"),
            model_env: Some("ANTHROPIC_MODEL"),
            model_with_provider: false,
            base_url_env: Some("ANTHROPIC_BASE_URL"),
            opencode_config: false,
        },
        "opencode" => Layout {
            instructions: "AGENTS.md",
            skills: Some(".opencode/skills"),
            model_env: Some("OPENCODE_MODEL"),
            model_with_provider: true,
            base_url_env: None,
            opencode_config: true,
        },
        // Assumed from the vendors' documentation, not yet seen live
        // (tech-debt: verify against a real codex-acp and gemini).
        "codex" => Layout {
            instructions: "AGENTS.md",
            skills: None,
            model_env: None,
            model_with_provider: false,
            base_url_env: Some("OPENAI_BASE_URL"),
            opencode_config: false,
        },
        "gemini" => Layout {
            instructions: "GEMINI.md",
            skills: None,
            model_env: Some("GEMINI_MODEL"),
            model_with_provider: false,
            base_url_env: None,
            opencode_config: false,
        },
        _ => Layout {
            instructions: "AGENTS.md",
            skills: None,
            model_env: None,
            model_with_provider: false,
            base_url_env: None,
            opencode_config: false,
        },
    }
}

/// The profile as files and variables for its agent, read against what the
/// working directory already holds so a person's own text survives.
///
/// # Errors
///
/// Returns a sentence when the working directory cannot be read.
pub fn materialise_profile(
    profile: &PersonalAgentProfile,
    provider: Option<&ModelProvider>,
    standing: Option<&Standing>,
) -> Result<Materialised, String> {
    let layout = layout_of(&profile.agent_id);
    let workspace = &profile.workspace;
    let mut out = Materialised::default();

    // The instruction file, always: even an empty role tells the agent who
    // it is and where it works, and rewriting the span idempotently keeps a
    // stale role from an earlier profile out of it.
    let path = workspace.join(layout.instructions);
    let existing = read_if_present(&path)?;
    let block = instruction_block(profile, standing);
    out.files
        .push((path, merged_instructions(existing.as_deref(), &block)));
    if !profile.role.trim().is_empty() {
        out.notes
            .push(format!("role written to {}", layout.instructions));
    }

    if let Some(skills_dir) = layout.skills {
        let root = workspace.join(skills_dir);
        let mut names = BTreeSet::new();
        for skill in &profile.agent_skills {
            let folder = root.join(&skill.name);
            out.files
                .push((folder.join("SKILL.md"), skill_markdown(skill)));
            out.files.push((folder.join(SKILL_MARKER), String::new()));
            names.insert(skill.name.clone());
        }
        out.skills = Some((root, names));
    } else if !profile.agent_skills.is_empty() {
        out.notes.push(format!(
            "this agent has no skills folder SWEM knows; {} skills stay on the profile only",
            profile.agent_skills.len()
        ));
    }

    let model = profile
        .model
        .as_deref()
        .filter(|model| !model.trim().is_empty());
    let joined = model.map(|model| {
        if model.contains('/') {
            model.to_owned()
        } else if let Some(provider) = provider {
            format!("{}/{model}", provider.id)
        } else {
            model.to_owned()
        }
    });
    let bare = model.map(|model| model.rsplit('/').next().unwrap_or(model).to_owned());
    if let (Some(variable), Some(joined), Some(bare)) = (layout.model_env, &joined, &bare) {
        let value = if layout.model_with_provider {
            joined.clone()
        } else {
            bare.clone()
        };
        out.environment.insert(variable.to_owned(), value.clone());
        out.notes.push(format!("model {value} on {variable}"));
    } else if model.is_some() {
        out.notes.push(
            "this agent reads no model variable SWEM knows; the model is offered in the session"
                .to_owned(),
        );
    }
    if let (Some(variable), Some(provider)) = (layout.base_url_env, provider)
        && let Some(url) = &provider.base_url
    {
        out.environment.insert(variable.to_owned(), url.clone());
    }

    if layout.opencode_config
        && let Some(joined) = &joined
    {
        let path = workspace.join("opencode.json");
        match read_if_present(&path)? {
            Some(text) if !owned_opencode_config(&text) => out.notes.push(
                "the working directory has its own opencode.json; the model is set in the session instead"
                    .to_owned(),
            ),
            _ => out
                .files
                .push((path, opencode_config(joined, provider))),
        }
    }
    Ok(out)
}

/// Write what [`materialise_profile`] decided: files only where their bytes
/// differ, and skill folders this module wrote but the profile no longer
/// names removed.
///
/// # Errors
///
/// Returns a sentence naming the path that could not be written.
pub fn write_materialised(materialised: &Materialised) -> Result<(), String> {
    for (path, content) in &materialised.files {
        if read_if_present(path)?.as_deref() == Some(content.as_str()) {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        crate::profile::write_atomically(path, content.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))?;
    }
    if let Some((root, names)) = &materialised.skills
        && root.is_dir()
    {
        for entry in
            std::fs::read_dir(root).map_err(|error| format!("{}: {error}", root.display()))?
        {
            let entry = entry.map_err(|error| format!("{}: {error}", root.display()))?;
            let folder = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if folder.is_dir() && folder.join(SKILL_MARKER).is_file() && !names.contains(&name) {
                std::fs::remove_dir_all(&folder)
                    .map_err(|error| format!("{}: {error}", folder.display()))?;
            }
        }
    }
    Ok(())
}

fn read_if_present(path: &Path) -> Result<Option<String>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

/// SWEM's span of the instruction file: who the agent is and how it is told
/// who speaks, where it works, and the role the person wrote.
fn instruction_block(profile: &PersonalAgentProfile, standing: Option<&Standing>) -> String {
    let mut block = String::new();
    block.push_str(SYSTEM_MARKER);
    block.push('\n');
    if let Some(standing) = standing {
        block.push_str(&crate::standing_explanation(
            &standing.name,
            &standing.handle,
            &standing.principal_name,
            &standing.principal_handle,
        ));
        let _ = writeln!(
            block,
            "\n## Where you work\n\nYour working directory is `{}`; the projects you are \
             attached to are reached through your tools.",
            profile.workspace.display()
        );
    } else {
        let _ = writeln!(
            block,
            "You are `{}`, an agent a person runs through SWEM. Your working directory is `{}`; \
             the projects you are attached to are reached through your tools.",
            profile.profile_id,
            profile.workspace.display()
        );
    }
    let role = profile.role.trim_end();
    if !role.is_empty() {
        block.push('\n');
        if standing.is_some() {
            block.push_str("## Your role\n\n");
        }
        block.push_str(role);
        block.push('\n');
    }
    block.push_str(USER_MARKER);
    block.push('\n');
    block
}

/// The file with SWEM's span replaced and everything else kept: the text
/// before the first marker, and the person's text after the second.
fn merged_instructions(existing: Option<&str>, block: &str) -> String {
    let Some(existing) = existing else {
        return block.to_owned();
    };
    if let (Some(start), Some(end)) = (existing.find(SYSTEM_MARKER), existing.find(USER_MARKER))
        && start <= end
    {
        let after = &existing[end + USER_MARKER.len()..];
        let after = after.strip_prefix('\n').unwrap_or(after);
        let mut merged = String::with_capacity(existing.len() + block.len());
        merged.push_str(&existing[..start]);
        merged.push_str(block);
        merged.push_str(after);
        return merged;
    }
    // A file the person wrote before SWEM touched it: the span goes on top,
    // once, and their whole text becomes the part below the marker.
    let mut merged = String::with_capacity(existing.len() + block.len() + 1);
    merged.push_str(block);
    merged.push('\n');
    merged.push_str(existing);
    merged
}

/// A SKILL.md the way agents read them: frontmatter with the name and when
/// to use it, then the instructions.
fn skill_markdown(skill: &AgentSkill) -> String {
    let description = skill.description.replace('\n', " ");
    let description = description.trim();
    let mut text = format!(
        "---\nname: {}\ndescription: {description}\n---\n\n",
        skill.name
    );
    text.push_str(skill.body.trim_end());
    text.push('\n');
    text
}

fn owned_opencode_config(text: &str) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.get(OPENCODE_OWNED).and_then(Value::as_bool))
        .unwrap_or(false)
}

/// opencode's project configuration: the model to boot on and, when the
/// provider has an address of its own, the provider declared with the models
/// it lists - opencode cannot enumerate a custom endpoint's catalogue.
fn opencode_config(model: &str, provider: Option<&ModelProvider>) -> String {
    let mut config = json!({
        "$schema": "https://opencode.ai/config.json",
        OPENCODE_OWNED: true,
        "model": model,
    });
    if let Some(provider) = provider
        && let Some(url) = &provider.base_url
    {
        let models: serde_json::Map<String, Value> = provider
            .models
            .iter()
            .map(|choice| {
                (
                    choice.id.clone(),
                    json!({ "name": if choice.name.is_empty() { &choice.id } else { &choice.name } }),
                )
            })
            .collect();
        let mut block = json!({
            "name": provider.name,
            "npm": "@ai-sdk/openai-compatible",
            "options": { "baseURL": url },
        });
        if !models.is_empty() {
            block["models"] = Value::Object(models);
        }
        config["provider"] = json!({ provider.id.clone(): block });
    }
    let mut text = serde_json::to_string_pretty(&config).unwrap_or_default();
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_span_is_replaced_and_the_persons_text_kept() {
        let block = format!("{SYSTEM_MARKER}\nnew\n{USER_MARKER}\n");
        let existing = format!("{SYSTEM_MARKER}\nold\n{USER_MARKER}\nmine\n");
        assert_eq!(
            merged_instructions(Some(&existing), &block),
            format!("{SYSTEM_MARKER}\nnew\n{USER_MARKER}\nmine\n")
        );
        // Applying it again changes nothing.
        let once = merged_instructions(Some(&existing), &block);
        assert_eq!(merged_instructions(Some(&once), &block), once);
    }

    #[test]
    fn a_file_without_markers_goes_below_the_span_once() {
        let block = format!("{SYSTEM_MARKER}\nrole\n{USER_MARKER}\n");
        let merged = merged_instructions(Some("# Their notes\n"), &block);
        assert_eq!(merged, format!("{block}\n# Their notes\n"));
        assert_eq!(merged_instructions(Some(&merged), &block), merged);
        assert_eq!(merged_instructions(None, &block), block);
    }

    #[test]
    fn a_foreign_opencode_config_is_recognised() {
        assert!(owned_opencode_config(r#"{"swem": true, "model": "x"}"#));
        assert!(!owned_opencode_config(r#"{"model": "x"}"#));
        assert!(!owned_opencode_config("not json"));
    }
}
