//! Schedules: messages that arrive on time.
//!
//! A schedule is what is said, when, to which agent, in which chat, and who
//! made it. It is a participant of the chat it speaks in, so what it says is
//! under its own name. Time is never kept inside an agent's machine: the
//! schedules live in the ledger, and whoever keeps time asks the ledger what
//! is due.
//!
//! A run is claimed by the schedule and the moment it was due, in one
//! transaction, so nothing is said twice whoever looks. A schedule that was
//! missed is said once, late, and not once for every time it was missed; one
//! missed by more than a day is not said at all. A schedule whose last run
//! has not ended is passed over.

use std::str::FromStr as _;

use chrono::{TimeZone as _, Utc};
use rusqlite::{OptionalExtension as _, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

use crate::chat_ledger::{new_id, now_ms};
use crate::{RoutingError, RoutingLedger};

/// The tables of time. They stand beside the rest of the ledger and are
/// made when they are missing, so a ledger written before them is opened
/// as it is and a product built before them opens this one.
pub(crate) const TIME_SCHEMA: &str = "
  CREATE TABLE IF NOT EXISTS schedules (
    schedule_id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL REFERENCES participants(participant_id),
    speaker_id TEXT NOT NULL REFERENCES participants(participant_id),
    made_by TEXT NOT NULL REFERENCES participants(participant_id),
    chat_id TEXT REFERENCES chats(chat_id),
    say TEXT NOT NULL,
    when_json TEXT NOT NULL,
    enabled INTEGER NOT NULL,
    created_ms INTEGER NOT NULL,
    next_due_ms INTEGER
  );
  CREATE INDEX IF NOT EXISTS schedules_due ON schedules(enabled, next_due_ms);
  CREATE TABLE IF NOT EXISTS schedule_runs (
    schedule_id TEXT NOT NULL,
    due_ms INTEGER NOT NULL,
    claimed_ms INTEGER NOT NULL,
    agent_id TEXT NOT NULL,
    say TEXT NOT NULL,
    chat_id TEXT,
    delivery_id TEXT,
    ended_ms INTEGER,
    state TEXT NOT NULL,
    note TEXT,
    PRIMARY KEY (schedule_id, due_ms)
  );
  CREATE INDEX IF NOT EXISTS schedule_runs_agent ON schedule_runs(agent_id, due_ms);";

/// A run that began this long after it was due is said to have run late.
const LATE_AFTER_MS: u64 = 2 * 60_000;
/// A run that would begin this long after it was due is not begun.
const TOO_LATE_MS: u64 = 24 * 60 * 60_000;
/// An agent makes a schedule no more often than this, in minutes.
const AN_AGENTS_LEAST_MINUTES: u64 = 5;
/// An agent has at most this many schedules of its own.
const AN_AGENTS_MOST: usize = 20;
/// The most words a schedule says.
const MOST_WORDS: usize = 16_000;

/// When a schedule is due.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum When {
    /// Every so many minutes, counted from when it was made.
    Every { minutes: u32 },
    /// By a cron line of five fields, in a named zone.
    Cron { line: String, zone: String },
    /// Once, at a moment.
    Once { at_ms: u64 },
}

const DAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

fn moment(ms: u64) -> Option<chrono::DateTime<Utc>> {
    Utc.timestamp_millis_opt(i64::try_from(ms).ok()?).single()
}

impl When {
    /// The zone of this machine, by its name; the universal one when the
    /// system does not say.
    #[must_use]
    pub fn zone_here() -> String {
        iana_time_zone::get_timezone()
            .ok()
            .filter(|zone| zone.parse::<chrono_tz::Tz>().is_ok())
            .unwrap_or_else(|| "UTC".to_owned())
    }

    fn cron(line: &str, zone: &str) -> Result<(croner::Cron, chrono_tz::Tz), String> {
        if line.split_whitespace().count() != 5 {
            return Err(format!(
                "a cron line has five fields - minute, hour, day, month, day of the week: {line:?}"
            ));
        }
        let cron = croner::Cron::from_str(line)
            .map_err(|error| format!("{line:?} is not a cron line: {error}"))?;
        let zone = zone
            .parse::<chrono_tz::Tz>()
            .map_err(|_| format!("{zone:?} is not the name of a time zone"))?;
        Ok((cron, zone))
    }

