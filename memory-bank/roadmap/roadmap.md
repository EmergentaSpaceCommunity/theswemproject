# Roadmap

Each item ends in something a person can do through a real entry point. Order is by dependency.
C1 and C4 are done. The Workbench items (A, then B) come next: with the project space gone the
harness and its Workbench are a product of their own, and as one they are judged by a person
using them on their own data (`product/agents.md`). C2 and C3 follow them.

## A — Agents and chats

The model is `product/agents.md`. Each item is one ExecPlan.

| Item | A person can | State |
|---|---|---|
| A1 | open an existing chat and go on talking after the agent's setup changed; an attached server that is not set up is a notice, not a refusal | **done** (2026-09-27, ExecPlan 004) |
| A2 | see who said what; keep two agents working at once; stop from the composer; answer a question after reopening the page | **done** (2026-09-27, ExecPlan 005) |
| A3 | read markdown and code; use a rail of agents and chats; switch theme | **done** (2026-09-27, ExecPlan 005, with A2) |
| A4 | give a key once in Providers; edit an agent in forms by section; see what this machine offers | **done** (2026-09-28, ExecPlan 006); the record of hosts is left to B1 |
| A5 | schedule a message into a chat, pause it, and let the agent make its own | **done** (2026-09-28, ExecPlan 007); time kept while the Workbench is closed, by the system's scheduler (ExecPlan 009); an outside scheduler comes with B6 |
| A6 | put two agents in one chat; a chain of agent replies waits for a person at its limit | **done** (2026-09-28, ExecPlan 008) |

## B — Hosts

| Item | A person can | State |
|---|---|---|
| B1 | run a real engine in a container on this machine, with its key, terminal and files inside | |
| B2 | limit where a sealed agent connects and allow a refused address from the chat | |
| B3 | make a new agent in four steps from providers that were looked at | |
| B4 | add a machine over SSH, install an engine there, chat, open a terminal, read files | |
| B5 | give a remote agent a server that lives with the Workbench | |
| B6 | put an agent on a Sprite that sleeps and is woken by a message or a schedule | |
| B7 | use Docker where Podman is absent | |

After B: a messenger channel linked to a chat.

## C1 — Embed the harness

`cargo run -p swem-host --example embed` prints a Workbench URL; a person installs an agent from
the registry and talks to it. The assembly recipe moves from the product binary into the crate as
a builder; the Podman resolver moves with it; no process-global state remains.
Related journeys: GJ-01, GJ-07. **Done** (2026-09-26, ExecPlan 001).

## C2 — API clients

`swem workbench serve --client-origin <origin>` prints a bearer; a page on that origin opens a
session and gets a reply; the page's own token is refused there. ADR-0005.
Related journeys: GJ-07.

## C3 — Store dependencies and `server.json`

An entry that requires another installs both; an unmet requirement is refused in words; a
`server.json` document added as an index lists its server.
Related journeys: GJ-03.

## C4 — A server's home App is a space

A server that declares a home App appears as a space; opening it shows the App; installing
`swem-cycle` from the Store opens its Project space. The host's own Project space is removed only
after that App renders everything it did. ADR-0006.
Related journeys: GJ-08. **C4a done** (2026-09-26, ExecPlan 002): the mechanism, proved with the
Apps fixture. **C4b done** in the Cycle repository (2026-09-27): the Project space is the hub's
home App, and its gate walks every journey through that space. **C4c done** (2026-09-27,
ExecPlan 003): the host's own Project space, its routes, `ProductSupply`, the project factory
and the package and tool routes are gone; an App says what a person is looking at through
`ui/update-model-context` and the agent is given it.

## Debt

- The install plan's fields are named for an agent whatever kind it installs (`agent_id`,
  `registry_id`); rename with C3.
- A server's environment values are given in the MCP catalogue, not from the Store row.
- `scripts/unwalked_controls.py` counts controls by the names the old page gave them and
  checks itself against five of those; it refuses to count the new page, whose controls a walk
  finds by what they say. Rewritten or retired when somebody needs the number again.
- No browser walk repeats GJ-09 or an App's context by itself; both are proven below the page
  and by hand.
- The Cycle's walks drive the rail and the addresses the old page had; they are moved when its
  pin moves to a core that has the new page.
