# Adversarial specification review — 2026-09-10

**Verdict: revise before implementation. 8 BLOCKERs, 22 MAJORs, 2 MINORs.** Reviewed the entire specification, decision outline, SYNTHESIS, REVIEW-of-research, and R7; consulted relevant R1/R4 passages and upstream solver documentation. No application code was written, executed, or benchmarked. Only this report was changed. Section references below refer to the specification unless explicitly prefixed `Outline`, `Research review`, or `R7`.

Authority: the outline controls decisions; the research review corrects SYNTHESIS; R7 supplies preflop facts. In particular, the outline's one-time-pack purchase decision supersedes R7's API-subscription recommendation. The normalized-cache proposal below is the change explicitly requested by this review task; it requires a corresponding amendment to the outline's exact-chip-cache policy.

Already correct, retain: §§2/6 count an all-in third player as non-HU; §2 has 1,755 flop classes, orbit sizes 4/12/24 and total 22,100; §§2/10.4 prohibit substituting another canonical board; §§1.3/3.3 preserve the private-use AGPL decision and separate worker. These are not findings of contradiction. The failures below concern how those rules are implemented or qualified.

## Findings

### S01 — BLOCKER — Street-root history is combined with decision-point money

**Spec:** §§2, 4.5, 5 step 6 and final paragraph, 10.2–10.3. **Quote:** “Pot and effective stack are the actual chips from `Derived`”.

The solve starts at the street root and subsequently applies the current-street history, but `Derived` describes the decision point. After a bet of 73 into 100, sending pot 173 and reduced remaining stacks, then applying Bet(73), counts that bet twice and changes all future sizing. Current minimum remaining stack is also not the street-root effective stack.

**Proposed edit:** Introduce a derived `StreetRootSnapshot` containing board, OOP/IP seats, pot before this street's first action, both stacks then, incoming ranges, and a separate ordered current-street history. Worker starting pot/stack must come only from this snapshot. Assert that replaying history reproduces current pot, contributions, remaining stacks and actor. Keep current `Derived` for legal-action validation and the decision-point EV reference. Provide the numerical fixture in T2; never mix a decision-root model with street-root replay.

### S02 — BLOCKER — Pruning observed-action alternatives removes the evidence needed for range conditioning

**Spec:** §§2, 9.2, 10.2, 13.2 `exact_size_insertion_and_prune`. **Quote:** “prune every sibling line of every taken action”.

Forcing an observed bet to be the only action makes its probability one for every surviving combo. A solve can then report low exploitability while losing the opponent's bet-versus-check information. This contradicts Outline §3's insertion of observed amounts with only future actions abstracted and undermines later Bayesian replay. The existing test explicitly rewards the bad pruning.

**Proposed edit:** Delete observed-prefix sibling pruning. Insert the exact observed wager alongside legal alternatives, solve from the street-root incoming ranges, and navigate to the observed decision. Bound complexity by reducing future size menus. If a decision-root solve using externally conditioned ranges is added later, give it a separate contract and approximation reason; do not silently obtain it by forcing the history. Change the test to require both the inserted wager and the relevant counterfactual alternatives, plus a nontrivial posterior range.

### S03 — BLOCKER — Hero-card removal corrupts the strategic game and can erase the requested combo

**Spec:** §§4.4, 8.3 final bullet, 9.2, 13.1 `blockers_zero_board_combos`. **Quote:** “every dealt seat has the uniform range … minus combos blocked by hero's cards and the board”.

This removes hero's actual holding from hero's own range. Applying hero-card blockers only to opponents still conditions the public solver game on information the opponent cannot observe, contrary to keeping hero's strategic range. Separately, a legal hero holding can have zero replay weight while the total range remains nonzero; the mass guard does not ensure an action/EV exists for that holding.

**Proposed edit:** Maintain public strategic ranges for solving, blocked by public cards only; enforce pairwise hole-card compatibility during evaluation. Make separate hero-conditioned copies of opposing ranges for actual-combo equity and terminal call calculations. Never apply actual hero cards to the public solve or presolve key. Define `HeroComboOutOfSupport`: show equity and no fabricated combo strategy/EV, unless an explicitly labelled support-completion policy is later specified. Split the blocker test into public-range and hero-conditioned-equity cases.

### S04 — BLOCKER — PokerData EV scaling uses the live SB ratio and omits reference normalization

**Spec:** §§2, 8.2, 13.1 `pokerdata_adapter_units_sb_to_bb`, 14.4 V9. **Quote:** “`ev_bb = ev_sb * sb_chips / bb_chips`”.

PokerData's source game has SB/BB = 0.5/1 (R7 §§1/4). A 2/5 live table must not turn 1.84 source SB into 0.736 bb: it is 0.92 source bb. R7 A2 also leaves the EV reference relative to posted blinds to be verified. Merely multiplying cannot guarantee the spec's fold-zero incremental convention; later preflop commitments matter as well as blinds. Straddle mapping introduces a further source-BB-to-live-chip conversion that is absent.

**Proposed edit:** Store source monetary units and verified EV-reference metadata in the manifest. Normalize source EV to decision-incremental units first (subtract source fold EV at a fold-legal node if A2 confirms a common origin), then multiply by **source** SB/source BB = 0.5. Ordinary mapped source bb corresponds to live `bb_chips`; straddle-mapped source bb corresponds to `straddle_chips`; divide by live BB only for display. No-check/no-fold nodes need an explicit commitment-offset rule. Gate numeric preflop EV on A2; preserve frequencies if reference semantics remain unknown. Test 1/2 and 2/5 sessions, both blind seats, re-raised nodes, and straddle mapping.

### S05 — BLOCKER — Cache hits can return the wrong node, actor, or EV owner

**Spec:** §§4.5, 5 step 6, 10.2, 10.4. **Quote:** “when nothing was inserted, `tree_id = template_id`”.

An in-tree history is absent from the key, yet the cached `Result.node` is the requested decision node, not necessarily the street root. Check changes the actor without changing pot, stacks, board or ranges. Hero's OOP/IP role is also absent although the payload is hero-specific. With S01 corrected, every in-tree decision in a street-root solve shares monetary inputs. Presolves exacerbate this by caching a root response for later non-root requests. Solver commit/adapter-EV version and lock/model identity are absent too.

**Proposed edit:** Store actor-owned strategy/EV at identified nodes, independent of hero. Lookup must select and validate the requested ordered action path and actor; a missing node is a miss. Include full structural identity and versioning from replacement C1 below. Either key an individual node by its path or store a navigable street solution; never serve the saved request's `Result.node` indiscriminately. Resolve this before relaxing chip matching.

### S06 — BLOCKER — The hard deadline is not enforced end to end

