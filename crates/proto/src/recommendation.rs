use serde::{Deserialize, Deserializer, Serialize, Serializer};
use crate::hand::{Action, LegalAction, Seat, Street};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct DecisionIdentity { pub hand_id: u64, pub hand_revision: u32, pub decision_id: u64, pub config_revision: u32, pub model_revision: u32 }

// Every `f32` and `Option<f32>` in this module crosses the Tauri IPC boundary to the UI, so each
// one carries a checked codec from `crate::numeric` (review S1): the value is validated in its wide
// f64 form before narrowing on the way in, and rejected rather than emitted as JSON `null` on the
// way out. Without it a required `f32` holding NaN makes the whole event undeserializable, and an
// `Option<f32>` holding `Some(NaN)` silently round-trips to `None` -- the advice just vanishes.
// `None` stays the one and only nullable value.

/// `Serialize`/`Deserialize` are hand-written (Task: P4.T2-followup), for the same reason as
/// `Action` (see its doc comment in `hand.rs`): `#[serde(tag = "kind")]` forces
/// `Deserializer::deserialize_any` to find the tag, which bincode 1.3.3 (spec 10.4's pinned cache
/// storage format) unconditionally refuses, and `ApproxReason` reaches `CacheEntry` directly via
/// `CacheEntry::reasons`. `ApproxReasonJson` keeps the identical tagged-map derive (including
/// every field's own `#[serde(with = "crate::numeric::...")]` codec, unchanged) for the
/// human-readable path; `ApproxReasonBincode` drops only the container's `tag` attribute -- its
/// fields keep the exact same `with = "crate::numeric::..."` codecs, which (Task:
/// P4.T2-followup) now branch on `is_human_readable()` themselves, so reusing them here is
/// already bincode-correct with no further change.
///
/// `BetTranslation.deviation` is spec 8.4's `d = min(|s - A|, |s - B|)`, a distance between pot
/// fractions: non-negative and finite, with no upper bound (a 100 bb open shove against a lone
/// 2.5 bb menu size is `d = 39`), so both mirrors carry it with the `non_negative` codec, never the
/// `[0, 1]` `probability` one (P3.T13 fix round 1, ruling 13-R4).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(tag = "kind"))]
pub enum ApproxReason {
    BetTranslation {
        street: Street,
        seat: Seat,
        #[cfg_attr(feature = "typescript", ts(as = "f32"))] observed_pct: f32,
        #[cfg_attr(feature = "typescript", ts(as = "Vec<(f32, f32)>"))] mapped: Vec<(f32, f32)>,
        #[cfg_attr(feature = "typescript", ts(as = "f32"))] deviation: f32,
        prominent: bool,
    },
    DepthBucket { seat: Seat, #[cfg_attr(feature = "typescript", ts(as = "f32"))] actual_bb: f32, used_bb: u16, prominent: bool },
    AsymmetricStacks { #[cfg_attr(feature = "typescript", ts(as = "Vec<f32>"))] stacks_bb: Vec<f32>, prominent: bool },
    RakeProfileMapped { actual: String, used: String },
    StraddleMapped { #[cfg_attr(feature = "typescript", ts(as = "[f32; 3]"))] posts: [f32; 3] },
    ShortHandedMapped { dealt: u8 },
    DeadlineBestSoFar { reached_bp: u16, target_bp: u16 },
    ChartRounded,
    EvReferenceUnverified,
    UnconditionedPriorStreet { street: Street, seat: Seat, cause: String },
    UnconditionedCurrentStreet,
    MultiwayStreetRoot { folded_this_street: u8, dead_this_street: u32 },
    SprBucketed { #[cfg_attr(feature = "typescript", ts(as = "f32"))] actual: f32, #[cfg_attr(feature = "typescript", ts(as = "f32"))] used: f32 },
    MenuRounded { #[cfg_attr(feature = "typescript", ts(as = "f32"))] max_delta_pct: f32 },
    BranchResidual { seat: Seat, #[cfg_attr(feature = "typescript", ts(as = "f32"))] residual_mass_pct: f32, cause: String },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind")]
enum ApproxReasonJson {
    BetTranslation {
        street: Street,
        seat: Seat,
        #[serde(with = "crate::numeric::finite")] observed_pct: f32,
        #[serde(with = "crate::numeric::mapped_sizes")] mapped: Vec<(f32, f32)>,
        #[serde(with = "crate::numeric::non_negative")] deviation: f32,
        prominent: bool,
    },
    DepthBucket { seat: Seat, #[serde(with = "crate::numeric::finite")] actual_bb: f32, used_bb: u16, prominent: bool },
    AsymmetricStacks { #[serde(with = "crate::numeric::finite_vec")] stacks_bb: Vec<f32>, prominent: bool },
    RakeProfileMapped { actual: String, used: String },
    StraddleMapped { #[serde(with = "crate::numeric::finite_array3")] posts: [f32; 3] },
    ShortHandedMapped { dealt: u8 },
    DeadlineBestSoFar { reached_bp: u16, target_bp: u16 },
    ChartRounded,
    EvReferenceUnverified,
    UnconditionedPriorStreet { street: Street, seat: Seat, cause: String },
    UnconditionedCurrentStreet,
    MultiwayStreetRoot { folded_this_street: u8, dead_this_street: u32 },
    SprBucketed { #[serde(with = "crate::numeric::finite")] actual: f32, #[serde(with = "crate::numeric::finite")] used: f32 },
    MenuRounded { #[serde(with = "crate::numeric::finite")] max_delta_pct: f32 },
    BranchResidual { seat: Seat, #[serde(with = "crate::numeric::finite")] residual_mass_pct: f32, cause: String },
}

#[derive(Serialize, Deserialize)]
enum ApproxReasonBincode {
    BetTranslation {
        street: Street,
        seat: Seat,
        #[serde(with = "crate::numeric::finite")] observed_pct: f32,
        #[serde(with = "crate::numeric::mapped_sizes")] mapped: Vec<(f32, f32)>,
        #[serde(with = "crate::numeric::non_negative")] deviation: f32,
        prominent: bool,
    },
    DepthBucket { seat: Seat, #[serde(with = "crate::numeric::finite")] actual_bb: f32, used_bb: u16, prominent: bool },
    AsymmetricStacks { #[serde(with = "crate::numeric::finite_vec")] stacks_bb: Vec<f32>, prominent: bool },
    RakeProfileMapped { actual: String, used: String },
    StraddleMapped { #[serde(with = "crate::numeric::finite_array3")] posts: [f32; 3] },
    ShortHandedMapped { dealt: u8 },
    DeadlineBestSoFar { reached_bp: u16, target_bp: u16 },
    ChartRounded,
    EvReferenceUnverified,
    UnconditionedPriorStreet { street: Street, seat: Seat, cause: String },
    UnconditionedCurrentStreet,
    MultiwayStreetRoot { folded_this_street: u8, dead_this_street: u32 },
    SprBucketed { #[serde(with = "crate::numeric::finite")] actual: f32, #[serde(with = "crate::numeric::finite")] used: f32 },
    MenuRounded { #[serde(with = "crate::numeric::finite")] max_delta_pct: f32 },
    BranchResidual { seat: Seat, #[serde(with = "crate::numeric::finite")] residual_mass_pct: f32, cause: String },
}

macro_rules! approx_reason_convert {
    ($src:ident, $dst:ident, $v:expr) => {
        match $v {
            $src::BetTranslation { street, seat, observed_pct, mapped, deviation, prominent } =>
                $dst::BetTranslation { street, seat, observed_pct, mapped, deviation, prominent },
            $src::DepthBucket { seat, actual_bb, used_bb, prominent } => $dst::DepthBucket { seat, actual_bb, used_bb, prominent },
            $src::AsymmetricStacks { stacks_bb, prominent } => $dst::AsymmetricStacks { stacks_bb, prominent },
            $src::RakeProfileMapped { actual, used } => $dst::RakeProfileMapped { actual, used },
            $src::StraddleMapped { posts } => $dst::StraddleMapped { posts },
            $src::ShortHandedMapped { dealt } => $dst::ShortHandedMapped { dealt },
            $src::DeadlineBestSoFar { reached_bp, target_bp } => $dst::DeadlineBestSoFar { reached_bp, target_bp },
            $src::ChartRounded => $dst::ChartRounded,
            $src::EvReferenceUnverified => $dst::EvReferenceUnverified,
            $src::UnconditionedPriorStreet { street, seat, cause } => $dst::UnconditionedPriorStreet { street, seat, cause },
            $src::UnconditionedCurrentStreet => $dst::UnconditionedCurrentStreet,
            $src::MultiwayStreetRoot { folded_this_street, dead_this_street } => $dst::MultiwayStreetRoot { folded_this_street, dead_this_street },
            $src::SprBucketed { actual, used } => $dst::SprBucketed { actual, used },
            $src::MenuRounded { max_delta_pct } => $dst::MenuRounded { max_delta_pct },
            $src::BranchResidual { seat, residual_mass_pct, cause } => $dst::BranchResidual { seat, residual_mass_pct, cause },
        }
    };
}

impl From<ApproxReason> for ApproxReasonJson {
    fn from(a: ApproxReason) -> Self { approx_reason_convert!(ApproxReason, ApproxReasonJson, a) }
}
impl From<ApproxReasonJson> for ApproxReason {
    fn from(a: ApproxReasonJson) -> Self { approx_reason_convert!(ApproxReasonJson, ApproxReason, a) }
}
impl From<ApproxReason> for ApproxReasonBincode {
    fn from(a: ApproxReason) -> Self { approx_reason_convert!(ApproxReason, ApproxReasonBincode, a) }
}
impl From<ApproxReasonBincode> for ApproxReason {
    fn from(a: ApproxReasonBincode) -> Self { approx_reason_convert!(ApproxReasonBincode, ApproxReason, a) }
}

impl Serialize for ApproxReason {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() { ApproxReasonJson::from(self.clone()).serialize(s) } else { ApproxReasonBincode::from(self.clone()).serialize(s) }
    }
}
impl<'de> Deserialize<'de> for ApproxReason {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<ApproxReason, D::Error> {
        if d.is_human_readable() { ApproxReasonJson::deserialize(d).map(ApproxReason::from) } else { ApproxReasonBincode::deserialize(d).map(ApproxReason::from) }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum UnsupportedReason {
    MultiwayEv { pot_eligible: u8 },
    MissingPreflopNode { key: String },
    HeroComboOutOfSupport,
    EngineError { message: String, retryable: bool },
    TreeTooLarge { estimate_bytes: u64 },
    DeadlineExceeded { stage: String },
    InvalidRanges,
    UnsupportedHistory { reason: String },
    FormatUnsupported { detail: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Coverage {
    Exact,
    Approximate { reasons: Vec<ApproxReason> },
    Unsupported { reason: UnsupportedReason, partial: Vec<ApproxReason> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Unavailable {
    NotInMenu, NoEvReference, HeroOutOfSupport, MovedProbability { from: Action }, ChartNoEv, NotEvaluated, Pending,
    BranchSupportIncomplete { #[serde(with = "crate::numeric::probability")] #[cfg_attr(feature = "typescript", ts(as = "f32"))] covered_posterior: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ActionAdvice {
    pub action: Action,
    #[serde(default, with = "crate::numeric::probability_opt")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<f32>"))]
    pub frequency: Option<f32>,
    #[serde(default, with = "crate::numeric::finite_opt")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<f32>"))]
    pub ev_bb: Option<f32>,
    pub unavailable: Option<Unavailable>,
    pub headline: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Availability { Ready, Pending, Unavailable { reason: String } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum EquityMethod { Exact, MonteCarlo { samples: u32, #[serde(with = "crate::numeric::probability")] #[cfg_attr(feature = "typescript", ts(as = "f32"))] std_err: f32 } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct EquityEstimate {
    #[serde(default, with = "crate::numeric::probability_opt")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<f32>"))]
    pub value: Option<f32>,
    pub availability: Availability,
    pub method: Option<EquityMethod>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct PotShares { pub pot_index: u8, pub population: String, pub shares: Vec<(Seat, EquityEstimate)> }

pub const POT_SHARES_POPULATION: &str = "hero combo fixed; opponents jointly sampled, disjoint, from hero-conditioned public ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct EquitySummary { pub hero_combo_vs_each: Vec<(Seat, EquityEstimate)>, pub hero_range_vs_each: Vec<(Seat, EquityEstimate)>, pub per_pot_shares: Vec<PotShares> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Assumptions {
    #[serde(with = "crate::numeric::mass_triples")]
    #[cfg_attr(feature = "typescript", ts(as = "Vec<(Seat, String, f32)>"))]
    pub ranges_used: Vec<(Seat, String, f32)>,
    pub tree_signature: String, pub template_id: String,
    pub source: String, pub source_accuracy: String, pub source_granularity: String,
    pub target_bp: u16, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub cache: String,
    pub translations: Vec<ApproxReason>, pub mappings: Vec<ApproxReason>, pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ExperimentalHu {
    pub opponent: Seat, pub hero_role: String, pub pot: u32, pub stack: u32, pub template_id: String,
    #[serde(with = "crate::numeric::mass_pair")]
    #[cfg_attr(feature = "typescript", ts(as = "[(Seat, String, f32); 2]"))]
    pub ranges_used: [(Seat, String, f32); 2],
    pub actions: Vec<ActionAdvice>, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub note: String,
}

pub const EXPERIMENTAL_NOTE: &str = "experimental, not solved: synthetic root, empty history, unconditioned ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ExploitAdvice {
    #[serde(with = "crate::numeric::probability")]
    #[cfg_attr(feature = "typescript", ts(as = "f32"))]
    pub alpha: f32,
    pub model_revision: u32, pub model_summary: String, pub gto_action: Action, pub exploit_action: Action,
    #[serde(default, with = "crate::numeric::finite_opt")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<f32>"))]
    pub ev_delta_bb: Option<f32>,
    pub locked_nodes: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Phase { Fast, Provisional, Final }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Recommendation {
    pub identity: DecisionIdentity, pub phase: Phase, pub coverage: Coverage, pub legal: Vec<LegalAction>,
    pub actions: Vec<ActionAdvice>,
    #[serde(with = "crate::numeric::probability")]
    #[cfg_attr(feature = "typescript", ts(as = "f32"))]
    pub unresolved_mass: f32,
    #[serde(default, with = "crate::numeric::action_weights_opt")]
    #[cfg_attr(feature = "typescript", ts(as = "Option<Vec<(Action, f32)>>"))]
    pub range_mix: Option<Vec<(Action, f32)>>,
    pub equity: EquitySummary, pub assumptions: Assumptions, pub experimental: Option<ExperimentalHu>, pub exploit: Option<ExploitAdvice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum RecommendationEvent {
    Fast(Recommendation),
    Equity { identity: DecisionIdentity, equity: EquitySummary },
    Progress {
        identity: DecisionIdentity, stage: String, iterations: u32,
        #[serde(default, with = "crate::numeric::non_negative_opt")] #[cfg_attr(feature = "typescript", ts(as = "Option<f32>"))] exploitability_pct: Option<f32>,
        elapsed_ms: u32,
    },
    Provisional(Recommendation),
    Final(Recommendation),
    NoDecision { identity: DecisionIdentity, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reasons_and_events_roundtrip() {
        let cov = Coverage::Approximate { reasons: vec![
            ApproxReason::BetTranslation { street: Street::Flop, seat: Seat(2), observed_pct: 0.73, mapped: vec![(0.5, 0.468), (1.0, 0.532)], deviation: 0.23, prominent: true },
            ApproxReason::MultiwayStreetRoot { folded_this_street: 1, dead_this_street: 50 },
            ApproxReason::ChartRounded,
        ] };
        let text = serde_json::to_string(&cov).unwrap();
        assert!(text.contains(r#""kind":"BetTranslation""#));
        assert!(text.contains(r#""kind":"ChartRounded""#));
        let back: Coverage = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cov);
        let uns = Coverage::Unsupported { reason: UnsupportedReason::UnsupportedHistory { reason: "multiway street root not reproducible at step 1".into() }, partial: vec![] };
        assert_eq!(serde_json::from_str::<Coverage>(&serde_json::to_string(&uns).unwrap()).unwrap(), uns);
        let id = DecisionIdentity { hand_id: 1, hand_revision: 7, decision_id: 3, config_revision: 1, model_revision: 0 };
        let ev = RecommendationEvent::NoDecision { identity: id.clone(), reason: "hero all-in".into() };
        assert_eq!(serde_json::from_str::<RecommendationEvent>(&serde_json::to_string(&ev).unwrap()).unwrap(), ev);
        let est = EquityEstimate { value: Some(0.25), availability: Availability::Ready, method: Some(EquityMethod::MonteCarlo { samples: 100_000, std_err: 0.0014 }) };
        assert_eq!(serde_json::from_str::<EquityEstimate>(&serde_json::to_string(&est).unwrap()).unwrap(), est);
    }

    #[test]
    fn asymmetric_stacks_prominent_flag_roundtrips() {
        let not_prominent = ApproxReason::AsymmetricStacks { stacks_bb: vec![100.0, 104.0], prominent: false };
        let text = serde_json::to_string(&not_prominent).unwrap();
        assert!(text.contains(r#""prominent":false"#), "expected prominent:false in {text}");
        assert_eq!(serde_json::from_str::<ApproxReason>(&text).unwrap(), not_prominent);

        let prominent = ApproxReason::AsymmetricStacks { stacks_bb: vec![100.0, 110.0], prominent: true };
        let text = serde_json::to_string(&prominent).unwrap();
        assert!(text.contains(r#""prominent":true"#), "expected prominent:true in {text}");
        assert_eq!(serde_json::from_str::<ApproxReason>(&text).unwrap(), prominent);
    }
}
