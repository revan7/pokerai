//! Decision log (spec section 5 step 10): every request and its `Final` is appended to
//! `%LOCALAPPDATA%\PokerAI\decisions.jsonl`, rotated at 50 MiB, 10 files; a write failure is
//! logged once per session rather than surfaced as an error to the caller (the log is diagnostic,
//! never load-bearing for a decision).
//!
//! `EngineCore` (Task 22) owns a `DecisionLog` as its fourth constructor argument, so this module
//! is built first and its public shape does not change afterwards (cross-plan section 4).
//!
//! Diagnostics (final review M2; spec 3.4, the worker's stderr is drained to the log; spec 12, a
//! `tree_mismatch` is logged with both trees). Beside the decisions, the same directory keeps
//! `diagnostics.jsonl` (rotated the same way): one `DiagnosticRecord` for every kill and restart
//! of the worker the engine makes, with its cause and the worker's bounded stderr tail, for every
//! `tree_mismatch`, with the engine's tree, for a panic `engine-main` contained, and (plan 4 Task
//! 10) for every live solution the cache refused to store (`cache_reject`). Nothing of these paths
//! goes to the process's stderr, which a windowed app does not show, except the first refused cache
//! entry of a session, which `EngineCore::log_cache_reject` also reports there once.
//!
//! `DecisionRecord::cache` is the delivered `Final`'s `assumptions.cache` (`miss`, `exact`,
//! `approximate` or `provisional`, spec 4.4), plan 4 Task 10's cache result of the decision.
//!
//! Most fields on `DecisionRecord`/`InputRecord` are plain integer/bool/`String`s with no numeric
//! domain of their own, or a nested `proto` type (`DecisionIdentity`, `Street`, `Coverage`,
//! `ApproxReason`, `HandConfig`, `Seat`, `Card`, `TakenAction`) that already carries its own
//! invariant-checked `Serialize`/`Deserialize` (finite-float domains, card validity, and so on).
//! Two invariants belong to these records themselves, though, and review round 1 (R1/R2) found
//! both unchecked in a first bare-derive pass:
//!
//! - `InputRecord::version`: only version 1 exists. The field keeps its bare `u16` type but
//!   carries a `#[serde(with = "input_record_version")]` codec -- the same "check on both serde
//!   directions" shape `proto::numeric` uses for its checked `f32` fields, just for an exact
//!   integer domain (`== 1`) rather than a float range.
//! - `DecisionRecord::{presolver_scenario, tier}`: one unit (spec section 5 step 10; tiers 1/2/3
//!   of spec section 10.5) -- both present or both absent, and `tier` in `{1, 2, 3}` when
//!   present. No single field's codec can express a two-field pairing, so `DecisionRecord` -- and
//!   only it, among the types here -- has a hand-written `Serialize`/`Deserialize` that checks the
//!   pair before delegating every field to a bare-derived wire mirror, `DecisionRecordWire`: the
//!   same "derive normally, add a validating wrapper for what the derive cannot express"
//!   structure `proto::Action`/`proto::ApproxReason` already use, minus their bincode-tag concern
//!   (this module only ever writes JSON, so there is no `is_human_readable` branch to make).
//!
//! Every other field, on both records, still has no domain of its own beyond what its type
//! already enforces, so it keeps a bare derive; `InputRecord` as a whole is still bare-derived,
//! with only its one invariant-bearing field carrying a codec, exactly like `proto::Assumptions`
//! or `proto::ExperimentalHu`.
//!
//! Neither the `u16` version check nor the `u8` tier-domain check needs the wide-`f64`-then-
//! narrow dance `proto::numeric` uses for floats: an integer's `Deserialize` is already exact (a
//! negative or oversized wire value is rejected outright, never silently rounded or truncated
//! into range the way `f64 -> f32` narrowing can round a near-boundary value inward), so the
//! domain check here only has to run after that already-exact deserialize.

use proto::{ApproxReason, Card, Coverage, DecisionIdentity, EffectiveTree, HandConfig, HandState, Seat, Street, TakenAction};
use serde::de::Error as DeError;
use serde::ser::Error as SerError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Rotate `decisions.jsonl` once it reaches this size (spec section 5 step 10).
pub const ROTATE_BYTES: u64 = 50 << 20;
/// Keep at most this many files (the live `decisions.jsonl` plus `decisions.N.jsonl` backlog).
pub const KEEP_FILES: usize = 10;
/// Current `InputRecord` version. Bumped whenever the record's shape changes.
pub const INPUT_RECORD_VERSION: u16 = 1;

