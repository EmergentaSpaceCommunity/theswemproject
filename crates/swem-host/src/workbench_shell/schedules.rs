//! A clock that writes into a conversation.
//!
//! Everything else in this host happens because somebody asked for it. A
//! schedule is the one thing that happens because time passed, and that makes
//! it the last surface: it says something nobody typed. A schedule is a
//! participant of the chat it speaks in, so the record says the clock said
//! it and not a person, and it speaks with the trust of whoever made it.
//!
//! The clock is the product's. A product that is not running does not fire,
//! and a product that was off does not catch up - it fires once when it comes
//! back and carries on. That is not a limitation to be worked around: a
//! person who closed their laptop on Friday does not want Monday to open with
//! seventy-two turns. The claim is written before the turn runs, so a crash
//! mid-turn loses the turn rather than repeating it forever.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{WorkbenchShellError, WorkbenchShellState};

const SCHEDULE_SCHEMA: &str = "swem.workbench-schedule.v1";
/// The surface a scheduled turn is written from. A person reading the lane
/// sees this where another turn would name a browser or a channel.
pub const SCHEDULE_SURFACE: &str = "schedule";
/// How often the clock looks. Nothing here is to the second, and a schedule
/// is late by at most this.
const TICK: std::time::Duration = std::time::Duration::from_secs(10);

/// One standing instruction: what to say, to whom, and how often.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Schedule {
    pub schema: String,
    pub schedule_id: String,
    pub profile_id: String,
    /// What the clock writes. Plain words, exactly as a person would type
    /// them, because that is what the agent receives.
    pub say: String,
    pub every_minutes: u32,
    pub enabled: bool,
    /// When this schedule last claimed a turn, in milliseconds since the
    /// epoch. Written before the turn runs, which is what makes a missed
    /// window one turn rather than a queue of them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_claimed_ms: Option<u64>,
    /// The lane this schedule writes into, once it has one. The same
    /// conversation every time, so a person reads it as one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_id: Option<String>,
    /// The chat this schedule speaks in, once it has one. The same chat
    /// every time, so a person reads it as one conversation, whatever
    /// session of the engine it is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    /// What happened last time, in a sentence a person can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_outcome: Option<String>,
}

/// What a person fills in to set one.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetScheduleBody {
    /// The schedule's name, which is also what the record calls its author.
    pub schedule_id: String,
    pub say: String,
    pub every_minutes: u32,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// The schedules this host keeps, as one JSON document each.
#[derive(Clone, Debug)]
pub struct ScheduleBook {
    root: PathBuf,
}

