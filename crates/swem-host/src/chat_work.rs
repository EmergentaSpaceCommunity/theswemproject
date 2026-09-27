//! What a chat owes and what it waits for: the part of the ledger that
//! outlives a page and a connection.
//!
//! A message into a chat is owed to the agents it is for. Each debt is a
//! delivery: queued when the message is said, running while the agent has
//! the turn, ended with how it ended. An agent that asks something in its
//! turn leaves a question: it waits in the ledger, not in the memory of a
//! connection, so a person who closed the page finds it when they come back.
//!
//! Every change is an event of the chat, so a page that follows the chat
//! with one cursor sees the work move without asking about it.

use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::chat_ledger::{new_id, now_ms};
use crate::{RoutingError, RoutingLedger, SurfaceEventSource};

/// Where a delivery is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// Owed; the agent has not begun.
    Queued,
    /// The agent has the turn.
    Running,
    /// The agent's turn ended by itself.
    Done,
    /// A person stopped it.
    Stopped,
    /// It could not be given, or the turn failed.
    Failed,
    /// The Workbench stopped while it was running.
    Interrupted,
}

impl DeliveryState {
    fn word(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Interrupted => "interrupted",
        }
    }

    fn of(word: &str) -> Result<Self, RoutingError> {
        match word {
            "queued" => Ok(Self::Queued),
            "running" => Ok(Self::Running),
            "done" => Ok(Self::Done),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            "interrupted" => Ok(Self::Interrupted),
            other => Err(RoutingError::InvalidBinding(format!(
                "unknown state of a delivery: {other}"
            ))),
        }
    }

    /// Whether nothing more will happen to it.
    #[must_use]
    pub fn ended(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

/// A message owed to an agent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Delivery {
    pub delivery_id: String,
    pub chat_id: String,
    pub agent_id: String,
    pub message_id: String,
    pub state: DeliveryState,
    /// How it ended, in words, when there is something to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    pub created_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_ms: Option<u64>,
}

/// Where a question is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionState {
    Waiting,
    Answered,
    /// Nobody can answer it any more: the turn that asked it is over.
    Lapsed,
}

impl QuestionState {
    fn word(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Answered => "answered",
            Self::Lapsed => "lapsed",
        }
    }

    fn of(word: &str) -> Result<Self, RoutingError> {
        match word {
            "waiting" => Ok(Self::Waiting),
            "answered" => Ok(Self::Answered),
            "lapsed" => Ok(Self::Lapsed),
            other => Err(RoutingError::InvalidBinding(format!(
                "unknown state of a question: {other}"
            ))),
        }
    }
}

/// Something an agent asked in its turn and waits for.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Question {
    pub question_id: String,
    pub chat_id: String,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
    /// What kind of answer it takes: `permission`, `form` or `link`.
    pub kind: String,
    /// What was asked, as the one who answers reads it.
    pub asked: Value,
    pub asked_ms: u64,
    pub state: QuestionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_ms: Option<u64>,
}

fn ms(value: Option<i64>) -> Option<u64> {
    value.and_then(|value| u64::try_from(value).ok())
}

/// An event of a chat that belongs to no route: the host's own record of
/// what the chat owes and waits for.
fn chat_event_in(
    transaction: &Transaction<'_>,
    chat_id: &str,
    kind: &str,
    payload: &Value,
) -> Result<i64, RoutingError> {
    transaction.execute(
        "INSERT INTO events(route_id, chat_id, event_id, kind, source, payload_json, at_ms)
         VALUES (NULL, ?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            chat_id,
            new_id("e")?,
            kind,
            serde_json::to_string(&SurfaceEventSource::Host)?,
            serde_json::to_string(payload)?,
            now_ms()
        ],
    )?;
    Ok(transaction.last_insert_rowid())
}

const DELIVERY_COLUMNS: &str = "delivery_id, chat_id, agent_id, message_id, state, outcome, \
                                created_ms, started_ms, ended_ms";

type DeliveryRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    Option<i64>,
    Option<i64>,
);

