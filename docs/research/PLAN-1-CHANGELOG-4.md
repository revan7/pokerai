# Plan 1 changelog 4 — execution errata (revision 3 -> 4)

Date: 2026-09-17

## Preamble

Plan 1 (`docs/superpowers/plans/2026-09-10-plan-1-foundation.md`, revision 3) was executed task-by-task on branch `phase-c` under `.superpowers/sdd/2026-09-10-plan-1-foundation/`. Each of its 25 tasks was implemented, then independently reviewed by a different agent against the plan's own task text. A number of reviews found that the plan's prescribed code — the sample implementation and, in a few cases, the sample tests printed inline in a task's Step block — carried a defect: it matched the brief exactly, but violated the spec, a global constraint, or a standing ruling made earlier in execution. These are recorded here as **plan-mandated** findings, distinct from ordinary implementer deviations.

Per the orchestrator's ruling recorded in `.superpowers/sdd/2026-09-10-plan-1-foundation/progress.md` ("plan-text corrections found during execution are collected into ONE errata changelog per plan at the end of that plan's execution ... instead of a revision bump per finding"), this document collects every such correction in one place instead of bumping the plan's revision line once per finding. **The plan's task text is not rewritten.** A reader of the plan's Step blocks will still see the original, defective sample code; the corrected behavior lives only in the committed source and in this changelog. The code and this changelog are the authoritative record of what was actually built; the plan document is now a historical snapshot of what was originally proposed.

Findings that were about the implementer's own work (not the plan's prescribed code), about report/journal evidence hygiene, or about orchestration process are excluded from this changelog — they were resolved in their own fix rounds and are not plan-text defects. One exception, `Task 10 — folded seats in the action order`, is called out explicitly below as NOT an erratum, since a superficial reading of its review could suggest otherwise.

### Standing rulings that drove most corrections

Five rulings, established early in execution and recorded in `.superpowers/sdd/2026-09-10-plan-1-foundation/constraints.md` item 14, recur across nearly every erratum below:

- **(a) Validate wide, then narrow, never clamp.** Numeric input from text or wire is validated in its wide form (`f64` / `u64`) *before* narrowing to `f32` / `u32`; invalid input is rejected, never silently clamped into range.
- **(b) Always-on `assert!` for infallible constructors.** Internal constructors and preconditions with infallible signatures enforce their invariants with always-on `assert!` (never `debug_assert!`), naming the offending index or value.
- **(c) Widen chip arithmetic to `u64`.** Chip arithmetic that can exceed `u32` on valid per-seat inputs is widened to `u64` (or `u64`/typed rejection at admission) before comparing or summing.
- **(d) No module-level warning filters.** No `warnings.simplefilter(...)`-style blanket suppression; expected warnings are caught narrowly, scoped to the exact operation that raises them.
- **(e) Tests must fail when required committed artifacts are missing.** A test that is supposed to check a committed fixture must fail — not silently skip — when that fixture is absent.

A sixth, cross-cutting pattern is visible across the range/tree/wire findings (Tasks 4, 6, 7, 19) even though it is not separately numbered in `constraints.md`: **every wire type validates on both serde directions** — an outbound `Serialize` must reject a value that would violate the type's own inbound `Deserialize` check, not merely forward it (which, for floats, silently becomes JSON `null`).

---

## Errata by task

### Task 1 — workspace skeleton (P1.T1)

**Erratum 1 — GNU fallback script does not check exit codes**

- **Task/step:** Step 3, `scripts/build-worker-gnu.ps1` (plan lines 216-217).
- **Plan text (<=25 words):** `cargo +stable-x86_64-pc-windows-gnu build --release -p solver-worker --target x86_64-pc-windows-gnu` (line 217; preceded by an equally unchecked `rustup toolchain install` at line 216).
- **What the code does instead:** The script checks `$LASTEXITCODE` immediately after both `rustup toolchain install` and `cargo ... build`, and throws before any staging if either is nonzero. A regression script, `scripts/tests/build-worker-gnu.tests.ps1`, stubs a failing `cargo` against a pre-existing stale `target\x86_64-pc-windows-gnu\release\solver-worker.exe` and asserts the script throws and never copies that stale binary into `target\release`.
- **Why:** R1, `task-1-review.md`, Important, plan-mandated — in Windows PowerShell 5.1, `$ErrorActionPreference = "Stop"` does not turn a native process's nonzero exit code into a terminating exception, so a failed native build fell through to the existing artifact-existence check, which was satisfied by a stale binary from an earlier successful build; the script then staged that stale binary and printed a success message.
- **Commit(s):** `a74e9b3` (fix round 1, base `831f26c`).

**Erratum 2 — premature `holdem-hand-evaluator` profile override**

- **Task/step:** Step 1, `Cargo.toml` (plan lines 127-128).
- **Plan text (<=25 words):** `[profile.dev.package.holdem-hand-evaluator]` / `opt-level = 3` (lines 127-128).
- **What the code does instead:** The override was removed from `Cargo.toml` in this task, since no crate yet depends on `holdem-hand-evaluator` (it only exists as a `[workspace.dependencies]` declaration at this point) and an unmatched profile override emits a `cargo` warning. The override returns in Task 23, the task that first makes `core-eval` actually depend on the evaluator.
- **Why:** R2, `task-1-review.md`, Important, plan-mandated — the reported gate emitted `warning: profile package spec `holdem-hand-evaluator` in profile `dev` did not match any packages`, a present configuration defect and repeated build noise.
- **Commit(s):** `a74e9b3` (removal, fix round 1, base `831f26c`); `f97fe1a` (Task 23, override re-added alongside the real dependency).

### Task 2 — proto cards and combos (P1.T2)

- **Task/step:** Step 1, `crates/proto/src/cards.rs` (plan lines 347, 392, 399).
- **Plan text (<=25 words):** `debug_assert!(rank < 13 && suit < 4);` (line 347); similarly `debug_assert!(a != b);` (line 392) and `debug_assert!((i as usize) < COMBOS);` (line 399).
- **What the code does instead:** All three guards (in `Card::new`, `combo_index`, `combo_cards`) are always-on `assert!`s carrying descriptive panic messages, with `# Panics` doc comments added. Three `#[should_panic]` regression tests reproduce the exact invalid inputs and are confirmed to fail under `--release` before the fix and pass after.
- **Why:** R1, `task-2-review.md`, Important, plan-mandated (standing ruling (b)) — with debug assertions disabled, `Card::new(0, 4)` returned `Card(4)`, `combo_index(Card(1), Card(1))` returned `1`, and `combo_cards(1326)` returned `[Card(0), Card(52)]` instead of panicking.
- **Commit(s):** `5a1ee13` (fix round 1, base `68fb01f`).

### Task 3 — proto game config and hand-state types (P1.T3)

