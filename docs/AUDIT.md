---
type: review
status: current
date: 2026-09-17
supersedes: none
related:
  - docs/design/2026-09-10-design-outline.md
  - docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md
  - docs/superpowers/plans/2026-09-10-plan-1-foundation.md
  - docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md
  - docs/research/REVIEW-cross-plan.md
  - docs/research/REVIEW-phase-A.md
  - docs/INDEX.md
  - docs/CONVENTIONS.md
---

# Consistency audit checklist

## 1. Purpose and when it runs

Drift between the outline, spec, plans and code has caused three spec review rounds and five plan review
rounds already. This checklist is the fixed, delegable procedure that catches drift early instead of after a
rewrite. Run it:

- **Before starting execution of each plan** (plans 1-5), against that plan and everything it depends on.
- **After any spec or plan revision** (a new `SPEC-CHANGELOG-N.md` or `PLAN-N-CHANGELOG-M.md` lands).
- **After every 10 completed implementation tasks**, counted across all plans by commits whose body carries a `Task: P<n>.T<m>` line (`CLAUDE.md` section 5; `git log --format=%B | grep -E "^Task: P[0-9]+\.T[0-9]+"`, one match per implementation commit).
- **On request**, e.g. before a release gate (V21/V22) or a handoff between orchestrator sessions.

Each run produces one dated report (section 3). A run that finds a BLOCKER stops execution of the affected
plan(s) until the fix lands and the check is re-run clean.

## 2. Checks

### (a) Outline vs spec

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| A1 | Every bullet under outline §0 and §0b vs a spec sentence stating the same fact and value | For each bullet's key term (vendor name, `flop_budget_s`, MSVC workload, rake/cap, depths), run `grep -n "<term>" docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` | A spec sentence exists with the same value; no bullet is un-grep-able | Term, spec line number, match/gap |
| A2 | Every spec sentence tagged **(spec decision)** vs the outline | `grep -n "(spec decision)" docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, then check each is either absent from the outline (spec-only, allowed) or consistent with it (never contradicting) | No spec-decision sentence contradicts an outline value | List of spec-decision lines with outline cross-reference or "spec-only" |
| A3 | Outline §11 validation list (V1-V4, V9, V14, V21) vs spec §14.4 table | Diff the two id lists by eye; spec §14.4 also carries V22 (spec-only, release gate) | Every outline-listed id appears in §14.4 with a matching question; extra spec-only ids are noted, not flagged | Ids matched, ids added by spec only |

### (b) Spec vs plans

A spec subsection, test or validation id can legitimately span several plans: one plan **produces** it (defines/implements the behavior, recorded in that plan's own Self-review as its task), and any number of other plans **consume** it (integrate against it, run it, or extend its coverage, recorded in *their* Self-review as a consumer, not a second producer). B1-B3 below check that every id has exactly one producer and that every consumer reference actually resolves to that producer — not that every id has exactly one plan touching it at all. Record each id's check as **completed** (producing task done, code/tests exist), **scheduled** (producing task not yet reached, no BLOCKER), **conditional** (spec marks it conditional on an external event, e.g. V9 on the PokerData purchase per spec §14.4 — never flagged missing), or **deferred** (explicitly phase-2, e.g. non-goals in the outline §1) — a MAJOR/BLOCKER applies only to an id with no producer and no completed/scheduled/conditional/deferred disposition.

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| B1 | Every spec §1-§16 numbered subsection in scope for the plan set vs the plans' Self-review "Spec coverage" tables | `grep -n "^### \|^## " docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` for the section list; `grep -n "§" <plan>.md` inside each plan's `## Self-review` section for covered sections, noting whether each hit is that plan's own producing task or a stated consumer/integration reference to another plan's task | Every subsection has exactly one producing plan.task; a subsection that spans plans by design (e.g. §13.1 across plans 1/3/4) has each plan's specific contribution recorded, not treated as ambiguous ownership; phase-2 backlog items are exempt | Subsection id, producing plan.task, consuming plan.task(s) if any, disposition (completed/scheduled/conditional/deferred), or "no producer" |
| B2 | Every test named in spec §13.1/13.2/13.3/13.4/13.5 vs the plans' Self-review coverage tables, with the spec's expected numbers (fixture counts, tolerances, thresholds) | List test names with `grep -oE '\`[a-z_0-9]+\`' ` on spec §13, then `grep -rn "<test_name>" docs/superpowers/plans/*.md` | Each test name has exactly one plan.task that defines/writes it (the producer); any other plan naming it is a stated consumer (e.g. running it, citing its result) not a second definition; any numeric value quoted at the producing site (e.g. "200 hands", "1e-3", "p95 <= 6 s") matches the spec's value | Test name, producing plan.task, consuming plan.task(s) if any, number match y/n, disposition |
| B3 | Every validation id used in spec §14.4 (V1, V2, V3, V4, V9, V14, V21, V22) vs an owning task | `grep -n "^| V" docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` for the id list, then `grep -rn "\bV[0-9]\+\b" docs/superpowers/plans/*.md` | Every id is claimed by at least one producing task; V9 is recorded **conditional** (spec §14.4, gated on the PokerData purchase, currently deferred by the user) and never flagged as a gap; V21/V22 are recorded **scheduled** until the release gate actually runs (see G4) | Id, owning plan.task(s), disposition |

