# ADR-0006 — A server that declares a home App gets a space of its own

**Status:** accepted (2026-09-26 for the mechanism, roadmap C4a, ExecPlan 002; 2026-09-27 the
host's own Project space left it, roadmap C4c, ExecPlan 003).

## Context

MCP Apps let a server ship its own surface, rendered by the host in a sandboxed origin and bound to
a tool. The specification has no notion of a surface a host shows on its own, without a tool call.
The Workbench needs exactly that for a project server: a place to open, before any tool has been
called.

## Decision

A convention inside the specification's own metadata: an App resource (`ui://…`,
`text/html;profile=mcp-app`) carrying the vendor marker `_meta["swem/home"]: true`. The Workbench
shows a space for every declared server that lists one - a project's server, one declared by
hand, one installed from the Store - and opening the space reads the App and mounts it, with no
tool call: the App reads its server through the same relay every App call takes, as a project's
App already does. The host keeps no registry of spaces: the marker is read off discovery, each
server dialled once per run and kept.

## Consequences

The Project space stops being the host's page and becomes the project server's App. Any server
with a home App gets the same treatment. The convention disappears when the ecosystem standardises
app-only hosts.
