# ExecPlan 008 — A chat of several

**Status:** active (2026-09-28). Roadmap A6; `product/agents.md` (Participants and chats);
ADR-0007.

## Outcome

A person starts a chat with two agents. They write `@` and pick who a message is for; the one
named answers and the other does not. Beside the chat they see who is in it, each with its
handle and what it is doing, bring another agent in and take one out, and set the chat's two
rules: whether agents answer only when named, and after how many replies of agents to each
other the chain waits for a person. When agents answer each other the chat says how far the
chain has gone, and at the limit it says it waits; the person writes, and it goes on.

## Acceptance scenario

**Given** the owner's data root with two agents, **when** the person starts a chat of both,
writes a message that names nobody, **then** the chat says nobody was named and nobody
answers. **When** they type `@`, pick the first agent and send, **then** that agent answers
and the second does not. **When** the first agent's answer names the second and the second's
names the first, **then** they answer each other, the chat counts the replies, and after the
limit it waits for a person and says so. **When** the person writes again, **then** the count
starts over. **When** they take the second agent out of the chat, **then** naming it reaches
nobody, and it is not listed.

## Current state (2026-09-28)

- Below the page a chat of several works since ExecPlan 005: who a message is for
  (`chats.rs::recipients`), what an agent answers being passed to the agents it names, the
  count of replies per chat held at the chat's limit (`chat/held`).
- The page draws a chat of several with its members' names in the header and a composer whose
  placeholder says to name somebody with `@`. It offers nothing to pick from, shows no
  members beside the chat, no rules, and says nothing when a message named nobody.
- A chat has `answer_rule` and `reply_limit` in the ledger; nothing sets them and only
  `named` is acted on. A participant joins a chat; nothing takes one out.

## Constraints

- An agent talks to another agent only in a chat they are both in, by naming it.
- What an agent says is for the agents it names and nobody else, whatever the chat's rule.
- Guests and linked chats are the channel slice, not this one; nothing is drawn for them.
- Work in this repository only.

## Plan

1. **Rules and members below the page**: set a chat's rule (`named`, `always`) and its limit;
   take a participant out; say in the record who came, who left and what rule changed. A
   person's words go to every agent when the rule is `always` and nobody is named.
2. **Beside the chat**: who is in it with what each is doing, bring an agent in, take one
   out, the two rules.
3. **In the chat**: pick who a message is for after `@`; names in what was said stand out; a
   message that named nobody says so; the chain's count and its wait are said.
4. **The walk**: two agents in one chat through the real binary.

## Progress

- [x] 1 rules and members below the page: `rule_chat`, `leave_chat`, `chat/ruled`, `chat/left`; `always`
      acted on; `PATCH /api/chats/{id}` takes the rules, `POST|DELETE /api/chats/{id}/members`; an
      agent that asked another by name is given its answer (`chats.rs::asker`); a chain's wait is said
      once
- [x] 2 beside the chat (`page/Members.tsx`): who is in it and what each is doing, Add someone, Take out,
      the two rules
- [x] 3 in the chat: who a message is for is picked after `@`; names stand out in what was said; what
      named nobody says so; how far a chain has gone and that it waits
- [x] 4 the walk: `product_front_door::a_person_puts_two_agents_in_one_chat`

## Discoveries

- With real engines a chat of two showed what the rules lacked. One agent, told to ask the
  other and report, asked it by name; the other answered; the first was never given the
  answer, because the answer named nobody. The product's model says an agent answers when it
  is named or answered, and only the first half was built. An agent that asked another by name
  is given its answer; what it says next goes on only to who it names.
- Two agents whose answers were both held at the limit made the chat say twice that it waits.
  It is said once for a wait.
- A message that names two agents is for both, also when the person meant one to ask the
  other. The rule stays as simple as it is said: who is named is told.

## Decision log

- 2026-09-28: a chat's rule is `named` or `always`. `always` means what a person says and
  names nobody is for every agent in the chat; what an agent says is for who it names and for
  who asked it, whatever the rule.
- 2026-09-28: whoever started a chat stays in it; an agent is taken out and brought back. What
  an agent that was taken out was owed and had not begun is not begun.
- 2026-09-28: guests and linked chats of the accepted design are not drawn: they are the
  channel slice, and nothing stands behind them.

## Validation

- 2026-09-28: `chat_runtime::agents_in_one_chat_answer_when_named_and_a_chain_waits_for_a_person`
  and the unit tests of `chats.rs` green: what names nobody is for nobody; the one named
  answers; a chain of two waits at a limit of two and says so once; a person writes and the
  count starts over; one taken out is not in the chat; where agents always answer, what names
  nobody is for both. The page's tests 31 green. The walk through the real binary in Chrome:
  green.
- 2026-09-28, by hand in Chrome on a copy of the owner's data root with two agents on the real
  engine: a chat of both was started from the rail; after `@` the two were offered; the one
  named answered; told to ask the other and report, the first asked it by name, was given
  "Rome" and said "It said Rome."; the chat counted two replies of four.

- 2026-09-28, the end of the slice, the computer kept awake for the run: lint clean,
  `scripts/suites.sh` green whole, the product gate fifteen green in one run.

## Outcome / remaining gaps

Done 2026-09-28. A person starts a chat of two agents, picks who a message is for after `@`,
sees beside the chat who is in it and by what rules, brings an agent in and takes one out, and
watches a chain of agents answering each other wait for them at the chat's limit. An agent
that asks another by name is told what it answered.

Remaining:

- A message that names two agents is for both. Saying that one should ask the other is done by
  naming the one and writing the other's handle without the `@`.
- A chat's name is changed by the host and has no control on the page of a chat of several.
- Guests, channels and linked chats are the channel slice.
- An App beside a chat and what an agent offers with a slash are drawn in a chat of one agent
  only.
- The owner's own data root was not opened by this build; the scenario was walked on a copy.