### (c) Cross-plan interfaces

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| C1 | Each plan's "Interfaces consumed from plan N" (Consumes) block vs the producing plan's **current** Self-review "Cross-plan surface this plan freezes" (Produces) block, and vs that producer's own later `PLAN-N-CHANGELOG-M.md` if one exists | `grep -n -A2 "Consumes\|consumed from plan" <consumer>.md` and `grep -n -A2 "Cross-plan surface\|freezes" <producer>.md`, diff type/function name, path (crate::module), and signature by eye; if the producer has a later changelog, use *its* stated signature, not the producer's original draft | Name, path and signature match verbatim **after applying the latest authoritative decision** (the producer's current revision, or an explicit changelog request/disposition that supersedes it). A `docs/research/REVIEW-cross-plan.md` §1 entry (e.g. M13) is evidence of a past resolution attempt, never itself a pass — if a later `PLAN-N-CHANGELOG-M.md` recorded a different decision (as `PLAN-2-CHANGELOG-1.md` line 98/126 did for `Paths`, rejecting M13's names), the consumer must match *that* decision, and a consumer still citing the superseded name is a live mismatch, not a documented-resolved one. Only an explicitly specified adapter (named, with signatures checked at both ends) may bridge a real difference | Symbol, consumer plan.task, producer plan.task, latest authoritative decision (source), match y/n |
| C2 | No type or file is produced (defined first) by two plans | `grep -rn "^- \`" docs/superpowers/plans/*.md` over each plan's "freezes"/Produces list; collect symbol names, sort, look for duplicates | Zero duplicate producers, except a documented wholesale replacement (e.g. `engine::snapshots::SnapshotStore` plan 2 -> plan 3, recorded in both plans' deviation lists) | Symbol, producing plan(s), replacement documented y/n |
| C3 | No orphan Consumes item (something a plan consumes that no plan documents producing) | Cross-check every Consumes symbol from C1 against `docs/research/REVIEW-cross-plan.md` §2 "Duplicates and orphans"; confirm each of the 8 orphans there is now assigned to a producer | Zero unassigned orphans | Orphan id, assigned producer plan.task, or "still open" |

