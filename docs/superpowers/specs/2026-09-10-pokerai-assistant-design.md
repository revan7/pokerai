# PokerAI assistant: design specification (2026-09-10, revision 3)

Source of truth for decisions: `docs/design/2026-09-10-design-outline.md` (including its three amendments of today: cache exactness and the 2% SPR rule, the multiway street-root projection rule, Tauri as the only shell with the MSVC prerequisite). Facts: `docs/research/SYNTHESIS.md` as corrected by `docs/research/REVIEW-of-research.md`; preflop data facts from `docs/research/R7-pokerdata-verification.md`; solver facts measured on the target machine (i7-13700K, 64 GB, Windows 11) from `docs/research/R8-solver-bench.md`; the pinned upstream `src/action_tree.rs` and `src/game/interpreter.rs` were read at the pinned commit for sections 4.6 and 10.3. Review findings applied from `docs/research/REVIEW-of-spec-1.md` and `docs/research/REVIEW-of-spec-2.md`; dispositions in `docs/research/SPEC-CHANGELOG-1.md` and `SPEC-CHANGELOG-2.md`. Items the outline leaves unspecified are chosen here and marked **(spec decision)**. A figure marked "measured" comes from R8; every other latency or memory figure is a hypothesis until `bench` measures it. No fixture, benchmark report or test artifact exists yet: every file under `fixtures/`, `bench/spots/`, `engine/tests/golden/` and `docs/bench/` named below is **to be created** in the week stated in section 13.0, and the acceptance data it must contain is given in this document.

---

## 1. Purpose and scope

### 1.1 Goal
For each hero decision point in a 6-max NLHE cash hand entered by hand at a live table, return within a 15 s hard deadline (measured from admission of the `recommend` call, human entry excluded):
- the legal actions, with a frequency and an incremental EV in big blinds (fold = 0) wherever the coverage rules of section 6 provide them,
- a coverage label (Exact / Approximate / Unsupported) with the reasons,
- the assumptions used (ranges, tree, source accuracy, accuracy reached, elapsed time, translations, mappings),
- progressively: a fast answer first (legal actions, equity when ready), then the solver answer when ready.

GTO baseline first. Exploit slice (section 11) only after the baseline is validated.

### 1.2 Non-goals (phase 1)
Screen capture; tournaments/ICM; solved EV for 3+ pot-eligible players; GPU solving; neural models; phone/LAN UI; distribution to anyone else; antes **(spec decision: not in the game config)**; button/Mississippi straddles and re-straddles **(spec decision: only one fully posted UTG straddle is modelled)**; two-handed (two dealt seats) hands **(spec decision: `FormatUnsupported`, section 4.3)**; bunching of folded players' cards **(spec decision: fixed off, section 10.3)**; decision-root solves with externally conditioned ranges (every main-path solve is rooted at the street root, section 10.2; the only exception is the isolated experimental surrogate of section 6, which is never in `actions`); any multiway-to-HU projection other than the outline's exact-reproduction rule (section 10.2); straddle and 250bb preflop trees (phase 2 per the outline); full flop-library completion (section 10.5); a second shell architecture (Tauri only, section 3.6).

### 1.3 User decisions (final, from the outline)
| Decision | Value |
|---|---|
| Approach | A: solver-core assistant, HU postflop solver + preflop store |
| Format | 6-max NLHE cash, manual entry, Windows 11 desktop, private use only |
| Preflop data | PokerData one-time packs (MonkerSolver provenance verified in R7; 8 depths 20-200bb; 5% rake, 0.5bb cap; 169-class frequencies + per-action EV in source small blinds). Purchase and the rights email are the user's own actions (V9). Fallback = transcribed free charts labelled "rounded, no EV" |
| Game config | Per session: blinds, UTG straddle, rake (pot rake with cap) or time charge, seats and stacks; frozen per hand (section 4.2) |
| Week-1 targets | 100bb and 200bb, 5% pot rake with cap, UTG straddle |
| Budget | Strong baseline in 1-2 weeks, then iterate |
| AGPL | Private use imposes no obligations; solver still isolated in its own process |

---

## 2. Definitions

- **Decision point**: a state where `Derived.to_act == hero`, hero has two legal hero cards on record and at least two legal actions. Anything else is **NoDecision** (hero all-in, hero folded, another seat to act, one legal action, hero cards unknown, hand complete) and produces no request.
- **Street root**: the state at the start of the current street, before its first action. **`StreetRootSnapshot`** (section 4.3) is the *financial* snapshot only: board, the two pot-eligible seats with their OOP/IP roles, the matched pot at the root, dead money, both remaining stacks at the root, and the ordered **history** of current-street actions. Ranges are not part of it; **`SolveInput`** (section 4.3) joins a snapshot with the two public ranges produced by replay (section 9) and the effective tree (section 4.6). Worker pot and stacks come only from the snapshot; `Derived` describes the decision point and is used for legal-action validation and for the EV reference. A street that started with 3+ pot-eligible players is projected to a HU root only under the exact-reproduction rule of section 10.2.
- **Pot-eligible player**: a player who has not folded, including all-in players. **HU spot**: exactly 2 pot-eligible players at the decision point. A third player who is all-in makes the spot non-HU.
- **Positions**: clockwise from the button: BTN, SB, BB, UTG, HJ, CO. Preflop order without straddle: UTG, HJ, CO, BTN, SB, BB. With the UTG straddle: HJ, CO, BTN, SB, BB, UTG (the straddler keeps the physical name UTG). Postflop order: SB, BB, UTG, HJ, CO, BTN, skipping folded and all-in seats. **OOP** = the pot-eligible player earliest in postflop order.
- **Effective stack**: min of the two HU players' remaining stacks at the street root; the deeper stack's excess is uncontestable and is excluded from the strategic pot model. **Depth** (preflop lookups): section 8.3, measured per node prefix in source-blind units.
- **Money units**: all `HandState`, worker and cache wager amounts (blinds, stacks, bets, pots) are integer chips (`u32`), one chip = the session's accounting tick and legal wager quantum (`GameConfig.chip_label` names its value, "$1" or "$5") **(spec decision)**. The modelled rake cap is the one fractional monetary quantity: it is carried end to end as integer thousandths of a chip (`cap_mchips: u32`; 0.5 bb at a 5-chip BB = 2.5 chips = `2500`) and converted to `f64` chips only at the library boundary **(spec decision)**. EV and modelled rake amounts are signed finite `f32` chips. Display: `ev_bb = ev_chips / bb_chips` rounded to 0.01 bb at render time only. Nothing is rounded to fit a stored node. Sums are checked (`pot + stacks < 2^31` for the library's signed inputs); overflow is `EngineError`.
- **Bet size (% pot)**: `s = (chips added beyond a call) / (pot after the bettor's call)`. An opening bet has call = 0. Raises are entered as **raise-to** amounts = the actor's total contribution on this street; the paid delta is `to - previous contribution`. Template raise sizes are multiples of the facing wager (`2.5x`) **(spec decision: matches the trees measured in R8)**.
- **EV convention**: `EV(a) = E[hero's final stack | a] - hero's stack at the decision point`, in chips. Fold = 0 exactly; chips already in the pot are sunk. The same reference holds for check. Rake is subtracted at the terminals that pay it.
- **Coverage labels** (about input/model matching, never a claim of full-game GTO):
  - `Exact`: every input matched the declared model identically (node present, sizes in tree, depth equal, rake profile equal, requested accuracy reached on the raw exploitability, no translation, no bucketing of any kind, no rounding of any menu size). A cache hit is `Exact` only for identical normalized inputs (outline §3 as amended; section 10.4). The source's own accuracy is reported in assumptions (`source_accuracy = unverified` for PokerData until R7 A6 supplies exploitability).
  - `Approximate{reasons}`: one or more of `BetTranslation`, `DepthBucket`, `AsymmetricStacks`, `RakeProfileMapped`, `StraddleMapped`, `ShortHandedMapped`, `DeadlineBestSoFar`, `ChartRounded`, `EvReferenceUnverified`, `UnconditionedPriorStreet`, `UnconditionedCurrentStreet`, `MultiwayStreetRoot`, `SprBucketed`, `MenuRounded`, `BranchResidual`. Only reasons actually incurred are emitted; a reason is never emitted from a worst-case bound. Approximate does not promise a numeric EV (charts have none).
  - `Unsupported{reason}`: no numeric EV; equity where computable. Reasons: `MultiwayEv`, `MissingPreflopNode`, `HeroComboOutOfSupport`, `EngineError`, `TreeTooLarge`, `DeadlineExceeded`, `InvalidRanges`, `UnsupportedHistory`, `FormatUnsupported`.
