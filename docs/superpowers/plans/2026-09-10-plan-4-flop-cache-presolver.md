# Plan 4: Flop path, cache and pre-solver Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the flop decision path, pot-normalized flop/turn cache, resumable idle pre-solver, and reproducible flop/e2e/fault baseline gates.

**Architecture:** `cache` owns normalized entries, candidate selection, bounded storage, and a scheduler whose executor is supplied by `engine`. The engine supplies chart replay, materialization, worker admission, identity checks, snapshot registration, and independent final delivery; cached solutions enter the same validation and assembly path as live results. `bench` measures exact production templates and exercises the engine through Plan 2's `WorkerLink` seam without linking the solver.

**Tech Stack:** Rust edition 2021, stable Rust 1.95 or newer, MSVC pinned by Plan 1's `rust-toolchain.toml`, std threads/channels, serde, thiserror, SHA-256, bincode + zstd; Python 3.12, PokerKit 0.7.5, pytest.

**Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 5, §§2, 3.2–3.5, 4.2–4.6, 5 step 7, 7, 9, 10.1–10.6, 12, 13.1/13.3/13.5, 14.4 V3/V21/V22. Approval: `docs/design/2026-09-10-design-outline.md` §0b and amended §6. Measurements: `docs/research/R8-solver-bench.md`, especially A.2/A.5/A.7. Libraries: `docs/research/R3-libraries.md`.

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
- Path `%LOCALAPPDATA%\PokerAI\cache\v3\<key[0..2]>\<key>.bin`; bincode + zstd; payload sha256/schema/proto header; 64 MiB compressed, 256 MiB decoded; two entries per key cell. Default `cache_quota_bytes = 10 GiB`; oldest `last_hit` eviction. Read/decode error: miss, best-effort delete. Write failure preserves recommendation.
- Solver preferences: threads 16, target 50 bp, `flop_budget_s = 10`, range `1..=30`. Outline §0b confirms default 10. From monotonic admission `t0`: first attempt river 2 s, turn 6 s, flop `flop_budget_s`; final river/turn 15 s, flop `5 s + flop_budget_s`. Worker gets remaining minus 100 ms delivery and 50 ms pipe margins; extraction river/turn 200 ms, flop 600 ms. Watchdog emits at final deadline minus 100 ms.
- Fast <= 0.3 s; validation/replay <= 0.15 s; cache lookup <= 0.5 s; equity owns 0.5 s after Fast. Cancel kill threshold 1.5 s; solving heartbeat 5 s. Suspend/resume expires requests. I/O and kill/restart cannot block final delivery.
- Worker memory: f32 estimate <= 2 GiB selects f32; else i16 estimate <= 8 GiB selects i16; else `TreeTooLarge`. Also `estimate * 1.25 <= memory_limit_bytes`, default 10 GiB; process job-object limit 16 GiB. Cache `mode` records solver storage, not extra lossy quantization.
- `flop_fast_v1`: bets 0.5, raises 2.5x, add-all-in 1.0, force-all-in 0.15, cap 3. `flop_min_v1`: bets 0.75, all-in-only raises, add-all-in 1.5, force-all-in 0.15, cap 1. Merging 0; explicit empty turn/river donks. Template change requires version bump.
- SRP lookup tries pre-solver `flop_fast_v1`, then a distinct admitted live template. Live SRP uses `flop_min_v1` iff V3 p95 <= 10 s at both 100bb and 200bb; otherwise `flop_fast_v1` to deadline. Three or more preflop wagers use `flop_fast_v1`. Classify from history, never ranges.
- Pre-solver: no active hand and no request for 30 s; background true, deadline 600000 ms, target 50, `flop_fast_v1`; three retries with 30 s backoff. Live admission cancels background. Persist pending/done/failed{n} and cursor in `queue.json`; done only while a valid at-target entry exists.
- Every validated solution reaching Final/Provisional uses `register_snapshot`; same-decision Final replaces Provisional. Baseline model revision 0. River caching, experimental caching, PokerData acquisition/conversion and exploit slice are outside this plan.
- Each task has red/green checks and one commit. `cargo test --workspace` passes before each commit. Trailer: `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

---

## Interface ownership and execution order

Execute after Plans 1–3. Consume the spec's public surfaces without renaming:

```rust
canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm);
hash_scaled(range: &Range1326) -> [u8; 32];
block_public(range: &mut Range1326, board: &[Card]);
build_effective_tree(root: &StreetRootSnapshot, selection: &TemplateSelection)
    -> Result<EffectiveTree, UnsupportedReason>;
