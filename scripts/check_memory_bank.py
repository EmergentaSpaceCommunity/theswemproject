#!/usr/bin/env python3
"""Small structural guardrail for SWEM's repository memory.

This intentionally checks only stable, high-value invariants. It must not grow into a new paper
ontology or a giant documentation linter.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MB = ROOT / "memory-bank"

REQUIRED = [
    ROOT / "AGENTS.md",
    ROOT / "ARCHITECTURE.md",
    MB / "README.md",
    MB / "product" / "journeys.md",
    MB / "product" / "capabilities.md",
    MB / "architecture" / "boundaries.md",
    MB / "roadmap" / "roadmap.md",
    MB / "exec-plans" / "PLANS.md",
    MB / "memory" / "lessons.md",
    MB / "decisions" / "README.md",
]

errors: list[str] = []

for path in REQUIRED:
    if not path.is_file():
        errors.append(f"missing required memory file: {path.relative_to(ROOT)}")

agents = ROOT / "AGENTS.md"
if agents.is_file():
    line_count = len(agents.read_text(encoding="utf-8").splitlines())
    if line_count > 200:
        errors.append(f"AGENTS.md is {line_count} lines; keep the global map <= 200 lines")

active_dir = MB / "exec-plans" / "active"
if active_dir.exists():
    active = [p for p in active_dir.glob("*.md") if p.name.lower() != "readme.md"]
    if len(active) > 1:
        errors.append(
            "more than one active ExecPlan: " + ", ".join(p.name for p in sorted(active))
        )

# Every roadmap item should say which journey(s) it serves.
roadmap = MB / "roadmap" / "roadmap.md"
if roadmap.is_file():
    text = roadmap.read_text(encoding="utf-8")
    chunks = re.split(r"(?m)^## (?=[A-Z]\d+\b)", text)[1:]
    for chunk in chunks:
        title = chunk.splitlines()[0].strip()
        if "Related journeys:" not in chunk:
            errors.append(f"roadmap milestone lacks 'Related journeys:': {title}")

# Golden journey identifiers used in current docs should resolve to an actual GJ heading.
gj_file = MB / "product" / "journeys.md"
if gj_file.is_file():
    gj_text = gj_file.read_text(encoding="utf-8")
    known = set(re.findall(r"(?m)^## (GJ-\d+)\b", gj_text))
    if not known:
        errors.append("no GJ-* headings found in journeys.md")
    for path in [roadmap, MB / "product" / "capabilities.md"]:
        if not path.is_file():
            continue
        refs = set(re.findall(r"\bGJ-\d+\b", path.read_text(encoding="utf-8")))
        unknown = refs - known
        if unknown:
            errors.append(
                f"{path.relative_to(ROOT)} references unknown journeys: {sorted(unknown)}"
            )

# ADR file names should remain stable and readable.
adr_dir = MB / "decisions"
if adr_dir.exists():
    for path in adr_dir.glob("ADR-*.md"):
        if not re.match(r"ADR-\d{4}-[a-z0-9-]+\.md$", path.name):
            errors.append(f"invalid ADR filename: {path.name}")

if errors:
    print("memory-bank check FAILED", file=sys.stderr)
    for error in errors:
        print(f"- {error}", file=sys.stderr)
    raise SystemExit(1)

print("memory-bank check OK")
