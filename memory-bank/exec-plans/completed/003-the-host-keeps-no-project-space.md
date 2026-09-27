# ExecPlan 003 — The host keeps no Project space of its own

**Status:** completed (2026-09-27). Roadmap C4 (C4c); ADR-0002 (the host names no domain),
ADR-0006 (a server's home App is a space).

## Outcome

The Workbench shows Agent, Store and one space per server that declares a home App - nothing
else. What a person does with projects they do in the space of the server that serves them; the
host renders it as it renders any App. An App can say what a person is looking at
(`ui/update-model-context`), and the agent is given it with the next turn: the binding the
host's own panel had, now any App's to make.

## Acceptance scenario

**Given** the Apps fixture declared with `--home`, and an agent session that attaches it,
**When** a person opens the fixture's space, presses its "give this to the agent" control, goes
to the Agent space and sends a turn,
**Then** the Agent space shows what the agent is given and from which server, the turn carries
it, and "Let go of it" takes it off (`workbench_shell_context_browser`). The page offers no
Project space; `/api/projects` answers not found; `genericity.rs` refuses `swem://project` and
the envelope's media type in the host's sources.

## Current state (2026-09-27)

The host still mounts its own Project space (`web/apps-host/src/project/*`, hidden under a
server's space), serves its routes (`/api/projects…`, `/api/packages…`, `/api/tools…`,
`/api/recipes`), keeps `ProjectSources`, `ProjectFactory`, `ProductSupply`, `packages.rs` and
`supplies.rs` for it, and binds an agent's context to a project revision by reading the
project's envelope (`workbench_project.rs`). The project server ships the same space as its own
App and its gate walks every journey through it. Generic code reads project state in four
places: `every_declaration`, the `"project"` connection id of space Apps, the upload root of a
space, and the page's `DomainApp`/`fetchJson` under `src/project/`.

## Constraints

- Nothing generic is lost: session Apps, spaces, the catalogue, the Store, agents, schedules,
  terminals, the editor door.
- The context an App gives is the specification's: content blocks, the last update wins, sent
  with the next turn. The host interprets none of it; it refuses context from a server the
  session does not attach, as it refused a project the session did not attach.
- A server the product declares is still listed and attachable; it is not "a project".

## Plan

1. Host: the context binding becomes generic (`ModelContext { server_name, content }`), bound
   and cleared through the same routes; `workbench_project.rs` goes.
2. Host: project routes, `ProjectFactory`, `ProjectSources`, `ProductSupply`, `packages.rs`,
   `supplies.rs`, `recipes.rs`, `project_tools.rs` go; `project_apps.rs` becomes
   `server_apps.rs` with what spaces use; the builder loses `project_server`,
   `product_supply`, `package_directories` and the boot scan.
3. Page: `src/project/` goes; `DomainApp` and `fetchJson` move to generic homes; the bridge
   answers `ui/update-model-context`; the Agent space shows what the agent is given.
4. The product: `swem` declares the project hub as a server when it is installed, instead of
   handing it to the builder as a project server.
5. Tests, the fixture's context control, `genericity.rs`, docs.

## Progress

- [x] 1 context: `workbench_shell/model_context.rs`, limits of 16 blocks and 16 KiB of text,
      a server the connection does not attach is refused
- [x] 2 host removal
- [x] 3 page
- [x] 4 product
- [x] 5 tests and docs

## Discoveries

- With the project space gone, the Agent space is all a person meets, and it does not stand as
  a product: see `product/agents.md`, which this plan's outcome led to.
- A profile that attaches a server which is no longer declared cannot open any session, new or
  old. The owner's own profile is in that state. ExecPlan 004 removes the gate.

## Decision log

- 2026-09-27: the agent-context binding is not deleted with the panel; it becomes the
  specification's `ui/update-model-context`, so the project server's App can offer "work with
  this project" again and any other App can offer its own.

## Validation

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `scripts/suites.sh`: every suite passes except `callback_authority` (two tests that assume a
  Linux filesystem and signal names; roadmap debt, red on macOS before this plan).
- `workbench_shell_context_browser` passed in Chrome: the fixture's App gave its context, the
  Agent space showed it, the turn carried it, "Let go of it" took it off.
- The product started on a real data root shows Agent, Store and the project server's space.

## Outcome / remaining gaps

Done. The host renders no project vocabulary and serves no project route.

Remaining, and not this plan's: the browser drivers of the Agent space describe a page that
roadmap item A2 removes; they are deleted with it.
