# PokerAI assistant: design specification (2026-09-10)

Source of truth for decisions: `docs/design/2026-09-10-design-outline.md`. Facts: `docs/research/SYNTHESIS.md` as corrected by `docs/research/REVIEW-of-research.md`. Items the outline leaves unspecified are chosen here and marked **(spec decision)**. Every numeric latency or memory figure is a hypothesis until `bench` measures it on the target machine (i7-13700K, 64 GB, Windows 11).

---

## 1. Purpose and scope

### 1.1 Goal
For each hero decision point in a 6-max NLHE cash hand entered by hand at a live table, return within a 15 s hard deadline (measured from the `recommend` call, human entry excluded):
- the legal actions, each with a frequency and an incremental EV in big blinds (fold = 0),
- a coverage label (Exact / Approximate / Unsupported) with the reasons,
- the assumptions used (ranges, tree, accuracy reached, elapsed time, translations),
- progressively: a fast equity-only answer first, then the solver answer when ready.

GTO baseline first. Exploit slice (section 11) only after the baseline is validated.

### 1.2 Non-goals (phase 1)
Screen capture; tournaments/ICM; solved EV for 3+ pot-eligible players; GPU solving; neural models; phone/LAN UI; distribution to anyone else; antes **(spec decision: not in the game config)**; button/Mississippi straddles **(spec decision: only the UTG straddle is modelled)**.

### 1.3 User decisions (final, from the outline)
| Decision | Value |
|---|---|
| Approach | A: solver-core assistant, HU postflop solver + preflop store |
| Format | 6-max NLHE cash, manual entry, Windows 11 desktop, private use only |
| Preflop data | PokerData (after verifying provenance and offline/live-use rights, V9); fallback = transcribed free charts labelled "rounded, no EV" |
| Game config | Per session: blinds, UTG straddle, rake (pot rake with cap) or time charge, seats and stacks |
| Week-1 targets | 100bb and 200bb, 5% pot rake with cap, UTG straddle |
| Budget | Strong baseline in 1-2 weeks, then iterate |
| AGPL | Private use imposes no obligations; solver still isolated in its own process |

---

## 2. Definitions

- **Decision point**: a state where hero is to act and has at least two legal actions. If hero is all-in, or the only opponent is all-in and hero can only call or fold, see section 5 step 6.
- **Street root**: the first decision node of the current street. **Line / history**: the actions taken on the current street before the decision point.
- **Pot-eligible player**: a player who has not folded, including all-in players. A folded player is never pot-eligible. **HU spot**: exactly 2 pot-eligible players. A third player who is all-in makes the spot non-HU (their share of the main pot changes payoffs).
- **Effective stack**: min of the two HU players' remaining stacks. **Depth**: effective stack at the start of the hand, in bb.
- **Money units**: all `HandState`, worker and cache amounts are integer chips (`u32`). The engine converts EV to bb only for display: `ev_bb = ev_chips / bb_chips`. Nothing is ever rounded to fit a stored node.
- **Bet size (% pot)**: `s = (chips added beyond a call) / (pot after the bettor's call)`. An opening bet has call = 0, so `s = bet / pot`. Raises are entered as raise-to amounts in chips and converted with this formula.
- **EV convention**: `EV(a) = E[hero's final stack | a] - hero's current stack`, in chips, converted to bb. Fold = 0 exactly; chips already in the pot are sunk. Rake is subtracted at the terminals that pay it. The same reference holds for check (no fold available).
- **Coverage labels**:
  - `Exact`: every input was matched exactly (node present, sizes in tree, depth within tolerance, solve reached its accuracy target).
  - `Approximate{reasons}`: numeric EV shown, with one or more reasons: `BetTranslation`, `DepthBucket`, `RakeProfileMapped`, `StraddleMapped`, `DeadlineBestSoFar`, `ChartRounded` (chart-transcription source: frequencies only), `UnconditionedPriorStreet`.
  - `Unsupported{reason}`: no numeric EV; equity only. Reasons: `MultiwayEv`, `MissingPreflopNode`, `EngineError`, `TreeTooLarge`.
- **Canonical flop**: the representative of a flop's suit-isomorphism class under the 4! suit permutations. There are 1,755 classes: 13 trips (orbit 4), 312 paired (orbit 12), 286 distinct-rank patterns × 5 suit patterns (monotone orbit 4; three two-tone placements orbit 12 each; rainbow orbit 24). Orbit sizes are 4, 12 or 24 only and sum to 22,100. A class that is not in the cache is solved live; there is no "nearest flop" substitution.
- **Tree template**: a versioned id naming per-street bet sizes (% pot), raise sizes, all-in thresholds and raise cap for a solve. **Effective tree**: a template plus exact chip sizes inserted for actions already taken on the current street plus pruned sibling lines (section 10.2).
- **Bet translation**: mapping an observed size `s` that is not in a tree to the adjacent tree sizes `A <= s <= B` with pseudo-harmonic weights `f_A = (B - s)(1 + A) / ((B - A)(1 + s))`, `f_B = 1 - f_A`. Below the smallest size: `f_A = 1`. Above the largest non-all-in size: map to the largest size, or to all-in when all-in is in the tree and `s` is closer to it. Deviation `d = min(|s - A|, |s - B|)` over the sizes actually used. Label `BetTranslation` when `d > 0.10` **(spec decision: threshold 10% of pot)**. Used for prior streets and for preflop lookups; never for the current street (section 10.2).
- **Range**: a `[f32; 1326]` weight vector, weights in `[0, 1]`, indexed by combo index (section 4.1).

---

## 3. Architecture

### 3.1 Processes
| Process | Technology | Role |
|---|---|---|
| `pokerai-ui` | Tauri 2 app: Rust shell crate `pokerai-app` + Vite/React/TypeScript frontend | Keyboard-first entry, display, undo stack; hosts the `engine` in-process |
| `solver-worker` | Separate Rust binary (AGPL-3.0) | Wraps b-inary/postflop-solver at a pinned commit; JSON lines over stdio; one job at a time |
| `tools/` | Python 3.12, dev only | Test oracles (phevaluator, PokerKit), chart transcription ingestion, PokerData fetch, bench report scripts. Never a runtime dependency |

The engine spawns the worker at app start with `std::process::Command`, keeps it alive between requests (allocations stay warm), restarts it on exit or protocol error.

