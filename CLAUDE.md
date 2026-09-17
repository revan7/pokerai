# PokerAI — root contract (CLAUDE.md)

## 1. Purpose

PokerAI is a Windows 11 desktop assistant for 6-max NLHE cash, played live and entered by hand at the table: a Rust workspace with a Tauri 2 UI, a heads-up CFR postflop solver pinned as a vendored fork and run in a separate worker process, plus a preflop strategy store. The project is documentation-only right now — spec revision 8 and implementation plans 1-5 at revision 3 (114 tasks; order in docs/superpowers/plans/EXECUTION-ORDER.md); execution started 2026-09-17 on branch phase-c. An orchestrator (Claude, "Fable") owns the outline and delegates every atomic task to subagents (Claude Sonnet/Opus/Haiku, OpenAI Codex CLI), with an independent review gate on each.

## 2. Repository map

Current tree:

| Path | Contents |
|---|---|
| `docs/design/2026-09-10-design-outline.md` | Orchestrator decisions. Highest authority. |
| `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` | The contract spec, revision 8, 809 lines. |
| `docs/superpowers/plans/2026-09-10-plan-1-foundation.md` | Plan 1: workspace + `proto`, `core-model`, `core-ranges`, `core-iso`, `core-eval`, Python oracles. |
| `docs/superpowers/plans/2026-09-10-plan-2-worker-engine.md` | Plan 2: `solver-worker`, `engine`. |
| `docs/superpowers/plans/2026-09-10-plan-3-preflop-replay.md` | Plan 3: `core-preflop`, `core-replay`. |
| `docs/superpowers/plans/2026-09-10-plan-4-flop-cache-presolver.md` | Plan 4: `cache`, background pre-solver. |
| `docs/superpowers/plans/2026-09-10-plan-5-ui.md` | Plan 5: `pokerai-app`, `apps/pokerai-ui`. |
| `docs/research/R1..R8-*.md` | Research reports (solvers, preflop, libraries, exploit layer, app stack, neural feasibility, PokerData rights, solver bench). |
| `docs/research/SYNTHESIS.md` | Research synthesis, corrected by `REVIEW-of-research.md`. |
| `docs/research/REVIEW-of-{spec,plan,research}-*.md` | Independent review reports. |
| `docs/research/SPEC-CHANGELOG-N.md`, `PLAN-N-CHANGELOG-M.md` | Revision changelogs (section 3). |
| `docs/INDEX.md` | Doc map and supersession status; read first each session. |
| `docs/CONVENTIONS.md` | Naming/style conventions. |
| `docs/AUDIT.md` | Consistency-audit checklist (section 3 of that file says how to run it; results land in `docs/research/AUDIT-YYYY-MM-DD.md`). |
| `docs/log/YYYY-MM-DD.md` | Per-day task log. |
| `docs/README.md` | Docs landing page. |
| `AGENTS.md` | Codex CLI's instructions. |
| `CLAUDE.md` | This file. |

Future code layout (spec §3.2, not yet created):

```
Cargo.toml                      workspace manifest; .cargo/config.toml sets -C target-feature=+avx2
crates/proto                    serde types shared by UI, engine, worker (single source of truth)
crates/core-model                cards, positions, hand lifecycle/state machine, legal actions
crates/core-ranges               range strings <-> Range1326, blocking, hero-conditioned copies, hashing
crates/core-iso                  board canonicalization (1,755 flop classes), suit permutations
crates/core-eval                 7-card evaluator, range-vs-range equity (exact + Monte Carlo)
crates/core-preflop              PreflopSource trait; PokerDataJson and ChartTranscription adapters
crates/core-replay               Bayesian public-range replay across streets
crates/cache                     on-disk street-solution store; background pre-solver queue
crates/engine                    coverage classifier, tree builder, worker client, result assembly
crates/bench                     benchmark harness; release gate
solver-worker/                   AGPL-3.0 binary; JSON-lines stdio protocol; wraps the vendored solver
third_party/postflop-solver/     vendored fork, pinned commit, LICENSE, PINNED_COMMIT, PATCHES.md
apps/pokerai-ui/                 Vite/React/TypeScript frontend + src-tauri (crate pokerai-app)
tools/                           Python 3.12, dev only: test oracles, chart ingestion, bench scripts
fixtures/                        committed test fixtures: hands, eval oracles, charts, worker wire, e2e
docs/                            as above
```

## 3. Sources of truth and precedence

Precedence, highest first: **outline > spec > plans > research**. The outline (`docs/design/2026-09-10-design-outline.md`) records orchestrator decisions and wins any conflict. The spec is the contract implementers build against. Plans are the spec broken into atomic tasks. Research reports are evidence, not requirements — they only bind through the spec or a changelog.

