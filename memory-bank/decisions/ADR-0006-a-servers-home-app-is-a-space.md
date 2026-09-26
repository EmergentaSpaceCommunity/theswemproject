# ADR-0006 — A server that declares a home App gets a space of its own

**Status:** proposed (lands with roadmap item C4).

## Context

MCP Apps let a server ship its own surface, rendered by the host in a sandboxed origin and bound to
a tool. The specification has no notion of a surface a host shows on its own, without a tool call.
The Workbench needs exactly that for a project server: a place to open, before any tool has been
called.

## Decision

A convention inside the specification's own metadata: a tool with `_meta.ui` naming a `ui://`
resource, app-only visibility, and the vendor marker `_meta["swem/home"]: true`. The Workbench
shows a space for every attached server that declares one; opening the space reads the App and
calls that tool through the same relay every App call takes, handing the result to the View. The
host keeps no registry of spaces: the marker is read off discovery, once per run, cached.

## Consequences

The Project space stops being the host's page and becomes the project server's App. Any server
with a home App gets the same treatment. The convention disappears when the ecosystem standardises
app-only hosts.
