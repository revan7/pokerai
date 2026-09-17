---
type: changelog
status: current
date: 2026-09-17
supersedes: none
related:
  - ../superpowers/plans/2026-09-10-plan-5-ui.md
  - REVIEW-cross-plan-2.md
  - PLAN-5-CHANGELOG-1.md
---

# Plan 5 revision 2 — changelog

Plan: `docs/superpowers/plans/2026-09-10-plan-5-ui.md`
Task: B3f (seam re-check edits for plan 5)
Input: `docs/research/REVIEW-cross-plan-2.md` — section 6 group "E05 — Plan 5", every "E06" item naming plan 5, section 4.2 ("Plan 5's eight 'Required from Plan 2' entries"), findings F01/F02/F03/F08/F10/F15/F18/F21/F22 as applicable.
Orchestrator decisions applied (2026-09-17, binding): (1) Plan 2's `engine::Paths { log_dir, worker_exe, preflop, cache }` stands; Plan 5 constructs those field names. (2) `Engine::new -> Result<Engine, EngineError>` stands; startup banners come from `Engine::startup_report().banners` (`StartupReport { worker_ready, worker_threads, build_features, cpu_features, capabilities, cpu_lacks_avx2, quarantined_bundles, cache_state, banners }`); the tuple-return expectation and the stale "resolved façade" self-review claim (F02) are deleted. (3) `Engine::begin_hand` takes `proto::BeginHand` directly per Plan 2's revised API; the shell-side `core_model::BeginHand` conversion (F03) is removed. (4) F18: runtime staging reads `fixtures/charts/sources.manifest.json` (Plan 3) and handles an `unsupported` depth (e.g. 200bb) without assuming it exists. (5) F21: V22's final gate gets a chart-backed UI acceptance input, added to the E2E task per the report. (6) F15: every workspace dependency Plan 5 uses is declared.

