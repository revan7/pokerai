//! Wire envelope decoded from a source bundle (spec section 8.2), and the semantic
//! `PreflopSource` interface (spec section 8.1) that later plan-3 tasks implement against.

use proto::Position;
use serde::{Deserialize, Serialize};

/// `Send + Sync` because the engine's fast path reads the store from `engine-main`
/// while `Engine` holds it (spec section 8.1's method signatures are unchanged; `dyn
/// PreflopSource` inherits the auto traits, and both adapters below are plain data).
pub trait PreflopSource: Send + Sync {
    fn bundle_info(&self) -> &BundleInfo;
    fn lookup(&self, key: &PreflopNodeKey) -> Option<PreflopNode>;

    /// True if none of this source's stored nodes carry EV data (spec section 8.2/8.3: a
    /// `ChartTranscription` bundle never carries `evs`; `checked_envelope`/`ChartTranscription::
    /// lookup` already enforce this at load time and at lookup time respectively -- this is a
    /// third, read-only view over the same guarantee, added for P3.T7's Rust-boundary proof
    /// that a shipped chart bundle loads with no EV anywhere in its node map).
    ///
    /// Defaults to `true` for any source whose node map is not reachable through this trait
    /// (e.g. a test double with no nodes at all, vacuously true); `PokerDataJson` and
    /// `ChartTranscription` (`store.rs`) both override it with the real
    /// `self.nodes.values().all(|n| n.ev_source_sb.is_none())` check over their own map.
    fn nodes_have_no_ev(&self) -> bool {
        true
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreflopNodeKey {
    pub depth_bb: u16,
    pub rake_profile: String,
    pub straddle: bool,
    pub history: Vec<(Position, PreflopStep)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PreflopStep {
    Fold,
    Check,
    Call,
    Raise { to_bb_x1000: u32 },
    AllIn,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreflopNode {
    pub actor: Position,
    pub actions: Vec<PreflopStep>,
    pub probs: Vec<Vec<f32>>,
    pub ev_source_sb: Option<Vec<Vec<Option<f32>>>>,
    pub unreachable: [bool; 169],
    pub committed_by_actor_sb: f32,
    /// Whether this node's fold EV already passed the load-time wide (`f64`) admission gate
    /// (`crate::store::check_fold_consistency_wide`, run inside `checked_envelope` on the
    /// ORIGINAL wire-precision bytes) -- P3.T9 fix round 2, N1.
    ///
    /// `crate::store::build_node_map` sets this `true` on every node it produces, because it is
    /// only ever called on an envelope that has already passed that gate. A `PreflopNode` built
    /// any other way (this struct is `pub` and every field is constructible directly, and
    /// `PreflopStore::from_sources` accepts a source without ever running the loader) must set
    /// this `false`: no wide check ran, so [`crate::ev::expand_node`]'s own boundary assertion
    /// still re-derives the narrow (`f32`) cross-check as the only safety net available for that
    /// data.
    ///
    /// [`crate::ev::expand_node`] never re-derives a narrow residual for a node with this `true`:
    /// narrowing an already-wide-admitted EV to `f32` before re-checking it can shift a residual
    /// by less than a part in `1e7`, right across the spec's `1e-3` tolerance boundary, and
    /// disagree with the wide gate that already admitted it (the exact defect this field fixes --
    /// see the re-review's `195.0009999`-under-`AbsoluteStackVerified` repro). The wide gate is
    /// the one admission decision; this field carries that decision forward instead of asking the
    /// question a second time on narrower data.
    pub fold_wide_verified: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceKind {
    PokerDataJson,
    ChartTranscription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvReference {
    DecisionIncrementalVerified,
    NetHandStartVerified,
    AbsoluteStackVerified,
    Unverified,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RakeProfile {
    /// Domain `[0, 1)` (a fraction, never a full rake), wide-checked before narrowing.
    #[serde(deserialize_with = "crate::numeric::deserialize_rake_rate", serialize_with = "crate::numeric::serialize_rake_rate")]
    pub rate: f32,
    /// Domain `>= 0`, wide-checked before narrowing (rejects e.g. an `f64` cap that
    /// overflows `f32` on narrowing).
    #[serde(deserialize_with = "crate::numeric::deserialize_cap_bb", serialize_with = "crate::numeric::serialize_cap_bb")]
    pub cap_bb: f32,
    pub no_flop_no_drop: bool,
}

/// `rake: None` means the source's rake is undocumented, never exact; `rake_profile`'s
/// descriptive text is retained regardless. A chart PDF that does not publish its rake
/// cannot be assigned the PokerData profile as fact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BundleInfo {
    pub bundle_id: String,
    pub source: SourceKind,
    pub depth_bb: u16,
    pub depths: Vec<u16>,
    /// `[sb, bb]`: both finite and strictly positive, `bb >= sb`, wide-checked before
    /// narrowing (`crate::numeric`).
    #[serde(deserialize_with = "crate::numeric::deserialize_source_blinds", serialize_with = "crate::numeric::serialize_source_blinds")]
    pub source_blinds: [f32; 2],
    pub rake_profile: String,
    pub rake: Option<RakeProfile>,
    pub straddle: bool,
    pub version: u16,
    pub game: String,
    pub ev_unit: String,
    pub ev_reference: EvReference,
    pub license_note: String,
    pub accuracy: String,
    pub sha256: String,
}

/// Wire envelope decoded from a source bundle (spec section 8.2). `deny_unknown_fields`
/// so an unrecognized provider field fails validation instead of being silently dropped.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub bundle_id: String,
    pub depth_bb: u16,
    pub rake_profile: String,
    pub straddle: bool,
    pub class_order: String,
    pub nodes: Vec<EnvelopeNode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeNode {
    /// `(position, step, amount)`; `amount == 0` means "no amount" (see `valid_step`).
    pub history: Vec<(String, String, u32)>,
    pub actor: String,
    pub actions: Vec<EnvelopeAction>,
    /// Wire numbers are read as `f64` and domain-checked (`[0, 1]`) before narrowing to
    /// `f32`, and re-checked the same way on serialize (`crate::numeric`, standing ruling).
    #[serde(deserialize_with = "crate::numeric::deserialize_weights", serialize_with = "crate::numeric::serialize_weights")]
    pub weights: Vec<Vec<f32>>,
    /// Same wide-then-narrow domain check as `weights` (finite only; see `crate::numeric`),
    /// applied per present cell -- `None` entries pass through untouched.
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "crate::numeric::deserialize_evs", serialize_with = "crate::numeric::serialize_evs")]
    pub evs: Option<Vec<Vec<Option<f32>>>>,
    pub unreachable_classes: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvelopeAction {
    pub step: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_bb_x1000: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}
