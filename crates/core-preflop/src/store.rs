//! Independent-bundle loader and quarantine (spec section 8.2): reads `<child>/manifest.json`
//! + `<child>/nodes.json` pairs from a directory, hash-and-content-validates each one on its
//! own bytes (never another bundle's), and renames a failing bundle to a `.bad` sibling with a
//! startup banner instead of aborting the whole store. A valid sibling bundle is never affected
//! by another bundle's failure (spec: "A failing bundle is quarantined (renamed `.bad`) with a
//! startup banner; remaining bundles stay active").
//!
//! Turn order and each seat's pre-decision committed amount (`committed_by_actor_sb`) are
//! derived here from the source posts and each node's `history` -- the wire envelope never
//! carries a second, divergent copy of either.

use crate::envelope::{
    BundleInfo, Envelope, EnvelopeAction, PreflopNode, PreflopNodeKey, PreflopSource, PreflopStep, SourceKind,
};
use crate::validate::{decode, BundleError, MAX_BUNDLE_BYTES};
use proto::Position;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

/// Reads `path` up to `MAX_BUNDLE_BYTES + 1` bytes (never more, regardless of the file's
/// reported size) and rejects anything over `MAX_BUNDLE_BYTES` -- a bounded read before any
/// parsing ever sees the bytes, per the standing ruling.
pub fn bounded_read(path: &Path) -> Result<Vec<u8>, BundleError> {
    let mut bytes = Vec::new();
    File::open(path)?.take(MAX_BUNDLE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BUNDLE_BYTES {
        return Err(BundleError::TooLarge);
    }
    Ok(bytes)
}

/// Hashes `raw` (the node file's exact bytes) against `info.sha256`, decodes and structurally
/// validates it (`decode`, which runs every rule in `validate.rs`), and cross-checks the
/// envelope against its manifest: matching `bundle_id`/`depth_bb`/`rake_profile`/`straddle`,
/// and the fixed source contract (`game = "nl"`, `version = 2`, `source_blinds = [0.5, 1.0]`,
/// `ev_unit = "source_sb"`). A hash match alone never passes without content validation, and a
/// content match alone never passes without the hash matching first.
pub fn checked_envelope(info: &BundleInfo, raw: &[u8]) -> Result<Envelope, BundleError> {
    let digest = format!("{:x}", Sha256::digest(raw));
    if digest != info.sha256 {
        return Err(BundleError::Hash);
    }
    let e = decode(raw)?;
    if e.bundle_id != info.bundle_id
        || e.depth_bb != info.depth_bb
        || e.rake_profile != info.rake_profile
        || e.straddle != info.straddle
        || info.game != "nl"
        || info.version != 2
        || info.source_blinds != [0.5, 1.0]
        || info.ev_unit != "source_sb"
    {
        return Err(BundleError::Content("manifest/envelope mismatch".into()));
    }
    Ok(e)
}

/// `action_major[a][c] -> class_major[c][a]`: every row of `action_major` must be the same
/// length (169, already guaranteed by `validate`'s shape check upstream), and every row of the
/// result has length `action_major.len()`.
pub fn transpose<T: Clone>(action_major: &[Vec<T>]) -> Vec<Vec<T>> {
    (0..169).map(|c| action_major.iter().map(|a| a[c].clone()).collect()).collect()
}

fn parse_position(s: &str) -> Result<Position, BundleError> {
    Ok(match s {
        "UTG" => Position::Utg,
        "HJ" => Position::Hj,
        "CO" => Position::Co,
        "BTN" => Position::Btn,
        "SB" => Position::Sb,
        "BB" => Position::Bb,
        other => return Err(BundleError::Content(format!("unknown position {other}"))),
    })
}

fn parse_step_token(step: &str, amount: Option<u32>) -> Result<PreflopStep, BundleError> {
    Ok(match (step, amount) {
        ("fold", None) => PreflopStep::Fold,
        ("check", None) => PreflopStep::Check,
        ("call", None) => PreflopStep::Call,
        ("raise", Some(v)) if v > 0 => PreflopStep::Raise { to_bb_x1000: v },
        ("allin", None) => PreflopStep::AllIn,
        _ => return Err(BundleError::Content(format!("invalid step {step:?} amount {amount:?}"))),
    })
}

fn parse_step(a: &EnvelopeAction) -> Result<PreflopStep, BundleError> {
    parse_step_token(&a.step, a.to_bb_x1000)
}

/// Maps the envelope's `(position, step, amount)` history tuples onto `(Position,
/// PreflopStep)`, `amount == 0` meaning "no amount" (mirroring `valid_step` in `validate.rs`).
/// Every rejection names the offending history.
fn parse_history(history: &[(String, String, u32)]) -> Result<Vec<(Position, PreflopStep)>, BundleError> {
    history
        .iter()
        .map(|(p, s, v)| {
            let bad = || BundleError::Content(format!("invalid history entry in {history:?}"));
            let pos = parse_position(p).map_err(|_| bad())?;
            let step = parse_step_token(s, (*v != 0).then_some(*v)).map_err(|_| bad())?;
            Ok((pos, step))
        })
        .collect()
}

/// Preflop turn order (spec section 2, "Positions"): `UTG, HJ, CO, BTN, SB, BB` without a
/// straddle; `HJ, CO, BTN, SB, BB, UTG` with the UTG straddle (the straddler keeps the physical
/// name UTG and acts last).
fn acting_order(straddle: bool) -> [Position; 6] {
    if straddle {
        [Position::Hj, Position::Co, Position::Btn, Position::Sb, Position::Bb, Position::Utg]
    } else {
        [Position::Utg, Position::Hj, Position::Co, Position::Btn, Position::Sb, Position::Bb]
    }
}

/// Walks `history` confirming each step belongs to the seat whose turn it is (skipping folded
/// seats in the fixed rotation) and that no seat acts again after folding. `history` here is
/// always a single, strict, non-repeating pass through the rotation, which every node in this
/// plan's fixtures observes -- a re-opened second orbit after a raise is out of this task's
/// scope (later plan-3 lookup/translation work owns that).
fn check_turn_order(history: &[(Position, PreflopStep)], straddle: bool) -> Result<(), BundleError> {
    let order = acting_order(straddle);
    let mut folded = std::collections::HashSet::new();
    let mut idx = 0usize;
    for (pos, step) in history {
        let mut skips = 0;
        while folded.contains(&order[idx % 6]) {
            idx += 1;
            skips += 1;
            if skips > 6 {
                return Err(BundleError::Content(format!("no active actor left in history {history:?}")));
            }
        }
        if order[idx % 6] != *pos {
            return Err(BundleError::Content(format!("out-of-turn actor in history {history:?}")));
        }
        if matches!(step, PreflopStep::Fold) {
            folded.insert(*pos);
        }
        idx += 1;
    }
    Ok(())
}

/// Replays the source posts (`SB = 1`, `BB = 2`, source-SB units -- spec section 2's blind
/// pair `sb = 0.5 bb` / `bb = 1 bb` scaled to source SB) and then `history`, tracking every
/// seat's total contribution this street. Fold/Check make no payment; Call matches the
/// street's highest contribution so far, capped at the source stack; Raise sets the raiser's
/// contribution to `to_bb_x1000 / 500` (bb x1000 -> source-SB units, since 1 bb = 2 source SB);
/// AllIn sets it to the full source stack `2 * depth_bb`. Returns `actor`'s own contribution
/// immediately before `actor`'s decision at this node -- their blind post if they have not yet
/// acted in `history`. `combos` never enters this calculation.
fn committed_before(history: &[(Position, PreflopStep)], actor: Position, depth_bb: u16) -> f32 {
    let stack_sb = 2.0 * depth_bb as f32;
    let mut committed: std::collections::HashMap<Position, f32> = [
        (Position::Utg, 0.0),
        (Position::Hj, 0.0),
        (Position::Co, 0.0),
        (Position::Btn, 0.0),
        (Position::Sb, 1.0),
        (Position::Bb, 2.0),
    ]
    .into_iter()
    .collect();
    for (pos, step) in history {
        let highest = committed.values().cloned().fold(0.0f32, f32::max);
        let amount = match step {
            PreflopStep::Fold | PreflopStep::Check => committed[pos],
            PreflopStep::Call => highest.min(stack_sb),
            PreflopStep::Raise { to_bb_x1000 } => (*to_bb_x1000 as f32) / 500.0,
            PreflopStep::AllIn => stack_sb,
        };
        committed.insert(*pos, amount);
    }
    committed[&actor]
}

/// Builds the class-major node map for one already hash-and-content-validated envelope:
/// action-major -> class-major transpose for `weights`/`evs`, turn-order and no-action-after-
/// fold checks, and `committed_by_actor_sb` for each node's actor. A duplicate `node_key` (two
/// nodes with the same `(depth_bb, rake_profile, straddle, history)`) is rejected.
pub fn build_node_map(info: &BundleInfo, e: &Envelope) -> Result<BTreeMap<String, PreflopNode>, BundleError> {
    let mut map = BTreeMap::new();
    for n in &e.nodes {
        let history = parse_history(&n.history)?;
        check_turn_order(&history, info.straddle)?;
        let mut unreachable = [false; 169];
        for &c in &n.unreachable_classes {
            unreachable[c] = true;
        }
        let actor = parse_position(&n.actor)?;
        let node = PreflopNode {
            actor,
            actions: n.actions.iter().map(parse_step).collect::<Result<_, _>>()?,
            probs: transpose(&n.weights),
            ev_source_sb: n.evs.as_ref().map(|rows| transpose(rows)),
            unreachable,
            committed_by_actor_sb: committed_before(&history, actor, info.depth_bb),
        };
        let key = node_key(&PreflopNodeKey {
            depth_bb: info.depth_bb,
            rake_profile: info.rake_profile.clone(),
            straddle: info.straddle,
            history,
        });
        if map.insert(key.clone(), node).is_some() {
            return Err(BundleError::Content(format!("duplicate node {key}")));
        }
    }
    Ok(map)
}

/// Deterministic serialization of `(depth_bb, rake_profile, straddle, history)` -- the one key
/// both adapters below store nodes under and look nodes up by.
pub fn node_key(k: &PreflopNodeKey) -> String {
    serde_json::to_string(&(k.depth_bb, &k.rake_profile, k.straddle, &k.history)).expect("validated finite node key")
}

/// One PokerData-derived bundle: hash-and-content-validated, its nodes keyed by `node_key`.
pub struct PokerDataJson {
    pub info: BundleInfo,
    pub nodes: BTreeMap<String, PreflopNode>,
}

/// One chart-transcription-derived bundle. Same validated shape as `PokerDataJson` -- charts
/// omitting `evs` and carrying `ChartRounded` instead of `EvReferenceUnverified` is a lookup-
/// time distinction (spec section 6/8.3), not a structural one this loader enforces.
pub struct ChartTranscription {
    pub info: BundleInfo,
    pub nodes: BTreeMap<String, PreflopNode>,
}

impl PreflopSource for PokerDataJson {
    fn bundle_info(&self) -> &BundleInfo {
        &self.info
    }
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode> {
        self.nodes.get(&node_key(key)).cloned()
    }
}

impl PreflopSource for ChartTranscription {
    fn bundle_info(&self) -> &BundleInfo {
        &self.info
    }
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode> {
        self.nodes.get(&node_key(key)).cloned()
    }
}

/// Reads one bundle: manifest bytes -> `BundleInfo`, node bytes -> hash-checked,
/// content-validated `Envelope`, then the class-major node map keyed by `node_key`.
pub fn load_bundle(manifest: &Path, nodes: &Path) -> Result<Box<dyn PreflopSource>, BundleError> {
    let info: BundleInfo = serde_json::from_slice(&bounded_read(manifest)?)?;
    let envelope = checked_envelope(&info, &bounded_read(nodes)?)?;
    let map = build_node_map(&info, &envelope)?;
    Ok(match info.source {
        SourceKind::PokerDataJson => Box::new(PokerDataJson { info, nodes: map }),
        SourceKind::ChartTranscription => Box::new(ChartTranscription { info, nodes: map }),
    })
}

fn quarantine_name(dir: &Path, bundle_id: &str) -> std::path::PathBuf {
    let first = dir.join(format!("{bundle_id}.bad"));
    if !first.exists() {
        return first;
    }
    (1u32..).map(|n| dir.join(format!("{bundle_id}.{n}.bad"))).find(|p| !p.exists()).expect("a free quarantine name exists")
}

/// The independent-bundle store (spec section 8.1's `PreflopStore`): every bundle validated on
/// its own bytes, a failing bundle quarantined without affecting any other.
pub struct PreflopStore {
    bundles: Vec<Box<dyn PreflopSource>>,
}

impl PreflopStore {
    pub fn from_sources(bundles: Vec<Box<dyn PreflopSource>>) -> Self {
        Self { bundles }
    }

    pub fn bundles(&self) -> &[Box<dyn PreflopSource>] {
        &self.bundles
    }

    pub fn bundle_of(&self, bundle_id: &str) -> Option<&dyn PreflopSource> {
        self.bundles.iter().find(|b| b.bundle_info().bundle_id == bundle_id).map(|b| b.as_ref())
    }

    /// Returns the store plus one banner line per rejected bundle. A failure of one bundle
    /// never removes another: every child is validated on its own bytes. Reads sorted
    /// immediate children of `dir` only, skipping names already ending `.bad`.
    pub fn open(dir: &Path) -> (Self, Vec<String>) {
        let mut names: Vec<std::ffi::OsString> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
            Err(e) => return (Self { bundles: vec![] }, vec![format!("preflop dir {}: {e}", dir.display())]),
        };
        names.sort();
        let (mut bundles, mut banners) = (Vec::new(), Vec::new());
        for name in names {
            let child = dir.join(&name);
            let id = name.to_string_lossy().to_string();
            if id.ends_with(".bad") || !child.is_dir() {
                continue;
            }
            match load_bundle(&child.join("manifest.json"), &child.join("nodes.json")) {
                Ok(source) => bundles.push(source),
                Err(e) => {
                    let target = quarantine_name(dir, &id);
                    // Both ends must stay immediate children of `dir`; never follow a link out.
                    let inside = |p: &Path| p.parent() == Some(dir);
                    let renamed = if inside(&child) && inside(&target) {
                        std::fs::rename(&child, &target).map_err(|e| e.to_string())
                    } else {
                        Err("quarantine path escapes the preflop directory".into())
                    };
                    banners.push(match renamed {
                        Ok(()) => format!("preflop bundle {id} quarantined as {}: {e}", target.display()),
                        Err(why) => format!("preflop bundle {id} rejected ({e}); rename failed: {why}"),
                    });
                }
            }
        }
        (Self { bundles }, banners)
    }
}
