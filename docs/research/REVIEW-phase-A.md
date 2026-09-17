---
type: review
status: current
date: 2026-09-17
supersedes: none
related:
  - CLAUDE.md
  - AGENTS.md
  - docs/INDEX.md
  - docs/CONVENTIONS.md
  - docs/AUDIT.md
  - docs/log/README.md
  - docs/log/2026-09-10.md
  - docs/log/2026-09-17.md
---

Phase A review: read all ten assigned files fully, one file at a time; checked their claims against the named sources; completed a cross-file pass and rechecked the findings against source lines and Git history. Locations below use the reviewed, unmodified files' line numbers. Findings: **0 BLOCKER, 10 MAJOR, 10 MINOR**. Only this report was written.

1. **PA-01 — MAJOR**
   - **Location:** `docs/INDEX.md:113` (also line 82); `docs/AUDIT.md:119`.
   - **Quoted problem:** “§1-§3 of that review are fully applied”
   - **Evidence:** The index reduces Phase B to regenerating execution order and effort. Two actual interface disagreements remain: Plan 2 line 6418 defines `Paths { log_dir, worker_exe, preflop, cache }`, while Plan 4 line 63 and Plan 5 line 77 require `worker`/`log`; Plan 2 lines 6422–6424 return `Engine` and expose `startup_report()`, while Plan 5 line 56 requires a tuple from `Engine::new`. The pause entry and AUDIT correctly retain these concerns. `PLAN-2-CHANGELOG-1.md:98`, `:101`, and `:126` record the later orchestrator choices, so an earlier APPLIED disposition does not establish agreement.
   - **Concrete edit:** Correct both INDEX summaries to retain the field-name and startup-interface seams alongside order/effort regeneration. In AUDIT, merge the duplicate `Paths` items and cite the later Plan 2 decisions instead of asking the reviewer to choose an API anew. Record that the surrogate and oracle producers exist at Plan 4 Tasks 11 and 23 (`:1982`, `:3729`); the remaining check is stale consumer references to Tasks 10/20. Append a matching current handoff to the journal after the edits, without claiming Phase B is complete.

2. **PA-02 — MAJOR**
   - **Location:** `docs/log/2026-09-10.md:10` (also lines 20, 25, 35, 40, 45, 50, 55, 60, 70, 75, 80, 85, 90, 95).
   - **Quoted problem:** “`docs/research/PokerData-verification.md`”
   - **Evidence:** That file and `docs/research/DECISIONS-2026-09-10.md` do not exist. The plan paths omit the date, and the Plan 2/5 slugs are also wrong. The 14:17 commit changes review/changelog **2**, not review 1; 14:54 uses changelog **3**, not 4; 15:21 uses changelog **4**, not 5. Git file lists also show interleaved changes that commit subjects alone conceal: for example, `f164807` changes only Plan 2; Plan 5's changelog is in `f71c7ce`.
   - **Concrete edit:** Add a clearly marked reconstruction correction keyed by the 19 commit hashes, using `git show --format=fuller --name-only <hash>` to distinguish a subject's reported work from files actually committed. Use `R7-pokerdata-verification.md`, the design outline for approval decisions, and the five exact dated plan paths already verified in INDEX. Correct the review/changelog associations above and record interleaved file changes explicitly. Preserve earlier entries under the append-only rule.

3. **PA-03 — MAJOR**
   - **Location:** `docs/log/2026-09-10.md:44` (also lines 11, 19, 46, 81).
   - **Quoted problem:** “MSVC C++ workload installed”
   - **Evidence:** `af0b02f` adds outline §0b saying the user **authorized installing** the workload, not that installation was verified. The 12:39 chart-first decision belongs to the 15:31 approval; at `7f42ca0`, charts are a fallback to PokerData. The spec stored at `a266e87` is already revision **2** after the first fix pass. No workspace/toolchain file exists in this documentation-only tree, despite the 19:28 entry's unqualified “MSVC pinned via rust-toolchain.toml.”
   - **Concrete edit:** Append corrections separating authorization, planned implementation, and measured completion. Attribute chart-first to its actual decision commit; state the first spec fix pass ended at revision 2; describe the MSVC pin as a plan requirement. If installation was verified later, cite that later evidence separately instead of backdating it to the approval commit.

