# R7 — PokerData procurement fact-check

Date: 2026-09-10. Method: read-only review of public pages only (pokerdata.io, docs.pokerdata.io, pokerstudy.ai, the free keyless discovery endpoint, and the public range explorer). No account, trial key, or purchase was made. Note: `pokerdata.io/app/dashboard` auto-provisions a free Starter key on first open ("no card, no sign-up"), so that page was deliberately not opened.

Operator: "PokerData ... operated by HFT Studio LLC" — https://pokerdata.io/privacy. Sister product: Poker Study AI (pokerstudy.ai); Studio subscriptions are shared across both sites.

## Status summary

| # | Item | Status |
|---|------|--------|
| 1 | Solver provenance (NLHE 6-max cash) | VERIFIED solver = MonkerSolver; NOT FOUND iterations / exploitability |
| 2 | Bundle list | VERIFIED 8 depths 20–200bb, 5% / 0.5bb cap; CONTRADICTED 250bb, straddle, ante (none exist) |
| 3 | Branch coverage | VERIFIED full tree incl. squeeze and 5-way multiway (with tree simplifications noted) |
| 4 | Schema | VERIFIED 169-class + per-action per-hand EV in small blinds; CONTRADICTED per-combo (1326) |
| 5 | Terms of service | NOT FOUND — no ToS page exists; caching explicitly encouraged; nothing on live use / bots |
| 6 | Pricing | VERIFIED API $100 / $250 per month; one-time packs $50 each, $250 for all 8; Studio $199/mo |
| 7 | Alternatives | see section 7 |

Recommendation: **GO-WITH-CAVEATS** (details in section 8).

---

## 1. Solver provenance

**VERIFIED — the NLHE 6-max packs are MonkerSolver solves.**
- Pack page fields: "Solver: MonkerSolver", "Viewer: MonkerViewer (free)", "No-limit hold'em · 6-max", "100bb effective", "Rake 5%, 0.5bb cap", "Raise sizes 49–200% pot + all-in". — https://pokerdata.io/sims/nlh-6max-100bb (same wording at /sims/nlh-6max-150bb and /sims/nlh-6max-200bb)
- Sibling site: preflop packs are "MonkerSolver .7z exports viewable in MonkerViewer"; postflop packs are "our own DCFR strategy-tree bundles (gzipped JSON ...)" — https://www.pokerstudy.ai/sims

**NOT FOUND — iterations, exploitability / Nash distance, MonkerSolver version, bucketing settings for the NLHE cash packs.**
- The pack pages carry none of these.
- MCP docs: "Provenance includes the solve generation timestamp, iterations, and exploitability percentage when those values are present in the published pack index." — https://docs.pokerdata.io/llm-tools/ (i.e. only if the pack index has them; the NLHE pages do not show them).
- The `/methodology` page ("Methodology — how our solves are tested") describes the **tournament** packs solved by their in-house GPU CFR ("our tournament packs are solved card-exact — all 1,326 hole-card combinations") and compares against MonkerSolver. It does not cover the cash packs. It also states MonkerSolver "groups hands into buckets", which confirms the cash packs are bucketed (169-class), not card-exact. — https://pokerdata.io/methodology
- Manifest metadata: `generatedAt: 2026-06-09`, `bundleCount: 48`, `stacks: [20,30,40,50,70,100,150,200]`, `positions: [BB,BTN,CO,HJ,SB,UTG]` — https://pokerdata.io/api/v1/ranges/nl/v2 (keyless). No solver/rake/accuracy fields in the manifest.

