# Contributing

Issues, ideas and patches are welcome. Open an issue for a bug or a gap you hit; open a
discussion for an idea; open a pull request for a change.

## The agreement

Every contribution is taken under the Contributor License Agreement in `CLA.md`. You keep the
copyright in what you wrote; the agreement grants the author - and the author alone - the right to
license your contribution together with the rest of SWEM under other terms as well as the AGPL.
It gives you nothing beyond the AGPL, which is what everyone has. `LICENSES.md` says why it is
shaped that way.

You agree to it by putting this line in the description of your first pull request, and it is
read as standing for every later one:

```text
I have read CLA.md and agree to it.
```

A pull request without it is read and discussed, and is not merged.

## Before you send a change

`AGENTS.md` is the operating contract for anyone changing this repository, people and coding
agents alike; read it first. In short:

- a change is finished when a person can do something from a real entry point that they could
  not do before, or a concrete problem is gone;
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/suites.sh`;
  `scripts/gate.sh` when the change can reach the product's page;
- when the page changes, rebuild `crates/swem-host/web/apps-host/dist/workbench.js`
  (`npm run build`); a test checks the committed bundle against its source;
- say in the pull request what you ran and what you could not run.

Changes written with a coding agent are welcome; you are responsible for what you send, and
`AGENTS.md` is written to be handed to the agent.

## Running the suites and the gate

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

## What goes where

- A bug or a regression: an issue with what you ran, what you expected and what happened.
- A security problem: not an issue; see `SECURITY.md`.
- A durable decision about a boundary: an ADR in `docs/decisions/`, in a pull request of its own
  or with the change that needs it.
- Something a person can newly do: the table in `README.md` and, if it was there, the item in
  `ROADMAP.md`.
