# Review of `docs/superpowers/plans/2026-09-10-plan-1-foundation.md` (2026-09-10)

Reviewed against spec revision 5 (`docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`), the decision outline
(`docs/design/2026-09-10-design-outline.md` §0b), the writing-plans skill, the series brief, and `docs/research/R8-solver-bench.md`.
Plan read in full (4,914 lines). Two claims were checked by running the plan's own algorithms (suit canonicalization,
orbit sizes); cross-plan interface use was checked by scanning plans 2-5.

**Verdict: NOT READY.** 3 BLOCKER, 7 MAJOR, 12 MINOR.

Summary: spec coverage for this plan's scope is complete (every §13.1 row for `proto`, `core-model`, `core-ranges`,
`core-iso`, `core-eval` is present under the spec's exact test name, with the spec's numbers), there are no placeholders,
and the hand-checked arithmetic in Tasks 10-12 is correct. The plan fails on three tests that cannot pass as written and
on seven interface names that plans 2-5 already consume under different shapes.

---

## BLOCKER

| ID | Task / step | Problem (quoted) | Concrete edit |
|---|---|---|---|
| B1 | Task 6, Step 1 (line 953) | `assert_eq!(tree.menus[&Street::River].oop.donk, None);` — `.oop` is a `Menu { bet, raise }`; `donk` is a field of `PlayerMenus`, not of `Menu`. Compile error in the first test of the task. | Replace with `assert_eq!(tree.menus[&Street::River].donk, None);` |
| B2 | Task 17, Step 1 (lines 3781-3783) | `let (a,_) = canonicalize(&cards("AhKd2c7s"), &[]); let (b,_) = canonicalize(&cards("AhKd2c7h"), &[]); assert_eq!(a, b, "the turn is canonicalized within the flop's stabilizer");` — false. `AhKd2c` is rainbow, so its stabilizer is trivial and the minimal perm is the identity; verified keys are `[0,45,50,23]` vs `[0,45,50,22]`. The two boards are genuinely different (7 of the ace's suit vs 7 of the unseen suit). | Use a flop with a non-trivial stabilizer: `canonicalize(&cards("AsAh2c7s"), &[])` and `canonicalize(&cards("AsAh2c7h"), &[])` (verified: both `[0,49,50,21]`). Keep the neighbouring `assert_ne!` on `AhKd2c7s7h` vs `AhKd2c7h7s` (verified distinct). |
| B3 | Task 21, Step 1 (lines 4586-4605) | `for k in 0..20 { ... let hero = heroes[k % 5]; ... if board.iter().any(...) { continue; } ... } assert!(checked >= 18);` — hero `"7h7d"` and board `"Kd7d2c"` are both at index 1 and share `7d`, so k = 1, 6, 11, 16 are skipped and `checked == 16`. The assertion fails, and the spec's "20 spots" (§13.1 `equity_mc_within_standard_error`) is not met. | Change `heroes[1]` from `"7h7d"` to `"7h7s"` (no board conflict on `Kd7d2c`); `checked` then reaches 20. Optionally decouple the indices (`boards[(k / 4) % 5]`) so the 20 spots are 20 distinct (hero, board, villain) triples rather than 5 pairs x 4 villains. |

---

## MAJOR

All seven are interface/verification defects; none require redesign.

| ID | Task / step | Problem | Concrete edit |
|---|---|---|---|
| M1 | Task 3, Step 4 (line 605) | `pub struct StreetRootSnapshot { ... pub bb_chips: u32 }` adds a field the spec §4.3 does not have. Plan 2 builds this struct with literals at two sites (`2026-09-10-plan-2-worker-engine.md:804` and `:3922`), both without `bb_chips` — they will not compile. | Deviation is justified (`replay_root` needs the minimum bet to reproduce the legal set of an unopened street), so keep the field but (a) record it in the plan's "Interfaces handed to plans 2-5" table as a *required* literal field, and (b) file the matching edit against plan 2's two `StreetRootSnapshot { .. }` literals. Alternative that needs no plan-2 edit: pass the minimum bet as a second argument, `replay_root(&StreetRootSnapshot, bb_chips: u32)`. |
| M2 | Task 11, Step 5 (line 2365) | `pub struct BeginHand { pub hand_id: u64, ..., pub stacks_start: Vec<u32>, ... }`. Plan 2 (`:44`, `:4383`, `:4419`) and plan 5 (`:76`) construct `BeginHand { button, hero, hero_cards, dealt, stacks }` — field named `stacks`, and no `hand_id` at all. | Rename `stacks_start` to `stacks` in `BeginHand` (the spec name `stacks_start` belongs to `HandState`, not to this input struct) and either drop `hand_id` from `BeginHand` (assign it in the engine, per spec §4.3 "assigned by the engine") or add it to the plan-2/plan-5 literals. State the final shape in the interfaces table. |
| M3 | Task 12, Step 3 (lines 2653-2665) | `RootError::{Multiway { pot_eligible: u8 }, ProjectionNotReproducing { step: u32 }, NoDecision, Preflop, Inconsistent { step: u32 }}`. Plan 2 (`:4470`) and plan 3 (`:1748`) match `RootError::Multiway` as a **unit** variant and neither has a `_` arm, so both matches are non-exhaustive and the pattern is wrong. Spec §3.5 declares `Multiway` without a payload. | Either drop the `pot_eligible` payload (plan 2 already recomputes it as `n` before calling `street_root`), or keep it and file edits against plan 2 `:4470` and plan 3 `:1748` plus a `_ =>` arm for `Preflop`/`Inconsistent`. Also fix plan 2's table (`:44`) which documents `step: usize` while plan 1 uses `u32`. |
| M4 | Task 7, Step 3 (lines 1219-1284) | `proto::worker` names drift from what plan 2 consumes in ~15 places: plan 1 has `ReadyInfo`, plan 2 imports `proto::worker::Ready`; plan 1 has `EngineMessage::Lock { id, spot, locks }`, plan 2 expects `EngineMessage::Lock(LockRequest)` with `LockRequest { id, spot, locks }`; plan 2 imports `proto::worker::{PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}` (`:1683`, `:3392`, `:3527`, `:3772`, `:5250`) — plan 1 defines `PROTO_VERSION` at the crate root only and defines neither `SOLVER_COMMIT` nor `ADAPTER_VERSION`. | In Task 7: rename `ReadyInfo` to `Ready`; add `pub struct LockRequest { id, spot, locks }` and make the variant `Lock(LockRequest)`; add `pub use crate::PROTO_VERSION;`, `pub const ADAPTER_VERSION: u16 = 1;` and `pub const SOLVER_COMMIT: &str = "9d1509fe5077d019825f833eed04b16d342dfda1";` to `worker.rs` (spec §3.7 pins the commit; plan 2's `include_str!("PINNED_COMMIT")` must then be checked against it, not duplicated). Update the interfaces table accordingly. |
| M5 | Task 2, Step 3 (lines 303-327) | `Card` gets `FromStr`, `Display`, `Card::checked`, `Card::all` but no `Card::parse`. Plan 2 calls `Card::parse(...)` / `proto::Card::parse(...)` 24 times. | Add to `impl Card`: `pub fn parse(s: &str) -> Result<Card, CardParseError> { s.parse() }`, and list it in the interfaces table. |
| M6 | Task 13 (line 3006) + Task 14, Step 1 (line 3270) | The generator writes `pots` straight from PokerKit (`[{"amount": p.amount, "eligible": sorted(...)} for p in state.pots]`) and Task 14 asserts `assert_eq!(d.pots, pots)` element by element, while `layer_pots` (Task 11) *merges* "adjacent layers with the same eligible set". Nothing in the plan records PokerKit's decomposition rule (or that `Pot.amount`/`Pot.player_indices` exist in 0.7.5). Task 14's instruction — "the fixture is the oracle: fix `core-model`, never the fixture" — would push an executor to un-merge `layer_pots`, which then breaks `side_pot_three_allins` / `allin_runout_single_survivor` (which expect `pot(202,[0,5])`, a merged layer). Deadlock across two tasks. | In `gen_fixtures.py::snapshot`, normalize before writing: build the pot list from PokerKit, then fold each pot into the previous one when `eligible` is equal or empty. Add one sentence to Task 13 stating the normalization, and to Task 14's Step 2 note that pot *totals* are the oracle and the layering rule is `core-model`'s. |
| M7 | Task 1, Step 1 (lines 125-129) | `rust-toolchain.toml` pins the **whole repository** to `stable-x86_64-pc-windows-msvc`. Spec §3.6 says "week-1 core and worker work proceeds on the GNU toolchain while the workload is pending, and the whole workspace is built once with MSVC before the E2E test"; R8 §5 records that the MSVC target could not be built at measurement time and that the vendored solver, `zstd-sys` and the ~2x AVX2 gain were all measured on GNU. The pin silently moves plan 2's AGPL solver build, its `build.rs` AVX2 check and every bench number onto an unvalidated toolchain. | Keep the pin only if the plan states, in Global Constraints, that it is repo-wide and that V1 in plan 2 must re-verify the vendored solver + `zstd` + `+avx2` under MSVC before any timing is compared with R8 (whose figures are GNU). Otherwise pin `channel = "stable-x86_64-pc-windows-gnu"` in plan 1 and switch in plan 5, as §3.6 describes. |

---

## MINOR

| ID | Task / step | Problem | Edit |
|---|---|---|---|
| m1 | Header, "Architecture" (line 7) | "it depends only on `serde`/`thiserror`/`sha2`" — Task 1's `crates/proto/Cargo.toml` has no `sha2` (correct; hashing lives in `core-ranges`). | Drop `sha2` from the sentence. |
| m2 | Task 9 / Task 17 Cargo.toml | `core-model` declares `serde` but no type in the crate derives serde; `core-iso` declares `core-ranges` but neither `src` nor `tests/iso.rs` uses it (the Interfaces block claims "`core_ranges::parse_range` (tests only)"). | Drop `serde` from `core-model`; keep `core-ranges` in `core-iso` (spec §3.2 lists it) but remove the "tests only" claim or use it in the test. |
| m3 | Task 20 Interfaces (line 4241) | `EquityMode::MonteCarlo { seed, max_samples }` extends spec §3.5's `MonteCarlo{seed}`. | Justified (the caller needs a sample cap alongside the budget); already declared in Self-review item 3 — keep, and mark it as a spec §3.5 amendment rather than a plan-local choice. |
| m4 | Header / Task 20 | `pokers` is dropped for an in-house weighted-equity implementation. Spec §3.2 names `pokers` (MIT) as the mechanism for `core-eval`. | Justified by R3 (`pokers 0.10` stores weights as `u8` percent and cannot carry `Range1326`); record it as an explicit §3.2 amendment so plan 4's bench/gate text does not expect `pokers` in `Cargo.toml`. |
| m5 | Task 20, Step 3 (lines 4544-4552) | The interim `mc.rs` returns `EquityStatus::BudgetExceeded` for every Monte Carlo request until Task 21 replaces it. | Acceptable (nothing calls it between tasks) but make the stub `unreachable!("Monte Carlo lands in Task 21")` or return `EquityStatus::InvalidRanges` so a mis-ordered execution fails loudly instead of silently reporting a budget overrun. |
| m6 | Task 21, Step 3 (lines 4737-4746) | `sample_joint_holes` loops `while out.len() < count` with no rejection cap, unlike `monte_carlo` (10,000,000). A request with no disjoint assignment hangs the test binary. | Add the same 10M rejection cap and return the partial vector (or panic with a message). |
| m7 | Task 11, Step 5 (lines 2391, 2432) | `derive` and `settle_pots` call `simulate(state).expect("a HandState built by core-model replays consistently")`. `HandState` is a serde type; a corrupted or externally supplied state aborts the host process. Spec §3.5 gives `derive` no `Result`, so the signature is correct. | Keep the signature; add an internal fallback that returns `Derived::default()` with `to_act: None` (and an empty `Settlement`) instead of panicking, or document that both are only ever called on states this crate produced. |
| m8 | Task 19, Step 3 (lines 4196-4201) | `let mut hand = *partial;` requires `Hand: Copy` and `hand.len()` requires `Hand::len`. Neither is in the plan's own "Verified environment and library facts" list (which names only `new`, `from_slice`, `add_card`, `evaluate`). | Add both to the verified-facts list, or write `let mut hand = partial.clone();` and drop the `debug_assert!` on `len()`. |
| m9 | Task 7 Interfaces (line 1107) | "`SolveRequest` (16 spec fields, `deny_unknown_fields`)" — §4.5 and the struct itself have 17. | Say 17. |
| m10 | Task 15, Step 3 (line 3510) | `expand_body` treats any 4-character token whose 2nd and 4th characters are in `cdhs` as an explicit combo, before the `+`/dash branches. Correct for every token in the tests, but the precedence is undocumented. | Add a one-line comment stating the precedence (explicit combo, then `+`, then dash, then shape). |
| m11 | Task 1, Step 1 (line 163) | `serde_json` is a normal dependency of `proto` although only `#[cfg(test)]` code uses it. | Move to `[dev-dependencies]` (unit tests inside the lib can use dev-dependencies). |
| m12 | "Running everything" (line 4813) | "about 15 s in the `dev` profile once the fixtures exist". `oracle_map()` re-evaluates all 2,598,960 five-card hands once per `core-eval` oracle test, `equity_mc_within_standard_error` performs roughly 2 x 10^7 evaluations, and `iso_class_count_1755` / `iso_orbit_sizes` each canonicalize 22,100 flops over 24 permutations with a `Vec` allocation per key. | Either cache `oracle_map()` behind a `OnceLock`, reuse `board_key`'s buffer, or restate the expected wall time (1-3 minutes is realistic at `opt-level = 1`). |

---

## Dimension-by-dimension result

**1. Spec coverage — PASS.** Every §13.1 row in this plan's scope exists under the spec's exact name, with the spec's
numbers: `state_machine_pokerkit_fixtures`, `straddle_action_order_utg`, `min_raise_and_short_allin_no_reopen`,
`cumulative_short_allins_reopen` (10/14/17 vs 10/15/19 with the 8-chip full raise), `side_pot_three_allins`
(150/100, 100 returned, invariant at every step), `side_pot_two_contested` (200/150/200, no refund, sum 550),
`allin_runout_single_survivor` (both directions, 202 pot, 200 refund), `dealt_seats_3_to_6`, `street_root_reconstruction`
(100/500 root, 150/300/400 progression, `NoDecision`, third all-in stays multiway), `multiway_root_projection`
(dead 0 / dead 50 with min re-raise 350 / step 1 rejection), `card_parser_roundtrip`, `range_roundtrip_pio_strings`,
`class_expansion_multiplicity`, `public_blocking_board_only` (1,176), `hero_conditioned_copy`,
`range_hash_scale_invariant`, `iso_class_count_1755`, `iso_orbit_sizes` (52 / 3,744 / 18,304 / 22,100 — reproduced
independently during this review), `iso_stabilizer_tiebreak`, `eval_vs_phevaluator_full_5card`,
`eval_vs_phevaluator_random_7card` (+ `exhaustive`), `terminal_payoff_equity_times_pot` (equity x 100, x 95 at cap 5,000
mchips), `equity_budget_respected`, `equity_mc_within_standard_error`, `equity_joint_disjoint_sampling`. Fixtures
`fixtures/hands` and `fixtures/eval` are produced (§13.0). Chart/preflop/worker fixtures correctly belong to plans 2-3.

**2. Spec deviations — 5 found, 4 justified.** `StreetRootSnapshot.bb_chips` (M1, justified, needs a cross-plan edit);
`RootError` extra variants and payload (M3, `Preflop`/`Inconsistent` justified and spec-consistent with §10.2's
"a mismatch on a genuine HU root is `EngineError`", the `Multiway` payload is not and should go); `pokers` replaced by an
in-house implementation (m4, justified by R3, §3.5 interface unchanged); `EquityMode::MonteCarlo{max_samples}` (m3,
justified); repo-wide MSVC pin against §3.6 (M7, must be qualified). The plan's own "Gaps and deliberate deviations"
list names 8 of these accurately — its honesty is the reason they are cheap to fix.

**3. Placeholders — PASS.** No `TBD` / `TODO` / "similar to Task N" / "add validation" / code-less code steps. The only
transient artifact is the Task 20 `mc.rs` dispatcher (m5), which compiles and is replaced in Task 21.

**4. Correctness — 3 blockers plus M6.** B1 is a compile error, B2 and B3 are assertions that are arithmetically false.
Everything else that was hand-simulated is right: `Round::apply` reproduces every asserted legal set and
`last_full_raise` value in Tasks 10-12; `refund_uncalled` + `layer_pots` reproduce every asserted pot and refund in
Task 11 (including the 202/[0,5] merge and the `max_by_key` tie that correctly yields no refund); `street_root` +
`replay_root` reproduce all three §10.2 worked cases including `ProjectionNotReproducing{step: 1}`;
`resolve_chip_path` returns the right value on all six wire cases; the `hash_scaled` power-of-two invariance is exact in
IEEE `f32`; `class_of` / `combo_index` / `combo_cards` are bijective with no `u8`/`u16` overflow.

**5. Task ordering — PASS.** `cargo test --workspace` stays green after every task: `proto` is built bottom-up (cards ->
config -> range -> recommendation -> tree -> worker -> validators), fixtures are generated (Tasks 13, 18) before the Rust
tests that read them (Tasks 14, 19), `core-ranges` precedes `core-iso` and `core-eval`, and the Task 20 `mc` stub keeps
Task 20 green before Task 21 replaces it. No task depends on a later one.

**6. Interfaces — FAIL (M1-M5).** The `Produces` blocks and the "Interfaces handed to plans 2-5" table are complete and
match spec §3.5 names for `core-model`, `core-ranges`, `core-iso` and `core-eval`. They do **not** match what plans 2-5
already consume in five places (`StreetRootSnapshot` literal shape, `BeginHand` field names and `hand_id`, `RootError`
variant shape, `proto::worker::{Ready, LockRequest, PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}`, `Card::parse`).
These must be reconciled in one direction before either plan is executed. Note also that plan 2 re-implements
`resolve_chip_path` in `engine::tree::resolve` although plan 1 already exports `proto::resolve_chip_path` — one of the two
should be deleted (plan 2's issue, recorded here for the reconciliation pass).

**7. YAGNI — PASS.** Nothing beyond the plan's scope or phase 1. `ExploitAdvice`/`SeatTag`/`QuickFact` (§11) are pulled in
only because `Recommendation.exploit` and `SeatConfig` in §4.4/§4.2 require them to compile. `sample_joint_holes` and
`Xoshiro256` are exported for tests and are the smallest surface that makes §13.1's disjointness assertion possible.
The `exhaustive` feature is a spec requirement. No preflop, cache, tree-materialization or bench code leaks in from
plans 2-4.