replay(input: ReplayInput<'_>) -> ReplayOutput;
PreflopStore::query(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize)
    -> PreflopAnswer;
register_snapshot(identity: &DecisionIdentity, snapshot: StreetSnapshot);
```

`proto::worker::validate_solution`, `WorkerLink`, the clock seam and engine admission/assembly belong to Plans 1–2. Their concrete argument wrappers come from those definitions: the spec does not supply complete Rust declarations. Do not create substitute validators or a second WorkerLink. Each integration task defines its new helper signatures and maps the existing boundary explicitly. Module filenames for extensions below are owned by this plan; import prerequisite items from their actual modules.

Plan 2's available interface table specifies `validate_solution(sol: &StreetSolution, materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>`; use that exact call below. Its existing modules are `engine/src/deadline.rs`, `engine/src/log.rs`, `engine/src/engine.rs`, `engine/src/core.rs`, and `engine/src/testing.rs`. Extend them rather than adding duplicate deadline/log/config implementations. Plan 3 provides `PreflopStore::open(dir: &Path) -> (Self, Vec<String>)` and sibling chart manifests `<name>.manifest.json`.

Tagged proto enums cannot be deserialized directly by bincode. Task 5 uses a binary DTO with JSON metadata for tagged proto values and binary numeric matrices, preserving the normative JSON schema.

Verified 2026-09-10: [bincode 1.3.3](https://docs.rs/bincode/1.3.3/bincode/), [zstd 0.13.3](https://docs.rs/zstd/0.13.3/zstd/), [sha2 0.10.9](https://docs.rs/sha2/0.10.9/sha2/), [thiserror 2.0.17](https://docs.rs/thiserror/2.0.17/thiserror/). Cache bincode 1.3.3 coexists with solver bincode 2.0.0-rc.3. Inherit serde/serde_json/thiserror from Plan 1.

## File structure

Generated paths expand to the inventories in Tasks 16–17. Modify existing modules at named symbols, not speculative line numbers.

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
| `crates/engine/src/core.rs`, `crates/engine/src/testing.rs`, `crates/bench/Cargo.toml` | Extend request/test seams and enable the engine testing feature for bench |
| `solver-worker/Cargo.toml`, `solver-worker/src/main.rs`, `solver-worker/src/memory.rs`, `solver-worker/tests/bench_mode.rs` | Opt-in diagnostic storage-mode measurements; unchanged production defaults |
| `crates/engine/src/cache_bridge.rs` | Canonical query/store and bounded lookup |
| `crates/engine/src/flop.rs` | Template and hit/provisional/live transitions |
| `crates/engine/src/deadline.rs`, `crates/engine/src/engine.rs` | Budget/deadline extensions |
| `crates/engine/src/snapshots.rs` | Existing single registration path |
| `crates/engine/src/presolve.rs` | Chart replay and background executor |
| `crates/engine/src/log.rs` | Scenario hit rates and violations |
| `crates/engine/src/bench_support.rs` | Facade keeping bench dependencies downward |
| `crates/engine/tests/support/mod.rs` | WorkerLink/fake-clock harness extensions |
| `crates/engine/tests/cache_key_structural_identity.rs` | T4 with production materializer |
| `crates/engine/tests/flop_path_golden.rs` | Hits, provisional, budgets and identity |
| `crates/engine/tests/cache_snapshot_replay.rs` | Cached prior-street translation |
| `crates/engine/tests/presolver_engine.rs` | Replay and live cancellation |
| `crates/engine/tests/golden/{cache_scale,flop_path,cache_snapshot_replay}.json` | Reviewed expected values |
| `crates/bench/src/main.rs`, `crates/bench/src/gen_spots.rs`, `crates/bench/src/flop.rs` | CLI, replay spots, V3 matrix |
| `crates/bench/src/lib.rs` | Re-export bench modules for integration tests and share them with main |
| `crates/bench/src/e2e.rs`, `crates/bench/src/fault.rs`, `crates/bench/src/faulty_worker.rs` | Recorded inputs and WorkerLink faults |
| `crates/bench/src/report.rs`, `crates/bench/src/gate.rs` | Report and executable baseline gate |
| `crates/bench/tests/{flop,e2e,fault,gate}.rs` | Bench contracts |
| `tools/gen_fixtures.py`, `tools/e2e_hands.py` | PokerKit generator extensions |
| `tools/tests/test_e2e_hands.py` | Inventory, legality and expectations |
| `fixtures/hands/e2e/001.json` through `fixtures/hands/e2e/050.json`, `fixtures/hands/e2e/manifest.json` | Versioned inputs and hashes |
| `bench/spots/{flop_fast,flop_min,sources}.json` | Six lines × three boards and source lock |
| `docs/bench/2026-09-10-i7-13700K.md` | Measured V3/V21/V22 report |

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
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
[dependencies]
proto = { path = "../proto" }
core-iso = { path = "../core-iso" }
core-ranges = { path = "../core-ranges" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
sha2 = "=0.10.9"
bincode = "=1.3.3"
zstd = "=0.13.3"
```

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

**Interfaces:** Consumes `StreetSolution`, `NodeStrategy`, `EffectiveTree`, `MaterializedNode`, `validate_solution`; Produces `CacheEntry`, `CachedNode`, `SourceInputs`, `resolve_path(&[MaterializedNode],&[Action])->Option<OrdinalPath>`, `chip_path(&[MaterializedNode],&[u8])->Option<ChipPath>`, `normalize(&StreetSolution,&EffectiveTree,u32)->Result<Vec<CachedNode>,CacheError>`, `validate_entry(&CacheEntry)->Result<(),CacheError>`.

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
pub fn resolve_path(t:&[proto::MaterializedNode],p:&[proto::Action])->Option<proto::OrdinalPath> {
    let mut out=Vec::new();
    for action in p {
        let n=t.iter().find(|n|n.path==out)?;
        out.push(u8::try_from(n.actions.iter().position(|a|a==action)?).ok()?);
    }
    t.iter().any(|n|n.path==out).then_some(out)
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
    let menus=[Street::Flop,Street::Turn,Street::River].into_iter().map(|street|
        (street,PlayerMenus{oop:side.clone(),ip:side.clone(),donk:Some(vec![])})).collect();
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

Board blocking, canonical identity and fraction checks precede numeric matrix reconstruction. No validator needs a later-task fixture. Source key digests are checked against the filename/cell by the reader; production structural signatures are supplied by the existing engine signature builder and verified again against each query before serving.

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
    if e.tree.materialized.len()!=t.materialized.len() {return None;}
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

Sort full materialized lists by ordinal path once during validation; reject duplicate paths. Compare all streets, not just exported nodes or root. A min-raise/all-in/deduplication/wager-cap boundary changing kinds, paths, actors, streets or terminal classification must miss. Cap activation is checked even when topology is identical. Use source and query absolute caps after normalized rake-key equality.

- [ ] **Step 4 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 5 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): enforce full-tree SPR menu and rake predicates' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 4: Preserve inherited reasons and filter raw accuracy

**Files:** Create `crates/cache/src/label.rs`; modify `crates/cache/src/lib.rs`, `crates/cache/tests/lookup.rs`.

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

    (if reasons.is_empty()&&!provisional {proto::Coverage::Exact}
        else {proto::Coverage::Approximate{reasons}},provisional)
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

Metadata contains exact proto values, source ranges, tree and reasons; numerical node matrices are stored by bincode, not strings. Reject cells whose entries have different key digests. Serialization is bounded too: wrap JSON/bincode writers with a capped writer returning `InvalidData` above 256 MiB rather than allowing an unbounded oversized metadata allocation.

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

**Interfaces:** Consumes `CacheEntry`, `encode(&Cell)` and `read_cell(&Path)` from Tasks 2/5. Produces `retain_two(Vec<CacheEntry>)->Vec<CacheEntry>`, `IndexRow{key:[u8;32],bytes:u64,last_hit:u64}`, `victims(&[IndexRow],u64)->Vec<[u8;32]>`, `write_atomic(&Path,&[u8])->Result<(),CacheError>`, `Cache::open(PathBuf,u64)->Cache`, `Cache::store(&CacheEntry)`; store is best-effort/nonblocking. Internal `StoreReceipt` allows the pre-solver to await durable completion away from live paths.

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

Use `std::thread::Builder::new().name("cache-writer".into())` and `sync_channel(8)` carrying `Store(Arc<CacheEntry>, optional receipt)`, `Touch(key,payload_digest,last_hit)`, `Delete(key)`, `Shutdown`. `Cache::store` uses `try_send`; a full/disconnected queue is a skipped write logged once per session. Writer validates, reads existing cell, applies replacement, encodes, writes, updates last-hit index, then evicts. Index is rebuilt by scanning bounded valid `.bin` cells on open; it is not a second authoritative file. Temporary siblings are ignored on reads and cleaned by the writer after restart. No mutex spans file I/O. Cache open failure yields a disabled Cache whose lookup is Miss.

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

**Files:** Modify `crates/cache/src/{lib,lookup,label,entry}.rs`, `crates/cache/tests/lookup.rs`; create `crates/engine/src/cache_bridge.rs`; modify `crates/engine/Cargo.toml`, `crates/engine/src/lib.rs`.

**Interfaces:** Consumes `CacheEntry`, `compare`, `label`, `read_cell`, `validate_solution` and `Derived`. Produces `CacheQuery`, `CacheHit`, `Lookup`, `Cache::lookup(&CacheQuery)->Lookup`, `legal_menu(&[Action],&[LegalAction])->bool`. Engine extension `make_cache_query(input:&SolveInput,derived:&Derived,bb_chips:u32,rake:&Rake,reasons:&[ApproxReason],budget:Duration)->Result<CacheQuery,UnsupportedReason>` supplies query actor from OOP/IP seat mapping and `Derived.to_act`.

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

- [ ] **Step 4 (5 min): Implement selection and reconstruction.** For each of three bucket keys read at most one cell/two entries, validate source before comparing, and require non-SPR key fields equal by comparing `at_bucket(0)`. Exclude candidates missing requested covered ordinal path or with the wrong actor; this is a node-only miss, not deletion. Rank survivors with `Comparison::rank`, then raw exploitability, then payload digest. Accuracy above target remains a survivor and becomes Provisional after ranking; do not reorder to prefer an at-target but more distant candidate.

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
```

Use this copier for signed EV rows and probability rows; copy availability booleans through the same source/destination indices. Never run signed EVs through range validation. For every exported node: query materialized actions and chip path at the identical ordinal positions, inverse suits on combo rows, EV times `P_query`. Select requested index by ordinal path and verify actor. Build full `StreetSolution` with query chip paths, source iterations/memory/mode/export/locks, and raw exploitability times query P. Run shared `validate_solution` against query materialization, then `legal_menu` for the requested node. Failure is Miss; invalid source files delete, ordinary query mismatch does not. Include realized menu and source mode in notes.

- [ ] **Step 5 (5 min): Bound lookup I/O and wire canonical query construction.** One fixed cache-reader thread uses a bounded queue; `lookup` posts without blocking and waits at most `min(query.budget,500 ms)` using `recv_timeout`. If already blocked/full, immediately return Miss. Results carry a request token; timed-out replies cannot contaminate later lookups. No new reader is spawned per timeout. Engine invokes lookup on fast-path work, never watchdog/engine-main, and passes remaining absolute budget. Async deletion/touch goes to cache-writer. No read or writer join occurs on the delivery path.

```rust
pub fn lookup_wait<T>(rx:&std::sync::mpsc::Receiver<T>,budget:std::time::Duration)->Option<T> {
    rx.recv_timeout(budget.min(std::time::Duration::from_millis(500))).ok()
}
```

`make_cache_query`: board-block copies of both public ranges, canonicalize jointly, apply selected permutation, compute hashes; root P includes dead money, eff is min stack; get existing structural `tree_signature` from Plan 2, never hash raw materialized chips. Rake uses `rate.to_bits()`, reduced `cap_mchips : 1000*P`, collection-rule version; TimeCharge is unraked 0/0. Requested ordinal path resolves root history against the query tree. No hero, bb or requested path enters `KeyFields`.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache crates/engine Cargo.lock
git commit -m 'feat(cache): serve bounded validated query-tree node hits' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 8: Freeze the complete T4 structural-identity scale contract

**Files:** Create `crates/engine/tests/cache_key_structural_identity.rs`, `crates/engine/tests/golden/cache_scale.json`; modify `crates/engine/tests/support/mod.rs`, `crates/cache/tests/{entry,lookup,storage}.rs`.

**Interfaces:** Consumes production `build_effective_tree`, `Cache`, `CacheEntry`, `CacheQuery`, Plan 2's template registry. Produces test support `CacheRig::new(template:&str,p:u32,eff:u32,cap_mchips:u32)->CacheRig`, `CacheRig::query(p:u32,eff:u32,cap_mchips:u32,path:&[Action],actor:&str)->CacheQuery`, `CacheRig::hit(&CacheQuery)->CacheHit`, `CacheRig::miss(&CacheQuery)->bool`. Rig owns a temporary cache directory and `pub entry:CacheEntry`; it uses the real materializer and stores actor-distinct synthetic valid matrices, so no worker solve or timing noise enters T4.

- [ ] **Step 1 (3 min): Copy this §10.4 scale check verbatim into the T4 test's contract comment.**
- **Scale check** (contract, section 13.1 T4): pot/stack/cap 100/500/5000 mchips and 200/1000/10000 mchips with identical ranges, proportional quantum and identical fractional tree hit the same entry with identical frequencies and doubled chip EV, `Exact` (identical SPR rationals, identical realized fractions: every pot and stack doubles exactly, so `max(dev) = 0` at every node), at the root, at IP's node after Bet(50) versus Bet(100), and at OOP's node after Check, Bet(50) versus Check, Bet(100) (both actors at non-root nodes; a call closes the street, so the next decision lives in the next street's own root and is never requested from this entry, section 5); 100/500 versus 103/515 with a one-chip quantum has SPR `5 : 1` in both and rounds the root half-pot bet to 50 versus 52 chips (52/103 = 0.5049, a **root-action** deviation of 0.49%); the reported `MenuRounded{max_delta_pct}` is the maximum over the complete materialized list and is larger: after bet/call the turn pots are 200 and 207, the half-pot bets 100 and 104 (`104/103 - 100/100 = 0.0097`, 0.97%), and deeper nodes deviate further; T4 freezes the value computed from the two materialized lists (asserting `0.97 <= max_delta_pct < 5`), never the root number, and the candidate is `Approximate{MenuRounded}`, never `Exact`; 100/500 versus 100/508 (SPR 5.08, `delta = 1.6%`) is `SprBucketed` (and `MenuRounded` only if some realized size actually differs); 100/500 versus 100/511 (`delta = 2.2%`) misses; the `MenuRounded{2.0}` case is a specified pair of trees: test template `menu_round_test_v1` (single 0.33 bet, all-in-only raises, cap 1, add-all-in 1.5, force-all-in 0.15, flop root) at entry 100/500 versus query 20/100 (SPR `5 : 1` in both): the 0.33 bet is 33 chips (0.33) versus 7 chips (0.35) at every node with an unchanged pot (`dev = 0.02`), 55 versus 11 (0.55 in both) after bet/call, 91 versus 18 (0.91 versus 0.90) after bet/call/bet/call, the all-in raise-to amounts deviate by at most 0.02 (467 versus 93 after bet/call), and the river all-in is added in both trees after bet/call/bet/call (`412 <= 414`, `82 <= 84`) and in neither elsewhere, so the topologies agree and `max(dev) = 0.02`; a single-node comparison is never used, so a query whose root bet is exactly representable still carries `MenuRounded` when a deeper node rounds differently; a different canonical board always misses.

- [ ] **Step 2 (5 min): Write the red scale/actor test.** `CacheRig::new` constructs root board Kh7d2c with OOP Seat(2), IP Seat(0), equal effective stacks, dead 0 and empty history, parses board-only public AA/KK ranges, calls the production template/materializer, exports all current-street nodes, and assigns `available` only to board-compatible supported combos. For node index n, use uniform legal probabilities and `EV(a)=10*n+a`, except fold = 0; this makes wrong actor/path selection observable. `hit` unwraps any non-Miss lookup. Register only test templates `menu_round_test_v1` and `check_jam_test_v1` in the test harness, not the production selector.

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

- [ ] **Step 3 (2 min): Run red:** `cargo test -p engine --test cache_key_structural_identity`; absent rig or failed assertion.
- [ ] **Step 4 (5 min): Complete the rig, raw-fraction golden and all T4 mutation cases.** `menu_round_test_v1` uses 0.33 bets on each street, a-only raises, cap1, add1.5, force0.15, merge0, empty donks. Check/jam uses a-only bets, no ordinary raises, zero automatic thresholds; at P100 compare eff500 and504, cap55200. `check_only_test_v1` uses empty bet/raise/donk menus, zero thresholds and cap1 on all three streets to isolate SPR5.00/5.08/5.11 with identical realized menus. Under a wagering template, AllIn500 versus AllIn508 can have dev0.08 and correctly fail the separate0.05 menu filter even though SPR passes; test that rejection too. Never bypass menu validation on an SPR hit. Test query mutations separately; each starts from a valid copy and changes only the indicated field:

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

Run once with `POKERAI_RECORD_GOLDENS=1`, review, unset it, then normal green tests. No automatic golden acceptance in CI. Cache corruption mutations from Task5 now run with these real fixtures, completing `cache_payload_validated`.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --test cache_key_structural_identity`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache/tests crates/engine/tests
git commit -m 'test(cache): freeze complete T4 scale actor and topology cases' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 9: Implement flop budget and evidence-based template selection

**Files:** Create `crates/engine/src/flop.rs`; modify `crates/engine/src/{lib,engine,deadline}.rs`; create `crates/engine/tests/flop_path_golden.rs`.

**Interfaces:** Consumes `SolverPrefs`, `TakenAction`, existing absolute-deadline clock; Produces `FlopPolicy{min_admitted:bool}`, `FlopPolicy::live_template(u32)->&'static str`, `flop_deadlines(u64,u8)->Result<(u64,u64),UnsupportedReason>`. Extend existing `set_config(GameConfig)` and `Deadlines` without resetting t0.

- [ ] **Step 1 (4 min): Write `flop_budget_setting_golden`.**

```rust
#[test]
fn flop_budget_setting_golden() {
    use engine::flop::{flop_deadlines,FlopPolicy};
    assert_eq!(flop_deadlines(1000,10).unwrap(),(11000,16000));
    assert_eq!(flop_deadlines(1000,30).unwrap(),(31000,36000));
    assert!(flop_deadlines(0,0).is_err());assert!(flop_deadlines(0,31).is_err());
    assert_eq!(FlopPolicy{min_admitted:false}.live_template(2),"flop_fast_v1");
    assert_eq!(FlopPolicy{min_admitted:true}.live_template(2),"flop_min_v1");
    assert_eq!(FlopPolicy{min_admitted:true}.live_template(3),"flop_fast_v1");
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine flop_budget_setting_golden`.
- [ ] **Step 3 (5 min): Implement budget calculations and V3 admission.**

```rust
pub struct FlopPolicy {pub min_admitted:bool}
impl FlopPolicy {
    pub fn live_template(&self,preflop_wagers:u32)->&'static str {
        if preflop_wagers<3&&self.min_admitted {"flop_min_v1"}else{"flop_fast_v1"}
    }
    pub fn from_v3(p95_100:Option<f64>,p95_200:Option<f64>)->Self {
        Self{min_admitted:[p95_100,p95_200].iter().all(|v|
            v.is_some_and(|x|x.is_finite()&&x>0.0&&x<=10.0))}
    }
}
pub fn flop_deadlines(t0:u64,budget:u8)->Result<(u64,u64),proto::UnsupportedReason> {
    if !(1..=30).contains(&budget) {return Err(proto::UnsupportedReason::EngineError{
        message:"flop_budget_s must be in 1..=30".into(),retryable:false});}
    let first=t0.checked_add(u64::from(budget)*1000).ok_or(proto::UnsupportedReason::EngineError{
        message:"deadline overflow".into(),retryable:false})?;
    let final_at=first.checked_add(5000).ok_or(proto::UnsupportedReason::EngineError{
        message:"deadline overflow".into(),retryable:false})?;
    Ok((first,final_at))
}
```

Preflop wager count includes the opening live BB/straddle as first wager, plus voluntary raises/all-ins that increase the facing amount. Forced SB/BB posts are not counted twice, calls/checks/folds do not increment. Thus an ordinary open plus call is2 and a raise/re-raise is3. Detect limped pots separately and use conservative fast policy; do not label them as a presolver SRP scenario.

`set_config` rejects0/31, default remains10; it increments config revision through Plan2's path. Active hand money is frozen in HandConfig; solver preference is captured once at admission for a request. Load V3 admission only from a report matching exact template signature, solver/adapter/rules versions, mode policy, source hash and machine; absent/stale evidence defaults fast.

- [ ] **Step 4 (4 min): Extend fake-clock tests of existing watchdog and worker send.** At elapsed250ms from t0, default flop wire deadline=`10000-250-150=9600`; max flop=`30000-250-150=29600`. Watchdog at14,900/34,900ms. A turn at max flop preference still emits at14,900, first attempt6000, extraction200; flop extraction600. Assert config mutation does not extend an already admitted request.

```rust
assert_eq!(10_000_u64.saturating_sub(250).saturating_sub(150),9600);
assert_eq!(30_000_u64.saturating_sub(250).saturating_sub(150),29600);
assert_eq!(flop_deadlines(0,30).unwrap().1-100,34_900);
```

- [ ] **Step 5 (3 min): Run green:** `cargo test -p engine flop_budget`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): apply flop budgets and measured template policy' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 10: Route flop and turn through cache, provisional and live solve

**Files:** Modify `crates/engine/src/{core,flop,cache_bridge,log}.rs`, `crates/engine/tests/{flop_path_golden,support/mod}.rs`; create `crates/engine/tests/golden/flop_path.json`.

**Interfaces:** Consumes `Cache::lookup`, `Cache::store`, Plan2 WorkerLink/admission/retry/assembly, `SolveInput`, `register_snapshot`. Produces `CacheRoute::{Final(CacheHit),Refine{retained:Option<CacheHit>}}`, `choose_cache_route(Vec<Lookup>)->CacheRoute`; bridge `entry_from_solution(input:&SolveInput,solution:&StreetSolution,reasons:&[ApproxReason],bb_chips:u32,rake:&Rake,elapsed_ms:u32)->Result<CacheEntry,UnsupportedReason>`. Harness extension `run_flop_script(cache:Vec<Lookup>,raw:f32,status:&str)->Vec<RecommendationEvent>` uses the existing fake worker with a complete validated payload; does not mock the engine result.

- [ ] **Step 1 (4 min): Add `deadline_best_so_far_labelling` to the existing fake-worker golden.** Fix root P100, raw1.9, target50, returned before worker deadline; assert all action EVs, 190/50 reason, raw stored0.019, miss logged without SRP street violation. Repeat same state under a fresh decision_id: cache emits Provisional before live refinement; same hand/revision/config/model are retained. An at-target `ok` raw0.4 has no newly incurred DeadlineBestSoFar. A turn best_so_far logs a street violation. A malformed payload never enters the cache.

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
    let (raw,p,target)=(1.9_f32,100_u32,50_u16);
    assert!(raw as f64/p as f64>target as f64/10000.0);
    assert_eq!((raw as f64/p as f64*10000.0).round() as u16,190);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine deadline_best_so_far_labelling`; the newly added engine assertions must fail before routing changes.
- [ ] **Step 3 (5 min): Implement cache-route selection with retained provisional evidence.**

```rust
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
```

The driver performs the first lookup immediately and emits a Provisional when present; a second distinct live-template lookup uses the same overall500ms cache budget and remaining street budget. Avoid duplicate lookup if live=fast. If a usable at-target second result arrives, publish Final. If both are provisional, retain the better validated accuracy and refine; no successful cache hit dispatches an unnecessary live solve.

- [ ] **Step 4 (5 min): Wire the existing HU pipeline.** For each Flop/Turn decision, derive street root, replay root public ranges, materialize query tree(s), then cache route. Final hit: validate current identity/expiry, assemble from query node/Derived legal menu, register snapshot, emit once. Provisional: same validation/assembly/registration, retain in watchdog state, then send live solve through existing admission. Miss: send live solve. River bypasses cache. Worker result: shared validation, resolve every chip path, requested actor versus Derived, accumulate inherited reasons plus BestSoFar if incurred, snapshot, Final, then enqueue storage only for Flop/Turn. Apply memory rule in worker; assert returned mode and log it, do not add an engine-side mode field to the wire.

```rust
pub fn cacheable(street:proto::Street,experimental:bool,locks_applied:u16,baseline:bool)->bool {
    !experimental&&matches!(street,proto::Street::Flop|proto::Street::Turn)
        &&(!baseline||locks_applied==0)
}
pub fn is_street_violation(street:proto::Street,srp_miss:bool,best_so_far:bool,late:bool)->bool {
    late||(best_so_far&&!(street==proto::Street::Flop&&srp_miss))
}
```

- [ ] **Step 4a (5 min): Extend the existing multiway experimental branch to flop templates.** Its contract remains spec §6: choose the opponent with highest range-vs-range equity against hero's public root range; use the current total pot, minimum remaining stack, empty history and unconditioned current-street root ranges. Skip if that opponent is all-in, stack is zero, or equity exhausted the budget. This isolated request never enters SolveInput/cache/snapshots. Read hero's root node when OOP or the node after OOP Check when IP, and attach only to ExperimentalHu; the main actions still carry no solved multiway EV.

```rust
pub fn experimental_financials(d:&proto::Derived,hero:proto::Seat,opponent:proto::Seat)
    ->Option<(u32,u32)> {
    if *d.all_in.get(opponent.0 as usize)? {return None;}
    let stack=(*d.stacks_remaining.get(hero.0 as usize)?)
        .min(*d.stacks_remaining.get(opponent.0 as usize)?);
    (stack>0&&d.pot>0).then_some((d.pot,stack))
}
```

Use the remaining street/final deadlines and the fixed note `experimental, not solved: synthetic root, empty history, unconditioned ranges`. Add `experimental_surrogate_golden` assertions for flop pot/stack/ranges/history and block separation; this supplies the experimental assertions needed by recorded multiway fixtures even if Plan 2 implemented only its river/turn branch.

Use actual result tree after `_min` retry in entry/tree_signature, not first-attempt template. Retained cache payload may be better than a later live best-so-far: final assembly can keep the better validated payload, disclosing both accuracy and incurred reasons, while still logging the live terminal. Never claim live refinement resumed a finalized solver; it starts fresh. Retry remains once, with remaining-time/min-template p95 admission; tree_mismatch never retries.

- [ ] **Step 5 (5 min): Complete flop-path goldens.** Cases: exact synthetic hit (no reasons), chart hit (ChartRounded), SPR-only, menu-only, both, provisional->ok, provisional->no_iteration, cold SRP best_so_far, cold SRP raw-target ok, 3-bet fast, stale hit/result discarded, missed actor/path, disk blocked, turn store, river no store, experimental no store. Freeze canonical event projections (phase, identity, reasons, actions/EVs, mode, snapshot origin) in `flop_path.json`; exclude wall-clock nondeterminism. Assert exactly one Final at or before watchdog expiry even when cache reader remains blocked.

```rust
fn final_count(events:&[proto::RecommendationEvent])->usize {
    events.iter().filter(|e|matches!(e,proto::RecommendationEvent::Final(_))).count()
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --test flop_path_golden`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): complete cache provisional and live flop routing' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 11: Register cache snapshots and translate prior-street bets

**Files:** Modify `crates/engine/src/{cache_bridge,snapshots}.rs`, `crates/engine/tests/support/mod.rs`; create `crates/engine/tests/cache_snapshot_replay.rs`, `crates/engine/tests/golden/cache_snapshot_replay.json`.

**Interfaces:** Consumes exact `SnapshotKey`, `SnapshotProvenance`, `StreetSnapshot`, `ReplayInput`, `replay`; Produces `snapshot_from_hit(identity:&DecisionIdentity,input:&SolveInput,hit:&CacheHit,origin:&str)->StreetSnapshot`. Root hashes are query public range hashes at that root; snapshot tree/nodes are query-sized, original-suit values.

- [ ] **Step 1 (5 min): Write cache-hit replay integration assertions.** Cache-hit Final at flop root, append Bet50/Call and turn4d: root and IP call likelihoods each condition once, returned turn input uses replayed marginals, and no turn strategy is requested from the flop entry. Equivalent100/500 and200/1000 paths must produce equal normalized turn ranges. Then solve at prefix Check, append villain Bet73/Call against snapshot menus50/100: expect prior-street BetTranslation, and the turn solves from turn root.

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
        stack_oop_root:500,stack_ip_root:500,dead_this_street:0,projected_from:2,history:vec![]},
        ranges:query.source.ranges.clone(),tree:query.tree.clone(),target_bp:50};
    let snapshot=engine::cache_bridge::snapshot_from_hit(&id,&input,&hit,"cache_exact");
    assert_eq!(snapshot.provenance.identity_at_solve,id);
    assert_eq!(snapshot.covered_paths,hit.covered_paths);
    assert_eq!(snapshot.tree,input.tree);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --test cache_snapshot_replay`; cache origin/prefix/range assertions fail until registration is wired.
- [ ] **Step 3 (5 min): Construct snapshots from the query, never from stored chip amounts.**

```rust
pub fn snapshot_from_hit(id:&proto::DecisionIdentity,input:&proto::SolveInput,
    hit:&cache::lookup::CacheHit,origin:&str)->core_replay::StreetSnapshot {
    let reasons=match &hit.coverage {proto::Coverage::Exact=>Vec::new(),
        proto::Coverage::Approximate{reasons}=>reasons.clone(),
        proto::Coverage::Unsupported{partial,..}=>partial.clone()};
    core_replay::StreetSnapshot {
        key:core_replay::SnapshotKey{hand_id:id.hand_id,config_revision:id.config_revision,
            model_revision:id.model_revision,street:input.root.street,root_board:input.root.board.clone(),
            root_range_hashes:[core_ranges::hash_scaled(&input.ranges[0]),core_ranges::hash_scaled(&input.ranges[1])],
            tree_signature:hit.tree_signature.clone()},
        provenance:core_replay::SnapshotProvenance{identity_at_solve:id.clone(),
            solved_prefix:input.root.history.clone(),origin:origin.into()},
        tree:hit.tree.clone(),nodes:hit.solution.nodes.clone(),covered_paths:hit.covered_paths.clone(),
        exploitability_chips:hit.solution.exploitability_chips,reasons,
    }
}
```

Set `CacheHit.tree_signature` directly from the query key; it is never parsed from display notes. Origins are exactly `cache_exact`, `cache_approximate`, `cache_provisional`; live remains `live`. Call existing `register_snapshot` only after active identity and expiry checks. Provisional replacement uses full identity; later-street invalidation and solved-prefix retention remain Plan3 behavior.

- [ ] **Step 4 (5 min): Exercise all three export coverages and later replay.** Requested-node-only covering[Check]: missing root does not condition OOP, translated villain wager does condition, uncovered call does not. Root-only: check conditions, villain wager and call do not. Complete street: all three actions condition. Preserve already conditioned masses when a later path is uncovered. Compare golden1326 arrays and log_reach, branch weights, reasons. Add turn->river translation with the same cases; no prior street is re-solved. Current-street73 is inserted exactly alongside50, never translated. Undo keeps prefix-compatible snapshots, discards incompatible/later ones; mismatched hand/config/model/ranges/board cannot be reused.

```rust
let expected_origins=["cache_exact","cache_approximate","cache_provisional"];
assert!(expected_origins.contains(&snapshot.provenance.origin.as_str()));
assert_eq!(snapshot.provenance.identity_at_solve,identity_at_validation);
assert_eq!(snapshot.covered_paths.len(),snapshot.nodes.len());
```

- [ ] **Step 5 (3 min): Run green:** `cargo test -p engine --test cache_snapshot_replay`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): replay query-sized cache snapshots across streets' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 12: Enumerate presolver tiers and canonical-flop order

**Files:** Create `crates/cache/src/presolver/{mod,scenarios}.rs`, `crates/cache/tests/presolver_queue.rs`; modify `crates/cache/src/lib.rs`.

**Interfaces:** Consumes `Position`, `Card`, `core_iso::canonicalize`, `orbit_size`; Produces `Scenario{tier:u8,depth_bb:u16,opener:Position,caller:Position,three_bettor:Option<Position>}`, `scenarios()->Vec<Scenario>`, `canonical_flops_ordered()->Vec<Vec<Card>>`. Each scenario's id is a deterministic serialization of these fields.

- [ ] **Step 1 (3 min): Write tier counts and ordering tests.**

```rust
#[test]
fn tier_scenarios_and_flop_order_are_complete() {
    use cache::presolver::scenarios::*;
    let s=scenarios();assert_eq!(s.len(),24);
    assert_eq!(s.iter().filter(|x|x.tier==1).count(),4);
    assert_eq!(s.iter().filter(|x|x.tier==2).count(),8);
    assert_eq!(s.iter().filter(|x|x.tier==3).count(),12);
    let f=canonical_flops_ordered();assert_eq!(f.len(),1755);
    assert_eq!(1755*4,7020);assert_eq!(1755*24,42120);
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache --test presolver_queue`.
- [ ] **Step 3 (5 min): Implement the exact scenario inventory.** In 3-bet cases caller denotes the original opener who calls the 3-bet; opponent is three_bettor.

```rust
use proto::Position;
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize)]
pub struct Scenario {pub tier:u8,pub depth_bb:u16,pub opener:Position,pub caller:Position,
    pub three_bettor:Option<Position>}
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

- [ ] **Step 4 (5 min): Enumerate all C(52,3) flops through Plan1 canonicalization.** Use zero ranges for board-only canonical representative enumeration (the canonical board is unaffected by range tie-break); do not hash or solve with these zero ranges. Deduplicate lexicographically by Card ids; compute orbit_size; sort by descending orbit then rank/suit lexicographic canonical board. At actual job preparation canonicalize again using replayed public ranges for stabilizer tie-break.

```rust
pub fn raw_flops()->impl Iterator<Item=[proto::Card;3]> {
    (0_u8..50).flat_map(|a|((a+1)..51).flat_map(move|b|
        ((b+1)..52).map(move|c|[proto::Card(a),proto::Card(b),proto::Card(c)])))
}

pub fn canonical_flops_ordered()->Vec<Vec<proto::Card>> {
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
}
```

Task iteration order is `(tier, flop_order, scenario_order)`, not all1755 boards for one scenario. Assert orbit24 before12 before4 and orbit counts sum22,100; all four tier1 scenarios occur before moving to the second board. Never use `flop_full_v1`.

- [ ] **Step 5 (3 min): Run green:** `cargo test -p cache --test presolver_queue`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): enumerate required presolver scenarios and flop order' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 13: Persist queue identities, cursor and verified completion

**Files:** Create `crates/cache/src/presolver/queue.rs`; modify `crates/cache/src/presolver/mod.rs`, `crates/cache/tests/presolver_queue.rs`.

**Interfaces:** Consumes `Scenario`, `Rational`, `KeyFields::scenario_identity` and `write_atomic`. Produces `TaskStatus::{Pending,Done,Failed{n:u8}}`, `QueueFile`, `QueueItem`, `Queue::open(PathBuf)->Result<Queue,CacheError>`, `Queue::save()->Result<(),CacheError>`, `Queue::reconcile(&dyn Fn(&QueueItem)->bool)`, `Queue::record_failure(id:&str,now_ms:u64)`. Normalized identity is key without bucket plus exact scenario SPR, not raw hand id or board index.

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
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache presolver_done_requires_a_valid_entry`.
- [ ] **Step 3 (5 min): Define durable schema and transitions.**

```rust
#[derive(Clone,Debug,serde::Serialize,serde::Deserialize)]
#[serde(rename_all="snake_case")]
pub enum TaskStatus {Pending,Done,Failed{n:u8}}
#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub struct QueueItem {
    pub identity:[u8;32],pub scenario:super::scenarios::Scenario,pub board:Vec<proto::Card>,
    pub spr:crate::key::Rational,pub status:TaskStatus,pub retry_after_unix_ms:u64,
    pub attempts:u8,pub last_error:Option<String>,
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
```

QueueFile version1 is independent of cache schema3. Cursor is next `(tier_index,flop_index,scenario_index)`; completed/failed items before cursor are revisited for invalidation or eligible retry. Persist before launch with status Pending and incremented attempts; in-flight is runtime-only so restart resumes pending work. Initial attempt plus three retries (maximum4 attempts); cancellation for live work returns Pending without consuming a retry. `failed{n}` records actual failed attempt count; after the fourth failure it remains failed. Thirty-second backoff uses monotonic time in process; persisted UTC deadline on restart is clamped to0..30s, preventing a clock jump from stalling the queue indefinitely.

- [ ] **Step 4 (5 min): Save queue atomically and reconcile it with storage.** Use Task6 `write_atomic` for `queue.json`; bounded JSON read (64MiB), explicit schema version, valid cursor/tier/board/spr/identity, no duplicate item keys. A corrupt queue is reported and rebuilt deterministically; valid cache cells still establish completion. Successful worker terminal is insufficient: validate solution, await writer receipt, then read/validate the exact normalized scenario identity at raw<=0.005 before Done. Read-only cache/full disk leaves Pending/Failed; corrupt/evicted/stale-range entries change Done back to Pending on startup and periodic reconciliation. Exact identity excludes bucket but includes source SPR, versions, range hashes, model, tree and rake.

```rust
pub fn queue_path(cache_root:&std::path::Path)->std::path::PathBuf {cache_root.join("queue.json")}
pub fn save_queue(path:&std::path::Path,file:&QueueFile)->Result<(),crate::CacheError> {
    let bytes=serde_json::to_vec_pretty(file)?;
    if bytes.len()>64*1024*1024 {return Err(crate::CacheError::Invalid("queue size"));}
    crate::storage::write_atomic(path,&bytes)
}
```

- [ ] **Step 5 (4 min): Test persistence roundtrip.** Save cursor after tier1 board0/scenario2; reopen; verify next job is scenario2 and completed earlier entries are not repeated. Delete its cache cell then reconcile: Pending. Replace source bundle hash: new identity, old entries remain isolated. Simulate crash before/after entry rename and before queue save: neither falsely marks Done. Failed attempt1/2/3 retries at30s; failure4 does not. Cancellation/restart does not burn retries.

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

### Task 14: Schedule idle jobs and cancel them for live admission

**Files:** Create `crates/cache/src/presolver/scheduler.rs`; modify `crates/cache/src/presolver/{mod,queue}.rs`, `crates/cache/tests/presolver_queue.rs`.

**Interfaces:** Consumes `Queue`, `QueueItem`, `Scenario`, `CacheEntry` and `SolveInput`. Produces `Presolver::{start,pause,resume,status}`, `PresolveExecutor` (defined here, distinct from WorkerLink), `PresolverStatus`, and events `HandInProgress(bool)`, `LiveRequest`, `Pause`, `Resume`, `Shutdown`. Cache never imports engine, replay or preflop. Executor uses engine admission over channels.

- [ ] **Step 1 (4 min): Write the30s gate and pause tests.**

```rust
#[test]
fn idle_requires_no_hand_and_thirty_seconds() {
    use cache::presolver::scheduler::eligible;
    assert!(!eligible(false,false,29_999,0));assert!(eligible(false,false,30_000,0));
    assert!(!eligible(true,false,60_000,0));assert!(!eligible(false,true,60_000,0));
    assert!(!eligible(false,false,30_000,30_000));
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p cache idle_requires_no_hand_and_thirty_seconds`.
- [ ] **Step 3 (5 min): Define the executor/handle contracts and scheduler predicate.**

```rust
pub struct PreparedJob {pub item:queue::QueueItem,pub input:proto::SolveInput,pub rake:proto::Rake,pub bb_chips:u32}
pub enum JobPoll {Running,Complete(crate::entry::CacheEntry),Cancelled,Failed(String)}
pub trait PresolveExecutor:Send+'static {
    fn prepare(&mut self,scenario:&scenarios::Scenario,board:&[proto::Card])->Result<PreparedJob,String>;
    fn submit(&mut self,job:PreparedJob)->Result<u64,String>;
    fn poll(&mut self,id:u64)->JobPoll;
    fn cancel(&mut self,id:u64);
    fn entry_exists_at_target(&mut self,item:&queue::QueueItem)->bool;
}
#[derive(Clone,Default)]
pub struct PresolverStatus {
    pub paused:bool,pub running:Option<String>,pub pending:u32,pub done:u32,pub failed:u32,
    pub tier_done:[u32;3],pub tier_total:[u32;3],pub measured_p50_s:Option<f64>,
    pub estimated_remaining_s:Option<f64>,pub scenario_hits:Vec<(String,u64,u64)>,
}
pub fn eligible(hand_in_progress:bool,paused:bool,now:u64,last_activity:u64)->bool {
    !hand_in_progress&&!paused&&now.saturating_sub(last_activity)>=30_000
}
```

`Presolver::start(dir:PathBuf,executor:Box<dyn PresolveExecutor>)->Presolver`; `pause(&self)`, `resume(&self)`, `status(&self)->PresolverStatus`, `notify_hand(&self,bool)`, `notify_live_request(&self)`, `shutdown(&self)`. Use one named presolver thread, a bounded command channel, status Arc/RwLock; no status lock across I/O or executor calls. Poll is nonblocking and submit only enqueues; scheduling thread remains responsive while worker runs.

- [ ] **Step 4 (5 min): Implement event transitions and cancellation.** A live request synchronously flags cancellation at engine admission, even before presolver thread consumes its notification. Presolver sees the live event, updates last activity, cancels active id, returns its item to Pending, waits for terminal/confirmed exit through engine. BeginHand also cancels; Complete/Abandoned restarts idle timer. Pause persists paused=true and stops scheduling; let an already-running job finish unless live/hand/shutdown cancels it. Resume preserves cursor and waits for idle. Shutdown requests cancel, persists queue, and never joins a blocked cache thread on the UI command path.

```rust
pub enum ScheduleAction {Wait,Cancel(u64),Launch}
pub fn next_action(active:Option<u64>,live:bool,can_start:bool)->ScheduleAction {
    match(active,live,can_start) {
        (Some(id),true,_)=>ScheduleAction::Cancel(id),
        (None,false,true)=>ScheduleAction::Launch,_=>ScheduleAction::Wait,
    }
}
```

Use600000ms background deadline/target50/extraction600 via executor. The engine's existing control thread handles cancellation at iteration boundaries, ack<=50ms, kill after1.5s. A background ack must not release admission; live job may proceed only after background terminal or confirmed exit. Background work never emits user-facing decision events or snapshots and cannot overwrite current retained live evidence.

- [ ] **Step 5 (4 min): Test restart, cancellation and durable completion with a fake executor.** Implement fake trait with `VecDeque<JobPoll>`, cancel counter and a bool durable-entry flag. Assert no submission before30s; exactly one while running; live cancellation count1; Pending remains when worker completes but durable flag=false; Done only after true. After pause/restart/resume, cursor and failed counters match. Polling/cancel uses fake clock advances, no30s sleeps.

```rust
#[test]
fn live_request_cancels_background_before_new_launch() {
    use cache::presolver::scheduler::{next_action,ScheduleAction};
    assert!(matches!(next_action(Some(7),true,true),ScheduleAction::Cancel(7)));
    assert!(matches!(next_action(Some(7),false,true),ScheduleAction::Wait));
    assert!(matches!(next_action(None,false,true),ScheduleAction::Launch));
}
```

- [ ] **Step 6 (3 min): Run green:** `cargo test -p cache --test presolver_queue`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/cache
git commit -m 'feat(cache): schedule idle presolves with live cancellation' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 15: Prepare presolves by chart replay and report actual coverage

**Files:** Create `crates/engine/src/presolve.rs`, `crates/engine/tests/presolver_engine.rs`; modify `crates/engine/src/{engine,log}.rs`.

**Interfaces:** Implements `cache::presolver::PresolveExecutor` using existing Engine and WorkerLink; Consumes `PreflopStore::query`, `replay(ReplayInput)`, `build_effective_tree`, last-used `HandConfig`. Produces `scenario_hand(cfg:&HandConfig,scenario:&Scenario,board:&[Card],store:&PreflopStore)->Result<HandState,UnsupportedReason>`, `scenario_hit_rates(records:&[DecisionRecord])->Vec<(String,u64,u64)>`, Engine `presolver_status/pause/resume` delegation for Plan5.

- [ ] **Step 1 (5 min): Write chart-replay equivalence and admission tests.** Construct tier1 BTN/BB100 with last config1/2 and the same legal prefix manually; compare both1326 vectors, range hashes, root pot/stacks, tree signature, reasons. Change hero cards: identical job identity. Change chart bundle: new hashes/identity. At live admission during Solving, assert background cancel before live send and no release at ack.

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
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p engine --test presolver_engine`.
- [ ] **Step 3 (5 min): Implement scenario prefixes using actual chart sizes.** Begin a six-seat synthetic hand with stacks=`depth_bb*bb_chips`, last-used blinds/rake/straddle and buttonSeat0; use core-model legal order. Query each prefix through the active highest-precedence bundle; choose its resolved open/3-bet size, explicit folds by unused seats, and Call by named defender/opener. No generic percentage ranges or guessed missing nodes. Set flop through core-model, call replay with no postflop snapshots, use its public root marginals and accumulated reasons, derive root and materialize fast. If a needed prefix is absent, record failed reason and keep it visible; replay's specified missing-prior-node fallback is allowed only when deliberately frozen in source provenance, never silently invented.

```rust
let replay=core_replay::replay(core_replay::ReplayInput{
    cfg:&state.config,state:&state,store:&store,snapshots:&[]});
let root=core_model::street_root(&state).map_err(|e|format!("{e:?}"))?;
let ranges=[replay.ranges[root.oop.0 as usize].clone().ok_or("missing OOP range")?,
            replay.ranges[root.ip.0 as usize].clone().ok_or("missing IP range")?];
assert!(ranges.iter().all(|r|r.0.iter().any(|w|*w>0.0)));
```

Use checked multiplication and model legality for source sizes; freeze actual folds and raises in queue scenario provenance. The synthetic hand has no hero-card effect. Prepare against a snapshot of config/bundle revisions; changes cancel outstanding background preparation and invalidate its queue generation, while older cache entries remain addressable only by their original hashes.

- [ ] **Step 4 (5 min): Submit through the single engine worker owner.** Background Solve uses input root P/stacks, public ranges, fast tree, history empty, target50, deadline600000, extraction600, memory10GiB and backgroundtrue. Worker sets BELOW_NORMAL and applies the same2GiB f32/i16 rule, returning mode in payload. No parallel worker is spawned. Completion invokes entry_from_solution with replay reasons and exact job versions; store, await receipt, re-read raw accuracy then mark Done. Failure/backoff/cancel feed scheduler; no acquired-data path is required while V9 is deferred.

```rust
pub const BACKGROUND_DEADLINE_MS:u32=600_000;
pub const BACKGROUND_TARGET_BP:u16=50;
pub const BACKGROUND_TEMPLATE:&str="flop_fast_v1";
```

- [ ] **Step 5 (5 min): Extend DecisionRecord and status computation.** Log every actual decision's input record/identity, scenario id+tier when matched, cache result (`miss|exact|approximate|provisional`), raw reached exploitability, selected template/mode, street/final deadline violations and elapsed. Existing log remains `%LOCALAPPDATA%\PokerAI\decisions.jsonl`, rotation50MiB×10. Match by real preflop line/depth/config; don't call a mismatching line a scenario hit. Hit-rate numerator counts at-target exact/approximate cache Finals; denominator includes matching requests with misses/provisional; disclose provisional separately. Aggregate from decision log, not fabricated queue coverage.

```rust
pub fn hit_rate(hits:u64,requests:u64)->Option<f64> {
    (requests>0).then(||hits as f64/requests as f64)
}
pub fn remaining_seconds(pending:u32,measured_p50:Option<f64>)->Option<f64> {
    measured_p50.filter(|x|x.is_finite()&&*x>0.0).map(|x|pending as f64*x)
}
```

Report measured time-to-target by scenario/template/mode; recompute1755×p50, sum remaining counts. Initial R8 estimates must be labelled unmeasured production estimates:100bb1755×27s≈13h/scenario, tier1≈52h;200bb1755×52s≈25h/scenario; tier2≈62h; phase2 full≈112h/scenario excluded. Tier1 starts now and continues after release; no claim that entire tier is complete from a successful scheduler test. Completion of tiers2/3 is not a release requirement.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p engine --test presolver_engine`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/engine
git commit -m 'feat(engine): drive chart-replay presolves and measured hit rates' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 16: Freeze chart provenance and all fifty recorded inputs

**Files:** Modify `tools/gen_fixtures.py`; create `tools/e2e_hands.py`, `tools/tests/test_e2e_hands.py`, `fixtures/hands/e2e/{001..050}.json`, `fixtures/hands/e2e/manifest.json`, `bench/spots/sources.json`; modify `crates/engine/src/bench_support.rs` for record validation.

**Interfaces:** Consumes Plan1 PokerKit state/fixture adapter and Plan3 chart bundles/manifests. Produces `records()->list[dict]`, `write_e2e(root:Path)->None`, CLI `python tools/gen_fixtures.py e2e`, version1 records with config/stacks/hero/button/dealt/events/expected/fault. Manifest is not a hand; e2e runner loads exactly001–050.

- [ ] **Step 1 (4 min): Write inventory tests before generating files.**

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
```

- [ ] **Step 2 (2 min): Run red:** `python -m pytest tools/tests/test_e2e_hands.py -q`; missing generator.
- [ ] **Step 3 (5 min): Freeze source URLs, versions, sizes and node inventory.** Use Plan3's inspected/transcribed source bytes, not another fetch that can drift. Sources: PokerCoaching100, `https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf`, six-page Implementable GTO snapshot2026-09-10; RangeConverter200, `https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem`, download page `https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash`, 13-page500z/50%-rounded snapshot2026-09-10. Source version is content sha256 plus transcription envelope version2, not an invented publisher release. Store final PDF URL/hash from Plan3's audit in sources.json.

```python
import hashlib, json
from pathlib import Path

def freeze_sources(repo: Path) -> dict:
    result = {'version': 1, 'snapshot_date': '2026-09-10', 'bundles': []}
    for name in ('pokercoaching_100', 'rangeconverter_200'):
        raw = (repo / 'fixtures/charts' / f'{name}.json').read_bytes()
        manifest = json.loads((repo / 'fixtures/charts' / f'{name}.manifest.json').read_text())
        result['bundles'].append({'name': name, 'sha256': hashlib.sha256(raw).hexdigest(),
            'manifest': manifest, 'nodes': [n['history'] for n in json.loads(raw)['nodes']]})
    return result
```

Freeze covered nodes needed for all six benchmark lines and24 presolver scenarios: unopened folds/RFI UTG/HJ/CO/BTN/SB, BB/SB/BTN responses to each named opener, and original opener responses to listed3-bets. Missing-node fallbacks are explicit:100bb PokerCoaching has no verified complete vs3-bet chart, so a missing opener-response prefix stops that replay branch at its pre-action public masses with `UnconditionedPriorStreet{Preflop,cause:"missing node <key>"}`; no synthetic probability is inserted. Synthetic fixture-only missing nodes are UTG limp, CO limp and BB cold-call versus a3-bet; mark exact absent prefix in manifest. For straddle, virtual-role missing prefix follows the same rule. Require chart-source audit from Plan3 to be complete before this task passes; no benchmark generation with unknown source version/node inventory.

- [ ] **Step 4 (5 min): Define explicit common record/event constructors.** The record schema below is a benchmark input format, translated into existing proto commands by Task18. It does not replace proto or depend on its JSON field layout.

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

RakeProfileMapped is required wherever Plan3's chart manifest declares undocumented rake; freeze that from source audit. All chart-based supported fixtures require ChartRounded. At generation, independently verify chosen hero8h7h is positive in the replayed chart marginal; if a source chart excludes it, generation fails for review and an explicit revised fixture hero is committed before measurements. Do not silently select another hand during bench.

- [ ] **Step 5 (5 min): Generate the22 designated supported records with exact action inputs.**

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

Freeze admitted projection expectations `folded_this_street=1,dead_this_street=0/50`; reproduce facing100/min_raise350 for the paid case. Additional chart missing-prefix reasons must be enumerated by the audited source mapping and hand-checked, not harvested from the final engine output.

- [ ] **Step 6 (5 min): Define remaining28 records from this finite class inventory.** IDs and differences are fixed, not random samples:

| IDs | Inputs relative to common constructors | Expected result |
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
|037–046|Default supported001 input, faults in order Oom,Eof,Malformed,Oversized,TreeMismatch,BlockedCacheIo,SlowAllocation,NoIteration,ClockJump,SuspendResume|Fault expectations in Task19|
|047–050|Default turn009 withOom, river015 withEof, turn009 withNoIteration, flop001 withSlowAllocation at default budget10 (also exercised at30 by the fault runner)|Task19 reasons; default final watchdog14.9s; separate max-budget fault variant34.9s|

Implement transformations as functions returning complete independent records, assign ids in inventory order, and serialize every event/config/stack into each JSON. No runtime references to another fixture remain in the files.

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

- [ ] **Step 6a (5 min): Implement the finite exceptional-input transformations and `records()`.** Each is a complete copy with explicit events, never an expectation-only sample.

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

Here `numeric_ev:false` on the four straddle cases means they are outside the designated22 numeric-EV release subset; it is not an instruction to suppress a valid numeric postflop result. Unsupported fixtures separately require numeric absence. Only the supported flag is used for the count assertion. Add `supported_baseline` explicitly when writing the manifest to keep these meanings distinct.

- [ ] **Step 7 (5 min): Validate and freeze before benchmark generation.** Extend existing PokerKit adapter to play every record to its decision, compare legal actor, pot, committed amounts, stacks and board at every step; straddle and projections must be legal real sequences first. `gen_fixtures.py e2e --check` regenerates in memory and compares bytes. Use sorted JSON keys and SHA256 in manifest. Review exact reason sets against source-node inventory and spec; supported22 require numericEV after solver, remaining28 carry their declared classification. No real benchmark results are used to invent expectations.

```python
def write_e2e(root: Path):
    rows=records()
    assert len(rows)==50
    hashes={}
    for i,row in enumerate(rows,1):
        row['id']=f'{i:03}'
        raw=(json.dumps(row,sort_keys=True,indent=2)+'\n').encode()
        (root/f'{i:03}.json').write_bytes(raw)
        hashes[f'{i:03}.json']=hashlib.sha256(raw).hexdigest()
    (root/'manifest.json').write_text(json.dumps({'version':1,'synthetic':True,
        'supported_ids':[f'{i:03}' for i in range(1,23)],'sha256':hashes},sort_keys=True,indent=2)+'\n')
```

- [ ] **Step 8 (3 min): Run green:** `python -m pytest tools/tests/test_e2e_hands.py -q`; `python tools/gen_fixtures.py e2e --check`; `cargo test --workspace`.
- [ ] **Step 9 (2 min): Commit.**

```powershell
git add tools fixtures/hands/e2e bench/spots/sources.json crates/engine/src/bench_support.rs
git commit -m 'test(bench): freeze chart provenance and fifty recorded decisions' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 17: Generate chart-replay flop suites and measure V3 modes

**Files:** Modify `crates/bench/src/{main,gen_spots,report}.rs`, `crates/engine/src/bench_support.rs`; create `crates/bench/src/{lib,flop}.rs`, `crates/bench/tests/flop.rs`, `bench/spots/{flop_fast,flop_min}.json`; modify `solver-worker/Cargo.toml`, `solver-worker/src/{main,memory}.rs` only for an opt-in diagnostic storage-mode launch flag; add `solver-worker/tests/bench_mode.rs`.

**Interfaces:** Consumes `scenario_hand`, `replay`, `SolveInput` and frozen sources.json through the engine facade `generate_flop_spots(source:&Path)->Result<Vec<FlopBenchSpot>,EngineError>`; bench depends only on engine/proto. Produces `FlopBenchSpot{line_id:String,depth_bb:u16,pot_class:String,input:SolveInput,source_hashes:Vec<String>}`, `RunConfig{template:String,threads:u8,mode:String,cold:bool,reps:u32}`, `flop_matrix()->Vec<RunConfig>`, CLI `bench gen-spots` and `bench run --suite flop_fast|flop_min --threads N --reps R --out docs/bench/`.

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
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test flop`; add assertions against generated files/runner so these fail before generation.
- [ ] **Step 3 (5 min): Implement gen-spots using the frozen source lock.** Six line definitions above; open/call and open/BB3bet/openerCall with all explicit folds, actual chart-resolved sizes,100/200bb. Call the same engine scenario/replay facade as live/presolver, retain missing-node fallback reasons already frozen in sources.json, board-block canonical public ranges, derive financial root. Encode full1326 vectors rather than substituting uniform chart-like strings. Attach sources hash, exact history, range hashes, counts/masses, P/stacks/cap, template signature and expected inherited reasons. Hash mismatch aborts generation. Both suite files contain six scenario entries with three board variants,18 concrete solve inputs each.

```rust
pub const BOARDS:[&str;3]=["Kh7d2c","Jh9h6c","8s8d3c"];
pub const THREADS:[u8;3]=[8,16,24];
pub const MODES:[&str;2]=["f32","i16"];
pub const TEMPLATES:[&str;2]=["flop_fast_v1","flop_min_v1"];
pub struct RunConfig {pub template:String,pub threads:u8,pub mode:String,pub cold:bool,pub reps:u32}
pub fn flop_matrix()->Vec<RunConfig> {
    let mut out=Vec::new();
    for template in TEMPLATES {for threads in THREADS {for mode in MODES {for cold in [true,false] {
        out.push(RunConfig{template:template.into(),threads,mode:mode.into(),cold,reps:5});
    }}}}
    out
}
```

Create `crates/bench/src/lib.rs` with `pub mod flop;` and exports of the existing report/gen_spots modules; main imports these library modules. Add e2e/fault/gate exports in the tasks that create them, never declarations for nonexistent future files. This makes integration-test imports concrete and keeps each task's workspace build green.

- [ ] **Step 4 (5 min): Add diagnostic mode forcing without changing production protocol.** Production memory selection remains §10.3. Feature `bench-mode` (default off) permits launch `--bench-storage-mode f32|i16`; it never accepts this as a solve JSON field. Default worker rejects the flag. Build diagnostic binary into `target/bench-mode`, not the normal packaged path; ready adds capability `bench_storage_override`, which normal Engine rejects unless the bench facade explicitly opts in. Forced mode still checks memory headroom and16GiB job-object limit. Bench records both estimates and refusals; a failed allocation is data, never silently switched mode.

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

Compile forced branch only under feature, while existing production memory tests continue using `None`. Default benchmark memory limit10GiB; for diagnostic forced-f32 measurements that would exceed it, explicitly use12GiB request limit under unchanged16GiB process limit and mark `diagnostic_memory_limit` in report; these runs cannot certify live admission. No direct library dependency in bench or engine. Test default flag rejection and mode boundaries2GiB,2GiB+1,4GiB,8GiB, headroom. Requests retain the normative wire schema.

- [ ] **Step 5 (5 min): Run matrix with separate target and deadline trials.** Cold means new validated worker process for each of five runs; warm means five solves in a validated retained process, still rebuilding/freeing games per contract. Threads8/16/24, f32/i16 every spot and template. Long measurement deadline600000 finds raw50bp time-to-target; separate production-policy trials use10s street budget and default memory selection. Record `memory_usage()` both estimates, actual chosen mode, peak working set/total RSS (engine+worker), full admission-to-terminal latency and solve time, iterations, raw exploitability, cancellation latency, startup/cold classification, exact template signature. No censored deadline observation is counted as time-to-target.

```rust
pub fn target_reached(raw:f32,pot:u32)->bool {raw.is_finite()&&raw as f64/pot as f64<=0.005}
pub fn v3_min_admitted(p95_100:Option<f64>,p95_200:Option<f64>)->bool {
    [p95_100,p95_200].into_iter().all(|x|x.is_some_and(|s|s.is_finite()&&s<=10.0&&s>0.0))
}
```

V3:3-bet fast p95<=10s and reproduce analogue4.1–6.7s/0.55–0.9GB within2x; SRP record100bb19–35s/3.2–5.3GB f32/1.6–2.7GB i16 and200bb39–64s/5.2–8.8GB comparison, without requiring live50bp in10s. Measure min at both depths and all boards before admitting it. Record f32/i16 crossover near2GiB and4GiB using the same exact templates plus deterministic range-width probes when six lines do not bracket both points; probe results are diagnostic, never substituted baseline ranges. Recompute1755×measuredp50 throughput. Report deviations/outliers; never discard slow runs to pass p95.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p bench --test flop`; `cargo test -p solver-worker`; `cargo test --workspace`. Benchmark execution itself is Task21 after remaining report/gate code.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/bench crates/engine/src/bench_support.rs bench/spots solver-worker
git commit -m 'feat(bench): generate replay flop suites and V3 measurements' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 18: Replay recorded hands through the real engine

**Files:** Create `crates/bench/src/e2e.rs`, `crates/bench/tests/e2e.rs`; modify `crates/bench/src/{main,report}.rs`, `crates/engine/src/bench_support.rs`.

**Interfaces:** Consumes Task16 `RecordedHand` and existing `Engine` commands/WorkerLink. Produces CLI `bench e2e` and alias `bench run --suite e2e`; engine facade `run_record(record:&RecordedHand,options:&RunOptions)->Result<DecisionRun,EngineError>`. `RunOptions{cache_dir:PathBuf,worker_factory:Box<dyn Fn()->Box<dyn WorkerLink>>,clock:Arc<dyn Clock>,cold_cache:bool}` injects transport without a dependency on the future fault module. `RecordedHand` is version1 Task16 schema; `DecisionRun` contains emitted events/timestamps, log record, peakRSS, solver raw accuracy and snapshots. Inputs use public Engine commands; timing starts immediately before recommend admission, excludes human-entry playback.

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
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test e2e`; include run_record assertions before runner exists.
- [ ] **Step 3 (5 min): Implement command playback and event capture.** Validate schema/hash first; build config and confirmed starting stacks, begin_hand, set hero cards, apply each action only when recorded seat equals derived actor, set full board only at AwaitingBoard, recommend at final recorded decision. Use chart bundles only for baseline; bind prescribed fault through WorkerLink when requested. No direct fabricated HandState reaches recommend. Wait for matching Final, retain late Equity for validation, reject stale events. On completion finish/abandon hand and clean only that run's isolated cache/log directory.

```rust
pub fn evaluated_actions_present(rec:&proto::Recommendation)->bool {
    !rec.actions.is_empty()&&rec.actions.iter().all(|a|a.frequency.is_some()&&a.ev_bb.is_some())
}
pub fn final_for<'a>(events:&'a[proto::RecommendationEvent],id:&proto::DecisionIdentity)
    ->Option<&'a proto::Recommendation> {
    events.iter().find_map(|e|match e {proto::RecommendationEvent::Final(r) if &r.identity==id=>Some(r),_=>None})
}
```

- [ ] **Step 4 (5 min): Enforce expected coverage and cold/hit passes.** First pass has an empty cache per hand, default budget10; all22 designated supported inputs require numericEV with expected reasons. SRP8 miss results require DeadlineBestSoFar iff selected live template did not reach raw50bp; chart reasons always remain, so normal baseline hits are Approximate rather than Exact. Second targeted pass pre-solves supported flop spots to target and measures cache-hit p95<=0.5s with no newly incurred DeadlineBestSoFar. Turn/river cases missing prior requests legitimately retain UnconditionedPriorStreet; separate Task11 goldens verify conditioned cached replay. Every fault fixture has its expected Unsupported reason. Verify two admitted projection dead-money values and rejected step1. Multiway experimental advice stays outside actions; unsupported/no-support fixtures never get fabricated numericEV.

```rust
pub fn has_deadline_reason(c:&proto::Coverage)->bool {
    let reasons=match c {proto::Coverage::Approximate{reasons}=>reasons,
        proto::Coverage::Unsupported{partial,..}=>partial,proto::Coverage::Exact=>return false};
    reasons.iter().any(|r|matches!(r,proto::ApproxReason::DeadlineBestSoFar{..}))
}
```

Record first/final latency and violations independently. Bench may time out a hung runner as a failure; it must not manufacture Final or omit the failed sample. `--reps5` cold-cache default-budget admission-to-Final samples include SRP misses, retries and restarts. Fixed fixture failure definitions determine expectations; do not rewrite them after seeing measured output.

- [ ] **Step 5 (3 min): Run green:** `cargo test -p bench --test e2e`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/bench crates/engine/src/bench_support.rs
git commit -m 'feat(bench): replay fifty frozen hands through engine admission' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 19: Inject worker, disk and clock faults behind WorkerLink

**Files:** Create `crates/bench/src/{fault,faulty_worker}.rs`, `crates/bench/tests/fault.rs`; modify `crates/bench/Cargo.toml`, `crates/bench/src/main.rs`, `crates/engine/src/{bench_support,testing}.rs`, `crates/engine/tests/flop_path_golden.rs`.

**Interfaces:** Consumes Plan2 `engine::worker::WorkerLink`, `engine::testing::{FakeClock,FakeWorker,FakeReply}`, existing raw-line parser/WorkerLinkError, cache I/O test seam. Produces `Fault`, `FaultyWorker` implementing that existing trait by delegation to FakeWorker, `run_fault(fault:Fault,street:Street,budget:u8,retained:bool)->FaultRun`; CLI `bench fault` / `bench run --suite fault`. Enable engine feature `testing` in the bench dependency. Use actual trait signatures from Plan2; no alternate worker transport contract.

- [ ] **Step 1 (4 min): Define exhaustive injections and red suite assertion.**

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
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test fault`; missing run_fault or violated deadline.
- [ ] **Step 3 (5 min): Configure fault scripts through the existing trait.** `FaultyWorker` contains `FakeWorker`; its send/recv/restart/kill methods delegate, except the configured script feeds raw bytes to the real parser for malformed/oversized cases and injects confirmed exit errors for OOM/EOF. Add raw-byte FakeReply support if Plan2 doesn't expose it yet; reuse its bounded stdout decoder. A compile assertion proves the actual trait is used:

