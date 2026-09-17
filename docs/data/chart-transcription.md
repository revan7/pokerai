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
- Action-size legend, rake disclosure, rounding rule (fix round 1, R1): the environment has no
  `pdftoppm`/poppler, so pages cannot be rendered as images; the PDF's font (obj 19,
  `MUFUZY+ArialMT`) does carry a real `/ToUnicode` CMap (obj 86) once the compressed cross-
  reference streams are decompressed -- this was missed on first pass because a raw byte-grep
  for `/ToUnicode` cannot see a reference that is itself compressed inside this PDF's object
  streams, and because the page-1 text uses literal `(...)` string operands, not the `<...>`
  hex-string form the first extraction attempt handled. Re-decoded correctly (2-byte
  big-endian codes through the real CMap; verified by spelling out `"Instructions"` as the
  first decoded word). Page 1 (PDF object 15) is titled "Instructions" and states, verbatim:
  - **Action-size legend:** "When 3-betting from in position a 3.5x raise size is used. When
    3-betting from out of position a 4x raise sizing is used. When in the big blind, facing a
    small blind limp, a 3.5x raise size is used. When 4-betting from out of position a 2.5x
    raise size is used. When 4-betting from in position a 2.3x raise size is used." ... "Bet
    Sizing: The RFI ranges assume a 2.5bb raise from every position except for the small
    blind. The small blind RFI assumes a 3bb raise size."
  - **Rounding rule:** these are called "Implementable GTO Charts" because "there are many
    instances where the GTO strategy suggests playing a specific hand some portion of the
    time, perhaps 33%. In the Implementable GTO Charts, if there are three hands, such as
    Q-6o, Q-5o and Q-4o, that each get played 33% of the time, only one is played, making the
    strategy much easier to implement while only sacrificing a tiny bit of equity." This is a
    *dominant-action* simplification (collapse a near-tied mix to one deterministic action per
    hand), not RangeConverter's "round every frequency to the nearest 50%" rule below -- the
    two sources use different simplification conventions and Task 5/6 must not conflate them.
  - **Rake disclosure:** no statement on rake anywhere in this document (pages 1-6 inspected:
    page 1 is the only text-bearing page per the object scan below; the case-insensitive
    string `rake` occurs zero times anywhere in the file, checked both in the raw bytes and
    across every decompressed stream, i.e. including page content that is itself compressed).
  - Method and scope: `chart_ingest.py` does not do PDF parsing, so this was done by a
    throwaway extraction script (not committed; not part of this task's Files list), built for
    exactly these two files: it locates each top-level `N 0 obj`/`endobj`, inflates
    `FlateDecode` streams, parses the font's `/ToUnicode` `beginbfchar`/`beginbfrange` CMap,
    and decodes each `Tj`/`TJ` text-show operand (both the `<hex>` and `(literal)` string
    forms) through that map. Pages 2-6 (PDF objects 21, 31, 42, 55, 96) were also decoded: page
    2 is "Raise First In (RFI)" plus the five position labels (Lojack/Hijack/Button/Small
    Blind/Cutoff), page 3 "Facing RFI: In Position", page 4 "Facing RFI: Out of Position", page
    5 "Blind vs Blind" plus its three sub-headers -- these are page/section titles only. Page 6
    (object 96) places one full-page raster image (`/Im0 Do`) and carries no text at all; if it
    holds a color-swatch legend, that content is only visible by rendering the image, which
    this environment's Read tool cannot do here (`pdftoppm is not installed`, confirmed when
    attempting `Read` with a `pages` argument on this file). The 13x13 hand grids themselves
    (the colored cells) are drawn as vector paths on pages 2-5, not as text, and were not
    transcribed here -- that remains Task 5's grid-by-grid job.

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
  commit (the article page carries no chart data itself, only provenance -- the manifest's
  depth-200 `source_file`/`sha256` still point at the untouched `rangeconverter_200.pdf`, never
  at this file): the Rails CSRF `<meta name="csrf-token" content="...">` value and the page's
  hidden-form `authenticity_token` input value were each replaced with the literal
  `[stripped]`. No other session, account or cookie value (email, user id, remember-token,
  session id) was found in the page; the page's `data-pw-auth` attribute and empty
  `profitwell('start', {'user_id': ""})` call are the publisher's own site-wide analytics
  snippet (the same for every visitor), not session/account information about this fetch, and
  were left as-is. **(fix round 1, R3)** The redacted file's own integrity is now pinned
  separately, in `fixtures/charts/sources.manifest.json`'s top-level `provenance_artifacts`
  array (`artifact_id: "rangeconverter_200_article"`): it records the original `fetch` values
  (`fetched_bytes: 16584`, `fetched_sha256`
  `33b9007cf6318b347a5ade89d6e2ba099b126dc86207a45dbd24f9afeda3a76d`) separately from the
  committed, post-redaction values (`committed_bytes: 16432`, `committed_sha256`
  `f9cba1810f1486b320934f15a07be59ab62d28d860f39fa04586d667b7506c7c`), plus the redaction note
  above and the fetch URL/time. `tools/tests/test_chart_ingest.py`'s
  `test_sources_manifest_provenance_artifacts_present_and_integrity_checked` and
  `test_rangeconverter_article_provenance_record_matches_the_acquisition_report` read the
  committed file's actual bytes and fail if either the file goes missing or its hash drifts
  from `committed_sha256`.
