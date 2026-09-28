# Runtime topology

```text
swem (product binary)
  `-- swem-host: WorkbenchShellState
        +-- profiles (<data>/profiles), the ledger of chats (routes.jsonl; routes.jsonl.v1 is
        |     the copy made before it became one)
        +-- chat runtime: one worker per agent that is owed a turn; a lock per agent beside
        |     the ledger (<ledger>.turns/<agent>.lock) shared with the editor door's process
        +-- installer and Store (<data>/installed, <data>/indexes)
        +-- MCP catalogue (<data>/mcp-servers), model providers (<data>/model-providers)
        +-- environments: this machine, or a Podman container per profile
        +-- HTTP surface for the page (127.0.0.1:<port>, per-run token); one stream per page
        +-- sandbox origin for MCP Apps (127.0.0.1:<sandbox port>)
        +-- editor door: `swem acp --profile <id>` over stdio
        `-- declared servers, one child process each, dialled when needed
              +-- for an agent session that attaches them
              `-- for a space: a server that offers a home App (the Cycle hub, when the
                  product declares one, serves its projects behind that one declaration)
```

Data lives under `~/.local/share/swem/workbench` (`$XDG_DATA_HOME`, `%LOCALAPPDATA%\SWEM`).
