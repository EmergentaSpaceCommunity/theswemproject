# Roadmap

What SWEM can do today is the table in `README.md`. This is what comes next, by capability, in
the order it is being built. Each item ends in something a person does from a real entry point;
the decisions behind them are in `docs/decisions/`.

## Agents on machines that are not this one

Today an agent runs on the computer SWEM runs on, directly or in a container. Next, in order:

1. **A sealed container with its key and its network.** The engine runs in a Podman or Docker
   container with the person's key, a terminal and files inside; where it may connect is a list
   the person allows from the chat.
2. **A machine over SSH.** Add a machine, install an engine there, talk, open a terminal, read
   files - from the same page.
3. **The servers that live with the Workbench, carried to the machine.** The project server
   and the agent's own schedules reach an agent wherever it runs.
4. **A machine that sleeps.** An agent on a cloud machine that is woken by a message or a
   schedule, with nothing of ours left running inside it.
5. **Docker beside Podman.**

## Kinds of machine and of keeper of time from the Store

A kind of machine (a cloud provider) or a kind of keeper of time (a scheduler outside) arrives as
a package a person installs, like an agent or a server, instead of a list fixed in the code.

## A Workbench on a server, finished

Sign-in with passkeys works today. Left: a certificate the Workbench gets itself, the page at a
phone's width, and the checks on Linux and on a phone that are marked unverified in `SECURITY.md`.

## Channels

An agent is reached from Telegram today: a bot added on Providers → Channels, the owner known by
a code, a stranger kept out, the bot in a group and a forum topic, a guest let into a chat, the
agent's questions answered with a button, files each way within the messenger's limits, and
deliveries to the door of a Workbench served at an address (`docs/channels.md`). Next: a Mini
App for files beyond the limits; another messenger as a package of the same kind.

## Teams of agents

Several agents already share a chat, answer when named, and are held before they loop. Next:
roles, delegation, and a project both can work on - which is the Cycle's business, in its own
repository.

## Built in, finished

The harness is built into another product in a page of code today. Left: Apps of servers drawn
for several people in one process; the product's own palette on the page; an address that opens
a chat with words already in it.

## Known debt

- The install plan's fields are named for an agent whatever kind it installs; renamed when the
  receipt schema next changes.
- A server's environment values are given in the MCP catalogue, not from the Store row.
- A `uvx` package is pinned by version, not by hash; a lockfile per install comes later.
