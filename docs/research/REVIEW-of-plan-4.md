# Review of plan 4 (flop path, cache, pre-solver) — 2026-09-10

Reviewed: `docs/superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md` (2,180 lines, 21 tasks) against spec revision 5 (§§2, 3.2–3.5, 4.2–4.6, 5, 7, 9, 10.1–10.6, 12, 13.1/13.3/13.5, 14.4), the decision outline (§0b, amended §6), `R8-solver-bench.md`, the writing-plans skill, and the series brief. Cross-checked against plans 1, 2, 3 and 5 for the interfaces this plan consumes and produces.

**Verdict: NOT READY — 3 BLOCKER, 9 MAJOR, 13 MINOR.**

The cache crate itself (Tasks 1–6) is the strongest part: keys, rationals, the SPR/menu/rake predicates, the accuracy filter and the bounded codec are real code with correct arithmetic. Everything that has to *attach* that crate to the engine (cache ownership, routing, the pre-solver thread, the T4 rig's templates) is prose or missing, and three of those gaps stop execution.

---

## BLOCKERS

**B1 — Task 8, Step 2/4: the T4 test templates are never registered anywhere.**
Quote: "Register only test templates `menu_round_test_v1` and `check_jam_test_v1` in the test harness, not the production selector."
`CacheRig::new(template, …)` builds trees "using the real materializer", i.e. plan 2's `engine::tree::build_effective_tree` → `Templates::get(id)`, whose table is a private `OnceLock<Vec<TemplateSpec>>` in `crates/engine/src/tree/templates.rs` with no registration seam. `check_jam_test_v1`, `menu_round_test_v1` and `check_only_test_v1` (used at Step 2, line 941) are not in it, so `CacheRig::new` cannot produce a tree and spec §13.1 T4 cannot run. Adding them also breaks plan 2's `assert_eq!(Templates::ids().len(), 9)`, so `cargo test --workspace` goes red.
Edit: add `crates/engine/src/tree/templates.rs` to Task 8's **Files**, give the three specs verbatim (`check_jam_test_v1`: all-in-only bets, no ordinary raises, add/force 0.0, cap 1; `check_only_test_v1`: empty bet/raise/donk menus, thresholds 0.0, cap 1; `menu_round_test_v1`: single 0.33 bet, `a`-only raises, add 1.5, force 0.15, cap 1, flop root, all three streets), and a step updating plan 2's `Templates::ids().len()` assertion to 12.

**B2 — Task 2, Step 5a (and every test template of Task 8): the menu types used do not exist, and `a`-only bet menus are unrepresentable.**
Quote: "use proto::{Action,Street,MaterializedNode,EffectiveTree,PlayerMenus,SideMenu,MenuSize}; … let side=SideMenu{bet:vec![MenuSize::AllIn],raise:vec![MenuSize::AllIn]};"
Plan 1 (the proto owner) defines `Menu { bet: Vec<f32>, raise: Vec<RaiseSize> }`, `PlayerMenus { oop: Menu, ip: Menu, donk: Option<Vec<f32>> }`, `RaiseSize::{Mult(f32), AllInOnly}` — no `SideMenu`, no `MenuSize`. Plan 2 assumes the `SideMenu`/`MenuSize` shape instead, so the two upstream plans already disagree and plan 4 inherits the break. Worse: spec §13.1 T4 requires a "test template with `a`-only bet menus" for the rake-cap pair 500/504; under plan 1's `bet: Vec<f32>` an all-in bet size cannot be expressed at all, so the spec-mandated rake-cap case cannot be built.
Edit: name one authority in the "Interface ownership" block (`MenuSize::{Pot(f32), AllIn}` + `SideMenu`, matching plans 2/4) and add a step that reconciles `crates/proto/src/tree.rs`, or state the fallback (`Menu.bet: Vec<MenuSize>`), before Task 2 Step 5a is written.

**B3 — Tasks 7/10/14/15: nothing ever opens the cache or owns the pre-solver.**
Quote (Task 6 Interfaces): "Produces … `Cache::open(PathBuf,u64)->Cache`, `Cache::store(&CacheEntry)`".
No task adds a cache handle to `EngineCore`/`Engine::new`, no task adds the `cache` field to plan 2's `Paths { log_dir, worker_exe }`, no task resolves `%LOCALAPPDATA%\PokerAI\cache\v3` (a Global Constraint, never assigned to a task), no task supplies `cache_quota_bytes = 10 GiB` (it exists in no proto type), and no task starts/stops `Presolver` or wires `notify_hand`/`notify_live_request`/`shutdown` into `begin_hand`/`finish_hand`/`abandon_hand`/`recommend`. Plan 5 already consumes the missing field: it declares `Paths { worker, preflop, cache, log }` and `cache: local.join("cache/v3")`.
Edit: in Task 7 add a step that extends `engine::Paths` with `cache: PathBuf`, resolves `%LOCALAPPDATA%\PokerAI\cache\v3` (and `queue.json` beside it), adds `pub cache: cache::Cache` plus `pub presolver: Option<Presolver>` to `EngineCore` with a `CACHE_QUOTA_BYTES: u64 = 10 * 1024 * 1024 * 1024` default, and a shutdown step joining the `cache-writer`/`presolver` threads in Task 15.

---

## MAJOR

**M1 — Task 21, Step 4: `bench oracle` is run but implemented by no task in any plan.**
Quote: "cargo run --release -p bench -- oracle".
Plan 2's bench CLI is `run | gen-spots | materialize`; plan 4 adds `gen-spots`, `e2e`, `fault`, `gate` — never `oracle`. The gate's `analytic_oracles_ok` input (Task 20) has no producer.
Edit: add an `oracle` subcommand to Task 20's Files/Interfaces (runs the §13.1 exhaustive suites and the §13.2 analytic contract tests, writes their pass/fail into the report's JSON block), or replace the Task 21 line with the `cargo test -p solver-worker --features exhaustive` commands that actually produce that evidence.

