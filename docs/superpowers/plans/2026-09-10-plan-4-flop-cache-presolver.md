# Plan 4: Flop path, cache and pre-solver Implementation Plan

Revision 3 (2026-09-17): verification edits R2, R3, R5, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-4-CHANGELOG-3.md

Revision 2 (2026-09-17): seam re-check edits E04/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-4-CHANGELOG-2.md

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the flop decision path, pot-normalized flop/turn cache, resumable idle pre-solver, and reproducible flop/e2e/fault baseline gates.

**Architecture:** `cache` owns normalized entries, candidate selection, bounded storage, and a scheduler whose executor is supplied by `engine`. The engine supplies chart replay, materialization, worker admission, identity checks, snapshot registration, and independent final delivery; cached solutions enter the same validation and assembly path as live results. `bench` measures exact production templates and exercises the engine through Plan 2's `WorkerLink` seam without linking the solver.

**Tech Stack:** Rust edition 2021, stable Rust 1.95 or newer, MSVC pinned by Plan 1's `rust-toolchain.toml`, std threads/channels, serde, thiserror, SHA-256, bincode + zstd; Python 3.12, PokerKit 0.7.5, pytest.

**Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 7, §§2, 3.2–3.5, 4.2–4.6, 5 step 7, 6, 7, 9, 10.1–10.6, 12, 13.1/13.3/13.5, 14.4 V3/V21/V22. Approval: `docs/design/2026-09-10-design-outline.md` §0b and amended §6. Measurements: `docs/research/R8-solver-bench.md`, especially A.2/A.5/A.7. Libraries: `docs/research/R3-libraries.md`. Cross-plan resolutions: `docs/research/REVIEW-cross-plan.md` §§1, 2. REVIEW-cross-plan §§4–5 are historical. Use REVIEW-cross-plan-2 §7 after its listed corrections are accepted.

## Global Constraints

- Windows 11 desktop, private use only; 6-max NLHE cash, manual entry. Rust edition 2021; `cargo test`; `thiserror` errors; worker messages use `#[serde(tag = "type")]`; `rayon` only in the worker.
- Dependency direction strictly downward: `proto` <- `core-*` <- `cache`/`engine` <- `bench`/`pokerai-app`. Cache project dependencies: `proto`, `core-iso`, `core-ranges`. Bench: `proto`, `engine`. No crate depends on `solver-worker`.
- Engine threads are std threads, never Tokio workers: `engine-main`, `fast-path`, `watchdog`, `worker-stdin`, `worker-stdout`, `worker-stderr`, `cache-writer`, `presolver`. One worker job; admission releases on terminal result or confirmed exit, never on `ack`.
- Non-worker licenses: `MIT OR Apache-2.0`. Solver remains a separate AGPL-3.0 process at commit `9d1509fe5077d019825f833eed04b16d342dfda1`; adapter version 1; `proto_version` 3; cache `schema_version` 3; tree `rules_version` 3. Preserve Plan 2's patches and Plan 1's `+avx2` for both Windows targets.
- Wagers are integer chips (`u32`); one chip is the legal wager quantum. Rake cap: `cap_mchips: u32`, thousandths of a chip. EV: signed finite `f32` chips. `pot + stacks < 2^31`. Display `ev_bb = ev_chips / bb_chips`, rounded to 0.01 bb at render time only. Fold EV = 0 exactly.
- Public ranges never contain hero-card conditioning. Canonicalization: unordered flop, ordered turn, flop stabilizer, lexicographically minimal serialized `(oop, ip)` public-range tuple, then permutation. 1,755 canonical flops represent 22,100 flops; no nearest-flop substitution.
- `hash_scaled`: one IEEE `f32` division per weight by its maximum, sha256 of all 1326 normalized bit patterns. No lossy quantization. Power-of-two scale invariance; other rescalings may miss.
- Cache reference `P = pot_root + dead_this_street`, `eff = min(stack_oop_root, stack_ip_root)`, exact SPR rational `eff : P`; `spr_bucket = round(ln(SPR) / ln(1.02))`; search `b - 1`, `b`, `b + 1`.
- Equal full topology and terminal rake-cap predicates required; `delta = abs(SPR_query - SPR_entry) / SPR_entry <= 0.02`; `dev = abs(to_query / P_query - to_entry / P_entry)`, `max(dev) <= 0.05`; order by delta, maximum deviation, raw exploitability. `Exact` iff SPR rationals equal, every realized fraction equal, raw accuracy passes, and no inherited reasons.
- Accuracy: `exploitability_over_P <= target_bp / 10000`, raw. `SprBucketed` iff delta > 0; `MenuRounded{max_delta_pct = 100 * max(dev)}` iff maximum deviation > 0. Reasons accumulate, including `Unsupported.partial`.
- Path `%LOCALAPPDATA%\PokerAI\cache\v3\<key[0..2]>\<key>.bin`; bincode + zstd; payload sha256/schema/proto header; 64 MiB compressed, 256 MiB decoded; two entries per key cell. Default `cache_quota_bytes = 10 GiB`; oldest `last_hit` eviction. Read/decode error: miss, best-effort delete. Write failure preserves recommendation. The cache root is `engine::Paths.cache` (Plan 2 declares the field, this plan resolves and consumes it); `queue.json` is its sibling inside the same `v3` directory.
- Solver preferences: threads 16, target 50 bp, `flop_budget_s = 10`, range `1..=30`; `Engine::set_config` rejects `flop_budget_s` outside `1..=30` and `threads == 0` (§13.3 `flop_budget_setting_golden`). Outline §0b confirms default 10. From monotonic admission `t0`: first attempt river 2 s, turn 6 s, flop `flop_budget_s`; final river/turn 15 s, flop `5 s + flop_budget_s`. Worker gets remaining minus 100 ms delivery and 50 ms pipe margins; extraction river/turn 200 ms, flop 600 ms. Watchdog emits at final deadline minus 100 ms.
- Fast <= 0.3 s; validation/replay <= 0.15 s; cache lookup <= 0.5 s; equity owns 0.5 s after Fast. Cancel kill threshold 1.5 s; solving heartbeat 5 s. Suspend/resume expires requests. I/O and kill/restart cannot block final delivery.
- Worker memory: f32 estimate <= 2 GiB selects f32; else i16 estimate <= 8 GiB selects i16; else `TreeTooLarge`. Also `estimate * 1.25 <= memory_limit_bytes`, default 10 GiB; process job-object limit 16 GiB. Cache `mode` records solver storage, not extra lossy quantization.
- `flop_fast_v1`: bets 0.5, raises 2.5x, add-all-in 1.0, force-all-in 0.15, cap 3. `flop_min_v1`: bets 0.75, all-in-only raises, add-all-in 1.5, force-all-in 0.15, cap 1. Merging 0; explicit empty turn/river donks. Template change requires version bump.
- SRP lookup tries pre-solver `flop_fast_v1`, then a distinct admitted live template. Live SRP uses `flop_min_v1` iff V3 p95 <= 10 s at both 100bb and 200bb; otherwise `flop_fast_v1` to deadline. Three or more preflop wagers use `flop_fast_v1`. Classify from history, never ranges.
- Pre-solver: no active hand and no request for 30 s; background true, deadline 600000 ms, target 50, `flop_fast_v1`; three retries with 30 s backoff. Live admission cancels background. Persist pending/done/failed{n} and cursor in `queue.json`; done only while a valid at-target entry exists.
- Every validated solution reaching Final/Provisional uses `register_snapshot`; same-decision Final replaces Provisional. Baseline model revision 0. River caching, experimental caching, PokerData acquisition/conversion and exploit slice are outside this plan.
- The §6 `experimental` synthetic-root surrogate is created by this plan (cross-plan Or1/R4): no earlier plan builds it. It runs for River, Turn and Flop, never enters `SolveInput`, the cache or snapshots, and is rendered only in the separate `experimental` block.
- Each task has red/green checks and one commit. `cargo test --workspace` passes before each commit. Trailer: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

---

## Interface ownership and execution order

Execute tasks against the hard prerequisites in REVIEW-cross-plan-2 §7 after the documented seam corrections are applied. Every prerequisite must be implemented, green and reviewed before a dependent task starts. A plan number alone is not an execution dependency. Plan 5 Task 14 precedes the final Plan 4 Task 26 release decision. Serialize edits to shared files when working in one checkout, and serialize measured benchmark/acceptance runs on the benchmark machine.

Consume the spec's public surfaces without renaming:

```rust
canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm);
hash_scaled(range: &Range1326) -> [u8; 32];
block_public(range: &mut Range1326, board: &[Card]);
build_effective_tree(root: &StreetRootSnapshot, selection: &TemplateSelection)
    -> Result<EffectiveTree, UnsupportedReason>;
replay(input: ReplayInput<'_>) -> ReplayOutput;
PreflopStore::query(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize)
    -> PreflopAnswer;
Engine::register_snapshot(&mut self, &DecisionIdentity, StreetSnapshot) -> bool;
```

`Engine::register_snapshot(&mut self, &DecisionIdentity, StreetSnapshot) -> bool` is the Plan 3 Task 18 façade; live/cache internals use the same `SnapshotStore::register`. There is no free `engine::snapshots::register_snapshot` function.

`proto::worker::validate_solution`, `WorkerLink`, the clock seam and engine admission/assembly belong to Plans 1–2. Their concrete argument wrappers come from those definitions: the spec does not supply complete Rust declarations. Do not create substitute validators or a second WorkerLink. Each integration task defines its new helper signatures and maps the existing boundary explicitly. Module filenames for extensions below are owned by this plan; import prerequisite items from their actual modules.

### Resolved upstream names (cross-plan review §1; use these exact spellings)

| Item | Resolved owner and form | Consumed by |
|---|---|---|
| Bet/raise menu types (M1) | Plan 1 Task 6: `MenuSize::{Pot(f32), AllIn}` (untagged wire `0.33` / `"a"`), `SideMenu { bet: Vec<MenuSize>, raise: Vec<MenuSize> }`, `PlayerMenus { oop: SideMenu, ip: SideMenu, donk: Option<Vec<MenuSize>> }`. `donk` is `None` on the root street and `Some(vec![])` after it (Plan 2 `spec()` helper). | Tasks 2, 8 |
| Chip-path resolution (M21/D2) | Plan 1 Task 6: `proto::resolve_chip_path(&[MaterializedNode], &[Action]) -> Option<OrdinalPath>`. This plan re-exports it as `cache::entry::resolve_path`; it never re-implements the §2 rule. | Tasks 2, 7, 12 |
| Worker constants (M5/Or6/m1) | Plan 1 Task 7: `proto::worker::{PROTO_VERSION: u16 = 3, SOLVER_COMMIT: &str = "9d1509fe5077d019825f833eed04b16d342dfda1", ADAPTER_VERSION: u16 = 1}`. | Tasks 2, 5, 8 |
| `engine::Paths` (M13/Or5) | Plan 2 Task 29: `Paths { log_dir: PathBuf, worker_exe: PathBuf, preflop: PathBuf, cache: PathBuf }`. This plan reads `.cache`. `log_dir` is a directory; `DecisionLog` appends `decisions.jsonl` inside it. | Tasks 6, 7 |
| Deadlines (D5/M3) | Plan 2 Task 20 owns `Deadlines` and watchdog arithmetic: `Deadlines::for_request(t0_ms, Street, flop_budget_s)`, `street_budget_ms`, `final_delivery_ms`, `extraction_margin_ms`, `Deadlines::watchdog_fire_ms()`, `retry_admitted`. This plan adds **no** parallel deadline arithmetic. | Task 9 |
| `Engine::set_config` (M10/M4) | Plan 2 Task 29: `set_config(&mut self, GameConfig) -> Result<u32, EngineError>`. Plan 2 Task 29 already supplies validation and next-hand queuing; this plan adds tests. | Task 9 |
| `Engine::shutdown` (M11) | Plan 2 Task 29: `shutdown(&mut self)` with an idempotent flag. | Task 16 |
| `StreetRootSnapshot` (M9/S1) | Plan 1 Task 3 includes `bb_chips: u32`; every literal in this plan sets it. | Tasks 11, 12 |
| Snapshot store (M15/D1) | Plan 3 Task 13 defines snapshot records; Task 14 replaces Plan 2 Task 27's `SolvedStreet` and `SnapshotStore` with `core_replay::{SnapshotKey, SnapshotProvenance, StreetSnapshot, SnapshotStore}`. `Engine::register_snapshot(&mut self, &DecisionIdentity, StreetSnapshot) -> bool` is the Plan 3 Task 18 façade; live/cache internals use the same `SnapshotStore::register`. | Task 12 |
| `engine::bench_support` (M17/R5) | Plan 2 Task 5 creates `crates/engine/src/bench_support.rs` with `prepared_range` and `range_mass`; later tasks extend this file. This plan **modifies** it. | Tasks 17–22 |
| Test templates (M20/D10, B1) | Plan 2 Task 2 owns `Templates::with_extra(&[TemplateSpec])`, `Templates::base_ids()` and the `test-templates` feature. This plan registers `check_jam_test_v1`, `check_only_test_v1`, `menu_round_test_v1` through `with_extra` (Task 8). `Templates::base_ids().len()` stays 9; `Templates::ids()` includes registered extras. | Task 8 |
| Result assembly (M14) | Plan 2 Task 26 owns `final_from_solution` and the main recommendation assembler; `advice_rows` is not an upstream API. | Tasks 10, 11 |
| Engine test doubles | The `testing` feature is declared in Plan 2 Task 2; `FakeClock`/`FakeWorker`/`RecordingSink` are produced in Task 19. | Tasks 8–12, 15–16, 21–22 |
| Background jobs (R2) | Plan 2's `SolvePlan.background` is a real field; Plan 2 exercises only `false`. This plan sends `background: true` for pre-solver jobs. | Task 16 |

Plan 2's available interface table specifies `validate_solution(sol: &StreetSolution, materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>`; use that exact call below. Plan 2 Task 29 owns the public `Engine` façade (`new`, `startup_report`, `set_config`, `shutdown`, `set_hero_cards`, `begin_hand`, `Paths`); Task 21 owns `DecisionLog` only. Plan 2's complete engine module list after its Task 29 is `clock, identity, tree/{mod,templates,materialize,effective,signature,resolve}, worker/{mod,link,process,ready}, deadline, watchdog, core, solve, coverage, equity, allin, assemble, log, snapshots, ranges, serve, engine`, plus `testing` behind `#[cfg(any(test, feature = "testing"))]`. Extend those modules at their named symbols rather than adding duplicate deadline/log/config/serve implementations. The flop route is cut into `engine/src/serve.rs` (`serve_request`) and `engine/src/solve.rs` (`run_solve`, `SolvePlan`), which is why both appear in this plan's file structure. Plan 3 provides `PreflopStore::open(dir: &Path) -> (Self, Vec<String>)`, `PreflopStore::query`, `replay(ReplayInput) -> ReplayOutput` and sibling chart manifests `<name>.manifest.json`.

Tagged proto enums cannot be deserialized directly by bincode. Task 5 uses a binary DTO with JSON metadata for tagged proto values and binary numeric matrices, preserving the normative JSON schema.

Verified 2026-09-10: [bincode 1.3.3](https://docs.rs/bincode/1.3.3/bincode/), [zstd 0.13.3](https://docs.rs/zstd/0.13.3/zstd/), [sha2 0.10.9](https://docs.rs/sha2/0.10.9/sha2/), [thiserror 2.0.17](https://docs.rs/thiserror/2.0.17/thiserror/). Cache bincode 1.3.3 coexists with solver bincode 2.0.0-rc.3. Per cross-plan M18 the workspace pins one line of each shared crate in Plan 1's `[workspace.dependencies]` (`sha2 = "0.10.9"`, `thiserror = "2.0"`, `serde`, `serde_json`); `cache` and `engine` take them with `workspace = true` so `hash_scaled` and the cache key digest share one `Digest` trait.

### Interface notes owed to Plan 2 — all seven are satisfied by the revised Plan 2

Nothing in this list is still owed. The revised Plan 2 already declares every one of them; this plan adapts to the declared shapes and adds no competing declaration (REVIEW-cross-plan-2 §4.1).

| Note | Plan 2's declared shape | This plan's obligation |
|---|---|---|
| 1. Test-template seam | Plan 2 Task 2: `Templates::with_extra(&[TemplateSpec])`, `Templates::base_ids()`, feature `test-templates`. | Task 8 registers its three templates through `with_extra` and asserts `base_ids().len() == 9`. No Plan 2 registry rewrite, no second `Templates::get` patch. |
| 2. Fallible `set_config` | Plan 2 Task 29: `set_config(&mut self, GameConfig) -> Result<u32, EngineError>`, with blind/thread/flop validation, revision allocation and next-hand queuing. | Task 9 preserves that body and adds tests plus `flop_budget_valid`. It never replaces the implementation. |
| 3. Four `Paths` fields | Plan 2 Task 29: `Paths { log_dir, worker_exe, preflop, cache }`. | Tasks 6–7 read `.cache` and pass `log_dir` as a directory. The old `{ worker, preflop, cache, log }` spelling is gone. |
| 4. `bench_support.rs` | Plan 2 Task 5: `engine::bench_support::{prepared_range, range_mass}`. | Tasks 17–22 extend that file; this plan does not claim `parse_range`/`block_public`/`hash_scaled` re-exports it does not own. |
| 5. `crates/bench/src/lib.rs` | Plan 2 Task 5 declares the library target with `suite`, `gen_spots`, `materialize`, `runner`, `report`. | Task 20 adds `pub mod flop;` beside them. |
| 6. Background parameter | Plan 2 Task 22 owns `SolvePlan.background`; Plan 2 exercises only `false`. | Task 16 sends `true`, through the single worker owner (see Tasks 15–16). Field availability is not a working background scheduler. |
| 7. Idempotent `shutdown` | Plan 2 Task 29: `shutdown(&mut self)`, idempotent. | Task 16 extends cleanup without changing the receiver and without adding a second engine owner. |

## File structure

Generated paths expand to the inventories in Tasks 17–20. Modify existing modules at named symbols, not speculative line numbers.

| File | Responsibility |
|---|---|
| `Cargo.toml`, `Cargo.lock` | Cache member and codec dependencies |
| `crates/cache/Cargo.toml`, `crates/cache/src/lib.rs` | Crate, Cache handle, bounded I/O and errors |
| `crates/cache/src/key.rs` | Reduced rationals and canonical identity |
| `crates/cache/src/entry.rs` | Actor-owned normalized payload and validation |
| `crates/cache/src/lookup.rs` | Tree/rake/SPR/menu acceptance and node reconstruction |
| `crates/cache/src/label.rs` | Raw accuracy and inherited/incurred reasons |
| `crates/cache/src/storage.rs` | Bounded codec, header, delete, atomic writer |
| `crates/cache/src/quota.rs` | Two-entry selection and last-hit eviction |
| `crates/cache/src/presolver/mod.rs` | Presolver/executor/status interfaces |
| `crates/cache/src/presolver/queue.rs` | Durable identities, cursor, retries |
| `crates/cache/src/presolver/scenarios.rs` | Tiers and canonical-flop ordering |
| `crates/cache/src/presolver/scheduler.rs` | Idle/pause/resume/cancel scheduling |
| `crates/cache/tests/support/mod.rs` | Valid small fixtures |
| `crates/cache/tests/{key,entry,lookup,storage,quota,presolver_queue}.rs` | Cache tests |
| `crates/engine/Cargo.toml`, `crates/engine/src/lib.rs` | Wire cache into Engine |
| `crates/engine/src/core.rs` (modify) | `EngineCore.cache`, `EngineCore.flop_policy`, cache route state |
| `crates/engine/src/engine.rs` (modify) | `Paths.cache` resolution, `Engine.presolver` handle, presolver lifecycle and delegation |
| `crates/engine/src/serve.rs` (modify) | Flop/Turn street dispatch, cache route, Provisional emission |
| `crates/engine/src/solve.rs` (modify) | Retained-payload handoff and background plans |
| `crates/engine/src/testing.rs` (modify), `crates/bench/Cargo.toml` (modify) | Extend request/test seams and enable the engine testing feature for bench |
| `crates/engine/src/tree/templates.rs` (modify) | `install_cache_test_templates()` helper for T4, built on Plan 2's `Templates::with_extra` |
| `solver-worker/Cargo.toml`, `solver-worker/src/main.rs`, `solver-worker/src/memory.rs`, `solver-worker/tests/bench_mode.rs` | Opt-in diagnostic storage-mode measurements; unchanged production defaults |
| `crates/engine/src/cache_bridge.rs` | Canonical query/store and bounded lookup |
| `crates/engine/src/flop.rs` | Template and hit/provisional/live transitions |
| `crates/engine/src/experimental.rs` | §6 isolated synthetic-root surrogate |
| `crates/engine/src/deadline.rs` (modify) | `flop_budget_valid` only; all budget arithmetic stays in Plan 2's functions |
| `crates/engine/src/snapshots.rs` (modify) | Existing single registration path |
| `crates/engine/src/presolve.rs` | Chart replay and background executor |
| `crates/engine/src/log.rs` (modify) | Scenario hit rates and violations |
| `crates/engine/src/bench_support.rs` (modify; created by Plan 2 Task 5) | Facade keeping bench dependencies downward; owns `PreparedBenchSpot` |
| `crates/engine/tests/support/mod.rs` | WorkerLink/fake-clock harness extensions |
| `crates/engine/tests/cache_key_structural_identity.rs` | T4 with production materializer |
| `crates/engine/tests/flop_path_golden.rs` | Hits, provisional, budgets and identity |
| `crates/engine/tests/experimental_surrogate.rs` | §6 surrogate golden |
| `crates/engine/tests/cache_snapshot_replay.rs` | Cached prior-street translation |
| `crates/engine/tests/presolver_engine.rs` | Replay and live cancellation |
| `crates/engine/tests/golden/{cache_scale,flop_path,experimental_surrogate,cache_snapshot_replay}.json` | Reviewed expected values |
| `crates/bench/src/main.rs` (modify), `crates/bench/src/gen_spots.rs` (modify), `crates/bench/src/flop.rs` | CLI, replay spots, V3 matrix |
| `crates/bench/src/lib.rs` (modify; declared by Plan 2 Task 5) | Re-export bench modules for integration tests and share them with main |
| `crates/bench/src/e2e.rs`, `crates/bench/src/fault.rs`, `crates/bench/src/faulty_worker.rs` | Recorded inputs and WorkerLink faults |
| `crates/bench/src/report.rs` (modify), `crates/bench/src/gate.rs`, `crates/bench/src/oracle.rs` | Report, executable baseline gate and the §13.1/§13.2 oracle runner |
| `crates/bench/tests/{flop,e2e,fault,gate,oracle}.rs` | Bench contracts |
| `tools/gen_fixtures.py`, `tools/e2e_hands.py` | PokerKit generator extensions |
| `tools/tests/test_e2e_hands.py` | Inventory, legality and expectations |
| `fixtures/hands/e2e/001.json` through `fixtures/hands/e2e/050.json`, `fixtures/hands/e2e/manifest.json` | Versioned inputs and hashes |
| `bench/spots/{river_std,river_min,turn_std,turn_min,flop_fast,flop_min}.json` (modify/regenerate; created by Plan 2 Task 5), `bench/spots/sources.json` (create) | All six §13.5 baseline suites regenerated from chart replay, plus the source lock |
| `docs/bench/2026-09-10-i7-13700K.md` (modify/append; created by Plan 2 Task 30) | Measured V3/V21/V22 report, with R8 rows retained as separate reference evidence |

### Task 1: Introduce canonical cache keys and exact rational identity

**Files:** Create `crates/cache/Cargo.toml`, `crates/cache/src/{lib,key}.rs`, `crates/cache/tests/key.rs`; modify `Cargo.toml`, `Cargo.lock`.

**Interfaces:** Consumes `Card`, `Street`, `hash_scaled`; Produces `Rational::new(u64,u64)->Result<Rational,CacheError>`, `spr_bucket(Rational)->i32`, `KeyFields::digest()->[u8;32]`, `KeyFields::at_bucket(i32)->KeyFields`, `KeyFields::scenario_identity(Rational)->[u8;32]`.

- [ ] **Step 1 (3 min): Write the failing key test.**

```rust
#[test]
fn normalized_rationals_do_not_embed_chip_scale() {
    use cache::key::{Rational,spr_bucket};
    assert_eq!(Rational::new(500,100).unwrap(),Rational::new(1000,200).unwrap());
    assert_eq!(Rational::new(5000,100_000).unwrap(),Rational::new(10000,200_000).unwrap());
    assert_eq!(spr_bucket(Rational::new(500,100).unwrap()),81);
    assert!(Rational::new(1,0).is_err());
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test key`; missing package/module.
- [ ] **Step 3 (4 min): Add the workspace member and manifest.**

```toml
[package]
name = "cache"
version.workspace = true
edition.workspace = true
license.workspace = true
[features]
# Plan 5 enables this to generate the `PresolverStatus` TypeScript binding (cross-plan Or7).
typescript = ["dep:ts-rs"]
[dependencies]
proto = { path = "../proto" }
core-iso = { path = "../core-iso" }
core-ranges = { path = "../core-ranges" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
sha2 = { workspace = true }
ts-rs = { workspace = true, optional = true }
bincode = "=1.3.3"
zstd = "=0.13.3"
```

Cross-plan M18: `sha2`, `thiserror`, `serde` and `serde_json` come from Plan 1's `[workspace.dependencies]` (pinned `sha2 = "0.10.9"`), so `core_ranges::hash_scaled` and `KeyFields::digest` link one `Digest` trait. Add `"crates/cache"` to the workspace only if Plan 1's `members = ["crates/*"]` glob is not already in force; `Cargo.lock` is regenerated by the first build.

- [ ] **Step 4 (5 min): Implement reduction and key serialization.**

```rust
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use crate::CacheError;
#[derive(Clone,Copy,Debug,Eq,PartialEq,Serialize,Deserialize)]
pub struct Rational { pub num:u64,pub den:u64 }
impl Rational {
    pub fn new(num:u64,den:u64)->Result<Self,CacheError> {
        if den==0 {return Err(CacheError::Invalid("zero denominator"));}
        let (mut a,mut b)=(num,den);
        while b!=0 {(a,b)=(b,a%b);}
        Ok(Self{num:num/a,den:den/a})
    }
    pub fn value(self)->f64 {self.num as f64/self.den as f64}
}
pub fn spr_bucket(s:Rational)->i32 {(s.value().ln()/1.02_f64.ln()).round() as i32}
#[derive(Clone,Debug,Eq,PartialEq,Serialize,Deserialize)]
pub enum Model {Baseline,Locked{fingerprint:[u8;32]}}
#[derive(Clone,Debug,Eq,PartialEq,Serialize,Deserialize)]
pub struct RakeKey {pub rate_bits:u32,pub cap_over_p:Rational,pub collection_rule_version:u16}
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
pub struct KeyFields {
    pub schema_version:u16,pub solver_commit:String,pub adapter_version:u16,
    pub rules_version:u16,pub canonical_board:Vec<proto::Card>,pub root_street:proto::Street,
    pub spr_bucket:i32,pub tree_signature:String,pub rake:RakeKey,
    pub range_hash_oop:[u8;32],pub range_hash_ip:[u8;32],pub model:Model,
}
impl KeyFields {
    pub fn digest(&self)->[u8;32] {Sha256::digest(serde_json::to_vec(self).expect("finite key")).into()}
    pub fn at_bucket(&self,b:i32)->Self {let mut k=self.clone();k.spr_bucket=b;k}
    pub fn scenario_identity(&self,spr:Rational)->[u8;32] {
        Sha256::digest(serde_json::to_vec(&(self.at_bucket(0),spr)).expect("finite key")).into()
    }
}
```

`lib.rs` exports `key` and this error type. Validate positive P/eff and finite nonnegative rake before construction. Canonical bytes use fixed struct field order, integers, strings and enum tags; no unordered maps. Seats, hand id, bb, raw chips, quantum, hero, target and requested path cannot enter the key.

```rust
#[derive(Debug,thiserror::Error)]
pub enum CacheError {
    #[error("invalid cache value: {0}")] Invalid(&'static str),
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Codec(#[from] bincode::Error),
}
pub mod key;
```

- [ ] **Step 5 (3 min): Run green:** `cargo test -p cache --test key`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add Cargo.toml Cargo.lock crates/cache
git commit -m 'feat(cache): define normalized structural keys' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 2: Normalize actor-owned payloads and resolve ordinal paths

**Files:** Create `crates/cache/src/entry.rs`, `crates/cache/tests/entry.rs`, `crates/cache/tests/support/mod.rs`; modify `crates/cache/src/lib.rs`.

**Interfaces:** Consumes `StreetSolution`, `NodeStrategy`, `EffectiveTree`, `MaterializedNode`, `proto::resolve_chip_path`, `proto::worker::{validate_solution, SOLVER_COMMIT, ADAPTER_VERSION}` (Plan 1 Tasks 6/7 per cross-plan M5/M21); Produces `CacheEntry`, `CachedNode`, `SourceInputs`, `resolve_path` (a re-export of `proto::resolve_chip_path`), `chip_path(&[MaterializedNode],&[u8])->Option<ChipPath>`, `normalize(&StreetSolution,&EffectiveTree,u32)->Result<Vec<CachedNode>,CacheError>`, `validate_entry(&CacheEntry)->Result<(),CacheError>`, `sorted_by_path(&[MaterializedNode])->bool`.

- [ ] **Step 1 (4 min): Write an actor/path test with two distinct source nodes.**

```rust
#[test]
fn actor_owned_paths_are_not_root_aliases() {
    use proto::{Action,MaterializedNode,Street};
    let tree=vec![
        MaterializedNode{path:vec![],street:Street::Flop,actor:"oop".into(),
            actions:vec![Action::Check,Action::Bet{to:50}],terminal_pots:vec![None,None]},
        MaterializedNode{path:vec![0],street:Street::Flop,actor:"ip".into(),
            actions:vec![Action::Check,Action::Bet{to:50}],terminal_pots:vec![None,None]},
        MaterializedNode{path:vec![1],street:Street::Flop,actor:"ip".into(),
            actions:vec![Action::Fold,Action::Call],terminal_pots:vec![Some(100),None]},
    ];
    assert_eq!(cache::entry::resolve_path(&tree,&[Action::Bet{to:50}]),Some(vec![1]));
    assert_eq!(cache::entry::chip_path(&tree,&[0]),Some(vec![Action::Check]));
    assert_eq!(cache::entry::resolve_path(&tree,&[Action::Bet{to:51}]),None);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test entry`; missing entry module.
- [ ] **Step 3 (5 min): Define the payload and reversible path conversion.**

```rust
#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub struct SourceInputs {
    pub pot:u32,pub stack_oop:u32,pub stack_ip:u32,pub spr:crate::key::Rational,
    pub bb_chips:u32,pub quantum_over_p:crate::key::Rational,pub cap_mchips:u32,
    pub ranges:[proto::Range1326;2],
}
#[allow(non_snake_case)]
#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub struct CachedNode {
    pub path:proto::OrdinalPath,pub actor:String,pub probs:Vec<Vec<f32>>,
    pub ev_over_P:Vec<Vec<f32>>,pub available:Vec<bool>,
}
#[allow(non_snake_case)]
#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub struct CacheEntry {
    pub key:crate::key::KeyFields,pub source:SourceInputs,pub tree:proto::EffectiveTree,
    pub fractions:Vec<Vec<Option<crate::key::Rational>>>,pub nodes:Vec<CachedNode>,
    pub covered_paths:Vec<proto::OrdinalPath>,pub exploitability_over_P:f64,
    pub target_bp:u16,pub iterations:u32,pub elapsed_ms:u32,pub memory_bytes:u64,
    pub mode:String,pub locks_applied:u16,pub export:String,
    pub reasons:Vec<proto::ApproxReason>,pub created:u64,pub last_hit:u64,
}
pub fn chip_path(t:&[proto::MaterializedNode],p:&[u8])->Option<proto::ChipPath> {
    p.iter().enumerate().map(|(i,&a)|
        t.iter().find(|n|n.path==p[..i])?.actions.get(a as usize).cloned()).collect()
}
/// Cross-plan M21/D2: the §2 chip-path rule has exactly one implementation, in `proto`.
/// This crate re-exports it and never defines a second walk.
pub use proto::resolve_chip_path as resolve_path;
/// Materialized lists are compared position-by-position in Task 3, so the payload must
/// carry them in ordinal-path order with no duplicates (review m2).
pub fn sorted_by_path(t:&[proto::MaterializedNode])->bool {
    t.windows(2).all(|w|w[0].path<w[1].path)
}
```

- [ ] **Step 4 (5 min): Implement normalization after shared source validation.**

```rust
pub fn normalize(s:&proto::worker::StreetSolution,t:&proto::EffectiveTree,p:u32)
    ->Result<Vec<CachedNode>,crate::CacheError> {
    if p==0 {return Err(crate::CacheError::Invalid("zero pot"));}
    proto::worker::validate_solution(s,&t.materialized)
        .map_err(|_|crate::CacheError::Invalid("invalid solution"))?;
    s.nodes.iter().map(|n| {
        let path=resolve_path(&t.materialized,&n.path)
            .ok_or(crate::CacheError::Invalid("unresolved path"))?;
        let m=t.materialized.iter().find(|m|m.path==path).unwrap();
        if m.actor!=n.actor || m.actions!=n.actions {
            return Err(crate::CacheError::Invalid("actor or source menu"));
        }
        Ok(CachedNode{path,actor:n.actor.clone(),probs:n.probs.clone(),
            ev_over_P:n.ev_chips.iter().map(|r|r.iter().map(|v|v/p as f32).collect()).collect(),
            available:n.available.clone()})
    }).collect()
}
```

`validate_entry` reconstructs a source `StreetSolution`: nodes use `chip_path`, source materialized actions, original actor, matrices and `ev_over_P * source.pot`; `covered_paths` uses the same conversion; requested is the first exported node. Call `proto::worker::validate_solution` with this solution and source tree using Plan 1's signature, mapping any error to `CacheError::Invalid("invalid solution")`. Require unique ordinal paths, current-root-street exports, source ranges of 1326 finite [0,1] values with positive mass, checked pot/stacks, canonical board/range hashes, schema/proto/solver/adapter/rules versions, source SPR/key bucket and rational fractions recomputed exactly, finite nonnegative exploitability, allowed mode, export and locks/model agreement. An entry is never a Recommendation or request identity.

- [ ] **Step 5 (4 min): Add `support::rows(actions:usize)->(Vec<Vec<f32>>,Vec<Vec<f32>>,Vec<bool>)` for tests; use valid board-compatible combo indices supplied by each test.**

```rust
pub fn rows(actions:usize)->(Vec<Vec<f32>>,Vec<Vec<f32>>,Vec<bool>) {
    let mut p=vec![vec![0.0;actions];1326];let mut ev=p.clone();let mut a=vec![false;1326];
    // AhAd index: Card(50), Card(49); fixture board Kh7d2c does not block them.
    let i=50*49/2+49;a[i]=true;
    p[i].fill(1.0/actions as f32);
    for j in 0..actions {ev[i][j]=j as f32*10.0;}
    (p,ev,a)
}
```

- [ ] **Step 5a (5 min): Define `support::entry() -> CacheEntry` now so storage tests never depend on Task 8.** Its complete check/jam skeleton has six check-line nodes across three streets, each with its own jam response; only the four flop nodes are exported. Each later cache test imports this helper with `mod support`.

```rust
pub fn entry()->cache::entry::CacheEntry {
    use proto::{Action,Street,MaterializedNode,EffectiveTree,PlayerMenus,SideMenu,MenuSize};
    use cache::{entry::{CacheEntry,SourceInputs},key::{KeyFields,RakeKey,Rational,Model}};
    let original=vec![proto::Card(46),proto::Card(21),proto::Card(0)];
    let r=core_ranges::parse_range("AA").unwrap();
    let (_,perm)=core_iso::canonicalize(&original,&[&r,&r]);
    let mut board=original.iter().map(|c|core_iso::apply(&perm,*c)).collect::<Vec<_>>();
    board.sort_by_key(|c|c.0);
    let r=core_iso::apply_range(&perm,&r);let hash=core_ranges::hash_scaled(&r);
    let mut materialized=Vec::new();
    for n in 0..6 {
        let street=[Street::Flop,Street::Turn,Street::River][n/2];
        let actor=if n%2==0 {"oop"}else{"ip"};let other=if n%2==0 {"ip"}else{"oop"};
        materialized.push(MaterializedNode{path:vec![0;n],street,actor:actor.into(),
            actions:vec![Action::Check,Action::AllIn{to:500}],
            terminal_pots:vec![if n==5 {Some(100)}else{None},None]});
        let mut facing=vec![0;n];facing.push(1);
        materialized.push(MaterializedNode{path:facing,street,actor:other.into(),
            actions:vec![Action::Fold,Action::Call],terminal_pots:vec![Some(100),Some(1100)]});
    }
    materialized.sort_by(|a,b|a.path.cmp(&b.path));
    let side=SideMenu{bet:vec![MenuSize::AllIn],raise:vec![MenuSize::AllIn]};
    // review m12 / Plan 2 `spec()`: donk is None on the root street, an explicit empty list after it.
    let menus=[Street::Flop,Street::Turn,Street::River].into_iter().map(|street|
        (street,PlayerMenus{oop:side.clone(),ip:side.clone(),
            donk:if street==Street::Flop {None} else {Some(vec![])}})).collect();
    let tree=EffectiveTree{rules_version:3,template_id:"check_jam_test_v1".into(),root_street:Street::Flop,
        menus,add_allin_threshold:0.0,force_allin_threshold:0.0,merging_threshold:0.0,
        wager_cap:1,inserted:vec![],materialized};
    let nodes=tree.materialized.iter().filter(|n|n.street==Street::Flop).enumerate().map(|(k,n)| {
        let (mut probs,mut ev_chips,mut available)=rows(2);
        for i in 0..1326 {available[i]=r.0[i]>0.0;
            probs[i]=if available[i] {vec![0.5,0.5]}else{vec![0.0,0.0]};
            ev_chips[i]=if available[i] {vec![0.0,10.0]}else{vec![0.0,0.0]};}
        if matches!(n.actions[0],Action::Fold) {for row in &mut ev_chips {row[0]=0.0;}}
        for (i,row) in ev_chips.iter_mut().enumerate() {if available[i] {row[1]+=k as f32;}}
        proto::worker::NodeStrategy{path:cache::entry::chip_path(&tree.materialized,&n.path).unwrap(),
            actor:n.actor.clone(),actions:n.actions.clone(),probs,ev_chips,available}
    }).collect::<Vec<_>>();
    let solution=proto::worker::StreetSolution{covered_paths:nodes.iter().map(|n|n.path.clone()).collect(),
        nodes,requested:0,exploitability_chips:0.4,iterations:100,memory_bytes:1024,
        mode:"f32".into(),locks_applied:0,export:"street".into()};
    let nodes=cache::entry::normalize(&solution,&tree,100).unwrap();
    let fractions=tree.materialized.iter().map(|n|n.actions.iter().map(|a|match a {
        Action::Bet{to}|Action::Raise{to}|Action::AllIn{to}=>Some(Rational::new(*to as u64,100).unwrap()),
        _=>None}).collect()).collect();
    CacheEntry{key:KeyFields{schema_version:3,solver_commit:proto::worker::SOLVER_COMMIT.into(),
        adapter_version:1,rules_version:3,canonical_board:board,root_street:Street::Flop,spr_bucket:81,
        tree_signature:"check_jam_test_v1".into(),rake:RakeKey{rate_bits:0.05_f32.to_bits(),
            cap_over_p:Rational::new(5000,100000).unwrap(),collection_rule_version:1},
        range_hash_oop:hash,range_hash_ip:hash,model:Model::Baseline},
        source:SourceInputs{pot:100,stack_oop:500,stack_ip:500,spr:Rational::new(5,1).unwrap(),
            bb_chips:2,quantum_over_p:Rational::new(1,100).unwrap(),cap_mchips:5000,ranges:[r.clone(),r]},
        tree,fractions,covered_paths:nodes.iter().map(|n|n.path.clone()).collect(),nodes,
        exploitability_over_P:0.004,target_bp:50,iterations:100,elapsed_ms:10,memory_bytes:1024,
        mode:"f32".into(),locks_applied:0,export:"street".into(),reasons:vec![],created:1,last_hit:1}
}
```

The synthetic key signature above is a test fixture value; T4 uses the real structural SHA-256. Keep fixture-only signatures out of all production cache paths. Remap the support row's AA combo through `perm` as well if the selected canonical permutation moves its suits; AA class support remains positive, and actor-distinct rows remain identifiable.

- [ ] **Step 5b (5 min): Implement source reconstruction in `validate_entry` before introducing storage.**

```rust
pub fn validate_entry(e:&CacheEntry)->Result<(),crate::CacheError> {
    use crate::CacheError;use crate::key::{Rational,spr_bucket};
    let bad=||CacheError::Invalid("invalid solution");let s=&e.source;
    if s.pot==0||s.stack_oop==0||s.stack_ip==0||s.bb_chips==0||
        s.pot as u64+s.stack_oop as u64+s.stack_ip as u64>=1_u64<<31||
        e.key.schema_version!=3||e.key.rules_version!=3||e.tree.rules_version!=3||
        e.key.adapter_version!=proto::worker::ADAPTER_VERSION||
        e.key.solver_commit!=proto::worker::SOLVER_COMMIT||
        !matches!(e.key.root_street,proto::Street::Flop|proto::Street::Turn)||
        e.tree.root_street!=e.key.root_street||!e.exploitability_over_P.is_finite()||
        e.exploitability_over_P<0.0||!matches!(e.mode.as_str(),"f32"|"i16")||
        e.nodes.is_empty()||e.nodes.len()>100_000 {return Err(bad());}
    if s.spr!=Rational::new(s.stack_oop.min(s.stack_ip) as u64,s.pot as u64)?||
        e.key.spr_bucket!=spr_bucket(s.spr)||e.key.rake.cap_over_p!=Rational::new(s.cap_mchips as u64,1000*s.pot as u64)? {
        return Err(bad());
    }
    // review m2: Task 3 zips the two materialized lists positionally, so ordering is a payload invariant.
    if !sorted_by_path(&e.tree.materialized)||e.tree.materialized.is_empty() {return Err(bad());}
    let board=&e.key.canonical_board;
    let length=if e.key.root_street==proto::Street::Flop {3}else{4};
    if board.len()!=length||board.iter().any(|c|c.0>=52)||
        board.iter().map(|c|c.0).collect::<std::collections::BTreeSet<_>>().len()!=board.len()||
        !f32::from_bits(e.key.rake.rate_bits).is_finite()||
        !(0.0..=1.0).contains(&f32::from_bits(e.key.rake.rate_bits))||
        s.quantum_over_p.num==0||s.quantum_over_p.den==0||
        !matches!(e.export.as_str(),"street"|"truncated")||
        (matches!(e.key.model,crate::key::Model::Baseline)&&e.locks_applied!=0) {return Err(bad());}
    let fractions=e.tree.materialized.iter().map(|n|n.actions.iter().map(|a|match a {
        proto::Action::Bet{to}|proto::Action::Raise{to}|proto::Action::AllIn{to}=>
            Some(Rational::new(*to as u64,s.pot as u64).unwrap()),_=>None}).collect::<Vec<_>>()).collect::<Vec<_>>();
    if fractions!=e.fractions {return Err(bad());}
    for (range,hash) in s.ranges.iter().zip([e.key.range_hash_oop,e.key.range_hash_ip]) {
        if range.0.iter().any(|x|!x.is_finite()||!(0.0..=1.0).contains(x))||
            !range.0.iter().any(|x|*x>0.0)||core_ranges::hash_scaled(range)!=hash {return Err(bad());}
        for hi in 1_u8..52 {for lo in 0_u8..hi {
            if board.iter().any(|c|c.0==hi||c.0==lo)&&range.0[hi as usize*(hi as usize-1)/2+lo as usize]!=0.0 {
                return Err(bad());
            }
        }}
    }
    let (_,perm)=core_iso::canonicalize(board,&[&s.ranges[0],&s.ranges[1]]);
    let mut canonical=board.iter().map(|c|core_iso::apply(&perm,*c)).collect::<Vec<_>>();
    canonical[..3].sort_by_key(|c|c.0);
    if canonical!=*board||core_iso::apply_range(&perm,&s.ranges[0])!=s.ranges[0]||
        core_iso::apply_range(&perm,&s.ranges[1])!=s.ranges[1] {return Err(bad());}
    if e.covered_paths!=e.nodes.iter().map(|n|n.path.clone()).collect::<Vec<_>>()||
        e.covered_paths.iter().collect::<std::collections::BTreeSet<_>>().len()!=e.nodes.len() {return Err(bad());}
    let mut nodes=Vec::new();
    for n in &e.nodes {
        let m=e.tree.materialized.iter().find(|m|m.path==n.path).ok_or_else(bad)?;
        if m.actor!=n.actor||m.street!=e.key.root_street {return Err(bad());}
        nodes.push(proto::worker::NodeStrategy{path:chip_path(&e.tree.materialized,&n.path).ok_or_else(bad)?,
            actor:n.actor.clone(),actions:m.actions.clone(),probs:n.probs.clone(),
            ev_chips:n.ev_over_P.iter().map(|r|r.iter().map(|x|x*s.pot as f32).collect()).collect(),
            available:n.available.clone()});
    }
    let sol=proto::worker::StreetSolution{covered_paths:nodes.iter().map(|n|n.path.clone()).collect(),nodes,
        requested:0,exploitability_chips:(e.exploitability_over_P*s.pot as f64) as f32,
        iterations:e.iterations,memory_bytes:e.memory_bytes,mode:e.mode.clone(),locks_applied:e.locks_applied,
        export:e.export.clone()};
    proto::worker::validate_solution(&sol,&e.tree.materialized).map_err(|_|bad())?;Ok(())
}
```

Board blocking, canonical identity and fraction checks precede numeric matrix reconstruction. No validator needs a later-task fixture. Source key digests are checked against the filename/cell by the reader; production structural signatures are supplied by the existing engine signature builder (`engine::tree::tree_signature`) and verified again against each query before serving. `proto::worker::{SOLVER_COMMIT, ADAPTER_VERSION}` come from Plan 1 Task 7 (cross-plan M5/Or6); this plan never redefines them. Add a `sorted_by_path` mutation case to `crates/cache/tests/entry.rs`: swapping two materialized nodes must fail `validate_entry`.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): normalize actor-owned street payloads' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 3: Compare complete trees, SPR and terminal rake-cap activation