### 3.2 Crates
| Crate | Purpose | Depends on | License |
|---|---|---|---|
| `proto` | Serde types: `Card`, `GameConfig`, `HandState`, `Action`, `Recommendation`, `Coverage`, worker messages. Single source of truth for UI (via TS bindings), engine and worker | `serde` | MIT OR Apache-2.0 |
| `core-model` | "AsKd" parser and display; 6-max seat/position model with UTG straddle posting and action order; hand-state machine (streets, pot, side pots, all-ins, min-raise, short all-in does not reopen); legal actions | `proto` | MIT OR Apache-2.0 |
| `core-ranges` | Pio-style range strings with weights ("AKs:0.5, 77+") to and from `[f32; 1326]`; 169-class to 1326 expansion (6/4/12 multiplicities); card removal for board and known cards; merge/scale | `proto` | MIT OR Apache-2.0 |
| `core-iso` | Flop canonicalization to 1,755 classes with orbit sizes; suit permutation and inverse; turn/river extension of a canonical flop. Exact-match keys only | `proto` | MIT OR Apache-2.0 |
| `core-eval` | 7-card evaluator (b-inary holdem-hand-evaluator, MIT; rs-poker as fallback feature); weighted range-vs-range equity via `pokers` (MIT): exact enumeration and Monte Carlo | `proto`, `core-ranges` | MIT OR Apache-2.0 |
| `core-preflop` | `PreflopSource` trait; adapters `PokerDataJson` and `ChartTranscription`; versioned bundles on disk; node lookup; depth/rake/straddle mapping | `proto`, `core-ranges`, `core-model` | MIT OR Apache-2.0 |
| `core-replay` | Bayesian range replay for every seat across all streets | `proto`, `core-ranges`, `core-model`, `core-preflop` | MIT OR Apache-2.0 |
| `cache` | On-disk exact-match store of flop/turn solves; background pre-solver queue | `proto`, `core-iso` | MIT OR Apache-2.0 |
| `engine` | Coverage classifier; tree builder; bet translation; cache lookup; worker client with deadline, cancel, restart; result assembly; decision log | all `core-*`, `cache`, `proto` | MIT OR Apache-2.0 |
| `bench` | Spawns the worker on fixed spots per template; p50/p95 wall time and peak RSS; markdown report; release gate | `proto`, `engine` | MIT OR Apache-2.0 |
| `solver-worker` | Binary; protocol loop; adapter to postflop-solver | `proto`, `third_party/postflop-solver` | AGPL-3.0 |
| `pokerai-app` | Tauri shell: commands, `Channel<T>` events, worker lifecycle, config persistence | `engine`, `proto`, `tauri` | MIT OR Apache-2.0 |

Dependency direction is strictly downward in the table order: `proto` <- `core-*` <- `cache`/`engine` <- `bench`/`pokerai-app`. `solver-worker` depends on `proto` only. No crate depends on `solver-worker`; the engine talks to it only over stdio.

Repository layout **(spec decision)**:
```
Cargo.toml                      workspace
crates/{proto,core-model,core-ranges,core-iso,core-eval,core-preflop,core-replay,cache,engine,bench}
solver-worker/                  AGPL crate (workspace member, separate binary)
third_party/postflop-solver/    vendored fork at pinned commit, LICENSE retained, PINNED_COMMIT file
apps/pokerai-ui/                Vite frontend + src-tauri (crate pokerai-app)
tools/                          Python 3.12 (pyproject.toml), dev only
fixtures/                       committed test fixtures (generated by tools/)
docs/
```

### 3.3 AGPL boundary
- `solver-worker` is the only crate that links AGPL code. Its `Cargo.toml` declares `license = "AGPL-3.0"`. All other crates are `MIT OR Apache-2.0`.
- The boundary is a process boundary with a documented JSON-lines protocol (`proto::worker`). `proto` is permissive; AGPL code may consume it.
- Private use creates no obligation. `third_party/postflop-solver/LICENSE`, the pinned commit and our diff (`third_party/postflop-solver/PATCHES.md`) are kept so a Corresponding Source archive can be produced if the app is ever shared or run for other users over a network. That review is a phase-2 item; nothing in phase 1 is distributed or served.

### 3.4 Threading model
- Tauri commands never block: `recommend` enqueues a request and returns; results flow through `Channel<RecommendationEvent>`.
- Engine threads (std threads, never Tokio workers) **(spec decision)**:
  - `engine-main`: owns the request queue, admission (at most one solve in flight), cancellation, worker client state.
  - `fast-path`: fast-phase validation, replay and equity for the newest request.
  - `worker-stdin` / `worker-stdout`: blocking pipe I/O with the worker.
  - `presolver`: schedules background jobs when idle (section 10.4).
- The worker uses rayon with 16 threads by default **(spec decision; bench compares 8/16/24 in V2/V3)**. Windows priority class of the worker: NORMAL for live jobs, BELOW_NORMAL while a pre-solve job runs **(spec decision)**.

### 3.5 Component interfaces (public Rust surface; sketches)
| Component | Interface |
|---|---|
| `core-model` | `parse_card(&str) -> Result<Card>`, `parse_hand(&str) -> Result<[Card; 2]>`, `derive(&GameConfig, &HandState) -> Derived`, `apply_action(&GameConfig, &HandState, Action) -> Result<HandState, RulesError>`, `new_hand(&GameConfig, button, hero, hero_cards) -> HandState`, `set_board(&HandState, &[Card]) -> Result<HandState>` |
| `core-ranges` | `parse_range(&str) -> Result<Range1326>`, `range_to_string(&Range1326) -> String`, `expand_169(&[f32; 169]) -> Range1326`, `remove_blocked(&mut Range1326, &[Card])`, `mass(&Range1326) -> f32` |
| `core-iso` | `canonical_flop(&[Card; 3]) -> (CanonicalFlop, SuitPerm)`, `orbit_size(CanonicalFlop) -> u8`, `apply(&SuitPerm, Card) -> Card`, `inverse(&SuitPerm) -> SuitPerm`, `canonical_board(&[Card]) -> (CanonicalBoard, SuitPerm)` for 4 and 5 cards |
| `core-eval` | `rank7(&[Card; 7]) -> u16`, `equity_exact(&[Range1326], &[Card]) -> Vec<f32>`, `equity_mc(&[Range1326], &[Card], samples: u32, seed: u64) -> Vec<f32>`, `combo_equity(combo, &Range1326, &[Card]) -> f32` |
| `core-preflop` | section 8.1 |
| `core-replay` | section 9.1 |
| `cache` | `Cache::open(dir) -> Cache`, `get(&CacheKey) -> Option<CacheEntry>`, `put(&CacheKey, &CacheEntry)`, `Presolver::{start, pause, resume, status}` |
| `engine` | `Engine::new(GameConfig, Paths) -> Engine`, `set_config(GameConfig)`, `recommend(HandState, sink: Box<dyn Fn(RecommendationEvent) + Send>) -> DecisionId`, `cancel(DecisionId)`, `shutdown()` |
| `pokerai-app` (Tauri commands) | `set_game_config(cfg)`, `new_hand(button, hero, hero_cards) -> HandState`, `apply_action(state, action) -> Result<HandState>`, `set_board(state, cards) -> Result<HandState>`, `recommend(state, on_event: Channel<RecommendationEvent>) -> DecisionId`, `cancel(decision_id)`, `set_seat_tag(seat, tag, facts)`, `presolver_status() / presolver_pause() / presolver_resume()`. Undo is UI-local (snapshot stack), no command |
| `bench` | CLI `bench run --suite {river_std,turn_std,flop_fast,flop_min,e2e} --threads N --out docs/bench/` |
| `solver-worker` | stdin/stdout JSON lines, section 4.5; exit code 0 on `Shutdown`, non-zero on panic |