/// Only version 1 of `InputRecord` exists today. A wire value other than 1 -- including one that
/// does not even fit a `u16` -- is rejected outright rather than accepted under the version-1
/// layout; the same check runs on serialize, so an in-memory record built with an unsupported
/// version (the field is `pub`, so `from_state` is not the only way to construct one) can never
/// reach disk either.
mod input_record_version {
    use super::{DeError, Deserialize, Deserializer, SerError, Serializer, INPUT_RECORD_VERSION as SUPPORTED};

    pub fn serialize<S: Serializer>(v: &u16, s: S) -> Result<S::Ok, S::Error> {
        if *v != SUPPORTED {
            return Err(SerError::custom(format!("InputRecord.version {v} is not the supported version {SUPPORTED}")));
        }
        s.serialize_u16(*v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
        let v = u16::deserialize(d)?;
        if v != SUPPORTED {
            return Err(DeError::custom(format!("InputRecord.version {v} is not the supported version {SUPPORTED}")));
        }
        Ok(v)
    }
}

/// Versioned input record (spec section 5 step 10: "versioned input record"). Hero's cards are
/// recorded here, and only here -- never derived into any other part of the record -- per the
/// standing rule that hero's actual cards never enter a public range, solve input or cache key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    #[serde(with = "input_record_version")]
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
///
/// `Serialize`/`Deserialize` are hand-written (see the module doc comment): `presolver_scenario`
/// and `tier` must be validated as a pair, which no per-field codec can express, so this delegates
/// every field to the bare-derived `DecisionRecordWire` mirror below and checks the pair itself
/// before returning `Ok`.
#[derive(Debug, Clone, PartialEq)]
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

/// Identical field-for-field shape to `DecisionRecord`, kept private and bare-derived purely so
/// the hand-written impls above can reuse serde's normal struct machinery for every field except
/// the one pairing check it cannot express (see the module doc comment).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DecisionRecordWire {
    identity: DecisionIdentity,
    street: Street,
    coverage: Coverage,
    reasons: Vec<ApproxReason>,
    elapsed_ms: u32,
    cache: String,
    presolver_scenario: Option<String>,
    tier: Option<u8>,
    reached_bp: Option<u16>,
    street_violation: bool,
    final_violation: bool,
    template_id: String,
    input: InputRecord,
}

impl From<DecisionRecord> for DecisionRecordWire {
    fn from(r: DecisionRecord) -> Self {
        DecisionRecordWire {
            identity: r.identity,
            street: r.street,
            coverage: r.coverage,
            reasons: r.reasons,
            elapsed_ms: r.elapsed_ms,
            cache: r.cache,
            presolver_scenario: r.presolver_scenario,
            tier: r.tier,
            reached_bp: r.reached_bp,
            street_violation: r.street_violation,
            final_violation: r.final_violation,
            template_id: r.template_id,
            input: r.input,
        }
    }
}

impl From<DecisionRecordWire> for DecisionRecord {
    fn from(w: DecisionRecordWire) -> Self {
        DecisionRecord {
            identity: w.identity,
            street: w.street,
            coverage: w.coverage,
            reasons: w.reasons,
            elapsed_ms: w.elapsed_ms,
            cache: w.cache,
            presolver_scenario: w.presolver_scenario,
            tier: w.tier,
            reached_bp: w.reached_bp,
            street_violation: w.street_violation,
            final_violation: w.final_violation,
            template_id: w.template_id,
            input: w.input,
        }
    }
}

/// The tiers spec section 10.5 defines for the pre-solver queue (tier 1, tier 2, tier 3).
const VALID_TIERS: [u8; 3] = [1, 2, 3];

