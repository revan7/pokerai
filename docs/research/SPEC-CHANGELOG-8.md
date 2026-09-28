# Spec changelog: revision 8 -> revision 9

Source: the plan-3 final whole-plan review, `.superpowers/sdd/2026-09-10-plan-3-preflop-replay/final-review.md` (Claude Opus 5.5, independent final reviewer; base `60f021a`, head `6d1d567`), section "Spec questions" **Q1-Q5**, and its "Rulings" line at the end of the review ("(F-M3 + Q1) FIX with a SPEC AMENDMENT... (Q2) ruling 16-R2 AMENDED... (Q3) SPEC-CHANGELOG-6 records the as-built 9.1 interface... (Q4) bundle-level quarantine stands, spec 8.3 line 521 and 13.1 T1 amended... (Q5) observed_pct and mapped fractions are POT FRACTIONS, name kept, spec 4.4 line 223 amended, plan 3 line 1528 is an erratum").

**Filename note**: the final review's own text proposes filing these edits as `SPEC-CHANGELOG-6.md` (it was scoped to plan 3's docs and did not check the rest of `docs/research/`). `SPEC-CHANGELOG-6.md` and `SPEC-CHANGELOG-7.md` already exist on `phase-c` — an unrelated F20 donk-option fix (revision 6 -> 7) and its R1 precision follow-up (revision 7 -> 8), both from `REVIEW-cross-plan-2.md`/`REVIEW-cross-plan-3.md`, merged earlier in the branch's history. Per CLAUDE.md section 3 ("the next spec edit adds `SPEC-CHANGELOG-6.md`, not a rewrite of an existing one" — generalized: the next spec edit adds the next unused number, never overwrites an existing changelog), this document is filed as **`SPEC-CHANGELOG-8.md`**. The revision arithmetic is unaffected: the spec was already at revision 8 before this edit (as the final review's own base assumed), so this changelog still bumps revision 8 -> 9.

Spec edited in place: `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (now revision 9).

| # | Section touched | Edit applied |
|---|---|---|
| Q1-1 | Title / revision line | Bumped `revision 8` -> `revision 9` in the title; added a new revision line "Revision 9 (2026-09-28): ... (Q1-Q5) from the plan-3 final whole-branch review; changelog SPEC-CHANGELOG-8.md." directly under the title, above the existing Revision 8 line (kept unchanged, not replaced). |
| Q1-2 | 4.4, `BranchResidual.cause` doc comment (spec line 231) | Added `"stopped <cause>"` to the cause vocabulary, between `"missing node <key>"` and `"uncovered path <ordinal path>"`. |
| Q1-3 | 8.4, end of the "Node translation and assembly" paragraph (spec line 533) | Added a sentence: a branch stopped for a cause other than a missing node (section 9.3's bounded fallback: an unmappable action or size, a rejected split) contributes its posterior to `unresolved_mass` with `BranchResidual{seat: hero, residual_mass_pct, cause: "stopped <cause>"}`. |
| — | 4.4 line 412 (coverage-rules table) and 4.4 line 647-650 (assumptions/failure table) | **Not changed**, per Q1's own instruction ("Lines 412 and 647 keep their labels") — both already read `BranchResidual{cause: "missing node"}` and describe the missing-node case specifically, which is still correct; they do not need the new `"stopped <cause>"` wording. |
| Q2 | 8.4, same paragraph, the `MissingPreflopNode{key}` sentence (spec line 533) | Replaced "with the key of the heaviest branch (largest `q_k`)" with "with the key of the heaviest branch of `B+` that retains a key; when only the cap residual remains, the key is the label `cap residual (no node)`" — amending ruling 16-R2 so the key never names a branch whose node is present when only the cap residual is positive. |
| Q3 | 9.1, after the `ReplayInput`/`ReplayOutput`/`replay` code block (spec line 554) | Added a sentence recording the as-built interface: `ReplayInput.missing`, `ReplayOutput.snapshots_used`, `core_replay::SnapshotMiss` with `SnapshotStore::{record_miss, misses_for_identity}`, `RootRanges.snapshots_used`, `replay_decision`/`DecisionLookup`. |
| Q4-1 | 8.3, `net_hand_start_verified` cross-check (spec line 523) | Replaced "else the node is unloadable" with "else the bundle is quarantined (section 8.2; a failed cross-check indicates a systematic unit or scaling error in that bundle)". |
| Q4-2 | 13.1, T1 test-list row (spec line 712) | Replaced "a fold EV of -0.5 makes the node unloadable" with "a fold EV of -0.5 quarantines the bundle". |
| Q5 | 4.4, `ApproxReason::BetTranslation` field list (spec line 225) | Added inline comments to `observed_pct: f32` and `mapped: Vec<(f32, f32)>` stating both are pot fractions (e.g. `0.73`), the field name kept for wire stability. |

## Exact lines changed (before / after)

### Q1-1 — title and revision line (spec lines 1-5)

Before:
> \# PokerAI assistant: design specification (2026-09-10, revision 8)
>
> Revision 8 (2026-09-17): R1 from docs/research/REVIEW-cross-plan-3.md; changelog SPEC-CHANGELOG-7.md.

After:
> \# PokerAI assistant: design specification (2026-09-10, revision 9)
>
> Revision 9 (2026-09-28): branch-stop cause vocabulary, the `MissingPreflopNode` key fallback, section 9.1's as-built replay interface, bundle-level EV-cross-check quarantine, and the `observed_pct`/`mapped` pot-fraction units (Q1-Q5) from the plan-3 final whole-branch review; changelog SPEC-CHANGELOG-8.md.
>
> Revision 8 (2026-09-17): R1 from docs/research/REVIEW-cross-plan-3.md; changelog SPEC-CHANGELOG-7.md.

### Q1-2 — section 4.4, `BranchResidual` cause comment (spec line 231)

Before:
> `MenuRounded { max_delta_pct: f32 /* realized, section 10.4 */ }, BranchResidual { seat: Seat, residual_mass_pct: f32, cause: String /* "cap" | "missing node <key>" | "uncovered path <ordinal path>", section 8.4 */ },`

After:
> `MenuRounded { max_delta_pct: f32 /* realized, section 10.4 */ }, BranchResidual { seat: Seat, residual_mass_pct: f32, cause: String /* "cap" | "missing node <key>" | "stopped <cause>" | "uncovered path <ordinal path>", section 8.4 */ },`

### Q1-3 / Q2 — section 8.4, "Node translation and assembly" paragraph, `MissingPreflopNode` key clause and the trailing sentence (spec line 533)

Before (the relevant clause and the paragraph's end):
> **Current lookup availability**: when hero has a node in no branch of `B+`, the decision is `Unsupported{MissingPreflopNode{key}}` with the key of the heaviest branch (largest `q_k`); when hero lacks a node in only some of them, those branches stop (section 9.3), their posterior joins `unresolved_mass`, and `BranchResidual{seat: hero, residual_mass_pct, cause: "missing node <key>"}` is added. The range mix uses `sum_{k has node} q_k * w_{H,k}[c]` as the mass over the branches with nodes.

After:
> **Current lookup availability**: when hero has a node in no branch of `B+`, the decision is `Unsupported{MissingPreflopNode{key}}` with the key of the heaviest branch of `B+` that retains a key; when only the cap residual remains, the key is the label `cap residual (no node)`; when hero lacks a node in only some of them, those branches stop (section 9.3), their posterior joins `unresolved_mass`, and `BranchResidual{seat: hero, residual_mass_pct, cause: "missing node <key>"}` is added. The range mix uses `sum_{k has node} q_k * w_{H,k}[c]` as the mass over the branches with nodes. A branch stopped for another cause (section 9.3's bounded fallback: an unmappable action or size, a rejected split) contributes its posterior to `unresolved_mass` with `BranchResidual{seat: hero, residual_mass_pct, cause: "stopped <cause>"}`.

### Q3 — section 9.1, after the interface code block (new sentence, spec line 554)

Before: (code block ends, section 9.2 follows directly with no prose in between)

After, sentence added immediately after the closing ` ``` `:
> These types are a sketch; the implementation extends them with `ReplayInput.missing: &[(Street, String)]`, `ReplayOutput.snapshots_used: Vec<(Street, SnapshotProvenance)>`, `core_replay::SnapshotMiss` with `SnapshotStore::{record_miss, misses_for_identity}`, `RootRanges.snapshots_used` (the plan-2 seam that carries selected-snapshot provenance into the turn/river `Final`, section 9.3), and `replay_decision`/`DecisionLookup` for hero's own node lookup.

