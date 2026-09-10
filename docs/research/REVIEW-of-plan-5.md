# Review of Plan 5 (Tauri UI + E2E), 2026-09-10

Plan reviewed: `docs/superpowers/plans/2026-09-10-plan-5-ui.md` (2,708 lines).
Contract: spec revision 5 `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md`; outline §0b; BRIEF series map; writing-plans SKILL.md.
Cross-checked against the concrete declarations in Plans 1–4 and against R5.
External facts re-verified on 2026-09-10 (npm registry, crates.io, docs.rs for ts-rs 12.0.1 and tauri 2.11.5).

**Verdict: NOT READY.** 6 BLOCKER, 12 MAJOR, 15 MINOR.

The frontend half of this plan is unusually strong: all nine §13.4 test names are present, the keystroke trace in Task 10 was hand-simulated end to end and is internally consistent, and the serde witnesses in Task 1 match Plan 1's actual attributes. The failure is concentrated in the **engine façade**: the plan's "Required engine surface" block (lines 53–78) does not match the `Engine` API that Plan 2 actually produces, and the plan's own "Remaining integration gap" section (line 2708) admits this without resolving it. Task 2 and Task 3 cannot compile as written.

---

## 1. Verified external facts (no findings; recorded so the executor does not re-verify)

| Item | Result |
|---|---|
| `tauri` 2.11.5, `tauri-build` 2.6.3, `ts-rs` 12.0.1, `tauri-driver` 2.0.6 | all exist on crates.io at exactly those versions |
| `@tauri-apps/api` 2.11.1, `@tauri-apps/cli` 2.11.4 | exist; both are current `latest` |
| `@wdio/tauri-service` 1.3.0 | exists (latest 1.4.0). The unscoped `wdio-tauri-service` **404s** — the plan's warning at line 47 is correct |
| vite 7.3.6, vitest 4.1.11, react/react-dom 18.3.1, typescript 5.9.3, jsdom 26.1.0, all eslint/testing-library/@types pins | all exist |
| ts-rs 12.0.1 `TS` trait | `name(&Config)` and `inline(&Config)` are required; `ident`, `decl`, `output_path() -> Option<PathBuf>` (no config arg), `visit_dependencies(&mut impl TypeVisitor)` are provided. `Config::new()`, `with_large_int(impl Into<String>)` exist. `serde-compat` is the sole default feature. **`Config` is not documented as `Clone`** — the plan correctly passes `&r.cfg` everywhere |
| `tauri::test` | `INVOKE_KEY`, `mock_builder`, `mock_context`, `noop_assets`, `get_ipc_response`, `MockRuntime` all exist |
| `tauri::webview::InvokeRequest` | all seven fields exist with the types the plan uses (`invoke_key: String`, `headers: HeaderMap`) |
| `WebviewWindowBuilder` | `on_page_load(Fn(WebviewWindow<R>, PageLoadPayload))`, `devtools`, `initialization_script`, `min_inner_size`, `inner_size`, `title` all exist |
| `WebviewWindow::eval(&self, impl Into<String>) -> Result<()>` | exists |

One correction that the plan gets *right by accident*: `tauri::ipc::Channel::new` takes **`Fn`**, not `FnMut`. Task 2's streaming test uses a `Mutex`-based closure, so it satisfies `Fn + Send + Sync` ✓. `Channel` implements `Clone` and `Serialize` but **not** `Deserialize`; the plan only ever takes it as a command argument (via `CommandArg`), which is correct.

---

## 2. BLOCKERS

### BL-1 — `Engine::set_config` returns `u32`, not `Result<u32, EngineError>`
**Task 2, Step 5** (`service.rs` additions, `Op::Config` arm):
> "`c.config_revision = e.set_config(c.clone()).map_err(engine_error)?;`"

Plan 2 (`2026-09-10-plan-2-worker-engine.md`, `crates/engine/src/engine.rs`) declares:
```rust
pub fn set_config(&mut self, cfg: GameConfig) -> u32
```
`.map_err` on a `u32` is E0599. The plan's façade block at line 56 asserts `-> Result<u32, EngineError>` — that is not the produced API.

**Edit:** change the façade block line 56 to `Engine::set_config(&mut self, config: GameConfig) -> u32;` and the Task 2 arm to
```rust
Op::Config(mut c) => { c.config_revision = e.set_config(c.clone()); (self.accepted_config)(c.clone()); encode(c) }
```
Note the residual subtlety: `set_config` takes `c` before its revision is stamped, so `accepted_config` must receive the *stamped* copy (as written) — keep that ordering.

### BL-2 — `Engine::shutdown` consumes `self`
**Task 2, Step 5** (`impl EnginePort for EngineAdapter::shutdown`):
> "`fn shutdown(&mut self) { self.engine.shutdown(); ... }`"

