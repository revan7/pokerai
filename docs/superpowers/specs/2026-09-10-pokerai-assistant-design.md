# PokerAI assistant: design specification (2026-09-10, revision 2)

Source of truth for decisions: `docs/design/2026-09-10-design-outline.md`. Facts: `docs/research/SYNTHESIS.md` as corrected by `docs/research/REVIEW-of-research.md`; preflop data facts from `docs/research/R7-pokerdata-verification.md`; solver facts measured on the target machine (i7-13700K, 64 GB, Windows 11) from `docs/research/R8-solver-bench.md`. Review findings applied from `docs/research/REVIEW-of-spec-1.md`; the disposition of every finding is in `docs/research/SPEC-CHANGELOG-1.md`. Items the outline leaves unspecified are chosen here and marked **(spec decision)**. A figure marked "measured" comes from R8; every other latency or memory figure is a hypothesis until `bench` measures it.

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
Screen capture; tournaments/ICM; solved EV for 3+ pot-eligible players; GPU solving; neural models; phone/LAN UI; distribution to anyone else; antes **(spec decision: not in the game config)**; button/Mississippi straddles and re-straddles **(spec decision: only one fully posted UTG straddle is modelled)**; bunching of folded players' cards **(spec decision: fixed off, section 10.3)**; decision-root solves with externally conditioned ranges (every solve is rooted at the street root, section 10.2); straddle and 250bb preflop trees (phase 2 per the outline); full flop-library completion (section 10.5).

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
- **Street root**: the state at the start of the current street, before its first action. **`StreetRootSnapshot`** (section 4.3): board, the two pot-eligible seats with their OOP/IP roles, the matched pot at the root, both remaining stacks at the root, incoming public ranges, and the ordered **history** of current-street actions. Worker pot and stacks come only from this snapshot; `Derived` describes the decision point and is used for legal-action validation and for the EV reference.
- **Pot-eligible player**: a player who has not folded, including all-in players. **HU spot**: exactly 2 pot-eligible players at the decision point. A third player who is all-in makes the spot non-HU.
- **Positions**: clockwise from the button: BTN, SB, BB, UTG, HJ, CO. Preflop order without straddle: UTG, HJ, CO, BTN, SB, BB. With the UTG straddle: HJ, CO, BTN, SB, BB, UTG (the straddler keeps the physical name UTG). Postflop order: SB, BB, UTG, HJ, CO, BTN, skipping folded and all-in seats. **OOP** = the pot-eligible player earliest in postflop order.
- **Effective stack**: min of the two HU players' remaining stacks at the street root; the deeper stack's excess is uncontestable and is excluded from the strategic pot model. **Depth** (preflop lookups): section 8.3, measured per node prefix in source-blind units.
- **Money units**: all `HandState`, worker and cache amounts are integer chips (`u32`), one chip = the session's accounting tick and legal wager quantum (`GameConfig.chip_label` names its value, "$1" or "$5") **(spec decision)**. EV and modelled rake are signed finite `f32` chips. Display: `ev_bb = ev_chips / bb_chips` rounded to 0.01 bb at render time only. Nothing is rounded to fit a stored node. Sums are checked (`pot + stacks < 2^31` for the library's signed inputs); overflow is `EngineError`.
- **Bet size (% pot)**: `s = (chips added beyond a call) / (pot after the bettor's call)`. An opening bet has call = 0. Raises are entered as **raise-to** amounts = the actor's total contribution on this street; the paid delta is `to - previous contribution`. Template raise sizes are multiples of the facing wager (`2.5x`) **(spec decision: matches the trees measured in R8)**.
- **EV convention**: `EV(a) = E[hero's final stack | a] - hero's stack at the decision point`, in chips. Fold = 0 exactly; chips already in the pot are sunk. The same reference holds for check. Rake is subtracted at the terminals that pay it.
- **Coverage labels** (about input/model matching, never a claim of full-game GTO):
  - `Exact`: every input matched the declared model exactly (node present, sizes in tree, depth equal, rake profile equal, requested accuracy reached, no translation, no bucketing). The source's own accuracy is reported in assumptions (`source_accuracy = unverified` for PokerData until R7 A6 supplies exploitability).
  - `Approximate{reasons}`: one or more of `BetTranslation`, `DepthBucket`, `AsymmetricStacks`, `RakeProfileMapped`, `StraddleMapped`, `ShortHandedMapped`, `DeadlineBestSoFar`, `ChartRounded`, `EvReferenceUnverified`, `UnconditionedPriorStreet`, `UnconditionedCurrentStreet`, `MultiwayStreetRoot`, `SprBucketed`, `MenuRounded`. Approximate does not promise a numeric EV (charts have none).
  - `Unsupported{reason}`: no numeric EV; equity where computable. Reasons: `MultiwayEv`, `MissingPreflopNode`, `HeroComboOutOfSupport`, `EngineError`, `TreeTooLarge`, `DeadlineExceeded`, `InvalidRanges`, `UnsupportedHistory`, `FormatUnsupported`.
