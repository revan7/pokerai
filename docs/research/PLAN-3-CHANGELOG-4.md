# Plan 3 changelog 4 — execution errata (revision 3 -> 4)

Date: 2026-09-28

## Preamble

Plan 3 (`docs/superpowers/plans/2026-09-10-plan-3-preflop-replay.md`, revision 3) was executed task-by-task on branch `phase-c` under `.superpowers/sdd/2026-09-10-plan-3-preflop-replay/`: 19 tasks (`core-preflop`, `core-replay`, the engine's preflop/replay seams, the chart-transcription tooling and fixtures), a pre-flight interface scan before Task 1, and a final whole-branch review (`final-review.md`, Opus, base `60f021a`, head `6d1d567`, 65 commits carrying `Task: P3.T<n>`, 51 non-merge). Each task was implemented, then independently reviewed by a different agent against the plan's own task text. As with plans 1 and 2 (`docs/research/PLAN-1-CHANGELOG-4.md`, `docs/research/PLAN-2-CHANGELOG-4.md`), a number of reviews found that the plan's prescribed code — sample implementations, sample tests, Consumes/Produces lists and, in a few cases, prose describing an interface — carried a defect: it matched the brief exactly, but violated the spec, a global constraint, an interface actually shipped by plans 1-2, or a standing ruling made earlier in execution.

Per the orchestrator's ruling recorded during plan 1's execution (`.superpowers/sdd/2026-09-10-plan-1-foundation/progress.md`, "plan-text corrections found during execution are collected into ONE errata changelog per plan at the end of that plan's execution ... instead of a revision bump per finding") and reapplied for plans 2 and 3, this document collects every such plan-3 correction in one place. **The plan's task text is not rewritten.** A reader of the plan's Step blocks still sees the original, defective sample code or prose; the corrected behavior lives only in the committed source, in the per-task review reports, and in this changelog.

Findings that were about the implementer's own added code (not the plan's prescribed text), about report/journal evidence hygiene, or about orchestration process are excluded, following the same rule plans 1 and 2 used. Tasks 3, 4 and 11 are called out explicitly in "Not an erratum" below, since their review findings could otherwise be misread as plan-text defects.