```rust
fn assert_worker_link<T:engine::worker::WorkerLink>() {}
#[test]
fn faulty_worker_uses_production_boundary() {assert_worker_link::<FaultyWorker>();}
pub fn malformed_line()->Vec<u8> {b"{\"type\":\"result\",broken}\n".to_vec()}
pub fn oversized_line()->Vec<u8> {vec![b'x';16*1024*1024+1]}
```

| Fault | Injection | Expected no-retained result |
|---|---|---|
|Oom|Confirmed abnormal process exit during allocation; repeat on allowed retry|EngineError with typed WorkerExit; explicit worker out_of_memory variant separately ends TreeTooLarge after min retry|
|Eof|stdout EOF before terminal; restart also EOF|EngineError, once-only Final|
|Malformed|Invalid JSON and a separate complete JSON result with invalid probability matrix|EngineError; neither cached nor snapshotted|
|Oversized|16MiB+1 bytes before newline|EngineError from bounded reader; no unbounded allocation|
|TreeMismatch|non-retryable tree_mismatch terminal|EngineError retryablefalse; no retry/smaller template|
|BlockedCacheIo|Cache reader held on barrier; worker also silent for the timeout variant|DeadlineExceeded while cache remains blocked; separate returning-worker variant still yields valid live Final|
|SlowAllocation|Progress Building, no terminal; kill/restart barrier remains held|DeadlineExceeded by watchdog before kill finishes|
|NoIteration|error no_iteration; min retry not admitted by remaining-time estimate|DeadlineExceeded; no empty best_so_far|
|ClockJump|Jump wall clock ±1day while worker stays silent; monotonic time advances normally|DeadlineExceeded at the monotonic watchdog boundary; wall clock never rebases admission or retry budgets|
|SuspendResume|Resume event invalidates identity/expiry during solve|DeadlineExceeded immediately on resume; worker restart independent|