**M2 — Tasks 16–19: `crates/engine/src/bench_support.rs` is modified four times and created never.**
Quote (Task 16 Files): "modify `crates/engine/src/bench_support.rs` for record validation."
Plan 2 has no `bench_support.rs` (its engine module list ends at `engine.rs`/`log.rs`). Task 16 is the first user.
Edit: change Task 16's Files to "Create `crates/engine/src/bench_support.rs`; modify `crates/engine/src/lib.rs` (`pub mod bench_support;`)" and show the `RecordedHand`/`RunOptions`/`DecisionRun` type definitions there rather than only in Task 18's Interfaces prose.

**M3 — Task 9, Step 3: `flop_deadlines` duplicates plan 2's deadline arithmetic that the same plan forbids duplicating.**
Quote: "pub fn flop_deadlines(t0:u64,budget:u8)->Result<(u64,u64),proto::UnsupportedReason>".
Plan 2 already ships `deadline::street_budget_ms(street, flop_budget_s)`, `final_delivery_ms(street, flop_budget_s)` (`5_000 + budget*1000` for flop), `Deadlines::for_request`, `watchdog_fire_ms()` — and plan 4's own preamble says "Extend them rather than adding duplicate deadline/log/config implementations". Two sources of truth for the flop budget is exactly the drift the constraint forbids.
Edit: delete `flop_deadlines`; make `flop_budget_setting_golden` assert on `Deadlines::for_request(1000, Street::Flop, 10)` (street 11000, final 16000, `watchdog_fire_ms()` 15900; budget 30 → 31000/36000/35900) and add only the `1..=30` validation (M4).

**M4 — Task 9, Step 3: the spec-required `set_config` rejection has no code and no reachable API.**
Quote: "`set_config` rejects0/31, default remains10; it increments config revision through Plan2's path."
Spec §13.3 `flop_budget_setting_golden`: "31 is rejected by `set_config`", and plan 5 Step 1 (Task ~28) says "the engine must also reject it independently (Plan 4's `flop_budget_setting_golden`)". Plan 2's `Engine::set_config(&mut self, cfg: GameConfig) -> u32` cannot fail, and plan 4 changes neither its signature nor plan 5's expectation. The test only asserts `flop_deadlines(0,31).is_err()`.
Edit: add a step changing `Engine::set_config` to `-> Result<u32, EngineError>` (rejecting `flop_budget_s` outside `1..=30` and `threads == 0`), show the code, and assert `engine.set_config(cfg_with_31).is_err()` in the golden; note the ripple in plan 5's `set_game_config` command.

