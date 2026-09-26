# Architecture

SWEM is an agent harness with a Workbench. It hosts ACP agents, MCP servers and MCP Apps for a
person, and knows nothing about what the person is making with them: the domain side - projects,
packages, music, software - is the Cycle, a separate MCP server in its own repository, which this
Workbench hosts like any other server.

## Two crates

### `swem-host` - the harness

A library plus test fixtures. It owns:

- **The product.** `product.rs`: the data root's layout by name (`DataRoot`), the builder
  (`Product`) that assembles everything the product is - profiles, ledger, declared servers,
  discovery, installs, the Store, projects, the catalogue, model providers, the container
  resolver, project creation - and the doors onto it (`Assembled::serve`, `editor_door`). The
  product binary and an embedding application are both its callers.
- **Agents.** Discovery of ACP agents: a built-in catalogue of the ones this build describes, the
  agents a person declares (`<data root>/agents/*.json`), and the receipts under the install root.
  Launch and the ACP v1 handshake (`verify_launch`), sessions over ACP with the host's own
  permission lane, terminals the agent asks for run in the profile's environment, file callbacks
  held to the profile's workspace.
- **Profiles.** A personal agent profile is one agent set up one way: where it works, where it runs
  (this machine, or a Podman container holding the workspace and nothing else), how it asks for
  permission, which MCP servers it attaches by name, which secrets it is launched with, and its
  setup - model provider and model, role, skills - which the host writes into the agent's own
  files before launch (`agent_setup.rs`: `CLAUDE.md`/`AGENTS.md`, a skills folder, the model as
  the agent's own variable and as the session's option when the agent offers one).
- **The installer** (`install.rs`). One road for everything the product fetches: a plan the person
  reads and consents to by its exact id, a staged fetch checked against the digest the plan named,
  one rename into place, one receipt (`swem:install-receipt@0.1`). Kinds: agent, tool, server,
  skill. Everything lands under `<data root>/installed/<kind>/<id>/<version>/`.
- **The Store** (`workbench_shell/store.rs`). One list over the indexes the product reads: the ACP
  registry, fetched live with the last good copy kept under `<data root>/indexes/`, and the catalogs
  a person adds by URL (`swem:catalog@0.1`). An installed server is declared in the MCP catalogue for
  a profile to attach; an installed skill is read from its `SKILL.md` for a profile to take a copy
  of; an installed agent is rediscovered and offered.
- **The Workbench shell** (`workbench_shell.rs` and its modules). The HTTP surface the page talks
  to, guarded by the page's own origin and a per-run secret; connections, routes and the event
  stream a page long-polls; the MCP catalogue (`mcp_servers.rs`), model providers
  (`model_providers.rs`), schedules, terminals, the editor door (`editor_door.rs`: the same product
  answered over stdio to an editor that speaks ACP), and MCP Apps (`workbench_apps.rs`: a server's
  surface rendered in a sandboxed origin, its calls relayed through the host).
- **The Project space's host side** (`workbench_project.rs`, `workbench_content.rs`): reading a
  project server's resources and calling its tools on behalf of the page, and creating a project
  through a `ProjectFactory` the product hands it. The host reads what the server publishes; it
  never interprets a domain.
- **The page** (`web/apps-host`): React over the shell's HTTP surface. Three spaces - Agent,
  Project, Store - one design vocabulary (`web/view-kit`), no protocol nouns on screen; diagnostics
  behind a Dev switch.

The host names no domain. A structural test (`tests/genericity.rs`) scans its source for domain
words and fails when one appears. `cargo tree -p swem-host -e normal` names no other SWEM crate.

### `swem-cli` - the product

The `swem` binary: the crate's product builder over this machine's data root, given what this
distribution adds - the catalog it ships, the observer command, and the Cycle hub beside it, from
the Store or on `PATH`, as the server projects are made on. Subcommands cover the same ground from
a terminal: `swem agents list|plan-install|install|verify|session`, `swem profiles`,
`swem environments`, `swem acp --profile <id>`, `swem workbench serve`.

The product knows the Cycle only as a server it can be given: its name (`swem-cycle`), the hub's
subcommand (`serve --projects …`) it dials to make a project, and the `create_project` tool that
answers a project's declaration. Nothing of the Cycle is compiled in.

## Boundaries that hold

- **The wire is the protocols'.** ACP to agents, MCP to servers, MCP Apps for surfaces. Nothing
  SWEM-specific travels on them; an agent or a server that never heard of SWEM works.
- **Indexes are consumed, never hosted.** The Store reads the ACP registry and catalogs others
  publish. There is no registry backend and no marketplace.
- **Consent is to a plan, by id.** Every install shows what it would fetch, from which index, and
  applies only against the exact plan the person confirmed. A plan that moved is refused.
- **Secrets never reach a record.** A profile's secrets live in its vault and are injected at
  launch; a declaration's environment values are never listed back.
- **The page does not hold domain truth.** What the page shows of a project is read from the
  server that serves it, at the revision it names.
- **A walk presses what a person presses.** The product gate drives the real binary in a real
  browser through real controls; no test satisfies a product claim by seeding an internal API.

## Data root

```text
<data root>/
  profiles/<id>/profile.json      a personal agent profile, its vault beside it
  routes.jsonl, routes.sqlite3    sessions and their events
  agents/*.json                   agents a person declared
  installed/<kind>/<id>/<ver>/    everything installed, each with installation.json
  indexes/                        the registry's last good copy, added catalogs
  mcp-servers/<name>.json         MCP servers a person declared or installed
  model-providers/<id>.json       model providers a person added
  workspaces/, agent-homes/       where profiles work and live
  environments/                   prepared environments (containers)
  projects/<slug>/                projects, when a Cycle serves them
```
