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

## The people a bot meets

Everybody the bot meets - whoever writes to it alone, whoever speaks in a group it is in - is a
person here: a guest, named as the messenger names them, in the chat they were met in. Being
met is not being allowed: the agent takes a turn only on what the owner says and on what those
the owner **allowed** say. The bot's row on the Channels page lists whom it has met, each with
**Allow** or **Forbid**; "Who else may write to it" when adding the bot is only the default for
the next new person (nobody, or anyone).

Somebody who may not speak is heard where the room is shared - their words in a group are in the
chat, so the agent knows who said what when it is next spoken to - and is told once, alone with
the bot, that its owner has to allow them; never again, and never in a group. Allowed, they get
a chat of their own with the agent when they write to the bot alone (named after them), and
their word in a group the bot is in. Letting somebody into a chat of several, under **Add
someone**, allows them as well.

A guest is a participant like anybody else: what they said is theirs on the page, and the agent
is told who is speaking and that they are a guest, not the owner. The page inside the messenger
is theirs too, but only for their own chats: what the agent put out is the owner's; the commands
`/app` and `/status` answer only those who may speak; only the owner's `/app` opens a tunnel.

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
page. When it asks for a form or a link, the question arrives in words with an **Answer**
button that opens the page inside the messenger at that question: its fields drawn from what
the agent asked for, **Answer**, **Not this**, or something else said instead - declining the
form and saying the words in the chat, where the agent reads them as the answer. The page also
says words into the chat at any time. Only the owner answers the agent's questions.

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

The bot opens the Workbench's own page inside the messenger: `/app` to the bot, the **Answer**
button under a form the agent asks, or whenever a file is too big for the bot to carry. It is
the same page the Workbench draws in a browser - no copy of it, nothing hosted anywhere -
drawn for somebody who came through a messenger, at a phone's width: no rail, the bot's agent
alone, and the tabs that are theirs. The owner has **Chat** (the chat as it stands, the agent's
questions with their fields, the composer - which attaches a file of any size; it lands in the
agent's inbox with your words), **Files** (the agent's folder, what it handed out, to read and
to edit) and **Apps** (the Apps the agent's servers bring, drawn where they always are - in the
sandbox, through the Workbench; through a tunnel, at a second address of the same tunnel).
When a tool of the agent brings an App, the bot says so and its **Open** button opens the page
on that chat's Apps. Somebody the bot met has **Chat** of their own chats, and nothing else:
not the agent's files, not what it put out, not its questions, not its Apps.

Who you are, the page learns once: the messenger signs who opened it, the channel checks the
signature against the bot's token (the Workbench never sees the token and verifies nothing
itself), and the Workbench's own door gives the page a session for a day - the same session for
the same signed data, nobody for a signature that is not the messenger's. From then on the page
asks the Workbench as the page always does, and the Workbench answers as far as that person
reaches: the owner from the messenger reaches this bot's agent - its chats, its files, its
questions - and nothing of the Workbench's setting up (terminals, providers, keys, the Store,
access are the passkey's or the laptop's); a guest reaches their chats. The bot's button carries
the Workbench's own address: the one it is served at, or a tunnel's. The page loads no script of
the messenger's, and names none: the bot's button says where in the address the messenger puts
its signed data. Telegram opens a page inside it over HTTPS only.

## A tunnel: an address from outside, for a while

A Workbench on a laptop has no address the outside can reach. A **tunnel** is a package from
the Store that stands at one for a while (ADR-0015). The one that came with the product drives
Cloudflare's quick tunnel: no account, no key. The first time, `/app` to the bot asks its owner
once - with what is fetched and from where (`cloudflared`, from Cloudflare's release, checked by
digest) - and one tap on **Install and open** installs it, opens the tunnel and sends the button
to the page; **Install cloudflared and open** under Providers → Channels does the same. After
that, `/app` just opens it. What the tunnel reaches is the page, and behind it only somebody who came through
a messenger - never sign-in, the run's secret, a token, or a channel's webhook door. It closes
after thirty minutes unused (what keeps it open is the page being used by somebody known), or
with **Close** on the page; the bot then takes back the buttons it sent through it and says to
send `/app` again. A quick tunnel holds a stream back until it ends, so through it the page
asks for what happened every little while instead of listening; it says what it has to say
a moment later than on the Workbench, and nothing else differs. Cloudflare
calls a quick tunnel a thing for testing and development: no uptime promise, a new address each
time, and some seconds before the edge answers from a fresh one - the bot's button is sent once
it does, so a tap never meets the edge's own error page - which is what an address for a while
is. Webhooks never go through a tunnel; on a laptop
the bot asks the messenger for what is new, which works behind any network.

Any program that answers the tunnel shape (`swem-sdk`, `tunnel`) is a tunnel package: a named
tunnel, another vendor, a peer-to-peer endpoint. A tunnel is refused on a Workbench served at an
address, which has one already.

The bot answers `/status` with how things stand (what the agent is doing, the chat, how
updates arrive), `/stop`, and `/app`; while the agent works, the message being answered wears
👀 and the bot is seen typing. The page inside the messenger is opened by the button the bot sends in
answer to `/app` - to whoever asks and may - so nobody sees a menu button they cannot use; no
domain has to be registered with the messenger for this.

## Where the bot lives

A bot belongs to one Workbench: the one that runs its channel and holds its key. A messenger
allows one consumer of a bot's updates at a time, and so does SWEM. To move a bot, remove it
here and add it there with its token; nothing else travels (ADR-0014).

What is walked by the product's gate: direct messages, pairing, a stranger kept out, a group, a
topic, a guest let in, a question answered with a button, a file each way, a delivery at the
door of a served Workbench and why there is none otherwise, the page inside the messenger with a
60 MB file through it. A real messenger is tried by hand.