**M5 — Task 10, Step 4: the plan's central change is prose with the wrong files.**
Quote: "**Step 4 (5 min): Wire the existing HU pipeline.** For each Flop/Turn decision, derive street root, replay root public ranges, materialize query tree(s), then cache route."
The flop route must be cut into plan 2's `crates/engine/src/serve.rs` (`serve_request`) and `crates/engine/src/solve.rs` (`run_solve`, `SolvePlan`), neither of which appears in any Files list of plan 4; the preamble's inventory of plan 2 ("Its existing modules are `engine/src/deadline.rs`, `engine/src/log.rs`, `engine/src/engine.rs`, `engine/src/core.rs`, and `engine/src/testing.rs`") omits `tree/`, `worker/`, `solve.rs`, `serve.rs`, `watchdog.rs`, `assemble.rs`, `snapshots.rs`, `coverage.rs`. A 5-minute step containing one 4-line predicate cannot deliver cache→Provisional→live routing, snapshot registration, retained-payload assembly and store-on-terminal.
Edit: correct the module inventory; split Step 4 into (a) `serve.rs`: street dispatch calling `make_cache_query` + `choose_cache_route`, (b) `solve.rs`/`core.rs`: Provisional emission and retained-payload handoff to the watchdog, (c) `cache_bridge.rs`: `entry_from_solution` + store enqueue — each with its code.

**M6 — Task 13, Steps 3–4: the durable queue API is promised without an implementation.**
Quote (Interfaces): "Produces `TaskStatus::{Pending,Done,Failed{n:u8}}`, `QueueFile`, `QueueItem`, `Queue::open(PathBuf)->Result<Queue,CacheError>`, `Queue::save()->Result<(),CacheError>`, `Queue::reconcile(…)`, `Queue::record_failure(id,now_ms)`".
The code shows only `reconcile_status`, `retry_delay`, `queue_path` and `save_queue`. `Queue` itself — cursor advance, bounded read, duplicate-key rejection, corrupt-file rebuild, attempts/backoff bookkeeping — is prose.
Edit: add the `pub struct Queue { path, file: QueueFile }` definition with `open` (bounded 64 MiB read, version/cursor/board/spr/identity validation, deterministic rebuild), `save`, `next_pending(&self, now_ms)`, `reconcile`, `record_failure` as real code in Step 4.

**M7 — Task 14, Steps 3–4: the pre-solver scheduler is prose.**
Quote: "`Presolver::start(dir:PathBuf,executor:Box<dyn PresolveExecutor>)->Presolver`; `pause(&self)`, `resume(&self)`, `status(&self)->PresolverStatus`, `notify_hand(&self,bool)`, `notify_live_request(&self)`, `shutdown(&self)`."
Only `eligible` and `next_action` have code; the named thread, the bounded command channel, the `Arc<RwLock<PresolverStatus>>`, the submit/poll/cancel loop and the "background terminal or confirmed exit before live proceeds" rule (spec §7 admission) are described, not written. This is the §10.5 deliverable.
Edit: show `Presolver::start` (thread body: command recv with timeout, `eligible` gate, `prepare`→`submit`→`poll` loop, cancel-on-live, queue save on every transition) and the `PresolverCommand` enum in Step 4.

**M8 — Task 7, Steps 4–5: the §10.4 lookup core has no code.**
Quote: "**Step 4 (5 min): Implement selection and reconstruction.** For each of three bucket keys read at most one cell/two entries, validate source before comparing, and require non-SPR key fields equal by comparing `at_bucket(0)`."
`Cache::lookup` — the three-bucket read, candidate filtering, `Comparison::rank` ordering with the accuracy tiebreak, node reconstruction (query chips at identical ordinals, `ev_over_P * P_query`, `validate_solution`, `legal_menu`), the fixed reader thread and the request-token discipline — is prose plus `map_rows`/`lookup_wait`. Same for `make_cache_query` (Step 5), which has no code at all despite an exact signature in Interfaces.
Edit: give `pub fn lookup(&self, q: &CacheQuery) -> Lookup` and `make_cache_query` as code; keep the reader-thread setup in `Cache::open` (B3).

**M9 — Task 14/15: `PresolverStatus` is not serializable and the engine delegation has no signatures.**
Quote (Task 14): "#[derive(Clone,Default)] pub struct PresolverStatus {…}"; (Task 15) "Engine `presolver_status/pause/resume` delegation for Plan5".
Plan 5 renders it directly: "`presolver_status` | `{}` | Plan 4 `PresolverStatus` serialized unchanged", against `Engine::presolver_status(&self) -> PresolverStatus`, `presolver_pause(&mut self)`, `presolver_resume(&mut self)`, and generates TS bindings from it.
Edit: derive `Serialize, Deserialize, Debug` (and `ts_rs::TS` if plan 5 binds it) on `PresolverStatus`, and state the three `Engine` signatures verbatim in Task 15's Produces.

---

## MINOR

