# ExecPlan 017 — The harness built into a product

**Status:** completed (2026-09-29). Roadmap S4. ADR-0003, ADR-0011.

## Outcome

Somebody who makes a product of their own builds the harness into it in a page of code. Their
server answers; the Workbench is drawn under an address of theirs, called by their product's
name, for each of their people apart. Who a person is, the product says; what servers an
agent of theirs is handed, the product says; where what a person gave is kept, the product
says.

## Acceptance scenario

**Given** the example product in this tree, which is not the Cycle: a server of its own with
two people, each behind the example's own way in. **When** the first opens their agents,
**then** the Workbench is drawn under that person's address, called by the product's name,
with them as its owner by the name the product gave; they make an agent and talk to it, and
the agent was handed the server the product resolved for it. **When** the second opens
theirs, **then** none of the first's agents or chats are there. **When** somebody the product
does not know asks, nothing of the harness answers.

## Current state (2026-09-29)

- `Product::at(root).assemble()?.serve(bind, sandbox_bind, bundle)` binds listeners of its
  own; nothing answers a request handed to it.
- The router takes requests of one body type, the listener's.
- The page asks the host at addresses from the root (`/api/...`, `/workbench.js`), in about
  a dozen places all its calls pass through.
- The owner is named from the system's user. The name on the page is written in the page.
- A declared server is declared once for every agent, by its name.
- Keys are files under the data root (`keys.rs`, `closed.rs`).
- Nothing in the harness is global to the process but two counters and two fixed lists; one
  process assembles many products, each over a root of its own.

## Relevant product journey

GJ-07.

## Legacy evidence

None here.

## Constraints

- ADR-0002: the harness names no domain and no product that builds it in.
- ADR-0011: who asks is known before anything is answered. Built in, the product says who.
- One way to answer a request: the listener of the harness's own command goes through what
  an embedder calls.
- The example is a product, walked by the gate through its own front door. It is not a
  fixture of a test.

## Plan

1. A request answered without a listener, under a path; the name; the owner as the product
   names them; two people in one process; the example and its walk.
2. A server resolved for an agent when its session opens.
3. The keeper of what a person gave, supplied by the product.
4. What an embedder reads: the README's section and the crate's documentation.

## Progress

- [x] 1 answered without a listener, under a path, by name, for each person
- [x] 2 a server for an agent
- [x] 3 the keeper of secrets
- [x] 4 what an embedder reads: the README's section; the crate's documentation on the
      builder and `product/built_in.rs`

## Discoveries

- The page asked its host at addresses from the root in a dozen places, all through a few
  calls; it now asks beside where it was opened (`base.ts`), and the shell names its script
  beside itself, so nothing else knew the path.
- Where Apps are drawn is an origin with no path in the sandbox page, in the addresses of a
  View and in the checks a message from an App passes. A path per person there is a change
  of its own; in a harness built in Apps are not drawn until it is made.

## Decision log

- 2026-09-29: who asks is told by the product inside the process, as an extension the request
  carries, never as a header a caller could send. A request handed over without it is
  answered by nothing, the page included.
- 2026-09-29: the router takes requests whatever server heard them (`AskedBody`); the
  harness's own listener converts its own. One router, two doors.
- 2026-09-29: a server the product resolves is handed beside a profile's attachments, after
  them, and not twice under one name; the page says which were given by the product.
- 2026-09-29: the keeper of secrets keeps documents by names that are the harness's own
  (`keys/<provider>.json`, `profiles/<id>/secrets.json`, `mcp-servers/<name>.json`). The
  default keeper is the files the harness kept before, so nothing moved for a computer of
  one's own.

## Validation

- `product::a_product_resolves_a_server_for_each_agent`, `product::a_product_keeps_what_a_
  person_gave_where_it_says` (with a keeper in memory: the three names, the words on the
  page, nothing under the data root).
- The gate's walk `a_product_of_ones_own_has_the_harness_built_in`: from outside, nobody the
  product did not let in gets the page or anything behind it; Ada finds her Workbench under
  her path, called Example, with her as its owner; she makes an agent, talks to it, and its
  settings say the product's server was given to it; Bo finds none of hers; Bo at her
  address is refused by the product itself.
- By hand with `curl` against the example: the redirect of a path without its last stroke,
  the page, the script beside it, the door's standing with the product's name.

## Outcome and remaining gaps

A product of one's own builds the harness in. Left: Apps drawn in a harness built in for
several people; the built-in machine and keepers hidden or shown by the product, which has
nothing to hide until there are machines that are not this one (S2); the page's own sentences
that say SWEM; a chat opened from outside with words in it, which a product does through the
routes it hands over.
