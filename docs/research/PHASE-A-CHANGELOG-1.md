---
type: changelog
status: current
date: 2026-09-17
supersedes: none
related:
  - REVIEW-phase-A.md
  - ../../CLAUDE.md
  - ../../AGENTS.md
  - ../INDEX.md
  - ../CONVENTIONS.md
  - ../AUDIT.md
  - ../log/README.md
  - ../log/2026-09-10.md
  - ../log/2026-09-17.md
---

# Phase A changelog 1: fix pass against REVIEW-phase-A.md

Source: `docs/research/REVIEW-phase-A.md` (10 MAJOR PA-01..PA-10, 10 MINOR PA-11..PA-20, verdict NOT READY) plus the orchestrator's own notes O1-O5 (scratchpad `RA-orchestrator-notes.md`), applied together with five orchestrator decisions recorded in this task's brief where the review left a choice (task-to-commit mapping, the test-gate carve-out for documentation work, the canonical journal template, the journal-correction rule, and the authoritative `Paths`/`startup_report()` decision). Ten files edited in place: `CLAUDE.md`, `AGENTS.md`, `.gitignore`, `docs/README.md`, `docs/INDEX.md`, `docs/CONVENTIONS.md`, `docs/AUDIT.md`, `docs/log/README.md`, `docs/log/2026-09-10.md`, `docs/log/2026-09-17.md`. Nothing was committed by this task.

## Orchestrator notes (O1-O5) — all in `CLAUDE.md`