- **Task/step:** Step 1, `crates/proto/tests/` unknown-field regression (plan line 478).
- **Plan text (<=25 words):** `text.replace(r#""stacks""#, r#""stacks_start""#)` (line 478), asserted to fail deserialization as proof of `deny_unknown_fields`.
- **What the code does instead:** The test now keeps a fully valid `BeginHand` payload and adds an extra field (`hand_id`), asserting that deserialization fails only because of that extra field; a positive control without the extra field is retained.
- **Why:** R1, `task-3-review.md`, Important, plan-mandated — renaming a required field (`stacks` -> `stacks_start`) makes the required `stacks` field go missing, so deserialization fails regardless of whether `deny_unknown_fields` is present; the only negative test for the admission boundary could not detect loss of that guard.
- **Commit(s):** `fcdb385` (fix round 1, base `289077e`).

### Task 4 — `proto::Range1326` (P1.T4)

**Erratum 1 — serialization does not validate weights**

- **Task/step:** Step 1, `crates/proto/src/range.rs`, `Serialize` impl (plan line 769).
- **Plan text (<=25 words):** `for w in self.0.iter() { seq.serialize_element(w)?; }` (line 769).
- **What the code does instead:** `Serialize` validates all 1326 weights for finiteness and `[0,1]` before opening the sequence, returning `serde::ser::Error::custom` naming the offending combo index rather than forwarding an invalid value.
- **Why:** R1, `task-4-review.md`, Important, plan-mandated — a focused probe showed NaN and either infinity serialized successfully as JSON `null`, and `-0.1`/`1.5` serialized successfully as literal out-of-domain numbers, all of which the type's own decoder then rejected; spec section 2 requires validation at every boundary.
- **Commit(s):** `9a64528` (fix round 1, base `8b4c90a`).

**Erratum 2 — deserialization narrows before validating**

- **Task/step:** Step 1, `crates/proto/src/range.rs`, `visit_seq` (plan line 782).
- **Plan text (<=25 words):** `seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?` decoded directly into `f32` before the domain check (line 782).
- **What the code does instead:** The incoming numeric value is validated in its wider `f64` representation before narrowing to the stored `f32`, per standing ruling (a).
- **Why:** R2, `task-4-review.md`, Important, plan-mandated — `1.00000001` rounded to and was accepted as `1.0`, and `-1e-50` rounded to and was accepted as `-0.0`; both source numbers were outside `[0,1]`.
- **Commit(s):** `9a64528` (fix round 1, base `8b4c90a`).

### Task 5 — recommendation, coverage and event types (P1.T5)

- **Task/step:** Step 1, `crates/proto/src/recommendation.rs`, `ApproxReason` (plan line 874).
- **Plan text (<=25 words):** `AsymmetricStacks { stacks_bb: Vec<f32> },` (line 874).
- **What the code does instead:** Added `prominent: bool` to the variant, matching spec §4.4/§8.3's requirement to distinguish a small stack mismatch from one exceeding 5% of the depth used; a round-trip test covers both `true` and `false`.
- **Why:** R1, `task-5-review.md`, Important, plan-mandated — the variant as written in the plan's snippet cannot carry the required prominence flag that spec §13.1 explicitly exercises (e.g. 100-vs-104 not prominent, 100-vs-110 prominent).
- **Commit(s):** `b159b75` (fix round 1, base `8fc0afc`).

### Task 6 — effective tree and chip-path resolution (P1.T6)

**Erratum 1 — `MenuSize::Pot` domain not validated on either serde direction**

- **Task/step:** Step 3, `crates/proto/src/tree.rs`, `MenuSizeVisitor::visit_f64` and `Serialize` (plan line 1091, and the `Serialize` match at line 1081).
- **Plan text (<=25 words):** `if v.is_finite() && v > 0.0 { Ok(MenuSize::Pot(v as f32)) } else { Err(...) }` (line 1091).
- **What the code does instead:** Validates both the incoming `f64` and the narrowed `f32` for finiteness and strict positivity on deserialize; `Serialize` now rejects an invalid public `MenuSize::Pot` value instead of forwarding it.
- **Why:** R1, `task-6-review.md`, Important, plan-mandated — a probe showed JSON `1e39` accepted as `Pot(inf)` and `1e-50` accepted as `Pot(0.0)` on deserialize, and zero/negative/NaN/infinite values serialized successfully (NaN/infinity as JSON `null`) since `Pot` is publicly constructible.
- **Commit(s):** `aca740a` (fix round 1, base `4d49c4c`).

**Erratum 2 — chip-path ordinal index silently wraps**

- **Task/step:** Step 3, `crates/proto/src/tree.rs`, `resolve_chip_path` (plan line 1151).
- **Plan text (<=25 words):** `ordinal.push(i as u8);` (line 1151).
- **What the code does instead:** Replaced with `u8::try_from(i).ok()?`, returning `None` for an index the plan's `u8` ordinal cannot represent.
- **Why:** R2, `task-6-review.md`, Important, plan-mandated — with 257+ distinct actions, index 256 cast to `0`, so a path selecting the 257th action could resolve to an unrelated node at ordinal `[0]`.
- **Commit(s):** `aca740a` (fix round 1, base `4d49c4c`).

**Erratum 3 — terminal marker skipped on the final edge**

- **Task/step:** Step 3, `crates/proto/src/tree.rs`, `resolve_chip_path` (plan lines 1152-1153).
- **Plan text (<=25 words):** `if k + 1 == path.len() { break; }` (line 1152), which skips the `terminal_pots.get(i)` check on line 1153 for the path's last action.
- **What the code does instead:** The terminal-marker check now also runs on the final edge (before any early exit), so a terminal child cannot be returned as if it were a decision node.
- **Why:** R3, `task-6-review.md`, Important, plan-mandated — with a root `Check` marked terminal (`Some(100)`) and a stray materialized node at `[0]`, resolving `[Check]` incorrectly returned `Some([0])` instead of `None`.
- **Commit(s):** `aca740a` (fix round 1, base `4d49c4c`).

### Task 7 — `proto::worker` wire messages (P1.T7)

**Erratum 1 — float wire fields narrow silently**

- **Task/step:** Step 1, `crates/proto/src/worker.rs`, float-bearing fields (plan lines 1370, 1389, e.g. `pub probs: Vec<Vec<f32>>, pub ev_chips: Vec<Vec<f32>>, pub available: Vec<bool>,`).
- **Plan text (<=25 words):** `pub probs: Vec<Vec<f32>>, pub ev_chips: Vec<Vec<f32>>, pub available: Vec<bool>,` (line 1370); `exploitability_chips: Option<f32>` (line 1389), both using derived serde.
- **What the code does instead:** Reusable serde float codecs decode into `f64`, check finiteness/representability/domain, then narrow to `f32`, applied to every float field (probabilities, EVs, both exploitability fields, progress); invalid in-memory values are rejected on serialize instead of becoming JSON `null`.
- **Why:** T7-R1, `task-7-review.md`, Important, plan-mandated (standing ruling (a)) — derived serde accepted progress `1e39` as `Some(inf)`, probability `1.00000001` as `1.0`, and `-1e-50` as `-0.0`; `Some(NaN)`/`Some(INFINITY)` serialized successfully as `null`.
- **Commit(s):** `746625d` (fix round 1, base `0f8864a`).

