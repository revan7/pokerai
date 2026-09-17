# Final cross-plan seam re-check — 2026-09-17

**READY FOR EXECUTION: NO.** The revised plans still contain 24 finding groups: 1 BLOCKER, 20 MAJOR and 3 MINOR. There are 3 duplicate-ownership groups (8 affected paths/symbols) and 12 orphan advertised names/dependency keys/test targets, itemized below. These categories overlap; they are not additive. The blocker is a contradiction inside spec revision 6, not an unavailable implementation tool.

Review steps completed: **5/5**. Tasks inventoried: **114/114** (25 + 30 + 19 + 26 + 14). Only this report is changed. No implementation, spec/plan edit, changelog, daily-log update, installation or commit was performed; the request’s report-only rule overrides the general log rule.

## 1. Authorities, method and citation key

Read CLAUDE.md and AGENTS.md, then the original [cross-plan review](REVIEW-cross-plan.md), all five revised plans, and the requested spec sections (3.2, 3.5, 4, 6, 7, 10.4, 10.5, 13, 14). Original review §§1–3 were treated as historical resolutions to verify; §§4–5 were excluded as stale scheduling authority. The user’s two explicit decisions — Plan 2’s Paths names and startup_report API — control this review.

| Key | Source | Reviewed size / tasks |
|---|---|---|
| P1 | [2026-09-10-plan-1-foundation.md](../superpowers/plans/2026-09-10-plan-1-foundation.md) | 5436 lines; 25 tasks |
| P2 | [2026-09-10-plan-2-worker-engine.md](../superpowers/plans/2026-09-10-plan-2-worker-engine.md) | 7099 lines; 30 tasks |
| P3 | [2026-09-10-plan-3-preflop-replay.md](../superpowers/plans/2026-09-10-plan-3-preflop-replay.md) | 3172 lines; 19 tasks |
| P4 | [2026-09-10-plan-4-flop-cache-presolver.md](../superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md) | 4062 lines; 26 tasks |
| P5 | [2026-09-10-plan-5-ui.md](../superpowers/plans/2026-09-10-plan-5-ui.md) | 3153 lines; 14 tasks |
| S | [2026-09-10-pokerai-assistant-design.md](../superpowers/specs/2026-09-10-pokerai-assistant-design.md) | 807 lines, revision 6 |

Locations below refer to the **reviewed Markdown**, not nonexistent future Rust source. `P4.T11/Step 3:L2051` means Plan 4, Task 11, Step 3, line 2051. Every quoted fragment was located in its source; no quote exceeds 30 whitespace-delimited words. Public interface blocks, implementation snippets, manifests, file ownership, tests and self-review claims were cross-checked. A matching name alone was insufficient: receiver, argument/return/error types, visibility, feature gate, initialization and ownership also matter.

“NO” is deliberate: F20 needs an authority decision, and F09/F12/F13/F14 require concrete interface work beyond spelling fixes. The edit text in §6 is a proposed correction set, **not changes already applied**. The execution graph in §7 is conditional on that set and the F20 decision.

## 2. Findings: evidence on both sides

### F01 — MAJOR — Paths fields and log-directory semantics

- **P2.T29/Interfaces:L6418**: “pub struct Paths { pub log_dir: PathBuf, pub worker_exe: PathBuf, pub preflop: PathBuf, pub cache: PathBuf }”
- **P3.T17/Interfaces:L2702**: “Paths { worker, preflop, cache, log }”
- **P4:L63**: “`Paths { worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf }`”
- **P5.T5/Step 4:L1119**: “log: local.join("decisions.jsonl")”

**Concrete edit / consequence:** P2 wins. Rename worker/log in P3–P5; pass the directory to log_dir because DecisionLog appends decisions.jsonl (P2.T21:L4821). E03/E04/E05.

### F02 — MAJOR — Startup result and warnings

- **P2.T29/Interfaces:L6422**: “pub fn new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError>;”
- **P2.T29/Interfaces:L6424**: “pub fn startup_report(&self) -> StartupReport;”
- **P5.T5/Step 5a:L1218**: “let (engine,engine_warnings)=engine::Engine::new(config.clone(),paths)”

**Concrete edit / consequence:** P5 destructures a non-tuple. Use startup_report().banners; P3/P4 populate that same report. E03/E04/E05.

### F03 — MAJOR — BeginHand conversion at the wrong boundary

