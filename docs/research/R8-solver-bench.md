# R8 - postflop-solver feasibility bench (measured on this machine)

Date: 2026-09-10. Throwaway spike; all code lives in the session scratchpad (`spike-solver/`), nothing was installed system-wide.

## 1. What was tested

| Item | Value |
| --- | --- |
| Repo | https://github.com/b-inary/postflop-solver (AGPL-3.0-or-later, development suspended Oct 2023) |
| Commit | `9d1509fe5077d019825f833eed04b16d342dfda1` - 2023-10-01 21:59:55 +0900 "docs: update README" (branch `main`) |
| Reference for the flop spot | wasm-postflop `97360db7` README "Comparison" section (3betpotFAST) |
| CPU / RAM / OS | Intel i7-13700K (8P+8E, 24 logical threads), 64 GB, Windows 11 Pro 10.0.26200 |
| Rust | `rustc 1.95.0 (59807616e 2026-04-14)`, **host `x86_64-pc-windows-gnu`** (the active/default rustup toolchain on this machine is the MinGW one; see section 6) |
| Crate features | default = `bincode` + `rayon` (rayon is on by default, no extra flag needed). `zstd` also built OK. `custom-alloc` not tested (nightly only). |
| Build profile | `--release` (opt-level 3). Two variants: default codegen (x86-64 baseline = SSE2) and `RUSTFLAGS="-C target-cpu=native"` (AVX2) |
| Threads | rayon global pool default = 24 (all logical CPUs). `RAYON_NUM_THREADS` is honoured (tested 16 and 8). |
| Accuracy target | exploitability <= 0.5 % of starting pot (and 0.3 % where noted); checked every 10 iterations, exactly like the crate's `solve()` |
| Timing method | wall clock around the `solve_step` loop (incl. the exploitability checks), separate process per run; "cold" = first process launch of that scenario, "warm" = immediately following identical run |
| Memory | `PostFlopGame::memory_usage()` estimate + Windows peak working set via `psapi!GetProcessMemoryInfo` read at the end of the process |

Background load at sample time was ~15 % CPU (other software on the box), which explains a few outlier runs (see 3.4).

## 2. Build on Windows

`cargo build --release --examples` **does not compile out of the box**. Two unrelated fixes were needed (both bit-rot, neither Windows-specific):

1. **bincode version drift (compile error).** `Cargo.toml` says `bincode = "2.0.0-rc.3"`, which on a fresh lockfile resolves to `bincode 2.0.1` (stable release with a changed `Decode<Context>` derive API) -> 8 errors in `src/mutex_like.rs`, then 74 errors from the derive macro. Fix used: `cargo update -p bincode --precise 2.0.0-rc.3` **and** `cargo update -p bincode_derive --precise 2.0.0-rc.3` (both needed; must be repeated in every downstream project, or pin `=2.0.0-rc.3` in a fork).
2. **Rust 1.95 deny-by-default lint `dangerous_implicit_autorefs`** in `src/action_tree.rs` lines 393/396/408 (`&*(*node).children[i].lock()` on a raw pointer) -> 3 errors. Fix used: the compiler-suggested 3-line patch `&*(&(*node).children)[i].lock()`. Alternatives: `RUSTFLAGS=-A dangerous_implicit_autorefs`, or depend on the crate via `git =` instead of `path =` (cargo then passes `--cap-lints allow` to it).

After that: builds in ~4 s incremental, 6 warnings, all `hiding a lifetime that's elided elsewhere is confusing` (cosmetic). The three shipped examples run correctly on Windows (`basic` 0.35 s, `node_locking` 0.16 s, `file_io` 0.33 s wall). `--features zstd` also builds (zstd-sys compiled with the MinGW gcc through the `cc` crate).

## 3. Measured solves

Spots (uniform-weight Pio-style range strings; dash ranges must be written high-to-low, e.g. `JJ-22`, or parsing fails):

