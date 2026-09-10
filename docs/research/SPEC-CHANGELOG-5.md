# Spec changelog: revision 5 -> revision 6

Source: `docs/research/REVIEW-cross-plan.md` section 3 ("Spec changes proposed by the plans"), all 17 ACCEPTED
items (S1-S17). REJECTED items (R1-R6) are plan-side corrections and are not applied to the spec.
Also applied: plan 2's `ev_convention_non_root_payoffs` fixture correction in section 13.2 (accepted per
instruction), which is the same edit as S7.

Spec edited in place: `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (now revision 6, 807 lines).

| # | Section touched | Edit applied |
|---|---|---|
| S1 | 4.3 `StreetRootSnapshot` | Added field `pub bb_chips: u32, // minimum bet for replay_root; ignored by the worker`. |
| S2 | 3.5 `core-model` row | `RootError` widened from `{Multiway, ProjectionNotReproducing{step}, NoDecision}` to `{Multiway{pot_eligible}, ProjectionNotReproducing{step}, NoDecision, Preflop, Inconsistent{step}}`. |
| S3 | 3.5 `core-eval` row; 7 equity row | Added `EquityMode::MonteCarlo{seed, max_samples}` and `exact_cost(&EquityRequest) -> u64`; both rows now gate exact-vs-MC on `exact_cost(&req) <= 2 * 10^7` (section 7's row previously spelled this out as `pairs * runouts <= 2 * 10^7`, now expressed via the named function). |
| S4 | 3.2 `core-eval` row | Replaced "weighted range-vs-range equity via `pokers` (MIT)" with "weighted range-vs-range equity in-house over the `Evaluator` trait", with the measured reason (`pokers 0.10` stores weights as `u8` percent, cannot represent `Range1326`) inline. |
| S5 | 13.1 `state_machine_pokerkit_fixtures` | Added the two generator normalizations (straddle minimum open `2S`; fold-out survivor bet split into matched + refund) and the dropped-seed note (1 in 201, where PokerKit's reopening rule diverges from section 4.3). |
| S6 | 4.3, after the data-model code block (`Derived`/`HandState`) | Added the per-seat-vector convention: length-6 vectors indexed by `Seat.0`; an undealt seat has `folded = true`, `all_in = false`, zeros elsewhere; `HandState.stacks_start` is in `dealt` order. Placed as prose immediately after the code block (ambiguous placement — see note below). |
| S7 | 13.2 `ev_convention_non_root_payoffs` | Replaced OOP `AA` with OOP `QQ` (kept `66`); reworded the `+200` case from "`AA` (equity 1)" to "`QQ` (equity 1 through card removal: zero compatible mass vs. IP's locked QQ combos, full compatible mass vs. IP's 54o combos)". Signed positive (+200), negative (-50), check and fold cases are unchanged. This is the same fix as the standalone section-13.2 correction requested alongside S1-S17. |
| S8 | 13.2 `river_check_only_terminal_oracle` | Added: runs in `crates/engine/tests/worker_link.rs` (still spawning the worker binary) because its oracle is `core-eval`, which `solver-worker` may not depend on (section 3.2). |
| S9 | 13.2 `ev_conservation` | Added: checked in-process through the adapter's mapping; IP's root-range EV is not an actor-owned wire export. |
| S10 | 13.2 `tree_materialization_matches_library` | Split the assertion into (a) in-process equality over the 47 boundary cases and (b) the wire `tree_mismatch` cases, keeping the same test name/spot and all existing content (narrowest reading — see note below). |
| S11 | 4.4 `ApproxReason::AsymmetricStacks` | Added the missing `prominent: bool` field (already assumed by section 8.3 prose and by the 13.1 `depth_bucket_labels_per_prefix` test). |
| S12 | 13.1 `replay_bayes_two_combos` (T3) | Replaced "posterior (0.9, 0.1) for every seat including hero" with section 8.4's wording: the one-element branch posterior is `pi_{S,0}[c] == 1` for every seat; the actor's normalized combo distribution is `(0.9, 0.1)`; an unacted seat (hero) stays uniform. |
| S13 | 9.2, board-blocking bullet | Added: board blocking is applied to each seat's output marginal at the street root, never to per-branch masses, so section 8.4's equal-total invariant and the seat-independent residual share are preserved. |
| S14 | 9.2, case 3 of the postflop per-street walk | Added: a branch whose mapped continuation is not covered freezes navigation for that branch for the remainder of the street; later observed actions in other branches are unaffected. |
| S15 | 3.5 `engine` row | `set_config` now returns `Result<config_revision, EngineError>`; added `set_hero_cards([Card; 2]) -> Result<HandState, EngineError>`; `shutdown()` -> `shutdown(&mut self)`. |
| S16 | 4.3, data-model code block | Added `pub struct BeginHand { pub button: Seat, pub hero: Seat, pub dealt: Vec<Seat>, pub stacks: Vec<u32>, pub hero_cards: Option<[Card; 2]> }` as the section-5-step-2 admission DTO, with a comment distinguishing it from `core-model`'s internal input (which additionally carries the engine-assigned `hand_id`). |
| S17 | 13.0 fixture inventory | Noted, on the `fixtures/eval/phevaluator_5card.bin`, 7-card samples row, that only 200,000 samples are committed and the 10,000,000-sample 7-card oracle is locally generated and gitignored (`--features exhaustive` only). |

## Ambiguous items — narrowest reading taken

- **S6**: "add after `Derived`" could mean an inline comment inside the `Derived` struct or a prose note. Since the
  content (full sentences about per-seat vector length/indexing and about `HandState.stacks_start`, which lives on
  a different struct) does not fit as a Rust comment, it was added as a new prose paragraph immediately after the
  section-4.3 code block closes, before the existing "Dealt seats (spec decision)" paragraph.
- **S10**: "split into (a) ... (b) ..." could mean splitting into two separate table rows (with a new test name for
  the wire-mismatch half) or labelling the two halves within the existing row. To avoid inventing an unspecified
  test name/spot for the wire-mismatch half, the existing single row was kept and its assertion column was
  labelled "(a) In-process equality over the 47 boundary cases" / "(b) Wire `tree_mismatch` cases", with no content
  removed or changed.
- **S3**: applied to both cited locations (3.5's `core-eval` row and 7's equity row), replacing the equivalent
  ad hoc `pairs * runouts <= 2 * 10^7` phrasing in section 7 with the newly named `exact_cost(&req) <= 2 * 10^7`
  so the two sections state one gating rule through one named function.

No item was left unclear enough to warrant a TBD; all 17 accepted items plus the section 13.2 fixture correction
are fully applied.