/// `presolver_scenario` and `tier` are one unit: both present when the decision matches a
/// pre-solver scenario and tier, both absent otherwise (spec section 5 step 10), and when present
/// `tier` is one of `VALID_TIERS`. Shared by `DecisionRecord`'s `Serialize` and `Deserialize` so
/// the same rule runs, worded the same way, on both directions.
fn check_presolver_metadata(scenario: &Option<String>, tier: Option<u8>) -> Result<(), String> {
    match (scenario, tier) {
        (None, None) => Ok(()),
        (Some(_), Some(t)) if VALID_TIERS.contains(&t) => Ok(()),
        (Some(_), Some(t)) => {
            Err(format!("tier {t} is outside the tiers spec section 10.5 defines {VALID_TIERS:?}"))
        }
        (scenario, tier) => Err(format!(
            "presolver_scenario and tier must both be present or both be absent, got scenario={scenario:?} tier={tier:?}"
        )),
    }
}

impl Serialize for DecisionRecord {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        check_presolver_metadata(&self.presolver_scenario, self.tier).map_err(SerError::custom)?;
        DecisionRecordWire::from(self.clone()).serialize(s)
    }
}

impl<'de> Deserialize<'de> for DecisionRecord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = DecisionRecordWire::deserialize(d)?;
        check_presolver_metadata(&wire.presolver_scenario, wire.tier).map_err(DeError::custom)?;
        Ok(wire.into())
    }
}

/// One diagnostics record (final review M2): what the engine did to the worker, or saw go wrong,
/// and why, with what it knew then. `diagnostics.jsonl` beside the decisions; see the module doc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiagnosticRecord {
    /// Engine-clock milliseconds when it happened.
    pub at_ms: u64,
    /// `kill`, `restart`, `tree_mismatch`, `panic` or `cache_reject`.
    pub event: String,
    /// The decision concerned, when one is.
    pub identity: Option<DecisionIdentity>,
    /// What happened and why, in words: the cause, and a restart's outcome (`restarted`, or `restart failed: ..`); for a
    /// `tree_mismatch`, the worker's report of where the library's realized tree first differs from the engine's.
    pub detail: String,
    /// The worker's stderr output at that point, bounded by the link's ring (§3.4, 64 KiB); empty when the link has
    /// none.
    pub stderr_tail: String,
    /// A `tree_mismatch`'s engine tree, its materialized nodes included (spec 12: logged with both trees; the library's
    /// side is the worker's, in `detail` and on its stderr).
    pub engine_tree: Option<EffectiveTree>,
}

/// The two record files of a log directory, each rotated on its own (`decisions`, `diagnostics`).
#[derive(Clone, Copy)]
enum Stem { Decisions, Diagnostics }

impl Stem {
    fn name(self) -> &'static str { match self { Stem::Decisions => "decisions", Stem::Diagnostics => "diagnostics" } }
}

