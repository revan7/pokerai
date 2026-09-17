---
type: changelog
status: current
date: 2026-09-17
supersedes: none
related:
  - ../superpowers/plans/2026-09-10-plan-3-preflop-replay.md
  - REVIEW-cross-plan-2.md
  - PLAN-3-CHANGELOG-1.md
---

# Plan 3 revision 2 — changelog

Plan: `docs/superpowers/plans/2026-09-10-plan-3-preflop-replay.md`
Task: B3d (seam re-check edits for plan 3)
Input: `docs/research/REVIEW-cross-plan-2.md` — section 6 group "E03 — Plan 3", every "E06" item naming plan 3, section 4.3 ("Plan 3's four named seams"), findings F05/F06/F07/F11/F18/F22 as applicable.
Orchestrator decisions applied: Plan 2's `engine::Paths { log_dir, worker_exe, preflop, cache }`; `Engine::startup_report()`; `engine::ranges::RangeSource` ownership as the report prescribes (shared `Arc<Mutex<Box<dyn RangeSource>>>`); single `range_vs_range` implementation (report assigns it to Plan 2/`engine::equity`, with Plan 4 holding the duplicate — plan 3 does not implement `range_vs_range` at all, so nothing to remove here).

**Result: 3,172 lines / 19 tasks -> 3,174 lines / 19 tasks.** No tasks added, removed or renumbered. Every E03 item applied; every E06 item naming plan 3 applied; F06/F07/F23 applied via E03; F01/F02 applied via E03's T17 paragraph; F05, F11 and F18 not applicable to this plan's text (evidence cited from plan 3 in the report was already correct); F22's plan-3-specific sub-items applied via the E06 rows below, its register_snapshot note confirms plan 3 was already correct.

---

## 1. Section 6, group E03 — Plan 3 (exact items)