    /// Whether it can be kept.
    ///
    /// # Errors
    ///
    /// Says in words what is wrong with it.
    pub fn check(&self) -> Result<(), String> {
        match self {
            Self::Every { minutes: 0 } => {
                Err("a schedule is at least a minute apart from itself".into())
            }
            Self::Every { minutes } if *minutes > 60 * 24 * 366 => {
                Err("a schedule is at most a year apart from itself".into())
            }
            Self::Every { .. } => Ok(()),
            Self::Cron { line, zone } => Self::cron(line, zone).map(|_| ()),
            Self::Once { at_ms } => moment(*at_ms)
                .map(|_| ())
                .ok_or_else(|| "that moment cannot be kept".to_owned()),
        }
    }

    /// The least time between two of its moments, in minutes; nothing for
    /// one that is due once.
    #[must_use]
    pub fn least_apart(&self, from_ms: u64) -> Option<u64> {
        match self {
            Self::Every { minutes } => Some(u64::from(*minutes)),
            Self::Once { .. } => None,
            Self::Cron { .. } => {
                // The nearest two of its next few moments.
                let mut at = from_ms;
                let mut least: Option<u64> = None;
                for _ in 0..8 {
                    let next = self.next_after(at, from_ms)?;
                    if at != from_ms {
                        let apart = (next - at) / 60_000;
                        least = Some(least.map_or(apart, |known| known.min(apart)));
                    }
                    at = next;
                }
                least
            }
        }
    }

    /// The first moment it is due after `after_ms`. `made_ms` is when the
    /// schedule was made, which is where an interval counts from.
    #[must_use]
    pub fn next_after(&self, after_ms: u64, made_ms: u64) -> Option<u64> {
        match self {
            Self::Every { minutes } => {
                let apart = u64::from(*minutes).max(1) * 60_000;
                let passed = after_ms.saturating_sub(made_ms) / apart;
                Some(made_ms + (passed + 1) * apart)
            }
            Self::Once { at_ms } => (*at_ms > after_ms).then_some(*at_ms),
            Self::Cron { line, zone } => {
                let (cron, zone) = Self::cron(line, zone).ok()?;
                let after = moment(after_ms)?.with_timezone(&zone);
                let next = cron.find_next_occurrence(&after, false).ok()?;
                u64::try_from(next.timestamp_millis()).ok()
            }
        }
    }

    /// When it is due, as a person says it.
    #[must_use]
    pub fn in_words(&self) -> String {
        match self {
            Self::Every { minutes: 1 } => "Every minute".into(),
            Self::Every { minutes } if minutes % (60 * 24) == 0 => match minutes / (60 * 24) {
                1 => "Every day".into(),
                days => format!("Every {days} days"),
            },
            Self::Every { minutes } if minutes % 60 == 0 => match minutes / 60 {
                1 => "Every hour".into(),
                hours => format!("Every {hours} hours"),
            },
            Self::Every { minutes } => format!("Every {minutes} minutes"),
            Self::Once { at_ms } => moment(*at_ms).map_or_else(
                || "Once".to_owned(),
                |at| {
                    let zone = Self::zone_here()
                        .parse::<chrono_tz::Tz>()
                        .unwrap_or(chrono_tz::UTC);
                    format!(
                        "Once, {}",
                        at.with_timezone(&zone).format("%-d %B %Y at %H:%M")
                    )
                },
            ),
            Self::Cron { line, zone } => {
                let fields: Vec<&str> = line.split_whitespace().collect();
                let number = |field: &str| field.parse::<u32>().ok();
                let said = match fields.as_slice() {
                    [minute, hour, "*", "*", day] => match (number(minute), number(hour)) {
                        (Some(minute), Some(hour)) if *day == "*" => {
                            Some(format!("Every day at {hour:02}:{minute:02}"))
                        }
                        (Some(minute), Some(hour)) => number(day)
                            .and_then(|day| DAYS.get(usize::try_from(day % 7).ok()?))
                            .map(|day| format!("Every {day} at {hour:02}:{minute:02}")),
                        _ => None,
                    },
                    _ => None,
                };
                let said = said.unwrap_or_else(|| {
                    croner::Cron::from_str(line)
                        .map_or_else(|_| line.clone(), |cron| cron.describe())
                });
                if *zone == Self::zone_here() {
                    said
                } else {
                    format!("{said}, {zone} time")
                }
            }
        }
    }
}

