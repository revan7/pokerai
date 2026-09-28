# Plan 2 changelog 4 — execution errata (revision 3 -> 4)

Date: 2026-09-28

## Preamble

Plan 2 (`docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md`, revision 3) was executed task-by-task on branch `phase-c` under `.superpowers/sdd/2026-09-10-plan-2-worker-engine/`: 30 tasks (`solver-worker`, `crates/engine`, `crates/bench`, the worker wire fixtures), three post-close follow-ups (P2.W1, P2.W2, P2.W3), and a final whole-branch review (`final-review.md`, 92 plan-2 task commits, footprint `crates/engine`, `solver-worker`, `crates/bench`, `docs/bench`, `fixtures/{worker,solver}`, `third_party/postflop-solver`). Each task was implemented, then independently reviewed by a different agent against the plan's own task text. As with plan 1 (`docs/research/PLAN-1-CHANGELOG-4.md`), a number of reviews found that the plan's prescribed code — sample implementations, sample tests and, in a few cases, prose describing an interface — carried a defect: it matched the brief exactly, but violated the spec, a global constraint, or a standing ruling made earlier in execution.

Per the orchestrator's ruling recorded during plan 1's execution (`.superpowers/sdd/2026-09-10-plan-1-foundation/progress.md`, "plan-text corrections found during execution are collected into ONE errata changelog per plan at the end of that plan's execution ... instead of a revision bump per finding") and reapplied here, this document collects every such plan-2 correction in one place. **The plan's task text is not rewritten.** A reader of the plan's Step blocks still sees the original, defective sample code or prose; the corrected behavior lives only in the committed source, in the per-task review reports, and in this changelog.

Findings that were about the implementer's own added code (not the plan's prescribed text), about report/journal evidence hygiene, or about orchestration process are excluded, following the same rule plan 1 used. One task, `Task 18 — Engine worker link`, is called out explicitly below as **not** an erratum, since its review findings could otherwise be misread as plan-text defects.

