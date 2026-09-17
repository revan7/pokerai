# Plan 4 changelog 2 — seam re-check edits (2026-09-17)

**Document:** `docs/superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md`
**Resulting revision:** revision 2 (revision line added under the title).
**Driver:** `docs/research/REVIEW-cross-plan-2.md` §6 group **E04 — Plan 4**, every **E06** row naming plan 4, and the dispositions in §4.1 (plan 4's seven notes owed to plan 2) and §4.4 (surrogate and bench-oracle ownership).
**Binding orchestrator decisions applied:** plan 2's `engine::Paths { log_dir, worker_exe, preflop, cache }` and `Engine::new -> Result<Engine, EngineError>` + `startup_report()` stand and plan 4 adapts; the `Templates` registry names and feature flags match plan 2's revised names; a single `range_vs_range` owner; the section-6 surrogate honours its separate contract; presolver commands never lock the solve owner and the status path/helper creation order is fixed; `bench oracle` command names match real test targets.

Task numbering is unchanged (26 tasks). Every edit was applied to the task body **and** its matching Files/Interfaces/self-review claim.

---

## APPLIED

### 1. F01 / F22 — `Paths` shape and owner (E04 "Resolved upstream table/notes and T7 Paths consumption")
**APPLIED.** The resolved-upstream row and the T7 prose that read `Plan 2 Task 21: Paths { worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf }` now read `Plan 2 Task 29: Paths { log_dir: PathBuf, worker_exe: PathBuf, preflop: PathBuf, cache: PathBuf }`, with an explicit note that `log_dir` is a directory because `DecisionLog` appends `decisions.jsonl` inside it (P2.T21:L4821). T7 Step 5a's `DecisionLog::open(&paths.log)` line is gone. Why: the user's decision that plan 2's names stand.

### 2. F08 / §4.1 note 2 — `set_config` ownership (E04 "T9 / Step 3")
**APPLIED.** T9's replacement `Engine::set_config` body, `self.core.lock().unwrap().set_config(self.config.clone());` and the `Engine::new` sentence `core.set_config(cfg.clone())?` are deleted. Replaced with the E04 text: Plan 2 Task 29's `set_config`/`apply_config` bodies are preserved (blind/thread/flop validation, revision allocation, next-hand queuing, shared-config lock); `flop_budget_valid` is only a wrapper over the existing `FLOP_BUDGET_RANGE`; `Engine::config()` returns `queued_config.clone().unwrap_or_else(|| self.config.clone())`; new serving helpers consume the request configuration captured by `serve_request` and never lock `EngineCore` from a command. The "Plan 2 interface note (M4/M10/S15)" heading became "Upstream configuration seam (M4/M10/S15, resolved)". Why: F08 — plan 4's replacement lost blind validation and next-hand queuing, blocked behind a solve, and `core.set_config` returns `()` so the `?` was invalid.

### 3. F08 — request-captured `target_bp` (E04 "T10 / Step 4 and T11 / Step 3")
**APPLIED.** Every `core.config.solver.target_bp` in T10 is replaced by the request's captured `target_bp`. `target_bp: u16` was added to the parameters of `probe_cache`, `cache_phase` and `recommendation_from_hit`, and forwarded from `serve_request`'s existing `let config = core.config();` (`let target_bp = config.solver.target_bp;`). T10's Interfaces block states the new parameter. Configuration is not recaptured mid-decision. The surrogate also takes `target_bp` (see item 7).

### 4. F04 / §4.1 note 1 — template registry (E04 "T8 / Step 1a")
**APPLIED.** `TEST_TEMPLATES`, `register_test_template`, `test_extra` (both cfg arms) and the replacement `Templates::get` are deleted. The build closure stays inside `install_cache_test_templates`, now gated `#[cfg(any(test, feature = "test-templates"))]`, and its three `register_test_template` calls became one `Templates::with_extra(&[...])` call with the exact E04 argument list. The assertion became `assert_eq!(Templates::base_ids().len(), 9);` plus the `get`/`ids` loop. T8 Interfaces now produces only `install_cache_test_templates` and `CacheRig`; the "Plan 2 interface note (blocker B1)" heading became "Upstream seam (cross-plan M20/D10, resolved)". Every engine test command that builds `CacheRig` (`cache_key_structural_identity`, `cache_snapshot_replay`, `flop_path_golden`, `deadline_best_so_far_labelling`, and the T24 regression list) now passes `--features testing,test-templates`. Production template selection stays limited to its named production ids.

### 5. F11 — four foundation API names (E04 substitution table)
**APPLIED**, all four, at every occurrence, with the "if Plan 1 named it differently" hedges removed:
- `core_iso::invert(perm)` → `core_iso::inverse(perm)` (T7 Step 5 code and prose).
- `core_iso::SuitPerm::identity()` → `core_iso::SuitPerm::IDENTITY` (T10 Step 4 code and prose; prose now quotes `pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);`).
- `state.config.straddle.unwrap_or(state.config.bb_chips)` → `state.config.straddle.as_ref().map_or(state.config.bb_chips, |s| s.amount_chips)` (T9 `preflop_wagers`).
- `core_model::postflop_order(state)` → `core_model::postflop_order(state.button, &state.dealt)`, and the advertised signature `(&HandState) -> Vec<Seat>` → `(button: Seat, dealt: &[Seat]) -> Vec<Seat>` (T11 Step 3 code and prose).

### 6. F05 — duplicate `range_vs_range` (E04 "T11 / Interfaces + Step 3")
**APPLIED.** The entire `equity::range_vs_range(...) -> Option<f32>` definition in T11 Step 3 is deleted, together with its production claim in T11 Interfaces and the `crates/engine/src/equity.rs` entry in T11 Files. `choose_opponent`'s binding became `let Some((equity, _method)) = crate::equity::range_vs_range(range, hero_public, board, per_seat, cancel) else { continue };`, consuming Plan 2 Task 25's existing tuple-returning function. Why: D1 — one public function, one owner.

### 7. F12 — surrogate contract (E04 "T11 / Interfaces + Step 3", run_surrogate paragraph)
**APPLIED,** as an explicitly labelled binding contract repair rather than a claim that the sample already met it:
- `run_surrogate` now receives the actual hero cards, the hand's `bb_chips` and `rake`, and the request-captured `target_bp`, in addition to its existing synthetic input, ranges, board, deadline, identity and sink; the hero cards never condition the public solve ranges.
- The synthetic tree is still built from pot, equal effective stacks and empty history, but **`SolveInput` is never constructed** (spec S:L423). T11 now factors Plan 2's worker request/response, validation, heartbeat, cancellation and deadline handling below `run_solve` into a shared internal transport (`solve::{solve_request_from_parts, send_solve_request}`), and the surrogate builds its worker `SolveRequest` directly; `run_solve` remains the main-path `SolveInput` adapter. `crates/engine/src/solve.rs` was added to T11 Files (replacing `equity.rs`).
- `actions: crate::assemble::advice_rows(node, &[], 2)` and the paragraph describing `advice_rows` as "Plan 2 Task 19's row builder" are deleted; `advice_rows` does not exist upstream (F22/E06). Advice is now extracted explicitly: validate the whole solution, resolve the root or check child for hero's role, select the actual hero combo row, and use `ev_chips / bb_chips` once; fold EV stays 0 exactly and unavailable combos skip the surrogate.
- The literal `bb_chips: 2` became the hand's real `bb_chips`; `Rake::TimeCharge` became the hand's real `rake`.
- Output still bypasses cache and snapshot registration; the main recommendation stays `Unsupported{MultiwayEv}` with `actions` empty.
- Added the required non-2-BB, IP-after-check, actual-combo regression (`surrogate_uses_real_bb_and_the_actual_hero_combo_when_ip`) alongside the existing isolation golden, with its two new support helpers declared.

### 8. F13 — V3 policy evidence (E04 "T9 / Step 3 + Files/Interfaces, T10 / Step 4")
**APPLIED.** `core.bench_p95_ms.get("flop_min_v1@100bb")`/`@200bb` in T10 Step 4 is replaced by `let policy = &core.flop_policy;`, whose `live_template` result is used. T9 now owns `V3PolicyEvidence`, `load_v3_policy(path, expected_provenance)` and `EngineCore.flop_policy`, with the exact E04 contract text (template signature, solver commit, adapter/rules versions, storage-mode policy, source-lock hash, machine identity, finite positive p95 at both depths; conservative `FlopPolicy::from_v3(None, None)` plus a startup diagnostic on any mismatch; `EngineCore::new` initialises the conservative value; `Engine::new` loads evidence off the recommendation path; Task 24 serializes the matching evidence; Task 9's tests use synthetic evidence; no dependency cycle on Task 24). `FlopPolicy` gained `Clone, Copy, Debug, PartialEq` so it can be stored. T9 Files gained `core.rs`; T10 Interfaces records the consumption. The vague sentence "Load V3 admission only from a report matching the exact template signature…" is replaced by the explicit contract.

### 9. F09 / F10 — presolver ownership and status path (E04 "T7 / Step 5a and T15 / Step 3–4 and T16 / Step 4")
**APPLIED,** with the full E04 ownership contract quoted into T16:
- **T7** no longer adds `EngineCore.presolver`; it adds only `EngineCore.cache` and `with_cache` (initialised to `Cache::disabled()`), and says so explicitly. The file-structure row for `core.rs` changed from `EngineCore.presolver` to `EngineCore.flop_policy`.
- **T15** now defines `cache::presolver::remaining_seconds` in `presolver/mod.rs` **before** the scheduler uses it, and adds `pub use scheduler::PresolverStatus;` so `cache::presolver::PresolverStatus` is the path Plan 5 and Task 16 name. Its Interfaces block and the status round-trip test assert both.
- **T16** adds `Engine.presolver: Option<Arc<cache::presolver::scheduler::Presolver>>`, initialised to `None` by `with_core` and populated once in production startup. `presolver_status`/`pause`/`resume`/`notify_presolver_hand` use that handle directly with the exact E04 method bodies and **never** lock `EngineCore`; `presolver_status` returns `cache::presolver::PresolverStatus`. `EngineExecutor` holds a command/completion endpoint to the existing single worker owner, not an `Arc<Mutex<EngineCore>>`. The shutdown order now signals background cancellation first, then joins `engine-main`, the presolver, the cache threads and the worker, extending Plan 2 Task 29's idempotent body without changing the receiver.
- **T16 Step 5** no longer redefines `remaining_seconds`; `engine::log` only re-exports it (`pub use cache::presolver::remaining_seconds;`).
- Added T16's required red test `presolver_commands_never_wait_for_a_running_solve`: a worker held in an indefinitely running solve while status/pause/live notification return promptly, and background cancellation recorded before the live send.

### 10. F02 — startup diagnostics (E04 closing paragraph of the T20/T24 item)
**APPLIED.** T7 Step 5a's `Engine::new` snippet now installs the cache inside Plan 2 Task 29's existing `Engine::new` and pushes cache availability/error diagnostics onto `StartupReport.banners`/`cache_state`. The prose states that `Engine::new` keeps returning `Result<Engine, EngineError>` and never gains a second return value.

### 11. F14 — bench spot DTO (E04 "T20 / Step 3 + Interfaces/Files")
**APPLIED.** `generate_flop_spots(...) -> Result<Vec<FlopBenchSpot>, EngineError>` and `generate_street_spots(...) -> Result<Vec<Spot>, EngineError>` are replaced by the engine-owned `PreparedBenchSpot` DTO and both generators, verbatim from E04. `FlopBenchSpot` is deleted and the plan states that engine never imports `bench::suite::Spot`. `suite.rs` and `runner.rs` were added to T20's Modify list, with the E04 adaptation paragraph: `Spot`'s `oop_range`/`ip_range` become a serde-untagged `RangeSpec { Text(String), Weights(Range1326) }` (declared in the plan), R8 inputs stay `Text` and use `prepared_range`, chart inputs serialize full `Weights` arrays, source hashes / inherited reasons / explicit unavailable reason are added, an unavailable prepared spot has `input = None` and is never executed or counted, and `Suite::load`/`save`, `run_spot` and `gen-spots` are updated together.

### 12. F17 / F16 — oracle command names (E04 "T23 / Step 3a")
**APPLIED.** `--test river_analytic` and `--test ev_contracts` are replaced by one `contract_river` suite (Plan 2 Task 15's real target) plus the engine check-only suite `cargo test -p engine --release --test worker_link river_check_only_terminal_oracle -- --exact` (Plan 2 Task 18's real target). The `REQUIRED_SUITES` name list and its assertion in the red test were updated to match. `run_oracles` now actually runs `tools/.venv/Scripts/python tools/gen_eval_oracle.py --skip-5card --samples 10000000 --samples-name phevaluator_7card_10m.bin` when the fixture is absent, checks its exit code and returns early on failure — replacing the previous comment that merely promised generation. Full command, exit code and executed-test count are recorded, and a zero-test/ignored/skipped run cannot satisfy `analytic_oracles_ok`; the five-card committed oracle remains required. The worker command is dispatched through the V1-selected toolchain/target with `POKERAI_WORKER` set to the selected executable.

### 13. F21 — chart UI acceptance evidence (E04 "T23 / Step 3–5 and T26 / Interfaces/Step 1–2")
**APPLIED.** `GateInput` gained `chart_ui_e2e_ok: bool`, documented as sourced only from Plan 5 Task 14's actual named test result with the release candidate's worker/source/config provenance, missing evidence being `false`. `evaluate` gained the check `("chart UI E2E", g.chart_ui_e2e_ok)`, and the mutation test `missing_chart_ui_evidence_fails_v22` was added. The report section and a new "V21/V22 split" note state that V21 and the bench-only bounds may be reported independently while V22 additionally requires the UI evidence. Task 26 now consumes Tasks 24–25 **and** Plan 5 Task 14, states that Plan 5 Task 14 precedes it and that Plan 5 may start from supplied surfaces without waiting for this gate, reruns `bench gate` and records the final V21/V22 decision as the sole V22 authority.

### 14. F24 / F16 — duplicate artifact creation and worker toolchain (E04 "T20 Files, T24 Files/Step 2")
**APPLIED.** The six suite paths in T20 Files changed from Create to **modify/regenerate**, naming Plan 2 Task 5's `bench gen-spots --source r8 --out bench/spots` as their creator and stating this is intended replacement of the R8 data. `docs/bench/2026-09-10-i7-13700K.md` in T24 Files changed from Create to **modify/append**, naming Plan 2 Task 30 Step 4 as its creator and requiring R8-labelled rows to be retained as separate reference evidence. The file-structure table carries the same Modify/regenerate and Modify/append labels. T24 Step 2's forced-MSVC build block is replaced by the V1-selection-driven PowerShell block (reads `docs/bench/worker-toolchain.json`, GNU or MSVC branch, throws on a missing/invalid selection) plus the E04 text: production and diagnostic workers on the V1-selected toolchain/target, bench and UI on MSVC, a separate target directory for the diagnostic build, the selected toolchain reported on every measured row, all six chart suites regenerated before baseline measurements, and no `rustup default`/config mutation. T24 Step 2 also now serializes the matching `V3PolicyEvidence`.

### 15. E06 — stale cross-references naming plan 4
**APPLIED,** by subject, in the resolved-upstream table, the file-structure table, task Interfaces blocks and the self-review:
| Old subject | New text |
|---|---|
| Paths / `set_config` / `shutdown` / module list "complete at P2.T21" | "Plan 2 Task 29 owns the public Engine façade; Task 21 owns DecisionLog only." (plus the module list now says "after its Task 29") |
| deadline seam P2.T16 | "Plan 2 Task 20 owns Deadlines and watchdog arithmetic." |
| `bench_support.rs` P2.T22 with `parse_range`/`block_public`/`hash_scaled` re-exports | "Plan 2 Task 5 creates `engine::bench_support::{prepared_range, range_mass}`; later tasks extend this file." (three sites: table, file-structure row, T17 Files, self-review placeholder scan) |
| assembly/advice rows P2.T19 | "Plan 2 Task 26 owns `final_from_solution` and the main recommendation assembler; `advice_rows` is not an upstream API." |
| test fakes / `testing` feature P2.T15 | "The `testing` feature is declared in Plan 2 Task 2; FakeClock/FakeWorker/RecordingSink are produced in Task 19." |
| SnapshotStore P3.T11 / `SolvedStreetStore` | "Plan 3 Task 13 defines snapshot records; Task 14 replaces Plan 2 Task 27's `SolvedStreet` and `SnapshotStore` with `core_replay` types." |
| free `register_snapshot` / `engine::snapshots::register_snapshot` | "`Engine::register_snapshot(&mut self, &DecisionIdentity, StreetSnapshot) -> bool` is the Plan 3 Task 18 façade; live/cache internals use the same `SnapshotStore::register`." (public-surface block, T12 Interfaces, self-review) |
| Introduction citing REVIEW-cross-plan §§4–5 as current order | "REVIEW-cross-plan §§4–5 are historical. Use REVIEW-cross-plan-2 §7 after its listed corrections are accepted." |
| "Execute after Plans 1–3" | The E06 task-level prerequisite paragraph, including "Plan 5 Task 14 precedes the final Plan 4 Task 26 release decision." |

Also corrected: `bench oracle` is created by Plan 4 Task 23 (§4.4), with the explicit note that Plan 1 Task 22 supplies fixture generation and Plan 2 Tasks 5/30 implement `materialize`/`gen-spots`/`run`, not `oracle`.

### 16. §4.1 — the seven notes owed to Plan 2
**APPLIED.** The header section "Interface notes owed to Plan 2 (its cross-plan fix must land first)" is replaced by "Interface notes owed to Plan 2 — all seven are satisfied by the revised Plan 2", a disposition table stating plan 2's declared shape and plan 4's obligation for each note, opening with "Nothing in this list is still owed." Note 6 keeps the review's caveat that field availability is not a working background scheduler and points at the Task 16 ownership contract. The self-review's repeated list was rewritten to match ("Interfaces owed to Plan 2: none remain.").

### 17. Self-review consistency claims
**APPLIED.** Per the report's finding that plan 4's self-review overclaims consistency (§5.3: "Claims of exact type consistency are disproved by F04–F14"), the **Type consistency** paragraph is rewritten to label itself a revision-2 correction set and to state each corrected fact with its finding id (F04, F05, F08, F09, F10, F11, F12, F13, F14, F22). **Coverage gaps** now states that release completeness is not claimed from bench data alone and that V22 additionally requires Plan 5 Task 14's evidence (F21), and replaces "the actual MSVC build" with "the actual worker build on the V1-selected toolchain" (F16). The **Placeholder scan** `bench_support.rs` owner was corrected to Plan 2 Task 5.

### 18. Revision line
**APPLIED.** Added under the title: `Revision 2 (2026-09-17): seam re-check edits E04/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-4-CHANGELOG-2.md`.

---

## NOT APPLIED

### F15 — undeclared workspace dependency `ts-rs`
**NOT APPLIED — not a plan 4 edit.** The review cites `P4.T1/Step 3:L182: ts-rs = { workspace = true, optional = true }` as the *consumer*, but §5.1's resolution is "Add root key" and the edit text lives in **E01 (Plan 1)**: add `hex = "0.4"` and `ts-rs = "=12.0.1"` to Plan 1 Task 1's `[workspace.dependencies]`. Plan 4's inheriting entry is correct as written once Plan 1 declares the key. Editing plan 4 here would create a second dependency owner. Requires the plan 1 edit (E01) to land.

### F19 — `flop_full_v1` five-point materialization sweep
**NOT APPLIED — plan 2 edit (E02).** The 43→47 case change and the generator `TEMPLATES` list belong to Plan 2 Task 6/T8/T16 and its self-review. Plan 4 contains none of the cited text.

### F20 — BLOCKER, two wire outcomes for one invalid donk input
**NOT APPLIED — spec authority decision, not a plan 4 edit.** The review's own text states the contradiction is inside spec revision 6 and that resolving it "would require a separately authorized spec revision/changelog"; the affected plan text is Plan 2 Task 8/T13/T16. Plan 4 is unaffected either way.

### F23 — redundant glob-covered workspace members
**NOT APPLIED — plan 3 edit (E03).** Plan 4's Task 1 modifies the root `Cargo.toml` for the cache member, which the review's §5.2 workspace row does not flag; the redundant explicit member entries are Plan 3 Task 1 and Task 11.

### F03, F06, F07, F18
**NOT APPLIED — plan 2 / plan 3 / plan 5 edits.** F03 (BeginHand conversion) is E05; F06 (shared `RangeSource` Arc) and F07 (`pub(crate) emit`) are E02/E03; F18 (runtime chart staging) is E05. None cites plan 4 text.

### F16 — the V1 measurement itself
**PARTIALLY APPLIED (the plan 4 half).** Plan 4 now consumes the V1 selection in Tasks 23 and 24 as E04 requires. Creating and persisting the selection (`docs/bench/worker-toolchain.json`, the FLOP-FAST measurement convention, the 1.25× rule) is **E02, Plan 2 Task 1**, and is not applied here. Plan 4's consumption is inert until that file exists — which is exactly the E04 behaviour: missing selection evidence blocks later worker timing and never silently defaults to MSVC.

---

## Verification

Documentation-only task. **Rust/Python/UI execution: not applicable** — no workspace exists (spec/plan documents only). The named check is a grep for each finding's cited old text in the edited plan; the actual output is quoted in the task report and reproduced here.

Every cited old text greps to **0 matches** in `docs/superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md`:

```
F01    Paths { worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf } -> GONE (0)
F01    DecisionLog::open(&paths.log)                                             -> GONE (0)
F04    Self::all().iter().find(|t| t.id == id).or_else(|| test_extra(id))        -> GONE (0)
F04    `Templates::ids().len()` is still 9                                       -> GONE (0)
F05    equity::range_vs_range(&Range1326,&Range1326,&[Card],Duration,&AtomicBool)->Option<f32> -> GONE (0)
F08    self.core.lock().unwrap().set_config(self.config.clone());                -> GONE (0)
F08    target_bp:core.config.solver.target_bp                                    -> GONE (0)
F08    core.set_config(cfg.clone())?                                             -> GONE (0)
F09    self.core.lock().unwrap().presolver.as_ref()                              -> GONE (0)
F11    core_iso::invert(perm)                                                    -> GONE (0)
F11    core_iso::SuitPerm::identity()                                            -> GONE (0)
F11    state.config.straddle.unwrap_or(state.config.bb_chips)                    -> GONE (0)
F11    core_model::postflop_order(state)                                         -> GONE (0)
F12    actions:crate::assemble::advice_rows(node,&[],2)                          -> GONE (0)
F12    let solve=SolveInput{root:root.clone(),ranges:ranges.clone(),...          -> GONE (0)
F13    core.bench_p95_ms.get("flop_min_v1@100bb")                                -> GONE (0)
F14    ->Result<Vec<Spot>,EngineError>                                           -> GONE (0)
F14    FlopBenchSpot{line_id:String,depth_bb:u16,pot_class:String,input:SolveInput,...} -> GONE (0)
F17    "--test","river_analytic"                                                 -> GONE (0)
F17    "--test","ev_contracts"                                                   -> GONE (0)
F21    Consumes the complete report of Tasks 24–25 and `bench gate`.             -> GONE (0)
F22    `engine::snapshots::register_snapshot` is the single registration path     -> GONE (0)
F24    **Files:** Create `docs/bench/2026-09-10-i7-13700K.md`                     -> GONE (0)
```

E06 stale references, same file, same result:

```
Plan 2 Task 21                             -> GONE (0)
Plan 2 Task 16                             -> GONE (0)
Plan 2 Task 22 creates                     -> GONE (0)
Plan 2 Task 19's row builder               -> GONE (0)
the `testing` feature from Plan 2 Task 15  -> GONE (0)
Plan 3 Task 11                             -> GONE (0)
SolvedStreetStore                          -> GONE (0)
Execute after Plans 1–3                    -> GONE (0)
REVIEW-cross-plan.md` §§1, 2, 4, 5         -> GONE (0)
with the pinned MSVC toolchain and regenerate the suites -> GONE (0)
^cargo build --release -p solver-worker (unconditional)  -> none
^pub fn remaining_seconds  -> exactly 1 definition, at line 2762 (Task 15)
```

Replacement anchors present (occurrence counts):

```
Revision 2 (2026-09-17)                    1     PreparedBenchSpot                 6
log_dir, worker_exe, preflop, cache        3     contract_river                    5
Templates::with_extra                      10    river_check_only_terminal_oracle  2
core_iso::inverse(perm)                    1     chart_ui_e2e_ok                   5
SuitPerm::IDENTITY                         3     pub use scheduler::PresolverStatus 3
postflop_order(state.button,&state.dealt)  1     pub(crate) presolver: Option      1
amount_chips                               2     worker-toolchain.json             2
V3PolicyEvidence                           7     gen_eval_oracle.py                1
Plan 2 Task 29  19   Plan 2 Task 20  4   Plan 2 Task 26  3   Plan 2 Task 5  13   Plan 3 Task 13  2
```

Structure preserved: `grep -c '^### Task '` = **26** (numbering unchanged, 1–26); `grep -c '^```'` = **288** (144 balanced fences); every task retains Files, Interfaces, red/green steps and one commit with the required trailer.
