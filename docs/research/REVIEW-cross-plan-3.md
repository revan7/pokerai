# Phase B targeted seam verification — 2026-09-17

**READY FOR EXECUTION: YES WITH THE LISTED EDITS.**
**F01–F24: 17 RESOLVED / 7 PARTIAL / 0 NOT RESOLVED.** Checks completed: 6/6. Seven remaining edit groups below; no unresolved authority decision or environmental blocker.

Task: none. Read CLAUDE.md, AGENTS.md, INDEX, the latest dated log, REVIEW-cross-plan-2, and all six requested changelogs. Checked only the named seams in spec revision 7, Plans 1–5 revision 2, and EXECUTION-ORDER. This verdict is conditional on applying the edits; it does not approve the current text unchanged or claim implementation tests passed. Only this report is written; the explicit report-only instruction overrides the normal journal requirement. No commit.

P1–P5 mean the five dated plans under [plans](../superpowers/plans/); S means the [spec](../superpowers/specs/2026-09-10-pokerai-assistant-design.md). Locations are current Markdown lines. S1 means Step 1; I means Interfaces. Each evidence quotation below is at most 25 words.

## 1. F20: authority is settled; one row still needs qualification

The intended rule and Plan 2's tests agree. The spec's generic rejection row remains overbroad, so F20 is **PARTIAL**, with a small wording correction rather than another authority decision.

| Location | Exact evidence |
|---|---|
| S§4.5:L304 | “Structurally invalid input” and “ack{rejected, reason}” |
| S§4.6:L358 | “a `None` donk option is legal only at `root_street`” |
| S§4.6:L358 | “the same `None` for a later street (turn or river) is invalid input, not a tree mismatch” |
| S§13.2:L735 | “A deliberately altered `materialized` entry and a missing terminal marker each produce `result{error{tree_mismatch}}`” |
| S§13.2:L746 — residual | “`None` donk option in `tree`: typed rejection with `reason`, no work, worker stays alive” |
| P2.T8/I:L2129 | “A later-street `None` never produces `tree_mismatch` on the wire” |
| P2.T8/S1:L2195 | “let e = tree_config(&none_donk, c.pot, c.eff, 0.0, 0).unwrap_err();” |
| P2.T8/S1:L2205 | “tree_config(&c.tree, c.pot, c.eff, 0.0, 0).expect("root-street None donk is legal");” |
| P2.T13/S1:L3151 | “assert_eq!(a["status"], "rejected");” |
| P2.T13/S1:L3153 | “assert!(w.recv_until(S, \|m\| m["type"] == "result" && m["id"] == "5").is_none(), "a rejected request does no work");” |
| P2.T13/S1:L3156 | “assert_eq!(ack_of(&w, "6")["status"], "accepted");” |
| P2.T16/S1:L3881 | “assert_eq!(result_of(&w, "100")["error"]["code"], "tree_mismatch");” |
| P2.T16/S1:L3900 | “assert!(root_none["status"] == "ok" \|\| root_none["status"] == "best_so_far", "{root_none}");” |

Task 13 mutates the turn menu of a flop-rooted request (L3149), then sends an accepted root-None request to the same worker (L3155), covering rejection/no result/liveness. Task 16 tests altered materialization, missing terminal marker and per-street-reset materialization separately (L3880–3892). Task 8 also cross-checks the accepted root tree (L2206–2208). Section 4.5 supplies the general invalid-input category; section 4.6 specifies pre-admission rejection for this case.

One secondary literal mismatch remains: S:L358 gives reason "donk option missing for <street>", while P2:L2128/L3283 emits "<Street> donk option must be the explicit empty list, never None". Align that text in edit R1.

## 2. Finding-by-finding disposition

Producer and consumer quotations are separate; an unchanged correct producer counts as evidence, not as an applied edit.

