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
- **A space per server** - a server that offers a home App is a space of its own: the Cycle's
  projects, when a Cycle is installed (see below), or any other server's surface.

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

## Put it on a server of your own

Started at an address, the Workbench is reached from elsewhere and whoever comes signs in with a
passkey. It needs a name and TLS: a certificate of its own, or your proxy in front.

```text
swem workbench serve --at https://workbench.example.org --tls-cert chain.pem --tls-key key.pem
swem workbench serve --at https://workbench.example.org --behind-proxy \
     --apps-at https://apps.workbench.example.org
```

The first start prints a word that is used once. Open the address, give it the word and register
your device; you are shown codes to come back with, once. A second device is added from one that
is signed in, under Settings, Access, where tokens for programs are made as well. Anything that
would leave the way open - an address without TLS, a listener beyond this machine with no
certificate - is refused in words. A certificate renewed in its files is taken up without a
restart. Without a name, keep it on the server's own loopback and reach it through an SSH tunnel.

Building needs the system's OpenSSL headers (`libssl-dev`, `openssl-devel`, or Homebrew's
`openssl@3`), which the passkey library links.

## Embed it

The product is the crate's builder plus subcommands. An application starts the same Workbench in a
page of code:

```rust
use swem_host::product::{DataRoot, Product};

let served = Product::at(DataRoot::for_this_machine()?)
    .assemble()?
    .serve(([127, 0, 0, 1], 0).into(), ([127, 0, 0, 1], 0).into(), None)
    .await?;
println!("{}", served.url);
```

`cargo run -p swem-host --example embed` is exactly that. The builder takes what a distribution
adds - a shipped catalog, the agent registry to read, declared servers, the container image, the
observer command - and answers the assembled product,
which opens the Workbench door or the editor door. Nothing is process-global: two products in one
process read different roots and registries.

A product with a server, people and a way in of its own builds the harness in without a
listener of the harness's: it hands over the requests it heard, for whom it let in, and draws a
Workbench per person under a path of its own, called by its own name.

```rust
let ada = Product::at(DataRoot::at(root.join("ada")))
    .called("Example")
    .owned_by("Ada")
    .servers_for(|profile| vec![/* the product's server, for this agent */])
    .secrets_kept_by(vault)          // where the product keeps what a person gives
    .assemble()?
    .built_in(BuiltIn { under: "/people/ada/agents".into() })
    .await?;
// In the product's own server, once it knows the request is Ada's:
let response = ada.answer(request).await;
```

`cargo run -p swem-host --example built_in` is a product of that shape with two people, and the
product gate walks it. Who a person is, the product settles before it hands a request over;
the harness takes its word and asks nothing else.

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
and its Project space is its own App, shown as the Cycle's space.

The Store lists it. When `swem-cycle` is installed from there, beside this binary, or on `PATH`,
the product declares its hub as one of its servers (`swem-cycle serve --projects
<data>/projects …`): the hub's home App appears on the switcher, and an agent attaches the hub
by name and names a project on each call. Without it the product is an agent harness alone.

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

AGPL-3.0-or-later. Building SWEM into a product that keeps its source closed needs a commercial
licence from the copyright holder. See `LICENSES.md` for what that means for using, modifying,
running and embedding SWEM, `CONTRIBUTING.md` and `CLA.md` for how changes are taken, and `NOTICE`
for what travels with a copy.
