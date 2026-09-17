---
type: plan
status: current
date: 2026-09-17
supersedes: none
related:
  - 2026-09-10-plan-1-foundation.md
  - 2026-09-10-plan-2-worker-engine.md
  - 2026-09-10-plan-3-preflop-replay.md
  - 2026-09-10-plan-4-flop-cache-presolver.md
  - 2026-09-10-plan-5-ui.md
  - ../research/REVIEW-cross-plan-2.md
---

# Execution Order: All 114 Tasks

## Source and Notation

Extracted from `docs/research/REVIEW-cross-plan-2.md`, section 7 ("Revised execution graph: all 114 tasks"). The execution order is proposed after findings E01–E06 and decision F20. This document replaces unstructured plan-by-plan reading with a unified dependency graph suitable for coordinating parallel work.

**Task counts by plan:**
- Plan 1 (Foundation): 25 tasks
- Plan 2 (Worker + Engine): 30 tasks
- Plan 3 (Preflop Replay): 19 tasks
- Plan 4 (Flop Cache + Presolver): 26 tasks
- Plan 5 (UI): 14 tasks
- **Total: 114 tasks**

**Notation:** `n.t` or `P<n>.T<m>` = Plan `n`, Task `t` or Task `m`. The table below lists immediate hard prerequisites; transitive prerequisites also apply. A prerequisite includes the task's required green review, not merely a file existing. `★` marks membership in at least one structural critical path.

## Every Task and Its Prerequisites

| Plan 1 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| ★ 1.1 — Workspace skeleton | — | 1 | P1.T1 Files/Interfaces, L88 |
| ★ 1.2 — `proto` cards and combos | 1.1 | 2 | P1.T2 Files/Interfaces, L230 |
| ★ 1.3 — `proto` game config and hand-state types | 1.2 | 3 | P1.T3 Files/Interfaces, L429 |
| 1.4 — `proto::Range1326` | 1.2 | 3 | P1.T4 Files/Interfaces, L684 |
| 1.5 — `proto` recommendation, coverage and event types | 1.3 | 4 | P1.T5 Files/Interfaces, L808 |
| 1.6 — `proto` effective tree, materialized nodes and chip-path resolution | 1.3, 1.4 | 4 | P1.T6 Files/Interfaces, L989 |
| 1.7 — `proto::worker` wire messages | 1.6 | 5 | P1.T7 Files/Interfaces, L1177 |
| 1.8 — `proto::worker::validate_solution` and `validate_locks` | 1.7 | 6 | P1.T8 Files/Interfaces, L1404 |
| ★ 1.9 — `core-model` skeleton: errors, card parsing, positions, config helpers | 1.3 | 4 | P1.T9 Files/Interfaces, L1607 |
| ★ 1.10 — `core-model::betting::Round` (one betting street) | 1.9 | 5 | P1.T10 Files/Interfaces, L1881 |
| ★ 1.11 — `core-model` settlement: refunds, side pots and the conservation invariant | 1.9 | 5 | P1.T11 Files/Interfaces, L2128 |
| ★ 1.12 — `core-model::lifecycle::simulate` (hand replay through one betting round) | 1.10, 1.11 | 6 | P1.T12 Files/Interfaces, L2276 |
| ★ 1.13 — `core-model` public state API | 1.12 | 7 | P1.T13 Files/Interfaces, L2509 |
| ★ 1.14 — `core-model` street root, `replay_root` and the §10.2 projection rule | 1.13 | 8 | P1.T14 Files/Interfaces, L2804 |
| 1.15 — `tools/` Python project with pinned oracle versions | 1.1 | 2 | P1.T15 Files/Interfaces, L3096 |
| 1.16 — `tools/gen_fixtures.py`, the PokerKit hand generator | 1.15 | 3 | P1.T16 Files/Interfaces, L3187 |
| 1.17 — generate and commit the 200 hand fixtures | 1.16 | 4 | P1.T17 Files/Interfaces, L3616 |
| ★ 1.18 — `core-model` replays the PokerKit fixtures | 1.14, 1.17 | 9 | P1.T18 Files/Interfaces, L3648 |
| 1.19 — `core-ranges` Pio range strings and 169-class expansion | 1.4 | 4 | P1.T19 Files/Interfaces, L3783 |
| 1.20 — `core-ranges` blocking, hero conditioning and `hash_scaled` | 1.19 | 5 | P1.T20 Files/Interfaces, L4060 |
| 1.21 — `core-iso` suit permutations and canonical boards | 1.19 | 5 | P1.T21 Files/Interfaces, L4163 |
| 1.22 — `tools/gen_eval_oracle.py` and the phevaluator fixtures | 1.15 | 3 | P1.T22 Files/Interfaces, L4400 |
| 1.23 — `core-eval` evaluator trait, b-inary backend and oracle tests | 1.22, 1.19 | 5 | P1.T23 Files/Interfaces, L4544 |
| 1.24 — `core-eval` exact equity, per-combo equity and terminal payoffs | 1.23, 1.20, 1.5 | 6 | P1.T24 Files/Interfaces, L4709 |
| 1.25 — `core-eval` Monte Carlo with joint disjoint sampling | 1.24 | 7 | P1.T25 Files/Interfaces, L5048 |

