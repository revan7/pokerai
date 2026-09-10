# Review of Plan 3 (preflop charts + replay), 2026-09-10

Plan reviewed: `docs/superpowers/plans/2026-09-10-plan-3-preflop-replay.md` (1,947 lines, 16 tasks).
Against: spec revision 5 (`docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`), the decision
outline §0b, the writing-plans SKILL, the series brief, `docs/research/R8-solver-bench.md`, and the
sibling plans 1 and 2 (for the interfaces this plan consumes).

**Verdict: NOT READY.** 2 BLOCKER, 8 MAJOR, 16 MINOR.

The numeric core of the plan is strong: every §13.1 figure for T1, T3, T6, T7,
`replay_off_tree_pseudo_harmonic` and `replay_branch_cap_residual` was recomputed during this review
and is correct (details in §8 below). The failures are concentrated in (a) the hand-off to Plan 2's
engine, which the plan targets at files and shapes Plan 2 does not produce, and (b) chart acquisition
and manifest generation.

---

## 1. Findings

Severity key: **BLOCKER** = execution cannot proceed correctly without a plan edit;
**MAJOR** = will cost an executor significant rework or silently produce a wrong system;
**MINOR** = should be fixed but is self-correcting during execution.

### BLOCKER

**B1 — Task 11 Step 4 / Task 15 Step 5: a second, conflicting `SnapshotStore`.**
Quoted: *"pub struct SnapshotStore{entries:Vec<StreetSnapshot>} … pub fn register(&mut self,active:&DecisionIdentity,snap:StreetSnapshot)->bool"* (Task 11 Step 4).
Plan 2 already ships `crates/engine/src/snapshots.rs` with
`snapshots::{SolvedStreet{identity_at_solve, street, board, tree, nodes, ordinal_paths, exploitability_chips, reasons, solved_prefix}, SnapshotStore::{new, register(&DecisionIdentity, SolvedStreet)->bool, invalidate_hand(u64), for_hand(u64)}}`,
held as `EngineCore.snapshots: Arc<Mutex<SnapshotStore>>` and wired into `serve_request`, mutation
invalidation and result delivery. Plan 2 states explicitly: *"plan 3 wraps `SolvedStreet` into
`StreetSnapshot` and adds `Engine::register_snapshot`"* and, in its hand-off list,
*"this plan provides `SnapshotStore::register(&DecisionIdentity, SolvedStreet)` with the same identity
rule, and plan 3 wraps it."* Plan 3 never mentions `SolvedStreet`, `engine::snapshots` or
`EngineCore.snapshots`; it defines an independent `core_replay::SnapshotStore` with a different API
(`invalidate(&HandState)`, `for_identity(&DecisionIdentity)`). Two stores will coexist: accepted
results register into one and `replay` reads the other, so every postflop street silently falls back
to `UnconditionedPriorStreet{"no compatible snapshot"}` and §9.2 prefix reuse never happens — while
all tests still pass, because Task 11/12 tests construct `StreetSnapshot`s by hand.
**Edit:** in Task 11 Step 4, state that `core_replay::SnapshotStore` *replaces*
`engine::snapshots::SnapshotStore`; delete `SolvedStreet` (its fields are a subset of `StreetSnapshot`
plus `board`→`key.root_board` and `ordinal_paths`→`covered_paths`); retype `EngineCore.snapshots` to
`Arc<Mutex<core_replay::SnapshotStore>>`; map Plan 2's `invalidate_hand(hand_id)` call sites onto
`invalidate(&HandState)`; and add `crates/engine/src/snapshots.rs` + `crates/engine/src/core.rs` to
Task 15's **Files**.

**B2 — Task 15 (Files, Steps 3/5) and Task 14 Step 3: the engine integration targets a file Plan 2 does not create, and a signature Plan 2's hand-off cannot accept.**
Quoted (Task 15 Files): *"modify `crates/engine/src/{lib.rs,postflop.rs}`"*; and (File structure)
*"`crates/engine/src/postflop.rs` | Replace initial range provider in Plan 2 turn/river path."*
Plan 2's engine module list contains no `postflop.rs`. The two real seams are:
 - **Root ranges:** `crates/engine/src/ranges.rs` — `RangeSource: Send { fn ranges_at_root(&self, state:&HandState, root:&StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>; }`, with `RootRanges { oop, ip, reasons, ranges_used }`, defaulted to `ExplicitRanges` in `EngineCore.range_source`. Plan 3's `prepare_root(state, store, snapshots, tree, target_bp) -> Result<(SolveInput, ReplayOutput), UnsupportedReason>` derives the root itself and returns a `SolveInput`, which does not implement or replace that trait; Plan 2 already derives the root and builds `SolveInput` in `serve.rs`, so `prepare_root` duplicates the street-root derivation and error classification the plan itself says to reuse.
 - **Preflop dispatch:** `crates/engine/src/serve.rs`, `serve_request`, arm `Classification::Preflop => … RecommendationEvent::Final(assemble::unsupported(&ctx, UnsupportedReason::EngineError{ message: "no preflop path in this build (plan 3)" …)))`. That arm is the exact line Task 14 must replace; it is named in neither Task 14's Files nor its steps.
