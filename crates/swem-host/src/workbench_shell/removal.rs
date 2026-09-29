//! Removing an agent.
//!
//! Nothing a person or the agent wrote is destroyed: its chats stay with
//! what was said in them, and the folder it worked in and its home stay
//! on the disk. What goes is the agent as somebody to write to: it is
//! retired in the ledger, its schedules are forgotten, what it held of its
//! own is forgotten, and its profile is set aside under the data root.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::chats::ledger_refusal;
use super::{WorkbenchShellError, WorkbenchShellState};

/// What became of an agent that was removed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AgentRemoved {
    pub profile_id: String,
    /// What it was called.
    pub name: String,
    /// The folder it worked in, which stays.
    pub workspace: PathBuf,
    /// Its own home, which stays.
    pub agent_home: PathBuf,
    /// How many schedules were forgotten with it.
    pub schedules: usize,
}

impl WorkbenchShellState {
    /// Keep what is removed under `root`.
    ///
    /// # Errors
    ///
    /// Refuses being enabled twice.
    pub fn enable_removal(&self, root: &Path) -> Result<(), WorkbenchShellError> {
        self.removed_root
            .set(root.to_owned())
            .map_err(|_| WorkbenchShellError::Conflict("what is removed is already kept".into()))
    }

    /// Remove an agent.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] for an unknown agent, and when its
    /// profile cannot be set aside.
    pub async fn remove_agent(
        self: &std::sync::Arc<Self>,
        profile_id: &str,
    ) -> Result<AgentRemoved, WorkbenchShellError> {
        let profile = self
            .inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let aside = self.removed_root.get().cloned().ok_or_else(|| {
            WorkbenchShellError::NotFound("this product does not keep what is removed".into())
        })?;
        let id = profile_id.to_owned();
        let agent = self
            .with_ledger(move |ledger| ledger.agent_of_profile(&id))
            .await
            .map_err(ledger_refusal)?;
        // Whatever it is doing is stopped, and what it has open is let go
        // of, before it is nobody.
        self.stop_agent(&agent.participant_id).await;
        self.let_go_of(Some(&agent.participant_id)).await;
        for terminal in self.terminals.list().await {
            if terminal.profile_id == profile_id {
                let _ = self.terminals.close(&terminal.terminal_id).await;
            }
        }
        let agent_id = agent.participant_id.clone();
        let schedules = self
            .with_ledger(move |ledger| {
                let schedules = ledger.schedules(Some(&agent_id))?;
                for schedule in &schedules {
                    ledger.forget_schedule(&schedule.schedule_id)?;
                }
                ledger.retire_agent(&agent_id)?;
                Ok(schedules.len())
            })
            .await
            .map_err(ledger_refusal)?;
        self.inventory
            .set_aside(profile_id, &aside.join("profiles"))
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        if let Some(keepers) = self.keepers.get() {
            let _ = keepers.choose_for(profile_id, None);
        }
        if let Some(root) = self.machine_root.get() {
            let _ = std::fs::remove_file(
                root.join("hosts")
                    .join("looks")
                    .join(format!("{profile_id}.json")),
            );
        }
        Ok(AgentRemoved {
            profile_id: profile_id.to_owned(),
            name: agent.name,
            workspace: profile.workspace,
            agent_home: profile.agent_home,
            schedules,
        })
    }
}