| Plan 2 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 2.1 — Vendor postflop-solver at the pinned commit with the two patches | 1.1 | 2 | P2.T1 Files/Interfaces, L111; §3 ledger |
| ★ 2.2 — Engine crate skeleton: clock, identity, tree templates | 1.8, 1.18, 1.21, 1.25 | 10 | P2.T2 Files/Interfaces, L207; §3 ledger |
| ★ 2.3 — Engine materializer with the pinned §4.6 rules | 2.2 | 11 | P2.T3 Files/Interfaces, L535; §3 ledger |
| ★ 2.4 — Effective tree with exact insertion, `tree_signature`, chip-path resolution, `tree_builder_golden` | 2.3 | 12 | P2.T4 Files/Interfaces, L861; §3 ledger |
| ★ 2.5 — `bench` crate skeleton: spot format, `gen-spots`, `materialize` | 2.4 | 13 | P2.T5 Files/Interfaces, L1094; §3 ledger |
| ★ 2.6 — `tools/gen_worker_fixtures.py` and the `fixtures/worker/*.jsonl` files | 2.5, 1.15 | 14 | P2.T6 Files/Interfaces, L1385; §3 ledger |
| 2.7 — `solver-worker` skeleton: AVX2 build check, card mapping, `ready`, EOF exit | 2.1, 1.8 | 7 | P2.T7 Files/Interfaces, L1692; §3 ledger |
| ★ 2.8 — Worker tree build, `add_line` insertion, `remove_line` wager cap, cross-check | 2.7, 2.6 | 15 | P2.T8 Files/Interfaces, L2015; §3 ledger |
| 2.9 — Worker memory admission (§10.3) and the §7 stop rule | 2.7 | 8 | P2.T9 Files/Interfaces, L2299; §3 ledger |
| ★ 2.10 — Worker extraction and lock application | 2.8 | 16 | P2.T10 Files/Interfaces, L2452; §3 ledger |
| ★ 2.11 — Worker job runner: Building -> Solving -> Extracting with cancel checkpoints | 2.9, 2.10 | 17 | P2.T11 Files/Interfaces, L2683; §3 ledger |
| ★ 2.12 — Worker stdout writer, bounded line reading and the three-thread wiring | 2.11 | 18 | P2.T12 Files/Interfaces, L2841; §3 ledger |
| ★ 2.13 — Worker state machine: admission, `ack` rules, cancel, shutdown | 2.12 | 19 | P2.T13 Files/Interfaces, L2984; §3 ledger |
| ★ 2.14 — Worker lock staging and the cancel lifecycle | 2.13 | 20 | P2.T14 Files/Interfaces, L3300; §3 ledger |
| ★ 2.15 — The pinned V1 fixture and the worker's river contract tests | 2.14 | 21 | P2.T15 Files/Interfaces, L3494; §3 ledger |
| 2.16 — Worker contract tests: materialization mismatch, wager cap, exact insertion, suit permutation, pinned example | 2.15 | 22 | P2.T16 Files/Interfaces, L3718; §3 ledger |
| 2.17 — Worker deadline and memory contracts | 2.15 | 22 | P2.T17 Files/Interfaces, L3882; §3 ledger |
| ★ 2.18 — Engine worker link: `WorkerLink`, `ProcessWorker`, `ready` validation, job object | 2.2, 2.15 | 22 | P2.T18 Files/Interfaces, L3973; §3 ledger |
| ★ 2.19 — Engine test doubles: `FakeClock`, `FakeWorker`, `RecordingSink`, solution builder | 2.18 | 23 | P2.T19 Files/Interfaces, L4354; §3 ledger |
| ★ 2.20 — Absolute deadlines and the independent watchdog | 2.19 | 24 | P2.T20 Files/Interfaces, L4525; §3 ledger |
| 2.21 — Decision log (§5 step 10) | 2.2 | 11 | P2.T21 Files/Interfaces, L4753; §3 ledger |
| ★ 2.22 — `EngineCore` and the `run_solve` happy path | 2.20, 2.21, 2.4 | 25 | P2.T22 Files/Interfaces, L4855; §3 ledger |
| ★ 2.23 — `run_solve` resilience: heartbeat, cancel-then-kill, `_min` retry, error-code policy | 2.22 | 26 | P2.T23 Files/Interfaces, L5153; §3 ledger |
| 2.24 — Coverage classifier (§6) and `coverage_classification_golden` | 2.19 | 24 | P2.T24 Files/Interfaces, L5348; §3 ledger |
| 2.25 — Equity adapter and the facing-all-in analytic fallback (`facing_allin_golden`) | 2.2 | 11 | P2.T25 Files/Interfaces, L5527; §3 ledger |
| 2.26 — Result assembly, headline rules, reason accumulation, equity merge (`recommendation_assembly_golden`) | 2.25, 2.20 | 25 | P2.T26 Files/Interfaces, L5697; §3 ledger |
| ★ 2.27 — Snapshot store and the root-range source | 2.22 | 26 | P2.T27 Files/Interfaces, L5943; §3 ledger |
| ★ 2.28 — `serve_request`: the river/turn decision path and `identity_race_golden` | 2.23, 2.24, 2.25, 2.26, 2.27 | 27 | P2.T28 Files/Interfaces, L6137; §3 ledger |
| ★ 2.29 — Public `Engine` API, startup report and `final_delivery_independent_of_worker` | 2.28 | 28 | P2.T29 Files/Interfaces, L6406; §3 ledger |
| 2.30 — `bench run` for the river and turn suites with the §13.5 report | 2.5, 2.20, 2.16, 2.17 | 25 | P2.T30 Files/Interfaces, L6806; §3 ledger |

