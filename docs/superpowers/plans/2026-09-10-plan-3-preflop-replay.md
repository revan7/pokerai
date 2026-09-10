# Plan 3: Preflop charts and replay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the chart-backed preflop decision path and public-range replay, including shared translated-history branches and reusable prior-street snapshots, and feed those ranges into the existing turn and river solves.

**Architecture:** `core-preflop` owns normalized sources, prefix-specific lookup, EV normalization, translation, and the shared branch arithmetic; `core-replay` re-exports the shared branch types and orchestrates preflop and completed-street replay. `engine` owns identity, snapshot registration, and recommendation delivery, consuming the Plan 1 model/range APIs and the Plan 2 street-root solver without linking the worker. Charts and synthetic EV fixtures use the same validated envelope, so enabling an authorized EV-bearing bundle requires data installation rather than a new execution path.

**Tech Stack:** Rust edition 2021, stable 1.95 or newer, the MSVC toolchain pinned by Plan 1's `rust-toolchain.toml`; existing workspace `serde`, `serde_json`, `thiserror`, and `sha2`; Python 3.12 standard library and `pytest` for development tools. No new registry version is introduced by this plan: inherit the versions already verified and pinned by Plan 1.

**Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` revision 6, especially §§3.5, 4.4, 5–6, 8–9, 12, 13.0–13.3. Read `docs/design/2026-09-10-design-outline.md` including §0b, `docs/research/R8-solver-bench.md`, and `docs/research/R7-pokerdata-verification.md` §4. This is the third of five sequential implementation plans; Plans 1 and 2 must pass before execution begins. The `.7z` converter, purchase, V9 sample, exploit slice, cache implementation, flop scheduling, and UI belong outside this plan.

Revision 6 resolved four questions this plan previously carried as conflicts, and the tasks below now implement the resolved text rather than a workaround: S11 gives `ApproxReason::AsymmetricStacks { stacks_bb: Vec<f32>, prominent: bool }` a real `prominent` field; S12 restates §13.1's T3 posterior in §8.4's terms; S13 fixes board blocking to the **output marginal at the street root, never per-branch masses**; S14 states that a branch whose mapped continuation is uncovered freezes navigation for the remainder of that street. S1 adds `StreetRootSnapshot.bb_chips` and S2 gives `RootError` five variants; both are consumed below.

## Global Constraints

- Format: 6-max NLHE cash, manual entry, Windows 11 desktop, private use only.
- Rust stable 1.95 or newer. Node 24; WebView2 present on Windows 11; Python 3.12 only for `tools/`.
- Plan 1 pins `stable-x86_64-pc-windows-msvc` in `rust-toolchain.toml`; use ordinary `cargo` commands, without a GNU override. The approved outline §0b supersedes the earlier pending MSVC installation and preflop-purchase language.
- Rust edition 2021; `cargo test`; `thiserror` for error enums; `serde` with `#[serde(tag = "type")]` for worker messages; `rayon` only in the worker. Python tests use `pytest`.
- Dependency direction is strictly downward: `proto` <- `core-*` <- `cache`/`engine` <- `bench`/`pokerai-app`. `solver-worker`'s only project dependency is `proto`; it additionally links the vendored solver. No crate depends on `solver-worker`.
- `core-preflop` depends on `proto`, `core-ranges`, `core-model`; `core-replay` additionally depends on `core-preflop`. Shared `SeatMass`/`HistoryBranch` definitions live in `core-preflop` and are re-exported by `core-replay`, avoiding a reverse dependency. Their fields and public names remain those of §9.1.
- All crates in this plan use `MIT OR Apache-2.0`. Solver commit `9d1509fe5077d019825f833eed04b16d342dfda1`, `bincode = "=2.0.0-rc.3"`, `bincode_derive = "=2.0.0-rc.3"`, and `+avx2` for both Windows targets remain Plan 1/2 requirements; no AGPL code is copied into these libraries.
- All `HandState`, worker and cache wager amounts are integer chips (`u32`), one chip = the session's accounting tick and legal wager quantum. `cap_mchips: u32` is thousandths of a chip. EV and modelled rake amounts are signed finite `f32` chips. Display: `ev_bb = ev_chips / bb_chips`, rounded to 0.01 bb at render time only. Check `pot + stacks < 2^31`.
- `EV(a) = E[hero's final stack | a] - hero's stack at the decision point`, in chips. Fold = 0 exactly; chips already in the pot are sunk.
- Card id = `rank_index*4 + suit_index`; ranks 2..A = 0..12; suits c,d,h,s = 0..3. For `lo < hi`, combo index = `hi*(hi-1)/2 + lo`. `Range1326` has exactly 1326 finite weights in `[0, 1]`. Class order is the 13×13 A-to-2 row-major grid: diagonal pairs, above diagonal suited, below diagonal offsuit.
- Public ranges never contain hero-card removal; hero-conditioned opposing copies are created only after replay for equity and terminal-call calculations. Bunching is fixed off. Vacant seats have no range.
- 3 to 6 dealt seats are supported. Two dealt seats: `FormatUnsupported{detail: "two dealt seats"}`. Straddle: one fully posted UTG straddle, six dealt seats, `S >= 2 * bb`; no button/Mississippi straddle, re-straddle, or short post.
- First-attempt deadlines: river 2 s, turn 6 s, flop `flop_budget_s`; final delivery: river/turn 15 s, flop `5 s + flop_budget_s`. Default `flop_budget_s = 10`, range `1..=30`; `target_bp = 50`; worker receives remaining time minus 100 ms delivery and 50 ms pipe margins. Validation + coverage + replay <= 0.15 s, Fast <= 0.3 s, in-memory preflop lookup <= 0.05 s. These are measurement targets, not new timing guarantees.
- Source envelopes: `game = "nl"`, `version = 2`, source blinds `sb = 0.5`, `bb = 1`; `ev_unit = "source_sb"`; dense action-major 169 arrays. Manifest and decoded content bounds: 64 MiB. Per-class sibling sum is `1 +- 1e-3`, or exactly 0 and explicitly unreachable.
- Source order: PokerData over charts; within source nearest depth; nearest-depth ties deeper; above 200 clamps to 200; final tie lexicographically smaller `bundle_id`. Missing nodes stay missing.
- Branch arithmetic is `f64`. Start `q = 1`; `q` is never rescaled. At most 4 live branches plus 1 residual. Positive output below `f32::MIN_POSITIVE` clamps to `f32::MIN_POSITIVE`; zero stays zero. No positive-reach threshold.
- Every event and registration checks `(hand_id, hand_revision, decision_id, config_revision, model_revision)`. Snapshot provenance identity is immutable. Reasons accumulate, including on `Unsupported.partial` and subsequent exact solves/cache hits.
- Charts always emit `ChartRounded` and omit `evs`. Baseline release does not depend on V9. `accuracy = "unverified"` is provenance, never a solver certificate.
- One implementation commit per task, with the exact trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Run `cargo test --workspace` after each task. The commands below run from `D:/Documents/Projects/PokerAI`; steps target 2–5 minutes each. Repeated transcription steps are explicitly one grid at a time, not one entire PDF per step, and Tasks 5 and 6 additionally commit once per transcribed grid so an interrupted transcription never loses completed work.
- Single implementations of shared rules: the chip-path rule lives once in `proto::resolve_chip_path(&[MaterializedNode], &[Action]) -> Option<OrdinalPath>` and every consumer here calls it with `&tree.materialized`; the headline rule lives once in `engine::assemble::headline(&mut [ActionAdvice], unresolved_mass: f32, HeadlineSource) -> Option<String>` and this plan extends its call sites rather than adding a second entry point; the snapshot record lives once as `core_replay::StreetSnapshot`, which replaces Plan 2's temporary `engine::snapshots::SolvedStreet`.

---

## File structure

Every path below is an executor output or modification; writing this plan creates only this Markdown file.

