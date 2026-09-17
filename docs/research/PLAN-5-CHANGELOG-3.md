# Plan 5 revision 3 — changelog (2026-09-17)

Plan: `docs/superpowers/plans/2026-09-10-plan-5-ui.md`.
Input: `docs/research/REVIEW-cross-plan-3.md` section 3 — **R4** (F15 dependency ownership), **R5** (F16 selected-worker propagation, plan 5's share), **R6** (F18 final staging cleanup), **R7** (F22 reference cleanup, the plan-5 items); and section 5's stale-reference disposition, which names "all five L13 spec-version labels" and "P5:L13 old façade citation" as actionable leftovers.

Size before: 3,183 lines, 14 tasks. No task was renumbered, no task was added or removed. Resulting revision: **3**.

---

## 1. Edits applied

| # | Finding | Location | Disposition | What changed |
|---|---|---|---|---|
| R4-1 | F15 — wrong dependency ownership | Task 1, Step 1 (L171) | **APPLIED** | "F15: Plan 1's `[workspace.dependencies]` table does not declare `ts-rs`, and Plan 5 is its first consumer — add `ts-rs = "=12.0.1"` to that root table in this same step; `crates/proto`'s manifest then inherits it via `workspace = true` in Step 4 below." replaced verbatim with "F15: Plan 1 Task 1 already declares `ts-rs = "=12.0.1"` in `[workspace.dependencies]`. Preserve that single root declaration; `crates/proto` inherits it with `workspace = true` in Step 4, as `cache` already does in Plan 4 Task 1." Step 1 no longer instructs adding a second root declaration. Verified against plan 1: Task 1's Global Constraints line and File-structure `Cargo.toml` fragment (`ts-rs = "=12.0.1"`) already declare it at the root. |
| R5-1 | F16 — unconditional MSVC worker build ignores the V1 selection | Task 5, Step 6 heading and script | **APPLIED** | Step 6 renamed "Stage the MSVC runtime and the available charts." → "Stage the selected worker and available charts." The unconditional `cargo +stable-x86_64-pc-windows-msvc build --release -p solver-worker --target x86_64-pc-windows-msvc` / `if ($LASTEXITCODE -ne 0) { throw 'worker build failed' }` pair is replaced by a selection-driven `if`/`elseif`/`else` block that reads `docs/bench/worker-toolchain.json`, builds through `scripts/build-worker-gnu.ps1` on the GNU selection or the MSVC cargo invocation on the MSVC selection, throws `'missing or invalid V1 worker-toolchain selection'` otherwise, and sets `$workerBinary` to the selected exe path. The subsequent `Copy-Item -LiteralPath (Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/solver-worker.exe') -Destination $stage` is replaced with `Copy-Item -LiteralPath $workerBinary -Destination $stage`. UI/app target directories (`target/x86_64-pc-windows-msvc/$profileDir/runtime`) are unchanged — the app itself always stays MSVC, only the worker binary follows the V1 selection. |
| R6-1 | F18 — final staged preflop directories never clear a bundle that became unsupported | Task 5, Step 6 script, `foreach ($profileDir in @('debug','release'))` loop | **APPLIED** | Before each final `New-Item -ItemType Directory -Force -Path (Join-Path $exeRuntime 'preflop')`, inserted a path-bounded cleanup: computes `$runtimePreflop`/`$targetRoot` with `[IO.Path]::GetFullPath`, throws `'runtime preflop path escapes project target directory'` if `$runtimePreflop` is not under `$targetRoot`, then removes `$runtimePreflop` recursively if it exists, before recreating it fresh. |
| R6-2 | F18 — missing verification for the cleanup | Task 5, Step 6 narrative (end of the paragraph following the script) | **APPLIED** | Appended: "**Verification:** stage with 200bb available, change its manifest status to unsupported, stage again, and verify its bundle/manifest pair is absent from `target/pokerai-runtime/preflop` and both executable `runtime/preflop` directories; the available 100bb pair remains." |
| R7-1 | F22 — stale "revision 6" spec-version label | Header, L13 (`**Spec:**` line) | **APPLIED** | "…pokerai-assistant-design.md\`, revision 6 (amendments S1–S17…" → "…, revision 7 (amendments S1–S17…" (only the version number; the historical amendment description is preserved unchanged, matching the same treatment given plans 2 and 4). |
| R7-2 | F22 — old façade citation | Header, L13 (`**Spec:**` line, same sentence) | **APPLIED** | "`docs/research/REVIEW-cross-plan.md` §§1, 2, 4, 5 for the resolved engine façade." replaced with "`docs/research/REVIEW-cross-plan-2.md` §4.2 for the engine façade; its §7 and EXECUTION-ORDER.md supply the task order. REVIEW-cross-plan.md §§4–5 are historical." |
| — | Title / revision line | Lines 1-4 | **APPLIED** | Added "Revision 3 (2026-09-17): verification edits R4, R5, R6, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-5-CHANGELOG-3.md" directly under the title, above the existing Revision 2 line (kept unchanged). |

## 2. Exact lines changed (before / after)

### R4 — Task 1, Step 1

Before:
> F15: Plan 1's `[workspace.dependencies]` table does not declare `ts-rs`, and Plan 5 is its first consumer — add `ts-rs = "=12.0.1"` to that root table in this same step; `crates/proto`'s manifest then inherits it via `workspace = true` in Step 4 below.

After:
> F15: Plan 1 Task 1 already declares `ts-rs = "=12.0.1"` in `[workspace.dependencies]`. Preserve that single root declaration; `crates/proto` inherits it with `workspace = true` in Step 4, as `cache` already does in Plan 4 Task 1.

### R5 — Task 5, Step 6 heading

Before: `- [ ] **Step 6 (4 min): Stage the MSVC runtime and the available charts.**`
After: `- [ ] **Step 6 (4 min): Stage the selected worker and available charts.**`

### R5 — Task 5, Step 6 script (worker build + copy)

Before:
```powershell
try {
    cargo +stable-x86_64-pc-windows-msvc build --release -p solver-worker --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'worker build failed' }
    $stage = Join-Path $projectRoot 'target/pokerai-runtime'
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path (Join-Path $stage 'preflop') | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/solver-worker.exe') -Destination $stage
```

After:
```powershell
try {
    $selection = Get-Content -LiteralPath 'docs/bench/worker-toolchain.json' -Raw | ConvertFrom-Json
    if ($selection.worker_toolchain -eq 'stable-x86_64-pc-windows-gnu' -and
        $selection.worker_target -eq 'x86_64-pc-windows-gnu') {
        powershell -NoProfile -File scripts/build-worker-gnu.ps1
        if ($LASTEXITCODE -ne 0) { throw 'GNU worker build failed' }
        $workerBinary = Join-Path $projectRoot 'target/x86_64-pc-windows-gnu/release/solver-worker.exe'
    } elseif ($selection.worker_toolchain -eq 'stable-x86_64-pc-windows-msvc' -and
              $selection.worker_target -eq 'x86_64-pc-windows-msvc') {
        cargo +stable-x86_64-pc-windows-msvc build --release -p solver-worker --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw 'MSVC worker build failed' }
        $workerBinary = Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/solver-worker.exe'
    } else { throw 'missing or invalid V1 worker-toolchain selection' }
    $stage = Join-Path $projectRoot 'target/pokerai-runtime'
    if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
    New-Item -ItemType Directory -Force -Path (Join-Path $stage 'preflop') | Out-Null
    Copy-Item -LiteralPath $workerBinary -Destination $stage
```

### R6 — Task 5, Step 6 script (final staging loop)

Before:
```powershell
    foreach ($profileDir in @('debug','release')) {
        $exeRuntime = Join-Path $projectRoot "target/x86_64-pc-windows-msvc/$profileDir/runtime"
        New-Item -ItemType Directory -Force -Path (Join-Path $exeRuntime 'preflop') | Out-Null
```

After:
```powershell
    foreach ($profileDir in @('debug','release')) {
        $exeRuntime = Join-Path $projectRoot "target/x86_64-pc-windows-msvc/$profileDir/runtime"
        $runtimePreflop = [IO.Path]::GetFullPath((Join-Path $exeRuntime 'preflop'))
        $targetRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target')).TrimEnd('\') + '\'
        if (-not $runtimePreflop.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'runtime preflop path escapes project target directory'
        }
        if (Test-Path -LiteralPath $runtimePreflop) {
            Remove-Item -LiteralPath $runtimePreflop -Recurse -Force
        }
        New-Item -ItemType Directory -Force -Path (Join-Path $exeRuntime 'preflop') | Out-Null
```

### R7 — header L13

Before:
> **Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 6 (amendments S1–S17 from `docs/research/REVIEW-cross-plan.md` §3, plus the §13.2 fixture correction), especially §§2, 3.1–3.6, 4.1–4.4, 5, 5.1, 6, 7, 10.5, 12, 13.4, 14.4. Also read `docs/design/2026-09-10-design-outline.md` §0b, `docs/research/R8-solver-bench.md`, `docs/research/R5-app-stack.md`, and `docs/research/REVIEW-cross-plan.md` §§1, 2, 4, 5 for the resolved engine façade. Date: 2026-09-10. …

After:
> **Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 7 (amendments S1–S17 from `docs/research/REVIEW-cross-plan.md` §3, plus the §13.2 fixture correction), especially §§2, 3.1–3.6, 4.1–4.4, 5, 5.1, 6, 7, 10.5, 12, 13.4, 14.4. Also read `docs/design/2026-09-10-design-outline.md` §0b, `docs/research/R8-solver-bench.md`, `docs/research/R5-app-stack.md`, and `docs/research/REVIEW-cross-plan-2.md` §4.2 for the engine façade; its §7 and EXECUTION-ORDER.md supply the task order. REVIEW-cross-plan.md §§4–5 are historical. Date: 2026-09-10. …

### Title / revision line

Before:
> \# Plan 5: Tauri UI and E2E Implementation Plan
>
> Revision 2 (2026-09-17): seam re-check edits E05/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-5-CHANGELOG-2.md

After:
> \# Plan 5: Tauri UI and E2E Implementation Plan
>
> Revision 3 (2026-09-17): verification edits R4, R5, R6, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-5-CHANGELOG-3.md
>
> Revision 2 (2026-09-17): seam re-check edits E05/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-5-CHANGELOG-2.md

## 3. Not changed

- Task 14's E2E build note ("it now inherits `runtime/` from this same script via `beforeBuildCommand`") is unchanged — it already deferred to Step 6, which is what was edited.
- `bundle.resources`/`bundle.active` (MA-11) staging-declarative text is unchanged; only the worker build/copy and the final-directory cleanup were touched.
- No other section, task or fixture list in plan 5 was touched.

## 4. Verification

Documentation-only task: Rust/Python/UI execution: not applicable — no workspace exists. Verification is the grep check named in the task: every finding's cited old text is absent from the plan and the replacement is present. The quoted command output is recorded in the journal entry `docs/log/2026-09-17.md` and in the agent's report.
