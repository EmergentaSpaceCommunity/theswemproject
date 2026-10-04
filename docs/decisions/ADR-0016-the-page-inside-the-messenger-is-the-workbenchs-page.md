# ADR-0016 — The page inside the messenger is the Workbench's page, and who asks is a participant

**Status:** accepted (2026-10-04). Completes ADR-0011's principal; revises ADR-0015's gate;
retires the page of ADR-0013's addendum of 2026-10-03.

## Context

The first page a bot opened inside the messenger was a second interface: a static page of its
own (`web/mini-app`) with its own chat, file list, upload and form drawing, behind an API of
its own (`/api/channels/<id>/app/*`) that let itself in by the messenger's signature on every
call - beside the door, not through it - and answered any origin. A hosted copy on the
author's zone was the default, because a page on another origin could not hold the Workbench's
session. The owner saw what it was: not the product's design, a stub of a chat, duplication,
files to read only. The survey found what made it so: the principal of ADR-0011 carried only
by what somebody came and never who, so every handler acted as the owner, and per-person
rights could exist only outside the door.

## Decision

- **Who asks is a participant.** `Principal { participant, by }`. The owner by every old way;
  a participant the bot has met by a new one, `CameBy::Messenger`. What a principal may is one
  rule, `Scope`: everything, or - for somebody from a messenger - what is theirs within the
  bot's agent: the owner reaches that agent's chats, questions and files; a guest their own
  chats. The door decides routes by the way in, as it does for a knock token; handlers decide
  rows by the scope. `May` stays what a token holds.
- **The messenger's signature is exchanged once for a session.** `POST
  /api/access/by-channel/<id>`: the channel package says who (`verify_app`, now declared in
  the SDK beside `receive`), the ledger says who that is here, and the door sets a cookie the
  page cannot read, for a day, kept in memory beside the tunnel. The same signed data is the
  same session; a forged one is nobody; age and replay stay the channel's (ADR-0015). Closing
  the tunnel puts everybody out. Nothing is carried per call any more; the page asks as the
  page always asks.
- **The gate serves the page, and lets in a messenger session and nothing else.** One router,
  one `let_in`, told which listener a request came through; on the gate the run's secret, a
  passkey session and a token are not principals, and of what is open to anybody only the
  exchange and the door's standing are - not the passkey ceremonies, not a channel's webhook
  door. What ADR-0015 kept out stays out. A messenger session is what keeps a tunnel open.
- **One page.** `web/apps-host`, drawn for a messenger principal: no rail, the bot's agent,
  the tabs the scope allows, at a phone's width, opened on what the bot pointed at (`?open=`,
  `?channel=`, `?signed=` naming where the messenger puts its data - the page names no
  messenger). What the phone needed the page gains for everybody: a viewport, a narrow layout.
  The bot's buttons carry the Workbench's own address; when a tunnel closes the bot takes them
  back (`take_back`, declared beside `verify_app`) and says to send `/app` again. The hosted
  front, `app_at`, the product's hosted default and `web/mini-app` are gone.
- **What a messenger session may is bounded by what the bot's token already commands.** Whoever
  holds the token can make the agent say and do anything through the messenger; the session
  adds no power beyond that agent. Widenings are decided one at a time: reading the agent's
  files came with the session; editing them with the Files tab; Apps' tool calls with the Apps
  tab.

## Consequences

- `access.rs` gains `participant` and `Scope`; `door.rs` the exchange, the messenger sessions,
  `Through` and `open_to_a_messenger`; `stream.rs` takes who asks and filters the stream by
  scope; the gate is `route_shell` with the marker. Every route a messenger session may reach
  is named in one list, with a test beside it.
- `verify_app`, `take_back`, the `app` button and `Bot.app_data_fragment` are the SDK's;
  the fixture channel answers them, so the gate walks need no messenger.
- Threading who asks through every other handler is debt, recorded in `ROADMAP.md`; until then
  the door refuses a messenger session the rest.
- Apps the agent answers with reach the messenger as a button and the page's Apps tab; through
  a tunnel the sandbox stands at a second address of the same package, started once more, and
  `app_open` answers the sandbox of the listener the page came through.

## Links

ADR-0011, ADR-0013, ADR-0015; `docs/channels.md`.
