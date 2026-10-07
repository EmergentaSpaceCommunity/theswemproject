# Architecture

SWEM is an agent harness with a Workbench. It hosts ACP agents, MCP servers and MCP Apps for a
person, and knows nothing about what the person is making with them: the domain side - projects,
packages, music, software - is the Cycle, a separate MCP server in its own repository, which this
Workbench hosts like any other server.

## Five crates

### `swem-host` - the harness

A library plus test fixtures. It owns:

- **The product.** `product.rs`: the data root's layout by name (`DataRoot`), the builder
  (`Product`) that assembles everything the product is - profiles, ledger, declared servers,
  discovery, installs, the Store, the catalogue, model providers, the container resolver - and
  the doors onto it (`Assembled::serve`, `editor_door`). The
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
- **The Store's takers** (`workbench_shell/store.rs`). The Store itself is `swem-store`, below;
  the harness registers what it takes: an agent (rediscovered and offered), an MCP server
  (declared in the catalogue for a profile to attach, its values kept across an update, its
  declaration forgotten before a removal), a skill (read from its `SKILL.md` for a profile to
  take a copy of). A declared or installed server whose catalog entry says `takes` is reached
  through the harness's own MCP client for the kind it takes. A product the harness is built into
  registers takers of its own (`Product::takes`). ADR-0012.
- **Channels** (`workbench_shell/channels.rs`). How people reach an agent from a messenger: a
  channel is a package of kind `swem/channel@1`, a program the harness starts and keeps and
  talks to through one shape (`swem_sdk::channel`); the harness pulls what arrived and says it
  in the bound chat as the bound participant, and carries an agent's turn back as it is
  written. Owner by pairing code, anybody else a guest by the channel's policy; the harness
  names no messenger. The Telegram channel ships as `swem-channel-telegram` beside the binary
  (`crates/swem-channel-telegram`: the Bot API over long-polling, drafts while a turn is
  written, Markdown to the HTML Telegram shows, a door with a secret when the Workbench is served
  at an address, the messenger's signature of who opened the Workbench's page inside it). The
  page inside the messenger is the Workbench's own page, drawn for somebody who came through a
  messenger: the signature is exchanged once at the door for a session, and who asks is a
  participant with a scope (`access.rs`, `door.rs`). An App reaches the messenger only where
  its package declared it works (`_meta["swem/platforms"]`, ADR-0017). A bot belongs to one
  harness and its token is one more key. ADR-0013, ADR-0014, ADR-0016.
- **Tunnels** (`workbench_shell/tunnel.rs`). An address from outside for a while, for a
  Workbench that is not served at one: a package of kind `swem/tunnel@1` (the twin of a
  channel, `swem_sdk::tunnel`) stands at a public address for the gate - a second loopback
  listener that runs the one router told it is the gate, where only a messenger session is let
  in. `swem-tunnel-cloudflare` ships beside the binary and drives the `cloudflared` tool the
  Store installs. The harness names no tunnel vendor. ADR-0015, ADR-0016.
- **The ledger of chats** (`routing.rs`, `chat_ledger.rs`, `chat_work.rs`). One SQLite file:
  participants, chats and their members, messages with their sender, the session an agent has
  in a chat, what a chat owes an agent (deliveries) and what an agent asks (questions), and
  under them the events of every session in one order. ADR-0007.
- **Keys** (`keys.rs`, `workbench_shell/provider_keys.rs`). A key belongs to the provider it
  opens. It is kept under `<data root>/keys`, a directory its owner alone can enter, one file
  per provider; the engine of an agent on this machine and the terminals it opens are handed
  the key of the agent's provider under the variable that kind of key is read from, then what
  the agent holds of its own over it. A key is never written into a profile, the ledger or a
  command line.
- **Time** (`time.rs`, `workbench_shell/timekeeper.rs`, `time_tools.rs`). A schedule is a message
  that arrives on time: what is said, when (every so often, a cron line in a named zone,
  once), to which agent, in which chat, made by whom. Schedules and their runs are tables of
  the ledger, added beside the rest without moving its version. One process keeps time, the
  one that holds the keeper's lock beside the ledger; a run is claimed by its schedule and
  the moment it was due. What was missed is said once, late; a question asked with nobody
  there is refused after half an hour in the engine's own words. An agent makes schedules
  for itself through a server handed to its session (`swem-time`), whose agent and chat are
  fixed when it is started. Nothing about time lives in an agent's machine.
  Who keeps time is a provider (`keepers.rs`, `workbench_shell/keepers.rs`,
  `host/system_scheduler.rs`): the running product, and the system's own scheduler, which
  is given a job (launchd, a systemd user timer) that starts `swem time keep` every minute.
  That command takes the keeper's lock or leaves, says what is due to the agents whose time
  the system keeps, stays until each turn has ended and stops; a look that finds nothing to
  do is told from the ledger alone. The choice - the keeper agents have, and the one chosen
  for an agent - is kept under `<data root>/time`, not in a profile. A Workbench that found
  the lock taken asks for it again as time passes. The job holds the command, where the data
  is and the search path; never a key.
