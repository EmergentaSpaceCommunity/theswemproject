# Working on SWEM

This file is for anyone changing this repository, people and coding agents alike. It is the
operating contract, not the architecture: read `ARCHITECTURE.md` for what the pieces are and
`README.md` for how to run and test them.

## Sources of truth

Recover context in this order: `git status` and the code, tests and runtime behaviour (what exists
now); `README.md` (what a person can do today, and what is not there yet); `ROADMAP.md` (what
comes next, by capability); `ARCHITECTURE.md` and `docs/architecture/` (current boundaries);
`docs/decisions/` (durable decisions, as ADRs). A change that touches a boundary or a decision
updates those; a change that adds or removes something a person can do updates the README's
table.

## What a change is for

SWEM is a product a person uses. A change is finished when a person can do something from a real
entry point - the Workbench, the `swem` command line, an editor over ACP - that they could not do
before, or when a concrete regression, reliability problem or developer blocker is gone. Records,
abstractions, protocols and refactors are means; they are not what a change delivers.

Before substantial work, be able to say in one sentence: *after this, what can a person do that
they could not do before?* If the answer is not concrete, reconsider the task.

## How to work

- **Vertically.** Real entry point → real state → the operation → persistence → a visible result.
  Finish a path a person walks before proving another mechanism underneath.
- **Without speculative infrastructure.** Do not add a generic abstraction, registry, plugin
  mechanism or compatibility layer because it may be useful later. Add it when the capability at
  hand needs it; generalise after repeated pressure appears.
- **On the protocols as they are.** ACP to agents, MCP to servers, MCP Apps for surfaces. Nothing
  SWEM-specific goes on the wire.
- **Domain-free in the host.** `swem-host` names no domain and links no domain crate;
  `tests/genericity.rs` and `cargo tree -p swem-host -e normal` enforce it. What a project *is* is
  the Cycle's business, reached over MCP.
- **One operation, several adapters.** When something is meaningful to a person and to an agent,
  build one operation and expose it through the page and the protocol rather than two
  implementations.
- **Consent to a plan, by id.** Anything the product fetches onto a machine is planned, shown,
  confirmed against the exact plan id, applied and receipted. Do not add a path that installs
  without that road.
- **No second source of truth in the page.** The page reads state from the host and shows it; it
  does not keep its own version of what a profile, a session or a project is.
- **Preserve capability.** Before replacing or removing a working path, show the equivalent
  behaviour or say why keeping it would be wrong. Silent loss of useful behaviour is a regression.

## Validation

Unit tests are necessary where relevant and insufficient for a product claim. Validate narrowly
while developing, then exercise the affected journey through the entry point a person has.

- Browser claims need the browser walk (`scripts/gate.sh`, or one walk by name). Restart and
  persistence claims need a restart. Agent claims need a real supported agent path; the fixture
  agents in `crates/swem-host/src/bin` stand in for a vendor's when they honestly can.
- A walk presses the controls a person presses. It does not satisfy acceptance by calling an API
  the page never calls.
- Run `scripts/suites.sh` before committing and `scripts/gate.sh` for anything that can reach the
  product path. Report what could not be run, and why, rather than implying it passed.
- When the page changes, rebuild `crates/swem-host/web/apps-host/dist/workbench.js`
  (`npm run build`); a test checks the committed bundle against its source.

## Scope

Do not rewrite adjacent architecture while implementing a capability. Note the debt and continue,
unless it blocks acceptance or causes an immediate correctness or safety problem. Do not create a
second implementation of an existing mechanism without naming the old one and how it goes away.

## Before you finish

- format and lint (`cargo fmt --all`, `cargo clippy --workspace --all-targets`);
- run the relevant suites and walks;
- read the diff for unrelated work, duplicated mechanisms and secrets;
- update only the documentation whose truth changed;
- leave the remaining gaps written down, not implied;
- leave the repository coherent for whoever comes next.

Never call something done because a type, a test or a record exists while the thing a person was
promised still cannot be exercised.