| # | Severity | Disposition | Edit |
|---|---|---|---|
| O1 | MAJOR | APPLIED | `CLAUDE.md` §2 `docs/AUDIT.md` row rewritten to "Consistency-audit checklist (section 3 of that file says how to run it; results land in `docs/research/AUDIT-YYYY-MM-DD.md`)." Mirrored in `docs/INDEX.md`, `docs/README.md`, `docs/CONVENTIONS.md`, `docs/log/README.md`. |
| O2 | MINOR | APPLIED (no change needed) | Verified `scripts/build-worker-gnu.ps1` against plan 1 (`grep -n "gnu\|build-worker" docs/superpowers/plans/2026-09-10-plan-1-foundation.md`): plan 1 §3.6 fallback Task 1 names this exact script. `CLAUDE.md` §6 already used the correct name; left unchanged. |
| O3 | MAJOR | APPLIED | `CLAUDE.md` §7 "Never" list split into two bullets: the design-outline bullet (orchestrator's own document, dated amendments only) and a separate human-authored-notes/private-memory bullet. |
| O4 | MINOR | APPLIED | Removed "Maintained concurrently" from every `CLAUDE.md` §2 row; kept "read first each session" on the `docs/INDEX.md` row only. |
| O5 | MINOR | APPLIED | `CLAUDE.md` §4 now states Claude Code loads it automatically and Codex reaches it through `AGENTS.md`, before the unchanged three-step order. |

## MAJOR findings

| ID | Location | Disposition | Edit |
|---|---|---|---|
| PA-01 | `docs/INDEX.md`, `docs/AUDIT.md` | APPLIED | `docs/INDEX.md` gained an Orientation block stating both live seams (`Paths` field names, `Engine::new`/`startup_report()`) with the authoritative decision and evidence (`PLAN-2-CHANGELOG-1.md` lines 98/101/126), replacing the "§1-§3 of that review are fully applied" overstatement in both the `REVIEW-cross-plan.md` row and "Current authorities". `docs/AUDIT.md` §4 rewritten: the two seams marked still-open, and the surrogate/`bench oracle` ownership (plan 4 Tasks 11/23, confirmed by direct grep of both plans) marked resolved with only plan 2's stale Task-10/20 self-review references left to fix. A journal entry recording this fix pass was appended to `docs/log/2026-09-17.md` (task A5); Phase B is explicitly not claimed complete. |
| PA-02 | `docs/log/2026-09-10.md` | APPLIED | Re-ran `git show --format=fuller --name-only` on all 19 commits. Fixed plain path typos in place (`R7-pokerdata-verification.md`, the design outline, the five exact dated plan filenames). Appended a dated "Corrections (2026-09-17)" section fixing the 14:17/14:54/15:21 review-and-changelog off-by-one numbers and documenting the interleaved file lists for `fe0a01d`/`77d7fa8`/`f71c7ce`/`f164807`/`720f0f8`, all quoting the actual `git show --name-only` output. Original entries left otherwise unedited (append-only). |
| PA-03 | `docs/log/2026-09-10.md` | APPLIED | Corrections section separates authorization (`af0b02f`, 15:31, outline §0b) from measured/verified installation (evidenced in plan 1's Tech Stack and verification note at `ddcbe04`, 18:15, in the orchestrator's own session, not a commit); attributes the chart-first decision to `af0b02f` rather than `7f42ca0`; states the first spec fix pass already reaches revision 2 at `a266e87`; describes `rust-toolchain.toml` as a plan-1-Task-1 requirement, not an existing file. |
| PA-04 | `CLAUDE.md` §5, `AGENTS.md` | APPLIED | Per orchestrator decision 2: `CLAUDE.md` §5 and `AGENTS.md` step 4 now scope the `cargo test --workspace` gate to "from the moment the Cargo workspace exists" and require, for documentation-only tasks or any task before the workspace exists, the named checks with quoted output and an explicit "Rust/Python/UI execution: not applicable" statement — never an absent workspace or a doc task represented as a passed suite. |
| PA-05 | `docs/log/README.md`, `CLAUDE.md` §5, `AGENTS.md` | APPLIED | Per orchestrator decision 3: one canonical entry template now lives in `docs/log/README.md` (task id required-when-assigned else "none"; what changed; files; verification with "not applicable" allowed; decisions; open items; `commit: pending` or a hash) with a new "Who writes what" section explaining the implementer-writes-pending / orchestrator-appends-`committed <hash> for <task id>` split and that the orchestrator alone edits `docs/INDEX.md`. `CLAUDE.md` §5 and `AGENTS.md` link to it instead of restating the fields. `docs/AUDIT.md` F1 updated so a `commit: pending` entry is not itself a gap. |
| PA-06 | `docs/AUDIT.md`, `CLAUDE.md` §5 | APPLIED | Per orchestrator decision 1: `CLAUDE.md` §5 defines the `Task: P<n>.T<m>` / `Task: A<n>`\|`B<n>` / `Task: none` trailer-style commit-body line as the one mapping mechanism, with a sentence that the plans' own commit examples are read with this line added (plans not edited). `docs/AUDIT.md` §1 and G1 now count/grep by `git log --format=%B \| grep -E "^Task: P[0-9]+\.T[0-9]+"` instead of the unspecified "tagged `P<n>.T<m>`" convention. |
| PA-07 | `docs/AUDIT.md` B1/B2/B3 | APPLIED | Rewrote the producer/consumer model above the (b) table: one producing plan.task per id, any number of consuming/integrating plan.task references, and a completed/scheduled/conditional/deferred disposition per id — V9 explicitly conditional (never flagged), V21/V22 explicitly scheduled until the release gate runs, phase-2/non-goal items exempt. |
| PA-08 | `docs/AUDIT.md` C1 | APPLIED | C1 rewritten to require the producer's **current** revision (or its latest changelog) to match every consumer; an old `REVIEW-cross-plan.md` §1 resolution (e.g. M13) is evidence of a past attempt only, not a pass, when a later `PLAN-N-CHANGELOG-M.md` recorded a different decision — the exact `Paths` case is named as the worked example. Only an explicitly specified, both-ends-checked adapter may bridge a real difference. |
| PA-09 | `docs/AUDIT.md` D1/D2 | APPLIED | D1 rewritten to check each revision transition against a changelog that explicitly states its source/target/resulting revision rather than filename-suffix equality (spec revision 6 legitimately documented by `SPEC-CHANGELOG-5.md` since changelog count and resulting revision are different counters); `PLAN-N-CHANGELOG-1.md` recognized as a plan's revision evidence even without a plan-header revision line; an explicit header revision is required only from that plan's next revision onward. D2 accepts existing changelogs' own disposition wording (e.g. `SPEC-CHANGELOG-5.md`'s "Edit applied" column) and applies the uniform-wording expectation prospectively. |
| PA-10 | `docs/AUDIT.md` G4 | APPLIED | G4 rewritten to require, at a release audit, every §13.5 measurement (not just the six suite columns): cold-cache e2e p95 <= 15 s, zero final-delivery fault-injection violations, supported-fixture EV/coverage, cache-hit latency, and the `bench oracle` V21/V22 tolerances. Earlier (before-plan/revision/ten-task) audits record which of these are "not yet due" instead of claiming a pass from a report's mere presence. |

## MINOR findings

