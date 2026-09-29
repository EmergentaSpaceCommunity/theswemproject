# ExecPlan 011 — A new agent in four steps

**Status:** completed (2026-09-28). Roadmap B3; `product/agents.md` (An agent, Looking before
promising). The boards "New agent: engine and identity", "New agent: put together", "New agent:
sign-in and a look inside" and "Providers: hosts" of the approved design.

## Outcome

A person presses **New agent** and makes one in four short steps, from providers that were
looked at: what it is built on and who it is; where it works, its model and who keeps its time;
how it signs in, and what it will have where it lives; what it may do. They can stop after the
second step and have the agent. Under Providers they see the engines beside the models, the
hosts and time.

## Acceptance scenario

**Given** the owner's data root, **when** the person presses New agent, chooses Claude Code,
names the agent, gives it a handle, a colour and what it is for, and continues, **then** the
second step offers the hosts this product has with what each gives and whether it is ready -
one that cannot take an agent says why and cannot be chosen - the models that were set up and
the keepers of time. **When** they continue, **then** the agent exists, the third step says
how it is signed in and shows what was found from inside its machine: the engine and its
version, whether its model answered, the tools it has, its workspace and whether a container
can be started there. **When** they finish, **then** they are in the agent's chat and it
answers. **When** they open Providers, Engines, **then** the engine is there with the agents
that stand on it.

## Current state (2026-09-28)

- New agent is one form: a name and an engine. Everything else is set afterwards in Settings.
- Nothing is looked at from inside an agent's machine but the engine's handshake. The runner
  has `probe` (ExecPlan 010) and nothing calls it.
- Providers has Models, Hosts and Time. Engines are shown in the Store and in the form of a
  new agent. The design has Engines and Channels as well, and "Add a host".
- Podman has no machine on this computer, so "Containers here" cannot take an agent; the
  wizard shows that and is done by hand for an agent on this machine.

## Relevant product journey

GJ-01 (first run) and GJ-02 (set up an agent) are what this replaces the beginning of.

## Legacy evidence

None here.

## Constraints

- Nothing is offered that was not looked at; a host that cannot take an agent is shown with
  why, and is not chosen.
- What a step sets is what Settings sets, by the same operations. The wizard is another way
  in, not another record.
- No protocol word and no control for a walk's sake on the page. Base UI, `react-hook-form`,
  the kit.
- What the design shows and cannot be done yet - a host over SSH, Sprites, a channel - is
  shown with why, in words, and is not a dead control.

## Plan

1. The look inside: the shell asks the runner where the agent lives, adds the engine's
   handshake and whether a session opens, keeps what was found with its time
   (`<data>/hosts/looks/<profile>.json`). `GET` and `POST /api/profiles/{id}/look`.
2. The wizard: four steps on the page, the agent made when the second is left.
3. Providers, Engines; "Add a host" and Channels said as they stand.
4. By hand on a copy of the owner's data root with the real engine; suites; the gate's walks
   that make an agent.

## Progress

- [x] 1 the look inside
- [x] 2 the wizard
- [x] 3 Providers: Engines, Channels, Add a host
- [x] 4 by hand, suites, gate

## Discoveries

- What an engine says of itself when it is greeted is the bridge SWEM talks to it through
  (`@agentclientprotocol/claude-agent-acp` 0.81.0), not the coding agent a person knows. The
  page names the engine as the catalogue does and gives the version it was told.
- Installing an engine made an agent of it at once, named after the engine. In steps a person
  takes that is an agent nobody named; the wizard and Engines install without it, and the
  first run's walk makes the agent in the steps after installing.
- The consent to an install was a dialog of the browser. In the wizard and under Engines it is
  a dialog of the page. The Store still asks with the browser's.

## Decision log

- 2026-09-28: the agent is made when the second step is left, because the third looks inside
  its machine and signs it in where it lives, and neither can be done for an agent that is
  not there. After that the first two steps are not gone back to; what they set is changed in
  Settings.
- 2026-09-28: whether an agent is signed in is told by asking its engine for a session, with
  the agent's own variables, and reading whether it asks to be signed in first. It costs
  starting the engine, and is done when the agent is made and when a person asks.
- 2026-09-28: "Add a host" is on the page with the kinds the design has and says that none
  other than this machine is reached yet, instead of being left out until it works.

## Validation

- `looks` (2): a machine looked at from inside with an engine that starts - greeted, a
  session opened, the programs, the workspace - and the look kept; an engine that does not
  start is what was found, and the machine is looked at all the same. `swem-runner`'s
  `exec` test: the workspace owned and what is free there.
- `test/page-looks.test.mjs` (3).
- The gate, fifteen green in one run: every walk that makes an agent takes the first two
  steps; the first run installs an engine by the page's own consent dialog.
- By hand in Chrome on a copy of the owner's data root: an agent made in four steps on
  Claude Code, named, given a handle, a colour and what it is for; "Containers here" shown
  with why it cannot start and not chosen; the look inside - the engine starting in 0.9 s,
  signed in to its model and answering in 4.1 s, git, node and python with their versions,
  the workspace with what is free, no container - and the agent, asked in its chat what it
  is for, answering with what was written in the first step. Providers: Engines with the
  agents that stand on each, Channels, the dialog of adding a host.

## Outcome / remaining gaps

Done 2026-09-28 for an agent on this machine.

Remaining:

- An agent in a container is greeted and not looked at from inside; its network is not
  looked at for any agent. Both come with the machine an agent keeps (roadmap B1, B2).
- The fourth step sets how the agent asks. Its tools and skills are given in Settings.
- The first two steps are not gone back to once the agent is made.
- The Store's consent is still the browser's dialog.
- A host over SSH, Sprites and a messenger are said and cannot be added (roadmap B4, B6, C).
