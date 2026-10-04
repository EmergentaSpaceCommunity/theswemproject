# ADR-0013 — A channel is a package with a fixed shape, and the harness names no messenger

**Status:** accepted (2026-10-02).

## Context

People reach their agents from the Workbench's page and from an editor. A messenger - Telegram
first - is the third way in, and the first that brings people the harness does not know: a bot
is written to by whoever finds it. Messengers differ in everything but the shape of what they
carry: somebody wrote in a chat, with files, in reply to something; the bot answers, possibly as
it writes; a question with options; a file each way.

Two things were settled already: a package of any kind is taken by a host that registers a
`Taker` and checks a candidate against a `Shape` (ADR-0012); the harness's MCP client receives
no notifications from a server it hosts, so a server cannot push to the harness.

## Decision

- A channel is a package of kind `swem/channel@1`: an MCP server program the harness starts and
  keeps, and talks to through the tools of one shape (`swem_sdk::channel`): `look`, `pull`,
  `send`, `stream_begin/update/end`, `ask`, `send_file`, `fetch_file`. The harness checks the
  shape when the package is installed and before the channel is started; a program short of it
  is refused in words.
- The harness calls, the channel answers. What arrived on the messenger's side comes through
  `pull`, which waits a while and answers what it has. Nothing of the messenger's transport -
  polling, webhooks, sockets - reaches the harness; the channel owns it.
- The harness names no messenger. The Telegram channel is a package, shipped with the product
  as the first one, and anything else is written to the same shape under any licence.
- A messenger's person is a participant here: the owner, bound by a pairing code they say to
  the bot once; anybody else a guest, made on first contact and trusted as a guest in the
  envelope, let in by the channel's policy (nobody, by invitation, anyone). A messenger's chat is
  a chat here, bound once and kept; a message carries the messenger's reference, so one thing
  delivered twice is said once.
- What an agent says in a bound chat is carried back as it is written: a stream begun on the
  first chunk, updated with what was said so far, ended with the message; a turn that stopped or
  failed ends the stream with what there was. A question with options is asked on the
  messenger's side and answered from there.
- Anything arriving through a channel is a reason to look at the clock, so a harness woken by a
  message says what was due meanwhile.

## Consequences

The harness grows one module (`workbench_shell/channels.rs`), two ledger tables (`identities`,
`channel_chats`) and a taker; it grows no knowledge of Telegram. A channel package's key is
kept as every key is (`channel-<id>`). A channel that needs an address from outside - a webhook,
a Mini App - asks the harness for a door, which the harness has only when served at an address.

**The door** (added 2026-10-02). A channel that answers `receive` beyond the shape can be reached
at `POST /api/channels/<id>/receive`, open to anybody while the Workbench is served at an
address, as the knock for time is. The harness hands what arrived to the channel as it came -
headers and body - and verifies nothing: the channel knows its messenger's secret (Telegram's
secret token, chosen by the channel and never seen by the harness or the page) and refuses what
does not carry it. The person chooses on the page whether the bot is asked or delivered to; the
channel is started again with the door in its settings and tells the messenger. A channel
without `receive` is asked, wherever the Workbench is.

**The page inside the messenger** (added 2026-10-03). A channel that answers `verify_app
{init_data}` with the person the messenger signed lets the Workbench serve a page of its own
inside the messenger: `/channels/<id>/app`, with `/api/channels/<id>/app*` open to any origin
while served, each request carrying the messenger's signed data in a header and nothing of a
session. The harness maps the person to the identity bound through the channel and to the chat
they have through the bot; the channel alone checks the signature, with the token only it
holds. The bot offers the page with a `web_app` button, which the channel's `send` takes as an
optional `app` beyond the shape.

**Declared, and exchanged once** (added 2026-10-04, with ADR-0016). `verify_app` and the
`app {label, url}` button on `send` are named in the SDK (`channel::VERIFY_APP`,
`channel::AppButton`) as what a channel answers beyond the shape when it has a page inside the
messenger, as `receive` is; `take_back {chat, reference}` beside them, for a button to a page
whose address closed; and `look` names in `app_data_fragment` where the messenger puts the
signed data in the page's address. The signed data is no longer carried per call: the page
exchanges it once at `POST /api/access/by-channel/<id>` for a session of the Workbench's own
door, and who asks from then on is a participant - the owner within the bot's agent, or a
guest within their chats (ADR-0016). The paragraph above describes what stood until then.
