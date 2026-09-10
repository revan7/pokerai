# Plan 4 revision 1 — changelog (2026-09-10)

Target: `docs/superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md`.
Inputs: `docs/research/REVIEW-of-plan-4.md` (3 BLOCKER, 9 MAJOR, 13 MINOR) and
`docs/research/REVIEW-cross-plan.md` §§1, 2, 4, 5 (every resolution naming plan 4).
Spec: `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` revision 6.

Result: **2,180 lines / 21 tasks -> 4,062 lines / 26 tasks**, 172 checkbox steps, one commit per task,
all code fences balanced, no placeholder strings.

---

## Blockers

| ID | Disposition | What changed |
|---|---|---|
| B1 — T4 test templates never registered | **APPLIED** | New Task 8 Step 1a adds a `#[cfg(any(test, feature = "testing"))]` registration seam to `crates/engine/src/tree/templates.rs`: `TEST_TEMPLATES: Mutex<Vec<&'static TemplateSpec>>`, `register_test_template`, `test_extra`, and `install_cache_test_templates()` with the verbatim specs for `check_jam_test_v1` (`a`-only bets, no ordinary raises, add/force 0.0, cap 1), `check_only_test_v1` (empty bet/raise/donk menus, thresholds 0.0, cap 1) and `menu_round_test_v1` (single 0.33 bet, `a`-only raises, add 1.5, force 0.15, cap 1, flop root, all three streets). `Templates::ids()` stays derived from `all()`, so plan 2's `assert_eq!(Templates::ids().len(), 9)` remains valid. `crates/engine/src/tree/templates.rs` added to Task 8's Files; the rig calls `install_cache_test_templates()` behind a `Once`. |
| B2 — `SideMenu`/`MenuSize` do not exist; `a`-only bet menus unrepresentable | **APPLIED (as resolved for plan 1)** | Cross-plan M1 makes plan 1 Task 6 the authority: `MenuSize::{Pot(f32), AllIn}`, `SideMenu { bet, raise }`, `PlayerMenus { oop, ip, donk: Option<Vec<MenuSize>> }`. Plan 4 already used those spellings; a new "Resolved upstream names" table in the interface-ownership block names plan 1 as the owner and records the wire form. Root-street `donk` corrected to `None` (see m12). No fallback needed: `bet: Vec<MenuSize>` expresses the `a`-only bet menu §13.1 T4 requires. |
| B3 — nothing opens the cache or owns the pre-solver | **APPLIED** | Task 6 now produces `CACHE_QUOTA_BYTES = 10 GiB`, `default_cache_root()` (`%LOCALAPPDATA%\PokerAI\cache\v3`), and the real `Cache` struct with `open`/`disabled`/`store`/`store_tracked`/`touch`/`shutdown` plus the `cache-writer` thread body. Task 7 Step 5a adds `EngineCore { cache: cache::Cache, presolver: Option<Presolver> }` (defaulted to `Cache::disabled()` / `None`, so plan 2's test rigs are untouched) and resolves the root from `engine::Paths.cache` inside `Engine::new`. Task 16 starts the `Presolver`, wires `notify_hand` into `begin_hand`/`finish_hand`/`abandon_hand`, `notify_live_request` into `recommend`, and joins `presolver` + `cache-writer` + `cache-reader` in `Engine::shutdown(&mut self)`. `queue.json` is documented as a sibling in the same `v3` directory (Global Constraints). |

## Major

| ID | Disposition | What changed |
|---|---|---|
| M1 — `bench oracle` has no implementer | **APPLIED** | New `crates/bench/src/oracle.rs` in Task 23 with `REQUIRED_SUITES` (core-eval exhaustive, core-iso exhaustive, solver-worker analytic river, solver-worker EV convention/conservation, engine `facing_allin_golden`), `run_oracles(out)`, `OracleReport`, `report::append_oracle_block`, CLI `bench oracle [--out]`, and `crates/bench/tests/oracle.rs`. The gate reads `analytic_oracles_ok` from exactly that JSON key; a missing key is `false`. |
| M2 — `bench_support.rs` modified four times, created never | **APPLIED (variant)** | Resolved together with cross-plan M17/R5: `crates/engine/src/bench_support.rs` is **created by plan 2 Task 22** (recorded as plan-2-facing note 4 and in the resolved-names table), and plan 4 marks every use as *modify*. Plan 4 now defines the missing types as real code: `SourceLock`/`SourceBundle`/`UnavailableBundle` + `load_source_lock`/`verify_source_lock` (Task 17), `RecordedHand`/`RecordedEvent`/`load_records` (Task 19), `RunOptions`/`DecisionRun`/`run_record` signature (Task 21). |
| M3 — `flop_deadlines` duplicates plan 2's arithmetic | **APPLIED** | `flop_deadlines` deleted. Task 9 now asserts on `Deadlines::for_request(1000, Street::Flop, 10)` = 11000 / 16000 / watchdog 15900, budget 30 = 31000 / 36000 / 35900, turn at 6000 / 14900 / extraction 200, `worker_deadline_ms(250, 10_000) == Some(9_600)` and `Some(29_600)`. The only addition to `deadline.rs` is `flop_budget_valid(u8) -> bool`. Recorded as cross-plan D5 in the resolved-names table. |
| M4 — `set_config` rejection has no code and no reachable API | **APPLIED** | Task 9 adds `set_config_rejects_out_of_range_flop_budget` (0, 31, 255 and `threads == 0` all rejected; a rejected config leaves the previous revision and value) and the implementation body of `Engine::set_config(&mut self, GameConfig) -> Result<u32, EngineError>` with validation **before** `IdentityState::set_config()`, plus `Engine::config()`. Declared as plan-2-facing note 2. |
| M5 — the central routing change is prose with the wrong files | **APPLIED** | Plan 2's module inventory corrected (`tree/*`, `worker/*`, `solve.rs`, `serve.rs`, `watchdog.rs`, `assemble.rs`, `snapshots.rs`, `coverage.rs`, `ranges.rs` all listed). Task 10 Step 4 is now real `serve.rs` code (`CACHE_BUDGET_MS`, `Probe`, `probe_cache`, `cache_phase`, `cache_label_for`, the street/template dispatch that replaces plan 2's flop rejection, and the `CacheRoute::Final` branch with `recommendation_from_hit`); Step 4a is the `solve.rs`/`serve.rs` Provisional emission and `Armed.retained` handoff with the `keep_retained` rule; Step 4b is `cache_bridge::entry_from_solution` + `snapshot_from_hit` + the store enqueue and the `finish()` changes (`cache` label, `is_street_violation`). File structure lists `serve.rs` and `solve.rs` as modified. |
| M6 — durable queue API promised without implementation | **APPLIED** | Task 14 Step 4 now contains the whole `Queue`: `open` (bounded 64 MiB read, version/cursor/board/spr/identity validation, duplicate-key rejection, deterministic `rebuild`), `all_items`, `identity_of`, `save`, `next_pending`, `advance_cursor`, `record_launch`, `record_done`, `record_cancel`, `record_failure`, `reconcile`, `status_counts`, `tier_counts`, plus `QueueItem::identity_hex`. A restart/reconcile test was added to Step 1. |
| M7 — the pre-solver scheduler is prose | **APPLIED** | Task 15 Step 4 now contains `PresolverCommand`, the `Presolver` struct with its synchronous `live: AtomicBool`, and the full named-thread body: bounded `recv_timeout` command drain, `eligible` gate, `next_action` dispatch, cancel-then-drain-to-terminal (§7 "background terminal or confirmed exit before live proceeds"), `prepare`->`submit`->`poll` loop, `store_and_verify` before `record_done`, queue save on every transition, and a status publish that never holds the lock across I/O. `pause`/`resume`/`status`/`notify_hand`/`notify_live_request`/`shutdown`/`join_for_shutdown` are all written. |
| M8 — the §10.4 lookup core has no code | **APPLIED** | Task 7 Step 4 adds `map_rows`, `map_flags`, `payload_digest`, `reconstruct` (query chips at identical ordinals, `ev_over_P * P_query`, `validate_solution`, `legal_menu`, realized-menu and mode notes) and `select` (three-bucket filtering, `at_bucket(0)` equality, covered-path filter, `Comparison::rank` then raw exploitability then payload digest). New Step 4a adds `ReadCommand`, the fixed `cache-reader` thread, `Cache::start_reader` and `Cache::lookup` with the request token and the `min(budget, 500 ms)` wait. Step 5 gives `key_and_source` + `make_cache_query` as code. |
| M9 — `PresolverStatus` not serializable; engine delegation unsigned | **APPLIED** | `PresolverStatus` derives `Clone, Debug, Default, PartialEq, Serialize, Deserialize` plus `#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]`, with an optional `ts-rs` dependency and a `typescript` feature in `crates/cache/Cargo.toml`. A JSON round-trip test was added. Task 16 states the three engine signatures verbatim: `Engine::presolver_status(&self) -> cache::presolver::scheduler::PresolverStatus`, `presolver_pause(&mut self)`, `presolver_resume(&mut self)`, with bodies. |

