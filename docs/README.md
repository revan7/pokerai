---
type: index
status: current
date: 2026-09-17
supersedes: none
related:
  - INDEX.md
  - CONVENTIONS.md
  - ../CLAUDE.md
---

# Documentation

This directory contains all project documentation, organized by type and purpose. The project is documentation-only right now (see `../CLAUDE.md` section 1) — `bench/` and `notes/` below are planned destinations, not yet created.

## Quick Navigation

Start here and follow this reading order:

1. **[INDEX.md](./INDEX.md)** — Master index of all documents with status and links
2. **The newest dated file in [log/](./log/)** — `YYYY-MM-DD.md` by filename; read it for the latest updates. `log/README.md` is the journal's format convention, not a dated entry, and is never "newest."
3. **[../CLAUDE.md](../CLAUDE.md)** — Project setup and agent coordination (Claude Code loads it automatically; Codex reaches the same rules through `../AGENTS.md`)
4. **The specific document you need** — linked from INDEX.md

## What's Here

- **[research/](./research/)** — Investigation findings, benchmarks, analysis, and review/audit reports (`REVIEW-*.md`, `AUDIT-YYYY-MM-DD.md`)
- **[design/](./design/)** — Architecture, module design, UI mockups; currently the orchestrator's design outline
- **[superpowers/specs/](./superpowers/specs/)** — Feature specifications
- **[superpowers/plans/](./superpowers/plans/)** — Implementation plans and task breakdowns
- **bench/** — *Planned.* Performance measurements and benchmark results; created when plan 4/5 tasks first write a report. No directory exists yet.
- **[log/](./log/)** — Daily append-only journal and progress tracking
- **notes/** — *Planned.* Human-authored notes (never edited by agents). No directory exists yet; nothing here today is human-authored notes.

## Key Documents

- **[CONVENTIONS.md](./CONVENTIONS.md)** — File naming, frontmatter, and documentation standards
- **[AUDIT.md](./AUDIT.md)** — Consistency-audit checklist: when to run it and what each check compares. It is the procedure, not results — a run's findings land in a dated `docs/research/AUDIT-YYYY-MM-DD.md` report instead.
- **[INDEX.md](./INDEX.md)** — Complete navigation index

## For Agents

See [CONVENTIONS.md](./CONVENTIONS.md) for:
- Frontmatter format for new documents
- Where each document type lives
- Revision and changelog procedures
- Cross-reference style guidelines