* BTN open (IP, 463-472 combos after board removal): `22+,A2s+,K2s+,Q2s+,J4s+,T6s+,96s+,86s+,75s+,65s,54s,A2o+,K8o+,Q9o+,J9o+,T9o`
* BB defend (OOP, 564-574 combos): `JJ-22,AJs-A2s,K2s+,Q2s+,J2s+,T4s+,95s+,84s+,74s+,63s+,53s+,43s,AQo-A2o,K5o+,Q7o+,J8o+,T8o+,98o,87o`
* Flop spots use the crate's `examples/basic.rs` ranges (OOP 179 combos: `66+,A8s+,A5s-A4s,AJo+,K9s+,KQo,QTs+,JTs,96s+,85s+,75s+,65s,54s`; IP 264 combos: `QQ-22,AQs-A2s,ATo+,K5s+,KJo+,Q8s+,J8s+,T7s+,96s+,86s+,75s+,64s+,53s+`). The wasm-postflop README gives the 3betpotFAST board/pot/stack only via screenshots (Qs Jh 2h, pot 180, stack 910, OOP root bet 94 = 52 % pot); its ranges are weighted (214.1 combos shown) and are not published, so they could not be reproduced. Turn/river sizes of the Pio preset are also unknown; 52 % + 2.5x was used on every street.

| ID | Tree | Start | Pot / stack | Sizes (bet / raise), thresholds |
| --- | --- | --- | --- | --- |
| RIVER | Td9d6h Qc 2s | river | 100 / 100 | 33 %, 75 %, all-in / 2.5x; add_allin 1.5, force_allin 0 (0.15 would replace the 75 % bet by all-in at SPR 1), merge 0.1. Root actions: Check, Bet 33, Bet 75, AllIn 100 |
| TURN | Td9d6h Qc | turn | 60 / 140 | 33 %, 75 % / 2.5x; add_allin 1.5, force_allin 0.15, merge 0.1 |
| FLOP-FAST | Qs Jh 2h | flop | 180 / 910 | 52 % / 2.5x on every street; add_allin 1.0, force_allin 0.15, merge 0.1 |
| FLOP-FULL | Qs Jh 2h | flop | 180 / 910 | 33 %, 75 % / 2.5x on every street; add_allin 1.0, force_allin 0.15, merge 0.1 |

### 3.1 Main table (default codegen, 32-bit floats, 24 threads)

| Spot | Hands OOP/IP | Target | Run | Time to target (iterations) | Final expl. | Crate mem est. | Peak working set |
| --- | --- | --- | --- | --- | --- | --- | --- |
| RIVER | 564 / 463 | 0.5 % | cold | **0.004 s** (50) | 0.273 % | 0.3 MB | 6.6 MB |
| RIVER | | 0.5 % | warm | 0.004 s (50) | 0.273 % | 0.3 MB | 6.6 MB |
| RIVER | | 0.3 % | - | 0.004 s (50) | 0.273 % | | |
| TURN | 574 / 472 | 0.5 % | cold | **0.168 s** (60) | 0.461 % | 32.0 MB | 46.1 MB |
| TURN | | 0.5 % | warm | 0.165 s (60) | 0.461 % | 32.0 MB | 46.3 MB |
| TURN | | 0.3 % | - | 0.221 s (80) | 0.290 % | | 46.0 MB |
| FLOP-FAST | 179 / 264 | 0.5 % / 0.3 % | cold (run 1) | **13.4 s** (80) / **17.8 s** (100) | 0.271 % | 957.9 MB | 970 MB |
| FLOP-FAST | | 0.5 % / 0.3 % | warm (run 2) | 17.0 s / 23.2 s (outlier, see 3.4) | 0.271 % | | 970 MB |
| FLOP-FAST | | 0.5 % / 0.3 % | run 3 | 11.7 s / 16.3 s | 0.271 % | | 970 MB |
| FLOP-FAST | | 0.5 % / 0.3 % | run 4 | 11.4 s / 15.7 s | 0.271 % | | 970 MB |
| FLOP-FULL | 179 / 264 | 0.5 % | cold | **110.2 s** (140) (outlier, see 3.4) | 0.463 % | 3802 MB | 3815 MB |
| FLOP-FULL | | 0.5 % | warm | **42.2 s** (140) | 0.463 % | | 3815 MB |
| FLOP-FULL | | 0.5 % | run 3 | 41.5 s (140) | 0.463 % | | 3815 MB |