The ledger (`progress.md`) is the source of truth for this document; every ruling id cited below is quoted from it, and every citation was independently re-verified by grep before this document was written (see the verification section of the agent's report).

### Recurring patterns across the errata below

Several themes recur across unrelated tasks, all traceable to the engine's core delivery guarantee (spec 7: one `Final` per request, independent of the worker) and its identity guarantee (spec 12: a superseded or expired reply changes nothing):

- **Deadline and street-violation compliance is judged from the engine-clock arrival time of the first terminal, never from when a watchdog or client thread happens to resume.** The plan's own sample code used a boolean `terminal_seen` flag instead (Tasks 20, 22, 23, 28; carried into the final review's F-I1).
- **Identity and deadline expiry are rechecked on every received message — including EOF, spawn and protocol-error paths — before the message is classified or acted on**, not only on a successful reply (Tasks 22, 23; generalized in the final review's F-I1/F-I2).
- **A rejected or superseded request discards its staged/in-flight state instead of partially applying it** (Tasks 14, 29).
- **Deterministic in-process test seams replace real-time spawned-worker assertions for verifying status classification**, because a short wall-clock budget cannot guarantee solver progress under contention; the plan's own sample tests used fixed real-time deadlines and exact wall-clock windows instead (Tasks 14, 16, 17, 20).
- **Numeric wire domains match `proto::numeric`'s own inclusive/exclusive convention** rather than the plan's inherited prose (Task 13's rake domain).

---

## Errata by task

### Task 6 — `tools/gen_worker_fixtures.py` and the `fixtures/worker/*.jsonl` files (P2.T6)

- **Task/step:** `lock_river_lines`, plan lines 1688 and 1697.
- **Plan text (<=25 words):** `"""IP's river node locked to bet QQ 100% and 54o 20% (ev_convention_non_root_payoffs); OOP holds AA and 66."""` (line 1688); `solve_line("51", SPOT_LOCK, RIVER_BOARD, parse_range("AA,66"), ...)` (line 1697).
- **What the code does instead:** The generated `lock_river.jsonl` fixture gives OOP the range `QQ,66`, not `AA,66`; the docstring and generator both changed together, and Task 15's stale "spec names AA" comment was independently corrected to explain why (ruling 15-M2).
- **Why:** task-6-review R1, Important, plan-mandated — spec §13.2 requires each OOP `QQ` combo to block every IP `QQ` combo, which `AA,66` cannot exercise; the brief's `AA` was a copy error against the spec.
- **Commit(s):** `30e837b` + `c0cb56c` (fix round 1).

### Task 11 — Worker job runner: Building -> Solving -> Extracting with cancel checkpoints (P2.T11)

- **Task/step:** Step 1, the §7 stop rule and `report_exploitability`'s tolerance handling.
- **Plan text:** the plan's tolerance wording is a fixed `[-1e-6, 0)` floor around zero exploitability (ledger paraphrase; the ledger records this as "the `[-1e-6, 0)` floor wording is superseded", not as a literal quoted plan line — see `progress.md:76`).
- **What the code does instead:** `report_exploitability(raw, pot)` uses a pot-relative tolerance `max(1e-6, 8*eps*pot)` instead of the fixed floor, so the check scales with the pot sizes the §13.2 sweep actually exercises.
- **Why:** fix round 1 (`progress.md:76`), plan-mandated — a fixed near-zero floor is too tight at large pots and too loose at small ones; no formal `Ruling (11-...)` id was assigned in the ledger for this item (it is recorded directly in the Task 11 fix-round entry, not a separately numbered ruling).
- **Also recorded, not separately fixed in code:** `out_of_memory` is unproducible under the plan's admission design (memory admission is the defence) — tagged in the ledger as a **spec** erratum candidate, not a plan-2 changelog item; carried forward here for completeness but not claimed as a plan-2 defect.
- **Commit(s):** `1ab50bf` (fix round 1, merged into `8a1275f` + `1ab50bf`).

### Task 12 — Worker stdout writer, bounded line reading and the three-thread wiring (P2.T12)

- **Task/step:** Step 1, `writer::EXIT_WRITER_FAULT` and the request-line length check.
- **Plan text:** the plan assigns exit code 3 to a writer fault and counts the line-length limit including the trailing newline (ledger paraphrase, `progress.md:81-82`; no single plan line isolates this cleanly enough to quote verbatim without risking a false quote).
- **What the code does instead:** `writer::EXIT_WRITER_FAULT = 3` was kept as implemented, flagged as a **spec** erratum candidate (exit codes are a spec table, not a plan invention) rather than changed here; the line-limit-counts-the-trailing-newline behavior was likewise kept and flagged, not changed.
- **Why:** fix round 1 (`progress.md:81`, `progress.md:82`) — both items are recorded as candidates for a **spec** clarification (exit-code table, line-limit wording), not as plan-2 text defects that were corrected in code; included here per the brief's instruction to record every ledger-tagged candidate, with the caveat that these two target the spec, not the plan.
- **Commit(s):** `788f08e` + `9e134ca`.

### Task 13 — Worker state machine: admission, `ack` rules, cancel, shutdown (P2.T13)

**Erratum 1 — `precheck` accepts `rake_rate == 1.0`**

- **Task/step:** Step 3, `precheck`, plan line 3294.
- **Plan text (<=25 words):** `if !req.rake_rate.is_finite() || req.rake_rate < 0.0 || req.rake_rate > 1.0 { return Err("rake_rate outside [0, 1]".into()); }` (line 3294) — rejects only `> 1.0`, accepting exactly `1.0`.
- **What the code does instead:** `precheck` and `tree_build` both reject `rake_rate >= 1.0`, matching `proto::numeric`'s own domain `[0, 1)` for the rake rate.
- **Why:** task-13-review P2T13R I1, Important, plan-mandated (`progress.md:84`, "precheck accepts rake_rate == 1.0; the domain is [0, 1) per proto numeric.rs — tree_build aligned too") — a rake rate of exactly 1.0 (100%) is not a valid rake and is rejected everywhere else float domains are validated in this codebase.
- **Commit(s):** `1cefed8` + `4455d0e` (fix round 1, verified).

**Erratum 2 — a watchdog spawn failure silently removes the exit bound**

- **Task/step:** Step 3/4, the worker's watchdog-thread spawn path.
- **Plan text:** the plan's sample construction spawns the exit-confirmation watchdog without handling a spawn failure (ledger paraphrase, `progress.md:84`; no isolated plan line to quote without risking misattribution).
- **What the code does instead:** a watchdog spawn failure now publishes `Exit(0)` instead of leaving the process's exit bound unenforced.
- **Why:** task-13-review P2T13R I2, Important, plan-mandated (`progress.md:84`, "watchdog spawn failure silently removes the exit bound") — without this, a spawn failure (a rare OS condition) would leave the writer's exit-timer contract unbounded, contradicting the same task's own exit-code policy.
- **Commit(s):** `1cefed8` + `4455d0e` (fix round 1, verified — gate 800 passed).
- **Also recorded, not code-changed here:** the exit-timer-exit-without-a-result case (the engine synthesizes a result per review ruling) is carried as a wording note for the spec's exit-code table.

### Task 14 — Worker lock staging and the cancel lifecycle (P2.T14)

**Erratum 1 — `validate_locks` listed without a tree parameter**

- **Task/step:** plan line 55 (the `proto::worker` validation row in the cross-plan interfaces table): `validate_locks(&[NodeLock]) -> Result<(), String>`.
- **What the code does instead:** structural (non-tree) checks run at staging time as the plan's signature implies, but the tree-dependent checks run when the next `solve` consumes the staged lock set (a refusal ends that solve `lock_mismatch` immediately after `accepted`), which needs the tree and cannot be expressed by the plan's tree-free signature alone.
- **Why:** ruling (14-C2), Important, plan-mandated (`progress.md:88`) — "plan 2 line 55 lists validate_locks without a tree parameter: erratum candidate."
- **Commit(s):** `8b58396` (Task 14 implementation).

**Erratum 2 — the brief's `cancel_between_iterations` sample test pattern**

- **Task/step:** the brief's sample cancel test, filed by the plan under spec §13.2's "spawn the binary" heading.
- **What the code does instead:** `cancel_between_iterations` drives a deterministic checkpoint barrier in the job runner's test support (Building / Solving-iteration / Extracting-node boundaries) instead of a spawned-binary real-time race; the seam is inert (no-op hooks) in production. The spawned-worker fixture check is kept as a separate smoke test under its own name, making no claim about stage placement.
- **Why:** Ruling (14-I2), Important, plan-mandated (`progress.md:91`) — "the brief's sample pattern (plan-mandated) conflicts with the brief's deterministic-seam requirement and spec 13.2; plan-2 erratum candidate." Filing `cancel_between_iterations` under spec §13.2's "spawn the binary" text is itself flagged as a spec/plan wording erratum (`progress.md:92,95`).
- **Commit(s):** `8152bb7` (fix round 1) + `397d575` (round 2, orchestrator-verified).

### Task 15 — The pinned V1 fixture and the worker's river contract tests (P2.T15)

- **Task/step:** Step 1, the per-node EV bound assertion, plan line 3761.
- **Plan text (<=25 words):** `assert!(e.as_f64().unwrap().abs() <= 1100.0);` (line 3761) — a fixed, loose 1100-chip bound applied to every EV entry, alongside a plan-asserted `iterations: 0`.
- **What the code does instead:** task-15-review confirmed the per-node EV bound and the real iteration count under the spec (Compliant/Approved), so no code changed this bound directly in Task 15; it is recorded here as an erratum candidate because Task 16's tighter, pinned-fixture bound (see Ruling 16-I5 below) later replaces the same style of loose constant with the spec's absolute `1e-3` chip tolerance at 120 deterministic iterations, showing the 1100/`iterations: 0` pattern in the brief understates the achievable precision.
- **Why:** task-15-review (`progress.md:99`), "Erratum candidates: the brief's |EV| <= 1100 bound and iterations: 0; spec 13.2 'flop node' for a turn-rooted example" — no formal `Ruling (15-...)` id covers this specific pair; `Ruling (15-M1/M2)` (verified, `progress.md:99`) covers two unrelated Minor fixes (a missing-fixture hint, and correcting a stale "spec names AA" comment to reference Task 6's QQ,66 correction) in the same fix round.
- **Commit(s):** `4cbed6b` + `ca22837` (fix round 1, Haiku, orchestrator-verified).

### Task 16 — Worker contract tests: materialization mismatch, wager cap, exact insertion, suit permutation, pinned example (P2.T16)

**Erratum 1 — deterministic contract coverage instead of spawned real-time smoke checks**

- **Task/step:** the flop/turn deadline-status contract tests.
- **What the code does instead:** deterministic status coverage runs in process through the job runner's hook seam (the P2.T14 barrier pattern) with a real completed iteration and measurement; the spawned fixture runs stay as conditional smoke checks with a generous liveness bound; exact wall-clock assertions (e.g. "1000/2000 ms") become liveness allowances, never correctness conditions.
- **Why:** Ruling (16-I1), Important, plan-mandated (`progress.md:113`) — "a real-time budget cannot guarantee solver progress under contention."
- **Commit(s):** `a7511b7` + `2c2a9b9` (fix round 1, re-review Approved 5/5).

**Erratum 2 — `suit_permutation_metamorphic` under-specified board/weight shapes**

- **Task/step:** the suit-permutation metamorphic test.
- **What the code does instead:** the test now uses a valid flop-root request with three-card rainbow, paired and monotone boards (spec §13.2), a suit permutation that moves the monotone board's suit, and asymmetric suit weights so range permutation is actually exercised, asserting the transformation changes the inputs.
- **Why:** Ruling (16-I2), Important, plan-mandated (`progress.md:114`).
- **Commit(s):** `a7511b7` + `2c2a9b9`.

**Erratum 3 — comparisons run before dimension/identity assertions**

- **Task/step:** the materialization/wire contract tests' comparison order.
- **What the code does instead:** every numeric comparison is now preceded by asserting node counts, identities, actors, exact ordered action menus, requested/covered-path identity, 1326-row dimensions and each row's action count, with availability compared both directly and after combo permutation.
- **Why:** Ruling (16-I3), Important, plan-mandated (`progress.md:115`).
- **Commit(s):** `a7511b7` + `2c2a9b9`.

**Erratum 4 — the pinned example fixture's tolerance is pot-scaled instead of absolute, and the bound is loose**

- **Task/step:** `pinned_example_fixture` and its per-entry EV/probability bounds, plan lines around 3913 (`cap3_three_wagers`) and 4002 (`(n["ev_chips"][i][a] ... ).abs() <= 1e-3 * 200.0`).
- **Plan text (<=25 words):** `assert!((n["ev_chips"][i][a].as_f64().unwrap() - e["ev_chips"][i][a].as_f64().unwrap()).abs() <= 1e-3 * 200.0, ...)` (line 4002) — a tolerance scaled by the fixture's own pot (200 chips), rather than absolute.
- **What the code does instead:** `pinned_example_fixture` asserts against the oracle's frozen `0.5146258` chips of exploitability with the spec's absolute `1e-3`-chip per-entry EV bound, under a deterministic 120-iteration schedule (all 74,256 EV entries reproduced with zero difference), and the probability bound stays `1e-3`.
- **Why:** Ruling (16-I4) and Ruling (16-I5), Important, plan-mandated (`progress.md:116-117`) — "any floating-point allowance is derived from demonstrated arithmetic differences, never from pot scaling or iteration-count drift." Ruling (16-I5) explicitly lists "the brief's 1100 EV bound, river metamorphic spots, pot-scaled EV tolerance and timing constants" as plan-2 changelog candidates.
- **Commit(s):** `a7511b7` + `2c2a9b9`.

### Task 17 — Worker deadline and memory contracts (P2.T17)

- **Task/step:** the flop/turn deadline contract tests, plan lines 4047-4083 (fixed `deadline_ms: 1000`/`2000` with `elapsed_ms <= 1000`/`<= 2000` correctness assertions).
- **Plan text (<=25 words):** `assert!(r["elapsed_ms"].as_u64().unwrap() <= 1000, ...)` (representative of the plan's fixed-deadline correctness assertions at lines 4047-4083).
- **What the code does instead:** the forced `best_so_far`/`no_iteration` outcomes are established in process through the job runner's hook seam with at least one completed real iteration and measurement; the spawned fixture runs stay as smoke checks with a generous liveness bound whose accepted outcome is conditional on the measurement; the exact wall-clock assertions became liveness allowances, never correctness conditions, with corrected comments.
- **Why:** Ruling (17-I1) and Ruling (17-I2), Important, plan-mandated (`progress.md:104-105`) — "a short real-time budget cannot guarantee solver progress under contention"; spec §13.2's elapsed-time bounds are liveness allowances, not exact correctness bounds.
- **Commit(s):** `fe2d736` (fix round 1) + `bc3537f` (mechanical N1 round, orchestrator-verified).

### Not an erratum — Task 18: Engine worker link (`WorkerLink`, `ProcessWorker`, `ready` validation, job object) (P2.T18)

Task 18's review found two Important findings — Ruling (18-I1) (an unconfirmed EOF must never be cached as a confirmed exit; the exit code is confirmed only within the caller's remaining budget, never a new fixed wait) and Ruling (18-I2) (the seat-swap equivalence oracle only checked one of four node/role combinations) — plus Ruling (18-Q1) (the same monotonic-deadline rule extended to `send`) and Ruling (18-C1) (a `launches() <= 2` test relaxation, confirmed to be a race in the test's own observation, not the link). None of these were tagged `erratum candidate` in the ledger, unlike every other task in this document: `WorkerLink`/`ProcessWorker`'s exit-confirmation and restart-count behavior is the implementer's own construction of a component the plan describes only at the interface level (Interfaces table, not a literal Step-block sample implementation), so these are ordinary implementer-side defects caught by review, not plan-mandated errors in the plan's own prescribed text. `Ruling (18-N1)` (a documentation wording fix to `link.rs`'s `Exit` variant doc comment) is the same kind of implementer-side, documentation-only fix. They are recorded in `task-18-review.md` and the ledger (`progress.md:106-127`) but are excluded here for the same reason plan 1's changelog excluded its Task 10 folded-seat finding.

### Task 19 — Engine test doubles: `FakeClock`, `FakeWorker`, `RecordingSink`, solution builder (P2.T19)

- **Task/step:** `FakeReply` semantics, plan lines 4566-4574 (`InvalidateIdentity`/`Delay` doc comment) and the fake-worker timeline behavior generally.
- **Plan text (<=25 words):** the plan's sample `FakeWorker`/`FakeReply` listing times delays from `recv`, treats `Eof` as final, does not end a hung worker on `kill`, and its `InvalidateIdentity`/`Delay` interaction differs from what a real `WorkerLink` does on kill/restart (ledger paraphrase; the specific defects are enumerated in the "why" line below).
- **What the code does instead:** delays run on the worker timeline starting at the engine's first call after launch or restart; a `kill`/`restart` discards the whole EOF-to-Exit sequence of the killed process (including its due `StdinClosed` and any later events), keeping identity markers and later replies for the replacement; `Eof` is not final (`Exit` is); an `Ack{IdRef::Last}` means the last request of any kind; an undelayed `Exit` is due at the engine's first call, and `Delay{ms: 1}` places it after the first request; `StdinClosed` after a send models the lost-request `Ok`.
- **Why:** Ruling (19-D1), Ruling (19-I1), Ruling (19-I2), and the round-1/2 refinements Ruling (19-R1-1), Ruling (19-R1-2), Ruling (19-R1-3), Ruling (19-R2-1), all Important, plan-mandated (`progress.md:128-137`) — the Task 19 completion line records: "Erratum candidates: the brief listing (delays starting at recv, Eof finality, kill not ending a hung worker, the InvalidateIdentity/Delay code)."
- **Commit(s):** `c148197` + `049b661` + `ecf5d4d` + `b624244` (three fix rounds; re-review 2 Approved 3/3, 0 new).

### Task 20 — Absolute deadlines and the independent watchdog (P2.T20)

**Erratum 1 — the plan's boolean-only `terminal_seen`/`street_violation` interface**

- **Task/step:** Step (Produces), the `Armed` struct's `terminal_seen`/`street_violation` fields, plan lines 4677, 4730-4734, 4855, 4869 (Produces bullet and sample construction), and plan lines 6459-6491 (the caller's use of a plain `terminal_seen: Arc<AtomicBool>`).
- **Plan text (<=25 words):** `Armed { identity, street_deadline_ms, fire_ms, retained, fallback, stage, sink, delivered, terminal_seen, street_violation }` (line 4677) — booleans with no timestamp.
- **What the code does instead:** the first terminal's engine-clock arrival time is preserved in the shared watchdog state (`StreetDeadline`), or a late arrival is latched as a violation atomically at publication in that same coordinated state; street-deadline compliance is judged from that arrival time, never from when the watchdog thread happens to resume.
- **Why:** Ruling (20-I1), Important, plan-mandated (`progress.md:141`) — "the plan's boolean-only `terminal_seen` interface is superseded by that timestamped state (plan-2 erratum candidate)."
- **Commit(s):** `41e8b26` (fix round 1) + `2440d36` (round 1b) + `0c19844` (tests-only N1 round, orchestrator-verified).

**Erratum 2 — the plan's yield-loop test synchronization pattern**

- **Task/step:** the watchdog tests' synchronization, plan lines 4742 and 4781 (`std::thread::yield_now();` loops).
- **Plan text (<=25 words):** `std::thread::yield_now();` in a bounded spin loop (lines 4742, 4781) as the test's synchronization mechanism.
- **What the code does instead:** the watchdog tests synchronize by explicit acknowledgements (a gated test clock, a notifying sink, a test-only completion seam), never by bounded yield or settle loops; negative assertions run only after the relevant work is acknowledged.
- **Why:** Ruling (20-I2), Important, plan-mandated (`progress.md:142`) — a yield-loop is inherently racy for negative assertions ("nothing happened yet").
- **Commit(s):** `41e8b26` + `2440d36` + `0c19844`.

**Erratum 3 — the plan's gate command does not account for its own feature-gated test doubles**

- **Task/step:** the plan's per-task green-command instruction versus `tests/watchdog.rs`, plan line 4884 (`Run: cargo test -p engine --features testing then cargo test --workspace --release`).
- **Plan text (<=25 words):** `Run: cargo test -p engine --features testing` then `cargo test --workspace --release` (line 4884) — the second command does not enable `testing` for `engine`'s own integration test target.
- **What the code does instead:** `crates/engine/Cargo.toml` gains a self dev-dependency `engine = { path = ".", features = ["testing"] }`, so the crate's own integration tests see `engine::testing` under plain `cargo test --workspace`; the `cfg(feature)` gate on `tests/watchdog.rs` is removed, and the feature never enters the production build.
- **Why:** Ruling (20-C1), plan-mandated (`progress.md:138`) — without this, `cargo test --workspace` (the plan's own stated gate) compiled zero of `tests/watchdog.rs`'s tests, a gap spanning Tasks 20-29.
- **Commit(s):** `bcb9bd5` (round 0b, before the Codex review).

### Task 22 — `EngineCore` and the `run_solve` happy path (P2.T22)

- **Task/step:** the street-violation predicate, plan line 5444 (`let violated = |core: &EngineCore, first: bool| !first && core.clock.now_ms() >= plan.deadlines.street_deadline_ms;`).
- **Plan text (<=25 words):** `let violated = |core: &EngineCore, first: bool| !first && core.clock.now_ms() >= plan.deadlines.street_deadline_ms;` (line 5444) — judges the street verdict from whenever the closure happens to run, not a preserved first-terminal arrival time.
- **What the code does instead:** the street verdict is judged from the same first-terminal engine-clock arrival time published to the shared `StreetDeadline` (Task 20's Ruling 20-I1), carried through `SolveOutcome.first_terminal_ms`; the client publishes the first matching terminal's arrival at receipt, before validation or recovery.
- **Why:** Ruling (22-I4), Important, plan-mandated (`progress.md:189`, "plan line 5444 (street verdict by first-terminal arrival time)"); also carried at Task 19 review time (`progress.md:174`, "Task 23's street predicate keeps the first-terminal arrival-time rule (plan line 5444 is a carried plan defect)").
- **Also recorded:** Ruling (22-I3) — a worker `ok` that misses the raw target (because the worker compared against its own f32-rounded threshold) must be an explicit worker-contract `EngineError`, never `BestSoFar`/`Exact`; the brief's "D2 unreachable without a suspend" framing for a related identity-check window is listed as a second Task 22 erratum candidate (`progress.md:189`) but has no isolated plan quote beyond the suspend/resume handling generalized in Ruling (22-I1). Follow-up P2.W1 later makes the worker compare against the raw target using the engine's own f64 conversion, closing the root cause of 22-I3.
- **Commit(s):** `16747d6` + `fd19b1c` (two fix rounds) + `c65d308` (tests-only round 2).

### Task 23 — `run_solve` resilience: heartbeat, cancel-then-kill, `_min` retry, error-code policy (P2.T23)

**Erratum 1 — a fixed 10 s stand-in for the `_min` template's measured p95**

- **Task/step:** the retry-admission check, plan line 5470.
- **Plan text (<=25 words):** `let p95 = street_budget_ms(input.root.street, 10);` (line 5470) — a fixed 10-second stand-in for the measured p95 of the `_min` retry template.
- **What the code does instead:** kept as the stand-in for now (plan 4's timing matrix does not yet exist to supply a measured p95), but flagged: at a raised flop budget this fixed figure under-reserves against the actual retry cost.
- **Why:** Ruling (23-D3), Important, accepted (`progress.md:197`) — "spec 7 line 452: measured p95 of the _min template plus margins, before a report exists the street budget — the brief fixed 10 s under-reserves at a raised flop budget: erratum candidate."
- **Commit(s):** `26c67c0` + `54668b3` + `b81b9b1` (two fix rounds; re-review 2 Approved 1/1).

**Erratum 2 — error branches skip the expiry check**

- **Task/step:** the brief's two-worker-exits test case and the attempt's error-branch handling.
- **What the code does instead:** identity and the watchdog cutoff are checked on every receive result, including `Eof`/`Spawn`/`Protocol` errors, before classification or restart; at or after the fire, the attempt ends `DeadlinePassed` with no restart.
- **Why:** Ruling (23-I1), Important, plan-mandated (`progress.md:197`) — "the brief two-worker-exits case uses delayed Exit{code} events before the cutoff plus a separate EOF boundary test — inherited brief defect (erratum candidate)."
- **Commit(s):** `26c67c0` + `54668b3` + `b81b9b1`.

**Erratum 3 — the brief's `cancel_or_kill` helper accepts a late confirmation**

- **Task/step:** `cancel_or_kill`'s 1.5 s cancellation-confirmation window.
- **What the code does instead:** `cancel_or_kill` reads the clock immediately after its receive and enforces the 1.5 s bound before accepting `result{cancelled}`; an expired window routes to kill-and-restart.
- **Why:** Ruling (23-I2), Important, plan-mandated (`progress.md:197`) — "brief helper defect (erratum candidate)."
- **Commit(s):** `26c67c0` + `54668b3` + `b81b9b1`.

**Also recorded, not a code change:** Ruling (23-I3)'s non-retryable failed-restart classification is accepted as the brief's own conservative rule (not an erratum); the `attempt`/`attempt_no` naming split from plan-1-era review B1 is confirmed already corrected in the brief (not a live defect).

### Task 24 — Coverage classifier (§6) and `coverage_classification_golden` (P2.T24)

- **Task/step:** `seat_index`, plan line 5622.
- **Plan text (<=25 words):** `pub fn seat_index(state: &HandState, seat: Seat) -> usize { state.dealt.iter().position(|s| *s == seat).expect("dealt seat") }` (line 5622) — the seat's position within `dealt`, not `Seat.0`.
- **What the code does instead:** `seat_index` returns `usize::from(seat.0)` with a dealt-seat assertion, because `proto::Derived`'s per-seat vectors (e.g. `all_in`, used at line 5630/5646) are documented as indexed by `Seat.0` directly, and gapped dealt tables are valid; the brief's position-in-`dealt` lookup misreads `all_in` on a gapped table.
- **Why:** Ruling (24-D1), Important, plan-mandated, confirmed by the reviewer (`progress.md:186,190`) — "proto hand.rs:146; the brief position-in-dealt lookup is wrong: erratum candidate."
- **Also recorded:** Ruling (24-M1) — the golden fixture as originally written lacked a retained test case for the `Preflop` dispatch and for the `Inconsistent`-root -> `EngineError{retryable:false}` mapping; closed in a batched tests-only round with Task 27's 27-M1, orchestrator-verified.
- **Commit(s):** `5fbb841` (implementation) + `19c0adc` (tests-only round, with 27-M1).

### Task 26 — Result assembly, headline rules, reason accumulation, equity merge (`recommendation_assembly_golden`) (P2.T26)

**Erratum 1 — whole-collection equity replacement instead of per-pot/per-seat merge**

- **Task/step:** the §4.4 equity-merge sketch, plan lines 5921-5923 (`EquitySummary` construction comment "an Equity event after Final enriches it; Pending never replaces Ready", followed by a literal replacement-style sample).
- **What the code does instead:** per-pot equity merges match pots by `pot_index` and shares by seat under the same availability ordering as the pairwise populations (`Pending` never replaces `Ready` or `Unavailable`; `Unavailable` never replaces `Ready`); omitted pots and seats are retained with consistent population metadata.
- **Why:** Ruling (26-I1), Important, plan-mandated (`progress.md:150`) — "spec 4.4 (Equity enriches, Pending never replaces Ready) applies to every EquityEstimate; the brief's whole-collection replacement is the erratum."
- **Commit(s):** `9f0ec78` (implementation) + `c87bd5a` (fix round 1, re-review Approved 4/4).

**Erratum 2 — the brief's zero-reach `range_mix` sketch omits the mix entirely**

- **Task/step:** the §4.4/§6 `range_mix` sketch at zero available reach.
- **What the code does instead:** `range_mix` stays present whenever a node strategy exists: at zero available reach it is `Some(range_mix(node, reach))` with zero entries, accompanied by an explicit no-reach note, never presented as a normalized strategy.
- **Why:** Ruling (26-I2), Important, plan-mandated (`progress.md:151`).
- **Commit(s):** `9f0ec78` + `c87bd5a`.

**Erratum 3 — no golden case for a positive unresolved share**

- **Task/step:** the assembly/payload boundary's golden fixture set.
- **What the code does instead:** a named unresolved-mass golden case was added: known action masses preserved, a positive unresolved share, the explicit percentage note and display data, no headline of any kind, built from the assembly's real inputs.
- **Why:** Ruling (26-I3), Important, plan-mandated (`progress.md:152`).
- **Commit(s):** `9f0ec78` + `c87bd5a`.

### Task 27 — Snapshot store and the root-range source (P2.T27)

**Erratum 1 — `solved(other, ..)` after `&other` is a borrow-then-move compile defect**

- **Task/step:** a test helper, plan line 6134.
- **Plan text (<=25 words):** `assert!(s.register(&other, solved(other, Street::Turn)));` (line 6134) — borrows `other` in the first argument, then moves it into `solved(...)` in the second.
- **What the code does instead:** `other.clone()` is passed to `solved(...)` instead.
- **Why:** Ruling (27-D2), Important, plan-mandated, accepted by the reviewer (`progress.md:187,191`) — "the brief `solved(other, ..)` after `&other` is a compile defect."
- **Commit(s):** `8f03913` (implementation) + `17f81c2` (tests-only round, with 24-M1).

**Erratum 2 — the failure-table citation names spec section 13 instead of 12**

- **Task/step:** `ranges_at_root`'s `InvalidRanges` rationale (comment/doc citing the spec's failure table).
- **What the code does instead:** unchanged in code (the validation behavior — `InvalidRanges` for non-finite/out-of-range weights and for a pair with no compatible holding, checked before board blocking — was already correct); the citation itself is flagged as wrong.
- **Why:** ruling accepted at review (`progress.md:191`), plan-mandated — "the failure-table row is spec section 12 line 652, not 13 (erratum candidate for the brief)." Rulings (27-D3/D4) (input-validation ownership) were accepted alongside this without requiring a code change.
- **Commit(s):** `8f03913` + `17f81c2`.

### Task 28 — `serve_request`: the river/turn decision path and `identity_race_golden` (P2.T28)

**Erratum 1 — the all-in fallback's `facing` is the whole wager, not what hero owes**

- **Task/step:** the analytic all-in fallback's `AllInInput` construction, plan line 6514.
- **Plan text (<=25 words):** `facing_allin(&AllInInput { ..., facing: d.facing, ... }, ...)` (line 6514) — `d.facing` is the full wager amount other players must call, not hero's own call cost.
- **What the code does instead:** the fallback is fixed to use hero's actual call cost, not the whole wager; a regression reproduced a 20-chip pot shortfall from the plan's version.
- **Why:** Ruling (28-D5), Important, accepted, plan-mandated (`progress.md:210,228`) — "the brief all-in fallback `facing: d.facing` is the whole wager, not what hero owes (pot short by 20 chips in the regression) — accepted, erratum candidate."
- **Commit(s):** `716de90` (implementation) + `9871f03` + `acfdabe` + `00ce4bb` (three review rounds).

**Erratum 2 — an unconditional clock-based post-final kill/restart**

- **Task/step:** the post-final worker cleanup, plan lines 6532-6533.
- **Plan text (<=25 words):** `if matches!(&out.terminal, Terminal::Failed(UnsupportedReason::DeadlineExceeded { .. })) { core.worker.kill(); let _ = core.worker.restart(); }` (lines 6532-6533) — read on its own this can fire whenever a `Final` is `DeadlineExceeded`, including after `no_iteration` or an unsent request.
- **What the code does instead:** post-final kill/restart happens only when `DeadlineExceeded` is observed at or after the watchdog fire, never after `no_iteration` or an unsent request (which would kill an otherwise idle, healthy worker).
- **Why:** Ruling (28-D4), Important, accepted (`progress.md:210`) — narrows the brief's condition; recorded in the completion line as "the clock-based post-final restart condition" (`progress.md:228`).
- **Commit(s):** `716de90` + `9871f03` + `acfdabe` + `00ce4bb`.

**Erratum 3 — the `m14` accuracy-string formula understates the bound**

- **Task/step:** `assumptions.source_accuracy` formatting, plan line 6307.
- **Plan text (<=25 words):** `format!("exploitability <= {reached_bp} bp")` (line 6307, per review m14) — nearest-rounds `reached_bp`, which can round down and make the printed "<=" claim false by up to 0.5 bp.
- **What the code does instead:** the string rounds up so the printed bound is never understated relative to the measured exploitability.
- **Why:** Ruling (28-Q1), Minor, accepted (`progress.md:210,228`) — "if ruling m14 rounding can make the 'exploitability <= x bp' string false by up to 0.5 bp, the string must round up for the claim (MINOR fix here)."
- **Commit(s):** `716de90` + `9871f03` + `acfdabe` + `00ce4bb`.

**Erratum 4 — supersession matched by error-message text**

- **Task/step:** the superseded-outcome check, plan line 6529.
- **Plan text (<=25 words):** `let superseded = matches!(&out.terminal, Terminal::Failed(UnsupportedReason::EngineError { message, .. }) if message.contains("superseded"));` (line 6529) — matches on a substring of a human-readable message.
- **What the code does instead:** supersession is decided by `identity_active`, a typed identity check, not by scanning an error string for the word "superseded".
- **Why:** ruling accepted (`progress.md:210,228` — "identity lock held through emission and snapshot registration with the documented order identity -> sink/store, supersession by identity_active not by message text — accepted"; recorded in the completion line as "the brief matching supersession by error-message text."
- **Commit(s):** `716de90` + `9871f03` + `acfdabe` + `00ce4bb`.

**Erratum 5 — spec §12 line 645's `best_so_far` violation rule not reflected in the brief's street rule**

- **Task/step:** the street-violation predicate's handling of a `best_so_far` terminal.
- **What the code does instead:** left for the reviewer's judgement in the fix round (Ruling 28-D6: "a river/turn best_so_far logged as a street violation — reviewer decides from spec 7 and 12 wording"); recorded as an erratum candidate rather than resolved unilaterally in this task.
- **Why:** `progress.md:210,228` — "spec section 12 line 645 best_so_far violation not reflected in the brief street rule."
- **Commit(s):** n/a (recorded as a candidate; see the final review's F-M8/general street-violation treatment for a related, later-resolved item).

**Also recorded, not a plan-2 code erratum:** Ruling (28-Q3) — the Task 20 watchdog emitting `Final` without an identity check became follow-up P2.W3, not a Task 28 fix (see Follow-ups below).

### Task 29 — Public `Engine` API, startup report and `final_delivery_independent_of_worker` (P2.T29)

**Erratum 1 — a rejected `begin_hand` burns the hand id**

- **Task/step:** the `begin_hand` admission path, plan lines 6813-6825 region (Engine's admission handling; ledger paraphrase, no single isolable plan line for the id-burning behavior itself).
- **What the code does instead:** a rejected `begin_hand` leaves the hand in progress unchanged and burns no hand id.
- **Why:** Ruling (29-D1), confirmed by spec 12 line 639 ("state unchanged"), accepted (`progress.md:229,235`) — "the brief burned the id (erratum candidate)."
- **Commit(s):** `8d6e59b` (implementation) + `90e0690` (fix round 1).

**Erratum 2 — the brief's detached-thread shutdown sketch**

- **Task/step:** `Engine::shutdown`'s thread-teardown sketch.
- **What the code does instead:** `Engine` owns every thread it causes through an owned task registry with an interruptible watchdog stop; `shutdown` invalidates/cancels, stops scheduling, wakes and joins watchdog and equity work, and joins the request loop in an order that cannot wait on a held lock, before killing/reaping the process.
- **Why:** Ruling (29-I2), Important, plan-mandated (`progress.md:235`) — "the brief shutdown sketch is an erratum candidate."
- **Commit(s):** `90e0690` (fix round 1); re-review 1 Approved.

**Erratum 3 — the brief's `Arc<Mutex<EngineCore>>` sketch holds an engine lock during callbacks**

- **Task/step:** `EngineCore` ownership, plan line 6813 (`core: Arc<Mutex<EngineCore>>, slot: Arc<(Mutex<Slot>, Condvar)>,`).
- **Plan text (<=25 words):** `core: Arc<Mutex<EngineCore>>, slot: Arc<(Mutex<Slot>, Condvar)>,` (line 6813).
- **What the code does instead:** `EngineCore` is owned exclusively by engine-main, with no `Arc<Mutex<EngineCore>>` guard held across `serve_request`, so no engine lock is held during any sink callback; a `try_lock` probe sink pins this.
- **Why:** Ruling (29-I3), Important, plan-mandated (`progress.md:235`) — "the brief sketch is an erratum candidate," ties directly to Ruling (28-I1)'s no-lock-during-callback rule.
- **Commit(s):** `90e0690` (fix round 1); re-review 1 Approved.

**Erratum 4 — the brief's fail-fast `Engine::new` versus spec §12's degraded-engine reading**

- **Task/step:** `Engine::new`, plan line 6818.
- **Plan text (<=25 words):** `pub fn new(cfg: GameConfig, paths: Paths) -> Result<Engine, EngineError> { let worker = ProcessWorker::spawn(...).map_err(...)?; ... }` (line 6818) — construction fails outright on any worker-launch problem.
- **What the code does instead:** invalid config and a transient spawn failure (executable missing, launch fault) stay construction errors as the brief has it; but a **permanent ready refusal** (proto/commit/adapter/AVX2 mismatch) instead yields a DEGRADED engine — `Engine::new` succeeds, `StartupReport` reports `worker_ready: false` with a typed reason, and every `recommend` answers a non-retryable `EngineError` without relaunching the refused build.
- **Why:** Ruling (29-I4), Important, plan-mandated, supersedes the provisional Ruling (29-Q1, provisional) under precedence spec > plan (`progress.md:235`) — "spec 12 line 658; 3.7 line 141 is compatible." This is also final-review finding P4/29-I4's spec-tension item; see the Spec questions section below.
- **Commit(s):** `90e0690` (fix round 1, folds in follow-up P2.W2's typed `ReadyRefusal`); re-review 1 Approved 7/7 including W2.

### Task 30 — `bench run` for the river and turn suites with the §13.5 report (P2.T30)

- **Task/step:** the plan's bench-runner pseudocode (spot-id construction, argument parsing, report generation, and the cancel/violation predicates).
- **Plan text:** the ledger records the brief's pseudocode as carrying "a spot-id padding bug, nonexistent arg() helper, read-error suppression, cancel loop, wall-only violation predicate, missing metadata and frontmatter" (ledger paraphrase, `progress.md:180`; the plan's own pseudocode is prose/pseudocode rather than compilable Rust, so no single line is quoted verbatim here to avoid misrepresenting it as literal plan text).
- **What the code does instead, per finding (all Ruling ids verified in the ledger):**
  - Ruling (30-I1): cancel measurement tracks the ack and the terminal as independent optional observations, distinguishing completed-before-cancel from a transport/protocol timeout, with numeric latency only for an observed cancellation.
  - Ruling (30-I2): the street-violation predicate includes the river/turn target-miss outcome (raw target comparison, never display bp) besides late completion — not a wall-clock-only predicate.
  - Ruling (30-I3): each appended run renders hardware/toolchain/target/compiler/worker-commit/adapter metadata from `ready`, never inventing historical values.
  - Ruling (30-I4): `SolveRequest.spot` is the SHA-256 hex of the structural solve identity (not a padded display id); `SpotResult.spot` keeps the human-readable name.
  - Ruling (30-I5): a rejected solve ack is an immediate explicit benchmark failure, never a wait for a nonexistent terminal.
  - Ruling (30-I6): missing time-to-target statistics render as `n/a` with the successful-sample count, never a fabricated numeric zero.
  - Ruling (30-I7): only `NotFound` is treated as a missing report; any other read error propagates before the destination is opened or truncated.
  - Ruling (30-M1): the `docs/CONVENTIONS.md` YAML frontmatter is written once on a new/empty output file, never duplicated on append.
- **Why:** all eight rulings above are tagged Important/Minor and plan-mandated at `progress.md:155-162`, with the summary line "Plan-2 erratum candidates: every brief-inherited defect above."
- **Commit(s):** `44c7969` + `c1995d4` + `d031f90` + `e68b07e` (two fix rounds plus a mechanical N4 round, orchestrator-verified).

---

### Follow-ups W1-W3

**P2.W1 — the worker's target-met predicate did not use the engine's own conversion**

- **Task/step:** plan 2 lines 2428, 2517, 2932 (`solve_loop::LoopParams { deadline_ms: u32, extraction_margin_ms: u32, target_chips: f32, started: Instant }`, and its construction `target_chips: req.pot as f32 * req.target_bp as f32 / 10_000.0`).
- **Plan text (<=25 words):** `pub struct LoopParams { pub deadline_ms: u32, pub extraction_margin_ms: u32, pub target_chips: f32, pub started: Instant }` (line 2517); `target_chips: req.pot as f32 * req.target_bp as f32 / 10_000.0` (line 2932).
- **What the code does instead:** the generator hands `solve()` the largest `f32` that still meets the raw target (Ruling W1-D1, output byte-identical), and the worker's own `ok`/`best_so_far` comparison now uses the same raw-target-with-engine's-f64-conversion rule the engine uses, closing the gap Ruling (22-I3) identified.
- **Why:** Ruling (W1-D3/Q1), plan-mandated (`progress.md:206`) — "`LoopParams { target_chips: f32 }` in plan 2 lines 2428/2517/2932 is an erratum candidate for the plan-2 changelog at plan end (no plan edit now)."
- **Commit(s):** `dfb3bb6` (implementation, merged `3870e89`); review P2W1R Compliant/Approved, 0 findings.

**P2.W2 — a permanent ready refusal needed a typed error, not string parsing**

- Folded into Task 29's fix round 1 (`90e0690`): a typed `WorkerLinkError`/`ReadyRefusal` variant for a permanent ready refusal, with exhaustive matches updated (no classification change per Ruling 23-I3) and the W1 leftovers in `terminal_for`'s inline comment/error text closed. This is the mechanism Ruling (29-I4)'s degraded engine needs to avoid string-matching a human-readable ready-refusal message. Not itself a plan-text erratum — it is the follow-up brief `w2-brief.md` dispatched to close the open item recorded at Task 23 (`progress.md:197`, follow-up P2.W2).

**P2.W3 — the Task 20 watchdog emitted `Final` without an identity check (Ruling 28-Q3)**

- **Task/step:** the watchdog's fire-and-claim path (`watchdog.rs`), following on from Task 20's `Armed`/claim design.
- **What the code does instead:** `Armed` gains `identity_state`; `claim_if_active` checks the identity and claims delivery in one step under the generation lock, releasing the identity lock before recording `fired` and emitting (so Ruling 28-I1's no-lock-during-callback rule holds). A `Final` the watchdog delivered while the decision was active is logged exactly once even if the decision was superseded after the claim — both stale exits now retire the watchdog first, then log through `log_final` with its watchdog/deadline provenance if the slot holds a fired record, or record nothing if the slot was empty (superseded before the claim).
- **Why:** Ruling (28-Q3) (`progress.md:210`) — "the Task 20 watchdog emits its Final without an identity check — if the reviewer confirms a spec 4.3/4.4 breach it becomes follow-up P2.W3 (watchdog.rs, own round), not a Task 28 fix"; confirmed and implemented as Ruling (W3-D1)/(W3-D2)/(W3-D3), reviewed with Ruling (W3-I1) (one Important finding, fixed in one round), and refined again as Ruling (W3f-D2)/(W3f-Q1) in a follow-up fix round.
- **Commit(s):** `1b51e84` (implementation) + `55397c6` (fix round 1, merged `be5522c`); review P2W3R 1 Important fixed, orchestrator-verified.

---

### Plan-level issues from the final review

The final whole-branch review (`.superpowers/sdd/2026-09-10-plan-2-worker-engine/final-review.md`, Opus, base `7edd7f2`, head `be5522c`; verdict "Ready to close plan 2: With fixes") found 0 Critical, 4 Important (I1-I4) and 12 Minor findings against code the 30 task reviews above had already passed, plus six plan/spec-level erratum candidates P1-P6 in its "Plan issues (erratum candidates)" section. The orchestrator's rulings for these (`progress.md`, "Plan-2 final whole-branch review (Opus, final-review.md)" entry) are Ruling (F-I1) through Ruling (F-I4) and Ruling (F-M1) through Ruling (F-M12); **the one fix round implementing them had not yet landed when this changelog was written** (it is scheduled after plan 3 Task 14 merges, because both edit `serve.rs`/`core.rs`) — this section records the plan-level defects the review found and how the orchestrator ruled on each, not a claim that the code has already changed.

- **P1 (plan 2 Tasks 28/29):** the plan arms the watchdog inside `serve_request` (at dequeue), and its `recommend` sketch only enqueues, so a queued request has no independent watchdog until `engine-main` reaches it. This conflicts with spec 7's independent watchdog and spec 5 step 4 (cancel any other live job at request time); see final-review finding I1. Ruling (F-I1) (not yet landed): share the `Watchdog` (`Arc`) with `Engine`, build the new request's `Armed` in `recommend` itself, and pass the claim/fallback/`fired`/`StreetDeadline` into `serve` via `LiveRequest`; `run_attempt`/`cancel_or_kill` receive in slices of at most ~100 ms, re-checking identity between slices.
- **P2 (plan 2 Task 2 vs plan 3 Tasks 14/18):** `IdentityState::set_config` (plan 2 Task 2's own sketch) makes every later decision of the active hand carry the new session `config_revision`, although spec 4.2/12 say a mid-hand settings change "is not applied to the active hand"; plan 3 Task 14 (line 2375) and plan 3 Task 18 (line 2968) disagree with each other about which revision the active hand keeps. See final-review finding I4. Ruling (F-I4) (orchestrator ruling, not yet landed): a decision carries its hand's own `HandConfig.config_revision`, kept apart from the session counter; `set_config` allocates the session revision without invalidating the active hand's decisions; plan 3 Task 14's `for_identity` must compare by the hand revision.
- **P3 (plan 4 lines 81, 100, 3051; plan 2 Global Constraints "background"):** `SolvePlan.background` is described in plan 4 as a working seam, but plan 2's `run_solve` cannot serve a background job — its identity, deadline (`Deadlines::for_request` street budgets, not 600 s) and sink assumptions all require the plan's identity to be the active decision. See final-review finding M5; carried to plan 4.
- **P4 (spec 3.7):** the "CPU without AVX2 gets a startup banner" path is unreachable as built: `.cargo/config.toml` compiles every crate — including the engine and the Tauri app — with `+avx2`, so a non-AVX2 CPU can fault before any banner is ever shown, and the worker possibly before `ready`. Not yet resolved in code; recorded as a spec question below (the review offers two options: build the app without `+avx2` and gate the worker launch on a CPUID check in the engine, or drop the banner clause from the spec).
- **P5 (spec 4.5 vs 12):** spec 4.5 says a terminal `result` releases worker admission; spec 12's cancel row names only `result{cancelled}` as confirmation. Ruling 23-Q1 took the stricter reading (ack alone never confirms), which the final review's finding I2 shows can needlessly kill a healthy worker that already delivered its own terminal. Ruling (F-I2) (not yet landed) fixes the code-level race (rewriting `Superseded{running:true}` to `running:false` when the message is this solve's own `Result`/`ack{rejected}`); the spec-level question (amend spec 12 to accept the target's own terminal, or a post-terminal `already_finished` ack, as confirmation) is recorded below, unresolved.
- **P6 (spec 7/12):** the spec does not say what a first attempt that cannot be sent before its own street deadline should do; see final-review finding M8. It should behave like `no_iteration` and retry the `_min` template under `retry_admitted`, per Ruling (F-M8) (not yet landed, included in the planned fix round). Recorded as a spec question below.
- **Ledger candidates the final review confirmed, without contradicting them:** the `terminal_seen` AtomicBool (Task 20/22/28, above); the all-in `facing: d.facing` (Task 28, above); the m14 accuracy formula (Task 28, above); supersession matched by message text (Task 28, above); the `begin_hand` destructive rejection (Task 29, above); the detached-thread shutdown sketch (Task 29, above); `Arc<Mutex<EngineCore>>` (Task 29, above); fail-fast `Engine::new` (Task 29, above); the 23-D3 p95 proxy at a raised flop budget (Task 23, above); the 27-D2 compile defect (Task 27, above); the 24-D1 seat index (Task 24, above); the 30-* bench items (Task 30, below); the vacuous "same displayed revision" clause of `identity_race_golden` (not separately itemized here — see `final-review.md`, Minor findings).

---

## Spec questions raised (not resolved here)

These are tensions between the spec and what plan 2 built, surfaced during execution and recorded for the orchestrator; none is resolved by editing the spec in this document (per this task's scope, doc-only, no spec edit).

1. **P4 — spec 3.7's AVX2 startup-banner path is unreachable.** `.cargo/config.toml` compiles the whole workspace, including the engine and the Tauri app, with `+avx2`; a non-AVX2 CPU can fault before any startup banner is shown. Final review options: build the app without `+avx2` and CPUID-gate the worker launch, or drop the banner clause. Unresolved.
2. **P5 — cancel confirmation: spec 4.5's "terminal result releases admission" versus spec 12's cancel row naming only `result{cancelled}`.** Ruling 23-Q1 took the stricter reading; the final review's finding I2 shows this can kill a healthy worker that already delivered its own terminal to the client. Whether spec 12 should also accept the target's own terminal (or a post-terminal `already_finished` ack) as confirmation is unresolved.
3. **P6 — a first attempt with no room at send time.** The spec does not state what happens when a first attempt cannot be sent before its own street deadline; final-review finding M8 recommends treating it like `no_iteration` (retry the `_min` template under `retry_admitted`). Unresolved as a spec text change; Ruling (F-M8) plans to implement the behavior once the next fix round lands.
4. **Spec 12 line 658 versus spec 3.7 line 141 — the degraded-engine reading.** Ruling (29-I4) resolved this *in code* under precedence spec > plan: a permanent ready refusal yields a degraded `Engine` (construction succeeds, every `recommend` answers a non-retryable error) rather than the plan's brief fail-fast `Engine::new`. The spec text itself was judged compatible ("3.7 line 141 is compatible") rather than contradictory, so no spec changelog entry was required for this one — it is recorded here because Ruling (29-Q1, provisional) had initially read the two spec passages as being in tension before Ruling (29-I4) settled it in favor of spec 12's degraded-engine reading.

---

## Process notes (not plan-text errata)

- **Task 18** (see "Not an erratum" above): its Important findings were about the implementer's own `WorkerLink`/`ProcessWorker` construction, not the plan's prescribed Step-block text.
- **Task 21** (Decision log, §5 step 10): review P2T21R found 4 Important + 1 Minor findings (unchecked version fields, an unchecked scenario/tier pair and domain, swallowed rotation-deletion errors, gate evidence, one Minor), all fixed in one round (`6a359ec` + `171a21a`) — none were tagged `erratum candidate` in the ledger, consistent with these being implementer-side defects rather than plan-mandated ones; excluded here for the same reason as Task 18.
- **Ruling (18-I3)** (`progress.md:111`): a process erratum recorded against the orchestrator, not the implementer — plan 2's Global Constraints require `cargo test --workspace --release` at the end of every task, but the orchestrator's merge gates had been running only the debug workspace suite. From Task 18 onward every plan-2 merge gate runs both the debug suite and, after building `solver-worker` in release, the release workspace suite. This is an orchestration-process correction, not a plan-2 text defect, and is recorded here rather than in "Errata by task."

---

## Verification

Documentation-only task: **Rust/Python/UI execution: not applicable** — this task edits only `docs/research/PLAN-2-CHANGELOG-4.md`, the plan's revision line, and today's journal entry; no code changed. Verification is the check named in the dispatching brief: every erratum's cited `Ruling (<id>)` was independently confirmed present in `.superpowers/sdd/2026-09-10-plan-2-worker-engine/progress.md` by grep before this document was written (see the agent's report, `.superpowers/sdd/2026-09-10-plan-2-worker-engine/errata-report.md`, for the full grep output), and every plan line number quoted above was independently confirmed by grep against `docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md`.
