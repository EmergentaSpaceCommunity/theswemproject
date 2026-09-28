# ExecPlan 007 — A message that arrives on time

**Status:** active (2026-09-28). Roadmap A5; `product/agents.md` (Time).

## Outcome

A person opens an agent's **Schedules**. They make a schedule: what the agent is told, when -
every so many minutes, every day at a time, every week on a day, once, or a cron line - and
the chat it is said in. It is said on time, by the schedule, in that chat, and answered like
anything else said there. They pause it with a switch and turn it on again. They read the
last runs: when each was due, whether it ran late and why, how it ended. They ask the agent in
a chat to remind it of something, and the agent makes the schedule itself; it is in the list,
made by the agent. Providers says who keeps time.

## Acceptance scenario

**Given** the owner's data root with an agent and a standing instruction set in the old panel,
**when** the person opens the Workbench built from this plan and goes to the agent's
Schedules, **then** the old instruction is there as a schedule, made by them, saying what it
said. **When** they make a schedule of every two minutes into the agent's chat and wait,
**then** the chat shows the schedule's message under the schedule's name and the agent's
answer, and Last runs says it was answered. **When** they switch it off and wait two minutes,
**then** nothing is said. **When** they stop the product for five minutes with a schedule due
in between and start it, **then** it is said once and the run says it ran late. **When** they
ask the agent to check back every ten minutes, **then** a schedule made by the agent is in the
list, and they can switch it off.

## Current state (2026-09-28)

- A schedule is a JSON file under `<data>/schedules`: a name, a profile, words, an interval in
  minutes. A loop in the serving process looks every ten seconds, claims by rewriting the file
  and says the words into a chat as a participant of kind schedule (since ExecPlan 005).
- No pause on the page, no time of day, no run kept but the last outcome as a sentence that
  holds a number of milliseconds. A missed window runs once when the product is back and does
  not say it was late.
- A turn said by a schedule that asks before running a command waits for a person for as
  long as nobody answers; its deadline is paused while it waits (ExecPlan 005).
- An agent cannot make a schedule.
- The schedules are set in Settings, "On its own"; the gate's walk
  `a_person_leaves_a_standing_instruction_and_the_product_carries_it_out` presses that form.

## Relevant product journey

A new journey, written when this plan ends. GJ-09 for what a chat does with what is said.

## Legacy evidence

The reference kept the clock on the platform, not in the agent's machine; an agent made
schedules through a command that called the platform; cron and one-off times; limits of five
minutes at least and twenty per agent (`product/agents.md`, Time). The harness's own rule -
a window that was missed runs once and not once per window - is kept.

## Constraints

- Time is never kept inside an agent's machine.
- The ledger's schema version does not move: the tables of time are added beside what is
  there, so a product built before this plan opens the same data root.
- One keeper at a time: the serving process that holds the keeper's lock. A run is claimed in
  the ledger by the schedule and the moment it was due, so nothing is said twice whoever
  looks.
- An agent manages its own schedules only, in the chat it was asked in.
- Work in this repository only.

## Plan

1. **Schedules in the ledger** (`time.rs`): a schedule (what is said, when, to which agent,
   into which chat, made by whom, on or off, when it is next due) and its runs. When: every N
   minutes, a cron line in a named zone (`croner`, `chrono-tz`), once at a moment. The files
   of the old clock become schedules, and the directory is set aside.
2. **The keeper** (`workbench_shell/timekeeper.rs`): holds the lock, looks, claims what is due,
   says it, follows the delivery to its end and writes how the run ended. Late within a day
   is said late; later than that is skipped and said so. A schedule whose last run has not
   ended is skipped. The old loop and `ScheduleBook` leave.
3. **A question nobody answers**: a turn said by a schedule that asks waits thirty minutes,
   then is answered with the engine's own refusal.
4. **The agent's own schedules** (`swem time`, an MCP server over stdio on `rmcp`): make, list,
   pause, remove; the agent and the chat are given when it is started and cannot be changed by
   a call. Handed to every session on this machine.
5. **The page**: an agent's Schedules tab (who keeps time, the list with its switch, a new
   schedule, last runs); Providers, Time. "On its own" leaves Settings. The gate's walk moves.

## Progress

- [x] 1 schedules and runs in the ledger (`time.rs`): every so often, a cron line in a zone, once; a run
      claimed by its schedule and the moment it was due; late, too late, and the run before not ended
