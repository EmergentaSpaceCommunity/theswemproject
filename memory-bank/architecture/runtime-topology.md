# Runtime topology

```text
swem (product binary)
  `-- swem-host: WorkbenchShellState
        +-- profiles (<data>/profiles), sessions (routes.jsonl, routes.sqlite3)
        +-- installer and Store (<data>/installed, <data>/indexes)
        +-- MCP catalogue (<data>/mcp-servers), model providers (<data>/model-providers)
        +-- environments: this machine, or a Podman container per profile
        +-- HTTP surface for the page (127.0.0.1:<port>, per-run token)
        +-- sandbox origin for MCP Apps (127.0.0.1:<sandbox port>)
        +-- editor door: `swem acp --profile <id>` over stdio
        `-- attached servers, one child process each, dialled when needed
              `-- a project server (swem-cycle), when installed
```

Data lives under `~/.local/share/swem/workbench` (`$XDG_DATA_HOME`, `%LOCALAPPDATA%\SWEM`).
