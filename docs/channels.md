# Reaching an agent from a messenger

A channel is a door into chats from outside the Workbench. Telegram is the first one, and it
comes with the product: `swem-channel-telegram` beside the `swem` binary. Nothing of Telegram is
in the harness; a channel for another messenger is a package of the same kind (ADR-0013).

## Adding a bot

1. Make a bot in the messenger (on Telegram, with BotFather) and keep the token it gives you.
2. On the Workbench, Providers → Channels → **Add a bot**: a name, the messenger, the token,
   which of your agents answers there, and who else may write to it. The token is kept with your
   keys and never shown again.
3. The page shows the bot's username and a six-digit code. Write the code to the bot once. From
   then on the bot knows the messenger account that said it is you.

What you write to the bot is said in a chat with that agent, marked on the page as said from a
messenger; the agent's answer is drafted in the messenger while it is written and sent when it
is done, with the formatting the messenger shows (bold, code, lists, quotes, links; a table as
code). `/stop` stops the turn. A message longer than the messenger allows arrives in pieces.

## Who else may write to the bot

- **Nobody** (the default): the bot answers you only; anybody else gets one line saying so.
- **By invitation**: somebody who writes to the bot alone is told to wait. The bot's row on the
  Channels page says who waits; **Let in** puts them into the chat you have with the bot. From
  then on what they write to the bot is said there, with their name, you see it on your side,
  and the agent's answers reach them where they wrote from. A chat of several offers the same
  under **Add someone**. Taking them out of the chat ends it.
- **Anyone**: whoever writes gets a chat of their own with the bot's agent.

A guest is a participant like anybody else: what they said is theirs on the page, and the agent
is told who is speaking and that they are a guest, not you.

## Groups and topics

A group the bot is in is one chat of several here, named after the group. The agent takes a
turn only when the bot is spoken to: named (`@thebot`) or replied to; a command is always to
the bot. What else is said in the group by somebody already in the chat is written there for
the record, with no turn - so the agent knows the room when it is next spoken to - and what a
stranger says without speaking to the bot is left alone. Telegram decides what the bot sees at
all: with privacy mode on (the default for a bot) only what names it, replies to it or is a
command; an admin bot, or one with privacy mode off, sees everything. A forum topic is a chat of
its own, named after the group and the topic's number; rename it on the page.

## Questions

When the agent asks before doing something, the question arrives in the messenger as buttons
with the agent's own options; the one pressed is the answer, as if it had been pressed on the
page. What only the page can answer - a form, a link to open - is said to be so.

## Files

A file sent to the bot within the messenger's limit (20 MB on Telegram) comes with the message,
is shown on it, and lands in the agent's inbox, the way a file handed over on the page does. What
the agent puts in its outbox during a turn goes back through the bot when the turn ends, within
the messenger's limit (50 MB); beyond it the chat says so.

## The Bot API address

Telegram's own API is the default. A local Bot API server (which lifts the file limits) or any
other address is typed under **More** when the bot is added. The product's own gate runs the
channel against a Bot API that is a fixture at such an address; nothing in the tree talks to
Telegram.

## Asked, or delivered to

A channel asks the messenger for what is new, which works on a laptop behind any network. A
Workbench served at an address (`docs/serving.md`) has a door the messenger can deliver to
instead: **Have it delivered** on the bot's row tells the messenger the door and a secret the
two of them share; a delivery without the secret is refused by the channel, and the Workbench
verifies nothing it does not understand. **Ask instead** switches back. On a Workbench not served
at an address the row says why there is no door. Telegram delivers over HTTPS only, so the
address needs a certificate; with the Workbench served over plain HTTP behind your own proxy, the
proxy's address is what the messenger sees.

## The Workbench inside the messenger

A bot of a Workbench served at an address offers a page of the Workbench's inside the messenger
(a Mini App): `/app` to the bot, or whenever a file is too big for the bot to carry. Opened from
the bot, the page knows who you are by the messenger's signature, which the channel checks
against the bot's token - the Workbench never sees the token and verifies nothing itself - and
shows the chat you have with the bot, takes a file of any size to the agent (it lands in the
agent's inbox with your words, as a file handed over on the Workbench's page does), and lists
what the agent put out, to save. Opened by nobody, it says what it is; a signature that is not
the messenger's is refused. The page is one static file the Workbench serves at
`/channels/<id>/app`. To host it anywhere else - GitHub Pages, Cloudflare Pages, any static
host with HTTPS - put the committed `web/mini-app/dist/index.html` there (it is written from
`web/mini-app/index.html` by `node scripts/mini-app.mjs`, and a test keeps the two together),
and give the address under **More** when adding the bot ("Where the page the bot opens inside
the messenger is hosted"); the bot's button then carries this Workbench's address, and the
Workbench answers the page from any origin with the signature alone. The SWEM distribution
names a copy it hosts (`https://swem-telegram.emergenta.space`, put up from
`web/mini-app/wrangler.jsonc`) as what a bot is told when nobody is named - a convenience,
never a requirement: name your own, or leave the product's default out and the Workbench
serves the page itself (ADR-0015). The page loads no script
of the messenger's: it reads the signed data from the address it was opened with. Either way
the Workbench has to be reachable from the messenger: served at an address, or through a
tunnel. Telegram opens a Mini App over HTTPS only.

## A tunnel: an address from outside, for a while

A Workbench on a laptop has no address the outside can reach. A **tunnel** is a package from
the Store that stands at one for a while (ADR-0015). The one that came with the product drives
Cloudflare's quick tunnel: no account, no key - install the `cloudflared` tool from the Store
(it is fetched from Cloudflare's release, checked by digest) and that is all. Then `/app` to
the bot, or **Open a tunnel** under Providers → Channels, opens one; the bot's button points the
page at it. What the tunnel reaches is only the page and its API - never the Workbench itself,
sign-in, or anything a stranger could use. It closes after thirty minutes unused, or with
**Close** on the page; the page inside the messenger then says to send `/app` again. Cloudflare
calls a quick tunnel a thing for testing and development: no uptime promise, a new address each
time - which is what an address for a while is. Webhooks never go through a tunnel; on a laptop
the bot asks the messenger for what is new, which works behind any network.

Any program that answers the tunnel shape (`swem-sdk`, `tunnel`) is a tunnel package: a named
tunnel, another vendor, a peer-to-peer endpoint. A tunnel is refused on a Workbench served at an
address, which has one already.

The bot answers `/status` with how things stand (what the agent is doing, the chat, how
updates arrive), `/stop`, and `/app`; while the agent works, the message being answered wears
👀 and the bot is seen typing. On a Workbench served at an address the page is also the bot's
menu button, one tap away (the channel sets it when it starts); through a tunnel the address
changes, and `/app` is the way. No domain has to be registered with the messenger for this.

## Where the bot lives

A bot belongs to one Workbench: the one that runs its channel and holds its key. A messenger
allows one consumer of a bot's updates at a time, and so does SWEM. To move a bot, remove it
here and add it there with its token; nothing else travels (ADR-0014).

What is walked by the product's gate: direct messages, pairing, a stranger kept out, a group, a
topic, a guest let in, a question answered with a button, a file each way, a delivery at the
door of a served Workbench and why there is none otherwise, the page inside the messenger with a
60 MB file through it. A real messenger is tried by hand.
