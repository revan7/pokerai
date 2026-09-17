# Spec changelog: revision 7 -> revision 8

Source: `docs/research/REVIEW-cross-plan-3.md`, section 3, **R1 — MINOR — F20 precision** (task B3h). R1 found that
section 13.2's `protocol_rejections` row (`S:L746`) still said "`None` donk option in `tree`: typed rejection with
`reason`, no work, worker stays alive", omitting the root-street exception that section 4.6 and `S:L746`'s own
sibling assertion in `tree_materialization_matches_library` (`S:L735`) already state: a `None` donk option is legal
only at `root_street`; the same `None` for a later street is the invalid input that produces the typed rejection.
The un-qualified row fragment read as if *any* `None` donk option (including at the root) were rejected, which
contradicts `S§4.6:L358` and the accepted-root-`None` wire test in Plan 2 Task 13 (`ack_of(&w, "6")["status"] ==
"accepted"`).

Spec edited in place: `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (now revision 8).

| # | Section touched | Edit applied |
|---|---|---|
| R1-1 | Title / revision line | Bumped `revision 7` -> `revision 8` in the title; added a new revision line "Revision 8 (2026-09-17): R1 from docs/research/REVIEW-cross-plan-3.md; changelog SPEC-CHANGELOG-7.md." directly under the title, above the existing Revision 7 line (kept unchanged, not replaced). |
| R1-2 | 13.2, `protocol_rejections` row (`S:L746`) | Replaced the row fragment "`None` donk option in `tree`: typed rejection with `reason`, no work, worker stays alive" with "a `None` donk option on a street strictly after `root_street`: typed `ack{rejected}` with `reason`, no work, worker stays alive; root-street `None` is accepted", rendering every identifier as code per R1's instruction. |

## Exact lines changed (before / after)

### R1-2 — section 13.2, `protocol_rejections` row (spec line 746)

Before (row fragment, in context):
> …lock row summing to 0.4, lock row `[-0.1, 1.1]` (sum 1, out of `[0, 1]`), `None` donk option in `tree`: typed rejection with `reason`, no work, worker stays alive; an all-zero lock row is accepted as the free-combo sentinel; …

After:
> …lock row summing to 0.4, lock row `[-0.1, 1.1]` (sum 1, out of `[0, 1]`), a `None` donk option on a street strictly after `root_street`: typed `ack{rejected}` with `reason`, no work, worker stays alive; root-street `None` is accepted; an all-zero lock row is accepted as the free-combo sentinel; …

### R1-1 — title and revision line (spec lines 1-5)

Before:
> \# PokerAI assistant: design specification (2026-09-10, revision 7)
>
> Revision 7 (2026-09-17): F20 donk-option rule from docs/research/REVIEW-cross-plan-2.md; changelog SPEC-CHANGELOG-6.md.
>
> Revision 6 (2026-09-10): amendments S1-S17 from docs/research/REVIEW-cross-plan.md section 3, plus the section 13.2 fixture correction.

After:
> \# PokerAI assistant: design specification (2026-09-10, revision 8)
>
> Revision 8 (2026-09-17): R1 from docs/research/REVIEW-cross-plan-3.md; changelog SPEC-CHANGELOG-7.md.
>
> Revision 7 (2026-09-17): F20 donk-option rule from docs/research/REVIEW-cross-plan-2.md; changelog SPEC-CHANGELOG-6.md.
>
> Revision 6 (2026-09-10): amendments S1-S17 from docs/research/REVIEW-cross-plan.md section 3, plus the section 13.2 fixture correction.

## Not changed

- Section 4.6's donk-option-legality bullet (`S:L358`, added by `SPEC-CHANGELOG-6.md`) already states the root
  exception correctly and needed no edit; R1's fix brings section 13.2's fixture row into agreement with it.
- Section 13.2's `tree_materialization_matches_library` assertion (b) (`S:L735`, added by `SPEC-CHANGELOG-6.md`)
  already correctly excludes a `None` donk option from the `tree_mismatch` list and needed no edit.
- No other section, row or fixture list was touched.