**Spec:** §§1.1, 7, 10.5, 12, 13.2/13.5. **Quote:** “At the deadline the worker finalizes and returns `BestSoFar`”.

Finalizing at expiry contradicts reserving finalization time. Subtracting only fast-phase elapsed time ignores admission, cancellation, cache reads, worker startup and serialization. A single build/allocation/iteration/exploitability calculation can exceed its allowance. Progress contains no recoverable strategy; a hung worker cannot always supply best-so-far. A 1,000 ms test permits 1,100 ms, and p95 compliance cannot establish a hard deadline.

**Proposed edit:** Timestamp admission once with a monotonic engine clock. Set absolute engine deadlines: first-attempt completion at `t0 + {2,6,10}s`, final delivery at `t0 + 15s`. Every phase/retry receives only remaining time, including pipe transfer and extraction. Reserve a measured delivery margin (initially 100 ms) and extraction margin; do not begin another iteration unless it fits. Independent engine watchdog emits a retained, validated result, or `Unsupported{DeadlineExceeded}` with available equity, before final delivery expiry; cancellation/kill/restart proceeds independently. Best-so-far is available only when a complete validated payload exists. Use monotonic elapsed durations, never cross-process wall timestamps; after suspend/resume expire the request and reject late replies. Record actual max deadline violations as failures, separately from p95 and solution-quality gates.

### S07 — BLOCKER — Worker cancellation and terminal replies lack an implementable state machine

**Spec:** §§3.4, 4.5, 7, 10.3, 12. **Quote:** “`Cancel` -> `Result{Cancelled}` within one iteration”.

Engine pipe threads do not let a worker's blocking solve loop read stdin. `Ack` may merely acknowledge receipt, yet admission permits `Ack` **or** cancellation completion to release the slot. Error/Cancelled replies require a nonoptional `NodeStrategy` even if allocation failed. Staged locks have no lifetime, consumption rule or compatibility check. Duplicate IDs, a Solve while busy, cancel-after-result, EOF, truncated/oversized lines, startup timeout and stdout logging are unspecified.

**Proposed edit:** Define a worker control reader independent of its solve executor, one serialized stdout writer, a bounded latest-live-request queue, and states Starting/Idle/Building/Solving/Extracting/Stopping. Ack acknowledges receipt only; terminal Result or confirmed process exit releases admission. Every accepted Solve produces at most one terminal response; Error/Cancelled has no mandatory strategy. Specify duplicate/busy/unknown-target responses and EOF-triggered shutdown. Locks belong to a structural spot fingerprint, are consumed by exactly the next matching solve, rejected after solve start, and cleared on cancellation/error/restart. stdout is protocol-only; drain bounded stderr separately. Add the wire examples and validation limits in C2. Launch/startup arguments must carry thread count, and `Ready` must confirm it.

### S08 — BLOCKER — Version-only stale-result checks fail across undo, new hands, and config changes

**Spec:** §§3.5, 4.2–4.4, 5 steps 2/3/8, 9.1, 12. **Quote:** “Results whose `state_version` differs … are discarded”.

Different hands can share a version. Popping an old snapshot then incrementing its version can reuse a previously issued number. Re-requesting the same state has no distinct UI acceptance token. An undo to a non-hero turn need not issue another recommend, so old work may continue. `set_config` changes implied blind posts while an old hand retains only `config_id`; the engine owns no specified immutable config snapshot. Stale solves can contaminate replay even if their UI event is discarded.

**Proposed edit:** Accept events only for the active tuple `(hand_id, monotonic hand revision, decision_id, config_revision, model_revision)`. Revisions increment from a counter outside undo snapshots; a repeated recommend gets a new decision ID. Every mutation/new hand/undo explicitly cancels and invalidates descendants, even if there is no next decision. Freeze blinds/rake/stack-start configuration for each hand; session changes take effect next hand. If correcting an active hand's config is supported, rebuild from its start under a new revision. Apply the same identity checks before updating replay snapshots or publishing a result; discard stale Fast and Progress too.

### S09 — MAJOR — Raw-chip exact keys defeat the live-table cache; rounded range hashes already violate “exact”

**Spec:** §§2, 10.4, 13.1 `cache_key_exact_match_only`. **Quote:** “`pot_chips, eff_stack_chips, bb_chips, rake_rate, rake_cap_chips`”.

Equivalent scaled games miss, nonstandard live pots rarely equal presolve pots, and the raw-chip tree hash retains the same problem after replacing just the pot field. Conversely, u16/10000 range hashes alias distinct distributions without a distance check. “Any single field change misses” contradicts that quantization.

**Proposed edit:** Replace §10.4's key, matching rule, payload conversion and key test with C1 below. Use a pot-normalized candidate index, explicit grids and verified match tolerances; keep actual chips in `HandState`. Add `CacheQuantized`/`RangeQuantized` reasons and preserve all earlier approximation reasons. This is an intentional, user-requested change to Outline §3's exact-chip cache, not an interpretation that its current wording already permits approximate reuse.

### S10 — MAJOR — Data acquisition silently switches the chosen product and repeats settled uncertainties

**Spec:** §§1.3, 8.2, 14.4 V9, 15 risk 2, 16 question 3. **Quote:** “reads the raw JSON files saved by `tools/pokerdata_fetch.py`”.

Outline §0 chooses one-time packs; R7 §4 says those are `.7z` Monker exports with undocumented internal layout, not the API JSON consumed here. No conversion path is specified. V9 and §16 still call provenance/squeeze coverage unknown despite R7 verifying MonkerSolver and those branches. No 250bb/straddle/ante packs exist; §14.3 omits the outline's explicit custom straddle/250bb backlog.

**Proposed edit:** State the selected one-time-pack path and an acquisition gate: verify an authorized sample can be converted into the normalized local node format with frequencies **and** EV. Time-box raw-format investigation; keep the chart baseline usable if blocked. Do not assume pack ownership includes API access or require a subscription silently. Update V9 to R7 A1–A6: schema, units/reference, absolute sizes, actual acquired completeness, provenance metadata and written personal-use rights. Mark MonkerSolver and public branch facts resolved; distinguish SB-limp/BB-vs-limp support from absent open-limp/cold-call branches. List all eight source depths and explicitly defer actual straddle/250bb trees to phase 2.

### S11 — MAJOR — The PokerData adapter contract omits field and action-path semantics

**Spec:** §§8.1–8.3, 13.1 adapter tests, 14.4 V9. **Quote:** “Uses the provider's documented action path for node identity”.

An implementer must guess class order, sparse-field semantics, array orientation, current actor encoding, the meaning of `reach`, and percentage-to-chip conversion. `PreflopStep` lacks Check, although BB can check after SB limps. R7 explicitly warns that the explorer's percent labels have unresolved absolute-size semantics; blindly applying §2's formula may encode 1.9bb versus 2.5bb opens incorrectly.

