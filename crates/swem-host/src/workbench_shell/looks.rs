//! A look inside an agent's machine.
//!
//! Nothing is offered that was not looked at. This is the third look:
//! what an agent has where it lives - its engine and whether it starts,
//! whether it is signed in to its model, the programs it finds, the folder
//! it works in, and whether a container can be started there. What was
//! found is kept with its time, so a page shows it without looking again.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use swem_runner::{ProbeAnswer, ProbeRequest};

use super::{ResolvedAgentEnvironment, WorkbenchShellError, WorkbenchShellState};

pub const LOOK_INSIDE_SCHEMA: &str = "swem:look-inside@0.1";

/// The programs an agent is asked about, by the names they are started by.
const PROGRAMS: [&str; 7] = ["git", "node", "npm", "python3", "uv", "podman", "docker"];

/// An engine as it was found when it was started.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EngineLooked {
    /// Whether it started and answered the greeting.
    pub starts: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_in_ms: Option<u64>,
    /// Whether it opened a session; nothing when that was not told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_in: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_in_ms: Option<u64>,
    /// What it said when it did not start, or did not open a session.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub said: String,
}

/// What an agent has where it lives, as it was found.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LookInside {
    pub schema: String,
    pub profile_id: String,
    pub looked_ms: u64,
    pub engine: EngineLooked,
    /// The machine from inside; nothing for a machine that is not looked
    /// at from inside yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<ProbeAnswer>,
    /// Whether a container can be started where the agent lives.
    pub containers: bool,
    /// Why not, in words.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub containers_said: String,
}

impl WorkbenchShellState {
    fn look_kept_at(&self, profile_id: &str) -> Option<PathBuf> {
        self.machine_root.get().map(|root| {
            root.join("hosts")
                .join("looks")
                .join(format!("{profile_id}.json"))
        })
    }

    /// What was found the last time an agent's machine was looked at from
    /// inside.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent.
    pub fn look_inside_kept(
        &self,
        profile_id: &str,
    ) -> Result<Option<LookInside>, WorkbenchShellError> {
        self.inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        Ok(self
            .look_kept_at(profile_id)
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok()))
    }

    /// Look at an agent's machine from inside, now, and keep what was
    /// found.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent. An engine
    /// that does not start is what was found, not a failure.
    pub async fn look_inside(&self, profile_id: &str) -> Result<LookInside, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let looked_ms = u64::try_from(crate::chat_ledger::now_ms()).unwrap_or(0);
        let here = matches!(
            (self.resolver)(&profile).map(|connection| {
                let here = matches!(connection.environment, ResolvedAgentEnvironment::Direct);
                (connection, here)
            }),
            Ok((_, true))
        );
        let engine = if here {
            self.engine_tried(&profile).await
        } else {
            // A machine that is made for each session is greeted the way
            // a session greets it; whether it is signed in is told when
            // the agent keeps a machine of its own.
            match self.profile_handshake(profile_id).await {
                Ok(handshake) => EngineLooked {
                    starts: true,
                    name: handshake.agent_name,
                    version: handshake.agent_version,
                    started_in_ms: None,
                    signed_in: None,
                    answered_in_ms: None,
                    said: String::new(),
                },
                Err(error) => EngineLooked {
                    starts: false,
                    name: None,
                    version: None,
                    started_in_ms: None,
                    signed_in: None,
                    answered_in_ms: None,
                    said: error.to_string(),
                },
            }
        };
        let machine = if here {
            let request = ProbeRequest {
                workspace: Some(profile.workspace.clone()),
                programs: PROGRAMS.iter().map(|name| (*name).to_owned()).collect(),
            };
            tokio::task::spawn_blocking(move || swem_runner::look(&request))
                .await
                .ok()
        } else {
            None
        };
        let (containers, containers_said) = if here {
            match self.machine() {
                Some(look) if look.a_container_can_start() => (true, String::new()),
                Some(look) => (false, look.why_no_container().unwrap_or_default()),
                None => (false, "This machine was not looked at yet.".to_owned()),
            }
        } else {
            (false, "No container engine inside.".to_owned())
        };
        let look = LookInside {
            schema: LOOK_INSIDE_SCHEMA.into(),
            profile_id: profile_id.to_owned(),
            looked_ms,
            engine,
            machine,
            containers,
            containers_said,
        };
        if let Some(path) = self.look_kept_at(profile_id) {
            let bytes = serde_json::to_vec_pretty(&look)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            if let Some(folder) = path.parent() {
                let _ = std::fs::create_dir_all(folder);
            }
            // What could not be kept is still what was found.
            let _ = std::fs::write(path, bytes);
        }
        Ok(look)
    }

    /// Start the agent's engine as the agent would have it - its model,
    /// its provider's key, what it holds of its own - greet it and ask it
    /// for a session.
    async fn engine_tried(&self, profile: &crate::PersonalAgentProfile) -> EngineLooked {
        let not_started = |said: String| EngineLooked {
            starts: false,
            name: None,
            version: None,
            started_in_ms: None,
            signed_in: None,
            answered_in_ms: None,
            said,
        };
        let tried = async {
            let connection = (self.resolver)(profile).map_err(WorkbenchShellError::Failed)?;
            let provider = match &profile.model_provider {
                Some(id) => Some(self.model_provider_book()?.get(id)?),
                None => None,
            };
            let mut environment =
                crate::agent_setup::materialise_profile(profile, provider.as_ref(), None)
                    .map_err(WorkbenchShellError::Failed)?
                    .environment;
            environment.extend(super::credential_environment(
                profile,
                self.keys_of(profile)?,
            )?);
            let expected = crate::builtin_catalog()
                .into_iter()
                .find(|entry| entry.id == profile.agent_id)
                .and_then(|entry| entry.expected_agent_name);
            crate::try_the_engine(
                &connection.launch,
                &connection.agent_executable,
                environment,
                &profile.workspace,
                self.operation_timeout,
                expected,
            )
            .await
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
        };
        match tried.await {
            Ok(tried) => EngineLooked {
                starts: true,
                name: tried.handshake.agent_name,
                version: tried.handshake.agent_version,
                started_in_ms: Some(tried.started_in_ms),
                signed_in: tried.signed_in,
                answered_in_ms: tried.answered_in_ms,
                said: tried.said,
            },
            Err(error) => not_started(error.to_string()),
        }
    }
}
