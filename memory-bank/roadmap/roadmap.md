# Roadmap

Each item ends in something a person can do through a real entry point. Order is by dependency.
C1 and C4 are done. The Workbench items (A, then B) come next: with the project space gone the
harness and its Workbench are a product of their own, and as one they are judged by a person
using them on their own data (`product/agents.md`). S goes first among what is left of them:
it is what the harness needs before another product is built on it. C3 follows.

## A — Agents and chats

The model is `product/agents.md`. Each item is one ExecPlan.

| Item | A person can | State |
|---|---|---|
| A1 | open an existing chat and go on talking after the agent's setup changed; an attached server that is not set up is a notice, not a refusal | **done** (2026-09-27, ExecPlan 004) |
| A2 | see who said what; keep two agents working at once; stop from the composer; answer a question after reopening the page | **done** (2026-09-27, ExecPlan 005) |
| A3 | read markdown and code; use a rail of agents and chats; switch theme | **done** (2026-09-27, ExecPlan 005, with A2) |
| A4 | give a key once in Providers; edit an agent in forms by section; see what this machine offers | **done** (2026-09-28, ExecPlan 006); the record of hosts is left to B1 |
| A5 | schedule a message into a chat, pause it, and let the agent make its own | **done** (2026-09-28, ExecPlan 007); time kept while the Workbench is closed, by the system's scheduler (ExecPlan 009); a call from outside comes with S3 |
| A6 | put two agents in one chat; a chain of agent replies waits for a person at its limit | **done** (2026-09-28, ExecPlan 008) |

## B — Hosts

| Item | A person can | State |
|---|---|---|
| B1 | run a real engine in a container on this machine, with its key, terminal and files inside | **part one done** (2026-09-28, ExecPlan 010): the runner and an agent's files as a tree with an editor. The container waits for Podman to be set up on this computer |
| B2 | limit where a sealed agent connects and allow a refused address from the chat | |
| B3 | make a new agent in four steps from providers that were looked at | **done for an agent on this machine** (2026-09-28, ExecPlan 011); the look inside a container and its network wait for B1 and B2 |
| B4 | add a machine over SSH, install an engine there, chat, open a terminal, read files | |
| B5 | give a remote agent a server that lives with the Workbench | |
| B6 | put an agent on a Sprite that sleeps and is woken by a message or a schedule | |
| B7 | use Docker where Podman is absent | |

After B: a messenger channel linked to a chat.

## S — Served beyond one computer, built into another product

ADR-0010 and ADR-0011. Each item is one ExecPlan.

| Item | A person can | State |
|---|---|---|
| S1 | put the Workbench on a server of their own, come to it from a laptop and a phone with a passkey, and nobody else can | **active** (ExecPlan 015) |
| S4 | build the harness into a product of their own in a page of code, with their own people, name and servers | |
| S3 | read of each keeper of time when it works; give a scheduler outside an address to knock on; find that an agent with no keeper has no Schedules and no tools for them | after S1 |
| S2 | have an agent on a machine that is not this one | B1, B4, B5, B6 |
| S5 | install a kind of machine or of keeper of time from the Store | after S2 and S3 |

## C1 — Embed the harness

`cargo run -p swem-host --example embed` prints a Workbench URL; a person installs an agent from
the registry and talks to it. The assembly recipe moves from the product binary into the crate as
a builder; the Podman resolver moves with it; no process-global state remains.
Related journeys: GJ-01, GJ-07. **Done** (2026-09-26, ExecPlan 001).

## C2 — API clients

Taken up by S1: a program is given a token that says what it may do (ADR-0011). ADR-0005 is
superseded and was not built.
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
