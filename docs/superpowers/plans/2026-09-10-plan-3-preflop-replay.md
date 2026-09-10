# Plan 3: Preflop charts and replay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the chart-backed preflop decision path and public-range replay, including shared translated-history branches and reusable prior-street snapshots, and feed those ranges into the existing turn and river solves.

**Architecture:** `core-preflop` owns normalized sources, prefix-specific lookup, EV normalization, translation, and the shared branch arithmetic; `core-replay` re-exports the shared branch types and orchestrates preflop and completed-street replay. `engine` owns identity, snapshot registration, and recommendation delivery, consuming the Plan 1 model/range APIs and the Plan 2 street-root solver without linking the worker. Charts and synthetic EV fixtures use the same validated envelope, so enabling an authorized EV-bearing bundle requires data installation rather than a new execution path.

**Tech Stack:** Rust edition 2021, stable 1.95 or newer, the MSVC toolchain pinned by Plan 1's `rust-toolchain.toml`; existing workspace `serde`, `serde_json`, `thiserror`, and `sha2`; Python 3.12 standard library and `pytest` for development tools. No new registry version is introduced by this plan: inherit the versions already verified and pinned by Plan 1.

**Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` revision 5, especially §§3.5, 4.4, 5–6, 8–9, 12, 13.0–13.3. Read `docs/design/2026-09-10-design-outline.md` including §0b, `docs/research/R8-solver-bench.md`, and `docs/research/R7-pokerdata-verification.md` §4. This is the third of five sequential implementation plans; Plans 1 and 2 must pass before execution begins. The `.7z` converter, purchase, V9 sample, exploit slice, cache implementation, flop scheduling, and UI belong outside this plan.

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
- One commit per task, with the exact trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Run `cargo test --workspace` after each task. The commands below run from `D:/Documents/Projects/PokerAI`; steps target 2–5 minutes each. Repeated transcription steps are explicitly one grid at a time, not one entire PDF per step.

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
| `crates/core-replay/src/snapshot.rs` | Snapshot types, compatibility/selection, `SnapshotStore`. |
| `crates/core-replay/tests/branches.rs` | T3, T6, pseudo-harmonic, residual and zero-support tests. |
| `crates/core-replay/tests/replay.rs` | Uniform start, folded provenance, missing nodes and hero separation. |
| `crates/core-replay/tests/snapshots.rs` | Prefix reuse, compatibility, missing continuations. |
| `crates/core-replay/tests/assembly.rs` | T7 incomplete EV and hero-out-of-support results. |
| `crates/engine/Cargo.toml` | Add `core-preflop` and `core-replay`. |
| `crates/engine/src/lib.rs` | Existing `Engine` dispatch, store access, registration/lifecycle hooks. |
| `crates/engine/src/preflop.rs` | Preflop advice, range mix, coverage and headline handoff. |
| `crates/engine/src/replay_bridge.rs` | Replay inputs and root ranges; construct snapshots from validated solutions. |
| `crates/engine/src/postflop.rs` | Replace initial range provider in Plan 2 turn/river path. |
| `crates/engine/src/assemble.rs` | Extend existing assembly for known mass/unresolved mass and chart headlines. |
| `crates/engine/tests/preflop_replay.rs` | Engine paths, registration and inherited-reason tests. |
| `crates/engine/tests/golden/replay_weights_golden.json` | Independently computed complete 1326 vectors and log reach. |
| `crates/engine/tests/golden/bet_translation_golden.json` | Boundary, legality and no-created-EV cases. |
| `crates/engine/tests/preflop_goldens.rs` | Both §13.3 golden runners. |
| `tools/chart_ingest.py` | Fetch, normalize, validate and reproducibly build chart envelopes. |
| `tools/tests/test_chart_ingest.py` | Python matrix, key, manifest, inventory and reproduction tests. |
| `tools/gen_preflop_fixtures.py` | Deterministic synthetic v2 and independent replay golden generator. |
| `tools/tests/test_preflop_fixtures.py` | Synthetic semantics and golden arithmetic. |
| `docs/data/chart-transcription.md` | Source URLs/hashes, grid-by-grid transcription and verification record. |
| `fixtures/charts/sources/pokercoaching_100.pdf` | Frozen public source bytes; fetched by executor. |
| `fixtures/charts/sources/rangeconverter_200.pdf` | Frozen PDF resolved through the publisher download page. |
| `fixtures/charts/sources/rangeconverter_200.html` | Frozen article/download provenance, without account information. |
| `fixtures/charts/transcription/pokercoaching_100.json` | Auditable class-grid input, action legends, pages and node inventory. |
| `fixtures/charts/transcription/rangeconverter_200.json` | Auditable 0/0.5/1 grid input and node inventory. |
| `fixtures/charts/pokercoaching_100.json`, `fixtures/charts/pokercoaching_100.manifest.json` | 100bb normalized chart bundle and SHA-256 manifest. |
| `fixtures/charts/rangeconverter_200.json`, `fixtures/charts/rangeconverter_200.manifest.json` | 200bb normalized chart bundle and manifest. |
| `fixtures/preflop/synthetic_v2/node.json`, `range.json`, `spots.json` | Marked synthetic provider-shaped examples, never acquired vendor data. |
| `fixtures/preflop/synthetic_v2/nodes.json`, `manifest.json` | Dense synthetic EV-bearing normalized bundle. |
| `fixtures/preflop/synthetic_v2/cases.json` | Node paths, reference variants, explicit absences and expected values. |

## Interface ownership and execution gates

Consume Plan 1's `core_model::{begin_hand, apply_action, set_board, derive, street_root, replay_root}` with the exact §3.5 signatures; `apply_action(&HandState, Action) -> Result<HandState, RulesError>`, `derive(&HandState) -> Derived`, `street_root(&HandState) -> Result<StreetRootSnapshot, RootError>`. Consume `core_ranges::{expand_169, block_public, hero_conditioned, mass, hash_scaled}`: `expand_169(&[f32; 169]) -> Range1326`, `hero_conditioned(&Range1326, [Card; 2]) -> Range1326`, `hash_scaled(&Range1326) -> [u8; 32]`. Import all §4 types from `proto`, and `NodeStrategy`, `StreetSolution`, `validate_solution` from `proto::worker`; never redeclare them.

Plan 2 owns `Engine`, `build_effective_tree(&StreetRootSnapshot, &TemplateSelection) -> Result<EffectiveTree, UnsupportedReason>`, worker validation, identity and deadline machinery, and the existing result assembler. This plan adds `Engine::register_snapshot(&DecisionIdentity, StreetSnapshot)` and `Engine::preflop_store(&self) -> &PreflopStore`. The receiver may be `&mut self` in the engine-main state object; the argument contract is unchanged. Existing internal engine source symbols are inspected when these tasks execute; integrate into the actual Plan 2 dispatch rather than creating a second engine or worker client. New helper signatures below are Plan 3-owned contracts, not alternative names for spec APIs.

Two spec sketch details require care without changing Plan 1's `proto`: §4.4 `AsymmetricStacks` has only `stacks_bb`; compute prominence in a helper and disclose it in assumptions notes, rather than attempting a nonexistent `prominent` field. §13.1's T3 calls a two-combo distribution a posterior: with one branch `pi_{S,0}[c] == 1`; the actor's normalized combo distribution is `(0.9, 0.1)`, while an unacted seat remains uniform. Task 8 tests the binding §8.4 equations and records this wording conflict in Self-review.

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
pub trait PreflopSource {
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
pub fn valid_step(step: &str, amount: u32) -> bool {
    match step {
        "raise" => amount > 0,
        "fold" | "check" | "call" | "allin" => amount == 0,
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
        if n.history.iter().any(|(p,s,v)| !positions.contains(&p.as_str()) || !valid_step(s,*v))
            || n.actions.iter().any(|a| !valid_step(&a.step,a.to_bb_x1000.unwrap_or(0))) {
            return Err(bad("unresolved size or invalid token"));
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

Reject duplicate action kind/amount pairs and duplicate unreachable indices with the same set-insertion pattern used for history keys. Non-raise actions carrying a nonzero amount are invalid. Convert uppercase provider aliases only at ingestion (`LJ`→`UTG`, `MP`→`HJ`), not in this strict envelope reader.

- [ ] **Step 5: Verify and commit.** Run `cargo test -p core-preflop`, then `cargo test --workspace`. Expect all pass.

```powershell
git add Cargo.toml Cargo.lock crates/core-preflop
git commit -m "feat(core-preflop): validate normalized source envelopes" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 2: Load independent sources and quarantine invalid bundles

**Files:** Create `crates/core-preflop/src/store.rs`, `tools/gen_preflop_fixtures.py`, `tools/tests/test_preflop_fixtures.py`, all six `fixtures/preflop/synthetic_v2/*.json` files listed above; modify `src/{lib.rs,validate.rs}`, `tests/envelope.rs`.

