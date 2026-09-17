use serde::{Deserialize, Serialize};
use crate::hand::{Action, LegalAction, Seat, Street};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecisionIdentity { pub hand_id: u64, pub hand_revision: u32, pub decision_id: u64, pub config_revision: u32, pub model_revision: u32 }

// Every `f32` and `Option<f32>` in this module crosses the Tauri IPC boundary to the UI, so each
// one carries a checked codec from `crate::numeric` (review S1): the value is validated in its wide
// f64 form before narrowing on the way in, and rejected rather than emitted as JSON `null` on the
// way out. Without it a required `f32` holding NaN makes the whole event undeserializable, and an
// `Option<f32>` holding `Some(NaN)` silently round-trips to `None` -- the advice just vanishes.
// `None` stays the one and only nullable value.

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ApproxReason {
    BetTranslation {
        street: Street,
        seat: Seat,
        #[serde(with = "crate::numeric::finite")] observed_pct: f32,
        #[serde(with = "crate::numeric::mapped_sizes")] mapped: Vec<(f32, f32)>,
        #[serde(with = "crate::numeric::probability")] deviation: f32,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
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
pub enum Coverage {
    Exact,
    Approximate { reasons: Vec<ApproxReason> },
    Unsupported { reason: UnsupportedReason, partial: Vec<ApproxReason> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Unavailable {
    NotInMenu, NoEvReference, HeroOutOfSupport, MovedProbability { from: Action }, ChartNoEv, NotEvaluated, Pending,
    BranchSupportIncomplete { #[serde(with = "crate::numeric::probability")] covered_posterior: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionAdvice {
    pub action: Action,
    #[serde(default, with = "crate::numeric::probability_opt")]
    pub frequency: Option<f32>,
    #[serde(default, with = "crate::numeric::finite_opt")]
    pub ev_bb: Option<f32>,
    pub unavailable: Option<Unavailable>,
    pub headline: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Availability { Ready, Pending, Unavailable { reason: String } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum EquityMethod { Exact, MonteCarlo { samples: u32, #[serde(with = "crate::numeric::probability")] std_err: f32 } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquityEstimate {
    #[serde(default, with = "crate::numeric::probability_opt")]
    pub value: Option<f32>,
    pub availability: Availability,
    pub method: Option<EquityMethod>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PotShares { pub pot_index: u8, pub population: String, pub shares: Vec<(Seat, EquityEstimate)> }

pub const POT_SHARES_POPULATION: &str = "hero combo fixed; opponents jointly sampled, disjoint, from hero-conditioned public ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct EquitySummary { pub hero_combo_vs_each: Vec<(Seat, EquityEstimate)>, pub hero_range_vs_each: Vec<(Seat, EquityEstimate)>, pub per_pot_shares: Vec<PotShares> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assumptions {
    #[serde(with = "crate::numeric::mass_triples")]
    pub ranges_used: Vec<(Seat, String, f32)>,
    pub tree_signature: String, pub template_id: String,
    pub source: String, pub source_accuracy: String, pub source_granularity: String,
    pub target_bp: u16, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub cache: String,
    pub translations: Vec<ApproxReason>, pub mappings: Vec<ApproxReason>, pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentalHu {
    pub opponent: Seat, pub hero_role: String, pub pot: u32, pub stack: u32, pub template_id: String,
    #[serde(with = "crate::numeric::mass_pair")]
    pub ranges_used: [(Seat, String, f32); 2],
    pub actions: Vec<ActionAdvice>, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub note: String,
}

pub const EXPERIMENTAL_NOTE: &str = "experimental, not solved: synthetic root, empty history, unconditioned ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExploitAdvice {
    #[serde(with = "crate::numeric::probability")]
    pub alpha: f32,
    pub model_revision: u32, pub model_summary: String, pub gto_action: Action, pub exploit_action: Action,
    #[serde(default, with = "crate::numeric::finite_opt")]
    pub ev_delta_bb: Option<f32>,
    pub locked_nodes: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase { Fast, Provisional, Final }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub identity: DecisionIdentity, pub phase: Phase, pub coverage: Coverage, pub legal: Vec<LegalAction>,
    pub actions: Vec<ActionAdvice>,
    #[serde(with = "crate::numeric::probability")]
    pub unresolved_mass: f32,
    #[serde(default, with = "crate::numeric::action_weights_opt")]
    pub range_mix: Option<Vec<(Action, f32)>>,
    pub equity: EquitySummary, pub assumptions: Assumptions, pub experimental: Option<ExperimentalHu>, pub exploit: Option<ExploitAdvice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RecommendationEvent {
    Fast(Recommendation),
    Equity { identity: DecisionIdentity, equity: EquitySummary },
    Progress {
        identity: DecisionIdentity, stage: String, iterations: u32,
        #[serde(default, with = "crate::numeric::non_negative_opt")] exploitability_pct: Option<f32>,
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