- **Public range**: a seat's strategic range conditioned on public information only (its own observed actions, the board). Hero's actual cards are never applied to any public range, solve input or cache key. **Hero-conditioned copies** of opposing ranges (hero's cards removed) exist only for hero-combo equity and terminal call calculations. **`HeroComboOutOfSupport`**: hero's actual combo has zero weight in hero's public range at the decision node; equity and the range-level mix are shown, no per-combo strategy or EV is fabricated.
- **Canonical board**: the flop is canonicalized as an unordered set under the 24 suit permutations (1,755 classes: 13 trips orbit 4, 312 paired orbit 12, 286 distinct-rank patterns × 5 suit patterns: monotone orbit 4, three two-tone placements orbit 12 each, rainbow orbit 24; total 22,100). Turn and river are appended in dealt order and canonicalized within the flop's stabilizer. Among permutations giving the same canonical board, the one producing the lexicographically minimal serialized `(oop, ip)` public range tuple is chosen, then the lexicographically minimal permutation **(spec decision)**. A class absent from the cache is solved live; no "nearest flop" substitution, ever.
- **Tree template / effective tree**: section 10.1 / 10.2 and `EffectiveTree` in section 4.6.
- **Bet translation**: section 8.4 (likelihood interpolation and node translation, boundaries, prominence rule).
- **Range**: `Range1326`, a newtype over `[f32; 1326]`, weights in `[0, 1]`, indexed by combo index (section 4.1); on the wire a JSON array of exactly 1326 finite numbers, validated at every boundary.
- **Decision identity**: `(hand_id, hand_revision, decision_id, config_revision, model_revision)` (section 4.4). Every event, replay snapshot and result carries it; anything not matching the active identity is discarded.

---

## 3. Architecture

### 3.1 Processes
| Process | Technology | Role |
|---|---|---|
| `pokerai-ui` | Tauri 2 app: Rust shell crate `pokerai-app` + Vite/React/TypeScript frontend (Electron fallback, section 3.6) | Keyboard-first entry, display, undo; hosts the `engine` in-process |
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
| `core-model` | `parse_card`, `parse_hand`, `begin_hand(&HandConfig, BeginHand) -> Result<HandState>`, `apply_action(&HandState, Action) -> Result<HandState, RulesError>`, `set_board(&HandState, &[Card]) -> Result<HandState>`, `derive(&HandState) -> Derived`, `street_root(&HandState) -> Result<StreetRootSnapshot, RootError>`, `settle_pots(&HandState) -> Settlement` |
| `core-ranges` | `parse_range`, `range_to_string`, `expand_169(&[f32; 169]) -> Range1326`, `block_public(&mut Range1326, board)`, `hero_conditioned(&Range1326, hero: [Card; 2]) -> Range1326`, `mass`, `hash_scaled(&Range1326) -> [u8; 32]` |
| `core-iso` | `canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm)`, `orbit_size`, `apply(&SuitPerm, Card)`, `apply_range(&SuitPerm, &Range1326)`, `inverse` |
| `core-eval` | `rank7`, `equity(&EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult` (request: ranges or fixed hero combo, board, mode Exact/MonteCarlo{seed}, per-pot eligibility) |
| `core-preflop` | section 8.1 |
| `core-replay` | section 9.1 |
| `cache` | `Cache::open(dir, quota) -> Cache`, `lookup(&CacheQuery) -> Lookup` (Exact / Approximate / Provisional / Miss), `store(&CacheEntry)`, `Presolver::{start, pause, resume, status}` |
| `engine` | `Engine::new(GameConfig, Paths)`, `set_config(GameConfig) -> config_revision`, `begin_hand(..) -> HandState`, `apply_action`, `set_board`, `undo() -> HandState`, `recommend(sink) -> DecisionIdentity`, `cancel(decision_id)`, `finish_hand()`, `abandon_hand()`, `shutdown()` |
| `pokerai-app` (Tauri commands) | `set_game_config`, `begin_hand`, `set_hero_cards`, `apply_action`, `set_board`, `undo`, `recommend(on_event: Channel<RecommendationEvent>)`, `cancel`, `finish_hand`, `abandon_hand`, `set_seat_tag`, `presolver_status/pause/resume`. Undo is a command so the engine assigns the revision and invalidates work |
| `bench` | `bench run --suite {river_std,river_min,turn_std,turn_min,flop_fast,flop_min,e2e,fault} --threads N --reps R --out docs/bench/` |
| `solver-worker` | stdin/stdout JSON lines (section 4.5); exit code 0 on `shutdown` or stdin EOF, non-zero on panic |

TypeScript types for every `proto` type are generated at build time (`ts-rs` **(spec decision)**).

### 3.6 Environment prerequisites
- Rust stable 1.95 or newer. The active rustup toolchain on this machine is `stable-x86_64-pc-windows-gnu`; `stable-x86_64-pc-windows-msvc` is installed but Visual Studio 2022 has no C++ build tools (no `cl.exe`, `link.exe`).
- Tauri 2 officially requires the MSVC toolchain. Phase 1 therefore needs one of: (a) installing the Visual Studio 2022 "Desktop development with C++" workload (user decision, pending, open question 4), or (b) the Electron fallback for the shell (same Vite/React frontend, engine exposed through a small `napi-rs` or stdio bridge) **(spec decision on the fallback shape)**. The choice does not affect `engine`, `core-*`, `cache` or `solver-worker`.
- `solver-worker` and every non-Tauri crate build on either toolchain (R8 built the solver, its examples, `zstd-sys` and the bench FFI with the GNU toolchain).
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
    pub rake: Rake,                                  // PotRake{rate: f32, cap_chips: u32, no_flop_no_drop: bool} | TimeCharge (solved unraked)
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
pub struct StreetRootSnapshot {
    pub street: Street, pub board: Vec<Card>,
    pub oop: Seat, pub ip: Seat,
    pub pot_root: u32,                               // matched pot before this street's first action, including dead money from players folded on earlier streets
    pub stack_oop_root: u32, pub stack_ip_root: u32,
    pub dead_this_street: u32,                       // contributions this street by players who folded on this street (section 10.2)
    pub history: Vec<(Seat, Action)>,                // ordered current-street actions of oop/ip only
}
```
Lifecycle: `begin_hand` (button, hero, dealt seats, per-seat starting stacks confirmed by the user; the UI prefills each stack with the previous hand's start minus its committed chips and marks it unconfirmed until Enter **(spec decision)**) -> `Betting{Preflop}` -> `AwaitingBoard` when a street closes with two or more players not all-in -> `Betting{next}` after `set_board` -> `Complete` when one player remains, when all remaining are all-in (`AllInRunout`), or after river action; `finish_hand` / `abandon_hand` at any time. Completion and abandonment cancel outstanding work, invalidate every descendant identity and start the idle timer. `set_board` replaces the full validated board (3, 4 or 5 cards) and is legal only in `AwaitingBoard`; corrections go through `undo`. A street closes automatically when no further betting is possible.

Settlement rules (`core-model`): an uncalled bet or raise portion is returned to its owner before pots are built; then pots are layered by contribution levels with their eligible seats. Invariant checked after every action: `sum(stacks_remaining) + sum(pots) + returned = sum(stacks_start)`. Rake is modelled by the solver on matched terminal pots (`rate * pot` capped at `cap_chips`, once per hand); the room's rounding to whole chips is disclosed as `assumptions.notes`. `no_flop_no_drop` affects only preflop bundle selection (all solves are postflop). Cumulative reopening **(spec decision: TDA-style cash rule)**: a player who already acted may raise again only if the total raise since their last action is at least one full raise (`Derived.last_full_raise`); several short all-ins accumulate. Fewer than six dealt seats: supported for entry; preflop lookups use `ShortHandedMapped` (section 8.3).

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
    MultiwayStreetRoot { folded_this_street: u8 }, SprBucketed { actual: f32, used: f32 }, MenuRounded { max_delta_pct: f32 },
}
pub enum UnsupportedReason {
    MultiwayEv { pot_eligible: u8 }, MissingPreflopNode { key: String }, HeroComboOutOfSupport, EngineError { message: String, retryable: bool },
    TreeTooLarge { estimate_bytes: u64 }, DeadlineExceeded { stage: String }, InvalidRanges, UnsupportedHistory { reason: String }, FormatUnsupported { detail: String },
}
pub struct ActionAdvice { pub action: Action, pub frequency: Option<f32>, pub ev_bb: Option<f32>, pub availability: Option<String>, pub headline: bool }
pub struct EquitySummary { pub status: EquityStatus /* Ready | Pending | Unavailable{reason} */, pub hero_combo_vs_each: Vec<(Seat, f32)>,
    pub hero_range_vs_each: Vec<(Seat, f32)>, pub per_pot_shares: Vec<(u8 /* pot index */, Vec<(Seat, f32)>)>, pub method: EquityMethod /* Exact | MonteCarlo{samples, std_err} */ }
pub struct Assumptions { pub ranges_used: Vec<(Seat, String, f32 /* mass */)>, pub tree_signature: String, pub template_id: String,
    pub source: String, pub source_accuracy: String /* "unverified" | "exploitability <= x" */, pub source_granularity: String,
    pub target_bp: u16, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub cache: String /* miss | exact | approximate */,
    pub translations: Vec<ApproxReason>, pub mappings: Vec<ApproxReason>, pub notes: Vec<String> }
pub struct Recommendation {
    pub identity: DecisionIdentity, pub phase: Phase /* Fast | Provisional | Final */,
    pub coverage: Coverage, pub legal: Vec<LegalAction>,                 // legal intervals, always present
    pub actions: Vec<ActionAdvice>,                                       // evaluated menu (tree or store actions mapped to legal chips)
    pub equity: EquitySummary, pub assumptions: Assumptions,
    pub experimental: Option<ExperimentalHu>, pub exploit: Option<ExploitAdvice>,
}
pub enum RecommendationEvent { Fast(Recommendation), Equity { identity: DecisionIdentity, equity: EquitySummary },
    Progress { identity: DecisionIdentity, stage: String, iterations: u32, exploitability_pct: f32, elapsed_ms: u32 },
    Provisional(Recommendation), Final(Recommendation), NoDecision { identity: DecisionIdentity, reason: String } }
```
Headline: highest-EV action for hero's actual combo when EVs exist (ties by higher frequency, then earlier in menu order); with frequencies only (charts) the headline is labelled "highest-frequency chart action"; with neither there is no headline. Reasons accumulate across source, replay, cache and solve on every path; a later exact solve or cache hit never removes inherited reasons. Exploitability is compared with the target in raw chips before rounding to bp for display.

### 4.5 Worker protocol (`proto::worker`)
UTF-8 JSON Lines, one object per line, newline-terminated and flushed per message, `type` discriminator with fixed lowercase tags, ids as decimal strings. stdout carries protocol only; diagnostics go to stderr (drained by the engine, 64 KiB ring). Limits **(spec decision, initial)**: request line <= 1 MiB, result line <= 16 MiB, <= 100,000 exported nodes subject to the byte cap; over-limit input is rejected before work starts; an over-limit export returns the requested node only plus `export: "truncated"`.

Worker states: `Starting` (until `ready` is written) -> `Idle` -> `Building` (tree, memory check, allocation, locks) -> `Solving` (`solve_step` loop) -> `Extracting` (`finalize`, navigation, matrices, serialization) -> `Idle`; `Stopping` on `shutdown` or stdin EOF (the executor is asked to stop at the next iteration boundary; the process exits within 2 s or is killed by the engine). The `control` thread reads stdin in every state.

Messages:
```rust
pub enum Request {
    Solve { id, spot: String /* sha256 hex of the structural identity, for lock matching */, board: Vec<Card>, oop_range: Range1326, ip_range: Range1326,
            pot: u32, stack_oop: u32, stack_ip: u32, rake_rate: f32, rake_cap: u32, tree: EffectiveTree, history: Vec<Action>,
            target_bp: u16, deadline_ms: u32, extraction_margin_ms: u32, memory_limit_bytes: u64, background: bool },
    Lock { id, spot: String, locks: Vec<NodeLock> },
    Cancel { id, target: String },
    Shutdown { id },
}
pub enum Reply {
    Ready { proto_version: u16, solver_commit: String, adapter_version: u16, threads: u8, build_features: Vec<String>, cpu_features: Vec<String>, capabilities: Vec<String> },
    Ack { id, status: AckStatus /* accepted | staged | rejected{reason} | already_finished | unknown_target */ },
    Progress { id, stage: String, iterations: u32, exploitability_chips: f32, elapsed_ms: u32, memory_bytes: u64 },
    Result { id, status: SolveStatus /* ok | best_so_far | cancelled | error{code, message, retryable} */, elapsed_ms: u32,
             solution: Option<StreetSolution> },                     // present for ok and best_so_far only
}
pub struct StreetSolution { pub nodes: Vec<NodeStrategy>, pub requested: u32 /* index of the node reached by history */, pub exploitability_chips: f32,
    pub iterations: u32, pub memory_bytes: u64, pub mode: String /* f32 | i16 */, pub locks_applied: u16, pub export: String /* street | truncated */ }
pub struct NodeStrategy { pub path: Vec<Action>, pub actor: String /* oop | ip */, pub actions: Vec<Action>,
    pub probs: Vec<Vec<f32>> /* [1326][action], combo-major */, pub ev_chips: Vec<Vec<f32>> /* [1326][action], actor-owned, section-2 convention */, pub available: Vec<bool> /* [1326]: reach > 0 and not blocked */ }
pub struct NodeLock { pub path: Vec<Action>, pub actor: String, pub probs: Vec<Vec<f32>> /* [1326][action]; all-zero row = combo stays free */ }
```
Rules:
- `ack{accepted}` acknowledges receipt only. Every accepted `solve` produces exactly one terminal `result` (`ok`, `best_so_far`, `cancelled` or `error`); a completion racing a cancel yields either the valid result or `cancelled`, never both. Admission in the engine is released only by the terminal `result` or by confirmed process exit.
- `solve` while not `Idle`: `ack{rejected: busy}`, no work. Duplicate id of a live request: `ack{rejected: duplicate}`. `cancel` of the running job: `ack{accepted}` then `result{cancelled}` at the next iteration boundary; of a finished or unknown id: `ack{already_finished}` / `ack{unknown_target}`.
- Structurally invalid input (unknown tag, bad card, range length != 1326, non-finite number, history not representable in `tree`, node count over limit): `ack{rejected: reason}` for messages without work, `result{error{code: invalid_request, retryable: false}}` for an accepted `solve` that fails validation in `Building`.
- Locks belong to `spot`; `ack{staged}`; consumed atomically by the next `solve` with the same `spot`; a `solve` with a different `spot` while a lock is staged returns `result{error{code: lock_mismatch}}`; a `lock` received while not `Idle` is `ack{rejected: solve_in_progress}`; staged locks are cleared on consumption, cancellation, error and restart.
- `progress` is emitted at each stage transition and during `Solving` at iteration boundaries, coalesced to at most one per 100 ms. The writer never interleaves JSON with other output.
- `ready` is written once; the engine validates `proto_version` (2), `solver_commit`, `adapter_version`, `threads == requested`, `build_features` contains `avx2`. Startup timeout 5 s, then kill and retry once, then `EngineError`.
- Failure codes: `invalid_request`, `tree_too_large` (with `estimate_bytes`), `out_of_memory`, `lock_mismatch`, `internal` (panic caught at the executor boundary; `retryable: true`).

Wire examples (protocol conformance fixtures with full 1326-element vectors live in `fixtures/worker/*.jsonl`; the 1326-element vectors and matrices are elided only in this text and are referenced by fixture file):
```json
{"type":"ready","proto_version":2,"solver_commit":"9d1509fe5077d019825f833eed04b16d342dfda1","adapter_version":1,"threads":16,"build_features":["avx2"],"cpu_features":["avx2","fma"],"capabilities":["solve","lock","cancel","street_export","i16"]}
{"type":"solve","id":"41","spot":"3f9c0a7d2b1e4c6f8a9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f","board":["Qs","Jd","7h","3c","2d"],"oop_range":"<1326 numbers: fixtures/worker/river_two_combo.jsonl>","ip_range":"<1326 numbers: same file>","pot":100,"stack_oop":100,"stack_ip":100,"rake_rate":0.0,"rake_cap":0,"tree":{"rules_version":1,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":1.5,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[]},"history":[{"kind":"check"}],"target_bp":10,"deadline_ms":1500,"extraction_margin_ms":200,"memory_limit_bytes":4294967296,"background":false}
{"type":"ack","id":"41","status":"accepted"}
{"type":"progress","id":"41","stage":"building","iterations":0,"exploitability_chips":0.0,"elapsed_ms":3,"memory_bytes":331776}
{"type":"progress","id":"41","stage":"solving","iterations":50,"exploitability_chips":0.27,"elapsed_ms":8,"memory_bytes":6914048}
{"type":"progress","id":"41","stage":"extracting","iterations":50,"exploitability_chips":0.09,"elapsed_ms":9,"memory_bytes":6914048}
{"type":"result","id":"41","status":"ok","elapsed_ms":12,"solution":{"nodes":[{"path":[{"kind":"check"}],"actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"probs":"<[1326][2]: fixture>","ev_chips":"<[1326][2]: fixture>","available":"<[1326]: fixture>"},{"path":[{"kind":"check"},{"kind":"allin","to":100}],"actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"probs":"<[1326][2]: fixture>","ev_chips":"<[1326][2]: fixture>","available":"<[1326]: fixture>"}],"requested":0,"exploitability_chips":0.09,"iterations":50,"memory_bytes":6914048,"mode":"f32","locks_applied":0,"export":"street"}}
{"type":"cancel","id":"42","target":"41"}
{"type":"ack","id":"42","status":"already_finished"}
{"type":"solve","id":"43","spot":"<same shape as id 41; fixtures/worker/flop_cancel.jsonl>"}
{"type":"ack","id":"43","status":"accepted"}
{"type":"cancel","id":"44","target":"43"}
{"type":"ack","id":"44","status":"accepted"}
{"type":"result","id":"43","status":"cancelled","elapsed_ms":183}
{"type":"result","id":"45","status":"best_so_far","elapsed_ms":9870,"solution":{"nodes":"<fixtures/worker/flop_best_so_far.jsonl>","requested":2,"exploitability_chips":1.9,"iterations":70,"memory_bytes":1017118720,"mode":"f32","locks_applied":0,"export":"street"}}
{"type":"result","id":"46","status":"error","elapsed_ms":12,"error":{"code":"out_of_memory","message":"allocation failed","retryable":true}}
{"type":"lock","id":"47","spot":"3f9c0a7d2b1e4c6f8a9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f","locks":[{"path":[{"kind":"check"},{"kind":"allin","to":100}],"actor":"oop","probs":"<[1326][2]: fixtures/worker/lock_river.jsonl>"}]}
{"type":"ack","id":"47","status":"staged"}
{"type":"shutdown","id":"48"}
{"type":"ack","id":"48","status":"accepted"}
```
`proto_version` mismatch at `ready` is an `EngineError` for every request until the worker is rebuilt.

### 4.6 Effective tree (`proto::EffectiveTree`, materialized, hashed)
```rust
pub struct EffectiveTree {
    pub rules_version: u16,                          // 1; bumps when any rule below changes
    pub template_id: String, pub root_street: Street,
    pub menus: BTreeMap<Street, PlayerMenus>,        // per street: oop/ip bet sizes (pot fractions), raise multipliers (x facing wager), donk sizes (empty in phase 1)
    pub add_allin_threshold: f32,                    // upstream semantics at the pinned commit: all-in is added when the largest listed bet would exceed threshold*pot
    pub force_allin_threshold: f32,                  // upstream semantics: a wager whose post-call SPR would be below the threshold becomes all-in
    pub merging_threshold: f32,                      // 0.0 in phase 1 (spec decision); exact inserted sizes are never merged
    pub wager_cap: u8,                               // bets plus raises per street; beyond it only fold/call/all-in remain (enforced with remove_lines when the library has no native cap; V4 confirms)
    pub inserted: Vec<(Vec<Action> /* path */, String /* actor */, Action)>,   // observed sizes not on the menu, exact chips
}
```
Deterministic action order at every node: Fold, Check, Call, bets/raises ascending by `to`, AllIn **(spec decision)**. Future-menu sizes are converted to chips with round-half-up to the wager quantum, clamped to the legal interval, and duplicates (a size rounding onto another size, a call, or the all-in) are merged. Observed sizes are inserted exactly and are never merged, forced to all-in or capped away; an observed history that the rules cannot represent (a wager that would violate the library's minimum-raise rule) returns `Unsupported{UnsupportedHistory}` instead of a changed history. The structural signature `tree_signature = sha256(rules_version, template_id, root_street, menus, thresholds, wager_cap, inserted sizes as reduced pot-fraction rationals at their paths)` contains no raw chips.

---

## 5. Data flow for one decision

1. **Session start.** User sets `GameConfig`; the engine assigns `config_revision`, saves to `%APPDATA%\PokerAI\config.json`, spawns the worker, validates `ready`, opens the cache and validates the preflop bundles (section 8.2).
2. **Hand begin.** `begin_hand{button, hero, hero_cards, dealt, stacks}` with confirmed stacks; `HandConfig` frozen; `hand_id` and `hand_revision` assigned.
3. **Entry.** One keystroke per event (section 5.1); every mutation goes through `apply_action` / `set_board` / `undo` in the engine, which assigns a fresh `hand_revision`, cancels in-flight work and invalidates every descendant identity, even when no decision follows.
4. **Request.** At a decision point the UI calls `recommend`. `engine-main` stamps `t0` (monotonic), allocates `decision_id`, forms the identity tuple (a repeated request for the same state gets a new `decision_id`), re-derives the decision-point conditions of section 2 itself (answering `NoDecision` when they fail, independent of the UI), cancels any other live job and queues the request (depth 1, newest wins).
5. **Fast phase (target <= 0.3 s).** Validate (`Derived` recomputed and compared); classify coverage (section 6); replay public ranges (section 9); emit `Fast` with legal intervals, coverage so far, `frequency = None`, `ev_bb = None`, equity `Pending`. Equity runs with its own budget (section 7) and is delivered as `Equity` when ready.
6. **Preflop decision.** Store lookup (section 8): present node -> `Final` with frequencies and EV (PokerData with verified reference) or frequencies only (charts, or PokerData with `EvReferenceUnverified`); absent node -> `Final` `Unsupported{MissingPreflopNode}`.
7. **Postflop, HU.** Derive `StreetRootSnapshot` (`UnsupportedHistory` if it cannot be derived); build the effective tree (section 10.2); cache lookup (flop/turn): exact hit -> `Final`; approximate hit -> `Final` with `SprBucketed`; provisional hit (below target) -> `Provisional`, then live solve; miss -> `solve` with the remaining street budget; forward `Progress`; on `result` validate the requested node and actor against `Derived.to_act`, assemble `Final` (`Exact` if `exploitability <= target` and no inherited reasons, else `Approximate`), store in cache (flop/turn). Facing an all-in: same path (the jam is an inserted size; hero's menu is fold/call); if the worker is unavailable, the analytic fallback of section 6 applies.
8. **Postflop, 3+ pot-eligible.** `Final` `Unsupported{MultiwayEv}` with per-pot equity shares and `experimental` (section 6) when it completes within the street budget.
9. **Display.** Headline per section 4.4; frequency mix, coverage label and assumptions always visible. The UI accepts only events whose identity equals the active identity; `Fast`, `Equity`, `Progress`, `Provisional` and `Final` are all subject to the check.
10. **Log.** Every request and its `Final` (identity, coverage, reasons, elapsed, cache result, deadline violations, versioned input record) appended to `%LOCALAPPDATA%\PokerAI\decisions.jsonl` (rotated at 50 MiB, 10 files) **(spec decision)**.
11. **Hand end.** `finish_hand` / `abandon_hand`; the idle timer for the pre-solver starts.

Per-street behaviour of step 7: flop solves use the flop template from the flop root with ranges from preflop replay; turn solves are rooted at the turn (ranges = flop-end public ranges from replay); river solves are rooted at the river and never cached. Prior-street solutions are used only through replay (section 9), never as the current street's strategy.

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
| Postflop, HU, street root HU | Yes | `Exact` or `Approximate` with `BetTranslation` (prior street), `DeadlineBestSoFar`, `UnconditionedPriorStreet`, `SprBucketed`, `MenuRounded` (cache hits), plus inherited preflop reasons | Frequencies, EV, equity, progress |
| Postflop, HU now, street root 3+ players | Yes | `Approximate{MultiwayStreetRoot}` plus the reasons of the row above | As above, folded seats' actions dropped, dead money added to the root pot (section 10.2) |
| Postflop, HU, opponent all-in, worker unavailable | Yes | `Approximate{UnconditionedCurrentStreet}` plus inherited reasons | Analytic `EV(call)`, `EV(fold) = 0`, equity |
| Postflop, hero combo has zero weight at the node | No | `Unsupported{HeroComboOutOfSupport}` | Equity, range-level mix |
| Postflop, 3+ pot-eligible (any all-in counts) | No | `Unsupported{MultiwayEv}` | Per-pot equity shares; `experimental` block |
| NoDecision (section 2) | n/a | no request | UI shows "no decision" |
| Worker failure after retry / deadline expired | No | `Unsupported{EngineError}` / `Unsupported{DeadlineExceeded}` | Equity, visible reason |

Rules:
- Pot-eligible counting uses `Derived.folded` only; all-in players count. Folded players' ranges are retained (`folded_ranges`) for provenance; bunching is fixed off in phase 1.
- Reasons accumulate: a turn solve after a translated flop bet and a depth-bucketed preflop lookup carries both. An `Unsupported` result keeps the accumulated reasons in `partial`.
- Facing an all-in, analytic fallback **(spec decision)**: let `C` be the payable call and `W` the final matched pot after returning excess and adding `C`; `EV(call) = equity_actual_combo * (W - R) - C` with `R = min(rate * W, cap)`, ties counted as shares, using hero's actual combo against the opponent's public range conditioned on prior streets and hero-conditioned for equity; the current street's actions are unconditioned (`UnconditionedCurrentStreet`). Frequencies: call 100% when `EV(call) > 0`, fold 100% when `< 0`, call when `= 0` **(spec decision: tie rule)**. The primary path (worker solve with the jam inserted) conditions the jam through the solve.
- `experimental` (multiway only): a HU solve of hero versus the opponent whose public range has the highest range-vs-range equity against hero's public range (same equity routine, pairwise-compatible) **(spec decision)**; pot = current total pot, stack = min(hero, that opponent); skipped when that opponent is all-in or the equity phase overran; same template and remaining budget; rendered in a separate block "experimental, not solved", never in `actions`, never `Exact`.
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

Progressive display: `Fast` at <= 0.3 s; `Equity` when ready; `Progress` at most every 250 ms in the UI (iterations, exploitability % of pot, stage, elapsed); `Provisional` for a below-target cache hit; `Final` once. `target_bp` default 50 (0.5% of pot) **(spec decision; measured runs used the same target)**.

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
pub struct PreflopNode { pub actor: Position, pub actions: Vec<PreflopStep>, pub probs: Vec<Vec<f32>> /* [169][action] */, pub ev_source_sb: Option<Vec<Vec<Option<f32>>>> /* [169][action], None = unavailable */ }
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

The manifest carries `bundle_id`, `source`, `depths`, `rake_profile` ("5% cap 0.5bb"), `source_blinds`, `ev_unit = source_sb`, `ev_reference` (`fold_zero_verified` | `absolute_stack_verified` | `unverified`, per A2), `accuracy` (`unverified` unless A6 supplies exploitability), `straddle = false`, `license_note`, `sha256` of contents. Fixtures `fixtures/preflop/pokerdata_sample/{node,range,spots}.json` pin the acquired examples with exact field names and action tokens, a sparse zero-weight EV and the SB-limp/BB-check node.
- `ChartTranscription`: `fixtures/charts/<name>.json` from `tools/chart_ingest.py` (169-class frequencies, no EV, `ChartRounded`). Depths 100 (PokerCoaching) and 200 (RangeConverter).
- Validation at load: schema and content (169 keys in the declared order, finite values, weights in `[0, 1]`, sibling weights per class summing to at most `1 + 1e-3`, valid action tokens and sizes, decompression and file size bounds 64 MiB) after the manifest hash; a failing bundle is quarantined (renamed `.bad`) with a startup banner; remaining bundles stay active. A matching hash never substitutes for content checks.
- Source precedence: PokerData over charts; within a source the nearest depth **(spec decision)**.

### 8.3 Lookup rules (per prefix, hindsight-free)
- **Node reconstruction**: every historical node is rebuilt from its own prefix only: actor first, eligible seats = seats not folded before that action. Mappings are cached per prefix within a replay run and never recomputed with later folds.
- **Depth**: `depth = min(actor.stack_start, max over other eligible seats' stack_start) / unit` with `unit = bb_chips` (ordinary) or `straddle_chips` (straddle-mapped). Nearest acquired depth, ties deeper; above 200 clamps to 200. `DepthBucket{seat, actual, used}` whenever `actual != used` (the outline's "labelled when not exact"), `prominent` when `abs(actual - used) / used > 0.05`. `AsymmetricStacks` when any eligible seat's start stack differs from the chosen depth by more than 5% **(spec decision)**.
- **Rake**: tuple `(rate, cap_bb, no_flop_no_drop)`; exact match preferred; otherwise order by `abs(cap_bb - actual_cap_bb)`, then `abs(rate - actual)`, then collection rule, ties to the lower cap; `TimeCharge` prefers an unraked bundle, else the smallest cap. Any difference is `RakeProfileMapped`.
- **Straddle** (phase 1 requires one fully posted UTG straddle with `S >= 2 * bb` and six dealt seats; anything else is `FormatUnsupported`): physical order per section 2. Lookup-only virtual roles: HJ->UTG, CO->HJ, BTN->CO, SB->BTN, BB->SB, straddler->BB; cards, posts and OOP/IP are never rotated. Source bb = `S` for depth, sizes, cap and EV conversion. Reported normalized posts `(sb/S, bb/S, 1)`: 1/2/4 gives `(0.25, 0.5, 1)`, 2/5/10 gives `(0.2, 0.5, 1)`; the physical SB's post is not represented in the virtual tree (the physical BB's post matches the virtual SB post exactly when `bb/S = 0.5`). Label `StraddleMapped{posts}`.
- **Short-handed** (fewer than six dealt seats, no straddle): the vacant seats are mapped as the earliest positions folded, for lookup only; no folded range and no bunching contribution are attributed to them; `ShortHandedMapped{dealt}` **(spec decision)**.
- **Sizes**: an observed raise-to is converted to source bb (`to / unit`) and matched against the node's resolved sizes; exact within `0.5 * chip / unit` is used directly; otherwise section 8.4.
- **Unsupported**: no node for the history (per R7 §3: open-limps from UTG, HJ, CO and BTN are absent, cold-call versus 3-bet is absent for CO and SB, more raises than the tree; SB limp and BB-versus-limp exist) or no bundle: `Unsupported{MissingPreflopNode{key}}`; never a guessed fold.
- **EV normalization** (`PokerDataJson`, gated on `ev_reference`): `ev_inc_sb(a) = ev_sb(a) - ev_sb(fold)` at fold-legal nodes when the reference is `absolute_stack_verified`; at nodes without fold (BB facing a limp) `ev_inc_sb(a) = ev_sb(a) + committed_by_actor_sb` (the actor's posted chips in source SB) **(spec decision: commitment offset)**; no subtraction when `fold_zero_verified`. Then `ev_source_bb = ev_inc_sb * 0.5` (source SB / source BB), `ev_chips = ev_source_bb * unit` (`bb_chips` or `straddle_chips`), display `ev_chips / bb_chips`. Live blinds never enter the conversion (1.84 source SB is 0.92 source bb at 1/2 and at 2/5). While `ev_reference = unverified`, EV is suppressed and `EvReferenceUnverified` is added.
- 169 -> 1326 expansion gives every combo of a class the class values; public blocking by the board is applied by `core-replay`.

### 8.4 Bet translation
- **Likelihood interpolation** (replay, one observed action at one parent): with the source menu sizes at that node as pot fractions and observed `s`, adjacent `A <= s <= B`: `f_A = (B - s)(1 + A) / ((B - A)(1 + s))`, `f_B = 1 - f_A`, `P(obs | combo) = f_A P(A | combo) + f_B P(B | combo)`. Boundaries: `A == B` or a single size: `f = 1` (labelled clamp); below the smallest: clamp to it; above the largest non-all-in: interpolate with the all-in fraction when all-in is on the menu, else clamp. Deviation `d = min(|s - A|, |s - B|)` over the sizes used. The mixture opens branches `(path_A, f_A)`, `(path_B, f_B)`; later actions on the same street are evaluated in every open branch and combined as `P(seq) = sum_b f_b * prod_k P(a_k | branch b)`; at most 4 branches are kept (heaviest) **(spec decision)**. A branch whose continuation is absent follows section 9.3 for that branch (no silent renormalization).
- **Node translation** (current preflop decision at a translated node): the answer is the weighted mixture of the branch nodes' strategies over actions with identical kind and identical chip amount after mapping; an action present in one branch keeps only that branch's weight; EV is mixed only when both branches have EV under the same reference, otherwise EV is suppressed with `EvReferenceUnverified`.
- Every non-exact mapping is recorded in `assumptions.translations` with its deviation; the `BetTranslation` reason enters `Coverage` with `prominent = d > 0.10` **(spec decision: the outline's threshold controls prominence)**.
- **Legality after mapping**: source sizes are converted to chips (round-half-up to the quantum); an action illegal at the actual node (below min-raise, above the stack) has its probability moved to the nearest legal action in size order and the move is recorded in `assumptions.notes`; identical rounded actions merge. A missing action is never a guessed fold.
- Never used for the current postflop street (observed sizes are inserted exactly, section 10.2).

---

## 9. Range replay (`core-replay`)

### 9.1 Interface
```rust
pub struct SnapshotKey { pub hand_id: u64, pub config_revision: u32, pub model_revision: u32, pub street: Street, pub root_board: Vec<Card>, pub root_range_hashes: [[u8; 32]; 2], pub tree_signature: String }
pub struct StreetSnapshot { pub key: SnapshotKey, pub hand_revision_at_solve: u32, pub nodes: Vec<NodeStrategy>, pub exploitability_chips: f32, pub reasons: Vec<ApproxReason> }
pub struct ReplayInput<'a> { pub cfg: &'a HandConfig, pub state: &'a HandState, pub store: &'a PreflopStore, pub snapshots: &'a [StreetSnapshot] }
pub struct ReplayOutput { pub ranges: Vec<Option<Range1326>> /* per seat, public */, pub folded_ranges: Vec<Range1326>, pub log_reach: Vec<f64>, pub reasons: Vec<ApproxReason>, pub unsupported: Option<UnsupportedReason> }
pub fn replay(input: ReplayInput) -> ReplayOutput;
```

### 9.2 Rules
- Start: every dealt seat has the uniform range (weight 1 on all 1326 combos). Hero's actual cards are never applied.
- Preflop: for each observed action in order, `weight[combo] *= P(action | combo, node)` from the store node reconstructed for that prefix (section 8.3), with branch-weighted interpolation for off-menu sizes. Each action is applied exactly once from its prefix-specific node; a node's reach is never multiplied in again. Folds condition the folder's range (`folded_ranges`).
- Postflop: for each observed action on a completed street, `weight *= P(action | combo)` from the compatible `StreetSnapshot`'s node at that path (exact path match; an inserted observed size is a tree action, so its probability is the solved one, never forced to 1). Off-menu sizes in a snapshot tree do not occur (they were inserted when the street was solved); an action absent from the snapshot tree follows 9.3.
- Snapshot compatibility: same `hand_id`, `config_revision`, `model_revision`; the snapshot's `root_board` equals the street's board; its `root_range_hashes` equal the hashes of the public ranges this replay computes at that root; its tree contains every observed action of the street. Among compatible snapshots the lowest exploitability wins. Snapshots are recorded only from `ok` / `best_so_far` results whose identity was active when the result arrived; a mutation invalidates every snapshot of later streets and of the same street at a longer prefix.
- Public blocking by the board is applied at each street root; `log_reach` keeps the log of each seat's total mass for guards; the solver receives weights as is (no renormalization). Zero-mass guard: if a seat's mass drops below `1e-9` of its starting mass, keep the pre-action range and add `UnconditionedPriorStreet{cause: "zero mass"}` **(spec decision)**; low but positive mass is valid.
- Hero's own actions condition hero's public range the same way. Hero-conditioned copies are built after replay for equity only.

### 9.3 Missing strategies
- Missing preflop node during replay for a later decision: the seat's range stays at its pre-action value, traversal of that seat's later preflop actions stops until a prefix with a present node resumes, reason `UnconditionedPriorStreet{Preflop, seat, cause: "missing node <key>"}`; the decision stays `Approximate` (the postflop solve is exact given the declared ranges). The current decision's own lookup failing is `Unsupported{MissingPreflopNode}` (sections 6 and 12); the two cases are distinct.
- A completed street with no compatible snapshot (multiway at the time, engine error, deadline, or no request was made): unconditioned, `UnconditionedPriorStreet{street, seat, cause}`.
- Provenance and accuracy reasons stored with the snapshot are carried into the current result.

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

Thresholds use upstream `TreeConfig` semantics at the pinned commit (`add_allin_threshold`, `force_allin_threshold`, `merging_threshold = 0.0`); donk menus are empty. The `_min` templates are the crash/timeout retry and the `TreeTooLarge` fallback. Sizes are hypotheses **(spec decision)**; V3 re-measures the shipped templates with store ranges at 100bb and 200bb. A change increments the version suffix; old cache entries stay valid under their own signature.

### 10.2 Effective tree and street-root solve
- Root inputs come from `StreetRootSnapshot` only: `pot = pot_root + dead_this_street`, `stack_oop`, `stack_ip` (effective stack = min; the excess is uncontestable). `core-model` asserts that replaying `history` from the snapshot reproduces the decision point's pot, contributions, remaining stacks and actor; a mismatch is `EngineError` (never a silently adjusted root).
- Menus are converted to chips at each node (section 4.6). An already-taken action whose chips equal a menu size after rounding is in-tree. An off-menu taken action is inserted exactly at its node **alongside** the menu sizes; no sibling line is ever pruned. The solve starts from the incoming public ranges at the street root; the worker applies `history` to reach the requested node and exports every decision node of the current street. Complexity is bounded only by the future menus (templates), never by removing observed-prefix alternatives.
- Street root with 3+ pot-eligible players that is HU now **(spec decision)**: players who folded on this street are removed, their actions are dropped from `history`, their current-street contributions are added to the root pot as `dead_this_street`, and the result carries `MultiwayStreetRoot{folded_this_street}`.
- OOP/IP per section 2. `hero` is not a solve input; the payload is actor-owned, and the engine reads hero's node by path and actor.

### 10.3 Worker adapter (library mapping)
- `solve` -> `CardConfig{range: [oop, ip], flop, turn, river}` + `TreeConfig{initial_state, starting_pot, effective_stack, rake_rate, rake_cap, bet sizes, add_allin_threshold, force_allin_threshold, merging_threshold}`; `remove_lines` only for the wager cap; `memory_usage()` before allocation: f32 when the f32 estimate `<= 4 GiB`, else 16-bit when the compressed estimate `<= 8 GiB`, else `error{tree_too_large}` **(spec decision)**; the worker also refuses when `estimate * 1.25 > memory_limit_bytes` (headroom for tree, scratch, extraction and serialization). Staged locks applied with `lock_current_strategy` at each locked node before the first iteration; `solve_step` loop with deadline and cancel checks between iterations; `finalize`; `apply_history`; for every decision node of the current street: navigate, `cache_normalized_weights()`, `strategy()`, `expected_values_detail(current_player())`, `available` from reach and blocking; `back_to_root` between nodes.
- Matrix contract: the library returns action-major arrays over its compact private-hand list for the player to act; the adapter converts library index -> named combo -> `ComboIndex` and transposes to combo-major `[1326][action]`, zero-filled and masked for combos outside the compact list. EV is actor-owned at each node (requesting the other player's EV is a different shape and is not used). `normalize_ev` converts the library's values to the section-2 convention; its exact formula is fixed by inspecting `src/game/interpreter.rs` at the pinned commit during V4 (measured: values are absolute chip EV, folds report 0) and pinned by contract tests with positive, negative, check and fold payoffs at non-root nodes; no speculative pot or commitment offset.
- Rake: `rake_rate` and `rake_cap` both `> 0` for `is_raked()`; `TimeCharge` sends 0/0.
- Memory: the process is warm (allocations of the previous game are dropped after the terminal `result`; no arena reuse is guaranteed). The engine places the worker in a Windows job object with `JOB_OBJECT_LIMIT_PROCESS_MEMORY = 16 GiB` **(spec decision)** so a runaway allocation kills the worker, not the machine. An abnormal exit (OOM abort cannot be caught) is a typed `WorkerExit{code}` failure in the engine.
- `bunching`: no protocol field in phase 1; capability reserved (`capabilities` list) because R8 measured x27-x40 solve time with bunching on turn/river and did not attempt the flop.

### 10.4 Cache (`cache`): pot-normalized street solutions
Policy, not a per-hand EV-error guarantee; nothing here changes `HandState` chips or the board used for a live solve.
- **Reference state**: `P = pot` of the effective tree (section 10.2), `SPR = min(stack_oop, stack_ip) / P`. Every monetary quantity is expressed relative to `P`.
- **Key** = sha256 of the canonical serialization of: `schema_version` (2); `solver_commit`; `adapter_version`; `rules_version`; canonical board `(unordered flop, ordered turn)` (river solutions are not cached); `root_street`; `spr_bucket` = `round(ln(SPR) / ln(1.02))` (geometric grid, step 2%) **(spec decision)**; `tree_signature` (section 4.6: menus as pot fractions, inserted sizes as reduced pot-fraction rationals at their paths); rake `(rate as f32 bits, cap/P as a reduced rational, collection rule version)`; `range_hash_oop`, `range_hash_ip` = `hash_scaled` of each public range after suit canonicalization and board blocking, scaled so its maximum weight is exactly 1.0 and hashed bit-exactly (independent positive rescaling is irrelevant to the strategy); `model` = `baseline` or `locked{fingerprint}`. Absent from the key: seat ids, hand id, `bb_chips`, raw chip pot/stacks, the wager quantum (its effect is the chip rounding of menu sizes, handled at lookup), hero, `target_bp`, requested path. All action amounts in the hash are normalized; a raw-chip signature would undo the design.
- **Payload** (`CacheEntry`): key fields, exact source inputs (`pot`, both stacks, `SPR`, `bb_chips`, `quantum/P`, canonical ranges, materialized tree), every exported `NodeStrategy` with `ev_over_P` instead of chips, root ranges, `exploitability_over_P`, `reached_bp`, `target_bp`, iterations, elapsed, mode, inherited `reasons`, `created`, `last_hit`. Never a user `Recommendation` or request id.
- **Lookup**: compute the query key for buckets `b - 1`, `b`, `b + 1`; candidates must match every non-SPR field exactly (a different canonical board, range hash, tree signature, rake or version is a miss, never a substitute). Choose the candidate with the smallest `abs(SPR_query - SPR_entry) / SPR_entry`: `<= 0.01` -> `Exact`; `<= 0.02` -> `Approximate{SprBucketed{actual, used}}`; otherwise miss **(spec decision on the 2% approximate bound)**. Then select the requested node by ordered path and actor; missing node or actor mismatch is a miss. Inverse-map suits, convert action amounts to legal query chips (round-half-up to the quantum; merge duplicates; the inserted observed sizes are exact by construction), `ev_chips = ev_over_P * P_query`, validate finiteness and the menu against `Derived.legal`. The realized menu sizes are disclosed in `assumptions.notes`; when any rounded size deviates from the solved pot fraction by more than 1% of pot (`0.5 * quantum / P > 0.01`, pots under 50 quanta) the hit carries `MenuRounded{max_delta_pct}` **(spec decision)**; a hit that crosses a min-raise, all-in or rake-cap boundary after rounding is a miss. Coverage and assumptions are rebuilt from the query's provenance plus the entry's inherited reasons; an entry can never upgrade a source reason.
- **Accuracy filter**: `reached_bp <= target_bp` serves as `Exact`/`Approximate` per the rule above; a validated entry above the target is served as `Provisional` while a live solve refines within the deadline; `DeadlineBestSoFar` entries are stored with their `reached_bp` and never certify the current request's timing.
- **Replacement**: at most two entries per `(key)` cell: closest SPR and best accuracy; a dominated entry (both farther and less accurate) is replaced deterministically. Version mismatch is a miss.
- **Scale check** (contract): pot/stack/cap 100/500/5 and 200/1000/10 with identical ranges, proportional quantum and identical fractional tree hit the same entry with identical frequencies and doubled chip EV; 100/500 versus 103/515 with a one-chip quantum has SPR 5.0 in both and a rounded 0.5-pot bet of 50 versus 52 chips (0.505, deviation 0.5% of pot): exact hit with the realized size disclosed; 100/500 versus 100/508 (SPR 5.08) is `SprBucketed`; a 20-chip pot rounding a 0.33-pot bet from 6.6 to 7 chips (0.35, deviation 2%) is `MenuRounded`; a different canonical board always misses.
- **Storage**: `%LOCALAPPDATA%\PokerAI\cache\v2\<key[0..2]>\<key>.bin`, `bincode` + `zstd`, header with payload sha256, schema and `proto_version`; bounded sizes (64 MiB compressed, 256 MiB decoded); any read/decode error is a miss and the file is deleted (deletion failure is non-fatal); writes go through a temporary sibling and atomic rename on the `cache-writer` thread after validation; a write failure preserves the recommendation. Quota `cache_quota_bytes` default 10 GiB; eviction removes the oldest `last_hit` first **(spec decision)**.

### 10.5 Pre-solver (`cache::Presolver`)
- Runs when no hand is in progress (`Complete`/`Abandoned`, or none begun) and no request has arrived for 30 s; a live request cancels the running job at its next iteration boundary (lost work accepted); `presolver_pause()` stops scheduling.
- Jobs are `solve{background: true, deadline_ms: 600000}` with `flop_fast_v1`, `target_bp` 50; retries 3 with 30 s backoff; task status (`pending`, `done`, `failed{n}`) is persisted in `queue.json` keyed by the normalized game identity (cache key without `spr_bucket` plus the exact scenario SPR); a task is `done` only when a valid entry at target exists.
- Scenarios **(spec decision, explicit list)**, ranges from the store's replay of the line, chips from the last-used `HandConfig`: tier 1 (100bb): SRP BTN-open/BB-call, CO-open/BB-call, HJ-open/BB-call, UTG-open/BB-call; tier 2 (100bb): SRP SB-open/BB-call, BTN-open/SB-call, CO-open/BTN-call, HJ-open/BTN-call; 3-bet pots BTN-open/BB-3bet/BTN-call, CO-open/BTN-3bet/CO-call, BTN-open/SB-3bet/BTN-call, HJ-open/BTN-3bet/HJ-call; tier 3: the same twelve at 200bb. Within a tier the outer loop runs over canonical flops (orbit 24 before 12 before 4, then rank order) and the inner loop over the tier's scenarios, so scenarios advance together.
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
| Stack/pot inconsistency, board duplicate, unsupported format (re-straddle, short straddle post, straddle with fewer than six dealt seats) | Rejected with a message (`FormatUnsupported` for formats); state unchanged |
| History not representable in the tree | `Unsupported{UnsupportedHistory{reason}}`; equity shown |
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

### 13.1 Unit (Rust, `cargo test`; exhaustive suites behind `--features exhaustive`, run by `bench oracle` before release)
| Crate | Test | Assertion |
|---|---|---|
| core-model | `state_machine_pokerkit_fixtures` | Replays `fixtures/hands/*.json` (200 hands generated by `tools/gen_fixtures.py` with PokerKit: UTG straddle at 1/2/4 and 2/5/10, side pots, 2 and 3 all-ins, short all-ins, uncalled returns) and matches pot, settled pots, stacks, returns and legal actions at every step |
| core-model | `straddle_action_order_utg` | Preflop HJ, CO, BTN, SB, BB, UTG; postflop SB, BB, UTG, HJ, CO, BTN; posts `(0.25, 0.5, 1)` and `(0.2, 0.5, 1)` reported |
| core-model | `min_raise_and_short_allin_no_reopen` | A single short all-in does not reopen action for a player who already acted |
| core-model | `cumulative_short_allins_reopen` | bb 2: raise to 10, all-in 14, all-in 17: cumulative 7 < 8 does not reopen; raise to 10, all-in 15, all-in 19: cumulative 9 >= 8 reopens |
| core-model | `side_pot_three_allins` (V14) | Contributions 50/100/200 by exactly three players: main 150 (3 eligible), side 100 (2 eligible), 100 returned to the 200 stack; conservation holds |
| core-model | `side_pot_two_contested` | A fourth player matching 200 yields main 200, side 200, side 200 with correct eligibility |
| core-model | `street_root_reconstruction` (T2) | From root pot 100 / stacks 500: OOP bet to 50 -> pot 150, 450/500; IP raise to 150 -> pot 300, 450/350, OOP call cost 100; OOP call -> 400, 350/350; snapshot at the raise decision reports root 100/500, never 300; check-prefix actor changes at unchanged pot; hero all-in produces `NoDecision`; third all-in stays `MultiwayEv` even with a HU side pot |
| core-model | `card_parser_roundtrip` | "AsKd" and board strings parse and print identically; invalid strings rejected |
| core-ranges | `range_roundtrip_pio_strings` | Parse -> vector -> string -> vector identical for "AKs:0.5, 77+, A5o"; dash ranges written high-to-low |
| core-ranges | `class_expansion_multiplicity` | Pair 6, suited 4, offsuit 12; mass of "random" = 1326 |
| core-ranges | `public_blocking_board_only` | Combos containing board cards have weight 0; combos containing hero's cards keep their weight in public ranges |
| core-ranges | `hero_conditioned_copy` | The hero-conditioned copy zeroes combos sharing hero's cards and leaves the public range untouched |
| core-ranges | `range_hash_scale_invariant` | `hash_scaled(r) == hash_scaled(0.37 * r)`; any single weight change alters the hash |
| core-iso | `iso_class_count_1755` | Exactly 1,755 canonical flops |
| core-iso | `iso_orbit_sizes` | Orbits in {4, 12, 24}; 52 + 3,744 + 18,304 = 22,100 |
| core-iso | `iso_stabilizer_tiebreak` | Paired and monotone flops with permuted ranges canonicalize to the same key; distinct turn/river orders give distinct keys; inverse map round-trips every combo vector |
| core-eval | `eval_vs_phevaluator_full_5card` | All 2,598,960 5-card hands rank-order-equivalent to `fixtures/eval/phevaluator_5card.bin` |
| core-eval | `eval_vs_phevaluator_random_7card` | 200,000 fixed-seed 7-card samples match the oracle (10,000,000 under `exhaustive`) |
| core-eval | `equity_mc_within_standard_error` | Fixed seed; MC estimate within 4 standard errors of exact on 20 spots; reported `std_err` matches the binomial bound |
| core-eval | `equity_joint_disjoint_sampling` | Multiway samples never share a card; `InvalidRanges` when no compatible assignment exists; per-pot shares sum to 1 per pot |
| core-eval | `equity_budget_respected` | Cancel flag and budget stop enumeration within 50 ms of the budget |
| core-preflop | `pokerdata_units_source_scaling` (T1) | 1.84 source SB -> 0.92 source bb at 1/2 and at 2/5; non-zero fold EV subtracted under `absolute_stack_verified`; re-raise node; straddle-mapped node converts with `S` |
| core-preflop | `pokerdata_schema_mapping` | Sample fixtures: sparse zero-weight EV kept, absent EV is `None`, `combos` ignored, SB-limp/BB-check node loads, every explicit fold in history |
| core-preflop | `pokerdata_action_path_lookup` | Provider path -> `PreflopNodeKey` -> node for RFI, vs-3bet, vs-4bet, squeeze, SB limp; CO/SB cold-call-vs-3bet absent -> `MissingPreflopNode` |
| core-preflop | `depth_bucket_labels_per_prefix` | 97bb -> 100 `DepthBucket{prominent: false}`; 120bb -> 100 prominent; 125bb -> 150 (tie deeper); 260bb -> 200 clamped; a later fold does not change an earlier prefix's depth; `AsymmetricStacks` when stacks differ |
| core-preflop | `straddle_mapping_labels` | Virtual roles, `S` as unit, `StraddleMapped{posts}` at 1/2/4 and 2/5/10; short post and re-straddle rejected |
| core-preflop | `rake_profile_ordering` | 10%/$6 cap at 2/5 maps to "5% cap 0.5bb" with `RakeProfileMapped`; `TimeCharge` prefers unraked |
| core-preflop | `bundle_validation_quarantine` | Valid hash with malformed JSON, wrong shape, non-finite value, invalid token: bundle quarantined, others active |
| core-replay | `replay_bayes_two_combos` (T3) | Weights (1,1); likelihoods (.8,.2), (.25,1), (.9,.1) -> (.18,.02), posterior (0.9, 0.1) for every seat including hero; an inserted observed bet changes the posterior (forcing 1 fails) |
| core-replay | `replay_off_tree_pseudo_harmonic` | 73 into 100 with sizes 50/100: `f_A = 0.468`, `f_B = 0.532`, deviation 0.23, prominent; branch-dependent later actions combine as the sum over branches |
| core-replay | `replay_missing_continuation` | Deleted continuation -> pre-action range kept, `UnconditionedPriorStreet` persists, no invented branch, reach applied once |
| core-replay | `replay_snapshot_compatibility` | Two solves on one street plus undo: only the compatible revision's snapshot is used |
| core-replay | `replay_hero_out_of_support` | Positive total mass, zero hero-combo weight -> `HeroComboOutOfSupport` |
| cache | `cache_key_structural_identity` (T4) | Same board/ranges/tree at 2x chips hits with identical frequencies and doubled EV; check-prefix, opposite actor, changed path, changed adapter version, lock fingerprint, min-raise boundary, rake rule, different range hash and different canonical flop miss; SPR within 1% `Exact`, within 2% `SprBucketed`, beyond misses; a 20-chip pot hit carries `MenuRounded` |
| cache | `cache_inherited_reasons_survive` | A stored `ChartRounded`/`DeadlineBestSoFar`/`UnconditionedPriorStreet` reason appears on every hit; a stricter target never receives a stale `Exact` |
| cache | `cache_corrupt_entry_deleted` | Flipped byte -> miss, file removed; deletion failure non-fatal |
| cache | `cache_atomic_write_and_quota` | Temp-and-rename; oversized entry rejected; eviction by oldest `last_hit` under quota |

### 13.2 Solver contract (`solver-worker` integration tests, spawn the binary)
| Test | Spot | Assertion |
|---|---|---|
| `river_polarized_vs_bluffcatcher_analytic` | Qs Jd 7h 3c 2d; OOP (caller) AA weight 1; IP (bettor) QQ weight 1 (3 combos) + 54o weight 0.25 (12 combos, mass 3); pot 100, stacks 100; OOP check only (no leads, no raises), IP check or pot bet; no rake | IP bets 100% of QQ and 50 +- 3 pp of 54o; OOP calls 50 +- 3 pp; IP range EV 75 +- 1, OOP 25 +- 1 chips; exploitability <= 0.1% pot |
| `ev_convention_non_root_payoffs` | Same spot plus a turn-root spot with a check-check line | `normalize_ev` yields fold = 0 and analytic call/check values at non-root nodes; positive and negative EVs both match |
| `river_checkdown_seat_swap_oracle` | River, both stacks 0 (all-in earlier), no actions | Per-combo EV = equity x pot exactly for both seats; swapping seats leaves every value unchanged |
| `ev_conservation` | Identical ranges, river, no rake, `river_std_v1` | `EV_OOP + EV_IP = pot +- 0.5%` summed over the root ranges |
| `pinned_example_fixture` | `examples/basic.rs` ranges and board at the pinned commit; `fixtures/solver/basic_0p3.json` frozen from the library's own `solve()` | Per-combo strategy and EV at every flop node within 1e-3 through the worker protocol (the wasm-postflop README comparison cannot be reproduced: its ranges are unpublished, R8 §3) |
| `suit_permutation_metamorphic` | Any flop spot; monotone and paired boards included | Permuting suits of board and ranges gives identical strategies after inverse mapping (max abs diff <= 1e-4) |
| `rake_cap_applied` | River spot with 5%/cap where the cap binds | Terminal payoff differs from unraked by exactly the cap; `TimeCharge` matches unraked |
| `combo_matrix_two_named` | Ranges with "AsKs" and "7h7d" only, different action rows | Rows non-zero only at those indices; rows differ; `available` false elsewhere; transpose errors fail |
| `cancel_between_iterations` | Flop spot | `ack{accepted}` <= 50 ms; `result{cancelled}` within one iteration plus one exploitability pass (<= 1.0 s); no second terminal |
| `deadline_best_so_far` | Flop spot, `deadline_ms = 1000`, margin 200 | `best_so_far` with `elapsed_ms <= 1000`, a complete solution and `reached` measured |
| `exact_size_insertion_no_prune` | Villain flop bet 73 into 100, menu 50 | Tree contains both 50 and 73 at that node plus check; opponent's posterior after the 73 bet is not uniform over the root range; the requested node is the one after 73 |
| `lock_lifecycle` | River spot | `lock` then matching `solve`: locked rows unchanged, hero response differs; `lock` with another `spot` -> `lock_mismatch`; `lock` during `Solving` -> `rejected`; lock cleared after cancel |
| `protocol_rejections` | Malformed lines | Unknown tag, 1325-element range, non-finite number, duplicate id, busy `solve`, oversized line: typed rejection, no work, worker stays alive; EOF -> exit 0 within 2 s |
| `ready_reports_features` | Startup | `ready` contains `avx2` in `build_features`, `threads` equals the launch argument |
| `memory_admission` | Synthetic tree above the limit | `tree_too_large` with `estimate_bytes`, no allocation |

### 13.3 Engine goldens (`engine/tests/golden/*.json`, expected outputs committed)
`coverage_classification_golden` (HU flop; 3-way flop; third player all-in; two preflop folds then HU flop; HU now after a flop fold with and without dead money; opponent all-in; hero all-in; hero cards unknown), `bet_translation_golden` (below min, between, above max with and without all-in, single size, illegal mapped action moved), `tree_builder_golden` (in-tree detection with chip rounding; insertion alongside menus; duplicates merged; `tree_signature` stability; `UnsupportedHistory`), `replay_weights_golden` (three-seat preflop history; expected 1326 vectors), `recommendation_assembly_golden` (headline choice, tie break, chart headline wording, reason accumulation, `partial` on Unsupported), `facing_allin_golden` (T1: AhAd on Qs Jd 7h 3c 2d versus QQ + 54o weight 1/12: equity 0.25, pot 100 before the jam, jam/call 73, W 246, `EV(call) = -11.5` unraked, `-12.75` with cap rake 5; 54o weight 0.25: `+50` / `+47.5`; `-2.30 bb` at a 5-chip BB; hero's strategic range includes other hands and the headline uses AhAd), `identity_race_golden` (T5 with a fake worker and clock: hand A revision 7, undo to a non-decision, hand B with the same displayed revision, re-request B; A's Fast/Progress/Final and both B replies out of order; no stale event, snapshot or result accepted; an `ack` never frees admission).

### 13.4 UI
- Vitest + `@tauri-apps/api/mocks` `mockIPC`: `entry_flow_full_hand_keystrokes`, `begin_hand_stack_confirmation`, `undo_is_engine_command`, `stale_identity_discarded`, `illegal_keys_disabled`, `coverage_label_and_assumptions_render`, `progressive_fast_equity_provisional_final`, `no_decision_rendering`.
- One end-to-end WebDriver test on Windows (`tauri-driver`, `@wdio/tauri-service`; the Electron fallback uses `wdio` with `chromedriver`): `e2e_full_hand_srp_flop_recommendation` enters a full SRP hand to a flop decision and asserts a `Final` with a coverage label and numeric EV within 15 s.

### 13.5 Bench gate (`bench`)
- Suites: `bench/spots/{river_std,river_min,turn_std,turn_min,flop_fast,flop_min}.json` (5 spots each, store ranges at 100bb and 200bb, broad and narrow, high and low SPR); every production and retry template; `--reps 5` cold and warm; threads {8, 16, 24}.
- Recorded-hand suite committed now: `fixtures/hands/e2e/*.json`, 50 synthetic hands with a complete versioned input record (config, stacks, actions, board, expected coverage class) covering both depths, straddle mapping at 1/2/4 and 2/5/10, off-tree paths, HU-after-multiway streets, facing all-in, multiway, missing preflop nodes, and failure injections (worker OOM/EOF, malformed and oversized output, blocked cache I/O, slow allocation, no completed iteration, clock jump, suspend/resume). Real-session logs are added later with the same record format.
- Report `docs/bench/<date>-i7-13700K.md`: p50/p95/max wall time to `target_bp` 50, time-to-target, peak total RSS, thread count, cancel latency, Exact/Approximate/Unsupported proportions, count of street and final deadline violations.
- Gate: river p95 <= 2 s, turn p95 <= 6 s, flop_fast p95 <= 10 s, cache hit p95 <= 0.5 s, e2e (admission to `Final`, cold cache) p95 <= 15 s, zero final-delivery violations in the fault-injection suite, numeric EV present for every designated supported fixture. A gate failure reduces template size or scope; it never changes the definition of success.

---

## 14. Phasing and critical-path validations

### 14.1 Week 1
Toolchain decision (section 3.6); workspace and `proto` with wire fixtures; `core-model` with PokerKit fixtures incl. lifecycle and street-root snapshot; `core-ranges`; `core-iso`; `core-eval` with oracle tables (5-card full, 7-card sampled); V1 (worker builds with AVX2 at the pinned commit with the two patches); `bench` river/turn/flop_fast with store ranges (V2/V3); worker state machine, locks, per-action EV, cancel, progress (V4); `core-preflop` with the chart fallback and the PokerData converter/adapter behind V9; `core-replay`; engine river/turn path with absolute deadlines, watchdog and retry; recorded-hand suite; minimal keyboard UI showing `Fast`, `Equity` and `Final`.

### 14.2 Week 2
Flop path; `cache` (normalized keys) and the resumable pre-solver queue (tier 1 running, completion not required); bet translation; coverage labels and assumptions panel; undo as an engine command; decision log; all tests green; bench gate incl. fault injection; E2E test. Exploit slice only if all of the above is done and validated.

### 14.3 Phase 2 backlog
Full exploit layer (turn locks, best-response audit, observation updates); `flop_full_v1` pre-solves and more scenarios/depths; straddle and 250bb preflop trees (self-solved with HRC or a custom pack); bunching capability with its own preparation, memory and deadline tests; GPU batch pre-solving if a licensed backend appears; neural surrogate distilled from our own solves; multiway models; LAN/phone UI with the AGPL boundary review; quota and pre-solver dashboards.

### 14.4 Critical-path validations
| Id | Question | Method | Pass criterion | Blocks |
|---|---|---|---|---|
| V1 | postflop-solver builds on Windows 11 at the pinned commit with the two patches and `+avx2`; `ready` reports `avx2` | `cargo build --release -p solver-worker`; `river_polarized_vs_bluffcatcher_analytic`; `ready_reports_features` | Build and tests pass | everything postflop |
| V2 | Turn solve time for `turn_std_v1` with store ranges | `bench` turn spots | p95 <= 6 s (measured 0.17 s on the R8 analogue) | turn budget |
| V3 | Flop solve time and memory for `flop_fast_v1` with store ranges (460-570 hands per side, unmeasured in R8) | `bench` flop spots, cold and warm, threads {8,16,24} | p95 <= 10 s; f32 estimate <= 4 GiB; otherwise reduce the template | flop budget, pre-solver rate |
| V4 | Worker API: lock before solve, actor-owned per-action EV at non-root nodes, cancel between iterations, progress, `normalize_ev` formula from the pinned `interpreter.rs`, wager cap mechanism | contract tests of section 13.2 | All pass | deadline design, exploit slice, cache payload |
| V9 | PokerData: written personal-offline-use rights; pack conversion to the normalized format with frequencies and EV; R7 A1 schema, A2 units and reference, A3 absolute sizes, A4 completeness, A5 consistency, A6 provenance | User emails the vendor and obtains the sample/pack; `tools/pokerdata_convert.py` + `pokerdata_schema_mapping`, `pokerdata_units_source_scaling`, `pokerdata_action_path_lookup` | Rights confirmed and A1-A6 pass; A2 result sets `ev_reference`; else chart fallback only | preflop EV, pre-solver ranges |
| V14 | Settlement and reopening correctness of `core-model` | `side_pot_three_allins`, `side_pot_two_contested`, `cumulative_short_allins_reopen`, `state_machine_pokerkit_fixtures` | 200/200 fixtures match | coverage classifier, pot accounting |
| V21 | End-to-end max and p95 including cache misses, worker restarts and fault injection | `bench e2e` and `bench fault` on `fixtures/hands/e2e` | p95 <= 15 s, zero final-delivery violations | release |

---

## 15. Risks and mitigations (top 6)

| Rank | Risk | Mitigation in this design |
|---|---|---|
| 1 | Coverage mismatch: a six-max label masks a HU backend; multiway, all-in-third-player, HU-after-multiway and unsupported histories may be a large share of live decisions | Pot-eligible counting; `MultiwayStreetRoot` handled with dead money and a label; numeric EV only for supported roots; decision log measures the coverage matrix from the first sessions; multiway shows per-pot equity plus a separated experimental block |
| 2 | Preflop supply: pack format conversion may fail; EV reference and size semantics unverified (A2/A3); no straddle/250bb trees; accuracy unpublished | V9 gate with a 2-day time box and named alternatives; EV suppressed until A2; sizes unloadable until A3; `source_accuracy = unverified` displayed; charts labelled `ChartRounded`; missing nodes `Unsupported`, never inferred |
| 3 | Flop time and memory with realistic ranges: R8 measured 6.2 s / 970 MB at 179x264 hands; store ranges are 2-3x more combos and unmeasured; variance 1.3-2.6x | V3 with store ranges before the budget is trusted; AVX2 required; `_min` template; absolute deadlines with watchdog and `DeadlineBestSoFar`; i16 mode when the f32 estimate exceeds 4 GiB; cache and pre-solver for the common scenarios |
| 4 | Plausible-looking wrong EV (matrix orientation, EV reference, unit scaling, translation, street-root money, hero-card leakage) | Analytic river oracle (fixed masses), two-combo matrix test, non-root payoff contract, source-unit test at 1/2 and 2/5, street-root fixture, public-range blocking tests, suit-permutation metamorphic test, feature freeze until section 13 is green |
| 5 | Upstream solver unmaintained with 100+ `unsafe` sites and a lock-free `MutexLike`; compiler lints will keep breaking the vendored fork | Pinned commit with a patch log; worker isolated in its own process with typed exit handling; panics caught at the executor boundary; job object memory limit on the worker process |
| 6 | Operational/licensing fit fails late (toolchain gap for Tauri, venue rules, data rights) | Toolchain decision in week 1 with the Electron fallback; worker isolated with provenance kept; rights confirmed in writing before purchase; no distribution or network service; venue permission as open question 1 |

---

## 16. Open questions

1. Venue permission and physical setup: whether the intended room permits a device consultation during a hand. Not a design decision; it determines whether phase 1 is used live or in permitted study/replay.
2. Template sizes: `flop_fast_v1`, `turn_std_v1`, `river_std_v1` are placeholders until V2/V3 report with store ranges; the ids are versioned so the change is mechanical.
3. PokerData pack conversion: whether the `.7z` MonkerSolver export can be parsed into the normalized format with EV within the 2-day time box; if not, whether the user funds one month of API Pro or accepts the chart baseline. Also A2 (EV reference) and A3 (absolute size per label) outcomes.
4. Toolchain: install the Visual Studio 2022 "Desktop development with C++" workload for Tauri 2, or build the shell with the Electron fallback on the GNU toolchain (user decision, pending).
5. Wager cap mechanism: whether the pinned commit offers a native raise-count limit or `remove_lines` must implement it (V4).