**Interfaces:**
- Consumes: Task 1 `decode`, `BundleInfo`, envelope/public node types; `std::io::Read::take` and existing workspace `sha2`.
- Produces: `PreflopStore { bundles: Vec<Box<dyn PreflopSource>> }`; `PreflopStore::from_sources(Vec<Box<dyn PreflopSource>>) -> Self`, `bundles(&self) -> &[Box<dyn PreflopSource>]`, `open(dir: &Path) -> (Self, Vec<String>)`; `PokerDataJson`, `ChartTranscription`; `load_bundle(manifest: &Path, nodes: &Path) -> Result<Box<dyn PreflopSource>, BundleError>`; `node_key(&PreflopNodeKey) -> String`.

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
def make_node(history, actor, actions, committed):
    weights = [[0.0] * 169 for _ in actions]
    evs = [[None] * 169 for _ in actions]
    for c in range(169):
        weights[0][c] = 1.0
        evs[0][c] = 0.0
    if len(actions) > 1:
        weights[0][1], weights[1][1] = 0.65, 0.35
        evs[1][1] = 1.84
        evs[1][14] = 2.31  # Explicit EV retained even with zero action weight.
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

Write `node.json` with `synthetic: true`, `game: "nl"`, `version: 2`, `stack: 100`, `actor: "HJ"`, `history: "UTG_60%"`, and action records with sparse `weights`/`evs`. Write `range.json` with `synthetic: true`, `spot: "UTG_60%_HJ_Call"`, `actor: "HJ"`, `hand: "AKs"`, `freq: .35`, `ev: 1.84`, `combos: 61.9`, `weights: {"AKs": .35, "QQ": .62, "JJ": 1}`, `evs: {"AKs": 1.84, "QQ": 2.31}`. `spots.json` lists every table path with its resolved action sizes. `cases.json` records all four reference variants, SB committed=1, BB committed=2, source_stack_sb=200 and the absences. Provider `spot` ending in an action identifies an action range: remove that last pair when deriving its decision-node key. No provider HTTP calls or `.7z` decoding.

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

`node_key` serializes `(depth_bb,rake_profile,straddle,history)` deterministically. Build `committed_by_actor_sb` by replaying source posts `.5/1` and the source prefix in source-SB units: Fold/Check no payment; Call matches highest contribution capped at the source stack; Raise sets contribution to `to_bb_x1000/500`; AllIn sets it to `2*depth_bb`. Confirm actor turn order and no action after fold while building the map. Do not use `combos` in this calculation or in probabilities. The two source implementations and stable key need no separate parsing logic:

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

- [ ] **Step 5: Verify generated data and loader.** `python tools/gen_preflop_fixtures.py`, `python -m pytest tools/tests/test_preflop_fixtures.py -q`, `cargo test -p core-preflop --test envelope`, `cargo test --workspace`. Assert action-major→class-major transpose, sparse zero-weight EV retention, absent EV→None, explicit folds, SB-limp/BB-check, and unreachable class 168. Add the EV cross-check load failures in Task 6.

- [ ] **Step 6: Commit.**

```powershell
git add crates/core-preflop tools/gen_preflop_fixtures.py tools/tests/test_preflop_fixtures.py fixtures/preflop/synthetic_v2
git commit -m "feat(core-preflop): load isolated bundles with quarantine" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 3: Build the deterministic chart ingestion and validation tool

**Files:** Create `tools/chart_ingest.py`, `tools/tests/test_chart_ingest.py`, `docs/data/chart-transcription.md`.

**Interfaces:**
- Consumes: Task 1 envelope, Task 2 manifest; Python `json`, `hashlib`, `decimal`, `urllib.request`, `html.parser`, `argparse`, `pathlib`.
- Produces: `class_names() -> list[str]`, `build(transcription: dict) -> dict`, `validate(envelope: dict) -> None`; CLI `fetch`, `build`, `validate`, `verify` with explicit file arguments. Transcription format: envelope metadata plus `nodes` carrying exact history, actor, actions, `page`, `title`, `rows` (13 arrays of 13 cell strings) and a `legend` mapping a cell code to action probabilities; an `inventory` lists covered and absent node titles with reasons.

- [ ] **Step 1: Write failing Python tests.**

```python
from tools.chart_ingest import class_names, build, validate
import pytest
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

Wire `argparse` subcommands: `fetch URL OUTPUT`, `build TRANSCRIPTION OUTPUT MANIFEST`, `validate ENVELOPE`, `verify TRANSCRIPTION OUTPUT MANIFEST`. `build` uses transcription metadata for source/accuracy/rake/license, forces `source=ChartTranscription`, `ev_reference=unverified`, and hashes exactly the bytes it writes. `validate` prints every node's 169 class sums and aggregate minimum/maximum, exits nonzero on failure. `verify` prints only mismatches or a count of verified nodes/classes. Read the article HTML's anchor whose text is `6 max 200bb 500z GTO Ranges`; follow its public download page to the actual PDF using its observed link, preserving both URLs. A timeout does not justify a guessed S3 URL or a different-depth file.

- [ ] **Step 5: Verify and commit.** `python -m pytest tools/tests/test_chart_ingest.py -q`; `cargo test --workspace`.

```powershell
git add tools/chart_ingest.py tools/tests/test_chart_ingest.py docs/data/chart-transcription.md
git commit -m "feat(core-preflop): add reproducible chart ingestion tooling" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 4: Transcribe and independently verify the public chart node inventory

**Files:** Create all `fixtures/charts/` sources, transcriptions, envelopes and manifests in File structure; modify `docs/data/chart-transcription.md`, `tools/tests/test_chart_ingest.py`.

**Interfaces:**
- Consumes: `chart_ingest.py fetch/build/validate/verify`, exact §8.2 envelope and Task 2 loader.
- Produces: `pokercoaching_100.json`, `rangeconverter_200.json`, hashed manifests and an explicit covered/missing node checklist, available to Plan 4 benchmark generation. Every frequency is a transcription of a published cell, not a calculated strategy.

- [ ] **Step 1: Add failing committed-bundle tests.**

```python
from pathlib import Path
import json
import pytest
@pytest.mark.parametrize("name", ["pokercoaching_100", "rangeconverter_200"])
def test_published_chart_bundle(name):
    p = Path("fixtures/charts")
    e = json.loads((p / f"{name}.json").read_text(encoding="utf-8"))
    validate(e)
    assert e["depth_bb"] == (100 if name.startswith("pokercoaching") else 200)
    assert all("evs" not in n for n in e["nodes"])
    t = json.loads((p / "transcription" / f"{name}.json").read_text(encoding="utf-8"))
    assert build(t) == e
    assert {json.dumps(n["history"]) for n in e["nodes"]} == {
        json.dumps(n["history"]) for n in t["inventory"] if n["status"]=="covered"}
