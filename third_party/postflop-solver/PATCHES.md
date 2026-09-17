# Local patches to b-inary/postflop-solver (pinned 9d1509fe5077d019825f833eed04b16d342dfda1, AGPL-3.0-or-later)

1. `Cargo.toml`: `bincode = "=2.0.0-rc.3"` and `bincode_derive = "=2.0.0-rc.3"` (a fresh lockfile resolves 2.0.1, whose derive API breaks `src/mutex_like.rs`).
2. `src/action_tree.rs` lines 393, 396, 408: `&*(*node).children[i].lock()` -> `&*(&(*node).children)[i].lock()` (Rust 1.95 deny-by-default lint `dangerous_implicit_autorefs`).

No other source change. `LICENSE` is upstream's, unchanged. Build with `-C target-feature=+avx2` (workspace `.cargo/config.toml`).

## Added files (no upstream file is modified by these)

* `PINNED_COMMIT` and `PATCHES.md` (this file) - vendoring records.
* `tests/vendor_smoke.rs` and `tests/v1_flop_fast.rs` - PokerAI tests added beside upstream's `tests/kuhn.rs` and `tests/leduc.rs`.
* `Cargo.lock` - force-added (`git add -f`) over upstream's own `.gitignore`, which still excludes it (that `.gitignore` is left unmodified, so the two-source-patch boundary in the heading above stays exactly two). **Provenance: fresh resolution, not the measured one.** Neither the original P2.T1 implementer's worktree (`.claude/worktrees/agent-a1f54804f70026798/third_party/postflop-solver/`) nor this fix worktree ever held a `Cargo.lock` for this crate — the V1 measurement in the section below ran against an unlocked resolve, and that exact graph was never captured. This lockfile was generated with `cargo generate-lockfile` on 2026-09-17 against the same `Cargo.toml` (patch 1 applied), resolving: `bincode 2.0.0-rc.3`, `bincode_derive 2.0.0-rc.3` (both pinned exactly, as patch 1 requires), `once_cell 1.21.4`, `rayon 1.12.0`, `regex 1.13.1`, `zstd 0.12.4`, 34 packages total. It fixes the graph going forward; it does not retroactively reconstruct the graph the V1 numbers below ran against. Standalone commands from here on use `--locked` against this file so the graph cannot silently drift again.
* Dropped from the upstream checkout when vendoring: `.git/`, `target/` (upstream's own `.gitignore` already excludes both, plus `Cargo.lock` — see above for why `Cargo.lock` is force-added anyway).

## Reproducing (standalone)

The vendored crate is excluded from the workspace (see the root `Cargo.toml`) and resolves its own dependency graph via this directory's `Cargo.lock`. Standalone commands should pass `--locked` so that graph cannot drift:

```powershell
cargo test --locked --release --test vendor_smoke --manifest-path third_party/postflop-solver/Cargo.toml
cargo test --locked --release --test v1_flop_fast --manifest-path third_party/postflop-solver/Cargo.toml -- --ignored --nocapture
```

(Run from inside `third_party/postflop-solver/` with a bare `--manifest-path Cargo.toml`, or via a non-nested checkout — a worktree nested under the repo root defeats cargo's workspace-exclude discovery; see the P2.T1 fix report for the workaround used in this session.)

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

The future solver-worker's reproducibility comes from the ROOT Cargo.lock once plan 2 Task 7 adds it to the workspace; worker build/test commands use the root resolution with `--locked`. This vendored crate's own `Cargo.lock` (see "Added files" above) only pins the standalone `vendor_smoke`/`v1_flop_fast` graph, since the vendored crate stays excluded from the workspace and is never a dependency's dependent — it does not, by itself, control `solver-worker`'s resolution.
