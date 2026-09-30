# ExecPlan 015 — A Workbench on a server of one's own

**Status:** completed (2026-09-29) for what is done on this machine; what waits for a server
with a name and for a phone is the roadmap's, under S1. ADR-0011.

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

- [x] 1 the book of who may come in
- [x] 2 served at an address
- [x] 3 the page
- [x] 4 kept closed: the data root and the declared servers' values; the keeper of secrets as
      a seam is left to S4 (decision log)
- [x] 5 the walk, by hand on this machine, tried from outside with `curl`
- [ ] 6 on a server with a name of its own, from a laptop and a phone
- [ ] 7 the page at a phone's width

## Discoveries

- Chrome takes a passkey ceremony at a name whose certificate it was told to trust by its key
  (`--ignore-certificate-errors-spki-list`), so the whole of sign-in over TLS is walked on
  this machine with a certificate made for the trial and a name mapped to loopback.
- A device that holds passkeys is given to the browser over its debugging protocol
  (`WebAuthn.addVirtualAuthenticator`); the ceremony, the page and the host are the product's.
  Taken away and given again inside one page, the browser turned the next ceremony down; a
  second device is a second browser, which is what it is for a person too.
- Limiting tries with a passkey or a token would only let a stranger keep the owner out:
  neither can be guessed. What can be guessed - a word, a code - is limited, from one place
  and from everywhere.
- The secret of a run is still minted when served at an address and opens nothing there.

## Decision log

- 2026-09-29: a certificate is given as files, or a proxy stands in front. Getting one by
  itself (ACME) waits for a server with a name to try it on: a door is not given code that
  was never run.
- 2026-09-29: the keeper of secrets as a seam (`SecretKeeper`) is not made here. With one
  keeper it would be an abstraction nobody uses; it is made in S4, where a product that
  builds the harness in supplies a second. Here every file that holds a value is closed to
  others, the declared servers' values included, and so is the data root.
- 2026-09-29: what is written down of what was done is kept five thousand entries back, so
  that whoever knocks all day fills nothing up.
- 2026-09-29: `--at http://localhost:<port>` is allowed and listens on this machine alone: it
  is sign-in tried where a browser takes `localhost` for a safe place. Any other address is
  `https`.

## Validation

- `tests/access.rs` (7): a Workbench is made somebody's own once and they come back with
  their device; a device that was never registered does not come in; a second device by a
  word, and one taken away comes in no more; a code brings a person back once; a token opens
  what it says and nothing once withdrawn; tries that fail are limited and written down;
  nothing that opens is kept as it was said, and the book is its owner's alone. The
  ceremonies are done by a passkey kept in software and checked by the code that checks a
  real one.
- `product::at_an_address` (2): what is served, and the seven ways the way would be left
  open, each refused by its reason. `workbench_shell::door` (2): what anybody may ask, and
  what a code and a knock open.
- `tests/product.rs::what_a_person_gave_is_kept_closed_to_others`: the data root, the
  declared servers' folder, a declaration written earlier with a header in it, one made now.
- `test/page-door.test.mjs` (3).
- The gate, sixteen walks, all passed from a clean build (2026-09-29). The new one,
  `a_person_comes_to_their_workbench_from_elsewhere_with_a_passkey`: the first start, the
  codes, a token, signed out, the passkey, a browser that holds nothing refused, back with a
  code, that device registered, the first taken away; and from outside with `curl`, before
  anybody came in and with the token. An earlier run of the gate that day had fifteen of
  sixteen: the walk of a first agent lost its browser for a minute; alone it passed.
- By hand in Chrome at `http://localhost` and at `https://workbench.test:8443` with a
  certificate made for the trial: the same, with Settings, Access read in both themes. From
  outside with `curl` over TLS: the page answers anybody and tells the browser to come over
  TLS from now on; everything under `/api` answers 401 to who did not come in; another
  site's page 403; the secret of a run opens nothing; the fifth wrong word from one place is
  refused for a quarter of an hour; nothing but TLS 1.2 and 1.3 is spoken; where Apps are
  drawn has no `/api`.
- The certificate put anew into its files was served within a minute, with no restart.
- The command refused, each in its words: an address without `https`, a name with no
  certificate and no proxy, a proxy in front with no address for Apps, a proxy in front and
  a listener beyond this machine, an address with more than a name in it.
- With a proxy said to be in front, called as a proxy calls: where a request came from is
  what the proxy said last.

## Outcome and remaining gaps

What a person does on a server is built and was done on this machine. Not done where it
must run, and said so:

- on a server with a name of its own, from a laptop and a phone, with a passkey held by a
  real device;
- with a real proxy in front;
- an App opened at an address: that its origin answers over TLS was tried, an App was not
  opened there;
- on Linux: the harness was built and run on macOS only;
- a certificate got by the Workbench itself;
- the Workbench's own page at a phone's width: the door fits it, the rail and what is
  beside it were never drawn for it.
