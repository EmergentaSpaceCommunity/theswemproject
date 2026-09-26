# SWEM

SWEM is a workbench for working with coding agents: a place to install an agent, set it up with a
model, a role, keys, tools and skills, talk to it, keep the sessions, and give it the servers it
needs. It speaks the open protocols agents already speak - the Agent Client Protocol (ACP) to the
agent, the Model Context Protocol (MCP) to the servers an agent reaches, and MCP Apps for surfaces a
server brings with it - and adds nothing of its own on the wire.

It ships as one binary, `swem`, which starts a local web Workbench. The Workbench has three spaces:

- **Agent space** - the agents this computer has, the profiles made of them, the conversation, and
  the Setup of a profile: model provider and model, role, skills, MCP servers, where it runs, how it
  asks for permission, keys.
- **Store** - one list of what can be installed: every agent the public ACP registry lists, and the
  MCP servers and skills of the catalogs you add by address. Nothing is hosted here; every index is
  consumed.
- **Project space** - the projects served by a Cycle, when one is installed (see below).

The same product answers an editor over ACP (`swem acp --profile <id>`), so an editor that speaks
ACP works with the same agent, the same setup and the same sessions as the page.

## Run it

```text
cargo build -p swem-cli --bin swem
./target/debug/swem
```

The product prints the address of the Workbench and opens it. Data lives under
`~/.local/share/swem/workbench` (`$XDG_DATA_HOME/swem/workbench`, `%LOCALAPPDATA%\SWEM\workbench`):
profiles, sessions, the install root (`installed/<kind>/<id>/<version>/`), the indexes the Store
reads, the MCP servers you declare, the model providers you add.

Building the Workbench's page needs Node 22 or newer once:

```text
(cd crates/swem-host/web/apps-host && npm ci && npm run build)
```

## Develop

```text
scripts/suites.sh    # the crate suites and the page's node suite, about a minute
scripts/gate.sh      # the browser walks on the real binary, about ten minutes
```

The gate needs a Chromium or Chrome (`SWEM_BROWSER=/path/to/chrome` when it is not where the script
looks), Node 22+, and `tar`. Walks that need a vendor agent, a Podman machine or the network say so
and skip; nothing is skipped for being red. Run browser walks one at a time (`--test-threads=1`):
two at once contend for the machine hard enough that the browser can miss its debugging port.

The product gate (`crates/swem-cli/tests/product_front_door.rs`) starts the real binary on an empty
data directory and drives a real browser through what a person does: install an agent from the
plan it is shown, make it theirs, set it up, talk to it, keep the session across a restart, run it
in a container, work with it from an editor, add a catalog and install from the Store. Every
control a walk presses is a control a person presses.

## Repository map

| Path | What it is |
| --- | --- |
| `crates/swem-host` | The harness: ACP client, MCP host, MCP Apps host, the Workbench shell and its HTTP surface, agent profiles and environments, the installer and the Store. Knows no domain. |
| `crates/swem-host/web/apps-host` | The Workbench's page (React, TypeScript) and its node suite. `dist/workbench.js` is committed and checked against its source. |
| `crates/swem-cli` | The product: the `swem` binary that assembles the host into what a person runs, and the front-door gate. |
| `web/view-kit` | The design tokens both the page and MCP Apps draw with. |
| `scripts` | The suites, the gate, the container image build, and the count of controls no walk touches. |

## The Cycle

SWEM's project model - one Cycle per project, from what was asked to what was delivered, with the
domain packages that make a piece of music or a software model - is a separate program and a
separate repository, `swem-cycle`. It is an MCP server; this Workbench hosts it like any other,
and the Project space is its surface.

The Store lists it. When `swem-cycle` is installed from there, beside this binary, or on `PATH`,
the product creates projects through its hub: `swem-cycle serve --projects <data>/projects …` is
dialled to make a project, and it answers the declaration that serves that project (its own
`serve-project` over the project's workspace and journal), which the product stores and dials
like any other server. Without it the product is an agent harness alone and says so at start.

## Extending it

Everything the Workbench can be given is an index it reads:

- **Agents** come from the ACP registry (`https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`,
  or the mirror `SWEM_ACP_REGISTRY_INDEX` / `--acp-registry` names).
- **MCP servers and skills** come from a catalog, a `swem:catalog@0.1` document you publish
  anywhere and a person adds by address:

```json
{
  "schema": "swem:catalog@0.1",
  "name": "My tools",
  "entries": [
    {"kind": "server", "id": "notes", "name": "Notes", "version": "1.2.0",
     "description": "the notes an agent keeps", "env": ["NOTES_TOKEN"],
     "distribution": {"npx": {"package": "@example/notes-mcp@1.2.0"}}},
    {"kind": "server", "id": "search", "name": "Search", "version": "0.4.0",
     "distribution": {"binary": {"darwin-aarch64": {"archive": "https://…/search-darwin-aarch64.tar.gz",
                                                     "sha256": "…", "cmd": "./search"}}}},
    {"kind": "skill", "id": "review", "name": "Review", "version": "1.0.0",
     "distribution": {"archive": {"url": "https://…/review.tar.gz", "sha256": "…"}}}
  ]
}
```

A server is an npm package run through node or a native archive per platform, checked against its
digest; a skill is an archive holding a `SKILL.md`. Every install is planned, shown, consented to by
its exact plan id, and receipted under the install root.

## Licence

AGPL-3.0-or-later. See `LICENSES.md` for what that means for using, modifying and running SWEM, and
`NOTICE` for what travels with a copy.