TypeScript types for every `proto` type are generated at build time (`ts-rs` **(spec decision)**) so the UI never hand-declares a message shape.

---

## 4. Data model (Rust-like sketches; `proto` unless noted)

### 4.1 Cards, combos, seats
```rust
pub struct Card(pub u8);            // id = rank_index*4 + suit_index; ranks 2..A = 0..12; suits c,d,h,s = 0..3 (spec decision)
pub type ComboIndex = u16;          // for cards lo < hi: idx = hi*(hi-1)/2 + lo, 0..1326 (spec decision)
pub type Range1326 = Vec<f32>;      // len 1326, weights in [0,1]
pub enum Position { Utg, Hj, Co, Btn, Sb, Bb }
pub struct Seat(pub u8);            // 0..5, clockwise; button seat given per hand
pub enum Street { Preflop, Flop, Turn, River }
```
The worker adapter never assumes the library shares this ordering; it maps by named combos (test `combo_ordering_named_check`).

### 4.2 Game config (per session)
```rust
pub struct GameConfig {
    pub config_id: u64,
    pub sb_chips: u32, pub bb_chips: u32,
    pub straddle: Option<UtgStraddle>,      // amount_chips; posting and order in core-model
    pub rake: Rake,                         // PotRake{rate: f32, cap_chips: u32, no_flop_no_drop: bool} | TimeCharge (treated as unraked)
    pub seats: Vec<SeatConfig>,             // seat, stack_chips at session start, tag: Option<SeatTag>, facts: Vec<QuickFact>
    pub solver: SolverPrefs,                // threads: u8 = 16, bunching: bool = false, target_bp: u16 = 50
}
```

### 4.3 Hand state and actions
```rust
pub enum Action { Fold, Check, Call, Bet { to_chips: u32 }, Raise { to_chips: u32 }, AllIn }
pub struct TakenAction { pub seat: Seat, pub street: Street, pub action: Action, pub amount_chips: u32 /* total put in by this action */ }
pub struct HandState {
    pub hand_id: u64, pub state_version: u32,        // incremented on every mutation, including undo
    pub config_id: u64,
    pub button: Seat, pub hero: Seat, pub hero_cards: Option<[Card; 2]>,
    pub stacks_start: Vec<u32>,                      // per seat at hand start
    pub board: Vec<Card>,                            // 0, 3, 4 or 5 cards
    pub actions: Vec<TakenAction>,                   // posts are implied by GameConfig
    pub derived: Derived,                            // computed by core-model, never entered
}
pub struct Derived {
    pub street: Street, pub to_act: Option<Seat>,
    pub pot_chips: u32,                              // all chips committed so far, all streets
    pub committed_this_street: Vec<u32>, pub stacks_remaining: Vec<u32>,
    pub folded: Vec<bool>, pub all_in: Vec<bool>,
    pub pots: Vec<Pot>,                              // main and side pots: amount, eligible seats
    pub legal: Vec<LegalAction>,                     // kind, min_to_chips, max_to_chips
}
```

### 4.4 Recommendation and coverage
```rust
pub enum Coverage { Exact, Approximate { reasons: Vec<ApproxReason> }, Unsupported { reason: UnsupportedReason } }
pub enum ApproxReason {
    BetTranslation { street: Street, observed_pct: f32, mapped: Vec<(f32, f32)> /* (size, weight) */, deviation: f32 },
    DepthBucket { actual_bb: f32, used_bb: u16 }, RakeProfileMapped { actual: String, used: String },
    StraddleMapped, DeadlineBestSoFar { reached_bp: u16, target_bp: u16 }, ChartRounded, UnconditionedPriorStreet { street: Street },
}
pub enum UnsupportedReason { MultiwayEv { pot_eligible: u8 }, MissingPreflopNode { key: String }, EngineError { message: String }, TreeTooLarge }

pub struct ActionAdvice { pub action: Action, pub frequency: f32, pub ev_bb: Option<f32>, pub headline: bool }
pub struct Recommendation {
    pub hand_id: u64, pub state_version: u32, pub phase: Phase,      // Fast | Final
    pub coverage: Coverage,
    pub actions: Vec<ActionAdvice>,                                   // hero's actual combo; frequencies from hero's strategic range at that combo
    pub equity: EquitySummary,                                        // hero combo vs each live range; range vs range; method Exact|MonteCarlo{samples}
    pub assumptions: Assumptions,                                     // ranges_used per seat (summary string + combo count), tree_id, template_id,
                                                                      // target_bp, reached_bp, elapsed_ms, cache_hit, notes: Vec<String>
    pub experimental: Option<ExperimentalHu>,                         // multiway only: HU solve vs strongest range, section 6
    pub exploit: Option<ExploitAdvice>,                               // section 11
}
```

### 4.5 Worker protocol (`proto::worker`, one JSON object per line, `id` correlates request and replies)
```rust
pub enum Request {
    Solve { id: u64, spot_id: u64, board: Vec<Card>, oop: Range1326, ip: Range1326, hero_is_oop: bool,
            pot_chips: u32, eff_stack_chips: u32, rake_rate: f32, rake_cap_chips: u32,
            tree: EffectiveTree, history: Vec<Action>, target_bp: u16, deadline_ms: u32,
            bunching: Option<Vec<Range1326>>, report_street_strategy: bool },
    Lock   { id: u64, spot_id: u64, locks: Vec<NodeLock> },    // staged; applied after allocation, before solving (library lifecycle)
    Cancel { id: u64, target: u64 },
    Shutdown { id: u64 },
}
pub enum Reply {
    Ready    { proto_version: u16, solver_commit: String, threads: u8 },   // once at startup (spec decision)
    Progress { id: u64, iterations: u32, exploitability_chips: f32, elapsed_ms: u32 },  // at most every 100 ms
    Result   { id: u64, status: SolveStatus /* Ok | BestSoFar | Cancelled | Error{message} */,
               node: NodeStrategy,                        // at the decision node: per-combo action probs + per-combo per-action EV chips (hero)
               street_strategy: Option<Vec<NodeStrategy>>,// every decision node on the current street, in tree order
               reached_chips: f32, iterations: u32, elapsed_ms: u32, memory_bytes: u64 },
    Ack      { id: u64 },                                 // for Lock, Cancel, Shutdown
}
pub struct NodeStrategy { pub path: Vec<Action>, pub actor_is_oop: bool, pub actions: Vec<Action>, pub probs: Vec<Vec<f32>> /* [combo][action] */, pub ev_chips: Option<Vec<Vec<f32>>> }
pub struct NodeLock { pub path: Vec<Action>, pub actor_is_oop: bool, pub probs: Vec<Vec<f32>> }
```
`proto_version` mismatch at `Ready` is an `EngineError` for every request until the worker is rebuilt.

---

## 5. Data flow for one decision