**Erratum 2 — required nullable `exploitability_chips` accepts a missing key**

- **Task/step:** Step 1, `crates/proto/src/worker.rs`, `Progress` (plan lines 1193, 1389). Plan line 1193 states the intended contract: "optional fields omitted when `None` except `exploitability_chips`, which is always present (`null` until measured)."
- **Plan text (<=25 words):** `exploitability_chips: Option<f32>,` (line 1389) — a plain `Option` implemented against a field the plan's own prose (line 1193) calls "always present."
- **What the code does instead:** `exploitability_chips` deserializes through a required-nullable helper that rejects a missing key while still accepting an explicit `null`.
- **Why:** T7-R2, `task-7-review.md`, Important, plan-mandated — a `Progress` object with no `exploitability_chips` key deserialized successfully as `None`, conflating "key absent" with the field's legal "measured absent" null value; spec §4.5 marks the field required and nullable.
- **Commit(s):** `746625d` (fix round 1, base `0f8864a`).

### Task 8 — solution and lock validation (P1.T8)

**Erratum 1 — nonzero fold EV passes validation**

- **Task/step:** Step 1, `crates/proto/tests/validate_solution.rs`, positive fixture (plan line 1434).
- **Plan text (<=25 words):** `let mut ev = vec![vec![1.5; a]; 1326];` (line 1434), giving every available action — including `Action::Fold` — an EV of 1.5 in the fixture asserted to validate successfully.
- **What the code does instead:** The validator identifies fold columns from `node.actions` and rejects any nonzero EV on them, including on available rows; the fixture now sets fold EVs to zero.
- **Why:** T8-R1, `task-8-review.md`, Important, plan-mandated — the binding EV convention (spec section 2, and `NodeStrategy.ev_chips` at spec §4.5) requires fold EV exactly 0, but the validator never checked the fold column and the plan's own fixture violated the rule.
- **Commit(s):** `38daf09` (fix round 1, base `d5cf931`).

**Erratum 2 — rejection tests are not isolating**

- **Task/step:** Step 1, `crates/proto/tests/validate_solution.rs`, boundary rejection cases (plan line 1452, e.g. `s.nodes[0].probs[7] = vec![-0.1, 1.1]; // sums to 1 but leaves [0, 1]`).
- **Plan text (<=25 words):** `s.nodes[0].probs[7] = vec![-0.1, 1.1];` (line 1452) — combines an upper-bound and lower-bound violation in one row, so either guard alone still rejects it.
- **What the code does instead:** Added independent cases that isolate each rejection rule (e.g. `[1.0001, 0.0]` violates only the upper bound while the sum stays within tolerance), plus NaN/infinity probability rows, invalid export strings, negative/non-finite exploitability, an over-cap node count, and a row-sum-tolerance boundary case, so deleting any one guard fails a specific test.
- **Why:** T8-R2, `task-8-review.md`, Important, plan-mandated — static inspection showed several independent validator guards could be deleted without failing any of the plan's supplied tests, because every negative example also tripped a different, still-present guard.
- **Commit(s):** `38daf09` (fix round 1, base `d5cf931`).

### Task 9 — core-model skeleton: errors, cards, positions, config (P1.T9)

- **Task/step:** Step 1, `crates/core-model/src/positions.rs`, `validate_table` (plan line 1815).
- **Plan text (<=25 words):** `if s.amount_chips < 2 * cfg.bb_chips { return Err(RulesError::FormatUnsupported { detail: "short straddle post".into() }); }` (line 1815) — the multiplication happens in `u32`.
- **What the code does instead:** Widened to `u64::from(s.amount_chips) < 2 * u64::from(cfg.bb_chips)`, preserving the existing `FormatUnsupported` result, per standing ruling (c).
- **Why:** R1, `task-9-review.md`, Important, plan-mandated — with `bb_chips = 2_147_483_648` (representable in `u32`) the mathematical minimum straddle is `4_294_967_296`, outside `u32`; with overflow checks enabled the comparison panicked, and with checks disabled `2 * cfg.bb_chips` wrapped to `0`, so any straddle amount passed.
- **Commit(s):** `4125eeb` (fix round 1, base `0aecb07`).

### Task 10 — `core-model::betting::Round` (P1.T10)

- **Task/step:** Step 1, `crates/core-model/src/betting.rs`, `post` (plan line 2025, within the preflop bring-in logic).
- **Plan text (<=25 words):** `self.facing = self.facing.max(self.committed[i]);` (line 2025) — sets the amount other players must call from the actual chips posted, with no floor at the configured blind.
- **What the code does instead:** The nominal preflop bring-in (`min_bet`/configured blind) is preserved as the facing amount independently of what a short-stacked blind actually posted; the short blind's own commitment and all-in flag are unaffected.
- **Why:** R2, `task-10-review.md`, Important, plan-mandated — with blinds 1/2 and a 1-chip big-blind stack, the plan's formula let UTG's call drop to 1 and minimum raise-to drop to 3 instead of the correct call 2 / raise-to 4; a short big blind still requires other players to enter for the full nominal blind.
- **Commit(s):** `50ca0fe` (fix round 1, base `a5f1bfa`).

**Not an erratum — folded seats in the action order.** The same review's R1 (Critical) found that `Round::open`'s constructor added an `assert!(!folded[i], ...)` precondition rejecting folded seats from the action order. That assertion is **not** in the plan's sample code; it was an extra restriction the implementer added beyond the brief, which the plan's own later `Sim::open_street` caller (plan line ~2443) violates by design. It was fixed in the same commit (`50ca0fe`) by dropping the added assertion, but it does not belong in this plan-text errata list.

### Task 11 — core-model settlement (P1.T11)

- **Task/step:** Step 0 interfaces / Step 1, `crates/core-model/src/settlement.rs`, `layer_pots` and `check_conservation` signatures (plan line 2144).
- **Plan text (<=25 words):** `layer_pots(contributed: &[u32; 6], folded: &[bool; 6]) -> Vec<Pot>` (line 2144) — per-seat `u32` contributions layered directly into `u32` pot amounts, with no aggregate bound stated anywhere in the task.
- **What the code does instead:** Widened intermediate summation to `u64` and added the explicit aggregate precondition `sum(stacks_start) <= u32::MAX`, enforced as a typed `RulesError` at hand admission (Task 12/13), so settlement itself can keep its infallible signature while the unrepresentable case is rejected before it is ever reached.
- **Why:** R1, `task-11-review.md`, Important, plan-mandated — individually representable `u32` contributions (e.g. three contributions of 1,500,000,000) produce a combined pot or merged layer total exceeding `u32::MAX`, and the plan's own comment ("contributions are shares of the starting stacks, so every layer total fits in `u32`") does not follow from the per-seat types; the implementation reproduced this as a panic rather than a defined rejection.
- **Commit(s):** `88037cd` (fix round 1, base `50d295f`); admission check landed in Task 12 (`75ceb8d`, base `e3d05da`).

