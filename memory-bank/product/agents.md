# Agents, put together from providers

**Status:** accepted (2026-09-27). Chats and the page are built (roadmap A1-A3); providers,
time and hosts are not; the order of building is `roadmap/roadmap.md`, items A and B. Supersedes the "profile" of `journeys.md` GJ-01, GJ-02 and GJ-04 as each item
lands. Reference for the model: the SaaS platform's profiles, provider pool, channels and
schedules (not in this tree). What the code has today is stated in each section, read from the
code on the same date.

## Why

The Workbench was built around a *profile*: one person's settings of a coding agent on this
computer. Everything a person meets follows from that and is wrong for them:

- The rail is a select of settings, a list of "lanes" and the protocol's own verbs (Load, Resume,
  Disconnect, End session). Stop turn, Disconnect and End session act on the page's one
  connection, not on the agent on screen; after switching agents they act on the previous one.
- A conversation is bound to the exact setup it began with. Attaching a server, moving the
  working folder or changing where the agent runs makes every earlier conversation of that agent
  unopenable for good ("route binding drift"), and nothing on the page can undo it.
- An agent can only live on the computer the Workbench runs on. The one sealed way to run it, a
  Podman container, starts with no network and no keys, so a real engine cannot reach its model.
- A sign-in done in the Terminal panel is not seen by an agent that runs on this computer: the
  terminal and the agent have different homes. Terminal and Files are always this computer.
- A key is kept per profile, so the same key is typed once per agent.
- Nobody is anybody: a message has no author the product knows, only a line of text appended to
  it; a conversation has exactly one agent.
- Setup is one page of raw fields saved on every keystroke; chat is plain text with no markdown;
  a Dev switch, ids, counters and event logs are part of the product's page.

## The shape

Three layers, each closed over the one below it:

1. **Providers** are set up once, with their keys, and looked at (probed) when they are added.
2. **Agents** are put together from providers. Any number of agents from the same ones.
3. **Chats** are where people and agents talk. A chat has participants; every message has one.

Nothing in an agent is typed twice and nothing in it dangles: every part of an agent names a
provider that exists, and a provider says which agents stand on it before it can be removed.

## Providers

| Kind | What it gives | Built in | Today |
|---|---|---|---|
| Engine | a coding agent to stand on | the catalogue and the public agent registry | exists: catalogue, discovery, install by plan, receipts; two profiles may share one engine |
| Model | an address, a kind of key, models, **the key itself** | Anthropic, OpenAI, OpenRouter, Groq, Google, Ollama | built (roadmap A4): the key is the provider's, given once in Providers; an agent may still hold one of its own. An agent in a container is handed none until B1 |
| Host | a machine for an agent: start it, run the engine in it, reach its files, open a terminal in it, put it to sleep | this machine directly; Docker or Podman; a machine over SSH; Sprites | this machine and Podman here, chosen from a closed list of two; nothing remote |
| Time | keeps schedules and says when one is due | this SWEM | built (roadmap A5): schedules in the ledger, a time of day and cron, pause, runs kept, an agent's own schedules through a tool; one keeper, the running SWEM |
| Channel | a door into chats from outside | the Workbench, the editor door | the two exist as surfaces; a messenger bot does not |

A kind of provider that is not built in arrives as a package from the Store, like any other
extension. The page shows every kind under one place, **Providers**, with the same shape: what it
is, its key, what was found when it was looked at, which agents use it.

## An agent

| Part | What it holds | Where it comes from |
|---|---|---|
| Identity | name, `@handle`, colour, what it is for (its standing instructions) | the person |
| Engine | the coding agent it stands on | an engine provider |
| Model | provider and model; how the engine is signed in on its host | a model provider |
| Host | where it lives: its files, terminal, sign-in and everything it starts | a host provider |
| Time | who keeps its schedules | a time provider |
| Abilities | servers it attaches, skills, how it asks before acting | the Store, the person |
| Reach | who may write to it, which chats it is in, which channels lead to it | the person |

The handle is the agent's address everywhere: in a chat, in a schedule, from another agent.

## The host: where an agent lives

An agent does not live "on the computer the Workbench runs on". It lives on a host. The Workbench
talks to the host; it is one client of it.

What the harness has today:

