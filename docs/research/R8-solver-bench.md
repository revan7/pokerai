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