Plan 2 declares `pub fn shutdown(mut self)` (by value). Calling it through `&mut self` is E0507 (cannot move out of `self.engine`). This is not a signature the shell can work around locally: `EngineAdapter` owns the `Engine` behind a `Box<dyn EnginePort>` and `EnginePort::shutdown` takes `&mut self`.

**Edit:** pick one and record it in the façade block: either (a) change `EnginePort::shutdown` to `fn shutdown(self: Box<Self>)` and have `Service::spawn`'s thread call `port.shutdown()` after the drain loop (this requires `port` to be moved, so restructure the loop), or (b) hold `engine: Option<engine::Engine>` in `EngineAdapter` and `if let Some(e) = self.engine.take() { e.shutdown(); }`. (b) is the smaller change and keeps the trait as written. Add the chosen form to the reconciliation list at line 51.

### BL-3 — `Engine::set_hero_cards` does not exist anywhere in Plans 2–4
**Plan header, line 58** and **Task 2, Step 5** (`Op::Hero(c) => encode_hand(e.set_hero_cards(c)...)`).

Spec §3.5's `pokerai-app` row lists `set_hero_cards` as a Tauri command, but the spec's `engine` row does **not** list it, and Plan 2's produced surface (line 4985) is: `new, with_core, set_config, begin_hand, apply_action, set_board, undo, set_explicit_ranges, recommend, cancel, finish_hand, abandon_hand, shutdown, state`. `core_model::set_hero_cards` exists (Plan 1, `core-model/src/lib.rs` re-exports it) but the engine never wraps it — and it must be wrapped, because the wrap is what assigns a fresh `hand_revision` and invalidates descendant identities (spec §5 step 3). Plan 5's "Upstream contract gate" (line 51) lists receiver/return types, `Paths` and `EventSink` as things to reconcile; it does **not** flag the missing method.

**Edit:** add an explicit line to the contract gate: "`Engine::set_hero_cards(&mut self, cards: [Card; 2]) -> Result<HandState, EngineError>` is not produced by Plan 2; it must be added to `crates/engine/src/engine.rs` as a `mutate()`-wrapped call to `core_model::set_hero_cards`, before Task 2." Without this the `set_hero_cards` command, the `H` key, `entry_flow_full_hand_keystrokes` and the E2E all fail.

### BL-4 — `engine::Paths` has two fields, not four
**Task 3, Step 4** (`lifecycle.rs`):
> "`engine::Paths { worker: resource.join("runtime/solver-worker.exe"), preflop: ..., cache: local.join("cache/v3"), log: local.join("decisions.jsonl") }`"

Plan 2 declares `pub struct Paths { pub log_dir: PathBuf, pub worker_exe: PathBuf }` (line 5321) — note `log_dir` is a *directory* (the log crate rotates inside it), not a file path. Plan 3 says only "add a packaged-chart fallback directory to its existing `Paths` initializer if not already present" (line 1657) without naming the field. Plan 4 never touches `Paths`. So `preflop` and `cache` do not exist, `worker` is `worker_exe`, and `log` is `log_dir` with different semantics.

**Edit:** replace the `Paths` line in the façade block (line 74) with the real Plan 2 shape, and add to the contract gate the exact two fields Plan 5 needs added upstream, with their kind: `preflop_dir: PathBuf` (directory of `<name>.json` + `<name>.manifest.json` siblings — Plan 3 line 369 confirms that layout, so `stage-runtime.ps1` is correct) and `cache_dir: PathBuf`. Then change `lifecycle::paths` to
```rust
engine::Paths { worker_exe: resource.join("runtime/solver-worker.exe"),
                preflop_dir: resource.join("runtime/preflop"),
                cache_dir: local.join("cache/v3"),
                log_dir: local.clone() }
```
(`decisions.jsonl` is created by the engine inside `log_dir` per spec §5 step 10.)

### BL-5 — the `proto::BeginHand` → `core_model::BeginHand` conversion is described but never written
**Line 51** promises "the engine façade assigns the ID and converts to its core-model input", but **Task 2, Step 5** dispatches straight through:
> "`Op::Begin(b) => encode_hand(e.begin_hand(b).map_err(engine_error)?),`"

`Engine::begin_hand` takes `core_model::BeginHand { hand_id: u64, button, hero, dealt, stacks_start: Vec<u32>, hero_cards }` (Plan 1 line 2365). The new `proto::BeginHand` has no `hand_id` and names the field `stacks`. There is no conversion anywhere in the plan, and no task changes `Engine::begin_hand` to accept the ID-free DTO. (Plan 2's `Engine::begin_hand` ignores the caller's `hand_id` — it overwrites it in `stamp()` — so a `hand_id: 0` placeholder is safe, but that decision must be written down.)

