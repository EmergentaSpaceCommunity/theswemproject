# ADR-0012 — One Store, many hosts

**Status:** accepted (2026-09-30, roadmap P); amended 2026-10-01: the vocabulary is its own crate.
Extends ADR-0004.

## Context

The Store knew four kinds of package by a closed enum, and everything about each kind - its words,
what it accepted, what happened after install - lived in the harness. A kind it did not know
failed the whole catalog. There was no update and no removal; "newest" was a text sort of
version strings; a server reinstalled lost the values a person had given it; three plan-id
recipes existed.

Two other hosts want the same Store. A server the harness runs - the Cycle's hub - has packages
of its own, installed by its own installer, used without the Workbench, and would be better
listed, versioned and installed from the same page. A product the harness is built into has
kinds of its own - servers with a fixed set of tools it calls with its own rights - which are
not tools for agents and should not be shown as such.

## Decision

The Store is a library, `swem-store`, that knows kinds only by name. A kind is a reverse-DNS
name with a version of the kind (`swem.cycle/package@1`); the four old words stay as they were
on the wire. A **host** registers, per kind it takes, a `Taker`: the words for the kind, what it
accepts, a check of the staged tree before anything is promised, what happens after install and
before removal. The harness registers agents, servers and skills. A declared or installed server
whose catalog entry says `takes` is a taker too: the Store fetches, checks the digest, stages,
asks the server to plan the staged tree, commits, asks it to plan the tree where it lives and to
install by the plan id it answered, and writes its own receipt beside; removal is the server's
`remove`. A product the harness is built into registers takers of its own through the builder.
A kind nobody on an installation takes is listed as for something this product does not have,
and cannot be installed. Nothing in the Store names a product.

Vocabulary borrowed, not invented: `requires` (resolved from the same indexes, planned as a
closure under one consent, refused in words when nobody's index has it), `takes`, `uvx` beside
`npx`, `binary` and `archive`, semver where it parses, `catalog@0.2` with `@0.1` read as a
subset. For servers with a fixed tool set there is one shape checker: a host writes the tools it
calls and their input, starts a candidate once, lists its tools and compares. No server is
trusted to say what it implements.

## Consequences

An update is offered where an index has a newer version; what was installed can be removed
where its taker allows; a server updated keeps its values. A package of a kind for another host
is installed through that host's own road: the Cycle's validation, digest and child retirement
stay the Cycle's (ADR-0011 of the Cycle). The harness's check of a candidate can start a
program from the staged tree; a taker does that on a blocking thread. `rusqlite` is held at
0.32 so the harness links beside `sqlx 0.8` in one workspace. Indexes are still consumed, never
hosted (ADR-0004).

## Amendment (2026-10-01)

The vocabulary - `Kind`, catalogs, plans, receipts, `Taker`, `Through`, shapes - is its own crate,
`swem-sdk`, under Apache-2.0, apart from the Store that implements it. Without that a host, a
taker or a tool for catalogs could only be written by linking an AGPL crate, and the Store could
not be extended by anyone who does not accept the AGPL for their own code; with it, what is
written against the vocabulary is its author's under any licence (`LICENSE-EXCEPTION.md`).
