# ExecPlan 014 — A schedule is changed, and consent is asked by the page

**Status:** completed (2026-09-28). `product/agents.md` (Time). What ExecPlans 007 and 011 left.

## Outcome

A person changes a schedule they or the agent made: what is said and when. They no longer
forget it and make it again to correct a word. Whatever is installed from the Store is
consented to in a dialog of the page that says what is fetched and what described it, as an
engine already is; the browser's own dialog is gone from the product.

## Acceptance scenario

**Given** an agent with a schedule of every day at nine, **when** the person presses Change,
the form holds what the schedule says and when; **when** they make it every week on Monday at
ten and save, **then** the list says so, what is due next is a Monday at ten, and the chat
names the schedule by its new time. **When** they install a server from the Store, **then**
the page asks in its own dialog, and "Not now" installs nothing.

## Current state (2026-09-28)

- The host changes a schedule's words and time (`PATCH /api/schedules/{id}`); the page only
  turns one on and off and forgets it.
- The Store asks with `window.confirm`. The gate's walks of the Store read the browser's
  dialog.

## Relevant product journey

GJ-11, GJ-03.

## Legacy evidence

None here.

## Constraints

- What the form shows for a schedule is what the schedule is: a time of day reads as a time
  of day, not as a cron line, when it is one.
- The chat a schedule speaks in is not changed by changing the schedule.

## Plan

1. The page: what a schedule's time is, read back into what the form chooses from; the form
   for a new one and for a change; Change in the list.
2. The Store's consent as the page's dialog; the walks by what the dialog says.
3. By hand; suites; the gate.

## Progress

- [x] 1 a schedule is changed
- [x] 2 consent by the page
- [x] 3 by hand, suites, gate

## Discoveries

## Decision log

## Validation

- `test/page-time.test.mjs`: when a schedule speaks is read back into what the form chooses
  from - every so often in the unit it is whole in, a time of day, a day of the week, and a
  cron line where it is neither - and what is read back is what it was.
- The gate: what is installed from the Store, and the engine of the first run, is consented
  to in the page's dialog, which the walks read and answer
  (`a_person_installs_from_the_store_and_the_agent_uses_it`,
  `a_person_installs_a_server_with_a_home_app_and_opens_its_space`,
  `a_person_with_nothing_installs_an_agent_and_can_start_it`).
- By hand in Chrome on a copy of the owner's data root: a schedule of every two hours opened
  with Change held its words and "every 2 hours"; made every week on Monday at ten, with
  other words, and saved, the list and the host said so.

## Outcome / remaining gaps

Done 2026-09-28. Nothing on the page opens a dialog of the browser's.

Remaining:

- The chat a schedule speaks in is not changed; it is forgotten and made again for that.
- A walk does not refuse a consent: "Not now" was pressed by hand only.