1. **Session start.** User sets `GameConfig` (blinds, straddle, rake, seats, stacks); optionally seat tags. Saved to `%APPDATA%\PokerAI\config.json`. Engine spawns the worker, waits for `Ready`, opens the cache and preflop bundles.
2. **Hand entry.** `new_hand(button, hero, hero_cards)`; then per event one keystroke: fold/check/call/all-in, or bet/raise-to with chips typed; board cards per street. Every mutation goes through `apply_action(state, action) -> Result<HandState>` in `core-model` (pure function; illegal actions rejected). The UI keeps the undo stack as `HandState` snapshots; every accepted mutation or undo bumps `state_version`.
3. **Request.** At a decision point the UI calls `recommend(state, channel)`. `engine-main` cancels any in-flight job with an older `state_version`, then starts the fast phase.
4. **Fast phase (target <= 0.3 s).** Validate (`Derived` recomputed and compared); classify coverage (section 6); replay ranges for all seats (section 9); compute equity (exact enumeration on turn/river, Monte Carlo with 50,000 samples preflop/flop **(spec decision)**); emit `Fast(Recommendation)` with `actions` frequencies empty and `ev_bb = None`.
5. **Preflop decision.** `core-preflop` lookup (section 8). Present node: frequencies and EV (or frequencies only for charts) -> `Final` with `Exact`/`Approximate`. Absent node -> `Final` with `Unsupported{MissingPreflopNode}`.
6. **Postflop, HU.** If the opponent is all-in and hero can only call or fold: `EV(call)` from exact range-vs-range equity over all runouts with rake, `Exact`, no worker **(spec decision)**. Otherwise: build the effective tree (section 10.2); cache lookup (flop/turn only); hit -> `Final` immediately with the stored coverage; miss -> `Solve` to the worker with the street deadline; forward `Progress`; on `Result` assemble `Final` (`Exact` if `reached <= target`, else `Approximate{DeadlineBestSoFar}`); store in cache (flop/turn).
7. **Postflop, 3+ pot-eligible.** `Final` with `Unsupported{MultiwayEv}`, equity for all live players, and `experimental` (section 6) if it completes within the street deadline.
8. **Display.** Headline = highest-EV action for hero's actual combo (ties broken by higher frequency); the frequency mix is always shown. Coverage label and assumptions panel always visible. Results whose `state_version` differs from the UI's current version are discarded.
9. **Log.** Every request and its `Final` (coverage, reasons, elapsed, cache hit) is appended to `%LOCALAPPDATA%\PokerAI\decisions.jsonl` to measure coverage and latency over real sessions **(spec decision)**.

Per-street behaviour of step 6: **flop** solves use the flop template from the flop root with ranges from preflop replay; **turn** solves are new solves rooted at the turn (ranges = flop-end ranges from replay); **river** solves are rooted at the river and never cached. Prior-street solutions are used only through replay (section 9), never as the current street's strategy.

### 5.1 Keyboard entry (spec decision)
| Key | Effect |
|---|---|
| `N` | New hand: prompts button seat (digit 1-6), hero seat (digit), hero cards (four characters, rank then suit: `AsKd`) |
| `F` / `C` / `A` | Fold / check-or-call / all-in for the seat to act (`Derived.to_act`) |
| `B` then digits, `Enter` | Bet or raise **to** the typed chip amount; `Esc` cancels the entry |
| rank+suit characters, `Enter` | Board cards when the street ends (3, then 1, then 1); duplicates rejected |
| `Ctrl+Z` | Undo (pops one snapshot; bumps `state_version`) |
| `Space` | Re-request `recommend` for the current state |
| `T` | Open the seat-tag panel for the seat to act (exploit slice only) |

Illegal keys are disabled from `Derived.legal`; the seat to act is always derived, never chosen by hand. WebView2 browser accelerators (F5, Ctrl+P, F12) are suppressed with `preventDefault` in the frontend and `with_browser_accelerator_keys(false)` where Tauri 2 exposes it.

---

## 6. Coverage rules

| Situation | Numeric EV | Label | What is shown |
|---|---|---|---|
| Preflop, node present, PokerData bundle | Yes | `Exact`, or `Approximate` with `DepthBucket` / `RakeProfileMapped` / `StraddleMapped` / `BetTranslation` | Frequencies, EV, equity |
| Preflop, node present, chart bundle | No (frequencies only) | `Approximate{ChartRounded, ...}` | Frequencies, equity |
| Preflop, node absent | No | `Unsupported{MissingPreflopNode}` | Equity only |
| Postflop, exactly 2 pot-eligible, opponent not all-in | Yes | `Exact` or `Approximate` with `BetTranslation` (prior street), `DeadlineBestSoFar`, `UnconditionedPriorStreet`, plus any preflop reasons inherited from replay | Frequencies, EV, equity, progress |
| Postflop, exactly 2 pot-eligible, opponent all-in | Yes | `Exact` | EV(call), EV(fold)=0, equity |
| Postflop, 3+ pot-eligible (any all-in counts) | No | `Unsupported{MultiwayEv}` | Equity for all live players; `experimental` block |
| Hero all-in, or no legal choice | n/a | no request | UI shows "no decision" |
| Worker failure after retry | No | `Unsupported{EngineError}` | Equity only, visible reason |

Rules:
- Pot-eligible counting uses `Derived.folded` only; all-in players count. Folded players may be bunched (`SolverPrefs.bunching`, default off) but never add a player.
- Coverage reasons accumulate: a turn solve after a translated flop bet and a depth-bucketed preflop lookup carries both reasons.
- `experimental` (multiway only): a HU solve of hero vs the single opposing range with the highest weighted equity against hero's range **(spec decision)**, same template and deadline as a HU spot, rendered in a visually separate block titled "experimental, not solved", never in `actions`, never with the `Exact` label.
- A HU spot whose prior street was multiway is still HU now; its ranges are replayed with `UnconditionedPriorStreet` for the multiway street (section 9.3).

---

## 7. Latency budget and progressive display

Hard deadline: 15 s from `recommend` to `Final`, including retries. Street budgets apply to the first attempt.

| Stage | Budget | Notes |
|---|---|---|
| Validation + coverage + replay | <= 0.15 s | fast-path thread |
| Equity (fast phase emitted) | <= 0.3 s cumulative | exact turn/river; MC 50k preflop/flop |
| Preflop lookup | <= 0.05 s | in-memory bundle |
| Cache lookup (flop/turn) | <= 0.5 s | single file read + zstd |
| River solve | <= 2 s | `river_std_v1`; not cached |
| Turn solve | <= 6 s | `turn_std_v1` |
| Flop solve, cache miss | <= 10 s | `flop_fast_v1` |
| Worker cancel acknowledgement | <= 0.2 s | one iteration; measured by bench |
| Crash restart + retry | remaining time to 15 s | min template (section 12) |
| Exploit slice (river only) | + <= 2 s | second locked solve |