Tree build (`PostFlopGame::with_config`) is 1-70 ms and `allocate_memory` 0-10 ms for all spots (memory is demand-zero paged; the working set fills during the first iteration). `finalize()` is 0.2-0.35 s on the flop trees. The crate's `memory_usage()` estimate is within ~1.3 % of the measured peak working set on every spot.

Per-iteration cost (including the exploitability check every 10 iterations): RIVER ~0.08 ms, TURN ~2.8 ms, FLOP-FAST ~0.16 s (default) / ~0.077 s (native), FLOP-FULL ~0.30 s. For comparison the wasm-postflop README reports Desktop Postflop at 12.6 s / 15.6 s (0.5 % / 0.3 %), 1.27 GB, 16 threads on a Ryzen 7 3700X for its (larger-range) 3betpotFAST tree.

### 3.2 16-bit compression (`allocate_memory(true)`) on the flop trees

| Spot | Mode | Run | Time to 0.5 % / 0.3 % (iters) | Peak working set |
| --- | --- | --- | --- | --- |
| FLOP-FAST | f32 | best of 4 | 11.4 s / 15.7 s (80 / 100) | 970 MB |
| FLOP-FAST | i16 | cold | 29.2 s / 36.3 s (80 / 100) (outlier) | 510 MB |
| FLOP-FAST | i16 | warm | 15.0 s / 20.1 s (80 / 100) | 510 MB |
| FLOP-FULL | f32 | best of 3 | 41.5 s (140) | 3815 MB |
| FLOP-FULL | i16 | single | 44.2 s (130) | 1985 MB |

Compression halves memory (as the estimate says: 958 -> 498 MB, 3802 -> 1973 MB) at roughly +5 % (FLOP-FULL) to +25-30 % (FLOP-FAST) solve time; final exploitability is marginally different (0.276 % vs 0.271 %) because of the quantisation.

### 3.3 Threads and codegen (FLOP-FAST unless noted)

| Variant | Time to 0.5 % / 0.3 % | Note |
| --- | --- | --- |
| 24 threads (default) | 11.4-11.7 s / 15.7-16.3 s | best two of four runs |
| 16 threads (`RAYON_NUM_THREADS=16`) | 12.4 s / 16.7 s | same within noise |
| 8 threads | 16.0 s / 20.7 s | only ~1.35x slower than 24 -> memory-bandwidth bound |
| `-C target-cpu=native` (AVX2), 24 thr, run 1 | **6.2 s / 7.7 s** | ~1.9-2.1x faster than default codegen |
| `-C target-cpu=native`, run 2 | 6.2 s / 7.7 s | reproducible |
| `-C target-cpu=native`, TURN | 0.207 s / 0.271 s | no gain (tiny tree) |
| `-C target-cpu=native`, FLOP-FULL | 42.2 s | no gain (3.8 GB tree, memory bound) |

objdump on the two bench binaries: default build has 0 `vmulps/vaddps`, 109 SSE `mulps/addps` (1309 `ymm` references come from std/regex runtime-dispatched code); the native build has 282 `vmulps/vaddps`, 0 SSE `mulps/addps`, 16 965 `ymm` references, 0 FMA (`vfmadd`) in both. I.e. by default the solver's hot loops are 128-bit SSE2; `target-cpu=native` (or `-C target-feature=+avx2`) is required to get AVX2 and it pays off ~2x on trees that fit in cache/bandwidth headroom.

Parallelism detail: `PostFlopNode::enable_parallelization()` returns `river == NOT_DEALT`, so children are solved in parallel only above the river deal. A river-start tree is therefore solved on one thread (irrelevant here: 4 ms), a turn-start tree parallelises over the 44 (isomorphism-reduced) river cards and actions.

### 3.4 Variance

Three of ~20 runs were 1.3-2.6x slower than their repeats (FLOP-FAST run 2: 23 s vs 16 s; FLOP-FAST i16 cold: 36 s vs 20 s; FLOP-FULL cold: 110 s vs 42 s). The box had ~15 % background CPU load when sampled and the CPU is a hybrid P/E design with boost clocks, so single measurements are unreliable; the "best of N" numbers above are the ones to plan with, and any production deadline logic should be measured, not assumed.

### 3.5 Bunching (one folded player, complement of a CO open range)

