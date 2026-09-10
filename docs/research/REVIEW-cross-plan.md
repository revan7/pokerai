# Cross-plan consistency review (2026-09-10)

Scope: the seams between `docs/superpowers/plans/2026-09-10-plan-{1..5}-*.md` (17,427 lines, 92 tasks).
Internals of each plan are out of scope. Spec = `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` rev 5.
Reference for intent: the series brief's produce/consume map.

Result: **21 interface mismatches**, **8 orphans needing assignment** (plus 4 legitimately deferred), **10 duplicates**,
**17 accepted spec changes / 6 rejected plan deviations**.

---

## 1. Interface match

Convention: **P** = the producing plan must change, **C** = the consuming plan must change.
Rule applied: change the consumer unless the producer contradicts the spec or cannot express a spec requirement.

### M1 — `Menu`/`RaiseSize` vs `SideMenu`/`MenuSize`; bet menus cannot carry `"a"` — **P (plan 1)**
Plan 1 Task 6, Interfaces/Step 3:
> `RaiseSize::{Mult(f32), AllInOnly}` (wire: number or `"a"`), `Menu { bet: Vec<f32>, raise: Vec<RaiseSize> }`, `PlayerMenus { oop: Menu, ip: Menu, donk: Option<Vec<f32>> }`

Plan 2, "Interfaces consumed from plan 1", `proto | tree` row:
> `PlayerMenus { oop: SideMenu, ip: SideMenu, donk: Option<Vec<MenuSize>> }` … `SideMenu { bet: Vec<MenuSize>, raise: Vec<MenuSize> }`; `MenuSize::{Pot(f32), AllIn}` untagged (`0.5` / `"a"` …)

Plan 2 Task 2 Step 6 builds the production template with an all-in **in the bet list**:
> `spec("river_std_v1", River, &[(River, &[P(0.33), P(0.75), A])], &[P(2.5)], 1.5, 0.0, 3)`

Plan 4 Task 2 Step 1 uses the same names: `use proto::{…PlayerMenus,SideMenu,MenuSize};` … `SideMenu{bet:vec![MenuSize::AllIn],raise:vec![MenuSize::AllIn]}`.

Spec §10.1 `river_std_v1` bet sizes are **"0.33, 0.75 + a / 0.33, 0.75 + a"**; §4.6 opening-node rule: *"each bet size `r` gives `Bet(round(r * pot))`; an `a` entry gives `AllIn(max)`"*; §13.1 T4 requires a *"test template with `a`-only bet menus"*. `Vec<f32>` cannot represent this, so plan 1's type contradicts the spec.
**Fix:** plan 1 Task 6 renames `Menu` -> `SideMenu`, `RaiseSize` -> `MenuSize` with variants `Pot(f32)` / `AllIn`, `bet: Vec<MenuSize>`, `raise: Vec<MenuSize>`, `donk: Option<Vec<MenuSize>>`. Wire form is unchanged (`0.33` / `"a"`), so spec §4.5's `"bet":[1.0]` example still round-trips. Plan 5 Task 1 Step 3 must follow (`crate::PlayerMenus, crate::Menu, crate::RaiseSize` in the ts-rs registry and the `ts(type = r#"number | "a""#)` attribute).

### M2 — `Card::parse` / `ProtoError` do not exist — **P (plan 1, one added method)**
Plan 2 `proto | cards` row: `Card::parse(&str) -> Result<Card, ProtoError>`; used 24 times (e.g. Task 4 Step 1 `proto::Card::parse("Kh").unwrap()`, Task 21 `Card::parse("Ah")`).
Plan 1 Task 2 provides `impl FromStr for Card { type Err = CardParseError; }` only, and there is no `ProtoError` anywhere in the workspace.
**Fix:** plan 1 Task 2 adds `pub fn parse(s: &str) -> Result<Card, CardParseError> { s.parse() }` (one line, avoids 24 consumer edits); plan 2 drops `ProtoError`.

### M3 — `EngineMessage::Lock` variant shape — **C (plan 2)**
Plan 1 Task 7: `Lock { id: String, spot: String, locks: Vec<NodeLock> }` (struct variant).
Plan 2 row `proto::worker | messages`: `EngineMessage::{… Lock(LockRequest) …}` with `LockRequest { id, spot, locks: Vec<NodeLock> }`; Task 10 Step ~4 matches `EngineMessage::Lock(l) => {`.
Wire-identical under `#[serde(tag="type")]`; the Rust shapes are not.
**Fix:** plan 2 uses `EngineMessage::Lock { id, spot, locks }`; delete `LockRequest`.

### M4 — `ReadyInfo` vs `Ready` — **P (plan 1, rename)**
Plan 1 Task 7 produces `ReadyInfo`; plan 5 Task 1 registers `crate::ReadyInfo`. Plan 2 says `WorkerMessage::{Ready(Ready) …}` and `Ready { proto_version: u16, … }` (27 sites: Task 7, 14, 15 `FakeWorker::default_ready() -> Ready`, `worker::ready::validate_ready(&Ready, threads: u8)`).
The spec names only the wire tag `ready`. 27 consumer sites vs 6 producer sites.
**Fix:** plan 1 Task 7 names the struct `Ready`; plan 5 Task 1 Step 3 updates the one registry entry.