4. **PA-04 — MAJOR**
   - **Location:** `CLAUDE.md:73` (also line 71); `AGENTS.md:15`.
   - **Quoted problem:** “`cargo test --workspace` must be green after every task”
   - **Evidence:** There is no `Cargo.toml`. The command fails with the actual output recorded below. This rule makes a documentation/review task impossible to complete under the default contract. AGENTS qualifies TDD as applying to code; CLAUDE does not.
   - **Concrete edit:** Apply red/green TDD to code behavior changes and the workspace gate once the workspace exists. For documentation-only tasks, require the named source/path/inventory checks and their actual output, with Rust/Python/UI execution marked not applicable where appropriate. Keep genuine code-test failures blocking; do not represent an absent workspace as a passed test suite.

5. **PA-05 — MAJOR**
   - **Location:** `docs/log/README.md:21`; `CLAUDE.md:74`; `AGENTS.md:36`.
   - **Quoted problem:** “Task ID (e.g., "A3", "P1.T4") or omit”
   - **Evidence:** The template omits CLAUDE's required test result and commit hash and makes the task ID optional. AGENTS instead requests one line. The journal must be appended before committing, but agents may not commit and the task's own commit hash cannot already be known. Unqualified journal/INDEX update rules also conflict with report-only assignments such as this one. AUDIT F1 rejects legitimate completed but uncommitted work unless a blocker is recorded.
   - **Concrete edit:** Make one canonical entry template containing task ID when assigned, agent, changes/files, verification output or linked report, decisions/open items, and `commit: pending` or an existing hash. Have CLAUDE and AGENTS link to that template. Explicitly assign the orchestrator the scoped journal/INDEX updates and an append-only commit-link entry when workers are authorized to edit only their assigned deliverable. Let F1 distinguish pending commits from blockers.

6. **PA-06 — MAJOR**
   - **Location:** `docs/AUDIT.md:26` (also line 82); `CLAUDE.md:72`.
   - **Quoted problem:** “counted across all plans by commits tagged `P<n>.T<m>`”
   - **Evidence:** The commit convention requires no task tag, the journal permits omitting IDs, and the plans' actual commit examples contain no `P<n>.T<m>` identifier. For example, Plan 1 line 225 uses `chore: workspace skeleton`. A compliant implementation can therefore be invisible to the ten-task trigger and G1.
   - **Concrete edit:** Specify one mapping mechanism. Either require an implementation task ID in the commit subject/body while retaining the existing co-author trailer, or have the orchestrator maintain a task-to-commit ledger in the journal and count that ledger. Replace the audit's optional “actual task-tag convention” fallback with the chosen exact rule. All inspected co-author trailers themselves match CLAUDE.

7. **PA-07 — MAJOR**
   - **Location:** `docs/AUDIT.md:46` (also lines 47, 48, 84).
   - **Quoted problem:** “Every subsection maps to at least one task in exactly one plan”
   - **Evidence:** Spec §13.1 intentionally spans Plans 1, 3, and 4; §3.5 has producers/consumers across the series. A subsection cannot have exactly one owning plan. B2 similarly conflates a test's owner with references to it. G3 asks for every spec test as soon as code exists, even when later plans have not run. V9 is explicitly conditional and user-owned in spec §14.4; Plan 3 excludes the converter/purchase.
   - **Concrete edit:** Map individual requirements/tests to producing tasks, allow consumers and integration coverage in other plans, and distinguish definitions from mentions. Record each check as completed, scheduled, conditional, or deferred with its source. At a before-plan or ten-task audit, enforce code coverage only for completed tasks and prerequisites; at release, enforce the full baseline. Preserve V9's explicit conditional status.

