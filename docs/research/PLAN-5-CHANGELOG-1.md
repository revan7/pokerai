# Changelog: revision 1 of Plan 5 (Tauri UI + E2E)

Plan revised: `docs/superpowers/plans/2026-09-10-plan-5-ui.md`.
Inputs: `docs/research/REVIEW-of-plan-5.md` (6 BLOCKER, 12 MAJOR, 15 MINOR) and `docs/research/REVIEW-cross-plan.md` §§1, 2, 4, 5.
Result: 2,708 lines / 12 tasks -> 3,153 lines / 14 tasks (Task 2 split into Tasks 2–4 per the cross-plan review's §5 split table; every task from the old Task 3 onward renumbered +2).

Convention: **APPLIED** = plan text/code changed. **NOT APPLIED** = no plan 5 change was needed or made, with the reason (most commonly: the cross-plan review's resolution fixes Plan 2 instead, and Plan 5's original text already matched the resolved shape).

---

## 1. BLOCKERS (REVIEW-of-plan-5.md §2)

| ID | Disposition | Detail |
|---|---|---|
| BL-1 `Engine::set_config` return type | **NOT APPLIED** | The review's own suggested fix (drop to plain `u32`) is superseded by cross-plan review M10/S15, which fixes **Plan 2** to return `Result<u32, EngineError>` instead — matching what Plan 5's façade already assumed. No Plan 5 code changed; the façade block now cites M10/S15 explicitly and lists this as a "Required from plan 2" item with no Task 2/3/4 code changes. |
| BL-2 `Engine::shutdown` receiver | **NOT APPLIED** | Same pattern: cross-plan M11 fixes **Plan 2** to use `shutdown(&mut self)` with an internal idempotent flag, matching Plan 5's original `EngineAdapter::shutdown(&mut self) { self.engine.shutdown(); ... }`. No `Option<Engine>` restructuring was needed. |
| BL-3 `Engine::set_hero_cards` missing | **APPLIED** | Plan 5's façade already declared the correct signature; cross-plan Or3 confirms it is assigned to Plan 2 Task 21. Added an explicit "Required from plan 2" block naming `Engine::set_hero_cards(&mut self, cards: [Card; 2]) -> Result<HandState, EngineError>` so the orchestrator can hand it to Plan 2's fix, per the task brief. |
| BL-4 `engine::Paths` field count | **NOT APPLIED** | The review's suggested fix (rename to `worker_exe`/`preflop_dir`/`cache_dir`/`log_dir`, matching Plan 2's un-fixed 2-field struct) is superseded by cross-plan M13/Or5, which fixes **Plan 2** to produce the four-field shape (`worker`, `preflop`, `cache`, `log`) Plan 5 already assumed. No Plan 5 code changed. |
| BL-5 `proto::BeginHand -> core_model::BeginHand` conversion missing | **APPLIED (with a scope change directed by the orchestrator)** | Cross-plan M6/Or4 reassigns DTO *ownership* to Plan 1 Task 3 and suggests Plan 2's `Engine::begin_hand` accept `proto::BeginHand` directly (engine-side conversion). Per the orchestrator's explicit decision 1 ("write the `proto::BeginHand -> core_model::BeginHand` conversion code"), Plan 5 instead keeps `Engine::begin_hand` on its **original** Plan 2 signature (`core_model::BeginHand`) and writes the conversion in the shell (new Task 4). Plan 5 Task 1 no longer defines `proto::BeginHand` itself (Or4 moved that to Plan 1); it only adds the `ts-rs` derive to the type Plan 1 now produces. `core-model` was added as a direct `pokerai-app` dependency for the one struct literal. |
| BL-6 no startup-diagnostics interface | **APPLIED** | Added to the façade: `Engine::new(GameConfig, Paths) -> Result<(Engine, Vec<String>), EngineError>` (was `Result<Engine, EngineError>`). Task 5's `lifecycle::startup` now destructures `(engine, engine_warnings)` and extends the existing `warnings` vector, so the bootstrap banner path is no longer dead code. |