**Files:** Create `crates/cache/src/lookup.rs`, `crates/cache/tests/lookup.rs`; modify `crates/cache/src/lib.rs`.

**Interfaces:** Consumes `CacheEntry`, `MaterializedNode`; Produces `Comparison{delta:f64,max_dev:f64,delta_num:u128,delta_den:u128,menu_num:u128,menu_den:u128}`, `compare(e:&CacheEntry,tree:&EffectiveTree,p:u32,eff:u32,cap_mchips:u32)->Option<Comparison>`, `action_to(&Action)->Option<u32>`, `cap_agrees(f64,u32,u32,u32,u32)->bool`.

- [ ] **Step 1 (3 min): Write the independent rake boundary test.**

```rust
#[test]
fn terminal_rake_cap_changes_without_topology_changes() {
    let r=0.05_f32 as f64;
    assert!(r*1100.0<55.2);assert!(r*1108.0>=55.2);
    assert!(!cache::lookup::cap_agrees(r,55200,1100,55200,1108));
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test lookup`.
- [ ] **Step 3 (5 min): Implement full-skeleton checks, exact inclusive thresholds, and ranking values.** Keep numerator/denominator pairs in Comparison so candidate ordering uses exact cross-products; f64 fields are display values. All cross-products fit u128 under the checked monetary bound.

```rust
pub struct Comparison {
    pub delta:f64,pub max_dev:f64,
    pub delta_num:u128,pub delta_den:u128,pub menu_num:u128,pub menu_den:u128,
}
impl Comparison {
    pub fn rank(&self,other:&Self)->std::cmp::Ordering {
        (self.delta_num*other.delta_den).cmp(&(other.delta_num*self.delta_den))
            .then((self.menu_num*other.menu_den).cmp(&(other.menu_num*self.menu_den)))
    }
}
pub fn action_to(a:&proto::Action)->Option<u32> {
    match a {proto::Action::Bet{to}|proto::Action::Raise{to}|proto::Action::AllIn{to}=>Some(*to),_=>None}
}
pub fn cap_agrees(rate:f64,ce:u32,pe:u32,cq:u32,pq:u32)->bool {
    (rate*pe as f64>=ce as f64/1000.0)==(rate*pq as f64>=cq as f64/1000.0)
}
pub fn compare(e:&crate::entry::CacheEntry,t:&proto::EffectiveTree,p:u32,eff:u32,cap:u32)
    ->Option<Comparison> {
    if p==0||eff==0 {return None;}
    let pe=e.source.pot as u128;let pq=p as u128;
    let ee=e.source.stack_oop.min(e.source.stack_ip) as u128;
    let difference=(eff as u128*pe).abs_diff(ee*pq);
    if 100*difference>2*ee*pq {return None;}
    // review m2: both lists are ordinal-path sorted (`validate_entry` for the entry, the
    // materializer for the query), so a positional zip is a total comparison.
    if e.tree.materialized.len()!=t.materialized.len()
        ||!crate::entry::sorted_by_path(&e.tree.materialized)
        ||!crate::entry::sorted_by_path(&t.materialized) {return None;}
    let mut maximum=0_u128;
    for (a,b) in e.tree.materialized.iter().zip(&t.materialized) {
        if a.path!=b.path||a.actor!=b.actor||a.street!=b.street||a.actions.len()!=b.actions.len()
            ||a.terminal_pots.len()!=a.actions.len()||b.terminal_pots.len()!=b.actions.len() {return None;}
        for i in 0..a.actions.len() {
            if std::mem::discriminant(&a.actions[i])!=std::mem::discriminant(&b.actions[i]) {return None;}
            match (a.terminal_pots[i],b.terminal_pots[i]) {
                (None,None)=>{},
                (Some(x),Some(y)) if cap_agrees(f32::from_bits(e.key.rake.rate_bits) as f64,
                    e.source.cap_mchips,x,cap,y)=>{},_=>return None,
            }
            if let (Some(x),Some(y))=(action_to(&a.actions[i]),action_to(&b.actions[i])) {
                maximum=maximum.max((y as u128*pe).abs_diff(x as u128*pq));
            }
        }
    }
    if 100*maximum>5*pe*pq {return None;}
    Some(Comparison{delta:difference as f64/(ee*pq) as f64,max_dev:maximum as f64/(pe*pq) as f64,
        delta_num:difference,delta_den:ee*pq,menu_num:maximum,menu_den:pe*pq})
}
```

Sortedness is enforced by `validate_entry` (Task 2 `sorted_by_path`), which also rejects duplicate paths; `compare` re-checks both lists rather than assuming the caller sorted them. Compare all streets, not just exported nodes or root. A min-raise/all-in/deduplication/wager-cap boundary changing kinds, paths, actors, streets or terminal classification must miss. Cap activation is checked even when topology is identical. Use source and query absolute caps after normalized rake-key equality.

- [ ] **Step 4 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 5 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): enforce full-tree SPR menu and rake predicates' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 4: Preserve inherited reasons and filter raw accuracy

**Files:** Create `crates/cache/src/label.rs`; modify `crates/cache/src/lib.rs`, `crates/cache/tests/lookup.rs` (add `mod support;` so the new label test can build Task 2's fixture).

**Interfaces:** Consumes `ApproxReason`, `Coverage`, `Comparison`; Produces `accuracy_ok(f64,u16)->bool`, `merge_reasons(&[ApproxReason],&[ApproxReason])->Vec<ApproxReason>`, `label(&CacheEntry,Rational,&Comparison,u16,&[ApproxReason])->(Coverage,bool)`; bool denotes Provisional.

- [ ] **Step 1 (3 min): Write `cache_inherited_reasons_survive`.**

```rust
#[test]
fn cache_inherited_reasons_survive() {
    use cache::label::{accuracy_ok,merge_reasons};use proto::ApproxReason;
    assert!(!accuracy_ok(0.005049,50));assert!(accuracy_ok(0.005049,51));
    assert!(accuracy_ok(0.005,50));assert!(!accuracy_ok(f64::NAN,50));
    let reasons=vec![ApproxReason::ChartRounded,
        ApproxReason::DeadlineBestSoFar{reached_bp:190,target_bp:50},
        ApproxReason::UnconditionedPriorStreet{street:proto::Street::Flop,seat:proto::Seat(2),
            cause:"uncovered path []".into()}];
    assert_eq!(merge_reasons(&reasons,&[ApproxReason::ChartRounded]).len(),3);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache cache_inherited_reasons_survive`.
- [ ] **Step 3 (5 min): Implement raw accuracy and labels.**

```rust
pub fn accuracy_ok(raw:f64,target:u16)->bool {raw.is_finite()&&raw>=0.0&&raw<=target as f64/10000.0}
pub fn merge_reasons(a:&[proto::ApproxReason],b:&[proto::ApproxReason])->Vec<proto::ApproxReason> {
    let mut out=Vec::new();let mut seen=std::collections::BTreeSet::new();
    for r in a.iter().chain(b) {
        if seen.insert(serde_json::to_vec(r).expect("validated reason")) {out.push(r.clone());}
    }
    out
}
pub fn label(e:&crate::entry::CacheEntry,spr:crate::key::Rational,c:&crate::lookup::Comparison,
    target:u16,query_reasons:&[proto::ApproxReason])->(proto::Coverage,bool) {
    let mut reasons=merge_reasons(query_reasons,&e.reasons);
    if spr!=e.source.spr {reasons.push(proto::ApproxReason::SprBucketed{
        actual:spr.value() as f32,used:e.source.spr.value() as f32});}
    if c.max_dev>0.0 {reasons.push(proto::ApproxReason::MenuRounded{max_delta_pct:(100.0*c.max_dev) as f32});}
    let provisional=!accuracy_ok(e.exploitability_over_P,target);
    // review m3 / §6 "only reasons actually incurred are emitted": an above-target hit that
    // incurred no reason must not become `Approximate{reasons: []}`. It keeps `Coverage::Exact`
    // data and is disclosed solely by `Phase::Provisional` plus the raw reached exploitability.
    (if reasons.is_empty() {proto::Coverage::Exact}
        else {proto::Coverage::Approximate{reasons}},provisional)
}
```

Add the boundary case to the same test file:

```rust
#[test]
fn provisional_without_reasons_is_never_an_empty_approximate() {
    let entry=support::entry();                       // exploitability_over_P = 0.004, no reasons
    let comparison=cache::lookup::Comparison{delta:0.0,max_dev:0.0,
        delta_num:0,delta_den:1,menu_num:0,menu_den:1};
    let (coverage,provisional)=cache::label::label(&entry,entry.source.spr,&comparison,30,&[]);
    assert!(provisional,"0.004 > 30 bp");
    assert!(matches!(coverage,proto::Coverage::Exact));
}
```

Provisional carries exactly its inherited/query/incurred reasons; phase and raw reached value disclose accuracy. Do not invent a new deadline reason for a source solve that met its own looser target. Never remove a stored `DeadlineBestSoFar` when a looser target passes. Never certify current timing from an old deadline reason. A separately stored refined entry may have different reasons. Repeat label tests with f32/i16 entries; mode is disclosed and does not change raw accuracy or labels.

- [ ] **Step 4 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 5 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): preserve reasons and unrounded accuracy' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 5: Add bounded binary storage and delete corrupt entries

**Files:** Create `crates/cache/src/storage.rs`, `crates/cache/tests/storage.rs`; modify `crates/cache/src/lib.rs`.

**Interfaces:** Consumes `CacheEntry`, `validate_entry`; Produces `Cell{entries:Vec<CacheEntry>}`, `encode(&Cell)->Result<Vec<u8>,CacheError>`, `decode(&[u8])->Result<Cell,CacheError>`, `entry_path(&Path,[u8;32])->PathBuf`, `read_cell(&Path)->Option<Cell>`. Boundaries are per cell, including its two entries.

- [ ] **Step 1 (3 min): Write header rejection tests before the codec.**

```rust
#[test]
fn cache_corrupt_entry_deleted() {
    let dir=std::env::temp_dir().join(format!("pokerai-cache-corrupt-{}",std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();let path=dir.join("bad.bin");
    std::fs::write(&path,b"not a cache header").unwrap();
    assert!(cache::storage::read_cell(&path).is_none());assert!(!path.exists());
    // A directory cannot be removed as a file: deletion failure is still a miss.
    assert!(cache::storage::read_cell(&dir).is_none());
    std::fs::remove_dir(&dir).unwrap();
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test storage`.
- [ ] **Step 3 (5 min): Define the fixed header and binary DTO.** Header is 56 bytes: magic 4 (`PAI3`), schema u16 LE, proto u16 LE, compressed length u64 LE, decoded length u64 LE, decoded-payload sha256 32. Validate header before allocation; require the actual file length to equal header + declared compressed length.

```rust
use std::io::Read;
use bincode::Options;
use sha2::{Digest,Sha256};
use crate::{CacheError,entry::CacheEntry};
pub const COMPRESSED_MAX:u64=64*1024*1024;
pub const DECODED_MAX:u64=256*1024*1024;
pub struct Cell {pub entries:Vec<CacheEntry>}
#[derive(serde::Serialize,serde::Deserialize)]
struct DiskEntry {metadata:Vec<u8>,nodes:Vec<crate::entry::CachedNode>}
struct CappedBuffer {bytes:Vec<u8>,limit:usize}
impl std::io::Write for CappedBuffer {
    fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {
        if bytes.len()>self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"cache size limit"));
        }
        self.bytes.extend_from_slice(bytes);Ok(bytes.len())
    }
    fn flush(&mut self)->std::io::Result<()> {Ok(())}
}
fn bounded_json<T:serde::Serialize>(value:&T)->Result<Vec<u8>,CacheError> {
    let mut out=CappedBuffer{bytes:Vec::new(),limit:DECODED_MAX as usize};
    serde_json::to_writer(&mut out,value)?;Ok(out.bytes)
}
fn options()->impl Options {
    bincode::DefaultOptions::new().with_fixint_encoding().with_little_endian()
        .with_limit(DECODED_MAX).reject_trailing_bytes()
}
pub fn entry_path(dir:&std::path::Path,key:[u8;32])->std::path::PathBuf {
    let hex=key.iter().map(|b|format!("{b:02x}")).collect::<String>();
    dir.join(&hex[..2]).join(format!("{hex}.bin"))
}
pub fn encode(cell:&Cell)->Result<Vec<u8>,CacheError> {
    if cell.entries.is_empty()||cell.entries.len()>2 {return Err(CacheError::Invalid("cell count"));}
    // review m4: the digest agreement is a writer invariant too, not only a reader check.
    if cell.entries.iter().any(|e|e.key.digest()!=cell.entries[0].key.digest()) {
        return Err(CacheError::Invalid("cell key digest"));
    }
    let mut disk=Vec::new();
    for e in &cell.entries {
        crate::entry::validate_entry(e)?;
        let mut metadata=e.clone();metadata.nodes.clear();
        disk.push(DiskEntry{metadata:bounded_json(&metadata)?,nodes:e.nodes.clone()});
    }
    if options().serialized_size(&disk)?>DECODED_MAX {return Err(CacheError::Invalid("decoded size"));}
    let payload=options().serialize(&disk)?;
    let compressed=zstd::stream::encode_all(payload.as_slice(),3)?;
    if compressed.len() as u64>COMPRESSED_MAX {return Err(CacheError::Invalid("compressed size"));}
    let mut out=b"PAI3".to_vec();out.extend(3_u16.to_le_bytes());out.extend(3_u16.to_le_bytes());
    out.extend((compressed.len() as u64).to_le_bytes());out.extend((payload.len() as u64).to_le_bytes());
    out.extend(Sha256::digest(&payload));out.extend(compressed);Ok(out)
}
```

Metadata contains exact proto values, source ranges, tree and reasons; numerical node matrices are stored by bincode, not strings. Cells whose entries have different key digests are rejected by both `encode` and `decode`; add a `cargo test -p cache --test storage` case that builds a two-entry `Cell` with one mutated `spr_bucket` and asserts `encode` returns `Err`. Serialization is bounded too: wrap JSON/bincode writers with a capped writer returning `InvalidData` above 256 MiB rather than allowing an unbounded oversized metadata allocation.

- [ ] **Step 4 (5 min): Implement bounded streaming decompression and validation.**

```rust
pub fn decode(bytes:&[u8])->Result<Cell,CacheError> {
    let invalid=||CacheError::Invalid("header or payload");
    if bytes.len()<56||&bytes[..4]!=b"PAI3"||bytes[4..8]!=[3,0,3,0] {return Err(invalid());}
    let compressed=u64::from_le_bytes(bytes[8..16].try_into().unwrap());
    let decoded=u64::from_le_bytes(bytes[16..24].try_into().unwrap());
    if compressed>COMPRESSED_MAX||decoded>DECODED_MAX||bytes.len() as u64!=56+compressed {return Err(invalid());}
    let mut reader=zstd::stream::read::Decoder::new(&bytes[56..])?;
    reader.window_log_max(28)?;
    let mut payload=Vec::new();reader.take(DECODED_MAX+1).read_to_end(&mut payload)?;
    if payload.len() as u64!=decoded||Sha256::digest(&payload).as_slice()!=&bytes[24..56] {return Err(invalid());}
    let disk:Vec<DiskEntry>=options().deserialize(&payload)?;
    if disk.is_empty()||disk.len()>2 {return Err(invalid());}
    let mut entries=Vec::new();
    for d in disk {
        let mut e:CacheEntry=serde_json::from_slice(&d.metadata)?;e.nodes=d.nodes;
        crate::entry::validate_entry(&e)?;entries.push(e);
    }
    if entries.iter().any(|e|e.key.digest()!=entries[0].key.digest()) {return Err(invalid());}
    Ok(Cell{entries})
}
pub fn read_cell(path:&std::path::Path)->Option<Cell> {
    let result=(||->Result<Cell,CacheError>{
        let f=std::fs::File::open(path)?;
        if f.metadata()?.len()>56+COMPRESSED_MAX {return Err(CacheError::Invalid("file size"));}
        let mut bytes=Vec::new();f.take(57+COMPRESSED_MAX).read_to_end(&mut bytes)?;decode(&bytes)
    })();
    match result {Ok(cell)=>Some(cell),Err(_)=>{let _=std::fs::remove_file(path);None}}
}
```

- [ ] **Step 5 (4 min): Add `cache_payload_validated`.** Use Task 2's `support::entry()` now, not a later task. Write each mutation with a recomputed header/hash; corruption must be caught by content validation, not just checksum. Parameter cases: available row `[0.2,0.2]`, `[-0.1,1.1]`, `[1.1,-0.1]`, wrong 1326 shape, nonzero unavailable EV/probability, invalid requested/path, covered paths out of order, wrong actor, duplicate path, unmatched materialized node, nonfinite EV/exploitability, wrong versions. Every corrupt cell yields miss and deletion. Separate tests flip a compressed byte, truncate the header, declare 64 MiB + 1 compressed, and create a zstd bomb exceeding 256 MiB decoded.

```rust
#[test]
fn cache_payload_validated() {
    for row in [vec![0.2,0.2],vec![-0.1,1.1],vec![1.1,-0.1]] {
        let mut e=support::entry();let i=e.nodes[0].available.iter().position(|x|*x).unwrap();
        e.nodes[0].probs[i]=row;assert!(cache::entry::validate_entry(&e).is_err());
    }
    let mut e=support::entry();e.covered_paths.reverse();
    assert!(cache::entry::validate_entry(&e).is_err());
}
```

The test-only unchecked writer uses the same binary representation with `Vec<(Vec<u8>,Vec<CachedNode>)>` (same field sequence as DiskEntry), then reconstructs the 56-byte header and zstd payload; it deliberately skips validate_entry solely to test the reader's content checks. Do not export an unchecked writer from cache.