**Proposed edit:** Add a mapping table: `game/version` validate NL/v2; `stack` is source bb; `actor` is node player-to-act; `weights[class]` is conditional action probability, absent key = 0; `evs[class]` is action EV in source SB, absent key = unavailable, never zero; `combos` is aggregate weighted mass, not probability or reach. Declare the 169-class order and equal 6/4/12 combo expansion. Remove `reach` unless its independent source/derivation is defined. Preserve every explicit Fold in history; add Check and distinguish decision-node history from an action-ending `spot`. Pin an acquired `/node`, `/range` and `/spots` example with exact field names/action tokens, sparse zero-weight EV and SB-limp/BB-check. Resolve R7 A3 before numeric size mapping; do not invent missing vendor fields.

### S12 — MAJOR — Bet translation mixes incompatible continuations and can output illegal advice

**Spec:** §§2, 8.3 Sizes, 9.2, 10.2, 13.3. **Quote:** “`probs = f_A * probs_A + f_B * probs_B` and likewise for EV”.

Adjacent raise branches can have different actors' subsequent menus, lengths, minimum raises, pot/stack references and node availability. Array-wise EV mixing is undefined. PokerData often has only one raise size, so two adjacent branches do not exist. The definition suppresses `BetTranslation` at deviation ≤0.10, but §8.3 emits it for every mismatch. Later actions along a translated branch have no traversal rule. Bucket/straddle-derived recommendations can exceed the actual stack or fail the actual min-raise.

**Proposed edit:** Separate (a) likelihood interpolation for an observed action at one parent from (b) translating a whole continuation/node. Carry branch IDs and weights through later history, using stable actor/action identities; do not zip arrays. Define A=B, zero-size, one-size and all-in boundaries explicitly; with a single size use a labelled clamp. If a necessary continuation is absent, apply S15 rather than renormalizing surviving branches silently. Every nonexact mapping records its reason/deviation; 0.10 controls warning prominence only. Return only legal chip actions after mapping; merge identical rounded actions, and never renormalize away an illegal positive-probability recommendation as though the source solution were unchanged. Suppress numeric EV where no common payoff/action reference has been established. Missing actions must not become guessed folds.

### S13 — MAJOR — Depth and rake selection can change during replay and mislabel asymmetric games

**Spec:** §§2, 8.1, 8.3, 13.1 depth tests. **Quote:** “`min(hero_stack_start, max over unfolded opponents' stack_start)`”.

Definition §2 is HU, but the lookup is six-player and hero-dependent even when querying another seat. “Unfolded” can mean at the historical node or in the final state, creating hindsight: an opponent's later fold changes earlier node selection. Uniform-stack packs do not exactly represent unequal active stacks. At straddle mapping, depth switches units after depth selection is described. Rake chooses “closest” by cap only, ignoring rate, no-flop-no-drop and ties. Fallback order across bundles/depths is incomplete.

**Proposed edit:** Reconstruct every historical node using only its prefix; determine its actor first. For a declared phase-1 scalar approximation use `min(actor_start_stack, max(other players still eligible at that prefix: start_stack))`, in the selected source-blind unit. Cache the mapping per prefix; never recompute using later folds. Record the full relevant stack vector and `AsymmetricStacks` when it differs from the pack model. Choose nearest acquired depth, ties deeper; define the 5% test as `abs(actual-used)/used <= .05`, including endpoints, and label every unequal depth as `DepthBucket` per Outline §3 (5% may control prominence). Out-of-catalog depths clamp with a reason, never imply a 250bb pack. Specify source precedence and stable tie breaks. Prefer exact rake tuple; otherwise use a fixed documented tuple-distance ordering across rate, cap and collection rule, always labelled, including TimeCharge.

### S14 — MAJOR — Straddle mapping fails to distinguish physical positions, virtual roles and money

**Spec:** §§4.1–4.2, 8.3 Straddle, 13.1 straddle tests. **Quote:** “SB and BB posts (0.25 and 0.5 straddle units) are not represented”.

Those fractions assume SB/BB/straddle = 1/2/4; at 2/5/10 they are 0.2/0.5. The virtual non-straddle tree has its own SB/BB posts, so the mismatch is not just deleting two posts. The mapping moves BTN to virtual CO and physical SB to virtual BTN, changing positional meaning. There is no rule for returned source EV, rake cap, all-in amount, short straddle or supported seat count.

**Proposed edit:** Define physical positions clockwise from a six-handed button as BTN, SB, BB, UTG, HJ, CO. UTG retains that physical name when straddling; physical preflop order is HJ, CO, BTN, SB, BB, UTG; postflop order is SB, BB, UTG, HJ, CO, BTN, skipping folded/all-in seats. Keep the six physical→virtual mappings in §8.3 as lookup-only roles; never rotate actual cards, posts or OOP/IP. Source BB = straddle amount for depth, sizing, cap and EV conversions. Report actual normalized posts `(SB/S, BB/S, 1)` and virtual posts by mapped seat, plus the changed positional assumptions. Phase 1 may require a fully posted UTG straddle with S≥2BB and six dealt seats; explicitly reject unsupported short posts/re-straddles rather than guessing their order. Test 2/5/10 as well as 1/2/4.

### S15 — MAJOR — Replay cannot identify applicable strategy snapshots or recover missing branches consistently

**Spec:** §§5, 9.1–9.3, 10.4, 12, 15 risk 2. **Quote:** “`StreetStrategies /* by (hand_id, street) */`”.

Several decisions on the same street create different trees and histories; one `(hand_id,street)` slot cannot establish compatibility. `report_street_strategy` is optional with no dispatch rule, yet turn/river replay requires it. A missing preflop node is both always Unsupported (§12/15) and Approximate (§9.3). Omitting one historical action can make all later provider paths unavailable. Per-street deadline/depth/chart reasons are not stored in `NodeStrategy`, so they can disappear on a later exact solve/cache hit. Absolute reach weights plus the global mass threshold also turn long rare lines into silently unconditioned priors.

**Proposed edit:** Index snapshots by hand/config/model revision, street-root board/range identity, effective tree and covered paths. Retain compatible snapshots only; invalidate descendants after corrections. Request/store current-street strategies for every successful solve used as a replay source. Replay each action exactly once from the prefix-specific strategy, never multiply a node's reach and its constituent action probabilities twice. Missing node/path: preserve the pre-action range, stop traversal of unavailable descendants until an explicitly matching source resumes, and record seat/action/path/source reason. Distinguish current preflop lookup Unsupported from postflop replay Approximate in §§12/15. Carry provenance/accuracy reasons with replay snapshots. Keep separate normalized posterior and log reach mass for numerical guards; valid low-probability lines are not automatically impossible.

