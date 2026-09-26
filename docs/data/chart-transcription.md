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
| 44 | absent | 6 | Absent-node audit: vs-4bet (BB facing the SB 4bet after BB 3bet) | UTG f, HJ f, CO f, BTN f, SB r3, BB r10.5, SB r26.25 | BB | -- |
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

Completed 2026-09-26. Depth 200 is `"available"` in `fixtures/charts/sources.manifest.json`, so
the bundle ships. Source: the committed `fixtures/charts/sources/rangeconverter_200.pdf`
(SHA-256 `f0797be2...9afeda3a76d`, as recorded under Task 4). Outputs:
`fixtures/charts/transcription/rangeconverter_200.json` (35 transcribed grids and a 44-row
inventory), the built envelope `fixtures/charts/rangeconverter_200.json` (35 nodes, 5915
classes, SHA-256 `8595c0a803d4d07bce0ea136b3dcb195d784ca9a5a7bbfdff4f986e352f9434d`) and its
manifest `fixtures/charts/rangeconverter_200.manifest.json` (`source = "ChartTranscription"`,
`ev_reference = "unverified"`, `rake_profile = "undocumented"`, `rake = null`, `accuracy =
"unverified"`, no `evs` anywhere).

### Page numbering and page content

Pages are numbered in **physical PDF page order** (PDFium's order). For this PDF that order
matches Task 4's numbering above.

| Physical page | Text-layer title | Content |
|---|---|---|
| 1 | "No Limit Texas Holdem 6 max Poker Charts: 200bb" | Intro text and one 1200x407 image. No grid. |
| 2 | "How to Use the 6 max Preflop Range Charts" | Text only: the three chart families and the 50% rounding rule quoted under Task 4. |
| 3 | "Raise First In (RFI) - 6 max 200bb Ranges" | 5 grids: UTG, MP, CO, BTN, SB RFI |
| 4 | "MP vs RFI - 6 max 200bb Ranges" | 1 grid: MP vs UTG RFI |
| 5 | "CO vs RFI - 6 max 200bb Ranges" | 2 grids: CO vs UTG, CO vs MP RFI |
| 6 | "BTN vs RFI - 6 max 200bb Ranges" | 3 grids: BTN vs UTG, MP, CO RFI |
| 7 | "SB vs RFI - 6 max 200bb Ranges" | 4 grids: SB vs UTG, MP, CO, BTN RFI |
| 8 | "BB vs RFI - 6 max 200bb Ranges" | 5 grids: BB vs UTG, MP, CO, BTN, SB RFI |
| 9 | "UTG RFI vs 3bet - 6 max 200bb Ranges" | 5 grids: UTG vs MP, CO, BTN, SB, BB 3bet |
| 10 | "MP RFI vs 3bet - 6 max 200bb Ranges" | 4 grids: MP vs CO, BTN, SB, BB 3bet |
| 11 | "CO RFI vs 3bet - 6 max 200bb Ranges" | 3 grids: CO vs BTN, SB, BB 3bet |
| 12 | "BTN RFI vs 3bet - 6 max 200bb Ranges" | 2 grids: BTN vs SB, BB 3bet |
| 13 | "SB RFI vs 3bet - 6 max 200bb Ranges" | 1 grid: SB vs BB 3bet |

Each of pages 3-13 places one 2048x1313 RGB image (with a grey soft mask; these 11 + 11 are
Task 4's "22 images"). The image holds up to five grids laid out four across and one below. Each
grid has its chart label above it (for example "UTG vs MP 3bet") and its legend panel below it.
The publisher's **MP is HJ** here.

Correction note (2026-09-26): Task 4 quoted the page titles as "6max 200bb". The text layer
reads "6 max 200bb", with a space.

### How the pages were read

- **Renderer:** `tools/chart_render.py` and `pypdfium2` (Task 5's tooling), used unchanged.
  Every image went to the session scratch directory outside the repository. No PNG is committed.
  Renders made:
  - pages 1-13 at 60 DPI, for layout;
  - each grid page's embedded image, decoded at native resolution (2048x1313);
  - each of the 35 grids cropped from it and magnified 2x;
  - each grid re-cropped from a separate 300 DPI PDFium page render (a second rendering path,
    with the soft mask applied);
  - 4x-6x crops of every disputed cell;
  - 2x-3x crops of every legend panel.
- **Lattice:** the cell lattice is 38.2 x 42.13 px in the native image. It was fitted from the
  dark separator lines of every grid, with a residual under 1.2 px.
- **Orientation check:** every cell prints its own hand label. For example `AKs` is at row 0,
  column 1 and `AKo` is at row 1, column 0. So row r, column c is class `r*13+c` of
  `class_names()`: pairs on the diagonal, suited above it, offsuit below it, with the high rank
  first. This was confirmed on every grid before transcription.
- **Pass 1 (the transcription):** native image, 2x, top row first, left to right. Written to a
  scratch file per grid.
- **Pass 2 (blind reread, per the orchestrator's instruction):**
  - The 300 DPI page render, read bottom row first and right to left within each row.
  - Written to a separate scratch file per grid. The pass-1 files were not opened during pass 2.
  - The two passes were then diffed by script.
  - Limitation, disclosed: one agent did both passes in one session, so pass 1 was in that
    agent's working context even though its file was not viewed. Pass 3 is the fully
    independent read.
  - One pass-2 recording slip was caught before the diff. The pass-2 file for "CO vs BTN 3bet"
    was first written with inconsistent row orientation. That grid was re-read from its pass-2
    image and the file rewritten. Pass 1 was not consulted.
- **Pass 3 (pixel classifier, a throwaway script, not committed):**
  - For each cell it finds its own separator lines and classifies every interior pixel to the
    nearest legend fill colour: orange, green, blue, grey or navy.
  - It separates the action-coloured strip rows from the grey or navy background.
  - It measures each action colour's share of the strip width by a per-column vote.
  - Mixed cells split 0.441-0.559 of the width. Pure cells are 1.000. No cell has a third
    colour.
- **Comparison:** 35 x 169 = 5915 cells. There were **13 disagreements**, each re-inspected at
  4x-6x from the native image:
  - "SB vs CO RFI": 87o, 76s, 76o, 65s. "SB vs BTN RFI": 87o, 77, 76s. "MP vs BB 3bet": ATs,
    A9s, A7s, A6s. These 11 were pass-2 recording slips (values shifted along a row). The zoom
    confirmed pass 1 and pass 3.
  - "UTG vs BTN 3bet" 75s and "UTG vs BB 3bet" 75s were recorded as grey with no strip by pass 1
    (and by pass 2 for the BB grid). The zoom shows one full pixel row of the legend's Call green
    at the bottom of the cell: (63,143,107) and (62,137,104), within 6 and 12 of the fill colour.
    Both were **corrected to Call**, per the thin-strip rule below.
  - After resolution all three reads agree on 5915/5915 cells. First-pass corrections: 2.

### What the cells mean

- **Colours:**
  - orange = the legend's raise row (for example "2.5bb" or "8.5bb");
  - green = "Call" (on "SB RFI", the SB completing, i.e. a limp);
  - blue = "Fold";
  - a cell split into two colours = 50% each, per page 2's rule that "the frequency of an action
    for each hand combo is rounded to the nearest 50%". Every weight is exactly 0, 0.5 or 1.
- **Pages 3-8 (RFI, vs-RFI):** every cell is full height. No cell is uncoloured.
- **Pages 9-13 (vs-3bet):**
  - Each cell shows the opener's range. The action colours fill a strip whose height is how much
    of the class the opener raised; the rest of the cell is grey. Only the strip's horizontal
    colour split is transcribed. Its height is reach, not an action frequency.
  - Navy cells are outside the opener's displayed range.
  - These charts omit no action: every legend has Fold, Call and a 4bet size.
- **Unreachable classes:** navy cells, and grey cells with no readable strip, give no action
  data. They are the node's `unreachable_classes`, with all-zero weights, and are recorded as
  codes `.` and `g`. For every one of them the opener's own page-3 raise weight is exactly 0.
  `test_rangeconverter_200_unreachable_classes_have_zero_opening_weight` checks this.
- **Thin-strip rule:**
  - A strip counts only when at least one full pixel row inside the cell matches a legend fill
    colour (within 20 of it).
  - Cells whose bottom edge only has an antialiased tint, i.e. a blend between grey and a fill
    colour, are unreadable and are recorded as `g`. These are:
    - 64s in all five UTG vs-3bet grids: (57,113,93) x3, (52,90,83) and (54,100,87);
    - 75s in UTG vs MP, CO and SB 3bet, where the tint merges into the cell separator;
    - T5s in "SB vs BB 3bet": (20,64,95).
  - Each has an opening weight of 0.
- **Classes with data but zero published opening weight:** a thin strip means the opener raises
  the class at a frequency the page-3 grid rounds to 0. These classes keep their published
  strategy, and their reach is 0 through the page-3 node. They are:
  - UTG vs 3bet: A2s, K7s, QTo, T8s, 97s, 86s, 55, 44, 33, plus 75s against the BTN and BB;
  - MP: 97s, 86s;
  - CO: Q9o, J9o, A7o;
  - BTN: 74s;
  - SB: K7o, 87o, 74s.

### Legend as read from the raster pages

Each legend panel is one bar per action, labelled with the action or the raise-to size and the
publisher's percentage. The percentages are unrounded solver frequencies. On pages 3-8 they are
a share of all 1326 combos. On pages 9-13 they are a share of the opener's range reaching the
node. Every panel's percentages sum to 100 within 0.02 (display rounding).
`test_rangeconverter_200_published_legends_are_complete` pins this, independently of the cells.
No raster page mentions rake.

| Page | Chart | Legend rows as read | Codes used | Mixed cells | Unreachable |
|---|---|---|---|---|---|
| 3 | UTG RFI | Fold 81.95%; 2.5bb 18.05% | R RF F | 9 | 0 |
| 3 | MP RFI | Fold 77.49%; 2.5bb 22.51% | R RF F | 16 | 0 |
| 3 | CO RFI | Fold 69.85%; 2.5bb 30.15% | R RF F | 8 | 0 |
| 3 | BTN RFI | Fold 53.37%; 2.5bb 46.63% | R RF F | 1 | 0 |
| 3 | SB RFI | Fold 53.65%; Call 7.19%; 3.0bb 39.16% | R RC RF CF F | 18 | 0 |
| 4 | MP vs UTG RFI | Fold 91.44%; 8.5bb 8.56% | R RF F | 12 | 0 |
| 5 | CO vs UTG RFI | Fold 90.63%; 8.5bb 9.37% | R RF F | 11 | 0 |
| 5 | CO vs MP RFI | Fold 89.1%; 8.5bb 10.9% | R RF F | 11 | 0 |
| 6 | BTN vs UTG RFI | Fold 87.0%; Call 4.37%; 8.5bb 8.63% | R RC RF F | 28 | 0 |
| 6 | BTN vs MP RFI | Fold 85.81%; Call 2.85%; 8.5bb 11.34% | R RC RF F | 19 | 0 |
| 6 | BTN vs CO RFI | Fold 83.64%; Call 1.18%; 8.5bb 15.19% | R RF F | 13 | 0 |
| 7 | SB vs UTG RFI | Fold 93.99%; 10.9bb 6.01% | R RF F | 7 | 0 |
| 7 | SB vs MP RFI | Fold 92.6%; 10.9bb 7.4% | R RF F | 6 | 0 |
| 7 | SB vs CO RFI | Fold 90.63%; 10.9bb 9.37% | R RF F | 8 | 0 |
| 7 | SB vs BTN RFI | Fold 86.44%; 10.9bb 13.56% | R RF F | 5 | 0 |
| 8 | BB vs UTG RFI | Fold 73.13%; Call 22.4%; 11.05bb 4.47% | R RC C CF F | 24 | 0 |
| 8 | BB vs MP RFI | Fold 69.76%; Call 24.47%; 11.05bb 5.77% | R RC C CF F | 18 | 0 |
| 8 | BB vs CO RFI | Fold 64.3%; Call 27.71%; 11.05bb 7.99% | R RC C CF F | 27 | 0 |
| 8 | BB vs BTN RFI | Fold 50.15%; Call 37.2%; 11.05bb 12.64% | R RC C CF F | 31 | 0 |
| 8 | BB vs SB RFI | Fold 42.19%; Call 39.91%; 10.0bb 17.89% | R RC RF C CF F | 37 | 0 |
| 9 | UTG vs MP 3bet | Fold 58.08%; Call 23.23%; 23.6bb 18.69% | R RC C CF F g . | 23 | 115 |
| 9 | UTG vs CO 3bet | Fold 54.99%; Call 25.05%; 23.6bb 19.96% | R RC C CF F g . | 22 | 115 |
| 9 | UTG vs BTN 3bet | Fold 48.52%; Call 32.11%; 23.8bb 19.36% | R RC C CF F g . | 21 | 114 |
| 9 | UTG vs SB 3bet | Fold 50.38%; Call 36.79%; 25.05bb 12.82% | R RC C CF F g . | 14 | 115 |
| 9 | UTG vs BB 3bet | Fold 49.08%; Call 41.8%; 25.0bb 9.11% | R RC C CF F g . | 13 | 114 |
| 10 | MP vs CO 3bet | Fold 56.69%; Call 24.55%; 23.8bb 18.75% | R RC C CF F . | 25 | 109 |
| 10 | MP vs BTN 3bet | Fold 51.82%; Call 28.42%; 23.6bb 19.75% | R RC C CF F . | 24 | 109 |
| 10 | MP vs SB 3bet | Fold 50.37%; Call 37.09%; 25.05bb 12.54% | R RC C CF F . | 10 | 109 |
| 10 | MP vs BB 3bet | Fold 49.76%; Call 41.08%; 25.0bb 9.14% | R RC C CF F . | 9 | 109 |
| 11 | CO vs BTN 3bet | Fold 53.56%; Call 27.79%; 23.6bb 18.65% | R RC C CF F . | 31 | 95 |
| 11 | CO vs SB 3bet | Fold 50.54%; Call 37.83%; 25.05bb 11.63% | R RC C CF F . | 12 | 95 |
| 11 | CO vs BB 3bet | Fold 50.72%; Call 39.37%; 25.0bb 9.91% | R RC C CF F . | 11 | 95 |
| 12 | BTN vs SB 3bet | Fold 52.29%; Call 37.83%; 25.05bb 9.88% | R RC C CF F . | 18 | 71 |
| 12 | BTN vs BB 3bet | Fold 53.14%; Call 37.19%; 25.0bb 9.66% | R RC C CF F . | 14 | 71 |
| 13 | SB vs BB 3bet | Fold 53.98%; Call 27.02%; 27.05bb 19.0% | R RC C CF F g . | 24 | 72 |

The raise-size labels were re-read at 3x, because 23.6 and 23.8 differ between charts.

The rounded grids' reach-weighted action shares track these unrounded percentages. For example,
"UTG RFI" grid 82.2/17.8 against legend 81.95/18.05, and "BB vs SB RFI" 42.23/41.63/16.14
against 42.19/39.91/17.89. This is informational only, never evidence that a cell was read
correctly. "BTN vs CO RFI"'s Call row (1.18%) rounds to no cell, so its call weight is 0 in
every class. The call stays on the menu, because the legend lists it.

### Menus and resolved sizes

Every size comes from this PDF's own legend panels and is frozen as a raise-to in `to_bb_x1000`.

- **Opens:** 2.5bb (`2500`) for UTG, MP, CO and BTN. 3.0bb (`3000`) for the SB, which also has
  Call (a limp) on its menu.
- **3bets (vs-RFI legends, pages 4-8):**
  - MP, CO and BTN vs any open: 8.5bb (`8500`);
  - SB: 10.9bb (`10900`);
  - BB vs UTG, MP, CO and BTN: 11.05bb (`11050`);
  - BB vs SB: 10.0bb (`10000`).
- **4bets (vs-3bet legends, pages 9-13):**
  - UTG vs MP and vs CO: 23.6bb; UTG vs BTN: 23.8bb;
  - MP vs CO: 23.8bb; MP vs BTN: 23.6bb; CO vs BTN: 23.6bb;
  - any opener vs the SB: 25.05bb; any opener vs the BB: 25.0bb;
  - SB vs BB: 27.05bb.
- **vs-3bet keys (judgement call, disclosed):**
  - A vs-3bet page prints only the 4bet size. The 3bet the opener faces is resolved from the
    matching vs-RFI legend in this PDF, for the same two seats. For example "UTG vs MP 3bet"
    uses the 8.5bb from "MP vs UTG RFI" (page 4).
  - Each inventory reason names its source page.
  - `test_rangeconverter_200_every_history_raise_is_a_published_menu_size` checks every raise
    in every history against the menu of the node that published it.
  - The key lists every fold: before the open, between the open and the 3bet, and after the
    3bettor until action returns to the opener.
- **Menus follow each chart's legend rows:**
  - fold / raise where the legend has no Call row (MP and CO vs RFI, SB vs RFI, the non-SB RFI
    grids);
  - fold / call / raise otherwise.

### Inventory (44 rows: 36 covered rows for 35 distinct node keys, 8 absent)

| # | Status | Page | Title | History key | Next actor | Menu |
|---|---|---|---|---|---|---|
| 1 | covered | 3 | UTG RFI | (empty) | UTG | fold / raise 2.5bb |
| 2 | covered | 3 | MP RFI | UTG f | HJ | fold / raise 2.5bb |
| 3 | covered | 3 | CO RFI | UTG f, HJ f | CO | fold / raise 2.5bb |
| 4 | covered | 3 | BTN RFI | UTG f, HJ f, CO f | BTN | fold / raise 2.5bb |
| 5 | covered | 3 | SB RFI | UTG f, HJ f, CO f, BTN f | SB | fold / call / raise 3bb |
| 6 | covered | 4 | MP vs UTG RFI | UTG r2.5 | HJ | fold / raise 8.5bb |
| 7 | covered | 5 | CO vs UTG RFI | UTG r2.5, HJ f | CO | fold / raise 8.5bb |
| 8 | covered | 5 | CO vs MP RFI | UTG f, HJ r2.5 | CO | fold / raise 8.5bb |
| 9 | covered | 6 | BTN vs UTG RFI | UTG r2.5, HJ f, CO f | BTN | fold / call / raise 8.5bb |
| 10 | covered | 6 | BTN vs MP RFI | UTG f, HJ r2.5, CO f | BTN | fold / call / raise 8.5bb |
| 11 | covered | 6 | BTN vs CO RFI | UTG f, HJ f, CO r2.5 | BTN | fold / call / raise 8.5bb |
| 12 | covered | 7 | SB vs UTG RFI | UTG r2.5, HJ f, CO f, BTN f | SB | fold / raise 10.9bb |
| 13 | covered | 7 | SB vs MP RFI | UTG f, HJ r2.5, CO f, BTN f | SB | fold / raise 10.9bb |
| 14 | covered | 7 | SB vs CO RFI | UTG f, HJ f, CO r2.5, BTN f | SB | fold / raise 10.9bb |
| 15 | covered | 7 | SB vs BTN RFI | UTG f, HJ f, CO f, BTN r2.5 | SB | fold / raise 10.9bb |
| 16 | covered | 8 | BB vs UTG RFI | UTG r2.5, HJ f, CO f, BTN f, SB f | BB | fold / call / raise 11.05bb |
| 17 | covered | 8 | BB vs MP RFI | UTG f, HJ r2.5, CO f, BTN f, SB f | BB | fold / call / raise 11.05bb |
| 18 | covered | 8 | BB vs CO RFI | UTG f, HJ f, CO r2.5, BTN f, SB f | BB | fold / call / raise 11.05bb |
| 19 | covered | 8 | BB vs BTN RFI | UTG f, HJ f, CO f, BTN r2.5, SB f | BB | fold / call / raise 11.05bb |
| 20 | covered | 8 | BB vs SB RFI | UTG f, HJ f, CO f, BTN f, SB r3 | BB | fold / call / raise 10bb |
| 21 | covered | 9 | UTG vs MP 3bet | UTG r2.5, HJ r8.5, CO f, BTN f, SB f, BB f | UTG | fold / call / raise 23.6bb |
| 22 | covered | 9 | UTG vs CO 3bet | UTG r2.5, HJ f, CO r8.5, BTN f, SB f, BB f | UTG | fold / call / raise 23.6bb |
| 23 | covered | 9 | UTG vs BTN 3bet | UTG r2.5, HJ f, CO f, BTN r8.5, SB f, BB f | UTG | fold / call / raise 23.8bb |
| 24 | covered | 9 | UTG vs SB 3bet | UTG r2.5, HJ f, CO f, BTN f, SB r10.9, BB f | UTG | fold / call / raise 25.05bb |
| 25 | covered | 9 | UTG vs BB 3bet | UTG r2.5, HJ f, CO f, BTN f, SB f, BB r11.05 | UTG | fold / call / raise 25bb |
| 26 | covered | 10 | MP vs CO 3bet | UTG f, HJ r2.5, CO r8.5, BTN f, SB f, BB f | HJ | fold / call / raise 23.8bb |
| 27 | covered | 10 | MP vs BTN 3bet | UTG f, HJ r2.5, CO f, BTN r8.5, SB f, BB f | HJ | fold / call / raise 23.6bb |
| 28 | covered | 10 | MP vs SB 3bet | UTG f, HJ r2.5, CO f, BTN f, SB r10.9, BB f | HJ | fold / call / raise 25.05bb |
| 29 | covered | 10 | MP vs BB 3bet | UTG f, HJ r2.5, CO f, BTN f, SB f, BB r11.05 | HJ | fold / call / raise 25bb |
| 30 | covered | 11 | CO vs BTN 3bet | UTG f, HJ f, CO r2.5, BTN r8.5, SB f, BB f | CO | fold / call / raise 23.6bb |
| 31 | covered | 11 | CO vs SB 3bet | UTG f, HJ f, CO r2.5, BTN f, SB r10.9, BB f | CO | fold / call / raise 25.05bb |
| 32 | covered | 11 | CO vs BB 3bet | UTG f, HJ f, CO r2.5, BTN f, SB f, BB r11.05 | CO | fold / call / raise 25bb |
| 33 | covered | 12 | BTN vs SB 3bet | UTG f, HJ f, CO f, BTN r2.5, SB r10.9, BB f | BTN | fold / call / raise 25.05bb |
| 34 | covered | 12 | BTN vs BB 3bet | UTG f, HJ f, CO f, BTN r2.5, SB f, BB r11.05 | BTN | fold / call / raise 25bb |
| 35 | covered | 13 | SB vs BB 3bet | UTG f, HJ f, CO f, BTN f, SB r3, BB r10 | SB | fold / call / raise 27.05bb |
| 36 | covered | 3 | Blind limp lines: SB first-in limp strategy | UTG f, HJ f, CO f, BTN f | SB | same node as row 5 (its call column) |
| 37 | absent | -- | Blind limp lines: BB vs SB limp | UTG f, HJ f, CO f, BTN f, SB c | BB | -- |
| 38 | absent | -- | Blind limp lines: SB vs BB isolation raise after an SB limp | UTG f, HJ f, CO f, BTN f, SB c (resolvable prefix only) | SB | -- (BB raise size unpublished) |
| 39 | absent | -- | Absent-node audit: multiway caller (BTN facing a UTG open and an MP call) | UTG r2.5, HJ c, CO f | BTN | -- |
| 40 | absent | -- | Absent-node audit: squeeze (BB facing a CO open and a BTN call) | UTG f, HJ f, CO r2.5, BTN c, SB f | BB | -- |
| 41 | absent | -- | Absent-node audit: cold call or cold 4bet facing an open and a 3bet (CO facing UTG open, MP 3bet) | UTG r2.5, HJ r8.5 | CO | -- |
| 42 | absent | -- | Absent-node audit: vs-4bet (MP facing the UTG 4bet after MP 3bet) | UTG r2.5, HJ r8.5, CO f, BTN f, SB f, BB f, UTG r23.6 | HJ | -- |
| 43 | absent | -- | Absent-node audit: vs-4bet (BB facing the SB 4bet after BB 3bet) | UTG f, HJ f, CO f, BTN f, SB r3, BB r10, SB r27.05 | BB | -- |
| 44 | absent | -- | Absent-node audit: open-limp outside the SB (MP facing a UTG limp) | UTG c | HJ | -- |

Notes on the inventory:

- **Every audit-matrix candidate in the brief is covered:** RFI 5, vs-RFI 15, vs-3bet 15. Each
  row's `reason` states the page, the resolved sizes and where each size came from.
- **The SB first-in limp strategy is not a separate grid.** It is the call column of "SB RFI",
  so rows 5 and 36 share one node key and the bundle holds that node once.
- **Rows 37-44 are the visual audit's absences.** No page has a BB-vs-limp, SB-vs-isolation,
  multiway, squeeze, cold-call-vs-3bet or vs-4bet grid. No non-SB RFI legend has a limp row.
- **Row 38 cannot be keyed.** The BB's isolation size is published nowhere, so it records only
  the resolvable prefix and says so in its reason.
- **The sizes in rows 42-43 are from this PDF.** They are recorded only to identify the missing
  spot.
- **Next actors are checked twice:** by
  `test_rangeconverter_200_every_node_actor_is_the_next_seat_to_act` (which mirrors
  `core_preflop::store::next_actor`) and by `build`'s `validate`.

### Per-grid verification record

Every one of the 35 grids in the legend table above meets the same standard:

- page and exact chart label as in the table;
- checked class count 169;
- pass 1 (native 2x, top-down), pass 2 (300 DPI render, bottom row first and right to left,
  written blind to a separate file) and pass 3 (pixel classifier) compared by script, every
  disagreement re-inspected at 4x-6x;
- verification complete 2026-09-26.

Per-grid disagreements and corrections:

- **0 disagreements on 30 grids:** all 5 RFI grids; MP vs UTG; CO vs UTG and MP; BTN vs UTG,
  MP and CO; SB vs UTG and MP; BB vs UTG, MP, CO, BTN and SB; UTG vs MP, CO and SB 3bet; MP vs
  CO, BTN and SB 3bet; CO vs BTN, SB and BB 3bet; BTN vs SB and BB 3bet; SB vs BB 3bet.
- **SB vs CO RFI (page 7):** 4 disagreements, all pass-2 slips. Pass 1 stands; 0 corrections.
- **SB vs BTN RFI (page 7):** 3 disagreements, all pass-2 slips. Pass 1 stands; 0 corrections.
- **MP vs BB 3bet (page 10):** 4 disagreements, all pass-2 slips. Pass 1 stands; 0 corrections.
- **UTG vs BTN 3bet (page 9):** 1 disagreement, 75s. Corrected from `g` to Call (thin-strip
  rule).
- **UTG vs BB 3bet (page 9):** 1 disagreement, 75s. Corrected from `g` to Call (thin-strip rule).

- **Boundary hands:** every cell was compared, which includes every range boundary.
- **Sum-to-one:** the class-sum check was not used as evidence that any cell was read
  correctly. It is reported only as `validate` output.

### Build / validate / verify (2026-09-26)

```
> python tools/chart_ingest.py build fixtures/charts/transcription/rangeconverter_200.json fixtures/charts/rangeconverter_200.json fixtures/charts/rangeconverter_200.manifest.json
(exit 0)
> python tools/chart_ingest.py validate fixtures/charts/rangeconverter_200.json
(35 per-node class-sum lines: nodes 0-19 (RFI, vs-RFI) all 169 sums = 1.0; the 15 vs-3bet nodes 20-34 have 54/54/55/54/55, 60 x4, 74 x3, 98, 98, 97 sums = 1.0 and the rest = 0.0, exactly their declared unreachable classes)
{"aggregate_min": 0.0, "aggregate_max": 1.0}
> python tools/chart_ingest.py verify fixtures/charts/transcription/rangeconverter_200.json fixtures/charts/rangeconverter_200.json fixtures/charts/rangeconverter_200.manifest.json
verified 35 nodes, 5915 classes
```

A sanity load through the Rust boundary was also run. It used a scratch crate outside the
repository, which is not committed; Task 7 owns the committed Rust test.
`core_preflop::load_bundle` accepted the bundle as `ChartTranscription`, depth 200,
`EvReference::Unverified`.

For Task 7, the covered prefixes at 200bb include:

- BTN open, then BB call ("BB vs BTN RFI", page 8);
- CO open, then BB call ("BB vs CO RFI", page 8);
- BTN open, then BB 3bet, then BTN call ("BTN vs BB 3bet", page 12).

## Frozen coverage and Plan 4 handoff (Task 7)

*Not started.*

## Replay/bet-translation goldens audit (Task 19)

*Not started.*