### (d) Revision and changelog integrity

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| D1 | Every revision transition (a spec header's stated revision number, or a plan's stated task/line count after a review) vs a changelog that explicitly identifies its source revision, target document, and resulting revision | `grep -n "^# .*[Rr]evision\|^# Plan .* revision" docs/superpowers/specs/*.md docs/superpowers/plans/*.md`, then `ls docs/research/*CHANGELOG*.md` and read each changelog's own stated source→target, not just its filename suffix | Every transition is covered by a changelog whose text names the exact source revision, the target document, and the resulting revision — do not infer this from `SPEC-CHANGELOG-N` / revision-N suffix equality (spec revision 6 is legitimately documented by `SPEC-CHANGELOG-5.md`, since changelog count and resulting revision are different counters: changelog M takes revision M to M+1). A plan revision's evidence is its `PLAN-N-CHANGELOG-1.md` recording the before/after task-and-step counts even when the plan file itself carries no separate numeric revision line; require an explicit header revision line only on that plan's *next* revision onward. Accept an existing changelog's documented legacy disposition as-is rather than reporting it missing | Source revision, target document, resulting revision, changelog file, present y/n |
| D2 | Every changelog row is marked applied/rejected/deferred with a reason | `grep -n "APPLIED\|REJECTED\|DEFERRED\|Status" docs/research/*CHANGELOG*.md` per row | No row is missing a status word or a clear synonym (`SPEC-CHANGELOG-5.md`'s "Edit applied" column heading counts); a REJECTED or DEFERRED row states why in the same cell. This uniform-wording expectation applies **prospectively** to changelogs written after this row was added — an existing changelog's own documented disposition wording is accepted as-is, not reported as a missing-status finding | Changelog file, row id, status, reason present y/n |

### (e) INDEX.md status accuracy

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| E1 | Every file under `docs/` (any type, not just Markdown) vs a row in `docs/INDEX.md` | `rg --files --hidden --no-ignore docs \| sort` (or, without `rg`, `find docs -type f \| sort`) vs the paths listed in `docs/INDEX.md`, normalized to repo-relative form | Every file has exactly one row (check row multiplicity too — a path listed twice is a finding even if the path exists); no row points at a deleted path; a row marked `current (pending)` for a deliberately planned, not-yet-created path (`docs/INDEX.md`'s legend) is not a missing-file finding, but is checked against whatever commissioned it (a log entry, task brief, or this checklist's own §4) | Missing paths, stale rows, duplicate rows, planned-path rows and their commissioning source |
| E2 | Every "superseded" mark in `docs/INDEX.md` vs the actual supersession | For each superseded row, find the superseding document's frontmatter `supersedes:` field (or its changelog's "Target"/"Source" line) and confirm it names the superseded path | Every superseded row is named by exactly one current document; no row is marked current while a later revision exists | Path, marked status, actual status |

### (f) Journal completeness

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| F1 | Every commit since the last audit vs a `docs/log/YYYY-MM-DD.md` entry | Identify the prior audit's exact boundary from its own recorded "audited HEAD" (see section 3), not merely its date, then `git log <prior-audited-HEAD>..HEAD --oneline`; grep the log files across that commit range for a matching entry (by task id, file touched, or commit summary) | Every commit has at least one journal entry. A journal entry with `commit: pending` and a stated task id is **not** a gap — it is legitimate in-flight work (the implementing agent never commits; `CLAUDE.md` section 5) and is checked instead against whether the orchestrator's later `committed <hash> for <task id>` line exists; only an entry with no commit, no `commit: pending`, and no recorded blocker is a finding | Commit hash, journal file/entry found y/n, pending-commit entries and whether later resolved |