### S16 — MAJOR — Facing-all-in shortcut uses the wrong equity and unconditionally claims Exact

**Spec:** §§2, 5 step 6, 6, 9.2. **Quote:** “`EV(call)` from exact range-vs-range equity over all runouts with rake, `Exact`”.

Headline EV must use hero's actual holding, not aggregate strategic-range equity. Current-street actions, including the jam, have not been conditioned by completed-street replay. Approximate prior ranges remain approximate even if showdown enumeration is exact. Call cost, unmatched excess, tie shares, action frequencies and hero's own all-in/no-choice branch are missing from the flow. A preflop call/fold decision remains a store decision under the outline.

**Proposed edit:** For postflop genuine HU facing all-in, obtain a declared opponent range conditioned on the observed prefix/jam (or add an unconditioned-current-street reason). Let C be the actual payable call and W the final matched pot after returning excess and adding C; compute `EV(call)=E[hero's share of W after terminal rake]-C`, `EV(fold)=0`. For fixed W/rake R this is `equity_of_actual_combo*(W-R)-C`, including ties. Preserve inherited approximation reasons; Exact concerns enumeration conditional on the stated model. Emit pure optimal call/fold frequencies, with a declared deterministic tie rule. Hero all-in/folded, no to-act or only one legal action means NoDecision, no solve. Validate this server-side too. Preflop uses the store/missing-node rule.

### S17 — MAJOR — Pot settlement and raise-reopening tests encode incomplete or wrong rules

**Spec:** §§3.2, 4.3, 6, 13.1 model tests. **Quote:** “Three all-ins with different stacks produce main + two side pots”.

With exactly three contributors committed 50/100/200, the last 100 is uncalled and returned: main 150, contested side 100, not another contested side pot. Treating excess as pot money corrupts EV. “A short all-in does not reopen” is incomplete for cumulative short raises faced since a player's last action. Side-pot eligibility is modeled but equity output does not specify each pot's eligible set. A two-player side pot with a third main-pot claimant must remain unsupported for solved EV.

**Proposed edit:** Define unmatched-bet returns before constructing settled pots; conserve `sum(remaining stacks) + unawarded pot chips + rake already collected = sum(starting stacks)`. Returns and awards transfer pot chips back into stacks; they are not additional terms. Replace the three-all-in expected result with 150/100/refund100, or explicitly add a fourth matching contributor if testing two contested side pots. Track each player's last-action facing amount and the last full raise; define cumulative reopening under the selected cash rules, with independent fixtures. Provide per-pot equity shares for unequal eligibility and retain the 3+ player EV prohibition even when only two players can bet. Define automatic street closure when no further betting is possible.

### S18 — MAJOR — Chip denomination, rounding, rake and integer boundaries are inconsistent

**Spec:** §§2, 4.2–4.5, 8.1–8.2, 10.2–10.3. **Quote:** “all `HandState`, worker and cache amounts are integer chips (`u32`)”.

EV is signed/fractional in the actual protocol; preflop values are already bb despite “only for display.” A 0.5bb rake cap at a 5-chip BB is 2.5 chips, unrepresentable as `u32`. No smallest chip unit, split-pot odd-chip rule, rake rounding/collection convention or safe aggregate limit is selected. `to_bb_x100` loses distinctions, while library stack/pot inputs have a narrower signed domain. Size rounding can create a below-minimum raise or duplicate Call/AllIn.

**Proposed edit:** Define an integer accounting tick and legal wager quantum per session; blinds/stacks/actions use checked integer ticks, EV and modeled rake use signed finite floating-point ticks. State display precision (e.g. 0.01 bb, never used in calculations). Preserve exact observed raise-to as total street contribution; paid delta is new contribution minus prior contribution. For future template actions: round-half-up to wager quantum, enforce min/max/full-raise rules, merge duplicates, preserve exact observed actions. Define rake on matched gross terminal pots, cap once per hand, no-flop-no-drop and odd-chip allocation; disclose continuous solver rake as approximate if the room rounds differently. Reject overflowing sums/library conversion; keep rational source sizing until the final chip conversion. Cache the rule version and normalized quantum.

### S19 — MAJOR — The equity phase has unbounded work and ambiguous multiway output

**Spec:** §§3.5, 4.4, 5 step 4, 6, 7. **Quote:** “exact enumeration on turn/river, Monte Carlo with 50,000 samples preflop/flop”.

Exactness does not imply sub-300ms computation, particularly across broad multiway ranges. Fixed sample count does not guarantee a deadline, and no API accepts a deadline/cancel flag. Individual nonempty ranges can have no jointly compatible hand assignment. Pairwise equities do not describe multiway shares or side pots. Optional multiway HU solving can also have zero effective stack if the chosen opponent is all-in.

**Proposed edit:** Give equity its own cancellable time budget; enumerate only below a measured workload bound, otherwise time-bounded MC with actual samples and standard error. Fast must allow equity pending/unavailable. Sample joint mutually disjoint hole cards with correct weights and runouts; return `InvalidRanges` when compatible joint mass is zero. Distinguish actual-hero equity, range equity, pairwise diagnostics and per-pot multiway shares. Select the strongest range only from a defined compatible metric; specify the experimental surrogate's pot/stack assumptions and skip it when terminal or out of budget. Never attach its numeric EV to main action advice.

### S20 — MAJOR — Recommendation and coverage types cannot express the stated flow

**Spec:** §§2, 4.4, 5 steps 4/8, 6–7. **Quote:** “`actions` frequencies empty and `ev_bb = None`”.

`frequency: f32` cannot be empty while still returning legal actions. `EquitySummary` is mandatory even before equity is available. Approximate supposedly implies numeric EV but charts have none. A chart has no highest-EV headline. A present PokerData node is called Exact despite R7's unverified source accuracy and the definition requiring a reached solver target. §§5/10.4 can replace accumulated reasons with only accuracy/stored coverage. A finite tree cannot provide values for every legal continuous bet size.

**Proposed edit:** Separate legal action intervals from the evaluated action menu; make frequency, EV, equity and measured accuracy explicitly optional with availability reasons. Define NoDecision and Fast/Progress/Final event payloads, including the S08 identity. Coverage is about input/model matching: approximation does not promise EV availability, and Exact means exact within the declared abstraction/tolerances, not proven full-game GTO. For source lookups show `source_accuracy=unverified`, `source_granularity=169-class`, no invented reached target. Accumulate source/replay/cache/solve reasons on every path. Label chart headline “highest-frequency chart action”; no EV ranking when EV is absent. Compare raw exploitability against the target before rounding bp for display.

### S21 — MAJOR — Named combos alone do not define the solver adapter's matrix and EV contract