**Edit:** either (a) add to Task 2 Step 5, in the `Op::Begin` arm:
```rust
Op::Begin(b) => {
    // Engine::begin_hand overwrites hand_id from its own counter (Plan 2 `stamp`);
    // the shell never allocates one.
    let core = core_model::BeginHand { hand_id: 0, button: b.button, hero: b.hero,
        dealt: b.dealt, stacks_start: b.stacks, hero_cards: b.hero_cards };
    encode_hand(e.begin_hand(core).map_err(engine_error)?)
}
```
and add `core-model = { path = "../../../crates/core-model" }` to `src-tauri/Cargo.toml` (allowed: `core-*` sits below `pokerai-app` in spec §3.2's dependency order); or (b) put the conversion in the engine by changing `Engine::begin_hand` to take `proto::BeginHand`, which is cleaner but is a Plan 2 change. Pick one in the contract gate. Also rename the DTO to `BeginHandRequest` — two public `BeginHand` types in the same workspace is a trap for exactly the executor this plan is written for (Plan 2's `engine.rs` already has `use core_model::{..., BeginHand}; use proto::*;`, where the explicit import silently wins over the glob).

### BL-6 — no interface exists for the startup warnings §12 and §3.7 require
**Task 3, Step 5a**, prose after the code:
> "Route startup bundle warnings through the engine's completed diagnostics surface into the same `warnings` vector at the façade reconciliation gate; no shell parser or second chart store."

There is no such surface. `Engine::new` returns `Result<Engine, EngineError>` — success carries nothing. Spec §12 requires "Preflop bundle failing hash, schema or content validation | Quarantined; **startup banner**; remaining bundles active", and §3.7 requires a startup banner when the CPU lacks AVX2. Plan 3's `PreflopStore::open` does return `(store, warnings)` (Plan 4 line ~1476 uses `let (store,warnings)=core_preflop::PreflopStore::open(...)`), but the engine swallows them. The shell's `warnings: Vec<String>` is therefore always empty in production, and the bootstrap banner path is dead code.

**Edit:** add to the contract gate a required upstream signature, e.g. `Engine::new(GameConfig, Paths) -> Result<(Engine, Vec<String>), EngineError>` **or** `Engine::startup_warnings(&self) -> Vec<String>`, and consume it in `lifecycle::startup` where `warnings.push(...)` already is. Until that exists, `presolver_controls_and_bootstrap_warnings_render` (Task 10 Step 5) only tests a prop that production never populates.

---

## 3. MAJOR

### MA-1 — `presolverLines` does not type-check and does not lint
**Task 6, Step 3** (`src/state/session.ts`):
> "`const number=(key:string)=>typeof data[key]==='number'&&Number.isFinite(data[key])?data[key]:null;`"

`data` is `Record<string, unknown>` and `key` is `string` (not a literal type), so TypeScript performs **no** narrowing on an element access with a non-literal index. `number()` returns `unknown`, and the next lines do `(eta/3600).toFixed(1)` and `p50.toFixed(1)` — TS2571 "Object is of type 'unknown'" ×2. Separately, `Array.isArray(data.scenario_hits)` narrows to `any[]`, so `for(const row of data.scenario_hits)` binds `row: any` and `row[0]`/`row[1]`/`row[2]` are `any` — `@typescript-eslint/no-unsafe-assignment` and `no-unsafe-argument` fire under `recommendedTypeChecked`. Same for `data.tier_done[i]`. `npm run check` (a required per-task gate) fails.

**Edit:**
```ts
const num=(key:string):number|null=>{const v=data[key];return typeof v==='number'&&Number.isFinite(v)?v:null;};
```
and for the arrays, narrow into a typed local before use:
```ts
const tiers=(key:string):unknown[]=>Array.isArray(data[key])?data[key] as unknown[]:[];
const hits=tiers('scenario_hits');
for(const row of hits){ if(!Array.isArray(row))continue; const r=row as unknown[];
  const name=r[0],hit=r[1],total=r[2];
  if(typeof name==='string'&&typeof hit==='number'&&typeof total==='number') output.push(...); }
```

### MA-2 — `reasons.map(...)` is a call on a union of array types (TS2349)
**Task 9, Step 3** (`RecommendationPanel`):
> "`const reasons=!r||r.coverage.kind==='Exact'?[]:r.coverage.kind==='Approximate'?r.coverage.reasons:[r.coverage.reason,...r.coverage.partial];`"

The conditional's type is `never[] | ApproxReason[] | (UnsupportedReason|ApproxReason)[]`. Calling the generic `.map` on a union of three array types is rejected ("Each member of the union type … has signatures, but none of those signatures are compatible with each other"), because TypeScript's union-signature rule permits at most one generic member.

