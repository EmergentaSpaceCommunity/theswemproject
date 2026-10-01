# Building the harness into a product of your own

`swem-host` is a library. A product that has a server, people and a way in of its own builds a
Workbench into it in a page of code: one per person, each over a data root of its own, drawn under
a path of the product's server, called by the product's name, given the product's servers and
taking packages of the product's own kind through the Store. This document says everything such
a product needs and everything it does not get yet. `crates/swem-host/examples/built_in.rs` is a
product of that shape and the gate walks it; every snippet below is taken from it or from the
tests.

## The builder

```rust
use std::sync::Arc;
use swem_host::product::{BuiltIn, DataRoot, Product};

let data_root = DataRoot::at(root.join("ada"));
let helpers = Helpers::at(&data_root.installed())?;       // a taker of the product's own kind
let agents = Product::at(data_root)
    .called("Example")                                    // the name on the page
    .owned_by("Ada")                                      // the person the Workbench is for
    .takes(Arc::clone(&helpers) as Arc<dyn swem_host::Taker>)
    .servers_for(move |profile| { /* the product's servers, for this agent */ vec![] })
    .secrets_kept_by(vault)                               // where what a person gives is kept
    .assemble()?
    .built_in(BuiltIn { under: "/people/ada/agents".into() })
    .await?;
```

What the builder takes, and what each is for:

| Call | What it does |
|---|---|
| `Product::at(DataRoot)` | One Workbench over one root: profiles, the ledger of chats, keys, schedules, installed packages, indexes. One process assembles as many as it likes (three hundred take 12.6 s and 8 MB together). The root is made 0700. |
| `.called(name)` | The page's title and brand, and the words that say what came with the product. |
| `.owned_by(name)` | The person the Workbench belongs to, as the participant every chat has. |
| `.servers_for(fn)` | MCP servers resolved for an agent when its session opens, after the servers its profile attaches and never twice under one name. The page shows them as "Given by \<name\>". What of the product an agent may reach is settled here, per profile, by the product. |
| `.secrets_kept_by(keeper)` | Where keys and grants go instead of files under the root: a `SecretKeeper` of `read`, `write`, `remove`, `list` over documents named `keys/<provider>.json`, `profiles/<id>/secrets.json`, `mcp-servers/<name>.json`. Given one, nothing of them touches the root. |
| `.takes(taker)` | A kind of package of the product's own, taken through the Store. Below. |
| `.shipped_catalog(catalog)` | What the product ships as already there: bundled servers, and what they take. |
| `.declare(server)` | A server the product runs beside the harness, attachable by name and reachable for what it takes. |
| `.acp_registry(url)` | The index agents come from, when not the public one. |
| `.container_image`, `.agent_in_image`, `.mcp_observer`, `.time_tools`, `.keep_time`, `.operation_timeout` | What a distribution sets: the image agents are sealed in, the observer, the tools for schedules, the keeper of time, how long a turn may take. |

## Three ways to open a door

| | Who listens | Who says who asks | Sign-in |
|---|---|---|---|
| `Assembled::serve(bind, apps_bind, bundle)` | the harness, on loopback | the secret of this run, carried by the page | none: this machine |
| `Assembled::serve_at(ServeAt {address, listen, closed, apps_address, ..})` | the harness, at a name, over TLS (`Closed::Certificate`, `Closed::ProxyInFront`) | passkeys, tokens for programs, codes to come back with | the harness's own |
| `Assembled::built_in(BuiltIn {under})` | nobody: the product hands requests over | the product, before it calls `answer` | the product's own |

Built in, `Answering::answer(request)` takes any `http_body::Body` request and answers as the
Workbench does; the page asks its host beside where it was opened, so it works under any path. A
request handed over is answered for whom the product let in, and for nobody else: call `answer`
only for a request the product has decided is this person's, because whoever it is answered for
has this Workbench's terminals and keys. The example does that with a cookie of its own and
answers 403 itself otherwise; the gate checks that nothing behind the path answers a stranger.