## 2. MAJORS (REVIEW-of-plan-5.md §3)

| ID | Disposition | Where |
|---|---|---|
| MA-1 `presolverLines` doesn't type-check | **APPLIED** | Task 8 Step 3: `num()` returns `number\|null` explicitly; array fields narrowed through an explicit `unknown[]` local before indexing. |
| MA-2 `reasons.map` union-type error | **APPLIED** | Task 11 Step 3: `reasons` is now explicitly typed `Array<ApproxReason\|UnsupportedReason>`; both types added to the `import type` list. |
| MA-3 `Object.fromEntries` resolves to `any` | **APPLIED** | Task 10 Step 4: `.map(s=>[...] as const)` plus an explicit `StackDrafts` annotation on `defaults`. |
| MA-4 `event.detail` is `any` | **APPLIED** | Task 12 Step 3: `instanceof CustomEvent` guard, `detail: unknown` local, then narrowed before use in the state updater. |
| MA-5 duplicate `$` import in the E2E spec | **APPLIED** | Task 14 Step 1: `import {browser, $, $$, expect} from '@wdio/globals';`. |
| MA-6 WebDriver `application` path off by one | **APPLIED** | Task 14 Step 3: three `..` segments instead of four; prose corrected from "four directories above" to "three." |
| MA-7 `#[ts(type=...)]` used as a container attribute | **APPLIED** | Task 1 Step 4: field-level placement on `Card`/`Range1326`; `MenuSize` (post-M1-rename) gets a hand-written `impl TS` (`name`/`inline`/`output_path`) instead of a derive attribute, with a fallback instruction if `decl`/`ident` don't compose as expected. |
| MA-8 mid-hand `set_game_config` breaks every subsequent `recommend` | **APPLIED** | Task 7 Step 3: `config_revision` dropped from `Recommendations.request`'s admission cross-check (kept on the five-field *event* identity check). Task 12 Step 5 adds the regression test `mid_hand_config_save_does_not_break_the_next_recommend`. |
| MA-9 `begin_hand_stack_confirmation` doesn't test the confirmation rule | **APPLIED** | The spec-§13.4-named test `begin_hand_stack_confirmation` moved to Task 10 (where `EntryController` exists) and now drives the real controller, asserting `fake.calls` has zero `begin_hand` entries until every stack is confirmed. Task 9 keeps a differently-named pure-reducer test (`wizard_defers_begin_until_every_stack_is_confirmed`) as the "second test" the review asked to retain. |
| MA-10 six verify-steps with no test code | **APPLIED (all six)** | Task 5 Step 7 (overwrite/blocked-parent persistence + held-request double-stop); Task 8 Step 5 (`TimeCharge`/five-seat/below-2×BB/`2.501`→`2501` matrix); Task 10 Step 6 (bet bounds/Escape/Check-before-Call/all-in `to`/Ctrl+Z/repeat-key/native-input matrix); Task 11 Step 7 (`HeroComboOutOfSupport`/`DeadlineBestSoFar`); Task 12 Step 5 (NoDecision table + identity-matched server NoDecision + Ready-survives-Pending + MA-8 regression); Task 14 Step 5 (decision-log cross-check against the DOM's `data-decision-id`). |
| MA-11 `bundle.active:false` leaves resources unstaged | **APPLIED** | Task 5 Step 6: `stage-runtime.ps1` now also copies `runtime/` next to both `debug` and `release` profile output directories (pre-created before Cargo runs, via `beforeDevCommand`/`beforeBuildCommand`), and states explicitly that `bundle.active` stays `false` for all of phase 1. Task 14's `run-e2e.ps1` no longer duplicates the copy logic — it only asserts the exe and staged `runtime/solver-worker.exe` exist. |
| MA-12 `presolver_status/pause/resume` signatures undeclared | **APPLIED** | Added to the "Required from plan 2" block: `presolver_status(&self) -> cache::presolver::PresolverStatus`, `presolver_pause(&mut self)`, `presolver_resume(&mut self)`, with the note that `&self` is sufficient since `EnginePort::dispatch` already holds `&mut Engine`. |

## 3. MINORS (REVIEW-of-plan-5.md §4)

All 15 applied (all were judged trivial — a type/text fix, a code comment, or a small deletion):

| ID | Applied where |
|---|---|
| MI-1 | Task 1 Step 3: dropped the redundant `tsconfig.json` `"types"` entries. |
| MI-2 | Task 1 Step 4: added the `[[example]]` block to the `Cargo.toml` additions snippet. |
| MI-3 | Task 1 Step 4: replaced the open-ended instruction with "complete as of this date; the witness test enforces it." |
| MI-4 | Task 1 Step 6: added the `dist/index.html` placeholder fallback instruction. |
| MI-5 | Task 3 Step 3: added the "expand the macro by hand" fallback note. |
| MI-6 | Task 4 Step 3: comment on `MAX_SAFE_ID`/`safe_identity` noting it is a boundary assertion, unreachable given Plan 2's monotonic counters. |
| MI-7 | Task 4 Step 3: comment on `ChannelSink::emit`'s silent drop being unreachable in practice, tied to MI-6. |
| MI-8 | Task 7 Step 3: comment on `estimate()` only guarding Ready against Pending (not Unavailable), noting this is literal-correct per spec §4.4. |
| MI-9 | Task 7 Step 3: comment recording the trailing-edge (vs. leading-edge) progress-coalescing choice. |
| MI-10 | Task 9 Step 3: comment on `prefill`'s `start - (start - remaining)` form matching spec §4.3's literal wording. |
| MI-11 | Task 10 Step 4: deleted the dead `case 'H':` switch arm; comment explains why it was unreachable. |
| MI-12 | Already flagged in the surrounding prose ("cap the local queue at 64 keys and show overflow"); left as is — judged adequately explained already. |
| MI-13 | Task 11 Step 3: comment on `headline()`'s `coverage.kind==='Unsupported'` short-circuit being an added (correct) rule, not literally one of spec §4.4's three. |
| MI-14 | Task 11 Step 3: added `<p data-testid="legal-intervals">Legal: {r.legal.map(describe).join(' · ')}</p>` rendering the recommendation's own legal intervals. |
| MI-15 | Task 14 Step 1: named `webdriverio@9.30.0`/`@wdio/types@9.29.1` as the known-good fallback pair if `--save-exact @wdio/cli@9 ...` resolves a conflicting newer 9.x. |

## 4. Cross-plan review items affecting Plan 5 (REVIEW-cross-plan.md §§1, 2, 4, 5)

| ID | Disposition | Detail |
|---|---|---|
| M1 (`Menu`/`RaiseSize` -> `SideMenu`/`MenuSize`) | **APPLIED** | Task 1's ts-rs registry now lists `crate::SideMenu, crate::MenuSize` (was `crate::Menu, crate::RaiseSize`); the hand-written `TS` impl follows the rename (see MA-7). |
| M4 (`ReadyInfo` -> `Ready`) | **APPLIED** | Task 1's registry now lists `crate::worker::Ready` (was `crate::worker::ReadyInfo`); this was the "one registry entry" the cross-plan review named for Plan 5 Task 1 Step 3. |
| M6 (`core_model::BeginHand` field set / DTO ownership) | **APPLIED, scope adjusted by orchestrator** | See BL-5 above. |
| M10 (`Engine::set_config` return type) | **Informational — no Plan 5 code change** | See BL-1. |
| M11 (`Engine::shutdown` receiver) | **Informational — no Plan 5 code change** | See BL-2. |
| M12 (`recommend` sink type, `Box<dyn EventSink>` vs `+ Send`) | **APPLIED** | Façade and Task 4's `EngineAdapter::dispatch` `Op::Recommend` arm now use `Box<dyn EventSink>` with no `+ Send`; `trait EventSink: Send` carries the bound as a supertrait. This is explicitly a Plan-5-side fix per the cross-plan review's own "C (plan 5)" tag. |
| M13 (`engine::Paths` layout) | **Informational — no Plan 5 code change** | See BL-4. |
| M14 (`Engine::set_hero_cards` producer) | **Informational — no Plan 5 code change** | See BL-3. |
| Or3 (`Engine::set_hero_cards` assignment) | **Reflected in the "Required from plan 2" note** | |
| Or4 (`proto::BeginHand` DTO assignment to Plan 1 Task 3) | **APPLIED** | Task 1 Step 4 no longer defines the struct, only adds the derive. |
| Or5 (`Paths.preflop`/`.cache` assignment) | **Reflected in the façade comment** | No Plan 5 code change; Plan 3 populates `preflop`, Plan 4 reads `cache`. |
| Or7 (`PresolverStatus: Serialize` assignment to Plan 4 Task 14) | **Reflected in the façade comment** | Task 3's `presolver_status()` JSON projection is unaffected either way (already hand-built). |
| S15 (spec §3.5 `engine` row edit) | **Reflected** | Matches the façade's `set_config`/`set_hero_cards`/`shutdown` shapes. |
| S16 (spec §4.3 `BeginHand` DTO) | **Superseded by Or4's reassignment** | The DTO itself is now Plan 1's to define; spec §4.3 already documents it there per M6's fix text. |
| §5 split table, "P5.T2 ... split before execution" | **APPLIED** | Task 2 (447 lines) split into Task 2 (`Service`/`Op`/`EnginePort` + dispatcher tests), Task 3 (the 14 commands + argument tests), Task 4 (`Channel<RecommendationEvent>` forwarding + the real engine adapter). All later tasks renumbered +2. |

Cross-plan items **not** applying to Plan 5 (M2, M3, M5, M7–M9, M15–M21; Or1, Or2, Or6, Or8; most of §3's other S-items; the deferred/rejected lists) were reviewed and confirmed out of scope — they modify Plans 1–4 internally and do not change Plan 5's produced or consumed interfaces.

---

## 5. Required from Plan 2 (exact signatures, for the orchestrator to hand to Plan 2's fix)

```rust
Engine::new(config: GameConfig, paths: Paths) -> Result<(Engine, Vec<String>), EngineError>;
Engine::set_config(&mut self, config: GameConfig) -> Result<u32, EngineError>;
Engine::shutdown(&mut self);
Engine::set_hero_cards(&mut self, cards: [Card; 2]) -> Result<HandState, EngineError>;
Engine::presolver_status(&self) -> cache::presolver::PresolverStatus;
Engine::presolver_pause(&mut self);
Engine::presolver_resume(&mut self);
Paths { worker: PathBuf, preflop: PathBuf, cache: PathBuf, log: PathBuf }
```

Unchanged from Plan 2's already-produced API (no edit needed, cited so Plan 2's author can confirm nothing else moved):

```rust
Engine::begin_hand(&mut self, begin: core_model::BeginHand) -> Result<HandState, EngineError>;
Engine::apply_action(&mut self, action: Action) -> Result<HandState, EngineError>;
Engine::set_board(&mut self, board: &[Card]) -> Result<HandState, EngineError>;
Engine::undo(&mut self) -> Result<HandState, EngineError>;
Engine::recommend(&mut self, sink: Box<dyn EventSink>) -> Result<DecisionIdentity, EngineError>;
Engine::cancel(&mut self, decision_id: u64);
Engine::finish_hand(&mut self);
Engine::abandon_hand(&mut self);
trait EventSink: Send { fn emit(&mut self, event: RecommendationEvent); }
```

## 6. Result

- 2,708 -> 3,153 lines; 12 -> 14 tasks; 12 -> 14 commits (one per task, unchanged ratio).
- All 6 BLOCKERs, all 12 MAJORs, and all 15 MINORs dispositioned (3 BLOCKERs applied with code/doc changes, 3 BLOCKERs found already correct once the cross-plan resolution is read onto Plan 2 instead of Plan 5).
- Every cross-plan review item naming Plan 5 applied or explicitly reflected.
- No spec-§13.4-named test dropped; `begin_hand_stack_confirmation` moved from Task 9 to Task 10 (where `EntryController` exists) to make it meaningful, per MA-9.
