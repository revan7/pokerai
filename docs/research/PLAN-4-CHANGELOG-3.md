# Plan 4 revision 3 — changelog (2026-09-17)

Plan: `docs/superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md`.
Input: `docs/research/REVIEW-cross-plan-3.md` section 3 — **R2** (F12 / new extraction seam, plan 4's share), **R3** (F13 loader-signature mismatch), **R5** (F16 selected-worker propagation, plan 4's share), **R7** (F22 reference cleanup, the plan-4 item); and section 5's stale-reference disposition, which names "all five L13 spec-version labels" as an actionable leftover (plan 4's L13 is one of the five).

Size before: 4,250 lines, 26 tasks. No task was renumbered, no task was added or removed. Resulting revision: **3**.

---

## 1. Edits applied

| # | Finding | Location | Disposition | What changed |
|---|---|---|---|---|
| R2-1 | F12 — no declared signature | Task 11, Interfaces (L2006) | **APPLIED** | The vague "and in `solve.rs` the shared internal transport function extracted from `run_solve` (see Step 4)" now names both functions and points to their exact `pub(crate)` signatures declared in Step 4. |
| R2-2 | F12 — no declared signature | Task 11, Step 4 | **APPLIED** | Inserted the exact contract block before the `run_surrogate` implementation: the full `solve_request_from_parts(core: &mut EngineCore, root: &proto::StreetRootSnapshot, ranges: &[proto::Range1326; 2], target_bp: u16, plan: &SolvePlan, build: &crate::tree::TreeBuild, deadline_ms: u32) -> proto::worker::SolveRequest` and `send_solve_request(core: &mut EngineCore, request: proto::worker::SolveRequest, plan: &SolvePlan, build: &crate::tree::TreeBuild, sink: &SharedSink) -> SolveOutcome` declarations, plus the ownership/behavior-preservation sentence from R2 verbatim. |
| R2-3 | F12 — illustrated calls lack DecisionIdentity/Deadlines/id-allocator/memory-limit | Task 11, Step 4, `run_surrogate` body | **APPLIED** | Replaced `let request=crate::solve::solve_request_from_parts(&root,&ranges,&build.tree,target_bp, rake.clone(),deadlines,identity); let out=crate::solve::send_solve_request(core,request,input.hero_role,sink);` with a `SolvePlan` construction (identity/deadlines/rake/hero role from the surrogate's own inputs, `template_id` from `input`, `retry_template_id: None`, `background: false`), a `deadline_ms` computed via `plan.deadlines.worker_deadline_ms(core.clock.now_ms(), plan.deadlines.street_deadline_ms)?`, and calls matching the new signatures' exact argument order. |
| R3-1 | F13 — Interfaces/impl signature mismatch | Task 9, Interfaces (L1456) | **APPLIED** | `load_v3_policy(path:&Path,expected_provenance:&Provenance)->FlopPolicy` replaced with `load_v3_policy(path:&std::path::Path,expected:&V3PolicyEvidence)->FlopPolicy`, matching Step 3's already-correct implementation at (then) L1580 verbatim. No implementation change — Step 3 was already right. |
| R5-1 | F16 — GNU `--target` placed before `test` | Task 23, Step 3a, `run_oracles` body | **APPLIED** | `.args(worker_toolchain_args(suite)).args(suite.args).output();` → `.args(worker_toolchain_args(suite)).output();` — the helper now returns the complete argument vector, called once. |
| R5-2 | F16 — `worker_toolchain_args` contract undeclared | Task 23, Step 3a, prose after the `run_oracles` code block | **APPLIED** | Replaced the "adding `+<toolchain>` and `--target <target>` on the GNU branch" description with the exact contract: `worker_toolchain_args(suite: &OracleSuite) -> Vec<&'static str>` reads/validates the V1 selection and returns the **complete** Cargo argument vector; for the GNU worker suite it returns `["+stable-x86_64-pc-windows-gnu", "test", "-p", "solver-worker", "--release", "--target", "x86_64-pc-windows-gnu", "--test", "contract_river"]`; otherwise `suite.args` unchanged; `--target` must follow `test`, never precede it. |
| R7-1 | F22 — stale "revision 6" spec-version label | Header, L13 (`**Spec:**` line) | **APPLIED** | `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md\`, revision 6,` → `…, revision 7,` (the spec's current revision at the time this review was written; the spec is bumped to revision 8 concurrently by task B3h's own R1 edit — see `SPEC-CHANGELOG-7.md` — but R7's text names revision 7 exactly, so that is what is applied here per the binding instruction to apply the edits exactly as written). |
| — | Title / revision line | Lines 1-4 | **APPLIED** | Added "Revision 3 (2026-09-17): verification edits R2, R3, R5, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-4-CHANGELOG-3.md" directly under the title, above the existing Revision 2 line (kept unchanged). |

## 2. Exact lines changed (before / after)

### R2 — Task 11 Interfaces (before L2006)

Before (fragment):
> …Produces `experimental::{…}`; and in `solve.rs` the shared internal transport function extracted from `run_solve` (see Step 4); and the `crates/engine/tests/support/mod.rs` helpers…

After:
> …Produces `experimental::{…}`; and in `solve.rs` the shared internal transport extracted from `run_solve`, `solve_request_from_parts` and `send_solve_request`, with the exact `pub(crate)` signatures given in Step 4; and the `crates/engine/tests/support/mod.rs` helpers…

### R2 — Task 11 Step 4 (contract block inserted before the `run_surrogate` code)

After (new):
```rust
pub(crate) fn solve_request_from_parts(
    core: &mut EngineCore,
    root: &proto::StreetRootSnapshot,
    ranges: &[proto::Range1326; 2],
    target_bp: u16,
    plan: &SolvePlan,
    build: &crate::tree::TreeBuild,
    deadline_ms: u32,
) -> proto::worker::SolveRequest;

pub(crate) fn send_solve_request(
    core: &mut EngineCore,
    request: proto::worker::SolveRequest,
    plan: &SolvePlan,
    build: &crate::tree::TreeBuild,
    sink: &SharedSink,
) -> SolveOutcome;
```
Plan 4 Task 11 owns these crate-visible extractions. The builder preserves Plan 2's request-id allocation, spot hash, memory limit, rake, history and background flag without constructing SolveInput. The sender shares one-attempt transport, whole-solution validation, identity checks, heartbeat, cancellation and absolute deadlines. Keep main-path requested-node/hero-actor checks and Task 23 retry/admission policy in run_solve; the surrogate validates its separately selected root/check-child advice row and skips on failure.

### R2 — Task 11 Step 4, `run_surrogate` call sites

Before:
```rust
let request=crate::solve::solve_request_from_parts(&root,&ranges,&build.tree,target_bp,
    rake.clone(),deadlines,identity);
let out=crate::solve::send_solve_request(core,request,input.hero_role,sink);
```

After:
```rust
let plan=SolvePlan{identity:identity.clone(),deadlines:deadlines.clone(),
    template_id:input.template_id.clone(),retry_template_id:None,
    rake:rake.clone(),hero_actor:input.hero_role.into(),background:false};
let deadline_ms=plan.deadlines.worker_deadline_ms(core.clock.now_ms(),plan.deadlines.street_deadline_ms)?;
let request=crate::solve::solve_request_from_parts(core,&root,&ranges,target_bp,
    &plan,&build,deadline_ms);
let out=crate::solve::send_solve_request(core,request,&plan,&build,sink);
```

### R3 — Task 9 Interfaces

Before:
> `flop::{V3PolicyEvidence, load_v3_policy(path:&Path,expected_provenance:&Provenance)->FlopPolicy}`

After:
> `flop::{V3PolicyEvidence, load_v3_policy(path:&std::path::Path,expected:&V3PolicyEvidence)->FlopPolicy}`

### R5 — Task 23 Step 3a

Before:
```rust
let output=std::process::Command::new(suite.program)
    .args(worker_toolchain_args(suite)).args(suite.args).output();
```
and prose: "Dispatch the `solver-worker` command through the V1-selected toolchain/target (`worker_toolchain_args` reads `docs/bench/worker-toolchain.json`, adding `+<toolchain>` and `--target <target>` on the GNU branch and nothing on the MSVC branch)…"

After:
```rust
let output=std::process::Command::new(suite.program)
    .args(worker_toolchain_args(suite)).output();
```
and prose: "Dispatch every suite through `worker_toolchain_args(suite: &OracleSuite) -> Vec<&'static str>`, which reads and validates the V1 selection from `docs/bench/worker-toolchain.json` and returns the complete Cargo argument vector for that suite; call `.args(worker_toolchain_args(suite))` once, without appending `suite.args` again. For the GNU worker suite it returns `["+stable-x86_64-pc-windows-gnu", "test", "-p", "solver-worker", "--release", "--target", "x86_64-pc-windows-gnu", "--test", "contract_river"]`; otherwise it returns `suite.args` unchanged — `--target` must follow the `test` subcommand, never precede it."

### R7 — header L13

Before:
> **Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 6, §§2, 3.2–3.5, …

After:
> **Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 7, §§2, 3.2–3.5, …

### Title / revision line

Before:
> \# Plan 4: Flop path, cache and pre-solver Implementation Plan
>
> Revision 2 (2026-09-17): seam re-check edits E04/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-4-CHANGELOG-2.md

After:
> \# Plan 4: Flop path, cache and pre-solver Implementation Plan
>
> Revision 3 (2026-09-17): verification edits R2, R3, R5, R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-4-CHANGELOG-3.md
>
> Revision 2 (2026-09-17): seam re-check edits E04/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-4-CHANGELOG-2.md

## 3. Not changed

- R4 and R6 are plan-5-only edits; no plan 4 text corresponds to them.
- Task 9 Step 3's implementation of `load_v3_policy` was already correct (`path:&std::path::Path,expected:&V3PolicyEvidence`); only the stale Interfaces declaration was wrong, and only that line was touched.
- No other section, task or fixture list in plan 4 was touched.

## 4. Verification

Documentation-only task: Rust/Python/UI execution: not applicable — no workspace exists. Verification is the grep check named in the task: every finding's cited old text is absent from the plan and the replacement is present. The quoted command output is recorded in the journal entry `docs/log/2026-09-17.md` and in the agent's report.
