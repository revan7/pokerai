//! Decision log (spec section 5 step 10): every request and its `Final` is appended to
//! `%LOCALAPPDATA%\PokerAI\decisions.jsonl`, rotated at 50 MiB, 10 files; a write failure is
//! logged once per session rather than surfaced as an error to the caller (the log is diagnostic,
//! never load-bearing for a decision).
//!
//! `EngineCore` (Task 22) owns a `DecisionLog` as its fourth constructor argument, so this module
//! is built first and its public shape does not change afterwards (cross-plan section 4).
//!
//! Every field on `DecisionRecord`/`InputRecord` is either a plain integer/bool/String with no
//! numeric domain of its own, or a nested `proto` type (`DecisionIdentity`, `Street`, `Coverage`,
//! `ApproxReason`, `HandConfig`, `Seat`, `Card`, `TakenAction`) that already carries its own
//! invariant-checked `Serialize`/`Deserialize` (finite-float domains, card validity, and so on).
//! Neither record adds a numeric or cross-field invariant beyond what those nested types already
//! enforce, so both use a bare derive here, matching every other container type in `proto` (e.g.
//! `Recommendation`, `Assumptions`) that wraps invariant-bearing fields without re-deriving their
//! checks at the container level.

use proto::{ApproxReason, Card, Coverage, DecisionIdentity, HandConfig, HandState, Seat, Street, TakenAction};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Rotate `decisions.jsonl` once it reaches this size (spec section 5 step 10).
pub const ROTATE_BYTES: u64 = 50 << 20;
/// Keep at most this many files (the live `decisions.jsonl` plus `decisions.N.jsonl` backlog).
pub const KEEP_FILES: usize = 10;
/// Current `InputRecord` version. Bumped whenever the record's shape changes.
pub const INPUT_RECORD_VERSION: u16 = 1;

/// Versioned input record (spec section 5 step 10: "versioned input record"). Hero's cards are
/// recorded here, and only here -- never derived into any other part of the record -- per the
/// standing rule that hero's actual cards never enter a public range, solve input or cache key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    pub version: u16,
    pub config: HandConfig,
    pub button: Seat,
    pub hero: Seat,
    pub dealt: Vec<Seat>,
    pub stacks_start: Vec<u32>,
    pub hero_cards: Option<[Card; 2]>,
    pub actions: Vec<TakenAction>,
    pub board: Vec<Card>,
    pub range_hashes: Vec<String>,
}

impl InputRecord {
    pub fn from_state(s: &HandState, range_hashes: Vec<String>) -> Self {
        Self {
            version: INPUT_RECORD_VERSION,
            config: s.config.clone(),
            button: s.button,
            hero: s.hero,
            dealt: s.dealt.clone(),
            stacks_start: s.stacks_start.clone(),
            hero_cards: s.hero_cards,
            actions: s.actions.clone(),
            board: s.board.clone(),
            range_hashes,
        }
    }
}

/// One logged decision (spec section 5 step 10): identity, coverage, reasons, elapsed time, cache
/// result with the pre-solver scenario class and tier when the decision matches one (both `None`
/// otherwise), reached exploitability of a best-so-far, deadline violations, and the versioned
/// input record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub identity: DecisionIdentity,
    pub street: Street,
    pub coverage: Coverage,
    pub reasons: Vec<ApproxReason>,
    pub elapsed_ms: u32,
    pub cache: String,
    pub presolver_scenario: Option<String>,
    pub tier: Option<u8>,
    pub reached_bp: Option<u16>,
    pub street_violation: bool,
    pub final_violation: bool,
    pub template_id: String,
    pub input: InputRecord,
}

/// Appends `DecisionRecord`s as newline-delimited JSON (LF only), rotating `decisions.jsonl` at
/// `rotate_bytes` and keeping at most `keep` files. Append-only: an existing line is never
/// rewritten, only rotated whole-file.
pub struct DecisionLog {
    dir: PathBuf,
    rotate_bytes: u64,
    keep: usize,
    failed_once: bool,
}

impl DecisionLog {
    /// Opens the log at `dir` with the spec's defaults (`ROTATE_BYTES`, `KEEP_FILES`).
    pub fn open(dir: &Path) -> Self {
        Self::with_limits(dir, ROTATE_BYTES, KEEP_FILES)
    }

    /// Opens the log at `dir` with explicit rotation limits (used by tests to force rotation
    /// quickly without waiting to accumulate 50 MiB). `keep` must be at least 1 -- the live
    /// `decisions.jsonl` always counts as one of the kept files, and `rotate` below indexes
    /// `self.keep - 1` -- so this infallible constructor enforces it with an always-on `assert!`
    /// rather than a `debug_assert!` or a silently clamped value (standing ruling: internal
    /// constructors with infallible signatures assert their invariants, naming the offending
    /// value, rather than clamp).
    pub fn with_limits(dir: &Path, rotate_bytes: u64, keep: usize) -> Self {
        assert!(keep >= 1, "DecisionLog::with_limits: keep must be at least 1, got {keep}");
        Self { dir: dir.to_path_buf(), rotate_bytes, keep, failed_once: false }
    }

