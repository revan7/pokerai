# Plan 2: Solver worker and engine river/turn path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Vendor the pinned postflop-solver, build the `solver-worker` binary (spec §4.5 protocol, §10.3 adapter, §4.6 cross-check and wager cap), and build the `engine` crate's river/turn path (identity, admission, worker client with absolute deadlines and watchdog, tree templates and materializer, coverage classifier, facing-all-in fallback, result assembly, decision log) plus the `bench` river/turn suites and the worker wire fixtures.

**Architecture:** `solver-worker` is an AGPL process with three threads (`control` reads stdin in every state, `writer` serializes stdout, `executor` owns the `PostFlopGame` and the rayon pool); it only depends on `proto` and the vendored library, and every request runs Building -> Solving -> Extracting with cancel checkpoints between steps. `engine` owns the worker process behind the `WorkerLink` trait (a `ProcessWorker` in production, a scripted `FakeWorker` with a `FakeClock` in tests), materializes every tree itself with the pinned §4.6 rules (the worker only cross-checks), and assembles `Recommendation`s under a `DecisionIdentity` that every event carries. `bench` drives the worker through the engine's link on fixed spot files and writes the §13.5 report.

**Tech Stack:** Rust 2021 on `stable-x86_64-pc-windows-msvc` (1.95), pinned repo-wide by plan 1 Task 1's `rust-toolchain.toml`; ordinary `cargo` commands, no per-plan toolchain override. Vendored `postflop-solver` at `9d1509fe` (features `bincode`, `rayon`, `zstd`), `serde`/`serde_json`, `sha2`, `hex`, `thiserror`, `rayon 1` (worker only) — all from `[workspace.dependencies]`; `libc`-free Win32 FFI via `extern "system"` declarations; Python 3.12 + `pytest` for `tools/gen_worker_fixtures.py`.

**Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (revision 6), sections 2, 3.1-3.7, 4.4-4.6, 5, 6, 7, 10.1-10.3, 10.6, 12, 13.0, 13.2, 13.3, 13.5. Decisions: `docs/design/2026-09-10-design-outline.md` §0b. Measured facts: `docs/research/R8-solver-bench.md` §2-§4 and addendum A.5. Cross-plan resolutions: `docs/research/REVIEW-cross-plan.md` sections 1-5.

## Global Constraints

- Pinned upstream commit `9d1509fe5077d019825f833eed04b16d342dfda1`; features default (`bincode` + `rayon`) plus `zstd`; `custom-alloc` never used. Patches: (1) `Cargo.toml` pins `bincode = "=2.0.0-rc.3"`, `bincode_derive = "=2.0.0-rc.3"`; (2) `src/action_tree.rs` lines 393/396/408 `&*(*node).children[i].lock()` -> `&*(&(*node).children)[i].lock()`.
- AVX2 is a build requirement: `.cargo/config.toml` (plan 1) sets `-C target-feature=+avx2` for both Windows targets; `solver-worker/build.rs` fails when `CARGO_CFG_TARGET_FEATURE` lacks `avx2`; the engine refuses a worker whose `ready.build_features` lacks `avx2` (`EngineError("worker built without AVX2")`).
- Toolchain: the workspace is pinned to `stable-x86_64-pc-windows-msvc` by plan 1 Task 1's `rust-toolchain.toml`. This plan uses ordinary `cargo` commands and never sets a per-plan toolchain override. Plan 1's V1 check builds the vendored solver on MSVC and, if that fails or is more than 25% slower than the GNU build, falls back to building **only** `solver-worker` with `+stable-x86_64-pc-windows-gnu` (spec §3.6); that check is referenced, never duplicated here. If it selects the GNU fallback, the only change to this plan is the `cargo build`/`cargo test` invocations for `-p solver-worker`, which then carry `+stable-x86_64-pc-windows-gnu`; every engine, bench and workspace command stays MSVC.
- Dependency versions come from plan 1's `[workspace.dependencies]`: every crate this plan creates writes `serde.workspace = true`, `serde_json.workspace = true`, `sha2.workspace = true`, `hex.workspace = true`, `thiserror.workspace = true`, plus `version.workspace = true`, `edition.workspace = true`, `license.workspace = true` (except `solver-worker`, which declares `license = "AGPL-3.0"` explicitly). Never a second `sha2` or `thiserror` major (cross-plan M18/R6).
- Workspace membership: plan 1's root manifest uses `members = ["crates/*"]`, so `crates/engine` (Task 2) and `crates/bench` (Task 5) become members the moment their manifests exist. `solver-worker` is outside `crates/`, so Task 7 adds `"solver-worker"` to `members`. Task 1 adds only `exclude = ["third_party/postflop-solver"]` (cross-plan D9). No task ever lists a member whose manifest does not yet exist.
- `background` is a per-request parameter of the solve path (`SolvePlan.background` -> `SolveRequest.background`), never an engine invariant. This plan exercises only `false`; plan 4's pre-solver sends `true` with `deadline_ms: 600000` (spec §10.5, cross-plan R2).
- `solver-worker` is `license = "AGPL-3.0"`; every other crate is `MIT OR Apache-2.0`. `solver-worker` depends only on `proto` plus the vendored solver; no crate depends on `solver-worker`. Dependency direction `proto` <- `core-*` <- `engine` <- `bench`.
- Money: integer chips (`u32`), rake cap `cap_mchips` (thousandths of a chip, `f64` chips only at the library boundary), EV `f32` chips, `ev_bb = ev_chips / bb_chips` at render time only; `pot + stacks < 2^31` else `EngineError`.
- Protocol (§4.5): UTF-8 JSON Lines, `#[serde(tag = "type")]`, lowercase tags, unknown fields rejected, ids decimal strings; request line <= 1 MiB, result line <= 16 MiB, <= 100,000 exported nodes; `proto_version` 3, `adapter_version` 1; failure codes exactly `invalid_request`, `tree_mismatch`, `tree_too_large`, `out_of_memory`, `lock_mismatch`, `no_iteration`, `internal`.
- Library lifecycle (§3.7): locks after `allocate_memory` and before `finalize()`; nothing solves after `finalize()`; `solve_step` granularity one iteration; `cache_normalized_weights()` after every navigation before reading EVs.
- Memory admission (§10.3): f32 when the f32 estimate `<= 2 GiB`, else i16 when the i16 estimate `<= 8 GiB`, else `tree_too_large`; refuse when `estimate * 1.25 > memory_limit_bytes`; engine default `memory_limit_bytes = 10 GiB`; job object `JOB_OBJECT_LIMIT_PROCESS_MEMORY = 16 GiB`.
- Stop rule (§7): between iterations stop when `elapsed + 1.5 * max_iteration_so_far + (exploitability pass if due) + extraction_margin_ms > deadline_ms`; exploitability every 10 iterations and additionally whenever fewer than 10 iterations fit before the stop point; `best_so_far` only with at least one measurement, else `no_iteration`.
- Deadlines (§7): from monotonic `t0`: first attempt `t0 + 2 s` river, `t0 + 6 s` turn; final delivery `t0 + 15 s` (river, turn); worker `deadline_ms = remaining - 100 ms - 50 ms` at send time; extraction margin river/turn 200 ms (flop 600 ms); heartbeat: no `progress` for 5 s in `Solving` fails the worker; cancel `ack` <= 50 ms, kill after 1.5 s without `result{cancelled}`; watchdog emits at `final delivery - 100 ms`; startup timeout 5 s, one retry, then `EngineError`.
- Tree rules (§4.6): action order Fold, Check, Call, bets/raises ascending by `to`, AllIn; `rules_version` 3; `merging_threshold` 0.0; donk menus on turn and river are the explicit empty list; `matched` and `pot` accumulate from the tree root; wager cap on non-all-in wagers per street with the observed prefix never removed.
- Headline (§4.4): highest EV only when every action has `ev_bb` (ties by higher frequency, then menu order), else highest-frequency headline only when every action has `frequency` and `unresolved_mass == 0`, else none. Reasons accumulate and are never removed. Exploitability compared with the target in raw chips.
- Commits: one per task, `feat(<crate>): ...` / `test(<crate>): ...` / `chore: ...`, trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Rust edition 2021; `thiserror` for error enums; `rayon` only in the worker; engine threads are std threads.
- **Per-task green command: `cargo test --workspace --release`.** It must pass at the end of every task, with no `--skip` and no known-failing test left in the tree. Release is mandatory because the vendored library is about 30x slower in debug and §13.2's timing assertions assume optimized code; the root manifest additionally carries `[profile.dev.package.postflop-solver] opt-level = 3` (Task 7) so an ad-hoc debug run is not pathological. Any test that needs an artifact built by a `cargo build` step (the `solver-worker.exe` used by `crates/engine/tests/worker_link.rs`) discovers it through `POKERAI_WORKER` or the standard target directory and `#[ignore]`s itself when the binary is absent, so the workspace command never fails on a missing artifact.

---

## Interfaces consumed from plan 1 (resolved signatures)

These are the **resolved** names of `docs/research/REVIEW-cross-plan.md` section 1 (items M1-M9, M18, M21, Or4, Or6), which plan 1 is being revised to produce. They are not assumptions: import them exactly as written. Every difference from the previous draft of this plan is listed in `docs/research/PLAN-2-CHANGELOG-1.md`.

| Crate | Item | Resolved signature |
|---|---|---|
| `proto` | cards | `Card(pub u8)` with `Card::rank(self) -> u8`, `Card::suit(self) -> u8`, `Card::new(rank, suit)`, `impl FromStr for Card { type Err = CardParseError; }` and the convenience `Card::parse(s: &str) -> Result<Card, CardParseError>` (plan 1 Task 2, cross-plan M2); `Display` as `"As"`; serde as the two-character string. There is no `ProtoError` — error text goes through `CardParseError` |
| `proto` | combos | `ComboIndex = u16`; `combo_index(a: Card, b: Card) -> ComboIndex` (`hi*(hi-1)/2 + lo`); `combo_cards(i: ComboIndex) -> [Card; 2]` (`[lo, hi]`); `Range1326(pub [f32; 1326])` with `Clone`, `PartialEq`, serde as a JSON array of exactly 1326 finite numbers |
| `proto` | enums | `Street::{Preflop, Flop, Turn, River}` (serde lowercase, `Ord`), `Seat(pub u8)`, `Action::{Fold, Check, Call, Bet{to: u32}, Raise{to: u32}, AllIn{to: u32}}` (serde `tag = "kind"`, lowercase: `{"kind":"allin","to":100}`), `LegalAction::{Fold, Check, Call{cost}, Bet{min_to, max_to}, Raise{min_to, max_to}, AllIn{to}}` |
| `proto` | state | `GameConfig`, `SolverPrefs { threads: u8, target_bp: u16, flop_budget_s: u8 }` (`Default` = 16/50/10), `Rake::{PotRake{rate: f32, cap_mchips: u32, no_flop_no_drop: bool}, TimeCharge}`, `HandConfig` (+ `HandConfig::from_game(&GameConfig)`), `HandState`, `Derived`, `SolveInput { root, ranges: [Range1326; 2], tree: EffectiveTree, target_bp: u16 }`; `StreetRootSnapshot { street, board, oop, ip, pot_root, stack_oop_root, stack_ip_root, dead_this_street, projected_from, history, bb_chips: u32 }` — **`bb_chips` is a required field** (plan 1 dev 1 / spec S1 / cross-plan M9); the worker ignores it, `replay_root` needs it for the minimum bet |
| `proto` | admission DTO | `proto::BeginHand { button: Seat, hero: Seat, dealt: Vec<Seat>, stacks: Vec<u32>, hero_cards: Option<[Card; 2]> }` (plan 1 Task 3, cross-plan M6/Or4, spec S16): the ID-free DTO of §5 step 2, `stacks` in dealt-seat order. This is what `Engine::begin_hand` takes; it is **not** `core_model::BeginHand` |
| `proto` | tree | `EffectiveTree { rules_version: u16, template_id: String, root_street: Street, menus: BTreeMap<Street, PlayerMenus>, add_allin_threshold: f32, force_allin_threshold: f32, merging_threshold: f32, wager_cap: u8, inserted: Vec<(ChipPath, String, Action)>, materialized: Vec<MaterializedNode> }`; `PlayerMenus { oop: SideMenu, ip: SideMenu, donk: Option<Vec<MenuSize>> }` (`donk` serde default `None`); `SideMenu { bet: Vec<MenuSize>, raise: Vec<MenuSize> }`; `MenuSize::{Pot(f32), AllIn}` untagged (`0.5` / `"a"`; for `raise` the number is a multiple of the facing wager) — this is the type resolved by cross-plan M1, so `river_std_v1`'s spec §10.1 bet menu `0.33, 0.75 + a` is expressible; `MaterializedNode { path: OrdinalPath, street: Street, actor: String, actions: Vec<Action>, terminal_pots: Vec<Option<u32>> }`; `OrdinalPath = Vec<u8>`; `ChipPath = Vec<Action>`; `RULES_VERSION: u16 = 3`; `resolve_chip_path(materialized: &[MaterializedNode], path: &[Action]) -> Option<OrdinalPath>` (the single normative implementation of the §2 rule, cross-plan M21/D2 — this plan re-exports it and never reimplements it); all with `Clone`, `PartialEq`, `Debug`, serde |
| `proto` | results | `DecisionIdentity`, `Coverage`, `ApproxReason` (incl. `AsymmetricStacks { stacks_bb: Vec<f32>, prominent: bool }`, spec S11), `UnsupportedReason`, `Unavailable`, `ActionAdvice`, `Availability`, `EquityEstimate`, `EquityMethod::{Exact, MonteCarlo{samples, std_err}}`, `EquitySummary`, `PotShares`, `Assumptions`, `ExperimentalHu`, `Recommendation`, `Phase::{Fast, Provisional, Final}`, `RecommendationEvent` exactly as §4.4 |
| `proto::worker` | messages | `EngineMessage::{Solve(SolveRequest), Lock { id: String, spot: String, locks: Vec<NodeLock> }, Cancel{id, target}, Shutdown{id}}` — `Lock` is a **struct variant** (cross-plan M3); there is no `LockRequest` type. `WorkerMessage::{Ready(Ready), Ack{id, status: AckStatus, reason: Option<String>, replaced: Option<bool>}, Progress{id, stage: Stage, iterations: u32, exploitability_chips: Option<f32>, elapsed_ms: u32, memory_bytes: u64}, Result{id, status: ResultStatus, elapsed_ms: u32, solution: Option<StreetSolution>, error: Option<WorkerError>}}`, both `#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]`; `SolveRequest` with the 16 §4.5 `solve` fields; `Ready { proto_version: u16, solver_commit: String, adapter_version: u16, threads: u8, build_features: Vec<String>, cpu_features: Vec<String>, capabilities: Vec<String> }` — the struct is named **`Ready`** (cross-plan M4); `AckStatus::{Accepted, Staged, Rejected, AlreadyFinished, UnknownTarget}` and `ResultStatus::{Ok, BestSoFar, Cancelled, Error}` (`rename_all = "snake_case"`); `Stage::{Building, Solving, Extracting}`; `WorkerError { code, message, retryable, estimate_bytes: Option<u64> }`, `StreetSolution`, `NodeStrategy`, `NodeLock { path: Vec<Action>, actor: String, probs: Vec<Vec<f32>> }` as §4.5 |
| `proto::worker` | constants | `pub use crate::PROTO_VERSION;` (= 3), `pub const SOLVER_COMMIT: &str = "9d1509fe5077d019825f833eed04b16d342dfda1";`, `pub const ADAPTER_VERSION: u16 = 1;`, `REQUEST_LINE_MAX = 1 << 20`, `RESULT_LINE_MAX = 16 << 20`, `MAX_EXPORTED_NODES = 100_000`, `FAILURE_CODES` (plan 1 Task 7, cross-plan M5/Or6). All three version constants live in `proto::worker`; this plan imports them from there and **defines none of them** (cross-plan m9) |
| `proto::worker` | validation | `validate_solution(sol: &StreetSolution, materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>` (the §4.5 matrix rules; returns the resolved ordinal path of every node in `nodes` order); `validate_locks(&[NodeLock]) -> Result<(), String>` |
| `core-model` | state | `core_model::BeginHand { hand_id: u64, button: Seat, hero: Seat, dealt: Vec<Seat>, stacks_start: Vec<u32>, hero_cards: Option<[Card; 2]> }` (**engine-assigned `hand_id`, field `stacks_start`**, cross-plan M6), `begin_hand(&HandConfig, BeginHand) -> Result<HandState, RulesError>`, `apply_action(&HandState, Action) -> Result<HandState, RulesError>`, `set_board(&HandState, &[Card]) -> Result<HandState, RulesError>`, `set_hero_cards(&HandState, [Card; 2]) -> Result<HandState, RulesError>`, `derive(&HandState) -> Derived`, `is_decision_point(&HandState) -> bool`, `street_root(&HandState) -> Result<StreetRootSnapshot, RootError>` with `RootError::{Multiway{pot_eligible: u8}, ProjectionNotReproducing{step: u32}, NoDecision, Preflop, Inconsistent{step: u32}}` (**five variants, `step` is `u32`**, cross-plan M7 / spec S2), `replay_root(&StreetRootSnapshot) -> Result<Derived, RulesError>`; `RulesError::FormatUnsupported{detail}` for two dealt seats. Per-seat `Derived` vectors have length 6 and are indexed by `Seat.0`; `HandState.stacks_start` is in `dealt` order (spec S6) |
| `core-ranges` | ranges | `parse_range(&str) -> Result<Range1326, RangeError>`, `range_to_string(&Range1326) -> String`, `block_public(&mut Range1326, board: &[Card])`, `hero_conditioned(&Range1326, hero: [Card; 2]) -> Range1326`, `mass(&Range1326) -> f32`, `hash_scaled(&Range1326) -> [u8; 32]` |
| `core-iso` | suits | `canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm)`, `apply(&SuitPerm, Card) -> Card`, `apply_range(&SuitPerm, &Range1326) -> Range1326`, `inverse(&SuitPerm) -> SuitPerm` |
| `core-eval` | equity | `PlayerRange { seat: Seat, range: Range1326 }` (a fixed hero combo is a range with one supported combo); `EquityMode::{Exact, MonteCarlo { seed: u64, max_samples: u32 }}` (spec S3); `PotEligibility { pot_index: u8, eligible: Vec<Seat> }`; `EquityRequest { board: Vec<Card>, players: Vec<PlayerRange>, mode: EquityMode, pots: Vec<PotEligibility> }` with `EquityRequest::single_pot(board, players, mode)` (empty `pots` = one pot, every player eligible); `EquityStatus::{Ready, Cancelled, BudgetExceeded, InvalidRanges}`; `EquityShare { pot_index: u8, seat: Seat, value: f32, std_err: f32 }`; `EquityResult { status: EquityStatus, method: Option<EquityMethod>, shares: Vec<EquityShare>, samples: u64, elapsed: Duration }`; `equity(&EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult`; `exact_cost(&EquityRequest) -> u64` — **the engine picks `EquityMode::Exact` iff `exact_cost(&req) <= 20_000_000`, never its own cost formula** (cross-plan M8); `per_combo_equity(hero: &Range1326, villain: &Range1326, board: &[Card]) -> [f32; 1326]`; `terminal_payoff(equity: f32, pot: u32, rake: &Rake) -> f32`. Only `crates/engine/src/equity.rs` and Task 18's `river_check_only_terminal_oracle` touch this API |

---

## File structure

| Path | Responsibility |
|---|---|
| `Cargo.toml` (modify) | Task 1 adds `exclude = ["third_party/postflop-solver"]`; Task 7 adds the member `"solver-worker"` and `[profile.dev.package.postflop-solver] opt-level = 3`. `crates/engine` and `crates/bench` are covered by plan 1's `members = ["crates/*"]` glob |
| `third_party/postflop-solver/` | vendored library at the pinned commit with the two patches; `LICENSE`, `PINNED_COMMIT`, `PATCHES.md` |
| `solver-worker/Cargo.toml`, `build.rs` | AGPL crate; AVX2 build check |
| `solver-worker/src/main.rs` | argument parsing (`--threads N`), rayon pool, thread wiring, exit codes |
| `solver-worker/src/lib.rs` | module wiring, `ADAPTER_CAPABILITIES`, `ready_message(threads)` |
| `solver-worker/src/cards.rs` | proto `Card`/`Range1326`/`ComboIndex` <-> library `Card`/`Range`/`(Card, Card)` by named combos |
| `solver-worker/src/tree_build.rs` | `EffectiveTree` -> `TreeConfig` -> `ActionTree`, `add_line` insertion, `remove_line` wager cap, library-tree enumeration, cross-check |
| `solver-worker/src/history.rs` | proto `history` -> library action indices; chip path -> library line |
| `solver-worker/src/memory.rs` | f32/i16 admission and headroom rule |
| `solver-worker/src/solve_loop.rs` | §7 stop rule, exploitability cadence, cancel checkpoints, progress coalescing |
| `solver-worker/src/extract.rs` | per-node extraction to `NodeStrategy` (combo-major), export limits, `covered_paths` |
| `solver-worker/src/locks.rs` | lock matrix validation, staged-lock application before the first iteration |
| `solver-worker/src/job.rs` | one `solve` request end to end: Building -> Solving -> Extracting, `JobOutcome` |
| `solver-worker/src/protocol.rs` | worker state machine, `ack` rules, cancel/shutdown/EOF, id bookkeeping, line limits |
| `solver-worker/src/writer.rs` | single serialized stdout writer over a bounded channel |
| `solver-worker/src/win.rs` | `SetPriorityClass`, `GetProcessMemoryInfo` FFI (`#[cfg(windows)]`) |
| `solver-worker/src/testutil.rs` | fixture loading helpers shared by unit tests (`#[cfg(test)]`) |
| `solver-worker/examples/gen_basic_fixture.rs` | writes `fixtures/solver/basic_0p3.json` from the library's own `solve()` (V1) |
| `solver-worker/tests/common/mod.rs` | spawn helper `Worker::spawn(threads)`, `send`, `recv_until`, fixture readers |
| `solver-worker/tests/{startup,protocol,contract_river,contract_tree,contract_deadline}.rs` | §13.2 contract tests |
| `crates/engine/Cargo.toml` | `MIT OR Apache-2.0`; deps `proto`, `core-model`, `core-ranges`, `core-iso`, `core-eval`, `serde`, `serde_json`, `sha2`, `hex`, `thiserror` |
| `crates/engine/src/lib.rs` | module wiring, `EngineError`, re-exports |
| `crates/engine/src/clock.rs` | `Clock` trait, `SystemClock` |
| `crates/engine/src/identity.rs` | `IdentityState`: hand/revision/decision counters, active identity, invalidation |
| `crates/engine/src/tree/mod.rs`, `templates.rs`, `materialize.rs`, `effective.rs`, `signature.rs`, `resolve.rs` | §10.1 templates (plus the `cfg(any(test, feature = "test-templates"))` extension point plan 4 needs), §4.6 materializer, §10.2 exact insertion, `tree_signature`, `node_at`, and the re-export of `proto::resolve_chip_path` |
| `crates/engine/src/bench_support.rs` | the only facade `bench` may use for range parsing and blocking (spec §3.2: `bench` depends on `proto` and `engine` only) |
| `crates/engine/src/startup.rs` | `StartupReport`: worker features, bundle quarantine banners, cache state; rendered by plan 5 |
| `crates/engine/src/worker/mod.rs`, `link.rs`, `process.rs`, `job_object.rs`, `ready.rs` | `WorkerLink`, `ProcessWorker` (spawn, pipes, stderr ring, restart, kill), Win32 job object, `ready` validation |
| `crates/engine/src/testing.rs` | `FakeClock`, `FakeWorker`, `FakeReply`, `RecordingSink`, hand-state builders (`pub`, behind feature `testing` and `cfg(test)`) |
| `crates/engine/src/deadline.rs` | `StreetBudget`, absolute deadline arithmetic, retry admission |
| `crates/engine/src/watchdog.rs` | `Watchdog` thread: street violation flag, final delivery emission |
| `crates/engine/src/solve.rs` | `run_solve`: admission, send, receive loop, heartbeat, cancel/kill, `_min` retry, restart, validation, progress forwarding |
| `crates/engine/src/coverage.rs` | §6 classifier |
| `crates/engine/src/equity.rs` | equity summary and hero-combo equity through `core-eval` |
| `crates/engine/src/allin.rs` | facing-all-in analytic fallback |
| `crates/engine/src/assemble.rs` | `Recommendation` assembly, headline, reason accumulation, `Equity` merge |
| `crates/engine/src/log.rs` | `DecisionLog` JSONL with rotation |
| `crates/engine/src/snapshots.rs` | `SnapshotStore` of validated `SolvedStreet`s keyed by identity: the **single** registration path (spec §9.2) that plan 3 Task 11 wraps into `core_replay::StreetSnapshot` |
| `crates/engine/src/core.rs` | `EngineCore` (worker, clock, log, snapshots, identity, watchdog), `EventSink`, `serve_request` for river/turn |
| `crates/engine/src/engine.rs` | public `Engine` API of §3.5 (threads `engine-main`, `fast-path`), request slot of depth 1 |
| `crates/engine/tests/golden/*.json` | expected values for the §13.3 goldens |
| `crates/engine/tests/{tree_builder,coverage,facing_allin,assembly,identity_race,final_delivery,worker_link}.rs` | engine tests |
| `crates/bench/Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/suite.rs`, `src/runner.rs`, `src/report.rs`, `src/gen_spots.rs`, `src/materialize.rs` | `bench` CLI: `run`, `gen-spots`, `materialize`. `lib.rs` re-exports the modules so plan 4's integration tests can link them (cross-plan D4); deps are `proto` and `engine` only, without `engine`'s `testing` feature |
| `bench/spots/{river_std,river_min,turn_std,turn_min}.json` | generated spot files |
| `tools/gen_worker_fixtures.py`, `tools/tests/test_gen_worker_fixtures.py` | `fixtures/worker/*.jsonl` generator and its tests |
| `fixtures/worker/{river_two_combo,flop_cancel,flop_best_so_far,lock_river,materialization_cases}.jsonl` | generated wire fixtures |
| `fixtures/solver/basic_0p3.json` | V1 regression fixture |

---
## Task 1: Vendor postflop-solver at the pinned commit with the two patches

**Files:**
- Create: `third_party/postflop-solver/` (full source tree of the pinned commit, `target/` excluded), `third_party/postflop-solver/PINNED_COMMIT`, `third_party/postflop-solver/PATCHES.md`
- Modify: `Cargo.toml` (workspace root, from plan 1)
- Test: `third_party/postflop-solver/tests/vendor_smoke.rs`

**Interfaces:**
- Consumes: nothing from the workspace.
- Produces: path dependency `postflop-solver = { path = "../third_party/postflop-solver", features = ["zstd"] }` exposing `ActionTree`, `TreeConfig`, `BetSizeOptions`, `DonkSizeOptions`, `BetSize`, `BoardState`, `Action`, `CardConfig`, `Range`, `PostFlopGame`, `solve_step`, `compute_exploitability`, `finalize`, `card_from_str`, `NOT_DEALT`.

- [ ] **Step 1: Copy the pinned sources**

The clone at the pinned commit (already patched) is `C:/Users/tlmat/AppData/Local/Temp/claude/D--Documents-Projects-PokerAI/5a6f19a1-1055-4a9a-9e96-fa6b5fca4cc9/scratchpad/spike-solver/postflop-solver`. If it is gone, re-clone: `git clone https://github.com/b-inary/postflop-solver third_party/postflop-solver && git -C third_party/postflop-solver checkout 9d1509fe5077d019825f833eed04b16d342dfda1`, then apply the patches of step 2 by hand.

```bash
mkdir -p third_party
cp -r "C:/Users/tlmat/AppData/Local/Temp/claude/D--Documents-Projects-PokerAI/5a6f19a1-1055-4a9a-9e96-fa6b5fca4cc9/scratchpad/spike-solver/postflop-solver" third_party/postflop-solver
rm -rf third_party/postflop-solver/.git third_party/postflop-solver/target third_party/postflop-solver/Cargo.lock
echo 9d1509fe5077d019825f833eed04b16d342dfda1 > third_party/postflop-solver/PINNED_COMMIT
```

- [ ] **Step 2: Apply and record the two patches**

Edit `third_party/postflop-solver/Cargo.toml` `[dependencies]` so the bincode lines read:

```toml
bincode = { version = "=2.0.0-rc.3", optional = true }
bincode_derive = { version = "=2.0.0-rc.3", optional = true }
```

and under `[features]` set `bincode = ["dep:bincode", "dep:bincode_derive"]` (the derive crate is otherwise resolved transitively at the wrong version). Verify with `grep -n "children)\[" third_party/postflop-solver/src/action_tree.rs` that lines 393, 396 and 408 read `&*(&(*node).children)[0].lock()` / `[index].lock()`; if not, apply that change. Write `third_party/postflop-solver/PATCHES.md`:

```markdown
# Local patches to b-inary/postflop-solver (pinned 9d1509fe5077d019825f833eed04b16d342dfda1, AGPL-3.0-or-later)

1. `Cargo.toml`: `bincode = "=2.0.0-rc.3"` and `bincode_derive = "=2.0.0-rc.3"` (a fresh lockfile resolves 2.0.1, whose derive API breaks `src/mutex_like.rs`).
2. `src/action_tree.rs` lines 393, 396, 408: `&*(*node).children[i].lock()` -> `&*(&(*node).children)[i].lock()` (Rust 1.95 deny-by-default lint `dangerous_implicit_autorefs`).

No other source change. `LICENSE` is upstream's, unchanged. Build with `-C target-feature=+avx2` (workspace `.cargo/config.toml`).
```

- [ ] **Step 3: Wire the workspace (exclude only)**

The vendored tree has its own lockfile and dependency set and must not join the workspace. In the root `Cargo.toml`, inside the existing `[workspace]` table (plan 1 Task 1), add exactly one key and change nothing else:

```toml
exclude = ["third_party/postflop-solver"]
```

Do **not** add any `members` entry here. Plan 1's `members = ["crates/*"]` glob picks up `crates/engine` (Task 2) and `crates/bench` (Task 5) as soon as their manifests exist, and `"solver-worker"` is added by Task 7, which creates that crate. A `members` entry naming a directory without a manifest makes cargo refuse to load the workspace ("failed to load manifest for workspace member"), which would break every command from here to Task 7.

- [ ] **Step 4: Write the smoke test**

`third_party/postflop-solver/tests/vendor_smoke.rs`:

```rust
use postflop_solver::*;

#[test]
fn river_tree_root_menu_matches_r8() {
    let bet = BetSizeOptions { bet: vec![BetSize::PotRelative(0.33), BetSize::PotRelative(0.75), BetSize::AllIn], raise: vec![BetSize::PrevBetRelative(2.5)] };
    let cfg = TreeConfig {
        initial_state: BoardState::River, starting_pot: 100, effective_stack: 100, rake_rate: 0.0, rake_cap: 0.0,
        flop_bet_sizes: [bet.clone(), bet.clone()], turn_bet_sizes: [bet.clone(), bet.clone()], river_bet_sizes: [bet.clone(), bet],
        turn_donk_sizes: Some(DonkSizeOptions { donk: vec![] }), river_donk_sizes: Some(DonkSizeOptions { donk: vec![] }),
        add_allin_threshold: 1.5, force_allin_threshold: 0.0, merging_threshold: 0.0,
    };
    let tree = ActionTree::new(cfg).unwrap();
    assert_eq!(format!("{:?}", tree.available_actions()), "[Check, Bet(33), Bet(75), AllIn(100)]");
}
```

- [ ] **Step 5: Run it**

Run: `cargo test --manifest-path third_party/postflop-solver/Cargo.toml --release --test vendor_smoke`
Expected: PASS; `cargo tree --manifest-path third_party/postflop-solver/Cargo.toml -i bincode` shows `bincode v2.0.0-rc.3`.

Then confirm the workspace still loads and is green:

Run: `cargo test --workspace --release`
Expected: plan 1's tests pass, unchanged (the vendored tree is excluded and is not a member).

This is also the task where plan 1's V1 toolchain check is consumed: if that check selected the GNU fallback for the solver (MSVC build failure, or more than 25% slower), prefix the two `--manifest-path` commands above and every later `-p solver-worker` command with `+stable-x86_64-pc-windows-gnu`. Record which branch was taken in `third_party/postflop-solver/PATCHES.md` under a `## Toolchain` heading (one line: the toolchain, the two measured `vendor_smoke --release` wall times, and the ratio). Do not add a second `rust-toolchain.toml`.

- [ ] **Step 6: Commit**

```bash
git add third_party Cargo.toml
git commit -m "chore: vendor postflop-solver at 9d1509fe with the bincode pin and autoref patch

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 2: Engine crate skeleton: clock, identity, tree templates

**Files:**
- Create: `crates/engine/Cargo.toml`, `crates/engine/src/lib.rs`, `crates/engine/src/clock.rs`, `crates/engine/src/identity.rs`, `crates/engine/src/tree/mod.rs`, `crates/engine/src/tree/templates.rs`
- Test: unit tests inside `identity.rs` and `templates.rs`

**Interfaces:**
- Consumes: `proto::{DecisionIdentity, Street, PlayerMenus, SideMenu, MenuSize}`.
- Produces: `clock::Clock { fn now_ms(&self) -> u64; fn wait_until(&self, t_ms: u64); }`, `clock::SystemClock`; `identity::IdentityState::{new(), set_config() -> u32, begin_hand() -> (u64, u32), mutate() -> u32, next_decision(&mut self) -> Option<DecisionIdentity>, is_active(&DecisionIdentity) -> bool, invalidate_hand(), cancel_active()}`; `tree::templates::{TemplateSpec, Templates::get(&str) -> Option<&'static TemplateSpec>, Templates::ids() -> Vec<&'static str>, Templates::min_variant(&str) -> Option<&'static str>, Templates::with_extra(&[TemplateSpec])}` (the last one behind `cfg(any(test, feature = "test-templates"))`); `EngineError`.

- [ ] **Step 1: Crate manifest and lib**

`crates/engine/Cargo.toml` (every shared dependency comes from plan 1's `[workspace.dependencies]`; never a second `sha2` or `thiserror` major):

```toml
[package]
name = "engine"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
proto = { path = "../proto" }
core-model = { path = "../core-model" }
core-ranges = { path = "../core-ranges" }
core-iso = { path = "../core-iso" }
core-eval = { path = "../core-eval" }
serde.workspace = true
serde_json.workspace = true
sha2.workspace = true
hex.workspace = true
thiserror.workspace = true

[features]
# Fake clock, fake worker and recording sink (Task 19). Never enabled by a release consumer.
testing = []
# Extra tree templates registered by a downstream test harness (plan 4 Task 8).
test-templates = []
```

`crates/engine` needs no `members` edit: plan 1's `members = ["crates/*"]` glob covers it from the moment this manifest exists.

`crates/engine/src/lib.rs`:

```rust
pub mod clock;
pub mod identity;
pub mod tree;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("engine error: {0}")]
    Message(String),
    #[error("rules: {0}")]
    Rules(String),
    #[error("overflow: pot + stacks must stay below 2^31")]
    Overflow,
}
```

- [ ] **Step 2: Failing identity test**

Create `crates/engine/src/identity.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mutation_invalidates_and_revisions_never_repeat() {
        let mut s = IdentityState::new();
        let cfg = s.set_config();
        let (hand, rev1) = s.begin_hand();
        let id1 = s.next_decision().unwrap();
        assert_eq!((id1.hand_id, id1.hand_revision, id1.config_revision, id1.model_revision), (hand, rev1, cfg, 0));
        assert!(s.is_active(&id1));
        let rev2 = s.mutate();
        assert!(rev2 > rev1);
        assert!(!s.is_active(&id1));
        let id2 = s.next_decision().unwrap();
        let id3 = s.next_decision().unwrap();
        assert!(id3.decision_id > id2.decision_id && !s.is_active(&id2) && s.is_active(&id3));
        s.invalidate_hand();
        assert!(s.next_decision().is_none() && !s.is_active(&id3));
    }
}
```

- [ ] **Step 3: Run it to see it fail**

Run: `cargo test -p engine identity`
Expected: FAIL (`IdentityState` not found).

- [ ] **Step 4: Implement `clock.rs` and `identity.rs`**

`crates/engine/src/clock.rs`:

```rust
use std::time::{Duration, Instant};

pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
    /// Blocks until `now_ms() >= t_ms` (returns at once when already past).
    fn wait_until(&self, t_ms: u64);
}

pub struct SystemClock { origin: Instant }

impl SystemClock { pub fn new() -> Self { Self { origin: Instant::now() } } }
impl Default for SystemClock { fn default() -> Self { Self::new() } }

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 { self.origin.elapsed().as_millis() as u64 }
    fn wait_until(&self, t_ms: u64) {
        let now = self.now_ms();
        if t_ms > now { std::thread::sleep(Duration::from_millis(t_ms - now)); }
    }
}
```

`crates/engine/src/identity.rs` (above the test module):

```rust
use proto::DecisionIdentity;

/// Session counters of §4.4/§5: revisions are never reused; a mutation, completion or
/// abandonment invalidates every earlier identity of the hand.
#[derive(Debug, Default)]
pub struct IdentityState {
    config_revision: u32,
    model_revision: u32,
    next_hand_id: u64,
    next_revision: u32,
    next_decision_id: u64,
    hand: Option<(u64, u32)>,           // (hand_id, current hand_revision)
    active: Option<DecisionIdentity>,
}

impl IdentityState {
    pub fn new() -> Self { Self { next_hand_id: 1, next_revision: 1, next_decision_id: 1, ..Default::default() } }
    pub fn set_config(&mut self) -> u32 { self.config_revision += 1; self.config_revision }
    pub fn config_revision(&self) -> u32 { self.config_revision }
    pub fn begin_hand(&mut self) -> (u64, u32) {
        let hand_id = self.next_hand_id; self.next_hand_id += 1;
        let rev = self.next_revision; self.next_revision += 1;
        self.hand = Some((hand_id, rev)); self.active = None;
        (hand_id, rev)
    }
    /// Every apply_action / set_board / undo: fresh revision, in-flight work invalidated.
    pub fn mutate(&mut self) -> u32 {
        let rev = self.next_revision; self.next_revision += 1;
        if let Some(h) = self.hand.as_mut() { h.1 = rev; }
        self.active = None;
        rev
    }
    pub fn current_revision(&self) -> Option<u32> { self.hand.map(|h| h.1) }
    pub fn next_decision(&mut self) -> Option<DecisionIdentity> {
        let (hand_id, hand_revision) = self.hand?;
        let decision_id = self.next_decision_id; self.next_decision_id += 1;
        let id = DecisionIdentity { hand_id, hand_revision, decision_id, config_revision: self.config_revision, model_revision: self.model_revision };
        self.active = Some(id.clone());
        Some(id)
    }
    pub fn is_active(&self, id: &DecisionIdentity) -> bool { self.active.as_ref() == Some(id) }
    pub fn active(&self) -> Option<&DecisionIdentity> { self.active.as_ref() }
    /// finish_hand / abandon_hand: no decision can be requested until the next begin_hand.
    pub fn invalidate_hand(&mut self) { self.hand = None; self.active = None; }
    pub fn cancel_active(&mut self) { self.active = None; }
}
```

- [ ] **Step 5: Failing template test**

Create `crates/engine/src/tree/mod.rs` with `pub mod templates;` and `crates/engine/src/tree/templates.rs` starting with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proto::{MenuSize, PlayerMenus, SideMenu, Street};
    #[test]
    fn river_std_matches_spec_10_1() {
        let t = Templates::get("river_std_v1").unwrap();
        let m = &t.menus[&Street::River];
        assert_eq!(m.oop.bet, vec![MenuSize::Pot(0.33), MenuSize::Pot(0.75), MenuSize::AllIn]);
        assert_eq!(m.oop.raise, vec![MenuSize::Pot(2.5)]);
        assert_eq!((t.add_allin_threshold, t.force_allin_threshold, t.wager_cap, t.merging_threshold), (1.5, 0.0, 3, 0.0));
        assert_eq!(Templates::get("flop_fast_v1").unwrap().menus[&Street::Turn].donk, Some(vec![]));
        assert_eq!(Templates::min_variant("turn_std_v1"), Some("turn_min_v1"));
        assert_eq!(Templates::min_variant("river_min_v1"), None);
        // the seven §10.1 ids are present by name, plus this plan's two §13.2 test templates
        for id in ["flop_fast_v1", "flop_full_v1", "flop_min_v1", "turn_std_v1", "turn_min_v1", "river_std_v1", "river_min_v1"] {
            assert!(Templates::ids().contains(&id), "missing {id}");
        }
        assert_eq!(Templates::ids().len(), 9);
    }

    #[test]
    fn extra_templates_register_and_do_not_disturb_the_production_set() {
        // plan 4 Task 8 registers `check_jam_test_v1`, `menu_round_test_v1` and `check_only_test_v1` through this seam
        let extra = [TemplateSpec { id: "check_only_test_v1", root_street: Street::River,
            menus: std::collections::BTreeMap::from([(Street::River, PlayerMenus { oop: SideMenu { bet: vec![], raise: vec![] }, ip: SideMenu { bet: vec![], raise: vec![] }, donk: None })]),
            add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1 }];
        Templates::with_extra(&extra);
        assert_eq!(Templates::get("check_only_test_v1").unwrap().wager_cap, 1);
        assert_eq!(Templates::get("river_std_v1").unwrap().wager_cap, 3);
        assert!(Templates::ids().len() >= 10 && Templates::min_variant("check_only_test_v1").is_none());
        Templates::with_extra(&[]);   // idempotent reset so test order never matters
        assert_eq!(Templates::ids().len(), 9);
    }
}
```

- [ ] **Step 6: Implement the templates (§10.1 plus the test templates of §13.2)**

Above the test module in `templates.rs`:

```rust
use proto::{MenuSize, PlayerMenus, SideMenu, Street};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateSpec {
    pub id: &'static str,
    pub root_street: Street,
    pub menus: BTreeMap<Street, PlayerMenus>,
    pub add_allin_threshold: f32,
    pub force_allin_threshold: f32,
    pub merging_threshold: f32,
    pub wager_cap: u8,
}

fn side(bet: &[MenuSize], raise: &[MenuSize]) -> SideMenu { SideMenu { bet: bet.to_vec(), raise: raise.to_vec() } }
fn pm(bet: &[MenuSize], raise: &[MenuSize], donk: Option<Vec<MenuSize>>) -> PlayerMenus {
    PlayerMenus { oop: side(bet, raise), ip: side(bet, raise), donk }
}
const P: fn(f32) -> MenuSize = MenuSize::Pot;
const A: MenuSize = MenuSize::AllIn;

fn spec(id: &'static str, root: Street, streets: &[(Street, &[MenuSize])], raise: &[MenuSize], add: f32, force: f32, cap: u8) -> TemplateSpec {
    let mut menus = BTreeMap::new();
    for (s, bets) in streets {
        // Donk menus (§4.6): `None` on the root street, the explicit empty list on every later street.
        // A root-street `None` is legal and is never sent to the library: upstream ignores `turn_donk_sizes`
        // at a turn root because `prev_action` is `None` there, so no donk node exists to size. Only a LATER
        // street's `None` is a defect, and Tasks 8 and 13 reject exactly that. Recorded as a deviation.
        let donk = if *s == root { None } else { Some(vec![]) };
        menus.insert(*s, pm(bets, raise, donk));
    }
    TemplateSpec { id, root_street: root, menus, add_allin_threshold: add, force_allin_threshold: force, merging_threshold: 0.0, wager_cap: cap }
}

fn build() -> Vec<TemplateSpec> {
    use Street::*;
    vec![
        spec("flop_fast_v1", Flop, &[(Flop, &[P(0.5)]), (Turn, &[P(0.5)]), (River, &[P(0.5)])], &[P(2.5)], 1.0, 0.15, 3),
        spec("flop_full_v1", Flop, &[(Flop, &[P(0.33), P(0.75)]), (Turn, &[P(0.33), P(0.75)]), (River, &[P(0.33), P(0.75)])], &[P(2.5)], 1.0, 0.15, 3),
        spec("flop_min_v1", Flop, &[(Flop, &[P(0.75)]), (Turn, &[P(0.75)]), (River, &[P(0.75)])], &[A], 1.5, 0.15, 1),
        spec("turn_std_v1", Turn, &[(Turn, &[P(0.33), P(0.75)]), (River, &[P(0.33), P(0.75)])], &[P(2.5)], 1.5, 0.15, 3),
        spec("turn_min_v1", Turn, &[(Turn, &[P(0.75)]), (River, &[P(0.75)])], &[A], 1.5, 0.15, 1),
        spec("river_std_v1", River, &[(River, &[P(0.33), P(0.75), A])], &[P(2.5)], 1.5, 0.0, 3),
        spec("river_min_v1", River, &[(River, &[P(0.75)])], &[A], 1.5, 0.0, 1),
        // test templates of §13.2 / §4.5
        spec("facing_test_v1", Flop, &[(Flop, &[P(1.0)]), (Turn, &[P(1.0)]), (River, &[P(1.0)])], &[P(2.5)], 1.0, 0.15, 3),
        TemplateSpec { id: "river_oracle_v1", root_street: River,
            menus: BTreeMap::from([(River, PlayerMenus { oop: side(&[], &[]), ip: side(&[P(1.0)], &[]), donk: None })]),
            add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1 },
    ]
}

/// Registry of the §10.1 production templates plus this plan's two §13.2 test templates.
/// `with_extra` is the single seam a downstream test harness uses to add its own templates
/// (plan 4 Task 8 registers `check_jam_test_v1`, `menu_round_test_v1`, `check_only_test_v1`);
/// it is compiled out of release builds, so the production registry is always exactly 9 ids.
pub struct Templates;

#[cfg(any(test, feature = "test-templates"))]
static EXTRA: std::sync::RwLock<Vec<&'static TemplateSpec>> = std::sync::RwLock::new(Vec::new());

impl Templates {
    fn base() -> &'static [TemplateSpec] { static T: OnceLock<Vec<TemplateSpec>> = OnceLock::new(); T.get_or_init(build) }
    /// Replaces the extra registrations with `extra` (pass `&[]` to clear). Each entry is leaked once so
    /// `get` can keep returning `&'static`; test harnesses call this a handful of times per process.
    #[cfg(any(test, feature = "test-templates"))]
    pub fn with_extra(extra: &[TemplateSpec]) {
        let leaked: Vec<&'static TemplateSpec> = extra.iter().cloned().map(|t| &*Box::leak(Box::new(t))).collect();
        *EXTRA.write().unwrap() = leaked;
    }
    #[cfg(any(test, feature = "test-templates"))]
    fn extra() -> Vec<&'static TemplateSpec> { EXTRA.read().unwrap().clone() }
    #[cfg(not(any(test, feature = "test-templates")))]
    fn extra() -> Vec<&'static TemplateSpec> { Vec::new() }
    /// Extra registrations shadow the base set, so a harness can also override a production template.
    pub fn get(id: &str) -> Option<&'static TemplateSpec> {
        Self::extra().into_iter().find(|t| t.id == id).or_else(|| Self::base().iter().find(|t| t.id == id))
    }
    pub fn ids() -> Vec<&'static str> {
        let mut v: Vec<&'static str> = Self::base().iter().map(|t| t.id).collect();
        for t in Self::extra() { if !v.contains(&t.id) { v.push(t.id); } }
        v
    }
    /// The crash/timeout/TreeTooLarge retry template of §10.1 (`_min` of the same street); None for a `_min` or test template.
    pub fn min_variant(id: &str) -> Option<&'static str> {
        match id { "flop_fast_v1" | "flop_full_v1" => Some("flop_min_v1"), "turn_std_v1" => Some("turn_min_v1"), "river_std_v1" => Some("river_min_v1"), _ => None }
    }
}
```

- [ ] **Step 7: Run and commit**

Run: `cargo test -p engine` then `cargo test --workspace --release`
Expected: 3 passed in `engine` (`mutation_invalidates_and_revisions_never_repeat`, `river_std_matches_spec_10_1`, `extra_templates_register_and_do_not_disturb_the_production_set`); the workspace stays green.

```bash
git add crates/engine
git commit -m "feat(engine): crate skeleton with clock, identity counters and the section 10.1 templates

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 3: Engine materializer with the pinned §4.6 rules

**Files:**
- Create: `crates/engine/src/tree/materialize.rs`
- Modify: `crates/engine/src/tree/mod.rs`
- Test: unit tests in `materialize.rs`

**Interfaces:**
- Consumes: `Templates`, `TemplateSpec` (Task 2); `proto::{Action, MaterializedNode, MenuSize, OrdinalPath, Street, UnsupportedReason}`.
- Produces: `tree::materialize::{MaterializeInput { template: &TemplateSpec, starting_pot: u32, eff: u32, prefix: &[(usize, Action)] }, Materialized { nodes: Vec<MaterializedNode>, inserted: Vec<(Vec<Action>, String, Action)>, history: Vec<Action>, decision_path: OrdinalPath }, materialize(&MaterializeInput) -> Result<Materialized, UnsupportedReason>, actor_name(usize) -> String, order_key(&Action) -> (u8, u32)}`. Actor index 0 = oop, 1 = ip.

- [ ] **Step 1: Write the failing boundary tests**

Create `crates/engine/src/tree/materialize.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::templates::Templates;
    use proto::{Action, Street};

    fn mat(template: &str, p: u32, eff: u32, prefix: &[(usize, Action)]) -> Materialized {
        materialize(&MaterializeInput { template: Templates::get(template).unwrap(), starting_pot: p, eff, prefix }).unwrap()
    }
    fn node<'a>(m: &'a Materialized, path: &[u8]) -> &'a MaterializedNode { m.nodes.iter().find(|n| n.path == path).unwrap() }
    fn bet(to: u32) -> Action { Action::Bet { to } }
    fn raise(to: u32) -> Action { Action::Raise { to } }
    fn allin(to: u32) -> Action { Action::AllIn { to } }

    #[test]
    fn river_oracle_wire_example() {
        let m = mat("river_oracle_v1", 100, 100, &[(0, Action::Check)]);
        assert_eq!(m.history, vec![Action::Check]);
        assert_eq!(m.decision_path, vec![0]);
        assert_eq!(m.inserted.len(), 0);
        let r = node(&m, &[]);
        assert_eq!((r.actor.as_str(), &r.actions[..], &r.terminal_pots[..]), ("oop", &[Action::Check][..], &[None][..]));
        let ip = node(&m, &[0]);
        assert_eq!((&ip.actions[..], &ip.terminal_pots[..]), (&[Action::Check, allin(100)][..], &[Some(100), None][..]));
        let oop = node(&m, &[0, 1]);
        assert_eq!((oop.actor.as_str(), &oop.actions[..], &oop.terminal_pots[..]), ("oop", &[Action::Fold, Action::Call][..], &[Some(100), Some(300)][..]));
        assert_eq!(m.nodes.len(), 3);
    }

    #[test]
    fn opening_allin_and_force_boundaries() {
        // turn_std_v1 (no `a` entry), add 1.5: P=100, stack 150 lists AllIn(150) (150 <= 150); 151 does not
        assert!(node(&mat("turn_std_v1", 100, 150, &[]), &[]).actions.contains(&allin(150)));
        assert!(!node(&mat("turn_std_v1", 100, 151, &[]), &[]).actions.iter().any(|a| matches!(a, Action::AllIn { .. })));
        // river_std_v1 at P=100, stacks 100 reproduces the measured RIVER root menu (its `a` entry and the threshold both list the all-in)
        assert_eq!(node(&mat("river_std_v1", 100, 100, &[]), &[]).actions, vec![Action::Check, bet(33), bet(75), allin(100)]);
        // flop_fast_v1 opening bet 0.5 at force 0.15: stack 80 forces (80 <= 50 + 30), 81 does not
        assert_eq!(node(&mat("flop_fast_v1", 100, 80, &[]), &[]).actions, vec![Action::Check, allin(80)]);
        assert_eq!(node(&mat("flop_fast_v1", 100, 81, &[]), &[]).actions, vec![Action::Check, bet(50), allin(81)]);
    }

    #[test]
    fn facing_boundaries_of_section_4_6() {
        let prefix = [(0usize, bet(100))];
        let ip = |eff: u32| node(&mat("facing_test_v1", 100, eff, &prefix), &[1]).actions.clone();
        assert_eq!(ip(350), vec![Action::Fold, Action::Call, raise(250), allin(350)]);
        assert_eq!(ip(400), vec![Action::Fold, Action::Call, raise(250), allin(400)]);
        assert_eq!(ip(401), vec![Action::Fold, Action::Call, raise(250)]);
        assert_eq!(ip(340), vec![Action::Fold, Action::Call, allin(340)]);
        assert_eq!(ip(341), vec![Action::Fold, Action::Call, raise(250), allin(341)]);
        assert_eq!(ip(240), vec![Action::Fold, Action::Call, allin(240)]);
        // stack 100: OOP's bet becomes the all-in; IP's node is Fold, Call only
        let m = mat("facing_test_v1", 100, 100, &prefix);
        assert_eq!(m.history, vec![allin(100)]);
        assert_eq!(node(&m, &[1]).actions, vec![Action::Fold, Action::Call]);
    }

    #[test]
    fn cross_street_operand_and_donk_pinning() {
        let m = mat("facing_test_v1", 100, 350, &[]);
        // after OOP Bet(100), IP Call: turn root is an opening node with matched = 100, pot = 300, stacks 250/250
        let turn = node(&m, &[1, 1]);
        assert_eq!((turn.street, turn.actor.as_str(), &turn.actions[..]), (Street::Turn, "oop", &[Action::Check, allin(250)][..]));
        // after OOP Check, IP Bet(100), OOP Call: a donk node with the same operand
        let donk = node(&m, &[0, 1, 1]);
        assert_eq!((donk.street, &donk.actions[..]), (Street::Turn, &[Action::Check, allin(250)][..]));
        // after turn Check, Check the river keeps matched = 100, pot = 300
        let river = node(&m, &[1, 1, 0, 0]);
        assert_eq!((river.street, &river.actions[..]), (Street::River, &[Action::Check, allin(250)][..]));
        // terminal markers: IP fold after the flop bet closes at the matched pot 100; a call is a continuation
        let ip = node(&m, &[1]);
        assert_eq!(ip.terminal_pots[0], Some(100));
        assert_eq!(ip.terminal_pots[1], None);
    }

    #[test]
    fn exact_insertion_and_wager_cap() {
        // villain (oop) bets 73 into 100 with the 0.5 menu: both 50 and 73 are present, 73 inserted, requested node is IP's after 73
        let m = mat("flop_fast_v1", 100, 500, &[(0, bet(73))]);
        assert_eq!(node(&m, &[]).actions, vec![Action::Check, bet(50), bet(73)]);
        assert_eq!(m.inserted, vec![(vec![], "oop".to_string(), bet(73))]);
        assert_eq!(m.decision_path, vec![2]);
        assert_eq!(m.history, vec![bet(73)]);
        // cap 1 (flop_min_v1): after one wager only fold/call/all-in remain; a two-wager prefix is kept intact
        let m1 = mat("flop_min_v1", 100, 500, &[(0, bet(40)), (1, raise(120))]);
        assert_eq!(node(&m1, &[1]).actions, vec![Action::Fold, Action::Call, raise(120), allin(500)]);
        assert_eq!(node(&m1, &[1, 2]).actions, vec![Action::Fold, Action::Call, allin(500)]);
        assert_eq!(m1.decision_path, vec![1, 2]);
        // a wager below the tree minimum is UnsupportedHistory
        let e = materialize(&MaterializeInput { template: Templates::get("flop_fast_v1").unwrap(), starting_pot: 100, eff: 500, prefix: &[(0, bet(30)), (1, raise(40))] });
        assert!(matches!(e, Err(proto::UnsupportedReason::UnsupportedHistory { .. })));
    }
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p engine materialize`
Expected: FAIL (`materialize` not found).

- [ ] **Step 3: Implement the materializer**

Above the tests in `materialize.rs` (add `pub mod materialize;` to `tree/mod.rs`):

```rust
use super::templates::TemplateSpec;
use proto::{Action, MaterializedNode, MenuSize, OrdinalPath, Street, UnsupportedReason};

pub const OOP: usize = 0;
pub const IP: usize = 1;

pub fn actor_name(actor: usize) -> String { if actor == OOP { "oop".into() } else { "ip".into() } }
pub fn order_key(a: &Action) -> (u8, u32) {
    match a { Action::Fold => (0, 0), Action::Check => (1, 0), Action::Call => (2, 0), Action::Bet { to } | Action::Raise { to } => (3, *to), Action::AllIn { to } => (4, *to) }
}
fn unsupported(reason: impl Into<String>) -> UnsupportedReason { UnsupportedReason::UnsupportedHistory { reason: reason.into() } }
fn round(x: f64) -> i64 { x.round() as i64 }   // f64::round: half away from zero, as upstream

#[derive(Clone, Copy, PartialEq, Debug)]
enum Prev { Root, Check, Chance, Wager }
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind { Opening, Donk, Facing }

/// Mirror of upstream `BuildTreeInfo` plus the street/actor bookkeeping of the betting skeleton.
#[derive(Clone, Debug)]
struct Walk { street: Street, actor: usize, prev: Prev, wagers_this_street: u8, allin: bool, oop_call: bool, stack: [i64; 2], prev_amount: i64, matched: i64 }

struct Operands { to_call: i64, pot: i64, max: i64, min: i64 }
fn operands(w: &Walk, p: i64) -> Operands {
    let own = w.stack[w.actor];
    let opp = w.stack[w.actor ^ 1];
    let to_call = own - opp;
    let pot = p + 2 * (w.matched + to_call);
    let max = opp + w.prev_amount;
    let min = (w.prev_amount + to_call).clamp(1, max);
    Operands { to_call, pot, max, min }
}
fn kind(w: &Walk) -> Kind {
    match w.prev { Prev::Chance if w.oop_call => Kind::Donk, Prev::Root | Prev::Check | Prev::Chance => Kind::Opening, Prev::Wager => Kind::Facing }
}

/// The menu of a node: built, clamped, forced, sorted and deduplicated exactly as upstream `push_actions` (§4.6).
fn menu(w: &Walk, t: &TemplateSpec, p: i64) -> Result<Vec<Action>, UnsupportedReason> {
    let o = operands(w, p);
    let street = t.menus.get(&w.street).ok_or_else(|| unsupported(format!("template {} has no menu for {:?}", t.id, w.street)))?;
    let side = if w.actor == OOP { &street.oop } else { &street.ip };
    let add = t.add_allin_threshold as f64;
    let force = t.force_allin_threshold as f64;
    if t.merging_threshold != 0.0 { return Err(unsupported("merging_threshold is pinned to 0.0")); }
    let allin = Action::AllIn { to: o.max as u32 };
    let mut v: Vec<Action> = Vec::new();
    match kind(w) {
        Kind::Donk => {
            v.push(Action::Check);
            let donk = street.donk.as_ref().ok_or_else(|| unsupported("donk menu must be the explicit empty list"))?;
            if !donk.is_empty() { return Err(unsupported("donk sizes are pinned empty in phase 1")); }
            if o.max <= round(o.pot as f64 * add) { v.push(allin.clone()); }
        }
        Kind::Opening => {
            v.push(Action::Check);
            for b in &side.bet {
                match b { MenuSize::Pot(r) => v.push(Action::Bet { to: round(o.pot as f64 * *r as f64) as u32 }), MenuSize::AllIn => v.push(allin.clone()) }
            }
            if o.max <= round(o.pot as f64 * add) { v.push(allin.clone()); }
        }
        Kind::Facing => {
            v.push(Action::Fold);
            v.push(Action::Call);
            if !w.allin {
                for r in &side.raise {
                    match r { MenuSize::Pot(x) => v.push(Action::Raise { to: round(w.prev_amount as f64 * *x as f64) as u32 }), MenuSize::AllIn => v.push(allin.clone()) }
                }
                if o.max <= w.prev_amount + round(o.pot as f64 * add) { v.push(allin.clone()); }
            }
        }
    }
    for a in v.iter_mut() {
        let (amt, is_bet) = match *a { Action::Bet { to } => (to as i64, true), Action::Raise { to } => (to as i64, false), _ => continue };
        let c = amt.clamp(o.min, o.max);
        let thr = round((o.pot + 2 * (c - w.prev_amount)) as f64 * force);
        *a = if o.max <= c + thr { allin.clone() } else if is_bet { Action::Bet { to: c as u32 } } else { Action::Raise { to: c as u32 } };
    }
    v.sort_by_key(order_key);
    v.dedup();
    Ok(v)
}

/// Observed action -> tree action at this node (§10.2 exact insertion, §4.6 all-in normalization).
fn map_observed(k: Kind, o: &Operands, obs: &Action) -> Result<Action, UnsupportedReason> {
    match obs {
        Action::Fold | Action::Check | Action::Call => Ok(obs.clone()),
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => {
            let to = *to as i64;
            if to >= o.max { return Ok(Action::AllIn { to: o.max as u32 }); }   // equal to max is forced; above max is the deeper stack's uncontestable excess
            if matches!(obs, Action::AllIn { .. }) { return Err(unsupported(format!("all-in to {to} below the effective maximum {}", o.max))); }
            if to < o.min { return Err(unsupported(format!("wager to {to} below the tree minimum {}", o.min))); }
            Ok(match k { Kind::Facing => Action::Raise { to: to as u32 }, _ => Action::Bet { to: to as u32 } })
        }
    }
}

struct Child { walk: Walk, terminal_pot: Option<u32> }
fn next_street(n: &mut Walk) {
    n.street = match n.street { Street::Flop => Street::Turn, Street::Turn => Street::River, s => s };
    n.actor = OOP; n.prev = Prev::Chance; n.wagers_this_street = 0;
}
/// Mirror of upstream `create_next` plus the terminal classification of §2 (fold, called wager, showdown).
fn child(w: &Walk, a: &Action, p: i64) -> Child {
    let o = operands(w, p);
    let mut n = w.clone();
    let river = w.street == Street::River;
    match a {
        Action::Fold => Child { walk: n, terminal_pot: Some((p + 2 * w.matched) as u32) },
        Action::Check => {
            n.oop_call = false; n.prev = Prev::Check;
            if w.actor == OOP { n.actor = IP; return Child { walk: n, terminal_pot: None }; }
            if river { return Child { walk: n, terminal_pot: Some((p + 2 * w.matched) as u32) }; }
            next_street(&mut n); Child { walk: n, terminal_pot: None }
        }
        Action::Call => {
            n.matched += o.to_call; n.stack[w.actor] = w.stack[w.actor ^ 1]; n.prev_amount = 0; n.oop_call = w.actor == OOP; n.prev = Prev::Chance;
            if river || w.allin { return Child { walk: n, terminal_pot: Some((p + 2 * n.matched) as u32) }; }
            next_street(&mut n); Child { walk: n, terminal_pot: None }
        }
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => {
            let amt = *to as i64;
            n.matched += o.to_call;
            n.stack[w.actor] -= amt - w.prev_amount + o.to_call;
            n.prev_amount = amt;
            n.allin = matches!(a, Action::AllIn { .. });
            if !n.allin { n.wagers_this_street += 1; }
            n.actor = w.actor ^ 1; n.prev = Prev::Wager;
            Child { walk: n, terminal_pot: None }
        }
    }
}

pub struct MaterializeInput<'a> { pub template: &'a TemplateSpec, pub starting_pot: u32, pub eff: u32, pub prefix: &'a [(usize, Action)] }
#[derive(Debug, Clone, Default)]
pub struct Materialized { pub nodes: Vec<MaterializedNode>, pub inserted: Vec<(Vec<Action>, String, Action)>, pub history: Vec<Action>, pub decision_path: OrdinalPath }

pub fn materialize(inp: &MaterializeInput) -> Result<Materialized, UnsupportedReason> {
    if inp.starting_pot == 0 || inp.eff == 0 { return Err(unsupported("zero pot or zero effective stack")); }
    if (inp.starting_pot as u64) + 2 * (inp.eff as u64) >= (1u64 << 31) { return Err(unsupported("pot + stacks exceed 2^31")); }
    let p = inp.starting_pot as i64;
    let root = Walk { street: inp.template.root_street, actor: OOP, prev: Prev::Root, wagers_this_street: 0, allin: false, oop_call: false, stack: [inp.eff as i64; 2], prev_amount: 0, matched: 0 };
    let mut out = Materialized::default();
    let mut found = false;
    walk(inp, p, &root, &mut Vec::new(), &mut Vec::new(), true, &mut out, &mut found)?;
    if !found { return Err(unsupported("prefix closes the street before hero's decision")); }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk(inp: &MaterializeInput, p: i64, w: &Walk, path: &mut OrdinalPath, chip: &mut Vec<Action>, on_prefix: bool, out: &mut Materialized, found: &mut bool) -> Result<(), UnsupportedReason> {
    let t = inp.template;
    let mut menu = menu(w, t, p)?;
    let mut prefix_action: Option<Action> = None;
    if on_prefix {
        if path.len() < inp.prefix.len() {
            let (actor, obs) = &inp.prefix[path.len()];
            if *actor != w.actor { return Err(unsupported(format!("prefix actor mismatch at step {}", path.len()))); }
            let mapped = map_observed(kind(w), &operands(w, p), obs)?;
            if !menu.contains(&mapped) {
                if matches!(mapped, Action::Fold | Action::Check | Action::Call) { return Err(unsupported(format!("{mapped:?} not available at step {}", path.len()))); }
                menu.push(mapped.clone());
                menu.sort_by_key(order_key);
                menu.dedup();
                out.inserted.push((chip.clone(), actor_name(w.actor), mapped.clone()));
            }
            prefix_action = Some(mapped);
        } else {
            out.decision_path = path.clone();
            *found = true;
        }
    }
    // wager cap (§4.6): observed prefix actions are never removed
    if w.wagers_this_street >= t.wager_cap {
        menu.retain(|a| !matches!(a, Action::Bet { .. } | Action::Raise { .. }) || prefix_action.as_ref() == Some(a));
    }
    let children: Vec<Child> = menu.iter().map(|a| child(w, a, p)).collect();
    out.nodes.push(MaterializedNode { path: path.clone(), street: w.street, actor: actor_name(w.actor), actions: menu.clone(), terminal_pots: children.iter().map(|c| c.terminal_pot).collect() });
    for (i, (a, c)) in menu.iter().zip(children.iter()).enumerate() {
        if c.terminal_pot.is_some() { continue; }
        let child_on_prefix = prefix_action.as_ref() == Some(a);
        path.push(i as u8);
        chip.push(a.clone());
        if child_on_prefix { out.history.push(a.clone()); }
        walk(inp, p, &c.walk, path, chip, child_on_prefix, out, found)?;
        path.pop();
        chip.pop();
    }
    Ok(())
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p engine materialize` then `cargo test --workspace --release`
Expected: 5 passed (every boundary of §4.6 as listed in `tree_materialization_matches_library`); the workspace stays green.

```bash
git add crates/engine/src/tree
git commit -m "feat(engine): betting-skeleton materializer with the pinned section 4.6 rules

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 4: Effective tree with exact insertion, `tree_signature`, chip-path resolution, `tree_builder_golden`

**Files:**
- Create: `crates/engine/src/tree/effective.rs`, `crates/engine/src/tree/signature.rs`, `crates/engine/src/tree/resolve.rs`, `crates/engine/tests/tree_builder.rs`, `crates/engine/tests/golden/tree_builder_golden.json` (recorded on the first green run, see step 4)
- Modify: `crates/engine/src/tree/mod.rs`

**Interfaces:**
- Consumes: `materialize` (Task 3), `Templates` (Task 2); `proto::{StreetRootSnapshot, EffectiveTree, Seat, Action, MaterializedNode, OrdinalPath, UnsupportedReason}`, `proto::worker::validate_solution`.
- Produces: `tree::TemplateSelection { template_id: String, observed: Vec<(Seat, Action)> }` with `from_history(template_id: &str, history: &[(Seat, Action)]) -> Self`; `tree::TreeBuild { tree: EffectiveTree, history: Vec<Action>, decision_path: OrdinalPath, pot: u32, eff: u32 }`; `tree::build_tree_full(&StreetRootSnapshot, &TemplateSelection) -> Result<TreeBuild, UnsupportedReason>`; `tree::build_effective_tree(&StreetRootSnapshot, &TemplateSelection) -> Result<EffectiveTree, UnsupportedReason>`; `tree::materialize_at(template: &TemplateSpec, pot: u32, eff: u32, prefix: &[(usize, Action)]) -> Result<TreeBuild, UnsupportedReason>`; `tree::tree_signature(&EffectiveTree, p: u32) -> String`; `tree::node_at(&[MaterializedNode], &[u8]) -> Option<&MaterializedNode>`; and the re-export `pub use proto::resolve_chip_path;` — the §2 chip-path rule has exactly one implementation, in `proto` (cross-plan M21/D2). This plan never writes a second one.

- [ ] **Step 1: Failing test `crates/engine/tests/tree_builder.rs`**

```rust
use engine::tree::{build_effective_tree, build_tree_full, materialize_at, node_at, resolve_chip_path, tree_signature, TemplateSelection, Templates};
use proto::worker::{validate_solution, NodeStrategy, StreetSolution};
use proto::{Action, MaterializedNode, Seat, Street, StreetRootSnapshot, UnsupportedReason};

fn snap(pot: u32, oop_stack: u32, ip_stack: u32, history: Vec<(Seat, Action)>) -> StreetRootSnapshot {
    StreetRootSnapshot { street: Street::Flop, board: vec![proto::Card::parse("Kh").unwrap(), proto::Card::parse("7d").unwrap(), proto::Card::parse("2c").unwrap()],
        oop: Seat(2), ip: Seat(0), pot_root: pot, stack_oop_root: oop_stack, stack_ip_root: ip_stack, dead_this_street: 0, projected_from: 2, history, bb_chips: 2 }
}
fn bet(to: u32) -> Action { Action::Bet { to } }

#[test]
fn tree_builder_golden() {
    // in-tree detection: 50 into 100 equals the flop_fast_v1 menu size; nothing inserted
    let t = build_tree_full(&snap(100, 500, 700, vec![(Seat(2), bet(50))]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(50))])).unwrap();
    assert!(t.tree.inserted.is_empty());
    assert_eq!((t.pot, t.eff, &t.history[..], &t.decision_path[..]), (100, 500, &[bet(50)][..], &[1u8][..]));
    // insertion alongside menus, duplicates merged, requested node after 73
    let sel = TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(73))]);
    let a = build_tree_full(&snap(100, 500, 700, vec![(Seat(2), bet(73))]), &sel).unwrap();
    assert_eq!(node_at(&a.tree.materialized, &[]).unwrap().actions, vec![Action::Check, bet(50), bet(73)]);
    assert_eq!(a.tree.inserted, vec![(vec![], "oop".to_string(), bet(73))]);
    assert_eq!(a.decision_path, vec![2]);
    // duplicates merged (1): an observed size that lands exactly on a menu size inserts nothing and leaves one entry
    let dup = build_tree_full(&snap(100, 500, 700, vec![(Seat(2), bet(50))]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(50))])).unwrap();
    let root = node_at(&dup.tree.materialized, &[]).unwrap();
    assert!(dup.tree.inserted.is_empty());
    assert_eq!(root.actions, vec![Action::Check, bet(50)]);
    assert_eq!(root.actions.iter().filter(|x| **x == bet(50)).count(), 1);
    // duplicates merged (2): at eff 340 facing_test_v1's menu raise 250 is force-collapsed onto the all-in that the
    // add-threshold also lists; the clamp-force-dedupe order leaves exactly one AllIn(340) and no Raise
    let coll = build_tree_full(&snap(100, 340, 340, vec![(Seat(2), bet(100))]), &TemplateSelection::from_history("facing_test_v1", &[(Seat(2), bet(100))])).unwrap();
    let ip = node_at(&coll.tree.materialized, &[1]).unwrap();
    assert_eq!(ip.actions, vec![Action::Fold, Action::Call, Action::AllIn { to: 340 }]);
    assert_eq!(ip.actions.iter().filter(|x| matches!(x, Action::AllIn { .. })).count(), 1);
    // signature stability across chip scales: 100/500 + 73 versus 200/1000 + 146 (same reduced rational 73/100)
    let b = build_tree_full(&snap(200, 1000, 1400, vec![(Seat(2), bet(146))]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(146))])).unwrap();
    assert_eq!(tree_signature(&a.tree, 100), tree_signature(&b.tree, 200));
    assert_ne!(tree_signature(&a.tree, 100), tree_signature(&t.tree, 100));
    // the materialized lists agree node by node with doubled chips
    assert_eq!(a.tree.materialized.len(), b.tree.materialized.len());
    for (x, y) in a.tree.materialized.iter().zip(&b.tree.materialized) {
        assert_eq!((&x.path, x.street, &x.actor), (&y.path, y.street, &y.actor));
        assert_eq!(x.terminal_pots.iter().map(|p| p.map(|v| v * 2)).collect::<Vec<_>>(), y.terminal_pots);
        for (p, q) in x.actions.iter().zip(&y.actions) {
            match (p, q) { (Action::Bet { to: u }, Action::Bet { to: v }) | (Action::Raise { to: u }, Action::Raise { to: v }) | (Action::AllIn { to: u }, Action::AllIn { to: v }) => assert_eq!(u * 2, *v), _ => assert_eq!(p, q) }
        }
    }
    // facing-node all-in of §4.6 through the snapshot path: eff 350 and 400 list it, 401 does not
    let ip_menu = |eff: u32| { let s = snap(100, eff, eff, vec![(Seat(2), bet(100))]); node_at(&build_effective_tree(&s, &TemplateSelection::from_history("facing_test_v1", &s.history)).unwrap().materialized, &[1]).unwrap().actions.clone() };
    assert!(ip_menu(350).contains(&Action::AllIn { to: 350 }) && ip_menu(400).contains(&Action::AllIn { to: 400 }));
    assert!(!ip_menu(401).iter().any(|x| matches!(x, Action::AllIn { .. })));
    // chip paths of a worker result resolve to ordinal paths; an unresolvable path is "invalid solution"
    let zero = |n: usize| vec![vec![0.0f32; n]; 1326];
    let node = |path: Vec<Action>, actor: &str, actions: Vec<Action>| NodeStrategy { path, actor: actor.into(), probs: zero(actions.len()), ev_chips: zero(actions.len()), actions, available: vec![false; 1326] };
    let sol = StreetSolution { nodes: vec![node(vec![bet(73)], "ip", vec![Action::Fold, Action::Call, Action::Raise { to: 183 }]), node(vec![], "oop", vec![Action::Check, bet(50), bet(73)])],
        requested: 0, exploitability_chips: 0.1, iterations: 10, memory_bytes: 1, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![vec![bet(73)], vec![]] };
    assert_eq!(validate_solution(&sol, &a.tree.materialized).unwrap(), vec![vec![2u8], vec![]]);
    assert_eq!(resolve_chip_path(&a.tree.materialized, &[bet(60)]), None);
    let mut bad = sol.clone(); bad.nodes[0].path = vec![bet(60)]; bad.covered_paths[0] = vec![bet(60)];
    assert!(validate_solution(&bad, &a.tree.materialized).is_err());
    // UnsupportedHistory: a raise below the minimum, a wrong seat, a wrong root street
    assert!(matches!(build_tree_full(&snap(100, 500, 500, vec![(Seat(2), bet(30)), (Seat(0), Action::Raise { to: 40 })]), &TemplateSelection::from_history("flop_fast_v1", &[])), Err(UnsupportedReason::UnsupportedHistory { .. })));
    assert!(matches!(build_tree_full(&snap(100, 500, 500, vec![(Seat(4), bet(30))]), &TemplateSelection::from_history("flop_fast_v1", &[])), Err(UnsupportedReason::UnsupportedHistory { .. })));
    assert!(matches!(build_tree_full(&snap(100, 500, 500, vec![]), &TemplateSelection::from_history("turn_std_v1", &[])), Err(UnsupportedReason::EngineError { .. })));
    // materialize_at for bench/tools and the golden file
    let m = materialize_at(Templates::get("river_oracle_v1").unwrap(), 100, 100, &[(0, Action::Check)]).unwrap();
    golden::check("tree_builder_golden", &serde_json::json!({ "river_oracle": m.tree.materialized, "flop_fast_73": a.tree.materialized }));
}

mod golden {
    pub fn check(name: &str, value: &serde_json::Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.json"));
        if std::env::var("UPDATE_GOLDEN").is_ok() || !path.exists() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
        }
        let stored: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(&stored, value, "golden {name} differs; rerun with UPDATE_GOLDEN=1 after hand-checking");
    }
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p engine --test tree_builder`
Expected: FAIL to compile (`build_tree_full` missing).

- [ ] **Step 3: Implement `resolve.rs`, `signature.rs`, `effective.rs`**

`crates/engine/src/tree/mod.rs`:

```rust
pub mod effective;
pub mod materialize;
pub mod resolve;
pub mod signature;
pub mod templates;
pub use effective::{build_effective_tree, build_tree_full, materialize_at, TemplateSelection, TreeBuild};
pub use resolve::{node_at, resolve_chip_path};
pub use signature::tree_signature;
pub use templates::{TemplateSpec, Templates};
```

`crates/engine/src/tree/resolve.rs` — the §2 chip-path rule has one normative implementation, `proto::resolve_chip_path` (plan 1 Task 6). This module only re-exports it and adds the ordinal lookup, which `proto` does not provide:

```rust
use proto::MaterializedNode;

/// The single implementation of the §2 chip-path rule lives in `proto`; every consumer re-exports it
/// so cache, replay and worker can never disagree (cross-plan M21/D2).
pub use proto::resolve_chip_path;

pub fn node_at<'a>(materialized: &'a [MaterializedNode], path: &[u8]) -> Option<&'a MaterializedNode> {
    materialized.iter().find(|n| n.path.as_slice() == path)
}
```

`crates/engine/src/tree/signature.rs`:

```rust
use proto::{resolve_chip_path, Action, EffectiveTree, MenuSize};
use sha2::{Digest, Sha256};
use std::fmt::Write;

fn gcd(a: u64, b: u64) -> u64 { if b == 0 { a } else { gcd(b, a % b) } }
fn size(m: &MenuSize) -> String { match m { MenuSize::Pot(x) => format!("{x:.6}"), MenuSize::AllIn => "a".into() } }
fn sizes(v: &[MenuSize]) -> String { v.iter().map(size).collect::<Vec<_>>().join(",") }

/// §4.6: sha256 over rules_version, template_id, root_street, nominal menus (explicit donk lists included),
/// thresholds, wager_cap and the inserted sizes as reduced rationals `to / P` at their ordinal paths. No raw chips.
pub fn tree_signature(tree: &EffectiveTree, p: u32) -> String {
    let mut s = String::new();
    let _ = write!(s, "rules={}|template={}|root={:?}|", tree.rules_version, tree.template_id, tree.root_street);
    for (street, m) in &tree.menus {
        let donk = match &m.donk { None => "none".to_string(), Some(d) => format!("[{}]", sizes(d)) };
        let _ = write!(s, "{:?}:oop.bet=[{}],oop.raise=[{}],ip.bet=[{}],ip.raise=[{}],donk={};", street, sizes(&m.oop.bet), sizes(&m.oop.raise), sizes(&m.ip.bet), sizes(&m.ip.raise), donk);
    }
    let _ = write!(s, "|add={:.6}|force={:.6}|merge={:.6}|cap={}|", tree.add_allin_threshold, tree.force_allin_threshold, tree.merging_threshold, tree.wager_cap);
    for (chip, actor, action) in &tree.inserted {
        let path = resolve_chip_path(&tree.materialized, chip).unwrap_or_default();
        let (kind, to) = match action { Action::Bet { to } => ("bet", *to), Action::Raise { to } => ("raise", *to), Action::AllIn { to } => ("allin", *to), Action::Fold => ("fold", 0), Action::Check => ("check", 0), Action::Call => ("call", 0) };
        let g = gcd(to as u64, p as u64).max(1);
        let _ = write!(s, "ins@{:?}:{}:{}:{}/{};", path, actor, kind, to as u64 / g, p as u64 / g);
    }
    hex::encode(Sha256::digest(s.as_bytes()))
}
```

`crates/engine/src/tree/effective.rs`:

```rust
use super::materialize::{materialize, MaterializeInput, Materialized};
use super::templates::{TemplateSpec, Templates};
use proto::{Action, EffectiveTree, OrdinalPath, Seat, StreetRootSnapshot, UnsupportedReason};

/// §4.6 `rules_version`; owned by `proto` (plan 1 Task 6) and re-exported so there is one value in the workspace.
pub use proto::RULES_VERSION;

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateSelection { pub template_id: String, pub observed: Vec<(Seat, Action)> }
impl TemplateSelection {
    /// The observed sizes to insert are the wagers of the current street's history (§10.2); checks/calls are kept for the walk.
    pub fn from_history(template_id: &str, history: &[(Seat, Action)]) -> Self { Self { template_id: template_id.to_string(), observed: history.to_vec() } }
}

#[derive(Debug, Clone)]
pub struct TreeBuild { pub tree: EffectiveTree, pub history: Vec<Action>, pub decision_path: OrdinalPath, pub pot: u32, pub eff: u32 }

fn engine_error(msg: impl Into<String>) -> UnsupportedReason { UnsupportedReason::EngineError { message: msg.into(), retryable: false } }

fn assemble(template: &TemplateSpec, m: Materialized, pot: u32, eff: u32) -> TreeBuild {
    let tree = EffectiveTree { rules_version: RULES_VERSION, template_id: template.id.to_string(), root_street: template.root_street, menus: template.menus.clone(),
        add_allin_threshold: template.add_allin_threshold, force_allin_threshold: template.force_allin_threshold, merging_threshold: template.merging_threshold,
        wager_cap: template.wager_cap, inserted: m.inserted, materialized: m.nodes };
    TreeBuild { tree, history: m.history, decision_path: m.decision_path, pot, eff }
}

/// Materializes a template at explicit chips with an actor-labelled prefix (0 = oop, 1 = ip); used by tests, `bench` and the fixture generator.
pub fn materialize_at(template: &TemplateSpec, pot: u32, eff: u32, prefix: &[(usize, Action)]) -> Result<TreeBuild, UnsupportedReason> {
    let m = materialize(&MaterializeInput { template, starting_pot: pot, eff, prefix })?;
    Ok(assemble(template, m, pot, eff))
}

/// §10.2: root inputs come from the snapshot only; `pot = pot_root + dead_this_street`, `eff = min(stacks)`; the history is
/// walked from the root and every off-menu wager is inserted exactly alongside the menu.
pub fn build_tree_full(root: &StreetRootSnapshot, sel: &TemplateSelection) -> Result<TreeBuild, UnsupportedReason> {
    let template = Templates::get(&sel.template_id).ok_or_else(|| engine_error(format!("unknown template {}", sel.template_id)))?;
    if template.root_street != root.street { return Err(engine_error(format!("template {} is rooted at {:?}, street is {:?}", template.id, template.root_street, root.street))); }
    let pot = root.pot_root.checked_add(root.dead_this_street).ok_or(UnsupportedReason::EngineError { message: "pot overflow".into(), retryable: false })?;
    let eff = root.stack_oop_root.min(root.stack_ip_root);
    let mut prefix = Vec::with_capacity(sel.observed.len());
    for (seat, a) in &sel.observed {
        let actor = if *seat == root.oop { 0 } else if *seat == root.ip { 1 } else { return Err(UnsupportedReason::UnsupportedHistory { reason: format!("seat {} is not a street-root player", seat.0) }); };
        prefix.push((actor, a.clone()));
    }
    let m = materialize(&MaterializeInput { template, starting_pot: pot, eff, prefix: &prefix })?;
    Ok(assemble(template, m, pot, eff))
}

pub fn build_effective_tree(root: &StreetRootSnapshot, sel: &TemplateSelection) -> Result<EffectiveTree, UnsupportedReason> {
    build_tree_full(root, sel).map(|b| b.tree)
}
```

- [ ] **Step 4: Run, hand-check, record the golden**

Run: `cargo test -p engine --test tree_builder` then `cargo test --workspace --release`
Expected: PASS; the first run writes `crates/engine/tests/golden/tree_builder_golden.json`. Hand-check it against §4.5: the `river_oracle` entry must be exactly the three nodes of the wire example (`[]` oop `[check]` `[null]`; `[0]` ip `[check, allin 100]` `[100, null]`; `[0,1]` oop `[fold, call]` `[100, 300]`), and in `flop_fast_73` the root must be `[check, bet 50, bet 73]` with `[null, null, null]` and the node `[2]` (ip facing 73: `to_call = 73`, `pot = 246`, `max = 500`, `min = 146`) must be `[fold, call, raise 183]` with terminal pots `[100, null, null]`: `round(2.5 * 73) = 183` (182.5 rounds away from zero), the force test `500 <= 183 + round((246 + 220) * 0.15) = 253` fails and the add test `500 <= 73 + round(246 * 1.0) = 319` fails, so no all-in is listed. Fix the code, not the file, if either differs; then commit the file.

- [ ] **Step 5: Commit**

```bash
git add crates/engine
git commit -m "feat(engine): effective tree with exact insertion, tree signature and chip-path resolution

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 5: `bench` crate skeleton: spot format, `gen-spots`, `materialize`

**Files:**
- Create: `crates/bench/Cargo.toml`, `crates/bench/src/lib.rs`, `crates/bench/src/main.rs`, `crates/bench/src/suite.rs`, `crates/bench/src/gen_spots.rs`, `crates/bench/src/materialize.rs`, `crates/engine/src/bench_support.rs`
- Modify: `crates/engine/src/lib.rs` (add `pub mod bench_support;`)
- Test: unit tests in `suite.rs`, `gen_spots.rs` and `bench_support.rs`; CLI smoke in step 6

**Interfaces:**
- Consumes: `engine::tree::{materialize_at, Templates}`, `engine::bench_support::prepared_range`; `proto::{Action, Card, Rake, Range1326, Street}`.
- Produces: `engine::bench_support::{prepared_range(text: &str, board: &[Card]) -> Result<Range1326, String>, range_mass(&Range1326) -> f32}` — the **only** way `bench` reaches range parsing and board blocking, because spec §3.2 fixes `bench`'s dependencies to `proto` and `engine` (cross-plan M17/R5). `bench::suite::{Spot { id: String, template_id: String, root_street: Street, board: Vec<Card>, oop_range: String, ip_range: String, pot: u32, stack_oop: u32, stack_ip: u32, rake: Rake, history: Vec<Action>, target_bp: u16, range_source: String }, Suite { suite: String, spots: Vec<Spot> }, Suite::load(&Path) -> Result<Suite, String>, Suite::save(&self, &Path)}`; `crates/bench/src/lib.rs` re-exporting `pub mod {suite, gen_spots, materialize}` so plan 4's integration tests can link them (cross-plan D4); CLI `bench materialize --template ID --pot P --eff E [--prefix oop:bet:73,ip:raise:200]` printing `{"tree": EffectiveTree, "history": [Action], "decision_path": [u8]}` to stdout; `bench gen-spots --source r8 --out bench/spots`.

**Range source (spec §13.5, cross-plan Or8/R3, orchestrator decision 4):** §13.5 defines the six baseline suites on **chart-replay** ranges. This task can only emit the R8 uniform ranges, so `--source r8` writes `range_source: "r8_uniform"` into every spot and the resulting suites are an explicitly labelled **interim** set: any `docs/bench/*.md` section produced from them is a pre-baseline reference run and does not satisfy the V2/V22 gate. `--source chart` is added when plan 3's chart bundles are wired into `bench gen-spots`, and plan 4 Task 17 regenerates all six `bench/spots/*.json` from chart replay before the gate is claimed. `generate` therefore rejects every source other than `r8` with a message naming `chart`, so the interim status cannot be forgotten.

- [ ] **Step 1: Failing tests**

`crates/bench/src/suite.rs` (tests only for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suite_roundtrip() {
        let s = Suite { suite: "river_std".into(), spots: vec![Spot { id: "srp100_dry".into(), template_id: "river_std_v1".into(), root_street: proto::Street::River,
            board: "Kh7d2c4d9s".as_bytes().chunks(2).map(|c| proto::Card::parse(std::str::from_utf8(c).unwrap()).unwrap()).collect(),
            oop_range: "AA".into(), ip_range: "KK".into(), pot: 241, stack_oop: 882, stack_ip: 882, rake: proto::Rake::TimeCharge, history: vec![], target_bp: 50, range_source: "r8_uniform".into() }] };
        let dir = std::env::temp_dir().join(format!("bench_suite_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("river_std.json");
        s.save(&p).unwrap();
        let back = Suite::load(&p).unwrap();
        assert_eq!(back.spots[0].board.len(), 5);
        assert_eq!(back.spots[0].id, "srp100_dry");
    }
}
```

`crates/bench/src/gen_spots.rs` (tests only for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn six_spots_per_suite_on_three_boards() {
        for suite in ["river_std", "river_min", "turn_std", "turn_min"] {
            let s = generate(suite, "r8").unwrap();
            assert_eq!(s.spots.len(), 6);
            let boards: std::collections::BTreeSet<String> = s.spots.iter().map(|x| x.board[..3].iter().map(|c| c.to_string()).collect::<String>()).collect();
            assert_eq!(boards.len(), 3);
            assert!(s.spots.iter().all(|x| x.template_id.starts_with(&suite[..suite.find('_').unwrap()])));
            assert!(s.spots.iter().all(|x| x.board.len() == if suite.starts_with("river") { 5 } else { 4 }));
            assert!(s.spots.iter().all(|x| x.range_source == "r8_uniform"), "the interim source must be labelled in every spot");
        }
        assert!(generate("flop_fast", "r8").is_err());
        // §13.5's chart-replay baseline is not available in this plan; the error names it so the gap stays explicit
        let e = generate("river_std", "chart").unwrap_err();
        assert!(e.contains("chart"), "{e}");
    }
}
```

`crates/engine/src/bench_support.rs` (tests only for now):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proto::Card;
    #[test]
    fn prepared_range_parses_and_blocks_the_board() {
        let board: Vec<Card> = ["Kh", "7d", "2c"].iter().map(|s| Card::parse(s).unwrap()).collect();
        let r = prepared_range("AA,KK", &board).unwrap();
        assert_eq!((range_mass(&r) * 1000.0).round() as u32, 9000);   // 6 aces + 3 kings (Kh is on the board)
        assert!(prepared_range("not a range", &board).is_err());
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p engine bench_support` then `cargo test -p bench`
Expected: FAIL to compile (`bench_support` and the `bench` crate do not exist).

- [ ] **Step 3: Implement the `engine::bench_support` facade**

`bench` may depend on `proto` and `engine` only (spec §3.2). Everything it needs from `core-ranges` goes through this one file. Add `pub mod bench_support;` to `crates/engine/src/lib.rs` and create `crates/engine/src/bench_support.rs` above its test module:

```rust
//! The façade `bench` uses instead of depending on `core-*` directly (spec §3.2 dependency direction).
//! Plan 4 extends this module; it never grows a second entry point.
use proto::{Card, Range1326};

/// Parses a Pio-style range string and applies board blocking, exactly as the engine does at a street root.
pub fn prepared_range(text: &str, board: &[Card]) -> Result<Range1326, String> {
    let mut r = core_ranges::parse_range(text).map_err(|e| format!("range {text:?}: {e}"))?;
    core_ranges::block_public(&mut r, board);
    Ok(r)
}
/// Total weight of a range, for report and sanity checks.
pub fn range_mass(r: &Range1326) -> f32 { core_ranges::mass(r) }
```

- [ ] **Step 4: Implement the `bench` crate**

`crates/bench/Cargo.toml` — note the absence of `features = ["testing"]`: feature unification would otherwise compile the fake clock and fake worker into every release build of the workspace, and `bench` uses no test double.

```toml
[package]
name = "bench"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
proto = { path = "../proto" }
engine = { path = "../engine" }
serde.workspace = true
serde_json.workspace = true
```

`crates/bench/src/lib.rs` (so plan 4's integration tests can link the modules; `main.rs` keeps its own `mod` lines out of the binary's way by using the library):

```rust
pub mod gen_spots;
pub mod materialize;
pub mod suite;
```

`crates/bench/src/suite.rs` (above the tests):

```rust
use proto::{Action, Card, Rake, Street};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Spot {
    pub id: String, pub template_id: String, pub root_street: Street, pub board: Vec<Card>,
    pub oop_range: String, pub ip_range: String, pub pot: u32, pub stack_oop: u32, pub stack_ip: u32,
    pub rake: Rake, pub history: Vec<Action>, pub target_bp: u16,
    /// "r8_uniform" (this plan's labelled interim) | "chart_replay" (the §13.5 baseline, plan 3 bundles + plan 4 Task 17) | "store_replay" (V9)
    pub range_source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Suite { pub suite: String, pub spots: Vec<Spot> }

impl Suite {
    pub fn load(path: &Path) -> Result<Suite, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir).map_err(|e| e.to_string())?; }
        std::fs::write(path, serde_json::to_string_pretty(self).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
    }
}
```

`crates/bench/src/gen_spots.rs` (above the tests):

```rust
use crate::suite::{Spot, Suite};
use proto::{Card, Rake, Street};

// R8 addendum A.1 ranges (uniform weights). INTERIM: spec §13.5's baseline set uses chart-replay ranges,
// which arrive with plan 3's bundles (`--source chart`) and are regenerated for all six suites by plan 4 Task 17.
// Note: A.1 publishes combo counts 646 / 804 for these two strings; the strings as published expand to 634 / 720
// (recomputed in Task 6). The strings, not A.1's counts, are the definition here and in the fixture generator.
pub const BTN_OPEN: &str = "22+,A2s+,K2s+,Q2s+,J3s+,T6s+,96s+,86s+,75s+,65s,54s,43s,A2o+,K7o+,Q8o+,J8o+,T8o+,98o";
pub const BB_DEFEND: &str = "JJ-22,AJs-A2s,K2s+,Q2s+,J2s+,T2s+,92s+,84s+,74s+,63s+,53s+,43s,32s,AJo-A2o,K5o+,Q7o+,J8o+,T8o+,98o,97o,87o,76o";
pub const CO_CALL_3BET: &str = "QQ-22,AKs-ATs,A5s-A4s,KQs-KTs,QJs-QTs,JTs,J9s,T9s,T8s,98s,87s,76s,65s,54s,AQo-AJo,KQo,KJo";
pub const BTN_3BET: &str = "TT+,AJs+,A5s-A2s,KJs+,QJs,JTs,T9s,76s,65s,54s,AQo+,KQo,KJo";
/// Board classes of addendum A.1 with fixed turn and river cards; chips are 0.1 bb (bb_chips = 10) as in the addendum.
const BOARDS: [(&str, &str); 3] = [("dry", "Kh7d2c4d9s"), ("wet", "Jh9h6c2d5s"), ("paired", "8s8d3cQh4c")];

fn cards(s: &str, n: usize) -> Vec<Card> { s.as_bytes().chunks(2).take(n).map(|c| Card::parse(std::str::from_utf8(c).unwrap()).unwrap()).collect() }

pub fn generate(suite: &str, source: &str) -> Result<Suite, String> {
    // §13.5's baseline set is chart-replay; `r8` is the labelled interim of this plan. `chart` is accepted only
    // once plan 3's bundles are wired in, and plan 4 Task 17 regenerates all six suites before the gate is claimed.
    if source != "r8" {
        return Err(format!("source {source:?} is not available in this plan: only \"r8\" (uniform R8 addendum A.1 ranges, interim) is implemented; the \"chart\" replay baseline of spec section 13.5 arrives with plan 3's bundles and is regenerated for all six suites by plan 4"));
    }
    let (street, template, n) = match suite {
        "river_std" => (Street::River, "river_std_v1", 5), "river_min" => (Street::River, "river_min_v1", 5),
        "turn_std" => (Street::Turn, "turn_std_v1", 4), "turn_min" => (Street::Turn, "turn_min_v1", 4),
        _ => return Err(format!("suite {suite} is not a river/turn suite of this plan")),
    };
    // SRP BTN-vs-BB at 100bb and 200bb after a 3 bb flop c-bet is called (turn: pot 11.5 bb) and a 55% turn bet called (river: pot 24.1 bb)
    let depths = [(100u32, "srp100"), (200u32, "srp200")];
    let mut spots = Vec::new();
    for (bb, tag) in depths {
        for (class, board) in BOARDS {
            let (pot, stack) = match street { Street::Turn => (115, bb * 10 - 55), _ => (241, bb * 10 - 118) };
            spots.push(Spot { id: format!("{tag}_{class}"), template_id: template.into(), root_street: street, board: cards(board, n),
                oop_range: BB_DEFEND.into(), ip_range: BTN_OPEN.into(), pot, stack_oop: stack, stack_ip: stack,
                rake: Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false }, history: vec![], target_bp: 50, range_source: "r8_uniform".into() });
        }
    }
    Ok(Suite { suite: suite.into(), spots })
}
```

`crates/bench/src/materialize.rs`:

```rust
use engine::tree::{materialize_at, Templates};
use proto::Action;

/// `--prefix oop:bet:73,ip:raise:200,oop:call` -> actor-labelled prefix.
pub fn parse_prefix(s: &str) -> Result<Vec<(usize, Action)>, String> {
    if s.trim().is_empty() { return Ok(vec![]); }
    s.split(',').map(|item| {
        let parts: Vec<&str> = item.trim().split(':').collect();
        let actor = match parts.first().copied() { Some("oop") => 0, Some("ip") => 1, _ => return Err(format!("bad actor in {item}")) };
        let to = |i: usize| parts.get(i).ok_or(format!("missing amount in {item}"))?.parse::<u32>().map_err(|e| e.to_string());
        let action = match parts.get(1).copied() {
            Some("check") => Action::Check, Some("call") => Action::Call, Some("fold") => Action::Fold,
            Some("bet") => Action::Bet { to: to(2)? }, Some("raise") => Action::Raise { to: to(2)? }, Some("allin") => Action::AllIn { to: to(2)? },
            _ => return Err(format!("bad action in {item}")),
        };
        Ok((actor, action))
    }).collect()
}

pub fn run(template: &str, pot: u32, eff: u32, prefix: &str) -> Result<String, String> {
    let t = Templates::get(template).ok_or(format!("unknown template {template}"))?;
    let b = materialize_at(t, pot, eff, &parse_prefix(prefix)?).map_err(|e| format!("{e:?}"))?;
    serde_json::to_string(&serde_json::json!({ "tree": b.tree, "history": b.history, "decision_path": b.decision_path })).map_err(|e| e.to_string())
}
```

`crates/bench/src/main.rs`:

```rust
use bench::{gen_spots, materialize, suite};

fn arg(args: &[String], name: &str) -> Option<String> { args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned()) }

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code = match args.get(1).map(String::as_str) {
        Some("materialize") => {
            let r = materialize::run(&arg(&args, "--template").unwrap_or_default(), arg(&args, "--pot").and_then(|v| v.parse().ok()).unwrap_or(0),
                arg(&args, "--eff").and_then(|v| v.parse().ok()).unwrap_or(0), &arg(&args, "--prefix").unwrap_or_default());
            match r { Ok(json) => { println!("{json}"); 0 } Err(e) => { eprintln!("{e}"); 2 } }
        }
        Some("gen-spots") => {
            let out = std::path::PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "bench/spots".into()));
            let source = arg(&args, "--source").unwrap_or_else(|| "r8".into());
            let mut code = 0;
            for suite in ["river_std", "river_min", "turn_std", "turn_min"] {
                match gen_spots::generate(suite, &source).and_then(|s| s.save(&out.join(format!("{suite}.json")))) { Ok(()) => println!("wrote {suite}"), Err(e) => { eprintln!("{e}"); code = 2 } }
            }
            code
        }
        _ => { eprintln!("usage: bench materialize --template ID --pot P --eff E [--prefix ...] | bench gen-spots [--source r8] [--out DIR] | bench run ... (Task 30)"); 1 }
    };
    std::process::exit(code);
}
```

- [ ] **Step 5: Run tests**

Run: `cargo test -p bench` and `cargo test -p engine bench_support`
Expected: 2 passed in `bench`, 1 passed in `engine`.

- [ ] **Step 6: CLI smoke and spot files**

Run: `cargo run -q -p bench -- materialize --template river_oracle_v1 --pot 100 --eff 100 --prefix oop:check`
Expected: one JSON line whose `tree.materialized` has 3 nodes and `decision_path` is `[0]`.

Run: `cargo run -q -p bench -- gen-spots --source r8 --out bench/spots`
Expected: `bench/spots/{river_std,river_min,turn_std,turn_min}.json` written, 6 spots each, every spot's `range_source` `"r8_uniform"`.

- [ ] **Step 7: Commit**

Run: `cargo test --workspace --release`
Expected: green.

```bash
git add crates/bench crates/engine bench/spots
git commit -m "feat(bench): spot format, interim r8 spot generator, materialize subcommand and the engine bench_support facade

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 6: `tools/gen_worker_fixtures.py` and the `fixtures/worker/*.jsonl` files

**Files:**
- Create: `tools/gen_worker_fixtures.py`, `tools/tests/test_gen_worker_fixtures.py`, `fixtures/worker/{river_two_combo,flop_cancel,flop_best_so_far,lock_river,materialization_cases}.jsonl` (generated)
- Modify: `tools/pyproject.toml` (plan 1) only if `pytest` is not already a dev dependency

**Interfaces:**
- Consumes: `bench materialize` (Task 5) through `subprocess`; the §4.5 definitions.
- Produces: the five JSONL fixtures. Each fixture holds engine -> worker lines only; expectations live in the Rust tests. Line ids: `river_two_combo`: `solve 41`, `cancel 42 -> 41`, `shutdown 48`; `flop_cancel`: `solve 43`, `cancel 44 -> 43`; `flop_best_so_far`: `solve 45`; `lock_river`: `lock 47`, `solve 51` (same `spot`), `shutdown 52`; `materialization_cases`: one object per case `{"case", "template_id", "pot", "eff", "prefix": [[actor, action]], "tree", "history", "decision_path"}` (43 cases).
- This generator is the **single definition** of the two-combo river ranges (cross-plan D7): plan 1's `crates/proto/tests/wire_examples.rs` loads `fixtures/worker/river_two_combo.jsonl` rather than restating them in Rust, so the two can never drift. Nothing here re-derives them.

- [ ] **Step 1: Failing Python tests `tools/tests/test_gen_worker_fixtures.py`**

```python
import json
import gen_worker_fixtures as g


def test_combo_index_and_cards():
    assert g.card_id("2c") == 0 and g.card_id("As") == 51 and g.card_id("Qs") == 43
    assert g.combo_index(0, 1) == 0 and g.combo_index(51, 50) == 1325
    assert g.combo_index(g.card_id("Ah"), g.card_id("Ad")) == g.combo_index(g.card_id("Ad"), g.card_id("Ah"))


def test_parse_range_counts():
    assert sum(1 for w in g.parse_range("AA") if w > 0) == 6
    assert sum(1 for w in g.parse_range("AKs") if w > 0) == 4
    assert sum(1 for w in g.parse_range("54o:0.25") if w > 0) == 12
    assert abs(sum(g.parse_range("54o:0.25")) - 3.0) < 1e-9
    assert sum(1 for w in g.parse_range("96s+") if w > 0) == 12          # 96s, 97s, 98s
    assert sum(1 for w in g.parse_range("QQ-88") if w > 0) == 30         # five pairs
    assert sum(1 for w in g.parse_range("98o-65o") if w > 0) == 48       # four offsuit classes
    # Recomputed from the strings themselves. R8 addendum A.1 publishes 646 / 804 for these two ranges, but the
    # strings as published expand to 634 / 720 under this grammar (CO_CALL_3BET 194 and BTN_3BET 138 match A.1
    # exactly, so the parser is right and A.1's two figures are not reproducible from its own strings).
    # The strings are the definition; `crates/bench/src/gen_spots.rs` freezes the same two constants.
    assert sum(1 for w in g.parse_range(g.BTN_OPEN) if w > 0) == 634
    assert sum(1 for w in g.parse_range(g.BB_DEFEND) if w > 0) == 720
    assert sum(1 for w in g.parse_range(g.CO_CALL_3BET) if w > 0) == 194
    assert sum(1 for w in g.parse_range(g.BTN_3BET) if w > 0) == 138


def test_river_two_combo_definition():
    lines = [json.loads(l) for l in g.river_two_combo_lines()]
    solve = lines[0]
    assert solve["type"] == "solve" and solve["id"] == "41" and solve["board"] == ["Qs", "Jd", "7h", "3c", "2d"]
    oop, ip = solve["oop_range"], solve["ip_range"]
    assert len(oop) == 1326 and len(ip) == 1326
    assert sum(1 for w in oop if w == 1.0) == 6 and sum(oop) == 6.0
    assert sum(1 for w in ip if w == 1.0) == 3 and sum(1 for w in ip if w == 0.25) == 12
    assert solve["tree"]["template_id"] == "river_oracle_v1" and len(solve["tree"]["materialized"]) == 3
    assert solve["history"] == [{"kind": "check"}] and solve["memory_limit_bytes"] == 10737418240
    assert lines[1] == {"type": "cancel", "id": "42", "target": "41"} and lines[2] == {"type": "shutdown", "id": "48"}


def test_write_all_with_stub_materializer(tmp_path):
    def stub(template, pot, eff, prefix):
        return {"tree": {"template_id": template, "materialized": []}, "history": [], "decision_path": []}
    written = g.write_all(tmp_path, materializer=stub)
    names = sorted(p.name for p in written)
    assert names == ["flop_best_so_far.jsonl", "flop_cancel.jsonl", "lock_river.jsonl", "materialization_cases.jsonl", "river_two_combo.jsonl"]
    cases = [json.loads(l) for l in (tmp_path / "materialization_cases.jsonl").read_text().splitlines()]
    #        6 phase-1 templates x 5 points, 1 flop_full point, 7 facing stacks, facing_350_full, cap1, cap3, insert_73, basic_turn_std
    assert len(cases) == 6 * 5 + 1 + 7 + 1 + 1 + 1 + 1 + 1 == 43
    lock = [json.loads(l) for l in (tmp_path / "lock_river.jsonl").read_text().splitlines()]
    assert lock[0]["type"] == "lock" and lock[1]["type"] == "solve" and lock[0]["spot"] == lock[1]["spot"]
    rows = lock[0]["locks"][0]["probs"]
    assert len(rows) == 1326 and all(len(r) == 2 for r in rows)
    assert sum(1 for r in rows if r == [0.0, 1.0]) == 3 and sum(1 for r in rows if r == [0.8, 0.2]) == 12
```

- [ ] **Step 2: Run to see them fail**

Run: `cd tools && python -m pytest tests/test_gen_worker_fixtures.py -q`
Expected: FAIL (`gen_worker_fixtures` not importable).

- [ ] **Step 3: Write `tools/gen_worker_fixtures.py`**

```python
#!/usr/bin/env python
"""Generates fixtures/worker/*.jsonl from the definitions of spec sections 4.5 and 13.2.

Card id = rank * 4 + suit (ranks 2..A = 0..12, suits c,d,h,s = 0..3); combo index = hi*(hi-1)/2 + lo (spec 4.1).
The flop trees come from `bench materialize` (engine materializer); the river oracle tree is written by hand
exactly as the section 4.5 wire example.
"""
import json
import pathlib
import subprocess
import sys

RANKS = "23456789TJQKA"
SUITS = "cdhs"
GIB = 1024 ** 3
SPOT_RIVER = "3f9c0a7d2b1e4c6f8a9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f"
SPOT_FLOP = "5a1c9e3b7d2f4a6c8e0b1d3f5a7c9e2b4d6f8a0c1e3b5d7f9a2c4e6b8d0f1a3c"
SPOT_LOCK = "9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f3f9c0a7d2b1e4c6f8a"
# R8 addendum A.1 range strings, copied verbatim; the same four constants are frozen in crates/bench/src/gen_spots.rs.
BTN_OPEN = "22+,A2s+,K2s+,Q2s+,J3s+,T6s+,96s+,86s+,75s+,65s,54s,43s,A2o+,K7o+,Q8o+,J8o+,T8o+,98o"
BB_DEFEND = "JJ-22,AJs-A2s,K2s+,Q2s+,J2s+,T2s+,92s+,84s+,74s+,63s+,53s+,43s,32s,AJo-A2o,K5o+,Q7o+,J8o+,T8o+,98o,97o,87o,76o"
CO_CALL_3BET = "QQ-22,AKs-ATs,A5s-A4s,KQs-KTs,QJs-QTs,JTs,J9s,T9s,T8s,98s,87s,76s,65s,54s,AQo-AJo,KQo,KJo"
BTN_3BET = "TT+,AJs+,A5s-A2s,KJs+,QJs,JTs,T9s,76s,65s,54s,AQo+,KQo,KJo"
BASIC_OOP = "66+,A8s+,A5s-A4s,AJo+,K9s+,KQo,QTs+,JTs,96s+,85s+,75s+,65s,54s"
BASIC_IP = "QQ-22,AQs-A2s,ATo+,K5s+,KJo+,Q8s+,J8s+,T7s+,96s+,86s+,75s+,64s+,53s+"


def card_id(s):
    return RANKS.index(s[0]) * 4 + SUITS.index(s[1])


def combo_index(a, b):
    lo, hi = sorted((a, b))
    return hi * (hi - 1) // 2 + lo


def class_combos(r1, r2, suited):
    """Card-id pairs of a 169-class: r1 == r2 pair; suited True/False/None (None = both)."""
    out = []
    if r1 == r2:
        return [(r1 * 4 + s, r1 * 4 + t) for s in range(4) for t in range(s + 1, 4)]
    for s in range(4):
        for t in range(4):
            if suited is True and s != t or suited is False and s == t:
                continue
            out.append((r1 * 4 + s, r2 * 4 + t))
    return out


def parse_token(tok):
    """One Pio group -> list of (r1, r2, suited); grammar of the postflop-solver Range doc comment."""
    if len(tok) == 4 and tok[1] in SUITS and tok[3] in SUITS:          # specific combo "AsKs"
        return [("specific", card_id(tok[:2]), card_id(tok[2:]))]
    plus = tok.endswith("+")
    body = tok[:-1] if plus else tok
    if "-" in body:
        hi, lo = body.split("-")
        r1, r2 = RANKS.index(hi[0]), RANKS.index(hi[1])
        l1, l2 = RANKS.index(lo[0]), RANKS.index(lo[1])
        suited = None if len(hi) == 2 else hi[2] == "s"
        if r1 == r2:                                                    # QQ-88
            return [(r, r, None) for r in range(l2, r1 + 1)]
        if r1 == l1:                                                    # A9s-A6s
            return [(r1, r, suited) for r in range(l2, r2 + 1)]
        return [(r1 - k, r2 - k, suited) for k in range(0, r1 - l1 + 1)]  # 98o-65o
    r1, r2 = RANKS.index(body[0]), RANKS.index(body[1])
    suited = None if len(body) == 2 else body[2] == "s"
    if not plus:
        return [(r1, r2, suited)]
    if r1 == r2:
        return [(r, r, None) for r in range(r1, 13)]
    return [(r1, r, suited) for r in range(r2, r1)]


def parse_range(text):
    vec = [0.0] * 1326
    for group in (g.strip() for g in text.split(",") if g.strip()):
        tok, weight = (group.split(":") + ["1"])[:2]
        w = float(weight)
        for entry in parse_token(tok):
            if entry[0] == "specific":
                vec[combo_index(entry[1], entry[2])] = w
                continue
            r1, r2, suited = entry
            for a, b in class_combos(r1, r2, suited):
                vec[combo_index(a, b)] = w
    return vec


def block(vec, board):
    dead = {card_id(c) for c in board}
    out = list(vec)
    for hi in range(1, 52):
        for lo in range(hi):
            if hi in dead or lo in dead:
                out[combo_index(lo, hi)] = 0.0
    return out


def action(kind, to=None):
    return {"kind": kind} if to is None else {"kind": kind, "to": to}


RIVER_ORACLE_TREE = {
    "rules_version": 3, "template_id": "river_oracle_v1", "root_street": "river",
    "menus": {"river": {"oop": {"bet": [], "raise": []}, "ip": {"bet": [1.0], "raise": []}}},
    "add_allin_threshold": 0.0, "force_allin_threshold": 0.0, "merging_threshold": 0.0, "wager_cap": 1, "inserted": [],
    "materialized": [
        {"path": [], "street": "river", "actor": "oop", "actions": [action("check")], "terminal_pots": [None]},
        {"path": [0], "street": "river", "actor": "ip", "actions": [action("check"), action("allin", 100)], "terminal_pots": [100, None]},
        {"path": [0, 1], "street": "river", "actor": "oop", "actions": [action("fold"), action("call")], "terminal_pots": [100, 300]},
    ],
}
RIVER_BOARD = ["Qs", "Jd", "7h", "3c", "2d"]


def solve_line(id_, spot, board, oop, ip, pot, stack, tree, history, target_bp, deadline_ms, margin_ms, rake=(0.0, 0)):
    return json.dumps({"type": "solve", "id": id_, "spot": spot, "board": board, "oop_range": block(oop, board), "ip_range": block(ip, board),
                       "pot": pot, "stack_oop": stack, "stack_ip": stack, "rake_rate": rake[0], "rake_cap_mchips": rake[1], "tree": tree,
                       "history": history, "target_bp": target_bp, "deadline_ms": deadline_ms, "extraction_margin_ms": margin_ms,
                       "memory_limit_bytes": 10 * GIB, "background": False}, separators=(",", ":"))


def river_two_combo_lines():
    yield solve_line("41", SPOT_RIVER, RIVER_BOARD, parse_range("AA"), parse_range("QQ,54o:0.25"), 100, 100, RIVER_ORACLE_TREE, [action("check")], 10, 1500, 200)
    yield json.dumps({"type": "cancel", "id": "42", "target": "41"}, separators=(",", ":"))
    yield json.dumps({"type": "shutdown", "id": "48"}, separators=(",", ":"))


def lock_river_lines():
    """IP's river node locked to bet QQ 100% and 54o 20% (ev_convention_non_root_payoffs); OOP holds AA and 66."""
    rows = [[0.0, 0.0] for _ in range(1326)]
    for i, w in enumerate(block(parse_range("QQ"), RIVER_BOARD)):
        if w > 0:
            rows[i] = [0.0, 1.0]
    for i, w in enumerate(block(parse_range("54o"), RIVER_BOARD)):
        if w > 0:
            rows[i] = [0.8, 0.2]
    yield json.dumps({"type": "lock", "id": "47", "spot": SPOT_LOCK, "locks": [{"path": [action("check")], "actor": "ip", "probs": rows}]}, separators=(",", ":"))
    yield solve_line("51", SPOT_LOCK, RIVER_BOARD, parse_range("AA,66"), parse_range("QQ,54o:0.25"), 100, 100, RIVER_ORACLE_TREE, [action("check")], 10, 1500, 200)
    yield json.dumps({"type": "shutdown", "id": "52"}, separators=(",", ":"))


def bench_materialize(template, pot, eff, prefix):
    """Runs `bench materialize`; prefix is a list of [actor, action] pairs (actor 'oop'|'ip')."""
    items = ",".join(f"{a}:{x['kind']}" + (f":{x['to']}" if "to" in x else "") for a, x in prefix)
    cmd = ["cargo", "run", "-q", "--release", "-p", "bench", "--", "materialize", "--template", template, "--pot", str(pot), "--eff", str(eff), "--prefix", items]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=pathlib.Path(__file__).resolve().parents[1]).stdout
    return json.loads(out)


FLOP_BOARD = ["Qs", "Jh", "2h"]


def flop_lines(materializer):
    m = materializer("flop_fast_v1", 180, 910, [])
    oop, ip = parse_range(BASIC_OOP), parse_range(BASIC_IP)
    cancel = [solve_line("43", SPOT_FLOP, FLOP_BOARD, oop, ip, 180, 910, m["tree"], m["history"], 50, 30000, 600),
              json.dumps({"type": "cancel", "id": "44", "target": "43"}, separators=(",", ":"))]
    best = [solve_line("45", SPOT_FLOP, FLOP_BOARD, oop, ip, 180, 910, m["tree"], m["history"], 1, 2000, 600)]
    return cancel, best


# flop_full_v1 is phase-2 only (spec section 10.1) and its (180, 910) skeleton is roughly 3,000 nodes, so it gets a
# single small rules case instead of the five-point sweep the phase-1 templates get.
TEMPLATES = ["flop_fast_v1", "flop_min_v1", "turn_std_v1", "turn_min_v1", "river_std_v1", "river_min_v1"]


def materialization_cases(materializer):
    cases = []
    for t in TEMPLATES:
        for pot, eff in [(100, 100), (100, 150), (180, 910), (100, 149), (100, 151)]:
            cases.append((f"{t}_{pot}_{eff}", t, pot, eff, []))
    cases.append(("flop_full_v1_100_100", "flop_full_v1", 100, 100, []))
    for eff in [350, 400, 401, 340, 341, 240, 100]:
        cases.append((f"facing_{eff}", "facing_test_v1", 100, eff, [["oop", action("bet", 100)]]))
    cases.append(("facing_350_full", "facing_test_v1", 100, 350, []))
    cases.append(("cap1_two_wagers", "flop_min_v1", 100, 500, [["oop", action("bet", 40)], ["ip", action("raise", 120)]]))
    cases.append(("cap3_three_wagers", "turn_std_v1", 100, 1000, [["oop", action("bet", 33)], ["ip", action("raise", 83)], ["oop", action("raise", 208)]]))
    cases.append(("insert_73", "flop_fast_v1", 100, 500, [["oop", action("bet", 73)]]))
    cases.append(("basic_turn_std", "turn_std_v1", 200, 900, []))      # the pinned example spot of Task 12
    for case, t, pot, eff, prefix in cases:
        m = materializer(t, pot, eff, prefix)
        yield json.dumps({"case": case, "template_id": t, "pot": pot, "eff": eff, "prefix": prefix, "tree": m["tree"], "history": m["history"], "decision_path": m["decision_path"]}, separators=(",", ":"))


def write_all(out_dir, materializer=bench_materialize):
    out_dir = pathlib.Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    cancel, best = flop_lines(materializer)
    files = {"river_two_combo.jsonl": list(river_two_combo_lines()), "lock_river.jsonl": list(lock_river_lines()),
             "flop_cancel.jsonl": cancel, "flop_best_so_far.jsonl": best, "materialization_cases.jsonl": list(materialization_cases(materializer))}
    written = []
    for name, lines in files.items():
        p = out_dir / name
        p.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        written.append(p)
    return written


if __name__ == "__main__":
    target = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "worker"
    for p in write_all(target):
        print(f"wrote {p} ({p.stat().st_size} bytes)")
```

- [ ] **Step 4: Run the tests, then generate the fixtures**

Run: `cd tools && python -m pytest tests/test_gen_worker_fixtures.py -q`
Expected: 4 passed.

Run (repo root, needs Task 5's `bench`): `python tools/gen_worker_fixtures.py`
Expected: five files under `fixtures/worker/`; `river_two_combo.jsonl` is three lines; `materialization_cases.jsonl` has 43 lines; the `flop_*` solve lines carry a `flop_fast_v1` tree rooted at `flop` with `history: []`.

- [ ] **Step 5: Commit**

Run: `cargo test --workspace --release`
Expected: green (no Rust code changed; the fixtures are consumed from Task 8 on).

```bash
git add tools/gen_worker_fixtures.py tools/tests/test_gen_worker_fixtures.py fixtures/worker
git commit -m "feat(tools): worker wire fixture generator and the section 4.5 fixtures

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 7: `solver-worker` skeleton: AVX2 build check, card mapping, `ready`, EOF exit

**Files:**
- Create: `solver-worker/Cargo.toml`, `solver-worker/build.rs`, `solver-worker/src/lib.rs`, `solver-worker/src/cards.rs`, `solver-worker/src/win.rs`, `solver-worker/src/main.rs`, `solver-worker/tests/common/mod.rs`, `solver-worker/tests/startup.rs`
- Test: `solver-worker/tests/startup.rs::ready_reports_features`, unit tests in `cards.rs`

**Interfaces:**
- Consumes: `proto::worker::{WorkerMessage, Ready, PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}` (all three constants are produced by plan 1 Task 7; this crate defines none of them, cross-plan M5), `proto::{Card, Range1326, ComboIndex, combo_index, combo_cards}`, `postflop_solver::{Card as LibCard, Range as LibRange, NOT_DEALT}`.
- Produces: `solver_worker::{CAPABILITIES, build_features(), cpu_features(), ready_message(threads: u8) -> WorkerMessage}` and the re-export `pub use proto::worker::SOLVER_COMMIT;`; `cards::{to_lib(Card) -> LibCard, from_lib(LibCard) -> Card, range_to_lib(&Range1326) -> Result<LibRange, String>, lib_hand_to_combo((LibCard, LibCard)) -> ComboIndex, board_to_lib(&[Card]) -> Result<([LibCard; 3], LibCard, LibCard), String>}`; `win::set_priority_class(below_normal: bool)`; test helper `common::Worker::{spawn(threads) -> Worker, send(&mut self, &str), recv(&self, Duration) -> Option<serde_json::Value>, recv_until(&self, Duration, impl Fn(&Value) -> bool) -> Option<Value>, close_stdin(&mut self), wait_exit(&mut self, Duration) -> Option<i32>, kill(&mut self)}`, `common::fixture_lines(name: &str) -> Vec<String>`.
- No crate depends on `solver-worker` (spec §3.2): the engine reads `SOLVER_COMMIT` from `proto::worker`, never from here.

- [ ] **Step 1: Failing integration test `solver-worker/tests/startup.rs`**

```rust
mod common;
use common::Worker;
use std::time::Duration;

#[test]
fn ready_reports_features() {
    let mut w = Worker::spawn(4);
    let ready = w.recv(Duration::from_secs(5)).expect("ready within 5 s");
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["proto_version"], 3);
    assert_eq!(ready["adapter_version"], 1);
    assert_eq!(ready["threads"], 4);
    assert_eq!(ready["solver_commit"], "9d1509fe5077d019825f833eed04b16d342dfda1");
    assert!(ready["build_features"].as_array().unwrap().iter().any(|f| f == "avx2"));
    for cap in ["solve", "lock", "cancel", "street_export", "i16"] {
        assert!(ready["capabilities"].as_array().unwrap().iter().any(|c| c == cap), "missing capability {cap}");
    }
    // stdin EOF behaves like shutdown without the ack: exit 0 within 2 s
    w.close_stdin();
    assert_eq!(w.wait_exit(Duration::from_secs(2)), Some(0));
}
```

`solver-worker/tests/common/mod.rs`:

```rust
#![allow(dead_code)]
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub struct Worker { child: Child, stdin: Option<ChildStdin>, lines: Receiver<String> }

impl Worker {
    pub fn spawn(threads: u8) -> Worker {
        let mut child = Command::new(env!("CARGO_BIN_EXE_solver-worker"))
            .args(["--threads", &threads.to_string()])
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .spawn().expect("spawn solver-worker");
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || { for line in BufReader::new(stdout).lines() { match line { Ok(l) => { if tx.send(l).is_err() { break; } } Err(_) => break } } });
        Worker { stdin: child.stdin.take(), child, lines: rx }
    }
    pub fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("stdin open");
        stdin.write_all(line.trim_end().as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
        stdin.flush().unwrap();
    }
    pub fn recv(&self, timeout: Duration) -> Option<Value> {
        match self.lines.recv_timeout(timeout) { Ok(l) => Some(serde_json::from_str(&l).unwrap_or_else(|e| panic!("non-JSON stdout line {l:?}: {e}"))), Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => None }
    }
    /// Skips messages until `pred` holds; returns None at the deadline.
    pub fn recv_until(&self, timeout: Duration, pred: impl Fn(&Value) -> bool) -> Option<Value> {
        let end = Instant::now() + timeout;
        loop {
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() { return None; }
            let v = self.recv(left)?;
            if pred(&v) { return Some(v); }
        }
    }
    pub fn close_stdin(&mut self) { self.stdin.take(); }
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<i32> {
        let end = Instant::now() + timeout;
        loop {
            if let Ok(Some(st)) = self.child.try_wait() { return st.code(); }
            if Instant::now() >= end { return None; }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn kill(&mut self) { let _ = self.child.kill(); let _ = self.child.wait(); }
}
impl Drop for Worker { fn drop(&mut self) { if self.child.try_wait().ok().flatten().is_none() { self.kill(); } } }

pub fn fixture_lines(name: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/worker").join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (run tools/gen_worker_fixtures.py)", path.display())).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p solver-worker --test startup`
Expected: FAIL (crate does not exist).

- [ ] **Step 3: Workspace member, crate, build check, library modules**

First add the member (it is outside `crates/`, so plan 1's glob does not cover it) and the debug profile override, in the root `Cargo.toml`:

```toml
[workspace]
members = ["crates/*", "solver-worker"]
exclude = ["third_party/postflop-solver"]

# A debug test build of the workspace still exercises optimized solver code; the per-task green command
# is `cargo test --workspace --release`, but an ad-hoc debug run must not take 30x as long.
[profile.dev.package.postflop-solver]
opt-level = 3
```

`solver-worker/Cargo.toml` — the one crate with an explicit `license` (AGPL boundary, spec §3.3); everything else comes from the workspace:

```toml
[package]
name = "solver-worker"
version.workspace = true
edition.workspace = true
license = "AGPL-3.0"
build = "build.rs"

[[bin]]
name = "solver-worker"
path = "src/main.rs"

[dependencies]
proto = { path = "../crates/proto" }
postflop-solver = { path = "../third_party/postflop-solver", features = ["zstd"] }
serde.workspace = true
serde_json.workspace = true
rayon = "1"
```

`solver-worker/build.rs`:

```rust
fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_FEATURE");
    let features = std::env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    if !features.split(',').any(|f| f == "avx2") {
        panic!("solver-worker requires AVX2 (spec 3.7): build with -C target-feature=+avx2 via .cargo/config.toml; CARGO_CFG_TARGET_FEATURE was {features:?}");
    }
}
```

`solver-worker/src/lib.rs`:

```rust
pub mod cards;
pub mod win;

use proto::worker::{Ready, WorkerMessage, ADAPTER_VERSION, PROTO_VERSION};

pub const CAPABILITIES: [&str; 5] = ["solve", "lock", "cancel", "street_export", "i16"];

/// The pinned commit is owned by `proto::worker` (plan 1 Task 7) because the engine validates `ready`
/// against it and may not depend on this crate (spec §3.2). Re-exported, never redefined.
pub use proto::worker::SOLVER_COMMIT;

/// What is actually vendored on disk, read at build time. A mismatch with `SOLVER_COMMIT` is a build error:
/// the `ready` message must never claim a commit the binary was not built from.
const VENDORED_COMMIT: &str = include_str!("../../third_party/postflop-solver/PINNED_COMMIT");
const _: () = {
    // `const` string comparison: same length and same bytes.
    let (a, b) = (VENDORED_COMMIT.as_bytes(), SOLVER_COMMIT.as_bytes());
    assert!(a.len() >= b.len(), "third_party/postflop-solver/PINNED_COMMIT is shorter than proto::worker::SOLVER_COMMIT");
    let mut i = 0;
    while i < b.len() { assert!(a[i] == b[i], "vendored commit differs from proto::worker::SOLVER_COMMIT"); i += 1; }
};

pub fn build_features() -> Vec<String> {
    let mut v = Vec::new();
    if cfg!(target_feature = "avx2") { v.push("avx2".to_string()); }
    if cfg!(target_feature = "fma") { v.push("fma".to_string()); }
    if cfg!(target_feature = "avx512f") { v.push("avx512f".to_string()); }
    v
}

pub fn cpu_features() -> Vec<String> {
    let mut v = Vec::new();
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") { v.push("avx2".to_string()); }
        if std::is_x86_feature_detected!("fma") { v.push("fma".to_string()); }
        if std::is_x86_feature_detected!("avx512f") { v.push("avx512f".to_string()); }
    }
    v
}

pub fn ready_message(threads: u8) -> WorkerMessage {
    WorkerMessage::Ready(Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.to_string(), adapter_version: ADAPTER_VERSION, threads,
        build_features: build_features(), cpu_features: cpu_features(), capabilities: CAPABILITIES.iter().map(|s| s.to_string()).collect() })
}
```

`solver-worker/src/cards.rs`:

```rust
//! Card and range mapping by named combos (§4.1: the adapter never assumes the library's ordering).
use postflop_solver::{Card as LibCard, Range as LibRange, NOT_DEALT};
use proto::{combo_cards, combo_index, Card, ComboIndex, Range1326};

/// Library encoding (card.rs): `4 * rank + suit`, ranks 2..A = 0..12, suits c, d, h, s = 0..3.
pub fn to_lib(c: Card) -> LibCard { (c.rank() << 2) | c.suit() }
pub fn from_lib(c: LibCard) -> Card { Card::new(c >> 2, c & 3) }

pub fn range_to_lib(r: &Range1326) -> Result<LibRange, String> {
    let mut out = LibRange::new();
    for (i, &w) in r.0.iter().enumerate() {
        if w == 0.0 { continue; }
        if !w.is_finite() || !(0.0..=1.0).contains(&w) { return Err(format!("weight {w} at combo {i} outside [0, 1]")); }
        let [lo, hi] = combo_cards(i as ComboIndex);
        out.set_weight_by_cards(to_lib(lo), to_lib(hi), w);
    }
    Ok(out)
}

pub fn lib_hand_to_combo(h: (LibCard, LibCard)) -> ComboIndex { combo_index(from_lib(h.0), from_lib(h.1)) }

/// Sorted flop (as `flop_from_str` does) plus turn and river or `NOT_DEALT`.
pub fn board_to_lib(board: &[Card]) -> Result<([LibCard; 3], LibCard, LibCard), String> {
    if !(3..=5).contains(&board.len()) { return Err(format!("board has {} cards", board.len())); }
    let mut ids: Vec<LibCard> = board.iter().map(|c| to_lib(*c)).collect();
    let mut sorted = ids.clone(); sorted.sort_unstable(); sorted.dedup();
    if sorted.len() != ids.len() { return Err("duplicate board card".into()); }
    let mut flop = [ids[0], ids[1], ids[2]]; flop.sort_unstable();
    let turn = if ids.len() > 3 { ids[3] } else { NOT_DEALT };
    let river = if ids.len() > 4 { ids[4] } else { NOT_DEALT };
    ids.clear();
    Ok((flop, turn, river))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_combo_roundtrip() {
        let as_ = Card::parse("As").unwrap(); let ks = Card::parse("Ks").unwrap();
        assert_eq!(to_lib(as_), 51); assert_eq!(to_lib(Card::parse("2c").unwrap()), 0);
        let mut r = Range1326([0.0; 1326]); r.0[combo_index(as_, ks) as usize] = 0.5;
        let lib = range_to_lib(&r).unwrap();
        assert_eq!(lib.get_weight_by_cards(to_lib(as_), to_lib(ks)), 0.5);
        let (hands, weights) = lib.get_hands_weights(0);
        assert_eq!((hands.len(), weights[0]), (1, 0.5));
        assert_eq!(lib_hand_to_combo(hands[0]), combo_index(as_, ks));
        assert!(range_to_lib(&Range1326([1.5; 1326])).is_err());
    }
}
```

`solver-worker/src/win.rs`:

```rust
//! §3.4: the worker runs at BELOW_NORMAL while a `background: true` job is solving, NORMAL otherwise.
//! The child's peak working set is measured by the ENGINE from the parent process
//! (`crates/engine/src/worker/process.rs`), so the worker exposes no memory query of its own.
#[cfg(windows)]
mod imp {
    #[link(name = "kernel32")]
    extern "system" { fn GetCurrentProcess() -> isize; fn SetPriorityClass(h: isize, class: u32) -> i32; }
    const NORMAL_PRIORITY_CLASS: u32 = 0x20;
    const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
    pub fn set_priority_class(below_normal: bool) {
        unsafe { SetPriorityClass(GetCurrentProcess(), if below_normal { BELOW_NORMAL_PRIORITY_CLASS } else { NORMAL_PRIORITY_CLASS }); }
    }
}
#[cfg(not(windows))]
mod imp {
    pub fn set_priority_class(_below_normal: bool) {}
}
pub use imp::set_priority_class;
```

- [ ] **Step 4: `main.rs` (ready, EOF exit; the protocol loop arrives in Task 10)**

```rust
use std::io::{BufRead, Write};

fn parse_threads(args: &[String]) -> u8 {
    args.iter().position(|a| a == "--threads").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u8>().ok()).filter(|n| *n >= 1).unwrap_or(16)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads = parse_threads(&args);
    rayon::ThreadPoolBuilder::new().num_threads(threads as usize).build_global().expect("rayon pool");
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    serde_json::to_writer(&mut out, &solver_worker::ready_message(threads)).expect("ready");
    out.write_all(b"\n").unwrap();
    out.flush().unwrap();
    // Until Task 10 the control loop only drains stdin; EOF exits 0 (§4.5: EOF behaves like shutdown without the ack).
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        if line.is_err() { break; }
    }
    std::process::exit(0);
}
```

- [ ] **Step 5: Run and commit**

Run: `cargo test -p solver-worker --release` then `cargo test --workspace --release`
Expected: `named_combo_roundtrip` and `ready_reports_features` pass; the workspace stays green now that `solver-worker` is a member.

(If plan 1's V1 check selected the GNU fallback, the first command is `cargo +stable-x86_64-pc-windows-gnu test -p solver-worker --release`; the workspace command stays MSVC and simply skips nothing, because `cargo test --workspace` builds `solver-worker` with the pinned toolchain and the AVX2 `build.rs` check is toolchain-independent.)

```bash
git add solver-worker Cargo.toml
git commit -m "feat(solver-worker): crate skeleton with AVX2 build check, card mapping and ready message

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 8: Worker tree build, `add_line` insertion, `remove_line` wager cap, cross-check

**Files:**
- Create: `solver-worker/src/tree_build.rs`, `solver-worker/src/history.rs`, `solver-worker/src/testutil.rs`
- Modify: `solver-worker/src/lib.rs` (add `pub mod tree_build; pub mod history; #[cfg(test)] pub mod testutil;`)
- Test: unit tests in `tree_build.rs` over `fixtures/worker/{river_two_combo,materialization_cases}.jsonl`

**Interfaces:**
- Consumes: `cards` (Task 7); fixtures (Task 6); `postflop_solver::{Action as LibAction, ActionTree, BetSize, BetSizeOptions, BoardState, DonkSizeOptions, TreeConfig, PostFlopGame}`; `proto::{Action, EffectiveTree, MaterializedNode, MenuSize, SideMenu, Street}`.
- Produces: `tree_build::{to_lib_action(&Action) -> LibAction, from_lib_action(LibAction) -> Option<Action>, board_state(Street) -> BoardState, tree_config(&EffectiveTree, pot: u32, eff: u32, rake_rate: f32, rake_cap_mchips: u32) -> Result<TreeConfig, String>, build(&EffectiveTree, pot, eff, rake_rate, rake_cap_mchips, history: &[Action]) -> Result<ActionTree, String>, apply_wager_cap(&mut ActionTree, cap: u8, history: &[LibAction]) -> Result<(), String>, enumerate(&mut ActionTree, root_street: Street, starting_pot: u32) -> Result<Vec<MaterializedNode>, String>, cross_check(lib: &[MaterializedNode], expected: &[MaterializedNode]) -> Result<(), String>}`; `history::{history_to_lib(&[Action]) -> Vec<LibAction>, indices_for(&mut PostFlopGame, &[Action]) -> Result<Vec<usize>, String>}`; `testutil::{solve_request(fixture: &str, line: usize) -> SolveRequest, cases() -> Vec<Case>}` with `Case { case, template_id, pot, eff, prefix: Vec<(String, Action)>, tree: EffectiveTree, history: Vec<Action>, decision_path: Vec<u8> }`.

**Two recorded readings of the donk rule (both in the self-review deviations list):**
1. A **root-street** `donk` of `None` is legal and is never sent to the library: upstream ignores `turn_donk_sizes` at a turn root because `prev_action` is `None` there. `tree_config` and `precheck` therefore require `Some(vec![])` only for streets strictly after the root. Spec §13.2's "a `None` donk option produces `result{error{tree_mismatch}}`" is read as being about a later street.
2. For a **later** street's `None`, §13.2 lists the case twice: once under `tree_materialization_matches_library` as `result{error{tree_mismatch}}` and once under `protocol_rejections` as "a typed rejection with `reason`". The spec is self-inconsistent here. This plan picks the **cheap** answer: the structural check is in `precheck`, so it costs no work and the reply is `ack{rejected, reason: "... donk option must be the explicit empty list, never None"}`. `tree_config` keeps the same check as a defence in depth for callers that bypass `precheck` (the in-process tests below), where it surfaces as `tree_mismatch`.

- [ ] **Step 1: Failing unit tests (bottom of `solver-worker/src/tree_build.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{cases, solve_request};
    use postflop_solver::Action as L;

    #[test]
    fn river_fixture_tree_matches_library() {
        let req = solve_request("river_two_combo", 0);
        let mut tree = build(&req.tree, req.pot, req.stack_oop.min(req.stack_ip), req.rake_rate, req.rake_cap_mchips, &req.history).unwrap();
        assert_eq!(tree.available_actions(), &[L::Check]);
        tree.apply_history(&[L::Check]).unwrap();
        assert_eq!(tree.available_actions(), &[L::Check, L::AllIn(100)]);
        let lib = enumerate(&mut tree, req.tree.root_street, req.pot).unwrap();
        cross_check(&lib, &req.tree.materialized).unwrap();
        assert_eq!(lib.len(), 3);
    }

    #[test]
    fn every_materialization_case_matches_library() {
        let all = cases();
        assert_eq!(all.len(), 43);
        for c in &all {
            let mut tree = build(&c.tree, c.pot, c.eff, 0.0, 0, &c.history).unwrap_or_else(|e| panic!("{}: {e}", c.case));
            let lib = enumerate(&mut tree, c.tree.root_street, c.pot).unwrap();
            cross_check(&lib, &c.tree.materialized).unwrap_or_else(|e| panic!("{}: {e}", c.case));
            // the decision node is reachable through the library with the observed history
            tree.apply_history(&history::history_to_lib(&c.history)).unwrap_or_else(|e| panic!("{}: {e}", c.case));
        }
        let cap1 = all.iter().find(|c| c.case == "cap1_two_wagers").unwrap();
        let mut t = build(&cap1.tree, cap1.pot, cap1.eff, 0.0, 0, &cap1.history).unwrap();
        t.apply_history(&history::history_to_lib(&cap1.history)).unwrap();
        assert_eq!(t.available_actions(), &[L::Fold, L::Call, L::AllIn(500)]);
        t.apply_history(&[L::Bet(40)]).unwrap();
        assert_eq!(t.available_actions(), &[L::Fold, L::Call, L::Raise(120), L::AllIn(500)]);   // the observed prefix survives the cap
        let ins = all.iter().find(|c| c.case == "insert_73").unwrap();
        let mut t = build(&ins.tree, ins.pot, ins.eff, 0.0, 0, &ins.history).unwrap();
        assert_eq!(t.available_actions(), &[L::Check, L::Bet(50), L::Bet(73)]);
        t.apply_history(&[L::Bet(73)]).unwrap();
        assert!(t.available_actions().contains(&L::Raise(183)));
    }

    #[test]
    fn altered_materialized_entry_is_a_mismatch() {
        let c = cases().into_iter().find(|c| c.case == "facing_350_full").unwrap();
        let mut tree = build(&c.tree, c.pot, c.eff, 0.0, 0, &c.history).unwrap();
        let lib = enumerate(&mut tree, c.tree.root_street, c.pot).unwrap();
        let mut altered = c.tree.materialized.clone();
        altered[1].terminal_pots[0] = None;                      // missing terminal marker
        assert!(cross_check(&lib, &altered).is_err());
        let mut reset = c.tree.materialized.clone();             // a per-street reset menu at the turn root
        let turn = reset.iter().position(|n| n.path == [1, 1]).unwrap();
        reset[turn].actions = vec![Action::Check, Action::Bet { to: 100 }];
        reset[turn].terminal_pots = vec![None, None];
        assert!(cross_check(&lib, &reset).is_err());
        let mut none_donk = c.tree.clone();
        none_donk.menus.get_mut(&Street::Turn).unwrap().donk = None;
        assert!(tree_config(&none_donk, c.pot, c.eff, 0.0, 0).is_err());
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p solver-worker --release --lib tree_build`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `testutil.rs`, `history.rs`, `tree_build.rs`**

`solver-worker/src/testutil.rs`:

```rust
use proto::worker::{EngineMessage, SolveRequest};
use proto::{Action, EffectiveTree};
use serde::Deserialize;

pub fn fixture_lines(name: &str) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/worker").join(format!("{name}.jsonl"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e} (run tools/gen_worker_fixtures.py)", path.display())).lines().filter(|l| !l.trim().is_empty()).map(String::from).collect()
}

pub fn solve_request(fixture: &str, line: usize) -> SolveRequest {
    match serde_json::from_str::<EngineMessage>(&fixture_lines(fixture)[line]).expect("parse fixture line") { EngineMessage::Solve(r) => r, other => panic!("line {line} of {fixture} is not a solve: {other:?}") }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Case { pub case: String, pub template_id: String, pub pot: u32, pub eff: u32, pub prefix: Vec<(String, Action)>, pub tree: EffectiveTree, pub history: Vec<Action>, pub decision_path: Vec<u8> }

pub fn cases() -> Vec<Case> { fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str(l).expect("case")).collect() }
```

`solver-worker/src/history.rs`:

```rust
use crate::tree_build::to_lib_action;
use postflop_solver::{Action as LibAction, PostFlopGame};
use proto::Action;

pub fn history_to_lib(history: &[Action]) -> Vec<LibAction> { history.iter().map(to_lib_action).collect() }

/// Action indices for `PostFlopGame::apply_history`, validated against `available_actions()` at each step
/// (memory must be allocated: `play` panics otherwise). Leaves the game at the root.
pub fn indices_for(game: &mut PostFlopGame, chip: &[Action]) -> Result<Vec<usize>, String> {
    game.back_to_root();
    let mut idx = Vec::with_capacity(chip.len());
    for a in chip {
        if game.is_terminal_node() || game.is_chance_node() { game.back_to_root(); return Err(format!("path continues past a terminal or chance node at {a:?}")); }
        let la = to_lib_action(a);
        let i = game.available_actions().iter().position(|x| *x == la).ok_or_else(|| format!("{a:?} not available at {:?}", game.history()))?;
        game.play(i);
        idx.push(i);
    }
    game.back_to_root();
    Ok(idx)
}
```

`solver-worker/src/tree_build.rs` (above the tests):

```rust
//! §10.3 tree mapping and the §4.6 cross-check: the engine materializes, the worker mirrors and compares.
use postflop_solver::{Action as LibAction, ActionTree, BetSize, BetSizeOptions, BoardState, DonkSizeOptions, TreeConfig};
use proto::{Action, EffectiveTree, MaterializedNode, MenuSize, SideMenu, Street};

pub fn to_lib_action(a: &Action) -> LibAction {
    match a { Action::Fold => LibAction::Fold, Action::Check => LibAction::Check, Action::Call => LibAction::Call,
        Action::Bet { to } => LibAction::Bet(*to as i32), Action::Raise { to } => LibAction::Raise(*to as i32), Action::AllIn { to } => LibAction::AllIn(*to as i32) }
}
pub fn from_lib_action(a: LibAction) -> Option<Action> {
    Some(match a { LibAction::Fold => Action::Fold, LibAction::Check => Action::Check, LibAction::Call => Action::Call,
        LibAction::Bet(x) => Action::Bet { to: x as u32 }, LibAction::Raise(x) => Action::Raise { to: x as u32 }, LibAction::AllIn(x) => Action::AllIn { to: x as u32 }, _ => return None })
}
pub fn board_state(s: Street) -> Result<BoardState, String> {
    match s { Street::Flop => Ok(BoardState::Flop), Street::Turn => Ok(BoardState::Turn), Street::River => Ok(BoardState::River), Street::Preflop => Err("preflop is not a solve root".into()) }
}
fn next_street(s: Street) -> Street { match s { Street::Flop => Street::Turn, _ => Street::River } }
fn actor_name(a: usize) -> String { if a == 0 { "oop".into() } else { "ip".into() } }

/// Sizes are converted through `f32 -> f64` exactly as the engine computes them; never through the `"33%"` string parser.
fn bet_sizes(side: &SideMenu) -> BetSizeOptions {
    let bet = side.bet.iter().map(|m| match m { MenuSize::Pot(r) => BetSize::PotRelative(*r as f64), MenuSize::AllIn => BetSize::AllIn }).collect();
    let raise = side.raise.iter().map(|m| match m { MenuSize::Pot(x) => BetSize::PrevBetRelative(*x as f64), MenuSize::AllIn => BetSize::AllIn }).collect();
    BetSizeOptions { bet, raise }
}

pub fn tree_config(t: &EffectiveTree, pot: u32, eff: u32, rake_rate: f32, rake_cap_mchips: u32) -> Result<TreeConfig, String> {
    if t.rules_version != 3 { return Err(format!("rules_version {} unsupported (expected 3)", t.rules_version)); }
    if t.merging_threshold != 0.0 { return Err("merging_threshold must be 0.0".into()); }
    if pot == 0 || eff == 0 { return Err("starting pot and effective stack must be positive".into()); }
    if (pot as u64) + 2 * (eff as u64) >= (1u64 << 31) { return Err("pot + stacks exceed 2^31".into()); }
    let initial_state = board_state(t.root_street)?;
    let sides = |s: Street| -> Result<[BetSizeOptions; 2], String> {
        if s < t.root_street { return Ok(Default::default()); }       // earlier streets are ignored by the library
        let m = t.menus.get(&s).ok_or_else(|| format!("no menu for {s:?}"))?;
        Ok([bet_sizes(&m.oop), bet_sizes(&m.ip)])
    };
    for s in [Street::Turn, Street::River] {
        if s > t.root_street {
            match t.menus.get(&s).and_then(|m| m.donk.as_ref()) { Some(d) if d.is_empty() => {}, Some(_) => return Err(format!("{s:?} donk sizes must be empty")), None => return Err(format!("{s:?} donk option must be the explicit empty list (never None)")) }
        }
    }
    Ok(TreeConfig {
        initial_state, starting_pot: pot as i32, effective_stack: eff as i32,
        rake_rate: rake_rate as f64, rake_cap: rake_cap_mchips as f64 / 1000.0,
        flop_bet_sizes: sides(Street::Flop)?, turn_bet_sizes: sides(Street::Turn)?, river_bet_sizes: sides(Street::River)?,
        turn_donk_sizes: Some(DonkSizeOptions { donk: vec![] }), river_donk_sizes: Some(DonkSizeOptions { donk: vec![] }),
        add_allin_threshold: t.add_allin_threshold as f64, force_allin_threshold: t.force_allin_threshold as f64, merging_threshold: 0.0,
    })
}

/// Template tree, then every inserted observed size with `add_line`, then the wager cap with `remove_line`.
pub fn build(t: &EffectiveTree, pot: u32, eff: u32, rake_rate: f32, rake_cap_mchips: u32, history: &[Action]) -> Result<ActionTree, String> {
    let mut tree = ActionTree::new(tree_config(t, pot, eff, rake_rate, rake_cap_mchips)?)?;
    for (chip, _actor, action) in &t.inserted {
        let mut line: Vec<LibAction> = chip.iter().map(to_lib_action).collect();
        line.push(to_lib_action(action));
        tree.add_line(&line).map_err(|e| format!("insert {action:?} at {chip:?}: {e}"))?;
    }
    let lib_history: Vec<LibAction> = history.iter().map(to_lib_action).collect();
    apply_wager_cap(&mut tree, t.wager_cap, &lib_history)?;
    tree.apply_history(&lib_history).map_err(|e| format!("history not representable: {e}"))?;
    tree.back_to_root();
    Ok(tree)
}

/// §4.6 wager cap: at every node where the non-all-in wagers of the street (prefix included) reached `cap`,
/// every non-all-in bet and raise is removed, except an observed prefix action.
pub fn apply_wager_cap(tree: &mut ActionTree, cap: u8, history: &[LibAction]) -> Result<(), String> {
    let mut to_remove: Vec<Vec<LibAction>> = Vec::new();
    walk_cap(tree, &mut Vec::new(), 0, cap, history, &mut to_remove)?;
    for line in to_remove { tree.remove_line(&line)?; }
    tree.back_to_root();
    Ok(())
}
fn walk_cap(tree: &mut ActionTree, line: &mut Vec<LibAction>, wagers: u8, cap: u8, history: &[LibAction], out: &mut Vec<Vec<LibAction>>) -> Result<(), String> {
    tree.apply_history(line)?;
    if tree.is_terminal_node() { return Ok(()); }
    let wagers = if tree.is_chance_node() { 0 } else { wagers };      // a street transition resets the count
    for a in tree.available_actions().to_vec() {
        let observed = line.len() < history.len() && history[..line.len()] == line[..] && history[line.len()] == a;
        let is_wager = matches!(a, LibAction::Bet(_) | LibAction::Raise(_));
        line.push(a);
        if wagers >= cap && is_wager && !observed { out.push(line.clone()); } else { walk_cap(tree, line, wagers + is_wager as u8, cap, history, out)?; }
        line.pop();
    }
    Ok(())
}

/// The library tree as a betting skeleton (§2): every action node of every street, with the actor derived from the
/// path, street from chance crossings, and terminal pots from `total_bet_amount()`.
pub fn enumerate(tree: &mut ActionTree, root_street: Street, starting_pot: u32) -> Result<Vec<MaterializedNode>, String> {
    let mut out = Vec::new();
    walk_enum(tree, &mut Vec::new(), &mut Vec::new(), root_street, 0, starting_pot as i64, &mut out)?;
    tree.back_to_root();
    Ok(out)
}
fn walk_enum(tree: &mut ActionTree, line: &mut Vec<LibAction>, path: &mut Vec<u8>, street: Street, actor: usize, p: i64, out: &mut Vec<MaterializedNode>) -> Result<(), String> {
    tree.apply_history(line)?;
    let actions = tree.available_actions().to_vec();
    let mut node = MaterializedNode { path: path.clone(), street, actor: actor_name(actor), actions: Vec::new(), terminal_pots: Vec::new() };
    let mut children = Vec::new();
    for a in &actions {
        line.push(*a);
        tree.apply_history(line)?;
        let terminal = tree.is_terminal_node();
        let pot = if terminal { let t = tree.total_bet_amount(); Some((p + 2 * t[0].min(t[1]) as i64) as u32) } else { None };
        let (s, act) = if terminal { (street, actor) } else if tree.is_chance_node() { (next_street(street), 0) } else { (street, if *a == LibAction::Check { 1 } else { actor ^ 1 }) };
        node.actions.push(from_lib_action(*a).ok_or("chance action in a decision menu")?);
        node.terminal_pots.push(pot);
        children.push((*a, terminal, s, act));
        line.pop();
    }
    out.push(node);
    for (i, (a, terminal, s, act)) in children.into_iter().enumerate() {
        if terminal { continue; }
        line.push(a); path.push(i as u8);
        walk_enum(tree, line, path, s, act, p, out)?;
        line.pop(); path.pop();
    }
    Ok(())
}

/// Any difference is `tree_mismatch` (§4.6): never a silently different tree.
pub fn cross_check(lib: &[MaterializedNode], expected: &[MaterializedNode]) -> Result<(), String> {
    if lib.len() != expected.len() { return Err(format!("node count {} (library) != {} (engine)", lib.len(), expected.len())); }
    for (a, b) in lib.iter().zip(expected) {
        if a != b { return Err(format!("first difference at path {:?}: library {:?} {:?} {:?} {:?} vs engine {:?} {:?} {:?} {:?}", a.path, a.street, a.actor, a.actions, a.terminal_pots, b.street, b.actor, b.actions, b.terminal_pots)); }
    }
    Ok(())
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p solver-worker --release --lib` then `cargo test --workspace --release`
Expected: `tree_build` tests pass (43 cases cross-checked, including the equality boundaries and the cross-street operand of §4.6); the workspace stays green.

```bash
git add solver-worker
git commit -m "feat(solver-worker): tree mapping, exact insertion, wager cap and the materialization cross-check

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 9: Worker memory admission (§10.3) and the §7 stop rule

**Files:**
- Create: `solver-worker/src/memory.rs`, `solver-worker/src/solve_loop.rs`
- Modify: `solver-worker/src/lib.rs` (add `pub mod memory; pub mod solve_loop;`)
- Test: unit tests in `memory.rs` and `solve_loop.rs`

**Interfaces:**
- Consumes: `proto::worker::WorkerError`; `postflop_solver::{PostFlopGame, solve_step, compute_exploitability}`.
- Produces: `memory::{GIB, Admission { compressed: bool, estimate_bytes: u64, mode: &'static str }, admit(f32_bytes: u64, i16_bytes: u64, memory_limit_bytes: u64) -> Result<Admission, WorkerError>}`; `solve_loop::{LoopParams { deadline_ms: u32, extraction_margin_ms: u32, target_chips: f32, started: Instant }, LoopOutcome { iterations: u32, exploitability: Option<f32>, reached_target: bool, cancelled: bool }, should_stop(elapsed_ms: f64, max_iter_ms: f64, expl_due: bool, margin_ms: f64, deadline_ms: f64) -> bool, expl_due(next_iteration: u32, fits: f64) -> bool, run(&PostFlopGame, &LoopParams, cancel: &AtomicBool, progress: impl FnMut(u32, Option<f32>)) -> LoopOutcome}`.

- [ ] **Step 1: Failing tests**

Bottom of `solver-worker/src/memory.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_rule_of_section_10_3() {
        assert_eq!(admit(GIB, GIB / 2, 10 * GIB).unwrap().mode, "f32");
        assert_eq!(admit(2 * GIB, GIB, 10 * GIB).unwrap().mode, "f32");           // inclusive
        let i16 = admit(3 * GIB, GIB + GIB / 2, 10 * GIB).unwrap();
        assert!(i16.compressed && i16.estimate_bytes == GIB + GIB / 2);
        let e = admit(20 * GIB, 10 * GIB, 10 * GIB).unwrap_err();
        assert_eq!((e.code.as_str(), e.retryable, e.estimate_bytes), ("tree_too_large", false, Some(10 * GIB)));
        let e = admit(GIB, GIB / 2, GIB).unwrap_err();                              // headroom: 1.25 GiB > 1 GiB
        assert_eq!((e.code.as_str(), e.estimate_bytes), ("tree_too_large", Some(GIB)));
    }
}
```

Bottom of `solver-worker/src/solve_loop.rs` — the §7 stop rule is arithmetic, so it is tested as arithmetic, without a solve:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stop_rule_of_section_7() {
        // stop when elapsed + 1.5 * max_iteration + (exploitability pass if due) + margin > deadline
        assert!(!should_stop(1000.0, 200.0, false, 200.0, 2000.0));            // 1000 + 300 + 200 = 1500 <= 2000
        assert!(!should_stop(1500.0, 200.0, false, 200.0, 2000.0));            // 1500 + 300 + 200 = 2000, not strictly greater
        assert!(should_stop(1501.0, 200.0, false, 200.0, 2000.0));
        assert!(should_stop(1500.0, 200.0, true, 200.0, 2000.0));              // the due exploitability pass costs one iteration
        assert!(!should_stop(0.0, 0.0, false, 200.0, 300.0));                  // before the first iteration only the margin counts
        assert!(should_stop(0.0, 0.0, false, 600.0, 300.0));                   // margin alone exceeds the deadline: no_iteration
    }
    #[test]
    fn exploitability_cadence() {
        // every 10 iterations ...
        assert!(expl_due(10, 1000.0) && expl_due(20, 1000.0) && !expl_due(11, 1000.0));
        // ... and additionally whenever fewer than 10 iterations still fit
        assert!(expl_due(3, 9.5) && expl_due(1, 0.0));
        assert!(!expl_due(3, 10.0));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p solver-worker --release --lib memory solve_loop`
Expected: FAIL to compile (`admit`, `should_stop`, `expl_due` missing).

- [ ] **Step 3: Implement `memory.rs` and `solve_loop.rs`**

Add `pub mod memory; pub mod solve_loop;` to `solver-worker/src/lib.rs`.

`solver-worker/src/memory.rs`:

```rust
use proto::worker::WorkerError;

pub const GIB: u64 = 1 << 30;

#[derive(Debug, Clone, Copy)]
pub struct Admission { pub compressed: bool, pub estimate_bytes: u64, pub mode: &'static str }

/// §10.3: f32 when the f32 estimate <= 2 GiB, else i16 when the i16 estimate <= 8 GiB, else tree_too_large;
/// then refuse when `estimate * 1.25 > memory_limit_bytes`.
pub fn admit(f32_bytes: u64, i16_bytes: u64, memory_limit_bytes: u64) -> Result<Admission, WorkerError> {
    let too_large = |estimate: u64, msg: &str| WorkerError { code: "tree_too_large".into(), message: msg.into(), retryable: false, estimate_bytes: Some(estimate) };
    let a = if f32_bytes <= 2 * GIB { Admission { compressed: false, estimate_bytes: f32_bytes, mode: "f32" } }
        else if i16_bytes <= 8 * GIB { Admission { compressed: true, estimate_bytes: i16_bytes, mode: "i16" } }
        else { return Err(too_large(i16_bytes, "i16 estimate above 8 GiB")); };
    if a.estimate_bytes + a.estimate_bytes / 4 > memory_limit_bytes { return Err(too_large(a.estimate_bytes, "estimate * 1.25 above memory_limit_bytes")); }
    Ok(a)
}
```

`solver-worker/src/solve_loop.rs` (above the tests):

```rust
use postflop_solver::{compute_exploitability, solve_step, PostFlopGame};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub struct LoopParams { pub deadline_ms: u32, pub extraction_margin_ms: u32, pub target_chips: f32, pub started: Instant }
#[derive(Debug, Clone, Copy)]
pub struct LoopOutcome { pub iterations: u32, pub exploitability: Option<f32>, pub reached_target: bool, pub cancelled: bool }

/// §7 stop rule, as pure arithmetic so it can be tested without a solve: stop when
/// `elapsed + 1.5 * max_iteration_so_far + (one more iteration if an exploitability pass is due) + margin > deadline`.
/// `compute_exploitability` costs about one iteration (measured, R8).
pub fn should_stop(elapsed_ms: f64, max_iter_ms: f64, expl_due: bool, margin_ms: f64, deadline_ms: f64) -> bool {
    elapsed_ms + 1.5 * max_iter_ms + if expl_due { max_iter_ms } else { 0.0 } + margin_ms > deadline_ms
}
/// §7 cadence: every 10 iterations, and additionally whenever fewer than 10 iterations still fit before the stop point.
pub fn expl_due(next_iteration: u32, fits: f64) -> bool { next_iteration % 10 == 0 || fits < 10.0 }

pub fn run(game: &PostFlopGame, p: &LoopParams, cancel: &AtomicBool, mut progress: impl FnMut(u32, Option<f32>)) -> LoopOutcome {
    let deadline = p.deadline_ms as f64;
    let margin = p.extraction_margin_ms as f64;
    let elapsed = || p.started.elapsed().as_secs_f64() * 1000.0;
    let fits_now = |max_iter_ms: f64| if max_iter_ms > 0.0 { (deadline - elapsed() - margin) / max_iter_ms } else { f64::INFINITY };
    let mut iters = 0u32;
    let mut max_iter_ms = 0.0f64;
    let mut expl: Option<f32> = None;
    let mut last_progress = Instant::now();
    loop {
        if cancel.load(Ordering::SeqCst) { return LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: true }; }
        let due = expl_due(iters + 1, fits_now(max_iter_ms));
        if should_stop(elapsed(), max_iter_ms, due, margin, deadline) { break; }
        let t = Instant::now();
        solve_step(game, iters);
        iters += 1;
        max_iter_ms = max_iter_ms.max(t.elapsed().as_secs_f64() * 1000.0);
        if expl_due(iters, fits_now(max_iter_ms)) {
            let e = compute_exploitability(game);
            expl = Some(e);
            if e <= p.target_chips { progress(iters, expl); return LoopOutcome { iterations: iters, exploitability: expl, reached_target: true, cancelled: false }; }
        }
        if last_progress.elapsed().as_millis() >= 100 { progress(iters, expl); last_progress = Instant::now(); }
    }
    LoopOutcome { iterations: iters, exploitability: expl, reached_target: false, cancelled: false }
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p solver-worker --release --lib memory solve_loop` then `cargo test --workspace --release`
Expected: 3 passed (`admission_rule_of_section_10_3`, `stop_rule_of_section_7`, `exploitability_cadence`); the workspace stays green.

```bash
git add solver-worker
git commit -m "feat(solver-worker): section 10.3 memory admission and the section 7 stop rule

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 10: Worker extraction and lock application

**Files:**
- Create: `solver-worker/src/extract.rs`, `solver-worker/src/locks.rs`
- Modify: `solver-worker/src/lib.rs` (add `pub mod extract; pub mod locks;`)
- Test: unit tests in `extract.rs` and `locks.rs` over `fixtures/worker/{materialization_cases,lock_river}.jsonl`

**Interfaces:**
- Consumes: `cards`, `history`, `tree_build` (Tasks 7-8), `testutil::fixture_lines` (Task 8); `postflop_solver::PostFlopGame`; `proto::worker::{SolveRequest, NodeLock, NodeStrategy, StreetSolution, ResultStatus, WorkerMessage, MAX_EXPORTED_NODES, RESULT_LINE_MAX}`.
- Produces: `extract::{chip_path_of(&[MaterializedNode], &[u8]) -> Option<Vec<Action>>, extract_node(&mut PostFlopGame, &MaterializedNode, chip_path: &[Action]) -> Result<NodeStrategy, String>, SolutionMeta { exploitability_chips: f32, iterations: u32, memory_bytes: u64, mode: &'static str, locks_applied: u16 }, street_solution(&mut PostFlopGame, &SolveRequest, meta: SolutionMeta, cancel: &AtomicBool) -> Result<Option<StreetSolution>, String>}` (`Ok(None)` = cancelled between nodes); `locks::{validate(&[NodeLock]) -> Result<(), String>, apply(&mut PostFlopGame, &NodeLock, &[MaterializedNode]) -> Result<(), String>}`.
- The export limits are **imported**, never redefined: `proto::worker::MAX_EXPORTED_NODES` (100,000) and `proto::worker::RESULT_LINE_MAX` (16 MiB) come from plan 1 Task 7 (cross-plan m9).

- [ ] **Step 1: Failing tests**

Bottom of `solver-worker/src/extract.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cases;
    #[test]
    fn ordinal_paths_become_chip_paths() {
        let c = cases().into_iter().find(|c| c.case == "insert_73").unwrap();
        let m = &c.tree.materialized;
        assert_eq!(chip_path_of(m, &[]), Some(vec![]));
        assert_eq!(chip_path_of(m, &[2]), Some(vec![Action::Bet { to: 73 }]));
        assert_eq!(chip_path_of(m, &[1]), Some(vec![Action::Bet { to: 50 }]));
        assert_eq!(chip_path_of(m, &[9]), None);              // ordinal outside the menu
        let r = cases().into_iter().find(|c| c.case == "river_std_v1_100_100").unwrap();
        let root = r.tree.materialized.iter().find(|n| n.path.is_empty()).unwrap();
        assert_eq!(chip_path_of(&r.tree.materialized, &[0]), Some(vec![root.actions[0].clone()]));
    }
}
```

Bottom of `solver-worker/src/locks.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fixture_lines;
    use proto::worker::EngineMessage;

    fn staged() -> Vec<NodeLock> {
        match serde_json::from_str::<EngineMessage>(&fixture_lines("lock_river")[0]).unwrap() {
            EngineMessage::Lock { locks, .. } => locks,
            other => panic!("line 0 of lock_river is not a lock: {other:?}"),
        }
    }

    #[test]
    fn lock_matrix_rules_of_section_4_5() {
        let ok = staged();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].probs.len(), 1326);
        validate(&ok).unwrap();                                    // all-zero rows are the free-combo sentinel
        let mut wrong_sum = ok.clone(); wrong_sum[0].probs[0] = vec![0.1, 0.3];
        assert!(validate(&wrong_sum).unwrap_err().contains("sums to"));
        let mut out_of_range = ok.clone(); out_of_range[0].probs[0] = vec![-0.1, 1.1];
        assert!(validate(&out_of_range).unwrap_err().contains("outside"));
        let mut ragged = ok.clone(); ragged[0].probs[5] = vec![1.0];
        assert!(validate(&ragged).unwrap_err().contains("entries"));
        let mut short = ok.clone(); short[0].probs.pop();
        assert!(validate(&short).unwrap_err().contains("1326"));
        let mut actor = ok.clone(); actor[0].actor = "hero".into();
        assert!(validate(&actor).unwrap_err().contains("actor"));
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p solver-worker --release --lib extract locks`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `extract.rs` and `locks.rs`**

Add `pub mod extract; pub mod locks;` to `solver-worker/src/lib.rs`.

`solver-worker/src/extract.rs` (above the tests):

```rust
use crate::cards::lib_hand_to_combo;
use crate::history::indices_for;
use crate::tree_build::from_lib_action;
use postflop_solver::PostFlopGame;
use proto::worker::{NodeStrategy, ResultStatus, SolveRequest, StreetSolution, WorkerMessage};
use proto::{Action, MaterializedNode};
use std::sync::atomic::{AtomicBool, Ordering};

/// §4.5 limits, owned by `proto::worker` (plan 1 Task 7). Re-exported so `job.rs` and `protocol.rs`
/// read the same numbers `validate_solution` enforces; never redefined here.
pub use proto::worker::{MAX_EXPORTED_NODES, RESULT_LINE_MAX};

pub fn chip_path_of(materialized: &[MaterializedNode], ordinal: &[u8]) -> Option<Vec<Action>> {
    let mut chip = Vec::with_capacity(ordinal.len());
    for k in 0..ordinal.len() {
        let node = materialized.iter().find(|n| n.path == ordinal[..k])?;
        chip.push(node.actions.get(ordinal[k] as usize)?.clone());
    }
    Some(chip)
}

/// One decision node: navigate, `cache_normalized_weights`, `strategy`, `expected_values_detail(current_player())`,
/// transpose the action-major compact arrays to combo-major `[1326][action]`; `available` = reach > 0 (blocked hands are absent).
pub fn extract_node(game: &mut PostFlopGame, node: &MaterializedNode, chip_path: &[Action]) -> Result<NodeStrategy, String> {
    let idx = indices_for(game, chip_path)?;
    game.apply_history(&idx);
    if game.is_terminal_node() || game.is_chance_node() { game.back_to_root(); return Err(format!("{chip_path:?} is not a decision node")); }
    game.cache_normalized_weights();
    let player = game.current_player();
    let actor = if player == 0 { "oop" } else { "ip" };
    if actor != node.actor { game.back_to_root(); return Err(format!("actor {actor} at {chip_path:?} differs from the skeleton's {}", node.actor)); }
    let lib_actions: Vec<Action> = game.available_actions().iter().filter_map(|a| from_lib_action(*a)).collect();
    if lib_actions != node.actions { game.back_to_root(); return Err(format!("menu {lib_actions:?} at {chip_path:?} differs from the skeleton's {:?}", node.actions)); }
    let hands = game.private_cards(player).to_vec();
    let (n, na) = (hands.len(), node.actions.len());
    let strat = game.strategy();
    let evs = game.expected_values_detail(player);
    let weights = game.weights(player).to_vec();
    if strat.len() != n * na || evs.len() != n * na { game.back_to_root(); return Err("matrix shape".into()); }
    let mut probs = vec![vec![0.0f32; na]; 1326];
    let mut ev_chips = vec![vec![0.0f32; na]; 1326];
    let mut available = vec![false; 1326];
    for (h, hand) in hands.iter().enumerate() {
        if weights[h] <= 0.0 { continue; }
        let c = lib_hand_to_combo(*hand) as usize;
        available[c] = true;
        for a in 0..na { probs[c][a] = strat[a * n + h]; ev_chips[c][a] = evs[a * n + h]; }
    }
    game.back_to_root();
    Ok(NodeStrategy { path: chip_path.to_vec(), actor: node.actor.clone(), actions: node.actions.clone(), probs, ev_chips, available })
}

#[derive(Debug, Clone, Copy)]
pub struct SolutionMeta { pub exploitability_chips: f32, pub iterations: u32, pub memory_bytes: u64, pub mode: &'static str, pub locks_applied: u16 }

fn assemble(nodes: Vec<NodeStrategy>, requested: u32, export: &str, m: &SolutionMeta) -> StreetSolution {
    let covered_paths = nodes.iter().map(|n| n.path.clone()).collect();
    StreetSolution { nodes, requested, exploitability_chips: m.exploitability_chips, iterations: m.iterations, memory_bytes: m.memory_bytes, mode: m.mode.into(), locks_applied: m.locks_applied, export: export.into(), covered_paths }
}
fn result_len(sol: &StreetSolution, id: &str) -> usize {
    serde_json::to_vec(&WorkerMessage::Result { id: id.into(), status: ResultStatus::Ok, elapsed_ms: 0, solution: Some(sol.clone()), error: None }).map(|v| v.len()).unwrap_or(usize::MAX)
}

/// Every decision node of the current street (§4.5 limits); `Ok(None)` when cancelled between nodes.
pub fn street_solution(game: &mut PostFlopGame, req: &SolveRequest, meta: SolutionMeta, cancel: &AtomicBool) -> Result<Option<StreetSolution>, String> {
    let street: Vec<&MaterializedNode> = req.tree.materialized.iter().filter(|n| n.street == req.tree.root_street).collect();
    let paths: Vec<Vec<Action>> = street.iter().map(|n| chip_path_of(&req.tree.materialized, &n.path).ok_or("unresolvable ordinal path")).collect::<Result<_, _>>()?;
    let requested = paths.iter().position(|p| *p == req.history).ok_or("history is not a decision node of the street")?;
    let truncated = street.len() > MAX_EXPORTED_NODES;
    let order: Vec<usize> = if truncated { vec![requested] } else { (0..street.len()).collect() };
    let mut nodes = Vec::with_capacity(order.len());
    for &i in &order {
        if cancel.load(Ordering::SeqCst) { return Ok(None); }
        nodes.push(extract_node(game, street[i], &paths[i])?);
    }
    let requested_index = if truncated { 0 } else { requested } as u32;
    let sol = assemble(nodes, requested_index, if truncated { "truncated" } else { "street" }, &meta);
    if result_len(&sol, &req.id) > RESULT_LINE_MAX {
        let only = sol.nodes.into_iter().nth(requested_index as usize).ok_or("requested node missing")?;
        return Ok(Some(assemble(vec![only], 0, "truncated", &meta)));
    }
    Ok(Some(sol))
}
```

`solver-worker/src/locks.rs` (above the tests):

```rust
use crate::cards::lib_hand_to_combo;
use crate::history::indices_for;
use crate::extract::chip_path_of;
use postflop_solver::PostFlopGame;
use proto::worker::NodeLock;
use proto::MaterializedNode;

/// §4.5: every entry in [0, 1]; rows all zero (free-combo sentinel) or summing to 1 +- 1e-3; exactly 1326 rows.
pub fn validate(locks: &[NodeLock]) -> Result<(), String> {
    for (k, l) in locks.iter().enumerate() {
        if l.probs.len() != 1326 { return Err(format!("lock {k}: {} rows, expected 1326", l.probs.len())); }
        if l.actor != "oop" && l.actor != "ip" { return Err(format!("lock {k}: actor {}", l.actor)); }
        let width = l.probs.first().map(|r| r.len()).unwrap_or(0);
        if width == 0 { return Err(format!("lock {k}: empty rows")); }
        for (c, row) in l.probs.iter().enumerate() {
            if row.len() != width { return Err(format!("lock {k}: row {c} has {} entries, expected {width}", row.len())); }
            if row.iter().any(|p| !p.is_finite() || *p < 0.0 || *p > 1.0) { return Err(format!("lock {k}: row {c} has an entry outside [0, 1]")); }
            let sum: f32 = row.iter().sum();
            if sum != 0.0 && (sum - 1.0).abs() > 1e-3 { return Err(format!("lock {k}: row {c} sums to {sum}")); }
        }
    }
    Ok(())
}

/// Combo-major `[1326][action]` -> the library's `[action][hand]` over `private_cards(player)`; zero rows stay free.
pub fn apply(game: &mut PostFlopGame, lock: &NodeLock, materialized: &[MaterializedNode]) -> Result<(), String> {
    let node = materialized.iter().find(|n| chip_path_of(materialized, &n.path).as_deref() == Some(&lock.path[..])).ok_or_else(|| format!("lock path {:?} is not a decision node", lock.path))?;
    if node.actor != lock.actor { return Err(format!("lock actor {} at {:?} differs from {}", lock.actor, lock.path, node.actor)); }
    if node.actions.len() != lock.probs[0].len() { return Err(format!("lock at {:?} has {} columns, node has {} actions", lock.path, lock.probs[0].len(), node.actions.len())); }
    let idx = indices_for(game, &lock.path)?;
    game.apply_history(&idx);
    let player = game.current_player();
    let hands = game.private_cards(player).to_vec();
    let (n, na) = (hands.len(), node.actions.len());
    let mut slice = vec![0.0f32; n * na];
    for (h, hand) in hands.iter().enumerate() {
        let row = &lock.probs[lib_hand_to_combo(*hand) as usize];
        for a in 0..na { slice[a * n + h] = row[a]; }
    }
    game.lock_current_strategy(&slice);
    game.back_to_root();
    Ok(())
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p solver-worker --release --lib` then `cargo test --workspace --release`
Expected: `ordinal_paths_become_chip_paths` and `lock_matrix_rules_of_section_4_5` pass alongside the Task 8 and Task 9 tests; the workspace stays green.

```bash
git add solver-worker
git commit -m "feat(solver-worker): per-node extraction with the section 4.5 export limits and lock matrix handling

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 11: Worker job runner: Building -> Solving -> Extracting with cancel checkpoints

**Files:**
- Create: `solver-worker/src/job.rs`
- Modify: `solver-worker/src/lib.rs` (add `pub mod job;`)
- Test: unit tests in `job.rs`

**Interfaces:**
- Consumes: `cards`, `tree_build`, `win` (Task 7-8), `memory`, `solve_loop` (Task 9), `extract`, `locks` (Task 10); `postflop_solver::{PostFlopGame, CardConfig, finalize}`; `proto::worker::{SolveRequest, NodeLock, StreetSolution, WorkerError, Stage, validate_solution}`.
- Produces: `job::{JobControl { cancel: Arc<AtomicBool>, progress: Box<dyn FnMut(Stage, u32, Option<f32>, u32, u64) + Send> }, JobOutcome::{Ok(StreetSolution), BestSoFar(StreetSolution), Cancelled, Error(WorkerError)}, JobResult { outcome: JobOutcome, elapsed_ms: u32 }, run(&SolveRequest, staged: Option<&[NodeLock]>, &mut JobControl) -> JobResult, error(code: &str, message: impl Into<String>, retryable: bool, estimate_bytes: Option<u64>) -> WorkerError}`.

- [ ] **Step 1: Failing tests (bottom of `solver-worker/src/job.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::solve_request;
    use proto::worker::validate_solution;
    use proto::{combo_index, Card};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    fn ctl() -> JobControl { JobControl { cancel: Arc::new(AtomicBool::new(false)), progress: Box::new(|_, _, _, _, _| {}) } }
    fn combo(a: &str, b: &str) -> usize { combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize }

    #[test]
    fn river_two_combo_solves_to_the_analytic_solution() {
        let req = solve_request("river_two_combo", 0);
        let r = run(&req, None, &mut ctl());
        let sol = match r.outcome { JobOutcome::Ok(s) => s, other => panic!("{other:?}") };
        // every decision node of the street is exported: [] oop, [check] ip, [check, allin] oop; the requested node is the one reached by history
        assert_eq!((sol.nodes.len(), sol.requested, sol.mode.as_str(), sol.export.as_str()), (3, 1, "f32", "street"));
        assert!(sol.exploitability_chips <= 0.1, "exploitability {}", sol.exploitability_chips);
        validate_solution(&sol, &req.tree.materialized).unwrap();
        let ip = &sol.nodes[1];
        assert_eq!(ip.actor, "ip");
        for qq in [combo("Qc", "Qd"), combo("Qc", "Qh"), combo("Qd", "Qh")] { assert!(ip.available[qq] && ip.probs[qq][1] > 0.97, "QQ bets: {:?}", ip.probs[qq]); }
        let bluff = ip.probs[combo("5c", "4d")][1];
        assert!((bluff - 0.5).abs() <= 0.03, "54o bluffs {bluff}");
        let oop = &sol.nodes[2];
        let call = oop.probs[combo("Ac", "Ad")][1];
        assert!((call - 0.5).abs() <= 0.03, "AA calls {call}");
        assert_eq!(oop.ev_chips[combo("Ac", "Ad")][0], 0.0);                  // fold = 0 by the identity of §10.3
        assert!(!ip.available[combo("Ac", "Ad")] && ip.probs[combo("Ac", "Ad")] == vec![0.0, 0.0]);
    }

    #[test]
    fn cancel_before_building_and_no_iteration() {
        let req = solve_request("river_two_combo", 0);
        let mut c = ctl();
        c.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(run(&req, None, &mut c).outcome, JobOutcome::Cancelled));
        let mut short = req.clone();
        short.deadline_ms = 300; short.extraction_margin_ms = 600;
        match run(&short, None, &mut ctl()).outcome { JobOutcome::Error(e) => assert_eq!((e.code.as_str(), e.retryable), ("no_iteration", false)), other => panic!("{other:?}") }
    }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p solver-worker --release --lib job`
Expected: FAIL to compile (`job::run` missing).

- [ ] **Step 3: Implement `job.rs`**

Add `pub mod job;` to `solver-worker/src/lib.rs`.

`solver-worker/src/job.rs` (above the tests):

```rust
use crate::{cards, extract, locks, memory, solve_loop, tree_build, win};
use postflop_solver::{finalize, CardConfig, PostFlopGame};
use proto::worker::{NodeLock, SolveRequest, Stage, StreetSolution, WorkerError, validate_solution};
use proto::Street;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

pub struct JobControl { pub cancel: Arc<AtomicBool>, pub progress: Box<dyn FnMut(Stage, u32, Option<f32>, u32, u64) + Send> }
#[derive(Debug)]
pub enum JobOutcome { Ok(StreetSolution), BestSoFar(StreetSolution), Cancelled, Error(WorkerError) }
#[derive(Debug)]
pub struct JobResult { pub outcome: JobOutcome, pub elapsed_ms: u32 }

pub fn error(code: &str, message: impl Into<String>, retryable: bool, estimate_bytes: Option<u64>) -> WorkerError {
    WorkerError { code: code.into(), message: message.into(), retryable, estimate_bytes }
}
fn ms(t: Instant) -> u32 { t.elapsed().as_millis().min(u32::MAX as u128) as u32 }

/// Building -> Solving -> Extracting with a cancel checkpoint after every Building step, at every iteration boundary and after every extracted node.
pub fn run(req: &SolveRequest, staged: Option<&[NodeLock]>, ctl: &mut JobControl) -> JobResult {
    let t0 = Instant::now();
    let done = |o: JobOutcome| JobResult { outcome: o, elapsed_ms: ms(t0) };
    macro_rules! checkpoint { () => { if ctl.cancel.load(Ordering::SeqCst) { return done(JobOutcome::Cancelled); } } }
    let invalid = |m: String| JobOutcome::Error(error("invalid_request", m, false, None));
    win::set_priority_class(req.background);
    (ctl.progress)(Stage::Building, 0, None, ms(t0), 0);
    checkpoint!();
    let eff = req.stack_oop.min(req.stack_ip);
    let mut tree = match tree_build::build(&req.tree, req.pot, eff, req.rake_rate, req.rake_cap_mchips, &req.history) { Ok(t) => t, Err(e) => return done(invalid(e)) };
    checkpoint!();
    let lib = match tree_build::enumerate(&mut tree, req.tree.root_street, req.pot) { Ok(l) => l, Err(e) => return done(invalid(e)) };
    if let Err(e) = tree_build::cross_check(&lib, &req.tree.materialized) { return done(JobOutcome::Error(error("tree_mismatch", e, false, None))); }
    checkpoint!();
    let street_nodes = req.tree.materialized.iter().filter(|n| n.street == req.tree.root_street).count();
    if street_nodes > extract::MAX_EXPORTED_NODES { return done(invalid(format!("{street_nodes} street nodes exceed the export limit"))); }
    let expected_cards = match req.tree.root_street { Street::Flop => 3, Street::Turn => 4, Street::River => 5, Street::Preflop => 0 };
    if req.board.len() != expected_cards { return done(invalid(format!("{} board cards for a {:?} root", req.board.len(), req.tree.root_street))); }
    let (flop, turn, river) = match cards::board_to_lib(&req.board) { Ok(b) => b, Err(e) => return done(invalid(e)) };
    let ranges = match (cards::range_to_lib(&req.oop_range), cards::range_to_lib(&req.ip_range)) { (Ok(o), Ok(i)) => [o, i], (Err(e), _) | (_, Err(e)) => return done(invalid(e)) };
    let mut game = match PostFlopGame::with_config(CardConfig { range: ranges, flop, turn, river }, tree) { Ok(g) => g, Err(e) => return done(invalid(e)) };
    checkpoint!();
    let (f32_bytes, i16_bytes) = game.memory_usage();
    let adm = match memory::admit(f32_bytes, i16_bytes, req.memory_limit_bytes) { Ok(a) => a, Err(e) => return done(JobOutcome::Error(e)) };
    (ctl.progress)(Stage::Building, 0, None, ms(t0), adm.estimate_bytes);
    checkpoint!();
    game.allocate_memory(adm.compressed);
    checkpoint!();
    let mut locks_applied = 0u16;
    if let Some(ls) = staged {
        for l in ls {
            if let Err(e) = locks::apply(&mut game, l, &req.tree.materialized) { return done(JobOutcome::Error(error("lock_mismatch", e, false, None))); }
            locks_applied += 1;
            checkpoint!();
        }
    }
    (ctl.progress)(Stage::Solving, 0, None, ms(t0), adm.estimate_bytes);
    let params = solve_loop::LoopParams { deadline_ms: req.deadline_ms, extraction_margin_ms: req.extraction_margin_ms, target_chips: req.pot as f32 * req.target_bp as f32 / 10_000.0, started: t0 };
    let cancel = ctl.cancel.clone();
    let out = { let progress = &mut ctl.progress; solve_loop::run(&game, &params, &cancel, |it, e| progress(Stage::Solving, it, e, ms(t0), adm.estimate_bytes)) };
    if out.cancelled { return done(JobOutcome::Cancelled); }
    let Some(expl) = out.exploitability else { return done(JobOutcome::Error(error("no_iteration", "no exploitability measurement before stop point", false, None))); };
    (ctl.progress)(Stage::Extracting, out.iterations, Some(expl), ms(t0), adm.estimate_bytes);
    finalize(&mut game);
    let meta = extract::SolutionMeta { exploitability_chips: expl, iterations: out.iterations, memory_bytes: adm.estimate_bytes, mode: adm.mode, locks_applied };
    let sol = match extract::street_solution(&mut game, req, meta, &ctl.cancel) { Ok(Some(s)) => s, Ok(None) => return done(JobOutcome::Cancelled), Err(e) => return done(JobOutcome::Error(error("internal", e, true, None))) };
    if let Err(e) = validate_solution(&sol, &req.tree.materialized) { return done(JobOutcome::Error(error("internal", format!("self-validation failed: {e}"), true, None))); }
    win::set_priority_class(false);
    done(if out.reached_target { JobOutcome::Ok(sol) } else { JobOutcome::BestSoFar(sol) })
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p solver-worker --release --lib` then `cargo test --workspace --release`
Expected: all pass, including the analytic polarized-versus-bluffcatcher solution (IP bets QQ 100%, 54o 50 +- 3 pp; OOP calls 50 +- 3 pp).

```bash
git add solver-worker
git commit -m "feat(solver-worker): job runner with the Building/Solving/Extracting pipeline and cancel checkpoints

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 12: Worker stdout writer, bounded line reading and the three-thread wiring

**Files:**
- Create: `solver-worker/src/writer.rs`, `solver-worker/tests/lines.rs`
- Modify: `solver-worker/src/main.rs` (replace the drain loop with the `control`/`writer`/`executor` wiring), `solver-worker/src/protocol.rs` (created here with only the shared state, `read_line` and the executor loop), `solver-worker/src/lib.rs` (add `pub mod protocol; pub mod writer;`)
- Test: unit tests in `protocol.rs` for `read_line`; `solver-worker/tests/lines.rs::{oversized_request_line_is_rejected_and_the_worker_survives, eof_exits_zero}`

**Interfaces:**
- Consumes: `job` (Task 11); `proto::worker::{AckStatus, EngineMessage, NodeLock, SolveRequest, Stage, WorkerMessage, REQUEST_LINE_MAX}`.
- Produces: `writer::{Out::{Msg(WorkerMessage), Exit(i32)}, spawn_writer() -> SyncSender<Out>}`; `protocol::{WorkerState::{Idle, Building, Solving, Extracting, Stopping}, LiveJob { id: String, cancel: Arc<AtomicBool> }, Job { req: SolveRequest, locks: Option<Vec<NodeLock>>, cancel: Arc<AtomicBool> }, Proto { state, live, finished, staged, stopping }, Shared { proto: Mutex<Proto>, out: SyncSender<Out>, jobs: Sender<Job> }, Incoming::{Line(String), TooLong, Eof}, read_line(&mut impl BufRead) -> io::Result<Incoming>, executor_loop(Arc<Shared>, Receiver<Job>), terminal(&Shared, id: &str, JobOutcome, elapsed_ms: u32)}`. `MAX_REQUEST_LINE` is **not** defined here: `protocol.rs` re-exports `proto::worker::REQUEST_LINE_MAX` (cross-plan m9).
- `handle_line` / `handle_message` arrive in Task 13; until then `main.rs` answers every well-formed line with `ack{rejected, reason: "not implemented yet"}`, which is what `lines.rs` asserts.

- [ ] **Step 1: Failing tests**

Bottom of `solver-worker/src/protocol.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufReader;
    fn read_all(input: &[u8]) -> Vec<String> {
        let mut r = BufReader::with_capacity(64, input);
        let mut out = Vec::new();
        loop {
            match read_line(&mut r).unwrap() {
                Incoming::Line(l) => out.push(l),
                Incoming::TooLong => out.push("<too long>".into()),
                Incoming::Eof => return out,
            }
        }
    }
    #[test]
    fn bounded_line_reading() {
        assert_eq!(read_all(b"{\"a\":1}\n{\"b\":2}\n"), vec!["{\"a\":1}", "{\"b\":2}"]);
        assert_eq!(read_all(b"{\"a\":1}\r\n"), vec!["{\"a\":1}"]);      // CRLF is trimmed
        assert_eq!(read_all(b"{\"a\":1}"), vec!["{\"a\":1}"]);          // a final line without a newline still arrives
        assert_eq!(read_all(b"\n\n"), vec!["", ""]);                    // blank lines are lines; handle_line ignores them
        let over = format!("{}\n{{\"b\":2}}\n", "x".repeat(MAX_REQUEST_LINE + 1));
        assert_eq!(read_all(over.as_bytes()), vec!["<too long>", "{\"b\":2}"]);   // the oversized line is consumed to its newline
    }
}
```

`solver-worker/tests/lines.rs`:

```rust
mod common;
use common::Worker;
use serde_json::Value;
use std::time::Duration;

const S: Duration = Duration::from_secs(1);

#[test]
fn oversized_request_line_is_rejected_and_the_worker_survives() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let big = format!(r#"{{"type":"cancel","id":"6","target":"{}"}}"#, "x".repeat(1_100_000));
    w.send(&big);
    let a = w.recv_until(5 * S, |m: &Value| m["type"] == "ack").unwrap();
    assert_eq!(a["status"], "rejected");
    assert!(a["reason"].as_str().unwrap().contains("1 MiB"));
    // the worker is still reading: a following well-formed line is answered, not swallowed
    w.send(r#"{"type":"cancel","id":"7","target":"nope"}"#);
    assert_eq!(w.recv_until(5 * S, |m: &Value| m["id"] == "7").unwrap()["type"], "ack");
}

#[test]
fn eof_exits_zero() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    w.close_stdin();
    assert_eq!(w.wait_exit(2 * S), Some(0));   // §4.5: EOF behaves like shutdown without the ack
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p solver-worker --release --lib protocol` and `cargo test -p solver-worker --release --test lines`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `writer.rs` and the shared state of `protocol.rs`**

`writer.rs` is complete here and never changes again. For `protocol.rs`, write only the items in this task's Produces block — the `use` block, `MAX_REQUEST_LINE`, `WorkerState`, `LiveJob`, `Job`, `Proto`, `Shared`, `Incoming`, `read_line`, `send`/`ack`/`rejected`, `terminal` and `executor_loop`, copied verbatim from the full `protocol.rs` listing in Task 13 Step 3 — plus this placeholder in place of Task 13's `handle_line` / `handle_message` / `handle_eof`:

```rust
/// Task 13 replaces this with the §4.5 state machine.
pub fn handle_line(shared: &Shared, line: &str) {
    if line.trim().is_empty() { return; }
    let id = serde_json::from_str::<serde_json::Value>(line).ok()
        .and_then(|v| v.get("id").and_then(|i| i.as_str().map(String::from))).unwrap_or_else(|| "unknown".into());
    rejected(shared, &id, "not implemented yet");
}
/// stdin EOF: exit 0 without an ack (§4.5). Task 13 replaces this with `begin_stop`.
pub fn handle_eof(shared: &Shared) { let _ = shared.out.send(Out::Exit(0)); }
```

`solver-worker/src/writer.rs`:

```rust
use proto::worker::WorkerMessage;
use std::io::Write;
use std::sync::mpsc::{sync_channel, SyncSender};

pub enum Out { Msg(WorkerMessage), Exit(i32) }

/// The single stdout writer: one JSON object per line, flushed per message; `Exit` flushes and terminates the process.
pub fn spawn_writer() -> SyncSender<Out> {
    let (tx, rx) = sync_channel::<Out>(256);
    std::thread::Builder::new().name("writer".into()).spawn(move || {
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for item in rx {
            match item {
                Out::Msg(m) => { if serde_json::to_writer(&mut out, &m).is_err() { std::process::exit(3); } let _ = out.write_all(b"\n"); let _ = out.flush(); }
                Out::Exit(code) => { let _ = out.flush(); std::process::exit(code); }
            }
        }
        std::process::exit(0);
    }).expect("writer thread");
    tx
}
```

- [ ] **Step 4: Wire the three threads in `main.rs`**

Use the `main.rs` of Task 13 Step 4 verbatim. It is written once, there, and does not change again.

- [ ] **Step 5: Run and commit**

Run: `cargo test -p solver-worker --release` then `cargo test --workspace --release`
Expected: `bounded_line_reading`, `oversized_request_line_is_rejected_and_the_worker_survives` and `eof_exits_zero` pass; `startup::ready_reports_features` still passes.

```bash
git add solver-worker
git commit -m "feat(solver-worker): serialized stdout writer, bounded line reading and the control/writer/executor wiring

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 13: Worker state machine: admission, `ack` rules, cancel, shutdown

**Files:**
- Modify: `solver-worker/src/protocol.rs` (add `precheck`, `lenient_id`, `handle_message`, `handle_eof`, `begin_stop`), `solver-worker/src/main.rs`
- Create: `solver-worker/tests/protocol.rs`
- Test: `solver-worker/tests/protocol.rs::protocol_rejections`

**Interfaces:**
- Consumes: Task 12's `protocol` scaffolding, `job` (Task 11), fixtures (Task 6).
- Produces: `protocol::{MAX_REQUEST_LINE (re-export of proto::worker::REQUEST_LINE_MAX), lenient_id(&str) -> String, precheck(&SolveRequest) -> Result<(), String>, handle_line(&Shared, &str), handle_message(&Shared, EngineMessage), handle_eof(&Shared)}`.

**Ack ordering (§4.5, review M1):** the duplicate test must come **before** the busy test. A duplicate id of a *live* request implies `state != Idle`, so a busy-first ordering makes `reason: "duplicate"` unreachable. The order is: `stopping` -> `duplicate` (live id or a remembered finished id) -> `busy` (`state != Idle`) -> `precheck`.

**Id recovery (§4.5, review M2):** `lenient_id` must recover the id from a line that is *not* valid JSON (`"pot":NaN` is the case §13.2 names), because the ack has to carry the sender's id. A `serde_json` parse is tried first; on failure a byte scan finds `"id"` and takes the next quoted string.

- [ ] **Step 1: Failing integration test `solver-worker/tests/protocol.rs`**

```rust
mod common;
use common::{fixture_lines, Worker};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn with_id(line: &str, id: &str) -> String { edit(line, |v| v["id"] = json!(id)) }
fn ack_of(w: &Worker, id: &str) -> Value { w.recv_until(Duration::from_secs(5), |m| m["type"] == "ack" && m["id"] == id).unwrap_or_else(|| panic!("no ack for {id}")) }
fn result_of(w: &Worker, id: &str, t: Duration) -> Value { w.recv_until(t, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result for {id}")) }
const S: Duration = Duration::from_secs(1);

#[test]
fn protocol_rejections() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let river = fixture_lines("river_two_combo");
    let flop = fixture_lines("flop_cancel");
    w.send(r#"{"type":"bogus","id":"1"}"#);
    assert_eq!(ack_of(&w, "1")["status"], "rejected");
    w.send(r#"{"type":"cancel","id":"2","target":"x","extra":1}"#);
    assert_eq!(ack_of(&w, "2")["status"], "rejected");
    w.send(&edit(&with_id(&river[0], "3"), |v| { v["oop_range"].as_array_mut().unwrap().pop(); }));
    assert_eq!(ack_of(&w, "3")["status"], "rejected");
    // `NaN` is not valid JSON, so the id can only come from `lenient_id`'s byte scan
    w.send(&with_id(&river[0], "4").replacen("\"pot\":100", "\"pot\":NaN", 1));
    assert_eq!(ack_of(&w, "4")["status"], "rejected");
    w.send(&edit(&with_id(&flop[0], "5"), |v| v["tree"]["menus"]["turn"]["donk"] = Value::Null));
    let a = ack_of(&w, "5");
    assert_eq!(a["status"], "rejected");
    assert!(a["reason"].as_str().unwrap().contains("donk"));
    // busy: a long flop solve, then a river solve is rejected "busy"; a duplicate id of the live job is "duplicate"
    w.send(&with_id(&flop[0], "11"));
    assert_eq!(ack_of(&w, "11")["status"], "accepted");
    w.send(&with_id(&river[0], "12"));
    let a = ack_of(&w, "12");
    assert_eq!((a["status"].as_str(), a["reason"].as_str()), (Some("rejected"), Some("busy")));
    w.send(&with_id(&flop[0], "11"));
    assert_eq!(ack_of(&w, "11")["reason"], "duplicate");
    w.send(r#"{"type":"cancel","id":"13","target":"11"}"#);
    assert_eq!(ack_of(&w, "13")["status"], "accepted");
    assert_eq!(result_of(&w, "11", 5 * S)["status"], "cancelled");
    // a finished id is still remembered: re-sending it is a duplicate, not a fresh admission
    w.send(&with_id(&flop[0], "11"));
    assert_eq!(ack_of(&w, "11")["reason"], "duplicate");
    // a valid river solve: progress during building carries exploitability_chips null
    w.send(&with_id(&river[0], "15"));
    let p = w.recv_until(5 * S, |m| m["type"] == "progress" && m["id"] == "15").unwrap();
    assert!(p["stage"] == "building" && p["exploitability_chips"].is_null());
    assert_eq!(result_of(&w, "15", 5 * S)["status"], "ok");
    w.send(r#"{"type":"cancel","id":"16","target":"nope"}"#);
    assert_eq!(ack_of(&w, "16")["status"], "unknown_target");        // still alive after every rejection
    w.send(r#"{"type":"shutdown","id":"17"}"#);
    assert_eq!(ack_of(&w, "17")["status"], "accepted");
    assert_eq!(w.wait_exit(2 * S), Some(0));
}
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p solver-worker --release --test protocol`
Expected: FAIL (Task 12's `handle_line` rejects everything with "not implemented yet").

- [ ] **Step 3: Complete `protocol.rs`**

`solver-worker/src/writer.rs` is unchanged from Task 12. `solver-worker/src/protocol.rs` reaches this shape (the `EngineMessage::Lock` arm and the staged-lock consumption stay stubbed until Task 14):

```rust
use crate::job::{self, JobControl, JobOutcome};
use crate::writer::Out;
use proto::worker::{AckStatus, EngineMessage, NodeLock, ResultStatus, SolveRequest, Stage, WorkerMessage};
use proto::Street;
use std::collections::VecDeque;
use std::io::{self, BufRead};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};

/// §4.5's 1 MiB request-line limit, owned by `proto::worker` (plan 1 Task 7); re-exported, never redefined.
pub use proto::worker::REQUEST_LINE_MAX as MAX_REQUEST_LINE;
const FINISHED_IDS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerState { Idle, Building, Solving, Extracting, Stopping }

pub struct LiveJob { pub id: String, pub cancel: Arc<AtomicBool> }
pub struct Job { pub req: SolveRequest, pub locks: Option<Vec<NodeLock>>, pub cancel: Arc<AtomicBool> }
pub struct Proto { pub state: WorkerState, pub live: Option<LiveJob>, pub finished: VecDeque<String>, pub staged: Option<(String, Vec<NodeLock>)>, pub stopping: bool }
pub struct Shared { pub proto: Mutex<Proto>, pub out: SyncSender<Out>, pub jobs: Sender<Job> }

impl Proto { pub fn new() -> Self { Self { state: WorkerState::Idle, live: None, finished: VecDeque::new(), staged: None, stopping: false } } }
impl Default for Proto { fn default() -> Self { Self::new() } }

pub enum Incoming { Line(String), TooLong, Eof }

/// Bounded line read: a line over `MAX_REQUEST_LINE` is consumed to its newline and reported as `TooLong`.
pub fn read_line(reader: &mut impl BufRead) -> io::Result<Incoming> {
    let mut buf: Vec<u8> = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() { return Ok(if buf.is_empty() && !too_long { Incoming::Eof } else if too_long { Incoming::TooLong } else { Incoming::Line(String::from_utf8_lossy(&buf).into_owned()) }); }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') { Some(i) => (i + 1, true), None => (chunk.len(), false) };
        if !too_long { buf.extend_from_slice(&chunk[..take]); if buf.len() > MAX_REQUEST_LINE { too_long = true; buf.clear(); } }
        reader.consume(take);
        if done { return Ok(if too_long { Incoming::TooLong } else { Incoming::Line(String::from_utf8_lossy(&buf).trim_end_matches(['\n', '\r']).to_string()) }); }
    }
}

/// The ack must carry the sender's id even when the line is not valid JSON (§13.2's `"pot":NaN` case), so a
/// failed parse falls back to a byte scan: find the key `"id"`, skip the colon and whitespace, take the next
/// quoted string. Only used for rejections, so a wrong guess costs an unmatched ack, never a wrong admission.
fn lenient_id(line: &str) -> String {
    if let Some(id) = serde_json::from_str::<serde_json::Value>(line).ok().and_then(|v| v.get("id").and_then(|i| i.as_str().map(String::from))) { return id; }
    scan_id(line).unwrap_or_else(|| "unknown".into())
}
fn scan_id(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let key = br#""id""#;
    let mut i = 0;
    while i + key.len() <= b.len() {
        if &b[i..i + key.len()] == key {
            let mut j = i + key.len();
            while j < b.len() && (b[j] as char).is_whitespace() { j += 1; }
            if j < b.len() && b[j] == b':' {
                j += 1;
                while j < b.len() && (b[j] as char).is_whitespace() { j += 1; }
                if j < b.len() && b[j] == b'"' {
                    let start = j + 1;
                    let mut k = start;
                    while k < b.len() && b[k] != b'"' { if b[k] == b'\\' { k += 1; } k += 1; }
                    if k <= b.len() { return Some(line[start..k.min(line.len())].to_string()); }
                }
            }
        }
        i += 1;
    }
    None
}
fn send(shared: &Shared, m: WorkerMessage) { let _ = shared.out.send(Out::Msg(m)); }
fn ack(shared: &Shared, id: &str, status: AckStatus, reason: Option<String>, replaced: Option<bool>) {
    send(shared, WorkerMessage::Ack { id: id.to_string(), status, reason, replaced });
}
fn rejected(shared: &Shared, id: &str, reason: impl Into<String>) { ack(shared, id, AckStatus::Rejected, Some(reason.into()), None); }

/// Cheap structural checks answered with `ack{rejected}` (no work); deeper checks are `result{error{invalid_request}}` in Building.
fn precheck(req: &SolveRequest) -> Result<(), String> {
    let t = &req.tree;
    if t.materialized.is_empty() { return Err("materialized tree is empty".into()); }
    if t.root_street == Street::Preflop { return Err("root_street must be flop, turn or river".into()); }
    for s in [Street::Turn, Street::River] {
        if s > t.root_street {
            match t.menus.get(&s).and_then(|m| m.donk.as_ref()) { Some(d) if d.is_empty() => {}, Some(_) => return Err(format!("{s:?} donk sizes must be empty")), None => return Err(format!("{s:?} donk option must be the explicit empty list, never None")) }
        }
    }
    if t.materialized.iter().filter(|n| n.street == t.root_street).count() > crate::extract::MAX_EXPORTED_NODES { return Err("street node count exceeds 100000".into()); }
    let mut seen = std::collections::HashSet::new();
    if !(3..=5).contains(&req.board.len()) || !req.board.iter().all(|c| seen.insert(c.0)) { return Err("board must be 3 to 5 distinct cards".into()); }
    if req.pot == 0 || req.stack_oop == 0 || req.stack_ip == 0 { return Err("pot and stacks must be positive".into()); }
    if !req.rake_rate.is_finite() || req.rake_rate < 0.0 || req.rake_rate > 1.0 { return Err("rake_rate outside [0, 1]".into()); }
    Ok(())
}

pub fn handle_line(shared: &Shared, line: &str) {
    if line.trim().is_empty() { return; }
    match serde_json::from_str::<EngineMessage>(line) {
        Ok(msg) => handle_message(shared, msg),
        Err(e) => rejected(shared, &lenient_id(line), format!("invalid message: {e}")),
    }
}

pub fn handle_message(shared: &Shared, msg: EngineMessage) {
    match msg {
        EngineMessage::Solve(req) => {
            let mut p = shared.proto.lock().unwrap();
            if p.stopping { drop(p); return rejected(shared, &req.id, "stopping"); }
            // §4.5 ack order: DUPLICATE IS TESTED BEFORE BUSY. A duplicate id of a live request always implies
            // `state != Idle`, so a busy-first order would make `reason: "duplicate"` unreachable.
            if p.live.as_ref().is_some_and(|l| l.id == req.id) || p.finished.contains(&req.id) { drop(p); return rejected(shared, &req.id, "duplicate"); }
            if p.state != WorkerState::Idle { drop(p); return rejected(shared, &req.id, "busy"); }
            if let Err(e) = precheck(&req) { drop(p); return rejected(shared, &req.id, e); }
            let cancel = Arc::new(AtomicBool::new(false));
            p.state = WorkerState::Building;
            p.live = Some(LiveJob { id: req.id.clone(), cancel: cancel.clone() });
            drop(p);
            ack(shared, &req.id, AckStatus::Accepted, None, None);
            let _ = shared.jobs.send(Job { req, locks: None, cancel });   // Task 14 consumes `p.staged` here
        }
        // Task 14 replaces this arm with lock staging.
        EngineMessage::Lock { id, .. } => rejected(shared, &id, "lock staging arrives in Task 14"),
        EngineMessage::Cancel { id, target } => {
            let p = shared.proto.lock().unwrap();
            if let Some(live) = p.live.as_ref().filter(|l| l.id == target) { live.cancel.store(true, Ordering::SeqCst); drop(p); ack(shared, &id, AckStatus::Accepted, None, None); }
            else if p.finished.contains(&target) { drop(p); ack(shared, &id, AckStatus::AlreadyFinished, None, None); }
            else { drop(p); ack(shared, &id, AckStatus::UnknownTarget, None, None); }
        }
        EngineMessage::Shutdown { id } => { ack(shared, &id, AckStatus::Accepted, None, None); begin_stop(shared); }
    }
}

/// stdin EOF: shutdown without the ack.
pub fn handle_eof(shared: &Shared) { begin_stop(shared); }

fn begin_stop(shared: &Shared) {
    let mut p = shared.proto.lock().unwrap();
    p.stopping = true;
    p.staged = None;
    match p.live.as_ref() { Some(live) => { live.cancel.store(true, Ordering::SeqCst); p.state = WorkerState::Stopping; }
        None => { p.state = WorkerState::Stopping; let _ = shared.out.send(Out::Exit(0)); } }
    drop(p);
    let out = shared.out.clone();
    std::thread::spawn(move || { std::thread::sleep(std::time::Duration::from_millis(1900)); let _ = out.send(Out::Exit(0)); });   // §4.5: exit within 2 s regardless
}

fn terminal(shared: &Shared, id: &str, outcome: JobOutcome, elapsed_ms: u32) {
    let (status, solution, error) = match outcome {
        JobOutcome::Ok(s) => (ResultStatus::Ok, Some(s), None), JobOutcome::BestSoFar(s) => (ResultStatus::BestSoFar, Some(s), None),
        JobOutcome::Cancelled => (ResultStatus::Cancelled, None, None), JobOutcome::Error(e) => (ResultStatus::Error, None, Some(e)),
    };
    send(shared, WorkerMessage::Result { id: id.to_string(), status, elapsed_ms, solution, error });
    let mut p = shared.proto.lock().unwrap();
    p.live = None;
    p.finished.push_back(id.to_string());
    if p.finished.len() > FINISHED_IDS { p.finished.pop_front(); }
    if p.stopping { p.state = WorkerState::Stopping; let _ = shared.out.send(Out::Exit(0)); } else { p.state = WorkerState::Idle; }
}

/// The executor thread: one job at a time, panics caught at the boundary (`internal`, retryable).
pub fn executor_loop(shared: Arc<Shared>, jobs: Receiver<Job>) {
    for job in jobs {
        let id = job.req.id.clone();
        let (sh, pid) = (shared.clone(), id.clone());
        let mut ctl = JobControl { cancel: job.cancel.clone(), progress: Box::new(move |stage, iterations, exploitability_chips, elapsed_ms, memory_bytes| {
            { let mut p = sh.proto.lock().unwrap(); if !p.stopping { p.state = match stage { Stage::Building => WorkerState::Building, Stage::Solving => WorkerState::Solving, Stage::Extracting => WorkerState::Extracting }; } }
            let _ = sh.out.send(Out::Msg(WorkerMessage::Progress { id: pid.clone(), stage, iterations, exploitability_chips, elapsed_ms, memory_bytes }));
        }) };
        let started = std::time::Instant::now();
        let (outcome, elapsed) = match catch_unwind(AssertUnwindSafe(|| job::run(&job.req, job.locks.as_deref(), &mut ctl))) {
            Ok(r) => (r.outcome, r.elapsed_ms),
            Err(p) => { let msg = p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
                        (JobOutcome::Error(job::error("internal", msg, true, None)), started.elapsed().as_millis() as u32) }
        };
        terminal(&shared, &id, outcome, elapsed);
    }
}
```

- [ ] **Step 4: Wire the threads in `main.rs`**

Replace the drain loop and the ready write:

```rust
use solver_worker::protocol::{executor_loop, handle_eof, handle_line, read_line, Incoming, Proto, Shared};
use solver_worker::writer::{spawn_writer, Out};
use std::sync::{mpsc::channel, Arc, Mutex};

fn parse_threads(args: &[String]) -> u8 {
    args.iter().position(|a| a == "--threads").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u8>().ok()).filter(|n| *n >= 1).unwrap_or(16)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let threads = parse_threads(&args);
    rayon::ThreadPoolBuilder::new().num_threads(threads as usize).build_global().expect("rayon pool");
    let out = spawn_writer();
    let (jobs_tx, jobs_rx) = channel();
    let shared = Arc::new(Shared { proto: Mutex::new(Proto::new()), out: out.clone(), jobs: jobs_tx });
    let _ = out.send(Out::Msg(solver_worker::ready_message(threads)));
    let exec = shared.clone();
    std::thread::Builder::new().name("executor".into()).spawn(move || executor_loop(exec, jobs_rx)).expect("executor thread");
    // control: this thread reads stdin in every state
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    loop {
        match read_line(&mut reader) {
            Ok(Incoming::Line(l)) => handle_line(&shared, &l),
            Ok(Incoming::TooLong) => { let _ = out.send(Out::Msg(proto::worker::WorkerMessage::Ack { id: "unknown".into(), status: proto::worker::AckStatus::Rejected, reason: Some("line exceeds 1 MiB".into()), replaced: None })); }
            Ok(Incoming::Eof) | Err(_) => { handle_eof(&shared); break; }
        }
    }
    std::thread::park();   // the writer exits the process (Out::Exit) once the live job, if any, has written its terminal result
}
```

- [ ] **Step 5: Run and commit**

Run: `cargo test -p solver-worker --release --test protocol` then `cargo test --workspace --release`
Expected: `protocol_rejections` passes together with `startup`, `lines` and the lib tests; the workspace stays green.

```bash
git add solver-worker
git commit -m "feat(solver-worker): section 4.5 state machine with duplicate-before-busy ack ordering and lenient id recovery

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 14: Worker lock staging and the cancel lifecycle

**Files:**
- Modify: `solver-worker/src/protocol.rs` (the `EngineMessage::Lock` arm and the staged-lock consumption in the `Solve` arm)
- Create: `solver-worker/tests/locks_and_cancel.rs`
- Test: `solver-worker/tests/locks_and_cancel.rs::{lock_staging_rejections, cancel_between_iterations, lock_lifecycle}`

**Interfaces:**
- Consumes: Task 13's `protocol`, `locks::validate` (Task 10), fixtures.
- Produces: no new names; `Proto.staged: Option<(String, Vec<NodeLock>)>` becomes live (staged on `lock`, taken by the next `solve`, `lock_mismatch` when the spot differs, discarded either way).

- [ ] **Step 1: Failing integration tests `solver-worker/tests/locks_and_cancel.rs`**

```rust
mod common;
use common::{fixture_lines, Worker};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn with_id(line: &str, id: &str) -> String { edit(line, |v| v["id"] = json!(id)) }
fn ack_of(w: &Worker, id: &str) -> Value { w.recv_until(Duration::from_secs(5), |m| m["type"] == "ack" && m["id"] == id).unwrap_or_else(|| panic!("no ack for {id}")) }
fn result_of(w: &Worker, id: &str, t: Duration) -> Value { w.recv_until(t, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result for {id}")) }
const S: Duration = Duration::from_secs(1);

#[test]
fn lock_staging_rejections() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let river = fixture_lines("river_two_combo");
    let lock = fixture_lines("lock_river");
    // a row summing to 0.4 and a row [-0.1, 1.1] are rejected; an all-zero row is the free-combo sentinel
    w.send(&edit(&with_id(&lock[0], "7"), |v| v["locks"][0]["probs"][0] = json!([0.1, 0.3])));
    assert_eq!(ack_of(&w, "7")["status"], "rejected");
    w.send(&edit(&with_id(&lock[0], "8"), |v| v["locks"][0]["probs"][0] = json!([-0.1, 1.1])));
    assert_eq!(ack_of(&w, "8")["status"], "rejected");
    w.send(&with_id(&lock[0], "9"));
    let a = ack_of(&w, "9");
    assert_eq!((a["status"].as_str(), a["replaced"].as_bool()), (Some("staged"), Some(false)));
    w.send(&with_id(&lock[0], "10"));
    assert_eq!(ack_of(&w, "10")["replaced"], true);
    // a staged lock belonging to another spot fails the next solve with lock_mismatch and is discarded
    w.send(&with_id(&river[0], "14"));
    assert_eq!(ack_of(&w, "14")["status"], "accepted");
    let r = result_of(&w, "14", 5 * S);
    assert_eq!((r["status"].as_str(), r["error"]["code"].as_str()), (Some("error"), Some("lock_mismatch")));
    w.send(&with_id(&river[0], "15"));
    assert_eq!(result_of(&w, "15", 5 * S)["status"], "ok");
    w.close_stdin();
    assert_eq!(w.wait_exit(2 * S), Some(0));
}

#[test]
fn cancel_between_iterations() {
    let mut w = Worker::spawn(8);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let flop = fixture_lines("flop_cancel");
    // Solving: ack <= 50 ms, result{cancelled} within one iteration plus one exploitability pass (<= 1.0 s), never a second terminal
    w.send(&flop[0]);
    assert_eq!(ack_of(&w, "43")["status"], "accepted");
    w.recv_until(20 * S, |m| m["type"] == "progress" && m["stage"] == "solving" && m["iterations"].as_u64().unwrap() >= 1).expect("solving progress");
    let t = Instant::now();
    w.send(&flop[1]);
    assert_eq!(ack_of(&w, "44")["status"], "accepted");
    assert!(t.elapsed() <= Duration::from_millis(50), "ack took {:?}", t.elapsed());
    let r = result_of(&w, "43", S);
    assert_eq!(r["status"], "cancelled");
    assert!(w.recv_until(Duration::from_millis(300), |m| m["type"] == "result").is_none());
    w.send(r#"{"type":"cancel","id":"45","target":"43"}"#);
    assert_eq!(ack_of(&w, "45")["status"], "already_finished");
    // Building: cancel sent right after the solve is confirmed within one build step
    w.send(&with_id(&flop[0], "46"));
    w.send(r#"{"type":"cancel","id":"47","target":"46"}"#);
    assert_eq!(ack_of(&w, "46")["status"], "accepted");
    assert_eq!(ack_of(&w, "47")["status"], "accepted");
    assert_eq!(result_of(&w, "46", 2 * S)["status"], "cancelled");
    // Extracting: a cancel after `finalize` yields exactly one terminal (cancelled, or the racing completion)
    w.send(&edit(&with_id(&flop[0], "48"), |v| { v["deadline_ms"] = json!(1500); v["extraction_margin_ms"] = json!(600); v["target_bp"] = json!(1); }));
    w.recv_until(10 * S, |m| m["type"] == "progress" && m["stage"] == "extracting").expect("extracting progress");
    w.send(r#"{"type":"cancel","id":"49","target":"48"}"#);
    let r = result_of(&w, "48", 5 * S);
    assert!(["cancelled", "best_so_far", "ok"].contains(&r["status"].as_str().unwrap()));
    assert!(w.recv_until(Duration::from_millis(500), |m| m["type"] == "result").is_none());
}

#[test]
fn lock_lifecycle() {
    let mut w = Worker::spawn(4);
    assert_eq!(w.recv(5 * S).unwrap()["type"], "ready");
    let lock = fixture_lines("lock_river");
    let flop = fixture_lines("flop_cancel");
    let qq = proto::combo_index(proto::Card::parse("Qc").unwrap(), proto::Card::parse("Qd").unwrap()) as usize;
    let o54 = proto::combo_index(proto::Card::parse("5c").unwrap(), proto::Card::parse("4d").unwrap()) as usize;
    let aa = proto::combo_index(proto::Card::parse("Ac").unwrap(), proto::Card::parse("Ad").unwrap()) as usize;
    // unlocked reference: AA calls about 50%
    w.send(&edit(&lock[1], |v| { v["id"] = json!("60"); v["spot"] = json!("ffff"); }));
    let free = result_of(&w, "60", 5 * S);
    let free_call = free["solution"]["nodes"][2]["probs"][aa][1].as_f64().unwrap();   // nodes: [] oop, [check] ip, [check, allin] oop
    assert!((free_call - 0.5).abs() < 0.1);
    // lock then the matching solve: locked rows unchanged, locks_applied 1, hero's response differs (AA folds against the 20% bluff frequency)
    w.send(&lock[0]);
    assert_eq!(ack_of(&w, "47")["status"], "staged");
    w.send(&lock[1]);
    let r = result_of(&w, "51", 5 * S);
    assert_eq!(r["status"], "ok");
    assert_eq!(r["solution"]["locks_applied"], 1);
    let ip = &r["solution"]["nodes"][1];
    assert_eq!(ip["probs"][qq], json!([0.0, 1.0]));
    assert!((ip["probs"][o54][1].as_f64().unwrap() - 0.2).abs() < 1e-3);
    assert!(r["solution"]["nodes"][2]["probs"][aa][1].as_f64().unwrap() < 0.05);
    // a lock with another spot: the next solve with the fixture spot is lock_mismatch and the lock is discarded
    w.send(&edit(&lock[0], |v| { v["id"] = json!("61"); v["spot"] = json!("abcd"); }));
    assert_eq!(ack_of(&w, "61")["status"], "staged");
    w.send(&with_id(&lock[1], "62"));
    assert_eq!(result_of(&w, "62", 5 * S)["error"]["code"], "lock_mismatch");
    w.send(&with_id(&lock[1], "63"));
    assert_eq!(result_of(&w, "63", 5 * S)["solution"]["locks_applied"], 0);
    // lock during Solving is rejected; a lock consumed by a cancelled solve is gone
    w.send(&with_id(&flop[0], "64"));
    assert_eq!(ack_of(&w, "64")["status"], "accepted");
    w.send(&with_id(&lock[0], "65"));
    assert_eq!(ack_of(&w, "65")["reason"], "solve_in_progress");
    w.send(r#"{"type":"cancel","id":"66","target":"64"}"#);
    assert_eq!(result_of(&w, "64", 5 * S)["status"], "cancelled");
    w.send(&with_id(&lock[0], "67"));
    assert_eq!(ack_of(&w, "67")["status"], "staged");
    w.send(&edit(&with_id(&flop[0], "68"), |v| v["spot"] = lock_spot(&lock[0])));
    w.send(r#"{"type":"cancel","id":"69","target":"68"}"#);
    assert_eq!(result_of(&w, "68", 5 * S)["status"], "cancelled");
    w.send(&with_id(&lock[1], "70"));
    assert_eq!(result_of(&w, "70", 5 * S)["solution"]["locks_applied"], 0);
    w.send(r#"{"type":"shutdown","id":"71"}"#);
    assert_eq!(ack_of(&w, "71")["status"], "accepted");
    assert_eq!(w.wait_exit(2 * S), Some(0));
}
fn lock_spot(line: &str) -> Value { serde_json::from_str::<Value>(line).unwrap()["spot"].clone() }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p solver-worker --release --test locks_and_cancel`
Expected: FAIL — Task 13's `handle_message` answers every `lock` line `ack{rejected, reason: "lock staging arrives in Task 14"}`.

- [ ] **Step 3: Add the lock-staging arm to `protocol.rs`**

Replace the placeholder `EngineMessage::Lock` arm and the `Solve` arm's staged-lock handling with:

```rust
        EngineMessage::Solve(req) => {
            let mut p = shared.proto.lock().unwrap();
            if p.stopping { drop(p); return rejected(shared, &req.id, "stopping"); }
            // §4.5 ack order: duplicate BEFORE busy. A duplicate id of a live request always implies
            // `state != Idle`, so testing busy first would make `reason: "duplicate"` unreachable.
            if p.live.as_ref().is_some_and(|l| l.id == req.id) || p.finished.contains(&req.id) { drop(p); return rejected(shared, &req.id, "duplicate"); }
            if p.state != WorkerState::Idle { drop(p); return rejected(shared, &req.id, "busy"); }
            if let Err(e) = precheck(&req) { drop(p); return rejected(shared, &req.id, e); }
            let cancel = Arc::new(AtomicBool::new(false));
            p.state = WorkerState::Building;
            p.live = Some(LiveJob { id: req.id.clone(), cancel: cancel.clone() });
            // A staged lock lives for exactly one solve and is consumed here, whatever the outcome.
            let staged = p.staged.take();
            drop(p);
            ack(shared, &req.id, AckStatus::Accepted, None, None);
            match staged {
                Some((spot, _)) if spot != req.spot => terminal(shared, &req.id, JobOutcome::Error(job::error("lock_mismatch", "staged lock belongs to another spot", false, None)), 0),
                staged => { let _ = shared.jobs.send(Job { req, locks: staged.map(|(_, l)| l), cancel }); }
            }
        }
        EngineMessage::Lock { id, spot, locks } => {
            let mut p = shared.proto.lock().unwrap();
            if p.state != WorkerState::Idle { drop(p); return rejected(shared, &id, "solve_in_progress"); }
            if let Err(e) = locks::validate(&locks) { drop(p); return rejected(shared, &id, e); }
            let replaced = p.staged.replace((spot, locks)).is_some();
            drop(p);
            ack(shared, &id, AckStatus::Staged, None, Some(replaced));
        }
```

and add `use crate::locks;` to the imports. `begin_stop` already clears `p.staged`, so a shutdown or EOF never leaves a lock behind.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p solver-worker --release` then `cargo test --workspace --release`
Expected: `lock_staging_rejections`, `cancel_between_iterations` and `lock_lifecycle` pass alongside `startup`, `lines`, `protocol` and the lib tests. If `cancel_between_iterations` shows an ack above 50 ms, the control thread is being starved by the rayon pool: launch the test with `--threads 8` (as written) and confirm the writer channel is not full (256 messages); the `ack` path never touches the executor.

```bash
git add solver-worker
git commit -m "feat(solver-worker): lock staging and the cancel lifecycle across Building, Solving and Extracting

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 15: The pinned V1 fixture and the worker's river contract tests

**Files:**
- Create: `solver-worker/examples/gen_basic_fixture.rs`, `fixtures/solver/basic_0p3.json` (generated), `fixtures/worker/basic_turn_std_request.jsonl` (generated), `solver-worker/tests/contract_river.rs`
- Test: `river_polarized_vs_bluffcatcher_analytic`, `ev_convention_non_root_payoffs`, `ev_conservation`, `rake_cap_applied`, `combo_matrix_two_named`

**Interfaces:**
- Consumes: `common::{Worker, fixture_lines}` (Task 7), `solver_worker::{tree_build, cards, extract}` and `postflop_solver` for the in-process conservation check and the fixture generator, fixtures (Task 6).
- Produces: `fixtures/solver/basic_0p3.json` (the V1 regression solution) and `fixtures/worker/basic_turn_std_request.jsonl` (its `solve` request line), consumed by `ev_convention_non_root_payoffs` here and by Tasks 16 and 17. Note: `river_check_only_terminal_oracle` needs `core-eval` and therefore lives in `crates/engine/tests/worker_link.rs` (Task 18), which spawns the same binary.

**Why the generator is in this task (review M9a):** `ev_convention_non_root_payoffs` reads `fixtures/worker/basic_turn_std_request.jsonl`. Generating that file in a later task would leave a failing test in the tree for one commit and force a `--skip`, which the per-task green command forbids. The example binary is therefore step 1 here.

**Spec deviation, recorded here and in the self-review (spec S7 applies it to §13.2):** §13.2 `ev_convention_non_root_payoffs` names OOP's nut hand `AA` with equity 1 against the locked betting range, but on `Qs Jd 7h 3c 2d` three queens beat aces (the same fact makes AA the bluff-catcher of the analytic test). The stated numbers (`+200` with equity 1, `-50` with equity `0.6 / 3.6`) are reproduced exactly with OOP = `QQ` (each OOP QQ combo removes every IP QQ combo, so the locked betting range against it is 54o at 20%) and `66`; the test uses `QQ,66`. Spec revision 6 carries this correction.

- [ ] **Step 1: Write and run the V1 fixture generator**

`solver-worker/examples/gen_basic_fixture.rs` (V1: the library's own `solve()` on the `examples/basic.rs` ranges and board, `turn_std_v1` at pot 200 / stack 900, target 0.3%):

```rust
use postflop_solver::*;
use proto::worker::{EngineMessage, SolveRequest, StreetSolution};
use proto::{Card, EffectiveTree, Range1326};
use solver_worker::{cards, extract, tree_build};
use std::sync::atomic::AtomicBool;

fn to_range1326(r: &Range) -> Range1326 {
    let mut out = Range1326([0.0; 1326]);
    let (hands, weights) = r.get_hands_weights(0);
    for (h, w) in hands.iter().zip(weights) { out.0[cards::lib_hand_to_combo(*h) as usize] = w; }
    out
}

fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let case: serde_json::Value = std::fs::read_to_string(root.join("fixtures/worker/materialization_cases.jsonl")).unwrap().lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap()).find(|c| c["case"] == "basic_turn_std").expect("basic_turn_std case");
    let tree: EffectiveTree = serde_json::from_value(case["tree"].clone()).unwrap();
    let oop: Range = "66+,A8s+,A5s-A4s,AJo+,K9s+,KQo,QTs+,JTs,96s+,85s+,75s+,65s,54s".parse().unwrap();
    let ip: Range = "QQ-22,AQs-A2s,ATo+,K5s+,KJo+,Q8s+,J8s+,T7s+,96s+,86s+,75s+,64s+,53s+".parse().unwrap();
    let board: Vec<Card> = ["Td", "9d", "6h", "Qc"].iter().map(|s| Card::parse(s).unwrap()).collect();
    let (mut oop_v, mut ip_v) = (to_range1326(&oop), to_range1326(&ip));
    for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { oop_v.0[i] = 0.0; ip_v.0[i] = 0.0; } }
    let req = SolveRequest { id: "basic".into(), spot: "b".repeat(64), board: board.clone(), oop_range: oop_v.clone(), ip_range: ip_v.clone(), pot: 200, stack_oop: 900, stack_ip: 900,
        rake_rate: 0.0, rake_cap_mchips: 0, tree: tree.clone(), history: vec![], target_bp: 30, deadline_ms: 20000, extraction_margin_ms: 200, memory_limit_bytes: 10 << 30, background: false };
    let action_tree = tree_build::build(&tree, 200, 900, 0.0, 0, &[]).unwrap();
    let (flop, turn, river) = cards::board_to_lib(&board).unwrap();
    let mut game = PostFlopGame::with_config(CardConfig { range: [cards::range_to_lib(&oop_v).unwrap(), cards::range_to_lib(&ip_v).unwrap()], flop, turn, river }, action_tree).unwrap();
    game.allocate_memory(false);
    let expl = solve(&mut game, 1000, 200.0 * 0.003, false);
    let meta = extract::SolutionMeta { exploitability_chips: expl, iterations: 0, memory_bytes: game.memory_usage().0, mode: "f32", locks_applied: 0 };
    let sol: StreetSolution = extract::street_solution(&mut game, &req, meta, &AtomicBool::new(false)).unwrap().unwrap();
    std::fs::create_dir_all(root.join("fixtures/solver")).unwrap();
    std::fs::write(root.join("fixtures/solver/basic_0p3.json"), serde_json::to_string(&sol).unwrap()).unwrap();
    std::fs::write(root.join("fixtures/worker/basic_turn_std_request.jsonl"), format!("{}\n", serde_json::to_string(&EngineMessage::Solve(req)).unwrap())).unwrap();
    println!("exploitability {expl:.4} chips, {} nodes", sol.nodes.len());
}
```

Run: `cargo run --release -p solver-worker --example gen_basic_fixture`
Expected: `fixtures/solver/basic_0p3.json` and `fixtures/worker/basic_turn_std_request.jsonl` written; exploitability <= 0.6 chips.

- [ ] **Step 2: Write the tests**

```rust
mod common;
use common::{fixture_lines, Worker};
use proto::{combo_index, Card, Range1326};
use serde_json::{json, Value};
use std::time::Duration;

const S: Duration = Duration::from_secs(1);
fn c(s: &str) -> Card { Card::parse(s).unwrap() }
fn ci(a: &str, b: &str) -> usize { combo_index(c(a), c(b)) as usize }
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }
fn result_of(w: &Worker, id: &str) -> Value { w.recv_until(20 * S, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result {id}")) }
fn node<'a>(r: &'a Value, path: &Value) -> &'a Value { r["solution"]["nodes"].as_array().unwrap().iter().find(|n| &n["path"] == path).unwrap_or_else(|| panic!("no node {path}")) }
/// Range-level EV of the actor at a node: sum_c w_c * sum_a p_ca * ev_ca / sum_c w_c over available combos.
fn range_ev(n: &Value, weights: &[f64]) -> f64 {
    let (mut num, mut den) = (0.0, 0.0);
    for (i, w) in weights.iter().enumerate() {
        if *w <= 0.0 || n["available"][i] != true { continue; }
        let p = n["probs"][i].as_array().unwrap(); let e = n["ev_chips"][i].as_array().unwrap();
        num += w * p.iter().zip(e).map(|(p, e)| p.as_f64().unwrap() * e.as_f64().unwrap()).sum::<f64>(); den += w;
    }
    num / den
}
fn weights_of(v: &Value, key: &str) -> Vec<f64> { v[key].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect() }
fn vec1326(entries: &[(usize, f32)]) -> Vec<f32> { let mut v = vec![0.0f32; 1326]; for (i, w) in entries { v[*i] = *w; } v }

#[test]
fn river_polarized_vs_bluffcatcher_analytic() {
    let mut w = Worker::spawn(4); ready(&w);
    let line = &fixture_lines("river_two_combo")[0];
    w.send(line);
    let r = result_of(&w, "41");
    assert_eq!(r["status"], "ok");
    let req: Value = serde_json::from_str(line).unwrap();
    assert!(r["solution"]["exploitability_chips"].as_f64().unwrap() <= 0.1);                         // <= 0.1% of the 100 pot
    let ip = node(&r, &json!([{"kind": "check"}]));
    for qq in [ci("Qc", "Qd"), ci("Qc", "Qh"), ci("Qd", "Qh")] { assert!(ip["probs"][qq][1].as_f64().unwrap() > 0.97); }
    let bluff: f64 = (0..1326).filter(|i| req["ip_range"][*i] == 0.25).map(|i| ip["probs"][i][1].as_f64().unwrap()).sum::<f64>() / 12.0;
    assert!((bluff - 0.5).abs() <= 0.03, "54o bluff frequency {bluff}");
    let oop = node(&r, &json!([{"kind": "check"}, {"kind": "allin", "to": 100}]));
    let call = oop["probs"][ci("Ac", "Ad")][1].as_f64().unwrap();
    assert!((call - 0.5).abs() <= 0.03, "AA call frequency {call}");
    // range EV: IP 75 +- 1 at its node (OOP always checks, so IP's node carries the whole root range); OOP 25 +- 1 at the root
    assert!((range_ev(ip, &weights_of(&req, "ip_range")) - 75.0).abs() <= 1.0);
    assert!((range_ev(node(&r, &json!([])), &weights_of(&req, "oop_range")) - 25.0).abs() <= 1.0);
}

#[test]
fn ev_convention_non_root_payoffs() {
    let mut w = Worker::spawn(4); ready(&w);
    let lock = fixture_lines("lock_river");
    w.send(&lock[0]);
    let oop = vec1326(&[(ci("Qc", "Qd"), 1.0), (ci("Qc", "Qh"), 1.0), (ci("Qd", "Qh"), 1.0), (ci("6c", "6d"), 1.0), (ci("6c", "6h"), 1.0), (ci("6c", "6s"), 1.0), (ci("6d", "6h"), 1.0), (ci("6d", "6s"), 1.0), (ci("6h", "6s"), 1.0)]);
    w.send(&edit(&lock[1], |v| { v["id"] = json!("80"); v["oop_range"] = json!(oop); }));
    let r = result_of(&w, "80");
    assert_eq!((r["status"].as_str(), r["solution"]["locks_applied"].as_u64()), (Some("ok"), Some(1)));
    let facing = node(&r, &json!([{"kind": "check"}, {"kind": "allin", "to": 100}]));
    for qq in [ci("Qc", "Qd"), ci("Qc", "Qh"), ci("Qd", "Qh")] {
        assert_eq!(facing["ev_chips"][qq][0], 0.0);                                                   // fold = 0
        assert!((facing["ev_chips"][qq][1].as_f64().unwrap() - 200.0).abs() <= 1e-3, "QQ call EV {}", facing["ev_chips"][qq][1]);   // equity 1 * 300 - 100
    }
    let e66 = facing["ev_chips"][ci("6c", "6d")][1].as_f64().unwrap();
    assert!((e66 + 50.0).abs() <= 1e-3, "66 call EV {e66}");                                         // 0.6 / 3.6 * 300 - 100
    // turn-root spot with a check line: fold rows are 0 at IP's facing node and every EV is bounded by the stakes
    let basic = fixture_lines("basic_turn_std_request")[0].clone();
    w.send(&edit(&basic, |v| v["id"] = json!("81")));
    let r = result_of(&w, "81");
    let nodes = r["solution"]["nodes"].as_array().unwrap();
    let ip_facing = nodes.iter().find(|n| n["actor"] == "ip" && n["actions"][0]["kind"] == "fold").unwrap();
    for i in 0..1326 { if ip_facing["available"][i] == true { assert_eq!(ip_facing["ev_chips"][i][0], 0.0); } }
    for n in nodes { for i in 0..1326 { for e in n["ev_chips"][i].as_array().unwrap() { assert!(e.as_f64().unwrap().abs() <= 1100.0); } } }
}

#[test]
fn ev_conservation() {
    // identical full ranges, river_std_v1 at 100/100, no rake: EV_OOP + EV_IP = pot +- 0.5% at the root (library API, adapter mapping)
    use postflop_solver::*;
    use solver_worker::{cards, tree_build};
    let case: Value = fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str::<Value>(l).unwrap()).find(|c| c["case"] == "river_std_v1_100_100").unwrap();
    let tree: proto::EffectiveTree = serde_json::from_value(case["tree"].clone()).unwrap();
    let board = ["Qs", "Jd", "7h", "3c", "2d"].map(c);
    let mut full = Range1326([1.0; 1326]);
    for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { full.0[i] = 0.0; } }
    let action_tree = tree_build::build(&tree, 100, 100, 0.0, 0, &[]).unwrap();
    let (flop, turn, river) = cards::board_to_lib(&board).unwrap();
    let mut game = PostFlopGame::with_config(CardConfig { range: [cards::range_to_lib(&full).unwrap(), cards::range_to_lib(&full).unwrap()], flop, turn, river }, action_tree).unwrap();
    game.allocate_memory(false);
    solve(&mut game, 1000, 0.1, false);
    game.cache_normalized_weights();
    let total: f32 = (0..2).map(|p| compute_average(&game.expected_values(p), game.normalized_weights(p))).sum();
    assert!((total - 100.0).abs() <= 0.5, "EV_OOP + EV_IP = {total}");
}

#[test]
fn rake_cap_applied() {
    let mut w = Worker::spawn(4); ready(&w);
    let base = &fixture_lines("river_two_combo")[0];
    // check-only tree: both menus empty, the only line is check-check (matched pot 100)
    let check_only = |v: &mut Value| {
        v["tree"]["menus"]["river"]["ip"]["bet"] = json!([]);
        v["tree"]["materialized"] = json!([{"path": [], "street": "river", "actor": "oop", "actions": [{"kind": "check"}], "terminal_pots": [null]},
                                           {"path": [0], "street": "river", "actor": "ip", "actions": [{"kind": "check"}], "terminal_pots": [100]}]);
        v["oop_range"] = json!(vec1326(&[(ci("Ac", "Ad"), 1.0), (ci("6c", "6d"), 1.0)]));
    };
    w.send(&edit(base, |v| { check_only(v); v["id"] = json!("90"); }));
    let unraked = result_of(&w, "90");
    w.send(&edit(base, |v| { check_only(v); v["id"] = json!("91"); v["rake_rate"] = json!(0.05); v["rake_cap_mchips"] = json!(3000); }));
    let raked = result_of(&w, "91");
    w.send(&edit(base, |v| { check_only(v); v["id"] = json!("92"); v["rake_rate"] = json!(0.05); v["rake_cap_mchips"] = json!(0); }));   // cap 0 = unraked (library: both must be > 0)
    let cap_zero = result_of(&w, "92");
    for combo in [ci("Ac", "Ad"), ci("6c", "6d")] {
        let u = node(&unraked, &json!([]))["ev_chips"][combo][0].as_f64().unwrap();
        let r = node(&raked, &json!([]))["ev_chips"][combo][0].as_f64().unwrap();
        let z = node(&cap_zero, &json!([]))["ev_chips"][combo][0].as_f64().unwrap();
        assert!(u > 0.0);
        assert!((r / u - 0.97).abs() <= 1e-3, "raked/unraked = {}", r / u);   // 5% of 100 = 5 capped at 3: the payoff is 97 instead of 100
        assert!((z - u).abs() <= 1e-3);
    }
}

#[test]
fn combo_matrix_two_named() {
    let mut w = Worker::spawn(4); ready(&w);
    let base = &fixture_lines("river_two_combo")[0];
    // OOP AsKs only, IP 7h7d only on Qs Jd 7c 3c 2d: IP holds trips and bets, OOP holds nothing and folds
    w.send(&edit(base, |v| { v["id"] = json!("95"); v["board"] = json!(["Qs", "Jd", "7c", "3c", "2d"]);
        v["oop_range"] = json!(vec1326(&[(ci("As", "Ks"), 1.0)])); v["ip_range"] = json!(vec1326(&[(ci("7h", "7d"), 1.0)])); }));
    let r = result_of(&w, "95");
    assert_eq!(r["status"], "ok");
    let ip = node(&r, &json!([{"kind": "check"}]));
    let oop = node(&r, &json!([{"kind": "check"}, {"kind": "allin", "to": 100}]));
    assert_eq!((ip["probs"].as_array().unwrap().len(), ip["probs"][0].as_array().unwrap().len()), (1326, 2));
    for i in 0..1326 {
        let ip_row = ip["probs"][i].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>();
        let oop_row = oop["probs"][i].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect::<Vec<_>>();
        if i == ci("7h", "7d") { assert!(ip["available"][i] == true && ip_row[1] > 0.99); }
        else { assert!(ip["available"][i] == false && ip_row == [0.0, 0.0]); }
        if i == ci("As", "Ks") { assert!(oop["available"][i] == true && oop_row[0] > 0.99); }
        else { assert!(oop["available"][i] == false && oop_row == [0.0, 0.0]); }
    }
    assert_ne!(ip["probs"][ci("7h", "7d")], oop["probs"][ci("As", "Ks")]);
}
```

`basic_turn_std_request` is the one-line fixture written by step 1 of this task, so all five tests run in one pass and nothing is skipped.

- [ ] **Step 3: Run and commit**

Run: `cargo test -p solver-worker --release --test contract_river` then `cargo test --workspace --release`
Expected: 5 passed; the workspace stays green with no `--skip`.

```bash
git add solver-worker fixtures/solver fixtures/worker
git commit -m "test(solver-worker): pinned V1 fixture plus river analytic, EV convention, conservation, rake cap and combo matrix contracts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 16: Worker contract tests: materialization mismatch, wager cap, exact insertion, suit permutation, pinned example

**Files:**
- Create: `solver-worker/tests/contract_tree.rs`
- Modify: nothing in `tools/` (the case `("basic_turn_std", "turn_std_v1", 200, 900, [])` is already in Task 6's generator; 43 cases)
- Test: `tree_materialization_matches_library`, `wager_cap_remove_lines`, `exact_size_insertion_no_prune`, `suit_permutation_metamorphic`, `pinned_example_fixture`

**Interfaces:**
- Consumes: `common::{Worker, fixture_lines}` (Task 7), `solver_worker::{history, tree_build}` (Task 8), `fixtures/solver/basic_0p3.json` and `fixtures/worker/basic_turn_std_request.jsonl` (Task 15), `fixtures/worker/materialization_cases.jsonl` (Task 6).
- Produces: nothing new; these tests gate the §4.6 wire behaviour.

- [ ] **Step 1: Write `solver-worker/tests/contract_tree.rs`**

```rust
mod common;
use common::{fixture_lines, Worker};
use proto::{combo_cards, combo_index, Card, Range1326};
use serde_json::{json, Value};
use std::time::Duration;

const S: Duration = Duration::from_secs(1);
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }
fn result_of(w: &Worker, id: &str) -> Value { w.recv_until(60 * S, |m| m["type"] == "result" && m["id"] == id).unwrap_or_else(|| panic!("no result {id}")) }
fn cases() -> Vec<Value> { fixture_lines("materialization_cases").iter().map(|l| serde_json::from_str(l).unwrap()).collect() }
fn case(name: &str) -> Value { cases().into_iter().find(|c| c["case"] == name).unwrap() }

#[test]
fn tree_materialization_matches_library() {
    // equality is checked in-process for all 43 cases (Task 8); here every disagreement is result{error{tree_mismatch}} on the wire
    let mut w = Worker::spawn(4); ready(&w);
    let river = &fixture_lines("river_two_combo")[0];
    w.send(&edit(river, |v| { v["id"] = json!("100"); v["tree"]["materialized"][2]["terminal_pots"][1] = json!(299); }));
    assert_eq!(result_of(&w, "100")["error"]["code"], "tree_mismatch");
    w.send(&edit(river, |v| { v["id"] = json!("101"); v["tree"]["materialized"][1]["terminal_pots"][0] = Value::Null; }));   // missing terminal marker
    assert_eq!(result_of(&w, "101")["error"]["code"], "tree_mismatch");
    let flop = &fixture_lines("flop_cancel")[0];
    let full = case("facing_350_full");
    w.send(&edit(flop, |v| {                                                                       // per-street reset menu at the turn root of facing_test_v1
        v["id"] = json!("102"); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350); v["tree"] = full["tree"].clone();
        let turn = v["tree"]["materialized"].as_array().unwrap().iter().position(|n| n["path"] == json!([1, 1])).unwrap();
        v["tree"]["materialized"][turn]["actions"] = json!([{"kind": "check"}, {"kind": "bet", "to": 100}]);
        v["tree"]["materialized"][turn]["terminal_pots"] = json!([null, null]);
    }));
    assert_eq!(result_of(&w, "102")["error"]["code"], "tree_mismatch");
    w.send(&edit(flop, |v| { v["id"] = json!("103"); v["pot"] = json!(100); v["stack_oop"] = json!(350); v["stack_ip"] = json!(350); v["tree"] = full["tree"].clone(); v["deadline_ms"] = json!(400); v["extraction_margin_ms"] = json!(200); }));
    let ok = result_of(&w, "103");
    assert!(ok["status"] == "ok" || ok["status"] == "best_so_far", "{ok}");                        // the unaltered facing_350 tree is accepted
}

#[test]
fn wager_cap_remove_lines() {
    use postflop_solver::Action as L;
    use solver_worker::{history::history_to_lib, tree_build::build};
    for (name, expected_after_prefix) in [("cap1_two_wagers", vec![L::Fold, L::Call, L::AllIn(500)]), ("cap3_three_wagers", vec![L::Fold, L::Call, L::AllIn(1000)])] {
        let c = case(name);
        let tree: proto::EffectiveTree = serde_json::from_value(c["tree"].clone()).unwrap();
        let history: Vec<proto::Action> = serde_json::from_value(c["history"].clone()).unwrap();
        let mut t = build(&tree, c["pot"].as_u64().unwrap() as u32, c["eff"].as_u64().unwrap() as u32, 0.0, 0, &history).unwrap();
        t.apply_history(&history_to_lib(&history)).unwrap();
        assert_eq!(t.available_actions(), &expected_after_prefix[..], "{name}");
        // The observed prefix survives the cap: at the node BEFORE each observed action, that action is still offered.
        // (Applying the whole line first would put the tree at the child AFTER the action, whose menu never contains it.)
        let mut line = history_to_lib(&history);
        while !line.is_empty() {
            let last = line.pop().unwrap();
            t.apply_history(&line).unwrap();
            assert!(t.available_actions().contains(&last), "{name}: {last:?} was removed by the cap at {line:?}");
        }
    }
}

#[test]
fn exact_size_insertion_no_prune() {
    let mut w = Worker::spawn(8); ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    let ins = case("insert_73");
    w.send(&edit(flop, |v| { v["id"] = json!("110"); v["pot"] = json!(100); v["stack_oop"] = json!(500); v["stack_ip"] = json!(500);
        v["tree"] = ins["tree"].clone(); v["history"] = ins["history"].clone(); v["deadline_ms"] = json!(4000); v["extraction_margin_ms"] = json!(600); }));
    let r = result_of(&w, "110");
    assert!(r["status"] == "ok" || r["status"] == "best_so_far");
    let sol = &r["solution"];
    let root = sol["nodes"].as_array().unwrap().iter().find(|n| n["path"] == json!([])).unwrap();
    assert_eq!(root["actions"], json!([{"kind": "check"}, {"kind": "bet", "to": 50}, {"kind": "bet", "to": 73}]));
    let requested = &sol["nodes"][sol["requested"].as_u64().unwrap() as usize];
    assert_eq!((requested["path"].clone(), requested["actor"].as_str()), (json!([{"kind": "bet", "to": 73}]), Some("ip")));
    // the opponent's posterior after the 73 bet is not uniform: OOP's Bet(73) probability varies across its available combos
    let p73: Vec<f64> = (0..1326).filter(|i| root["available"][*i] == true).map(|i| root["probs"][i][2].as_f64().unwrap()).collect();
    let (min, max) = p73.iter().fold((1.0f64, 0.0f64), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    assert!(max - min > 0.05, "Bet(73) is uniform: {min}..{max}");
}

fn permute_card(c: Card, perm: [u8; 4]) -> Card { Card::new(c.rank(), perm[c.suit() as usize]) }
fn permute_range(r: &[f32], perm: [u8; 4]) -> Vec<f32> {
    let mut out = vec![0.0f32; 1326];
    for i in 0..1326 { let [a, b] = combo_cards(i as u16); out[combo_index(permute_card(a, perm), permute_card(b, perm)) as usize] = r[i]; }
    out
}

#[test]
fn suit_permutation_metamorphic() {
    let mut w = Worker::spawn(4); ready(&w);
    let base = &fixture_lines("river_two_combo")[0];
    let perm = [0u8, 2, 1, 3];   // swap diamonds and hearts
    for (k, board) in [vec!["Qs", "Jd", "7h", "3c", "2d"], vec!["Qs", "Qd", "7h", "3c", "2d"], vec!["Qs", "Js", "7s", "3s", "2s"]].into_iter().enumerate() {
        let id_a = format!("12{k}a"); let id_b = format!("12{k}b");
        w.send(&edit(base, |v| { v["id"] = json!(id_a); v["board"] = json!(board); }));
        let a = result_of(&w, &id_a);
        let req: Value = serde_json::from_str(base).unwrap();
        let pb: Vec<String> = board.iter().map(|s| permute_card(Card::parse(s).unwrap(), perm).to_string()).collect();
        let oop: Vec<f32> = req["oop_range"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        let ip: Vec<f32> = req["ip_range"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
        w.send(&edit(base, |v| { v["id"] = json!(id_b); v["board"] = json!(pb); v["oop_range"] = json!(permute_range(&oop, perm)); v["ip_range"] = json!(permute_range(&ip, perm)); }));
        let b = result_of(&w, &id_b);
        assert!(a["status"] == "ok" && b["status"] == "ok");
        for (na, nb) in a["solution"]["nodes"].as_array().unwrap().iter().zip(b["solution"]["nodes"].as_array().unwrap()) {
            for i in 0..1326 {
                let [x, y] = combo_cards(i as u16);
                let j = combo_index(permute_card(x, perm), permute_card(y, perm)) as usize;
                assert_eq!(na["available"][i], nb["available"][j]);
                for (pa, pb) in na["probs"][i].as_array().unwrap().iter().zip(nb["probs"][j].as_array().unwrap()) { assert!((pa.as_f64().unwrap() - pb.as_f64().unwrap()).abs() <= 1e-4); }
            }
        }
    }
    let _ = Range1326([0.0; 1326]);
}

#[test]
fn pinned_example_fixture() {
    let mut w = Worker::spawn(16); ready(&w);
    let req = &fixture_lines("basic_turn_std_request")[0];
    let expected: Value = serde_json::from_str(&std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/solver/basic_0p3.json")).unwrap()).unwrap();
    w.send(req);
    let r = result_of(&w, "basic");
    assert_eq!(r["status"], "ok");
    let nodes = r["solution"]["nodes"].as_array().unwrap();
    let exp = expected["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), exp.len());
    for (n, e) in nodes.iter().zip(exp) {
        assert_eq!(n["path"], e["path"]);
        for i in 0..1326 {
            for a in 0..n["actions"].as_array().unwrap().len() {
                assert!((n["probs"][i][a].as_f64().unwrap() - e["probs"][i][a].as_f64().unwrap()).abs() <= 1e-3, "probs at {:?} combo {i} action {a}", n["path"]);
                assert!((n["ev_chips"][i][a].as_f64().unwrap() - e["ev_chips"][i][a].as_f64().unwrap()).abs() <= 1e-3 * 200.0, "ev at {:?} combo {i} action {a}", n["path"]);
            }
        }
    }
}
```

- [ ] **Step 2: Run and commit**

Run: `cargo test -p solver-worker --release --test contract_tree` then `cargo test --workspace --release`
Expected: all pass (the EV tolerance of the pinned fixture is 1e-3 of the pot, i.e. 0.2 chips, because rayon summation order is not deterministic across runs; probabilities within 1e-3).

```bash
git add solver-worker
git commit -m "test(solver-worker): materialization mismatch, wager cap, exact insertion, suit permutation and pinned example contracts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 17: Worker deadline and memory contracts

**Files:**
- Create: `solver-worker/tests/contract_deadline.rs`
- Test: `deadline_best_so_far_bounded`, `deadline_no_iteration`, `memory_admission`

**Interfaces:**
- Consumes: `common::{Worker, fixture_lines}` (Task 7); `fixtures/worker/{basic_turn_std_request,flop_cancel,flop_best_so_far}.jsonl` (Tasks 6 and 15); `proto::worker::validate_solution`.
- Produces: nothing new. `flop_best_so_far.jsonl` is consumed here as the second case of `deadline_best_so_far_bounded`, so §13.0's fixture inventory has a reader in this plan.

- [ ] **Step 1: Write the tests**

```rust
mod common;
use common::{fixture_lines, Worker};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const S: Duration = Duration::from_secs(1);
fn edit(line: &str, f: impl FnOnce(&mut Value)) -> String { let mut v: Value = serde_json::from_str(line).unwrap(); f(&mut v); v.to_string() }
fn ready(w: &Worker) { assert_eq!(w.recv(5 * S).unwrap()["type"], "ready"); }

#[test]
fn deadline_best_so_far_bounded() {
    // turn spot (measured 0.17 s to 50 bp), deadline 1000 ms, margin 200, target 1 bp: a complete validated solution either way
    let mut w = Worker::spawn(16); ready(&w);
    let req = &fixture_lines("basic_turn_std_request")[0];
    w.send(&edit(req, |v| { v["id"] = json!("130"); v["deadline_ms"] = json!(1000); v["extraction_margin_ms"] = json!(200); v["target_bp"] = json!(1); }));
    let r = w.recv_until(5 * S, |m| m["type"] == "result" && m["id"] == "130").unwrap();
    assert!(r["elapsed_ms"].as_u64().unwrap() <= 1000, "elapsed {}", r["elapsed_ms"]);
    let expl = r["solution"]["exploitability_chips"].as_f64().unwrap();
    match r["status"].as_str().unwrap() { "ok" => assert!(expl <= 0.02), "best_so_far" => assert!(expl > 0.02 && expl.is_finite()), other => panic!("{other}") }
    let tree: proto::EffectiveTree = serde_json::from_value(serde_json::from_str::<Value>(req).unwrap()["tree"].clone()).unwrap();
    let sol: proto::worker::StreetSolution = serde_json::from_value(r["solution"].clone()).unwrap();
    proto::worker::validate_solution(&sol, &tree.materialized).unwrap();

    // Second case, §13.0's `flop_best_so_far` fixture: a flop spot at target 1 bp that cannot reach target inside
    // 2 s, so `best_so_far` carries a measured exploitability and a validated street export.
    let flop = &fixture_lines("flop_best_so_far")[0];
    w.send(&edit(flop, |v| { v["id"] = json!("133"); v["deadline_ms"] = json!(2000); v["extraction_margin_ms"] = json!(600); v["target_bp"] = json!(1); }));
    let r = w.recv_until(15 * S, |m| m["type"] == "result" && m["id"] == "133").unwrap();
    assert_eq!(r["status"], "best_so_far", "{r}");
    assert!(r["elapsed_ms"].as_u64().unwrap() <= 2000, "elapsed {}", r["elapsed_ms"]);
    let expl = r["solution"]["exploitability_chips"].as_f64().unwrap();
    assert!(expl.is_finite() && expl > 0.0);
    let tree: proto::EffectiveTree = serde_json::from_value(serde_json::from_str::<Value>(flop).unwrap()["tree"].clone()).unwrap();
    let sol: proto::worker::StreetSolution = serde_json::from_value(r["solution"].clone()).unwrap();
    proto::worker::validate_solution(&sol, &tree.materialized).unwrap();
}

#[test]
fn deadline_no_iteration() {
    // flop spot, deadline 300 ms, margin 600: no iteration fits; error no_iteration within 300 ms plus one build step
    let mut w = Worker::spawn(8); ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    let t = Instant::now();
    w.send(&edit(flop, |v| { v["id"] = json!("131"); v["deadline_ms"] = json!(300); v["extraction_margin_ms"] = json!(600); }));
    let r = w.recv_until(5 * S, |m| m["type"] == "result" && m["id"] == "131").unwrap();
    assert_eq!((r["status"].as_str(), r["error"]["code"].as_str(), r["error"]["retryable"].as_bool()), (Some("error"), Some("no_iteration"), Some(false)));
    assert!(r.get("solution").map(|s| s.is_null()).unwrap_or(true));
    assert!(t.elapsed() <= Duration::from_millis(300 + 2000), "took {:?}", t.elapsed());
}

#[test]
fn memory_admission() {
    // a limit below the estimate is tree_too_large with estimate_bytes and no allocation (answered in milliseconds)
    let mut w = Worker::spawn(4); ready(&w);
    let flop = &fixture_lines("flop_cancel")[0];
    let t = Instant::now();
    w.send(&edit(flop, |v| { v["id"] = json!("132"); v["memory_limit_bytes"] = json!(64 * 1024 * 1024); }));
    let r = w.recv_until(5 * S, |m| m["type"] == "result" && m["id"] == "132").unwrap();
    assert_eq!((r["status"].as_str(), r["error"]["code"].as_str(), r["error"]["retryable"].as_bool()), (Some("error"), Some("tree_too_large"), Some(false)));
    assert!(r["error"]["estimate_bytes"].as_u64().unwrap() > 64 * 1024 * 1024);
    assert!(t.elapsed() <= Duration::from_secs(2));
}
```

- [ ] **Step 2: Run and commit**

Run: `cargo test -p solver-worker --release --test contract_deadline` then `cargo test --workspace --release`
Expected: 3 passed; the workspace stays green.

```bash
git add solver-worker/tests/contract_deadline.rs
git commit -m "test(solver-worker): deadline best-so-far, no-iteration and memory admission contracts

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 18: Engine worker link: `WorkerLink`, `ProcessWorker`, `ready` validation, job object

**Files:**
- Create: `crates/engine/src/worker/mod.rs`, `crates/engine/src/worker/link.rs`, `crates/engine/src/worker/ready.rs`, `crates/engine/src/worker/process.rs`, `crates/engine/src/worker/job_object.rs`, `crates/engine/tests/worker_link.rs`
- Modify: `crates/engine/src/lib.rs` (add `pub mod worker;` and the `EventSink` trait)
- Test: `worker_link.rs::{process_worker_spawns_validates_ready_and_restarts, ready_validation_rules, river_check_only_terminal_oracle}`

**Interfaces:**
- Consumes: `proto::worker::{EngineMessage, WorkerMessage, Ready, PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}`; `core_eval::{equity, exact_cost, EquityRequest, EquityMode, EquityStatus, PlayerRange}` (oracle test only, using plan 1's resolved shape).
- **Workspace-green rule (review M9b):** the oracle and process tests need `target/release/solver-worker.exe`, which no cargo dependency builds. They discover it through `POKERAI_WORKER` or the workspace target directory and return early with a printed reason when it is absent, so `cargo test --workspace --release` is green either way. CI and the task's own run step build it first.
- Produces: `worker::link::{WorkerLinkError::{Eof, Protocol(String), LineTooLong(usize), Spawn(String), Exit{code: i32}}, WorkerLink}` with `fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError>; fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError>; fn restart(&mut self) -> Result<(), WorkerLinkError>; fn kill(&mut self); fn ready(&self) -> Option<&Ready>; fn peak_working_set_bytes(&self) -> u64 { 0 }`; `worker::ready::{validate_ready(&Ready, threads: u8) -> Result<(), String>, cpu_lacks_avx2(&Ready) -> bool}`; `worker::process::{ProcessWorker::spawn(exe: &Path, threads: u8) -> Result<ProcessWorker, WorkerLinkError>, MAX_RESULT_LINE, STDERR_RING}`; `worker::job_object::assign(child: &Child) -> Result<JobHandle, String>` (16 GiB process memory limit, kill on close); `EventSink` trait in `lib.rs`: `pub trait EventSink: Send { fn emit(&mut self, ev: RecommendationEvent); }`.

- [ ] **Step 1: Failing tests `crates/engine/tests/worker_link.rs`**

```rust
use engine::worker::link::{WorkerLink, WorkerLinkError};
use engine::worker::process::ProcessWorker;
use engine::worker::ready::validate_ready;
use proto::worker::{EngineMessage, Ready, WorkerMessage, PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION};
use std::path::PathBuf;
use std::time::Duration;

/// The engine never links the worker (§3.2), so the binary is discovered, not built by this crate.
/// Order: `POKERAI_WORKER`, then the release build next to this test's target directory, then the debug one.
/// `None` means "not built in this run" and the caller `#[ignore]`s itself, so `cargo test --workspace --release`
/// is green whether or not `cargo build --release -p solver-worker` has run.
fn worker_exe() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("POKERAI_WORKER") { let p = PathBuf::from(p); if p.exists() { return Some(p); } }
    let name = if cfg!(windows) { "solver-worker.exe" } else { "solver-worker" };
    // CARGO_MANIFEST_DIR is crates/engine; the workspace target dir is ../../target
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    for profile in ["release", "debug"] {
        let p = target.join(profile).join(name);
        if p.exists() { return Some(p); }
    }
    None
}
/// Returns the path or prints why the test is skipped. Used as `let Some(exe) = require_exe() else { return; };`.
fn require_exe() -> Option<PathBuf> {
    match worker_exe() {
        Some(p) => Some(p),
        None => { eprintln!("skipping: solver-worker binary not found; run `cargo build --release -p solver-worker` or set POKERAI_WORKER"); None }
    }
}

#[test]
fn ready_validation_rules() {
    let ok = Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.into(), adapter_version: ADAPTER_VERSION, threads: 16, build_features: vec!["avx2".into()], cpu_features: vec!["avx2".into()], capabilities: vec!["solve".into()] };
    assert!(validate_ready(&ok, 16).is_ok());
    assert!(validate_ready(&ok, 8).unwrap_err().contains("threads"));
    let mut no_avx = ok.clone(); no_avx.build_features = vec![];
    assert_eq!(validate_ready(&no_avx, 16).unwrap_err(), "worker built without AVX2");
    let mut v = ok.clone(); v.proto_version = 2;
    assert!(validate_ready(&v, 16).unwrap_err().contains("proto_version"));
    let mut c = ok.clone(); c.solver_commit = "deadbeef".into();
    assert!(validate_ready(&c, 16).unwrap_err().contains("commit"));
}

#[test]
fn process_worker_spawns_validates_ready_and_restarts() {
    let Some(exe) = require_exe() else { return; };
    let mut w = ProcessWorker::spawn(&exe, 4).unwrap();
    assert_eq!(w.ready().unwrap().threads, 4);
    w.send(&EngineMessage::Cancel { id: "1".into(), target: "none".into() }).unwrap();
    match w.recv(Duration::from_secs(2)).unwrap() { Some(WorkerMessage::Ack { id, .. }) => assert_eq!(id, "1"), other => panic!("{other:?}") }
    w.kill();
    assert!(matches!(w.recv(Duration::from_millis(500)), Err(WorkerLinkError::Exit { .. }) | Err(WorkerLinkError::Eof)));
    w.restart().unwrap();
    assert_eq!(w.ready().unwrap().threads, 4);
    w.send(&EngineMessage::Shutdown { id: "2".into() }).unwrap();
    assert!(matches!(w.recv(Duration::from_secs(2)).unwrap(), Some(WorkerMessage::Ack { .. })));
}

#[test]
fn river_check_only_terminal_oracle() {
    // §13.2 (spec S8: this test lives here because its oracle is `core-eval`, which `solver-worker` may not depend on).
    // Check-check only, pot 100, stacks 100, no all-in: EV(check) per combo equals equity_actual_combo * 100 within 1e-3;
    // swapping seats leaves every value unchanged.
    use core_eval::{equity, exact_cost, EquityMode, EquityRequest, EquityStatus, PlayerRange};
    use proto::{combo_cards, combo_index, Action, Card, EffectiveTree, MaterializedNode, PlayerMenus, Range1326, Seat, SideMenu, Street};
    use proto::worker::SolveRequest;
    let Some(exe) = require_exe() else { return; };
    let board: Vec<Card> = ["Qs", "Jd", "7h", "3c", "2d"].iter().map(|s| Card::parse(s).unwrap()).collect();
    let mut oop = Range1326([0.0; 1326]); let mut ip = Range1326([0.0; 1326]);
    for (a, b) in [("Ac", "Ad"), ("Ah", "As"), ("6c", "6d"), ("Kc", "Kd")] { oop.0[combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize] = 1.0; }
    for (a, b, w) in [("Qc", "Qd", 1.0), ("5c", "4d", 0.25), ("5h", "4s", 0.25), ("Tc", "9c", 0.5)] { ip.0[combo_index(Card::parse(a).unwrap(), Card::parse(b).unwrap()) as usize] = w; }
    let menus = PlayerMenus { oop: SideMenu { bet: vec![], raise: vec![] }, ip: SideMenu { bet: vec![], raise: vec![] }, donk: None };
    let tree = EffectiveTree { rules_version: 3, template_id: "check_only_test".into(), root_street: Street::River, menus: [(Street::River, menus)].into_iter().collect(),
        add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1, inserted: vec![],
        materialized: vec![MaterializedNode { path: vec![], street: Street::River, actor: "oop".into(), actions: vec![Action::Check], terminal_pots: vec![None] },
                           MaterializedNode { path: vec![0], street: Street::River, actor: "ip".into(), actions: vec![Action::Check], terminal_pots: vec![Some(100)] }] };
    let mut w = ProcessWorker::spawn(&exe, 4).unwrap();
    let mut run = |id: &str, oop: &Range1326, ip: &Range1326| -> proto::worker::StreetSolution {
        w.send(&EngineMessage::Solve(SolveRequest { id: id.into(), spot: "c".repeat(64), board: board.clone(), oop_range: oop.clone(), ip_range: ip.clone(), pot: 100, stack_oop: 100, stack_ip: 100,
            rake_rate: 0.0, rake_cap_mchips: 0, tree: tree.clone(), history: vec![], target_bp: 1, deadline_ms: 2000, extraction_margin_ms: 200, memory_limit_bytes: 10 << 30, background: false })).unwrap();
        loop { if let Some(WorkerMessage::Result { solution, status, .. }) = w.recv(Duration::from_secs(5)).unwrap() { assert_eq!(status, proto::worker::ResultStatus::Ok); return solution.unwrap(); } }
    };
    // `core-eval`'s shape (plan 1 Task 20): players are seat-tagged ranges, the result carries per-seat shares.
    // A fixed hero combo is a range with one supported combo; hero is seat 0 and the villain seat 1 in every query.
    let hero_equity = |hero_combo: usize, villain: &Range1326| -> f32 {
        let mut fixed = Range1326([0.0; 1326]); fixed.0[hero_combo] = 1.0;
        let mut opp = villain.clone();
        let [x, y] = combo_cards(hero_combo as u16);
        for j in 0..1326 { let [p, q] = combo_cards(j as u16); if [p, q].iter().any(|c| *c == x || *c == y) { opp.0[j] = 0.0; } }
        let req = EquityRequest::single_pot(board.clone(), vec![PlayerRange { seat: Seat(0), range: fixed }, PlayerRange { seat: Seat(1), range: opp }], EquityMode::Exact);
        assert!(exact_cost(&req) <= 20_000_000, "the §7 rule admits exact enumeration for this request");
        let res = equity(&req, Duration::from_secs(5), &std::sync::atomic::AtomicBool::new(false));
        assert_eq!(res.status, EquityStatus::Ready);
        res.shares.iter().find(|s| s.pot_index == 0 && s.seat == Seat(0)).expect("hero share").value
    };
    let a = run("1", &oop, &ip);
    let b = run("2", &ip, &oop);
    for (sol, hero, villain) in [(&a, &oop, &ip), (&b, &ip, &oop)] {
        let node = &sol.nodes[0];
        for i in 0..1326 {
            if hero.0[i] == 0.0 { continue; }
            let eq = hero_equity(i, villain);
            assert!((node.ev_chips[i][0] - eq * 100.0).abs() <= 1e-3, "combo {i}: ev {} vs equity*100 {}", node.ev_chips[i][0], eq * 100.0);
        }
    }
    // IP's node (after OOP's check) carries the same identity for IP's combos; swapping seats swapped the roles, not the values
    for i in 0..1326 { if ip.0[i] > 0.0 { assert!((a.nodes[1].ev_chips[i][0] - b.nodes[0].ev_chips[i][0]).abs() <= 1e-3); } }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo build --release -p solver-worker && cargo test -p engine --test worker_link`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`crates/engine/src/lib.rs` additions:

```rust
pub mod worker;
pub trait EventSink: Send { fn emit(&mut self, ev: proto::RecommendationEvent); }
```

`crates/engine/src/worker/mod.rs`:

```rust
pub mod job_object;
pub mod link;
pub mod process;
pub mod ready;
pub use link::{WorkerLink, WorkerLinkError};
pub use process::ProcessWorker;
```

`crates/engine/src/worker/link.rs`:

```rust
use proto::worker::{EngineMessage, Ready, WorkerMessage};
use std::time::Duration;

#[derive(Debug, Clone, thiserror::Error)]
pub enum WorkerLinkError {
    #[error("worker stdout closed")] Eof,
    #[error("protocol: {0}")] Protocol(String),
    #[error("line too long: {0} bytes")] LineTooLong(usize),
    #[error("spawn: {0}")] Spawn(String),
    #[error("worker exited with code {code}")] Exit { code: i32 },
}

/// The engine's only view of the worker (§3.1): a process in production, a scripted fake in tests.
pub trait WorkerLink: Send {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError>;
    /// `Ok(None)` on timeout; `Err(Eof | Exit)` once the process is gone.
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError>;
    /// Kill, reap, respawn and validate `ready` (§12).
    fn restart(&mut self) -> Result<(), WorkerLinkError>;
    fn kill(&mut self);
    fn ready(&self) -> Option<&Ready>;
    fn peak_working_set_bytes(&self) -> u64 { 0 }
}
```

`crates/engine/src/worker/ready.rs`:

```rust
use proto::worker::{Ready, ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};

/// §4.5: proto_version 3, the pinned commit, the adapter version, `threads == requested`, `avx2` in build_features.
pub fn validate_ready(r: &Ready, threads: u8) -> Result<(), String> {
    if r.proto_version != PROTO_VERSION { return Err(format!("proto_version {} != {}", r.proto_version, PROTO_VERSION)); }
    if r.solver_commit.trim() != SOLVER_COMMIT { return Err(format!("solver commit {} != pinned {}", r.solver_commit, SOLVER_COMMIT)); }
    if r.adapter_version != ADAPTER_VERSION { return Err(format!("adapter_version {} != {}", r.adapter_version, ADAPTER_VERSION)); }
    if r.threads != threads { return Err(format!("threads {} != requested {}", r.threads, threads)); }
    if !r.build_features.iter().any(|f| f == "avx2") { return Err("worker built without AVX2".into()); }
    Ok(())
}
pub fn cpu_lacks_avx2(r: &Ready) -> bool { !r.cpu_features.iter().any(|f| f == "avx2") }
```

`crates/engine/src/worker/job_object.rs`:

```rust
//! §10.3: the worker runs inside a Windows job object with a 16 GiB process memory limit; kill on close.
use std::process::Child;

pub struct JobHandle(#[allow(dead_code)] isize);

#[cfg(windows)]
mod imp {
    use super::JobHandle;
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    #[repr(C)] struct IoCounters { r: u64, w: u64, o: u64, rt: u64, wt: u64, ot: u64 }
    #[repr(C)] struct BasicLimit { per_process_user_time: i64, per_job_user_time: i64, limit_flags: u32, min_ws: usize, max_ws: usize, active_process_limit: u32, affinity: usize, priority_class: u32, scheduling_class: u32 }
    #[repr(C)] struct ExtendedLimit { basic: BasicLimit, io: IoCounters, process_memory_limit: usize, job_memory_limit: usize, peak_process_memory: usize, peak_job_memory: usize }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *const u8, name: *const u16) -> isize;
        fn SetInformationJobObject(job: isize, class: u32, info: *const u8, len: u32) -> i32;
        fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
        fn CloseHandle(h: isize) -> i32;
    }
    const JOB_OBJECT_LIMIT_PROCESS_MEMORY: u32 = 0x100;
    const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: u32 = 9;
    pub const LIMIT_BYTES: usize = 16 << 30;
    pub fn assign(child: &Child) -> Result<JobHandle, String> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job == 0 { return Err("CreateJobObjectW failed".into()); }
            let mut info: ExtendedLimit = std::mem::zeroed();
            info.basic.limit_flags = JOB_OBJECT_LIMIT_PROCESS_MEMORY | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            info.process_memory_limit = LIMIT_BYTES;
            if SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, &info as *const _ as *const u8, std::mem::size_of::<ExtendedLimit>() as u32) == 0 { CloseHandle(job); return Err("SetInformationJobObject failed".into()); }
            if AssignProcessToJobObject(job, child.as_raw_handle() as isize) == 0 { CloseHandle(job); return Err("AssignProcessToJobObject failed".into()); }
            Ok(JobHandle(job))
        }
    }
    impl Drop for JobHandle { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
}
#[cfg(not(windows))]
mod imp { use super::JobHandle; use std::process::Child; pub fn assign(_child: &Child) -> Result<JobHandle, String> { Ok(JobHandle(0)) } }

pub fn assign(child: &Child) -> Result<JobHandle, String> { imp::assign(child) }
```

`crates/engine/src/worker/process.rs`:

```rust
use super::job_object::{self, JobHandle};
use super::link::{WorkerLink, WorkerLinkError};
use super::ready::validate_ready;
use proto::worker::{EngineMessage, Ready, WorkerMessage};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const MAX_RESULT_LINE: usize = 16 << 20;
pub const STDERR_RING: usize = 64 << 10;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

pub struct ProcessWorker {
    exe: PathBuf, threads: u8,
    child: Option<Child>, stdin: Option<ChildStdin>, lines: Option<Receiver<Result<String, WorkerLinkError>>>,
    ready: Option<Ready>, stderr: Arc<Mutex<VecDeque<u8>>>, _job: Option<JobHandle>,
}

impl ProcessWorker {
    /// Spawns with `--threads N`, assigns the job object, validates `ready` within 5 s; one retry, then `Spawn`.
    pub fn spawn(exe: &Path, threads: u8) -> Result<ProcessWorker, WorkerLinkError> {
        let mut w = ProcessWorker { exe: exe.to_path_buf(), threads, child: None, stdin: None, lines: None, ready: None, stderr: Arc::new(Mutex::new(VecDeque::new())), _job: None };
        let mut last = String::new();
        for _ in 0..2 { match w.launch() { Ok(()) => return Ok(w), Err(e) => { last = e.to_string(); w.kill(); } } }
        Err(WorkerLinkError::Spawn(format!("worker did not become ready: {last}")))
    }
    pub fn stderr_tail(&self) -> String { String::from_utf8_lossy(&self.stderr.lock().unwrap().iter().copied().collect::<Vec<u8>>()).into_owned() }

    fn launch(&mut self) -> Result<(), WorkerLinkError> {
        let mut child = Command::new(&self.exe).args(["--threads", &self.threads.to_string()]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().map_err(|e| WorkerLinkError::Spawn(format!("{}: {e}", self.exe.display())))?;
        self._job = job_object::assign(&child).ok();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        self.stdin = child.stdin.take();
        let ring = self.stderr.clone();
        std::thread::Builder::new().name("worker-stderr".into()).spawn(move || {
            let mut buf = [0u8; 4096];
            let mut r = stderr;
            while let Ok(n) = r.read(&mut buf) { if n == 0 { break; } let mut g = ring.lock().unwrap(); g.extend(&buf[..n]); while g.len() > STDERR_RING { g.pop_front(); } }
        }).expect("stderr thread");
        let (tx, rx) = channel();
        std::thread::Builder::new().name("worker-stdout".into()).spawn(move || {
            let mut reader = BufReader::with_capacity(1 << 16, stdout);
            let mut line = Vec::new();
            loop {
                line.clear();
                let mut too_long = false;
                loop {
                    let chunk = match reader.fill_buf() { Ok(c) => c, Err(_) => return };
                    if chunk.is_empty() { return; }
                    let (take, done) = match chunk.iter().position(|b| *b == b'\n') { Some(i) => (i + 1, true), None => (chunk.len(), false) };
                    if !too_long { line.extend_from_slice(&chunk[..take]); if line.len() > MAX_RESULT_LINE { too_long = true; } }
                    reader.consume(take);
                    if done { break; }
                }
                let item = if too_long { Err(WorkerLinkError::LineTooLong(line.len())) } else { Ok(String::from_utf8_lossy(&line).trim_end().to_string()) };
                if tx.send(item).is_err() { return; }
            }
        }).expect("stdout thread");
        self.lines = Some(rx);
        self.child = Some(child);
        match self.recv(STARTUP_TIMEOUT)? {
            Some(WorkerMessage::Ready(r)) => { validate_ready(&r, self.threads).map_err(WorkerLinkError::Protocol)?; self.ready = Some(r); Ok(()) }
            Some(other) => Err(WorkerLinkError::Protocol(format!("expected ready, got {other:?}"))),
            None => Err(WorkerLinkError::Protocol("no ready within 5 s".into())),
        }
    }
    fn exit_error(&mut self) -> WorkerLinkError {
        match self.child.as_mut().and_then(|c| c.try_wait().ok().flatten()) { Some(st) => WorkerLinkError::Exit { code: st.code().unwrap_or(-1) }, None => WorkerLinkError::Eof }
    }
}

impl WorkerLink for ProcessWorker {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> {
        let stdin = self.stdin.as_mut().ok_or(WorkerLinkError::Eof)?;
        let mut line = serde_json::to_vec(msg).map_err(|e| WorkerLinkError::Protocol(e.to_string()))?;
        line.push(b'\n');
        stdin.write_all(&line).and_then(|_| stdin.flush()).map_err(|_| WorkerLinkError::Eof)
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let rx = match self.lines.as_ref() { Some(r) => r, None => return Err(WorkerLinkError::Eof) };
        match rx.recv_timeout(timeout) {
            Ok(Ok(line)) => serde_json::from_str::<WorkerMessage>(&line).map(Some).map_err(|e| WorkerLinkError::Protocol(format!("{e}: {}", &line[..line.len().min(200)]))),
            Ok(Err(e)) => Err(e),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(self.exit_error()),
        }
    }
    fn restart(&mut self) -> Result<(), WorkerLinkError> {
        self.kill();
        let mut last = String::new();
        for _ in 0..2 { match self.launch() { Ok(()) => return Ok(()), Err(e) => { last = e.to_string(); self.kill(); } } }
        Err(WorkerLinkError::Spawn(format!("restart failed: {last}")))
    }
    fn kill(&mut self) {
        self.stdin.take();
        if let Some(mut c) = self.child.take() { let _ = c.kill(); let _ = c.wait(); }
        self.lines.take();
        self.ready = None;
        self._job = None;
    }
    fn ready(&self) -> Option<&Ready> { self.ready.as_ref() }
    fn peak_working_set_bytes(&self) -> u64 { peak_ws(self.child.as_ref()) }
}

#[cfg(windows)]
fn peak_ws(child: Option<&Child>) -> u64 {
    use std::os::windows::io::AsRawHandle;
    #[repr(C)] #[allow(non_snake_case)] struct Pmc { cb: u32, PageFaultCount: u32, PeakWorkingSetSize: usize, WorkingSetSize: usize, a: usize, b: usize, c: usize, d: usize, e: usize, f: usize }
    #[link(name = "psapi")] extern "system" { fn GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32; }
    let Some(c) = child else { return 0 };
    unsafe { let mut p: Pmc = std::mem::zeroed(); p.cb = std::mem::size_of::<Pmc>() as u32; if GetProcessMemoryInfo(c.as_raw_handle() as isize, &mut p, p.cb) == 0 { 0 } else { p.PeakWorkingSetSize as u64 } }
}
#[cfg(not(windows))]
fn peak_ws(_child: Option<&Child>) -> u64 { 0 }
```

- [ ] **Step 4: Run and commit**

Run: `cargo build --release -p solver-worker && cargo test -p engine --test worker_link`, then `cargo test --workspace --release`
Expected: 3 passed (the oracle test compares the worker's terminal EVs with `core-eval`'s exact enumeration on 4 x 4 combos); the workspace stays green, and stays green in a clean tree where the two spawning tests skip themselves.

```bash
git add crates/engine
git commit -m "feat(engine): WorkerLink trait, ProcessWorker with ready validation, job object and stderr ring

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 19: Engine test doubles: `FakeClock`, `FakeWorker`, `RecordingSink`, solution builder

**Files:**
- Create: `crates/engine/src/testing.rs`
- Modify: `crates/engine/src/lib.rs` (`#[cfg(any(test, feature = "testing"))] pub mod testing;`)
- Test: unit tests in `testing.rs`

**Interfaces:**
- Consumes: `Clock`, `WorkerLink`, `IdentityState`, `EventSink`, `proto::worker::*`.
- Produces: `testing::{FakeClock::{new() -> Arc<FakeClock>, advance_ms(&self, u64), set_ms(&self, u64)}, IdRef::{Last, Fixed(String)}, FakeReply::{Ack{id: IdRef, status: AckStatus, reason: Option<String>}, Progress{id: IdRef, stage: Stage, iterations: u32, exploitability_chips: Option<f32>, elapsed_ms: u32}, Result{id: IdRef, status: ResultStatus, solution: Option<StreetSolution>, error: Option<WorkerError>, elapsed_ms: u32}, Delay{ms: u64}, Eof, Malformed(String), Oversized(usize), Hang, InvalidateIdentity}, FakeState { sent: Vec<EngineMessage>, kills: u32, restarts: u32, last_solve_id: Option<String>, cancels: Vec<String> }, FakeWorker::scripted(clock: Arc<FakeClock>, identity: Arc<Mutex<IdentityState>>, script: Vec<FakeReply>) -> (Box<FakeWorker>, Arc<Mutex<FakeState>>), FakeWorker::default_ready() -> Ready, RecordingSink { events: Arc<Mutex<Vec<Recorded>>> } with Recorded { at_ms: u64, kills: u32, event: RecommendationEvent }, RecordingSink::new(clock: Arc<dyn Clock>, state: Option<Arc<Mutex<FakeState>>>) -> (RecordingSink, Arc<Mutex<Vec<Recorded>>>), uniform_solution(tree: &EffectiveTree, requested: &[Action], exploitability_chips: f32) -> StreetSolution}`.

- [ ] **Step 1: Failing unit test (bottom of `testing.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use proto::worker::{AckStatus, EngineMessage};
    #[test]
    fn fake_worker_advances_the_fake_clock_and_resolves_ids() {
        let clock = FakeClock::new();
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let (mut w, state) = FakeWorker::scripted(clock.clone(), identity, vec![
            FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None }, FakeReply::Delay { ms: 700 }, FakeReply::Hang]);
        w.send(&EngineMessage::Cancel { id: "9".into(), target: "x".into() }).unwrap();
        w.send(&EngineMessage::Shutdown { id: "7".into() }).unwrap();
        assert_eq!(state.lock().unwrap().sent.len(), 2);
        assert!(matches!(w.recv(Duration::from_millis(100)).unwrap(), Some(WorkerMessage::Ack { .. })));
        assert!(w.recv(Duration::from_millis(500)).unwrap().is_none());
        assert_eq!(clock.now_ms(), 500);
        assert!(w.recv(Duration::from_millis(500)).unwrap().is_none());     // 200 ms of delay, then the hang eats the rest
        assert_eq!(clock.now_ms(), 1000);
        w.kill();
        assert_eq!(state.lock().unwrap().kills, 1);
    }
}
```

- [ ] **Step 2: Implement `testing.rs`**

```rust
use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::worker::link::{WorkerLink, WorkerLinkError};
use crate::EventSink;
use proto::worker::{AckStatus, EngineMessage, Ready, ResultStatus, Stage, StreetSolution, WorkerError, WorkerMessage, NodeStrategy, ADAPTER_VERSION, PROTO_VERSION, SOLVER_COMMIT};
use proto::{Action, EffectiveTree, RecommendationEvent};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub struct FakeClock { now: Mutex<u64>, cv: Condvar }
impl FakeClock {
    pub fn new() -> Arc<FakeClock> { Arc::new(FakeClock { now: Mutex::new(0), cv: Condvar::new() }) }
    pub fn advance_ms(&self, ms: u64) { let mut n = self.now.lock().unwrap(); *n += ms; self.cv.notify_all(); }
    pub fn set_ms(&self, t: u64) { let mut n = self.now.lock().unwrap(); *n = t; self.cv.notify_all(); }
}
impl Clock for FakeClock {
    fn now_ms(&self) -> u64 { *self.now.lock().unwrap() }
    fn wait_until(&self, t_ms: u64) { let mut n = self.now.lock().unwrap(); while *n < t_ms { n = self.cv.wait(n).unwrap(); } }
}

#[derive(Debug, Clone)]
pub enum IdRef { Last, Fixed(String) }
#[derive(Debug, Clone)]
pub enum FakeReply {
    Ack { id: IdRef, status: AckStatus, reason: Option<String> },
    Progress { id: IdRef, stage: Stage, iterations: u32, exploitability_chips: Option<f32>, elapsed_ms: u32 },
    Result { id: IdRef, status: ResultStatus, solution: Option<StreetSolution>, error: Option<WorkerError>, elapsed_ms: u32 },
    Delay { ms: u64 }, Eof, Malformed(String), Oversized(usize), Hang,
    /// Simulates a mutation arriving while the solve is live: the active identity is cancelled.
    /// `recv` consumes it and continues to the next scripted item in the SAME call, so a script that wants the
    /// client to observe the invalidation before the next reply must write `InvalidateIdentity, Delay { ms: 1 }, ...`;
    /// the `Delay` makes `recv` return `Ok(None)` and the receive loop re-check the identity (see Task 28).
    InvalidateIdentity,
}
#[derive(Debug, Default)]
pub struct FakeState { pub sent: Vec<EngineMessage>, pub kills: u32, pub restarts: u32, pub last_solve_id: Option<String>, pub cancels: Vec<String> }

pub struct FakeWorker { script: VecDeque<FakeReply>, state: Arc<Mutex<FakeState>>, clock: Arc<FakeClock>, identity: Arc<Mutex<IdentityState>>, ready: Ready }
impl FakeWorker {
    pub fn default_ready() -> Ready { Ready { proto_version: PROTO_VERSION, solver_commit: SOLVER_COMMIT.into(), adapter_version: ADAPTER_VERSION, threads: 16, build_features: vec!["avx2".into()], cpu_features: vec!["avx2".into()], capabilities: vec!["solve".into(), "lock".into(), "cancel".into(), "street_export".into(), "i16".into()] } }
    pub fn scripted(clock: Arc<FakeClock>, identity: Arc<Mutex<IdentityState>>, script: Vec<FakeReply>) -> (Box<FakeWorker>, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState::default()));
        (Box::new(FakeWorker { script: script.into(), state: state.clone(), clock, identity, ready: Self::default_ready() }), state)
    }
    fn id(&self, r: &IdRef) -> String { match r { IdRef::Fixed(s) => s.clone(), IdRef::Last => self.state.lock().unwrap().last_solve_id.clone().unwrap_or_default() } }
}
impl WorkerLink for FakeWorker {
    fn send(&mut self, msg: &EngineMessage) -> Result<(), WorkerLinkError> {
        let mut s = self.state.lock().unwrap();
        match msg { EngineMessage::Solve(r) => s.last_solve_id = Some(r.id.clone()), EngineMessage::Cancel { target, .. } => s.cancels.push(target.clone()), _ => {} }
        s.sent.push(msg.clone());
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<WorkerMessage>, WorkerLinkError> {
        let mut left = timeout.as_millis() as u64;
        loop {
            match self.script.front_mut() {
                None | Some(FakeReply::Hang) => { self.clock.advance_ms(left); return Ok(None); }
                Some(FakeReply::Delay { ms }) => {
                    let step = (*ms).min(left); *ms -= step; left -= step; self.clock.advance_ms(step);
                    if *ms == 0 { self.script.pop_front(); }
                    if left == 0 { return Ok(None); }
                }
                Some(FakeReply::InvalidateIdentity) => { self.identity.lock().unwrap().cancel_active(); self.script.pop_front(); }
                Some(_) => break,
            }
        }
        let reply = self.script.pop_front().unwrap();
        match reply {
            FakeReply::Eof => Err(WorkerLinkError::Eof),
            FakeReply::Malformed(s) => Err(WorkerLinkError::Protocol(s)),
            FakeReply::Oversized(n) => Err(WorkerLinkError::LineTooLong(n)),
            FakeReply::Ack { id, status, reason } => Ok(Some(WorkerMessage::Ack { id: self.id(&id), status, reason, replaced: None })),
            FakeReply::Progress { id, stage, iterations, exploitability_chips, elapsed_ms } => Ok(Some(WorkerMessage::Progress { id: self.id(&id), stage, iterations, exploitability_chips, elapsed_ms, memory_bytes: 1 })),
            FakeReply::Result { id, status, solution, error, elapsed_ms } => Ok(Some(WorkerMessage::Result { id: self.id(&id), status, elapsed_ms, solution, error })),
            FakeReply::Delay { .. } | FakeReply::Hang | FakeReply::InvalidateIdentity => unreachable!(),
        }
    }
    fn restart(&mut self) -> Result<(), WorkerLinkError> { self.state.lock().unwrap().restarts += 1; Ok(()) }
    fn kill(&mut self) { self.state.lock().unwrap().kills += 1; }
    fn ready(&self) -> Option<&Ready> { Some(&self.ready) }
}

#[derive(Debug, Clone)]
pub struct Recorded { pub at_ms: u64, pub kills: u32, pub event: RecommendationEvent }
pub struct RecordingSink { clock: Arc<dyn Clock>, state: Option<Arc<Mutex<FakeState>>>, events: Arc<Mutex<Vec<Recorded>>> }
impl RecordingSink {
    pub fn new(clock: Arc<dyn Clock>, state: Option<Arc<Mutex<FakeState>>>) -> (RecordingSink, Arc<Mutex<Vec<Recorded>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        (RecordingSink { clock, state, events: events.clone() }, events)
    }
}
impl EventSink for RecordingSink {
    fn emit(&mut self, ev: RecommendationEvent) {
        let kills = self.state.as_ref().map(|s| s.lock().unwrap().kills).unwrap_or(0);
        self.events.lock().unwrap().push(Recorded { at_ms: self.clock.now_ms(), kills, event: ev });
    }
}

/// A valid `StreetSolution` for every decision node of the tree's root street: all combos available, uniform probabilities, EV = action index in chips.
pub fn uniform_solution(tree: &EffectiveTree, requested: &[Action], exploitability_chips: f32) -> StreetSolution {
    let mut nodes = Vec::new();
    for n in tree.materialized.iter().filter(|n| n.street == tree.root_street) {
        let mut chip: Vec<Action> = Vec::new();
        for k in 0..n.path.len() { let parent = tree.materialized.iter().find(|m| m.path == n.path[..k]).unwrap(); chip.push(parent.actions[n.path[k] as usize].clone()); }
        let na = n.actions.len();
        nodes.push(NodeStrategy { path: chip, actor: n.actor.clone(), actions: n.actions.clone(), probs: vec![vec![1.0 / na as f32; na]; 1326],
            ev_chips: vec![(0..na).map(|a| a as f32).collect(); 1326], available: vec![true; 1326] });
    }
    let requested_index = nodes.iter().position(|n| n.path == requested).expect("requested path is a decision node") as u32;
    let covered_paths = nodes.iter().map(|n| n.path.clone()).collect();
    StreetSolution { nodes, requested: requested_index, exploitability_chips, iterations: 50, memory_bytes: 1 << 20, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths }
}
```

- [ ] **Step 3: Run and commit**

Run: `cargo test -p engine testing` then `cargo test --workspace --release`
Expected: PASS; the workspace stays green.

```bash
git add crates/engine
git commit -m "feat(engine): fake clock, scripted fake worker and recording sink for deterministic tests

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 20: Absolute deadlines and the independent watchdog

**Files:**
- Create: `crates/engine/src/deadline.rs`, `crates/engine/src/watchdog.rs`, `crates/engine/tests/watchdog.rs`
- Modify: `crates/engine/src/lib.rs` (add `pub mod deadline; pub mod watchdog;` and the temporary `assumptions_stub()` helper of step 1)
- Test: unit tests in `deadline.rs`; `crates/engine/tests/watchdog.rs::{watchdog_emits_final_at_delivery_minus_100ms, watchdog_disarm_retires_the_generation}`

**Interfaces:**
- Consumes: `clock::Clock`, `EventSink` (Task 18), `testing::{FakeClock, RecordingSink}` (Task 19); `proto::{Coverage, DecisionIdentity, Phase, Recommendation, RecommendationEvent, Street, UnsupportedReason}`.
- Produces: `deadline::{DELIVERY_MARGIN_MS, PIPE_MARGIN_MS, WATCHDOG_LEAD_MS, Deadlines { t0_ms, street_deadline_ms, final_delivery_ms, extraction_margin_ms }, Deadlines::for_request(t0_ms: u64, street: Street, flop_budget_s: u8) -> Deadlines, Deadlines::worker_deadline_ms(&self, now_ms: u64, until_ms: u64) -> Option<u32>, Deadlines::watchdog_fire_ms(&self) -> u64, street_budget_ms(Street, u8) -> u64, final_delivery_ms(Street, u8) -> u64, extraction_margin_ms(Street) -> u32, retry_admitted(now_ms, final_delivery_ms, p95_ms, extraction_margin_ms) -> bool}`; `watchdog::{SharedSink = Arc<Mutex<Box<dyn EventSink>>>, Armed { identity, street_deadline_ms, fire_ms, retained, fallback, stage, sink, delivered, terminal_seen, street_violation }, Watchdog::new(Arc<dyn Clock>) -> Watchdog, Watchdog::arm(&self, Armed), Watchdog::disarm(&self)}`; `crate::assumptions_stub() -> proto::Assumptions`.

- [ ] **Step 1: Failing tests**

Bottom of `crates/engine/src/deadline.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_margins_and_retry_admission_of_section_7() {
        let river = Deadlines::for_request(0, Street::River, 10);
        assert_eq!((river.street_deadline_ms, river.final_delivery_ms, river.extraction_margin_ms), (2_000, 15_000, 200));
        assert_eq!(river.watchdog_fire_ms(), 14_900);
        let turn = Deadlines::for_request(1_000, Street::Turn, 10);
        assert_eq!((turn.street_deadline_ms, turn.final_delivery_ms), (7_000, 16_000));
        // the flop budget stretches both the first-attempt deadline and the final delivery, and only on the flop
        let flop = Deadlines::for_request(0, Street::Flop, 30);
        assert_eq!((flop.street_deadline_ms, flop.final_delivery_ms, flop.extraction_margin_ms), (30_000, 35_000, 600));
        assert_eq!(Deadlines::for_request(0, Street::Flop, 10).final_delivery_ms, 15_000);
        // deadline_ms = remaining - 100 - 50 at send time; None when not even the extraction margin fits
        assert_eq!(river.worker_deadline_ms(500, 2_000), Some(1_350));
        assert_eq!(river.worker_deadline_ms(0, 15_000), Some(14_850));
        assert_eq!(river.worker_deadline_ms(1_800, 2_000), None);        // 50 <= 200
        assert_eq!(river.worker_deadline_ms(3_000, 2_000), None);        // already past
        // a retry is admitted only when the p95 of the retry template plus every margin still fits
        assert!(retry_admitted(6_200, 15_000, 6_000, 200));               // 8_800 >= 6_350
        assert!(!retry_admitted(9_000, 15_000, 6_000, 200));              // 6_000 <  6_350
    }
}
```

`crates/engine/tests/watchdog.rs`:

```rust
use engine::deadline::Deadlines;
use engine::testing::{FakeClock, RecordingSink};
use engine::watchdog::{Armed, SharedSink, Watchdog};
use proto::{Coverage, DecisionIdentity, EquitySummary, Phase, Recommendation, RecommendationEvent, Street, UnsupportedReason};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

fn identity() -> DecisionIdentity { DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 } }
fn fallback() -> Recommendation {
    Recommendation { identity: identity(), phase: Phase::Fast,
        coverage: Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage: String::new() }, partial: vec![] },
        legal: vec![], actions: vec![], unresolved_mass: 0.0, range_mix: None,
        equity: EquitySummary { hero_combo_vs_each: vec![], hero_range_vs_each: vec![], per_pot_shares: vec![] },
        assumptions: engine::assumptions_stub(), experimental: None, exploit: None }
}
fn armed(sink: SharedSink, stage: &str) -> (Armed, Arc<AtomicBool>, Arc<AtomicBool>, Arc<AtomicBool>) {
    let d = Deadlines::for_request(0, Street::River, 10);
    let (delivered, terminal_seen, violation) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(false)));
    let a = Armed { identity: identity(), street_deadline_ms: d.street_deadline_ms, fire_ms: d.watchdog_fire_ms(),
        retained: Arc::new(Mutex::new(None)), fallback: fallback(), stage: Arc::new(Mutex::new(stage.to_string())),
        sink, delivered: delivered.clone(), terminal_seen: terminal_seen.clone(), street_violation: violation.clone() };
    (a, delivered, terminal_seen, violation)
}
/// Spins on the fake clock (the watchdog thread is woken by `FakeClock::set_ms`, not by wall time).
fn wait_for(events: &Arc<Mutex<Vec<engine::testing::Recorded>>>, n: usize) -> Vec<engine::testing::Recorded> {
    for _ in 0..100_000 {
        let g = events.lock().unwrap();
        if g.len() >= n { return g.clone(); }
        drop(g);
        std::thread::yield_now();
    }
    panic!("watchdog did not emit {n} event(s)");
}

#[test]
fn watchdog_emits_final_at_delivery_minus_100ms() {
    let clock = FakeClock::new();
    let (sink, events) = RecordingSink::new(clock.clone(), None);
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let wd = Watchdog::new(clock.clone());
    let (a, delivered, _terminal, violation) = armed(sink, "extracting");
    wd.arm(a);
    clock.set_ms(14_900);
    let ev = wait_for(&events, 1);
    assert_eq!(ev.len(), 1, "exactly one Final");
    assert_eq!(ev[0].at_ms, 14_900);
    match &ev[0].event {
        RecommendationEvent::Final(r) => {
            assert_eq!(r.phase, Phase::Final);
            match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, "extracting"), c => panic!("{c:?}") }
        }
        e => panic!("{e:?}"),
    }
    assert!(delivered.load(Ordering::SeqCst));
    assert!(violation.load(Ordering::SeqCst), "no terminal was seen by the street deadline");
}

#[test]
fn watchdog_disarm_retires_the_generation() {
    let clock = FakeClock::new();
    let (sink, events) = RecordingSink::new(clock.clone(), None);
    let sink: SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let wd = Watchdog::new(clock.clone());
    let (a, _delivered, terminal, violation) = armed(sink, "solving");
    terminal.store(true, Ordering::SeqCst);
    wd.arm(a);
    wd.disarm();
    clock.set_ms(20_000);
    for _ in 0..100_000 { std::thread::yield_now(); }
    assert!(events.lock().unwrap().is_empty(), "a retired generation emits nothing");
    assert!(!violation.load(Ordering::SeqCst), "a terminal was seen before the street deadline");
}
```

The test needs an `Assumptions` value and `assemble` does not exist yet (Task 26). Add this one helper to `crates/engine/src/lib.rs`; Task 26's `assemble::empty_assumptions` is built on top of it and this stays as the zero value.

```rust
/// The zero `Assumptions` of §4.4: no ranges, no tree, nothing measured yet.
pub fn assumptions_stub() -> proto::Assumptions {
    proto::Assumptions { ranges_used: vec![], tree_signature: String::new(), template_id: String::new(), source: String::new(), source_accuracy: "unverified".into(),
        source_granularity: "1326 combos".into(), target_bp: 50, reached_bp: None, elapsed_ms: 0, cache: "miss".into(), translations: vec![], mappings: vec![], notes: vec![] }
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p engine --features testing deadline` and `cargo test -p engine --features testing --test watchdog`
Expected: FAIL to compile.

- [ ] **Step 3: Implement `deadline.rs` and `watchdog.rs`**

`crates/engine/src/deadline.rs` (above the tests):

```rust
use proto::Street;

pub const DELIVERY_MARGIN_MS: u64 = 100;
pub const PIPE_MARGIN_MS: u64 = 50;
pub const WATCHDOG_LEAD_MS: u64 = 100;

/// §7 street budgets: river 2 s, turn 6 s, flop `flop_budget_s`.
pub fn street_budget_ms(street: Street, flop_budget_s: u8) -> u64 { match street { Street::River => 2_000, Street::Turn => 6_000, Street::Flop => flop_budget_s as u64 * 1_000, Street::Preflop => 0 } }
/// §7 final delivery: 15 s for river and turn, `5 s + flop_budget_s` for flop decisions.
pub fn final_delivery_ms(street: Street, flop_budget_s: u8) -> u64 { match street { Street::Flop => 5_000 + flop_budget_s as u64 * 1_000, _ => 15_000 } }
pub fn extraction_margin_ms(street: Street) -> u32 { if street == Street::Flop { 600 } else { 200 } }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines { pub t0_ms: u64, pub street_deadline_ms: u64, pub final_delivery_ms: u64, pub extraction_margin_ms: u32 }
impl Deadlines {
    pub fn for_request(t0_ms: u64, street: Street, flop_budget_s: u8) -> Self {
        Self { t0_ms, street_deadline_ms: t0_ms + street_budget_ms(street, flop_budget_s), final_delivery_ms: t0_ms + final_delivery_ms(street, flop_budget_s), extraction_margin_ms: extraction_margin_ms(street) }
    }
    /// `deadline_ms = remaining - delivery_margin - pipe_margin` at send time; None when no iteration could fit.
    pub fn worker_deadline_ms(&self, now_ms: u64, until_ms: u64) -> Option<u32> {
        let d = until_ms.saturating_sub(now_ms).saturating_sub(DELIVERY_MARGIN_MS + PIPE_MARGIN_MS);
        if d <= self.extraction_margin_ms as u64 { None } else { Some(d as u32) }
    }
    pub fn watchdog_fire_ms(&self) -> u64 { self.final_delivery_ms - WATCHDOG_LEAD_MS }
}
/// §7: a retry is admitted only if `remaining >= p95(_min template) + margins`. Until a measured bench matrix
/// exists (plan 4 Task 21) the caller passes the street budget as the p95 proxy.
pub fn retry_admitted(now_ms: u64, final_delivery_ms: u64, p95_ms: u64, extraction_margin_ms: u32) -> bool {
    final_delivery_ms.saturating_sub(now_ms) >= p95_ms + DELIVERY_MARGIN_MS + PIPE_MARGIN_MS + extraction_margin_ms as u64
}
```

`crates/engine/src/watchdog.rs`:

```rust
use crate::clock::Clock;
use crate::EventSink;
use proto::{Coverage, DecisionIdentity, Phase, Recommendation, RecommendationEvent, UnsupportedReason};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub type SharedSink = Arc<Mutex<Box<dyn EventSink>>>;

pub struct Armed {
    pub identity: DecisionIdentity, pub street_deadline_ms: u64, pub fire_ms: u64,
    pub retained: Arc<Mutex<Option<Recommendation>>>,   // a Provisional or an earlier best_so_far (plan 4 fills it)
    pub fallback: Recommendation,                       // Unsupported{DeadlineExceeded{stage}} with the equity so far; stage filled at fire time
    pub stage: Arc<Mutex<String>>, pub sink: SharedSink,
    pub delivered: Arc<AtomicBool>, pub terminal_seen: Arc<AtomicBool>, pub street_violation: Arc<AtomicBool>,
}

/// §7: independent of the worker client. At the street deadline it records a violation when no first-attempt terminal
/// arrived; at `final delivery - 100 ms` it emits `Final` with the retained payload or `DeadlineExceeded`. `disarm` retires
/// the armed generation; a retired thread wakes at its times and does nothing.
pub struct Watchdog { clock: Arc<dyn Clock>, generation: Arc<AtomicU64> }
impl Watchdog {
    pub fn new(clock: Arc<dyn Clock>) -> Self { Self { clock, generation: Arc::new(AtomicU64::new(0)) } }
    pub fn arm(&self, a: Armed) {
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let (clock, generation) = (self.clock.clone(), self.generation.clone());
        std::thread::Builder::new().name("watchdog".into()).spawn(move || {
            clock.wait_until(a.street_deadline_ms);
            if generation.load(Ordering::SeqCst) == gen && !a.terminal_seen.load(Ordering::SeqCst) { a.street_violation.store(true, Ordering::SeqCst); }
            clock.wait_until(a.fire_ms);
            if generation.load(Ordering::SeqCst) != gen || a.delivered.swap(true, Ordering::SeqCst) { return; }
            let mut rec = a.retained.lock().unwrap().take().unwrap_or_else(|| a.fallback.clone());
            rec.phase = Phase::Final;
            if let Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } = &mut rec.coverage { *stage = a.stage.lock().unwrap().clone(); }
            a.sink.lock().unwrap().emit(RecommendationEvent::Final(rec));
        }).expect("watchdog thread");
    }
    pub fn disarm(&self) { self.generation.fetch_add(1, Ordering::SeqCst); }
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p engine --features testing` then `cargo test --workspace --release`
Expected: `budgets_margins_and_retry_admission_of_section_7`, `watchdog_emits_final_at_delivery_minus_100ms` and `watchdog_disarm_retires_the_generation` pass; the workspace stays green.

```bash
git add crates/engine
git commit -m "feat(engine): section 7 absolute deadlines and the worker-independent watchdog

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 21: Decision log (§5 step 10)

**Files:**
- Create: `crates/engine/src/log.rs`
- Modify: `crates/engine/src/lib.rs` (`pub mod log;`)

**Interfaces:**
- Consumes: `proto::{ApproxReason, Card, Coverage, DecisionIdentity, HandConfig, HandState, Rake, Seat, Street, TakenAction}`.
- Produces: `log::{DecisionRecord { identity, street, coverage, reasons: Vec<ApproxReason>, elapsed_ms: u32, cache: String, presolver_scenario: Option<String>, tier: Option<u8>, reached_bp: Option<u16>, street_violation: bool, final_violation: bool, template_id: String, input: InputRecord }, InputRecord { version: u16 (1), config: HandConfig, button: Seat, hero: Seat, dealt: Vec<Seat>, stacks_start: Vec<u32>, hero_cards: Option<[Card; 2]>, actions: Vec<TakenAction>, board: Vec<Card>, range_hashes: Vec<String> }, InputRecord::from_state(&HandState, range_hashes: Vec<String>) -> InputRecord, DecisionLog::open(dir: &Path) -> DecisionLog, DecisionLog::with_limits(dir, rotate_bytes: u64, keep_files: usize) -> DecisionLog, DecisionLog::append(&mut self, &DecisionRecord), ROTATE_BYTES = 50 MiB, KEEP_FILES = 10}`.
- **Why the log comes before `EngineCore`:** `EngineCore` owns a `DecisionLog`, so building the log first lets `EngineCore::new` take its four arguments from the first line it is written and never change arity afterwards (cross-plan section 4, green-workspace risk 1).

- [ ] **Step 1: Failing unit test (bottom of `log.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn hand_config() -> proto::HandConfig {
        proto::HandConfig { config_revision: 1, sb_chips: 5, bb_chips: 10, straddle: None,
            rake: proto::Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false }, chip_label: "$1".into() }
    }
    fn rec(i: u64) -> DecisionRecord {
        let hc = hand_config();
        DecisionRecord { identity: proto::DecisionIdentity { hand_id: i, hand_revision: 1, decision_id: i, config_revision: 1, model_revision: 0 }, street: proto::Street::River, coverage: proto::Coverage::Exact, reasons: vec![], elapsed_ms: 12, cache: "miss".into(), presolver_scenario: None, tier: None, reached_bp: Some(30), street_violation: false, final_violation: false, template_id: "river_std_v1".into(),
            input: InputRecord { version: 1, config: hc, button: proto::Seat(0), hero: proto::Seat(2), dealt: vec![], stacks_start: vec![], hero_cards: None, actions: vec![], board: vec![], range_hashes: vec!["x".repeat(64)] } }
    }
    #[test]
    fn appends_jsonl_and_rotates() {
        let dir = std::env::temp_dir().join(format!("pokerai_log_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = DecisionLog::with_limits(&dir, 2_000, 3);
        for i in 0..40 { log.append(&rec(i)); }
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"decisions.jsonl".to_string()) && names.contains(&"decisions.1.jsonl".to_string()));
        assert!(names.len() <= 3, "{names:?}");
        let first = std::fs::read_to_string(dir.join("decisions.jsonl")).unwrap();
        let last: DecisionRecord = serde_json::from_str(first.lines().last().unwrap()).unwrap();
        assert_eq!(last.identity.decision_id, 39);
    }
}
```

- [ ] **Step 2: Implement `log.rs`**

```rust
//! §5 step 10: `%LOCALAPPDATA%\PokerAI\decisions.jsonl`, rotated at 50 MiB, 10 files; a write failure is logged once per session.
use proto::{ApproxReason, Card, Coverage, DecisionIdentity, HandConfig, HandState, Seat, Street, TakenAction};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const ROTATE_BYTES: u64 = 50 << 20;
pub const KEEP_FILES: usize = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputRecord { pub version: u16, pub config: HandConfig, pub button: Seat, pub hero: Seat, pub dealt: Vec<Seat>, pub stacks_start: Vec<u32>, pub hero_cards: Option<[Card; 2]>, pub actions: Vec<TakenAction>, pub board: Vec<Card>, pub range_hashes: Vec<String> }
impl InputRecord {
    pub fn from_state(s: &HandState, range_hashes: Vec<String>) -> Self {
        Self { version: 1, config: s.config.clone(), button: s.button, hero: s.hero, dealt: s.dealt.clone(), stacks_start: s.stacks_start.clone(), hero_cards: s.hero_cards, actions: s.actions.clone(), board: s.board.clone(), range_hashes }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionRecord { pub identity: DecisionIdentity, pub street: Street, pub coverage: Coverage, pub reasons: Vec<ApproxReason>, pub elapsed_ms: u32, pub cache: String, pub presolver_scenario: Option<String>, pub tier: Option<u8>, pub reached_bp: Option<u16>, pub street_violation: bool, pub final_violation: bool, pub template_id: String, pub input: InputRecord }

pub struct DecisionLog { dir: PathBuf, rotate_bytes: u64, keep: usize, failed_once: bool }
impl DecisionLog {
    pub fn open(dir: &Path) -> Self { Self::with_limits(dir, ROTATE_BYTES, KEEP_FILES) }
    pub fn with_limits(dir: &Path, rotate_bytes: u64, keep: usize) -> Self { Self { dir: dir.to_path_buf(), rotate_bytes, keep, failed_once: false } }
    fn path(&self, k: usize) -> PathBuf { if k == 0 { self.dir.join("decisions.jsonl") } else { self.dir.join(format!("decisions.{k}.jsonl")) } }
    fn rotate(&self) -> std::io::Result<()> {
        let _ = std::fs::remove_file(self.path(self.keep - 1));
        for k in (1..self.keep).rev() { let from = self.path(k - 1); if from.exists() { std::fs::rename(&from, self.path(k))?; } }
        Ok(())
    }
    pub fn append(&mut self, rec: &DecisionRecord) {
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let current = self.path(0);
            if current.exists() && std::fs::metadata(&current)?.len() >= self.rotate_bytes { self.rotate()?; }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&current)?;
            f.write_all(serde_json::to_string(rec).map_err(std::io::Error::other)?.as_bytes())?;
            f.write_all(b"\n")
        })();
        if let Err(e) = result { if !self.failed_once { eprintln!("decision log write failed (further failures not reported): {e}"); self.failed_once = true; } }
    }
}
```

- [ ] **Step 3: Run and commit**

Run: `cargo test -p engine --features testing` then `cargo test --workspace --release`
Expected: `appends_jsonl_and_rotates` passes; the workspace stays green. Nothing outside `log.rs` changes: `EngineCore` (Task 22) takes the `DecisionLog` as its fourth constructor argument from the start.

```bash
git add crates/engine
git commit -m "feat(engine): rotating decisions.jsonl log with the versioned input record

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 22: `EngineCore` and the `run_solve` happy path

**Files:**
- Create: `crates/engine/src/core.rs`, `crates/engine/src/solve.rs`, `crates/engine/tests/solve_client.rs`
- Modify: `crates/engine/src/lib.rs` (add `pub mod core; pub mod solve;`)
- Test: `solve_client.rs::{deadline_arithmetic_and_request_fields, stale_ids_discarded}`

**Interfaces:**
- Consumes: Tasks 2, 4, 18, 19, 20, 21; `core_ranges::hash_scaled`; `proto::worker::validate_solution`.
- Produces: `core::{DEFAULT_MEMORY_LIMIT_BYTES, EngineCore { worker: Box<dyn WorkerLink>, clock: Arc<dyn Clock>, identity: Arc<Mutex<IdentityState>>, watchdog: Watchdog, log: DecisionLog, next_request_id: u64, memory_limit_bytes: u64, stage: Arc<Mutex<String>> }, EngineCore::new(Box<dyn WorkerLink>, Arc<dyn Clock>, Arc<Mutex<IdentityState>>, DecisionLog) -> Self, next_id(&mut self) -> String, identity_active(&self, &DecisionIdentity) -> bool, set_stage(&self, &str), reset_stage(&self, &str), stage(&self) -> String}`; `solve::{HEARTBEAT_MS, CANCEL_KILL_MS, SolvePlan { identity, deadlines, template_id, retry_template_id: Option<String>, rake: Rake, hero_actor: String, background: bool }, Terminal::{Ok, BestSoFar, Failed(UnsupportedReason)}, SolveOutcome { terminal, solution: Option<StreetSolution>, ordinal_paths, decision_path, tree: EffectiveTree, elapsed_ms: u32, template_used: String, street_violation: bool, restarts: u8, reached_bp: Option<u16> }, run_solve(&mut EngineCore, &SolveInput, &SolvePlan, &SharedSink) -> SolveOutcome, spot_hash(&EffectiveTree, pot: u32, board: &[Card], ranges: &[Range1326; 2]) -> String}`.
- **`EngineCore::new` takes four arguments from the start** (worker, clock, identity, log). The decision log is built in Task 21 precisely so this constructor never changes arity later (cross-plan section 4, green-workspace risk 1).
- **`SolvePlan.background` is a parameter, not a constant** (cross-plan R2): this plan only ever passes `false`, plan 4's pre-solver passes `true` with `deadline_ms: 600000`. The test below pins that it reaches the wire.

- [ ] **Step 1: Failing tests `crates/engine/tests/solve_client.rs`**

```rust
use engine::core::EngineCore;
use engine::deadline::Deadlines;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::solve::{run_solve, SolvePlan, Terminal};
use engine::testing::{uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use engine::tree::{build_tree_full, TemplateSelection};
use engine::watchdog::SharedSink;
use proto::worker::{AckStatus, EngineMessage, ResultStatus, Stage, WorkerError};
use proto::{Action, Card, Range1326, Rake, RecommendationEvent, Seat, SolveInput, Street, StreetRootSnapshot, UnsupportedReason};
use std::sync::{Arc, Mutex};

fn snap(street: Street) -> StreetRootSnapshot {
    let board = match street { Street::River => "Qs Jd 7h 3c 2d", _ => "Qs Jd 7h 3c" };
    StreetRootSnapshot { street, board: board.split(' ').map(|s| Card::parse(s).unwrap()).collect(), oop: Seat(2), ip: Seat(0), pot_root: 100,
        stack_oop_root: 100, stack_ip_root: 100, dead_this_street: 0, projected_from: 2, history: vec![], bb_chips: 2 }
}
fn full_range(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
struct Rig { core: EngineCore, input: SolveInput, plan: SolvePlan, sink: SharedSink, events: Arc<Mutex<Vec<engine::testing::Recorded>>>, state: Arc<Mutex<engine::testing::FakeState>>, clock: Arc<FakeClock> }
fn rig(street: Street, script: Vec<FakeReply>) -> Rig {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let id = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
    let (worker, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let core = EngineCore::new(worker, clock.clone(), identity, DecisionLog::open(&std::env::temp_dir().join("pokerai_solve_client_log")));
    let root = snap(street);
    let template = if street == Street::River { "river_std_v1" } else { "turn_std_v1" };
    let tree = build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)).unwrap().tree;
    let input = SolveInput { root: root.clone(), ranges: [full_range(&root.board), full_range(&root.board)], tree, target_bp: 50 };
    let plan = SolvePlan { identity: id, deadlines: Deadlines::for_request(0, street, 10), template_id: template.into(), retry_template_id: engine::tree::Templates::min_variant(template).map(String::from), rake: Rake::TimeCharge, hero_actor: "oop".into(), background: false };
    let (sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    Rig { core, input, plan, sink: Arc::new(Mutex::new(Box::new(sink))), events, state, clock }
}
fn ok_for(street: Street, template: &str, expl: f32) -> FakeReply {
    let root = snap(street);
    let tree = build_tree_full(&root, &TemplateSelection::from_history(template, &[])).unwrap().tree;
    FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(uniform_solution(&tree, &[], expl)), error: None, elapsed_ms: 5 }
}
fn ack() -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None } }
fn err(code: &str, retryable: bool, est: Option<u64>) -> FakeReply { FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(WorkerError { code: code.into(), message: code.into(), retryable, estimate_bytes: est }), elapsed_ms: 1 } }
fn solves(state: &Arc<Mutex<engine::testing::FakeState>>) -> Vec<proto::worker::SolveRequest> { state.lock().unwrap().sent.iter().filter_map(|m| if let EngineMessage::Solve(r) = m { Some(r.clone()) } else { None }).collect() }

#[test]
fn deadline_arithmetic_and_request_fields() {
    let mut r = rig(Street::River, vec![ack(), FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 10, exploitability_chips: Some(0.8), elapsed_ms: 3 }, ok_for(Street::River, "river_std_v1", 0.3)]);
    r.clock.set_ms(500);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!(out.terminal, Terminal::Ok);
    let req = &solves(&r.state)[0];
    assert_eq!((req.deadline_ms, req.extraction_margin_ms, req.memory_limit_bytes, req.background), (2000 - 500 - 150, 200, 10 << 30, false));
    assert_eq!((req.pot, req.stack_oop, req.stack_ip, req.rake_rate, req.rake_cap_mchips), (100, 100, 100, 0.0, 0));
    assert_eq!(req.spot.len(), 64);
    assert_eq!((out.reached_bp, out.template_used.as_str(), out.street_violation, out.restarts), (Some(30), "river_std_v1", false, 0));
    assert_eq!(out.ordinal_paths[out.solution.as_ref().unwrap().requested as usize], out.decision_path);
    let ev = r.events.lock().unwrap();
    assert!(matches!(&ev[0].event, RecommendationEvent::Progress { stage, iterations: 10, exploitability_pct: Some(p), .. } if stage == "solving" && (*p - 0.8).abs() < 1e-6));
    // the watchdog fires at t0 + 14.9 s for a river decision
    assert_eq!(r.plan.deadlines.watchdog_fire_ms(), 14_900);
    assert_eq!(Deadlines::for_request(0, Street::Turn, 10).street_deadline_ms, 6_000);
    drop(ev);
    // `background` is a request parameter, not an engine invariant: plan 4's pre-solver sends `true`
    let mut bg = rig(Street::River, vec![ack(), ok_for(Street::River, "river_std_v1", 0.3)]);
    bg.plan.background = true;
    assert_eq!(run_solve(&mut bg.core, &bg.input, &bg.plan, &bg.sink).terminal, Terminal::Ok);
    assert!(solves(&bg.state)[0].background);
}

#[test]
fn stale_ids_discarded() {
    let root = snap(Street::River);
    let tree = build_tree_full(&root, &TemplateSelection::from_history("river_std_v1", &[])).unwrap().tree;
    let stale = FakeReply::Result { id: IdRef::Fixed("old".into()), status: ResultStatus::Ok, solution: Some(uniform_solution(&tree, &[], 0.1)), error: None, elapsed_ms: 1 };
    let mut r = rig(Street::River, vec![ack(), stale, FakeReply::Progress { id: IdRef::Fixed("old".into()), stage: Stage::Solving, iterations: 3, exploitability_chips: None, elapsed_ms: 1 }, ok_for(Street::River, "river_std_v1", 0.3)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.reached_bp), (Terminal::Ok, Some(30)));
    assert!(r.events.lock().unwrap().is_empty(), "a stale progress is never forwarded");
    let _ = (Action::Check, UnsupportedReason::InvalidRanges, err("x", false, None));
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p engine --features testing --test solve_client`
Expected: FAIL to compile (`EngineCore`, `run_solve` missing).

- [ ] **Step 3: Implement `core.rs`**

```rust
use crate::clock::Clock;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::watchdog::Watchdog;
use crate::worker::link::WorkerLink;
use proto::DecisionIdentity;
use std::sync::{Arc, Mutex};

pub const DEFAULT_MEMORY_LIMIT_BYTES: u64 = 10 << 30;

/// The watchdog reports the FURTHEST stage a request reached, so `set_stage` only ever moves forward:
/// a `_min` retry that starts Building again must not rewind a stage the first attempt already reached (§7).
fn stage_rank(s: &str) -> u8 { match s { "building" => 1, "solving" => 2, "extracting" => 3, _ => 0 } }

/// State owned by `engine-main` (§3.4). Task 27 adds the snapshot store, the game config and the range source.
pub struct EngineCore {
    pub worker: Box<dyn WorkerLink>,
    pub clock: Arc<dyn Clock>,
    pub identity: Arc<Mutex<IdentityState>>,
    pub watchdog: Watchdog,
    pub log: DecisionLog,
    pub next_request_id: u64,
    pub memory_limit_bytes: u64,
    pub stage: Arc<Mutex<String>>,
}
impl EngineCore {
    pub fn new(worker: Box<dyn WorkerLink>, clock: Arc<dyn Clock>, identity: Arc<Mutex<IdentityState>>, log: DecisionLog) -> Self {
        Self { watchdog: Watchdog::new(clock.clone()), worker, clock, identity, log, next_request_id: 1, memory_limit_bytes: DEFAULT_MEMORY_LIMIT_BYTES, stage: Arc::new(Mutex::new("fast".into())) }
    }
    pub fn next_id(&mut self) -> String { let id = self.next_request_id; self.next_request_id += 1; id.to_string() }
    pub fn identity_active(&self, id: &DecisionIdentity) -> bool { self.identity.lock().unwrap().is_active(id) }
    /// Advances the reported stage; never rewinds it.
    pub fn set_stage(&self, s: &str) { let mut g = self.stage.lock().unwrap(); if stage_rank(s) > stage_rank(&g) { *g = s.to_string(); } }
    /// Starts a new request at `s`; only `serve_request` calls this, at admission.
    pub fn reset_stage(&self, s: &str) { *self.stage.lock().unwrap() = s.to_string(); }
    pub fn stage(&self) -> String { self.stage.lock().unwrap().clone() }
}
```

- [ ] **Step 4: Implement `solve.rs` (one attempt)**

Task 23 adds the heartbeat, the cancel-then-kill and the retry loop; everything else is final here.

```rust
use crate::core::EngineCore;
use crate::deadline::Deadlines;
use crate::tree::{build_tree_full, tree_signature, TemplateSelection, TreeBuild};
use crate::watchdog::SharedSink;
use crate::worker::link::WorkerLinkError;
use core_ranges::hash_scaled;
use proto::worker::{validate_solution, AckStatus, EngineMessage, ResultStatus, SolveRequest, Stage, StreetSolution, WorkerError, WorkerMessage};
use proto::{Card, DecisionIdentity, EffectiveTree, OrdinalPath, Rake, Range1326, RecommendationEvent, SolveInput, UnsupportedReason};
use sha2::{Digest, Sha256};
use std::time::Duration;

pub const HEARTBEAT_MS: u64 = 5_000;
pub const CANCEL_KILL_MS: u64 = 1_500;
const RESULT_GRACE_MS: u64 = 500;

#[derive(Debug, Clone)]
pub struct SolvePlan { pub identity: DecisionIdentity, pub deadlines: Deadlines, pub template_id: String, pub retry_template_id: Option<String>, pub rake: Rake, pub hero_actor: String, pub background: bool }
#[derive(Debug, Clone, PartialEq)]
pub enum Terminal { Ok, BestSoFar, Failed(UnsupportedReason) }
#[derive(Debug, Clone)]
pub struct SolveOutcome { pub terminal: Terminal, pub solution: Option<StreetSolution>, pub ordinal_paths: Vec<OrdinalPath>, pub decision_path: OrdinalPath, pub tree: EffectiveTree, pub elapsed_ms: u32, pub template_used: String, pub street_violation: bool, pub restarts: u8, pub reached_bp: Option<u16> }

pub fn spot_hash(tree: &EffectiveTree, pot: u32, board: &[Card], ranges: &[Range1326; 2]) -> String {
    let mut h = Sha256::new();
    h.update(tree_signature(tree, pot).as_bytes());
    for c in board { h.update([c.0]); }
    h.update(hash_scaled(&ranges[0])); h.update(hash_scaled(&ranges[1]));
    hex::encode(h.finalize())
}
fn stage_name(s: Stage) -> &'static str { match s { Stage::Building => "building", Stage::Solving => "solving", Stage::Extracting => "extracting" } }
fn engine_error(m: impl Into<String>, retryable: bool) -> UnsupportedReason { UnsupportedReason::EngineError { message: m.into(), retryable } }

pub(crate) enum AttemptEnd { Result { status: ResultStatus, solution: Option<StreetSolution>, error: Option<WorkerError> }, Exit(i32), Protocol(String), Heartbeat, Hang, Rejected(String), Superseded, DeadlinePassed }

pub(crate) fn request(core: &mut EngineCore, input: &SolveInput, plan: &SolvePlan, b: &TreeBuild, deadline_ms: u32) -> SolveRequest {
    let (rake_rate, rake_cap_mchips) = match plan.rake { Rake::PotRake { rate, cap_mchips, .. } => (rate, cap_mchips), Rake::TimeCharge => (0.0, 0) };
    SolveRequest { id: core.next_id(), spot: spot_hash(&b.tree, b.pot, &input.root.board, &input.ranges), board: input.root.board.clone(),
        oop_range: input.ranges[0].clone(), ip_range: input.ranges[1].clone(), pot: b.pot, stack_oop: input.root.stack_oop_root, stack_ip: input.root.stack_ip_root,
        rake_rate, rake_cap_mchips, tree: b.tree.clone(), history: b.history.clone(), target_bp: input.target_bp, deadline_ms,
        extraction_margin_ms: plan.deadlines.extraction_margin_ms, memory_limit_bytes: core.memory_limit_bytes, background: plan.background }
}

/// One send/receive cycle. Task 23 adds the heartbeat branch and the cancel-then-kill of a superseded request.
pub(crate) fn run_attempt(core: &mut EngineCore, plan: &SolvePlan, sink: &SharedSink, req: &SolveRequest) -> AttemptEnd {
    if let Err(e) = core.worker.send(&EngineMessage::Solve(req.clone())) { return match e { WorkerLinkError::Exit { code } => AttemptEnd::Exit(code), WorkerLinkError::Eof => AttemptEnd::Exit(-1), other => AttemptEnd::Protocol(other.to_string()) }; }
    let sent = core.clock.now_ms();
    let expected_by = sent + req.deadline_ms as u64 + 150 + RESULT_GRACE_MS;
    loop {
        if !core.identity_active(&plan.identity) { return AttemptEnd::Superseded; }
        let now = core.clock.now_ms();
        if now >= plan.deadlines.watchdog_fire_ms() { return AttemptEnd::DeadlinePassed; }
        if now >= expected_by { return AttemptEnd::Hang; }
        let wait = (expected_by - now).min(plan.deadlines.watchdog_fire_ms() - now);
        match core.worker.recv(Duration::from_millis(wait.max(1))) {
            Ok(Some(WorkerMessage::Ack { id, status, reason, .. })) if id == req.id => { if status == AckStatus::Rejected { return AttemptEnd::Rejected(reason.unwrap_or_default()); } }
            Ok(Some(WorkerMessage::Progress { id, stage, iterations, exploitability_chips, .. })) if id == req.id => {
                core.set_stage(stage_name(stage));
                sink.lock().unwrap().emit(RecommendationEvent::Progress { identity: plan.identity.clone(), stage: stage_name(stage).into(), iterations,
                    exploitability_pct: exploitability_chips.map(|c| 100.0 * c / req.pot as f32), elapsed_ms: (core.clock.now_ms() - plan.deadlines.t0_ms) as u32 });
            }
            Ok(Some(WorkerMessage::Result { id, status, solution, error, .. })) if id == req.id => {
                // A result that races a mutation is discarded, not validated (§4.4).
                if !core.identity_active(&plan.identity) { return AttemptEnd::Superseded; }
                return AttemptEnd::Result { status, solution, error };
            }
            Ok(_) => {}   // stale ids (a superseded request's replies) and stray lines are discarded
            Err(WorkerLinkError::Exit { code }) => return AttemptEnd::Exit(code),
            Err(WorkerLinkError::Eof) => return AttemptEnd::Exit(-1),
            Err(e) => return AttemptEnd::Protocol(e.to_string()),
        }
    }
}

pub(crate) fn validate(b: &TreeBuild, plan: &SolvePlan, sol: &StreetSolution) -> Result<Vec<OrdinalPath>, String> {
    let paths = validate_solution(sol, &b.tree.materialized)?;
    let r = sol.requested as usize;
    if paths.get(r) != Some(&b.decision_path) { return Err("requested node is not the decision node".into()); }
    if sol.nodes[r].actor != plan.hero_actor { return Err(format!("requested node actor {} is not hero's {}", sol.nodes[r].actor, plan.hero_actor)); }
    Ok(paths)
}

pub(crate) fn succeeded(core: &EngineCore, t_start: u64, b: &TreeBuild, template: &str, sol: StreetSolution, paths: Vec<OrdinalPath>, target_bp: u16, street_violation: bool, restarts: u8) -> SolveOutcome {
    let reached_bp = Some((10_000.0 * sol.exploitability_chips / b.pot as f32).round() as u16);
    let terminal = if sol.exploitability_chips / b.pot as f32 <= target_bp as f32 / 10_000.0 { Terminal::Ok } else { Terminal::BestSoFar };
    SolveOutcome { terminal, decision_path: b.decision_path.clone(), tree: b.tree.clone(), solution: Some(sol), ordinal_paths: paths,
        elapsed_ms: (core.clock.now_ms() - t_start) as u32, template_used: template.to_string(), street_violation, restarts, reached_bp }
}

pub(crate) fn failed(core: &EngineCore, t_start: u64, reason: UnsupportedReason, input: &SolveInput, template: &str, restarts: u8, street_violation: bool) -> SolveOutcome {
    SolveOutcome { terminal: Terminal::Failed(reason), solution: None, ordinal_paths: vec![], decision_path: vec![], tree: input.tree.clone(),
        elapsed_ms: (core.clock.now_ms() - t_start) as u32, template_used: template.to_string(), street_violation, restarts, reached_bp: None }
}

/// Maps a non-success attempt end to its §12 reason, whether a retry is allowed and whether the worker must be restarted.
pub(crate) fn classify(end: AttemptEnd) -> (UnsupportedReason, bool, bool) {
    match end {
        AttemptEnd::Result { status: ResultStatus::Cancelled, .. } => (engine_error("worker cancelled the job", true), true, false),
        AttemptEnd::Result { error: Some(e), .. } => match e.code.as_str() {
            "tree_mismatch" | "invalid_request" | "lock_mismatch" => (engine_error(format!("{}: {}", e.code, e.message), false), false, false),
            "no_iteration" => (UnsupportedReason::DeadlineExceeded { stage: "solving".into() }, true, false),
            "tree_too_large" | "out_of_memory" => (UnsupportedReason::TreeTooLarge { estimate_bytes: e.estimate_bytes.unwrap_or(0) }, true, false),
            _ => (engine_error(format!("{}: {}", e.code, e.message), e.retryable), true, false),
        },
        AttemptEnd::Result { .. } => (engine_error("result without solution or error", false), false, false),
        AttemptEnd::Exit(code) => (engine_error(format!("WorkerExit{{code: {code}}}"), true), true, true),
        AttemptEnd::Protocol(m) => (engine_error(format!("protocol error: {m}"), true), true, true),
        AttemptEnd::Heartbeat => (engine_error("no progress for 5 s during Solving", true), true, true),
        AttemptEnd::Hang => (engine_error("no terminal result by the worker deadline", true), true, true),
        AttemptEnd::Rejected(r) => (engine_error(format!("solve rejected: {r}"), true), false, false),
        AttemptEnd::Superseded => (engine_error("superseded by a newer request", false), false, false),
        AttemptEnd::DeadlinePassed => (UnsupportedReason::DeadlineExceeded { stage: "building".into() }, false, false),
    }
}

/// §5 step 7 / §7: one live solve with an absolute deadline. Task 23 wraps this in the retry loop.
pub fn run_solve(core: &mut EngineCore, input: &SolveInput, plan: &SolvePlan, sink: &SharedSink) -> SolveOutcome {
    let t_start = core.clock.now_ms();
    let template = plan.template_id.clone();
    let b = match build_tree_full(&input.root, &TemplateSelection::from_history(&template, &input.root.history)) { Ok(b) => b, Err(r) => return failed(core, t_start, r, input, &template, 0, false) };
    let now = core.clock.now_ms();
    let Some(deadline_ms) = plan.deadlines.worker_deadline_ms(now, plan.deadlines.street_deadline_ms) else { return failed(core, t_start, UnsupportedReason::DeadlineExceeded { stage: core.stage() }, input, &template, 0, false) };
    core.set_stage("building");
    let req = request(core, input, plan, &b, deadline_ms);
    let end = run_attempt(core, plan, sink, &req);
    if let AttemptEnd::Result { status: ResultStatus::Ok | ResultStatus::BestSoFar, solution: Some(sol), .. } = end {
        return match validate(&b, plan, &sol) {
            Ok(paths) => succeeded(core, t_start, &b, &template, sol, paths, input.target_bp, false, 0),
            Err(e) => failed(core, t_start, engine_error(format!("invalid solution: {e}"), false), input, &template, 0, false),
        };
    }
    let (reason, _retry, restart) = classify(end);
    if restart { let _ = core.worker.restart(); }
    failed(core, t_start, reason, input, &template, restart as u8, false)
}
```

- [ ] **Step 5: Run and commit**

Run: `cargo test -p engine --features testing --test solve_client` then `cargo test --workspace --release`
Expected: 2 passed; the workspace stays green.

```bash
git add crates/engine
git commit -m "feat(engine): EngineCore and the run_solve happy path with absolute deadlines and stale-id discard

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 23: `run_solve` resilience: heartbeat, cancel-then-kill, `_min` retry, error-code policy

**Files:**
- Modify: `crates/engine/src/solve.rs` (`run_attempt` gains the heartbeat branch and the superseded cancel; `run_solve` becomes the two-attempt loop), `crates/engine/tests/solve_client.rs`
- Test: `solve_client.rs::{superseded_request_cancels_then_kills_after_1_5s, heartbeat_failure_restarts_and_retries_min, error_codes_retry_policy}`

**Interfaces:**
- Consumes: Task 22's `solve.rs`; `deadline::{retry_admitted, street_budget_ms}`.
- Produces: `solve::cancel_or_kill(&mut EngineCore, target: &str)` and the final `run_solve` semantics. No new public types.

**Three corrections applied here (reviews B1, M3, M5):**
1. **Naming.** The loop variable is `attempt_no` and the function it calls is `run_attempt`. Binding a `u8` named `attempt` in the same scope as `fn attempt` shadows the function in the value namespace, so `attempt(...)` becomes "expected function, found u8" and the crate does not compile.
2. **Stage on retry.** `core.set_stage` only advances (Task 22's `core.rs`), so a retry that restarts at Building never rewinds a stage an earlier attempt reached. Case (c) of `final_delivery_independent_of_worker` (Task 29) therefore reports `extracting`, while case (b), which never left Building, still reports `building`.
3. **Street violation.** §7 defines it as "no first-attempt terminal arrived by `t0 + street budget`", not "the attempt ended after it". `run_solve` records `first_attempt_terminal` (set only when attempt 0 returned an `AttemptEnd::Result`) and evaluates `street_violation = !first_attempt_terminal && now >= street_deadline_ms` at the moment it returns.

- [ ] **Step 1: Failing tests (append to `crates/engine/tests/solve_client.rs`)**

```rust
#[test]
fn superseded_request_cancels_then_kills_after_1_5s() {
    // the mutation lands 100 ms after the ack; the client cancels, waits 1.5 s for result{cancelled}, then kills
    let mut r = rig(Street::River, vec![ack(), FakeReply::Delay { ms: 100 }, FakeReply::InvalidateIdentity, FakeReply::Hang]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(matches!(out.terminal, Terminal::Failed(UnsupportedReason::EngineError { ref message, .. }) if message.contains("superseded")));
    let s = r.state.lock().unwrap();
    assert_eq!((s.cancels.len(), s.kills, s.restarts), (1, 1, 1));
    assert!(r.clock.now_ms() >= 100 + 1500);
    assert!(r.events.lock().unwrap().is_empty());   // nothing is emitted for a superseded identity
}

#[test]
fn heartbeat_failure_restarts_and_retries_min() {
    // Progress at t = 1200 ms, then nothing for 5 s: at t = 6200 the heartbeat fires, the worker is killed and
    // respawned, and one retry with turn_min_v1 is admitted. 6200 is past the 6 s turn budget and the first attempt
    // never produced a terminal, so the street was violated even though the retry succeeds.
    let mut r = rig(Street::Turn, vec![ack(), FakeReply::Delay { ms: 1200 },
        FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 1, exploitability_chips: None, elapsed_ms: 1 },
        FakeReply::Delay { ms: 5100 }, FakeReply::Hang, ack(), ok_for(Street::Turn, "turn_min_v1", 0.4)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal.clone(), out.template_used.as_str(), out.restarts), (Terminal::Ok, "turn_min_v1", 1));
    let sent = solves(&r.state);
    assert_eq!((sent.len(), sent[1].tree.template_id.as_str()), (2, "turn_min_v1"));
    assert_eq!(r.clock.now_ms(), 6_200);
    // the retry gets only the time remaining to final delivery, minus the delivery and pipe margins
    assert_eq!(sent[1].deadline_ms, 15_000 - 6_200 - 150);
    assert!(out.street_violation, "no first-attempt terminal arrived before t0 + 6 s");
}

#[test]
fn error_codes_retry_policy() {
    // tree_too_large twice: TreeTooLarge with the retry's estimate
    let mut r = rig(Street::Turn, vec![ack(), err("tree_too_large", false, Some(9_000_000_000)), ack(), err("tree_too_large", false, Some(3_000_000_000))]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(matches!(out.terminal, Terminal::Failed(UnsupportedReason::TreeTooLarge { estimate_bytes: 3_000_000_000 })));
    assert_eq!(solves(&r.state).len(), 2);
    assert!(!out.street_violation, "the first attempt produced a terminal well inside the 6 s budget");
    // no_iteration: not retried on the same template, the _min template is tried
    let mut r = rig(Street::Turn, vec![ack(), err("no_iteration", false, None), ack(), ok_for(Street::Turn, "turn_min_v1", 0.4)]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert_eq!((out.terminal, out.template_used.as_str(), out.restarts), (Terminal::Ok, "turn_min_v1", 0));
    // tree_mismatch: never retried, non-retryable EngineError, exactly one solve
    let mut r = rig(Street::River, vec![ack(), err("tree_mismatch", false, None)]);
    assert!(matches!(run_solve(&mut r.core, &r.input, &r.plan, &r.sink).terminal, Terminal::Failed(UnsupportedReason::EngineError { retryable: false, .. })));
    assert_eq!(solves(&r.state).len(), 1);
    // worker exit: restart and retry with _min; a second failure is a retryable EngineError
    let mut r = rig(Street::River, vec![ack(), FakeReply::Eof, ack(), FakeReply::Eof]);
    let out = run_solve(&mut r.core, &r.input, &r.plan, &r.sink);
    assert!(matches!(out.terminal, Terminal::Failed(UnsupportedReason::EngineError { retryable: true, .. })));
    assert_eq!((out.restarts, r.state.lock().unwrap().restarts), (2, 2));
    // a rejected ack (busy) frees nothing by itself: the outcome is a retryable EngineError and no result was accepted
    let mut r = rig(Street::River, vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Rejected, reason: Some("busy".into()) }]);
    assert!(matches!(run_solve(&mut r.core, &r.input, &r.plan, &r.sink).terminal, Terminal::Failed(UnsupportedReason::EngineError { retryable: true, .. })));
}
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p engine --features testing --test solve_client`
Expected: FAIL (`run_solve` makes a single attempt and never cancels, restarts or retries).

- [ ] **Step 3: Add the heartbeat and the cancel-then-kill to `solve.rs`**

Add `use crate::deadline::{retry_admitted, street_budget_ms};` to the imports and replace `run_attempt`, adding `cancel_or_kill` above it:

```rust
/// Cancel-then-kill of §7/§12: send `cancel`, allow at most 1.5 s for `result{cancelled}`, then kill and restart.
pub(crate) fn cancel_or_kill(core: &mut EngineCore, target: &str) {
    let id = core.next_id();
    if core.worker.send(&EngineMessage::Cancel { id, target: target.to_string() }).is_err() { let _ = core.worker.restart(); return; }
    let until = core.clock.now_ms() + CANCEL_KILL_MS;
    loop {
        let now = core.clock.now_ms();
        if now >= until { core.worker.kill(); let _ = core.worker.restart(); return; }
        match core.worker.recv(Duration::from_millis(until - now)) {
            Ok(Some(WorkerMessage::Result { id, status: ResultStatus::Cancelled, .. })) if id == target => return,
            Ok(_) => continue,
            Err(_) => { let _ = core.worker.restart(); return; }
        }
    }
}

pub(crate) fn run_attempt(core: &mut EngineCore, plan: &SolvePlan, sink: &SharedSink, req: &SolveRequest) -> AttemptEnd {
    if let Err(e) = core.worker.send(&EngineMessage::Solve(req.clone())) { return match e { WorkerLinkError::Exit { code } => AttemptEnd::Exit(code), WorkerLinkError::Eof => AttemptEnd::Exit(-1), other => AttemptEnd::Protocol(other.to_string()) }; }
    let sent = core.clock.now_ms();
    let expected_by = sent + req.deadline_ms as u64 + 150 + RESULT_GRACE_MS;
    let (mut last_progress, mut solving) = (sent, false);
    loop {
        if !core.identity_active(&plan.identity) { cancel_or_kill(core, &req.id); return AttemptEnd::Superseded; }
        let now = core.clock.now_ms();
        if now >= plan.deadlines.watchdog_fire_ms() { return AttemptEnd::DeadlinePassed; }
        if solving && now - last_progress >= HEARTBEAT_MS { return AttemptEnd::Heartbeat; }
        if now >= expected_by { return AttemptEnd::Hang; }
        let mut wait = (expected_by - now).min(plan.deadlines.watchdog_fire_ms() - now);
        if solving { wait = wait.min(HEARTBEAT_MS - (now - last_progress)); }
        match core.worker.recv(Duration::from_millis(wait.max(1))) {
            Ok(Some(WorkerMessage::Ack { id, status, reason, .. })) if id == req.id => { if status == AckStatus::Rejected { return AttemptEnd::Rejected(reason.unwrap_or_default()); } }
            Ok(Some(WorkerMessage::Progress { id, stage, iterations, exploitability_chips, .. })) if id == req.id => {
                last_progress = core.clock.now_ms(); solving = stage == Stage::Solving; core.set_stage(stage_name(stage));
                sink.lock().unwrap().emit(RecommendationEvent::Progress { identity: plan.identity.clone(), stage: stage_name(stage).into(), iterations,
                    exploitability_pct: exploitability_chips.map(|c| 100.0 * c / req.pot as f32), elapsed_ms: (last_progress - plan.deadlines.t0_ms) as u32 });
            }
            Ok(Some(WorkerMessage::Result { id, status, solution, error, .. })) if id == req.id => {
                // A result that races a mutation is discarded, not validated (§4.4).
                if !core.identity_active(&plan.identity) { cancel_or_kill(core, &req.id); return AttemptEnd::Superseded; }
                return AttemptEnd::Result { status, solution, error };
            }
            Ok(_) => {}
            Err(WorkerLinkError::Exit { code }) => return AttemptEnd::Exit(code),
            Err(WorkerLinkError::Eof) => return AttemptEnd::Exit(-1),
            Err(e) => return AttemptEnd::Protocol(e.to_string()),
        }
    }
}
```

- [ ] **Step 4: Replace `run_solve` with the two-attempt loop**

```rust
/// §5 step 7 / §7 / §12: one live solve, absolute deadlines, heartbeat, cancel-then-kill, one `_min` retry under admission.
pub fn run_solve(core: &mut EngineCore, input: &SolveInput, plan: &SolvePlan, sink: &SharedSink) -> SolveOutcome {
    let t_start = core.clock.now_ms();
    let mut restarts = 0u8;
    let mut first_attempt_terminal = false;
    let mut template = plan.template_id.clone();
    // §7: the violation is "no FIRST-ATTEMPT terminal by the street deadline", evaluated when the solve returns.
    let violated = |core: &EngineCore, first: bool| !first && core.clock.now_ms() >= plan.deadlines.street_deadline_ms;
    let mut b = match build_tree_full(&input.root, &TemplateSelection::from_history(&template, &input.root.history)) { Ok(b) => b, Err(r) => return failed(core, t_start, r, input, &template, restarts, violated(core, first_attempt_terminal)) };
    for attempt_no in 0..2u8 {
        let until = if attempt_no == 0 { plan.deadlines.street_deadline_ms } else { plan.deadlines.final_delivery_ms };
        let now = core.clock.now_ms();
        let Some(deadline_ms) = plan.deadlines.worker_deadline_ms(now, until) else { return failed(core, t_start, UnsupportedReason::DeadlineExceeded { stage: core.stage() }, input, &template, restarts, violated(core, first_attempt_terminal)) };
        core.set_stage("building");                       // advances only; a retry never rewinds the reported stage
        let req = request(core, input, plan, &b, deadline_ms);
        let end = run_attempt(core, plan, sink, &req);
        if attempt_no == 0 && matches!(end, AttemptEnd::Result { .. }) { first_attempt_terminal = true; }
        if let AttemptEnd::Result { status: ResultStatus::Ok | ResultStatus::BestSoFar, solution: Some(sol), .. } = end {
            let sv = violated(core, first_attempt_terminal);
            return match validate(&b, plan, &sol) {
                Ok(paths) => succeeded(core, t_start, &b, &template, sol, paths, input.target_bp, sv, restarts),
                Err(e) => failed(core, t_start, engine_error(format!("invalid solution: {e}"), false), input, &template, restarts, sv),
            };
        }
        let deadline_passed = matches!(end, AttemptEnd::DeadlinePassed);
        let (reason, retry_allowed, restart) = classify(end);
        let sv = violated(core, first_attempt_terminal);
        if deadline_passed { return failed(core, t_start, UnsupportedReason::DeadlineExceeded { stage: core.stage() }, input, &template, restarts, sv); }
        if !retry_allowed && matches!(reason, UnsupportedReason::EngineError { ref message, .. } if message.contains("superseded")) { return failed(core, t_start, reason, input, &template, restarts, sv); }
        if restart { restarts += 1; if core.worker.restart().is_err() { return failed(core, t_start, engine_error("worker restart failed", false), input, &template, restarts, sv); } }
        let Some(retry) = plan.retry_template_id.as_deref().filter(|_| attempt_no == 0 && retry_allowed) else { return failed(core, t_start, reason, input, &template, restarts, sv) };
        let now = core.clock.now_ms();
        // §7 retry admission; the street budget stands in for the measured `_min` p95 until plan 4's matrix exists.
        let p95 = street_budget_ms(input.root.street, 10);
        if !retry_admitted(now, plan.deadlines.final_delivery_ms, p95, plan.deadlines.extraction_margin_ms) { return failed(core, t_start, reason, input, &template, restarts, sv); }
        template = retry.to_string();
        b = match build_tree_full(&input.root, &TemplateSelection::from_history(&template, &input.root.history)) { Ok(b) => b, Err(r) => return failed(core, t_start, r, input, &template, restarts, sv) };
    }
    let sv = violated(core, first_attempt_terminal);
    failed(core, t_start, engine_error("retry exhausted", true), input, &template, restarts, sv)
}
```

- [ ] **Step 5: Run and commit**

Run: `cargo test -p engine --features testing --test solve_client` then `cargo test --workspace --release`
Expected: 5 passed. In `heartbeat_failure_restarts_and_retries_min` the fake clock reaches 6 200 ms when the heartbeat fires, still inside the turn's 15 s final delivery, so the retry is admitted with `p95 = 6000`: `15000 - 6200 = 8800 >= 6000 + 350`.

```bash
git add crates/engine
git commit -m "feat(engine): run_solve heartbeat, cancel-then-kill, _min retry and the section 12 error-code policy

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 24: Coverage classifier (§6) and `coverage_classification_golden`

**Files:**
- Create: `crates/engine/src/coverage.rs`, `crates/engine/tests/coverage.rs`, `crates/engine/tests/golden/coverage_classification_golden.json`
- Modify: `crates/engine/src/lib.rs` (`pub mod coverage;`), `crates/engine/src/testing.rs` (hand builders)

**Interfaces:**
- Consumes: `core_model::{begin_hand, apply_action, set_board, derive, street_root, RootError, BeginHand}`; `proto::{HandState, Derived, StreetRootSnapshot, ApproxReason, UnsupportedReason, Street, Seat}`. `RootError` has the five resolved variants `{Multiway{pot_eligible: u8}, ProjectionNotReproducing{step: u32}, NoDecision, Preflop, Inconsistent{step: u32}}` (cross-plan M7 / spec S2) and `core_model::BeginHand` carries `hand_id` and `stacks_start` (cross-plan M6).
- Produces: `coverage::{Classification::{NoDecision{reason: String}, Preflop, HuStreet{root: StreetRootSnapshot, reasons: Vec<ApproxReason>, facing_allin: bool, opponent: Seat}, Multiway{pot_eligible: u8}, Unsupported(UnsupportedReason)}, classify(&HandState) -> Classification, pot_eligible(&Derived) -> u8, decision_point(&HandState, &Derived) -> Result<(), String>, seat_index(&HandState, Seat) -> usize}`; `testing::{cfg_1_2() -> (GameConfig, HandConfig), hand(dealt_stacks: &[(Seat, u32)], button: Seat, hero: Seat, hero_cards: Option<[Card; 2]>) -> HandState, play(state, actions: &[Action]) -> HandState, board(state, &str) -> HandState}`.

- [ ] **Step 1: Failing golden test `crates/engine/tests/coverage.rs`**

```rust
use engine::coverage::{classify, Classification};
use engine::testing::{board, hand, play};
use proto::{Action, ApproxReason, Card, Seat, UnsupportedReason};

fn label(c: &Classification) -> String {
    match c {
        Classification::NoDecision { .. } => "no_decision".into(),
        Classification::Preflop => "preflop".into(),
        Classification::Multiway { pot_eligible } => format!("multiway:{pot_eligible}"),
        Classification::Unsupported(UnsupportedReason::UnsupportedHistory { .. }) => "unsupported_history".into(),
        Classification::Unsupported(r) => format!("unsupported:{r:?}"),
        Classification::HuStreet { reasons, facing_allin, .. } => {
            let mut s = "hu_street".to_string();
            for r in reasons { if let ApproxReason::MultiwayStreetRoot { folded_this_street, dead_this_street } = r { s += &format!(":MultiwayStreetRoot{{{folded_this_street},{dead_this_street}}}"); } }
            if *facing_allin { s += ":facing_allin"; }
            s
        }
    }
}
fn c(s: &str) -> Card { Card::parse(s).unwrap() }
const B: Seat = Seat(0); const SB: Seat = Seat(1); const BB: Seat = Seat(2); const UTG: Seat = Seat(3); const HJ: Seat = Seat(4); const CO: Seat = Seat(5);
fn six(stacks: [u32; 6]) -> Vec<(Seat, u32)> { (0..6).map(|i| (Seat(i as u8), stacks[i])).collect() }
fn fold3(s: proto::HandState) -> proto::HandState { play(&s, &[Action::Fold, Action::Fold, Action::Fold]) }   // UTG, HJ, CO
fn r(to: u32) -> Action { Action::Raise { to } }
fn b(to: u32) -> Action { Action::Bet { to } }

#[test]
fn coverage_classification_golden() {
    let aa = Some([c("Ah"), c("Ad")]);
    let mut cases: Vec<(&str, String)> = Vec::new();
    // HU flop: three folds, BTN raises to 30, SB folds, BB (hero) calls
    let hu = board(&play(&fold3(hand(&six([1000; 6]), B, BB, aa)), &[r(30), Action::Fold, Action::Call]), "Kh 7d 2c");
    cases.push(("hu_flop", label(&classify(&hu))));
    // 3-way flop: CO raises, BTN calls, SB folds, BB calls
    let three = board(&play(&hand(&six([1000; 6]), B, BB, aa), &[Action::Fold, Action::Fold, r(30), Action::Call, Action::Fold, Action::Call]), "Kh 7d 2c");
    cases.push(("three_way_flop", label(&classify(&three))));
    // third player all-in: BTN (100 chips) shoves preflop, SB folds, BB calls, CO calls; BB to act on the flop with BTN all-in
    let mut st = six([1000; 6]); st[0].1 = 100;
    let allin3 = board(&play(&hand(&st, B, BB, aa), &[Action::Fold, Action::Fold, r(30), Action::AllIn { to: 100 }, Action::Fold, Action::Call, Action::Call]), "Kh 7d 2c");
    cases.push(("third_player_allin", label(&classify(&allin3))));
    // two preflop folds then HU flop: UTG folds, HJ raises, CO/BTN/SB fold, BB calls
    let hu2 = board(&play(&hand(&six([1000; 6]), B, BB, aa), &[Action::Fold, r(30), Action::Fold, Action::Fold, Action::Fold, Action::Call]), "Kh 7d 2c");
    cases.push(("two_folds_then_hu_flop", label(&classify(&hu2))));
    // §10.2 projection cases: three limpers (BTN, SB, BB) see the flop with 1,000 behind each; postflop order A = SB, B = BB, C = BTN
    let limped = |hero: Seat| board(&play(&hand(&six([1010; 6]), B, hero, aa), &[Action::Fold, Action::Fold, Action::Fold, Action::Call, Action::Call, Action::Check]), "Kh 7d 2c");
    cases.push(("projection_a_admitted_dead_0", label(&classify(&play(&limped(SB), &[b(50), Action::Fold, r(150)])))));
    cases.push(("projection_b_admitted_dead_50", label(&classify(&play(&limped(B), &[b(50), Action::Call, r(150), r(250), Action::Fold])))));
    cases.push(("projection_c_not_reproducible", label(&classify(&play(&limped(BB), &[b(50), Action::Call, r(150), Action::Fold])))));
    // opponent all-in on the flop; hero all-in; hero cards unknown
    cases.push(("opponent_allin", label(&classify(&play(&hu, &[Action::Check, Action::AllIn { to: 970 }])))));
    cases.push(("hero_allin", label(&classify(&play(&hu, &[Action::AllIn { to: 970 }])))));
    cases.push(("hero_cards_unknown", label(&classify(&board(&play(&fold3(hand(&six([1000; 6]), B, BB, None)), &[r(30), Action::Fold, Action::Call]), "Kh 7d 2c")))));
    // two dealt seats: FormatUnsupported at begin_hand
    let (_, hc) = engine::testing::cfg_1_2();
    let two = core_model::begin_hand(&hc, core_model::BeginHand { button: B, hero: SB, hero_cards: aa, dealt: vec![B, SB], stacks: vec![1000, 1000] });
    cases.push(("two_dealt_seats", match two { Err(e) => format!("format_unsupported:{}", e.to_string().contains("two dealt seats")), Ok(_) => "accepted".into() }));
    let _ = (UTG, HJ, CO);
    let expected: Vec<(String, String)> = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/coverage_classification_golden.json")).unwrap()).unwrap();
    assert_eq!(cases.iter().map(|(k, v)| (k.to_string(), v.clone())).collect::<Vec<_>>(), expected);
}
```

`crates/engine/tests/golden/coverage_classification_golden.json` (hand-checked from §6 and §10.2, written before the first run):

```json
[["hu_flop","hu_street"],["three_way_flop","multiway:3"],["third_player_allin","multiway:3"],["two_folds_then_hu_flop","hu_street"],
 ["projection_a_admitted_dead_0","hu_street:MultiwayStreetRoot{1,0}"],["projection_b_admitted_dead_50","hu_street:MultiwayStreetRoot{1,50}"],
 ["projection_c_not_reproducible","unsupported_history"],["opponent_allin","hu_street:facing_allin"],["hero_allin","no_decision"],
 ["hero_cards_unknown","no_decision"],["two_dealt_seats","format_unsupported:true"]]
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p engine --features testing --test coverage`
Expected: FAIL to compile.

- [ ] **Step 3: Implement the hand builders (append to `testing.rs`) and `coverage.rs`**

Append to `crates/engine/src/testing.rs`:

```rust
use core_model::{apply_action, begin_hand, derive, set_board, BeginHand};
use proto::{Card, GameConfig, HandConfig, HandState, Rake, Seat, SolverPrefs};
pub fn cfg_1_2() -> (GameConfig, HandConfig) {
    let rake = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    let hc = HandConfig { config_revision: 1, sb_chips: 5, bb_chips: 10, straddle: None, rake: rake.clone(), chip_label: "$1".into() };
    (GameConfig { config_revision: 1, chip_label: "$1".into(), sb_chips: 5, bb_chips: 10, straddle: None, rake, seats: vec![], solver: SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 } }, hc)
}
pub fn hand(dealt_stacks: &[(Seat, u32)], button: Seat, hero: Seat, hero_cards: Option<[Card; 2]>) -> HandState {
    let (_, hc) = cfg_1_2();
    begin_hand(&hc, BeginHand { button, hero, hero_cards, dealt: dealt_stacks.iter().map(|d| d.0).collect(), stacks: dealt_stacks.iter().map(|d| d.1).collect() }).expect("begin_hand")
}
/// Applies actions for whoever is to act, in order.
pub fn play(state: &HandState, actions: &[Action]) -> HandState { let mut s = state.clone(); for a in actions { s = apply_action(&s, a.clone()).unwrap_or_else(|e| panic!("{a:?}: {e}")); } s }
pub fn board(state: &HandState, cards: &str) -> HandState { set_board(state, &cards.split(' ').map(|c| Card::parse(c).unwrap()).collect::<Vec<_>>()).expect("set_board") }
pub fn derived(state: &HandState) -> proto::Derived { derive(state) }
```

`crates/engine/src/coverage.rs`:

```rust
use core_model::{derive, street_root, RootError};
use proto::{ApproxReason, Derived, HandState, Seat, Street, StreetRootSnapshot, UnsupportedReason};

#[derive(Debug, Clone)]
pub enum Classification {
    NoDecision { reason: String },
    Preflop,
    HuStreet { root: StreetRootSnapshot, reasons: Vec<ApproxReason>, facing_allin: bool, opponent: Seat },
    Multiway { pot_eligible: u8 },
    Unsupported(UnsupportedReason),
}

pub fn seat_index(state: &HandState, seat: Seat) -> usize { state.dealt.iter().position(|s| *s == seat).expect("dealt seat") }
/// §6: pot-eligible counting uses `Derived.folded` only; all-in players count.
pub fn pot_eligible(d: &Derived) -> u8 { d.folded.iter().filter(|f| !**f).count() as u8 }

/// §2 decision point: hero to act, two known hero cards, at least two legal actions.
pub fn decision_point(state: &HandState, d: &Derived) -> Result<(), String> {
    if !matches!(state.phase, proto::HandPhase::Betting { .. }) { return Err("hand is not in a betting phase".into()); }
    if d.to_act != Some(state.hero) { return Err("another seat is to act".into()); }
    if d.all_in[seat_index(state, state.hero)] { return Err("hero is all-in".into()); }
    if state.hero_cards.is_none() { return Err("hero cards unknown".into()); }
    if d.legal.len() < 2 { return Err("one legal action".into()); }
    Ok(())
}

pub fn classify(state: &HandState) -> Classification {
    let d = derive(state);
    if let Err(reason) = decision_point(state, &d) { return Classification::NoDecision { reason }; }
    if d.street == Street::Preflop { return Classification::Preflop; }
    let n = pot_eligible(&d);
    if n >= 3 { return Classification::Multiway { pot_eligible: n }; }
    match street_root(state) {
        Ok(root) => {
            let reasons = if root.projected_from >= 3 { vec![ApproxReason::MultiwayStreetRoot { folded_this_street: root.projected_from - 2, dead_this_street: root.dead_this_street }] } else { vec![] };
            let opponent = if root.oop == state.hero { root.ip } else { root.oop };
            let facing_allin = d.facing > 0 && d.all_in[seat_index(state, opponent)];
            Classification::HuStreet { root, reasons, facing_allin, opponent }
        }
        Err(RootError::ProjectionNotReproducing { step }) => Classification::Unsupported(UnsupportedReason::UnsupportedHistory { reason: format!("multiway street root not reproducible at step {step}") }),
        Err(RootError::Multiway { pot_eligible }) => Classification::Multiway { pot_eligible },
        Err(RootError::NoDecision) => Classification::NoDecision { reason: "no decision at the street root".into() },
        Err(RootError::Preflop) => Classification::Preflop,
        // §10.2: a genuine HU root that does not replay is an engine defect, not an unsupported history.
        Err(RootError::Inconsistent { step }) => Classification::Unsupported(UnsupportedReason::EngineError { message: format!("street root does not replay at step {step}"), retryable: false }),
    }
}
```

- [ ] **Step 4: Run and commit**

Run: `cargo test -p engine --features testing --test coverage`
Expected: PASS (if `core-model`'s error text for two dealt seats differs from "two dealt seats", match on `RulesError::FormatUnsupported { .. }` instead; the spec fixes the detail string).

```bash
git add crates/engine
git commit -m "feat(engine): section 6 coverage classifier with the projection cases golden

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 25: Equity adapter and the facing-all-in analytic fallback (`facing_allin_golden`)

**Files:**
- Create: `crates/engine/src/equity.rs`, `crates/engine/src/allin.rs`, `crates/engine/tests/facing_allin.rs`
- Modify: `crates/engine/src/lib.rs` (`pub mod equity; pub mod allin;`)

**Interfaces:**
- Consumes: `core_eval::{equity, EquityRequest, EquityMode, EquityResult}` (only `equity.rs`), `core_ranges::hero_conditioned`, `proto::{ActionAdvice, Availability, EquityEstimate, EquityMethod, EquitySummary, Rake, Range1326, Card, Seat}`.
- Produces: `equity::{hero_combo_equity(hero: [Card; 2], opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<(f32, EquityMethod)>, range_vs_range(hero_public: &Range1326, opp_public: &Range1326, board, budget, cancel) -> Option<(f32, EquityMethod)>, pending_summary(opponents: &[Seat]) -> EquitySummary, equity_summary(hero: Option<[Card; 2]>, hero_public: &Range1326, opponents: &[(Seat, Range1326)], board: &[Card], budget: Duration, cancel: &AtomicBool) -> EquitySummary, EQUITY_BUDGET_MS}`; `allin::{AllInInput { hero: [Card; 2], board: Vec<Card>, opp_public: Range1326, call_cost: u32, pot: u32, facing: u32, rake: Rake, bb_chips: u32 }, AllInAnswer { equity: f32, method: EquityMethod, w: u32, r: f32, ev_call_chips: f32, actions: Vec<ActionAdvice> }, facing_allin(&AllInInput, budget, cancel) -> Result<AllInAnswer, UnsupportedReason>}`.

- [ ] **Step 1: Failing test `crates/engine/tests/facing_allin.rs`**

```rust
use engine::allin::{facing_allin, AllInInput};
use proto::{combo_index, Action, Card, Rake, Range1326};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

fn c(s: &str) -> Card { Card::parse(s).unwrap() }
fn range(entries: &[(&str, &str, f32)]) -> Range1326 { let mut r = Range1326([0.0; 1326]); for (a, b, w) in entries { r.0[combo_index(c(a), c(b)) as usize] = *w; } r }
fn qq_54o(w54: f32) -> Range1326 {
    let mut e = vec![("Qc", "Qd", 1.0), ("Qc", "Qh", 1.0), ("Qd", "Qh", 1.0)];
    for f in ["5c", "5d", "5h", "5s"] { for g in ["4c", "4d", "4h", "4s"] { if f.as_bytes()[1] != g.as_bytes()[1] { e.push((f, g, w54)); } } }
    range(&e)
}
fn input(opp: Range1326, rake: Rake) -> AllInInput {
    AllInInput { hero: [c("Ah"), c("Ad")], board: "Qs Jd 7h 3c 2d".split(' ').map(c).collect(), opp_public: opp, call_cost: 73, pot: 173, facing: 73, rake, bb_chips: 5 }
}

#[test]
fn facing_allin_golden() {
    let cancel = AtomicBool::new(false);
    let unraked = Rake::TimeCharge;
    let raked = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    // T1: AhAd versus QQ + 54o at weight 1/12: equity 0.25, W = 246, EV(call) = -11.5 unraked, -12.75 with cap 5; -2.30 bb at a 5-chip BB
    let a = facing_allin(&input(qq_54o(1.0 / 12.0), unraked.clone()), Duration::from_secs(2), &cancel).unwrap();
    assert!((a.equity - 0.25).abs() < 1e-6 && a.w == 246 && a.r == 0.0);
    assert!((a.ev_call_chips + 11.5).abs() < 1e-3);
    let call = a.actions.iter().find(|x| x.action == Action::Call).unwrap();
    let fold = a.actions.iter().find(|x| x.action == Action::Fold).unwrap();
    assert!((call.ev_bb.unwrap() + 2.30).abs() < 1e-3 && fold.ev_bb == Some(0.0));
    assert_eq!((fold.frequency, call.frequency, fold.headline, call.headline), (Some(1.0), Some(0.0), true, false));
    let b = facing_allin(&input(qq_54o(1.0 / 12.0), raked.clone()), Duration::from_secs(2), &cancel).unwrap();
    assert!((b.r - 5.0).abs() < 1e-6 && (b.ev_call_chips + 12.75).abs() < 1e-3);
    // 54o at weight 0.25: equity 0.5, +50 unraked, +47.5 raked; call 100% and the headline
    let d = facing_allin(&input(qq_54o(0.25), unraked), Duration::from_secs(2), &cancel).unwrap();
    assert!((d.ev_call_chips - 50.0).abs() < 1e-3);
    assert!(d.actions.iter().find(|x| x.action == Action::Call).unwrap().headline);
    let e = facing_allin(&input(qq_54o(0.25), raked), Duration::from_secs(2), &cancel).unwrap();
    assert!((e.ev_call_chips - 47.5).abs() < 1e-3);
    // hero's strategic range is irrelevant here: only hero's actual combo enters; an opponent range with no compatible combo is InvalidRanges
    assert!(matches!(facing_allin(&input(range(&[("Ah", "Ad", 1.0)]), Rake::TimeCharge), Duration::from_secs(2), &cancel), Err(proto::UnsupportedReason::InvalidRanges)));
}
```

- [ ] **Step 2: Implement `equity.rs` and `allin.rs`**

`crates/engine/src/equity.rs`:

```rust
//! The only file that touches `core-eval`'s request type (assumed shape in the plan header).
use core_eval::{equity, EquityMode, EquityRequest};
use core_ranges::hero_conditioned;
use proto::{combo_index, Availability, Card, EquityEstimate, EquityMethod, EquitySummary, Range1326, Seat};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const EQUITY_BUDGET_MS: u64 = 500;
const EXACT_LIMIT: u64 = 20_000_000;   // §7: exact when pairs * runouts <= 2e7

fn choose(n: u64, k: u64) -> u64 { (0..k).fold(1u64, |acc, i| acc * (n - i) / (i + 1)) }
fn mode_for(pairs: u64, board_len: usize) -> EquityMode {
    let runouts = choose((52 - board_len - 4) as u64, (5 - board_len) as u64);
    if pairs * runouts <= EXACT_LIMIT { EquityMode::Exact } else { EquityMode::MonteCarlo { seed: 7 } }
}
fn run(hero: Range1326, opp: Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<(f32, EquityMethod)> {
    let pairs = hero.0.iter().filter(|w| **w > 0.0).count() as u64 * opp.0.iter().filter(|w| **w > 0.0).count() as u64;
    let res = equity(&EquityRequest { hero, opponents: vec![opp], board: board.to_vec(), mode: mode_for(pairs, board.len()) }, budget, cancel);
    res.hero_equity.map(|e| (e, res.method))
}

pub fn hero_combo_equity(hero: [Card; 2], opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<(f32, EquityMethod)> {
    let mut fixed = Range1326([0.0; 1326]);
    fixed.0[combo_index(hero[0], hero[1]) as usize] = 1.0;
    run(fixed, hero_conditioned(opp_public, hero), board, budget, cancel)
}
pub fn range_vs_range(hero_public: &Range1326, opp_public: &Range1326, board: &[Card], budget: Duration, cancel: &AtomicBool) -> Option<(f32, EquityMethod)> {
    run(hero_public.clone(), opp_public.clone(), board, budget, cancel)
}
fn estimate(r: Option<(f32, EquityMethod)>) -> EquityEstimate {
    match r { Some((v, m)) => EquityEstimate { value: Some(v), availability: Availability::Ready, method: Some(m) }, None => EquityEstimate { value: None, availability: Availability::Unavailable { reason: "no compatible holdings".into() }, method: None } }
}
pub fn pending_summary(opponents: &[Seat]) -> EquitySummary {
    let p = || EquityEstimate { value: None, availability: Availability::Pending, method: None };
    EquitySummary { hero_combo_vs_each: opponents.iter().map(|s| (*s, p())).collect(), hero_range_vs_each: opponents.iter().map(|s| (*s, p())).collect(), per_pot_shares: vec![] }
}
/// §4.4 populations: hero's actual combo versus each seat's hero-conditioned public range; hero's public range versus each public range.
pub fn equity_summary(hero: Option<[Card; 2]>, hero_public: &Range1326, opponents: &[(Seat, Range1326)], board: &[Card], budget: Duration, cancel: &AtomicBool) -> EquitySummary {
    let combo = opponents.iter().map(|(s, r)| (*s, estimate(hero.and_then(|h| hero_combo_equity(h, r, board, budget, cancel))))).collect();
    let range = opponents.iter().map(|(s, r)| (*s, estimate(range_vs_range(hero_public, r, board, budget, cancel)))).collect();
    EquitySummary { hero_combo_vs_each: combo, hero_range_vs_each: range, per_pot_shares: vec![] }
}
```

`crates/engine/src/allin.rs`:

```rust
//! §6 facing-an-all-in analytic fallback: `EV(call) = equity * (W - R) - C`, fold = 0, tie rule = call.
use crate::equity::hero_combo_equity;
use proto::{Action, ActionAdvice, Card, EquityMethod, Rake, Range1326, UnsupportedReason};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct AllInInput { pub hero: [Card; 2], pub board: Vec<Card>, pub opp_public: Range1326, pub call_cost: u32, pub pot: u32, pub facing: u32, pub rake: Rake, pub bb_chips: u32 }
#[derive(Debug, Clone)]
pub struct AllInAnswer { pub equity: f32, pub method: EquityMethod, pub w: u32, pub r: f32, pub ev_call_chips: f32, pub actions: Vec<ActionAdvice> }

pub fn facing_allin(inp: &AllInInput, budget: Duration, cancel: &AtomicBool) -> Result<AllInAnswer, UnsupportedReason> {
    let (equity, method) = hero_combo_equity(inp.hero, &inp.opp_public, &inp.board, budget, cancel).ok_or(UnsupportedReason::InvalidRanges)?;
    let c = inp.call_cost;
    // W: final matched pot after returning the uncalled excess (facing - C) and adding C
    let w = inp.pot + 2 * c - inp.facing;
    let r = match inp.rake { Rake::PotRake { rate, cap_mchips, .. } => (rate * w as f32).min(cap_mchips as f32 / 1000.0), Rake::TimeCharge => 0.0 };
    let ev_call_chips = equity * (w as f32 - r) - c as f32;
    let call_wins = ev_call_chips >= 0.0;   // spec decision: EV(call) == 0 -> call
    let bb = inp.bb_chips as f32;
    let actions = vec![
        ActionAdvice { action: Action::Fold, frequency: Some(if call_wins { 0.0 } else { 1.0 }), ev_bb: Some(0.0), unavailable: None, headline: !call_wins },
        ActionAdvice { action: Action::Call, frequency: Some(if call_wins { 1.0 } else { 0.0 }), ev_bb: Some(ev_call_chips / bb), unavailable: None, headline: call_wins },
    ];
    Ok(AllInAnswer { equity, method, w, r, ev_call_chips, actions })
}
```

- [ ] **Step 3: Run and commit**

Run: `cargo test -p engine --features testing --test facing_allin`
Expected: PASS (T1 numbers within 1e-3).

```bash
git add crates/engine
git commit -m "feat(engine): equity adapter and the facing-all-in analytic fallback with the T1 golden

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 26: Result assembly, headline rules, reason accumulation, equity merge (`recommendation_assembly_golden`)

**Files:**
- Create: `crates/engine/src/assemble.rs`, `crates/engine/tests/assembly.rs`, `crates/engine/tests/golden/recommendation_assembly_golden.json` (recorded on the first green run)
- Modify: `crates/engine/src/lib.rs` (`pub mod assemble;`)

**Interfaces:**
- Consumes: `proto::*`, `proto::worker::NodeStrategy`.
- Produces: `assemble::{AssemblyCtx { identity: DecisionIdentity, legal: Vec<LegalAction>, hero_combo: Option<ComboIndex>, bb_chips: u32, equity: EquitySummary }, HeadlineSource::{Solved, Chart, PokerDataUnverified}, accumulate(Coverage, Vec<ApproxReason>) -> Coverage, coverage_for_solve(exploitability_chips: f32, pot: u32, target_bp: u16, best_so_far: bool, inherited: Vec<ApproxReason>) -> Coverage, headline(&mut [ActionAdvice], unresolved_mass: f32, HeadlineSource) -> Option<String>, hero_reach(nodes: &[NodeStrategy], ordinal_paths: &[OrdinalPath], requested: usize, hero_public: &Range1326, hero_actor: &str) -> Vec<f32>, range_mix(&NodeStrategy, reach: &[f32]) -> Vec<(Action, f32)>, map_to_legal(&Action, &[LegalAction]) -> Option<Action>, final_from_solution(&AssemblyCtx, node: &NodeStrategy, reach: &[f32], coverage: Coverage, assumptions: Assumptions) -> Recommendation, unsupported(&AssemblyCtx, UnsupportedReason, partial: Vec<ApproxReason>, Assumptions) -> Recommendation, fast(&AssemblyCtx, coverage_so_far: Coverage, Assumptions) -> Recommendation, merge_equity(&mut Recommendation, &EquitySummary), empty_assumptions(template_id: &str) -> Assumptions}`.

- [ ] **Step 1: Failing test `crates/engine/tests/assembly.rs`**

```rust
use engine::assemble::*;
use proto::worker::NodeStrategy;
use proto::*;

fn ctx() -> AssemblyCtx {
    let id = DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 };
    AssemblyCtx { identity: id, legal: vec![LegalAction::Fold, LegalAction::Call { cost: 50 }, LegalAction::Raise { min_to: 100, max_to: 500 }], hero_combo: Some(combo_index(Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap())), bb_chips: 10, equity: engine::equity::pending_summary(&[Seat(0)]) }
}
fn node(rows: &[(usize, [f32; 3], [f32; 3])]) -> NodeStrategy {
    let mut n = NodeStrategy { path: vec![Action::Bet { to: 50 }], actor: "ip".into(), actions: vec![Action::Fold, Action::Call, Action::Raise { to: 150 }], probs: vec![vec![0.0; 3]; 1326], ev_chips: vec![vec![0.0; 3]; 1326], available: vec![false; 1326] };
    for (c, p, e) in rows { n.probs[*c] = p.to_vec(); n.ev_chips[*c] = e.to_vec(); n.available[*c] = true; }
    n
}
fn adv(a: Action, f: Option<f32>, ev: Option<f32>, u: Option<Unavailable>) -> ActionAdvice { ActionAdvice { action: a, frequency: f, ev_bb: ev, unavailable: u, headline: false } }
fn who(actions: &[ActionAdvice]) -> Option<Action> { actions.iter().find(|a| a.headline).map(|a| a.action.clone()) }

#[test]
fn recommendation_assembly_golden() {
    let c = ctx();
    let hero = c.hero_combo.unwrap() as usize;
    let mut record = serde_json::Map::new();
    // (1) headline only with complete EVs: highest EV, fold = 0
    let n = node(&[(hero, [0.2, 0.3, 0.5], [0.0, 12.5, 30.0])]);
    let reach = vec![1.0f32; 1326];
    let rec = final_from_solution(&c, &n, &reach, Coverage::Exact, empty_assumptions("river_std_v1"));
    assert_eq!(who(&rec.actions), Some(Action::Raise { to: 150 }));
    assert!(rec.assumptions.notes.iter().any(|s| s == "headline: highest EV"));
    assert_eq!((rec.actions[1].ev_bb, rec.actions[0].ev_bb, rec.unresolved_mass), (Some(1.25), Some(0.0), 0.0));
    record.insert("complete_ev".into(), serde_json::to_value(&rec.actions).unwrap());
    // tie break: equal EV -> higher frequency, then earlier in menu order
    let mut t = vec![adv(Action::Fold, Some(0.2), Some(0.0), None), adv(Action::Call, Some(0.3), Some(3.0), None), adv(Action::Raise { to: 150 }, Some(0.5), Some(3.0), None)];
    assert_eq!(headline(&mut t, 0.0, HeadlineSource::Solved).as_deref(), Some("highest EV"));
    assert_eq!(who(&t), Some(Action::Raise { to: 150 }));
    t[1].frequency = Some(0.5);
    headline(&mut t, 0.0, HeadlineSource::Solved);
    assert_eq!(who(&t), Some(Action::Call));
    // (2) no EV headline when one action lacks EV; frequency headline wording per source
    let mut u = vec![adv(Action::Fold, Some(0.1), Some(0.0), None), adv(Action::Call, Some(0.6), None, Some(Unavailable::NoEvReference)), adv(Action::Raise { to: 150 }, Some(0.3), Some(9.0), None)];
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::Solved), None);
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::Chart).as_deref(), Some("highest-frequency chart action"));
    assert_eq!(who(&u), Some(Action::Call));
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::PokerDataUnverified).as_deref(), Some("highest-frequency source action, EV reference unverified"));
    u[1].unavailable = Some(Unavailable::BranchSupportIncomplete { covered_posterior: 0.2 });
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::PokerDataUnverified).as_deref(), Some("highest-frequency action, EV incomplete"));
    // (3) no headline of any kind when unresolved_mass > 0
    let mut v = vec![adv(Action::Fold, Some(0.3), Some(0.0), None), adv(Action::Call, Some(0.5), Some(2.0), None)];
    assert_eq!(headline(&mut v, 0.2, HeadlineSource::Chart), None);
    assert!(v.iter().all(|a| !a.headline));
    // reason accumulation: never removed, Exact upgrades to Approximate, Unsupported keeps partial
    let acc = accumulate(Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] }, vec![ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 }]);
    assert!(matches!(&acc, Coverage::Approximate { reasons } if reasons.len() == 2));
    assert!(matches!(accumulate(acc.clone(), vec![]), Coverage::Approximate { .. }));
    assert!(matches!(accumulate(Coverage::Exact, vec![]), Coverage::Exact));
    assert!(matches!(coverage_for_solve(0.3, 100, 50, false, vec![]), Coverage::Exact));
    assert!(matches!(coverage_for_solve(1.9, 100, 50, true, vec![]), Coverage::Approximate { reasons } if reasons == vec![ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 }]));
    let un = unsupported(&c, UnsupportedReason::MultiwayEv { pot_eligible: 3 }, vec![ApproxReason::ChartRounded], empty_assumptions(""));
    assert!(matches!(&un.coverage, Coverage::Unsupported { partial, .. } if partial == &vec![ApproxReason::ChartRounded]));
    assert!(un.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none()) && un.actions.len() == 3);
    // range_mix present under HeroComboOutOfSupport; frequencies absent
    let other = combo_index(Card::parse("Kh").unwrap(), Card::parse("Kd").unwrap()) as usize;
    let n2 = node(&[(other, [0.0, 1.0, 0.0], [0.0, 5.0, 0.0])]);
    let rec2 = final_from_solution(&c, &n2, &reach, Coverage::Exact, empty_assumptions("river_std_v1"));
    assert!(matches!(rec2.coverage, Coverage::Unsupported { reason: UnsupportedReason::HeroComboOutOfSupport, .. }));
    assert_eq!(rec2.range_mix.as_ref().unwrap()[1], (Action::Call, 1.0));
    assert!(rec2.actions.iter().all(|a| a.frequency.is_none() && matches!(a.unavailable, Some(Unavailable::HeroOutOfSupport))));
    // an Equity event after Final enriches it; Pending never replaces Ready
    let mut fin = rec.clone();
    let ready = EquitySummary { hero_combo_vs_each: vec![(Seat(0), EquityEstimate { value: Some(0.61), availability: Availability::Ready, method: Some(EquityMethod::Exact) })], hero_range_vs_each: vec![(Seat(0), EquityEstimate { value: None, availability: Availability::Pending, method: None })], per_pot_shares: vec![] };
    merge_equity(&mut fin, &ready);
    assert_eq!(fin.equity.hero_combo_vs_each[0].1.value, Some(0.61));
    merge_equity(&mut fin, &engine::equity::pending_summary(&[Seat(0)]));
    assert_eq!(fin.equity.hero_combo_vs_each[0].1.value, Some(0.61));
    record.insert("out_of_support".into(), serde_json::to_value(&rec2).unwrap());
    golden::check("recommendation_assembly_golden", &serde_json::Value::Object(record));
}
mod golden {
    pub fn check(name: &str, value: &serde_json::Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.json"));
        if std::env::var("UPDATE_GOLDEN").is_ok() || !path.exists() { std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap(); }
        let stored: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(&stored, value, "golden {name} differs");
    }
}
```

- [ ] **Step 2: Implement `assemble.rs`**

```rust
use proto::worker::NodeStrategy;
use proto::*;

pub struct AssemblyCtx { pub identity: DecisionIdentity, pub legal: Vec<LegalAction>, pub hero_combo: Option<ComboIndex>, pub bb_chips: u32, pub equity: EquitySummary }
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeadlineSource { Solved, Chart, PokerDataUnverified }

/// §4.4: reasons accumulate; an Exact base with reasons becomes Approximate; a later exact result never removes inherited reasons.
pub fn accumulate(base: Coverage, more: Vec<ApproxReason>) -> Coverage {
    let push = |v: &mut Vec<ApproxReason>| { for r in more.iter() { if !v.iter().any(|x| format!("{x:?}") == format!("{r:?}")) { v.push(r.clone()); } } };
    match base {
        Coverage::Exact => { let mut v = Vec::new(); push(&mut v); if v.is_empty() { Coverage::Exact } else { Coverage::Approximate { reasons: v } } }
        Coverage::Approximate { mut reasons } => { push(&mut reasons); Coverage::Approximate { reasons } }
        Coverage::Unsupported { reason, mut partial } => { push(&mut partial); Coverage::Unsupported { reason, partial } }
    }
}
/// `Exact` iff raw `exploitability / P <= target_bp / 10000` and no reasons at all (§5 step 7); a best_so_far adds `DeadlineBestSoFar`.
pub fn coverage_for_solve(exploitability_chips: f32, pot: u32, target_bp: u16, best_so_far: bool, inherited: Vec<ApproxReason>) -> Coverage {
    let mut reasons = inherited;
    if best_so_far { reasons.push(ApproxReason::DeadlineBestSoFar { reached_bp: (10_000.0 * exploitability_chips / pot as f32).round() as u16, target_bp }); }
    let at_target = exploitability_chips / pot as f32 <= target_bp as f32 / 10_000.0;
    if at_target && reasons.is_empty() { Coverage::Exact } else { accumulate(Coverage::Approximate { reasons: vec![] }, reasons) }
}

pub fn headline(actions: &mut [ActionAdvice], unresolved_mass: f32, source: HeadlineSource) -> Option<String> {
    for a in actions.iter_mut() { a.headline = false; }
    if actions.is_empty() || unresolved_mass > 0.0 { return None; }
    if actions.iter().all(|a| a.ev_bb.is_some()) {
        let mut best = 0;
        for i in 1..actions.len() {
            let (a, b) = (&actions[i], &actions[best]);
            if a.ev_bb > b.ev_bb || (a.ev_bb == b.ev_bb && a.frequency.unwrap_or(0.0) > b.frequency.unwrap_or(0.0)) { best = i; }
        }
        actions[best].headline = true;
        return Some("highest EV".into());
    }
    if actions.iter().all(|a| a.frequency.is_some()) {
        let incomplete = actions.iter().any(|a| matches!(a.unavailable, Some(Unavailable::BranchSupportIncomplete { .. })));
        let label = if incomplete { "highest-frequency action, EV incomplete" } else { match source { HeadlineSource::Chart => "highest-frequency chart action", HeadlineSource::PokerDataUnverified => "highest-frequency source action, EV reference unverified", HeadlineSource::Solved => return None } };
        let mut best = 0;
        for i in 1..actions.len() { if actions[i].frequency > actions[best].frequency { best = i; } }
        actions[best].headline = true;
        return Some(label.into());
    }
    None
}

/// Hero's reach at the requested node: root public weights times hero's own earlier action probabilities on the path (exported nodes).
pub fn hero_reach(nodes: &[NodeStrategy], ordinal_paths: &[OrdinalPath], requested: usize, hero_public: &Range1326, hero_actor: &str) -> Vec<f32> {
    let mut reach: Vec<f32> = hero_public.0.to_vec();
    let target = &ordinal_paths[requested];
    for k in 0..target.len() {
        if let Some(i) = ordinal_paths.iter().position(|p| p[..] == target[..k]) {
            if nodes[i].actor == hero_actor { let a = target[k] as usize; for c in 0..1326 { reach[c] *= nodes[i].probs[c][a]; } }
        }
    }
    reach
}
pub fn range_mix(node: &NodeStrategy, reach: &[f32]) -> Vec<(Action, f32)> {
    let mut mass = 0.0f32;
    let mut acc = vec![0.0f32; node.actions.len()];
    for c in 0..1326 { if node.available[c] && reach[c] > 0.0 { mass += reach[c]; for a in 0..acc.len() { acc[a] += reach[c] * node.probs[c][a]; } } }
    node.actions.iter().cloned().zip(acc.into_iter().map(|x| if mass > 0.0 { x / mass } else { 0.0 })).collect()
}
/// Tree actions carry the tree's chips; hero's real all-in amount comes from `Derived.legal`.
pub fn map_to_legal(a: &Action, legal: &[LegalAction]) -> Option<Action> {
    match a {
        Action::Fold => legal.iter().any(|l| matches!(l, LegalAction::Fold)).then(|| a.clone()),
        Action::Check => legal.iter().any(|l| matches!(l, LegalAction::Check)).then(|| a.clone()),
        Action::Call => legal.iter().any(|l| matches!(l, LegalAction::Call { .. })).then(|| a.clone()),
        Action::Bet { to } => legal.iter().find_map(|l| match l { LegalAction::Bet { min_to, max_to } if to >= min_to && to <= max_to => Some(a.clone()), _ => None }),
        Action::Raise { to } => legal.iter().find_map(|l| match l { LegalAction::Raise { min_to, max_to } if to >= min_to && to <= max_to => Some(a.clone()), _ => None }),
        Action::AllIn { .. } => legal.iter().find_map(|l| match l { LegalAction::AllIn { to } => Some(Action::AllIn { to: *to }), _ => None }),
    }
}
pub fn empty_assumptions(template_id: &str) -> Assumptions {
    Assumptions { ranges_used: vec![], tree_signature: String::new(), template_id: template_id.into(), source: "solver-worker".into(), source_accuracy: "unverified".into(), source_granularity: "1326 combos".into(),
        target_bp: 50, reached_bp: None, elapsed_ms: 0, cache: "miss".into(), translations: vec![], mappings: vec![], notes: vec![] }
}
fn base(ctx: &AssemblyCtx, phase: Phase, coverage: Coverage, actions: Vec<ActionAdvice>, unresolved_mass: f32, range_mix: Option<Vec<(Action, f32)>>, assumptions: Assumptions) -> Recommendation {
    Recommendation { identity: ctx.identity.clone(), phase, coverage, legal: ctx.legal.clone(), actions, unresolved_mass, range_mix, equity: ctx.equity.clone(), assumptions, experimental: None, exploit: None }
}
fn legal_menu(ctx: &AssemblyCtx, u: Option<Unavailable>) -> Vec<ActionAdvice> {
    ctx.legal.iter().map(|l| { let a = match l { LegalAction::Fold => Action::Fold, LegalAction::Check => Action::Check, LegalAction::Call { .. } => Action::Call, LegalAction::Bet { min_to, .. } => Action::Bet { to: *min_to }, LegalAction::Raise { min_to, .. } => Action::Raise { to: *min_to }, LegalAction::AllIn { to } => Action::AllIn { to: *to } };
        ActionAdvice { action: a, frequency: None, ev_bb: None, unavailable: u.clone(), headline: false } }).collect()
}

pub fn final_from_solution(ctx: &AssemblyCtx, node: &NodeStrategy, reach: &[f32], coverage: Coverage, mut assumptions: Assumptions) -> Recommendation {
    let mix = Some(range_mix(node, reach));
    let hero_ok = ctx.hero_combo.map(|c| node.available[c as usize] && reach[c as usize] > 0.0).unwrap_or(false);
    if !hero_ok {
        let partial = match coverage { Coverage::Approximate { reasons } => reasons, Coverage::Unsupported { partial, .. } => partial, Coverage::Exact => vec![] };
        let actions = node.actions.iter().map(|a| ActionAdvice { action: map_to_legal(a, &ctx.legal).unwrap_or(a.clone()), frequency: None, ev_bb: None, unavailable: Some(Unavailable::HeroOutOfSupport), headline: false }).collect();
        return base(ctx, Phase::Final, Coverage::Unsupported { reason: UnsupportedReason::HeroComboOutOfSupport, partial }, actions, 0.0, mix, assumptions);
    }
    let c = ctx.hero_combo.unwrap() as usize;
    let mut actions: Vec<ActionAdvice> = node.actions.iter().enumerate().map(|(i, a)| match map_to_legal(a, &ctx.legal) {
        Some(mapped) => ActionAdvice { action: mapped, frequency: Some(node.probs[c][i]), ev_bb: Some(node.ev_chips[c][i] / ctx.bb_chips as f32), unavailable: None, headline: false },
        None => ActionAdvice { action: a.clone(), frequency: Some(node.probs[c][i]), ev_bb: None, unavailable: Some(Unavailable::NotInMenu), headline: false },
    }).collect();
    if let Some(label) = headline(&mut actions, 0.0, HeadlineSource::Solved) { assumptions.notes.push(format!("headline: {label}")); }
    base(ctx, Phase::Final, coverage, actions, 0.0, mix, assumptions)
}
pub fn unsupported(ctx: &AssemblyCtx, reason: UnsupportedReason, partial: Vec<ApproxReason>, assumptions: Assumptions) -> Recommendation {
    base(ctx, Phase::Final, Coverage::Unsupported { reason, partial }, legal_menu(ctx, Some(Unavailable::NotEvaluated)), 0.0, None, assumptions)
}
/// §5 step 5: legal intervals, coverage so far, no frequency or EV, equity pending.
pub fn fast(ctx: &AssemblyCtx, coverage_so_far: Coverage, assumptions: Assumptions) -> Recommendation {
    base(ctx, Phase::Fast, coverage_so_far, legal_menu(ctx, Some(Unavailable::Pending)), 0.0, None, assumptions)
}
/// §4.4 event merging: a Ready estimate is never replaced by a Pending one.
pub fn merge_equity(rec: &mut Recommendation, eq: &EquitySummary) {
    fn merge(dst: &mut Vec<(Seat, EquityEstimate)>, src: &[(Seat, EquityEstimate)]) {
        for (seat, est) in src {
            match dst.iter_mut().find(|(s, _)| s == seat) {
                Some((_, d)) => { if est.availability == Availability::Ready || d.availability != Availability::Ready { *d = est.clone(); } }
                None => dst.push((*seat, est.clone())),
            }
        }
    }
    merge(&mut rec.equity.hero_combo_vs_each, &eq.hero_combo_vs_each);
    merge(&mut rec.equity.hero_range_vs_each, &eq.hero_range_vs_each);
    if !eq.per_pot_shares.is_empty() { rec.equity.per_pot_shares = eq.per_pot_shares.clone(); }
}
```

- [ ] **Step 3: Run, hand-check the recorded golden, commit**

Run: `cargo test -p engine --features testing --test assembly`
Expected: PASS; the recorded `recommendation_assembly_golden.json` must show `complete_ev` with `Raise{150}` as the only `headline: true`, `ev_bb` 0.0 / 1.25 / 3.0, and `out_of_support` with `coverage.reason == HeroComboOutOfSupport`, `range_mix[1] == [Call, 1.0]`.

```bash
git add crates/engine
git commit -m "feat(engine): recommendation assembly, headline rules, reason accumulation and equity merge

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 21: `Engine` API, request slot of depth 1, river/turn decision path, `identity_race_golden`, `final_delivery_independent_of_worker`

**Files:**
- Create: `crates/engine/src/snapshots.rs`, `crates/engine/src/ranges.rs`, `crates/engine/src/serve.rs`, `crates/engine/src/engine.rs`, `crates/engine/tests/identity_race.rs`, `crates/engine/tests/final_delivery.rs`
- Modify: `crates/engine/src/lib.rs` (modules and `pub use engine::{Engine, Paths}`), `crates/engine/src/core.rs` (add `snapshots`, `config`, `range_source`)

**Interfaces:**
- Consumes: every earlier engine task; `core_model::{begin_hand, apply_action, set_board, derive}`; `core_ranges::{block_public, hash_scaled, range_to_string, mass}`.
- Produces: `snapshots::{SolvedStreet { identity_at_solve: DecisionIdentity, street: Street, board: Vec<Card>, tree: EffectiveTree, nodes: Vec<NodeStrategy>, ordinal_paths: Vec<OrdinalPath>, exploitability_chips: f32, reasons: Vec<ApproxReason>, solved_prefix: Vec<(Seat, Action)> }, SnapshotStore::{new(), register(&mut self, active: &DecisionIdentity, s: SolvedStreet) -> bool, invalidate_hand(&mut self, hand_id: u64), for_hand(&self, hand_id) -> Vec<&SolvedStreet>}}` (plan 3 wraps `SolvedStreet` into `StreetSnapshot` and adds `Engine::register_snapshot`); `ranges::{RootRanges { oop: Range1326, ip: Range1326, reasons: Vec<ApproxReason>, ranges_used: Vec<(Seat, String, f32)> }, RangeSource: Send { fn ranges_at_root(&self, state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>; }, ExplicitRanges { oop: Option<Range1326>, ip: Option<Range1326> }}`; `serve::{LiveRequest { identity, state: HandState, t0_ms: u64, sink: SharedSink }, serve_request(&mut EngineCore, LiveRequest)}`; `engine::{Paths { log_dir: PathBuf, worker_exe: PathBuf }, Engine::new(GameConfig, Paths) -> Result<Engine, EngineError>, Engine::with_core(EngineCore) -> Engine, set_config(&mut self, GameConfig) -> u32, begin_hand(&mut self, BeginHand) -> Result<HandState, EngineError>, apply_action(&mut self, Action) -> Result<HandState, EngineError>, set_board(&mut self, &[Card]) -> Result<HandState, EngineError>, undo(&mut self) -> Result<HandState, EngineError>, set_explicit_ranges(&mut self, oop: Range1326, ip: Range1326), recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError>, cancel(&mut self, decision_id: u64), finish_hand(&mut self), abandon_hand(&mut self), shutdown(self), state(&self) -> Option<HandState>}`.

- [ ] **Step 1: Failing tests**

`crates/engine/tests/identity_race.rs`:

```rust
use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::ranges::ExplicitRanges;
use engine::serve::{serve_request, LiveRequest};
use engine::testing::{board, hand, play, uniform_solution, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use engine::tree::{build_tree_full, TemplateSelection};
use proto::worker::{AckStatus, ResultStatus, Stage};
use proto::{Action, Card, DecisionIdentity, Range1326, RecommendationEvent, Seat, Street};
use std::sync::{Arc, Mutex};

fn river_state() -> proto::HandState {
    let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    let s = hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), aa);
    let s = play(&s, &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
    let s = board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s");
    s   // river, BB (hero) to act, pot 65, stacks 970
}
fn full(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
fn ident(events: &[engine::testing::Recorded]) -> Vec<DecisionIdentity> {
    events.iter().map(|r| match &r.event { RecommendationEvent::Fast(x) | RecommendationEvent::Provisional(x) | RecommendationEvent::Final(x) => x.identity.clone(), RecommendationEvent::Equity { identity, .. } | RecommendationEvent::Progress { identity, .. } | RecommendationEvent::NoDecision { identity, .. } => identity.clone() }).collect()
}
fn solution_for(state: &proto::HandState) -> proto::worker::StreetSolution {
    let root = core_model::street_root(state).unwrap();
    let b = build_tree_full(&root, &TemplateSelection::from_history("river_std_v1", &root.history)).unwrap();
    uniform_solution(&b.tree, &b.history, 0.2)
}

#[test]
fn identity_race_golden() {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let state_a = river_state();
    let sol = solution_for(&state_a);
    // hand A: the solve is live when the undo arrives (InvalidateIdentity); A's late ok result carries A's request id and is discarded
    let script = vec![FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None }, FakeReply::Progress { id: IdRef::Last, stage: Stage::Solving, iterations: 10, exploitability_chips: Some(0.5), elapsed_ms: 2 },
        FakeReply::InvalidateIdentity, FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 3 }, FakeReply::Delay { ms: 1500 },   // no result{cancelled} within 1.5 s: kill
        // hand B, request 2 (the re-request): request 1's stale result arrives first, then request 2's
        FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None }, FakeReply::Result { id: IdRef::Fixed("B1".into()), status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 3 },
        FakeReply::Result { id: IdRef::Last, status: ResultStatus::Ok, solution: Some(sol.clone()), error: None, elapsed_ms: 4 }];
    let (worker, state) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_race_log")));
    core.range_source = Box::new(ExplicitRanges { oop: Some(full(&state_a.board)), ip: Some(full(&state_a.board)) });
    let (sink, events) = RecordingSink::new(clock.clone(), Some(state.clone()));
    let sink: engine::watchdog::SharedSink = Arc::new(Mutex::new(Box::new(sink)));
    let id_a = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); for _ in 0..6 { s.mutate(); } s.next_decision().unwrap() };
    assert_eq!(id_a.hand_revision, 7);
    serve_request(&mut core, LiveRequest { identity: id_a.clone(), state: state_a.clone(), t0_ms: clock.now_ms(), sink: sink.clone() });
    // undo to a non-decision, then hand B with the same displayed state; request B once, then re-request (new decision_id)
    let (id_b1, id_b2) = { let mut s = identity.lock().unwrap(); s.mutate(); s.begin_hand(); let b1 = s.next_decision().unwrap(); let b2 = s.next_decision().unwrap(); (b1, b2) };
    assert!(id_b1.hand_id != id_a.hand_id && id_b2.decision_id > id_b1.decision_id);
    serve_request(&mut core, LiveRequest { identity: id_b2.clone(), state: state_a.clone(), t0_ms: clock.now_ms(), sink: sink.clone() });
    let ev = events.lock().unwrap();
    let ids = ident(&ev);
    // A produced Fast and Progress only (its Final was never emitted); nothing carries B1; exactly one Final and it is B2's
    assert!(ids.iter().all(|i| *i == id_a || *i == id_b2), "{ids:?}");
    assert!(!ev.iter().any(|r| matches!(&r.event, RecommendationEvent::Final(x) if x.identity == id_a)));
    assert_eq!(ev.iter().filter(|r| matches!(r.event, RecommendationEvent::Final(_))).count(), 1);
    assert!(ev.iter().any(|r| matches!(&r.event, RecommendationEvent::Final(x) if x.identity == id_b2 && matches!(x.coverage, proto::Coverage::Exact))));
    // no stale snapshot: the store holds B2's solution only; A's request cancelled/killed the worker (ack never freed admission)
    assert_eq!(core.snapshots.lock().unwrap().for_hand(id_b2.hand_id).len(), 1);
    assert_eq!(core.snapshots.lock().unwrap().for_hand(id_a.hand_id).len(), 0);
    let st = state.lock().unwrap();
    assert!(st.cancels.len() == 1 && st.kills == 1);
    let _ = Street::River;
}
```

`crates/engine/tests/final_delivery.rs`:

```rust
use engine::core::EngineCore;
use engine::identity::IdentityState;
use engine::log::DecisionLog;
use engine::ranges::ExplicitRanges;
use engine::serve::{serve_request, LiveRequest};
use engine::testing::{board, hand, play, FakeClock, FakeReply, FakeWorker, IdRef, RecordingSink};
use proto::worker::{AckStatus, ResultStatus, Stage, WorkerError};
use proto::{Action, Card, Coverage, Range1326, RecommendationEvent, Seat, UnsupportedReason};
use std::sync::{Arc, Mutex};

fn river_state() -> proto::HandState {
    let aa = Some([Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap()]);
    let s = play(&hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), aa), &[Action::Fold, Action::Fold, Action::Fold, Action::Raise { to: 30 }, Action::Fold, Action::Call]);
    board(&play(&board(&play(&board(&s, "Kh 7d 2c"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d"), &[Action::Check, Action::Check]), "Kh 7d 2c 4d 9s")
}
fn full(board: &[Card]) -> Range1326 { let mut r = Range1326([1.0; 1326]); for i in 0..1326 { let [a, b] = proto::combo_cards(i as u16); if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; } } r }
fn ack() -> FakeReply { FakeReply::Ack { id: IdRef::Last, status: AckStatus::Accepted, reason: None } }

fn run(script: Vec<FakeReply>, expected_stage: &str) {
    let clock = FakeClock::new();
    let identity = Arc::new(Mutex::new(IdentityState::new()));
    let state = river_state();
    let (worker, fake) = FakeWorker::scripted(clock.clone(), identity.clone(), script);
    let mut core = EngineCore::new(worker, clock.clone(), identity.clone(), DecisionLog::open(&std::env::temp_dir().join("pokerai_delivery_log")));
    core.range_source = Box::new(ExplicitRanges { oop: Some(full(&state.board)), ip: Some(full(&state.board)) });
    let (sink, events) = RecordingSink::new(clock.clone(), Some(fake.clone()));
    let id = { let mut s = identity.lock().unwrap(); s.set_config(); s.begin_hand(); s.next_decision().unwrap() };
    serve_request(&mut core, LiveRequest { identity: id.clone(), state, t0_ms: 0, sink: Arc::new(Mutex::new(Box::new(sink))) });
    let ev = events.lock().unwrap();
    let finals: Vec<_> = ev.iter().filter(|r| matches!(r.event, RecommendationEvent::Final(_))).collect();
    assert_eq!(finals.len(), 1, "exactly one Final");
    let f = finals[0];
    assert_eq!((f.at_ms, f.kills), (14_900, 0), "Final at t0 + 14.9 s before any kill or restart");
    match &f.event { RecommendationEvent::Final(r) => match &r.coverage { Coverage::Unsupported { reason: UnsupportedReason::DeadlineExceeded { stage }, .. } => assert_eq!(stage, expected_stage), c => panic!("{c:?}") }, _ => unreachable!() }
    assert!(fake.lock().unwrap().kills >= 1, "the kill/restart proceeds after delivery");
}

#[test]
fn final_delivery_independent_of_worker() {
    // (a) a worker that never replies
    run(vec![FakeReply::Hang], "building");
    // (b) no_iteration on the first attempt, then the retry hangs
    run(vec![ack(), FakeReply::Result { id: IdRef::Last, status: ResultStatus::Error, solution: None, error: Some(WorkerError { code: "no_iteration".into(), message: "".into(), retryable: false, estimate_bytes: None }), elapsed_ms: 1 }, ack(), FakeReply::Hang], "building");
    // (c) hangs in extraction
    run(vec![ack(), FakeReply::Progress { id: IdRef::Last, stage: Stage::Extracting, iterations: 40, exploitability_chips: Some(0.3), elapsed_ms: 9 }, FakeReply::Hang], "extracting");
}
```

- [ ] **Step 2: Implement `snapshots.rs`, `ranges.rs`, `serve.rs`, `engine.rs`**

`crates/engine/src/snapshots.rs`:

```rust
use proto::worker::NodeStrategy;
use proto::{Action, ApproxReason, Card, DecisionIdentity, EffectiveTree, OrdinalPath, Seat, Street};

#[derive(Debug, Clone)]
pub struct SolvedStreet { pub identity_at_solve: DecisionIdentity, pub street: Street, pub board: Vec<Card>, pub tree: EffectiveTree, pub nodes: Vec<NodeStrategy>, pub ordinal_paths: Vec<OrdinalPath>, pub exploitability_chips: f32, pub reasons: Vec<ApproxReason>, pub solved_prefix: Vec<(Seat, Action)> }

/// Validated solutions keyed by identity (§9.2 single registration path; plan 3 wraps them into `StreetSnapshot`).
#[derive(Default)]
pub struct SnapshotStore { items: Vec<SolvedStreet> }
impl SnapshotStore {
    pub fn new() -> Self { Self::default() }
    /// Rejects anything whose identity is not the active one (§4.4: stale results are never written).
    pub fn register(&mut self, active: &DecisionIdentity, s: SolvedStreet) -> bool {
        if s.identity_at_solve != *active { return false; }
        self.items.retain(|x| !(x.identity_at_solve.hand_id == s.identity_at_solve.hand_id && x.street == s.street && x.identity_at_solve.decision_id == s.identity_at_solve.decision_id));
        self.items.push(s);
        true
    }
    pub fn invalidate_hand(&mut self, hand_id: u64) { self.items.retain(|x| x.identity_at_solve.hand_id != hand_id); }
    pub fn for_hand(&self, hand_id: u64) -> Vec<&SolvedStreet> { self.items.iter().filter(|x| x.identity_at_solve.hand_id == hand_id).collect() }
}
```

`crates/engine/src/ranges.rs`:

```rust
use core_ranges::{block_public, mass, range_to_string};
use proto::{ApproxReason, HandState, Range1326, Seat, StreetRootSnapshot, UnsupportedReason};

pub struct RootRanges { pub oop: Range1326, pub ip: Range1326, pub reasons: Vec<ApproxReason>, pub ranges_used: Vec<(Seat, String, f32)> }
/// Public ranges at the street root. Plan 3 supplies the replay implementation; this plan's engine takes explicit ranges.
pub trait RangeSource: Send {
    fn ranges_at_root(&self, state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>;
}
pub struct ExplicitRanges { pub oop: Option<Range1326>, pub ip: Option<Range1326> }
impl RangeSource for ExplicitRanges {
    fn ranges_at_root(&self, _state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason> {
        let (Some(mut oop), Some(mut ip)) = (self.oop.clone(), self.ip.clone()) else { return Err(UnsupportedReason::InvalidRanges) };
        block_public(&mut oop, &root.board);
        block_public(&mut ip, &root.board);
        if mass(&oop) <= 0.0 || mass(&ip) <= 0.0 { return Err(UnsupportedReason::InvalidRanges); }
        let used = vec![(root.oop, range_to_string(&oop), mass(&oop)), (root.ip, range_to_string(&ip), mass(&ip))];
        Ok(RootRanges { oop, ip, reasons: vec![], ranges_used: used })
    }
}
```

`crates/engine/src/core.rs` additions: fields `pub snapshots: Arc<Mutex<SnapshotStore>>` (shared with `Engine`, so a mutation never waits for a running request), `pub config: GameConfig`, `pub range_source: Box<dyn RangeSource>`; `new` initializes `snapshots: Arc::new(Mutex::new(SnapshotStore::new()))`, `config: GameConfig { config_revision: 0, chip_label: "$1".into(), sb_chips: 5, bb_chips: 10, straddle: None, rake: Rake::TimeCharge, seats: vec![], solver: SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 } }` and `range_source: Box::new(ExplicitRanges { oop: None, ip: None })`; add `pub fn set_config(&mut self, cfg: GameConfig) { self.config = cfg; }`.

`crates/engine/src/serve.rs`:

```rust
//! §5 steps 4-10 for one request on `engine-main` (river and turn; preflop and flop paths arrive in plans 3 and 4).
use crate::allin::{facing_allin, AllInInput};
use crate::assemble::{self, AssemblyCtx};
use crate::coverage::{classify, Classification};
use crate::core::EngineCore;
use crate::deadline::Deadlines;
use crate::equity::{equity_summary, pending_summary, EQUITY_BUDGET_MS};
use crate::log::{DecisionRecord, InputRecord};
use crate::snapshots::SolvedStreet;
use crate::solve::{run_solve, SolvePlan, Terminal};
use crate::tree::{build_tree_full, tree_signature, TemplateSelection, Templates};
use crate::watchdog::{Armed, SharedSink};
use core_model::derive;
use core_ranges::hash_scaled;
use proto::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct LiveRequest { pub identity: DecisionIdentity, pub state: HandState, pub t0_ms: u64, pub sink: SharedSink }

fn emit(core: &EngineCore, req: &LiveRequest, delivered: Option<&AtomicBool>, ev: RecommendationEvent) {
    if !core.identity_active(&req.identity) { return; }
    if let Some(d) = delivered { if matches!(ev, RecommendationEvent::Final(_)) && d.swap(true, Ordering::SeqCst) { return; } }
    req.sink.lock().unwrap().emit(ev);
}

pub fn serve_request(core: &mut EngineCore, req: LiveRequest) {
    let d = derive(&req.state);
    let class = classify(&req.state);
    let hero_combo = req.state.hero_cards.map(|h| combo_index(h[0], h[1]));
    let bb = req.state.config.bb_chips;
    let mut ctx = AssemblyCtx { identity: req.identity.clone(), legal: d.legal.clone(), hero_combo, bb_chips: bb, equity: pending_summary(&[]) };
    let mut assumptions = assemble::empty_assumptions("");
    assumptions.target_bp = core.config.solver.target_bp;
    let (root, inherited, facing_allin, opponent) = match class {
        Classification::NoDecision { reason } => { emit(core, &req, None, RecommendationEvent::NoDecision { identity: req.identity.clone(), reason }); return; }
        Classification::Preflop => { emit(core, &req, None, RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::EngineError { message: "no preflop path in this build (plan 3)".into(), retryable: false }, vec![], assumptions))); return; }
        Classification::Multiway { pot_eligible } => { emit(core, &req, None, RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::MultiwayEv { pot_eligible }, vec![], assumptions))); return; }
        Classification::Unsupported(reason) => { emit(core, &req, None, RecommendationEvent::Final(assemble::unsupported(&ctx, reason, vec![], assumptions))); return; }
        Classification::HuStreet { root, reasons, facing_allin, opponent } => (root, reasons, facing_allin, opponent),
    };
    if root.street == Street::Flop { emit(core, &req, None, RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::EngineError { message: "no flop path in this build (plan 4)".into(), retryable: false }, inherited, assumptions))); return; }
    ctx.equity = pending_summary(&[opponent]);
    // deadlines and watchdog (§7)
    let deadlines = Deadlines::for_request(req.t0_ms, root.street, core.config.solver.flop_budget_s);
    let delivered = Arc::new(AtomicBool::new(false));
    let terminal_seen = Arc::new(AtomicBool::new(false));
    let street_violation = Arc::new(AtomicBool::new(false));
    let retained = Arc::new(Mutex::new(None));
    core.set_stage("fast");
    let fallback = assemble::unsupported(&ctx, UnsupportedReason::DeadlineExceeded { stage: String::new() }, inherited.clone(), assumptions.clone());
    core.watchdog.arm(Armed { identity: req.identity.clone(), street_deadline_ms: deadlines.street_deadline_ms, fire_ms: deadlines.watchdog_fire_ms(), retained: retained.clone(), fallback, stage: core.stage.clone(), sink: req.sink.clone(), delivered: delivered.clone(), terminal_seen: terminal_seen.clone(), street_violation: street_violation.clone() });
    // fast phase (§5 step 5): ranges, Fast event, equity
    let ranges = match core.range_source.ranges_at_root(&req.state, &root) { Ok(r) => r, Err(reason) => { finish(core, &req, &deadlines, &delivered, RecommendationEvent::Final(assemble::unsupported(&ctx, reason, inherited, assumptions)), None, &root, false, false); return; } };
    let inherited: Vec<ApproxReason> = inherited.into_iter().chain(ranges.reasons.clone()).collect();
    let coverage_so_far = assemble::accumulate(Coverage::Exact, inherited.clone());
    assumptions.ranges_used = ranges.ranges_used.clone();
    emit(core, &req, None, RecommendationEvent::Fast(assemble::fast(&ctx, coverage_so_far.clone(), assumptions.clone())));
    let (hero_public, opp_public) = if root.oop == req.state.hero { (&ranges.oop, &ranges.ip) } else { (&ranges.ip, &ranges.oop) };
    {   // `fast-path` thread (§3.4): equity with its own 0.5 s budget, delivered as `Equity` only while the identity is active; the UI merges it into the Final (§4.4)
        let (identity, sink, hero, hp, op, board_cards, id_state) = (req.identity.clone(), req.sink.clone(), req.state.hero_cards, hero_public.clone(), opp_public.clone(), root.board.clone(), core.identity.clone());
        std::thread::Builder::new().name("fast-path".into()).spawn(move || {
            let cancel = AtomicBool::new(false);
            let eq = equity_summary(hero, &hp, &[(opponent, op)], &board_cards, Duration::from_millis(EQUITY_BUDGET_MS), &cancel);
            if id_state.lock().unwrap().is_active(&identity) { sink.lock().unwrap().emit(RecommendationEvent::Equity { identity, equity: eq }); }
        }).expect("fast-path thread");
    }
    let cancel = AtomicBool::new(false);
    // tree and solve (§5 step 7): river and turn are rooted at the street root, never cached (river) / cache arrives in plan 4 (turn)
    let template = if root.street == Street::River { "river_std_v1" } else { "turn_std_v1" };
    let build = match build_tree_full(&root, &TemplateSelection::from_history(template, &root.history)) { Ok(b) => b, Err(reason) => { finish(core, &req, &deadlines, &delivered, RecommendationEvent::Final(assemble::unsupported(&ctx, reason, inherited, assumptions)), None, &root, false, false); return; } };
    assumptions.template_id = template.into();
    assumptions.tree_signature = tree_signature(&build.tree, build.pot);
    let hero_actor = if root.oop == req.state.hero { "oop" } else { "ip" };
    let input = SolveInput { root: root.clone(), ranges: [ranges.oop.clone(), ranges.ip.clone()], tree: build.tree.clone(), target_bp: core.config.solver.target_bp };
    let plan = SolvePlan { identity: req.identity.clone(), deadlines, template_id: template.into(), retry_template_id: Templates::min_variant(template).map(String::from), rake: req.state.config.rake.clone(), hero_actor: hero_actor.into(), background: false };
    let out = run_solve(core, &input, &plan, &req.sink);
    terminal_seen.store(true, Ordering::SeqCst);
    let elapsed = (core.clock.now_ms() - req.t0_ms) as u32;
    assumptions.elapsed_ms = elapsed;
    assumptions.reached_bp = out.reached_bp;
    assumptions.source = format!("solver-worker@{}", proto::worker::SOLVER_COMMIT);
    assumptions.source_accuracy = out.solution.as_ref().map(|s| format!("exploitability <= {:.3} chips", s.exploitability_chips)).unwrap_or_else(|| "n/a".into());
    let event = match (&out.terminal, &out.solution) {
        (Terminal::Ok | Terminal::BestSoFar, Some(sol)) => {
            let coverage = assemble::coverage_for_solve(sol.exploitability_chips, build.pot, core.config.solver.target_bp, out.terminal == Terminal::BestSoFar, inherited.clone());
            let requested = sol.requested as usize;
            let reach = assemble::hero_reach(&sol.nodes, &out.ordinal_paths, requested, hero_public, hero_actor);
            let rec = assemble::final_from_solution(&ctx, &sol.nodes[requested], &reach, coverage, assumptions.clone());
            let snap = SolvedStreet { identity_at_solve: req.identity.clone(), street: root.street, board: root.board.clone(), tree: out.tree.clone(), nodes: sol.nodes.clone(), ordinal_paths: out.ordinal_paths.clone(), exploitability_chips: sol.exploitability_chips, reasons: inherited.clone(), solved_prefix: root.history.clone() };
            let active = core.identity.lock().unwrap().active().cloned();
            if let Some(a) = active { core.snapshots.lock().unwrap().register(&a, snap); }
            RecommendationEvent::Final(rec)
        }
        (Terminal::Failed(reason), _) => {
            let fallback_ok = facing_allin && matches!(reason, UnsupportedReason::EngineError { .. } | UnsupportedReason::DeadlineExceeded { .. });
            let analytic = if fallback_ok && req.state.hero_cards.is_some() {
                let call_cost = d.legal.iter().find_map(|l| if let LegalAction::Call { cost } = l { Some(*cost) } else { None }).unwrap_or(0);
                facing_allin(&AllInInput { hero: req.state.hero_cards.unwrap(), board: req.state.board.clone(), opp_public: opp_public.clone(), call_cost, pot: d.pot, facing: d.facing, rake: req.state.config.rake.clone(), bb_chips: bb }, Duration::from_millis(EQUITY_BUDGET_MS), &cancel).ok()
            } else { None };
            match analytic {
                Some(a) => {
                    let mut rec = assemble::unsupported(&ctx, UnsupportedReason::InvalidRanges, vec![], assumptions.clone());
                    rec.coverage = assemble::accumulate(Coverage::Approximate { reasons: vec![ApproxReason::UnconditionedCurrentStreet] }, inherited.clone());
                    rec.actions = a.actions.clone();
                    rec.assumptions.notes.push(format!("analytic all-in fallback: equity {:.4}, W {}, R {:.2}, EV(call) {:.2} chips; headline: highest EV", a.equity, a.w, a.r, a.ev_call_chips));
                    RecommendationEvent::Final(rec)
                }
                None => RecommendationEvent::Final(assemble::unsupported(&ctx, reason.clone(), inherited.clone(), assumptions.clone())),
            }
        }
        _ => RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::EngineError { message: "terminal without solution".into(), retryable: false }, inherited.clone(), assumptions.clone())),
    };
    let superseded = matches!(&out.terminal, Terminal::Failed(UnsupportedReason::EngineError { message, .. }) if message.contains("superseded"));
    if !superseded { finish(core, &req, &deadlines, &delivered, event, out.reached_bp, &root, out.street_violation || street_violation.load(Ordering::SeqCst), out.terminal == Terminal::BestSoFar); }
    else { core.watchdog.disarm(); }
    // §7: after a final-delivery emission the kill/restart of a still-busy worker proceeds independently
    if matches!(&out.terminal, Terminal::Failed(UnsupportedReason::DeadlineExceeded { .. })) { core.worker.kill(); let _ = core.worker.restart(); }
}

#[allow(clippy::too_many_arguments)]
fn finish(core: &mut EngineCore, req: &LiveRequest, deadlines: &Deadlines, delivered: &AtomicBool, event: RecommendationEvent, reached_bp: Option<u16>, root: &StreetRootSnapshot, street_violation: bool, best_so_far: bool) {
    let now = core.clock.now_ms();
    let final_violation = now >= deadlines.watchdog_fire_ms();
    emit(core, req, Some(delivered), event.clone());
    core.watchdog.disarm();
    if let RecommendationEvent::Final(rec) = &event {
        let reasons = match &rec.coverage { Coverage::Approximate { reasons } => reasons.clone(), Coverage::Unsupported { partial, .. } => partial.clone(), Coverage::Exact => vec![] };
        let hashes = rec.assumptions.ranges_used.iter().map(|(_, s, _)| { let r = core_ranges::parse_range(s).unwrap_or(Range1326([0.0; 1326])); hex::encode(hash_scaled(&r)) }).collect();
        core.log.append(&DecisionRecord { identity: req.identity.clone(), street: root.street, coverage: rec.coverage.clone(), reasons, elapsed_ms: (now - req.t0_ms) as u32, cache: "miss".into(), presolver_scenario: None, tier: None, reached_bp,
            street_violation: street_violation && !(best_so_far && root.street == Street::Flop), final_violation, template_id: rec.assumptions.template_id.clone(), input: InputRecord::from_state(&req.state, hashes) });
    }
}
```

`crates/engine/src/engine.rs`:

```rust
//! §3.5 public surface: commands never block; `engine-main` serves the request slot of depth 1 (newest wins).
use crate::clock::SystemClock;
use crate::core::EngineCore;
use crate::identity::IdentityState;
use crate::log::DecisionLog;
use crate::ranges::ExplicitRanges;
use crate::serve::{serve_request, LiveRequest};
use crate::worker::process::ProcessWorker;
use crate::{EngineError, EventSink};
use core_model::{apply_action, begin_hand, set_board, BeginHand};
use proto::*;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

pub struct Paths { pub log_dir: PathBuf, pub worker_exe: PathBuf }
struct Slot { pending: Option<LiveRequest>, stop: bool }
pub struct Engine {
    identity: Arc<Mutex<IdentityState>>, clock: Arc<dyn crate::clock::Clock>, snapshots: Arc<Mutex<crate::snapshots::SnapshotStore>>,
    core: Arc<Mutex<EngineCore>>, slot: Arc<(Mutex<Slot>, Condvar)>,
    state: Option<HandState>, undo: Vec<HandState>, config: GameConfig, ranges: Option<(Range1326, Range1326)>,
    main: Option<std::thread::JoinHandle<()>>,
}
impl Engine {
    pub fn new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError> {
        let worker = ProcessWorker::spawn(&paths.worker_exe, cfg.solver.threads).map_err(|e| EngineError::Message(e.to_string()))?;
        let identity = Arc::new(Mutex::new(IdentityState::new()));
        let clock: Arc<dyn crate::clock::Clock> = Arc::new(SystemClock::new());
        let mut core = EngineCore::new(Box::new(worker), clock.clone(), identity.clone(), DecisionLog::open(&paths.log_dir));
        core.set_config(cfg.clone());
        let mut e = Engine::with_core(core);
        e.set_config(cfg);
        Ok(e)
    }
    pub fn with_core(core: EngineCore) -> Engine {
        let identity = core.identity.clone();
        let clock = core.clock.clone();
        let snapshots = core.snapshots.clone();
        let config = core.config.clone();
        let core = Arc::new(Mutex::new(core));
        let slot = Arc::new((Mutex::new(Slot { pending: None, stop: false }), Condvar::new()));
        let (c2, s2) = (core.clone(), slot.clone());
        let main = std::thread::Builder::new().name("engine-main".into()).spawn(move || loop {
            let req = { let (m, cv) = &*s2; let mut g = m.lock().unwrap(); while g.pending.is_none() && !g.stop { g = cv.wait(g).unwrap(); } if g.stop { return; } g.pending.take().unwrap() };
            serve_request(&mut c2.lock().unwrap(), req);
        }).expect("engine-main");
        Engine { identity, clock, snapshots, core, slot, state: None, undo: vec![], config, ranges: None, main: Some(main) }
    }
    pub fn set_config(&mut self, cfg: GameConfig) -> u32 {
        let rev = self.identity.lock().unwrap().set_config();
        self.config = GameConfig { config_revision: rev, ..cfg };
        self.core.lock().unwrap().set_config(self.config.clone());
        rev
    }
    fn hand_config(&self) -> HandConfig { HandConfig { config_revision: self.config.config_revision, sb_chips: self.config.sb_chips, bb_chips: self.config.bb_chips, straddle: self.config.straddle.clone(), rake: self.config.rake.clone(), chip_label: self.config.chip_label.clone() } }
    fn stamp(&self, mut s: HandState, hand_id: u64, rev: u32) -> HandState { s.hand_id = hand_id; s.hand_revision = rev; s }
    pub fn begin_hand(&mut self, req: BeginHand) -> Result<HandState, EngineError> {
        let s = begin_hand(&self.hand_config(), req).map_err(|e| EngineError::Rules(e.to_string()))?;
        let (hand_id, rev) = self.identity.lock().unwrap().begin_hand();
        self.undo.clear();
        self.state = Some(self.stamp(s, hand_id, rev));
        Ok(self.state.clone().unwrap())
    }
    fn mutate(&mut self, next: HandState) -> HandState {
        let rev = self.identity.lock().unwrap().mutate();
        if let Some(prev) = self.state.take() { self.undo.push(prev); }
        let hand_id = self.undo.last().map(|p| p.hand_id).unwrap_or(next.hand_id);
        let s = self.stamp(next, hand_id, rev);
        self.snapshots.lock().unwrap().invalidate_hand(hand_id);
        self.state = Some(s.clone());
        s
    }
    pub fn apply_action(&mut self, a: Action) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = apply_action(cur, a).map_err(|e| EngineError::Rules(e.to_string()))?;
        Ok(self.mutate(next))
    }
    pub fn set_board(&mut self, cards: &[Card]) -> Result<HandState, EngineError> {
        let cur = self.state.as_ref().ok_or(EngineError::Message("no hand".into()))?;
        let next = set_board(cur, cards).map_err(|e| EngineError::Rules(e.to_string()))?;
        Ok(self.mutate(next))
    }
    pub fn undo(&mut self) -> Result<HandState, EngineError> {
        let prev = self.undo.pop().ok_or(EngineError::Message("nothing to undo".into()))?;
        let rev = self.identity.lock().unwrap().mutate();
        let s = self.stamp(prev, self.state.as_ref().map(|s| s.hand_id).unwrap_or(0), rev);
        self.snapshots.lock().unwrap().invalidate_hand(s.hand_id);
        self.state = Some(s.clone());
        Ok(s)
    }
    /// Plan 2 only: the public ranges at the street root (plan 3 replaces this with replay).
    pub fn set_explicit_ranges(&mut self, oop: Range1326, ip: Range1326) { self.ranges = Some((oop.clone(), ip.clone())); self.core.lock().unwrap().range_source = Box::new(ExplicitRanges { oop: Some(oop), ip: Some(ip) }); }
    pub fn recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError> {
        let state = self.state.clone().ok_or(EngineError::Message("no hand".into()))?;
        let identity = self.identity.lock().unwrap().next_decision().ok_or(EngineError::Message("no hand in progress".into()))?;
        let t0_ms = self.clock.now_ms();
        let (m, cv) = &*self.slot;
        m.lock().unwrap().pending = Some(LiveRequest { identity: identity.clone(), state, t0_ms, sink: Arc::new(Mutex::new(sink)) });   // depth 1: an older pending request is dropped
        cv.notify_one();
        Ok(identity)
    }
    pub fn cancel(&mut self, decision_id: u64) { let mut i = self.identity.lock().unwrap(); if i.active().map(|a| a.decision_id) == Some(decision_id) { i.cancel_active(); } }
    pub fn finish_hand(&mut self) { self.end_hand(); }
    pub fn abandon_hand(&mut self) { self.end_hand(); }
    fn end_hand(&mut self) { let hand_id = self.state.as_ref().map(|s| s.hand_id); self.identity.lock().unwrap().invalidate_hand(); if let Some(h) = hand_id { self.snapshots.lock().unwrap().invalidate_hand(h); } self.state = None; self.undo.clear(); }
    pub fn state(&self) -> Option<HandState> { self.state.clone() }
    pub fn shutdown(mut self) {
        { let (m, cv) = &*self.slot; m.lock().unwrap().stop = true; cv.notify_all(); }
        if let Some(h) = self.main.take() { let _ = h.join(); }
        let mut core = self.core.lock().unwrap();
        let id = core.next_id();
        let _ = core.worker.send(&proto::worker::EngineMessage::Shutdown { id });
        core.worker.kill();
    }
}
```

`crates/engine/src/lib.rs` (final module list): `clock, identity, tree, worker, deadline, watchdog, core, solve, coverage, equity, allin, assemble, log, snapshots, ranges, serve, engine` plus `#[cfg(any(test, feature = "testing"))] pub mod testing;` and `pub use engine::{Engine, Paths};`.

- [ ] **Step 3: Run and commit**

Run: `cargo test -p engine --features testing` then `cargo test --workspace`
Expected: all green, including `identity_race_golden` (A: Fast, Equity, Progress only; B2: the single Final; one cancel, one kill) and `final_delivery_independent_of_worker` (Final at 14 900 ms with `kills == 0` at emission in all three scripts).

```bash
git add crates/engine
git commit -m "feat(engine): Engine API with the depth-1 request slot, river/turn decision path, snapshot store and identity goldens

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Task 30: `bench run` for the river and turn suites with the §13.5 report

**Files:**
- Create: `crates/bench/src/runner.rs`, `crates/bench/src/report.rs`
- Modify: `crates/bench/src/main.rs` (add `run`)
- Test: unit tests in `report.rs`; a smoke run in step 4

**Interfaces:**
- Consumes: `engine::worker::process::ProcessWorker`, `engine::worker::link::WorkerLink`, `engine::tree::{materialize_at, Templates, tree_signature}`, `engine::deadline::{street_budget_ms, extraction_margin_ms}`, `core_ranges::{parse_range, block_public}`, `suite::{Spot, Suite}`.
- Produces: `runner::{SpotResult { spot: String, rep: u32, cold: bool, wall_ms: u64, ack_ms: u64, status: String, reached_bp: Option<u16>, iterations: u32, memory_bytes: u64, peak_ws_bytes: u64, mode: String, street_violation: bool, final_violation: bool }, run_spot(worker: &mut dyn WorkerLink, spot: &Spot, rep: u32, cold: bool) -> SpotResult, cancel_latency(worker: &mut dyn WorkerLink, spot: &Spot) -> (u64, u64)}`; `report::{Report::new(suite: &str, threads: u8, reps: u32), push(&mut self, SpotResult), set_cancel_latency(ack_ms, result_ms), to_markdown(&self) -> String, append_to(&self, path: &Path)}`; CLI `bench run --suite river_std|river_min|turn_std|turn_min --threads N --reps R --out docs/bench/`.

- [ ] **Step 1: Failing report test (bottom of `report.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn r(wall: u64, status: &str, bp: u16) -> crate::runner::SpotResult { crate::runner::SpotResult { spot: "s".into(), rep: 1, cold: false, wall_ms: wall, ack_ms: 1, status: status.into(), reached_bp: Some(bp), iterations: 50, memory_bytes: 1 << 20, peak_ws_bytes: 2 << 20, mode: "f32".into(), street_violation: wall > 2000, final_violation: wall > 15000 } }
    #[test]
    fn percentiles_and_sections() {
        let mut rep = Report::new("river_std", 16, 5);
        for w in [10, 20, 30, 40, 2500] { rep.push(r(w, if w == 2500 { "best_so_far" } else { "ok" }, if w == 2500 { 90 } else { 30 })); }
        rep.set_cancel_latency(4, 120);
        let md = rep.to_markdown();
        assert!(md.contains("| river_std | 16 |") && md.contains("p50 30 ms") && md.contains("p95 2500 ms") && md.contains("max 2500 ms"));
        assert!(md.contains("Exact 80.0% / Approximate 20.0% / Unsupported 0.0%") && md.contains("street violations 1") && md.contains("final violations 0"));
        assert!(md.contains("cancel ack 4 ms, result 120 ms"));
        assert_eq!(percentile(&[1, 2, 3, 4], 0.5), 3);
    }
}
```

- [ ] **Step 2: Implement `runner.rs`, `report.rs`, the `run` subcommand**

`crates/bench/src/runner.rs`:

```rust
use crate::suite::Spot;
use engine::deadline::{extraction_margin_ms, street_budget_ms};
use engine::tree::{materialize_at, Templates};
use engine::worker::link::WorkerLink;
use proto::worker::{EngineMessage, ResultStatus, SolveRequest, WorkerMessage};
use proto::{Rake, Range1326};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, serde::Serialize)]
pub struct SpotResult { pub spot: String, pub rep: u32, pub cold: bool, pub wall_ms: u64, pub ack_ms: u64, pub status: String, pub reached_bp: Option<u16>, pub iterations: u32, pub memory_bytes: u64, pub peak_ws_bytes: u64, pub mode: String, pub street_violation: bool, pub final_violation: bool }

fn range(s: &str, board: &[proto::Card]) -> Range1326 { let mut r = core_ranges::parse_range(s).expect("range"); core_ranges::block_public(&mut r, board); r }
static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn request(spot: &Spot, deadline_ms: u32) -> SolveRequest {
    let t = Templates::get(&spot.template_id).expect("template");
    let prefix: Vec<(usize, proto::Action)> = spot.history.iter().enumerate().map(|(i, a)| (i % 2, a.clone())).collect();   // oop acts first at every root of this plan's suites
    let b = materialize_at(t, spot.pot, spot.stack_oop.min(spot.stack_ip), &prefix).expect("materialize");
    let (rake_rate, rake_cap_mchips) = match spot.rake { Rake::PotRake { rate, cap_mchips, .. } => (rate, cap_mchips), Rake::TimeCharge => (0.0, 0) };
    SolveRequest { id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string(), spot: format!("{:0>64}", spot.id.len()), board: spot.board.clone(), oop_range: range(&spot.oop_range, &spot.board), ip_range: range(&spot.ip_range, &spot.board),
        pot: spot.pot, stack_oop: spot.stack_oop, stack_ip: spot.stack_ip, rake_rate, rake_cap_mchips, tree: b.tree, history: b.history, target_bp: spot.target_bp, deadline_ms, extraction_margin_ms: extraction_margin_ms(spot.root_street), memory_limit_bytes: 10 << 30, background: false }
}

pub fn run_spot(worker: &mut dyn WorkerLink, spot: &Spot, rep: u32, cold: bool) -> SpotResult {
    let budget = street_budget_ms(spot.root_street, 10);
    let req = request(spot, (budget - 150) as u32);
    let id = req.id.clone();
    let t = Instant::now();
    worker.send(&EngineMessage::Solve(req)).expect("send");
    let (mut ack_ms, mut res) = (0u64, None);
    while res.is_none() {
        match worker.recv(Duration::from_secs(60)).expect("worker alive") {
            Some(WorkerMessage::Ack { id: i, .. }) if i == id => ack_ms = t.elapsed().as_millis() as u64,
            Some(WorkerMessage::Result { id: i, status, solution, error, .. }) if i == id => res = Some((status, solution, error)),
            Some(_) => {}
            None => break,
        }
    }
    let wall_ms = t.elapsed().as_millis() as u64;
    let (status, solution, _) = res.unwrap_or((ResultStatus::Error, None, None));
    let status_s = match status { ResultStatus::Ok => "ok", ResultStatus::BestSoFar => "best_so_far", ResultStatus::Cancelled => "cancelled", ResultStatus::Error => "error" }.to_string();
    let reached_bp = solution.as_ref().map(|s| (10_000.0 * s.exploitability_chips / spot.pot as f32).round() as u16);
    SpotResult { spot: spot.id.clone(), rep, cold, wall_ms, ack_ms, status: status_s, reached_bp, iterations: solution.as_ref().map(|s| s.iterations).unwrap_or(0),
        memory_bytes: solution.as_ref().map(|s| s.memory_bytes).unwrap_or(0), peak_ws_bytes: worker.peak_working_set_bytes(), mode: solution.as_ref().map(|s| s.mode.clone()).unwrap_or_default(),
        street_violation: wall_ms > budget, final_violation: wall_ms > 15_000 }
}

/// Sends a solve, cancels after 50 ms, measures the ack and the terminal latency (§13.5 "cancel latency").
pub fn cancel_latency(worker: &mut dyn WorkerLink, spot: &Spot) -> (u64, u64) {
    let req = request(spot, 30_000);
    let id = req.id.clone();
    worker.send(&EngineMessage::Solve(req)).expect("send");
    std::thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    let cid = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst).to_string();
    worker.send(&EngineMessage::Cancel { id: cid.clone(), target: id.clone() }).expect("send");
    let (mut ack, mut result) = (0u64, 0u64);
    loop {
        match worker.recv(Duration::from_secs(10)).expect("worker alive") {
            Some(WorkerMessage::Ack { id: i, .. }) if i == cid => ack = t.elapsed().as_millis() as u64,
            Some(WorkerMessage::Result { id: i, .. }) if i == id => { result = t.elapsed().as_millis() as u64; break; }
            Some(_) => {}
            None => break,
        }
    }
    (ack, result)
}
```

`crates/bench/src/report.rs`:

```rust
use crate::runner::SpotResult;
use std::path::Path;

pub fn percentile(sorted: &[u64], p: f64) -> u64 { if sorted.is_empty() { return 0; } let idx = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1; sorted[idx] }

pub struct Report { suite: String, threads: u8, reps: u32, rows: Vec<SpotResult>, cancel: Option<(u64, u64)> }
impl Report {
    pub fn new(suite: &str, threads: u8, reps: u32) -> Self { Self { suite: suite.into(), threads, reps, rows: vec![], cancel: None } }
    pub fn push(&mut self, r: SpotResult) { self.rows.push(r); }
    pub fn set_cancel_latency(&mut self, ack_ms: u64, result_ms: u64) { self.cancel = Some((ack_ms, result_ms)); }
    /// §13.5: p50/p95/max wall to target, time-to-target, peak RSS, threads, cancel latency, coverage proportions, violation counts.
    pub fn to_markdown(&self) -> String {
        let mut walls: Vec<u64> = self.rows.iter().map(|r| r.wall_ms).collect(); walls.sort_unstable();
        let at_target: Vec<u64> = { let mut v: Vec<u64> = self.rows.iter().filter(|r| r.status == "ok").map(|r| r.wall_ms).collect(); v.sort_unstable(); v };
        let n = self.rows.len().max(1) as f64;
        let pct = |s: &str| 100.0 * self.rows.iter().filter(|r| r.status == s).count() as f64 / n;
        let unsupported = 100.0 * self.rows.iter().filter(|r| r.status == "error" || r.status == "cancelled").count() as f64 / n;
        let peak = self.rows.iter().map(|r| r.peak_ws_bytes).max().unwrap_or(0);
        let mut md = format!("## Suite {} ({} reps)\n\n| suite | threads | wall p50 / p95 / max | time-to-target p50 / p95 | peak RSS | coverage | violations | cancel |\n|---|---|---|---|---|---|---|---|\n", self.suite, self.reps);
        md += &format!("| {} | {} | p50 {} ms / p95 {} ms / max {} ms | p50 {} ms / p95 {} ms | {:.1} MB | Exact {:.1}% / Approximate {:.1}% / Unsupported {:.1}% | street violations {}, final violations {} | {} |\n",
            self.suite, self.threads, percentile(&walls, 0.5), percentile(&walls, 0.95), walls.last().copied().unwrap_or(0), percentile(&at_target, 0.5), percentile(&at_target, 0.95), peak as f64 / 1048576.0,
            pct("ok"), pct("best_so_far"), unsupported, self.rows.iter().filter(|r| r.street_violation).count(), self.rows.iter().filter(|r| r.final_violation).count(),
            self.cancel.map(|(a, r)| format!("cancel ack {a} ms, result {r} ms")).unwrap_or_else(|| "n/a".into()));
        md += "\n| spot | rep | cold | wall ms | status | reached bp | iterations | memory est | peak WS | mode |\n|---|---|---|---|---|---|---|---|---|---|\n";
        for r in &self.rows { md += &format!("| {} | {} | {} | {} | {} | {} | {} | {:.1} MB | {:.1} MB | {} |\n", r.spot, r.rep, r.cold, r.wall_ms, r.status, r.reached_bp.map(|b| b.to_string()).unwrap_or_else(|| "-".into()), r.iterations, r.memory_bytes as f64 / 1048576.0, r.peak_ws_bytes as f64 / 1048576.0, r.mode); }
        md + "\n"
    }
    pub fn append_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() { std::fs::create_dir_all(d)?; }
        let header = if path.exists() { String::new() } else { "# Bench report (i7-13700K, spec 13.5)\n\n".to_string() };
        std::fs::write(path, format!("{}{}{}", std::fs::read_to_string(path).unwrap_or_default(), header, self.to_markdown()))
    }
}
```

`crates/bench/src/main.rs` (add `mod runner; mod report;` and the `run` arm):

```rust
        Some("run") => {
            let suite_name = arg(&args, "--suite").unwrap_or_else(|| "river_std".into());
            let threads: u8 = arg(&args, "--threads").and_then(|v| v.parse().ok()).unwrap_or(16);
            let reps: u32 = arg(&args, "--reps").and_then(|v| v.parse().ok()).unwrap_or(5);
            let out = std::path::PathBuf::from(arg(&args, "--out").unwrap_or_else(|| "docs/bench".into()));
            let suite = suite::Suite::load(&std::path::PathBuf::from("bench/spots").join(format!("{suite_name}.json"))).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(2) });
            let exe = std::env::var("POKERAI_WORKER").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("target/release/solver-worker.exe"));
            let mut rep = report::Report::new(&suite_name, threads, reps);
            for spot in &suite.spots {
                let mut worker = engine::worker::process::ProcessWorker::spawn(&exe, threads).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(2) });   // cold = first run in a fresh process
                for r in 1..=reps { let res = runner::run_spot(&mut worker, spot, r, r == 1); println!("{} rep {} {} {} ms", res.spot, r, res.status, res.wall_ms); rep.push(res); }
                if spot.id == suite.spots[0].id { let (a, b) = runner::cancel_latency(&mut worker, spot); rep.set_cancel_latency(a, b); }
                engine::worker::link::WorkerLink::kill(&mut worker);
            }
            let date = std::env::var("BENCH_DATE").unwrap_or_else(|_| "2026-09-10".into());
            match rep.append_to(&out.join(format!("{date}-i7-13700K.md"))) { Ok(()) => 0, Err(e) => { eprintln!("{e}"); 2 } }
        }
```

- [ ] **Step 3: Run the unit tests**

Run: `cargo test -p bench`
Expected: 3 passed.

- [ ] **Step 4: Smoke run and commit**

Run: `cargo build --release -p solver-worker && cargo run --release -p bench -- run --suite river_std --threads 16 --reps 2 --out docs/bench/`
Expected: `docs/bench/2026-09-10-i7-13700K.md` with a `river_std` section; every river spot `ok` at target within 2 s (measured 4 ms on the R8 analogue); `cancel ack` well under 50 ms. Repeat for `river_min`, `turn_std`, `turn_min` (turn spots at 200bb with two sizes: about 1 s each).

```bash
git add crates/bench docs/bench
git commit -m "feat(bench): bench run for the river and turn suites with the section 13.5 report

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Self-review

### 1. Spec coverage

| Spec section | Requirement | Task(s) |
|---|---|---|
| §3.1, §3.2 | worker process, crate layout, dependency direction, `solver-worker` depends only on `proto` + vendored solver | 1, 2, 5, 7 |
| §3.3 | AGPL boundary: `license = "AGPL-3.0"`, `LICENSE`/`PINNED_COMMIT`/`PATCHES.md` retained | 1, 7 |
| §3.4 | worker threads `control`/`writer`/`executor`, rayon 16 by `--threads`, priority class by `background`; engine `engine-main`, `fast-path`, `watchdog`, `worker-stdout`/`worker-stderr` threads | 7, 10, 14, 16, 21 |
| §3.5 | `Engine` API (river/turn), `build_effective_tree`, `bench run` CLI, worker exit codes | 4, 5, 7, 10, 21, 22 |
| §3.7 | pinned commit, features, two patches, AVX2 build check, `ready` features, engine refusal without AVX2, library lifecycle constraints | 1, 7, 9, 14 |
| §4.4 | identity tuple, headline rules, reason accumulation, `Unsupported.partial`, event merging (`Equity` never downgrades) | 2, 19, 21 |
| §4.5 | wire schema, limits (1 MiB / 16 MiB / 100,000 nodes, truncated export), states, every ack/result rule, lock staging, progress cadence and `null` accuracy, `ready` validation, failure codes, matrix validation | 6, 7, 9, 10, 13, 14 |
| §4.6 | pinned action construction (opening/donk/facing, clamp-force-dedupe, cross-street `matched`), exact insertion, wager cap, cross-check, `tree_signature`, `rules_version` 3 | 3, 4, 8 |
| §5 steps 4-10 | admission, identity per request, fast phase, HU solve path, result validation, snapshot registration, decision log | 20, 21 |
| §6 | classifier table and rules, facing-all-in analytic fallback with the T1 numbers | 17, 18, 21 |
| §7 | budgets, `deadline_ms = remaining - 150`, extraction margins, worker stop rule, heartbeat 5 s, cancel 1.5 s kill, watchdog at final - 100 ms, retry admission | 9, 16, 21 |
| §10.1 | seven templates, `_min` retry mapping | 2 |
| §10.2 | root inputs from the snapshot, exact insertion alongside menus, projection labels | 4, 17 |
| §10.3 | adapter mapping, memory admission, headroom, locks before the first iteration, extraction, index transposition, `normalize_ev` identity, rake mapping, job object 16 GiB | 8, 9, 11, 14 |
| §10.6 | `best_so_far` labelled `DeadlineBestSoFar` with per-action EV | 16, 19, 21 |
| §12 | typed handling of `tree_mismatch`, `no_iteration`, exit/heartbeat, `tree_too_large`, cancel-not-confirmed, stale identity | 16, 21 |
| §13.0 | `fixtures/worker/*.jsonl`, `fixtures/solver/basic_0p3.json`, `bench/spots/*.json` | 5, 6, 12 |
| §13.2 | every contract test (see the deviations below for three placements) | 7, 8, 10, 11, 12, 13, 14 |
| §13.3 | `tree_builder_golden`, `coverage_classification_golden`, `facing_allin_golden`, `recommendation_assembly_golden`, `identity_race_golden`, `final_delivery_independent_of_worker` | 4, 17, 18, 19, 21 |
| §13.5 | report columns, river/turn suites, cancel latency, violation counts | 22 |

Gaps and deviations (all deliberate, each recorded in the task that carries it):

1. `ev_convention_non_root_payoffs` (Task 11): the spec's OOP hand `AA` cannot have equity 1 on `Qs Jd 7h 3c 2d` (three queens beat aces); the stated `+200` / `-50` values are reproduced with OOP `QQ,66` through card removal. The spec text should be corrected to `QQ`.
2. `river_check_only_terminal_oracle` runs in `crates/engine/tests/worker_link.rs` (Task 14) because its oracle is `core-eval`, which `solver-worker` may not depend on; it still spawns the real binary.
3. `ev_conservation` (Task 11) checks the library's root EVs in-process through the adapter's mapping, because IP's root-range EV is not an actor-owned wire export.
4. `tree_materialization_matches_library` equality over the 47 cases is checked in-process (Task 8, `every_materialization_case_matches_library`); the wire test of Task 12 covers the `tree_mismatch` cases.
5. `register_snapshot(&DecisionIdentity, StreetSnapshot)` needs `core-replay`'s `StreetSnapshot` (plan 3); this plan provides `SnapshotStore::register(&DecisionIdentity, SolvedStreet)` with the same identity rule, and plan 3 wraps it.
6. Preflop and flop decisions answer `Final` `Unsupported{EngineError{"no ... path in this build"}}` until plans 3 and 4; multiway answers `Unsupported{MultiwayEv}` without the `experimental` block. The §6 experimental surrogate (`experimental_surrogate_golden`) is in no plan of the series map; it belongs with the flop path (plan 4), reusing `run_solve` on a synthetic root.
7. The engine always sends `background: false`; the worker's priority-class switch is exercised only by plan 4's pre-solver.
8. `bench gen-spots --source r8` freezes uniform R8 ranges; the chart-replay baseline set (`--source file`) arrives with plan 3.
9. `deadline_best_so_far_labelling` and `flop_budget_setting_golden` (§13.3) need the cache's `Provisional` and the flop budget and are plan 4's, as the brief assigns.

### 2. Placeholder scan

Searched the plan for `TBD`, `TODO`, `implement later`, `fill in`, `add validation`, `handle edge cases`, `similar to Task`: none. Every code step contains the code; every test step contains the test; the two golden files recorded on the first run (`tree_builder_golden.json`, `recommendation_assembly_golden.json`) have their hand-check values stated in the task, and `coverage_classification_golden.json` is written in full before the first run.

### 3. Type consistency

- `Templates::get(&str) -> Option<&'static TemplateSpec>` (Task 2) is used by Tasks 3, 4, 5, 12, 22 with that signature; `Templates::min_variant` by Tasks 16 and 21.
- `materialize(&MaterializeInput) -> Result<Materialized, UnsupportedReason>` (Task 3) feeds `materialize_at` and `build_tree_full` (Task 4); `TreeBuild { tree, history, decision_path, pot, eff }` is consumed by `bench materialize` (Task 5), `run_solve` (Task 16) and `serve_request` (Task 21).
- `WorkerLink { send, recv, restart, kill, ready, peak_working_set_bytes }` (Task 14) is implemented by `ProcessWorker` (14) and `FakeWorker` (15) and consumed by `run_solve` (16), `serve_request` (21) and `bench` (22).
- `FakeReply::{Ack, Progress, Result, Delay, Eof, Malformed, Oversized, Hang, InvalidateIdentity}` and `IdRef::{Last, Fixed}` (Task 15) are the variants used in Tasks 16 and 21.
- `EngineCore::new(worker, clock, identity)` (Task 16) gains the `log` argument in Task 20; Task 20 states that `solve_client.rs`'s rig is updated, and Task 21's tests use the four-argument form. `EngineCore.snapshots` is `Arc<Mutex<SnapshotStore>>` in Task 21 and every use locks it.
- `SharedSink = Arc<Mutex<Box<dyn EventSink>>>` (Task 16) is the sink type of `run_solve`, `Armed`, `LiveRequest` and `Engine::recommend`.
- `SolvePlan { identity, deadlines, template_id, retry_template_id, rake, hero_actor, background }` and `SolveOutcome { terminal, solution, ordinal_paths, decision_path, tree, elapsed_ms, template_used, street_violation, restarts, reached_bp }` (Task 16) are read field by field in Task 21.
- `assemble::{final_from_solution, unsupported, fast, accumulate, coverage_for_solve, hero_reach, empty_assumptions, merge_equity}` (Task 19) are the names called in Task 21; `equity::{equity_summary, pending_summary, EQUITY_BUDGET_MS}` (Task 18) likewise.
- Worker: `job::run(&SolveRequest, Option<&[NodeLock]>, &mut JobControl) -> JobResult` (Task 9) is what `executor_loop` (Task 10) calls; `locks::validate` is used by `handle_message`; `extract::MAX_NODES` by `precheck`; `tree_build::{build, enumerate, cross_check}` and `history::history_to_lib` by Tasks 9, 11, 12 and the example binary.
- Fixture ids: `river_two_combo` (41/42/48), `flop_cancel` (43/44), `flop_best_so_far` (45), `lock_river` (47/51/52), `basic_turn_std_request` (`basic`) are the ids the tests of Tasks 10-13 look up.
