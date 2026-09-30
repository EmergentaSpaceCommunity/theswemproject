# ExecPlan 018 — One Store, many hosts

**Status:** completed on the core's side (2026-09-30). Roadmap P (C3 folded in). ADR-0004,
ADR-0012.

## Outcome

A person installs, updates and removes packages of any kind from one Store: what the harness
takes (agents, servers, skills), what a server the harness runs takes for itself (the Cycle's
packages, through the hub's own installer), and what a product the harness is built into takes
for itself (a kind of its own, checked against the shape of tools it calls). A package that
requires another is installed with it under one consent. A kind nobody here takes is said so.
Whoever builds the harness into a product reads one document that says all of it.

## Acceptance scenario

**Given** a Workbench with a catalog added by address. **When** a person installs a skill that
requires another, **then** the consent says what is also installed and both land; **when** the
catalog is published again with a newer version, **then** "Update to" is offered and taken;
**when** they remove one, **then** it is gone from the page and from the install root; a row of
a kind nobody here takes cannot be installed and says why.

**Given** the example product built on the harness, which takes helpers. **When** a person
installs a helper whose tools are not what the product calls, **then** it is refused in the
product's words and nothing lands; **when** they install one that answers the shape, **then**
their agent is given it; **when** they remove it, **then** it is given no more.

**Given** a declared server whose entry says it takes a kind. **When** a package of that kind
is installed, **then** the server plans and installs it on its own road and the Store keeps the
receipt; removed, the server removes it.

## Current state (2026-09-30, before)

`InstallKind` a closed enum of four in the harness; an unknown kind failed the catalog; no
update, no removal; versions sorted as text; a server reinstalled lost its env; three plan-id
recipes; `rusqlite 0.40` refused beside `sqlx 0.8`.

## Constraints

- ADR-0002: nothing in the Store names a product. The Cycle's hub is reached through `takes`
  in its catalog entry, never by name in code.
- ADR-0004: indexes consumed, never hosted; consent to a plan by id.
- ADR-0011 of the Cycle: its packages are validated, digested and retired by its own
  installer. The Store calls it; it does not copy it.
- The Cycle's repository is not touched without the owner's word.

## Plan

- P0 link beside `sqlx 0.8`.
- P1 `swem-store`: kinds by name, takers, `requires`, `uvx`, semver, remove, one plan recipe.
- P2 the page: kinds by host, the closure in one consent, update, remove, a kind nobody takes.
- P3 a server that takes: `Through`, `Delegated`, the fixture, `takes` for the Cycle in the
  shipped catalog. The Cycle's half waits for the owner.
- P4 the shape checker; the example product takes a kind of its own.
- P5 `docs/building-in.md`.

## Progress

- [x] P0: `rusqlite 0.32`; `scripts/links-beside-sqlx.sh` builds a program that depends on
      both; `rmcp 3.5` with `process-wrap 10`.
- [x] P1: `crates/swem-store` (no dependency on `swem-host`); the harness keeps a thin wrapper
      with three takers.
- [x] P2: the Store walk of the gate covers the closure, the unknown kind, the update and the
      removal.
- [x] P3, the core's half: `Through`, `Delegated`, `ServersHere` (the harness's MCP client);
      `swem-mcp-taker-fixture`; `default-catalog.json` at `@0.2` with `takes` for
      `swem.cycle/package@1`.
- [ ] P3, the Cycle's half (the owner's go): `check_assembly` against the installed set at
      plan time; a published `catalog.json` with `hello-node`; its walk on the new pin. Already
      there in the hub: `plan_package` by directory, staging by plan id, `remove_package`,
      children retired after install.
- [x] P4: `swem_store::shape`; `tools_listed_by` / `tools_listed_by_blocking`; the example
      takes `example/helper@1`, refuses a helper whose tools are not its shape, gives what it
      took to every agent, lets it go on removal; the gate walks it.
- [x] P5: `docs/building-in.md`; README; ARCHITECTURE; ADR-0012.

## Discoveries

- A server that plans a staged tree and installs from where it planned finds nothing after the
  Store's rename. The Store asks the server to plan twice: of the staged tree, so a package the
  server refuses is never committed, and of the tree where it lives, which is what is
  installed. A plan id made of the tree's contents answers the same both times; the fixture
  keeps the last plan under an id.
- The page said "SWEM" in the Store where it meant the product; it now says what the product
  is called, as the rail does.
- A taker's check that starts a program runs on a blocking thread of the runtime the product
  serves on; `tools_listed_by_blocking` does the `block_in_place` there, so a product's taker
  does not.

## Decision log

- 2026-09-30: kinds are open strings; the four old words stay on the wire, other kinds are
  reverse-DNS with a version and land under a directory made of them.
- 2026-09-30: a server takes a kind by saying so in its catalog entry; the argument shapes
  default to the Cycle's hub's tools and can be templated.
- 2026-09-30: the shape checker compares names, the properties the host sends with their
  types, and what the server requires against what the host always sends; nothing else.

## Validation

- `cargo test -p swem-store`: kinds, takers, the closure, semver, a check that refuses, a
  server that takes through a fake host, shapes.
- `cargo test -p swem-host --test store`: the harness's takers; a declared fixture server that
  takes packages through the Store (multi-thread runtime).
- The gate: the Store walk (closure, unknown kind, update, removal), the built-in walk (the
  product's own kind, refused and taken and given, removed).
- `scripts/links-beside-sqlx.sh`.

## Outcome and remaining gaps

Done as above. Left: the Cycle's half of P3; the Store page in sections by host (it filters
by kind, with the host's words); Apps in a harness built in for several people; `uvx` pinned
by a lockfile; the page's palette for a product that builds the harness in.
