# ExecPlan 010 — An agent's files, and a machine of its own

**Status:** completed (2026-09-28) for its first part; its second part is taken up by a plan of
its own once a container can be started on the computer it is done on. Roadmap B1;
`product/agents.md` (Hosts).

## Outcome

Two parts, each something a person does.

**Part one, files.** A person opens an agent's **Files** and sees the folder it works in as a
tree. They open a file and read it with its syntax coloured, change it and save it; when the
agent changed it meanwhile they are told and choose. They make a file and a folder, rename and
remove. What they handed over and what the agent handed back are there as before.

**Part two, the machine.** A person puts an agent in a container on this computer. The
container is the agent's own and stays: its engine, its sign-in and what it installed are there
the next day. Its key reaches the engine without appearing in a command line. Its terminal and
its files are inside that machine.

## Acceptance scenario

**Part one. Given** the owner's data root with an agent that has worked in a repository,
**when** the person opens its Files, **then** the tree shows that folder; **when** they open
a source file, change a line, press Save and ask the agent in the chat what the line says,
**then** the agent reads the changed line. **When** the agent rewrites the file while the
person has it open changed, and the person saves, **then** the page says it changed since and
saves nothing until they choose.

**Part two. Given** Podman set up on this computer, **when** the person chooses "In a container"
for an agent with a model provider whose key was given, consents to what is put in place and
writes to it, **then** the agent answers; `whoami` in its terminal answers from inside; its
Files show what is inside; the key is in neither `ps` nor `podman inspect`; after the
Workbench is restarted the same container is used.

## Current state (2026-09-28)

- Files shows two lists, inbox and outbox, read by `tokio::fs` from the workspace on this
  machine (`workbench_files.rs`); nothing else of the workspace is shown and nothing is edited.
- A container is made for each connection and removed after it; the workspace is mounted from
  this machine; no key and no network reach it.
- Podman on this computer has no machine: QEMU does not start until Podman's installer is run
  by the owner. Part two cannot be done by hand until then.

## Relevant product journey

GJ-05 (an agent in a container) is rewritten by part two. Part one adds a journey of its own.

## Legacy evidence

The reference showed an agent's files as a tree with an editor, read from inside its machine
(`product/agents.md`, Hosts). Nothing of it is code here.

## Constraints

- One way to reach an agent's files, whatever its machine: requests the runner answers
  (`swem-runner`, its `fs`). On this machine the library answers in the product's own process;
  in a container the same requests go to the runner inside. No second implementation for the
  container.
- Nothing is read or written outside the folder the agent works in: a name that climbs out and
  a link that leads out are refused by the runner, not by the page.
- Nothing variable crosses a shell; a key never appears in a command line.
- The page is styled by the kit only. The tree and the editor are mature libraries
  (`headless-tree`, CodeMirror 6).
- Isolation is what the host's record says, never what a probe finds at open.

## Plan

Part one:

1. `crates/swem-runner`: the library's `fs` - list, read, write with what was read, make a
   folder, rename, remove - confined to a root; the binary's `fs` answering one request on its
   standard streams.
2. The shell: an agent's files through the runner; routes for the tree, a file, its change.
3. The page: Files as a tree and an editor; what was handed over and back beside them.
4. By hand on a copy of the owner's data root with the real engine; suites; the gate.

Part two (after Podman is set up here):

5. The runner's `exec`, `probe`, `idle`, `reap`; static builds for Linux.
6. `host/`: a reach, this machine's, and a container's over it (`PodmanDialect`); the record of
   hosts; a container per agent that stays.
7. Engine set-up inside: the plan with the machine's platform, pushed and checked; the key in
   the header of `exec`; the terminal and the files through the reach.
8. What is deleted: the container per connection, its cleanup, `--container-image`.
9. By hand; suites; the gate; documents.

## Progress

- [x] 1 the runner's `fs`
- [x] 2 the shell's routes
- [x] 3 the page
- [x] 4 part one by hand, suites, gate
- [x] 5 the runner's `exec`, `probe`, `idle`; static builds for Linux (`reap` with the container)
- [ ] 6-9 part two: not begun here; a plan of its own takes it up

## Discoveries

- "Written by Ada 4 minutes ago" on the board cannot be said: a file says when it was changed
  and not by whom. The page says "Changed 4 minutes ago".
- The page's bundle was 2.56 MB before this plan and is 3.29 MB with the tree and the editor
  (eight languages). Both are over the 2.5 MB the plan of 2026-09-27 named. It is one file
  because it is embedded in the binary; what it costs is the first load, and it is served
  compressed. What to drop first when it matters: languages of the chat's code colouring.
- The walks of the gate that read what was handed over and back find it by attributes the old
  panel had (`data-files-area`, `data-file-name`, `#files-refresh`). They are kept on the new
  page so the walks stand; they are debt of the same kind ExecPlan 006 names.
