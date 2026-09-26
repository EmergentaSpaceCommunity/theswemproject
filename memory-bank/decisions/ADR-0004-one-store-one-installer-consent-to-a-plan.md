# ADR-0004 — One store, one installer, consent to a plan by its id

**Status:** accepted.

## Context

Everything a person adds to SWEM is fetched onto their machine: an agent from the ACP registry, an
MCP server or a skill from a catalog, a tool a package declares. Three install roads with three
receipts were three places to look and three ways to be wrong.

## Decision

One installer: a plan the person reads and consents to by its exact id, a staged fetch checked
against the digest the plan named, one rename into place, one receipt schema, one root under the
data root (`installed/<kind>/<id>/<version>/`). One Store over the indexes the product reads: the
ACP registry, and catalogs a person adds by address. Indexes are consumed, never hosted; SWEM runs
no registry.

## Consequences

A plan that moved since it was shown is refused. A supported distribution is an npm package run
through node with scripts disabled, a digest-verified native archive, or a digest-verified archive
holding a skill; nothing executes a remote manifest as shell.