- **Public range**: a seat's strategic range conditioned on public information only (its own observed actions, the board). Hero's actual cards are never applied to any public range, solve input or cache key. **Hero-conditioned copies** of opposing ranges (hero's cards removed) exist only for hero-combo equity and terminal call calculations. **`HeroComboOutOfSupport`**: hero's actual combo has zero weight in hero's public range at the decision node; equity and the range-level mix are shown, no per-combo strategy or EV is fabricated.
- **Canonical board**: the flop is canonicalized as an unordered set under the 24 suit permutations (1,755 classes: 13 trips orbit 4, 312 paired orbit 12, 286 distinct-rank patterns × 5 suit patterns: monotone orbit 4, three two-tone placements orbit 12 each, rainbow orbit 24; total 22,100). Turn and river are appended in dealt order and canonicalized within the flop's stabilizer. Among permutations giving the same canonical board, the one producing the lexicographically minimal serialized `(oop, ip)` public range tuple is chosen, then the lexicographically minimal permutation **(spec decision)**. A class absent from the cache is solved live; no "nearest flop" substitution, ever.
- **Tree template / effective tree**: section 10.1 / 10.2 and `EffectiveTree` in section 4.6.
- **Bet translation**: section 8.4 (likelihood interpolation and node translation, boundaries, prominence rule).
- **Range**: `Range1326`, a newtype over `[f32; 1326]`, weights in `[0, 1]`, indexed by combo index (section 4.1); on the wire a JSON array of exactly 1326 finite numbers, validated at every boundary. **Range hash** (`hash_scaled`): after suit canonicalization and board blocking, every weight is divided (one IEEE `f32` division each) by the range's maximum weight, and the resulting 1326 `f32` bit patterns are hashed with sha256. The normalization is deterministic and bit-exact; two ranges hash equal only when their normalized bit patterns are identical. A rescaling by a power of two is exactly invariant; any other rescaling may or may not be, and a differing hash is simply a miss (no lossy quantization is ever applied to make ranges match) **(spec decision)**. A range with a single supported combo normalizes to weight 1.0 whatever its magnitude.
- **Materialized tree and ordinal node path**: the effective tree is materialized by the engine as the explicit list of action nodes of every street with their actions in the deterministic order of section 4.6. A node is identified by its **ordinal path**: the sequence of action indices from the street root in that order. Two materialized trees have **equal topology** when they contain the same ordinal paths with the same action kinds at every index; they may differ in chip amounts. Wire messages carry chip actions; the engine, the cache and replay identify nodes by ordinal path (sections 9, 10.4).
- **Decision identity**: `(hand_id, hand_revision, decision_id, config_revision, model_revision)` (section 4.4). Every event, replay snapshot and result carries it; anything not matching the active identity is discarded.

---

## 3. Architecture

### 3.1 Processes
| Process | Technology | Role |
|---|---|---|
| `pokerai-ui` | Tauri 2 app: Rust shell crate `pokerai-app` + Vite/React/TypeScript frontend (the only shell architecture in phase 1; prerequisite in section 3.6) | Keyboard-first entry, display, undo; hosts the `engine` in-process |
| `solver-worker` | Separate Rust binary (AGPL-3.0) | Wraps b-inary/postflop-solver at commit `9d1509fe5077d019825f833eed04b16d342dfda1`; JSON lines over stdio; one job at a time |
| `tools/` | Python 3.12, dev only | Test oracles (phevaluator, PokerKit), chart ingestion, PokerData pack conversion, bench report scripts. Never a runtime dependency |

The engine owns the worker process (spawn with `std::process::Command`, launch argument `--threads N`, keep alive between requests as a warm process, restart on exit or protocol error). `pokerai-app` only delegates start/stop to the engine.

### 3.2 Crates
| Crate | Purpose | Depends on | License |
|---|---|---|---|
| `proto` | Serde types: `Card`, `GameConfig`, `HandConfig`, `HandState`, `Action`, `StreetRootSnapshot`, `Recommendation`, `Coverage`, `RecommendationEvent`, `EffectiveTree`, worker messages. Single source of truth for UI (TS bindings via `ts-rs`), engine and worker | `serde` | MIT OR Apache-2.0 |
| `core-model` | "AsKd" parser and display; positions and action order incl. the UTG straddle; hand lifecycle and state machine (streets, matched pot, uncalled-bet return, side pots, all-ins, cumulative reopening, street closure); legal actions; `StreetRootSnapshot` derivation | `proto` | MIT OR Apache-2.0 |
| `core-ranges` | Pio-style range strings with weights to and from `Range1326`; 169-class to 1326 expansion (6/4/12); public blocking (board), hero-conditioned copies; merge/scale; range hashing | `proto` | MIT OR Apache-2.0 |
| `core-iso` | Board canonicalization with stabilizer tie-break; suit permutation and inverse applied to combo-indexed vectors | `proto`, `core-ranges` | MIT OR Apache-2.0 |
| `core-eval` | 7-card evaluator (b-inary holdem-hand-evaluator, MIT; rs-poker is the named contingency only if it fails V1, not implemented otherwise); weighted range-vs-range equity via `pokers` (MIT): exact enumeration and time-bounded Monte Carlo with joint disjoint sampling; per-pot shares | `proto`, `core-ranges` | MIT OR Apache-2.0 |
| `core-preflop` | `PreflopSource` trait; adapters `PokerDataJson` and `ChartTranscription`; bundle validation; per-prefix depth/rake/straddle/short-handed mapping; translation | `proto`, `core-ranges`, `core-model` | MIT OR Apache-2.0 |
| `core-replay` | Bayesian public-range replay for every seat with branch-weighted translation and snapshot compatibility | `proto`, `core-ranges`, `core-model`, `core-preflop` | MIT OR Apache-2.0 |
| `cache` | Pot-normalized on-disk store of street solutions (section 10.4); background pre-solver queue | `proto`, `core-iso`, `core-ranges` | MIT OR Apache-2.0 |
| `engine` | Coverage classifier; effective-tree builder; cache lookup; worker client (admission, absolute deadlines, watchdog, cancel, restart); result assembly; decision log | all `core-*`, `cache`, `proto` | MIT OR Apache-2.0 |
| `bench` | Drives the worker and the engine on fixed suites; markdown report; release gate | `proto`, `engine` | MIT OR Apache-2.0 |
| `solver-worker` | Binary; protocol state machine; adapter to postflop-solver | `proto`; links `third_party/postflop-solver` | AGPL-3.0 |
| `pokerai-app` | Tauri shell: commands, `Channel<RecommendationEvent>`, config persistence | `engine`, `proto`, `tauri` | MIT OR Apache-2.0 |

Dependency direction is strictly downward: `proto` <- `core-*` <- `cache`/`engine` <- `bench`/`pokerai-app`. `solver-worker`'s only project dependency is `proto`; it additionally links the vendored solver. No crate depends on `solver-worker`.

Repository layout **(spec decision)**:
```
Cargo.toml                      workspace; .cargo/config.toml sets target-feature=+avx2 (section 3.7)
crates/{proto,core-model,core-ranges,core-iso,core-eval,core-preflop,core-replay,cache,engine,bench}
solver-worker/                  AGPL crate (workspace member, separate binary, build.rs checks avx2)
third_party/postflop-solver/    vendored fork at the pinned commit, LICENSE retained, PINNED_COMMIT, PATCHES.md
apps/pokerai-ui/                Vite frontend + src-tauri (crate pokerai-app)
tools/                          Python 3.12 (pyproject.toml), dev only
fixtures/                       committed test fixtures (hands, eval oracles, charts, worker wire, e2e suite)
docs/
```

### 3.3 AGPL boundary
- `solver-worker` is the only crate that links AGPL code; its `Cargo.toml` declares `license = "AGPL-3.0"`. All other crates are `MIT OR Apache-2.0`.
- The boundary is a process boundary with a documented JSON-lines protocol (`proto::worker`). `proto` is permissive; AGPL code may consume it.
- Private use creates no obligation. `third_party/postflop-solver/LICENSE`, `PINNED_COMMIT` and `PATCHES.md` are kept so a Corresponding Source archive can be produced if the app is ever shared or served. That review is a phase-2 item.

### 3.4 Threading model
- Tauri commands never block: `recommend` enqueues and returns; results flow through `Channel<RecommendationEvent>`.
- Engine threads (std threads, never Tokio workers) **(spec decision)**: `engine-main` (request queue of depth 1 holding the latest live request, admission, identity, cancellation, worker client state); `fast-path` (validation, replay, equity with its own cancellable budget); `watchdog` (absolute deadlines, section 7); `worker-stdin` / `worker-stdout` (blocking pipe I/O); `worker-stderr` (bounded drain to the log); `cache-writer` (atomic writes off the recommendation path); `presolver` (section 10.5).
- Worker threads: `control` (reads stdin continuously, independent of the solve), `writer` (single serialized stdout writer over a bounded channel), `executor` (owns the game object, runs the rayon pool). Rayon threads: 16 by launch argument **(spec decision; measured: 16 threads equal 24 within noise, 8 threads 1.35x slower, the solve is memory-bandwidth bound)**. The worker sets its own Windows priority class to BELOW_NORMAL while a `background: true` job runs and NORMAL otherwise **(spec decision)**.

### 3.5 Component interfaces (public Rust surface; sketches, illustrative where marked)
| Component | Interface |
|---|---|
| `core-model` | `parse_card`, `parse_hand`, `begin_hand(&HandConfig, BeginHand) -> Result<HandState>`, `apply_action(&HandState, Action) -> Result<HandState, RulesError>`, `set_board(&HandState, &[Card]) -> Result<HandState>`, `derive(&HandState) -> Derived`, `street_root(&HandState) -> Result<StreetRootSnapshot, RootError>` (`RootError::{Multiway, ProjectionNotReproducing{step}, NoDecision}`; the projection rule is in section 10.2), `replay_root(&StreetRootSnapshot) -> Result<Derived, RulesError>` (replays `history` from the root as a HU street), `settle_pots(&HandState) -> Settlement` |
| `core-ranges` | `parse_range`, `range_to_string`, `expand_169(&[f32; 169]) -> Range1326`, `block_public(&mut Range1326, board)`, `hero_conditioned(&Range1326, hero: [Card; 2]) -> Range1326`, `mass`, `hash_scaled(&Range1326) -> [u8; 32]` |
| `core-iso` | `canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm)`, `orbit_size`, `apply(&SuitPerm, Card)`, `apply_range(&SuitPerm, &Range1326)`, `inverse` |
| `core-eval` | `rank7`, `equity(&EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult` (request: ranges or fixed hero combo, board, mode Exact/MonteCarlo{seed}, per-pot eligibility) |
| `core-preflop` | section 8.1 |
| `core-replay` | section 9.1 |
| `cache` | `Cache::open(dir, quota) -> Cache`, `lookup(&CacheQuery) -> Lookup` (Exact / Approximate{reasons} / Provisional{reasons} / Miss), `store(&CacheEntry)`, `Presolver::{start, pause, resume, status}` |
| `engine` | `Engine::new(GameConfig, Paths)`, `set_config(GameConfig) -> config_revision`, `begin_hand(..) -> HandState`, `apply_action`, `set_board`, `undo() -> HandState`, `recommend(sink) -> DecisionIdentity`, `cancel(decision_id)`, `finish_hand()`, `abandon_hand()`, `shutdown()`; internal: `build_effective_tree(&SolveInput) -> Result<EffectiveTree, UnsupportedReason>` (materializes every street with the pinned rules of section 4.6), `register_snapshot(&DecisionIdentity, StreetSnapshot)` (single registration path for live results and cache hits, section 9.2) |
| `pokerai-app` (Tauri commands) | `set_game_config`, `begin_hand`, `set_hero_cards`, `apply_action`, `set_board`, `undo`, `recommend(on_event: Channel<RecommendationEvent>)`, `cancel`, `finish_hand`, `abandon_hand`, `set_seat_tag`, `presolver_status/pause/resume`. Undo is a command so the engine assigns the revision and invalidates work |
| `bench` | `bench run --suite {river_std,river_min,turn_std,turn_min,flop_fast,flop_min,e2e,fault} --threads N --reps R --out docs/bench/` |
| `solver-worker` | stdin/stdout JSON lines (section 4.5); exit code 0 on `shutdown` or stdin EOF, non-zero on panic |

TypeScript types for every `proto` type are generated at build time (`ts-rs` **(spec decision)**).

### 3.6 Environment prerequisites
- Rust stable 1.95 or newer. The active rustup toolchain on this machine is `stable-x86_64-pc-windows-gnu`; `stable-x86_64-pc-windows-msvc` is installed but Visual Studio 2022 has no C++ build tools (no `cl.exe`, `link.exe`) (measured, R8 §5).
- Tauri 2 requires the MSVC target. **Prerequisite (outline §2): the user installs the Visual Studio 2022 "Desktop development with C++" workload before UI work starts (user action, pending).** `pokerai-app` and the E2E test build with `stable-x86_64-pc-windows-msvc`; there is no second shell architecture in phase 1 (the contingency if the install is refused is recorded once, in section 16, and is not a contract of this design).
- `solver-worker` and every non-Tauri crate build on either toolchain (R8 built the solver, its examples, `zstd-sys` and the bench FFI with the GNU toolchain); week-1 core and worker work proceeds on the GNU toolchain while the workload is pending, and the whole workspace is built once with MSVC before the E2E test.
- Node 24; WebView2 present on Windows 11; Python 3.12 only for `tools/`.

### 3.7 Solver build requirements
- Pinned upstream commit `9d1509fe5077d019825f833eed04b16d342dfda1` (2023-10-01, "docs: update README"; upstream development suspended). Features: default (`bincode` + `rayon`) plus `zstd`; `custom-alloc` is nightly-only and not used.
- Vendored patches recorded in `third_party/postflop-solver/PATCHES.md`: (1) `Cargo.toml` pins `bincode = "=2.0.0-rc.3"` and `bincode_derive = "=2.0.0-rc.3"` (a fresh lockfile otherwise resolves 2.0.1 and fails to compile); (2) `src/action_tree.rs` lines 393/396/408: `&*(*node).children[i].lock()` becomes `&*(&(*node).children)[i].lock()` for the Rust 1.95 deny-by-default lint `dangerous_implicit_autorefs`.
- AVX2 is a build requirement: the default codegen leaves the hot loops in 128-bit SSE2 (measured: reduced flop 11.4 s SSE2 versus 6.2 s AVX2). `.cargo/config.toml` sets `rustflags = ["-C", "target-feature=+avx2"]` for `x86_64-pc-windows-gnu` and `x86_64-pc-windows-msvc`; `solver-worker/build.rs` fails the build when `CARGO_CFG_TARGET_FEATURE` lacks `avx2`. At startup the worker reports `build_features` (from `cfg!(target_feature)`) and `cpu_features` (from `is_x86_feature_detected!`) in `ready`; the engine refuses a worker whose `build_features` lacks `avx2` (`EngineError("worker built without AVX2")`) and shows a startup banner when the CPU lacks it. FMA is not required (measured: no FMA instructions emitted in either build).
- Library lifecycle constraints (measured in R8, binding on the adapter): locks are applied after `allocate_memory` and before `finalize()`; nothing can be solved after `finalize()` (a re-lock means `allocate_memory` again and a re-solve from scratch); `solve_step` granularity is one iteration (0.08-0.3 s on flop trees) with no in-iteration cancel, so deadline and cancel checks happen between iterations; `compute_exploitability` costs about one iteration; `cache_normalized_weights()` must be called after every navigation (`play`, `back_to_root`, `apply_history`) before reading EVs or the library panics; tree build is 1-70 ms, `allocate_memory` 0-10 ms, `finalize` 0.2-0.35 s on flop trees; `memory_usage()` is within 1.3% of the measured peak working set.

---

## 4. Data model (Rust-like sketches; `proto` unless noted)

### 4.1 Cards, combos, seats
```rust
pub struct Card(pub u8);            // id = rank_index*4 + suit_index; ranks 2..A = 0..12; suits c,d,h,s = 0..3 (spec decision)
pub type ComboIndex = u16;          // for cards lo < hi: idx = hi*(hi-1)/2 + lo, 0..1326 (spec decision)
pub struct Range1326(pub [f32; 1326]);
pub enum Position { Btn, Sb, Bb, Utg, Hj, Co }
pub struct Seat(pub u8);            // 0..5, clockwise; button seat given per hand
pub enum Street { Preflop, Flop, Turn, River }
```
169-class order **(spec decision)**: 13×13 grid, row-major from A down to 2; `i == j` pair, `i < j` suited, `i > j` offsuit; `class = i*13 + j`. The worker adapter never assumes the library shares any ordering; it maps by named combos (section 10.3).

### 4.2 Game config (per session) and hand config (frozen per hand)
```rust
pub struct GameConfig {
    pub config_revision: u32,                        // assigned by the engine on every set_config
    pub chip_label: String,                          // display only, "$1"; all amounts are chips
    pub sb_chips: u32, pub bb_chips: u32,
    pub straddle: Option<UtgStraddle>,               // amount_chips >= 2*bb_chips, fully posted, six dealt seats (section 8.3)
    pub rake: Rake,                                  // PotRake{rate: f32, cap_mchips: u32 /* thousandths of a chip, section 2 */, no_flop_no_drop: bool} | TimeCharge (solved unraked)
    pub seats: Vec<SeatConfig>,                      // seat, tag: Option<SeatTag>, facts: Vec<QuickFact>
    pub solver: SolverPrefs,                         // threads: u8 = 16, target_bp: u16 = 50; no bunching field in phase 1
}
pub struct HandConfig { pub config_revision: u32, pub sb_chips: u32, pub bb_chips: u32, pub straddle: Option<UtgStraddle>, pub rake: Rake, pub chip_label: String }
```
`HandConfig` is copied into `HandState` at `begin_hand`; a session `set_config` takes effect from the next hand. Correcting the active hand's config is not supported; abandon and re-enter **(spec decision)**.

### 4.3 Hand state, lifecycle, settlement, street root
```rust
pub enum Action { Fold, Check, Call, Bet { to: u32 }, Raise { to: u32 }, AllIn { to: u32 } }   // to = total street contribution
pub struct TakenAction { pub seat: Seat, pub street: Street, pub action: Action, pub paid: u32 }
pub enum HandPhase { Betting { street: Street }, AwaitingBoard { street: Street }, Complete { reason: CompleteReason /* FoldedOut | AllInRunout | ShowdownReached */ }, Abandoned }
pub struct HandState {
    pub hand_id: u64, pub hand_revision: u32,        // revision assigned by the engine from a session counter; never reused, not stored in undo snapshots
    pub config: HandConfig, pub phase: HandPhase,
    pub button: Seat, pub hero: Seat, pub hero_cards: Option<[Card; 2]>,
    pub dealt: Vec<Seat>, pub stacks_start: Vec<u32>,  // per dealt seat, confirmed by the user at begin_hand
    pub board: Vec<Card>,                            // 0, 3, 4 or 5 cards
    pub actions: Vec<TakenAction>,                   // posts are implied by HandConfig
    pub derived: Derived,
}
pub struct Derived {
    pub street: Street, pub to_act: Option<Seat>,
    pub pot: u32,                                    // all chips committed so far, all streets, before uncalled returns
    pub committed_this_street: Vec<u32>, pub stacks_remaining: Vec<u32>,
    pub folded: Vec<bool>, pub all_in: Vec<bool>,
    pub facing: u32, pub last_full_raise: u32,       // for cumulative reopening
    pub pots: Vec<Pot>,                              // settled pots: amount, eligible seats (uncalled money already returned)
    pub legal: Vec<LegalAction>,                     // Fold | Check | Call{cost} | Bet{min_to,max_to} | Raise{min_to,max_to} | AllIn{to}
}
pub struct StreetRootSnapshot {                      // financial snapshot only; no ranges (section 2)
    pub street: Street, pub board: Vec<Card>,
    pub oop: Seat, pub ip: Seat,
    pub pot_root: u32,                               // matched pot before this street's first action, including dead money from players folded on earlier streets
    pub stack_oop_root: u32, pub stack_ip_root: u32,
    pub dead_this_street: u32,                       // contributions this street by players folded on this street, present only under the projection rule of section 10.2
    pub projected_from: u8,                          // pot-eligible players at the street start (2 = genuine HU root; 3+ = projected, MultiwayStreetRoot)
    pub history: Vec<(Seat, Action)>,                // ordered current-street actions of oop/ip only
}
pub struct SolveInput { pub root: StreetRootSnapshot, pub ranges: [Range1326; 2] /* oop, ip public ranges at the root, from replay */, pub tree: EffectiveTree, pub target_bp: u16 }
```
**Dealt seats (spec decision)**: 3 to 6 dealt seats are supported. BTN, SB and BB are the button and the next two dealt seats clockwise; the remaining `n - 3` dealt seats take the *last* `n - 3` names of the sequence UTG, HJ, CO in clockwise order (4 dealt seats: CO; 5: HJ, CO). Preflop order is the non-blind seats clockwise from the seat after BB, then BTN, SB, BB; postflop order is SB, BB, then the others clockwise. The straddle requires six dealt seats. Two dealt seats are `FormatUnsupported{detail: "two dealt seats"}` (the button would have to post the SB and act first; not modelled in phase 1). These are the actual rules; the lookup-only `ShortHandedMapped` mapping of section 8.3 is separate.

Lifecycle: `begin_hand` (button, hero, dealt seats, per-seat starting stacks confirmed by the user; the UI prefills each stack with the previous hand's start minus its committed chips and marks it unconfirmed until Enter **(spec decision)**) -> `Betting{Preflop}` -> street closure. **A street closes** when every pot-eligible player has either matched the highest wager, is all-in, or has folded, and no player still owes a response; then uncalled portions are returned and pots are settled (below). After closure: `Complete{FoldedOut}` when one pot-eligible player remains; `Complete{AllInRunout}` when **fewer than two** pot-eligible players still have chips (all remaining all-in, or exactly one survivor retains chips after calling a shorter all-in), whatever the street; `Complete{ShowdownReached}` after river closure with two or more players holding chips; otherwise `AwaitingBoard{next}` -> `Betting{next}` after `set_board`. `finish_hand` / `abandon_hand` at any time. Completion and abandonment cancel outstanding work, invalidate every descendant identity and start the idle timer; no board or showdown entry is required after `AllInRunout` (next-hand stacks are confirmed at `begin_hand`). `set_board` replaces the full validated board (3, 4 or 5 cards) and is legal only in `AwaitingBoard`; corrections go through `undo`. Every state reachable by a legal action has exactly one of these transitions **(spec decision)**.

Settlement rules (`core-model`): at street closure an uncalled bet or raise portion is returned to its owner (a transfer from live commitment back to the stack, never an extra term), then pots are layered by contribution levels with their eligible seats. **Conservation invariant**, checked after every action, every refund and every settlement: `sum(stacks_remaining) + live_commitments + sum(unawarded pot chips) + rake_collected = sum(stacks_start)`, where `live_commitments` are the current street's contributions not yet settled into pots (a bet still awaiting a response counts exactly once, here, and is neither refunded early nor added to a pot until closure) and `rake_collected` is 0 in the model (rake is applied only inside the solver's terminal payoffs). Awards and returns move chips between these terms and never change the sum. Rake is modelled by the solver on matched terminal pots (`rate * pot` capped at `cap_mchips / 1000`, once per hand); the room's rounding to whole chips is disclosed as `assumptions.notes`. `no_flop_no_drop` affects only preflop bundle selection (all solves are postflop). Cumulative reopening **(spec decision: TDA-style cash rule)**: a player who already acted may raise again only if the total raise since their last action is at least one full raise (`Derived.last_full_raise`); several short all-ins accumulate.

### 4.4 Recommendation, coverage, events
```rust
pub struct DecisionIdentity { pub hand_id: u64, pub hand_revision: u32, pub decision_id: u64, pub config_revision: u32, pub model_revision: u32 }
pub enum Coverage { Exact, Approximate { reasons: Vec<ApproxReason> }, Unsupported { reason: UnsupportedReason, partial: Vec<ApproxReason> } }
pub enum ApproxReason {
    BetTranslation { street: Street, seat: Seat, observed_pct: f32, mapped: Vec<(f32, f32)>, deviation: f32, prominent: bool },
    DepthBucket { seat: Seat, actual_bb: f32, used_bb: u16, prominent: bool }, AsymmetricStacks { stacks_bb: Vec<f32> },
    RakeProfileMapped { actual: String, used: String }, StraddleMapped { posts: [f32; 3] }, ShortHandedMapped { dealt: u8 },
    DeadlineBestSoFar { reached_bp: u16, target_bp: u16 }, ChartRounded, EvReferenceUnverified,
    UnconditionedPriorStreet { street: Street, seat: Seat, cause: String }, UnconditionedCurrentStreet,
    MultiwayStreetRoot { folded_this_street: u8, dead_this_street: u32 }, SprBucketed { actual: f32, used: f32 },
    MenuRounded { max_delta_pct: f32 /* realized, section 10.4 */ }, BranchResidual { seat: Seat, residual_mass_pct: f32 },
}
pub enum UnsupportedReason {
    MultiwayEv { pot_eligible: u8 }, MissingPreflopNode { key: String }, HeroComboOutOfSupport, EngineError { message: String, retryable: bool },
    TreeTooLarge { estimate_bytes: u64 }, DeadlineExceeded { stage: String }, InvalidRanges, UnsupportedHistory { reason: String }, FormatUnsupported { detail: String },
}
pub enum Unavailable { NotInMenu, NoEvReference, HeroOutOfSupport, MovedProbability { from: Action }, ChartNoEv, NotEvaluated, Pending }
pub struct ActionAdvice { pub action: Action, pub frequency: Option<f32>, pub ev_bb: Option<f32>, pub unavailable: Option<Unavailable> /* why frequency or EV is None */, pub headline: bool }
pub enum Availability { Ready, Pending, Unavailable { reason: String } }
pub struct EquityEstimate { pub value: Option<f32> /* present iff Ready */, pub availability: Availability, pub method: Option<EquityMethod> /* Exact | MonteCarlo{samples: u32, std_err: f32}; present iff Ready */ }
pub struct EquitySummary {
    pub hero_combo_vs_each: Vec<(Seat, EquityEstimate)>,      // population: hero's actual combo fixed vs that seat's hero-conditioned public range, pairwise, all runouts
    pub hero_range_vs_each: Vec<(Seat, EquityEstimate)>,      // population: hero's public range vs that seat's public range, pairwise-compatible combos, all runouts
    pub per_pot_shares: Vec<PotShares>,                       // multiway only
}
pub struct PotShares { pub pot_index: u8, pub population: String /* fixed text "hero combo fixed; opponents jointly sampled, disjoint, from hero-conditioned public ranges" */, pub shares: Vec<(Seat, EquityEstimate)> }
pub struct Assumptions { pub ranges_used: Vec<(Seat, String, f32 /* mass */)>, pub tree_signature: String, pub template_id: String,
    pub source: String, pub source_accuracy: String /* "unverified" | "exploitability <= x" */, pub source_granularity: String,
    pub target_bp: u16, pub reached_bp: Option<u16> /* display only; comparisons use raw values */, pub elapsed_ms: u32, pub cache: String /* miss | exact | approximate | provisional */,
    pub translations: Vec<ApproxReason>, pub mappings: Vec<ApproxReason>, pub notes: Vec<String> }
pub struct ExperimentalHu {                                    // section 6: isolated synthetic-root surrogate, never in `actions`, never Exact
    pub opponent: Seat, pub hero_role: String /* oop | ip */, pub pot: u32, pub stack: u32, pub template_id: String,
    pub ranges_used: [(Seat, String, f32); 2] /* street-root public ranges, unconditioned by the current street */,
    pub actions: Vec<ActionAdvice>, pub reached_bp: Option<u16>, pub elapsed_ms: u32,
    pub note: String /* fixed text "experimental, not solved: synthetic root, empty history, unconditioned ranges" */ }
pub struct Recommendation {
    pub identity: DecisionIdentity, pub phase: Phase /* Fast | Provisional | Final */,
    pub coverage: Coverage, pub legal: Vec<LegalAction>,                 // legal intervals, always present
    pub actions: Vec<ActionAdvice>,                                       // evaluated menu for hero's actual combo (tree or store actions mapped to legal chips)
    pub range_mix: Option<Vec<(Action, f32)>>,                            // range-level mix: mass-weighted action frequencies over hero's public range at the node; present whenever a node strategy exists, the only strategy output under HeroComboOutOfSupport
    pub equity: EquitySummary, pub assumptions: Assumptions,
    pub experimental: Option<ExperimentalHu>, pub exploit: Option<ExploitAdvice>,
}
pub enum RecommendationEvent { Fast(Recommendation), Equity { identity: DecisionIdentity, equity: EquitySummary },
    Progress { identity: DecisionIdentity, stage: String, iterations: u32, exploitability_pct: Option<f32> /* None until measured */, elapsed_ms: u32 },
    Provisional(Recommendation), Final(Recommendation), NoDecision { identity: DecisionIdentity, reason: String } }
```
Headline **(spec decision)**: (1) when every action of `actions` has `ev_bb` (fold counts as 0 when legal), the highest-EV action for hero's actual combo (ties by higher frequency, then earlier in menu order), labelled "highest EV"; (2) otherwise no EV ranking is performed, even if some actions have EV; when every action has `frequency`, the headline is the highest-frequency action labelled "highest-frequency chart action" (`ChartTranscription`) or "highest-frequency source action, EV reference unverified" (`PokerDataJson` with `EvReferenceUnverified`); (3) with neither there is no headline. Reasons accumulate across source, replay, cache and solve on every path; a later exact solve or cache hit never removes inherited reasons. Exploitability is compared with the target in raw chips before rounding to bp for display. Event merging in the UI **(spec decision)**: an `Equity` event carrying the active identity enriches whatever `Fast`/`Provisional`/`Final` is displayed, including a `Final` that arrived earlier; a `Ready` estimate is never replaced by a `Pending` one (the engine emits at most one `Ready` per estimate per identity and never emits `Pending` after it).

### 4.5 Worker protocol (`proto::worker`)
UTF-8 JSON Lines, one object per line, newline-terminated and flushed per message, `type` discriminator with fixed lowercase tags, ids as decimal strings. stdout carries protocol only; diagnostics go to stderr (drained by the engine, 64 KiB ring). Limits **(spec decision, initial)**: request line <= 1 MiB, result line <= 16 MiB, <= 100,000 exported nodes subject to the byte cap; over-limit input is rejected before work starts; an over-limit export returns the requested node only plus `export: "truncated"` and `covered_paths` listing exactly the exported nodes.

Worker states: `Starting` (until `ready` is written) -> `Idle` -> `Building` (tree, tree cross-check, memory check, allocation, locks) -> `Solving` (`solve_step` loop) -> `Extracting` (`finalize`, navigation, matrices, serialization) -> `Idle`; `Stopping` on `shutdown` or stdin EOF. The `control` thread reads stdin in every state.

**Tagged wire schema (normative; serde `#[serde(tag = "type")]`, all tags lowercase, unknown fields rejected):**

| `type` | Direction | Fields (all required unless marked `?`) |
|---|---|---|
| `ready` | worker -> engine, once | `proto_version: u16` (3), `solver_commit: String`, `adapter_version: u16`, `threads: u8`, `build_features: [String]`, `cpu_features: [String]`, `capabilities: [String]` |
| `solve` | engine -> worker | `id`, `spot: String` (sha256 hex of the structural identity, for lock matching), `board: [Card]`, `oop_range: Range1326`, `ip_range: Range1326`, `pot: u32`, `stack_oop: u32`, `stack_ip: u32`, `rake_rate: f32`, `rake_cap_mchips: u32`, `tree: EffectiveTree` (section 4.6, materialized), `history: [Action]`, `target_bp: u16`, `deadline_ms: u32`, `extraction_margin_ms: u32`, `memory_limit_bytes: u64`, `background: bool` |
| `lock` | engine -> worker | `id`, `spot: String`, `locks: [NodeLock]` |
| `cancel` | engine -> worker | `id`, `target: String` |
| `shutdown` | engine -> worker | `id` |
| `ack` | worker -> engine | `id`, `status: "accepted" \| "staged" \| "rejected" \| "already_finished" \| "unknown_target"`, `reason?: String` (present iff `rejected`), `replaced?: bool` (present iff `staged`: a previously staged lock set was discarded) |
| `progress` | worker -> engine | `id`, `stage: "building" \| "solving" \| "extracting"`, `iterations: u32`, `exploitability_chips: f32 \| null` (null until the first measurement), `elapsed_ms: u32`, `memory_bytes: u64` |
| `result` | worker -> engine, terminal | `id`, `status: "ok" \| "best_so_far" \| "cancelled" \| "error"`, `elapsed_ms: u32`, `solution?: StreetSolution` (present iff `ok` or `best_so_far`), `error?: WorkerError` (present iff `error`; a sibling field, never nested in `status`) |

```rust
pub struct WorkerError { pub code: String, pub message: String, pub retryable: bool, pub estimate_bytes: Option<u64> /* present for tree_too_large */ }
pub struct StreetSolution { pub nodes: Vec<NodeStrategy>, pub requested: u32 /* index into nodes of the node reached by history */, pub exploitability_chips: f32,
    pub iterations: u32, pub memory_bytes: u64, pub mode: String /* f32 | i16 */, pub locks_applied: u16, pub export: String /* street | truncated */,
    pub covered_paths: Vec<Vec<Action>> /* the paths of every exported node, in export order */ }
pub struct NodeStrategy { pub path: Vec<Action>, pub actor: String /* oop | ip */, pub actions: Vec<Action>,
    pub probs: Vec<Vec<f32>> /* [1326][action], combo-major */, pub ev_chips: Vec<Vec<f32>> /* [1326][action], actor-owned, section-2 convention */, pub available: Vec<bool> /* [1326]: reach > 0 and not blocked */ }
pub struct NodeLock { pub path: Vec<Action>, pub actor: String, pub probs: Vec<Vec<f32>> /* [1326][action]; all-zero row = combo stays free */ }
```
Failure codes (complete list): `invalid_request` (retryable false), `tree_mismatch` (the library's realized tree differs from `tree.materialized`; retryable false), `tree_too_large` (with `estimate_bytes`; retryable false), `out_of_memory` (retryable true), `lock_mismatch` (retryable false), `no_iteration` (no exploitability measurement fit before the stop point of section 7; retryable false), `internal` (panic caught at the executor boundary; retryable true).

Matrix validation (both directions, `proto::worker::validate_solution`) **(spec decision)**: `probs` and `ev_chips` are exactly `[1326][actions.len()]`, all values finite; for every combo with `available[i] == true` the probability row sums to `1 +- 1e-3`; for `available[i] == false` both rows are all zeros; `requested < nodes.len()`; `covered_paths[k] == nodes[k].path`. The worker validates locks the same way (rows either all zero or summing to `1 +- 1e-3`). A violation is `result{error{invalid_request}}` for input and `EngineError("invalid solution")` for output; the same validator runs on every cache payload before it is served (section 10.4).

State transitions and rules:
- `ack{accepted}` acknowledges receipt only. Every accepted `solve` produces exactly one terminal `result` (`ok`, `best_so_far`, `cancelled` or `error`); the single exception is process death, for which the engine synthesizes the terminal (`WorkerExit{code}`, section 10.3) on confirmed exit. A completion racing a cancel yields either the valid result or `cancelled`, never both. Admission in the engine is released only by the terminal `result` or by confirmed process exit.
- `solve` while not `Idle`: `ack{rejected, reason: "busy"}`, no work. Duplicate id of a live request: `ack{rejected, reason: "duplicate"}`.
- `cancel` of the running job: `ack{accepted}`, then `result{cancelled}` at the next checkpoint of the current state: in `Building` after the current step (tree build, cross-check, memory check, allocation, lock application are each a step); in `Solving` at the next iteration boundary (an iteration is never interrupted; measured 0.08-0.3 s on flop trees); in `Extracting` after the node currently being extracted (`finalize()` itself is not interruptible, measured 0.2-0.35 s). `cancel` of a finished id: `ack{already_finished}`; of an unknown id: `ack{unknown_target}`.
- `shutdown` in any state: `ack{accepted}`; a running job receives `result{cancelled}` at its next checkpoint; the process then exits 0 within 2 s (the engine kills it otherwise). Stdin EOF behaves like `shutdown` without the `ack`.
- Structurally invalid input (unknown tag, unknown field, bad card, range length != 1326, non-finite number, history not representable in `tree`, node count over limit, invalid lock matrix): `ack{rejected, reason}` for messages without work, `result{error{invalid_request}}` for an accepted `solve` that fails validation in `Building`.
- Locks belong to `spot`; `ack{staged}`; a second `lock` while one is staged replaces it for any `spot` (`ack{staged, replaced: true}`) **(spec decision)**; consumed atomically by the next `solve` with the same `spot`; a `solve` with a different `spot` while a lock is staged returns `result{error{lock_mismatch}}` and discards the staged lock; a `lock` received while not `Idle` is `ack{rejected, reason: "solve_in_progress"}`; staged locks are cleared on consumption, cancellation, error and restart.
- `progress` is emitted at each stage transition and during `Solving` at iteration boundaries, coalesced to at most one per 100 ms; `exploitability_chips` is `null` until `compute_exploitability` has run once. The writer never interleaves JSON with other output.
- `ready` is written once; the engine validates `proto_version` (3), `solver_commit`, `adapter_version`, `threads == requested`, `build_features` contains `avx2`. Startup timeout 5 s, then kill and retry once, then `EngineError`.

Wire examples. In the river example the IP 1.0-pot bet equals `max` and is therefore forced to all-in by the pinned rule (section 4.6), so the materialized action is `allin`. The two-combo river fixture is defined by data, not by an existing file: `oop_range` has weight 1.0 at the six AA combos and 0 elsewhere; `ip_range` has weight 1.0 at the three legal QQ combos on `Qs Jd 7h 3c 2d` and 0.25 at the twelve 54o combos (section 13.2 oracle). The literal JSONL files `fixtures/worker/{river_two_combo,flop_cancel,flop_best_so_far,lock_river}.jsonl` with full 1326-element vectors are **to be created** in week 1 by `tools/gen_worker_fixtures.py` from these definitions (section 13.0); the vectors are elided only in this text:
```json
{"type":"ready","proto_version":3,"solver_commit":"9d1509fe5077d019825f833eed04b16d342dfda1","adapter_version":1,"threads":16,"build_features":["avx2"],"cpu_features":["avx2","fma"],"capabilities":["solve","lock","cancel","street_export","i16"]}
{"type":"solve","id":"41","spot":"3f9c0a7d2b1e4c6f8a9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f","board":["Qs","Jd","7h","3c","2d"],"oop_range":"<1326 numbers per the definition above>","ip_range":"<1326 numbers per the definition above>","pot":100,"stack_oop":100,"stack_ip":100,"rake_rate":0.0,"rake_cap_mchips":0,"tree":{"rules_version":2,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"actor":"oop","actions":[{"kind":"check"}]},{"path":[0],"actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}]},{"path":[0,1],"actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}]}]},"history":[{"kind":"check"}],"target_bp":10,"deadline_ms":1500,"extraction_margin_ms":200,"memory_limit_bytes":4294967296,"background":false}
{"type":"ack","id":"41","status":"accepted"}
{"type":"progress","id":"41","stage":"building","iterations":0,"exploitability_chips":null,"elapsed_ms":3,"memory_bytes":331776}
{"type":"progress","id":"41","stage":"solving","iterations":50,"exploitability_chips":0.27,"elapsed_ms":8,"memory_bytes":6914048}
{"type":"progress","id":"41","stage":"extracting","iterations":50,"exploitability_chips":0.09,"elapsed_ms":9,"memory_bytes":6914048}
{"type":"result","id":"41","status":"ok","elapsed_ms":12,"solution":{"nodes":[{"path":[{"kind":"check"}],"actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"probs":"<[1326][2]>","ev_chips":"<[1326][2]>","available":"<[1326]>"},{"path":[{"kind":"check"},{"kind":"allin","to":100}],"actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"probs":"<[1326][2]>","ev_chips":"<[1326][2]>","available":"<[1326]>"}],"requested":0,"exploitability_chips":0.09,"iterations":50,"memory_bytes":6914048,"mode":"f32","locks_applied":0,"export":"street","covered_paths":[[{"kind":"check"}],[{"kind":"check"},{"kind":"allin","to":100}]]}}
{"type":"cancel","id":"42","target":"41"}
{"type":"ack","id":"42","status":"already_finished"}
{"type":"solve","id":"43","spot":"<flop spot of the same shape>"}
{"type":"ack","id":"43","status":"accepted"}
{"type":"cancel","id":"44","target":"43"}
{"type":"ack","id":"44","status":"accepted"}
{"type":"result","id":"43","status":"cancelled","elapsed_ms":183}
{"type":"result","id":"45","status":"best_so_far","elapsed_ms":9870,"solution":{"nodes":"<flop nodes>","requested":2,"exploitability_chips":1.9,"iterations":70,"memory_bytes":1017118720,"mode":"f32","locks_applied":0,"export":"street","covered_paths":"<paths of every exported node>"}}
{"type":"result","id":"46","status":"error","elapsed_ms":12,"error":{"code":"out_of_memory","message":"allocation failed","retryable":true}}
{"type":"result","id":"49","status":"error","elapsed_ms":2,"error":{"code":"tree_too_large","message":"f32 estimate above limit","retryable":false,"estimate_bytes":9126805504}}
{"type":"result","id":"50","status":"error","elapsed_ms":410,"error":{"code":"no_iteration","message":"no exploitability measurement before stop point","retryable":false}}
{"type":"lock","id":"47","spot":"3f9c0a7d2b1e4c6f8a9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f","locks":[{"path":[{"kind":"check"},{"kind":"allin","to":100}],"actor":"oop","probs":"<[1326][2]>"}]}
{"type":"ack","id":"47","status":"staged","replaced":false}
{"type":"shutdown","id":"48"}
{"type":"ack","id":"48","status":"accepted"}
```
`proto_version` mismatch at `ready` is an `EngineError` for every request until the worker is rebuilt.

### 4.6 Effective tree (`proto::EffectiveTree`, materialized, hashed)
```rust
pub struct EffectiveTree {
    pub rules_version: u16,                          // 2; bumps when any rule below changes
    pub template_id: String, pub root_street: Street,
    pub menus: BTreeMap<Street, PlayerMenus>,        // per street: oop/ip bet sizes (pot fractions), raise multipliers (x facing wager), donk sizes (empty in phase 1)
    pub add_allin_threshold: f32,                    // pinned semantics below
    pub force_allin_threshold: f32,                  // pinned semantics below
    pub merging_threshold: f32,                      // 0.0 in phase 1 (spec decision); exact inserted sizes are never merged
    pub wager_cap: u8,                               // non-all-in bets plus raises per street; enforcement below
    pub inserted: Vec<(Vec<Action> /* path */, String /* actor */, Action)>,   // observed sizes not on the menu, exact chips
    pub materialized: Vec<MaterializedNode>,         // every action node of every street, computed by the engine with the rules below; cross-checked by the worker
}
pub struct MaterializedNode { pub path: Vec<u8> /* ordinal path, section 2 */, pub actor: String /* oop | ip */, pub actions: Vec<Action> /* chip amounts, deterministic order */ }
```
Deterministic action order at every node: Fold, Check, Call, bets/raises ascending by `to`, AllIn **(spec decision)**.

**Pinned action-construction semantics** (read from `src/action_tree.rs` at the pinned commit; the engine's materializer and the library must agree, and V4 pins them with equality-boundary tests). At a node let `to_call` be the amount the actor owes, `prev` the facing wager as a raise-to amount (0 for an opening bet), `pot` = starting pot plus both players' contributions plus `to_call` (**the pot after the actor's call**), `max` = the actor's all-in raise-to amount (opponent's remaining stack plus `prev`), `min = clamp(prev + to_call, 1, max)`:
- a bet of pot fraction `r` is `round(r * pot)`; a raise of pot fraction `r` is `prev + round(r * pot)`; a raise of `x` times the facing wager is `round(x * prev)` (`round` = the library's `f64::round`, half away from zero);
- **all-in is added when `max <= round(pot * add_allin_threshold)`**, i.e. when the actor's all-in wager is at most the threshold times the post-call pot (comparison inclusive, on the rounded integer). Example: root `P = 100`, stacks 100, menu 33%, threshold 1.5: `100 <= 150`, all-in is added although 33 does not exceed 150; threshold 0.0 never adds all-in;
- **a listed wager `a` is forced to all-in when `max <= a + round((pot + 2 * (a - prev)) * force_allin_threshold)`**, i.e. when the remaining stack after the wager is at most the threshold times the pot after the opponent calls (inclusive, rounded); threshold 0.0 forces only a wager that already equals `max`;
- wagers are clamped to `[min, max]`, sorted, deduplicated; `merging_threshold = 0.0` merges nothing;
- the library rejects `starting_pot <= 0` or `effective_stack <= 0` (pinned validation), so a street with a zero effective stack is never sent to the worker (section 6 covers all-in cases analytically or via `NoDecision`).

Menu sizes are converted to chips exactly as above (the wager quantum is one chip, so no further rounding exists), and duplicates (a size rounding onto another size, a call, or the all-in) are merged. Observed sizes are inserted exactly with `ActionTree::add_line` and are never merged, forced to all-in or capped away; an observed history that the rules cannot represent (a wager below the library's `min` at its node) returns `Unsupported{UnsupportedHistory}` instead of a changed history. **Wager cap (spec decision, frozen)**: the pinned library has no native per-street raise-count limit (only `BetSize::Additive`'s `raise_cap`, unused here), so the engine enforces the cap on the materialized tree and the worker mirrors it with `ActionTree::remove_line`: at every node where the count of non-all-in wagers on the street (observed prefix included) has reached `wager_cap`, every non-all-in bet and raise is removed; fold, check, call and all-in remain; observed prefix actions are never removed even when the prefix alone exceeds the cap. The worker enumerates the library tree after insertion and cap enforcement and compares it with `tree.materialized`; any difference is `result{error{tree_mismatch}}`, never a silently different tree.

The structural signature `tree_signature = sha256(rules_version, template_id, root_street, nominal menus, thresholds, wager_cap, inserted sizes as reduced pot-fraction rationals `to / P` at their ordinal paths)` contains no raw chips and no realized rounding; the realized `materialized` list is stored with each cache payload and compared at lookup (section 10.4).

---

## 5. Data flow for one decision

1. **Session start.** User sets `GameConfig`; the engine assigns `config_revision`, saves to `%APPDATA%\PokerAI\config.json`, spawns the worker, validates `ready`, opens the cache and validates the preflop bundles (section 8.2).
2. **Hand begin.** `begin_hand{button, hero, hero_cards, dealt, stacks}` with confirmed stacks; `HandConfig` frozen; `hand_id` and `hand_revision` assigned.
3. **Entry.** One keystroke per event (section 5.1); every mutation goes through `apply_action` / `set_board` / `undo` in the engine, which assigns a fresh `hand_revision`, cancels in-flight work and invalidates every descendant identity, even when no decision follows.
4. **Request.** At a decision point the UI calls `recommend`. `engine-main` stamps `t0` (monotonic), allocates `decision_id`, forms the identity tuple (a repeated request for the same state gets a new `decision_id`), re-derives the decision-point conditions of section 2 itself (answering `NoDecision` when they fail, independent of the UI), cancels any other live job and queues the request (depth 1, newest wins).
5. **Fast phase (target <= 0.3 s).** Validate (`Derived` recomputed and compared); classify coverage (section 6); replay public ranges (section 9); emit `Fast` with legal intervals, coverage so far, `frequency = None`, `ev_bb = None`, equity `Pending`. Equity runs with its own budget (section 7) and is delivered as `Equity` when ready.
6. **Preflop decision.** Store lookup (section 8): present node -> `Final` with frequencies and EV (PokerData with verified reference) or frequencies only (charts, or PokerData with `EvReferenceUnverified`); absent node -> `Final` `Unsupported{MissingPreflopNode}`.
7. **Postflop, HU.** Derive `StreetRootSnapshot` (genuine HU root, or the projection of section 10.2; `Unsupported{UnsupportedHistory}` when neither applies); form `SolveInput` with the replayed root ranges; build and materialize the effective tree (sections 4.6, 10.2); cache lookup (flop/turn, section 10.4): hit -> `Final` carrying exactly the reasons the hit incurred (none, `SprBucketed`, `MenuRounded`) plus inherited ones; provisional hit (accuracy above target) -> `Provisional`, then live solve; miss -> `solve` with the remaining street budget; forward `Progress`; on `result` validate the payload (section 4.5), the requested node and its actor against `Derived.to_act`, assemble `Final` (`Exact` if raw `exploitability_chips / P <= target_bp / 10000` and no reasons at all, else `Approximate`), store in cache (flop/turn). Every validated solution that reaches `Final` or `Provisional`, whether live or from the cache, is registered as a replay snapshot through the single `register_snapshot` path (section 9.2). Facing an all-in: same path (the jam is an inserted size; hero's menu is fold/call); if the worker is unavailable, the analytic fallback of section 6 applies.
8. **Postflop, 3+ pot-eligible.** `Final` `Unsupported{MultiwayEv}` with per-pot equity shares and `experimental` (the isolated synthetic-root surrogate of section 6) when it completes within the street budget.
9. **Display.** Headline per section 4.4; frequency mix, coverage label and assumptions always visible. The UI accepts only events whose identity equals the active identity; `Fast`, `Equity`, `Progress`, `Provisional` and `Final` are all subject to the check.
10. **Log.** Every request and its `Final` (identity, coverage, reasons, elapsed, cache result, deadline violations, versioned input record) appended to `%LOCALAPPDATA%\PokerAI\decisions.jsonl` (rotated at 50 MiB, 10 files) **(spec decision)**.
11. **Hand end.** `finish_hand` / `abandon_hand`; the idle timer for the pre-solver starts.

Per-street behaviour of step 7: flop solves use the flop template from the flop root with ranges from preflop replay; turn solves are rooted at the turn (ranges = flop-end public ranges from replay); river solves are rooted at the river and never cached. Prior-street solutions are used only through replay (section 9), never as the current street's strategy; a later action on a prior street that was not in that street's solved tree is translated (section 8.4) against that snapshot, never re-solved.

### 5.1 Keyboard entry (spec decision)
| Key | Effect |
|---|---|
| `N` | Begin hand: button seat (digit 1-6), hero seat, dealt seats (toggle digits), per-seat stacks (prefilled, Enter confirms), hero cards (`AsKd`, may be entered later with `H`) |
| `H` then four characters | Hero cards when not entered at `N` (`AsKd`); rejected if they duplicate a board card |
| `F` / `C` / `A` | Fold / check-or-call / all-in for `Derived.to_act` |
| `B` then digits, `Enter` | Bet or raise **to** the typed chip amount; `Esc` cancels |
| rank+suit characters, `Enter` | Board cards in `AwaitingBoard` (3, then 1, then 1); duplicates rejected |
| `Ctrl+Z` | Undo (engine command; new revision) |
| `Space` | Re-request `recommend` (new `decision_id`) |
| `E` / `X` | Finish hand / abandon hand |
| `T` | Seat-tag panel for the seat to act (exploit slice only) |

Illegal keys are disabled from `Derived.legal`; the seat to act is always derived. WebView2 accelerators (F5, Ctrl+P, F12) are suppressed with `preventDefault` and `with_browser_accelerator_keys(false)`.

---

## 6. Coverage rules

| Situation | Numeric EV | Label | What is shown |
|---|---|---|---|
| Preflop, node present, PokerData bundle, EV reference verified (A2) | Yes | `Exact`, or `Approximate` with `DepthBucket` / `AsymmetricStacks` / `RakeProfileMapped` / `StraddleMapped` / `ShortHandedMapped` / `BetTranslation` | Frequencies, EV, equity, `source_accuracy = unverified` |
| Preflop, node present, PokerData bundle, reference unverified | No | `Approximate{EvReferenceUnverified}` plus any mapping reasons | Frequencies, equity |
| Preflop, node present, chart bundle | No | `Approximate{ChartRounded}` plus any mapping reasons | Frequencies (headline = highest-frequency), equity |
| Preflop, node absent | No | `Unsupported{MissingPreflopNode}` | Equity only |
| Postflop, HU, street root HU | Yes | `Exact` or `Approximate` with `BetTranslation` (prior street), `DeadlineBestSoFar`, `UnconditionedPriorStreet`, `BranchResidual`, `SprBucketed`, `MenuRounded` (cache hits), plus inherited preflop reasons | Frequencies, EV, equity, progress |
| Postflop, HU now, street root 3+ players, projection reproduces the current legal state exactly (section 10.2) | Yes | `Approximate{MultiwayStreetRoot}` plus the reasons of the row above | As above, folded seats' same-street chips as dead money at the root |
| Postflop, HU now, street root 3+ players, projection does not reproduce the current state | No | `Unsupported{UnsupportedHistory}` | Equity only (no other multiway projection exists in phase 1) |
| Postflop, HU, opponent all-in, worker unavailable | Yes | `Approximate{UnconditionedCurrentStreet}` plus inherited reasons | Analytic `EV(call)`, `EV(fold) = 0`, equity |
| Postflop, hero combo has zero weight at the node | No | `Unsupported{HeroComboOutOfSupport}` | Equity, range-level mix |
| Postflop, 3+ pot-eligible (any all-in counts) | No | `Unsupported{MultiwayEv}` | Per-pot equity shares; `experimental` block |
| NoDecision (section 2) | n/a | no request | UI shows "no decision" |
| Worker failure after retry / deadline expired | No | `Unsupported{EngineError}` / `Unsupported{DeadlineExceeded}` | Equity, visible reason |

Rules:
- Pot-eligible counting uses `Derived.folded` only; all-in players count. Folded players' ranges are retained (`folded_ranges`) for provenance; bunching is fixed off in phase 1.
- Reasons accumulate: a turn solve after a translated flop bet and a depth-bucketed preflop lookup carries both. An `Unsupported` result keeps the accumulated reasons in `partial`.
- Facing an all-in, analytic fallback **(spec decision)**: let `C` be the payable call and `W` the final matched pot after returning excess and adding `C`; `EV(call) = equity_actual_combo * (W - R) - C` with `R = min(rate * W, cap)`, ties counted as shares, using hero's actual combo against the opponent's public range conditioned on prior streets and hero-conditioned for equity; the current street's actions are unconditioned (`UnconditionedCurrentStreet`). Frequencies: call 100% when `EV(call) > 0`, fold 100% when `< 0`, call when `= 0` **(spec decision: tie rule)**. The primary path (worker solve with the jam inserted) conditions the jam through the solve.
- `experimental` (multiway only; outline §5): an **isolated synthetic-root HU solve** with a contract separate from every main-path solve **(spec decision)**: opponent = the seat whose public range has the highest range-vs-range equity against hero's public range (same equity routine, pairwise-compatible); synthetic root = `pot` = the current total pot at the decision (all chips committed, all streets), `stack` = min(hero's remaining stack, that opponent's remaining stack), **empty history**, ranges = hero's and the opponent's public ranges **at the current street root** (the current street's actions are not applied: "unconditioned ranges", stated in `ranges_used`), hero's role = OOP if hero precedes the opponent in postflop order else IP; hero's advice is read at the synthetic root when hero is OOP and at the node after OOP's check when hero is IP; same template as the street and the remaining street budget; skipped when the opponent is all-in, when `stack == 0`, or when the equity phase overran. It never goes through `SolveInput`, the cache or replay snapshots, and is rendered in a separate block "experimental, not solved", never in `actions`, never `Exact`.
- A HU spot whose prior street was multiway is still HU now; that street's actions are replayed with `UnconditionedPriorStreet` (section 9.3).

---

## 7. Latency budget, deadlines and progressive display

Absolute deadlines from the monotonic admission time `t0`: first-attempt completion `t0 + 2 s` (river), `t0 + 6 s` (turn), `t0 + 10 s` (flop); final delivery `t0 + 15 s`, including retries, restarts, cache reads, pipe transfer and extraction. Every phase receives only the remaining time. The worker receives relative `deadline_ms = remaining - delivery_margin (100 ms) - pipe_margin (50 ms)` computed at send time; no cross-process timestamps. After a suspend/resume the request is expired and late replies are rejected by identity and expiry.

| Stage | Budget | Measured (R8) / note |
|---|---|---|
| Validation + coverage + replay | <= 0.15 s | fast-path thread |
| Fast event | <= 0.3 s | equity may be `Pending` |
| Equity | own budget: 0.5 s after `Fast`, cancellable; exact when `pairs * runouts <= 2 * 10^7` else time-bounded MC reporting samples and standard error **(spec decision)** | hypothesis |
| Preflop lookup | <= 0.05 s | in-memory bundle |
| Cache lookup (flop/turn) | <= 0.5 s | bounded file read + zstd + validation |
| River solve | <= 2 s | measured 4 ms to 0.5% (564x463 hands, 6.6 MB); headroom about 500x |
| Turn solve | <= 6 s | measured 0.17 s to 0.5%, 0.22 s to 0.3% (574x472 hands, 46 MB); headroom about 30x |
| Flop solve, cache miss | <= 10 s | measured 6.2 s to 0.5% with AVX2 (11.4 s without) for the reduced tree at 179x264 hands, 970 MB; the full two-size tree is 42 s at 3.8 GB (44 s at 2.0 GB in 16-bit mode) and is never used live; run-to-run variance 1.3-2.6x, so a slow run reaches `DeadlineBestSoFar`; realistic 460-570-hand ranges are unmeasured (V3) |
| Extraction margin (worker) | river/turn 0.2 s, flop 0.6 s **(spec decision)** | `finalize` measured 0.2-0.35 s on flop trees |
| Worker cancel | `ack` <= 50 ms; `result{cancelled}` <= one iteration + one exploitability pass | measured iteration 0.08-0.3 s on flop trees; engine kills after 1.5 s **(spec decision)** |
| Crash restart + retry | remaining time; admitted only if `remaining >= p95(_min template) + margins` from the bench report (before a report exists: the street budget) | `ready` within 0.5 s of spawn is a hypothesis (the upstream example binaries ran in 0.16-0.35 s wall, measured) |
| Exploit slice (river only) | + <= 2 s | river solves are 4 ms |

Worker deadline logic: between iterations the executor stops when `elapsed + 1.5 * max_iteration_so_far + (exploitability pass if due) + extraction_margin_ms > deadline_ms`; exploitability is checked every 10 iterations and additionally whenever fewer than 10 iterations fit before the stop point. At the stop point the worker finalizes and returns `best_so_far` only if at least one exploitability measurement exists; otherwise `error{code: no_iteration, retryable: false}`.

Progressive display: `Fast` at <= 0.3 s; `Equity` when ready (each estimate carries its own availability, method, samples and standard error; it enriches an already displayed `Final` of the same identity); `Progress` at most every 250 ms in the UI (iterations, exploitability % of pot or "measuring" while `None`, stage, elapsed); `Provisional` for an above-target cache hit; `Final` once. `target_bp` default 50 (0.5% of pot) **(spec decision; measured runs used the same target)**.

Watchdog (engine, independent of the worker client): at `t0 + street budget` it records a street-deadline violation if no first-attempt terminal arrived (the result is still accepted if it arrives before final delivery); at `t0 + 15 s - 100 ms` it emits `Final` with the retained validated payload (a `Provisional` or an earlier `best_so_far`) or `Unsupported{DeadlineExceeded{stage}}` with the available equity, then cancellation/kill/restart proceeds independently. Late replies are discarded. Deadline violations are logged as failures separately from p95 gates.

Admission: one live solve at a time; a new request cancels the older one; a `background` job is cancelled by any live request. Gate: `bench` p95 per street within budget, zero final-delivery violations in fault-injection runs, e2e p95 <= 15 s including cache misses.

---

## 8. Preflop store (`core-preflop`)

### 8.1 Interface
```rust
pub trait PreflopSource {
    fn bundle_info(&self) -> &BundleInfo;                       // id, source kind, depth_bb, source blinds, rake profile, straddle: bool, version, ev_unit, ev_reference, license note, accuracy
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode>;
}
pub struct PreflopNodeKey { pub depth_bb: u16, pub rake_profile: String, pub straddle: bool, pub history: Vec<(Position, PreflopStep)> }
pub enum PreflopStep { Fold, Check, Call, Raise { to_bb_x1000: u32 }, AllIn }      // history keeps every explicit fold; Check exists for BB after a limp
pub struct PreflopNode { pub actor: Position, pub actions: Vec<PreflopStep>, pub probs: Vec<Vec<f32>> /* [169][action] */, pub ev_source_sb: Option<Vec<Vec<Option<f32>>>> /* [169][action], None = unavailable */,
    pub unreachable: [bool; 169] /* class has no data at this node (sibling weights sum to 0); its probs row is all zero and never used */, pub committed_by_actor_sb: f32 /* from the prefix, for EV normalization */ }
pub struct PreflopStore { bundles: Vec<Box<dyn PreflopSource>> }
impl PreflopStore { pub fn query(&self, cfg: &HandConfig, state: &HandState, prefix_len: usize) -> PreflopAnswer /* node expanded to 1326 + reasons, or Unsupported; actor derived from the prefix */ }
```

### 8.2 Sources, acquisition and validation
- **Selected path (outline §0): one-time packs.** The packs are MonkerSolver `.7z` exports with an undocumented internal layout (R7 §4). `tools/pokerdata_convert.py` converts a pack into the normalized local node format below; the free Starter API sample (R7 §9 step 2, the user's action) provides the reference for field semantics and for check A5. Acquisition gate (V9): written personal-offline-use confirmation from the vendor; an authorized sample converted into the normalized format with frequencies **and** EV, passing R7 A1-A6. Raw-format investigation is time-boxed to 2 working days **(spec decision)**; if blocked, the alternatives are one month of API Pro to pull the trees as JSON (a separate user purchase decision, never assumed) or the chart baseline only. MonkerSolver provenance, the eight depths {20, 30, 40, 50, 70, 100, 150, 200} and the branch facts of R7 §3 are resolved; straddle and 250bb trees are phase 2.
- **Normalized local node format** (`%LOCALAPPDATA%\PokerAI\preflop\<bundle_id>\nodes.json` + `manifest.json`), produced by the converter, read by `PokerDataJson`:

| Field | Meaning |
|---|---|
| `game`, `version` | must be `"nl"`, `2` (source API schema); the converter records the pack file hash and MonkerViewer version in `provenance` |
| `stack` | source big blinds; source blinds fixed at `sb = 0.5`, `bb = 1` |
| `actor` | position to act at the node |
| `history` | ordered `(position, step)` pairs including every explicit fold; a `spot` string that ends with an action is a decision-node history plus that action, never a node |
| `weights[class]` | conditional probability of the action for the class; absent key = 0 |
| `evs[class]` | action EV in source small blinds; absent key = unavailable (`None`), never 0 |
| `combos` | aggregate weighted combo mass of the action; informational, not reach and not probability |
| `sizes` | per raise action: the label ("60%") and the resolved raise-to in source bb (R7 A3); a node with an unresolved size is not loadable |

The manifest carries `bundle_id`, `source`, `depths`, `rake_profile` ("5% cap 0.5bb"), `source_blinds`, `ev_unit = source_sb`, `ev_reference` (section 8.3, per A2), `accuracy` (`unverified` unless A6 supplies exploitability), `straddle = false`, `license_note`, `sha256` of contents.
- **Normalized node envelope** (`nodes.json`, produced by the converter and by `chart_ingest.py` alike; dense, the source's sparse `weights`/`evs` maps are densified) **(spec decision)**:
```json
{"bundle_id":"pokerdata_nlh6max_100","depth_bb":100,"rake_profile":"5% cap 0.5bb","straddle":false,"class_order":"A-2 row-major, section 4.1",
 "nodes":[{"history":[["UTG","raise",2500],["HJ","call",0]],"actor":"CO",
   "actions":[{"step":"fold"},{"step":"call"},{"step":"raise","to_bb_x1000":7900,"label":"79%"},{"step":"allin"}],
   "weights":[[169 numbers per action, class order]],"evs":[[169 entries per action: number or null]],
   "unreachable_classes":[]}]}
```
`history` steps are `fold | check | call | raise(to_bb_x1000) | allin`; `weights[a][c]` is the conditional probability of action `a` for class `c` (absent source key -> `0.0`); `evs[a][c]` is the action EV in source small blinds (absent source key -> `null`, never 0; `evs` may be omitted entirely for chart bundles).
- Validation at load (after the manifest hash; a matching hash never substitutes for content checks): schema (every array exactly 169 entries per action, valid step tokens, resolved sizes), finite values, weights in `[0, 1]`; **per class, the sibling weights sum to `1 +- 1e-3`, or to exactly 0 with the class listed in `unreachable_classes`**; any other sum is a content failure. Decompression and file size bounds 64 MiB. A failing bundle is quarantined (renamed `.bad`) with a startup banner; remaining bundles stay active. A class that is unreachable at hero's decision node yields `HeroComboOutOfSupport` (range mix still shown); during replay an unreachable class already has zero reach and is left unchanged.
- Sample fixtures `fixtures/preflop/pokerdata_sample/{node,range,spots}.json` (exact vendor field names and action tokens, a sparse zero-weight EV, the SB-limp/BB-check node) are a **V9 deliverable to be created** from the authorized sample; until then `pokerdata_schema_mapping` runs against a synthetic fixture `fixtures/preflop/synthetic_v2/*.json` written from R7 §4's documented response shape and marked synthetic (section 13.0).
- `ChartTranscription`: `fixtures/charts/<name>.json` from `tools/chart_ingest.py` (169-class frequencies, no EV, `ChartRounded`), same envelope with `evs` omitted. Depths 100 (PokerCoaching) and 200 (RangeConverter); **to be created** in week 1 (section 13.0). Charts are the release fallback and the baseline gate's range source (section 13.5).
- Source precedence: PokerData over charts; within a source the nearest depth; between bundles ranked equal on every rule of section 8.3, the lexicographically smaller `bundle_id` **(spec decision, deterministic tie)**.

### 8.3 Lookup rules (per prefix, hindsight-free)
- **Node reconstruction**: every historical node is rebuilt from its own prefix only: actor first, eligible seats = seats not folded before that action. Mappings are cached per prefix within a replay run and never recomputed with later folds.
- **Depth**: `depth = min(actor.stack_start, max over other eligible seats' stack_start) / unit` with `unit = bb_chips` (ordinary) or `straddle_chips` (straddle-mapped). Nearest acquired depth, ties deeper; above 200 clamps to 200. `DepthBucket{seat, actual, used}` whenever `actual != used` (the outline's "labelled when not exact"), `prominent` when `abs(actual - used) / used > 0.05`. `AsymmetricStacks{stacks_bb}` whenever **any** eligible seat's start stack (in `unit`) differs from the depth used, by any amount; `prominent` when some difference exceeds 5% of the depth used **(spec decision: label always, 5% controls prominence only)**. Actor 100bb versus opponent 104bb therefore maps to depth 100 with `AsymmetricStacks` and is never `Exact`.
- **Rake**: tuple `(rate, cap_bb, no_flop_no_drop)`; exact match preferred; otherwise order by `abs(cap_bb - actual_cap_bb)`, then `abs(rate - actual)`, then collection rule, ties to the lower cap; `TimeCharge` prefers an unraked bundle, else the smallest cap. Any difference is `RakeProfileMapped`.
- **Straddle** (phase 1 requires one fully posted UTG straddle with `S >= 2 * bb` and six dealt seats; anything else is `FormatUnsupported`): physical order per section 2. Lookup-only virtual roles: HJ->UTG, CO->HJ, BTN->CO, SB->BTN, BB->SB, straddler->BB; cards, posts and OOP/IP are never rotated. Source bb = `S` for depth, sizes, cap and EV conversion. Reported normalized posts `(sb/S, bb/S, 1)`: 1/2/4 gives `(0.25, 0.5, 1)`, 2/5/10 gives `(0.2, 0.5, 1)`; the physical SB's post is not represented in the virtual tree (the physical BB's post matches the virtual SB post exactly when `bb/S = 0.5`). Label `StraddleMapped{posts}`.
- **Short-handed** (fewer than six dealt seats, no straddle): the vacant seats are mapped as the earliest positions folded, for lookup only; no folded range and no bunching contribution are attributed to them; `ShortHandedMapped{dealt}` **(spec decision)**.
- **Sizes**: an observed raise-to is converted to source bb (`to / unit`) and matched against the node's resolved sizes; exact within `0.5 * chip / unit` is used directly; otherwise section 8.4.
- **Unsupported**: no node for the history (per R7 §3: open-limps from UTG, HJ, CO and BTN are absent, cold-call versus 3-bet is absent for CO and SB, more raises than the tree; SB limp and BB-versus-limp exist) or no bundle: `Unsupported{MissingPreflopNode{key}}`; never a guessed fold.
- **EV normalization** (`PokerDataJson`, gated on `ev_reference`, which A2 sets to exactly one of the following; each formula is scoped to its variant) **(spec decision)**. Let `committed` be the actor's chips already committed in the hand before the decision (posts and earlier preflop wagers, in source SB, known from the prefix) and `stack_dec = source_stack_sb - committed` the actor's stack at the decision:
  - `decision_incremental_verified` (source EV is `E[final stack] - stack_dec`, fold = 0): `ev_inc_sb(a) = ev_sb(a)` unchanged, at every node including no-fold nodes;
  - `net_hand_start_verified` (source EV is `E[final stack] - source_stack_sb`, so fold = `-committed`, e.g. SB fold -1, BB fold -2): `ev_inc_sb(a) = ev_sb(a) + committed` at every node; at fold-legal nodes this equals `ev_sb(a) - ev_sb(fold)` and the two are cross-checked (`|difference| <= 1e-3` else the node is unloadable);
  - `absolute_stack_verified` (source EV is `E[final stack]`, so fold = `stack_dec`): `ev_inc_sb(a) = ev_sb(a) - stack_dec` at every node; at fold-legal nodes cross-checked against `ev_sb(fold)` the same way;
  - `unverified`: EV is suppressed for every node and `EvReferenceUnverified` is added; frequencies remain.
  A value that cannot be normalized under the declared variant (missing `ev_sb(a)`, non-finite result, a failed cross-check) is `None` for that action with `unavailable: NoEvReference`; nothing is inferred. Then `ev_source_bb = ev_inc_sb * 0.5` (source SB / source BB), `ev_chips = ev_source_bb * unit` (`bb_chips` or `straddle_chips`), display `ev_chips / bb_chips`. Live blinds never enter the conversion (1.84 source SB is 0.92 source bb at 1/2 and at 2/5).
- 169 -> 1326 expansion gives every combo of a class the class values; public blocking by the board is applied by `core-replay`.

### 8.4 Bet translation
- **Likelihood interpolation** (replay, one observed action at one parent): with the source menu sizes at that node as pot fractions and observed `s`, adjacent `A <= s <= B`: `f_A = (B - s)(1 + A) / ((B - A)(1 + s))`, `f_B = 1 - f_A`, `P(obs | combo) = f_A P(A | combo) + f_B P(B | combo)`. Boundaries: `A == B` or a single size: `f = 1` (labelled clamp); below the smallest: clamp to it; above the largest non-all-in: interpolate with the all-in fraction when all-in is on the menu, else clamp. Deviation `d = min(|s - A|, |s - B|)` over the sizes used.
- **Branch posterior (spec decision, replaces the branch mixture of revision 2)**. Replay keeps, per seat, a set of branches `b`, each with a source node position and an un-normalized per-combo mass `w_b[c]` (`f64`; the seat starts with one branch, `w[c] = 1`). Every observed action is applied **exactly once**, as a multiplication of each branch's mass: an on-menu action `a` gives `w_b[c] *= P_b(a | c)`; an off-menu wager splits every branch `b` into `b_A`, `b_B` with `w_{bA}[c] = w_b[c] * f_A * P_b(A | c)` and `w_{bB}[c] = w_b[c] * f_B * P_b(B | c)`, so that `sum_b w_b[c]` after the split equals `w_b[c] * P(obs | c)` and nothing is conditioned twice. The seat's public range is the marginal `r[c] = sum_b w_b[c]`; the per-combo branch posterior is `pi_b[c] = w_b[c] / r[c]` (defined where `r[c] > 0`). After each update all branch masses are rescaled by one common factor so that `max_c r[c] = 1`, and the log of that factor is accumulated in `log_reach` (section 9.2). Branch cap: at most 4 branches per seat; when a split would exceed it, the branches with the smallest total mass `sum_c w_b[c]` (ties: the earlier-created branch is kept) are merged into one **residual branch** that keeps their summed masses, has no source node, and is not conditioned by later actions of the same street (its masses stay fixed); the residual's share of the seat's mass is reported as `BranchResidual{seat, residual_mass_pct}` and is never discarded. A branch whose continuation node is absent follows section 9.3 for that branch (no silent renormalization).
- **Node translation** (current preflop decision of hero at a translated node): for hero's combo `c`, `P(a | c) = sum_b pi_b[c] * P_b(a | c)` over the branches whose node has an action of identical kind and identical chip amount after mapping (a residual branch contributes to no action and its `pi` is reported as unconditioned mass); `EV(a | c) = sum_{b: a in b} pi_b[c] * EV_b(a | c) / sum_{b: a in b} pi_b[c]`, i.e. the posterior-weighted average over the branches in which `a` exists (the branch is the state of the world and does not depend on the action hero is about to choose) **(spec decision)**; EV is present only when every contributing branch has EV under the same `ev_reference`, otherwise `None` with `unavailable: NoEvReference`. Actions present in no branch are `unavailable: NotInMenu`. The range mix uses `r[c]` as the mass.
- Every non-exact mapping is recorded in `assumptions.translations` with its deviation; the `BetTranslation` reason enters `Coverage` with `prominent = d > 0.10` **(spec decision: the outline's threshold controls prominence)**.
- **Legality after mapping** **(spec decision)**: source sizes are converted to chips (round-half-up to the quantum); an action illegal at the actual node is remapped by kind: a raise below the minimum raise moves to the smallest legal menu raise if one exists, else to call; a wager above the stack moves to all-in; the destination's probability is the sum; the destination keeps its own EV only if it was itself a source action with a normalized EV, and a destination created by the move carries `ev = None` with `unavailable: MovedProbability{from}`; the moved source action, its probability and its destination are listed in `assumptions.notes`. Identical rounded actions merge the same way. A missing action is never a guessed fold; a missing probability or EV stays absent.
- Used for prior-street postflop actions that are absent from that street's snapshot tree (section 9.2), with the snapshot node's menu sizes as the interpolation sizes. Never used for the current postflop street (observed sizes are inserted exactly, section 10.2).

---

## 9. Range replay (`core-replay`)

### 9.1 Interface
```rust
pub struct SnapshotKey { pub hand_id: u64, pub config_revision: u32, pub model_revision: u32, pub street: Street, pub root_board: Vec<Card>, pub root_range_hashes: [[u8; 32]; 2], pub tree_signature: String }
pub struct SnapshotProvenance { pub identity_at_solve: DecisionIdentity /* immutable; the identity under which the solution was validated */, pub solved_prefix: Vec<(Seat, Action)> /* the street history inserted when it was solved */, pub origin: String /* live | cache_exact | cache_approximate | cache_provisional */ }
pub struct StreetSnapshot { pub key: SnapshotKey, pub provenance: SnapshotProvenance, pub tree: EffectiveTree /* materialized */, pub nodes: Vec<NodeStrategy>, pub covered_paths: Vec<Vec<u8>> /* ordinal paths of every exported node */, pub exploitability_chips: f32, pub reasons: Vec<ApproxReason> }
pub struct Branch { pub node: Option<(String /* source: store node key or snapshot ordinal path */, u8 /* branch id, creation order */)>, pub mass: Vec<f64> /* [1326], un-normalized */, pub residual: bool }
pub struct SeatBranches { pub seat: Seat, pub branches: Vec<Branch> /* at most 4 plus one residual, section 8.4 */ }
pub struct ReplayInput<'a> { pub cfg: &'a HandConfig, pub state: &'a HandState, pub store: &'a PreflopStore, pub snapshots: &'a [StreetSnapshot] }
pub struct ReplayOutput { pub ranges: Vec<Option<Range1326>> /* per seat, public marginal */, pub branches: Vec<SeatBranches> /* per seat, section 8.4 */, pub folded_ranges: Vec<Range1326>, pub log_reach: Vec<f64>, pub reasons: Vec<ApproxReason>, pub unsupported: Option<UnsupportedReason> }
pub fn replay(input: ReplayInput) -> ReplayOutput;
```

### 9.2 Rules
- Start: every dealt seat has the uniform range (weight 1 on all 1326 combos), one branch. Hero's actual cards are never applied. Replay arithmetic is `f64`; ranges are converted to `f32` at the output boundary **(spec decision)**.
- Preflop: for each observed action in order, the branch update of section 8.4 with `P(action | combo, node)` from the store node reconstructed for that prefix (section 8.3), likelihood interpolation for off-menu sizes. Each action is applied exactly once from its prefix-specific node; a node's reach is never multiplied in again. Folds condition the folder's range (`folded_ranges`).
- Postflop, per completed street: select the compatible snapshot (below), then walk the street's observed actions in order from the snapshot's root, tracking the current ordinal path per branch: (1) the node at the current path is present in `covered_paths` and the action is in its menu (an inserted observed size is a tree action, so its probability is the solved one, never forced to 1): multiply by its probability and advance; (2) the node is present but the action is a wager not in its menu (an action entered *after* that solve, or a later decision's insertion): translate it with section 8.4 against that node's menu sizes, split branches, advance each branch along its mapped action; (3) the node at the current path is absent (uncovered continuation, truncated export, or a branch whose mapped continuation is not covered): conditioning for that seat stops at that point **keeping the range as conditioned so far**, with `UnconditionedPriorStreet{street, seat, cause: "uncovered path <ordinal path>"}`; the already applied prefix is never discarded.
- Snapshot compatibility: same `hand_id`, `config_revision`, `model_revision`; the snapshot's `root_board` equals the street's board; its `root_range_hashes` equal the hashes of the public ranges this replay computes at that root. It is **not** required to contain every observed action of the street. Among compatible snapshots the one whose `covered_paths` cover the longest prefix of the observed street history wins, ties to the lowest exploitability, then the most recent `identity_at_solve.decision_id` **(spec decision)**.
- Registration: every validated solution that reaches `Final` or `Provisional` (live `ok`/`best_so_far` results and cache hits alike) is registered through `engine::register_snapshot` under the identity active when it was validated; `provenance.identity_at_solve` is immutable thereafter and is distinct from the active request identity. A `Provisional` registration is replaced by the `Final` of the same decision. A mutation invalidates every snapshot of later streets and every snapshot of the same street whose `solved_prefix` is not a prefix of the new history; a snapshot whose `solved_prefix` remains a prefix stays valid and its covered paths are reused.
- Public blocking by the board is applied at each street root; `log_reach[seat]` accumulates the log of every common rescaling factor (section 8.4) and is reported for guards and provenance; the solver receives the marginal with maximum weight 1 (a positive rescaling is irrelevant to the strategy). **Zero compatible support**: if an update leaves the seat's marginal exactly zero for every combo, the update is rejected, the pre-action range is kept and `UnconditionedPriorStreet{cause: "zero support after <action>"}` is added **(spec decision)**; a rare but positive reach is valid at any magnitude (no threshold).
- Hero's own actions condition hero's public range the same way. Hero-conditioned copies are built after replay for equity only.

### 9.3 Missing strategies
- Missing preflop node during replay for a later decision: the seat's range stays at its pre-action value, traversal of that seat's later preflop actions stops until a prefix with a present node resumes, reason `UnconditionedPriorStreet{Preflop, seat, cause: "missing node <key>"}`; the decision stays `Approximate` (the postflop solve is exact given the declared ranges). The current decision's own lookup failing is `Unsupported{MissingPreflopNode}` (sections 6 and 12); the two cases are distinct.
- A completed street with no compatible snapshot (multiway at the time, engine error, deadline, or no request was made): unconditioned, `UnconditionedPriorStreet{street, seat, cause}`.
- A branch whose continuation is absent (9.2 case 3) stops there; other branches continue; the residual rule of section 8.4 applies to cap overflow.
- Provenance and accuracy reasons stored with the snapshot (`reasons`, `origin`) are carried into the current result.

---

## 10. Postflop engine (`engine` + `solver-worker` + `cache`)

### 10.1 Tree templates (bet sizes as pot fractions; raise sizes as multiples of the facing wager; "a" = all-in only)
| Template id | Street | Bet sizes OOP / IP per street | Raise | add-all-in | force-all-in | Wager cap | Measured analogue (R8) |
|---|---|---|---|---|---|---|---|
| `flop_fast_v1` | flop root | flop 0.5 / 0.5; turn 0.5; river 0.5 | 2.5x | 1.0 | 0.15 | 3 | FLOP-FAST: 6.2 s to 0.5% AVX2 (7.7 s to 0.3%), 970 MB f32, 510 MB i16 (+25-30% time), 179x264 hands |
| `flop_full_v1` | flop root, pre-solver phase 2 only | flop 0.33, 0.75 / 0.33, 0.75; same on turn, river | 2.5x | 1.0 | 0.15 | 3 | FLOP-FULL: 42 s, 3.8 GB f32; 44 s, 2.0 GB i16; never solved live |
| `flop_min_v1` | flop root | 0.75 / 0.75; turn 0.75; river 0.75 | a | 1.5 | 0.15 | 1 | unmeasured (smaller than FLOP-FAST) |
| `turn_std_v1` | turn root | turn 0.33, 0.75 / 0.33, 0.75; river 0.33, 0.75 | 2.5x | 1.5 | 0.15 | 3 | TURN: 0.17 s, 46 MB, 574x472 hands |
| `turn_min_v1` | turn root | 0.75 / 0.75; river 0.75 | a | 1.5 | 0.15 | 1 | unmeasured |
| `river_std_v1` | river root | 0.33, 0.75 + a / 0.33, 0.75 + a | 2.5x | 1.5 | 0.0 | 3 | RIVER: 4 ms, 6.6 MB (force-all-in 0.15 would replace the 0.75 bet by all-in at SPR 1) |
| `river_min_v1` | river root | 0.75 / 0.75 | a | 1.5 | 0.0 | 1 | unmeasured |

Thresholds use the pinned `TreeConfig` semantics written out in section 4.6 (`add_allin_threshold`: all-in added when the all-in wager is at most the threshold times the post-call pot, inclusive; `force_allin_threshold`: inclusive on the post-call remaining stack; `merging_threshold = 0.0`); donk menus are empty. Under those semantics `river_std_v1` at `P = 100`, stacks 100 adds the all-in (`100 <= 150`), matching the measured RIVER root menu Check / Bet 33 / Bet 75 / AllIn 100. The `_min` templates are the crash/timeout retry and the `TreeTooLarge` fallback. The sizes are the **initial contracts** of phase 1 **(spec decision)**: they are fixed now, V3 re-measures them with store ranges at 100bb and 200bb, and any change is made only by incrementing the version suffix; old cache entries stay valid under their own signature.

### 10.2 Effective tree and street-root solve
- Root inputs come from `StreetRootSnapshot` only: `pot = pot_root + dead_this_street`, `stack_oop`, `stack_ip` (effective stack = min; the excess is uncontestable). `core-model` asserts that replaying `history` from the snapshot (`replay_root`) reproduces the decision point's total pot, both survivors' current-street contributions, remaining stacks, facing amount, minimum raise and actor; a mismatch on a genuine HU root is `EngineError` (never a silently adjusted root).
- Menus are converted to chips at each node (section 4.6). An already-taken action whose chips equal a menu size is in-tree. An off-menu taken action is inserted exactly at its node **alongside** the menu sizes; no sibling line is ever pruned. The solve starts from the incoming public ranges at the street root; the worker applies `history` to reach the requested node and exports every decision node of the current street. Complexity is bounded only by the future menus (templates), never by removing observed-prefix alternatives.
- **Street root with 3+ pot-eligible players that is HU now (outline §5, amended)**: the projected root removes the players who folded on this street, drops their actions from `history`, adds their current-street contributions to the root pot as `dead_this_street`, and keeps the two survivors' actions in order. The projection is admitted **only if `replay_root` on the projected snapshot is legal at every step and reproduces the current legal state exactly** (the same quantities as the assertion above, plus the legal action set of `Derived`); then the result carries `Approximate{MultiwayStreetRoot{folded_this_street, dead_this_street}}`. Otherwise the decision is `Unsupported{UnsupportedHistory{reason: "multiway street root not reproducible at step k"}}` with equity only. Worked cases: "A bets 50, B folds, C raises to 150, A to act" projects to A Bet(50), C Raise(150): legal, reproduced, admitted with `dead = 0`; "A bets 50, C calls 50, B raises to 150, C folds, A to act" projects to A Bet(50), B Raise(150) with `dead = 50`: admitted; "A bets 50, B calls 50, C raises to 150, A folds, B to act" projects to B Call, C Raise(150): a call at an unbet root is illegal at step 1, `UnsupportedHistory`. No other multiway projection exists in phase 1.
- OOP/IP per section 2. `hero` is not a solve input; the payload is actor-owned, and the engine reads hero's node by ordinal path and actor.

### 10.3 Worker adapter (library mapping)
- `solve` -> `CardConfig{range: [oop, ip], flop, turn, river}` + `TreeConfig{initial_state, starting_pot, effective_stack, rake_rate, rake_cap, bet sizes, add_allin_threshold, force_allin_threshold, merging_threshold}` -> `ActionTree`; then `ActionTree::add_line` for every inserted observed size and `ActionTree::remove_line` for the wager cap (section 4.6), then the tree cross-check against `tree.materialized` (`tree_mismatch` on any difference); `memory_usage()` before allocation: f32 when the f32 estimate `<= 4 GiB`, else 16-bit when the compressed estimate `<= 8 GiB`, else `error{tree_too_large}` **(spec decision)**; the worker also refuses when `estimate * 1.25 > memory_limit_bytes` (headroom for tree, scratch, extraction and serialization). Staged locks applied with `lock_current_strategy` at each locked node before the first iteration; `solve_step` loop with deadline and cancel checks between iterations; `finalize`; `apply_history`; for every decision node of the current street: navigate, `cache_normalized_weights()`, `strategy()`, `expected_values_detail(current_player())`, `available` from reach and blocking; `back_to_root` between nodes.
- Matrix contract: the library returns action-major arrays over its compact private-hand list for the player to act; the adapter converts library index -> named combo -> `ComboIndex` and transposes to combo-major `[1326][action]`, zero-filled and masked for combos outside the compact list. EV is actor-owned at each node (requesting the other player's EV is a different shape and is not used).
- **`normalize_ev` is the identity (spec decision, from the pinned `interpreter.rs`)**: `expected_values_detail(player)` for the current player returns `cfv * normalizer * w_raw / w_normalized + starting_pot / 2 + node.amount + bias`, with fold rows set to 0 and `bias = max(0, own total wager - opponent total wager) = 0` for the player to act. The library's counterfactual value is relative to owning half the starting pot, so the returned value is `E[final stack | action, combo] - (effective stack - own contribution since the root)` = `E[final stack] - current remaining stack`, exactly the section-2 convention (fold = 0 by the same identity); rake enters through the terminal payoffs and needs no adjustment. No pot or commitment offset is applied. This derivation is gated on the contract test `ev_convention_non_root_payoffs` (section 13.2) with signed positive, negative, check and fold payoffs at non-root nodes; if the test disagrees, the adapter is wrong, not the convention.
- Rake: `rake_rate > 0` and `rake_cap = cap_mchips as f64 / 1000.0 > 0` for `is_raked()`; `TimeCharge` sends 0/0.
- Memory: the process is warm (allocations of the previous game are dropped after the terminal `result`; no arena reuse is guaranteed). The engine places the worker in a Windows job object with `JOB_OBJECT_LIMIT_PROCESS_MEMORY = 16 GiB` **(spec decision)** so a runaway allocation kills the worker, not the machine. An abnormal exit (OOM abort cannot be caught) is a typed `WorkerExit{code}` failure in the engine.
- `bunching`: no protocol field in phase 1; capability reserved (`capabilities` list) because R8 measured x27-x40 solve time with bunching on turn/river and did not attempt the flop.

### 10.4 Cache (`cache`): pot-normalized street solutions
Policy, not a per-hand EV-error guarantee; nothing here changes `HandState` chips or the board used for a live solve. Authority: outline §3 as amended today: **Exact means identical normalized inputs, no bucketing; a hit whose SPR differs by at most 2% (geometric grid) is allowed and is always `Approximate{SprBucketed}`.**
- **Reference state**: `P = pot` of the effective tree (section 10.2), `eff = min(stack_oop, stack_ip)`, `SPR = eff / P` as the reduced rational `eff : P`. Every monetary quantity is expressed relative to `P`.
- **Key** = sha256 of the canonical serialization of: `schema_version` (3); `solver_commit`; `adapter_version`; `rules_version`; canonical board `(unordered flop, ordered turn)` (river solutions are not cached); `root_street`; `spr_bucket` = `round(ln(SPR) / ln(1.02))` (geometric grid, step 2%, a candidate index only, never a label) **(spec decision)**; `tree_signature` (section 4.6: nominal menus as pot fractions, inserted sizes as reduced pot-fraction rationals at their ordinal paths); rake `(rate as f32 bits, cap_mchips : 1000 P as a reduced rational, collection rule version)`; `range_hash_oop`, `range_hash_ip` (`hash_scaled`, section 2); `model` = `baseline` or `locked{fingerprint}`. Absent from the key: seat ids, hand id, `bb_chips`, raw chip pot/stacks, the wager quantum (its effect is the chip rounding of menu sizes, handled at lookup), hero, `target_bp`, requested path. All action amounts in the hash are normalized; a raw-chip signature would undo the design.
- **Payload** (`CacheEntry`): key fields, exact source inputs (`pot`, both stacks, `SPR` rational, `bb_chips`, `quantum/P`, canonical ranges), the **realized materialized tree** (section 4.6, chip amounts and their reduced `to : P` rationals per action), every exported `NodeStrategy` keyed by **ordinal path** with `ev_over_P` instead of chips, `covered_paths`, root ranges, `exploitability_over_P` (raw), `target_bp` of the solve, iterations, elapsed, mode, inherited `reasons`, `created`, `last_hit`. Never a user `Recommendation` or request id.
- **Lookup**: compute the query key for buckets `b - 1`, `b`, `b + 1`; candidates must match every non-SPR key field exactly (a different canonical board, range hash, tree signature, rake or version is a miss, never a substitute). For each candidate: (1) **topology**: the query's materialized tree and the entry's must have equal topology (section 2); otherwise the candidate is rejected (this is how a min-raise, all-in, merge, cap or rake-cap boundary crossed by rounding is detected: it changes the topology or the node set); (2) **SPR**: `delta = abs(SPR_query - SPR_entry) / SPR_entry`; `delta > 0.02` rejects; (3) **menu deviation**: over every action of every node, `dev = abs(to_query / P_query - to_entry / P_entry)` in pot units, `max_delta_pct = 100 * max(dev)`; a `max(dev) > 0.05` rejects (pots under ten quanta) **(spec decision)**. Among surviving candidates choose the smallest `delta`, then the smallest `max(dev)`, then the best accuracy. Label: **`Exact` iff `delta == 0` (SPR rationals equal) and `max(dev) == 0` (identical realized trees) and the accuracy filter passes and the entry inherits no reasons**; otherwise `Approximate` with exactly the reasons incurred: `SprBucketed{actual, used}` iff `delta > 0`, `MenuRounded{max_delta_pct}` iff `max(dev) > 0`, plus inherited ones. Then select the requested node by the query's ordinal path (section 2) and validate its actor against `Derived.to_act`; a path outside `covered_paths` or an actor mismatch is a miss for that node only (a check prefix or a different covered path selects its own node from the same entry). Inverse-map suits, take action chip amounts from the **query's** materialized tree at the same ordinal positions (never re-rounded from the entry), `ev_chips = ev_over_P * P_query`, run `validate_solution` (section 4.5), validate the menu against `Derived.legal`. The realized menu sizes are disclosed in `assumptions.notes`. Coverage and assumptions are rebuilt from the query's provenance plus the entry's inherited reasons; an entry can never upgrade a source reason.
- **Accuracy filter** (raw, never on rounded bp): an entry serves a request at target `t` iff `exploitability_over_P <= t / 10000`; an entry with `exploitability_over_P = 0.005049` (displayed 50 bp) does **not** serve a 50-bp request and does serve a 51-bp one. A validated entry above the target is served as `Provisional` (with its own reasons) while a live solve refines within the deadline; `DeadlineBestSoFar` entries are stored with their raw exploitability and never certify the current request's timing.
- **Replacement**: at most two entries per `(key)` cell: closest SPR and best accuracy; a dominated entry (both farther and less accurate) is replaced deterministically. Version mismatch is a miss.
- **Scale check** (contract, section 13.1 T4): pot/stack/cap 100/500/5000 mchips and 200/1000/10000 mchips with identical ranges, proportional quantum and identical fractional tree hit the same entry with identical frequencies and doubled chip EV, `Exact` (identical SPR rationals, identical realized fractions), at the root **and** at a non-root node reached by Bet(50)/Call versus Bet(100)/Call, for both actors; 100/500 versus 103/515 with a one-chip quantum has SPR `5 : 1` in both and rounds a 0.5-pot bet to 50 versus 52 chips (52/103 = 0.5049): `Approximate{MenuRounded{0.49}}`, never `Exact`; 100/500 versus 100/508 (SPR 5.08, `delta = 1.6%`) is `SprBucketed` (and `MenuRounded` only if some realized size actually differs); 100/500 versus 100/511 (`delta = 2.2%`) misses; a 20-chip pot rounding a 0.33-pot bet from 6.6 to 7 chips (0.35, `dev = 0.02`) is `MenuRounded{2.0}`; an exactly representable half-pot bet at `P = 20` (10 chips) carries no `MenuRounded`; a different canonical board always misses.
- **Storage**: `%LOCALAPPDATA%\PokerAI\cache\v3\<key[0..2]>\<key>.bin`, `bincode` + `zstd`, header with payload sha256, schema and `proto_version`; bounded sizes (64 MiB compressed, 256 MiB decoded); any read/decode error is a miss and the file is deleted (deletion failure is non-fatal); writes go through a temporary sibling and atomic rename on the `cache-writer` thread after validation; a write failure preserves the recommendation. Quota `cache_quota_bytes` default 10 GiB; eviction removes the oldest `last_hit` first **(spec decision)**.

### 10.5 Pre-solver (`cache::Presolver`)
- Runs when no hand is in progress (`Complete`/`Abandoned`, or none begun) and no request has arrived for 30 s; a live request cancels the running job at its next iteration boundary (lost work accepted); `presolver_pause()` stops scheduling.
- Jobs are `solve{background: true, deadline_ms: 600000}` with `flop_fast_v1`, `target_bp` 50; retries 3 with 30 s backoff; task status (`pending`, `done`, `failed{n}`) is persisted in `queue.json` keyed by the normalized game identity (cache key without `spr_bucket` plus the exact scenario SPR); a task is `done` only when a valid entry at target exists.
- Scenarios **(spec decision, explicit list)**, ranges from the store's replay of the line using the active bundle of highest precedence (the chart bundles while V9 is pending: entries are keyed by range hash, so chart-range entries serve chart-range queries and a later PokerData bundle simply produces new entries), chips from the last-used `HandConfig`: tier 1 (100bb): SRP BTN-open/BB-call, CO-open/BB-call, HJ-open/BB-call, UTG-open/BB-call; tier 2 (100bb): SRP SB-open/BB-call, BTN-open/SB-call, CO-open/BTN-call, HJ-open/BTN-call; 3-bet pots BTN-open/BB-3bet/BTN-call, CO-open/BTN-3bet/CO-call, BTN-open/SB-3bet/BTN-call, HJ-open/BTN-3bet/HJ-call; tier 3: the same twelve at 200bb. Within a tier the outer loop runs over canonical flops (orbit 24 before 12 before 4, then rank order) and the inner loop over the tier's scenarios, so scenarios advance together.
- Throughput planning (R8): `1,755 flops x 6.2 s = 3.0 h` per scenario with `flop_fast_v1` (about 12 h of idle time for tier 1, 72 h for all 24 scenarios); `1,755 x 42 s = 20.5 h` per scenario with `flop_full_v1` (phase 2). Both figures were measured with 179x264-hand ranges; store ranges are larger and unmeasured (V3). Status reports estimated remaining work from measured time-to-target and the hit rate from the decision log. Full completion, a general scheduler and guaranteed coverage are not release requirements.

### 10.6 Deadline and best-so-far
- Street budgets from section 7 are converted to `deadline_ms` at send time; the worker's stop rule and `best_so_far` conditions are in section 7.
- The engine labels `best_so_far` as `DeadlineBestSoFar{reached_bp}` and still returns per-action EVs; a `best_so_far` without a complete validated payload is impossible by protocol (`solution` is present or the status is `error`).
- Retry policy is in section 12; a retry is admitted only if time and memory admission pass.

---

## 11. Exploit slice (OPTIONAL in phase 1; built only after sections 5-10 are validated and gates pass with time remaining; otherwise phase 2)

- Inputs: per-seat `SeatTag` in {unknown, nit, TAG, loose-passive, calling-station, LAG, maniac}; `QuickFact` in {never_folds_river, rarely_bluffs, limps_a_lot, over_3bets, folds_to_pressure}. Any change increments `model_revision`; baseline artifacts (cache entries, replay snapshots, decision logs) carry `model_revision = 0` and are never overwritten by modelled ones.
- Preflop: tags scale the opponent's reach weights per action class and position using the R4 §3.1 multiplier table; `limps_a_lot` adds 20 pp open-limp probability; `over_3bets` sets the 3-bet factor to 1.75. Applied in a separate modelled replay (`model_revision > 0`), never in the baseline replay. Defaults in `exploit_defaults.toml` with `calibrated = false`. Precedence: a fact overrides the tag default for the same quantity; two facts on the same quantity take the more extreme value **(spec decision)**.
- River locks (HU river decisions only): from the baseline solve's street nodes, the opponent's locked strategy at their river nodes: `never_folds_river` -> fold target 5% raw; `rarely_bluffs` -> `kB = 0.25` on low-showdown-value betting combos, mass moved to check; `folds_to_pressure` -> `+15 pp` folds versus the observed size class; tag-only defaults from R4 §3.3. Credibility shrinkage `c = 0.35` (tag) or `0.60` (fact): `target = F0 + c * (raw - F0)`; transformations keep every row a probability vector and preserve value actions; unreachable combos stay free.
- Evaluations (river solves are 4 ms): (1) `lock` (opponent model) + `solve` -> hero best response `sigma_br`; (2) `lock` (opponent model and hero fixed at `sigma_gto`) + `solve` -> `EV_gto`; (3) `lock` (opponent model and hero fixed at `sigma_x = (1 - alpha) sigma_gto + alpha sigma_br`, `alpha = 0.25` when a tag or fact applies, `0` for unknown/TAG without facts) + `solve` -> `EV_x`. Both complete policies are evaluated under the same frozen opponent model and range distribution; `ev_delta = EV_x - EV_gto` for hero's actual combo. If any lock is infeasible the delta is omitted and only the experimental action mix is described.
- Display: GTO headline versus exploit headline with `ev_delta_bb`; the panel is labelled "uncalibrated defaults" and is visually separate. No best-response audit in phase 1.
- `ExploitAdvice { alpha, model_revision, model_summary, gto_action, exploit_action, ev_delta_bb: Option<f32>, locked_nodes: u16 }`.

---

## 12. Error handling

| Failure | Handling |
|---|---|
| Illegal action entered | Rejected by `apply_action`; UI disables illegal keys; undo restores the previous snapshot under a new revision |
| Stack/pot inconsistency, board duplicate, unsupported format (re-straddle, short straddle post, straddle with fewer than six dealt seats, two dealt seats) | Rejected with a message (`FormatUnsupported` for formats); state unchanged |
| History not representable in the tree; multiway street root whose projection does not reproduce the current state (section 10.2) | `Unsupported{UnsupportedHistory{reason}}`; equity shown |
| Worker `tree_mismatch` (library tree differs from the engine's materialized tree) | `Unsupported{EngineError{retryable: false}}`; logged with both trees; a rules bug, never retried with another template |
| Worker `no_iteration` (no measurement fit before the stop point) | Not retried on the same template; the `_min` template is tried only if time admission passes; else the watchdog path (`DeadlineExceeded` or the retained payload) |
| Worker exit (crash, OOM abort), protocol error, no `progress` for 5 s during `Solving` **(spec decision: heartbeat)** | Typed failure; kill, reap, respawn, validate `ready`; retry once with the `_min` template only if time and memory admission pass; else `Unsupported{EngineError{message, retryable}}` |
| Worker `tree_too_large` / `out_of_memory` | Retry once with the `_min` template under the same admission; then `Unsupported{TreeTooLarge}` |
| Street deadline reached | `best_so_far` -> `Approximate{DeadlineBestSoFar}`; violation logged |
| Final delivery deadline reached | Watchdog emits the retained payload or `Unsupported{DeadlineExceeded{stage}}`; late replies discarded |
| Cancel not confirmed (`result{cancelled}`) within 1.5 s | Kill and respawn the worker; admission released on confirmed exit |
| Missing depth / straddle tree / rake profile / short-handed tree | Nearest with the corresponding `ApproxReason`; never silent |
| Missing preflop node (current decision) | `Unsupported{MissingPreflopNode}` with the key in assumptions |
| Missing preflop node (replay of a prior action) | `Approximate{UnconditionedPriorStreet}` (section 9.3) |
| Empty or incompatible ranges (no jointly compatible holdings) | `Unsupported{InvalidRanges}` |
| Corrupt, oversized or version-mismatched cache entry | Miss, delete (failure non-fatal), re-solve; counted in the log |
| Cache or log write failure, disk full, read-only directory | Recommendation delivered unchanged; write skipped and logged once per session |
| Preflop bundle failing hash, schema or content validation | Quarantined; startup banner; remaining bundles active; charts remain usable |
| Stale event or result (identity mismatch) | Discarded by the UI and by `engine-main`; never written to replay snapshots |
| Config changed mid-hand | Not applied to the active hand (frozen `HandConfig`); applies from the next hand |
| `proto_version` / commit / adapter / AVX2 mismatch at `ready` | Every request answers `EngineError("worker/proto version mismatch")` until rebuilt |
| Suspend/resume during a request | Request expired; `Unsupported{DeadlineExceeded}`; worker restarted if a job was running |

---

## 13. Testing strategy

### 13.0 Fixture inventory (nothing below exists today; every entry is to be created in the stated week)
| Path | Producer | Week | Acceptance data lives in |
|---|---|---|---|
| `fixtures/worker/{river_two_combo,flop_cancel,flop_best_so_far,lock_river}.jsonl` | `tools/gen_worker_fixtures.py` from the definitions in section 4.5 | 1 | section 4.5 (range definitions, expected messages) |
| `fixtures/hands/*.json` (200 PokerKit hands) | `tools/gen_fixtures.py` | 1 | section 13.1 `state_machine_pokerkit_fixtures` |
| `fixtures/eval/phevaluator_5card.bin`, 7-card samples | `tools/gen_eval_oracle.py` | 1 | section 13.1 |
| `fixtures/charts/{pokercoaching_100,rangeconverter_200}.json` | `tools/chart_ingest.py` (manual transcription, `ChartRounded`) | 1 | section 8.2 envelope |
| `fixtures/preflop/synthetic_v2/*.json` | hand-written from R7 §4's documented response shape, marked synthetic | 1 | section 8.2 |
| `fixtures/preflop/pokerdata_sample/*.json` | V9 (authorized sample through `tools/pokerdata_convert.py`) | conditional on V9 | section 8.2, R7 A1-A6 |
| `fixtures/solver/basic_0p3.json` | V1: the library's own `solve()` on `examples/basic.rs` at the pinned commit | 1 | section 13.2 `pinned_example_fixture` |
| `fixtures/hands/e2e/*.json` (50 synthetic hands) | `tools/gen_fixtures.py` with the class list of section 13.5 | 1 | section 13.5 |
| `bench/spots/*.json` | `bench gen-spots` from chart-replay ranges (baseline) and store ranges (V9) | 1 | section 13.5 |
| `engine/tests/golden/*.json` | recorded from the first green run after hand-checked values | 2 | section 13.3 (numeric values given there) |
| `docs/bench/<date>-i7-13700K.md` | `bench run` | 2 | section 13.5 gate |

### 13.1 Unit (Rust, `cargo test`; exhaustive suites behind `--features exhaustive`, run by `bench oracle` before release)
| Crate | Test | Assertion |
|---|---|---|
| core-model | `state_machine_pokerkit_fixtures` | Replays `fixtures/hands/*.json` (200 hands generated by `tools/gen_fixtures.py` with PokerKit: UTG straddle at 1/2/4 and 2/5/10, side pots, 2 and 3 all-ins, short all-ins, uncalled returns) and matches pot, settled pots, stacks, returns and legal actions at every step |
| core-model | `straddle_action_order_utg` | Preflop HJ, CO, BTN, SB, BB, UTG; postflop SB, BB, UTG, HJ, CO, BTN; posts `(0.25, 0.5, 1)` and `(0.2, 0.5, 1)` reported |
| core-model | `min_raise_and_short_allin_no_reopen` | A single short all-in does not reopen action for a player who already acted |
| core-model | `cumulative_short_allins_reopen` | bb 2: raise to 10, all-in 14, all-in 17: cumulative 7 < 8 does not reopen; raise to 10, all-in 15, all-in 19: cumulative 9 >= 8 reopens |
| core-model | `side_pot_three_allins` (V14) | Contributions 50/100/200 by exactly three players (starting stacks 50/100/200, total 350): main 150 (3 eligible), side 100 (2 eligible), 100 returned to the 200 stack; the section-4.3 invariant `stacks + live + pots + rake = 350` holds after every action, after the refund (stacks 100, pots 250) and after settlement |
| core-model | `side_pot_two_contested` | Starting stacks 50/100/200/200 (total 550), all four all-in: main 200 (4 eligible), side 150 (3 eligible), side 200 (2 eligible), no refund; pots sum to 550; invariant holds after each action |
| core-model | `allin_runout_single_survivor` | 100-chip player shoves, 300-chip player calls, everyone else folded: street closes, no refund, `Complete{AllInRunout}` although one survivor retains 200 chips; the same with the caller shorter: refund to the shover, then `AllInRunout` |
| core-model | `dealt_seats_3_to_6` | 3 dealt seats: BTN, SB, BB only; 4: CO added; 5: HJ, CO; preflop and postflop orders per section 4.3; 2 dealt seats rejected `FormatUnsupported`; straddle with 5 dealt seats rejected |
| core-model | `street_root_reconstruction` (T2) | From root pot 100 / stacks 500: OOP bet to 50 -> pot 150, 450/500; IP raise to 150 -> pot 300, 450/350, OOP call cost 100; OOP call -> 400, 350/350; snapshot at the raise decision reports root 100/500, never 300; check-prefix actor changes at unchanged pot; hero all-in produces `NoDecision`; third all-in stays `MultiwayEv` even with a HU side pot |
| core-model | `multiway_root_projection` | The three worked cases of section 10.2: "A 50, B fold, C 150, A" admitted with dead 0; "A 50, C call, B 150, C fold, A" admitted with dead 50 and root pot + 50; "A 50, B call, C 150, A fold, B" -> `RootError::ProjectionNotReproducing{step: 1}`; every admitted projection replays to the exact `Derived` of the real state |
| core-model | `card_parser_roundtrip` | "AsKd" and board strings parse and print identically; invalid strings rejected |
| core-ranges | `range_roundtrip_pio_strings` | Parse -> vector -> string -> vector identical for "AKs:0.5, 77+, A5o"; dash ranges written high-to-low |
| core-ranges | `class_expansion_multiplicity` | Pair 6, suited 4, offsuit 12; mass of "random" = 1326 |
| core-ranges | `public_blocking_board_only` | Combos containing board cards have weight 0; combos containing hero's cards keep their weight in public ranges |
| core-ranges | `hero_conditioned_copy` | The hero-conditioned copy zeroes combos sharing hero's cards and leaves the public range untouched |
| core-ranges | `range_hash_scale_invariant` | `hash_scaled(r) == hash_scaled(0.5 * r) == hash_scaled(4.0 * r)` (ratio-preserving powers of two); `hash_scaled(0.37 * r)` is not asserted equal (whatever it is, an unequal hash is a miss); for a range with at least two supported combos, changing one weight by anything other than a common rescaling alters the hash; a one-combo range hashes equal at weights 0.2 and 0.9 |
| core-eval | `terminal_payoff_equity_times_pot` | River, matched pot 100, no further betting: per-combo terminal payoff under the section-2 convention equals `equity_actual_combo * 100` for both seats (exact enumeration); swapping seats leaves every value unchanged; with cap rake 5 (5000 mchips) every payoff is `equity * 95` |
| core-iso | `iso_class_count_1755` | Exactly 1,755 canonical flops |
| core-iso | `iso_orbit_sizes` | Orbits in {4, 12, 24}; 52 + 3,744 + 18,304 = 22,100 |
| core-iso | `iso_stabilizer_tiebreak` | Paired and monotone flops with permuted ranges canonicalize to the same key; distinct turn/river orders give distinct keys; inverse map round-trips every combo vector |
| core-eval | `eval_vs_phevaluator_full_5card` | All 2,598,960 5-card hands rank-order-equivalent to `fixtures/eval/phevaluator_5card.bin` |
| core-eval | `eval_vs_phevaluator_random_7card` | 200,000 fixed-seed 7-card samples match the oracle (10,000,000 under `exhaustive`) |
| core-eval | `equity_mc_within_standard_error` | Fixed seed; MC estimate within 4 standard errors of exact on 20 spots; reported `std_err` matches the binomial bound |
| core-eval | `equity_joint_disjoint_sampling` | Multiway samples never share a card; `InvalidRanges` when no compatible assignment exists; per-pot shares sum to 1 per pot |
| core-eval | `equity_budget_respected` | Cancel flag and budget stop enumeration within 50 ms of the budget |
| core-preflop | `pokerdata_units_source_scaling` (T1) | 1.84 source SB -> 0.92 source bb at 1/2 and at 2/5 under `decision_incremental_verified`; under `net_hand_start_verified` an SB node with fold EV -1 and raise EV 1.84 gives `ev_inc = 2.84` and the fold cross-check passes, while a fold EV of -0.5 makes the node unloadable; under `absolute_stack_verified` `stack_dec` is subtracted at a no-fold BB-vs-limp node; `unverified` suppresses every EV; re-raise node; straddle-mapped node converts with `S` |
| core-preflop | `pokerdata_schema_mapping` | Synthetic fixture (section 13.0) until V9, then the sample: dense envelope with 169 entries per action, sparse zero-weight EV kept, absent EV is `null`/`None`, `combos` ignored, SB-limp/BB-check node loads, every explicit fold in history, an all-zero class row listed in `unreachable_classes` loads and yields `HeroComboOutOfSupport` for that class |
| core-preflop | `pokerdata_action_path_lookup` | Provider path -> `PreflopNodeKey` -> node for RFI, vs-3bet, vs-4bet, squeeze, SB limp; CO/SB cold-call-vs-3bet absent -> `MissingPreflopNode` |
| core-preflop | `depth_bucket_labels_per_prefix` | 97bb -> 100 `DepthBucket{prominent: false}`; 120bb -> 100 prominent; 125bb -> 150 (tie deeper); 260bb -> 200 clamped; a later fold does not change an earlier prefix's depth; actor 100 versus opponent 104 -> depth 100 with `AsymmetricStacks{prominent: false}` and never `Exact`; 100 versus 110 -> prominent; two bundles ranked equal resolve to the smaller `bundle_id` |
| core-preflop | `straddle_mapping_labels` | Virtual roles, `S` as unit, `StraddleMapped{posts}` at 1/2/4 and 2/5/10; short post and re-straddle rejected |
| core-preflop | `rake_profile_ordering` | 10%/$6 cap at 2/5 maps to "5% cap 0.5bb" with `RakeProfileMapped`; `TimeCharge` prefers unraked |
| core-preflop | `bundle_validation_quarantine` | Valid hash with malformed JSON, wrong shape, non-finite value, invalid token, a class row summing to 0.4, an all-zero row not listed in `unreachable_classes`: bundle quarantined, others active |
| core-replay | `replay_bayes_two_combos` (T3) | Weights (1,1); likelihoods (.8,.2), (.25,1), (.9,.1) -> masses proportional to (.18,.02), posterior (0.9, 0.1) for every seat including hero, marginal rescaled to max 1 after each update with `log_reach = ln(0.18)`; an inserted observed bet changes the posterior (forcing 1 fails); a combo at relative reach 1e-30 stays positive (no threshold) |
| core-replay | `replay_off_tree_pseudo_harmonic` | 73 into 100 with sizes 50/100: `f_A = 0.468`, `f_B = 0.532`, deviation 0.23, prominent; for a combo with `P_A = 0.9`, `P_B = 0.1` the branch posterior is `pi_A = 0.468 * 0.9 / (0.468 * 0.9 + 0.532 * 0.1) = 0.888`; a later on-menu action multiplies each branch once and the marginal equals the hand-computed sum; node translation mixes strategies with `pi` and EV with the action-conditional rule of section 8.4 |
| core-replay | `replay_branch_cap_residual` | Three successive off-menu wagers by one seat (8 branches) are capped to 4: the four heaviest survive, the rest form one residual branch whose mass equals the dropped sum, `BranchResidual{residual_mass_pct}` is reported, total mass is unchanged, later actions leave the residual fixed |
| core-replay | `replay_missing_continuation` | Deleted continuation -> the range as conditioned up to that point is kept (not the pre-street range), `UnconditionedPriorStreet{cause: "uncovered path <ordinal path>"}` persists, no invented branch, reach applied once |
| core-replay | `replay_snapshot_prefix_reuse` | A flop snapshot solved at prefix [check] with menu 50/100: later flop actions [check, bet 73, call] condition through the snapshot (check from its node, 73 translated over 50/100, the call from the covered continuation nodes); with a truncated export covering only the requested node, conditioning stops after the check with the reason and the check's conditioning is kept |
| core-replay | `replay_snapshot_compatibility` | Two solves on one street plus undo: only the compatible revision's snapshot is used; among two compatible snapshots the one covering the longer observed prefix wins; a cache-hit `Final` registers a snapshot exactly like a live result |
| core-replay | `replay_hero_out_of_support` | Positive total mass, zero hero-combo weight -> `HeroComboOutOfSupport` with `range_mix` present and `actions` carrying no frequency or EV |
| cache | `cache_key_structural_identity` (T4) | 100/500 and 200/1000 (proportional cap and quantum, identical ranges and fractional tree) hit the same entry `Exact` with identical frequencies and doubled EV at the root and at the node after Bet(50)/Call versus Bet(100)/Call, for both actors; a check prefix and a different covered path select their own nodes from the same entry; an uncovered path, a wrong actor, a changed adapter version, a lock fingerprint, a rake rule, a different range hash, a different canonical flop and a rounding that changes topology (min-raise or all-in boundary) miss; SPR 5.00 versus 5.08 is `SprBucketed`, 5.00 versus 5.11 misses; 103/515 is `MenuRounded{0.49}` and not `Exact`; a 20-chip pot with a 6.6 -> 7 rounding carries `MenuRounded{2.0}`; an exactly representable half-pot bet at `P = 20` carries no reason; a hit is used for replay of the next street |
| cache | `cache_inherited_reasons_survive` | A stored `ChartRounded`/`DeadlineBestSoFar`/`UnconditionedPriorStreet` reason appears on every hit; an entry with raw `exploitability_over_P = 0.005049` (displayed 50 bp) is `Provisional` for a 50-bp request and serves a 51-bp request; a stricter target never receives a stale `Exact` |
| cache | `cache_payload_validated` | An entry whose probability rows do not sum to 1 for an available combo, or whose `covered_paths` disagree with its nodes, is a miss and is deleted |
| cache | `cache_corrupt_entry_deleted` | Flipped byte -> miss, file removed; deletion failure non-fatal |
| cache | `cache_atomic_write_and_quota` | Temp-and-rename; oversized entry rejected; eviction by oldest `last_hit` under quota |

### 13.2 Solver contract (`solver-worker` integration tests, spawn the binary)
| Test | Spot | Assertion |
|---|---|---|
| `river_polarized_vs_bluffcatcher_analytic` | Qs Jd 7h 3c 2d; OOP (caller) AA weight 1; IP (bettor) QQ weight 1 (3 combos) + 54o weight 0.25 (12 combos, mass 3); pot 100, stacks 100; OOP check only (no leads, no raises), IP check or pot bet; no rake | IP bets 100% of QQ and 50 +- 3 pp of 54o; OOP calls 50 +- 3 pp; IP range EV 75 +- 1, OOP 25 +- 1 chips; exploitability <= 0.1% pot |
| `ev_convention_non_root_payoffs` | Same spot plus a turn-root spot with a check-check line and a river node reached after Bet(100) where OOP faces the bet | The identity `normalize_ev` yields fold = 0, the analytic call value `equity * 300 - 100` (negative for the bluff-catcher's weak combos, positive for strong ones) at the facing node and `equity * pot` at check nodes; no offset passes all four signed cases |
| `river_check_only_terminal_oracle` | River `Qs Jd 7h 3c 2d`, pot 100, stacks 100 (positive, as the pinned validation requires), empty bet menus for both players, `add_allin_threshold = 0.0` (no all-in added); the only line is check-check | Per-combo EV(check) at the OOP root and at IP's node equals `equity_actual_combo * 100` within 1e-3 against the `core-eval` exact enumeration; swapping the two ranges between seats leaves every value unchanged |
| `ev_conservation` | Identical ranges, river, no rake, `river_std_v1` | `EV_OOP + EV_IP = pot +- 0.5%` summed over the root ranges |
| `tree_materialization_matches_library` | Every template of section 10.1 at (P, stack) = (100, 100), (100, 150), (180, 910), (100, 149) and (100, 151) | The engine's `materialized` list equals the library's enumerated tree; at `add_allin_threshold = 1.5`, `P = 100`, stack 150 the all-in is added (`150 <= 150`) and at stack 151 it is not; at `force_allin_threshold = 0.15` a wager leaving exactly `round(new_pot * 0.15)` behind is forced and one chip more is not; a deliberately altered `materialized` entry produces `result{error{tree_mismatch}}` |
| `wager_cap_remove_lines` | `flop_min_v1` (cap 1) and `turn_std_v1` (cap 3) | After the cap is reached only fold/call/all-in remain at every node (`available_actions()`); an observed prefix with two wagers under cap 1 is kept intact and the decision node offers fold/call/all-in only |
| `pinned_example_fixture` | `examples/basic.rs` ranges and board at the pinned commit; `fixtures/solver/basic_0p3.json` **to be created** in V1 from the library's own `solve()` | Per-combo strategy and EV at every flop node within 1e-3 through the worker protocol: an adapter regression test, not an independent solver certificate (the wasm-postflop README comparison cannot be reproduced: its ranges are unpublished, R8 §3) |
| `suit_permutation_metamorphic` | Any flop spot; monotone and paired boards included | Permuting suits of board and ranges gives identical strategies after inverse mapping (max abs diff <= 1e-4) |
| `rake_cap_applied` | River spot with 5%/cap where the cap binds | Terminal payoff differs from unraked by exactly the cap; `TimeCharge` matches unraked |
| `combo_matrix_two_named` | Ranges with "AsKs" and "7h7d" only, different action rows | Rows non-zero only at those indices; rows differ; `available` false elsewhere; transpose errors fail |
| `cancel_between_iterations` | Flop spot; the same cancel sent during `Building` and during `Extracting` | `ack{accepted}` <= 50 ms; in `Solving`, `result{cancelled}` within one iteration plus one exploitability pass (<= 1.0 s); in `Building` within one build step; in `Extracting` within one node's extraction after `finalize`; never a second terminal; a cancel after `result` answers `already_finished` |
| `deadline_best_so_far_bounded` | Turn spot (measured 0.17 s to target), `deadline_ms = 1000`, margin 200, `target_bp = 1` (not reachable in time) | `best_so_far` with `elapsed_ms <= 1000`, a complete validated solution and a measured `exploitability_chips` |
| `deadline_no_iteration` | Flop spot, `deadline_ms = 300`, `extraction_margin_ms = 600` (no iteration can fit: `finalize` alone is 0.2-0.35 s measured) | `result{error{no_iteration, retryable: false}}` with no `solution`, within 300 ms plus one build step; the engine test `final_delivery_independent_of_worker` (section 13.3) covers the 15 s bound |
| `exact_size_insertion_no_prune` | Villain flop bet 73 into 100, menu 50 | Tree contains both 50 and 73 at that node plus check; opponent's posterior after the 73 bet is not uniform over the root range; the requested node is the one after 73 |
| `lock_lifecycle` | River spot | `lock` then matching `solve`: locked rows unchanged, hero response differs; `lock` with another `spot` -> `lock_mismatch`; `lock` during `Solving` -> `rejected`; lock cleared after cancel |
| `protocol_rejections` | Malformed lines | Unknown tag, unknown field, 1325-element range, non-finite number, duplicate id, busy `solve`, oversized line, lock row summing to 0.4: typed rejection with `reason`, no work, worker stays alive; `progress` during `building` carries `exploitability_chips: null`; EOF -> exit 0 within 2 s; a second `lock` answers `staged, replaced: true` |
| `ready_reports_features` | Startup | `ready` contains `avx2` in `build_features`, `threads` equals the launch argument |
| `memory_admission` | Synthetic tree above the limit | `tree_too_large` with `estimate_bytes`, no allocation |

### 13.3 Engine goldens (`engine/tests/golden/*.json`, to be created in week 2 with the values below)
`coverage_classification_golden` (HU flop; 3-way flop; third player all-in; two preflop folds then HU flop; the three projection cases of section 10.2 with their labels `MultiwayStreetRoot{1, 0}`, `MultiwayStreetRoot{1, 50}` and `UnsupportedHistory`; opponent all-in; hero all-in; hero cards unknown; two dealt seats), `bet_translation_golden` (below min, between, above max with and without all-in, single size, illegal mapped action moved with `MovedProbability` and no EV on the created destination), `tree_builder_golden` (in-tree detection; insertion alongside menus; duplicates merged; `tree_signature` stability across chip scales; the materialized list at both scales; `UnsupportedHistory`), `replay_weights_golden` (three-seat preflop history; expected 1326 vectors and `log_reach`), `recommendation_assembly_golden` (headline only with complete EVs; no EV headline when one action lacks EV; highest-frequency headline wording for charts and for unverified PokerData; tie break; reason accumulation; `partial` on Unsupported; `range_mix` present under `HeroComboOutOfSupport`; an `Equity` event after `Final` enriches it and `Pending` never replaces `Ready`), `facing_allin_golden` (T1: AhAd on Qs Jd 7h 3c 2d versus QQ + 54o weight 1/12: equity 0.25, pot 100 before the jam, jam/call 73, W 246, `EV(call) = -11.5` unraked, `-12.75` with cap rake 5000 mchips; 54o weight 0.25: `+50` / `+47.5`; `-2.30 bb` at a 5-chip BB; hero's strategic range includes other hands and the headline uses AhAd), `experimental_surrogate_golden` (3-way flop: the surrogate's pot equals the total pot, stack the minimum, history empty, ranges the street-root public ranges, block separate, never in `actions`), `identity_race_golden` (T5 with a fake worker and clock: hand A revision 7, undo to a non-decision, hand B with the same displayed revision, re-request B; A's Fast/Progress/Final and both B replies out of order; no stale event, snapshot or result accepted; an `ack` never frees admission), `final_delivery_independent_of_worker` (fake worker that never replies, or replies `no_iteration`, or hangs in extraction: `Final` with the retained payload or `Unsupported{DeadlineExceeded}` is emitted at `t0 + 14.9 s` on the fake clock before any kill or restart completes; the retained payload is a `Provisional` when one exists).

### 13.4 UI
- Vitest + `@tauri-apps/api/mocks` `mockIPC`: `entry_flow_full_hand_keystrokes`, `begin_hand_stack_confirmation`, `undo_is_engine_command`, `stale_identity_discarded`, `illegal_keys_disabled`, `coverage_label_and_assumptions_render`, `progressive_fast_equity_provisional_final`, `no_decision_rendering`.
- One end-to-end WebDriver test on Windows (`tauri-driver`, `@wdio/tauri-service`, MSVC build): `e2e_full_hand_srp_flop_recommendation` enters a full SRP hand to a flop decision and asserts a `Final` with a coverage label and numeric EV within 15 s, using the chart bundles (no dependency on V9).

### 13.5 Bench gate (`bench`)
- Suites (**to be created** in week 1, section 13.0): `bench/spots/{river_std,river_min,turn_std,turn_min,flop_fast,flop_min}.json`, 5 spots each: **baseline set** with ranges from the chart bundles' replay (100bb PokerCoaching, 200bb RangeConverter; SRP BTN-vs-BB and CO-vs-BB, 3-bet BTN-vs-BB; broad and narrow, high and low SPR) and, conditional on V9, a **store set** of the same lines with PokerData ranges; every production and retry template; `--reps 5` cold and warm; threads {8, 16, 24}.
- Recorded-hand suite (**to be created** in week 1): `fixtures/hands/e2e/*.json`, 50 synthetic hands with a complete versioned input record (config, stacks, actions, board, expected coverage class and expected reasons). Designated **supported baseline fixtures** (numeric EV required, chart ranges): 8 HU flop SRP decisions (both depths, BTN-vs-BB and CO-vs-BB, in-tree and off-tree villain sizes), 6 HU turn decisions, 6 HU river decisions including 2 facing an all-in, 2 admitted multiway projections; the remaining 28 cover straddle mapping at 1/2/4 and 2/5/10 (expected `StraddleMapped` plus `ChartRounded`), a rejected projection (`UnsupportedHistory`), multiway (`MultiwayEv` with `experimental`), missing preflop nodes (`MissingPreflopNode`), hero out of support, and failure injections (worker OOM/EOF, malformed and oversized output, `tree_mismatch`, blocked cache I/O, slow allocation, no completed iteration, clock jump, suspend/resume) with their expected `Unsupported` reasons. Real-session logs are added later with the same record format.
- Report `docs/bench/<date>-i7-13700K.md` (**to be created** by the first `bench run`): p50/p95/max wall time to `target_bp` 50, time-to-target, peak total RSS, thread count, cancel latency, Exact/Approximate/Unsupported proportions, count of street and final deadline violations, per suite (baseline, store).
- **Baseline gate (independent of V9)**: on the baseline suites, river p95 <= 2 s, turn p95 <= 6 s, flop_fast p95 <= 10 s, cache hit p95 <= 0.5 s, e2e (admission to `Final`, cold cache) p95 <= 15 s, zero final-delivery violations in the fault-injection suite, numeric EV present with the expected coverage class for every designated supported baseline fixture, the T1 and analytic-river values within their stated tolerances. **Acquired-data gate (conditional on V9)**: the store suites meet the same latency bounds and `pokerdata_*` tests pass; failing it keeps the chart baseline as the release. A gate failure reduces template size or scope; it never changes the definition of success.

---

## 14. Phasing and critical-path validations

### 14.1 Week 1
MSVC workload installed by the user (section 3.6; core and worker work proceeds on the GNU toolchain meanwhile); workspace and `proto` with the generated wire fixtures (section 13.0); `core-model` with PokerKit fixtures incl. lifecycle, `AllInRunout`, dealt-seat rules, street-root snapshot and the projection rule; `core-ranges`; `core-iso`; `core-eval` with oracle tables (5-card full, 7-card sampled); V1 (worker builds with AVX2 at the pinned commit with the two patches; `basic_0p3.json` frozen); chart bundles ingested; `bench` river/turn/flop_fast with chart-replay ranges (V2/V3 baseline; store ranges added when V9 passes); worker state machine, tree materialization cross-check, locks, per-action EV, cancel, progress (V4); `core-preflop` with the chart fallback and the PokerData converter/adapter behind V9; `core-replay` with branch posteriors and snapshot prefix reuse; engine river/turn path with absolute deadlines, watchdog and retry; recorded-hand suite; minimal keyboard UI showing `Fast`, `Equity` and `Final` (MSVC build).

### 14.2 Week 2
Flop path; `cache` (normalized keys, materialized-tree comparison) and the resumable pre-solver queue (tier 1 running on chart-replay ranges, completion not required); prior-street bet translation; coverage labels and assumptions panel; undo as an engine command; decision log; goldens recorded; all tests green; baseline bench gate incl. fault injection; E2E test on the MSVC build; acquired-data gate if V9 has passed. Exploit slice only if all of the above is done and validated.

### 14.3 Phase 2 backlog
Full exploit layer (turn locks, best-response audit, observation updates); `flop_full_v1` pre-solves and more scenarios/depths; straddle and 250bb preflop trees (self-solved with HRC or a custom pack); bunching capability with its own preparation, memory and deadline tests; GPU batch pre-solving if a licensed backend appears; neural surrogate distilled from our own solves; multiway models; LAN/phone UI with the AGPL boundary review; quota and pre-solver dashboards.

### 14.4 Critical-path validations
| Id | Question | Method | Pass criterion | Blocks |
|---|---|---|---|---|
| V1 | postflop-solver builds on Windows 11 at the pinned commit with the two patches and `+avx2`; `ready` reports `avx2` | `cargo build --release -p solver-worker`; `river_polarized_vs_bluffcatcher_analytic`; `ready_reports_features` | Build and tests pass | everything postflop |
| V2 | Turn solve time for `turn_std_v1` with chart-replay ranges (baseline) and store ranges (when V9 passes) | `bench` turn spots | p95 <= 6 s (measured 0.17 s on the R8 analogue) | turn budget |
| V3 | Flop solve time and memory for `flop_fast_v1` with chart-replay ranges and, when V9 passes, store ranges (460-570 hands per side, unmeasured in R8) | `bench` flop spots, cold and warm, threads {8,16,24} | p95 <= 10 s; f32 estimate <= 4 GiB; otherwise reduce the template | flop budget, pre-solver rate |
| V4 | Worker API: lock before solve, actor-owned per-action EV at non-root nodes with the identity `normalize_ev`, cancel in every state, progress with `null` accuracy, engine materialization equal to the library tree at the equality boundaries, `remove_line` wager cap | contract tests of section 13.2 | All pass | deadline design, exploit slice, cache payload |
| V9 | PokerData (separate, conditional gate): written personal-offline-use rights; pack conversion to the normalized envelope with frequencies and EV; R7 A1 schema, A2 units and reference variant, A3 absolute sizes, A4 completeness, A5 consistency, A6 provenance | User emails the vendor and obtains the sample/pack; `tools/pokerdata_convert.py` + `pokerdata_schema_mapping`, `pokerdata_units_source_scaling`, `pokerdata_action_path_lookup` on the sample fixtures | Rights confirmed and A1-A6 pass; A2 result sets `ev_reference`; else chart fallback only | preflop EV, store bench suites; blocks nothing in the baseline gate |
| V14 | Settlement, lifecycle and reopening correctness of `core-model` | `side_pot_three_allins`, `side_pot_two_contested`, `allin_runout_single_survivor`, `cumulative_short_allins_reopen`, `multiway_root_projection`, `state_machine_pokerkit_fixtures` | 200/200 fixtures match; invariant holds at every step | coverage classifier, pot accounting |
| V21 | End-to-end max and p95 including cache misses, worker restarts and fault injection | `bench e2e` and `bench fault` on `fixtures/hands/e2e` (chart ranges) | p95 <= 15 s, zero final-delivery violations, every designated supported baseline fixture yields numeric EV | release |
| V22 | Baseline release gate independent of PokerData **(spec decision)** | section 13.5 baseline gate on the baseline suites and the chart E2E | All bounds met | release |

---

## 15. Risks and mitigations (top 6)

| Rank | Risk | Mitigation in this design |
|---|---|---|
| 1 | Coverage mismatch: a six-max label masks a HU backend; multiway, all-in-third-player, HU-after-multiway and unsupported histories may be a large share of live decisions | Pot-eligible counting; HU-after-multiway admitted only when the projection reproduces the current legal state (labelled `MultiwayStreetRoot`), otherwise `UnsupportedHistory`; numeric EV only for supported roots; decision log measures the coverage matrix from the first sessions; multiway shows per-pot equity plus the separated synthetic-root experimental block |
| 2 | Preflop supply: pack format conversion may fail; EV reference and size semantics unverified (A2/A3); no straddle/250bb trees; accuracy unpublished | V9 gate with a 2-day time box and named alternatives; EV suppressed until A2; sizes unloadable until A3; `source_accuracy = unverified` displayed; charts labelled `ChartRounded`; missing nodes `Unsupported`, never inferred |
| 3 | Flop time and memory with realistic ranges: R8 measured 6.2 s / 970 MB at 179x264 hands; store ranges are 2-3x more combos and unmeasured; variance 1.3-2.6x | V3 with store ranges before the budget is trusted; AVX2 required; `_min` template; absolute deadlines with watchdog and `DeadlineBestSoFar`; i16 mode when the f32 estimate exceeds 4 GiB; cache and pre-solver for the common scenarios |
| 4 | Plausible-looking wrong EV (matrix orientation, EV reference, unit scaling, translation, street-root money, hero-card leakage) | Analytic river oracle (fixed masses), two-combo matrix test, non-root payoff contract, source-unit test at 1/2 and 2/5, street-root fixture, public-range blocking tests, suit-permutation metamorphic test, feature freeze until section 13 is green |
| 5 | Upstream solver unmaintained with 100+ `unsafe` sites and a lock-free `MutexLike`; compiler lints will keep breaking the vendored fork | Pinned commit with a patch log; worker isolated in its own process with typed exit handling; panics caught at the executor boundary; job object memory limit on the worker process |
| 6 | Operational/licensing fit fails late (MSVC prerequisite for Tauri, venue rules, data rights) | MSVC workload install requested from the user in week 1 while core and worker work proceeds on the GNU toolchain; worker isolated with provenance kept; rights confirmed in writing before purchase; baseline release independent of the purchase (V22); no distribution or network service; venue permission as open question 1 |

---

## 16. Open questions

1. Venue permission and physical setup: whether the intended room permits a device consultation during a hand. Not a design decision; it determines whether phase 1 is used live or in permitted study/replay.
2. Template sizes: whether the initial contracts `flop_fast_v1`, `turn_std_v1`, `river_std_v1` (fixed in section 10.1) survive V2/V3 with chart and store ranges; a change is a version bump, never an edit in place.
3. PokerData pack conversion: whether the `.7z` MonkerSolver export can be parsed into the normalized envelope with EV within the 2-day time box; if not, whether the user funds one month of API Pro or accepts the chart baseline (which releases on its own gate, V22). Also A2 (which `ev_reference` variant) and A3 (absolute size per label) outcomes.
4. MSVC prerequisite: the Visual Studio 2022 "Desktop development with C++" workload is not yet installed (user action, pending). If the user refuses the install, the contingency is an Electron shell hosting the same Vite/React frontend; that is not a phase-1 contract, and adopting it would require a recorded architecture decision replacing section 3.1 and the Tauri command/`Channel` contracts before any shell work.