The ledger (`progress.md`) and the final whole-branch review (`final-review.md`) are the source of truth for this document; every ruling id and plan-issue id (`P1`-`P26`, `Q1`-`Q5`) cited below is quoted or paraphrased from one of those two files, and every citation was independently re-verified by reading the cited ledger lines before this document was written (see the agent's report, `.superpowers/sdd/2026-09-10-plan-3-preflop-replay/errata-report.md`, for the verification detail). `final-review.md`'s own "Plan issues (feed PLAN-3-CHANGELOG-4)" section states it carries "the erratum candidates the ledger recorded, plus four new ones (P23-P26)" — this document's task-by-task errata below are organized to cover that same set, cross-referenced by `Pn` id in the dedicated index at the end.

### Recurring patterns across the errata below

Several themes recur across unrelated tasks:

- **Spec text, not the plan's sample code, is authoritative whenever the two disagree** (the project's standing "spec > plan" precedence): the branch-kernel batching rule (Task 13), the branch-cap merge/compaction rule (Task 12), and the `BranchResidual` cause vocabulary (Task 16, ruling 19-I1) each replace a plan sketch that read naturally but produced a result the spec's worked examples (or its "no invented likelihood" principle) forbid.
- **A brief's `panic!`/`unwrap`/`expect`/fixed-type sketch becomes an explicit, typed, recoverable result** once a reviewer finds an input that reaches it in production, not only in the brief's own happy-path test (Task 10's `wager_fraction -> Option<f64>`; Task 14's invalidation sketch; Task 15's panicking fold and walk-ending exclusion).
- **"One implementation" is a real constraint the plan itself states, and several of its own Step blocks violate it** by duplicating logic the plan elsewhere insists lives in one place: Task 17's private `merge_reasons` duplicates plan 2's shipped `assemble::accumulate`; Task 13's per-branch `split_action` calls duplicate work a single per-generation batch call already does correctly.
- **A plan-3 Consumes/Produces list, module-list copy or cross-reference to plan 2's shipped code is stale** relative to what plan 2 actually shipped by the time plan 3 executed (Task 13's Consumes list, the Global Constraints module list, Task 18's `classify` framing) — cosmetic, not compile-time, but worth correcting for a future reader.

---

## Errata by task

### Task 1 — Normalized preflop envelope (`decode`, `Envelope`, `BundleInfo`) (P3.T1)

- **Task/step:** Step 4, the envelope matrix and metadata float fields (`EnvelopeNode.weights: Vec<Vec<f32>>`, `evs: Option<Vec<Vec<Option<f32>>>>`, metadata floats), plan's `#[derive(Deserialize)]` sketch.
- **Plan text (paraphrase, pre-flight scan):** plain `#[derive(Deserialize)]` f32 decoding narrows an out-of-range f64 wire token before `validate()`'s `(0.0..=1.0).contains(p)` check ever sees it, so a narrowed value can silently round into range and pass — the same failure mode standing ruling (a) exists to prevent, and the codebase already has the correct pattern twice over (`Range1326`'s hand-written `Deserialize`, `proto::worker`'s `narrow_checked`/`deserialize_matrix`) that the plan's sketch does not reuse.
- **What the code does instead:** aggregate matrices and metadata floats are validated wide (f64) on both serde directions before narrowing to f32, with a checked codec, applied as a pre-review second commit so the task review covered both the implementation and the fix together; review P3T1R then found two further gaps (aggregates checked only after narrowing; metadata floats unvalidated) and two Minor findings, all applied.
- **Why:** implementer-flagged deviation, accepted (`progress.md:103`), plus review P3T1R R1-R4 (`progress.md:105`), all "plan-mandated" — feeds final review **P7** ("envelope matrices and the aggregate rules must be validated wide, before narrowing, on both serde directions, and the metadata floats need checked codecs").
- **Commit(s):** `4b30a86` + `b6dfd29` + `0516edf` (pre-review fix plus one fix round; review clean).

### Task 2 — Independent source loaders and bundle quarantine (`load_bundle`, `open`) (P3.T2)

- **Task/step:** the loader sketch (link containment, declared-actor check, chart-EV rejection, narrowing-before-validation).
- **Plan text (paraphrase):** the brief's loader sketch admits a symlink outside the bundle directory, does not check the declared actor against the node's own actor, does not reject a chart bundle that carries EV, and narrows wire floats before validating them.
- **What the code does instead:** `open`/`load_bundle` enforce link-safe containment, a declared-actor check, chart-EV rejection at load time (with lookup-time clearing), and wide validation before narrowing, plus three further Minor fixes (diagnostics, an N1 boundary check folded from Task 2 into `checked_envelope`).
- **Why:** review P3T2R R1-R8, Important/Minor, plan-mandated — "the brief's loader sketch has the same containment, actor, chart-EV and narrowing defects" (`progress.md:112`). Feeds final review **P8** ("Task 2's loader sketch has four defects: link containment, the declared-actor check, chart EV rejection, and narrowing before validation (T2 R1-R8)").
- **Commit(s):** `e5a6c45` + `ccba731` (review clean after 1 fix round). Open item (not a plan defect): the file-symlink containment case could not be exercised without `SeCreateSymbolicLinkPrivilege` in this environment.

### Tasks 5 and 6 — Chart-grid transcription (raster render, cell-by-cell read, blind second pass) (P3.T5, P3.T6)

- **Task/step:** the plan's "one commit per task" rule versus the transcription tasks' natural per-grid commit cadence; physical PDF page numbering; the required blind second-pass re-read.
- **Plan text (paraphrase):** the plan's global "one commit per task" convention does not fit a 22-grid (Task 5) / 35-grid (Task 6) transcription process, and the brief does not specify physical PDF page numbering for citing a grid's source; an early reading of the blind-second-pass requirement treated a different agent's visual spot-check as sufficient, before being corrected to a full hidden-first-pass re-read of every cell.
- **What the code/process does instead:** transcription proceeds by grid with per-grid commits recorded in the task's own documentation trail; each grid's source page is cited by its physical PDF page number; the blind second pass is a full independent re-transcription (own scratch transcription, reverse row order, then a programmatic diff) — Task 5: 22/22 grids, 3,718 cells, 0 mismatches; Task 6: 35 grids, 5,915 cells, 0 mismatches.
- **Why:** Task 5 completion line, "erratum candidates: one commit per task vs per-grid commits; physical page numbering" (`progress.md:139`); the blind-second-pass correction at `progress.md:137-138` ("the brief requires a hidden-first-pass re-read of all 169 cells of every grid; the Opus verifier was redirected to a full blind re-read"). Feeds final review **P9** ("Tasks 5/6: 'one commit per task' contradicts per-grid commits. Physical PDF page numbering is also needed, and the second pass must be a blind re-read of every cell (T5 ruling)").
- **Commit(s):** Task 5: `911f0a8`. Task 6: `742839e` + `7211961`.

### Task 7 — `published_chart_bundles_load` and `nodes_have_no_ev` (P3.T7)

- **Task/step:** Step 1 (test), `assert!(info.nodes_have_no_ev(), ...)`.
- **Plan text (<=25 words, pre-flight scan finding):** the test calls `info.nodes_have_no_ev()` where `info: &BundleInfo`, but `BundleInfo` has no `nodes` field; the task's own prose places the method on the adapters reached through `source: Box<dyn PreflopSource>`.
- **What the code does instead:** the test calls `source.nodes_have_no_ev()` on `source: Box<dyn PreflopSource>`, matching the surrounding prose; review P3T7R additionally ruled the trait method has no default (every implementor is explicit) after finding a default-true implementation unsafe.
- **Why:** pre-flight ruling 2 (`progress.md:89`, "R2 ... T7's own prose places the method on the adapters ... never on `BundleInfo`, which has no `nodes` field at all"); review P3T7R R1 (`progress.md:151`, "default-true `nodes_have_no_ev` is unsafe — ruled no default"). Feeds final review **P2** ("Task 7 (line 971): `info.nodes_have_no_ev()` should be `source.nodes_have_no_ev()`. The trait method has no default and every implementor is explicit (T7-R1)").
- **Commit(s):** `d93de0b` + `94bc0ef` (review clean after 1 fix round).

### Task 8 — PokerData/chart-transcription adapters and straddle/rake rules (P3.T8)

- **Task/step:** Step 1 (test) and Step 4 (`check_straddle`), six occurrences of `proto::RulesError`, plan lines 1030, 1033, 1036, 1206, 1213, 1215.
- **Plan text (<=25 words):** `Result<(), proto::RulesError>` and matching test assertions on `proto::RulesError::FormatUnsupported{..}` at all six sites (plan lines 1030, 1033, 1036, 1206, 1213, 1215) — `proto` has no `error` module; the type is `core_model::RulesError`.
- **What the code does instead:** all six occurrences are `core_model::RulesError`; review P3T8R found five further Important findings (check ordering versus `prefix_state`, zero-depth rejection at admission rather than silent filtering, historical sizes resolved against the selected source menu with exact widened comparison, an unrepresentable raise as a typed `UnsupportedHistory` rather than a panic, the per-invocation mapping cache placement) and one Minor, all fixed in one round.
- **Why:** pre-flight ruling 1 (`progress.md:88`, "replace every `proto::RulesError` with `core_model::RulesError` (6 sites)"); review P3T8R R1-R5 (`progress.md:128`). Feeds final review **P1** ("Task 8 (lines 1030, 1033, 1036, 1206, 1213, 1215): `proto::RulesError` should be `core_model::RulesError` (pre-flight ruling 1)").
- **Commit(s):** `833b43d` + `8414c96` (review clean after 1 fix round).

### Task 9 — Combo-class table, EV normalization, fold verification (P3.T9)

**Erratum 1 — `debug_assert!` instead of an always-on `assert!`**

- **Task/step:** Step 4, `ComboClasses::build()`'s invariant check, plan line 1417.
- **Plan text (<=25 words):** `debug_assert!(class_of.iter().all(|&c| c != u16::MAX));` — an infallible internal constructor's invariant, compiled out in `--release`.
- **What the code does instead:** an always-on `assert!`, per the project's standing ruling (b).
- **Why:** pre-flight ruling 4 (`progress.md:91,98`). Feeds final review **P4** ("Task 9 (line 1417): `debug_assert!` should be an always-on `assert!` (ruling 4)").
- **Commit(s):** `4a6811b` + `8c0384f` + `97bf7b3` (review clean after 2 fix rounds).

**Erratum 2 — narrow-vs-wide fold admission mismatch**

- **Task/step:** the fold-EV verification and expansion boundary check.
- **What the code does instead:** wide (f64) fold admission at load time is the single verdict; expanded nodes carry a `fold_wide_verified` flag and expansion trusts it rather than re-checking with a narrower (f32) predicate that could panic on a loader-accepted bundle (26 of 3,200 last-unit grid cases reproduced the mismatch under the brief's narrow re-check).
- **Why:** review P3T9R R1-R3 plus the re-review's new MAJOR N1 (`progress.md:136,141`, "narrow boundary assert can panic on a loader-accepted bundle: wide residual 0.0009999 vs narrow 0.0010071 ... one wide predicate at load is the single source of truth"). Feeds final review **P10** ("Task 9: the wide load-time fold admission is the single verdict. Nodes carry `fold_wide_verified`, and expansion trusts it (T9 N1)").
- **Commit(s):** `4a6811b` + `8c0384f` + `97bf7b3` (round 2).

### Task 10 — Bet-size interpolation, off-menu mapping, legalization (`interpolate`, `wager_fraction`) (P3.T10)

- **Task/step:** the brief's merge code (EV ownership/provenance) and `wager_fraction`'s signature.
- **Plan text (paraphrase, plan lines around 1478, 1503):** the brief's merge code loses EV ownership and provenance when combining rounded destinations; `wager_fraction` is a plain (panicking on a recoverable mismatch) rather than `Option<f64>`-returning function.
- **What the code does instead:** `wager_fraction -> Option<f64>` and `menu_fractions -> Option<Vec<(usize, f64)>>` (`None` routes to the unmappable-size stop branch); `Destination` gains `source: Action` and `moved: bool`; the merge preserves EV ownership and provenance rather than the brief's lossy combination. Left open at Task 10's own close (carried, not yet fixed there): identical rounded actions should keep an EV only when all sources agree, which needs a rounding flag `expand_node` does not yet supply; no row-sum assertion in `legalize_row` (an f32 re-check hazard).
- **Why:** review P3T10R R1-R6, Important, "plan-mandated, spec 8.4 wins" (`progress.md:150,152,153`). Feeds final review **P11** ("Task 10 (lines 1478, 1503): `wager_fraction -> Option<f64>` and `menu_fractions -> Option<Vec<(usize, f64)>>`. `Destination` gains `source` and `moved`. The brief's merge code lost EV ownership and provenance. Spec 8.4's 'prefer an exactly represented destination' needs a rounding flag that `expand_node` does not provide (T10 R1-R6)").
- **Commit(s):** `daddc6a` + `34cb837` (review clean after 1 fix round).

### Task 12 — Branch-cap merge, compaction, id remap (`cap_branches`) (P3.T12)

- **Task/step:** the brief's sample compaction, parent remap and pairwise merge (`cap_branches`).
- **Plan text (paraphrase):** the brief compacts branch ids and merges overflow branches with a pairwise-averaging sketch that does not guarantee creation-order tie-breaking, can alias an ancestor id after saturation, and can round a representable positive marginal to zero through `q * mass`.
- **What the code does instead, across five fix rounds:** creation-ordered compaction through a wide counter narrowed only once ids are final (ruling 12-R1); a collision-free old-to-new id map built before narrowing, so no ancestor resolves to an unrelated branch (12-R2); the residual merge normalizes weights before multiplying by mass so a representable positive marginal never rounds to zero (12-R3); when a correctly-rounded residual mass would still make `q_R * mass` round to zero relative to a positive pre-cap marginal, the mass is stepped up one representable unit at a time, at most 8 steps, else the cap is rejected (12-N1, discovered by re-review); the pre/post-cap marginal comparison that triggers stepping is made explicit and bounded (12-N1b); the unavoidable-support-loss assert is moved after the stepping so a step-recoverable case is accepted, not rejected (12-N1c); when merged branches carry positive support that the averaged residual mass still rounds to zero after stepping, the residual mass is floored at one representable unit rather than rejected (12-N1d).
- **Why:** review P3T12R, 3 Important, "plan-mandated (the brief's sample carries them; spec/prose wins)" (`progress.md:160`), then rulings 12-R1/R2/R3 (`progress.md:161-163`) and the re-review chain 12-N1/N1b/N1c/N1d (`progress.md:165,169,171,173`, each tied to spec 9.3's no-threshold positive-reach rule). Feeds final review **P12** ("Task 12: the brief's compaction, parent remap and pairwise merge are replaced by creation-ordered compaction, a collision-free remap and the normalized scaled merge with the N1/N1b/N1c/N1d support rules (rulings 12-R1 to 12-N1d)").
- **Commit(s):** `f769c75` + `28436f5` + `e796f7d` + `fcec230` + `61a153a` + `f80cfa5` (review clean after 5 fix rounds; re-review 2 Approved 4/4, 0 new).

### Task 13 — `apply_preflop_action`, off-menu split, source-unit interpolation (P3.T13)

**Erratum 1 — Consumes list omits Task 10's interpolation helpers (cosmetic)**

- **Task/step:** the Consumes list.
- **What the code does instead:** the code already calls `interpolate`, `wager_fraction` and `menu_fractions` directly in `apply_preflop_action`'s off-menu branch; only the Consumes list's text is incomplete.
- **Why:** pre-flight ruling 8, cosmetic, no code change (`progress.md:95,99`). Feeds final review **P6** ("Task 13's Consumes list omits `interpolate`, `wager_fraction` and `menu_fractions` (ruling 8)").
- **Commit(s):** n/a (documentation-only observation; no plan-3 code change).

**Erratum 2 — per-branch `split_action` calls instead of one per-generation batch**

- **Task/step:** plan line 2044.
- **Plan text (<=25 words, paraphrase):** one `split_action` call per branch when several branches split on the same observed action, rather than one call over the whole generation.
- **What the code does instead:** a batch split (`split_batch`) allocates and remaps ids once against the complete pre-action generation; the common-menu `split_action` stays as a thin wrapper for existing single-branch consumers.
- **Why:** ruling 13-pre (`progress.md:167`, "Task 13 expands a generation with ONE split_action call over the whole branch list per observed action ... plan 3 line 2044's per-branch call shape is an erratum"), confirmed and generalized as ruling 13-R3 in the Task 13 review round (`progress.md:186`). Feeds final review **P13** ("Task 13 (line 2044): per-branch `split_action` calls are replaced by one `split_batch` per generation (13-pre, 13-R3)").
- **Commit(s):** `ea18b47` + `92e395b` (review clean after 1 fix round).

**Erratum 3 — expanded-action history recipe instead of carried source-step identity**

- **Task/step:** the walk/query-translated history recipe.
- **What the code does instead:** the selected source-step identity is carried for navigation instead of reconstructed from a rounded chip amount; a chip-only query that cannot recover an edge unambiguously returns an explicit unresolved result, never another edge.
- **Why:** ruling 13-R1 (`progress.md:184`, "spec 8.3 exact matching ... and 8.4's shared mapped history; the brief's expanded-action recipe is the erratum"). Feeds final review **P13**.
- **Commit(s):** `ea18b47` + `92e395b`.

**Erratum 4 — live-post interpolation instead of source-unit interpolation**

- **Task/step:** the interpolation arithmetic for off-menu wagers.
- **What the code does instead:** interpolation reconstructs source posts and step contributions in source units before chip rounding, in one consistent source scale; the physical SB post is not represented in the virtual straddle tree (spec 8.3).
- **Why:** ruling 13-R2 (`progress.md:185`). Feeds final review **P13**.
- **Commit(s):** `ea18b47` + `92e395b`.

**Erratum 5 — `BetTranslation.deviation` wire codec bound to `[0, 1]`**

- **Task/step:** the proto codec for `ApproxReasonJson`/`ApproxReasonBincode`.
- **Plan text (paraphrase):** the deviation field is bounded `[0, 1]`, rejecting a shove-sized deviation above 1, against spec 8.4's unbounded deviation.
- **What the code does instead:** both codecs use `crate::numeric::non_negative` instead; the wire test that required rejecting 1.5 is replaced by round-trip tests at 0, 1 and a shove-sized value above 1, plus rejection of negative/non-finite values.
- **Why:** ruling 13-R4, "supersedes 13-D" (`progress.md:187`); `crates/proto` edits explicitly authorized for this round. Feeds final review **P13** ("The proto deviation codec bound of `[0, 1]` is replaced by `non_negative` (13-R4)").
- **Commit(s):** `ea18b47` + `92e395b`.

### Task 14 — Snapshot invalidation on mutation/undo/end-hand (P3.T14)

**Erratum 1 — invalidation compares the unfiltered street history**

- **Task/step:** rule (4) of the invalidation sketch.
- **Plan text (paraphrase):** rule (4) compares a candidate snapshot's `solved_prefix` against the current unfiltered street history.
- **What the code does instead:** invalidation recovers the admitted **projected** root history at each historical cutoff (via a crate-private `decision_roots`, replaying the truncated hand with `core_model::lifecycle::simulate`) and validates the surviving prefix in that projection, because `solved_prefix` is itself a projected (two-seat) history (spec 10.2); comparing it to the unfiltered history deletes still-valid snapshots.
- **Why:** ruling 14-I1, "overturns the provisional 14-Q2" (`progress.md:198`, "spec 9.1 line 543, 9.2 line 557 ... comparing it to the unfiltered street history deletes still-valid snapshots"). Feeds final review **P14** ("Task 14: the invalidation sketch compares the unfiltered street history; it must recover the projected root at each cutoff (14-I1)").
- **Commit(s):** `a577046` + `aa43fc2` + `9605a66` (review clean after 1 fix round).

**Erratum 2 — forced-`Betting`/`derive` recovery sketch instead of `lifecycle::simulate`**

- **Task/step:** the model-based cutoff-recovery sketch.
- **What the code does instead:** `decision_roots` replays via `core_model::lifecycle::simulate` (hero-gated like Task 15's `snapshot_root`) instead of the plan's forced-`Betting`-phase/`derive` recovery sketch.
- **Why:** ruling 14f-D1, "erratum candidate for plan 3" (`progress.md:199,202`). Feeds final review **P14** ("The forced-Betting/`derive` recovery is replaced by `lifecycle::simulate` (14f-D1)").
- **Commit(s):** `a577046` + `aa43fc2` + `9605a66`.

**Erratum 3 — decision identity should carry the hand's own config revision**

- **Task/step:** plan line 2375 (compare with plan line 2968, Task 18).
- **What the code does instead:** the hand's own `HandConfig.config_revision` is used for the compatibility comparison, not the session `config_revision` counter, consistent with plan-2's final-review ruling F-I4.
- **Why:** carried from plan 2's final review (`progress.md:195`, "(I4) a decision identity carries its hand `HandConfig.config_revision`, not the session counter; Task 14 `for_identity`/`CompatKey` compares by that hand revision and Task 18 line 2968 is the consistent reading (line 2375 is the erratum)"). Feeds final review **P14** ("Line 2375 should use the hand config revision as line 2968 does (plan-2 F-I4)").
- **Commit(s):** `a577046` + `aa43fc2` + `9605a66`.

### Task 15 — Postflop replay walk, snapshot selection, disclosure (P3.T15)

- **Task/step:** the postflop walk sketch (walk-ending exclusion, fold handling, `expect`/`unwrap`), and its disclosure boundary.
- **Plan text (paraphrase, accepted deviations D4-D7):** the brief's walk sketch excludes an unreproducible candidate at the wrong point, uses a fold that panics when it lands on a seat already projected out of the hand, and uses `expect`/`unwrap` in places where an explicit freeze cause belongs.
- **What the code does instead:** the unreproducible candidate is excluded before selection (not mid-walk); a projected-out seat's fold is disclosed rather than walked or panicked on; `expect`/`unwrap` sites become explicit, typed freeze causes. Two further review findings (not in the brief at all) were added: the postflop walk discloses the hero cap residual per spec 8.4 at every walk exit, recomputed from post-action weights (ruling 15-I1); a street that opened multiway without a compatible HU snapshot takes the cause `"multiway prior street"` rather than a generic no-snapshot cause (ruling 15-I2).
- **Why:** implemented deviations D4-D7, "brief sketch erratum candidates: panicking fold, walk-ending exclusion" (`progress.md:206`); review P3T15R (`progress.md:209`, "D1-D10, D12-D15 accepted"); ruling 15-I1 (`progress.md:209`, "the postflop walk discloses the cap residual per spec 8.4 ... at the postflop replay output boundary"); ruling 15-I2 (`progress.md:209`, "a street that opened multiway without a compatible HU snapshot emits `multiway prior street`"). Feeds final review **P15** ("Task 15's sketch has three problems: its walk-ending exclusion of an unreproducible candidate, a panicking fold on a projected-out seat, and `expect`/`unwrap` where explicit freeze causes belong. The postflop walk must disclose the cap residual at its output boundary (15-I1, 15-Q5), and a street that opened multiway takes the `multiway prior street` cause (15-I2)").
- **Commit(s):** `01403f7` + `942c1b2` + `d91679b` (review clean after 1 fix round plus one tiny orchestrator-verified round 2).

### Task 16 — Branch-to-node mixing, `MissingPreflopNode` key selection (`mix_nodes`) (P3.T16)

**Erratum 1 — a single `"missing node <heaviest key>"` cause for every unresolved share**

- **Task/step:** the assembly sketch, plan lines 2673-2676.
- **Plan text (<=25 words, paraphrase):** whenever `unresolved_mass > 0`, emit `BranchResidual{cause: "missing node <key>"}`, naming a present node even when only the cap residual (which has no node by construction) lacks a strategy.
- **What the code does instead (per ruling 19-I1, found during Task 19's review, fixed in the Task 19 fix round since it edits the same `translate.rs` Task 16 wrote):** one `BranchResidual` is emitted per cause among the unresolved branches — the persistent cap-residual share gets `cause: "cap"`; only a genuinely absent node gets `cause: "missing node <key>"` (the heaviest such key).
- **Why:** Ruling 19-I1, "spec > plan" (`progress.md:242-243`, "Spec line 229 ... allows only reasons actually incurred. The plan-3 Task 16 sketch (lines 2673-2676) and core-preflop translate.rs ... emit 'missing node <key>' whenever unresolved_mass > 0, naming a present node when only the cap residual lacks a strategy: erratum for PLAN-3-CHANGELOG-4"). Feeds final review **P16** ("Task 16 (lines 2673-2676): the single `'missing node <heaviest key>'` cause is wrong against spec 8.4. Use one `BranchResidual` per cause (19-I1)").
- **Commit(s):** the cause fix lands in the Task 19 fix round, `3895681` (round 1) on `478b92a`; Task 16's own commits are `be3e93b` + `b011d4e`.

**Erratum 2 — an admitted chart's row probabilities are not normalized once at the mapped-node boundary**

- **Task/step:** the mixing sketch's handling of a source row admitted under spec 8.2's `1 +- 1e-3` tolerance.
- **What the code does instead:** a complete reachable row within the spec 8.2 admission tolerance is normalized exactly once at the mapped-node boundary before mixing (with a documented `1e-5` slack for f32 wire round-trip), so assembly's `[0, 1]` checks always see normalized probabilities; a row outside tolerance is rejected with a diagnostic; an all-zero unreachable row is preserved as-is.
- **Why:** rulings 16-R1 and 16-R1b (`progress.md:179,182`, "spec 8.2 admits the rounding and an admitted chart must never abort a recommendation"). Feeds final review **P16** ("Admitted source rows are normalized once at the mapped-node boundary (16-R1, 16-R1b)").
- **Commit(s):** `be3e93b` + `b011d4e` (review clean after 1 fix round).

**Erratum 3 — the retained-key fallback for `MissingPreflopNode` (ruling 16-R2, now amended)**

- **Task/step:** the all-branches-missing key-selection fallback.
- **What the code does instead (as originally ruled):** when the heaviest positive-posterior branch is the residual, the key falls back to the heaviest known stopped/live key across *all* branches, including zero-posterior ones — which the final whole-branch review found could return a present node's key over an actually-missing key, conflicting with ruling 19-I1's "never a key whose node is present". The final review's **Q2** amends this: the fallback is now restricted to positive-posterior branches of `B+` that retain a key, and a residual-only case returns the label `"cap residual (no node)"` (spec text amended by `SPEC-CHANGELOG-8.md`, spec revision 9).
- **Why:** ruling 16-R2 (`progress.md:180`); amended by final review **Q2** (see `SPEC-CHANGELOG-8.md`). Feeds final review **P16** ("The retained-key fallback is 16-R2, and Q2 asks whether it should stand" — resolved: amended).
- **Commit(s):** `be3e93b` + `b011d4e` (16-R2 as originally ruled); the Q2 amendment is a spec/code-level correction tracked by the final review's own fix round, outside this documentation task's scope.

### Task 17 — Preflop decision assembly, quarantine, hero lookup (`preflop_final`) (P3.T17)

**Erratum 1 — `base.actions_legal()` does not exist**

- **Task/step:** Step 3, `preflop_final`, plan lines 2783-2844.
- **Plan text (<=25 words):** `legalize_expanded(e, &base.actions_legal())` where `base: Recommendation` — no such method exists on `Recommendation` in the shipped `proto` crate or plan 2's text.
- **What the code does instead:** `legalize_expanded(e, &base.legal)`, using the `Recommendation.legal: Vec<LegalAction>` field plan 2's `assemble::fast`/`assemble::unsupported` already populate.
- **Why:** pre-flight rubric defect R3 (BLOCKER) and ruling 3 (`progress.md:75,90`). Feeds final review **P3** ("Task 17 (lines 2783-2844): `base.actions_legal()` should be `base.legal`, and `merge_reasons` should be `assemble::accumulate` (rulings 3 and 5)").
- **Commit(s):** `481ba8b` + `b04738a` + `2d466b9` (review clean after 1 fix round plus one tiny round 2).

**Erratum 2 — a private `merge_reasons` duplicates plan 2's shipped `assemble::accumulate`**

- **Task/step:** the reason-accumulation helper, contradicting Task 17's own Consumes list (which names `assemble::accumulate` as consumed and real).
- **Plan text (paraphrase):** a private `merge_reasons(&mut Coverage, Vec<ApproxReason>)` reimplements the identical three-way `Coverage` merge semantics plan 2's `assemble::accumulate` already provides.
- **What the code does instead:** `preflop_final`/`assign_mix` call `assemble::accumulate` directly at each of `merge_reasons`'s three call sites; `merge_reasons` is deleted.
- **Why:** pre-flight rubric defect R5 (MAJOR — verbatim duplicated logic) and ruling 5, option (b) (`progress.md:77,100`, "single implementation of reason accumulation; spec section 2 coverage semantics live in one place"). Feeds final review **P3**.
- **Commit(s):** `481ba8b` + `b04738a` + `2d466b9`.

**Erratum 3 — "mass/log provenance" wording implies a wire field that does not exist**

- **Task/step:** the brief's disclosure wording for `log_reach`.
- **What the code does instead:** `log_reach` is not surfaced as a new `Recommendation`/`Assumptions` field; the Task 19 replay golden covers it through the replay output directly.
- **Why:** ruling accepted at review P3T17R, "Q3 accepted as the Task 19 replay-golden carry (the brief 'mass/log provenance' wording is an erratum candidate; no new wire field)" (`progress.md:222`). Feeds final review **P17** ("the 'mass/log provenance' wording needs no wire field (17-Q3)").
- **Commit(s):** `481ba8b` + `b04738a` + `2d466b9`.

**Erratum 4 — read-only packaged-chart policy versus spec 8.2 quarantine**

- **Task/step:** plan line 437 ("Packaged fixtures are loaded read-only; quarantine applies to installed copies").
- **Plan text (<=25 words):** a failing packaged chart pair is excluded with a startup banner but never renamed/quarantined, unlike an installed bundle.
- **What the code does instead:** a failing packaged pair is quarantined by the same collision-safe `.bad` rename with a startup banner as an installed bundle; if the rename itself cannot be performed (unwritable path), a clear non-fatal diagnostic reports the unsuccessful quarantine and the pair is still excluded from the merged store.
- **Why:** ruling 17-I1, spec 8.2 (`progress.md:222`, "the read-only 'excluded with a banner and never renamed' policy is withdrawn"); refined by rulings 17-N1/17-N2 in re-review (ordinary files only, never a directory; the pair rename is atomic in effect, rolled back on partial failure). Feeds final review **P17** ("The read-only packaged-chart policy (line 437 ...) is replaced by quarantine of failing packaged pairs (17-I1)").
- **Commit(s):** `481ba8b` + `b04738a` + `2d466b9`.

**Erratum 5 — the selected `MissingPreflopNode` key is not surfaced in `Assumptions`**

- **Task/step:** Step 3's assembly of the `Unsupported{MissingPreflopNode{key}}` outcome.
- **What the code does instead:** a note carrying the exact selected key is added to `Assumptions` whenever this outcome is assembled (key-selection rule unchanged; inherited reasons kept).
- **Why:** ruling 17-I2, spec 12 (`progress.md:222`). Feeds final review **P17** ("The missing-node key must also appear in the assumptions (17-I2, spec 12)").
- **Commit(s):** `481ba8b` + `b04738a` + `2d466b9`.

**Erratum 6 — hero's own lookup should go through the replay walk, not a standalone chip-amount query**

- **Task/step:** how Task 17 obtains hero's decision-node lookup.
- **What the code does instead:** hero's lookups go through `replay`/`walk_preflop` (which expose the chosen source step or key per branch), never through a standalone `query_translated` on chip amounts, which would report an unresolved node where the walk actually resolved one.
- **Why:** ruling 17-pre, set at Task 13's close for Task 17's dispatch (`progress.md:190`, "Task 17 obtains hero's decision-node lookups through the replay walk ... never through a standalone `query_translated` on chip amounts"). Feeds final review **P17** ("Hero lookups go through the replay walk (17-pre)").
- **Commit(s):** `481ba8b` + `b04738a` + `2d466b9`.

### Task 18 — Engine replay bridge: range source, registration, misses (`ReplayRanges`) (P3.T18)

- **Task/step:** the register-site sketch and the miss/provenance channel to the `Final`.
- **Plan text (paraphrase, plan line ~2945):** the register site uses `input.tree` (the tree the request was built to solve) rather than the tree actually solved (relevant when the `_min` retry template answers instead); the sketch does not require registration to stay inside the accepted-claim critical section without releasing the identity lock between the `is_active` check and the registration itself; the sketch names no channel by which selected-snapshot provenance reaches the turn/river `Final`.
- **What the code does instead:** the register site uses `out.tree` (the tree actually solved) (ruling 18-D6); registration happens only inside the accepted claim, under the identity lock, via `register_accepted`, with no lock release between the active-identity check and the registration (ruling 28-I2, carried from plan 2); `RootRanges::snapshots_used: Vec<(Street, SnapshotProvenance)>` is added to plan 2's `RootRanges` seam, and `replay_bridge::snapshot_note` appends one note per selected snapshot to `Assumptions.notes` before the fallback refresh (ruling 18-I3, overturning the provisional 18-Q1 plan-4-deferral reading). Two further Important review findings, also fixed in the same round: a registered snapshot must carry the *full* coverage reasons of the solve that produced it, including `DeadlineBestSoFar` (ruling 18-I1, overturning provisional 18-D11); delivered-outcome miss recording extends to `retire_unserved` and `contain_panic`, not only the served/watchdog-won/stale-after-fire exits (ruling 18-I2, overturning provisional 18-Q5).
- **Why:** review P3T18R (`progress.md:230,232`, "(18-I1) FIX ... (18-I2) FIX ... (18-I3) FIX ... spec 9.3 line 565 binds and Task 18 is the consumer"). Feeds final review **P18** ("Task 18 (line 2945): the register site must use the tree actually solved (`out.tree`), not `input.tree`. Registration must happen inside the accepted claim, without releasing the identity lock between the `is_active` check and the registration. Provenance reaches the `Final` through `RootRanges::snapshots_used` (18-I3). Snapshots carry the full coverage reasons (18-I1). Misses are recorded at every delivered exit (18-I2). `ReplayInput.missing` and `ReplayOutput.snapshots_used` are additions beyond spec 9.1's sketch (see Q3)") — the `ReplayInput.missing`/`ReplayOutput.snapshots_used` additions are now recorded in the spec itself by `SPEC-CHANGELOG-8.md` (Q3).
- **Commit(s):** `3c41aef` + `347e6e1` + `57d23a6` (review clean: 3 Important fixed in one round, 2 Minor fixed in a tiny orchestrator-verified round 2).

**Also recorded — Task 18's brief did not assign the postflop `assumptions.translations`/`.mappings` fill (final review P23 / F-I1):** neither Task 17 nor Task 18's brief states that the postflop (`serve.rs` turn/river) path must fill `Assumptions.translations` and `.mappings` from the replay's inherited reasons the way Task 17's own preflop path does at `preflop.rs:211-212`; the final whole-branch review found both lists empty on every turn/river `Final` that followed a translated preflop or flop action (spec 8.4 line 532, 4.4 line 249). This is a plan-3 brief gap, not a task-18 code defect as built — final review finding **F-I1**, ruled **FIX** in the final review's own separate fix round (`final-fix-report.md`; not this documentation task's scope to implement). Feeds final review **P23** ("Task 18 does not say that the postflop path fills `assumptions.translations` and `assumptions.mappings` from the replay's reasons (F-I1)").

**Also recorded — `Engine::register_snapshot` cannot be the registration seam plan 4 is told to use (final review P26):** the spec's Interface table (section 3.5-adjacent engine row) and plan 4's own text describe `Engine::register_snapshot(&mut self, ..)` as the façade the background pre-solver registers cache snapshots through. It cannot be called from inside `serve` (engine-main holds `&mut EngineCore` there already); the actual in-engine seam Task 18 built and plan 3's own delivery path uses is `replay_bridge::register_accepted` inside an accepted delivery. Feeds final review **P26** ("`Engine::register_snapshot(&mut self, ..)` is described as plan 4's registration façade. It cannot be called from `serve` ... the plan should name `replay_bridge::register_accepted` inside an accepted delivery as the in-engine seam") — recorded here for plan 4's benefit; no plan-3 code changes.

### Task 19 — Golden fixtures for `core-preflop`/`core-replay`, oracle cross-check (P3.T19)

**Erratum 1 — the brief's off-menu prose sizes do not exercise the intended boundary**

- **Task/step:** the brief's off-menu-line prose.
- **Plan text (<=25 words, paraphrase):** a 2.5 bb / 8.75 bb off-menu pair, which does not let `A`, `B` and the interpolation fraction `s` hold exact representable values together.
- **What the code does instead:** a 20/40 chip line with a 2.25/3.5 bb menu and a raise to 113 chips, chosen so `A`, `B` and `s` hold exactly.
- **Why:** ruling 19-D2, accepted, "erratum candidate (brief prose 2.5/8.75 bb)" (`progress.md:240`). Feeds final review **P19** ("Task 19: the brief's 2.5/8.75 bb off-menu line becomes the 20/40, 2.25/3.5 bb, raise-to-113 line (19-D2)").
- **Commit(s):** `6bbadf7` + `3895681` + `1d234a7` (review clean: 1 Important fixed across two rounds).

**Erratum 2 — the brief's legal-move row does not sum to 1**

- **Task/step:** the legal-move golden row.
- **What the code does instead:** an explicit `Fold` probability of `0.3` is added so the row sums to exactly 1.
- **Why:** ruling 19-D6, accepted, "erratum candidate" (`progress.md:240`). Feeds final review **P19** ("Fold 0.3 is added so the legal-move row sums to 1 (19-D6)").
- **Commit(s):** `6bbadf7` + `3895681` + `1d234a7`.

---

## Global and cross-task plan issues (from the final review)

These plan-3 issues are not tied to one task's own Step block; they are recorded here as the final review's **P5**, **P20**, **P21**, **P22**, **P24** and **P25**.

- **P5 — the Global Constraints module list is stale, and Task 18's `classify` framing/message are stale.** The plan's copy of `crates/engine/src`'s module list (line 113) omits `bench_support`, `startup` and `pub use startup::StartupReport;`, all present in plan 2's own later, self-reviewed list; Task 18's prose frames the `RootError::Inconsistent` fix as conditional on plan 2 "still" having a three-variant `classify` match (it already has five) and gives a message string (`"street root inconsistent at step {step}"`) that differs from what plan 2 actually emits (`"street root does not replay at step {step}"`). Neither is a coding defect: Task 17's file-modify instruction is safe against the real list, and no plan-3 test asserts the stale string. Pre-flight rulings 6 and 7 (`progress.md:93-94`), rubric defects R6/R7 (`progress.md:78-79`).
- **P20 — the plan-3 header and `constraints.md` cite spec "revision 7"; the spec was already at revision 8 when plan 3 executed** (`constraints.md` line 1). Stale cross-reference, no code impact — plan 3 revision 3's own text was drafted against spec revision 7 and never re-synced after the spec's F20/R1 bumps to revision 8. Not corrected by this changelog (task text is never patched); noted for a future plan-3 revision if one is ever needed for another reason.
- **P21 — NOT an erratum, resolved by spec amendment instead.** Plan 3 line 1463 (the SB fold-EV cross-check) rejects the whole bundle on a failed cross-check; the pre-revision-9 spec (line 521) said "the node is unloadable". The final review's **Q4** ruled the *code's* bundle-level behavior correct (it matches `store.rs:85,125-151`'s actual quarantine granularity) and amended the *spec* to match, via `SPEC-CHANGELOG-8.md` — "a failed EV cross-check indicates a systematic unit or scaling error in that bundle". Plan 3 line 1463 is therefore not listed as a plan defect here.
- **P22 — the unit of `observed_pct`/`mapped` (plan line 1528) is a genuine plan-text erratum.** Plan 3 line 1528 specifies `observed_pct: 100*s, mapped: [(100*A, f_A), ...]` (percent-display values), but the code (`core-replay/src/preflop.rs:734-735`) emits pot fractions (e.g. `0.73`), matching plan 1 line 837's own convention and the Task 19 goldens, which freeze fractional values. The final review's **Q5** confirms the code and the (now-amended) spec are correct and plan 3 line 1528 is the erratum, not a spec gap; `SPEC-CHANGELOG-8.md` states the pot-fraction unit explicitly. Plan 5's rendering must format the fraction as a percentage for display (carried to plan 5, not a plan-3 fix).
- **P24 — the Interface section (and plan 4 line 83) imply `PreflopStore::open` reads packaged `<name>.manifest.json`/`<name>.json` file pairs.** `open` reads bundle *directories* only and silently skips ordinary files; packaged pairs are loaded by `engine::preflop::load_store` instead (ruling 17-D3, `progress.md:220`). Recorded so plan 4 is not built assuming `open` handles packaged pairs directly — plan 4's own seam notes in `final-review.md` ("Seams plan 4 and plan 5 will have to change", item (b)) restate this.
- **P25 — a stopped branch's cause other than a missing node had no disclosure wording in the spec's vocabulary.** This is the same underlying gap ruling 19-I1 fixed in code (Task 16, Erratum 1 above) and the final review's own finding **F-M3**; it is now resolved at the spec level by `SPEC-CHANGELOG-8.md` (**Q1**), which adds `"stopped <cause>"` to the `BranchResidual.cause` vocabulary.

## Not an erratum

- **Task 3 (`tools/chart_ingest.py`, deterministic chart ingestion).** Review P3T3R found 3 Important + 2 Minor findings (Python admitting values the Rust schema rejects, unvalidated inventory rows, a grid test that cannot detect a transpose, diagnostics, report evidence) plus one further Minor at re-review (valid-boundary acceptance tests missing). All were about the implementer's own tool code and its own test coverage, not the plan's prescribed Step-block text; none is tagged as a plan-mandated erratum candidate in the ledger, and none appears in the final review's P1-P26 list. Excluded here for the same reason plan 2's changelog excluded Task 18/21.
- **Task 4 (network acquisition of the two public chart PDFs).** Review P3T4R found 3 Important + 1 Minor (legend/rake/rounding not read from the PDFs directly, an integrity test that did not hash bytes, redacted HTML lacking a provenance record, evidence). Same reasoning as Task 3: implementer-side tool defects, not plan-text defects; not in the final review's P-list.
- **Task 11 (branch conditioning kernel: `condition`, `marginal`, `posterior`, `rescale`).** Review P3T11R found 2 Important (positive reach underflowing to zero in `q*w` accumulation, solved by scaled accumulation with a named assertion on unrepresentable results; frozen/aggregate paths bypassing branch-weight validation) plus 1 Minor (wording). Both are about the implementer's own numerical-accumulation and validation code — the plan's own `condition`/`marginal`/`posterior`/`rescale` sketch was found *self-consistent* against its own test in the pre-flight scan (Table B) and is not listed as defective anywhere in the ledger or the final review's P-list. Excluded for the same reason as Tasks 3 and 4.
- **The final review's own code-level findings F-I1, F-M1 and F-M2** (turn/river `assumptions.translations`/`.mappings` empty; a module-doc wording gap; the `prominent = deviation > 0.10` float-boundary comparison) are **not** listed in `final-review.md`'s "Plan issues (feed PLAN-3-CHANGELOG-4)" section — they are code-review findings against the implemented crates, addressed by the final review's own separate fix round (`final-fix-report.md`), not plan-3 task-text defects. F-I1 is cross-referenced above under Task 18 (as P23) because the final review explicitly ties it to a plan-3 brief gap; F-M1 and F-M2 are pure implementation findings and are not repeated here.

## Spec questions raised — resolved in this session, not carried forward unresolved

Unlike plan 2's changelog (which left its spec questions open), all five of plan 3's final-review spec questions were given rulings by the orchestrator in the same review round and are now resolved in the spec text itself:

1. **Q1 — does the `BranchResidual` cause vocabulary cover preflop stops other than a missing node?** No, as written. Resolved: `SPEC-CHANGELOG-8.md` adds `"stopped <cause>"` to spec line 231 and a sentence to spec line 533. Code fix: final review **F-M3** (separate fix round, not this task).
2. **Q2 — which key does `MissingPreflopNode` carry when the heaviest positive branch is the residual?** Ruling 16-R2's all-branch fallback could name a present node. Resolved: `SPEC-CHANGELOG-8.md` amends spec line 533 to restrict the fallback to `B+` branches that retain a key, with the label `"cap residual (no node)"` for a residual-only case.
3. **Q3 — should spec 9.1 record the as-built replay interface?** Yes. Resolved: `SPEC-CHANGELOG-8.md` adds a sentence after the section 9.1 code block naming `ReplayInput.missing`, `ReplayOutput.snapshots_used`, `core_replay::SnapshotMiss`, `RootRanges.snapshots_used`, `replay_decision`/`DecisionLookup`.
4. **Q4 — node-level or bundle-level rejection on a failed EV cross-check?** Bundle-level, matching the code. Resolved: `SPEC-CHANGELOG-8.md` amends spec lines 521 and 712 (13.1 T1) from "the node is unloadable" to "the bundle is quarantined".
5. **Q5 — what unit is `observed_pct`?** Pot fraction, matching the code; plan 3 line 1528's `100*s` is the erratum (see P22 above), not the spec. Resolved: `SPEC-CHANGELOG-8.md` adds an inline comment to spec line 225.

See `SPEC-CHANGELOG-8.md` for the exact before/after spec text of all five. (That document is filed as `-8` rather than the final review's own proposed `-6`, because `SPEC-CHANGELOG-6.md` and `-7.md` already exist on `phase-c` for an unrelated donk-option fix — see that file's own filename note.)

## Verification

Documentation-only task: **Rust/Python/UI execution: not applicable** — this task edits only `docs/research/PLAN-3-CHANGELOG-4.md`, the plan's revision lines, and today's journal entry; no code changed. Verification is the check named in the dispatching brief: every `Pn`/`Qn` id and every `Ruling (<id>)` cited above was independently confirmed present in `.superpowers/sdd/2026-09-10-plan-3-preflop-replay/progress.md` and `final-review.md` by reading those files in full before this document was written (see the agent's report, `.superpowers/sdd/2026-09-10-plan-3-preflop-replay/errata-report.md`, for the grep/count evidence), and every plan line number quoted above was taken from the final whole-branch review's own citations against `docs/superpowers/plans/2026-09-10-plan-3-preflop-replay.md`.