Revision rule: any change to the spec or a plan bumps that document's revision line (e.g. "revision 6" -> "revision 7") and adds a changelog file under `docs/research/`, named `SPEC-CHANGELOG-N.md` (spec) or `PLAN-N-CHANGELOG-M.md` (plan N, changelog M). `docs/research/SPEC-CHANGELOG-1.md` through `SPEC-CHANGELOG-5.md` and `PLAN-1-CHANGELOG-1.md` through `PLAN-5-CHANGELOG-1.md` already exist as examples of this pattern — the next spec edit adds `SPEC-CHANGELOG-6.md`, not a rewrite of an existing one. A changelog entry states what changed and why, with a pointer to the review finding that drove it. Superseded documents are never deleted or moved — they stay in place at their original path and are marked superseded in `docs/INDEX.md`, which is the only place that tracks current-vs-superseded status.

## 4. Orienting in a new session

Claude Code loads this file automatically at the start of a session, and Codex reaches the same rules through `AGENTS.md` (section 1 there points back here); neither file is a step below, they are already in effect.

1. Read `docs/INDEX.md` for the current map and what is superseded.
2. Read the newest dated file in `docs/log/` (by filename, `YYYY-MM-DD.md`; `docs/log/README.md` is the format convention, not a dated entry, and is never "newest") for what the last session did and left pending.
3. Read only the specific document your task needs (the relevant spec section, the one plan, the one research report).

Never read the whole `docs/research/` tree in one session — it is evidence for specific claims, not a briefing document. If a task turns out to need a second or third research report beyond the one named in the plan, that is a signal to check whether the plan under-specified the task, not a reason to keep pulling in more files.

## 5. Working conventions