- [ ] **Step 4 (5 min): Implement run_fault with independent clock/watchdog scheduling.** Use the engine harness, fake monotonic and wall clocks, and real watchdog callback. The test driver advances both scheduler and watchdog independently of WorkerLink polling, so a fake worker blocked forever cannot advance/own delivery time. Inject faults in Building/Solving/Extracting; inject after Provisional and without it. Hold kill, cache read, cache write, stdout read and allocation barriers while advancing to14.9s; assert Final first, release barriers only afterward. Max-budget flop fires34.9s; turn/river remain14.9s even with preference30. Wall jump is not suspend. Suspend immediately expires and rejects any later reply even if monotonic implementation excludes sleep.

```rust
pub struct FaultRun {
    pub final_count:usize,pub final_at_ms:u64,pub final_delivery_violations:u32,
    pub accepted_late_result:bool,pub snapshot_count_after_expiry:usize,
}
pub fn final_limit_ms(street:proto::Street,budget:u8)->u64 {
    if street==proto::Street::Flop {5000+u64::from(budget)*1000-100}else{14900}
}
```

For retained payload cases other than SuspendResume, Final is the validated Provisional with inherited reasons and numericEV; terminal worker failure must not erase it. Assert no expired cache result registers a snapshot, no duplicate terminal/Final, no ack releases admission, and no exception changes money/history. Run the same deterministic fault scripts across all streets, plus max flop budget. Real-clock smoke runs exercise blocked I/O/dead worker delivery but remain separate from deterministic zero-violation proofs.