| Spot | `BunchingData::process` | Extra memory | Solve to 0.5 % | Without bunching |
| --- | --- | --- | --- | --- |
| RIVER | 0.12 s | 2.1 MB | 0.162 s (50 it) | 0.004 s (x40) |
| TURN | 0.23 s | 99.8 MB | 4.55 s (60 it) | 0.168 s (x27) |

Usable for turn/river spots; a flop-start tree with bunching was not attempted (would be minutes at this ratio).

## 4. API surface check

All in `postflop_solver::*` (`src/game/interpreter.rs`, `src/game/base.rs`, `src/solver.rs`, `src/utility.rs`, `src/bunching.rs`, `src/file.rs`). "Exercised" = run in the bench or an example on this machine.

| # | Need | Function(s) | Usage / caveats | Status |
| --- | --- | --- | --- | --- |
| i | Root strategy per combo | `game.back_to_root(); game.strategy() -> Vec<f32>` | Length `#actions * #hands`, index `a * n_hands + h`; hand order from `game.private_cards(player)` (`holes_to_strings` for text). Works at any decision node (`play(i)` / `apply_history`). Panics on chance/terminal nodes. | Exercised (river root: 4 actions x 564 hands; opponent node after AllIn: 2 x 463) |
| ii | Per-action EV per combo for the player to act | `game.cache_normalized_weights(); game.expected_values_detail(player) -> Vec<f32>` | Same `[action][hand]` layout when `player == current_player()`; values are absolute chip EV (pot/2 + bets already added); folds report 0. `expected_values(player)` = strategy-weighted per hand, `equity(player)` alongside. Requires `finalize()`d (solved) game and `cache_normalized_weights()` **after every navigation** (`play`, `back_to_root`, `apply_history` invalidate it; forgetting panics with "Normalized weights are not cached"). | Exercised |
| iii | Lock a node's strategy and keep solving | `game.lock_current_strategy(&[f32])`, `unlock_current_strategy()`, `current_locking_strategy()` | Slice is `[action][hand]`; per-hand partial locking (all-non-positive row = hand stays free); normalised per hand. Allowed after `allocate_memory` and any time before `finalize()` (so: run N `solve_step`s, lock, continue). **After `finalize()` nothing can be solved further** - `solve`/`solve_step` panic on a solved game; re-locking means `allocate_memory` again (resets regrets) and re-solving from scratch. | Exercised via `examples/node_locking` |
| iv | Incremental solve / deadline / cancel / progress | `solve_step(&game, iter)`, `compute_exploitability(&game)`, `finalize(&mut game)`; convenience wrapper `solve(&mut game, max_iters, target, print)` | The bench's loop (step, check exploitability every 10 iters, stop on target or deadline, then `finalize`) is exactly what `solve()` does, with a deadline added. Cancellation granularity = one iteration (0.08-0.3 s on flop trees here); no callback or abort inside an iteration; `compute_exploitability` is itself a full tree pass (~1 iteration cost), so check it every 5-10 iterations. Progress = iteration count + last exploitability. | Exercised (all runs) |
| v | Rake | `TreeConfig { rake_rate: f64, rake_cap: f64, .. }` | Rake in chips of the pot with cap in chips; both must be > 0 for `is_raked()`; exploitability then = MES EV - current EV. | Exercised (river, 5 % / cap 3: solved in 4 ms, 40 iters) |
| vi | Start from turn or river | `TreeConfig.initial_state = BoardState::Turn / River` + `CardConfig { turn, river }` (`card_from_str`, `NOT_DEALT`) | Bet-size arrays for earlier streets are ignored. | Exercised |
| vii | Bunching | `BunchingData::new(&[Range; <=4], flop)`, `.process(print)` (or `phase1/2/3` + `*_proceed_by_percent()` with `progress_percent()` for incremental prep), `game.set_bunching_effect(&bd)`, `reset_bunching_effect()`, `memory_usage_bunching()` | Set before `allocate_memory`. `BunchingData` is itself saveable with `save_data_to_file`. Cost: see 3.5. | Exercised |
| viii | Save / load a solved game | `save_data_to_file(&game, memo, path, zstd_level: Option<i32>)`, `load_data_from_file::<PostFlopGame>(path, max_mem: Option<u64>) -> (game, memo)`; stream variants `save_data_into_std_write` / `load_data_from_std_read`; `set_target_storage_mode(BoardState)` + `target_memory_usage()` to drop river (or turn+river) data before saving | Needs `bincode` feature (default); compression needs `zstd` feature (builds here). File size ~= `memory_usage()` (f32 or i16 mode). | Exercised (river: 0.1 MB, save <0.01 s, load 0.01 s, equity identical after reload; `examples/file_io` also OK) |