| Item | Status | What changed |
|---|---|---|
| Header L109 / T17 Interfaces:L2702 / T17 Step 3:L2762 (F01) — old `Paths { worker, preflop, cache, log }` shapes | **APPLIED** | All three occurrences replaced with Plan 2's actual struct `Paths { log_dir: PathBuf, worker_exe: PathBuf, preflop: PathBuf, cache: PathBuf }`: the "three engine seams" header prose, Task 17's Interfaces Consumes line, and the Task 17 Step 3 startup paragraph. |
| T1 / Step 1 + T11 / Files + Step 1 (F23) — "append `crates/core-preflop`/`crates/core-replay` to members" | **APPLIED** | Both instructions replaced with "the existing `crates/*` workspace glob already includes this crate once its `Cargo.toml` is created; do not append an explicit member," matching Plan 1's own note about Plan 2 (`crates/*` already covers `crates/engine`/`crates/bench`, "which plan 2 must not re-add"). Task 1's Cargo.toml snippet changed `version = "0.1.0"` / `edition = "2021"` / `license = "MIT OR Apache-2.0"` to `version.workspace = true` / `edition.workspace = true` / `license.workspace = true`, matching Plan 1's own crate manifests (e.g. `core-model`, confirmed by grep) rather than leaving a hard-coded package block inconsistent with "inherit ... from workspace.package." Task 11's Files line drops "modify `Cargo.toml` (append ... to members)" and adds the same glob note plus a pointer to Task 1's package-field pattern. |
| Header L114, T18 Interfaces + Step 3:L2877/L2923 (F06) — Box-only `range_source` init and `ReplayRanges` missing `identity` in the advertised record | **APPLIED** | Header seam-2 prose: `EngineCore.range_source` changed from `Box<dyn RangeSource>` to `Arc<Mutex<Box<dyn RangeSource>>>` ("the SAME shared handle `Engine` clones"), `ReplayRanges`'s advertised fields gained `identity: Arc<Mutex<IdentityState>>`, and the call site became `core.range_source.lock().unwrap().ranges_at_root(...)`. T18 Interfaces Produces line: same `identity` field added. T18 Step 3 "root-range seam" paragraph: `.ranges_at_root(...)` call and the install description now go through the locked shared handle. T18 Step 3 post-code prose (old L2923): replaced "`EngineCore::new` sets `range_source: Box::new(ReplayRanges {...})`" with the locked-handle install (`*core.range_source.lock().unwrap() = Box::new(ReplayRanges {...})`) performed after `core.preflop` loads and before `Engine::with_core` starts engine-main, plus a note that `EngineCore::new` keeps its four-argument signature and Plan 2's own default (`Arc::new(Mutex::new(Box::new(ExplicitRanges { oop: None, ip: None })))` — verified against Plan 2's actual `ExplicitRanges` definition and default, which already use the same `Arc<Mutex<Box<dyn RangeSource>>>` shape, so this edit makes plan 3 consistent with plan 2's real code rather than plan 2's stale prose). The `ReplayRanges` struct body and its `impl RangeSource` in the Step 3 code block already declared `identity`, so no code-block edit was needed there — only the surrounding prose was out of date. |
| T17 / Step 3 startup paragraph (F02/F06/F07) — missing startup_report/banners and emit-import text | **APPLIED** | Appended to the same startup paragraph (after the F01 Paths fix): "Load the `PreflopStore` before handing `EngineCore` to `Engine::with_core`. Keep its `Arc` on `EngineCore` and clone it into `Engine` for `preflop_store()`. After construction, append every loader warning to `engine.startup.banners`; `startup_report()` is the single UI diagnostics surface and `Engine::new` still returns `Result<Engine, EngineError>`. Import `crate::serve::emit` in `preflop.rs`; do not create a second event-emission helper." Also added `emit` to Task 17's Interfaces Consumes list (`serve::{serve_request, LiveRequest, emit}`) so the import requirement is visible in both places. |

## 2. E06 items naming plan 3

| Stale subject/reference | Status | What changed |
|---|---|---|
| P3 says tools project was P1.T13 (L567, now L569) | **APPLIED** | "Plan 1 Task 13 already created `tools/pyproject.toml`..." -> "Plan 1 Task 15 already created `tools/pyproject.toml`...", matching the report's exact replacement ("Plan 1 Task 15 creates tools/pyproject.toml and tools/tests/conftest.py"). |
| P3.T19 says Plan 2 Recommendation serde | **APPLIED** | Task 19 Consumes line: "Plan 2 Recommendation serde and identity test harness" -> "Plan 1 Task 5's Recommendation serde, and Plan 2's identity/fake-worker test harness," matching the report's exact replacement. |
| P3's whole-plan prerequisite at L11 ("This is the third of five sequential implementation plans; Plans 1 and 2 must pass before execution begins.") | **APPLIED** | Replaced with the report's shared replacement text: "Execute tasks against the hard prerequisites in REVIEW-cross-plan-2 §7 after the documented seam corrections are applied. Every prerequisite must be implemented, green and reviewed before a dependent task starts. A plan number alone is not an execution dependency. Plan 5 Task 14 precedes the final Plan 4 Task 26 release decision. Serialize edits to shared files when working in one checkout, and serialize measured benchmark/acceptance runs on the benchmark machine." |
| Introductions refer to REVIEW-cross-plan §§4–5 as current order | **NOT APPLICABLE** | Grepped plan 3 for `REVIEW-cross-plan`, `§§4–5`, `§§4-5`: no matches. Plan 3's introduction never carried this stale reference, so there is nothing to replace. |
| P2 says snapshots swap is P3.T11; preflop startup P3.T14 | **NOT APPLICABLE (out of scope)** | This is a stale citation living in *Plan 2's* document (P2 says X about P3), not in plan 3's own text; plan 3 already correctly assigns the snapshot swap to Task 14 (module list at header L113) and startup loading to Task 17. Fixing Plan 2's citation is Plan 2's edit, outside this task's scope (plan 3 only). |
| P4 says SnapshotStore P3.T11 / SolvedStreetStore; P4 advertises free `register_snapshot` | **NOT APPLICABLE (out of scope)** | Both are stale citations in *Plan 4's* document. Plan 3's own Task 18 Step 5 already defines `Engine::register_snapshot(&mut self, active: &DecisionIdentity, snapshot: StreetSnapshot) -> bool` as a method, matching the report's confirmation in the F22 addendum ("no free function is promised by the producer"). No plan 3 edit needed or made. |

## 3. Findings F05, F06, F07, F11, F18, F22 — disposition

| Finding | Status for plan 3 | Reason |
|---|---|---|
| F05 — duplicate `range_vs_range` | **NOT APPLICABLE** | Grepped plan 3 for `range_vs_range`: no matches. The duplicate pair is P2.T25 (owner, `engine::equity::range_vs_range`, tuple-returning) vs P4.T11 (duplicate, `Option<f32>`-returning); section 5.1's D1 group and F05's concrete edit ("Delete P4's duplicate function") both name Plan 4, not Plan 3. Plan 3 implements no equity helper of this name, so the orchestrator's "remove the duplicate from plan 3 if plan 3 is the duplicate" condition does not hold — plan 3 is not the duplicate. |
| F06 — RangeSource shared ownership lost | **APPLIED** | See §1 above (header L114, T18 Interfaces/Step 3). |
| F07 — private `emit` consumed by a sibling module | **APPLIED (plan-3 side only)** | Plan 3 now documents `use crate::serve::emit` in `preflop.rs` and lists `emit` in Task 17's Consumes. The companion fix — making Plan 2's `fn emit(...)` `pub(crate) fn emit(...)` in `crates/engine/src/serve.rs` (P2.T28/Step 3) — is Plan 2's own document and is outside this task's scope (verified: Plan 2 currently still declares `fn emit(` privately at its Step 3, line ~6359; that edit belongs to a Plan 2 seam-re-check task, not this one). |
| F11 — four incorrect foundation API uses | **NOT APPLICABLE** | Grepped plan 3 for `core_iso::invert`, `SuitPerm::identity`, `postflop_order(state)`, `straddle.unwrap_or`: no matches. Plan 3 already uses the correct forms throughout (`cfg.straddle.as_ref().map(|s|s.amount_chips).unwrap_or(cfg.bb_chips)` at its straddle helper, `UtgStraddle{amount_chips:s}` in its straddle fixture builder). F11's four incorrect uses are all in Plan 4 (P4.T7/T9/T10/T11); plan 3 was never a source of this finding. |
| F18 — runtime staging assumes conditional chart availability | **NOT APPLICABLE** | F18 cites P3.T6/Interfaces:L875 and P3.T4/Interfaces:L729 as the *correct* evidence ("otherwise a recorded, tested absence that leaves the workspace green"; "Nothing downstream reads a chart bundle whose depth row is not 'available'"). The finding's concrete edit targets only P5.T5's staging script (hard-coded two-name loop, wildcard `LiteralPath` copy) — that is E05 (Plan 5), not a plan 3 edit. Plan 3's own chart-availability handling is already correct and required no change. |
| F22 — stale task numbers and purported exports | **APPLIED via §2 above; register_snapshot sub-note confirmed already correct** | F22's own citations (P2:L7062/L7065, P4.T11/T23 Interfaces) are inside Plan 2 and Plan 4, not plan 3, so its "Apply semantic table E06" instruction is what reaches plan 3 — covered by the three APPLIED rows in §2. Its addendum about `engine::snapshots::register_snapshot` vs `P3.T18/Step 5:L2956`'s `pub fn register_snapshot(...)` confirms plan 3's existing text is the correct producer signature; no plan 3 edit was needed or made for that sub-item. |

## 4. Section 4.3 "Plan 3's four named seams" — cross-check

| Seam | Report result | Plan 3 action |
|---|---|---|
| `engine::snapshots` re-export | "Correct staged replacement ... Retain `for_hand` compatibility for P2 tests." | **No edit** — verified `for_hand` (Task 14, `SnapshotStore::for_hand`) and `for_identity` (Task 14/17/18) both still present; the seam was already correct. |
| `engine::ranges::RangeSource::ranges_at_root` | "Exact name/signature/error exist. P3's shared field/init shape is wrong, F06." | **Edited** — see F06 above. |
| `serve_request` `Classification::Preflop` arm | "Correct hook. Shared `emit` needs `pub(crate)`, F07." | **Edited (plan-3 side)** — see F07 above; the hook/replacement line itself (Task 17 Step 3) was already correct and untouched. |
| `proto::resolve_chip_path` | "Single implementation ... No `core_replay::resolve_path` implementation." | **No edit** — verified plan 3's Interface-ownership section already states "Do **not** define a `core_replay::resolve_path`"; already correct. |

## 5. Verification

Documentation-only task — Rust/Python/UI execution not applicable. Grepped the revised plan for every finding's cited old text; all absent (exit code 1 / no matches for each):

```
grep -n "worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf\|Paths { worker, preflop, cache, log }" <plan>   -> no matches (F01)
grep -n "EngineCore.range_source: Box<dyn RangeSource>\|range_source: Box::new(ReplayRanges { store, snapshots, identity })" <plan>  -> no matches (F06)
grep -n "append \`crates/core-preflop\`\|append \`crates/core-replay\`" <plan>   -> no matches (F23)
grep -n "Plan 1 Task 13 already created" <plan>   -> no matches (E06 tools citation)
grep -n "Plan 2 Recommendation serde" <plan>   -> no matches (E06 T19 citation)
grep -n "Plans 1 and 2 must pass before execution begins" <plan>   -> no matches (E06 whole-plan prerequisite)
grep -n "range_vs_range" <plan>   -> no matches (F05 not applicable)
grep -n "core_iso::invert|SuitPerm::identity|postflop_order(state)|straddle.unwrap_or" <plan>   -> no matches (F11 not applicable)
```

And confirmed the replacement text is present (`log_dir: PathBuf, worker_exe: PathBuf`, `Arc<Mutex<Box<dyn RangeSource>>>`, `core.range_source.lock().unwrap()`, the glob-preserving Task 1/11 sentences, `Plan 1 Task 15 already created`, `Plan 1 Task 5's Recommendation serde`, `REVIEW-cross-plan-2 §7`, `` Import `crate::serve::emit` ``) at the lines listed in §1–§2 above.

## 6. Not applied

- F05, F11, F18 — not applicable to plan 3's own text (see §3); their concrete edits land in Plan 4 or Plan 5 documents, which are out of this task's scope.
- The E06 rows whose stale text lives inside *Plan 2's* or *Plan 4's* documents (snapshots-swap citation, SnapshotStore/register_snapshot citations) — out of scope for a plan-3-only edit; plan 3's own text for these subjects was already correct.
- Making Plan 2's `fn emit(...)` `pub(crate)` (F07's other half) — that edit is inside `docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md`, out of this task's scope (plan 3 only, no commit). Plan 3's own half (the `crate::serve::emit` import note) is applied and documented in §1/§3 above.

Nothing in this list was declined on the merits; each is either not a plan-3 defect or belongs to a different plan's document under this session's single-file edit scope.
