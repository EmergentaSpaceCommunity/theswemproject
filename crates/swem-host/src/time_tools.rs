//! An agent's own schedules, as tools of its session.
//!
//! An agent that is asked to remind or to check back makes a schedule
//! itself, through a server the host hands to its session. Who the agent is
//! and which chat it was asked in are given when the server is started and
//! no call can change them: an agent makes schedules for itself, in the chat
//! it is in, and pauses and removes only the ones it made. The schedules
//! are kept in the ledger, where whoever keeps time finds them; nothing
//! about time lives in the agent's machine.

use std::path::PathBuf;

use chrono::DateTime;
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{Json, ServerHandler, ServiceExt as _, schemars, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

use crate::{NewTimedMessage, RoutingLedger, TimedMessage, TimedMessageChange, When};

/// The name the server is handed to a session under.
pub const TIME_TOOLS: &str = "swem-time";

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Make {
    /// What is said to you when it is due, in the words you want to read then.
    say: String,
    /// Said every so many minutes, five at least.
    #[serde(default)]
    every_minutes: Option<u32>,
    /// Said by a cron line of five fields: minute, hour, day of the month,
    /// month, day of the week. For example `0 9 * * 1-5`.
    #[serde(default)]
    cron: Option<String>,
    /// The time zone of the cron line, by its name, such as `Europe/Kyiv`.
    /// The zone of the computer when left out.
    #[serde(default)]
    zone: Option<String>,
    /// Said once, at this moment, as RFC 3339: `2026-09-28T18:30:00+03:00`.
    #[serde(default)]
    at: Option<String>,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Pause {
    schedule_id: String,
    /// True to pause it, false to turn it on again.
    paused: bool,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Remove {
    schedule_id: String,
}

#[derive(Clone, Debug, Deserialize, schemars::JsonSchema)]
struct Nothing {}

/// A schedule as the agent is told of it.
#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct Told {
    schedule_id: String,
    say: String,
    /// When it is said, in words.
    when: String,
    /// When it is next said, as RFC 3339 in the time of this computer;
    /// nothing when it is paused or will not be said again.
    next: Option<String>,
    paused: bool,
    /// Whether you made it. You pause and remove only what you made.
    made_by_you: bool,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct Listed {
    schedules: Vec<Told>,
}

#[derive(Clone, Debug, Serialize, schemars::JsonSchema)]
struct Removed {
    removed: String,
}

#[derive(Clone, Debug)]
struct TimeTools {
    ledger: PathBuf,
    agent_id: String,
    chat_id: String,
    tool_router: ToolRouter<Self>,
}

impl TimeTools {
    fn told(&self, schedule: TimedMessage) -> Told {
        Told {
            next: schedule
                .next_due_ms
                .and_then(|next| i64::try_from(next).ok())
                .and_then(DateTime::from_timestamp_millis)
                .map(|next| {
                    // In the time of the person who asked, so the agent
                    // says an hour they recognise.
                    let zone = When::zone_here()
                        .parse::<chrono_tz::Tz>()
                        .unwrap_or(chrono_tz::UTC);
                    next.with_timezone(&zone)
                        .to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
                }),
            paused: !schedule.enabled,
            made_by_you: schedule.made_by == self.agent_id,
            when: schedule.when_in_words,
            say: schedule.say,
            schedule_id: schedule.schedule_id,
        }
    }

    fn ledger(&self) -> Result<RoutingLedger, String> {
        RoutingLedger::open(&self.ledger).map_err(|error| error.to_string())
    }

    /// One of the agent's own schedules, or why it is not the agent's.
    fn its_own(&self, ledger: &mut RoutingLedger, schedule_id: &str) -> Result<(), String> {
        let schedule = ledger
            .schedule(schedule_id)
            .map_err(|error| error.to_string())?;
        if schedule.agent_id != self.agent_id {
            return Err(format!("there is no schedule {schedule_id}"));
        }
        if schedule.made_by != self.agent_id {
            return Err(
                "that schedule was made by a person; it is theirs to pause or remove".to_owned(),
            );
        }
        Ok(())
    }
}

#[tool_router]
impl TimeTools {
    #[tool(
        description = "Have something said to you on time, in this chat: a reminder, a check to come back to. Give what should be said and exactly one of every_minutes, cron or at."
    )]
    fn make_schedule(&self, Parameters(make): Parameters<Make>) -> Result<Json<Told>, String> {
        let when = match (make.every_minutes, make.cron, make.at) {
            (Some(minutes), None, None) => When::Every { minutes },
            (None, Some(line), None) => When::Cron {
                line,
                zone: make.zone.unwrap_or_else(When::zone_here),
            },
            (None, None, Some(at)) => When::Once {
                at_ms: DateTime::parse_from_rfc3339(&at)
                    .ok()
                    .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
                    .ok_or_else(|| format!("{at:?} is not a moment as RFC 3339 writes it"))?,
            },
            _ => return Err("give exactly one of every_minutes, cron and at".to_owned()),
        };
        let kept = self
            .ledger()?
            .keep_schedule_of_an_agent(&NewTimedMessage {
                agent_id: self.agent_id.clone(),
                made_by: self.agent_id.clone(),
                chat_id: Some(self.chat_id.clone()),
                say: make.say,
                when,
            })
            .map_err(|error| error.to_string())?;
        Ok(Json(self.told(kept)))
    }

    #[tool(
        description = "What is said to you on time: every schedule of yours, with when it is next said."
    )]
    fn list_schedules(
        &self,
        Parameters(Nothing {}): Parameters<Nothing>,
    ) -> Result<Json<Listed>, String> {
        let schedules = self
            .ledger()?
            .schedules(Some(&self.agent_id))
            .map_err(|error| error.to_string())?;
        Ok(Json(Listed {
            schedules: schedules
                .into_iter()
                .map(|schedule| self.told(schedule))
                .collect(),
        }))
    }

    #[tool(description = "Pause a schedule you made, or turn it on again.")]
    fn pause_schedule(
        &self,
        Parameters(Pause {
            schedule_id,
            paused,
        }): Parameters<Pause>,
    ) -> Result<Json<Told>, String> {
        let mut ledger = self.ledger()?;
        self.its_own(&mut ledger, &schedule_id)?;
        let kept = ledger
            .change_schedule(
                &schedule_id,
                &TimedMessageChange {
                    enabled: Some(!paused),
                    ..Default::default()
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(Json(self.told(kept)))
    }

    #[tool(description = "Remove a schedule you made. What it said stays said.")]
    fn remove_schedule(
        &self,
        Parameters(Remove { schedule_id }): Parameters<Remove>,
    ) -> Result<Json<Removed>, String> {
        let mut ledger = self.ledger()?;
        self.its_own(&mut ledger, &schedule_id)?;
        ledger
            .forget_schedule(&schedule_id)
            .map_err(|error| error.to_string())?;
        Ok(Json(Removed {
            removed: schedule_id,
        }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for TimeTools {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(TIME_TOOLS, env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Schedules: messages said to you on time, in the chat you are in. Make one when \
                 you are asked to remind of something or to check back later. Time is kept for \
                 you; you need not stay awake for it.",
            )
    }
}

/// Serve an agent's schedules over this process's standard streams, until
/// the engine that started it lets go.
///
/// # Errors
///
/// Says in words why the ledger cannot be opened or the streams failed.
pub async fn serve_time_tools(
    ledger: PathBuf,
    agent_id: String,
    chat_id: String,
) -> Result<(), String> {
    // Refused at the door rather than at the first call.
    RoutingLedger::open(&ledger)
        .and_then(|ledger| ledger.participant(&agent_id))
        .map_err(|error| error.to_string())?;
    TimeTools {
        ledger,
        agent_id,
        chat_id,
        tool_router: TimeTools::tool_router(),
    }
    .serve(rmcp::transport::stdio())
    .await
    .map_err(|error| error.to_string())?
    .waiting()
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}
