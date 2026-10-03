# ADR-0014 — Where an agent lives and what travels

**Status:** accepted (2026-10-02).

## Context

A person has a laptop, a desktop, maybe a server, maybe machines in a cloud, and agents on
several of them. Keys open providers; a passkey opens a served Workbench; a bot's token opens
a messenger. With channels (ADR-0013) a bot joins the things an agent is reached through, and
the question of what belongs where, and what may be copied, had to be answered once rather than
per feature. The earlier decisions already fixed two corners: only the harness runs a turn, and
a keeper of time only knocks (ADR-0010); who may come in is a passkey or a token the
served harness itself made (ADR-0011).

## Decision

1. **An agent lives in one harness, with one ledger.** Its profile, chats, schedules, questions
   and channels are that harness's. Nothing is synchronised between harnesses. Moving an agent
   is an explicit act that carries its profile, chats and schedules, and nothing else.
2. **Keys never travel.** A moved agent's providers are set up again on the harness it moved
   to, or are kept by the product the harness is built into. No key is copied by SWEM, written
   into a profile, a ledger, a log or a command line, or sent over a channel.
3. **A person is known to each served harness by a passkey registered there.** The page may
   stand over several harnesses with a session on each. There is no account of SWEM's anywhere,
   and no server of SWEM's between a person and their harness.
4. **A bot belongs to one harness:** the one that runs its channel and holds its token. A
   messenger allows one consumer of a bot at a time, and so does SWEM. Moving a bot is removing
   it on one Workbench and adding it with its token on another, by hand.

## Consequences

- "It works while my laptop is closed" is chosen by where the agent is kept - on a server, or in
  a place that is woken by a call - never by copying the agent somewhere else as well.
- A channel's token is one more key: kept under the harness's keys, handed to the channel's
  program in its environment, absent from the channel's document and from every message.
- An outside identity (a messenger account) binds to a participant of one ledger; the same
  person on two harnesses is two bindings, made with two codes.
- What a product that builds the harness in gets for free: one harness per person, with that
  product's own sign-in in front, needs nothing shared between harnesses.

## Links

- ADR-0010, ADR-0011, ADR-0013; `docs/channels.md`; `docs/serving.md`.