- Action-size legend, rake disclosure, rounding rule (fix round 1, R1): this PDF's two fonts
  (objs 22/23, `Arial-BoldMT`/`ArialMT`) do have real `/ToUnicode` CMaps (objs 52/54, 42+61
  entries), decoded with the same method as the depth-100 source above. Every one of the 13
  pages' content streams was checked. Pages 1-2 (PDF objects 24, 28) carry the article's
  intro/methodology text (partly duplicated across the two objects, i.e. the same running
  paragraph); pages 3-13 (objects 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50) each decode to
  *only* their own short title (e.g. "Raise First In (RFI) - 6max 200bb Ranges", "MP vs RFI -
  6max 200bb Ranges", ... "SB RFI vs 3bet - 6max 200bb Ranges") -- confirmed by an operator
  count on each page's stream (a handful of `Tj` calls per page, matching one title's worth of
  glyphs) plus the fact that this document embeds 22 full-page raster images at 2048x1313 px
  (`/Subtype /Image`; one per grid page) rendered under those titles via `Do`, alongside a
  23rd smaller (1200x407) image on the intro page. The grid cells themselves (hand notation,
  frequencies, any color-swatch/size legend) are therefore pixels, not text, on every one of
  pages 3-13, and reading them needs page rendering, which this environment's Read tool cannot
  do here (`pdftoppm is not installed`, confirmed when attempting `Read` with a `pages`
  argument on this file).
  - **Rounding rule (pages 1-2 text, found):** "Each poker hand chart has been simplified so
    that the frequency of an action for each hand combo is rounded to the nearest 50%. If you
    see a hand with a mix of colours, it means you should take one action half the time and
    the other action half the time. You can either use a randomizer (e.g. one action if high
    card first, other action if small card first) or base your action on your reads of your
    opponent, e.g. if they play too tightly or too loosely." This is RangeConverter's own
    stated rule and differs from PokerCoaching's dominant-action rule above -- the two chart
    sets round differently and must be transcribed (Task 5/6) against their own stated rule,
    never against each other's.
  - **Action-size legend:** no statement in the extractable text (pages 1-2); the only mention
    of a specific number in that text is the frequency-rounding example above (33%, 72%,
    "nearest 50%"), not a raise-size legend. Any bet-size legend that exists is inside the
    raster grid images on pages 3-13 and was not visually read (see above).
  - **Rake disclosure:** no statement on rake anywhere in this document. The case-insensitive
    string `rake` occurs zero times in the raw file bytes (checked directly; this PDF, unlike
    the depth-100 source, is not further compressed at the cross-reference level, so a raw
    byte scan is sufficient here).
  - Method: the same throwaway, task-local extraction script as the depth-100 source (not
    committed; not part of this task's Files list) -- inflate `FlateDecode` streams, parse the
    `/ToUnicode` CMaps, decode every `Tj`/`TJ` operand (both `<hex>` and `(literal)` string
    forms) through them.

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
