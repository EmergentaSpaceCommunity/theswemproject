# Apps

An App is a surface a server brings: an MCP Apps resource (`ui://…`, MIME
`text/html;profile=mcp-app`), drawn by the Workbench in a sandbox of its own origin and
spoken to over the MCP Apps protocol, with every call relayed through the harness. A server
of an agent brings Apps into the agent's chat; a server with a home App is a space of its own
(ADR-0006). The page inside the messenger draws them too (ADR-0016). This page is for whoever
writes a server with an App.

## Declaring an App

List the resource in `resources/list` with the App MIME, and link it from the tools that bring
it (`_meta.ui.resourceUri` on the tool, with `_meta.ui.visibility` as the specification says).
The resource's content carries the specification's own `_meta.ui` - `csp`, `permissions`,
`prefersBorder`, and `origin: "isolated"` when the View needs a real origin (storage, workers,
cross-origin isolation).

On the resource **listing**, two keys of SWEM's inside the specification's `_meta`:

- `swem/home: true` - this is the server's home App, shown as a space of its own.
- `swem/platforms: ["web", "desktop", "mobile"]` - where the App works, in the
  specification's `platform` words. Leave it out and the App works on `web` and `desktop`:
  nothing is assumed to fit a phone. An App a tool names without listing the resource declares
  nothing, and so is the Workbench's only - to be offered on a phone, list it.

The fixture the harness tests with declares, in `swem-mcp-apps-fixture`:

```json
{"uri": "ui://apps-fixture/notes", "title": "Notes", "mimeType": "text/html;profile=mcp-app",
 "_meta": {"swem/platforms": ["web", "desktop", "mobile"], "swem/home": true}}
```

Both keys go when the ecosystem standardises what they say.

## What the host tells an App

At `ui/initialize`, and again as `ui/notifications/host-context-changed` whenever it changes,
the host context of the specification: the `theme` and the style variables (the design
system's whole vocabulary), and where the App is - `platform` (`web` on the Workbench's page,
`mobile` on the page inside a messenger; the place, never the width), `displayMode` (`inline`
beside a chat, `fullscreen` in a space or on the messenger page's Apps tab),
`availableDisplayModes` (the one mode of that place), `containerDimensions` and
`deviceCapabilities` (touch, hover). An App that adapts to a phone adapts on these; the
declaration above still decides whether it is offered there.

## Where an App is offered

On the Workbench, every App of an agent's servers is beside its chat, and a home App is a
space. On the page inside the messenger, the Apps tab opens the Apps that declared `mobile`
and lists the rest as the Workbench's; when a tool brings an App, the bot offers it with a
button if it declared `mobile`, and says it is for the Workbench if not. Through a tunnel the
sandbox stands at a second address of the same tunnel package.

## What is refused

The relay allows `tools/call`, `tools/list` and `resources/read`, never `ui/*`; a tool with
visibility `["model"]` cannot be called from an App; a server cannot reach another server's
tools; a resource that is not an App cannot be opened as one. The tests in
`crates/swem-host/tests/workbench_shell_apps*.rs` and `apps_fixture.rs` are the exact list.