| File | Responsibility |
|---|---|
| `Cargo.toml`, `Cargo.lock` | Add the two workspace members and inherit existing dependency versions. |
| `crates/core-preflop/Cargo.toml` | Permissive library manifest with downward dependencies. |
| `crates/core-preflop/src/lib.rs` | Public exports and `PreflopAnswer` surface. |
| `crates/core-preflop/src/envelope.rs` | Manifest, wire envelope, `PreflopSource` and normalized types. |
| `crates/core-preflop/src/validate.rs` | Schema, bounded input, semantic and hash validation. |
| `crates/core-preflop/src/store.rs` | `PokerDataJson`, `ChartTranscription`, loader, quarantine, source index. |
| `crates/core-preflop/src/lookup.rs` | Prefix-only reconstruction, source ordering and node selection. |
| `crates/core-preflop/src/depth.rs` | Depth, asymmetry, rake candidate ordering. |
| `crates/core-preflop/src/straddle.rs` | Physical-to-virtual roles, source units, short-handed lookup folds. |
| `crates/core-preflop/src/ev.rs` | Four EV reference variants and 169-to-1326 expansion. |
| `crates/core-preflop/src/translate.rs` | Pseudo-harmonic interpolation, mapped legal actions, branch advice assembly. |
| `crates/core-preflop/src/branches.rs` | Shared history types, likelihood updates, marginal/posterior, cap and rescale. |
| `crates/core-preflop/tests/envelope.rs` | Schema and quarantine tests. |
| `crates/core-preflop/tests/lookup.rs` | Path, depth, straddle, rake and prefix regressions. |
| `crates/core-preflop/tests/ev.rs` | Synthetic EV scaling/reference tests. |
| `crates/core-preflop/tests/translate.rs` | Size boundaries and legal probability moves. |
| `crates/core-replay/Cargo.toml` | Replay library and downward dependencies. |
| `crates/core-replay/src/lib.rs` | Exact §9.1 input/output, orchestration and output conversion. |
| `crates/core-replay/src/branches.rs` | Re-export branch types; replay branch audit and board blocking. |
| `crates/core-replay/src/preflop.rs` | Once-only prefix walk and frozen missing-node stops. |
| `crates/core-replay/src/postflop.rs` | Completed-street ordinal walk, partial exports and translation. |
| `crates/core-replay/src/snapshot.rs` | Snapshot types, compatibility/selection, `SnapshotStore` (replaces Plan 2's `engine::snapshots::SolvedStreet`/`SnapshotStore`). |
| `crates/core-replay/tests/branches.rs` | T3, T6, pseudo-harmonic, residual and zero-support tests. |
| `crates/core-replay/tests/replay.rs` | Uniform start, folded provenance, missing nodes and hero separation. |
| `crates/core-replay/tests/snapshots.rs` | Prefix reuse, compatibility, missing continuations. |
| `crates/core-replay/tests/assembly.rs` | T7 incomplete EV and hero-out-of-support results. |
| `crates/engine/Cargo.toml` | Add `core-preflop` and `core-replay`; add the `testing` feature to the integration test's dev-dependency on itself. |
| `crates/engine/src/lib.rs` | Plan 2's module list plus `pub mod preflop;` and `pub mod replay_bridge;`. |
| `crates/engine/src/preflop.rs` | Preflop advice, range mix, coverage and the `HeadlineSource` mapping handed to `assemble::headline`. |
| `crates/engine/src/replay_bridge.rs` | `ReplayRanges` (`RangeSource` impl), replay inputs, snapshots built from validated solutions. |
| `crates/engine/src/ranges.rs` | Plan 2's `RangeSource`/`RootRanges`/`ExplicitRanges`; `ReplayRanges` is installed into `EngineCore.range_source`. |
| `crates/engine/src/snapshots.rs` | Plan 2's `SolvedStreet`/`SnapshotStore` are deleted; the module re-exports `core_replay::{StreetSnapshot, SnapshotStore}`. |
| `crates/engine/src/serve.rs` | Replace the `Classification::Preflop` arm; build `StreetSnapshot` instead of `SolvedStreet` at the register site. |
| `crates/engine/src/core.rs` | `EngineCore.snapshots` retyped to `Arc<Mutex<core_replay::SnapshotStore>>`; `range_source` initialized to `ReplayRanges`. |
| `crates/engine/src/engine.rs` | `Paths.preflop` populated; `register_snapshot`, `preflop_store`; `invalidate_hand` call sites moved to `invalidate(&HandState)`. |
| `crates/engine/src/assemble.rs` | Extend existing assembly for known mass/unresolved mass; `headline` gains no second entry point. |
| `crates/engine/tests/preflop_replay.rs` | Engine paths, registration and inherited-reason tests. |
| `crates/engine/tests/golden/replay_weights_golden.json` | Independently computed complete 1326 vectors and log reach. |
| `crates/engine/tests/golden/bet_translation_golden.json` | Boundary, legality and no-created-EV cases. |
| `crates/engine/tests/preflop_goldens.rs` | Both §13.3 golden runners. |
| `tools/chart_ingest.py` | Fetch, normalize, validate and reproducibly build chart envelopes. |
| `tools/tests/test_chart_ingest.py` | Python matrix, key, manifest, inventory and reproduction tests. |
| `tools/gen_preflop_fixtures.py` | Deterministic synthetic v2 and independent replay golden generator. |
| `tools/tests/test_preflop_fixtures.py` | Synthetic semantics and golden arithmetic. |
| `docs/data/chart-transcription.md` | Source URLs/hashes, grid-by-grid transcription and verification record. |
| `fixtures/charts/sources.manifest.json` | Which chart depths this build actually acquired (`available` / `unsupported`), with source hashes. Read by Tasks 5–7, 17, 19 and Plan 4. |
| `fixtures/charts/sources/pokercoaching_100.pdf` | Frozen public source bytes; fetched by executor. |
| `fixtures/charts/sources/rangeconverter_200.pdf` | Frozen PDF resolved through the publisher download page. Conditional: absent when depth 200 is `unsupported`. |
| `fixtures/charts/sources/rangeconverter_200.html`, `rangeconverter_200.download.html` | Frozen article and download-page provenance, without account information. |
| `fixtures/charts/transcription/pokercoaching_100.json` | Auditable class-grid input, action legends, pages and node inventory. |
| `fixtures/charts/transcription/rangeconverter_200.json` | Auditable 0/0.5/1 grid input and node inventory. Conditional on depth 200. |
| `fixtures/charts/pokercoaching_100.json`, `fixtures/charts/pokercoaching_100.manifest.json` | 100bb normalized chart bundle and SHA-256 manifest. |
| `fixtures/charts/rangeconverter_200.json`, `fixtures/charts/rangeconverter_200.manifest.json` | 200bb normalized chart bundle and manifest. Conditional on depth 200. |
| `fixtures/preflop/synthetic_v2/node.json`, `range.json`, `spots.json` | Marked synthetic provider-shaped examples, never acquired vendor data. |
| `fixtures/preflop/synthetic_v2/nodes.json`, `manifest.json` | Dense synthetic EV-bearing normalized bundle. |
| `fixtures/preflop/synthetic_v2/cases.json` | Node paths, reference variants, explicit absences and expected values. |

## Interface ownership and execution gates

Consume Plan 1's `core_model::{begin_hand, apply_action, set_board, derive, street_root, replay_root}` with the exact §3.5 signatures; `apply_action(&HandState, Action) -> Result<HandState, RulesError>`, `derive(&HandState) -> Derived`, `street_root(&HandState) -> Result<StreetRootSnapshot, RootError>`. Consume `core_ranges::{expand_169, block_public, hero_conditioned, mass, hash_scaled}`: `expand_169(&[f32; 169]) -> Range1326`, `hero_conditioned(&Range1326, [Card; 2]) -> Range1326`, `hash_scaled(&Range1326) -> [u8; 32]`. Import all §4 types from `proto`, and `NodeStrategy`, `StreetSolution`, `validate_solution` from `proto::worker`; never redeclare them.

Also consume `proto::resolve_chip_path(materialized: &[MaterializedNode], path: &[Action]) -> Option<OrdinalPath>` for every chip-path-to-ordinal-path resolution in this plan. Do **not** define a `core_replay::resolve_path`; the spec §2 rule has exactly one implementation and `core-replay` calls it as `proto::resolve_chip_path(&tree.materialized, chips)`.

Plan 2's `crates/engine/src` module list is `clock, identity, tree, worker, deadline, watchdog, core, solve, coverage, equity, allin, assemble, log, snapshots, ranges, serve, engine` plus `#[cfg(any(test, feature = "testing"))] pub mod testing;` and `pub use engine::{Engine, Paths};`. There is no `postflop.rs`. Plan 2 owns `build_effective_tree(&StreetRootSnapshot, &TemplateSelection) -> Result<EffectiveTree, UnsupportedReason>`, worker validation, identity and deadline machinery, `serve::serve_request(&mut EngineCore, LiveRequest)` and the result assembler. Its `Engine` surface is `new(GameConfig, Paths) -> Result<Engine, EngineError>`, `set_config(&mut self, GameConfig) -> Result<u32, EngineError>`, `begin_hand`, `apply_action`, `set_board`, `set_hero_cards`, `undo`, `set_explicit_ranges`, `recommend(&mut self, Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError>`, `cancel`, `finish_hand`, `abandon_hand`, `state`, `shutdown(&mut self)`, with `Paths { worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf }`. This plan adds `Engine::register_snapshot(&DecisionIdentity, StreetSnapshot) -> bool` and `Engine::preflop_store(&self) -> &PreflopStore`, populates `Paths.preflop`, and integrates into the actual Plan 2 dispatch rather than creating a second engine or worker client. New helper signatures below are Plan 3-owned contracts, not alternative names for spec APIs.

**The three engine seams this plan hooks into, by exact name.** Nothing else in `crates/engine` changes shape.

1. `crates/engine/src/snapshots.rs` — Plan 2 ships a temporary `SolvedStreet { identity_at_solve, street, board, tree, nodes, ordinal_paths, exploitability_chips, reasons, solved_prefix }` and `SnapshotStore::{new, register(&mut self, &DecisionIdentity, SolvedStreet) -> bool, invalidate_hand(&mut self, u64), for_hand(&self, u64) -> Vec<&SolvedStreet>}`, held as `EngineCore.snapshots: Arc<Mutex<SnapshotStore>>` and cloned into `Engine.snapshots`. Task 14 **deletes both types** and makes `crates/engine/src/snapshots.rs` a re-export module: `pub use core_replay::{SnapshotStore, StreetSnapshot, SnapshotKey, SnapshotProvenance};`. `SolvedStreet`'s fields are a subset of `StreetSnapshot` (`board` becomes `key.root_board`, `ordinal_paths` becomes `covered_paths`, the rest move into `key`/`provenance`), so no data is lost. `EngineCore.snapshots` is retyped to `Arc<Mutex<core_replay::SnapshotStore>>`; Plan 2's three `invalidate_hand(hand_id)` call sites in `engine.rs` (`mutate`, `undo`, `end_hand`) are rewired as Task 18 Step 5 specifies — `mutate` and `undo` to `invalidate(&HandState)`, `end_hand` keeping `invalidate_hand`; `serve.rs`'s single `for_hand` use in `identity_race_golden` becomes `for_identity(&DecisionIdentity)`. This replacement happens **inside Task 14**, in one commit, so `identity_race_golden` never goes red between tasks.
2. `crates/engine/src/ranges.rs` — `pub trait RangeSource: Send { fn ranges_at_root(&self, state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>; }` with `RootRanges { oop: Range1326, ip: Range1326, reasons: Vec<ApproxReason>, ranges_used: Vec<(Seat, String, f32)> }`, defaulted to `ExplicitRanges` in `EngineCore.range_source: Box<dyn RangeSource>`. Task 18 adds `ReplayRanges { store: Arc<PreflopStore>, snapshots: Arc<Mutex<SnapshotStore>> }` implementing that trait and installs it in `EngineCore::new`; `serve_request` already calls `core.range_source.ranges_at_root(&req.state, &root)` and is not otherwise touched for the turn/river path.
3. `crates/engine/src/serve.rs`, `serve_request` — the arm `Classification::Preflop => { emit(core, &req, None, RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::EngineError { message: "no preflop path in this build (plan 3)".into(), retryable: false }, vec![], assumptions))); return; }` is the exact line Task 17 replaces with a call into `crate::preflop::serve_preflop`.

Headline text is produced once, by Plan 2's `assemble::headline(&mut [ActionAdvice], unresolved_mass: f32, HeadlineSource) -> Option<String>` with `HeadlineSource::{Solved, Chart, PokerDataUnverified}`, which already implements every §4.4 rule this plan needs (highest EV; else highest-frequency with the chart / unverified / EV-incomplete wording; no headline when `unresolved_mass > 0`). Task 17 supplies the `HeadlineSource` and does **not** add a second `set_headline`.

Revision 6 removed the two spec sketch mismatches this plan previously worked around. `ApproxReason::AsymmetricStacks { stacks_bb: Vec<f32>, prominent: bool }` now carries `prominent`, so Task 8 emits the flag on the reason itself instead of hiding it in an assumptions note. §13.1's T3 now states §8.4's reading directly (one-element branch posterior `pi_{S,0}[c] == 1`; the actor's normalized combo distribution is `(0.9, 0.1)`; an unacted seat stays uniform), so Task 11 asserts the spec text as written.

### Task 1: Define and validate the normalized preflop envelope

**Files:** Create `crates/core-preflop/{Cargo.toml,src/lib.rs,src/envelope.rs,src/validate.rs,tests/envelope.rs}`; modify `Cargo.toml`, `Cargo.lock`.

**Interfaces:**
- Consumes: `proto::{Position, Range1326, HandConfig, HandState, Action, ApproxReason, UnsupportedReason, Unavailable}`.
- Produces: exact §8.1 `PreflopSource`, `PreflopNodeKey`, `PreflopStep`, `PreflopNode`; `validate(&Envelope) -> Result<(), BundleError>`; `decode(&[u8]) -> Result<Envelope, BundleError>`; wire `Envelope`, `EnvelopeNode`, `EnvelopeAction`; `BundleInfo`, `SourceKind`, `EvReference`, `RakeProfile`.

- [ ] **Step 1: Add the crate and a failing matrix test (2–5 minutes).** In the root manifest append `crates/core-preflop` to members. Use inherited versions in its manifest:

```toml
[package]
name = "core-preflop"
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
[dependencies]
proto = { path = "../proto" }
core-model = { path = "../core-model" }
core-ranges = { path = "../core-ranges" }
serde = { workspace = true, features = ["derive"] }
serde_json.workspace = true
thiserror.workspace = true
sha2.workspace = true
```

```rust
#[test]
fn pokerdata_schema_mapping() {
    let mut e = core_preflop::Envelope {
        bundle_id: "synthetic".into(), depth_bb: 100,
        rake_profile: "5% cap 0.5bb".into(), straddle: false,
        class_order: "A-2 row-major, section 4.1".into(),
        nodes: vec![core_preflop::EnvelopeNode {
            history: vec![], actor: "UTG".into(),
            actions: vec![core_preflop::EnvelopeAction {
                step: "fold".into(), to_bb_x1000: None, label: None,
            }], weights: vec![vec![1.0; 169]], evs: None,
            unreachable_classes: vec![],
        }],
    };
    assert!(core_preflop::validate(&e).is_ok());
    e.nodes[0].weights[0][17] = 0.4;
    assert!(core_preflop::validate(&e).is_err());
    e.nodes[0].weights[0][17] = 0.0;
    assert!(core_preflop::validate(&e).is_err());
    e.nodes[0].unreachable_classes.push(17);
    assert!(core_preflop::validate(&e).is_ok());
}
```

- [ ] **Step 2: Run the failing test.** `cargo test -p core-preflop --test envelope pokerdata_schema_mapping`; expect missing envelope/validator symbols, not a dependency-download failure.

- [ ] **Step 3: Define the wire and public types.** Use `#[derive(Clone, Debug, Serialize, Deserialize)]` and `#[serde(deny_unknown_fields)]` on envelope structs, optional EVs with `#[serde(default, skip_serializing_if = "Option::is_none")]`. History is `Vec<(String, String, u32)>`, matching JSON three-tuples. The public structs are:

```rust
/// `Send + Sync` because the engine's fast path reads the store from `engine-main`
/// while `Engine` holds it (§8.1's method signatures are unchanged; `dyn PreflopSource`
/// inherits the auto traits, and both adapters below are plain data).
pub trait PreflopSource: Send + Sync {
    fn bundle_info(&self) -> &BundleInfo;
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode>;
}
pub struct PreflopNodeKey {
    pub depth_bb: u16, pub rake_profile: String, pub straddle: bool,
    pub history: Vec<(Position, PreflopStep)>,
}
pub enum PreflopStep { Fold, Check, Call, Raise { to_bb_x1000: u32 }, AllIn }
pub struct PreflopNode {
    pub actor: Position, pub actions: Vec<PreflopStep>,
    pub probs: Vec<Vec<f32>>, pub ev_source_sb: Option<Vec<Vec<Option<f32>>>>,
    pub unreachable: [bool; 169], pub committed_by_actor_sb: f32,
}
pub enum SourceKind { PokerDataJson, ChartTranscription }
pub enum EvReference {
    DecisionIncrementalVerified, NetHandStartVerified,
    AbsoluteStackVerified, Unverified,
}
pub struct RakeProfile { pub rate: f32, pub cap_bb: f32, pub no_flop_no_drop: bool }
pub struct BundleInfo {
    pub bundle_id: String, pub source: SourceKind, pub depth_bb: u16,
    pub depths: Vec<u16>, pub source_blinds: [f32; 2],
    pub rake_profile: String, pub rake: Option<RakeProfile>,
    pub straddle: bool, pub version: u16, pub game: String,
    pub ev_unit: String, pub ev_reference: EvReference,
    pub license_note: String, pub accuracy: String, pub sha256: String,
}
```

Use serde renames `decision_incremental_verified`, `net_hand_start_verified`, `absolute_stack_verified`, `unverified`; exact source adapter strings `PokerDataJson` and `ChartTranscription`. `rake: None` means source rake is undocumented, never exact; retain its descriptive `rake_profile`. A chart PDF that does not publish its rake cannot be assigned the PokerData profile as fact. Derive Clone/Debug/PartialEq on public node/key types, Copy/Eq on SourceKind and EvReference, and serde on BundleInfo, RakeProfile, key/step and the source/reference enums. Serialize stable keys as their JSON tuple representation. Define the wire structs explicitly:

```rust
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub bundle_id:String, pub depth_bb:u16, pub rake_profile:String,
    pub straddle:bool, pub class_order:String, pub nodes:Vec<EnvelopeNode>,
}
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeNode {
    pub history:Vec<(String,String,u32)>, pub actor:String,
    pub actions:Vec<EnvelopeAction>, pub weights:Vec<Vec<f32>>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub evs:Option<Vec<Vec<Option<f32>>>>,
    pub unreachable_classes:Vec<usize>,
}
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeAction {
    pub step:String,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub to_bb_x1000:Option<u32>,
    #[serde(default,skip_serializing_if="Option::is_none")]
    pub label:Option<String>,
}
```

- [ ] **Step 4: Implement the semantic checks and bounded decoder.** Apply the exact sum rule, bounds before indexing, no silent renormalization:

```rust
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("bundle content: {0}")] Content(String),
    #[error("bundle exceeds 64 MiB")] TooLarge,
    #[error("bundle hash mismatch")] Hash,
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Io(#[from] std::io::Error),
}
pub const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;
pub fn decode(bytes: &[u8]) -> Result<Envelope, BundleError> {
    if bytes.len() as u64 > MAX_BUNDLE_BYTES { return Err(BundleError::TooLarge); }
    let envelope: Envelope = serde_json::from_slice(bytes)?;
    validate(&envelope)?;
    Ok(envelope)
}
pub fn valid_step(step: &str, amount: Option<u32>) -> bool {
    match step {
        // A raise must carry a resolved positive size; every other token must carry no
        // amount at all (`Some(0)` is a malformed size field, not an omitted one).
        "raise" => amount.is_some_and(|v| v > 0),
        "fold" | "check" | "call" | "allin" => amount.is_none(),
        _ => false,
    }
}
pub fn validate(e: &Envelope) -> Result<(), BundleError> {
    let bad = |s: &str| BundleError::Content(s.into());
    if e.class_order != "A-2 row-major, section 4.1" || e.depth_bb == 0 {
        return Err(bad("class order or depth"));
    }
    let positions = ["UTG", "HJ", "CO", "BTN", "SB", "BB"];
    let mut keys = std::collections::BTreeSet::new();
    for n in &e.nodes {
        if !positions.contains(&n.actor.as_str()) || n.actions.is_empty()
            || !keys.insert(serde_json::to_string(&n.history)?) {
            return Err(bad("actor, empty menu, or duplicate history"));
        }
        // History tuples are `(position, step, amount)` with 0 meaning "no amount",
        // which is why the history check maps 0 to None before calling `valid_step`.
        if n.history.iter().any(|(p,s,v)|
                !positions.contains(&p.as_str()) || !valid_step(s, (*v != 0).then_some(*v)))
            || n.actions.iter().any(|a| !valid_step(&a.step, a.to_bb_x1000)) {
            return Err(bad("unresolved size or invalid token"));
        }
        let mut menu = std::collections::BTreeSet::new();
        if n.actions.iter().any(|a| !menu.insert((a.step.clone(), a.to_bb_x1000))) {
            return Err(bad("duplicate action kind and amount"));
        }
        let mut unreachable_seen = std::collections::BTreeSet::new();
        if n.unreachable_classes.iter().any(|c| !unreachable_seen.insert(*c)) {
            return Err(bad("duplicate unreachable class"));
        }
        if n.weights.len() != n.actions.len() || n.weights.iter().any(|r| r.len()!=169)
            || n.unreachable_classes.iter().any(|&c| c>=169) {
            return Err(bad("shape"));
        }
        for c in 0..169 {
            let row: Vec<f32> = n.weights.iter().map(|a| a[c]).collect();
            if row.iter().any(|p| !p.is_finite() || !(0.0..=1.0).contains(p)) {
                return Err(bad("probability bound"));
            }
            let sum: f64 = row.iter().map(|&p| p as f64).sum();
            let unreachable = n.unreachable_classes.contains(&c);
            if (unreachable && sum != 0.0) || (!unreachable && (sum-1.0).abs()>1e-3) {
                return Err(bad("sibling sum or unreachable declaration"));
            }
        }
        if let Some(evs) = &n.evs {
            if evs.len()!=n.actions.len() || evs.iter().any(|r| r.len()!=169)
                || evs.iter().flatten().flatten().any(|v| !v.is_finite()) {
                return Err(bad("EV shape or finite value"));
            }
        }
    }
    Ok(())
}
```

The two `BTreeSet` inserts above are the duplicate rejections: identical `(step, to_bb_x1000)` action pairs and repeated `unreachable_classes` indices both fail. Non-raise actions carrying any amount — including an explicit `0` — are invalid. Convert uppercase provider aliases only at ingestion (`LJ`→`UTG`, `MP`→`HJ`), not in this strict envelope reader.

- [ ] **Step 5: Verify and commit.** Run `cargo test -p core-preflop`, then `cargo test --workspace`. Expect all pass.

```powershell
git add Cargo.toml Cargo.lock crates/core-preflop
git commit -m "feat(core-preflop): validate normalized source envelopes" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 2: Load independent sources and quarantine invalid bundles

**Files:** Create `crates/core-preflop/src/store.rs`, `tools/gen_preflop_fixtures.py`, `tools/tests/test_preflop_fixtures.py`, all six `fixtures/preflop/synthetic_v2/*.json` files listed above; modify `src/{lib.rs,validate.rs}`, `tests/envelope.rs`.

**Interfaces:**
- Consumes: Task 1 `decode`, `BundleInfo`, envelope/public node types; `std::io::Read::take` and existing workspace `sha2`.
- Produces, with these exact signatures: `PreflopStore { bundles: Vec<Box<dyn PreflopSource>> }`; `PreflopStore::from_sources(bundles: Vec<Box<dyn PreflopSource>>) -> Self`, `PreflopStore::bundles(&self) -> &[Box<dyn PreflopSource>]`, `PreflopStore::bundle_of(&self, bundle_id: &str) -> Option<&dyn PreflopSource>`, `PreflopStore::open(dir: &Path) -> (Self, Vec<String>)`; `PokerDataJson`, `ChartTranscription`; `bounded_read(path: &Path) -> Result<Vec<u8>, BundleError>`; `checked_envelope(info: &BundleInfo, raw: &[u8]) -> Result<Envelope, BundleError>`; `transpose<T: Clone>(action_major: &[Vec<T>]) -> Vec<Vec<T>>`; `build_node_map(info: &BundleInfo, e: &Envelope) -> Result<BTreeMap<String, PreflopNode>, BundleError>`; `load_bundle(manifest: &Path, nodes: &Path) -> Result<Box<dyn PreflopSource>, BundleError>`; `node_key(k: &PreflopNodeKey) -> String`.

- [ ] **Step 1: Add failing hash-plus-content tests.** Each case uses a fresh temporary directory below `std::env::temp_dir()` named with process id and case index, written by the test and removed at its end. Keep a valid sibling bundle in every test.

```rust
#[test]
fn bundle_validation_quarantine() {
    let raw = include_bytes!("../../../fixtures/preflop/synthetic_v2/nodes.json");
    let mut v: serde_json::Value = serde_json::from_slice(raw).unwrap();
    v["nodes"][0]["weights"][0][0] = serde_json::json!(0.4);
    let malformed = serde_json::to_vec(&v).unwrap();
    assert!(core_preflop::decode(&malformed).is_err());
    assert!(core_preflop::decode(br#"{"nodes": ["#).is_err());
    v["nodes"][0]["history"] = serde_json::json!([["UTG","mystery",0]]);
    assert!(core_preflop::decode(&serde_json::to_vec(&v).unwrap()).is_err());
}
```

Extend this test with wrong action count, 168 entries, `1e400`, `[-0.1,1.1]`, undeclared all-zero class, hash mismatch, and 64 MiB+1 input. Recompute a valid SHA-256 for every malformed-content case before invoking `open`; assert `.bad` exists, one banner is returned, and the good source still answers a direct key lookup. A valid hash alone must never pass.

- [ ] **Step 2: Run red.** `cargo test -p core-preflop --test envelope`; expect absent fixture/loader symbols.

- [ ] **Step 3: Generate the marked synthetic v2 data.** `tools/gen_preflop_fixtures.py` uses only standard-library JSON/hash/math/pathlib. Start with this real dense construction and add each case from the explicit path table below using `make_node`:

```python
import hashlib, json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, allow_nan=False) + "\n", encoding="utf-8")
def make_node(history, actor, actions):
    # `committed_by_actor_sb` is derived by the Rust loader from the source posts and
    # this history; the generator must not carry a second, divergent copy of it.
    weights = [[0.0] * 169 for _ in actions]
    evs = [[None] * 169 for _ in actions]
    for c in range(169):
        weights[0][c] = 1.0
        evs[0][c] = 0.0
    if len(actions) > 1:
        weights[0][1], weights[1][1] = 0.65, 0.35   # class 1 = AKs
        evs[1][1] = 1.84
        evs[1][14] = 2.31  # class 14 = KK: explicit EV retained even with zero action weight.
    weights[0][168] = 0.0
    return {"history": history, "actor": actor, "actions": actions,
            "weights": weights, "evs": evs, "unreachable_classes": [168]}
def manifest_for(envelope):
    raw = (json.dumps(envelope, indent=2, allow_nan=False) + "\n").encode()
    return {"bundle_id": envelope["bundle_id"], "source": "PokerDataJson",
            "game": "nl", "version": 2, "depth_bb": 100, "depths": [100],
            "source_blinds": [0.5, 1.0], "rake_profile": "5% cap 0.5bb",
            "rake": {"rate": .05, "cap_bb": .5, "no_flop_no_drop": True},
            "straddle": False, "ev_unit": "source_sb",
            "ev_reference": "decision_incremental_verified", "accuracy": "unverified",
            "license_note": "Synthetic test data; not vendor data; no V9 claim",
            "sha256": hashlib.sha256(raw).hexdigest()}
```

| Fixture node | Complete history before actor | Menu |
|---|---|---|
| UTG RFI | empty | fold, raise 2500 |
| HJ vs UTG RFI | UTG raise 2500 | fold, call, raise 8750 |
| UTG vs HJ 3bet | UTG raise 2500; HJ raise 8750; CO, BTN, SB, BB fold | fold, call, raise 22000 |
| HJ vs UTG 4bet | preceding history; UTG raise 22000 | fold, call, allin |
| BB squeeze | UTG raise 2500; HJ call; CO, BTN, SB fold | fold, call, raise 12000 |
| SB limp | UTG, HJ, CO, BTN fold | fold, call, raise 3000 |
| BB vs SB limp | preceding history; SB call | check, raise 3500 |
| Explicit absences | CO/SB cold-call-vs-3bet; UTG/HJ/CO/BTN open-limp | no node, recorded only in `cases.json` |

Write `node.json` with `synthetic: true`, `game: "nl"`, `version: 2`, `stack: 100`, `actor: "HJ"`, `history: "UTG_60%"`, and action records with sparse `weights`/`evs`. Write `range.json` with `synthetic: true`, `spot: "UTG_60%_HJ_Call"`, `actor: "HJ"`, `hand: "AKs"`, `freq: .35`, `ev: 1.84`, `combos: 61.9`, `weights: {"AKs": .35, "KK": 0, "JJ": 1}`, `evs: {"AKs": 1.84, "KK": 2.31}`. `node.json`/`range.json`/`spots.json` are the **provider-shaped** files (R7 §4's sparse wire layout, exercised by the key/spot parsers); `nodes.json` is the dense normalized envelope. Their one shared invariant, asserted in `tools/tests/test_preflop_fixtures.py`, is the sparse zero-weight EV: class 14 (`KK`) carries `2.31` with weight `0` in **both** shapes. `range.json`'s remaining entries are illustration only and are not required to reproduce the dense grid. `spots.json` lists every table path with its resolved action sizes. `cases.json` records all four reference variants, SB committed=1, BB committed=2, source_stack_sb=200 and the absences. Provider `spot` ending in an action identifies an action range: remove that last pair when deriving its decision-node key. No provider HTTP calls or `.7z` decoding.

- [ ] **Step 4: Implement bounded load and per-bundle quarantine.** Manifest is the same `BundleInfo` shape. Reject mismatched ids/depth/profile/straddle, `game != "nl"`, `version != 2`, source blinds other than `[.5,1]`, or `ev_unit != "source_sb"`. Hash raw file bytes before JSON decoding. Directory layout is `<bundle_id>/manifest.json + nodes.json`; packaged charts use sibling `<name>.manifest.json + <name>.json` and enter the same loader.

```rust
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};
pub fn bounded_read(path: &Path) -> Result<Vec<u8>, BundleError> {
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BUNDLE_BYTES+1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BUNDLE_BYTES { return Err(BundleError::TooLarge); }
    Ok(bytes)
}
pub fn checked_envelope(info: &BundleInfo, raw: &[u8]) -> Result<Envelope, BundleError> {
    let digest = format!("{:x}", Sha256::digest(raw));
    if digest != info.sha256 { return Err(BundleError::Hash); }
    let e = decode(raw)?;
    if e.bundle_id != info.bundle_id || e.depth_bb != info.depth_bb
        || e.rake_profile != info.rake_profile || e.straddle != info.straddle
        || info.game != "nl" || info.version != 2 || info.source_blinds != [0.5,1.0]
        || info.ev_unit != "source_sb" {
        return Err(BundleError::Content("manifest/envelope mismatch".into()));
    }
    Ok(e)
}
pub fn transpose<T: Clone>(action_major: &[Vec<T>]) -> Vec<Vec<T>> {
    (0..169).map(|c| action_major.iter().map(|a| a[c].clone()).collect()).collect()
}
```

`open` reads sorted immediate children only, skips names ending `.bad`, validates independently, and appends successful adapters. On failure rename that bundle directory to a sibling `<bundle_id>.bad`; if occupied, use `<bundle_id>.<n>.bad` with the smallest free integer. Record rename failure in the banner and exclude the bundle anyway. Resolve and check that the rename source/destination both remain immediate children of the supplied directory; never follow a symlink out of it. Packaged fixtures are loaded read-only; quarantine applies to installed copies. JSON is uncompressed here, so the decoded 64 MiB bound is the file bound; any future decompression must feed the same bounded reader. Both adapters hold the same validated map; charts force EV absence and `ChartRounded`, not `EvReferenceUnverified`.

```rust
pub struct PreflopStore { bundles: Vec<Box<dyn PreflopSource>> }

/// Reads one bundle: manifest bytes -> `BundleInfo`, node bytes -> hash-checked,
/// content-validated `Envelope`, then the class-major node map keyed by `node_key`.
pub fn load_bundle(manifest: &Path, nodes: &Path) -> Result<Box<dyn PreflopSource>, BundleError> {
    let info: BundleInfo = serde_json::from_slice(&bounded_read(manifest)?)?;
    let envelope = checked_envelope(&info, &bounded_read(nodes)?)?;
    let map = build_node_map(&info, &envelope)?;   // transpose, committed_by_actor_sb, turn order
    Ok(match info.source {
        SourceKind::PokerDataJson => Box::new(PokerDataJson { info, nodes: map }),
        SourceKind::ChartTranscription => Box::new(ChartTranscription { info, nodes: map }),
    })
}

fn quarantine_name(dir: &Path, bundle_id: &str) -> std::path::PathBuf {
    let first = dir.join(format!("{bundle_id}.bad"));
    if !first.exists() { return first; }
    (1u32..).map(|n| dir.join(format!("{bundle_id}.{n}.bad")))
        .find(|p| !p.exists()).expect("a free quarantine name exists")
}

impl PreflopStore {
    /// Returns the store plus one banner line per rejected bundle. A failure of one
    /// bundle never removes another: every child is validated on its own bytes.
    pub fn open(dir: &Path) -> (Self, Vec<String>) {
        let mut names: Vec<std::ffi::OsString> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
            Err(e) => return (Self { bundles: vec![] }, vec![format!("preflop dir {}: {e}", dir.display())]),
        };
        names.sort();
        let (mut bundles, mut banners) = (Vec::new(), Vec::new());
        for name in names {
            let child = dir.join(&name);
            let id = name.to_string_lossy().to_string();
            if id.ends_with(".bad") || !child.is_dir() { continue; }
            match load_bundle(&child.join("manifest.json"), &child.join("nodes.json")) {
                Ok(source) => bundles.push(source),
                Err(e) => {
                    let target = quarantine_name(dir, &id);
                    // Both ends must stay immediate children of `dir`; never follow a link out.
                    let inside = |p: &Path| p.parent() == Some(dir);
                    let renamed = if inside(&child) && inside(&target) {
                        std::fs::rename(&child, &target).map_err(|e| e.to_string())
                    } else { Err("quarantine path escapes the preflop directory".into()) };
                    banners.push(match renamed {
                        Ok(()) => format!("preflop bundle {id} quarantined as {}: {e}", target.display()),
                        Err(why) => format!("preflop bundle {id} rejected ({e}); rename failed: {why}"),
                    });
                }
            }
        }
        (Self { bundles }, banners)
    }
}
```

```rust
pub fn build_node_map(info:&BundleInfo,e:&Envelope)
    ->Result<std::collections::BTreeMap<String,PreflopNode>,BundleError>{
    let mut map=std::collections::BTreeMap::new();
    for n in &e.nodes{
        let history=parse_history(&n.history)?;          // Vec<(Position, PreflopStep)>
        check_turn_order(&history,info.straddle)?;       // actor order; no action after a fold
        let mut unreachable=[false;169];
        for &c in &n.unreachable_classes{unreachable[c]=true;}
        let node=PreflopNode{
            actor:parse_position(&n.actor)?,
            actions:n.actions.iter().map(parse_step).collect::<Result<_,_>>()?,
            probs:transpose(&n.weights),                 // [actions][169] -> [169][actions]
            ev_source_sb:n.evs.as_ref().map(|rows|transpose(rows)),
            unreachable,
            committed_by_actor_sb:committed_before(&history,parse_position(&n.actor)?,info.depth_bb),
        };
        let key=node_key(&PreflopNodeKey{depth_bb:info.depth_bb,
            rake_profile:info.rake_profile.clone(),straddle:info.straddle,history});
        if map.insert(key.clone(),node).is_some(){
            return Err(BundleError::Content(format!("duplicate node {key}")));
        }
    }
    Ok(map)
}
```

`parse_history`, `parse_position` and `parse_step` map the envelope's strings onto `Position`/`PreflopStep` and return `BundleError::Content` naming the offending history; `check_turn_order` walks the history confirming each step belongs to the seat whose turn it is and that no position acts after folding; `committed_before` is the source-post replay described next. Every rejection carries the offending history in its message.

`node_key` serializes `(depth_bb,rake_profile,straddle,history)` deterministically. Build `committed_by_actor_sb` by replaying **the source posts, 1 SB and 2 SB**, and the source prefix, all in source-SB units. The source blinds are `sb = 0.5 bb` and `bb = 1 bb`, which in source SB are exactly 1 and 2 — writing `.5/1` here would halve every `committed` and silently change `net_hand_start_verified` and `absolute_stack_verified` results. Fold/Check no payment; Call matches highest contribution capped at the source stack; Raise sets contribution to `to_bb_x1000/500`; AllIn sets it to `2*depth_bb`. Confirm actor turn order and no action after fold while building the map. Do not use `combos` in this calculation or in probabilities. The two source implementations and stable key need no separate parsing logic:

```rust
pub struct PokerDataJson {
    pub info:BundleInfo, pub nodes:std::collections::BTreeMap<String,PreflopNode>,
}
pub struct ChartTranscription {
    pub info:BundleInfo, pub nodes:std::collections::BTreeMap<String,PreflopNode>,
}
pub fn node_key(k:&PreflopNodeKey)->String {
    serde_json::to_string(&(k.depth_bb,&k.rake_profile,k.straddle,&k.history))
        .expect("validated finite node key")
}
impl PreflopSource for PokerDataJson {
    fn bundle_info(&self)->&BundleInfo{&self.info}
    fn lookup(&self,key:&PreflopNodeKey)->Option<PreflopNode>{self.nodes.get(&node_key(key)).cloned()}
}
impl PreflopSource for ChartTranscription {
    fn bundle_info(&self)->&BundleInfo{&self.info}
    fn lookup(&self,key:&PreflopNodeKey)->Option<PreflopNode>{self.nodes.get(&node_key(key)).cloned()}
}
impl PreflopStore {
    pub fn from_sources(bundles:Vec<Box<dyn PreflopSource>>)->Self{Self{bundles}}
    pub fn bundles(&self)->&[Box<dyn PreflopSource>]{&self.bundles}
}
```

- [ ] **Step 5: Verify generated data and loader.** `python tools/gen_preflop_fixtures.py`, `python -m pytest tools/tests/test_preflop_fixtures.py -q`, `cargo test -p core-preflop --test envelope`, `cargo test --workspace`. Assert action-major→class-major transpose, sparse zero-weight EV retention, absent EV→None, explicit folds, SB-limp/BB-check, and unreachable class 168. Add the EV cross-check load failures in Task 9.

- [ ] **Step 6: Commit.**

```powershell
git add crates/core-preflop tools/gen_preflop_fixtures.py tools/tests/test_preflop_fixtures.py fixtures/preflop/synthetic_v2
git commit -m "feat(core-preflop): load isolated bundles with quarantine" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 3: Build the deterministic chart ingestion and validation tool

**Files:** Create `tools/chart_ingest.py`, `tools/tests/test_chart_ingest.py`, `docs/data/chart-transcription.md`.

**Interfaces:**
- Consumes: Task 1 envelope, Task 2 manifest; Python `json`, `hashlib`, `decimal`, `urllib.request`, `html.parser`, `argparse`, `pathlib`.
- Produces: `class_names() -> list[str]`, `build(transcription: dict) -> dict`, `validate(envelope: dict) -> None`; CLI `fetch`, `build`, `validate`, `verify` with explicit file arguments. Transcription format: envelope metadata (`bundle_id`, `depth_bb`, `rake_profile`, `straddle`, `license_note`, optional `accuracy`, optional `rake`) plus `nodes` carrying exact history, actor, actions, `page`, `title`, `rows` (13 arrays of 13 cell strings) and a `legend` mapping a cell code to action probabilities; plus an `inventory` list. An inventory row is `{"title": str, "status": "covered" | "absent", "history": [[position, step, amount]], "page": int | null, "reason": str}`: `history` is the normalized node key of that candidate (present on every row, so a covered row's history can be compared to the built envelope's node histories and an absent row still records what was looked for), and `reason` states the audit result — the resolved sizes for a covered row, the visual finding for an absent one.

- [ ] **Step 1: Write failing Python tests.**

Plan 1 Task 13 already created `tools/pyproject.toml` (`testpaths = ["tests"]`) and `tools/tests/conftest.py`, which does `sys.path.insert(0, str(Path(__file__).resolve().parents[1]))` — that puts **`tools/`** on `sys.path`, not the repository root. Import the module as `chart_ingest`, never `tools.chart_ingest`, and resolve every fixture path from `Path(__file__).resolve().parents[2]` (the repository root) rather than from the current working directory. No new conftest or path plumbing is added by this plan.

```python
from pathlib import Path
from chart_ingest import class_names, build, validate
import pytest
ROOT = Path(__file__).resolve().parents[2]
def test_class_order_and_sibling_sum():
    assert class_names()[:3] == ["AA", "AKs", "AQs"]
    assert class_names()[13:15] == ["AKo", "KK"]
    t = {"bundle_id":"test", "depth_bb":100, "rake_profile":"undocumented",
         "straddle":False, "nodes":[{"history":[], "actor":"UTG", "page":2,
         "title":"UTG RFI", "actions":[{"step":"fold"},{"step":"raise","to_bb_x1000":2500}],
         "legend":{"F":[1,0],"R":[0,1],"M":[.5,.5]},
         "rows":[["F"]*13 for _ in range(13)]}]}
    t["nodes"][0]["rows"][0][0] = "M"
    e = build(t)
    assert e["nodes"][0]["weights"][0][0] == .5
    assert "evs" not in e["nodes"][0]
    validate(e)
    e["nodes"][0]["weights"][0][0] = .1
    with pytest.raises(ValueError, match="sum"):
        validate(e)
```

- [ ] **Step 2: Run red.** `python -m pytest tools/tests/test_chart_ingest.py -q`; expect missing module/functions.

- [ ] **Step 3: Implement class ordering and action-major conversion.** No nearest-hand substitution; no EV key; never infer a menu action from its color name.

```python
import json, math
RANKS = "AKQJT98765432"
def class_names():
    names = []
    for i, a in enumerate(RANKS):
        for j, b in enumerate(RANKS):
            names.append(a+a if i==j else a+b+"s" if i<j else b+a+"o")
    return names
def build(t):
    out = {k:t[k] for k in ("bundle_id","depth_bb","rake_profile","straddle")}
    out.update(class_order="A-2 row-major, section 4.1", nodes=[])
    for src in t["nodes"]:
        if len(src["rows"])!=13 or any(len(r)!=13 for r in src["rows"]):
            raise ValueError("grid shape")
        cells = [src["legend"][code] for row in src["rows"] for code in row]
        if any(len(v)!=len(src["actions"]) for v in cells):
            raise ValueError("legend action count")
        out["nodes"].append({"history":src["history"], "actor":src["actor"],
            "actions":src["actions"], "weights":[list(x) for x in zip(*cells)],
            "unreachable_classes":src.get("unreachable_classes",[])})
    validate(out)
    return out
def validate(e):
    seen = set()
    for n in e["nodes"]:
        key = json.dumps(n["history"], separators=(",",":"))
        if key in seen: raise ValueError("duplicate node")
        seen.add(key)
        # Without this, `zip(*[])` below yields nothing and a zero-action node
        # would pass the Python validator while the Rust loader rejects it.
        if not n["actions"]: raise ValueError("empty menu")
        if len(n["weights"])!=len(n["actions"]): raise ValueError("actions shape")
        if any(len(r)!=169 for r in n["weights"]): raise ValueError("169 shape")
        for c, row in enumerate(zip(*n["weights"])):
            if any(not math.isfinite(x) or not 0<=x<=1 for x in row):
                raise ValueError("probability bound")
            s = math.fsum(row)
            if c in n["unreachable_classes"]:
                if s != 0: raise ValueError("unreachable sum")
            elif abs(s-1)>1e-3: raise ValueError(f"class {c} sum {s}")
        if "evs" in n: raise ValueError("chart EV forbidden")
```

Use the Task 1 token/position/amount/duplicate rules in this validator as well; exercise them with explicit parametrized malformed rows. The Rust loader remains the final boundary validator. `verify` rebuilds from the transcription, compares exact serialized bytes and manifest SHA-256, and compares the inventory's covered keys to the actual output keys. Its nonzero exit includes the node and class name; no automatic repair.

- [ ] **Step 4: Implement fetching and CLI wiring.** The fetch mode must accept only the supplied public URL or a resolved public PDF link, write bytes, record the final redirected URL and SHA-256, and fail on HTML returned as a PDF.

```python
from pathlib import Path
from urllib.request import urlopen, Request
import hashlib, argparse
def fetch(url, output):
    with urlopen(Request(url, headers={"User-Agent":"PokerAI-chart-ingest/1"}), timeout=30) as r:
        data = r.read(64*1024*1024+1)
        final_url = r.url
    if len(data)>64*1024*1024: raise ValueError("64 MiB limit")
    if output.suffix.lower()==".pdf" and not data.startswith(b"%PDF-"):
        raise ValueError("publisher returned non-PDF; inspect download page links")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(data)
    print(json.dumps({"url":final_url,"sha256":hashlib.sha256(data).hexdigest(),"bytes":len(data)}))
def encoded(value):
    return (json.dumps(value, indent=2, allow_nan=False)+"\n").encode("utf-8")
```

The manifest must deserialize into the Rust `BundleInfo` of Task 1, which has no serde defaults: a manifest missing `game`, `version`, `depth_bb`, `depths`, `source_blinds`, `ev_unit` or `sha256` fails to deserialize, and `checked_envelope` additionally rejects anything whose `game != "nl"`, `version != 2`, `source_blinds != [0.5, 1.0]` or `ev_unit != "source_sb"`. Emit all fifteen fields, in the same shape `manifest_for` uses in `gen_preflop_fixtures.py`:

```python
BUNDLE_INFO_FIELDS = ["bundle_id","source","depth_bb","depths","source_blinds","rake_profile",
                      "rake","straddle","version","game","ev_unit","ev_reference",
                      "license_note","accuracy","sha256"]
def manifest_for_chart(envelope, transcription):
    raw = encoded(envelope)
    manifest = {
        "bundle_id": envelope["bundle_id"],
        "source": "ChartTranscription",
        "depth_bb": envelope["depth_bb"],
        "depths": [envelope["depth_bb"]],
        "source_blinds": [0.5, 1.0],
        "rake_profile": envelope["rake_profile"],
        # A chart that does not publish its rake is `null`, never the PokerData profile.
        "rake": transcription.get("rake"),
        "straddle": envelope["straddle"],
        "version": 2,
        "game": "nl",
        "ev_unit": "source_sb",
        "ev_reference": "unverified",
        "license_note": transcription["license_note"],
        "accuracy": transcription.get("accuracy", "unverified"),
        "sha256": hashlib.sha256(raw).hexdigest(),
    }
    if manifest["rake"] is None and manifest["rake_profile"] != "undocumented":
        raise ValueError("a chart without a published rake must use rake_profile 'undocumented'")
    if sorted(manifest) != sorted(BUNDLE_INFO_FIELDS):
        raise ValueError("manifest keys do not match the Rust BundleInfo field set")
    return manifest
```

`tools/tests/test_chart_ingest.py` asserts `sorted(manifest_for_chart(e, t)) == sorted(BUNDLE_INFO_FIELDS)` and that `BUNDLE_INFO_FIELDS` equals the fifteen field names of Task 1's `BundleInfo`, so a later field added on one side fails the Python suite before it fails `load_bundle`. Both fixtures ship `rake_profile: "undocumented"` with `rake: null` — the only bundles in this plan that exercise `rake_rank`'s unknown arm.

Wire `argparse` subcommands: `fetch URL OUTPUT`, `build TRANSCRIPTION OUTPUT MANIFEST`, `validate ENVELOPE`, `verify TRANSCRIPTION OUTPUT MANIFEST`. `build` writes `encoded(envelope)` to OUTPUT and `encoded(manifest_for_chart(envelope, transcription))` to MANIFEST, forces `source=ChartTranscription` and `ev_reference=unverified`, and hashes exactly the bytes it writes. `validate` prints every node's 169 class sums and aggregate minimum/maximum, exits nonzero on failure. `verify` prints only mismatches or a count of verified nodes/classes. Read the article HTML's anchor whose text is `6 max 200bb 500z GTO Ranges`; follow its public download page to the actual PDF using its observed link, preserving both URLs. A timeout does not justify a guessed S3 URL or a different-depth file.

- [ ] **Step 5: Verify and commit.** `python -m pytest tools/tests/test_chart_ingest.py -q`; `cargo test --workspace`.

```powershell
git add tools/chart_ingest.py tools/tests/test_chart_ingest.py docs/data/chart-transcription.md
git commit -m "feat(core-preflop): add reproducible chart ingestion tooling" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 4: Acquire the public chart sources and record which depths shipped

Transcription is externally blocked work and was previously one 8-step task with a single commit; it is split into four review-gated tasks (4 acquisition, 5 the 100bb grids, 6 the 200bb grids, 7 the frozen coverage handoff) so a reviewer can accept the acquisition record without accepting a transcription, and so an interrupted transcription keeps its finished grids.

**Files:** Create `fixtures/charts/sources/pokercoaching_100.pdf`, `fixtures/charts/sources/rangeconverter_200.html`, `fixtures/charts/sources/rangeconverter_200.pdf` (conditional, see Step 3), `fixtures/charts/sources.manifest.json`; modify `docs/data/chart-transcription.md`, `tools/tests/test_chart_ingest.py`.

**Interfaces:**
- Consumes: `chart_ingest.py fetch`.
- Produces: `fixtures/charts/sources.manifest.json`, the machine-readable answer to "which chart depths does this build actually have?", read by Tasks 5–7, 17 and 19 and handed to Plan 4:

```json
{"schema_version": 1,
 "depths": [
   {"depth_bb": 100, "bundle_id": "pokercoaching_100", "status": "available",
    "source_file": "fixtures/charts/sources/pokercoaching_100.pdf",
    "final_url": "<recorded by fetch>", "sha256": "<recorded by fetch>", "bytes": 0,
    "acquired_utc": "<YYYY-MM-DDTHH:MM:SSZ>", "note": ""},
   {"depth_bb": 200, "bundle_id": "rangeconverter_200", "status": "available",
    "source_file": "fixtures/charts/sources/rangeconverter_200.pdf",
    "final_url": "<recorded by fetch>", "sha256": "<recorded by fetch>", "bytes": 0,
    "acquired_utc": "<YYYY-MM-DDTHH:MM:SSZ>", "note": ""}]}
```

`status` is `"available"` or `"unsupported"`. A depth that could not be acquired keeps its row with `status: "unsupported"`, `source_file: null`, `sha256: null`, and `note` beginning with the exact token `depth 200 unsupported` (or `depth 100 unsupported`) followed by the attempts made. Nothing downstream reads a chart bundle whose depth row is not `"available"`.

- [ ] **Step 1: Add the failing availability test.** In `tools/tests/test_chart_ingest.py`:

```python
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[2]
def load_availability():
    return json.loads((ROOT / "fixtures/charts/sources.manifest.json").read_text(encoding="utf-8"))
def available_depths():
    return {d["depth_bb"]: d for d in load_availability()["depths"] if d["status"] == "available"}
def test_sources_manifest_shape():
    m = load_availability()
    assert m["schema_version"] == 1
    assert [d["depth_bb"] for d in m["depths"]] == [100, 200]
    for d in m["depths"]:
        assert d["status"] in ("available", "unsupported")
        if d["status"] == "available":
            assert (ROOT / d["source_file"]).exists()
            assert len(d["sha256"]) == 64 and d["bytes"] > 0
        else:
            assert d["source_file"] is None and d["sha256"] is None
            assert d["note"].startswith(f"depth {d['depth_bb']} unsupported")
    assert available_depths(), "no chart depth acquired; the release has no range source"
```

- [ ] **Step 2: Run red, then fetch the 100bb source.** `python -m pytest tools/tests/test_chart_ingest.py -q` fails on the missing manifest. Then:

```powershell
python tools/chart_ingest.py fetch https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf fixtures/charts/sources/pokercoaching_100.pdf
```

`fetch` prints one JSON line with the keys `url` (the final redirected URL), `sha256` and `bytes` and refuses HTML returned as a PDF. Copy those three values into the depth-100 row. Planning-time verification (2026-09-10): PokerCoaching is a six-page PDF and the URL responded.

- [ ] **Step 3: Fetch the 200bb source through its download page, with an explicit fallback chain.** Planning-time verification (2026-09-10): RangeConverter's article page is accessible, but its PDF download timed out. Attempt these in order and record every attempt (UTC time, URL, outcome) in `docs/data/chart-transcription.md`:

```powershell
# 3a. the article page, which carries the download link
python tools/chart_ingest.py fetch https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem fixtures/charts/sources/rangeconverter_200.html
# 3b. the publisher download page linked from the anchor whose text is "6 max 200bb 500z GTO Ranges"
python tools/chart_ingest.py fetch https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash fixtures/charts/sources/rangeconverter_200.download.html
# 3c. the actual PDF link observed on 3b (never a guessed S3 path, never a different depth)
python tools/chart_ingest.py fetch <observed PDF url from 3b> fixtures/charts/sources/rangeconverter_200.pdf
```

Retry 3a and 3b once each after 60 seconds before moving on; a timeout is not evidence that the file is gone. If 3c still fails, try one archived copy: `python tools/chart_ingest.py fetch https://web.archive.org/web/2/<the observed PDF url> fixtures/charts/sources/rangeconverter_200.pdf`. An archived copy is acceptable only when `fetch` confirms the `%PDF-` magic and the document's own title and depth match the 200bb chart set; record the archive URL and timestamp as the `final_url`. Do not substitute a different depth, a different publisher, or a screenshot.

If the article page (3a) succeeded, commit it regardless — it is the provenance record — after stripping any account or session information from the saved HTML.

- [ ] **Step 4: Write `sources.manifest.json` and the acquisition record.** For each depth write the row with the values `fetch` printed. If Step 3 exhausted every attempt, write the depth-200 row as:

```json
{"depth_bb": 200, "bundle_id": "rangeconverter_200", "status": "unsupported",
 "source_file": null, "final_url": null, "sha256": null, "bytes": 0,
 "acquired_utc": "<attempt time>",
 "note": "depth 200 unsupported: article page <ok|timeout>, download page <ok|timeout>, PDF link <timeout|404>, web archive <miss>; retried each once at 60 s"}
```

In `docs/data/chart-transcription.md` record, per acquired source: acquisition UTC date, final URL, bytes and SHA-256, document title and version, page count, depth, the action-size legend, the rake disclosure (or its absence) and the rounding rule. For an unavailable source record the attempt log instead, and state that the release ships the remaining depth only.

- [ ] **Step 5: Run green and commit.** `python -m pytest tools/tests/test_chart_ingest.py -q`; `cargo test --workspace` (unchanged — no Rust file is touched by this task).

```powershell
git add fixtures/charts/sources fixtures/charts/sources.manifest.json docs/data/chart-transcription.md tools/tests/test_chart_ingest.py
git commit -m "chore(charts): freeze public chart source bytes and depth availability" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 5: Inventory and transcribe the PokerCoaching 100bb grids

**Files:** Create `fixtures/charts/transcription/pokercoaching_100.json`, `fixtures/charts/pokercoaching_100.json`, `fixtures/charts/pokercoaching_100.manifest.json`; modify `docs/data/chart-transcription.md`, `tools/tests/test_chart_ingest.py`.

**Interfaces:**
- Consumes: `chart_ingest.py build/validate/verify`, Task 3's transcription and inventory-row shape, Task 4's `sources.manifest.json`.
- Produces: the 100bb `ChartTranscription` bundle and its hashed manifest, plus a complete covered/absent inventory. Every frequency is a transcription of a published cell, not a calculated strategy.

- [ ] **Step 1: Add the failing committed-bundle test for 100bb.**

```python
import json
import pytest
from chart_ingest import build, validate
def _bundle(name):
    p = ROOT / "fixtures/charts"
    e = json.loads((p / f"{name}.json").read_text(encoding="utf-8"))
    t = json.loads((p / "transcription" / f"{name}.json").read_text(encoding="utf-8"))
    return e, t
@pytest.mark.skipif(100 not in available_depths(), reason="depth 100 unsupported")
def test_pokercoaching_100_bundle():
    e, t = _bundle("pokercoaching_100")
    validate(e)
    assert e["depth_bb"] == 100
    assert all("evs" not in n for n in e["nodes"])
    assert build(t) == e
    assert {json.dumps(n["history"]) for n in e["nodes"]} == {
        json.dumps(r["history"]) for r in t["inventory"] if r["status"] == "covered"}
    assert any(r["status"] == "absent" for r in t["inventory"]), "the absent audit is part of the record"
```

- [ ] **Step 2: Run red.** `python -m pytest tools/tests/test_chart_ingest.py -q`; expect the missing transcription/bundle files.

- [ ] **Step 3: Inventory the PDF one page at a time.** The [public PDF](https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf) text establishes these grids; visually inspect all six pages and legends, including the last page which has no extracted text. Use `LJ` as `UTG` in normalized keys. Write one inventory row per checklist line, in Task 3's row shape, before transcribing anything.

| Node checklist | Published pages / required treatment |
|---|---|
| RFI UTG, HJ, CO, BTN, SB | Page 2; all preceding positions explicitly folded. |
| HJ vs UTG; CO vs UTG/HJ; BTN vs UTG/HJ/CO | Page 3; one opener, intervening folds. |
| SB vs UTG/HJ/CO/BTN; BB vs UTG/HJ/CO/BTN | Page 4; one opener, every intervening fold. |
| SB first-in strategy; BB vs SB limp; BB vs SB raise | Page 5; SB first-in overlaps the page-2 key: reconcile the displayed split policy, do not create duplicate nodes. |
| RFI response to a later 3bet | Inspect legend and final page: some first-action colors may encode a future response. Only add a distinct vs-3bet node if a complete conditional strategy, opponent-position scope, and sizes are explicitly supplied. Otherwise inventory `absent`; do not mistake raise/call and raise/fold annotations for independent current actions. |
| BB RFI; squeezes; cold calls vs 3bet; vs-4bet | Not established by the extracted grids; mark absent unless a complete grid and exact prefix is visually present. |

The published sizing instruction establishes 2.5bb opens except SB 3bb; IP 3bets 3.5×, OOP 4×; BB vs SB limp 3.5bb; OOP 4bets 2.5×, IP 2.3×. Freeze resolved raise-to values, not labels. For each chart title compute its complete folded prefix and actual next actor. A BB defense after an opener and folds is distinct from the same defense after a caller. No chart from one history is inserted under another. Commit the inventory before the first grid:

```powershell
git add fixtures/charts/transcription/pokercoaching_100.json docs/data/chart-transcription.md
git commit -m "chore(charts): inventory pokercoaching_100 node coverage" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 4: Transcribe one grid (repeat this checkbox once per inventory row marked covered).** Record 13 rows of 13 cell codes and a local legend; pure colors are 0/1; preserve PokerCoaching's actual displayed implemented choices. Read high rank first for offsuit cells. For an uncolored cell use the legend's documented fold/check action, not an unconditional fold default. `unreachable_classes` is used only when the source explicitly provides no data for a conditional class. A class absent because the opener never reaches the response may be declared unreachable only after checking the preceding source action's class weight is zero. If the response chart supplies a strategy there, preserve it. Rebuild after each grid:

```powershell
python tools/chart_ingest.py build fixtures/charts/transcription/pokercoaching_100.json fixtures/charts/pokercoaching_100.json fixtures/charts/pokercoaching_100.manifest.json
python tools/chart_ingest.py validate fixtures/charts/pokercoaching_100.json
```

- [ ] **Step 5: Independently reread that grid, then commit it (repeat with Step 4, one grid per repetition).** Compare all 169 cells in reverse row order against the rendered source, with the first-pass JSON hidden while interpreting each row. Compare class/action totals, all boundary hands and every mixed cell. Record page, title, checked class count = 169, corrections and verification completion in the documentation. Never use a sum-to-one check as evidence that the correct cell was transcribed. Render/download inspection is development work; the plan writer does not produce those files. Then commit that grid alone, so an interrupted transcription keeps everything already verified:

```powershell
git add fixtures/charts/transcription/pokercoaching_100.json fixtures/charts/pokercoaching_100.json fixtures/charts/pokercoaching_100.manifest.json docs/data/chart-transcription.md
git commit -m "chore(charts): transcribe <page N, exact chart title>" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 6: Verify the whole bundle and commit.** `python tools/chart_ingest.py verify fixtures/charts/transcription/pokercoaching_100.json fixtures/charts/pokercoaching_100.json fixtures/charts/pokercoaching_100.manifest.json`; `python -m pytest tools/tests/test_chart_ingest.py -q`; `cargo test --workspace` (no Rust file is touched yet — the Rust load is Task 7).

```powershell
git add fixtures/charts docs/data/chart-transcription.md tools/tests/test_chart_ingest.py
git commit -m "feat(core-preflop): transcribe and verify the pokercoaching 100bb chart bundle" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 6: Inventory and transcribe the RangeConverter 200bb grids

**Files:** Create `fixtures/charts/transcription/rangeconverter_200.json`, `fixtures/charts/rangeconverter_200.json`, `fixtures/charts/rangeconverter_200.manifest.json` (all three only when depth 200 is `available`); modify `docs/data/chart-transcription.md`, `tools/tests/test_chart_ingest.py`.

**Interfaces:**
- Consumes: `chart_ingest.py build/validate/verify`, Task 3's transcription and inventory-row shape, Task 4's `sources.manifest.json`.
- Produces: the 200bb `ChartTranscription` bundle and its hashed manifest **when depth 200 is available**; otherwise a recorded, tested absence that leaves the workspace green.

- [ ] **Step 1: Add the failing committed-bundle test for 200bb, gated on availability.**

```python
@pytest.mark.skipif(200 not in available_depths(), reason="depth 200 unsupported")
def test_rangeconverter_200_bundle():
    e, t = _bundle("rangeconverter_200")
    validate(e)
    assert e["depth_bb"] == 200
    assert all("evs" not in n for n in e["nodes"])
    assert build(t) == e
    assert {json.dumps(n["history"]) for n in e["nodes"]} == {
        json.dumps(r["history"]) for r in t["inventory"] if r["status"] == "covered"}
def test_unavailable_depth_ships_no_bundle():
    for d in load_availability()["depths"]:
        if d["status"] == "unsupported":
            assert not (ROOT / f"fixtures/charts/{d['bundle_id']}.json").exists()
            assert not (ROOT / f"fixtures/charts/{d['bundle_id']}.manifest.json").exists()
```

The second test is the contingency's guard: an unavailable depth must ship **no** bundle rather than an empty or partly transcribed one.

- [ ] **Step 2: Run red, and branch on availability.** `python -m pytest tools/tests/test_chart_ingest.py -q`. If depth 200 is `unsupported` in `sources.manifest.json`, the 200bb case skips, `test_unavailable_depth_ships_no_bundle` passes, and this task consists of Step 6 only: record in `docs/data/chart-transcription.md` that the 200bb chart family is not shipped, that Plan 4's 200bb bench spots (§13.5) and tier-3 pre-solver scenarios (§10.5) therefore have no chart range source, and commit. Do not create an empty `rangeconverter_200.json`. Otherwise continue with Steps 3–6.

- [ ] **Step 3: Inventory the PDF one page at a time.** The [200bb article](https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem) establishes RFI, vs-RFI, and vs-3bet chart families and 50% rounding. Use this finite audit matrix, writing one inventory row per candidate — `covered` with PDF page, title and resolved sizes, or `absent` with the visual audit result. The matrix is an audit checklist, not a claim that every candidate is in the PDF:

| Family | Candidate decisions to inspect individually |
|---|---|
| RFI | UTG; HJ (publisher may call it MP); CO; BTN; SB. |
| vs-RFI | HJ vs UTG; CO vs UTG/HJ; BTN vs UTG/HJ/CO; SB vs UTG/HJ/CO/BTN; BB vs UTG/HJ/CO/BTN/SB. |
| vs-3bet | UTG vs HJ/CO/BTN/SB/BB; HJ vs CO/BTN/SB/BB; CO vs BTN/SB/BB; BTN vs SB/BB; SB vs BB. |
| Blind limp lines | SB first-in limp strategy; BB vs SB limp; SB vs BB isolation. |
| Absent-node audit | Multiway callers, squeezes, cold-call-vs-3bet, vs-4bet, open-limps outside SB. |

Do not copy sizes from RangeConverter's unrelated reports or the PokerCoaching PDF. Record each resolved size from this PDF. For a vs-3bet key include folds before the open, between open and 3bet, and after the 3bettor until action returns to the opener. If the PDF lacks sufficient size information, that node is unloadable and recorded as absent-with-unresolved-size, never silently assigned a size. Commit the inventory before the first grid:

```powershell
git add fixtures/charts/transcription/rangeconverter_200.json docs/data/chart-transcription.md
git commit -m "chore(charts): inventory rangeconverter_200 node coverage" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 4: Transcribe one grid (repeat this checkbox once per inventory row marked covered).** Record 13 rows of 13 cell codes and a local legend; pure colors are 0/1 and equal-split cells are 0/.5/1 for RangeConverter's 50% rounding. Read high rank first for offsuit cells. For an uncolored cell use the legend's documented fold/check action, not an unconditional fold default. `unreachable_classes` is used only when the source explicitly provides no data for a conditional class, and only after checking that the preceding source action's class weight is zero. Rebuild after each grid:

```powershell
python tools/chart_ingest.py build fixtures/charts/transcription/rangeconverter_200.json fixtures/charts/rangeconverter_200.json fixtures/charts/rangeconverter_200.manifest.json
python tools/chart_ingest.py validate fixtures/charts/rangeconverter_200.json
```

- [ ] **Step 5: Independently reread that grid, then commit it (repeat with Step 4, one grid per repetition).** Compare all 169 cells in reverse row order against the rendered source, with the first-pass JSON hidden while interpreting each row. Compare class/action totals, all boundary hands and every mixed cell. Record page, title, checked class count = 169, corrections and verification completion in the documentation. Never use a sum-to-one check as evidence that the correct cell was transcribed.

```powershell
git add fixtures/charts/transcription/rangeconverter_200.json fixtures/charts/rangeconverter_200.json fixtures/charts/rangeconverter_200.manifest.json docs/data/chart-transcription.md
git commit -m "chore(charts): transcribe <page N, exact chart title>" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

- [ ] **Step 6: Verify (or record the absence) and commit.** When the bundle exists: `python tools/chart_ingest.py verify fixtures/charts/transcription/rangeconverter_200.json fixtures/charts/rangeconverter_200.json fixtures/charts/rangeconverter_200.manifest.json`. Either way: `python -m pytest tools/tests/test_chart_ingest.py -q`; `cargo test --workspace`.

```powershell
git add fixtures/charts docs/data/chart-transcription.md tools/tests/test_chart_ingest.py
git commit -m "feat(core-preflop): transcribe and verify the rangeconverter 200bb chart bundle" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 7: Freeze chart coverage and load both layers through the Rust boundary

**Files:** Modify `docs/data/chart-transcription.md`, `tools/tests/test_chart_ingest.py`, `crates/core-preflop/tests/envelope.rs`.

**Interfaces:**
- Consumes: Task 2's `load_bundle`, Tasks 4–6's bundles and `sources.manifest.json`, `chart_ingest.py verify`.
- Produces: the frozen covered/missing node checklist and source hashes that Plan 4 reads before generating any bench suite or 50-hand input; the Rust-side proof that each shipped chart bundle loads as `ChartTranscription` with no EV.

- [ ] **Step 1: Add the failing Rust chart-load test.** In `crates/core-preflop/tests/envelope.rs`. It reads the same availability manifest, so an unshipped depth is skipped rather than failed:

```rust
#[test]
fn published_chart_bundles_load(){
    let root=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let raw=std::fs::read(root.join("fixtures/charts/sources.manifest.json")).unwrap();
    let m:serde_json::Value=serde_json::from_slice(&raw).unwrap();
    let mut loaded=0;
    for d in m["depths"].as_array().unwrap(){
        if d["status"]!="available"{continue;}
        let id=d["bundle_id"].as_str().unwrap();
        let dir=root.join("fixtures/charts");
        let source=core_preflop::load_bundle(
            &dir.join(format!("{id}.manifest.json")),&dir.join(format!("{id}.json"))).unwrap();
        let info=source.bundle_info();
        assert_eq!(info.source,core_preflop::SourceKind::ChartTranscription);
        assert_eq!(info.depth_bb as u64,d["depth_bb"].as_u64().unwrap());
        assert_eq!(info.ev_reference,core_preflop::EvReference::Unverified);
        assert_eq!(info.rake_profile,"undocumented");
        assert!(info.rake.is_none(),"an unpublished chart rake is never assigned a profile");
        assert!(info.nodes_have_no_ev(),"charts omit evs entirely");
        loaded+=1;
    }
    assert!(loaded>0,"at least one chart depth must ship");
}
```

`nodes_have_no_ev(&self) -> bool` is a two-line inherent method on `BundleInfo`'s owning adapters added here: `self.nodes.values().all(|n| n.ev_source_sb.is_none())` on both `PokerDataJson` and `ChartTranscription`, exposed through a `PreflopSource::nodes_have_no_ev` default of `true` only where the map is reachable — implement it as an added trait method with the body above on both adapters, not as a downcast.

- [ ] **Step 2: Run red.** `cargo test -p core-preflop --test envelope published_chart_bundles_load`; expect the missing method and, if Tasks 5–6 have not run, missing files.

- [ ] **Step 3: Freeze node coverage and the benchmark fallbacks.** `verify` requires equality between covered inventory keys and bundle keys; all absent candidates stay out. In `docs/data/chart-transcription.md` document the exact available prefixes: BTN/CO/HJ/UTG open → BB call at 100bb, and BTN/CO open → BB call plus BTN open → BB 3bet → BTN call at 200bb **when depth 200 shipped and those nodes are actually covered**. A missing response contributes the §9.3 frozen-range fallback with `UnconditionedPriorStreet`; label that scenario explicitly as a synthetic missing-node fallback for Plan 4. It never becomes a chart node, and it is never reported to Plan 4 as a fully conditioned benchmark. Add the explicit Plan 4 handoff paragraph: which depths shipped, which prefixes are covered at each, and — if depth 200 is `unsupported` — that §13.5's 200bb baseline spots and §10.5's tier-3 pre-solver scenarios have no chart range source in this build.

- [ ] **Step 4: Run both validation layers.** `python tools/chart_ingest.py verify <transcription> <envelope> <manifest>` for every shipped bundle; `python -m pytest tools/tests/test_chart_ingest.py -q`; `cargo test -p core-preflop --test envelope`; `cargo test --workspace`. Hashes and inventory must be frozen at this point, before Plan 4 generates any bench suite or 50-hand input.

- [ ] **Step 5: Commit.**

```powershell
git add fixtures/charts docs/data/chart-transcription.md tools/tests/test_chart_ingest.py crates/core-preflop
git commit -m "feat(core-preflop): freeze chart coverage and load chart bundles in Rust" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 8: Reconstruct each prefix and select depth, rake and virtual roles

**Files:** Create `crates/core-preflop/src/{lookup.rs,depth.rs,straddle.rs}`, `crates/core-preflop/tests/lookup.rs`; modify `src/{lib.rs,store.rs}`.

**Interfaces:**
- Consumes: `derive(&HandState) -> Derived`, Task 2 source maps and keys, `HandState.actions: Vec<TakenAction>`.
- Produces, with these exact signatures: `prefix_state(state: &HandState, prefix_len: usize) -> HandState`; `start_stack(state: &HandState, seat: Seat) -> u32`; `physical_positions(s: &HandState) -> Vec<(Seat,Position)>`; `short_handed_prefix(n: usize) -> Vec<(Position,PreflopStep)>`; `source_unit(cfg: &HandConfig) -> u32`; `virtual_position(p: Position, mapped: bool) -> Position`; `normalized_posts(sb: u32, bb: u32, s: u32) -> [f32; 3]`; `check_straddle(state: &HandState) -> Result<(), RulesError>`; `depth_for(actor: u32, others: &[u32], unit: u32) -> f64`; `bucket(actual: f64, available: &[u16]) -> Option<u16>`; `prominent_depth(actual: f64, used: u16) -> bool`; `asymmetric(stacks: &[f64], used: u16) -> (bool,bool)`; `cap_bb(cap_mchips: u32, unit: u32) -> f64`; `rake_rank(actual: &Rake, candidate: &BundleInfo, unit: u32) -> (u8,f64,f64,u8,f64)`; `rake_reason(actual: &Rake, candidate: &BundleInfo, unit: u32) -> Option<ApproxReason>`; `rank_key(candidate: &BundleInfo, actual_depth: f64, actual_rake: &Rake, unit: u32) -> (u8,f64,u8,(u8,f64,f64,u8,f64))`; `observed_history(prefix: &HandState, roles: &[(Seat,Position)], mapped: bool, unit: u32) -> Vec<(Position,PreflopStep)>`; `PreflopStore::query(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize) -> PreflopAnswer`.
- `PreflopAnswer` is Plan 3-owned: `pub struct PreflopAnswer { pub key: String, pub actor: Option<Seat>, pub node: Option<PreflopNode>, pub bundle: Option<BundleInfo>, pub unit: u32, pub reasons: Vec<ApproxReason>, pub notes: Vec<String>, pub unsupported: Option<UnsupportedReason> }` with `PreflopAnswer::empty() -> Self` (all fields `None`/empty, `unit: 0`). It gains `expanded: Option<ExpandedNode>` in Task 9; do not fabricate a `NodeStrategy` with zero EVs.

- [ ] **Step 1: Write failing depth/rake/straddle tests with the exact thresholds.**

```rust
#[test]
fn depth_bucket_labels_per_prefix() {
    use core_preflop::{asymmetric,bucket,depth_for,prominent_depth};
    let acquired = [20,30,40,50,70,100,150,200];
    for (actual,used,prominent) in [(97.,100,false),(120.,100,true),(125.,150,true),(260.,200,true)] {
        assert_eq!(bucket(actual,&acquired),Some(used));
        assert_eq!(prominent_depth(actual,used),prominent);
    }
    assert_eq!(depth_for(100,&[104],1),100.0);
    // §8.3: label AsymmetricStacks on any difference; 5% controls prominence only.
    assert_eq!(asymmetric(&[100.0,104.0],100),(true,false));
    assert_eq!(asymmetric(&[100.0,110.0],100),(true,true));
    assert_eq!(asymmetric(&[100.0,100.0],100),(false,false));
}
#[test]
fn straddle_mapping_labels() {
    use proto::Position::*;
    for (physical,virtual_role) in [(Hj,Utg),(Co,Hj),(Btn,Co),(Sb,Btn),(Bb,Sb),(Utg,Bb)] {
        assert_eq!(core_preflop::virtual_position(physical,true),virtual_role);
    }
    assert_eq!(core_preflop::normalized_posts(1,2,4),[0.25,0.5,1.0]);
    assert_eq!(core_preflop::normalized_posts(2,5,10),[0.2,0.5,1.0]);
    // §8.3 rejects a short post, a short-handed straddle and a re-straddle before lookup.
    let short = straddled_state(/*sb*/1,/*bb*/2,/*S*/3,/*dealt*/6,/*stacks*/400);  // S < 2 * bb
    assert!(matches!(core_preflop::check_straddle(&short),
        Err(proto::RulesError::FormatUnsupported{..})));
    let five_handed = straddled_state(1,2,4,5,400);
    assert!(matches!(core_preflop::check_straddle(&five_handed),
        Err(proto::RulesError::FormatUnsupported{..})));
    let cannot_post = straddled_state(1,2,4,6,3);        // stack below the full straddle
    assert!(matches!(core_preflop::check_straddle(&cannot_post),
        Err(proto::RulesError::FormatUnsupported{detail})
        if detail=="straddler's starting stack does not cover the straddle"));
    // A re-straddle cannot be represented: `HandConfig.straddle` is `Option<UtgStraddle>`.
    assert!(serde_json::from_str::<proto::HandConfig>(
        r#"{"config_revision":0,"sb_chips":1,"bb_chips":2,
            "straddle":[{"amount_chips":4},{"amount_chips":8}],
            "rake":{"kind":"time_charge"},"chip_label":"$1"}"#).is_err());
}
```

`straddled_state(sb,bb,s,dealt,stacks)` is a helper in this test file that builds a `HandConfig` with `straddle: Some(UtgStraddle{amount_chips:s})` and calls Plan 1's `begin_hand` with `dealt` seats of `stacks` chips each, so every case above is an actual `HandState`, not a hand-edited one.

Add `rake_profile_ordering` using actual `PotRake{rate:.10,cap_mchips:6000,no_flop_no_drop:true}`, BB=5: cap_bb=1.2 maps to a candidate `.05/.5/true` with `RakeProfileMapped`; `TimeCharge` picks unraked before any raked bundle, otherwise smallest cap. Candidate ties on cap distance then rate then collection-rule mismatch then lower cap; equal remaining rank chooses smaller bundle id. The rank tuple alone is not the requirement: the test must also drive `PreflopStore::query` and assert that the `5% cap 0.5bb` bundle is the one **selected** and that `PreflopAnswer.reasons` contains `ApproxReason::RakeProfileMapped{actual,used}` with `used == "5% cap 0.5bb"`, which is what §13.1's row states. Likewise the 100-versus-104 case is asserted through `query`: `PreflopAnswer.reasons` contains `ApproxReason::AsymmetricStacks{stacks_bb,prominent:false}` and the answer's coverage contribution is never `Exact`.

```rust
#[test]
fn rake_profile_ordering(){
    let mut source:core_preflop::BundleInfo=serde_json::from_str(include_str!(
        "../../../fixtures/preflop/synthetic_v2/manifest.json")).unwrap();
    let actual=proto::Rake::PotRake{rate:0.10,cap_mchips:6000,no_flop_no_drop:true};
    let rank=core_preflop::rake_rank(&actual,&source,5);
    assert!((rank.1-0.7).abs()<1e-6);
    assert!((rank.2-0.05).abs()<1e-6);
    assert_eq!(rank.3,0);
    let raked=core_preflop::rake_rank(&proto::Rake::TimeCharge,&source,5);
    source.rake=Some(core_preflop::RakeProfile{rate:0.0,cap_bb:0.0,no_flop_no_drop:true});
    let free=core_preflop::rake_rank(&proto::Rake::TimeCharge,&source,5);
    assert!(free.0<raked.0);
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-preflop --test lookup`; expect missing mapping functions.

- [ ] **Step 3: Implement pure depth and role functions.** Binding formulas: `depth = min(actor.stack_start, max over other eligible seats' stack_start) / unit`; `DepthBucket` whenever `actual != used`, prominent iff `abs(actual-used)/used > .05`; AsymmetricStacks whenever **any** eligible starting stack differs from used, prominent iff any relative difference exceeds .05. Eligible means not folded **at this prefix**, including all-in players; stack values are starting stacks, not remaining stacks, and every one of them is read through `start_stack(state, seat)` below.

```rust
/// `HandState.stacks_start` is aligned with `HandState.dealt` (Plan 1, spec §4.3 S6),
/// while `Derived`'s per-seat vectors are indexed by `Seat.0`. Indexing `stacks_start`
/// by `seat.0` silently returns another seat's stack, which would give a wrong depth
/// bucket, wrong `AsymmetricStacks` and a wrong `MissingPreflopNode` rate. Every depth
/// and eligibility computation below goes through this function; none indexes
/// `stacks_start` directly.
pub fn start_stack(state: &HandState, seat: Seat) -> u32 {
    let i = state.dealt.iter().position(|&s| s == seat).expect("dealt seat");
    state.stacks_start[i]
}
pub fn depth_for(actor: u32, others: &[u32], unit: u32) -> f64 {
    actor.min(others.iter().copied().max().unwrap_or(actor)) as f64 / unit as f64
}
/// Nearest acquired depth, ties deeper. The `d <= 200` filter implements §8.3's "above
/// 200 clamps to 200" only while 200 is itself an acquired depth: with 200 present, the
/// nearest-under-200 choice for any actual above 200 is 200 exactly. That holds for the
/// chart set and for the eight PokerData depths; a future bundle set whose deepest
/// acquired depth is below 200 must clamp explicitly instead of relying on this filter.
pub fn bucket(actual: f64, available: &[u16]) -> Option<u16> {
    available.iter().copied().filter(|&d| d<=200).min_by(|a,b| {
        (actual-*a as f64).abs().total_cmp(&(actual-*b as f64).abs())
            .then_with(|| b.cmp(a))
    })
}
pub fn prominent_depth(actual: f64, used: u16) -> bool {
    (actual-used as f64).abs()/used as f64 > 0.05
}
/// Returns `(label, prominent)` for `ApproxReason::AsymmetricStacks{stacks_bb, prominent}`
/// (§4.4 as amended by revision 6 S11): label on any difference, prominent above 5%.
pub fn asymmetric(stacks: &[f64], used: u16) -> (bool,bool) {
    (stacks.iter().any(|&s| s!=used as f64),
     stacks.iter().any(|&s| (s-used as f64).abs()/used as f64>0.05))
}
pub fn virtual_position(p: Position, mapped: bool) -> Position {
    use Position::*;
    if !mapped { return p; }
    match p { Hj=>Utg, Co=>Hj, Btn=>Co, Sb=>Btn, Bb=>Sb, Utg=>Bb }
}
pub fn normalized_posts(sb:u32, bb:u32, s:u32) -> [f32;3] {
    [sb as f32/s as f32,bb as f32/s as f32,1.0]
}
```

Physical positions use dealt seats sorted clockwise from button; indices 0,1,2 are BTN,SB,BB, remaining names are the last `n-3` of UTG,HJ,CO. `short_handed_prefix(n: usize) -> Vec<(Position,PreflopStep)>` returns UTG,HJ,CO folded at n=3; UTG,HJ at n=4; UTG at n=5; empty at n=6. These are lookup-only folds: do not append them to `HandState.actions` or create ranges for them. Emit `ShortHandedMapped{dealt}`.

```rust
pub fn source_unit(cfg:&HandConfig)->u32{
    cfg.straddle.as_ref().map(|s|s.amount_chips).unwrap_or(cfg.bb_chips)
}
pub fn physical_positions(s:&HandState)->Vec<(Seat,Position)>{
    use Position::*;
    let mut seats=s.dealt.clone();
    seats.sort_by_key(|seat|(seat.0+6-s.button.0)%6);
    let n=seats.len();assert!((3..=6).contains(&n));
    seats.into_iter().enumerate().map(|(i,seat)|{
        (seat,match i{0=>Btn,1=>Sb,2=>Bb,_=>[Utg,Hj,Co][6-n+i-3]})
    }).collect()
}
pub fn short_handed_prefix(n:usize)->Vec<(Position,PreflopStep)>{
    assert!((3..=6).contains(&n));
    [Position::Utg,Position::Hj,Position::Co].into_iter().take(6-n)
        .map(|p|(p,PreflopStep::Fold)).collect()
}
```

- [ ] **Step 4: Implement prefix reconstruction and deterministic candidate ranking.**

```rust
pub fn prefix_state(state: &HandState, prefix_len: usize) -> HandState {
    assert!(prefix_len <= state.actions.len());
    let mut prefix = state.clone();
    prefix.actions.truncate(prefix_len);
    prefix.board.clear();
    prefix.phase = proto::HandPhase::Betting { street: proto::Street::Preflop };
    prefix.derived = core_model::derive(&prefix);
    prefix
}
pub fn cap_bb(cap_mchips:u32, unit:u32) -> f64 {
    cap_mchips as f64 / (1000.0*unit as f64)
}
pub fn rake_rank(actual:&proto::Rake,candidate:&BundleInfo,unit:u32)->(u8,f64,f64,u8,f64){
    let Some(c)=&candidate.rake else{return (2,f64::INFINITY,f64::INFINITY,1,f64::INFINITY)};
    match actual{
        proto::Rake::TimeCharge=>(if c.rate==0.0 || c.cap_bb==0.0{0}else{1},
            c.cap_bb as f64,c.rate as f64,0,c.cap_bb as f64),
        proto::Rake::PotRake{rate,cap_mchips,no_flop_no_drop}=>(0,
            (c.cap_bb as f64-cap_bb(*cap_mchips,unit)).abs(),
            (c.rate as f64-*rate as f64).abs(),u8::from(c.no_flop_no_drop!=*no_flop_no_drop),c.cap_bb as f64),
    }
}
/// `RakeProfileMapped` on any difference (§8.3: "Any difference is `RakeProfileMapped`").
/// An undocumented source rake always maps, with the fixed `used` string.
pub fn rake_reason(actual:&proto::Rake,candidate:&BundleInfo,unit:u32)->Option<ApproxReason>{
    let used=match &candidate.rake{
        None=>"undocumented chart rake".to_string(),
        Some(c)=>{
            let exact=match actual{
                proto::Rake::TimeCharge=>c.rate==0.0 && c.cap_bb==0.0,
                proto::Rake::PotRake{rate,cap_mchips,no_flop_no_drop}=>
                    c.rate==*rate && (c.cap_bb as f64-cap_bb(*cap_mchips,unit)).abs()<1e-9
                        && c.no_flop_no_drop==*no_flop_no_drop,
            };
            if exact{return None;}
            candidate.rake_profile.clone()
        }
    };
    Some(ApproxReason::RakeProfileMapped{actual:format!("{actual:?}"),used})
}
/// Lexicographic candidate ranking: source kind (PokerData before charts), nearest
/// depth with the deeper tie, rake rank, then (applied by the caller) bundle id.
pub fn rank_key(candidate:&BundleInfo,actual_depth:f64,actual_rake:&proto::Rake,unit:u32)
    ->(u8,f64,u8,(u8,f64,f64,u8,f64)){
    let kind=match candidate.source{SourceKind::PokerDataJson=>0,SourceKind::ChartTranscription=>1};
    let used=bucket(actual_depth,&candidate.depths);
    let distance=used.map(|d|(actual_depth-d as f64).abs()).unwrap_or(f64::INFINITY);
    // 0 for the deeper of two equidistant depths, so ties resolve deeper.
    let shallower=used.map(|d|u8::from((d as f64)<actual_depth)).unwrap_or(1);
    (kind,distance,shallower,rake_rank(actual_rake,candidate,unit))
}
/// The observed preflop prefix as source steps under the (possibly virtual) roles.
/// An exact-sized raise matches the node's resolved size within `0.5 * chip / unit`;
/// an off-menu size is left as its own `PreflopStep::Raise` and handled as a
/// translated branch by Tasks 10 and 13, never rounded into a key here.
pub fn observed_history(prefix:&HandState,roles:&[(Seat,Position)],mapped:bool,unit:u32)
    ->Vec<(Position,PreflopStep)>{
    prefix.actions.iter().filter(|a|a.street==proto::Street::Preflop).map(|a|{
        let role=roles.iter().find(|(s,_)|*s==a.seat).expect("dealt seat").1;
        (virtual_position(role,mapped),to_source_step(&a.action,unit))
    }).collect()
}
/// §8.3 phase-1 straddle format gate, applied before any lookup. The dealt-count and
/// `S >= 2 * bb` rules live in Plan 1's `validate_table`; this reuses them rather than
/// restating them, and adds the one condition `validate_table` does not check: the
/// straddler's starting stack must cover the full post.
pub fn check_straddle(state:&HandState)->Result<(),proto::RulesError>{
    let cfg=&state.config;
    core_model::validate_table(cfg,state.button,&state.dealt)?;
    let Some(s)=&cfg.straddle else{return Ok(())};
    let roles=physical_positions(state);
    let straddler=roles.iter().find(|(_,p)|*p==Position::Utg)
        .map(|(seat,_)|*seat)
        .ok_or(proto::RulesError::FormatUnsupported{detail:"no UTG seat to straddle".into()})?;
    if start_stack(state,straddler)<s.amount_chips{
        return Err(proto::RulesError::FormatUnsupported{
            detail:"straddler's starting stack does not cover the straddle".into()});
    }
    Ok(())
}
```

`to_source_step(&Action, unit) -> PreflopStep` converts a chip action into its source step, dividing a raise-to by `unit` into `to_bb_x1000`. `HandConfig.straddle` is `Option<UtgStraddle>`, so a re-straddle cannot be represented at all: it fails `HandConfig` deserialization, before any query. The check above exists because an externally deserialized config can still carry a short post or the wrong dealt count.

```rust
pub fn query(&self, cfg:&HandConfig, state:&HandState, prefix_len:usize) -> PreflopAnswer {
    let mut answer = PreflopAnswer::empty();
    let prefix = prefix_state(state, prefix_len);
    if prefix.actions.iter().any(|a| a.street != proto::Street::Preflop) {
        answer.unsupported = Some(UnsupportedReason::UnsupportedHistory{
            reason:"preflop query over a postflop prefix".into()});
        return answer;
    }
    if *cfg != state.config {
        answer.unsupported = Some(UnsupportedReason::UnsupportedHistory{
            reason:"config does not match the frozen hand config".into()});
        return answer;
    }
    let mapped = cfg.straddle.is_some();
    if let Err(e) = check_straddle(&prefix) {
        answer.unsupported = Some(UnsupportedReason::FormatUnsupported{detail:e.to_string()});
        return answer;
    }
    let unit = source_unit(cfg);
    let roles = physical_positions(&prefix);
    let Some(actor) = prefix.derived.to_act else {
        answer.unsupported = Some(UnsupportedReason::UnsupportedHistory{reason:"no actor at prefix".into()});
        return answer;
    };
    answer.actor = Some(actor);
    // Eligible = dealt and not folded *at this prefix*, all-in included (§8.3).
    let eligible:Vec<Seat> = prefix.dealt.iter().copied()
        .filter(|s| !prefix.derived.folded[s.0 as usize]).collect();
    let others:Vec<u32> = eligible.iter().copied().filter(|&s| s!=actor)
        .map(|s| start_stack(&prefix, s)).collect();
    let actual_depth = depth_for(start_stack(&prefix, actor), &others, unit);
    let stacks_bb:Vec<f64> = eligible.iter().map(|&s| start_stack(&prefix,s) as f64/unit as f64).collect();
    let observed = observed_history(&prefix, &roles, mapped, unit); // Vec<(Position, PreflopStep)>
    let short = short_handed_prefix(prefix.dealt.len());
    let mut ranked:Vec<&Box<dyn PreflopSource>> = self.bundles.iter().collect();
    ranked.sort_by(|a,b| rank_key(a.bundle_info(),actual_depth,&cfg.rake,unit)
        .partial_cmp(&rank_key(b.bundle_info(),actual_depth,&cfg.rake,unit)).unwrap()
        .then_with(|| a.bundle_info().bundle_id.cmp(&b.bundle_info().bundle_id)));
    let Some(candidate) = ranked.first() else {
        answer.unsupported = Some(UnsupportedReason::MissingPreflopNode{key:"no bundle".into()});
        return answer;
    };
    let info = candidate.bundle_info();
    let Some(used) = bucket(actual_depth, &info.depths) else {
        answer.unsupported = Some(UnsupportedReason::MissingPreflopNode{key:"no acquired depth".into()});
        return answer;
    };
    // The key is built from the CANDIDATE, not from the live config.
    let key = PreflopNodeKey{
        depth_bb: used,
        rake_profile: info.rake_profile.clone(),
        straddle: info.straddle,
        history: short.iter().cloned().chain(observed.iter().cloned()).collect(),
    };
    answer.key = node_key(&key);
    answer.bundle = Some(info.clone());
    answer.unit = unit;
    if actual_depth != used as f64 {
        answer.reasons.push(ApproxReason::DepthBucket{seat:actor,actual_bb:actual_depth as f32,
            used_bb:used,prominent:prominent_depth(actual_depth,used)});
    }
    let (label,prominent) = asymmetric(&stacks_bb, used);
    if label { answer.reasons.push(ApproxReason::AsymmetricStacks{
        stacks_bb:stacks_bb.iter().map(|&s| s as f32).collect(), prominent}); }
    if let Some(reason) = rake_reason(&cfg.rake, info, unit) { answer.reasons.push(reason); }
    if mapped { answer.reasons.push(ApproxReason::StraddleMapped{
        posts:normalized_posts(cfg.sb_chips,cfg.bb_chips,unit)}); }
    if !short.is_empty() { answer.reasons.push(ApproxReason::ShortHandedMapped{dealt:prefix.dealt.len() as u8}); }
    match candidate.lookup(&key) {
        Some(node) => answer.node = Some(node),
        None => answer.unsupported = Some(UnsupportedReason::MissingPreflopNode{key:answer.key.clone()}),
    }
    answer
}
```

`query` builds **one `PreflopNodeKey` per candidate bundle**, never one key from the live config. `depth_bb` comes from `bucket(actual_depth, candidate.depths)`; `rake_profile` and `straddle` come from `candidate.bundle_info()` — a chart bundle's `rake_profile` is `"undocumented"` and its `straddle` is `false` even for a straddled hand, because §8.3's virtual roles are lookup-only. The **live** rake and straddle appear only in the emitted `RakeProfileMapped`/`StraddleMapped` reasons. Building the key from `cfg.rake`/`cfg.straddle` would miss every chart node while still emitting those reasons, which is exactly the failure §8.3 forbids ("never a guessed node"; "Missing nodes stay missing"). `observed_history` maps each observed preflop action through `virtual_position(role, mapped)` and matches exact raise sizes with the `0.5 * chip / unit` tolerance below; `rank_key` returns the lexicographic tuple described next, and `rake_reason` returns `Some(RakeProfileMapped{actual,used})` whenever the candidate profile is not an exact match.

`query` rejects a prefix containing a postflop action, reconstructs the actor via `derive`, and derives eligibility from that prefix. It checks `cfg == state.config`; unit is actual BB unless straddle-mapped, then S. A malformed externally deserialized straddle configuration is rejected even though normal `core-model` entry already rejects it. Verify S≥2BB, six dealt seats and a starting stack covering the full straddle; unsupported formats return `FormatUnsupported`, not a guessed node. The config permits only one UTG straddle, so re-straddle input must fail model deserialization/validation before query.

Ranking is lexicographic: source kind (PokerData first), nearest depth with deeper tie, rake rank, bundle id. For rake rank use `(unknown,cap_distance,rate_distance,collection_mismatch,cap)` for PotRake; `TimeCharge` uses unraked first, then cap. Unknown source rake sorts after every documented profile at the same source/depth and always emits `RakeProfileMapped{actual,used:"undocumented chart rake"}`. Do not search lower-ranked bundles to hide a missing node after a source has been selected. For exact-sized historical raises compare `abs(to/unit - source_to/1000) <= .5/unit`; off-menu histories are handled as separate translated branches in Tasks 10/13, not rounded into a key. Cache the selected mapping by prefix and branch-translated history within a replay invocation.

- [ ] **Step 5: Test full path and hindsight independence.** `pokerdata_action_path_lookup` looks up every Task 2 path and asserts actor, action count and explicit folds; removes the last pair from `range.spot` before node lookup. Query the same early prefix before and after a later large-stack player's fold: selected depth/reasons/key identical. Test 3/4/5-seat mappings, physical posts unchanged under virtual role mapping, missing current nodes, no bundle, and equal rank ids `a`/`b`. Run `cargo test -p core-preflop --test lookup`, then `cargo test --workspace`.

```rust
#[test]
fn pokerdata_action_path_lookup(){
    use core_preflop::{PreflopNodeKey,PreflopStep::*};
    use proto::Position::*;
    let dir=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/preflop/synthetic_v2");
    let source=core_preflop::load_bundle(&dir.join("manifest.json"),&dir.join("nodes.json")).unwrap();
    let mut three=vec![(Utg,Raise{to_bb_x1000:2500}),(Hj,Raise{to_bb_x1000:8750})];
    three.extend([(Co,Fold),(Btn,Fold),(Sb,Fold),(Bb,Fold)]);
    let mut four=three.clone();four.push((Utg,Raise{to_bb_x1000:22000}));
    let squeeze=vec![(Utg,Raise{to_bb_x1000:2500}),(Hj,Call),(Co,Fold),(Btn,Fold),(Sb,Fold)];
    let sb=vec![(Utg,Fold),(Hj,Fold),(Co,Fold),(Btn,Fold)];
    let mut limp=sb.clone();limp.push((Sb,Call));
    for (history,actor) in [(vec![],Utg),(three,Utg),(four,Hj),(squeeze,Bb),(sb,Sb),(limp,Bb)]{
        let key=PreflopNodeKey{depth_bb:100,rake_profile:"5% cap 0.5bb".into(),straddle:false,history};
        let node=source.lookup(&key).unwrap();assert_eq!(node.actor,actor);
        assert_eq!(node.probs.len(),169);assert!(!node.actions.is_empty());
    }
    for history in [vec![(Utg,Call)],
        vec![(Utg,Raise{to_bb_x1000:2500}),(Hj,Raise{to_bb_x1000:8750}),(Co,Call)],
        vec![(Utg,Raise{to_bb_x1000:2500}),(Hj,Raise{to_bb_x1000:8750}),(Co,Fold),(Btn,Fold),(Sb,Call)]]{
        let key=PreflopNodeKey{depth_bb:100,rake_profile:"5% cap 0.5bb".into(),straddle:false,history};
        assert!(source.lookup(&key).is_none());
    }
}
```

The direct missing source lookups above must also be exercised through `PreflopStore::query` on legal model histories, asserting `UnsupportedReason::MissingPreflopNode` rather than merely an absent Option. The synthetic `range.spot` parser maps `60%` through spots.json's resolved 2500 entry; it never treats the percent label as a numeric raise-to.

- [ ] **Step 6: Commit.**

```powershell
git add crates/core-preflop
git commit -m "feat(core-preflop): reconstruct prefix-specific chart lookups" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 9: Normalize all EV reference variants and expand classes

**Files:** Create `crates/core-preflop/src/ev.rs`, `crates/core-preflop/tests/ev.rs`; modify `src/{lib.rs,store.rs,validate.rs,lookup.rs}`.

**Interfaces:**
- Consumes: `PreflopNode`, `BundleInfo`, `PreflopAnswer`, `core_ranges::expand_169(&[f32;169]) -> Range1326`.
- Produces: `normalize_ev(reference: EvReference, value: Option<f32>, committed: f32, source_stack_sb: f32) -> Option<f32>`; `verify_fold(reference: EvReference, fold: Option<f32>, committed: f32, source_stack_sb: f32) -> bool`; `ev_chips(inc_sb: f32, unit: u32) -> f32`; `ExpandedNode { actor: Seat, actions: Vec<Action>, probs: Vec<Vec<f32>>, ev_chips: Vec<Vec<Option<f32>>>, available: Vec<bool>, ev_reference: EvReference, source: SourceKind }`; `ComboClasses::{build() -> Self, class(&self, combo: usize) -> usize}`; `expand_column(&ComboClasses, rows: &[Vec<f32>], a: usize) -> Vec<f32>`; `expand_optional(&ComboClasses, rows: &[Vec<Option<f32>>], a: usize) -> Vec<Option<f32>>`; `expand_mask(&ComboClasses, unreachable: &[bool; 169]) -> Vec<bool>`; `expand_node(node: &PreflopNode, info: &BundleInfo, actor: Seat, unit: u32, actor_max_to: u32) -> ExpandedNode`.

- [ ] **Step 1: Write the exact `pokerdata_units_source_scaling` test.**

```rust
#[test]
fn pokerdata_units_source_scaling() {
    use core_preflop::{EvReference::*,normalize_ev,verify_fold,ev_chips};
    for (sb,bb) in [(1,2),(2,5)] {
        let v=normalize_ev(DecisionIncrementalVerified,Some(1.84),sb as f32,200.).unwrap();
        assert!((v*0.5-0.92).abs()<1e-6);
        assert!((ev_chips(v,bb)/bb as f32-0.92).abs()<1e-6);
    }
    assert!((normalize_ev(NetHandStartVerified,Some(1.84),1.,200.).unwrap()-2.84).abs()<1e-6);
    assert!(verify_fold(NetHandStartVerified,Some(-1.),1.,200.));
    assert!(!verify_fold(NetHandStartVerified,Some(-0.5),1.,200.));
    assert_eq!(normalize_ev(AbsoluteStackVerified,Some(203.),2.,200.),Some(5.));
    assert_eq!(normalize_ev(Unverified,Some(1.84),1.,200.),None);
    assert_eq!(normalize_ev(DecisionIncrementalVerified,None,5.,200.),None);
    assert!((ev_chips(1.84,10)/5.-1.84).abs()<1e-6);
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-preflop --test ev pokerdata_units_source_scaling`.

- [ ] **Step 3: Implement the four binding formulas.** Let `committed` include source posts and previous wagers before this prefix, in source SB; `stack_dec = source_stack_sb - committed`:

```rust
pub fn normalize_ev(r:EvReference, v:Option<f32>, committed:f32, start:f32) -> Option<f32> {
    let v=v?;
    let out=match r {
        EvReference::DecisionIncrementalVerified=>v,
        EvReference::NetHandStartVerified=>v+committed,
        EvReference::AbsoluteStackVerified=>v-(start-committed),
        EvReference::Unverified=>return None,
    };
    out.is_finite().then_some(out)
}
pub fn verify_fold(r:EvReference, fold:Option<f32>, committed:f32, start:f32) -> bool {
    match (r,fold) {
        (EvReference::Unverified,_) | (_,None)=>true,
        (EvReference::DecisionIncrementalVerified,Some(v))=>v.abs()<=1e-3,
        (EvReference::NetHandStartVerified,Some(v))=>(v+committed).abs()<=1e-3,
        (EvReference::AbsoluteStackVerified,Some(v))=>(v-(start-committed)).abs()<=1e-3,
    }
}
pub fn ev_chips(inc_sb:f32,unit:u32)->f32 { inc_sb*0.5*unit as f32 }
```

For `net_hand_start_verified`, cross-check `ev_sb(a)+committed` against `ev_sb(a)-ev_sb(fold)` within 1e-3. For `absolute_stack_verified`, cross-check `ev_sb(a)-stack_dec` against `ev_sb(a)-ev_sb(fold)` within 1e-3. A present conflicting fold reference makes the node unloadable, as §13.1 requires; a missing individual EV remains None/`NoEvReference`. Missing fold EV is not evidence of a contradiction; known prefix commitments still define the declared verified variant. Missing EVs, non-finite results, and unknown references never become zeros. For verified fold with a passing cross-check, store normalized fold as exact `0.0`. Charts suppress every EV including fold and use `ChartNoEv`; unverified EV sources suppress every EV and add `EvReferenceUnverified`. A single-source missing EV uses `NoEvReference`.

- [ ] **Step 4: Expand probability and EV columns without transposing the wrong axis.**

`PreflopNode.probs` and `PreflopNode.ev_source_sb` are class-major `[169][actions]`, so a column must be extracted before expansion; and the 169 indicator expansions are computed **once**, at bundle load, into a `ComboClasses` table that every later expansion borrows.

```rust
/// combo -> class, built once from the 169 indicator expansions (§4.1's 6/4/12 orbits).
pub struct ComboClasses { class_of: [u16; 1326] }
impl ComboClasses {
    pub fn build() -> Self {
        let mut class_of = [u16::MAX; 1326];
        for c in 0..169 {
            let mut indicator = [0.0f32; 169]; indicator[c] = 1.0;
            let mask = core_ranges::expand_169(&indicator);   // a valid Range1326
            for (i, m) in mask.0.iter().enumerate() { if *m != 0.0 { class_of[i] = c as u16; } }
        }
        debug_assert!(class_of.iter().all(|&c| c != u16::MAX));
        Self { class_of }
    }
    pub fn class(&self, combo: usize) -> usize { self.class_of[combo] as usize }
}
/// Column `a` of a class-major `[169][actions]` matrix, expanded to 1326 combos.
pub fn expand_column(classes:&ComboClasses, rows:&[Vec<f32>], a:usize)->Vec<f32> {
    (0..1326).map(|i| rows[classes.class(i)][a]).collect()
}
pub fn expand_optional(classes:&ComboClasses, rows:&[Vec<Option<f32>>], a:usize)->Vec<Option<f32>> {
    (0..1326).map(|i| rows[classes.class(i)][a]).collect()
}
pub fn expand_mask(classes:&ComboClasses, unreachable:&[bool;169])->Vec<bool> {
    (0..1326).map(|i| unreachable[classes.class(i)]).collect()
}
```

```rust
pub fn expand_node(node:&PreflopNode,info:&BundleInfo,actor:Seat,unit:u32,actor_max_to:u32)
    ->ExpandedNode{
    let classes=info.combo_classes();                 // the table built once at bundle load
    let charts=matches!(info.source,SourceKind::ChartTranscription);
    let actions:Vec<Action>=node.actions.iter()
        .map(|s|to_chip_action(s,unit,actor_max_to)).collect();
    let cols:Vec<Vec<f32>>=(0..node.actions.len())
        .map(|a|expand_column(&classes,&node.probs,a)).collect();
    let evs:Vec<Vec<Option<f32>>>=(0..node.actions.len()).map(|a|{
        if charts || info.ev_reference==EvReference::Unverified {return vec![None;1326];}
        let src=node.ev_source_sb.as_ref();
        let col=src.map(|rows|expand_optional(&classes,rows,a)).unwrap_or_else(||vec![None;1326]);
        col.into_iter().map(|v|normalize_ev(info.ev_reference,v,
            node.committed_by_actor_sb,info.source_stack_sb()).map(|x|ev_chips(x,unit))).collect()
    }).collect();
    let unreachable=expand_mask(&classes,&node.unreachable);
    ExpandedNode{
        actor,actions,
        probs:(0..1326).map(|c|cols.iter().map(|col|col[c]).collect()).collect(),
        ev_chips:(0..1326).map(|c|evs.iter().map(|col|col[c]).collect()).collect(),
        available:unreachable.iter().map(|&u|!u).collect(),
        ev_reference:info.ev_reference,source:info.source,
    }
}
```

`to_chip_action` applies the integer half-up conversion below; `BundleInfo::combo_classes()` returns the `ComboClasses` table cached on the adapter at load; `source_stack_sb()` is `2 * depth_bb as f32`. EVs can be negative or above 1, so `expand_optional` never passes the EV payload through a range validator — only `ComboClasses::build` uses `expand_169`, and it feeds it a legal indicator. The result is `[1326][actions]` with the class unreachable mask copied to all 6/4/12 combos of each class, and `available[i] == !unreachable[i]`. No board blocking here. Convert raise-to using integer half-up `(source_milli_bb as u64 * unit as u64 + 500)/1000` with checked u32 conversion; AllIn uses actual actor maximum, not source depth. Legality mapping runs in Task 10 after source expansion.

- [ ] **Step 5: Verify source-unit and loader integration.** Extend the test to a re-raise prefix with committed=5 source SB, no-fold BB vs limp with `stack_dec=198`, and straddle units 4 and 10. Modify synthetic fixtures in memory for each reference; bad net-start fold -.5 makes the containing node unloadable and rejects that inconsistent bundle, while other validated bundles remain active. Assert changing actual SB from 1 to 2 while BB unit=5 never multiplies EV by actual SB. Assert null EV differs from numeric 0, including a zero-frequency action carrying EV=2.31. Run `cargo test -p core-preflop`, `cargo test --workspace`.

- [ ] **Step 6: Commit.**

```powershell
git add crates/core-preflop
git commit -m "feat(core-preflop): normalize verified source EV references" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 10: Implement wager interpolation and legality after mapping

**Files:** Create `crates/core-preflop/src/translate.rs`, `crates/core-preflop/tests/translate.rs`; modify `src/lib.rs`.

**Interfaces:**
- Consumes: `Action`, `LegalAction`, `Derived`, `ExpandedNode`.
- Produces: `Interpolation { choices: Vec<(usize,f64)>, deviation: f64, clamped: bool }`; `interpolate(s:f64, menu:&[(usize,f64)]) -> Option<Interpolation>`; `wager_fraction(to:u32, own:u32, call:u32, pot:u32)->f64`; `Destination { index: usize, created: bool }`; `destination_map(actions:&[Action], legal:&[LegalAction]) -> Result<(Vec<Action>, Vec<Destination>), UnsupportedReason>`; `legalize_row(menu:&[Action], map:&[Destination], probs:&[f32], evs:&[Option<f32>], notes:&mut Vec<String>) -> MappedAdvice`; `MappedAction { action:Action, probability:f32, ev_chips:Option<f32>, unavailable:Option<Unavailable> }`; `MappedAdvice { actions:Vec<MappedAction>, notes:Vec<String>, unsupported:Option<UnsupportedReason> }`. The map is built **once** per node and reused for all 1326 rows: a single per-combo `legalize` could not express the shared map that Step 4 requires, so the two halves are separate functions.

- [ ] **Step 1: Write failing boundary tests.**

```rust
#[test]
fn bet_translation_boundaries() {
    let t=core_preflop::interpolate(0.73,&[(0,0.5),(1,1.)]).unwrap();
    assert!((t.choices[0].1-81.0/173.0).abs()<1e-12);
    assert!((t.deviation-0.23).abs()<1e-12);
    assert!(t.deviation>0.10);
    assert_eq!(core_preflop::interpolate(0.2,&[(0,0.5),(1,1.)]).unwrap().choices,vec![(0,1.)]);
    assert_eq!(core_preflop::interpolate(2.,&[(0,0.5),(1,1.)]).unwrap().choices,vec![(1,1.)]);
    assert_eq!(core_preflop::interpolate(0.73,&[(0,0.5)]).unwrap().choices,vec![(0,1.)]);
    assert_eq!(core_preflop::interpolate(0.5,&[(0,0.5),(1,1.)]).unwrap().choices,vec![(0,1.)]);
    let jam=core_preflop::interpolate(1.5,&[(0,0.5),(1,1.),(2,2.)]).unwrap();
    assert_eq!(jam.choices,vec![(1,0.4),(2,0.6)]);
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-preflop --test translate`.

- [ ] **Step 3: Implement the binding pseudo-harmonic formula.** Sizes are pot fractions at the parent node, not raise-to/BB. `s = (chips added beyond a call)/(pot after the bettor's call)`; for raise-to this is `(to-own-call)/(pot+call)`. Replay uses the financial state reconstructed for each mapped parent, not a later observed pot.

```rust
pub fn wager_fraction(to:u32,own:u32,call:u32,pot:u32)->f64 {
    (to as f64-own as f64-call as f64)/(pot as f64+call as f64)
}
pub fn interpolate(s:f64,menu:&[(usize,f64)])->Option<Interpolation> {
    if !s.is_finite() || s<0.0 || menu.is_empty(){return None;}
    let mut sizes=menu.to_vec();
    sizes.sort_by(|a,b|a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    sizes.dedup_by(|a,b|a.1==b.1);
    if sizes.iter().any(|(_,x)|!x.is_finite() || *x<0.0){return None;}
    if let Some(&(i,_))=sizes.iter().find(|(_,x)|*x==s){
        return Some(Interpolation{choices:vec![(i,1.0)],deviation:0.0,clamped:false});
    }
    let first=sizes[0]; let last=*sizes.last()?;
    if sizes.len()==1 || s<first.1 || s>last.1 {
        let (i,x)=if s<first.1 {first}else{last};
        return Some(Interpolation{choices:vec![(i,1.0)],deviation:(s-x).abs(),clamped:true});
    }
    let pair=sizes.windows(2).find(|p|p[0].1<=s && s<=p[1].1)?;
    let ((i,a),(j,b))=(pair[0],pair[1]);
    let fa=(b-s)*(1.0+a)/((b-a)*(1.0+s));
    Some(Interpolation{choices:vec![(i,fa),(j,1.0-fa)],
        deviation:(s-a).abs().min((s-b).abs()),clamped:false})
}
```

Copy the exact formula into the source doc comment: `f_A = (B-s)(1+A)/((B-A)(1+s))`, `f_B=1-f_A`, `P(obs|combo)=f_A P(A|combo)+f_B P(B|combo)`. Single size/equal size clamps with f=1; below minimum clamps; above largest non-all-in interpolates with an actual all-in menu action if present, otherwise clamps. Non-exact mappings emit `BetTranslation{street,seat,observed_pct:100*s,mapped:[(100*A,f_A),(100*B,f_B)],deviation:d,prominent:d>.10}` and a clamp note when appropriate. Frequencies remain fractions; `observed_pct`/mapped size fields are percent display values, deviation is in pot units as §8.4.

- [ ] **Step 4: Implement legal destination choice and merging.** Convert amounts first. Preserve deterministic order Fold,Check,Call,wagers ascending,AllIn. Build the legal source menu before moving anything; then process every source probability exactly once. A below-min Raise goes to the smallest legal **source menu** raise, else Call; a wager above max goes to legal AllIn; a legal action maps to itself. Match actual `LegalAction` intervals and do not treat an open Bet as a Raise. Reject impossible nonwager mismatches to the caller as `UnsupportedHistory`; never guess Fold.

`destination_map` runs once per node and returns the deterministic output menu plus, for each source action index, where its mass goes and whether that destination exists only because of the move:

```rust
pub struct Destination { pub index: usize, pub created: bool }

pub fn destination_map(actions:&[Action],legal:&[LegalAction])
    ->Result<(Vec<Action>,Vec<Destination>),UnsupportedReason>{
    let mut menu:Vec<Action>=Vec::new();
    let mut created:Vec<bool>=Vec::new();
    let mut push=|menu:&mut Vec<Action>,created:&mut Vec<bool>,a:Action,new:bool|->usize{
        match menu.iter().position(|x|*x==a){
            Some(i)=>{if !new{created[i]=false;} i}
            None=>{menu.push(a);created.push(new);menu.len()-1}
        }
    };
    // Pass 1: every source action that is itself legal owns its destination.
    let mut mapped:Vec<Option<usize>>=vec![None;actions.len()];
    for (k,a) in actions.iter().enumerate(){
        if is_legal(a,legal){ mapped[k]=Some(push(&mut menu,&mut created,a.clone(),false)); }
    }
    // Pass 2: illegal source actions move by kind (§8.4 legality-after-mapping).
    for (k,a) in actions.iter().enumerate(){
        if mapped[k].is_some(){continue;}
        let target=match a{
            Action::Raise{to} if below_min(*to,legal)=>
                smallest_legal_menu_raise(actions,legal).or_else(||call_action(legal)),
            Action::Bet{to}|Action::Raise{to} if above_max(*to,legal)=>allin_action(legal),
            _=>None,
        };
        let Some(t)=target else{
            return Err(UnsupportedReason::UnsupportedHistory{
                reason:format!("no legal destination for {a:?}")});
        };
        // `new` is true only when this move is what puts the destination on the menu.
        let new=!menu.contains(&t) && !actions.iter().any(|x|*x==t && is_legal(x,legal));
        mapped[k]=Some(push(&mut menu,&mut created,t,new));
    }
    order_menu(&mut menu,&mut created,&mut mapped);   // Fold,Check,Call,wagers ascending,AllIn
    Ok((menu,mapped.into_iter().enumerate()
        .map(|(_,i)|{let i=i.expect("every source action is mapped");
            Destination{index:i,created:created[i]}}).collect()))
}
```

`legalize_row` then walks one combo's probability/EV row through that map exactly once:

```rust
pub fn legalize_row(menu:&[Action],map:&[Destination],probs:&[f32],evs:&[Option<f32>],
    notes:&mut Vec<String>)->MappedAdvice{
    let mut out:Vec<MappedAction>=menu.iter().map(|a|MappedAction{
        action:a.clone(),probability:0.0,ev_chips:None,unavailable:None}).collect();
    let mut own_ev:Vec<Option<f32>>=vec![None;menu.len()];
    let mut ev_conflict=vec![false;menu.len()];
    for (k,d) in map.iter().enumerate(){
        merge_probability(&mut out[d.index],&menu[d.index],probs[k],d.created,notes);
        if !d.created {
            // Only a destination that was itself a source action keeps an EV, and only
            // its own: two identically rounded source actions colliding here have no
            // unique payoff, so the EV is omitted with a move note.
            match own_ev[d.index]{
                None=>own_ev[d.index]=evs[k],
                Some(prev) if Some(prev)!=evs[k]=>ev_conflict[d.index]=true,
                _=>{}
            }
        }
    }
    for i in 0..menu.len(){
        if !ev_conflict[i] && out[i].unavailable.is_none() { out[i].ev_chips=own_ev[i]; }
        else if ev_conflict[i] { notes.push(format!("Two rounded sources collide on {:?}; EV omitted",menu[i])); }
    }
    MappedAdvice{actions:out,notes:std::mem::take(notes),unsupported:None}
}

pub fn merge_probability(
    destination:&mut MappedAction, from:&Action, probability:f32, created:bool,
    notes:&mut Vec<String>,
) {
    destination.probability+=probability;
    if created {
        destination.ev_chips=None;
        destination.unavailable=Some(Unavailable::MovedProbability{from:from.clone()});
    }
    notes.push(format!("Moved {:?} probability {} to {:?}",from,probability,destination.action));
}
```

`is_legal`, `below_min`, `above_max`, `smallest_legal_menu_raise`, `call_action`, `allin_action` and `order_menu` are private helpers over `&[LegalAction]` in the same file; `order_menu` sorts the menu into Fold, Check, Call, wagers ascending, AllIn and rewrites `created`/`mapped` indices to match, so the menu order is identical for all 1326 rows.

Use chip-valued `MappedAction` entries until final engine conversion. A destination already present as a source action retains only **its own** normalized EV. Created destinations get None and `MovedProbability{from}`; never average a moved source's payoff into them. Identical rounded actions merge by the same rule: prefer an independently exactly represented destination's EV; if no unique destination payoff exists, omit it with a move note. All 1326 rows go through the one `destination_map` built for that node, so every row's menu is identical. When `destination_map` returns `Err(UnsupportedHistory{..})` the caller sets `MappedAdvice.unsupported` to that reason for every row and reports no actions; no probability disappears into a default action.

- [ ] **Step 5: Verify legal moves and commit.** Cases: Raise(7) below min=10 maps to existing legal Raise(12), preserving Raise(12)'s EV; with no legal source raise maps to Call; Bet(120) at max=100 creates AllIn(100) with None EV; existing source AllIn(100) retains its own EV; two half-up rounded wagers collide; no missing probability becomes 0 or fold. Check total probability conserved. Run `cargo test -p core-preflop --test translate`, `cargo test --workspace`.

```powershell
git add crates/core-preflop
git commit -m "feat(core-preflop): translate wagers and preserve legal probability mass" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 11: Implement shared history-branch Bayesian updates

**Files:** Create `crates/core-preflop/src/branches.rs`, `crates/core-replay/{Cargo.toml,src/lib.rs,src/branches.rs,tests/branches.rs}`; modify `Cargo.toml` (append `crates/core-replay` to members), `Cargo.lock`, and `crates/core-preflop/src/lib.rs` (add `pub mod branches;`).

**Interfaces:**
- Consumes: `proto::{Seat,Action,Range1326}`, interpolation choices from Task 10.
- Produces: exact §9.1 `SeatMass`, `HistoryBranch` in core-preflop, re-exported from core-replay; `initial(seats:&[Seat])->Vec<HistoryBranch>`; `marginal(branches:&[HistoryBranch],seat:Seat)->Vec<f64>`; `posterior(branches:&[HistoryBranch],seat:Seat,combo:usize)->Vec<f64>`; `condition(branch:&HistoryBranch,actor:Seat,p:&[f64],factor:f64)->Option<HistoryBranch>`; `rescale(branches:&mut [HistoryBranch], log_reach:&mut [f64])`; `range_output(&[f64])->Range1326`.

- [ ] **Step 1: Add replay crate and failing T3.** Manifest follows Task 1's package fields; dependencies are `proto`, `core-model`, `core-ranges`, `core-preflop` as sibling paths and existing workspace `serde`, `serde_json`, `thiserror`. Add no rayon. `core-replay/src/branches.rs` uses `pub use core_preflop::branches::*;`, and `core-replay/src/lib.rs` flattens the crate root so the names below resolve as `core_replay::HistoryBranch`, not `core_replay::branches::HistoryBranch`:

```rust
pub mod branches;
pub use branches::*;                 // SeatMass, HistoryBranch, initial, marginal, posterior, condition, rescale, range_output
// added by later tasks in this plan:
// pub mod preflop; pub mod postflop; pub mod snapshot;
// pub use snapshot::*;              // SnapshotKey, SnapshotProvenance, StreetSnapshot,
//                                   // SnapshotStore, select_snapshot, covered_prefix
```

Task 14 uncomments the `snapshot` lines when it creates that module; Task 13 adds `preflop`, Task 15 adds `postflop`. Without these re-exports the tests below (`core_replay::HistoryBranch`, `core_replay::StreetSnapshot`, `core_replay::select_snapshot`) do not compile.

Revision 6's §13.1 T3 now states §8.4's reading, so the assertions below are the spec text: a one-element branch has posterior `[1.0]` for every supported combo, the **actor's** normalized combo distribution is `(0.9, 0.1)`, and the unacted seat stays uniform.

```rust
fn close(a:f64,b:f64){assert!((a-b).abs()<1e-10,"{a} != {b}");}
fn two_combos()->Vec<core_replay::HistoryBranch>{
    let mut b=core_preflop::branches::initial(&[proto::Seat(0),proto::Seat(1)]);
    for s in &mut b[0].seats { s.mass.fill(0.0);s.mass[0]=1.;s.mass[1]=1.; }
    b
}
#[test]
fn replay_bayes_two_combos(){
    use core_preflop::branches::*;
    let mut b=two_combos(); let mut logs=vec![0.;6];
    for (pair,m) in [([0.8,0.2],0.5),([0.25,1.],0.4),([0.9,0.1],0.5)] {
        let mut p=vec![0.;1326]; p[..2].copy_from_slice(&pair);
        let old=b[0].q;
        b=vec![condition(&b[0],proto::Seat(0),&p,1.).unwrap()];
        close(b[0].q/old,m);
        if m==0.5 && pair[0]==0.9 {
            close(b[0].seats[0].mass[0],9.);close(b[0].seats[0].mass[1],1.);
        }
        rescale(&mut b,&mut logs);
    }
    close(b[0].q,0.1); close(logs[0],0.18_f64.ln());
    let r=marginal(&b,proto::Seat(0));close(r[0],1.);close(r[1],1./9.);
    close(r[0]/(r[0]+r[1]),0.9);close(r[1]/(r[0]+r[1]),0.1);
    assert_eq!(posterior(&b,proto::Seat(1),0),vec![1.]);
    let h=marginal(&b,proto::Seat(1));close(h[0],h[1]);
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test branches replay_bayes_two_combos`.

- [ ] **Step 3: Define shared types and the exact conditioning kernel.**

```rust
#[derive(Clone,Debug)]
pub struct SeatMass { pub seat:Seat, pub node:Option<String>, pub mass:Vec<f64> }
#[derive(Clone,Debug)]
pub struct HistoryBranch {
    pub id:u8, pub parent:Option<u8>, pub split_by:Option<Seat>,
    pub translated:Vec<(Seat,Action)>, pub q:f64, pub residual:bool,
    pub stopped:Option<String>, pub seats:Vec<SeatMass>,
}
pub fn initial(seats:&[Seat])->Vec<HistoryBranch>{
    vec![HistoryBranch{id:0,parent:None,split_by:None,translated:vec![],q:1.,
        residual:false,stopped:None,seats:seats.iter().map(|&seat|
        SeatMass{seat,node:None,mass:vec![1.;1326]}).collect()}]
}
pub fn condition(b:&HistoryBranch,actor:Seat,p:&[f64],factor:f64)->Option<HistoryBranch>{
    assert_eq!(p.len(),1326);
    assert!(p.iter().all(|p|p.is_finite() && (0.0..=1.0).contains(p)));
    assert!((0.0..=1.0).contains(&factor));
    if b.residual || b.stopped.is_some(){return Some(b.clone());}
    let index=b.seats.iter().position(|s|s.seat==actor)?;
    let w=&b.seats[index].mass;
    let total:f64=w.iter().sum();
    if total==0. {return None;}
    let m=w.iter().zip(p).map(|(w,p)|w*p).sum::<f64>()/total;
    if m==0. || factor==0. {return None;}
    let mut child=b.clone();child.q*=factor*m;
    for (w,p) in child.seats[index].mass.iter_mut().zip(p){*w*=p/m;}
    Some(child)
}
pub fn marginal(bs:&[HistoryBranch],seat:Seat)->Vec<f64>{
    let mut out=vec![0.;1326];
    for b in bs {if let Some(s)=b.seats.iter().find(|s|s.seat==seat){
        for (r,w) in out.iter_mut().zip(&s.mass){*r+=b.q*w;}
    }}out
}
pub fn posterior(bs:&[HistoryBranch],seat:Seat,c:usize)->Vec<f64>{
    let r=marginal(bs,seat)[c];
    bs.iter().map(|b|if r==0. {0.}else{
        b.q*b.seats.iter().find(|s|s.seat==seat).unwrap().mass[c]/r
    }).collect()
}
```

Binding invariant and formulas (keep in source documentation): `r_S[c] = sum_k q_k*w_{S,k}[c]`; `pi_{S,k}[c] = q_k*w_{S,k}[c]/r_S[c]`; `M_k = sum_c w_{V,k}[c]P_k(a|c)/sum_c w_{V,k}[c]`; `w_{V,k}[c] *= P_k(a|c)/M_k`; `q_k *= M_k`. Other seats' masses do not change. An off-menu child X uses `q_k*f_X*M_{kX}` and the same division by `M_{kX}`, copies every other seat, and advances the shared translated history along X. Thus `sum_X q_{kX}w_{V,kX}[c] = q_kw_{V,k}[c](f_AP_A[c]+f_BP_B[c])`. Omitting `/M_k` conditions twice and must fail tests.

- [ ] **Step 4: Implement seat-common rescaling and positive-output preservation.**

```rust
pub fn rescale(bs:&mut [HistoryBranch],logs:&mut [f64]){
    let seats:Vec<Seat>=bs.first().map(|b|b.seats.iter().map(|s|s.seat).collect()).unwrap_or_default();
    for seat in seats {
        let m=marginal(bs,seat).into_iter().fold(0.0_f64,f64::max);
        if m==0. {continue;}
        for b in bs.iter_mut(){for s in &mut b.seats{if s.seat==seat{
            for w in &mut s.mass{*w/=m;}
        }}}
        logs[seat.0 as usize]+=m.ln();
    }
}
pub fn range_output(r:&[f64])->Range1326{
    assert_eq!(r.len(),1326);
    Range1326(std::array::from_fn(|i|{
        if r[i]==0. {0.}else{r[i].max(f32::MIN_POSITIVE as f64) as f32}
    }))
}
```

One `m_S=max_c r_S[c]` is removed from every branch of seat S, including residual and stopped ones; `ln(m_S)` is added to log reach, not its negation. Never alter q during rescale. Transactional updates are built into a candidate list first: if all applying branches have zero integrated support, reject the update, retain pre-action branches and add `UnconditionedPriorStreet{cause:"zero support after <action>"}`. Residual/stopped mass must not make an otherwise impossible action look supported. No threshold at 1e-30 or any other positive magnitude.

- [ ] **Step 5: Add exact pseudo-harmonic and cross-actor tests.** `replay_off_tree_pseudo_harmonic`: fA=81/173, fB=92/173, d=.23; P_A=(.9,.3), P_B=(.1,.5); M=.6/.3; q=.28092485549/.15953757225; conditional masses=(1.5,.5)/(1/3,5/3); unscaled marginal=(.47456647399,.40635838150); pi_A≈(.888,.346). Later P'_A=(.5,1), P'_B=(.2,.8) gives M'=.625/.7, q≈.1756/.1117, unscaled marginal≈(.2213,.3532), output≈(.6267,1), log=ln(.3532), pi_A≈(.952,.398). Assert rounded spec figures with tolerance 5e-4 and exact hand-computed formulas with 1e-10.

```rust
#[test]
fn replay_off_tree_pseudo_harmonic(){
    use core_preflop::branches::*;
    let v=proto::Seat(0);let b=two_combos();
    let f=81.0/173.0;let g=1.0-f;
    let likelihood=|a:f64,b:f64|{let mut p=vec![0.0;1326];p[0]=a;p[1]=b;p};
    let mut bs=vec![condition(&b[0],v,&likelihood(0.9,0.3),f).unwrap(),
                    condition(&b[0],v,&likelihood(0.1,0.5),g).unwrap()];
    close(bs[0].q,f*0.6);close(bs[1].q,g*0.3);
    close(bs[0].seats[0].mass[0],1.5);close(bs[0].seats[0].mass[1],0.5);
    close(bs[1].seats[0].mass[0],1.0/3.0);close(bs[1].seats[0].mass[1],5.0/3.0);
    let first=marginal(&bs,v);close(first[0],f*0.9+g*0.1);close(first[1],f*0.3+g*0.5);
    let mut logs=vec![0.0;6];rescale(&mut bs,&mut logs);
    let qa=bs[0].q;let qb=bs[1].q;
    bs[0]=condition(&bs[0],v,&likelihood(0.5,1.0),1.0).unwrap();
    bs[1]=condition(&bs[1],v,&likelihood(0.2,0.8),1.0).unwrap();
    close(bs[0].q/qa,0.625);close(bs[1].q/qb,0.7);
    let unscaled=[f*0.9*0.5+g*0.1*0.2,f*0.3+g*0.5*0.8];
    rescale(&mut bs,&mut logs);
    let r=marginal(&bs,v);close(r[0],unscaled[0]/unscaled[1]);close(r[1],1.0);
    close(logs[0],unscaled[1].ln());
    close(posterior(&bs,v,0)[0],f*0.9*0.5/unscaled[0]);
    close(posterior(&bs,v,1)[0],f*0.3/unscaled[1]);
}
```

```rust
#[test]
fn replay_cross_actor_branches(){
    use core_preflop::branches::*;
    let b=two_combos();let v=proto::Seat(0);let h=proto::Seat(1);
    let mut pa=vec![0.;1326];pa[..2].copy_from_slice(&[0.8,0.1]);
    let mut pb=vec![0.;1326];pb[..2].copy_from_slice(&[0.1,0.4]);
    let mut children=vec![condition(&b[0],v,&pa,0.6).unwrap(),condition(&b[0],v,&pb,0.4).unwrap()];
    close(children[0].q,0.27);close(children[1].q,0.10);
    close(marginal(&children,v)[0],0.52);close(marginal(&children,v)[1],0.22);
    close(posterior(&children,h,0)[0],0.27/0.37);
    children[0]=condition(&children[0],v,&vec![0.9;1326],1.).unwrap();
    children[1]=condition(&children[1],v,&vec![0.1;1326],1.).unwrap();
    close(children[0].q,0.243);close(children[1].q,0.010);
    close(marginal(&children,v)[0],0.436);close(marginal(&children,v)[1],0.070);
    close(posterior(&children,h,0)[0],0.243/0.253);
    assert_eq!(children[0].seats[1].mass,b[0].seats[1].mass);
}
```

Repeat T6 with rescale after each action: villain logs add ln(.52), hero logs add ln(.37); outputs villain=(1,.4231), hero=(1,1); villain combo posteriors=(.923,.077)/(.273,.727); later hero=.9605/.0395. Keep translated child histories as distinct Raise A/B and verify hero's next key follows each. A zero-M child disappears for every seat. Assert 1e-30 remains positive and a 1e-50 output clamps to `f32::MIN_POSITIVE`, zero remains zero.

- [ ] **Step 6: Verify and commit.** `cargo test -p core-replay --test branches`, `cargo test --workspace`.

```powershell
git add Cargo.toml Cargo.lock crates/core-preflop crates/core-replay
git commit -m "feat(core-replay): condition shared history branches once per action" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 12: Cap live branches with a persistent frozen residual

**Files:** Modify `crates/core-preflop/src/branches.rs`, `crates/core-replay/tests/branches.rs`.

**Interfaces:**
- Consumes: Task 11 history branches and shared q/masses.
- Produces: `cap_branches(&mut Vec<HistoryBranch>)`; `residual_reason(&[HistoryBranch], Seat)->Option<ApproxReason>`; `split_action(&[HistoryBranch], Seat, &[(Action,f64,Vec<f64>)])->Vec<HistoryBranch>` for a common menu in tests; production replay builds per-branch choices before a single global cap.

- [ ] **Step 1: Write `replay_branch_cap_residual` red.** Use uniform likelihood .5 at all combos. Split **all current live branches for one observed wager before capping**, preserving parent creation order, child A before B. Three wagers f=.6/.4 produce all eight weights `.027,.018,.018,.018,.012,.012,.012,.008`, total .125. Assert four live + one residual, live total .081, residual .044 and share 35.2%. Later on-menu .5 halves only live total to .0405; residual remains .044, share `100*.044/.0845 = 52.0710059` (52.1 rounded). Fourth split keeps `.00405` and earliest three `.0027` children; residual extends by `.0081` to `.0521`.

```rust
#[test]
fn replay_branch_cap_residual(){
    use core_preflop::branches::*;
    let mut b=two_combos();let actor=proto::Seat(0);
    let choices=vec![(proto::Action::Raise{to:50},0.6,vec![0.5;1326]),
                     (proto::Action::Raise{to:100},0.4,vec![0.5;1326])];
    for _ in 0..3 {b=split_action(&b,actor,&choices);cap_branches(&mut b);}
    assert_eq!(b.iter().filter(|b|!b.residual).count(),4);
    close(b.iter().find(|b|b.residual).unwrap().q,0.044);
    close(b.iter().map(|b|b.q).sum(),0.125);
    b=b.iter().filter_map(|b|condition(b,actor,&vec![0.5;1326],1.)).collect();
    close(b.iter().filter(|b|!b.residual).map(|b|b.q).sum(),0.0405);
    close(b.iter().find(|b|b.residual).unwrap().q,0.044);
    b=split_action(&b,actor,&choices);cap_branches(&mut b);
    close(b.iter().find(|b|b.residual).unwrap().q,0.0521);
    assert_eq!(b.iter().filter(|b|b.residual).count(),1);
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test branches replay_branch_cap_residual`.

- [ ] **Step 3: Implement q-weighted residual merging.** `q_R' = q_R + sum_k q_k`; `w_{S,R}'[c] = (q_R*w_{S,R}[c] + sum_k q_k*w_{S,k}[c]) / q_R'`. This preserves each seat's marginal; do not discard or normalize away residual support.

```rust
pub fn cap_branches(bs:&mut Vec<HistoryBranch>){
    let mut live=Vec::new();let mut residual=None;
    for b in bs.drain(..){if b.residual{residual=Some(b)}else{live.push(b)}}
    live.sort_by(|a,b|b.q.total_cmp(&a.q).then(a.id.cmp(&b.id)));
    let overflow=if live.len()>4 {live.split_off(4)}else{vec![]};
    for b in overflow {
        if let Some(r)=&mut residual {
            let total=r.q+b.q;
            for (rs,ss) in r.seats.iter_mut().zip(&b.seats){
                assert_eq!(rs.seat,ss.seat);
                for (rw,w) in rs.mass.iter_mut().zip(&ss.mass){*rw=(r.q * *rw+b.q*w)/total;}
            }r.q=total;
        }else{
            let mut r=b;r.residual=true;r.translated.clear();r.stopped=None;
            for s in &mut r.seats{s.node=None;}residual=Some(r);
        }
    }
    live.sort_by_key(|b|b.id);bs.extend(live);
    if let Some(r)=residual{bs.push(r);}
}
pub fn residual_reason(bs:&[HistoryBranch],hero:Seat)->Option<ApproxReason>{
    let r=bs.iter().find(|b|b.residual)?;
    Some(ApproxReason::BranchResidual{seat:hero,
        residual_mass_pct:(100.*r.q/bs.iter().map(|b|b.q).sum::<f64>()) as f32,
        cause:"cap".into()})
}
```

Stopped non-residual branches remain in the four non-residual slots; they are frozen and can be merged on a later overflow, preserving their missing-node reason. Do not create more than one residual. Recompute the displayed share after later evidence; storing only the original 35.2% would be wrong. Common seat rescale still applies to frozen masses; their shape and q are not conditioned.

- [ ] **Step 4: Implement deterministic child creation and audit.** Child ids are monotonically assigned within an observed-action batch; use a wider private creation counter and, if it exceeds u8, stably compact all retained ids and parent references before assigning new ids. The public `id:u8` field remains unchanged; compaction preserves relative creation order and tie-breaking. Map every seat's next node from the **same** translated history. Audit before/after cap `marginal` equality for every seat/combo with 1e-12 tolerance, nonuniform residual masses too. Check no later on-menu or split modifies residual q or applies a guessed likelihood to it.

```rust
/// Splits every live branch across one observed wager's mapped menu choices,
/// preserving parent creation order and, within a parent, choice order (child A
/// before child B). Residual and stopped branches are copied unchanged. The caller
/// runs `cap_branches` once, after every parent has been expanded — never per parent.
pub fn split_action(bs:&[HistoryBranch],actor:Seat,choices:&[(Action,f64,Vec<f64>)])
    ->Vec<HistoryBranch>{
    let mut next_id:u32=bs.iter().map(|b|b.id as u32+1).max().unwrap_or(0);
    let mut out:Vec<HistoryBranch>=Vec::new();
    for b in bs {
        if b.residual || b.stopped.is_some(){ out.push(b.clone()); continue; }
        for (action,f,p) in choices {
            // `condition` applies q *= f * M and w *= P/M; a zero-M child is not created.
            let Some(mut child)=condition(b,actor,p,*f) else{continue};
            child.parent=Some(b.id);
            child.split_by=Some(actor);
            child.translated.push((actor,action.clone()));
            child.id=u8::try_from(next_id).unwrap_or(u8::MAX);   // compacted below if saturated
            next_id+=1;
            out.push(child);
        }
    }
    if next_id>u8::MAX as u32 { compact_ids(&mut out); }
    out
}
/// Rewrites ids to `0..out.len()` in creation order, remapping every `parent`
/// reference, so relative creation order and every tie-break above are preserved.
fn compact_ids(out:&mut [HistoryBranch]){
    let old:Vec<u8>=out.iter().map(|b|b.id).collect();
    for (i,b) in out.iter_mut().enumerate(){
        b.parent=b.parent.and_then(|p|old.iter().position(|&o|o==p).map(|k|k as u8));
        b.id=i as u8;
    }
}
```

A parent whose every child has `M = 0` disappears entirely; all parents having no surviving child is the zero-support rejection of Task 11 Step 4, handled by the caller, which keeps the pre-action branch list.

- [ ] **Step 5: Verify and commit.** `cargo test -p core-replay --test branches`; `cargo test --workspace`.

```powershell
git add crates/core-preflop/src/branches.rs crates/core-replay/tests/branches.rs
git commit -m "feat(core-replay): preserve capped branches in a frozen residual" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 13: Replay preflop prefixes with frozen missing-node stops

**Files:** Create `crates/core-replay/src/{preflop.rs,snapshot.rs}`, `crates/core-replay/tests/replay.rs`; modify `crates/core-replay/src/{lib.rs,branches.rs}` and `crates/core-preflop/src/lookup.rs`.

**Interfaces:**
- Consumes: `PreflopStore::query`, `prefix_state`, `ExpandedNode`, branch kernel, `core_model::derive` and `apply_action`.
- Produces: exact `ReplayInput`, `ReplayOutput`, `replay(input: ReplayInput) -> ReplayOutput`; `walk_preflop(input: &ReplayInput, output: &mut ReplayOutput)`; `apply_preflop_action(input: &ReplayInput, output: &mut ReplayOutput, prefix_len: usize, seat: Seat, observed: &Action) -> bool`; `query_translated(store: &PreflopStore, cfg: &HandConfig, state: &HandState, prefix_len: usize, branch: &HistoryBranch) -> PreflopAnswer`; `stop_branch(b: &mut HistoryBranch, cause: String)`; `missing_reason(seat: Seat, key: &str) -> ApproxReason`; `zero_reason(street: Street, seat: Seat, action: &Action) -> ApproxReason`; `board_mask(board: &[Card]) -> Range1326`; `block_and_rescale(output: &mut ReplayOutput, board: &[Card])`; `publish(output: &mut ReplayOutput, state: &HandState)`; exact `SnapshotKey`, `SnapshotProvenance`, `StreetSnapshot` records for input typing (selection is Task 14).

- [ ] **Step 1: Add red tests for uniform start and missing-node freeze.** `replay_missing_continuation` initially exercises its preflop half here: a source answers the first action and a later key, but not the middle one. Assert the entire history branch retains its pre-missing-action q and all seats' masses through the remaining preflop actions, every seat.node=None, and the reason names the missing prefix. A branch with the middle node present continues independently. Build source maps from Task 2 synthetic fixture in memory; remove a selected key to create the missing case, not an invented source response.

```rust
#[test]
fn preflop_stop_is_shared_and_frozen(){
    use core_preflop::branches::*;
    let mut b=initial(&[proto::Seat(0),proto::Seat(1),proto::Seat(2)]);
    b[0].q=0.27;
    b[0].stopped=Some("missing node UTG_2500_HJ_8750".into());
    for s in &mut b[0].seats{s.node=None;}
    let frozen=b[0].clone();
    for actor in [proto::Seat(0),proto::Seat(1),proto::Seat(2)]{
        let next=condition(&b[0],actor,&vec![0.2;1326],1.).unwrap();
        assert_eq!(next.q,frozen.q);
        for (a,z) in next.seats.iter().zip(&frozen.seats){assert_eq!(a.mass,z.mass);}
    }
}
```

The integration test calls public `replay`, checks a fresh hand yields one q=1 branch and all dealt ranges uniform, and compares hands differing only in hero cards: public ranges, branches' q/masses and log reach identical. Use Plan 1's actual `BeginHand` initializer and `apply_action` to construct legal states; its §3.5 function names are the contract. Do not hand-edit `Derived` in a fixture.

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test replay`; expect missing public replay types/function.

- [ ] **Step 3: Define the exact replay surface.**

```rust
pub struct ReplayInput<'a> {
    pub cfg:&'a HandConfig, pub state:&'a HandState,
    pub store:&'a PreflopStore, pub snapshots:&'a [StreetSnapshot],
}
pub struct ReplayOutput {
    pub ranges:Vec<Option<Range1326>>, pub branches:Vec<HistoryBranch>,
    pub folded_ranges:Vec<Range1326>, pub log_reach:Vec<f64>,
    pub reasons:Vec<ApproxReason>, pub unsupported:Option<UnsupportedReason>,
}
pub struct SnapshotKey {
    pub hand_id:u64, pub config_revision:u32, pub model_revision:u32,
    pub street:Street, pub root_board:Vec<Card>,
    pub root_range_hashes:[[u8;32];2], pub tree_signature:String,
}
pub struct SnapshotProvenance {
    pub identity_at_solve:DecisionIdentity,
    pub solved_prefix:Vec<(Seat,Action)>, pub origin:String,
}
pub struct StreetSnapshot {
    pub key:SnapshotKey, pub provenance:SnapshotProvenance, pub tree:EffectiveTree,
    pub nodes:Vec<proto::worker::NodeStrategy>, pub covered_paths:Vec<OrdinalPath>,
    pub exploitability_chips:f32, pub reasons:Vec<ApproxReason>,
}
```

Derive Clone/Debug and serde for snapshot structures using Plan 1's proto serde support. `ranges` and `log_reach` are seat-id indexed, length six; absent seats are None/log 0. `folded_ranges` contains actual folded dealt seats in increasing seat id; their `SeatMass` entries remain in branches for provenance. Define a private `ReplayState` only if needed for prefix cache/path counters; it must not replace the public types.

- [ ] **Step 4: Implement translated-prefix query without rewriting actual money.** `query_translated` selects mappings from the **observed prefix** for starting stacks/eligible seats/rake/roles, and constructs the node history from the branch's mapped actions. It replays mapped source contributions in source units to compute each parent pot/facing amount. Do not call actual `apply_action` on an illegal rounded source raise and change the observed state; source navigation and actual model legality are separate. Cache keys contain observed prefix index plus translated history, not final-hand folded flags.

For every observed preflop action, perform this transaction:

```rust
pub fn stop_branch(b:&mut HistoryBranch,cause:String){
    b.stopped=Some(cause);
    for s in &mut b.seats{s.node=None;}
}
pub fn missing_reason(seat:Seat,key:&str)->ApproxReason{
    ApproxReason::UnconditionedPriorStreet{street:Street::Preflop,seat,
        cause:format!("missing node {key}")}
}
pub fn zero_reason(street:Street,seat:Seat,action:&Action)->ApproxReason{
    ApproxReason::UnconditionedPriorStreet{street,seat,
        cause:format!("zero support after {action:?}")}
}
```

```rust
/// One observed preflop action, applied to every live branch as one transaction.
/// Returns false when no applying branch had support, in which case nothing changed.
pub fn apply_preflop_action(input:&ReplayInput, output:&mut ReplayOutput,
    prefix_len:usize, seat:Seat, observed:&Action)->bool{
    let before=output.branches.clone();
    let mut next:Vec<HistoryBranch>=Vec::new();
    let mut any_support=false;
    for b in &output.branches{
        if b.residual || b.stopped.is_some(){ next.push(b.clone()); continue; }
        let answer=query_translated(input.store,input.cfg,input.state,prefix_len,b);
        output.reasons.extend(answer.reasons.iter().cloned());
        let Some(expanded)=answer.expanded.as_ref() else{
            let mut stopped=b.clone();
            stop_branch(&mut stopped,format!("missing node {}",answer.key));
            output.reasons.push(missing_reason(seat,&answer.key));
            next.push(stopped); continue;
        };
        match menu_index(expanded,observed){
            Some(a)=>{                                   // on-menu: one `condition`
                let p:Vec<f64>=expanded.probs.iter().map(|row|row[a] as f64).collect();
                if let Some(mut child)=condition(b,seat,&p,1.0){
                    child.translated.push((seat,expanded.actions[a].clone()));
                    any_support=true; next.push(child);
                }
            }
            None=>{                                      // off-menu: Task 10 interpolation
                let (own,call,pot)=source_parent_money(input,prefix_len,b,seat);
                let to=raise_to(observed).expect("a non-wager is always on the menu");
                let s=wager_fraction(to,own,call,pot);
                let Some(t)=interpolate(s,&menu_fractions(expanded,own,call,pot)) else{
                    let mut stopped=b.clone();
                    stop_branch(&mut stopped,format!("unmappable size at {}",answer.key));
                    output.reasons.push(missing_reason(seat,&answer.key));
                    next.push(stopped); continue;
                };
                output.reasons.push(translation_reason(Street::Preflop,seat,s,&t));
                let choices:Vec<(Action,f64,Vec<f64>)>=t.choices.iter().map(|&(a,f)|
                    (expanded.actions[a].clone(),f,
                     expanded.probs.iter().map(|row|row[a] as f64).collect())).collect();
                let children=split_action(std::slice::from_ref(b),seat,&choices);
                any_support|=children.iter().any(|c|!std::ptr::eq(c,b));
                next.extend(children);
            }
        }
    }
    if !any_support{
        output.branches=before;                          // §9.2 zero support: reject the update
        output.reasons.push(zero_reason(Street::Preflop,seat,observed));
        return false;
    }
    output.branches=next;
    cap_branches(&mut output.branches);                  // one global cap per observed action
    rescale(&mut output.branches,&mut output.log_reach);
    true
}

pub fn walk_preflop(input:&ReplayInput,output:&mut ReplayOutput){
    for (i,taken) in input.state.actions.iter().enumerate(){
        if taken.street!=Street::Preflop{break;}
        apply_preflop_action(input,output,i,taken.seat,&taken.action);
    }
}

pub fn replay(input:ReplayInput)->ReplayOutput{
    let mut output=ReplayOutput{
        ranges:vec![None;6],
        branches:initial(&input.state.dealt),
        folded_ranges:vec![], log_reach:vec![0.0;6],
        reasons:vec![], unsupported:None,
    };
    walk_preflop(&input,&mut output);
    // Completed streets only, in order; the current street's own actions are inserted
    // exactly by Plan 2's street-root solve and are never replayed here.
    for street in completed_streets(input.state){
        block_and_rescale(&mut output,&root_board(input.state,street));
        walk_postflop(&input,street,&mut output);        // Task 15; Task 13 ships the fallback
    }
    block_and_rescale(&mut output,&current_root_board(input.state));
    publish(&mut output,input.state);
    output
}
```

Loop over `state.actions` until the first non-preflop action. For each live branch query its own prefix; missing node calls `stop_branch` and records the reason before any likelihood. For an on-menu action take that action's expanded probability column and call `condition` once. For an off-menu raise use Task 10 source-parent pot fractions, call `condition` for each fX, and append the mapped action to each child. For exact action advance the shared history once. Update every seat's next-node position from that common history. An unreachable class with already zero mass is left zero; a positive class with no data gets no guessed likelihood. Candidates for all branches are collected before zero-support rejection and cap (the `before`/`next` pair above); an all-zero applying update keeps the prior branch values. If the update is rejected, do not advance a translated betting branch using an unobserved menu action. Record the explicit reason; a later known prefix is permitted only when its path can still be derived without a guessed action.

```rust
/// `PreflopStore::query` with one substitution: depth, eligibility, rake, roles and the
/// short-handed folds still come from the OBSERVED prefix (§8.3 is hindsight-free and
/// money is never rewritten), but the node history comes from the branch's mapped
/// actions, so a branch that translated villain's raise to menu size A looks up hero's
/// next node under A. The mapping is cached per `(prefix_len, branch.translated)`.
pub fn query_translated(store:&PreflopStore,cfg:&HandConfig,state:&HandState,
    prefix_len:usize,branch:&HistoryBranch)->PreflopAnswer{
    let mut answer=store.query(cfg,state,prefix_len);
    let Some(info)=answer.bundle.clone() else{return answer};
    let prefix=prefix_state(state,prefix_len);
    let roles=physical_positions(&prefix);
    let mapped=cfg.straddle.is_some();
    let key=PreflopNodeKey{
        depth_bb:answer_depth(&answer),
        rake_profile:info.rake_profile.clone(),
        straddle:info.straddle,
        history:short_handed_prefix(prefix.dealt.len()).into_iter()
            .chain(branch.translated.iter().map(|(seat,action)|{
                let role=roles.iter().find(|(s,_)|s==seat).expect("dealt seat").1;
                (virtual_position(role,mapped),to_source_step(action,answer.unit))
            })).collect(),
    };
    answer.key=node_key(&key);
    answer.node=store.bundle_of(&info.bundle_id).and_then(|b|b.lookup(&key));
    answer.unsupported=answer.node.is_none()
        .then(||UnsupportedReason::MissingPreflopNode{key:answer.key.clone()});
    answer.expanded=answer.node.as_ref().map(|n|
        expand_node(n,&info,answer.actor.expect("actor"),answer.unit,
            max_raise_to(&prefix,answer.actor.expect("actor"))));
    answer
}
```

`answer_depth` reads the bucketed depth back off `answer.key`, `to_source_step` converts a chip `Action` into the source `PreflopStep` (raise-to in `to_bb_x1000`), `bundle_of` looks a bundle up by id on the store, and `max_raise_to` is the actor's actual maximum at that prefix. `source_parent_money` replays that branch's mapped source contributions in source units to give `(own, call, pot)` at the parent; `menu_index`, `menu_fractions`, `raise_to`, `translation_reason` and `completed_streets`/`current_root_board` are private helpers in the same module. Never call `core_model::apply_action` with an illegal rounded source raise: source navigation and actual model legality are separate. Cache keys contain the observed prefix index plus the translated history, not final-hand folded flags.

- [ ] **Step 5: Add board blocking at the output marginal and boundary conversion.** Revision 6 (S13) is explicit: *"Public blocking by the board is applied to each seat's output marginal at the street root, never to per-branch masses, so section 8.4's equal-total invariant and the seat-independent residual share are preserved."* Per-branch masses are therefore never zeroed by the board. At each street root, build the blocker mask through the Plan 1 API, apply it to the seat's `f64` marginal, take that blocked maximum as the seat's common rescale factor, and divide **every branch's** masses for that seat by that one scalar — a factor common to a seat's branches changes no posterior and no branch weight, so the equal-total invariant survives. Nothing is quantized to `f32` before the output boundary.

```rust
pub fn board_mask(board:&[Card])->Range1326{
    let mut mask=Range1326([1.;1326]);core_ranges::block_public(&mut mask,board);mask
}
/// §9.2 as amended by revision 6 (S13). Blocks each seat's marginal, rescales that
/// seat's masses in every branch (residual and stopped included) by the one blocked
/// maximum, and accumulates `ln(m_S)` into `log_reach[seat]`.
pub fn block_and_rescale(output:&mut ReplayOutput,board:&[Card]){
    let mask=board_mask(board);
    let seats:Vec<Seat>=output.branches.first()
        .map(|b|b.seats.iter().map(|s|s.seat).collect()).unwrap_or_default();
    for seat in seats{
        let r=marginal(&output.branches,seat);
        let m=r.iter().zip(mask.0).map(|(w,keep)|if keep==0.0{0.0}else{*w})
            .fold(0.0_f64,f64::max);
        if m==0.0{ output.unsupported=Some(UnsupportedReason::InvalidRanges); continue; }
        for b in output.branches.iter_mut(){
            for s in b.seats.iter_mut().filter(|s|s.seat==seat){
                for w in &mut s.mass{*w/=m;}
            }
        }
        output.log_reach[seat.0 as usize]+=m.ln();
    }
}
/// The published range is the seat's marginal with the board removed pointwise.
pub fn publish(output:&mut ReplayOutput,state:&HandState){
    let mask=board_mask(&state.board);
    output.ranges=(0..6).map(|i|{
        let seat=Seat(i);
        state.dealt.contains(&seat).then(||{
            let r=marginal(&output.branches,seat);
            range_output(&r.iter().zip(mask.0).map(|(w,keep)|if keep==0.0{0.0}else{*w})
                .collect::<Vec<f64>>())
        })
    }).collect();
    output.folded_ranges=(0..6).filter(|&i|state.derived.folded[i])
        .filter_map(|i|output.ranges[i].clone()).collect();
}
```

`replay` initializes, walks preflop, then completed streets (Task 15), blocks and rescales at the current street root, and publishes. For this task completed streets have no consumed snapshots yet: emit `UnconditionedPriorStreet` per actual acting seat/street with `cause:"no compatible snapshot"`, retain preflop masses, block each root in order. This is a complete tested fallback, not a temporary fake strategy. Task 15 enriches it when snapshots exist. A zero blocked marginal for any seat returns `InvalidRanges`; cross-seat disjoint-support validation remains Plan 1 `core-eval`/Plan 2 admission. Test explicitly that after `block_and_rescale`, `sum_c w_{S,k}[c]` is still equal across every branch of a seat — that is the invariant the old per-branch blocking broke and that S13 restores.

- [ ] **Step 6: Verify replay outputs and commit.** `cargo test -p core-replay --test replay`; assert folding conditions that seat before retaining folded_ranges, hero actions condition public hero normally, zero support preserves every q/mass, no source reach is multiplied twice, and preflop stopping clears only on entering a postflop street (residual never clears). `cargo test --workspace`.

```powershell
git add crates/core-replay crates/core-preflop/src/lookup.rs
git commit -m "feat(core-replay): replay preflop with frozen missing-node branches" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 14: Select compatible snapshots and preserve prefix-valid provenance

**Files:** Modify `crates/core-replay/src/{snapshot.rs,lib.rs}`; create `crates/core-replay/tests/snapshots.rs`; **delete** `SolvedStreet` and `SnapshotStore` from `crates/engine/src/snapshots.rs` and replace that module with re-exports; modify `crates/engine/src/core.rs` (retype `snapshots`), `crates/engine/src/serve.rs` (build a `StreetSnapshot` at the register site, read through `for_identity`), `crates/engine/src/engine.rs` (three `invalidate_hand` call sites), `crates/engine/Cargo.toml` (add `core-replay`), `Cargo.lock`.

**Interfaces:**
- Consumes: Task 13 snapshot types; `hash_scaled`; `proto::resolve_chip_path(&[MaterializedNode], &[Action]) -> Option<OrdinalPath>`.
- Produces: `SnapshotStore::new() -> Self`, `register(&mut self, active: &DecisionIdentity, snapshot: StreetSnapshot) -> bool`, `invalidate(&mut self, state: &HandState)`, `invalidate_hand(&mut self, hand_id: u64)`, `for_identity(&self, identity: &DecisionIdentity) -> Vec<StreetSnapshot>`, `for_hand(&self, hand_id: u64) -> Vec<&StreetSnapshot>`; `CompatKey { hand_id: u64, config_revision: u32, model_revision: u32, street: Street, root_board: Vec<Card>, root_range_hashes: [[u8; 32]; 2] }` with `CompatKey::of(&SnapshotKey) -> CompatKey`; `compatible(a: &SnapshotKey, b: &CompatKey) -> bool`; `select_snapshot<'a>(snapshots: &'a [StreetSnapshot], key: &CompatKey, history: &[(Seat, Action)]) -> Option<&'a StreetSnapshot>`; `covered_prefix(snapshot: &StreetSnapshot, history: &[(Seat, Action)]) -> usize`; `street_number(s: Street) -> u8`.

**This task performs the Plan 2 snapshot replacement, in one commit.** Plan 2 ships `engine::snapshots::{SolvedStreet, SnapshotStore}` as an explicitly temporary store ("plan 3 wraps `SolvedStreet` into `StreetSnapshot`"). `SolvedStreet`'s fields are a strict subset of `StreetSnapshot`: `board` becomes `key.root_board`, `ordinal_paths` becomes `covered_paths` (spec §9.1's name), `identity_at_solve` and `solved_prefix` move into `provenance`, and `street` moves into `key`. Delete both types and make `crates/engine/src/snapshots.rs` `pub use core_replay::{SnapshotKey, SnapshotProvenance, SnapshotStore, StreetSnapshot};`. Retype `EngineCore.snapshots` and `Engine.snapshots` to `Arc<Mutex<core_replay::SnapshotStore>>`. `serve.rs`'s register site builds a `StreetSnapshot` (Task 18 Step 4's `snapshot_from_solution` is the constructor once Task 18 lands; until then build it inline with the same field mapping) and `identity_race_golden`'s two `for_hand` assertions keep working because `for_hand` is retained on the new store. Doing the swap here, rather than spreading it across Tasks 14–18, is what keeps `identity_race_golden` green between tasks.

`resolve_path` is **not** produced by this task. Chip-path resolution is spec §2's normative rule with one implementation, `proto::resolve_chip_path`; `core-replay` calls it as `proto::resolve_chip_path(&snapshot.tree.materialized, chips)`.

`select_snapshot` takes a `CompatKey`, not a full `SnapshotKey`. §9.2's compatibility predicate does not include `tree_signature` (different trees may compete when their incoming public roots match), and replay does not possess one — asking the caller for a `SnapshotKey` would invite it to fabricate a signature that the predicate then ignores. `CompatKey` carries exactly the six compared fields.

- [ ] **Step 1: Write red compatibility and registration tests.** `replay_snapshot_compatibility`: same hand/config/model/board/range hashes succeeds despite a later hand_revision; differing hand/config/model/board/ranges fails. Two compatible snapshots: longest covered observed prefix wins, then lower raw exploitability, then larger decision_id. Undo invalidates later streets and same-street snapshots whose solved_prefix no longer prefixes the new history; keep compatible earlier snapshots with their original immutable identity. A stale result is rejected even if its prefix would fit.

```rust
#[test]
fn snapshot_prefix_predicate(){
    use proto::{Seat,Action};
    let old=vec![(Seat(1),Action::Check)];
    let longer=vec![(Seat(1),Action::Check),(Seat(0),Action::Bet{to:73})];
    assert!(longer.starts_with(&old));
    assert!(!old.starts_with(&longer));
    assert!(!vec![(Seat(1),Action::Bet{to:50})].starts_with(&old));
}
```

Add this snapshot helper to the same test file. It uses full `[1326][action]` matrices and the exact `NodeStrategy` fields; no worker is needed. Its tree is a small fixture skeleton, never a production solver template. Nodes are kept in covered_paths order, and chip paths remain on NodeStrategy.

```rust
fn identity(decision_id:u64)->proto::DecisionIdentity{
    proto::DecisionIdentity{hand_id:1,hand_revision:7,decision_id,config_revision:1,model_revision:0}
}
fn snapshot(id:proto::DecisionIdentity,covered:Vec<proto::OrdinalPath>,exploitability:f32)
    ->core_replay::StreetSnapshot{
    use proto::{Action::*,Card,MaterializedNode,Street};
    let board=vec![Card(46),Card(21),Card(0)];
    let mut public=proto::Range1326([1.0;1326]);core_ranges::block_public(&mut public,&board);
    let specs=vec![
        (vec![],vec![],"oop",vec![Check],vec![None]),
        (vec![0],vec![Check],"ip",vec![Check,Bet{to:50},Bet{to:100}],vec![Some(100),None,None]),
        (vec![0,1],vec![Check,Bet{to:50}],"oop",vec![Fold,Call],vec![Some(100),Some(200)]),
        (vec![0,2],vec![Check,Bet{to:100}],"oop",vec![Fold,Call],vec![Some(100),Some(300)]),
    ];
    let materialized=specs.iter().map(|(p,_,actor,actions,terminal)|MaterializedNode{
        path:p.clone(),street:Street::Flop,actor:(*actor).into(),actions:actions.clone(),terminal_pots:terminal.clone()
    }).collect();
    let nodes=covered.iter().map(|path|{
        let (_,chips,actor,actions,_)=specs.iter().find(|s|s.0==*path).unwrap();
        let available:Vec<bool>=public.0.iter().map(|&w|w>0.0).collect();
        proto::worker::NodeStrategy{path:chips.clone(),actor:(*actor).into(),actions:actions.clone(),
            probs:available.iter().map(|&a|vec![if a{1.0/actions.len() as f32}else{0.0};actions.len()]).collect(),
            ev_chips:vec![vec![0.0;actions.len()];1326],available}
    }).collect();
    core_replay::StreetSnapshot{
        key:core_replay::SnapshotKey{hand_id:1,config_revision:1,model_revision:0,street:Street::Flop,
            root_board:board,root_range_hashes:[core_ranges::hash_scaled(&public);2],tree_signature:"snapshot_test_v1".into()},
        provenance:core_replay::SnapshotProvenance{identity_at_solve:id,
            solved_prefix:vec![(proto::Seat(1),Check)],origin:"live".into()},
        tree:proto::EffectiveTree{rules_version:3,template_id:"snapshot_test_v1".into(),root_street:Street::Flop,
            menus:std::collections::BTreeMap::new(),add_allin_threshold:0.0,force_allin_threshold:0.0,
            merging_threshold:0.0,wager_cap:1,inserted:vec![],materialized},
        nodes,covered_paths:covered,exploitability_chips:exploitability,reasons:vec![],
    }
}
#[test]
fn replay_snapshot_compatibility(){
    use proto::{Action::*,Seat};
    let history=vec![(Seat(1),Check),(Seat(0),Bet{to:50}),(Seat(1),Call)];
    let short=snapshot(identity(1),vec![vec![]],0.1);
    let long=snapshot(identity(2),vec![vec![],vec![0],vec![0,1],vec![0,2]],0.5);
    let key=core_replay::CompatKey::of(&short.key);let mut all=vec![short,long];
    assert_eq!(core_replay::select_snapshot(&all,&key,&history).unwrap().provenance.identity_at_solve.decision_id,2);
    all[1].key.model_revision=1;
    assert_eq!(core_replay::select_snapshot(&all,&key,&history).unwrap().provenance.identity_at_solve.decision_id,1);
    all[0].key.hand_id=2;
    assert!(core_replay::select_snapshot(&all,&key,&history).is_none());
    // A later hand_revision does not break compatibility: it is not a compared field.
    all[0].key.hand_id=1;
    let mut newer=all[0].clone();newer.provenance.identity_at_solve.hand_revision=9;
    assert!(core_replay::compatible(&newer.key,&key));
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test snapshots replay_snapshot_compatibility`.

- [ ] **Step 3: Implement ordinal resolution and compatibility.** The input is a complete materialized betting skeleton even when the strategy export omits the root.

```rust
#[derive(Clone,Debug,PartialEq,Eq)]
pub struct CompatKey{
    pub hand_id:u64, pub config_revision:u32, pub model_revision:u32,
    pub street:Street, pub root_board:Vec<Card>, pub root_range_hashes:[[u8;32];2],
}
impl CompatKey{
    pub fn of(k:&SnapshotKey)->CompatKey{CompatKey{hand_id:k.hand_id,
        config_revision:k.config_revision,model_revision:k.model_revision,street:k.street,
        root_board:k.root_board.clone(),root_range_hashes:k.root_range_hashes}}
}
pub fn compatible(a:&SnapshotKey,b:&CompatKey)->bool{ CompatKey::of(a)==*b }

/// §9.2: consecutive observed actions from the street root whose every mapped ordinal
/// path (both children of a translated wager, since both carry nonzero interpolation
/// coefficients) is present in `covered_paths`. Counting stops at the first uncovered
/// action; the real walk continues past it, this only measures export coverage.
pub fn covered_prefix(s:&StreetSnapshot,h:&[(Seat,Action)])->usize{
    let mut frontier:Vec<OrdinalPath>=vec![vec![]];
    let mut count=0usize;
    for (_seat,action) in h{
        if frontier.iter().any(|p|!s.covered_paths.contains(p)){break;}
        let mut next:Vec<OrdinalPath>=Vec::new();
        for path in &frontier{
            let Some(node)=s.tree.materialized.iter().find(|n|n.path==*path) else{
                return count;                                   // path left the skeleton
            };
            match node.actions.iter().position(|x|x==action){
                Some(i)=>next.push([path.as_slice(),&[i as u8]].concat()),
                None=>{
                    // Off-menu: both interpolation children must be covered to count.
                    let Some(strategy)=node_by_path(s,path) else{return count};
                    let Some(t)=interpolate(observed_fraction(s,path,action),
                        &menu_sizes(node)) else{return count};
                    let _=strategy;
                    for &(i,f) in &t.choices{ if f>0.0 {
                        next.push([path.as_slice(),&[i as u8]].concat());
                    }}
                }
            }
        }
        if next.is_empty(){break;}
        frontier=next;
        count+=1;
    }
    count
}
pub fn select_snapshot<'a>(all:&'a [StreetSnapshot],key:&CompatKey,h:&[(Seat,Action)])
    ->Option<&'a StreetSnapshot>{
    all.iter().filter(|s|compatible(&s.key,key)).max_by(|a,b|{
        covered_prefix(a,h).cmp(&covered_prefix(b,h))
            .then_with(||b.exploitability_chips.total_cmp(&a.exploitability_chips))
            .then(a.provenance.identity_at_solve.decision_id.cmp(&b.provenance.identity_at_solve.decision_id))
    })
}
```

`node_by_path` is Task 15's `snapshot_node_at`, forward-declared here as a private helper on the same module (its public name is fixed in Task 15 so it does not collide with Plan 2's `tree::node_at`); `menu_sizes(node)` returns the node's wager pot fractions and `observed_fraction` computes `s` at that node with Task 10's `wager_fraction`. Tree signature is provenance in `SnapshotKey`, never part of the compatibility predicate: different trees can compete if their incoming public roots match. Selection measures export coverage, not a guessed likelihood of the whole hand. Tie-breaking uses raw `exploitability_chips`; no display rounding. Never require coverage of all later actions to reuse a snapshot. `solved_prefix` is observed chips/seat history, not a translated synthetic history.

- [ ] **Step 4: Implement active-registration gate and mutation invalidation.**

```rust
pub struct SnapshotStore{entries:Vec<StreetSnapshot>}
impl SnapshotStore{
    pub fn new()->Self{Self{entries:vec![]}}
    pub fn register(&mut self,active:&DecisionIdentity,snap:StreetSnapshot)->bool{
        if snap.provenance.identity_at_solve!=*active{return false;}
        if snap.key.hand_id!=active.hand_id || snap.key.config_revision!=active.config_revision
            || snap.key.model_revision!=active.model_revision{return false;}
        self.entries.retain(|s|s.key.street!=snap.key.street
            || s.provenance.identity_at_solve!=*active);
        self.entries.push(snap);true
    }
    pub fn for_identity(&self,id:&DecisionIdentity)->Vec<StreetSnapshot>{
        self.entries.iter().filter(|s|s.key.hand_id==id.hand_id
            && s.key.config_revision==id.config_revision && s.key.model_revision==id.model_revision)
            .cloned().collect()
    }
    /// Retained so Plan 2's `identity_race_golden` keeps compiling after the swap.
    pub fn for_hand(&self,hand_id:u64)->Vec<&StreetSnapshot>{
        self.entries.iter().filter(|s|s.key.hand_id==hand_id).collect()
    }
    pub fn invalidate_hand(&mut self,hand_id:u64){
        self.entries.retain(|s|s.key.hand_id!=hand_id);
    }
    /// §9.2 mutation invalidation, prefix-based. Four rules, in this order:
    /// (1) a different hand is dropped entirely; (2) every snapshot of a street later
    /// than the state's current or awaited street is dropped; (3) a same-street
    /// snapshot whose `root_board` no longer matches that street's board is dropped;
    /// (4) a same-street snapshot survives iff the new street history still starts
    /// with its `solved_prefix` — append-only mutations therefore keep earlier roots.
    /// Retained snapshots keep their original immutable provenance; `undo` assigns a
    /// new `hand_revision` but never rewrites it.
    pub fn invalidate(&mut self,state:&HandState){
        let current=street_number(state.derived.street);
        self.entries.retain(|s|{
            if s.key.hand_id!=state.hand_id{return false;}                       // (1)
            if street_number(s.key.street)>current{return false;}                // (2)
            if s.key.root_board!=root_board(state,s.key.street){return false;}   // (3)
            let history=street_history(state,s.key.street);                      // (4)
            history.starts_with(&s.provenance.solved_prefix)
        });
    }
}
pub fn street_number(s:Street)->u8{
    match s{Street::Preflop=>0,Street::Flop=>1,Street::Turn=>2,Street::River=>3}
}
```

`street_history` and `root_board` are Task 15's functions; declare them in `snapshot.rs`'s sibling module and use them here (Task 15 only adds the postflop walk that also consumes them). `begin_hand`, `finish_hand` and `abandon_hand` clear by hand identity through `invalidate_hand`; `apply_action`, `set_board` and `undo` call `invalidate(&new_state)`. A config or model change assigns a new revision, and `for_identity` then returns nothing for the old snapshots without deleting them, so an in-flight hand's frozen `HandConfig` is not retroactively reclassified.

Register only already validated solutions from engine, never raw worker results. A Provisional registration is replaced by the Final of the same decision at the same street via the retain rule; the engine permits this order only and will not let a late Provisional replace a Final. Different decisions remain selection candidates. `invalidate` removes different-hand entries, all streets later than the new state's current/awaited street, and same-street entries if `!new_history.starts_with(solved_prefix)` or root board changed. Preserve earlier roots for append-only mutations. On begin/finish/abandon clear as dictated by hand identity; undo uses a new revision but does not rewrite retained provenance. `for_identity` intentionally does not require equal hand_revision or decision_id; registration does.

`ReplayInput` has no model revision field. Therefore the engine supplies the `for_identity`-filtered snapshot slice, and baseline direct callers use model_revision=0 snapshots. Document this precondition on `replay`; never derive the active model from an arbitrary snapshot. The engine remains the identity authority.

- [ ] **Step 5: Perform the engine-side swap in this same task.** Delete `SolvedStreet` and `SnapshotStore` from `crates/engine/src/snapshots.rs`, leaving `pub use core_replay::{SnapshotKey, SnapshotProvenance, SnapshotStore, StreetSnapshot};`. Add `core-replay = { path = "../core-replay" }` to `crates/engine/Cargo.toml`. In `core.rs` retype `pub snapshots: Arc<Mutex<SnapshotStore>>` to the re-exported type and keep `EngineCore::new` initializing it with `SnapshotStore::new()`. In `serve.rs` replace the `SolvedStreet` struct literal at the register site with the `StreetSnapshot` built from the same values — `key: SnapshotKey { hand_id: req.identity.hand_id, config_revision: req.identity.config_revision, model_revision: req.identity.model_revision, street: root.street, root_board: root.board.clone(), root_range_hashes: [hash_scaled(&ranges.oop), hash_scaled(&ranges.ip)], tree_signature: assumptions.tree_signature.clone() }`, `provenance: SnapshotProvenance { identity_at_solve: req.identity.clone(), solved_prefix: root.history.clone(), origin: "live".into() }`, `tree: out.tree.clone()`, `nodes: sol.nodes.clone()`, `covered_paths: out.ordinal_paths.clone()`, `exploitability_chips: sol.exploitability_chips`, `reasons: inherited.clone()`. In `engine.rs` replace the three `self.snapshots.lock().unwrap().invalidate_hand(hand_id)` calls in `mutate`, `undo` and `end_hand`: `mutate` and `undo` call `invalidate(&s)` with the new state, `end_hand` keeps `invalidate_hand(h)`.

- [ ] **Step 6: Verify and commit.** Run `cargo test -p core-replay --test snapshots`, then Plan 2's `cargo test -p engine --features testing` — in particular `identity_race_golden`, whose two `for_hand` assertions must still hold — then `cargo test --workspace`. Include same prefix with different boards, identical displayed revision in another hand, older decision with better exploitability, and cache origins `cache_exact`, `cache_approximate`, `cache_provisional` behaving identically to `live`. Add an append-only case (`apply_action` extends the street history) asserting that a snapshot whose `solved_prefix` still prefixes the new history survives with its original `identity_at_solve` unchanged.

```powershell
git add Cargo.lock crates/core-replay crates/engine
git commit -m "feat(core-replay): select and retain compatible street snapshots" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 15: Walk completed streets through partial snapshot exports

**Files:** Create `crates/core-replay/src/postflop.rs`; modify `src/{lib.rs,branches.rs,snapshot.rs}`, `tests/{snapshots.rs,branches.rs,replay.rs}`.

**Interfaces:**
- Consumes: `select_snapshot`, `NodeStrategy`, `covered_paths:Vec<OrdinalPath>`, shared branches, `interpolate`, `wager_fraction`.
- Produces: `walk_postflop(input: &ReplayInput, street: Street, output: &mut ReplayOutput)`; `snapshot_node_at<'a>(s: &'a StreetSnapshot, path: &[u8]) -> Option<&'a proto::worker::NodeStrategy>` (named `snapshot_node_at`, not `node_at`: Plan 2 already owns `tree::node_at(&[MaterializedNode], &[u8])` with different semantics, and two same-named lookups across crates invite a wrong call); `street_history(s: &HandState, street: Street) -> Vec<(Seat, Action)>`; `root_board(s: &HandState, street: Street) -> Vec<Card>`; `snapshot_root(state: &HandState, snapshot: &StreetSnapshot) -> Result<StreetRootSnapshot, UnsupportedReason>`; `uncovered(street: Street, seat: Seat, path: &[u8]) -> ApproxReason`; internal per-branch `WalkPath { ordinal: Option<OrdinalPath>, unknown_cause: Option<String> }`.

