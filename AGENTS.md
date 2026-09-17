# Codex Agents: Rules and workflow

Read CLAUDE.md first; every rule there applies to you.

## Project summary

PokerAI is a Windows desktop poker assistant for 6-max NLHE cash games. The app bundles a preflop strategy store (PokerData or transcribed charts), a postflop solver (vendored b-inary/postflop-solver in a separate worker process), and a Tauri UI with keyboard-first entry. Implementation phase: 1-2 weeks to a strong solver baseline (river + turn + flop with cache and pre-solver). AGPL boundary: solver-worker links AGPL code; all other crates are MIT/Apache-2.0. Private use creates no obligations.

## How Codex works here

You receive ONE atomic task in your prompt. Follow these steps:

1. **Orient**: if you have not already this session, skim `docs/INDEX.md`, then the newest dated file in `docs/log/` (`YYYY-MM-DD.md` by filename; `docs/log/README.md` is the format convention, never "newest"). This is quick context, not a substitute for step 2.
2. **Understand the task**: Read the spec section(s) it names. Identify what to build or fix.
3. **Plan atomically**: Split the task into small steps. If the task involves code, start with TDD: write a failing test, run it, implement, verify green.
4. **Run and verify**: After each step, run the test command named in the task (or `cargo test --workspace` if unspecified and the Cargo workspace already exists). For a documentation-only task, or any task before the workspace exists (no `Cargo.toml` yet), there is no Rust/Python/UI suite to run: say "Rust/Python/UI execution: not applicable" explicitly and instead run the task's own named checks (paths exist, an inventory matches, a source quote is verified against the cited file). Paste **the real command output** (not a summary) into your report either way.
5. **Self-review**: Compare your result against the spec section the task cites. Check for correctness, coverage, and adherence to conventions. Flag any gaps or risks.
6. **Stop if blocked**: If you hit a blocker (missing info, unsupported operation, dependency issue), **stop and report the blocker** instead of guessing.

## Output conventions

- **Reports go to `docs/research/`** using the filename given in the task.
- **Chat message: at most 12 lines.** Format: one-line verdict, counts (tasks done / total, blockers), then brief findings or next steps.
- **Review findings** use this format per issue:
  - ID + severity (BLOCKER / MAJOR / MINOR)
  - Location (file:line or task/step)
  - Quoted problem (max 30 words)
  - Concrete edit (what to change)

## Boundaries

- **Do not commit.** The orchestrator commits; you write the work tree only.
- **Do not modify files outside the task's assignment.** The task names the file(s) to change; edit only those.
- **Do not install software or add dependencies** not named in the task.
- **Do not edit the spec or plans** without also writing the changelog the task names (e.g., `SPEC-CHANGELOG-6.md`).
- **If blocked, report the blocker** instead of guessing. Name the missing fact or resource.
- **Log your changes**: append one entry to `docs/log/YYYY-MM-DD.md` using the canonical template in `docs/log/README.md` (create the file from that template if it doesn't exist yet today) — fill every field: time, agent, task id (the one you were given, or `none`), what changed, files touched, verification (quoted output or a linked report; `not applicable` for a doc task with no runnable suite), decisions, open items. You do not commit, so always write `commit: pending` in the last field — the orchestrator appends a separate `committed <hash> for <task id>` line once it commits your work; never edit your own entry to add the hash.
- **Task-to-commit mapping**: state your task id clearly in your report. The orchestrator adds it as a `Task: <id>` trailer-style line above `Co-Authored-By` when it commits your work (`Task: P<n>.T<m>` for an implementation task, `Task: A<n>`/`Task: B<n>` for a documentation/phase task, `Task: none` if you were given no id) — this is the only task/commit mapping the project uses, so an accurate task id in your report matters.
- **Propose, don't write, `docs/INDEX.md`**: the orchestrator owns that file. If your task adds or changes a document, propose the INDEX row(s) in your report instead of editing `docs/INDEX.md` yourself, unless the task explicitly assigns you that file.

## Useful commands

| Command | Purpose |
|---------|---------|
| `cargo test --workspace` | Run all Rust tests; abort on first failure |
| `cargo build --release -p solver-worker` | Build solver worker in release mode |
| `cargo run -p bench -- run --suite <name>` | Run benchmark suite (river_std, flop_fast, etc.) |
| `python -m pytest tools/` | Run Python test oracles (dev only) |
| `npm test` (in `apps/pokerai-ui/`) | Run React/Tauri UI tests |

## Where things are

| Item | Path |
|------|------|
| Project root | `/d/Documents/Projects/PokerAI/` |
| Design outline (source of truth for decisions) | `docs/design/2026-09-10-design-outline.md` |
| Spec (revision 6, authoritative) | `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` |
| Plans 1–5 (implementation phases) | `docs/superpowers/plans/2026-09-10-plan-{1,2,3,4,5}-*.md` |
| Research docs (validation, reviews, benchmarks) | `docs/research/` (R1–R8 + reviews + changelogs) |
| Index of all docs | `docs/INDEX.md` — every file under `docs/`, its status, and the reading order |
| Daily log | `docs/log/YYYY-MM-DD.md` (create from `docs/log/README.md`'s template if needed) |
| Consistency-audit checklist | `docs/AUDIT.md` — when/how to run it; results of a run land in `docs/research/AUDIT-YYYY-MM-DD.md` |
| Rust workspace (future layout, not created yet) | `Cargo.toml` at root; crates in `crates/`, solver worker in `solver-worker/`, UI in `apps/pokerai-ui/` |
| Vendored solver (future, not created yet) | `third_party/postflop-solver/` (pinned, AGPL, LICENSE and PATCHES.md retained) |
| Test fixtures and benchmarks (future, not created yet) | `fixtures/`; bench reports planned for `docs/bench/` once plan 4/5 produce them |

---

**Your task is atomic. Read it once, execute it end-to-end, report once. Do not commit. Do not edit outside the task's scope.**
