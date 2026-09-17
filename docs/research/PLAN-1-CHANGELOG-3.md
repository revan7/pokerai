# Plan 1 revision 3 — changelog (2026-09-17)

Plan: `docs/superpowers/plans/2026-09-10-plan-1-foundation.md`.
Input: `docs/research/REVIEW-cross-plan-3.md` section 5 (stale-reference grep disposition), which names "all five L13 spec-version labels" as an actionable leftover — plan 1's own header (L13) cites "(revision 6)" while the spec, as of `REVIEW-cross-plan-3.md`'s own review pass, was already revision 7. Section 3's **R7** supplies the replacement text ("P1–P5 current Spec headers, each L13: replace the current-version label 'revision 6' with 'revision 7'"). Per task B3h's binding instruction, "Plan 1 and plan 3 are not edited unless section 5 names a concrete leftover in them (then apply the same rule)" — section 5 does name this leftover in plan 1, so the same revision/changelog rule applies here even though plan 1 carries no other B3h edit.

Size before: unchanged (no task content touched). Resulting revision: **3**.

---

## 1. Edit applied

| # | Finding | Location | Disposition | What changed |
|---|---|---|---|---|
| §5/R7-1 | F22 — stale "revision 6" spec-version label | Header, L13 (`**Spec:**` line) | **APPLIED** | "…pokerai-assistant-design.md\` (revision 6), sections 2, 3.2, …" → "…(revision 7), sections 2, 3.2, …". |
| — | Title / revision line | Lines 1-3 | **APPLIED** | Added "Revision 3 (2026-09-17): verification edits R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-1-CHANGELOG-3.md" directly under the title, above the existing Revision 2 line (kept unchanged). |

## 2. Exact lines changed (before / after)

### Header L13

Before:
> **Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (revision 6), sections 2, 3.2, 3.5, 3.6, 3.7, 4.1-4.6, 10.2, 13.0, 13.1. Decisions: `docs/design/2026-09-10-design-outline.md` §0b. Library facts: `docs/research/R3-libraries.md`. Cross-plan interface resolutions: `docs/research/REVIEW-cross-plan.md` §1-§2.

After:
> **Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (revision 7), sections 2, 3.2, 3.5, 3.6, 3.7, 4.1-4.6, 10.2, 13.0, 13.1. Decisions: `docs/design/2026-09-10-design-outline.md` §0b. Library facts: `docs/research/R3-libraries.md`. Cross-plan interface resolutions: `docs/research/REVIEW-cross-plan.md` §1-§2.

### Title / revision line

Before:
> \# Plan 1: Foundation Implementation Plan
>
> Revision 2 (2026-09-17): seam re-check edits E01/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-1-CHANGELOG-2.md

After:
> \# Plan 1: Foundation Implementation Plan
>
> Revision 3 (2026-09-17): verification edits R7 from docs/research/REVIEW-cross-plan-3.md; changelog PLAN-1-CHANGELOG-3.md
>
> Revision 2 (2026-09-17): seam re-check edits E01/E06 from docs/research/REVIEW-cross-plan-2.md; changelog PLAN-1-CHANGELOG-2.md

## 3. Not changed

- No other R1-R7 item names plan 1. Section 5's other grep rows either found zero stale matches in plan 1 or
  named only plan 2/plan 4/plan 5 sites (see `REVIEW-cross-plan-3.md` section 5's full disposition table).
- No task, step, interface or test in plan 1 was touched — this is a single header-line correction.

## 4. Verification

Documentation-only task: Rust/Python/UI execution: not applicable — no workspace exists. Verification is the grep check named in the task: the old "(revision 6)" text at the header line is absent and the replacement "(revision 7)" is present. The quoted command output is recorded in the journal entry `docs/log/2026-09-17.md` and in the agent's report.