- [ ] **Step 1: Write red `replay_snapshot_prefix_reuse` for all three exports.** Use root pot=100, stacks sufficient for 50/100/73, OOP Check then IP Bet(73) then OOP Call; snapshot solved at prefix Check, menu Bet50/Bet100. Export variants:

| Export | Covered paths | Applied likelihoods |
|---|---|---|
| requested-node-only | `[0]` (Check at root is ordinal 0) | Skip OOP root Check; translate IP bet; skip OOP calls at `[0,1]`/`[0,2]`. |
| root-only | `[]` | Condition OOP Check; skip uncovered IP bet and uncovered calls. |
| complete street | `[]`, `[0]`, `[0,1]`, `[0,2]` | Condition Check, translated bet and each mapped Call exactly once. |

Use Check likelihood .5 on the OOP supported combos, IP Bet50=(.9,.3), Bet100=(.1,.5); continuation Call=(.5,1) after50 and (.2,.8) after100. Include complementary actions so every available row sums to one. For each export, calculate expected vector/log using the Task 11 formulas independently and assert that an omitted action changes neither q nor masses in that branch. For requested-node-only, reasons must include `uncovered path []` and both mapped call paths. For root-only, IP's uncovered bet has no likelihood and no guessed split.

Add §13.1's T3 assertion that belongs to this rule and has no home in Task 11's pure-kernel test: **an inserted observed bet changes the posterior; forcing its probability to 1 fails.** In the complete-street export, add a fourth variant whose snapshot menu contains an *inserted* `Bet{to:73}` at `[0]` with solved probability `(.4,.2)` and run the same `Check, Bet73, Call` line. Assert that the resulting IP marginal equals the independently computed `.4`/`.2` conditioning, and add a negative assertion that the vector produced by forcing `P(Bet73|c) = 1` differs from it by more than the 1e-10 tolerance — an inserted size is a tree action with a solved probability, never a certainty. Task 19's audit table records that `replay_cross_actor_branches`' inserted-size half lives here.

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test snapshots replay_snapshot_prefix_reuse`; expect fallback-only behavior to differ from expected conditioned ranges.

- [ ] **Step 3: Implement covered-node lookup and once-only path walk.**

```rust
pub fn snapshot_node_at<'a>(s:&'a StreetSnapshot,path:&[u8])->Option<&'a proto::worker::NodeStrategy>{
    s.covered_paths.iter().position(|p|p==path).and_then(|i|s.nodes.get(i))
}
pub fn street_history(s:&HandState,street:Street)->Vec<(Seat,Action)>{
    s.actions.iter().filter(|a|a.street==street).map(|a|(a.seat,a.action.clone())).collect()
}
pub fn root_board(s:&HandState,street:Street)->Vec<Card>{
    let n=match street{Street::Preflop=>0,Street::Flop=>3,Street::Turn=>4,Street::River=>5};
    s.board.iter().take(n).copied().collect()
}
pub fn uncovered(street:Street,seat:Seat,path:&[u8])->ApproxReason{
    ApproxReason::UnconditionedPriorStreet{street,seat,cause:format!("uncovered path {path:?}")}
}
```

At a street root: block board, rescale, compute incoming OOP/IP public `hash_scaled` values before any action on that street. Select one compatible snapshot for the street, inherit all its reasons/origin, reset each non-residual branch's stopped field and ordinal path to root (a preflop stop is scoped to preflop). Residual remains frozen. Root OOP/IP roles come from the genuine or admitted projected `StreetRootSnapshot`, never virtual preflop positions.

Recover a snapshot's financial root from its solved prefix through the model, not from the final decision pot. `snapshot_root(state:&HandState,snapshot:&StreetSnapshot)->Result<StreetRootSnapshot,UnsupportedReason>` enumerates actual same-street prefix cutoffs, so an admitted multiway projection includes the folded seats' intervening actions and dead money rather than truncating by the HU prefix's shorter length:

```rust
pub fn snapshot_root(state:&HandState,snapshot:&StreetSnapshot)
    ->Result<proto::StreetRootSnapshot,UnsupportedReason>{
    let street=snapshot.key.street;
    let before=state.actions.iter().take_while(|a|street_number(a.street)<street_number(street)).count();
    let count=state.actions.iter().filter(|a|a.street==street).count();
    for cut in 0..=count{
        let mut solved=state.clone();solved.actions.truncate(before+cut);
        solved.board=snapshot.key.root_board.clone();
        solved.phase=proto::HandPhase::Betting{street};solved.derived=core_model::derive(&solved);
        if solved.derived.to_act!=Some(solved.hero) || solved.derived.legal.len()<2{continue;}
        if let Ok(mut root)=core_model::street_root(&solved){
            if root.history==snapshot.provenance.solved_prefix{
                root.history.clear();return Ok(root);
            }
        }
    }
    Err(UnsupportedReason::UnsupportedHistory{reason:"snapshot root not reproducible".into()})
}
```

Snapshot registration has already validated the requested actor and source tree; the retained solved prefix must reproduce its financial root before reuse. A failed reconstruction makes that candidate incompatible, with an explicit unconditioned reason if no candidate remains, never adjusted chips. Replay a mapped parent by copying this financial root, setting history to that branch's mapped HU actions, and calling `core_model::replay_root`; its Derived supplies pot, actor contribution and call cost for `wager_fraction`. Export coverage is not needed to reconstruct money from a known skeleton action.

At each observed action and each live branch: resolve the materialized node at the current ordinal path. If its strategy is exported and available, extract the matching action column, call `condition` once and advance by its action index. **Inserted observed sizes use their solved probability, never 1.** If exported but wager absent from its strategy menu, compute fractions at that branch's mapped financial prefix, interpolate over actual menu wagers, split and advance children by menu ordinals, then cap globally. If unexported, add `uncovered path <ordinal path>`, keep q and every mass conditioned so far, and advance only through an exactly represented skeleton action. A later covered node still conditions its actor; absence never clears an earlier prefix update and does not create a stopped-preflop flag.

```rust
pub fn walk_postflop(input:&ReplayInput,street:Street,output:&mut ReplayOutput){
    let board=root_board(input.state,street);
    let history=street_history(input.state,street);
    let hashes=[hash_of(output,oop_seat(input.state,street)),hash_of(output,ip_seat(input.state,street))];
    let key=CompatKey{hand_id:input.state.hand_id,
        config_revision:input.state.config.config_revision,
        model_revision:model_revision_of(input.snapshots),
        street,root_board:board.clone(),root_range_hashes:hashes};
    let Some(snapshot)=select_snapshot(input.snapshots,&key,&history) else{
        for seat in acting_seats(&history){
            output.reasons.push(ApproxReason::UnconditionedPriorStreet{
                street,seat,cause:"no compatible snapshot".into()});
        }
        return;                       // masses are kept exactly as the prior street left them
    };
    if snapshot_root(input.state,snapshot).is_err(){
        for seat in acting_seats(&history){
            output.reasons.push(ApproxReason::UnconditionedPriorStreet{
                street,seat,cause:"snapshot root not reproducible".into()});
        }
        return;
    }
    output.reasons.extend(snapshot.reasons.iter().cloned());
    // A preflop stop is scoped to preflop: every non-residual branch restarts at the
    // snapshot root with a defined ordinal path. The residual stays frozen.
    let mut paths:Vec<WalkPath>=output.branches.iter().map(|b|
        if b.residual{WalkPath{ordinal:None,unknown_cause:Some("residual".into())}}
        else{WalkPath{ordinal:Some(vec![]),unknown_cause:None}}).collect();
    for b in output.branches.iter_mut().filter(|b|!b.residual){ b.stopped=None; }
    for (seat,action) in &history{
        let before=output.branches.clone();
        let before_paths=paths.clone();
        let mut next:Vec<HistoryBranch>=Vec::new();
        let mut next_paths:Vec<WalkPath>=Vec::new();
        let mut any_support=false;
        for (b,path) in output.branches.iter().zip(&paths){
            let Some(ordinal)=path.ordinal.clone() else{                    // frozen branch
                next.push(b.clone());next_paths.push(path.clone());continue;
            };
            let Some(node)=snapshot.tree.materialized.iter().find(|n|n.path==ordinal) else{
                next.push(b.clone());
                next_paths.push(WalkPath{ordinal:None,unknown_cause:Some("path left the skeleton".into())});
                continue;
            };
            match (snapshot_node_at(snapshot,&ordinal),node.actions.iter().position(|x|x==action)){
                // (1) exported and on the menu: condition once, advance by the index.
                (Some(strategy),Some(i)) if *seat==seat_of(&strategy.actor,input.state,street)=>{
                    let p:Vec<f64>=strategy.probs.iter().enumerate()
                        .map(|(c,row)|if strategy.available[c]{row[i] as f64}else{0.0}).collect();
                    match condition(b,*seat,&p,1.0){
                        Some(mut child)=>{child.translated.push((*seat,action.clone()));
                            any_support=true;next.push(child);
                            next_paths.push(WalkPath{ordinal:Some([ordinal.as_slice(),&[i as u8]].concat()),unknown_cause:None});}
                        None=>{}                                    // M_k == 0: branch removed
                    }
                }
                // (2) exported but the wager is off this node's menu: translate and split.
                (Some(_),None)=>{
                    let money=core_model::replay_root(&mapped_root(input,snapshot,b,&ordinal))
                        .expect("a skeleton prefix replays");
                    let s=wager_fraction(raise_to(action).unwrap(),
                        money.committed_this_street[seat.0 as usize],money.facing,money.pot);
                    match interpolate(s,&menu_sizes(node)){
                        Some(t)=>{
                            output.reasons.push(translation_reason(street,*seat,s,&t));
                            for &(i,f) in &t.choices{
                                if f<=0.0{continue;}
                                let p:Vec<f64>=snapshot_node_at(snapshot,&ordinal).unwrap()
                                    .probs.iter().map(|row|row[i] as f64).collect();
                                if let Some(mut child)=condition(b,*seat,&p,f){
                                    child.parent=Some(b.id);child.split_by=Some(*seat);
                                    child.translated.push((*seat,node.actions[i].clone()));
                                    any_support=true;next.push(child);
                                    next_paths.push(WalkPath{ordinal:Some([ordinal.as_slice(),&[i as u8]].concat()),unknown_cause:None});
                                }
                            }
                        }
                        None=>{next.push(b.clone());
                            next_paths.push(WalkPath{ordinal:None,unknown_cause:Some("unmappable size".into())});}
                    }
                }
                // (3) unexported: never conditioned; advance only through an exact
                // skeleton action, otherwise freeze this branch for the street (S14).
                (None,index)=>{
                    output.reasons.push(uncovered(street,*seat,&ordinal));
                    next.push(b.clone());
                    next_paths.push(match index{
                        Some(i)=>WalkPath{ordinal:Some([ordinal.as_slice(),&[i as u8]].concat()),
                            unknown_cause:Some(format!("uncovered path {ordinal:?}"))},
                        None=>WalkPath{ordinal:None,
                            unknown_cause:Some(format!("uncovered path {ordinal:?}"))},
                    });
                }
                (Some(_),Some(_))=>{                    // exported node, but a different actor
                    next.push(b.clone());next_paths.push(path.clone());
                }
            }
        }
        if !any_support && next.len()==output.branches.len()
            && next.iter().zip(&before).any(|(a,z)|a.q!=z.q){
            output.branches=before;paths=before_paths;
            output.reasons.push(zero_reason(street,*seat,action));
            continue;
        }
        output.branches=next;paths=next_paths;
        cap_branches_with(&mut output.branches,&mut paths);
        rescale(&mut output.branches,&mut output.log_reach);
    }
}
```

`cap_branches_with` is `cap_branches` extended to drop the `WalkPath` entries of merged branches alongside them, so the two vectors stay index-aligned; `hash_of`, `oop_seat`, `ip_seat`, `acting_seats`, `seat_of`, `mapped_root`, `menu_sizes`, `raise_to`, `translation_reason` and `model_revision_of` are private helpers in this module. `model_revision_of` reads the revision from the supplied slice's first entry and asserts every entry agrees — the engine filters by `for_identity` before calling `replay`, so a mixed slice is a caller bug.

- [ ] **Step 4: Preserve justified paths when an uncovered node has a later off-menu wager.** The complete skeleton is available even for unexported nodes, so in-menu observations advance without likelihood. When an off-menu observed action is absent from an unexported skeleton node too, §9.2 does not supply either a strategy likelihood or a unique next ordinal. Do not split using f alone, do not append a nonexistent index, and do not guess a branch. Retain the branch's q/masses, mark `WalkPath.ordinal=None` with the original uncovered cause, and leave the remaining actions on that branch/street unconditioned. Reset path at the next street root. Revision 6 (S14) makes this the specified behaviour rather than a plan deviation: §9.2 now reads *"A branch whose mapped continuation is not covered freezes navigation for that branch for the remainder of the street; later observed actions in other branches are unaffected."* Assert exactly that in the root-only fixture — the frozen branch's `q` and masses are unchanged by every later action, while a sibling branch whose path stayed defined keeps conditioning.

For `replay_missing_continuation`, use a different fixture where all observed actions **are** skeleton actions but one strategy node is deleted: after skipping that action advance its exact ordinal and verify another seat, the same actor at a later covered node, and other branches continue normally. This distinguishes a missing export from an unknowable mapped continuation.

- [ ] **Step 5: Test zero support, missing snapshots, no current-street conditioning.** A covered action with zero integrated likelihood in every applying branch rejects the whole update; all pre-action q/masses survive and cause is `zero support after <action>`. Positive 1e-30 remains valid. A completed multiway street without a compatible HU snapshot emits `UnconditionedPriorStreet{cause:"multiway prior street"}` for actual actors. Other missing snapshots use concrete engine failure/deadline/no-request provenance when available, otherwise `no compatible snapshot`. Keep the already computed incoming ranges. Never solve a prior street during replay.

`replay` loops over completed streets only: for turn, replay flop; for river, replay flop then turn; for flop, preflop only. Block current root board after prior streets, but exclude current-street actions because Plan 2's street-root solve conditions them. Test appending a current-street Bet(73) leaves `SolveInput.ranges` unchanged while the effective tree/history changes. Apply the T6 cross-seat test through snapshot nodes too, not only through the preflop branch kernel.

- [ ] **Step 6: Verify and commit.** `cargo test -p core-replay`; `cargo test --workspace`. All partial-export tests run without worker processes or re-solving.

```powershell
git add crates/core-replay
git commit -m "feat(core-replay): walk covered snapshot paths and translate later wagers" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 16: Assemble branch-supported strategy, EV and unresolved mass