| Piece | State |
|---|---|
| Requirements, guarantees, evidence, lease (`environment.rs`) | general, names no backend; reusable as is |
| This machine, directly | works; filtered environment, own process group; not sealed |
| Podman container here | sealed and inspected before attach, but started with no network and given no keys; a container per conversation, removed at its end |
| Choice of backend | a closed list of two (`EnvironmentBackend`), two arms in `product.rs`, string checks on `"podman"` and `"direct-process"` in six files |
| Transport | a struct that always spawns a local process; no constructor for anything but direct and Podman |
| Docker, SSH, Sprites | nothing |
| Terminal and Files | always this machine, whatever the agent runs in |
| Machine set-up, image, clean-up of leftovers | command line only; the page offers the container row even where Podman is absent |

What the reference had: one interface (`create`, `get`, `delete`, `ensure_running`, `exec`,
`fs_list/read/write/delete`, services, checkpoints) with one implementation, Sprites. Its machines
hibernate when idle and are woken by the next turn; nothing is paid for while they sleep.

What a kind of host must be able to do is one thing: run a command on the machine and carry its
streams. Everything else - looking at the machine, installing an engine, files, the engine's own
stream - is done by a small runner of ours put on the host, the same on every kind. Whether an
agent there is sealed in a container is written on the host, never guessed when a chat opens: a
host that lost its container engine refuses, it does not quietly unseal. Nothing that varies
crosses a shell, and a key never appears in a command line.

## Looking before promising: probes

Nothing is offered that was not looked at. Three looks, each stored with its time and shown:

| Look | When | What it finds | Today |
|---|---|---|---|
| The machine SWEM runs on | at start, and on request | system, architecture, processor, memory, disk; Docker and Podman and whether they are ready | built (roadmap A4), shown under Providers, Hosts |
| A host | when added, and on request | reachable, who we are there, system and resources, which container engine, what an agent gets there | nothing |
| Inside an agent's machine | when the agent is made, after a move, on request | the engine and its version, signed in or not, the tools it has, its workspace, its network, **whether it can start containers of its own** | the engine's handshake only |

The third look is what lets the product say in advance that work needing a container, such as a
check against a real database, will be blocked where this agent lives.

## Participants and chats

**A participant** is anyone who can say something: a person, an agent, a guest. Every participant
has one id of one kind and a handle. A guest is a person or a bot from outside that the owner has
allowed by name. An identity outside (a messenger user, a bot) is linked to a participant; it is
never an id of its own in a chat.

**A chat** has participants and messages. Every message names its sender. A chat of a person and
one agent is the ordinary case, not a different thing. A chat of several is where agents meet:
**an agent talks to another agent only in a chat they are both in**, in the open, by naming it.
There is no side channel between agents.

**A session** is the engine's own memory of a chat. It belongs to the agent and lives on its host:
one session per agent per chat. The chat belongs to the product and survives any change of the
agent's setup; when the engine cannot resume its session, the chat goes on in a fresh one with
its history given.

Rules of a chat of several:

- an agent answers when it is named, or answered, or (if the chat says so) always;
- an agent that is named is given what was said since its last turn, not only the one message;
- replies of agents to agents are counted per chat and kept; after the limit (four by default)
  the chain waits for a person, whose message resets the count;
- an agent does one turn at a time across all its chats, schedules included; what arrives
  meanwhile waits its turn and is shown as waiting.

**A channel** links a chat outside to a chat here. Linking a messenger chat makes its people
guests of this chat; agents of this product that sit in both are recognised as themselves, and a
foreign bot is a guest like any other. Everything above works with no channel at all.

What the code has today (2026-09-27, roadmap A2): participants, chats, messages with their
sender, deliveries and questions in the ledger; the envelope; the rules above, with the count
of agents' replies kept per chat. The page draws a chat of several with its members, its two rules,
the names picked after `@` and the chain's count (roadmap A6). Guests and channels are not
built. What it had before: `Correspondent { surface, author,
addressed_to }`, an author typed as free text, every turn labelled "You", one agent per
conversation. What the reference had: authorship only in an envelope
around the text (`<message from chat_id sender user_id scope sender_kind sender_bot>`), nothing
stored; an allow-list of raw messenger ids typed by hand; agents meeting only inside a messenger
group; the reply count kept in memory per receiving agent. The envelope stays, as the way an
engine is told who spoke. The rest is replaced by the model above.

## Time

A schedule is a message that arrives on time: what is said, when, to which agent, into which chat,
made by whom. It is not a process inside the agent's machine.