### Task 12 — `core-model::lifecycle::simulate` (P1.T12)

**Erratum 1 — folded seats remain eligible in already-settled pots**

- **Task/step:** Step 1, `crates/core-model/src/lifecycle.rs`, `Sim::derived` snapshot (plan line 2462).
- **Plan text (<=25 words):** `pots: self.pots.clone(),` (line 2462) — `self.pots` is rebuilt only in `close`, not refreshed after a fold on the current street.
- **What the code does instead:** After a fold, already-settled pot eligibility is refreshed against the new folded flags (preserving the chip total); live current-street commitments stay outside settled pots until closure.
- **Why:** R1, `task-12-review.md`, Important, plan-mandated — with button 5, dealt `[5,0,1]`, a flop fold by seat 5, and BTN still owing a response, the snapshot still listed the folded seat 5 as eligible for the already-settled 6-chip pot; the committed `h0001` PokerKit fixture independently removes it immediately.
- **Commit(s):** `75ceb8d` (fix round 1, base `e3d05da`).

**Erratum 2 — a matched sole survivor is left in `Betting`**

- **Task/step:** Step 1, `crates/core-model/src/lifecycle.rs`, closure condition (plan line 2487).
- **Plan text (<=25 words):** `if sim.round.eligible_count() == 1 || sim.round.closed() { sim.close()?; }` (line 2487).
- **What the code does instead:** Closure is now recognized when at most one eligible player retains chips and that player owes no response to the outstanding wager, even if `Round` still carries a redundant pending `Check` entry, so the hand proceeds to `Complete { AllInRunout }` and refunds correctly instead of waiting on a fictitious check.
- **Why:** R2, `task-12-review.md`, Important, plan-mandated — with SB posting its last chip, BB posting the full blind, and BTN calling all-in, every opponent of BB is all-in and BB already matches the wager, yet the plan's condition left the snapshot `Betting { street: Preflop }` with BB pending instead of closing and refunding BB's unmatched chip.
- **Commit(s):** `75ceb8d` (fix round 1, base `e3d05da`).

**Erratum 3 — the external-state replay gate does not validate the board**

- **Task/step:** Step 1, `crates/core-model/src/lifecycle.rs`, `simulate`'s street-opening loop (plan line 2490).
- **Plan text (<=25 words):** `if state.board.len() >= street.board_len() { sim.open_street(state, street); } else { break; }` (line 2490) — opens streets on board length alone.
- **What the code does instead:** `simulate` now validates board length (0/3/4/5), card uniqueness, and hero-card collision before replay, and at successful completion requires the supplied board to match the board actually reached by replay (rejecting partial, future, or unreached board data) rather than trusting whatever `HandState.board` an external caller supplied.
- **Why:** R3, `task-12-review.md`, Important, plan-mandated — the plan's gate accepted a duplicate flop, a six-card board, a one-card partial flop, a flop dealt before preflop closed, and a flop colliding with hero's known cards; spec §4.3 restricts `HandState.board` to validated 0/3/4/5-card entry, and `simulate` is the only validation boundary for an externally supplied `HandState`.
- **Commit(s):** `75ceb8d` (fix round 1, base `e3d05da`).

### Task 13 — `core-model` public state API (P1.T13)

- **Task/step:** Interfaces note (plan line 2527).
- **Plan text (<=25 words):** "`derive` and `settle_pots` have no `Result` (spec §3.5) and panic on a state that does not replay... `abandon` all refresh through `simulate` and return `Err`..." (line 2527) — this sentence describes `abandon` alongside the fallible mutators without noting that `abandon` itself is infallible and its `derive()` call can panic via `expect`.
- **What the code does instead:** `abandon`'s own doc comment now states the precondition explicitly (it requires a replayable state) and carries a `# Panics` section; no behavior changed, since `abandon` already correctly returned `HandState` rather than `Result`.
- **Why:** R1, `task-13-review.md`, Minor (not Important, but carried forward as a mandatory documentation-only fix at the next task's dispatch per the reviewer's ruling) — the plan's own prose gives `abandon` the wrong failure contract, grouping it with functions that "return `Err`" when it is in fact infallible and panics via `derive`'s `expect`.
- **Commit(s):** `49bef40` (Task 14's commit, which the ledger records as "includes the T13 doc side edit"; Task 13's own commit `57005d0` was reviewed clean with this Minor carried forward rather than fixed in place).

### Task 16 — PokerKit fixture generator (`tools/gen_fixtures.py`) (P1.T16)

- **Task/step:** Step 1, `tools/gen_fixtures.py`, module import (plan line 3347).
- **Plan text (<=25 words):** `warnings.simplefilter("ignore")  # Mode.CASH_GAME warns on folds that face no wager; never recorded as legal` (line 3347).
- **What the code does instead:** The module-level blanket filter is removed. The generator avoids calling `state.can_fold()` when `owed == 0` (testing `owed > 0` first) instead of relying on suppression, and any remaining expected PokerKit warnings are caught with narrowly scoped `warnings.catch_warnings()` blocks around the specific operation.
- **Why:** R1, `task-16-review.md`, Important, plan-mandated (standing ruling (d)) — a process-wide `simplefilter("ignore")` installed by importing the generator silenced every warning category for any caller, and the reported pytest run under this same mechanism still showed 32 warnings, showing the suppression did not even achieve its stated purpose.
- **Commit(s):** `8ade620` (fix round 1, base `5e320b2`).

### Task 19 — core-ranges Pio range strings and 169-class expansion (P1.T19)

**Erratum 1 — text weight validated after narrowing**

- **Task/step:** Step 1, `crates/core-ranges/src/parse.rs`, `parse_weight` (plan line 4012).
- **Plan text (<=25 words):** `let w: f32 = text.trim().parse().map_err(...)?;` followed by the domain check (line 4012).
- **What the code does instead:** Parses and validates a wider numeric representation before narrowing to `f32`, per standing ruling (a).
- **Why:** R1, `task-19-review.md`, Important, plan-mandated — `1.00000001` rounded to and passed as `1.0`, and `-1e-50` rounded to and passed as `-0.0` (erasing a selected class), both via `parse_range`.
- **Commit(s):** `87b68ca` (fix round 1, base `c8aef4c`).

**Erratum 2 — `expand_169` builds an unvalidated `Range1326`**