**Files:** Modify `crates/core-preflop/src/{translate.rs,lib.rs}`, `crates/core-replay/src/lib.rs`; create `crates/core-replay/tests/assembly.rs`.

**Interfaces:**
- Consumes: `ExpandedNode`, `posterior`, legal mapped action identity, `ActionAdvice`, `Unavailable`.
- Produces: `BranchNode { branch_id:u8, node:Option<ExpandedNode>, key:String }`; `MixedNode { actions:Vec<ActionAdvice>, unresolved_mass:f32, range_mix:Option<Vec<(Action,f32)>>, reasons:Vec<ApproxReason>, notes:Vec<String>, unsupported:Option<UnsupportedReason> }`; `mix_nodes(branches:&[HistoryBranch], nodes:&[BranchNode], hero:Seat, hero_combo:usize, bb_chips:u32)->MixedNode`; `mix_action(posterior:&[f64], probs:&[Option<f64>], evs:&[Option<f64>], same_reference:bool)->(f64,Option<f64>,Option<Unavailable>)`.

- [ ] **Step 1: Write red T7 `replay_incomplete_branch_ev`.**

```rust
#[test]
fn replay_incomplete_branch_ev(){
    use core_preflop::mix_action;
    let (p,ev,why)=mix_action(&[0.2,0.8],&[Some(0.5),None],&[Some(10.),None],true);
    assert!((p-0.1).abs()<1e-12);assert_eq!(ev,None);
    assert!(matches!(why,Some(proto::Unavailable::BranchSupportIncomplete{covered_posterior})
        if (covered_posterior-0.2).abs()<1e-6));
    let (_,ev,_)=mix_action(&[0.2,0.8],&[Some(0.5),Some(0.2)],&[Some(10.),Some(-2.)],true);
    assert!((ev.unwrap()-0.4).abs()<1e-12);
    let (_,ev,why)=mix_action(&[0.19,0.76,0.05],&[Some(0.5),Some(0.2),None],
        &[Some(10.),Some(-2.),None],true);
    assert_eq!(ev,None);
    assert!(matches!(why,Some(proto::Unavailable::BranchSupportIncomplete{covered_posterior})
        if (covered_posterior-0.95).abs()<1e-6));
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test assembly replay_incomplete_branch_ev`.