**Edit:** annotate the binding:
```ts
const reasons: Array<ApproxReason|UnsupportedReason> = !r || r.coverage.kind==='Exact' ? []
  : r.coverage.kind==='Approximate' ? r.coverage.reasons
  : [r.coverage.reason, ...r.coverage.partial];
```
and add `ApproxReason, UnsupportedReason` to the `import type` list at the top of `Recommendation.tsx` (currently only `Recommendation, Action`).

### MA-3 — `Object.fromEntries(... .map(s => [s.seat, x]))` resolves to `any`
**Task 8, Step 4** (`EntryController.handle`, case `'N'`):
> "`const defaults={...Object.fromEntries(this.config.seats.map(s=>[s.seat,100*this.config.bb_chips])),...this.stacks};`"

`.map(s=>[a,b])` infers `number[][]`, which misses the tuple overload and falls through to `fromEntries(entries: Iterable<readonly any[]>): any`. `defaults` is `any`; spreading it and passing it to `prefill` trips `no-unsafe-assignment` / `no-unsafe-argument`. `npm run check` fails.

**Edit:** `this.config.seats.map(s=>[s.seat,100*this.config.bb_chips] as const)` and annotate `const defaults: StackDrafts = {...}`.

### MA-4 — `event.detail` is `any` in the warning listener
**Task 10, Step 3** (`SessionApp`):
> "`if(event instanceof CustomEvent&&typeof event.detail==='string')setNotices(old=>[...old,event.detail]);`"

`CustomEvent` without a type argument is `CustomEvent<any>`; the `typeof` narrowing on a *property* is discarded inside the nested `old => ...` arrow, so `event.detail` there is `any` → `no-unsafe-assignment`. Same class of failure as MA-3.

**Edit:**
```ts
const listener=(event:Event)=>{
  if(!(event instanceof CustomEvent))return;
  const detail:unknown=event.detail;
  if(typeof detail==='string')setNotices(old=>[...old,detail]);
};
```

### MA-5 — the E2E spec file imports `$` twice and never imports `$$`
**Task 12, Step 1** (`e2e/full-hand.e2e.ts`), first line:
> "`import {browser,$,$,expect} from '@wdio/globals';`"

A duplicate binding in one import clause is a syntax error (`Identifier '$' has already been declared`), so the file never parses. Line 2607 then uses `$$` (`const evs=await $$('[data-testid="main-ev"]')`), which is not imported at all.

**Edit:** `import {browser, $, $$, expect} from '@wdio/globals';`

### MA-6 — the WebDriver `application` path resolves one directory above the repo root
**Task 12, Step 3** (`e2e/wdio.conf.ts`):
> "`application: fileURLToPath(new URL('../../../../target/x86_64-pc-windows-msvc/release/pokerai-app.exe', import.meta.url))`"

`new URL` resolves relative to the *directory* of the base URL. From `apps/pokerai-ui/e2e/`, four `..` segments give `D:/Documents/Projects/` — one level above the repo root. The plan's own note ("four directories above `e2e/wdio.conf.ts`") miscounts by treating the file name as a directory level. Three are needed.

**Edit:** `'../../../target/x86_64-pc-windows-msvc/release/pokerai-app.exe'`. Keep the plan's instruction to assert the resolved absolute path exists before `wdio run` (the `run-e2e.ps1` script already checks the exe with its own independently computed path, so the mismatch would otherwise only surface as an opaque driver failure).

### MA-7 — `#[ts(type = "...")]` is used as a container attribute, which ts-rs does not support
**Task 1, Step 4**, prose after the exporter code:
> "on the original `Card`, use `#[cfg_attr(feature = "typescript", ts(type = "string"))]`; on `Range1326`, use `ts(type = "Array<number>")`; on Plan 1's manually serialized `RaiseSize`, use `ts(type = r#"number | "a""#)`."

ts-rs documents `#[ts(type = "..")]` as a **field** attribute; the container-level override is `#[ts(as = "..")]`. As written on the type, the derive rejects the attribute. `Card` and `Range1326` are newtype structs (`Card(pub u8)`, `Range1326(pub [f32; COMBOS])`), so the field form works; `RaiseSize` is an enum with a hand-written `Serialize`/`Deserialize` and has no field to hang it on.

**Edit:** specify the exact placement, e.g.
```rust
pub struct Card(#[cfg_attr(feature="typescript", ts(type = "string"))] pub u8);
```
and for `RaiseSize` use the container form `#[cfg_attr(feature="typescript", ts(as = "RaiseSizeTs"))]` with a private `#[derive(TS)] enum/alias`, or implement `TS` for `RaiseSize` by hand (three methods: `name`, `inline`, plus `output_path`). Also note that `Range1326`'s override is probably unnecessary: ts-rs's `array_tuple_limit` defaults to 64, so `[f32; 1326]` already emits `Array<number>`; keeping the override is harmless if placed on the field, but the plan should say which.

