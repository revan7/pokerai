# R1 — Open-source NLHE solver survey (as of 2026-09-10)

Context: local Windows 11 desktop assistant for 6-max NLHE cash, live table, manual input, target ≤5–15 s per recommendation. Hardware: RTX 5080 16 GB, multi-core CPU, Python 3.12, Node 24, Rust available.

Method: GitHub REST API (`api.github.com/repos/...`) for license/`pushed_at`/stars (fetched 2026-09-10), repo READMEs/docs, and the b-inary cross-solver benchmark. Every number is attributed. "Not verified" marks claims I could not confirm from a primary source (several forum pages returned HTTP 403).

---

## 1. Summary table

| Solver | Lang | License (SPDX) | Last push | Status | HU postflop | Multiway postflop | Rake | Node lock | ICM | Turn/river-only | Exploit. target | GPU | Embedding |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| bupticybee/TexasSolver | C++ | AGPL-3.0 | 2026-08-26 | Functional; author moved to closed GPU product | Yes | No | No | No | No | Yes (4/5-card board) | Yes (`set_accuracy`) | No | CLI `console_solver -i file`, JSON dump |
| bupticybee/TexasSolverGPU | Python (viewer) + closed binaries | **Proprietary EULA, no OSI license** | 2026-03-21 | Binary-only distribution | Yes | not verified | not verified | Yes (GUI) | not verified | not verified | not verified | CUDA (required) | Windows GUI only; JSON export via viewer.py |
| b-inary/postflop-solver | Rust | AGPL-3.0 | 2024-07-09 | Development suspended Oct 2023; stable | Yes | No (bunching for ≤4 folded players) | Yes (`rake_rate`,`rake_cap`) | Yes (`lock_current_strategy`) | No | Yes (`initial_state`) | Yes (`solve(..., target_exploitability)`) | No | Rust crate (git dep), bincode/zstd serialization |
| b-inary/desktop-postflop | Rust+Vue (Tauri) | AGPL-3.0 | 2023-11-13 | Suspended; last release v0.2.7 (2023-10-01) | Yes | No | Yes (UI) | No (roadmap struck) | No | Yes | Yes | No | GUI app only |
| b-inary/wasm-postflop | Rust+Vue (WASM) | AGPL-3.0 | 2023-10-01 | Suspended | Yes | No | Yes | No | No | Yes | Yes | No | Browser app only |
| MatthewPDingle/GTOpen | Rust (+CUDA nvrtc) | **None (no LICENSE file; API `license: null`)** | 2026-09-10 | Very active (created 2026-06-13) | Yes | No (preflop lab 2–9 players only) | Yes (% + cap) | Yes | No | Yes | Yes (% of pot) | CUDA optional | Rust crates, `gto-server` CLI, localhost HTTP/JSON API |
| exinori/DCFR-SOLVER | Rust | MIT | 2026-03-16 | New (created 2026-03-15), single burst of commits | Yes | No | No | No | No | Yes (`--street turn/river`) | not stated | No | CLI, JSON/HTML export |
| google-deepmind/open_spiel | C++/Python | Apache-2.0 | 2026-08-31 | Very active | Generic tabular CFR; `universal_poker` (ACPC) up to 10 players | In principle (ACPC), impractical | No | No | No | Yes (board/handReaches params) | Via exploitability API | No (3rd-party GPU ports exist) | pip wheels; but ACPC/universal_poker needs source build |
| JoakimMich/opensolver | Rust | None (null) | 2022-08-30 | Abandoned | Yes | No | not verified | No | No | not verified | not verified | No | UPI text protocol |
| noambrown/poker_solver | Python + C++ | MIT | 2026-01-05 | Reference/teaching code, 2 commits | River only | No | No | No | No | River only | not stated | No | JSON config |
| amaster97/poker_solver | Python + Rust (PyO3) | MIT | 2026-06-18 | New, active-ish | Yes (card abstraction 256/128/64) | No | No | Yes | No | Yes | not stated | No | Python lib, CLI, JSON |
| krukah/robopoker | Rust | MIT | 2026-09-08 | Active | Pluribus-style blueprint + subgame search (bot, not range solver) | 6-max blueprint (feature flag) | No | No | No | n/a | n/a | No | Rust crates, HTTP/WebSocket |
| ericgjackson/slumbot2019 | C++ | MIT | 2023-09-18 | Dormant | Yes (abstracted CFR+/MCCFR) | MCCFR only, partial | No | No | No | Subgame resolving | n/a | No | Executables |
| AI-Decision/DecisionHoldem | C++/Python | AGPL-3.0 | 2024-05-29 | Dormant; parts binary-only | Yes (blueprint + depth-limited) | No | No | No | No | n/a | n/a | No | `.so` + Python |
| a9876543245/DEEPFOLD-SOLVER | C++/CUDA/Rust/TS | **"All rights reserved"** (source "for transparency", no OSI license) | 2026-08-07 | Commercial (members-only installers) | Yes | claims multiway aggregation | not stated | Yes | No | not verified | not verified | CUDA | Not embeddable legally |
| bupticybee/TexasSolverGPU, DEEPFOLD, gto-poker-solver-arena | — | — | — | Not open source / not real solvers | | | | | | | | | |