- [ ] **Step 3: Implement the complete-support EV rule.** For positive hero posterior B+, an action is present only when kind and **actual mapped chip amount** match. `P(a|c)=sum_{k has a} pi_{H,k}[c]*P_k(a|c)`; `unresolved_mass=sum_{k has no node} pi_{H,k}[c]`. `EV(a|c)=sum_{k in B+} pi_{H,k}[c]*EV_k(a|c)` only if every positive branch has this action with normalized EV under the same reference. Never weight EV by action frequency and never divide by covered posterior.

```rust
pub fn mix_action(pi:&[f64],ps:&[Option<f64>],evs:&[Option<f64>],same:bool)
    ->(f64,Option<f64>,Option<Unavailable>){
    assert_eq!(pi.len(),ps.len());assert_eq!(pi.len(),evs.len());
    let freq=pi.iter().zip(ps).map(|(w,p)|w*p.unwrap_or(0.)).sum();
    let active:Vec<usize>=(0..pi.len()).filter(|&k|pi[k]>0.).collect();
    let present=active.iter().filter(|&&k|ps[k].is_some()).count();
    if present==0{return (freq,None,Some(Unavailable::NotInMenu));}
    let covered:f64=active.iter().filter(|&&k|ps[k].is_some() && evs[k].is_some()).map(|&k|pi[k]).sum();
    let complete=active.iter().all(|&k|ps[k].is_some() && evs[k].is_some());
    if complete && same {
        return (freq,Some(active.iter().map(|&k|pi[k]*evs[k].unwrap()).sum()),None);
    }
    if present!=active.len(){
        return (freq,None,Some(Unavailable::BranchSupportIncomplete{covered_posterior:covered as f32}));
    }
    (freq,None,Some(Unavailable::NoEvReference))
}
```