Also available: `remove_lines()` (delete lines after tree build, e.g. per-runout), `ActionTree::add_line/remove_line`, `available_actions()`, `possible_cards()` (bitmask; isomorphic runouts are merged), `play(usize)`, `history()/apply_history()`, `current_player()`, `total_bet_amount()`, `memory_usage()`, `is_memory_allocated()`. Not available: any callback/observer hooks, async/cancellable solve, warm-starting a solve from a previously finalised game, per-node exploitability, multi-way (2 players only; bunching only accounts for folded players' cards).

## 5. Windows-specific notes

* **Toolchain actually used: `stable-x86_64-pc-windows-gnu`** (MinGW at `~/.local/mingw`). `stable-x86_64-pc-windows-msvc` is installed in rustup, but VS 2022 Community on this machine has **no C++ build tools** (`VC/` contains only `Auxiliary` and `Redist`; no `cl.exe`, `link.exe`, `dumpbin.exe`), so the MSVC target could not be built or compared. Everything (crate, examples, bench with `psapi`/`kernel32` FFI, `zstd-sys` C code) builds fine with the GNU toolchain.
* No `cc`/C dependency in the default feature set; only `--features zstd` compiles C (worked via MinGW gcc).
* Path length: no problems (scratchpad paths ~170 chars, `target/` nested below).
* Threading default: rayon auto-sizes to 24 threads (P-cores, E-cores and HT). 16 threads is as fast; leave the default or set `RAYON_NUM_THREADS` to keep cores free for the rest of the assistant. Rayon's global pool is process-wide, so the solver will share it with any other rayon user in the process.
* AVX2 is **not** used unless you compile with `-C target-cpu=native` / `+avx2` (~2x on FLOP-FAST). No `.cargo/config.toml` in the repo; the CI builds with `RUSTFLAGS: --deny warnings` on Linux only.
* `custom-alloc` feature needs nightly (`allocator_api`) and assumes a single `PostFlopGame` per process; not measured. Stable build allocates per node with the system allocator; `allocate_memory` itself is instant, page-faulting happens during iteration 1.
* Unsafe: `MutexLike` is a `UnsafeCell` wrapper that performs no locking ("extremely unsafe" per its own docs); correctness relies on rayon splitting disjoint children. 100+ `unsafe` sites in `src/`. Rust 1.95's new lint already caught one raw-pointer pattern (section 2); expect more maintenance as the compiler evolves since upstream is unmaintained.
* One process = one solve at a time is the intended model; running two 3.8 GB flop solves concurrently in one process would double memory and fight for bandwidth.

## 6. Bottom line for the live assistant

* River and turn spots at realistic BB-vs-BTN range sizes: **4 ms and 0.17 s** to 0.5 % - effectively free; turn to 0.3 % in 0.22 s, with bunching 4.6 s.
* Flop, one size + one raise per street (Pio-"FAST"-like), 179x264 hands: **~11-13 s (0.5 %) / ~16-18 s (0.3 %) at 970 MB** by default, **~6.2 s / 7.7 s with AVX2** (`target-cpu=native`); 510 MB with 16-bit mode at +25 % time.
* Flop, two sizes + one raise per street: **~42 s to 0.5 % at 3.8 GB** (2.0 GB in 16-bit mode, +5 % time); AVX2 does not help there; one cold run took 110 s.
* Build needs a fork (pin `bincode`/`bincode_derive` to `=2.0.0-rc.3` and a 3-line lint fix); upstream is unmaintained and AGPL-3.0.
* API has everything on the list except async cancellation inside an iteration and continuing a solve after `finalize()`; a caller-side `solve_step` loop gives deadline/progress control at 0.1-0.3 s granularity.

## Addendum: realistic-range flop benchmarks (2026-09-10)

Same build as section 3.3 (commit `9d1509fe` + the two patches, `RUSTFLAGS="-C target-cpu=native"` = AVX2, `--release`, rayon 24 threads), same timing method (wall clock around the `solve_step` loop, exploitability checked every 10 iterations, one process per run, "cold" = first process launch of that spot/board, "warm" = the immediately following identical runs). Raw lines: scratchpad `spike-solver/results-v2.txt` (60 runs); bench source `spike-solver/bench/src/main.rs` (v2). Chip unit is 0.1 bb (pot 55 = 5.5 bb) so that 2.5x raises round sensibly. Background load ~7-22 % CPU from other software during the session.

### A.1 Ranges and trees

Uniform-weight Pio-style strings (dash ranges high-to-low):

| Range | Share | Combos | String |
| --- | --- | --- | --- |
| BTN open (IP in SRP) | 48.7 % | 646 | `22+,A2s+,K2s+,Q2s+,J3s+,T6s+,96s+,86s+,75s+,65s,54s,43s,A2o+,K7o+,Q8o+,J8o+,T8o+,98o` |
| BB defend / call (OOP in SRP; QQ+, AK, AQs assumed 3-bet) | 60.6 % | 804 | `JJ-22,AJs-A2s,K2s+,Q2s+,J2s+,T2s+,92s+,84s+,74s+,63s+,53s+,43s,32s,AJo-A2o,K5o+,Q7o+,J8o+,T8o+,98o,97o,87o,76o` |
| CO open-then-call vs 3-bet (OOP in 3BP) | 14.6 % | 194 | `QQ-22,AKs-ATs,A5s-A4s,KQs-KTs,QJs-QTs,JTs,J9s,T9s,T8s,98s,87s,76s,65s,54s,AQo-AJo,KQo,KJo` |
| BTN 3-bet vs CO (IP in 3BP) | 10.4 % | 138 | `TT+,AJs+,A5s-A2s,KJs+,QJs,JTs,T9s,76s,65s,54s,AQo+,KQo,KJo` |

Spots: **(a)** SRP BTN vs BB 100 bb, pot 5.5 / stack 97.5; **(b)** 3-bet pot CO vs BTN, pot 19 / stack 91; **(c)** SRP 200 bb, pot 5.5 / stack 197.5; **turn** = (a) ranges at 200 bb after a 3 bb flop c-bet is called (Kh7d2c 4d, pot 11.5 / stack 194.5). Boards: `Kh7d2c` (dry rainbow), `Jh9h6c` (wet two-tone), `8s8d3c` (paired). After board removal the SRP spots have 629-647 x 559-572 hands, the 3-bet pot 170-180 x 121-137.

Trees: **FAST** = one bet 55 % pot + one raise 2.5x on every street, `add_allin_threshold 1.0`, `force_allin 0.15`, `merging 0.1`, **no donk bets**; **TWO** = 33 % / 75 % + raise 2.5x on every street, `add_allin 1.5`, otherwise identical. Note on donk bets: `turn_donk_sizes: None` (used in section 3) means "use the default bet sizes", i.e. OOP may lead after calling the previous street; disabling donks needs `Some(DonkSizeOptions { donk: vec![] })`. That alone shrinks spot (a) on K72r from 6.8 GB to 5.3 GB.

### A.2 FAST tree, f32, 24 threads, AVX2 (5 runs per cell; cold = run 1, p50/p95 over runs 2-5)

| Spot | Board | Hands OOP x IP | Cold: 0.5 % / 0.3 % | Warm 0.5 %: p50 (p95) | Warm 0.3 %: p50 (p95) | Iters 0.5 / 0.3 | `memory_usage()` f32 / i16 | Peak WS |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| (a) SRP 100 bb | Kh7d2c | 642 x 572 | **35.1 s / 42.0 s** | 34.7 s (35.0) | 41.6 s (41.8) | 100 / 120 | 5279 / 2680 MB | 5299 MB |
| (a) | Jh9h6c | 629 x 559 | 19.3 s / 25.5 s | 19.4 s (19.5) | 25.8 s (26.0) | 90 / 120 | 3148 / 1600 MB | 3167 MB |
| (a) | 8s8d3c | 637 x 566 | 26.6 s / 35.2 s | 26.5 s (26.5) | 35.2 s (35.3) | 120 / 160 | 3291 / 1673 MB | 3309 MB |
| (b) 3BP | Kh7d2c | 176 x 121 | **6.6 s / 7.9 s** | 6.7 s (6.7) | 8.0 s (8.0) | 100 / 120 | 889 / 471 MB | 900 MB |
| (b) | Jh9h6c | 170 x 125 | 4.3 s / 5.1 s | 4.2 s (4.2) | 5.0 s (5.1) | 100 / 120 | 539 / 286 MB | 550 MB |
| (b) | 8s8d3c | 180 x 137 | 5.0 s / 9.5 s | 5.0 s (5.0) | 9.6 s (16.3 *) | 110 / 210 | 596 / 315 MB | 608 MB |
| (c) SRP 200 bb | Kh7d2c | 642 x 572 | 125.2 s / 149.9 s * | **63.7 s (82.0 *)** | 81.0 s (99.1 *) | 110 / 140 | 8780 / 4454 MB | 8803 MB |
| (c) | Jh9h6c | 629 x 559 | 38.5 s / 48.9 s | 39.2 s (39.3) | 49.6 s (49.6) | 110 / 140 | 5231 / 2656 MB | 5254 MB |
| (c) | 8s8d3c | 637 x 566 | 47.3 s / 61.6 s | 47.5 s (47.6) | 62.0 s (62.2) | 130 / 170 | 5470 / 2777 MB | 5493 MB |

\* Outliers (background load / sustained-load throttling, see A.6): (c) K72r runs 1-2 took 125 s and 86 s, runs 3-6 63.4-64.3 s; (b) 883r run 5 reached 0.3 % at 17.5 s (its 0.5 % time was a normal 5.0 s). Excluding these, run-to-run spread is < 2 %.

Cold vs warm: no meaningful difference. The only cold cost is the first iteration (demand-zero page faulting): 1.12 s cold vs 0.66 s warm on the 5.3 GB tree, 1.70 vs 1.17 s on 8.8 GB, unchanged on < 1 GB trees. There is no file I/O, so OS caching plays no role. Peak working set tracks `memory_usage()` within 0.5 % on every run.

Scaling: per-iteration cost is ~0.066-0.078 s per GB of f32 tree on this box (0.35 s/it at 5.3 GB, 0.58 s/it at 8.8 GB, 0.066 s/it at 0.9 GB), and 90-130 iterations are needed for 0.5 % - i.e. **time to 0.5 % ~ 6.5-8 s per GB of tree**. The dry rainbow board is the worst case on every spot because it has the least suit isomorphism (more distinct turn/river runouts), not because of the ranges. Compared with section 3.1 (179 x 264 hands, 0.96 GB, 6.2 s): the 3.4x larger hand count gives 3.3-5.5x the memory and 3-10x the time.

### A.3 Two-size tree (33 % / 75 % + 2.5x, all-in 150 %), target 0.5 %, 5-minute cap, board Jh9h6c

| Spot | Mode | `memory_usage()` | Peak WS | Result |
| --- | --- | --- | --- | --- |
| (b) 3BP | f32 | 1864 MB | 1875 MB | **18.3 s cold / 18.2 s warm** (130 it), final 0.452 % |
| (a) SRP 100 bb | f32 | 22285 MB | 22306 MB | **not converged at 300 s**: 0.504 % after 195 it (1.0 % at 217.6 s / 140 it); 1.55 s/it, would have crossed 0.5 % at ~310 s |
| (a) SRP 100 bb | i16 | 11301 MB | 11324 MB | **229.6 s** (180 it), 1.27 s/it, flat per-iteration time throughout |
| (c) SRP 200 bb | i16 (f32 would be 34.8 GB, more than the 41 GB free) | 17648 MB | 17671 MB | **not converged at 300 s**: 0.736 % after 156 it (1.0 % at 251 s / 130 it); 1.93 s/it steady, ~350-390 s projected |

The Jh9h6c board is the *smallest* of the three for these spots; the dry K72r two-size trees are 37.4 GB f32 / 19.0 GB i16 for (a) and 58.5 GB / 29.6 GB for (c) and were not run (only ~41 GB of the 64 GB was free). Two-size flop trees at realistic SRP ranges are therefore a 4-7 minute, 11-37 GB job on this machine - offline only.

### A.4 Turn solve, (a) ranges at 200 bb, two-size tree (Kh7d2c 4d, pot 11.5 bb, stack 194.5 bb)

626 x 560 hands, `memory_usage()` 130 MB (66 MB i16), peak WS 148 MB. Three runs: **0.96-0.98 s to 0.5 %** (100 it), 1.23-1.24 s to 0.3 % (130 it). The FAST turn tree is 33 MB. Turn-start solves remain effectively free even at 200 bb with two sizes.

### A.5 16-bit compression (`allocate_memory(true)`) on FAST trees

| Spot / board | f32: 0.5 % / 0.3 %, peak WS | i16: 0.5 % / 0.3 %, peak WS | Delta |
| --- | --- | --- | --- |
| (a) Kh7d2c | 34.7 s / 41.6 s, 5299 MB | 27.0-27.9 s / 36.1-37.0 s, 2699 MB (2 runs) | **-20 % time, -49 % memory** |
| (a) Jh9h6c | 19.4 s / 25.8 s, 3167 MB | 17.1 s / 22.9 s, 1618 MB (1 run) | -12 % time, -49 % memory |
| (c) Kh7d2c | 63.4-64.3 s / 80.3-81.5 s, 8803 MB | 51.8 s / 65.8 s, 4475 MB (1 clean run; a first attempt launched right after a 5-minute full-load run took 174.7 s and is discarded as throttled) | -18 % time, -49 % memory |
| (a) two-size Jh9h6c | 1.55 s/it, 22.3 GB | 1.27 s/it, 11.3 GB | -18 % per iteration |

This reverses the section 3.2 finding (i16 was +25-30 % slower on the 0.96 GB tree): at 3-22 GB the solver is memory-bandwidth bound, so halving the bytes wins despite the encode/decode work. Iteration counts are the same or slightly lower in i16 (quantisation changes the path: 90 vs 100 iterations on (a) K72r); final exploitability differs in the third digit. **Use i16 for any flop tree above ~2 GB.**

### A.6 Variance notes

Within a spot/board, warm runs repeat to < 2 % (p95 == p50 to one decimal) except the flagged outliers. Two slow runs (174.7 s vs 51.8 s; 84 iterations vs 156 in 300 s) happened immediately after a 5-minute all-core AVX2 run, and (c) K72r's first two runs were 2x / 1.35x slow while a WMI process scan ran; a run with `--verbose` per-10-iteration timing showed no drift *within* a 4-5 minute run, so the effect is between processes (clock/thermal state or background load), not the solver. Plan with the p50 and keep a deadline in the `solve_step` loop.

### A.7 Verdict on the 10 s budget

**No.** With realistic preflop ranges the reduced (FAST) flop tree does not meet 10 s on this CPU for single-raised pots, cold or warm:

* (a) SRP 100 bb: **35 s** on the worst board (K72r), 19-27 s on the others, at 3.2-5.3 GB; 27 s / 17 s in i16. Even 1.0 % exploitability takes 28 s on K72r.
* (c) SRP 200 bb: **64 s** worst board (52 s i16), 39-48 s others, 5.2-8.8 GB.
* (b) 3-bet pot (170-180 x 121-137 hands): **4.1-6.7 s to 0.5 %** (p95 6.7 s), 5-8 s to 0.3 % (9.6 s p50 on the paired board), 0.55-0.9 GB - this is the only flop spot that fits, and it fits on all three boards.

Rule of thumb from A.2: a FAST f32 tree must be <= ~1.3-1.5 GB (i16: <= ~2 GB) to finish 0.5 % inside 10 s here, i.e. roughly <= 250 x 250 hands, or the SRP ranges cut to about a third of their combos. Practical options for SRP flops: solve them offline and load (`save_data_to_file`, optionally `set_target_storage_mode(BoardState::Turn)` to keep only flop+turn data), start live solves from the turn (~1 s at 200 bb with two sizes, section A.4), or accept a 20-35 s flop solve with a progress indicator. The two-size flop tree is offline-only at these ranges (4-7 min, 11-37 GB).
