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
/// and the fixed source contract (`game = "nl"`, `version = 2`, `ev_unit = "source_sb"`,
/// `source_blinds == [0.5, 1.0]`). A hash match alone never passes without content
/// validation, and a content match alone never passes without the hash matching first.
///
/// N1 (P3.T2 re-review, minor): the exact `[0.5, 1.0]` source-blind check now also runs here,
/// on `info.source_blinds` (already narrowed to `f32` -- both `0.5` and `1.0` are exactly
/// representable, so this catches any narrowed value that is not precisely that pair) --
/// not only in `check_exact_source_blinds` below, which `load_bundle` runs first, on the
/// manifest's original wide `f64` bytes, before `BundleInfo` is even produced (R4). That
/// earlier wide check is what actually prevents a near-miss `f64` value (e.g.
/// `[0.500000001, 1.000000001]`) from rounding into the accepted pair on narrowing; this
/// function's own check is what prevents a `pub` caller who constructs a `BundleInfo`
/// directly and calls `checked_envelope` without ever going through `load_bundle`/
/// `check_exact_source_blinds` at all from skipping the source-blind contract entirely. Since
/// `load_bundle` always calls `check_exact_source_blinds` first and that call already forces
/// the wide value to be exactly `[0.5, 1.0]` before `info` is produced, this added check can
/// never fire on that path -- `load_bundle`'s behaviour is unchanged.
///
/// R3: a `ChartTranscription`-labelled envelope carrying any EV data is rejected here, at
/// load time -- `ChartTranscription::lookup` additionally clears `ev_source_sb` on every
/// result as a second, independent guarantee, but that lookup-time behavior is not a
/// substitute for rejecting the bad bundle in the first place.
///
/// P3.T9 fix round 1, R2: also runs [`check_fold_consistency_wide`] on `raw` -- the load-time
/// fold-EV admission gate, on the ORIGINAL wire-precision (`f64`) EV cells, before anything is
/// narrowed to the `f32` `Envelope` this function returns.
pub fn checked_envelope(info: &BundleInfo, raw: &[u8]) -> Result<Envelope, BundleError> {
    let digest = format!("{:x}", Sha256::digest(raw));
    if digest != info.sha256 {
        return Err(BundleError::Hash);
    }
    let e = decode(raw)?;
    check_depths(&info.depths)?;
    if e.bundle_id != info.bundle_id
        || e.depth_bb != info.depth_bb
        || e.rake_profile != info.rake_profile
        || e.straddle != info.straddle
        || info.game != "nl"
        || info.version != 2
        || info.ev_unit != "source_sb"
        || info.source_blinds != [0.5, 1.0]
    {
        return Err(BundleError::Content("manifest/envelope mismatch".into()));
    }
    if info.source == SourceKind::ChartTranscription && e.nodes.iter().any(|n| n.evs.is_some()) {
        return Err(BundleError::Content("chart bundle must not carry EV data".into()));
    }
    check_fold_consistency_wide(raw, info)?;
    Ok(e)
}

/// Wide-form (`f64`) mirror of just enough of the wire node shape to check fold-EV consistency
/// (P3.T9 fix round 1, R2) against the ORIGINAL wire-precision cells -- before `decode` (in
/// `validate.rs`) ever narrows them to `f32` for the `Envelope`/`PreflopNode` this bundle
/// otherwise uses everywhere else. A private, read-only second parse of the same `raw` bytes
/// `checked_envelope` already hash-verified and structurally decoded (via `decode`, above),
/// mirroring the existing `WideSourceBlinds` pattern already in this file: a second wide-precision
/// pass over the same bytes, for exactly the same reason (standing ruling (a), wide before
/// narrow). `evs` here is action-major, matching the wire's own `EnvelopeNode::evs` shape --
/// `build_node_map`'s `transpose` step (class-major) has not run at this point.
#[derive(serde::Deserialize)]
struct WideFoldNode {
    history: Vec<(String, String, u32)>,
    actor: String,
    actions: Vec<EnvelopeAction>,
    #[serde(default)]
    evs: Option<Vec<Vec<Option<f64>>>>,
}

#[derive(serde::Deserialize)]
struct WideFoldEnvelope {
    nodes: Vec<WideFoldNode>,
}

