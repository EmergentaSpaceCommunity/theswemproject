# ExecPlan 009 — Time kept while the Workbench is closed

**Status:** completed (2026-09-28). `product/agents.md` (Time, Providers). Follows ExecPlan 007.

## Outcome

A person opens **Providers, Time** and reads who can keep time: this SWEM, the system's own
scheduler, an outside scheduler. They turn the system's scheduler on. They choose, for an
agent, who keeps its schedules. They close the Workbench. A schedule of that agent comes due;
the machine starts SWEM, the message is said in its chat and answered, and SWEM leaves. They
open the Workbench and read the message, the answer and the run. They turn the scheduler off
and nothing of it is left on the machine.

## Why time is a provider

An agent's machine sleeps between messages and cannot wake itself; a clock inside it would
keep it awake or die with it. So time is kept outside the machine, and who keeps it differs
by what it can do: the running SWEM keeps it only while it runs; the system's scheduler starts
SWEM when the Workbench is closed and needs the computer to be on; an outside scheduler
reaches a SWEM that is deployed where it can be reached, and is what wakes an agent on a
machine that sleeps elsewhere. ExecPlan 007 built the first and showed it as one line with a
time zone, which said nothing of this.

## Acceptance scenario

**Given** a data root with an agent and a schedule of every five minutes into its chat,
**when** the person turns the system's scheduler on under Providers, Time, makes it the
agent's keeper and closes the Workbench, **then** within a minute of the schedule being due
the chat holds the schedule's message and the agent's answer, and no SWEM is left running.
**When** they open the Workbench, **then** Last runs says it was answered and Time says when
the scheduler looked last. **When** the Workbench is open, **then** it keeps time itself and
the scheduler, finding that, leaves at once. **When** they turn it off, **then** the
system holds no job of it and the agent's schedules wait for SWEM to run.

## Current state (2026-09-28)

- `keep_time` takes the keeper's lock once when the door opens and never again; a Workbench
  that found it taken keeps no time for as long as it runs.
- `claim_what_is_due` and `say_what_is_due` are apart, and nothing but the serving process
  calls them. No command says what is due and leaves.
- Providers, Time shows one keeper. An agent has no keeper of its own.

## Relevant product journey

GJ-11 (a message that arrives on time); a part is added to it when this ends.

## Legacy evidence

The reference kept the clock on the platform for the cost of a sleeping machine
(`product/agents.md`, Time). Nothing there ran on a person's own computer.

## Constraints

- Time is never kept inside an agent's machine.
- The ledger's schema version does not move.
- One keeper at a time over a ledger, by the keeper's lock; a run is claimed once whoever
  looks.
- The system's job holds no key and no secret. It holds the command, where the data is and
  the search path the Workbench was started with, because the engines are found on it.
- What cannot be done by hand here is said as not checked: systemd on Linux. Windows is not
  built.

## Plan

1. The shell: a Workbench that did not get the keeper's lock asks again as time passes.
   `claim_due` takes the agents it is asked for. `Assembled::keep_time_once`: the lock or
   leave, what nobody answered, what is due for the agents of this keeper, each said and
   followed to its end, a note of the look under `<data>/time`.
2. Who keeps an agent's time and the keeper agents have, kept under
   `<data>/time/keepers.json` (see the decision log: not on the profile).
3. The system's scheduler (`host/system_scheduler.rs`): what is written for launchd and for
   a systemd user timer, put in place and taken away, and how it stands, read from the
   system each time.
4. The command `swem time keep`, given to the product like the time tools' command.
5. Routes and the page: `GET /api/time` with the three keepers; turn on, turn off, make
   default; the agent's Settings, Time.
6. By hand on this Mac, on a copy of the owner's data root; the gate; documents.

## Progress

- [x] 1 the shell keeps time once and leaves
- [x] 2 an agent's keeper
- [x] 3 the system's scheduler
- [x] 4 the command
- [x] 5 routes and the page
- [x] 6 by hand, the gate, documents

## Discoveries

- A look must not cost a product put together: most looks find nothing. `swem time keep`
  asks the ledger which agents have something due and whether any of them is this keeper's,
  and leaves; the product is assembled only when there is something to say.
- A look that found nothing would forget the one that said something; the note keeps when it
  last said something.
- launchd counts a job's runs and its last exit; `launchctl print` is how the page knows the
  job is held, so a job removed by hand is off on the page.
- With the scheduler as the keeper agents have, "This SWEM" shows no agents while it says
  "Keeping time": an open Workbench says what is due to everybody, whoever their keeper is.

## Decision log

- 2026-09-28: who keeps an agent's time is kept beside the keepers, not on the profile. A
  profile's revision is what sessions are compared against; a choice about time is not a
  change of the agent's setup and must not say so in its chats.
- 2026-09-28: turning the scheduler off gives its agents back to the running SWEM: the
  default returns and what was chosen for an agent is forgotten. Nobody is left with a
  keeper that keeps no time.

- 2026-09-28: the job looks every minute and leaves when nothing is due, instead of being
  rewritten for the next moment something is due. A schedule is not to the second, a look
  that finds nothing opens the ledger and nothing else, and a job that is rewritten on every
  change is a second record of schedules that can be wrong.
- 2026-09-28: an outside scheduler is shown and cannot be added. It needs a SWEM that can be
  reached and a credential for who calls it (appendix C2 of the plan of 2026-09-27); it is
  built with the first host that sleeps elsewhere (roadmap B6).

## Validation

- `keepers` (2), `host::system_scheduler` (3, what launchd and systemd are given),
  `chat_runtime::time_is_kept_once_for_the_agents_of_a_keeper`: one agent's time is the
  system's, the other's is not; what is due is said to the first only and answered; the look
  is written down; where somebody keeps time another finds nothing to do; the page is told
  who keeps whose time.
- By hand on macOS 12, a copy of the owner's data root, Claude Code as the engine. Turned on
  from Providers, Time with "for every agent" left checked: launchd held the job, its first
  run found the Workbench keeping time and left. A schedule of every minute turned on and
  the Workbench stopped: the next run of the job said it 54 seconds after it was due, the
  agent answered "pong" 45 seconds later, no SWEM was left running, the log holds one line.
  The Workbench opened again kept time itself; Last runs says Answered; Time says when the
  scheduler looked last. Turned off from the page: no file under LaunchAgents, launchd knows
  no such job, the default is this SWEM again.
- A Workbench started while another process held the keeper's lock said "Another SWEM here
  keeps it now" and kept time within seconds of the lock being let go.
- `scripts/suites.sh`, the product gate: see the outcome.

## Outcome / remaining gaps

Done 2026-09-28. Providers, Time says who can keep time and what each cannot do; the
system's own scheduler is turned on and off there; an agent's keeper is chosen in its
Schedules; time is kept while the Workbench is closed.

Remaining:

- systemd: what it is given is written and read back in a test; it was not done on a
  machine. Windows has no keeper but the running SWEM.
- An outside scheduler is shown and cannot be added (roadmap B6).
- The job starts the SWEM that turned it on. When that file moves the page says so and the
  person turns it on again.
- The job is given the search path and where the data is, nothing else: an engine that is
  signed in by a variable of a person's shell is not signed in under the scheduler. A key
  given in Providers is, because the product hands it over.
- A look is every minute: while the Workbench is closed a schedule is said up to a minute
  after it is due.
- Last runs does not say who kept time for a run.
- A question an agent asks in a turn the scheduler began waits half an hour and is refused.
  Answering it from a Workbench opened meanwhile, while the turn runs in the other process,
  was not tried.
