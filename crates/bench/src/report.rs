//! The §13.5 bench report: p50/p95/max wall time to `target_bp`, time-to-target, peak RSS, thread
//! count, cancel latency, Exact/Approximate/Unsupported proportions, and street/final deadline
//! violation counts, per suite, each section preceded by the metadata of the run that produced it.
//! Deterministic for fixed inputs: rows are appended in the order `push` receives them, formatting is
//! fixed-precision, and there is no wall-clock or randomness in `to_markdown` itself (only the
//! `SpotResult`s and the `RunMetadata` it is given carry measured or captured values).
//!
//! A statistic with no sample is `n/a` with its sample count, never a numeric zero: a zero is only
//! ever a measured zero.

use crate::runner::{ack_status_name, result_status_name, CancelOutcome, SpotResult};
use std::io::Write;
use std::path::Path;

/// The report's title, written once, after the frontmatter, on a new or empty file.
pub const TITLE: &str = "# Bench report (i7-13700K, spec 13.5)\n\n";

/// Nearest-rank percentile over an already sorted slice: `idx = ceil(len * p)` clamped to `1..=len`,
/// minus one. `p50` of `[1,2,3,4]` is therefore `2` and `p50` of `[10,20,30,40,2500]` is `30`. An
/// empty slice has no percentile and reports `0`; `to_markdown` never passes it one (it renders `n/a`
/// with the sample count instead).
pub fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[idx]
}

/// The identity of one appended run, rendered with its section so later measurements can always be
/// tied to the machine, the binaries and the configuration that produced them. Every field is text
/// the capture wrote; a value that could not be captured says so ("unavailable (...)"), never a guess.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunMetadata {
    /// When the run started, UTC (`utc_timestamp`).
    pub recorded_at: String,
    /// CPU name and logical processor count.
    pub hardware: String,
    /// The Rust toolchain resolved at run time and the `bench` binary's own target and profile.
    pub toolchain: String,
    /// The worker executable (path, size, modification time) and what is known of how it was built.
    pub worker_binary: String,
    /// The worker's `ready` line (`ready_identity`): protocol, solver commit, adapter version, threads, features.
    pub worker_ready: String,
    /// The suite's `target_bp` and how it is compared.
    pub target: String,
    /// The street and final-delivery budgets (`runner::budget_summary`).
    pub budgets: String,
    /// The suite file, its spot count, streets, templates and range source.
    pub spots: String,
}

/// The `docs/CONVENTIONS.md` frontmatter of a new report dated `date`, followed by a blank line. The
/// `related` paths are relative to the default output directory, `docs/bench/`.
pub fn frontmatter(date: &str) -> String {
    format!(
        "---\ntype: benchmark\nstatus: current\ndate: {date}\nsupersedes: none\nrelated:\n  - ../superpowers/specs/2026-09-10-pokerai-assistant-design.md\n  - ../superpowers/plans/2026-09-10-plan-2-worker-engine.md\n  - worker-toolchain.json\n---\n\n"
    )
}

/// Every field of the worker's `ready` line, in wire order.
pub fn ready_identity(ready: &proto::worker::Ready) -> String {
    format!(
        "proto_version {}, solver_commit {}, adapter_version {}, threads {}, build_features [{}], cpu_features [{}], capabilities [{}]",
        ready.proto_version,
        ready.solver_commit,
        ready.adapter_version,
        ready.threads,
        ready.build_features.join(", "),
        ready.cpu_features.join(", "),
        ready.capabilities.join(", ")
    )
}

/// `unix_secs` as an ISO 8601 UTC timestamp, `YYYY-MM-DDTHH:MM:SSZ` (the proleptic Gregorian calendar,
/// days-to-civil conversion after H. Hinnant).
pub fn utc_timestamp(unix_secs: u64) -> String {
    let (days, rem) = (unix_secs / 86_400, unix_secs % 86_400);
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rem / 3_600, rem % 3_600 / 60, rem % 60)
}