/// The load-time fold-EV admission gate (P3.T9 fix round 1, R2). Checks every node's *present*
/// fold EV, at every hand class, against [`crate::ev::verify_fold_wide`] -- the one shared
/// predicate [`crate::ev::verify_fold`] (the narrowed `f32` twin used for in-memory validation,
/// e.g. at the `expand_node` boundary) also calls -- on the ORIGINAL wire-precision (`f64`) cell,
/// never a value that has already been narrowed to `f32`. The commitment each class is checked
/// against is computed in `f64` throughout too ([`committed_before_f64`]), never rounded to `f32`
/// before the check runs.
///
/// This is the sole fold-EV admission decision: once a bundle passes here, `build_node_map` never
/// runs a second, narrow-only check that could reject it again -- a narrow-only re-check can
/// disagree with this wide one purely from `f32` rounding, right at the `1e-3` tolerance boundary,
/// which is exactly the defect this fix corrects. A failing node's error names its index, history
/// and offending class, matching `check_envelope`'s own per-node wrapping.
fn check_fold_consistency_wide(raw: &[u8], info: &BundleInfo) -> Result<(), BundleError> {
    let wide: WideFoldEnvelope = serde_json::from_slice(raw)?;
    for (index, n) in wide.nodes.iter().enumerate() {
        let Some(fold_idx) = n.actions.iter().position(|a| a.step == "fold") else {
            continue;
        };
        let Some(evs) = n.evs.as_ref() else {
            continue;
        };
        let Some(fold_row) = evs.get(fold_idx) else {
            continue;
        };
        let history = parse_history(&n.history)?;
        let actor = parse_position(&n.actor)?;
        let committed = committed_before_f64(&history, actor, info.depth_bb);
        let start = info.source_stack_sb_f64();
        for (c, fold_ev) in fold_row.iter().enumerate() {
            if !crate::ev::verify_fold_wide(info.ev_reference, *fold_ev, committed, start) {
                return Err(BundleError::Content(format!(
                    "node {index} (history {:?}): class {c}'s fold EV {fold_ev:?} is inconsistent with committed {committed} under {:?} (tolerance 1e-3)",
                    n.history, info.ev_reference
                )));
            }
        }
    }
    Ok(())
}

/// Every acquired depth a manifest declares is at least one bb (P3.T8 fix round 1, R2).
///
/// A zero is a malformed declaration, not a coverage gap: admitted, `[0]` would look like an
/// ordinary `MissingPreflopNode` and `[0, 100]` would silently behave like `[100]`, leaving corrupt
/// input active and apparently healthy; the depth labels would also divide by it. Rejecting it here
/// -- inside the admission path both `load_bundle` and a direct `checked_envelope` caller go
/// through -- means the store's existing quarantine-and-banner behaviour applies (spec section 8.2:
/// "A failing bundle is quarantined (renamed `.bad`) with a startup banner; remaining bundles stay
/// active") instead of the rule being silently repaired at lookup time.
///
/// The envelope's own singular `depth_bb` is validated by `validate::check_envelope`; this is the
/// manifest's `depths` list, which nothing else checks.
pub fn check_depths(depths: &[u16]) -> Result<(), BundleError> {
    match depths.iter().position(|&d| d == 0) {
        Some(i) => Err(BundleError::Content(format!("depths[{i}] is 0; every acquired depth is at least 1 bb"))),
        None => Ok(()),
    }
}

/// The manifest's `source_blinds` field at its original wide `f64` precision -- unlike
/// `BundleInfo::source_blinds` (already narrowed to `f32` by `crate::numeric`'s general
/// positive-finite-ordered codec), so this task's exact `[0.5, 1.0]` contract (spec section
/// 8.2) can be checked before narrowing could let a near-miss value (e.g. `[0.500000001,
/// 1.000000001]`) round into the accepted pair (standing ruling (a): validate wide before
/// narrowing). Run once, on the manifest's raw bytes, before `BundleInfo` is ever produced
/// for the loader (R4) -- not a second, separately drifting copy of the blind-domain logic
/// already in `numeric.rs`, which only checks positive/finite/ordered, not this exact pair.
#[derive(serde::Deserialize)]
struct WideSourceBlinds {
    source_blinds: [f64; 2],
}