### MA-8 — a mid-hand `set_game_config` makes every subsequent `recommend` fail
**Task 5, Step 3** (`Recommendations.request`) + **Task 10, Step 3** (`save`):
> "`if(id.hand_id!==hand.hand_id || id.hand_revision!==hand.hand_revision || id.config_revision!==hand.config.config_revision) { await this.backend.cancel(id.decision_id); throw new Error('Admission identity does not match the active hand'); }`"

Spec §4.2/§12: a session `set_config` bumps `config_revision` but the active hand keeps its frozen `HandConfig.config_revision`. Plan 2's `IdentityState::next_decision()` stamps the **session** `config_revision` into the identity. So after any mid-hand save, `id.config_revision` is `n+1` while `hand.config.config_revision` is `n`, and the admission check throws for the rest of the hand — including the `save` handler's own `if(h&&decisionReason(h)===null)void rec.request(h)`, which produces a red banner immediately on saving settings during a hand. This directly contradicts the plan's own global constraint ("a session `set_config` takes effect from the next hand").

**Edit:** drop `config_revision` from the admission cross-check (the five-field `sameIdentity` check on *events* already guarantees the UI only renders events of the admitted decision), keep the `hand_id`/`hand_revision` comparison, and add a regression test: save a new config mid-hand, press Space, assert a `Final` renders and no `role="alert"` appears. Alternatively record the engine-side rule that the identity carries the hand's frozen revision — but that is a Plan 2 change and must then go in the contract gate.

### MA-9 — `begin_hand_stack_confirmation` does not test the confirmation rule it is named for
**Task 7, Step 1**:
> "`expect(wizardResult(w)).toBeNull();expect(fake.calls).toHaveLength(0);`"

The test drives the pure `wizardKey` reducer directly and never constructs an `EntryController`, so `fake.calls` is trivially empty at every point — the assertion cannot fail. Spec §4.3's requirement is behavioural: "the UI prefills each stack with the previous hand's start minus its committed chips and **marks it unconfirmed until Enter**", i.e. no `begin_hand` command is issued while any selected stack is unconfirmed. §13.4 names this test.

**Edit:** rewrite around the controller: `const entry=new EntryController(tauriBackend,rec,config,{}); await entry.key('N'); await entry.key('1'); await entry.key('1'); await entry.key('Enter');` then five Enters, assert `fake.calls.filter(([n])=>n==='begin_hand')` is empty, then the sixth + card Enter and assert exactly one `begin_hand` with the confirmed stacks. Keep the pure-reducer assertions as a second test.

### MA-10 — six "verify" steps demand tests but supply no test code
SKILL.md lists "Write tests for the above (without actual test code)" as a plan failure. Instances:
- **Task 3, Step 7:** "Extend persistence test to overwrite a pre-existing file and to make the parent path a regular file… Extend stop test to hold an active fake request, signal close twice, then release it and observe exactly one engine shutdown."
- **Task 6, Step 5:** "Add input cases for `TimeCharge`, straddle with five seats, straddle below twice BB, and cap `2.501`→`2501`."
- **Task 8, Step 6:** "Add B below-min/above-max cases, B+Esc sends nothing, C chooses Check before Call, A uses the engine's `to`, Ctrl+Z clears partial input, key repeats send no mutation, and config text inputs retain native typing."
- **Task 9, Step 7:** "Confirm HeroComboOutOfSupport displays range mix with no per-combo EV; DeadlineBestSoFar displays both reached/target bp."
- **Task 10, Step 5:** "Add all NoDecision reasons as table-driven app assertions, including an active server `NoDecision` delivered in an identity-matched channel. Verify Ready equity appears in the DOM when it arrives after a Final and remains after a subsequent Pending."
- **Task 12, Step 5:** "Check the new profile's decision log for the same decision ID and cache miss."

These are the *most* spec-load-bearing assertions in the plan (the `HeroComboOutOfSupport` range-mix rule, the `DeadlineBestSoFar` reached/target display, the Pending-never-replaces-Ready rule) and they are the ones with no code.

**Edit:** write each out. They are all small; e.g. Task 9's is
```tsx
test('hero_out_of_support_and_deadline_best_so_far_render',()=>{
  installMockIpc(new FakeBackend());
  const r=recommendation({coverage:{kind:'Unsupported',reason:{kind:'HeroComboOutOfSupport'},partial:[]},
    actions:[], range_mix:[[{kind:'check'},.7],[{kind:'bet',to:28},.3]]});
  render(<RecommendationPanel display={view(r)} fallbackReason={null}/>);
  expect(screen.getByRole('region',{name:'Range-level mix'})).toBeVisible();
  expect(screen.queryAllByTestId('main-ev')).toHaveLength(0);
  const d=recommendation({coverage:{kind:'Approximate',reasons:[{kind:'DeadlineBestSoFar',reached_bp:190,target_bp:50}]}});
  render(<RecommendationPanel display={view(d)} fallbackReason={null}/>);
  expect(screen.getByText(/190/)).toBeVisible();expect(screen.getByText(/50/)).toBeVisible();
});
```

