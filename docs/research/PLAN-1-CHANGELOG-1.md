# Plan 1 revision 1 — changelog (2026-09-10)

Target: `docs/superpowers/plans/2026-09-10-plan-1-foundation.md`
Sources: `docs/research/REVIEW-of-plan-1.md` (3 BLOCKER, 7 MAJOR, 12 MINOR), `docs/research/REVIEW-cross-plan.md` §1, §2, §4, §5,
spec revision 6 (`docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`), and the orchestrator's four binding decisions.

Result: **5,436 lines, 25 tasks, 121 checkbox steps** (was 4,914 lines, 21 tasks, 105 steps). Format, TDD step shape, real code in every
code step, one commit per task and the Self-review section are preserved. No placeholders were introduced.

---

## 1. `REVIEW-of-plan-1.md`

### BLOCKER

| ID | Status | What changed |
|---|---|---|
| B1 | **APPLIED** | Task 6 Step 1: `tree.menus[&Street::River].oop.donk` -> `tree.menus[&Street::River].donk`, with the message "donk is a field of PlayerMenus, not of SideMenu". |
| B2 | **APPLIED** | Task 21 (`iso_stabilizer_tiebreak`): the equality now uses the paired flop `AsAh2c7s` / `AsAh2c7h` (both canonicalize to `[0,49,50,21]`); the rainbow pair `AhKd2c7s` / `AhKd2c7h` is kept as an explicit `assert_ne!` with the verified keys `[0,45,50,23]` / `[0,45,50,22]` in a comment. The neighbouring turn/river-order `assert_ne!` is untouched. |
| B3 | **APPLIED** | Task 25 (`equity_mc_within_standard_error`): hero `"7h7d"` -> `"7h7s"`, and the indices are decoupled (`boards[(k / 4) % 5]`, hero period 5, villain period 4) so the 20 spots are 20 distinct (hero, board, villain) triples. The `continue` guard became a hard `assert!` (no spot can be silently skipped) and `assert!(checked >= 18)` became `assert_eq!(checked, 20, "spec 13.1 measures 20 spots")`. All 20 hero/board pairs were re-checked for card conflicts. |

### MAJOR