**Tree assumptions observed in the public explorer** (https://pokerdata.io/explorer?stack=100, which loads the real pack data keyless):
- Blinds 0.5 / 1 (SB shows 99.5, BB 99 at 100bb). No ante, no straddle.
- One open size per position (label "Raise 60%"); one 3-bet size ("77%" IP, "150%" SB vs BTN, "195%" BB vs BTN+SB call); 4-bet "95%" (opener) / "50%" (cold, IP) / "85%" (SB) / "75%" (BB); 5-bet = all-in only at 100bb, "50%" or all-in at 200bb; squeeze sizes 79–198%.
- The "X%" labels are pot-percentages, but the explorer's stack arithmetic implies raise-to = facing bet + X% × pot (UTG 100 → 98.1 after the open, i.e. a 1.9bb open), whereas the usual Monker/Pio convention (call + X% × pot-after-call) gives 2.5bb. **The absolute bb size per label must be pinned down after purchase** (acceptance check A3).

## 2. Exact bundle list (NLHE 6-max cash)

**VERIFIED.** Library listing — https://pokerdata.io/ (and https://pokerdata.io/app, sitemap https://pokerdata.io/sitemap.xml):

| Pack | Rake | Size | Page |
|------|------|------|------|
| NLHE 6-max 20bb | 5% · 0.5bb cap | 3 MB | /sims/nlh-6max-20bb |
| 30bb | 5% · 0.5bb cap | 7 MB | /sims/nlh-6max-30bb |
| 40bb | 5% · 0.5bb cap | 9 MB | /sims/nlh-6max-40bb |
| 50bb | 5% · 0.5bb cap | 9 MB | /sims/nlh-6max-50bb |
| 70bb | 5% · 0.5bb cap | 8 MB | /sims/nlh-6max-70bb |
| 100bb | 5% · 0.5bb cap | 7 MB | /sims/nlh-6max-100bb |
| 150bb | 5% · 0.5bb cap | 12 MB | /sims/nlh-6max-150bb |
| 200bb | 5% · 0.5bb cap | 19 MB | /sims/nlh-6max-200bb |

API side: "Stack depths 20, 30, 40, 50, 70, 100, 150, and 200bb", 48 bundles (8 stacks × 6 positions) — https://docs.pokerdata.io/nlhe/

**CONTRADICTED (not offered): 250bb, UTG straddle, ante variants.** No such pack in the library, sitemap, manifest, or docs. The MCP docs state the data does "not support tournament antes, asymmetric tournament stacks, ICM, PKO, satellites, or multiway postflop" — https://docs.pokerdata.io/llm-tools/. The Poker Study AI roadmap has no straddle / 250bb item — https://www.pokerstudy.ai/roadmap. A "Request custom packs" link exists at https://www.pokerstudy.ai/sims/request (terms/price unknown).

Rake note: 5% with a 0.5bb cap is an online (500z-style) structure, not a live-room structure (typically 10% with a $4–$6 cap, or time charge). EV magnitudes and marginal-hand frequencies will differ from a live game.

## 3. Branch coverage

**VERIFIED — full preflop tree, walked in the public explorer at 100bb and 200bb.** Docs: `/spots` returns "every valid decision node for a stack, tagged by depth (1 = RFI, 2 = vs open, …)"; paths are an "underscore-joined chain of `POSITION_ACTION` pairs"; "Every fold must be explicitly written" — https://docs.pokerdata.io/action-paths/, https://docs.pokerdata.io/nlhe/. The explorer ends every line with: "The packs are preflop solves, so this is where the data (and the preview) ends."

Observed nodes (100bb unless noted):

| Line | Actor options observed |
|------|------------------------|
| RFI (every position) | Fold / Raise 60% (single size) |
| SB first-in | Fold / Call (limp) / Raise 76% |
| vs RFI (HJ vs UTG) | Fold / Call / Raise 77% |
| SB vs BTN open | Fold / Call / Raise 150% |
| BB vs BTN open + SB call | Fold / Call / Raise 195% |
| vs 3-bet, opener (UTG vs HJ) | Fold / Call / Raise 95% / All-in |
| vs 3-bet, cold (CO) | Fold / Raise 50% / All-in — **no cold-call** |
| vs 3-bet, cold (BTN) | Fold / Call / Raise 50% / All-in |
| vs 3-bet, cold (SB) | Fold / Raise 85% / All-in — **no cold-call** |
| vs 3-bet, cold (BB) | Fold / Call / Raise 75% / All-in |
| vs 4-bet (HJ) | Fold / Call / All-in (100bb); Fold / Call / Raise 50% / All-in (200bb) |
| vs 5-bet jam (UTG) | Fold / Call |
| Squeeze (CO after UTG open + HJ call) | Fold / Call / Raise 79% / All-in |
| Multiway: BTN after 2 callers | Fold / Call / Raise 81% / All-in |
| Multiway: SB after 3 callers | Fold / Call / Raise 165% / All-in |
| Multiway: BB after 4 callers | Fold / Call / Raise 198% / All-in |
| vs squeeze (opener UTG) | Fold / Call / Raise 95% / All-in |
| vs squeeze (caller HJ after UTG calls) | Fold / Call / Raise 95% / All-in |

Simplifications to be aware of: single open size, single 3-bet size per spot, no cold-call vs 3-bet for CO and SB, no non-all-in 5-bet at 100bb. Multiway lines exist all the way to 5 callers, and BB-vs-limp lines exist via the SB limp node.

## 4. Data schema

**VERIFIED (API v2):**
- "Weights are on the 169-grid (AA, AKs, AKo…), each in [0, 1]; v2 responses carry per-hand EV in small blinds." — https://pokerdata.io/api
- "Only nonzero weights are returned"; EV is returned "including hands the action is never taken with"; combo weights "pair 6, suited 4, offsuit 12". — https://docs.pokerdata.io/nlhe/
- `/node` returns "Every available action at a node, with ranges" → per-action frequency and per-action EV for every 169 class. `/range` returns one action's range. `/spots` lists all nodes.
- Example response (docs):

```json
{"game":"nl","version":2,"stack":100,"spot":"UTG_60%_HJ_Call","actor":"HJ",
 "hand":"AKs","freq":0.35,"ev":1.84,"combos":61.9,
 "weights":{"AKs":0.35,"QQ":0.62,"JJ":1},"evs":{"AKs":1.84,"QQ":2.31}}
```

- Units cross-check: the explorer labels the column "EV BB" but shows AA UTG-RFI +9.15, KK +4.83, AA HJ-3-bet +16.68 — magnitudes consistent with **small blinds** (≈ +4.6bb / +2.4bb / +8.3bb), matching the docs. Treat the explorer label as a UI inaccuracy; confirm on real data (acceptance check A2).

**CONTRADICTED — per-combo (1326) data.** Both the API and the MonkerSolver packs are 169-class ("MonkerSolver ... groups hands into buckets" — /methodology). No suit-specific weights or EVs are available for NLHE.

**Raw pack format:** ".7z" MonkerSolver export, viewable in the free MonkerViewer (https://pokerdata.io/sims/nlh-6max-100bb). Internal file layout (per-node `.rng` files, EV storage) is not documented publicly; a parser is required (see ksoeze/PreflopAdvisor on GitHub as a reference reader for Monker preflop trees). The API JSON is the easier machine-readable path.

## 5. Terms of service

**NOT FOUND — no terms-of-service page exists on either site.**
- pokerdata.io sitemap lists only `/`, `/explorer`, `/api`, `/methodology`, `/support`, `/privacy`, `/sims/*` — https://pokerdata.io/sitemap.xml. Site footer: "Support | Privacy | Sign in" (no Terms). `/terms`, `/tos`, `/legal`, `/license`, `/terms-of-service`, `/terms-and-conditions` all return 404. The sign-in page shows no consent text.
- pokerstudy.ai sitemap lists `/privacy` but no terms page — https://www.pokerstudy.ai/sitemap.xml.

**Local caching / offline storage — VERIFIED as permitted and encouraged (API).**
- "The underlying solves are effectively immutable, so cache aggressively on your side — a spot you fetched once will not change out from under you." — https://docs.pokerdata.io/rate-limits/
- "Cache by URL. Responses are immutable in practice; an in-memory map keyed by URL removes most latency and rate-limit pressure." — https://docs.pokerdata.io/llm-tools/
- "Responses are privately cacheable; the underlying solves are effectively immutable." — https://pokerdata.io/api
- Packs: "Instant download after payment, re-downloadable any time" — https://pokerdata.io/sims/nlh-6max-200bb. The pack is a downloaded file by design.

**Real-time / live-table use, bots, third-party software, redistribution — NOT FOUND.** No clause on any pokerdata.io, docs.pokerdata.io, or pokerstudy.ai public page mentions real-time assistance, bots, live play, third-party tools, or redistribution. The only related statement is on the sister site: "Poker Study AI is a study and training tool. It does not offer real-money gambling ..." — https://www.pokerstudy.ai/privacy. Absence of a prohibition is not a licence grant; get written confirmation by email before relying on it (see section 9). For contrast, competitors do prohibit this: pokerai.bet — "Real-time assistance at real-money tables is prohibited."; GTO Wizard Terms 7.1 — "User must not use Service during a live poker game ...".

**Refunds / cancellation:** "Both plans are cancellable yourself from your account — Studio and the ranges API each have a manage/cancel control there." "For a refund or a charge you don't recognise, email us with the receipt" — https://pokerdata.io/support. No written refund policy.

## 6. Pricing

**VERIFIED.**

API (pdk_ key), from https://pokerdata.io/ and https://docs.pokerdata.io/rate-limits/:

| Tier | Price | Requests | Data | Rate |
|------|-------|----------|------|------|
| Starter | Free | 100 lifetime | 50 MB lifetime | 10/s, burst 25 |
| Pro | $100 / month | 250,000 / month | 5 GB / month | 10/s, burst 25 |
| Max | $250 / month | 5,000,000 / month | 100 GB / month | 50/s, burst 100 |

"Fixed prices, hard limits, no surprise overages." "All of your keys share one budget". Keys "survive a lapsed subscription and start working again on renewal" — https://docs.pokerdata.io/authentication/.

One-time packs (Stripe), from https://pokerdata.io/sims/nlh-6max-100bb and -150bb / -200bb:
- "$50 one-time" per depth (.7z, MonkerViewer).
- NLHE 6-max bundle, all 8 depths: "$250" ("$400 singly"), checkout link `/checkout/nlh-6max`.

Studio subscription: "$199/ month" via Poker Study AI — "Every PLO & NLHE preflop sim, 12bb–200bb", "Direct CDN downloads — resumable in most browsers", "Cancel it yourself any time from your account" — https://pokerdata.io/pricing, https://www.pokerstudy.ai/pricing. Studio is what the postflop replay API requires — https://docs.pokerdata.io/authentication/.

Ambiguity: pokerstudy.ai/pricing lists "API & MCP access (personal tokens)" under Studio, while pokerdata.io/support treats Studio and "the ranges API" as two separate subscriptions. Confirm by email whether Studio includes the pdk_ ranges API.

Bulk export: no bulk endpoint in the API; the one-time pack is the bulk export. A full-tree pull of 100/150/200bb via `/spots` + `/node` is feasible inside one Pro month (compressed packs are 7 / 12 / 19 MB; thousands of nodes at 10 req/s).

## 7. Alternatives (one line each)

- **Pokerai.bet** — MelaSolver-GPU API, JSON frequencies + raise amounts, 6-max NLHE at 100bb + 40bb only, no rake/straddle noted, from $29/mo; explicitly bans real-time assistance at real-money tables. — https://pokerai.bet/
- **GTO Wizard** — has the closest live formats (8-max live cash 100–300bb with "Live Rake (10%, 2bb CAP)"; 8-max single straddle 50–1000bb, with/without ante) but no export/API, and its Terms prohibit use "during a live poker game" and third-party apps. — https://blog.gtowizard.com/live-cash-solutions-and-4000-new-scenarios-for-cash-mtt-formats/, https://gtowizard.com/terms/
- **HRC (HoldemResources Calculator) Pro, $299.99/yr** — self-solve any depth (250bb) with rake and straddles ("straddles as additional posted blinds"), and "export format that includes all strategies and EVs in .json"; preflop-only equity model (check-down assumption) is the accuracy caveat. — https://www.holdemresources.net/blog/2023-hrc-v3-release/, https://www.holdemresources.net/docs/cashgame/
- **MonkerSolver (self-solve)** — the same engine behind PokerData's packs; supports antes, straddle, rake % + cap; solves take days–weeks on CPU; licence cost and output parsing not verified here. — https://www.monkerguy.com/
- **Simple GTO cash bundle, $199 one-time** — 6-max/4-max/3-max/HU, 40–200bb, 500z rake, "Complete preflop tree: RFI, 3-bet, 4-bet, cold-call, and defense", but files require the Simple Preflop solver to read; no straddle. — https://simplegto.com/products/cash-bundle
- **GTOBase** (6-max 40–200bb, web viewer) and **RangeConverter** ($198/$398 downloadable charts) — no machine-readable EV export verified. — https://gtobase.com/, https://rangeconverter.com/
- **ksoeze/PreflopAdvisor** (GitHub) — Python reader for Monker preflop trees (frequencies + EVs, PLO-oriented); useful reference if parsing the .7z packs. — https://github.com/ksoeze/PreflopAdvisor

## 8. Recommendation: GO-WITH-CAVEATS

PokerData is the cheapest verified source of a full 6-max NLHE preflop tree with per-action, per-hand-class EVs in a machine-readable form, for 100/150/200bb with rake. It does **not** cover the full requirement.

Caveats that must be accepted or worked around:
1. **169-class, not 1326 combos** (MonkerSolver bucketing). Suit-specific EVs are unavailable; the app must treat all suit combos of a class identically.
2. **No 250bb, no straddle, no ante.** Those must be self-solved (HRC Pro is the practical route: rake + straddle + JSON EV export) or commissioned via the custom-pack request form.
3. **No terms of service.** Local caching is explicitly encouraged and packs are sold as downloads, but nothing addresses live-table use or redistribution. Obtain written confirmation for personal offline use in a private tool. (Live-venue rules on device use at the table are a separate matter for the user.)
4. **No published iterations / exploitability** for the NLHE cash packs; accuracy is unverified.
5. **Tree simplifications:** single open and 3-bet size per spot, no cold-call vs 3-bet for CO/SB, 5-bet is jam-only at 100bb; the exact bb amount per "X%" label must be resolved after purchase.
6. **Online rake model (5% / 0.5bb cap)**, not live rake; treat EVs as directional for a live game.

Recommended purchase path: one month of **API Pro ($100)** to pull the 100 / 150 / 200bb trees as JSON (freq + EV per action per class) via `/spots` → `/node`, cache locally (explicitly permitted), then cancel. Optionally add the **$250 one-time NLHE bundle** for a permanent raw copy with no subscription (requires a MonkerSolver-file parser). Validate schema and units first on the free Starter allowance (100 requests / 50 MB) before paying.

## 9. What the user must do personally (purchase steps)

1. Email michael@pokerdata.io before paying and ask for written answers to: (a) is there a licence / ToS, and is personal, offline use in a private desktop tool permitted; (b) NLHE 6-max pack solver settings (MonkerSolver version, iterations, exploitability, bucketing, raise-size semantics); (c) can 250bb and UTG-straddle NLHE 6-max packs be commissioned (also via https://www.pokerstudy.ai/sims/request) and at what price; (d) does Studio include the pdk_ ranges API.
2. Open https://pokerdata.io/app/dashboard — this mints a free Starter key (100 requests / 50 MB, no card). Run acceptance checks A1–A2 below on it.
3. Sign in with the email you want purchases keyed to ("Pack purchases are keyed to the email used at checkout").
4. Subscribe to API Pro ($100/mo) from the dashboard and mint a pdk_ key; and/or buy the NLHE 6-max bundle ($250) from https://pokerdata.io/sims/nlh-6max-100bb → "Buy NLHE 6-max" (Stripe checkout at `/checkout/nlh-6max`).
5. Keep the Stripe receipt (required for support and any refund request).
6. Store the API key outside the repo (never in client-side or public code, per docs). Download packs from `/downloads` on the purchasing device or via the Stripe receipt link.
7. After extraction is complete and verified, cancel the API subscription from the account page.

## 10. Acceptance checks on the data (before relying on it)

- **A1 Schema**: `GET /api/v1/ranges/nl/v2/node?stack=100&history=UTG` → fields `game, version=2, stack, actor, actions[]` each with `weights` and `evs`; 169 keys; weights in [0,1]; sibling-action weights per hand sum to ≈1 (allowing for "only nonzero weights are returned").
- **A2 Units**: confirm EV is in small blinds and its reference point: UTG RFI AA ≈ +9.15, KK ≈ +4.83 (explorer values); check fold EV for non-blind positions = 0, SB fold ≈ −1, BB fold ≈ −2 (SB units); confirm whether EVs are net of blinds posted.
- **A3 Sizes**: from `/spots?stack=100` and `/node`, record every raise label per node and resolve each to a bb amount (test both conventions: facing-bet + X%·pot vs call + X%·(pot after call)); confirm opens are 2.5bb-type sizes, not 1.9bb; do the same at 150/200bb.
- **A4 Completeness**: count nodes per depth tag (1..5+) at 100/150/200bb; assert presence of: all 6 RFI nodes, SB limp and BB-vs-limp nodes, every vs-RFI pair, 3-bet/4-bet/5-bet lines, squeeze nodes (e.g. `UTG_60%_HJ_Call_CO`), multiway nodes to 5 callers, and note the missing cold-call-vs-3-bet nodes for CO/SB so the app never asks for them.
- **A5 Consistency**: for a sample of nodes, weights and EVs from the API must match the explorer display and (if bought) the .7z pack parsed locally.
- **A6 Provenance**: query the MCP/API provenance envelope for timestamp, iterations, exploitability; if absent, record "unverified accuracy" in the app's data manifest.
- **A7 Sanity vs known solutions**: compare RFI frequencies and a few 3-bet/4-bet frequencies at 100bb against a reference (e.g. GTO Wizard 500z / Simple GTO screenshots); expect ±3 points; larger gaps indicate a size-semantics or unit error.
- **A8 Budget**: total requests and bytes for a full pull of three depths must stay under 250k / 5 GB; log 429s and honour `Retry-After`.
