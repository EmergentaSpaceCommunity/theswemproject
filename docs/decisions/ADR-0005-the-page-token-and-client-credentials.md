# ADR-0005 — The API answers the page's own token, and a client credential an embedder mints

**Status:** superseded by ADR-0011 (2026-09-29) before it was built: a program is given a
token that says what it may do.

## Context

The Workbench's HTTP API is guarded by the page's own origin and a per-run secret the page is
handed once in its address. An application that embeds the harness and draws its own page on
another origin cannot reach the API at all.

## Decision

Beside the page token, the state holds client credentials an embedder mints for a named origin. A
request carrying such a credential as a bearer, from that origin or from no browser origin, is
admitted with the same rights as the page, and the response names that origin alone in its CORS
headers. The page's own guard is unchanged; nothing widens it. Credentials are per run and in
memory; OAuth is not this decision.

## Consequences

An embedder's page drives a session with one header. A third origin, or the page token used as a
bearer, is refused.
