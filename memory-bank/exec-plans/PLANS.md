# ExecPlan protocol

ExecPlans are living implementation documents for substantial work. They exist so an agent can
resume a difficult task from repository state without prior chat context.

Use an ExecPlan for multi-file features, migrations, architectural changes, hard debugging, or work
likely to span a long autonomous session. Small local changes can use a lightweight plan in the
agent's normal task context.

## Core rule

An ExecPlan starts with a **user-observable outcome**, not an internal mechanism.

Good:

> From a clean data directory, a person installs an agent from the plan the Workbench shows,
> makes it theirs and gets an answer from it, with nothing composed by hand.

Bad:

> Make declaration storage mutable.

Mutable declaration storage may be a step inside the good plan if the actual product path requires
it.

## Required sections

Every active ExecPlan contains:

1. **Outcome** — what becomes possible from a real entry point.
2. **Acceptance scenario** — concrete given/when/then behavior.
3. **Current state** — facts from the current code/runtime, including interrupted work.
4. **Relevant product journey** — links to golden journey(s).
5. **Legacy evidence** — old behavior worth preserving, if relevant.
6. **Constraints** — current ADR/architecture boundaries that matter.
7. **Plan** — a small sequence of vertical implementation slices.
8. **Progress** — checked/unchecked work, updated during execution.
9. **Discoveries** — facts learned that change understanding.
10. **Decision log** — task-local decisions with reasons; promote durable ones to ADRs.
11. **Validation** — exact tests/manual/browser/live checks run and their results.
12. **Outcome/remaining gaps** — what is actually proven at completion.

## Plan maintenance

Update the plan as implementation changes. Do not preserve an invalidated plan for narrative
consistency.

If evidence changes architecture assumptions, record the discovery, update the plan, and add an ADR
only if the decision is durable beyond this task.

Do not expand the task to fix every adjacent issue. Record unrelated debt in the roadmap's debt section.

## Completion

A plan may move to `completed/` only when its promised observable outcome is exercised through the
intended product path, or when it is explicitly abandoned/superseded with a reason.

Green unit tests for internal pieces are not enough if the plan promises product behavior.

At completion:

- promote durable truths to the appropriate memory-bank files;
- update the roadmap and capability statuses from evidence;
- move the plan to `completed/`;
- avoid leaving completed task narrative in root instructions.