### (g) Code vs plan (once code exists)

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| G1 | Every plan task vs a commit | `git log --format=%B \| grep -B5 -E "^Task: P[0-9]+\.T[0-9]+"` to pair each `Task: P<n>.T<m>` trailer line with its commit (`git log --format="%H %B"` per commit, filtered the same way), against each plan's task list | Every task with status "done" in the plan or log has exactly one commit carrying its `Task:` line; no commit claims a task twice | Task id, commit hash, found y/n |
| G2 | Full workspace test suite | `cargo test --workspace` (once the workspace exists; before then this check is `not applicable`, per `CLAUDE.md` section 5 — not a failure) | Exit code 0, zero failing/ignored-without-reason tests | Command output, pass/fail, or `not applicable` with the reason |
| G3 | Every test named in spec §13 vs a test function in the codebase, split by the runner/language the owning task actually uses | For a Rust test collected in B2: `grep -rn "fn <test_name>" crates/ solver-worker/ apps/`. For a TypeScript/Vitest/WebDriver test (spec §13.4, plan 5): `grep -rn "test(['\"]<test_name>\|it(['\"]<test_name>" apps/pokerai-ui/`. Use the owning task's own file/runner (from B2's producing plan.task) to pick which grep applies, then run that suite and quote its output | Every named test exists under its runner's exact convention (`fn` for Rust, `test(...)`/`it(...)` for TypeScript); none renamed without a recorded deviation; a Rust-only search is never used to judge a UI test | Test name, runner, file:line, found y/n |
| G4 | Baseline gate results present, matching spec §13.5's actual criteria | Before the release gate is due: record that explicitly rather than running a check (see section 3 timing). At a release audit: `ls docs/bench/*.md`, read the dated report's machine-readable JSON block, and check it against §13.5's full list — all six suite columns (`river_std, river_min, turn_std, turn_min, flop_fast, flop_min`) with p50/p95/peak-RSS, cold-cache e2e p95 <= 15 s, zero final-delivery violations under fault injection, the supported-fixture EV/coverage results, cache-hit latency, and the `bench oracle` analytic tolerances (V21/V22) | At a release audit: a dated report exists with every one of the above measurements recorded and passing, not merely the six suite columns present. At any earlier audit (before-plan, revision, ten-task): record which of these gated measurements are not yet due, rather than claiming a pass from a report's mere presence | Report path, each §13.5 measurement present/passing y/n, or "not yet due" |
| G5 | No stub markers in shipped code | `grep -rn "TODO\|unimplemented!\|todo!" crates/ solver-worker/ apps/ --include=*.rs` | Zero matches outside `tools/` (dev-only) and outside a Self-review-documented interim stub (e.g. plan 1's `mc.rs` `unreachable!`) | File:line, justified y/n |

### (h) Memory vs repo

| ID | What to compare | How | Pass criterion | Record |
|---|---|---|---|---|
| H1 | Orchestrator's private memory files (`~/.claude/projects/PokerAI/memory/*.md`) vs `CLAUDE.md` and `docs/INDEX.md` | Read each memory file's stated decisions (budgets, toolchain state, purchase status, phase status) and grep `CLAUDE.md` §3/§6 and `docs/INDEX.md` for the same facts | No memory file states a decision, value, or status that contradicts the repo's current `CLAUDE.md` or `docs/INDEX.md`; a stale memory fact is flagged for `consolidate-memory`, never silently trusted | Fact, memory file, repo value, contradiction y/n |

## 3. How to run it

Delegate the run to one reviewer agent (Codex, or Claude Opus if Codex is unavailable) with this file as its
brief and no other instructions. The agent works through sections (a)-(h) in order, running the exact commands
above, and writes findings to `docs/research/AUDIT-YYYY-MM-DD.md` using this format per finding:

```
ID: <check id, e.g. C1>
Severity: BLOCKER | MAJOR | MINOR
Location: <file:line or plan.task>
Quote: <the exact conflicting text, <=30 words>
Edit: <the concrete fix>
```

**Multiple runs on one date.** The before-plan, revision, ten-task and on-request triggers can all fire the
same day. Do not overwrite same-day output: append a new `## Run N (HH:MM, trigger: <which one>)` block to
that date's `AUDIT-YYYY-MM-DD.md` instead of replacing the file, and never delete a failed run's block when a
recheck follows it — the failed run and its successful recheck are both kept, in order.

**Audited range.** Every run states two commit hashes at its top: `Prior audited HEAD: <hash or "none">` (the
`Audited HEAD` recorded by the most recent prior run block, across all `AUDIT-*.md` files, not just today's)
and `Audited HEAD: <hash>` (this run's `git rev-parse HEAD`). F1 above uses this exact range, not a date
range, so two audits on the same calendar day still check disjoint commit sets.

A BLOCKER (a check that fails outright: a missing changelog, a red test, a duplicate producer with no
resolution, a contradicted memory fact) stops execution of the affected plan until it is fixed and this audit
is re-run clean on that section. MAJOR and MINOR findings are queued but do not block.

## 4. Known open items

Seeded from the current handoff (updated 2026-09-17 during the Phase A fix pass, against `docs/research/REVIEW-phase-A.md` PA-01/PA-08); carry these forward until each is closed by a dated audit report:

- **Still open — `engine::Paths` field names.** Plan 2's current revision defines `Paths { log_dir, worker_exe, preflop, cache }` (`PLAN-2-CHANGELOG-1.md` line 98, request (c)); this is the authoritative decision (recorded 2026-09-17, `docs/INDEX.md` Orientation) — it explicitly rejected cross-plan review M13's `worker`/`log` naming. Plans 4 and 5 still read/construct the rejected `Paths { worker, preflop, cache, log }` (`2026-09-10-plan-4-flop-cache-presolver.md:63`, `2026-09-10-plan-5-ui.md:77,1116`). Reconcile both to plan 2's names before either consumer plan's `Paths`-touching tasks are executed. (M13's C1 entry in `REVIEW-cross-plan.md` §1 is evidence that a resolution was *attempted*, not that the current signatures agree — see C1's pass criterion above.)
- **Still open — startup diagnostics call site.** Plan 2 exposes them only via `Engine::startup_report()` called after construction (`PLAN-2-CHANGELOG-1.md` line 101, request (f)); this is the authoritative decision. Plan 5 still expects `Engine::new(config, paths) -> Result<(Engine, Vec<String>), EngineError>` to return them directly (`2026-09-10-plan-5-ui.md:56,1012,1218`) and its own self-review (`:3153`) calls this "resolved, not remaining," which is stale. Update plan 5 to call `Engine::new` for a bare `Engine` and then `startup_report()` separately.
- **Still open — execution order and effort.** `docs/research/REVIEW-cross-plan.md` §4's execution-order table (`P<plan>.T<task>` rows) and §5's total-effort table both use task numbers/counts from before the plan-revision changelogs (92 tasks / 498 steps); task numbers have since shifted in at least plans 1 and 2, and the true total is 114 tasks / 606 steps. Re-derive both tables from the current plan files.
- **Resolved, confirm only in the next audit run.** Ownership of the §6 experimental synthetic-root surrogate and `bench oracle`: plan 2's own Self-review (`2026-09-10-plan-2-worker-engine.md:7062,7065`) still cites the stale task numbers "plan 4 Task 10" (surrogate) and "plan 4 Task 20" (`bench oracle`) from before plan 4's revision. Plan 4's *current* revision actually produces the surrogate at **Task 11** (`2026-09-10-plan-4-flop-cache-presolver.md:1982`) and `bench oracle` at **Task 23** (`:3729`), and plan 4's own Self-review claims both (`:4041`, `:4051`). Ownership is settled; only plan 2's two stale consumer references need the number update, tracked with the other Phase B edits above.
- **Resolved, historical only.** The cross-plan review's M13 naming for `Paths` was superseded by `PLAN-2-CHANGELOG-1.md`'s orchestrator-directed decision (line 126: "The orchestrator's names are implemented. Plan 5 Task 3 must construct `log_dir` and `worker_exe`, not `log` and `worker`") — plan 2 uses the orchestrator's names, not M13's. This is the same fact as the first bullet above; it is not a separate open question once C1 is re-run against the current producer.
