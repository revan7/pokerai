use serde::{Deserialize, Serialize};
use crate::hand::{Action, LegalAction, Seat, Street};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct DecisionIdentity { pub hand_id: u64, pub hand_revision: u32, pub decision_id: u64, pub config_revision: u32, pub model_revision: u32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum ApproxReason {
    BetTranslation { street: Street, seat: Seat, observed_pct: f32, mapped: Vec<(f32, f32)>, deviation: f32, prominent: bool },
    DepthBucket { seat: Seat, actual_bb: f32, used_bb: u16, prominent: bool },
    AsymmetricStacks { stacks_bb: Vec<f32>, prominent: bool },
    RakeProfileMapped { actual: String, used: String },
    StraddleMapped { posts: [f32; 3] },
    ShortHandedMapped { dealt: u8 },
    DeadlineBestSoFar { reached_bp: u16, target_bp: u16 },
    ChartRounded,
    EvReferenceUnverified,
    UnconditionedPriorStreet { street: Street, seat: Seat, cause: String },
    UnconditionedCurrentStreet,
    MultiwayStreetRoot { folded_this_street: u8, dead_this_street: u32 },
    SprBucketed { actual: f32, used: f32 },
    MenuRounded { max_delta_pct: f32 },
    BranchResidual { seat: Seat, residual_mass_pct: f32, cause: String },
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
    BranchSupportIncomplete { covered_posterior: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ActionAdvice { pub action: Action, pub frequency: Option<f32>, pub ev_bb: Option<f32>, pub unavailable: Option<Unavailable>, pub headline: bool }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Availability { Ready, Pending, Unavailable { reason: String } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum EquityMethod { Exact, MonteCarlo { samples: u32, std_err: f32 } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct EquityEstimate { pub value: Option<f32>, pub availability: Availability, pub method: Option<EquityMethod> }

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
    pub ranges_used: Vec<(Seat, String, f32)>, pub tree_signature: String, pub template_id: String,
    pub source: String, pub source_accuracy: String, pub source_granularity: String,
    pub target_bp: u16, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub cache: String,
    pub translations: Vec<ApproxReason>, pub mappings: Vec<ApproxReason>, pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ExperimentalHu {
    pub opponent: Seat, pub hero_role: String, pub pot: u32, pub stack: u32, pub template_id: String,
    pub ranges_used: [(Seat, String, f32); 2], pub actions: Vec<ActionAdvice>, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub note: String,
}

pub const EXPERIMENTAL_NOTE: &str = "experimental, not solved: synthetic root, empty history, unconditioned ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct ExploitAdvice { pub alpha: f32, pub model_revision: u32, pub model_summary: String, pub gto_action: Action, pub exploit_action: Action, pub ev_delta_bb: Option<f32>, pub locked_nodes: u16 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Phase { Fast, Provisional, Final }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Recommendation {
    pub identity: DecisionIdentity, pub phase: Phase, pub coverage: Coverage, pub legal: Vec<LegalAction>,
    pub actions: Vec<ActionAdvice>, pub unresolved_mass: f32, pub range_mix: Option<Vec<(Action, f32)>>,
    pub equity: EquitySummary, pub assumptions: Assumptions, pub experimental: Option<ExperimentalHu>, pub exploit: Option<ExploitAdvice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum RecommendationEvent {
    Fast(Recommendation),
    Equity { identity: DecisionIdentity, equity: EquitySummary },
    Progress { identity: DecisionIdentity, stage: String, iterations: u32, exploitability_pct: Option<f32>, elapsed_ms: u32 },
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
