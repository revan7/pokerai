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

Both public chart sources named in the plan were fetched with `chart_ingest.py fetch`
(never a guessed or substituted URL). `fixtures/charts/sources.manifest.json` records the
machine-readable availability row for each depth; both are `"available"` for this build. Raw
per-attempt values below match that manifest exactly.

### Depth 100bb -- PokerCoaching (`pokercoaching_100`)

- Acquired: 2026-09-17T19:44:42Z.
- Fetched URL: `https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf`.
- Final URL (as reported by `fetch`, no redirect): same as above.
- Bytes: 2055584. SHA-256: `f5686c6dd672c782249752c60cd4ff5850378271e2139abc2b15e690149d091b`.
- Committed unmodified at `fixtures/charts/sources/pokercoaching_100.pdf` (`fetch` confirmed the
  `%PDF-` magic; hash above is of the exact committed bytes).
- Document title/version: the PDF's Info dictionary carries no `/Title` entry; `/Creator` and
  `/Producer` are both `Google` (a Google Slides export), `/CreationDate`
  `D:20240409144131-06'00'` (2024-04-09). The file is referred to by its URL slug,
  `online-6max-gto-charts.pdf`.
- Page count: **6** -- the compressed object streams (`FlateDecode`) decompress to exactly one
  `/Type /Pages` tree node and exactly six `/Type /Page` leaf objects (counted with a
  Type-vs-Type-plural-safe match, i.e. `/Type /Page` not immediately followed by another letter,
  so it cannot also match `/Type /Pages`), matching the plan's planning-time verification
  ("PokerCoaching is a six-page PDF").
- Action-size legend, rake disclosure, rounding rule: **not extracted at acquisition time**.
  This PDF embeds a subsetted font with no `/ToUnicode` CMap, so its text-show operators decode
  to glyph IDs, not Unicode text -- automated extraction is unreliable and this task does not
  attempt to eyeball-transcribe chart content (that is Task 5's job, reading the chart
  cell-by-cell). No rendering/OCR tool was available in this environment and none was installed
  to stay inside this task's Files list. Task 5 records the legend, rake disclosure and rounding
  rule as part of its own grid-by-grid transcription of this same file.

### Depth 200bb -- RangeConverter (`rangeconverter_200`)

Planning-time verification (2026-09-10) recorded RangeConverter's article page as accessible but
its PDF download as timing out, so the fallback chain was attempted in order:

1. **3a, article page** -- `fetch https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem`
   at 2026-09-17T19:46:50Z: **ok**, 16584 bytes, no retry needed.
2. **3b, publisher download page** -- `fetch https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash`
   at 2026-09-17T19:46:59Z: **ok**, 5169125 bytes, no retry needed. Unlike the plan's anticipated
   shape (an HTML page carrying a further download link), this URL's response body was itself
   the PDF (`fetch` reported it as the final URL with no redirect; the bytes start `%PDF-1.4`
   and its Info dictionary's `/Title` is `6-Max 200BB Poker Charts - No Limit Texas Holdem Cash`,
   `/Producer` `Skia/PDF m117 Google Docs Renderer`) -- there was no separate anchor to follow.
3. **3c, the "observed PDF url"** -- since step 3b's own URL already served the PDF, step 3c
   re-fetched that identical URL (never a guessed S3 path, never a different depth or publisher)
   with a `.pdf` output name so `fetch`'s PDF-magic check ran: at 2026-09-17T19:48:50Z, **ok**,
   5169125 bytes, SHA-256 `f0797be2a9894c520af90e70c17e2b7dee208215eab58b6ae953e8cc81bf70e2` --
   identical to step 3b's bytes, confirming the same content. No web-archive fallback was needed.
- Committed at `fixtures/charts/sources/rangeconverter_200.pdf` (the 3c fetch; unmodified,
  hash above is of the exact committed bytes) and `fixtures/charts/sources/rangeconverter_200.html`
  (the 3a article page, the provenance record, committed regardless per the plan).
- Document title/version: PDF `/Title` `6-Max 200BB Poker Charts - No Limit Texas Holdem Cash`,
  `/Producer` `Skia/PDF m117 Google Docs Renderer`, no `/CreationDate`/`/ModDate` in the Info
  dictionary. The article page's `<title>` is `6 max 200bb Poker Charts No Limit Texas Holdem`.
- Page count: the PDF's root `/Type /Pages` object declares `/Count 13` with two `/Kids` groups
  of 8 and 5 pages (`8 + 5 = 13`), confirmed against the raw object dictionaries.
- The committed `rangeconverter_200.html` had its per-request session artifacts redacted before
  commit (the article page carries no chart data itself, only provenance, so this does not
  affect any hash the manifest tracks -- the manifest's depth-200 `source_file`/`sha256` point
  at the untouched `rangeconverter_200.pdf`, not at this file): the Rails CSRF `<meta
  name="csrf-token" content="...">` value and the page's hidden-form `authenticity_token` input
  value were each replaced with the literal `[stripped]`. No other session, account or cookie
  value (email, user id, remember-token, session id) was found in the page; the page's
  `data-pw-auth` attribute and empty `profitwell('start', {'user_id': ""})` call are the
  publisher's own site-wide analytics snippet (the same for every visitor), not session/account
  information about this fetch, and were left as-is.
- Action-size legend, rake disclosure, rounding rule: **not extracted at acquisition time**, for
  the same reason as the depth-100 source above -- this is a large (13-page, 5 MB) vector/image
  chart PDF and reliable automated text extraction was not available in this environment. Task 6
  records these as part of its own grid-by-grid transcription of this file.

### Release availability given the above

Both depths acquired successfully; `fixtures/charts/sources.manifest.json` marks `100` and `200`
both `"available"`. Nothing here recorded an `"unsupported"` depth.

## PokerCoaching 100bb transcription (Task 5)

*Not started.*

## RangeConverter 200bb transcription (Task 6)

*Not started.*

## Frozen coverage and Plan 4 handoff (Task 7)

*Not started.*

## Replay/bet-translation goldens audit (Task 19)

*Not started.*