### MA-11 — `bundle.active: false` leaves `bundle.resources` unstaged for dev and `--no-bundle`
**Task 1, Step 5** sets `"bundle":{"active":false}`; **Task 3, Step 6** then says only "Set `bundle.resources` to `{ "../../../target/pokerai-runtime/": "runtime/" }`" without re-enabling `active`. Task 3, Step 7's verification runs `tauri build --no-bundle` and then says to "Inspect startup with chart bundles" — but with `active:false` (and with `--no-bundle` regardless) the resource mapping is a bundler step and nothing lands next to the exe, so `app.path().resource_dir()` finds no `runtime/`. Task 12's `run-e2e.ps1` copies `runtime/` by hand (lines 2655–2663) precisely because of this — but Task 3's own verification step would already fail.

**Edit:** in Task 3, Step 6, state explicitly that `bundle.active` stays `false` for phase 1, that `bundle.resources` is declarative only until a bundling task exists, and move the `runtime/` copy out of `run-e2e.ps1` into `stage-runtime.ps1` (which already builds the worker) so `desktop:dev` and `desktop:build` both place it next to the exe. Then Task 3 Step 7's inspection is reachable and Task 12 loses duplicated copy logic.

### MA-12 — `Engine::presolver_status/pause/resume` have no declared signature upstream
**Task 2, Step 5** reads `s.paused, s.running, s.pending, s.done, s.failed, s.tier_done, s.tier_total, s.measured_p50_s, s.estimated_remaining_s, s.scenario_hits` from `e.presolver_status()`.

The field names and types all match Plan 4's `cache::presolver::PresolverStatus` (`#[derive(Clone,Default)]`, no `Serialize` — the plan is right to project it by hand). But Plan 4 only says it "Produces … Engine `presolver_status/pause/resume` delegation for Plan5" (line 1470) with no receivers, no return type, and no statement of which module re-exports `PresolverStatus`. Plan 5 assumes `presolver_status(&self)`, but `Presolver::status(&self)` is behind an `Arc/RwLock` inside the engine's own state, so `&self` may or may not be achievable.

**Edit:** add the three signatures to the contract gate verbatim, and note that `EnginePort::dispatch(&mut self, ...)` gives `&mut Engine`, so `&self` and `&mut self` both work — the executor only needs the return type of `presolver_status` and the import path of `PresolverStatus` confirmed before Task 2.

---

## 4. MINOR

- **MI-1 (Task 1, Step 3, `tsconfig.json`):** `"types":["vitest/globals","@testing-library/jest-dom"]`. `globals` is not enabled in `vite.config.ts`, and `setup.ts` already imports `@testing-library/jest-dom/vitest`, which is the v6 way to register the matchers on Vitest's `expect`. Listing the bare package in `types` is redundant and can pull in Jest-shaped global augmentation. Drop both entries, or keep only `"@testing-library/jest-dom"` if a matcher type turns up missing.
- **MI-2 (Task 1, Step 4):** "Declare the example with `required-features = ["typescript"]`" — no `[[example]]` block is shown. Add
  ```toml
  [[example]]
  name = "export_ts"
  required-features = ["typescript"]
  ```
  to the `crates/proto/Cargo.toml` additions snippet.