- [ ] **Step 5 (3 min): Run green:** `cargo test -p bench --test fault`; `cargo test -p engine final_delivery`; `cargo test --workspace`.
- [ ] **Step 6 (2 min): Commit.**

```powershell
git add crates/bench crates/engine
git commit -m 'test(bench): enforce final delivery under worker disk and clock faults' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 20: Report measured results and enforce the baseline release gate

**Files:** Modify `crates/bench/src/{main,report}.rs`; create `crates/bench/src/gate.rs`, `crates/bench/tests/gate.rs`.

**Interfaces:** Consumes Plan2 report/SpotResult plus Tasks17–19 run records. Produces `GateInput`, `GateResult{passed:bool,failures:Vec<String>}`, `evaluate(&GateInput)->GateResult`, CLI `bench gate --report <path>`. Markdown includes a machine-readable JSON block with exact unrounded measurements so gates never parse rounded table cells.

- [ ] **Step 1 (4 min): Write inclusive-boundary and missing-evidence tests.**

```rust
#[test]
fn nearest_rank_p95_and_inclusive_bounds() {
    assert_eq!(percentile(&[1.0,2.0,3.0,4.0,10.0],0.95),Some(10.0));
    assert!(bound(Some(2.0),2.0));assert!(!bound(Some(2.000001),2.0));
    assert!(!bound(None,15.0));assert!(!bound(Some(f64::NAN),15.0));
}
```

- [ ] **Step 2 (2 min): Run red:** `cargo test -p bench --test gate`.
- [ ] **Step 3 (5 min): Implement non-rounded timing/accuracy aggregates.**

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
}
pub struct GateResult {pub passed:bool,pub failures:Vec<String>}
pub fn evaluate(g:&GateInput)->GateResult {
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
        ("required presolver has verified production entries",g.presolver_verified_entries>0),
    ];
    let failures=checks.into_iter().filter(|(_,ok)|!*ok).map(|(s,_)|s.into()).collect::<Vec<_>>();
    GateResult{passed:failures.is_empty(),failures}
}
```