## Minor

| ID | Disposition | What changed |
|---|---|---|
| m1 — `SOLVER_COMMIT`/`ADAPTER_VERSION` unproduced | **APPLIED** | Named in the resolved-names table as plan 1 Task 7 (`proto::worker::{PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}`, cross-plan M5/Or6) and in Task 2's Consumes line. |
| m2 — positional `zip` assumes node ordering | **APPLIED** | `cache::entry::sorted_by_path` added; `validate_entry` rejects an unsorted or empty materialized list; `compare` re-checks both lists rather than trusting the caller. A swap-two-nodes mutation case added to `tests/entry.rs`. |
| m3 — `Approximate { reasons: [] }` | **APPLIED** | `label` now returns `Coverage::Exact` whenever no reason was incurred; the above-target case is disclosed by `Phase::Provisional` and the raw reached value alone. Test `provisional_without_reasons_is_never_an_empty_approximate` added. |
| m4 — key-digest check only in `decode` | **APPLIED** | `encode` rejects a cell whose entries disagree on `key.digest()` before serializing; a matching storage test was added. |
| m5 — 22,100-flop enumeration in a plain unit test | **APPLIED** | `canonical_flops_ordered()` returns `&'static [Vec<Card>]` from a `OnceLock`; the cheap test asserts the frozen `CANONICAL_FLOP_COUNT = 1755`, and the full enumeration (plus orbit ordering and the 22,100 sum) moves behind `--features exhaustive` (added to `crates/cache/Cargo.toml`). |
| m6 — `experimental_surrogate_golden` not a named test | **APPLIED** | New Task 11 creates `crates/engine/tests/experimental_surrogate.rs` with a test literally named `experimental_surrogate_golden` and `crates/engine/tests/golden/experimental_surrogate.json`. |
| m7 — non-spec gate condition | **APPLIED** | `presolver_verified_entries` moved out of the pass/fail array into `GateResult.informational`; `evaluate` returns `passed = true` with one informational line when the queue is empty. Test `empty_presolver_queue_is_informational_only` added. |
| m8 — undeclared CLI flags | **APPLIED** | Task 20's Produces declares `bench run --suite … --threads --reps [--mode f32\|i16\|auto] [--temperature cold\|warm] [--deadline-trials] --out` and `bench gen-spots [--suite all\|<name>]`; Task 21 declares `bench e2e [--reps N] [--cache cold\|presolved]`; Task 23 declares `bench gate --report` and `bench oracle [--out]`. |
| m9 — `gen_fixtures.py e2e --check` wiring absent | **APPLIED** | Task 19 Step 3 shows the complete `argparse` subcommand table (`hands`, `worker`, `sources`, `e2e`, each with `--check`) and `check_e2e`, which regenerates in memory, compares bytes and re-verifies the manifest hashes. |
| m10 — 2,160-solve matrix sized "3 min" | **APPLIED** | Task 24 carries an "Expected wall-clock" block: ~40–110 min per `suite × threads × mode × temperature` command, 24 commands, one to two machine-days; each command appends its own rows and skips cells already present, so the loop is resumable. Task 25 does the same for the street suites (~20 min), the two e2e passes (~90 min each, plus 3–6 h to pre-solve), the fault suite (seconds) and `bench oracle` (2–4 h). |
| m11 — fault fixtures asserted before the fault module exists | **APPLIED** | Task 21 states explicitly that records 037–050 are skipped: `run_record` returns `Err` for any record with `fault.is_some()`, and its tests assert 36 runnable / 14 skipped. Task 22 supplies `fault_worker_factory` / `options_for`, removes that rejection and asserts all fourteen run. |
| m12 — `donk: Some(vec![])` on the root street | **APPLIED** | The Task 2 fixture builds `donk: if street == Street::Flop { None } else { Some(vec![]) }`, matching plan 2's `spec()` helper and §4.6. |
| m13 — `Assumptions.cache` populated nowhere | **APPLIED** | `cache_label_for(&[Probe])` produces `miss \| exact \| approximate \| provisional`; `serve_request` writes it into `assumptions.cache` before assembly, `recommendation_from_hit` carries it, and plan 2's hard-coded `cache: "miss".into()` in `finish` becomes `rec.assumptions.cache.clone()`. Test `cache_labels_are_recorded_for_every_route` added. |