- **MI-3 (Task 1, Step 4):** "include every additional public type Plan 1 introduces in that same registry" is an instruction without a list. I checked it: the 58-entry `roots!` list is **complete** against Plan 1's `proto` (cards/game/hand/range/recommendation/tree/worker), including `SolveInput`, `CardParseError`, `Menu`, `PlayerMenus`, `RaiseSize` and all eleven worker types. Replace the instruction with "the registry below is complete against Plan 1 as of this date; the witness test in Step 4 enforces it."
- **MI-4 (Task 1, Step 5/6):** `lib.rs::run()` contains `tauri::generate_context!()`, which is compiled by `cargo test -p pokerai-app` as well. `frontendDist` is `../dist`, and `.gitignore` ignores `apps/pokerai-ui/dist/`. In debug builds with `devUrl` set, Tauri resolves the dev URL rather than embedding assets, so this *should* pass on a clean checkout — but it is the plan's single riskiest untested assumption for "green `cargo test --workspace` after every task". Add to Step 6: "if `generate_context!` errors on a missing `dist`, create `apps/pokerai-ui/dist/index.html` as a committed placeholder and un-ignore it."
- **MI-5 (Task 2, Step 4):** the `command!` `macro_rules!` wraps `#[tauri::command]`, which itself emits a `#[macro_export] macro_rules! __cmd__<name>` consumed by `generate_handler!`. This nesting is plausible but unverified. Add a one-line note: "if `generate_handler!` cannot resolve `commands::__cmd__set_game_config`, expand the macro by hand — the fourteen functions are three lines each."
- **MI-6 (Task 2, Step 5):** `MAX_SAFE_ID`/`safe_identity` reject `hand_id`/`decision_id` above 2^53−1 with an `AppError::Engine`. The spec has `hand_id: u64` and no such limit; this invents a failure mode. It is defensible (JSON numbers), but say so and note that Plan 2's `IdentityState` counters start at 1 and increment, so the check can never fire in practice — it is an assertion, not a feature.
- **MI-7 (Task 2, Step 5):** `ChannelSink::emit` silently drops an event whose identity fails `safe_identity`. Dropping an event without any user-visible trace conflicts with §12's "visible errors" posture. Prefer emitting a `NoDecision`-shaped shell warning, or at minimum add a comment saying the drop is unreachable (see MI-6).
- **MI-8 (Task 5, Step 3):** `estimate()` protects `Ready` only against `Pending`, not against `Unavailable`. Spec §4.4 says "a `Ready` estimate is never replaced by a `Pending` one" and that the engine emits at most one `Ready`, so this is literal-correct; note that the narrower guard is deliberate.
- **MI-9 (Task 5, Step 3):** progress coalescing uses a trailing 250 ms timer, so the *first* `Progress` is delayed by 250 ms. Spec §7 says "at most every 250 ms", which permits this, but a leading-edge throttle would show progress sooner on a slow flop. State the choice.
- **MI-10 (Task 7, Step 3):** `prefill` computes `values[seat] = start - (start - remaining)`, which is identically `remaining`. It matches spec §4.3's wording literally ("previous hand's start minus its committed chips"), and the `undefined` guards need both values, so the form is defensible — but add a one-line comment, because it reads as a bug.
- **MI-11 (Task 8, Step 4):** `case 'H':` inside the `switch` is dead — the earlier explicit `if(h&&upper==='H'&&this.value.mode!=='hero'&&this.value.text==='')` branch already returns for every `H` that reaches it. Delete the switch case or the early branch.
- **MI-12 (Task 8, Step 4):** the 64-key input queue cap and its "Input queue full" message are not in the spec. Harmless and cheap; flag as a deliberate addition rather than leaving it unexplained.
- **MI-13 (Task 9, Step 3):** `headline()` returns `null` whenever `coverage.kind === 'Unsupported'`. Spec §4.4's three headline rules do not mention coverage at all. The behaviour is right (Unsupported carries no numeric EV and, for `MultiwayEv`, no `actions`), but it is an added rule — record it as a deviation with its justification, so a later reader does not "fix" it.
- **MI-14 (Task 9):** `Recommendation.legal` (spec §4.4: "legal intervals, always present") is never rendered. `Entry.tsx` shows legality derived from `HandState.derived.legal` instead. During the `Fast` phase these agree, but the recommendation's own interval list — the thing spec §1.1 promises as part of the answer — is invisible. Add a one-line render of `r.legal` to `RecommendationPanel`.
- **MI-15 (Task 12, Step 1):** `npm install --save-exact ... webdriverio@9 @wdio/cli@9 @wdio/types@9` resolves each to the newest 9.x, while `@wdio/tauri-service` 1.3.0 pins `webdriverio 9.30.0` and `@wdio/types 9.29.1` in its own dependencies. The plan already hedges ("lock exact compatible versions at execution"), but name the two pinned versions so the executor has a known-good fallback. Also note `@wdio/tauri-service` 1.4.0 now exists; 1.3.0 remains valid.

---

## 5. Spec coverage (Plan 5's scope per the BRIEF series map)

Every requirement in scope maps to a task. No gap found beyond BL-6 (§12/§3.7 startup banners have no delivery mechanism) and MI-14 (§4.4 `legal`).