- **Removal** (`workbench_shell/removal.rs`). An agent that is removed is retired in the ledger -
  its profile and its handle let go of, what waited for it not begun - its schedules are
  forgotten, and its profile is set aside under `<data root>/removed` without what it held of
  its own. Its chats, the folder it worked in and its home are not touched.
- **The look inside** (`workbench_shell/looks.rs`). What an agent has where it lives: its engine
  started as the agent has it, greeted and asked for a session (`try_the_engine`), the machine
  as the runner finds it from inside, and whether a container can be started there. Kept under
  `<data root>/hosts/looks`, one file per agent, with its time.
- **An agent's files** (`workbench_shell/files.rs`). The folder an agent works in as a tree, a
  file opened, saved while it is what was opened, made, renamed and removed: each is a request
  to the runner. What a person handed over and what the agent handed back (`workbench_files.rs`)
  are two folders of the same place, listed beside the tree.
- **Hosts** (`host/`, `workbench_shell/hosts.rs`). This machine is looked at while the door
  opens and when a person asks: system, processors, memory, disk, and whether Podman and
  Docker are there and answer. What was found is kept under `<data root>/hosts`. Where an
  agent may live is offered from it, and a place that cannot take an agent today says why.
  Setting containers up is the plan the command line has (`podman_provisioning_plan`), kept by
  the shell until a person agrees to it by its id; its steps are run one by one, how each went
  is read from `GET /api/hosts`, and the machine is looked at again at the end.
- **The Workbench shell** (`workbench_shell.rs` and its modules). The HTTP surface the page talks
  to. Who asks is found in one place before anything is answered (`door.rs`): on the machine a
  person sits at, by the page's own origin and the secret of the run; served at an address, by
  sign-in - a device that holds a passkey, a program that holds a token that says what it may
  do, a code to come back with (`access.rs`, the book of who may come in; ADR-0011). Served at
  an address the way is TLS, by a certificate of its own read from files or by a proxy in front
  on the same machine (`product/at_an_address.rs`), and anything else is refused in words.
  Built into a product (`product/built_in.rs`) nothing listens: the product hands over what it
  heard, for whom it let in, and the Workbench is drawn under a path of the product's server.
  What a person gives - keys, a profile's secrets, declared servers' values - is kept by one
  keeper (`secrets.rs`): files closed to others, or what the product supplies. Chats (`chats.rs`: who a message is
  for - who it names, every agent where the chat says so, and the agent that asked), the work of answering them (`runtime.rs`: one turn at a time per agent, claimed in the
  ledger and under a file lock because the editor door is another process; a session is opened
  when a message needs it and let go of when idle; a chat whose engine lost its session goes on
  in a fresh one that is given what was said), and the one stream a page follows every chat by
  (`stream.rs`, server-sent events from a place in the ledger's order; a page that hears
  nothing - a tunnel's edge holds a stream back - asks for the same record, `/api/now` and
  `/api/happened`, instead). What an engine is told
  about who spoke is one block the host makes per turn (`envelope.rs`). Beside them: the MCP
  catalogue (`mcp_servers.rs`), model providers
  (`model_providers.rs`), terminals, the editor door (`editor_door.rs`: the same chats
  answered over stdio to an editor that speaks ACP), and MCP Apps (`workbench_apps.rs`: a server's
  surface rendered in a sandboxed origin, its calls relayed through the host: `tools/call` of a
  tool its server declares as App-visible, and `resources/read`; a tool the host has not seen is
  asked of the server once more before it is refused, because a server may gain tools while it
  runs).
- **Spaces** (`workbench_shell/server_apps.rs`): a declared server that marks one of its App
  resources as its home is a space on the Workbench; opening the space opens that App, outside
  any agent session, through the same sandbox and the same relay. The host keeps no registry of
  spaces and draws none for a server. The page keeps an opened space open while a person is
  elsewhere on it.
- **What the agent is given** (`workbench_shell/model_context.rs`): an App says what a person is
  looking at (`ui/update-model-context`); the page keeps the last one and sends it with the
  person's next words; the host checks its shape, gives the blocks above the turn to an agent
  that attaches the App's server, and leaves them out for one that does not. It interprets
  none of it.
