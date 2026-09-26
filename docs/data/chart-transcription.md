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

Completed 2026-09-26. Source: the committed `fixtures/charts/sources/pokercoaching_100.pdf`
(SHA-256 `f5686c6d...149d091b`, as recorded under Task 4). Outputs:
`fixtures/charts/transcription/pokercoaching_100.json` (22 transcribed grids and a 45-row
inventory), the built envelope `fixtures/charts/pokercoaching_100.json` (22 nodes, 3718
classes, SHA-256 `97670f854018a17f6c9e1bcc851ec1a24a3e29492633aa25f60bf6ed19c4fa86`) and its
manifest `fixtures/charts/pokercoaching_100.manifest.json` (`source = "ChartTranscription"`,
`ev_reference = "unverified"`, `rake_profile = "undocumented"`, `rake = null`, `accuracy =
"unverified"`, no `evs` anywhere).

### Page numbering and page content (correction note, 2026-09-26)

This section numbers pages in **physical PDF page order**, which is the order every PDF viewer
and PDFium use. Task 4 above numbered pages by PDF object order, and so did the plan's Task 5
checklist. The two orders differ:

| Physical page | Content | Task 4 / plan numbering |
|---|---|---|
| 1 | Cover: one full-page photo collage (1920x1483 JPEG), with a promotional title over a tilted, partly hidden fragment of a colour grid. It has no chart title, no legend and no complete grid, so it produces no node. | "page 6 (object 96)": the page with no extracted text |
| 2 | "Instructions": text only. The bet-sizing paragraph and the "Implementable" rounding rule that Task 4 decoded; the rendered page matches that decoded text. | "page 1" |
| 3 | "Raise First In (RFI)": 5 grids (Lojack, Hijack, Cutoff, Button, Small Blind) | "page 2" |
| 4 | "Facing RFI: In Position": 6 grids (HJ vs LJ, CO vs LJ, CO vs HJ, BTN vs LJ, BTN vs HJ, BTN vs CO) | "page 3" |
| 5 | "Facing RFI: Out of Position": 8 grids (SB vs LJ/HJ/CO/BTN, BB vs LJ/HJ/CO/BTN) | "page 4" |
| 6 | "Blind vs Blind": 3 grids (Small Blind Strategy, Big Blind vs SB Limp, Big Blind vs SB raise) | "page 5" |

A second correction to Task 4: the 13x13 grids are not vector paths. Each grid, together with
its legend panel, is one embedded JPEG (`DCTDecode`, about 400x480 px, 30 px cell pitch), so
there are 22 grid images on pages 3-6. The chart titles are the only text objects on those
pages. Each title was matched to the image directly beneath it by page coordinates: every
title's centre lies over its image's centre.

### How the pages were read

- **Renderer (a deviation from the Files list, see the report):** `tools/chart_render.py`,
  using `pypdfium2==5.13.0` (PDFium 153.0.7999.0), which is now pinned in
  `tools/pyproject.toml` and `tools/requirements.txt`. PNGs are written by a stdlib encoder, and
  the output is deterministic (tested). All renders went to the session scratch directory
  outside the repository and none is committed. Renders made: all six pages at 100 DPI, for
  layout and titles; each of the 22 grid image regions cropped at 400 DPI, for pass 1; the same
  regions at 300 DPI, a second rendering, for pass 2; and each embedded JPEG decoded at native
  resolution, for pass 3.
- **Orientation check:** every cell prints its own hand label, for example `AKs` at row 0
  column 1 and `AKo` at row 1 column 0. So row r, column c is class `r*13+c` of `class_names()`:
  pairs on the diagonal, suited above it, offsuit below it, with the high rank first in both.
  This was confirmed on every grid before any cell was transcribed.
- **Pass 1** (400 DPI, top row first) was the transcription.
- **Pass 2** (300 DPI, read in reverse row order from the 22s row up to the A row) was the
  reread. It was recorded separately and then compared by script. Limitation, disclosed: a
  single agent performed both visual passes, so pass 1 was not hidden from pass 2 in the strict
  sense the brief intends. Pass 3 is the independent read.