The §8.4 wording overlaps two unavailable cases: if every branch has a but some EV cannot normalize, use the specifically stated `NoEvReference`; if a node/action is absent in some positive branches, use `BranchSupportIncomplete`, including covered=0 when unresolved node mass exists. Chart-only complete nodes override NoEvReference with `ChartNoEv`; unverified-source complete nodes use NoEvReference plus EvReferenceUnverified. A legality-created destination preserves `MovedProbability` when that is the reason for its missing EV. Add explicit tests for each precedence; none fabricate a value. Branch-complete EV checks compare EvReference values before averaging; a mixed-reference set yields None/NoEvReference even when individually normalized numbers happen to agree.

- [ ] **Step 4: Build hero combo advice and the range-level mix.** Union legal mapped menus in deterministic order, mix each action through the kernel, and convert final chip EV to bb once. Some no-node branches: add `BranchResidual{seat:hero,residual_mass_pct:100*unresolved_mass,cause:"missing node <key>"}`, mark their current preflop nodes stopped, include unresolved mass, no renormalization. All positive branches lack hero node: `Unsupported{MissingPreflopNode{key}}`, choose heaviest q branch's retained key (ties creation order). Residual has no source key; use the heaviest known stopped/live prefix key from query diagnostics, not an empty invented node.