/// Appends `DecisionRecord`s as newline-delimited JSON (LF only), rotating `decisions.jsonl` at
/// `rotate_bytes` and keeping at most `keep` files, and `DiagnosticRecord`s to
/// `diagnostics.jsonl` the same way. Append-only: an existing line is never rewritten, only
/// rotated whole-file.
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

    fn path_of(&self, stem: Stem, k: usize) -> PathBuf {
        let stem = stem.name();
        if k == 0 { self.dir.join(format!("{stem}.jsonl")) } else { self.dir.join(format!("{stem}.{k}.jsonl")) }
    }

    /// Shifts `decisions.{k}.jsonl` -> `decisions.{k+1}.jsonl` for every kept file, dropping
    /// whatever already occupies the last slot, then leaves slot 0 (`decisions.jsonl`) free for
    /// `append` to recreate. Never rewrites the content of an existing line -- only whole files
    /// move.
    ///
    /// The dropped slot's removal ignores only `ErrorKind::NotFound` (the common case: nothing
    /// occupies that slot yet) and propagates every other error -- in particular a sharing
    /// violation from another handle that permits read/write but denies delete. Swallowing that
    /// unconditionally (as a first pass did) would let `rotate` report success while the
    /// oversized file it was supposed to remove is still sitting there, so the caller's `append`
    /// would go on to write past the rotation threshold indefinitely.
    fn rotate(&self, stem: Stem) -> std::io::Result<()> {
        match std::fs::remove_file(self.path_of(stem, self.keep - 1)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        for k in (1..self.keep).rev() {
            let from = self.path_of(stem, k - 1);
            if from.exists() {
                std::fs::rename(&from, self.path_of(stem, k))?;
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
        let line = serde_json::to_string(rec).map_err(std::io::Error::other);
        self.write_line(Stem::Decisions, line);
    }

    /// Appends one diagnostics record to `diagnostics.jsonl` (final review M2), rotated and reported
    /// on failure exactly as `append` is.
    pub fn diagnostic(&mut self, rec: &DiagnosticRecord) {
        let line = serde_json::to_string(rec).map_err(std::io::Error::other);
        self.write_line(Stem::Diagnostics, line);
    }

    /// One JSON line appended to `stem`'s live file, rotating it first once it is at or over the
    /// threshold. A write failure never propagates (the log is diagnostic); the first of the session
    /// is reported to stderr, the only place left to report the log's own directory failing.
    fn write_line(&mut self, stem: Stem, line: std::io::Result<String>) {
        let result = (|| -> std::io::Result<()> {
            let line = line?;
            std::fs::create_dir_all(&self.dir)?;
            let current = self.path_of(stem, 0);
            if current.exists() && std::fs::metadata(&current)?.len() >= self.rotate_bytes {
                self.rotate(stem)?;
            }
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&current)?;
            f.write_all(line.as_bytes())?;
            f.write_all(b"\n")
        })();
        if let Err(e) = result {
            if !self.failed_once {
                eprintln!("{} log write failed (further failures this session are not reported): {e}", stem.name());
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

    /// A per-test temp directory (test name + pid); every test using this removes it both before
    /// (a previous run may have crashed mid-test) and after it runs.
    fn unique_temp_dir(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("pokerai_log_{test_name}_{}", std::process::id()))
    }

    fn valid_input_record() -> InputRecord {
        InputRecord {
            version: 1,
            config: hand_config(),
            button: proto::Seat(0),
            hero: proto::Seat(2),
            dealt: vec![],
            stacks_start: vec![],
            hero_cards: None,
            actions: vec![],
            board: vec![],
            range_hashes: vec!["x".repeat(64)],
        }
    }

    /// R1 (review round 1): only version 1 exists; every other representable value -- including
    /// negative and oversized wire numbers that do not even fit a `u16` -- must be rejected on
    /// both serialize and deserialize, never converted or clamped into 1.
    #[test]
    fn input_record_version_is_validated_on_both_serde_directions() {
        let mut bad = valid_input_record();
        bad.version = 0;
        assert!(serde_json::to_string(&bad).is_err(), "version 0 must be rejected on serialize");
        bad.version = 2;
        assert!(serde_json::to_string(&bad).is_err(), "version 2 must be rejected on serialize");

        let good = valid_input_record();
        let text = serde_json::to_string(&good).unwrap();
        assert_eq!(serde_json::from_str::<InputRecord>(&text).unwrap(), good);

        let mut value = serde_json::to_value(&good).unwrap();
        for bad_version in [serde_json::json!(0), serde_json::json!(2), serde_json::json!(-1), serde_json::json!(70_000)] {
            value["version"] = bad_version.clone();
            assert!(
                serde_json::from_value::<InputRecord>(value.clone()).is_err(),
                "version {bad_version} must be rejected on deserialize"
            );
        }
    }

    /// R2 (review round 1): `presolver_scenario`/`tier` are one unit (spec section 5 step 10,
    /// section 10.5's tiers 1/2/3) -- both present or both absent, and `tier` in `{1, 2, 3}` when
    /// present -- on both serialize and deserialize.
    #[test]
    fn decision_record_presolver_metadata_is_validated_on_both_serde_directions() {
        let both_absent = rec(1);
        let text = serde_json::to_string(&both_absent).unwrap();
        assert_eq!(serde_json::from_str::<DecisionRecord>(&text).unwrap(), both_absent);

        let mut both_present = rec(2);
        both_present.presolver_scenario = Some("srp_btn_open_bb_call".into());
        both_present.tier = Some(1);
        let text = serde_json::to_string(&both_present).unwrap();
        assert_eq!(serde_json::from_str::<DecisionRecord>(&text).unwrap(), both_present);

        let mut scenario_only = rec(3);
        scenario_only.presolver_scenario = Some("srp_btn_open_bb_call".into());
        assert!(serde_json::to_string(&scenario_only).is_err(), "scenario without a tier must be rejected");

        let mut tier_only = rec(4);
        tier_only.tier = Some(1);
        assert!(serde_json::to_string(&tier_only).is_err(), "tier without a scenario must be rejected");

        for bad_tier in [0u8, 4, 255] {
            let mut invalid_tier = rec(5);
            invalid_tier.presolver_scenario = Some("srp_btn_open_bb_call".into());
            invalid_tier.tier = Some(bad_tier);
            assert!(serde_json::to_string(&invalid_tier).is_err(), "tier {bad_tier} must be rejected");
        }

        let base = serde_json::to_value(&both_present).unwrap();

        let mut missing_tier = base.clone();
        missing_tier["tier"] = serde_json::Value::Null;
        assert!(
            serde_json::from_value::<DecisionRecord>(missing_tier).is_err(),
            "scenario without a tier must be rejected on deserialize"
        );

        let mut missing_scenario = base.clone();
        missing_scenario["presolver_scenario"] = serde_json::Value::Null;
        assert!(
            serde_json::from_value::<DecisionRecord>(missing_scenario).is_err(),
            "tier without a scenario must be rejected on deserialize"
        );

        for bad_tier in [serde_json::json!(0), serde_json::json!(4), serde_json::json!(255), serde_json::json!(-1)] {
            let mut invalid = base.clone();
            invalid["tier"] = bad_tier.clone();
            assert!(
                serde_json::from_value::<DecisionRecord>(invalid).is_err(),
                "wire tier {bad_tier} must be rejected on deserialize"
            );
        }
    }

    /// R3 (review round 1): with `keep = 1` the sole rotation step deletes `decisions.jsonl`
    /// itself (the rename loop is empty); this is the intended, successful case.
    #[test]
    fn keep_one_rotation_replaces_the_single_file() {
        let dir = unique_temp_dir("keep_one_rotation_replaces_the_single_file");
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = DecisionLog::with_limits(&dir, 1, 1);
        log.append(&rec(1));
        log.append(&rec(2));
        let names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["decisions.jsonl".to_string()], "keep=1 must never leave a decisions.1.jsonl behind");
        let text = std::fs::read_to_string(dir.join("decisions.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 1, "keep=1 must retain only the newest record");
        let back: DecisionRecord = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(back.identity.decision_id, 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// R3 (review round 1): if the rotation deletion fails for a reason other than "already
    /// gone" (e.g. another handle denies delete-sharing while still permitting read/write), that
    /// error must propagate through the append error path -- and no write must follow it. This
    /// holds a second handle to `decisions.jsonl` open with `FILE_SHARE_READ | FILE_SHARE_WRITE`
    /// but not `FILE_SHARE_DELETE`, so `remove_file` hits a real Windows sharing violation while
    /// the file would otherwise still be perfectly appendable.
    #[cfg(windows)]
    #[test]
    fn failed_rotation_deletion_is_propagated_and_blocks_append() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = unique_temp_dir("failed_rotation_deletion_is_propagated_and_blocks_append");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("decisions.jsonl");
        std::fs::write(&target, b"seed\n").unwrap();

        // Win32 `CreateFile` share-mode bits, used as raw literals so this test needs no extra
        // dependency: `FILE_SHARE_READ` = 0x1, `FILE_SHARE_WRITE` = 0x2. `FILE_SHARE_DELETE`
        // (0x4) is deliberately withheld, so a concurrent delete of this path fails with a
        // sharing violation while read/write access from another handle keeps working --
        // reproducing R3's "another handle allows read/write sharing but not delete sharing".
        const FILE_SHARE_READ_WRITE_NO_DELETE: u32 = 0x1 | 0x2;
        let held = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ_WRITE_NO_DELETE)
            .open(&target)
            .expect("hold the live file open without delete sharing");

        let mut log = DecisionLog::with_limits(&dir, 1, 1); // rotate_bytes=1: "seed\n" already qualifies
        log.append(&rec(99));

        let after = std::fs::read_to_string(&target).unwrap();
        assert_eq!(after, "seed\n", "a failed rotation (deletion denied) must not be followed by an append");

        drop(held);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
