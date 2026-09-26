# ExecPlan 001 — Embed the harness

**Status:** completed (2026-09-26). Roadmap C1; ADR-0003 (the product root composes).

## Outcome

An application starts a Workbench through the crate's product builder in a page of code:
`cargo run -p swem-host --example embed` prints an address, and the product binary is that same
builder plus subcommands. Nothing about the product lives only in the binary: the data root's
layout, the assembly, the container resolver and the doors are the crate's.

## Acceptance scenario

**Given** the crate and an empty directory,
**When** `examples/embed.rs` runs with `SWEM_EMBED_ROOT` pointing at it,
**Then** it prints `SWEM Workbench: http://127.0.0.1:<port>/?token=…` and a browser's first
request to that address is answered with the page; **and** two products assembled in one process
with different registries each read their own; **and** the product binary, rebuilt over the
builder, still passes its first-run walk (`a_person_with_nothing_installs_an_agent_and_can_start_it`).

## Current state (2026-09-26)

- `assemble_product` in `crates/swem-cli/src/main.rs` holds the whole recipe: the data root's
  paths, the boot scan of `projects/*/project.json`, the resolver closure with the Podman
  container path (`in_a_container`, `owner_of`), onboarding, discovery, installs, Store,
  projects, catalogue, model providers, project creation, observer. `run_workbench` adds the
  session secret, the schedules and the HTTP door; `run_acp` the editor door.
- One process-global: `set_acp_registry_index` (a `OnceLock`), read by the Store and the
  install plan.
- No example embeds the host; `tests/workbench_shell.rs` assembles a shell by hand.

## Relevant product journey

GJ-07 (embed the harness), GJ-01 (first run: the product binary is the builder's first user).

## Constraints

`genericity.rs` stays green: the crate spells no project server's subcommand or arguments; the
caller does (`ProjectServer { command, args }`). The product binary keeps every flag it has.

## Plan

1. `crates/swem-host/src/product.rs`: `DataRoot` (the layout by name), `Product` builder,
   `Assembled { state, root }`, `Assembled::serve` (secret, clock, HTTP door) and
   `Assembled::editor_door`; `product/resolver.rs` takes `in_a_container` and `owner_of` as they
   are, the agent's path in the image a builder setting.
2. The registry index moves onto the shell state (`set_acp_registry_index`,
   `acp_registry_index()`); the crate's free function stays the default (environment or the CDN);
   `install_plan_at` takes the index.
3. `examples/embed.rs` in a page of code; `tests/product.rs` proves two products in one process
   and the example's door.
4. `crates/swem-cli/src/main.rs` shrinks to the builder: `product()` builds it, `run_workbench`
   and `run_acp` open the doors, `cycle_hub` spells the project server.

## Progress

- [x] 1 builder, resolver, doors
- [x] 2 registry index on the state
- [x] 3 example and its tests
- [x] 4 the product binary over the builder (validation below)

## Discoveries

- `cargo test` builds a crate's examples beside its binaries (`target/debug/examples/<name>`), so
  a test reaches `examples/embed.rs` from `CARGO_BIN_EXE_<a bin>`'s directory without running
  cargo from inside a test.
- The Cycle distribution lists packages from two directories (the ones beside its binary and the
  person's); the builder takes them as `package_directories`, defaulting to the root's own.

## Decision log

- 2026-09-26: the model providers the product ships and the secret types stay as they are (a
  `OnceLock` of immutable shipped rows); they are constants, not settings, and two products in
  one process share them by definition. The registry index is the one global that was a setting.

## Validation

- 2026-09-26: `tests/product.rs` green (two products, two registries; the example's door
  answers a browser's first request with the page); `genericity`, `workbench_shell` (18),
  `store`, `installs`, `distribution_supply` and the lib's 74 unit tests green; clippy clean on
  the crate and the product; the product gate's first-run walk
  `a_person_with_nothing_installs_an_agent_and_can_start_it` green in Chrome on the binary
  rebuilt over the builder.

## Outcome / remaining gaps

Done. The product binary is the builder's first caller; an application embeds the Workbench in a
page of code. The shipped model providers stay a process-wide constant (decision log). The
Cycle distribution's product root is rewritten over the builder in its own repository.
