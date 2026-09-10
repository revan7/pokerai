# R3 - Library survey: evaluators, ranges, rules engines, isomorphism

Scope: 6-max NLHE cash desktop assistant on Windows 11; Rust (or C++) solver core, Python 3.12 glue, desktop UI.
Date of survey: 2026-09-10. All version/date facts below were read from PyPI/crates.io JSON, GitHub pages, or docs.rs on that date unless marked "not verified".

Conventions: "M/s" = million evaluations per second. "MC" = Monte Carlo. "Exact" = full enumeration.

---

## 1. Hand evaluators / equity calculators

### 1.1 Summary table

| Library | Lang | License | Latest release / activity | 7-card speed (claimed) | Equity vs RANGE | Notes |
|---|---|---|---|---|---|---|
| PokerKit | Python | MIT | 0.7.5, 2026-08-22 (PyPI) | ~1.02 M hands/s (own docs, i7-1255U, Py 3.11.5) | MC only (`calculate_equities`), ranges via `parse_range`, no weights | Also a full rules engine (see §3) |
| treys | Python | MIT | 0.1.8, 2022-06-21 (PyPI); last commit 2023-03-09 | ~3.23 M hands/s (PokerKit's bench of treys, same machine) | No | Stale; evaluator only |
| eval7 | Python (Cython) | MIT | 0.1.11, 2026-07-29 (PyPI); wheels cp39-cp315 incl. Windows | No numeric claim in README | Yes: hand-vs-range exact + MC, all-hands-vs-range; weights parsed but ignored by equity | Weighted PokerStove-style parser |
| phevaluator (PokerHandEvaluator) | C/C++ core, Python pkg | Apache-2.0 | Python 0.6.0, 2026-07-11; repo commits Aug 2026 (dependabot) | 56 M/s random 7-card, 60 M/s sequential (own Google Benchmark) | No (pre-flop equity *example* only) | Perfect-hash, ~100 kB tables |
| OMPEval | C++ | ISC (+libdivide) | Last commit 2016-08-21 | 272 M/s random, 775 M/s sequential (i7-3770K @4.28 GHz) | Yes: range-vs-range, MC + exact, max 6 players, multithreaded | Unmaintained but stable; many forks |
| poker-eval (pokersource) | C | GPL-3.0-or-later | 2005-era; forks on GitHub | not verified | Enumeration APIs (not verified for range strings) | GPL - see §5 |
| SKPokerEval | C++ | LICENSE.md present; terms not verified | 309 commits; dates not verified | 146-223 M/s (OMPEval's table) | No | 97 KiB tables |
| rs-poker | Rust | Apache-2.0 | 5.1.0, 2026-08-13 (crates.io) | "50M+ hands/sec per core", 7-card "< 25 ns" (README) | MC per-hand (`MonteCarloGame`); no range-vs-range | Also arena rules engine (§3) |
| holdem-hand-evaluator (b-inary) | Rust | MIT | Git only (not on crates.io: search returned 0) | "~1.2G eval/s sequential @Ryzen 9 5950X single-threaded" (README) | No | ~212 kB tables; used by postflop-solver |
| pokers (EddieMataEwy) | Rust | MIT | 0.10.0, 2026-03-29 (crates.io) | not published; OMPEval rewrite lineage | Yes: up to 9 ranges, MC + exact, multithreaded, weights via `@` | Maintained fork of rust_poker |
| rust_poker (kmurf1999) | Rust | MIT | 0.1.14, 2021-03-04 | "Fast hand evaluation" (no number) | Yes: up to 6 ranges, MC + exact | Superseded by `pokers` |
| pyrust-poker | Python bindings to rust_poker | MIT | 0.2.3, 2024-12-22; only `cp312 manylinux` wheel | - | Yes (same as rust_poker) | No Windows wheel on PyPI |
| aya-poker | Rust | Zlib OR Apache-2.0 OR MIT | 0.1.0, 2023-10-31 (single release) | Charts only, no absolute numbers | No | Compile-time PHF tables; 0-7 cards; many variants |
| poker (deus-x-mackina) | Rust | MIT | 0.7.0, 2025-06-15 | not published | No | 5-card and 3-card only - not 7-card |
| poker_eval (oscar6echo) | Rust | MIT | 0.1.0, 2024-02-05 (single release) | not verified | not verified | Low adoption (2.4k downloads) |
| robopoker `deuce` | Rust | MIT (repo) | robopoker 1.2.0, 2026-09-06 | "Nanosecond evaluation, outperforming Cactus Kev" (README; not verified) | not verified | Part of a Pluribus-style solver workspace |

### 1.2 Per-library detail

**PokerKit** - https://github.com/uoftcprg/pokerkit , PyPI https://pypi.org/project/pokerkit/
- MIT. 0.7.5 released 2026-08-22; requires Python >=3.11. 493 stars, 99% coverage claim. Paper: arXiv 2308.07327 (IEEE ToG 2025 per search snippet; not verified).
- Evaluator: deuces/treys-style prime-product lookup. Docs benchmark (https://pokerkit.readthedocs.io/en/stable/evaluation.html): "PokerKit 1016740.7 hands/s vs treys 3230966.4 hands/s" on "single core of Intel Core i7-1255U ... Python 3.11.5"; "treys is a bit faster ... inevitable consequence of having a generalized high-level interface".
- Equity: `pokerkit.analysis.calculate_equities(...)` and `calculate_hand_strength(...)` are Monte Carlo (`sample_count`), with optional `executor` for multiprocessing (https://pokerkit.readthedocs.io/en/stable/analysis.html). No exact enumeration. Ranges are "simply a set of frozen sets of cards" from `parse_range`; no weights.
- API quality: high (typed, doctested), but hand evaluation is pure Python and ~50x slower than C-backed options.

**treys** - https://github.com/ihendley/treys
- MIT, "A Python 3 port of Deuces". PyPI 0.1.8 on 2022-06-21; last commit 2023-03-09. README: "Treys won't beat a C implementation (~250k eval/s)". No range or equity API. Not recommended for new work.

**eval7** - https://github.com/julianandrews/pyeval7 , PyPI https://pypi.org/project/eval7/
- MIT. 0.1.11 on 2026-07-29 with wheels for CPython 3.9-3.15 on Windows/macOS/Linux (from PyPI file list). Cython core, "based on Anonymous7's codebase".
- Ranges: "a parser for weighted PokerStove style hand ranges", e.g. `"AQs+, 0.4(AsKs)"`, `"AJ+, ATs, KQ+, 33-JJ, 0.8(QQ+, KJs)"`. Note: weight syntax is `w(hands)`, NOT Pio's `hand:w`.
- Equity: `py_hand_vs_range_exact`, `py_hand_vs_range_monte_carlo`, `py_all_hands_vs_range` - README: "These don't yet support weighted ranges and could probably benefit from optimization." No range-vs-range function; Deck code "too slow for full range vs. range equity calculations" (README).
- No published hands/s figure.

**phevaluator / PokerHandEvaluator** - https://github.com/HenryRLee/PokerHandEvaluator , PyPI https://pypi.org/project/phevaluator/
- Apache-2.0. Python 0.6.0 on 2026-07-11, requires Python >=3.10,<4; compiled wheels incl. `cp314-win_amd64`. Repo has no GitHub Releases; develop branch commits dated Aug 2026 (dependabot).
- Algorithm: "perfect hash algorithm to get the hand strength from a pre-computed hash table ... ~100kb for the 7 card evaluation". Benchmarks (README, Google Benchmark 2023-09-02): random 5/6/7-card 72/64/56 M/s; all 133,784,560 7-card hands in 2.23 s (60 M/s); PLO 29-32 M/s.
- Ports listed in README: JS, Dart, Haskell, Go, TypeScript, C#, and Rust ("poker_engine ... by Alexander Leones", https://github.com/aleo101/poker_engine - status not verified).
- Equity: none in the library; a "Hold'em pre-flop equity estimator" example only.

**OMPEval** - https://github.com/zekyll/OMPEval
- ISC (libdivide bundled under its own license). C++11. Last commit 2016-08-21. Forks exist (isitgto, valentino-sm, vanip, dalorveen) - none verified as maintained.
- README benchmark (i7-3770K @4.28 GHz, single thread, M eval/s): Seq OMP 775 / SKPE 223 / 2+2 1588 / ACE 80; Rand OMP 272 / SKPE 146 / 2+2 19 / ACE 43. "perfect hashing to reduce the size of the main lookup table from 36MB to 200kB"; init ~10 ms.
- Equity: `EquityCalculator` with `CardRange` ("QQ+,AKs,AcQc", "A2s+", "random"), "Supports Monte Carlo simulation and full enumeration", "Max 6 players", automatic multithreading; "2-10x faster (per thread) than the free version of Equilab". This is exactly the 6-max range-vs-range design we need; no weights in the range syntax (not verified whether the API accepts per-combo weights).

**poker-eval (pokersource)** - https://pokersource.sourceforge.net/ , forks e.g. https://github.com/atinm/poker-eval
- GPL-3.0-or-later (FreshPorts/Homebrew metadata). Maintainer note dated 2005. Python binding `pypoker-eval` also GPL. Old, 32/64-bit C, enumeration-based. GPL makes it unusable in a non-GPL app (see §5).

**SKPokerEval** - https://github.com/kennethshackleton/SKPokerEval
- "A fast and lightweight 32-bit Texas Hold'em 7-card hand evaluator written in C++", "97KiB" tables. LICENSE.md exists but I could not fetch it (404 on raw/blob URLs) - license not verified. OMPEval was "originally based on SKPokerEval".

**rs-poker** - https://github.com/elliottneilclark/rs-poker , crates.io `rs_poker`
- Apache-2.0. 5.1.0 on 2026-08-13 (5.0.0 2026-06-09, 4.1.0 2025-08-28); 92k downloads. README: "Hand evaluation at 50M+ hands/sec per core"; docs: 5-card "~20 nanoseconds per hand", 7-card "< 25 ns". Uses u64 CardBitSet, Gosper's hack, PDEP.
- Equity: `holdem::MonteCarloGame` for multi-player MC given specific hands; `OutsCalculator`. No range-vs-range calculator found in docs.
- Range parsing: `holdem::RangeParser::{parse_one, parse_many}` -> `Vec<FlatHand>`; accepts "AKs", "AKo", "AhKh", "TT+", "T9o+", "A9s+", "JT-67s", "AQ-J9". No weights.
- Also: arena (rules engine, §3), PLO4-7, ICM, Open Hand History export, CFR agent.

**holdem-hand-evaluator (b-inary)** - https://github.com/b-inary/holdem-hand-evaluator
- MIT, Rust. "Super fast hand rank evaluator ... ~1.2G eval/s sequential @Ryzen 9 5950X single-threaded", "small lookup tables (about 212KB)". Not on crates.io (crates.io search for the name returned 0 results) - use as a git dependency or vendor (MIT permits). Used inside postflop-solver (but the evaluator itself is MIT, separate repo). PyPI `holdem-hand-evaluator` 0.4.1 (2023-10-09) is a thin binding exposing only `evaluate_hand()`; no license metadata on PyPI.

**pokers** - https://github.com/EddieMataEwy/pokers , crates.io `pokers`
- MIT (copyright Kyle Murphy 2020 + Eduardo Mata Ewy 2024 - i.e. the maintained fork of rust_poker, itself "a rust rewrite of zekyll's C++ equity calculator, OMPEval"). 0.10.0 on 2026-03-29; 15.6k downloads; ~11.5k LOC.
- "range vs range equities for up to 9 different ranges specified by equilab-like range strings", "monte carlo simulations and exact equity calculations", "Multithreaded", cancel token + progress callback, "Individual hand and combo results". Weights via `@`, e.g. `"AA,KK,QQ@50"` (README example). No Python bindings.

**rust_poker / pyrust-poker** - https://github.com/kmurf1999/rust_poker , https://github.com/Townsheriff/python_rust_poker
- MIT. rust_poker 0.1.14 (2021-03-04), unmaintained; up to 6 ranges, MC + exact, multithreaded; "first build ... will take extra time to generate the hand evaluation table"; optional `hand_indexer` dep (§4). pyrust-poker 0.2.3 (2024-12-22) wraps it; PyPI ships only `cp312 manylinux_2_34_x86_64` wheel + sdist - **no Windows wheel**, sdist needs a Rust toolchain.

**aya-poker** - https://github.com/dtrifuno/aya-poker
- Zlib OR Apache-2.0 OR MIT. Single release 0.1.0 (2023-10-31). "compile-time generated perfect hash function lookup tables"; "evaluate hands with 0 to 7 cards, with the missing cards counting as the worst possible kickers"; supports lowball, short-deck, Omaha (no Omaha tables), Badugi. Benchmark charts only (Ryzen 5 2600), no absolute numbers in README.

**poker (deus-x-mackina)** - https://github.com/deus-x-mackina/poker - MIT, 0.7.0 (2025-06-15). treys port; docs describe "three-card and five-card" evaluation only. Not suitable for 7-card.

**robopoker** - https://github.com/krukah/robopoker - MIT. Workspace of crates (robopoker/pokerkit/vitals/daybook/elkan 1.2.0 on 2026-09-06; rbp-core/rbp-database/rbp-auth/rbp-gameplay 1.0.0 on 2026-02-12). README claims "Fastest open-source hand evaluator ... Nanosecond evaluation, outperforming Cactus Kev" - not verified. Note the Rust crate named `pokerkit` here is unrelated to Python PokerKit.

### 1.3 Newer (2024-2026) options found
- rs-poker 4.x/5.x (2025-2026) - most actively released Rust option.
- pokers 0.8-0.10 (Feb-Mar 2026) - the only maintained Rust range-vs-range calculator on crates.io.
- robopoker 1.x (2026) - solver workspace; evaluator claims not verified.
- eval7 0.1.11 and phevaluator 0.6.0 (both Jul 2026) - Python binaries for 3.13/3.14.
- No new evaluator algorithm faster than OMPEval/PHE/b-inary was found; the 2+2 evaluator remains fastest for sequential (1588 M/s) but has a ~130 MB table and poor random-access speed (19 M/s).

---

## 2. Range notation parsers

Target syntax: `AA,KK,AKs,A5s-A2s,77+,KQo` with weights `AKs:0.5` (PioSolver style).

### 2.1 Formats in the wild
- **PioSolver text**: comma-separated groups, optional `:weight` in [0,1] (search snippet from Pio docs: "AA:1 KK:0.5 Means that you have AA twice as often as KK"; also `#Range0#AA,KK,(...)` / `#Range1#AA:0.25,(..)` in script files). Direct fetch of https://piosolver.com/docs/viewer/numbers_in_piosolver/ did not show the example string - exact page not verified.
- **PioSolver UPI**: `set_range OOP|IP <1326 floats>` in the fixed order returned by `show_hand_order` ("2d2c 2h2c 2h2d ..."); https://piosolver.com/docs/upi/commands/ .
- **GTO Wizard**: Ranges tab "copy the ranges from any node into standard Piosolver / GTO+ text input" (https://help.gtowizard.com/ranges-tab/). A 2+2 thread describes the output as pasteable into "Pio, GTO+, HRC, Flopzilla, Power Equilab". Exact weight delimiter in GTO Wizard export not verified (assumed Pio `:`).
- **GTO+**: bracket syntax (`[75]AA[/75]`) - not verified.
- **EquiLab / PokerStove**: unweighted "QQ+,AKs"; PokerStove weights as `0.4(AsKs)` (eval7 dialect).

### 2.2 Parsers

| Library | Lang | License | Grammar | Weights | Export |
|---|---|---|---|---|---|
| postflop-solver `Range` | Rust | **AGPL-3.0** | "inspired by PioSOLVER": singleton (AA, AKs, AKo, AsAh), plus (TT+, ATs+, T9o+), dash (QQ-88, A9s-A6s, 98o-65o); comma-separated; `FromStr`, `from_sanitized_str` | Yes: "optional weight separated by a colon", e.g. `AK:0.5` | `to_string`, `raw_data()` -> `[f32; 1326]` | 
| pokers `HandRange` | Rust | MIT | equilab-like: "AK,22+", plus/dash, "random" | Yes but `@` percent syntax (`QQ@50`) | not verified |
| rs-poker `RangeParser` | Rust | Apache-2.0 | AKs/AKo/AhKh/TT+/T9o+/A9s+/JT-67s/AQ-J9 | No | No |
| rust_poker `hand_range` | Rust | MIT | equilab-like | not verified | not verified (unmaintained) |
| eval7 `HandRange` | Python | MIT | "extended PokerStove": AJ+, ATs, KQ+, 33-JJ, AsKs | Yes, `0.8(QQ+, KJs)` (PokerStove style, not Pio) | No Pio export |
| PokerKit `parse_range` | Python | MIT | AKs, T9o, 22, 33-55, T9s-QJs, J8s+, `|` union, `-` difference | No | No (returns set of frozensets) |
| poker (pokerregion) `Range` | Python | MIT | 22+, 66-, 55-33, AJo+, A5o-, XX, AX, 2s2h | No | `to_ascii`, `to_html` | 
| pypokertools | Python | MIT | PokerStove tokenizer example | No | No |
| OMPEval `CardRange` | C++ | ISC | EquiLab-like, "random" | No (not verified) | No |

Sources: https://raw.githubusercontent.com/b-inary/postflop-solver/main/src/range.rs ; https://github.com/EddieMataEwy/pokers ; https://docs.rs/rs_poker/latest/rs_poker/holdem/struct.RangeParser.html ; https://github.com/julianandrews/pyeval7 ; https://pokerkit.readthedocs.io/en/stable/analysis.html ; https://poker.readthedocs.io/en/latest/range.html (package last released 2019-09-09, effectively unmaintained).

### 2.3 Finding
There is **no permissively-licensed, maintained parser in Python or Rust that speaks the Pio `hand:weight` dialect and round-trips to the 1326-float UPI vector**. The only one that does (postflop-solver) is AGPL. The grammar is small (three group forms + colon weight); writing our own (~150 lines) and treating postflop-solver's doc comment as the spec (not copying code) is the right call. `pokers` is closest in Rust but uses `@percent`.

---

## 3. Game-state / rules engines for NLHE

### Python
| Library | License | Latest | 6-max NLHE rules | Notes |
|---|---|---|---|---|
| **PokerKit** | MIT | 0.7.5, 2026-08-22 | Yes: blinds/straddles/antes, betting rounds, side pots, all-in with automated chip distribution, multi-runouts; hand-history (PHH) import/export; `Statistics` | Best-documented; slow evaluator (pure Python) |
| texasholdem (SirRender00) | MIT | 0.11.0, PyPI 2024-09-21 | Yes: "Compliance with World Series of Poker Official Rules", multi-pot `chips_at_stake`, history export/import, text GUI, agents | Python >=3.8,<4; 108 stars; roadmap to 1.0 |
| clubs (fschlatt) | **GPL-3.0** | 0.1.4, 2021-11-16 | "full n-player No Limit Texas Hold'em or Pot Limit Omaha", configurable blinds/antes/raise sizes | GPL; stale |
| pokerengine | MIT | 1.9.7, 2025-04-11 | "enables the entire process of playing Texas Hold'em"; C++/pybind11 | No repo URL on PyPI; side-pot behaviour not verified |

Sources: https://github.com/uoftcprg/pokerkit ; https://pypi.org/pypi/pokerkit/json ; https://github.com/SirRender00/texasholdem ; https://pypi.org/pypi/texasholdem/json ; https://github.com/fschlatt/clubs ; https://pypi.org/pypi/pokerengine/json .

### Rust
| Crate | License | Latest | 6-max NLHE rules | Notes |
|---|---|---|---|---|
| **rs-poker `arena`** | Apache-2.0 | 5.1.0, 2026-08-13 | `GameState` has `num_players`, `dealer_idx`, `small_blind/big_blind/ante`, `stacks`, `player_bet`, `total_pot`, `player_active`/`player_all_in` bitsets, `player_winnings`, `round`; agents (Calling/Random/CFR), tournaments, OHH export | **Caveat**: `max_raises_per_round: Option<u8>` "Default is Some(3). When exceeded, raises are converted to calls" - must set `None` for NL cash. Explicit side-pot struct not confirmed in docs (winnings vector implies settlement) |
| chironaut | MIT OR Apache-2.0 | 0.5.0, 2026-09-06 | NL/PL/FL; button/SB/BB/UTG; preflop-river; "Automatic all-in scenarios and side pot calculations"; run-it-twice/thrice; snapshot validation; property + fuzz tests | GitHub URL (burnmandont/chironaut) returned 404 on fetch; only 2k downloads; young |
| robopoker `kicker` (+ `rbp-gameplay` 1.0.0, 2026-02-12) | MIT | 1.2.0, 2026-09-06 | "side-pot/all-in/tie settlement" (README); designed for solver, not table play | Heavy workspace (Postgres, telemetry) |
| postflop-solver | **AGPL-3.0**, "Development suspended" (Oct 2023) | - | Postflop action tree only, "up to four folded players (6-max game)"; not a hand-dealing engine | License blocker |
| holdem (vsupalov), casino_poker, RumenDamyanov/rust-poker | various | old / not verified | not verified | Not evaluated further |

Sources: https://docs.rs/rs_poker/latest/rs_poker/arena/game_state/struct.GameState.html ; https://lib.rs/crates/chironaut ; https://crates.io/api/v1/crates/chironaut ; https://github.com/krukah/robopoker ; https://github.com/b-inary/postflop-solver .

### C/C++
No maintained permissive C++ NLHE rules engine was found worth adopting; OMPEval and PHE are evaluators only.

---

## 4. Board / flop canonicalization and hand isomorphism

### 4.1 The numbers
- 22,100 = C(52,3) raw flops; 1,755 suit-isomorphic classes. Derivation: 13 trips + 13*12*2 paired (two-tone or rainbow) + C(13,3)*5 unpaired (monotone, rainbow, 3 two-tone placements) = 13 + 312 + 1430 = 1755. Confirmed in PioSolver blog ("22100 possible flops in Holdem out of which 1755 are strategically different", https://piosolver.com/blog/2015-11-05-flop-subsets/) and GTO Wizard blog (https://blog.gtowizard.com/poker-subsets-and-abstractions/).
- Each canonical flop must carry a weight = number of raw flops it represents (1, 3, 4, 6, 12, 24 depending on suit pattern) for subset averaging; GTO Wizard: "Each flop is assigned a weight".

### 4.2 Implementations

| Item | Lang | License | What it gives | Source |
|---|---|---|---|---|
| **kdub0/hand-isomorphism** (Kevin Waugh) | C | BSD-style ("Copyright (c) 2013 Kevin Waugh ... Redistribution and use in source and binary forms are permitted provided that the above copyright notice and this paragraph are duplicated") | `hand_indexer_init(rounds, cards_per_round)`, `hand_index_last`, `hand_unindex`; perfect, minimal indices for any round structure; e.g. [2] -> 169 preflop, [2,3] flop round -> 1,286,792 (standard figures; not re-verified from README, API lives in `src/hand_index.h`). A `[3]`-only indexer enumerates exactly the 1,755 canonical flops by construction (expected; not executed to verify) | https://github.com/kdub0/hand-isomorphism ; paper https://www.cs.cmu.edu/~kwaugh/publications/isomorphism13.pdf |
| `hand_indexer` crate (kmurf1999) | Rust FFI over Waugh's C | MIT | 0.1.2, 2021-02-25; "A rust interface around the poker hand indexer library by Kevin Waugh"; used by rust_poker/pokers. Needs a C compiler at build (cc) - MSVC on Windows | https://crates.io/api/v1/crates/hand_indexer |
| cleverpiggy/poker-hand-indexer | Rust FFI (src + src_c) | **No license field** in Cargo.toml (package also named `hand_indexer` 0.1.0 - name clash, not the crates.io one) | Wrapper exporting `Indexer` | https://github.com/cleverpiggy/poker-hand-indexer |
| botm/hand-isomorphism | Java | not verified | Direct port | https://github.com/botm/hand-isomorphism |
| robopoker `deuce` | Rust | MIT | Own implementation of "A Fast and Optimal Hand Isomorphism Algorithm" (Waugh 2013) with "bijective card encodings" | https://github.com/krukah/robopoker |
| pypokertools `examples/isomorph.py` | Python | MIT | `get_canonical(flop)`, `get_all_canonicals()`, `get_suit_isomorphs(flop)`, `get_translation_dict(flop)`; docstring: "22,100 possible flops, but only 1,755 canonical versions"; suit patterns AAA/AAB/ABA/ABB/ABC mapped to c/d/h | https://github.com/mjwestcott/pypokertools |
| gist dabd/5902296 | text | none stated | Plain list of the 1,755 flops ("isomorph flops (1755)") | https://gist.github.com/dabd/5902296 |
| GTOpen | ? | not verified | Batch-solves over "weighted canonical flop subset (47/95/184 or all 1755)" | https://github.com/MatthewPDingle/GTOpen |
| PokerAlpha article | C++ snippet | article | Canonical board signature: per-suit rank multisets, choose the lexicographically smallest of 24 suit permutations, hash to u64 | https://poker-alpha.com/en/insights/iso-canonical-signature/ |
| postflop-solver | Rust | AGPL | Combines "isomorphic chances (turn and river deals)" within a solve; flop is a fixed input | https://github.com/b-inary/postflop-solver |

No Python port/binding of Waugh's indexer was found (searched PyPI/GitHub; "not found" != "does not exist").

### 4.3 Finding
Flop canonicalization is ~50 lines (sort by rank desc, relabel suits by first appearance with a tie-break for paired ranks, weight = 24 / |stabilizer|). The reference implementations above (pypokertools, PokerAlpha) are enough to validate against the count 1755 and the gist list. For hand indexing during solving (hole cards + board, all streets), Waugh's C library via the MIT `hand_indexer` crate is the standard; alternatively port the algorithm to pure Rust (robopoker did, MIT) to avoid the C toolchain dependency on Windows.

---

## 5. Licenses - conflicts and cautions

- **AGPL-3.0**: postflop-solver, desktop-postflop, wasm-postflop. Linking or copying any of it (including the range parser) into a distributed desktop app obligates AGPL for the whole app. Also "Development suspended". Do not vendor; use only as a spec/reference.
- **GPL-3.0**: clubs (Python engine), poker-eval / pypoker-eval (C). Same copyleft problem for a distributed binary.
- **Unverified / missing license**: SKPokerEval (LICENSE.md not readable in this survey), cleverpiggy/poker-hand-indexer (no license in Cargo.toml), gist dabd/5902296, aleo101/poker_engine (PHE Rust port), PyPI `holdem-hand-evaluator` binding (no license metadata). Treat as unusable until verified.
- **Permissive and mutually compatible**: MIT (PokerKit, treys, eval7, pokers, rust_poker, b-inary evaluator, chironaut, robopoker, pypokertools, texasholdem), Apache-2.0 (phevaluator, rs-poker), ISC (OMPEval; libdivide has its own permissive license - check the file if vendoring), BSD-style (Waugh; keep copyright paragraph), tri-license (aya-poker). Apache-2.0 + MIT mixing is fine for either an MIT or proprietary product; retain NOTICE/attribution.
- **Name collision**: robopoker publishes a Rust crate named `pokerkit` (unrelated to Python PokerKit).
- **Windows packaging**: pyrust-poker has no Windows wheel; b-inary's evaluator is git-only; `hand_indexer` needs MSVC `cc` build. phevaluator, eval7, pokerkit all ship Windows wheels for 3.12.

---

## 6. Recommendation

### Python 3.12 side (glue / analysis / tests)
1. **PokerKit (MIT, 0.7.5)** - the rules engine: 6-max NLHE state machine with side pots, all-ins, multi-runouts, PHH hand histories, plus `parse_range` and MC `calculate_equities` for quick checks. Actively maintained through Aug 2026.
2. **phevaluator (Apache-2.0, 0.6.0)** - C-backed 7-card evaluator with `cp312-win_amd64` wheels when Python needs raw evaluation (56 M/s in C; Python call overhead will dominate) and as an oracle for testing the Rust evaluator.
3. **eval7 (MIT, 0.1.11)** - optional, only for exact/MC hand-vs-range checks in tests; its weights are parsed but ignored by equity and its syntax is PokerStove-style, so do not build the product range format on it.
4. **Own range parser** (Pio `hand:weight` grammar, 1326-float import/export, GTO Wizard paste) - or expose the Rust one through PyO3 so there is a single implementation.
5. Skip: treys (stale, no ranges), poker/pokerregion (2019), clubs (GPL), pyrust-poker (no Windows wheel, wraps unmaintained rust_poker).

### Rust side (solver core)
1. **Evaluator: b-inary/holdem-hand-evaluator (MIT, git dep)** - ~1.2 G eval/s sequential claim with 212 kB tables, already proven inside postflop-solver; fallback is rs-poker's evaluator (Apache-2.0, on crates.io, 50 M+/s). Validate both against phevaluator on all 133,784,560 7-card hands.
2. **Range-vs-range equity: `pokers` (MIT, 0.10.0, Mar 2026)** - OMPEval-lineage MC + exact enumeration for up to 9 ranges, multithreaded, weighted combos, cancel token; write a Pio-syntax -> pokers-syntax adapter (`:0.5` -> `@50`). If we want zero dependencies, port OMPEval's (ISC) enumeration design.
3. **Rules engine: rs-poker `arena` (Apache-2.0, 5.1.0, Aug 2026)** - most-downloaded, actively released, agents/CFR hooks, OHH export; set `max_raises_per_round = None`. Watch **chironaut (MIT/Apache, 0.5.0, Sept 2026)** as a leaner alternative with explicit side-pot and run-it-twice support if rs-poker's pot model proves awkward.
4. **Isomorphism: `hand_indexer` crate (MIT) over Waugh's C (BSD-style)** for hole+board indexing in the abstraction/solver; implement flop canonicalization (1755 classes with weights) in-house and unit-test against the gist list and pypokertools. Consider a pure-Rust port later to drop the C build step on Windows.
5. **Range parser: in-house** (Pio grammar per §2; postflop-solver's doc comment as spec, no code copied), with `raw_data() -> [f32; 1326]` in Pio `show_hand_order` order for import/export.
6. Skip: postflop-solver (AGPL, suspended), rust_poker (2021), `poker` crate (5-card only), aya-poker (single 2023 release; fine but unproven), poker_eval.

### C/C++ (only if the core ends up in C++)
- **OMPEval (ISC)** for evaluator + 6-player range-vs-range MC/enumeration; **PokerHandEvaluator C (Apache-2.0)** as a simpler evaluator; **Waugh hand-isomorphism (BSD-style)** for indexing. Avoid poker-eval (GPL) and SKPokerEval (license not verified).
