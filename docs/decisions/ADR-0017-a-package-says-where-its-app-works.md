# ADR-0017 — A package says where its App works; the host says where it is

**Status:** accepted (2026-10-05). Beside ADR-0006; completes ADR-0016's Apps.

## Context

The harness with the Cycle and the Apps is one environment: an agent makes a feature in one
pass, and the product is the place it lives - a chat, files, Apps, a messenger, a phone. An
App's fitness for a phone cannot be assumed there. Until now the page inside the messenger
offered every App of the agent's servers on a phone, and the bot's "brought an App" button
went out for any App; the host told an App, at `ui/initialize`, its theme and nothing of
where it was, although the MCP Apps specification's host context names `platform`,
`displayMode`, `containerDimensions` and `deviceCapabilities` for exactly that.

## Decision

- **A package declares where its App works, on the App's resource listing, inside the
  specification's own metadata**, beside the home marker of ADR-0006:
  `_meta["swem/platforms"]`, a list from the specification's `platform` words - `web`,
  `desktop`, `mobile`. Undeclared, an App works on `web` and `desktop`: nothing is assumed
  to fit a phone. An App a tool names only by `_meta.ui.resourceUri`, without listing the
  resource, has no listing and so declares nothing. The convention goes when the ecosystem
  standardises a declaration of the kind.
- **The platform is the place, not the width.** The page inside the messenger is `mobile`;
  the Workbench's own page is `web`; the harness never says `desktop`. Width, touch and hover
  are reported where the specification puts them, so a messenger's desktop client at a wide
  window is told `mobile`, `hover: true`, and its width - which is exactly true.
- **What is offered where follows the declaration.** The bot, whose place is the messenger,
  offers its button for an App only when the App declares `mobile`; otherwise it says the
  agent brought an App for the Workbench, and sends no button. The page inside the messenger
  opens only Apps that declare `mobile` and lists the rest as the Workbench's; a brought App
  opens by itself only where it works. The same App, found by the server that brought it and
  the resource the observation names, from the list the page is given.
- **The host tells an App where it is, in the specification's words**: `platform`,
  `displayMode` (`inline` beside a chat, `fullscreen` in a space or on the Apps tab),
  `availableDisplayModes` (the one mode of that place; a request for another is answered with
  the mode there is), `containerDimensions`, `deviceCapabilities`, and
  `ui/notifications/host-context-changed` when the room changes, as when the theme does.
  An App's own display modes are its `availableDisplayModes` in the `ui/initialize` answer -
  the specification's declaration, not a second one of ours.

## Consequences

- `workbench_apps::discovered_apps` keeps the listing's `title` and `_meta["swem/platforms"]`
  on `DiscoveredAppResource`, which reaches the page as the list of Apps and the open App.
  The observation of a tool call names the App's resource. The Apps fixture declares all three
  on its notes App and nothing under `--undeclared`; its acknowledgement records what the host
  told it.
- The Store's catalog carries no per-App facts: the Store learns a server's tools by starting
  it once, and Apps the same way at discovery. A catalog that repeated the server would be a
  second truth.
- `locale`, `timeZone`, `userAgent` and `safeAreaInsets` are not sent until an App needs them;
  no mode switching until an App asks.
- The Cycle's domains serve Apps through the same bridge and declare the same key in their
  own manifests - the Cycle's plan, not this repository's.

## Links

ADR-0006, ADR-0016; `docs/apps.md`.