- **P2.T29/Interfaces:L6426**: “pub fn begin_hand(&mut self, req: proto::BeginHand) -> Result<HandState, EngineError>;”
- **P5.T4/Step 3:L925**: “let core = core_model::BeginHand { hand_id: 0, button: b.button, hero: b.hero,”

**Concrete edit / consequence:** P2 already assigns IDs and converts at L6704–6707. Pass b directly; remove the shell conversion and core-model dependency. E05.

### F04 — MAJOR — Template registry targets obsolete names and features

- **P2.T2/Step 6:L494**: “pub fn with_extra(extra: &[TemplateSpec])”
- **P2.T2/Step 6:L508**: “pub fn base_ids()”
- **P4.T8/Step 1a:L1367**: “Self::all().iter().find(|t| t.id == id).or_else(|| test_extra(id))”
- **P4.T8/Step 1a:L1371**: “`Templates::ids().len()` is still 9”

**Concrete edit / consequence:** P2 has base(), not all(); ids() includes extras. Use the existing test-templates/with_extra seam and base_ids(). E04.

### F05 — MAJOR — Duplicate range_vs_range with incompatible result

- **P2.T25/Interfaces:L5536**: “range_vs_range(hero_public: &Range1326, opp_public: &Range1326, board, budget, cancel) -> Option<(f32, EquityMethod)>”
- **P4.T11/Interfaces:L1988**: “equity::range_vs_range(&Range1326,&Range1326,&[Card],Duration,&AtomicBool)->Option<f32>”

**Concrete edit / consequence:** Delete P4’s duplicate function and consume P2.T25, destructuring Some((equity, _method)). E04.

### F06 — MAJOR — RangeSource shared ownership is lost

- **P2.T27/Interfaces:L5952**: “range_source: Arc<Mutex<Box<dyn RangeSource>>>”
- **P3.T18/Step 3:L2923**: “`EngineCore::new` sets `range_source: Box::new(ReplayRanges { store, snapshots, identity })`”

**Concrete edit / consequence:** Preserve the shared Arc/Mutex and install the replay implementation after loading the store, before engine-main starts. Add identity to the advertised ReplayRanges fields. E03.

### F07 — MAJOR — Private emit consumed by a sibling module

- **P2.T28/Step 3:L6269**: “fn emit(core: &EngineCore, req: &LiveRequest, delivered: Option<&AtomicBool>, ev: RecommendationEvent)”
- **P3.T17/Step 3:L2769**: “emit(core,req,None,RecommendationEvent::Fast(”

**Concrete edit / consequence:** Make the P2 helper pub(crate), and import it in P3’s new preflop.rs. Keep the identity check. E02/E03.

### F08 — MAJOR — Configuration regression and wrong shared-field access

- **P2.T27/Interfaces:L5952**: “config: Arc<Mutex<GameConfig>>”
- **P2.T29/Step 4:L6697**: “if self.state.is_some() { self.queued_config = Some(stamped); } else { self.apply_config(stamped); }”
- **P4.T9/Step 3:L1575**: “self.core.lock().unwrap().set_config(self.config.clone());”
- **P4.T10/Step 4:L1709**: “target_bp:core.config.solver.target_bp”

**Concrete edit / consequence:** P4 loses blind validation/next-hand queuing, blocks behind a solve and fails field access. Keep P2’s setter and pass its captured request config to new helpers. core.set_config returns (), so P4:L1581’s ? is also invalid (P2:L6120). E04.

### F09 — MAJOR — Presolver commands lock the solve owner

- **P2.T29/Step 4:L6647**: “Only `shutdown` locks the `EngineCore` itself”
- **P2.T29/Step 4:L6680**: “serve_request(&mut c2.lock().unwrap(), req);”
- **P4.T16/Step 4:L2953**: “self.core.lock().unwrap().presolver.as_ref()”

**Concrete edit / consequence:** Status/pause/resume/hand notification can wait for a whole solve. Keep the presolver handle on Engine independently of the worker owner; marshal background jobs through the existing owner. E04.

### F10 — MAJOR — Presolver status path and helper creation order

- **P5:L72**: “Engine::presolver_status(&self) -> cache::presolver::PresolverStatus;”
- **P4.T15/Step 3:L2683**: “pub struct PresolverStatus {”
- **P4.T15/Step 4:L2807**: “estimated_remaining_s:crate::presolver::remaining_seconds(pending,measured),”
- **P4.T16/Step 5:L2976**: “pub fn remaining_seconds(pending:u32,measured_p50:Option<f64>)->Option<f64>”

**Concrete edit / consequence:** Add the parent status re-export. Create remaining_seconds in T15, not T16. Move T7’s premature presolver field to T16 per F09. P5 must assign its three presolver methods to P4.T16. E04/E05.

### F11 — MAJOR — Four incorrect foundation API uses

- **P1.T3/Step 3:L487**: “pub struct UtgStraddle { pub amount_chips: u32 }”
- **P4.T9/Step 3:L1548**: “state.config.straddle.unwrap_or(state.config.bb_chips)”
- **P1.T21/Step 3:L4298**: “pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);”
- **P4.T10/Step 4:L1772**: “core_iso::SuitPerm::identity()”
- **P1.T9/Step 3:L1844**: “pub fn postflop_order(button: Seat, dealt: &[Seat]) -> Vec<Seat>”
- **P4.T11/Step 3:L2107**: “core_model::postflop_order(state)”
- **P1.T21/Step 3:L4320**: “pub fn inverse(”
- **P4.T7/Step 5:L1252**: “inverse_perm:core_iso::invert(perm)”

**Concrete edit / consequence:** Use amount_chips, SuitPerm::IDENTITY, core_iso::inverse and postflop_order(state.button, &state.dealt). E04.

### F12 — MAJOR — Surrogate violates its separate contract and consumes absent advice_rows

- **S:L423**: “It never goes through `SolveInput`, the cache or replay snapshots”
- **P4.T11/Step 4:L2132**: “let solve=SolveInput{root:root.clone(),ranges:ranges.clone(),tree:build.tree.clone(),”
- **P4.T11/Step 4:L2149**: “actions:crate::assemble::advice_rows(node,&[],2)”
- **P2.T26/Step 2:L5884**: “pub fn final_from_solution(ctx: &AssemblyCtx, node: &NodeStrategy, reach: &[f32], coverage: Coverage, mut assumptions: Assumptions) -> Recommendation”

**Concrete edit / consequence:** No plan produces advice_rows; cited P2.T19 produces test doubles. The call has neither actual hero combo nor real BB. Introduce a separate synthetic solve request and explicit advice extraction; retain transport/deadline controls. E04.

### F13 — MAJOR — V3 policy reads an unowned field

- **P4.T10/Step 4:L1758**: “core.bench_p95_ms.get("flop_min_v1@100bb")”
- **P2.T22/Interfaces:L4864**: “memory_limit_bytes: u64, stage: Arc<Mutex<String>>”
- **P4.T9/Step 3:L1581**: “Load V3 admission only from a report matching the exact template signature”

**Concrete edit / consequence:** No task creates/loads bench_p95_ms; two unscoped floats cannot validate provenance. P4.T9 must own a policy record, loader and field with a conservative missing/stale default. E04.

### F14 — MAJOR — Engine facade returns bench-owned Spot with incompatible range storage

- **P2.T5/Interfaces:L1103**: “bench::suite::{Spot { id: String, template_id: String”
- **P2.T5/Interfaces:L1103**: “oop_range: String, ip_range: String”
- **P4.T20/Interfaces:L3482**: “generate_street_spots(street:Street,template:&str,source:&Path)->Result<Vec<Spot>,EngineError>”
- **P4.T20/Step 3:L3517**: “Encode full 1326 vectors rather than substituting uniform chart-like strings.”

**Concrete edit / consequence:** bench depends on engine. Define the DTO in engine, return it from the facade, and adapt bench’s suite schema and runner to vectors. E04.

### F15 — MAJOR — Undeclared workspace dependencies

- **P1.T1/Step 1:L115**: “sha2 = "0.10.9"”
- **P2.T2/Step 1:L237**: “hex.workspace = true”
- **P4.T1/Step 3:L182**: “ts-rs = { workspace = true, optional = true }”
- **P5.T1/Step 4:L307**: “version = "=12.0.1"”

**Concrete edit / consequence:** P1’s root table declares neither hex nor ts-rs. P5 adds only a proto-local ts-rs table. Optional workspace inheritance still needs its key during manifest parsing. E01/E05.

### F16 — MAJOR — Unowned V1 toolchain measurement and broken fallback propagation

- **P1.T1/Step 3:L197**: “The **V1 check in plan 2** must therefore build and run the pinned-solver example on MSVC”
- **P2:L17**: “Plan 1's V1 check builds the vendored solver on MSVC”
- **P2.T1/Step 5:L194**: “the two measured `vendor_smoke --release` wall times”
- **P5.T5/Step 6:L1259**: “cargo +stable-x86_64-pc-windows-msvc build --release -p solver-worker --target x86_64-pc-windows-msvc”

**Concrete edit / consequence:** P2 consumes a nonexistent earlier check, and smoke wall time is not FLOP-FAST. P2.T7’s MSVC workspace command and P4.T24/P5.T5 rebuilds defeat the build-failure fallback. Assign and persist the actual gate, then reuse it in tests/builds/oracles/staging. E01/E02/E04/E05.

### F17 — MAJOR — Oracle command names absent test targets

- **P2.T15/Interfaces:L3497**: “`solver-worker/tests/contract_river.rs`”
- **P4.T23/Step 3a:L3820**: “args:&["test","-p","solver-worker","--release","--test","river_analytic"]”
- **P4.T23/Step 3a:L3822**: “args:&["test","-p","solver-worker","--release","--test","ev_contracts"]”
- **P2.T18/Interfaces:L3978**: “`worker_link.rs::{process_worker_spawns_validates_ready_and_restarts, ready_validation_rules, river_check_only_terminal_oracle}`”

**Concrete edit / consequence:** Use contract_river, add the engine check-only test, and actually run P1’s 10M oracle generator before exhaustive tests. P4:L3828–3843 merely promises generation in a comment. E04.

### F18 — MAJOR — Runtime staging assumes conditional chart availability

- **P3.T6/Interfaces:L875**: “otherwise a recorded, tested absence that leaves the workspace green.”
- **P3.T4/Interfaces:L729**: “Nothing downstream reads a chart bundle whose depth row is not `"available"`.”
- **P5.T5/Step 6:L1264**: “foreach ($name in @('pokercoaching_100','rangeconverter_200'))”

**Concrete edit / consequence:** Stage only available bundle/manifest pairs from sources.manifest.json, retaining absence warnings. Replace the LiteralPath wildcard at L1278 with file enumeration. Required release cells still fail when a depth is absent. E05.

### F19 — MAJOR — Four required flop_full materialization cases omitted

- **S:L733**: “**(a) In-process equality over the 47 boundary cases**”
- **P2.T6/Step 3:L1628**: “single small rules case instead of the five-point sweep”
- **P2.T6/Step 3:L1637**: “cases.append(("flop_full_v1_100_100", "flop_full_v1", 100, 100, []))”

**Concrete edit / consequence:** Put flop_full_v1 in the five-point loop, delete its singleton, update 43 to 47. Phase-2 solving does not waive the explicit rules contract. E02.

### F20 — BLOCKER — Spec requires two wire outcomes for one invalid donk input

- **S:L733**: “a `None` donk option and a missing terminal marker each produce `result{error{tree_mismatch}}`”
- **S:L744**: “`None` donk option in `tree`: typed rejection with `reason`, no work, worker stays alive”
- **P2.T8/Interfaces:L2028**: “This plan picks the **cheap** answer:”

**Concrete edit / consequence:** Proposed decision: later-street None is ack rejected/no work; root None stays legal; tree_mismatch covers altered materialization/terminal markers. The authority must resolve the contradiction; this report does not silently adopt one branch. E02.

### F21 — MAJOR — Final V22 has no chart UI acceptance input

- **S:L784**: “section 13.5 baseline gate on the baseline suites and the chart E2E”
- **P4.T26/Interfaces:L4003**: “Consumes the complete report of Tasks 24–25 and `bench gate`.”
- **P5.T14/Step 5:L3118**: “Do not claim V22 passes unless Plan 4's independent bench gates also passed.”

**Concrete edit / consequence:** GateInput has no UI evidence field (P4:L3773–3802). Add one; P5.T14 precedes P4.T26. Permit P5 to start from supplied surfaces without waiting for the final P4 gate. E04/E05.

### F22 — MINOR — Stale task numbers and purported exports

- **P2:L7062**: “**plan 4 Task 10's to create**”
- **P4.T11/Interfaces:L1982**: “### Task 11: Create the section 6 experimental synthetic-root surrogate”
- **P2:L7065**: “`bench oracle` (§13.1) is plan 4 Task 20's”
- **P4.T23/Interfaces:L3729**: “### Task 23: Report measured results, run the oracle suites and enforce the baseline gate”

**Concrete edit / consequence:** Owners exist, but stale cross-references recur throughout handoffs/self-reviews. Apply semantic table E06, including the snapshot registration and benchmark facade corrections. The old review §§4–5 are not execution authorities.

### F23 — MINOR — Redundant glob-covered workspace members

- **P1.T1/Step 1:L103**: “members = ["crates/*"]”
- **P3.T1/Step 1:L129**: “In the root manifest append `crates/core-preflop` to members.”
- **P3.T11/Interfaces:L1627**: “append `crates/core-replay` to members”

**Concrete edit / consequence:** Keep the glob; add only solver-worker and src-tauri when their manifests exist. These are redundant entries, not distinct crate implementations. E03.

### F24 — MINOR — Generated baseline artifacts called Create twice

- **P2.T5/Interfaces:L1103**: “`bench gen-spots --source r8 --out bench/spots`”
- **P4.T20/Interfaces:L3480**: “bench/spots/{river_std,river_min,turn_std,turn_min,flop_fast,flop_min}.json”
- **P2.T30/Step 4:L7002**: “`docs/bench/2026-09-10-i7-13700K.md`”
- **P4.T24/Interfaces:L3890**: “**Files:** Create `docs/bench/2026-09-10-i7-13700K.md`”

**Concrete edit / consequence:** Mark the six suite paths Modify/regenerate and the report Modify/append. Preserve R8-labelled evidence separately. Alongside F05, three duplicate-ownership groups cover eight artifacts. E04.

F22 also includes the wrong registration export: P4:L4058: “`engine::snapshots::register_snapshot`” versus P3.T18/Step 5:L2956: “pub fn register_snapshot(&mut self,active:&DecisionIdentity,snapshot:StreetSnapshot)->bool”. The actual façade is a method on Engine, delegating to the shared core_replay store; no free function is promised by the producer.


## 3. Complete Consumes-to-owner ledger

Each row covers **all items in that task’s Consumes block**, including grouped imports. The consumer’s Files/Interfaces location is linked through its source key and exact line. Producer task references identify the authoritative Produces declaration and its implementation; §2 records exceptions with paired quotes. Same-task helper production (for example P2.T5 prepared_range) is deliberate. Standard-library and pinned upstream dependencies are external inputs, not missing project producers. “Match” is a document contract result, not a claim that unimplemented code passed compilation.

### Plan 2

| Consumer / task start | Consumed family | Producing task(s) | Result |
|---|---|---|---|
| P2.T1, L111 | Vendor API; no workspace input | P1.T1 (root/exclusion); pinned upstream in P2.T1 | F16 |
| P2.T2, L207 | DecisionIdentity, Street, menus | P1.T3/T5/T6; all core manifests P1.T9/T19/T21/T23 | F15 |
| P2.T3, L535 | Templates and materialization inputs | P2.T2; proto P1.T3/T5/T6 | Match |
| P2.T4, L861 | materialize/Templates/tree DTOs/validate_solution | P2.T2–3; P1.T3/T6/T8 | Match |
| P2.T5, L1094 | materialize_at/Templates/prepared_range and proto | P2.T4; prepared_range is produced within this task; P1.T2–4 | Match |
| P2.T6, L1385 | bench materialize and wire schema | P2.T5; P1.T7–8; tools P1.T15 | F19/F20 |
| P2.T7, L1692 | wire constants/cards and vendor card/range API | P1.T2/T4/T7; P2.T1 | Match; F16 build rule |
| P2.T8, L2015 | cards, fixtures, vendor tree API and proto menus | P2.T1/T6/T7; P1.T3/T6 | F19/F20 |
| P2.T9, L2299 | WorkerError and vendor solve iteration API | P1.T7; P2.T1/T7 | Match |
| P2.T10, L2452 | cards/history/tree_build/testutil::fixture_lines, vendor game, wire solution/limits | P2.T7–8 (fixture_lines body L2109); P1.T7–8 | Match |
| P2.T11, L2683 | cards/tree/win/memory/loop/extract/locks, vendor finalize, wire validator | P2.T7–10; P1.T7–8 | Match |
| P2.T12, L2841 | job and ack/message/line limit types | P2.T11; P1.T7 | Match; T12 explicitly copies scaffolding from T13, not a reverse dependency |
| P2.T13, L2984 | protocol scaffolding/job/fixtures | P2.T12/T11/T6 | Match except F20 |
| P2.T14, L3300 | protocol/locks::validate/fixtures | P2.T13/T10/T6 | Match |
| P2.T15, L3494 | Worker/fixture_lines/tree_build/cards/extract/vendor API | P2.T7/T8/T10/T14; fixtures T6 | Match |
| P2.T16, L3718 | Worker/history/tree build, basic fixture and materialization cases | P2.T7/T8/T15/T6 | F19/F20 |
| P2.T17, L3882 | Worker/basic/cancel/best_so_far fixtures/validate_solution | P2.T7/T15/T6; P1.T8 | Match |
| P2.T18, L3973 | wire/Ready/constants; exact equity oracle API | P1.T7/T24–25; P2.T15 real worker contracts | Match |
| P2.T19, L4354 | Clock/WorkerLink/IdentityState/EventSink/wire | P2.T2/T18; P1.T7 | Match |
| P2.T20, L4525 | Clock/EventSink/FakeClock/RecordingSink/recommendation DTOs | P2.T2/T18/T19; P1.T3/T5 | Match |
| P2.T21, L4753 | decision, hand and log input DTOs | P1.T2/T3/T5 | Match |
| P2.T22, L4855 | engine T2/T4/T18–21; hash_scaled and validate_solution | P2.T2/T4/T18–21; P1.T20/T8 | Match |
| P2.T23, L5153 | run_solve/deadline admission and budgets | P2.T22/T20 | Match |
| P2.T24, L5348 | model state/root/RootError/BeginHand and coverage DTOs | P1.T9/T13/T14; P1.T3/T5 | Match: all five RootError variants handled |
| P2.T25, L5527 | equity/estimate types, exact_cost, hero_conditioned | P1.T24–25/T20/T5; P1.T2–4 | Match; P4 must preserve tuple result (F05) |
| P2.T26, L5697 | proto and NodeStrategy | P1.T2–7; body also P2.T20 assumptions_stub and T25 pending_summary | Match |
| P2.T27, L5943 | identity/range hashing, GameConfig/strategy/worker DTOs | P2.T2; P1.T20/T3–7 | Match; this is the shared Arc/Mutex producer |
| P2.T28, L6137 | every earlier engine task; derive/street_root/hash_scaled/parse_range | P2.T2–4/T18–27; P1.T13/T14/T19/T20 | Match; visibility change F07 |
| P2.T29, L6406 | every earlier engine task; model mutations and CoreBeginHand | P2.T28 and its closure; P1.T13 | Match; authoritative startup/Paths/admission contracts |
| P2.T30, L6806 | ProcessWorker/WorkerLink/tree/deadlines/prepared_range/Spot/Suite | P2.T18/T4/T20/T5; real worker T15–17 | Match; F16 selected toolchain |

### Plan 3

| Consumer / task start | Consumed family | Producing task(s) | Result |
|---|---|---|---|
| P3.T1, L121 | proto positions/ranges/hand/actions/reasons | P1.T2–5; manifests P1.T9/T19 | F23 metadata only |
| P3.T2, L328 | decode/BundleInfo/envelopes, bounded IO, sha2 | P3.T1; std; P1.T1 | Match |
| P3.T3, L557 | envelope/manifest and Python standard library | P3.T1–2; P1.T15 | Match; stale task citation F22 |
| P3.T4, L706 | chart_ingest fetch | P3.T3 | Match; acquisition is an implementation deliverable |
| P3.T5, L797 | ingest, transcription/inventory schema, availability manifest | P3.T3–4 | Match |
| P3.T6, L869 | same inputs as T5 | P3.T3–4 | Match; 200bb outcome explicitly conditional |
| P3.T7, L938 | load_bundle, acquired bundles/availability, ingest verify | P3.T2/T4–6 | Match |
| P3.T8, L989 | derive/source maps/keys/TakenAction history | P1.T13/T3; P3.T2 | Match |
| P3.T9, L1341 | PreflopNode/BundleInfo/PreflopAnswer/expand_169 | P3.T1/T2/T8; P1.T19 | Match |
| P3.T10, L1468 | Action/LegalAction/Derived/ExpandedNode | P1.T3; P3.T9 | Match |
| P3.T11, L1625 | Seat/Action/Range1326/interpolation | P1.T3–4; P3.T10 | Match; shared branch types re-exported |
| P3.T12, L1808 | history branches/shared masses | P3.T11 | Match |
| P3.T13, L1920 | query/prefix_state/ExpandedNode/branch kernel/model mutations | P3.T8/T9/T11–12; P1.T13 | Match |
| P3.T14, L2173 | snapshot records/hash_scaled/resolve_chip_path; existing engine store | P3.T13; P1.T20/T6; P2.T27–29 | Match: explicit atomic replacement, not duplicate |
| P3.T15, L2386 | select_snapshot/NodeStrategy/covered_paths/branches/interpolation | P3.T14/T13/T11/T10; P1.T7 | Match; street_history/root_board are implemented early in T14 |
| P3.T16, L2584 | ExpandedNode/posterior/mapped actions/ActionAdvice/Unavailable | P3.T9/T11/T10; P1.T5 | Match |
| P3.T17, L2697 | Engine/Paths/serve/classifier/assemble/core/replay/store/mix | P2.T29/T28/T24/T26/T22; P3.T2/T8/T13/T15/T16 | F01/F02/F07 |
| P3.T18, L2869 | model root/replay_root/RootError, replay, RangeSource/RootRanges, tree/validator/path/events | P1.T14/T8/T6/T5; P3.T13–15; P2.T27/T4/T28 | F06; trait name, return and error otherwise exact |
| P3.T19, L2975 | all P3 APIs and recommendation/identity harness | P3.T1–18; P1.T5 serde; P2.T2/T19 | Match; wrong serde owner citation F22 |

### Plan 4

| Consumer / task start | Consumed family | Producing task(s) | Result |
|---|---|---|---|
| P4.T1, L143 | Card/Street/hash_scaled/canonical board | P1.T2/T3/T20/T21 | F15 manifest |
| P4.T2, L248 | wire solution/node/tree/materialized/path/versions | P1.T6–8; P4.T1 error/key scaffold | Match |
| P4.T3, L492 | CacheEntry/MaterializedNode | P4.T2; P1.T6 | Match |
| P4.T4, L573 | ApproxReason/Coverage/Comparison | P1.T5; P4.T3 | Match |
| P4.T5, L646 | CacheEntry/validate_entry | P4.T2 | Match; binary DTO preserves tagged JSON metadata |
| P4.T6, L792 | CacheEntry/encode/read_cell | P4.T2/T5 | Match |
| P4.T7, L987 | entry/compare/label/read_cell/validate_solution/Derived/Paths.cache/tree_signature | P4.T2–6; P1.T8/T3; P2.T29/T4; engine replay integration P3.T18 | F01/F09/F10/F11/F13 |
| P4.T8, L1297 | effective tree/cache/key/query/Templates | P2.T4/T2; P4.T2/T6/T7 | F04 |
| P4.T9, L1457 | SolverPrefs/TakenAction/Deadlines and public config API | P1.T3; P2.T20/T29; fake support from P4.T8 | F08/F11/F13 |
| P4.T10, L1605 | cache lookup/store/bridge/serve/solve/watchdog/assembly/tree/snapshots/SolveInput | P4.T6–9; P2.T28/T22–23/T20/T26/T4; P3.T18; P1.T6 | F08/F11/F13 |
| P4.T11, L1982 | experimental proto/equity/deadlines/solve/tree/advice rows | P1.T5/T24–25; P2.T25/T20/T22–23/T4/T26 | F05/F11/F12; advice_rows absent |
| P4.T12, L2180 | snapshot/replay types, snapshot_from_hit, Engine and store registration | P3.T13–15/T18; P4.T10 | Match; F22 obsolete path/number |
| P4.T13, L2266 | Position/Card/canonicalize/orbit_size | P1.T3/T2/T21 | Match |
| P4.T14, L2376 | Scenario/Rational/KeyFields/write_atomic | P4.T13/T1/T6 | Match |
| P4.T15, L2630 | Queue/QueueItem/Scenario/CacheEntry/SolveInput | P4.T14/T13/T2; P1.T6 | F10 forward helper; no engine dependency |
| P4.T16, L2860 | PreflopStore/query/replay/tree/street_root/config/entry_from_solution/store receipt | P3.T2/T8/T13–18; P2.T4; P1.T14/T3; P4.T10/T6/T15 | F09/F10 |
| P4.T17, L2991 | inspected chart pairs and availability manifest | P3.T4–7 | Match: propagates unavailable depth |
| P4.T18, L3121 | PokerKit adapter and frozen sources | P1.T16; P4.T17 | Match |
| P4.T19, L3330 | record schema/generator, legality and source lock (implicit inputs) | P4.T18/T17; P1.T16 | Match; explicit Produces assigns loaders here |
| P4.T20, L3476 | scenario/replay/SolveInput/source lock and generator facade | P4.T16/T17; P3.T13–18; P1.T6; facade implementations assigned in this task | F14/F24 |
| P4.T21, L3576 | RecordedHand/load_records/Engine/WorkerLink | P4.T19; P2.T29/T18 plus P3.T18/P4.T10–16 integrations | Match; RunOptions/DecisionRun are explicitly owned here |
| P4.T22, L3645 | WorkerLink/FakeClock/FakeWorker/FakeReply/parser/errors/cache IO seam | P2.T18/T19; P4.T6–7/T21 | Match; no second transport trait |
| P4.T23, L3729 | report/SpotResult and T20–22 records | P2.T30; P4.T20–22; oracle tests P1.T21/T23–25 and P2.T15/T18/T25 | F17/F21 |
| P4.T24, L3886 | every preceding P4 task | P4.T1–23 transitively | F16/F24; owns measured V3 evidence |
| P4.T25, L3952 | runners T20–23 and river/turn suites | P4.T20–24; P2.T5/T30 (replaced chart suites from P4.T20) | Match |
| P4.T26, L3997 | complete measurement report and bench gate | P4.T24–25/T23; additionally P5.T14 required by S§14 V22 | F21 |

### Plan 5

| Consumer / task start | Consumed family | Producing task(s) | Result |
|---|---|---|---|
| P5.T1, L155 | every proto type; engine dependency manifest | P1.T2–8; P2.T2 (engine crate exists) | Match; optional ts-rs, F15 root pin |
| P5.T2, L463 | proto; typed dispatcher independent of actual Engine | P1.T2–5; P5.T1 scaffold | Match |
| P5.T3, L645 | Service/Op/EnginePort/AppError/Channel | P5.T2/T1 pinned Tauri | Match |
| P5.T4, L840 | Engine/EventSink/Paths/BeginHand/status methods | P2.T29/T18; P1.T3; P4.T16/T15 | F01/F03/F10 |
| P5.T5, L1008 | new/shutdown/Service and chart files | P2.T29; P5.T2/T4; P3.T4–7 | F01/F02/F16/F18 |
| P5.T6, L1368 | generated config/hand/actions/cards/identity/events | P5.T1 exporter; originals P1.T2–5 | Match |
| P5.T7, L1570 | Backend recommend/cancel and event DTOs | P5.T6; P1.T5 via P5.T1 | Match |
| P5.T8, L1798 | GameConfig/Seat/Backend.set_game_config | P1.T3 via P5.T1; P5.T6 | Match |
| P5.T9, L1972 | hand/derived types and stack drafts | P1.T3 via P5.T1; P5.T8 | Match |
| P5.T10, L2123 | Backend/Recommendations/Wizard/Derived.legal | P5.T6/T7/T9; P1.T3 | Match |
| P5.T11, L2423 | Recommendation/Assumptions/EquitySummary/ExperimentalHu/DisplayState | P1.T5 via P5.T1; P5.T7 | Match; UI only formats |
| P5.T12, L2649 | all components/controller/recommendations/backend/bootstrap | P5.T5–11 | Match |
| P5.T13, L2876 | DOM keydown, pinned WebviewWindowBuilder and accelerator constraints | P5.T1/T10/T12 | Match |
| P5.T14, L2983 | real MSVC app, selected worker, charts and complete engine/flop policy | P5.T5/T12/T13; P2 worker; P3.T7/T18; P4.T10–25 | F16/F21: worker may be GNU; record UI evidence before final V22 |


## 4. Explicit disposition of the requested residuals

### 4.1 Plan 4’s seven notes owed to Plan 2 (P4:L79–87)

| Note | P2 revised producer and evidence | Disposition |
|---|---|---|
| 1: test-template seam | P2.T2 Interfaces:L215, Step 6:L481–512: `Templates::with_extra(&[TemplateSpec])`, `base_ids()`, feature `test-templates` | **Satisfied by P2.** P4.T8’s second registry/get patch is stale; F04. No P2 registry rewrite. |
| 2: fallible set_config | P2.T29:L6425, L6689–6700: `Result<u32, EngineError>`, validation and next-hand queuing | **Satisfied, including rejection body.** P4 must not replace it; F08. |
| 3: four Paths fields | P2.T29:L6418: `log_dir, worker_exe, preflop, cache` | **Four fields satisfied; requested spelling is not.** User chose P2; fix P4, F01. |
| 4: bench_support.rs | P2.T5 Files:L1097 and Interfaces:L1103: `prepared_range`, `range_mass` | **File satisfied.** Producer is T5, not T22. P4:L69’s claimed re-exports of parse_range/block_public/hash_scaled are not the actual contract; replace the claim, F22. |
| 5: bench library target | P2.T5:L1103, module list L1211–1214; T30 extends it at L6972 | **Satisfied.** Consumers link `bench::{suite,gen_spots,materialize,runner,report}`. |
| 6: background parameter | P2 global:L20: “This plan exercises only `false`”; T22:L4864/L5012–5023 owns `SolvePlan.background` | **Satisfied as a field/wire contract.** Single-owner presolver execution still needs F09; do not confuse parameter availability with a working background scheduler. |
| 7: mutable idempotent shutdown | P2.T29:L6436/L6444 and implementation L6775–6785 | **Satisfied.** P4 extends cleanup, without changing receiver or adding another engine owner. |

**Missing P2 declarations among these seven: none.** P4 is consuming obsolete shapes/numbers in notes 1–4; notes 2 and 7 are already implemented by revised T29. The new private-helper visibility fix is separately assigned to P2.T28 (F07), not one of these seven.

### 4.2 Plan 5’s eight “Required from Plan 2” entries (P5:L54–80)

| Entry | Actual producer / exact shape | Result and task to amend |
|---|---|---|
| new | P2.T29:L6422–6424: `Result<Engine, EngineError>` plus `startup_report(&self) -> StartupReport` | **P5 wrong**, user decision; E05. P2.T29 unchanged. |
| set_config | P2.T29:L6425: `(&mut self, GameConfig) -> Result<u32, EngineError>` | Satisfied. Preserve queuing and validation. |
| shutdown | P2.T29:L6436: `(&mut self)`, idempotent | Satisfied. |
| set_hero_cards | P2.T29:L6427: `(&mut self, [Card; 2]) -> Result<HandState, EngineError>` | Satisfied, including mutation revision. |
| presolver_status | **Not in P2.** P4.T16:L2864/L2952: `(&self) -> cache::presolver::scheduler::PresolverStatus` | Move to “Required from P4.T16”; add parent re-export in P4.T15. Amend **P2.T29 Interfaces** with an explicit future-owner note, not a premature cache dependency. |
| presolver_pause | **Not in P2.** P4.T16:L2956: `(&mut self)` | Same ownership correction; P2.T29 note / P4.T16 implementation. |
| presolver_resume | **Not in P2.** P4.T16:L2959: `(&mut self)` | Same ownership correction. |
| Paths | P2.T29:L6418: `{log_dir,worker_exe,preflop,cache}` | **P5 wrong**, user decision; E05. |

Thus **3/8 match P2 exactly, 2/8 require consumer corrections, and 3/8 are produced later by P4, not P2**. None warrants fabricating a second façade or adding cache to P2 before its crate exists. P5.T4, rather than the mock-only T2, is the actual real-engine compilation gate. The additionally advertised unchanged `begin_hand` is wrong (F03); recommend’s `Box<dyn EventSink>`, cancellation, action/board mutation and finish/abandon otherwise match P2.T29.

### 4.3 Plan 3’s four named seams

| Seam | Exact ownership / evidence | Result |
|---|---|---|
| engine::snapshots re-export | P2.T27:L5951/L6034–6060 creates temporary `SolvedStreet` and `SnapshotStore`. P3.T14:L2181 and Step 5:L2377 **deletes both** and installs `pub use core_replay::{SnapshotKey, SnapshotProvenance, SnapshotStore, StreetSnapshot};` | Correct staged replacement. It is not a P2 re-export before core-replay exists and not simultaneous duplicate storage. Retain for_hand compatibility for P2 tests. |
| engine::ranges::RangeSource::ranges_at_root | P2.T27:L5952/L6065–6070: `(&self,&HandState,&StreetRootSnapshot) -> Result<RootRanges,UnsupportedReason>` | Exact name/signature/error exist. P3’s shared field/init shape is wrong, F06. |
| serve_request Classification::Preflop arm | P2.T28:L6289 exact placeholder; P3.T17:L2751–2757 quotes/replaces that arm | Correct hook. Shared emit needs pub(crate), F07. |
| proto::resolve_chip_path | P1.T6:L997/L1136: `(&[MaterializedNode], &[Action]) -> Option<OrdinalPath>`; P2.T4:L984 re-export; P3.T14 consumes original | Single implementation; exact result type, no error enum. No core_replay::resolve_path implementation. |

### 4.4 Surrogate and bench oracle are owned exactly once

- **Surrogate: P4.T11**, Files:L1984, Interfaces:L1988, Steps 1–4. P2.T28 only emits main `Unsupported{MultiwayEv}`; its self-review:L7062 explicitly defers creation (wrongly numbered T10). P1.T5 supplies only the `ExperimentalHu` DTO; P5.T11 only renders it. No competing surrogate implementation exists. F05 is a duplicated equity helper inside the unique surrogate task; F12 is that task’s broken contract.
- **bench oracle: P4.T23/Step 3a:L3809–3847** creates `crates/bench/src/oracle.rs` and the CLI. P1.T22 supplies fixture generation/exhaustive test inputs, not this subcommand. P2.T5/T30 implements materialize/gen-spots/run, not oracle. P1:L5370 and P2:L7065’s “Task 20” references must become T23. F17 fixes the command’s consumers, without creating another oracle runner.

## 5. Ownership and workspace consistency

### 5.1 Duplicate and orphan inventory

Duplicate counts use **ownership groups**, so a six-file brace group is one finding, not six independent root causes.

| Group | First owner | Conflicting creation / required edit |
|---|---|---|
| D1: one public function | P2.T25 engine::equity::range_vs_range | P4.T11 recreates it with Option<f32>; delete the second body (F05). |
| D2: six generated suite paths | P2.T5 generator writes river_std, river_min, turn_std, turn_min, flop_fast, flop_min under bench/spots | P4.T20 Files:L3480 calls them Create. Change to Modify/regenerate; this is intended replacement of R8 data, not coexistence (F24). |
| D3: one report path | P2.T30 Step 4:L7002 writes docs/bench/2026-09-10-i7-13700K.md | P4.T24 Files:L3890 calls it Create. Modify/append provenance-separated measurements (F24). |

**3 groups / 8 artifacts.** P4’s test-template registry is a conflicting extension mechanism (F04), not another identically named public function counted again. Explicit modifications to Engine::set_config, core.rs, serve.rs, bench main/report and proto TS derives are not accidental file recreation. P3’s snapshot/branch re-exports are not duplicates. The two BeginHand types intentionally live in different crates with different ownership.

| Orphan advertised name/key/target | Consumer | Resolution |
|---|---|---|
| workspace.dependencies.hex | P2.T2 | Add root key, F15 |
| workspace.dependencies.ts-rs | P4.T1 | Add root key, F15 |
| Templates::all | P4.T8 registry patch | Remove obsolete patch; use with_extra/base_ids, F04 |
| core_iso::invert | P4.T7 | inverse, F11 |
| core_iso::SuitPerm::identity | P4.T10 | IDENTITY constant, F11 |
| engine::assemble::advice_rows | P4.T11 | Explicit surrogate-only extractor, F12 |
| EngineCore.bench_p95_ms | P4.T10 | Validated policy field/loader, F13 |
| cache::presolver::PresolverStatus | P5 required façade | Re-export scheduler::PresolverStatus, F10 |
| engine::snapshots::register_snapshot | P4 resolved table/self-review | Engine::register_snapshot / SnapshotStore::register, F22 |
| engine::bench_support::Spot (unqualified Spot in that façade) | P4.T20 | Engine-owned PreparedBenchSpot DTO, F14 |
| solver-worker integration target river_analytic | P4.T23 | contract_river, F17 |
| solver-worker integration target ev_contracts | P4.T23 | contract_river plus engine check-only oracle, F17 |

**12 orphan advertised names/keys/targets.** Some have an obvious differently named producer; they still do not exist at the advertised path. V1’s unassigned *measurement* is counted as F16, not a thirteenth missing public symbol. remaining_seconds and the three presolver methods have owners, so they are ordering/ownership-location mismatches, not orphans.

### 5.2 Workspace, dependencies, toolchain and versions

| Contract | Producer / consumers / spec | Result |
|---|---|---|
| Crate names and directories | P1 creates proto, core-model, core-ranges, core-iso, core-eval in crates/; P2 creates engine/bench there and solver-worker/ at root; P3 core-preflop/core-replay; P4 cache; P5 apps/pokerai-ui/src-tauri package pokerai-app, library pokerai_app. S§3.2:L80–98 | Names agree. Hyphenated packages correctly import with underscores. P5.T4’s src-tauri/src/Cargo.toml path is erroneous and disappears with F03’s unnecessary dependency edit. |
| Workspace members | P1.T1:L103 `["crates/*"]`; P2.T1 adds only vendor exclusion (L153–162); P2.T7 adds solver-worker; P5.T1:L161 adds src-tauri | Canonical final set: `["crates/*", "solver-worker", "apps/pokerai-ui/src-tauri"]`; exclude `["third_party/postflop-solver"]`. P3 redundant entries F23. P1 self-review’s P2.T1 worker-member claim is stale; actual owner T7. |
| Dependency direction | S§3.2 and P2 globals:L19–21; P4:L18/54; P5:L18 | Worker links only proto + vendored AGPL project; no other crate depends on worker. Bench sees engine/proto. P5’s core-model workaround and P4 engine-to-bench Spot violate the intended stated dependency surface; F03/F14. Cache’s PresolveExecutor callback is downward dependency injection, not a cache→engine dependency. |
| Shared pins | P1.T1:L112–116: `serde = { version = "1.0", features = ["derive"] }`, `serde_json = "1.0"`, `thiserror = "2.0"`, `sha2 = "0.10.9"`; P2.T2:L234–238; P3.T1:L141–144; P4.T1:L178–181; P5.T1:L190–192 | Requested versions agree; they are Cargo version requirements, not exact equals pins. Missing inherited hex/ts-rs keys F15. P2’s “all from workspace.dependencies” prose is too broad for its worker-local rayon="1"; clarify without adding another dependency owner. |
| Evaluator | P1.T1:L116 pins holdem-hand-evaluator rev d7b2a5bba4015f96855d5cd66d21f781d3cbcf9b; P1.T23 backend; S§3.2/§3.5 revised weighted equity | Agrees. No pokers runtime dependency; no additional evaluator contingency implemented. |
| Solver + bincode | P2.T1:L135–148: vendor commit 9d1509fe5077d019825f833eed04b16d342dfda1; both bincode and bincode_derive =2.0.0-rc.3, optional; default bincode/rayon plus zstd, no custom-alloc | Agrees with P4:L18 and P5:L33/spec. Cache’s independent =1.3.3 and zstd =0.13.3 (P4.T1) are intentional; do not unify bincode majors across the process/storage boundary. |
| Toolchain | P1.T1:L131–135 `stable-x86_64-pc-windows-msvc`; edition 2021, rust-version 1.95; S§3.6 | One repo toolchain file, no competing pin. “stable” is a moving channel with a minimum-version requirement, not an exact compiler release lock; record rustc -Vv in measurements. V1/fallback execution contradicts this intended rule, F16. |
| AVX2 | P1.T1:L140–145 flags for GNU and MSVC; P2.T7 build.rs and ready checks; P4:L18; P5:L33 | Same `rustflags = ["-C", "target-feature=+avx2"]`. No duplicate config. Apply to the selected worker target. |
| proto_version | P1.T1:L178 `proto::PROTO_VERSION: u16 = 3`; P1.T7:L1301 `pub use crate::PROTO_VERSION`; P2/P4 import it | Exactly one definition, re-export not duplication. |
| rules_version | P1.T6:L1063 `RULES_VERSION: u16 = 3`; P2.T4:L1030 re-export; P4 keys/validation use 3 | Value agrees with S§4.6. Literal validation against 3 does not create another public constant. |
| solver/adapter identity | P1.T7:L1302–1303 owns SOLVER_COMMIT and ADAPTER_VERSION=1; P2 worker re-exports SOLVER_COMMIT; P4 imports | Agrees; no source duplication. |
| Cache schema | P4.T1/T2 key and validator: schema_version=3, path cache/v3; S§10.4 | Agrees. QueueFile version 1 (P4:L2453), chart acquisition schema 1 (P3:L717), and recorded hand/input version 1 are separate schemas, not conflicts. No unowned cache SCHEMA_VERSION constant is advertised. |
| Startup and paths | P2.T29 + P3.T17 + P4.T7/T16; P5.T5 | Required corrections F01–F03/F06/F08–F10/F18; unmodified P2 public signatures remain the base. |

### 5.3 Self-review deviations checked against revision 6

- **P1 self-review:L5407–5424:** StreetRootSnapshot.bb_chips, five RootError variants, evaluator interface, MenuSize/SideMenu, the two BeginHand types, normalized PokerKit fixtures, per-seat indexing and exhaustive 10M gate agree with revision 6. The toolchain executor and oracle task-number claims still need F16/F22.
- **P2 self-review:L7051–7066:** QQ/66 payoff correction, in-process conservation, engine-owned check-only oracle, root-street None and temporary snapshot replacement are supported by revised S§13.2/§4.6 and P3.T14. The 43-case exception is **not** authorized by S§13.2 (F19). The later-street None choice explicitly admits an unresolved spec contradiction (F20). Preflop/flop/surrogate/bench data deferrals have later owners, with corrected numbering. Off-legal advice remains documented as temporary; P3.T10 owns source translation, while valid live trees/current-street insertion and P4 cache legal-menu validation must keep that fallback unreachable for supported output; do not label it a completed P4 translation implementation.
- **P3 deviations:L3155–3170:** Extended BundleInfo metadata, generated marked-synthetic fixtures, CompatKey without tree_signature, headline notes and identity-filtered snapshot slices have explicit owners and do not redefine proto. Timing is required to be measured, not asserted in CI. Conditional acquisition is correctly propagated in P4, but P5 staging breaks it (F18).
- **P4 self-review:L4033–4062:** Claimed single snapshot store, shared deadlines, typed callback, serializable status, independent cache bincode and diagnostic-only mode are consistent intentions. Claims of exact type consistency are disproved by F04–F14; header’s parent status path and free registration function are not supplied. Release completeness is overstated without P5.T14 (F21).
- **P5 self-review:L3125–3153:** Build-time proto generation and real chart E2E ownership are clear. Its “resolved façade” claims retain tuple startup and shell-side CoreBeginHand conversion after P2 changed them (F02/F03). Status is manually projected to JSON and accepted as unknown in TypeScript; P4’s optional TS derive is not a justification for a missing workspace dependency.


## 6. Minimum coordinated edit set, grouped by plan

These are **exact replacement/addition texts for the orchestrator to apply**. No source document was edited in this review. Apply each change to the task body **and its matching Files/Interfaces/self-review claim**; otherwise the same contradictory contract remains in two places. E06 supplies exact corrected cross-reference text, not a blind numeric substitution.

### E01 — Plan 1

**T1 / Step 1, [workspace.dependencies] (F15).** Add:

```toml
hex = "0.4"
ts-rs = "=12.0.1"
```

Keep serde="1.0", serde_json="1.0", thiserror="2.0", sha2="0.10.9" and the evaluator rev unchanged. A root optional-use version declaration does not enable ts-rs in proto or the worker.

**T1 / Step 3 (F16), replace the sentence at L218** with:

> Do not execute the GNU worker build in this task: solver-worker is created in Plan 2 Task 7. Plan 2 Task 1 owns the V1 MSVC FLOP-FAST measurement and records the worker toolchain selection; this task only creates the fallback script. Every subsequent worker build, worker test and runtime staging step consumes that selection, while engine, bench and pokerai-app remain MSVC.

Correct the oracle owner and worker membership self-review references using E06.

### E02 — Plan 2

**T1 / Step 5 and globals Toolchain (F16), replace the circular V1 paragraph** with:

> This task performs the V1 toolchain check; no earlier task has performed it. Build the pinned solver with AVX2 on MSVC, then run the same FLOP-FAST workload and measurement convention as R8 §5. Record rustc -Vv, target, solver commit, flags, workload, the measured MSVC solve time and the R8 GNU comparator in PATCHES.md. vendor_smoke is a correctness smoke test and is not the performance comparator. If the MSVC build fails or its comparable solve time is more than 1.25 times the GNU value, select GNU for solver-worker only. Otherwise select MSVC. Also create docs/bench/worker-toolchain.json with worker_toolchain and worker_target strings and the measurement provenance. The allowed pairs are stable-x86_64-pc-windows-msvc / x86_64-pc-windows-msvc and stable-x86_64-pc-windows-gnu / x86_64-pc-windows-gnu. Missing selection evidence blocks later worker timing; it never silently defaults to MSVC.

Add that JSON file to T1 Files. If R8’s exact comparator cannot be reproduced or identified, report the missing measurement fact; do not substitute tree-construction wall time.

**T7 / Step 5 and the series-wide green/build rule (F16), replace the parenthetical claiming the MSVC workspace “skips nothing”** with:

> On the MSVC branch run cargo test --workspace --release. On the GNU branch run cargo test --workspace --exclude solver-worker --release under the pinned MSVC toolchain, then cargo +stable-x86_64-pc-windows-gnu test -p solver-worker --release --target x86_64-pc-windows-gnu. Both commands must pass; this partitions the workspace by selected compiler and skips no package. Build/stage the GNU worker through scripts/build-worker-gnu.ps1 and set POKERAI_WORKER to that selected binary for process tests. No ordinary workspace command may rebuild the rejected MSVC worker. Apply this rule to all later plan test gates, bench oracle, diagnostic workers and staging.

P2.T30 Step 4 must use the selected build before its bench run. Change Tech Stack’s “all from [workspace.dependencies]” to:

> serde, serde_json, sha2, hex and thiserror inherit workspace versions. rayon = "1" is a worker-local dependency.

**T6 / Step 3, generator (F19).** Replace L1627–1629 with:

```python
# All seven section-10.1 templates receive the section-13.2 five-point sweep.
TEMPLATES = ["flop_fast_v1", "flop_min_v1", "flop_full_v1",
             "turn_std_v1", "turn_min_v1", "river_std_v1", "river_min_v1"]
```

Delete `cases.append(("flop_full_v1_100_100", "flop_full_v1", 100, 100, []))`. Retain the other twelve cases and the all-node cross-street comparisons. Replace **43 cases → 47 cases** in T6 Interfaces, T8 expected output, T16 Files/test comment, and self-review items 4/14. Replace deviation 14 with:

> All seven templates, including flop_full_v1, receive the five-point materialization sweep. Phase-2 solve selection does not exempt a template from section 13.2’s rules tests.

**T28 / Step 3, emit declaration (F07).** Replace `fn emit(` with `pub(crate) fn emit(`; preserve its complete signature/body. Add to Interfaces:

```rust
pub(crate) fn emit(
    core: &EngineCore,
    req: &LiveRequest,
    delivered: Option<&AtomicBool>,
    ev: RecommendationEvent,
);
```

This is a crate-visible engine helper, not public IPC.

**T29 / Interfaces, after the produced façade (F10).** Add:

> Plan 4 Task 15 owns cache::presolver::PresolverStatus. Plan 4 Task 16 adds Engine::presolver_status(&self) -> cache::presolver::PresolverStatus, Engine::presolver_pause(&mut self), and Engine::presolver_resume(&mut self). They are not produced by this task; Plan 5’s real adapter must depend on Plan 4 Task 16. Do not create placeholder methods or add cache before its crate exists.

**T8 contract note / T13 precheck / T16 wire assertions (F20).** They remain blocked on the spec decision. Recommended exact **spec §13.2 tree_materialization_matches_library, wire clause replacement**:

> A deliberately altered materialized entry or a missing terminal marker produces result{error{tree_mismatch}}. A missing or None donk option on a street strictly after the root is structurally invalid and produces ack{rejected, reason}, with no work; a root-street None is legal. tree_config independently rejects a later-street None for direct callers.

Then replace P2.T8’s “This plan picks” deviation with:

> The revised section 13.2 requires precheck to reject a later-street None donk before work. The root street may use None. Direct tree_config validation retains the same structural check; wire tree_mismatch tests use accepted requests with altered materialization or terminal markers.

This decision would require a separately authorized spec revision/changelog; it is **not made by this read-only report**. If the authority instead chooses result{error{tree_mismatch}}, amend both spec rows and P2.T13’s precheck/ack-result assertions consistently.

### E03 — Plan 3

**Header L109, T17 Interfaces/Step 3:L2762 (F01).** Replace every old Paths shape with:

```rust
Paths { log_dir: PathBuf, worker_exe: PathBuf, preflop: PathBuf, cache: PathBuf }
```

**T1 / Step 1 and T11 / Files + Step 1 (F23).** Replace instructions to append either core crate to members with:

> The existing crates/* workspace glob includes this crate when its Cargo.toml is created. Preserve that glob; do not append an explicit crates/core-preflop or crates/core-replay member. Inherit version, edition and license from workspace.package.

**T17 / Step 3 startup paragraph (F02/F06/F07).** Add:

> Load the PreflopStore before handing EngineCore to Engine::with_core. Keep its Arc on EngineCore and clone it into Engine for preflop_store(). After construction, append every loader warning to engine.startup.banners; startup_report() is the single UI diagnostics surface and Engine::new still returns Result<Engine,EngineError>. Import crate::serve::emit in preflop.rs; do not create a second event-emission helper.

**Header L114 and T18 / Interfaces + Step 3:L2877/L2923 (F06).** Replace the Box-only initialization and direct-call prose with:

```rust
// EngineCore and Engine keep the SAME shared source handle.
pub range_source: Arc<Mutex<Box<dyn RangeSource>>>;

// Install after core.preflop has been loaded, before Engine::with_core starts engine-main.
*core.range_source.lock().unwrap() = Box::new(ReplayRanges {
    store: core.preflop.clone(),
    snapshots: core.snapshots.clone(),
    identity: core.identity.clone(),
});

// serve_request's existing call remains locked:
core.range_source.lock().unwrap().ranges_at_root(&req.state, &root)
```

Replace the advertised ReplayRanges record with:

```rust
pub struct ReplayRanges {
    pub store: Arc<PreflopStore>,
    pub snapshots: Arc<Mutex<core_replay::SnapshotStore>>,
    pub identity: Arc<Mutex<IdentityState>>,
}
```

Keep EngineCore::new’s four-argument signature and an empty preflop/default ExplicitRanges for unit-test construction; production Engine::new installs loaded replay ranges. Engine::set_explicit_ranges updates the boxed value through the same shared mutex. Do not change the trait signature/error type or P3.T14’s atomic snapshot re-export.

### E04 — Plan 4

**Resolved upstream table/notes and T7 Paths consumption (F01/F22).** Replace the Paths shape with P2’s four fields from E03 and its owner with **Plan 2 Task 29**. Replace “this plan supplies the validation body” with “Plan 2 Task 29 already supplies validation and next-hand queuing; this plan adds tests.” Correct other owners via E06.

**T7 / Step 5 and T9 / Step 3; T10 / Step 4; T11 / Step 3 (F11).** Apply these exact substitutions at every occurrence:

| Old | Replacement |
|---|---|
| `core_iso::invert(perm)` | `core_iso::inverse(perm)` |
| `core_iso::invert(&SuitPerm) -> SuitPerm` | `core_iso::inverse(&SuitPerm) -> SuitPerm` |
| `core_iso::SuitPerm::identity()` | `core_iso::SuitPerm::IDENTITY` |
| `state.config.straddle.unwrap_or(state.config.bb_chips)` | `state.config.straddle.as_ref().map_or(state.config.bb_chips, \|s\| s.amount_chips)` |
| `core_model::postflop_order(state)` | `core_model::postflop_order(state.button, &state.dealt)` |
| `core_model::postflop_order(&HandState) -> Vec<Seat>` | `core_model::postflop_order(Seat, &[Seat]) -> Vec<Seat>` |

Remove the “if Plan 1 named it differently” hedges; the exact producer is known.

**T8 / Step 1a (F04).** Delete TEST_TEMPLATES, register_test_template, test_extra, and the replacement Templates::get. Keep the build closure inside install_cache_test_templates, gate that helper with `#[cfg(any(test, feature = "test-templates"))]`, and replace its three register calls with:

```rust
Templates::with_extra(&[
    build("check_jam_test_v1", vec![AllIn], vec![], 0.0, 0.0),
    build("check_only_test_v1", vec![], vec![], 0.0, 0.0),
    build("menu_round_test_v1", vec![Pot(0.33)], vec![AllIn], 1.5, 0.15),
]);
```

Enable both testing and test-templates on the engine’s test-only self dependency/test commands that use CacheRig. Replace the assertion with:

```rust
assert_eq!(Templates::base_ids().len(), 9);
for id in ["check_jam_test_v1", "check_only_test_v1", "menu_round_test_v1"] {
    assert!(Templates::get(id).is_some());
    assert!(Templates::ids().contains(&id));
}
```

Update T8 Interfaces to produce only install_cache_test_templates and CacheRig; P2 owns with_extra. Keep production template selection limited to its named production IDs.

**T9 / Step 3 (F08).** Delete the replacement set_config body and `core.set_config(cfg.clone())?` sentence. Replace with:

> Preserve Plan 2 Task 29’s set_config and apply_config bodies, including blind/thread/flop validation, revision allocation, next-hand queuing and the shared-config lock. Add flop_budget_valid only as a wrapper over the existing FLOP_BUDGET_RANGE. Keep the config() getter, returning queued_config.clone().unwrap_or_else(|| self.config.clone()) so settings UI reads the accepted next-hand configuration. Rejected settings change neither revision nor active work. New serving helpers consume the one request configuration captured by serve_request; they do not lock EngineCore from a command.

**T10 / Step 4 and T11 / Step 3 (F08).** Replace all `core.config.solver.target_bp` with the request’s captured `target_bp`. Add `target_bp: u16` to probe_cache, cache_phase, recommendation_from_hit and run_surrogate parameters where used; forward `config.solver.target_bp` from the existing `let config = core.config();` in serve_request (P2:L6282). Do not recapture config midway through a decision.

**T9 / Step 3 + Files/Interfaces, T10 / Step 4 (F13).** Add this contract text:

> Task 9 owns V3PolicyEvidence, load_v3_policy and EngineCore.flop_policy. Evidence contains the exact template signature, solver commit, adapter/rules versions, storage-mode policy, source-lock hash, machine identity and finite positive p95 seconds at 100bb and 200bb. load_v3_policy(path, expected_provenance) returns FlopPolicy::from_v3(Some(p95_100), Some(p95_200)) only when every provenance field matches; otherwise it returns FlopPolicy::from_v3(None,None), with a startup diagnostic. EngineCore::new initializes that conservative value. Engine::new loads evidence off the recommendation path before engine-main starts. Task 24 serializes the matching evidence alongside its raw V3 measurements; Task 9’s tests use synthetic evidence. Missing production evidence does not create a dependency cycle on Task 24.

Replace the nonexistent bench_p95_ms lookup at L1757–1759 with:

```rust
let policy = &core.flop_policy;
```

Use that policy’s live_template result. Add derives/imports needed to store the policy; do not introduce a map of unqualified timings.

**T7 / Step 5a and T15 / Step 3–4 and T16 / Step 4 (F09/F10).** Replace the core-held Presolver and lifecycle listing with this ownership contract:

> Task 7 adds only EngineCore.cache and with_cache, initializing cache to Cache::disabled(); it does not mention Presolver. Task 15 defines remaining_seconds in cache::presolver before scheduler uses it and adds pub use scheduler::PresolverStatus to cache::presolver. Task 16 adds Engine.presolver: Option<Arc<cache::presolver::scheduler::Presolver>>, initialized to None by with_core and populated once in production startup. Status, pause, resume, hand notification and live notification use this independent handle directly and never acquire EngineCore’s mutex. EngineExecutor holds a command/completion endpoint to the existing single worker owner, not an Arc<Mutex<EngineCore>> used by UI control calls. That owner serializes live and background send/receive, confirms background terminal or exit before sending live work, and observes the presolver cancellation flag while background work is running. A background job has its own bookkeeping and never requires an active live DecisionIdentity or emits user recommendation events/snapshots. No second ProcessWorker is spawned. Shutdown signals background cancellation before waiting for engine-main, joins presolver, stops cache threads, and then shuts down/kills the worker; repeated shutdown is a no-op.

Exact façade methods (using the new Engine field):

```rust
pub fn presolver_status(&self) -> cache::presolver::PresolverStatus {
    self.presolver.as_ref().map(|p| p.status()).unwrap_or_default()
}
pub fn presolver_pause(&mut self) {
    if let Some(p) = &self.presolver { p.pause(); }
}
pub fn presolver_resume(&mut self) {
    if let Some(p) = &self.presolver { p.resume(); }
}
fn notify_presolver_hand(&self, in_progress: bool) {
    if let Some(p) = &self.presolver { p.notify_hand(in_progress); }
}
```

Move the existing remaining_seconds body from T16 Step 5 into T15; T16 only adds `pub use cache::presolver::remaining_seconds;` to engine::log. T16’s red tests must include a worker held in an indefinitely running solve while status/pause/live notification return promptly and cancellation precedes live dispatch.

**T11 / Interfaces + Step 3 (F05/F12).** Delete the entire duplicate equity::range_vs_range definition. Consume P2.T25’s existing tuple-returning function; change the choose_opponent binding to:

```rust
let Some((equity, _method)) = crate::equity::range_vs_range(
    range, hero_public, board, per_seat, cancel
) else { continue };
```

Replace the run_surrogate listing and advice_rows prose with this exact contract:

> run_surrogate receives the actual hero cards for advice extraction, the hand’s bb_chips and rake, and the request-captured target_bp, in addition to its existing synthetic input/public ranges/board/deadline/identity/sink. The hero cards never condition those public solve ranges. Build the synthetic tree from pot, equal effective stacks and empty history, but never construct SolveInput. Task 11 factors the existing worker request/response, validation, heartbeat, cancellation and deadline handling below run_solve into a shared internal transport function accepting a worker SolveRequest; run_solve continues to be the main-path SolveInput adapter, and the surrogate constructs its SolveRequest directly. Its output bypasses cache and snapshot registration. Validate the entire returned solution, resolve the root or check child for hero’s role, select the actual hero combo row, and use ev_chips / bb_chips once to construct ActionAdvice. Preserve fold EV zero and unavailable/out-of-support handling; do not call advice_rows or pass an empty reach vector to final_from_solution. Return only ExperimentalHu, with EXPERIMENTAL_NOTE; the main recommendation remains Unsupported{MultiwayEv} with actions empty. Use the real rake, never a hard-coded TimeCharge, and the real BB, never 2. Skip the surrogate on all-in opponent, zero stack, exhausted equity/remaining budget or worker failure. Add a non-2-BB, IP-after-check, actual-combo regression alongside the existing isolation golden.

P4 owns this task and shared transport extraction; amend T11 Files to include solve.rs. This is a required contract repair before implementation, not a claim that the current sample code already meets it.

**T20 / Step 3 + Interfaces/Files (F14).** Replace the two unowned facade return types with:

```rust
// crates/engine/src/bench_support.rs — owned by this task
pub struct PreparedBenchSpot {
    pub line_id: String,
    pub depth_bb: u16,
    pub pot_class: String,
    pub input: Option<proto::SolveInput>,
    pub rake: proto::Rake,
    pub source_hashes: Vec<String>,
    pub inherited_reasons: Vec<proto::ApproxReason>,
    pub unavailable_reason: Option<String>,
}
pub fn generate_flop_spots(source: &Path)
    -> Result<Vec<PreparedBenchSpot>, EngineError>;
pub fn generate_street_spots(street: Street, template: &str, source: &Path)
    -> Result<Vec<PreparedBenchSpot>, EngineError>;
```

Replace the old FlopBenchSpot production claim with this DTO and both generators. Add suite.rs and runner.rs to Modify and insert:

> bench adapts PreparedBenchSpot into its own persisted Spot; engine never imports bench::suite::Spot. Change Spot’s oop_range/ip_range to a serde-untagged RangeSpec with Text(String) and Weights(Range1326) variants. R8 inputs remain Text and use prepared_range; chart inputs serialize full Weights arrays. Add source hashes, inherited reasons and explicit unavailable reason fields. An unavailable prepared spot has input=None and must never be executed or counted as a measurement. Update Suite::load/save, run_spot and gen-spots together, preserving old R8 JSON reading and enforcing chart provenance.

**T23 / Step 3a (F17/F16).** Replace the two nonexistent worker suites with one `contract_river` suite; add the engine check-only suite. The required set is:

```text
cargo test -p core-eval --features exhaustive --release
cargo test -p core-iso --features exhaustive --release
cargo test -p solver-worker --release --test contract_river
cargo test -p engine --release --test worker_link river_check_only_terminal_oracle -- --exact
cargo test -p engine --features testing --release facing_allin_golden
```

Dispatch the worker command through the V1-selected toolchain/target, with the selected built executable supplied through POKERAI_WORKER. Before these commands, if the 10M oracle is absent, run:

```text
tools/.venv/Scripts/python tools/gen_eval_oracle.py --skip-5card --samples 10000000 --samples-name phevaluator_7card_10m.bin
```

Check its exit code; do not proceed on failure. Record full command, exit code and executed-test count; an ignored/skipped or zero-test oracle cannot satisfy analytic_oracles_ok. The five-card committed oracle remains required.

**T23 / Step 3–5 and T26 / Interfaces/Step 1–2 (F21).** Add:

> GateInput has chart_ui_e2e_ok: bool, sourced only from Plan 5 Task 14’s actual named test result with the same worker/source/config provenance as the release candidate. Missing evidence is false. V21 and the bench-only bounds may be reported independently, but V22 additionally requires chart_ui_e2e_ok. Add the check ("chart UI E2E", g.chart_ui_e2e_ok) and a mutation test removing/failing that evidence. Task 26 consumes Tasks 24–25 and Plan 5 Task 14, then reruns bench gate and records the final V21/V22 decision. Never claim V22 from bench-only data.

**T20 Files, T24 Files/Step 2 (F24/F16).** Change the six suite paths from Create to Modify/regenerate and the existing dated report from Create to Modify/append. Replace the forced-MSVC build text with:

> Build production and diagnostic workers using the V1-selected worker toolchain/target; keep bench and the UI on MSVC. The diagnostic build uses a separate target directory and must not overwrite the production binary. Report the selected toolchain on every measured row. Regenerate all six chart suites before baseline measurements and retain prior R8 rows as explicitly separate reference evidence.

Add cache availability/error diagnostics to the existing StartupReport.banners/cache_state during startup, not a second return value from Engine::new.

### E05 — Plan 5

**Required façade block (F01/F02/F03/F10).** Replace the eight-entry block and its stale “binding fixes” comments with:

```rust
// Required from Plan 2 Task 29:
Engine::new(config: GameConfig, paths: Paths) -> Result<Engine, EngineError>;
Engine::startup_report(&self) -> StartupReport; // StartupReport.banners: Vec<String>
Engine::set_config(&mut self, config: GameConfig) -> Result<u32, EngineError>;
Engine::shutdown(&mut self);
Engine::set_hero_cards(&mut self, cards: [Card; 2]) -> Result<HandState, EngineError>;
Paths { log_dir: PathBuf, worker_exe: PathBuf, preflop: PathBuf, cache: PathBuf }

// Required from Plan 4 Task 16, with status re-exported by Task 15:
Engine::presolver_status(&self) -> cache::presolver::PresolverStatus;
Engine::presolver_pause(&mut self);
Engine::presolver_resume(&mut self);

// Admission DTO from Plan 1 Task 3, converted only inside the engine:
Engine::begin_hand(&mut self, begin: proto::BeginHand) -> Result<HandState, EngineError>;
```

Replace the gate prose at L51 and L151 with:

> Plan 2 Task 29’s produced façade is authoritative. The shell passes proto::BeginHand unchanged; the engine allocates identity and constructs core_model::BeginHand. Startup warnings come from Engine::startup_report().banners. Plan 4 Task 15 provides serializable PresolverStatus and Task 16 provides its engine delegation. Tasks 1–3 use the mock seam; Task 4 is the first real-engine adapter gate.

Update self-review:L3142/L3151/L3153 to the same contract.

**T1 / Step 4, proto ts-rs dependency (F15).** Replace its local version entry with:

```toml
[dependencies.ts-rs]
workspace = true
optional = true
features = ["serde-compat"]
```

Preserve any existing features in that table and keep proto’s typescript feature optional.

**T4 / Files + Step 3 (F03).** Files becomes:

> Modify apps/pokerai-ui/src-tauri/src/{service.rs,tests.rs}. No new core-model dependency is needed.

Replace the entire Op::Begin conversion arm with:

```rust
Op::Begin(b) => encode_hand(e.begin_hand(b).map_err(engine_error)?),
```

Remove the heading/prose that directs shell conversion and the planned core-model Cargo dependency. Keep ID-safe serialization tests.

**T5 / Step 4, Paths literal (F01).** Replace its literal with:

```rust
engine::Paths {
    worker_exe: resource.join("runtime/solver-worker.exe"),
    preflop: resource.join("runtime/preflop"),
    cache: local.join("cache/v3"),
    log_dir: local,
}
```

Here local is %LOCALAPPDATA%/PokerAI, not decisions.jsonl.

**T5 / Interfaces + Step 5a:L1215–1219 (F02).** Replace tuple destructuring/warning extension with:

```rust
let engine = engine::Engine::new(config.clone(), paths)
    .map_err(|e| e.to_string())?;
warnings.extend(engine.startup_report().banners);
```

Replace L1249’s startup sentence with:

> Engine::new returns Result<Engine,EngineError>. Extend the bootstrap warnings vector with engine.startup_report().banners after successful construction; Plans 3 and 4 populate that report with loader/cache diagnostics. Startup work stays on app-startup.

**T5 / Step 6, runtime staging (F16/F18).** Rename the step “Stage the selected worker runtime and available charts”. Replace the unconditional build/copy at L1259/L1263 with:

```powershell
$selection = Get-Content -LiteralPath 'docs/bench/worker-toolchain.json' -Raw | ConvertFrom-Json
if ($selection.worker_toolchain -eq 'stable-x86_64-pc-windows-gnu') {
    & powershell -NoProfile -File scripts/build-worker-gnu.ps1
    if ($LASTEXITCODE -ne 0) { throw 'GNU worker build failed' }
    $workerBinary = Join-Path $projectRoot 'target/release/solver-worker.exe'
} elseif ($selection.worker_toolchain -eq 'stable-x86_64-pc-windows-msvc') {
    cargo +stable-x86_64-pc-windows-msvc build --release -p solver-worker --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'MSVC worker build failed' }
    $workerBinary = Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release/solver-worker.exe'
} else {
    throw 'Missing or invalid V1 worker toolchain selection'
}
# After $stage has been created:
Copy-Item -LiteralPath $workerBinary -Destination $stage -Force
```

Replace the hard-coded two-name loop with:

```powershell
$chartRoot = Join-Path $projectRoot 'fixtures/charts'
$availabilityPath = Join-Path $chartRoot 'sources.manifest.json'
$availability = Get-Content -LiteralPath $availabilityPath -Raw | ConvertFrom-Json
$stagedPreflop = Join-Path $stage 'preflop'
Copy-Item -LiteralPath $availabilityPath -Destination $stagedPreflop -Force
foreach ($depth in $availability.depths) {
    if ($depth.status -ne 'available') { continue }
    foreach ($suffix in @('.json','.manifest.json')) {
        Copy-Item -LiteralPath (Join-Path $chartRoot ($depth.bundle_id + $suffix)) -Destination $stagedPreflop -Force
    }
}
```

Stage into a fresh build-specific directory before promotion, so a previously available bundle cannot remain in a reused destination after becoming unsupported. Validate every available pair through the existing loader/hash check. Replace L1278’s wildcard LiteralPath copy with:

```powershell
Get-ChildItem -LiteralPath $stagedPreflop -File | ForEach-Object {
    Copy-Item -LiteralPath $_.FullName -Destination (Join-Path $exeRuntime 'preflop') -Force
}
```

T5 consumes sources.manifest.json plus only its available sibling pairs. T14 requires the actual 100bb chart SRP to exist and keeps its numeric-EV/ChartRounded assertions; absence fails that acceptance requirement. Do not weaken P4’s required 200bb benchmark matrix.

**T14 / final acceptance paragraph (F21).** Replace the final V22 sentence with:

> Record this named chart E2E test’s pass/fail, elapsed admission-to-Final time and worker/source/config provenance as chart_ui_e2e evidence for Plan 4 Task 26. A missing or failed UI result blocks V22. Plan 4 Task 26 combines this evidence with the independent measured bench bounds and is the sole final V22 decision.

### E06 — Cross-reference and execution-gate cleanup (all plans)

Apply by **subject**, including File-structure and Self-review sections; the same old task number represented different subjects before revision.

| Stale subject/reference | Exact replacement text |
|---|---|
| P1 says P2.T1 adds solver-worker member; P1.T1 Step 3 says worker exists after P2.T1 | “Plan 2 Task 1 excludes the vendor; Plan 2 Task 7 creates solver-worker and adds that member.” |
| P1/P2 say bench oracle is P4.T20 | “bench oracle is created by Plan 4 Task 23.” |
| P2 says surrogate is P4.T10 | “The section-6 experimental surrogate and experimental_surrogate_golden are created by Plan 4 Task 11.” |
| P2 says chart suite regeneration is P4.T17 | “Plan 4 Task 17 freezes source hashes; Task 20 regenerates all six chart-replay suites.” |
| P2 says snapshots swap is P3.T11; preflop startup P3.T14 | “Plan 3 Task 14 replaces snapshots; Task 17 loads the store and installs preflop serving; Task 18 installs replay ranges.” |
| P3 says tools project was P1.T13 (L567) | “Plan 1 Task 15 creates tools/pyproject.toml and tools/tests/conftest.py.” |
| P4 says Paths/set_config/shutdown/module-list complete at P2.T21 | “Plan 2 Task 29 owns the public Engine façade; Task 21 owns DecisionLog only.” |
| P4 says deadline seam P2.T16 | “Plan 2 Task 20 owns Deadlines and watchdog arithmetic.” |
| P4 says bench_support.rs P2.T22 with range re-exports | “Plan 2 Task 5 creates engine::bench_support::{prepared_range,range_mass}; later tasks extend this file.” |
| P4 says assembly/advice rows P2.T19 | “Plan 2 Task 26 owns final_from_solution and the main recommendation assembler; advice_rows is not an upstream API.” |
| P4 says test fakes/testing feature P2.T15 | “The testing feature is declared in Plan 2 Task 2; FakeClock/FakeWorker/RecordingSink are produced in Task 19.” |
| P4 says SnapshotStore P3.T11 / SolvedStreetStore | “Plan 3 Task 13 defines snapshot records; Task 14 replaces Plan 2 Task 27’s SolvedStreet and SnapshotStore with core_replay types.” |
| P4 advertises free register_snapshot / engine::snapshots::register_snapshot | “Engine::register_snapshot(&mut self, &DecisionIdentity, StreetSnapshot) -> bool is the Plan 3 Task 18 façade; live/cache internals use the same SnapshotStore::register.” |
| P5 says status lacks Serialize and P4.T14 owns it | “Plan 4 Task 15 owns PresolverStatus with Serialize/Deserialize; Task 16 owns engine delegation.” |
| P3.T19 says Plan 2 Recommendation serde | “Recommendation serde is owned by Plan 1 Task 5; Plan 2 supplies the identity/fake-worker harness.” |
| Introductions refer to REVIEW-cross-plan §§4–5 as current order | “REVIEW-cross-plan §§4–5 are historical. Use REVIEW-cross-plan-2 §7 after its listed corrections are accepted.” |

Replace P3’s whole-plan prerequisite at L11, P4’s “Execute after Plans 1–3” at L40, and P5’s whole-plan gate at L11 with:

> Execute tasks against the hard prerequisites in REVIEW-cross-plan-2 §7 after the documented seam corrections are applied. Every prerequisite must be implemented, green and reviewed before a dependent task starts. A plan number alone is not an execution dependency. Plan 5 Task 14 precedes the final Plan 4 Task 26 release decision. Serialize edits to shared files when working in one checkout, and serialize measured benchmark/acceptance runs on the benchmark machine.

This is necessary to make the proposed task-level waves authoritative rather than contradict the old whole-plan gates. It is a scheduling proposal; the present unedited documents are still not ready.


## 7. Revised execution graph: all 114 tasks

**Status:** proposed order after E01–E06 and an authoritative F20 decision. The current unedited plans cannot have a fully executable order: manifest/visibility/name orphans are not solved by reordering, and the forward presolver-helper reference conflicts with each-task-green. Whole-plan prerequisite prose must be replaced as E06 specifies before these finer waves are used.

Notation `n.t` = Plan n, Task t. The table lists the immediate hard prerequisites used in the graph; their transitive prerequisites also apply. A prerequisite includes the task’s required green review, not merely a file existing. `★` marks membership in at least one **structural critical path** under one unit per task. There are no uniform real durations: chart acquisition is uncertain, P4.T24 alone estimates one to two machine-days (P4:L3894). A wall-clock critical path cannot be claimed from task counts. The long measurement tail is explicitly marked below.

Implementation edges include crate/manifest availability and required test artifacts, not only named Consumes items: P2.T2 needs all foundation crate manifests and validated foundation APIs; P5.T1’s manifest needs engine to exist even though its tests mock the backend. P2.T12 may copy the documented scaffolding from T13’s listing without executing T13 first. P3.T14 implements street_history/root_board early as its own text requires; T15 subsequently consumes them. F10 moves remaining_seconds to T15 and removes Presolver from T7, avoiding a false cycle. P4.T9 can be implemented before measured V3 data exists because missing data selects the conservative policy.

One additional **measurement-isolation edge** is included: P4.T25 → P5.T14. This avoids running the UI timing acceptance concurrently with the benchmark machine’s final measurement batch. It does not mean the UI functionally consumes every T25 measurement. Shared file edits in a wave must be merged/serialized; dependency independence is not permission to overwrite another task’s work or benchmark under concurrent CPU load.

### 7.1 Every task and its prerequisites

| Plan 1 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| ★ 1.1 — Workspace skeleton | — | 1 | P1.T1 Files/Interfaces, L88 |
| ★ 1.2 — `proto` cards and combos | 1.1 | 2 | P1.T2 Files/Interfaces, L230 |
| ★ 1.3 — `proto` game config and hand-state types | 1.2 | 3 | P1.T3 Files/Interfaces, L429 |
| 1.4 — `proto::Range1326` | 1.2 | 3 | P1.T4 Files/Interfaces, L684 |
| 1.5 — `proto` recommendation, coverage and event types | 1.3 | 4 | P1.T5 Files/Interfaces, L808 |
| 1.6 — `proto` effective tree, materialized nodes and chip-path resolution | 1.3, 1.4 | 4 | P1.T6 Files/Interfaces, L989 |
| 1.7 — `proto::worker` wire messages | 1.6 | 5 | P1.T7 Files/Interfaces, L1177 |
| 1.8 — `proto::worker::validate_solution` and `validate_locks` | 1.7 | 6 | P1.T8 Files/Interfaces, L1404 |
| ★ 1.9 — `core-model` skeleton: errors, card parsing, positions, config helpers | 1.3 | 4 | P1.T9 Files/Interfaces, L1607 |
| ★ 1.10 — `core-model::betting::Round` (one betting street) | 1.9 | 5 | P1.T10 Files/Interfaces, L1881 |
| ★ 1.11 — `core-model` settlement: refunds, side pots and the conservation invariant | 1.9 | 5 | P1.T11 Files/Interfaces, L2128 |
| ★ 1.12 — `core-model::lifecycle::simulate` (hand replay through one betting round) | 1.10, 1.11 | 6 | P1.T12 Files/Interfaces, L2276 |
| ★ 1.13 — `core-model` public state API | 1.12 | 7 | P1.T13 Files/Interfaces, L2509 |
| ★ 1.14 — `core-model` street root, `replay_root` and the §10.2 projection rule | 1.13 | 8 | P1.T14 Files/Interfaces, L2804 |
| 1.15 — `tools/` Python project with pinned oracle versions | 1.1 | 2 | P1.T15 Files/Interfaces, L3096 |
| 1.16 — `tools/gen_fixtures.py`, the PokerKit hand generator | 1.15 | 3 | P1.T16 Files/Interfaces, L3187 |
| 1.17 — generate and commit the 200 hand fixtures | 1.16 | 4 | P1.T17 Files/Interfaces, L3616 |
| ★ 1.18 — `core-model` replays the PokerKit fixtures | 1.14, 1.17 | 9 | P1.T18 Files/Interfaces, L3648 |
| 1.19 — `core-ranges` Pio range strings and 169-class expansion | 1.4 | 4 | P1.T19 Files/Interfaces, L3783 |
| 1.20 — `core-ranges` blocking, hero conditioning and `hash_scaled` | 1.19 | 5 | P1.T20 Files/Interfaces, L4060 |
| 1.21 — `core-iso` suit permutations and canonical boards | 1.19 | 5 | P1.T21 Files/Interfaces, L4163 |
| 1.22 — `tools/gen_eval_oracle.py` and the phevaluator fixtures | 1.15 | 3 | P1.T22 Files/Interfaces, L4400 |
| 1.23 — `core-eval` evaluator trait, b-inary backend and oracle tests | 1.22, 1.19 | 5 | P1.T23 Files/Interfaces, L4544 |
| 1.24 — `core-eval` exact equity, per-combo equity and terminal payoffs | 1.23, 1.20, 1.5 | 6 | P1.T24 Files/Interfaces, L4709 |
| 1.25 — `core-eval` Monte Carlo with joint disjoint sampling | 1.24 | 7 | P1.T25 Files/Interfaces, L5048 |

| Plan 2 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 2.1 — Vendor postflop-solver at the pinned commit with the two patches | 1.1 | 2 | P2.T1 Files/Interfaces, L111; §3 ledger |
| ★ 2.2 — Engine crate skeleton: clock, identity, tree templates | 1.8, 1.18, 1.21, 1.25 | 10 | P2.T2 Files/Interfaces, L207; §3 ledger |
| ★ 2.3 — Engine materializer with the pinned §4.6 rules | 2.2 | 11 | P2.T3 Files/Interfaces, L535; §3 ledger |
| ★ 2.4 — Effective tree with exact insertion, `tree_signature`, chip-path resolution, `tree_builder_golden` | 2.3 | 12 | P2.T4 Files/Interfaces, L861; §3 ledger |
| ★ 2.5 — `bench` crate skeleton: spot format, `gen-spots`, `materialize` | 2.4 | 13 | P2.T5 Files/Interfaces, L1094; §3 ledger |
| ★ 2.6 — `tools/gen_worker_fixtures.py` and the `fixtures/worker/*.jsonl` files | 2.5, 1.15 | 14 | P2.T6 Files/Interfaces, L1385; §3 ledger |
| 2.7 — `solver-worker` skeleton: AVX2 build check, card mapping, `ready`, EOF exit | 2.1, 1.8 | 7 | P2.T7 Files/Interfaces, L1692; §3 ledger |
| ★ 2.8 — Worker tree build, `add_line` insertion, `remove_line` wager cap, cross-check | 2.7, 2.6 | 15 | P2.T8 Files/Interfaces, L2015; §3 ledger |
| 2.9 — Worker memory admission (§10.3) and the §7 stop rule | 2.7 | 8 | P2.T9 Files/Interfaces, L2299; §3 ledger |
| ★ 2.10 — Worker extraction and lock application | 2.8 | 16 | P2.T10 Files/Interfaces, L2452; §3 ledger |
| ★ 2.11 — Worker job runner: Building -> Solving -> Extracting with cancel checkpoints | 2.9, 2.10 | 17 | P2.T11 Files/Interfaces, L2683; §3 ledger |
| ★ 2.12 — Worker stdout writer, bounded line reading and the three-thread wiring | 2.11 | 18 | P2.T12 Files/Interfaces, L2841; §3 ledger |
| ★ 2.13 — Worker state machine: admission, `ack` rules, cancel, shutdown | 2.12 | 19 | P2.T13 Files/Interfaces, L2984; §3 ledger |
| ★ 2.14 — Worker lock staging and the cancel lifecycle | 2.13 | 20 | P2.T14 Files/Interfaces, L3300; §3 ledger |
| ★ 2.15 — The pinned V1 fixture and the worker's river contract tests | 2.14 | 21 | P2.T15 Files/Interfaces, L3494; §3 ledger |
| 2.16 — Worker contract tests: materialization mismatch, wager cap, exact insertion, suit permutation, pinned example | 2.15 | 22 | P2.T16 Files/Interfaces, L3718; §3 ledger |
| 2.17 — Worker deadline and memory contracts | 2.15 | 22 | P2.T17 Files/Interfaces, L3882; §3 ledger |
| ★ 2.18 — Engine worker link: `WorkerLink`, `ProcessWorker`, `ready` validation, job object | 2.2, 2.15 | 22 | P2.T18 Files/Interfaces, L3973; §3 ledger |
| ★ 2.19 — Engine test doubles: `FakeClock`, `FakeWorker`, `RecordingSink`, solution builder | 2.18 | 23 | P2.T19 Files/Interfaces, L4354; §3 ledger |
| ★ 2.20 — Absolute deadlines and the independent watchdog | 2.19 | 24 | P2.T20 Files/Interfaces, L4525; §3 ledger |
| 2.21 — Decision log (§5 step 10) | 2.2 | 11 | P2.T21 Files/Interfaces, L4753; §3 ledger |
| ★ 2.22 — `EngineCore` and the `run_solve` happy path | 2.20, 2.21, 2.4 | 25 | P2.T22 Files/Interfaces, L4855; §3 ledger |
| ★ 2.23 — `run_solve` resilience: heartbeat, cancel-then-kill, `_min` retry, error-code policy | 2.22 | 26 | P2.T23 Files/Interfaces, L5153; §3 ledger |
| 2.24 — Coverage classifier (§6) and `coverage_classification_golden` | 2.19 | 24 | P2.T24 Files/Interfaces, L5348; §3 ledger |
| 2.25 — Equity adapter and the facing-all-in analytic fallback (`facing_allin_golden`) | 2.2 | 11 | P2.T25 Files/Interfaces, L5527; §3 ledger |
| 2.26 — Result assembly, headline rules, reason accumulation, equity merge (`recommendation_assembly_golden`) | 2.25, 2.20 | 25 | P2.T26 Files/Interfaces, L5697; §3 ledger |
| ★ 2.27 — Snapshot store and the root-range source | 2.22 | 26 | P2.T27 Files/Interfaces, L5943; §3 ledger |
| ★ 2.28 — `serve_request`: the river/turn decision path and `identity_race_golden` | 2.23, 2.24, 2.25, 2.26, 2.27 | 27 | P2.T28 Files/Interfaces, L6137; §3 ledger |
| ★ 2.29 — Public `Engine` API, startup report and `final_delivery_independent_of_worker` | 2.28 | 28 | P2.T29 Files/Interfaces, L6406; §3 ledger |
| 2.30 — `bench run` for the river and turn suites with the §13.5 report | 2.5, 2.20, 2.16, 2.17 | 25 | P2.T30 Files/Interfaces, L6806; §3 ledger |

| Plan 3 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 3.1 — Define and validate the normalized preflop envelope | 1.18, 1.20, 1.5 | 10 | P3.T1 Files/Interfaces, L121; §3 ledger |
| 3.2 — Load independent sources and quarantine invalid bundles | 3.1, 1.15 | 11 | P3.T2 Files/Interfaces, L328; §3 ledger |
| 3.3 — Build the deterministic chart ingestion and validation tool | 3.2 | 12 | P3.T3 Files/Interfaces, L557; §3 ledger |
| 3.4 — Acquire the public chart sources and record which depths shipped | 3.3 | 13 | P3.T4 Files/Interfaces, L706; §3 ledger |
| 3.5 — Inventory and transcribe the PokerCoaching 100bb grids | 3.4 | 14 | P3.T5 Files/Interfaces, L797; §3 ledger |
| 3.6 — Inventory and transcribe the RangeConverter 200bb grids | 3.4 | 14 | P3.T6 Files/Interfaces, L869; §3 ledger |
| 3.7 — Freeze chart coverage and load both layers through the Rust boundary | 3.5, 3.6 | 15 | P3.T7 Files/Interfaces, L938; §3 ledger |
| 3.8 — Reconstruct each prefix and select depth, rake and virtual roles | 3.2 | 12 | P3.T8 Files/Interfaces, L989; §3 ledger |
| 3.9 — Normalize all EV reference variants and expand classes | 3.8 | 13 | P3.T9 Files/Interfaces, L1341; §3 ledger |
| 3.10 — Implement wager interpolation and legality after mapping | 3.9 | 14 | P3.T10 Files/Interfaces, L1468; §3 ledger |
| 3.11 — Implement shared history-branch Bayesian updates | 3.10 | 15 | P3.T11 Files/Interfaces, L1625; §3 ledger |
| 3.12 — Cap live branches with a persistent frozen residual | 3.11 | 16 | P3.T12 Files/Interfaces, L1808; §3 ledger |
| 3.13 — Replay preflop prefixes with frozen missing-node stops | 3.12 | 17 | P3.T13 Files/Interfaces, L1920; §3 ledger |
| ★ 3.14 — Select compatible snapshots and preserve prefix-valid provenance | 3.13, 2.29 | 29 | P3.T14 Files/Interfaces, L2173; §3 ledger |
| ★ 3.15 — Walk completed streets through partial snapshot exports | 3.14 | 30 | P3.T15 Files/Interfaces, L2386; §3 ledger |
| 3.16 — Assemble branch-supported strategy, EV and unresolved mass | 3.12 | 17 | P3.T16 Files/Interfaces, L2584; §3 ledger |
| ★ 3.17 — Integrate the engine's chart and EV-bearing preflop decision path | 3.7, 3.15, 3.16, 2.29 | 31 | P3.T17 Files/Interfaces, L2697; §3 ledger |
| ★ 3.18 — Feed replayed root ranges into turn and river solves and register snapshots | 3.17 | 32 | P3.T18 Files/Interfaces, L2869; §3 ledger |
| 3.19 — Freeze replay and bet-translation goldens and audit every scoped test | 3.18 | 33 | P3.T19 Files/Interfaces, L2975; §3 ledger |

| Plan 4 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 4.1 — Introduce canonical cache keys and exact rational identity | 1.21, 1.20, 1.8 | 7 | P4.T1 Files/Interfaces, L143; §3 ledger |
| 4.2 — Normalize actor-owned payloads and resolve ordinal paths | 4.1 | 8 | P4.T2 Files/Interfaces, L248; §3 ledger |
| 4.3 — Compare complete trees, SPR and terminal rake-cap activation | 4.2 | 9 | P4.T3 Files/Interfaces, L492; §3 ledger |
| 4.4 — Preserve inherited reasons and filter raw accuracy | 4.3 | 10 | P4.T4 Files/Interfaces, L573; §3 ledger |
| 4.5 — Add bounded binary storage and delete corrupt entries | 4.2 | 9 | P4.T5 Files/Interfaces, L646; §3 ledger |
| 4.6 — Write atomically, retain two representatives and enforce quota | 4.5 | 10 | P4.T6 Files/Interfaces, L792; §3 ledger |
| ★ 4.7 — Serve validated ordinal nodes through bounded cache lookup | 4.6, 4.4, 3.18 | 33 | P4.T7 Files/Interfaces, L987; §3 ledger |
| ★ 4.8 — Freeze the complete T4 structural-identity scale contract | 4.7 | 34 | P4.T8 Files/Interfaces, L1297; §3 ledger |
| ★ 4.9 — Implement flop budget and evidence-based template selection | 2.29, 4.8 | 35 | P4.T9 Files/Interfaces, L1457; §3 ledger |
| ★ 4.10 — Route flop and turn through cache, provisional and live solve | 4.8, 4.9, 3.19 | 36 | P4.T10 Files/Interfaces, L1605; §3 ledger |
| ★ 4.11 — Create the section 6 experimental synthetic-root surrogate | 4.10, 2.25 | 37 | P4.T11 Files/Interfaces, L1982; §3 ledger |
| ★ 4.12 — Register cache snapshots and translate prior-street bets | 4.10 | 37 | P4.T12 Files/Interfaces, L2180; §3 ledger |
| 4.13 — Enumerate presolver tiers and canonical-flop order | 4.1 | 8 | P4.T13 Files/Interfaces, L2266; §3 ledger |
| 4.14 — Persist queue identities, cursor and verified completion | 4.13, 4.6 | 11 | P4.T14 Files/Interfaces, L2376; §3 ledger |
| 4.15 — Schedule idle jobs and cancel them for live admission | 4.14 | 12 | P4.T15 Files/Interfaces, L2630; §3 ledger |
| ★ 4.16 — Prepare presolves by chart replay, own the Presolver and report coverage | 4.15, 4.10, 3.18 | 37 | P4.T16 Files/Interfaces, L2860; §3 ledger |
| 4.17 — Freeze chart provenance, source hashes and the node inventory | 3.7, 2.5, 1.16 | 16 | P4.T17 Files/Interfaces, L2991; §3 ledger |
| 4.18 — Define the fifty recorded inputs as code | 4.17 | 17 | P4.T18 Files/Interfaces, L3121; §3 ledger |
| 4.19 — Generate, legality-check and freeze the fifty recorded hands | 4.18 | 18 | P4.T19 Files/Interfaces, L3330; §3 ledger |
| 4.20 — Regenerate all six chart-replay suites and measure V3 modes | 4.16, 4.19, 2.30 | 38 | P4.T20 Files/Interfaces, L3476; §3 ledger |
| ★ 4.21 — Replay recorded hands through the real engine | 4.19, 4.11, 4.12, 4.16 | 38 | P4.T21 Files/Interfaces, L3576; §3 ledger |
| ★ 4.22 — Inject worker, disk and clock faults behind WorkerLink | 4.21 | 39 | P4.T22 Files/Interfaces, L3645; §3 ledger |
| ★ 4.23 — Report measured results, run the oracle suites and enforce the baseline gate | 4.20, 4.22 | 40 | P4.T23 Files/Interfaces, L3729; §3 ledger |
| ★ 4.24 — Run the deterministic regression and the flop measurement matrix | 4.23 | 41 | P4.T24 Files/Interfaces, L3886; §3 ledger |
| ★ 4.25 — Measure the baseline suites, e2e, faults, pre-solved hits and the oracles | 4.24 | 42 | P4.T25 Files/Interfaces, L3952; §3 ledger |
| ★ 4.26 — Evaluate the release gate and freeze the reviewed baseline | 4.25, 5.14 | 44 | P4.T26 Files/Interfaces, L3997; §3 ledger; S§14 V22 / F21 |

| Plan 5 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 5.1 — Scaffold the strict React/Tauri workspace and build-time types | 2.2 | 11 | P5.T1 Files/Interfaces, L155; §3 ledger |
| 5.2 — Implement the bounded command dispatcher and typed errors | 5.1 | 12 | P5.T2 Files/Interfaces, L463; §3 ledger |
| 5.3 — Register the fourteen Tauri commands and prove IPC argument contracts | 5.2 | 13 | P5.T3 Files/Interfaces, L645; §3 ledger |
| 5.4 — Compile the real engine adapter and forward the recommendation channel | 5.3, 4.16 | 38 | P5.T4 Files/Interfaces, L840; §3 ledger |
| 5.5 — Persist configuration and own application startup and shutdown | 5.4, 3.7, 2.1 | 39 | P5.T5 Files/Interfaces, L1008; §3 ledger |
| 5.6 — Add the typed Backend, scripted fake, and mockIPC harness | 5.1 | 12 | P5.T6 Files/Interfaces, L1368; §3 ledger |
| 5.7 — Enforce active identity and merge progressive events | 5.6 | 13 | P5.T7 Files/Interfaces, L1570; §3 ledger |
| 5.8 — Build the session configuration screen | 5.6 | 13 | P5.T8 Files/Interfaces, L1798; §3 ledger |
| 5.9 — Implement confirmed hand setup and card entry | 5.8 | 14 | P5.T9 Files/Interfaces, L1972; §3 ledger |
| 5.10 — Wire the exact keyboard map to engine mutations | 5.7, 5.9 | 15 | P5.T10 Files/Interfaces, L2123; §3 ledger |
| 5.11 — Render recommendations, coverage, assumptions, equity, and experimental output | 5.7 | 14 | P5.T11 Files/Interfaces, L2423; §3 ledger |
| 5.12 — Compose the desktop session and complete the keystroke suite | 5.5, 5.8, 5.10, 5.11 | 40 | P5.T12 Files/Interfaces, L2649; §3 ledger |
| 5.13 — Suppress WebView accelerators and document the pinned Tauri 2 APIs | 5.12 | 41 | P5.T13 Files/Interfaces, L2876; §3 ledger |
| ★ 5.14 — Prove the chart-backed Windows SRP flop E2E contract | 5.13, 4.25 | 43 | P5.T14 Files/Interfaces, L2983; §3 ledger; measurement-isolation edge above |

### 7.2 One complete sequential topological order

Read left to right, then continue on the next line. Every one of the 114 tasks occurs exactly once. This is the wave-flattened order, which preserves all prerequisites; it does not require numeric task order.

```text
1.1 → 1.2 → 1.15 → 2.1 → 1.3 → 1.4 → 1.16 → 1.22 → 1.5 → 1.6 → 1.9 → 1.17
1.19 → 1.7 → 1.10 → 1.11 → 1.20 → 1.21 → 1.23 → 1.8 → 1.12 → 1.24 → 1.13 → 1.25
2.7 → 4.1 → 1.14 → 2.9 → 4.2 → 4.13 → 1.18 → 4.3 → 4.5 → 2.2 → 3.1 → 4.4
4.6 → 2.3 → 2.21 → 2.25 → 3.2 → 4.14 → 5.1 → 2.4 → 3.3 → 3.8 → 4.15 → 5.2
5.6 → 2.5 → 3.4 → 3.9 → 5.3 → 5.7 → 5.8 → 2.6 → 3.5 → 3.6 → 3.10 → 5.9
5.11 → 2.8 → 3.7 → 3.11 → 5.10 → 2.10 → 3.12 → 4.17 → 2.11 → 3.13 → 3.16 → 4.18
2.12 → 4.19 → 2.13 → 2.14 → 2.15 → 2.16 → 2.17 → 2.18 → 2.19 → 2.20 → 2.24 → 2.22
2.26 → 2.30 → 2.23 → 2.27 → 2.28 → 2.29 → 3.14 → 3.15 → 3.17 → 3.18 → 3.19 → 4.7
4.8 → 4.9 → 4.10 → 4.11 → 4.12 → 4.16 → 4.20 → 4.21 → 5.4 → 4.22 → 5.5 → 4.23
5.12 → 4.24 → 5.13 → 4.25 → 5.14 → 4.26
```

### 7.3 Parallel waves

All prerequisites of every task in a wave appear in earlier waves. “Parallel” describes dependency independence; shared-file integration and benchmark hardware still follow the isolation rule above.

| Wave | Tasks |
|---|---|
| 1 | ★ 1.1 |
| 2 | ★ 1.2, 1.15, 2.1 |
| 3 | ★ 1.3, 1.4, 1.16, 1.22 |
| 4 | 1.5, 1.6, ★ 1.9, 1.17, 1.19 |
| 5 | 1.7, ★ 1.10, ★ 1.11, 1.20, 1.21, 1.23 |
| 6 | 1.8, ★ 1.12, 1.24 |
| 7 | ★ 1.13, 1.25, 2.7, 4.1 |
| 8 | ★ 1.14, 2.9, 4.2, 4.13 |
| 9 | ★ 1.18, 4.3, 4.5 |
| 10 | ★ 2.2, 3.1, 4.4, 4.6 |
| 11 | ★ 2.3, 2.21, 2.25, 3.2, 4.14, 5.1 |
| 12 | ★ 2.4, 3.3, 3.8, 4.15, 5.2, 5.6 |
| 13 | ★ 2.5, 3.4, 3.9, 5.3, 5.7, 5.8 |
| 14 | ★ 2.6, 3.5, 3.6, 3.10, 5.9, 5.11 |
| 15 | ★ 2.8, 3.7, 3.11, 5.10 |
| 16 | ★ 2.10, 3.12, 4.17 |
| 17 | ★ 2.11, 3.13, 3.16, 4.18 |
| 18 | ★ 2.12, 4.19 |
| 19 | ★ 2.13 |
| 20 | ★ 2.14 |
| 21 | ★ 2.15 |
| 22 | 2.16, 2.17, ★ 2.18 |
| 23 | ★ 2.19 |
| 24 | ★ 2.20, 2.24 |
| 25 | ★ 2.22, 2.26, 2.30 |
| 26 | ★ 2.23, ★ 2.27 |
| 27 | ★ 2.28 |
| 28 | ★ 2.29 |
| 29 | ★ 3.14 |
| 30 | ★ 3.15 |
| 31 | ★ 3.17 |
| 32 | ★ 3.18 |
| 33 | 3.19, ★ 4.7 |
| 34 | ★ 4.8 |
| 35 | ★ 4.9 |
| 36 | ★ 4.10 |
| 37 | ★ 4.11, ★ 4.12, ★ 4.16 |
| 38 | 4.20, ★ 4.21, 5.4 |
| 39 | ★ 4.22, 5.5 |
| 40 | ★ 4.23, 5.12 |
| 41 | ★ 4.24, 5.13 |
| 42 | ★ 4.25 |
| 43 | ★ 5.14 |
| 44 | ★ 4.26 |

### 7.4 Critical path and release tail

One of the longest structural chains is **44 tasks / 43 edges**:

```text
1.1 → 1.2 → 1.3 → 1.9 → 1.10 → 1.12 → 1.13 → 1.14 → 1.18
2.2 → 2.3 → 2.4 → 2.5 → 2.6 → 2.8 → 2.10 → 2.11 → 2.12
2.13 → 2.14 → 2.15 → 2.18 → 2.19 → 2.20 → 2.22 → 2.23 → 2.28
2.29 → 3.14 → 3.15 → 3.17 → 3.18 → 4.7 → 4.8 → 4.9 → 4.10
4.11 → 4.21 → 4.22 → 4.23 → 4.24 → 4.25 → 5.14 → 4.26
```

All 48 zero-slack task nodes are starred in the tables; tied branches exist (for example P1.T10/T11 and P4.T11/T12/T16). The governing measured tail is **P4.T24 (V3 matrix) → P4.T25 (remaining measured suites) → P5.T14 (chart UI E2E) → P4.T26 (combined final V22)**. V3 policy data is consumed at runtime after measurement, not required to compile the policy’s conservative initial state.

First wave: **P1.T1 only**. Once reviewed, wave 2 is **P1.T2, P1.T15, P2.T1**. The wave order is conditional on the corrections; it is not a claim that implementation has started.

## 8. Verification, self-review and remaining decision

### 8.1 Finding-by-finding falsification check

| Finding | Self-review against the quoted authority / rejected false positive |
|---|---|
| F01 | Includes P3, not just requested P4/P5. Renaming log alone is insufficient because P2 appends the filename; directory semantics checked. |
| F02 | StartupReport is publicly re-exported at P2:L6790; this is a consumer mismatch, not a missing warning capability. |
| F03 | The two BeginHand types are intentional. Only the shell argument is wrong; no proto type deletion or second DTO is proposed. |
| F04 | Read P2’s actual registry body, not just its Interfaces. with_extra already exists, uses test-templates, and ids includes extras. |
| F05 | Same fully qualified engine function, same arguments, different return types and two bodies; genuine duplicated ownership. |
| F06 | Trait itself matches. The failure is specifically Box versus the shared Arc/Mutex field cloned into Engine; constructor arity stays four. |
| F07 | P2 emit has no pub(crate); P3 preflop.rs is a sibling and contains calls. No competing preflop-local helper is produced. |
| F08 | Checked P2 setter’s validation, queued_config branch and config lock, not only its Result signature. P4’s replacement loses those behaviors. |
| F09 | Checked engine-main’s actual mutex scope across serve_request. Independent UI dispatcher threads do not remove that engine lock wait. |
| F10 | Status derives are already present in P4.T15; no claim that serialization is missing. Missing parent re-export and future helper are separate, verified details. |
| F11 | All four actual P1 definitions located: UtgStraddle.amount_chips, IDENTITY, inverse, and postflop_order(button,dealt). No new aliases are necessary. |
| F12 | Spec says no SolveInput, not merely no cache. Checked absent advice_rows against full P2 assembler. Actual-combo/BB/rake need explicit transfer. |
| F13 | Searched every plan for bench_p95_ms. Only P4’s presumed-existing-field comment and reads exist; there is no field/loader producer. |
| F14 | Spot is bench::suite::Spot with string ranges; engine owns the proposed generators. Defining another same-named Spot without an adapter would hide the type problem. |
| F15 | Read every workspace dependency table. P5’s ts-rs is local to proto and cannot satisfy cache’s workspace inheritance; optional does not waive manifest resolution. |
| F16 | P1 explicitly delegates the measurement to P2; P2 explicitly consumes P1’s check. Smoke construction time is not the required FLOP-FAST comparator. Checked P4 diagnostic and P5 staging paths too. |
| F17 | P2 supplies contract_river.rs and worker_link.rs, not the two P4 target names. Five real required suites remain after replacement, including check-only. |
| F18 | P3 records absent depth as a valid green-workspace outcome. P4 honestly fails incomplete release measurements; P5 packaging must not pretend absence is impossible. |
| F19 | Spec explicitly includes every section-10.1 template and 47 cases. Phase-2 solve status is not a materialization-test exemption. |
| F20 | Both incompatible outcomes occur in the same authoritative §13.2. P2 acknowledges choosing one; no later revision in the supplied authorities resolves it. |
| F21 | P5.T14 is the chart UI acceptance test, distinct from bench e2e. GateInput lacks its evidence; final V22 therefore must follow it. |
| F22 | Actual revised task headings establish T11/T23 ownership; older task numbers were not used to construct the graph. Re-exports were distinguished from absent free functions. |
| F23 | Exact package names/paths agree; repeated glob-covered member declarations are metadata cleanup, not extra crate implementations. |
| F24 | P2 already writes all six suite files and the same default-dated report. Regeneration/append is intended; the corrective edit is ownership wording/provenance preservation, not deletion of either benchmark stage. |

The ledger was checked for every P2–P5 task; no matching-looking trait was substituted for a different receiver, error or result. File-structure claims and self-review declarations were checked against the task bodies. Intentional handoffs (snapshot replacement, generated TS derives, chart-suite regeneration, benchmarks extending existing files) were kept distinct from competing implementations.

### 8.2 Actual verification output

No crate exists yet. The repository’s requested default command was attempted:

```text
> cargo test --workspace
error: could not find `Cargo.toml` in `D:\Documents\Projects\PokerAI` or any parent directory
Exit code: 1
```

This is the expected pre-implementation repository state, **not a passed Rust test suite** and not an additional seam finding. No npm/Python/worker tests are claimed to have run.

A read-only Node document validator parsed the current sources and the written report. It checked task counts, all 84 paired finding quotes at their cited source lines, their 30-word maximum, all 89 consumer ledger rows, all 114 graph nodes, every edge against the sequential order and waves, the critical chain and Markdown fences. Real output:

```text
Task inventory: 25 + 30 + 19 + 26 + 14 = 114
Findings: 24 (BLOCKER=1, MAJOR=20, MINOR=3)
Paired finding quotes verified against source lines: 84
Consumes ledger rows: 89/89
DAG: 114 nodes, 173 edges, acyclic
Sequential order: 114 unique tasks; every prerequisite precedes its consumer
Parallel waves: 44; every prerequisite is in an earlier wave
Structural critical path: 44 tasks / 43 edges
Markdown fences: balanced
Document checks: PASS
Exit code: 0
```

Command: the following PowerShell here-string piped to `node -` (no helper file was created):

```powershell
@'
const fs = require('fs');
const assert = require('assert/strict');
const names = ['2026-09-10-plan-1-foundation.md','2026-09-10-plan-2-worker-engine.md','2026-09-10-plan-3-preflop-replay.md','2026-09-10-plan-4-flop-cache-presolver.md','2026-09-10-plan-5-ui.md'];
const sources = Object.fromEntries(names.map((n,i) => ['P'+(i+1),fs.readFileSync('docs/superpowers/plans/'+n,'utf8').split(/\r?\n/)]));
sources.S=fs.readFileSync('docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md','utf8').split(/\r?\n/);
const report=fs.readFileSync('docs/research/REVIEW-cross-plan-2.md','utf8');
const counts=names.map((n,i)=>sources['P'+(i+1)].filter(x=>/^#{2,3} Task \d+:/.test(x)).length);
assert.deepEqual(counts,[25,30,19,26,14]);
console.log('Task inventory: '+counts.join(' + ')+' = '+counts.reduce((a,b)=>a+b,0));
const findingSection=report.split('## 2. Findings:')[1].split('## 3. Complete')[0];
const findings=[...findingSection.matchAll(/^### (F\d+) \u2014 (BLOCKER|MAJOR|MINOR) \u2014 /gm)];
assert.equal(findings.length,24);
const severity=['BLOCKER','MAJOR','MINOR'].map(s=>s+'='+findings.filter(f=>f[2]===s).length);
console.log('Findings: '+findings.length+' ('+severity.join(', ')+')');
let quoteCount=0;
for(const m of findingSection.matchAll(/^- \*\*(P\d|S)(?:\.T\d+\/[^:]+)?:L(\d+)\*\*: \u201c(.*)\u201d$/gm)){
 const [,key,line,q]=m;assert(sources[key][+line-1].includes(q),'quote '+key+':'+line);
 assert(q.trim().split(/\s+/).length<=30);quoteCount++;
}
assert.equal(quoteCount,84);console.log('Paired finding quotes verified against source lines: '+quoteCount);
const ledger=report.split('## 3. Complete')[1].split('## 4. Explicit')[0];
assert.equal([...ledger.matchAll(/^\| P[2-5]\.T\d+, L\d+ \|/gm)].length,89);
console.log('Consumes ledger rows: 89/89');
const graphBlock=report.split('### 7.1 Every task')[1].split('### 7.2')[0];
const graph={};const waveByTask={};
for(const m of graphBlock.matchAll(/^\| (?:\u2605 )?(\d+\.\d+) \u2014 .*? \| ([^|]+) \| (\d+) \|/gm)){
 graph[m[1]]=m[2].trim()==='\u2014'?[]:m[2].trim().split(', ');
 waveByTask[m[1]]=+m[3];
}
assert.equal(Object.keys(graph).length,114);
const expected=new Set(counts.flatMap((n,p)=>Array.from({length:n},(_,i)=>(p+1)+'.'+(i+1))));
assert.deepEqual(new Set(Object.keys(graph)),expected);
for(const [t,ds] of Object.entries(graph))for(const d of ds){assert(expected.has(d));assert(waveByTask[d]<waveByTask[t],t+' <- '+d);}
const sequential=report.split('### 7.2 One complete')[1].split('### 7.3')[0].match(/\x60{3}text\n([\s\S]*?)\x60{3}/)[1].match(/\d+\.\d+/g);
assert.equal(sequential.length,114);assert.deepEqual(new Set(sequential),expected);
const pos=Object.fromEntries(sequential.map((x,i)=>[x,i]));
for(const [t,ds] of Object.entries(graph))for(const d of ds)assert(pos[d]<pos[t]);
const waveRows=report.split('### 7.3 Parallel waves')[1].split('### 7.4')[0];
const seen=new Set();let nWaves=0;
for(const m of waveRows.matchAll(/^\| (\d+) \| (.*) \|$/gm)){
 nWaves++;for(const t of m[2].match(/\d+\.\d+/g)){assert(!seen.has(t));seen.add(t);assert.equal(waveByTask[t],+m[1]);}
}
assert.equal(nWaves,44);assert.equal(seen.size,114);
const critical=report.split('### 7.4 Critical path')[1].match(/\x60{3}text\n([\s\S]*?)\x60{3}/)[1].match(/\d+\.\d+/g);
assert.equal(critical.length,44);
for(let i=1;i<critical.length;i++)assert(graph[critical[i]].includes(critical[i-1]));
console.log('DAG: 114 nodes, '+Object.values(graph).reduce((n,x)=>n+x.length,0)+' edges, acyclic');
console.log('Sequential order: 114 unique tasks; every prerequisite precedes its consumer');
console.log('Parallel waves: 44; every prerequisite is in an earlier wave');
console.log('Structural critical path: 44 tasks / 43 edges');
assert.equal([...report.matchAll(/^\x60{3}/gm)].length%2,0);
console.log('Markdown fences: balanced');
console.log('Document checks: PASS');
'@ | node -
```

`git -c safe.directory=D:/Documents/Projects/PokerAI diff --check` returned exit 0 with **no output**. The final status shows only `M docs/research/REVIEW-cross-plan-2.md`; the report's structure was also checked by the validator above. The rules, plans, spec, .gitignore and docs maintenance files were left untouched.

### 8.3 Remaining decision and completion boundary

The review deliverable is complete. Implementation readiness is not: first resolve **F20** in the authoritative spec, then apply E01–E06 with the required plan/spec changelogs in a separate task, and re-check the changed seams. The proposals preserve P2’s Paths/startup choices, assign every new helper to an existing numbered task, and keep 114 tasks; they do not make unmeasured timing, missing charts or a future release gate pass by assertion.

After those corrections, the first execution wave is **P1.T1**. Until then, this report’s graph is a validated **proposed** dependency order rather than authorization to ignore the conflicting current plans.
