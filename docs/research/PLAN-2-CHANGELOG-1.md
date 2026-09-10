# Plan 2 revision 1 — changelog (2026-09-10)

Plan: `docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md`.
Inputs: `docs/research/REVIEW-of-plan-2.md` (3 BLOCKER, 9 MAJOR, 15 MINOR), `docs/research/REVIEW-cross-plan.md` sections 1-5, spec revision 6, orchestrator decisions 1-6.

Size before: 5,678 lines, 22 tasks, 90 steps. Size after: **7,099 lines, 30 tasks, 128 steps**.

---

## 1. `REVIEW-of-plan-2.md` — BLOCKER

| ID | Disposition | Where / what changed |
|---|---|---|
| B1 — loop variable shadows `fn attempt` | **APPLIED** | Task 23 step 4: the loop variable is `attempt_no` and the function is `run_attempt`. The naming rule is stated in the task's "Three corrections applied here" block so it cannot be reintroduced. |
| B2 — workspace members added before the crates exist | **APPLIED** | Task 1 step 3 now adds **only** `exclude = ["third_party/postflop-solver"]`. `crates/engine` (Task 2) and `crates/bench` (Task 5) are covered by plan 1's `members = ["crates/*"]` glob; Task 7 adds `"solver-worker"` when it creates that crate. Recorded as a Global Constraint. |
| B3 — six `proto` items missing or mis-shaped | **APPLIED** | The header table "Interfaces consumed from plan 1" was replaced with the **resolved** signatures of cross-plan §1: `Ready` (not `ReadyInfo`), `EngineMessage::Lock { id, spot, locks }` struct variant with `LockRequest` deleted, `proto::worker::{PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}`, `MenuSize::{Pot, AllIn}` / `SideMenu` (which *can* express `river_std_v1`'s `0.33, 0.75 + a`), `core_model::BeginHand { hand_id, …, stacks_start, … }` plus the new `proto::BeginHand` admission DTO, `Card::parse -> Result<Card, CardParseError>` with `ProtoError` deleted, `StreetRootSnapshot.bb_chips`, five-variant `RootError`, and plan 1's `EquityRequest`/`EquityResult`. Every consuming site in the plan was updated. |

## 2. `REVIEW-of-plan-2.md` — MAJOR

| ID | Disposition | Where / what changed |
|---|---|---|
| M1 — busy branch precedes duplicate | **APPLIED** | Task 13 (state machine): the `Solve` arm tests `stopping` -> `duplicate` -> `busy` -> `precheck`, with the reason stated in the task header and in a code comment. Task 14 repeats the ordering in the final arm. `protocol_rejections` also now re-sends a *finished* id and asserts `duplicate`. |
| M2 — `lenient_id` cannot recover an id from invalid JSON | **APPLIED** | Task 13 step 3: `lenient_id` falls back to `scan_id`, a byte scan for `"id"` followed by the next quoted string. The `"pot":NaN` case keeps id `"4"`. |
| M3 — retry resets the reported stage | **APPLIED** | `EngineCore::set_stage` now only ever **advances** (rank fast < building < solving < extracting), with `reset_stage` used once at admission. Task 23 documents it; `final_delivery_independent_of_worker` case (c) reports `extracting`, case (b) `building`. |
| M4 — scripted `InvalidateIdentity` never yields | **APPLIED** | Two changes: `FakeReply::InvalidateIdentity`'s doc comment states it is consumed inside the same `recv` call and that a script must follow it with `Delay { ms: 1 }`; `identity_race_golden` (Task 28) now scripts `InvalidateIdentity, Delay { ms: 1 }, Result…`; and `run_attempt` re-checks the identity **immediately after** a terminal arrives, discarding a result that races a mutation. |
| M5 — `street_violation` computed from the attempt end time | **APPLIED** | Task 23: `first_attempt_terminal` is set only when attempt 0 returns an `AttemptEnd::Result`; `street_violation = !first_attempt_terminal && now >= street_deadline_ms`. The `heartbeat_failure_restarts_and_retries_min` script was rewritten (`Delay 1200` before the progress) so the heartbeat fires at t = 6 200 ms, past the 6 s turn budget, and the assertions are now exact (`clock == 6_200`, `deadline_ms == 15_000 - 6_200 - 150`). |
| M6 — percentile assertion contradicts the implementation | **APPLIED** | Task 30: the nearest-rank definition is documented on `percentile`, the assertion is `2`, and three more rows were added (`p95`, the `p50 30 ms` row, the empty slice). |
| M7 — `wager_cap_remove_lines` asserts at the wrong node | **APPLIED** | Task 16: the loop pops the last action first, applies the shorter prefix, then asserts the popped action is still offered. |
| M8 — wrong range combo counts | **APPLIED** | Task 6: `634` / `720`, plus new assertions pinning `CO_CALL_3BET` 194 and `BTN_3BET` 138 (which do match A.1, proving the parser). `CO_CALL_3BET` and `BTN_3BET` were added to the Python module. A comment in `crates/bench/src/gen_spots.rs` records that A.1's published 646 / 804 are not reproducible from its own strings and that the strings are the definition. Also in the self-review deviations list. |
| M9 — workspace not green after tasks 11 and 14 | **APPLIED** | (a) `gen_basic_fixture` moved into Task 15 as its step 1, so `ev_convention_non_root_payoffs` has its fixture when it lands and no `--skip` is needed. (b) `worker_link.rs` discovers the binary via `POKERAI_WORKER` or the target directory and returns early with a printed reason when it is absent, so the workspace command is green in a clean tree. (c) A Global Constraint states the per-task green command once: **`cargo test --workspace --release`**, "with no `--skip` and no known-failing test left in the tree", and every task's run step now ends with it. |

## 3. `REVIEW-of-plan-2.md` — MINOR

| ID | Disposition | Where |
|---|---|---|
| m1 — root-street `donk: None` reading undocumented | **APPLIED** | Comment in `templates.rs::spec`, a two-point block in Task 8, and self-review deviation 5. |
| m2 — `ack{rejected}` vs `tree_mismatch` for a `None` donk | **APPLIED** | Task 8's block cites both §13.2 rows and states the choice; self-review deviation 6. |
| m3 — `bench` enables `engine/testing` | **APPLIED** | Task 5's `crates/bench/Cargo.toml` drops `features = ["testing"]`, with the feature-unification reason in the surrounding text. |
| m4 — `cancel_latency` measures 0 on a fast river spot | **APPLIED** | Task 30: `cancel_latency` waits for the first `progress{stage:"solving", iterations>=1}` instead of sleeping 50 ms, returns `Option<(u64,u64)>`, and the report prints `n/a` when the solve finished first (with a unit test for that row). |
| m5 — `tree_builder_golden` omits "duplicates merged" | **APPLIED** | Task 4: two new cases — an observed size landing exactly on a menu size (nothing inserted, one entry) and the force-collapse of `raise 250` onto `AllIn(340)` at eff 340 (exactly one all-in, no raise). |
| m6 — five `flop_full_v1` fixture cases for a phase-2 template | **APPLIED** | Task 6: `flop_full_v1` gets one case at `(100, 100)`; 43 cases total. Every "47" in the plan became 43. |
| m7 — two produced-but-unused items | **APPLIED** | `win::peak_working_set_bytes` deleted from `solver-worker` (the engine measures the child's peak from the parent); `EngineCore.bench_p95_ms` deleted and the retry admission calls `street_budget_ms` directly, documented as the p95 proxy until plan 4's measured matrix exists. |
| m8 — out-of-interval tree action labelled `NotInMenu` | **APPLIED** | Task 26: such an action keeps its frequency, gets no EV, is marked `Unavailable::NotEvaluated`, and adds an explicit `assumptions.notes` entry naming §8.4's `MovedProbability` mapping as plan 4's. Self-review deviation 8. |
| m9 — §4.5 limits redefined instead of imported | **APPLIED** | `extract.rs` re-exports `proto::worker::{MAX_EXPORTED_NODES, RESULT_LINE_MAX}`; `protocol.rs` re-exports `REQUEST_LINE_MAX` as `MAX_REQUEST_LINE`; `solver_worker::SOLVER_COMMIT` and `engine::tree::RULES_VERSION` are re-exports too. All call sites updated. |
| m10 — `identity_race_golden` cannot show "the same displayed revision" | **APPLIED** | Task 28 asserts `(id_b1.hand_revision, id_b2.hand_revision) == (9, 9)`, that A's and B1's identities are inactive, and records the spec clause as vacuously satisfied. Self-review deviation 7. |
| m11 — R8 uniform ranges are not §13.5's baseline | **APPLIED** (superset) | See cross-plan Or8 below: the interim status is now in the Task 5 header, in the `Spot.range_source` value, in a `generate("…","chart")` error assertion, and in a mandatory banner at the top of the generated `docs/bench/*.md`. |
| m12 — `flop_best_so_far.jsonl` has no reader | **APPLIED** | Task 17: `deadline_best_so_far_bounded` gained a second case that sends the fixture at target 1 bp with a 2 s deadline and validates the `best_so_far` export. |
| m13 — propagate the `AA` -> `QQ,66` correction to the spec | **APPLIED** (verified) | The spec is already at revision 6 with S7 applied; Task 15 and the self-review cite S7 rather than asking for a change. |
| m14 — `source_accuracy` in chips | **APPLIED** | Task 28: `assumptions.source_accuracy = format!("exploitability <= {bp} bp")`, asserted in `identity_race_golden` (`"exploitability <= 31 bp"`). Raw chips remain in the snapshot and the decision log. |
| m15 — `facing_allin_golden` does not exercise the hero-range clause | **APPLIED** | `AllInInput` gained `hero_public: Option<Range1326>` (accepted, deliberately unused) and the golden asserts that passing a four-hand hero range changes neither equity, `W`, `EV(call)` nor the headline. |

## 4. `REVIEW-cross-plan.md` items naming plan 2

| Item | Disposition | Where |
|---|---|---|
| M1 menu types (`SideMenu`/`MenuSize`) | **APPLIED** (no plan-2 change needed; header now states it is resolved, not assumed) | Header table. |
| M2 `Card::parse` / no `ProtoError` | **APPLIED** | Header table; `ProtoError` removed from the plan. |
| M3 `EngineMessage::Lock` struct variant | **APPLIED** | Header table; `handle_message` matches `EngineMessage::Lock { id, spot, locks }`; `LockRequest` deleted. |
| M4 `Ready` (not `ReadyInfo`) | **APPLIED** | Header table; unchanged elsewhere (the plan already used `Ready`). |
| M5 `proto::worker::{PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}` | **APPLIED** | Header table; Task 7 re-exports `SOLVER_COMMIT` and adds a `const` assertion that the vendored `PINNED_COMMIT` matches it. |
| M6 `BeginHand` field set | **APPLIED** | `Engine::begin_hand(proto::BeginHand)` converts to `core_model::BeginHand` with the engine-assigned `hand_id`; `testing::hand` and the coverage test use `hand_id`/`stacks_start`. |
| M7 `RootError` variants and width | **APPLIED** | Task 24: `Multiway{pot_eligible}`, `ProjectionNotReproducing{step: u32}`, `NoDecision`, `Preflop -> Classification::Preflop`, `Inconsistent{step} -> Unsupported{EngineError{retryable:false}}`. |
| M8 `core-eval` shape | **APPLIED** | Task 25's `equity.rs` rewritten against `EquityRequest::single_pot` / `PlayerRange` / `shares` / `status`, using `exact_cost(&req) <= 20_000_000` instead of a local formula, with `MonteCarlo{seed, max_samples}`. Task 18's oracle rewritten the same way. |
| M9 `StreetRootSnapshot.bb_chips` | **APPLIED** | Added to every literal (Tasks 4, 22, 27). |
| M10 `set_config -> Result<u32, EngineError>` | **APPLIED** | Task 29, with `flop_budget_s` range validation and a test. |
| M11 `shutdown(&mut self)` | **APPLIED** | Task 29, idempotent via a `stopped` flag, plus `Drop`. |
| M12 `recommend` sink type | **APPLIED** (plan-5 change; recorded) | Task 29 states `Box<dyn EventSink>` is the type plan 5 must write. |
| M13 / Or5 `Paths` layout | **APPLIED** | `Paths { log_dir, worker_exe, preflop, cache }` — the orchestrator's field names, which differ from the cross-plan note's `worker`/`log`; plan 5 must use `log_dir`/`worker_exe`. Flagged here as the one naming divergence from cross-plan §1. |
| M14 / Or3 `set_hero_cards` | **APPLIED** | Task 29. |
| M15 / D1 snapshot store | **APPLIED** (per orchestrator g) | Kept as `engine::snapshots::{SnapshotStore, SolvedStreet}` rather than renaming to `SolvedStreetStore`; Task 27 documents the wrap seam and the single-commit replacement rule. |
| M16 / D6 `RangeSource` vs `prepare_root` | **APPLIED** | Task 27 documents `RangeSource::ranges_at_root` as the only provider and `serve_request`'s `Classification::Preflop` arm as the other plan-3 hook; both hooks are marked in the code with `PLAN 3 HOOK` / `PLAN 4 HOOK` comments. |
| M17 / R5 `bench` importing `core-ranges` | **APPLIED** | New `engine::bench_support::{prepared_range, range_mass}` (Task 5) is the only path; Task 30's runner uses it; `bench`'s manifest depends on `proto` and `engine` only. |
| M18 / R6 dependency versions | **APPLIED** | Every new manifest uses `serde.workspace`, `serde_json.workspace`, `sha2.workspace`, `hex.workspace`, `thiserror.workspace`, `version/edition/license.workspace` (except `solver-worker`'s explicit AGPL license). Stated once as a Global Constraint. |
| M19 / R1 toolchain | **APPLIED** | The GNU statement is gone. Tech Stack names MSVC via plan 1's `rust-toolchain.toml`; a Global Constraint references plan 1's V1 MSVC/GNU-fallback check (build the solver on MSVC, fall back to GNU for `solver-worker` only if it fails or is >25% slower) and Task 1 step 5 records which branch was taken in `PATCHES.md`. Nothing is duplicated. |
| M20 / D10 `Templates::ids().len() == 9` | **APPLIED** | `Templates::base_ids()` (always the 9 compiled templates) carries the assertion, so it cannot race; `Templates::with_extra(&[TemplateSpec])` behind `cfg(any(test, feature = "test-templates"))` is the seam plan 4 uses for `check_jam_test_v1`, `menu_round_test_v1`, `check_only_test_v1`. |
| M21 / D2 `resolve_chip_path` | **APPLIED** | `engine::tree::resolve.rs` is now `pub use proto::resolve_chip_path;` plus `node_at`; `signature.rs` imports from `proto`. |
| D3 `node_at` | **APPLIED** (no change) | Plan 2 keeps `node_at`; the rename is plan 3's. |
| D4 `bench` files | **APPLIED** | `crates/bench/src/lib.rs` added in Task 5 and extended in Task 30 so plan 4's integration tests can link; noted in the self-review's cross-plan surface. |
| D5 flop deadline arithmetic | **APPLIED** (recorded) | Self-review §4 states plan 4 extends `Deadlines` rather than adding `flop_deadlines`. |
| D7 two-combo river ranges | **APPLIED** | Task 6's Produces block states the generator is the single definition and that plan 1's `wire_examples.rs` loads the fixture. |
| D8 `assemble::headline` | **APPLIED** (recorded) | Self-review §4 states plan 3 extends `headline`, not a second entry point. |
| D9 root `Cargo.toml` members | **APPLIED** | See B2. |
| Or1 §6 experimental surrogate | **APPLIED** (recorded, reassigned) | Self-review deviation 10 now says plan 4 Task 10 **creates** it (there is nothing to extend), per R4. |
| Or2 `bench oracle` | **APPLIED** (recorded) | Self-review deviation 13 assigns it to plan 4 Task 20. |
| Or4 `proto::BeginHand` | **APPLIED** | Consumed from plan 1 Task 3; header table row added. |
| Or6 worker constants | **APPLIED** | See M5. |
| Or8 / R3 chart-replay baseline | **APPLIED** (per orchestrator decision 4) | `--source r8` is a labelled interim: `range_source: "r8_uniform"` on every spot, a test asserting `generate(_, "chart")` errors with a message naming chart replay, and a mandatory "pre-baseline reference run" banner in the generated bench report. The plan says the chart integration arrives with plan 3's bundles and that **plan 4 Task 17 regenerates all six suites** before the V2/V22 gate. |
| R2 "engine always sends `background: false`" | **APPLIED** | Removed as an invariant. A Global Constraint states `background` is a request parameter; `deadline_arithmetic_and_request_fields` asserts a `SolvePlan.background = true` reaches the wire; `serve.rs` comments that plan 4's pre-solver builds the same plan with `true`. |
| §4 execution-order risks | **APPLIED** | Risk 1 (`EngineCore::new` arity) is eliminated: the decision log is now **Task 21**, before `EngineCore` (Task 22), so the constructor is four-argument from its first line and no later task rewrites the rig. Risk 2 (snapshot deletion) is documented in Task 27. Risk 3 (`ts-rs`) is plan 5's. |
| §5 oversized tasks (T21, T16, T10, T9) | **APPLIED** | T9 -> Tasks 9 (memory + stop rule), 10 (extract + locks), 11 (job runner). T10 -> Tasks 12 (writer + bounded line reading + thread wiring), 13 (state machine + ack rules), 14 (lock staging + cancel lifecycle). T16 -> Tasks 20 (deadlines + watchdog), 22 (`EngineCore` + `run_solve` happy path), 23 (heartbeat, cancel-kill, retry). T21 -> Tasks 27 (snapshots + ranges), 28 (`serve_request` + `identity_race_golden`), 29 (`Engine` API + startup report + `final_delivery_independent_of_worker`). Every task keeps its own failing test, implementation and commit; all cross-references were renumbered. |

## 5. Orchestrator interface requests (a)-(h)

| Request | Disposition | Where |
|---|---|---|
| (a) `Engine::set_hero_cards(&mut self, [Card; 2]) -> Result<HandState, EngineError>` | **APPLIED** | Task 29; a mutation (fresh revision, in-flight work invalidated, snapshots dropped), tested in `engine_api.rs`. |
| (b) `Engine::set_config(&mut self, GameConfig) -> Result<u32, EngineError>` rejecting `flop_budget_s` outside `1..=30` and queueing during a hand | **APPLIED** | Task 29; `FLOP_BUDGET_RANGE`, blind and thread validation, `queued_config` applied at the next `begin_hand` (or `end_hand`), with `set_config_validates_and_queues` covering 31, 0, 30 and the mid-hand queue. |
| (c) `Paths { log_dir, worker_exe, preflop, cache }` | **APPLIED** | Task 29, with the note that plan 3 populates `preflop` and plan 4 reads `cache`. Field names follow the orchestrator, not cross-plan M13's `worker`/`log`. |
| (d) test-only `Templates` extension point | **APPLIED** | `Templates::with_extra(&[TemplateSpec])` behind `cfg(any(test, feature = "test-templates"))`, with the `test-templates` feature declared in `crates/engine/Cargo.toml` and a test proving extras shadow without enlarging `base_ids()`. Signature uses `TemplateSpec` (this plan's type), not `TreeTemplate`. |
| (e) `Engine::shutdown(&mut self)` | **APPLIED** | Task 29, idempotent, plus `Drop`. |
| (f) `Engine::startup_report() -> StartupReport` | **APPLIED** | New `crates/engine/src/startup.rs`: worker readiness, threads, build/CPU features, capabilities, the CPU-lacks-AVX2 banner, `quarantined_bundles` (plan 3) and `cache_state` (plan 4). Captured at construction so the call never blocks behind a running solve. |
| (g) `engine::snapshots::{SnapshotStore, SolvedStreet}` kept, wrap seams documented | **APPLIED** | Task 27's two-point "seams plans 3 and 4 attach to" block, plus `PLAN 3 HOOK` / `PLAN 4 HOOK` comments in `serve.rs`. |
| (h) `background` is a solve-path parameter | **APPLIED** | See R2 above. |

## 6. Additional corrections made while applying the above

These were not in either review but were required for the applied changes to be consistent:

1. **`EngineCore.config` and `.range_source` are `Arc<Mutex<_>>`.** With `Engine` holding the `EngineCore` behind a mutex, `set_config` / `set_explicit_ranges` / `startup_report` would have blocked behind a 15-second solve, contradicting §3.4 ("commands never block"). `serve_request` now takes one config snapshot per request and locks the range source only to call `ranges_at_root`.
2. **`solve_loop`'s stop rule extracted as `should_stop` / `expl_due`**, so Task 9 can unit-test §7 arithmetic without a solve.
3. **`Templates::base_ids()`** added so the "9 templates" assertion cannot race the `with_extra` test.
4. **`Engine::begin_hand` invalidates the identity when `core_model::begin_hand` rejects the admission**, so `recommend` can never hand out an identity for a hand that does not exist.
5. **`assumptions_stub()`** added in Task 20 (the zero `Assumptions` of §4.4) so the watchdog test can build a `Recommendation` before `assemble` exists; `assemble::empty_assumptions` is now defined in terms of it.
6. **`solver-worker` asserts at compile time** that `third_party/postflop-solver/PINNED_COMMIT` matches `proto::worker::SOLVER_COMMIT`, so `ready` can never claim a commit the binary was not built from.
7. **`Task 12`'s placeholder `handle_line`** is written out (it answers `ack{rejected, "not implemented yet"}`) so that task's tests fail for the right reason and the crate compiles at every task boundary.

## 7. NOT APPLIED

Nothing from `REVIEW-of-plan-2.md` or from the cross-plan items naming plan 2 was left unapplied. Two items are recorded rather than implemented, because they belong to other plans:

- **Or1 (§6 experimental surrogate / `experimental_surrogate_golden`)** and **Or2 (`bench oracle`)** are plan 4's. Plan 2 records the assignment in its self-review; it creates nothing.
- **M12 (`recommend` sink type)** is a plan-5 edit. Plan 2 states the type it exposes so plan 5 can follow it.

One deliberate divergence from a cross-plan note, flagged for the orchestrator:

- **`Paths` field names.** Cross-plan M13 suggests `Paths { worker, preflop, cache, log }`; orchestrator decision 3(c) specifies `Paths { log_dir, worker_exe, preflop, cache }`. The orchestrator's names are implemented. Plan 5 Task 3 must construct `log_dir` and `worker_exe`, not `log` and `worker`.