| Plan 3 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 3.1 — Define and validate the normalized preflop envelope | 1.18, 1.20, 1.5 | 10 | P3.T1 Files/Interfaces, L121; §3 ledger |
| 3.2 — Load independent sources and quarantine invalid bundles | 3.1, 1.15 | 11 | P3.T2 Files/Interfaces, L328; §3 ledger |
| 3.3 — Build the deterministic chart ingestion and validation tool | 3.2 | 12 | P3.T3 Files/Interfaces, L557; §3 ledger |
| 3.4 — Acquire the public chart sources and record which depths shipped | 3.3 | 13 | P3.T4 Files/Interfaces, L706; §3 ledger |
| 3.5 — Inventory and transcribe the PokerCoaching 100bb grids | 3.4 | 14 | P3.T5 Files/Interfaces, L797; §3 ledger |
| 3.6 — Inventory and transcribe the RangeConverter 200bb grids | 3.4 | 14 | P3.T6 Files/Interfaces, L869; §3 ledger |
| 3.7 — Freeze chart coverage and load both layers through the Rust boundary | 3.5, 3.6 | 15 | P3.T7 Files/Interfaces, L938; §3 ledger |
| 3.8 — Reconstruct each prefix and select depth, rake and virtual roles | 3.2 | 12 | P3.T8 Files/Interfaces, L989; §3 ledger |
| 3.9 — Normalize all EV reference variants and expand classes | 3.8 | 13 | P3.T9 Files/Interfaces, L1341; §3 ledger |
| 3.10 — Implement wager interpolation and legality after mapping | 3.9 | 14 | P3.T10 Files/Interfaces, L1468; §3 ledger |
| 3.11 — Implement shared history-branch Bayesian updates | 3.10 | 15 | P3.T11 Files/Interfaces, L1625; §3 ledger |
| 3.12 — Cap live branches with a persistent frozen residual | 3.11 | 16 | P3.T12 Files/Interfaces, L1808; §3 ledger |
| 3.13 — Replay preflop prefixes with frozen missing-node stops | 3.12 | 17 | P3.T13 Files/Interfaces, L1920; §3 ledger |
| ★ 3.14 — Select compatible snapshots and preserve prefix-valid provenance | 3.13, 2.29 | 29 | P3.T14 Files/Interfaces, L2173; §3 ledger |
| ★ 3.15 — Walk completed streets through partial snapshot exports | 3.14 | 30 | P3.T15 Files/Interfaces, L2386; §3 ledger |
| 3.16 — Assemble branch-supported strategy, EV and unresolved mass | 3.12 | 17 | P3.T16 Files/Interfaces, L2584; §3 ledger |
| ★ 3.17 — Integrate the engine's chart and EV-bearing preflop decision path | 3.7, 3.15, 3.16, 2.29 | 31 | P3.T17 Files/Interfaces, L2697; §3 ledger |
| ★ 3.18 — Feed replayed root ranges into turn and river solves and register snapshots | 3.17 | 32 | P3.T18 Files/Interfaces, L2869; §3 ledger |
| 3.19 — Freeze replay and bet-translation goldens and audit every scoped test | 3.18 | 33 | P3.T19 Files/Interfaces, L2975; §3 ledger |