    fn path(&self, k: usize) -> PathBuf {
        if k == 0 { self.dir.join("decisions.jsonl") } else { self.dir.join(format!("decisions.{k}.jsonl")) }
    }

    /// Shifts `decisions.{k}.jsonl` -> `decisions.{k+1}.jsonl` for every kept file, dropping
    /// whatever already occupies the last slot, then leaves slot 0 (`decisions.jsonl`) free for
    /// `append` to recreate. Never rewrites the content of an existing line -- only whole files
    /// move.
    fn rotate(&self) -> std::io::Result<()> {
        let _ = std::fs::remove_file(self.path(self.keep - 1));
        for k in (1..self.keep).rev() {
            let from = self.path(k - 1);
            if from.exists() {
                std::fs::rename(&from, self.path(k))?;
            }
        }
        Ok(())
    }

    /// Appends one record as a single JSON line (LF-terminated only -- `write_all(b"\n")` never
    /// depends on a platform line-ending convention), rotating first if the live file is already
    /// at or over the rotation threshold. A write failure never propagates to the caller (the log
    /// is diagnostic, not load-bearing for a decision); the first failure of this `DecisionLog`
    /// instance is reported to stderr, and every failure after that in the same session is
    /// silent.
    pub fn append(&mut self, rec: &DecisionRecord) {
        let result = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&self.dir)?;
            let current = self.path(0);
            if current.exists() && std::fs::metadata(&current)?.len() >= self.rotate_bytes {
                self.rotate()?;
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&current)?;
            f.write_all(serde_json::to_string(rec).map_err(std::io::Error::other)?.as_bytes())?;
            f.write_all(b"\n")
        })();
        if let Err(e) = result {
            if !self.failed_once {
                eprintln!("decision log write failed (further failures this session are not reported): {e}");
                self.failed_once = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn hand_config() -> proto::HandConfig {
        proto::HandConfig { config_revision: 1, sb_chips: 5, bb_chips: 10, straddle: None,
            rake: proto::Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false }, chip_label: "$1".into() }
    }
    fn rec(i: u64) -> DecisionRecord {
        let hc = hand_config();
        DecisionRecord { identity: proto::DecisionIdentity { hand_id: i, hand_revision: 1, decision_id: i, config_revision: 1, model_revision: 0 }, street: proto::Street::River, coverage: proto::Coverage::Exact, reasons: vec![], elapsed_ms: 12, cache: "miss".into(), presolver_scenario: None, tier: None, reached_bp: Some(30), street_violation: false, final_violation: false, template_id: "river_std_v1".into(),
            input: InputRecord { version: 1, config: hc, button: proto::Seat(0), hero: proto::Seat(2), dealt: vec![], stacks_start: vec![], hero_cards: None, actions: vec![], board: vec![], range_hashes: vec!["x".repeat(64)] } }
    }
    #[test]
    fn appends_jsonl_and_rotates() {
        let dir = std::env::temp_dir().join(format!("pokerai_log_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = DecisionLog::with_limits(&dir, 2_000, 3);
        for i in 0..40 { log.append(&rec(i)); }
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"decisions.jsonl".to_string()) && names.contains(&"decisions.1.jsonl".to_string()));
        assert!(names.len() <= 3, "{names:?}");
        let first = std::fs::read_to_string(dir.join("decisions.jsonl")).unwrap();
        let last: DecisionRecord = serde_json::from_str(first.lines().last().unwrap()).unwrap();
        assert_eq!(last.identity.decision_id, 39);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// `InputRecord::from_state` is the one other behavior this task's brief lists under
    /// "Produces" that the append/rotate test above never exercises (it builds `InputRecord`
    /// literals directly). CLAUDE.md section 5 requires TDD for any task that changes code
    /// behavior, so this is written and run RED before `from_state` exists.
    #[test]
    fn from_state_maps_fields_and_keeps_hero_cards_only_there() {
        let cards = [proto::Card::new(0, 0), proto::Card::new(1, 1)];
        let state = proto::HandState {
            hand_id: 7,
            hand_revision: 1,
            config: hand_config(),
            phase: proto::HandPhase::Betting { street: proto::Street::Preflop },
            button: proto::Seat(0),
            hero: proto::Seat(1),
            hero_cards: Some(cards),
            dealt: vec![proto::Seat(0), proto::Seat(1)],
            stacks_start: vec![100, 100],
            board: vec![],
            actions: vec![],
            derived: proto::Derived::default(),
        };
        let input = InputRecord::from_state(&state, vec!["h".repeat(64)]);
        assert_eq!(input.version, 1);
        assert_eq!(input.hero, proto::Seat(1));
        assert_eq!(input.button, proto::Seat(0));
        assert_eq!(input.hero_cards, Some(cards));
        assert_eq!(input.stacks_start, vec![100, 100]);
        assert_eq!(input.range_hashes, vec!["h".repeat(64)]);

        // Round-trips through JSON with the hero cards intact -- the one place they are allowed
        // to appear (spec section 2: never in a public range, solve input or cache key).
        let text = serde_json::to_string(&input).unwrap();
        let back: InputRecord = serde_json::from_str(&text).unwrap();
        assert_eq!(back, input);
    }
}
