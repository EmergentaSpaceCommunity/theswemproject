# ADR-0011 — Who may come in

**Status:** accepted (2026-09-29, roadmap S1). Supersedes ADR-0005, which was never built.

## Context

The Workbench listened on the machine it runs on and nowhere else. It minted a secret for each
run and handed it to the person in the address it printed; its page was admitted by its own
origin and that secret. That is a door for one person at one computer.

A person who puts the Workbench on a server of their own, to reach it from a laptop and a
phone, had no way to do it that did not leave their agents, their terminals and their files
open to whoever found the address. The command could not listen elsewhere; the library would
have let an embedder listen anywhere with nothing but the secret of the run.

## Decision

- **Who asks is known before anything is answered.** Every request that is let in has a
  principal: the participant who asks, and by what they came - the page of this run, a signed-in
  session, a token, or the word of the product the harness is built into. There is one place
  where this is found out.
- **Two ways to serve.** On this machine, as before: the secret of the run. At an address: only
  over TLS - a certificate of its own or the person's proxy in front, which has to be said -
  and only with sign-in. The command refuses anything else, in words. An address is a name: a
  passkey belongs to a name.
- **The owner claims it once.** A Workbench started at an address for the first time prints a
  word that is used once. Whoever comes with it registers a device and is the owner.
- **Passkeys, not passwords.** The private key stays in the person's device; the server keeps
  the public one. A second device is added from one that is signed in. Words to come back
  with are shown once, kept hashed, and each is used once. Nothing of this is of our own
  making: it is WebAuthn as browsers carry it and a maintained library.
- **A session** is a cookie the page cannot read, sent only over TLS and only to this name,
  kept hashed on the server, ended from the page. Tries to come in are limited.
- **Tokens for programs**: shown once, kept hashed, each saying what it may do, withdrawn from
  the page. A token that may only say "look at what is due" opens nothing else.
- **What is kept is kept closed.** Every file that holds a value a person gave is readable by
  them alone. Where the machine gives the service a key from outside the data, values are
  sealed under it. No key is laid beside the data to look like protection. A product that
  builds the harness in supplies its own keeper.
- **What was done is written down**: who came in, from where, by what, and what was refused.
- **The guard by origin stays.** It answers which page asks; sign-in answers who. Neither
  replaces the other.

## Consequences

Behind this door is a terminal. It is tried from outside, from a machine that has nothing,
before it is called done.

The second origin, where a server's Apps are drawn, is a door too and is served under the same
rules.

Several people in one chat, and an agent that serves a team, are not this decision. The
principal is what they will stand on.
