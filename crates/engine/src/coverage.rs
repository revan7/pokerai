//! The coverage classifier (spec 6): which row of the coverage table the current decision falls in, before
//! any range, tree or solve is built.
//!
//! [`classify`] reads only the hand's public, financial state. It decides whether there is a decision at
//! all (spec 2), whether it is preflop, whether it is multiway for numeric EV (three or more pot-eligible
//! players, any all-in counting: never approximated, spec 6), and for a heads-up postflop decision which
//! street root to solve from: the genuine HU root, or the spec 10.2 projection of a street that opened
//! multiway (`Approximate{MultiwayStreetRoot}`), or `Unsupported{UnsupportedHistory}` when that projection
//! does not reproduce the decision exactly. A classification is a claim about input and model matching
//! only, never about full-game GTO. Hero's cards are read only to establish that a decision exists; they
//! never enter the street root (`core_model::street_root` builds it from public state alone).

use core_model::{derive, street_root, RootError};
use proto::{ApproxReason, Derived, HandState, Seat, Street, StreetRootSnapshot, UnsupportedReason};

#[derive(Debug, Clone)]
pub enum Classification {
    NoDecision { reason: String },
    Preflop,
    HuStreet { root: StreetRootSnapshot, reasons: Vec<ApproxReason>, facing_allin: bool, opponent: Seat },
    Multiway { pot_eligible: u8 },
    Unsupported(UnsupportedReason),
}

/// Where `seat` sits in `Derived`'s per-seat vectors: `Seat.0`, because those vectors are indexed by seat id
/// (length 6, undealt seats folded), not by position in `state.dealt` — a gapped or unordered `dealt` list is a
/// legal table (`core_model::validate_table`). Panics (always) when `seat` is not dealt.
pub fn seat_index(state: &HandState, seat: Seat) -> usize {
    assert!(state.dealt.contains(&seat), "seat {} is not a dealt seat", seat.0);
    usize::from(seat.0)
}
/// §6: pot-eligible counting uses `Derived.folded` only; all-in players count.
pub fn pot_eligible(d: &Derived) -> u8 { d.folded.iter().filter(|f| !**f).count() as u8 }

/// §2 decision point: hero to act, two known hero cards, at least two legal actions.
pub fn decision_point(state: &HandState, d: &Derived) -> Result<(), String> {
    if !matches!(state.phase, proto::HandPhase::Betting { .. }) { return Err("hand is not in a betting phase".into()); }
    if d.to_act != Some(state.hero) { return Err("another seat is to act".into()); }
    if d.all_in[seat_index(state, state.hero)] { return Err("hero is all-in".into()); }
    if state.hero_cards.is_none() { return Err("hero cards unknown".into()); }
    if d.legal.len() < 2 { return Err("one legal action".into()); }
    Ok(())
}

/// The spec 6 row of the current decision. Same precondition as `core_model::derive`: `state` must replay (a
/// state `core_model` built), else this panics.
pub fn classify(state: &HandState) -> Classification {
    let d = derive(state);
    if let Err(reason) = decision_point(state, &d) { return Classification::NoDecision { reason }; }
    if d.street == Street::Preflop { return Classification::Preflop; }
    let n = pot_eligible(&d);
    if n >= 3 { return Classification::Multiway { pot_eligible: n }; }
    match street_root(state) {
        Ok(root) => {
            let reasons = if root.projected_from >= 3 { vec![ApproxReason::MultiwayStreetRoot { folded_this_street: root.projected_from - 2, dead_this_street: root.dead_this_street }] } else { vec![] };
            let opponent = if root.oop == state.hero { root.ip } else { root.oop };
            let facing_allin = d.facing > 0 && d.all_in[seat_index(state, opponent)];
            Classification::HuStreet { root, reasons, facing_allin, opponent }
        }
        Err(RootError::ProjectionNotReproducing { step }) => Classification::Unsupported(UnsupportedReason::UnsupportedHistory { reason: format!("multiway street root not reproducible at step {step}") }),
        Err(RootError::Multiway { pot_eligible }) => Classification::Multiway { pot_eligible },
        Err(RootError::NoDecision) => Classification::NoDecision { reason: "no decision at the street root".into() },
        Err(RootError::Preflop) => Classification::Preflop,
        // §10.2: a genuine HU root that does not replay is an engine defect, not an unsupported history.
        Err(RootError::Inconsistent { step }) => Classification::Unsupported(UnsupportedReason::EngineError { message: format!("street root does not replay at step {step}"), retryable: false }),
    }
}