impl ScheduleBook {
    /// Open the directory the schedules live in.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory cannot be made or canonicalized.
    pub fn open(root: &Path) -> Result<Self, WorkbenchShellError> {
        std::fs::create_dir_all(root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let root = std::fs::canonicalize(root)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        Ok(Self { root })
    }

    fn path_of(&self, schedule_id: &str) -> Result<PathBuf, WorkbenchShellError> {
        crate::profile::validate_id("schedule id", schedule_id)
            .map_err(WorkbenchShellError::Invalid)?;
        Ok(self.root.join(format!("{schedule_id}.json")))
    }

    /// Every schedule, by name.
    ///
    /// # Errors
    ///
    /// Fails when the directory or a document in it cannot be read.
    pub fn list(&self) -> Result<Vec<Schedule>, WorkbenchShellError> {
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&self.root)
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
            let schedule: Schedule = serde_json::from_slice(&bytes).map_err(|error| {
                WorkbenchShellError::Failed(format!(
                    "{} is not a schedule: {error}",
                    path.display()
                ))
            })?;
            if schedule.schema != SCHEDULE_SCHEMA {
                return Err(WorkbenchShellError::Failed(format!(
                    "{} has schema {}, which this host does not read",
                    path.display(),
                    schedule.schema
                )));
            }
            found.push(schedule);
        }
        found.sort_by(|left, right| left.schedule_id.cmp(&right.schedule_id));
        Ok(found)
    }

    fn write(&self, schedule: &Schedule) -> Result<(), WorkbenchShellError> {
        let path = self.path_of(&schedule.schedule_id)?;
        let bytes = serde_json::to_vec_pretty(schedule)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        let temporary = path.with_extension("json.writing");
        std::fs::write(&temporary, &bytes)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))?;
        std::fs::rename(&temporary, &path)
            .map_err(|error| WorkbenchShellError::Failed(error.to_string()))
    }

    /// Set a schedule, or correct one already here. A corrected schedule
    /// keeps its lane and its claim, so changing the wording does not make it
    /// fire again at once.
    ///
    /// # Errors
    ///
    /// Refuses an unusable name, empty words, and an interval of no minutes.
    pub fn set(
        &self,
        profile_id: &str,
        body: &SetScheduleBody,
    ) -> Result<Schedule, WorkbenchShellError> {
        if body.say.trim().is_empty() {
            return Err(WorkbenchShellError::Invalid(
                "a schedule that says nothing would tell the agent nothing".into(),
            ));
        }
        if body.every_minutes == 0 {
            return Err(WorkbenchShellError::Invalid(
                "a schedule needs an interval of at least a minute".into(),
            ));
        }
        let existing = self
            .list()?
            .into_iter()
            .find(|schedule| schedule.schedule_id == body.schedule_id);
        let schedule = Schedule {
            schema: SCHEDULE_SCHEMA.into(),
            schedule_id: body.schedule_id.clone(),
            profile_id: profile_id.to_owned(),
            say: body.say.clone(),
            every_minutes: body.every_minutes,
            enabled: true,
            // A schedule fires once as soon as it is set, so a person sees
            // what it does instead of waiting an interval to find out, and
            // then keeps to its interval.
            last_claimed_ms: existing.as_ref().and_then(|old| old.last_claimed_ms),
            route_id: existing.as_ref().and_then(|old| old.route_id.clone()),
            chat_id: existing.as_ref().and_then(|old| old.chat_id.clone()),
            last_outcome: existing.and_then(|old| old.last_outcome),
        };
        self.write(&schedule)?;
        Ok(schedule)
    }

    /// Forget a schedule. The lane it wrote into is left alone: what was said
    /// was said.
    ///
    /// # Errors
    ///
    /// Refuses an unusable name and a schedule that is not here.
    pub fn forget(&self, schedule_id: &str) -> Result<(), WorkbenchShellError> {
        let path = self.path_of(schedule_id)?;
        std::fs::remove_file(&path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => {
                WorkbenchShellError::NotFound(format!("no schedule called {schedule_id}"))
            }
            _ => WorkbenchShellError::Failed(error.to_string()),
        })
    }

    /// The schedules whose window has passed, each claimed as it is taken.
    /// Claiming first is what keeps a slow turn from being started twice and
    /// a product that was off from catching up.
    /// [`Self::claim_due`] by another name, so a test can state the claim
    /// rule at an exact clock instead of waiting for one.
    #[doc(hidden)]
    pub fn claim_due_for_tests(&self, now: u64) -> Result<Vec<Schedule>, WorkbenchShellError> {
        self.claim_due(now)
    }

    fn claim_due(&self, now: u64) -> Result<Vec<Schedule>, WorkbenchShellError> {
        let mut due = Vec::new();
        for mut schedule in self.list()? {
            if !schedule.enabled {
                continue;
            }
            let window = u64::from(schedule.every_minutes) * 60_000;
            let ready = schedule
                .last_claimed_ms
                .is_none_or(|last| now.saturating_sub(last) >= window);
            if !ready {
                continue;
            }
            schedule.last_claimed_ms = Some(now);
            self.write(&schedule)?;
            due.push(schedule);
        }
        Ok(due)
    }

    fn record_outcome(&self, schedule_id: &str, spoke_in: Option<&SpokeIn>, outcome: &str) {
        let Ok(mut schedules) = self.list() else {
            return;
        };
        let Some(schedule) = schedules
            .iter_mut()
            .find(|schedule| schedule.schedule_id == schedule_id)
        else {
            return;
        };
        if let Some(spoke_in) = spoke_in {
            schedule.chat_id = Some(spoke_in.chat_id.clone());
            if let Some(route_id) = &spoke_in.route_id {
                schedule.route_id = Some(route_id.clone());
            }
        }
        schedule.last_outcome = Some(outcome.to_owned());
        let _ = self.write(schedule);
    }
}

/// Where a schedule spoke: the chat, and the session of the engine the chat
/// was in when the turn ended.
struct SpokeIn {
    chat_id: String,
    route_id: Option<String>,
}