Treat a nonzero final-delivery violation anywhere as a release failure, even when p95 passes; include counts separately for all runs, fault runs, and each street. SRP best_so_far within street budget is expected; late first terminal is still a street violation. Turn/river best_so_far is a street violation even before final delivery. The gate does not demand SRP raw50bp live in10s. It demands a validated terminal in budget, numericEV, measured raw reached accuracy, correct DeadlineBestSoFar reason, and pre-solved hit latency.

- [ ] **Step 4 (5 min): Extend report fields and evidence provenance.** Report path `docs/bench/<date>-i7-13700K.md` (first dated2026-09-10). Include CPU/RAM/OS, toolchain, commit/adapter/rules/proto versions, +avx2, source hashes/node-fallback inventory, fixture hashes, template signatures, thread count, cold/warm and f32/i16. Per baseline/store suite: p50/p95/max wall time to target50, separate time-to-target and admission-to-terminal/Final, memory_usage estimates, peak totalRSS, cancel latency, Exact/Approximate/Unsupported counts/proportions, raw reached exploitability, mode, first-terminal and final-delivery violation counts. Time-to-target is null/censored when target was not reached, never the deadline duration.

```rust
#[derive(serde::Serialize)]
pub struct AccuracyObservation {
    pub target_bp:u16,pub exploitability_over_p:f64,pub terminal_ms:u64,
    pub time_to_target_ms:Option<u64>,pub status:String,
}
```