- What the container path is today, for part two (read 2026-09-28):
  - A container is created per connection and per handshake (`prepare_podman_lease`,
    `podman create` then `start --interactive --attach`), inspected while created and not
    running, and removed by `PodmanCleanupGuard`; the evidence of eight guarantees comes from
    that inspection, and `environment/terminal` in the ledger from the removal.
  - The engine is the container's entrypoint; its environment is fixed at create and checked
    exactly; `session.rs` refuses any environment given later to an engine that is not on this
    machine. No key reaches a container: the resolver sets no secrets.
  - The terminal of an agent in a container runs on this machine with the agent's home as
    HOME. Files, the instructions and what is handed over name this machine's paths.
  - The image is whatever `--container-image` or `SWEM_CONTAINER_IMAGE` names, pinned by
    digest and never pulled by the product; the plan to acquire one is reached from the
    command line only.
  - The hermetic tests stand on `swem-podman-fixture`, which knows create, start, rm, inspect
    and exists, and no `exec`. The gate's walk of a container asserts that no container of
    ours is left after the agent is put to sleep: a container that stays changes that walk.
  - `run_personal_agent_in_podman` and `PodmanCredentialLease` serve one ignored live test.

## Decision log

- 2026-09-28: the page was first made without the board "Agent: files" of the approved
  design, which the copy at hand lacked, and was then brought in line with it: Upload and New
  file in words, "From you" and "From" the agent above the tree, where the workspace is said
  under it, a file opened to be read with Download and Edit. Rename, Remove, a new folder and
  the question before leaving what was not saved are not on the board and were kept.

- 2026-09-28: the header of `exec` is one line on standard input and the program replaces
  the runner, so there is one process, its streams are the ones the runner was given, and a
  signal reaches the program itself. For a program that is given a terminal the header is a
  file read once and removed, because a terminal echoes what is typed into it.
- 2026-09-28: the runner is linked for Linux by the toolchain's own linker
  (`scripts/build-runner.sh`). It is Rust alone, so no cross compiler is needed, and it is
  under one megabyte for either architecture.
- 2026-09-28: the runner does not reap what it started. `idle` is what a machine runs while
  no engine does, and the container is started with the engine's own init, which reaps.
  Ending what an engine left behind needs a call the runner cannot make without unsafe code
  or a dependency; it is decided with the container, where it can be tried.

- 2026-09-28: part one first. It does not need a container, it is what a person misses on the
  page today, and it makes the one way to reach files that part two sends into the machine.

## Validation

Part one:

- `swem-runner/tests/fs.rs` (5): a folder listed folders first and a file read; nothing outside
  the folder reached by a name that climbs or a link that leads out, read or written; a file
  written only while it is what was read, and what it allowed kept; made, renamed, removed; the
  program answering one request on its streams.
- `swem-host/tests/agent_files.rs` (3): the same through the shell, with a file that changed
  meanwhile answered and not failed.
- `test/page-files.test.mjs` (4).
- By hand in Chrome on a copy of the owner's data root, the agent on the real engine, both
  themes: the tree opened folder by folder; a Rust file read coloured; a note changed and
  saved, and the agent asked in its chat for the note's last line answered with the line that
  was written; the note written by somebody else meanwhile - "It changed since you opened it.
  Nothing was saved." - and saved over by choice; leaving a changed file asked first; a file
  made in a folder, written, renamed and removed; a picture and a file whose name was taken
  brought from the person's machine - the second asked about and replaced - and the picture
  shown on the page, the same bytes on the disk.

Part two, what can be done without a container:

- `swem-runner/tests/exec.rs` (5), on macOS: what follows the header reaches the program byte
  for byte; a key given in the header is in the program's variables and in no process's
  command line, the program is the process the runner was, and only the variables that were
  to be kept are kept; a header in a file is removed once read; a program that is not there
  is said and nothing starts; the machine looked at from inside; `idle` stays.
- `scripts/build-runner.sh` gives two static programs for Linux, x86_64 and aarch64. They
  were built and not run: there is no Linux machine here.

## Outcome / remaining gaps

Part one is done 2026-09-28. A person reads and changes an agent's files on the page, and what
is asked about them is asked of the runner. The runner can start a program as a header says
and look at a machine from inside, and is built as one static program for Linux.

Remaining:

- Part two, the machine of an agent's own. It needs Podman able to start a machine on the
  computer it is done on; on the owner's that waits for Podman's installer, which asks for
  their password. What the container path is today is in Discoveries, for the plan that
  takes it up. Nothing of it was changed here.
- What was handed over and back is still read from this machine's disk by the harness itself
  (ADR-0008).
- The walks of the gate find what was handed over and back by attributes of the old panel.
- The page's bundle is 3.29 MB.
- A file bigger than two megabytes is not opened on the page; it is downloaded.
- The static programs for Linux were built and not run.