```rust
#[test]
fn bounded_header_rejects_oversized_lengths_before_decode() {
    let mut bytes=vec![0_u8;56];bytes[..8].copy_from_slice(b"PAI3\x03\x00\x03\x00");
    bytes[8..16].copy_from_slice(&(cache::storage::COMPRESSED_MAX+1).to_le_bytes());
    assert!(cache::storage::decode(&bytes).is_err());
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): validate bounded checksummed disk cells' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 6: Write atomically, retain two representatives and enforce quota

**Files:** Create `crates/cache/src/quota.rs`, `crates/cache/tests/quota.rs`; modify `crates/cache/src/{lib,storage}.rs`, `crates/cache/tests/storage.rs`.

**Interfaces:** Consumes `CacheEntry`, `encode(&Cell)` and `read_cell(&Path)` from Tasks 2/5. Produces `retain_two(Vec<CacheEntry>)->Vec<CacheEntry>`, `IndexRow{key:[u8;32],bytes:u64,last_hit:u64}`, `victims(&[IndexRow],u64)->Vec<[u8;32]>`, `write_atomic(&Path,&[u8])->Result<(),CacheError>`, `CACHE_QUOTA_BYTES: u64`, `default_cache_root()->PathBuf`, `Cache::open(root:PathBuf,quota_bytes:u64)->Cache`, `Cache::root(&self)->&Path`, `Cache::disabled()->Cache`, `Cache::store(&self,&CacheEntry)`, `Cache::store_tracked(&self,&CacheEntry)->StoreReceipt`, `Cache::touch(&self,key:[u8;32],payload_digest:Vec<u8>,last_hit:u64)`, `Cache::shutdown(&self)`, `StoreReceipt::wait(self,std::time::Duration)->bool`; store is best-effort/nonblocking. `store_tracked`'s receipt lets the pre-solver await durable completion away from live paths.

- [ ] **Step 1 (3 min): Write oldest-first eviction and atomic replacement tests.**

```rust
#[test]
fn quota_uses_last_hit_not_creation_order() {
    use cache::quota::{IndexRow,victims};
    let rows=vec![IndexRow{key:[1;32],bytes:60,last_hit:9},IndexRow{key:[2;32],bytes:60,last_hit:3}];
    assert_eq!(victims(&rows,100),vec![[2;32]]);
}
#[test]
fn cache_atomic_write_and_quota() {
    let path=std::env::temp_dir().join(format!("pokerai-atomic-{}.bin",std::process::id()));
    cache::storage::write_atomic(&path,b"old").unwrap();
    cache::storage::write_atomic(&path,b"new").unwrap();
    assert_eq!(std::fs::read(&path).unwrap(),b"new");std::fs::remove_file(path).unwrap();
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test quota`; missing quota functions.
- [ ] **Step 3 (5 min): Implement deterministic replacement and eviction.** For the spec's otherwise unspecified replacement reference, define closest SPR relative to the cell's geometric center `1.02^spr_bucket`. This is a storage-selection rule only; lookup always measures distance to the actual query. Keep the closest and most accurate representatives, deduplicating when one wins both; ties use canonical payload digest, never arrival order. A candidate both farther and less accurate cannot replace either representative.

```rust
pub struct IndexRow {pub key:[u8;32],pub bytes:u64,pub last_hit:u64}
pub fn victims(rows:&[IndexRow],quota:u64)->Vec<[u8;32]> {
    let mut order=rows.iter().collect::<Vec<_>>();order.sort_by_key(|r|(r.last_hit,r.key));
    let mut used: u128=rows.iter().map(|r|r.bytes as u128).sum();let mut out=Vec::new();
    for r in order {if used<=quota as u128 {break;}used-=r.bytes as u128;out.push(r.key);}
    out
}
pub fn retain_two(mut entries:Vec<crate::entry::CacheEntry>)->Vec<crate::entry::CacheEntry> {
    let digest=|e:&crate::entry::CacheEntry| {
        use sha2::Digest;let mut identity=e.clone();identity.created=0;identity.last_hit=0;
        sha2::Sha256::digest(serde_json::to_vec(&identity).unwrap()).to_vec()
    };
    entries.sort_by(|a,b| {
        let center=1.02_f64.powi(a.key.spr_bucket);
        let da=(a.source.spr.value()-center).abs()/center;
        let db=(b.source.spr.value()-center).abs()/center;
        da.total_cmp(&db).then(a.exploitability_over_P.total_cmp(&b.exploitability_over_P))
            .then(digest(a).cmp(&digest(b)))
    });
    if entries.len()<=1 {return entries;}
    let closest=entries.remove(0);
    entries.sort_by(|a,b|a.exploitability_over_P.total_cmp(&b.exploitability_over_P)
        .then(digest(a).cmp(&digest(b))));
    if entries[0].exploitability_over_P<closest.exploitability_over_P {vec![closest,entries.remove(0)]}
    else {vec![closest]}
}
```

Exclude mutable created/last_hit timestamps from the tie digest as in the code. These timestamps must not change replacement ordering on every hit. Cells with two entries evict individual oldest entries first, rewriting a surviving cell; when both are removed, delete its file. Maintain an entry-digest-to-cell map beside the entry index. Actual disk accounting counts each cell once and is recomputed after each eviction/rewrite; the quota loop takes the next oldest entry until measured bytes are under quota. Assign persisted `last_hit` on the writer using `max(unix_ms, previous_max_last_hit + 1)`, including on restart, so clock reversal cannot make a fresh hit the oldest entry.

- [ ] **Step 4 (5 min): Implement temp-and-rename and the writer loop.**

```rust
pub fn write_atomic(path:&std::path::Path,bytes:&[u8])->Result<(),crate::CacheError> {
    use std::io::Write;use std::sync::atomic::{AtomicU64,Ordering};
    static SEQ:AtomicU64=AtomicU64::new(0);
    let parent=path.parent().ok_or(crate::CacheError::Invalid("parent"))?;
    std::fs::create_dir_all(parent)?;
    let tmp=path.with_extension(format!("{}.{}.tmp",std::process::id(),SEQ.fetch_add(1,Ordering::Relaxed)));
    let result=(||->Result<(),crate::CacheError>{
        let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;f.sync_all()?;drop(f);std::fs::rename(&tmp,path)?;Ok(())
    })();
    if result.is_err() {let _=std::fs::remove_file(tmp);}result
}
```

```rust
// crates/cache/src/lib.rs
pub const CACHE_QUOTA_BYTES:u64=10*1024*1024*1024;

/// §10.4 storage root. `engine::Paths.cache` normally supplies it (cross-plan M13/Or5);
/// this fallback exists for tools and tests that have no `Paths`.
pub fn default_cache_root()->std::path::PathBuf {
    let base=std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("PokerAI").join("cache").join("v3")
}

pub enum WriteCommand {
    Store(Box<crate::entry::CacheEntry>,Option<std::sync::mpsc::SyncSender<bool>>),
    Touch{key:[u8;32],payload_digest:Vec<u8>,last_hit:u64},
    Delete([u8;32]),
    Shutdown,
}
pub struct StoreReceipt(Option<std::sync::mpsc::Receiver<bool>>);
impl StoreReceipt {
    /// True only when the writer reported a durable, validated cell on disk.
    pub fn wait(self,budget:std::time::Duration)->bool {
        self.0.map_or(false,|rx|rx.recv_timeout(budget).unwrap_or(false))
    }
}
pub struct Cache {
    root:std::path::PathBuf,
    writer:Option<std::sync::mpsc::SyncSender<WriteCommand>>,
    reader:Option<std::sync::mpsc::SyncSender<crate::lookup::ReadCommand>>,   // Task 7
    skipped:std::sync::atomic::AtomicBool,
}
impl Cache {
    pub fn disabled()->Cache {
        Cache{root:std::path::PathBuf::new(),writer:None,reader:None,
            skipped:std::sync::atomic::AtomicBool::new(false)}
    }
    pub fn root(&self)->&std::path::Path {&self.root}
    /// A failure to create the directory yields a disabled cache: every lookup is `Miss`
    /// and every store is dropped, and the recommendation path is unaffected (§12).
    pub fn open(root:std::path::PathBuf,quota_bytes:u64)->Cache {
        if std::fs::create_dir_all(&root).is_err() {return Cache::disabled();}
        let (tx,rx)=std::sync::mpsc::sync_channel::<WriteCommand>(8);
        let dir=root.clone();
        if std::thread::Builder::new().name("cache-writer".into()).spawn(move|| {
            let mut index=crate::quota::scan_index(&dir);
            crate::quota::sweep_temporaries(&dir);
            let mut max_last_hit=index.iter().map(|r|r.last_hit).max().unwrap_or(0);
            while let Ok(command)=rx.recv() {
                match command {
                    WriteCommand::Shutdown=>break,
                    WriteCommand::Delete(key)=>{
                        let _=std::fs::remove_file(crate::storage::entry_path(&dir,key));
                        index.retain(|r|r.key!=key);
                    }
                    WriteCommand::Touch{key,payload_digest,last_hit}=>{
                        max_last_hit=max_last_hit.max(last_hit).max(max_last_hit+1);
                        let _=crate::quota::apply_touch(&dir,key,&payload_digest,max_last_hit,&mut index);
                    }
                    WriteCommand::Store(entry,receipt)=>{
                        max_last_hit=max_last_hit.max(entry.last_hit).max(max_last_hit+1);
                        let mut stored=*entry;stored.last_hit=max_last_hit;
                        let ok=crate::quota::store_entry(&dir,stored,&mut index).is_ok();
                        if let Some(r)=receipt {let _=r.try_send(ok);}
                    }
                }
                crate::quota::enforce_quota(&dir,quota_bytes,&mut index);
            }
        }).is_err() {return Cache::disabled();}
        Cache{root,writer:Some(tx),reader:None,skipped:std::sync::atomic::AtomicBool::new(false)}
    }
    fn send(&self,command:WriteCommand) {
        let Some(tx)=self.writer.as_ref() else {return};
        if tx.try_send(command).is_err()
            && !self.skipped.swap(true,std::sync::atomic::Ordering::Relaxed) {
            eprintln!("cache write skipped: writer queue full or gone (not reported again this session)");
        }
    }
    pub fn store(&self,entry:&crate::entry::CacheEntry) {
        self.send(WriteCommand::Store(Box::new(entry.clone()),None));
    }
    /// Used by the pre-solver only (Task 16); never on a live delivery path.
    pub fn store_tracked(&self,entry:&crate::entry::CacheEntry)->StoreReceipt {
        let Some(tx)=self.writer.as_ref() else {return StoreReceipt(None)};
        let (rtx,rrx)=std::sync::mpsc::sync_channel(1);
        if tx.try_send(WriteCommand::Store(Box::new(entry.clone()),Some(rtx))).is_err() {
            return StoreReceipt(None);
        }
        StoreReceipt(Some(rrx))
    }
    pub fn touch(&self,key:[u8;32],payload_digest:Vec<u8>,last_hit:u64) {
        self.send(WriteCommand::Touch{key,payload_digest,last_hit});
    }
    pub fn shutdown(&self) {
        if let Some(tx)=self.writer.as_ref() {let _=tx.send(WriteCommand::Shutdown);}
        if let Some(tx)=self.reader.as_ref() {let _=tx.send(crate::lookup::ReadCommand::Shutdown);}
    }
}
```

`quota.rs` supplies the four writer helpers used above: `scan_index(&Path)->Vec<IndexRow>` (bounded `read_cell` over every `.bin` under the two-hex shards, recording measured file bytes and the maximum `last_hit` per cell), `sweep_temporaries(&Path)` (removes `*.tmp` siblings left by a crash), `apply_touch(&Path,[u8;32],&[u8],u64,&mut Vec<IndexRow>)->Result<(),CacheError>` (re-reads the cell, sets `last_hit` on the entry whose payload digest matches, re-encodes and `write_atomic`s it), `store_entry(&Path,CacheEntry,&mut Vec<IndexRow>)->Result<(),CacheError>` (`validate_entry`, read existing cell, `retain_two`, `encode`, `write_atomic`, update the row) and `enforce_quota(&Path,u64,&mut Vec<IndexRow>)` (loops `victims`, evicting the oldest entry of a two-entry cell by rewriting the survivor and deleting single-entry cells, recomputing measured bytes after each rewrite). `Cache::store` uses `try_send`; a full or disconnected queue is a skipped write logged once per session. The index is rebuilt from disk on open and is never a second authoritative file. No mutex spans file I/O. `Cache::reader` stays `None` until Task 7 installs the bounded reader thread.

- [ ] **Step 5 (4 min): Verify crash and disk-failure behavior.** Inject failure before rename: old cell remains readable; after rename: new cell is complete. Inject write denial/full disk: receipt reports failure and recommendation is unaffected. Test quota after touch and after restart, equal-last-hit key ordering, two-entry maximum, and a dominated insertion. `cache_atomic_write_and_quota` also rejects oversized encode before any rename.

```rust
#[test]
fn quota_ties_are_deterministic() {
    use cache::quota::{IndexRow,victims};
    let rows=vec![IndexRow{key:[2;32],bytes:10,last_hit:1},IndexRow{key:[1;32],bytes:10,last_hit:1}];
    assert_eq!(victims(&rows,10),vec![[1;32]]);
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): add atomic writer replacement and LRU quota' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 7: Serve validated ordinal nodes through bounded cache lookup

**Files:** Modify `crates/cache/src/{lib,lookup,label,entry}.rs`, `crates/cache/tests/lookup.rs`; create `crates/engine/src/cache_bridge.rs`; modify `crates/engine/Cargo.toml`, `crates/engine/src/{lib,core,engine}.rs`.

**Interfaces:** Consumes `CacheEntry`, `compare`, `label`, `read_cell`, `validate_solution`, `Derived`, `engine::Paths.cache` (Plan 2 Task 29), `engine::tree::tree_signature`. Produces `CacheQuery`, `CacheHit`, `Lookup`, `ReadCommand`, `Cache::lookup(&CacheQuery)->Lookup`, `legal_menu(&[Action],&[LegalAction])->bool`, `map_rows`. Engine extensions `cache_bridge::make_cache_query(input:&SolveInput,derived:&Derived,bb_chips:u32,rake:&Rake,reasons:&[ApproxReason],signature:&str,perm:&SuitPerm,target_bp:u16,budget:Duration)->Result<CacheQuery,UnsupportedReason>` (query actor from the OOP/IP seat mapping and `Derived.to_act`) and the single `EngineCore` field `pub cache: cache::Cache` plus `EngineCore::with_cache`. This task adds **no** presolver field: the `Presolver` handle lives on `Engine` and is added by Task 16, so a status/pause/resume command never locks the solve owner.

- [ ] **Step 1 (4 min): Add illegal-menu rejection test.**

```rust
#[test]
fn query_menu_must_be_legal_without_probability_moves() {
    use proto::{Action,LegalAction};use cache::lookup::legal_menu;
    let legal=vec![LegalAction::Fold,LegalAction::Call{cost:50},LegalAction::Raise{min_to:100,max_to:500},LegalAction::AllIn{to:500}];
    assert!(legal_menu(&[Action::Fold,Action::Call,Action::Raise{to:100}],&legal));
    assert!(!legal_menu(&[Action::Raise{to:99}],&legal));
    assert!(!legal_menu(&[Action::AllIn{to:501}],&legal));
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache query_menu_must_be_legal_without_probability_moves`.
- [ ] **Step 3 (5 min): Define query/result types and strict legality.**

```rust
pub struct CacheQuery {
    pub key:crate::key::KeyFields,pub source:crate::entry::SourceInputs,
    pub tree:proto::EffectiveTree,pub requested:proto::OrdinalPath,pub actor:String,
    pub legal:Vec<proto::LegalAction>,pub target_bp:u16,pub reasons:Vec<proto::ApproxReason>,
    pub inverse_perm:core_iso::SuitPerm,pub budget:std::time::Duration,
}
pub struct CacheHit {
    pub solution:proto::worker::StreetSolution,pub tree:proto::EffectiveTree,
    pub covered_paths:Vec<proto::OrdinalPath>,pub coverage:proto::Coverage,
    pub raw_exploitability_over_p:f64,pub notes:Vec<String>,pub source_mode:String,pub tree_signature:String,
}
pub enum Lookup {
    Exact{hit:CacheHit},Approximate{hit:CacheHit,reasons:Vec<proto::ApproxReason>},
    Provisional{hit:CacheHit,reasons:Vec<proto::ApproxReason>},Miss,
}
pub fn legal_menu(actions:&[proto::Action],legal:&[proto::LegalAction])->bool {
    use proto::{Action as A,LegalAction as L};
    actions.iter().all(|a|legal.iter().any(|l|match(a,l) {
        (A::Fold,L::Fold)|(A::Check,L::Check)|(A::Call,L::Call{..})=>true,
        (A::Bet{to},L::Bet{min_to,max_to})|(A::Raise{to},L::Raise{min_to,max_to})=>(*min_to..=*max_to).contains(to),
        (A::AllIn{to:a},L::AllIn{to:b})=>a==b,_=>false,
    }))
}
```

- [ ] **Step 4 (5 min): Implement candidate selection and node reconstruction.** For each of three bucket keys read at most one cell/two entries, validate source before comparing, and require non-SPR key fields equal by comparing `at_bucket(0)`. Exclude candidates missing the requested covered ordinal path or with the wrong actor; this is a node-only miss, not deletion. Rank survivors with `Comparison::rank`, then raw exploitability, then payload digest. Accuracy above target remains a survivor and becomes Provisional after ranking; do not reorder to prefer an at-target but more distant candidate.

```rust
pub fn map_rows(rows:&[Vec<f32>],inverse:&core_iso::SuitPerm)->Vec<Vec<f32>> {
    let mut out=vec![Vec::new();1326];
    for hi in 1_u8..52 {for lo in 0_u8..hi {
        let a=core_iso::apply(inverse,proto::Card(lo)).0;
        let b=core_iso::apply(inverse,proto::Card(hi)).0;
        let (x,y)=(a.min(b) as usize,a.max(b) as usize);
        let source=hi as usize*(hi as usize-1)/2+lo as usize;
        out[y*(y-1)/2+x]=rows[source].clone();
    }}
    out
}
pub fn map_flags(flags:&[bool],inverse:&core_iso::SuitPerm)->Vec<bool> {
    let mut out=vec![false;1326];
    for hi in 1_u8..52 {for lo in 0_u8..hi {
        let a=core_iso::apply(inverse,proto::Card(lo)).0;
        let b=core_iso::apply(inverse,proto::Card(hi)).0;
        let (x,y)=(a.min(b) as usize,a.max(b) as usize);
        out[y*(y-1)/2+x]=flags[hi as usize*(hi as usize-1)/2+lo as usize];
    }}
    out
}

fn payload_digest(e:&crate::entry::CacheEntry)->Vec<u8> {
    use sha2::Digest;let mut identity=e.clone();identity.created=0;identity.last_hit=0;
    sha2::Sha256::digest(serde_json::to_vec(&identity).expect("validated entry")).to_vec()
}

/// Rebuild the requested node set in the query's chips and suits (§10.4 lookup step 4).
/// Returns `None` for any query-side mismatch: the caller turns that into `Lookup::Miss`
/// without deleting the source file.
pub fn reconstruct(e:&crate::entry::CacheEntry,q:&CacheQuery)->Option<CacheHit> {
    let p=q.source.pot as f32;
    let mut nodes=Vec::new();
    for cached in &e.nodes {
        let m=q.tree.materialized.iter().find(|m|m.path==cached.path)?;
        if m.actor!=cached.actor {return None;}
        nodes.push(proto::worker::NodeStrategy{
            path:crate::entry::chip_path(&q.tree.materialized,&cached.path)?,
            actor:cached.actor.clone(),actions:m.actions.clone(),
            probs:map_rows(&cached.probs,&q.inverse_perm),
            ev_chips:map_rows(&cached.ev_over_P,&q.inverse_perm).into_iter()
                .map(|row|row.into_iter().map(|v|v*p).collect()).collect(),
            available:map_flags(&cached.available,&q.inverse_perm)});
    }
    let requested=e.nodes.iter().position(|n|n.path==q.requested)?;
    if nodes[requested].actor!=q.actor {return None;}
    let covered_paths=e.covered_paths.clone();
    let solution=proto::worker::StreetSolution{
        covered_paths:nodes.iter().map(|n|n.path.clone()).collect(),nodes,
        requested:u32::try_from(requested).ok()?,
        exploitability_chips:(e.exploitability_over_P*q.source.pot as f64) as f32,
        iterations:e.iterations,memory_bytes:e.memory_bytes,mode:e.mode.clone(),
        locks_applied:e.locks_applied,export:e.export.clone()};
    proto::worker::validate_solution(&solution,&q.tree.materialized).ok()?;
    if !legal_menu(&solution.nodes[requested].actions,&q.legal) {return None;}
    let realized=solution.nodes[requested].actions.iter()
        .filter_map(|a|action_to(a).map(|to|format!("{to}"))).collect::<Vec<_>>().join(", ");
    Some(CacheHit{tree:q.tree.clone(),covered_paths,coverage:proto::Coverage::Exact,
        raw_exploitability_over_p:e.exploitability_over_P,
        notes:vec![format!("cache realized menu at the requested node: [{realized}]"),
                   format!("cache source storage mode: {}", e.mode)],
        source_mode:e.mode.clone(),tree_signature:q.key.tree_signature.clone(),solution})
}

/// §10.4: read buckets `b - 1`, `b`, `b + 1`, filter, rank, then label.
pub fn select(cells:Vec<crate::storage::Cell>,q:&CacheQuery)
    ->Option<(crate::entry::CacheEntry,crate::lookup::Comparison)> {
    let mut candidates=Vec::new();
    for cell in cells {
        for e in cell.entries {
            if crate::entry::validate_entry(&e).is_err() {continue;}
            if e.key.at_bucket(0)!=q.key.at_bucket(0) {continue;}
            if !e.covered_paths.contains(&q.requested) {continue;}
            let eff=q.source.stack_oop.min(q.source.stack_ip);
            let Some(c)=crate::lookup::compare(&e,&q.tree,q.source.pot,eff,q.source.cap_mchips)
                else {continue};
            candidates.push((e,c));
        }
    }
    candidates.sort_by(|(ea,ca),(eb,cb)|
        ca.rank(cb)
            .then(ea.exploitability_over_P.total_cmp(&eb.exploitability_over_P))
            .then(payload_digest(ea).cmp(&payload_digest(eb))));
    candidates.into_iter().next()
}
```

Use `map_rows` for signed EV rows and probability rows and `map_flags` for availability, through identical source/destination indices. Never run signed EVs through range validation. Every exported node takes the **query's** materialized actions and chip path at the identical ordinal position; EV is `ev_over_P * P_query`, never re-rounded from the entry. Failure is Miss; only a source file that fails `decode`/`validate_entry` is deleted (by `read_cell`), an ordinary query mismatch is not. Include the realized menu and the source mode in `notes`.

- [ ] **Step 4a (5 min): Implement the bounded reader thread and `Cache::lookup`.** One fixed `cache-reader` thread owns all cell reads; `lookup` posts without blocking and waits at most `min(query.budget, 500 ms)`. A full queue, a closed channel or a timeout is an immediate `Miss`. Every request carries a monotonic token, so a reply that arrives after its timeout is dropped by the next waiter instead of contaminating it. No reader is ever respawned per timeout, and no lookup joins the writer.

```rust
pub enum ReadCommand {
    Cells{token:u64,keys:[[u8;32];3],reply:std::sync::mpsc::SyncSender<(u64,Vec<crate::storage::Cell>)>},
    Shutdown,
}

impl crate::Cache {
    /// Installed by `Cache::open` after the writer starts (Task 6 leaves `reader: None`).
    pub fn start_reader(&mut self) {
        let (tx,rx)=std::sync::mpsc::sync_channel::<ReadCommand>(4);
        let dir=self.root().to_path_buf();
        if std::thread::Builder::new().name("cache-reader".into()).spawn(move|| {
            while let Ok(command)=rx.recv() {
                match command {
                    ReadCommand::Shutdown=>break,
                    ReadCommand::Cells{token,keys,reply}=>{
                        let mut seen=std::collections::BTreeSet::new();
                        let cells=keys.iter().filter(|k|seen.insert(**k))
                            .filter_map(|k|crate::storage::read_cell(&crate::storage::entry_path(&dir,*k)))
                            .collect::<Vec<_>>();
                        let _=reply.try_send((token,cells));
                    }
                }
            }
        }).is_ok() {self.reader=Some(tx);}
    }

    pub fn lookup(&self,q:&crate::lookup::CacheQuery)->crate::lookup::Lookup {
        use crate::lookup::Lookup;
        let Some(reader)=self.reader.as_ref() else {return Lookup::Miss};
        let token=self.next_token.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
        let keys=[q.key.at_bucket(q.key.spr_bucket-1).digest(),q.key.digest(),
                  q.key.at_bucket(q.key.spr_bucket+1).digest()];
        let (tx,rx)=std::sync::mpsc::sync_channel(1);
        if reader.try_send(ReadCommand::Cells{token,keys,reply:tx}).is_err() {return Lookup::Miss;}
        let budget=q.budget.min(std::time::Duration::from_millis(500));
        let Ok((replied,cells))=rx.recv_timeout(budget) else {return Lookup::Miss};
        if replied!=token {return Lookup::Miss;}
        let Some((entry,comparison))=crate::lookup::select(cells,q) else {return Lookup::Miss};
        let Some(mut hit)=crate::lookup::reconstruct(&entry,q) else {return Lookup::Miss};
        let (coverage,provisional)=crate::label::label(&entry,q.source.spr,&comparison,
            q.target_bp,&q.reasons);
        hit.coverage=coverage.clone();
        self.touch(entry.key.digest(),crate::lookup::payload_digest(&entry),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
                .map_or(0,|d|d.as_millis() as u64));
        let reasons=match &coverage {proto::Coverage::Approximate{reasons}=>reasons.clone(),_=>vec![]};
        match (provisional,reasons.is_empty()) {
            (true,_)=>Lookup::Provisional{hit,reasons},
            (false,true)=>Lookup::Exact{hit},
            (false,false)=>Lookup::Approximate{hit,reasons},
        }
    }
}
```

Add `next_token: std::sync::atomic::AtomicU64` to `Cache` (initialised to 1 in `open`/`disabled`), call `start_reader()` at the end of `Cache::open`, and make `payload_digest` `pub(crate)`. The engine calls `lookup` only from the request's own `fast-path` work, never from `watchdog` or `engine-main`, and passes the remaining absolute budget.

- [ ] **Step 5 (5 min): Implement `make_cache_query` in `crates/engine/src/cache_bridge.rs`.**

```rust
use cache::entry::SourceInputs;
use cache::key::{KeyFields, Model, RakeKey, Rational, spr_bucket};
use cache::lookup::CacheQuery;
use core_iso::SuitPerm;
use proto::*;

fn unsupported(message:&str)->UnsupportedReason {
    UnsupportedReason::EngineError{message:message.into(),retryable:false}
}

/// The shared §10.4 reference state: key fields and exact source inputs for one solve input at
/// its street root. `signature` is Plan 2's `engine::tree::tree_signature(&input.tree, pot)`;
/// this function never hashes raw materialized chips. `perm` is the permutation returned by the
/// joint canonicalization of the board and both public ranges. Task 10's `entry_from_solution`
/// calls the same helper, so a stored entry and a query can never disagree on the key.
pub fn key_and_source(input:&SolveInput,bb_chips:u32,rake:&Rake,signature:&str,perm:&SuitPerm)
    ->Result<(KeyFields,SourceInputs),UnsupportedReason> {
    let root=&input.root;
    if !matches!(root.street,Street::Flop|Street::Turn) {return Err(unsupported("cache covers flop and turn only"));}
    let pot=root.pot_root.checked_add(root.dead_this_street).ok_or_else(||unsupported("pot overflow"))?;
    let eff=root.stack_oop_root.min(root.stack_ip_root);
    if pot==0||eff==0||bb_chips==0 {return Err(unsupported("degenerate cache reference state"));}
    let spr=Rational::new(eff as u64,pot as u64).map_err(|_|unsupported("spr"))?;
    let (rate_bits,cap_mchips,collection_rule_version)=match rake {
        Rake::TimeCharge=>(0.0_f32.to_bits(),0,1),
        Rake::PotRake{rate,cap_mchips,..}=>(rate.to_bits(),*cap_mchips,1),
    };
    let mut ranges=[input.ranges[0].clone(),input.ranges[1].clone()];
    for r in &mut ranges {core_ranges::block_public(r,&root.board);}
    let canonical=root.board.iter().map(|c|core_iso::apply(perm,*c)).collect::<Vec<_>>();
    let mut canonical_board=canonical.clone();
    canonical_board[..3].sort_by_key(|c|c.0);
    let mapped=[core_iso::apply_range(perm,&ranges[0]),core_iso::apply_range(perm,&ranges[1])];
    let key=KeyFields{schema_version:3,solver_commit:proto::worker::SOLVER_COMMIT.into(),
        adapter_version:proto::worker::ADAPTER_VERSION,rules_version:3,canonical_board,
        root_street:root.street,spr_bucket:spr_bucket(spr),tree_signature:signature.into(),
        rake:RakeKey{rate_bits,
            cap_over_p:Rational::new(cap_mchips as u64,1000*pot as u64).map_err(|_|unsupported("cap"))?,
            collection_rule_version},
        range_hash_oop:core_ranges::hash_scaled(&mapped[0]),
        range_hash_ip:core_ranges::hash_scaled(&mapped[1]),model:Model::Baseline};
    let source=SourceInputs{pot,stack_oop:root.stack_oop_root,stack_ip:root.stack_ip_root,spr,
        bb_chips,quantum_over_p:Rational::new(1,pot as u64).map_err(|_|unsupported("quantum"))?,
        cap_mchips,ranges:mapped};
    Ok((key,source))
}

/// Build the §10.4 query. The query actor comes from the OOP/IP seat mapping and `Derived.to_act`;
/// hero, `bb_chips`, `target_bp` and the requested path never enter `KeyFields`.
#[allow(clippy::too_many_arguments)]
pub fn make_cache_query(input:&SolveInput,derived:&Derived,bb_chips:u32,rake:&Rake,
    reasons:&[ApproxReason],signature:&str,perm:&SuitPerm,target_bp:u16,
    budget:std::time::Duration)->Result<CacheQuery,UnsupportedReason> {
    let (key,source)=key_and_source(input,bb_chips,rake,signature,perm)?;
    let root=&input.root;
    let requested=proto::resolve_chip_path(&input.tree.materialized,
        &root.history.iter().map(|(_,a)|a.clone()).collect::<Vec<_>>())
        .ok_or_else(||unsupported("requested path is not in the query tree"))?;
    let actor=if derived.to_act==Some(root.oop) {"oop"} else if derived.to_act==Some(root.ip) {"ip"}
        else {return Err(unsupported("actor is neither street-root seat"))};
    Ok(CacheQuery{key,source,tree:input.tree.clone(),requested,actor:actor.into(),
        legal:derived.legal.clone(),target_bp,reasons:reasons.to_vec(),
        inverse_perm:core_iso::inverse(perm),budget})
}
```

`core_iso::inverse(&SuitPerm) -> SuitPerm` is Plan 1 Task 21's inverse of a suit permutation (`P1.T21`); that is the exact producer name. Hero, `bb_chips`, `target_bp` and the requested path stay out of `KeyFields`; `TimeCharge` is the unraked `0/0` rake key. Add a `cargo test -p engine` unit case in `cache_bridge.rs` asserting that two inputs differing only in `bb_chips`, hero seat or `target_bp` produce the same `key.digest()`.

- [ ] **Step 5a (5 min): Own the cache handle in `EngineCore` and resolve `Paths.cache`.** (Blocker B3.)

```rust
// crates/engine/src/core.rs — one added field and the cache constructor argument
pub struct EngineCore {
    // ... existing fields (worker, clock, identity, watchdog, next_request_id,
    //     memory_limit_bytes, stage, log, snapshots, config, range_source)
    pub cache: cache::Cache,
    // `flop_policy` is added by Task 9; no presolver field is ever added to EngineCore.
}

impl EngineCore {
    /// Plan 2's four-argument `new` keeps its signature; the cache is installed separately so
    /// every existing test rig (`solve_client.rs`, `identity_race.rs`, `final_delivery.rs`)
    /// keeps compiling with a disabled cache.
    pub fn with_cache(mut self, cache: cache::Cache) -> Self { self.cache = cache; self }
}
```

`EngineCore::new` initialises `cache: cache::Cache::disabled()`, so no Plan 2 test changes. This task does **not** mention `Presolver`: Task 16 adds `Engine.presolver` as an independent handle so a UI command never waits behind a solve. `crates/engine/src/engine.rs` resolves the root once, inside Plan 2 Task 29's existing `Engine::new`, without altering its `Result<Engine, EngineError>` signature or its configuration handling:

```rust
// crates/engine/src/engine.rs — inside Plan 2 Task 29's Engine::new, after EngineCore::new
// Plan 2 Task 29 already builds the core (including DecisionLog::open(&paths.log_dir)) and
// applies the validated configuration. This plan adds only the cache handle and its banner.
core.cache = cache::Cache::open(paths.cache.clone(), cache::CACHE_QUOTA_BYTES);
if let Some(message) = core.cache.availability_warning() {
    startup.banners.push(message);   // StartupReport.banners, surfaced by startup_report()
}
```

`Paths.cache` is declared by Plan 2 Task 29 as `Paths { log_dir, worker_exe, preflop, cache }` (cross-plan M13/Or5, user decision) and Plan 5 fills it with `local.join("cache/v3")`; when a caller constructs `Paths` by hand, `cache::default_cache_root()` produces the same `%LOCALAPPDATA%\PokerAI\cache\v3`. Cache availability and error diagnostics are appended to the existing `StartupReport.banners`/`cache_state` during startup; `Engine::new` keeps returning `Result<Engine, EngineError>` and never gains a second return value. Add `cache = { path = "../cache" }` to `crates/engine/Cargo.toml` and `pub mod cache_bridge;` to `crates/engine/src/lib.rs`. A `Cache::disabled()` handle makes every lookup `Miss` and every store a no-op, so a missing or unwritable directory never blocks delivery (§12).

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache`; `cargo test -p engine`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache crates/engine Cargo.lock
git commit -m 'feat(cache): serve bounded validated query-tree node hits' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 8: Freeze the complete T4 structural-identity scale contract

**Files:** Create `crates/engine/tests/cache_key_structural_identity.rs`, `crates/engine/tests/golden/cache_scale.json`; modify `crates/engine/src/tree/templates.rs`, `crates/engine/tests/support/mod.rs`, `crates/cache/tests/{entry,lookup,storage}.rs`.

**Interfaces:** Consumes production `build_effective_tree`, `Cache`, `CacheEntry`, `CacheQuery`, and Plan 2 Task 2's `Templates::with_extra(&[TemplateSpec])` / `Templates::base_ids()` seam behind the `test-templates` feature. Produces only `engine::tree::templates::install_cache_test_templates()` (`#[cfg(any(test, feature = "test-templates"))]`); test support `CacheRig::new(template:&str,p:u32,eff:u32,cap_mchips:u32)->CacheRig`, `CacheRig::query(p:u32,eff:u32,cap_mchips:u32,path:&[Action],actor:&str)->CacheQuery`, `CacheRig::hit(&CacheQuery)->CacheHit`, `CacheRig::miss(&CacheQuery)->bool`. Rig owns a temporary cache directory and `pub entry:CacheEntry`; it uses the real materializer and stores actor-distinct synthetic valid matrices, so no worker solve or timing noise enters T4.

**Upstream seam (cross-plan M20/D10, resolved).** Plan 2 Task 2 already owns the extension mechanism: `Templates::with_extra(&[TemplateSpec])` registers extras, `Templates::base_ids()` returns the nine production ids, and `Templates::ids()` includes registered extras. Nothing is added to `Templates::get` here and no second registry exists. Every engine test command that builds `CacheRig` enables both `testing` and `test-templates` (`--features testing,test-templates`), and the engine's test-only self dependency enables the same pair. Production template selection (Task 9) is limited to its named production ids and never sees a test template.

- [ ] **Step 1 (3 min): Copy this §10.4 scale check verbatim into the T4 test's contract comment.**
- **Scale check** (contract, section 13.1 T4): pot/stack/cap 100/500/5000 mchips and 200/1000/10000 mchips with identical ranges, proportional quantum and identical fractional tree hit the same entry with identical frequencies and doubled chip EV, `Exact` (identical SPR rationals, identical realized fractions: every pot and stack doubles exactly, so `max(dev) = 0` at every node), at the root, at IP's node after Bet(50) versus Bet(100), and at OOP's node after Check, Bet(50) versus Check, Bet(100) (both actors at non-root nodes; a call closes the street, so the next decision lives in the next street's own root and is never requested from this entry, section 5); 100/500 versus 103/515 with a one-chip quantum has SPR `5 : 1` in both and rounds the root half-pot bet to 50 versus 52 chips (52/103 = 0.5049, a **root-action** deviation of 0.49%); the reported `MenuRounded{max_delta_pct}` is the maximum over the complete materialized list and is larger: after bet/call the turn pots are 200 and 207, the half-pot bets 100 and 104 (`104/103 - 100/100 = 0.0097`, 0.97%), and deeper nodes deviate further; T4 freezes the value computed from the two materialized lists (asserting `0.97 <= max_delta_pct < 5`), never the root number, and the candidate is `Approximate{MenuRounded}`, never `Exact`; 100/500 versus 100/508 (SPR 5.08, `delta = 1.6%`) is `SprBucketed` (and `MenuRounded` only if some realized size actually differs); 100/500 versus 100/511 (`delta = 2.2%`) misses; the `MenuRounded{2.0}` case is a specified pair of trees: test template `menu_round_test_v1` (single 0.33 bet, all-in-only raises, cap 1, add-all-in 1.5, force-all-in 0.15, flop root) at entry 100/500 versus query 20/100 (SPR `5 : 1` in both): the 0.33 bet is 33 chips (0.33) versus 7 chips (0.35) at every node with an unchanged pot (`dev = 0.02`), 55 versus 11 (0.55 in both) after bet/call, 91 versus 18 (0.91 versus 0.90) after bet/call/bet/call, the all-in raise-to amounts deviate by at most 0.02 (467 versus 93 after bet/call), and the river all-in is added in both trees after bet/call/bet/call (`412 <= 414`, `82 <= 84`) and in neither elsewhere, so the topologies agree and `max(dev) = 0.02`; a single-node comparison is never used, so a query whose root bet is exactly representable still carries `MenuRounded` when a deeper node rounds differently; a different canonical board always misses.

- [ ] **Step 1a (5 min): Add `install_cache_test_templates` to `crates/engine/src/tree/templates.rs` over Plan 2's existing seam.** Without the three §13.1 T4 templates `CacheRig::new` cannot build their trees and the T4 test cannot run. The helper is compiled only under `cfg(test)` or the `test-templates` feature, so the production selector and `Templates::base_ids()` are unchanged. It adds no registry, no `register_test_template`, no `test_extra` and no replacement `Templates::get`: Plan 2 Task 2 owns `with_extra`.

```rust
// appended to crates/engine/src/tree/templates.rs
/// The three §13.1 T4 templates. `add`/`force` are literal spec values, `merging` is 0 and
/// donk menus follow the §4.6 rule (`None` on the root street, explicit empty after it).
/// Registration goes through Plan 2 Task 2's `Templates::with_extra`; calling this more than
/// once is safe because `with_extra` is idempotent per id.
#[cfg(any(test, feature = "test-templates"))]
pub fn install_cache_test_templates() {
    use proto::Street::{Flop, River, Turn};
    use proto::MenuSize::{AllIn, Pot};
    let streets = [Flop, Turn, River];
    let build = |id: &'static str, bet: Vec<proto::MenuSize>, raise: Vec<proto::MenuSize>,
                 add: f32, force: f32| {
        let mut menus = std::collections::BTreeMap::new();
        for s in streets {
            menus.insert(s, proto::PlayerMenus {
                oop: proto::SideMenu { bet: bet.clone(), raise: raise.clone() },
                ip: proto::SideMenu { bet: bet.clone(), raise: raise.clone() },
                donk: if s == Flop { None } else { Some(vec![]) } });
        }
        TemplateSpec { id, root_street: Flop, menus, add_allin_threshold: add,
            force_allin_threshold: force, merging_threshold: 0.0, wager_cap: 1 }
    };
    // 1. `a`-only bets, no ordinary raises: the §10.4 terminal rake-cap pair 500 / 504.
    // 2. empty menus everywhere: isolates SPR 5.00 / 5.08 / 5.11 with identical realized menus.
    // 3. single 0.33 bet, `a`-only raises: the specified `MenuRounded{2.0}` pair 100/500 vs 20/100.
    Templates::with_extra(&[
        build("check_jam_test_v1", vec![AllIn], vec![], 0.0, 0.0),
        build("check_only_test_v1", vec![], vec![], 0.0, 0.0),
        build("menu_round_test_v1", vec![Pot(0.33)], vec![AllIn], 1.5, 0.15),
    ]);
}
```

`crates/engine/tests/support/mod.rs` calls `install_cache_test_templates()` from `CacheRig::new` behind a `std::sync::Once`. `crates/engine/Cargo.toml` carries `testing` from Plan 2 Task 2; the engine's test-only self dependency and every `CacheRig` test command enable `testing,test-templates` together. Add a unit assertion in `templates.rs` after `install_cache_test_templates()`:

```rust
assert_eq!(Templates::base_ids().len(), 9);
for id in ["check_jam_test_v1", "check_only_test_v1", "menu_round_test_v1"] {
    assert!(Templates::get(id).is_some());
    assert!(Templates::ids().contains(&id));
}
```

- [ ] **Step 2 (5 min): Write the red scale/actor test.** `CacheRig::new` constructs root board Kh7d2c with OOP Seat(2), IP Seat(0), equal effective stacks, dead 0, `bb_chips: 2` and empty history, parses board-only public AA/KK ranges, calls the production template/materializer, exports all current-street nodes, and assigns `available` only to board-compatible supported combos. For node index n, use uniform legal probabilities and `EV(a)=10*n+a`, except fold = 0; this makes wrong actor/path selection observable. `hit` unwraps any non-Miss lookup. The three test templates reach `build_effective_tree` only through Plan 2's `Templates::with_extra` under the `cfg`-gated helper of Step 1a; the production selector of Task 9 never names them.

```rust
#[test]
fn cache_key_structural_identity() {
    use proto::{Action,Coverage,ApproxReason};
    let rig=CacheRig::new("flop_fast_v1",100,500,5000);
    for (small,large,actor) in [
        (vec![],vec![],"oop"),
        (vec![Action::Bet{to:50}],vec![Action::Bet{to:100}],"ip"),
        (vec![Action::Check,Action::Bet{to:50}],vec![Action::Check,Action::Bet{to:100}],"oop"),
        (vec![Action::Check],vec![Action::Check],"ip"),
    ] {
        let a=rig.hit(&rig.query(100,500,5000,&small,actor));
        let b=rig.hit(&rig.query(200,1000,10000,&large,actor));
        assert!(matches!(b.coverage,Coverage::Exact));
        let na=&a.solution.nodes[a.solution.requested as usize];
        let nb=&b.solution.nodes[b.solution.requested as usize];
        assert_eq!(na.actor,actor);assert_eq!(na.probs,nb.probs);
        for (x,y) in na.ev_chips.iter().flatten().zip(nb.ev_chips.iter().flatten()) {
            assert!((2.0*x-y).abs()<1e-5);
        }
    }
    let rounded=rig.hit(&rig.query(103,515,5150,&[],"oop"));
    let value=match rounded.coverage {Coverage::Approximate{reasons}=>reasons.iter().find_map(|r|
        if let ApproxReason::MenuRounded{max_delta_pct}=r {Some(*max_delta_pct)}else{None}).unwrap(),
        _=>panic!("rounded menu cannot be Exact")};
    assert!(0.97<=value&&value<5.0);
    assert!(100.0*(52.0_f32/103.0-50.0/100.0)<value);
    let spr_rig=CacheRig::new("check_only_test_v1",100,500,5000);
    let bucketed=spr_rig.hit(&spr_rig.query(100,508,5000,&[],"oop"));
    assert!(matches!(bucketed.coverage,Coverage::Approximate{..}));
    assert!(spr_rig.miss(&spr_rig.query(100,511,5000,&[],"oop")));
    let menu=CacheRig::new("menu_round_test_v1",100,500,5000);
    let hit=menu.hit(&menu.query(20,100,1000,&[],"oop"));
    let text=serde_json::to_string(&hit.coverage).unwrap();assert!(text.contains("MenuRounded"));
    let a=menu.query(20,100,1000,&[],"oop");
    let d=cache::lookup::compare(&menu.entry,&a.tree,20,100,1000).unwrap();
    assert!((d.max_dev*100.0-2.0).abs()<1e-9);
}
```

- [ ] **Step 3 (2 min): Run red:** `cargo test -p engine --features testing,test-templates --test cache_key_structural_identity`; absent rig or failed assertion.
- [ ] **Step 4 (5 min): Complete the rig, raw-fraction golden and all T4 mutation cases.** The three templates are exactly the ones registered in Step 1a. With `check_jam_test_v1` at P100 compare eff500 and504 at cap55200 (the terminal rake-cap pair). `check_only_test_v1` isolates SPR5.00/5.08/5.11 with identical realized menus. `menu_round_test_v1` supplies the specified `MenuRounded{2.0}` pair. Under a wagering template, AllIn500 versus AllIn508 can have dev0.08 and correctly fail the separate0.05 menu filter even though SPR passes; test that rejection too. Never bypass menu validation on an SPR hit. Test query mutations separately; each starts from a valid copy and changes only the indicated field:

```rust
let mut q=rig.query(100,500,5000,&[],"oop");q.key.adapter_version+=1;assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.key.rules_version+=1;assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.key.schema_version+=1;assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.key.solver_commit.push('x');assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.key.model=cache::key::Model::Locked{fingerprint:[9;32]};assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.key.rake.collection_rule_version+=1;assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.key.range_hash_oop[0]^=1;assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.actor="ip".into();assert!(rig.miss(&q));
let mut q=rig.query(100,500,5000,&[],"oop");q.requested=vec![255];assert!(rig.miss(&q));
```

Also vary canonical board (genuinely different flop), root street, tree signature, rake rate and normalized cap, IP hash, and locked fingerprint against another locked fingerprint. Vary hero/seat/hand/bb/target/path separately: only target/path affect serving, never key digest. Permute all 24 suits and inverse-map named combo rows; same canonical key and exact row recovery. Proportional quantum is diagnostic metadata, not a key field; production always uses one-chip wagers.

Test real materialized boundary pairs 150/151 at add1.5, 80/81 with half-pot/force0.15, facing stacks400/401, plus a min-raise/deduplication boundary. Assert topology mismatch directly, even if another predicate would also reject. Root-exact/deeper-rounded test uses P100/stack500 versus P200/stack1000 with an inserted fraction chosen so the root is exact, then a one-chip deeper size alteration in a test materialized query: maximum deviation must be nonzero. Menu >5% rejects. At target raw0.005049 test 50 versus51; stored ChartRounded/DeadlineBestSoFar/UnconditionedPriorStreet all survive; i16 and f32 behave identically.

- [ ] **Step 5 (4 min): Freeze complete materialized lists and computed maximum.** Generate JSON from the two real lists, maximum deviation and the path/action attaining it, then inspect the lists against §4.6 before committing. Test compares the current computation to the committed number, as well as `0.97 <= value < 5`; never freeze only the root's0.49. This generation occurs during this task, not a future benchmark.

```rust
let frozen=serde_json::json!({"entry":rig.entry.tree.materialized,
    "query":rig.query(103,515,5150,&[],"oop").tree.materialized,
    "max_delta_pct":value});
if std::env::var_os("POKERAI_RECORD_GOLDENS").is_some() {
    std::fs::write("tests/golden/cache_scale.json",serde_json::to_vec_pretty(&frozen).unwrap()).unwrap();
}
let expected:serde_json::Value=serde_json::from_slice(&std::fs::read("tests/golden/cache_scale.json").unwrap()).unwrap();
assert_eq!(frozen,expected);
```

Run once with `POKERAI_RECORD_GOLDENS=1`, review, unset it, then normal green tests. No automatic golden acceptance in CI. The cache-corruption mutations of Task 5 now run with these real fixtures, completing `cache_payload_validated`.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --features testing,test-templates --test cache_key_structural_identity`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache/tests crates/engine/src/tree/templates.rs crates/engine/tests
git commit -m 'test(cache): freeze complete T4 scale actor and topology cases' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 9: Implement flop budget and evidence-based template selection

**Files:** Create `crates/engine/src/flop.rs`; modify `crates/engine/src/{lib,core,engine,deadline}.rs`; create `crates/engine/tests/flop_path_golden.rs`.

**Interfaces:** Consumes `SolverPrefs`, `TakenAction`, and Plan 2 Task 20's `deadline::{Deadlines, street_budget_ms, final_delivery_ms, extraction_margin_ms}` unchanged, plus Plan 2 Task 29's existing `Engine::set_config`/`apply_config`; Produces `deadline::flop_budget_valid(u8)->bool`, `flop::{FlopPolicy{min_admitted:bool}, FlopPolicy::live_template(u32)->&'static str, FlopPolicy::from_v3(Option<f64>,Option<f64>)->FlopPolicy, preflop_wagers(&HandState)->u32}`, `flop::{V3PolicyEvidence, load_v3_policy(path:&std::path::Path,expected:&V3PolicyEvidence)->FlopPolicy}`, the `EngineCore.flop_policy` field, and `Engine::config(&self)->GameConfig`. This task does **not** produce an `Engine::set_config` body.

**Cross-plan D5/M3:** this task adds **no** parallel deadline function. `flop_deadlines` is deleted from the plan; every flop budget number comes from Plan 2 Task 20's `Deadlines::for_request(t0_ms, Street::Flop, flop_budget_s)` and `Deadlines::watchdog_fire_ms()`, which Plan 2 already implements as `5_000 + budget * 1_000` for flop final delivery and a 100 ms watchdog lead.

**Upstream configuration seam (M4/M10/S15, resolved).** Plan 2 Task 29 already declares and implements `Engine::set_config(&mut self, GameConfig) -> Result<u32, EngineError>`, including blind/thread/flop validation, revision allocation, next-hand queuing and the shared-config lock. This task preserves that body and adds tests plus `flop_budget_valid`; it never replaces it.

- [ ] **Step 1 (4 min): Write `flop_budget_setting_golden` against Plan 2's `Deadlines`.**

```rust
#[test]
fn flop_budget_setting_golden() {
    use engine::deadline::{flop_budget_valid,Deadlines};
    use engine::flop::FlopPolicy;
    use proto::Street;
    let ten=Deadlines::for_request(1000,Street::Flop,10);
    assert_eq!((ten.street_deadline_ms,ten.final_delivery_ms,ten.extraction_margin_ms),(11_000,16_000,600));
    assert_eq!(ten.watchdog_fire_ms(),15_900);
    let thirty=Deadlines::for_request(1000,Street::Flop,30);
    assert_eq!((thirty.street_deadline_ms,thirty.final_delivery_ms),(31_000,36_000));
    assert_eq!(thirty.watchdog_fire_ms(),35_900);
    // a turn decision keeps 6 s / 15 s / 14.9 s even at the maximum flop preference
    let turn=Deadlines::for_request(0,Street::Turn,30);
    assert_eq!((turn.street_deadline_ms,turn.watchdog_fire_ms(),turn.extraction_margin_ms),(6_000,14_900,200));
    assert_eq!(Deadlines::for_request(0,Street::Flop,10).watchdog_fire_ms(),14_900);
    assert_eq!(Deadlines::for_request(0,Street::Flop,30).watchdog_fire_ms(),34_900);
    // the wire deadline at 250 ms of elapsed time (§7 margins 100 + 50)
    assert_eq!(Deadlines::for_request(0,Street::Flop,10).worker_deadline_ms(250,10_000),Some(9_600));
    assert_eq!(Deadlines::for_request(0,Street::Flop,30).worker_deadline_ms(250,30_000),Some(29_600));
    assert!(!flop_budget_valid(0));assert!(flop_budget_valid(1));
    assert!(flop_budget_valid(30));assert!(!flop_budget_valid(31));
    assert_eq!(FlopPolicy{min_admitted:false}.live_template(2),"flop_fast_v1");
    assert_eq!(FlopPolicy{min_admitted:true}.live_template(2),"flop_min_v1");
    assert_eq!(FlopPolicy{min_admitted:true}.live_template(3),"flop_fast_v1");
}

#[test]
fn set_config_rejects_out_of_range_flop_budget() {
    let mut engine=support::engine_with_fake_worker();
    let base=support::game_config();
    assert!(engine.set_config(proto::GameConfig{solver:proto::SolverPrefs{threads:16,target_bp:50,
        flop_budget_s:10},..base.clone()}).is_ok());
    for bad in [0_u8,31,255] {
        let cfg=proto::GameConfig{solver:proto::SolverPrefs{threads:16,target_bp:50,
            flop_budget_s:bad},..base.clone()};
        assert!(engine.set_config(cfg).is_err(),"flop_budget_s {bad} must be rejected");
    }
    let zero_threads=proto::GameConfig{solver:proto::SolverPrefs{threads:0,target_bp:50,
        flop_budget_s:10},..base};
    assert!(engine.set_config(zero_threads).is_err());
    // a rejected config leaves the previous revision and value in place
    assert_eq!(engine.config().solver.flop_budget_s,10);
}
```

`support::engine_with_fake_worker()` builds `Engine::with_core(EngineCore::new(...))` from Plan 2's `FakeWorker`/`FakeClock`, and `support::game_config()` returns the 1/2 no-rake `GameConfig` of Plan 2's `testing::cfg_1_2()`. Add `pub fn config(&self) -> GameConfig` to `Engine` alongside the existing `state()` accessor.

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --features testing flop_budget_setting_golden`; `cargo test -p engine --features testing set_config_rejects_out_of_range_flop_budget`.
- [ ] **Step 3 (5 min): Implement budget validation, the config getter and evidence-based V3 admission.**

```rust
// crates/engine/src/deadline.rs — the only addition this plan makes to Plan 2's module
/// §4.2 / §13.3: the per-session flop budget is 1..=30 seconds, default 10.
pub fn flop_budget_valid(flop_budget_s: u8) -> bool { (1..=30).contains(&flop_budget_s) }
```

```rust
// crates/engine/src/flop.rs
use proto::{Action, HandState, Street, TakenAction};

#[derive(Clone,Copy,Debug,PartialEq)]
pub struct FlopPolicy {pub min_admitted:bool}
impl FlopPolicy {
    /// §10.1: three or more preflop wagers always use `flop_fast_v1`; a single-raised pot uses
    /// `flop_min_v1` only when V3 admitted it at both depths.
    pub fn live_template(&self,preflop_wagers:u32)->&'static str {
        if preflop_wagers<3&&self.min_admitted {"flop_min_v1"}else{"flop_fast_v1"}
    }
    /// V3: `flop_min_v1` is admitted iff its measured p95 is at most 10 s at 100bb and 200bb.
    pub fn from_v3(p95_100:Option<f64>,p95_200:Option<f64>)->Self {
        Self{min_admitted:[p95_100,p95_200].iter().all(|v|
            v.is_some_and(|x|x.is_finite()&&x>0.0&&x<=10.0))}
    }
}

/// §10.1 classification from history, never from ranges. The live BB (or straddle) is the
/// first wager; each voluntary action that raises the facing amount adds one. Forced posts are
/// not counted twice and calls/checks/folds never increment.
pub fn preflop_wagers(state:&HandState)->u32 {
    let mut wagers=1;
    let mut facing=state.config.straddle.as_ref().map_or(state.config.bb_chips,|s|s.amount_chips);
    for TakenAction{street,action,..} in &state.actions {
        if *street!=Street::Preflop {break;}
        if let Action::Raise{to}|Action::Bet{to}|Action::AllIn{to}=action {
            if *to>facing {facing=*to;wagers+=1;}
        }
    }
    wagers
}

/// A limped pot (no voluntary raise) is not a pre-solver SRP scenario and is served with the
/// conservative fast template.
pub fn is_limped(state:&HandState)->bool {preflop_wagers(state)==1}
```

```rust
// crates/engine/src/engine.rs — the only engine.rs addition of this task
/// Settings UI reads the accepted next-hand configuration, not the frozen active one.
pub fn config(&self) -> GameConfig {
    self.queued_config.clone().unwrap_or_else(|| self.config.clone())
}
```

Preserve Plan 2 Task 29's `set_config` and `apply_config` bodies, including blind/thread/flop validation, revision allocation, next-hand queuing and the shared-config lock. Add `flop_budget_valid` only as a wrapper over the existing `FLOP_BUDGET_RANGE`. Rejected settings change neither revision nor active work. New serving helpers consume the one request configuration captured by `serve_request`; they do not lock `EngineCore` from a command. Active-hand money stays frozen in `HandConfig`; the solver preference is captured once at admission, so a later `set_config` cannot extend an already admitted request.

**V3 policy evidence (owned by this task).** Task 9 owns `V3PolicyEvidence`, `load_v3_policy` and `EngineCore.flop_policy`. Evidence contains the exact template signature, solver commit, adapter/rules versions, storage-mode policy, source-lock hash, machine identity and finite positive p95 seconds at 100bb and 200bb. `load_v3_policy(path, expected_provenance)` returns `FlopPolicy::from_v3(Some(p95_100), Some(p95_200))` only when every provenance field matches; otherwise it returns `FlopPolicy::from_v3(None, None)`, with a startup diagnostic. `EngineCore::new` initializes that conservative value. `Engine::new` loads evidence off the recommendation path before `engine-main` starts. Task 24 serializes the matching evidence alongside its raw V3 measurements; Task 9's tests use synthetic evidence. Missing production evidence does not create a dependency cycle on Task 24.

```rust
// crates/engine/src/flop.rs — the provenance-checked policy record
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize,PartialEq)]
pub struct V3PolicyEvidence {
    pub template_signature:String,pub solver_commit:String,pub adapter_version:u16,
    pub rules_version:u16,pub storage_mode_policy:String,pub source_lock_sha256:String,
    pub machine_id:String,pub p95_100bb_s:f64,pub p95_200bb_s:f64,
}
pub fn load_v3_policy(path:&std::path::Path,expected:&V3PolicyEvidence)->FlopPolicy {
    let Ok(bytes)=std::fs::read(path) else {return FlopPolicy::from_v3(None,None)};
    let Ok(e)=serde_json::from_slice::<V3PolicyEvidence>(&bytes)
        else {return FlopPolicy::from_v3(None,None)};
    let provenance_matches=e.template_signature==expected.template_signature
        &&e.solver_commit==expected.solver_commit&&e.adapter_version==expected.adapter_version
        &&e.rules_version==expected.rules_version
        &&e.storage_mode_policy==expected.storage_mode_policy
        &&e.source_lock_sha256==expected.source_lock_sha256&&e.machine_id==expected.machine_id;
    if provenance_matches {FlopPolicy::from_v3(Some(e.p95_100bb_s),Some(e.p95_200bb_s))}
    else {FlopPolicy::from_v3(None,None)}
}
```

Add `pub flop_policy: crate::flop::FlopPolicy` to `EngineCore`, initialized by `EngineCore::new` to `FlopPolicy::from_v3(None, None)`; derive `Clone`/`Debug` on `FlopPolicy` so it can be stored. Do not introduce a map of unqualified timings.

- [ ] **Step 4 (4 min): Extend the fake-clock watchdog and worker-send tests.** Using Plan 2's `FakeClock`, assert the flop wire deadline at 250 ms of elapsed time (9,600 default / 29,600 maximum), the watchdog at 14,900 / 34,900 ms, a turn at maximum flop preference still at 14,900 with first attempt 6,000 and extraction 200, and flop extraction 600. Assert that calling `set_config` with a larger budget while a request is in flight does not move that request's `Deadlines` (the admitted `SolvePlan` owns its own copy).

```rust
#[test]
fn config_change_does_not_extend_an_admitted_request() {
    use engine::deadline::Deadlines;use proto::Street;
    let admitted=Deadlines::for_request(0,Street::Flop,10);
    let later=Deadlines::for_request(0,Street::Flop,30);
    assert_eq!(admitted.watchdog_fire_ms(),14_900);
    assert_eq!(later.watchdog_fire_ms(),34_900);
    assert_eq!(admitted.watchdog_fire_ms(),14_900,"the admitted copy is immutable");
}
```

- [ ] **Step 5 (3 min): Run green:** `cargo test -p engine --features testing flop_budget`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): apply flop budgets and measured template policy' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 10: Route flop and turn through cache, provisional and live solve

**Files:** Modify `crates/engine/src/{core,flop,cache_bridge,log,serve,solve}.rs`, `crates/engine/tests/{flop_path_golden,support/mod}.rs`; create `crates/engine/tests/golden/flop_path.json`.

**Interfaces:** Consumes `Cache::lookup`, `Cache::store`, `cache_bridge::{key_and_source,make_cache_query}` (Task 7), Plan 2's `serve::serve_request`, `solve::{run_solve,SolvePlan,Terminal,SolveOutcome}`, `watchdog::Armed.retained`, `assemble::{accumulate,final_from_solution,hero_reach}`, `tree::{build_tree_full,tree_signature,TemplateSelection}`, Plan 3 Task 18's `Engine::register_snapshot` / `SnapshotStore::register`, `SolveInput`, and Task 9's `EngineCore.flop_policy`. Produces `flop::{CacheRoute::{Final(CacheHit),Refine{retained:Option<CacheHit>}}, choose_cache_route(Vec<Lookup>)->CacheRoute, cacheable(Street,bool,u16,bool)->bool, is_street_violation(Street,bool,bool,bool)->bool}`; `serve::{CACHE_BUDGET_MS, Probe, probe_cache, cache_phase, cache_label_for}` — `probe_cache`, `cache_phase` and `recommendation_from_hit` all take the request-captured `target_bp: u16` rather than reading `core.config`; `cache_bridge::{entry_from_solution(input:&SolveInput,solution:&StreetSolution,reasons:&[ApproxReason],bb_chips:u32,rake:&Rake,signature:&str,perm:&SuitPerm,elapsed_ms:u32,target_bp:u16)->Result<CacheEntry,UnsupportedReason>, snapshot_from_hit(identity:&DecisionIdentity,input:&SolveInput,hit:&CacheHit,origin:&str)->StreetSnapshot}`; and the `crates/engine/tests/support/mod.rs` harness `{run_flop_script(cache:Vec<Lookup>,raw:f32,status:&str)->Vec<RecommendationEvent>, final_count(&[RecommendationEvent])->usize, provisional_hit(raw_over_p:f64)->CacheHit, exact_hit()->Vec<Lookup>, approximate_hit()->Vec<Lookup>, provisional_route()->Vec<Lookup>, solve_input_for(&CacheQuery)->SolveInput}`. `run_flop_script` seeds the rig's cache directory so `Cache::lookup` really returns the scripted results, then drives Plan 2's existing fake worker with a complete validated payload; it never mocks the engine result.

`cache::lookup::{Lookup, CacheHit}` gain `#[derive(Clone)]` in this task so a probe list can be inspected after routing.