8. **PA-08 — MAJOR**
   - **Location:** `docs/AUDIT.md:54`.
   - **Quoted problem:** “or the mismatch is a documented, resolved item in `docs/research/REVIEW-cross-plan.md` §1”
   - **Evidence:** M13 documents a resolution to `worker`/`log`, but the later Plan 2 changelog deliberately adopts `worker_exe`/`log_dir`; consumers still disagree. An old resolution record can satisfy this exception while current signatures do not match. This would allow the central interface check to pass the concrete mismatch in PA-01.
   - **Concrete edit:** Require the current producer and all current consumers to agree after applying the latest authoritative decision. Historical dispositions are evidence pointers, not an alternative pass criterion. Permit only explicitly specified adapters whose actual signatures are checked at both ends.

9. **PA-09 — MAJOR**
   - **Location:** `docs/AUDIT.md:62` (also line 63).
   - **Quoted problem:** “Every bump N has `SPEC-CHANGELOG-N.md`”
   - **Evidence:** Spec revision 6 is documented by `SPEC-CHANGELOG-5.md`; CLAUDE correctly says the next edit creates changelog 6. Changelog sequence and resulting spec revision are different counters. The plan headers have no own numeric revision line to match the proposed grep. Existing valid changelogs also lack the mandated uniform Target/Source/status-row format: `SPEC-CHANGELOG-5.md` conveys application in its introduction and “Edit applied” column heading.
   - **Concrete edit:** Check each actual revision transition against a changelog that explicitly identifies its source, target document, and resulting revision; do not infer it from suffix equality. Recognize the existing plan revision evidence in `PLAN-N-CHANGELOG-1.md`, and require an explicit header revision on the next plan revision. Apply any new row-status schema prospectively and accept documented legacy dispositions rather than reporting nonexistent missing changelogs.

10. **PA-10 — MAJOR**
    - **Location:** `docs/AUDIT.md:85`.
    - **Quoted problem:** “A report exists, dated, with all six suites and the p50/p95/peak-RSS columns of §13.5”
    - **Evidence:** G4 can pass without any passing acceptance result. Spec §13.5 also requires cold-cache e2e p95 within 15 s, zero final-delivery violations in fault injection, supported-fixture EV/coverage, cache-hit latency, and analytic tolerances. The six street suite columns alone do not establish V21/V22, even though audits can be requested before those gates.
    - **Concrete edit:** For release audits, require the recorded baseline gate result and supporting street, e2e, fault, cache, and oracle evidence, checked against the actual §13.5 criteria. Preserve conditional store/V9 evidence separately. Earlier audits should record which gated measurements are not yet due, not claim a pass from report presence alone.

11. **PA-11 — MINOR**
    - **Location:** `AGENTS.md:57` (also line 59); `docs/README.md:20` (also line 22).
    - **Quoted problem:** “no separate INDEX.md yet; structure is self-explanatory”
    - **Evidence:** `docs/INDEX.md` and `docs/AUDIT.md` both exist; root `AUDIT.md` does not. README presents `docs/bench/` and `docs/notes/` as existing navigation destinations, but neither directory exists.
    - **Concrete edit:** Point AGENTS to `docs/INDEX.md` and `docs/AUDIT.md`; remove the stale creation claims. Mark bench/notes as planned destinations, without active directory links until created. Label the workspace/code paths as future layout, matching CLAUDE.

12. **PA-12 — MINOR**
    - **Location:** `AGENTS.md:3`; `CLAUDE.md:62`; `docs/README.md:9`; `docs/INDEX.md:98`.
    - **Quoted problem:** “Read CLAUDE.md first; every rule there applies to you.”
    - **Evidence:** README and INDEX instead require INDEX → newest log → CLAUDE → task document. CLAUDE's list skips itself; AGENTS's work steps go directly to the spec after its initial instruction. These are inconsistent executable reading orders, not just different link labels.
    - **Concrete edit:** State one order everywhere. If the agent contract must be loaded automatically first, explain that bootstrap explicitly, then use INDEX → newest dated `YYYY-MM-DD.md` log → task-specific document for orientation. Exclude `log/README.md` when selecting the newest journal.

