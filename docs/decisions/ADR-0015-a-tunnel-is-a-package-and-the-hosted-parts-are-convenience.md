# ADR-0015 — A tunnel is a package of a fixed shape, and the hosted parts are convenience

**Status:** accepted (2026-10-03).

## Context

A Workbench on a laptop has no address the outside can reach. Most of what a channel does
needs none: the Telegram channel asks the messenger for what is new (ADR-0013). Two things do
need one: the page a bot opens inside the messenger (its API is called from the person's phone)
and webhooks. A served Workbench (ADR-0011) has an address; a laptop behind NAT does not, and a
person should not need a server, an account or a key of any vendor's to open that page from a
laptop.

What was weighed: a tunnel vendor's program driven by the harness (vendor knowledge in the
harness; refused); a relay of SWEM's through a hosted Worker (every byte of every user's files
through a public service; refused); peer identity by key (iroh) with a relay for browsers (right
later, for agents reaching agents; the browser side is relay-only and would put wasm in a page
that needs none today); Cloudflare's quick tunnel - no account, no key, an address that changes
every run, "for testing and development" by Cloudflare's own words, which is exactly what an
address for a while is.

## Decision

- **A tunnel is a package of kind `swem/tunnel@1`**, the twin of a channel: an MCP server
  program the harness starts and keeps, answering `open {url} -> {origin}`, `close` and `look`
  (`swem_sdk::tunnel`), checked against the shape at install and at start as channels are. The
  harness names no tunnel vendor. The first package, `swem-tunnel-cloudflare`, ships beside the
  binary and drives Cloudflare's quick tunnel; `cloudflared` itself is a **tool** (`swem/tool@1`)
  the Store installs - a bare executable is now a binary distribution - and its path is handed
  to the package as `SWEM_TOOL_CLOUDFLARED` (the tools on hand, each under
  `swem_sdk::tunnel::tool_variable`). A named tunnel, `ngrok`, Tailscale Funnel or a peer-to-peer
  endpoint are other packages of the same shape.
- **A tunnel opens the app and nothing else.** The harness binds a second loopback listener for
  the tunnel, the **gate** (`door::route_the_gate`), whose router is an allow-list: the page a bot
  opens inside the messenger and its API, which authorise by the messenger's signature on every
  call; everything else is 404 with a body that names nothing. The gate never reaches `let_in`,
  the Workbench page, sign-in, the stream, the run's secret, the knock, or a channel's webhook
  door: an address that changes every half hour is no place for a standing promise to a
  messenger. A Workbench served at an address, or behind a proxy, refuses to open a tunnel: it
  has an origin, and two would be two truths.
- **The tunnel is for a while and in memory.** It opens when `/app` is sent to a bot or from
  the Channels page, closes after thirty minutes unused (each request the channel verified
  keeps it open) or when the person closes it, and its origin is never written to a document,
  the ledger or a log. `app_origin()` is the one question channels ask (the served address, else
  the tunnel); the webhook door keeps asking `served_origin()`.
- **Polling on a laptop; webhooks and deliveries where there is an address.** The harness never
  takes other people's messenger updates on a hosted service.
- **The hosted parts are convenience, never protocol.** The page a bot opens inside the
  messenger is one static file (`web/mini-app/dist/index.html`), hostable anywhere; the
  product may name a hosted copy as the default, the harness has none and serves the page
  itself. A knock service for Workbenches that sleep (a keeper kind, S5) will be a public
  instance with quotas and a one-command self-host. No account anywhere; identity is a key; no
  user data passes through anything SWEM hosts.

Glossary, since three doors were one word: **the door** is sign-in (`door.rs`, who may come
in); **a channel's door** is where the messenger delivers (`Reach::Door`, `receive`); **the
gate** is the listener a tunnel points at.

## Consequences

- The Store gains two kinds the harness takes (`Tunnels`, `Tools`) and one distribution form
  (a bare executable); the shipped catalog names `cloudflared` by release with its digests.
- CI and the gate never download a tunnel vendor: the fixture tunnel stands at a loopback
  address of its own and passes everything through to the gate whole - holding each answer
  back until it ends, as a vendor's edge was measured to (ADR-0016), so a walk "through the
  tunnel" meets what the real road does.
- The age and replay of the messenger's signed data are the channel package's business
  (ADR-0013); the gate adds no check of its own and none is needed for an app that is read by
  signature per call.
- **Revised by ADR-0016 (2026-10-04).** The gate now serves the Workbench's own page and lets
  in somebody who came through a messenger, by a session the door minted from the messenger's
  signature - and nobody else; what this decision kept out stays out. The hosted copy of a
  separate page is retired: it had become a second interface, and a page on another origin
  could not hold the Workbench's session. "The hosted parts are convenience, never protocol"
  stands for what remains hosted (the knock service, when it exists).

## Links

- ADR-0011, ADR-0012, ADR-0013, ADR-0014; `docs/channels.md`.
