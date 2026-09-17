---
type: convention
status: current
date: 2026-09-17
supersedes: none
related:
  - ../INDEX.md
  - 2026-09-10.md
  - 2026-09-17.md
---

# Journal Convention

This directory contains a daily append-only journal of all work on the PokerAI project. Each day has one file named `YYYY-MM-DD.md` containing all entries for that date.

## Why This Journal

The PokerAI project involves many agents across multiple sessions (Claude models as orchestrators and subagents; OpenAI Codex for reviews and implementation). The journal is the single shared source of truth about what changed, when, and why. Every agent reads it first (after `docs/INDEX.md`) to orient quickly before starting work.

## Rules

- **Append-only**: Never rewrite, edit, or delete earlier entries in any file. If a decision changes, record the new decision as a new entry.
- **One file per day**: Create `YYYY-MM-DD.md` on the first entry of that day. Use ISO 8601 dates (e.g., `2026-09-17.md`).
- **One entry per completed task**: Every finished task appends exactly one entry.
- **Local time in 24h format**: All timestamps are `HH:MM` in the timezone where work happened (Europe/Berlin in the current setup).

## Entry Format (the one canonical template)

This is the single canonical journal-entry template. `../../CLAUDE.md` section 5 and `../../AGENTS.md` both
link here rather than restating these fields — if you are updating them, update this block, not a second copy.

Each entry is a bullet block with exactly these fields, in this order:

```
- **HH:MM** | Agent (e.g., "Claude Fable orchestrator", "Sonnet subagent task A3", "Codex P1.T4") | Task ID
  - What changed (1–3 lines, active voice: "created X", "fixed bug in Y", "reviewed plan Z")
  - Files touched: `path/to/file.rs` (add/edit/delete noted inline or in 1–2 words if multiple)
  - Verification: quoted command output, or a link to a report/review that contains it; write
    "not applicable" for a documentation-only task with no runnable suite — never omit this field
    and never claim a result without quoting it (see `../../CLAUDE.md` section 5)
  - Decisions: ...; Open items: ...
  - Commit: `pending` (the default — an implementing agent never commits; see "Who writes what" below)
    or the actual hash once it is known
```

**Task ID** is the id you were given (e.g. `A3`, `P1.T4`), written exactly as given — never omitted. If no
task id was assigned to your work (a truly ad hoc action), write `none`; that is the only allowed placeholder,
and it is a deliberate statement, not a gap.

## Who writes what

The journal is append-only (see Rules above), and an implementing agent does not commit its own work — the
orchestrator does (`../../CLAUDE.md` section 5, `../../AGENTS.md`). This means:

- The agent that did the work appends its entry with `Commit: pending`, filling every other field. It never
  waits for a commit to exist before logging what it did, and never edits its own entry afterward.
- Once the orchestrator commits that work, it appends a **separate**, new one-line entry —
  `committed <hash> for <task id>` — rather than editing the pending entry above it. A reader (or `docs/AUDIT.md`
  check F1) pairs the two by task id.
- `docs/INDEX.md` is the orchestrator's file to edit; an agent proposes the INDEX row(s) its own task needs in
  its report instead of writing them into `docs/INDEX.md` directly.

## Template to Copy

Paste this block into each daily file at the start, then fill in entries as work completes:

```markdown
# 2026-MM-DD

- **HH:MM** | Agent name | Task ID (or "none")
  - What changed (1–3 lines)
  - Files touched: `path/to/file`
  - Verification: quoted output / linked report / "not applicable"
  - Decisions: ...; Open items: ...
  - Commit: pending
```

## Where This Fits

The newest **dated** file in `docs/log/` (`YYYY-MM-DD.md`; this README is the format convention, never "newest") is the **second thing any agent reads** when resuming work (after `docs/INDEX.md`). Read the latest daily file, then trace backwards if needed. The journal is companion to:

- `docs/INDEX.md` — top-level map
- `docs/CONVENTIONS.md` — naming, style, tooling
- `docs/AUDIT.md` — the consistency-audit checklist (procedure; a run's results land in a dated `docs/research/AUDIT-YYYY-MM-DD.md` report)
- Plans in `docs/superpowers/plans/` — detailed task lists
- Memory files in the user's `~/.claude/projects/PokerAI/memory/` — session-specific state and decisions

Every completed task (whether from orchestrator pausing, a subagent finishing, or a review cycle) appends one entry to the current daily file before committing.