### Q4-1 — section 8.3, `net_hand_start_verified` cross-check (spec line 523)

Before (fragment):
> ...at fold-legal nodes this equals `ev_sb(a) - ev_sb(fold)` and the two are cross-checked (`|difference| <= 1e-3` else the node is unloadable);

After:
> ...at fold-legal nodes this equals `ev_sb(a) - ev_sb(fold)` and the two are cross-checked (`|difference| <= 1e-3` else the bundle is quarantined (section 8.2; a failed cross-check indicates a systematic unit or scaling error in that bundle));

### Q4-2 — section 13.1, T1 test-list row (spec line 712)

Before (fragment):
> ...under `net_hand_start_verified` an SB node with fold EV -1 and raise EV 1.84 gives `ev_inc = 2.84` and the fold cross-check passes, while a fold EV of -0.5 makes the node unloadable; under `absolute_stack_verified`...

After:
> ...under `net_hand_start_verified` an SB node with fold EV -1 and raise EV 1.84 gives `ev_inc = 2.84` and the fold cross-check passes, while a fold EV of -0.5 quarantines the bundle; under `absolute_stack_verified`...

### Q5 — section 4.4, `ApproxReason::BetTranslation` field list (spec line 225)

Before:
> `BetTranslation { street: Street, seat: Seat, observed_pct: f32, mapped: Vec<(f32, f32)>, deviation: f32, prominent: bool },`

After:
> `BetTranslation { street: Street, seat: Seat, observed_pct: f32 /* pot fraction, e.g. 0.73; field name kept for wire stability */, mapped: Vec<(f32, f32)> /* first element of each pair is a pot fraction too */, deviation: f32, prominent: bool },`

## Not changed

- Section 4.4 lines 412 and 647-650 (coverage-rules and assumptions/failure tables): both already read `BranchResidual{cause: "missing node"}`/`"missing node"` for the missing-node case specifically and are still correct; Q1 explicitly said these labels stay.
- Plan 3 line 1528 (`observed_pct: 100*s, mapped: [(100*A, f_A), ...]`) is **not** a spec error to fix here: the final review's ruling on Q5 is that the code (pot fractions) and the spec (now explicit about pot fractions) are correct, and plan 3's `100*s`/`100*A` percent-display prose is the erratum — it is recorded in `docs/research/PLAN-3-CHANGELOG-4.md` (plan-level issue P22), not corrected by editing the spec to match it.
- No other section, row or fixture list was touched.
