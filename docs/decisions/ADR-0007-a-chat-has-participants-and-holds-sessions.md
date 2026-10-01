# ADR-0007 — A chat has participants and holds sessions

**Status:** accepted (2026-09-27, roadmap A2, ExecPlan 005).

## Context

The ledger knew one thing: a route, which is one agent's one native session, and the events on
it. Who wrote a turn was a line of text appended to the turn. A conversation therefore had
exactly one agent, died with its native session, and could not say who said what. ACP has one
user role and no author; the engines keep their own session stores, which the host does not
read.

## Decision

The ledger gains, in the same database, what the product calls a chat.

- **A participant** is a person, an agent, a guest or a schedule: one id of one kind
  (`p_` and twenty-six characters of time and randomness), a handle, a name. An agent's
  participant carries its profile id. The owner is the first person.
- **A chat** has members and messages. **A message** names its sender and the channel it came
  through. A message is positioned by an event in the one sequence every event already has, so
  one cursor follows everything.
- **A session** joins a chat, an agent and a route: the engine's own memory of that chat. A chat
  holds one current session per agent and any number of earlier ones. The route row keeps what
  the setup was when the session began.
- **A delivery** is a message waiting for, running in, or finished by an agent's turn. An agent
  takes one delivery at a time; the claim is a transaction in the ledger and a file lock held for
  the turn, because the editor door is another process.
- **A question** an agent asks is kept in the ledger until it is answered, so it outlives the
  page that showed it.

Every route that existed becomes a chat of the owner and that route's agent, with its messages
read out of the events that were recorded. The database is copied before it is changed.

What an engine is told about who spoke is one structured block per turn, made by the host from
these records. A principal's own words stay the first block, as they were typed.

## Consequences

A chat survives its agent's setup, its engine's session, and the page. Several agents can be in
one chat, and each message has one sender the product knows. `Correspondent` and the provenance
line leave the code. The events of a route stay what they were: a record of what the engine
said, never input to it.

An older binary refuses the newer database by its version, as it already does.