- [ ] **Step 1 (4 min): Add `deadline_best_so_far_labelling` to the existing fake-worker golden.** Fix root P100, raw1.9, target50, returned before the worker deadline; assert all action EVs, the 190/50 reason, raw stored 0.019, and `assumptions.cache == "miss"` logged without an SRP street violation. Repeat the same state under a fresh decision_id: the cache emits Provisional before live refinement, and the same hand/revision/config/model are retained. An at-target `ok` raw0.4 has no newly incurred `DeadlineBestSoFar`. A turn `best_so_far` logs a street violation. A malformed payload never enters the cache.

```rust
#[test]
fn deadline_best_so_far_labelling() {
    let events=support::run_flop_script(vec![cache::lookup::Lookup::Miss],1.9,"best_so_far");
    let final_result=events.iter().find_map(|e|match e {
        proto::RecommendationEvent::Final(r)=>Some(r),_=>None}).unwrap();
    let reasons=match &final_result.coverage {proto::Coverage::Approximate{reasons}=>reasons,
        _=>panic!("above-target result must be approximate")};
    assert!(reasons.iter().any(|r|matches!(r,proto::ApproxReason::DeadlineBestSoFar{
        reached_bp:190,target_bp:50})));
    assert!(final_result.actions.iter().all(|a|a.ev_bb.is_some()));
    assert_eq!(final_result.assumptions.cache,"miss");
    let (raw,p,target)=(1.9_f32,100_u32,50_u16);
    assert!(raw as f64/p as f64>target as f64/10000.0);
    assert_eq!((raw as f64/p as f64*10000.0).round() as u16,190);
}

#[test]
fn provisional_hit_is_emitted_then_refined() {
    let hit=support::provisional_hit(0.019);   // raw over P above the 50 bp target
    let events=support::run_flop_script(vec![cache::lookup::Lookup::Provisional{
        hit:hit.clone(),reasons:vec![proto::ApproxReason::ChartRounded]}],0.4,"ok");
    let phases=events.iter().filter_map(|e|match e {
        proto::RecommendationEvent::Provisional(r)|proto::RecommendationEvent::Final(r)=>
            Some((r.phase,r.identity.clone(),r.assumptions.cache.clone())),_=>None}).collect::<Vec<_>>();
    assert_eq!(phases.len(),2);
    assert_eq!(phases[0].0,proto::Phase::Provisional);
    assert_eq!(phases[1].0,proto::Phase::Final);
    assert_eq!(phases[0].1,phases[1].1,"identity is retained across the refinement");
    assert_eq!(phases[0].2,"provisional");
    assert_eq!(support::final_count(&events),1);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --features testing,test-templates deadline_best_so_far_labelling`; the newly added engine assertions must fail before routing changes.
- [ ] **Step 3 (5 min): Implement cache-route selection with retained provisional evidence.**

```rust
// crates/engine/src/flop.rs
pub enum CacheRoute {
    Final(cache::lookup::CacheHit),Refine{retained:Option<cache::lookup::CacheHit>},
}
pub fn choose_cache_route(results:Vec<cache::lookup::Lookup>)->CacheRoute {
    use cache::lookup::Lookup;let mut retained=None;
    for result in results {
        match result {
            Lookup::Exact{hit}|Lookup::Approximate{hit,..}=>return CacheRoute::Final(hit),
            Lookup::Provisional{hit,..}=>{
                if retained.as_ref().map_or(true,|old:&cache::lookup::CacheHit|
                    hit.raw_exploitability_over_p<old.raw_exploitability_over_p) {retained=Some(hit);}
            },Lookup::Miss=>{},
        }
    }
    CacheRoute::Refine{retained}
}
/// §10.5: the pre-solver always writes `flop_fast_v1` entries, so a flop lookup probes that
/// template first. Task 16's `presolve::BACKGROUND_TEMPLATE` re-exports this constant, which
/// keeps the routing task free of a forward dependency on `presolve.rs`.
pub const PRESOLVER_TEMPLATE:&str="flop_fast_v1";
/// §5 step 7 / §10.4: only Flop and Turn are cacheable; experimental surrogates, locked models
/// and river solves never are.
pub fn cacheable(street:proto::Street,experimental:bool,locks_applied:u16,baseline:bool)->bool {
    !experimental&&matches!(street,proto::Street::Flop|proto::Street::Turn)
        &&(!baseline||locks_applied==0)
}
/// §5 step 10: a late first terminal is always a street violation; a `best_so_far` is one
/// except on a single-raised-pot flop miss, which is the designed outcome (§7, §10.6).
pub fn is_street_violation(street:proto::Street,srp_miss:bool,best_so_far:bool,late:bool)->bool {
    late||(best_so_far&&!(street==proto::Street::Flop&&srp_miss))
}
```

The driver performs the pre-solver-template lookup first and emits a Provisional when one is present; a second distinct live-template lookup shares the same overall 500 ms cache budget and the remaining street budget. A duplicate lookup is skipped when the live template equals the pre-solver template. If a usable at-target result arrives, publish Final. If both are provisional, retain the better validated accuracy and refine; no successful at-target cache hit dispatches an unnecessary live solve.

- [ ] **Step 4 (5 min): Cut the flop/turn route into `crates/engine/src/serve.rs`.** This replaces Plan 2's `if root.street == Street::Flop { … "no flop path in this build (plan 4)" … return; }` line and inserts the cache phase between the Fast/equity phase and `build_tree_full`.

```rust
// crates/engine/src/serve.rs — additions
use crate::cache_bridge::{entry_from_solution, make_cache_query, snapshot_from_hit};
use crate::flop::{cacheable, choose_cache_route, is_street_violation, preflop_wagers, CacheRoute, FlopPolicy};
use cache::lookup::{CacheHit, CacheQuery, Lookup};

/// §7: the whole decision may spend at most 0.5 s in cache lookups, both probes together.
pub const CACHE_BUDGET_MS: u64 = 500;

pub struct Probe { pub template_id: String, pub tree: EffectiveTree, pub signature: String,
    pub pot: u32, pub result: Lookup }

#[allow(clippy::too_many_arguments)]
fn probe_cache(core:&EngineCore,root:&StreetRootSnapshot,ranges:&[Range1326;2],d:&Derived,
    state:&HandState,inherited:&[ApproxReason],perm:&core_iso::SuitPerm,template:&str,
    target_bp:u16,budget:Duration)->Option<Probe> {
    let build=build_tree_full(root,&TemplateSelection::from_history(template,&root.history)).ok()?;
    let signature=tree_signature(&build.tree,build.pot);
    let input=SolveInput{root:root.clone(),ranges:[ranges[0].clone(),ranges[1].clone()],
        tree:build.tree.clone(),target_bp};
    let query:CacheQuery=make_cache_query(&input,d,state.config.bb_chips,&state.config.rake,
        inherited,&signature,perm,target_bp,budget).ok()?;
    let result=core.cache.lookup(&query);
    Some(Probe{template_id:template.into(),tree:build.tree,signature,pot:build.pot,result})
}

/// Pre-solver template first, then a distinct live template, sharing one budget.
/// `target_bp` is the request's captured value, forwarded from `serve_request`'s single
/// `let config = core.config();`. Configuration is never recaptured mid-decision.
#[allow(clippy::too_many_arguments)]
fn cache_phase(core:&EngineCore,root:&StreetRootSnapshot,ranges:&[Range1326;2],d:&Derived,
    state:&HandState,inherited:&[ApproxReason],perm:&core_iso::SuitPerm,live_template:&str,
    target_bp:u16,t0_ms:u64)->(CacheRoute,Vec<Probe>) {
    let mut order:Vec<&str>=Vec::new();
    if root.street==Street::Flop {order.push(crate::flop::PRESOLVER_TEMPLATE);}
    if !order.contains(&live_template) {order.push(live_template);}
    let mut probes=Vec::new();
    for template in order {
        let spent=core.clock.now_ms().saturating_sub(t0_ms);
        let left=CACHE_BUDGET_MS.saturating_sub(spent);
        if left==0 {break;}
        let Some(p)=probe_cache(core,root,ranges,d,state,inherited,perm,template,target_bp,
            Duration::from_millis(left)) else {continue};
        let terminal=matches!(p.result,Lookup::Exact{..}|Lookup::Approximate{..});
        probes.push(p);
        if terminal {break;}
    }
    let route=choose_cache_route(probes.iter().map(|p|p.result.clone()).collect());
    (route,probes)
}

/// §4.4 `Assumptions.cache`: `miss | exact | approximate | provisional` (review m13).
fn cache_label_for(probes:&[Probe])->String {
    let mut label="miss";
    for p in probes {
        label=match (&p.result,label) {
            (Lookup::Exact{..},_)=>"exact",
            (Lookup::Approximate{..},"exact")=>"exact",
            (Lookup::Approximate{..},_)=>"approximate",
            (Lookup::Provisional{..},"miss")=>"provisional",
            _=>label,
        };
    }
    label.into()
}
```

```rust
// crates/engine/src/serve.rs — inside serve_request, immediately after the `Classification::HuStreet`
// destructuring, replacing Plan 2's flop rejection
// Task 9 owns the provenance-checked policy; it is loaded once at startup, never here.
let policy=&core.flop_policy;
let template=match root.street {
    Street::River=>"river_std_v1",
    Street::Turn=>"turn_std_v1",
    Street::Flop=>policy.live_template(preflop_wagers(&req.state)),
    Street::Preflop=>unreachable!("classified as Preflop above"),
};
```

```rust
// crates/engine/src/serve.rs — after the Fast event and the `fast-path` equity spawn,
// before `build_tree_full`
// `config` is `serve_request`'s existing single capture; `target_bp` travels with the request.
let target_bp=config.solver.target_bp;
let mut route=CacheRoute::Refine{retained:None};
let mut perm=core_iso::SuitPerm::IDENTITY;
if matches!(root.street,Street::Flop|Street::Turn) {
    let (_,p)=core_iso::canonicalize(&root.board,&[&ranges.oop,&ranges.ip]);
    perm=p;
    let (r,probes)=cache_phase(core,&root,&[ranges.oop.clone(),ranges.ip.clone()],&d,
        &req.state,&inherited,&perm,template,target_bp,req.t0_ms);
    assumptions.cache=cache_label_for(&probes);
    if let Some(p)=probes.iter().find(|p|matches!(p.result,Lookup::Exact{..}|Lookup::Approximate{..}
        |Lookup::Provisional{..})) {
        assumptions.template_id=p.template_id.clone();
        assumptions.tree_signature=p.signature.clone();
    }
    route=r;
}
let retained_raw=match &route {
    CacheRoute::Final(_)=>None,
    CacheRoute::Refine{retained}=>retained.as_ref().map(|h|h.raw_exploitability_over_p),
};
if let CacheRoute::Final(hit)=&route {
    let rec=recommendation_from_hit(core,&req,&ctx,hit,hero_public,hero_actor,&inherited,
        &assumptions,target_bp,Phase::Final);
    let input=SolveInput{root:root.clone(),ranges:[ranges.oop.clone(),ranges.ip.clone()],
        tree:hit.tree.clone(),target_bp};
    let origin=match assumptions.cache.as_str() {"exact"=>"cache_exact",_=>"cache_approximate"};
    let active=core.identity.lock().unwrap().active().cloned();
    if let Some(a)=active {
        // §9.2 single registration path: Plan 3 Task 18 retyped `EngineCore.snapshots` to
        // `Arc<Mutex<core_replay::SnapshotStore>>`. The snapshot is stamped with the request's
        // own identity, so `register` refuses it when the active identity has moved on.
        let snapshot=snapshot_from_hit(&req.identity,&input,hit,origin);
        core.snapshots.lock().unwrap().register(&a,snapshot);
    }
    finish(core,&req,&deadlines,&delivered,RecommendationEvent::Final(rec),None,&root,false,false);
    return;
}
```

`core_iso::SuitPerm::IDENTITY` is Plan 1 Task 21's identity-permutation constant (`pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);`), not a constructor call; the River branch never uses it. `recommendation_from_hit` is the shared assembler:

```rust
#[allow(clippy::too_many_arguments)]
fn recommendation_from_hit(core:&EngineCore,req:&LiveRequest,ctx:&AssemblyCtx,hit:&CacheHit,
    hero_public:&Range1326,hero_actor:&str,inherited:&[ApproxReason],
    assumptions:&Assumptions,target_bp:u16,phase:Phase)->Recommendation {
    let requested=hit.solution.requested as usize;
    let mut a=assumptions.clone();
    a.source=format!("cache@{}",proto::worker::SOLVER_COMMIT);
    a.source_accuracy=format!("raw exploitability_over_P {:.6}",hit.raw_exploitability_over_p);
    a.reached_bp=Some((hit.raw_exploitability_over_p*10_000.0).round() as u16);
    a.elapsed_ms=(core.clock.now_ms()-req.t0_ms) as u32;
    a.notes.extend(hit.notes.iter().cloned());
    let coverage=crate::assemble::accumulate(hit.coverage.clone(),inherited.to_vec());
    let reach=crate::assemble::hero_reach(&hit.solution.nodes,&hit.covered_paths,requested,
        hero_public,hero_actor);
    let mut rec=crate::assemble::final_from_solution(ctx,&hit.solution.nodes[requested],&reach,
        coverage,a);
    rec.phase=phase;
    rec
}
```

- [ ] **Step 4a (5 min): Emit the Provisional and hand its payload to the watchdog (`solve.rs`/`core.rs`).** A retained cache payload must survive a live terminal failure, so it is written into Plan 2's `Armed.retained` slot before admission.

```rust
// crates/engine/src/serve.rs — the Refine branch, before build_tree_full/run_solve
if let CacheRoute::Refine{retained:Some(hit)}=&route {
    let mut provisional_assumptions=assumptions.clone();
    provisional_assumptions.cache="provisional".into();
    let rec=recommendation_from_hit(core,&req,&ctx,hit,hero_public,hero_actor,&inherited,
        &provisional_assumptions,target_bp,Phase::Provisional);
    let input=SolveInput{root:root.clone(),ranges:[ranges.oop.clone(),ranges.ip.clone()],
        tree:hit.tree.clone(),target_bp};
    let active=core.identity.lock().unwrap().active().cloned();
    if let Some(a)=active {
        let snapshot=snapshot_from_hit(&req.identity,&input,hit,"cache_provisional");
        core.snapshots.lock().unwrap().register(&a,snapshot);
    }
    emit(core,&req,None,RecommendationEvent::Provisional(rec.clone()));
    // §7: if the live refinement misses final delivery the watchdog emits this exact payload
    let mut promoted=rec;promoted.phase=Phase::Final;
    *retained.lock().unwrap()=Some(promoted);
}
```

```rust
// crates/engine/src/serve.rs — after run_solve, replacing Plan 2's `Terminal::Ok | BestSoFar` arm
let live_over_p=out.solution.as_ref().map(|s|s.exploitability_chips as f64/build.pot as f64);
let keep_retained=match (retained_raw,live_over_p) {
    (Some(cached),Some(live))=>cached<live,
    (Some(_),None)=>true,
    _=>false,
};
```

When `keep_retained` is true the Final is the retained payload already sitting in `retained` (promoted to `Phase::Final`), with the live terminal still logged: `assumptions.notes` gains `format!("live refinement reached raw {live:.6}; the retained cache payload {cached:.6} is better")`, both accuracies are disclosed, and the incurred reasons of both sources accumulate. A live refinement never claims to have resumed a finalized solver; it starts fresh. The retry policy is unchanged: once, under Plan 2's `retry_admitted` with the `_min` template's p95, and `tree_mismatch` never retries. `solve.rs` needs one change only — `SolvePlan.background` is already a field, so pass `background: false` here and `true` from Task 16.

- [ ] **Step 4b (5 min): Build and enqueue the entry, and build the hit snapshot (`cache_bridge.rs`).**

```rust
// crates/engine/src/cache_bridge.rs
/// Store the terminal solution of a Flop/Turn solve. The **actual result tree** is used
/// (after a `_min` retry it is the retry's tree), never the first-attempt template.
#[allow(clippy::too_many_arguments)]
pub fn entry_from_solution(input:&SolveInput,solution:&proto::worker::StreetSolution,
    reasons:&[ApproxReason],bb_chips:u32,rake:&Rake,signature:&str,perm:&SuitPerm,
    elapsed_ms:u32,target_bp:u16)->Result<cache::entry::CacheEntry,UnsupportedReason> {
    let (key,source)=key_and_source(input,bb_chips,rake,signature,perm)?;
    let pot=source.pot;
    // forward-map every combo row into canonical suits (the same copier, with `perm` instead
    // of its inverse), then let `normalize` divide EV by P and resolve chip paths to ordinals
    let mut canonical=solution.clone();
    for node in &mut canonical.nodes {
        node.probs=cache::lookup::map_rows(&node.probs,perm);
        node.ev_chips=cache::lookup::map_rows(&node.ev_chips,perm);
        node.available=cache::lookup::map_flags(&node.available,perm);
    }
    let nodes=cache::entry::normalize(&canonical,&input.tree,pot)
        .map_err(|e|unsupported(&format!("cache normalize: {e}")))?;
    let fractions=input.tree.materialized.iter().map(|n|n.actions.iter().map(|a|match a {
        Action::Bet{to}|Action::Raise{to}|Action::AllIn{to}=>
            cache::key::Rational::new(*to as u64,pot as u64).ok(),_=>None})
        .collect::<Vec<_>>()).collect::<Vec<_>>();
    let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
        .map_or(0,|d|d.as_millis() as u64);
    let entry=cache::entry::CacheEntry{key,source,tree:input.tree.clone(),fractions,
        covered_paths:nodes.iter().map(|n|n.path.clone()).collect(),nodes,
        exploitability_over_P:solution.exploitability_chips as f64/pot as f64,
        target_bp,iterations:solution.iterations,elapsed_ms,memory_bytes:solution.memory_bytes,
        mode:solution.mode.clone(),locks_applied:solution.locks_applied,
        export:solution.export.clone(),reasons:reasons.to_vec(),created:now,last_hit:now};
    cache::entry::validate_entry(&entry).map_err(|e|unsupported(&format!("cache entry: {e}")))?;
    Ok(entry)
}

/// §9.1 snapshot built from the **query**, never from stored chip amounts.
pub fn snapshot_from_hit(id:&proto::DecisionIdentity,input:&SolveInput,
    hit:&cache::lookup::CacheHit,origin:&str)->core_replay::StreetSnapshot {
    let reasons=match &hit.coverage {proto::Coverage::Exact=>Vec::new(),
        proto::Coverage::Approximate{reasons}=>reasons.clone(),
        proto::Coverage::Unsupported{partial,..}=>partial.clone()};
    core_replay::StreetSnapshot {
        key:core_replay::SnapshotKey{hand_id:id.hand_id,config_revision:id.config_revision,
            model_revision:id.model_revision,street:input.root.street,
            root_board:input.root.board.clone(),
            root_range_hashes:[core_ranges::hash_scaled(&input.ranges[0]),
                               core_ranges::hash_scaled(&input.ranges[1])],
            tree_signature:hit.tree_signature.clone()},
        provenance:core_replay::SnapshotProvenance{identity_at_solve:id.clone(),
            solved_prefix:input.root.history.clone(),origin:origin.into()},
        tree:hit.tree.clone(),nodes:hit.solution.nodes.clone(),
        covered_paths:hit.covered_paths.clone(),
        exploitability_chips:hit.solution.exploitability_chips,reasons,
    }
}
```

```rust
// crates/engine/src/serve.rs — after the live Final is assembled and registered
if let (Terminal::Ok|Terminal::BestSoFar,Some(sol))=(&out.terminal,&out.solution) {
    let baseline=true;   // §10.4 model = baseline in phase 1
    if cacheable(root.street,false,sol.locks_applied,baseline) {
        let stored=SolveInput{root:root.clone(),ranges:[ranges.oop.clone(),ranges.ip.clone()],
            tree:out.tree.clone(),target_bp};
        let signature=tree_signature(&out.tree,build.pot);
        match entry_from_solution(&stored,sol,&inherited,req.state.config.bb_chips,
            &req.state.config.rake,&signature,&perm,elapsed,target_bp) {
            Ok(entry)=>core.cache.store(&entry),
            Err(reason)=>core.log_cache_reject(&req.identity,&reason),
        }
    }
}
```

`EngineCore::log_cache_reject(&DecisionIdentity,&UnsupportedReason)` writes one `eprintln!` per session and is added to `core.rs`; a malformed or unvalidatable payload therefore never enters the cache and never affects delivery. `Cache::store` is non-blocking, so no delivery path waits for the writer. The river branch is unchanged and never stores. Finally, Plan 2's `finish` learns the real label: replace its hard-coded `cache: "miss".into()` with `cache: rec.assumptions.cache.clone()` and its inline street-violation expression with `is_street_violation(root.street, srp_miss, best_so_far, final_violation_of_first_attempt)`, where `srp_miss` is `root.street == Street::Flop && preflop_wagers(&req.state) < 3 && rec.assumptions.cache == "miss"`.

- [ ] **Step 5 (5 min): Complete flop-path goldens.** Cases: exact synthetic hit (no reasons), chart hit (`ChartRounded`), SPR-only, menu-only, both, provisional->ok, provisional->no_iteration, cold SRP best_so_far, cold SRP raw-target ok, 3-bet fast, stale hit/result discarded, missed actor/path, disk blocked, turn store, river no store. Freeze canonical event projections (phase, identity, reasons, actions/EVs, mode, `assumptions.cache`, snapshot origin) in `flop_path.json`; exclude wall-clock nondeterminism. Assert exactly one Final at or before watchdog expiry even when the cache reader stays blocked.

```rust
// crates/engine/tests/support/mod.rs
pub fn final_count(events:&[proto::RecommendationEvent])->usize {
    events.iter().filter(|e|matches!(e,proto::RecommendationEvent::Final(_))).count()
}
```

```rust
#[test]
fn cache_labels_are_recorded_for_every_route() {
    for (route,expected) in [
        (support::exact_hit(),"exact"),
        (support::approximate_hit(),"approximate"),
        (support::provisional_route(),"provisional"),
        (vec![cache::lookup::Lookup::Miss],"miss"),
    ] {
        let events=support::run_flop_script(route,0.4,"ok");
        let last=events.iter().rev().find_map(|e|match e {
            proto::RecommendationEvent::Final(r)=>Some(r),_=>None}).unwrap();
        assert_eq!(last.assumptions.cache,expected);
        assert_eq!(support::final_count(&events),1);
    }
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --features testing,test-templates --test flop_path_golden`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): complete cache provisional and live flop routing' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 11: Create the section 6 experimental synthetic-root surrogate

Cross-plan Or1/R4 and review m6: no earlier plan builds the `experimental` block, so this task **creates** it for River, Turn and Flop and adds the §13.3 golden under its spec name.

**Files:** Create `crates/engine/src/experimental.rs`, `crates/engine/tests/experimental_surrogate.rs`, `crates/engine/tests/golden/experimental_surrogate.json`; modify `crates/engine/src/{lib,serve,solve}.rs`, `crates/engine/tests/support/mod.rs`.

**Interfaces:** Consumes `proto::{ExperimentalHu, EXPERIMENTAL_NOTE, ActionAdvice, Derived, Seat, Range1326}`, Plan 2 Task 25's existing `engine::equity::range_vs_range(hero_public:&Range1326, opp_public:&Range1326, board:&[Card], budget:Duration, cancel:&AtomicBool) -> Option<(f32, EquityMethod)>`, Plan 2's `deadline::Deadlines`, `solve::{run_solve, SolvePlan, Terminal}`, `tree::{materialize_at, Templates, TemplateSelection, build_tree_full}`. This task creates **no** equity helper: Plan 2 Task 25 is the single owner of `range_vs_range` and its tuple result is consumed as-is. Produces `experimental::{SurrogateInput{hero:Seat,opponent:Seat,pot:u32,stack:u32,hero_role:&'static str,template_id:String}, choose_opponent(hero:Seat,hero_public:&Range1326,others:&[(Seat,Range1326)],board:&[Card],budget:Duration,cancel:&AtomicBool)->Option<Seat>, surrogate_input(d:&Derived,state:&HandState,hero:Seat,opponent:Seat,street:Street,template_id:&str)->Option<SurrogateInput>, run_surrogate(core:&mut EngineCore,input:&SurrogateInput,ranges:[Range1326;2],board:&[Card],hero_cards:[Card;2],bb_chips:u32,rake:&Rake,target_bp:u16,deadlines:&Deadlines,identity:&DecisionIdentity,sink:&SharedSink)->Option<ExperimentalHu>}`; and in `solve.rs` the shared internal transport extracted from `run_solve`, `solve_request_from_parts` and `send_solve_request`, with the exact `pub(crate)` signatures given in Step 4; and the `crates/engine/tests/support/mod.rs` helpers `{three_way_flop()->HandState, three_way_turn()->HandState, three_way_river()->HandState, three_way_flop_with_all_in_opponent()->HandState, street_root_public_ranges(&HandState)->RootRanges2{hero:Range1326,others:Vec<(Seat,Range1326)>}, run_three_way_flop_script(&HandState)->Vec<RecommendationEvent>, snapshot_count(&HandState)->usize, cache_entry_count()->usize}`, all built on Plan 2's `FakeWorker`/`FakeClock` and the rig's temporary cache directory.

- [ ] **Step 1 (4 min): Write `experimental_surrogate_golden` with the spec's three-way flop.**

```rust
#[test]
fn experimental_surrogate_golden() {
    // three-way flop: hero BB (Seat 2), UTG (Seat 3) and BTN (Seat 0) still in.
    // Pot 300 at the decision, remaining stacks hero 700, UTG 900, BTN 400.
    let state=support::three_way_flop();
    let d=core_model::derive(&state);
    let roots=support::street_root_public_ranges(&state);       // unconditioned street-root ranges
    let cancel=std::sync::atomic::AtomicBool::new(false);
    let opponent=engine::experimental::choose_opponent(state.hero,&roots.hero,&roots.others,
        &state.board,std::time::Duration::from_millis(500),&cancel).unwrap();
    let input=engine::experimental::surrogate_input(&d,&state,state.hero,opponent,
        proto::Street::Flop,"flop_fast_v1").unwrap();
    assert_eq!(input.pot,d.pot,"the synthetic pot is the current total pot");
    assert_eq!(input.stack,d.stacks_remaining[state.hero.0 as usize]
        .min(d.stacks_remaining[opponent.0 as usize]),"the synthetic stack is the minimum");
    assert_eq!(input.template_id,"flop_fast_v1","same template as the street");
    assert!(matches!(input.hero_role,"oop"|"ip"));
    let events=support::run_three_way_flop_script(&state);
    let final_result=events.iter().rev().find_map(|e|match e {
        proto::RecommendationEvent::Final(r)=>Some(r),_=>None}).unwrap();
    // §6: MultiwayEv, no numeric EV in `actions`, the surrogate lives in its own block
    assert!(matches!(final_result.coverage,
        proto::Coverage::Unsupported{reason:proto::UnsupportedReason::MultiwayEv{..},..}));
    assert!(final_result.actions.iter().all(|a|a.ev_bb.is_none()));
    let experimental=final_result.experimental.as_ref().expect("experimental block");
    assert_eq!(experimental.note,proto::EXPERIMENTAL_NOTE);
    assert_eq!(experimental.opponent,opponent);
    assert_eq!((experimental.pot,experimental.stack),(input.pot,input.stack));
    assert!(!experimental.actions.is_empty());
    // never Exact, never a snapshot, never a cache entry
    assert!(!matches!(final_result.coverage,proto::Coverage::Exact));
    assert_eq!(support::snapshot_count(&state),0);
    assert_eq!(support::cache_entry_count(),0);
    let frozen=serde_json::json!({"opponent":experimental.opponent,"hero_role":experimental.hero_role,
        "pot":experimental.pot,"stack":experimental.stack,"template_id":experimental.template_id,
        "ranges_used":experimental.ranges_used,"note":experimental.note});
    if std::env::var_os("POKERAI_RECORD_GOLDENS").is_some() {
        std::fs::write("tests/golden/experimental_surrogate.json",
            serde_json::to_vec_pretty(&frozen).unwrap()).unwrap();
    }
    let expected:serde_json::Value=serde_json::from_slice(
        &std::fs::read("tests/golden/experimental_surrogate.json").unwrap()).unwrap();
    assert_eq!(frozen,expected);
}

#[test]
fn surrogate_is_skipped_when_the_opponent_is_all_in_or_stackless() {
    let state=support::three_way_flop_with_all_in_opponent();
    let d=core_model::derive(&state);
    assert!(engine::experimental::surrogate_input(&d,&state,state.hero,proto::Seat(3),
        proto::Street::Flop,"flop_fast_v1").is_none());
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --features testing --test experimental_surrogate`; the module does not exist.
- [ ] **Step 3 (5 min): Implement opponent selection and the synthetic root.**

No equity function is defined here. Plan 2 Task 25 already owns `engine::equity::range_vs_range`, including the §7 exact/Monte-Carlo selection rule, and returns `Option<(f32, EquityMethod)>`; the surrogate destructures that tuple and ignores the method.