- **Pass 3, a pixel classifier:** a throwaway script that is not committed. It finds each
  image's 30 px cell lattice from the bright 2 px gaps between cells. For each cell it takes the
  dominant quantized fill colour of the 22x22 px interior, ignoring the minority text pixels.
  It flags any cell whose interior holds a second fill colour covering at least 15% of that
  interior. Its output was written to a file and not viewed until pass 1 had been recorded.
- **Result:** all three reads agree on **3718/3718 cells** (22 grids x 169). The classifier
  flagged **0** cells as possibly mixed and found **0** unknown colours. Every grid's per-action
  combo totals equal the combo counts printed in its own legend panel (table below). There are
  no mixed cells: every cell is one pure colour, which fits page 2's rule of playing one
  dominant action per hand. Every weight is therefore exactly `0.0` or `1.0`.
- **Pass-1 recording slips:** 5 row strings of the page-6 Small Blind Strategy grid (rows A, K,
  Q, T and 8) were mistyped while being written down, not misread. They were caught on
  self-review before any comparison ran. No cell needed a correction after comparison.

### Legend as read from the raster pages

Each grid image has its own legend panel. It has columns "Action" and "Hands", and each row
shows: action label | % of all 1326 combos | combos / denominator | % of played combos.

- **Colours:** red = Raise (RFI and BB vs SB limp), 3Bet or 3bet (facing RFI, BB vs SB raise),
  or Raise/4bet (SB strategy). Blue = Limp (SB RFI), Call, Check (BB vs SB limp), or Raise/Call
  (SB strategy). Green = Raise/Fold. Dark grey = Limp/Raise. Orange = Limp/Call. Pink =
  Limp/Fold. White (uncoloured) = the legend's own "Fold" row in every chart.
- **Pair borders:** a dark border on the diagonal pair cells marks the pairs. It is not an
  action.
- **No bet sizes:** the raster legends carry none. Sizes come only from the page-2 bet-sizing
  paragraph that Task 4 decoded, which the page-2 render confirms. RFI is 2.5bb, SB RFI 3bb,
  in-position 3bet 3.5x, out-of-position 3bet 4x, BB raise facing an SB limp 3.5x, out-of-
  position 4bet 2.5x, in-position 4bet 2.3x.
- **Rake:** no raster page mentions rake.

Published legend rows, read from each image. Combo counts are what the tests pin through
`published_combos`:

| Page | Chart | Legend rows (label: % of all, combos) |
|---|---|---|
| 3 | Lojack | Raise 17.0% 226/226; Fold 83.0% 1100/1326 |
| 3 | Hijack | Raise 21.4% 284/284; Fold 78.6% 1042/1326 |
| 3 | Cutoff | Raise 27.8% 368/368; Fold 72.2% 958/1326 |
| 3 | Button | Raise 43.3% 574/574; Fold 56.7% 752/1326 |
| 3 | Small Blind | Raise 24.3% 322/826; Limp 38.0% 504/826; Fold 37.7% 500/1326 |
| 4 | HJ vs LJ RFI | 3Bet 8.1% 108/108; Fold 91.9% 1218/1326 |
| 4 | CO vs LJ RFI | 3Bet 8.6% 114/114; Fold 91.4% 1212/1326 |
| 4 | CO vs HJ RFI | 3Bet 10.1% 134/134; Fold 89.9% 1192/1326 |
| 4 | BTN vs LJ RFI | 3Bet 7.2% 96/188; Call 6.9% 92/188; Fold 85.8% 1138/1326 |
| 4 | BTN vs HJ RFI | 3Bet 8.9% 118/202; Call 6.3% 84/202; Fold 84.8% 1124/1326 |
| 4 | BTN vs CO RFI | 3Bet 12.1% 160/232; Call 5.4% 72/232; Fold 82.5% 1094/1326 |
| 5 | SB vs LJ RFI | 3Bet 7.2% 96/96; Fold 92.8% 1230/1326 |
| 5 | SB vs HJ RFI | 3Bet 8.7% 116/116; Fold 91.3% 1210/1326 |
| 5 | SB vs CO RFI | 3Bet 11.0% 146/146; Fold 89.0% 1180/1326 |
| 5 | SB vs BTN RFI | 3Bet 15.1% 200/200; Fold 84.9% 1126/1326 |
| 5 | BB vs LJ RFI | 3Bet 5.7% 76/382; Call 23.1% 306/382; Fold 71.2% 944/1326 |
| 5 | BB vs HJ RFI | 3Bet 7.4% 98/418; Call 24.1% 320/418; Fold 68.5% 908/1326 |
| 5 | BB vs CO RFI | 3Bet 9.7% 128/470; Call 25.8% 342/470; Fold 64.6% 856/1326 |
| 5 | BB vs BTN RFI | 3Bet 13.4% 178/754; Call 43.4% 576/754; Fold 43.1% 572/1326 |
| 6 | Small Blind Strategy | Raise/4bet 4.4% 58/826; Raise/Call 9.0% 120/826; Raise/Fold 10.9% 144/826; Limp/Raise 5.1% 68/826; Limp/Call 15.4% 204/826; Limp/Fold 17.5% 232/826; Fold 37.7% 500/1326 |
| 6 | Big Blind vs SB Limp | Raise 40.4% 536/1326; Check 59.6% 790/1326; Fold 0.0% 0/1326 |
| 6 | Big Blind vs SB raise | 3bet 16.4% 218/858; Call 48.3% 640/858; Fold 35.3% 468/1326 |

