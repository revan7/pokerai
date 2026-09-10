# SYNTHESIS — decision brief for architecture proposals (2026-09-10)

Inputs: R1 solvers, R2 preflop, R3 libraries, R4 exploit layer, R5 app stack, R6 neural feasibility. Target: local Win11 6-max NLHE cash assistant, manual entry, frequencies + EV in 5–15 s; RTX 5080 16 GB, i7-13700K (16c/24t), 64 GB; strong baseline in 1–2 weeks. All numbers are as reported by the cited report; none were measured on the target machine.

---

## 1. Hard facts that constrain the design

### 1.1 Postflop solver (R1, corroborated by R6)
- **Only viable open-source engine: b-inary/postflop-solver** — Rust, **AGPL-3.0**, development suspended Oct 2023, last push 2024-07-09, not on crates.io (git dependency, pin a commit; author warned of breaking changes without version bumps). DCFR, no card abstraction, suit isomorphism, rayon, f32 or 16-bit compressed arenas, `memory_usage()` pre-sizing, bincode/zstd save/load.
- Features: HU only; flop/turn/river start; rake (`rake_rate`, `rake_cap`); node locking (`lock_current_strategy`); `solve(..., target_exploitability)`; bunching for ≤4 folded players (only implementation that has it). No ICM, no multiway, no Python bindings, no GPU.
- **Fastest measured CPU engine** on an apples-to-apples spot: beats PioSOLVER Free and GTO+ (see §6). TexasSolver (C++, AGPL, frozen; author moved to closed TexasSolverGPU) is ~5× slower, ~2.2× RAM, no rake, no locks → CLI fallback only.
- **GTOpen** (Rust + CUDA, created 2026-06-13, daily commits, single author): HU postflop, rake, locks, CUDA ≈10× over its own 16-thread CPU path, preflop lab 2–9 players (approximate continuation). **No LICENSE file (`license: null`) → not legally embeddable.** No external validation vs Pio.
- **Closed / unusable**: TexasSolverGPU (proprietary EULA, GUI only), DEEPFOLD (all rights reserved), OpenSpiel (Apache but tabular CFR, orders of magnitude too slow for NLHE), DCFR-SOLVER (MIT but one-day code drop, unbenchmarked).
- **Not possible open-source**: multiway (3+) postflop solving — every range solver is HU-only; ICM — none (irrelevant for cash). Multiway exists only in bot frameworks (robopoker, noregrets) and commercial tools (HRC v4, GTO Wizard 3-way).
- **Latency reality**: river < 1 s, turn expected single-digit s (extrapolated, unpublished); full flop trees take minutes → **a precomputed flop library is required**; live solves are turn/river plus "FAST"-style flop trees (1–2 sizes, 100% all-in threshold).
- **AGPL for private use** (R1 §5.4): running/modifying privately is not "conveying"; §13 network clause needs remote users. Only constrains future distribution (then whole combined work → AGPL + source) — see §4.

