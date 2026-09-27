# ExecPlan 005 — Who said what, and several agents at once

**Status:** active (2026-09-27). Roadmap A2 and A3; ADR-0007; `product/agents.md`.

## Outcome

A person sees a rail of their agents and chats. They open a chat and read who said what; they
write, and the agent answers while another agent is working in another chat. They stop a turn
from the composer. A question an agent asks waits for them: they close the page, open it again
and answer. Replies are read as markdown with code highlighted. The theme switches on the page.
Nothing on the page is a protocol word or a debugging control.

## Acceptance scenario

**Given** the owner's own data root with one agent and its earlier chat, and a second agent
made from the same engine,
**When** the person opens the Workbench, sends a message in the earlier chat, switches to the
second agent, starts a chat there and sends a message while the first is still answering,
closes the tab while one of them is asking before running a command, opens the page again and
allows it,
**Then** both chats show every message with its sender, both answers arrive, the question is
still there after the page was reopened and its answer lets the turn finish, and the database
was copied beside itself before it was changed.

## Current state (2026-09-27)

After A1 a chat opens whatever changed, but a chat is still one route: one agent, one native
session. The page holds one connection; five long-poll loops per connection over HTTP/1 leave
no room for a second live agent. A turn's author is a line of text; the page labels every turn
"You". A permission question lives in the memory of one connection and counts against the
turn's ten-minute deadline.

## Constraints

- The old page and its routes keep working until the new page replaces them in this plan; the
  editor door, the Store, server spaces and Apps in a chat keep working throughout.
- The owner's ledger is copied before migration and the migration is one transaction.
- The engine is told who spoke by the host; nothing a participant typed can close the host's
  block or pose as another sender.
- The page is styled by the view kit alone. What the kit lacks is added to the kit.

## Plan

1. Ledger v2 (`routing.rs`, new `ledger/` modules): participants, chats, members, messages,
   sessions, deliveries, questions; `events` gains `chat_id` and `at_ms`; migration of routes.
2. `envelope.rs`: the block an engine is given; the standing explanation in
   `agent_setup.rs::instruction_block`. `Correspondent` leaves.
3. `workbench_shell/chats.rs` and `runtime.rs`: a message into a chat becomes deliveries; one
   worker per agent; continue, or a fresh session with the history given; questions kept;
   stop; the deadline waits while a question does.
4. `workbench_shell/stream.rs`: one event stream per page with a cursor; chat-addressed routes.
5. The page: stores per chat over the stream; rail of agents and chats; chat with senders,
   markdown and code; composer with stop; question cards; theme switch; agent header. Built on
   the libraries the spike confirms. Kit additions in `web/view-kit/kit.css`.
6. What is replaced leaves: the connection and route HTTP routes the page used, the Dev
   drawer, the lifecycle buttons, the old page modules, and the browser drivers that drove them.

## Progress

- [x] 1 ledger: participants, chats, members, messages, sessions; every route a chat; checked on a copy of a real ledger (eight messages, each with its sender)
- [ ] 2 envelope
- [ ] 3 chats and runtime
- [ ] 4 stream and routes
- [ ] 5 page
- [ ] 6 removal

## Discoveries

(none yet)

## Decision log

- 2026-09-27: A2 and A3 of the roadmap land together. The page is built once, on the
  libraries, rather than once on the old parts and again on the new.

## Validation

(none yet)

## Outcome / remaining gaps

Not started.
