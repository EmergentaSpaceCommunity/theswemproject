# Capabilities

| Capability | Status | Evidence |
|---|---|---|
| Install an agent from the plan the page shows; any id the ACP registry lists | proven | GJ-01; `swem agents install <registry-id>` walked by `a_person_installs_an_agent_the_product_never_heard_of` |
| Agent settings in parts: what it is put together from, its name and handle, what it is for, where it works, servers, skills, how it asks, how it signs in | proven | GJ-02; the gate's walks press every part but the name and the two folders, which were done by hand |
| A provider's key given once and handed to every agent that answers from it | proven | GJ-10; `provider_keys`, `keys` |
| This machine as it was found; a container offered only where one can start | proven below the page and by hand | GJ-10; `hosts` |
| Store over the ACP registry and added catalogs; servers and skills installed and used | proven | GJ-03; `tests/store.rs` |
| Chats listed, read with each message under its sender, gone on with, kept across restarts | proven by hand and below the page | GJ-04 |
| Several agents at work at once; a turn stopped from the composer; a question answered after the page was closed | proven by hand and below the page | GJ-09 |
| Markdown, tables and coloured code in a chat; a rail of agents and chats; both themes | proven by hand | ExecPlan 005 |
| Agent in a Podman container | proven where Podman exists | GJ-05 |
| Editor door over ACP: same agent, files, resume, several windows | proven | GJ-06 |
| MCP Apps hosted in a sandboxed origin beside a chat, opened by a person or brought by a tool of the agent; calls relayed through the host | proven | `workbench_shell_apps_browser`, `workbench_apps_engine_probe_browser` |
| A form or a link an agent asks for, answered in the chat | proven | `workbench_shell_apps_browser` |
| A tool of a server that is a person's to call, asked and answered by forms in the panel of Apps | proven in the Cycle's repository | its walk of the composition App; this repository's fixture server asks nothing this way |
| Embedding through a product builder | done | `swem_host::product`, `examples/embed.rs`, `tests/product.rs`; the product binary is the builder's first caller |
| Client credentials for an embedder's own page | missing | roadmap C2 |
| Store dependencies and the MCP registry's `server.json` as an index | missing | roadmap C3 |
| A server's home App as a space, the same App when a person comes back to it | proven | `workbench_shell_spaces.rs`, GJ-08's walk; the host draws no space for a server |
| What an App says a person is looking at, given to the agent | proven below the page | `chat_runtime`; `ui/update-model-context`, text and resource links; no browser walk of its own since the page was rebuilt |
| Projects | the project server's | the hub of a `swem-cycle` installed from the Store, beside the binary or on PATH is declared as a server; its space and its projects are its own |
