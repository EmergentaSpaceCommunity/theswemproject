# ADR-0001 — The repository keeps its memory by kind of truth

**Status:** accepted; amended 2026-10-01: the working memory left the public repository.

## Context

A product built with coding agents accumulates plans, findings, decisions and debt faster than
any one document can hold. When all of it lives in one place, the newest paragraph wins and the
product specification, the architecture and the task list become one text nobody can trust.

## Decision

The repository carries a `memory-bank/` split by kind of truth: product journeys and capabilities,
architecture, decisions, the roadmap, one active ExecPlan at a time, and general lessons. The root
`AGENTS.md` is the operating contract and stays short. `scripts/check_memory_bank.py` enforces
the structure: required files exist, one active plan, roadmap items name their journeys.

## Consequences

Agents read progressively (contract, journey, roadmap item, plan) rather than everything. A fact
is recorded once, where its kind lives, and promoted from a plan only when it is durable.

## Amendment (2026-10-01)

When the repository was prepared for publication, the working memory - product journeys and
capabilities, the roadmap with its dates, the ExecPlans and the lessons - was moved to the
maintainer's private tree. A working diary is written for the people writing it; in a public
repository it is noise to a contributor and, worse, looks like documentation. What stays here is
what a contributor needs and what is durable: this folder of decisions (`docs/decisions/`), the
architecture (`ARCHITECTURE.md`, `docs/architecture/`), the public roadmap (`ROADMAP.md`, by
capability, without dates) and the operating contract (`AGENTS.md`). The structure check went
with the memory.