impl WorkbenchShellState {
    /// Keep this host's schedules in `root` and start the clock that runs
    /// them. The clock lives as long as the state does.
    ///
    /// # Errors
    ///
    /// Fails closed when the directory is unusable or schedules are already
    /// enabled here.
    pub fn enable_schedules(self: &Arc<Self>, root: &Path) -> Result<(), WorkbenchShellError> {
        let book = ScheduleBook::open(root)?;
        self.schedules
            .set(book)
            .map_err(|_| WorkbenchShellError::Conflict("schedules are already enabled".into()))?;
        let state = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(TICK);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                state.run_due_schedules();
            }
        });
        Ok(())
    }

    fn schedule_book(&self) -> Result<&ScheduleBook, WorkbenchShellError> {
        self.schedules
            .get()
            .ok_or_else(|| WorkbenchShellError::NotFound("this host keeps no schedules".into()))
    }

    /// The schedules of one profile.
    ///
    /// # Errors
    ///
    /// Fails when schedules are not enabled or cannot be read.
    pub fn profile_schedules(
        &self,
        profile_id: &str,
    ) -> Result<Vec<Schedule>, WorkbenchShellError> {
        Ok(self
            .schedule_book()?
            .list()?
            .into_iter()
            .filter(|schedule| schedule.profile_id == profile_id)
            .collect())
    }

    /// Set one on a profile.
    ///
    /// # Errors
    ///
    /// Refuses an unknown profile and an unusable schedule.
    pub fn set_schedule(
        &self,
        profile_id: &str,
        body: &SetScheduleBody,
    ) -> Result<Schedule, WorkbenchShellError> {
        self.inventory
            .select(profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        self.schedule_book()?.set(profile_id, body)
    }

    /// Forget one.
    ///
    /// # Errors
    ///
    /// Refuses a schedule that is not here.
    pub fn forget_schedule(&self, schedule_id: &str) -> Result<(), WorkbenchShellError> {
        self.schedule_book()?.forget(schedule_id)
    }

    /// Say everything whose window has passed. Each is a message into the
    /// schedule's chat, said by the schedule and answered like any other.
    /// One that waits for an answer holds nobody else up: each is followed
    /// to its end by itself.
    fn run_due_schedules(self: &Arc<Self>) {
        let Ok(book) = self.schedule_book() else {
            return;
        };
        let Ok(due) = book.claim_due(now_ms()) else {
            return;
        };
        for schedule in due {
            let state = Arc::clone(self);
            tokio::spawn(async move {
                let ran = state.run_schedule(&schedule).await;
                let Ok(book) = state.schedule_book() else {
                    return;
                };
                match ran {
                    Ok((spoke_in, said)) => {
                        book.record_outcome(&schedule.schedule_id, Some(&spoke_in), &said);
                    }
                    Err(error) => {
                        book.record_outcome(&schedule.schedule_id, None, &error.to_string());
                    }
                }
            });
        }
    }

    async fn run_schedule(
        self: &Arc<Self>,
        schedule: &Schedule,
    ) -> Result<(SpokeIn, String), WorkbenchShellError> {
        self.inventory
            .select(&schedule.profile_id)
            .map_err(|error| WorkbenchShellError::NotFound(error.to_string()))?;
        let (name, profile_id, chat_id, route_id) = (
            schedule.schedule_id.clone(),
            schedule.profile_id.clone(),
            schedule.chat_id.clone(),
            schedule.route_id.clone(),
        );
        let (chat_id, agent_id, clock) = self
            .with_ledger(move |ledger| {
                let owner = ledger.owner()?;
                let agent = ledger.agent_of_profile(&profile_id)?;
                let clock = ledger.schedule_participant(&name, &owner.participant_id)?;
                // The chat it spoke in before; or the chat of the lane it
                // wrote into before there were chats; or a chat of its own.
                let known = match (chat_id, route_id) {
                    (Some(chat_id), _) => Some(chat_id),
                    (None, Some(route_id)) => ledger
                        .session_of_route(&route_id)?
                        .map(|session| session.chat_id),
                    (None, None) => None,
                };
                let chat_id = match known {
                    Some(chat_id) => chat_id,
                    None => {
                        ledger
                            .start_chat(
                                &name,
                                &owner.participant_id,
                                std::slice::from_ref(&agent.participant_id),
                            )?
                            .chat_id
                    }
                };
                ledger.join_chat(&chat_id, &clock.participant_id)?;
                Ok((chat_id, agent.participant_id, clock.participant_id))
            })
            .await
            .map_err(super::chats::ledger_refusal)?;
        let said = self
            .say_in_chat_as(
                &chat_id,
                Some(clock),
                crate::CHANNEL_SCHEDULE,
                super::Saying {
                    text: schedule.say.clone(),
                    blocks: Vec::new(),
                    content_refs: Vec::new(),
                    // The window it claimed names the message, so a clock
                    // that says the same window twice has said it once.
                    client_ref: Some(format!(
                        "{}@{}",
                        schedule.schedule_id,
                        schedule.last_claimed_ms.unwrap_or(0)
                    )),
                },
            )
            .await?;
        let Some(owed) = said
            .deliveries
            .iter()
            .find(|delivery| delivery.agent_id == agent_id)
        else {
            return Err(WorkbenchShellError::Conflict(format!(
                "nobody in the chat answers {}: its agent is one of several there and was not named",
                schedule.schedule_id
            )));
        };
        let ended = self.ended(&owed.delivery_id).await?;
        let (chat, agent) = (chat_id.clone(), agent_id);
        let route_id = self
            .with_ledger(move |ledger| ledger.current_session(&chat, &agent))
            .await
            .map_err(super::chats::ledger_refusal)?
            .map(|session| session.route_id);
        let how = serde_json::to_value(ended.state)
            .ok()
            .and_then(|state| state.as_str().map(str::to_owned))
            .unwrap_or_default();
        Ok((
            SpokeIn { chat_id, route_id },
            match ended.outcome {
                Some(outcome) => format!("ran at {} ({how}: {outcome})", now_ms()),
                None => format!("ran at {} ({how})", now_ms()),
            },
        ))
    }
}
