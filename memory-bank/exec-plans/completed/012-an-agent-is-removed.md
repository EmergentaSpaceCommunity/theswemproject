# ExecPlan 012 — An agent is removed, and who may write to it

**Status:** completed (2026-09-28). `product/agents.md` (An agent, Participants and chats). The
boards "Agent: put together" (the part Remove) and "Agent: channels and who may write" of the
approved design.

## Outcome

A person removes an agent they no longer want: in its Settings, under Remove, they read what
goes and what stays, and agree. The agent is gone from the rail and from what can be written
to; its chats stay with what was said in them; its schedules stop; its folder stays where it
is. Its name can be given to a new agent. Under the agent's **Channels** a person reads the
doors into its chats and who may write to it.

## Acceptance scenario

**Given** a data root with three agents, one of them made to be removed, in a chat of several
and with a schedule, **when** the person opens its Settings, Remove, and agrees, **then** the
rail has two agents; the chat of several still shows what the removed agent said, under its
name, and naming it there reaches nobody; its schedule is gone from what is due; its folder is
on the disk. **When** they make a new agent of the same name, **then** it is a new agent with
no chats. **When** the Workbench is restarted, **then** all of that is as it was.

## Current state (2026-09-28)

- No operation removes an agent. A profile is a folder of the inventory, and a folder that
  is taken away by hand leaves its participant in every list. The ledger has a column for
  when a participant was retired and nothing writes it.
- An agent's page has no Channels. Who may write to an agent is not said anywhere: it is
  whoever is in a chat with it.

## Relevant product journey

GJ-02 (set up an agent) gains its end. GJ-12 (a chat of several) for who is in a chat.

## Legacy evidence

None here.

## Constraints

- Nothing a person or an agent wrote is destroyed: chats and their messages stay, the folder
  the agent worked in and its home stay on the disk, and the page says where.
- What the agent held of its own - a key - is forgotten.
- The ledger's schema version does not move.
- An agent that is answering is stopped before it is removed.

## Plan

1. The ledger: a participant retired - its profile let go of, its handle freed, what waited
   for it not begun, its schedules stopped. The inventory: a profile set aside under the data
   root, without what it held of its own.
2. The shell: `DELETE /api/profiles/{id}`; sessions and terminals let go of; the keeper
   chosen for it and the look inside forgotten.
3. The page: Settings, Remove; the rail and every list of agents without the retired; the
   agent's Channels.
4. By hand; suites; the gate.

## Progress

- [x] 1 the ledger and the inventory
- [x] 2 the shell
- [x] 3 the page
- [x] 4 by hand, suites, gate

## Discoveries

- A handle is one of a kind among everybody, the retired included, so an agent that is
  removed is given another (`scribe-removed`) and its own is free. What it said names it by
  its name, which it keeps.
- In a chat of two agents where one is removed the other is the only agent there, and answers
  whatever a person says, as the rules of a chat have it.

## Decision log

- 2026-09-28: a profile is set aside, not deleted. What an agent was put together from is
  small and is what a person would want back; what it held of its own is removed with it.
- 2026-09-28: the folder the agent worked in is never removed by the product. It may hold a
  person's repository.

## Validation

- `removal::an_agent_is_removed_and_what_it_said_stays`: an agent that spoke in a chat of
  several and had a schedule is removed; it is retired, without profile and with another
  handle; its schedule is gone; what it said is in the chat and what it wrote on the disk;
  naming it reaches another agent only; its profile is aside without its secrets; a new agent
  of its name is another participant with its handle.
- By hand in Chrome on a copy of the owner's data root: the agent made by the wizard removed
  from its Settings after the dialog; the page went to another agent, the rail was without
  it, its profile was under `removed/profiles`, its folder where it was. An agent's Channels
  read: the two doors, the command copied, who may write.

## Outcome / remaining gaps

Done 2026-09-28.

Remaining:

- A guest and a messenger are said and cannot be added; "Allow someone" is not on the page.
- What was set aside is not shown anywhere and is not brought back by the product.
- An agent removed while another process answers for it (an editor) is not stopped there.
