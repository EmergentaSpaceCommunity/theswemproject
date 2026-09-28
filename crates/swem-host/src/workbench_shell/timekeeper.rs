//! Who keeps time: the running product.
//!
//! One process keeps time at once, the one that holds the keeper's lock
//! beside the ledger. It asks the ledger what is due, says it into its chat
//! as the schedule, follows the turn to its end and writes how the run
//! ended. Nothing about it lives in an agent's machine.
//!
//! A turn a schedule began has nobody watching it. What its agent asks
//! waits for a person for half an hour and is then answered with the
//! engine's own refusal, so the turn ends and the next one is not passed
//! over for ever.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::chats::ledger_refusal;
use super::{Saying, WorkbenchShellError, WorkbenchShellState};
use crate::time::LeftUnanswered;
use crate::{
    DeliveryState, Due, NewTimedMessage, ParticipantKind, RunState, TimedMessage,
    TimedMessageChange, TimedRun, When,
};

/// How often the keeper looks. Nothing here is to the second.
const LOOK: Duration = Duration::from_secs(5);
/// How long a question in a turn a schedule began waits for a person.
const WAITS_FOR_A_PERSON_MS: u64 = 30 * 60_000;

/// What the page gives to make a schedule.
#[derive(Clone, Debug, Deserialize)]
pub struct NewScheduleBody {
    /// The agent that is told.
    pub agent_id: String,
    /// The chat it is said in; nothing for a new chat each time.
    #[serde(default)]
    pub chat_id: Option<String>,
    pub say: String,
    pub when: When,
}

/// A schedule as the page shows it.
#[derive(Clone, Debug, Serialize)]
pub struct ScheduleShown {
    #[serde(flatten)]
    pub schedule: TimedMessage,
    /// What whoever made it is called.
    pub made_by_name: String,
    /// What the chat it is said in is called; nothing for a new chat each
    /// time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_title: Option<String>,
}

/// Who keeps time, as Providers shows it.
#[derive(Clone, Debug, Serialize)]
pub struct KeeperStanding {
    /// Whether this process is the one keeping time.
    pub keeping: bool,
    /// The zone a time of day is kept in, by its name.
    pub zone: String,
    /// The profiles of the agents that have schedules.
    pub used_by: Vec<String>,
    pub schedules: usize,
}

/// A schedule of the old clock, as its file kept it.
#[derive(Deserialize)]
struct KeptInAFile {
    schedule_id: String,
    profile_id: String,
    say: String,
    every_minutes: u32,
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    last_claimed_ms: Option<u64>,
    #[serde(default)]
    chat_id: Option<String>,
}

fn now_ms() -> u64 {
    u64::try_from(crate::chat_ledger::now_ms()).unwrap_or(0)
}

/// What a chat is called when nobody named it and a schedule began it.
fn titled(say: &str) -> String {
    let line = say.lines().next().unwrap_or_default().trim();
    line.chars().take(120).collect()
}