- **Who keeps time is a provider.** By default it is the running SWEM. Time is never kept inside
  an agent's machine: a machine that sleeps between messages costs nothing between runs, and a
  clock inside it would either keep it awake or die with it.
- **When a schedule is due** the keeper tells the product; the product wakes the agent's machine,
  delivers the message as a turn in the schedule's chat with the schedule as its sender, and lets
  the machine sleep again.
- **An agent makes schedules itself** through a server the product hands to its session
  (`create`, `list`, `pause`, `remove`), limited to its own schedules. No command inside the
  machine and no token written into it.
- A run that was missed because the keeper was off runs once when it is back, and says it was late.

What the code has today (2026-09-28, roadmap A5): what is described above, kept by the running
SWEM. What it had before: a ten-second loop, schedules as files, an interval in minutes, no
pause; an agent could not make one. What the reference had: the clock on the platform for the same reason of cost, a
command inside the machine that called the platform with a token, cron and one-off times, limits
(five minutes at least, twenty per agent).

## What follows for the product

- **Chats never become unopenable.** A chat remembers what setup it started with and continues
  with the agent's current one.
- **No protocol words and no lifecycle buttons.** A person starts a chat, picks one, and stops a
  turn from the composer. An agent has a state a person understands: working, ready, asleep,
  needs sign-in.
- **Several agents are alive at once.** The page keeps state per agent and per chat.
- **Files and terminal are on the agent's host.** A file browser over its workspace with preview,
  edit, upload and download; terminals in its machine, its own and the ones it started.
- **Keys are given once.** A key belongs to its provider. It is handed to an agent's machine when
  it starts and is never written into it.
- **The design is the project's own.** `web/view-kit/palette.css` and `kit.css`, both themes, the
  theme switch on the page. What the kit lacks is added to the kit, from the palette's variables.
- **Nothing for debugging in the product's page.** Diagnostics are a command and an address a
  developer types, never a control a person sees.
- **Mature parts, not hand-rolled ones.** Chat, markdown and code rendering, forms, file tree and
  editor come from established libraries.

## Nothing lost

Every ability the harness has today and every one the reference had, and where it is in this model.

| Ability | From | Where it goes |
|---|---|---|
| Engines from a catalogue and the public registry, installed by plan | harness | Providers, Engines |
| Several profiles of one engine | harness | any number of agents from one engine |
| Model providers, shipped and declared | harness | Providers, Models, now holding their keys |
| Sign-in of the engine (its own methods) | harness | done on the agent's host, checked by the look inside |
| Servers from the Store and of one's own, attached by name | harness | Abilities |
| Skills | harness | Abilities |
| Asking before acting, per profile | harness | Abilities; the question appears in the chat |
| What the agent is given from an App | harness | the chat's composer |
| Spaces of servers that bring a home App | harness | Apps in the rail |
| The Store | harness | unchanged; also the source of new kinds of provider |
| The editor door | harness | a channel; its turns carry the editor as sender |
| Inbox and outbox | harness | folders of the agent's workspace, in Files |
| Terminals the agent starts | harness | Terminal, beside the person's own |
| Guarantees and evidence of an environment | harness | what a host says an agent gets; shown in Providers and in the agent's host |
| Plans for a Podman machine, an image, clean-up | harness | actions of a host in Providers, with the same consent |
| The ledger of events | harness | under every chat; never on the page as a log |
| Light and dark theme, passed into Apps | harness | the theme switch in the rail |
| Generated identity in the engine's instructions | reference | Identity, with who it may talk to and what it may use |
| The message envelope | reference | how an engine is told who spoke, in every chat |
| A pool of providers shared by agents | reference | Providers |
| A machine that sleeps and is woken by a turn | reference | a property of a host; Sprites is one |
| Snapshots of a machine | reference | an action of a host that has them |
| A bot per agent, allow-list, group rules | reference | a channel linked to a chat; participants and rules of the chat |
| Agents naming each other, reply limit | reference | chats of several |
| Schedules made by the agent, the clock outside the machine | reference | Time |
| Files, terminal, settings, channel per agent | reference | the agent's tabs |

## Order of work

`roadmap/roadmap.md`, items A (agents and chats) and B (hosts). Chats and the page come first,
on agents that live on this machine; hosts follow.

## How it is judged

By a person opening the product on their own data and doing the thing. A walk that passes on a
clean fixture while the owner's conversation does not open is not evidence.