- **Task/step:** Step 1, `crates/core-ranges/src/parse.rs`, `expand_169` (plan line 4050).
- **Plan text (<=25 words):** `Range1326::from_fn(|i| classes[class_of(i) as usize])` (line 4050) — copies every supplied class weight unchecked.
- **What the code does instead:** Added an always-on precondition assertion over all 169 input weights (finiteness and `[0,1]`) before constructing the range, naming the invalid class, per standing ruling (b); the infallible signature is preserved.
- **Why:** R2, `task-19-review.md`, Important, plan-mandated — `Range1326::from_fn` itself performs no validation, so `expand_169(&[f32::NAN; 169])` produced 1326 NaN weights and `expand_169(&[1.5; 169])` produced 1326 out-of-domain weights, silently violating the global range invariant.
- **Commit(s):** `87b68ca` (fix round 1, base `c8aef4c`).

### Task 20 — `hash_scaled` and blocking (P1.T20)

- **Task/step:** Step 1, `crates/core-ranges/tests/ranges.rs`, zero-range hash test (plan line 4116).
- **Plan text (<=25 words):** `assert_eq!(hash_scaled(&Range1326::zero()), hash_scaled(&Range1326::zero()));` (line 4116) — a self-comparison of the same deterministic function on identical input.
- **What the code does instead:** Replaced with a frozen, independently computed SHA-256 digest for the 5,304-byte all-zero encoding (`650334c621b6458c000e9fa79435730c304f4e4f12e48108590969e715006239`) and a second frozen digest for an asymmetric normalized vector, pinning both the all-zero branch and the little-endian byte order.
- **Why:** T20-Q1, `task-20-review.md`, Important, plan-mandated — the self-comparison would still pass if the all-zero encoding changed from `0.0` to `1.0`, or if `to_le_bytes()` were replaced by `to_be_bytes()`, so the plan's own prescribed test pinned no actual encoding.
- **Commit(s):** `74b9ed8` (fix round 1, base `9e086c0`).

### Task 21 — suit permutations and canonical boards (P1.T21)

**Erratum 1 — `-0.0` weights are dropped by `apply_range`**

- **Task/step:** Step 1, `crates/core-iso/src/lib.rs`, `apply_range` (plan line 4339).
- **Plan text (<=25 words):** `if w != 0.0 {` (line 4339) — skips copying any weight equal to zero, including negative zero.
- **What the code does instead:** Transfers weights whose bit pattern is not positive-zero, preserving `-0.0` through permutation instead of dropping it to an implicit `0.0`.
- **Why:** R1, `task-21-review.md`, Important, plan-mandated — `Range1326` accepts and preserves `-0.0` as a distinct finite weight in `[0,1]`, but the plan's brief requires comparing actual `f32` bit patterns for the tie-break; with a range containing `-0.0` at one combo, this produced a wrong permutation choice (a strictly larger bit-pattern serialization was preferred by mistake).
- **Commit(s):** `ef3e28c` (fix round 1, base `c00de8a`).

**Erratum 2 — `SuitPerm` accepts non-bijective maps**

- **Task/step:** Step 1, `crates/core-iso/src/lib.rs`, `SuitPerm` (plan line 4303).
- **Plan text (<=25 words):** `pub struct SuitPerm(pub [u8; 4]);` (line 4303) — a public tuple with derived (unchecked) deserialization and no invariant enforcement anywhere.
- **What the code does instead:** Deserialization rejects non-bijective/out-of-range suit maps; the bijection invariant is also enforced with an always-on assertion at public operation boundaries (e.g. `inverse`, `apply`), per standing ruling (b).
- **Why:** R2, `task-21-review.md`, Important, plan-mandated — both `[0,0,2,3]` (duplicate image) and `[0,1,2,4]` (out-of-range image) decoded successfully; `inverse` then silently returned a wrong result for the first and panicked on an out-of-bounds index for the second.
- **Commit(s):** `ef3e28c` (fix round 1, base `c00de8a`).

**Erratum 3 — `CanonicalBoard` accepts invalid or noncanonical boards**

- **Task/step:** Step 1, `crates/core-iso/src/lib.rs`, `CanonicalBoard` (plan line 4319).
- **Plan text (<=25 words):** `pub struct CanonicalBoard { cards: Vec<Card> }` (line 4319) — a private-field type with derived (unchecked) deserialization.
- **What the code does instead:** Deserialization now validates length, card domain, uniqueness, and canonical form before constructing a board (rejecting a noncanonical payload rather than silently re-canonicalizing it); `canonicalize` rejects repeated cards.
- **Why:** R3, `task-21-review.md`, Important, plan-mandated — `{"cards":[]}` decoded successfully and then `flop()` panicked; a valid but noncanonical card sequence decoded unchanged; `canonicalize` accepted repeated cards and constructed an impossible board — all reachable through this task's own public API.
- **Commit(s):** `ef3e28c` (fix round 1, base `c00de8a`).

### Task 22 — evaluator oracle generator (`tools/gen_eval_oracle.py`) (P1.T22)

- **Task/step:** Step 1, `tools/tests/test_gen_eval_oracle.py`, committed-fixture assertions (plan line 4458).
- **Plan text (<=25 words):** `if committed.exists():` (line 4458) — the five-card fixture's size/prefix assertions run only if the committed file happens to be present; the seven-card test never opens the committed 200k-sample file at all.
- **What the code does instead:** The five-card committed-fixture test now requires the file to exist (no conditional skip) before asserting its exact size and prefix; a new seven-card committed-fixture test requires `fixtures/eval/phevaluator_7card_200k.bin` to exist and be exactly 1,800,000 bytes, comparing a prefix against independently constructed records with `random.Random(20260910)`, per standing ruling (e).
- **Why:** R1, `task-22-review.md`, Important, plan-mandated — both required committed oracle files could be entirely absent and these tests would still pass, masking an incomplete checkout of this task's primary deliverables.
- **Commit(s):** `26aa8cf` (fix round 1, base `79ff343`).

### Task 23 — `core-eval` evaluator adapter (P1.T23)

- **Task/step:** Step 1, `crates/core-eval/src/evaluator.rs`, `BinaryEvaluator` (plan lines 4678, 4683).
- **Plan text (<=25 words):** `Hand::from_slice(&ids)` (line 4678) with no card-ID or duplicate validation; `debug_assert!((5..=7).contains(&hand.len()), "evaluate needs 5 to 7 cards");` (line 4683).
- **What the code does instead:** Card IDs and uniqueness (including overlap between a partial board and `extra`) are validated before calling into the backend; the 5-to-7-card count precondition is an always-on assertion, per standing ruling (b); an opaque validated partial-hand wrapper keeps a raw, unchecked backend `Hand` from bypassing the adapter's constructor.
- **Why:** R1, `task-23-review.md`, Important, plan-mandated — the pinned `holdem-hand-evaluator` backend performs unchecked array access in both `add_card` and `evaluate` and documents that invalid card counts "may crush" (crash); the plan's adapter forwarded raw IDs directly to that unchecked backend, and the count guard vanished entirely in release builds.
- **Commit(s):** `456a4ed` (fix round 1, base `f97fe1a`).

### Task 24 — exact equity, per-combo equity, terminal payoffs (P1.T24)

**Erratum 1 — assignment search bypasses deadlines and cancellation**