impl WorkbenchShellState {
    /// Keep time: bring in what the old clock kept under `kept_in_files`,
    /// take the keeper's lock beside the ledger, and look from now on. A
    /// process that finds the lock taken keeps no time; the one that holds
    /// it does.
    ///
    /// # Errors
    ///
    /// Fails when the old schedules cannot be brought in, and when time is
    /// kept here already.
    pub async fn keep_time(
        self: &Arc<Self>,
        kept_in_files: &Path,
    ) -> Result<(), WorkbenchShellError> {
        self.bring_in_schedules(kept_in_files).await?;
        let lock_path = self.ledger_path.with_extension("timekeeper.lock");
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(|error| {
                WorkbenchShellError::Failed(format!("{}: {error}", lock_path.display()))
            })?;
        if lock.try_lock().is_err() {
            // Somebody else keeps time over this ledger.
            return Ok(());
        }
        self.timekeeper
            .set(lock)
            .map_err(|_| WorkbenchShellError::Conflict("time is kept here already".into()))?;
        self.follow_runs_left_going().await;
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(LOOK);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                state.refuse_what_nobody_answered(now_ms()).await;
                for due in state.claim_what_is_due(now_ms()).await {
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        state.say_what_is_due(due).await;
                    });
                }
            }
        });
        Ok(())
    }

    /// The schedules of the old clock become schedules of the ledger, and
    /// their directory is set aside.
    async fn bring_in_schedules(&self, kept_in_files: &Path) -> Result<(), WorkbenchShellError> {
        if !kept_in_files.is_dir() {
            return Ok(());
        }
        let mut kept = Vec::new();
        for entry in std::fs::read_dir(kept_in_files)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
        {
            let path = entry
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?
                .path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(&path)
                .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
            // A file that is not a schedule is left where it is, in the
            // directory that is set aside; nothing is lost and nothing
            // that cannot be read is said on time.
            if let Ok(schedule) = serde_json::from_slice::<KeptInAFile>(&bytes) {
                kept.push(schedule);
            }
        }
        kept.sort_by(|left, right| left.schedule_id.cmp(&right.schedule_id));
        let known: Vec<String> = self
            .profiles()?
            .into_iter()
            .map(|profile| profile.profile_id)
            .collect();
        self.with_ledger(move |ledger| {
            let owner = ledger.owner()?;
            for old in kept {
                if !known.contains(&old.profile_id) {
                    continue;
                }
                let agent = ledger.agent_of_profile(&old.profile_id)?;
                // The chat it spoke in, when the agent is still in it.
                let chat_id = match old.chat_id {
                    Some(chat_id)
                        if ledger.chat(&chat_id).is_ok_and(|chat| {
                            chat.members
                                .iter()
                                .any(|member| member.participant_id == agent.participant_id)
                        }) =>
                    {
                        chat_id
                    }
                    _ => {
                        ledger
                            .start_chat(
                                &old.schedule_id,
                                &owner.participant_id,
                                std::slice::from_ref(&agent.participant_id),
                            )?
                            .chat_id
                    }
                };
                let when = When::Every {
                    minutes: old.every_minutes.max(1),
                };
                let made = old.last_claimed_ms.unwrap_or_else(now_ms);
                let brought = ledger.keep_schedule_made_at(
                    &NewTimedMessage {
                        agent_id: agent.participant_id,
                        made_by: owner.participant_id.clone(),
                        chat_id: Some(chat_id),
                        say: old.say,
                        when: when.clone(),
                    },
                    made,
                    when.next_after(made, made),
                )?;
                if !old.enabled {
                    ledger.change_schedule(
                        &brought.schedule_id,
                        &TimedMessageChange {
                            enabled: Some(false),
                            ..Default::default()
                        },
                    )?;
                }
            }
            Ok(())
        })
        .await
        .map_err(ledger_refusal)?;
        let aside = kept_in_files.with_extension("v1");
        std::fs::rename(kept_in_files, &aside).map_err(|error| {
            WorkbenchShellError::Failed(format!(
                "the old schedules were brought in and could not be set aside at {}: {error}",
                aside.display()
            ))
        })
    }

    /// Runs a keeper that stopped left going: each is followed to the end
    /// of its turn, or ended here when it was never said.
    async fn follow_runs_left_going(self: &Arc<Self>) {
        let Ok(going) = self.with_ledger(crate::RoutingLedger::runs_going).await else {
            return;
        };
        for run in going {
            let state = Arc::clone(self);
            tokio::spawn(async move {
                match run.delivery_id.clone() {
                    Some(delivery_id) => {
                        state
                            .follow_to_its_end(&run.schedule_id, run.due_ms, &delivery_id)
                            .await;
                    }
                    None => {
                        state
                            .end_run(
                                &run.schedule_id,
                                run.due_ms,
                                RunState::Failed,
                                Some("SWEM stopped before it was said".into()),
                            )
                            .await;
                    }
                }
            });
        }
    }

    /// Take up what is due at a moment. The keeper does it as time passes;
    /// a test does it at a moment of its choosing.
    pub async fn claim_what_is_due(&self, at_ms: u64) -> Vec<Due> {
        self.with_ledger(move |ledger| ledger.claim_due(at_ms))
            .await
            .unwrap_or_default()
    }

    async fn end_run(&self, schedule_id: &str, due_ms: u64, state: RunState, note: Option<String>) {
        let schedule_id = schedule_id.to_owned();
        let _ = self
            .with_ledger(move |ledger| {
                ledger.run_ended(&schedule_id, due_ms, state, note.as_deref())
            })
            .await;
    }

    async fn follow_to_its_end(&self, schedule_id: &str, due_ms: u64, delivery_id: &str) {
        let (state, note) = match self.ended(delivery_id).await {
            Ok(delivery) => match delivery.state {
                DeliveryState::Done => (RunState::Answered, None),
                DeliveryState::Stopped => (RunState::Stopped, delivery.outcome),
                _ => (RunState::Failed, delivery.outcome),
            },
            Err(error) => (RunState::Failed, Some(error.to_string())),
        };
        self.end_run(schedule_id, due_ms, state, note).await;
    }

    /// Say what is due into its chat, as the schedule, and follow the turn
    /// to its end. It returns when the run has ended.
    pub async fn say_what_is_due(self: &Arc<Self>, due: Due) {
        let (schedule_id, due_ms) = (due.schedule.schedule_id.clone(), due.due_ms);
        match self.said_on_time(&due).await {
            Ok(delivery_id) => {
                self.follow_to_its_end(&schedule_id, due_ms, &delivery_id)
                    .await;
            }
            Err(error) => {
                self.end_run(
                    &schedule_id,
                    due_ms,
                    RunState::Failed,
                    Some(error.to_string()),
                )
                .await;
            }
        }
    }

    async fn said_on_time(self: &Arc<Self>, due: &Due) -> Result<String, WorkbenchShellError> {
        let schedule = due.schedule.clone();
        let (chat_id, named) = self
            .with_ledger(move |ledger| {
                let chat_id = if let Some(chat_id) = &schedule.chat_id {
                    chat_id.clone()
                } else {
                    let owner = ledger.owner()?;
                    ledger
                        .start_chat(
                            &titled(&schedule.say),
                            &owner.participant_id,
                            std::slice::from_ref(&schedule.agent_id),
                        )?
                        .chat_id
                };
                ledger.join_chat(&chat_id, &schedule.speaker_id)?;
                // In a chat of several an agent answers when it is named.
                let chat = ledger.chat(&chat_id)?;
                let several = chat
                    .members
                    .iter()
                    .filter(|member| member.kind == ParticipantKind::Agent && !member.retired)
                    .count()
                    > 1;
                let named = if several {
                    chat.members
                        .iter()
                        .find(|member| member.participant_id == schedule.agent_id)
                        .map(|agent| agent.handle.clone())
                } else {
                    None
                };
                Ok((chat_id, named))
            })
            .await
            .map_err(ledger_refusal)?;
        let text = match named {
            Some(handle) if !due.schedule.say.contains(&format!("@{handle}")) => {
                format!("@{handle} {}", due.schedule.say)
            }
            _ => due.schedule.say.clone(),
        };
        let said = self
            .say_in_chat_as(
                &chat_id,
                Some(due.schedule.speaker_id.clone()),
                crate::CHANNEL_SCHEDULE,
                Saying {
                    text,
                    blocks: Vec::new(),
                    content_refs: Vec::new(),
                    context: None,
                    // The moment it was due names the message, so what is
                    // said twice for one moment is said once.
                    client_ref: Some(format!("{}@{}", due.schedule.schedule_id, due.due_ms)),
                },
            )
            .await?;
        let owed = said
            .deliveries
            .iter()
            .find(|delivery| delivery.agent_id == due.schedule.agent_id)
            .ok_or_else(|| {
                WorkbenchShellError::Conflict(
                    "its agent is no longer in the chat it is said in".into(),
                )
            })?;
        let (schedule_id, due_ms, delivery_id) = (
            due.schedule.schedule_id.clone(),
            due.due_ms,
            owed.delivery_id.clone(),
        );
        let said_as = delivery_id.clone();
        self.with_ledger(move |ledger| ledger.run_said(&schedule_id, due_ms, &chat_id, &said_as))
            .await
            .map_err(ledger_refusal)?;
        Ok(delivery_id)
    }

    /// Answer, with the engine's own refusal, what agents asked in turns
    /// schedules began and nobody came to answer by `at_ms`.
    pub async fn refuse_what_nobody_answered(&self, at_ms: u64) {
        let asked_by = at_ms.saturating_sub(WAITS_FOR_A_PERSON_MS);
        let Ok(left) = self
            .with_ledger(move |ledger| ledger.questions_left_to_schedules(asked_by))
            .await
        else {
            return;
        };
        for LeftUnanswered {
            question_id,
            kind,
            asked,
            speaker_id,
        } in left
        {
            let answer = if kind == "permission" {
                let options = asked.get("options").and_then(Value::as_array);
                let refusal = options.into_iter().flatten().find(|option| {
                    option
                        .get("kind")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| kind.starts_with("reject"))
                });
                match refusal.and_then(|option| option.get("optionId")) {
                    Some(option) => json!({ "option": option }),
                    // An engine that offers no way to refuse is stopped.
                    None => continue,
                }
            } else {
                json!({ "action": "decline" })
            };
            let _ = self
                .answer_in_chat_by(&question_id, answer, Some(speaker_id))
                .await;
        }
    }

    async fn shown(
        &self,
        schedules: Vec<TimedMessage>,
    ) -> Result<Vec<ScheduleShown>, WorkbenchShellError> {
        self.with_ledger(move |ledger| {
            schedules
                .into_iter()
                .map(|schedule| {
                    let made_by_name = ledger
                        .participant(&schedule.made_by)
                        .map_or_else(|_| String::new(), |maker| maker.name);
                    let chat_title = schedule.chat_id.as_ref().map(|chat_id| {
                        ledger
                            .chat(chat_id)
                            .ok()
                            .map(|chat| chat.title)
                            .filter(|title| !title.is_empty())
                            .unwrap_or_else(|| "Its chat".to_owned())
                    });
                    Ok(ScheduleShown {
                        schedule,
                        made_by_name,
                        chat_title,
                    })
                })
                .collect()
        })
        .await
        .map_err(ledger_refusal)
    }

    /// An agent's schedules and their last runs, or everybody's.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn schedules_shown(
        &self,
        agent_id: Option<String>,
    ) -> Result<(Vec<ScheduleShown>, Vec<TimedRun>), WorkbenchShellError> {
        let (schedules, runs) = self
            .with_ledger(move |ledger| {
                Ok((
                    ledger.schedules(agent_id.as_deref())?,
                    ledger.runs(agent_id.as_deref(), 20)?,
                ))
            })
            .await
            .map_err(ledger_refusal)?;
        Ok((self.shown(schedules).await?, runs))
    }

    /// Make a schedule, as the person the Workbench is.
    ///
    /// # Errors
    ///
    /// Refuses what cannot be kept, in words.
    pub async fn make_schedule(
        &self,
        body: NewScheduleBody,
    ) -> Result<ScheduleShown, WorkbenchShellError> {
        let kept = self
            .with_ledger(move |ledger| {
                let made_by = ledger.owner()?.participant_id;
                ledger.keep_schedule(&NewTimedMessage {
                    agent_id: body.agent_id,
                    made_by,
                    chat_id: body.chat_id,
                    say: body.say,
                    when: body.when,
                })
            })
            .await
            .map_err(ledger_refusal)?;
        Ok(self.shown(vec![kept]).await?.remove(0))
    }

    /// Change a schedule, or turn it off or on.
    ///
    /// # Errors
    ///
    /// Refuses a schedule that is not here and a change that cannot be kept.
    pub async fn change_schedule(
        &self,
        schedule_id: &str,
        change: TimedMessageChange,
    ) -> Result<ScheduleShown, WorkbenchShellError> {
        let schedule_id = schedule_id.to_owned();
        let kept = self
            .with_ledger(move |ledger| ledger.change_schedule(&schedule_id, &change))
            .await
            .map_err(ledger_refusal)?;
        Ok(self.shown(vec![kept]).await?.remove(0))
    }

    /// Forget a schedule.
    ///
    /// # Errors
    ///
    /// Refuses a schedule that is not here.
    pub async fn forget_schedule(&self, schedule_id: &str) -> Result<(), WorkbenchShellError> {
        let schedule_id = schedule_id.to_owned();
        self.with_ledger(move |ledger| ledger.forget_schedule(&schedule_id))
            .await
            .map_err(ledger_refusal)
    }

    /// Who keeps time and for whom.
    ///
    /// # Errors
    ///
    /// Returns [`WorkbenchShellError`] when the ledger cannot be read.
    pub async fn keeper(&self) -> Result<KeeperStanding, WorkbenchShellError> {
        let (schedules, agents) = self
            .with_ledger(|ledger| {
                let schedules = ledger.schedules(None)?;
                let mut agents = Vec::new();
                for schedule in &schedules {
                    if let Some(profile) = ledger.participant(&schedule.agent_id)?.profile_id
                        && !agents.contains(&profile)
                    {
                        agents.push(profile);
                    }
                }
                Ok((schedules.len(), agents))
            })
            .await
            .map_err(ledger_refusal)?;
        Ok(KeeperStanding {
            keeping: self.timekeeper.get().is_some(),
            zone: When::zone_here(),
            used_by: agents,
            schedules,
        })
    }
}