### 1.2 Preflop data (R2)
- **No complete, freely licensed, machine-readable 6-max cash strategy + per-action EV set exists** for 100/150/200/250bb + straddles. A free 250bb collection was not found at all.
- Best free references (no EV, rights to embed not verified): RangeConverter 200bb PDF (labelled 500z, frequencies rounded to 50%); PokerCoaching 100bb PDF (simplified, no mixing); FreeBetRange (HRC-derived, per-range Pio-format copy on free plan, weights/whole-config export paid); GTOBase viewer (Simple Preflop Holdem provenance, 40–200bb, no bulk export).
- Paid candidates: **PokerData** — JSON API, 48 bundles at 20–200bb, 5% rake cap 0.5bb, bundles ~7/12/19 MB (100/150/200bb), per-action EV in **SB units**; Starter = 100-request/50 MB lifetime trial; Pro **$100/month**; cash-solver provenance, offline/live-use rights, 250bb/straddles **not verified**. **GTOsims** — **$519** for one 100bb NL200 pack (Simple Preflop Holdem, 5% rake cap 1.37bb, `.bin` + Pio + PNG); terms restrict reuse; deeper packs not verified. **GTO Wizard**: terms prohibit live use; no bulk export → calibration only. **HRC** v3+ exports all strategies + EVs as JSON and models straddles/rake (price and target-machine runtime not stated).
- **Self-solving preflop**: feasible only as a simplified research game (equity/realization terminals). DCFR-SOLVER (MIT) reports 2.2M infosets, 100M iterations in 14 min (hardware unstated); GTOpen preflop lab 6-seat 2.3 s/175 iters on RTX 3090; NoRegrets 200M iters in 79 min on 16-core Apple Silicon, 2B iters in 5h20 with 36–45 GB RSS and still large multiway LBR losses. Multiplayer CFR has no Nash convergence guarantee. R2 verdict: label any self-solve "approximate preflop model", not GTO.
- **Straddles must be modelled as actual posting/action order**, not stack relabelling ($2/$5/$10, $1,000 = 200 bb but 100 straddle units with 1.7-unit dead money). 4 depths × 3 posting structures × 2 rake profiles = 24 configs before size menus → do not target the Cartesian product in week 1.

### 1.3 Libraries (R3)
- **Rust core**: evaluator b-inary/holdem-hand-evaluator (MIT, git only, ~1.2 G eval/s claimed, 212 kB tables) or rs-poker (Apache-2.0, 5.1.0, 50 M+/s, on crates.io); range-vs-range equity **pokers** 0.10.0 (MIT, OMPEval lineage, MC + exact, ≤9 ranges, weights via `@50`); rules engine rs-poker `arena` (must set `max_raises_per_round = None`; side-pot struct not confirmed) or chironaut (MIT/Apache, explicit side pots, young); isomorphism `hand_indexer` crate (MIT) over Waugh's C (BSD-style; needs MSVC `cc`).
- **In-house (small)**: Pio `hand:weight` range parser + 1326-float vector (~150 lines; no permissive one exists); flop canonicalization to 1,755 classes with weights (~50 lines).
- **Python 3.12 (tests/oracles/analysis only)**: PokerKit 0.7.5 (MIT, full rules engine incl. straddles/side pots/PHH, pure-Python evaluator ~1 M/s), phevaluator 0.6.0 (Apache-2.0, 56 M/s, cp312 Windows wheel) as evaluator oracle, eval7 optional.
- Avoid: poker-eval/clubs (GPL), pyrust-poker (no Windows wheel), treys (stale). R3 explicitly says do **not** vendor postflop-solver code (AGPL) — see tension §2.1.

### 1.4 App stack (R5)
- **Primary: Tauri 2 + Vite/React/TS + Rust workspace** (`poker-core`, `poker-proto`, later `poker-server` axum). Solver is a crate in the same workspace (no FFI); `#[tauri::command]` + `Channel<T>` for progress; run solves on `std::thread`/rayon, never Tokio workers; ~10 MB NSIS installer, WebView2 already in Win11. Costs: WebView2 browser accelerators (F5/Ctrl+P/F12) need `preventDefault` or wry `with_browser_accelerator_keys(false)`; LLMs emit Tauri v1 code (pin v2, cheat-sheet in CLAUDE.md).
- **Fallback: Electron** + same React/TS + `poker-core` as standalone binary over stdio JSON lines (`utilityProcess`). Most mainstream, Playwright E2E, 100–250 MB bundle; zero native-addon work.
- GPU path (any stack): cudarc 0.19.9 or wgpu behind a Cargo feature; if CUDA, run solver as sidecar/child process so a device fault cannot kill the UI.
- Phone/LAN later = add axum crate serving the same Vite build; not a rewrite.