```

- [ ] **Step 2: Run red, then fetch each source separately.** Test initially fails because chart data is absent. Fetch:

```powershell
python tools/chart_ingest.py fetch https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf fixtures/charts/sources/pokercoaching_100.pdf
python tools/chart_ingest.py fetch https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem fixtures/charts/sources/rangeconverter_200.html
```

The publisher's article links to [RangeConverter's 200bb download page](https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash). Follow its actual PDF download and pass that observed URL to `fetch` with `fixtures/charts/sources/rangeconverter_200.pdf`. Record acquisition UTC date, final URL, bytes/hash, document title/version, page count, depth, action-size legend, rake disclosure and rounding rule in `docs/data/chart-transcription.md`. Planning-time verification (2026-09-10): PokerCoaching is a six-page PDF; RangeConverter's article is accessible, but its PDF download timed out. The executor must finish this source inspection before asserting a complete inventory.

- [ ] **Step 3: Inventory PokerCoaching one page at a time.** The [public PDF](https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf) text establishes these grids; visually inspect all six pages and legends, including the last page which has no extracted text. Use `LJ` as `UTG` in normalized keys.

| Node checklist | Published pages / required treatment |
|---|---|
| RFI UTG, HJ, CO, BTN, SB | Page 2; all preceding positions explicitly folded. |
| HJ vs UTG; CO vs UTG/HJ; BTN vs UTG/HJ/CO | Page 3; one opener, intervening folds. |
| SB vs UTG/HJ/CO/BTN; BB vs UTG/HJ/CO/BTN | Page 4; one opener, every intervening fold. |
| SB first-in strategy; BB vs SB limp; BB vs SB raise | Page 5; SB first-in overlaps the page-2 key: reconcile the displayed split policy, do not create duplicate nodes. |
| RFI response to a later 3bet | Inspect legend and final page: some first-action colors may encode a future response. Only add a distinct vs-3bet node if a complete conditional strategy, opponent-position scope, and sizes are explicitly supplied. Otherwise inventory `absent`; do not mistake raise/call and raise/fold annotations for independent current actions. |
| BB RFI; squeezes; cold calls vs 3bet; vs-4bet | Not established by the extracted grids; mark absent unless a complete grid and exact prefix is visually present. |

The published sizing instruction establishes 2.5bb opens except SB 3bb; IP 3bets 3.5×, OOP 4×; BB vs SB limp 3.5bb; OOP 4bets 2.5×, IP 2.3×. Freeze resolved raise-to values, not labels. For each chart title compute its complete folded prefix and actual next actor. A BB defense after an opener and folds is distinct from the same defense after a caller. No chart from one history is inserted under another.

- [ ] **Step 4: Inventory RangeConverter one page at a time.** The [200bb article](https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem) establishes RFI, vs-RFI, and vs-3bet chart families and 50% rounding. Use this finite audit matrix, marking each cell `covered` with PDF page/title/resolved sizes or `absent` with the visual audit result; the matrix is an audit checklist, not a claim that every candidate is in the PDF:

| Family | Candidate decisions to inspect individually |
|---|---|
| RFI | UTG; HJ (publisher may call it MP); CO; BTN; SB. |
| vs-RFI | HJ vs UTG; CO vs UTG/HJ; BTN vs UTG/HJ/CO; SB vs UTG/HJ/CO/BTN; BB vs UTG/HJ/CO/BTN/SB. |
| vs-3bet | UTG vs HJ/CO/BTN/SB/BB; HJ vs CO/BTN/SB/BB; CO vs BTN/SB/BB; BTN vs SB/BB; SB vs BB. |
| Blind limp lines | SB first-in limp strategy; BB vs SB limp; SB vs BB isolation. |
| Absent-node audit | Multiway callers, squeezes, cold-call-vs-3bet, vs-4bet, open-limps outside SB. |

Do not copy sizes from RangeConverter's unrelated reports or the PokerCoaching PDF. Record each resolved size from this PDF. For a vs-3bet key include folds before the open, between open and 3bet, and after the 3bettor until action returns to the opener. If the PDF lacks sufficient size information, that node is unloadable and recorded as absent-with-unresolved-size, never silently assigned a size.

- [ ] **Step 5: Transcribe each grid (repeat this checkbox once per inventory row marked covered).** Record 13 rows of 13 cell codes and a local legend; pure colors are 0/1, equal split cells are 0/.5/1 for RangeConverter; preserve PokerCoaching's actual displayed implemented choices. Read high rank first for offsuit cells. For an uncolored cell use the legend's documented fold/check action, not an unconditional fold default. `unreachable_classes` is used only when the source explicitly provides no data for a conditional class. A class absent because the opener never reaches the response may be declared unreachable only after checking the preceding source action's class weight is zero. If the response chart supplies a strategy there, preserve it.

Repeatable command after each grid edit:

```powershell
python tools/chart_ingest.py build fixtures/charts/transcription/pokercoaching_100.json fixtures/charts/pokercoaching_100.json fixtures/charts/pokercoaching_100.manifest.json
python tools/chart_ingest.py validate fixtures/charts/pokercoaching_100.json
python tools/chart_ingest.py build fixtures/charts/transcription/rangeconverter_200.json fixtures/charts/rangeconverter_200.json fixtures/charts/rangeconverter_200.manifest.json
python tools/chart_ingest.py validate fixtures/charts/rangeconverter_200.json
```

- [ ] **Step 6: Independently reread each completed grid (one grid per checkbox repetition).** Compare all 169 cells in reverse row order against the rendered source, with the first-pass JSON hidden while interpreting each row. Compare class/action totals, all boundary hands and every mixed cell. Record page, title, checked class count=169, corrections, and verification completion in the documentation. Never use a sum-to-one check as evidence that the correct cell was transcribed. Render/download inspection is development work; the plan writer does not produce those files.

- [ ] **Step 7: Freeze node coverage and benchmark fallbacks.** `verify` requires equality between covered inventory keys and bundle keys; all absent candidates stay out. Document exact available BTN/CO/HJ/UTG open→BB call prefixes at 100bb and BTN/CO open→BB call at 200bb, plus BTN open→BB 3bet→BTN call when actually supported. A missing response contributes the §9.3 frozen-range fallback with `UnconditionedPriorStreet`; label that scenario explicitly as a synthetic missing-node fallback for Plan 4. It never becomes a chart node or an unsupported-data benchmark reported as fully conditioned. Hashes and inventory must be frozen before Plan 4 generates any bench suite or 50-hand inputs.

- [ ] **Step 8: Run both validation layers and commit.** `python -m pytest tools/tests/test_chart_ingest.py -q`; run `verify` for both files; load both using `load_bundle` in the Rust envelope test, assert `ChartTranscription` and omitted EV; `cargo test --workspace`.

```powershell
git add fixtures/charts docs/data/chart-transcription.md tools/tests/test_chart_ingest.py crates/core-preflop/tests/envelope.rs
git commit -m "feat(core-preflop): transcribe and verify public chart bundles" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 5: Reconstruct each prefix and select depth, rake and virtual roles

**Files:** Create `crates/core-preflop/src/{lookup.rs,depth.rs,straddle.rs}`, `crates/core-preflop/tests/lookup.rs`; modify `src/{lib.rs,store.rs}`.

**Interfaces:**
- Consumes: `derive(&HandState) -> Derived`, Task 2 source maps and keys, `HandState.actions: Vec<TakenAction>`.
- Produces: `prefix_state(&HandState, usize) -> HandState`; `physical_positions(&HandState) -> Vec<(Seat,Position)>`; `source_unit(&HandConfig) -> u32`; `virtual_position(Position, bool) -> Position`; `depth_for(actor: u32, others: &[u32], unit: u32) -> f64`; `bucket(actual: f64, available: &[u16]) -> Option<u16>`; `rake_rank(actual: &Rake, candidate: &BundleInfo, unit: u32) -> (u8,f64,f64,u8,f64)`; `PreflopStore::query(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize) -> PreflopAnswer`.
- `PreflopAnswer` is Plan 3-owned: `pub struct PreflopAnswer { pub key: String, pub actor: Option<Seat>, pub node: Option<PreflopNode>, pub bundle: Option<BundleInfo>, pub unit: u32, pub reasons: Vec<ApproxReason>, pub notes: Vec<String>, pub unsupported: Option<UnsupportedReason> }`. It gains `expanded: Option<ExpandedNode>` in Task 6; do not fabricate a `NodeStrategy` with zero EVs.

- [ ] **Step 1: Write failing depth/rake/straddle tests with the exact thresholds.**

```rust
#[test]
fn depth_bucket_labels_per_prefix() {
    use core_preflop::{bucket,depth_for};
    let acquired = [20,30,40,50,70,100,150,200];
    for (actual,used,prominent) in [(97.,100,false),(120.,100,true),(125.,150,true),(260.,200,true)] {
        assert_eq!(bucket(actual,&acquired),Some(used));
        assert_eq!((actual-used as f64).abs()/used as f64>0.05,prominent);
    }
    assert_eq!(depth_for(100,&[104],1),100.0);
    assert!((104.0-100.0)/100.0 <= 0.05);
    assert!((110.0-100.0)/100.0 > 0.05);
}
#[test]
fn straddle_mapping_labels() {
    use proto::Position::*;
    for (physical,virtual_role) in [(Hj,Utg),(Co,Hj),(Btn,Co),(Sb,Btn),(Bb,Sb),(Utg,Bb)] {
        assert_eq!(core_preflop::virtual_position(physical,true),virtual_role);
    }
    assert_eq!(core_preflop::normalized_posts(1,2,4),[0.25,0.5,1.0]);
    assert_eq!(core_preflop::normalized_posts(2,5,10),[0.2,0.5,1.0]);
}
```

Add `rake_profile_ordering` using actual `PotRake{rate:.10,cap_mchips:6000,no_flop_no_drop:true}`, BB=5: cap_bb=1.2 maps to a candidate `.05/.5/true` with `RakeProfileMapped`; `TimeCharge` picks unraked before any raked bundle, otherwise smallest cap. Candidate ties on cap distance then rate then collection-rule mismatch then lower cap; equal remaining rank chooses smaller bundle id. Assert 100/104 carries AsymmetricStacks even without prominent display.

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

- [ ] **Step 3: Implement pure depth and role functions.** Binding formulas: `depth = min(actor.stack_start, max over other eligible seats' stack_start) / unit`; `DepthBucket` whenever `actual != used`, prominent iff `abs(actual-used)/used > .05`; AsymmetricStacks whenever **any** eligible starting stack differs from used, prominent iff any relative difference exceeds .05. Eligible means not folded **at this prefix**, including all-in players; stack values are starting stacks, not remaining stacks.

```rust
pub fn depth_for(actor: u32, others: &[u32], unit: u32) -> f64 {
    actor.min(others.iter().copied().max().unwrap_or(actor)) as f64 / unit as f64
}
pub fn bucket(actual: f64, available: &[u16]) -> Option<u16> {
    available.iter().copied().filter(|&d| d<=200).min_by(|a,b| {
        (actual-*a as f64).abs().total_cmp(&(actual-*b as f64).abs())
            .then_with(|| b.cmp(a))
    })
}
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
```

`query` must reject a prefix containing a postflop action, reconstruct actor via `derive`, and derive eligibility from that prefix. Check `cfg == state.config`; unit is actual BB unless straddle-mapped, then S. A malformed externally deserialized straddle configuration is rejected even though normal `core-model` entry already rejects it. Verify S≥2BB, six dealt seats and a starting stack covering the full straddle; unsupported formats return `FormatUnsupported`, not a guessed node. The config permits only one UTG straddle, so re-straddle input must fail model deserialization/validation before query.

