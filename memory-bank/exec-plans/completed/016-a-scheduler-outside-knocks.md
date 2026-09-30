# ExecPlan 016 — A scheduler outside knocks

**Status:** completed (2026-09-29). Roadmap S3, its first part. ADR-0010.

## Outcome

A person whose Workbench is served at an address gives a scheduler of theirs - a crontab on
another machine, a service that calls an address on time - an address to knock on and a token
that may do nothing else. When it knocks, the Workbench looks at what is due and says it.
Under Providers, Time they read that it was added, when it knocked last and what was found.

## Acceptance scenario

**Given** a Workbench served at an address and an agent with a schedule that is due, **when**
a person adds a call from outside under Providers, Time, **then** they are shown the address
to knock on and a token, once, with a line for a crontab. **When** something knocks with that
token, **then** what is due is said in its chat, once, however many times it knocks; the
page says when it knocked last and how many messages were said. **When** the same token asks
for anything else, it is refused. On a Workbench that is on one computer alone the page says
that nothing outside reaches it and how to serve it at an address.

## Current state (2026-09-29)

- The door lets a token that may say what is due through to `POST /api/time/due` and to
  nothing else (ExecPlan 015); nothing answers there.
- Providers, Time shows a third keeper, "An outside scheduler", that cannot be added.
- The running Workbench looks every five seconds and claims what is due for every agent,
  keyed by the schedule and the time it was due (`claim_due`), so a second look finds
  nothing.

## Relevant product journey

GJ-11, GJ-14.

## Legacy evidence

The reference service kept the clock on the platform, which was always on. Here a Workbench
may sleep where it runs; what wakes it is a call, and the call carries nothing.

## Constraints

- ADR-0010: a keeper tells a harness to look; it holds no words, no key, no machine's token.
- A call from outside is not chosen for an agent: whoever knocks has the Workbench look for
  every agent. It is a keeper of the Workbench.
- The token is made by the book of who may come in and is withdrawn under Settings, Access.

## Plan

1. The look at a knock (`timekeeper.rs`), the route, the third keeper as it stands.
2. The page: add, what to give the scheduler, when it knocked last.
3. Below the page; the gate's walk of coming in gains the knock; by hand with a crontab's line.

## Progress

- [x] 1 the look at a knock
- [x] 2 the page
- [x] 3 proved

## Discoveries

- While the Workbench runs it looks every five seconds by itself, so a knock finds what is
  due only when it comes first. What a knock is for is a Workbench that does not run until it
  is called. That cannot be shown on this machine: nothing here starts a program because an
  address was called.

## Decision log

- 2026-09-29: a scheduler outside is added by making its token, and is "added" for as long
  as a token that may knock is there. Nothing else is kept of it: the Workbench does not
  know what knocks, and need not.
- 2026-09-29: the first kind is whatever can call an address with a header. Kinds that set
  their own times through a provider's API come as packages of the Store (roadmap S5); for
  a Workbench that sleeps they matter, because every knock wakes it.

## Validation

- `chat_runtime::a_knock_has_what_is_due_said_once`: before anything is due a knock finds
  nothing; at the moment two schedules of two agents are due it has both said; a second
  knock at the same moment has nothing said; both are answered though the knock waited for
  neither; when it last had something said is kept; on one computer alone the keeper is
  shown as one that cannot be reached.
- The gate's walk of coming in: the token a person made knocks and is answered; nobody's
  knock is refused; the token opens nothing else.
- By hand in Chrome at `http://localhost`, signed in: Providers, Time before and after, in
  both themes; Add shows the address, the token and the line; `curl` with the token twice,
  with the token at another route (403), with nothing (401); the row says when it knocked
  last and that nothing was due.

## Outcome and remaining gaps

A scheduler outside can be given an address and a token, and its knock is answered. Not
done: a Workbench that sleeps where it runs, woken by the knock (it needs a machine whose
provider starts a service when its address is called); a scheduler that knocks only when
something is due.