**Spec:** §§4.1, 4.5, 10.3, 13.2. **Quote:** “per-combo action probs + per-combo per-action EV chips (hero)”.

The upstream API returns action-major arrays over a compact private-hand list; per-action EV is available for the current actor, whereas requesting the other player returns a different shape. Board-blocked strategies may be undefined. A one-combo test cannot expose a transpose, and the fold-zero test alone cannot catch an erroneous offset applied to all nonfold actions. [Upstream `PostFlopGame` API](https://b-inary.github.io/postflop_solver/postflop_solver/struct.PostFlopGame.html#method.expected_values_detail)

**Proposed edit:** Specify compact-library-index ↔ named combo ↔ 1326-index conversion, action-major→combo-major transpose, actor-owned EV for each street node, and explicit unavailable masks for unsupported/zero-reach combos. Validate the requested node actor against `Derived.to_act`; recache normalized weights after every navigation used for EV extraction. Pin `normalize_ev` to the chosen commit with positive, negative, check and fold payoffs at non-root nodes; do not apply a speculative extra pot/commitment offset. Supply at least two named combos with different action rows and EVs. The inspected source already contains action-specific normalization, so its exact commit must be inspected rather than inferred from method names. [Upstream interpreter](https://raw.githubusercontent.com/b-inary/postflop-solver/master/src/game/interpreter.rs)

### S22 — MAJOR — Hash validation does not make bundles/cache payloads valid or bounded

**Spec:** §§8.2, 10.4, 12. **Quote:** “Bundle not loaded; startup banner names the bundle”.

Manifest mismatch is handled, but valid-hash malformed JSON, wrong shapes, nonfinite values, absent mandatory EV, invalid action paths, decompression failures and partial writes are not. Cache decompression, serialization and disk writes can stall the hard deadline. There is no response to disk-full, read-only cache, locked files or unbounded cache/log growth. Deleting a bad entry can itself fail.

**Proposed edit:** Validate both content and schema before atomic bundle activation; reject/quarantine only the bad bundle, retaining chart fallback with an explicit reason. Bound file, decompressed and decoded allocation sizes. Treat all cache read/decode errors as misses; deletion failure is nonfatal. Write cache entries through a temporary sibling and atomic replacement after validation, off the recommendation path; cache/log write failures preserve the valid recommendation. Put a default disk quota (e.g. 10 GiB) and deterministic eviction policy in settings; rotate the decision log. A matching hash never substitutes for range/action/EV checks.

### S23 — MAJOR — Memory estimates, OOM behavior and “warm allocations” disagree

**Spec:** §§3.1, 10.3, 12, 14.4 V3. **Quote:** “allocations stay warm; the game object is dropped after `Result`”.

Keeping the process alive does not promise solver arena reuse after dropping the game. The 4/8 GB limits do not state whether compressed or uncompressed estimates, tree construction, bunching, extraction and JSON copies count. OOM may terminate the worker before an Error can be serialized. Killing/restarting and retrying the same allocation pattern can consume all remaining time. Upstream reports separate compressed/uncompressed and bunching memory estimates. [Memory API](https://b-inary.github.io/postflop_solver/postflop_solver/struct.PostFlopGame.html#method.memory_usage)

**Proposed edit:** Call persistence “warm process”; no arena-reuse guarantee unless V2/V3 measures an explicit supported reuse path. Select f32 only if its estimate fits 4 GiB, otherwise compressed only if its own estimate fits the declared limit; cap total worker working set with headroom for tree/scratch/extraction/serialization. Check budget before allocation and account for startup tree construction. Treat process OOM/abnormal exit as a typed engine failure: retain fast output, kill/reap, retry once with the smaller template only if time/memory admission passes, then return Unsupported. Never rely on catching an allocation abort. Keep maximum serialized strategy size within the extraction budget.

### S24 — MAJOR — Background presolving is underspecified and overpromised for two weeks

**Spec:** §§3.4, 4.5, 10.4, 14.2–14.3. **Quote:** “`target_bp` 50, no deadline”.

The worker requires a finite u32 deadline. “Pauses instantly” contradicts iteration-boundary cancellation. Presence means completion even for BestSoFar entries that are supposed to be requeued. A cursor cannot distinguish stale configurations, failed jobs or corrupt entries. The scenario list contains `...`; 48 scenarios and 10 seconds to convergence are unverified. Exhausting one scenario over every flop before the next may postpone other common scenarios for days. There is no hand-end transition to enable the idle rule.

**Proposed edit:** Ship a small explicit scenario list for 100/200bb, interleave common scenarios over canonical flops, and persist task status keyed by the normalized game identity. An entry completes a task only when valid and at target. Use finite cancellable background slices with bounded retries/backoff, or explicitly represent background/no-live-deadline in the protocol; live preemption follows S06/S07. Report estimated remaining work from measured time-to-target and hit rates. Keep the resumable queue in week 2, but full 84k completion, a generalized scheduler and guaranteed coverage are not release requirements (Outline §10 defers library breadth). Add S30's explicit hand-end transition.

### S25 — MAJOR — Tree templates do not determine a reproducible legal tree

**Spec:** §§2, 4.5, 10.1–10.3. **Quote:** “`merging_threshold`”.

That parameter is passed but has no value. Donk defaults, the denominator of all-in thresholds, inclusive comparisons, raise-cap counting and ordering of insertion/rounding/pruning are absent. A merge or forced-all-in rule may remove the exact observed action. Templates may exclude a legal observed raise because a cap has already been reached. The upstream add-all-in and force-all-in settings use different ratios. [TreeConfig definitions](https://b-inary.github.io/postflop_solver/postflop_solver/struct.TreeConfig.html)

**Proposed edit:** Define `EffectiveTree` fully: per-player/per-street bet/raise/donk menus, all-in rules, merging (start with zero), cap semantics and deterministic action order. Encode 150% as 1.5 and specify the upstream maximum-bet/pot and post-call-SPR tests exactly. Count full raises explicitly; define all-in and short-raise treatment. Apply limits to future branches, preserving every legal observed prefix. Exact inserted sizes cannot be merged/forced away. Hash the materialized structural tree plus semantic version, and pin backend commit/features and defaults before V1. If a legal history cannot be represented, return a typed unsupported-history reason rather than silently changing it.

### S26 — MAJOR — The analytic solver oracle is numerically wrong; other tests can certify weak invariants

**Spec:** §§13.1–13.2, 14.4 V1. **Quote:** “QQ weight 1 + 54o weight 0.1875 (equal mass)”.

On Qs Jd 7h 3c 2d, QQ has 3 legal combos, while 54o has 12: the stated masses are 3 and 2.25. They are not equal. With the stated one-bet toy game, the equilibrium bluff rate is 2/3 of 54o and bettor/caller EVs are 600/7 and 100/7 chips, not 50% and 75/25. V1 therefore blocks a correct implementation. EV sum = pot is conservation, not positional EV symmetry. MC(50k) within 0.5 pp on every fixture can fail by sampling fluctuation. The published benchmark test checks only aggregate betting, missing EV/unit errors.

**Proposed edit:** Change 54o weight to **0.25**, giving 3/3 mass and the stated 50%/75/25 oracle; explicitly prohibit raises and OOP leads in that fixture. Rename the symmetry check as conservation and add an actual payoff/seat-swap oracle. Use sample-error-aware MC assertions with deterministic seeds. Freeze the full published board/ranges/pot/stacks/tree and compare per-combo/action results plus aggregate EV, not just bet frequency. The source comparison reports bet ≈55.2% and EV ≈105.1 at its stated setup/accuracy. [Published comparison](https://github.com/b-inary/wasm-postflop#comparison) Add T1–T5 below; existing named tests are insufficient without their numeric and temporal assertions.

### S27 — MAJOR — Release gates can pass on fallbacks, omit required templates, or require data unavailable before release

**Spec:** §§7, 13.5, 14.4 V2/V3/V21. **Quote:** “50 recorded hands from `decisions.jsonl`”.

There may be no real decision log before implementation/release; §5's stated log fields do not guarantee reconstructible inputs. Five spots give weak p95 information, no repeat count is specified, turn/river minimum templates are absent from the bench inputs, and quick Unsupported outputs can satisfy the e2e latency gate without usable coverage. Target-time failure and hard-stop failure are different outcomes.

**Proposed edit:** Commit a predeclared synthetic/independent recorded-hand suite now as a spec requirement, covering both depths, straddle mapping, broad/narrow ranges, high/low SPR, off-tree paths and failure cases. Run every production/retry template with stated cold/warm repetitions and thread configurations. Report p50/p95/max end-to-end time, time-to-target, peak total RSS, Exact/Approximate/Unsupported proportions and cancellation latency. Require zero hard-deadline violations in fault-injection runs and successful numeric output for designated supported fixtures. Augment with actual-session logs later; define a complete versioned input record if logs are to be replayable. Bench failure reduces template/scope, not silently the definition of success.

### S28 — MAJOR — Optional bunching and exhaustive normal-test workloads threaten the baseline critical path

**Spec:** §§4.2/4.5, 6, 10.3, 13.1, 14.1. **Quote:** “`bunching: Option<Vec<Range1326>>`”.

Default-off bunching nevertheless has a public toggle, worker input and cache dimensions, but no data-preparation contract or latency gate. It creates work beyond the outline's mandatory baseline; “folded players may be bunched” is permissive. Ten million 7-card cases/oracle tables in normal `cargo test`, plus an evaluator fallback feature, can consume integration time without addressing the EV/replay failures above. Full library population is a similar scope trap (S24).

**Proposed edit:** Phase 1 explicitly fixes bunching=false; retain folded ranges and a reserved versioned capability, but no usable toggle until preparation, correctness, memory and deadline tests pass. Keep the primary evaluator, modest fixed-seed 7-card tests and required 5-card validation; run larger oracle generation/exhaustive suites as a separate release/offline command instead of every edit cycle. Defer alternate evaluator implementation, general lock tooling, quota dashboards and full presolve completion. Keep the outline's core state/range/EV checks and Windows E2E; do not trade them for optional breadth.

### S29 — MAJOR — The optional exploit slice mutates the baseline and reports an EV delta for another policy

**Spec:** §§11, 4.4, 9, 14.2. **Quote:** “Applied inside replay before postflop solving”.

Tags alter the only ranges, so the displayed GTO baseline can change when a tag is edited. The displayed policy is a 25% mixture, but the EV delta compares two headline actions under the locked response's continuation, not both complete policies under a frozen common model. More than one hero river decision makes that difference consequential. Lock/fact composition, unreachable combos and partial opponent locks are unspecified. R4 details are substantially larger than this slice.

**Proposed edit:** Keep immutable baseline and modeled replay snapshots with separate model revisions. Do not let the optional slice overwrite baseline cache/replay artifacts. Implement numeric exploit delta only if both complete policies can be evaluated under the same fixed opponent model and range distribution; otherwise omit the delta and describe only the experimental action mix. Define tag/fact precedence and feasible probability transformations before enabling locks. Defer the entire slice to phase 2 unless baseline gates pass with time remaining, as the outline permits; do not import R4's full auditor or unvalidated transforms into the baseline dependency chain.

### S30 — MAJOR — Session stacks, hand completion and partially entered states lack transitions

**Spec:** §§3.5, 4.2–4.3, 5.1, 10.4. **Quote:** “`stack_chips at session start`”.

`new_hand` accepts no new stacks, yet stacks vary every hand and determine depth/ranges. There is no finish/abandon hand or per-hand stack confirmation, although idle presolving requires no hand in progress. Unknown villain cards prevent automatic showdown settlement in many manually entered hands. Board entry supports adding cards but the command alternates between whole-board and incremental semantics. `hero_cards` is optional while headline advice assumes a known holding; fewer occupied seats have no preflop-format mapping.

**Proposed edit:** Add an explicit hand lifecycle: begin with confirmed per-seat starting stacks and dealt-seat set; betting; awaiting board; terminal/abandoned. On completion permit manual next-hand stack confirmation rather than requiring showdown/card/payout entry; no automatic reuse of session-start stacks. Specify `set_board` as replacement of the full validated board (UI appends locally), legal only at the appropriate transition or explicit correction. No combo advice without two legal hero cards. Either require six dealt seats for phase-1 preflop coverage or add a labelled unsupported-format rule; do not equate vacant seats with known folding ranges. Finish/abandon cancels outstanding work and enables idle scheduling.

### S31 — MINOR — Public-interface sketches disagree with their declared types and ownership

**Spec:** §§2, 3.2–3.5, 4.1–4.5. **Quote:** “`solver-worker` depends on `proto` only”.

The dependency table also lists the vendored solver; `pokerai-app` and engine both own worker lifecycle; Range is variously a fixed array and Vec; `RecommendationEvent`, `Phase`, `EquitySummary`, `Assumptions`, `EffectiveTree`, `SolveStatus` and several error types lack definitions. These invite separate, incompatible implementations despite “single source of truth.”

**Proposed edit:** Say “only project protocol dependency is proto; additionally links the vendored solver,” assign process ownership to engine and lifecycle delegation to the app, and choose one validated 1326-vector representation at boundaries. Add one complete type/schema appendix with all referenced enums, optionality, indexing and wire tags, using S07/S20 corrections; identify sketches as illustrative only where deliberately incomplete.

### S32 — MINOR — Suit canonicalization needs ordered streets and a stabilizer tie-break

**Spec:** §§2, 3.5 `canonical_board`, 10.4, 13.1 iso tests. **Quote:** “turn/river extension of a canonical flop”.

A flop can admit several suit permutations to the same representative. Choosing one using raw suit order can prevent suit-equivalent range states from hitting the same key. Treating four/five board cards as an unordered set can also alias different flop/turn histories. Round-trip/count tests alone do not rule this out.

**Proposed edit:** Canonicalize the unordered flop followed by ordered turn and river; among permutations with the same canonical board select the lexicographically minimal serialized public range tuple (including folded ranges when enabled), with a deterministic final tie-break. Store the chosen inverse map and apply it to every combo-indexed output. Test paired/monotone board stabilizers and distinct turn/river ordering. Never replace a missing canonical board with any other board.

## C1 — Concrete replacement for §10.4: pot-normalized cache contract

This is proposed policy, not a measured error guarantee. It resolves S05/S09 and supersedes `cache_key_exact_match_only`. No matching step changes actual `HandState` chips or the board used for a live solve.

1. **Reference state.** Let P>0 be the matched gross pot at the street root from S01. Store both seats' root remaining stacks, not just an ambiguously timed effective stack. Normalize stacks/contributions/wager quantum/rake cap by P. Express each bet/raise by its pot fraction using §2's after-call denominator at that node; include its ordered structural path, actor, action kind and normalized raise-to amount. Include future menus, all-in/merge/cap rules, full ordered observed prefix and requested node. Excess that cannot be contested is tracked separately and excluded from the strategic pot/payoff model.

2. **Candidate-key identity.** Key schema version; canonical `(unordered flop, ordered turn, ordered river)`; canonical OOP/IP range fingerprints and support masks; solver commit + adapter/EV/rules versions; root street/actor and relative player roles; complete effective-tree structural signature and requested path; rake collection/rounding rule; bunching/model kind and folded-range/lock fingerprints when applicable; quantized numbers below. Absolute seat IDs, hand ID, bb_chips and raw chip pot/stack do not affect a mathematically equivalent actor-owned solution. All action amounts in the hash must be normalized too; a raw-chip `tree_id` would undo the change.

3. **Grid.** Use nearest grid point, ties upward, with versioned deterministic serialization. Preserve exact unquantized source inputs in each payload for verification.

   | Quantity | Candidate grid step |
   |---|---:|
   | Each root stack/P (SPR) | 0.01 |
   | Each node's bet/raise pot fraction, including observed sizes | 0.01 |
   | Observed contribution/P or raise-to/P | 0.001 |
   | Rake cap/P | 0.001 |
   | Rake rate | 0.0001 |
   | Wager quantum/P | 0.001 |
   | Canonical range weights, divided by that range's maximum positive weight | 0.0001 |

   Keep exact support masks so tiny positive weights do not become zero silently. For comparing ranges, convert the saved and query vectors to unit-sum distributions and compute total variation, `TV=0.5*sum(abs(p-q))`. Independent positive rescaling of a range is irrelevant to strategy; retain original/log reach mass only as provenance. No normalization can repair an empty or incompatible range.

4. **Matching and labels.** Search only the same canonical board and structural identity. Grid equality yields a candidate, not an Exact result. A candidate is **Exact within tolerance X=10^-6** only when every normalized monetary quantity/pot fraction differs by at most 10^-6, rake rate by at most 10^-8, and each range TV by at most 10^-6. Support, actor/path, discrete legal topology and rake rules must agree exactly, and rescaled outgoing actions must survive chip rounding with the same normalized-size tolerance. Solve accuracy must satisfy the requested target and all inherited approximation reasons must be empty. UI wording: “Exact for stated model; normalized-input tolerance 10^-6.” This is an input-matching tolerance, not a per-hand EV-error certificate.

5. **Approximate reuse.** Otherwise reuse only within the same quantization cell, with each saved/query numeric difference ≤its full grid step, each range TV≤0.001, and identical support/legal topology. Emit `Approximate{CacheQuantized{field_deltas,source_inputs,used_inputs}}`, plus `RangeQuantized{tv}` when applicable and every inherited reason. This includes differences caused by chip rounding. Reject candidates that cross min-raise, jam/force-all-in, action-merging, rake-cap activation or pot-eligibility boundaries; verify the actual materialized tree, not only the template name. No validated approximation outside these bounds: live solve the **actual** board/state or return Unsupported. Tighten grids/disable approximate reuse if holdout action-EV loss is unacceptable; these starting grids are engineering defaults.

6. **Payload.** Store actor-owned probabilities, action identities, `ev_over_P`, exploitability/P with its measurement status, complete available street-node strategies, root ranges, source inputs and semantic versions; never cache a final user `Recommendation` or request ID. On hit, select the requested node, inverse-map suits, convert actions to legal query chips, and compute `ev_bb=(ev_over_P*P_query)/bb_chips_query`. Validate the menu and finite values before display. For approximate hits, explicitly state that these are scaled cached-model EV estimates. Rebuild current coverage/assumptions from query provenance; source approximation/accuracy cannot be upgraded by a new request.

7. **Accuracy and replacement.** Requested `target_bp` is an acceptance filter, not an exact-key field: a better solution may serve a looser target. A worse validated solution may be shown immediately as Approximate while refinement is attempted within the original deadline. Do not store `DeadlineBestSoFar` as if it certifies current-request timing. Keep bounded candidates per cell (e.g. two, closest-input and best-accuracy), replacing dominated entries deterministically. Schema/solver/EV-version mismatch is a miss, not approximate compatibility.

8. **Scale check.** Pot/stack/cap 100/500/5 and 200/1000/10 with identical ranges, proportional wager quantum and identical fractional tree have the same strategic solution: frequencies unchanged, chip EV doubled. BB display may differ and is computed for the request. Separately, for an unraked root with no observed prefix, pot/stack 100/500 versus 103/515 and a one-chip wager quantum, a rounded future 50%-pot action is 50 versus 52: the realized fractions differ and can qualify only under the approximate rule, provided the rest of the candidate/tree checks pass. A different canonical board always misses, even if every numeric field is equal. Normalization cannot make different ranges, discrete all-in structures or rake models equivalent.

## C2 — Precise worker-wire additions for §4.5

Choose UTF-8 JSON Lines with a named `type` discriminator and fixed lowercase tags, newline-terminated/flush per message. IDs are decimal strings on the wire (avoid JS u64 precision loss). One proposed cancel exchange, as literal JSON data:

```json
{"type":"cancel","id":"42","target":"41"}
{"type":"ack","id":"42","status":"accepted"}
{"type":"result","id":"41","status":"cancelled","elapsed_ms":183}
```

An unknown completed target acknowledges `status:"already_finished"`; an allocation failure is representable without dummy strategy arrays:

```json
{"type":"result","id":"41","status":"error","error":{"code":"out_of_memory","message":"allocation failed"},"elapsed_ms":12}
```

Specify that racing completion yields either the original valid Result or Cancelled, never two terminals for the same accepted Solve. An Ack alone never means the solve stopped. `Ready` includes protocol version, backend commit, adapter version, capabilities and applied thread count; reject a mismatch against the engine's expected versions. Unknown message tags, invalid numbers/cards/range lengths and structurally invalid histories produce typed rejection; duplicate IDs do not start more work. Select bounded limits (initial proposal: 1 MiB request line, 16 MiB result line, 100,000 exported street nodes subject to the byte cap); reject over-limit trees/results early enough to return the current node or a typed error. Keep reader/writer queues bounded and coalesce progress; never interleave JSON with solver progress logs.

Before implementation, add complete literal JSON examples to the spec for Ready, Solve, Progress during each lifecycle stage, Ok, BestSoFar, Cancel/Cancelled, typed Error, Lock/Ack and Shutdown/Ack/EOF. The Solve fixture must have full valid 1326-element vectors, materialized EffectiveTree, a nonempty history and expected node/actor; no ellipses masquerading as executable fixtures. A separate compact two-combo explanation may accompany it. Define matrix dimensions, unavailable masks, atomic lock consumption and whether a failure is retryable. Protocol conformance fixtures are specification data, not a request to implement a worker during this review.

## Five highest-value additional tests for §13

These are independent-oracle or metamorphic tests, not assertions copied from the proposed implementation. Fix the existing S17/S26 oracles as well.

### T1 — Signed incremental EV across source units and an actual-combo call/fold decision

**Spec:** §§2, 5 step 6, 8.2, 10.3, 13.2. **Findings caught:** S01/S03/S04/S16/S18/S21.

Use river Qs Jd 7h 3c 2d, actual hero AhAd. Villain QQ weight 1 gives three value combos; 54o weight 1/12 gives one bluff combo, hence hero equity 0.25. Declare this as the already-conditioned jam range. Pot before jam=100, jam/call=73: matched final pot=246, fold EV=0, call EV=−11.5 chips; with fixed cap-rake 5, call EV=−12.75. At 54o weight 0.25, equity=0.5, call EV=+50 unraked or +47.5 raked. Include other hands in hero's strategic range and assert the headline still uses AhAd's conditional equity. A 5-chip BB gives −2.30 bb for the first unraked case. Separately feed a synthetic provider fixture with EV=1.84 source SB and verified zero reference: it is 0.92 source bb for both 1/2 and 2/5 live blinds. Add nonzero source fold EV, a re-raise node and a mapped straddle to verify reference subtraction/source scaling. Wrong signs, factor-of-two/live-SB bugs and sunk-chip double subtraction must fail.

### T2 — Street-root reconstruction, actor transitions, uncalled money and cumulative reopening

**Spec:** §§4.3/4.5, 8.3, 10.2, 13.1/13.3. **Findings caught:** S01/S05/S14/S17/S18/S25.

From a HU street root pot100/stacks500 each: OOP bet-to50 → pot150, stacks450/500; IP raise-to150 → pot300, stacks450/350, OOP call cost100; OOP calls → pot400, stacks350/350; next street starts at400. At every prefix assert actor, call amount, legal min/max, contributions and chip conservation against an independently transcribed/PokerKit fixture. Assert worker root pot100/stack500 at the intermediate raise decision, never current pot300. Add check-prefix actor changes at unchanged pot/stack; 2/5/10 physical action order/post fractions; 50/100/200 all-in contributions yielding main150/side100/refund100; and a full raise followed by cumulative short all-ins sufficient to reopen to a previous actor. Third all-in remains Unsupported EV even with a HU side pot; hero's own all-in produces no request.

### T3 — Whole-history replay with informative actions, missing nodes and stale street solves

**Spec:** §§8.3, 9, 10.2–10.4, 13.3. **Findings caught:** S02/S03/S08/S12/S13/S15.

Give a seat two compatible named combos with initial weights (1,1). Preflop raise likelihoods (.8,.2), later call likelihoods (.25,1), and flop bet likelihoods (.9,.1) must yield (.18,.02), posterior (0.9,0.1). Test every seat, including hero, with hand-authored class/node tables. An observed inserted bet must change the posterior; forcing its probability to one fails. Add a 73%-pot translation across 50/100 using the exact pseudo-harmonic formula and later branch-dependent actions; independently compute mixture likelihoods/path weights. Delete a needed continuation and assert the declared unconditioned fallback and persistent reason, without an invented branch or a second application of reach. Two solves on one street plus undo must select compatible revisions only. An opponent folding later must not change earlier depth mapping. Include positive total range but zero hero-combo support.

### T4 — Cache equivalence, same-cell approximation and semantic non-equivalence

**Spec:** §§10.2/10.4, 13.1/13.2; replacement C1. **Findings caught:** S05/S09/S20/S21/S25/S32.

Compare cold solves and hits for a game and a 2× chip/rake-cap/wager-quantum scale: probabilities equal within solver tolerance, chip EV doubles, actual bb conversion correct. Permute suits on monotone and paired boards and both ranges, then inverse-map every result. Add same-cell SPR/bet/range perturbations: Exact only within C1's numerical tolerance, otherwise Approximate with measured deltas. Check-prefix, swapped hero role, changed decision path, opposite actor, changed source EV version, lock/model identity, min-raise/all-in boundary, rake rule and incompatible support must select the correct node or miss. A changed canonical flop must always miss. A requested stricter target must not receive a stale Exact label; a cached chart/deadline/unconditioned reason must survive.

### T5 — Deterministic request-race and deadline/failure matrix

**Spec:** §§3.4, 4.5, 5, 7, 9.1, 12, 13.4/13.5. **Findings caught:** S06/S07/S08/S19/S22/S23/S24/S27/S30.

Use a controllable fake worker/clock at test implementation time. Start hand A/revision7, undo to a nondecision, start hand B with the same displayed revision, re-request B, and deliver A's Fast/Progress/Final and both B replies out of order. No stale event, replay snapshot or active recommendation may be accepted. A Cancel Ack while the job continues must not free admission. Repeat with mid-hand config edits, presolver preemption, worker OOM/EOF, malformed/oversized output, invalid-hash and valid-hash-invalid-schema bundle, blocked cache I/O, slow allocation/finalize, no completed iteration, wall-clock jumps and suspend/resume. Assert bounded queues/retries, legal fast output by its target where feasible, one terminal outcome per active request by the engine hard deadline, and no dependency on kill/restart completing before that fallback. Measure max completion time and quality/coverage, not merely p95 or presence of a Final event.