Include V3 admission decision for min at both depths, forced-mode diagnostic flags, crossover probes, throughput1755×p50, tier counts and observed decision-log hit rates. Show V21 and V22 separately: V21 e2e p95<=15/zero final violations/every supported numeric fixture; V22 all baseline bounds and analytic values. V9 deferred: store section explicitly `not run: V9 deferred`, never zero-time pass or blocker for baseline. An acquired-data gate later uses identical bounds and pokerdata_* tests; failure keeps chart baseline.

Analytic acceptance: facing-all-in T1 AhAd on QsJd7h3c2d versus QQ+54o weight1/12 yields equity0.25, W246, callEV−11.5 unraked/−12.75 capped5000mchips;54o0.25 yields+50/+47.5;−2.30bb at BB5. Use1e−3 chip tolerance for exact analytic values. Polarized river: valuebetQQ100%, bluff54o50±3pp, call50±3pp, IP75±1/OOP25±1 chips, raw exploitability<=0.1%pot. Check-only/non-root contract tests retain their stated1e−3 tolerances. Plan2 supplies these tests; gate records their actual exit/results.

- [ ] **Step 5 (4 min): Mutation-test the gate data.** Construct a passing GateInput then independently fail each bound by a small positive increment, remove a matrix row, omit a fixture, change source hash, set supported_numeric false, add one final violation and censor a target time. Every mutation fails with its own named reason. Real missing files/runs cannot default to true. Exit0 only when passed, otherwise nonzero and retain report.

