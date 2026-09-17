# Plan 2 revision 3 — changelog (2026-09-17)

Plan: `docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md`.
Input: `docs/research/REVIEW-cross-plan-3.md` section 3 — **R1** (F20 precision: the root-street exception and the reason-string mismatch), **R2** (F12 forward-owner note for the Task 11 extraction seam), **R7** (F22 reference cleanup, the plan-2 items); and section 5's stale-reference disposition, which names "P2:L4207 (exact-equity owner)" and "all five L13 spec-version labels" as actionable leftovers.

Size before: 7,270 lines, 30 tasks. No task was renumbered, no task was added or removed. Resulting revision: **3**.

---

## 1. Edits applied

| # | Finding | Location | Disposition | What changed |
|---|---|---|---|---|
| R1-1 | F20 — wire reason string doesn't match the spec's corrected row | Task 8, "The donk rule" intro, point 2 | **APPLIED** | "the reply is `ack{rejected, reason: "... donk option must be the explicit empty list, never None"}`" → "the reply is `ack{rejected, reason: "donk option missing for <street>"}`". |
| R1-2 | F20 — `precheck`'s wire rejection used the old reason text | Task 13, Step 3, `precheck` body (the later-street `None` arm) | **APPLIED** | `None => return Err(format!("{s:?} donk option must be the explicit empty list, never None"))` → `None => return Err(format!("donk option missing for {}", match s { Street::Turn => "turn", Street::River => "river", _ => unreachable!() }))`. The `Some(_) => "{s:?} donk sizes must be empty"` arm (a different case — sizes present but non-empty) is unchanged. |
| — | Kept unchanged per R1 | Task 8, Step 1, `tree_config`'s own direct-caller error (`"{s:?} donk option must be the explicit empty list (never None)"`) and its test (`assert!(e.contains("donk option must be the explicit empty list"))`) | **NOT APPLIED — intentionally preserved** | R1 explicitly says "Keep Task 8's direct-caller tree_config error and its test unchanged; that is not a wire reason." `tree_config` is called directly by in-process callers that bypass `precheck` (e.g. `bench materialize`); its error is mapped by `job::run` to `invalid_request`, never surfaced as the worker's `ack{rejected}` wire reason. Verified still present, untouched. |
| R2-1 | F12 — Plan 2 produces no forward-owner note for the later extraction | Task 22, Interfaces (after the `SolvePlan.background` bullet, before Step 1) | **APPLIED** | Inserted the exact blockquote: "Forward owner: Plan 4 Task 11 modifies crates/engine/src/solve.rs to extract solve_request_from_parts and send_solve_request with the exact signatures declared there. Plan 2 produces run_solve first; it does not produce these helpers early. The later extraction preserves run_solve's signature, identity/deadline/heartbeat/cancellation/validation behavior and Task 23 retry policy, and lets the synthetic surrogate bypass SolveInput, cache and snapshots. Plan 4 Task 11 owns both the extraction and its callers." |
| R2-2 | F12 — Task 23 (which owns the retry policy the note promises is preserved) had no reference to it | Task 23, Interfaces | **APPLIED** | Added a bullet: "See Task 22's forward-owner note: Plan 4 Task 11 later extracts `solve_request_from_parts` and `send_solve_request` from this task's `run_solve`, preserving the retry policy fixed here." |
| §5-1 | F22 — stale task reference (section 5 "P2:L4207 (exact-equity owner)") | Task 18, Step 1, `core-eval` comment | **APPLIED** | `// \`core-eval\`'s shape (plan 1 Task 20): players are seat-tagged ranges, the result carries per-seat shares.` → `…(plan 1 Task 24): …` — this is R7's first bullet ("P2.T18/S1:L4207: replace 'plan 1 Task 20' with 'plan 1 Task 24'"), applied here under the section 5 stale-reference disposition since it names this exact site as an actionable leftover even though the task's plan-2 scope line names only R1/R2. |
| §5-2 | F22 — stale "revision 6" spec-version label (section 5 "all five L13 spec-version labels") | Header, L13 (`**Spec:**` line) | **APPLIED** | "…pokerai-assistant-design.md\` (revision 6), sections 2, …" → "…(revision 7), sections 2, …" — R7's second bullet, applied to plan 2 under the section 5 disposition. |
| — | Title / revision line | Lines 1-4 | **APPLIED** | Added "Revision 3 (2026-09-17): verification edits R1, R2, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-2-CHANGELOG-3.md" directly under the title, above the existing Revision 2 line (kept unchanged). R7 is credited because both section-5-directed edits (the L13 label and the plan-1-Task-20/24 reference) are literally R7's bulleted text, even though the task's own plan-2 scope line named only R1/R2 explicitly; "Also apply section 5's stale-reference disposition where it lists concrete leftovers" independently requires them. |