- Tasks are atomic: one crate/module/behavior per task, matching the granularity in the plans.
- TDD strictly for any task that changes code behavior: write a failing test, run it and confirm it fails, implement, run it and confirm it passes (`superpowers:test-driven-development`). Documentation-only tasks (specs, plans, this file, reviews, journal, index) have no code to fail red first; their verification is the check named in the task (paths exist, an inventory matches, a source quote is verified) with the actual command output quoted — see the gate rule below.
- One commit per task. Message format is exactly `feat(<crate>): ...`, `test(<crate>): ...`, `chore: ...`, or `docs: ...`. The body carries a trailer-style line `Task: <id>` on its own line above the `Co-Authored-By` trailer: implementation commits use `Task: P<n>.T<m>` (plan `n`, task `m`); documentation/orchestration commits use `Task: A<n>` or `Task: B<n>` (phase A/B item `n`) or `Task: none` when no task id was assigned. This is the one mapping mechanism from commit to task (`docs/AUDIT.md`'s ten-task count and its `git log` grep both key off this exact line) — no other tag or convention is used. Every commit still ends with the trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. The five plans' own commit-message examples (e.g. plan 1's `chore: workspace skeleton`) predate this rule; read every such example with a `Task: P<n>.T<m>` line added above the co-author trailer — the plans are not edited to add it.
- The `cargo test --workspace` gate applies from the moment the Cargo workspace exists (plan 1 Task 1 onward, once `Cargo.toml` is created) — it must be green after every task from then on, and Python tests green under `pytest` for anything touching `tools/`. Exhaustive suites run behind `--features exhaustive` and are not part of the default gate. Before the workspace exists, and for any documentation-only task at any time (even after the workspace exists), there is no Rust/Python/UI suite to run for that task: state "Rust/Python/UI execution: not applicable" explicitly and instead run and quote the task's own named checks (grep/diff/inventory verification, source-quote checks against the cited file). Never represent an absent workspace, or a doc task's lack of code, as a passed test suite. Quote the actual command output before claiming green — never assert it from memory.
- After each task, append one entry to today's `docs/log/YYYY-MM-DD.md` (create the file from the canonical template in `docs/log/README.md` if it doesn't exist yet today) using that template's fields exactly: time, agent, task id (required when one was assigned to the task, `none` otherwise), what changed, files touched, verification (quoted output or a linked report; `not applicable` is allowed for a doc task with no runnable suite), decisions, open items, and `commit: pending` or the actual hash if already known. Because the journal is append-only and agents other than the orchestrator do not commit, an implementing agent always writes `commit: pending`; after the orchestrator commits that work, it appends a separate one-line journal entry `committed <hash> for <task id>` rather than editing the pending entry. The orchestrator alone edits `docs/INDEX.md`; an agent proposes the INDEX row(s) its task needs in its own report instead of writing them.
- Every task is reviewed by a different agent than the one who implemented it before any task that depends on it proceeds.
- Review findings are redelegated back to an implementer and fixed; they are never hand-waved, downgraded, or marked resolved without a re-run of the failing check.

## 6. Engineering rules (exact values, from the spec)

| Rule | Value |
|---|---|
| Rust edition | 2021 |
| Toolchain | Pinned to `stable-x86_64-pc-windows-msvc` via `rust-toolchain.toml`, repo-wide. GNU fallback (spec §3.6): plan 2's V1 must build and run the pinned solver example on MSVC with `+avx2` and compare its FLOP-FAST time to the GNU figure in R8 §5; if that build fails or is >25% slower, `solver-worker` alone is built with `cargo +stable-x86_64-pc-windows-gnu` via `scripts/build-worker-gnu.ps1`, while the rest of the workspace stays MSVC. |
| AVX2 | Required. `.cargo/config.toml` sets `rustflags = ["-C", "target-feature=+avx2"]` for both `x86_64-pc-windows-gnu` and `x86_64-pc-windows-msvc` (spec §3.7); `solver-worker/build.rs` fails the build if `CARGO_CFG_TARGET_FEATURE` lacks `avx2`; the engine refuses a worker whose reported `build_features` lacks it. |
| Solver pin | `postflop-solver` at commit `9d1509fe5077d019825f833eed04b16d342dfda1`, vendored under `third_party/postflop-solver/`, with two patches recorded in `PATCHES.md`: (1) pin `bincode`/`bincode_derive` to `=2.0.0-rc.3`; (2) fix the `dangerous_implicit_autorefs` lint in `src/action_tree.rs` lines 393/396/408. |
| Dependency direction | `proto` <- `core-*` <- `cache`/`engine` <- `bench`/`pokerai-app` (spec §3.2), strictly downward. `solver-worker`'s only project dependency is `proto`, plus the vendored solver; no crate depends on `solver-worker`. |
| AGPL boundary | `solver-worker` is the only `AGPL-3.0` crate; every other crate is `MIT OR Apache-2.0` (spec §3.3). The boundary is a process boundary over a documented JSON-lines protocol. Private use creates no obligation; `LICENSE`, `PINNED_COMMIT`, `PATCHES.md` are kept so Corresponding Source could be produced if the app is ever shared (a phase-2 item, not built now). |
| Money units | Integer chips (`u32`) everywhere in `HandState`, worker, and cache wire types; the rake cap is `cap_mchips: u32` (thousandths of a chip); EV is signed finite `f32` chips; fold = 0 exactly; chips already in the pot are sunk (spec §2). |
| Coverage labels | `Exact`, `Approximate{reasons}`, `Unsupported{reason}` — never a claim of full-game GTO, only of input/model matching (spec §2). |
| No nearest-flop substitution | A canonical board class absent from the cache is solved live. Never substitute a different flop, ever. |
| Multiway EV | 3+ pot-eligible unfolded players is `Unsupported` for numeric EV; only weighted range-vs-range equity and a visually separated experimental heuristic are shown (spec §6). |
| Hero's cards | Never enter any public range, solve input, or cache key. Hero-conditioned copies exist only for hero-combo equity and terminal calculations (spec §2). |

## 7. Agent rules

Do:
- Follow TDD and the atomic-task/one-commit-per-task discipline of section 5.
- Quote actual command output (test runs, build logs) as evidence before claiming success.
- Stop and report when blocked — ambiguous spec text, a failing environment prerequisite, a review finding you can't resolve — rather than guessing.
- Use this review report format exactly: **ID**, **severity** (`BLOCKER`/`MAJOR`/`MINOR`), **location** (file:line or section), the **quoted problem**, and a **concrete edit** that would fix it.

Never:
- Edit the spec or a plan without adding the matching changelog file (section 3).
- Restructure `docs/` folders or rename documents, ever — supersession is tracked in `docs/INDEX.md`, not by moving files.
- Claim tests pass without having run them and quoting the output.
- Modify any file outside the assigned task's scope.
- Commit unless the task explicitly calls for a commit.
- Edit `docs/design/2026-09-10-design-outline.md`: it is the orchestrator's own document, not a human-authored note — only the orchestrator amends it, and every amendment is dated (see its section 0b).
- Write into human-authored notes (anything under `docs/notes/`, or the user's own files) or the orchestrator's private memory directory.
- Fabricate a review finding's severity, location, or fix to look more or less serious than the evidence supports.

## 8. Model and tool policy

- Sonnet: well-specified implementation and doc tasks.
- Opus: reasoning-heavy work — architecture judgment calls, ambiguous spec resolution.
- Haiku: mechanical, low-risk edits.
- Codex CLI (`codex exec`): review gates, plus a share of implementation.
- Fable (Claude): orchestrates; delegates rather than implementing directly whenever a subagent can do the task.

Codex reads `AGENTS.md` for its own operating instructions; keep it in sync with this file's engineering rules whenever one changes. Codex quota is finite and can be exhausted mid-project — when it is, a Claude reviewer substitutes for the review gate without lowering the bar (same report format, same independence from the implementer).

## 9. Skills in use

Feature and plan work follows `superpowers:brainstorming` -> `superpowers:writing-plans` -> `superpowers:subagent-driven-development`, with `superpowers:verification-before-completion` gating any claim of done. Each of the five plans directs its executing agent to `superpowers:subagent-driven-development` (preferred) or `superpowers:executing-plans` to run it task-by-task against its checkbox list. Specs live under `docs/superpowers/specs/`, plans under `docs/superpowers/plans/`; both follow the naming convention `YYYY-MM-DD-<slug>.md` already in use, and neither directory is renamed or reshuffled (section 7).
