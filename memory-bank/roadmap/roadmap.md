# Roadmap

Each item ends in something a person can do through a real entry point, proved by a named walk.
Order is by dependency: C1 first; C4's second half only after the project server ships its own
space (its repository's item Y4).

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
home App, and its gate walks every journey through that space. **C4c** is next: the host's own
Project space, ProductSupply and project routes go. Owed with it: `ui/update-model-context`, so
an App can say what a person is looking at (the agent-context binding the panel had).

## Debt

- The install plan's fields are named for an agent whatever kind it installs (`agent_id`,
  `registry_id`); rename with C3.
- A server's environment values are given in the MCP catalogue, not from the Store row.
- Two callback-boundary tests assume a Linux filesystem and signal names.
