# ExecPlan 006 — A key given once, an agent edited in forms, and what this machine offers

**Status:** active (2026-09-27). Roadmap A4; `product/agents.md` (Providers, An agent, Looking
before promising).

## Outcome

A person opens **Providers** from the rail. Under Models they give a provider its key once;
every agent that answers from that provider uses it, and nothing is typed per agent. Under
Hosts they see what the machine SWEM runs on has - system, processor, memory, disk, whether
Podman or Docker is there and ready - looked at when SWEM started and again when they ask.
An agent's Settings are forms by section, in the project's own design, saved when the person
says so; where an agent may run is offered from what was found, not from a fixed list.

## Acceptance scenario

**Given** the owner's data root with two agents of one engine, one of which has the key in its
own settings,
**When** the person opens the Workbench built from this plan, goes to Providers, sees the key
already standing on the provider it belongs to, makes nothing else, and writes to each agent,
**Then** both answer; the second agent's settings hold no key; Providers says which agents use
the provider. **When** they open Hosts, **then** the page says what this machine is and that a
container cannot be started here (no Podman machine), and the second agent's settings do not
offer a container as if it could run. **When** they change the agent's name for what it is for
and its way of asking in Settings and press Save, **then** the next turn runs with both.

## Current state (2026-09-27)

- A key sits in each profile's `secrets.json` under a variable name; a provider names only the
  kind of key it takes. At launch every secret of the profile is put into the engine's
  environment; nothing looks the provider's key up. Two providers that take one kind of key
  cannot both be used.
- `secrets.json` is written mode 0600; its directory has default permissions.
- `GET /api/environments` answers a fixed list of two; the page is never told whether Podman
  exists. The only look at the machine is `swem environments probe`, for Podman alone.
- A terminal on this machine gets `HOME = agent_home`; an engine run directly gets the
  machine's `HOME`. A sign-in done in the terminal is therefore not the engine's.
- Settings is one panel of ten sections on the old page's classes (`shell.html`), one status
  line for all of it. Selects and checkboxes save at once; text saves on a button.
- No form library is in the page's dependencies.
- No Rust test touches a secret or the routes of providers and environments over HTTP.

## Relevant product journey

GJ-02 (set up an agent), GJ-01 (first run). A new journey for Providers is written when this
plan ends.

## Legacy evidence

The reference kept a pool of providers shared by agents, with the key on the provider
(`product/agents.md`, Nothing lost). The harness's vault per profile is what is replaced; a
profile's own secret stays possible, as "this agent uses its own".

## Constraints

- A key is never written into a profile, a ledger, a log or a command line. The store says on
  the page what keeps it.
- A profile that has a key of its own keeps working through the migration; nothing is removed
  from a profile unless the same value stands on its provider.
- An agent in a container gets no key until roadmap B1; this plan does not pretend otherwise.
- The page is styled by the kit and `workbench.css`; what the kit lacks is added to the kit.
- The gate's walks find Settings by ids the panel has today; a walk is changed in the step
  that changes what it presses, and only then.
- Work in this repository only. The Cycle is not touched.

## Plan

1. **A key on its provider.** `keys.rs`: a store under `<data>/keys`, the directory 0700, a
   file 0600 per provider. Providers say whether they hold a key and who uses them; routes to
   give and take a key. At launch the engine and its terminals get the key of the agent's
   provider under the variable its kind names, then the profile's own secrets over it. At
   start, a profile's secret of its provider's kind moves to the provider when the provider
   has none or the same. The Providers place on the rail, Models: give a key once.
2. **What this machine offers.** `host/`: the look at this machine (system, architecture,
   processors, memory, disk, Podman and Docker and whether each is ready), taken at start and
   on request, kept with its time. Hosts in Providers show it. Where an agent may run is
   offered from it: a container is offered only where one can be started, and said why not.
3. **Settings in forms by section.** Put together (engine, model, host), Identity (name, what
   it is for), Where it works, Tools, Skills, Permissions; kit fields; each section saved on
   its own button with its own word of what happened. A terminal on this machine directly has
   the machine's home, so a sign-in there is the engine's.
4. **The record of hosts.** `HostBook` in place of `EnvironmentBackend`: this machine directly
   and containers here are two records with what an agent gets written on each; the string
   checks on a backend's name leave the places the record can answer.

## Progress

- [x] 1a the key store (`keys.rs`): a directory its owner's alone, a file per provider, the value never printed
- [x] 1b providers hold keys (`workbench_shell/provider_keys.rs`): who uses a provider and whether it has
      its key; `PUT|DELETE /api/model-providers/{id}/key`; the engine and the terminals of an agent on
      this machine get its provider's key, then its own over it; an agent's key of its provider's kind
      moves to the provider at start
- [x] 1c Providers on the rail (`page/Providers.tsx`, `page/providers.ts`), Models: what opens each
      provider, given in a dialog whose field hides what is typed, replaced, taken away; who answers
      from it
- [x] 2a the look at this machine (`host/this_machine.rs`, `workbench_shell/hosts.rs`): taken while the
      door opens and when asked, kept under `<data>/hosts`; `GET /api/hosts`,
      `POST /api/hosts/this-machine/look`; `/api/environments` says whether each can be chosen today and
      why not; moving an agent where it cannot live is refused in the look's words
- [x] 2b Hosts in Providers: the machine as it was found, Look again, the two places an agent may live
      with who lives there; an agent's settings do not offer a container that cannot start and say why