## Cross-plan resolutions naming plan 4

| Item | Disposition | Where |
|---|---|---|
| M1 `SideMenu`/`MenuSize` (P: plan 1) | **APPLIED** | Resolved-names table; Tasks 2, 8. |
| M9 `StreetRootSnapshot.bb_chips` (S1) | **APPLIED** | Every literal now sets `bb_chips` (Task 12 test, Task 11 surrogate root). |
| M10 `set_config -> Result` | **APPLIED** | Task 9 + plan-2-facing note 2. |
| M11 `shutdown(&mut self)` | **APPLIED** | Task 16 shutdown ordering + plan-2-facing note 7. |
| M13 / Or5 `Paths { worker, preflop, cache, log }` | **APPLIED** | Task 7 Step 5a reads `.cache`; plan-2-facing note 3; Global Constraints. |
| M15 / D1 `SnapshotStore` | **APPLIED** | Plan 4 adds **no** second registration path: the cache route uses the same `EngineCore.snapshots` store; Task 12 only adds `snapshots::CACHE_ORIGINS` and the shared-rule test, consuming plan 3 Task 18's `Engine::register_snapshot`. |
| M17 / R5 `engine::bench_support` | **APPLIED** | Created by plan 2 Task 22, modified here; plan-2-facing note 4. |
| M18 dependency versions | **APPLIED** | `crates/cache/Cargo.toml` uses `version/edition/license.workspace = true` and `sha2/thiserror/serde/serde_json = { workspace = true }`; a note records the single `sha2 = "0.10.9"` pin. |
| M19 / R1 toolchain | **APPLIED** | Task 24 Step 2 states the MSVC pin takes precedence over earlier GNU notes and forbids `rustup default`/config mutation. |
| M20 / D10 `Templates::ids().len() == 9` | **APPLIED (orchestrator decision 2)** | Test templates go through a `cfg`-gated seam; `ids()` still returns only the 9 production/§13.2 ids, so plan 2's assertion is unchanged. |
| M21 / D2 `resolve_chip_path` | **APPLIED** | `cache::entry::resolve_path` is `pub use proto::resolve_chip_path as resolve_path;`. |
| D4 bench CLI/report/gen_spots/main ownership | **APPLIED** | File-structure rows marked *(modify)*; `crates/bench/src/lib.rs` recorded as plan-2-facing note 5. |
| D5 flop deadline arithmetic | **APPLIED** | See M3. |
| Or1 / R4 §6 experimental surrogate + golden | **APPLIED** | New Task 11 **creates** it for River, Turn and Flop (highest range-vs-range-equity opponent, total pot, min stack, empty history, street-root unconditioned ranges, hero role by postflop order, isolated from `SolveInput`/cache/snapshots) with `experimental_surrogate_golden` and an all-street isolation test. |
| Or2 `bench oracle` | **APPLIED** | See M1; Task 23 produces it, Task 25 runs it. |
| Or7 `PresolverStatus` serialization | **APPLIED** | See M9. |
| Or8 / R3 chart-replay ranges on all six suites | **APPLIED** | Task 20 regenerates `river_std`, `river_min`, `turn_std`, `turn_min`, `flop_fast`, `flop_min`; the test asserts no suite still carries plan 2's `r8_uniform` source. |
| R2 `background: true` | **APPLIED** | Resolved-names table plus Task 16 Step 4: `SolvePlan.background` is a real field plan 2 merely never sets to `true`. Recorded as plan-2-facing note 6. |
| §5 split of P4.T16 | **APPLIED** | Split into Task 17 (chart source/manifest lock), Task 18 (`e2e_hands.py` + inventory tests) and Task 19 (generate, legality-check and freeze the 50 records). |
| §5 split of P4.T21 | **APPLIED** | Split into Task 24 (deterministic regression, builds, flop matrix), Task 25 (baseline/e2e/fault/pre-solved/oracle measurement) and Task 26 (gate evaluation and reviewed report). |
| §5 note "several steps are multi-part paragraphs" | **PARTIALLY APPLIED** | Task 10's oversized Step 4 was split into 4 / 4a / 4b, Task 7's into 4 / 4a / 5 / 5a, and Task 9's into validation + policy. Task 8 Step 4 (rig + all T4 mutation cases) and Task 20 Steps 3–5 remain multi-part. *Reason:* the orchestrator designated only T16 and T21 for task-level splitting; splitting these further would renumber tasks a second time without a review finding requiring it. |

## Not applied

1. **Cross-plan §4 execution-order table** — NOT APPLIED (out of scope). The table in `REVIEW-cross-plan.md` rows 18–30 addresses `P4.T1`–`P4.T21`; plan 4 now has 26 tasks. The mapping is: old T1–T15 keep their numbers except that the new experimental-surrogate task takes slot 11 (old T11–T15 shift to 12–16); old T16 becomes 17/18/19; old T17–T20 become 20/21/22/23; old T21 becomes 24/25/26. The cross-plan re-check should renumber that table; plan 4 itself does not own it.
2. **Cross-plan §5 multi-part-step note** — PARTIALLY APPLIED, see the row above.

Everything else listed in `REVIEW-of-plan-4.md` and in the cross-plan resolutions naming plan 4 is applied.