```rust
pub fn mix_nodes(branches:&[HistoryBranch],nodes:&[BranchNode],hero:Seat,hero_combo:usize,
    bb_chips:u32)->MixedNode{
    let pi=posterior(branches,hero,hero_combo);          // one entry per branch, sums to 1
    let index=|id:u8|branches.iter().position(|b|b.id==id).expect("branch id");
    let covered:Vec<u8>=nodes.iter().filter(|n|n.node.is_some()).map(|n|n.branch_id).collect();
    let unresolved:f64=branches.iter().enumerate()
        .filter(|(_,b)|!covered.contains(&b.id)).map(|(k,_)|pi[k]).sum();
    if pi.iter().enumerate().all(|(k,&w)|w<=0.0 || !covered.contains(&branches[k].id)){
        // Hero has a node in no positive branch: the heaviest branch names the key.
        let heaviest=branches.iter().filter(|b|pi[index(b.id)]>0.0)
            .max_by(|a,b|a.q.total_cmp(&b.q).then(b.id.cmp(&a.id)));
        let key=heaviest.and_then(|b|nodes.iter().find(|n|n.branch_id==b.id).map(|n|n.key.clone()))
            .unwrap_or_else(||heaviest_known_key(nodes));
        return MixedNode{actions:vec![],unresolved_mass:unresolved as f32,range_mix:None,
            reasons:vec![],notes:vec![],
            unsupported:Some(UnsupportedReason::MissingPreflopNode{key})};
    }
    let same=one_ev_reference(nodes);
    let mut out=MixedNode{actions:vec![],unresolved_mass:unresolved as f32,
        range_mix:Some(range_mix(branches,nodes,hero,&covered)),
        reasons:vec![],notes:vec![],unsupported:None};
    for action in union_menu(nodes){                    // deterministic §4.6 order
        let ps:Vec<Option<f64>>=branches.iter().map(|b|node_prob(nodes,b.id,&action,hero_combo)).collect();
        let evs:Vec<Option<f64>>=branches.iter().map(|b|node_ev(nodes,b.id,&action,hero_combo)).collect();
        let (freq,ev,why)=mix_action(&pi,&ps,&evs,same);
        out.actions.push(ActionAdvice{action,frequency:Some(freq as f32),
            ev_bb:ev.map(|v|(v/bb_chips as f64) as f32),unavailable:why,headline:false});
    }
    if unresolved>0.0{
        out.reasons.push(ApproxReason::BranchResidual{seat:hero,
            residual_mass_pct:(100.0*unresolved) as f32,
            cause:format!("missing node {}",heaviest_known_key(nodes))});
        out.notes.push(format!("{:.1}% of the posterior has no strategy",100.0*unresolved));
    }
    out
}
```

`node_prob`/`node_ev` return `None` when that branch has no node, or has a node whose menu lacks an action of identical kind **and identical mapped chip amount**; `union_menu` unions the mapped menus in Fold, Check, Call, wagers-ascending, AllIn order; `one_ev_reference` is true iff every contributing node carries the same `EvReference`; `heaviest_known_key` returns the retained key of the heaviest stopped-or-live branch from the query diagnostics, never an empty invented node. Range mix uses the node-covered mass, independently of the actual hero combo:

```rust
pub fn range_mix_weight(branches:&[HistoryBranch],hero:Seat,c:usize,covered:&[u8])->f64{
    branches.iter().filter(|b|covered.contains(&b.id)).map(|b|
        b.q*b.seats.iter().find(|s|s.seat==hero).unwrap().mass[c]).sum()
}
```

For each action accumulate `sum_{c,k has node} q_k*w_Hk[c]*P_k(a|c)` and divide by `sum_{c,k has node} q_k*w_Hk[c]`; disclose excluded unresolved range mass in notes. A branch's missing action contributes zero known frequency, but a missing node contributes unresolved share. Hero actual combo with zero public marginal or an explicitly unreachable class: `Unsupported{HeroComboOutOfSupport}`; per-combo `actions` have frequency=None and EV=None (`HeroOutOfSupport`), while a positive range-level mix remains. Never replace it with a nearby hand's strategy.

- [ ] **Step 5: Verify mass conservation and commit.** `replay_incomplete_branch_ev` asserts frequency(raise)=.2*P1, fold and call EV=.2*EV1+.8*EV2, unresolved=0; residual posterior .05 makes **all** EV None, incomplete covered≤.95, unresolved=.05; `sum(frequency)+unresolved=1` where hero supported. `replay_hero_out_of_support` asserts range_mix exists and no per-combo advice values. Test unequal action menus at the same kind but different chips, mixed reference variants, and no-node heaviest-key selection. Run `cargo test -p core-preflop`, `cargo test -p core-replay`, `cargo test --workspace`.

```powershell
git add crates/core-preflop crates/core-replay
git commit -m "feat(core-preflop): assemble advice only over complete branch support" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 17: Integrate the engine's chart and EV-bearing preflop decision path

**Files:** Create `crates/engine/src/preflop.rs`, `crates/engine/tests/preflop_replay.rs`; modify `crates/engine/Cargo.toml` (add `core-preflop`, and `[dev-dependencies] engine = { path = ".", features = ["testing"] }` so the integration test can reach Plan 2's `FakeClock`/`FakeWorker`/`RecordingSink`, which live in `crates/engine/src/testing.rs` behind `#[cfg(any(test, feature = "testing"))]`), `crates/engine/src/lib.rs` (add `pub mod preflop;`), `crates/engine/src/serve.rs` (replace the `Classification::Preflop` arm), `crates/engine/src/engine.rs` (`Paths.preflop`, `preflop_store`), `crates/engine/src/core.rs` (hold the loaded store), `crates/engine/src/assemble.rs` (only if a §6 row needs a new `Assumptions` field — the headline function itself is unchanged), `Cargo.lock`.

**Interfaces:**
- Consumes: Plan 2 `Engine::new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError>`, `Engine::recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError>`, `Paths { worker, preflop, cache, log }`, `serve::{serve_request, LiveRequest}`, `coverage::Classification`, `assemble::{AssemblyCtx, HeadlineSource, headline, accumulate, unsupported, fast, empty_assumptions}`, `core::EngineCore`; `core_replay::replay(ReplayInput) -> ReplayOutput`, `PreflopStore::{open, query}`, `core_preflop::mix_nodes`.
- Produces: `Engine::preflop_store(&self) -> &PreflopStore`; `preflop::serve_preflop(core: &mut EngineCore, req: &LiveRequest, ctx: &AssemblyCtx, assumptions: Assumptions)`; `preflop::preflop_final(base: Recommendation, state: &HandState, store: &PreflopStore, snapshots: &[StreetSnapshot]) -> Recommendation`; `preflop::headline_source(source: SourceKind, ev_reference: EvReference) -> HeadlineSource`.

**No second headline function.** Plan 2's `assemble::headline(&mut [ActionAdvice], unresolved_mass: f32, HeadlineSource) -> Option<String>` already implements every §4.4 rule this task needs, including the exact three label strings and the `unresolved_mass > 0` suppression, and §4.4 has one headline rule. This task maps a preflop source onto `HeadlineSource` and calls it; it does **not** add `set_headline`.

- [ ] **Step 1: Write failing engine preflop coverage tests.** Use the existing Plan 2 engine test constructor, sink, fake clock and worker injection surface; inject a worker that records all sends and verify zero `solve` messages on every preflop path. Cases in §6:

| Source/node state | Final expectation |
|---|---|
| Verified synthetic complete node | EVs normalized per Task 9; Exact only with no mapping/replay reasons. |
| Synthetic unverified reference | Frequencies, no EVs, `EvReferenceUnverified`; matching frequency headline text. |
| Chart node | Frequencies, no EVs, `ChartRounded`; highest-frequency chart headline. |
| Node only in some positive hero branches | Known frequencies, unresolved share, BranchResidual, no headline, no EV. |
| Node in no positive hero branches | `Unsupported{MissingPreflopNode}`, inherited reasons in partial. |
| Zero hero combo but positive public range | HeroComboOutOfSupport, range_mix present, no combo frequencies/EV. |

```rust
#[test]
fn chart_headline_requires_complete_frequency_support(){
    use engine::assemble::{headline,HeadlineSource};
    use engine::preflop::headline_source;
    use core_preflop::{EvReference,SourceKind};
    use proto::{Action,ActionAdvice,Unavailable};
    // The preflop path chooses the source; the label itself is Plan 2's one rule.
    assert_eq!(headline_source(SourceKind::ChartTranscription,EvReference::Unverified),
        HeadlineSource::Chart);
    assert_eq!(headline_source(SourceKind::PokerDataJson,EvReference::Unverified),
        HeadlineSource::PokerDataUnverified);
    assert_eq!(headline_source(SourceKind::PokerDataJson,EvReference::DecisionIncrementalVerified),
        HeadlineSource::Solved);
    let mut actions=vec![
        ActionAdvice{action:Action::Fold,frequency:Some(0.2),ev_bb:None,
            unavailable:Some(Unavailable::ChartNoEv),headline:false},
        ActionAdvice{action:Action::Raise{to:5},frequency:Some(0.8),ev_bb:None,
            unavailable:Some(Unavailable::ChartNoEv),headline:false},
    ];
    assert_eq!(headline(&mut actions,0.,HeadlineSource::Chart).as_deref(),
        Some("highest-frequency chart action"));
    assert!(actions[1].headline);
    assert_eq!(headline(&mut actions,0.05,HeadlineSource::Chart),None);
    assert!(actions.iter().all(|a|!a.headline));
}
```

- [ ] **Step 2: Run red.** `cargo test -p engine --test preflop_replay`.

- [ ] **Step 3: Replace `serve_request`'s preflop arm.** The exact line Plan 2 ships in `crates/engine/src/serve.rs` is

```rust
Classification::Preflop => { emit(core, &req, None, RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::EngineError { message: "no preflop path in this build (plan 3)".into(), retryable: false }, vec![], assumptions))); return; }
```

Replace it with

```rust
Classification::Preflop => { crate::preflop::serve_preflop(core, &req, &ctx, assumptions); return; }
```

and nothing else in `serve_request` changes: `classify`, the identity/NoDecision gate, the deadline and watchdog arming and the turn/river path below are untouched. `serve_preflop` emits Fast with legal intervals and pending equity through the same `emit`/`assemble::fast` pair `serve_request` uses, then replays every observed preflop action and emits Final under the same active identity, never sending a `solve` message. `Unsupported` retains partial reasons; equity arrives afterwards through the existing Equity event path. NoDecision stays Plan 2's gate (another actor, unknown cards, folded or all-in hero, fewer than two legal actions, completed hand).

`Paths` is Plan 2's four-field struct `{ worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf }`; this task **populates** `preflop`, it does not add the field. `Engine::new` loads the store once, at construction, from `paths.preflop` and then from the packaged chart fixtures directory, using the same validated loader, surfacing quarantine banners and retaining the remaining sources; synthetic fixtures are never loaded in production. The store lives on `EngineCore` as `preflop: Arc<PreflopStore>` and is cloned into `Engine`, so `Engine::preflop_store(&self) -> &PreflopStore` (Plan 4's consumption point) hands out the already loaded store and no disk read ever happens inside a recommendation. `PreflopSource: Send + Sync` (declared in Task 1) is what makes the `Arc` shareable with `engine-main`.

```rust
pub fn serve_preflop(core:&mut EngineCore,req:&LiveRequest,ctx:&AssemblyCtx,
    mut assumptions:Assumptions){
    let store=core.preflop.clone();
    let snapshots=core.snapshots.lock().unwrap().for_identity(&req.identity);
    emit(core,req,None,RecommendationEvent::Fast(
        assemble::fast(ctx,Coverage::Exact,assumptions.clone())));
    let base=assemble::fast(ctx,Coverage::Exact,std::mem::take(&mut assumptions));
    let rec=preflop_final(base,&req.state,&store,&snapshots);
    emit(core,req,None,RecommendationEvent::Final(rec));
}

pub fn preflop_final(mut base:Recommendation,state:&HandState,store:&PreflopStore,
    snapshots:&[StreetSnapshot])->Recommendation{
    let replayed=core_replay::replay(ReplayInput{cfg:&state.config,state,store,snapshots});
    merge_reasons(&mut base.coverage,replayed.reasons.clone());
    base.assumptions.ranges_used=ranges_used(&replayed,state);
    if let Some(reason)=replayed.unsupported{
        base.coverage=Coverage::Unsupported{reason,partial:partial_of(&base.coverage)};
        base.phase=proto::Phase::Final;
        return base;
    }
    let hero_combo=match state.hero_cards.map(|h|proto::combo_index(h[0],h[1])){
        Some(c)=>c,
        None=>{base.coverage=Coverage::Unsupported{
            reason:UnsupportedReason::UnsupportedHistory{reason:"hero cards unknown".into()},
            partial:partial_of(&base.coverage)};return base;}
    };
    // One BranchNode per branch: hero's mapped current node in that branch, or None.
    let prefix_len=state.actions.len();
    let mut nodes:Vec<BranchNode>=Vec::new();
    let mut source=None;
    for b in &replayed.branches{
        let answer=core_replay::query_translated(store,&state.config,state,prefix_len,b);
        merge_reasons(&mut base.coverage,answer.reasons.clone());
        base.assumptions.notes.extend(answer.notes.clone());
        if let Some(info)=&answer.bundle{ source.get_or_insert((info.source,info.ev_reference)); }
        let legalized=answer.expanded.as_ref().map(|e|legalize_expanded(e,&base.actions_legal()));
        nodes.push(BranchNode{branch_id:b.id,node:legalized,key:answer.key});
    }
    let mix=core_preflop::mix_nodes(&replayed.branches,&nodes,state.hero,hero_combo,
        state.config.bb_chips);
    assign_mix(&mut base,mix);
    if let Some((kind,reference))=source{
        if let Some(label)=assemble::headline(&mut base.actions,base.unresolved_mass,
            headline_source(kind,reference)){
            base.assumptions.notes.push(format!("headline: {label}"));
        }
    }
    base
}

pub fn headline_source(source:SourceKind,ev_reference:EvReference)->HeadlineSource{
    match source{
        SourceKind::ChartTranscription=>HeadlineSource::Chart,
        SourceKind::PokerDataJson if ev_reference==EvReference::Unverified=>
            HeadlineSource::PokerDataUnverified,
        SourceKind::PokerDataJson=>HeadlineSource::Solved,
    }
}
```

`legalize_expanded` runs Task 10's `destination_map` once for the node and `legalize_row` for hero's combo (and for every combo when the range mix needs it); `ranges_used` and `partial_of` are private helpers; `Recommendation::actions_legal()` is the `Vec<LegalAction>` already carried in `AssemblyCtx`. The reason-merging helpers are unchanged:

```rust
pub fn merge_reasons(coverage:&mut Coverage,additional:Vec<ApproxReason>){
    if additional.is_empty(){return;}
    match coverage{
        Coverage::Exact=>*coverage=Coverage::Approximate{reasons:additional},
        Coverage::Approximate{reasons}=>reasons.extend(additional),
        Coverage::Unsupported{partial,..}=>partial.extend(additional),
    }
}
pub fn assign_mix(rec:&mut Recommendation,mix:core_preflop::MixedNode){
    rec.actions=mix.actions;rec.unresolved_mass=mix.unresolved_mass;rec.range_mix=mix.range_mix;
    rec.assumptions.notes.extend(mix.notes);
    merge_reasons(&mut rec.coverage,mix.reasons);
    if let Some(reason)=mix.unsupported{
        let partial=match &rec.coverage{
            Coverage::Approximate{reasons}=>reasons.clone(),
            Coverage::Unsupported{partial,..}=>partial.clone(),Coverage::Exact=>vec![],
        };
        rec.coverage=Coverage::Unsupported{reason,partial};
    }
    rec.phase=proto::Phase::Final;
}
```

Keep `assumptions.source`, `source_accuracy`, `source_granularity="169-class"`, ranges_used mass/log provenance, translations/mappings and notes accurate for all participating bundles. Chart EV absence alone does not remove numeric **postflop** EV later; charts are incoming-range provenance. A current preflop missing node and a missing historical strategy are distinct result paths.

- [ ] **Step 4: Confirm the headline rule needs no change, and add the preflop mapping.** Plan 2's `assemble::headline` already returns `"highest EV"` when every action has `ev_bb`, `"highest-frequency action, EV incomplete"` when any action carries `Unavailable::BranchSupportIncomplete`, `"highest-frequency chart action"` for `HeadlineSource::Chart`, `"highest-frequency source action, EV reference unverified"` for `HeadlineSource::PokerDataUnverified`, `None` for `HeadlineSource::Solved` without complete EVs, and `None` whenever `unresolved_mass > 0`. Those are exactly §4.4's rules for this path, so `assemble.rs` gains no new function. The only new code is `headline_source` from Step 3, in `crates/engine/src/preflop.rs`.

