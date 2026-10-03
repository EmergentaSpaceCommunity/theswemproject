# Publishing a catalog

Everything the Workbench can be given is an index it reads. Nothing is hosted by SWEM: every
index is consumed, never served, and what you publish is yours, under any licence you like
(`LICENSE-EXCEPTION.md`).

- **Agents** come from the ACP registry (`https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`,
  or the mirror `SWEM_ACP_REGISTRY_INDEX` / `--acp-registry` names).
- **MCP servers, skills and packages of other kinds** come from a catalog, a `swem:catalog@0.2`
  document you publish anywhere and a person adds by address (`@0.1` still reads):

```json
{
  "schema": "swem:catalog@0.2",
  "name": "My tools",
  "entries": [
    {"kind": "server", "id": "notes", "name": "Notes", "version": "1.2.0",
     "description": "the notes an agent keeps", "env": ["NOTES_TOKEN"],
     "distribution": {"npx": {"package": "@example/notes-mcp@1.2.0"}}},
    {"kind": "server", "id": "search", "name": "Search", "version": "0.4.0",
     "distribution": {"binary": {"darwin-aarch64": {"archive": "https://…/search-darwin-aarch64.tar.gz",
                                                     "sha256": "…", "cmd": "./search"}}}},
    {"kind": "skill", "id": "review", "name": "Review", "version": "1.0.0",
     "distribution": {"archive": {"url": "https://…/review.tar.gz", "sha256": "…"}},
     "requires": [{"kind": "skill", "id": "manners"}]},
    {"kind": "swem.cycle/package@1", "id": "hello-node", "name": "Hello node", "version": "0.2.0",
     "distribution": {"archive": {"url": "https://…/hello-node.tar.gz", "sha256": "…"}}}
  ]
}
```

A server is an npm package run through node, a Python package run through `uv`, or a native
archive per platform, checked against its digest; a `binary` whose `archive` is neither a zip
nor a tarball is one bare executable, placed as its `cmd`; a skill is an archive holding a
`SKILL.md`; a tool (`"kind": "tool"`) is a program other packages need and are handed the path
of, checked by its digest alone; a channel (`swem/channel@1`) and a tunnel (`swem/tunnel@1`)
are programs checked against their shape when installed (`docs/channels.md`); a
package of another kind is an archive for whoever takes that kind - a server whose entry says
`takes`, as the Cycle's hub takes `swem.cycle/package@1`, or the product the harness is built
into. A kind nobody here takes is listed as such and cannot be installed. What an entry
`requires` is installed with it under one consent. Every install is planned, shown, consented to
by its exact plan id, and receipted under the install root; a newer version in an index is offered
as an update, and what was installed can be removed.

A server that takes packages for itself says so with `takes` in its entry, and a product that
builds the harness in takes kinds of its own: `docs/building-in.md`, "The Store: kinds and
takers".