fn delivery_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

fn delivery_of(row: DeliveryRow) -> Result<Delivery, RoutingError> {
    let (delivery_id, chat_id, agent_id, message_id, state, outcome, created, started, ended) = row;
    Ok(Delivery {
        delivery_id,
        chat_id,
        agent_id,
        message_id,
        state: DeliveryState::of(&state)?,
        outcome,
        created_ms: u64::try_from(created).unwrap_or(0),
        started_ms: ms(started),
        ended_ms: ms(ended),
    })
}

fn delivery_in(transaction: &Transaction<'_>, delivery_id: &str) -> Result<Delivery, RoutingError> {
    transaction
        .query_row(
            &format!("SELECT {DELIVERY_COLUMNS} FROM deliveries WHERE delivery_id = ?1"),
            [delivery_id],
            delivery_row,
        )
        .optional()?
        .map(delivery_of)
        .transpose()?
        .ok_or_else(|| RoutingError::InvalidBinding(format!("there is no delivery {delivery_id}")))
}

fn delivery_moved(transaction: &Transaction<'_>, delivery: &Delivery) -> Result<(), RoutingError> {
    chat_event_in(
        transaction,
        &delivery.chat_id,
        "chat/delivery",
        &json!({
            "delivery_id": delivery.delivery_id,
            "agent_id": delivery.agent_id,
            "message_id": delivery.message_id,
            "state": delivery.state,
            "outcome": delivery.outcome,
        }),
    )?;
    Ok(())
}

const QUESTION_COLUMNS: &str = "question_id, chat_id, agent_id, delivery_id, kind, asked_json, \
                                asked_ms, state, answer_json, answered_by, answered_ms";

type QuestionRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
);

fn question_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QuestionRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
    ))
}

fn question_of(row: QuestionRow) -> Result<Question, RoutingError> {
    let (
        question_id,
        chat_id,
        agent_id,
        delivery_id,
        kind,
        asked,
        asked_ms,
        state,
        answer,
        answered_by,
        answered_ms,
    ) = row;
    Ok(Question {
        question_id,
        chat_id,
        agent_id,
        delivery_id,
        kind,
        asked: serde_json::from_str(&asked)?,
        asked_ms: u64::try_from(asked_ms).unwrap_or(0),
        state: QuestionState::of(&state)?,
        answer: answer
            .map(|answer| serde_json::from_str(&answer))
            .transpose()?,
        answered_by,
        answered_ms: ms(answered_ms),
    })
}

fn question_in(transaction: &Transaction<'_>, question_id: &str) -> Result<Question, RoutingError> {
    transaction
        .query_row(
            &format!("SELECT {QUESTION_COLUMNS} FROM questions WHERE question_id = ?1"),
            [question_id],
            question_row,
        )
        .optional()?
        .map(question_of)
        .transpose()?
        .ok_or_else(|| RoutingError::InvalidBinding(format!("there is no question {question_id}")))
}

fn question_moved(transaction: &Transaction<'_>, question: &Question) -> Result<(), RoutingError> {
    chat_event_in(
        transaction,
        &question.chat_id,
        "chat/question",
        &json!({
            "question_id": question.question_id,
            "agent_id": question.agent_id,
            "delivery_id": question.delivery_id,
            "kind": question.kind,
            "state": question.state,
            "asked": question.asked,
            "answer": question.answer,
            "answered_by": question.answered_by,
        }),
    )?;
    Ok(())
}

/// Every question a delivery left waiting lapses with it.
fn lapse_with(
    transaction: &Transaction<'_>,
    condition: &str,
    value: &str,
) -> Result<(), RoutingError> {
    let waiting: Vec<String> = {
        let mut statement = transaction.prepare(&format!(
            "SELECT question_id FROM questions WHERE state = 'waiting' AND {condition} = ?1"
        ))?;
        let rows = statement.query_map([value], |row| row.get(0))?;
        rows.collect::<Result<_, _>>()?
    };
    for question_id in waiting {
        transaction.execute(
            "UPDATE questions SET state = 'lapsed' WHERE question_id = ?1",
            [&question_id],
        )?;
        question_moved(transaction, &question_in(transaction, &question_id)?)?;
    }
    Ok(())
}