**Result: 3,153 lines / 14 tasks -> 3,176 lines / 14 tasks.** No tasks added, removed or renumbered. Every E05 item applied except the F16 (worker-toolchain GNU/MSVC selection) half of the bundled "T5 / Step 6" edit, which is out of this task's authorized finding list (F16 was not among F01/F02/F03/F08/F10/F15/F18/F21/F22) and is left for a separate task; F08 not applicable to plan 5's own text; F22 applied via its one plan-5-specific E06 row (the rest of F22's citations are inside Plan 2/Plan 4 documents).

---

## 1. Section 6, group E05 — Plan 5 (exact items)

| Item | Status | What changed |
|---|---|---|
| Required façade block (F01/F02/F03/F10) | **APPLIED** | Replaced the eight-entry "Required from Plan 2" block and its stale "binding fixes" comments (intro, before Task 1) with the report's grouped structure: `new`/`startup_report`/`set_config`/`shutdown`/`set_hero_cards`/`Paths`/`begin_hand` under "Required from Plan 2 Task 29" (begin_hand now takes `proto::BeginHand`, not `core_model::BeginHand`; Paths uses `log_dir`/`worker_exe`), and `presolver_status`/`presolver_pause`/`presolver_resume` moved to a new "Required from Plan 4 Task 16, with status re-exported by Task 15" group. Added a `StartupReport` field-list comment per the orchestrator's explicit field names. The "Unchanged from Plan 2's produced API" block keeps `apply_action`/`set_board`/`undo`/`recommend`/`cancel`/`finish_hand`/`abandon_hand`/`trait EventSink`. |
| Gate prose at L51 and L151 | **APPLIED** | Both paragraphs (the "Upstream contract gate" intro and the File-structure paragraph about `proto::BeginHand` vs. `core_model::BeginHand`) replaced with the report's short authoritative-façade text: shell passes `proto::BeginHand` unchanged, engine allocates identity/constructs `core_model::BeginHand`, startup warnings come from `startup_report().banners`, Plan 4 Task 15/16 own `PresolverStatus`, Tasks 1–3 use the mock seam and Task 4 is the first real-engine gate. |
| Self-review lines (F02/F03) | **APPLIED** | §12 coverage row: "`Engine::new`'s startup diagnostics ... (BL-6)" -> "`Engine::startup_report().banners` ... (F02, revision 2)". Type-consistency check: "Task 4 converts between them" -> "passed to `Engine::begin_hand` unchanged (F03, revision 2); the engine ... constructs its internal `core_model::BeginHand`". "Engine façade: resolved, not remaining" paragraph fully rewritten to "Engine façade: revision 2 ..." citing F01/F02/F03/F10 and the four fields/three methods each now correctly owns. |
| T1 / Step 4, proto ts-rs dependency (F15) | **APPLIED** | `crates/proto/Cargo.toml` additions: `[dependencies.ts-rs]` changed from `version = "=12.0.1"` to `workspace = true` (keeping `optional = true` and `features = ["serde-compat"]`). Also added to T1 / Step 1: an explicit instruction to add `ts-rs = "=12.0.1"` to the root `[workspace.dependencies]` table in that same step, since Plan 1's table does not declare it and Plan 5 is its first consumer (F15's own "A root optional-use version declaration does not enable ts-rs in proto" point, satisfied here because Plan 5 is the one declaring it). |
| T4 / Files + Step 3 (F03) | **APPLIED** | Files line: `{service.rs,tests.rs,Cargo.toml}` with a `core-model` path dependency -> `{service.rs,tests.rs}`, "No new core-model dependency is needed." Step 3 header: dropped "and the `proto::BeginHand -> core_model::BeginHand` conversion." Code: the `Op::Begin` arm's `core_model::BeginHand { hand_id: 0, ... }` construction replaced with `encode_hand(e.begin_hand(b).map_err(engine_error)?)`. |
| T5 / Step 4, Paths literal (F01) | **APPLIED** | `lifecycle::paths`'s `engine::Paths { worker: ..., preflop: ..., cache: ..., log: local.join("decisions.jsonl") }` -> `engine::Paths { worker_exe: ..., preflop: ..., cache: ..., log_dir: local }`, with a comment that `log_dir` is the `%LOCALAPPDATA%/PokerAI` directory, not `decisions.jsonl` (DecisionLog appends that file inside it). |
| T5 / Interfaces + Step 5a (F02) | **APPLIED** | Interfaces Consumes line: `Engine::new(GameConfig, Paths) -> Result<(Engine, Vec<String>), EngineError>` (BL-6 tuple) -> `Engine::new(...) -> Result<Engine, EngineError>` plus `Engine::startup_report(&self) -> StartupReport`. Step 5a code: `let (engine,engine_warnings)=engine::Engine::new(...)...; warnings.extend(engine_warnings);` -> `let engine=engine::Engine::new(...)...; warnings.extend(engine.startup_report().banners);`. Trailing prose paragraph rewritten to match (no more "`Vec<String>` return element (BL-6)"). |
| T5 / Step 6, runtime staging (F18; F16 portion not applied) | **APPLIED (F18 half only)** | Step title: "Stage the MSVC runtime and wire resource mapping" -> "Stage the MSVC runtime and the available charts." Script: added `Remove-Item` of a pre-existing stage directory (fresh staging per build); replaced the hard-coded `foreach ($name in @('pokercoaching_100','rangeconverter_200'))` loop with a manifest-driven block reading `fixtures/charts/sources.manifest.json` and copying only `status -eq 'available'` bundle/manifest pairs (also staging the manifest itself); replaced the `Copy-Item -LiteralPath (Join-Path $stage 'preflop/*')` wildcard (which `-LiteralPath` never expands) with a `Get-ChildItem -File | ForEach-Object { Copy-Item ... }` enumeration. Trailing prose updated to describe manifest-driven, availability-conditional copying. The worker build command itself (`cargo +stable-x86_64-pc-windows-msvc build ...`, unconditional MSVC) was left unchanged — that is F16 (GNU/MSVC toolchain selection), which is bundled into the same report edit block but is not in this task's authorized finding list; see §4 "Not applied" below. |
| T14 / final acceptance paragraph (F21) | **APPLIED** | "Do not claim V22 passes unless Plan 4's independent bench gates also passed." -> the report's replacement: records `chart_ui_e2e` evidence (pass/fail, elapsed time, provenance) for Plan 4 Task 26; a missing/failed UI result blocks V22; Task 26 combines this with the independent bench bounds as the sole final V22 decision. Also updated the matching self-review row (§14.4 V21/V22) to describe the same `chart_ui_e2e` evidence hand-off. |

## 2. E06 items naming plan 5