- **Task/step:** Step 1, `crates/core-eval/src/equity.rs`, `ExactRun::assign` (plan lines 4885, 4947).
- **Plan text (<=25 words):** `if self.evals % 4096 == 0 { ... }` (line 4885, inside `Deadline::tick`, only called after a rank evaluation) and `if self.used[a] || self.used[b] { continue; }` (line 4947, a rejected-candidate path that never reaches `tick`).
- **What the code does instead:** Added an immediate entry deadline/cancel check plus a bounded-cadence poll during assignment traversal that also covers rejected candidates, with work-poll accounting kept separate from the public rank-evaluation count; completion is checked before reporting `Ready`/`InvalidRanges`.
- **Why:** R1, `task-24-review.md`, Important, plan-mandated — a six-player request whose last two seats collide on every assignment (`exact_cost = 9,150,625`, all below the exact-cost threshold) performed zero rank evaluations, never polled `Deadline::tick`, and returned `InvalidRanges` after 84 ms against a 1 ms budget instead of `BudgetExceeded`; spec section 7's cancellable equity budget and the §13.1 `equity_budget_respected` test require timely cancellation regardless of whether any evaluation occurred.
- **Commit(s):** `22a41b5` (fix round 1, base `f9a8f9d`).

**Erratum 2 — tests never exercise nonuniform joint weights**

- **Task/step:** Step 1, `crates/core-eval/tests/equity.rs`, range fixtures (plan line 4750, e.g. `player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board)`).
- **Plan text (<=25 words):** `player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board)` (line 4750) — every range in the plan's supplied tests uses unit combo weights.
- **What the code does instead:** Added a hand-derived case with unequal weights and a partial collision (two compatible tuples of different weight), asserting a specific fractional equity split that only holds if per-combo weights are actually multiplied into the joint probability.
- **Why:** R2, `task-24-review.md`, Important, plan-mandated — replacing the weighted product with an unweighted count would leave every one of the plan's own supplied test expectations unchanged, so the defining weighted-equity behavior had no regression coverage.
- **Commit(s):** `22a41b5` (fix round 1, base `f9a8f9d`).

**Erratum 3 — allocation inside the innermost tally loop (orchestrator finding, applied alongside R1/R2)**

- **Task/step:** Step 1, `crates/core-eval/src/equity.rs`, `Tally::award` (plan line 4907, `let winners: Vec<usize> = idx.iter().copied().filter(|i| ranks[*i] == best).collect();`).
- **Plan text (<=25 words):** `let winners: Vec<usize> = idx.iter().copied().filter(|i| ranks[*i] == best).collect();` (line 4907) — allocates a `Vec` per pot per showdown.
- **What the code does instead:** Replaced with a count-then-award two-pass over eligible indices that avoids allocating in the innermost loop, which the exact-mode enumeration can call up to roughly 10^7 times under the section-7 exact-cost ceiling.
- **Why:** Orchestrator finding O1 (`.superpowers/sdd/2026-09-10-plan-1-foundation/progress.md`, Task 24 entry), applied under the same ruling as R1/R2 — not a Codex review finding, but folded into the same fix round because it touches the same hot path the deadline fix modifies.
- **Commit(s):** `22a41b5` (fix round 1, base `f9a8f9d`).

### Task 25 — Monte Carlo equity with joint-disjoint sampling (P1.T25)

**Erratum 1 — compatibility DFS ignores deadline and cancellation**

- **Task/step:** Step 1, `crates/core-eval/src/mc.rs`, `compatible` and its caller (plan line 5212, `fn compatible(supports: &[Support], used: &mut [bool; 52], player: usize, steps: &mut u64) -> bool`).
- **Plan text (<=25 words):** `fn compatible(supports: &[Support], used: &mut [bool; 52], player: usize, steps: &mut u64) -> bool { ... }` (line 5212) — a pure step-capped DFS with no deadline or cancel-flag parameter.
- **What the code does instead:** The deadline is threaded into the compatibility traversal; `tick_work()` is polled for every candidate, and interruption is propagated distinctly from a compatible/incompatible/step-cap-unknown result, with a stop check before returning any preflight terminal status.
- **Why:** T25-R1, `task-25-review.md`, Important, plan-mandated — a six-player request whose incompatibility takes ~4 million candidate visits to prove returned `InvalidRanges` after a 1 ms budget or with cancellation already set, in both cases with zero samples and no observed stop, instead of `BudgetExceeded`/`Cancelled`.
- **Commit(s):** `aaf9174` (fix round 1, base `f812e6b`).

**Erratum 2 — the ten-million-rejection cap is dropped as an `InvalidRanges` trigger**

- **Task/step:** Step 1, `crates/core-eval/src/mc.rs`, rejection-cap fallback (plan lines 5241, 5271).
- **Plan text (<=25 words):** `if rejections > 10_000_000 { break; }` (line 5241) and `if samples == 0 && rejections > 10_000_000 { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }` (line 5271).
- **What the code does instead:** `InvalidRanges` is reserved for an empty support or a completed incompatibility proof only; reaching the rejection cap with no witnessed sample now continues polling toward the budget/cancel stop (or falls through to a bounded exact check when affordable) and returns `BudgetExceeded`/`Cancelled` with zero samples instead of `InvalidRanges`. The ten-million-rejection `InvalidRanges` clause from the plan is dropped entirely.
- **Why:** T25-R2, `task-25-review.md`, Important, plan-mandated — with a proven-compatible but extremely rare tuple (probability ~1e-12, e.g. `AcAd:1.0,KcKd:1e-12` vs `AcAh:1.0` on a board where the aces collide), exact mode returns `Ready` with a real answer while the plan's rejection-cap rule returned `InvalidRanges` for the same valid, weighted input after exhausting ten million failed draws; spec §13.1 (line ~706) reserves `InvalidRanges` for the absence of a compatible assignment, which this case does not have.
- **Commit(s):** `aaf9174` (fix round 1, base `f812e6b`).

---

## Process rulings recorded during execution

These are orchestration decisions from `.superpowers/sdd/2026-09-10-plan-1-foundation/progress.md`'s "Rulings" — not plan-text defects, and not part of the errata list above.

- **Worktrees:** wave 1 (P1.T1) ran a single implementer directly in this tree; from wave 2 onward, parallel implementers worked in isolated worktrees (`Agent isolation: worktree`), each merged into `phase-c` by the orchestrator.
- **Journal handling:** wave-2+ implementers working in worktrees do not append to `docs/log/` directly (parallel appends would conflict at merge); each puts its journal entry text in its report, and the orchestrator appends it at merge time.
- **Review-package base = merge-base:** task review ranges are computed against the merge-base of each worktree branch, not an arbitrary prior commit.
- **Short-hash diff names:** review/fix diff package filenames use short commit hashes; two review dispatches (T8R, T24R) hit a packaging error from a full-hash alias mismatch and were relaunched with corrected short-hash paths, with no code change involved.
- **Vendored-crate warnings out of the gate's scope:** compiler warnings originating in the vendored `holdem-hand-evaluator` dependency are not treated as this workspace's warning-gate findings (T14R Minor, deferred to the final review).
- **SDD workspace retained:** this plan's `.superpowers/sdd/` workspace is kept after the final review (gitignored; the journal cites its review reports as evidence), contrary to the executing skill's default delete-on-completion step.