Ranking is lexicographic: source kind (PokerData first), nearest depth with deeper tie, rake rank, bundle id. For rake rank use `(unknown,cap_distance,rate_distance,collection_mismatch,cap)` for PotRake; `TimeCharge` uses unraked first, then cap. Unknown source rake sorts after every documented profile at the same source/depth and always emits `RakeProfileMapped{actual,used:"undocumented chart rake"}`. Do not search lower-ranked bundles to hide a missing node after a source has been selected. For exact-sized historical raises compare `abs(to/unit - source_to/1000) <= .5/unit`; off-menu histories are handled as separate translated branches in Tasks 7/10, not rounded into a key. Cache the selected mapping by prefix and branch-translated history within a replay invocation.

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

### Task 6: Normalize all EV reference variants and expand classes

**Files:** Create `crates/core-preflop/src/ev.rs`, `crates/core-preflop/tests/ev.rs`; modify `src/{lib.rs,store.rs,validate.rs,lookup.rs}`.

**Interfaces:**
- Consumes: `PreflopNode`, `BundleInfo`, `PreflopAnswer`, `core_ranges::expand_169(&[f32;169]) -> Range1326`.
- Produces: `normalize_ev(reference: EvReference, value: Option<f32>, committed: f32, source_stack_sb: f32) -> Option<f32>`; `verify_fold(reference: EvReference, fold: Option<f32>, committed: f32, source_stack_sb: f32) -> bool`; `ev_chips(inc_sb: f32, unit: u32) -> f32`; `ExpandedNode { actor: Seat, actions: Vec<Action>, probs: Vec<Vec<f32>>, ev_chips: Vec<Vec<Option<f32>>>, available: Vec<bool>, ev_reference: EvReference, source: SourceKind }`; `expand_node(&PreflopNode, &BundleInfo, Seat, u32, u32) -> ExpandedNode` (last arguments source unit and actor's actual max raise-to).

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

```rust
pub fn expand_column(values:&[f32;169])->Vec<f32> {
    core_ranges::expand_169(values).0.to_vec()
}
pub fn expand_optional(values:&[Option<f32>;169])->Vec<Option<f32>> {
    let mut out=vec![None;1326];
    for (c,&value) in values.iter().enumerate(){
        let mut indicator=[0.0;169];indicator[c]=1.0;
        let mask=core_ranges::expand_169(&indicator);
        for (dst,m) in out.iter_mut().zip(mask.0){if m!=0.0{*dst=value;}}
    }out
}
```

EVs can be negative or >1, so `expand_optional` never passes the EV payload through a range validator. Cache the 169 indicator expansions once at bundle load for efficient column expansion; the indicators are valid Range1326 values. Keep `[1326][actions]`, copy class unreachable masks to all 6/4/12 combos. No board blocking here. Convert raise-to using integer half-up `(source_milli_bb as u64 * unit as u64 + 500)/1000` with checked u32 conversion; AllIn uses actual actor maximum, not source depth. Legality mapping runs in Task 7 after source expansion.

- [ ] **Step 5: Verify source-unit and loader integration.** Extend the test to a re-raise prefix with committed=5 source SB, no-fold BB vs limp with `stack_dec=198`, and straddle units 4 and 10. Modify synthetic fixtures in memory for each reference; bad net-start fold -.5 makes the containing node unloadable and rejects that inconsistent bundle, while other validated bundles remain active. Assert changing actual SB from 1 to 2 while BB unit=5 never multiplies EV by actual SB. Assert null EV differs from numeric 0, including a zero-frequency action carrying EV=2.31. Run `cargo test -p core-preflop`, `cargo test --workspace`.

- [ ] **Step 6: Commit.**

```powershell
git add crates/core-preflop
git commit -m "feat(core-preflop): normalize verified source EV references" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 7: Implement wager interpolation and legality after mapping

**Files:** Create `crates/core-preflop/src/translate.rs`, `crates/core-preflop/tests/translate.rs`; modify `src/lib.rs`.

**Interfaces:**
- Consumes: `Action`, `LegalAction`, `Derived`, `ExpandedNode`.
- Produces: `Interpolation { choices: Vec<(usize,f64)>, deviation: f64, clamped: bool }`; `interpolate(s:f64, menu:&[(usize,f64)]) -> Option<Interpolation>`; `wager_fraction(to:u32, own:u32, call:u32, pot:u32)->f64`; `legalize(actions:&[Action], legal:&[LegalAction], probs:&[f32], evs:&[Option<f32>])->MappedAdvice`; `MappedAction { action:Action, probability:f32, ev_chips:Option<f32>, unavailable:Option<Unavailable> }`; `MappedAdvice { actions:Vec<MappedAction>, notes:Vec<String>, unsupported:Option<UnsupportedReason> }`.

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
    if let Some(&(i,x))=sizes.iter().find(|(_,x)|*x==s){
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

- [ ] **Step 4: Implement legal destination choice and merging.** Convert amounts first. Preserve deterministic order Fold,Check,Call,wagers ascending,AllIn. Build the legal source menu before moving anything; then process every source probability exactly once. A below-min Raise goes to the smallest legal **source menu** raise, else Call; a wager above max goes to legal AllIn; a legal action maps to itself. Match actual `LegalAction` intervals and do not treat an open Bet as a Raise. Reject impossible nonwager mismatches to the caller as `UnsupportedHistory`; never guess Fold. The core merging loop is:

```rust
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

Use chip-valued `MappedAction` entries until final engine conversion. A destination already present as a source action retains only **its own** normalized EV. Created destinations get None and `MovedProbability{from}`; never average a moved source's payoff into them. Identical rounded actions merge by the same rule: prefer an independently exactly represented destination's EV; if no unique destination payoff exists, omit it with a move note. Compute all 1326 probability rows using one shared action-destination map to keep menus aligned. If no permitted destination exists, set MappedAdvice.unsupported to UnsupportedHistory; no probability disappears into a default action.

- [ ] **Step 5: Verify legal moves and commit.** Cases: Raise(7) below min=10 maps to existing legal Raise(12), preserving Raise(12)'s EV; with no legal source raise maps to Call; Bet(120) at max=100 creates AllIn(100) with None EV; existing source AllIn(100) retains its own EV; two half-up rounded wagers collide; no missing probability becomes 0 or fold. Check total probability conserved. Run `cargo test -p core-preflop --test translate`, `cargo test --workspace`.

```powershell
git add crates/core-preflop
git commit -m "feat(core-preflop): translate wagers and preserve legal probability mass" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 8: Implement shared history-branch Bayesian updates

**Files:** Create `crates/core-preflop/src/branches.rs`, `crates/core-replay/{Cargo.toml,src/lib.rs,src/branches.rs,tests/branches.rs}`; modify workspace manifests/lock and preflop exports.

**Interfaces:**
- Consumes: `proto::{Seat,Action,Range1326}`, interpolation choices from Task 7.
- Produces: exact §9.1 `SeatMass`, `HistoryBranch` in core-preflop, re-exported from core-replay; `initial(seats:&[Seat])->Vec<HistoryBranch>`; `marginal(branches:&[HistoryBranch],seat:Seat)->Vec<f64>`; `posterior(branches:&[HistoryBranch],seat:Seat,combo:usize)->Vec<f64>`; `condition(branch:&HistoryBranch,actor:Seat,p:&[f64],factor:f64)->Option<HistoryBranch>`; `rescale(branches:&mut [HistoryBranch], log_reach:&mut [f64])`; `range_output(&[f64])->Range1326`.

- [ ] **Step 1: Add replay crate and failing T3.** Manifest follows Task 1's package fields; dependencies are `proto`, `core-model`, `core-ranges`, `core-preflop` as sibling paths and existing workspace `serde`, `serde_json`, `thiserror`. Add no rayon. `core-replay/src/branches.rs` uses `pub use core_preflop::branches::*;`.

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

### Task 9: Cap live branches with a persistent frozen residual

**Files:** Modify `crates/core-preflop/src/branches.rs`, `crates/core-replay/tests/branches.rs`.

**Interfaces:**
- Consumes: Task 8 history branches and shared q/masses.
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

- [ ] **Step 4: Implement deterministic child creation and audit.** Child ids are monotonically assigned within an observed-action batch; use a wider private creation counter and, if it exceeds u8, stably compact all retained ids and parent references before assigning new ids. The public `id:u8` field remains unchanged; compaction preserves relative creation order and tie-breaking. Map every seat's next node from the **same** translated history. Audit before/after cap `marginal` equality for every seat/combo with 1e-12 tolerance, nonuniform residual masses too. Check no later on-menu or split modifies residual q or applies a guessed likelihood to it. `split_action` skips residual/stopped branches, clones their values, calls `condition` for each child, records `parent`, `split_by`, mapped action, then caps once after all parents are expanded.

- [ ] **Step 5: Verify and commit.** `cargo test -p core-replay --test branches`; `cargo test --workspace`.

```powershell
git add crates/core-preflop/src/branches.rs crates/core-replay/tests/branches.rs
git commit -m "feat(core-replay): preserve capped branches in a frozen residual" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 10: Replay preflop prefixes with frozen missing-node stops

**Files:** Create `crates/core-replay/src/{preflop.rs,snapshot.rs}`, `crates/core-replay/tests/replay.rs`; modify `crates/core-replay/src/{lib.rs,branches.rs}` and `crates/core-preflop/src/lookup.rs`.

**Interfaces:**
- Consumes: `PreflopStore::query`, `prefix_state`, `ExpandedNode`, branch kernel, `core_model::derive` and `apply_action`.
- Produces: exact `ReplayInput`, `ReplayOutput`, `replay(ReplayInput) -> ReplayOutput`; `walk_preflop(input:&ReplayInput, output:&mut ReplayOutput)`; `query_translated(&PreflopStore,&HandConfig,&HandState,usize,&HistoryBranch)->PreflopAnswer`; exact `SnapshotKey`, `SnapshotProvenance`, `StreetSnapshot` records for input typing (selection is Task 11).

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

Loop over `state.actions` until the first non-preflop action. For each live branch query its own prefix; missing node calls `stop_branch` and records the reason before any likelihood. For an on-menu action take that action's expanded probability column and call `condition` once. For an off-menu raise use Task 7 source-parent pot fractions, call `condition` for each fX, and append the mapped action to each child. For exact action advance the shared history once. Update every seat's next-node position from that common history. An unreachable class with already zero mass is left zero; a positive class with no data gets no guessed likelihood. Collect candidates for all branches before zero-support rejection and cap; keep prior branch values on an all-zero applying update. If the update is rejected, do not advance a translated betting branch using an unobserved menu action. Record the explicit reason; a later known prefix is permitted only when its path can still be derived without a guessed action.

- [ ] **Step 5: Add board blocking and boundary conversion.** At each street root construct a blocker mask through the Plan 1 API; apply it directly to f64 masses, never quantizing all masses to f32 in the middle of replay.

```rust
pub fn block_branches(branches:&mut [HistoryBranch],board:&[Card]){
    let mut mask=Range1326([1.;1326]);core_ranges::block_public(&mut mask,board);
    for b in branches{for s in &mut b.seats{for (w,keep) in s.mass.iter_mut().zip(mask.0){
        if keep==0.0{*w=0.0;}
    }}}
}
pub fn publish(output:&mut ReplayOutput,state:&HandState){
    output.ranges=(0..6).map(|i|{
        let seat=Seat(i);
        state.dealt.contains(&seat).then(||range_output(&marginal(&output.branches,seat)))
    }).collect();
    output.folded_ranges=(0..6).filter(|&i|state.derived.folded[i])
        .filter_map(|i|output.ranges[i].clone()).collect();
}
```

`replay` initializes, walks preflop, then completed streets (Task 12), blocks the current street-root board and rescales to max 1, publishes output. For this task completed streets have no consumed snapshots yet: emit `UnconditionedPriorStreet` per actual acting seat/street with `cause:"no compatible snapshot"`, retain preflop masses, block each root in order. This is a complete tested fallback, not a temporary fake strategy. Task 12 enriches it when snapshots exist. A zero total after blocking returns `InvalidRanges`; cross-seat disjoint-support validation remains Plan 1 `core-eval`/Plan 2 admission.

Board blocking can make per-seat mass totals differ between branches with distinct card distributions, despite §8.4's global equal-total assertion. Do not introduce a branch-specific board renormalization or alter q: that would change the specified marginal without a defined board-evidence rule. Keep pointwise blocking and flag this precise spec conflict in Self-review; preflop branch tests still enforce the equal-total invariant before board removal.

- [ ] **Step 6: Verify replay outputs and commit.** `cargo test -p core-replay --test replay`; assert folding conditions that seat before retaining folded_ranges, hero actions condition public hero normally, zero support preserves every q/mass, no source reach is multiplied twice, and preflop stopping clears only on entering a postflop street (residual never clears). `cargo test --workspace`.

```powershell
git add crates/core-replay crates/core-preflop/src/lookup.rs
git commit -m "feat(core-replay): replay preflop with frozen missing-node branches" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 11: Select compatible snapshots and preserve prefix-valid provenance

**Files:** Modify `crates/core-replay/src/{snapshot.rs,lib.rs}`; create `crates/core-replay/tests/snapshots.rs`.

**Interfaces:**
- Consumes: Task 10 snapshot types; `hash_scaled`, Plan 1 materialized tree and ordinal paths.
- Produces: `SnapshotStore::new()->Self`, `register(&mut self, active:&DecisionIdentity, snapshot:StreetSnapshot)->bool`, `invalidate(&mut self, state:&HandState)`, `for_identity(&self, identity:&DecisionIdentity)->Vec<StreetSnapshot>`; `select_snapshot<'a>(snapshots:&'a [StreetSnapshot], key:&SnapshotKey, history:&[(Seat,Action)]) -> Option<&'a StreetSnapshot>`; `resolve_path(tree:&EffectiveTree, chips:&[Action])->Option<OrdinalPath>`; `covered_prefix(snapshot:&StreetSnapshot,history:&[(Seat,Action)])->usize`.

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
    let key=short.key.clone();let mut all=vec![short,long];
    assert_eq!(core_replay::select_snapshot(&all,&key,&history).unwrap().provenance.identity_at_solve.decision_id,2);
    all[1].key.model_revision=1;
    assert_eq!(core_replay::select_snapshot(&all,&key,&history).unwrap().provenance.identity_at_solve.decision_id,1);
    all[0].key.hand_id=2;
    assert!(core_replay::select_snapshot(&all,&key,&history).is_none());
}
```

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test snapshots replay_snapshot_compatibility`.

