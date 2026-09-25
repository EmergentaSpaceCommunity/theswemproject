# Which licence covers what

SWEM is **AGPL-3.0-or-later** (`LICENSE`). Everything in this repository - the crates, the page,
the scripts and the documentation - is under it.

## What this means for you

**Using or modifying SWEM, including running it as a network service.** AGPL-3.0-or-later. If you
run a modified SWEM where other people can use it over a network, section 13 asks you to offer
those users the corresponding source of your modified version.

**Writing an agent, an MCP server, a skill or a catalog SWEM installs.** Nothing of it is linked
into SWEM: an agent speaks ACP over its own process, a server speaks MCP over its own process, a
skill is a document, a catalog is a document you publish. The AGPL does not reach them; license
them however you like.

**The Cycle.** SWEM's project model lives in its own repository under its own licence statement.

**Attribution.** The licence requires that copyright and licence notices survive in copies and
derivative works, and that modified files carry prominent notices saying that you changed them and
when (section 5). `NOTICE` lists what must travel with a copy.

## Contributions

A contribution policy is not yet in force. Until one is published here, contributions are taken
under the same licence as the repository (AGPL-3.0-or-later), and a change that a contributor would
not license that way should not be sent.
