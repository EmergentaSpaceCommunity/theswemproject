# ExecPlan 015 — A Workbench on a server of one's own

**Status:** active (begun 2026-09-29). Roadmap S1. ADR-0011.

## Outcome

A person starts the Workbench on a server, at a name, and comes to it from their laptop and
their phone with a passkey. Nobody else comes in: not to the page's routes, not to a terminal,
not to where Apps are drawn. They read who may come in and what was done, take a device away,
and give a program a token that says what it may do.

## Acceptance scenario

**Given** a Workbench started with `swem workbench serve --at https://<name>` and either a
certificate of its own or a proxy in front, **when** it starts for the first time, **then** it
prints a word that is used once. **When** a person opens the address, gives the word and
registers their device, **then** they are shown codes to come back with, once, and the
Workbench opens. **When** they open the address in a browser that has nothing, **then** they
are asked to sign in, and every route under `/api` refuses them. **When** they sign in with
the passkey, the Workbench opens. Under Settings, Access they add a second device with a word
made for it, take a device away, make a token and withdraw it, and read what was done and
what was refused. **When** the command is asked to listen beyond this machine without TLS or
without an address, **then** it refuses, in words.

## Current state (2026-09-29)

- The command binds `127.0.0.1` only; `Assembled::serve` prints `http://127.0.0.1:<port>/?token=`.
- One place lets a request in: `route_shell` (`workbench_shell.rs`), by the page's own origin
  (`from_the_workbenchs_own_page`) and the secret of the run (`carries_the_secret`). The cookie
  is `SameSite=Strict; HttpOnly`, without `Secure`.
- The second listener, where Apps are drawn, is bound on loopback and its address is written
  as `http://127.0.0.1:<port>`; what it serves is opened by the id of a connection.
- Both listeners speak HTTP/1 without TLS. Nothing in the build speaks TLS.
- A provider's key is a file 0600 in a directory 0700 (`keys.rs`), and so are a profile's
  secrets. The values of a declared server's environment and headers are written with no mode
  set (`workbench_shell/mcp_servers.rs`).
- Every route takes the owner for whoever asks (`chat_ledger::owner`).

## Relevant product journey

GJ-14.

## Legacy evidence

The reference service had a principal with scopes, tokens for programs kept hashed and shown
once, and the browser's session behind a port that named no provider of identity. Kept: the
principal, the tokens. Replaced: the provider of identity, by passkeys the Workbench checks
itself, because a Workbench of one's own has nobody else to ask.

## Constraints

- ADR-0011. The guard by origin stays.
- Passkeys by `webauthn-rs` (0.5, stable), TLS by `rustls`. Nothing of the ceremony or of the
  cryptography is written here.
- `webauthn-rs` 0.5 links the system's OpenSSL. Its next version does not; the dependency is
  moved when that version is released as stable.
- No id, attribute or control exists on the page for a walk's sake.
- The screens follow the boards "Served at an address: sign in", "the first start", "codes to
  come back with" and "Settings: access" of the design.

## Plan

1. **The book of who may come in** (`access.rs`): devices and their passkeys, words used
   once, sessions, codes to come back with, tokens and what each may do, what was done, and
   the limit on tries. Proved below the page with a software authenticator.
2. **Served at an address**: the principal found in `route_shell`; the routes of sign-in;
   the session's cookie; TLS of its own or a proxy in front; the second origin at an address;
   the command's flags and its refusals.
3. **The page**: sign in, the first start, codes; Settings, Access.
4. **What is kept is kept closed**: modes on every file that holds a value; the keeper of
   secrets as a seam.
5. The gate's walk with a virtual authenticator; by hand; tried from outside.

## Progress

- [ ] 1 the book of who may come in
- [ ] 2 served at an address
- [ ] 3 the page
- [ ] 4 kept closed
- [ ] 5 the walk, by hand, from outside

## Discoveries

## Decision log

- 2026-09-29: a certificate is given as files, or a proxy stands in front. Getting one by
  itself (ACME) waits for a server with a name to try it on: a door is not given code that
  was never run.
- 2026-09-29: `--at http://localhost:<port>` is allowed and listens on this machine alone: it
  is sign-in tried where a browser takes `localhost` for a safe place. Any other address is
  `https`.

## Validation

## Outcome and remaining gaps