Progressive display:
- `Fast` at <= 0.3 s: legal actions, equities, coverage label (may already be `Unsupported`).
- `Progress` every <= 250 ms while solving: iterations, exploitability as % of pot, elapsed.
- `Final` when the solve reaches `target_bp` (default 50 = 0.5% of pot **(spec decision)**), or at the deadline with best-so-far labelled `DeadlineBestSoFar{reached_bp}`.
- Deadline semantics in the worker: `deadline_ms` covers tree build, allocation, iterations, finalize and extraction; the worker reserves `max(100 ms, 10% of deadline)` for finalize and extraction **(spec decision)** and checks the clock between `solve_step` iterations.
- Admission: one live solve at a time; a new request cancels the older one; a pre-solve job is cancelled by any live request. The engine waits <= 500 ms for `Ack`/`Result{Cancelled}`, otherwise kills and restarts the worker **(spec decision)**.

Gate: `bench` p95 per street within budget and V21 end-to-end p95 <= 15 s including cache misses.

---

## 8. Preflop store (`core-preflop`)

### 8.1 Interface
```rust
pub trait PreflopSource {
    fn bundle_info(&self) -> &BundleInfo;                       // id, source kind, depth_bb, rake profile, straddle: bool, version, license note
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode>;
}
pub struct PreflopNodeKey { pub depth_bb: u16, pub rake_profile: String, pub straddle: bool, pub history: Vec<(Position, PreflopStep)> }
pub enum PreflopStep { Fold, Call, Raise { to_bb_x100: u32 }, AllIn }
pub struct PreflopNode { pub actor: Position, pub actions: Vec<PreflopStep>, pub probs: Vec<Vec<f32>> /* [169 class][action] */, pub ev_bb: Option<Vec<Vec<f32>>>, pub reach: Vec<f32> /* [169] */ }
pub struct PreflopStore { bundles: Vec<Box<dyn PreflopSource>> }
impl PreflopStore { pub fn query(&self, cfg: &GameConfig, state: &HandState, seat: Seat) -> PreflopAnswer /* node expanded to 1326 + reasons, or Unsupported */ }
```

