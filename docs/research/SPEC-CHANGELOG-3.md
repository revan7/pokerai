# SPEC-CHANGELOG-3 — disposition of REVIEW-of-spec-3 findings and the flop-policy amendment (2026-09-10)

Applies to `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`, revision 4 (801 lines, 16 sections). Authority order: the orchestrator's binding decisions, then the outline as amended today (four amendments, including the section-6 flop policy), then the review. Section numbers refer to revision 4. The pinned `src/action_tree.rs` (`push_actions`, `create_next`, `merge_bet_actions`, `BuildTreeInfo`) was re-read at commit `9d1509fe` for P01; R8's addendum A.1-A.7 was read for P12. No application code, fixture or benchmark was written or run.

## Blocking findings

| Id | Severity | Disposition | Where / how |
|---|---|---|---|
| P01 | BLOCKER | APPLIED | §4.6 rewritten as a transcription of the pinned builder: node quantities (`own`, `opp`, `to_call`, `prev`, `matched`, `pot = starting_pot + 2 * (matched + to_call)`, `max = opp + prev`, `min = clamp(prev + to_call, 1, max)`); three node kinds: **opening** (all-in added when `max <= round(pot * add)`), **donk** (explicit empty donk option pinned; `None` never sent), **facing** (raises only when the pending wager is not an all-in; **all-in added when `max <= prev + round(pot * add)`**; facing an all-in = Fold/Call only); **clamp, then force on the clamped amount, then sort/dedupe, then `merge_bet_actions` with 0.0**; equality examples for both rules (opening 150/151 at 1.5; facing 400/401 at 1.0 with the reviewer's 350 counterexample; force boundaries 80/81 opening and 340/341 facing; clamp-to-max 240). §10.1 threshold note restated. §13.2 `tree_materialization_matches_library` gains the `facing_test_v1` fixtures at stacks 350/400/401/340/341/240/100, the donk-pinning check and three `tree_mismatch` triggers. `rules_version` bumped to 3 (§4.6, wire example). §4.5 wire-example note corrected (clamp then force) |
| P02 | BLOCKER | APPLIED | §8.4 "Node translation and assembly": `EV(a,c) = sum_b pi_b[c] EV_b(a,c)` **only when every positive-posterior branch has `a` with a normalized EV**; otherwise `ev = None` with the new `Unavailable::BranchSupportIncomplete{covered_posterior}` (§4.4), `NoEvReference` or `NotInMenu`; missing branches are never normalized away; known frequency mass per action plus `Recommendation.unresolved_mass` (§4.4) with `sum + unresolved = 1`; the 0.2/0.8 example written out (revision 3 would have reported +10). §4.4 headline: EV headline only with complete EVs, frequency headline only when `unresolved_mass == 0` (new wording "EV incomplete"), otherwise no headline. **Cross-actor rule** (§8.4 "Shared history branches and actor transitions"): one shared history-branch list; an off-menu wager by `V` splits every seat's branch; non-acting seats receive `q_X = M_X / (M_A + M_B)` with `M_X` = `V`'s post-split branch total mass (range-integrated observed likelihood times `f_X`), marginals unchanged, posterior `q_k` before the seat acts; card-removal-free; zero-mass branches dropped; per-seat cap/residual; the two-actor numeric example (T6). **Explicit fallback** (§9.3): stopped branches keep the last justified masses, conditioning stops with reasons, `MissingPreflopNode` when no positive-posterior branch has a node, partial absence feeds `unresolved_mass` with `BranchResidual{cause: "missing node"}` (§6 new row, §12 new row). §9.1 types: `HistoryBranch`, `Branch.history/stopped`, `ReplayOutput.history_branches`. Tests T6 `replay_cross_actor_branches`, T7 `replay_incomplete_branch_ev`; `recommendation_assembly_golden` extended |

## Major and minor findings

| Id | Severity | Disposition | Where / how |
|---|---|---|---|
| P03 | MAJOR | APPLIED | §10.2 second worked case replaced by "A bet50, B call50, C raise150, A raise250, B fold, C to act" (order A, B, C; 1,000-chip stacks; projection A Bet(50), C Raise(150), A Raise(250); `dead = 50`; C faces 100, min re-raise 350); §13.1 `multiway_root_projection` updated with the same data and a note that out-of-turn sequences are rejected by `apply_action`; the zero-dead and rejected cases kept; §13.3 labels `MultiwayStreetRoot{1, 50}` unchanged |
| P04 | MAJOR | APPLIED | §10.4 scale check and §13.1 T4: non-root `Exact` hits at IP's node after Bet(50) vs Bet(100) and OOP's node after Check, Bet(50) vs Check, Bet(100); the call and turn card are appended separately to test cache-backed replay into the next street; no next-street strategy is requested from the flop entry |
| P05 | MAJOR | APPLIED | §13.1 `replay_snapshot_prefix_reuse` split into three explicit exports: requested-only `[check]` (root absent: check unconditioned, 73 translated at the covered node, call uncovered), root-only `[]` (check conditioned, bet uncovered), complete export; §9.2 case 3 restated per node with the walk continuing past an absent node (ordinal paths are well defined without the root) |
| P06 | MAJOR | APPLIED | §13.2 `deadline_best_so_far_bounded`: `ok` iff raw exploitability meets 1 bp before the stop point, else measured `best_so_far`; bounded delivery and a complete payload in either case; deterministic forced BestSoFar moved to the engine golden `deadline_best_so_far_labelling` (§13.3, fake worker/clock); `deadline_no_iteration` and `final_delivery_independent_of_worker` retained |
| P07 | MAJOR | APPLIED | §10.4: 0.49% is now the root-action deviation; `MenuRounded{max_delta_pct}` is the maximum over the complete materialized list (0.97% at the turn half-pot bets, larger deeper); T4 freezes the full-tree value (`0.97 <= v < 5`). The `MenuRounded{2.0}` case is a specified pair of trees (`menu_round_test_v1`, 100/500 vs 20/100) with its node-by-node deviations and all-in additions checked. Rake-cap crossings are detected by a separate **terminal rake-cap predicate** on `MaterializedNode.terminal_pots` (§4.6, §10.4 step 1b) with the 500/504, cap 55.2 example; topology comparison now includes actor, street and terminal-versus-continuation identity (§2); the claim that topology detects rake-cap crossings was removed; the 2% SPR policy and raw accuracy filter unchanged |
| P08 | MAJOR | APPLIED | §4.5 validator requires every probability in `[0, 1]` in addition to finiteness, dimensions, support mask and row sums; lock rows in `[0, 1]`, all-zero permitted solely as the free-combo sentinel; `[-0.1, 1.1]` rejection fixtures added to `cache_payload_validated` and `protocol_rejections`; §8.2's `[0, 1]` bundle check cross-referenced |
| P09 | MINOR | APPLIED | Cardinality settled as **4 source branches plus 1 residual** in §8.4, §9.1 and `replay_branch_cap_residual`; `log_reach` accumulates `ln` of the removed maximum (minus the log of the multiplier), always `<= 0`, in §8.4 and §9.2; T3 keeps `ln(0.18)` with the derivation (0.8 x 0.25 x 0.9) |
| P10 | MINOR | APPLIED | §4.3 dealt-seat order: "remaining non-button, non-blind dealt seats clockwise after BB, then BTN, SB, BB", with 3-, 4- and 5-seat enumerations; `dealt_seats_3_to_6` enumerates BTN/SB/BB and CO/BTN/SB/BB preflop and postflop |
| P11 | MINOR | APPLIED | §2 defines the materialized tree as the **betting skeleton** (abstract `ActionTree`, one collapsed chance boundary per street transition, street/actor/terminal markers, no card expansion; card expansion is the worker's); `ChipPath` (wire) and `OrdinalPath` (engine/cache/replay) named in §4.6 and used in §4.5/§9.1; ingestion resolves chip paths against the source materialized tree before cache selection or snapshot registration (unresolvable = `invalid solution`); exports stay limited to current-street decision nodes; `build_effective_tree` now takes `(&StreetRootSnapshot, &TemplateSelection)` so the builder's input exists before an `EffectiveTree` does (§3.5, §4.3) |
| P12 | MAJOR | APPLIED (with the outline's flop-policy amendment) | See "Flop policy" below; additionally the preamble distinguishes the addendum's analogue measurements from the production template; empty donk menus pinned to the explicit empty option (§4.6, §10.3, §13.2); f32/i16 crossover benchmarked at 2 GiB as well as 4 GiB (§10.3, V3); the 12-hour tier-1 estimate replaced by workload-qualified figures (§10.5); the 15 s hard deadline at the default and the labelled fallback kept; no narrowing of public ranges; no claim that the baseline gate has passed |

## Flop-policy items (outline §6 amendment; orchestrator decision 4)

| Item | Disposition | Where |
|---|---|---|
| Realistic-range measurements replace the flop numbers | APPLIED | §7 table (SRP 100bb 19-35 s / 3.2-5.3 GB; 200bb 39-64 s / 5.2-8.8 GB; 3-bet 4.1-6.7 s under 1 GB; i16 faster on large trees; turn 200bb two sizes 0.97 s / 148 MB; cold vs warm within 2% after iteration 1; outliers after sustained load), §10.1 table, §14.4 V3, §15 risk 3 |
| Cache plus pre-solver = PRIMARY single-raised-pot flop path, required phase-1 feature | APPLIED | §7 table, §10.5 status paragraph, §14.2, §1.2 non-goals narrowed to "beyond tier 1" |
| Live flop miss runs to the deadline and returns `DeadlineBestSoFar`; i16 when the f32 estimate exceeds 2 GiB; same rule is the storage-mode rule | APPLIED | §7, §10.3 (f32 <= 2 GiB, else i16 <= 8 GiB, else `tree_too_large`; `memory_limit_bytes` default 10 GiB), §10.5, §10.6, §12 |
| `flop_min_v1` measured in V3; becomes the live single-raised-pot template if it fits | APPLIED | §10.1 table and template-selection rule, §14.4 V3 criterion (c), §16 Q2 |
| 3-bet pots keep `flop_fast_v1` live | APPLIED | §10.1 template selection (pot class from the preflop history), §7, §13.5 gate |
| Per-session `flop_budget_s` (default 10, max 30); final delivery `5 s + flop_budget_s` (15 s at the default); pending user decision | APPLIED | §4.2 `SolverPrefs.flop_budget_s`, §7 deadlines (stretch applied to flop decisions only, spec decision), §13.3 `flop_budget_setting_golden`, §16 Q5 |
| Throughput: 1,755 x 27 s = 13 h per SRP scenario at 100bb i16, tier 1 about 52 h, about 25 h per scenario at 200bb; tier 1 completion required before the flop path counts as covered; hit rate as the coverage measure | APPLIED | §10.5 throughput paragraph and status paragraph, §5 step 10 (scenario class logged), §14.2 |
| Risk 3 and V3 pass criterion updated with the realistic-range facts | APPLIED | §15 risk 3, §14.4 V3 (a)-(d), §13.5 baseline gate (3-bet live bound; SRP best-so-far within budget; no live SRP claim) |

## Previously partial findings (closed by the P-edits above)

| Id | Disposition | Closing edit |
|---|---|---|
| N02 | APPLIED | P03 (out-of-turn fixture replaced) |
| N04 | APPLIED | P04 (non-root hits before the street closes; next-street replay tested separately) |
| N06 | APPLIED | P05 (truncated-export fixtures no longer condition an unexported ancestor) |
| N07 | APPLIED | P02 (complete-support EV, cross-actor rule) and P09 (cardinality, log sign) |
| N09 | APPLIED | P10 (dealt-seat order) |
| N10 | APPLIED | P06 (conditional deadline outcome) |
| N11 | APPLIED | P01 (facing-wager threshold, clamp/force order) |
| S09 | APPLIED | P04 and P07 (non-root scaled hits; full-tree deviation; rake-cap predicate) |
| S12 | APPLIED | P02 (no EV over incomplete branch support) |
| S15 | APPLIED | P02, P05 and P09 (cross-actor continuation, truncated-prefix oracle, log sign) |
| S22 | APPLIED | P08 (`[0, 1]` bound in the shared validator) |
| S25 | APPLIED | P01, P07 and P11 (facing threshold; payoff-boundary predicate; materialization scope) |
| S26 | APPLIED | P06 (no unconditional BestSoFar assertion) |
| S30 | APPLIED | P10 (duplicated BTN removed) |

## Planning follow-ups absorbed as small edits

- `ev_convention_non_root_payoffs` now names explicit ranges (OOP AA + 66, IP QQ + 54o at 0.25) and **locks** IP's river node (bet QQ 100%, 54o 20%) so that the positive (+200) and negative (-50) call values do not depend on an equilibrium tolerance (§13.2).
- `range_hash_scale_invariant` bounds its inputs (maximum weight <= 0.25, positive weights >= 2^-100) and asserts a changed hash on changed normalized bits, not injectivity (§13.1).
- The f64-to-f32 boundary for tiny positive weights is fixed: clamp to `f32::MIN_POSITIVE` (§9.2, spec decision).
- The tree builder's input before an `EffectiveTree` exists is `(&StreetRootSnapshot, &TemplateSelection)` (§3.5, §4.3).
- §13.5 states that the plan freezes chart URLs/versions, covered chart nodes, synthetic missing-node fallbacks and the 50-hand inputs before benchmarks are generated.

## Not applied

Nothing from the review or the orchestrator's decisions was left unapplied. One interpretation choice is recorded as a spec decision rather than taken from the review: the stretched final-delivery deadline `5 s + flop_budget_s` applies to flop decisions only; river and turn decisions keep `t0 + 15 s` (identical at the default). Section 16 Q5 records the setting itself as the pending user decision.
