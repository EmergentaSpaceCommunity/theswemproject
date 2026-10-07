# ADR-0019 — Hosts of one person trust each other by key

**Status:** proposed (2026-10-07). Nothing of it is built; no crate is linked for it.

## Context

A person runs SWEM in more than one place: a laptop, a server, a machine in a cloud. Today
each is a Workbench of its own with its own agents, chats, packages and journal, and the page
of one knows nothing of another. ADR-0011 admits a page to one host; ADR-0015 left peer
identity by key for later.

## Decision

- **A host is any installation of SWEM**, with its own agents, chats, servers, packages and
  journal. Nothing is shared by default: an agent sees the Apps, servers and environments of
  its own host, and isolation is set at the host. What a plugin requires is installed on each
  host that runs it; the Store plans per host (ADR-0018).
- **One key, one person.** Hosts of one person trust each other by that key: a page signed in
  to any of them sees all of them. "Add a host" takes a code said once, or a package of a
  machine provider that brings a host up in a cloud (ROADMAP, kinds of machine from the Store).
- **Page to host is HTTPS with a passkey**, as ADR-0011 has it, unchanged.
- **Host to host is iroh**: identity is the key, NAT is crossed without a server of ours, the
  relays of n0 by default and one's own when wanted. Without it a host behind a home router is
  not reliably reachable by another. Nothing SWEM-specific goes on the wire beyond MCP and ACP
  carried over it.
- **A host that sleeps is woken by a package**: a keeper at the host's side knocks (ADR-0015),
  not the core.

## Consequences

When built: the harness links iroh, under Apache-2.0/MIT; tests and the gate never reach a
public relay. The limits of the public relays are not measured. Chats across hosts are not in
this decision. Until built, the Workbench serves one host, as today.