### Menus, resolved sizes and judgement calls

Sizes are frozen as raise-to values in `to_bb_x1000`, never as labels.

- **Opens:** 2500 (2.5bb), and 3000 for the SB.
- **3bets:** in position (HJ/CO/BTN vs an open) 3.5 x 2.5 = 8.75bb, `8750`. Out of position
  (SB/BB vs an open) 4 x 2.5 = 10bb, `10000`. BB vs an SB 3bb open: the BB has position on the
  SB, so 3.5 x 3 = 10.5bb, `10500`.
- **BB raise over an SB limp:** 3.5 x 1bb = 3.5bb, `3500`.
- **SB 4bet over the BB 3bet:** out of position, 2.5 x 10.5 = 26.25bb, `26250`.
- **Menus follow each chart's legend rows.** HJ vs LJ, CO vs LJ, CO vs HJ and all four SB-vs-
  open charts have only the 3Bet and Fold rows, so their menu is `fold / raise` with no
  zero-weight call invented. Big Blind vs SB Limp shows "Fold 0/1326", but the BB owes nothing
  after a limp and `core_model` then offers Check rather than Fold, so the menu is
  `check / raise 3.5bb`.
- **SB first-in overlap:** page 3 "Small Blind" and page 6 "Small Blind Strategy" are the same
  node key (four folds, actor SB). The bundle holds that node once, transcribed from page 3.
  The page-6 first action matches page 3 in 169/169 cells: Raise/4bet, Raise/Call and
  Raise/Fold count as raise; Limp/Raise, Limp/Call and Limp/Fold count as limp. The totals match
  too: 322/504/500. Inventory rows 5 and 20 both record this key. The test
  `test_pokercoaching_100_sb_first_in_is_one_node_reconciled_across_pages_3_and_6` pins the
  reconciliation.
- **RFI response to a later 3bet:**
  - **Pages 3-5 (non-blind openers):** no colour encodes a future response; those legends are
    Raise/Fold and 3Bet/Call/Fold only. So all 14 opener-vs-3bettor spots are inventoried
    `absent`.
  - **Page 6, SB:** the Small Blind Strategy colours each SB raising hand with its complete
    response to the BB 3bet: Raise/4bet 58, Raise/Call 120, Raise/Fold 144. That covers all 322
    raise combos. The opponent scope is the BB alone, and every size comes from page 2. So one
    node was added: SB faces the BB 3bet, history `UTG f, HJ f, CO f, BTN f, SB r3, BB r10.5`,
    menu `fold / call / raise 26.25bb`.
  - **Unreachable classes:** the 119 classes the SB limps or folds first are
    `unreachable_classes`, with all-zero weights. Their page-3 raise weight is exactly 0, which
    `test_pokercoaching_100_unreachable_classes_are_exactly_those_the_actor_never_raised`
    checks.
  - **Raise/call and raise/fold annotations** were not treated as current actions anywhere.
