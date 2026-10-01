# Which licence covers what

SWEM is **AGPL-3.0-or-later** (`LICENSE`), for everyone, with the SWEM Extension Exception
(`LICENSE-EXCEPTION.md`) for what extends it. Everything in this repository - the crates, the
page, the scripts and the documentation - is under it, with one exception: `crates/swem-sdk`,
the vocabulary of the Store that a host, a taker, a catalog tool or a package is written against,
is **Apache-2.0** (its own `LICENSE`). A licence under any other terms for SWEM itself is granted
only by the author, wathipol, and only by written permission; see "Commercial use" below.

**Which way the licence can move.** Every version released under the AGPL stays under it; that
is the law of the licence, not a promise. The promise is this: the author will not move SWEM's
open version to a licence that is not approved by the OSI. If the licence ever changes, it
changes towards more freedom, not less.

The authority for this is
[ADR-0009](docs/decisions/ADR-0009-agpl-a-commercial-licence-by-permission-and-a-cla.md).

## What this means for you

**Using or modifying SWEM, including running it as a network service.** AGPL-3.0-or-later. If you
run a modified SWEM where other people can use it over a network, section 13 asks you to offer
those users the corresponding source of your modified version.

**Building SWEM into a product of your own.** SWEM's crates linked into an application make that
application a work the AGPL covers as a whole: it is conveyed under the AGPL, with its source, or
not at all. A product that is not offered under the AGPL needs the author's permission.

**Commercial use.** The AGPL does not forbid commercial use; it forbids keeping the source of the
whole closed. A separate commercial licence is available from the author alone, and it is the only
way to build SWEM into a product that keeps its source closed. Nobody else - no contributor, no
distributor - can grant it. Ask at hi@emergenta.space; the AGPL is the default, not the only offer.

**Writing an agent, an MCP server, a skill, a catalog, a package for the Cycle, or anything
against an SDK.** Yours, under any licence you like - proprietary, MIT, your own. Most of it is
not linked into SWEM at all: an agent speaks ACP over its own process, a server speaks MCP over
its own process, a skill and a catalog are documents. Where the line could be argued - a package
the Cycle loads, a taker or a host written against `swem-sdk` - `LICENSE-EXCEPTION.md` settles
it: a work that reaches SWEM only through its extension interfaces is not a covered work, by the
author's explicit additional permission under section 7 of the AGPL, and `swem-sdk` itself is
Apache-2.0. The Store lists what people
publish; SWEM claims nothing over it.

**The Cycle.** SWEM's project model lives in its own repository under its own licence statement.

**Attribution.** The licence requires that copyright and licence notices survive in copies and
derivative works, and that modified files carry prominent notices saying that you changed them and
when (section 5). `NOTICE` lists what must travel with a copy. Stripping authorship out of a copy
is a licence violation, not a matter of etiquette.

## Contributions

Contributions are welcome, and are taken under the Contributor License Agreement in `CLA.md`.
You keep the copyright in what you wrote. The agreement grants the author - and the author alone -
the right to license your contribution, together with the rest of SWEM, under other terms as well
as the AGPL. That right is what keeps the commercial offer above possible: without it, the first
merged contribution would leave SWEM licensable under the AGPL alone, for good. Contributing gives
you no right to license SWEM under other terms; it gives you what it gives everyone, the AGPL.
