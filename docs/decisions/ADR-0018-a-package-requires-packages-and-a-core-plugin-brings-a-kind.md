# ADR-0018 — A package requires packages, and a core plugin brings a kind

**Status:** accepted (2026-10-07). Extends ADR-0012.

## Context

The Store planned a package with its direct requirements and nothing further: what a
requirement itself required was not planned, the id a person confirmed covered the entry
alone, a receipt said nothing of what the plan had required, and Remove took a package away
whatever still needed it. That is enough for an agent and its server; it is not enough for an
ecosystem where a plugin builds on a plugin.

The case that made the gap visible is a product built on the harness that takes a capability
every one of its plugins needs - an inference runtime, say - out of its own binary and into an
open package, so that its other plugins require that one and build around it. Nothing in it is
SWEM's: SWEM's part is that the Store resolves, installs, records and protects a chain of
packages, and that a package may introduce a kind of resource of its own for other packages to
be of. The Cycle is built the same way, as its own track.

## Decision

Three rules in `swem-store` and one in the host, with no new mechanism:

- **A plan is the closure.** Planning an entry walks what it requires and what those require,
  from the same indexes. Each package is planned once, before what requires it; one that is
  installed or bundled at a version that satisfies is left as it is. A circle is refused naming
  its path; two requirements that do not meet on a version are refused naming both. The id the
  person confirms is a digest of the entry's plan with every plan in its closure, so a
  requirement that moved since the plan was shown moves the id and the install is refused.
  Each receipt keeps the entry's own plan id, so an installed package still matches its entry.
- **A receipt records `requires`.** Every installer writes what the plan required; a receipt
  written before this reads as requiring nothing.
- **What is required is not removed.** Removal asks the receipts and the bundled entries who
  requires the package and refuses naming them; the page shows "required by" where the Remove
  button would be. What requires it goes first. A requirement a plan brought in remembers the
  entry it came for in its receipt; once nothing requires it, the page says it is there for
  nothing, and the person decides.
- **What is required is handed in the environment.** Every package process the host starts - a
  tunnel, a channel - finds each tool on hand under `SWEM_TOOL_<ID>` (ADR-0015's variable, from
  one place in the host). A package finds what it requires as a program finds a database under
  `DATABASE_URL`, without knowing SWEM. A required *service* - a server package's address and a
  token for it - is not handed yet; the first package that requires one decides its shape.

A **core plugin** is nothing new to the Store: a server package whose entry says `takes`
introduces a kind of its own (ADR-0012), and other packages are of that kind and `require`
the core. The kind's name is the package's business; the harness names no kind but its own
four and links no package. The binary reads packages and carries none.

## Consequences

Consent stays one plan by id, now over everything that will be fetched. The page shows the
closure as it did (`also`), with nothing to learn. What is installed and relied on stays: a plan
that would replace an installed package with a version some installed receipt does not allow
is refused naming that receipt, so an upgrade never breaks what required the old version.
Trust between hosts, and what a plugin may reach on another host, is
ADR-0019.