| Plan 4 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 4.1 — Introduce canonical cache keys and exact rational identity | 1.21, 1.20, 1.8 | 7 | P4.T1 Files/Interfaces, L143; §3 ledger |
| 4.2 — Normalize actor-owned payloads and resolve ordinal paths | 4.1 | 8 | P4.T2 Files/Interfaces, L248; §3 ledger |
| 4.3 — Compare complete trees, SPR and terminal rake-cap activation | 4.2 | 9 | P4.T3 Files/Interfaces, L492; §3 ledger |
| 4.4 — Preserve inherited reasons and filter raw accuracy | 4.3 | 10 | P4.T4 Files/Interfaces, L573; §3 ledger |
| 4.5 — Add bounded binary storage and delete corrupt entries | 4.2 | 9 | P4.T5 Files/Interfaces, L646; §3 ledger |
| 4.6 — Write atomically, retain two representatives and enforce quota | 4.5 | 10 | P4.T6 Files/Interfaces, L792; §3 ledger |
| ★ 4.7 — Serve validated ordinal nodes through bounded cache lookup | 4.6, 4.4, 3.18 | 33 | P4.T7 Files/Interfaces, L987; §3 ledger |
| ★ 4.8 — Freeze the complete T4 structural-identity scale contract | 4.7 | 34 | P4.T8 Files/Interfaces, L1297; §3 ledger |
| ★ 4.9 — Implement flop budget and evidence-based template selection | 2.29, 4.8 | 35 | P4.T9 Files/Interfaces, L1457; §3 ledger |
| ★ 4.10 — Route flop and turn through cache, provisional and live solve | 4.8, 4.9, 3.19 | 36 | P4.T10 Files/Interfaces, L1605; §3 ledger |
| ★ 4.11 — Create the section 6 experimental synthetic-root surrogate | 4.10, 2.25 | 37 | P4.T11 Files/Interfaces, L1982; §3 ledger |
| ★ 4.12 — Register cache snapshots and translate prior-street bets | 4.10 | 37 | P4.T12 Files/Interfaces, L2180; §3 ledger |
| 4.13 — Enumerate presolver tiers and canonical-flop order | 4.1 | 8 | P4.T13 Files/Interfaces, L2266; §3 ledger |
| 4.14 — Persist queue identities, cursor and verified completion | 4.13, 4.6 | 11 | P4.T14 Files/Interfaces, L2376; §3 ledger |
| 4.15 — Schedule idle jobs and cancel them for live admission | 4.14 | 12 | P4.T15 Files/Interfaces, L2630; §3 ledger |
| ★ 4.16 — Prepare presolves by chart replay, own the Presolver and report coverage | 4.15, 4.10, 3.18 | 37 | P4.T16 Files/Interfaces, L2860; §3 ledger |
| 4.17 — Freeze chart provenance, source hashes and the node inventory | 3.7, 2.5, 1.16 | 16 | P4.T17 Files/Interfaces, L2991; §3 ledger |
| 4.18 — Define the fifty recorded inputs as code | 4.17 | 17 | P4.T18 Files/Interfaces, L3121; §3 ledger |
| 4.19 — Generate, legality-check and freeze the fifty recorded hands | 4.18 | 18 | P4.T19 Files/Interfaces, L3330; §3 ledger |
| 4.20 — Regenerate all six chart-replay suites and measure V3 modes | 4.16, 4.19, 2.30 | 38 | P4.T20 Files/Interfaces, L3476; §3 ledger |
| ★ 4.21 — Replay recorded hands through the real engine | 4.19, 4.11, 4.12, 4.16 | 38 | P4.T21 Files/Interfaces, L3576; §3 ledger |
| ★ 4.22 — Inject worker, disk and clock faults behind WorkerLink | 4.21 | 39 | P4.T22 Files/Interfaces, L3645; §3 ledger |
| ★ 4.23 — Report measured results, run the oracle suites and enforce the baseline gate | 4.20, 4.22 | 40 | P4.T23 Files/Interfaces, L3729; §3 ledger |
| ★ 4.24 — Run the deterministic regression and the flop measurement matrix | 4.23 | 41 | P4.T24 Files/Interfaces, L3886; §3 ledger |
| ★ 4.25 — Measure the baseline suites, e2e, faults, pre-solved hits and the oracles | 4.24 | 42 | P4.T25 Files/Interfaces, L3952; §3 ledger |
| ★ 4.26 — Evaluate the release gate and freeze the reviewed baseline | 4.25, 5.14 | 44 | P4.T26 Files/Interfaces, L3997; §3 ledger; S§14 V22 / F21 |