- **Content** (`workbench_content.rs`): the bytes a person hands an agent and an agent hands
  back, by descriptor.
- **The page** (`web/apps-host`): React over the shell's HTTP surface. A rail of agents and
  chats, Providers, the Store, and one place per server that offers a home App; an agent has
  its chat, its files, its terminal, its schedules and its settings, which are forms by part
  (`react-hook-form`). A chat is drawn from a store per chat, fed by the one
  stream. The thread and composer are `@assistant-ui/react` over those stores, text is
  `react-markdown`, code is coloured by `shiki`, dialogs and selects are Base UI. An agent's files
  are a tree (`headless-tree`) and an editor (CodeMirror 6). It is styled
  by the project's kit (`web/view-kit`: the palette's variables only) and its own layout
  (`workbench_shell/workbench.css`), in both themes. No protocol nouns and nothing for
  debugging on screen.

The host names no domain. A structural test (`tests/genericity.rs`) scans its source for domain
words and fails when one appears. `cargo tree -p swem-host -e normal` names no other SWEM crate
but the runner.

### `swem-sdk` - the vocabulary of the Store

Apache-2.0, so that anything written against it is its author's under any licence. Kinds by name
(`Kind`: the four built-in words, else reverse-DNS with a version, `swem.cycle/package@1`);
catalogs (`swem:catalog@0.2`, `@0.1` read as a subset) and their entries, distributions,
`requires` and `takes`; the install plan and the receipt (`swem:install-receipt@0.1`); `Taker`,
what a host registers per kind (words, accepts, check, after_install, removable, before_remove);
`Through`, how a host lets a server that says it `takes` a kind be called; `shape`, the tools a
host calls compared with what a server lists. Nothing in it installs anything.

### `swem-store` - the Store

A library over the vocabulary that knows no host. The ACP registry and catalogs as indexes, read live with the last good copy kept under `<data root>/indexes/`;
the installer (`install.rs`): a plan consented to by its exact id, a staged fetch checked against
the digest the plan named, the taker's check of the staged tree, one rename into place, one
receipt (`swem:install-receipt@0.1`) under `<data root>/installed/<kind>/<id>/<version>/`;
distributions `npx`, `uvx`, `binary`, `archive`; `requires` planned as the whole closure under one
consent whose id digests it, recorded in the receipt, and refusing to remove what is still
required; versions by semver where they parse; update and removal; the delegated taker that calls
a server through the host. ADR-0004, ADR-0012, ADR-0018.

### `swem-runner` - what is put on a host

A library and a small program (ADR-0008). What the product wants of an agent's machine is
asked of it: the agent's files (`fs`), confined to the folder the agent works in; a program
started as a header says (`exec`), so that a key is in nobody's command line; a look at the
machine from inside (`probe`). The harness uses the first today. On the
machine the product runs on the library answers in the product's own process; in a machine of
the agent's own the program answers on its standard streams. It depends on nothing of the
harness.

### `swem-cli` - the product

The `swem` binary: the crate's product builder over this machine's data root, given what this
distribution adds - the catalog it ships, the observer command, and the Cycle hub beside it, from
the Store or on `PATH`, declared as one of the product's servers. Subcommands cover the same ground from
a terminal: `swem agents list|plan-install|install|verify|session`, `swem profiles`,
`swem environments`, `swem acp --profile <id>`, `swem workbench serve`.

The product knows the Cycle only as a server it declares: its name (`swem-cycle`) and how the
hub is started (`serve --projects …` over directories of the product's data root). What the
hub serves, and the space it shows, are the hub's. Nothing of the Cycle is compiled in.

## Boundaries that hold

- **The wire is the protocols'.** ACP to agents, MCP to servers, MCP Apps for surfaces. Nothing
  SWEM-specific travels on them; an agent or a server that never heard of SWEM works.
- **Indexes are consumed, never hosted.** The Store reads the ACP registry and catalogs others
  publish. There is no registry backend and no marketplace.
- **Consent is to a plan, by id.** Every install shows what it would fetch, from which index, and
  applies only against the exact plan the person confirmed. A plan that moved is refused.
- **Secrets never reach a record.** A profile's secrets live in its vault and are injected at
  launch; a declaration's environment values are never listed back. What opens the Workbench -
  a word, a code, a session, a token - is kept as its digest. The data root and every file
  that holds a value a person gave are their owner's alone (`closed.rs`).
- **The page does not hold domain truth.** What a server's space shows is the server's App,
  reading the server through the host's relay; the host's own page draws none of it.
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
  projects/, plugins/             the Cycle hub's, when the product declares one; the harness
                                  reads neither
```
