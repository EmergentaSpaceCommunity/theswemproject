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
- **By invitation**: somebody who writes is a guest, and has nothing to say into until you join
  them to a chat from the page.
- **Anyone**: whoever writes gets a chat of their own with the bot's agent, as a guest. The
  agent is told who is speaking and that they are a guest.

A guest is a participant of the chat like anybody else; what they said is theirs on the page.

## What is not finished

Groups and forum topics, a guest invited from the page, a question the agent asks answered with
a button, and files each way within the messenger's limits are carried by the channel's shape
and partly by the code, and are not yet walked by the gate; `ROADMAP.md` has them next. Until
then, count on direct messages with the bot.

## The Bot API address

Telegram's own API is the default. A local Bot API server (which lifts the file limits) or any
other address is typed under **More** when the bot is added. The product's own gate runs the
channel against a Bot API that is a fixture at such an address; nothing in the tree talks to
Telegram.

## Where the bot lives

A bot belongs to one Workbench: the one that runs its channel and holds its key. A messenger
allows one consumer of a bot's updates at a time, and so does SWEM. To move a bot, remove it
here and add it there with its token; nothing else travels (ADR-0014).

The channel pulls from the messenger, so it works on a laptop behind any network. Webhooks for a
Workbench served at an address, and a Mini App for files beyond the limits, are on the roadmap.