## Chats from outside

The routes are the surface: `POST <under>/api/chats` with `{"agents": ["<profile>"], "title":
""}` opens a chat with those agents, and `POST <under>/api/chats/<id>/say` with `{"text": "...",
"blocks": []}` says something in it as the owner. A button of the product's that opens
`<under>/#/agents/<id>` opens that agent's chat on the page. There is no address yet that opens
the page on a chat with words filled in.

## Time

The harness keeps time in the process that assembled it and runs the turns there: a product that
stays up needs nothing more. A product that sleeps misses what was due while it slept; on return
it says what was due once, up to a day late, marked late. A scheduler outside can knock with
`POST <under>/api/time/due`, let in by a token that may say what is due and nothing else (made
under Settings, Access); a knock when nothing is due, or twice, does no harm. Nothing else runs a
turn: not a runner in an agent's machine, not a second cron of the product's.

## The Store: kinds and takers

The Store knows kinds only by name. A **host** registers, for each kind it takes, the words for
it, what it accepts, how a candidate is checked before it is promised, and what happens after
it is installed and before it is removed. The harness takes agents, MCP servers and skills. An
installed server that says it takes a kind takes packages for itself. A product the harness is
built into takes kinds of its own. A kind nobody on an installation takes is listed as for
something this product does not have, and cannot be installed.

A kind is named as a product names its own: reverse-DNS with the version of the kind, as
`example/helper@1`. A catalog is a `swem:catalog@0.2` document published anywhere and added by
address:

```json
{
  "schema": "swem:catalog@0.2",
  "name": "Helpers for the example",
  "entries": [
    {"kind": "example/helper@1", "id": "echoes", "name": "The helper that echoes", "version": "0.1.0",
     "distribution": {"archive": {"url": "https://…/echoes.tar.gz", "sha256": "…"}},
     "requires": [{"kind": "server", "id": "notes"}]},
    {"kind": "server", "id": "hub", "name": "The hub", "version": "1.0.0", "bundled": true,
     "takes": [{"kind": "example.hub/package@1", "plan": "plan_package",
                "install": "install_package", "remove": "remove_package",
                "one": "a package for the hub", "many": "Packages for the hub"}]}
  ]
}
```

Distributions: `npx` (an npm package, run through node), `uvx` (a Python package, installed by
`uv tool install` into the install root, its entry program found by name), `binary` (an archive
per platform with the program in it), `archive` (a tree, for skills and for kinds of your own).
Every one is checked against the digest the catalog gave. `requires` names what a package needs
from the same indexes; the plan lists it as "also installs" and one consent covers the closure; a
requirement nobody's index has is refused in words. Versions are compared as semver where they
parse; "Update to X" is offered where an index has a newer one; a server updated keeps the values
a person gave it.

### A taker in process

The vocabulary a taker is written against - `Kind`, `KindWords`, `Taker`, `CatalogEntry`,
`InstallPlan`, `InstallReceipt`, `Shape` - is the crate `swem-sdk`, Apache-2.0, re-exported by
`swem-host`; a taker, a host or a tool for catalogs written against it is yours under any licence
(`LICENSE-EXCEPTION.md`).

```rust
use std::path::Path;
use swem_sdk::{CatalogEntry, InstallPlan, InstallReceipt, Kind, KindWords, Shape, Taker};

impl Taker for Helpers {
    fn kind(&self) -> Kind { Kind::parse("example/helper@1").expect("a kind") }
    fn words(&self) -> KindWords {
        KindWords { one: "a helper for Example".into(), many: "Helpers".into(),
                    after_install: "Every agent of yours has it.".into() }
    }
    fn accepts(&self, entry: &CatalogEntry) -> Result<(), String> {
        if entry.distribution.archive.is_none() { return Err("a helper is an archive".into()); }
        Ok(())
    }
    fn check(&self, staged: &Path, plan: &InstallPlan) -> Result<(), String> {
        let server = Self::server_in(&staged.join("tree"), &plan.registry_id)?;
        let listed = swem_host::tools_listed_by_blocking(&server)?;
        self.shape.check(&listed).map_err(|short| format!("{} is not a helper: it {short}", plan.name))
    }
    fn after_install(&self, receipt: &InstallReceipt) -> Result<(), String> { /* keep it, give it */ Ok(()) }
    fn removable(&self) -> bool { true }
    fn before_remove(&self, receipt: &InstallReceipt) -> Result<(), String> { /* let it go */ Ok(()) }
}
```

