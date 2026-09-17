---
type: log
status: current
date: 2026-09-17
supersedes: none
related:
  - ../superpowers/plans/2026-09-10-plan-3-preflop-replay.md
  - ../superpowers/specs/2026-09-10-pokerai-assistant-design.md
---

# Chart transcription record

Source URLs/hashes, grid-by-grid transcription and verification record for the
`ChartTranscription` preflop bundles (plan 3, spec section 8.2). This file is populated
incrementally, one section per task (plan 3 Tasks 4, 5, 6, 7, and later 19 for the goldens
audit): each task appends to or fills in its own section below and never rewrites another
task's completed entries. Nothing here is deleted once written; a correction is recorded as a
new dated note under the relevant section, not by editing the original entry away.

## Tooling (Task 3)

`tools/chart_ingest.py` is the deterministic ingestion and validation tool every later section
below is produced with. It implements:

- `class_names() -> list[str]` -- the 169 class names in the fixed row-major order (`class_order
  = "A-2 row-major, section 4.1"`).
- `build(transcription: dict) -> dict` -- converts one hand transcription (13x13 grid rows, a
  legend mapping each cell code to an action-probability vector, and per-node metadata) into the
  normalized, dense, action-major `Envelope` dict that `core_preflop::decode` (Task 1) consumes.
  No nearest-hand substitution; charts never carry an `evs` key; a legend code's meaning always
  comes from the `legend` mapping itself, never inferred from its name.
- `validate(envelope: dict) -> None` -- every structural and numeric rule spec section 8.2
  states (shape, token/position/amount validity, uniqueness, the per-class sibling-sum tolerance
  `1 +- 1e-3`, the unreachable-class exact-zero rule, no chart EV). This is a second,
  independent implementation of Task 1's Rust rules; the Rust loader (`core_preflop::validate`)
  remains the final boundary validator regardless of what this tool accepts.
- `manifest_for_chart(envelope, transcription) -> dict` -- the matching `BundleInfo` manifest,
  forcing `source = "ChartTranscription"` and `ev_reference = "unverified"`. Its emitted key set
  is checked against the Rust `BundleInfo` struct's actual fifteen field names
  (`tools/tests/test_chart_ingest.py`), so a field added on one side without the other fails the
  Python suite before it ever reaches `load_bundle`.

CLI (`python tools/chart_ingest.py <command> ...`):

- `fetch URL OUTPUT` -- downloads exactly the given URL (never a guessed or synthesized
  alternative) to `OUTPUT`, bounded to 64 MiB, and refuses HTML returned in place of a PDF.
  Prints the final redirected URL, byte count and SHA-256 -- the values a later task's
  acquisition entry below records.
- `build TRANSCRIPTION OUTPUT MANIFEST` -- writes `build(transcription)` to `OUTPUT` and
  `manifest_for_chart(envelope, transcription)` to `MANIFEST`, hashing exactly the bytes it
  writes.
- `validate ENVELOPE` -- validates and prints every node's 169 class sums plus the aggregate
  minimum/maximum; exits nonzero on any rule violation.
- `verify TRANSCRIPTION OUTPUT MANIFEST` -- rebuilds from the transcription and compares the
  rebuilt bytes and manifest hash against what is already committed on disk exactly, and
  compares the transcription's `covered` inventory keys against the envelope's actual node
  keys. Reports the offending node and class name on any mismatch; never repairs automatically.

No chart source has been fetched and no grid has been transcribed yet -- this section records
the tool only. Task 4 begins the acquisition record below.

## Source acquisition (Task 4)

*Not started.*

## PokerCoaching 100bb transcription (Task 5)

*Not started.*

## RangeConverter 200bb transcription (Task 6)

*Not started.*

## Frozen coverage and Plan 4 handoff (Task 7)

*Not started.*

## Replay/bet-translation goldens audit (Task 19)

*Not started.*