/// A message that arrives on time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TimedMessage {
    pub schedule_id: String,
    /// The agent that is told.
    pub agent_id: String,
    /// The participant the schedule speaks as.
    pub speaker_id: String,
    pub made_by: String,
    /// The chat it is said in; nothing for a new chat each time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    pub say: String,
    pub when: When,
    /// `when`, as a person says it.
    pub when_in_words: String,
    pub enabled: bool,
    pub created_ms: u64,
    /// When it is next due; nothing when it is off or will not be due again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due_ms: Option<u64>,
}

/// What somebody gives to make a schedule.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NewTimedMessage {
    pub agent_id: String,
    pub made_by: String,
    #[serde(default)]
    pub chat_id: Option<String>,
    pub say: String,
    pub when: When,
}

/// What somebody changes about a schedule. What is left out stays.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TimedMessageChange {
    #[serde(default)]
    pub say: Option<String>,
    #[serde(default)]
    pub when: Option<When>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// How a run stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Said, and the agent has not finished answering.
    Running,
    Answered,
    /// The agent could not answer.
    Failed,
    /// Somebody stopped the turn.
    Stopped,
    /// Not said: the run before it had not ended, or it was too late.
    Skipped,
}

impl RunState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Answered => "answered",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            Self::Skipped => "skipped",
        }
    }

    fn named(name: &str) -> Self {
        match name {
            "answered" => Self::Answered,
            "failed" => Self::Failed,
            "stopped" => Self::Stopped,
            "skipped" => Self::Skipped,
            _ => Self::Running,
        }
    }
}

/// One time a schedule was due.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TimedRun {
    pub schedule_id: String,
    pub agent_id: String,
    /// What was said, as it was then.
    pub say: String,
    pub due_ms: u64,
    /// When the keeper took it up.
    pub claimed_ms: u64,
    /// Whether it was taken up late enough to say so.
    pub late: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
    pub state: RunState,
    /// What there is to say about it, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What the keeper is handed to say: a schedule, and the moment it was due.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Due {
    pub schedule: TimedMessage,
    pub due_ms: u64,
    pub late: bool,
}