- **SB limp, then facing the BB raise** (Limp/Raise, Limp/Call, Limp/Fold): inventoried
  `absent`, because the size is unresolved. No page states the size of the SB's re-raise after
  limping, and a node cannot exist without a raise-to amount.

### Inventory (45 rows: 23 covered rows for 22 distinct node keys, 22 absent)

| # | Status | Page | Title | History key | Next actor | Menu |
|---|---|---|---|---|---|---|
| 1 | covered | 3 | Raise First In (RFI): Lojack | (empty) | UTG | fold / raise 2.5bb |
| 2 | covered | 3 | Raise First In (RFI): Hijack | UTG f | HJ | fold / raise 2.5bb |
| 3 | covered | 3 | Raise First In (RFI): Cutoff | UTG f, HJ f | CO | fold / raise 2.5bb |
| 4 | covered | 3 | Raise First In (RFI): Button | UTG f, HJ f, CO f | BTN | fold / raise 2.5bb |
| 5 | covered | 3 | Raise First In (RFI): Small Blind | UTG f, HJ f, CO f, BTN f | SB | fold / call / raise 3bb |
| 6 | covered | 4 | Facing RFI: In Position: HJ vs LJ RFI | UTG r2.5 | HJ | fold / raise 8.75bb |
| 7 | covered | 4 | Facing RFI: In Position: CO vs LJ RFI | UTG r2.5, HJ f | CO | fold / raise 8.75bb |
| 8 | covered | 4 | Facing RFI: In Position: CO vs HJ RFI | UTG f, HJ r2.5 | CO | fold / raise 8.75bb |
| 9 | covered | 4 | Facing RFI: In Position: BTN vs LJ RFI | UTG r2.5, HJ f, CO f | BTN | fold / call / raise 8.75bb |
| 10 | covered | 4 | Facing RFI: In Position: BTN vs HJ RFI | UTG f, HJ r2.5, CO f | BTN | fold / call / raise 8.75bb |
| 11 | covered | 4 | Facing RFI: In Position: BTN vs CO RFI | UTG f, HJ f, CO r2.5 | BTN | fold / call / raise 8.75bb |
| 12 | covered | 5 | Facing RFI: Out of Position: SB vs LJ RFI | UTG r2.5, HJ f, CO f, BTN f | SB | fold / raise 10bb |
| 13 | covered | 5 | Facing RFI: Out of Position: SB vs HJ RFI | UTG f, HJ r2.5, CO f, BTN f | SB | fold / raise 10bb |
| 14 | covered | 5 | Facing RFI: Out of Position: SB vs CO RFI | UTG f, HJ f, CO r2.5, BTN f | SB | fold / raise 10bb |
| 15 | covered | 5 | Facing RFI: Out of Position: SB vs BTN RFI | UTG f, HJ f, CO f, BTN r2.5 | SB | fold / raise 10bb |
| 16 | covered | 5 | Facing RFI: Out of Position: BB vs LJ RFI | UTG r2.5, HJ f, CO f, BTN f, SB f | BB | fold / call / raise 10bb |
| 17 | covered | 5 | Facing RFI: Out of Position: BB vs HJ RFI | UTG f, HJ r2.5, CO f, BTN f, SB f | BB | fold / call / raise 10bb |
| 18 | covered | 5 | Facing RFI: Out of Position: BB vs CO RFI | UTG f, HJ f, CO r2.5, BTN f, SB f | BB | fold / call / raise 10bb |
| 19 | covered | 5 | Facing RFI: Out of Position: BB vs BTN RFI | UTG f, HJ f, CO f, BTN r2.5, SB f | BB | fold / call / raise 10bb |
| 20 | covered | 6 | Blind vs Blind: Small Blind Strategy (first action) | UTG f, HJ f, CO f, BTN f | SB | same node as row 5 (reconciled) |
| 21 | covered | 6 | Blind vs Blind: Big Blind vs SB Limp | UTG f, HJ f, CO f, BTN f, SB c | BB | check / raise 3.5bb |
| 22 | covered | 6 | Blind vs Blind: Big Blind vs SB raise | UTG f, HJ f, CO f, BTN f, SB r3 | BB | fold / call / raise 10.5bb |
| 23 | covered | 6 | Blind vs Blind: Small Blind Strategy (second action after Raise, facing the Big Blind 3bet) | UTG f, HJ f, CO f, BTN f, SB r3, BB r10.5 | SB | fold / call / raise 26.25bb |
| 24 | absent | 6 | Blind vs Blind: Small Blind Strategy (second action after Limp, facing the Big Blind raise) | UTG f, HJ f, CO f, BTN f, SB c, BB r3.5 | SB | -- (re-raise size unpublished) |
| 25 | absent | 3 | RFI response to a later 3bet: LJ open, HJ 3bet | UTG r2.5, HJ r8.75, CO f, BTN f, SB f, BB f | UTG | -- |
| 26 | absent | 3 | RFI response to a later 3bet: LJ open, CO 3bet | UTG r2.5, HJ f, CO r8.75, BTN f, SB f, BB f | UTG | -- |
| 27 | absent | 3 | RFI response to a later 3bet: LJ open, BTN 3bet | UTG r2.5, HJ f, CO f, BTN r8.75, SB f, BB f | UTG | -- |
| 28 | absent | 3 | RFI response to a later 3bet: LJ open, SB 3bet | UTG r2.5, HJ f, CO f, BTN f, SB r10, BB f | UTG | -- |
| 29 | absent | 3 | RFI response to a later 3bet: LJ open, BB 3bet | UTG r2.5, HJ f, CO f, BTN f, SB f, BB r10 | UTG | -- |
| 30 | absent | 3 | RFI response to a later 3bet: HJ open, CO 3bet | UTG f, HJ r2.5, CO r8.75, BTN f, SB f, BB f | HJ | -- |
| 31 | absent | 3 | RFI response to a later 3bet: HJ open, BTN 3bet | UTG f, HJ r2.5, CO f, BTN r8.75, SB f, BB f | HJ | -- |
| 32 | absent | 3 | RFI response to a later 3bet: HJ open, SB 3bet | UTG f, HJ r2.5, CO f, BTN f, SB r10, BB f | HJ | -- |
| 33 | absent | 3 | RFI response to a later 3bet: HJ open, BB 3bet | UTG f, HJ r2.5, CO f, BTN f, SB f, BB r10 | HJ | -- |
| 34 | absent | 3 | RFI response to a later 3bet: CO open, BTN 3bet | UTG f, HJ f, CO r2.5, BTN r8.75, SB f, BB f | CO | -- |
| 35 | absent | 3 | RFI response to a later 3bet: CO open, SB 3bet | UTG f, HJ f, CO r2.5, BTN f, SB r10, BB f | CO | -- |
| 36 | absent | 3 | RFI response to a later 3bet: CO open, BB 3bet | UTG f, HJ f, CO r2.5, BTN f, SB f, BB r10 | CO | -- |
| 37 | absent | 3 | RFI response to a later 3bet: BTN open, SB 3bet | UTG f, HJ f, CO f, BTN r2.5, SB r10, BB f | BTN | -- |
| 38 | absent | 3 | RFI response to a later 3bet: BTN open, BB 3bet | UTG f, HJ f, CO f, BTN r2.5, SB f, BB r10 | BTN | -- |
| 39 | absent | -- | BB RFI (folded to the big blind) | UTG f, HJ f, CO f, BTN f, SB f | (hand over) | -- |
| 40 | absent | -- | Squeeze: BB facing a LJ open and an HJ call | UTG r2.5, HJ c, CO f, BTN f, SB f | BB | -- |
| 41 | absent | -- | BB defence facing a BTN open and an SB call | UTG f, HJ f, CO f, BTN r2.5, SB c | BB | -- |
| 42 | absent | -- | Cold call or cold 4bet facing an open and a 3bet: CO facing LJ open, HJ 3bet | UTG r2.5, HJ r8.75 | CO | -- |
| 43 | absent | -- | vs-4bet: HJ facing the LJ 4bet after HJ 3bet | UTG r2.5, HJ r8.75, CO f, BTN f, SB f, BB f, UTG r21.875 | HJ | -- |
| 44 | absent | 6 | vs-4bet: BB facing the SB 4bet after BB 3bet | UTG f, HJ f, CO f, BTN f, SB r3, BB r10.5, SB r26.25 | BB | -- |
| 45 | absent | -- | Facing a limp outside the blinds: HJ facing a LJ limp | UTG c | HJ | -- |

