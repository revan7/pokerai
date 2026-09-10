# PokerAI assistant: design outline (orchestrator decisions, 2026-09-10)

Source of truth for the spec writer. Facts come from docs/research/SYNTHESIS.md, corrected by docs/research/REVIEW-of-research.md.

## 0. User decisions (final)
- Approach A: solver-core assistant. Format: 6-max NLHE cash. Input: manual entry at a live table. Windows 11 desktop. Private use only (AGPL imposes no obligations; keep the solver as a separate process anyway).
- Preflop data: PokerData, verified 2026-09-10 (docs/research/R7-pokerdata-verification.md): MonkerSolver provenance; depths 20-200bb (8 packs), 5% rake with 0.5bb cap; full preflop tree incl. squeezes and multiway callers; one open size and one 3-bet size; data is 169-class action frequencies plus per-action EV in small-blind units (not per-combo). No 250bb, no straddle, no ante packs. No written terms; the user emails the vendor to confirm offline caching and personal live use before buying. Purchase is the user's own action; recommended form: one-time packs ($50 per depth, $250 for all eight) rather than the API subscription. Fallback = transcribed free charts (labelled "rounded, no EV"). Straddle and 250bb trees are phase 2 (self-solve with HRC or a custom pack).
- Game config is a per-session setting (blinds, straddle, rake or time charge, stacks). Week-1 validation targets 100bb and 200bb, 5% pot rake with cap, UTG straddle supported.
- Budget: strong baseline in 1-2 weeks, then iterate.

## 1. Goals and non-goals
Goals: for each hero decision point, return legal actions with frequency + incremental EV (bb, fold = 0) + a coverage label (exact / approximate / unsupported) + assumptions, within a 15 s hard deadline (excluding human entry), with progressive display (fast equity estimate first, solver result when ready). GTO baseline first; exploit slice only after the baseline is validated.

Non-goals (phase 1): screen capture; tournaments/ICM; multiway (3+ pot-eligible) solved EV; GPU solving; neural models; phone/LAN UI; distribution to others.

## 2. Process architecture
- `pokerai-ui`: Tauri 2 + Vite + React + TypeScript. Keyboard-first entry. Talks to the engine via Tauri commands plus `Channel<T>` progress events.
- `engine` (Rust, inside the Tauri process, runs on dedicated std threads, never on Tokio workers): orchestrates one decision request.
- `solver-worker` (separate Rust binary, the AGPL boundary): wraps b-inary/postflop-solver pinned to a commit (vendored fork under `third_party/` with LICENSE retained). Protocol: JSON lines over stdio. Ops: `solve` (board, ranges as 1326-weight vectors, pot, effective stacks, rake rate/cap, bet-tree template, target exploitability, deadline ms), `progress`, `cancel`, `result` (root strategy per combo, per-action EV per combo, reached exploitability, elapsed), `lock` (per-node strategy lock, for the exploit slice), `shutdown`. The engine restarts the worker on crash and answers "unsupported (engine error)".
- `tools/` (Python 3.12, dev only): oracles for tests (phevaluator, PokerKit), chart-transcription ingestion scripts, benchmark report scripts. Python is never a runtime dependency of the app.

## 3. Rust workspace crates (one clear purpose each)
- `core-model`: cards/ranks/suits, "AsKd" parser, board, seats/positions (6-max with dealer button), game config (blinds, straddle posting and action order, rake rate/cap or time charge, stacks), action model, 6-max hand-state machine (streets, pot, side pots, all-ins, legal actions, min-raise rules). Serializable.
- `core-ranges`: Pio-style range string parser with weights ("AKs:0.5, 77+") to and from a 1326-combo weight vector; card removal/blocking; merge/scale helpers.
- `core-iso`: flop canonicalization to the 1,755 suit-isomorphic classes (with correct orbit sizes 4/12/24 summing to 22,100), suit permutation map and inverse, turn/river extension; used only for exact-match cache keys. No "nearest flop" substitution, ever.
- `core-eval`: 7-card evaluator (b-inary holdem-hand-evaluator, MIT; rs-poker fallback) plus weighted range-vs-range equity (`pokers`, MIT), Monte Carlo and exact; validated against phevaluator in tests.
- `core-preflop`: strategy store. Trait `PreflopSource` with two adapters: `PokerDataJson` (169-class per-action frequencies plus per-action EV in small blinds, converted to bb; expanded to 1326 combos with card removal at query time) and `ChartTranscription` (169-class frequencies, no EV, labelled "approximate"). Versioned bundle on disk. Node lookup by (depth bucket, rake profile, position, action sequence). Depth bucketing: nearest available depth, labelled when not exact. Straddle handling: explicit posting and action-order model; if the bundle lacks a straddle tree, map to the closest non-straddle node with the label "approximate (straddle)". Missing node returns `Unsupported`.
- `core-replay`: range replay. For every seat, condition its starting range on every observed action using the preflop store (Bayes: weight *= P(action | combo)); postflop, condition using the stored or solved strategy of the prior-street node (with bet translation for off-tree sizes). Hero keeps a strategic range (not collapsed to the holding); hero's actual combo is used only for the headline action.
- `engine`: coverage classifier (count unfolded pot-eligible players; a third player all-in makes it non-HU), tree-template builder (bet sizes as % pot mapped to game config; observed sizes of already-taken actions on the current street are inserted exactly; only future actions are abstracted), bet translation for prior-street off-tree sizes (pseudo-harmonic mapping; labelled when deviation exceeds a threshold), cache lookup, worker dispatch with deadline and best-so-far fallback, result assembly into `Recommendation`.
- `cache`: on-disk store of solved flop/turn root strategies plus reach ranges keyed by (canonical board, hash of both ranges, pot, effective stack, rake, tree template id, accuracy). Exact match only. Background pre-solver job that fills the most common hero scenarios (single-raised and 3-bet pots by position pair at 100bb and 200bb) across all 1,755 flops, prioritized by scenario frequency; runs when the app is idle; resumable.
- `proto`: shared serde types: `HandState`, `GameConfig`, `Recommendation`, `Coverage`, worker messages. Single source of truth for UI, engine, and worker.
- `bench`: benchmark harness: p50/p95 solve time and peak RSS per street and tree template on this machine; produces a markdown report; used as an acceptance gate.