| Plan 5 task | Hard prerequisites (plan.task) | Wave | Contract / file evidence |
|---|---|---|---|
| 5.1 — Scaffold the strict React/Tauri workspace and build-time types | 2.2 | 11 | P5.T1 Files/Interfaces, L155; §3 ledger |
| 5.2 — Implement the bounded command dispatcher and typed errors | 5.1 | 12 | P5.T2 Files/Interfaces, L463; §3 ledger |
| 5.3 — Register the fourteen Tauri commands and prove IPC argument contracts | 5.2 | 13 | P5.T3 Files/Interfaces, L645; §3 ledger |
| 5.4 — Compile the real engine adapter and forward the recommendation channel | 5.3, 4.16 | 38 | P5.T4 Files/Interfaces, L840; §3 ledger |
| 5.5 — Persist configuration and own application startup and shutdown | 5.4, 3.7, 2.1 | 39 | P5.T5 Files/Interfaces, L1008; §3 ledger |
| 5.6 — Add the typed Backend, scripted fake, and mockIPC harness | 5.1 | 12 | P5.T6 Files/Interfaces, L1368; §3 ledger |
| 5.7 — Enforce active identity and merge progressive events | 5.6 | 13 | P5.T7 Files/Interfaces, L1570; §3 ledger |
| 5.8 — Build the session configuration screen | 5.6 | 13 | P5.T8 Files/Interfaces, L1798; §3 ledger |
| 5.9 — Implement confirmed hand setup and card entry | 5.8 | 14 | P5.T9 Files/Interfaces, L1972; §3 ledger |
| 5.10 — Wire the exact keyboard map to engine mutations | 5.7, 5.9 | 15 | P5.T10 Files/Interfaces, L2123; §3 ledger |
| 5.11 — Render recommendations, coverage, assumptions, equity, and experimental output | 5.7 | 14 | P5.T11 Files/Interfaces, L2423; §3 ledger |
| 5.12 — Compose the desktop session and complete the keystroke suite | 5.5, 5.8, 5.10, 5.11 | 40 | P5.T12 Files/Interfaces, L2649; §3 ledger |
| 5.13 — Suppress WebView accelerators and document the pinned Tauri 2 APIs | 5.12 | 41 | P5.T13 Files/Interfaces, L2876; §3 ledger |
| ★ 5.14 — Prove the chart-backed Windows SRP flop E2E contract | 5.13, 4.25 | 43 | P5.T14 Files/Interfaces, L2983; §3 ledger; measurement-isolation edge above |