### 1.5 Exploit layer (R4)
- **Phase-1 scope**: persistent opponent identity per seat; 7 tags (unknown/nit/TAG/loose-passive/calling-station/LAG/maniac) as weak priors; scoped manual facts ("never folds river", "rarely bluffs", "limps a lot", "over-3-bets", "folds to pressure"); preflop multipliers on a per-combo baseline π0 + Bayesian replay of observed actions to get ranges; postflop ΔF/kB/kR transforms; frozen unlocked reference; **HU river first, tractable turn second**; baseline fallback elsewhere; matched EV (both policies vs the same model).
- **Default risk bound**: `alpha_cap = 0.25` exploit-policy mixture (0 for unknown/TAG with no fact); local worst-case budget `epsilon = min(0.10 bb, 0.005 × pot)`; `alpha = min(alpha_cap, epsilon / D)`, `D = max(0, W(σ0) − W(σX))` from an information-set-respecting best-response audit; credibility `c` = 0 / 0.35 (tag only) / 0.60 (observed) / 0.80 (strong history); gate: recommend exploit only if gain > `max(0.02 bb, 3 × eval_error)`.
- **Engine contract required**: obtain baseline, fix per-infoset policies, evaluate arbitrary complete policies, best-response audit, cancel. postflop-solver has locks; TexasSolver has none → excluded for R4.
- Latency allocation: input ≤1 s, reference ≤4 s, response ≤6 s, audit/render ≤3 s, reserve 1 s. All numeric tag defaults are **unverified engineering guesses**.

### 1.6 Neural verdict and GPU role (R6)
- **Verdict: solver-based phase 1; no neural training as a delivery dependency.** No open six-max NLHE release with verified professional-level strength exists (noregrets HU Slumbot test is negative: −714.5 ± 331.5 mbb/hand). Reference compute: Pluribus 12,400 CPU core-hours (non-neural); DeepStack 175 core-years for labels; ReBeL 720 V100s; AlphaHoldem 576 GPU-hours + 4,608 core-hours. 14 days = 336 GPU-hours on the 5080.
- **GPU in phase 1: "With the recommended CPU engine alone, the RTX 5080 has no solver role."** Only sanctioned GPU use: optional overnight batch pre-solving *if* a GPU backend (GTOpen) passes license + correctness checks. Do not write a CUDA CFR in the baseline window.
- Phase 2 (weeks 3–8): compact surrogate trained on our own solver's policy + action-EV labels for HU turn/river; then DeepStack-style range-conditioned CFV net. Human hand histories (PHH: 21.6M NLHE hands, 2009) → opponent priors only, never EV.
- EV contract: per-action incremental EV in bb, fold = 0, prior chips sunk; keep hero's strategic range (don't collapse to a singleton); never merge pairwise HU solves into a "multiway equilibrium".

---

## 2. Disagreements / tensions between reports