```rust
// crates/engine/src/experimental.rs
use crate::core::EngineCore;
use crate::deadline::Deadlines;
use crate::solve::{run_solve, SolvePlan, Terminal};
use crate::tree::{build_tree_full, TemplateSelection};
use crate::watchdog::SharedSink;
use proto::*;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub struct SurrogateInput {
    pub hero:Seat,pub opponent:Seat,pub pot:u32,pub stack:u32,
    pub hero_role:&'static str,pub template_id:String,
}

/// §6: the opponent is the seat whose public range has the highest range-vs-range equity
/// against hero's public range. A seat whose equity cannot be computed inside the budget is
/// skipped rather than guessed.
pub fn choose_opponent(hero:Seat,hero_public:&Range1326,others:&[(Seat,Range1326)],
    board:&[Card],budget:Duration,cancel:&AtomicBool)->Option<Seat> {
    let per_seat=budget.checked_div(others.len().max(1) as u32)?;
    let mut best:Option<(Seat,f32)>=None;
    for (seat,range) in others {
        if *seat==hero {continue;}
        let Some((equity,_method))=crate::equity::range_vs_range(
            range,hero_public,board,per_seat,cancel
        ) else {continue};
        if best.as_ref().map_or(true,|(_,b)|equity>*b) {best=Some((*seat,equity));}
    }
    best.map(|(seat,_)|seat)
}

/// §6 synthetic root: total pot, minimum remaining stack, empty history, hero OOP iff hero
/// precedes the opponent in postflop order. `None` skips the surrogate.
pub fn surrogate_input(d:&Derived,state:&HandState,hero:Seat,opponent:Seat,street:Street,
    template_id:&str)->Option<SurrogateInput> {
    if *d.all_in.get(opponent.0 as usize)? {return None;}
    let stack=(*d.stacks_remaining.get(hero.0 as usize)?)
        .min(*d.stacks_remaining.get(opponent.0 as usize)?);
    if stack==0||d.pot==0 {return None;}
    // seats in postflop action order
    let order=core_model::postflop_order(state.button,&state.dealt);
    let hero_first=order.iter().position(|s|*s==hero)?<order.iter().position(|s|*s==opponent)?;
    Some(SurrogateInput{hero,opponent,pot:d.pot,stack,
        hero_role:if hero_first {"oop"} else {"ip"},template_id:template_id.into()})
}
```

`core_model::postflop_order(button: Seat, dealt: &[Seat]) -> Vec<Seat>` is Plan 1 Task 9's postflop ordering helper; that is the exact producer signature and it takes the button and the dealt seats, not the whole state. The surrogate never constructs a `SolveInput` and never touches the cache or the snapshot store: it builds its own `StreetRootSnapshot` with `history: vec![]`, `dead_this_street: 0` and `projected_from` equal to the number of pot-eligible seats, and it always reads the root node when hero is OOP and the node after OOP's check when hero is IP.

- [ ] **Step 4 (5 min): Run the isolated solve and attach the block.**

**Surrogate contract (binding; this is a required interface repair, not a description of already-correct sample code).** `run_surrogate` receives the actual hero cards for advice extraction, the hand's `bb_chips` and `rake`, and the request-captured `target_bp`, in addition to its existing synthetic input, public ranges, board, deadline, identity and sink. The hero cards never condition those public solve ranges. Build the synthetic tree from pot, equal effective stacks and empty history, but **never construct `SolveInput`**. This task factors the existing worker request/response, validation, heartbeat, cancellation and deadline handling below `run_solve` into a shared internal transport function accepting a worker `SolveRequest`; `run_solve` continues to be the main-path `SolveInput` adapter, and the surrogate constructs its `SolveRequest` directly. Its output bypasses cache and snapshot registration. Validate the entire returned solution, resolve the root or check child for hero's role, select the actual hero combo row, and use `ev_chips / bb_chips` once to construct `ActionAdvice`. Preserve fold EV zero and unavailable/out-of-support handling; do **not** call `advice_rows` (no such upstream API exists — Plan 2 Task 26 owns `final_from_solution`) and never pass an empty reach vector to `final_from_solution`. Return only `ExperimentalHu`, with `EXPERIMENTAL_NOTE`; the main recommendation stays `Unsupported{MultiwayEv}` with `actions` empty. Use the real rake, never a hard-coded `TimeCharge`, and the real BB, never 2. Skip the surrogate on an all-in opponent, a zero stack, an exhausted equity/remaining budget or a worker failure.

**Exact extracted transport signatures (binding):**

```rust
pub(crate) fn solve_request_from_parts(
    core: &mut EngineCore,
    root: &proto::StreetRootSnapshot,
    ranges: &[proto::Range1326; 2],
    target_bp: u16,
    plan: &SolvePlan,
    build: &crate::tree::TreeBuild,
    deadline_ms: u32,
) -> proto::worker::SolveRequest;

pub(crate) fn send_solve_request(
    core: &mut EngineCore,
    request: proto::worker::SolveRequest,
    plan: &SolvePlan,
    build: &crate::tree::TreeBuild,
    sink: &SharedSink,
) -> SolveOutcome;
```

Plan 4 Task 11 owns these crate-visible extractions. The builder preserves Plan 2's request-id allocation, spot hash, memory limit, rake, history and background flag without constructing SolveInput. The sender shares one-attempt transport, whole-solution validation, identity checks, heartbeat, cancellation and absolute deadlines. Keep main-path requested-node/hero-actor checks and Task 23 retry/admission policy in run_solve; the surrogate validates its separately selected root/check-child advice row and skips on failure.

```rust
#[allow(clippy::too_many_arguments)]
pub fn run_surrogate(core:&mut EngineCore,input:&SurrogateInput,ranges:[Range1326;2],
    board:&[Card],hero_cards:[Card;2],bb_chips:u32,rake:&Rake,target_bp:u16,
    deadlines:&Deadlines,identity:&DecisionIdentity,sink:&SharedSink)
    ->Option<ExperimentalHu> {
    if bb_chips==0 {return None;}
    let root=StreetRootSnapshot{street:if board.len()==5 {Street::River}
            else if board.len()==4 {Street::Turn} else {Street::Flop},
        board:board.to_vec(),
        oop:if input.hero_role=="oop" {input.hero} else {input.opponent},
        ip:if input.hero_role=="oop" {input.opponent} else {input.hero},
        pot_root:input.pot,stack_oop_root:input.stack,stack_ip_root:input.stack,
        dead_this_street:0,projected_from:2,bb_chips,history:vec![]};
    let build=build_tree_full(&root,
        &TemplateSelection::from_history(&input.template_id,&[])).ok()?;
    // No SolveInput: the surrogate builds the worker request itself and sends it through the
    // transport function shared with run_solve (spec §6 / S:L423 separate-contract rule).
    let plan=SolvePlan{identity:identity.clone(),deadlines:deadlines.clone(),
        template_id:input.template_id.clone(),retry_template_id:None,
        rake:rake.clone(),hero_actor:input.hero_role.into(),background:false};
    let deadline_ms=plan.deadlines.worker_deadline_ms(core.clock.now_ms(),plan.deadlines.street_deadline_ms)?;
    let request=crate::solve::solve_request_from_parts(core,&root,&ranges,target_bp,
        &plan,&build,deadline_ms);
    let out=crate::solve::send_solve_request(core,request,&plan,&build,sink);
    let solution=match (&out.terminal,&out.solution) {
        (Terminal::Ok|Terminal::BestSoFar,Some(s))=>s.clone(),_=>return None};
    proto::worker::validate_solution(&solution,&build.tree.materialized).ok()?;
    // hero reads the root when OOP, the node after OOP's check when IP
    let path:Vec<Action>=if input.hero_role=="oop" {vec![]} else {vec![Action::Check]};
    let ordinal=proto::resolve_chip_path(&build.tree.materialized,&path)?;
    let index=out.ordinal_paths.iter().position(|p|*p==ordinal)?;
    let node=solution.nodes.get(index)?;
    // the actual hero combo row, never an empty reach vector
    let combo=core_ranges::combo_index(hero_cards[0],hero_cards[1])?;
    if !*node.available.get(combo)? {return None;}
    let actions=node.actions.iter().enumerate().map(|(a,action)| {
        let ev_chips=if matches!(action,Action::Fold) {0.0}
            else {*node.ev_chips.get(combo).and_then(|row|row.get(a))?};
        Some(ActionAdvice{action:action.clone(),
            frequency:node.probs.get(combo).and_then(|row|row.get(a)).copied(),
            ev_bb:Some(ev_chips/bb_chips as f32)})
    }).collect::<Option<Vec<_>>>()?;
    Some(ExperimentalHu{opponent:input.opponent,hero_role:input.hero_role.into(),
        pot:input.pot,stack:input.stack,template_id:input.template_id.clone(),
        ranges_used:[(root.oop,core_ranges::range_to_string(&ranges[0]),core_ranges::mass(&ranges[0])),
                     (root.ip,core_ranges::range_to_string(&ranges[1]),core_ranges::mass(&ranges[1]))],
        actions,reached_bp:out.reached_bp,elapsed_ms:out.elapsed_ms,
        note:EXPERIMENTAL_NOTE.into()})
}
```

`crates/engine/src/solve.rs` is therefore modified by this task: `solve_request_from_parts` and `send_solve_request` are the extracted transport seam, and `run_solve` keeps its existing signature by calling the same two functions. This plan adds no second `WorkerLink`, no second deadline arithmetic and no second validator.

In `serve.rs` the `Classification::Multiway { pot_eligible }` arm now builds the surrogate before emitting, using the **street-root unconditioned** public ranges of hero and the chosen opponent, the hand's real `bb_chips` and `rake`, the request-captured `target_bp`, the same template the street would use and the remaining street budget; it attaches the result to `rec.experimental` and leaves `rec.actions` empty. The surrogate is skipped when the equity phase overran its budget, when `surrogate_input` returns `None`, or when the solve fails: in every case `experimental` stays `None` and the `Unsupported{MultiwayEv}` result is unchanged.

- [ ] **Step 5 (4 min): Prove the isolation.** Assert that after a surrogate run the snapshot store for the hand is empty, the cache directory contains no `.bin` file, the main `actions` list carries no EV, and the coverage is never `Exact`. Repeat the golden for a three-way turn and a three-way river so all three streets are exercised, and add a case where the highest-equity opponent is all-in: the next-best seat is **not** substituted, the surrogate is skipped entirely.

```rust
#[test]
fn surrogate_never_enters_cache_or_snapshots_on_any_street() {
    for state in [support::three_way_flop(),support::three_way_turn(),support::three_way_river()] {
        let events=support::run_three_way_flop_script(&state);
        let last=events.iter().rev().find_map(|e|match e {
            proto::RecommendationEvent::Final(r)=>Some(r),_=>None}).unwrap();
        assert!(last.experimental.is_some());
        assert_eq!(support::snapshot_count(&state),0);
        assert_eq!(support::cache_entry_count(),0);
    }
}

/// The surrogate uses the hand's real BB and the real hero combo, not 2 and not a blank reach.
#[test]
fn surrogate_uses_real_bb_and_the_actual_hero_combo_when_ip() {
    let state=support::three_way_flop_ip_hero_bb_100();   // bb_chips = 100, hero acts IP
    let events=support::run_three_way_flop_script(&state);
    let last=events.iter().rev().find_map(|e|match e {
        proto::RecommendationEvent::Final(r)=>Some(r),_=>None}).unwrap();
    let experimental=last.experimental.as_ref().unwrap();
    assert_eq!(experimental.hero_role,"ip");
    // ev_bb is ev_chips / 100, never ev_chips / 2
    let ev_bb=experimental.actions.iter().find_map(|a|a.ev_bb).unwrap();
    assert!(ev_bb.abs()<experimental.stack as f32/100.0+1.0);
    // a different hero combo in the same public spot yields a different advice row
    let other=support::three_way_flop_ip_hero_bb_100_other_combo();
    let other_events=support::run_three_way_flop_script(&other);
    let other_last=other_events.iter().rev().find_map(|e|match e {
        proto::RecommendationEvent::Final(r)=>Some(r),_=>None}).unwrap();
    assert_ne!(other_last.experimental.as_ref().unwrap().actions,experimental.actions);
    // the public ranges are identical: hero cards never condition them
    assert_eq!(other_last.experimental.as_ref().unwrap().ranges_used,experimental.ranges_used);
}
```

Add `support::{three_way_flop_ip_hero_bb_100, three_way_flop_ip_hero_bb_100_other_combo}` to the helper list in this task's Interfaces block.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --features testing --test experimental_surrogate`; `cargo test --workspace`. Record the golden once with `POKERAI_RECORD_GOLDENS=1`, review it, then unset the variable.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): add the section 6 experimental synthetic-root surrogate' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 12: Register cache snapshots and translate prior-street bets

**Files:** Modify `crates/engine/src/{cache_bridge,snapshots}.rs`, `crates/engine/tests/support/mod.rs`; create `crates/engine/tests/cache_snapshot_replay.rs`, `crates/engine/tests/golden/cache_snapshot_replay.json`.

**Interfaces:** Consumes `core_replay::{SnapshotKey, SnapshotProvenance, StreetSnapshot, SnapshotStore, ReplayInput, replay}` (Plan 3 Task 13 defines snapshot records; Task 14 replaces Plan 2 Task 27's `SolvedStreet` and `SnapshotStore` with the `core_replay` types), `cache_bridge::snapshot_from_hit` (Task 10), Plan 3 Task 18's `Engine::register_snapshot(&mut self, active:&DecisionIdentity, snapshot:StreetSnapshot)->bool` and `SnapshotStore::register`. Produces the cached prior-street translation goldens and `snapshots::CACHE_ORIGINS: [&str; 3]`. This plan adds **no** second registration path and advertises no free `engine::snapshots::register_snapshot` function (cross-plan M15/D1): the cache route calls the same `EngineCore.snapshots` store that live results use. Root hashes are the query public range hashes at that root; snapshot tree/nodes are query-sized, original-suit values.

- [ ] **Step 1 (5 min): Write cache-hit replay integration assertions.** Cache-hit Final at the flop root, append Bet50/Call and turn 4d: the root and the IP call likelihoods each condition once, the returned turn input uses replayed marginals, and no turn strategy is requested from the flop entry. Equivalent 100/500 and 200/1000 paths must produce equal normalized turn ranges. Then solve at prefix Check, append villain Bet73/Call against snapshot menus 50/100: expect a prior-street `BetTranslation`, and the turn solves from the turn root.

```rust
#[test]
fn prior_street_translation_weights_are_fixed() {
    let (a,b,s)=(0.5_f64,1.0_f64,0.73_f64);
    let fa=(b-s)*(1.0+a)/((b-a)*(1.0+s));let fb=1.0-fa;
    assert!((fa-0.4682080924855491).abs()<1e-12);
    let marginal=[fa*0.9+fb*0.1,fa*0.3+fb*0.5];
    assert!((marginal[0]-0.4745664739884393).abs()<1e-12);
    assert!((marginal[1]-0.4063583815028902).abs()<1e-12);
    assert!((s-a).min((b-s).abs())>0.10);
}
#[test]
fn cache_hit_snapshot_keeps_query_identity_and_paths() {
    let rig=CacheRig::new("flop_fast_v1",100,500,5000);
    let query=rig.query(100,500,5000,&[],"oop");let hit=rig.hit(&query);
    let id=proto::DecisionIdentity{hand_id:1,hand_revision:7,decision_id:9,config_revision:1,model_revision:0};
    let input=proto::SolveInput{root:proto::StreetRootSnapshot{street:proto::Street::Flop,
        board:query.key.canonical_board.clone(),oop:proto::Seat(2),ip:proto::Seat(0),pot_root:100,
        stack_oop_root:500,stack_ip_root:500,dead_this_street:0,projected_from:2,bb_chips:2,
        history:vec![]},
        ranges:query.source.ranges.clone(),tree:query.tree.clone(),target_bp:50};
    let snapshot=engine::cache_bridge::snapshot_from_hit(&id,&input,&hit,"cache_exact");
    assert_eq!(snapshot.provenance.identity_at_solve,id);
    assert_eq!(snapshot.covered_paths,hit.covered_paths);
    assert_eq!(snapshot.tree,input.tree);
}
```

`StreetRootSnapshot` carries `bb_chips` (Plan 1 dev 1, spec S1, cross-plan M9); every literal in this plan sets it.

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --features testing,test-templates --test cache_snapshot_replay`; cache origin/prefix/range assertions fail until registration is wired.
- [ ] **Step 3 (5 min): Prove the cache route uses the existing registration path.** Plan 3 Task 14 already made `crates/engine/src/snapshots.rs` a re-export of `core_replay::{SnapshotStore, StreetSnapshot, SnapshotKey, SnapshotProvenance}` and Plan 3 Task 18 added `Engine::register_snapshot`. This plan adds only the origin constant and the test that pins the shared behaviour.

```rust
// crates/engine/src/snapshots.rs — one added constant beside Plan 3's re-exports
/// §9.1 provenance origins written by the cache route; live solves keep `"live"`.
pub const CACHE_ORIGINS:[&str;3]=["cache_exact","cache_approximate","cache_provisional"];
```

```rust
#[test]
fn cache_and_live_registrations_share_one_store_and_one_rule() {
    let rig=CacheRig::new("flop_fast_v1",100,500,5000);
    let mut store=core_replay::SnapshotStore::new();
    let active=proto::DecisionIdentity{hand_id:1,hand_revision:7,decision_id:9,
        config_revision:1,model_revision:0};
    let stale=proto::DecisionIdentity{decision_id:8,..active.clone()};
    let query=rig.query(100,500,5000,&[],"oop");let hit=rig.hit(&query);
    let input=support::solve_input_for(&query);
    for origin in engine::snapshots::CACHE_ORIGINS {
        let fresh=engine::cache_bridge::snapshot_from_hit(&active,&input,&hit,origin);
        assert!(store.register(&active,fresh));
        let old=engine::cache_bridge::snapshot_from_hit(&stale,&input,&hit,origin);
        assert!(!store.register(&active,old),"a stale identity never registers");
    }
    assert_eq!(store.for_identity(&active).len(),1,"a later origin replaces the earlier one");
}
```

Origins are exactly `cache_exact`, `cache_approximate`, `cache_provisional` for cache hits and `live` for solver results. `CacheHit.tree_signature` is set directly from the query key, never parsed from display notes. Provisional replacement uses the full identity; later-street invalidation and solved-prefix retention remain Plan 3's behaviour.

- [ ] **Step 4 (5 min): Exercise all three export coverages and later replay.** Requested-node-only covering `[Check]`: the missing root does not condition OOP, the translated villain wager does condition, an uncovered call does not. Root-only: the check conditions, the villain wager and call do not. Complete street: all three actions condition. Preserve already conditioned masses when a later path is uncovered. Compare golden 1326 arrays and `log_reach`, branch weights and reasons. Add turn->river translation with the same cases; no prior street is re-solved. A current-street 73 is inserted exactly alongside 50, never translated. Undo keeps prefix-compatible snapshots and discards incompatible or later ones; a mismatched hand/config/model/ranges/board cannot be reused.

```rust
let expected_origins=["cache_exact","cache_approximate","cache_provisional"];
assert!(expected_origins.contains(&snapshot.provenance.origin.as_str()));
assert_eq!(snapshot.provenance.identity_at_solve,identity_at_validation);
assert_eq!(snapshot.covered_paths.len(),snapshot.nodes.len());
```

- [ ] **Step 5 (3 min): Run green:** `cargo test -p engine --features testing,test-templates --test cache_snapshot_replay`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): replay query-sized cache snapshots across streets' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 13: Enumerate presolver tiers and canonical-flop order

**Files:** Create `crates/cache/src/presolver/{mod,scenarios}.rs`, `crates/cache/tests/presolver_queue.rs`; modify `crates/cache/src/lib.rs`.

**Interfaces:** Consumes `Position`, `Card`, `core_iso::canonicalize`, `core_iso::orbit_size`; Produces `Scenario{tier:u8,depth_bb:u16,opener:Position,caller:Position,three_bettor:Option<Position>}`, `Scenario::id(&self)->String`, `scenarios()->Vec<Scenario>`, `canonical_flops_ordered()->&'static [Vec<Card>]`, `raw_flops()->impl Iterator<Item=[Card;3]>`. Each scenario's id is a deterministic serialization of these fields.

- [ ] **Step 1 (3 min): Write tier counts and ordering tests.**

```rust
#[test]
fn tier_scenarios_and_flop_order_are_complete() {
    use cache::presolver::scenarios::*;
    let s=scenarios();assert_eq!(s.len(),24);
    assert_eq!(s.iter().filter(|x|x.tier==1).count(),4);
    assert_eq!(s.iter().filter(|x|x.tier==2).count(),8);
    assert_eq!(s.iter().filter(|x|x.tier==3).count(),12);
    assert_eq!(1755*4,7020);assert_eq!(1755*24,42120);
    // the cheap half of the contract: the count is asserted from the frozen constant, and the
    // expensive enumeration runs only under `--features exhaustive` (review m5).
    assert_eq!(CANONICAL_FLOP_COUNT,1755);
}

#[cfg(feature="exhaustive")]
#[test]
fn canonical_flop_enumeration_matches_the_frozen_count() {
    use cache::presolver::scenarios::*;
    let f=canonical_flops_ordered();
    assert_eq!(f.len(),CANONICAL_FLOP_COUNT);
    assert_eq!(raw_flops().count(),22_100);
    let orbits=f.iter().map(|b|core_iso::orbit_size(b)).collect::<Vec<_>>();
    assert!(orbits.windows(2).all(|w|w[0]>=w[1]),"orbit 24 before 12 before 4");
    assert_eq!(orbits.iter().sum::<usize>(),22_100);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test presolver_queue`.
- [ ] **Step 3 (5 min): Implement the exact scenario inventory.** In the 3-bet cases `caller` denotes the original opener who calls the 3-bet; the opponent is `three_bettor`.

```rust
use proto::Position;
#[derive(Clone,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
pub struct Scenario {pub tier:u8,pub depth_bb:u16,pub opener:Position,pub caller:Position,
    pub three_bettor:Option<Position>}
impl Scenario {
    /// Deterministic, stable across runs: it keys `queue.json` items.
    pub fn id(&self)->String {
        match self.three_bettor {
            None=>format!("t{}-{}bb-{:?}-open-{:?}-call",self.tier,self.depth_bb,self.opener,self.caller),
            Some(b)=>format!("t{}-{}bb-{:?}-open-{:?}-3bet-{:?}-call",self.tier,self.depth_bb,
                self.opener,b,self.caller),
        }
    }
}
pub fn scenarios()->Vec<Scenario> {
    use Position::*;
    let lines=[(Btn,Bb,None),(Co,Bb,None),(Hj,Bb,None),(Utg,Bb,None),
        (Sb,Bb,None),(Btn,Sb,None),(Co,Btn,None),(Hj,Btn,None),
        (Btn,Btn,Some(Bb)),(Co,Co,Some(Btn)),(Btn,Btn,Some(Sb)),(Hj,Hj,Some(Btn))];
    let mut out=Vec::new();
    for (i,(opener,caller,three_bettor)) in lines.iter().cloned().enumerate() {
        out.push(Scenario{tier:if i<4 {1}else{2},depth_bb:100,opener,caller,three_bettor});
    }
    for (opener,caller,three_bettor) in lines {
        out.push(Scenario{tier:3,depth_bb:200,opener,caller,three_bettor});
    }
    out
}
```

- [ ] **Step 4 (5 min): Enumerate all C(52,3) flops through Plan 1 canonicalization, once per process.** Use zero ranges for board-only canonical representative enumeration (the canonical board is unaffected by the range tie-break); do not hash or solve with these zero ranges. Deduplicate lexicographically by Card ids, compute `orbit_size`, and sort by descending orbit then rank/suit lexicographic canonical board. At actual job preparation canonicalize again using the replayed public ranges for the stabilizer tie-break.

```rust
/// §10.4: 1,755 canonical flops represent all 22,100. Frozen so the cheap unit test never
/// pays for the enumeration (review m5).
pub const CANONICAL_FLOP_COUNT:usize=1755;

pub fn raw_flops()->impl Iterator<Item=[proto::Card;3]> {
    (0_u8..50).flat_map(|a|((a+1)..51).flat_map(move|b|
        ((b+1)..52).map(move|c|[proto::Card(a),proto::Card(b),proto::Card(c)])))
}

/// Computed at most once per process: 22,100 canonicalizations are minutes in a debug build.
pub fn canonical_flops_ordered()->&'static [Vec<proto::Card>] {
    static ORDER:std::sync::OnceLock<Vec<Vec<proto::Card>>>=std::sync::OnceLock::new();
    ORDER.get_or_init(||{
        let zero=proto::Range1326([0.0;1326]);let mut unique=std::collections::BTreeMap::new();
        for raw in raw_flops() {
            let (canonical,perm)=core_iso::canonicalize(&raw,&[&zero,&zero]);
            let mut cards=raw.iter().map(|c|core_iso::apply(&perm,*c)).collect::<Vec<_>>();
            cards.sort_by_key(|c|c.0);
            let key=cards.iter().map(|c|c.0).collect::<Vec<_>>();
            unique.entry(key).or_insert((core_iso::orbit_size(&canonical),cards));
        }
        let mut rows=unique.into_iter().collect::<Vec<_>>();
        rows.sort_by(|(a,(oa,_)),(b,(ob,_))|ob.cmp(oa).then(a.cmp(b)));
        rows.into_iter().map(|(_,(_,cards))|cards).collect()
    }).as_slice()
}
```

Add `exhaustive = []` to `crates/cache/Cargo.toml`'s `[features]`. The task iteration order is `(tier, flop_order, scenario_order)`, not all 1,755 boards for one scenario: all four tier-1 scenarios occur before the queue moves to the second board. Never use `flop_full_v1`.

- [ ] **Step 5 (3 min): Run green:** `cargo test -p cache --test presolver_queue`; `cargo test -p cache --features exhaustive --test presolver_queue`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): enumerate required presolver scenarios and flop order' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 14: Persist queue identities, cursor and verified completion

**Files:** Create `crates/cache/src/presolver/queue.rs`; modify `crates/cache/src/presolver/mod.rs`, `crates/cache/tests/presolver_queue.rs`.

**Interfaces:** Consumes `Scenario`, `Rational`, `KeyFields::scenario_identity`, `storage::write_atomic`. Produces `TaskStatus::{Pending,Done,Failed{n:u8}}`, `QueueFile`, `QueueItem`, `Queue::{open(PathBuf)->Result<Queue,CacheError>, save(&self)->Result<(),CacheError>, next_pending(&self,now_ms:u64)->Option<QueueItem>, advance_cursor(&mut self), reconcile(&mut self,&dyn Fn(&QueueItem)->bool), record_failure(&mut self,id:&str,now_ms:u64,error:String), record_launch(&mut self,id:&str), record_done(&mut self,id:&str), record_cancel(&mut self,id:&str), status_counts(&self)->(u32,u32,u32), tier_counts(&self)->([u32;3],[u32;3])}`, `reconcile_status`, `retry_delay`, `queue_path`, `save_queue`. The normalized identity is the key without the bucket plus the exact scenario SPR, not a raw hand id or board index.

- [ ] **Step 1 (4 min): Write restart and done-revalidation tests.**

```rust
#[test]
fn presolver_done_requires_a_valid_entry() {
    use cache::presolver::queue::TaskStatus;
    let mut status=TaskStatus::Done;
    cache::presolver::queue::reconcile_status(&mut status,false);
    assert!(matches!(status,TaskStatus::Pending));
    cache::presolver::queue::reconcile_status(&mut status,true);
    assert!(matches!(status,TaskStatus::Done));
}

#[test]
fn queue_reopens_at_the_saved_cursor_without_repeating_done_work() {
    let dir=std::env::temp_dir().join(format!("pokerai-queue-{}",std::process::id()));
    let _=std::fs::remove_dir_all(&dir);std::fs::create_dir_all(&dir).unwrap();
    let mut q=cache::presolver::queue::Queue::open(dir.clone()).unwrap();
    let first=q.next_pending(0).unwrap();
    q.record_launch(&first.identity_hex());q.record_done(&first.identity_hex());
    q.advance_cursor();q.save().unwrap();
    let mut reopened=cache::presolver::queue::Queue::open(dir.clone()).unwrap();
    let next=reopened.next_pending(0).unwrap();
    assert_ne!(next.identity_hex(),first.identity_hex());
    // a missing cache cell demotes Done back to Pending on reconciliation
    reopened.reconcile(&|_item|false);
    assert!(reopened.next_pending(0).is_some());
    let (pending,done,failed)=reopened.status_counts();
    assert_eq!((done,failed),(0,0));assert!(pending>0);
    std::fs::remove_dir_all(&dir).unwrap();
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache presolver_done_requires_a_valid_entry`.
- [ ] **Step 3 (5 min): Define the durable schema and transitions.**

```rust
#[derive(Clone,Debug,PartialEq,Eq,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub enum TaskStatus {Pending,Done,Failed{n:u8}}
#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub struct QueueItem {
    pub identity:[u8;32],pub scenario:super::scenarios::Scenario,pub board:Vec<proto::Card>,
    pub spr:crate::key::Rational,pub status:TaskStatus,pub retry_after_unix_ms:u64,
    pub attempts:u8,pub last_error:Option<String>,
}
impl QueueItem {
    pub fn identity_hex(&self)->String {
        self.identity.iter().map(|b|format!("{b:02x}")).collect()
    }
}
#[derive(serde::Serialize,serde::Deserialize)]
pub struct QueueFile {
    pub version:u16,pub cursor:[usize;3],pub paused:bool,
    pub items:std::collections::BTreeMap<String,QueueItem>,
}
pub fn reconcile_status(status:&mut TaskStatus,valid:bool) {
    if valid {*status=TaskStatus::Done;}
    else if matches!(status,TaskStatus::Done) {*status=TaskStatus::Pending;}
}
pub fn retry_delay(attempts:u8)->Option<std::time::Duration> {
    (attempts<=3).then_some(std::time::Duration::from_secs(30))
}
pub fn queue_path(cache_root:&std::path::Path)->std::path::PathBuf {cache_root.join("queue.json")}
pub fn save_queue(path:&std::path::Path,file:&QueueFile)->Result<(),crate::CacheError> {
    let bytes=serde_json::to_vec_pretty(file)?;
    if bytes.len()>64*1024*1024 {return Err(crate::CacheError::Invalid("queue size"));}
    crate::storage::write_atomic(path,&bytes)
}
```

`QueueFile` version 1 is independent of cache `schema_version` 3. The cursor is the next `(tier_index, flop_index, scenario_index)`; completed and failed items before the cursor are revisited for invalidation or an eligible retry. The initial attempt plus three retries gives a maximum of four attempts; a cancellation for live work returns the item to Pending without consuming a retry. `failed{n}` records the actual failed attempt count and stays failed after the fourth failure. The 30-second backoff uses monotonic time in-process; a persisted UTC deadline is clamped to `0..30 s` on restart so a clock jump cannot stall the queue.

- [ ] **Step 4 (5 min): Implement the durable `Queue` itself.** (Review M6: the whole API is code, not prose.)

```rust
pub struct Queue {path:std::path::PathBuf,file:QueueFile}

impl Queue {
    /// Bounded read, explicit version, validated cursor/board/spr/identity, no duplicate keys.
    /// A corrupt or absent file is rebuilt deterministically from `scenarios()` and
    /// `canonical_flops_ordered()`; existing valid cache cells still establish completion
    /// through `reconcile`.
    pub fn open(cache_root:std::path::PathBuf)->Result<Queue,crate::CacheError> {
        let path=queue_path(&cache_root);
        let loaded=(||->Option<QueueFile>{
            let meta=std::fs::metadata(&path).ok()?;
            if meta.len()>64*1024*1024 {return None;}
            let file:QueueFile=serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            if file.version!=1 {return None;}
            let tiers=[1_u8,2,3];
            if file.cursor[0]>=tiers.len()||file.cursor[1]>super::scenarios::CANONICAL_FLOP_COUNT
                ||file.cursor[2]>super::scenarios::scenarios().len() {return None;}
            for (key,item) in &file.items {
                if *key!=item.identity_hex() {return None;}
                if item.board.len()!=3||item.spr.den==0 {return None;}
                if !(1..=3).contains(&item.scenario.tier) {return None;}
            }
            Some(file)
        })();
        let file=match loaded {Some(f)=>f,None=>Self::rebuild()};
        Ok(Queue{path,file})
    }

    /// Deterministic rebuild in `(tier, flop, scenario)` order. Boards come from Task 13's
    /// cached canonical order, so the same install always produces the same item set.
    fn rebuild()->QueueFile {
        let mut items=std::collections::BTreeMap::new();
        for item in Self::all_items() {items.insert(item.identity_hex(),item);}
        QueueFile{version:1,cursor:[0,0,0],paused:false,items}
    }

    fn all_items()->Vec<QueueItem> {
        let scenarios=super::scenarios::scenarios();
        let boards=super::scenarios::canonical_flops_ordered();
        let mut out=Vec::new();
        for tier in 1_u8..=3 {
            for board in boards {
                for scenario in scenarios.iter().filter(|s|s.tier==tier) {
                    // the exact SPR is filled at preparation time; the stored value is the
                    // scenario's nominal depth ratio and is part of the identity.
                    let spr=crate::key::Rational::new(scenario.depth_bb as u64,1)
                        .expect("nonzero denominator");
                    let identity=Self::identity_of(scenario,board,spr);
                    out.push(QueueItem{identity,scenario:scenario.clone(),board:board.clone(),
                        spr,status:TaskStatus::Pending,retry_after_unix_ms:0,attempts:0,
                        last_error:None});
                }
            }
        }
        out
    }

    /// The normalized game identity: the cache key without `spr_bucket`, plus the exact
    /// scenario SPR (§10.5). Versions, range hashes, model, tree and rake are folded in by
    /// `KeyFields::scenario_identity`; the board and scenario id keep entries separable.
    fn identity_of(scenario:&super::scenarios::Scenario,board:&[proto::Card],
        spr:crate::key::Rational)->[u8;32] {
        use sha2::Digest;
        let mut hasher=sha2::Sha256::new();
        hasher.update(scenario.id().as_bytes());
        hasher.update(board.iter().map(|c|c.0).collect::<Vec<_>>());
        hasher.update(spr.num.to_le_bytes());hasher.update(spr.den.to_le_bytes());
        hasher.finalize().into()
    }

    pub fn paused(&self)->bool {self.file.paused}
    pub fn set_paused(&mut self,paused:bool) {self.file.paused=paused;}
    pub fn cursor(&self)->[usize;3] {self.file.cursor}
    pub fn save(&self)->Result<(),crate::CacheError> {save_queue(&self.path,&self.file)}

    /// The next launchable item at or after the cursor, honouring the retry deadline.
    pub fn next_pending(&self,now_ms:u64)->Option<QueueItem> {
        self.file.items.values()
            .filter(|i|match &i.status {
                TaskStatus::Pending=>true,
                TaskStatus::Failed{n}=>*n<4&&i.retry_after_unix_ms<=now_ms,
                TaskStatus::Done=>false})
            .min_by_key(|i|(i.scenario.tier,i.identity_hex()))
            .cloned()
    }

    pub fn advance_cursor(&mut self) {
        let scenarios=super::scenarios::scenarios().len();
        let c=&mut self.file.cursor;
        c[2]+=1;
        if c[2]>=scenarios {c[2]=0;c[1]+=1;}
        if c[1]>=super::scenarios::CANONICAL_FLOP_COUNT {c[1]=0;c[0]=(c[0]+1).min(2);}
    }

    /// Persisted before launch: status stays Pending and `attempts` increments, so a restart
    /// resumes the work instead of losing it. In-flight state is runtime-only.
    pub fn record_launch(&mut self,id:&str) {
        if let Some(item)=self.file.items.get_mut(id) {item.attempts=item.attempts.saturating_add(1);}
    }
    pub fn record_done(&mut self,id:&str) {
        if let Some(item)=self.file.items.get_mut(id) {
            item.status=TaskStatus::Done;item.last_error=None;
        }
    }
    /// A live-work cancellation returns the item to Pending and refunds its attempt.
    pub fn record_cancel(&mut self,id:&str) {
        if let Some(item)=self.file.items.get_mut(id) {
            item.status=TaskStatus::Pending;item.attempts=item.attempts.saturating_sub(1);
        }
    }
    pub fn record_failure(&mut self,id:&str,now_ms:u64,error:String) {
        if let Some(item)=self.file.items.get_mut(id) {
            let n=item.attempts.min(4);
            item.status=TaskStatus::Failed{n};
            item.last_error=Some(error);
            item.retry_after_unix_ms=match retry_delay(n) {
                Some(d)=>now_ms.saturating_add(d.as_millis() as u64),
                None=>u64::MAX,          // the fourth failure is terminal
            };
        }
    }
    /// `valid` is "a validated at-target entry for this exact identity exists on disk".
    pub fn reconcile(&mut self,valid:&dyn Fn(&QueueItem)->bool) {
        for item in self.file.items.values_mut() {
            let ok=valid(item);
            let mut status=item.status.clone();
            reconcile_status(&mut status,ok);
            if !ok&&matches!(item.status,TaskStatus::Failed{..}) {continue;}
            item.status=status;
        }
    }
    pub fn status_counts(&self)->(u32,u32,u32) {
        let mut counts=(0,0,0);
        for item in self.file.items.values() {
            match item.status {TaskStatus::Pending=>counts.0+=1,TaskStatus::Done=>counts.1+=1,
                TaskStatus::Failed{..}=>counts.2+=1}
        }
        counts
    }
    pub fn tier_counts(&self)->([u32;3],[u32;3]) {
        let (mut done,mut total)=([0;3],[0;3]);
        for item in self.file.items.values() {
            let t=(item.scenario.tier as usize).saturating_sub(1).min(2);
            total[t]+=1;
            if matches!(item.status,TaskStatus::Done) {done[t]+=1;}
        }
        (done,total)
    }
}
```

`retry_delay` returns `None` exactly once the fourth attempt has failed, which is why that arm parks the item at `u64::MAX`. `queue.json` is written through Task 6's `write_atomic`. A successful worker terminal is not sufficient for `Done`: validate the solution, await the writer receipt, then read and validate the exact normalized scenario identity at raw `<= 0.005` before calling `record_done`. A read-only cache or full disk leaves the item Pending or Failed; corrupt, evicted or stale-range entries move Done back to Pending on startup and on periodic reconciliation.

- [ ] **Step 5 (4 min): Test persistence roundtrip.** Save the cursor after tier 1 board 0 scenario 2, reopen, verify the next job is scenario 2 and that completed earlier entries are not repeated. Delete its cache cell then reconcile: Pending. Replace the source bundle hash: a new identity, and old entries stay isolated. Simulate a crash before and after the entry rename and before the queue save: neither falsely marks Done. Failed attempts 1/2/3 retry at 30 s; failure 4 does not. Cancellation and restart do not burn retries.