- [ ] **Step 3: Implement ordinal resolution and compatibility.** The input is a complete materialized betting skeleton even when the strategy export omits the root.

```rust
pub fn resolve_path(tree:&EffectiveTree,chips:&[Action])->Option<OrdinalPath>{
    let mut path=Vec::new();
    for a in chips{
        let node=tree.materialized.iter().find(|n|n.path==path)?;
        let i=node.actions.iter().position(|x|x==a)?;
        path.push(u8::try_from(i).ok()?);
    }Some(path)
}
pub fn compatible(a:&SnapshotKey,b:&SnapshotKey)->bool{
    a.hand_id==b.hand_id && a.config_revision==b.config_revision
        && a.model_revision==b.model_revision && a.street==b.street
        && a.root_board==b.root_board && a.root_range_hashes==b.root_range_hashes
}
pub fn select_snapshot<'a>(all:&'a [StreetSnapshot],key:&SnapshotKey,h:&[(Seat,Action)])
    ->Option<&'a StreetSnapshot>{
    all.iter().filter(|s|compatible(&s.key,key)).max_by(|a,b|{
        covered_prefix(a,h).cmp(&covered_prefix(b,h))
            .then_with(||b.exploitability_chips.total_cmp(&a.exploitability_chips))
            .then(a.provenance.identity_at_solve.decision_id.cmp(&b.provenance.identity_at_solve.decision_id))
    })
}
```

Tree signature is provenance in `SnapshotKey`, not a compatibility-equality predicate in §9.2: different trees can compete if their incoming public roots match. `covered_prefix` simulates the ordinal walk including deterministic exact actions and translated size alternatives. Count consecutive observed actions for which all mapped paths with nonzero interpolation coefficients are covered; stop counting at the first uncovered action, although actual replay later continues. Selection measures export coverage, not a guessed likelihood of the whole hand. Tie-breaking uses raw `exploitability_chips`; no display rounding. Never require coverage of all later actions to reuse a snapshot. `solved_prefix` is observed chips/seat history, not a translated synthetic history.

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
}
pub fn street_number(s:Street)->u8{
    match s{Street::Preflop=>0,Street::Flop=>1,Street::Turn=>2,Street::River=>3}
}
```

Register only already validated solutions from engine, never raw worker results. A Provisional registration is replaced by the Final of the same decision at the same street via the retain rule; the engine permits this order only and will not let a late Provisional replace a Final. Different decisions remain selection candidates. `invalidate` removes different-hand entries, all streets later than the new state's current/awaited street, and same-street entries if `!new_history.starts_with(solved_prefix)` or root board changed. Preserve earlier roots for append-only mutations. On begin/finish/abandon clear as dictated by hand identity; undo uses a new revision but does not rewrite retained provenance. `for_identity` intentionally does not require equal hand_revision or decision_id; registration does.

`ReplayInput` has no model revision field. Therefore the engine supplies the `for_identity`-filtered snapshot slice, and baseline direct callers use model_revision=0 snapshots. Document this precondition on `replay`; never derive the active model from an arbitrary snapshot. The engine remains the identity authority.

- [ ] **Step 5: Verify and commit.** Run `cargo test -p core-replay --test snapshots`, `cargo test --workspace`. Include same prefix with different boards, identical displayed revision in another hand, older decision with better exploitability, and cache origins `cache_exact`, `cache_approximate`, `cache_provisional` behaving identically to `live`.

```powershell
git add crates/core-replay
git commit -m "feat(core-replay): select and retain compatible street snapshots" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 12: Walk completed streets through partial snapshot exports