### M5 — `proto::worker::{PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}` are not produced — **P (plan 1)**
Plan 2 row `proto::worker | messages`: *"constants `PROTO_VERSION: u16 = 3`, `SOLVER_COMMIT: &str`, `ADAPTER_VERSION: u16 = 1`"*; Task 7 Consumes `proto::worker::{…, PROTO_VERSION, ADAPTER_VERSION}`, Task 14 Consumes `proto::worker::{…, PROTO_VERSION, SOLVER_COMMIT, ADAPTER_VERSION}`.
Plan 1 Task 1 defines only `proto::PROTO_VERSION` at the crate root; neither other constant exists. Plan 2 Task 7 also *produces* `solver_worker::SOLVER_COMMIT`, contradicting its own Task 14 (`ProcessWorker` cannot depend on `solver-worker`, §3.2).
**Fix:** plan 1 Task 7 adds to `proto::worker`: `pub use crate::PROTO_VERSION;`, `pub const SOLVER_COMMIT: &str = "9d1509fe5077d019825f833eed04b16d342dfda1";`, `pub const ADAPTER_VERSION: u16 = 1;`. Plan 2 Task 7 re-exports rather than defines `SOLVER_COMMIT`.

### M6 — `core_model::BeginHand` field set — **both (see also Or4)**
Plan 1 Task 11: `state::{BeginHand { hand_id, button, hero, dealt, stacks_start, hero_cards }, begin_hand(&HandConfig, BeginHand) -> Result<HandState, RulesError>}`.
Plan 2 row `core-model | state`: `begin_hand(&HandConfig, BeginHand { button: Seat, hero: Seat, hero_cards: Option<[Card; 2]>, dealt: Vec<Seat>, stacks: Vec<u32> })`; used verbatim in Task 17 Step 3 (`BeginHand { button, hero, hero_cards, dealt: …, stacks: … }`) and Task 21 `Engine::begin_hand(&mut self, req: BeginHand)`.
Plan 5 line 51: *"Plan 1's `core_model::BeginHand` contains the engine-assigned `hand_id` and `stacks_start`; it is not an IPC argument. Task 1 adds an ID-free `proto::BeginHand` admission DTO whose `stacks` are in dealt-seat order."*
Three shapes for one concept.
**Fix:** plan 1 Task 3 adds `proto::BeginHand { button, hero, dealt, stacks, hero_cards }` (matching spec §5 step 2). `core_model::BeginHand` keeps `hand_id`/`stacks_start`. Plan 2's `Engine::begin_hand` takes `proto::BeginHand`, assigns `hand_id` from `IdentityState::begin_hand()` and converts; plan 2's test helpers use `stacks_start`. Plan 5 Task 1 no longer adds the DTO.

### M7 — `RootError` variants and payload width — **C (plan 2)**
Plan 1 Task 12: `RootError::{Multiway{pot_eligible: u8}, ProjectionNotReproducing{step: u32}, NoDecision, Preflop, Inconsistent{step: u32}}`.
Plan 2 row `core-model | state`: `RootError::{Multiway, ProjectionNotReproducing{step: usize}, NoDecision}`.
**Fix:** plan 2 Task 17 uses `u32` and handles the two extra variants (`Preflop` -> `Classification::Preflop`; `Inconsistent{step}` -> `Unsupported{EngineError{retryable:false}}`, spec §10.2).

### M8 — `core-eval` equity request/result shape — **C (plan 2)**
Plan 2 row `core-eval | equity`:
> `EquityRequest { hero: Range1326 …, opponents: Vec<Range1326>, board: Vec<Card>, mode: EquityMode::{Exact, MonteCarlo{seed: u64}} }` and `EquityResult { hero_equity: Option<f32> …, method: EquityMethod, complete: bool }`

Plan 1 Task 20:
> `EquityRequest { board: Vec<Card>, players: Vec<PlayerRange>, mode: EquityMode, pots: Vec<PotEligibility> }` … `EquityResult { status, method: Option<EquityMethod>, shares: Vec<EquityShare>, samples: u64, elapsed: Duration }`, `EquityMode::{Exact, MonteCarlo { seed: u64, max_samples: u32 }}`

Every field name differs. Plan 2 Task 18 Step 3 and Task 14's `river_check_only_terminal_oracle` oracle both use the wrong shape:
> `equity(&EquityRequest { hero: fixed, opponents: vec![opp], board: board.clone(), mode: EquityMode::Exact }, …).hero_equity.unwrap()`

Plan 2 also inlines the §7 exact/MC rule instead of calling the producer's helper:
> `fn mode_for(pairs: u64, board_len: usize) -> EquityMode { … if pairs * runouts <= EXACT_LIMIT { EquityMode::Exact } else { EquityMode::MonteCarlo { seed: 7 } } }`

whereas plan 1's handover table says *"The engine (plan 2) chooses `EquityMode::Exact` when `exact_cost(&req) <= 20_000_000`"*.
**Fix:** plan 2 Task 18 and Task 14 rewrite against plan 1's shape, add `EquityMode::MonteCarlo{seed, max_samples}`, read `EquityResult.shares`/`status`, and replace `mode_for` with `core_eval::exact_cost`. Plan 2 already scopes this ("Only `crates/engine/src/equity.rs` touches this API") but the Task 14 oracle is a second site.

### M9 — `StreetRootSnapshot` literals omit `bb_chips` — **C (plan 2)**
Plan 1 Task 3: *"`StreetRootSnapshot` (spec fields plus `bb_chips: u32`, needed by `replay_root` for the minimum bet; the worker ignores it)"*.
Plan 2 Task 4 Step 1 and Task 16 Step 1 build it literally without the field:
> `StreetRootSnapshot { street: Street::Flop, board: …, oop: Seat(2), ip: Seat(0), pot_root: pot, stack_oop_root: oop_stack, stack_ip_root: ip_stack, dead_this_street: 0, projected_from: 2, history }`