- [x] 2 the keeper (`workbench_shell/timekeeper.rs`): the lock beside the ledger, the look every five
      seconds, what the old clock kept brought in and its directory set aside as `schedules.v1`;
      `GET|POST /api/schedules`, `PATCH|DELETE /api/schedules/{id}`, `GET /api/time`; the old clock,
      its book of files and its addresses are gone
- [x] 3 a question nobody answers: after half an hour the schedule answers with the engine's own
      refusal, and the turn ends
- [x] 4 the agent's own schedules (`time_tools.rs`, `swem mcp time-tools`, `swem-time-tools`): make, list,
      pause, remove over stdio; handed to every session opened for a chat on this machine; no more
      often than five minutes and no more than twenty when an agent makes them
- [x] 5 the page: an agent's Schedules (who keeps time, the list with its switch, New schedule, Last
      runs), Providers Time; "On its own" left Settings; the gate's walk makes its schedule there

## Discoveries

- A schedule made every so often is first due one interval after it is made, not at once as the
  old clock's was. The gate's walk makes one of every minute and waits for it.
- A schedule spoke in a chat under the beginning of its own words, so every message of it was
  said twice, once as a name. It is called by when it speaks ("Every day at 06:00"), and is not
  listed among who is in the chat.
- The real engine said when its schedule is next due in universal time, because that is what it
  was told. It is told in the time of this computer.
- An engine names a tool of a server by how it addresses it (`mcp__swem-time__make_schedule`), and
  the page showed that. The page says what the tool does and whose it is.
- The time of day on the page is said the way the person's system says it, with or without AM.

## Decision log

- 2026-09-28: the system's own scheduler and an outside one are not drawn in Providers: nothing
  stands behind them, and a row that cannot be turned on is a promise. Time shows who keeps it
  now. The seam they would stand on is one function that says what is due.
- 2026-09-28: limits, as the reference had them: a schedule is no more often than every five
  minutes when an agent makes it, and an agent has at most twenty. A person may set a minute.

## Validation

- 2026-09-28, steps 1-3 and 5: `time` (3) and `chat_runtime` (10, two of them the keeper's) green: an
  old schedule is brought in and said on time under the schedule's name with its maker's trust;
  off, nothing is due; a question with nobody there is not answered at twenty-nine minutes and
  is refused after thirty, by the schedule, and what nobody allowed was not done; the run after
  one that has not ended is passed over. Lint clean for the workspace. The page's tests 29 green.
  The gate's walk of a schedule through the real binary in Chrome: green.

- 2026-09-28, step 4: `chat_runtime::an_agent_makes_its_own_schedule_in_the_chat_it_was_asked_in`
  green: made through the tool in the chat it was asked in; one of every minute refused with the
  reason; a person's schedule listed and not the agent's to remove; its own paused. The gate's
  walk makes the agent's own schedule through the real binary and the person switches it off.
- 2026-09-28, the acceptance scenario by hand in Chrome on a copy of the owner's data root, the
  real binary and the real engine: an instruction of the old clock was there as a schedule of
  every two hours, made by the person; a schedule of every minute was made in the form, said by
  the schedule and answered (five times, each kept as a run); the product was stopped five
  seconds before a run and started three and a half minutes after it, and the run was said once
  and kept as said late; asked to check back every ten minutes, the engine asked before it used
  its tool, was allowed, and made a schedule that stands in the list as made by the agent.

- 2026-09-28, the end of the slice, the computer kept awake for the run: lint clean; the product
  gate fourteen green in one run; `scripts/suites.sh` green but `time`, where two tests that
  began in the same instant were given one ledger by a fixture named for the instant alone.
  The fixture is named for its test; `time` is green five times of five.

## Outcome / remaining gaps

Done 2026-09-28. A person makes a schedule in an agent's Schedules, it is said on time by the
schedule and answered with nobody watching, they pause and forget it and read its runs; the
agent makes its own when asked; Providers says who keeps time.

Remaining:

- One keeper, the running SWEM: nothing is said while it is closed. The system's own scheduler
  and an outside one are not built; what they would stand on is `claim_due`.
- A schedule in a chat of several names its agent by its handle; choosing who is named is A6.
- A schedule is changed by pausing, forgetting and making another: the page has no form to
  change what one says or when, though the host takes the change.
- An agent in a container is handed no schedule tools until roadmap B5.
- The owner's own data root was not opened by this build; the scenario was walked on a copy.