Notes on the inventory:

- **LJ is always normalized to `UTG`.**
- **Every covered history lists every intervening fold explicitly.** Its next actor was checked
  both by the Python test `test_pokercoaching_100_every_node_actor_is_the_next_seat_to_act`,
  which mirrors `core_preflop::store::next_actor`, and by `build`'s `validate`.
- **Rows 40-45 are representative keys:** each stands for a whole family of spots that no page
  covers. Each row's `reason` field records the visual finding.
- **The BB defence rows are not interchangeable:** row 41, a BB decision after an opener and a
  caller, is deliberately distinct from the covered row 19, where the SB folds. No chart is
  inserted under another chart's history.
- **Physical page 1 (the cover)** was inspected for any encoding of a future response and holds
  none.

### Per-grid verification record

Every grid meets the same standard:

- page and exact chart title as in the table above;
- checked class count 169;
- pass 1 (400 DPI, top-down), pass 2 (300 DPI, reverse row order) and pass 3 (pixel classifier,
  native JPEG) all agree;
- per-code combo totals equal the published legend;
- mixed cells 0, corrections 0, verification complete 2026-09-26.

That record holds for: Lojack, Hijack, Cutoff, Button, Small Blind (page 3); HJ vs LJ, CO vs LJ,
CO vs HJ, BTN vs LJ, BTN vs HJ, BTN vs CO RFI (page 4); SB vs LJ/HJ/CO/BTN RFI and BB vs
LJ/HJ/CO/BTN RFI (page 5); Small Blind Strategy, Big Blind vs SB Limp, Big Blind vs SB raise
(page 6).

