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

A group the bot is in is one chat of several here, named after the group; whoever writes to the
bot in it is in the chat, and the page lists them beside it. Telegram decides what the bot sees
in a group: with privacy mode on (the default for a bot), only what mentions it, replies to it or
is a command. A forum topic is a chat of its own, named after the group and the topic's number;
rename it on the page.

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

## Where the bot lives

A bot belongs to one Workbench: the one that runs its channel and holds its key. A messenger
allows one consumer of a bot's updates at a time, and so does SWEM. To move a bot, remove it
here and add it there with its token; nothing else travels (ADR-0014).

A Mini App for files beyond the limits is on the roadmap. What is walked by the product's gate:
direct messages, pairing, a stranger kept out, a group, a topic, a guest let in, a question
answered with a button, a file each way, a delivery at the door of a served Workbench and why
there is none otherwise. A real messenger is tried by hand.
