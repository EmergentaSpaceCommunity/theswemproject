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
- [x] 2 envelope: the block (`envelope.rs`), named anew every turn by a value nothing said in the turn holds; the standing explanation written into the agent's instruction file at every open. `Correspondent` still stands: it leaves with its last callers in steps 3 and 6
- [x] 3 chats and runtime
  - [x] 3a in the serving process: a message is owed to the agents it is for (`chats.rs`,
    `recipients`), one worker per agent under the agent's turn lock (`runtime.rs`), the session
    continued or a fresh one given the history and a transcript file, questions kept and
    answered from the ledger, stop, the deadline standing still while a question waits
    (`session.rs`), what was left running settled when the product starts
  - [x] 3b the clock says its message into a chat as a schedule: a participant of kind
    `schedule` made by the owner; each due schedule is followed to its end by itself, so one
    that waits for an answer holds no other up
  - [x] 3c the editor door says its message into a chat and carries out its own delivery: the
    id an editor keeps is the chat's; a delivery is claimed by the door whose name begins the
    message's; the editor is asked what the agent asks, and the answer is kept in the chat
- [x] 4 stream and routes: `GET /api/stream` (server-sent events, `Last-Event-ID` or `?cursor=`, `state` whole for a page with no place, `reset` for a place the record never reached); `/api/people`, `/api/chats`, `/api/chats/{id}` (read, rename), `.../messages`, `.../stop`, `/api/questions/{id}` and `.../answer` (`stream.rs`). The old page's routes stand until step 6
- [ ] 5 page
  - [x] 5a the spike: the chat library's primitives with an external store render several
    senders by name with no Tailwind; unfinished marks are closed while an agent writes; one
    file, built as before
  - [x] 5b the page: rail of agents, chats of several, Apps and the Store; the agent's header
    with what it stands on and where it lives; its chats; the chat with senders, markdown,
    code, what the agent did, its plan, what it asks; composer with attach, send and stop; the
    theme switch; Files, Terminal and Settings of the agent as tabs. Kit gained `.k-caption`,
    `.k-avatar`, `.k-status`, `.k-badge`, `.k-rail`, `.k-rail-item`, `.k-segmented`,
    `.k-switch`, `.k-notice`, `.k-bubble`, `.k-toolcall`, `.k-prose`, `.k-code`, `.k-menu`
  - [x] the new-agent form: a name and the engine it stands on, so a second agent of one
    engine is made from the page
  - [x] 5c in the composer: what the agent offers with a slash, how much of its memory it
    has used, and what an App said the person is looking at, which goes with their words as
    part of the message and is given to an agent that attaches the App's server
  - [ ] 5d what needs the engine's live session from the page: the model and the mode the
    session offers, the Apps an agent's servers bring into its chat, a form or a link an
    agent asks for
- [ ] 6 removal

## Discoveries

- The old page, the editor door and the clock all reach the engine through
  `submit_workbench_prompt` with a `Correspondent`. It cannot leave before a message into a chat
  has another road (step 3), so step 2 adds the block and the explanation and removes nothing.
- The explanation says that a turn with no block was written by the principal. That is true of
  every turn the old road gives, so the instructions are right before and after step 3.
- The hub App's node suite needs `NODE_OPTIONS=--experimental-strip-types` on Node 22.17, as
  `scripts/suites.sh` of the Cycle sets it; run bare it fails on the first `.ts` import.

- An agent's message is kept by the ledger when the turn's `acp/prompt_response` is appended,
  and a person's when a prompt without the host's block is: one reader (`spoken_by_event`)
  serves the migration and both roads, so a chat written to through the old page has its
  messages too.
- `File::lock` is stable since Rust 1.89; the workspace said 1.88 and builds on 1.95 only
  (`rust-toolchain.toml`). `rust-version` is 1.89 now.

- A question is answered by whoever gets there, and that may be another process: the page
  beside an editor's door. So a permission's answer is written into the ledger and carried to
  the session by the process that runs the turn; nothing about it is kept in memory.
- The door's walks drove the old page to read the record. They read `/api/chats` now; what
  they looked at in the page's session list comes back with the new page.
- `native_session::cancellation_protocol_failure_and_transport_timeout_remain_distinct` holds a
  one-second deadline over a process start: green alone three times of three, red beside the
  other fifteen tests of its suite on this machine.

- The page's bundle is 2.4 MB (0.5 MB compressed), four times what it was: seventeen grammars
  for colouring code are most of the difference. It is served from this machine.
- The browser driver waited for the old page's bar of spaces; it waits for the rail now. A
  driver goes to a place by its address (`#/apps/<server>`, `#/agents/<id>`), as a person
  with a link does.

## Decision log

- 2026-09-27: a form's fields and a link's address are not written into the ledger, as they
  are not written into the route's events: the ledger keeps that a form or a link is asked
  and the page reads the rest from the session while the question waits.
- 2026-09-27: agents answering each other are counted per chat and held at the chat's limit
  from this slice on, so a chat of two agents cannot run away before A6 draws the limit.
- 2026-09-27: stopping a chat ends what waits behind the running turn as well.
- 2026-09-27: A2 and A3 of the roadmap land together. The page is built once, on the
  libraries, rather than once on the old parts and again on the new.

## Validation

- 2026-09-27, steps 1-3a: `cargo clippy --workspace --all-targets -- -D warnings` clean;
  `cargo test -p swem-host --no-fail-fast`: 48 targets green, `callback_authority` red as
  before this plan (macOS). `chat_runtime` (six walks over the echo engine): a message is
  answered and each has its sender; the same session goes on; two agents work at once while
  one waits for an answer; the question is read and answered after nobody looked; a turn is
  stopped; a turn outlasts its deadline while its question waits; with the engine's sessions
  deleted the chat goes on in a fresh one that is given what was said and the transcript.
- 2026-09-27, the acceptance scenario, by hand on the same copy with the real engine, the
  product restarted in between: a second agent was made from the page on the same engine; the
  first was asked to write a file and asked before it did; the page was closed and a new
  window showed the question still waiting; while it waited the second agent was asked
  something in its own chat and answered; the question was answered and the turn finished
  with the file written. The options of a question are the engine's own words.
- 2026-09-27, step 5b, by hand on a copy of the owner's data root with the real engine: the
  ledger was copied to `routes.jsonl.v1` and became chats; the earlier chat is read with each
  message under its sender; a message was sent from the page, the agent was shown working
  with Stop in the composer, and answered in the same session of the engine with a table and
  a block of Rust, drawn as a table and as coloured code; both themes. Pictures were taken
  through the project's own browser driver.
- 2026-09-27, steps 3b and 3c: `chat_runtime` gains the clock's walk. The editor door's four
  walks of the product gate, each alone against the real binary with the protocol's own client:
  a person works from their editor, the agent reads the file they have open, two windows, and
  closing the editor and coming back to the same conversation - green.
- 2026-09-27, step 4: `chat_runtime` gains the page's walk over HTTP: the state whole, a chat
  started, a message said, every event on the one stream in order with each message's sender,
  the chat read and renamed beside it, a page coming back to its place, a lost place reset.

## Outcome / remaining gaps

Not started.