## Sequential Topological Order

Read left to right, then continue on the next line. Every one of the 114 tasks occurs exactly once. This order preserves all prerequisites and does not require numeric task order.

```text
1.1 → 1.2 → 1.15 → 2.1 → 1.3 → 1.4 → 1.16 → 1.22 → 1.5 → 1.6 → 1.9 → 1.17
1.19 → 1.7 → 1.10 → 1.11 → 1.20 → 1.21 → 1.23 → 1.8 → 1.12 → 1.24 → 1.13 → 1.25
2.7 → 4.1 → 1.14 → 2.9 → 4.2 → 4.13 → 1.18 → 4.3 → 4.5 → 2.2 → 3.1 → 4.4
4.6 → 2.3 → 2.21 → 2.25 → 3.2 → 4.14 → 5.1 → 2.4 → 3.3 → 3.8 → 4.15 → 5.2
5.6 → 2.5 → 3.4 → 3.9 → 5.3 → 5.7 → 5.8 → 2.6 → 3.5 → 3.6 → 3.10 → 5.9
5.11 → 2.8 → 3.7 → 3.11 → 5.10 → 2.10 → 3.12 → 4.17 → 2.11 → 3.13 → 3.16 → 4.18
2.12 → 4.19 → 2.13 → 2.14 → 2.15 → 2.16 → 2.17 → 2.18 → 2.19 → 2.20 → 2.24 → 2.22
2.26 → 2.30 → 2.23 → 2.27 → 2.28 → 2.29 → 3.14 → 3.15 → 3.17 → 3.18 → 3.19 → 4.7
4.8 → 4.9 → 4.10 → 4.11 → 4.12 → 4.16 → 4.20 → 4.21 → 5.4 → 4.22 → 5.5 → 4.23
5.12 → 4.24 → 5.13 → 4.25 → 5.14 → 4.26
```

## Parallel Waves

All prerequisites of every task in a wave appear in earlier waves. "Parallel" describes dependency independence; shared-file integration and benchmark hardware still follow the isolation rules in the source review.

| Wave | Tasks |
|---|---|
| 1 | ★ 1.1 |
| 2 | ★ 1.2, 1.15, 2.1 |
| 3 | ★ 1.3, 1.4, 1.16, 1.22 |
| 4 | 1.5, 1.6, ★ 1.9, 1.17, 1.19 |
| 5 | 1.7, ★ 1.10, ★ 1.11, 1.20, 1.21, 1.23 |
| 6 | 1.8, ★ 1.12, 1.24 |
| 7 | ★ 1.13, 1.25, 2.7, 4.1 |
| 8 | ★ 1.14, 2.9, 4.2, 4.13 |
| 9 | ★ 1.18, 4.3, 4.5 |
| 10 | ★ 2.2, 3.1, 4.4, 4.6 |
| 11 | ★ 2.3, 2.21, 2.25, 3.2, 4.14, 5.1 |
| 12 | ★ 2.4, 3.3, 3.8, 4.15, 5.2, 5.6 |
| 13 | ★ 2.5, 3.4, 3.9, 5.3, 5.7, 5.8 |
| 14 | ★ 2.6, 3.5, 3.6, 3.10, 5.9, 5.11 |
| 15 | ★ 2.8, 3.7, 3.11, 5.10 |
| 16 | ★ 2.10, 3.12, 4.17 |
| 17 | ★ 2.11, 3.13, 3.16, 4.18 |
| 18 | ★ 2.12, 4.19 |
| 19 | ★ 2.13 |
| 20 | ★ 2.14 |
| 21 | ★ 2.15 |
| 22 | 2.16, 2.17, ★ 2.18 |
| 23 | ★ 2.19 |
| 24 | ★ 2.20, 2.24 |
| 25 | ★ 2.22, 2.26, 2.30 |
| 26 | ★ 2.23, ★ 2.27 |
| 27 | ★ 2.28 |
| 28 | ★ 2.29 |
| 29 | ★ 3.14 |
| 30 | ★ 3.15 |
| 31 | ★ 3.17 |
| 32 | ★ 3.18 |
| 33 | 3.19, ★ 4.7 |
| 34 | ★ 4.8 |
| 35 | ★ 4.9 |
| 36 | ★ 4.10 |
| 37 | ★ 4.11, ★ 4.12, ★ 4.16 |
| 38 | 4.20, ★ 4.21, 5.4 |
| 39 | ★ 4.22, 5.5 |
| 40 | ★ 4.23, 5.12 |
| 41 | ★ 4.24, 5.13 |
| 42 | ★ 4.25 |
| 43 | ★ 5.14 |
| 44 | ★ 4.26 |