**Files:** Create `crates/core-replay/src/postflop.rs`; modify `src/{lib.rs,branches.rs,snapshot.rs}`, `tests/{snapshots.rs,branches.rs,replay.rs}`.

**Interfaces:**
- Consumes: `select_snapshot`, `NodeStrategy`, `covered_paths:Vec<OrdinalPath>`, shared branches, `interpolate`, `wager_fraction`.
- Produces: `walk_postflop(input:&ReplayInput, street:Street, output:&mut ReplayOutput)`; `node_at(snapshot:&StreetSnapshot,path:&[u8])->Option<&NodeStrategy>` with elided lifetime made explicit in implementation; `street_history(&HandState,Street)->Vec<(Seat,Action)>`; `root_board(&HandState,Street)->Vec<Card>`; internal per-branch `WalkPath { ordinal:Option<OrdinalPath>, unknown_cause:Option<String> }`.

- [ ] **Step 1: Write red `replay_snapshot_prefix_reuse` for all three exports.** Use root pot=100, stacks sufficient for 50/100/73, OOP Check then IP Bet(73) then OOP Call; snapshot solved at prefix Check, menu Bet50/Bet100. Export variants:

| Export | Covered paths | Applied likelihoods |
|---|---|---|
| requested-node-only | `[0]` (Check at root is ordinal 0) | Skip OOP root Check; translate IP bet; skip OOP calls at `[0,1]`/`[0,2]`. |
| root-only | `[]` | Condition OOP Check; skip uncovered IP bet and uncovered calls. |
| complete street | `[]`, `[0]`, `[0,1]`, `[0,2]` | Condition Check, translated bet and each mapped Call exactly once. |

Use Check likelihood .5 on the OOP supported combos, IP Bet50=(.9,.3), Bet100=(.1,.5); continuation Call=(.5,1) after50 and (.2,.8) after100. Include complementary actions so every available row sums to one. For each export, calculate expected vector/log using the Task 8 formulas independently and assert that an omitted action changes neither q nor masses in that branch. For requested-node-only, reasons must include `uncovered path []` and both mapped call paths. For root-only, IP's uncovered bet has no likelihood and no guessed split.

- [ ] **Step 2: Run red.** `cargo test -p core-replay --test snapshots replay_snapshot_prefix_reuse`; expect fallback-only behavior to differ from expected conditioned ranges.

- [ ] **Step 3: Implement covered-node lookup and once-only path walk.**

```rust
pub fn node_at<'a>(s:&'a StreetSnapshot,path:&[u8])->Option<&'a proto::worker::NodeStrategy>{
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

- [ ] **Step 4: Preserve justified paths when an uncovered node has a later off-menu wager.** The complete skeleton is available even for unexported nodes, so in-menu observations advance without likelihood. When an off-menu observed action is absent from an unexported skeleton node too, §9.2 does not supply either a strategy likelihood or a unique next ordinal. Do not split using f alone, do not append a nonexistent index, and do not guess a branch. Retain the branch's q/masses, mark `WalkPath.ordinal=None` with the original uncovered cause, and leave the remaining actions on that branch/street unconditioned. Reset path at the next street root. This is the bounded §9.3 fallback; Self-review records that the spec's statement “every observed action is a tree action” is false for this explicit later-off-menu case. The root-only fixture above exercises it.

For `replay_missing_continuation`, use a different fixture where all observed actions **are** skeleton actions but one strategy node is deleted: after skipping that action advance its exact ordinal and verify another seat, the same actor at a later covered node, and other branches continue normally. This distinguishes a missing export from an unknowable mapped continuation.

- [ ] **Step 5: Test zero support, missing snapshots, no current-street conditioning.** A covered action with zero integrated likelihood in every applying branch rejects the whole update; all pre-action q/masses survive and cause is `zero support after <action>`. Positive 1e-30 remains valid. A completed multiway street without a compatible HU snapshot emits `UnconditionedPriorStreet{cause:"multiway prior street"}` for actual actors. Other missing snapshots use concrete engine failure/deadline/no-request provenance when available, otherwise `no compatible snapshot`. Keep the already computed incoming ranges. Never solve a prior street during replay.

`replay` loops over completed streets only: for turn, replay flop; for river, replay flop then turn; for flop, preflop only. Block current root board after prior streets, but exclude current-street actions because Plan 2's street-root solve conditions them. Test appending a current-street Bet(73) leaves `SolveInput.ranges` unchanged while the effective tree/history changes. Apply the T6 cross-seat test through snapshot nodes too, not only through the preflop branch kernel.

- [ ] **Step 6: Verify and commit.** `cargo test -p core-replay`; `cargo test --workspace`. All partial-export tests run without worker processes or re-solving.

```powershell
git add crates/core-replay
git commit -m "feat(core-replay): walk covered snapshot paths and translate later wagers" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 13: Assemble branch-supported strategy, EV and unresolved mass

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

Range mix uses the node-covered mass, independently of actual hero combo:

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

### Task 14: Integrate the engine's chart and EV-bearing preflop decision path

**Files:** Modify `crates/engine/{Cargo.toml,src/lib.rs,src/assemble.rs}`; create `crates/engine/src/preflop.rs`, `crates/engine/tests/preflop_replay.rs`; modify `Cargo.lock`.

**Interfaces:**
- Consumes: Plan 2 `Engine::new(GameConfig,Paths)`, `recommend(sink)->DecisionIdentity`, existing Fast/Final emitter and coverage assembler; `replay(ReplayInput)->ReplayOutput`, `PreflopStore::query`, `mix_nodes`.
- Produces: `Engine::preflop_store(&self)->&PreflopStore`; `preflop_final(base:Recommendation, state:&HandState, store:&PreflopStore, snapshots:&[StreetSnapshot])->Recommendation`; `set_headline(actions:&mut [ActionAdvice], unresolved_mass:f32, source:SourceKind, unverified:bool)->Option<&'static str>`.

- [ ] **Step 1: Write failing engine preflop coverage tests.** Use the existing Plan 2 engine test constructor, sink, fake clock and worker injection surface; inject a worker that records all sends and verify zero `solve` messages on every preflop path. Cases in §6:

| Source/node state | Final expectation |
|---|---|
| Verified synthetic complete node | EVs normalized per Task 6; Exact only with no mapping/replay reasons. |
| Synthetic unverified reference | Frequencies, no EVs, `EvReferenceUnverified`; matching frequency headline text. |
| Chart node | Frequencies, no EVs, `ChartRounded`; highest-frequency chart headline. |
| Node only in some positive hero branches | Known frequencies, unresolved share, BranchResidual, no headline, no EV. |
| Node in no positive hero branches | `Unsupported{MissingPreflopNode}`, inherited reasons in partial. |
| Zero hero combo but positive public range | HeroComboOutOfSupport, range_mix present, no combo frequencies/EV. |

```rust
#[test]
fn chart_headline_requires_complete_frequency_support(){
    use proto::{Action,ActionAdvice,Unavailable};
    let mut actions=vec![
        ActionAdvice{action:Action::Fold,frequency:Some(0.2),ev_bb:None,
            unavailable:Some(Unavailable::ChartNoEv),headline:false},
        ActionAdvice{action:Action::Raise{to:5},frequency:Some(0.8),ev_bb:None,
            unavailable:Some(Unavailable::ChartNoEv),headline:false},
    ];
    assert_eq!(engine::preflop::set_headline(&mut actions,0.,core_preflop::SourceKind::ChartTranscription,false),
        Some("highest-frequency chart action"));
    assert!(actions[1].headline);
    assert_eq!(engine::preflop::set_headline(&mut actions,0.05,core_preflop::SourceKind::ChartTranscription,false),None);
    assert!(actions.iter().all(|a|!a.headline));
}
```

- [ ] **Step 2: Run red.** `cargo test -p engine --test preflop_replay`.

- [ ] **Step 3: Implement the existing preflop dispatch branch.** At session startup load installed sources and packaged chart fixtures using the same validated loader; load once into memory, surface quarantine banners, and retain the remaining sources. Do not load synthetic fixtures in production. Existing paths/config plumbing from Plan 2 owns directories; add a packaged-chart fallback directory to its existing Paths initializer if not already present. `Engine::preflop_store` exposes the already loaded store for Plan 4. Because this source is accessed by the fast-path thread, use safe `Send + Sync` adapter storage (add those supertraits to `PreflopSource`; method signatures stay unchanged) and share by the engine's existing ownership mechanism.

On a preflop decision after Plan 2's validation/NoDecision/identity gate, emit Fast with legal intervals and pending equity, then replay all observed preflop actions. For each positive branch query hero's mapped current node, run legality mapping, assemble through `mix_nodes`, merge source/replay/mapping reasons, and emit Final under the same active identity. `Unsupported` retains partial reasons; equity can arrive afterwards through the existing Equity event path. NoDecision remains Plan 2's gate, including another actor, unknown cards, folded/all-in hero, fewer than two legal actions, completed hand.