```rust
#[test]
fn three_retries_follow_initial_attempt() {
    use cache::presolver::queue::retry_delay;
    assert_eq!(retry_delay(1).unwrap().as_secs(),30);
    assert!(retry_delay(3).is_some());assert!(retry_delay(4).is_none());
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache --test presolver_queue`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): persist resumable verified presolver queue' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 15: Schedule idle jobs and cancel them for live admission

**Files:** Create `crates/cache/src/presolver/scheduler.rs`; modify `crates/cache/src/presolver/{mod,queue}.rs`, `crates/cache/tests/presolver_queue.rs`, `crates/cache/Cargo.toml`.

**Interfaces:** Consumes `Queue`, `QueueItem`, `Scenario`, `CacheEntry`, `SolveInput`. Produces `PreparedJob`, `JobPoll`, `PresolveExecutor` (defined here, distinct from `WorkerLink`), `PresolverStatus` (**`Serialize`/`Deserialize`/`Debug`/`Clone`/`Default`/`PartialEq`**, cross-plan Or7/M9), `PresolverCommand`, `Presolver::{start(dir:PathBuf,executor:Box<dyn PresolveExecutor>)->Presolver, pause(&self), resume(&self), status(&self)->PresolverStatus, notify_hand(&self,bool), notify_live_request(&self), shutdown(&self)}`, `eligible`, `next_action`, **`cache::presolver::remaining_seconds(pending:u32,measured_p50:Option<f64>)->Option<f64>`** and the parent re-export **`pub use scheduler::PresolverStatus;` in `cache::presolver`**, so `cache::presolver::PresolverStatus` is the path Plan 5 and Task 16 name. `remaining_seconds` is defined here, before the scheduler calls it; Task 16 only re-exports it. Cache never imports engine, replay or preflop; the executor is a downward callback supplied by Task 16.

- [ ] **Step 1 (4 min): Write the 30 s gate and pause tests.**

```rust
#[test]
fn idle_requires_no_hand_and_thirty_seconds() {
    use cache::presolver::scheduler::eligible;
    assert!(!eligible(false,false,29_999,0));assert!(eligible(false,false,30_000,0));
    assert!(!eligible(true,false,60_000,0));assert!(!eligible(false,true,60_000,0));
    assert!(!eligible(false,false,30_000,30_000));
}

#[test]
fn presolver_status_round_trips_as_json() {
    // cross-plan Or7: plan 5 renders this struct directly, so it must serialize.
    let status=cache::presolver::scheduler::PresolverStatus{paused:true,
        running:Some("t1-100bb-Btn-open-Bb-call".into()),pending:7,done:2,failed:1,
        tier_done:[2,0,0],tier_total:[7020,14040,21060],measured_p50_s:Some(27.0),
        estimated_remaining_s:Some(189.0),scenario_hits:vec![("t1-100bb-Btn-open-Bb-call".into(),3,10)]};
    let text=serde_json::to_string(&status).unwrap();
    let back:cache::presolver::scheduler::PresolverStatus=serde_json::from_str(&text).unwrap();
    assert_eq!(status,back);
    assert!(text.contains("\"tier_total\":[7020,14040,21060]"));
    // the parent path Plan 5 and Engine::presolver_status name resolves to the same type
    let parent:cache::presolver::PresolverStatus=status.clone();
    assert_eq!(parent,status);
    assert_eq!(cache::presolver::remaining_seconds(7,Some(27.0)),Some(189.0));
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache idle_requires_no_hand_and_thirty_seconds`.
- [ ] **Step 3 (5 min): Define the executor/handle contracts and the scheduler predicate.**

```rust
pub struct PreparedJob {pub item:queue::QueueItem,pub input:proto::SolveInput,
    pub rake:proto::Rake,pub bb_chips:u32}
pub enum JobPoll {Running,Complete(crate::entry::CacheEntry),Cancelled,Failed(String)}
pub trait PresolveExecutor:Send+'static {
    fn prepare(&mut self,scenario:&scenarios::Scenario,board:&[proto::Card])->Result<PreparedJob,String>;
    fn submit(&mut self,job:PreparedJob)->Result<u64,String>;
    fn poll(&mut self,id:u64)->JobPoll;
    fn cancel(&mut self,id:u64);
    /// Blocks only inside the presolver thread: validates and durably stores the entry, then
    /// re-reads it and checks raw accuracy at target (§10.5 "done only with a valid entry").
    fn store_and_verify(&mut self,item:&queue::QueueItem,entry:&crate::entry::CacheEntry)->bool;
    fn entry_exists_at_target(&mut self,item:&queue::QueueItem)->bool;
    fn now_ms(&self)->u64;
    fn measured_p50_s(&self)->Option<f64>;
    fn scenario_hits(&self)->Vec<(String,u64,u64)>;
}
#[derive(Clone,Debug,Default,PartialEq,serde::Serialize,serde::Deserialize)]
#[cfg_attr(feature="typescript",derive(ts_rs::TS),ts(export))]
pub struct PresolverStatus {
    pub paused:bool,pub running:Option<String>,pub pending:u32,pub done:u32,pub failed:u32,
    pub tier_done:[u32;3],pub tier_total:[u32;3],pub measured_p50_s:Option<f64>,
    pub estimated_remaining_s:Option<f64>,pub scenario_hits:Vec<(String,u64,u64)>,
}
/// §10.5: no hand in progress, not paused, and no request for 30 s.
pub fn eligible(hand_in_progress:bool,paused:bool,now:u64,last_activity:u64)->bool {
    !hand_in_progress&&!paused&&now.saturating_sub(last_activity)>=30_000
}
pub enum ScheduleAction {Wait,Cancel(u64),Launch}
pub fn next_action(active:Option<u64>,live:bool,can_start:bool)->ScheduleAction {
    match(active,live,can_start) {
        (Some(id),true,_)=>ScheduleAction::Cancel(id),
        (None,false,true)=>ScheduleAction::Launch,_=>ScheduleAction::Wait,
    }
}
```

```rust
// crates/cache/src/presolver/mod.rs — the parent module of this task
pub mod queue;
pub mod scenarios;
pub mod scheduler;

/// Plan 5 and `Engine::presolver_status` name the parent path, so the status type is
/// re-exported here; `scheduler::PresolverStatus` remains its single definition.
pub use scheduler::PresolverStatus;

/// Defined here, before `scheduler` calls it (the scheduler's status publication uses
/// `crate::presolver::remaining_seconds`). `engine::log` re-exports it in Task 16.
pub fn remaining_seconds(pending:u32,measured_p50:Option<f64>)->Option<f64> {
    measured_p50.filter(|x|x.is_finite()&&*x>0.0).map(|x|pending as f64*x)
}

#[cfg(test)]
mod tests {
    #[test]
    fn remaining_seconds_needs_a_finite_positive_measurement() {
        assert_eq!(super::remaining_seconds(7,Some(27.0)),Some(189.0));
        assert_eq!(super::remaining_seconds(7,None),None);
        assert_eq!(super::remaining_seconds(7,Some(0.0)),None);
        assert_eq!(super::remaining_seconds(7,Some(f64::NAN)),None);
    }
}
```

- [ ] **Step 4 (5 min): Implement the named thread, the command channel and the cancellation rule.** (Review M7: the scheduler is code, not prose.)

```rust
pub enum PresolverCommand {HandInProgress(bool),LiveRequest,Pause,Resume,Shutdown}

pub struct Presolver {
    commands:std::sync::mpsc::SyncSender<PresolverCommand>,
    status:std::sync::Arc<std::sync::RwLock<PresolverStatus>>,
    handle:std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Set synchronously by `notify_live_request` at engine admission, so a live job is
    /// protected even before the presolver thread drains its channel (§7 admission).
    live:std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Presolver {
    pub fn start(dir:std::path::PathBuf,mut executor:Box<dyn PresolveExecutor>)->Presolver {
        use std::sync::atomic::Ordering;
        let (tx,rx)=std::sync::mpsc::sync_channel::<PresolverCommand>(32);
        let status=std::sync::Arc::new(std::sync::RwLock::new(PresolverStatus::default()));
        let live=std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (thread_status,thread_live)=(status.clone(),live.clone());
        let handle=std::thread::Builder::new().name("presolver".into()).spawn(move|| {
            let Ok(mut queue)=queue::Queue::open(dir) else {return};
            queue.reconcile(&|item|executor.entry_exists_at_target_ref(item));
            let _=queue.save();
            let mut hand_in_progress=false;
            let mut last_activity=executor.now_ms();
            let mut active:Option<(u64,String)>=None;
            loop {
                // 1. drain commands without holding the status lock
                match rx.recv_timeout(std::time::Duration::from_millis(250)) {
                    Ok(PresolverCommand::Shutdown)=>{
                        if let Some((id,job))=active.take() {executor.cancel(id);queue.record_cancel(&job);}
                        let _=queue.save();break;
                    }
                    Ok(PresolverCommand::Pause)=>{queue.set_paused(true);let _=queue.save();}
                    Ok(PresolverCommand::Resume)=>{queue.set_paused(false);let _=queue.save();
                        last_activity=executor.now_ms();}
                    Ok(PresolverCommand::HandInProgress(v))=>{hand_in_progress=v;
                        last_activity=executor.now_ms();}
                    Ok(PresolverCommand::LiveRequest)=>{last_activity=executor.now_ms();}
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)=>{}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected)=>{
                        if let Some((id,job))=active.take() {executor.cancel(id);queue.record_cancel(&job);}
                        let _=queue.save();break;
                    }
                }
                let now=executor.now_ms();
                let live_now=thread_live.swap(false,Ordering::SeqCst)||hand_in_progress;
                if live_now {last_activity=now;}
                let can_start=eligible(hand_in_progress,queue.paused(),now,last_activity);
                // 2. one scheduling decision
                match next_action(active.as_ref().map(|(id,_)|*id),live_now,can_start) {
                    ScheduleAction::Cancel(id)=>{
                        executor.cancel(id);
                        // §7: wait for the background terminal or a confirmed exit before the
                        // live job may proceed; poll is non-blocking so the thread stays live.
                        loop {
                            match executor.poll(id) {
                                JobPoll::Running=>std::thread::yield_now(),
                                _=>break,
                            }
                        }
                        if let Some((_,job))=active.take() {queue.record_cancel(&job);}
                        let _=queue.save();
                    }
                    ScheduleAction::Launch=>{
                        if let Some(item)=queue.next_pending(now) {
                            let key=item.identity_hex();
                            match executor.prepare(&item.scenario,&item.board)
                                .and_then(|job|executor.submit(job)) {
                                Ok(id)=>{queue.record_launch(&key);let _=queue.save();
                                    active=Some((id,key));}
                                Err(error)=>{queue.record_launch(&key);
                                    queue.record_failure(&key,now,error);let _=queue.save();}
                            }
                            queue.advance_cursor();
                        }
                    }
                    ScheduleAction::Wait=>{}
                }
                // 3. progress the running job
                if let Some((id,key))=active.clone() {
                    match executor.poll(id) {
                        JobPoll::Running=>{}
                        JobPoll::Complete(entry)=>{
                            if executor.store_and_verify(&queue_item(&queue,&key),&entry) {
                                queue.record_done(&key);
                            } else {
                                queue.record_failure(&key,now,"entry not durable at target".into());
                            }
                            active=None;let _=queue.save();
                        }
                        JobPoll::Cancelled=>{queue.record_cancel(&key);active=None;let _=queue.save();}
                        JobPoll::Failed(error)=>{queue.record_failure(&key,now,error);
                            active=None;let _=queue.save();}
                    }
                }
                // 4. publish status; the lock is never held across I/O or an executor call
                let (pending,done,failed)=queue.status_counts();
                let (tier_done,tier_total)=queue.tier_counts();
                let measured=executor.measured_p50_s();
                let mut guard=thread_status.write().expect("status");
                *guard=PresolverStatus{paused:queue.paused(),
                    running:active.as_ref().map(|(_,key)|key.clone()),pending,done,failed,
                    tier_done,tier_total,measured_p50_s:measured,
                    estimated_remaining_s:crate::presolver::remaining_seconds(pending,measured),
                    scenario_hits:executor.scenario_hits()};
            }
        }).ok();
        Presolver{commands:tx,status,handle:std::sync::Mutex::new(handle),live}
    }
    pub fn pause(&self) {let _=self.commands.try_send(PresolverCommand::Pause);}
    pub fn resume(&self) {let _=self.commands.try_send(PresolverCommand::Resume);}
    pub fn status(&self)->PresolverStatus {self.status.read().expect("status").clone()}
    pub fn notify_hand(&self,in_progress:bool) {
        if in_progress {self.live.store(true,std::sync::atomic::Ordering::SeqCst);}
        let _=self.commands.try_send(PresolverCommand::HandInProgress(in_progress));
    }
    /// Synchronous flag plus an asynchronous event: admission never waits for the channel.
    pub fn notify_live_request(&self) {
        self.live.store(true,std::sync::atomic::Ordering::SeqCst);
        let _=self.commands.try_send(PresolverCommand::LiveRequest);
    }
    /// Requests cancellation and persists the queue. The UI command path never joins the
    /// thread; `join_for_shutdown` is called only from `Engine::shutdown`.
    pub fn shutdown(&self) {let _=self.commands.try_send(PresolverCommand::Shutdown);}
    pub fn join_for_shutdown(&self) {
        if let Some(handle)=self.handle.lock().expect("presolver handle").take() {let _=handle.join();}
    }
}

fn queue_item(queue:&queue::Queue,key:&str)->queue::QueueItem {
    queue.item(key).cloned().expect("active item is in the queue")
}
```

Add `Queue::item(&self,&str)->Option<&QueueItem>` and an `entry_exists_at_target_ref(&self,&QueueItem)->bool` shim on `PresolveExecutor` implementations (an immutable wrapper over `entry_exists_at_target`, used inside the `reconcile` closure). Jobs use `600000 ms` background deadline, target 50 and extraction 600 through the executor. The engine's existing control thread handles cancellation at iteration boundaries, `ack <= 50 ms` and kill after 1.5 s; a background `ack` never releases admission, and a live job proceeds only after a background terminal or a confirmed exit. Background work never emits user-facing decision events or snapshots and cannot overwrite retained live evidence. A pause lets an already-running job finish unless a live request, a hand or a shutdown cancels it; a resume preserves the cursor and waits for idle.

- [ ] **Step 5 (4 min): Test restart, cancellation and durable completion with a fake executor.** Implement the trait with a `VecDeque<JobPoll>`, a cancel counter, a settable fake clock and a boolean durable-entry flag. Assert no submission before 30 s, exactly one while running, a live cancellation count of 1, Pending when the worker completes but the durable flag is false, and Done only after it is true. After pause/restart/resume the cursor and failed counters match. All polling and cancellation use fake clock advances; no test sleeps 30 s.

```rust
#[test]
fn live_request_cancels_background_before_new_launch() {
    use cache::presolver::scheduler::{next_action,ScheduleAction};
    assert!(matches!(next_action(Some(7),true,true),ScheduleAction::Cancel(7)));
    assert!(matches!(next_action(Some(7),false,true),ScheduleAction::Wait));
    assert!(matches!(next_action(None,false,true),ScheduleAction::Launch));
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache --test presolver_queue`; `cargo test -p cache --features typescript`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): schedule idle presolves with live cancellation' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 16: Prepare presolves by chart replay, own the Presolver and report coverage

**Files:** Create `crates/engine/src/presolve.rs`, `crates/engine/tests/presolver_engine.rs`; modify `crates/engine/src/{engine,core,serve,log}.rs`.

**Interfaces:** Implements `cache::presolver::scheduler::PresolveExecutor` using the existing `Engine`/`WorkerLink`; Consumes `PreflopStore::query`, `replay(ReplayInput)`, `build_effective_tree`, `core_model::street_root`, the last-used `HandConfig`, `cache_bridge::entry_from_solution`, `Cache::store_tracked`, and Task 15's `cache::presolver::{PresolverStatus, remaining_seconds}`. Produces `presolve::{BACKGROUND_DEADLINE_MS, BACKGROUND_TARGET_BP, BACKGROUND_TEMPLATE, EngineExecutor, scenario_hand(cfg:&HandConfig,scenario:&Scenario,board:&[Card],store:&PreflopStore)->Result<HandState,UnsupportedReason>, scenario_hit_rates(records:&[DecisionRecord])->Vec<(String,u64,u64)>}`; `log::hit_rate` plus the re-export `pub use cache::presolver::remaining_seconds;` in `engine::log`; the new field `Engine.presolver: Option<Arc<cache::presolver::scheduler::Presolver>>`; and the engine surface Plan 5 renders: `Engine::{presolver_status(&self)->cache::presolver::PresolverStatus, presolver_pause(&mut self), presolver_resume(&mut self)}` (cross-plan M9).

**Presolver ownership contract (F09/F10; binding).** Task 7 adds only `EngineCore.cache` and `with_cache`, initializing `cache` to `Cache::disabled()`; it does not mention `Presolver`. Task 15 defines `remaining_seconds` in `cache::presolver` before `scheduler` uses it and adds `pub use scheduler::PresolverStatus` to `cache::presolver`. This task adds `Engine.presolver: Option<Arc<cache::presolver::scheduler::Presolver>>`, initialized to `None` by `with_core` and populated once in production startup. Status, pause, resume, hand notification and live notification use this independent handle directly and **never acquire `EngineCore`'s mutex**. `EngineExecutor` holds a command/completion endpoint to the existing single worker owner, not an `Arc<Mutex<EngineCore>>` used by UI control calls. That owner serializes live and background send/receive, confirms background terminal or exit before sending live work, and observes the presolver cancellation flag while background work is running. A background job has its own bookkeeping and never requires an active live `DecisionIdentity` or emits user recommendation events/snapshots. No second `ProcessWorker` is spawned. Shutdown signals background cancellation before waiting for `engine-main`, joins the presolver, stops cache threads, and then shuts down/kills the worker; repeated shutdown is a no-op.

- [ ] **Step 1 (5 min): Write chart-replay equivalence and admission tests.** Construct tier 1 BTN/BB 100 with last config 1/2 and the same legal prefix manually; compare both 1326 vectors, range hashes, root pot/stacks, tree signature and reasons. Change hero cards: identical job identity. Change the chart bundle: new hashes and identity. At live admission during Solving, assert the background cancel precedes the live send and that no admission is released at `ack`.

```rust
fn replay_root_ranges(state:&proto::HandState,store:&core_preflop::PreflopStore)
    ->core_replay::ReplayOutput {
    core_replay::replay(core_replay::ReplayInput{cfg:&state.config,state,store,snapshots:&[]})
}
#[test]
fn presolver_ranges_from_chart_replay() {
    let (store,warnings)=core_preflop::PreflopStore::open(std::path::Path::new("../../fixtures/charts"));
    assert!(warnings.is_empty(),"{warnings:?}");
    let cfg=proto::HandConfig{config_revision:1,sb_chips:50,bb_chips:100,straddle:None,
        rake:proto::Rake::PotRake{rate:0.05,cap_mchips:50000,no_flop_no_drop:true},chip_label:"chip".into()};
    let scenario=cache::presolver::scenarios::Scenario{tier:1,depth_bb:100,
        opener:proto::Position::Btn,caller:proto::Position::Bb,three_bettor:None};
    let board=[proto::Card(46),proto::Card(21),proto::Card(0)];
    let state=engine::presolve::scenario_hand(&cfg,&scenario,&board,&store).unwrap();
    let root=core_model::street_root(&state).unwrap();assert_eq!(root.pot_root,550);
    let expected=replay_root_ranges(&state,&store);
    assert!(expected.ranges[2].as_ref().unwrap().0.iter().sum::<f32>()<1176.0);
    let mut changed=state.clone();changed.hero_cards=Some([proto::Card(50),proto::Card(49)]);
    let actual=replay_root_ranges(&changed,&store);
    assert_eq!(actual.ranges,expected.ranges);
    assert!(actual.reasons.iter().any(|r|matches!(r,proto::ApproxReason::ChartRounded)));
}

#[test]
fn engine_owns_the_presolver_lifecycle() {
    let mut engine=support::engine_with_fake_worker();
    assert_eq!(engine.presolver_status().running,None);
    engine.presolver_pause();
    assert!(support::eventually(||engine.presolver_status().paused));
    engine.presolver_resume();
    assert!(support::eventually(||!engine.presolver_status().paused));
    // begin_hand and recommend both stop background scheduling
    let _=engine.begin_hand(support::begin_hand_request());
    assert!(support::eventually(||engine.presolver_status().running.is_none()));
    engine.shutdown();
}

/// F09: the presolver handle is independent of the worker owner, so a running solve cannot
/// block status, pause or the live-admission notification.
#[test]
fn presolver_commands_never_wait_for_a_running_solve() {
    let mut engine=support::engine_with_worker_stuck_in_solve();   // never returns a terminal
    let started=std::time::Instant::now();
    let _=engine.presolver_status();
    engine.presolver_pause();
    engine.presolver_resume();
    assert!(started.elapsed()<std::time::Duration::from_millis(200),
        "UI control calls must not acquire EngineCore's mutex");
    // live admission sets the synchronous flag and cancellation precedes live dispatch
    let order=support::recommend_and_record_worker_order(&mut engine);
    let cancel=order.iter().position(|e|e=="background_cancel").unwrap();
    let live=order.iter().position(|e|e=="live_send").unwrap();
    assert!(cancel<live,"background cancellation precedes the live send");
    engine.shutdown();
}
```

Add `support::{engine_with_worker_stuck_in_solve, recommend_and_record_worker_order}` to the harness; both are built on Plan 2 Task 19's `FakeWorker`/`FakeClock`.

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --features testing --test presolver_engine`.
- [ ] **Step 3 (5 min): Implement scenario prefixes using actual chart sizes.** Begin a six-seat synthetic hand with stacks `depth_bb * bb_chips`, the last-used blinds/rake/straddle and button Seat(0); use core-model legal order. Query each prefix through the active highest-precedence bundle; choose its resolved open/3-bet size, explicit folds by unused seats, and a Call by the named defender or opener. No generic percentage ranges and no guessed missing nodes. Set the flop through core-model, call `replay` with no postflop snapshots, use its public root marginals and accumulated reasons, derive the root and materialize the fast tree. If a needed prefix is absent, record the failure reason and keep it visible; replay's specified missing-prior-node fallback is allowed only when deliberately frozen in the source provenance, never silently invented.

```rust
let replay=core_replay::replay(core_replay::ReplayInput{
    cfg:&state.config,state:&state,store:&store,snapshots:&[]});
let root=core_model::street_root(&state).map_err(|e|format!("{e:?}"))?;
let ranges=[replay.ranges[root.oop.0 as usize].clone().ok_or("missing OOP range")?,
            replay.ranges[root.ip.0 as usize].clone().ok_or("missing IP range")?];
assert!(ranges.iter().all(|r|r.0.iter().any(|w|*w>0.0)));
```

Use checked multiplication and model legality for the source sizes, and freeze the actual folds and raises in the queue scenario provenance. The synthetic hand has no hero-card effect. Prepare against a snapshot of the config and bundle revisions; a change cancels outstanding background preparation and invalidates its queue generation, while older cache entries stay addressable only by their original hashes.

- [ ] **Step 4 (5 min): Submit through the single engine worker owner and install the handle.** Background `Solve` uses the input root P/stacks, public ranges, fast tree, empty history, target 50, deadline 600000, extraction 600, memory 10 GiB and `background: true` (cross-plan R2: `SolvePlan.background` is a real field that Plan 2 simply never sets to `true`). The worker sets BELOW_NORMAL priority and applies the same 2 GiB f32 / i16 rule, returning the mode in the payload. No parallel worker is spawned. Completion invokes `entry_from_solution` with the replay reasons and exact job versions, stores through `Cache::store_tracked`, awaits the receipt, re-reads the raw accuracy and only then marks Done.

```rust
// crates/engine/src/presolve.rs
pub const BACKGROUND_DEADLINE_MS:u32=600_000;
pub const BACKGROUND_TARGET_BP:u16=50;
/// One definition, shared with the flop lookup order of Task 10.
pub use crate::flop::PRESOLVER_TEMPLATE as BACKGROUND_TEMPLATE;

impl cache::presolver::scheduler::PresolveExecutor for EngineExecutor {
    fn store_and_verify(&mut self,item:&cache::presolver::queue::QueueItem,
        entry:&cache::entry::CacheEntry)->bool {
        let receipt=self.cache.store_tracked(entry);
        if !receipt.wait(std::time::Duration::from_secs(30)) {return false;}
        self.entry_exists_at_target(item)
    }
    // prepare / submit / poll / cancel / entry_exists_at_target / now_ms / measured_p50_s /
    // scenario_hits as declared in Task 15.
}
```

```rust
// crates/engine/src/engine.rs — the Presolver lifecycle (blocker B3, F09)
pub struct Engine {
    // ... Plan 2 Task 29's existing fields ...
    /// Independent of the worker owner: a UI command never waits behind a running solve.
    pub(crate) presolver: Option<std::sync::Arc<cache::presolver::scheduler::Presolver>>,
}

impl Engine {
    fn start_presolver(&mut self, cache_root: std::path::PathBuf) {
        if cache_root.as_os_str().is_empty() {return;}    // disabled cache: no pre-solver
        // The executor talks to the single worker owner through its command/completion
        // endpoint; it does not capture an Arc<Mutex<EngineCore>> for UI control calls.
        let executor=Box::new(crate::presolve::EngineExecutor::new(self.owner_endpoint()));
        self.presolver=Some(std::sync::Arc::new(
            cache::presolver::scheduler::Presolver::start(cache_root,executor)));
    }
    pub fn presolver_status(&self)->cache::presolver::PresolverStatus {
        self.presolver.as_ref().map(|p|p.status()).unwrap_or_default()
    }
    pub fn presolver_pause(&mut self) {
        if let Some(p)=&self.presolver { p.pause(); }
    }
    pub fn presolver_resume(&mut self) {
        if let Some(p)=&self.presolver { p.resume(); }
    }
    fn notify_presolver_hand(&self,in_progress:bool) {
        if let Some(p)=&self.presolver { p.notify_hand(in_progress); }
    }
}
```

`Engine::with_core` initializes `presolver: None`; `Engine::new` calls `start_presolver(paths.cache.clone())` once, after `core.cache` is open. None of these four methods locks `EngineCore`.

`begin_hand` calls `notify_presolver_hand(true)`; `finish_hand` and `abandon_hand` call `notify_presolver_hand(false)`, which restarts the idle timer. `recommend` calls `self.presolver.notify_live_request()` **before** pushing into the request slot, so the synchronous `live` flag is set at admission without touching the worker owner. `Engine::shutdown(&mut self)` (cross-plan M11) extends Plan 2 Task 29's idempotent body, in order: signal background cancellation; stop the slot and join `engine-main`; `presolver.shutdown()` then `presolver.join_for_shutdown()`; `core.cache.shutdown()` (which stops `cache-writer` and `cache-reader`); the worker `Shutdown` message and `kill()`. Plan 2's existing `stopped: bool` makes a second call a no-op, the receiver is unchanged, and no UI command path ever joins those threads.

- [ ] **Step 5 (5 min): Extend the decision record and status computation.** Log every actual decision's input record and identity, the scenario id and tier when matched, the cache result (`miss|exact|approximate|provisional`), the raw reached exploitability, the selected template and mode, street and final deadline violations, and elapsed time. The existing log stays at `%LOCALAPPDATA%\PokerAI\decisions.jsonl` with 50 MiB × 10 rotation. Match by the real preflop line, depth and config; never call a mismatching line a scenario hit. The hit-rate numerator counts at-target exact/approximate cache Finals; the denominator includes matching requests with misses and provisionals; provisionals are disclosed separately. Aggregate from the decision log, never from fabricated queue coverage.

```rust
// crates/engine/src/log.rs
pub fn hit_rate(hits:u64,requests:u64)->Option<f64> {
    (requests>0).then(||hits as f64/requests as f64)
}
/// Task 15 owns the body in `cache::presolver`; this task only re-exports it for the report.
pub use cache::presolver::remaining_seconds;
```

`remaining_seconds` is defined in `cache::presolver` by Task 15 (the scheduler calls it there) and is only re-exported by `engine::log` for the report; this task contains no second definition. Report measured time-to-target by scenario, template and mode; recompute `1755 × p50` and sum the remaining counts. Initial R8 estimates must be labelled unmeasured production estimates: 100bb `1755 × 27 s ≈ 13 h` per scenario, tier 1 `≈ 52 h`; 200bb `1755 × 52 s ≈ 25 h` per scenario; tier 2 adds `≈ 62 h`; phase-2 `flop_full_v1` `≈ 112 h` per scenario is excluded. Tier 1 starts now and continues after release; a successful scheduler test never claims the tier is complete. Completion of tiers 2 and 3 is not a release requirement.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --features testing --test presolver_engine`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): drive chart-replay presolves and measured hit rates' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 17: Freeze chart provenance, source hashes and the node inventory

Cross-plan §5 splits the original 236-line fixture task into three review-gated units; this is (a) the source and manifest lock.

**Files:** Create `tools/chart_sources.py`, `tools/tests/test_chart_sources.py`, `bench/spots/sources.json`; modify `tools/gen_fixtures.py` (a `sources` subcommand), `crates/engine/src/bench_support.rs` (created by Plan 2 Task 5, cross-plan M17).

**Interfaces:** Consumes Plan 3's inspected chart bundles `fixtures/charts/<name>.json` and `<name>.manifest.json`, and Plan 3 Task 4's acquisition record `fixtures/charts/sources.manifest.json`, whose per-depth rows are `available` or `unsupported`. Produces `chart_sources.{available_depths(repo:Path)->dict, freeze_sources(repo:Path)->dict, write_sources(repo:Path)->None}`, CLI `python tools/gen_fixtures.py sources [--check]`; `engine::bench_support::{SourceLock, load_source_lock(&Path)->Result<SourceLock,String>, verify_source_lock(&SourceLock,&Path)->Result<(),String>}`.

**Conditional depth 200.** Plan 3 marks the RangeConverter 200bb bundle `unsupported` when its publisher download could not be resolved. `freeze_sources` therefore reads `sources.manifest.json` first and emits a bundle row only for a depth whose status is `available`; an `unsupported` depth is recorded in `lock['unavailable']` with the manifest's stated reason. Task 20 then generates the 100bb halves of the six suites and marks each 200bb line `not generated: depth 200 unsupported by the acquired sources`, and Task 23's gate reports `required_matrix_complete = false` for those cells rather than passing them. Nothing substitutes another publisher, another depth or a screenshot.

- [ ] **Step 1 (4 min): Write the provenance test before generating the lock.**

```python
import json
from pathlib import Path
from chart_sources import freeze_sources

REPO = Path(__file__).resolve().parents[2]

def test_sources_cover_every_required_node():
    lock = freeze_sources(REPO)
    names = [b['name'] for b in lock['bundles']]
    # depth 200 is present only when Plan 3's acquisition record marks it `available`
    assert names[0] == 'pokercoaching_100'
    assert set(names) | {b['name'] for b in lock['unavailable']} == {
        'pokercoaching_100', 'rangeconverter_200'}
    assert lock['version'] == 1 and lock['snapshot_date'] == '2026-09-10'
    assert all(len(b['sha256']) == 64 for b in lock['bundles'])
    assert all(b['reason'] for b in lock['unavailable'])
    required = {'', 'F', 'FF', 'FFF', 'FFFF', 'FFFFF'}          # unopened folds to each opener
    for bundle in lock['bundles']:
        histories = {n for n in bundle['nodes']}
        assert required <= histories, sorted(required - histories)
    # the three synthetic absences are declared, not discovered later
    assert lock['missing'] == ['UTG-limp', 'CO-limp', 'BB-cold-call-vs-3bet']
```

- [ ] **Step 2 (2 min): Run red:** `python -m pytest tools/tests/test_chart_sources.py -q`; missing module.
- [ ] **Step 3 (5 min): Freeze source URLs, versions, sizes and the node inventory.** Use Plan 3's inspected and transcribed source bytes, not another fetch that can drift. Sources: PokerCoaching 100, `https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf`, six-page Implementable GTO snapshot 2026-09-10; RangeConverter 200, `https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem`, download page `https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash`, 13-page 500z / 50%-rounded snapshot 2026-09-10. The source version is the content sha256 plus transcription envelope version 2, not an invented publisher release. Store the final PDF URL and hash from Plan 3's audit in `sources.json`.

```python
import hashlib, json
from pathlib import Path

SNAPSHOT_DATE = '2026-09-10'
SOURCES = {
    'pokercoaching_100': {
        'url': 'https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf',
        'pages': 6, 'envelope_version': 2},
    'rangeconverter_200': {
        'url': 'https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem',
        'download': 'https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash',
        'pages': 13, 'envelope_version': 2},
}
MISSING = ['UTG-limp', 'CO-limp', 'BB-cold-call-vs-3bet']

def available_depths(repo: Path) -> dict:
    """Plan 3 Task 4's acquisition record: {'pokercoaching_100': ('available', ''), ...}."""
    record = json.loads((repo / 'fixtures/charts/sources.manifest.json').read_text())
    return {row['name']: (row['status'], row.get('reason', '')) for row in record['depths']}

def freeze_sources(repo: Path) -> dict:
    statuses = available_depths(repo)
    result = {'version': 1, 'snapshot_date': SNAPSHOT_DATE, 'missing': list(MISSING),
              'bundles': [], 'unavailable': []}
    for name, meta in SOURCES.items():
        status, reason = statuses.get(name, ('unsupported', 'absent from sources.manifest.json'))
        if status != 'available':
            result['unavailable'].append({'name': name, 'reason': reason or status})
            continue
        raw = (repo / 'fixtures/charts' / f'{name}.json').read_bytes()
        manifest = json.loads((repo / 'fixtures/charts' / f'{name}.manifest.json').read_text())
        result['bundles'].append({'name': name, 'sha256': hashlib.sha256(raw).hexdigest(),
            'bytes': len(raw), 'source': meta, 'manifest': manifest,
            'nodes': [n['history'] for n in json.loads(raw)['nodes']]})
    return result

def write_sources(repo: Path) -> None:
    raw = (json.dumps(freeze_sources(repo), sort_keys=True, indent=2) + '\n').encode()
    (repo / 'bench/spots/sources.json').write_bytes(raw)
```

Freeze the covered nodes needed for all six benchmark lines and the 24 pre-solver scenarios: unopened folds and RFI for UTG/HJ/CO/BTN/SB, BB/SB/BTN responses to each named opener, and original-opener responses to the listed 3-bets. Missing-node fallbacks are explicit: the 100bb PokerCoaching bundle has no verified complete versus-3-bet chart, so a missing opener-response prefix stops that replay branch at its pre-action public masses with `UnconditionedPriorStreet{Preflop, cause: "missing node <key>"}`; no synthetic probability is inserted. The synthetic fixture-only missing nodes are UTG limp, CO limp and BB cold-call versus a 3-bet; the exact absent prefix is marked in the manifest. For a straddle the virtual-role missing prefix follows the same rule. Plan 3's chart-source audit must be complete before this task passes: no benchmark generation with an unknown source version or node inventory.

- [ ] **Step 4 (4 min): Verify the lock from Rust before any suite is generated.**

```rust
// crates/engine/src/bench_support.rs — added to Plan 2's facade
#[derive(serde::Deserialize)]
pub struct SourceBundle {pub name:String,pub sha256:String,pub bytes:u64,
    pub nodes:Vec<String>}
#[derive(serde::Deserialize)]
pub struct UnavailableBundle {pub name:String,pub reason:String}
#[derive(serde::Deserialize)]
pub struct SourceLock {pub version:u16,pub snapshot_date:String,pub missing:Vec<String>,
    pub bundles:Vec<SourceBundle>,pub unavailable:Vec<UnavailableBundle>}

pub fn load_source_lock(path:&std::path::Path)->Result<SourceLock,String> {
    let bytes=std::fs::read(path).map_err(|e|format!("sources.json: {e}"))?;
    let lock:SourceLock=serde_json::from_slice(&bytes).map_err(|e|format!("sources.json: {e}"))?;
    if lock.version!=1 {return Err("sources.json version must be 1".into());}
    Ok(lock)
}

/// Recompute each bundle's sha256 from the file the store actually loaded; a mismatch aborts
/// generation instead of silently benchmarking a drifted chart.
pub fn verify_source_lock(lock:&SourceLock,charts_dir:&std::path::Path)->Result<(),String> {
    use sha2::Digest;
    for bundle in &lock.bundles {
        let raw=std::fs::read(charts_dir.join(format!("{}.json",bundle.name)))
            .map_err(|e|format!("{}: {e}",bundle.name))?;
        let actual=hex::encode(sha2::Sha256::digest(&raw));
        if actual!=bundle.sha256||raw.len() as u64!=bundle.bytes {
            return Err(format!("{} drifted from the frozen source lock",bundle.name));
        }
    }
    Ok(())
}
```

Add the `sources` subcommand to `tools/gen_fixtures.py` (see Task 19 Step 3 for the shared `argparse` shape): `python tools/gen_fixtures.py sources` writes the file and `--check` regenerates it in memory and compares bytes.

- [ ] **Step 5 (3 min): Run green:** `python -m pytest tools/tests/test_chart_sources.py -q`; `python tools/gen_fixtures.py sources --check`; `cargo test -p engine`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add tools/chart_sources.py tools/tests/test_chart_sources.py tools/gen_fixtures.py bench/spots/sources.json crates/engine/src/bench_support.rs
git commit -m 'test(bench): freeze chart provenance and the covered node inventory' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 18: Define the fifty recorded inputs as code

Split (b) of the original fixture task: the generator and its inventory tests, with no files emitted yet.

**Files:** Create `tools/e2e_hands.py`, `tools/tests/test_e2e_hands.py`.

**Interfaces:** Consumes Plan 1's PokerKit state/fixture adapter and Task 17's `sources.json`. Produces `e2e_hands.{act, board, base, expect, supported_records, special_records, fault_records, records}`; version-1 records with config/stacks/hero/button/dealt/events/expected/fault. The record schema is a benchmark input format translated into existing proto commands by Task 21; it does not replace proto or depend on its JSON field layout.

- [ ] **Step 1 (4 min): Write inventory tests before writing the generator.**

```python
from e2e_hands import records

def test_fifty_frozen_inputs():
    rows = records()
    assert len(rows) == 50
    assert [r['id'] for r in rows] == [f'{n:03}' for n in range(1, 51)]
    assert sum(r['expected']['numeric_ev'] for r in rows) == 22
    assert [r['class'] for r in rows[:8]] == ['hu_flop_srp'] * 8
    assert [r['class'] for r in rows[8:14]] == ['hu_turn'] * 6
    assert [r['class'] for r in rows[14:20]] == ['hu_river'] * 6
    assert [r['class'] for r in rows[20:22]] == ['projection_admitted'] * 2
    assert all(r['version'] == 1 and r['events'] for r in rows)

def test_fault_records_are_the_last_fourteen():
    rows = records()
    assert [r['class'] for r in rows[36:]] == ['failure_injection'] * 14
    assert [r['fault'] for r in rows[36:46]] == list(FAULTS)
```

- [ ] **Step 2 (2 min): Run red:** `python -m pytest tools/tests/test_e2e_hands.py -q`; missing generator.
- [ ] **Step 3 (5 min): Define the explicit common record and event constructors.**