---

## 2. Per-candidate details

### 2.1 TexasSolver (CPU) — https://github.com/bupticybee/TexasSolver
- Language C++; license AGPL-3.0 (GitHub API `spdx_id: AGPL-3.0`); 2,541 stars; created 2020-01-29; `pushed_at` 2026-08-26 (README now says "a much much faster new GPU version is available here: TexasSolverGPU"). Last release v0.2.0 published 2021-11-04 (API `releases/latest`).
- Features (console README, `console` branch): `set_pot`, `set_effective_stack`, `set_board` (3/4/5 cards → flop/turn/river start), `set_range_ip/oop`, `set_bet_sizes` (per player/street/action), `set_allin_threshold`, `set_raise_limit`, `set_thread_num`, `set_accuracy`, `set_max_iteration`, `set_use_isomorphism`, `set_dump_rounds`, `build_tree`, `start_solve`, `dump_result` (JSON). Hold'em and short-deck. No rake, no node locking, no ICM, 2 players only (none of these appear in the command set).
- Solve times:
  - Own benchmark (README): flop, SPR=10, 6 threads: TexasSolver 0.1.0 172 s / 1,600 MB / 0.275% vs PioSOLVER 1.0 242 s / 492 MB / 0.29%. Hardware not stated. Source: https://github.com/bupticybee/TexasSolver/blob/master/README.md
  - Independent benchmark (b-inary, wasm-postflop README, Pio "3betpotFAST" preset, Ryzen 7 3700X, Windows 10): TexasSolver v0.2.0 — 6 threads: 103.5 s (0.5%), 149.0 s (0.3%), 285.9 s (0.1%); 16 threads: 67.1 s / 95.9 s / 182.6 s; memory 2.84 GB. Source: https://github.com/b-inary/wasm-postflop/blob/main/README.md
  - "River < 1 s, turn usually < 10 s on a MacBook Pro" comes from the older Java version README (https://github.com/bupticybee/TexasHoldemSolverJava); the C++ README claims 5× faster than Java and <1/3 memory. Not verified for C++ specifically.
- GPU: none in this repo.
- Embedding: run `console_solver -i input.txt`, parse dumped JSON; no library API or Python bindings (Java version had a Python API; C++ does not). Builds with qmake; GUI needs Qt5.
- Note: the project landing page (https://bupticybee.github.io/texassolver_page/) says "personal edition free; commercial usage contact author", which conflicts with AGPL-3.0 (which permits commercial use). Treat the repo LICENSE as authoritative but be aware of the author's stated intent.

### 2.2 TexasSolverGPU — https://github.com/bupticybee/TexasSolverGPU
- **Not open source.** README: "This repo is for binary distribution, metadata, screenshots, and helper tools only"; EULA.md: "no claim is made here that the private solver source code is open source or publicly licensed." GitHub API `license: null`. Language shown as Python (viewer only). 90 stars. Created 2026-03-19; releases v0.1.0 (2026-03-19) and v0.2.0 (2026-03-21), Windows x64 zip ~4.7 MB. Requires NVIDIA GPU + WebView2. VRAM requirement not stated.
- Benchmark (forum announcement, via search snippet — page returned 403, not verified directly): Qs Jh 2h flop, TexasSolverGPU 24.4 s vs PioSolver 86.1 s, both <0.2% exploitability, "same hardware", GPU model not stated. Source: https://forumserver.twoplustwo.com/167/poker-software/free-texassolver-gpu-high-performance-cuda-solver-4x-faster-than-cpu-1859729/
- Features listed: tree building, batch solving of many boards, node-lock inspection, practice mode, JSON results via `viewer.py`. Multiway/rake/ICM not documented.
- Embedding: GUI only; no CLI/API documented. Cannot be embedded.

### 2.3 b-inary postflop-solver / desktop-postflop / wasm-postflop
- postflop-solver — https://github.com/b-inary/postflop-solver — Rust library, AGPL-3.0 (API), 367 stars, `pushed_at` 2024-07-09; README: "As of October 2023, I have started developing a poker solver as a business and have decided to suspend development of this open-source project." Not on crates.io (use `git = ...` dependency). Author warns the crate is a GUI backend first and had breaking changes without version bumps.
  - Algorithm: Discounted CFR (γ=3.0), alternating updates, hand-vectorized, no card abstraction, suit isomorphism for turn/river deals, f32 (f64 sums) or 16-bit compressed mode, rayon multithreading, SIMD-verified. Bunching effect for up to 4 folded players (6-max) — "the only implementation that can handle the bunching effect" per README (slows solving significantly per API doc).
  - API (docs https://b-inary.github.io/postflop_solver/postflop_solver/): `TreeConfig { initial_state: BoardState (flop/turn/river), starting_pot, effective_stack, rake_rate, rake_cap, flop/turn/river_bet_sizes: [BetSizeOptions;2], turn/river_donk_sizes, add_allin_threshold, force_allin_threshold, merging_threshold }`; `PostFlopGame::{memory_usage, allocate_memory(compress), set_bunching_effect, lock_current_strategy, unlock_current_strategy, play, apply_history, strategy, expected_values, equity, remove_lines}`; `solve(game, max_num_iterations, target_exploitability: f32, print_progress) -> f32` (target in chips; use pot × 0.005 for 0.5%). Serialization via `bincode`, optional `zstd`. No ICM, no multiway, no Python bindings (none found on PyPI or in forks).
  - Solve times (Desktop Postflop v0.2.1 = this engine, native; wasm-postflop README benchmark, Pio "3betpotFAST" preset with all-in threshold 100%, Ryzen 7 3700X, Windows 10):
    - 6 threads: 32-bit 20.0 s (0.5%), 24.9 s (0.3%), 44.4 s (0.1%), 1.27 GB; 16-bit 19.8 / 24.7 / 44.0 s, 679 MB. PioSOLVER Free (6 threads): 22.9 / 28.2 / 60.1 s, 1.41 GB. GTO+: 22.0 / 31.4 / 67.7 s, 705 MB.
    - 16 threads: 32-bit 12.6 / 15.6 / 27.9 s; 16-bit 12.2 / 15.1 / 27.0 s. GTO+ 13.9 / 19.7 / 41.7 s.
    - WASM version ~2× slower (33.4 / 41.2 / 71.9 s at 6 threads).
    - Source: https://github.com/b-inary/wasm-postflop/blob/main/README.md
    - Turn/river-only timings: not published; extrapolation only (a turn tree is ~1/49 of the equivalent flop tree's chance branching, so ~0.3–2 s for FAST-style trees, single-digit seconds for larger trees — my estimate, not verified).
- desktop-postflop — https://github.com/b-inary/desktop-postflop — Tauri GUI, AGPL-3.0, 349 stars, last release v0.2.7 2023-10-01 ("This is the last release I will be making"), `pushed_at` 2023-11-13. Can use >4 GB RAM (WASM cannot). Roadmap items never implemented: node locking UI, aggregated multi-flop reports, trainer. Windows x86-64 AVX2 required.
- wasm-postflop — https://github.com/b-inary/wasm-postflop — AGPL-3.0, 592 stars, `pushed_at` 2023-10-01. Browser only.
- Forks: jiyee/GTO-Solva (fork of desktop-postflop, AGPL-3.0, pushed 2023-10-11) — no new development. No maintained fork with Python bindings found.

### 2.4 GTOpen — https://github.com/MatthewPDingle/GTOpen
- Rust solver core (`crates/solver`), HTTP server (`crates/server`, `gto-server` binary, default http://127.0.0.1:3737), browser UI (`web/`), optional CUDA engine (nvrtc). 11 stars; created 2026-06-13; `pushed_at` 2026-09-10 (daily commits). Contains `AGENTS.md` (AI-assisted development).
- **License: none.** Root listing shows no LICENSE/COPYING; GitHub API `license: null`; README/docs contain no license statement. Legally this is "all rights reserved" (GitHub ToS only permits viewing/forking on GitHub). Not safe to embed or modify without asking the author.
- Features (README + docs/technical_reference.md): heads-up postflop DCFR (α=1.5, β=0, γ=2, alternating, hand-vectorized), CFR+ and PCFR+ selectable; f32 or i16-compressed arenas ("equivalent to f32 within 0.1% pot exploitability"); bet sizes as % pot / multiples / all-in; rake (% + cap); node locking (frequency locks or player model), with correct exploitability under locks; flop/turn/river start; "Postflop is heads-up only. There is no ICM"; Preflop Lab 2–9 players (approximate continuation model, not a true multiway solve); reports over 47/95/184/1,755 canonical flops; player models from Ignition/CoinPoker datasets; convergence target = exploitability as % of pot with true best-response check.
- CUDA: "level-synchronous CFR with f32 working arenas in VRAM"; VRAM budget = free VRAM − 512 MB (override `SOLVER_GPU_MEM_MB`); estimates memory and falls back to CPU.
- Solve times (docs/postflop_performance.md; Ryzen 5950X, RTX 3090 24 GB, 64 GB RAM, 16 Rayon threads, Windows release build):
  - 1,346,813-node flop trees: GPU 51.8 ms/iter (rainbow, no rake) and 31.4 ms/iter (two-tone, 5% rake) per technical_reference; ~7.6 GB GPU staging+arenas.
  - Full solve to 0.3%-pot target (200 iterations): rainbow flop 12.0 s, two-tone flop 7.3 s (later 6.667 s in the 2026-09-07 benchmark); river (33-node toy) 0.024 s.
  - bench/results.json (bench spot, 100 iterations): CPU f32 DCFR 60.8 s / 2,500 MB; i16 71.9 s / 1,366 MB; GPU 6.7 s / 3,958 MB process + up to 6,955 MB arena → ~9–10× GPU speed-up over 16-thread CPU on that spot (units in the summary are ambiguous; ratio is what matters).
  - Preflop Lab: 6-seat 2.317 s, 8-seat 5.323 s (2026-09-07 benchmark).
- Embedding: localhost HTTP JSON API (`POST /api/spot`, `/api/solve`, `/api/node`, `/api/lock`, `/api/exploit`, `/api/runouts`, `/api/save|load`, preflop endpoints). No Python bindings. Own save format.
- Caveats: 3-month-old, one author, no license, no external validation against Pio published (a `bench_spot.json` exists). The "51.8/31.4 ms" figures and 7.6 GB arena for a 1.35M-node tree imply a 16 GB RTX 5080 can hold roughly that size of tree, not much larger.

### 2.5 DCFR-SOLVER — https://github.com/exinori/DCFR-SOLVER
- Rust, MIT (API), 18 stars, created 2026-03-15, `pushed_at` 2026-03-16 (one-day burst; no activity since). Written "from scratch, no dependencies on existing solver engines".
- Features: HU postflop from flop/turn/river (`--street`), range-vs-range with GTO+-style range syntax, bet sizes as % pot per street (+ geometric option), raise sizes, max raises per street (default 4), suit isomorphism; DCFR (default), CFR+ (`--no-dcfr`), EGT, QRE; 6-max preflop solver via external-sampling MCCFR; JSON and HTML export; rayon threads. No rake, no node locking, no ICM, no multiway postflop, no GPU, CLI only.
- Performance claims (README, hardware NOT stated): postflop "1.18 → 3.12 iter/s" after optimization (spot unspecified); preflop "100 million iterations in 14 minutes" (~122k it/s); "0.016% exploitability at 10K iterations" for the preflop solver. No wall-clock flop/turn/river solve times. Treat as unbenchmarked.

### 2.6 OpenSpiel CFR family — https://github.com/google-deepmind/open_spiel
- Apache-2.0 (API), 5,470 stars, `pushed_at` 2026-08-31. C++ algorithms: `cfr`, `cfr_br`, `external_sampling_mccfr`, `outcome_sampling_mccfr`, `tabular_exploitability`, `best_response`. Python: `cfr.py`, `cfr_br.py`, `discounted_cfr.py`, `mccfr.py`, `external/outcome_sampling_mccfr.py`, `exploitability.py`, `best_response.py`; Deep CFR / RCFR under other directories per docs/algorithms.md.
- Poker: `universal_poker` wraps the ACPC engine (optional dependency, `OPEN_SPIEL_BUILD_WITH_ACPC=ON`, requires building from source — not in the pip wheel per docs/install.md); "has not been extensively reviewed/tested by the DeepMind OpenSpiel team". Betting abstractions: `fcpa`, `fc`, `fullgame`, `fchpa`; params: numPlayers (max 10), blinds, stacks, potSize, boardCards, handReaches (for subgame solving with 1,326 hands). Windows support is "experimental" (docs/windows.md); ACPC on Windows not documented.
- Performance: tabular CFR expands the full extensive-form tree (chance nodes included) with no hand-vectorization or suit isomorphism, so NLHE flop/turn subgames are far out of reach; only tiny-abstraction river subgames are solvable (Kovařík et al. 2021, arXiv:2112.10890, report an FCPA river subgame with ~61.0M states / 21,620 decision points via search snippet — full-text numbers not verified). Kim 2024 (arXiv:2408.14778, RTX 4090 / Ryzen 3900X) reports a matrix-form GPU CFR up to 203.6× faster than OpenSpiel's C++ CFR on games up to ~550k nodes — none are hold'em. Conclusion: OpenSpiel is a research toolkit, not a viable ≤15 s NLHE postflop engine.

### 2.7 GPU-accelerated CFR for hold'em — what actually exists
- GTOpen (2.4): the only open-source-in-form, range-based NLHE postflop CUDA solver found; unlicensed.
- TexasSolverGPU (2.2) and DEEPFOLD-SOLVER (https://github.com/a9876543245/DEEPFOLD-SOLVER — C++/CUDA/Rust/TS, 412 stars, created 2026-04-22, pushed 2026-08-07; README: source "published for transparency", installers for "DEEPFOLD PRO members", "© DEEPFOLD — All rights reserved"; engine sidecar binaries and 31 MB chart library not in repo; recommends RTX 2000+ with 4 GB+ VRAM; claims "sub-percent exploitability in seconds for typical turn spots", no hardware): both closed.
- tjennings/poker_solver (https://github.com/tjennings/poker_solver, Python/PyTorch, MIT, 0 stars, pushed 2026-02-02): abstraction-based HUNL (169 preflop buckets, 25–184 flop-texture classes), CUDA/MPS batching, "~27 s per flop" (no hardware). Not a range-vs-range solver.
- gustafbergmark/pokerrust (Rust + CUDA 12.2, KTH thesis, 1 star, no license): CPU vs GPU CFR on fixed-flop subgames; no benchmarks in README.
- janrvdolf/gpucfr (C++/CUDA, no license, pushed 2025-06-06): vanilla CFR on Goofspiel EFGs only.
- uoftcprg/noregret (formerly gpugt; Python/CuPy, MIT, pushed 2026-09-09): Kuhn/Leduc/Liar's Dice + generic OpenSpiel wrapper; no NLHE range solver.
- Egiob/cfrx (JAX, MIT, pushed 2026-03-11): Kuhn/Leduc only.
- Kim 2024 "GPU-Accelerated CFR" (arXiv:2408.14778): matrix-form CFR, games ≤58M nodes (Battleship), not hold'em; code described as open-source but URL not in the HTML version (not verified).

### 2.8 Other candidates (checked, not recommended)
- JoakimMich/opensolver (Rust, no license, pushed 2022-08-30): DCFR with UPI text protocol; README admits ~2× slower than commercial on rainbow flops and worse on two-tone/monotone (isomorphism incomplete). Abandoned.
- noambrown/poker_solver (MIT, Python + C++, 166 stars, created/pushed 2026-01-05): educational river-only CFR/CFR+/MCCFR/DCFR with JSON bet-size config; 2 commits.
- amaster97/poker_solver (MIT, Python + Rust via maturin, 4 stars, pushed 2026-06-18): HUNL DCFR with card abstraction 256/128/64 buckets (so not exact range solves), node locking, Pio-style ranges, NiceGUI; README: "deep-stack full-range flop solves ... expect minutes". No benchmarks.
- krukah/robopoker (MIT, Rust, 220 stars, pushed 2026-09-08): Pluribus-parity blueprint MCCFR + depth-limited subgame search; trained on 16 vCPU / 120 GB; −13.1 bb/100 vs Slumbot (±14.0). It is a bot framework, not a configurable range solver; no rake/locks.
- ericgjackson/slumbot2019 (MIT, C++, pushed 2023-09-18): abstracted CFR+/MCCFR/targeted CFR and subgame resolving for HU; multiplayer only partially (MCCFR). Research tooling, dormant.
- AI-Decision/DecisionHoldem (AGPL-3.0, pushed 2024-05-29): HUNL bot (blueprint + depth-limited search), parts shipped as `.so` only.
- masterai-top/cfr-poker-ai-masterai (C++, license NOASSERTION — Apache-2.0 text plus conflicting MIT reference, pushed 2026-09-08): HUNL research bot; benchmark claims lack reproducibility.
- Jotaeme961/gto-poker-solver-arena (55 stars, "MIT"): SEO/marketing README; no solver algorithm disclosed. Ignore.
- facebookresearch/rebel (Apache-2.0): Liar's Dice only; archived 2024-11-01.

---

## 3. Cross-cutting findings
1. **No open-source solver does multiway (3+) postflop.** Every real range solver (TexasSolver, postflop-solver, GTOpen, DCFR-SOLVER, opensolver) is heads-up only. Only postflop-solver models folded players' card removal (bunching, ≤4 folders). Multiway support exists only in bot frameworks (robopoker, slumbot2019 MCCFR) or generic engines (OpenSpiel ACPC) that cannot solve real-size trees quickly. For a live 6-max game, multiway pots will need heuristics or HU approximations.
2. **No ICM anywhere** (irrelevant for cash).
3. **Rake**: postflop-solver (rate + cap), GTOpen (% + cap), desktop/wasm-postflop UI. TexasSolver and DCFR-SOLVER: none. Live cash rake matters for river bet/call thresholds.
4. **Node locking**: postflop-solver (`lock_current_strategy`), GTOpen, amaster97. TexasSolver: none.
5. **Fastest measured CPU engine** on an apples-to-apples spot is b-inary's engine (Desktop Postflop 12.6 s at 0.5% on 16 threads of a 2019 Ryzen 3700X; beats PioSOLVER Free and GTO+ on the same spot; TexasSolver is ~5× slower and uses ~2.2× the memory).
6. **GPU**: only GTOpen is open-in-form (≈10× over its own 16-thread CPU path; 7–12 s per 1.35M-node flop to 0.3% on an RTX 3090) but it has no license. The closed GPU products (TexasSolverGPU, DEEPFOLD) cannot be embedded.
7. **Maintenance**: b-inary's stack is frozen but stable; TexasSolver is effectively frozen (author moved to a closed product); GTOpen is the only actively developed one (3 months old, single author); DCFR-SOLVER is a one-day drop.

---

## 4. Consolidated solve-time evidence (all cited above)
| Spot | Engine | Hardware | Time | Target |
|---|---|---|---|---|
| Flop, Pio 3betpotFAST tree | Desktop Postflop 0.2.1 | Ryzen 3700X, 16 thr | 12.6 s / 27.9 s | 0.5% / 0.1% |
| Same | Desktop Postflop, 6 thr | same | 20.0 s / 44.4 s | 0.5% / 0.1% |
| Same | PioSOLVER Free, 6 thr | same | 22.9 s / 60.1 s | 0.5% / 0.1% |
| Same | TexasSolver 0.2.0, 16 thr | same | 67.1 s / 182.6 s | 0.5% / 0.1% |
| Flop, SPR 10 (own tree) | TexasSolver 0.1.0, 6 thr | unstated | 172 s | 0.275% |
| Flop, 1.35M nodes | GTOpen CUDA | RTX 3090 + 5950X | 7.3–12.0 s | 0.3% pot |
| Flop, bench spot, 100 iters | GTOpen CPU f32 vs CUDA | 5950X 16 thr / RTX 3090 | 60.8 s vs 6.7 s | n/a |
| Flop Qs Jh 2h | TexasSolverGPU (closed) | unstated | 24.4 s (Pio 86.1 s) | <0.2% (not verified) |
| River | GTOpen (33-node toy) | 5950X | 0.024 s | 0.3% |
| River / turn | TexasHoldemSolverJava | MacBook Pro | <1 s / <10 s | unstated |

No project publishes turn-only timings for realistic trees on modern CPUs; expect ≤ a few seconds for 2-size trees on 16+ threads (extrapolation, not verified).

---

## 5. Recommendation

### 5.1 Core engine: embed **b-inary/postflop-solver** (Rust, AGPL-3.0)
Reasons: fastest measured CPU engine; exact (no card abstraction); rake with cap; node locking; turn/river start; exploitability-targeted `solve()`; 16-bit compression halves RAM; `memory_usage()` lets you size trees before allocating; bunching effect is the only open-source handling of folded 6-max players; bincode/zstd save/load for a precomputed library. Frozen since 2023 but 367 stars and a clean crate; you have a Rust toolchain. Integration: build a small Rust sidecar (JSON over stdin/stdout or a localhost socket) or a PyO3 module for the Python 3.12 app. Pin a commit; the author warned of breaking changes.

Fallbacks/alternates: TexasSolver as a CLI fallback only (AGPL, no rake, ~5× slower). GTOpen's CUDA engine is technically attractive for the RTX 5080 (≈10× CPU, 7–12 s per large flop) but **has no license** — do not embed unless the author adds one (open an issue asking). DCFR-SOLVER (MIT) is unbenchmarked; OpenSpiel is unsuitable for latency.

### 5.2 Hitting ≤15 s live
- River and turn spots: solve live with postflop-solver on all cores (river < 1 s; turn expected single-digit seconds for ≤2 sizes/street — verify on your CPU).
- Flop spots: a "FAST"-style tree (1–2 sizes, 100% all-in threshold) reaches 0.5% in ~12.6 s on a 2019 8-core; a modern 16-core should land ~5–8 s (extrapolation). Full trees (3 sizes/street, several raises, 100 bb SRP) take minutes — so **yes, a precomputed flop library is required** for anything beyond minimal trees, and it also supplies the post-flop-action ranges you need to seed live turn/river solves.

### 5.3 Flop library sizing
- 22,100 flops → 1,755 suit-isomorphic classes (GTO Wizard glossary; PioSOLVER flop-subsets blog). Pio-standard subsets: 184 / 95 / 47 flops; GTOpen also reports over 47/95/184/1,755.
- Heads-up-to-flop preflop scenarios for 6-max cash (one stack depth): SRP open+call over position pairs = 15; 3-bet pots = 15; 4-bet pots = 15; plus limped/squeeze variants ≈ 45–60 scenarios. Live games vary stack depth, so ×2–3 stack buckets ≈ 100–180 scenarios.
- Solve counts: 1,755 × 45 ≈ 79k (one depth), ≈ 237k with 3 depths; 184 × 45 ≈ 8.3k (≈ 25k with 3 depths).
- Time: at ~20 s/flop (FAST-ish tree, modern CPU, 0.5%) → 184-subset ≈ 46 h per stack depth; full 1,755 ≈ 440 h. At ~120 s/flop (realistic 3-size trees) → 184-subset ≈ 12 days; 1,755 ≈ 110 days. Practical plan: 184-flop subset × core SRP/3-bet scenarios × 1–2 depths first; nearest-isomorph lookup for unsolved flops; store only flop-street strategies + ranges (small) and re-solve turn/river live.
- Storage: a FAST tree is ~0.7–1.3 GB in RAM but you only need to persist flop-node strategies/reach ranges per solve (MBs), not full trees.

### 5.4 License implications (AGPL-3.0 / GPL in a personal, non-distributed tool)
- AGPL §0: "propagate ... except executing it on a computer or modifying a private copy"; "Mere interaction with a user through a computer network, with no transfer of a copy, is not conveying." GNU FAQ: "The GPL does not require you to release your modified version ... You are free to make modifications and use them privately, without ever releasing them." Copies within one organization are not distribution.
- AGPL §13 only applies if "your modified version ... supports [remote network] interaction" with users; a desktop tool (or a localhost service used only by you) has no remote users, so it is not triggered. If you ever expose it to friends over a network or ship binaries, the whole combined work must be offered under AGPL-3.0 with Corresponding Source.
- Linking postflop-solver as a crate makes your binary a combined work (GNU FAQ on linking). Running TexasSolver/GTOpen as a separate process over pipes/HTTP is generally a separate program ("aggregate"), though the FAQ notes intimate data exchange can blur this. For private use none of this matters; it only constrains future distribution.
- GTOpen (no license) and TexasSolverGPU / DEEPFOLD (EULA / all rights reserved): no right to modify or embed; private execution of GTOpen is a gray area (GitHub ToS covers viewing/forking on GitHub, not local use) — ask the author for a license before relying on it.

### 5.5 Open items / risks
- Turn-tree timings on your CPU must be measured; no published numbers.
- Multiway pots are unsupported by every candidate — decide on a heuristic (e.g., HU vs. strongest range with tightened ranges + bunching) up front.
- postflop-solver is unmaintained; keep a vendored fork.
- RTX 5080 (16 GB) is only useful if GTOpen becomes licensed or you write your own CUDA path; today's recommendation is CPU-first.