The adapter is a modification of the existing branch dispatch, with the new function receiving the already assembled Fast base:

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

- [ ] **Step 4: Implement headline rules in `crates/engine/src/assemble.rs`.** Re-export the function from the preflop module with `pub use crate::assemble::set_headline;` so the public test path above resolves without a duplicate implementation.

```rust
pub fn set_headline(a:&mut [ActionAdvice],unresolved:f32,source:SourceKind,unverified:bool)
    ->Option<&'static str>{
    for x in a.iter_mut(){x.headline=false;}
    if unresolved>0.0 || a.is_empty(){return None;}
    let ev_complete=a.iter().all(|x|x.ev_bb.is_some());
    let freq_complete=a.iter().all(|x|x.frequency.is_some());
    let label=if ev_complete{"highest EV"}
        else if !freq_complete{return None;}
        else if a.iter().any(|x|matches!(&x.unavailable,Some(Unavailable::BranchSupportIncomplete{..}))){
            "highest-frequency action, EV incomplete"
        }else if matches!(source,SourceKind::ChartTranscription){"highest-frequency chart action"}
        else if unverified{"highest-frequency source action, EV reference unverified"}
        else{return None;};
    let best=(0..a.len()).max_by(|&i,&j|{
        let primary=if ev_complete{a[i].ev_bb.unwrap().total_cmp(&a[j].ev_bb.unwrap())}
            else{a[i].frequency.unwrap().total_cmp(&a[j].frequency.unwrap())};
        primary.then_with(||a[i].frequency.unwrap_or(0.).total_cmp(&a[j].frequency.unwrap_or(0.)))
            .then_with(||j.cmp(&i))
    }).unwrap();
    a[best].headline=true;Some(label)
}
```

Do not rank a partially EV-bearing menu by its known EVs. Display labels through Plan 2's existing note/headline representation; §4.4 does not define a new Recommendation headline string field, so do not add one to `proto`. Notes include `"x% of the posterior has no strategy"` whenever unresolved>0. Do not set chart fold EV=0 to evade the no-EV rule.

- [ ] **Step 5: Verify identities, source precedence and no-EV release behavior.** Run synthetic source tests without acquiring PokerData; load real chart fixtures for a legal RFI decision at each depth and assert Final frequencies sum to 1, legal wager sizes, ChartRounded and no EV. Source precedence chooses synthetic PokerData only when explicitly injected in tests. Check previous source/replay reasons survive current exact lookup; duplicated displays may be deduplicated by serialized reason identity without losing distinct missing paths. Run `cargo test -p engine --test preflop_replay`, existing `recommendation_assembly_golden`, `cargo test --workspace`.

- [ ] **Step 6: Commit.**

```powershell
git add Cargo.lock crates/engine crates/core-preflop/src/envelope.rs
git commit -m "feat(engine): deliver chart-backed preflop recommendations" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 15: Feed replayed root ranges into turn and river solves and register snapshots

**Files:** Create `crates/engine/src/replay_bridge.rs`; modify `crates/engine/src/{lib.rs,postflop.rs}`, `crates/engine/tests/preflop_replay.rs`.

**Interfaces:**
- Consumes: `street_root`, `replay_root`, `replay(ReplayInput)`, Plan 2 `build_effective_tree`, worker result validation/materialized resolution, existing Final/Provisional event path.
- Produces: exact engine `register_snapshot(&DecisionIdentity,StreetSnapshot)`; `prepare_root(state:&HandState,store:&PreflopStore,snapshots:&[StreetSnapshot],tree:EffectiveTree,target_bp:u16)->Result<(SolveInput,ReplayOutput),UnsupportedReason>`; `snapshot_from_solution(identity:&DecisionIdentity,input:&SolveInput,solution:&StreetSolution,ordinal_paths:Vec<OrdinalPath>,signature:String,origin:&str,reasons:Vec<ApproxReason>)->StreetSnapshot`.

- [ ] **Step 1: Add red root-range integration tests.** A fake worker captures Solve requests for a turn and a river after synthetic preflop replay and compatible prior-street snapshots. Compare full oop/ip arrays to direct replay; worker pot/stacks equal the financial StreetRootSnapshot, not decision-point Derived. Changing actual hero cards changes only equity/hero advice, never root ranges, hashes, q or SolveInput. Current-street Bet73 is inserted exactly by Plan 2, never translated here. A translated flop Bet73 contributes `BetTranslation` to a river Final with every other inherited reason still present.

Register a live `ok`, live `best_so_far`, and synthetic cache-hit Final through the **same** method, then append Call+next board and assert replay conditions from the registered nodes. A Provisional then Final of one decision leaves the Final snapshot; stale identities never register. These cache-origin tests construct an already validated StreetSnapshot; cache lookup/storage implementation is Plan 4.

- [ ] **Step 2: Run red.** `cargo test -p engine --test preflop_replay replay_feeds_street_root_solves`.

- [ ] **Step 3: Replace the Plan 2 root-range provider.** Existing financial root derivation and effective tree remain authoritative. Use replay once, hand its public root ranges to the worker, and separate hero-conditioned copies for equity:

```rust
pub fn prepare_root(state:&HandState,store:&PreflopStore,snapshots:&[StreetSnapshot],
    tree:EffectiveTree,target_bp:u16)->Result<(SolveInput,ReplayOutput),UnsupportedReason>{
    let root=core_model::street_root(state).map_err(|e|match e{
        core_model::RootError::ProjectionNotReproducing{step}=>UnsupportedReason::UnsupportedHistory{
            reason:format!("multiway street root not reproducible at step {step}")},
        core_model::RootError::Multiway=>UnsupportedReason::MultiwayEv{
            pot_eligible:state.dealt.iter().filter(|seat|!state.derived.folded[seat.0 as usize]).count() as u8},
        core_model::RootError::NoDecision=>UnsupportedReason::UnsupportedHistory{
            reason:"no street-root decision".into()},
    })?;
    let replayed=core_replay::replay(ReplayInput{cfg:&state.config,state,store,snapshots});
    if let Some(reason)=&replayed.unsupported{return Err(reason.clone());}
    let oop=replayed.ranges[root.oop.0 as usize].clone().ok_or(UnsupportedReason::InvalidRanges)?;
    let ip=replayed.ranges[root.ip.0 as usize].clone().ok_or(UnsupportedReason::InvalidRanges)?;
    Ok((SolveInput{root,ranges:[oop,ip],tree,target_bp},replayed))
}
pub fn opposing_equity_ranges(replayed:&ReplayOutput,state:&HandState)->Vec<(Seat,Range1326)>{
    let Some(hero)=state.hero_cards else{return vec![]};
    state.dealt.iter().copied().filter(|&s|s!=state.hero && !state.derived.folded[s.0 as usize])
        .filter_map(|seat|replayed.ranges[seat.0 as usize].as_ref()
            .map(|r|(seat,core_ranges::hero_conditioned(r,hero)))).collect()
}
```

Reuse Plan 2's coverage classification of root errors in production to avoid a duplicate classification path; the helper above shows the typed error mapping and input composition. Count only dealt seats for pot eligibility; if Plan 1's folded vector stores vacant=false, use `state.dealt` filtering. `RootError::Multiway` is decided before numeric solve. Plan 2's exact projection assertions, analytic facing-all-in fallback, legal-action validator, deadlines and watchdog remain in the existing solve path. The analytic fallback now consumes these replayed prior-street ranges, then hero-conditioned copies, with `UnconditionedCurrentStreet` still added.

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

Only call after `proto::worker::validate_solution` and Plan 2's chip→ordinal path resolution against **that solution's source materialized tree**, requested node/actor validation, active identity and expiry check. `covered_paths[i]` matches nodes[i] after resolution; every unresolvable path is `EngineError{message:"invalid solution",retryable:false}` and neither delivered nor registered. Store actual caller-suit/chip nodes, not un-inverted cache payloads. Snapshot key hashes are the exact public input ranges, never current-node/hero-conditioned ranges.

- [ ] **Step 5: Add single registration and invalidation hooks.** Engine `register_snapshot` checks current identity equality, enforces validated Final/Provisional phase ordering through existing decision state, then calls `SnapshotStore::register`. It is invoked by the shared accepted-result delivery function for live ok/best_so_far and by Plan 4's cache delivery using the same method. Register before accepted delivery becomes visible to a subsequent request; late rejected results never enter snapshots. Wire `apply_action`, `set_board`, `undo`, `begin_hand`, `finish_hand`, `abandon_hand`, model/config changes to the registry's prefix invalidation rules after their newly assigned revision. A config change mid-hand keeps frozen HandConfig and its old config_revision, so it does not retroactively reclassify an active hand.

When snapshot selection fails, append concrete `UnconditionedPriorStreet` causes and continue with declared ranges. Store cause diagnostics in the existing engine decision provenance, including no request, multiway street, engine failure, and deadline; no missing-source failure is silently upgraded to Exact by a later solve. Full DecisionIdentity is only a registration gate; valid prefix snapshots retain their original solve revision and are selectable under later revisions.

- [ ] **Step 6: Verify and commit.** Run `cargo test -p engine --test preflop_replay`, Plan 2 `identity_race_golden`, `facing_allin_golden`, `final_delivery_independent_of_worker`, then `cargo test --workspace`. Fake-clock tests assert replay is inside existing admission-to-deadline accounting and no synchronous disk load is added to recommendation dispatch. Measure release in-memory lookup/replay against .05/.15 s targets on real chart paths; report measured durations, not brittle CI wall-clock assertions.

```powershell
git add crates/engine
git commit -m "feat(engine): replay public root ranges and register accepted snapshots" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