Also: Plan 2's `Paths { log_dir: PathBuf, worker_exe: PathBuf }` has no preflop/chart directory, and
`Engine::new(GameConfig, Paths) -> Result<Engine, EngineError>` and
`recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError>` both return
`Result` (Task 14's Consumes writes `recommend(sink)->DecisionIdentity`).
**Edit:** replace `crates/engine/src/postflop.rs` with `crates/engine/src/ranges.rs` throughout;
restate Task 15 Step 3 as "implement `struct ReplayRanges { store, snapshots }` implementing
`RangeSource::ranges_at_root`, returning `RootRanges` built from `replay(...)` plus its reasons and
`ranges_used`, and set `EngineCore.range_source` to it"; keep `opposing_equity_ranges` as shown; add
`crates/engine/src/{engine.rs,serve.rs,core.rs,ranges.rs}` to the Files of Tasks 14/15; add
`preflop_dir: PathBuf` to `Paths` and name `engine.rs` as the file that changes; correct the two
`Engine` signatures to their `Result` forms.

### MAJOR

**M1 — Plan-wide: thirteen `Produces` signatures have no implementation code, only prose.**
The SKILL's No-Placeholders rule is explicit: *"Steps that describe what to do without showing how
(code blocks required for code steps)."* Functions declared in **Interfaces** with a signature but no
code anywhere: `PreflopStore::open` and `load_bundle` (Task 2 Step 4), `PreflopStore::query`
(Task 5 Step 4), `expand_node` (Task 6 Step 4), `legalize` (Task 7 Step 4), `split_action`
(Task 9 Step 4), `replay`, `walk_preflop`, `query_translated` (Task 10 Steps 4–5), `covered_prefix`
and `SnapshotStore::invalidate` (Task 11 Steps 3–4), `walk_postflop` (Task 12 Step 3), `mix_nodes`
(Task 13 Step 4), `preflop_final` and `Engine::register_snapshot` (Tasks 14–15). `replay` — the
plan's headline deliverable — has no code at any point. `covered_prefix` is the crux of §9.2 snapshot
selection (*"Count consecutive observed actions for which all mapped paths with nonzero interpolation
coefficients are covered"*) and drives `select_snapshot`'s ordering; `invalidate` carries four
conditional rules. **Edit:** add code blocks for at minimum `replay`, `walk_preflop`, `walk_postflop`,
`covered_prefix`, `invalidate`, `legalize` and `split_action`; the remaining six are short enough that
naming their exact body in prose is defensible, but the seven above are where an executor will diverge.

**M2 — Task 3 Step 4 / Task 4: the chart manifest writer has no code and does not specify the fields the Rust loader requires.**
Quoted: *"`build` uses transcription metadata for source/accuracy/rake/license, forces
`source=ChartTranscription`, `ev_reference=unverified`, and hashes exactly the bytes it writes."*
Task 2's `checked_envelope` rejects a bundle unless `info.game == "nl" && info.version == 2 &&
info.source_blinds == [0.5,1.0] && info.ev_unit == "source_sb"`, and `BundleInfo` has no serde
defaults, so a manifest missing `game`, `version`, `depths`, `source_blinds`, `ev_unit` or `sha256`
fails to deserialize. Task 4 Step 8 then loads both chart bundles through `load_bundle`, so the task
cannot pass as written. Note also that charts must carry `rake: None` with `rake_profile:
"undocumented"` (Task 3's test) — the only shape in the plan that exercises `rake_rank`'s unknown arm.
**Edit:** add the `manifest_for_chart(envelope, transcription) -> dict` body to Task 3 Step 4 with all
fifteen `BundleInfo` fields enumerated, mirroring `manifest_for` in `gen_preflop_fixtures.py`, and add
a Python assertion that the emitted manifest keys equal the Rust `BundleInfo` field set.

**M3 — Task 4: no contingency for the 200bb source, while two tests hard-require it.**
Quoted (Step 2): *"Planning-time verification (2026-09-10): … RangeConverter's article is accessible,
but its PDF download timed out."* Step 1's `@pytest.mark.parametrize("name", ["pokercoaching_100",
"rangeconverter_200"])` and Step 8's Rust envelope test both require both bundles, and Step 8 runs
`cargo test --workspace` — so a still-unavailable 200bb PDF leaves the workspace red with no way
forward inside the plan. Plan 4's 200bb bench spots (§13.5) and tier-3 pre-solver scenarios (§10.5)
also consume this bundle. Task 4 also carries a single commit (Step 8) for what is realistically
30–60 transcribe-then-independently-reread cycles across two PDFs; a stop mid-task loses everything.
**Edit:** add an explicit Step 2b: if the 200bb PDF is unobtainable after following the download page,
record the failure in `docs/data/chart-transcription.md`, ship 100bb only, mark the 200bb parametrize
case `pytest.mark.skipif` on file absence, drop the 200bb load from the Rust test, and record for
Plan 4 that 200bb bench spots and tier-3 scenarios have no range source. Add a per-grid commit
instruction to Steps 5/6 (`git commit -m "chore(charts): transcribe <title>"`).

**M4 — Task 5 Step 4: `PreflopNodeKey` must be built from the candidate bundle, not from the live config; never stated.**
`node_key` serializes `(depth_bb, rake_profile, straddle, history)`, and Task 5's tests hard-code
`rake_profile:"5% cap 0.5bb", straddle:false` — the synthetic bundle's own values. A chart bundle's
`rake_profile` is `"undocumented"` and its `straddle` is `false` even for a straddled hand (§8.3's
virtual roles are "lookup-only"), and `depth_bb` must be the *bucketed* depth. A `query` that builds
one key from `cfg.rake`/`cfg.straddle` misses every chart node while still emitting
`RakeProfileMapped`/`StraddleMapped`, which is exactly the failure mode §8.3 forbids
(*"never a guessed node"* / *"Missing nodes stay missing"*).
**Edit:** in Task 5 Step 4 add: "`query` builds one `PreflopNodeKey` **per candidate bundle**, taking
`depth_bb` from `bucket(actual, candidate.depths)`, `rake_profile` and `straddle` from
`candidate.bundle_info()`, and the history from `virtual_position`-mapped roles plus
`short_handed_prefix(n)`."

**M5 — Task 2 Step 4: the source-post units contradict the SB convention used everywhere else.**
Quoted: *"Build `committed_by_actor_sb` by replaying source posts `.5/1` and the source prefix in
source-SB units"*. Source blinds are `sb = 0.5 bb, bb = 1 bb`, i.e. **1 and 2 source SB**. Task 2
Step 3's `cases.json` records *"SB committed=1, BB committed=2"* and Task 6's T1 test passes
`committed = 1` (SB) and `committed = 2` (BB) and asserts `1.84 + 1 = 2.84`. Taking `.5/1` literally
halves `committed`, which changes `net_hand_start_verified` and `absolute_stack_verified` results and
breaks the fold cross-checks (`verify_fold(NetHandStart, Some(-1.), 1., 200.)` would fail at
`committed = 0.5`). **Edit:** replace `.5/1` with "the source posts, 1 SB and 2 SB".

**M6 — Task 5 Step 1: `depth_bucket_labels_per_prefix` asserts arithmetic on literals; `asymmetric()` is never exercised.**
Quoted: *"assert!((104.0-100.0)/100.0 <= 0.05); assert!((110.0-100.0)/100.0 > 0.05);"* — both are
tautologies over constants and cannot fail regardless of the implementation. The same pattern recurs
inside the bucket loop (`assert_eq!((actual-used as f64).abs()/used as f64>0.05, prominent)`), which
recomputes the prominence formula instead of calling the function. `asymmetric(stacks, used)` is
defined in Step 3 and called nowhere. The spec row requires *"actor 100 versus opponent 104 -> depth
100 with `AsymmetricStacks{prominent: false}` and never `Exact`; 100 versus 110 -> prominent"*.
**Edit:** replace the two literal asserts with
`assert_eq!(core_preflop::asymmetric(&[100.0,104.0],100),(true,false));`
`assert_eq!(core_preflop::asymmetric(&[100.0,110.0],100),(true,true));`
and add a `query`-level assertion that the returned `PreflopAnswer.reasons` contains
`ApproxReason::AsymmetricStacks{..}` and the coverage is not `Exact`.

**M7 — Tasks 5, 10, 15: `HandState.stacks_start` is dealt-aligned while `Derived` vectors are `Seat.0`-indexed; no mapping is shown.**
Plan 1 states both facts: *"`HandState.stacks_start` is aligned with `HandState.dealt`; `Derived`
vectors are indexed by `Seat.0`"* and *"Per-seat vectors are indexed by `Seat.0` (length 6); an
undealt seat is `folded = true`"*. Task 5 Step 3 says *"stack values are starting stacks, not
remaining stacks"* and `depth_for(actor, others, unit)` takes raw `u32`s, but no step shows the
`dealt`→seat mapping; indexing `stacks_start` by `seat.0` silently yields another seat's stack and a
wrong depth bucket, wrong `AsymmetricStacks` and a wrong `MissingPreflopNode` rate. Related: Task 15
Step 3's hedge *"if Plan 1's folded vector stores vacant=false, use `state.dealt` filtering"* is
backwards — Plan 1 stores vacant = **true** (the shown code is nevertheless correct, because it
filters `state.dealt` first).
**Edit:** add to Task 5 Step 3
`pub fn start_stack(state:&HandState, seat:Seat)->u32 { let i = state.dealt.iter().position(|&s| s==seat).expect("dealt seat"); state.stacks_start[i] }`
and use it in the depth/eligibility text; correct the Task 15 hedge to "Plan 1 marks undealt seats
`folded = true`, so `state.dealt` filtering is required, as shown".

**M8 — Tasks 14 and 15: the Files lists omit every file the steps actually change.**
Task 14 Files: *"Modify `crates/engine/{Cargo.toml,src/lib.rs,src/assemble.rs}`; create
`crates/engine/src/preflop.rs`, `crates/engine/tests/preflop_replay.rs`"* — yet Step 3 adds
`Send + Sync` supertraits to `PreflopSource` (in `crates/core-preflop/src/envelope.rs`, which Step 6's
`git add` does include but the Files list does not), adds a chart directory to `Paths`
(`crates/engine/src/engine.rs`), and replaces the preflop dispatch arm
(`crates/engine/src/serve.rs`). Task 15 Files name `postflop.rs` (see **B2**) and omit `engine.rs`
(`register_snapshot`, `preflop_store`), `core.rs` (`range_source`, `snapshots`), `ranges.rs` and
`snapshots.rs`. A per-task executor reading only its own task will not find the code it must modify.
**Edit:** rewrite both Files blocks against Plan 2's actual module list
(`lib.rs, clock.rs, identity.rs, tree/*, worker/*, testing.rs, deadline.rs, watchdog.rs, solve.rs,
coverage.rs, equity.rs, allin.rs, assemble.rs, log.rs, snapshots.rs, ranges.rs, serve.rs, core.rs,
engine.rs`) and update the File structure table accordingly.

### MINOR

**N1 — Task 8 Step 1/3:** tests reference `core_replay::HistoryBranch` and Task 11 references
`core_replay::{StreetSnapshot, SnapshotKey, select_snapshot}`, but the plan only specifies
*"`core-replay/src/branches.rs` uses `pub use core_preflop::branches::*;`"*, which puts the types at
`core_replay::branches::*`. Add "`lib.rs` re-exports `pub use branches::*;` and, from Task 10,
`pub use snapshot::*;`".

**N2 — Task 8 Step 1:** `replay_bayes_two_combos` omits the spec's T3 assertion *"an inserted observed
bet changes the posterior (forcing 1 fails)"*. Task 12 Step 3 states the rule
(*"Inserted observed sizes use their solved probability, never 1"*) but attaches no assertion to the
T3 test name. Add it to Task 12's `replay_snapshot_prefix_reuse` and note the relocation in the Task 16
audit table.

**N3 — Task 5 Step 1:** `straddle_mapping_labels` omits the spec's *"short post and re-straddle
rejected"*. Step 4's prose covers the behaviour but no assertion exists. Add two `assert!(matches!(…,
Err(RulesError::FormatUnsupported{..})))` cases to the named test.

**N4 — Task 5 Step 1:** `rake_profile_ordering` asserts only the rank tuple; it never asserts that the
`5% cap 0.5bb` bundle is *selected* nor that `RakeProfileMapped{actual, used}` is emitted, which is
what §13.1's row states (*"10%/$6 cap at 2/5 maps to '5% cap 0.5bb' with `RakeProfileMapped`"*). Add a
`PreflopStore::query` assertion on `PreflopAnswer.reasons`.

**N5 — Task 6 Step 4:** `expand_column(&[f32;169])` / `expand_optional(&[Option<f32>;169])` cannot be
called directly on `PreflopNode.probs` / `ev_source_sb`, which are `Vec<Vec<_>>` shaped `[169][actions]`;
no column-extraction code is shown. Also, `expand_optional` as written rebuilds all 169 indicator
expansions per call, contradicting the same step's *"Cache the 169 indicator expansions once at bundle
load"*. Show the column extraction and take the cached indicator table as a parameter.

**N6 — Task 7 Interfaces:** `legalize(actions:&[Action], legal:&[LegalAction], probs:&[f32], evs:&[Option<f32>])`
is per-combo, but Step 4 requires *"Compute all 1326 probability rows using one shared
action-destination map"*. The signature cannot express the shared map. Split into
`destination_map(actions, legal) -> Vec<(usize, Destination)>` plus a per-row `legalize_row`.

**N7 — Task 2 Step 3:** `make_node(history, actor, actions, committed)` never uses `committed`
(`committed_by_actor_sb` is computed by the Rust loader). Drop the parameter or use it to emit a
cross-check field in `cases.json`.

**N8 — Task 2 Step 3:** the sparse zero-weight EV lives at class 14 (`KK`) in `nodes.json`
(`evs[1][14] = 2.31`) but `range.json` documents it at `QQ` (`"evs": {"AKs": 1.84, "QQ": 2.31}`), and
`range.json`'s `weights {"QQ": .62, "JJ": 1}` have no counterpart in `make_node`. Align the two files
or state that `range.json` is a provider-shaped illustration only.

**N9 — Task 2/13.0 deviation:** §13.0 lists `fixtures/preflop/synthetic_v2/*.json` as *"hand-written
from R7 §4's documented response shape"*; the plan generates them with `tools/gen_preflop_fixtures.py`.
The change is an improvement (reproducible, hashable) but is not listed in the plan's
"Spec-coverage gaps / conflicts" section. Record it there.

**N10 — Task 1 Step 3 deviation:** `BundleInfo` gains `rake: Option<RakeProfile>` beyond §8.1's field
list and §8.2's manifest (which carries only the `rake_profile` string). This is necessary for §8.3's
`(rate, cap_bb, no_flop_no_drop)` ordering and is well argued in-line, but it is a spec extension that
`tools/pokerdata_convert.py` (V9, out of scope) will have to honour. Record it in the deviations
section with that note.

**N11 — Task 1 Step 4:** *"Reject duplicate action kind/amount pairs and duplicate unreachable indices
with the same set-insertion pattern used for history keys"* — described, not coded, in a step that
otherwise shows the full validator. Add the two `BTreeSet` inserts to the shown `validate` body.

**N12 — Task 3 Step 1 / Task 4 Step 1:** `from tools.chart_ingest import class_names, build, validate`
requires the repository root on `sys.path`; under pytest's default `prepend` import mode the inserted
directory is `tools/tests`, not the root. Task 4's test additionally uses the CWD-relative
`Path("fixtures/charts")`. State the arrangement (root `conftest.py`, or `pythonpath = ["."]` in
`tools/pyproject.toml`) and resolve fixture paths from `Path(__file__).resolve().parents[2]`.

**N13 — Task 3 Step 3:** `validate(e)` iterates `enumerate(zip(*n["weights"]))`, which yields nothing
for a node with an empty action list, so a zero-action node passes the Python validator (the Rust one
rejects it via `n.actions.is_empty()`). Add `if not n["actions"]: raise ValueError("empty menu")`.

**N14 — Task 4 Step 1:** the test compares `{json.dumps(n["history"]) for n in t["inventory"] if
n["status"]=="covered"}`, i.e. inventory rows carry a `history`, but Task 3's Produces describes the
inventory as *"covered and absent node **titles** with reasons"*. Add `history` and `status` to the
documented inventory row shape.

**N15 — Task 14 Step 1:** the test calls `engine::preflop::set_headline`, which requires
`pub mod preflop;` in `crates/engine/src/lib.rs` plus the Step 4 re-export; and Plan 2 places
`FakeClock`/`FakeWorker`/`RecordingSink` in `crates/engine/src/testing.rs` behind
`#[cfg(any(test, feature = "testing"))]`, so `crates/engine/tests/preflop_replay.rs` needs
`engine = { path = ".", features = ["testing"] }` as a dev-dependency. Neither is stated.

**N16 — Task 5 Step 3 / Task 16 Step 3:** two small semantic gaps. (a) `bucket` implements §8.3's
*"above 200 clamps to 200"* as nearest-depth with a `d <= 200` filter, which coincides with the spec
only while 200 is an acquired depth — true for the chart set, but state the assumption. (b) Task 16
Step 3 requires *"a second three-seat line containing an off-menu wager"* compared against *"the
independently implemented §8.4 kernel"*, but `replay_golden_expected()` only covers the uniform
on-menu line; the second oracle has no code.

---

## 2. Spec coverage (dimension 1)

Every requirement in this plan's slice of the series map is mapped to a task. No gaps found.

| Spec scope (brief's series map) | Tasks | Verdict |
|---|---|---|
| `tools/chart_ingest.py`, PokerCoaching 100bb + RangeConverter 200bb into the §8.2 envelope, `ChartRounded` | 3, 4 | covered (acquisition risk: **M3**) |
| §8.1 `PreflopSource`/`PreflopNodeKey`/`PreflopStep`/`PreflopNode`/`PreflopStore::query` | 1, 2, 5, 6 | covered, names verbatim |
| §8.2 envelope, hash, 64 MiB bounds, quarantine, sibling-sum rule | 1, 2 | covered |
| §8.3 prefix reconstruction, depth bucketing, asymmetry, rake, straddle, short-handed, size matching | 5 | covered (**M4**, **M6**, **M7**) |
| §8.3 four EV reference variants + fold cross-check + unit scaling | 6 | covered, exact |
| §8.4 likelihood interpolation, branch posterior kernel, cap/residual, node translation, legality after mapping | 7, 8, 9, 13 | covered, exact |
| §9.1 replay/snapshot types | 8, 10, 11 | covered, field-for-field |
| §9.2 start, per-action transaction, snapshot compatibility/selection, registration, blocking, log reach, zero support | 10, 11, 12, 15 | covered (**B1**) |
| §9.3 missing preflop node freeze, uncovered path, no compatible snapshot | 10, 12, 13 | covered |
| §5 step 6 preflop decision path; §6 preflop coverage rows | 14 | covered (**B2**) |
| §5 step 7 turn/river root ranges + `register_snapshot` | 15 | covered (**B1**, **B2**) |
| §12 error rows in scope (quarantine, missing node, zero support, stale identity, mutation invalidation) | 2, 10–15 | covered |
| §13.1 core-preflop rows (7) and core-replay rows (9) | 1–13, 16 | all 16 present by exact name |
| §13.3 `replay_weights_golden`, `bet_translation_golden` | 16 | covered, with an independent Python oracle |
| §13.5 chart handoff freeze before Plan 4 | 4 Step 7, 16 Step 6 | covered |

Out of scope and correctly excluded: the `.7z` PokerData converter, the V9 purchase/sample,
`fixtures/preflop/pokerdata_sample/*`, the exploit slice, `cache`, flop scheduling, the pre-solver,
bench suites, and the UI.

---

## 3. Spec deviations (dimension 2)

Deviations the plan declares, with a verdict on each:

| # | Deviation | Verdict |
|---|---|---|
| D1 | §13.1 T3 says the branch posterior is `(0.9, 0.1)` "for every seat including hero"; §8.4 gives `pi_{S,0}[c] == 1` with one branch. Task 8 follows §8.4 and asserts the actor's normalized combo distribution `(0.9, 0.1)` instead. | **Justified.** The two spec statements are literally incompatible; §8.4 carries the binding formulas and every other T3 number (`M = .5/.4/.5`, `q = .1`, masses `(9,1)`, `log = ln(.18)`) is reproduced exactly. Keep, and raise a spec erratum for §13.1's T3 wording. |
| D2 | §9.2 mandates pointwise board blocking at each root; §8.4 asserts equal per-seat mass totals across branches. Task 10 keeps literal blocking and drops the equal-total guarantee at a root. | **Justified and correctly scoped.** The plan does not invent a rebalancing rule and still enforces the invariant preflop, before removal. Needs a spec decision, not a plan change. |
| D3 | §9.2 case 3 asserts "every observed action is a tree action"; Task 12 Step 4 handles a later off-menu wager at an *unexported* node by freezing navigation with `WalkPath.ordinal = None`. | **Justified.** §9.2 case 2 and the plan's own root-only fixture both produce that state; the fallback is the §9.3 bounded one (retain conditioning, disclose, invent nothing). |
| D4 | §9.2 snapshot compatibility omits `tree_signature`; Task 11's `compatible()` therefore excludes it although §9.1 puts it in `SnapshotKey`. | **Correct reading of the spec**, and the plan says so. No change. |
| D5 | Task 14 Step 3 adds `Send + Sync` supertraits to §8.1's `PreflopSource`. | **Justified** (the store is read from the fast-path thread); method signatures are unchanged and `dyn PreflopSource` inherits the auto traits. File list must be corrected (**M8**). |
| D6 | §4.4 defines no headline string field; `set_headline` returns `Option<&'static str>` rendered through Plan 2's existing note/headline representation, and nothing is added to `proto`. | **Justified**, and the three label strings match §4.4 verbatim. |
| D7 | §4.4 `AsymmetricStacks` has only `stacks_bb`; prominence is computed in a helper and disclosed in assumption notes. | **Justified**; §8.3 requires prominence but §4.4 has no field for it. |
| D8 | `ReplayInput` has no `model_revision`, so the engine supplies the `for_identity`-filtered slice; baseline direct callers use `model_revision = 0`. | **Justified** and documented as a precondition on `replay`. |
| D9 | §7's `<= 0.15 s` / `<= 0.05 s` budgets are restated as "measurement targets, not new timing guarantees" and Task 15 Step 6 reports measured durations rather than asserting wall-clock. | **Justified** (CI flakiness), and the plan still requires the measurement. |
| **D10** | `BundleInfo` gains `rake: Option<RakeProfile>` beyond §8.1/§8.2. | Justified but **undeclared** — see **N10**. |
| **D11** | `fixtures/preflop/synthetic_v2/*.json` are script-generated, not "hand-written" per §13.0. | Justified but **undeclared** — see **N9**. |

Undeclared *behavioural* deviations: none found. Every formula the plan copies matches the spec —
`f_A = (B-s)(1+A)/((B-A)(1+s))`; `M_k = sum_c w*P / sum_c w`; `w *= P/M`; `q *= M`;
`q_{kX} = q_k f_X M_{kX}`; `r_S[c] = sum_k q_k w_{S,k}[c]`; `pi = q w / r`;
`q_R' = q_R + sum q_k` with the `q`-weighted mass average; `ln(m_S)` added (not subtracted);
`depth = min(actor, max(others))/unit`; prominence `> 0.05` (depth) and `> 0.10` (translation);
`ev_inc_sb` for all four references; `ev_chips = ev_inc_sb * 0.5 * unit`;
size tolerance `0.5 * chip / unit`; `s = (to - own - call)/(pot + call)`.

---

## 4. Placeholders (dimension 3)

No banned tokens (`TBD`, `TODO`, "similar to Task N", "add validation", "handle edge cases", `…`)
appear in the plan; the Self-review's placeholder scan is accurate on that narrow reading.
The substantive placeholder problem is **M1** (thirteen signatures with prose-only bodies), plus the
three narrower ones: **M2** (chart manifest writer), **N11** (duplicate rejection), **N16b** (second
golden oracle line). Two more steps describe assertions without code but supply every number, which is
acceptable: Task 8 Step 5's "Repeat T6 with rescale after each action" (all six figures given) and
Task 2 Step 1's extension list (`1e400`, `[-0.1,1.1]`, hash mismatch, 64 MiB+1).

---

## 5. Correctness of the code shown (dimension 4)

Compile- and borrow-checked by inspection; the following hold up:
`cap_branches`' disjoint-field borrow of `r.q` beside `r.seats.iter_mut()`; the `b = f(&b)`
reassignment pattern in Tasks 8/9; `publish`'s edition-2021 disjoint closure capture of
`output.branches` while assigning `output.ranges`; `Seat(i)` forcing `0..6` to `Range<u8>` in one
closure and `usize` in the other; `mask.0` being `Copy` inside the nested `zip`;
`std::array::from_fn::<f32, 1326>`; the `&(…)` tuple destructuring and `(*actor).into()` in Task 11's
snapshot builder; `let … else` (stable, 1.95 pinned); `evs.iter().flatten().flatten()` over
`Vec<Vec<Option<f32>>>`; integration tests reaching `core_preflop` through core-replay's
`[dependencies]`.

Real defects beyond those already filed:

- **C1 (folded into M4/M7).** `physical_positions`' `[Utg,Hj,Co][6-n+i-3]` is correct for n = 3..6
  (verified against §4.3's "last `n-3` names"), and `short_handed_prefix` is its exact complement.
- **C2 (MINOR, folded into N16).** `bucket`'s `.filter(|&d| d<=200)` is a no-op for the acquired set.
- **C3 (MINOR).** Task 7's `if let Some(&(i,x)) = sizes.iter().find(…)` binds `x` unused →
  `unused_variables` warning. Use `Some(&(i,_))`.
- **C4 (MINOR).** Task 1's `valid_step(&a.step, a.to_bb_x1000.unwrap_or(0))` accepts `Some(0)` on a
  non-raise action as equivalent to `None`; harmless but inconsistent with "Non-raise actions carrying
  a nonzero amount are invalid".
- **C5 (MINOR).** Task 11's `select_snapshot(all, key, history)` requires the caller to build a
  `SnapshotKey` including a `tree_signature` that replay does not possess; `compatible()` ignores it,
  so any string works, but the API invites a wrong call. Consider a `CompatKey` without it.

Tests that cannot pass as written: none, apart from **M2** (Task 4 Step 8's `load_bundle` on a chart
manifest missing `game`/`version`/`source_blinds`/`ev_unit`) and the vacuous asserts of **M6**
(which pass but test nothing).

---

## 6. Task ordering (dimension 5)

`cargo test --workspace` stays green after every task, and no task depends on a later one. Checked:
Task 1 (crate + validator) → Task 2 (fixtures + loader; `include_bytes!` of `nodes.json` makes Step 2's
"red" a missing-file compile error rather than a missing symbol, which the plan tolerates) → Tasks 3/4
(Python + fixtures, no Rust regression until Task 4 Step 8) → Tasks 5–7 (core-preflop) → Task 8 (adds
core-replay to the workspace, re-exporting the shared branch types **downward** so no cycle forms) →
Tasks 9–13 → Tasks 14–15 (engine) → Task 16 (goldens). Task 10 deliberately ships a *complete tested*
no-snapshot fallback so that Task 11/12 can enrich it without a red intermediate — a good call.
The only ordering caveat is **M3**: Task 4 becomes a hard stop if the 200bb source stays unavailable.
Task 8's Files line *"modify workspace manifests/lock and preflop exports"* should name
`Cargo.toml`/`Cargo.lock` and `crates/core-preflop/src/lib.rs` explicitly.

---

## 7. Interfaces (dimension 6)

Every §3.5 / §8.1 / §9.1 name the plan consumes or produces matches the spec character-for-character:
`begin_hand`, `apply_action`, `set_board`, `derive`, `street_root`, `replay_root`, `settle_pots`;
`expand_169`, `block_public`, `hero_conditioned`, `mass`, `hash_scaled`;
`PreflopSource::{bundle_info,lookup}`, `PreflopNodeKey`, `PreflopStep`, `PreflopNode`, `PreflopStore`,
`PreflopStore::query`; `SnapshotKey`, `SnapshotProvenance`, `StreetSnapshot`, `SeatMass`,
`HistoryBranch`, `ReplayInput`, `ReplayOutput`, `replay`; `Engine::register_snapshot`;
`build_effective_tree`; `proto::worker::{NodeStrategy, StreetSolution, validate_solution}`.
Shape declarations are consistent end to end and match the spec: envelope action-major `[actions][169]`,
`PreflopNode` class-major `[169][actions]`, `ExpandedNode`/`NodeStrategy` combo-major `[1326][actions]`;
money `u32` chips, `cap_mchips` thousandths, branch arithmetic `f64`, envelope probabilities `f32`.
Plan-3-owned helper types (`PreflopAnswer`, `Interpolation`, `MappedAction`, `MappedAdvice`,
`ExpandedNode`, `BranchNode`, `MixedNode`, `WalkPath`) are labelled as such and are not aliases for
spec names.

The interface failures are entirely on the Plan 2 boundary — **B1** (`SnapshotStore`/`SolvedStreet`),
**B2** (`RangeSource`/`serve_request`/`Paths`), **M8** (Files lists) — plus **N1**
(`core_replay::*` re-export path) and two smaller mismatches: Plan 2's `Engine::new` and `recommend`
return `Result<_, EngineError>`, and Plan 1's `StreetRootSnapshot` carries an extra `bb_chips: u32`
field that Task 11's fixture builder does not construct (it builds `EffectiveTree`, not
`StreetRootSnapshot`, so this is informational only).

---

## 8. Numeric verification of the §13.1 / §13.3 targets

All recomputed independently during this review; all correct.

| Test | Checked | Result |
|---|---|---|
| `pokerdata_units_source_scaling` (T1) | `1.84 SB → 0.92 bb` at 1/2 and 2/5; `ev_chips(1.84,10)/5 = 1.84` (straddle `S`); net-start `1.84+1 = 2.84`; fold `-1` passes, `-0.5` fails; absolute `203-(200-2) = 5`; `unverified → None` | exact |
| `replay_bayes_two_combos` (T3) | `M = .5, .4, .5`; `q = .1`; masses `(9,1)` pre-rescale; `logs = ln(.8)+ln(.25)+ln(.9) = ln(.18)`; marginal `(1, 1/9)` → `(.9,.1)` | exact |
| `replay_off_tree_pseudo_harmonic` | `f_A = 81/173 = .468208`, `d = .23`; `M = .6/.3`; `q = .280925/.159538`; masses `(1.5,.5)/(1/3,5/3)`; marginal `(.474566,.406358)`; `pi_A = (.888,.346)`; then `M' = .625/.7`, `q = .17558/.11168`, marginal `(.221329,.353179)`, output `(.6266,1)`, `log = ln(.353179)`, `pi_A = (.952,.398)` | exact, matches §13.1's rounded figures |
| `replay_cross_actor_branches` (T6) | `M = .45/.25`; `q = .27/.10`; masses `(1.7778,.2222)/(.4,1.6)`; marginal `(.52,.22)`; hero posterior `.27/.37`; after `.9/.1`: `q = .243/.010`, marginal `(.436,.070)`, hero `.9605/.0395`, hero masses untouched | exact |
| `replay_branch_cap_residual` | `.3^3=.027`, `.3^2·.2=.018×3`, `.3·.2^2=.012×3`, `.2^3=.008`, total `.125`; live `.081`, residual `.044`, share `35.2%`; on-menu `.5` → live `.0405`, share `100·.044/.0845 = 52.071%` (52.1); 4th split → `.00405`, `.0027×4`, `.0018×3`, keeps `.00405` + three earliest `.0027`, residual `+.0081 = .0521` | exact; the descending-`q`, ascending-`id` tie-break in `cap_branches` reproduces §8.4's "earlier-created branch is kept" |
| `replay_incomplete_branch_ev` (T7) | `freq = .2·.5 = .1`, `BranchSupportIncomplete{.2}`; complete case `.2·10 + .8·(-2) = .4`; residual case covered `= .19+.76 = .95` | exact |
| `bet_translation_golden` | `below .2→[1,0] d=.3`; `between .73→[81/173,92/173] d=.23`; `above_no_jam 1.5/[.5,1]→[0,1] d=.5`; `above_with_jam 1.5/[1,2]→[.4,.6] d=.5`; `single .73/[.5]→[1] d=.23`; `equal .73/[.5,.5]→[1,0] d=.23`; prominence `d > .10`, `d = .10` not prominent | exact |
| `replay_weights_golden` oracle | `combo_class`: grid index `12 - rank`, `high*13+low` suited, `low*13+high` offsuit → `AA=0`, `AKs=1`, `AKo=13`, multiplicities 6/4/12; combo enumeration `hi*(hi-1)/2 + lo`; `Kh7d2c = {46,21,0}` from `rank*4+suit` with `K=11,7=5,2=0` and `h=2,d=1,c=0`; three action-rescales plus one block-rescale per seat, matching §8.4's convention | exact |

One presentational note on the golden: `replay_golden_expected()` uses `math.fsum` for `M` while the
Rust kernel uses a naive `f64` sum over 1326 terms; the divergence is ~1e-15 relative, well inside the
stated 1e-6 (ranges) / 1e-10 (`q`, `log_reach`) tolerances.

---

## 9. YAGNI (dimension 7)

Nothing meaningful beyond scope or beyond phase 1. Checked and cleared:
`Engine::preflop_store` (the brief lists it as Plan 4's consumption point);
`tools/gen_preflop_fixtures.py` and the six `synthetic_v2` files (§13.0 fixture, V9-independent);
`docs/data/chart-transcription.md` (the §8.2 acquisition record and the §13.5 freeze);
`snapshot_root` (required to reconstruct pot fractions for §8.4 translation at a mapped parent, not
speculative); the `parent` / `split_by` fields on `HistoryBranch` (both are §9.1 fields);
`Send + Sync` on `PreflopSource` (needed by the fast-path thread).
No cache, no flop scheduling, no pre-solver, no bench suite, no UI, no `.7z` converter, no exploit
work, no V9 dependency in any baseline path. The plan's closing out-of-scope paragraph is accurate.

---

## 10. Recommended order of edits before execution

1. **B1** and **B2** — reconcile the engine boundary with Plan 2 (`SnapshotStore`/`SolvedStreet`,
   `RangeSource`, `serve_request`'s preflop arm, `Paths`), then **M8** (Files lists) falls out of it.
2. **M2** and **M3** — chart manifest writer and the 200bb contingency; these gate Task 4's own
   `cargo test --workspace`.
3. **M5**, **M4**, **M7** — the three unit/indexing traps that silently produce wrong numbers.
4. **M1** — add bodies for `replay`, `walk_preflop`, `walk_postflop`, `covered_prefix`, `invalidate`,
   `legalize`, `split_action`.
5. **M6** and the MINOR test-assertion gaps (**N2**, **N3**, **N4**).
6. The remaining MINORs, and add **D10**/**D11** to the deviations section.