```python
from copy import deepcopy

def act(seat, street, kind, to=None):
    action = {'kind': kind}
    if to is not None:
        action['to'] = to
    return {'type': 'action', 'seat': seat, 'street': street, 'action': action}

def board(cards):
    return {'type': 'board', 'cards': cards}

def base(depth=100, opener=0, flop='Kh7d2c'):
    # BTN0 SB1 BB2 UTG3 HJ4 CO5; integer accounting tick, BB100.
    events = []
    for seat in (3, 4, 5, 0, 1, 2):
        if seat == opener:
            events.append(act(seat, 'preflop', 'raise', 250))
        elif seat == 2:
            events.append(act(seat, 'preflop', 'call'))
        else:
            events.append(act(seat, 'preflop', 'fold'))
    return {'version': 1, 'config': {'sb_chips': 50, 'bb_chips': 100, 'straddle': None,
        'rake': {'rate': 0.05, 'cap_mchips': 50000, 'no_flop_no_drop': True},
        'flop_budget_s': 10, 'threads': 16, 'target_bp': 50},
        'button': 0, 'dealt': list(range(6)), 'stacks': [depth * 100] * 6,
        'hero': 2, 'hero_cards': '8h7h', 'events': events + [board(flop)], 'fault': None}

def expect(row, cls, numeric, reasons, unsupported=None):
    row['class'] = cls
    row['expected'] = {'numeric_ev': numeric, 'coverage': 'Approximate' if unsupported is None else 'Unsupported',
        'required_reasons': reasons, 'unsupported': unsupported,
        'flop_miss_accuracy': 'raw_target_or_DeadlineBestSoFar' if cls == 'hu_flop_srp' else None}
    return row
```

`RakeProfileMapped` is required wherever Plan 3's chart manifest declares undocumented rake; freeze that from the Task 17 audit. All chart-based supported fixtures require `ChartRounded`. At generation, independently verify that the chosen hero `8h7h` is positive in the replayed chart marginal; if a source chart excludes it, generation fails for review and an explicit revised fixture hero is committed before measurements. Never silently select another hand during a bench run.

- [ ] **Step 4 (5 min): Generate the 22 designated supported records with exact action inputs.**

```python
def supported_records():
    out = []
    for depth in (100, 200):
        for opener in (0, 5):
            for bet in (275, 401):
                r = base(depth, opener)
                r['events'] += [act(2, 'flop', 'check'), act(opener, 'flop', 'bet', bet)]
                out.append(expect(r, 'hu_flop_srp', True, ['ChartRounded']))
    for depth, opener, flop in [(100,0,'Kh7d2c'),(100,5,'Jh9h6c'),(100,0,'8s8d3c'),
                                (200,0,'Kh7d2c'),(200,5,'Jh9h6c'),(200,0,'8s8d3c')]:
        r = base(depth, opener, flop)
        r['events'] += [act(2,'flop','check'),act(opener,'flop','bet',275),act(2,'flop','call'),
                        board(flop+'4d'),act(2,'turn','check'),act(opener,'turn','bet',550)]
        out.append(expect(r,'hu_turn',True,['ChartRounded','UnconditionedPriorStreet']))
    for index, (depth, opener, flop) in enumerate([(100,0,'Kh7d2c'),(100,5,'Jh9h6c'),(200,0,'8s8d3c'),
                                                  (200,5,'Kh7d2c'),(100,0,'Jh9h6c'),(200,0,'8s8d3c')]):
        r = base(depth, opener, flop)
        r['events'] += [act(2,'flop','check'),act(opener,'flop','bet',275),act(2,'flop','call'),
            board(flop+'4d'),act(2,'turn','check'),act(opener,'turn','check'),board(flop+'4d2s'),
            act(2,'river','check'),act(opener,'river','allin' if index>=4 else 'bet',
                                   depth*100-525 if index>=4 else 550)]
        out.append(expect(r,'hu_river',True,['ChartRounded','UnconditionedPriorStreet']))
    for paid in (False, True):
        r = base()
        r['events'] = [act(3,'preflop','raise',250),act(4,'preflop','fold'),act(5,'preflop','fold'),
            act(0,'preflop','call'),act(1,'preflop','fold'),act(2,'preflop','call'),board('Kh7d2c'),
            act(2,'flop','bet',50)]
        if paid:
            r['hero']=0
            r['events'] += [act(3,'flop','call'),act(0,'flop','raise',150),
                            act(2,'flop','raise',250),act(3,'flop','fold')]
        else:
            r['events'] += [act(3,'flop','fold'),act(0,'flop','raise',150)]
        out.append(expect(r,'projection_admitted',True,['ChartRounded','MultiwayStreetRoot']))
    return out
```

Freeze the admitted projection expectations `folded_this_street=1, dead_this_street=0/50` and reproduce facing 100 / min_raise 350 for the paid case. Additional chart missing-prefix reasons must come from the audited source mapping of Task 17 and be hand-checked, never harvested from the final engine output.

- [ ] **Step 5 (5 min): Define the remaining 28 records from this finite class inventory.** IDs and differences are fixed, not random samples:

| IDs | Inputs relative to the common constructors | Expected result |
|---|---|---|
|023–026|Straddle configs1/2/4 and2/5/10, depths100/200; six seats, physical HJ opens to10/25, everyone folds except physical UTG straddler calls; flopKh7d2c, heroUTG, cards8h7h|Approximate; StraddleMapped posts[.25,.5,1]/[.2,.5,1], ChartRounded; any audited prior-prefix reason|
|027|Projection preflop of021; flop BB50, UTGCall, BTN150, BBFold; heroUTG|UnsupportedHistory, failed projection step1|
|028|Same three-way preflop, flop root, heroBB|MultiwayEv, experimental separate|
|029|Same three-way preflop, UTG starting250 and AllIn250 instead of Raise250, heroBB at flop|MultiwayEv; third all-in still pot-eligible|
|030|UTG stack300, BB/BTN10000; flop BB50, UTGAllIn50, BTNCall; turn4d BB100 BTNCall; river2s heroBB|MultiwayEv with contested HU side pot, experimental separate|
|031|UTG preflopCall, heroHJ next, AhAd|MissingPreflopNode|
|032|UTGFold HJFold COCall, heroBTN next, AhAd|MissingPreflopNode|
|033|UTGFold HJ250 CO850 BTNFold SBCall BBFold, heroHJ next, AhAd|MissingPreflopNode at absent SB-cold-call-vs3bet continuation|
|034–036|BTN/BB SRP at100/200/100, heroBTN7c2d; flop after BBCheck; for036 extend flopCheckCheck, turn4d BBCheck|HeroComboOutOfSupport, range_mix present, no hero strategy/EV|
|037–046|Default supported001 input, faults in order Oom,Eof,Malformed,Oversized,TreeMismatch,BlockedCacheIo,SlowAllocation,NoIteration,ClockJump,SuspendResume|Fault expectations in Task 22|
|047–050|Default turn009 withOom, river015 withEof, turn009 withNoIteration, flop001 withSlowAllocation at default budget10 (also exercised at30 by the fault runner)|Task 22 reasons; default final watchdog14.9s; separate max-budget fault variant34.9s|

Implement the transformations as functions returning complete independent records, assign ids in inventory order, and serialize every event, config and stack into each JSON. No runtime reference to another fixture remains in the files.

```python
FAULTS = ('Oom','Eof','Malformed','Oversized','TreeMismatch','BlockedCacheIo',
          'SlowAllocation','NoIteration','ClockJump','SuspendResume')

def fault_records(supported):
    out=[]
    for fault in FAULTS:
        r=deepcopy(supported[0]);r['fault']=fault
        reason='EngineError' if fault in FAULTS[:5] else 'DeadlineExceeded'
        out.append(expect(r,'failure_injection',False,['ChartRounded'],reason))
    for index,fault,budget in ((8,'Oom',10),(14,'Eof',10),(8,'NoIteration',10),(0,'SlowAllocation',10)):
        r=deepcopy(supported[index]);r['fault']=fault;r['config']['flop_budget_s']=budget
        out.append(expect(r,'failure_injection',False,['ChartRounded'],
                          'EngineError' if fault in ('Oom','Eof') else 'DeadlineExceeded'))
    return out
```

- [ ] **Step 5a (5 min): Implement the finite exceptional-input transformations and `records()`.** Each is a complete copy with explicit events, never an expectation-only sample.

```python
def special_records(supported):
    out=[]
    for sb,bb,straddle in ((1,2,4),(2,5,10)):
        for depth in (100,200):
            r=base(depth);r['hero']=3;r['stacks']=[depth*bb]*6
            r['config'].update(sb_chips=sb,bb_chips=bb,straddle=straddle)
            r['config']['rake']['cap_mchips']=bb*500
            r['events']=[act(4,'preflop','raise',straddle*5//2),
                act(5,'preflop','fold'),act(0,'preflop','fold'),act(1,'preflop','fold'),
                act(2,'preflop','fold'),act(3,'preflop','call'),board('Kh7d2c')]
            out.append(expect(r,'straddle_mapping',False,['ChartRounded','StraddleMapped']))
    pre=deepcopy(supported[20]['events'][:7])
    r=base();r['hero']=3;r['events']=deepcopy(pre)+[act(2,'flop','bet',50),
        act(3,'flop','call'),act(0,'flop','raise',150),act(2,'flop','fold')]
    out.append(expect(r,'projection_rejected',False,['ChartRounded'],'UnsupportedHistory'))
    r=base();r['events']=deepcopy(pre)
    out.append(expect(r,'multiway',False,['ChartRounded'],'MultiwayEv'))
    r=deepcopy(r);r['stacks'][3]=250;r['events'][0]=act(3,'preflop','allin',250)
    out.append(expect(r,'third_allin',False,['ChartRounded'],'MultiwayEv'))
    r=base();r['stacks'][3]=300;r['events']=deepcopy(pre)+[
        act(2,'flop','bet',50),act(3,'flop','allin',50),act(0,'flop','call'),
        board('Kh7d2c4d'),act(2,'turn','bet',100),act(0,'turn','call'),board('Kh7d2c4d2s')]
    out.append(expect(r,'multiway_side_pot',False,['ChartRounded'],'MultiwayEv'))
    missing=[(4,[act(3,'preflop','call')]),
        (0,[act(3,'preflop','fold'),act(4,'preflop','fold'),act(5,'preflop','call')]),
        (4,[act(3,'preflop','fold'),act(4,'preflop','raise',250),act(5,'preflop','raise',850),
            act(0,'preflop','fold'),act(1,'preflop','call'),act(2,'preflop','fold')])]
    for hero,events in missing:
        r=base();r['hero']=hero;r['hero_cards']='AhAd';r['events']=events
        out.append(expect(r,'missing_preflop',False,['ChartRounded'],'MissingPreflopNode'))
    for n,depth in enumerate((100,200,100)):
        r=base(depth);r['hero']=0;r['hero_cards']='7c2d'
        r['events'] += [act(2,'flop','check')]
        if n==2:
            r['events'] += [act(0,'flop','check'),board('Kh7d2c4d'),act(2,'turn','check')]
        out.append(expect(r,'hero_out_of_support',False,['ChartRounded'],'HeroComboOutOfSupport'))
    assert len(out)==14
    return out

def records():
    supported=supported_records()
    rows=supported+special_records(supported)+fault_records(supported)
    for i,row in enumerate(rows,1):
        row['id']=f'{i:03}'
    assert len(rows)==50
    return rows
```

Here `numeric_ev: false` on the four straddle cases means they are outside the designated 22 numeric-EV release subset; it is not an instruction to suppress a valid numeric postflop result. Unsupported fixtures separately require numeric absence. Only the supported flag is used for the count assertion; `supported_baseline` is written explicitly into the manifest in Task 19 to keep these meanings distinct.

- [ ] **Step 6 (3 min): Run green:** `python -m pytest tools/tests/test_e2e_hands.py -q`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add tools/e2e_hands.py tools/tests/test_e2e_hands.py
git commit -m 'test(bench): define the fifty recorded decision inputs' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 19: Generate, legality-check and freeze the fifty recorded hands

Split (c) of the original fixture task: emit the files and prove they are legal.

**Files:** Create `fixtures/hands/e2e/001.json` through `fixtures/hands/e2e/050.json`, `fixtures/hands/e2e/manifest.json`; modify `tools/gen_fixtures.py` (the `e2e` subcommand), `tools/e2e_hands.py` (`write_e2e`, `check_e2e`), `tools/tests/test_e2e_hands.py`, `crates/engine/src/bench_support.rs` (record loading and hash validation).

**Interfaces:** Produces `e2e_hands.{write_e2e(root:Path)->None, check_e2e(root:Path)->None}`, CLI `python tools/gen_fixtures.py e2e [--check]`, `engine::bench_support::{RecordedHand, load_records(&Path)->Result<Vec<RecordedHand>,String>}`. The manifest is not a hand; the e2e runner loads exactly 001–050.

- [ ] **Step 1 (4 min): Write the legality and byte-stability tests.**

```python
import json
from pathlib import Path
from e2e_hands import records, write_e2e

ROOT = Path(__file__).resolve().parents[2] / 'fixtures/hands/e2e'

def test_every_record_replays_legally_in_pokerkit():
    from gen_fixtures import replay_record          # Plan 1's PokerKit adapter, extended here
    for row in records():
        trace = replay_record(row)
        assert trace.legal_at_every_step, row['id']
        assert trace.final_actor == row['hero'] or row['expected']['unsupported'], row['id']

def test_generated_bytes_are_stable():
    first = {p.name: p.read_bytes() for p in sorted(ROOT.glob('*.json'))}
    write_e2e(ROOT)
    second = {p.name: p.read_bytes() for p in sorted(ROOT.glob('*.json'))}
    assert first == second
    manifest = json.loads((ROOT / 'manifest.json').read_text())
    assert len(manifest['sha256']) == 50
    assert manifest['supported_ids'] == [f'{n:03}' for n in range(1, 23)]
    assert manifest['supported_baseline'] == manifest['supported_ids']
```

- [ ] **Step 2 (2 min): Run red:** `python -m pytest tools/tests/test_e2e_hands.py -q`; missing `write_e2e` and `replay_record`.
- [ ] **Step 3 (5 min): Write the files, the manifest and the `argparse` wiring.** (Review m9: the subcommand is code, not prose.)

```python
import hashlib, json
from pathlib import Path

def write_e2e(root: Path):
    root.mkdir(parents=True, exist_ok=True)
    rows = records()
    assert len(rows) == 50
    hashes = {}
    for i, row in enumerate(rows, 1):
        row['id'] = f'{i:03}'
        raw = (json.dumps(row, sort_keys=True, indent=2) + '\n').encode()
        (root / f'{i:03}.json').write_bytes(raw)
        hashes[f'{i:03}.json'] = hashlib.sha256(raw).hexdigest()
    ids = [f'{n:03}' for n in range(1, 23)]
    (root / 'manifest.json').write_text(json.dumps({'version': 1, 'synthetic': True,
        'supported_ids': ids, 'supported_baseline': ids, 'sha256': hashes},
        sort_keys=True, indent=2) + '\n')

def check_e2e(root: Path):
    """Regenerate in memory and compare bytes; a difference is an error, never a rewrite."""
    rows = records()
    for i, row in enumerate(rows, 1):
        row['id'] = f'{i:03}'
        expected = (json.dumps(row, sort_keys=True, indent=2) + '\n').encode()
        actual = (root / f'{i:03}.json').read_bytes()
        if actual != expected:
            raise SystemExit(f'fixtures/hands/e2e/{i:03}.json differs from the generator')
    manifest = json.loads((root / 'manifest.json').read_text())
    for name, digest in manifest['sha256'].items():
        if hashlib.sha256((root / name).read_bytes()).hexdigest() != digest:
            raise SystemExit(f'manifest hash mismatch for {name}')
```

```python
# tools/gen_fixtures.py — the subcommand table, extended with `sources` (Task 17) and `e2e`
def main(argv=None):
    import argparse
    from pathlib import Path
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(prog='gen_fixtures')
    sub = parser.add_subparsers(dest='command', required=True)
    for name in ('hands', 'worker', 'sources', 'e2e'):
        p = sub.add_parser(name)
        p.add_argument('--check', action='store_true',
                       help='regenerate in memory and compare bytes instead of writing')
    args = parser.parse_args(argv)
    if args.command == 'sources':
        import chart_sources
        if args.check:
            raw = (json.dumps(chart_sources.freeze_sources(repo), sort_keys=True, indent=2) + '\n').encode()
            if raw != (repo / 'bench/spots/sources.json').read_bytes():
                raise SystemExit('bench/spots/sources.json differs from the generator')
        else:
            chart_sources.write_sources(repo)
        return 0
    if args.command == 'e2e':
        import e2e_hands
        root = repo / 'fixtures/hands/e2e'
        e2e_hands.check_e2e(root) if args.check else e2e_hands.write_e2e(root)
        return 0
    # existing 'hands' and 'worker' branches of plans 1 and 2 are unchanged
    return legacy_main(args, repo)
```

- [ ] **Step 4 (5 min): Validate every record against the PokerKit adapter before freezing.** Extend the existing adapter with `replay_record(row) -> Trace` that plays every event to its decision and compares the legal actor, pot, committed amounts, stacks and board at every step; straddle and projection records must be legal real sequences first. Review the exact reason sets against the Task 17 node inventory and the spec: the 22 supported records require numeric EV after the solver runs, and the remaining 28 carry their declared classification. No real benchmark result is used to invent an expectation.

```rust
// crates/engine/src/bench_support.rs
#[derive(Clone,serde::Deserialize)]
pub struct RecordedEvent {#[serde(rename="type")] pub kind:String,pub seat:Option<u8>,
    pub street:Option<String>,pub action:Option<serde_json::Value>,pub cards:Option<String>}
#[derive(Clone,serde::Deserialize)]
pub struct RecordedHand {pub id:String,pub version:u16,pub config:serde_json::Value,
    pub button:u8,pub dealt:Vec<u8>,pub stacks:Vec<u32>,pub hero:u8,
    pub hero_cards:String,pub events:Vec<RecordedEvent>,pub fault:Option<String>,
    pub class:String,pub expected:serde_json::Value}

/// Loads 001..050 and verifies every sha256 against `manifest.json` before returning.
pub fn load_records(dir:&std::path::Path)->Result<Vec<RecordedHand>,String> {
    use sha2::Digest;
    let manifest:serde_json::Value=serde_json::from_slice(
        &std::fs::read(dir.join("manifest.json")).map_err(|e|e.to_string())?)
        .map_err(|e|e.to_string())?;
    let hashes=manifest["sha256"].as_object().ok_or("manifest sha256")?;
    if hashes.len()!=50 {return Err(format!("expected 50 records, found {}",hashes.len()));}
    let mut out=Vec::new();
    for n in 1..=50 {
        let name=format!("{n:03}.json");
        let raw=std::fs::read(dir.join(&name)).map_err(|e|format!("{name}: {e}"))?;
        let expected=hashes[&name].as_str().ok_or_else(||format!("{name}: no hash"))?;
        if hex::encode(sha2::Sha256::digest(&raw))!=expected {
            return Err(format!("{name} does not match the frozen hash"));
        }
        out.push(serde_json::from_slice::<RecordedHand>(&raw).map_err(|e|format!("{name}: {e}"))?);
    }
    Ok(out)
}
```

- [ ] **Step 5 (3 min): Run green:** `python -m pytest tools/tests/test_e2e_hands.py -q`; `python tools/gen_fixtures.py e2e`; `python tools/gen_fixtures.py e2e --check`; `cargo test -p engine`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add tools fixtures/hands/e2e crates/engine/src/bench_support.rs
git commit -m 'test(bench): generate and freeze the fifty recorded decisions' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 20: Regenerate all six chart-replay suites and measure V3 modes

Cross-plan Or8/R3: §13.5 requires chart-replay ranges on **all six** baseline suites, and no other plan regenerates the river and turn suites; Plan 2 Task 5 only produced the `--source r8` uniform set.

**Files:** Modify `crates/bench/src/{main,gen_spots,report,suite,runner}.rs`, `crates/bench/src/lib.rs`, `crates/engine/src/bench_support.rs`; create `crates/bench/src/flop.rs`, `crates/bench/tests/flop.rs`; **modify/regenerate** the six suite files `bench/spots/{river_std,river_min,turn_std,turn_min,flop_fast,flop_min}.json`, which Plan 2 Task 5's `bench gen-spots --source r8 --out bench/spots` already created (this is intended replacement of the R8 data, not a second creation); modify `solver-worker/Cargo.toml`, `solver-worker/src/{main,memory}.rs` only for an opt-in diagnostic storage-mode launch flag; add `solver-worker/tests/bench_mode.rs`.

**Interfaces:** Consumes `scenario_hand`, `replay`, `SolveInput`, the frozen `sources.json` and `verify_source_lock` through the engine facade below; bench depends only on `engine` and `proto`, and **engine never imports `bench::suite::Spot`**. Produces the engine-owned DTO and both generators:

```rust
// crates/engine/src/bench_support.rs — owned by this task
pub struct PreparedBenchSpot {
    pub line_id: String,
    pub depth_bb: u16,
    pub pot_class: String,
    pub input: Option<proto::SolveInput>,
    pub rake: proto::Rake,
    pub source_hashes: Vec<String>,
    pub inherited_reasons: Vec<proto::ApproxReason>,
    pub unavailable_reason: Option<String>,
}
pub fn generate_flop_spots(source: &Path)
    -> Result<Vec<PreparedBenchSpot>, EngineError>;
pub fn generate_street_spots(street: Street, template: &str, source: &Path)
    -> Result<Vec<PreparedBenchSpot>, EngineError>;
```

There is no `FlopBenchSpot` and the facade never returns `bench`'s `Spot`. Also produces `RunConfig{template:String,threads:u8,mode:String,cold:bool,reps:u32}`, `flop_matrix()->Vec<RunConfig>`, CLI `bench gen-spots [--suite all|<name>]` and `bench run --suite river_std|river_min|turn_std|turn_min|flop_fast|flop_min|e2e --threads N --reps R [--mode f32|i16|auto] [--temperature cold|warm] [--deadline-trials] --out docs/bench/` (review m8: every flag invoked in Tasks 24–25 is declared here).

- [ ] **Step 1 (4 min): Write suite cardinality and exact-template assertions.**

```rust
#[test]
fn flop_matrix_has_six_lines_on_three_boards() {
    let matrix=bench::flop::flop_matrix();
    assert_eq!(matrix.len(),2*3*2*2);
    assert!(matrix.iter().all(|r|r.reps==5));
    let lines=[("BTN-BB-SRP",100),("CO-BB-SRP",100),("BTN-BB-SRP",200),
               ("CO-BB-SRP",200),("BTN-BB-3BP",100),("BTN-BB-3BP",200)];
    let boards=["Kh7d2c","Jh9h6c","8s8d3c"];
    assert_eq!(lines.len()*boards.len(),18);
    assert_eq!(18*2*2*3*2*5,2160); // spots,modes,templates,threads,cold/warm,reps
}

#[test]
fn all_six_suites_use_chart_replay_ranges() {
    let lock=engine::bench_support::load_source_lock(
        std::path::Path::new("../../bench/spots/sources.json")).unwrap();
    let depth_200_available=lock.bundles.iter().any(|b|b.name=="rangeconverter_200");
    for suite in ["river_std","river_min","turn_std","turn_min","flop_fast","flop_min"] {
        let s=bench::suite::Suite::load(std::path::Path::new(&format!("../../bench/spots/{suite}.json"))).unwrap();
        assert_eq!(s.spots.len(),6,"{suite}");
        // Uniform `r8` ranges from Plan 2 Task 5 must be gone from every suite (Or8).
        assert!(s.spots.iter().all(|spot|spot.range_source.starts_with("chart_replay:")
            ||spot.range_source.starts_with("unavailable:")),"{suite}");
        let unavailable=s.spots.iter().filter(|spot|spot.range_source.starts_with("unavailable:")).count();
        assert_eq!(unavailable,if depth_200_available {0} else {3},"{suite}");
    }
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test flop`; the suites still carry Plan 2's `r8_uniform` source.
- [ ] **Step 3 (5 min): Implement gen-spots for all six suites using the frozen source lock.** Six flop line definitions as above: open/call and open/BB-3bet/opener-call with all explicit folds, actual chart-resolved sizes, 100/200bb. The river and turn suites replay the same six lines forward to their street roots (flop check/check, then turn check/check for the river suites) so their ranges are chart-conditioned rather than uniform. Call the same engine scenario/replay facade as the live and pre-solver paths, retain the missing-node fallback reasons already frozen in `sources.json`, board-block the canonical public ranges and derive the financial root. Encode full 1326 vectors rather than substituting uniform chart-like strings. Attach the source hashes, exact history, range hashes, counts and masses, P/stacks/cap, template signature and expected inherited reasons. `verify_source_lock` runs first: a hash mismatch aborts generation. Each suite file contains six scenario entries; the flop suites expand to three board variants and 18 concrete solve inputs.

**Bench adapts the engine DTO; the engine never imports bench's schema (F14).** `bench` converts `PreparedBenchSpot` into its own persisted `Spot`. Change `Spot`'s `oop_range`/`ip_range` from `String` to a serde-untagged `RangeSpec` with `Text(String)` and `Weights(Range1326)` variants. R8 inputs remain `Text` and use `prepared_range`; chart inputs serialize full `Weights` arrays. Add source hashes, inherited reasons and an explicit unavailable reason field. An unavailable prepared spot has `input = None` and must never be executed or counted as a measurement. Update `Suite::load`/`save`, `run_spot` and `gen-spots` together, preserving the old R8 JSON reading and enforcing chart provenance.

```rust
// crates/bench/src/suite.rs — the range representation both sources share
#[derive(Clone,serde::Serialize,serde::Deserialize)]
#[serde(untagged)]
pub enum RangeSpec { Text(String), Weights(proto::Range1326) }
```

```rust
pub const BOARDS:[&str;3]=["Kh7d2c","Jh9h6c","8s8d3c"];
pub const THREADS:[u8;3]=[8,16,24];
pub const MODES:[&str;2]=["f32","i16"];
pub const TEMPLATES:[&str;2]=["flop_fast_v1","flop_min_v1"];
pub const SUITES:[(&str,&str);6]=[("river_std","river_std_v1"),("river_min","river_min_v1"),
    ("turn_std","turn_std_v1"),("turn_min","turn_min_v1"),
    ("flop_fast","flop_fast_v1"),("flop_min","flop_min_v1")];
pub struct RunConfig {pub template:String,pub threads:u8,pub mode:String,pub cold:bool,pub reps:u32}
pub fn flop_matrix()->Vec<RunConfig> {
    let mut out=Vec::new();
    for template in TEMPLATES {for threads in THREADS {for mode in MODES {for cold in [true,false] {
        out.push(RunConfig{template:template.into(),threads,mode:mode.into(),cold,reps:5});
    }}}}
    out
}
```

`crates/bench/src/lib.rs` (declared by Plan 2 Task 5, cross-plan D4) gains `pub mod flop;` beside the existing `suite`, `gen_spots`, `runner` and `report` modules; `main.rs` imports the library modules. The e2e, fault, gate and oracle exports are added by the tasks that create those files, never declared ahead of them, so each task's workspace build stays green. When Task 17's lock marks depth 200 `unsupported`, `gen-spots` still writes six entries per suite and gives each 200bb spot `range_source = "unavailable: depth 200 unsupported by the acquired sources"` with no ranges; the runner skips those spots and the gate reports `required_matrix_complete = false` rather than treating them as passes.

- [ ] **Step 4 (5 min): Add diagnostic mode forcing without changing the production protocol.** Production memory selection remains §10.3. The `bench-mode` feature (default off) permits the launch flag `--bench-storage-mode f32|i16`; it is never accepted as a solve JSON field. The default worker rejects the flag. Build the diagnostic binary into `target/bench-mode`, not the normal packaged path; `ready` adds the capability `bench_storage_override`, which the normal Engine rejects unless the bench facade explicitly opts in. A forced mode still checks memory headroom and the 16 GiB job-object limit. Bench records both estimates and refusals; a failed allocation is data, never a silent mode switch.

```rust
#[derive(Clone,Copy)]
pub enum BenchStorageMode {F32,I16}
pub fn admitted_estimate(f32_bytes:u64,i16_bytes:u64,forced:Option<BenchStorageMode>,limit:u64)
    ->Option<(bool,u64)> {
    let (compressed,bytes)=match forced {
        Some(BenchStorageMode::F32)=>(false,f32_bytes),Some(BenchStorageMode::I16)=>(true,i16_bytes),
        None if f32_bytes<=2*1024*1024*1024=>(false,f32_bytes),
        None if i16_bytes<=8*1024*1024*1024=>(true,i16_bytes),_=>return None,
    };
    (u128::from(bytes)*5<=u128::from(limit)*4).then_some((compressed,bytes))
}
```

Compile the forced branch only under the feature, while the existing production memory tests keep passing `None`. The default benchmark memory limit is 10 GiB; for diagnostic forced-f32 measurements that would exceed it, use an explicit 12 GiB request limit under the unchanged 16 GiB process limit and mark `diagnostic_memory_limit` in the report; those runs cannot certify live admission. No direct library dependency is added to bench or engine. Test default flag rejection and the mode boundaries 2 GiB, 2 GiB + 1, 4 GiB, 8 GiB and headroom. Requests retain the normative wire schema.

- [ ] **Step 5 (5 min): Declare the runner flags and the matrix semantics.** Cold means a new validated worker process for each of five runs; warm means five solves in a validated retained process, still rebuilding and freeing games per contract. Threads 8/16/24, f32 and i16 on every spot and template. The long measurement deadline 600000 finds the raw 50 bp time-to-target; separate production-policy trials (`--deadline-trials`) use the 10 s street budget and default memory selection. Record `memory_usage()` for both estimates, the actual chosen mode, peak working set and total RSS (engine + worker), full admission-to-terminal latency and solve time, iterations, raw exploitability, cancellation latency, startup/cold classification and the exact template signature. No censored deadline observation is counted as time-to-target.

```rust
pub fn target_reached(raw:f32,pot:u32)->bool {raw.is_finite()&&raw as f64/pot as f64<=0.005}
pub fn v3_min_admitted(p95_100:Option<f64>,p95_200:Option<f64>)->bool {
    [p95_100,p95_200].into_iter().all(|x|x.is_some_and(|s|s.is_finite()&&s<=10.0&&s>0.0))
}
```

V3: 3-bet fast p95 <= 10 s, reproducing the analogue 4.1–6.7 s / 0.55–0.9 GB within 2x; SRP records 100bb 19–35 s / 3.2–5.3 GB f32 / 1.6–2.7 GB i16 and 200bb 39–64 s / 5.2–8.8 GB for comparison, without requiring live 50 bp in 10 s. Measure `flop_min_v1` at both depths and all boards before admitting it. Record the f32/i16 crossover near 2 GiB and 4 GiB with the same exact templates plus deterministic range-width probes when the six lines do not bracket both points; probe results are diagnostic and never substituted baseline ranges. Recompute `1755 × measured p50` throughput. Report deviations and outliers; never discard a slow run to pass p95.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p bench --test flop`; `cargo test -p solver-worker`; `cargo test --workspace`. Benchmark execution itself is Tasks 24–25, after the remaining report and gate code.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/bench crates/engine/src/bench_support.rs bench/spots solver-worker
git commit -m 'feat(bench): regenerate all six replay suites and add V3 measurements' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 21: Replay recorded hands through the real engine

**Files:** Create `crates/bench/src/e2e.rs`, `crates/bench/tests/e2e.rs`; modify `crates/bench/src/{lib,main,report}.rs`, `crates/engine/src/bench_support.rs`.

**Interfaces:** Consumes Task 19's `RecordedHand`/`load_records` and the existing `Engine` commands and `WorkerLink`. Produces CLI `bench e2e [--reps N] [--cache cold|presolved]` and the alias `bench run --suite e2e`; engine facade `run_record(record:&RecordedHand,options:&RunOptions)->Result<DecisionRun,EngineError>`. `RunOptions{cache_dir:PathBuf,worker_factory:Box<dyn Fn()->Box<dyn WorkerLink>>,clock:Arc<dyn Clock>,cold_cache:bool}` injects the transport without depending on the future fault module (cross-plan M2: these three types are defined here, in `bench_support.rs`, not only described). `DecisionRun{events:Vec<RecommendationEvent>,timestamps:Vec<u64>,record:DecisionRecord,peak_rss_bytes:u64,raw_exploitability:Option<f64>,snapshots:usize}`; `bench::e2e::{default_options()->RunOptions, run_suite(reps:u32,presolved:bool,out:&Path)->E2eReport}` (`default_options` builds a per-run temporary cache directory, the real `ProcessWorker` factory and the system clock). Inputs use public `Engine` commands; timing starts immediately before `recommend` admission and excludes human-entry playback.

**Fault fixtures (review m11):** records 037–050 declare a `fault` and are **skipped by this task** — `run_record` returns `Err(EngineError::Message("fault fixtures require the fault runner"))` for any record whose `fault` is `Some`, and `crates/bench/tests/e2e.rs` asserts exactly 36 runnable records. Task 22 supplies `FaultyWorker`, sets `RunOptions.worker_factory` accordingly and enables all 14.

- [ ] **Step 1 (4 min): Write fixture-integrity and supported-result assertions.**

```rust
#[test]
fn e2e_inventory_is_exactly_fifty() {
    let _runner:fn(&engine::bench_support::RecordedHand,&engine::bench_support::RunOptions)
        ->Result<engine::bench_support::DecisionRun,engine::EngineError>=engine::bench_support::run_record;
    let root=std::path::Path::new("../../fixtures/hands/e2e");
    for id in 1..=50 {assert!(root.join(format!("{id:03}.json")).is_file());}
    let manifest:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest["supported_ids"].as_array().unwrap().len(),22);
    assert_eq!(manifest["sha256"].as_object().unwrap().len(),50);
    let records=engine::bench_support::load_records(root).unwrap();
    assert_eq!(records.iter().filter(|r|r.fault.is_none()).count(),36);
    assert_eq!(records.iter().filter(|r|r.fault.is_some()).count(),14);
}

#[test]
fn fault_records_are_skipped_until_the_fault_runner_exists() {
    let root=std::path::Path::new("../../fixtures/hands/e2e");
    let records=engine::bench_support::load_records(root).unwrap();
    let options=bench::e2e::default_options();
    for record in records.iter().filter(|r|r.fault.is_some()) {
        assert!(engine::bench_support::run_record(record,&options).is_err(),"{}",record.id);
    }
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test e2e`; include the `run_record` assertions before the runner exists.
- [ ] **Step 3 (5 min): Implement command playback and event capture.** Validate the schema and hash first; build the config and confirmed starting stacks, `begin_hand`, set hero cards, apply each action only when the recorded seat equals the derived actor, set the full board only at `AwaitingBoard`, and `recommend` at the final recorded decision. Use the chart bundles only for the baseline. No fabricated `HandState` reaches `recommend`. Wait for the matching Final, retain a late `Equity` for validation and reject stale events. On completion, `finish_hand` or `abandon_hand` and clean only that run's isolated cache and log directory.

```rust
pub fn evaluated_actions_present(rec:&proto::Recommendation)->bool {
    !rec.actions.is_empty()&&rec.actions.iter().all(|a|a.frequency.is_some()&&a.ev_bb.is_some())
}
pub fn final_for<'a>(events:&'a[proto::RecommendationEvent],id:&proto::DecisionIdentity)
    ->Option<&'a proto::Recommendation> {
    events.iter().find_map(|e|match e {proto::RecommendationEvent::Final(r) if &r.identity==id=>Some(r),_=>None})
}
```

- [ ] **Step 4 (5 min): Enforce expected coverage and the cold and hit passes.** The first pass has an empty cache per hand and the default budget 10; all 22 designated supported inputs require numeric EV with the expected reasons. The 8 SRP miss results require `DeadlineBestSoFar` iff the selected live template did not reach raw 50 bp; chart reasons always remain, so normal baseline hits are `Approximate` rather than `Exact`. A second targeted pass pre-solves the supported flop spots to target and measures cache-hit p95 <= 0.5 s with no newly incurred `DeadlineBestSoFar`. Turn and river cases missing prior requests legitimately retain `UnconditionedPriorStreet`; Task 12's goldens separately verify conditioned cached replay. Verify the two admitted projection dead-money values and the rejected step 1. Multiway experimental advice stays outside `actions`; unsupported and no-support fixtures never get fabricated numeric EV.

```rust
pub fn has_deadline_reason(c:&proto::Coverage)->bool {
    let reasons=match c {proto::Coverage::Approximate{reasons}=>reasons,
        proto::Coverage::Unsupported{partial,..}=>partial,proto::Coverage::Exact=>return false};
    reasons.iter().any(|r|matches!(r,proto::ApproxReason::DeadlineBestSoFar{..}))
}
```

Record first and final latency and violations independently. Bench may time out a hung runner as a failure; it must not manufacture a Final or omit the failed sample. `--reps 5` cold-cache default-budget admission-to-Final samples include SRP misses, retries and restarts. Fixed fixture failure definitions determine the expectations; they are never rewritten after seeing measured output.

- [ ] **Step 5 (3 min): Run green:** `cargo test -p bench --test e2e`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/bench crates/engine/src/bench_support.rs
git commit -m 'feat(bench): replay thirty-six frozen hands through engine admission' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 22: Inject worker, disk and clock faults behind WorkerLink

**Files:** Create `crates/bench/src/{fault,faulty_worker}.rs`, `crates/bench/tests/fault.rs`; modify `crates/bench/Cargo.toml`, `crates/bench/src/{lib,main,e2e}.rs`, `crates/engine/src/{bench_support,testing}.rs`, `crates/engine/tests/flop_path_golden.rs`.

**Interfaces:** Consumes Plan 2's `engine::worker::WorkerLink`, `engine::testing::{FakeClock,FakeWorker,FakeReply}`, the existing raw-line parser and `WorkerLinkError`, and the cache I/O test seam. Produces `Fault`, `FaultyWorker` implementing that existing trait by delegation to `FakeWorker`, `run_fault(fault:Fault,street:Street,budget:u8,retained:bool)->FaultRun`, `fault_worker_factory(Fault)->Box<dyn Fn()->Box<dyn WorkerLink>>`, `options_for(record:&RecordedHand)->RunOptions` (Task 21's `default_options` with the record's declared fault bound into `worker_factory`); CLI `bench fault` and `bench run --suite fault`. Enables the engine feature `testing` in the bench dependency. Uses the actual trait signatures from Plan 2; no alternate worker transport contract. This task also **enables records 037–050** in Task 21's runner by supplying `RunOptions.worker_factory` from `fault_worker_factory`, and removes `run_record`'s temporary rejection of `fault.is_some()`.

- [ ] **Step 1 (4 min): Define the exhaustive injections and the red suite assertion.**

```rust
#[derive(Clone,Copy,Debug)]
pub enum Fault {Oom,Eof,Malformed,Oversized,TreeMismatch,BlockedCacheIo,
    SlowAllocation,NoIteration,ClockJump,SuspendResume}