- [x] 3a Settings in parts on the kit (`page/settings/`): put together, identity, where it works, tools,
      skills, permissions, signing in, on its own; forms on `react-hook-form`; each part says for itself
      what became of what was done in it; a name and a handle are changed (`PATCH /api/people/{id}`);
      adding a provider moved to Providers; the old panel and its sign-in card are gone
- [x] 3b a terminal of an agent that lives on this machine directly has this machine's home, as its
      engine has
- [ ] 4 the record of hosts - not built in this plan, see the decision log

## Discoveries

- A form on `react-hook-form` hears what is typed, not what a script assigns to a field, so the
  walks type into the settings as a person does (`b.fill`).
- The two tests of `callback_authority` that were red on macOS assumed the temporary directory's
  name is its real one and that a hang-up is named without its number. They hold on both now.

- The owner's own agents hold no key: they are signed in the way their engine does it. So a
  provider without a key is not a fault, and the page says "Not given here", not a warning. The
  acceptance scenario's agent with a key of its own is made on the copy.
- The system's facts come from the `sysinfo` crate (system and disks only); the engines are asked
  by running them, each given eight seconds, the whole look forty-five.

## Decision log

- 2026-09-27: a profile with no provider chosen keeps its secrets where they are: there is no
  provider to move them to, and its engine reads them by their variable names as before.
- 2026-09-27: what is chosen from a list in Settings (the provider, the model, the host, how it asks,
  a server attached) is taken when it is chosen, as it was; what is written is taken on the part's
  own button. A person who picks from a list has said what they want.
- 2026-09-27: the parts of Settings stand in one column with their names beside it, as the accepted
  design draws Put together and Identity on one page. The names move the page to a part.
- 2026-09-27: `HostBook` in place of `EnvironmentBackend` is left to roadmap B1. What a person was
  promised here - this machine as it was found, the two places an agent may live, nothing offered
  that cannot start - is answered by `workbench_shell/hosts.rs` over the two environments there
  are. A record of hosts is wanted when there is a third kind of host to write into it.
- 2026-09-27: what the inside of an agent's machine has (the accepted design's right-hand card) is
  roadmap B3, the look inside; nothing stands in its place.
- 2026-09-27: Time and Channels are not tabs of Providers until roadmap A5 and the channel
  slice build what would stand under them.

## Validation

- 2026-09-27, step 1a-1b: `keys` (3) and `provider_keys` (2) green: a key given once reaches the
  engines of two agents and not of a third that answers from elsewhere; taken away, it reaches
  none; what a page is told never holds it; a shared key moves to the provider, a different one
  and one with no provider stay. Lint clean; `model_providers`, `workbench_shell`, `chat_runtime`
  green.

- 2026-09-27, step 2a: `hosts` green on this machine, where Podman is installed and has no machine:
  before a look nothing is refused; after it a container is not offered and moving an agent into
  one is refused with the reason; the next start says what was found.

- 2026-09-27, steps 1c and 2b, by hand in Chrome on a copy of the owner's data root, the real
  binary: Providers opens from the rail in both themes; Hosts says macOS 12.7.6, 4 cores, 8 GB,
  12 GB free, Podman with no machine set up, Docker not answering, and that a container cannot
  start and why; a made-up key was given to a provider no agent uses, stood as given with its
  time, and was taken away; the directory of keys is its owner's alone. No walk of the gate
  presses Providers yet: one is written with the forms of step 3, whose names it will press.

- 2026-09-27/28, steps 3a and 3b, the end of the slice. Lint clean. `scripts/suites.sh` green whole,
  `callback_authority` with it for the first time on macOS. The product gate, one walk after
  another: the first run was thirteen green and the walk of a command red, because the forms of
  the settings carried the class of a question's card and the walk took a hidden form for a
  question; the forms have a class of their own. The second run was thirteen green, that walk
  among them, and another red on a browser that did not answer for a minute: the run took two
  and a half hours, the computer having slept in it. That walk alone is green. So each of the
  fourteen was green after the last change, thirteen together and one alone; the container walk
  passes by finding nothing to run a container with. After that the page stopped beginning a
  refusal with its kind; the walk that reads refusals (first run, with a place the product does
  not have) was run again alone and is green.
- By hand in Chrome on the copy, the real binary: Settings in both themes; an agent renamed, given
  a handle and what it is for, seen in the rail and its header; somebody else's handle refused
  in words; where it works saved.

## Outcome / remaining gaps

Done 2026-09-28. A person opens Providers from the rail. Under Models they add a place a model
is served from and give it what opens it once; an agent that answers from it is handed it when
it starts. Under Hosts they read what this machine has and whether a container can start on
it. An agent's Settings are parts with their own forms and their own word of what happened.

Remaining:

- The record of hosts and the look inside an agent's machine: roadmap B1 and B3.
- An agent in a container is handed no key until B1.
- A provider can be opened by one kind of thing. A subscription token for Anthropic is still an
  agent's own, under Signing in.
- A new name reaches an agent's instructions when its next session starts, not the one open.
- The walks still find some controls of Settings by names the old panel had (`.server-attach`,
  `.skill-row`); new walks find controls by what they say.
- The owner's own data root was not opened by this build; the scenario was walked on a copy.

Afterwards (2026-09-28): Hosts only said what was found, and the owner asked where Podman is set
up. Set up was added beside Podman: the plan in words, agreement to it as offered, its steps as
they go, the look afterwards. A machine is refused before anything is downloaded when QEMU does
not start or the computer lacks the memory, processors or disk. Docker has no set-up; the image
an agent runs in is still consented to when the agent is put in a container.