```rust
pub fn gate_exit(result:&GateResult)->i32 {if result.passed {0}else{1}}
```

A failure reduces versioned template size or declared scope and reruns affected measurements; it never loosens the success definition. Report tier1 progress honestly:7020 jobs are the four-scenario coverage condition, not a prerequisite to claim that the scheduler implementation works.

- [ ] **Step 6 (3 min): Run green:** `cargo test -p bench --test gate`; `cargo test --workspace`.
- [ ] **Step 7 (2 min): Commit.**

```powershell
git add crates/bench
git commit -m 'feat(bench): enforce measured V21 and V22 baseline gates' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

### Task 21: Run the complete validation matrix and record the baseline result

**Files:** Create `docs/bench/2026-09-10-i7-13700K.md`; modify generated `bench/spots/{flop_fast,flop_min,sources}.json` only if deterministic generation differs before measurement; no golden auto-update in this task.

**Interfaces:** Consumes all preceding tasks and Plan2 analytic/river/turn suites. Produces reviewed baseline report, V3 policy evidence and explicit V21/V22 outcome. Long measurements are commands split by suite/thread/mode/cold-warm; each checklist item starts or reviews one bounded batch, with progress tracked until it finishes.

- [ ] **Step 1 (3 min): Run deterministic red/green gate regression first.** Re-run Task20's deliberately failing metric fixture; expected nonzero. Then its valid synthetic fixture; expectedzero. These verify gate behavior, not measured product success.

```powershell
cargo test -p bench --test gate
cargo test -p cache
cargo test -p engine --test cache_key_structural_identity
cargo test -p engine --test cache_snapshot_replay
cargo test -p engine --test flop_path_golden
cargo test --workspace
python -m pytest tools/tests/test_e2e_hands.py -q
python tools/gen_fixtures.py e2e --check
```

- [ ] **Step 2 (3 min): Build production and diagnostic workers with pinned MSVC.**

```powershell
cargo build --release -p solver-worker
cargo build --release -p solver-worker --features bench-mode --target-dir target/bench-mode
cargo run --release -p bench -- gen-spots
```

Require ready reports avx2, expected versions and threads. Diagnostic override capability must not appear in normal worker. Refuse sources/inputs mismatching frozen hashes. Existing foundation's MSVC pin takes precedence over earlier GNU feasibility notes; no rustup default/config mutation.

- [ ] **Step 3 (3 min per launch/review): Run exact-template flop matrix.** CLI mode/cold switches are diagnostic extensions from Task17; each command creates separate raw rows in the report's machine-readable block. Reps5 in each cold/warm cell, threads8/16/24; then production-auto-mode deadline trials.

```powershell
foreach ($threadCount in @(8,16,24)) {
    foreach ($suiteName in @('flop_fast','flop_min')) {
        foreach ($storageMode in @('f32','i16')) {
            foreach ($temperature in @('cold','warm')) {
                cargo run --release -p bench -- run --suite $suiteName --threads $threadCount --reps 5 --mode $storageMode --temperature $temperature --out docs/bench/
                if ($LASTEXITCODE -ne 0) { throw 'Flop measurement failed; preserve report and inspect the failed cell.' }
            }
        }
    }
}
cargo run --release -p bench -- run --suite flop_fast --threads 16 --reps 5 --mode auto --deadline-trials --out docs/bench/
cargo run --release -p bench -- run --suite flop_min --threads 16 --reps 5 --mode auto --deadline-trials --out docs/bench/
```

Do not replace missing measurements with R8 analogue numbers. Recompute V3 admission from exact production source signatures and both depths. Record refusals and investigate if required diagnostic matrix incomplete; no success claim with missing rows.

- [ ] **Step 4 (3 min per launch/review): Measure baseline, faults and pre-solved hits.**

```powershell
foreach ($suiteName in @('river_std','river_min','turn_std','turn_min')) {
    cargo run --release -p bench -- run --suite $suiteName --threads 16 --reps 5 --out docs/bench/
}
cargo run --release -p bench -- e2e --reps 5 --cache cold --out docs/bench/
cargo run --release -p bench -- e2e --reps 5 --cache presolved --out docs/bench/
cargo run --release -p bench -- fault --out docs/bench/
cargo run --release -p bench -- oracle
cargo run --release -p bench -- gate --report docs/bench/2026-09-10-i7-13700K.md
```

`--cache presolved` creates validated at-target entries using the real presolver executor, not fabricated files. Long suite runs may outlive a shell wait; keep progress/logs and resume the same process. Stop on failed correctness assertions; timing failures remain reported and require a versioned change or scoped release decision, never fabricated passes. Gate runs remain conditional until measured.

- [ ] **Step 5 (4 min): Review evidence and run final workspace check.** Verify supported22/total50, every prescribed fault in all required phases, zero final-delivery violations, raw versus display accuracy, source locks, V3 min-admission choice, mode switch points, presolver entry durability/status and hit-rate denominators. Record actual tier1 progress; do not wait52h merely to test scheduler completion logic. Confirm cache/queue/log failures never altered a delivered recommendation. Keep measured failures visible in report.

```powershell
cargo test --workspace
git diff --check
```

- [ ] **Step 6 (2 min): Commit the reviewed report and deterministic artifacts.**

```powershell
git add docs/bench/2026-09-10-i7-13700K.md bench/spots
git commit -m 'test(bench): record exact-template baseline and release-gate evidence' -m 'Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>'
```

## Self-review

| Spec requirement | Tasks |
|---|---|
|§2 normalized identity, public ranges, canonical suits, ordinal actors|1–3,7–8|
|§3.2 dependency direction and process boundary|1,7,12–15,17–19|
|§4.2 flop preference default10/range1..30; approval outline§0b|9–10|
|§4.4 reasons, phase/identity and numericEV assembly|4,7,10–11,18–19|
|§4.5 validate_solution, output bounds, actor-owned matrices|2,5,7–8,19|
|§4.6 full materialization/terminal markers/signature|1–3,7–8,17|
|§5 step7 cache->Final/Provisional/live; flop/turn store, river excluded|7,9–11|
|§5 step10 decision-log scenario/tier, raw accuracy and violations|10,15,18,20|
|§7 monotonic absolute deadlines, independent watchdog, suspend/resume|7,9–10,14,19–21|
|§9 cache snapshot registration, prior-street translation and prefix reuse|10–11|
|§10.1 exact templates, V3 admission and SRP/3-bet policy|9–10,17,20–21|
|§10.3 same2GiB/8GiB/headroom mode rule in live/background|10,15,17|
|§10.4 key, payload, exact labels/raw accuracy, three buckets, full topology/rake/menu checks|1–8|
|§10.4 two representatives, bounded checksummed codec, delete, atomic writer/quota|5–7|
|§10.4 T4 scale numbers and every listed identity/actor/path mutation|8,11|
|§10.5 exact scenarios/tiers/flop order, durable cursor, retries, idle/cancel, replay ranges/status|12–15|
|§12 read/write/worker/clock failures and retained evidence|5–7,10,13–15,19|
|§13.1 five named cache tests|4–8|
|§13.3 flop-path/budget/best-so-far/snapshot goldens|8–11,19|
|§13.5 chart locks, actual50 inputs, gen-spots, flop suites, e2e/fault/report/gate|16–21|
|§14.4 V3/V21/V22 exact baseline bounds|17–21|

**Placeholder scan:** No unresolved implementation markers found. Task numbering is sequential, every task has Files/Interfaces, red/green steps and one required-trailer commit, and code fences are balanced. This review checks plan text; implementation commands have not been run while writing it. Fixtures/source locks are generated and committed before measurements; their expected outcomes are not derived from benchmark results. Storage tests use the Task 2 fixture immediately, so they do not depend on later T4 materialization. Bench module exports are introduced only when their source files exist.

**Type consistency:** `CacheEntry` stores ordinal `CachedNode.path` and `ev_over_P`; wire `NodeStrategy` retains chip paths and `ev_chips`; `CacheHit` reconstructs query chips and inverse suits before validation/snapshot registration. `Range1326` stays public. `StreetSnapshot` is the exact core-replay type. `PresolveExecutor` is a downward-dependency callback, distinct from the existing WorkerLink. `ApproxReason` names and fields retain spec spelling. No target/hero/request/seat/raw-chip fields enter KeyFields. JSON metadata avoids bincode's internally tagged-enum incompatibility. Mode forcing is diagnostic-only and cannot alter the production wire or default admission.

**Coverage gaps:** No planned omission within Plan4's scope. The actual MSVC build, source-bundle audit/hero support, V3 timings, fifty-hand outcomes, V21/V22 pass status and tier1 coverage remain implementation-time evidence, not claims made by this planning document. V9 acquisition/converter, exploit slice, UI/WebDriver and completed tiers2/3 are intentionally owned elsewhere or deferred. Tier1 completion is reported as the coverage condition, while resumable tier1 execution is delivered here.
