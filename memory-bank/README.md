# SWEM memory bank

This directory is the versioned repository memory for the people and coding agents who change
SWEM. It is split by **kind of truth** so that one document cannot become product specification,
architecture, roadmap, history and task tracker at the same time.

## Authority model

| Question | Source |
|---|---|
| What must the product let a person do? | `product/` |
| How is the current implementation structured? | root `ARCHITECTURE.md` + `architecture/` |
| Why did we make a durable architectural choice? | `decisions/` |
| What capability comes next? | `roadmap/roadmap.md` |
| What is being implemented right now? | `exec-plans/active/` |
| What repeatedly went wrong and must not recur? | `memory/lessons.md` |

Code, tests and runtime behaviour remain the strongest evidence of what actually exists.

## Progressive disclosure

Do not preload the whole memory bank. Start with:

1. root `AGENTS.md`;
2. the relevant journey in `product/journeys.md`;
3. the current roadmap item;
4. the active ExecPlan;
5. only then the architecture and decision material the task needs.

## Update discipline

Do not update every file after every commit.

- Product behaviour changed -> update the relevant product document.
- Current architecture changed -> update the architecture map or a detail document.
- A durable choice changed -> add or supersede an ADR.
- A capability completed, blocked or reordered -> update the roadmap.
- A long task progressed -> update the active ExecPlan.
- A repeated failure taught a general lesson -> add it to `memory/lessons.md`.

Do not copy task narratives into permanent memory: completed ExecPlans and the git history keep
that history.

## Status vocabulary

- `proven` — exercised through the intended real path, with repository evidence;
- `partial` — a meaningful substrate or sub-path exists, but the promised capability is incomplete;
- `missing` — no usable implementation;
- `blocked` — cannot proceed because of an explicit dependency;
- `retired` — deliberately removed, with a current decision saying why;
- `unknown` — the evidence is insufficient; verify before claiming.

Never upgrade a status on prose alone.
