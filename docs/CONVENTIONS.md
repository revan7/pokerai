# Documentation Conventions

## Frontmatter for New Documents

All new documents include a YAML frontmatter block. Existing documents (anything committed before this convention took effect) are not retrofitted — this applies going forward, including to every doc committed since, so a document being "new" as of this rule's own commit does not exempt it.

```yaml
---
type: research | design | spec | plan | review | changelog | benchmark | log | index | convention
status: current | superseded | historical
date: YYYY-MM-DD
supersedes: path/to/document.md or none
related:
  - path/to/related1.md
  - path/to/related2.md
---
```

A daily journal file (`docs/log/YYYY-MM-DD.md`) may add one optional field, `reconstructed: YYYY-MM-DD`, when its entries were written well after the events they describe (e.g. after a multi-day pause) — `date` stays the day the entries describe, `reconstructed` records the day they were actually written, matching the "(reconstructed ...)" note `docs/INDEX.md` already carries in its Revision/date column for such a file.

`status` here is a coarse three-value field for quick scanning. `docs/INDEX.md`'s own Status column (`current`, `superseded by <path>`, `historical, applied`, `historical, partially applied`) is the authoritative, more granular record — if a document's frontmatter `status` and its `docs/INDEX.md` row ever appear to disagree, `docs/INDEX.md` wins and the frontmatter is stale until the next edit touches that document.

## File Naming Conventions

- **Dated documents**: `YYYY-MM-DD-<topic>.md` (e.g., `2026-09-17-flop-solver-design.md`)
- **Living documents** (fixed names):
  - `INDEX.md` — navigable index of all docs
  - `AUDIT.md` — the consistency-audit checklist (procedure, not results; see the exception note below)
  - `CONVENTIONS.md` — this file
  - `README.md` — directory guide

## Document Types and Locations

| Type | Location | Purpose |
|------|----------|---------|
| `research` | `docs/research/` | Investigation findings, benchmarks, analysis |
| `design` | `docs/design/` | Architecture, module design, UI mockups |
| `spec` | `docs/superpowers/specs/` | Feature specifications, protocol specs |
| `plan` | `docs/superpowers/plans/` | Implementation plans, step-by-step tasks |
| `review` | `docs/research/` | Code/design/spec/plan reviews and audit reports, named `REVIEW-*.md` or `AUDIT-YYYY-MM-DD.md` |
| `changelog` | `docs/research/` | SPEC-CHANGELOG-N.md, PLAN-N-CHANGELOG-M.md |
| `benchmark` | `docs/bench/` | Performance measurements, results |
| `log` | `docs/log/` | Daily standup logs, progress notes |

**Living-checklist exception:** `docs/AUDIT.md` is a fixed-name living document at `docs/` root (like `INDEX.md`, `CONVENTIONS.md`, `README.md`), not a `docs/research/` output — it is the checklist itself (when to run it, what each check compares). Running it produces a dated `review`-type report at `docs/research/AUDIT-YYYY-MM-DD.md`, which does follow the `review` row above.

## Revision and Change Management

When a spec or plan is revised:

1. Bump the revision number in the document's first line (e.g., `# Spec 3.6` → `# Spec 3.7`)
2. Create a changelog entry under `docs/research/` named `SPEC-CHANGELOG-N.md` or `PLAN-N-CHANGELOG-M.md`
3. Update the corresponding row in `docs/INDEX.md` with the new revision and date
4. Commit all three changes together: spec, changelog, and INDEX update

## Cross-References

- Use **relative markdown links**: `[link text](../path/to/file.md)`
- Wikilinks (`[[name]]`) are allowed **only in the orchestrator's private memory**, not in the repository

## Human Notes

- Human-authored notes live under `docs/notes/` (e.g., `docs/notes/project-decisions.md`)
- Agents never edit files under `docs/notes/`
- These are read-only reference material for context and decisions