## How to Use

**For executors:** Take the next wave whose prerequisites are all committed. Use the reference `git log --format=%B | grep "^Task: "` to list committed tasks. Never start a task whose prerequisite task is uncommitted. Shared file edits in a wave must be merged and serialized; dependency independence is not permission to overwrite another task's work or run benchmarks concurrently under CPU contention.

**For the orchestrator:** Mark waves as done in the checklist below once all their tasks are reviewed, approved, and committed. The measurement-isolation edge **P4.T25 → P5.T14** (noted in the prerequisites table) avoids running the UI timing acceptance concurrently with the benchmark machine's final measurement batch.

**Critical path notes:** 48 zero-slack task nodes are starred (★). One of the longest chains is 44 tasks (1.1 → 1.2 → 1.3 → 1.9 → 1.10 → 1.12 → 1.13 → 1.14 → 1.18 → 2.2 → 2.3 → 2.4 → 2.5 → 2.6 → 2.8 → 2.10 → 2.11 → 2.12 → 2.13 → 2.14 → 2.15 → 2.18 → 2.19 → 2.20 → 2.22 → 2.23 → 2.28 → 2.29 → 3.14 → 3.15 → 3.17 → 3.18 → 4.7 → 4.8 → 4.9 → 4.10 → 4.11 → 4.21 → 4.22 → 4.23 → 4.24 → 4.25 → 5.14 → 4.26). The measured tail is **P4.T24 (V3 matrix) → P4.T25 (baseline suites, e2e, faults) → P5.T14 (chart UI E2E) → P4.T26 (final release decision)**.

## Wave Completion Checklist

- [ ] Wave 1
- [ ] Wave 2
- [ ] Wave 3
- [ ] Wave 4
- [ ] Wave 5
- [ ] Wave 6
- [ ] Wave 7
- [ ] Wave 8
- [ ] Wave 9
- [ ] Wave 10
- [ ] Wave 11
- [ ] Wave 12
- [ ] Wave 13
- [ ] Wave 14
- [ ] Wave 15
- [ ] Wave 16
- [ ] Wave 17
- [ ] Wave 18
- [ ] Wave 19
- [ ] Wave 20
- [ ] Wave 21
- [ ] Wave 22
- [ ] Wave 23
- [ ] Wave 24
- [ ] Wave 25
- [ ] Wave 26
- [ ] Wave 27
- [ ] Wave 28
- [ ] Wave 29
- [ ] Wave 30
- [ ] Wave 31
- [ ] Wave 32
- [ ] Wave 33
- [ ] Wave 34
- [ ] Wave 35
- [ ] Wave 36
- [ ] Wave 37
- [ ] Wave 38
- [ ] Wave 39
- [ ] Wave 40
- [ ] Wave 41
- [ ] Wave 42
- [ ] Wave 43
- [ ] Wave 44