| Stale subject/reference | Status | What changed |
|---|---|---|
| "P5 says status lacks Serialize and P4.T14 owns it" | **APPLIED (as a byproduct of the required-façade-block edit)** | The old gate-prose sentence "Plan 4's `PresolverStatus` is a Rust struct without a `Serialize` derive today (cross-plan Or7 assigns that derive to Plan 4 Task 14)" is gone; the new gate prose (§1 above) states "Plan 4 Task 15 provides serializable `PresolverStatus` ... and Task 16 provides its engine delegation," matching the report's exact replacement text. |
| "Replace P3's whole-plan prerequisite at L11, P4's 'Execute after Plans 1–3' at L40, and **P5's whole-plan gate at L11**" | **APPLIED** | Plan 5's own whole-plan-gate sentence ("Date: 2026-09-10. This is the fifth plan; implementation begins after Plans 1–4 supply their public surfaces and chart fixtures.", in the `**Spec:**` line) replaced verbatim with the report's shared replacement paragraph ("Execute tasks against the hard prerequisites in REVIEW-cross-plan-2 §7 ... Serialize edits to shared files when working in one checkout, and serialize measured benchmark/acceptance runs on the benchmark machine."). |
| "Introductions refer to REVIEW-cross-plan §§4–5 as current order" | **NOT APPLICABLE** | Plan 5's `**Spec:**` line cites `docs/research/REVIEW-cross-plan.md` §§1, 2, 4, 5 for the *resolved engine façade* (a reconciliation citation, not a scheduling-order citation), and the plan makes no claim that those sections set current execution order — that claim lived only in the whole-plan-gate sentence just above, already replaced. The E06 row's own example concern (treating §§4–5 as authoritative *scheduling*) does not match what this citation is used for, so left as is. |

## 3. Section 4.2 "Plan 5's eight 'Required from Plan 2' entries" — cross-check

| Entry | Report result | Plan 5 action |
|---|---|---|
| `new` | "P5 wrong, user decision; E05. P2.T29 unchanged." | **Edited** — `Result<Engine, EngineError>` plus `startup_report()`; see F02 above. |
| `set_config` | "Satisfied. Preserve queuing and validation." | **No edit** — already `(&mut self, GameConfig) -> Result<u32, EngineError>`. |
| `shutdown` | "Satisfied." | **No edit** — already `(&mut self)`, idempotent. |
| `set_hero_cards` | "Satisfied, including mutation revision." | **No edit** — already correct. |
| `presolver_status` | "Not in P2 ... Move to 'Required from P4.T16'; add parent re-export in P4.T15." | **Edited** — moved out of the Plan-2 group into its own "Required from Plan 4 Task 16" group; see F10 above. |
| `presolver_pause` | Same as above | **Edited** — same regrouping. |
| `presolver_resume` | Same as above | **Edited** — same regrouping. |
| `Paths` | "P5 wrong, user decision; E05." | **Edited** — `log_dir`/`worker_exe` field names; see F01 above. |

