use serde::{Deserialize, Serialize};
use crate::hand::Seat;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UtgStraddle { pub amount_chips: u32 }

/// Rake rule (spec 2). `rate` is the same physical quantity as the worker wire's `rake_rate` and
/// carries the same half-open `[0, 1)` codec on both serde directions (review S1): `core-eval`'s
/// `terminal_payoff` reads it with no check of its own, so an `inf` or negative rate admitted here
/// would silently produce a non-finite or sign-flipped payoff.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rake {
    PotRake {
        #[serde(with = "crate::numeric::rake_rate")]
        rate: f32,
        cap_mchips: u32,
        no_flop_no_drop: bool,
    },
    TimeCharge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatTag { Unknown, Nit, Tag, LoosePassive, CallingStation, Lag, Maniac }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickFact { NeverFoldsRiver, RarelyBluffs, LimpsALot, Over3bets, FoldsToPressure }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeatConfig { pub seat: Seat, pub tag: Option<SeatTag>, pub facts: Vec<QuickFact> }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolverPrefs { pub threads: u8, pub target_bp: u16, pub flop_budget_s: u8 }

impl Default for SolverPrefs {
    fn default() -> Self { SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameConfig {
    pub config_revision: u32,
    pub chip_label: String,
    pub sb_chips: u32,
    pub bb_chips: u32,
    pub straddle: Option<UtgStraddle>,
    pub rake: Rake,
    pub seats: Vec<SeatConfig>,
    pub solver: SolverPrefs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandConfig {
    pub config_revision: u32,
    pub sb_chips: u32,
    pub bb_chips: u32,
    pub straddle: Option<UtgStraddle>,
    pub rake: Rake,
    pub chip_label: String,
}

impl HandConfig {
    pub fn from_game(g: &GameConfig) -> HandConfig {
        HandConfig { config_revision: g.config_revision, sb_chips: g.sb_chips, bb_chips: g.bb_chips, straddle: g.straddle, rake: g.rake, chip_label: g.chip_label.clone() }
    }
}
