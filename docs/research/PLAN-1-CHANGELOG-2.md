# Plan 1 revision 2 — changelog (2026-09-17)

Target: `docs/superpowers/plans/2026-09-10-plan-1-foundation.md`
Source: `docs/research/REVIEW-cross-plan-2.md` §6 group **E01 — Plan 1**, plus every **E06 — Cross-reference and
execution-gate cleanup (all plans)** item that names Plan 1, plus the findings those edits cite (F11, F15, F16,
F22, F23, F24 as applicable). Task: B3b (seam re-check).

Result: revision line added under the title. 22 lines changed (14 insertions / 8 deletions; `git diff --stat`).
No task renumbering, no removed code blocks, TDD/Run-Expected step structure preserved.

---

## E01 — Plan 1

| Item | Finding(s) | Status | What changed |
|---|---|---|---|
| T1 / Step 1, `[workspace.dependencies]`: add `hex = "0.4"` and `ts-rs = "=12.0.1"` | F15 | **APPLIED** | Added both lines to the `Cargo.toml` code block in Task 1 Step 1 (after `holdem-hand-evaluator`), keeping `serde = "1.0"`, `serde_json = "1.0"`, `thiserror = "2.0"`, `sha2 = "0.10.9"` and the evaluator rev unchanged. Added the clarifying sentence "A root optional-use version declaration does not enable ts-rs in `proto` or the worker" after the code block. Propagated to the matching claims: Task 1 Interfaces/Produces bullet (now lists `hex 0.4`, `ts-rs =12.0.1`), the Global Constraints "Dependency versions" bullet, self-review deviation 14 (ts-rs), and the "Reconciliations the consuming plans own" bullet for `serde`/`sha2`/etc. (now also names Plan 2's `hex.workspace = true` and Plan 4's `ts-rs = { workspace = true, optional = true }`). |
| T1 / Step 3, replace the sentence at L218 (old: "`solver-worker` does not exist until plan 2 Task 1") | F16 | **APPLIED** | Replaced with the exact E01 text: "Do not execute the GNU worker build in this task: solver-worker is created in Plan 2 Task 7. Plan 2 Task 1 owns the V1 MSVC FLOP-FAST measurement and records the worker toolchain selection; this task only creates the fallback script. Every subsequent worker build, worker test and runtime staging step consumes that selection, while engine, bench and pokerai-app remain MSVC." Kept the "Run:"/"Verify only that..." structure around it. |
| "Correct the oracle owner and worker membership self-review references using E06" | F16, F22 (via E06 rows 1–2) | **APPLIED** | See E06 rows below — both cross-references corrected in Task 1's Interfaces/Produces bullet and in the "Reconciliations"/Self-review sections. |

## E06 — Cross-reference and execution-gate cleanup, items naming Plan 1

| Stale subject/reference (E06 row) | Status | What changed |
|---|---|---|
| "P1 says P2.T1 adds solver-worker member; P1.T1 Step 3 says worker exists after P2.T1" | **APPLIED** | Task 1 Interfaces/Produces bullet: "Plan 2 extends the root manifest with solver-worker..." -> "Plan 2 Task 1 excludes the vendor; Plan 2 Task 7 creates `solver-worker`, extends the root manifest with `"solver-worker"` in `members`... and adds that member." Reconciliations list (pre-Self-review): "Plan 2 Task 1 adds only `"solver-worker"` to `members`..." -> "Plan 2 Task 1 excludes the vendor...; Plan 2 Task 7 creates `solver-worker` and adds that member to `members`...". Task 1 Step 3's L218 sentence (see E01/F16 row above) also removes the "does not exist until plan 2 Task 1" claim. |
| "P1/P2 say bench oracle is P4.T20" | **APPLIED** | "Running everything" section: "belongs to **plan 4 Task 20**" -> "belongs to **plan 4 Task 23**". Self-review deviation 11: "its runner is `bench oracle` in plan 4 Task 20 (Or2)" -> "...plan 4 Task 23 (Or2; corrected per REVIEW-cross-plan-2 E06)". Confirmed via grep: no remaining "Task 20" reference in Plan 1 refers to Plan 4's bench oracle (the sole surviving "Task 20" hit is Plan 1's own Task 20 heading, `core-ranges` blocking/hashing — unrelated). |
| "P3 says tools project was P1.T13 (L567)" | **NOT APPLICABLE to Plan 1** | This row corrects stale text inside Plan 3 (`docs/superpowers/plans/...plan-3-preflop-replay.md`), which is out of scope for this task (edit-only-Plan-1 restriction). Verified Plan 1's own text needs no matching correction: Plan 1's Task 15 heading ("`tools/` Python project with pinned oracle versions") and File structure table already correctly place `tools/pyproject.toml`/`tools/tests/conftest.py` at Task 15, consistent with the row's replacement text ("Plan 1 Task 15 creates tools/pyproject.toml and tools/tests/conftest.py"). No `P1.T13` reference exists anywhere in Plan 1 to remove. |
| "Introductions refer to REVIEW-cross-plan §§4–5 as current order" | **NOT APPLICABLE to Plan 1** | Plan 1's header line cites `docs/research/REVIEW-cross-plan.md` **§1-§2** ("Cross-plan interface resolutions"), not §§4-5. Per REVIEW-cross-plan-2 §1, the original review's §§1-3 were treated as historical resolutions to verify (still validly cited), while only §§4-5 (stale scheduling authority) needed replacement. Plan 1's citation is not the stale pattern this row targets, so it is left unchanged. Confirmed via grep (line 13 of the plan, unchanged). |
| Whole-plan prerequisite-gate replacement (paragraph after the E06 table) | **NOT APPLICABLE to Plan 1** | That paragraph explicitly replaces gates in Plan 3 (L11), Plan 4 (L40) and Plan 5 (L11) only; Plan 1 has no whole-plan prerequisite gate of that kind (it is the first plan in execution order) and is not named. |