`begin_hand` (called out separately in the report's §4.2 prose, "The additionally advertised unchanged `begin_hand` is wrong (F03)") — **Edited**, moved from "Unchanged from Plan 2's produced API" (with the old `core_model::BeginHand` parameter) into the "Required from Plan 2 Task 29" group with `proto::BeginHand`.

## 4. Findings F08, F15, F18, F22 — disposition

| Finding | Status for plan 5 | Reason |
|---|---|---|
| F08 — configuration regression / wrong shared-field access | **NOT APPLICABLE** | Grepped plan 5 for the finding's cited strings (`config: Arc<Mutex<GameConfig>>`, `queued_config = Some(stamped)`, `self.core.lock().unwrap().set_config`, `core.config.solver.target_bp`): no matches. All four citations are inside Plan 2 (P2.T27, P2.T29) and Plan 4 (P4.T9, P4.T10) documents; plan 5 names neither `EngineCore` nor its config field directly — it only calls `Engine::set_config`, whose signature plan 5 already assumed correctly. Nothing in plan 5's own text to change. |
| F15 — undeclared workspace dependencies | **APPLIED** | See §1 above (T1 Step 1 + Step 4). |
| F16 — worker toolchain selection (bundled into the same E05 "T5/Step 6" edit as F18, but not itself assigned to this task) | **NOT APPLIED** | The task's finding list is F01/F02/F03/F08/F10/F15/F18/F21/F22; F16 is not in it. The report's T5/Step 6 edit block bundles two independent fixes under one heading ("T5 / Step 6, runtime staging (F16/F18)"): a conditional GNU/MSVC worker-build selection (F16, reading `docs/bench/worker-toolchain.json`) and manifest-driven chart staging (F18). Only the F18 half was applied; the worker build command stays unconditional MSVC, unchanged from revision 1. Applying F16 here would exceed this task's authorized scope; it is left for whichever task is assigned F16. |
| F18 — runtime staging assumes conditional chart availability | **APPLIED** | See §1 above (T5 Step 6 script + Interfaces line). |
| F22 — stale task numbers and purported exports | **APPLIED via the one plan-5-specific E06 row in §2; otherwise not applicable** | F22's own citations (P2:L7062/L7065, P4.T11/T23 Interfaces, and its `engine::snapshots::register_snapshot` addendum) are all inside Plan 2 and Plan 4 documents, not plan 5's own text. Its instruction "Apply semantic table E06, including the snapshot registration and benchmark facade corrections" is what reaches plan 5 — satisfied by the "status lacks Serialize" row in §2. |

## 5. Verification

Documentation-only task — Rust/Python/UI execution not applicable. Grepped the revised plan for every finding's cited old text (all absent):

```
grep -n "worker: resource.join\|log: local.join" <plan>                                    -> no matches (F01)
grep -n "let (engine,engine_warnings)=engine::Engine::new" <plan>                           -> no matches (F02)
grep -n "Result<(Engine, Vec<String>), EngineError>" <plan>                                 -> no matches (F02)
grep -n "let core = core_model::BeginHand\|let core=core_model::BeginHand" <plan>           -> no matches (F03)
grep -n "Plan 4 promises only delegation, not these signatures" <plan>                      -> no matches (F10, old mis-grouping comment)
grep -n -A2 "\[dependencies.ts-rs\]" <plan>                                                 -> "workspace = true" (F15, was "version = \"=12.0.1\"")
grep -n "pokercoaching_100','rangeconverter_200" <plan>                                     -> no matches (F18, old hard-coded chart-name loop)
grep -n "'preflop/\*'" <plan>                                                                -> 1 match, but it is the NEW explanatory comment at line 1297
                                                                                                 ("... instead of a `-LiteralPath ... 'preflop/*'` wildcard, which -LiteralPath
                                                                                                 never expands") describing why the old pattern was replaced, not the old
                                                                                                 executable code itself; the actual `Copy-Item -LiteralPath ... 'preflop/*'`
                                                                                                 statement is gone, replaced by the `Get-ChildItem -File | ForEach-Object` loop.
grep -n "Do not claim V22 passes unless Plan 4's independent bench gates also passed" <plan> -> no matches (F21)
grep -n "without a \`Serialize\` derive today\|assigns that derive to Plan 4 Task 14" <plan> -> no matches (F22)
grep -n "config: Arc<Mutex<GameConfig>>\|queued_config = Some(stamped)" <plan>               -> no matches (F08, confirms not-applicable — plan 5 never had this text)
grep -n "This is the fifth plan; implementation begins after Plans 1–4" <plan>               -> no matches (E06 whole-plan gate)
```

And confirmed the replacement text is present at the edited locations: `Paths { log_dir: PathBuf, worker_exe: PathBuf, preflop: PathBuf, cache: PathBuf }` and `Engine::begin_hand(&mut self, begin: proto::BeginHand)` in the required-façade block; `worker_exe: resource.join(...)` / `log_dir: local` in `lifecycle::paths`; `let engine=engine::Engine::new(...)` / `warnings.extend(engine.startup_report().banners)` in Step 5a; the manifest-driven `foreach ($depth in $availability.depths)` block and `Get-ChildItem -LiteralPath $stagedPreflop -File` enumeration in Step 6; `chart_ui_e2e evidence for Plan 4 Task 26` in Task 14; `workspace = true` in the ts-rs dependency table.

Task count unchanged: `grep -c "^### Task "` on the revised plan returns 14. Title carries the required revision line: `Revision 2 (2026-09-17): seam re-check edits E05/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-5-CHANGELOG-2.md`.

## 6. Not applied

- F08 — not applicable to plan 5's own text (see §4); its four citations are inside Plan 2 and Plan 4 documents.
- F16 (worker-toolchain GNU/MSVC selection) — not in this task's authorized finding list, even though the report bundles it with F18 under one "T5/Step 6" heading; left for whichever task is assigned F16. The worker build command in `stage-runtime.ps1` remains unconditional MSVC, unchanged from revision 1.
- The E06 "Introductions refer to REVIEW-cross-plan §§4–5 as current order" row — plan 5's citation of those sections is for the resolved engine façade, not for scheduling order, so the row's concern does not match plan 5's actual text (see §2).
- Most of F22's own citations — inside Plan 2/Plan 4 documents, out of scope for a plan-5-only edit; plan 5's one specific stale reference (the PresolverStatus/Serialize sentence) was fixed via the required-façade-block edit (§2).

Nothing in this list was declined on the merits; each is either not a plan-5 defect, outside this task's authorized finding list, or already satisfied as a byproduct of another required edit.