| Finding | Status | Producer / authority evidence | Consumer evidence / residual |
|---|---|---|---|
| F01 Paths | RESOLVED | P2.T29/I:L6570 “pub struct Paths { pub log_dir: PathBuf, pub worker_exe: PathBuf, pub preflop: PathBuf, pub cache: PathBuf }” | P3.T17/I:L2704 “Paths { log_dir, worker_exe, preflop, cache }”; P4.T7/S5a:L1301 “Paths { log_dir, worker_exe, preflop, cache }”; P5.T5/S4:L1126 “log_dir: local”. P5:L1124 uses worker_exe; the log value is a directory. |
| F02 startup | RESOLVED | P2.T29/I:L6574 “pub fn new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError>;”; P2.T29/I:L6576 “pub fn startup_report(&self) -> StartupReport;” | P3.T17/S3:L2764 “append every loader warning to `engine.startup.banners`”; P4.T7/S5a:L1301 “`Engine::new` keeps returning `Result<Engine, EngineError>`”; P5.T5/S5a:L1227 “warnings.extend(engine.startup_report().banners);” |
| F03 BeginHand | RESOLVED | P2.T29/I:L6578 “pub fn begin_hand(&mut self, req: proto::BeginHand) -> Result<HandState, EngineError>;” | P5.T4/S3:L931 “encode_hand(e.begin_hand(b).map_err(engine_error)?)”. Shell conversion removed; engine owns identity. |
| F04 templates | RESOLVED | P2.T2/S6:L586 “pub fn with_extra(extra: &[TemplateSpec])”; P2.T2/S1:L336 “test-templates = []” | P4.T8/S1a:L1350 “Templates::with_extra(&[”; P4.T8/S1a:L1330 “#[cfg(any(test, feature = "test-templates"))]”. base_ids remains nine; test commands enable testing,test-templates. |
| F05 equity owner | RESOLVED | P2.T25/S2:L5767 “-> Option<(f32, EquityMethod)>”; exactly one range_vs_range definition across the plans. | P4.T11/S3:L2095 “let Some((equity,_method))=crate::equity::range_vs_range(”. P3.T17/S3:L2762 “equity arrives afterwards through the existing Equity event path.” P3 has no direct range_vs_range reference or duplicate; it retains the existing engine path and supplies replayed ranges. |
| F06 shared ranges | RESOLVED | P2.T27/S4:L6243 “pub range_source: Arc<Mutex<Box<dyn RangeSource>>>,” | P3.T18/S3:L2925 “*core.range_source.lock().unwrap() = Box::new(ReplayRanges { store, snapshots, identity })”. Installation precedes engine-main; four-argument constructor retained. |
| F07 emit | RESOLVED | P2.T28/S3:L6421 “pub(crate) fn emit(core: &EngineCore, req: &LiveRequest, delivered: Option<&AtomicBool>, ev: RecommendationEvent)” | P3.T17/S3:L2764 “Import `crate::serve::emit` in `preflop.rs`”; Interfaces also consumes emit. |
| F08 config | RESOLVED | P2.T29/S4:L6851 “if self.state.is_some() { self.queued_config = Some(stamped); } else { self.apply_config(stamped); }” | P4.T9/S3:L1568 “Preserve Plan 2 Task 29's `set_config` and `apply_config` bodies”; P4.T10/S4:L1788 “let target_bp=config.solver.target_bp;”. Queued config getter at L1564. |
| F09 presolver locking | RESOLVED | P2.T29/S4:L6834 “serve_request(&mut c2.lock().unwrap(), req);” establishes the solve-owner mutex. | P4.T16/S4:L3047 “pub(crate) presolver: Option<std::sync::Arc<cache::presolver::scheduler::Presolver>>,” lives on Engine; methods use that handle. Running-solve command regression at L2988–3003. |
| F10 presolver API | RESOLVED | P4.T15/S3:L2758 “pub use scheduler::PresolverStatus;”; P4.T15/S3:L2762 “pub fn remaining_seconds(pending:u32,measured_p50:Option<f64>)->Option<f64>”; P4.T16 owns Engine delegation. | P5 header:L84 “Required from Plan 4 Task 16, with status re-exported by Task 15”; P2.T29/I:L6593 “Plan 4 Task 15 owns `cache::presolver::PresolverStatus`”. Forward-owner note assigns all three methods to P4.T16. |
| F11 foundation APIs | RESOLVED | P1.T3/S3:L493 “pub struct UtgStraddle { pub amount_chips: u32 }”; P1.T9/S3:L1850 “pub fn postflop_order(button: Seat, dealt: &[Seat]) -> Vec<Seat>”; P1.T21/S3:L4304 “pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);”; P1.T21/S3:L4326 “pub fn inverse(p: &SuitPerm) -> SuitPerm” | P4.T7/S5:L1264 “inverse_perm:core_iso::inverse(perm),budget})”; P4.T9/S3:L1545 “state.config.straddle.as_ref().map_or(state.config.bb_chips,\|s\|s.amount_chips)”; P4.T10/S4:L1790 “let mut perm=core_iso::SuitPerm::IDENTITY;”; P4.T11/S3:L2112 “let order=core_model::postflop_order(state.button,&state.dealt);” |
| F12 surrogate | PARTIAL | S§6:L425 “It never goes through `SolveInput`, the cache or replay snapshots”; P2.T23/S4:L5431 “pub fn run_solve(core: &mut EngineCore, input: &SolveInput, plan: &SolvePlan, sink: &SharedSink) -> SolveOutcome” | P4.T11/S4:L2143 “let request=crate::solve::solve_request_from_parts(&root,&ranges,&build.tree,target_bp,”; L2155–2162 use actual combo and real BB. Separate contract repaired, but the extracted functions have no declarations and lose required execution context; R2. |
| F13 V3 evidence | PARTIAL | P4.T9/S3:L1575 “pub struct V3PolicyEvidence”; P4.T9/S3:L1594 “Add `pub flop_policy: crate::flop::FlopPolicy` to `EngineCore`” | P4.T10/S4:L1775 “let policy=&core.flop_policy;”; P4.T24/S2:L4107 “Also serialize the matching `V3PolicyEvidence`”. Ownership repaired; Interfaces takes &Provenance while implementation takes &V3PolicyEvidence (R3). |
| F14 bench DTO | RESOLVED | P4.T20/I:L3594 “pub struct PreparedBenchSpot”; both generators return Vec<PreparedBenchSpot>. P2.T5 owns persisted bench Spot. | P4.T20/S3:L3647 “`bench` converts `PreparedBenchSpot` into its own persisted `Spot`”; P4.T20/S3:L3653 “pub enum RangeSpec { Text(String), Weights(proto::Range1326) }”. suite.rs/runner.rs are Modify; no reverse dependency. |
| F15 dependency keys | PARTIAL | P1.T1/S1:L119 “hex = "0.4"”; P1.T1/S1:L120 “ts-rs = "=12.0.1"” | P2.T2/S1:L329 “hex.workspace = true”; P4.T1/S3:L194 “ts-rs = { workspace = true, optional = true }”; P5.T1/S4:L315 “workspace = true”. P5.T1/S1:L169 still says P1 did not declare ts-rs and directs another root addition; R4. |
| F16 toolchain | PARTIAL | P2.T1/S5:L266 “Write the selection to `docs/bench/worker-toolchain.json`”; L270–282 specify toolchain/target/provenance and the 1.25 rule. | P2.T7/S5:L2092 “Read `worker_toolchain` from `docs/bench/worker-toolchain.json`”; P4.T24/S2:L4093 reads it. P5.T5/S6:L1267 still unconditionally builds MSVC and L1272 copies MSVC. P4.T23 also puts target arguments before the Cargo subcommand; R5. |
| F17 oracle targets | RESOLVED | P2.T15/Files:L3626 “solver-worker/tests/contract_river.rs”; P2.T18/S1:L4184 “fn river_check_only_terminal_oracle()”. P1.T22 owns the generator. | P4.T23/S3a:L3964 “args:&["test","-p","solver-worker","--release","--test","contract_river"]”; L3967–3968 name worker_link and the exact check-only test; L3981–3990 execute the 10M generator and check failure. GNU argument construction is tracked separately under F16. |
| F18 chart staging | PARTIAL | P3.T4/I:L731 “Nothing downstream reads a chart bundle whose depth row is not `"available"`.” | P5.T5/S6:L1281 “foreach ($depth in $availability.depths)”; P5.T5/S6:L1282 “if ($depth.status -ne 'available') { continue }”. Manifest shape matches P3:L719–728 and wildcard copy is removed. Final debug/release preflop directories are never cleared, so unavailable old pairs linger; R6. |
| F19 47 cases | RESOLVED | S§13.2:L735 “In-process equality over the 47 boundary cases” | P2.T6/S3:L1720 “TEMPLATES = ["flop_fast_v1", "flop_min_v1", "flop_full_v1",”; P2.T6/S3:L1727 “for pot, eff in [(100, 100), (100, 150), (180, 910), (100, 149), (100, 151)]:”. Seven times five plus twelve = 47; Task 8 asserts 47 and five flop_full cases. |
| F20 donk | PARTIAL | S§4.6:L358 “a `None` donk option is legal only at `root_street`” | P2.T8/13/16 follow it; S§13.2:L746 remains unqualified and reason strings differ. See §1/R1. |
| F21 final V22 | RESOLVED | P5.T14/S5:L3141 “evidence for Plan 4 Task 26” | P4.T23/S3:L3937 “("chart UI E2E",g.chart_ui_e2e_ok),”; P4.T26/I:L4187 “is the sole final V22 decision.” P5.T14 precedes P4.T26 in both documents and graph. |
| F22 cross-references | PARTIAL | P1.T24/heading:L4715 “exact equity, per-combo equity and terminal payoffs”; P4.T11/heading:L2000 “Create the section 6 experimental synthetic-root surrogate”; P4.T23/heading:L3866 “run the oracle suites and enforce the baseline gate”; P2.T20 deadlines/T26 assembler/T29 façade; P3.T14 snapshots/T18 registration. | Most E06 references agree. P2.T18/S1:L4207 “`core-eval`'s shape (plan 1 Task 20)” remains wrong. All five current Spec headers say revision 6; P5 header still cites old review §§4–5 for façade authority. R7. |
| F23 workspace glob | RESOLVED | P1.T1/S1:L105 “members = ["crates/*"]” | P3.T1/S1:L131 “do not append an explicit `crates/core-preflop` member”; P3.T11/I:L1629 “do not append it to the root manifest's `members`”. |
| F24 generated artifacts | RESOLVED | P2.T5/I:L1195 “bench gen-spots --source r8 --out bench/spots”; P2.T30/S4:L7171 “docs/bench/2026-09-10-i7-13700K.md”. | P4.T20/Files:L3588 “**modify/regenerate** the six suite files”; P4.T24/Files:L4066 “**Files:** Modify/append `docs/bench/2026-09-10-i7-13700K.md`”. Provenance/reference rows retained. |

## 3. New extraction seam and shortest remaining edits

These are proposed edits, not changes made by this review. Apply the normal revision/changelog rule when editing plans/spec. Severity describes the remaining defect, not the severity of the original finding.

### R1 — MINOR — F20 precision

**Location:** S§13.2:L746; P2.T8/I:L2128 and T13/S3:L3283.
**Quoted problem:** “None donk option in tree” omits the root exception.

**Exact edits:** Replace the spec row fragment with “a None donk option on a street strictly after root_street: typed ack{rejected} with reason, no work, worker stays alive; root-street None is accepted”. Render identifiers as code in the spec. In Task 8's wire-rule paragraph use reason “donk option missing for <street>”. In Task 13's None branch use:
~~~rust
None => return Err(format!("donk option missing for {}", match s {
    Street::Turn => "turn", Street::River => "river", _ => unreachable!()
}))
~~~
Keep Task 8's direct-caller tree_config error and its test unchanged; that is not a wire reason.

### R2 — MAJOR — F12 / new transport seam

**Location:** P4.T11/Files:L2004, Interfaces:L2006, Step 4:L2143–2173; P2.T22/I and T23/I.
**Quoted problem:** “send_solve_request(core,request,input.hero_role,sink)”.

**Check result:** solve.rs **is listed under Modify**, via “crates/engine/src/{lib,serve,solve}.rs”; extraction is described, but **neither exact function signature is declared anywhere**. Plan 2 has no forward-owner note. The illustrated sender receives neither DecisionIdentity nor absolute Deadlines; these are not fields of the wire SolveRequest. The builder also lacks EngineCore's request-id allocator/memory limit. P2:T22:L5176–5200 proves those inputs are required.

**Exact replacement contract for P4.T11 Interfaces/Step 4:**
~~~rust
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
~~~
Add: “Plan 4 Task 11 owns these crate-visible extractions. The builder preserves Plan 2's request-id allocation, spot hash, memory limit, rake, history and background flag without constructing SolveInput. The sender shares one-attempt transport, whole-solution validation, identity checks, heartbeat, cancellation and absolute deadlines. Keep main-path requested-node/hero-actor checks and Task 23 retry/admission policy in run_solve; the surrogate validates its separately selected root/check-child advice row and skips on failure.”

Replace the illustrated calls with a SolvePlan carrying the supplied identity/deadlines/rake/hero role, template_id from input, retry_template_id: None and background: false. Compute remaining deadline with plan.deadlines.worker_deadline_ms(core.clock.now_ms(), plan.deadlines.street_deadline_ms)?; pass core, &root, &ranges, target_bp, &plan, &build, deadline_ms to the builder and core, request, &plan, &build, sink to the sender. Preserve run_solve's existing signature and regression tests.

**Exact forward-owner note to add to P2.T22 and reference from P2.T23:**

> Forward owner: Plan 4 Task 11 modifies crates/engine/src/solve.rs to extract solve_request_from_parts and send_solve_request with the exact signatures declared there. Plan 2 produces run_solve first; it does not produce these helpers early. The later extraction preserves run_solve's signature, identity/deadline/heartbeat/cancellation/validation behavior and Task 23 retry policy, and lets the synthetic surrogate bypass SolveInput, cache and snapshots. Plan 4 Task 11 owns both the extraction and its callers.

No graph change: P4.T11 already follows the completed Plan 2 solve path.

### R3 — MINOR — F13 loader signature

**Location:** P4.T9/I:L1456 versus Step 3:L1580.
**Quoted problem:** “load_v3_policy(path:&Path,expected_provenance:&Provenance)->FlopPolicy”.

**Exact edit:** Replace that Interfaces declaration with “load_v3_policy(path:&std::path::Path,expected:&V3PolicyEvidence)->FlopPolicy”, matching the owned implementation. Keep the conservative missing/stale-evidence behavior.

### R4 — MINOR — F15 dependency ownership

**Location:** P5.T1/S1:L169.
**Quoted problem:** “Plan 1's [workspace.dependencies] table does not declare ts-rs”.

**Exact replacement:** “Plan 1 Task 1 already declares ts-rs = "=12.0.1" in [workspace.dependencies]. Preserve that single root declaration; crates/proto inherits it with workspace = true in Step 4, as cache already does in Plan 4 Task 1.”

### R5 — MAJOR — F16 selected worker propagation

**Location:** P5.T5/S6:L1267–1272; P4.T23/S3a:L3997–3998/L4012.
**Quoted problem:** “cargo +stable-x86_64-pc-windows-msvc build --release -p solver-worker --target x86_64-pc-windows-msvc”.

**Exact P5 replacement:** Rename Step 6 “Stage the selected worker and available charts”. Replace the unconditional build with:
~~~powershell
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
~~~
Replace L1272 with “Copy-Item -LiteralPath $workerBinary -Destination $stage”. Keep UI target directories MSVC.

**Exact P4 replacement contract:** “worker_toolchain_args returns the complete Cargo argument vector for the suite; call .args(worker_toolchain_args(suite)) once, without appending suite.args again. For the GNU worker suite return ["+stable-x86_64-pc-windows-gnu", "test", "-p", "solver-worker", "--release", "--target", "x86_64-pc-windows-gnu", "--test", "contract_river"]; otherwise return suite.args unchanged. Read/validate the V1 selection and retain the selected POKERAI_WORKER path.”

The present .args(helper).args(suite.args) places --target before test when the helper prepends both selection arguments. The selection must survive into a valid Cargo invocation.

### R6 — MAJOR — F18 final staging cleanup

**Location:** P5.T5/S6:L1292–1301.
**Quoted problem:** “New-Item -ItemType Directory -Force -Path (Join-Path $exeRuntime 'preflop')”.

The temporary stage is refreshed, but both final runtime/preflop directories only receive copies. A prior available bundle can remain after its manifest row becomes unsupported.

**Exact edit:** Before creating each final preflop directory, add:
~~~powershell
$runtimePreflop = [IO.Path]::GetFullPath((Join-Path $exeRuntime 'preflop'))
$targetRoot = [IO.Path]::GetFullPath((Join-Path $projectRoot 'target')).TrimEnd('\') + '\'
if (-not $runtimePreflop.StartsWith($targetRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'runtime preflop path escapes project target directory'
}
if (Test-Path -LiteralPath $runtimePreflop) {
    Remove-Item -LiteralPath $runtimePreflop -Recurse -Force
}
~~~
Add to Step 6's verification: “Stage with 200bb available, change its manifest status to unsupported, stage again, and verify its bundle/manifest pair is absent from target/pokerai-runtime/preflop and both executable runtime/preflop directories; the available 100bb pair remains.”

### R7 — MINOR — F22 reference cleanup

**Locations and exact edits:**

- P2.T18/S1:L4207: replace “plan 1 Task 20” with “plan 1 Task 24”.
- P1–P5 current Spec headers, each L13: replace the current-version label “revision 6” with “revision 7”. Preserve historical revision-6 amendment notes elsewhere.
- P5 header L13: replace “docs/research/REVIEW-cross-plan.md §§1, 2, 4, 5 for the resolved engine façade” with “docs/research/REVIEW-cross-plan-2.md §4.2 for the engine façade; its §7 and EXECUTION-ORDER.md supply the task order. REVIEW-cross-plan.md §§4–5 are historical.”

## 4. Execution-order verification

**PASS.** All 114 prerequisite rows and all 44 wave rows are identical to REVIEW-cross-plan-2 §7, including task titles, evidence columns and stars. Wave membership contains each task exactly once; every hard prerequisite is in an earlier wave. The copied evidence line numbers are historical coordinates from the source review, not current plan line numbers.

Seeded random sampling (System.Random, seed 20260917) verified ten prerequisites; actual output follows. First three waves are correct.

~~~text
Prerequisite rows: source=114; copied=114; identical=True
Wave rows: source=44; copied=44; identical=True
Wave task occurrences=114; unique=114; invalid prerequisite edges=0
First three waves:
| 1 | ★ 1.1 |
| 2 | ★ 1.2, 1.15, 2.1 |
| 3 | ★ 1.3, 1.4, 1.16, 1.22 |
Seeded random sample (seed 20260917):
1.5: prerequisites=1.3; wave=4; exact source row=True
1.7: prerequisites=1.6; wave=5; exact source row=True
1.9: prerequisites=1.3; wave=4; exact source row=True
2.10: prerequisites=2.8; wave=16; exact source row=True
2.18: prerequisites=2.2, 2.15; wave=22; exact source row=True
2.24: prerequisites=2.19; wave=24; exact source row=True
2.29: prerequisites=2.28; wave=28; exact source row=True
3.1: prerequisites=1.18, 1.20, 1.5; wave=10; exact source row=True
4.1: prerequisites=1.21, 1.20, 1.8; wave=7; exact source row=True
4.15: prerequisites=4.14; wave=12; exact source row=True
Extraction helper declarations in P2/P4:
declarations=0
~~~

## 5. Stale-reference grep disposition

The scan covers exactly the spec and five dated plans. Literal occurrence does not imply a stale dependency.

- No old bench-oracle Task 20 reference; its owner is P4.T23. Remaining Task 20 references cover real local tasks or P4 suite regeneration.
- No “Plan 2 Task 21” or “Plan 2 Task 16” stale phrase.
- “Plan 2 Task 22”: P4:L98/L4250 correctly name SolvePlan.background. “Plan 2 Task 19”: P4:L3005 correctly names the fake worker/clock.
- advice_rows: P4:L77/L2123/L4246 all explicitly say it is absent or must not be called.
- FlopBenchSpot: P4:L3610 explicitly says there is no such DTO.
- test_extra: P4:L1322 explicitly forbids adding it.
- SolvedStreetStore and “cheap answer”: no matches.
- Remaining actionable references: P2:L4207 (exact-equity owner); all five L13 spec-version labels; P5:L13 old façade citation. R7 supplies replacements. The other revision-6 hits are historical amendment/decision references; S:L5 is deliberately preserved revision history.

Actual compact grep output:
~~~text
(?i)Plan 2 Task 21\b: 0
(?i)Plan 2 Task 16\b: 0
(?i)Plan 2 Task 22\b: 2
  P4:L98
  P4:L4250
(?i)Plan 2 Task 19\b: 1
  P4:L3005
SolvedStreetStore: 0
advice_rows: 3
  P4:L77
  P4:L2123
  P4:L4246
FlopBenchSpot: 1
  P4:L3610
test_extra: 1
  P4:L1322
(?i)cheap(?:\W|\*)+answer: 0
(?i)plan 1 Task 20: 1
  P2:L4207
revision 6: 29
  S:L5
  P1:L13
  P1:L5413
  P1:L5414
  P1:L5415
  P1:L5416
  P1:L5418
  P1:L5423
  P1:L5424
  P1:L5425
  P2:L13
  P2:L3635
  P2:L7224
  P3:L13
  P3:L15
  P3:L121
  P3:L1098
  P3:L1648
  P3:L2124
  P3:L2130
  P3:L2571
  P3:L2927
  P3:L3042
  P3:L3146
  P3:L3147
  P3:L3155
  P3:L3168
  P4:L13
  P5:L13
Bench-oracle Task-20 references:
0
Plan 5 V1 selection reads:
0
Plan 3 direct range_vs_range references:
0
Exact range_vs_range definitions across five plans:
1
~~~

## 6. Verification and self-review

**Rust/Python/UI execution: not applicable.** Documentation-only task; Cargo.toml does not exist. Python was unavailable for an initial read-only inventory attempt; equivalent checks completed with PowerShell/.NET. No packages or dependencies were installed.

Commands used: Get-Content -Encoding UTF8 for named sources; Select-String/rg -n for exact seam names and source locations; Compare-Object over prerequisite and wave table rows; System.Random(20260917) for the ten-row sample; Get-FileHash -Algorithm SHA256 over docs excluding this report for the no-other-edits check. The graph and grep blocks above are actual command output, not reconstructed summaries.

Self-review: each F01–F24 has one status; single-owner evidence is distinguished from consumers, and P3 is not falsely described as a direct range_vs_range caller. The F20 authority contradiction is settled, but its remaining scope/reason wording is disclosed. New helper names are not mistaken for declared signatures. F17's corrected test targets are separated from F16's GNU command construction. Historical mentions and negative examples are excluded from the actionable stale-reference list. No broad re-review or implementation claim is made.

Proposed INDEX row (orchestrator-owned; not applied): REVIEW-cross-plan-3.md | review | current | 2026-09-17 | Targeted Phase B verification: 17 resolved, 7 partial; execution graph copied exactly; ready after seven listed edit groups.

Final verification output (the docs digest matches the pre-write digest; all 55 other documents are unchanged):

~~~text
Source quotations verified=93; errors=0; limit=25 words per quotation
Finding rows=24; resolved=17; partial=7; not resolved=0
Sequential order code blocks identical: True
docs excluding report: files=55; SHA256=0424672DC2E68A0D507E6B34B987015B6841B7448928A45773F14070FEE93E0D
~~~