## Findings cited, applicability to Plan 1

| Finding | Applicability | Status | Reason |
|---|---|---|---|
| F11 — Four incorrect foundation API uses | **NOT APPLICABLE to Plan 1** | N/A (no P1 edit) | F11's four cited Plan 1 locations (`UtgStraddle.amount_chips` T3, `SuitPerm::IDENTITY` T21, `postflop_order(button, dealt)` T9, `inverse` T21) are the **correct** foundation definitions; the incorrect *uses* are all in Plan 4 (T7/T9/T10/T11), whose fix belongs to E04 (Plan 4), out of scope here. Confirmed via grep that all four Plan 1 definitions remain intact and unchanged. |
| F15 — Undeclared workspace dependencies | **APPLIED** | via E01 | See E01 table above. |
| F16 — Unowned V1 toolchain measurement and broken fallback propagation | **APPLIED** | via E01 | See E01 table above. |
| F22 — Stale task numbers and purported exports | **APPLIED (Plan-1-relevant portion only)** | via E06 rows 1–2 | F22's own citations (`P2:L7062`, `P2:L7065`) are Plan 2 text, out of scope; but F22's instruction to "apply semantic table E06" is satisfied for Plan 1 via the two E06 rows above (worker membership, bench oracle owner). |
| F23 — Redundant glob-covered workspace members | **NOT APPLICABLE to Plan 1** | No edit | F23's fix ("keep the glob; add only solver-worker and src-tauri when their manifests exist") is E03 (Plan 3), which removes Plan 3's redundant `members` appends. Plan 1's `members = ["crates/*"]` (Task 1 Step 1 and the File-structure table) is the glob being kept, correct as written, and is unchanged. Confirmed via grep. |
| F24 — Generated baseline artifacts called Create twice | **NOT APPLICABLE to Plan 1** | No edit | All four citations (`P2.T5`, `P4.T20`, `P2.T30`, `P4.T24`) are Plan 2/Plan 4 text; Plan 1 is not cited and the fix belongs to E04 (Plan 4). |

## Revision

Plan 1 is now at **revision 2**. Revision line added directly under the title:

> Revision 2 (2026-09-17): seam re-check edits E01/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-1-CHANGELOG-2.md

## Verification

Documentation-only task; Rust/Python/UI execution: not applicable (no code changed). Verification is by grep, run
after all edits, quoted here (see the task's final report for the same output):

- Old text confirmed **gone**: `does not exist until plan 2 Task 1` (0 matches), `Plan 2 Task 1 adds only` (0
  matches), `plan 4 Task 20` / `Task 20 (Or2)` / `Task 20's` (0 matches); the sole remaining `Task 20` hit in the
  file is Plan 1's own Task 20 heading (`core-ranges` blocking/hashing), unrelated to the bench-oracle
  cross-reference.
- New/kept text confirmed **present**: `hex = "0.4"` and `ts-rs = "=12.0.1"` in `[workspace.dependencies]` and in
  the Global Constraints/self-review prose (F15); the four F11 foundation-API definitions unchanged (F11 N/A);
  `members = ["crates/*"]` unchanged (F23 N/A); the Plan 1 header still cites `REVIEW-cross-plan.md` §1-§2, not
  §§4-5 (Introductions row N/A).

## Nothing else was left unapplied

Every E01 item for Plan 1 and every E06 row naming Plan 1 is either applied or recorded above as not applicable
with its reason. F11, F23 and F24 require no Plan 1 edit because their fixes are owned by other plans' edit
groups (E04, E03); this changelog records that disposition rather than silently skipping them.