| Spec | Tasks | Status |
|---|---|---|
| §3.1/3.6 Tauri 2 + Vite/React/TS, MSVC, worker owned by engine | 1, 3, 11, 12 | covered |
| §3.4 commands never block; std threads, never Tokio workers | 2, 3 | covered — bounded `try_send` + oneshot, named `app-dispatch`/`app-startup`/`app-shutdown`/`config-writer` threads, no `Mutex<Engine>` in a command |
| §3.5 fourteen command names, exact | 2, 4 | covered and verified name-by-name |
| §3.5 build-time `ts-rs` bindings for every `proto` type | 1 | covered; registry complete (MI-3); placement bug MA-7 |
| §4.2 config fields, frozen `HandConfig`, 10 s default / 30 s max | 3, 6, 10 | covered; MA-8 breaks the mid-hand path |
| §4.3 3–6 dealt seats, straddle preconditions, stack confirmation | 6, 7 | covered; MA-9 weakens the test |
| §4.4 five-field identity, admission race, Equity-after-Final, Ready≠Pending, phase ordering | 4, 5, 10 | covered; the buffered-early-event and generation logic is correct as written |
| §4.4/§6 headline rules, tie break, `unresolved_mass`, `range_mix`, `Unavailable` | 9 | covered, wording exact; MI-13 |
| §4.4/§6 coverage reasons + `partial`, assumptions, equity, experimental isolation, NoDecision | 9, 10 | covered |
| §5.1 keyboard map (N,H,F,C,A,B,board,Ctrl+Z,Space,E,X,T) | 7, 8 | covered; the E/X/H-before-board ordering and the `T`-as-ten-rank case are handled correctly |
| §5.1 WebView2 accelerator suppression | 11 | covered; correctly documents that Tauri 2.11.5 does **not** expose wry's `with_browser_accelerator_keys` (R5 §1 flagged this as unverified — the plan resolved it) |
| §7 progress "measuring" while `None`, 250 ms coalescing, 15 s flop delivery | 5, 9, 12 | covered |
| §10.5 pre-solver status/pause/resume | 2, 3, 10 | covered; MA-12 |
| §12 visible errors, frozen config, stale results | 2, 3, 5, 6, 8, 10 | covered except startup banners (BL-6) |
| §13.4 eight Vitest names + `mockIPC` + Rust MockRuntime tests | 2, 4–10 | all eight present verbatim |
| §13.4 `e2e_full_hand_srp_flop_recommendation` | 12 | present; MA-5, MA-6 |
| §14.4 V21/V22 | — | correctly deferred to Plan 4 |
| §11 exploit slice / V9 | 2, 9 | correctly deferred per outline §0b |

**Spot-checks that passed.** The Task 10 keystroke script consumes exactly its 18 scripted `HandState`s and its expected 19-name command trace is correct (I re-derived it action by action). The Task 12 E2E's seat arithmetic is correct for button seat 0 with six dealt seats (UTG = seat 3 → "Seat 4", then 5, 6, 1) under spec §2's preflop order. Task 1's serde witnesses match Plan 1's actual attributes exactly: `Action::AllIn` → `"allin"` (`rename_all="lowercase"`), `LegalAction::AllIn` → `"all_in"` (`snake_case"`), `Availability`/`Coverage` PascalCase (no `rename_all`), `Rake::TimeCharge` → `"time_charge"`, `HandPhase` tagged `"phase"`, `Card(51)` → `"As"` (12*4+3). Task 6's rake-cap decimal parse (`2.5` → `2500`, `2.501` → `2501`) and the `(rake.rate*100)` round-trip (`0.05*100 === 5` exactly in IEEE-754 doubles) are both correct. Task 2's queue-saturation test arithmetic (one item consumed by the blocked dispatcher, then exactly 32 buffered, then Busy) is correct for `sync_channel(32)`. Plan 3's packaged chart layout (`<name>.json` + `<name>.manifest.json` siblings, Plan 3 line 369) matches `stage-runtime.ps1`.

---

## 6. Task ordering

`cargo test --workspace` stays green after every task **provided the BL-1…BL-5 signature fixes land before Task 2**. No task depends on a later one. Task 2's `EngineAdapter` forward-declares `accepted_config`/`on_shutdown` and supplies no-op boxes until Task 3 — the plan states this explicitly and it is a legitimate seam, not a placeholder. Tasks 4–11 are frontend-only and cannot regress the Rust workspace. One gap: Task 2 defines `EngineAdapter` but never constructs one in a test, so the real-engine call sites (the five blockers above) are only checked by `cargo check`, not by a test — which is exactly why they went unnoticed.

## 7. Interfaces (Produces/Consumes vs §3.5 and the series map)

The Produces blocks are unusually complete and their names are the spec's. The single systematic failure is the Consumes side: **every** engine signature in the façade block at lines 53–78 was written from the spec's §3.5 sketch rather than from Plan 2's produced API, and five of the twelve differ (BL-1…BL-5). The plan is honest about this — line 51 and line 2708 both say Plan 2 was "still an interface outline" — but Plan 2 is now written, and reconciling it is a five-minute edit that turns six blockers into zero. Do that edit before execution rather than at Task 2.

## 8. YAGNI

Nothing meaningfully out of scope. The plan correctly omits R5 §7's `httpBackend`/axum LAN split, the exploit panel, preflop EV, and `flop_full_v1`. `TAURI-V2-NOTES.md` is not spec-required but is the documented mitigation for R5 §1(e) ("LLMs frequently suggest code from v1") and earns its place. The three additions that are pure invention — the 64-key input queue cap (MI-12), `MAX_SAFE_ID` (MI-6), and the `pokerai:warning` DOM CustomEvent channel — are each small and each serve a real spec requirement; only the first two need a sentence of justification.
