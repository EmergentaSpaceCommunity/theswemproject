# ADR-0019 — Hosts of one person trust each other by key

**Status:** proposed (2026-10-07; rewritten 2026-10-08). Nothing of it is built; no crate is
linked for it.

## Context

A person runs SWEM in more than one place: a laptop, a server, a machine in a cloud. Today
each is a Workbench of its own with its own agents, chats, packages and journal, and the page
of one knows nothing of another. ADR-0011 admits a page to one host with a passkey; ADR-0014
says a person is known to each served harness by a passkey registered there, and that no key
travels; ADR-0015 left peer identity by key for later.

The first form of this decision said the hosts of one person trust each other "by one key". One
key on every host is one secret copied to every machine: whoever takes it from any host has
them all, and no host can be withdrawn without withdrawing the rest. Nothing a person is
required to keep runs a machine while they are away, either: a passkey signs only when the
person is present, and it never leaves its device. So a host that answers a messenger at night
holds a secret of its own, like a machine in Tailscale, Syncthing or under an SSH certificate
authority; what is decided here is whose that secret is, what it may do, and how it is taken
away.

## Decision

- **A host is any installation of SWEM**, with its own agents, chats, servers, packages and
  journal. Nothing is shared by default: an agent sees the Apps, servers and environments of
  its own host, and isolation is set at the host. What a plugin requires is installed on each
  host that runs it; the Store plans per host (ADR-0018).
- **The person is the root, and the root is their presence.** What makes a host theirs is an act
  on a page they are signed in to with a passkey (ADR-0011). There is no key of the person's
  that a host keeps, and no "root key" to store or to lose: when every device is lost, the codes
  to come back with of each host let them in, as today, and the hosts are introduced again.
- **Each host has a key of its own**, made on it at its first start, never leaving it, kept
  under the host's data as its other secrets are (sealed where the machine gives a keeper,
  in the device's keychain or TPM when that is built, honestly "at rest" and never "no key").
  The key is the host's identity on the wire; its public half, shortened, is the host's
  fingerprint a person reads.
- **Hosts of one person are the hosts they introduced.** A host that is already theirs
  introduces a new one: the new host shows a short fingerprint and a word said once, the
  person gives the word on the page of the host that is theirs, the two hosts meet, both
  show the same fingerprint, and the person confirms on the page. The introducing host then
  signs the new host's public key with the person's name, the host's name, what it may do
  and until when; the new host keeps that signature beside its key. No secret travels: the
  word opens the meeting once, the signature is over a public key.
- **Trust is the signature, checked at every meeting.** A host admits another when it shows a
  key signed by a host the first one trusts, within its time, and not withdrawn. A signature is
  for weeks and renewed over a live link; a host not seen for longer than its time falls out by
  itself, so withdrawal needs no list every host must agree on. "Forget this host", on the page
  of any host of the person's, withdraws it from the rest at once.
- **A page signed in to one host sees them all.** The browser reaches one host over HTTPS with
  a passkey, as ADR-0011 has it; that host reaches the others as a peer, and the page shows
  their agents and chats under each host's name. A passkey may be registered on several hosts
  for coming to each directly, and need not be.
- **Host to host is iroh, and only iroh**: identity is the key, NAT is crossed without a
  server of ours, the relays of n0 by default (free, rate-limited, for development and hobby
  use by their own words) and one's own `iroh-relay` when wanted; a host behind a home router
  is reached by its peers, which nothing else gives. The signature above is checked after
  iroh's own handshake, over the key iroh already authenticated. Nothing SWEM-specific goes
  on the wire beyond MCP and ACP carried over it. The browser is not a peer: a page is served
  over HTTPS and reaches the other hosts through the host it is signed in to.
- **Sessions, codes and tokens belong to a host** (ADR-0011) and never travel between hosts.
- **A host that sleeps is woken by a package**: a keeper at the host's side knocks (ADR-0015),
  not the core. A peer cannot wake a sleeping machine, and a machine that sleeps when no
  request reaches its address (a sprite) drops its peer link as it sleeps; a knock at its
  address wakes it. A sleeping host tells its peers where to knock, and they knock before
  they dial.

## Consequences

When built: the harness links iroh, under Apache-2.0/MIT; tests and the gate never reach a
public relay. The limits of the public relays are not measured. Chats across hosts are not in
this decision. The first vertical step is a host with a face (its name and fingerprint on
Settings, Access), then "Add a host" between two `swem` processes on one machine, then the
page of one showing the agents of the other. Until built, the Workbench serves one host, as
today.

## Links

- ADR-0011, ADR-0014, ADR-0015, ADR-0018; `ROADMAP.md`, "Hosts of one person".
