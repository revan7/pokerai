//! The §13.5 bench report: p50/p95/max wall time to `target_bp`, time-to-target, peak RSS, thread
//! count, cancel latency, Exact/Approximate/Unsupported proportions, and street/final deadline
//! violation counts, per suite. Deterministic for fixed inputs: rows are appended in the order
//! `push` receives them, formatting is fixed-precision, and there is no wall-clock or randomness in
//! `to_markdown` itself (only the `SpotResult`s it is given carry the measured wall-clock numbers).

use crate::runner::SpotResult;
use std::path::Path;

/// Nearest-rank percentile over an already sorted slice: `idx = ceil(len * p)` clamped to `1..=len`,
/// minus one. `p50` of `[1,2,3,4]` is therefore `2` and `p50` of `[10,20,30,40,2500]` is `30`. An
/// empty slice has no percentile and reports `0` (the caller distinguishes "no rows" from "reported
/// zero" separately, e.g. cancel latency's `n/a`).
pub fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() as f64 * p).ceil() as usize).clamp(1, sorted.len()) - 1;
    sorted[idx]
}

pub struct Report {
    suite: String,
    threads: u8,
    reps: u32,
    rows: Vec<SpotResult>,
    cancel: Option<(u64, u64)>,
}

impl Report {
    pub fn new(suite: &str, threads: u8, reps: u32) -> Self {
        Self { suite: suite.into(), threads, reps, rows: vec![], cancel: None }
    }
    pub fn push(&mut self, r: SpotResult) {
        self.rows.push(r);
    }
    pub fn set_cancel_latency(&mut self, ack_ms: u64, result_ms: u64) {
        self.cancel = Some((ack_ms, result_ms));
    }

    /// §13.5: p50/p95/max wall to target, time-to-target, peak RSS, threads, cancel latency,
    /// coverage proportions, violation counts, then a per-spot detail table.
    pub fn to_markdown(&self) -> String {
        let mut walls: Vec<u64> = self.rows.iter().map(|r| r.wall_ms).collect();
        walls.sort_unstable();
        let at_target: Vec<u64> = {
            let mut v: Vec<u64> = self.rows.iter().filter(|r| r.status == "ok").map(|r| r.wall_ms).collect();
            v.sort_unstable();
            v
        };
        let n = self.rows.len().max(1) as f64;
        let pct = |s: &str| 100.0 * self.rows.iter().filter(|r| r.status == s).count() as f64 / n;
        let unsupported = 100.0 * self.rows.iter().filter(|r| r.status == "error" || r.status == "cancelled").count() as f64 / n;
        let peak = self.rows.iter().map(|r| r.peak_ws_bytes).max().unwrap_or(0);
        let mut md = format!(
            "## Suite {} ({} reps)\n\n| suite | threads | wall p50 / p95 / max | time-to-target p50 / p95 | peak RSS | coverage | violations | cancel |\n|---|---|---|---|---|---|---|---|\n",
            self.suite, self.reps
        );
        md += &format!(
            "| {} | {} | p50 {} ms / p95 {} ms / max {} ms | p50 {} ms / p95 {} ms | {:.1} MB | Exact {:.1}% / Approximate {:.1}% / Unsupported {:.1}% | street violations {}, final violations {} | {} |\n",
            self.suite,
            self.threads,
            percentile(&walls, 0.5),
            percentile(&walls, 0.95),
            walls.last().copied().unwrap_or(0),
            percentile(&at_target, 0.5),
            percentile(&at_target, 0.95),
            peak as f64 / 1_048_576.0,
            pct("ok"),
            pct("best_so_far"),
            unsupported,
            self.rows.iter().filter(|r| r.street_violation).count(),
            self.rows.iter().filter(|r| r.final_violation).count(),
            self.cancel.map(|(a, r)| format!("cancel ack {a} ms, result {r} ms")).unwrap_or_else(|| "n/a".into()),
        );
        md += "\n| spot | rep | cold | wall ms | status | reached bp | iterations | memory est | peak WS | mode |\n|---|---|---|---|---|---|---|---|---|---|\n";
        for r in &self.rows {
            md += &format!(
                "| {} | {} | {} | {} | {} | {} | {} | {:.1} MB | {:.1} MB | {} |\n",
                r.spot,
                r.rep,
                r.cold,
                r.wall_ms,
                r.status,
                r.reached_bp.map(|b| b.to_string()).unwrap_or_else(|| "-".into()),
                r.iterations,
                r.memory_bytes as f64 / 1_048_576.0,
                r.peak_ws_bytes as f64 / 1_048_576.0,
                r.mode,
            );
        }
        md + "\n"
    }

    /// Appends this suite's section to `path`, creating its parent directories and, on a fresh file,
    /// the §13.5 report header, then the file's existing bytes unchanged before the new section.
    pub fn append_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let existing = std::fs::read_to_string(path).unwrap_or_default();
        let header = if existing.is_empty() { "# Bench report (i7-13700K, spec 13.5)\n\n" } else { "" };
        std::fs::write(path, format!("{existing}{header}{}", self.to_markdown()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn r(wall: u64, status: &str, bp: u16) -> crate::runner::SpotResult {
        crate::runner::SpotResult {
            spot: "s".into(),
            rep: 1,
            cold: false,
            wall_ms: wall,
            ack_ms: 1,
            status: status.into(),
            reached_bp: Some(bp),
            iterations: 50,
            memory_bytes: 1 << 20,
            peak_ws_bytes: 2 << 20,
            mode: "f32".into(),
            street_violation: wall > 2000,
            final_violation: wall > 15000,
        }
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
}
