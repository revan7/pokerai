# Spec changelog: revision 6 -> revision 7

Source: `docs/research/REVIEW-cross-plan-2.md`, finding **F20 — BLOCKER — Spec requires two wire outcomes for one
invalid donk input** (task B3a). F20 cited a contradiction between two section 13.2 assertions: `S:L733`
(`tree_materialization_matches_library`) claimed a `None` donk option produces `result{error{tree_mismatch}}`,
while `S:L744` (`protocol_rejections`) claimed the same input is a typed `ack{rejected}` rejection with no work,
worker stays alive. `P2.T8/Interfaces:L2028` noted plan 2 had already picked "the cheap answer" without the
spec resolving which wire outcome is authoritative.

Orchestrator decision (binding, given in the B3a task): a `None` donk option for a **later street** (turn or
river menus) is invalid input — the worker answers `ack{rejected, reason: "donk option missing for <street>"}`
with no work, worker stays alive, exactly like the other structurally invalid inputs of section 4.5. A `None`
donk option at the **root street** is legal (the root player cannot face a donk node) and is materialized as an
empty donk menu. `result{error{tree_mismatch}}` is reserved for a realized library tree that differs from
`tree.materialized` (altered materialization, missing or extra terminal markers).

Spec edited in place: `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (now revision 7, 809 lines).

| # | Section touched | Edit applied |
|---|---|---|
| F20-1 | Title / revision line | Bumped `revision 6` -> `revision 7` in the title; added a new revision line "Revision 7 (2026-09-17): F20 donk-option rule from docs/research/REVIEW-cross-plan-2.md; changelog SPEC-CHANGELOG-6.md." directly under the title, above the existing Revision 6 line (kept unchanged, not replaced). |
| F20-2 | 4.6, donk node bullet (after the `oop_call_flag` paragraph, following the addendum A.1 sentence) | Added: "**Donk-option legality (spec decision)**: a `None` donk option is legal only at `root_street`, where the root player never faces a donk node, so it is harmless and materializes as an empty donk menu; the same `None` for a later street (turn or river) is invalid input, not a tree mismatch — the worker answers `ack{rejected, reason: "donk option missing for <street>"}` with no work and stays alive (section 4.5), never `result{error{tree_mismatch}}`." |
| F20-3 | 13.2 `tree_materialization_matches_library`, assertion (b) | Removed the `None` donk option from the `tree_mismatch` list (previously grouped with the altered-materialization and missing-terminal-marker cases) and added the cross-reference: a `None` donk option for a later street is invalid input, not a tree mismatch (section 4.6 spec decision) — it is `protocol_rejections`' typed `ack{rejected}` case, never `result{error{tree_mismatch}}`. The missing-terminal-marker case is unchanged and still produces `tree_mismatch`, alongside the deliberately-altered-`materialized`-entry case. |

## Exact lines changed (before / after)

### F20-2 — section 4.6, donk node bullet (spec line ~358)

Before (trailing clause of the bullet):
> `None` is never sent because it makes the library substitute the ordinary bet sizes (addendum A.1, measured: 6.8 GB versus 5.3 GB on one spot). When IP closed the street by calling, or the street ended check-check, OOP's first node of the next street is an ordinary opening node with the bet menu.

After:
> `None` is never sent because it makes the library substitute the ordinary bet sizes (addendum A.1, measured: 6.8 GB versus 5.3 GB on one spot). When IP closed the street by calling, or the street ended check-check, OOP's first node of the next street is an ordinary opening node with the bet menu. **Donk-option legality (spec decision)**: a `None` donk option is legal only at `root_street`, where the root player never faces a donk node, so it is harmless and materializes as an empty donk menu; the same `None` for a later street (turn or river) is invalid input, not a tree mismatch — the worker answers `ack{rejected, reason: "donk option missing for <street>"}` with no work and stays alive (section 4.5), never `result{error{tree_mismatch}}`.

### F20-3 — section 13.2, `tree_materialization_matches_library` assertion (b) (spec line ~735)

Before:
> **(b) Wire `tree_mismatch` cases**: a materialized list built with `matched` reset per street produces `result{error{tree_mismatch}}`. A deliberately altered `materialized` entry, a `None` donk option and a missing terminal marker each produce `result{error{tree_mismatch}}`

After:
> **(b) Wire `tree_mismatch` cases**: a materialized list built with `matched` reset per street produces `result{error{tree_mismatch}}`. A deliberately altered `materialized` entry and a missing terminal marker each produce `result{error{tree_mismatch}}`; a `None` donk option for a later street is invalid input, not a tree mismatch (section 4.6 spec decision) — it is `protocol_rejections`' typed `ack{rejected}` case below, never `result{error{tree_mismatch}}`

### F20-1 — title and revision line (spec lines 1-5)

Before:
> \# PokerAI assistant: design specification (2026-09-10, revision 6)
>
> Revision 6 (2026-09-10): amendments S1-S17 from docs/research/REVIEW-cross-plan.md section 3, plus the section 13.2 fixture correction.

After:
> \# PokerAI assistant: design specification (2026-09-10, revision 7)
>
> Revision 7 (2026-09-17): F20 donk-option rule from docs/research/REVIEW-cross-plan-2.md; changelog SPEC-CHANGELOG-6.md.
>
> Revision 6 (2026-09-10): amendments S1-S17 from docs/research/REVIEW-cross-plan.md section 3, plus the section 13.2 fixture correction.

## Not changed

- Section 4.5's generic "Structurally invalid input" rejection list (`ack{rejected, reason}` for messages
  without work) is unchanged — a `None` donk option for a later street is one instance of that existing category,
  not a new entry requiring its own bullet.
- Section 13.2's `protocol_rejections` row (`S:L744`, unchanged) already stated the correct outcome ("`None`
  donk option in `tree`: typed rejection with `reason`, no work, worker stays alive") and needed no edit; F20's
  contradiction was resolved by correcting `tree_materialization_matches_library` (F20-3) to stop claiming the
  same input also produces `tree_mismatch`.
- No other section, row or fixture list was touched.