13. **PA-13 — MINOR**
    - **Location:** `docs/CONVENTIONS.md:36`.
    - **Quoted problem:** “`review` | `docs/` (root) | Code reviews, design reviews”
    - **Evidence:** AGENTS line 21, all existing review reports, and AUDIT line 98 place reports under `docs/research/`. `docs/AUDIT.md` is a living checklist, not the output of an audit run.
    - **Concrete edit:** Set the review-report location to `docs/research/` with the established `REVIEW-*.md`/`AUDIT-*.md` naming. List `docs/AUDIT.md` explicitly as the living checklist exception and describe it consistently in README and CONVENTIONS.

14. **PA-14 — MINOR**
    - **Location:** `docs/CONVENTIONS.md:5`; `docs/log/README.md:31`.
    - **Quoted problem:** “All new documents include a YAML frontmatter block.”
    - **Evidence:** Of the seven new documents under `docs/` in this review, only AUDIT has frontmatter. README, INDEX, CONVENTIONS, log/README, and both daily logs do not. The copyable journal template perpetuates the omission. The user explicitly identifies these as new, uncommitted documents, so grandfathering existing documents does not clearly cover them.
    - **Concrete edit:** Add the documented fields to these six new docs and the journal template, with dates/statuses reflecting their actual roles; distinguish the reconstructed day's event date from its reconstruction date. Alternatively, explicitly define and apply a narrower frontmatter scope consistently. Keep INDEX dispositions authoritative and state how they relate to the coarse frontmatter status values.

15. **PA-15 — MINOR**
    - **Location:** `docs/log/2026-09-10.md:96` (also lines 18, 23, 98); `docs/log/2026-09-17.md:6`.
    - **Quoted problem:** “Codex quota exhausted (resume 2026-09-15 13:57 UTC)”
    - **Evidence:** The named repository sources and Git history do not establish that exact quota reset, the 20:00 pause time, or the per-entry reviewer/implementer identities. Commit times are recoverable; actor roles and service availability are not inferable from commit subjects. INDEX labels the old journal reconstructed, but the journal itself presents these details without provenance.
    - **Concrete edit:** Append a reconstruction/provenance note identifying commit-derived facts separately from an explicitly cited orchestrator handoff. Mark unsupported times/actors as unknown or approximate and quota state as unverified historical context; do not carry an expired expected reset forward as current availability evidence.

16. **PA-16 — MINOR**
    - **Location:** `docs/AUDIT.md:84`.
    - **Quoted problem:** “`grep -rn "fn <test_name>" crates/ solver-worker/ apps/`”
    - **Evidence:** This is a Rust declaration search, but G3 claims to cover every §13 test. Spec §13.4 and Plan 5 use Vitest/WebDriver TypeScript `test(...)`/`it(...)` names, which will not match `fn`. Correctly implemented UI tests would be reported missing.
    - **Concrete edit:** Split test discovery by runner/language: Rust function declarations and TypeScript named test cases, followed by actual test-run output. Use the owning task's test file and runner instead of a Rust-only existence test for all suites.

17. **PA-17 — MINOR**
    - **Location:** `docs/AUDIT.md:98` (also line 76).
    - **Quoted problem:** “writes findings to `docs/research/AUDIT-YYYY-MM-DD.md`”
    - **Evidence:** Before-plan, revision, ten-task, and fix-verification triggers can all fire on the same date. One output name cannot preserve multiple independent runs without a defined append/version rule. F1 also identifies the previous audit only by date, which does not identify the exact audited commit boundary.
    - **Concrete edit:** Use a distinct dated sequence/run identifier, or specify append-only run blocks within the daily report. Record the audited HEAD and prior audited HEAD, and check journal coverage using that commit range. Preserve both the failed run and its successful recheck.