1. **R1 vs R3 on postflop-solver (AGPL).** R1: embed it as the core crate (private use → AGPL harmless). R3: "Do not vendor; use only as a spec" and write a permissive range parser. Resolves on one user answer: will the app ever be distributed? If never → R1. If maybe → run it as a **sidecar process** (aggregate, not combined work) and keep `poker-core` permissive, or budget a clean-room DCFR engine (weeks).
2. **R2 vs R1/R6 on self-solving preflop.** R1 cites GTOpen preflop lab (2.3 s) and DCFR-SOLVER as if preflop is cheap; R6 says "start with precomputed six-seat preflop strategies"; R2 says self-solves are approximate research games and recommends licensing PokerData/GTOsims. Resolve by: (a) a 1-day benchmark of DCFR-SOLVER preflop on the i7 vs RangeConverter/GTOBase RFI charts; (b) the user's budget answer (§4.1).
3. **R5 (Rust-only core, TS UI) vs R3 (Python glue + Rust core) vs R1 (PyO3 module for "the Python 3.12 app").** Two rules engines in two languages is waste. Resolve: one host language. With Tauri, Python is test-oracle only (phevaluator, PokerKit fixtures), rules engine lives in Rust. R4 agrees ("Python for orchestration only if it meets the contract").
4. **R1 vs R4/R6 on multiway postflop.** R1 proposes a heuristic ("HU vs strongest range, tightened ranges + bunching"); R4/R6 say do not collapse opponents into a fictitious HU range and call it GTO — report "unsupported". Product-policy decision (§4.5): show a **labelled** approximation or nothing.
5. **R4's phase-1 exploit plan (10 days) vs the 1–2 week whole-baseline budget.** R4 assumes "an existing usable solver and range source" already integrated. Cannot both fit. Resolve: baseline first; exploit river-only slice if time remains, otherwise phase 2.
6. **R4's engine contract vs what postflop-solver exposes.** R4 needs "evaluate arbitrary complete policy pair" + "best response vs fixed hero policy". R1 lists `strategy`, `expected_values`, `equity`, `lock_current_strategy`, `solve()` → exploitability. Whether a fixed-policy evaluation/BR audit is exposed (vs. only solving) is not stated → validation task §3.
7. **R1 timing extrapolations vs R6 caution.** R1: FAST flop ~5–8 s on a modern 16-core, turn 0.3–2 s. R6: "a universal 15-second guarantee for arbitrary live states is not verified". Not contradictory; both say measure first.
8. **R2 vs R4 on the preflop baseline shape.** R4's transforms need per-combo action probabilities per context; R2 finds only 169-class charts rounded to 25–50% for free. A rounded chart degrades every downstream exploit number and range replay. Same resolution as §2.2.
9. **R1 ("chronologically the only actively developed solver is GTOpen") vs R6/R2 (license unverified; not a dependency).** All three agree not to embed today; R1 alone suggests asking the author for a license. Cheap action: open an issue.

---

## 3. "Not verified" claims that materially affect the design → validation tasks

Engine / latency
- V1. postflop-solver builds on Windows 11 with current stable Rust at a pinned commit; AVX2 path works on i7-13700K (R1).
- V2. Turn-tree solve time for a realistic 2–3-size tree on the i7 (no published numbers; R1 guess 0.3–2 s FAST, single-digit s larger).
- V3. FAST-style flop tree on the i7 to 0.5%: R1 extrapolates 5–8 s from 12.6 s on a 2019 Ryzen 3700X.
- V4. postflop-solver API can: lock per-infoset policy, evaluate a fixed policy pair, compute best response vs hero's fixed policy, cancel mid-solve, report progress (R4 contract; R1 API listing partial).
- V5. Regrets can be warm-started after a lock change without corrupting averages (R4).
- V6. Memory per FAST tree (679 MB 16-bit / 1.27 GB f32) and per stored flop-node strategy (R1 says "MBs", estimate).
- V7. GTOpen: license grant from author; correctness vs postflop-solver on identical spots; RTX 5080 timings (R1/R6: 6.7–12 s on RTX 3090, excludes tree build + upload).
- V8. TexasSolverGPU "24.4 s vs Pio 86.1 s" (forum page 403; unverified; irrelevant unless used as an external batch tool).

Preflop data
- V9. PokerData: cash-catalogue solver provenance, full-branch coverage, EV coverage, offline/live-use licence, 250bb/straddle availability and bulk price (R2).
- V10. GTOsims `.bin`: per-action EV extractable; reuse rights (R2).
- V11. FreeBetRange free export preserves mixed weights (R2).
- V12. DCFR-SOLVER preflop: chip accounting, continuation semantics (`oop_pot_tax`, board sampling), rake/straddle/depth configurability, hardware behind "100M iters/14 min" (R2).
- V13. rs-poker `6Max-RFI-GTO` example JSON provenance (R2/R3) — treat as example only.

Libraries
- V14. rs-poker `arena` handles side pots / multiple all-ins correctly for cash (R3: struct not confirmed).
- V15. `hand_indexer` builds with MSVC `cc` on Windows (R3).
- V16. b-inary evaluator 1.2 G eval/s and pokers weighted `@` API behaviour (R3) — check vs phevaluator on all 133,784,560 7-card hands.