fn unsigned(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn signed(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

const SCHEDULE_COLUMNS: &str = "schedule_id, agent_id, speaker_id, made_by, chat_id, say, \
                                when_json, enabled, created_ms, next_due_ms";

fn schedule_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<(TimedMessage, String)> {
    Ok((
        TimedMessage {
            schedule_id: row.get(0)?,
            agent_id: row.get(1)?,
            speaker_id: row.get(2)?,
            made_by: row.get(3)?,
            chat_id: row.get(4)?,
            say: row.get(5)?,
            when: When::Every { minutes: 1 },
            when_in_words: String::new(),
            enabled: row.get::<_, i64>(7)? != 0,
            created_ms: unsigned(row.get(8)?),
            next_due_ms: row.get::<_, Option<i64>>(9)?.map(unsigned),
        },
        row.get(6)?,
    ))
}

fn with_when(found: (TimedMessage, String)) -> Result<TimedMessage, RoutingError> {
    let (mut schedule, when) = found;
    schedule.when = serde_json::from_str(&when)?;
    schedule.when_in_words = schedule.when.in_words();
    Ok(schedule)
}

const RUN_COLUMNS: &str = "schedule_id, agent_id, say, due_ms, claimed_ms, chat_id, delivery_id, \
                           ended_ms, state, note";

fn run_of(row: &rusqlite::Row<'_>) -> rusqlite::Result<TimedRun> {
    let (due_ms, claimed_ms) = (unsigned(row.get(3)?), unsigned(row.get(4)?));
    Ok(TimedRun {
        schedule_id: row.get(0)?,
        agent_id: row.get(1)?,
        say: row.get(2)?,
        due_ms,
        claimed_ms,
        late: claimed_ms.saturating_sub(due_ms) > LATE_AFTER_MS,
        chat_id: row.get(5)?,
        delivery_id: row.get(6)?,
        ended_ms: row.get::<_, Option<i64>>(7)?.map(unsigned),
        state: RunState::named(&row.get::<_, String>(8)?),
        note: row.get(9)?,
    })
}

fn words(say: &str) -> Result<&str, RoutingError> {
    let said = say.trim();
    if said.is_empty() {
        return Err(RoutingError::InvalidBinding(
            "a schedule that says nothing would tell the agent nothing".into(),
        ));
    }
    if said.chars().count() > MOST_WORDS {
        return Err(RoutingError::InvalidBinding(
            "that is more than a schedule says at once".into(),
        ));
    }
    Ok(said)
}

fn schedule_in(
    transaction: &Transaction<'_>,
    schedule_id: &str,
) -> Result<TimedMessage, RoutingError> {
    let found = transaction
        .query_row(
            &format!("SELECT {SCHEDULE_COLUMNS} FROM schedules WHERE schedule_id = ?1"),
            [schedule_id],
            schedule_of,
        )
        .optional()?
        .ok_or_else(|| {
            RoutingError::InvalidBinding(format!("there is no schedule {schedule_id}"))
        })?;
    with_when(found)
}

impl RoutingLedger {
    /// Keep a schedule. It is first due at its next moment after now.
    ///
    /// # Errors
    ///
    /// Refuses an agent nobody is, a chat the agent is not in, words that
    /// say nothing and a time that cannot be kept.
    pub fn keep_schedule(&mut self, new: &NewTimedMessage) -> Result<TimedMessage, RoutingError> {
        self.keep_schedule_made_at(new, unsigned(now_ms()), None)
    }

    /// Keep a schedule an agent makes for itself: no more often than every
    /// five minutes, and no more than twenty of them.
    ///
    /// # Errors
    ///
    /// As [`Self::keep_schedule`], and refuses what is over an agent's
    /// limits, in words.
    pub fn keep_schedule_of_an_agent(
        &mut self,
        new: &NewTimedMessage,
    ) -> Result<TimedMessage, RoutingError> {
        new.when.check().map_err(RoutingError::InvalidBinding)?;
        if new
            .when
            .least_apart(unsigned(now_ms()))
            .is_some_and(|apart| apart < AN_AGENTS_LEAST_MINUTES)
        {
            return Err(RoutingError::InvalidBinding(format!(
                "a schedule an agent makes is at least {AN_AGENTS_LEAST_MINUTES} minutes apart from itself"
            )));
        }
        let has = self
            .schedules(Some(&new.agent_id))?
            .iter()
            .filter(|schedule| schedule.made_by == new.made_by)
            .count();
        if has >= AN_AGENTS_MOST {
            return Err(RoutingError::InvalidBinding(format!(
                "an agent has at most {AN_AGENTS_MOST} schedules; remove one first"
            )));
        }
        self.keep_schedule(new)
    }

    /// [`Self::keep_schedule`], made at a moment and first due at one: how
    /// a schedule kept before there were these tables is brought in.
    pub(crate) fn keep_schedule_made_at(
        &mut self,
        new: &NewTimedMessage,
        made_ms: u64,
        first_due_ms: Option<u64>,
    ) -> Result<TimedMessage, RoutingError> {
        let say = words(&new.say)?.to_owned();
        new.when.check().map_err(RoutingError::InvalidBinding)?;
        let now = unsigned(now_ms());
        let next_due = first_due_ms.or_else(|| new.when.next_after(now, made_ms));
        if next_due.is_none() {
            return Err(RoutingError::InvalidBinding(
                "that moment has passed already".into(),
            ));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let agent: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM participants
                           WHERE participant_id = ?1 AND kind = 'agent' AND retired_ms IS NULL)",
            [&new.agent_id],
            |row| row.get(0),
        )?;
        if !agent {
            return Err(RoutingError::InvalidBinding(format!(
                "{} is not an agent here",
                new.agent_id
            )));
        }
        if let Some(chat_id) = &new.chat_id {
            let in_it: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_members
                               WHERE chat_id = ?1 AND participant_id = ?2)",
                params![chat_id, new.agent_id],
                |row| row.get(0),
            )?;
            if !in_it {
                return Err(RoutingError::InvalidBinding(
                    "the agent is not in that chat".into(),
                ));
            }
        }
        let schedule_id = new_id("s")?;
        let speaker_id = new_id("p")?;
        // A schedule is called by when it speaks: what it says is under
        // its name in the chat, and would only be said twice.
        let name = new.when.in_words();
        transaction.execute(
            "INSERT INTO participants(participant_id, kind, handle, name, made_by, created_ms)
             VALUES (?1, 'schedule', ?2, ?3, ?4, ?5)",
            params![
                speaker_id,
                crate::chat_ledger::free_handle_in(
                    &transaction,
                    &crate::chat_ledger::handle_from(&name)
                )?,
                name,
                new.made_by,
                signed(made_ms)
            ],
        )?;
        transaction.execute(
            "INSERT INTO schedules(schedule_id, agent_id, speaker_id, made_by, chat_id, say,
                                   when_json, enabled, created_ms, next_due_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9)",
            params![
                schedule_id,
                new.agent_id,
                speaker_id,
                new.made_by,
                new.chat_id,
                say,
                serde_json::to_string(&new.when)?,
                signed(made_ms),
                next_due.map(signed)
            ],
        )?;
        let kept = schedule_in(&transaction, &schedule_id)?;
        transaction.commit()?;
        Ok(kept)
    }

    /// One schedule.
    ///
    /// # Errors
    ///
    /// Refuses a schedule that is not here.
    pub fn schedule(&mut self, schedule_id: &str) -> Result<TimedMessage, RoutingError> {
        let transaction = self.connection.transaction()?;
        let schedule = schedule_in(&transaction, schedule_id)?;
        transaction.commit()?;
        Ok(schedule)
    }

    /// The schedules of an agent, or every schedule, the oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn schedules(&mut self, agent_id: Option<&str>) -> Result<Vec<TimedMessage>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {SCHEDULE_COLUMNS} FROM schedules
             WHERE ?1 IS NULL OR agent_id = ?1 ORDER BY created_ms, schedule_id"
        ))?;
        let found = statement
            .query_map([agent_id], schedule_of)?
            .collect::<Result<Vec<_>, _>>()?;
        found.into_iter().map(with_when).collect()
    }

    /// Change what a schedule says or when, or turn it off or on. Turned on
    /// or given another time, it is next due at its next moment after now.
    ///
    /// # Errors
    ///
    /// Refuses a schedule that is not here and a change that cannot be kept.
    pub fn change_schedule(
        &mut self,
        schedule_id: &str,
        change: &TimedMessageChange,
    ) -> Result<TimedMessage, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let was = schedule_in(&transaction, schedule_id)?;
        let say = match &change.say {
            Some(say) => words(say)?.to_owned(),
            None => was.say.clone(),
        };
        let when = change.when.clone().unwrap_or_else(|| was.when.clone());
        when.check().map_err(RoutingError::InvalidBinding)?;
        let enabled = change.enabled.unwrap_or(was.enabled);
        let next_due = if !enabled {
            None
        } else if was.enabled && when == was.when {
            was.next_due_ms
        } else {
            when.next_after(unsigned(now_ms()), was.created_ms)
        };
        transaction.execute(
            "UPDATE schedules SET say = ?2, when_json = ?3, enabled = ?4, next_due_ms = ?5
             WHERE schedule_id = ?1",
            params![
                schedule_id,
                say,
                serde_json::to_string(&when)?,
                i64::from(enabled && next_due.is_some()),
                next_due.map(signed)
            ],
        )?;
        if when != was.when {
            transaction.execute(
                "UPDATE participants SET name = ?2 WHERE participant_id = ?1",
                params![was.speaker_id, when.in_words()],
            )?;
        }
        let kept = schedule_in(&transaction, schedule_id)?;
        transaction.commit()?;
        Ok(kept)
    }

    /// Forget a schedule. What it said stays said, and its runs stay read.
    ///
    /// # Errors
    ///
    /// Refuses a schedule that is not here.
    pub fn forget_schedule(&mut self, schedule_id: &str) -> Result<(), RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let schedule = schedule_in(&transaction, schedule_id)?;
        transaction.execute(
            "DELETE FROM schedules WHERE schedule_id = ?1",
            [schedule_id],
        )?;
        transaction.execute(
            "UPDATE participants SET retired_ms = ?2 WHERE participant_id = ?1",
            params![schedule.speaker_id, now_ms()],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Take up what is due at `now_ms`. Each schedule that is due is
    /// claimed by the moment it was due and moved to its next moment after
    /// now; what is to be said is handed back, and what is passed over is
    /// written down as passed over.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn claim_due(&mut self, now_ms: u64) -> Result<Vec<Due>, RoutingError> {
        self.claim_due_of(now_ms, None)
    }

    /// The agents a keeper would have something to do for at a moment,
    /// by their profiles: something of theirs is due, or a run of theirs
    /// was left going.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn profiles_with_time_to_keep(&mut self, now_ms: u64) -> Result<Vec<String>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT p.profile_id FROM participants p
             WHERE p.profile_id IS NOT NULL AND (
                 EXISTS(SELECT 1 FROM schedules s
                        WHERE s.agent_id = p.participant_id AND s.enabled = 1
                          AND s.next_due_ms IS NOT NULL AND s.next_due_ms <= ?1)
                 OR EXISTS(SELECT 1 FROM schedule_runs r
                           WHERE r.agent_id = p.participant_id AND r.state = 'running'))
             ORDER BY p.profile_id",
        )?;
        let profiles = statement
            .query_map([signed(now_ms)], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(profiles)
    }

    /// Claim what is due for the agents a keeper keeps time for, and leave
    /// the rest to whoever keeps theirs; every agent when none is named.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn claim_due_of(
        &mut self,
        now_ms: u64,
        agents: Option<&[String]>,
    ) -> Result<Vec<Due>, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let due = {
            let mut statement = transaction.prepare(&format!(
                "SELECT {SCHEDULE_COLUMNS} FROM schedules
                 WHERE enabled = 1 AND next_due_ms IS NOT NULL AND next_due_ms <= ?1
                 ORDER BY next_due_ms, schedule_id"
            ))?;
            statement
                .query_map([signed(now_ms)], schedule_of)?
                .collect::<Result<Vec<_>, _>>()?
        };
        let mut to_say = Vec::new();
        for found in due {
            let schedule = with_when(found)?;
            let Some(due_ms) = schedule.next_due_ms else {
                continue;
            };
            if agents.is_some_and(|agents| !agents.contains(&schedule.agent_id)) {
                continue;
            }
            let going: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM schedule_runs
                               WHERE schedule_id = ?1 AND state = 'running')",
                [&schedule.schedule_id],
                |row| row.get(0),
            )?;
            let passed_over = if going {
                Some("the run before it had not ended")
            } else if now_ms.saturating_sub(due_ms) > TOO_LATE_MS {
                Some("nobody kept time for more than a day after it was due")
            } else {
                None
            };
            let claimed = transaction.execute(
                "INSERT OR IGNORE INTO schedule_runs(schedule_id, due_ms, claimed_ms, agent_id,
                                                     say, chat_id, ended_ms, state, note)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    schedule.schedule_id,
                    signed(due_ms),
                    signed(now_ms),
                    schedule.agent_id,
                    schedule.say,
                    schedule.chat_id,
                    passed_over.map(|_| signed(now_ms)),
                    if passed_over.is_some() {
                        RunState::Skipped.as_str()
                    } else {
                        RunState::Running.as_str()
                    },
                    passed_over
                ],
            )?;
            let next = schedule.when.next_after(now_ms, schedule.created_ms);
            transaction.execute(
                "UPDATE schedules SET next_due_ms = ?2, enabled = ?3 WHERE schedule_id = ?1",
                params![
                    schedule.schedule_id,
                    next.map(signed),
                    i64::from(next.is_some())
                ],
            )?;
            // Claimed by somebody else between the look and the lock: theirs.
            if claimed == 1 && passed_over.is_none() {
                to_say.push(Due {
                    late: now_ms.saturating_sub(due_ms) > LATE_AFTER_MS,
                    due_ms,
                    schedule,
                });
            }
        }
        transaction.commit()?;
        Ok(to_say)
    }

    /// A run was said: where, and as which delivery.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be written.
    pub fn run_said(
        &mut self,
        schedule_id: &str,
        due_ms: u64,
        chat_id: &str,
        delivery_id: &str,
    ) -> Result<(), RoutingError> {
        self.connection.execute(
            "UPDATE schedule_runs SET chat_id = ?3, delivery_id = ?4
             WHERE schedule_id = ?1 AND due_ms = ?2",
            params![schedule_id, signed(due_ms), chat_id, delivery_id],
        )?;
        Ok(())
    }

    /// A run ended.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be written.
    pub fn run_ended(
        &mut self,
        schedule_id: &str,
        due_ms: u64,
        state: RunState,
        note: Option<&str>,
    ) -> Result<(), RoutingError> {
        self.connection.execute(
            "UPDATE schedule_runs SET state = ?3, note = ?4, ended_ms = ?5
             WHERE schedule_id = ?1 AND due_ms = ?2 AND state = 'running'",
            params![schedule_id, signed(due_ms), state.as_str(), note, now_ms()],
        )?;
        Ok(())
    }

    /// The runs nobody wrote the end of: a keeper that stopped in the middle
    /// left them, and the one that starts reads them to follow or end them.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn runs_going(&mut self) -> Result<Vec<TimedRun>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM schedule_runs WHERE state = 'running' ORDER BY due_ms"
        ))?;
        let runs = statement
            .query_map([], run_of)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(runs)
    }

    /// The last runs of an agent's schedules, or of every schedule, the
    /// newest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn runs(
        &mut self,
        agent_id: Option<&str>,
        at_most: usize,
    ) -> Result<Vec<TimedRun>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM schedule_runs
             WHERE ?1 IS NULL OR agent_id = ?1 ORDER BY due_ms DESC, schedule_id LIMIT ?2"
        ))?;
        let runs = statement
            .query_map(
                params![agent_id, i64::try_from(at_most).unwrap_or(i64::MAX)],
                run_of,
            )?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(runs)
    }
}

