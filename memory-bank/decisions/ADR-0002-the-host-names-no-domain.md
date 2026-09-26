# ADR-0002 — The host names no domain

**Status:** accepted.

## Context

SWEM's Workbench hosts agents and the servers they reach. What a person makes with them — a piece
of music, a service — is served by the Cycle and its packages over MCP. A host that knew a domain
word would have to change whenever a domain did, and could not be embedded in a product that
makes something else.

## Decision

The harness crate names no domain and links no domain crate. `cargo tree -p swem-host -e normal`
names no other SWEM crate, and a structural test (`tests/genericity.rs`) scans the crate's source
for domain vocabulary and fails when one appears. What a project is, is read from the server that
serves it, at the revision it names, through resources and tools the server declares.

## Consequences

The Project space of the Workbench is a surface the project server provides (see ADR-0006), not a
page the host writes. A new domain reaches the Workbench without a change to it.