| ID | Location | Disposition | Edit |
|---|---|---|---|
| PA-11 | `AGENTS.md`, `docs/README.md` | APPLIED | `AGENTS.md` "Where things are" table now points to `docs/INDEX.md` and `docs/AUDIT.md` by name and marks the Rust workspace, vendored solver, and fixtures/bench rows "future, not created yet". `docs/README.md` marks `bench/` and `notes/` as planned destinations with no active links, and states the documentation-only project state up front. |
| PA-12 | `AGENTS.md`, `CLAUDE.md` §4, `docs/README.md`, `docs/INDEX.md` | APPLIED | One reading order stated everywhere: `docs/INDEX.md` (Orientation) -> newest dated `docs/log/YYYY-MM-DD.md` (excluding `docs/log/README.md`, called out explicitly in all four files) -> `CLAUDE.md`/`AGENTS.md` (loaded automatically, not a read step, per O5) -> task document. `AGENTS.md` gained an explicit "Orient" step 1 before "Understand the task". |
| PA-13 | `docs/CONVENTIONS.md` | APPLIED | `review` type's location changed to `docs/research/` (`REVIEW-*.md`/`AUDIT-YYYY-MM-DD.md`), and a new "Living-checklist exception" note documents `docs/AUDIT.md` as the fixed-name procedure at `docs/` root, consistent with `docs/README.md`'s and `CLAUDE.md`'s description. |
| PA-14 | `docs/CONVENTIONS.md`, six new docs | APPLIED (option 1: add frontmatter, not narrow the scope) | Added YAML frontmatter to the six documents that lacked it: `docs/README.md`, `docs/INDEX.md`, `docs/CONVENTIONS.md`, `docs/log/README.md`, `docs/log/2026-09-10.md`, `docs/log/2026-09-17.md` (`docs/AUDIT.md` already had it). Added an optional `reconstructed: YYYY-MM-DD` field to the frontmatter schema for a journal file written well after its events, and a note that `docs/INDEX.md`'s Status column is authoritative over the coarser frontmatter `status` field. |
| PA-15 | `docs/log/2026-09-10.md`, `docs/log/2026-09-17.md` | APPLIED | Added a provenance note at the top of `docs/log/2026-09-10.md` distinguishing commit-derived facts (timestamps, file lists — reliable) from the orchestrator's contemporaneous account (agent roles, decisions/open-items text, the Codex-quota figure — not independently git-verifiable) rather than asserting either as equally sourced. |
| PA-16 | `docs/AUDIT.md` G3 | APPLIED | G3 split by runner/language: Rust `fn <test_name>` search for Rust-owned tests, a separate `test(...)`/`it(...)` grep for TypeScript/Vitest/WebDriver tests (spec §13.4, plan 5), selected by the owning task's runner from B2 — a Rust-only search is no longer used to judge a UI test. |
| PA-17 | `docs/AUDIT.md` §3 | APPLIED | Added a same-day multiple-runs rule (append `## Run N (HH:MM, trigger: ...)` blocks, never overwrite, keep a failed run alongside its recheck) and an audited-range rule (`Prior audited HEAD` / `Audited HEAD` hashes per run, with F1 keyed to that commit range instead of a date). |
| PA-18 | `docs/INDEX.md` | APPLIED | Every Path cell across all tables is now a relative Markdown link from `docs/INDEX.md`, retaining the full repo-relative path as its label. |
| PA-19 | `docs/AUDIT.md` E1 | APPLIED | E1 now enumerates all files recursively (`rg --files --hidden --no-ignore docs`, or `find docs -type f`) rather than `*.md` only, and explicitly checks row multiplicity (duplicate rows) and planned (`current (pending)`) rows against their commissioning source. |
| PA-20 | `docs/INDEX.md` | APPLIED | Added a short Orientation block at the top of `docs/INDEX.md` (state, authority order, the two live seams, phase status, start-here links) before the full inventory, which is now explicitly scannable on demand rather than read start to end. |

## Not applied

Nothing from `REVIEW-phase-A.md` or O1-O5 was left unapplied. Two clarifications on scope, not exceptions:

- **`.gitignore`** — reviewed against the assignment; no PA-* finding or O-note names it, and the review's own self-review paragraph confirms it "correctly excludes the listed build/dependency/cache/temp paths and leaves reports/lockfiles visible." Left unchanged.
- **The two live interface seams themselves** (`engine::Paths` field names in plans 4/5; `Engine::new` vs `startup_report()` in plan 5) are documented accurately in `docs/INDEX.md` and `docs/AUDIT.md` (PA-01/PA-08), but the plan files are not edited — that edit is explicitly Phase B's job (`docs/research/REVIEW-cross-plan-2.md`, current (pending)) and outside this task's ten-file scope, per the task's own instruction not to edit the plans.