/// A question an agent asked in a turn a schedule began, with nobody there
/// to answer it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeftUnanswered {
    pub question_id: String,
    pub kind: String,
    /// What was asked, as the ledger keeps it.
    pub asked: serde_json::Value,
    /// The schedule that began the turn, as a participant.
    pub speaker_id: String,
}

impl RoutingLedger {
    /// The questions that wait in turns schedules began and were asked at
    /// or before `asked_by_ms`.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn questions_left_to_schedules(
        &mut self,
        asked_by_ms: u64,
    ) -> Result<Vec<LeftUnanswered>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT q.question_id, q.kind, q.asked_json, m.sender_id
             FROM questions q
             JOIN deliveries d ON d.delivery_id = q.delivery_id
             JOIN messages m ON m.message_id = d.message_id
             JOIN participants p ON p.participant_id = m.sender_id
             WHERE q.state = 'waiting' AND p.kind = 'schedule' AND q.asked_ms <= ?1
             ORDER BY q.asked_ms",
        )?;
        let found = statement
            .query_map([signed(asked_by_ms)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        found
            .into_iter()
            .map(|(question_id, kind, asked, speaker_id)| {
                Ok(LeftUnanswered {
                    question_id,
                    kind,
                    asked: serde_json::from_str(&asked)?,
                    speaker_id,
                })
            })
            .collect()
    }
}
