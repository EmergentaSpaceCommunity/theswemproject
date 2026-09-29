# ADR-0009 - AGPL, a commercial licence by permission, and a CLA

**Status:** accepted. Restates for this repository what was decided for SWEM as a whole before
the harness and the Cycle had repositories of their own.

## Context

SWEM is open: anyone may read it, run it, change it and send changes. What the copyright holder
does not give away is the right to build it into a product that keeps its own source closed.
That needs permission.

When this repository was made, its licence statement kept the AGPL and lost the rest: it did not
say that a commercial licence exists, and it said that contributions are taken under the AGPL
alone. A contribution taken that way can be licensed under the AGPL and nothing else, by anyone,
the copyright holder included; the first one merged would have ended the commercial offer for
the whole of SWEM.

## Decision

- SWEM is **AGPL-3.0-or-later**. Built into another program, it makes that program a work the
  AGPL covers: it is conveyed with its source under the AGPL, or not at all.
- A **commercial licence** is granted by the copyright holder, by permission. It is the only
  way to build SWEM into a product that is not offered under the AGPL.
- Contributions are taken under a **Contributor License Agreement**, not under the AGPL alone
  and not under a certificate of origin. The contributor keeps their copyright; the copyright
  holder is granted the right to license the contribution under other terms as well.
- Until the agreement is adopted - reviewed, the copyright holder named, the way of agreeing
  chosen - **no outside contribution is merged**.
- What SWEM installs and talks to - agents, MCP servers, skills, catalogs - is not linked into
  it and is licensed by its author as they like.

## Consequences

The copyright holder builds SWEM into products of their own under any terms, because every line
is theirs or is licensed to them for that. Anybody else builds it into an open product under
the AGPL, or asks.

The AGPL does not forbid commercial use; it forbids keeping the source of the whole closed. A
product that is sold and is itself offered under the AGPL needs no permission. If that is ever
to need permission too, the licence is another one, and no longer an open-source one.

`NOTICE` and `CLA.md` carry a placeholder for the copyright holder's legal name.
