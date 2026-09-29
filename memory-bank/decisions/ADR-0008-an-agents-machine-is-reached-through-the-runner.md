# ADR-0008 — An agent's machine is reached through the runner

**Status:** accepted (2026-09-28, roadmap B1, ExecPlan 010). Amends one sentence of ADR-0002.

## Context

The harness read an agent's files with its own file calls, opened its terminal with a local
pty and started its engine as a local process. About forty places assumed that the agent, its
files and its sign-in are on the machine the product runs on. An agent in a container was
reached through the folder mounted into it from this machine, which is why its terminal ran
outside it and no key could be handed to it. Hosts that are not this machine - a container that
keeps its own disk, a machine over SSH, one that sleeps elsewhere - cannot be reached that way
at all.

## Decision

What the product wants of an agent's machine is asked of **the runner** (`crates/swem-runner`),
and the runner answers the same way wherever the machine is.

- On the machine the product runs on, the runner's library answers in the product's own process.
- In a machine of the agent's own, the runner's program is put there and answers on its
  standard streams. What is asked is read from standard input, never from the command line, so
  nothing of an agent's appears in a list of processes.
- There is one implementation of each thing asked. The harness keeps no second way to do the
  same on this machine.
- The first thing asked is files (`fs`): what a folder holds, a file read, a file written while
  it is what was read, a folder made, a name changed, something removed. Every request names a
  root and a place under it; what climbs out of the root or leads out of it by a link is
  refused by the runner. The harness does not check that again and does not need to.
- The runner depends on nothing of the harness and names no domain. It is small enough to be
  built as one static program for a host.

ADR-0002 said that `cargo tree -p swem-host -e normal` names no other SWEM crate. It names one
now, `swem-runner`, which is the harness's own part. What ADR-0002 decided stands: the harness
links no domain crate.

## Consequences

An agent's Files is the folder it works in as a tree with an editor, and it is the same page
for an agent in a container once the requests are sent into the container. Starting an engine,
opening a terminal and looking at a machine follow the same way (`exec`, `probe`), in the part
of ExecPlan 010 that needs a container engine here.

What was handed over and back (`workbench_files.rs`, the inbox and the outbox) still reads this
machine's disk directly. It moves to the runner with the container that keeps its own disk;
until then both reach the same folder.

A file is read and checked in two steps, so an agent that swaps a file for a link between them
could be served what the link leads to. The agent is not the adversary this guards against on a
person's own machine; on a host shared with others the runner opens without following links.
