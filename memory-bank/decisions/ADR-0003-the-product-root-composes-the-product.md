# ADR-0003 — The product root composes the product; components stay independently usable

**Status:** accepted.

## Context

The harness can run alone, a project server can run alone, and a stock MCP client can use either.
That independence is a capability. It must not become an onboarding requirement: a person who
starts the product should not have to compose those pieces by hand.

## Decision

One product root (`swem-cli`) assembles the harness into what a person runs — the data root, the
profiles, the sessions, the Store with the catalog the distribution ships, the environments — and
the same assembly is available to any embedding application through the crate's product builder.
Every component remains usable on its own, with the product root as the only place that knows
how they fit.

## Consequences

`swem` with no arguments starts a complete Workbench. An application embeds the harness with the
builder instead of copying the product root. Headless and explicitly wired modes remain escape
hatches, not the normal path.