impl RoutingLedger {
    /// Owe a message to agents of its chat. Owed twice to the same agent, it
    /// is the same debt.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a message that does not exist and for an
    /// agent that is not in the message's chat.
    pub fn deliver(
        &mut self,
        message_id: &str,
        agents: &[String],
    ) -> Result<Vec<Delivery>, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let chat_id: String = transaction
            .query_row(
                "SELECT chat_id FROM messages WHERE message_id = ?1",
                [message_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                RoutingError::InvalidBinding(format!("there is no message {message_id}"))
            })?;
        let mut owed = Vec::new();
        for agent_id in agents {
            let member: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM chat_members m
                               JOIN participants p ON p.participant_id = m.participant_id
                               WHERE m.chat_id = ?1 AND m.participant_id = ?2
                                 AND m.left_ms IS NULL AND p.kind = 'agent')",
                params![chat_id, agent_id],
                |row| row.get(0),
            )?;
            if !member {
                return Err(RoutingError::InvalidBinding(format!(
                    "{agent_id} is not an agent of chat {chat_id}"
                )));
            }
            let known: Option<String> = transaction
                .query_row(
                    "SELECT delivery_id FROM deliveries WHERE agent_id = ?1 AND message_id = ?2",
                    params![agent_id, message_id],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(delivery_id) = known {
                owed.push(delivery_in(&transaction, &delivery_id)?);
                continue;
            }
            let delivery_id = new_id("d")?;
            transaction.execute(
                "INSERT INTO deliveries(delivery_id, chat_id, agent_id, message_id, state, created_ms)
                 VALUES (?1, ?2, ?3, ?4, 'queued', ?5)",
                params![delivery_id, chat_id, agent_id, message_id, now_ms()],
            )?;
            let delivery = delivery_in(&transaction, &delivery_id)?;
            delivery_moved(&transaction, &delivery)?;
            owed.push(delivery);
        }
        transaction.commit()?;
        Ok(owed)
    }

    /// One delivery by id.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for one that does not exist.
    pub fn delivery(&mut self, delivery_id: &str) -> Result<Delivery, RoutingError> {
        let transaction = self.connection.transaction()?;
        let delivery = delivery_in(&transaction, delivery_id)?;
        transaction.commit()?;
        Ok(delivery)
    }

    /// The oldest message an agent is owed and has not begun, and takes it:
    /// the delivery is running when this returns.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn take_next_delivery(&mut self, agent_id: &str) -> Result<Option<Delivery>, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let next: Option<String> = transaction
            .query_row(
                "SELECT d.delivery_id FROM deliveries d
                 JOIN messages m ON m.message_id = d.message_id
                 WHERE d.agent_id = ?1 AND d.state = 'queued'
                 ORDER BY m.sequence LIMIT 1",
                [agent_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(delivery_id) = next else {
            transaction.commit()?;
            return Ok(None);
        };
        transaction.execute(
            "UPDATE deliveries SET state = 'running', started_ms = ?2 WHERE delivery_id = ?1",
            params![delivery_id, now_ms()],
        )?;
        let delivery = delivery_in(&transaction, &delivery_id)?;
        delivery_moved(&transaction, &delivery)?;
        transaction.commit()?;
        Ok(Some(delivery))
    }

    /// End a delivery. What it left waiting lapses. Ended already, it stays
    /// as it ended.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a delivery that does not exist and for a
    /// state that is not an end.
    pub fn end_delivery(
        &mut self,
        delivery_id: &str,
        state: DeliveryState,
        outcome: Option<&str>,
    ) -> Result<Delivery, RoutingError> {
        if !state.ended() {
            return Err(RoutingError::InvalidBinding(format!(
                "{} is not how a delivery ends",
                state.word()
            )));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let delivery = delivery_in(&transaction, delivery_id)?;
        if delivery.state.ended() {
            transaction.commit()?;
            return Ok(delivery);
        }
        transaction.execute(
            "UPDATE deliveries SET state = ?2, outcome = ?3, ended_ms = ?4 WHERE delivery_id = ?1",
            params![delivery_id, state.word(), outcome, now_ms()],
        )?;
        lapse_with(&transaction, "delivery_id", delivery_id)?;
        let delivery = delivery_in(&transaction, delivery_id)?;
        delivery_moved(&transaction, &delivery)?;
        transaction.commit()?;
        Ok(delivery)
    }

    /// What a chat still owes, or what every chat does: queued and running,
    /// oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn deliveries_open(&self, chat_id: Option<&str>) -> Result<Vec<Delivery>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {DELIVERY_COLUMNS} FROM deliveries
             WHERE state IN ('queued', 'running') AND (?1 IS NULL OR chat_id = ?1)
             ORDER BY created_ms, delivery_id"
        ))?;
        let rows = statement.query_map([chat_id], delivery_row)?;
        rows.map(|row| delivery_of(row?)).collect()
    }

    /// The agents that are owed something and have not begun it.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn agents_owed(&self) -> Result<Vec<String>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT agent_id FROM deliveries WHERE state = 'queued' ORDER BY agent_id",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What waited in a chat, for an agent or for every agent, is not begun.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be written.
    pub fn stop_queued(
        &mut self,
        chat_id: &str,
        agent_id: Option<&str>,
    ) -> Result<Vec<Delivery>, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let queued: Vec<String> = {
            let mut statement = transaction.prepare(
                "SELECT delivery_id FROM deliveries
                 WHERE state = 'queued' AND chat_id = ?1 AND (?2 IS NULL OR agent_id = ?2)
                 ORDER BY created_ms, delivery_id",
            )?;
            let rows = statement.query_map(params![chat_id, agent_id], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        let mut stopped = Vec::new();
        for delivery_id in queued {
            transaction.execute(
                "UPDATE deliveries SET state = 'stopped', ended_ms = ?2 WHERE delivery_id = ?1",
                params![delivery_id, now_ms()],
            )?;
            let delivery = delivery_in(&transaction, &delivery_id)?;
            delivery_moved(&transaction, &delivery)?;
            stopped.push(delivery);
        }
        transaction.commit()?;
        Ok(stopped)
    }

    /// Count one more answer of an agent that is for other agents, and say
    /// whether the chat still allows it. At the limit the answer is held:
    /// it is given to nobody until a person writes.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for a chat that does not exist.
    pub fn count_agent_reply(
        &mut self,
        chat_id: &str,
        message_id: &str,
    ) -> Result<bool, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (limit, replies): (i64, i64) = transaction
            .query_row(
                "SELECT reply_limit, agent_replies FROM chats WHERE chat_id = ?1",
                [chat_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| RoutingError::InvalidBinding(format!("there is no chat {chat_id}")))?;
        let allowed = replies < limit;
        if allowed {
            transaction.execute(
                "UPDATE chats SET agent_replies = agent_replies + 1 WHERE chat_id = ?1",
                [chat_id],
            )?;
        } else {
            chat_event_in(
                &transaction,
                chat_id,
                "chat/held",
                &json!({
                    "message_id": message_id,
                    "in_words": format!(
                        "agents have answered each other {limit} times; the chat waits for a person"
                    ),
                }),
            )?;
        }
        transaction.commit()?;
        Ok(allowed)
    }

    /// The agents that have a turn running, as the ledger has it.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn agents_running(&self) -> Result<Vec<String>, RoutingError> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT agent_id FROM deliveries WHERE state = 'running' ORDER BY agent_id",
        )?;
        let rows = statement.query_map([], |row| row.get(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// What a Workbench that stopped left behind, for the agents whose turn
    /// nobody holds any more: a delivery that was running was interrupted,
    /// and what it had asked can no longer be answered. What was queued
    /// stays owed. Returns how many were running.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read or written.
    pub fn settle_what_was_running(&mut self, agents: &[String]) -> Result<usize, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mut running: Vec<String> = Vec::new();
        for agent_id in agents {
            let mut statement = transaction.prepare(
                "SELECT delivery_id FROM deliveries WHERE state = 'running' AND agent_id = ?1",
            )?;
            let rows = statement.query_map([agent_id], |row| row.get(0))?;
            for row in rows {
                running.push(row?);
            }
        }
        for delivery_id in &running {
            transaction.execute(
                "UPDATE deliveries SET state = 'interrupted', outcome = ?2, ended_ms = ?3
                 WHERE delivery_id = ?1",
                params![
                    delivery_id,
                    "the Workbench stopped while this was being answered",
                    now_ms()
                ],
            )?;
            lapse_with(&transaction, "delivery_id", delivery_id)?;
            delivery_moved(&transaction, &delivery_in(&transaction, delivery_id)?)?;
        }
        transaction.commit()?;
        Ok(running.len())
    }

    /// Keep a question an agent asked.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] for an unknown chat, agent or delivery.
    pub fn ask(
        &mut self,
        chat_id: &str,
        agent_id: &str,
        delivery_id: Option<&str>,
        kind: &str,
        asked: &Value,
    ) -> Result<Question, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let question_id = new_id("q")?;
        transaction.execute(
            "INSERT INTO questions(question_id, chat_id, agent_id, delivery_id, kind, asked_json,
                                   asked_ms, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'waiting')",
            params![
                question_id,
                chat_id,
                agent_id,
                delivery_id,
                kind,
                serde_json::to_string(asked)?,
                now_ms()
            ],
        )?;
        let question = question_in(&transaction, &question_id)?;
        question_moved(&transaction, &question)?;
        transaction.commit()?;
        Ok(question)
    }

    /// One question by id.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for one that does not exist.
    pub fn question(&mut self, question_id: &str) -> Result<Question, RoutingError> {
        let transaction = self.connection.transaction()?;
        let question = question_in(&transaction, question_id)?;
        transaction.commit()?;
        Ok(question)
    }

    /// Keep the answer to a question that waits.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError::InvalidBinding`] for a question that does not
    /// exist and for one that no longer waits.
    pub fn answer_question(
        &mut self,
        question_id: &str,
        answer: &Value,
        answered_by: &str,
    ) -> Result<Question, RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let question = question_in(&transaction, question_id)?;
        if question.state != QuestionState::Waiting {
            return Err(RoutingError::InvalidBinding(format!(
                "question {question_id} is {} and cannot be answered",
                question.state.word()
            )));
        }
        transaction.execute(
            "UPDATE questions SET state = 'answered', answer_json = ?2, answered_by = ?3,
                                  answered_ms = ?4
             WHERE question_id = ?1",
            params![
                question_id,
                serde_json::to_string(answer)?,
                answered_by,
                now_ms()
            ],
        )?;
        let question = question_in(&transaction, question_id)?;
        question_moved(&transaction, &question)?;
        transaction.commit()?;
        Ok(question)
    }

    /// A question nobody can answer any more. One that does not wait is left
    /// as it is.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be written.
    pub fn lapse_question(&mut self, question_id: &str) -> Result<(), RoutingError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        lapse_with(&transaction, "question_id", question_id)?;
        transaction.commit()?;
        Ok(())
    }

    /// The questions that wait, in a chat or in every chat, oldest first.
    ///
    /// # Errors
    ///
    /// Returns [`RoutingError`] when the ledger cannot be read.
    pub fn questions_waiting(&self, chat_id: Option<&str>) -> Result<Vec<Question>, RoutingError> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {QUESTION_COLUMNS} FROM questions
             WHERE state = 'waiting' AND (?1 IS NULL OR chat_id = ?1)
             ORDER BY asked_ms, question_id"
        ))?;
        let rows = statement.query_map([chat_id], question_row)?;
        rows.map(|row| question_of(row?)).collect()
    }
}