---

## Addendum (2026-09-17): final whole-branch review fixes

Plan 1's final whole-branch review (`.superpowers/sdd/2026-09-10-plan-1-foundation/final-review.md`) found no Critical findings and four Important findings (S1, S2, S3, S10) against code the 25 task reviews above had already passed; the single fix round that followed (`final-fix-report.md`, commits `f270f53`, `c49a0a5`, `3b8607d`, `fbc4ee8`, plus the flake-fix `f13f8e0` and the residual gate fix `37bec95`) changed the interfaces and behaviours below relative to the plan's original Step-block text. As with every erratum above, the plan's own text is not rewritten and the plan stays at revision 4; this addendum is the record of what the code does instead.

### S1 — checked float validation extended to `game.rs`, `tree.rs` thresholds, and all of `recommendation.rs`

- **Task/step:** Task 3 Step 1, `crates/proto/src/game.rs`, `Rake::PotRake` (plan line 502); Task 6 Step 3, `crates/proto/src/tree.rs`, `EffectiveTree` (plan lines 1138-1140); Task 5 Step 1, `crates/proto/src/recommendation.rs`, `ActionAdvice` and the file's other `f32`/`Option<f32>` fields (plan line 921 representative).
- **Plan text (<=25 words):** `PotRake { rate: f32, cap_mchips: u32, no_flop_no_drop: bool },` (line 502); `pub add_allin_threshold: f32, pub force_allin_threshold: f32, pub merging_threshold: f32,` (lines 1138-1140); `pub frequency: Option<f32>, pub ev_bb: Option<f32>,` (line 921) — all four plain derived serde, no domain check.
- **What the code does instead:** the wide-then-narrow, symmetric-serialize float codecs already built for Task 4's `Range1326` and Task 6's `MenuSize`, and generalized in Task 7's worker wire, were promoted into a shared `proto::numeric` module and applied to every remaining `f32`/`Option<f32>` in these three files: `Rake::PotRake.rate` (domain `[0,1)`), `EffectiveTree`'s three thresholds (non-negative finite), and all of `recommendation.rs` (unit-interval or finite per field).
- **Why:** final-review finding S1, Important — measured `PotRake` accepting `rate: 1e39` as `inf` and `rate: -0.5`; `EffectiveTree` accepting `add_allin_threshold: inf, force_allin_threshold: -1`; and `ActionAdvice{frequency: Some(NaN)}` silently round-tripping to `frequency: None` across the Tauri IPC boundary — the same class of defect the T4/T6/T7 fixes above closed, left open on three files the earlier task reviews didn't probe.
- **Commit(s):** `f270f53`.

### S2 — one materialized-path index per `validate_solution`/`validate_locks` call

- **Task/step:** Task 8 Step 1, `crates/proto/src/worker.rs`, `validate_solution`/`validate_locks` (plan lines 1538-1539, 1562, 1592).
- **Plan text (<=25 words):** `let ordinal = resolve_chip_path(materialized, path)...; let node = materialized.iter().find(|m| m.path == ordinal).expect(...)` (lines 1538-1539) — run once per solution/lock node, and `resolve_chip_path` (Task 6, line 1147) itself rebuilds a full index every call.
- **What the code does instead:** `tree.rs` gained `index_materialized`/`resolve_chip_path_indexed`; `validate_solution` and `validate_locks` each build the index once at the top of the call and look up both the resolved path and the node in it, instead of rebuilding an index and running a second linear scan per node.
- **Why:** final-review finding S2, Important — measured quadratic cost, on the order of 100s of pure path resolution at the declared 100,000-node `MAX_EXPORTED_NODES` cap, inside plan 2's and plan 4's per-result/per-cache-entry fast path.
- **Commit(s):** `f270f53`.

### S3 — allocation-free exact enumeration

- **Task/step:** Task 24 Step 1, `crates/core-eval/src/equity.rs`, `ExactRun::runouts` (plan lines 4962, 4965).
- **Plan text (<=25 words):** `let deck: Vec<Card> = (0..52u8).map(Card).filter(|c| !self.used[c.0 as usize]).collect();` and `let mut board_cards = self.req.board.clone();` (lines 4962, 4965) — both rebuilt on every leaf tuple, including on the river where `deck` is never read.
- **What the code does instead:** `deck`, `board_cards` and `idx` became reusable buffers hoisted into `ExactRun`, cleared/refilled per tuple instead of reallocated; the river (`k == 0`) skips the deck build entirely and reuses a `PartialHand` combined once per run.
- **Why:** final-review finding S3, Important — measured roughly 70% of exact-enumeration wall time as these two allocations on a river spot (175,252 disjoint tuples); `mc.rs` already used hoisted buffers and the exact path had never been given the same treatment.
- **Commit(s):** `c49a0a5`.

### S10 — `per_combo_equity` takes a caller-supplied budget and cancel flag

- **Task/step:** Task 24 Step 1, `crates/core-eval/src/equity.rs`, `per_combo_equity` (plan lines 5015, 5021).
- **Plan text (<=25 words):** `pub fn per_combo_equity(hero: &Range1326, villain: &Range1326, board: &[Card]) -> [f32; COMBOS] {` (line 5015) calling `let res = exact(&req, Duration::from_secs(3600), &never);` (line 5021) — a fixed one-hour budget, a permanently-false cancel flag.
- **What the code does instead:** `per_combo_equity(hero, villain, board, budget: Duration, cancel: &AtomicBool) -> (EquityStatus, [f32; COMBOS])`, sharing one `Deadline` and one hoisted villain `Support` across all hero combos, polled once per hero combo, returning `Cancelled`/`BudgetExceeded` with the partial result on a stop.
- **Why:** final-review finding S10, Important — the function was the only public `core-eval` entry point that could not be cancelled; spec §5's decision flow requires abandonable work when hero's decision changes, and fixing the signature after plan 2's `river_check_only_terminal_oracle` depends on it would be a breaking change.
- **Commit(s):** `c49a0a5`.

### S5 — `-0.0` normalized to `+0.0` at range ingestion

