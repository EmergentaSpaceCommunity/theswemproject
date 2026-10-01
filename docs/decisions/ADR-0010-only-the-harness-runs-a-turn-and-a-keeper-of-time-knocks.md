# ADR-0010 — Only the harness runs a turn, and a keeper of time knocks

**Status:** accepted (2026-09-29, roadmap S). Stands beside ADR-0007 and ADR-0008.

## Context

An agent's machine may sleep when nothing is asked of it and cannot wake itself, which is why
who keeps time is a provider apart from the machine (`product/agents.md`, Time). A person's
Workbench may be on a computer that is closed when something is due. The question was who runs
the turn then. Two ways were weighed: the runner inside the machine runs it and the chat
catches up afterwards, or the harness is where it is on.

What a turn run inside the machine would bring with it:

- a second implementation of a turn: a client of the agent protocol inside the runner, which
  ADR-0008 keeps small and free of the harness, or each engine's own command for one turn with
  nobody attached - they differ, and the transcripts they leave are internal formats;
- keys kept inside the machine, because nobody is there to hand them over at the start;
- the schedules and what they say copied into the machine or to a third party;
- a second "one turn at a time", on the machine's disk, which the harness would have to honour;
- a door into the machine from outside. A provider's token commonly opens everything of an
  account, so it cannot be given to a scheduler; what is left is a program of ours behind a
  public address, kept alive inside the machine;
- the servers that live with the harness - the agent's own schedule tools among them - absent
  from such a turn, and the rule for a question nobody answers kept in a second place;
- two writers of one chat that cannot see each other, and a merge afterwards.

What it would give is work done at the hour it was due and not when the computer is opened.
What was answered is read when the computer is opened either way, because chats and channels
live with the harness. And it would answer only schedules: a message from a messenger or from
another agent that arrives while the harness is off is not delivered either.

So the question is not about time. An agent is its profile, chats, keys, schedules and
channels, and those live with a harness. The machine is where it works.

## Decision

- **Only the harness runs a turn.** There is one implementation of a turn. The runner is the
  way into a machine and is asked for files, a program started, a look; it is never asked to
  deliver.
- **A keeper of time does one thing: it tells a harness to look at what is due.** It holds when
  to knock. It never holds what is said, a key, or a machine's token. The ledger's claim, keyed
  by the schedule and the time it was due, stays the only truth: a keeper that knocks twice, or
  when nothing is due, does no harm.
- **Who can keep time depends on where the harness is**, not on the agent's machine:

  | Where the harness is | Who can keep its time | What is due while it is off |
  |---|---|---|
  | a computer that sleeps | the running Workbench; that computer's own scheduler | said once when it is back, up to a day late, marked late |
  | a server that is always on | the Workbench itself | nothing is missed |
  | a place that sleeps and is woken by a call | a call from outside | it is woken for it |

- **What the harness needs of a machine** is that it can wake it and enter it. That is looked
  at when the machine is added and before a run.
- **An agent is handed the tools for schedules** where a keeper is set up for it and the tools
  can be carried to its machine. Elsewhere it has no Schedules and the page says why.
- **Working while a person's computer is closed is chosen by where the agent is kept**: the
  Workbench on a server, reached with sign-in (ADR-0011); or, where the only machines a person
  has are ones that sleep, the Workbench itself put into such a machine as its service and
  knocked on from outside. The same program and the same ledger.

## Consequences

"Time is never kept inside an agent's machine" stands as written. The third keeper, a call
from outside, can be added where the Workbench is served at an address.

A person whose Workbench is on a laptop and whose agent is on a machine elsewhere gets what was
due when the laptop is opened, marked late. That is said on the page where the keeper is
chosen, before it is relied on.

This is not proposed again unless somebody has an agent on a machine that sleeps, no place for
a harness that is on or can be woken, and late is not enough. Even then what goes into the
machine is the harness, never a second executor.
