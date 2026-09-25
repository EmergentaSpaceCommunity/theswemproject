# Security

## Reporting a vulnerability

Please do not open a public issue for a vulnerability. Report it privately through the repository's
security advisories, and give it a way to be reproduced: what you ran, what you expected and what
happened. You will get an acknowledgement, and a fix or an explanation of why the behaviour is
intended. If you would like to be credited in the release that carries the fix, say so.

## What is in scope

SWEM runs agents and reaches outside itself on purpose, and some of that is by design rather than
by defect. Reports that concern the boundaries below are in scope.

**The install road.** Anything the product fetches onto a machine is planned, shown, confirmed by
its exact plan id, checked against the digest the plan named, and receipted. A path that installs
without that road, that applies a plan other than the one confirmed, that follows an archive entry
outside its own directory, or that runs a package's scripts during an npm install, is a
vulnerability.

**The agent's boundary.** An agent's file callbacks are held to its profile's workspace and its
terminals run in its profile's environment; a container environment holds the workspace and
nothing else of the machine. A path that reaches outside those, or that runs a command the
person's permission choice says to ask about without asking, is a vulnerability.

**Secrets.** A profile's secrets live in its vault and are injected at launch; a declaration's
environment values are never listed back. A secret value appearing in a record, a session event, a
log line, a page or an error message is a vulnerability.

**The Workbench and the App sandbox.** The Workbench's HTTP surface answers only its own page,
carrying the per-run secret; an MCP App is rendered in a sandboxed origin under a content security
policy. A page on another site working the Workbench, an App reaching the host's own surfaces or
another App's, is a vulnerability.

## What is not in scope

- **An agent doing what it was asked to do.** SWEM hands work to agents, including agents that run
  commands. That is the product.
- **A catalog entry doing what it says.** Installing is a decision a person makes against a plan
  that says what is fetched from where. Deceptive contents of a third party's server or skill are
  that party's problem - unless SWEM presented them inaccurately, which is in scope.
- **Third-party dependencies**, unless SWEM's use of one is what makes it exploitable. Report those
  upstream; tell us too if SWEM is affected.
- **Findings from a scanner with no demonstrated impact.**

## Supported versions

Nothing is released yet. This file describes the intended boundaries so they can be tested against;
it will state a support policy when there is a release to support.