/// The cancel column: a latency only for an observed cancellation.
fn cancel_cell(cancel: Option<&CancelOutcome>) -> String {
    match cancel {
        None => "n/a".into(),
        Some(CancelOutcome::Cancelled { ack_ms, result_ms }) => format!("cancel ack {ack_ms} ms, result {result_ms} ms"),
        Some(CancelOutcome::CompletedBeforeCancel { terminal, cancel_ack: None }) => {
            format!("n/a (solve completed before a cancel could be sent: {})", result_status_name(*terminal))
        }
        Some(CancelOutcome::CompletedBeforeCancel { terminal, cancel_ack: Some(ack) }) => format!(
            "n/a (solve completed before the cancel took effect: {}, cancel ack {})",
            result_status_name(*terminal),
            ack_status_name(*ack)
        ),
    }
}

pub struct Report {
    suite: String,
    threads: u8,
    reps: u32,
    rows: Vec<SpotResult>,
    cancel: Option<CancelOutcome>,
    metadata: Option<RunMetadata>,
}

impl Report {
    pub fn new(suite: &str, threads: u8, reps: u32) -> Self {
        Self { suite: suite.into(), threads, reps, rows: vec![], cancel: None, metadata: None }
    }
    pub fn push(&mut self, r: SpotResult) {
        self.rows.push(r);
    }
    pub fn set_cancel_latency(&mut self, ack_ms: u64, result_ms: u64) {
        self.cancel = Some(CancelOutcome::Cancelled { ack_ms, result_ms });
    }
    pub fn set_cancel(&mut self, outcome: CancelOutcome) {
        self.cancel = Some(outcome);
    }
    pub fn set_metadata(&mut self, metadata: RunMetadata) {
        self.metadata = Some(metadata);
    }

