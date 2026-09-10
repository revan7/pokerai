# Review of plan 2 (solver worker + engine river/turn), 2026-09-10

Plan: `docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md` (5,678 lines, 22 tasks, 90 checkbox steps).
Spec: `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` revision 5. Brief: series map row "Plan 2".
Solver source verified at the pinned commit `9d1509fe` (`spike-solver/postflop-solver/src/{action_tree,range,card,solver,utility}.rs`, `src/game/{base,interpreter}.rs`).

**Verdict: NOT READY.** 3 BLOCKER, 9 MAJOR, 15 MINOR.

The plan is unusually strong on substance: the §4.6 materializer is a faithful transcription of `push_actions` / `create_next` (I re-derived every boundary in the plan's tests — 150/151, 350/400/401, 340/341/240, the cross-street `matched` accumulation, the donk pinning, the clamp-force-dedupe order — and all of them are arithmetically correct against the pinned source), the §7 stop rule, the §10.3 admission rule and the T1 facing-all-in numbers all check out. The blockers are mechanical: one compile error, one workspace-ordering error, and a set of `proto` type conflicts with plan 1.

---

## BLOCKER

**B1 — Task 16, step 4 (`crates/engine/src/solve.rs`, `run_solve`): loop variable shadows the function it calls; the crate cannot compile.**
> "for attempt in 0..2u8 { … let end = attempt(core, plan, sink, &req);"

`fn attempt(core, plan, sink, req) -> AttemptEnd` and the loop binding `attempt: u8` are both in the value namespace; inside the loop body the binding wins, so `attempt(...)` is `expected function, found u8`. The same body then relies on the integer (`if attempt == 0 …`, `filter(|_| attempt == 0 …)`), so neither can simply be dropped.
**Edit:** rename the loop variable to `attempt_no` (keep `fn attempt`), or rename the function to `run_attempt`. Update the three uses of the integer and the one call site.

**B2 — Task 1, step 3: the workspace gains members that do not exist until tasks 2, 5 and 7, so every `cargo` command fails from task 1 to task 6.**
> "In the root `Cargo.toml` add `"solver-worker"`, `"crates/engine"`, `"crates/bench"` to `members`"

`crates/engine` is created in task 2, `crates/bench` in task 5, `solver-worker` in task 7. Cargo refuses to load a workspace whose member manifest is missing ("failed to load manifest for workspace member"), so task 1's own `cargo tree` check and the brief's requirement ("`cargo test --workspace` green after every task") both fail immediately.
**Edit:** in task 1 step 3 add only `exclude = ["third_party/postflop-solver"]`. Add `"crates/engine"` to `members` in task 2 step 1, `"crates/bench"` in task 5 step 3, `"solver-worker"` in task 7 step 3 (each already edits the root manifest or creates the crate).

**B3 — Header "Interfaces consumed from plan 1": six `proto` items do not exist, or do not have the assumed shape, in plan 1; one of them cannot be reconciled by renaming.**
> "`PlayerMenus { oop: SideMenu, ip: SideMenu, donk: Option<Vec<MenuSize>> }` … `MenuSize::{Pot(f32), AllIn}` untagged"

Plan 1 (`2026-09-10-plan-1-foundation.md:1020-1029`) defines `Menu { bet: Vec<f32>, raise: Vec<RaiseSize> }` and `PlayerMenus { oop: Menu, ip: Menu, donk: Option<Vec<f32>> }`. `Vec<f32>` **cannot express the `+ a` all-in entry that spec §10.1 gives `river_std_v1`** ("0.33, 0.75 + a"), which task 2 encodes as `spec("river_std_v1", River, &[(River, &[P(0.33), P(0.75), A])], …)` and task 3's `menu()` reads as `MenuSize::AllIn => v.push(allin.clone())`. This is a semantic conflict, not a `use` line. The other five:
- plan 1 has `ReadyInfo`, plan 2 uses `proto::worker::Ready` everywhere (tasks 7, 14, 15);
- plan 1 has `EngineMessage::Lock { id, spot, locks }` (struct variant), plan 2 uses `EngineMessage::Lock(LockRequest)` and `l.id / l.spot / l.locks` (task 10 `handle_message`);
- `proto::worker::SOLVER_COMMIT` and `proto::worker::ADAPTER_VERSION` **do not exist anywhere in plan 1** (used in tasks 7, 14, 15, 21);
- `PROTO_VERSION` lives at `proto::PROTO_VERSION` (plan 1 line 169), not `proto::worker::PROTO_VERSION`;
- plan 1's `BeginHand { hand_id, button, hero, dealt, stacks_start, hero_cards }` vs plan 2's assumed `{ button, hero, hero_cards, dealt, stacks }` (task 17 `hand()`, task 21 `Engine::begin_hand`).
**Edit:** reconcile in one pass before execution. Concretely: (a) have plan 1 export `MenuSize::{Pot(f32), AllIn}` and `SideMenu`/`PlayerMenus` over it (the spec's §4.6 `menus` must carry `"a"`), or add an `AllIn` marker to plan 1's `Menu`; (b) rename plan 2's `Ready` → `ReadyInfo`, `EngineMessage::Lock(LockRequest)` → the struct variant, and drop `LockRequest` from the header table; (c) add `SOLVER_COMMIT: &str = "9d1509fe…"` and `ADAPTER_VERSION: u16 = 1` to plan 1 task 7's Produces list and to `crates/proto/src/worker.rs`; (d) import `PROTO_VERSION` from the crate root; (e) fix the `BeginHand` literal in tasks 17 and 21 (`stacks_start`, and let `begin_hand` receive the `hand_id` the engine allocates).

---

## MAJOR

**M1 — Task 10, step 3 (`protocol.rs::handle_message`): the "busy" branch precedes the duplicate branch, so `reason: "duplicate"` is unreachable and the test asserting it fails.**
> "if p.state != WorkerState::Idle { drop(p); return rejected(shared, &req.id, "busy"); }"

A duplicate id of a *live* request implies `state != Idle`, so §4.5's rule "Duplicate id of a live request: `ack{rejected, reason: "duplicate"}`" is never reached. Task 10's `protocol_rejections` sends the flop solve twice with id `"11"` and asserts `ack_of(&w, "11")["reason"] == "duplicate"`; it will read `"busy"`.
**Edit:** move the duplicate test above the busy test: `if p.live.as_ref().is_some_and(|l| l.id == req.id) || p.finished.contains(&req.id) { … "duplicate" }` first, then the `stopping` and `state != Idle` checks.

**M2 — Task 10, step 1 (`protocol_rejections`) and step 3 (`lenient_id`): an id cannot be recovered from a line that is not valid JSON, so the `NaN` case gets `id: "unknown"`.**
> "w.send(&with_id(&river[0], \"4\").replacen(\"\\\"pot\\\":100\", \"\\\"pot\\\":NaN\", 1)); assert_eq!(ack_of(&w, \"4\")[\"status\"], \"rejected\");"

`lenient_id` is `serde_json::from_str::<Value>(line).ok()…unwrap_or("unknown")`; bare `NaN` is invalid JSON, so `from_str::<Value>` fails and the ack carries `"unknown"`. `ack_of` matches on `m["id"] == "4"` and will time out and panic.
**Edit:** make `lenient_id` fall back to a byte scan for `"id"` followed by the next quoted string when the JSON parse fails (a 6-line helper), or change the assertion to `w.recv_until(5*S, |m| m["type"]=="ack" && m["status"]=="rejected")` as the oversized-line case already does.

**M3 — Task 21, step 1 (`final_delivery_independent_of_worker`, case c) versus task 16 step 4: the retry resets the stage, so the expected `"extracting"` is reported as `"building"`.**
> "run(vec![ack(), FakeReply::Progress { … stage: Stage::Extracting … }, FakeReply::Hang], \"extracting\");"

`run_solve` executes `core.set_stage("building")` at the top of *every* attempt. In case (c) the first attempt ends `Hang` (retryable), the `_min` retry starts, the stage is overwritten with `"building"`, and the watchdog (and the `DeadlinePassed` path) report `DeadlineExceeded{stage: "building"}`.
**Edit:** set the stage to `"building"` only on `attempt_no == 0`, or better: have `attempt()` record the furthest stage reached for the request and have `run_solve` restore it before the retry. Then re-check case (b) (which legitimately expects `"building"`).

**M4 — Task 21, step 1 (`identity_race_golden`): the scripted `InvalidateIdentity` never yields, so no cancel and no kill happen; `cancels == 1 && kills == 1` fails.**
> "assert!(st.cancels.len() == 1 && st.kills == 1);"

`FakeWorker::recv` (task 15) loops over `InvalidateIdentity` and continues to the next scripted item in the same call, so it returns A's `Result{ok}` immediately. `attempt()` only tests `identity_active` at the *top* of its loop, so it accepts the result and returns `AttemptEnd::Result`; `cancel_or_kill` is never entered. (Everything else in the test does hold: the Final is suppressed by `emit`, and the snapshot is not registered because `active()` is `None`.)
**Edit:** insert `FakeReply::Delay { ms: 1 }` between `InvalidateIdentity` and A's late `Result` so `recv` returns `Ok(None)` and the loop re-checks the identity; and add an identity re-check immediately after `recv` returns a terminal in `attempt()` so a result that races a mutation is discarded rather than validated.

**M5 — Task 16, step 1 (`heartbeat_failure_restarts_and_retries_min`): `street_violation` is asserted but the heartbeat fires at t=5000 ms, inside the 6000 ms turn budget.**
> "assert!(out.street_violation, \"the first-attempt terminal never arrived before t0 + 6 s\");"

`run_solve` sets the flag only via `if attempt == 0 && core.clock.now_ms() > plan.deadlines.street_deadline_ms`. The scripted `Delay { ms: 5100 }` is consumed to 5000 ms by the 5 s heartbeat window, so the clock is 5000 at the end of attempt 0 and the flag stays false. Spec §7 defines the violation as "no first-attempt terminal arrived" by `t0 + street budget`, not "the attempt ended after it".
**Edit:** record `first_attempt_terminal: bool` (set only when attempt 0 produced a `Result`), and set `street_violation = !first_attempt_terminal && now_at_end_of_solve >= street_deadline_ms`, evaluated once after the loop; or arm the watchdog's `terminal_seen` flag from `run_solve` and read it there (this also removes the duplicate violation logic in `serve_request`).

**M6 — Task 22, step 1 (`percentiles_and_sections`): the asserted percentile contradicts the implementation.**
> "assert_eq!(percentile(&[1, 2, 3, 4], 0.5), 3);"

`percentile` is `idx = ceil(len * p).clamp(1, len) - 1` → `ceil(2.0) = 2`, `idx = 1`, `sorted[1] = 2`. The assertion expects 3.
**Edit:** change the assertion to `2`, or switch the implementation to the nearest-rank definition `idx = ceil(len * p) as usize` with a `min(len)-1` that rounds up (`((len as f64 * p).ceil() as usize).max(1).min(len)` then index `idx` when `p >= 0.5`). Pick one and make the other row of the same test (`p50 30 ms` over `[10,20,30,40,2500]`, which the current code satisfies) still hold.

**M7 — Task 12, step 2 (`wager_cap_remove_lines`): the "observed prefix survives" loop applies the whole line and then asserts its last action is available at the resulting node.**
> "while !line.is_empty() { t.apply_history(&line).unwrap(); assert!(t.available_actions().contains(&line[line.len() - 1])); line.pop(); }"

After `apply_history(line)` the current node is the child *after* `line`'s last action; its menu never contains that action. For `cap1_two_wagers` the first iteration asserts `[Fold, Call, AllIn(500)].contains(Raise(120))` and fails.
**Edit:**
```rust
while !line.is_empty() {
    let last = line.pop().unwrap();
    t.apply_history(&line).unwrap();
    assert!(t.available_actions().contains(&last), "{name}: {last:?} removed by the cap");
}
```

**M8 — Task 6, step 1 (`test_parse_range_counts`): the two asserted combo counts do not match the range strings.**
> "assert sum(1 for w in g.parse_range(g.BTN_OPEN) if w > 0) == 646 … g.parse_range(g.BB_DEFEND) … == 804"

I ran the plan's own grammar over the two strings: `BTN_OPEN` yields **634** combos and `BB_DEFEND` **720**; `CO_CALL_3BET` (194) and `BTN_3BET` (138) match R8 A.1 exactly, so the parser is right and R8's 646 / 804 figures do not correspond to the strings as written. The same strings are also frozen into `crates/bench/src/gen_spots.rs`, so the bench suites will not be comparable with addendum A.2 as claimed.
**Edit:** recompute both counts from the strings and assert 634 / 720 (and add a note that R8 A.1's 646 / 804 are not reproducible from its published strings), or recover the strings that actually produce 646 / 804 before freezing them in task 5 and task 6. Do this before `bench gen-spots` is committed.

**M9 — Tasks 11 and 14: `cargo test --workspace` is not green after those tasks, contrary to the brief.**
> "run the other four tests first with `cargo test -p solver-worker --release --test contract_river -- --skip ev_convention`"

(a) Task 11's `ev_convention_non_root_payoffs` reads `fixtures/worker/basic_turn_std_request.jsonl`, produced by task 12 step 1; the plan works around it with `--skip`, which leaves a failing test in the tree for one task. (b) Task 14's `worker_link.rs::exe()` asserts `target/release/solver-worker.exe` exists, so a plain `cargo test --workspace` (debug) panics in three tests. (c) The plan states that every worker test needs `--release` ("the library is 30x slower in debug and the timing assertions of §13.2 assume release"), but the workspace test command is never qualified.
**Edit:** move `gen_basic_fixture` (task 12 step 1) into task 11 as its step 1 so the fixture exists when the test lands; in `exe()` fall back to `env!("CARGO_BIN_EXE_solver-worker")`-style discovery or `option_env!("POKERAI_WORKER")` and `#[ignore]` the test when the binary is absent; and state once in Global Constraints that the per-task green command is `cargo test --workspace --release`.

---

## MINOR

**m1 — Task 2, step 6 / Task 8, step 3: the root street's `donk` is `None` in every template, which the spec's §13.2 row reads as a `tree_mismatch` trigger.**
> "let donk = if *s == root { None } else { Some(vec![]) };"

§4.6 pins "the turn and river donk menus to the explicit empty option" and §13.2 says "a `None` donk option … produce[s] `result{error{tree_mismatch}}`". The plan's `precheck`/`tree_config` only require `Some(vec![])` for streets *after* the root, which is correct against the library (upstream ignores `turn_donk_sizes` at a turn root, since `prev_action` is `None` there), but the interpretation is not recorded.
**Edit:** add it to the self-review deviations list: "a root-street `donk` of `None` is legal and is never sent to the library; only a later street's `None` is rejected".

**m2 — Task 10, step 3 (`precheck`): a `None` donk answers `ack{rejected}`, where §13.2's materialization row says `result{error{tree_mismatch}}`.**
> "None => return Err(format!(\"{s:?} donk option must be the explicit empty list, never None\"))"

§13.2 `protocol_rejections` also lists it as a "typed rejection with `reason`", so the spec is self-inconsistent; the plan silently picks one.
**Edit:** record the choice in the deviations list and cite both spec rows.

**m3 — Task 5, step 3: `bench` enables `engine`'s `testing` feature, so the fake clock/worker ship in release builds.**
> "engine = { path = \"../engine\", features = [\"testing\"] }"

Feature unification then enables `engine/testing` for the whole workspace build. `bench` only needs `engine::{tree, worker, deadline}`.
**Edit:** drop `features = ["testing"]` from `crates/bench/Cargo.toml`; task 22's runner uses no test double.

**m4 — Task 22, step 2 (`cancel_latency`): on a river spot the solve finishes in ~4 ms, so the cancel is issued after the terminal and both latencies record 0.**
> "std::thread::sleep(Duration::from_millis(50)); … worker.send(&EngineMessage::Cancel …)"

§13.5 requires a cancel-latency column in the report.
**Edit:** run `cancel_latency` against a flop spot (`flop_cancel` fixture shape) or send the cancel immediately after the first `progress{stage:"solving"}` rather than after a fixed 50 ms sleep; if the terminal arrives first, report `n/a` explicitly instead of `0`.

**m5 — Task 4, step 1: `tree_builder_golden` omits "duplicates merged", which §13.3 names.**
> "in-tree detection; insertion alongside menus; duplicates merged; `tree_signature` stability across chip scales"

**Edit:** add a case where an observed size rounds onto a menu size (e.g. `flop_fast_v1` at pot 100, observed `Bet(50)` — already present as in-tree — plus a case where the forced all-in and the added all-in collapse, e.g. `facing_test_v1` at eff 340) and assert `inserted.is_empty()` and no duplicate action in the node menu.

**m6 — Task 6, step 3: five `flop_full_v1` cases are materialized into a committed fixture although the template is phase-2 only (§10.1 "pre-solver phase 2 only").**
> "for t in TEMPLATES: for pot, eff in [(100, 100), (100, 150), (180, 910), (100, 149), (100, 151)]"

`flop_full_v1` at (180, 910) has roughly 3,000 skeleton nodes; the five cases add several MB to `fixtures/worker/materialization_cases.jsonl` and to task 8's cross-check runtime, for a template no phase-1 code path uses.
**Edit:** keep one `flop_full_v1` case (e.g. `(100, 100)`) for the rules check and drop the other four, or drop the template from `TEMPLATES` and add a single explicit case.

**m7 — Task 16, step 3 and Task 7, step 3: two produced items are never used.**
> "pub bench_p95_ms: HashMap<String, u64>" … "pub fn peak_working_set_bytes() -> u64"

`bench_p95_ms` is only ever read with `unwrap_or_else(street budget)` and nothing writes it; the worker's `win::peak_working_set_bytes` is never called (the engine measures the child's peak working set in `process.rs`).
**Edit:** either populate `bench_p95_ms` from `docs/bench/*.md` in task 22, or drop the field and the `unwrap_or_else` fallback; delete `win::peak_working_set_bytes` or use it to fill `StreetSolution.memory_bytes`.

**m8 — Task 19, step 2 (`final_from_solution`): a tree action outside `Derived.legal` is reported as `Unavailable::NotInMenu`.**
> "None => ActionAdvice { … unavailable: Some(Unavailable::NotInMenu) …"

§4.4 defines `NotInMenu` as "the action is not in the menu" (the source's menu), not "the amount is illegal at the real node". §8.4's legality-after-mapping rule uses `MovedProbability{from}` for that situation.
**Edit:** map an out-of-interval tree action to the nearest legal action per §8.4 and set `unavailable: MovedProbability { from }` on the destination, or add a note to `assumptions.notes` and keep `NotEvaluated`.

**m9 — Tasks 9 and 10 redefine the §4.5 limits instead of consuming plan 1's constants.**
> "pub const MAX_NODES: usize = 100_000;" … "pub const MAX_REQUEST_LINE: usize = 1 << 20;" … "pub const MAX_RESULT_LINE: usize = 16 << 20;"

Plan 1 already exports `REQUEST_LINE_MAX`, `RESULT_LINE_MAX`, `MAX_EXPORTED_NODES` from `proto::worker`, and `validate_solution` enforces the node cap with its own constant.
**Edit:** import the three constants from `proto::worker` in `extract.rs`, `protocol.rs` and `process.rs`; keep only re-exports.

**m10 — Task 21, step 1: `identity_race_golden` cannot reproduce §13.3's "hand B with the same displayed revision".**
> "assert!(id_b1.hand_id != id_a.hand_id && id_b2.decision_id > id_b1.decision_id);"

`IdentityState` draws revisions from a monotonic never-reused counter (§4.3), so hand B's revision is 9, not A's 7. The scenario the spec wants (a UI that shows the same revision number for two different hands) is unreachable by construction.
**Edit:** note in the task that the spec's "same displayed revision" is satisfied vacuously by the never-reused counter, and assert instead that an event carrying `(hand_id: A, hand_revision: 7)` is rejected while B's `(hand_id: B, hand_revision: 9)` is accepted.

**m11 — Task 5, step 3: the river/turn bench spots use R8 uniform ranges, not §13.5's chart-replay baseline set.**
> "// R8 addendum A.1 ranges (uniform weights); the chart-replay baseline replaces them in plan 3 (`--source file`)"

Recorded as deviation 8, correctly. But §13.5's gate is defined on the baseline (chart) set, so V2's turn p95 measured here does not satisfy the gate.
**Edit:** state in the task that the resulting `docs/bench/<date>-i7-13700K.md` section is a pre-baseline reference run and that plan 3 regenerates the suites and re-runs before V2/V22 are claimed.

**m12 — Task 6, step 3: `flop_best_so_far.jsonl` is generated but no test in the plan consumes it.**
> "best = [solve_line(\"45\", SPOT_FLOP, FLOP_BOARD, oop, ip, 180, 910, m[\"tree\"], m[\"history\"], 1, 2000, 600)]"

§13.0 requires the file, so generating it is right; but tasks 10-13 only use `river_two_combo`, `flop_cancel`, `lock_river`, `materialization_cases` and `basic_turn_std_request`.
**Edit:** use it in task 13's `deadline_best_so_far_bounded` as a second case (a flop spot that must return `best_so_far` with a measured exploitability), or note in task 6 that plan 4 consumes it.

**m13 — Task 11 records a correct spec correction that should be propagated.**
> "§13.2 `ev_convention_non_root_payoffs` names OOP's nut hand `AA` … three queens beat aces"

Verified: on `Qs Jd 7h 3c 2d`, OOP `QQ` removes every IP `QQ` combo, so the locked betting range is 54o at 20% (mass 0.6), equity 1, `EV(call) = +200`; `66` faces mass 3.6, equity 0.1667, `EV(call) = -50`. The plan's substitution reproduces the spec's numbers exactly.
**Edit:** open a spec change note so revision 6 of §13.2 reads `QQ` and `66`; no plan change needed.

**m14 — Task 21, step 2: `source_accuracy` is filled with a chip figure where §4.4 specifies a normalized one.**
> "assumptions.source_accuracy = out.solution.as_ref().map(|s| format!(\"exploitability <= {:.3} chips\", s.exploitability_chips))"

§4.4 gives `source_accuracy: String /* "unverified" | "exploitability <= x" */` and §7 compares in raw chips but displays in bp.
**Edit:** format as `format!("exploitability <= {} bp", reached_bp)` (or include both), so the UI string matches the spec's coverage vocabulary.

**m15 — Task 18, step 1: `facing_allin_golden` does not exercise §13.3's "hero's strategic range includes other hands and the headline uses AhAd".**
> "hero's strategic range is irrelevant here: only hero's actual combo enters"

True of the implementation, so the clause is vacuous — but the golden is the only place that documents it.
**Edit:** add one assertion that a non-trivial `hero_public` range passed alongside changes nothing in `AllInAnswer`, so the invariant is pinned rather than assumed.

---

## Dimension summary

**1. Spec coverage.** Complete for this plan's scope. All 18 §13.2 contract tests are present (tasks 7, 8, 10-14); all 6 in-scope §13.3 goldens are present (tasks 4, 17, 18, 19, 21). The §13.3 numbers are reproduced correctly: `facing_allin_golden`'s T1 (`W = 246`, `EV(call) = -11.5 / -12.75`, `+50 / +47.5`, `-2.30 bb`) is arithmetically exact; `coverage_classification_golden`'s three §10.2 projection cases are set up with the spec's exact stacks and labels (`MultiwayStreetRoot{1,0}`, `{1,50}`, `UnsupportedHistory`); `tree_materialization_matches_library`'s boundaries (150/151, 350/400/401, 340/341/240, stack 100, the turn/river cross-street operand) all reproduce upstream `push_actions` when hand-computed. Out-of-scope items (`experimental_surrogate_golden`, `deadline_best_so_far_labelling`, `flop_budget_setting_golden`, `replay_weights_golden`, `bet_translation_golden`) are correctly assigned to plans 3 and 4 and listed in the self-review.

**2. Spec deviations.** Nine are declared and each is justified; deviation 1 (the `AA` → `QQ,66` correction) is demonstrably right and should become a spec edit (m13). Three undeclared deviations: the root-street `donk: None` reading of §4.6/§13.2 (m1), `ack{rejected}` instead of `tree_mismatch` for a `None` donk (m2), and `source_accuracy` in chips (m14).

**3. Placeholders.** None. Every code step carries real code, every golden either ships in full (`coverage_classification_golden.json`) or has its hand-check values written out in the task. This dimension is clean.

**4. Correctness.** One compile-breaking error (B1) and six tests that cannot pass as written (M1, M2, M3, M4, M5, M6, M7) plus one wrong constant (M8). Everything I could check against the pinned library is right: `memory_usage() -> (uncompressed, compressed)` matches the `(f32_bytes, i16_bytes)` destructuring; `PostFlopGame::apply_history(&[usize])` matches `indices_for`'s index vector while `ActionTree::apply_history(&[Action])` matches `tree_build`'s use; `ActionTree::total_bet_amount()` returns per-player matched totals so `p + 2 * min(t)` is the correct terminal pot at fold, call and showdown; `add_line` inserts in sorted position and replaces a max-amount bet by `AllIn`, matching `map_observed`; `merge_bet_actions` at `merging_threshold = 0.0` merges nothing after dedupe; `BetSizeOptions`/`DonkSizeOptions` field names and `Default` are correct; `Range::{new, set_weight_by_cards, get_weight_by_cards, get_hands_weights}`, `compute_average`, `solve`, `solve_step`, `compute_exploitability`, `finalize`, `private_cards`, `strategy`, `expected_values_detail`, `lock_current_strategy` all exist with the assumed signatures. The `[1326][action]` transposition (`strat[a * n + h]`), the `available = weights > 0` rule, and the fold-row-zero identity are all consistent with §10.3.

**5. Task ordering.** Broken at task 1 (B2) and again after tasks 11 and 14 (M9). Otherwise the dependency order is sound: templates (2) → materializer (3) → effective tree (4) → bench CLI (5) → fixtures (6) → worker (7-13) → engine link (14) → doubles (15) → solve client (16) → classifier/equity/assembly/log (17-20) → Engine (21) → bench run (22). Task 20's retroactive `EngineCore::new` signature change is called out and the affected rig is named.

**6. Interfaces.** The Produces/Consumes blocks are internally consistent (I traced `Templates`, `materialize`, `TreeBuild`, `WorkerLink`, `FakeReply`, `SharedSink`, `SolvePlan`/`SolveOutcome`, `assemble::*`, `job::run` across every task and found no mismatch). The failure is at the plan-1 boundary (B3): six items differ or are missing, one of them structurally. `register_snapshot` is correctly deferred to plan 3 with `SnapshotStore::register` carrying the same identity rule; `Engine::{new, set_config, begin_hand, apply_action, set_board, undo, recommend, cancel, finish_hand, abandon_hand, shutdown}` matches §3.5 (only `set_hero_cards` and `set_seat_tag` are absent, which belong to plan 5's Tauri surface).

**7. YAGNI.** Mostly disciplined. Three small overreaches: five `flop_full_v1` fixture cases for a phase-2 template (m6), two produced-but-unused items (m7), and `bench` pulling in the `testing` feature (m3). `SnapshotStore` and `RangeSource`/`ExplicitRanges` are scaffolding for plans 3-4 but are load-bearing for `identity_race_golden` and for solving at all, so they are in scope.