18. **PA-18 — MINOR**
    - **Location:** `docs/INDEX.md:11` (all Path-column rows); `docs/CONVENTIONS.md:52`.
    - **Quoted problem:** “`docs/INDEX.md`”
    - **Evidence:** The index's paths are code spans rather than links. README promises an index “with status and links” and CONVENTIONS requires relative Markdown links. A fresh agent/user must copy or retype every path from the central navigation document.
    - **Concrete edit:** Turn each Path cell into a relative Markdown link from `docs/INDEX.md`, retaining the full repo-relative path as its label. Verify every destination against the same inventory that already passes.

19. **PA-19 — MINOR**
    - **Location:** `docs/AUDIT.md:69`.
    - **Quoted problem:** “`find docs -iname "*.md" \| sort`”
    - **Evidence:** E1 promises every file under `docs/`, but enumerates only Markdown. This happens to match today's 45-file tree; later images, PDFs, or benchmark data would be omitted without a finding. It also compares entries without expressly checking duplicate rows.
    - **Concrete edit:** Enumerate all files recursively, normalize repo-relative paths, and compare sets plus row multiplicities. For example, use `rg --files --hidden --no-ignore docs` and explicitly filter directories if using another enumerator. Require one row per actual file and separately classify deliberately planned paths.

20. **PA-20 — MINOR**
    - **Location:** `docs/INDEX.md:96`; `CLAUDE.md:7`.
    - **Quoted problem:** “what exists, what's current vs. historical, and where the open threads are”
    - **Evidence:** The prescribed initial documents contain 3,763 whitespace-delimited words: INDEX 1,996, CLAUDE 1,670, latest log 97. The complete plan counts appear in INDEX's rows, total, and authority list; CLAUDE repeats much of the current document map. Most of the index is historical detail before the reading-order/open-work section. This adds reading cost without strengthening a fresh agent's immediate safety checks.
    - **Concrete edit:** Put a short orientation block first: documentation-only state, authoritative revisions, actual pending seams, next authorized phase, and links to the root contract/newest log/task. Explicitly allow the rest of the inventory to be scanned on demand. Keep counts in one canonical table and replace duplicate maps/count lists with references, retaining the engineering rules and full inventory.

Verification and self-review:

- Inventory before this report: all **45 files** under `docs/` had exactly one INDEX row; there were no missing or nonexistent indexed paths. This report creates a 46th file and is intentionally not added to INDEX because the assignment permits editing only the report. Its eventual INDEX/journal update belongs to the orchestrator.
- Verified spec revision 6/807 lines, all plan task/step/line counts, solver commit `9d1509fe5077d019825f833eed04b16d342dfda1`, AVX2/MSVC/GNU rules, dependency direction, money units, coverage labels, and the co-author trailer against the specified sources. Plan “revision 1” is supported by each revision changelog; it is not an existing revision line in the plan headers.
- The historical status of spec/plan review cycles and changelogs is appropriate. `REVIEW-of-research.md` remains a current correction layer because its corrections are not merged into SYNTHESIS. `REVIEW-cross-plan.md` correctly says partially applied; PA-01 concerns its overstated closure description, not that status label.
- `.gitignore` correctly excludes the listed build/dependency/cache/temp paths and leaves reports/lockfiles visible. The optional 10M oracle ignore is explicitly produced by Plan 1 Task 1; absent future artifacts were not treated as current Phase A blockers. Template placeholders such as `HH:MM` are intentional examples, not unfinished project claims.
- Rechecked all reported locations and quotations. No finding assumes the application exists, treats a changelog claim as proof of consumer compatibility, or asks this review to repair Phase B implementation plans. The known `Paths` and startup seams are real; surrogate/oracle ownership exists, with stale task references remaining.

Actual compact inventory/count output (PowerShell file enumeration, INDEX Path-row extraction, task headings matched with `^#{2,3} Task [0-9]+:`, checkbox steps with `^\s*- \[ \]`):

