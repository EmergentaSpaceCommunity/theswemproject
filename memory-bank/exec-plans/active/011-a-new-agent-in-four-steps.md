# ExecPlan 011 — A new agent in four steps

**Status:** active (2026-09-28). Roadmap B3; `product/agents.md` (An agent, Looking before
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

- [ ] 1 the look inside
- [ ] 2 the wizard
- [ ] 3 Providers, Engines
- [ ] 4 by hand, suites, gate

## Discoveries

## Decision log

## Validation

## Outcome / remaining gaps