### 8.2 Adapters
- `PokerDataJson`: reads the raw JSON files saved by `tools/pokerdata_fetch.py` under `%LOCALAPPDATA%\PokerAI\preflop\pokerdata-<manifest-date>\`. Uses the provider's documented action path for node identity. Converts EV from small-blind units to bb (`ev_bb = ev_sb * sb_chips / bb_chips`, verified by test `pokerdata_adapter_units_sb_to_bb`). Keeps EVs for zero-weight actions. Depths available per manifest (20-200bb); rake profile "5% cap 0.5bb".
- `ChartTranscription`: reads `fixtures/charts/<name>.json` produced by `tools/chart_ingest.py` (169-class frequencies per node, no EV). Every answer carries `ChartRounded`. Depths: 100 (PokerCoaching) and 200 (RangeConverter).
- Bundles are versioned directories with `manifest.json` (`bundle_id`, `source`, `version`, `sha256` of contents, `license_note`, `depths`, `rake_profile`, `straddle`). The store loads all bundles present; PokerData bundles take precedence over charts at equal depth **(spec decision)**.

### 8.3 Lookup rules
- **Depth**: `depth_bb = min(hero_stack_start, max over unfolded opponents' stack_start) / bb`. Choose the bundle depth with the smallest absolute difference; ties go deeper **(spec decision)**. `Exact` if within 5% of the chosen depth, else `DepthBucket{actual, used}` **(spec decision: 5% tolerance)**. The actual depth is always listed in assumptions.
- **Rake**: choose the bundle whose (rate, cap in bb) is closest by cap difference; `RakeProfileMapped` when it differs from the session rake. `TimeCharge` prefers an unraked bundle; if none, nearest raked with the label.
- **Straddle**: `core-model` models the UTG straddle as a real post with preflop order HJ, CO, BTN, SB, BB, UTG(straddler); postflop order is unchanged. If a bundle with `straddle = true` exists at the mapped depth, use it. Otherwise map to the non-straddle tree **(spec decision on the mapping)**: positions shift one step (HJ->UTG, CO->HJ, BTN->CO, SB->BTN, BB->SB, straddler->BB), depth is measured in straddle units (`stack / straddle_chips`), raise sizes are converted to straddle units, label `StraddleMapped`. Assumptions note: "SB and BB posts (0.25 and 0.5 straddle units) are not represented".
- **Sizes**: each observed raise-to is converted to % pot at that node and matched against the bundle's sizes; an exact match (within the chip rounding of the bundle size) is used directly; otherwise bet translation (section 2) mixes the two adjacent nodes' strategies: `probs = f_A * probs_A + f_B * probs_B` and likewise for EV, reason `BetTranslation`.
- **Unsupported**: no node for the history (limp lines absent, branch cut by the bundle, more raises than the bundle tree), or no bundle at all. Returned as `Unsupported{MissingPreflopNode{key}}`; never a guessed fold.
- 169 -> 1326 expansion: each combo of a class receives the class values; card removal (board, hero's cards) is applied by the caller (`core-replay`), not by the store.

---

## 9. Range replay (`core-replay`)

### 9.1 Interface
```rust
pub struct ReplayInput<'a> { pub cfg: &'a GameConfig, pub state: &'a HandState, pub store: &'a PreflopStore, pub solved: &'a StreetStrategies /* by (hand_id, street) */ }
pub struct ReplayOutput { pub ranges: Vec<Option<Range1326>> /* per seat; None when folded before any action */, pub folded_ranges: Vec<Range1326>, pub reasons: Vec<ApproxReason>, pub unsupported: Option<UnsupportedReason> }
pub fn replay(input: ReplayInput) -> ReplayOutput;
```

### 9.2 Rules
- Start: every dealt seat has the uniform range of 1326 combos with weight 1, minus combos blocked by hero's cards and the board.
- Preflop: for each observed action in order, `weight[combo] *= P(action | combo, node)` from the store (expanded to 1326, with translation mixing for off-tree sizes). Folds condition the folder's range too (kept as `folded_ranges` for bunching).
- Postflop: for each observed action on a completed street, `weight[combo] *= P(action | combo)` from the `NodeStrategy` of that street's solve (from `street_strategy`, cached or in-session). An off-tree observed size on a completed street is handled by translation over the solved tree's sizes at that node (`BetTranslation`).
- Hero's own actions condition hero's range the same way; hero keeps a strategic range. Hero's actual combo is used only for the headline and the per-combo EV display.
- Zero-mass guard: if a seat's range mass drops below `1e-6` of its starting mass, keep the pre-action range and add `UnconditionedPriorStreet` for that street **(spec decision)**.
- Renormalization is never applied to weights (weights are reach probabilities); the solver receives them as is.

### 9.3 Missing strategies
- A preflop node missing during replay for a postflop decision: the action is left unconditioned for that seat, reason `UnconditionedPriorStreet{Preflop}`; the decision remains `Approximate` rather than `Unsupported`, because the postflop solve is still exact given the declared ranges **(spec decision)**.
- A completed street with no solve (multiway at the time, engine error, or hero not yet consulted on that street): unconditioned, reason `UnconditionedPriorStreet{street}`.

---

## 10. Postflop engine (`engine` + `solver-worker` + `cache`)

### 10.1 Tree templates (all sizes % pot; raise sizes % pot after the call; "a" = all-in)
| Template id | Street | Bet sizes OOP / IP | Raise sizes | add-all-in threshold | force-all-in threshold | Raise cap |
|---|---|---|---|---|---|---|
| `flop_fast_v1` | flop root | 50 / 50; turn 75; river 75 | 100 | 150% | 20% | 2 per street |
| `flop_min_v1` | flop root | 75 / 75; turn 75; river 75 | a | 150% | 20% | 1 per street |
| `turn_std_v1` | turn root | 33, 75 / 33, 75; river 50, 100 | 60 | 150% | 20% | 2 per street |
| `turn_min_v1` | turn root | 75 / 75; river 75 | a | 150% | 20% | 1 per street |
| `river_std_v1` | river root | 33, 75, 150 / 33, 75, 150 | 60 | 200% | 20% | 3 |
| `river_min_v1` | river root | 75 / 75 | a | 150% | 20% | 1 |

The `_min` templates are the crash/timeout retry and the `TreeTooLarge` fallback. Sizes are hypotheses **(spec decision)**; bench may change them, in which case the version suffix increments and old cache entries stay valid under their own id.

### 10.2 Effective tree for the current street
- Convert each template size at each node to chips with round-half-up. An already-taken action on the current street whose chips equal a template size (after rounding) is in-tree.
- Off-tree taken action: insert its exact chip amount as an additional size for that actor at that node; then prune every sibling line of every taken action (`remove_lines`) so the solve covers only the observed line plus the future subtree **(spec decision)**. The effective tree id is `sha256(template_id, inserted sizes, history)`; when nothing was inserted, `tree_id = template_id` and the history is applied inside the root solve.
- Pot and effective stack are the actual chips from `Derived`; nothing is rounded.
- OOP/IP: the player who acts first postflop is OOP. `hero_is_oop` is sent so the worker returns hero's EV.

### 10.3 Worker adapter (library mapping, subject to V4)
- `Solve` -> `CardConfig{range: [oop, ip], flop, turn, river}` + `TreeConfig{initial_state, starting_pot, effective_stack, rake_rate, rake_cap, bet sizes, add_allin_threshold, force_allin_threshold, merging_threshold}`; `remove_lines` for pruning; `memory_usage()` before `allocate_memory(compress)`: f32 when <= 4 GB, 16-bit when <= 8 GB, else `Result{Error{"TreeTooLarge"}}` **(spec decision)**; `set_bunching_effect` when `bunching` is present; staged locks applied via `lock_current_strategy` before the first iteration; `solve_step` loop with deadline and cancel checks; `finalize`; `apply_history`; `cache_normalized_weights`; `strategy` and `expected_values_detail` for the decision node; `street_strategy` by walking every decision node of the current street.
- EV normalization: the adapter converts library EV to the section 2 convention in one function (`normalize_ev`) whose behaviour is pinned by contract test `ev_convention_fold_is_zero`.
- Cancel: polled between iterations; `Cancel` -> `Result{Cancelled}` within one iteration.
- Memory: allocations stay warm; the game object is dropped after `Result` so the next job starts from a clean arena.

### 10.4 Cache and pre-solver (`cache`)
- Key: `(canonical board, suit permutation applied to ranges, sha256(oop quantized to u16/10000, ip quantized), pot_chips, eff_stack_chips, bb_chips, rake_rate, rake_cap_chips, tree_id, target_bp, bunching flag + folded ranges hash)` **(spec decision on quantization and bb_chips in key)**. Exact match only.
- Layout: `%LOCALAPPDATA%\PokerAI\cache\v1\<key[0..2]>\<key>.bin`, `bincode` + `zstd`, header with payload sha256 and `proto_version`. Corrupt or version-mismatched entry: delete and re-solve. River solves are not stored.
- Entry: `Result` fields plus the key fields and `reached_bp`. A `BestSoFar` entry is stored and served as `Approximate{DeadlineBestSoFar}`; the pre-solver re-queues it.
- Pre-solver policy **(spec decision on ordering and idle rule)**:
  - Runs when no hand is in progress and no request has arrived for 30 s; pauses instantly on any live request (cancel) and on `presolver_pause()`.
  - Scenarios: depths {100, 200} bb at the last-used `GameConfig` blinds and rake; pot types {SRP raiser vs BB caller, 3-bet pot 3-bettor vs caller}; position pairs from the preflop store's standard sizes; ranges from the store; template `flop_fast_v1`, `target_bp` 50, no deadline.
  - Order: pot type (SRP first), then position pair by scenario frequency list `[BTN-BB, CO-BB, HJ-BB, UTG-BB, SB-BB, BTN-SB, CO-BTN, HJ-BTN, ...]`, then flop class by orbit size descending (24 before 12 before 4), then rank order.
  - Resumable: an entry's presence in the cache is the completion mark; a `queue.json` records the cursor. Estimated volume: about 48 scenarios × 1,755 flops = 84k solves; at 10 s each about 10 days of idle time, delivered in priority order.

### 10.5 Deadline and best-so-far
- Street budgets from section 7 are passed as `deadline_ms` minus elapsed fast-phase time.
- At the deadline the worker finalizes and returns `BestSoFar` with `reached_chips`; the engine labels `DeadlineBestSoFar{reached_bp}` and still returns per-action EVs.
- Retry policy is in section 12.

---

## 11. Exploit slice (OPTIONAL in phase 1; built only after sections 5-10 are validated; otherwise phase 2)

- Inputs: per-seat `SeatTag` in {unknown, nit, TAG, loose-passive, calling-station, LAG, maniac}; `QuickFact` in {never_folds_river, rarely_bluffs, limps_a_lot, over_3bets, folds_to_pressure}. Set at session start or during a hand via `set_seat_tag`.
- Preflop: tags scale the opponent's reach weights per action class and position using the R4 §3.1 multiplier table; `limps_a_lot` adds 20 pp open-limp probability; `over_3bets` sets the 3-bet factor to 1.75. Applied inside replay before postflop solving. Defaults live in `exploit_defaults.toml` with `calibrated = false`.
- River locks (HU river decisions only): from the reference (unlocked) solve's `street_strategy`, compute the opponent's locked strategy at their river nodes: `never_folds_river` -> fold target 5% raw; `rarely_bluffs` -> `kB = 0.25` on low-showdown-value betting combos, mass moved to check; `folds_to_pressure` -> `+15 pp` folds vs the observed size class; tag-only defaults from R4 §3.3 river column. Credibility shrinkage `c = 0.35` (tag only) or `0.60` (fact): `target = F0 + c * (raw - F0)`. Composition rules preserve value actions.
- Second solve: `Lock` (staged) + `Solve` with the same effective tree and deadline <= 2 s. Hero's exploit policy `sigma_x = (1 - alpha) * sigma_gto + alpha * sigma_locked_response` with `alpha = 0.25` when a tag or fact applies, `0` for unknown/TAG without facts. No best-response audit in phase 1 (phase 2).
- Display: GTO headline action vs exploit headline action with `EV_delta = EV_locked(exploit action) - EV_locked(gto action)`, both read from the locked solve (same opponent model). The panel is labelled "uncalibrated defaults" and is visually separate from the baseline.
- `ExploitAdvice { alpha, model_summary, gto_action, exploit_action, ev_delta_bb, locked_nodes: u16 }`.

---

## 12. Error handling

| Failure | Handling |
|---|---|
| Illegal action entered | Rejected by `apply_action`; UI disables illegal keys from `Derived.legal`; undo restores the previous snapshot |
| Stack/pot inconsistency (typed chips exceed stack, board card duplicates a known card) | Rejected with a message; state unchanged |
| Worker crash, protocol error, or no `Progress` for 5 s during a solve **(spec decision: heartbeat)** | Kill, respawn, wait for `Ready`; retry once with the `_min` template of the street with the remaining budget; then `Unsupported{EngineError{message}}` with the message shown |
| Worker `TreeTooLarge` | Retry once with the `_min` template; then `Unsupported{TreeTooLarge}` |
| Deadline reached | `BestSoFar` -> `Approximate{DeadlineBestSoFar}` |
| Cancel not acknowledged within 500 ms | Kill and respawn the worker |
| Missing depth / straddle tree / rake profile | Nearest with the corresponding `ApproxReason`; never silent |
| Missing preflop node | `Unsupported{MissingPreflopNode}` with the key string in assumptions |
| Corrupt or version-mismatched cache entry | Ignore, delete, re-solve; counted in the decision log |
| Preflop bundle missing or failing its manifest sha256 | Bundle not loaded; startup banner names the bundle; lookups fall through to remaining bundles |
| Stale result (`state_version` mismatch) | Discarded by the UI and by `engine-main` |
| `proto_version` mismatch with the worker | Every request answers `EngineError("worker/proto version mismatch")` until rebuilt |

---

## 13. Testing strategy

### 13.1 Unit (Rust, `cargo test`)
| Crate | Test | Assertion |
|---|---|---|
| core-model | `state_machine_pokerkit_fixtures` | Replays `fixtures/hands/*.json` (200 hands generated by `tools/gen_fixtures.py` with PokerKit: UTG straddle, side pots, 2 and 3 all-ins, short all-ins) and matches pot, side pots, stacks and legal actions at every step |
| core-model | `straddle_action_order_utg` | Preflop order HJ, CO, BTN, SB, BB, UTG; postflop order unchanged |
| core-model | `min_raise_and_short_allin_no_reopen` | Min-raise amounts; a short all-in does not reopen action for a player who already acted |
| core-model | `side_pot_three_allins` (V14) | Three all-ins with different stacks produce main + two side pots with correct eligibility |
| core-model | `card_parser_roundtrip` | "AsKd" and board strings parse and print identically; invalid strings rejected |
| core-ranges | `range_roundtrip_pio_strings` | Parse -> vector -> string -> vector is identical for weighted strings incl. "AKs:0.5, 77+, A5o" |
| core-ranges | `class_expansion_multiplicity` | Pair 6, suited 4, offsuit 12; total mass of "random" = 1326 |
| core-ranges | `blockers_zero_board_combos` | Combos containing board or hero cards have weight 0 |
| core-iso | `iso_class_count_1755` | Exactly 1,755 canonical flops |
| core-iso | `iso_orbit_sizes` | Orbits are in {4, 12, 24}; 52 + 3,744 + 18,304 = 22,100 |
| core-iso | `iso_permutation_inverse_roundtrip` | permute(inverse(x)) = x for boards and ranges; turn/river extension canonical form is stable |
| core-eval | `eval_vs_phevaluator_full_5card` | All 2,598,960 5-card hands rank-order-equivalent to `fixtures/eval/phevaluator_5card.bin` |
| core-eval | `eval_vs_phevaluator_random_7card` | 10,000,000 fixed-seed 7-card samples match the oracle table |
| core-eval | `equity_exact_vs_mc` | MC(50k) within 0.5 pp of exact on 20 fixed spots |
| core-preflop | `pokerdata_adapter_units_sb_to_bb` | EV conversion; zero-weight actions keep EV |
| core-preflop | `pokerdata_action_path_lookup` | Provider action path -> `PreflopNodeKey` -> node, for RFI, vs-3bet, vs-4bet, squeeze |
| core-preflop | `depth_bucket_labels` | 97bb -> 100 Exact; 120bb -> 100 `DepthBucket`; 125bb -> 150 (tie goes deeper) |
| core-preflop | `straddle_mapping_labels` | Position shift and `StraddleMapped` when no straddle bundle |
| core-preflop | `missing_node_unsupported` | Limp line absent -> `Unsupported{MissingPreflopNode}` |
| core-replay | `replay_bayes_single_action` | weight *= P(action) for one raise; hero range not collapsed |
| core-replay | `replay_off_tree_pseudo_harmonic` | 73 into 100 with sizes 50/100: `f_A = 0.468`, `f_B = 0.532`, `BetTranslation{deviation 0.23}` |
| core-replay | `replay_multiway_prior_street_unconditioned` | Flop was 3-way -> turn replay carries `UnconditionedPriorStreet{Flop}` |
| cache | `cache_key_exact_match_only` | Any single field change misses; suit-permuted board with permuted ranges hits |
| cache | `cache_corrupt_entry_deleted` | Flipped byte -> miss, file removed, re-solve requested |
| cache | `cache_roundtrip` | bincode+zstd round trip of a `Result` |

### 13.2 Solver contract (`solver-worker` integration tests, spawn the binary)
| Test | Spot | Assertion |
|---|---|---|
| `river_polarized_vs_bluffcatcher_analytic` | Board Qs Jd 7h 3c 2d; OOP (caller) AA weight 1; IP (bettor) QQ weight 1 + 54o weight 0.1875 (equal mass); pot 100, stacks 100; OOP no bets, IP pot bet only, no rake | IP bets 100% of QQ, 50 ± 3 pp of 54o; OOP calls 50 ± 3 pp; IP range EV 75 ± 1 chip, OOP 25 ± 1; exploitability <= 0.1% pot |
| `ev_convention_fold_is_zero` | Same spot | `normalize_ev` yields fold = 0, call EV = analytic value for AA |
| `symmetric_ranges_ev_symmetry` | Identical ranges both players, river, no rake, `river_std_v1` | `EV_OOP + EV_IP = pot ± 0.5%`; check-check showdown equity 50 ± 0.1% |
| `wasm_postflop_benchmark_spot` | Published wasm-postflop README comparison spot | Root aggregate bet frequency within ±1.0 pp of the published value; exploitability <= 0.5% pot |
| `suit_permutation_metamorphic` | Any flop spot | Permuting suits of board and ranges gives identical strategies after inverse mapping (max abs diff <= 1e-4) |
| `rake_cap_applied` | River spot with 5%/cap where cap binds | Terminal payoff differs from unraked by exactly the cap |
| `combo_ordering_named_check` | Range "AsKs" only | The returned probs are non-zero only at the AsKs index |
| `cancel_within_iteration` | Flop spot | `Cancel` -> `Result{Cancelled}` within 200 ms of the next iteration boundary |
| `deadline_returns_best_so_far` | Flop spot, `deadline_ms = 1000` | `BestSoFar` with `elapsed_ms <= 1100` and a valid strategy |
| `exact_size_insertion_and_prune` | Villain flop bet 73 into 100 | Tree contains 73c at that node; sibling root sizes pruned; decision node reached |
| `lock_applied_before_solve` (exploit) | River spot with a locked opponent fold frequency | Locked node's probs unchanged after solving; hero response differs from unlocked |

### 13.3 Engine goldens (`engine/tests/golden/*.json`, expected outputs committed)
`coverage_classification_golden` (HU flop; 3-way flop; third player all-in on flop; two preflop folds then HU flop; opponent all-in; hero all-in), `bet_translation_golden` (below min size, between sizes, above max with and without all-in), `tree_builder_golden` (in-tree size detection with chip rounding; insertion; `tree_id` stability), `replay_weights_golden` (three-seat preflop history; expected 1326 vectors), `recommendation_assembly_golden` (headline choice, tie break, reason accumulation).

### 13.4 UI
- Vitest + `@tauri-apps/api/mocks` `mockIPC`: `entry_flow_full_hand_keystrokes`, `undo_restores_snapshot_and_version`, `stale_result_discarded`, `illegal_keys_disabled`, `coverage_label_and_assumptions_render`, `progressive_fast_then_final`.
- One end-to-end WebDriver test on Windows (`tauri-driver`, `@wdio/tauri-service`): `e2e_full_hand_srp_flop_recommendation` enters a full SRP hand to a flop decision and asserts a `Final` with a coverage label within 15 s.

### 13.5 Bench gate (`bench`)
Spots: `bench/spots/{river_std,turn_std,flop_fast,flop_min}.json` (5 spots each, fixed ranges from the store at 100bb and 200bb). Report `docs/bench/<date>-i7-13700K.md` with p50/p95 wall time to `target_bp` 50, peak worker RSS, thread count. Gate: river p95 <= 2 s, turn p95 <= 6 s, flop_fast p95 <= 10 s, cache hit p95 <= 0.5 s, e2e (request to `Final`, cold cache) p95 <= 15 s.

---

## 14. Phasing and critical-path validations

### 14.1 Week 1
Workspace and `proto`; `core-model` with PokerKit fixtures; `core-ranges`; `core-iso`; `core-eval` with oracle tables; V1 (worker builds); `bench` river/turn/flop_fast (V2/V3); worker API surface incl. locks, per-action EV, cancel, progress (V4); `core-preflop` with the chart fallback and the PokerData adapter behind V9; `core-replay`; engine river/turn path with deadline and retry; minimal keyboard UI showing `Fast` and `Final`.

### 14.2 Week 2
Flop path; `cache` and pre-solver; bet translation; coverage labels and assumptions panel; undo; decision log; all tests green; bench gate; E2E test. Exploit slice only if all of the above is done and validated.

### 14.3 Phase 2 backlog
Full exploit layer (turn locks, best-response audit, observation updates); more scenarios and depths in the flop library; GPU batch pre-solving if a licensed backend appears; neural surrogate distilled from our own solves; multiway models; LAN/phone UI with the AGPL boundary review.

### 14.4 Critical-path validations
| Id | Question | Method | Pass criterion | Blocks |
|---|---|---|---|---|
| V1 | postflop-solver builds on Windows 11 at the pinned commit with stable Rust; AVX2 path runs | `cargo build --release -p solver-worker`; run `river_polarized_vs_bluffcatcher_analytic` | Build and test pass | everything postflop |
| V2 | Turn solve time for `turn_std_v1` | `bench` turn spots | p95 <= 6 s | turn budget, template sizes |
| V3 | Flop solve time for `flop_fast_v1` | `bench` flop spots | p95 <= 10 s; memory <= 4 GB f32 | flop budget, pre-solver rate |
| V4 | Worker API: lock before solve, per-action EV (`expected_values_detail`), cancel between iterations (`solve_step`), progress | contract tests `lock_applied_before_solve`, `cancel_within_iteration`, `deadline_returns_best_so_far` | All pass | deadline design, exploit slice |
| V9 | PokerData: cash solver provenance, branch and EV coverage, offline/live-use rights, straddle/deeper availability | Request metadata and rights in writing; run `pokerdata_action_path_lookup` on the Starter sample | Rights confirmed and sample checks pass; else chart fallback only | preflop EV, pre-solver ranges |
| V14 | Side-pot correctness of `core-model` | `side_pot_three_allins`, `state_machine_pokerkit_fixtures` | 200/200 fixtures match | coverage classifier, pot accounting |
| V21 | End-to-end p95 including cache misses and worker restarts | `bench` e2e mode driving the engine with 50 recorded hands from `decisions.jsonl` | p95 <= 15 s | release |

---

## 15. Risks and mitigations (review top 5)

| Rank | Risk | Mitigation in this design |
|---|---|---|
| 1 | Coverage mismatch: a six-max label masks a HU backend; multiway, all-in-third-player and unsupported histories may be a large share of live decisions | Pot-eligible counting (section 6); numeric EV only for supported roots; decision log measures the coverage matrix from real sessions in the first days; multiway shows equity plus a clearly separated experimental block |
| 2 | No validated preflop strategy + EV supply; bad priors contaminate every solve | V9 on the critical path with a Starter-sample check; chart fallback labelled `ChartRounded`; missing nodes are `Unsupported`, never inferred |
| 3 | End-to-end time misses (cache misses, deep trees, human entry) | Persistent warm worker; `solve_step` deadline with best-so-far; `_min` retry templates; bench gate per street and e2e (V2/V3/V21); one solve in flight; measured cancel latency |
| 4 | Plausible-looking wrong EV (bet translation, combo ordering, EV reference, lifecycle) | Analytic river oracle, named-combo check, `normalize_ev` contract, suit-permutation metamorphic test, exact-size insertion instead of rounding pot/stacks, feature freeze until section 13 is green |
| 5 | Operational/licensing fit fails late (table reachability, venue rules, AGPL vs purchased data terms) | Worker isolated in its own AGPL process with provenance kept; data rights negotiated separately (V9); no distribution or network service in phase 1; venue permission recorded as open question 1 |

---

## 16. Open questions

1. Venue permission and physical setup: whether the intended room permits a device consultation during a hand, and where the desktop sits relative to the table. Not a design decision; it determines whether phase 1 is used live or in permitted study/replay.
2. Template sizes and thread count: `flop_fast_v1`, `turn_std_v1`, `river_std_v1` and the 16-thread default are placeholders until V2/V3 report; the ids are versioned so the change is mechanical.
3. PokerData branch coverage for live lines (limps, 4-6bb opens, squeezes): unknown until the V9 sample; determines how often preflop returns `MissingPreflopNode` versus `BetTranslation`.
