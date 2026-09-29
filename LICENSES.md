# Which licence covers what

SWEM is **AGPL-3.0-or-later** (`LICENSE`). Everything in this repository - the crates, the page,
the scripts and the documentation - is under it. A commercial licence is granted by the copyright
holder; see "Commercial use" below.

The authority for this is
[ADR-0009](memory-bank/decisions/ADR-0009-agpl-a-commercial-licence-by-permission-and-a-cla.md).

## What this means for you

**Using or modifying SWEM, including running it as a network service.** AGPL-3.0-or-later. If you
run a modified SWEM where other people can use it over a network, section 13 asks you to offer
those users the corresponding source of your modified version.

**Building SWEM into a product of your own.** SWEM's crates linked into an application make that
application a work the AGPL covers as a whole: it is conveyed under the AGPL, with its source, or
not at all. A product that is not offered under the AGPL needs the copyright holder's permission.

**Commercial use.** A separate commercial licence is available from the copyright holder, and it
is the only way to build SWEM into a product that keeps its source closed. Ask; the AGPL is the
default, not the only offer.

**Writing an agent, an MCP server, a skill or a catalog SWEM installs.** Nothing of it is linked
into SWEM: an agent speaks ACP over its own process, a server speaks MCP over its own process, a
skill is a document, a catalog is a document you publish. The AGPL does not reach them; license
them however you like.

**The Cycle.** SWEM's project model lives in its own repository under its own licence statement.

**Attribution.** The licence requires that copyright and licence notices survive in copies and
derivative works, and that modified files carry prominent notices saying that you changed them and
when (section 5). `NOTICE` lists what must travel with a copy. Stripping authorship out of a copy
is a licence violation, not a matter of etiquette.

## Contributions

Contributions are welcome, and are taken under the Contributor License Agreement in `CLA.md`.
You keep the copyright in what you wrote. The agreement grants the copyright holder the right to
license your contribution, together with the rest of SWEM, under other terms as well as the AGPL.
That right is what keeps the commercial offer above possible: without it, a contribution could be
licensed under the AGPL alone, and SWEM as a whole could no longer be licensed any other way.

`CLA.md` is a **draft**. It has not been reviewed by a lawyer and is not in force. Until it is
adopted, no outside contribution is merged; a contribution sent before then waits.
