# Capabilities

| Capability | Status | Evidence |
|---|---|---|
| Install an agent from the plan the page shows; any id the ACP registry lists | proven | GJ-01; `swem agents install <registry-id>` walked by `a_person_installs_an_agent_the_product_never_heard_of` |
| Agent setup: providers, model, role, skills, keys, servers, environment, permissions | proven | GJ-02 |
| Store over the ACP registry and added catalogs; servers and skills installed and used | proven | GJ-03; `tests/store.rs` |
| Sessions listed, resumed, kept across restarts | proven | GJ-04 |
| Agent in a Podman container | proven where Podman exists | GJ-05 |
| Editor door over ACP: same agent, files, resume, several windows | proven | GJ-06 |
| MCP Apps hosted in a sandboxed origin, calls relayed through the host | proven | `workbench_shell_apps_browser` |
| Embedding through a product builder | done | `swem_host::product`, `examples/embed.rs`, `tests/product.rs`; the product binary is the builder's first caller |
| Client credentials for an embedder's own page | missing | roadmap C2 |
| Store dependencies and the MCP registry's `server.json` as an index | missing | roadmap C3 |
| A server's home App as a space | proven | `workbench_shell_spaces.rs`, `workbench_shell_spaces_browser.rs`; the Project space as the project server's App is owed (roadmap C4b, C4c) |
| Project creation | partial | through the hub of a `swem-cycle` installed from the Store, beside the binary or on PATH; the Project space is still drawn by the host until C4 |