`check` sees the staged tree before anything is committed: refused, nothing lands and the person
reads why. `after_install` sees the receipt, whose `file` is the tree where it now lives
(`<installed>/<kind>/<id>/<version>/tree`); `load_receipts(installed, &kind)` reads them back at
start. `check` runs on a blocking thread; `tools_listed_by_blocking` starts a server once, lists
its tools and stops it from there, and `tools_listed_by` is the same for async code.

### Shapes

No standard lets a server declare "I implement tool set X". A product that takes a kind of MCP
server writes the shape it calls and checks every candidate against it:

```json
{"schema": "swem:shape@0.1", "id": "example/helper",
 "tools": [{"name": "echo", "inputSchema": {"type": "object",
            "properties": {"nonce": {"type": "string"}}, "required": ["nonce"]}}]}
```

`Shape::parse` reads it; `Shape::check(&listed)` answers, in words, every way a server falls
short: a tool it does not answer, a property it does not take or takes as another type, a
property it requires that the host does not always send. Names and generated schema noise
(`$schema`, `title`, descriptions, extra optional properties) do not matter.

### A server that takes a kind

A server declared or installed whose catalog entry has `takes` installs packages of that kind
for itself. The Store fetches, checks the digest and stages; then it calls the server's `plan`
tool on the staged tree (`{"source": {"kind": "directory", "path": "<tree>"}}`) and refuses the
package if the server does; commits the tree; calls `plan` again on the tree where it lives and
`install` with the `plan_id` the server answered (`{"plan_id": "..."}`); `remove` is `{"id":
"..."}`. The argument shapes are those defaults unless `plan_arguments`, `install_arguments` and
`remove_arguments` give templates with `{tree}`, `{plan_id}`, `{id}`, `{version}` in them, and
`answers` names the field the plan id is read from. The Store writes its own receipt beside, so
the page lists, updates and removes it. `crates/swem-host/src/bin/swem-mcp-taker-fixture.rs` is
the smallest server of that shape; `crates/swem-host/tests/store.rs` installs through it.

## Linking

`swem-host` depends on `rusqlite 0.32` (`libsqlite3-sys 0.30`), which links beside `sqlx 0.8`'s
SQLite in one workspace; `scripts/links-beside-sqlx.sh` builds a program that depends on both.
The passkey library links the system's OpenSSL headers (`libssl-dev`, `openssl-devel`, Homebrew's
`openssl@3`). The MCP client is `rmcp 3.5` with `process-wrap 10`.

## What is not there

- Apps of servers drawn in a harness built in for several people: the address Apps are drawn at
  is an origin with no path.
- The page's palette: its CSS is the harness's. The name is the product's.
- An address that opens the page on a chat with words filled in.
- Machines that are not this one, providers offered from above, and the built-in machine and
  keepers hidden by who embeds it: every agent runs on the machine the harness runs on, directly
  or in a container.
- A `uvx` package pinned by hash: `uvx` pins a version; a lockfile per install is a later step.

## Checking it

```text
cargo test -p swem-store                                     # the Store alone: kinds, takers, shapes
cargo test -p swem-host --test store                          # the harness's takers and a server that takes
cargo build -p swem-host --bins --example built_in
cargo test -p swem-cli --test product_front_door a_product_of_ones_own -- --ignored   # the walk
scripts/links-beside-sqlx.sh                                 # links beside sqlx
```