fn check_exact_source_blinds(manifest_bytes: &[u8]) -> Result<(), BundleError> {
    let wide: WideSourceBlinds = serde_json::from_slice(manifest_bytes)?;
    if wide.source_blinds != [0.5, 1.0] {
        return Err(BundleError::Content(format!("source_blinds {:?} must be exactly [0.5, 1.0]", wide.source_blinds)));
    }
    Ok(())
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

/// Advances past every already-folded seat starting at `*idx`, bounded to at most six skips
/// (there are only six seats) so a caller can never spin forever even on adversarial input.
fn skip_folded(
    order: &[Position; 6],
    idx: &mut usize,
    folded: &std::collections::HashSet<Position>,
    history: &[(Position, PreflopStep)],
) -> Result<(), BundleError> {
    let mut skips = 0;
    while folded.contains(&order[*idx % 6]) {
        *idx += 1;
        skips += 1;
        if skips > 6 {
            return Err(BundleError::Content(format!("no active actor left in history {history:?}")));
        }
    }
    Ok(())
}

/// Walks `history` through the fixed six-seat rotation, skipping folded seats, and returns
/// the next eligible actor: whichever position's turn it is after the last step in `history`
/// (or the first position in the rotation if `history` is empty). Confirms each recorded step
/// belongs to the seat whose turn it was, and that no seat acts again after folding.
///
/// Repeated orbits are supported, not excluded: a 3bet/4bet line legitimately revisits a seat
/// a second time around (this plan's own HJ-vs-UTG-4bet fixture does exactly that, and is a
/// required control for this), so `idx` simply keeps counting past 6 rather than being capped
/// to one pass. R2: the caller compares this return value against the node's *declared*
/// actor and rejects a mismatch (including a declared actor who has already folded) --
/// checking only the steps already present in `history`, as an earlier version of this
/// function did, would accept a node whose stated decision-maker is not actually who is next
/// to act, or who is no longer in the hand at all.
fn next_actor(history: &[(Position, PreflopStep)], straddle: bool) -> Result<Position, BundleError> {
    let order = acting_order(straddle);
    let mut folded = std::collections::HashSet::new();
    let mut idx = 0usize;
    for (pos, step) in history {
        skip_folded(&order, &mut idx, &folded, history)?;
        if order[idx % 6] != *pos {
            return Err(BundleError::Content(format!("out-of-turn actor in history {history:?}")));
        }
        if matches!(step, PreflopStep::Fold) {
            folded.insert(*pos);
        }
        idx += 1;
    }
    skip_folded(&order, &mut idx, &folded, history)?;
    Ok(order[idx % 6])
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
    committed_before_f64(history, actor, depth_bb) as f32
}

/// [`committed_before`]'s `f64` twin (P3.T9 fix round 1, R2): every seat's contribution tracked
/// in `f64` throughout, narrowed to `f32` only by [`committed_before`] itself (for storage in
/// [`PreflopNode::committed_by_actor_sb`]). [`check_fold_consistency_wide`] calls this directly so
/// the load-time wide fold-EV admission check never rounds a commitment to `f32` before comparing
/// it against a wire-precision EV cell -- the one implementation of the committed-so-far walk,
/// never a second copy that could drift from this one.
fn committed_before_f64(history: &[(Position, PreflopStep)], actor: Position, depth_bb: u16) -> f64 {
    let stack_sb = 2.0 * depth_bb as f64;
    let mut committed: std::collections::HashMap<Position, f64> = [
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
        let highest = committed.values().cloned().fold(0.0f64, f64::max);
        let amount = match step {
            PreflopStep::Fold | PreflopStep::Check => committed[pos],
            PreflopStep::Call => highest.min(stack_sb),
            PreflopStep::Raise { to_bb_x1000 } => (*to_bb_x1000 as f64) / 500.0,
            PreflopStep::AllIn => stack_sb,
        };
        committed.insert(*pos, amount);
    }
    committed[&actor]
}

/// Builds the class-major node map for one already hash-and-content-validated envelope:
/// action-major -> class-major transpose for `weights`/`evs`, turn-order and no-action-after-
/// fold checks, the declared-actor-matches-the-next-eligible-actor check (R2), and
/// `committed_by_actor_sb` for each node's actor. A duplicate `node_key` (two nodes with the
/// same `(depth_bb, rake_profile, straddle, history)`) is rejected.
///
/// P3.T9 fix round 1 (R2): fold-EV consistency is **not** re-checked here. That admission
/// decision is [`check_fold_consistency_wide`], run inside [`checked_envelope`] on the original
/// wire-precision EV cells before this function is ever called -- running a second, narrow-only
/// check on the already-`f32`-narrowed `PreflopNode` here could reject an already-accepted wire
/// value purely from rounding right at the `1e-3` boundary (the exact defect this fix corrects),
/// so the wide gate's decision is preserved through node construction rather than re-litigated.
///
/// P3.T9 fix round 2 (N1): every node this function produces is stamped
/// `fold_wide_verified: true`, because this function is only ever called (from [`load_bundle`])
/// on an envelope [`checked_envelope`] has already run [`check_fold_consistency_wide`] against
/// and accepted. [`crate::ev::expand_node`] trusts that stamp instead of re-deriving its own
/// narrow residual from the node's already-`f32`-narrowed EV cell, which is exactly the re-check
/// that could disagree with this wide admission purely from rounding at the tolerance boundary.
pub fn build_node_map(info: &BundleInfo, e: &Envelope) -> Result<BTreeMap<String, PreflopNode>, BundleError> {
    let mut map = BTreeMap::new();
    for n in &e.nodes {
        let history = parse_history(&n.history)?;
        let expected_actor = next_actor(&history, info.straddle)?;
        let actor = parse_position(&n.actor)?;
        if actor != expected_actor {
            return Err(BundleError::Content(format!(
                "declared actor {actor:?} does not match the next eligible actor {expected_actor:?} for history {history:?}"
            )));
        }
        let mut unreachable = [false; 169];
        for &c in &n.unreachable_classes {
            unreachable[c] = true;
        }
        let node = PreflopNode {
            actor,
            actions: n.actions.iter().map(parse_step).collect::<Result<_, _>>()?,
            probs: transpose(&n.weights),
            ev_source_sb: n.evs.as_ref().map(|rows| transpose(rows)),
            unreachable,
            committed_by_actor_sb: committed_before(&history, actor, info.depth_bb),
            // This function only ever runs on an envelope `checked_envelope` already ran
            // `check_fold_consistency_wide` against and accepted (P3.T9 fix round 2, N1).
            fold_wide_verified: true,
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
    /// R3: forces `ev_source_sb = None` on every result, regardless of what the stored
    /// `PreflopNode` actually carries. `checked_envelope` already refuses to load a chart
    /// bundle whose envelope carries EV data, but this is a second, independent guarantee
    /// that holds even for a `ChartTranscription` built directly (bypassing `load_bundle`
    /// entirely) rather than relying solely on that load-time check.
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode> {
        self.nodes.get(&node_key(key)).cloned().map(|mut node| {
            node.ev_source_sb = None;
            node
        })
    }
}

/// Reads one bundle: manifest bytes -> exact-wide-blinds check (R4) -> `BundleInfo`, node
/// bytes -> hash-checked, content-validated `Envelope`, then the class-major node map keyed
/// by `node_key`. The wide blind check runs on the manifest's raw bytes before `BundleInfo`
/// is deserialized, so it sees the original `f64` values, never the narrowed `f32` ones.
///
/// P3.T9 fix round 1 (R5): forces the process-wide combo-class table
/// ([`BundleInfo::combo_classes`]) to initialize right here, at bundle admission -- not deferred
/// to whichever bundle's node happens to be expanded first. The table depends only on
/// `core_ranges::expand_169`'s fixed class order, never on this (or any) bundle's own data, so
/// every bundle forces the same one-time build and every later caller borrows the identical
/// `'static` table.
pub fn load_bundle(manifest: &Path, nodes: &Path) -> Result<Box<dyn PreflopSource>, BundleError> {
    let manifest_bytes = bounded_read(manifest)?;
    check_exact_source_blinds(&manifest_bytes)?;
    let info: BundleInfo = serde_json::from_slice(&manifest_bytes)?;
    let _ = info.combo_classes();
    let envelope = checked_envelope(&info, &bounded_read(nodes)?)?;
    let map = build_node_map(&info, &envelope)?;
    Ok(match info.source {
        SourceKind::PokerDataJson => Box::new(PokerDataJson { info, nodes: map }),
        SourceKind::ChartTranscription => Box::new(ChartTranscription { info, nodes: map }),
    })
}

/// True if `path`'s own directory entry is a symlink or reparse point (e.g. a Windows
/// junction) -- checked with `symlink_metadata`, which never follows it. `Path::is_dir`/
/// `File::open` do follow it; R1 closes exactly that gap by never calling either on anything
/// on the path from a bundle's own directory entry to its two input files until this check
/// has passed.
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

/// True if `path` exists at all, including as a symlink -- even a dangling one whose target
/// is missing. Unlike `Path::exists` (which follows the link and reports `false` for a
/// dangling target, treating it as a free name), this treats a dangling link as occupied.
fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

fn quarantine_name(dir: &Path, bundle_id: &str) -> std::path::PathBuf {
    let first = dir.join(format!("{bundle_id}.bad"));
    if !occupied(&first) {
        return first;
    }
    (1u32..).map(|n| dir.join(format!("{bundle_id}.{n}.bad"))).find(|p| !occupied(p)).expect("a free quarantine name exists")
}

/// `child` is a real (non-link) entry, canonicalizing to an immediate child of
/// `canonical_dir`, whose own `manifest.json`/`nodes.json` are themselves real (non-link)
/// files -- checked (R1) before `load_bundle` ever calls `File::open` on any of the three.
/// Rejects, without reading through it, any link anywhere on that path: a directory link
/// (e.g. a junction) planted inside the store's directory, or an individual input file that
/// is itself a link -- either could otherwise resolve to content outside `canonical_dir`.
fn load_contained_bundle(canonical_dir: &Path, child: &Path) -> Result<Box<dyn PreflopSource>, BundleError> {
    if is_link(child) {
        return Err(BundleError::Content("bundle entry is a symlink or reparse point".into()));
    }
    let resolved = std::fs::canonicalize(child)?;
    if resolved.parent() != Some(canonical_dir) {
        return Err(BundleError::Content("bundle entry escapes the preflop directory".into()));
    }
    let manifest = child.join("manifest.json");
    let nodes = child.join("nodes.json");
    if is_link(&manifest) || is_link(&nodes) {
        return Err(BundleError::Content("bundle input file is a symlink or reparse point".into()));
    }
    load_bundle(&manifest, &nodes)
}

/// The independent-bundle store (spec section 8.1's `PreflopStore`): every bundle validated on
/// its own bytes, a failing bundle quarantined without affecting any other.
pub struct PreflopStore {
    /// Crate-visible so `crate::lookup::query` can rank the candidates directly (spec section 8.3);
    /// outside the crate the slice is reached through [`PreflopStore::bundles`].
    pub(crate) bundles: Vec<Box<dyn PreflopSource>>,
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
    /// never removes another: every child is validated on its own bytes, with link safety
    /// (R1) checked before anything is opened -- see `load_contained_bundle`. Reads sorted
    /// immediate children of `dir` only, skipping names already ending `.bad`; an ordinary
    /// non-directory, non-link entry (e.g. a stray README) is skipped silently, exactly like
    /// before, but a directory *link* is never silently accepted as one -- it is routed
    /// through the same contained-load-or-quarantine path as any other bundle, so it can
    /// never be read as valid and is quarantined (its own directory entry renamed, never its
    /// target) exactly like any other invalid bundle.
    pub fn open(dir: &Path) -> (Self, Vec<String>) {
        let canonical_dir = match std::fs::canonicalize(dir) {
            Ok(d) => d,
            Err(e) => return (Self { bundles: vec![] }, vec![format!("preflop dir {}: {e}", dir.display())]),
        };
        let mut names: Vec<std::ffi::OsString> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.file_name()).collect(),
            Err(e) => return (Self { bundles: vec![] }, vec![format!("preflop dir {}: {e}", dir.display())]),
        };
        names.sort();
        let (mut bundles, mut banners) = (Vec::new(), Vec::new());
        for name in names {
            let id = name.to_string_lossy().to_string();
            if id.ends_with(".bad") {
                continue;
            }
            let child = dir.join(&name);
            let meta = match std::fs::symlink_metadata(&child) {
                Ok(m) => m,
                Err(_) => continue, // vanished between read_dir and here; nothing to do
            };
            if !meta.file_type().is_symlink() && !meta.is_dir() {
                continue; // an ordinary non-bundle file; not a failure, skipped as before
            }
            match load_contained_bundle(&canonical_dir, &child) {
                Ok(source) => bundles.push(source),
                Err(e) => {
                    let target = quarantine_name(dir, &id);
                    // `child`/`target` are always `dir.join(<a plain OS name, no
                    // separators>)`, so this always holds by construction -- kept as
                    // defense in depth, not the link-safety check itself (that already ran,
                    // above, before anything was opened).
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