pub const FAULTS:[Fault;10]=[Fault::Oom,Fault::Eof,Fault::Malformed,Fault::Oversized,
    Fault::TreeMismatch,Fault::BlockedCacheIo,Fault::SlowAllocation,Fault::NoIteration,
    Fault::ClockJump,Fault::SuspendResume];
#[test]
fn fault_suite_zero_final_delivery_violations() {
    for fault in FAULTS {
        for retained in [false,true] {
            let r=run_fault(fault,proto::Street::Flop,10,retained);
            assert_eq!(r.final_count,1,"{fault:?}");assert_eq!(r.final_delivery_violations,0,"{fault:?}");
            assert!(r.final_at_ms<=14_900,"{fault:?}");assert!(!r.accepted_late_result);
        }
    }
}
#[test]
fn all_fourteen_fault_records_now_run() {
    let root=std::path::Path::new("../../fixtures/hands/e2e");
    let records=engine::bench_support::load_records(root).unwrap();
    for record in records.iter().filter(|r|r.fault.is_some()) {
        let options=bench::fault::options_for(record);
        let run=engine::bench_support::run_record(record,&options).expect(&record.id);
        assert_eq!(run.events.iter().filter(|e|matches!(e,
            proto::RecommendationEvent::Final(_))).count(),1,"{}",record.id);
    }
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test fault`; missing `run_fault` or a violated deadline.
- [ ] **Step 3 (5 min): Configure the fault scripts through the existing trait.** `FaultyWorker` contains a `FakeWorker`; its send/recv/restart/kill methods delegate, except that the configured script feeds raw bytes to the real parser for the malformed and oversized cases and injects confirmed exit errors for OOM and EOF. Add raw-byte `FakeReply` support if Plan 2 does not expose it yet, reusing its bounded stdout decoder. A compile assertion proves the actual trait is used:

```rust
fn assert_worker_link<T:engine::worker::WorkerLink>() {}
#[test]
fn faulty_worker_uses_production_boundary() {assert_worker_link::<FaultyWorker>();}
pub fn malformed_line()->Vec<u8> {b"{\"type\":\"result\",broken}\n".to_vec()}
pub fn oversized_line()->Vec<u8> {vec![b'x';16*1024*1024+1]}
```

| Fault | Injection | Expected no-retained result |
|---|---|---|
|Oom|Confirmed abnormal process exit during allocation; repeat on the allowed retry|EngineError with typed WorkerExit; an explicit worker out_of_memory variant separately ends TreeTooLarge after the min retry|
|Eof|stdout EOF before terminal; the restart also EOFs|EngineError, once-only Final|
|Malformed|Invalid JSON and a separate complete JSON result with an invalid probability matrix|EngineError; neither cached nor snapshotted|
|Oversized|16MiB+1 bytes before newline|EngineError from the bounded reader; no unbounded allocation|
|TreeMismatch|non-retryable tree_mismatch terminal|EngineError retryable false; no retry, no smaller template|
|BlockedCacheIo|Cache reader held on a barrier; the worker is also silent for the timeout variant|DeadlineExceeded while the cache stays blocked; a separate returning-worker variant still yields a valid live Final|
|SlowAllocation|Progress Building, no terminal; the kill/restart barrier stays held|DeadlineExceeded by the watchdog before the kill finishes|
|NoIteration|error no_iteration; the min retry is not admitted by the remaining-time estimate|DeadlineExceeded; no empty best_so_far|
|ClockJump|Jump the wall clock by ±1 day while the worker stays silent; monotonic time advances normally|DeadlineExceeded at the monotonic watchdog boundary; the wall clock never rebases admission or retry budgets|
|SuspendResume|A resume event invalidates identity and expiry during the solve|DeadlineExceeded immediately on resume; the worker restart is independent|

- [ ] **Step 4 (5 min): Implement `run_fault` with independent clock and watchdog scheduling.** Use the engine harness, fake monotonic and wall clocks, and the real watchdog callback. The test driver advances both the scheduler and the watchdog independently of `WorkerLink` polling, so a fake worker blocked forever cannot advance or own delivery time. Inject faults in Building, Solving and Extracting, and both after a Provisional and without one. Hold the kill, cache-read, cache-write, stdout-read and allocation barriers while advancing to 14.9 s; assert Final first and release the barriers only afterwards. A max-budget flop fires at 34.9 s; turn and river stay at 14.9 s even with preference 30. A wall jump is not a suspend. A suspend immediately expires and rejects any later reply even if the monotonic implementation excludes sleep.

```rust
pub struct FaultRun {
    pub final_count:usize,pub final_at_ms:u64,pub final_delivery_violations:u32,
    pub accepted_late_result:bool,pub snapshot_count_after_expiry:usize,
}
pub fn final_limit_ms(street:proto::Street,budget:u8)->u64 {
    if street==proto::Street::Flop {5000+u64::from(budget)*1000-100}else{14900}
}
```

For retained-payload cases other than SuspendResume, the Final is the validated Provisional with its inherited reasons and numeric EV; a terminal worker failure must not erase it. Assert that no expired cache result registers a snapshot, that there is no duplicate terminal or Final, that no `ack` releases admission, and that no exception changes money or history. Run the same deterministic fault scripts across all streets, plus the maximum flop budget. Real-clock smoke runs exercise blocked I/O and dead-worker delivery but stay separate from the deterministic zero-violation proofs.

- [ ] **Step 5 (3 min): Run green:** `cargo test -p bench --test fault`; `cargo test -p bench --test e2e`; `cargo test -p engine --features testing final_delivery`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/bench crates/engine
git commit -m 'test(bench): enforce final delivery under worker disk and clock faults' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 23: Report measured results, run the oracle suites and enforce the baseline gate

**Files:** Modify `crates/bench/src/{lib,main,report}.rs`; create `crates/bench/src/{gate,oracle}.rs`, `crates/bench/tests/{gate,oracle}.rs`.

**Interfaces:** Consumes Plan 2's `report`/`SpotResult` plus Tasks 20–22 run records. Produces `GateInput`, `GateResult{passed:bool,failures:Vec<String>}`, `evaluate(&GateInput)->GateResult`, `percentile`, `bound`, `gate_exit`; `oracle::{OracleSuite, run_oracles(out:&Path)->OracleReport, OracleReport{suites:Vec<(String,bool,String)>,passed:bool}}`; CLI `bench gate --report <path>` and `bench oracle [--out docs/bench/]`. The Markdown report includes a machine-readable JSON block with exact unrounded measurements so gates never parse rounded table cells.

**Cross-plan Or2 / review M1:** `bench oracle` is created by Plan 4 Task 23 (this task) and invoked in Task 25. Nothing else in the series implements it, and the gate's `analytic_oracles_ok` input has no other producer. Plan 1 Task 22 supplies fixture generation and exhaustive test inputs, not this subcommand; Plan 2 Tasks 5/30 implement `materialize`/`gen-spots`/`run`, not `oracle`.

- [ ] **Step 1 (4 min): Write inclusive-boundary, missing-evidence and oracle-wiring tests.**

```rust
#[test]
fn nearest_rank_p95_and_inclusive_bounds() {
    assert_eq!(percentile(&[1.0,2.0,3.0,4.0,10.0],0.95),Some(10.0));
    assert!(bound(Some(2.0),2.0));assert!(!bound(Some(2.000001),2.0));
    assert!(!bound(None,15.0));assert!(!bound(Some(f64::NAN),15.0));
}

#[test]
fn oracle_report_lists_every_required_suite() {
    let names=bench::oracle::REQUIRED_SUITES.iter().map(|s|s.name).collect::<Vec<_>>();
    assert_eq!(names,vec![
        "core-eval exhaustive (13.1 T1)",
        "core-iso exhaustive (13.1 T2)",
        "solver-worker contract_river (13.2)",
        "engine river check-only terminal oracle (13.2)",
        "engine facing_allin_golden (13.3)"]);
    // a suite that was never run is a failure, never an implicit pass
    let report=bench::oracle::OracleReport{suites:vec![],passed:false};
    assert!(!report.passed);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test gate`; `cargo test -p bench --test oracle`.
- [ ] **Step 3 (5 min): Implement non-rounded timing and accuracy aggregates.**

```rust
pub fn percentile(values:&[f64],p:f64)->Option<f64> {
    if values.is_empty()||values.iter().any(|x|!x.is_finite()||*x<0.0) {return None;}
    let mut x=values.to_vec();x.sort_by(f64::total_cmp);
    let index=((p*x.len() as f64).ceil() as usize).saturating_sub(1).min(x.len()-1);
    Some(x[index])
}
pub fn bound(value:Option<f64>,maximum:f64)->bool {value.is_some_and(|x|x.is_finite()&&x>=0.0&&x<=maximum)}
pub struct GateInput {
    pub river_p95_s:Option<f64>,pub turn_p95_s:Option<f64>,pub threebet_fast_p95_s:Option<f64>,
    pub srp_terminals_valid_and_on_time:bool,pub cache_hit_p95_s:Option<f64>,
    pub e2e_p95_s:Option<f64>,pub fault_final_violations:u32,pub all_final_violations:u32,pub supported_count:usize,
    pub supported_numeric_and_coverage_ok:bool,pub analytic_oracles_ok:bool,
    pub inventory_hashes_ok:bool,pub required_matrix_complete:bool,pub fault_matrix_complete:bool,
    pub presolver_verified_entries:usize,
    /// §14.4 V22 also requires UI acceptance evidence. Sourced **only** from Plan 5 Task 14's
    /// actual named chart E2E test result, with the same worker/source/config provenance as
    /// the release candidate. Missing evidence is `false`.
    pub chart_ui_e2e_ok:bool,
}
pub struct GateResult {pub passed:bool,pub failures:Vec<String>,pub informational:Vec<String>}
pub fn evaluate(g:&GateInput)->GateResult {
    // §13.5 release conditions. Every one of these must hold.
    let checks=[
        ("river p95 <= 2 s",bound(g.river_p95_s,2.0)),
        ("turn p95 <= 6 s",bound(g.turn_p95_s,6.0)),
        ("3-bet flop_fast p95 <= 10 s",bound(g.threebet_fast_p95_s,10.0)),
        ("SRP validated terminal within flop_budget_s",g.srp_terminals_valid_and_on_time),
        ("pre-solved cache p95 <= 0.5 s",bound(g.cache_hit_p95_s,0.5)),
        ("cold-cache default-budget e2e p95 <= 15 s",bound(g.e2e_p95_s,15.0)),
        ("zero fault final-delivery violations",g.fault_final_violations==0&&g.fault_matrix_complete),
        ("zero final-delivery violations across all measured runs",g.all_final_violations==0),
        ("22 supported fixtures have numeric EV and expected coverage",g.supported_count==22&&g.supported_numeric_and_coverage_ok),
        ("T1 and analytic river within tolerances",g.analytic_oracles_ok),
        ("frozen inputs and full required measurements",g.inventory_hashes_ok&&g.required_matrix_complete),
        ("chart UI E2E",g.chart_ui_e2e_ok),
    ];
    // Not a §13.5 condition (review m7): tier-1 coverage is progress, not a release blocker,
    // so a fresh install with an empty pre-solver queue must not fail V22 on it.
    let informational=if g.presolver_verified_entries>0 {Vec::new()}
        else {vec!["informational: the pre-solver has no verified production entries yet".to_string()]};
    let failures=checks.into_iter().filter(|(_,ok)|!*ok).map(|(s,_)|s.into()).collect::<Vec<_>>();
    GateResult{passed:failures.is_empty(),failures,informational}
}
pub fn gate_exit(result:&GateResult)->i32 {if result.passed {0}else{1}}
```

Treat a nonzero final-delivery violation anywhere as a release failure even when p95 passes; include counts separately for all runs, fault runs and each street. An SRP `best_so_far` within the street budget is expected; a late first terminal is still a street violation. A turn or river `best_so_far` is a street violation even before final delivery. The gate does not demand SRP raw 50 bp live in 10 s: it demands a validated terminal in budget, numeric EV, measured raw reached accuracy, the correct `DeadlineBestSoFar` reason and the pre-solved hit latency. The informational line is printed in the report and in the CLI output but never changes the exit code.

- [ ] **Step 3a (5 min): Implement `bench oracle`.**

```rust
// crates/bench/src/oracle.rs
pub struct OracleSuite {pub name:&'static str,pub program:&'static str,pub args:&'static [&'static str]}
pub const REQUIRED_SUITES:[OracleSuite;5]=[
    OracleSuite{name:"core-eval exhaustive (13.1 T1)",program:"cargo",
        args:&["test","-p","core-eval","--features","exhaustive","--release"]},
    OracleSuite{name:"core-iso exhaustive (13.1 T2)",program:"cargo",
        args:&["test","-p","core-iso","--features","exhaustive","--release"]},
    // Plan 2 Task 15 owns `solver-worker/tests/contract_river.rs`; there is no
    // `river_analytic` or `ev_contracts` target anywhere in the series.
    OracleSuite{name:"solver-worker contract_river (13.2)",program:"cargo",
        args:&["test","-p","solver-worker","--release","--test","contract_river"]},
    // Plan 2 Task 18 owns `engine/tests/worker_link.rs::river_check_only_terminal_oracle`.
    OracleSuite{name:"engine river check-only terminal oracle (13.2)",program:"cargo",
        args:&["test","-p","engine","--release","--test","worker_link",
               "river_check_only_terminal_oracle","--","--exact"]},
    OracleSuite{name:"engine facing_allin_golden (13.3)",program:"cargo",
        args:&["test","-p","engine","--features","testing","--release","facing_allin_golden"]},
];
pub struct OracleReport {pub suites:Vec<(String,bool,String)>,pub passed:bool}

/// Actually generates the 10,000,000-sample 7-card oracle when it is absent (it is gitignored,
/// spec §13.0 / S17) — this is an executed command with a checked exit code, not a promise in a
/// comment — then runs every §13.1 exhaustive suite and §13.2 analytic contract test and writes
/// their pass/fail into the report's machine-readable JSON block.
pub fn run_oracles(out:&std::path::Path)->OracleReport {
    let mut suites=Vec::new();
    if !oracle_fixture_present() {
        let generated=std::process::Command::new("tools/.venv/Scripts/python")
            .args(["tools/gen_eval_oracle.py","--skip-5card","--samples","10000000",
                   "--samples-name","phevaluator_7card_10m.bin"]).status();
        match generated {
            Ok(status) if status.success()=>{}
            other=>{
                suites.push(("10M 7-card oracle generation".to_string(),false,
                    format!("{other:?}")));
                crate::report::append_oracle_block(out,&suites);
                return OracleReport{suites,passed:false};   // do not proceed on failure
            }
        }
    }
    for suite in REQUIRED_SUITES {
        // The solver-worker command runs under the Plan 2 Task 1 V1-selected toolchain/target,
        // with the selected executable supplied through POKERAI_WORKER.
        let output=std::process::Command::new(suite.program)
            .args(worker_toolchain_args(suite)).output();
        let (ok,detail)=match output {
            Ok(o)=>(o.status.success()&&executed_test_count(&o)>0,
                    format!("exit {:?}; tests {}",o.status.code(),executed_test_count(&o))),
            Err(e)=>(false,format!("failed to launch: {e}")),
        };
        suites.push((suite.name.to_string(),ok,detail));
    }
    let passed=suites.iter().all(|(_,ok,_)|*ok);
    crate::report::append_oracle_block(out,&suites);
    OracleReport{suites,passed}
}
```

`report::append_oracle_block(&Path,&[(String,bool,String)])` appends the results to the dated report's JSON block under the key `analytic_oracles`, and the gate reads `analytic_oracles_ok` from exactly that key: a missing key is `false`, never an implicit pass. A launch failure is recorded as a failure with its message. Record the full command, the exit code and the executed-test count for every suite: an ignored, skipped or zero-test run cannot satisfy `analytic_oracles_ok`. The five-card committed oracle remains required. Dispatch every suite through `worker_toolchain_args(suite: &OracleSuite) -> Vec<&'static str>`, which reads and validates the V1 selection from `docs/bench/worker-toolchain.json` and returns the complete Cargo argument vector for that suite; call `.args(worker_toolchain_args(suite))` once, without appending `suite.args` again. For the GNU worker suite it returns `["+stable-x86_64-pc-windows-gnu", "test", "-p", "solver-worker", "--release", "--target", "x86_64-pc-windows-gnu", "--test", "contract_river"]`; otherwise it returns `suite.args` unchanged — `--target` must follow the `test` subcommand, never precede it. Set `POKERAI_WORKER` to the selected built executable; no ordinary workspace command may rebuild the rejected worker.

- [ ] **Step 4 (5 min): Extend the report fields and evidence provenance.** Report path `docs/bench/<date>-i7-13700K.md` (first dated 2026-09-10). Include CPU/RAM/OS, toolchain, commit/adapter/rules/proto versions, `+avx2`, source hashes and the node-fallback inventory, fixture hashes, template signatures, thread count, cold/warm and f32/i16. Per baseline and store suite: p50/p95/max wall time to target 50, separate time-to-target and admission-to-terminal/Final, `memory_usage` estimates, peak total RSS, cancel latency, Exact/Approximate/Unsupported counts and proportions, raw reached exploitability, mode, and first-terminal and final-delivery violation counts. Time-to-target is null/censored when the target was not reached, never the deadline duration.

```rust
#[derive(serde::Serialize)]
pub struct AccuracyObservation {
    pub target_bp:u16,pub exploitability_over_p:f64,pub terminal_ms:u64,
    pub time_to_target_ms:Option<u64>,pub status:String,
}
```

Include the V3 admission decision for `flop_min_v1` at both depths, forced-mode diagnostic flags, crossover probes, throughput `1755 × p50`, tier counts and observed decision-log hit rates. Record the V1-selected worker toolchain on every measured row. Show V21 and V22 separately: V21 is e2e p95 <= 15 / zero final violations / every supported numeric fixture; V22 is all baseline bounds, the analytic values **and** Plan 5 Task 14's `chart_ui_e2e` evidence. V9 is deferred: the store section reads exactly `not run: V9 deferred`, never a zero-time pass and never a blocker for the baseline. An acquired-data gate later uses identical bounds and the `pokerdata_*` tests; failing it keeps the chart baseline.

Analytic acceptance: facing-all-in T1 AhAd on QsJd7h3c2d versus QQ+54o weight 1/12 yields equity 0.25, W 246, call EV −11.5 unraked / −12.75 capped at 5000 mchips; 54o 0.25 yields +50 / +47.5; −2.30 bb at BB 5. Use a 1e−3 chip tolerance for exact analytic values. Polarized river: value-bet QQ 100%, bluff 54o 50 ± 3 pp, call 50 ± 3 pp, IP 75 ± 1 / OOP 25 ± 1 chips, raw exploitability <= 0.1% of pot. Check-only and non-root contract tests keep their stated 1e−3 tolerances. Plan 2 supplies these tests; `bench oracle` runs them and the gate records their actual results.

- [ ] **Step 5 (4 min): Mutation-test the gate data.** Construct a passing `GateInput`, then independently fail each bound by a small positive increment, remove a matrix row, omit a fixture, change a source hash, set `supported_numeric` false, add one final violation and censor a target time. Every mutation fails with its own named reason. Real missing files or runs cannot default to true. Exit 0 only when passed, otherwise nonzero, retaining the report.

```rust
#[test]
fn empty_presolver_queue_is_informational_only() {
    let mut input=passing_gate_input();
    input.presolver_verified_entries=0;
    let result=evaluate(&input);
    assert!(result.passed,"tier-1 coverage is progress, not a release condition");
    assert_eq!(result.informational.len(),1);
}

#[test]
fn missing_chart_ui_evidence_fails_v22() {
    let mut input=passing_gate_input();
    input.chart_ui_e2e_ok=false;             // Plan 5 Task 14 did not run, or failed
    let result=evaluate(&input);
    assert!(!result.passed);
    assert!(result.failures.iter().any(|f|f=="chart UI E2E"));
}
```

**V21/V22 split (F21).** V21 and the bench-only bounds may be reported independently from bench evidence alone. V22 **additionally** requires `chart_ui_e2e_ok`. Never claim V22 from bench-only data.

A failure reduces the versioned template size or the declared scope and reruns the affected measurements; it never loosens the definition of success. Report tier-1 progress honestly: 7,020 jobs are the four-scenario coverage condition, not a prerequisite for claiming that the scheduler implementation works.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p bench --test gate`; `cargo test -p bench --test oracle`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/bench
git commit -m 'feat(bench): add the oracle runner and the measured V21 and V22 gates' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 24: Run the deterministic regression and the flop measurement matrix

Cross-plan §5 splits the hours-long measurement task; this is batch 1. Each checklist item starts or reviews one bounded batch and appends its own rows to the report.

**Files:** Modify/append `docs/bench/2026-09-10-i7-13700K.md`, which Plan 2 Task 30 Step 4 already created; measurements are appended with their own provenance and Plan 2's R8-labelled rows are retained as explicitly separate reference evidence, never overwritten. Modify generated `bench/spots/{river_std,river_min,turn_std,turn_min,flop_fast,flop_min,sources}.json` only if deterministic generation differs before measurement. No golden is auto-updated in this task.

**Interfaces:** Consumes every preceding task. Produces the flop half of the reviewed baseline report and the V3 policy evidence.

**Expected wall-clock (review m10).** The flop matrix is `18 spots × 2 templates × 3 thread counts × 2 modes × 2 temperatures × 5 reps = 2160` solves. At the R8 A.2 rates the 12 SRP cells run 19–64 s each and the 6 3-bet cells 4.1–6.7 s, so one `--suite × --threads × --mode × --temperature` command is roughly **40–110 minutes** and the 24 commands total **one to two machine-days**. The loop is resumable: each command appends its own rows to the report and skips cells already present, so an interrupted batch is restarted with the same command line and no measurement is repeated or lost.

- [ ] **Step 1 (3 min): Run the deterministic red/green gate regression first.** Re-run Task 23's deliberately failing metric fixture; expect nonzero. Then its valid synthetic fixture; expect zero. These verify gate behaviour, not measured product success.

```powershell
cargo test -p bench --test gate
cargo test -p bench --test oracle
cargo test -p cache
cargo test -p cache --features exhaustive
cargo test -p engine --features testing,test-templates --test cache_key_structural_identity
cargo test -p engine --features testing,test-templates --test cache_snapshot_replay
cargo test -p engine --features testing,test-templates --test flop_path_golden
cargo test -p engine --features testing --test experimental_surrogate
cargo test -p engine --features testing --test presolver_engine
cargo test --workspace
python -m pytest tools/tests/test_chart_sources.py tools/tests/test_e2e_hands.py -q
python tools/gen_fixtures.py sources --check
python tools/gen_fixtures.py e2e --check
```

- [ ] **Step 2 (3 min): Build the production and diagnostic workers with the V1-selected worker toolchain and regenerate the suites.**

```powershell
$selection = Get-Content -LiteralPath 'docs/bench/worker-toolchain.json' -Raw | ConvertFrom-Json
if ($selection.worker_toolchain -eq 'stable-x86_64-pc-windows-gnu') {
    $tc = '+stable-x86_64-pc-windows-gnu'; $target = @('--target','x86_64-pc-windows-gnu')
} elseif ($selection.worker_toolchain -eq 'stable-x86_64-pc-windows-msvc') {
    $tc = '+stable-x86_64-pc-windows-msvc'; $target = @('--target','x86_64-pc-windows-msvc')
} else { throw 'Missing or invalid V1 worker toolchain selection' }
cargo $tc build --release -p solver-worker @target
cargo $tc build --release -p solver-worker --features bench-mode @target --target-dir target/bench-mode
cargo run --release -p bench -- gen-spots --suite all
git diff --stat bench/spots
```

Build production and diagnostic workers using the V1-selected worker toolchain/target; keep `bench` and the UI on MSVC. The diagnostic build uses a separate target directory and must not overwrite the production binary. Report the selected toolchain on every measured row. Regenerate all six chart suites before baseline measurements and retain prior R8 rows as explicitly separate reference evidence. Require that `ready` reports `avx2`, the expected versions and the requested thread count. The diagnostic override capability must not appear in the normal worker. Refuse any source or input that does not match the frozen hashes. No `rustup default` or config mutation, and no ordinary workspace command may rebuild a rejected worker.

Also serialize the matching `V3PolicyEvidence` (Task 9) alongside the raw V3 measurements, so production startup can load an admission decision whose provenance actually matches this run.

- [ ] **Step 3 (3 min per launch, then review each batch): Run the exact-template flop matrix.** The `--mode`, `--temperature` and `--deadline-trials` switches are declared in Task 20. Each command creates separate raw rows in the report's machine-readable block. Reps 5 in each cold/warm cell, threads 8/16/24, then the production-auto-mode deadline trials.

```powershell
foreach ($threadCount in @(8,16,24)) {
    foreach ($suiteName in @('flop_fast','flop_min')) {
        foreach ($storageMode in @('f32','i16')) {
            foreach ($temperature in @('cold','warm')) {
                cargo run --release -p bench -- run --suite $suiteName --threads $threadCount --reps 5 --mode $storageMode --temperature $temperature --out docs/bench/
                if ($LASTEXITCODE -ne 0) { throw 'Flop measurement failed; preserve the report and inspect the failed cell.' }
            }
        }
    }
}
cargo run --release -p bench -- run --suite flop_fast --threads 16 --reps 5 --mode auto --deadline-trials --out docs/bench/
cargo run --release -p bench -- run --suite flop_min --threads 16 --reps 5 --mode auto --deadline-trials --out docs/bench/
```

Do not replace missing measurements with R8 analogue numbers. Recompute V3 admission from the exact production source signatures at both depths. Record refusals and investigate if the required diagnostic matrix is incomplete; no success claim with missing rows.

- [ ] **Step 4 (3 min): Review the flop batch.** Confirm every one of the 24 cells appears in the JSON block, that the V3 admission decision for `flop_min_v1` is computed from both depths, that the f32/i16 crossover probes are present, and that no slow run was discarded.
- [ ] **Step 5 (2 min): Commit the flop half of the report.**

```powershell
git add docs/bench/2026-09-10-i7-13700K.md bench/spots
git commit -m 'test(bench): record the exact-template flop measurement matrix' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 25: Measure the baseline suites, e2e, faults, pre-solved hits and the oracles

Batch 2 of the measurement split.

**Files:** Modify `docs/bench/2026-09-10-i7-13700K.md`.

**Interfaces:** Consumes Tasks 20–23 runners and Plan 2 Task 15's `solver-worker/tests/contract_river.rs` plus Plan 2 Task 18's engine check-only terminal oracle. Produces the remaining measured evidence for V21 and V22 (UI acceptance evidence is Plan 5 Task 14's, consumed by Task 26).

**Expected wall-clock (review m10).** The four street suites are `4 × 6 spots × 5 reps` at 0.3–6 s per solve, about **20 minutes total**. The two e2e passes are `36 runnable records × 5 reps` at up to 15 s, about **90 minutes each**; the `presolved` pass additionally pre-solves the 8 supported flop spots to target, which is **3–6 hours** on this machine. The fault suite is deterministic on the fake clock and finishes in **seconds**. `bench oracle` runs the exhaustive suites and generates the 10,000,000-sample 7-card oracle on first use: allow **2–4 hours**. Every command appends and skips already-present rows, so each may be resumed.

- [ ] **Step 1 (3 min per launch, then review): Measure the four street baseline suites.**

```powershell
foreach ($suiteName in @('river_std','river_min','turn_std','turn_min')) {
    cargo run --release -p bench -- run --suite $suiteName --threads 16 --reps 5 --out docs/bench/
    if ($LASTEXITCODE -ne 0) { throw 'Baseline measurement failed; preserve the report.' }
}
```

- [ ] **Step 2 (3 min per launch, then review): Measure the cold and pre-solved e2e passes and the fault suite.**

```powershell
cargo run --release -p bench -- e2e --reps 5 --cache cold --out docs/bench/
cargo run --release -p bench -- e2e --reps 5 --cache presolved --out docs/bench/
cargo run --release -p bench -- fault --out docs/bench/
```

`--cache presolved` creates validated at-target entries with the real pre-solver executor, never fabricated files. A long suite run may outlive a shell wait; keep the progress logs and resume the same process. Stop on a failed correctness assertion; a timing failure stays reported and requires a versioned change or a scoped release decision, never a fabricated pass.

- [ ] **Step 3 (3 min per launch, then review): Run the oracle suites.**

```powershell
cargo run --release -p bench -- oracle --out docs/bench/
```

This is the only producer of `analytic_oracles_ok`. It generates the 10,000,000-sample 7-card oracle first when it is absent, checks that generation's exit code and stops on failure. A launch failure, a failing suite or a zero-test run is recorded as a failure in the report's JSON block. The `solver-worker` suite runs under the V1-selected toolchain/target with `POKERAI_WORKER` pointing at the selected binary.

- [ ] **Step 4 (4 min): Review the measured evidence.** Verify supported 22 of total 50, every prescribed fault in all required phases, zero final-delivery violations, raw versus display accuracy, the source locks, the V3 `flop_min_v1` admission choice, the mode switch points, pre-solver entry durability and status, and hit-rate denominators. Record the actual tier-1 progress; do not wait 52 h merely to test scheduler completion logic. Confirm that no cache, queue or log failure altered a delivered recommendation. Keep measured failures visible in the report.
- [ ] **Step 5 (2 min): Commit the measured evidence.**

```powershell
git add docs/bench/2026-09-10-i7-13700K.md
git commit -m 'test(bench): record baseline, e2e, fault and oracle measurements' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 26: Evaluate the release gate and freeze the reviewed baseline

Batch 3 of the measurement split: the executable V21/V22 decision.

**Files:** Modify `docs/bench/2026-09-10-i7-13700K.md`.

**Interfaces:** Consumes the complete report of Tasks 24–25, **Plan 5 Task 14's recorded `chart_ui_e2e` evidence** and `bench gate`. Produces the explicit V21/V22 outcome and is the sole final V22 decision.

**Execution order (F21).** Plan 5 Task 14 precedes this task. Plan 5 may start from supplied surfaces without waiting for this final gate; this task cannot complete without Plan 5 Task 14's actual named chart E2E result.

- [ ] **Step 1 (3 min): Record Plan 5 Task 14's chart UI evidence, then run the gate against the measured report.**

```powershell
# chart_ui_e2e_ok is set only from Plan 5 Task 14's actual named test result, with the same
# worker/source/config provenance as this release candidate. Missing evidence stays false.
cargo run --release -p bench -- gate --report docs/bench/2026-09-10-i7-13700K.md
"gate exit code: $LASTEXITCODE"
```

Exit 0 only when every §13.5 condition holds, including `chart UI E2E`. The informational pre-solver line (Task 23) is printed but never changes the exit code.

- [ ] **Step 2 (4 min): Record the V21 and V22 outcome in the report.** State each condition with its measured value, the pass/fail verdict, and — for any failure — the versioned template or scope reduction that follows, never a loosened definition. State the tier-1 coverage figure as progress. V21 and the bench-only bounds may be reported independently; V22 additionally requires the chart UI E2E evidence, and a missing or failed UI result blocks V22. Never claim V22 from bench-only data.
- [ ] **Step 3 (3 min): Run the final workspace check.**

```powershell
cargo test --workspace
git diff --check
```

- [ ] **Step 4 (2 min): Commit the reviewed report.**

```powershell
git add docs/bench/2026-09-10-i7-13700K.md bench/spots
git commit -m 'test(bench): record the release-gate evidence and the V21/V22 outcome' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

## Self-review

| Spec requirement | Tasks |
|---|---|
|§2 normalized identity, public ranges, canonical suits, ordinal actors|1–3,7–8|
|§3.2 dependency direction and process boundary|1,7,13–16,20–22|
|§4.2 flop preference default10/range1..30 and its `set_config` rejection; approval outline§0b|9–10|
|§4.4 reasons, phase/identity, `Assumptions.cache` and numericEV assembly|4,7,10–12,21–22|
|§4.5 validate_solution, output bounds, actor-owned matrices|2,5,7–8,22|
|§4.6 full materialization/terminal markers/signature|1–3,7–8,20|
|§5 step7 cache->Final/Provisional/live; flop/turn store, river excluded|7,9–10|
|§5 step10 decision-log scenario/tier, raw accuracy and violations|10,16,21,23|
|§6 coverage rules and the isolated experimental synthetic-root surrogate|4,10,11|
|§7 monotonic absolute deadlines, independent watchdog, suspend/resume|7,9–10,15,22,24–26|
|§9 cache snapshot registration, prior-street translation and prefix reuse|10–12|
|§10.1 exact templates, V3 admission and SRP/3-bet policy|9–10,20,23–26|
|§10.3 same2GiB/8GiB/headroom mode rule in live and background|10,16,20|
|§10.4 key, payload, exact labels/raw accuracy, three buckets, full topology/rake/menu checks|1–8|
|§10.4 two representatives, bounded checksummed codec, delete, atomic writer/quota|5–7|
|§10.4 T4 scale numbers and every listed identity/actor/path mutation|8,12|
|§10.5 exact scenarios/tiers/flop order, durable cursor, retries, idle/cancel, replay ranges/status|13–16|
|§12 read/write/worker/clock failures and retained evidence|5–7,10,14–16,22|
|§13.1 five named cache tests and the `bench oracle` runner for the exhaustive suites|4–8,23|
|§13.3 flop-path/budget/best-so-far/snapshot/experimental goldens|8–12,22|
|§13.5 chart locks, actual50 inputs, gen-spots for all six suites, e2e/fault/report/gate|17–26|
|§14.4 V3/V21/V22 exact baseline bounds|20–26|

**Placeholder scan:** No unresolved implementation markers. Task numbering is sequential 1–26, every task has Files/Interfaces, red/green steps and one required-trailer commit, and code fences are balanced. The four integrations the review called prose — the flop route (Task 10), `Cache::lookup` and `make_cache_query` (Task 7), the durable `Queue` (Task 14) and the `Presolver` thread (Task 15) — are now written as code in their own tasks. This review checks plan text; implementation commands have not been run while writing it. Fixtures and source locks are generated and committed before measurements and their expected outcomes are not derived from benchmark results. Storage tests use the Task 2 fixture immediately, so they never depend on later T4 materialization. Bench module exports are introduced only when their source files exist, and `crates/engine/src/bench_support.rs` is created by Plan 2 Task 5 (cross-plan M17) before Task 17 modifies it.

**Type consistency (revision 2; the revision-1 claim of exact consistency was disproved by REVIEW-cross-plan-2 F04–F14 and the listed items below are the corrections, not pre-existing properties).** `CacheEntry` stores ordinal `CachedNode.path` and `ev_over_P`; the wire `NodeStrategy` retains chip paths and `ev_chips`; `CacheHit` reconstructs query chips and inverse suits before validation and snapshot registration. `key_and_source` is the single builder shared by `make_cache_query` and `entry_from_solution`, so a stored entry and a query cannot disagree on the key. `resolve_path` is a re-export of `proto::resolve_chip_path` (cross-plan M21), so the §2 chip-path rule has one implementation. Menu types are Plan 1's resolved `SideMenu`/`MenuSize` (cross-plan M1) with `donk: None` on the root street. `StreetRootSnapshot` literals carry `bb_chips` (spec S1) and the surrogate's literal carries the hand's real `bb_chips`, never 2. `Range1326` stays public. `StreetSnapshot` is the exact `core_replay` type; `Engine::register_snapshot(&mut self, &DecisionIdentity, StreetSnapshot) -> bool` (Plan 3 Task 18) is the façade and `SnapshotStore::register` the single internal path — there is no free `engine::snapshots::register_snapshot` (cross-plan M15, F22). Foundation calls use the exact producer names: `core_iso::inverse`, `core_iso::SuitPerm::IDENTITY`, `core_model::postflop_order(Seat, &[Seat])` and `UtgStraddle.amount_chips` (F11). `engine::equity::range_vs_range` is Plan 2 Task 25's single owner returning `Option<(f32, EquityMethod)>`; this plan defines no second body (F05). The surrogate never constructs `SolveInput` and never calls `advice_rows`, which is not an upstream API (F12); Plan 2 Task 26 owns `final_from_solution`. `Templates::with_extra`/`base_ids` behind `test-templates` is Plan 2 Task 2's seam; this plan adds no registry and no `Templates::get` patch (F04). `Engine::set_config` keeps Plan 2 Task 29's validating, queuing body; this plan adds only `flop_budget_valid` and `Engine::config` (F08). `Deadlines` is Plan 2 Task 20's only deadline arithmetic (cross-plan D5). `EngineCore.flop_policy` with `V3PolicyEvidence`/`load_v3_policy` replaces the unowned `bench_p95_ms` lookup (F13). `PresolveExecutor` is a downward-dependency callback, distinct from `WorkerLink`. The `Presolver` handle lives on `Engine`, never on `EngineCore`, so status/pause/resume/notify never lock the solve owner (F09). `cache::presolver::PresolverStatus` re-exports `scheduler::PresolverStatus` and `cache::presolver::remaining_seconds` is defined in Task 15 before the scheduler uses it (F10). `PresolverStatus` derives `Serialize`/`Deserialize`/`PartialEq` and an optional `ts-rs` binding for Plan 5 (cross-plan Or7). `engine::bench_support::PreparedBenchSpot` is the engine-owned DTO; engine never imports `bench::suite::Spot` (F14). `ApproxReason` names and fields retain the spec spelling. No target/hero/request/seat/raw-chip field enters `KeyFields`. JSON metadata avoids bincode's internally tagged-enum incompatibility. Mode forcing is diagnostic-only and cannot alter the production wire or default admission.

**Coverage gaps:** No planned omission within Plan 4's scope. The actual worker build on the V1-selected toolchain, the source-bundle audit and hero support, the V3 timings, the fifty-hand outcomes, the V21/V22 pass status and tier-1 coverage remain implementation-time evidence, not claims made by this planning document. **Release completeness is not claimed from bench data alone:** V22 additionally requires Plan 5 Task 14's chart UI E2E evidence, consumed by Task 26 (F21). V9 acquisition and conversion, the exploit slice, the rest of the UI and WebDriver tests, and completed tiers 2 and 3 are intentionally owned elsewhere or deferred. Tier-1 completion is reported as the coverage condition, while resumable tier-1 execution is delivered here.

**Interfaces owed to Plan 2: none remain.** All seven notes are satisfied by the revised Plan 2 (REVIEW-cross-plan-2 §4.1), and this plan now consumes their declared shapes: `Templates::with_extra`/`base_ids` behind `test-templates` from Plan 2 Task 2 (Task 8); `Engine::set_config -> Result<u32, EngineError>` with its own validation and next-hand queuing body from Plan 2 Task 29 (Task 9 adds tests only); `engine::Paths { log_dir, worker_exe, preflop, cache }` from Plan 2 Task 29 (Task 7); `crates/engine/src/bench_support.rs` with `prepared_range`/`range_mass` created in Plan 2 Task 5 (Task 17); `crates/bench/src/lib.rs` declared in Plan 2 Task 5 (Task 20); `SolvePlan.background` as a real Plan 2 Task 22 field that Plan 2 merely never sets to `true` (Task 16, whose single-owner background execution is specified in this plan's Task 16 ownership contract); `Engine::shutdown(&mut self)` from Plan 2 Task 29 (Task 16 extends cleanup without changing the receiver).