```text
CLAUDE.md: 1670 whitespace-delimited words
docs/INDEX.md: 1996 whitespace-delimited words
docs/log/2026-09-17.md: 97 whitespace-delimited words
Pre-report docs files: 45
INDEX file rows: 45
2026-09-10-plan-1-foundation.md: tasks=25, steps=121, lines=5436
2026-09-10-plan-2-worker-engine.md: tasks=30, steps=128, lines=7099
2026-09-10-plan-3-preflop-replay.md: tasks=19, steps=107, lines=3172
2026-09-10-plan-4-flop-cache-presolver.md: tasks=26, steps=172, lines=4062
2026-09-10-plan-5-ui.md: tasks=14, steps=78, lines=3153
```

Actual `cargo test --workspace` output, exit code 1 (the expected absence of a workspace, used to verify PA-04; no application tests passed or failed):

```text
error: could not find `Cargo.toml` in `D:\Documents\Projects\PokerAI` or any parent directory
```

Actual `git -c safe.directory=D:/Documents/Projects/PokerAI log --date=iso --format="%ad %h %s"` output, exit code 0:

```text
2026-09-10 19:45:56 +0200 720f0f8 docs: plan 2 revised after review (30 tasks); changelog
2026-09-10 19:32:25 +0200 f164807 docs: plan 5 revised after review (14 tasks); changelog
2026-09-10 19:32:04 +0200 f71c7ce docs: plan 4 revised after review (26 tasks); changelog
2026-09-10 19:28:43 +0200 77d7fa8 docs: plan 1 revised after review (25 tasks); spec 3.6 toolchain rule; changelog
2026-09-10 19:27:16 +0200 fe0a01d docs: plan 3 revised after review (19 tasks); changelog
2026-09-10 19:02:51 +0200 8e94934 docs: plan reviews 1-5 and interim plan/changelog state before pause
2026-09-10 18:58:42 +0200 9d45fe0 docs: spec revision 6 (cross-plan amendments S1-S17); cross-plan and plan 3/4 reviews
2026-09-10 18:18:13 +0200 4f4619b docs: implementation plan 5 (Tauri UI + E2E) draft
2026-09-10 18:17:20 +0200 47c7ef8 docs: implementation plan 2 (solver worker + engine river/turn) draft
2026-09-10 18:15:05 +0200 ddcbe04 docs: implementation plans 1 (foundation), 3 (preflop/replay), 4 (flop/cache/presolver) drafts
2026-09-10 15:31:50 +0200 af0b02f docs: record approval decisions (spec rev 5 approved, flop budget 10 s, MSVC install, charts first)
2026-09-10 15:21:12 +0200 d69e7a1 docs: spec revision 5 after targeted verification (all-in operand, shared history branches, stop rule)
2026-09-10 14:54:40 +0200 6e837b4 docs: spec revision 4 after review round 3 and flop-policy amendment
2026-09-10 14:31:51 +0200 ab8d2ef docs: realistic-range flop benchmark addendum; outline flop-policy amendment
2026-09-10 14:17:36 +0200 2174ccc docs: spec revision 3 after review round 2; outline amendments
2026-09-10 13:23:44 +0200 a266e87 docs: design spec draft, spec review round 1, and applied changelog
2026-09-10 12:53:35 +0200 70b887e docs: solver benchmark report on the i7-13700K (R8)
2026-09-10 12:39:59 +0200 7f42ca0 docs: PokerData verification report, outline update, gitattributes
2026-09-10 12:27:42 +0200 eda6b5c docs: research reports and design outline for the PokerAI assistant
```

Git required a command-scoped `safe.directory` override for the sandbox identity; no Git configuration was changed. Additional read-only `git show` checks established the actual file lists and approval wording, rather than relying on commit summaries alone.

The inventory and technical constants are sound, but the journal, seam-closure claims, and audit/workflow rules need the edits above before Phase A can serve as a reliable collaboration contract.

**Verdict: NOT READY**
