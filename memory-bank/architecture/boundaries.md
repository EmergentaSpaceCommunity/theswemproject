# Boundaries

| Boundary | Holds because |
|---|---|
| The harness names no domain | `tests/genericity.rs`; `cargo tree -p swem-host -e normal` names only `swem-host` |
| The wire is the protocols' | ACP to agents, MCP to servers, MCP Apps for surfaces, Agent Skills for what an agent reads; nothing SWEM-specific travels on them |
| Consent to a plan by id | every install shows what it fetches from which index and applies only the plan the person confirmed |
| Secrets never reach a record | a profile's secrets live in its vault and are injected at launch; a declaration's values are never listed back |
| The page holds no domain truth | what the page shows of a server is read from that server at the revision it names |
| A walk presses what a person presses | the product gate drives the real binary in a real browser through real controls |
| Indexes are consumed, never hosted | the Store reads the ACP registry and catalogs others publish; there is no registry backend |