- **Task/step:** Task 4 Step 1, `crates/proto/src/range.rs`, `RangeVisitor::visit_seq` (plan line 782); Task 19 Step 1, `crates/core-ranges/src/parse.rs`, `parse_weight` (plan lines 4012-4013).
- **Plan text (<=25 words):** `let w: f32 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?;` (line 782); `let w: f32 = text.trim().parse().map_err(|_| RangeError::Weight(format!("cannot parse weight {text:?}")))?;` (line 4012) — both accept `-0.0` as a valid, distinct weight once in `[0,1]`.
- **What the code does instead:** both ingestion points map the validated weight through `(raw as f32) + 0.0` before storing it, folding `-0.0` to `+0.0` on the way in; `hash_scaled`, `apply_range` and `canonicalize` are unchanged and stay bit-exact, per the standing T20/T21 rulings.
- **Why:** final-review finding S5, Minor — `-0.0` passes the existing `[0,1]` domain check and then hashes differently from `+0.0` via `hash_scaled`'s `to_le_bytes()`, so a range and its own `range_to_string` round-trip, though numerically identical, produced different cache keys; closes the T20 and T21 parked notes at their single common root cause.
- **Commit(s):** `f270f53` (proto), `fbc4ee8` (core-ranges).

### S9 — `terminal_payoff` computes its pot/rake arithmetic in `f64`

- **Task/step:** Task 24 Step 1, `crates/core-eval/src/equity.rs`, `terminal_payoff` (plan lines 4874-4877).
- **Plan text (<=25 words):** `let pot = pot as f32;` (line 4875) ... `equity * (pot - r)` (line 4877) — the pot itself, not only the `f32` return, carried through the arithmetic.
- **What the code does instead:** widens `pot`, `rate` and `cap_mchips` to `f64` for the subtraction and multiplication, narrowing to `f32` once at the end; the public signature and spec §2's `f32` EV output are unchanged.
- **Why:** final-review finding S9, Minor — measured `terminal_payoff(1.0, 100_000_001, TimeCharge) = 100_000_000`, an off-by-one-chip error, because `f32` loses precision above 2^24 and a pot of that size is reachable at the spec's chip scale.
- **Commit(s):** `c49a0a5`.

### S7 / S8 — always-on guards in `positions.rs` and `config.rs`

**S7 — `positions.rs` seat-count guard**

- **Task/step:** Task 9 Step 1, `crates/core-model/src/positions.rs` (plan line 1826).
- **Plan text (<=25 words):** `debug_assert!((3..=6).contains(&n));` (line 1826) — the one survivor of standing ruling (b) left in the plan-1 tree, guarding a slice index the next line performs.
- **What the code does instead:** `assert!((3..=6).contains(&n), "positions: {n} dealt seats; 3 to 6 are supported");`, always-on and naming the count.
- **Why:** final-review finding S7, Minor — `positions` is `pub` and re-exported at the crate root with nothing forcing `validate_table` to run first; in release the guard compiled out and the next line panicked as a raw, unhelpful slice-index-out-of-range instead.
- **Commit(s):** `3b8607d`.

**S8 — `straddle_posts` zero-straddle guard**

- **Task/step:** Task 9 Step 1, `crates/core-model/src/config.rs`, `straddle_posts` (plan lines 1862-1864).
- **Plan text (<=25 words):** `cfg.straddle.map(|s| { let unit = s.amount_chips as f32; [cfg.sb_chips as f32 / unit, cfg.bb_chips as f32 / unit, 1.0] })` (lines 1862-1864) — divides by `amount_chips` with no precondition on it.
- **What the code does instead:** adds `assert!(s.amount_chips > 0, "straddle_posts: the straddle amount is at least one chip");` as the closure's first statement, always-on per standing ruling (b).
- **Why:** final-review finding S8, Minor — a zero `amount_chips` produced `posts = [inf, inf, 1.0]`, which then hit the same undeserializable-wire failure mode as S1 once serialized into `StraddleMapped`, even though `validate_table` already rejects `amount_chips < 2 * bb_chips` elsewhere.
- **Commit(s):** `3b8607d`.

### T3 — `monte_carlo` rejects a zero sample cap

- **Task/step:** Task 25 Step 1, `crates/core-eval/src/mc.rs`, `monte_carlo` (plan line 5250).
- **Plan text (<=25 words):** `pub fn monte_carlo(req: &EquityRequest, seed: u64, max_samples: u32, budget: Duration, cancel: &AtomicBool) -> EquityResult {` (line 5250) — no precondition on `max_samples`.
- **What the code does instead:** adds `assert!(max_samples > 0, "monte_carlo: max_samples is at least one sample")` at the top, always-on in `check_request`'s existing style.
- **Why:** final-review finding T3, Minor — `max_samples == 0` never entered the sampling loop and reported `EquityStatus::BudgetExceeded` with zero samples, blaming the clock for a caller error that `EquityStatus` (a plan-2 contract) has no honest value for.
- **Commit(s):** `c49a0a5`.

### Release-only gate on the exact-enumeration throughput test

- **Task/step:** not a plan-text erratum — `exact_enumeration_at_the_selection_ceiling_finishes_inside_the_budget` is a test the S3/T2 fix round itself added to `crates/core-eval/tests/equity.rs` (see S3 above and `final-fix-report.md` §16), with no counterpart in the plan's own text to quote.
- **Plan text (<=25 words):** n/a — the test did not exist in the plan; see above.
- **What the code does instead:** the test is gated `#[cfg(all(feature = "exhaustive", not(debug_assertions)))]` instead of a plain `#[cfg(feature = "exhaustive")]`, so it runs only in the release profile.
- **Why:** the scoped re-review's finding NEW-1 — spec §13.1's 0.5s budget for an enumeration at the §7 selection ceiling is a release-profile figure; measured in debug the fixture reaches only 14,151,680 of its 36,796,320 evaluations in 500ms (an earlier, marginal debug run had passed at 488ms, 12ms of headroom against a bound the spec never promised for that profile); release measures 405ms, 19% of headroom.
- **Commit(s):** `37bec95`.

### S6 — parked, not applied

`Round` (`crates/core-model/src/betting.rs`) and `Sim` (`crates/core-model/src/lifecycle.rs`) keep public fields rather than gaining private fields with read accessors, as final-review finding S6, Minor, recommended. The orchestrator parked this as a plan-2 follow-up (`final-fix-report.md` §1 table: "Not applied — parked by the orchestrator"); no commit.

### Parked for the user

- Clippy component (`rustup component add clippy`) is not installed; no first-party lint evidence can exist until it is, and installing it is a machine-level change no agent should make (final-review §5 item 1).
- A third vendored patch to `third_party/postflop-solver`, to silence its six `mismatched_lifetime_syntaxes` warnings, was considered and declined; spec §3.7 pins exactly two patches in `PATCHES.md`, and adding a third changes the vendored-fork contract, which is the user's call (final-review §5 item 2).
- `SeCreateSymbolicLinkPrivilege` is needed for a plan-3 core-preflop symlink-containment test case and was not available in this environment; out of plan-1's scope, carried forward as a standing environment gap.
- The gitignored 10M-record 7-card oracle fixture (`fixtures/eval/phevaluator_7card_10m.bin`) stays absent from the worktree by design (spec §13.0); anyone running the crate-wide `cargo test -p core-eval --features exhaustive` command rather than scoping to `--test equity` must first run `tools/gen_eval_oracle.py` (`final-fix-report.md` §18).