**Fix:** both helpers add `bb_chips: 2` (or the fixture's blind).

### M10 — `Engine::set_config` return type — **P (plan 2)**
Plan 2 Task 21: `set_config(&mut self, GameConfig) -> u32`.
Plan 5 "Required engine surface": `Engine::set_config(&mut self, config: GameConfig) -> Result<u32, EngineError>;`
Plan 4 Task 9 Interfaces: *"Extend existing `set_config(GameConfig)`"*, and its `flop_budget_setting_golden` must satisfy spec §13.3: *"31 is rejected by `set_config`"*.
**Fix:** plan 2 Task 21 returns `Result<u32, EngineError>` (a `u32` return cannot reject).

### M11 — `Engine::shutdown` receiver — **P (plan 2)**
Plan 2 Task 21: `shutdown(self)`. Plan 5: `Engine::shutdown(&mut self);` and Task 3 Interfaces "Consumes `Engine::shutdown()`" from inside Tauri managed state, which cannot move out of the handle.
**Fix:** plan 2 uses `shutdown(&mut self)` with an idempotent flag (plan 5 Task 3 requires "once-only shutdown").

### M12 — `recommend` sink type — **C (plan 5)**
Plan 2 Task 21: `recommend(&mut self, sink: Box<dyn EventSink>)`, with `pub trait EventSink: Send` (Task 14).
Plan 5: `Engine::recommend(&mut self, sink: Box<dyn EventSink + Send>)`. `dyn EventSink` and `dyn EventSink + Send` are distinct types in Rust even when `Send` is a supertrait.
**Fix:** plan 5 Tasks 2-3 use `Box<dyn EventSink>`.

### M13 — `engine::Paths` layout — **P (plan 2)**
Plan 2 Task 21 Step 3: `pub struct Paths { pub log_dir: PathBuf, pub worker_exe: PathBuf }`.
Plan 5: `Paths { worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf }` and Task 3 Step 4 constructs exactly those four fields.
Plan 3 Task 14 Step 3 only says *"add a packaged-chart fallback directory to its existing Paths initializer if not already present"*; plan 4 never touches `Paths` although it owns the cache directory.
**Fix:** plan 2 Task 21 defines all four fields up front (`worker`, `preflop`, `cache`, `log`); plan 3 populates `preflop`, plan 4 reads `cache` in `Cache::open`.

### M14 — `Engine::set_hero_cards` has no producer — **P (plan 2)**
Spec §3.5 lists `set_hero_cards` as a Tauri command; plan 5 registers it (`command!(set_hero_cards, HandState, Op::Hero(cards), cards: [Card; 2]);`, `Op::Hero(c) => encode_hand(e.set_hero_cards(c)…)`).
Plan 2 Task 21's `Engine` API has `begin_hand, apply_action, set_board, undo, set_explicit_ranges, recommend, cancel, finish_hand, abandon_hand, shutdown, state` — no `set_hero_cards`. Plan 1 has only the free function `core_model::set_hero_cards(&HandState, [Card;2])`.
**Fix:** plan 2 Task 21 adds `set_hero_cards(&mut self, [Card; 2]) -> Result<HandState, EngineError>` (mutation -> `IdentityState::mutate()`, spec §5 step 3).

### M15 — `SnapshotStore` / snapshot record field names — **C (plan 3), with a rename in plan 2**
Plan 2 Task 21: `snapshots::{SolvedStreet { identity_at_solve, street, board, tree, nodes, ordinal_paths, exploitability_chips, reasons, solved_prefix }, SnapshotStore::{new(), register(&mut self, active: &DecisionIdentity, s: SolvedStreet) -> bool, invalidate_hand(&mut self, hand_id: u64), for_hand(&self, hand_id) -> Vec<&SolvedStreet>}}` … *"(plan 3 wraps `SolvedStreet` into `StreetSnapshot` and adds `Engine::register_snapshot`)"*.
Plan 3 Task 11 instead produces a second store: `SnapshotStore::new()->Self`, `register(&mut self, active:&DecisionIdentity, snapshot:StreetSnapshot)->bool`, `invalidate(&mut self, state:&HandState)`, `for_identity(&self, identity:&DecisionIdentity)->Vec<StreetSnapshot>` in `crates/core-replay/src/snapshot.rs`, and does not wrap anything.
Spec §9.1 field name is `covered_paths`, not `ordinal_paths`; spec §9.2 invalidation is prefix-based, not hand-based.
**Fix:** plan 2 keeps a temporary `engine::snapshots::SolvedStreetStore` for its own river/turn path; plan 3 Task 11 explicitly *replaces* it with `core_replay::SnapshotStore` (`covered_paths`, `invalidate(&HandState)`) and `engine::snapshots` re-exports. Plan 4's file structure row *"`crates/engine/src/snapshots.rs` | Existing single registration path"* then resolves.

### M16 — root-range provider: `RangeSource` vs `prepare_root` — **C (plan 3)**
Plan 2 Task 21: `ranges::{RootRanges {…}, RangeSource: Send { fn ranges_at_root(&self, state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>; }, ExplicitRanges {…}}`, wired as `core.range_source`.
Plan 3 Task 15 produces a parallel entry point: `prepare_root(state:&HandState,store:&PreflopStore,snapshots:&[StreetSnapshot],tree:EffectiveTree,target_bp:u16)->Result<(SolveInput,ReplayOutput),UnsupportedReason>` plus a new module `crates/engine/src/postflop.rs` ("Replace initial range provider in Plan 2 turn/river path").
**Fix:** plan 3 implements `RangeSource` for a `ReplayRanges` type and installs it into `EngineCore.range_source`; `prepare_root` becomes its body. No `postflop.rs` — `serve.rs` already owns the path.

### M17 — `bench` dependency direction — **C (plan 2)**
Spec §3.2: `bench` depends on `proto`, `engine`. Plan 4 Global Constraints repeats it (*"Bench: `proto`, `engine`"*) and adds `crates/engine/src/bench_support.rs` as the facade.
Plan 2 Task 22 Interfaces: *"Consumes … `core_ranges::{parse_range, block_public}`, `suite::{Spot, Suite}`"*.
**Fix:** plan 2 Task 22 routes range parsing through an `engine::bench_support` helper (or plan 4's facade is created early in plan 2 Task 5).

### M18 — dependency versions disagree across three plans — **P (plan 1 workspace) + C (plans 2, 4)**
Plan 1 Task 1: `thiserror = "2.0"`, `sha2 = "0.11"`; *"crates.io versions: … thiserror 2.0.20, sha2 0.11.0"*.
Plan 2 Task 2 `crates/engine/Cargo.toml`: `sha2 = "0.10"`, `thiserror = "1"`, `serde = { version = "1", … }` (no `workspace = true`).
Plan 4 Task 1: `sha2 = "=0.10.9"`, and its verification line *"Verified 2026-09-10: … sha2 0.10.9 … thiserror 2.0.17"*.
Two `sha2` majors and two `thiserror` majors in one workspace; `hash_scaled` (core-ranges, 0.11) and the cache key digest (0.10.9) would use different `Digest` traits.
**Fix:** pin one line in `[workspace.dependencies]` (recommend `sha2 = "0.10.9"`, the version plan 4 actually verified) and make plans 2 and 4 use `sha2.workspace = true`, `thiserror.workspace = true`, `serde.workspace = true`. Also give `crates/engine` and `crates/cache` `version.workspace = true` / `edition.workspace = true` / `license.workspace = true` like plan 1's crates.

### M19 — toolchain — **C (plan 2)**
Plan 1 Task 1 writes `rust-toolchain.toml` with `channel = "stable-x86_64-pc-windows-msvc"` (repo-wide override).
Plan 2 Tech Stack: *"Rust 2021 (stable 1.95, GNU toolchain for this plan)"*.
Plans 3, 4, 5 all state the opposite (plan 3: *"Plan 1 pins `stable-x86_64-pc-windows-msvc` in `rust-toolchain.toml`; use ordinary `cargo` commands, without a GNU override"*).
**Fix:** plan 2 drops the GNU statement. `.cargo/config.toml` already carries `+avx2` for both targets, so the GNU fallback of spec §3.6 remains available without a second toolchain file.

### M20 — `Templates::ids().len() == 9` vs plan 4's test templates — **C (plan 4)**
Plan 2 Task 2 Step 5: `assert_eq!(Templates::ids().len(), 9);`
Plan 4 Task 8 Step 2: *"Register only test templates `menu_round_test_v1` and `check_jam_test_v1` in the test harness, not the production selector"*, and Task 8 Step 4 adds `check_only_test_v1`.
**Fix:** keep them out of `Templates` (plan 4's own instruction) — or, if a shared registry is needed for `build_effective_tree`, relax plan 2's assertion to `>= 9` and assert the seven §10.1 ids explicitly.

### M21 — `resolve_chip_path` implemented three times — **C (plans 2, 4)**
Plan 1 Task 6: `proto::resolve_chip_path(materialized: &[MaterializedNode], path: &[Action]) -> Option<OrdinalPath>` (spec §2 rule, used by `validate_solution`).
Plan 2 Task 4 produces `tree::resolve_chip_path(&[MaterializedNode], &[Action]) -> Option<OrdinalPath>`.
Plan 4 Task 2 produces `resolve_path(&[MaterializedNode],&[Action])->Option<OrdinalPath>`; plan 3 Task 11 produces `resolve_path(tree:&EffectiveTree, chips:&[Action])->Option<OrdinalPath>`.
Four names for one normative rule; a divergence silently breaks cache/replay/worker agreement.
**Fix:** all three consumers re-export `proto::resolve_chip_path` (plan 3's takes `&tree.materialized`).

*Verified consistent (no action):* `validate_solution(&StreetSolution, &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>` (plan 1 T8 = plan 2 row = plan 3 line 97 = plan 4 line 55); `build_effective_tree(&StreetRootSnapshot, &TemplateSelection) -> Result<EffectiveTree, UnsupportedReason>` (plan 2 T4 = plan 4 line 45); `replay(ReplayInput) -> ReplayOutput`, `PreflopStore::query`, `StreetSnapshot`/`SnapshotKey`/`SnapshotProvenance` (plan 3 = plan 4 T11); `canonicalize`, `hash_scaled`, `block_public`, `orbit_size` (plan 1 = plan 4); `WorkerLink`, `FakeClock`/`FakeWorker`/`FakeReply` and the `testing` feature (plan 2 T14/T15 = plan 4 T19); the serde witnesses in plan 5 line 80 (`allin` vs `all_in`, `phase` tag, PascalCase reason `kind`, `Seat.0` indexing) all match plan 1 Tasks 3/5.

---

## 2. Duplicates and orphans

### Duplicates (two plans create the same file, type or function)

| # | Item | Plans | Resolution |
|---|---|---|---|
| D1 | `SnapshotStore` + snapshot record | 2 T21 (`engine::snapshots::SolvedStreet`), 3 T11 (`core_replay::SnapshotStore`/`StreetSnapshot`) | See M15: plan 2's is a temporary `SolvedStreetStore`, deleted by plan 3 T11; only `core_replay::StreetSnapshot` survives (spec §9.1) |
| D2 | chip-path resolution | 1 T6, 2 T4, 3 T11, 4 T2 | See M21: single implementation in `proto` |
| D3 | `node_at` | 2 T4 (`node_at(&[MaterializedNode], &[u8])`), 3 T12 (`node_at(&StreetSnapshot, &[u8]) -> Option<&NodeStrategy>`) | Different crates and semantics; rename plan 3's to `snapshot_node_at` |
| D4 | `bench` CLI, `report.rs`, `gen_spots.rs`, `main.rs` | 2 T5/T22 (create), 4 T17/T20 (create+modify) | Plan 4 modifies plan 2's files; plan 4's file-structure rows for `report.rs`/`main.rs`/`gen_spots.rs` must be marked *modify*, and `crates/bench/src/lib.rs` (plan 4) added to plan 2 T5 so integration tests can link |
| D5 | flop deadline arithmetic | 2 T16 (`Deadlines::for_request(t0_ms, street, flop_budget_s)`), 4 T9 (`flop_deadlines(u64,u8) -> Result<(u64,u64), UnsupportedReason>`) | Plan 4 extends `Deadlines`, does not add a parallel function (its own Interfaces line already says "Extend existing … `Deadlines` without resetting t0") |
| D6 | root-range provider | 2 T21 (`RangeSource`), 3 T15 (`prepare_root`, `postflop.rs`) | See M16 |
| D7 | two-combo river range definition | 1 T7 (`river_two_combo_ranges()` in `crates/proto/tests/wire_examples.rs`), 2 T6 (`tools/gen_worker_fixtures.py`) | Plan 1 self-review item 5 names the Rust test file as the definition, but Python cannot import it. Make `tools/gen_worker_fixtures.py` the single source and have `wire_examples.rs` load `fixtures/worker/river_two_combo.jsonl` — or add a cross-check test that the two agree combo-by-combo |
| D8 | `engine::assemble` headline logic | 2 T19 (`headline`), 3 T14 (`set_headline(actions, unresolved_mass, source, unverified)`) | Plan 3 must extend `assemble::headline`, not add a second entry point (spec §4.4 has one headline rule) |
| D9 | root `Cargo.toml` members list | 1 T1 (`members = ["crates/*"]`), 2 T1 (adds `"crates/engine"`, `"crates/bench"`, `"solver-worker"`) | `crates/*` already covers engine/bench; plan 2 T1 adds only `"solver-worker"` and `exclude = ["third_party/postflop-solver"]` |
| D10 | test templates in the registry | 2 T2 (9 ids incl. `facing_test_v1`, `river_oracle_v1`), 4 T8 (`menu_round_test_v1`, `check_jam_test_v1`, `check_only_test_v1`) | See M20 |

### Orphans (used or required, produced by nobody) — assignments

| # | Orphan | Evidence | Assign to |
|---|---|---|---|
| Or1 | §6 `experimental` surrogate and `experimental_surrogate_golden` | Plan 2 self-review 6: *"The §6 experimental surrogate (`experimental_surrogate_golden`) is in no plan of the series map; it belongs with the flop path (plan 4)"*. Plan 4 T10 Step 4a says only *"Extend the **existing** multiway experimental branch to flop templates"* — nothing exists to extend | **Plan 4 Task 10, new step 4a**: *create* the surrogate for river, turn and flop (§6 contract: highest range-vs-range-equity opponent, total pot, min stack, empty history, street-root unconditioned ranges, hero role by postflop order, isolated from `SolveInput`/cache/snapshots) and add `experimental_surrogate_golden` |
| Or2 | `bench oracle` subcommand | Spec §13.1 header: *"exhaustive suites behind `--features exhaustive`, run by `bench oracle` before release"*; plan 1 "Running everything": *"`bench oracle` of plan 2 wraps this"*. Plan 2's CLI is `run` / `gen-spots` / `materialize`; plan 4 adds `gate` / `e2e` / `fault` | **Plan 4 Task 20**, alongside `bench gate` (it generates the 10M oracle then runs `cargo test -p core-eval --features exhaustive`) |
| Or3 | `Engine::set_hero_cards` | M14 | **Plan 2 Task 21** |
| Or4 | `proto::BeginHand` admission DTO | M6; plan 5 T1 adds it, but plan 2 T21 needs it first | **Plan 1 Task 3** |
| Or5 | `engine::Paths.preflop` and `.cache` | M13 | **Plan 2 Task 21** (declare); plan 3 T14 populates `preflop`, plan 4 T6 reads `cache` |
| Or6 | `proto::worker::{SOLVER_COMMIT, ADAPTER_VERSION}` and `worker::PROTO_VERSION` | M5 | **Plan 1 Task 7** |
| Or7 | `PresolverStatus` serialization | Plan 5 line 51: *"Plan 4's status is a Rust struct without a Serialize derive; Task 2 explicitly projects its documented fields to JSON"*; plan 5 T4: *"Status is `unknown` because Plan 4 owns its structure"* — the UI then has no typed contract for a spec §3.5 command | **Plan 4 Task 14**: derive `Serialize`/`Deserialize` on `PresolverStatus` (cache depends only on proto/core-iso/core-ranges, so it is free to) and add it to plan 5's ts-rs registry |
| Or8 | chart-replay ranges for the river/turn baseline suites | Spec §13.5: *"**baseline set** with ranges from the chart bundles' replay"* for all six suites. Plan 2 T5 generates `--source r8` (uniform); plan 2 self-review 8 says the chart set *"arrives with plan 3"*; plan 3 self-review: *"No benchmark or UI artifacts are generated in this plan"*; plan 4 T17 regenerates flop suites only | **Plan 4 Task 17**: regenerate all six `bench/spots/*.json` from `generate_flop_spots`-style chart replay, not just `flop_fast`/`flop_min` |

### Deliberately deferred (spec permits; no assignment needed)
- `pokers` crate (spec §3.2 `core-eval` row) — dropped, see spec edit S4.
- `tools/pokerdata_convert.py`, `fixtures/preflop/pokerdata_sample/*` — spec §13.0 marks them *"conditional on V9"*; plan 3 excludes them explicitly.
- §11 exploit slice / `ExploitAdvice` — spec §11 is "OPTIONAL in phase 1"; `proto::ExploitAdvice` is produced (plan 1 T5) and rendered as `null` (plan 5), which is the intended stub.
- 10,000,000-sample 7-card oracle file — gitignored, `--features exhaustive` only (plan 1 dev 6, spec §13.1 header). Its runner is Or2.
- `river_check_only_terminal_oracle` in the engine test crate — assigned (plan 2 T14) and covered by spec edit S8.

---

## 3. Spec changes proposed by the plans

### ACCEPT (17) — exact spec edits to make

| # | Source | Spec edit |
|---|---|---|
| S1 | Plan 1 dev 1 | §4.3 `StreetRootSnapshot`: add `pub bb_chips: u32, // minimum bet for replay_root; ignored by the worker`. *Reason:* `replay_root` cannot reproduce the legal set of an unopened street without it. |
| S2 | Plan 1 dev 2 | §3.5 `core-model` row: `RootError::{Multiway{pot_eligible}, ProjectionNotReproducing{step}, NoDecision, Preflop, Inconsistent{step}}`. *Reason:* the spec's three variants cannot express "preflop" or "a genuine HU root that does not replay", which §10.2 requires to be `EngineError`. |
| S3 | Plan 1 dev 3 | §3.5 `core-eval` row and §7 equity row: `EquityMode::MonteCarlo{seed, max_samples}`; add `exact_cost(&EquityRequest) -> u64` and state the engine applies `exact_cost <= 2 * 10^7`. *Reason:* the §7 rule needs a computable cost; MC needs a sample cap for the budget test. |
| S4 | Plan 1 dev 4 | §3.2 `core-eval` row: replace *"weighted range-vs-range equity via `pokers` (MIT)"* with *"weighted range-vs-range equity in-house over the `Evaluator` trait"* and add the reason. *Reason:* measured — `pokers 0.10` stores weights as `u8` percent and cannot represent `Range1326`. |
| S5 | Plan 1 dev 7 | §13.1 `state_machine_pokerkit_fixtures` row: record the two generator normalizations (straddle minimum open `2S`; fold-out survivor bet split into matched + refund) and that seeds where PokerKit's reopening rule diverges from §4.3 are dropped (1 in 201). *Reason:* the oracle is not bit-identical to the spec rules; the difference must be contractual, not folklore. |
| S6 | Plan 1 dev 9 | §4.3 after `Derived`: *"Per-seat vectors have length 6 and are indexed by `Seat.0`; an undealt seat has `folded = true`, `all_in = false` and zeros elsewhere. `HandState.stacks_start` is in `dealt` order."* *Reason:* plan 5 line 80 depends on exactly this split convention; it is currently undocumented. |
| S7 | Plan 2 dev 1 | §13.2 `ev_convention_non_root_payoffs`: replace OOP `AA` with OOP `QQ,66`. *Reason:* on `Qs Jd 7h 3c 2d` three queens beat aces, so `AA` cannot have equity 1; `QQ,66` reproduces the stated `+200` / `-50` exactly through card removal. |
| S8 | Plan 2 dev 2 | §13.2 `river_check_only_terminal_oracle`: add *"runs in `crates/engine/tests/worker_link.rs` (still spawning the worker binary) because its oracle is `core-eval`, which `solver-worker` may not depend on (§3.2)"*. |
| S9 | Plan 2 dev 3 | §13.2 `ev_conservation`: add *"checked in-process through the adapter's mapping; IP's root-range EV is not an actor-owned wire export"*. |
| S10 | Plan 2 dev 4 | §13.2 `tree_materialization_matches_library`: split into (a) in-process equality over the 47 boundary cases, (b) the wire `tree_mismatch` cases. |
| S11 | Plan 3 self-review, type check | §4.4 `ApproxReason::AsymmetricStacks { stacks_bb: Vec<f32>, prominent: bool }`. *Reason:* §8.3 mandates *"`prominent` when some difference exceeds 5% of the depth used"*, but the §4.4 sketch has no field to carry it; plan 3's workaround (assumptions note) loses the flag the UI renders. |
| S12 | Plan 3 conflict 1 | §13.1 T3 `replay_bayes_two_combos`: replace *"posterior (0.9, 0.1) for every seat including hero"* with §8.4's wording — one-element branch posterior `pi_{S,0}[c] == 1`; the **actor's** normalized combo distribution is `(0.9, 0.1)`; an unacted seat stays uniform. *Reason:* the two literal assertions cannot both hold; §8.4's formulas are binding. |
| S13 | Plan 3 conflict 2 | §9.2, board blocking: add *"board blocking is applied to each seat's output marginal at the street root, never to per-branch masses, so §8.4's equal-total invariant and the seat-independent residual share are preserved."* *Reason:* pointwise per-branch removal breaks the invariant that §8.4 and the `BranchResidual` percentage depend on; this is the minimal rule that keeps both. |
| S14 | Plan 3 conflict 3 | §9.2 case 3: add *"a branch whose mapped continuation is not covered freezes navigation for that branch for the remainder of the street; later observed actions in other branches are unaffected."* *Reason:* case 2 and the root-only export fixture allow a later wager outside the skeleton, which case 3's "all next ordinals remain defined" contradicts. |
| S15 | Plan 5 | §3.5 `engine` row: `set_config(GameConfig) -> Result<config_revision, EngineError>`; add `set_hero_cards([Card; 2]) -> Result<HandState, EngineError>`; `shutdown(&mut self)`. *Reason:* §13.3 requires `set_config` to reject `flop_budget_s = 31`; §3.5's own Tauri row already lists `set_hero_cards`; Tauri managed state cannot consume the handle. |
| S16 | Plan 5 | §4.3: add `pub struct BeginHand { button: Seat, hero: Seat, dealt: Vec<Seat>, stacks: Vec<u32>, hero_cards: Option<[Card;2]> }` as the admission DTO of §5 step 2, distinct from `core-model`'s internal input which carries the engine-assigned `hand_id`. |
| S17 | Plan 1 dev 6 | §13.0: mark the 10,000,000-sample 7-card oracle as locally generated and gitignored (only the 200k file is committed). |

### REJECT (6) — the plan must follow the spec

| # | Deviation | Why rejected |
|---|---|---|
| R1 | Plan 2 Tech Stack: *"stable 1.95, GNU toolchain for this plan"* | Plan 1 T1 pins `stable-x86_64-pc-windows-msvc` repo-wide in `rust-toolchain.toml`, and plans 3-5 all assume it. A per-plan toolchain would silently change the `.cargo/config.toml` target and the built `solver-worker.exe` that plan 5 stages. |
| R2 | Plan 2 self-review 7: *"The engine always sends `background: false`"* | §10.5 requires presolver jobs to be `solve{background: true, deadline_ms: 600000}`; plan 4 T14/T15 depend on it. `SolvePlan.background` already exists — state it as "plan 2 exercises only `false`", not as an engine invariant. |
| R3 | Plan 2 self-review 8: *"the chart-replay baseline set (`--source file`) arrives with plan 3"* | Plan 3's self-review: *"No benchmark or UI artifacts are generated in this plan"*. §13.5 requires chart-replay ranges on all six baseline suites. Reassign to plan 4 T17 (Or8). |
| R4 | Plan 4 T10 step 4a: *"Extend the **existing** multiway experimental branch"* | No plan creates it (plan 2 self-review 6 says so explicitly). Plan 4 must create the §6 surrogate for all three streets, not extend a non-existent branch (Or1). |
| R5 | Plan 2 T22 consuming `core_ranges::{parse_range, block_public}` | §3.2 fixes `bench`'s dependencies to `proto`, `engine`. Route through `engine::bench_support` (M17). |
| R6 | Plan 2 T2 `sha2 = "0.10"`, `thiserror = "1"`, non-workspace `serde` | §3.2's single workspace and plan 1's `[workspace.dependencies]`; two `sha2` majors would give `hash_scaled` and the cache key two different `Digest` traits (M18). |

---

## 4. Execution order

Notation `P<plan>.T<task>`. Anything on the same row may run in parallel. `cargo test --workspace` stays green
after every row because each plan's tasks are already ordered internally and no row depends on a later row.

**Phase 0 — the interface fixes above must be applied to the plan documents before execution starts.**
M1, M4, M5, M6, Or4, Or6, S1-S6, S11, S16 change plan 1 tasks that are the very first thing executed; applying
them later means re-cutting `proto` after five crates depend on it.

| Row | Tasks | Notes |
|---|---|---|
| 1 | **P1.T1** | Workspace, toolchain, `+avx2`, `proto` skeleton. Blocks everything. |
| 2 | P1.T2 · P1.T13 · P1.T18 · **P2.T1** | Three independent lanes open at once: proto cards; Python `tools/` + PokerKit fixtures; phevaluator oracle; vendored solver (needs only the root manifest). |
| 3 | P1.T3 (+`proto::BeginHand`) · P1.T4 · P1.T19 | `core-eval` lane starts after T18. |
| 4 | P1.T5 · P1.T6 (menu types, M1) · P1.T15 · P1.T20 | |
| 5 | P1.T7 (+`SOLVER_COMMIT`/`ADAPTER_VERSION`) · P1.T9 · P1.T16 · P1.T21 | |
| 6 | P1.T8 · P1.T10 · P1.T17 · P1.T14 | T14 needs T11+T13 — schedule after T11 lands; T17 needs T15. |
| 7 | P1.T11 · **P2.T2** | Engine skeleton needs only `proto` tree types (P1.T6). |
| 8 | P1.T12 · P2.T3 · **P2.T7** | Worker skeleton needs `proto::worker` (P1.T8) + vendor (P2.T1). |
| 9 | P2.T4 (needs P1.T8 + P1.T12) · P2.T8 | |
| 10 | P2.T5 · P2.T9 | |
| 11 | P2.T6 (Python, needs `bench materialize`) · P2.T10 | |
| 12 | P2.T11 · P2.T12 · P2.T13 · **P3.T1** | Plan 3's `core-preflop` lane needs only plan 1; it can start here and run beside the whole worker/engine lane. |
| 13 | P2.T14 (needs P1.T20 for the oracle) · P3.T2 · **P3.T3** | P3.T3/T4 (chart transcription) is externally blocked on PDF fetch — start it as early as possible. |
| 14 | P2.T15 · P2.T17 · P3.T4 (long) · P3.T5 | P2.T17 needs only `core-model`. |
| 15 | P2.T16 · P2.T18 (needs P1.T20/T21) · P3.T6 · P3.T7 | |
| 16 | P2.T19 · P2.T20 · P3.T8 · P3.T9 | |
| 17 | **P2.T21** (Engine API) · P3.T10 · P3.T11 · P3.T12 | |
| 18 | P2.T22 · P3.T13 · **P4.T1** | Cache key/payload lane needs only `proto` + `core-iso`/`core-ranges`. |
| 19 | **P3.T14** (needs P2.T21) · P4.T2 · P4.T3 | |
| 20 | P3.T15 · P4.T4 · P4.T5 · **P5.T1** | Plan 5 Task 1 (ts-rs bindings) needs only `proto`; start it here so the MSVC/Tauri toolchain risk surfaces early. |
| 21 | P3.T16 · P4.T6 · P4.T7 · P5.T2 (needs P2.T21 + M10-M14 fixes) | |
| 22 | P4.T8 (needs P2.T4 materializer) · P4.T9 · P5.T3 · P5.T4 | |
| 23 | P4.T10 (+ create `experimental`, Or1) · P4.T12 · P5.T5 · P5.T6 | |
| 24 | P4.T11 (needs P3.T15) · P4.T13 · P5.T7 · P5.T8 | |
| 25 | P4.T14 (+`PresolverStatus: Serialize`) · P4.T16 (long: 50 fixtures) · P5.T9 · P5.T10 | |
| 26 | P4.T15 · P4.T17 (all six suites, Or8) · P5.T11 | |
| 27 | P4.T18 · P4.T19 | |
| 28 | P4.T20 (+ `bench oracle`, Or2) | |
| 29 | **P5.T12** (E2E; needs P4.T10 flop path + P3 charts) | |
| 30 | **P4.T21** (V3/V21/V22 measurement matrix and release gate) | Must be last: it gates on the E2E result. |

Strictly sequential chokepoints: P1.T1 -> P1.T3 -> P1.T6/T8 -> P2.T2/T7 -> P2.T10 -> P2.T14 -> P2.T16 ->
**P2.T21** -> P3.T14/T15 -> P4.T9/T10 -> P4.T17/T18 -> P5.T12 -> P4.T21.

Green-workspace risks in this order:
- P2.T21 changes `EngineCore::new` arity (plan 2 self-review notes T20 adds `log`); P4 and P5 both construct it. Freeze the four-argument form at P2.T20 and update P2's `solve_client.rs` rig in the same task.
- P3.T11 deletes `engine::snapshots::SolvedStreetStore` (M15). Do it inside P3.T11, not spread over T11-T15, or plan 2's `identity_race_golden` (which asserts `core.snapshots.lock().unwrap().for_hand(...)`) goes red between tasks.
- P5.T1 adds `ts-rs` derives to every `proto` file. Gate it behind `feature = "typescript"` (plan 5 already does) so `cargo test --workspace` for plans 2-4 is unaffected.

---

## 5. Total effort

| Plan | Tasks | Checkbox steps | Longest task (lines) |
|---|---|---|---|
| 1 Foundation | 21 | 105 | T11 lifecycle/settlement/state (458) |
| 2 Worker + engine | 22 | 90 | T21 Engine API + 2 goldens (462) |
| 3 Preflop + replay | 16 | 92 | T4 chart transcription (83, but manual) |
| 4 Flop/cache/presolver | 21 | 142 | T16 chart lock + 50 e2e hands (236) |
| 5 UI + E2E | 12 | 69 | T2 commands + IPC contract tests (447) |
| **Total** | **92** | **498** | |

**Critical path ≈ 54 of the 92 tasks** (P1 ~7 deep after the fan-out, P2 ~10, P3 ~12, P4 ~13, P5 ~12).
About 38 tasks sit off the path (the Python lane, `core-ranges`/`core-iso`/`core-eval`, the whole cache
key/payload lane P4.T1-T8, and P5.T1-T11 once the engine façade is frozen), so a 3-4 lane parallel execution
recovers roughly 40% of the wall time.

### Tasks too large for one review-gated unit — split before execution

| Task | Why | Suggested split |
|---|---|---|
| P2.T21 (462 lines, 4 new modules + 2 §13.3 goldens) | `snapshots.rs`, `ranges.rs`, `serve.rs`, `engine.rs`, `identity_race_golden`, `final_delivery_independent_of_worker` in one commit | (a) `snapshots` + `ranges`; (b) `serve::serve_request` + `identity_race_golden`; (c) `Engine` public API + `final_delivery_independent_of_worker` |
| P2.T16 (421 lines) | `deadline.rs` + `watchdog.rs` + `EngineCore` + `run_solve` (admission, heartbeat, cancel/kill, `_min` retry) | (a) `deadline` + `watchdog`; (b) `EngineCore` + `run_solve` happy path; (c) heartbeat / cancel-kill / retry |
| P2.T10 (419 lines) | three threads, full state machine, cancel, shutdown, lock staging | (a) `writer` + `read_line`/line limits; (b) state machine + `ack` rules; (c) cancel/shutdown/EOF + lock staging |
| P2.T9 (376 lines, 5 modules) | `memory`, `solve_loop`, `extract`, `locks`, `job` at once | (a) `memory` + `locks`; (b) `solve_loop` (§7 stop rule); (c) `extract` + `job` |
| P1.T11 (458 lines, 3 modules) | `settlement.rs` + `lifecycle.rs` + `state.rs` with 4 §13.1 tests | (a) `settlement` (refund/layer/conservation); (b) `lifecycle::simulate`; (c) `state` public API |
| P1.T13 (430 lines) | Python project + generator + 200 committed fixtures | (a) `pyproject`/venv/conftest; (b) `gen_fixtures.py` + pytest; (c) generate and commit fixtures |
| P5.T2 (447 lines) | 14 commands, `Service` dispatcher, `EnginePort` seam, MockRuntime IPC tests | (a) `Service`/`Op`/`EnginePort` + dispatcher tests; (b) the 14 commands + argument tests; (c) `Channel<RecommendationEvent>` forwarding |
| P4.T16 | freezing chart provenance **and** authoring 50 recorded hands with expected coverage classes | (a) chart source/manifest lock; (b) `e2e_hands.py` + inventory tests; (c) generate + review the 50 records |
| P3.T4 | manual transcription and verification of two PDFs (169 cells per grid, externally blocked) | one commit per chart page/grid, with the inventory checklist updated per commit |
| P4.T21 | hours-long measurement matrix (6 spots × 3 boards × 2 modes × 3 thread counts × cold/warm) | already flagged in the plan as batched; make each batch its own checklist item with its own report append |

Also worth noting: P4 has 142 steps across 21 tasks (6.8 steps/task) with several steps written as multi-part
paragraphs ("Step 4 (5 min): Complete the rig, raw-fraction golden and **all T4 mutation cases**"); those exceed
the 2-5 minute rule even though the task count looks reasonable.