- **Boundary hands:** every cell was compared, which includes every range boundary.
- **Sum-to-one:** the class-sum check was not used as evidence that any cell was read
  correctly. It is reported only as `validate` output.

### Build / validate / verify (2026-09-26)

```
> python tools/chart_ingest.py build fixtures/charts/transcription/pokercoaching_100.json fixtures/charts/pokercoaching_100.json fixtures/charts/pokercoaching_100.manifest.json
(exit 0)
> python tools/chart_ingest.py validate fixtures/charts/pokercoaching_100.json
(22 per-node class-sum lines: nodes 0-20 all 169 sums = 1.0; node 21, the SB response to the BB 3bet, has 50 sums = 1.0 and 119 = 0.0, the declared unreachable classes)
{"aggregate_min": 0.0, "aggregate_max": 1.0}
> python tools/chart_ingest.py verify fixtures/charts/transcription/pokercoaching_100.json fixtures/charts/pokercoaching_100.json fixtures/charts/pokercoaching_100.manifest.json
verified 22 nodes, 3718 classes
```

## RangeConverter 200bb transcription (Task 6)

*Not started.*

## Frozen coverage and Plan 4 handoff (Task 7)

*Not started.*

## Replay/bet-translation goldens audit (Task 19)

*Not started.*

## Independent blind re-read of the 100bb grids (2026-09-26)

A second agent, different from the transcriber, re-read all 22 grids (pages 3-6 physical) from fresh pypdfium2 renders in reverse row order with the committed transcription closed, wrote its own values for every cell, and only then diffed them programmatically against `fixtures/charts/transcription/pokercoaching_100.json`: 3,718 cells read, 0 mismatches; the page-6 SB grid reduced to its first action also matches node 4 (169 cells, 0 mismatches); an independent pixel classification agrees on all 3,718 cells and no cell is mixed. Orientation (suited above the diagonal, offsuit below) and grid-to-node assignment were confirmed on the pages, and eight absent inventory rows were spot-checked as genuinely absent. The flagged node "SB facing the BB 3bet" (row 23) is kept as transcribed: its 10.5bb 3bet is page 2's in-position 3.5x rule (the BB acts in position against the SB preflop); node 20 (BB vs SB raise) uses the same size, so any later change must update both. This blind re-read satisfies the Step 5 hidden-first-pass requirement; the transcriber's own reverse re-read was not blind (disclosed above). Evidence: the plan-3 SDD workspace file `task-5-visual-verification.md`.
