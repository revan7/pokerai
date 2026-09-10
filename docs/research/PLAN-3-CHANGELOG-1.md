# Plan 3 revision 1 — changelog

Plan: `docs/superpowers/plans/2026-09-10-plan-3-preflop-replay.md`
Inputs: `docs/research/REVIEW-of-plan-3.md` (2 BLOCKER, 8 MAJOR, 16 MINOR), `docs/research/REVIEW-cross-plan.md` §§1, 2, 4, 5, spec revision 6, plans 1 and 2, `writing-plans` SKILL.

**Result: 1,947 lines / 16 tasks -> 3,172 lines / 19 tasks.** Every BLOCKER and MAJOR applied; every MINOR applied; every cross-plan resolution naming plan 3 applied. Nothing was declined.

---

## 1. Findings from REVIEW-of-plan-3.md

### BLOCKER

| ID | Status | What changed |
|---|---|---|
| B1 — second, conflicting `SnapshotStore` | **APPLIED** | New "Interface ownership" seam 1 states that Task 14 *deletes* `engine::snapshots::{SolvedStreet, SnapshotStore}` and makes that module `pub use core_replay::{SnapshotKey, SnapshotProvenance, SnapshotStore, StreetSnapshot};`. `SolvedStreet`'s field-by-field mapping into `StreetSnapshot` is spelled out (`board`->`key.root_board`, `ordinal_paths`->`covered_paths`). Task 14's **Files** now include `crates/engine/src/{snapshots.rs,core.rs,serve.rs,engine.rs}` + `Cargo.toml`/`Cargo.lock`; a new Task 14 Step 5 performs the swap in the same commit (per cross-plan §4's green-workspace note), retyping `EngineCore.snapshots`, rebuilding the `serve.rs` register site as a `StreetSnapshot` literal, and rewiring `engine.rs`'s three `invalidate_hand` sites. `SnapshotStore` keeps `for_hand`/`invalidate_hand` so Plan 2's `identity_race_golden` stays green. |
| B2 — engine integration targets a nonexistent file and an unusable signature | **APPLIED** | `crates/engine/src/postflop.rs` removed everywhere (file-structure table, Tasks 17/18 **Files**). Seam 2: Task 18 now produces `ReplayRanges` implementing `RangeSource::ranges_at_root(&self, &HandState, &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>` with a full body, installed into `EngineCore.range_source`; `prepare_root` is deleted with an explicit paragraph on why a parallel entry point cannot be reached from `serve_request`. Seam 3: Task 17 Step 3 quotes Plan 2's exact `Classification::Preflop` arm and its one-line replacement. `Paths { worker, preflop, cache, log }` is Plan 2's struct — plan 3 *populates* `preflop`. `Engine::new`/`recommend`/`set_config` corrected to their `Result` forms and `shutdown(&mut self)`. |

### MAJOR

| ID | Status | What changed |
|---|---|---|
| M1 — 13 prose-only `Produces` signatures | **APPLIED (all 15, exceeding the review's list of 7)** | Rust bodies added for `PreflopStore::open`, `load_bundle`, `build_node_map`, `PreflopStore::query`, `expand_node`, `destination_map`/`legalize_row`, `split_action`, `replay`, `walk_preflop`, `apply_preflop_action`, `query_translated`, `covered_prefix`, `SnapshotStore::invalidate`, `walk_postflop`, `mix_nodes`, `preflop_final`/`serve_preflop`/`headline_source`, `Engine::register_snapshot`, `ReplayRanges::ranges_at_root`, plus `check_straddle`, `rake_reason`, `rank_key`, `observed_history`, `start_stack`, `prominent_depth`, `block_and_rescale`, `board_mask`, `CompatKey`. Per orchestrator decision 1, every remaining `Produces` entry is now an exact Rust signature whose code lives in the same task. |
| M2 — chart manifest writer missing | **APPLIED** | Task 3 Step 4 gains `BUNDLE_INFO_FIELDS` and a full `manifest_for_chart(envelope, transcription) -> dict` enumerating all fifteen `BundleInfo` fields, refusing `rake: null` without `rake_profile == "undocumented"`, plus a Python assertion that the emitted keys equal the Rust field set. |
| M3 — no 200bb contingency | **APPLIED (orchestrator decision 2)** | New Task 4 owns acquisition and emits `fixtures/charts/sources.manifest.json` with per-depth `status: available \| unsupported`. Step 3 is an explicit fallback chain: article page -> download page -> observed PDF link, each retried once at 60 s, then one web-archive copy validated by `%PDF-` magic, title and depth. On total failure the depth row records the exact token `depth 200 unsupported` plus the attempt log. Tasks 5/6/7/17 gate on that file (`pytest.mark.skipif`, a Rust skip loop, "every available depth"), so the workspace stays green either way, and `test_unavailable_depth_ships_no_bundle` forbids an empty stand-in bundle. Per-grid commits added to Tasks 5 and 6 Steps 4–5. |
| M4 — key built from live config instead of candidate | **APPLIED** | Task 8 Step 4's `query` body builds one `PreflopNodeKey` **per candidate**, taking `depth_bb` from `bucket(actual, candidate.depths)` and `rake_profile`/`straddle` from `candidate.bundle_info()`, with a paragraph explaining that the live rake/straddle appear only in the emitted reasons. |
| M5 — source posts written `.5/1` | **APPLIED** | Replaced with "the source posts, 1 SB and 2 SB", with the arithmetic consequence stated. |
| M6 — tautological asserts, `asymmetric()` unexercised | **APPLIED** | The two literal asserts become `assert_eq!(asymmetric(&[100.,104.],100),(true,false))` / `(&[100.,110.],100),(true,true)`; the in-loop recomputation becomes `prominent_depth(actual,used)`; a `query`-level assertion on `PreflopAnswer.reasons` containing `AsymmetricStacks{prominent:false}` and never `Exact` was added. |
| M7 — `stacks_start` vs `Seat.0` indexing | **APPLIED** | `start_stack(state, seat) -> u32` added with the reason in a doc comment and used throughout the depth/eligibility path. The backwards hedge in old Task 15 Step 3 is replaced with "Plan 1 marks an undealt seat `folded = true` (S6), so filtering `state.dealt` first is required". |
| M8 — Files lists omit changed files | **APPLIED** | Tasks 14, 17 and 18 **Files** rewritten against Plan 2's real module list; the file-structure table gains rows for `ranges.rs`, `snapshots.rs`, `serve.rs`, `core.rs`, `engine.rs` and `sources.manifest.json`. `Send + Sync` moved to Task 1 where `PreflopSource` is defined, removing the cross-task file surprise. |

### MINOR

| ID | Status | What changed |
|---|---|---|
| N1 | APPLIED | Task 11 Step 1 shows `core-replay/src/lib.rs` with `pub use branches::*;` and the commented `snapshot`/`preflop`/`postflop` lines later tasks uncomment. |
| N2 | APPLIED | The T3 inserted-size assertion moved into Task 15's `replay_snapshot_prefix_reuse` as a fourth export variant (inserted `Bet{to:73}` with solved probability `(.4,.2)`) plus a negative assertion that forcing 1 differs; recorded in Task 19's audit table. |
| N3 | APPLIED | `straddle_mapping_labels` gains short-post, five-handed and stack-too-small cases through `check_straddle`, and a `serde_json::from_str::<HandConfig>` assertion that a re-straddle cannot deserialize. |
| N4 | APPLIED | `rake_profile_ordering` must now drive `PreflopStore::query` and assert the `5% cap 0.5bb` bundle is *selected* with `RakeProfileMapped{used:"5% cap 0.5bb"}`. |
| N5 | APPLIED | `expand_column`/`expand_optional` take a cached `ComboClasses` table and an action index, doing the column extraction from `[169][actions]`; `ComboClasses::build()` runs the 169 indicator expansions once at load. |
| N6 | APPLIED | `legalize` split into `destination_map(actions, legal) -> Result<(Vec<Action>, Vec<Destination>), UnsupportedReason>` (once per node) and `legalize_row(...)` (per combo), both with bodies. |
| N7 | APPLIED | `make_node`'s unused `committed` parameter dropped, with a comment saying the Rust loader derives it. |
| N8 | APPLIED | `range.json` realigned to class 14 = `KK` (matching `evs[1][14] = 2.31`), and the file is declared a provider-shaped illustration whose one asserted invariant with `nodes.json` is the sparse zero-weight EV. |
| N9 | APPLIED | Script-generated `synthetic_v2` fixtures declared as deviation **D2** in the new Self-review deviations table. |
| N10 | APPLIED | `BundleInfo.rake` (plus `depth_bb`, `game`, `version`) declared as deviation **D1**, with the note that V9's `tools/pokerdata_convert.py` must emit them. |
| N11 | APPLIED | Two `BTreeSet` inserts added to the shown `validate` body (duplicate action kind/amount, duplicate unreachable index). |
| N12 | APPLIED | Imports changed to `from chart_ingest import ...` (Plan 1's conftest puts `tools/`, not the repo root, on `sys.path`), and all fixture paths resolve from `Path(__file__).resolve().parents[2]`. |
| N13 | APPLIED | `if not n["actions"]: raise ValueError("empty menu")` added to the Python validator. |
| N14 | APPLIED | Task 3's Produces now documents the inventory row as `{title, status, history, page, reason}`. |
| N15 | APPLIED | Task 17 **Files** add `pub mod preflop;` to `lib.rs` and `[dev-dependencies] engine = { path = ".", features = ["testing"] }`; test commands use `--features testing`. |
| N16a | APPLIED | `bucket`'s `d <= 200` filter documented as valid only while 200 is an acquired depth, with the condition for a future bundle set. |
| N16b | APPLIED | Task 19 Step 3 gains `replay_golden_offmenu_expected()`, a complete second Python oracle for the off-menu line, plus the runner's assertions (both `f`, both `q`, BTN's 1326 marginal, three named per-combo posteriors, BB's combo-independent posterior). |
| C3 | APPLIED | `Some(&(i,x))` -> `Some(&(i,_))`. |
| C4 | APPLIED | `valid_step(step, amount: Option<u32>)` now rejects `Some(0)` on a non-raise; history tuples map `0 -> None` at the call site. |
| C5 | APPLIED | `select_snapshot` takes a `CompatKey` (six compared fields, no `tree_signature`) with `CompatKey::of(&SnapshotKey)`; tests updated. |
| C1, C2 | APPLIED (folded) | C1 into M4/M7; C2 into N16a. |

---

## 2. Cross-plan items naming plan 3

| Item | Status | What changed |
|---|---|---|
| §1 M13 — `engine::Paths` layout | APPLIED | Plan 3 no longer "adds a chart directory"; Plan 2 declares `Paths { worker, preflop, cache, log }` and Task 17 populates `preflop`. |
| §1 M15 / §2 D1 — `SnapshotStore` and snapshot record | APPLIED | See B1. `core_replay::StreetSnapshot` with `covered_paths` and `invalidate(&HandState)` is the only survivor; `engine::snapshots` re-exports it. |
| §1 M16 / §2 D6 — `RangeSource` vs `prepare_root` | APPLIED | See B2. `ReplayRanges: RangeSource` installed in `EngineCore.range_source`; no `postflop.rs`. |
| §1 M21 / §2 D2 — `resolve_chip_path` implemented four times | APPLIED | Plan 3's `resolve_path` deleted; a Global-Constraints line and an interface paragraph require `proto::resolve_chip_path(&tree.materialized, chips)`. Task 18's snapshot builder names it too. |
| §2 D3 — `node_at` collision | APPLIED | Renamed to `snapshot_node_at` throughout Task 15, with the reason recorded in its Produces line. |
| §2 D8 — duplicate headline logic | APPLIED | `set_headline` deleted. Task 17 produces only `headline_source(SourceKind, EvReference) -> HeadlineSource` and calls Plan 2's `assemble::headline`; Step 4 rewritten to verify rather than reimplement, and the Step 1 test asserts the three mappings plus Plan 2's labels. |
| §4 — P3.T11 must delete the old store inside one task | APPLIED | The whole swap is Task 14 Steps 5–6, one commit, with `identity_race_golden` in the verification list. |
| §5 — P3.T4 too large; one commit per grid | APPLIED (orchestrator decision 3) | Task 4 split into four review-gated tasks: **4** acquisition + availability manifest, **5** PokerCoaching 100bb inventory/transcription/verification, **6** RangeConverter 200bb (or recorded absence), **7** frozen coverage + Rust load. Tasks 5 and 6 commit the inventory once and then once per verified grid. Tasks 5–19 renumbered (old 5–16 -> new 8–19) and all 27 in-text cross-references, the Task 19 audit table and the Self-review task columns updated. |
| §1 M9 / spec S1 — `StreetRootSnapshot.bb_chips` | APPLIED (informational) | Plan 3 constructs no `StreetRootSnapshot` literal; `snapshot_root` returns one from `core_model::street_root`. Recorded in the header's revision-6 note. |
| §3 S2 — `RootError` five variants | APPLIED | Task 18 Step 3 lists all five and directs the executor to extend Plan 2's single `coverage::classify` rather than add a second classifier. |
| §3 S6 — `Seat.0` vs `dealt` order | APPLIED | Underpins M7's `start_stack` and the corrected Task 18 hedge. |
| §3 S11 — `AsymmetricStacks.prominent` | APPLIED | The old "compute prominence in a helper and disclose it in a note" workaround is deleted; `query` emits `AsymmetricStacks{stacks_bb, prominent}` directly. Old deviation D7 retired. |
| §3 S12 — T3 wording | APPLIED | Task 11 Step 1 now asserts the spec text as written; Self-review conflict 1 retired. |
| §3 S13 — board blocking | APPLIED (behavioural change) | `block_branches` (per-branch mass zeroing) replaced by `board_mask` + `block_and_rescale`, which blocks each seat's **output marginal**, uses that blocked maximum as the seat's one common rescale factor across all branches, and accumulates `ln(m_S)`. `publish` removes the board pointwise from the published marginal. Task 13 Step 5 adds an explicit test that per-branch mass totals stay equal after blocking. Self-review conflict 2 retired. |
| §3 S14 — uncovered mapped continuation | APPLIED | Task 15 Step 4 reframed from "the spec is false here" to asserting the amended rule; `WalkPath.ordinal = None` freezes that branch for the street while siblings continue. Self-review conflict 3 retired. |
| §2 Or8 / §3 R3 — chart-replay baseline suites | APPLIED as a handoff | Plan 3 still generates no bench artifacts (correct per the cross-plan reassignment to Plan 4 T17). Task 7 Step 3 adds the explicit Plan 4 handoff paragraph naming shipped depths, covered prefixes, and — when 200bb is unsupported — that §13.5's 200bb baseline spots and §10.5's tier-3 scenarios have no chart range source. |

---

## 3. Other edits made while applying the above

- Header **Spec** line: revision 5 -> revision 6, with a paragraph naming S11–S14, S1 and S2 and what each replaced.
- Global Constraints: new bullet naming the three single-implementation rules (chip path, headline, snapshot record); the commit rule now says "one implementation commit per task" and calls out Tasks 5/6's per-grid commits.
- Self-review rewritten: task columns renumbered; a six-row **declared deviations** table (D1 `BundleInfo.rake`, D2 script-generated fixtures, D3 `CompatKey`, D4 headline string, D5 `ReplayInput` model revision, D6 timing targets) replacing the old prose list; the four "conflicts that cannot be honestly marked resolved" reduced to one (chart inventory verification, open by design).
- Placeholder hygiene: every `…` and `...` removed from the document (6 sites), including inside code comments.
- Structure after revision: 19 tasks, 107 checkbox steps (5–6 per task), 102 balanced code fences, 24 commit trailers, zero banned placeholder tokens.

## 4. Not applied

None. Every finding in `REVIEW-of-plan-3.md` and every cross-plan resolution naming plan 3 was applied.