### Task 16: Freeze replay and bet-translation goldens and audit every scoped test

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

Use only the final correct board set `{46,21,0}` in the committed generator. Assert `combo_class` expands AA=6, AKs=4, AKo=12; expected vectors are f32-converted exactly once with `struct.pack/unpack` before JSON write, while q/logs remain doubles. Mark SB folded but retain its posterior vector and folded_ranges. Goldens compare every element with absolute 1e-6 and logs/q with 1e-10. Add a second three-seat line containing an off-menu wager and compare q-weighted branch posteriors using the independently implemented §8.4 kernel, so the engine golden exercises actual translation as well as uniform starts.

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
| core-preflop | `pokerdata_schema_mapping` 1–2/6; `bundle_validation_quarantine` 2/6; `pokerdata_action_path_lookup` 5; `depth_bucket_labels_per_prefix` 5; `straddle_mapping_labels` 5; `rake_profile_ordering` 5; `pokerdata_units_source_scaling` 6. |
| core-replay | `replay_bayes_two_combos` 8; `replay_off_tree_pseudo_harmonic` 8; `replay_cross_actor_branches` 8/12; `replay_branch_cap_residual` 9; `replay_missing_continuation` 10/12; `replay_snapshot_prefix_reuse` 12; `replay_snapshot_compatibility` 11/15; `replay_incomplete_branch_ev` 13; `replay_hero_out_of_support` 13. |
| engine | `replay_weights_golden`, `bet_translation_golden` 16; extend existing `recommendation_assembly_golden` for charts/unverified/T7 rather than invent another spec name; retain Plan 2 identity/deadline/analytic goldens. |

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
| §3.2, §3.5 dependency direction and APIs | 1–2, 8, 10, 14–15 | Exact public function/type names retained; branch definitions below replay avoid a cycle. Plan 1/2 APIs are consumed, no replacement engine/model/worker. |
| §4.1 class/combination ordering | 1, 3–4, 6, 16 | Explicit action-major envelope to combo-major expansion, independent 6/4/12 oracle. |
| §4.4 availability, headline, unresolved mass | 7, 13–14, 16 | No partial-EV ranking, no headline with unresolved>0, range_mix under HeroComboOutOfSupport. |
| §5 step 1 source loading; step 6 preflop | 2, 4, 14 | Chart baseline, synthetic EV testing; conditional V9 acquisition/converter excluded. |
| §5 step 7 turn/river root ranges and registration | 10–12, 15 | Completed streets only, public root ranges, exact current-street insertion stays in Plan 2. |
| §6 preflop rows and inherited postflop reasons | 5–7, 13–15 | Missing current node differs from historical missing strategy; reasons retained in Unsupported.partial. |
| §8.1 trait/store/query | 1–2, 5–6 | Both adapters share the envelope; future verified EV data loads through the same path. |
| §8.2 chart acquisition/transcription | 3–4 | URLs fixed, source hashes and actual PDF node inventory frozen by executor, 169 cells reread per grid, all EVs omitted. |
| §8.2 schema/hash/bounds/quarantine | 1–2, 6 | Both hash and semantic checks, sparse EV semantics, all-zero declaration, 64 MiB, independent bundle failure. |
| §8.3 prefixes/depth/asymmetry/rake/straddle/short-handed/sizes | 5–7, 10 | Exact formulas, six virtual roles, no hindsight; unknown chart rake is disclosed rather than fabricated. |
| §8.3 EV reference variants | 6 | All four variants, fold cross-check, no-fold BB node, re-raise, live SB independence and straddle S scaling. |
| §8.4 interpolation/branch kernel/cap | 7–9 | Exact f and M formulas, cross-actor q, common rescale, one persistent residual and deterministic ties. |
| §8.4 branch EV and legal moves | 7, 13–14 | Known frequencies, unresolved share, no EV over missing support, own destination EV only. |
| §9.1 public replay and snapshot types | 8, 10–11 | Exact fields, model-scoped snapshot slice supplied by engine. |
| §9.2 start, walk, reuse, zero support, log reach | 8, 10–12, 15–16 | Uniform public start, per-action transaction, f64 internal, positive output guard, hero copies only after replay. |
| §9.3 missing strategies and frozen branches | 10, 12–13 | Preflop freeze until next street; uncovered known postflop paths continue; missing path fallback disclosed. |
| §12 errors/lifecycle | 2, 10–15 | Quarantine, zero support, malformed path, stale identity and mutation invalidation covered. |
| §13.1 all core-preflop/core-replay rows | 1–2, 5–13, 15–16 | Exact names and supplied numeric targets listed; T3 wording conflict below. |
| §13.3 two requested engine goldens | 16 | Full vectors/log reach and all interpolation/legal-move cases; independent Python oracle. |
| §13.5 chart baseline handoff | 4, 16 | Freeze actual nodes and missing-node fallbacks before Plan 4 generates benchmark/E2E inputs; no bench claim made here. |

**Placeholder scan:** Completed against the saved plan: zero matches for the skill's banned deferred-code markers and ellipsis substitutions. The structural audit found 16 tasks, each with Files, Interfaces, checkbox steps, one commit trailer and a `cargo test --workspace` gate; code fences are balanced. Code blocks contain concrete types, algorithms, inputs and assertions; transcription rows are deliberately entered from the fetched source rather than invented in this planning document. Full chart source/data content is an executor deliverable, not pre-existing evidence. Plans 1/2 source has not yet been generated in this workspace, so engine integration consumes their specified surface and identifies the exact functions/modules to extend without asserting nonexistent line numbers.

**Type-consistency check:** Money stays u32 chips/cap_mchips, internal branch values f64, envelope probabilities f32, optional EVs remain optional. Envelope is action-major `[actions][169]`; PreflopNode is class-major `[169][actions]`; ExpandedNode/NodeStrategy are combo-major `[1326][actions]`. Snapshot covered paths are ordinal; NodeStrategy wire paths remain chip paths. `SeatMass` and `HistoryBranch` have one definition with core-replay re-exports. `AsymmetricStacks` uses the §4.4 `stacks_bb` field; prominence is computed separately. `ReplayInput` cannot carry active model_revision, so engine filters snapshots by identity before calling it. Source `ev_reference` variants are serialized with the exact lowercase strings; no new wire enums or recommendation fields are assumed.

**Spec-coverage gaps / conflicts that cannot be honestly marked resolved:**

1. **T3 wording:** §13.1 says posterior `(0.9,0.1)` “for every seat including hero” with a single initial branch and only one acting seat. §8.4 instead gives a one-element branch posterior `[1]` for every supported combo, the actor's normalized combo distribution `(0.9,0.1)`, and an unacted seat's uniform combo distribution. Task 8 follows the binding formulas and retains M=.5/.4/.5, q=.1, pre-final-rescale masses=(9,1), cumulative marginal=.18/.02 and log=ln(.18). Both literal assertions cannot pass together.
2. **Board blocking versus equal branch totals:** §9.2 requires pointwise board removal at each root; §8.4 asserts equal total masses for a seat across every branch and a seat-independent residual share. Different branch card distributions generally lose different mass to the board, breaking that invariant. No board-evidence/rebalancing rule is specified that preserves every public marginal and all seats' branch correlations. Task 10 preserves literal public board blocking and does not invent one; the equal-total/residual-share guarantee across such a root needs a spec decision before it can be certified.
3. **Uncovered off-menu continuation:** §9.2 case 3 says all next ordinals remain defined because every action is in the skeleton, while case 2 and the root-only Bet73 fixture explicitly allow a later wager outside that skeleton. If the parent strategy is also unexported, neither likelihood nor a unique child path exists. Task 12 retains conditioned masses, freezes navigation for that branch/street and discloses the uncovered path, consistent with the no-guessed-likelihood fallback; automatic resumption beyond that unknown child needs an explicit spec rule.
4. **Chart inventory verification:** PokerCoaching's extracted page headings and RangeConverter's article were checked at planning time, but RangeConverter's PDF download timed out and the sixth PokerCoaching page has no extracted text. Task 4 explicitly requires executor visual inspection, actual node/size coverage, hashes, and cell verification. The completed inventory is not yet verified; no missing response is assumed present. Rake metadata absent from a source is disclosed, not assigned the PokerData rake profile.

Out-of-scope items are deliberate series boundaries rather than omissions: `.7z`/authorized PokerData conversion and sample/rights acquisition remain V9; cache, flop policy, pre-solver, bench/E2E hands and release gate are Plan 4; Tauri/UI are Plan 5; exploit work is deferred. This plan itself has not executed Rust/Python tests or transcribed chart data.
