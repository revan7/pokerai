# Local patches to b-inary/postflop-solver (pinned 9d1509fe5077d019825f833eed04b16d342dfda1, AGPL-3.0-or-later)

1. `Cargo.toml`: `bincode = "=2.0.0-rc.3"` and `bincode_derive = "=2.0.0-rc.3"` (a fresh lockfile resolves 2.0.1, whose derive API breaks `src/mutex_like.rs`).
2. `src/action_tree.rs` lines 393, 396, 408: `&*(*node).children[i].lock()` -> `&*(&(*node).children)[i].lock()` (Rust 1.95 deny-by-default lint `dangerous_implicit_autorefs`).

No other source change. `LICENSE` is upstream's, unchanged. Build with `-C target-feature=+avx2` (workspace `.cargo/config.toml`).

## Added files (no upstream file is modified by these)

* `PINNED_COMMIT` and `PATCHES.md` (this file) - vendoring records.
* `tests/vendor_smoke.rs` and `tests/v1_flop_fast.rs` - PokerAI tests added beside upstream's `tests/kuhn.rs` and `tests/leduc.rs`.
* Dropped from the upstream checkout when vendoring: `.git/`, `target/`, `Cargo.lock` (upstream's own `.gitignore` already excludes the last two).

## Toolchain

Spec 3.6 V1 check, performed 2026-09-17 on the R8 machine (Intel i7-13700K, 8P+8E / 24 logical threads, 64 GB, Windows 11 Pro 10.0.26200).

| Item | Value |
| --- | --- |
| `rustc -Vv` | `rustc 1.95.0 (59807616e 2026-04-14)`, commit-hash `59807616e1fa2540724bfbac14d7976d7e4a3860`, commit-date 2026-04-14, release 1.95.0, LLVM 22.1.2; hosts `x86_64-pc-windows-msvc` and `x86_64-pc-windows-gnu` both installed at that version |
| Selected toolchain / target | `stable-x86_64-pc-windows-msvc` / `x86_64-pc-windows-msvc` |
| Solver commit | `9d1509fe5077d019825f833eed04b16d342dfda1` |
| Flags | `-C target-feature=+avx2` (workspace `.cargo/config.toml`, both targets), `--release` |
| Workload | R8 FLOP-FAST: `QsJh2h`, flop root, pot 180, eff. stack 910, 52 % bets / 2.5x raises on every street, `add_allin 1.0`, `force_allin 0.15`, `merging 0.1`, no donk options, `examples/basic.rs` ranges (OOP 179 / IP 264 combos), target 0.5 % of pot, exploitability checked every 10 iterations (`tests/v1_flop_fast.rs`) |
| `msvc_secs` | **5.418 s** (best of three: 5.446 / 5.482 / 5.418; 80 iterations, exploitability 0.8091 chips = 0.450 % of pot, `memory_usage()` 1 004 421 952 B = 957.9 MB) |
| R8 GNU comparator | 6.2 s (R8-solver-bench.md section 3.3, `-C target-cpu=native` (AVX2), 24 threads, 0.5 % target) |
| Ratio | 5.418 / 6.2 = **0.874** (threshold 1.25) |
| Decision | MSVC is within 25 % of the GNU comparator - `solver-worker` is built with MSVC like the rest of the workspace; `scripts/build-worker-gnu.ps1` stays unused. |

Cross-check run on this machine with `stable-x86_64-pc-windows-gnu` / `x86_64-pc-windows-gnu`, same flags and workload: best of three 5.631 s (5.631 / 5.670 / 5.699), identical 80 iterations, identical 0.8091 chips exploitability and identical 957.9 MB - i.e. MSVC is 3.8 % faster than GNU here, and the R8 6.2 s figure is a slightly slower sample of the same GNU workload.

The machine-readable copy of this selection is `docs/bench/worker-toolchain.json`; that file, not this one, is what later worker builds, tests, bench runs, oracle runs and staging steps read.