## 4. Data flow for one decision
1. Session start: the user sets the game config (blinds, straddle, rake, seats and stacks). Optionally tags seats (exploit slice).
2. Per hand: button position, hero seat and hole cards, then each action as it happens (one keystroke per action; bet sizes typed in chips), board cards per street. Undo stack for corrections.
3. At each hero decision point the UI calls `recommend(HandState)`. Engine: validate, classify coverage, replay ranges; if preflop read the store; else if heads-up build the tree, look up the cache, else solve via the worker with a deadline; assemble `Recommendation`.
4. The UI shows immediately: legal actions and range-vs-range equity, then updates with solver frequencies and EV when ready. Headline = highest-EV action for hero's combo (with the frequency mix shown). Always show the coverage label and assumptions (ranges used, tree, accuracy reached, time).

## 5. Coverage rules (numeric EV only where supported)
- Preflop, any number of players: from the store; missing node returns Unsupported (show equity only).
- Postflop, exactly 2 pot-eligible unfolded players (folded players may be bunched): Exact (solved) or Approximate (bet-translated, depth-bucketed, or deadline-hit best-so-far, each labelled with the reason).
- Postflop, 3+ pot-eligible players: Unsupported for EV. Show weighted range-vs-range equity for all live players and an explicitly "experimental, not solved" heuristic (HU solve vs the strongest opposing range), visually separated from solved outputs.

## 6. Latency budget (15 s hard, after entry)
River: at most 2 s. Turn: at most 6 s. Flop: cache hit at most 0.5 s, else reduced tree (1-2 sizes, high all-in threshold) at most 10 s. Deadline hit returns best-so-far with the reached exploitability, labelled Approximate. Progressive display from about 0.3 s (equity). The worker keeps allocations warm between requests. All budgets are hypotheses until `bench` measures them on the i7-13700K (validations V2/V3).

## 7. Exploit slice (phase 1 only if the baseline is done and validated; otherwise phase 2)
Seat tags: unknown / nit / TAG / loose-passive / calling-station / LAG / maniac; quick facts: "never folds river", "rarely bluffs", "limps a lot", "over-3-bets", "folds to pressure". Tags scale the opponent's preflop range weights (per-position multipliers); facts lock river strategies in the worker (fold-frequency and bluff-frequency locks). Exploit policy = mixture with cap alpha = 0.25 (0 for unknown/TAG without facts). Show the GTO action vs the exploit action with the EV delta, both evaluated against the same opponent model. All numeric defaults are uncalibrated and must be labelled as such.

## 8. Error handling
- UI validation: illegal actions disabled; stack/pot consistency checks; undo.
- Worker crash or timeout: restart, one retry with a smaller tree, else Unsupported (engine error) with a visible reason.
- Missing data (depth, straddle tree, node): nearest with label, never silent.
- Corrupt cache entry: ignore, delete, re-solve.

## 9. Testing strategy
- Unit (Rust): parsers, state machine (fixtures generated with PokerKit including straddles, side pots, multiple all-ins), evaluator vs phevaluator (random 7-card samples plus full 5-card enumeration), iso classes (count 1,755; orbit sizes sum to 22,100), range parser round-trips, cache keys.
- Solver contract: toy river spots with analytic solutions (polarized range vs bluff-catcher), symmetric-range spots (EV symmetry), and the published wasm-postflop benchmark spot (compare root strategy within tolerance).
- Engine: golden tests for coverage classification, bet translation, replay weights.
- UI: Vitest plus Tauri mockIPC for the entry flow; one end-to-end WebDriver test on Windows for a full hand.
- Bench: the `bench` report per street and template is a release gate (p95 within budget).

## 10. Phasing (baseline 1-2 weeks)
Week 1: workspace, proto, core-model, core-ranges, core-iso, core-eval; solver-worker builds on Windows at the pinned commit (V1); bench river/turn/reduced-flop (V2/V3); worker API surface for locks and per-action EV (V4); core-preflop with the PokerData adapter (pending rights, V9) and the chart fallback; core-replay; engine river/turn path; minimal keyboard UI.

Week 2: flop path, cache, background pre-solver; bet translation; coverage labels and assumptions panel; UI keyboard flow and undo; tests green; bench gate; exploit slice only if all of the above is done.

Phase 2 backlog: full exploit layer (turn locks, best-response audit, observation updates); full 1,755-flop library across more scenarios and depths; GPU batch pre-solving if a licensed backend appears; neural surrogate distilled from our own solver outputs; multiway models; LAN/phone UI (axum) with an explicit AGPL boundary review.

## 11. Critical-path validations (from the review)
V1 Windows build of postflop-solver; V2/V3 turn/flop timings on the i7; V4 worker API (locks, per-action EV, cancel, progress); V9 PokerData rights, provenance, coverage; V14 side-pot correctness of the rules engine; V21 end-to-end p95 including cache misses.