| ID | Status | What changed |
|---|---|---|
| M1 `StreetRootSnapshot.bb_chips` | **APPLIED** (option (a)) | Field kept. Spec §4.3 revision 6 now carries it (amendment S1), so it is no longer a deviation: deviation item 1 says so. Task 3's Interfaces and the handover table mark it a **required** literal field, and the new "Reconciliations the consuming plans own" list files the plan-2 edit (its Task 4 Step 1 and Task 16 Step 1 add `bb_chips`). Cross-plan M9 marks this **C**, so plan 1 itself does not change shape. |
| M2 `BeginHand` | **APPLIED**, via the cross-plan resolution | Not the rename the review suggested. Cross-plan M6/Or4 and spec §4.3 revision 6 (S16) resolve the three shapes into **two named types**: `proto::BeginHand { button, hero, dealt, stacks, hero_cards }` (id-free admission DTO) is added in Task 3 with its own round-trip test, and `core_model::BeginHand { hand_id, ..., stacks_start, ... }` (Task 13) is unchanged. Plan 2's `Engine::begin_hand` takes the DTO and converts; plan 5 no longer adds one. Side effect handled: three core-model test files that glob-import both crates now carry an explicit `use core_model::state::BeginHand;`. |
| M3 `RootError` | **APPLIED** (second branch of the review's own edit) | The `Multiway { pot_eligible: u8 }` payload is kept, because spec §3.5 **revision 6** (amendment S2) declares exactly the five variants including the payload — the review's premise ("spec §3.5 declares `Multiway` without a payload") was true of revision 5 only. Cross-plan M7 marks this **C (plan 2)**. Recorded in the handover table (five variants, `step: u32` not `usize`) and in the reconciliation list (plan 2: `Preflop` -> `Classification::Preflop`, `Inconsistent{step}` -> `Unsupported{EngineError{retryable:false}}`; plan 3's match arm takes the payload). |
| M4 `proto::worker` names | **APPLIED** except the `Lock` shape | `ReadyInfo` renamed to `Ready` (struct and `WorkerMessage::Ready(Ready)`). `proto::worker` now defines `pub use crate::PROTO_VERSION;`, `pub const SOLVER_COMMIT: &str = "9d15…dfda1";` and `pub const ADAPTER_VERSION: u16 = 1;`, with a new wire test `worker_identity_constants_match_the_ready_line` asserting they equal the §4.5 `ready` line. `EngineMessage::Lock { id, spot, locks }` **stays a struct variant**: cross-plan M3 marks that **C (plan 2)** (wire-identical, 1 producer site vs a `LockRequest` newtype plan 2 can delete). The note also states that `solver-worker` re-exports `SOLVER_COMMIT` rather than defining a second copy. |
| M5 `Card::parse` | **APPLIED** | `pub fn parse(s: &str) -> Result<Card, CardParseError> { s.parse() }` added to `impl Card`, asserted in `card_ids_match_spec`, and listed in the handover table with the explicit statement that **no `ProtoError` exists** in the workspace (plan 2 drops it). |
| M6 fixture pot layering | **APPLIED** | `tools/gen_fixtures.py` gains `normalize_pots(state, ring)`, which folds a PokerKit layer into the previous one when the eligible set is equal or empty; `snapshot()` calls it. The module docstring now names three normalizations. The schema prose, Task 11's Interfaces (the layering rule is declared normative there) and Task 18's policy all state the same rule. A new pytest, `test_pot_layers_follow_core_model_merging`, asserts no adjacent equal/empty eligible sets survive. |
| M7 toolchain | **APPLIED** per orchestrator decision 2 | The repo-wide `stable-x86_64-pc-windows-msvc` pin is **kept**. Global Constraints gain a Toolchain bullet stating it is repo-wide, that R8's figures are GNU, and the V1 check verbatim; Task 1 gains **Step 3**, which writes `scripts/build-worker-gnu.ps1` (complete PowerShell) as the documented fallback target, and Task 1's commit list includes it. Matching spec §3.6 edit made (see §4 below). |

### MINOR

| ID | Status | What changed |
|---|---|---|
| m1 | **APPLIED** | `sha2` dropped from the Architecture sentence ("`sha2` belongs to `core-ranges`, which owns the hashing"). |
| m2 | **APPLIED** | `serde` moved from `[dependencies]` to `[dev-dependencies]` in `crates/core-model/Cargo.toml` (with the reason inline). For `core-iso`, the "(tests only)" claim is replaced by a real use: `tests/iso.rs` now parses a Pio range with `core_ranges::parse_range` and round-trips it under every permutation, so the §3.2-mandated dependency is exercised. |
| m3 | **APPLIED** | `EquityMode::MonteCarlo { seed, max_samples }` and `exact_cost` are recorded as spec §3.5/§7 amendment **S3** (already in revision 6), not as a plan-local choice. |
| m4 | **APPLIED** | The `pokers` replacement is recorded as spec §3.2 amendment **S4** (already in revision 6), with the explicit note that plan 4's bench/gate text must not expect `pokers` in any `Cargo.toml`. |
| m5 | **APPLIED** | The interim `mc.rs` of Task 24 now returns nothing: its body is `unreachable!("Monte Carlo lands in Task 25; no test of Task 24 requests EquityMode::MonteCarlo")`. Unused imports (`result`, `EquityStatus`) were removed so the file still compiles warning-free. |
| m6 | **APPLIED** | `sample_joint_holes` gains the same 10,000,000-rejection cap as `monte_carlo` and returns the partial vector instead of spinning. |
| m7 | **APPLIED** (documentation branch) | Signatures unchanged (spec §3.5 gives `derive` no `Result`). Task 13's Interfaces and doc comments on `derive` and `settle_pots` state the precondition: they are called only on states this crate built, and any externally supplied `HandState` must be validated with `lifecycle::simulate` (Task 12), which returns `Result`. Splitting `simulate` into its own task (Task 12) makes that fallible entry point a first-class, separately reviewable deliverable. |
| m8 | **APPLIED** | `Hand` is `Copy` and `Hand::len() -> usize` added to the "Verified environment and library facts" list, and cross-referenced in the type-consistency section. |
| m9 | **APPLIED** | "16 spec fields" -> "17 spec fields". |
| m10 | **APPLIED** | `expand_body` gains a doc comment stating the token precedence (`random`, explicit combo, `+`, `-`, plain shape) and why `AKs+` never reaches the explicit-combo branch. |
| m11 | **APPLIED** | `serde_json` moved to `[dev-dependencies]` in `crates/proto/Cargo.toml`, with the reason inline. |
| m12 | **APPLIED** | `oracle_map()` is cached behind a `OnceLock` (`fn oracle_map() -> &'static [u16]` + `build_oracle_map`), and the wall-time claim in "Running everything" is restated as **1-3 minutes** with the three reasons named. Task 23's expected-runtime line was updated too. |

---

## 2. `REVIEW-cross-plan.md` — items naming plan 1

### §1 Interface match

| # | Direction | Status |
|---|---|---|
| M1 `Menu`/`RaiseSize` -> `SideMenu`/`MenuSize`, bet menus carry `a` | **P (plan 1)** | **APPLIED.** `MenuSize::{Pot(f32), AllIn}` (untagged: positive number or `"a"`), `SideMenu { bet: Vec<MenuSize>, raise: Vec<MenuSize> }`, `PlayerMenus { oop: SideMenu, ip: SideMenu, donk: Option<Vec<MenuSize>> }`. Serde impls, the `RIVER_ORACLE_TREE` assertions, the file-structure row and the handover table follow. A new test `bet_menus_carry_all_in_entries` covers §10.1 `river_std_v1` (`[0.33, 0.75, "a"]`), the §13.1 T4 `a`-only bet menu, the explicit empty donk list, the absent donk on the root street and the rejection of a non-positive fraction. Wire form unchanged, so §4.5's `"bet":[1.0]` still round-trips byte for byte. |
| M2 `Card::parse` / `ProtoError` | **P (plan 1)** | **APPLIED** (see MAJOR M5). |
| M3 `EngineMessage::Lock` shape | **C (plan 2)** | **NOT APPLIED to plan 1** — by design: the cross-plan rule change-the-consumer applies, and the orchestrator's instruction is to leave plan 1 as is where the consumer changes. Recorded in Task 7's Interfaces and in the reconciliation list so plan 2 deletes `LockRequest`. |
| M4 `ReadyInfo` -> `Ready` | **P (plan 1)** | **APPLIED.** |
| M5 `proto::worker::{PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}` | **P (plan 1)** | **APPLIED** (= Or6). |
| M6 `BeginHand` field set | both | **APPLIED** on plan 1's side: `proto::BeginHand` added in Task 3 (= Or4); `core_model::BeginHand` unchanged. Plan 2/plan 5 obligations recorded. |
| M7 `RootError` variants and payload width | **C (plan 2)** | **NOT APPLIED to plan 1.** Frozen shape and plan-2 obligations recorded. |
| M8 `EquityRequest`/`EquityResult` shape | **C (plan 2)** | **NOT APPLIED to plan 1.** Plan 1's shape is now written out in full in the handover table (`EquityRequest { board, players, mode, pots }`, `EquityResult { status, method, shares, samples, elapsed }`, `EquityMode::MonteCarlo { seed, max_samples }`), and the reconciliation list names both plan-2 sites (`crates/engine/src/equity.rs` and the Task 14 `river_check_only_terminal_oracle`) plus the replacement of plan 2's local `mode_for` by `core_eval::exact_cost`. |
| M9 `StreetRootSnapshot` literals omit `bb_chips` | **C (plan 2)** | **NOT APPLIED to plan 1.** Recorded as a required literal field. |
| M18 dependency versions | **P (plan 1 workspace)** | **APPLIED.** `[workspace.dependencies]` now pins `sha2 = "0.10.9"` (was `"0.11"`) and keeps `thiserror = "2.0"`; the Tech Stack line and the verified-versions line were updated with the reason (one `Digest` trait for `hash_scaled` and plan 4's cache key). A new Global Constraints bullet requires every crate of plans 1-5 to use `.workspace = true` for `serde`, `serde_json`, `thiserror`, `sha2` and `version/edition/license`. |
| M19 toolchain | **C (plan 2)** | **NOT APPLIED to plan 1** as a change of pin — the pin is kept per orchestrator decision 2. Plan 1 now carries the V1 re-verification, the 25% threshold and the `solver-worker`-only GNU fallback script, and records that plan 2 drops its "GNU toolchain for this plan" line. |
| M21 `resolve_chip_path` implemented three times | **C (plans 2, 4)** | **NOT APPLIED to plan 1.** The handover table names `proto::resolve_chip_path` "the single implementation of the §2 rule" and the reconciliation list requires plans 2, 3 and 4 to re-export it. |
| M10-M17, M20 | plans 2/3/4/5 | Out of scope for this plan; untouched. |

### §2 Duplicates and orphans

| # | Status |
|---|---|
| Or4 `proto::BeginHand` -> plan 1 Task 3 | **APPLIED.** |
| Or6 `proto::worker::{SOLVER_COMMIT, ADAPTER_VERSION}` + `worker::PROTO_VERSION` -> plan 1 Task 7 | **APPLIED.** |
| Or2 `bench oracle` -> plan 4 Task 20 | **APPLIED as a correction:** "Running everything" said "`bench oracle` of plan 2 wraps this"; it now says plan 4 Task 20 and notes plan 2's CLI is `run` / `gen-spots` / `materialize` only. |
| D2 chip-path resolution | **APPLIED as documentation** (single implementation stays in `proto`; consumers re-export). |
| D7 two-combo river range definition | **APPLIED as documentation, not as the file swap.** The alternative resolution (have `wire_examples.rs` load `fixtures/worker/river_two_combo.jsonl`) is **NOT APPLIED**: that fixture is produced by plan 2 Task 6, which runs after plan 1 Task 7 in the cross-plan execution order, so plan 1 cannot depend on it. `river_two_combo_ranges()` keeps a doc comment naming it the definition, and the reconciliation list makes plan 2's generator owe the combo-by-combo cross-check — the review's own second option. |
| D9 root `Cargo.toml` members | **APPLIED as documentation** in Task 1's Interfaces (plan 2 adds only `"solver-worker"` and the `exclude`). |
| Or1, Or3, Or5, Or7, Or8; D1, D3, D4, D5, D6, D8, D10 | Not plan 1; untouched. |

### §3 Spec changes

S1-S6 and S17 originate from plan 1's own deviations and are already in spec revision 6. The plan's "Gaps and deliberate deviations"
list was rewritten to cite them as accepted amendments rather than claim open deviations, and three new entries were added
(the `SideMenu`/`MenuSize` rename, the kept MSVC pin with its V1 check, and the workspace dependency pinning).

### §4 Execution order

Phase 0 items that touch plan 1 (M1, M4, M5, M6, Or4, Or6, S1-S6, S16) all land in Tasks 1, 3, 6 and 7 — the first tasks executed —
so `proto` does not need re-cutting after five crates depend on it.

### §5 Oversized tasks — splits (orchestrator decision 4)

| Old | New | Rationale |
|---|---|---|
| T11 (458 lines, 3 modules, 4 §13.1 tests) | **T11** `settlement` (refund / layering / conservation, new `tests/settlement.rs` with 3 tests incl. the 202/[0,5] merge and the `max_by_key` tie), **T12** `lifecycle::simulate` (new `tests/lifecycle_sim.rs` with 3 tests built from literal `HandState`s, incl. the three rejection cases), **T13** `state` public API (the original `tests/lifecycle.rs` with the four §13.1 tests) | Each unit is independently testable and reviewable; the split also gives m7's "validate with `simulate` first" a real, separately gated entry point. |
| T13 (430 lines: project + generator + 200 fixtures) | **T15** `tools/` project (new `tests/test_environment.py` pinning `pokerkit==0.7.5` / `phevaluator==0.6.0` and asserting the entry points the generators use), **T16** `gen_fixtures.py` + pytest, **T17** generate and commit the 200 fixtures (with an inventory check and a byte-reproducibility check via a second run + `git status`) | Same. T15 gets a real test so it is not a bare scaffolding commit. |

All tasks after the splits were renumbered (old 12->14, 13->16, 14->18, 15->19, 16->20, 17->21, 18->22, 19->23, 20->24, 21->25) and
**every** cross-reference was updated: task Interfaces blocks, `expect(...)` messages in test code, the handover table, the Self-review
coverage table, the deviation list and the type-consistency bullets. Step numbering was verified to be 1..N in all 25 tasks.

---

## 3. Other changes made while applying the above

- **Test-count expectations** in Tasks 3, 4, 5, 6, 7 and 16 were adjusted for the tests added (`begin_hand_dto_is_id_free_and_roundtrips`, `bet_menus_carry_all_in_entries`, `worker_identity_constants_match_the_ready_line`, `test_pot_layers_follow_core_model_merging`).
- **`test_generated_set_covers_spec_classes`**: the `max_pots >= 2` threshold was lowered from 60 to 20 with an explanatory comment, because after `normalize_pots` that stat counts *genuine* side pots (distinct eligible sets) rather than PokerKit's raw contribution levels. The 2026-09-10 measured figure of 86 was taken on the raw decomposition, so Task 16's expected-output line now says the side-pot count must be re-recorded from the normalized run and 86 must not be reused. Every other recorded figure (95 straddle hands, 65/25 all-in players, 36 short all-ins, 138 refunds, 23/56/121 finals, 1 dropped seed) is unaffected by the normalization and was kept.
- **`scripts/build-worker-gnu.ps1`** is a new file created by Task 1 and added to the file-structure table and Task 1's commit.
- **"Running everything"** was rewritten: realistic wall time, the corrected `bench oracle` owner, the Python test counts per task, and the toolchain check.
- **"Interfaces handed to plans 2-5"** was rewritten as a frozen-names table plus an explicit "Reconciliations the consuming plans own" list (10 entries) so each downstream plan sees exactly what it must change.

---

## 4. Spec edit

`docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` **§3.6, third bullet** — replaced

> `solver-worker` and every non-Tauri crate build on either toolchain …; week-1 core and worker work proceeds on the GNU toolchain while the workload is pending, and the whole workspace is built once with MSVC before the E2E test.

with a bullet that keeps the first clause and then states: the repository pins `stable-x86_64-pc-windows-msvc` in `rust-toolchain.toml`
from week 1 (with the measured justification), so core, worker and bench are all MSVC and there is no second toolchain file; **V1 must
build and run the pinned-solver example on MSVC with `+avx2` and compare its FLOP-FAST time with R8 §5 before any timing is compared
with R8**; and if that build fails or is more than 25% slower, the `solver-worker` binary alone is built with
`cargo +stable-x86_64-pc-windows-gnu` through `scripts/build-worker-gnu.ps1` while the rest of the workspace stays MSVC.

Nothing else in the spec was touched.

**Residual inconsistency left in place deliberately** (the instruction was to keep everything else in the spec untouched): §14's week-1
paragraph (line 766) still reads "core and worker work proceeds on the GNU toolchain meanwhile". That clause is now contradicted by
§3.6 and should be struck in the next spec revision. It affects no plan-1 task.

---

## 5. Nothing else was left unapplied

Every BLOCKER and MAJOR of `REVIEW-of-plan-1.md` is applied, and all 12 MINORs are applied (each was trivial). The only cross-plan
items not applied to plan 1 are the ones the cross-plan review itself marks **C (the consumer changes)** — M3, M7, M8, M9, M19, M21 —
plus D7's file-swap alternative, which the execution order forbids; all seven are recorded in the plan's reconciliation list so the
consuming plans can act on them.