- **m1 — Task 2, Step 5a / Step 5b:** `proto::worker::SOLVER_COMMIT` and `ADAPTER_VERSION` are consumed but plan 1 defines only `PROTO_VERSION`; plan 2 also only imports them. Name the producer or define them in Task 1.
- **m2 — Task 3, Step 3:** `for (a,b) in e.tree.materialized.iter().zip(&t.materialized)` assumes identical node ordering; the prose "Sort full materialized lists by ordinal path once during validation" is implemented nowhere (`validate_entry` neither sorts nor checks). Add a sortedness check to `validate_entry` or sort both lists in `compare`.
- **m3 — Task 4, Step 3:** an above-target hit with no reasons yields `Coverage::Approximate{reasons: vec![]}` — an Approximate with no reason, which §6 forbids ("Only reasons actually incurred are emitted"). State the intended coverage for a provisional-with-no-reasons hit (e.g. keep `Exact`-shape data but emit only in phase `Provisional`).
- **m4 — Task 5, Step 3:** prose "Reject cells whose entries have different key digests" is enforced in `decode` only, not `encode`. Add the check to `encode` before serializing.
- **m5 — Task 12, Step 1/4:** `canonical_flops_ordered()` canonicalizes all 22,100 flops with two 1326-float ranges inside a plain unit test; in a debug build this is minutes. Cache the result in a `OnceLock`, or gate the full enumeration behind `--features exhaustive` and assert 1,755 from a stored count.
- **m6 — Task 10, Step 4a:** spec §13.3 names the golden `experimental_surrogate_golden`; the plan only "adds `experimental_surrogate_golden` assertions" inside `flop_path_golden`. Create the test with the spec's name.
- **m7 — Task 20, Step 3:** `("required presolver has verified production entries", g.presolver_verified_entries>0)` is not one of §13.5's baseline-gate conditions. Keep it, but label it a non-spec informational check so a fresh install cannot fail V22 on it.
- **m8 — Task 21, Step 3/4:** `--mode`, `--temperature`, `--deadline-trials`, `--cache cold|presolved` are invoked but appear in no task's Produces (Task 17 lists only `--suite --threads --reps --out`). Add them to Task 17/18 CLI interfaces.
- **m9 — Task 16, Step 3/7:** `python tools/gen_fixtures.py e2e --check` is required in the green step but the subcommand wiring has no code. Show the `argparse` branch calling `write_e2e`/byte comparison.
- **m10 — Task 21, Step 3:** the matrix is `18 spots × 2 modes × 2 templates × 3 threads × 2 temperatures × 5 reps = 2160` flop solves at 19–64 s for the 12 SRP cells (R8 A.2) — one to two machine-days — yet the step is sized "3 min per launch/review". State the expected wall-clock per cell and that the loop is resumable.
- **m11 — Task 18, Step 4:** "Every fault fixture has its expected Unsupported reason" is asserted before Task 19 supplies `FaultyWorker`. Say explicitly that records 037–050 are skipped in Task 18 and enabled in Task 19.
- **m12 — Task 2, Step 5a:** the fixture sets `donk:Some(vec![])` on the root street; plan 2's materializer uses `None` there ("donk menus: None on the root street"). Harmless in a hand-built fixture but inconsistent with the tree the comparator will meet — use `None` for the flop entry.
- **m13 — Task 10, Step 4:** `Assumptions.cache` (`miss | exact | approximate | provisional`, §4.4) is populated nowhere; only the decision log (Task 15) records it. Assign it in the assembly step.

---

## Dimension summaries

**1. Spec coverage.** Every brief-assigned §10.4 item (key, payload, three-bucket lookup, topology/rake-cap/SPR/menu predicates, raw accuracy filter, two-entry replacement, bounded storage, quota, validation) maps to Tasks 1–8; §5 step 7 flop path to 9–11; §10.5 to 12–15; §13.5 bench/e2e/fault/gate to 16–21. All five §13.1 cache tests exist verbatim (`cache_key_structural_identity` T4 → Task 8, `cache_inherited_reasons_survive` → 4, `cache_payload_validated` → 5, `cache_corrupt_entry_deleted` → 5, `cache_atomic_write_and_quota` → 6), and the §13.3 goldens plan 2 explicitly hands over (`deadline_best_so_far_labelling`, `flop_budget_setting_golden`) are present with the spec's numbers (190/50 bp, 9600 ms wire deadline, 14,900/34,900 ms watchdog). Gaps: `set_config` rejection of 31 (M4), `experimental_surrogate_golden` by name (m6), `bench oracle` (M1), the `%LOCALAPPDATA%` cache root and 10 GiB quota (B3).