Do not rank a partially EV-bearing menu by its known EVs (Plan 2's implementation already refuses to). Display labels through Plan 2's existing note representation; §4.4 defines no `Recommendation` headline string field, so nothing is added to `proto`. Notes include `"x% of the posterior has no strategy"` whenever `unresolved_mass > 0` — emitted by `mix_nodes`, so the headline suppression and the note always agree. Do not set chart fold EV = 0 to evade the no-EV rule.

Verify the assumption rather than trusting it: the first assertion of Step 1's test above pins `headline_source`'s three mappings, and Plan 2's own `recommendation_assembly_golden` pins the label strings. If Plan 2's implementation has drifted when this task executes, fix it in `assemble.rs` — still one function, not two.

- [ ] **Step 5: Verify identities, source precedence and no-EV release behavior.** Run synthetic source tests without acquiring PokerData. Load the real chart fixtures for a legal RFI decision **at every depth listed `available` in `fixtures/charts/sources.manifest.json`** (Task 4), skipping any depth recorded `unsupported`, and assert Final frequencies sum to 1, legal wager sizes, `ChartRounded` and no EV. Source precedence chooses synthetic PokerData only when explicitly injected in tests. Check previous source/replay reasons survive a current exact lookup; duplicated displays may be deduplicated by serialized reason identity without losing distinct missing paths. Assert the fake worker recorded **zero** `solve` messages on every preflop path. Run `cargo test -p engine --features testing --test preflop_replay`, the existing `recommendation_assembly_golden`, then `cargo test --workspace`.

- [ ] **Step 6: Commit.**

```powershell
git add Cargo.lock crates/engine
git commit -m "feat(engine): deliver chart-backed preflop recommendations" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 18: Feed replayed root ranges into turn and river solves and register snapshots

**Files:** Create `crates/engine/src/replay_bridge.rs`; modify `crates/engine/src/lib.rs` (add `pub mod replay_bridge;`), `crates/engine/src/ranges.rs` (nothing but the doc line — `ReplayRanges` lives in `replay_bridge.rs` and implements the trait declared here), `crates/engine/src/core.rs` (initialize `range_source` to `ReplayRanges`), `crates/engine/src/serve.rs` (call `snapshot_from_solution` at the register site), `crates/engine/src/engine.rs` (`register_snapshot`, mutation hooks), `crates/engine/tests/preflop_replay.rs`. There is **no** `crates/engine/src/postflop.rs`: Plan 2's module list has no such file and `serve.rs` already owns the turn/river path.

**Interfaces:**
- Consumes: `core_model::{street_root, replay_root}` with `RootError::{Multiway{pot_eligible}, ProjectionNotReproducing{step}, NoDecision, Preflop, Inconsistent{step}}` (five variants, spec §3.5 as amended by S2); `core_replay::replay(ReplayInput) -> ReplayOutput`; Plan 2 `ranges::{RangeSource, RootRanges}`, `build_effective_tree`, `proto::worker::validate_solution`, `proto::resolve_chip_path`, the existing Final/Provisional event path.
- Produces: `ReplayRanges { store: Arc<PreflopStore>, snapshots: Arc<Mutex<SnapshotStore>> }` with `impl RangeSource for ReplayRanges`; `Engine::register_snapshot(&mut self, active: &DecisionIdentity, snapshot: StreetSnapshot) -> bool`; `snapshot_from_solution(id: &DecisionIdentity, input: &SolveInput, s: &StreetSolution, paths: Vec<OrdinalPath>, signature: String, origin: &str, reasons: Vec<ApproxReason>) -> StreetSnapshot`; `opposing_equity_ranges(replayed: &ReplayOutput, state: &HandState) -> Vec<(Seat, Range1326)>`.

**The root-range seam is `RangeSource`, not a parallel entry point.** Plan 2 derives the street root and builds `SolveInput` inside `serve_request`; the only thing it delegates is the ranges, through `core.range_source.ranges_at_root(&req.state, &root)`. A `prepare_root` that re-derived the root and returned its own `SolveInput` would duplicate the street-root derivation and the error classification this plan says to reuse, and would not be reachable from `serve_request` at all. This task therefore implements `ranges_at_root` and installs `ReplayRanges` in `EngineCore::new`; `serve_request` is unchanged for the turn/river path.

- [ ] **Step 1: Add red root-range integration tests.** A fake worker captures Solve requests for a turn and a river after synthetic preflop replay and compatible prior-street snapshots. Compare full oop/ip arrays to direct replay; worker pot/stacks equal the financial StreetRootSnapshot, not decision-point Derived. Changing actual hero cards changes only equity/hero advice, never root ranges, hashes, q or SolveInput. Current-street Bet73 is inserted exactly by Plan 2, never translated here. A translated flop Bet73 contributes `BetTranslation` to a river Final with every other inherited reason still present.

Register a live `ok`, live `best_so_far`, and synthetic cache-hit Final through the **same** method, then append Call+next board and assert replay conditions from the registered nodes. A Provisional then Final of one decision leaves the Final snapshot; stale identities never register. These cache-origin tests construct an already validated StreetSnapshot; cache lookup/storage implementation is Plan 4.

- [ ] **Step 2: Run red.** `cargo test -p engine --test preflop_replay replay_feeds_street_root_solves`.

- [ ] **Step 3: Implement `RangeSource` for `ReplayRanges` and install it.** Plan 2's `serve_request` already derived the root and classified its errors before it asks for ranges, so the implementation below receives the root and only produces ranges, reasons and `ranges_used`:

```rust
pub struct ReplayRanges{
    pub store:Arc<PreflopStore>,
    pub snapshots:Arc<Mutex<core_replay::SnapshotStore>>,
    pub identity:Arc<Mutex<IdentityState>>,
}
impl RangeSource for ReplayRanges{
    fn ranges_at_root(&self,state:&HandState,root:&StreetRootSnapshot)
        ->Result<RootRanges,UnsupportedReason>{
        // The engine is the identity authority: `ReplayInput` carries no model
        // revision, so the snapshot slice is filtered here, never inside replay.
        let active=self.identity.lock().unwrap().active().cloned()
            .ok_or(UnsupportedReason::EngineError{message:"no active decision".into(),retryable:false})?;
        let snapshots=self.snapshots.lock().unwrap().for_identity(&active);
        let replayed=core_replay::replay(ReplayInput{
            cfg:&state.config,state,store:&self.store,snapshots:&snapshots});
        if let Some(reason)=&replayed.unsupported{return Err(reason.clone());}
        let oop=replayed.ranges[root.oop.0 as usize].clone().ok_or(UnsupportedReason::InvalidRanges)?;
        let ip=replayed.ranges[root.ip.0 as usize].clone().ok_or(UnsupportedReason::InvalidRanges)?;
        if core_ranges::mass(&oop)<=0.0 || core_ranges::mass(&ip)<=0.0{
            return Err(UnsupportedReason::InvalidRanges);
        }
        let ranges_used=vec![
            (root.oop,core_ranges::range_to_string(&oop),core_ranges::mass(&oop)),
            (root.ip,core_ranges::range_to_string(&ip),core_ranges::mass(&ip))];
        Ok(RootRanges{oop,ip,reasons:replayed.reasons.clone(),ranges_used})
    }
}
pub fn opposing_equity_ranges(replayed:&ReplayOutput,state:&HandState)->Vec<(Seat,Range1326)>{
    let Some(hero)=state.hero_cards else{return vec![]};
    state.dealt.iter().copied().filter(|&s|s!=state.hero && !state.derived.folded[s.0 as usize])
        .filter_map(|seat|replayed.ranges[seat.0 as usize].as_ref()
            .map(|r|(seat,core_ranges::hero_conditioned(r,hero)))).collect()
}
```

`EngineCore::new` sets `range_source: Box::new(ReplayRanges { store, snapshots, identity })` from the fields it already holds; `Engine::set_explicit_ranges` still overwrites it with `ExplicitRanges`, which is how Plan 2's `identity_race_golden` and `final_delivery_independent_of_worker` keep running unchanged.

Root-error classification is **not** duplicated here. `classify` in `crates/engine/src/coverage.rs` already maps `street_root`'s errors, and revision 6 (S2) gives `RootError` five variants, so that one mapping must cover them all: `Preflop => Classification::Preflop`; `Multiway | RootError::Multiway{pot_eligible}` → `Classification::Multiway{pot_eligible}`; `ProjectionNotReproducing{step}` → `Unsupported(UnsupportedHistory{reason: format!("multiway street root not reproducible at step {step}")})`; `Inconsistent{step}` → `Unsupported(EngineError{message: format!("street root inconsistent at step {step}"), retryable: false})` per §10.2; `NoDecision` → `Classification::NoDecision`. If Plan 2's `classify` still has the three-variant match when this task runs, extend it there — do not add a second classifier in `replay_bridge.rs`.

Pot eligibility counts only dealt seats. Plan 1 marks an undealt seat `folded = true` (spec §4.3 as amended by S6), so filtering `state.dealt` first is required, exactly as the classifier does. `RootError::Multiway` is decided before any numeric solve. Plan 2's exact projection assertions, analytic facing-all-in fallback, legal-action validator, deadlines and watchdog remain in the existing solve path. The analytic fallback now consumes these replayed prior-street ranges, then hero-conditioned copies from `opposing_equity_ranges`, with `UnconditionedCurrentStreet` still added.

- [ ] **Step 4: Build snapshots from resolved, validated solutions.**

```rust
pub fn snapshot_from_solution(id:&DecisionIdentity,input:&SolveInput,s:&StreetSolution,
    paths:Vec<OrdinalPath>,signature:String,origin:&str,reasons:Vec<ApproxReason>)->StreetSnapshot{
    assert!(matches!(origin,"live"|"cache_exact"|"cache_approximate"|"cache_provisional"));
    StreetSnapshot{
        key:SnapshotKey{hand_id:id.hand_id,config_revision:id.config_revision,model_revision:id.model_revision,
            street:input.root.street,root_board:input.root.board.clone(),
            root_range_hashes:[core_ranges::hash_scaled(&input.ranges[0]),
                               core_ranges::hash_scaled(&input.ranges[1])],tree_signature:signature},
        provenance:SnapshotProvenance{identity_at_solve:id.clone(),solved_prefix:input.root.history.clone(),origin:origin.into()},
        tree:input.tree.clone(),nodes:s.nodes.clone(),covered_paths:paths,
        exploitability_chips:s.exploitability_chips,reasons,
    }
}
```

Only call after `proto::worker::validate_solution` and `proto::resolve_chip_path(&input.tree.materialized, chips)` — spec §2's single chip→ordinal rule, run against **that solution's source materialized tree** — plus requested node/actor validation, the active identity check and the expiry check. `covered_paths[i]` matches nodes[i] after resolution; every unresolvable path is `EngineError{message:"invalid solution",retryable:false}` and neither delivered nor registered. Store actual caller-suit/chip nodes, not un-inverted cache payloads. Snapshot key hashes are the exact public input ranges, never current-node/hero-conditioned ranges.

- [ ] **Step 5: Add single registration and invalidation hooks.**

```rust
impl Engine{
    /// §9.2's single registration path, shared by live ok/best_so_far and (plan 4) by
    /// cache delivery. Returns false for a stale identity, which is how a late result
    /// is refused: `SnapshotStore::register` re-checks the same rule.
    pub fn register_snapshot(&mut self,active:&DecisionIdentity,snapshot:StreetSnapshot)->bool{
        if self.identity.lock().unwrap().active().as_ref()!=Some(&active){return false;}
        self.snapshots.lock().unwrap().register(active,snapshot)
    }
    pub fn preflop_store(&self)->&PreflopStore{&self.preflop}
}
```

`serve.rs` calls the same rule inline on `engine-main` (`core.snapshots.lock().unwrap().register(&a, snap)` under the already fetched `active`), so there is one registration predicate, not two. A `Provisional` registration is replaced by the `Final` of the same decision at the same street through the store's `retain`; the engine permits only that order and never lets a late `Provisional` replace a `Final`. Registration happens before accepted delivery becomes visible to a subsequent request; late rejected results never enter snapshots. Wire `apply_action`, `set_board` and `undo` to `SnapshotStore::invalidate(&new_state)` **after** their newly assigned revision, and `begin_hand`, `finish_hand` and `abandon_hand` to `invalidate_hand`. A config change mid-hand keeps the frozen `HandConfig` and its old `config_revision`, so it does not retroactively reclassify an active hand; `for_identity` simply stops matching the older snapshots under the new revision.

When snapshot selection fails, append concrete `UnconditionedPriorStreet` causes and continue with declared ranges. Store cause diagnostics in the existing engine decision provenance, including no request, multiway street, engine failure, and deadline; no missing-source failure is silently upgraded to Exact by a later solve. Full DecisionIdentity is only a registration gate; valid prefix snapshots retain their original solve revision and are selectable under later revisions.

- [ ] **Step 6: Verify and commit.** Run `cargo test -p engine --test preflop_replay`, Plan 2 `identity_race_golden`, `facing_allin_golden`, `final_delivery_independent_of_worker`, then `cargo test --workspace`. Fake-clock tests assert replay is inside existing admission-to-deadline accounting and no synchronous disk load is added to recommendation dispatch. Measure release in-memory lookup/replay against .05/.15 s targets on real chart paths; report measured durations, not brittle CI wall-clock assertions.

```powershell
git add crates/engine
git commit -m "feat(engine): replay public root ranges and register accepted snapshots" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 19: Freeze replay and bet-translation goldens and audit every scoped test

**Files:** Create `crates/engine/tests/preflop_goldens.rs`, `crates/engine/tests/golden/{replay_weights_golden.json,bet_translation_golden.json}`; modify `tools/gen_preflop_fixtures.py`, `tools/tests/test_preflop_fixtures.py`, `docs/data/chart-transcription.md`, `crates/core-replay/tests/{branches.rs,snapshots.rs,assembly.rs}`.

**Interfaces:**
- Consumes: all Plan 3 public APIs, Plan 2 Recommendation serde and identity test harness.
- Produces: exact §13.3 tests `replay_weights_golden`, `bet_translation_golden`, full immutable expected vectors/logs and a fixture-generation audit; a completed §13.1 test-name inventory. No benchmark or UI artifacts are generated in this plan.

- [ ] **Step 1: Write failing golden runners.** Both fixtures contain `schema_version:1`, `synthetic:true`, complete input records, `expected` values and tolerances. `replay_weights_golden` loads the versioned hand through Plan 1's `begin_hand`/`apply_action`, injects its fixture sources, calls public replay, compares each of six optional full 1326 vectors, q, translated histories and six log_reach values. `bet_translation_golden` drives the public interpolation/legalize/mix functions for each fixture row, checking amount/frequency/EV/unavailable/reasons/notes and headlines. Compare numeric fields with their explicit tolerance, enum tags and strings exactly; never regenerate expected output by calling the implementation under test.

```rust
#[test]
fn replay_weights_golden(){
    let v:serde_json::Value=serde_json::from_str(include_str!("golden/replay_weights_golden.json")).unwrap();
    assert_eq!(v["schema_version"],1);
    let expected=v["expected"]["ranges"].as_array().unwrap();
    assert_eq!(expected.len(),6);
    for r in expected.iter().filter(|r|!r.is_null()){
        assert_eq!(r.as_array().unwrap().len(),1326);
    }
    assert_eq!(v["input"]["dealt"],serde_json::json!([0,1,2]));
}
```

Extend the runner with the replay comparison in this step; the concrete fixture line and independent oracle are below. The shape-only assertions are guards, not sufficient acceptance. The input's three seats are BTN=0, SB=1, BB=2, stacks=200 chips each, SB=1/BB=2, no straddle, standard .05/.5bb rake. Observed preflop line: BTN Raise5, SB Fold, BB Call, then flop `Kh7d2c`; target is flop root. Source keys include the short-handed virtual UTG/HJ/CO folds. Explicit synthetic class-varying likelihoods below make any actor mixup or double conditioning visible.

- [ ] **Step 2: Run red.** `cargo test -p engine --test preflop_goldens`; expect missing golden fixtures.

- [ ] **Step 3: Generate independent 1326 expectations in Python.** This oracle uses its own combo/class indexing and scalar Bayesian arithmetic, not Rust-generated expectations. BTN raise likelihood class c is `.8 if c%2==0 else .2`; SB fold `.25 if c%3==0 else 1`; BB call `.9 if c%5==0 else .1`. Complete each action menu with the complement so conditional rows sum to 1. Expand each class before range-integrating M; counts 6/4/12 matter. Sources are explicitly synthetic and embedded in the golden input, never added to charts.

```python
import math
def combo_class(lo, hi):
    r1, r2 = 12-lo//4, 12-hi//4
    high, low = min(r1,r2), max(r1,r2)
    if high == low: return high*13+low
    return high*13+low if lo%4 == hi%4 else low*13+high
def vector_for(fn):
    return [fn(combo_class(lo,hi)) for hi in range(1,52) for lo in range(hi)]
def replay_golden_expected():
    q=1.0
    masses=[[1.0]*1326 for _ in range(3)]
    logs=[0.0]*6
    likelihoods=[vector_for(lambda c:.8 if c%2==0 else .2),
                 vector_for(lambda c:.25 if c%3==0 else 1.0),
                 vector_for(lambda c:.9 if c%5==0 else .1)]
    for actor,p in enumerate(likelihoods):
        m=math.fsum(w*x for w,x in zip(masses[actor],p))/math.fsum(masses[actor])
        q*=m
        masses[actor]=[w*x/m for w,x in zip(masses[actor],p)]
        for seat in range(3):
            removed=max(q*w for w in masses[seat])
            masses[seat]=[w/removed for w in masses[seat]]
            logs[seat]+=math.log(removed)
    board={46,21,0} # Kh7d2c: rank indices 11,5,0; suits h,d,c.
    pairs=[(lo,hi) for hi in range(1,52) for lo in range(hi)]
    ranges=[]
    for seat in range(3):
        r=[q*w if lo not in board and hi not in board else 0.0
           for w,(lo,hi) in zip(masses[seat],pairs)]
        removed=max(r); logs[seat]+=math.log(removed)
        ranges.append([w/removed for w in r])
    return {"ranges":ranges+[None]*3,"log_reach":logs,"q":q}
```

Use only the final correct board set `{46,21,0}` in the committed generator. Assert `combo_class` expands AA=6, AKs=4, AKo=12; expected vectors are f32-converted exactly once with `struct.pack/unpack` before JSON write, while q/logs remain doubles. Mark SB folded but retain its posterior vector and folded_ranges. Goldens compare every element with absolute 1e-6 and logs/q with 1e-10. The board is applied to the **output marginal** only (revision 6 S13), which is exactly what the loop above does — the per-action rescale divides unblocked masses, and the board is removed once, at the end.

The second golden line exercises translation rather than a uniform start, and needs its own oracle rather than a prose instruction:

```python
def replay_golden_offmenu_expected():
    """Second three-seat line: BTN's observed raise sits between the source menu's
    2.5bb and 8.75bb entries, so §8.4 splits the initial branch in two. Written
    against the formulas, not against the Rust kernel."""
    A, B, s = 0.5, 1.0, 0.73                      # pot fractions at the parent
    fA = (B - s) * (1 + A) / ((B - A) * (1 + s))  # 81/173
    fB = 1 - fA
    pA = vector_for(lambda c: .8 if c % 2 == 0 else .2)   # P(A | class)
    pB = vector_for(lambda c: .1 if c % 2 == 0 else .5)   # P(B | class)
    branches = []
    for f, p in ((fA, pA), (fB, pB)):
        w = [1.0] * 1326
        m = math.fsum(x * y for x, y in zip(w, p)) / math.fsum(w)
        branches.append({"q": f * m, "mass": [x * y / m for x, y in zip(w, p)],
                         "translated": "A" if p is pA else "B"})
    # SB folds in both branches with the same class-varying likelihood.
    fold = vector_for(lambda c: .25 if c % 3 == 0 else 1.0)
    sb = [[1.0] * 1326, [1.0] * 1326]
    for k, b in enumerate(branches):
        m = math.fsum(x * y for x, y in zip(sb[k], fold)) / math.fsum(sb[k])
        b["q"] *= m
        sb[k] = [x * y / m for x, y in zip(sb[k], fold)]
    total = math.fsum(b["q"] for b in branches)
    btn_marginal = [math.fsum(b["q"] * b["mass"][c] for b in branches) for c in range(1326)]
    # Cross-seat evidence: BB has not acted, so its posterior is q_k / sum_j q_j
    # for every combo — the §8.4 property the uniform line cannot exercise.
    bb_posterior = [b["q"] / total for b in branches]
    btn_posterior = [[b["q"] * b["mass"][c] / btn_marginal[c] if btn_marginal[c] > 0 else 0.0
                      for b in branches] for c in range(1326)]
    return {"f": [fA, fB], "q": [b["q"] for b in branches],
            "btn_marginal": btn_marginal, "btn_posterior": btn_posterior,
            "bb_posterior": bb_posterior,
            "translated": [b["translated"] for b in branches]}
```

The runner asserts `f == [81/173, 92/173]` to 1e-12, both branch `q` values and translated histories, BTN's full 1326 marginal to 1e-6, BTN's per-combo branch posteriors at three named combos (`AA`, `AKs`, `72o`) to 1e-10, and that BB's branch posterior is combo-independent — an actor mixup or a double conditioning changes at least one of these.

- [ ] **Step 4: Freeze bet translation cases with exact numbers.**

```python
def bet_cases():
    return [
        {"name":"below","s":.2,"menu":[.5,1.],"f":[1.,0.],"deviation":.3,"clamped":True},
        {"name":"between","s":.73,"menu":[.5,1.],"f":[81/173,92/173],"deviation":.23,"clamped":False},
        {"name":"above_no_jam","s":1.5,"menu":[.5,1.],"f":[0.,1.],"deviation":.5,"clamped":True},
        {"name":"above_with_jam","s":1.5,"menu":[1.,2.],"f":[.4,.6],"deviation":.5,"clamped":False},
        {"name":"single","s":.73,"menu":[.5],"f":[1.],"deviation":.23,"clamped":True},
        {"name":"equal","s":.73,"menu":[.5,.5],"f":[1.,0.],"deviation":.23,"clamped":True},
    ]
```

Append legal-move fixtures: below-min Raise7 (.2, EV10) with source Call (.5, EV2), legal Raise12 (.3, EV3) becomes Call .5 EV2, Raise12 .5 EV3; without source Raise12, below-min mass joins Call and preserves its own EV; above-stack Bet120(.4,EV5) plus Check(.6,EV1), stack100 creates AllIn100 .4 EVNone/MovedProbability; a source AllIn100 present retains its own EV when Bet120 is moved. Include all three T7 headline cases, no headline when unresolved=.05, and hero-out-of-support range_mix. Positive frequencies plus unresolved must sum to 1. `BetTranslation` prominence d>.10; d=.10 exactly is not prominent.

- [ ] **Step 5: Complete the spec test-name audit.** These names must exist as real tests with numeric assertions, not comments or ignored stubs:

| Crate | Tests and implementing tasks |
|---|---|
| core-preflop | `pokerdata_schema_mapping` 1–2/9; `bundle_validation_quarantine` 2/9; `pokerdata_action_path_lookup` 8; `depth_bucket_labels_per_prefix` 8; `straddle_mapping_labels` 8; `rake_profile_ordering` 8; `pokerdata_units_source_scaling` 9. |
| core-replay | `replay_bayes_two_combos` 11; `replay_off_tree_pseudo_harmonic` 11; `replay_cross_actor_branches` 11/15 (the inserted-observed-size half of §13.1's T3 — "an inserted observed bet changes the posterior; forcing 1 fails" — is asserted in Task 15's `replay_snapshot_prefix_reuse`, because only the snapshot walk has an inserted size); `replay_branch_cap_residual` 12; `replay_missing_continuation` 13/15; `replay_snapshot_prefix_reuse` 15; `replay_snapshot_compatibility` 14/18; `replay_incomplete_branch_ev` 16; `replay_hero_out_of_support` 16. |
| engine | `published_chart_bundles_load` 7; `chart_headline_requires_complete_frequency_support` 17; `replay_feeds_street_root_solves` 18; `replay_weights_golden`, `bet_translation_golden` 19; extend the existing `recommendation_assembly_golden` for charts/unverified/T7 rather than invent another spec name; retain Plan 2's identity/deadline/analytic goldens unchanged. |

Run the actual test list and targeted suites:

```powershell
python -m pytest tools/tests/test_chart_ingest.py tools/tests/test_preflop_fixtures.py -q
cargo test -p core-preflop -- --list
cargo test -p core-replay -- --list
cargo test -p core-preflop
cargo test -p core-replay
cargo test -p engine --test preflop_goldens
cargo test -p engine --test preflop_replay
cargo test --workspace
```

- [ ] **Step 6: Commit the verified goldens.** Before committing, inspect the diff: only fixture/test/documentation changes for this task; no production function patched merely to agree with a newly recorded output. Confirm the source URLs/hashes and missing-node fallback inventory are available to Plan 4.

```powershell
git add tools/gen_preflop_fixtures.py tools/tests/test_preflop_fixtures.py crates/engine/tests/preflop_goldens.rs crates/engine/tests/golden/replay_weights_golden.json crates/engine/tests/golden/bet_translation_golden.json crates/core-replay/tests docs/data/chart-transcription.md
git commit -m "test(engine): freeze preflop replay and bet translation goldens" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

## Self-review

| Spec requirement | Tasks | Review result |
|---|---|---|
| §3.2, §3.5 dependency direction and APIs | 1–2, 11, 13, 17–18 | Exact public function/type names retained; branch definitions below replay avoid a cycle. Plan 1/2 APIs are consumed, no replacement engine/model/worker; the three engine seams are named at the top of this plan. |
| §4.1 class/combination ordering | 1, 3, 5–6, 9, 19 | Explicit action-major envelope to combo-major expansion through one cached `ComboClasses` table, independent 6/4/12 oracle. |
| §4.4 availability, headline, unresolved mass | 10, 16–17, 19 | No partial-EV ranking, no headline with unresolved>0, range_mix under HeroComboOutOfSupport; the headline itself is Plan 2's single `assemble::headline`. |
| §5 step 1 source loading; step 6 preflop | 2, 4–7, 17 | Chart baseline, synthetic EV testing; conditional V9 acquisition/converter excluded. Store loaded once into `Arc<PreflopStore>` at `Engine::new`. |
| §5 step 7 turn/river root ranges and registration | 13–15, 18 | Completed streets only, public root ranges through `RangeSource::ranges_at_root`, exact current-street insertion stays in Plan 2. |
| §6 preflop rows and inherited postflop reasons | 8–10, 16–18 | Missing current node differs from historical missing strategy; reasons retained in Unsupported.partial. |
| §8.1 trait/store/query | 1–2, 8–9 | Both adapters share the envelope; future verified EV data loads through the same path. |
| §8.2 chart acquisition/transcription | 3–7 | URLs fixed, source hashes and the actual PDF node inventory frozen by the executor, 169 cells reread per grid, all EVs omitted; an unobtainable depth is recorded, not faked. |
| §8.2 schema/hash/bounds/quarantine | 1–2, 9 | Both hash and semantic checks, duplicate action/class rejection, sparse EV semantics, all-zero declaration, 64 MiB, independent bundle failure. |
| §8.3 prefixes/depth/asymmetry/rake/straddle/short-handed/sizes | 8–10, 13 | Exact formulas, six virtual roles, no hindsight; keys built per candidate bundle; `start_stack` mapping between `dealt` order and `Seat.0`; unknown chart rake disclosed rather than fabricated. |
| §8.3 EV reference variants | 9 | All four variants, fold cross-check, no-fold BB node, re-raise, live SB independence and straddle S scaling; source posts are 1 SB and 2 SB. |
| §8.4 interpolation/branch kernel/cap | 10–12 | Exact f and M formulas, cross-actor q, common rescale, one persistent residual and deterministic ties. |
| §8.4 branch EV and legal moves | 10, 16–17 | Known frequencies, unresolved share, no EV over missing support, own destination EV only; one shared destination map for all 1326 rows. |
| §9.1 public replay and snapshot types | 11, 13–14 | Exact fields, model-scoped snapshot slice supplied by engine; `StreetSnapshot` is the only snapshot record in the workspace. |
| §9.2 start, walk, reuse, zero support, log reach | 11, 13–15, 18–19 | Uniform public start, per-action transaction, f64 internal, positive output guard, hero copies only after replay; board blocking at the output marginal per revision 6 S13. |
| §9.3 missing strategies and frozen branches | 13, 15–16 | Preflop freeze until next street; uncovered known postflop paths continue; uncovered mapped continuation freezes that branch for the street per revision 6 S14. |
| §12 errors/lifecycle | 2, 13–18 | Quarantine, zero support, malformed path, stale identity and prefix-based mutation invalidation covered. |
| §13.1 all core-preflop/core-replay rows | 1–2, 8–16, 18–19 | Exact names and supplied numeric targets listed in Task 19's audit table. |
| §13.3 two requested engine goldens | 19 | Full vectors/log reach, all interpolation/legal-move cases, and a second off-menu line; two independent Python oracles. |
| §13.5 chart baseline handoff | 7, 19 | Freeze actual nodes, source hashes and missing-node fallbacks before Plan 4 generates benchmark/E2E inputs; unavailable depths are handed over as such; no bench claim made here. |

**Placeholder scan:** Completed against the saved plan: zero matches for the skill's banned deferred-code markers and ellipsis substitutions. The structural audit found 19 tasks, each with Files, Interfaces, checkbox steps, a commit and a `cargo test --workspace` gate; code fences are balanced. Every signature named in a task's **Produces** block has a Rust body in the task that implements it. Code blocks contain concrete types, algorithms, inputs and assertions; transcription rows are deliberately entered from the fetched source rather than invented in this planning document. Full chart source/data content is an executor deliverable, not pre-existing evidence. Plans 1/2 source has not yet been generated in this workspace, so engine integration consumes their specified surface and names the exact functions and modules to extend without asserting nonexistent line numbers.

**Type-consistency check:** Money stays u32 chips/cap_mchips, internal branch values f64, envelope probabilities f32, optional EVs remain optional. Envelope is action-major `[actions][169]`; PreflopNode is class-major `[169][actions]`; ExpandedNode/NodeStrategy are combo-major `[1326][actions]`. Snapshot covered paths are ordinal; NodeStrategy wire paths remain chip paths, resolved by `proto::resolve_chip_path`. `SeatMass` and `HistoryBranch` have one definition with core-replay re-exports; `StreetSnapshot`/`SnapshotStore` likewise, with `engine::snapshots` re-exporting them. `AsymmetricStacks` carries both `stacks_bb` and `prominent` (revision 6 S11). `ReplayInput` cannot carry an active model_revision, so the engine filters snapshots by identity before calling it. `Engine::new`, `set_config` and `recommend` return `Result`; `shutdown` takes `&mut self`. Source `ev_reference` variants are serialized with the exact lowercase strings; no new wire enums or recommendation fields are assumed.

**Spec deviations declared by this plan:**

| # | Deviation | Status |
|---|---|---|
| D1 | `BundleInfo` carries `rake: Option<RakeProfile>`, plus `depth_bb`, `game` and `version`, beyond §8.1's manifest field list and §8.2's manifest, which carry only the `rake_profile` string. | Necessary: §8.3 orders rake candidates by `(rate, cap_bb, no_flop_no_drop)`, which the string cannot express, and `checked_envelope` must reject a foreign schema before decoding. `rake: None` means the source does not document its rake and is never treated as exact. **`tools/pokerdata_convert.py` (V9, out of scope) must emit these fields.** |
| D2 | `fixtures/preflop/synthetic_v2/*.json` are generated by `tools/gen_preflop_fixtures.py`; §13.0 describes them as *"hand-written from R7 §4's documented response shape"*. | Improvement: script generation is reproducible and hashable, and the script is committed alongside the fixtures so the shape is still auditable. The content is the same R7 §4 shape, still marked `synthetic: true`. |
| D3 | §9.2 snapshot compatibility omits `tree_signature`, so `compatible()` excludes it although §9.1 puts it in `SnapshotKey`. `select_snapshot` therefore takes a `CompatKey` of the six compared fields. | Correct reading of the spec, made explicit in the type so a caller cannot pass a fabricated signature that the predicate silently ignores. |
| D4 | §4.4 defines no headline string field; the label is returned by `assemble::headline` and rendered through Plan 2's existing note representation. Nothing is added to `proto`. | Unchanged from Plan 2; the three label strings match §4.4 verbatim. |
| D5 | `ReplayInput` has no `model_revision`, so the engine supplies the `for_identity`-filtered slice; direct baseline callers use `model_revision = 0`. | Documented as a precondition on `replay`; the engine remains the identity authority. |
| D6 | §7's `<= 0.15 s` / `<= 0.05 s` budgets are treated as measurement targets: Task 18 Step 6 reports measured durations instead of asserting wall-clock in CI. | The measurement is still required; only the brittle assertion is dropped. |

Revision 6 resolved the four items this plan previously listed as unresolvable conflicts: S12 (T3 wording), S13 (board blocking at the output marginal), S14 (uncovered mapped continuation freezes the branch) and S11 (`AsymmetricStacks.prominent`). Each is now implemented as specified rather than worked around, in Tasks 11, 13, 15 and 8 respectively.

**The one item that remains open, and is open by design:**

1. **Chart inventory verification.** PokerCoaching's extracted page headings and RangeConverter's article were checked at planning time; RangeConverter's PDF download timed out and the sixth PokerCoaching page has no extracted text. Tasks 4–7 require executor fetch, visual inspection, actual node and size coverage, hashes and per-cell verification, and Task 4's fallback chain plus `sources.manifest.json` make an unobtainable 200bb source a recorded, tested outcome rather than a red workspace. The completed inventory is not verified by this document; no missing response is assumed present, and rake metadata absent from a source is disclosed, never assigned the PokerData rake profile.

Out-of-scope items are deliberate series boundaries rather than omissions: `.7z`/authorized PokerData conversion and sample/rights acquisition remain V9; cache, flop policy, pre-solver, bench/E2E hands and the release gate are Plan 4; Tauri/UI are Plan 5; exploit work is deferred. This plan itself has not executed Rust/Python tests or transcribed chart data.