App stack
- V17. Tauri 2 exposes wry `with_browser_accelerator_keys(false)`; otherwise `preventDefault` suffices (R5).
- V18. Playwright `connectOverCDP` coexists with Tauri's WebView2 args (R5) — else use `@wdio/tauri-service`.
- V19. Rust compile-time/iteration comfort on this machine in week 1 (R5's trigger for the Electron fallback).

Exploit layer
- V20. Every numeric tag default (RFI multipliers, ΔF/kB/kR, `c`, `alpha_cap`, `epsilon`, prior strength 12) — uncalibrated (R4).
- V21. 5–15 s end-to-end including audit, cold and warm cache, p50/p95 (R4/R6).

Rules
- V22. Venue straddle order/amounts (UTG vs Mississippi) and rake/time-collection rule for the user's actual room (R2).

---

## 4. Open decisions only the user can make

1. **Preflop source: pay vs transcribe vs self-solve vs buy a solver.** (a) PokerData Pro $100/mo or bulk quote — best ingestion fit, per-action EV, provenance/live-use rights unverified. (b) GTOsims $519/pack — clearest provenance, 100bb only, EV extraction unverified. (c) Transcribe free charts (RangeConverter 200bb 50%-rounded; GTOBase viewer; FreeBetRange export) — $0, no EV, rights unverified, degrades exploit layer. (d) Self-solve with DCFR-SOLVER/own MCCFR — $0, approximate, days of audit, label as "approximate model". (e) Buy HRC/Simple Preflop Holdem and solve own straddle/250bb configs (JSON export with EVs; price/runtime not researched). Also: which 2–4 of the 24 configurations first.
2. **AGPL acceptance / distribution intent.** Private-only → link postflop-solver directly (fastest). Might share binaries → sidecar isolation now, or accept AGPL for the whole app, or fund a clean-room permissive DCFR engine (weeks, not in phase 1).
3. **Tauri vs Electron.** Tauri: one Cargo workspace, 10 MB, best solver integration, minor WebView2/LLM friction. Electron: mainstream, Playwright, 100–250 MB, solver as child process (which also solves the AGPL isolation question for free).
4. **Host language.** Rust core + TS UI (R5) vs Python orchestration (R1/R3 partially). Determines whether PokerKit/phevaluator are product deps or test oracles.
5. **Multiway postflop policy.** Show "unsupported — equity only" (R4/R6, honest) vs a labelled HU-vs-strongest-range approximation with bunching (R1, useful at live tables where multiway is common). Either way multiway gets range-vs-range equity from `pokers`.
6. **Exploit layer in phase 1?** River-only tags+locks with `alpha_cap 0.25` (≈3–4 days) vs GTO-only baseline first. R4's full phase 1 does not fit the 2-week budget alongside the solver/UI.
7. **GPU time-box.** Spend 1–2 days evaluating GTOpen (only if the author grants a license) for overnight flop batches vs defer all GPU work to phase 2. R6's default: defer.
8. **Flop library breadth for the first overnight run.** 47 vs 95 vs 184 flops × ~45 scenarios × 1–2 depths (≈2.5 h / 5 h / 10 h at 20 s per FAST solve on one depth; ×6 for realistic 120 s trees).
9. **Which live game first** (stakes, straddle habit, rake/time charge) — R2 says the single most common actual configuration should be the week-1 target.

---

## 5. Suggested phase-1 scope (bullets = components) and phase-2 backlog

Phase 1 (1–2 weeks)
- `poker-core` (Rust): card/hand parser ("AsKd"), Pio-syntax range parser ↔ 1326-vector, flop canonicalization (1,755 classes + weights), 6-max state machine with straddles/antes/side pots (rs-poker `arena` or in-house, ~PokerKit-validated).
- Evaluator + equity: b-inary evaluator (MIT) validated vs phevaluator; `pokers` for weighted range-vs-range equity (display + multiway fallback).
- Postflop engine adapter: postflop-solver at a pinned commit (in-process or sidecar per §4.2); HU flop/turn/river, rake + cap, locks; dedicated solve thread, cancel token, progress channel, 15 s deadline with lower-accuracy fallback.
- Preflop store: versioned local cache of licensed/transcribed strategies (169-class → combo weights with 6/4/12 multiplicities), explicit "unsupported node" results, straddle-aware position/order model.
- Range replay: Bayesian conditioning of every seat's range on the full observed action sequence (hero's strategic range kept; hero combo only for display).
- Flop pre-solve library: overnight batch over a Pio subset × core SRP/3-bet scenarios × 1–2 depths; persist flop-node strategies + reach ranges only; nearest-isomorph lookup.
- Live pipeline: flop from library (or FAST tree), turn/river solved live with ranges seeded from the library; cache keyed by ranges/pot/stacks/rake/tree/board.
- Result contract: per-action frequency + incremental EV (bb, fold = 0), coverage label (exact / approximate / unsupported), model assumptions, solve time.
- Desktop shell: Tauri 2 + React/TS, keyboard-first seat/action/card entry, undo stack, accelerator suppression; Electron fallback wired to the same `poker-proto`.
- Benchmark harness: p50/p95 solve times + peak RAM on the i7 for river/turn/FAST-flop trees (closes V1–V3, V6, V21).
- Tests: `cargo test` fixed-seed solve, evaluator oracle, rules-engine fixtures (PHH/Pluribus histories), `mockIPC` UI tests.
- Optional if time: exploit slice — seat tags + preflop multipliers + river-only combo locks, `alpha_cap 0.25`, matched EV display.