    /// §13.5: the run metadata, then p50/p95/max wall, time-to-target, peak RSS, threads, coverage
    /// proportions, violation counts and cancel latency, then a per-spot detail table.
    ///
    /// Time-to-target samples are the rows that reached the raw target (`SpotResult::at_target`), with
    /// their count out of all rows; with none, the column is `n/a`. A report with no rows renders its
    /// wall, peak RSS and coverage columns as `n/a (no rows)`.
    pub fn to_markdown(&self) -> String {
        let n = self.rows.len();
        let mut walls: Vec<u64> = self.rows.iter().map(|r| r.wall_ms).collect();
        walls.sort_unstable();
        let mut at_target: Vec<u64> = self.rows.iter().filter(|r| r.at_target).map(|r| r.wall_ms).collect();
        at_target.sort_unstable();
        let no_rows = || "n/a (no rows)".to_string();
        let wall = if walls.is_empty() {
            no_rows()
        } else {
            format!("p50 {} ms / p95 {} ms / max {} ms", percentile(&walls, 0.5), percentile(&walls, 0.95), walls[walls.len() - 1])
        };
        let time_to_target = if at_target.is_empty() {
            format!("n/a (0 of {n} at target)")
        } else {
            format!("p50 {} ms / p95 {} ms ({} of {n} at target)", percentile(&at_target, 0.5), percentile(&at_target, 0.95), at_target.len())
        };
        let (peak, coverage) = if n == 0 {
            (no_rows(), no_rows())
        } else {
            let pct = |pred: &dyn Fn(&SpotResult) -> bool| 100.0 * self.rows.iter().filter(|r| pred(r)).count() as f64 / n as f64;
            let peak = self.rows.iter().map(|r| r.peak_ws_bytes).max().unwrap_or(0);
            // Classified by the raw target (`SpotResult::at_target`), never by status alone: an `ok` that
            // missed the raw target (possible when its rounded display bp still equals the target) is
            // Approximate, matching `engine::assemble::coverage_for_solve`'s literal classification. A
            // `cancelled` status no longer reaches a row (`runner::run_spot` rejects an unsolicited one).
            (
                format!("{:.1} MB", peak as f64 / 1_048_576.0),
                format!(
                    "Exact {:.1}% / Approximate {:.1}% / Unsupported {:.1}%",
                    pct(&|r| r.status == "ok" && r.at_target),
                    pct(&|r| r.status == "best_so_far" || (r.status == "ok" && !r.at_target)),
                    pct(&|r| r.status == "error")
                ),
            )
        };
        let mut md = format!("## Suite {} ({} reps)\n\n", self.suite, self.reps);
        match &self.metadata {
            Some(m) => {
                md += &format!(
                    "- run started (UTC): {}\n- hardware: {}\n- toolchain: {}\n- worker binary: {}\n- worker ready: {}\n- target: {}\n- budgets: {}\n- spots: {}\n\n",
                    m.recorded_at, m.hardware, m.toolchain, m.worker_binary, m.worker_ready, m.target, m.budgets, m.spots
                );
            }
            None => md += "- run metadata: not recorded\n\n",
        }
        md += "| suite | threads | wall p50 / p95 / max | time-to-target p50 / p95 | peak RSS | coverage | violations | cancel |\n|---|---|---|---|---|---|---|---|\n";
        md += &format!(
            "| {} | {} | {wall} | {time_to_target} | {peak} | {coverage} | street violations {}, final violations {} | {} |\n",
            self.suite,
            self.threads,
            self.rows.iter().filter(|r| r.street_violation).count(),
            self.rows.iter().filter(|r| r.final_violation).count(),
            cancel_cell(self.cancel.as_ref()),
        );
        md += "\n| spot | rep | cold | wall ms | status | reached bp | at target | iterations | memory est | peak WS | mode |\n|---|---|---|---|---|---|---|---|---|---|---|\n";
        for r in &self.rows {
            md += &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {} | {:.1} MB | {:.1} MB | {} |\n",
                r.spot,
                r.rep,
                r.cold,
                r.wall_ms,
                r.status,
                r.reached_bp.map(|b| b.to_string()).unwrap_or_else(|| "-".into()),
                r.at_target,
                r.iterations,
                r.memory_bytes as f64 / 1_048_576.0,
                r.peak_ws_bytes as f64 / 1_048_576.0,
                r.mode,
            );
        }
        md + "\n"
    }

    /// Appends this suite's section to `path`, creating its parent directories and, on a missing or empty
    /// file only, the frontmatter dated `date` and the title before it.
    ///
    /// The existing bytes are never rewritten: the file is read first, and only a missing file (`NotFound`)
    /// counts as empty; any other read error (a file that is not UTF-8, one that cannot be read) is returned
    /// before the destination is opened for writing, so it is left exactly as it was. The new text is then
    /// appended (`O_APPEND`), never written over the old.
    pub fn append_to(&self, path: &Path, date: &str) -> std::io::Result<()> {
        let existing = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e),
        };
        let mut out = String::new();
        if existing.is_empty() {
            out += &frontmatter(date);
            out += TITLE;
        }
        out += &self.to_markdown();
        if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d)?;
        }
        std::fs::OpenOptions::new().create(true).append(true).open(path)?.write_all(out.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::worker::{AckStatus, ResultStatus};

    fn row(wall: u64, status: &str, bp: u16, at_target: bool) -> crate::runner::SpotResult {
        crate::runner::SpotResult {
            spot: "s".into(),
            rep: 1,
            cold: false,
            wall_ms: wall,
            ack_ms: 1,
            status: status.into(),
            reached_bp: Some(bp),
            at_target,
            iterations: 50,
            memory_bytes: 1 << 20,
            peak_ws_bytes: 2 << 20,
            mode: "f32".into(),
            street_violation: wall > 2000 || ((status == "ok" || status == "best_so_far") && !at_target),
            final_violation: wall > 15000,
        }
    }
    /// The brief's helper: an `ok` row is at target.
    fn r(wall: u64, status: &str, bp: u16) -> crate::runner::SpotResult { row(wall, status, bp, status == "ok") }

    /// The summary row's cells: suite, threads, wall, time-to-target, peak RSS, coverage, violations, cancel.
    fn cells(md: &str, suite: &str) -> Vec<String> {
        let line = md.lines().find(|l| l.starts_with(&format!("| {suite} | "))).unwrap_or_else(|| panic!("no summary row in:\n{md}"));
        line.trim_matches('|').split(" | ").map(|c| c.trim().to_string()).collect()
    }

    #[test]
    fn percentiles_and_sections() {
        let mut rep = Report::new("river_std", 16, 5);
        for w in [10, 20, 30, 40, 2500] {
            rep.push(r(w, if w == 2500 { "best_so_far" } else { "ok" }, if w == 2500 { 90 } else { 30 }));
        }
        rep.set_cancel_latency(4, 120);
        let md = rep.to_markdown();
        assert!(md.contains("| river_std | 16 |") && md.contains("p50 30 ms") && md.contains("p95 2500 ms") && md.contains("max 2500 ms"));
        assert!(md.contains("Exact 80.0% / Approximate 20.0% / Unsupported 0.0%") && md.contains("street violations 1") && md.contains("final violations 0"));
        assert!(md.contains("cancel ack 4 ms, result 120 ms"));
        // nearest rank: idx = ceil(len * p).clamp(1, len) - 1, so p50 of [1,2,3,4] is index 1 -> 2
        assert_eq!(percentile(&[1, 2, 3, 4], 0.5), 2);
        assert_eq!(percentile(&[1, 2, 3, 4], 0.95), 4);
        assert_eq!(percentile(&[10, 20, 30, 40, 2500], 0.5), 30);
        assert_eq!(percentile(&[], 0.5), 0);
        // a suite whose spots all finished before a cancel could be issued reports n/a, never a misleading 0
        let mut none = Report::new("river_min", 16, 1);
        none.push(r(4, "ok", 10));
        assert!(none.to_markdown().contains("| n/a |"));
    }

    // ---- I6: time-to-target is n/a with the sample count when nothing reached the target ----

    #[test]
    fn time_to_target_without_samples_is_na_with_the_count_and_a_measured_zero_stays_numeric() {
        let render = |rows: Vec<crate::runner::SpotResult>| {
            let mut rep = Report::new("river_std", 16, 1);
            rows.into_iter().for_each(|x| rep.push(x));
            cells(&rep.to_markdown(), "river_std")
        };
        // all failed
        let c = render(vec![r(5, "error", 0), r(7, "error", 0), r(6, "error", 0)]);
        assert_eq!(c[3], "n/a (0 of 3 at target)");
        assert_eq!(c[2], "p50 6 ms / p95 7 ms / max 7 ms", "wall time of a failed row is still measured");
        // all best_so_far
        assert_eq!(render(vec![r(1_900, "best_so_far", 190), r(1_950, "best_so_far", 170)])[3], "n/a (0 of 2 at target)");
        // an `ok` above the raw target is not a time-to-target sample, and (N2) the coverage column
        // classifies it Approximate, by the raw target, never Exact
        let ok_above_target = render(vec![row(40, "ok", 50, false)]);
        assert_eq!(ok_above_target[3], "n/a (0 of 1 at target)");
        assert_eq!(ok_above_target[5], "Exact 0.0% / Approximate 100.0% / Unsupported 0.0%");
        // empty
        let c = render(vec![]);
        assert_eq!((c[2].as_str(), c[3].as_str(), c[4].as_str(), c[5].as_str()), ("n/a (no rows)", "n/a (0 of 0 at target)", "n/a (no rows)", "n/a (no rows)"));
        // singleton success, and a genuine measured zero
        assert_eq!(render(vec![r(7, "ok", 30)])[3], "p50 7 ms / p95 7 ms (1 of 1 at target)");
        assert_eq!(render(vec![r(0, "ok", 30)])[3], "p50 0 ms / p95 0 ms (1 of 1 at target)");
        // mixed
        let c = render(vec![r(10, "ok", 30), r(2_500, "best_so_far", 90), r(50, "error", 0), r(30, "ok", 30), r(40, "error", 0)]);
        assert_eq!(c[3], "p50 10 ms / p95 30 ms (2 of 5 at target)");
    }

    // ---- I1: the cancel cell never shows a number for anything but an observed cancellation ----

    #[test]
    fn the_cancel_cell_distinguishes_a_cancellation_from_a_completion_and_from_no_measurement() {
        let cell = |outcome: Option<CancelOutcome>| {
            let mut rep = Report::new("river_std", 16, 1);
            rep.push(r(5, "ok", 30));
            if let Some(o) = outcome { rep.set_cancel(o); }
            cells(&rep.to_markdown(), "river_std")[7].clone()
        };
        assert_eq!(cell(Some(CancelOutcome::Cancelled { ack_ms: 0, result_ms: 14 })), "cancel ack 0 ms, result 14 ms");
        assert_eq!(
            cell(Some(CancelOutcome::CompletedBeforeCancel { terminal: ResultStatus::Ok, cancel_ack: Some(AckStatus::AlreadyFinished) })),
            "n/a (solve completed before the cancel took effect: ok, cancel ack already_finished)"
        );
        assert_eq!(
            cell(Some(CancelOutcome::CompletedBeforeCancel { terminal: ResultStatus::Ok, cancel_ack: None })),
            "n/a (solve completed before a cancel could be sent: ok)"
        );
        assert_eq!(cell(None), "n/a");
    }

    // ---- I3: every appended run renders its metadata ----

    fn fixed_metadata() -> RunMetadata {
        RunMetadata {
            recorded_at: "2026-09-27T10:11:12Z".into(),
            hardware: "13th Gen Intel(R) Core(TM) i7-13700K; 24 logical processors".into(),
            toolchain: "rustc 1.95.0 (59807616e 2026-04-14), host x86_64-pc-windows-msvc".into(),
            worker_binary: "target/release/solver-worker.exe (123 bytes)".into(),
            worker_ready: "proto_version 3, solver_commit abc".into(),
            target: "50 bp".into(),
            budgets: "street 2000 ms, final delivery 15000 ms".into(),
            spots: "6 spots".into(),
        }
    }

    #[test]
    fn the_run_metadata_is_rendered_between_the_heading_and_the_summary() {
        let mut rep = Report::new("river_std", 16, 1);
        rep.push(r(5, "ok", 30));
        rep.set_metadata(fixed_metadata());
        let md = rep.to_markdown();
        let block = "- run started (UTC): 2026-09-27T10:11:12Z\n\
                     - hardware: 13th Gen Intel(R) Core(TM) i7-13700K; 24 logical processors\n\
                     - toolchain: rustc 1.95.0 (59807616e 2026-04-14), host x86_64-pc-windows-msvc\n\
                     - worker binary: target/release/solver-worker.exe (123 bytes)\n\
                     - worker ready: proto_version 3, solver_commit abc\n\
                     - target: 50 bp\n\
                     - budgets: street 2000 ms, final delivery 15000 ms\n\
                     - spots: 6 spots\n";
        let (heading, meta, summary) = (md.find("## Suite river_std").unwrap(), md.find(block).unwrap_or_else(|| panic!("no metadata block in:\n{md}")), md.find("| suite | threads |").unwrap());
        assert!(heading < meta && meta < summary, "{md}");
        // a report without captured metadata says so rather than rendering nothing
        let mut bare = Report::new("river_std", 16, 1);
        bare.push(r(5, "ok", 30));
        assert!(bare.to_markdown().contains("- run metadata: not recorded\n"));
    }

    #[test]
    fn the_ready_identity_lists_every_ready_field() {
        let ready = proto::worker::Ready {
            proto_version: 3,
            solver_commit: "9d1509fe5077d019825f833eed04b16d342dfda1".into(),
            adapter_version: 1,
            threads: 16,
            build_features: vec!["avx2".into()],
            cpu_features: vec!["avx2".into(), "fma".into()],
            capabilities: vec!["solve".into(), "lock".into()],
        };
        assert_eq!(
            ready_identity(&ready),
            "proto_version 3, solver_commit 9d1509fe5077d019825f833eed04b16d342dfda1, adapter_version 1, threads 16, build_features [avx2], cpu_features [avx2, fma], capabilities [solve, lock]"
        );
    }

    #[test]
    fn utc_timestamps_are_iso_8601() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_timestamp(1_790_510_672), "2026-09-27T12:04:32Z");
    }

    // ---- I7 and M1: appending never destroys an existing report; the frontmatter is written once ----

    struct TempDir(std::path::PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("bench-report-{name}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    fn section() -> Report {
        let mut rep = Report::new("river_std", 16, 1);
        rep.push(r(5, "ok", 30));
        rep
    }

    fn assert_fresh_report(text: &str) {
        let head = "---\ntype: benchmark\nstatus: current\ndate: 2026-09-10\nsupersedes: none\nrelated:\n  - ";
        assert!(text.starts_with(head), "frontmatter missing or malformed:\n{text}");
        let close = text[4..].find("\n---\n").map(|i| i + 4).expect("the frontmatter block is closed");
        assert!(text[close..].starts_with("\n---\n\n# Bench report (i7-13700K, spec 13.5)\n\n## Suite river_std"), "{text}");
    }

    #[test]
    fn a_missing_report_is_created_with_frontmatter_title_and_section() {
        let dir = TempDir::new("missing");
        let path = dir.0.join("sub").join("2026-09-10-i7-13700K.md");
        section().append_to(&path, "2026-09-10").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_fresh_report(&text);
        // a second append adds its section and never a second frontmatter or title
        section().append_to(&path, "2026-09-10").unwrap();
        let again = std::fs::read_to_string(&path).unwrap();
        assert!(again.starts_with(&text), "the first append's bytes are kept");
        assert_eq!(again.matches("type: benchmark").count(), 1);
        assert_eq!(again.matches("# Bench report").count(), 1);
        assert_eq!(again.matches("## Suite river_std").count(), 2);
    }

    #[test]
    fn an_empty_report_file_gets_the_frontmatter_once() {
        let dir = TempDir::new("empty");
        let path = dir.0.join("2026-09-10-i7-13700K.md");
        std::fs::write(&path, b"").unwrap();
        section().append_to(&path, "2026-09-10").unwrap();
        assert_fresh_report(&std::fs::read_to_string(&path).unwrap());
    }

    #[test]
    fn an_existing_report_keeps_its_bytes_and_gets_no_second_frontmatter() {
        let dir = TempDir::new("valid");
        let path = dir.0.join("2026-09-10-i7-13700K.md");
        let old = "---\ntype: benchmark\n---\n\n# Bench report\n\nold section\n";
        std::fs::write(&path, old).unwrap();
        section().append_to(&path, "2026-09-10").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(old), "{text}");
        assert!(text[old.len()..].starts_with("## Suite river_std"), "{text}");
        assert_eq!(text.matches("type: benchmark").count(), 1);
    }

    #[test]
    fn an_unreadable_report_is_an_error_and_its_bytes_are_preserved() {
        let dir = TempDir::new("non-utf8");
        let path = dir.0.join("2026-09-10-i7-13700K.md");
        let old = [0xff, 0xfe, 0x41, 0x00];
        std::fs::write(&path, old).unwrap();
        let err = section().append_to(&path, "2026-09-10").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData, "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), old, "the original bytes must survive the failed append");
    }
}