**2. Spec deviations.** No "deviations" section exists; the de-facto ones are: (a) bincode envelope with JSON metadata for tagged proto values — **justified**, bincode 1.x cannot decode internally tagged enums, and the normative JSON schema is preserved; (b) replacement reference defined as distance to `1.02^spr_bucket` — **justified**, §10.4 leaves "closest SPR" undefined and the plan confines it to storage selection; (c) solver-worker `bench-mode` feature + `--bench-storage-mode` + `bench_storage_override` capability — **justified**, §13.5 demands f32 *and* i16 on every flop spot and §10.3's production rule would pick only one, and the plan keeps it default-off in a separate target dir; (d) `flop_deadlines` duplicating §7 arithmetic — **not justified**, must follow plan 2 (M3); (e) extra gate condition (m7) — tolerable but out of §13.5. Numbers were re-derived and match the spec: `spr_bucket(5) = 81`, delta/dev cross-products, `cap_agrees` at 1100/1108 vs 55.2 chips, T4's 0.49 % root vs 0.97 % maximum, `MenuRounded{2.0}` for 100/500 vs 20/100, `f_A = 0.4682080924855491` and marginals 0.47457/0.40636 for the 73-into-100 translation, tiers 4/8/12 = 24, 1,755 flops, 7,020 tier-1 jobs, the 22 supported / 50 recorded inventory, and the §10.5 throughput figures (13 h, 52 h, 25 h, 62 h, 112 h).

**3. Placeholders.** No `TBD`/`TODO`/"similar to task N"/"add validation" strings; 142 steps, and the 41 without code are the red/green run steps plus the spec-quotation step. The real violations are the prose-only implementation steps of the plan's hardest integrations — M5, M6, M7, M8 — which the skill counts as placeholders ("Steps that describe what to do without showing how").

**4. Correctness.** Verified compile-level: `Rational` reduction and error path, `Vec<u8> == p[..i]` slice comparison, `?`-in-closure `collect::<Option<_>>`, bincode 1.3.3 `Options` chain, `zstd::stream::read::Decoder::window_log_max`, `Windows fs::rename` overwrite semantics, `serde` support for `[u8; 32]` and plan 1's hand-written `Range1326` codec, `Card` ids (Ah=50, Ad=49, combo 1274; Kh7d2c = 46/21/0), and the `available`/`probs`/`ev_chips` shapes that `validate_solution` will check. Failures found: B2 (nonexistent `SideMenu`/`MenuSize`), m1 (undefined constants), m2 (ordering assumption), m3 (empty-reason Approximate), m4 (encode check). Everything else in Tasks 1–6 should compile and the shown tests should pass as written.

**5. Task ordering.** Mostly disciplined: the storage tests deliberately use Task 2's `support::entry()` instead of Task 8's rig, bench module exports are added only when their files exist, and Task 18 avoids depending on Task 19's fault module through `RunOptions.worker_factory`. Breaks: B1 (workspace red once test templates land, and T4 cannot run before then), M2 (Task 16 modifies a file no task creates), m11 (Task 18 asserts Task 19's behaviour), M1 (Task 21 runs a subcommand nobody wrote).

**6. Interfaces.** `Cache::{open, lookup, store}`, `Lookup::{Exact, Approximate, Provisional, Miss}`, `Presolver::{start, pause, resume, status}` match §3.5 verbatim; `PresolveExecutor` is correctly a downward callback so `cache` never imports `engine` (§3.2); `snapshot_from_hit` produces plan 3's exact `StreetSnapshot`/`SnapshotKey`/`SnapshotProvenance` with `origin ∈ {cache_exact, cache_approximate, cache_provisional}` (§9.1); `validate_solution(sol, materialized)` is used with plan 1's real signature; `DecisionRecord`'s `presolver_scenario`/`tier` fields already exist in plan 2. Mismatches: B2 (menu types), B3 (`Paths.cache` required by plan 5), M9 (`PresolverStatus` not serializable, engine delegation unsigned), M5 (plan 2 module inventory wrong), m1.

**7. YAGNI.** Nothing is outside phase 1: river caching, experimental caching, V9/PokerData and the exploit slice are explicitly excluded; `flop_full_v1` is explicitly never used; tiers 2/3 are enumerated but not required for release. The `bench-mode` worker feature, the u128 ranking fields in `Comparison`, and the crossover probes are each traceable to a spec requirement (§13.5 modes, §10.4 exact ordering, V3 switch points). The only excess is measurement volume rather than scope: 2,160 flop runs (m10) plus the 22,100-flop unit test (m5).