Phase 2 backlog
- Full R4 exploit layer: turn trees, best-response audit + certificate, Beta/Dirichlet observation updates, schema, RNR/DBR, later OX-Search-style safety.
- GPU: GTOpen (if licensed) or own cudarc path for batch pre-solving; only then consider online GPU solves.
- Neural: compact surrogate on solver labels (HU turn/river), then DeepStack-style CFV leaf net for depth-limited resolving.
- Multiway: exact bunching, side pots, labelled multiway models; evaluate commercial HRC/GTO Wizard outputs as calibration.
- Preflop breadth: commissioned 250bb/straddle trees; audited self-solve tier (DCFR-SOLVER) as "approximate".
- Flop library to all 1,755 flops × more depths; aggregated reports.
- Population priors from PHH / Hand2Note live DB; per-room straddle/rake presets.
- LAN/phone UI via axum; clean-room permissive solver if distribution is wanted.

---

## 6. Numbers table

| Figure | Value | Hardware / context | Report |
|---|---|---|---|
| Desktop Postflop 0.2.1 flop, Pio 3betpotFAST tree | 12.6 / 15.6 / 27.9 s @ 0.5/0.3/0.1% | Ryzen 7 3700X, 16 thr, Win10 | R1 |
| Same, 6 thr | 20.0 / 24.9 / 44.4 s; 1.27 GB f32, 679 MB 16-bit | same | R1 |
| PioSOLVER Free, same spot, 6 thr | 22.9 / 28.2 / 60.1 s; 1.41 GB | same | R1 |
| GTO+, same spot, 16 thr | 13.9 / 19.7 / 41.7 s | same | R1 |
| TexasSolver 0.2.0, same spot, 16 thr | 67.1 / 95.9 / 182.6 s; 2.84 GB | same | R1 |
| wasm-postflop (browser) | ~2× slower than native | same | R1 |
| GTOpen CUDA, 1.35M-node flop to 0.3% pot | 6.7–12.0 s (excl. tree build/upload); 3.6–7.6 GB VRAM | RTX 3090 + 5950X, 16 thr | R1/R6 |
| GTOpen CPU f32 vs CUDA, bench spot, 100 iters | 60.8 s / 2.5 GB vs 6.7 s (≈9–10×) | same | R1 |
| GTOpen river toy (33 nodes) | 0.024 s | 5950X | R1 |
| GTOpen preflop lab 6-seat / 8-seat | 2.317 s (175 it, 503 MB) / 5.323 s (125 it, 1.31 GB) | RTX 3090 | R1/R2 |
| Turn / river live solve (estimate) | turn 0.3–2 s FAST, single-digit s larger; river < 1 s | extrapolation, unverified | R1 |
| FAST flop on modern 16-core (estimate) | 5–8 s @ 0.5% | extrapolation, unverified | R1 |
| Canonical flops | 1,755 of 22,100; Pio subsets 184 / 95 / 47 | — | R1/R3 |
| HU-to-flop scenarios, 6-max | ≈45–60 per depth; ×2–3 depths ≈ 100–180 | — | R1 |
| Flop library cost | 184×45 ≈ 8.3k solves ≈ 46 h @ 20 s; 1,755×45 ≈ 79k ≈ 440 h; @120 s: 12 days / 110 days | one depth | R1 |
| DCFR-SOLVER preflop | 2.2M infosets; 100M iters in 14 min (122k it/s) | hardware unstated | R2 |
| NoRegrets 6-max MCCFR | 200M iters / 79 min; 2B iters / 5h20, 36–45 GB RSS | 16-core Apple Silicon | R2 |
| Pluribus blueprint | 12,400 core-hours, <512 GB RAM; play <128 GB, 1–33 s/decision | 64 cores | R2/R6 |
| PokerData | 48 bundles, 20–200bb; 7/12/19 MB; Pro $100/mo, 250k req, 5 GB; 5% rake cap 0.5bb | — | R2 |
| GTOsims 100bb NL200 pack | $519; 5% rake cap 1.37bb | — | R2 |
| Hand2Note live DB | 1,039,274 hands, 37,942 players; TAG sample 3-bet 13%, fold-to-3bet 26% | streamed live games | R2 |
| PHH human NLHE dataset | 21,605,687 hands (2009, NL25–NL1000) | — | R6 |
| Evaluator speeds | b-inary ~1.2 G/s seq (5950X, 1 thread); OMPEval 272 M/s rand, 775 M/s seq (i7-3770K); phevaluator 56 M/s; rs-poker 50 M+/s/core; PokerKit ~1.0 M/s, treys 3.2 M/s (i7-1255U) | as cited | R3 |
| Installer size | Tauri ≈ 8.6 MiB vs Electron ≈ 244 MiB (macOS measurement) | — | R5 |
| Exploit defaults | alpha_cap 0.25; epsilon = min(0.10 bb, 0.005×pot); c = 0/0.35/0.60/0.80; gate 0.02 bb; prior strength 12 | proposed, unverified | R4 |
| Latency budget | 1 + 4 + 6 + 3 + 1 s = 15 s | proposed | R4 |
| GPU capacity | 14 days = 336 GPU-h; 60 days = 1,440 GPU-h | RTX 5080 | R6 |
| Neural compute refs | AlphaHoldem 576 GPU-h + 4,608 core-h; DeepStack 175 core-years labels; ReBeL 720 V100 | as published | R6 |
| noregrets vs Slumbot | −714.5 ± 331.5 mbb/hand (blueprint) | HU, 10k hands | R6 |

## Top-10 sources
1. https://github.com/b-inary/postflop-solver — core engine (AGPL, suspended)
2. https://github.com/b-inary/wasm-postflop/blob/main/README.md — cross-solver benchmark table
3. https://github.com/MatthewPDingle/GTOpen — CUDA solver, no license
4. https://github.com/bupticybee/TexasSolver — AGPL CLI fallback
5. https://github.com/exinori/DCFR-SOLVER — MIT, 6-max preflop MCCFR
6. https://docs.pokerdata.io/nlhe/ and https://pokerdata.io/ — preflop JSON + EV, pricing
7. https://gtosims.com/products/100bb-6-max-cash-rake-nl200-pokerstars — $519 pack spec
8. https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem — free 200bb charts
9. https://v2.tauri.app/develop/calling-rust/ — commands, Channel, async runtime
10. https://github.com/EddieMataEwy/pokers and https://github.com/b-inary/holdem-hand-evaluator — MIT equity + evaluator
