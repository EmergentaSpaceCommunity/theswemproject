# ExecPlan 013 — An agent's terminals, as the design has them

**Status:** completed (2026-09-28). `product/agents.md` (What follows for the product: files and
terminal are on the agent's host). The board "Agent: terminal" of the approved design.

## Outcome

A person opens an agent's **Terminal** and sees the terminals that are open where the agent
lives, each by what runs in it and who started it: theirs, and the ones the agent started.
They choose one and it is on the screen, from its beginning. They open a new one, type, and
close it. They watch what the agent runs without ending it. The page says where the terminal
is. It follows the theme, and fills the page.

## Acceptance scenario

**Given** an agent that works alone inside its workspace and was told to leave a command
running, **when** the person opens its Terminal, **then** the command is listed as started by
the agent; **when** they choose it, **then** what it said is on the screen; **when** they
press New terminal, **then** a shell of their own is on the screen, in the agent's folder,
and the agent's command is still listed and running; **when** they type a command, **then**
it answers; **when** they close theirs, **then** it is gone from the list.

## Current state (2026-09-28)

- The panel is what it was before the page was remade: "Open a terminal", "Close", "Stop
  watching", a list with "Watch" buttons, the terminal's id on the screen, the word
  "profile", colours read from names the palette no longer has, a screen of a fixed height.
- The gate's walks find it by ids (`#terminal-open`, `#terminal-id`, `#terminal-detach`).

## Relevant product journey

GJ-01 (a terminal in the agent's environment) and the walk
`a_person_watches_their_agent_run_a_command`.

## Legacy evidence

None here.

## Constraints

- One screen; the terminal chosen is read from its beginning, which the host keeps.
- A terminal that is not looked at does not hold a connection.
- A terminal the agent started is ended only when the person says so, in words that say
  whose command it is.
- An agent in a container: its terminal is on this machine until it keeps a machine of its
  own, and the page says so.
- No id of a terminal on the page.

## Plan

1. The page: the strip of terminals, New terminal, the screen, the words; the palette.
2. The walks: by what the page says.
3. By hand; suites; the gate.

## Progress

- [x] 1 the page
- [x] 2 the walks
- [x] 3 by hand, suites, gate

## Discoveries

## Decision log

- 2026-09-28: there is no "Stop watching". A person who watched what the agent runs chooses
  another terminal or opens their own; the agent's goes on, and is ended only by its own
  control, which says whose command it ends.
- 2026-09-28: a terminal is called by what runs in it; a shell given one line to run is that
  line. Two shells of a person's are called the same, as two windows of a terminal are.

## Validation

- `test/page-terminals.test.mjs` (2): what a terminal is called.
- The gate: `a_person_watches_their_agent_run_a_command` finds the agent's command among the
  terminals by what it runs and who started it, watches it, and opens a terminal of the
  person's own while it runs; the first run opens one and types in it. Both by what the page
  says.
- By hand in Chrome on a copy of the owner's data root, both themes: a terminal opened in the
  agent's folder and a command typed in it; a second opened; the first chosen again with what
  it said from its beginning; both closed, none left on the host.

## Outcome / remaining gaps

Done 2026-09-28.

Remaining:

- The terminal of an agent in a container is on this machine, and the page says so; it moves
  inside with the machine an agent keeps (roadmap B1).
- One screen: two terminals are not seen side by side.
