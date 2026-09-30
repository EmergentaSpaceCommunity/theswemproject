# Capabilities

| Capability | Status | Evidence |
|---|---|---|
| Install an agent from the plan the page shows; any id the ACP registry lists | proven | GJ-01; `swem agents install <registry-id>` walked by `a_person_installs_an_agent_the_product_never_heard_of` |
| A new agent in four steps; a look inside its machine before it is used | proven below the page, in the gate for the first two steps, and by hand | GJ-01; `looks`, every walk that makes an agent |
| An agent removed: retired in its chats, its profile set aside, its folder left where it is | proven below the page and by hand | GJ-02; `removal` |
| Agent settings in parts: what it is put together from, its name and handle, what it is for, where it works, servers, skills, how it asks, how it signs in | proven | GJ-02; the gate's walks press every part but the name and the two folders, which were done by hand |
| A provider's key given once and handed to every agent that answers from it | proven | GJ-10; `provider_keys`, `keys` |
| A schedule said on time in a chat, paused, forgotten; its runs kept, late ones said late | proven | GJ-11; `time`, `chat_runtime` |
| Time kept while the Workbench is closed, by the system's scheduler; a keeper chosen for an agent | proven on macOS by hand; systemd not checked; Windows not built | GJ-11; `chat_runtime::time_is_kept_once_for_the_agents_of_a_keeper`, `keepers` |
| A scheduler outside knocks on a Workbench served at an address; what is due is said once however often it knocks | proven below the page, in the gate and by hand with `curl`; a Workbench woken by the knock was not tried | GJ-11; `chat_runtime::a_knock_has_what_is_due_said_once` |
| An agent makes, lists, pauses and removes its own schedules | proven | GJ-11; `chat_runtime::an_agent_makes_its_own_schedule_in_the_chat_it_was_asked_in`; by hand with a real engine |
| An agent's files as a tree with an editor; saved only while it is what was opened | proven below the page and by hand | GJ-13; `swem-runner/tests/fs.rs`, `agent_files` |
| This machine as it was found; a container offered only where one can start | proven below the page and by hand | GJ-10; `hosts` |
| Store over the ACP registry and added catalogs; servers and skills installed and used | proven | GJ-03; `tests/store.rs` |
| Chats listed, read with each message under its sender, gone on with, kept across restarts | proven by hand and below the page | GJ-04 |
| Several agents at work at once; a turn stopped from the composer; a question answered after the page was closed | proven by hand and below the page | GJ-09 |
| A chat of several agents: named and answered, members brought in and taken out, the chain of replies held at the chat's limit | proven | GJ-12; `chat_runtime`, `chats.rs` |
| Markdown, tables and coloured code in a chat; a rail of agents and chats; both themes | proven by hand | ExecPlan 005 |
| Agent in a Podman container | proven where Podman exists | GJ-05 |
| Editor door over ACP: same agent, files, resume, several windows | proven | GJ-06 |
| MCP Apps hosted in a sandboxed origin beside a chat, opened by a person or brought by a tool of the agent; calls relayed through the host | proven | `workbench_shell_apps_browser`, `workbench_apps_engine_probe_browser` |
| A form or a link an agent asks for, answered in the chat | proven | `workbench_shell_apps_browser` |
| A tool of a server that is a person's to call, asked and answered by forms in the panel of Apps | proven in the Cycle's repository | its walk of the composition App; this repository's fixture server asks nothing this way |
| Embedding through a product builder | done | `swem_host::product`, `examples/embed.rs`, `tests/product.rs`; the product binary is the builder's first caller |
| A Workbench served at an address: the first start by a word used once, a device registered with a passkey, codes to come back with, a second device by a word, a device taken away, tokens for programs that say what they may do, what was done and refused | proven on this machine, at `localhost` and at a name over TLS with a certificate made for the trial; not yet done on a server with a name of its own, nor with a phone | GJ-14; `tests/access.rs`, `product_front_door::a_person_comes_to_their_workbench_from_elsewhere_with_a_passkey` |
| A certificate got by the Workbench itself (ACME) | missing | ExecPlan 015, decision log |
| A token for an embedder's own page on another origin | missing | roadmap S4; a token is made and checked, the answer names no other origin yet |
| Store dependencies and the MCP registry's `server.json` as an index | missing | roadmap C3 |
| A server's home App as a space, the same App when a person comes back to it | proven | `workbench_shell_spaces.rs`, GJ-08's walk; the host draws no space for a server |
| What an App says a person is looking at, given to the agent | proven below the page | `chat_runtime`; `ui/update-model-context`, text and resource links; no browser walk of its own since the page was rebuilt |
| Projects | the project server's | the hub of a `swem-cycle` installed from the Store, beside the binary or on PATH is declared as a server; its space and its projects are its own |