## 2. Exact lines changed (before / after)

### R1 — Task 8, "The donk rule", point 2

Before:
> 2. A missing or `None` donk option on a street strictly **after** the root is **structurally invalid input**: `precheck` rejects it before any work and the reply is `ack{rejected, reason: "... donk option must be the explicit empty list, never None"}`. No `solve` is admitted, no tree is built, the worker stays alive.

After:
> 2. A missing or `None` donk option on a street strictly **after** the root is **structurally invalid input**: `precheck` rejects it before any work and the reply is `ack{rejected, reason: "donk option missing for <street>"}`. No `solve` is admitted, no tree is built, the worker stays alive.

### R1 — Task 13, Step 3, `precheck`

Before:
```rust
match t.menus.get(&s).and_then(|m| m.donk.as_ref()) { Some(d) if d.is_empty() => {}, Some(_) => return Err(format!("{s:?} donk sizes must be empty")), None => return Err(format!("{s:?} donk option must be the explicit empty list, never None")) }
```

After:
```rust
match t.menus.get(&s).and_then(|m| m.donk.as_ref()) { Some(d) if d.is_empty() => {}, Some(_) => return Err(format!("{s:?} donk sizes must be empty")), None => return Err(format!("donk option missing for {}", match s {
    Street::Turn => "turn", Street::River => "river", _ => unreachable!()
})) }
```

### R2 — Task 22 Interfaces (forward-owner note inserted)

After (new, inserted before Step 1):
> Forward owner: Plan 4 Task 11 modifies crates/engine/src/solve.rs to extract solve_request_from_parts and send_solve_request with the exact signatures declared there. Plan 2 produces run_solve first; it does not produce these helpers early. The later extraction preserves run_solve's signature, identity/deadline/heartbeat/cancellation/validation behavior and Task 23 retry policy, and lets the synthetic surrogate bypass SolveInput, cache and snapshots. Plan 4 Task 11 owns both the extraction and its callers.

### R2 — Task 23 Interfaces (cross-reference added)

After (new bullet):
> - See Task 22's forward-owner note: Plan 4 Task 11 later extracts `solve_request_from_parts` and `send_solve_request` from this task's `run_solve`, preserving the retry policy fixed here.

### §5 / R7 — Task 18, Step 1 comment

Before: `    // \`core-eval\`'s shape (plan 1 Task 20): players are seat-tagged ranges, the result carries per-seat shares.`
After: `    // \`core-eval\`'s shape (plan 1 Task 24): players are seat-tagged ranges, the result carries per-seat shares.`

### §5 / R7 — header L13

Before: `**Spec:** \`docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md\` (revision 6), sections 2, 3.1-3.7, …`
After: `**Spec:** \`docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md\` (revision 7), sections 2, 3.1-3.7, …`

### Title / revision line

Before:
> \# Plan 2: Solver worker and engine river/turn path Implementation Plan
>
> Revision 2 (2026-09-17): seam re-check edits E02/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-2-CHANGELOG-2.md

After:
> \# Plan 2: Solver worker and engine river/turn path Implementation Plan
>
> Revision 3 (2026-09-17): verification edits R1, R2, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-2-CHANGELOG-3.md
>
> Revision 2 (2026-09-17): seam re-check edits E02/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-2-CHANGELOG-2.md

## 3. Not changed

- R3, R4, R5, R6 name no plan 2 location; no corresponding edit exists here.
- Task 16's wire assertions (`tree_mismatch` cases) and Task 8/13's other tests are unchanged — R1 only touches the reason string, not the pass/fail behavior of any existing assertion.
- No other section, task or fixture list in plan 2 was touched.

## 4. Verification

Documentation-only task: Rust/Python/UI execution: not applicable — no workspace exists. Verification is the grep check named in the task: every finding's cited old text is absent from the plan and the replacement is present. The quoted command output is recorded in the journal entry `docs/log/2026-09-17.md` and in the agent's report.
