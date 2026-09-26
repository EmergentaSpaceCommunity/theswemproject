# ExecPlan 002 — A server's home App is a space

**Status:** completed (2026-09-26). Roadmap C4 (the mechanism, C4a); ADR-0006.

## Outcome

A server that declares a home App appears on the space switcher beside the host's own spaces,
and choosing it mounts the App in the main area, opened without a tool call. Installed from the
Store or declared by hand, any such server gets the same treatment; the host keeps no registry
of spaces.

## Acceptance scenario

**Given** the Apps fixture declared in the MCP catalogue with `--home`,
**When** the page opens,
**Then** the switcher offers `notes` beside Agent, Project and Store; choosing it mounts the
fixture's App in the main area (`workbench_shell_spaces_browser`); `/api/spaces` lists it and
the same server without the marker is not a space (`workbench_shell_spaces`).

## State before (2026-09-26)

The host discovered App tools and resources per attached server; a project's App opened through
its project (`/api/projects/{server}/apps/open`) without a tool call and read the server through
the relay. The page drew three spaces of its own. ADR-0006 proposed a marker on a tool.

## Constraints

Inside the Apps specification's own metadata; no registry; `genericity.rs` green.

## Plan

1. `DiscoveredAppResource.home` from `_meta["swem/home"]` on the App resource.
2. `spaces()` over every declaration (projects and the catalogue), each server dialled once and
   kept; `space_open(server, uri)` reuses the project's App open without the project check.
   Routes `GET /api/spaces`, `POST /api/spaces/{server}/open`.
3. The page: one switcher button per space, the App mounted in the main area through the
   space's own door; the fixture's `--home`.
4. Tests: `workbench_shell_spaces.rs`, `workbench_shell_spaces_browser.rs`.

## Progress

- [x] 1 marker — `HOME_APP_MARKER` read off the App resource's `_meta` in `workbench_apps.rs`.
- [x] 2 host — `SpaceView`, `every_declaration`, `spaces`, `space_open` and the shared
  `open_app_of` in `workbench_shell/project_apps.rs`; the two routes in `workbench_shell.rs`.
- [x] 3 page and fixture — `Space` gains `{ server }`, `useSpaces` polls `/api/spaces`,
  `#space-server-<server>` buttons, `<main id="server-space">` mounting `DomainApp` with an
  `openPath`; `swem-mcp-apps-fixture --home` marks its App resource.
- [x] 4 tests — in-process, browser, and the Store-to-space gate walk.

## Discoveries

- A server that has not answered `list_resources` once cannot be a space; the listing dials each
  declaration lazily and keeps the child, so the first `/api/spaces` after a Store install takes
  the server's start-up time. The page polls every ten seconds and on visibility change, which
  covers a fresh install without a reload.
- The browser driver read the App's status line too early ("sandbox ready" is an intermediate
  state); the driver waits until the status text is empty before asserting the App's content.

## Decision log

- 2026-09-26: the marker sits on the App resource, not on a tool. A project's App already
  opens without a tool call and reads the server through the relay; a home App is the same
  door, so ADR-0006 is amended to say so.

## Validation

- `cargo test -p swem-host --test workbench_shell_spaces` — 2 passed (a marked server is a
  space and opens; the same server without the marker is not).
- `cargo test -p swem-host --test workbench_shell_spaces_browser` — 1 passed in Chrome: the
  switcher offers the fixture, choosing it mounts the App, the App reads the server.
- `cargo test -p swem-host --test genericity`, `--test workbench_shell_apps` (9),
  `--test web_bundle_freshness` — green.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- Product gate: `cargo test -p swem-cli --test product_front_door
  a_person_installs_a_server_with_a_home_app_and_opens_its_space -- --ignored` — a person
  installs the fixture from the Store on the real binary's page, the new space appears without
  a reload, and choosing it mounts the App. One earlier run failed only on the known
  "browser never opened a DevTools port" flake and passed alone.

## Outcome / remaining gaps

Landed. What remains is the Cycle's side: the hub must ship the Project space as its home App
(roadmap C4b, in the Cycle repository) before C4c deletes the host's own Project space.
