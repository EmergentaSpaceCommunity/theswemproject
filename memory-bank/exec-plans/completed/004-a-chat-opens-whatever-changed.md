# ExecPlan 004 — A chat opens whatever changed since it began

**Status:** completed (2026-09-27). Roadmap A1; `product/agents.md` ("Chats never become
unopenable").

## Outcome

A person picks an earlier chat of an agent and reads it at once; they write, and the agent goes
on with its current setup. A server the agent attaches that is not set up on this computer is
said on the page and can be detached; it stops nothing.

## Acceptance scenario

**Given** the owner's own data root: the profile `claude-code` attaches a server that is no
longer declared, and an earlier chat of that profile was begun with nothing attached,
**When** the person opens the Workbench, picks that chat, reads it, and sends a message,
**Then** both earlier turns are shown before any engine is started, the message is answered in
the same chat, the page says which attached server is not set up and offers to detach it, and
after a restart of the product the same holds.

## Current state (2026-09-27)

Four gates close that chat, and the first closes a new one as well:

1. `product.rs` (the resolver): an attachment naming no declared server fails the resolution.
2. `workbench_shell.rs` `open_connection_with_id`: every attachment must pair with a resolved
   declaration, fail-closed.
3. The same function, Load/Resume: the stored binding is compared whole with the profile's
   current one; any difference is a worded refusal (409).
4. The same function, after the engine is ready: `bind_route` compares the whole binding again.

`live_route_connection` repeats comparison 3. `amend_profile` refuses a set of attachments that
still holds the undeclared name, so the page cannot change the others, and the page lists only
declared servers, so the undeclared one cannot be detached. The page opens an engine as soon as
a chat is picked; history is read only after a connection exists.

## Constraints

- `bind_route` stays the proof that a retry names the same native session: route id, agent,
  profile and native session id are still compared.
- An attachment that is absent only narrows what the agent can reach; nothing is widened by
  skipping it. The permission boundary is built from the attachments that resolved.
- The engine is never given old prompts again by the host. Continuing is the engine's own
  `session/resume` or `session/load`.
- The page of this slice is the present one; it is replaced in A2 and A3. Only what the
  acceptance needs changes in it.

## Plan

1. `routing.rs`: `RouteIdentity` and `require_identity`; `bind_route` inserts the whole row and
   proves identity. `require_binding` goes.
2. `workbench_shell.rs`: Load/Resume and `live_route_connection` prove identity only; what
   changed since the chat began is recorded once as `host/setup_changed` with the sentences
   `configuration_differences` already makes.
3. `session.rs`: `NativeSessionStart::Continue` - resume when the engine offers it, else load -
   and the page's "resume" asks for it.
4. Resolver and pairing: an undeclared attachment is skipped, recorded as
   `host/attachment_unavailable`, and said in the connection's notes. `amend_profile` carries an
   attachment the profile already holds even when its server is not declared.
5. Page: picking a chat reads it from the ledger with no connection; the first message opens
   the connection and is then sent; an attached server that is not declared is listed with
   "not set up on this computer" and can be detached.
6. When the engine cannot continue (its session is gone), the refusal says so in words and the
   chat stays readable. Going on in a fresh native session inside the same chat needs the chat
   to be more than one route; that is A2.

## Progress

- [x] 1 identity
- [x] 2 no refusal on a changed setup
- [x] 3 continue
- [x] 4 an attachment that is not set up
- [x] 5 page
- [x] 6 worded refusal when the engine cannot continue

## Discoveries

- With the resolver no longer failing, the profile's handshake answers again: the rail said
  "the agent could not be reached" for the same reason the chat did not open.
- `host/setup_changed` and `host/attachment_unavailable` take their event id from what they
  say, so opening the same chat again appends nothing.

## Decision log

- 2026-09-27: the fallback to a fresh native session with the history given is left to A2,
  where a chat holds several sessions. Here a chat is still one route.

## Validation

Done by hand in Chrome on the owner's own data root, the ledger and profiles copied first:

- the earlier chat was picked and read with no agent started; a message was sent and answered
  in the same chat; the record went from 186 to 208 events;
- the product was stopped and started; the chat was read again, with the new exchange in it; a
  second message was answered;
- Setup lists the attached server as "Attached, but not set up on this computer" with Detach;
- the ledger holds one `host/setup_changed` ("it now reaches ...") and one
  `host/attachment_unavailable` for that chat.

`cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings` are clean.
`scripts/suites.sh` passes except `callback_authority` (roadmap debt, macOS). One test of
`native_session` (`cancellation_protocol_failure_and_transport_timeout_remain_distinct`) failed
two runs of three while the product and an agent were running on the same machine and passed
otherwise; it holds a one-second deadline and is not touched by this change.

## Outcome / remaining gaps

Done. A chat opens and goes on whatever changed about its agent since it began.

Remaining: when the engine itself no longer has the session, the chat is readable and the
refusal is in words, but going on needs a new chat. A2 makes a chat hold several sessions and
removes that.
