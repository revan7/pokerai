//! Street root and the multiway projection rule (spec 3.5, 4.3, 10.2).
//!
//! [`street_root`] turns the current hero decision into the [`StreetRootSnapshot`] the engine solves
//! from: the board, the two pot-eligible seats in their OOP/IP roles, the matched pot and both
//! remaining stacks as the street opened, and the ordered current-street history of those two seats.
//! The snapshot is financial only — no ranges (spec 2) — and hero's cards never enter it.
//!
//! A street that opened with three or more pot-eligible players and is heads-up at the decision is
//! projected to a HU root under the exact-reproduction rule of spec 10.2: the players who folded on
//! this street are removed, their actions are dropped from `history`, their current-street chips are
//! added to the root as `dead_this_street`, and the two survivors' actions are kept in order. The
//! projection is admitted **only** if [`replay_root`] is legal at every step and reproduces the
//! decision exactly — total pot, both survivors' current-street contributions and remaining stacks,
//! facing amount, `last_full_raise`, actor and legal action set. Anything else is
//! [`RootError::ProjectionNotReproducing`] naming the 1-based step that failed. No other multiway
//! projection exists in phase 1, and a root is never silently adjusted to make one fit.

use proto::{Derived, HandPhase, HandState, Pot, Seat, Street, StreetRootSnapshot};
use crate::betting::Round;
use crate::error::RulesError;
use crate::lifecycle::{simulate, Sim};
use crate::positions::postflop_order;
use crate::state::is_decision_point;

/// Why the current decision has no street root (spec 3.5, 10.2).
///
/// `Multiway`, `ProjectionNotReproducing` and `NoDecision` are the spec's three variants; `Preflop`
/// and `Inconsistent` name the two cases the engine maps itself — a preflop decision has no street
/// root to build, and a genuine HU root that does not replay is an `EngineError`, never a root the
/// engine adjusts to fit.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RootError {
    /// Three or more pot-eligible players at the decision: `Unsupported` for numeric EV (spec 6).
    #[error("{pot_eligible} pot-eligible players at the decision point")]
    Multiway { pot_eligible: u8 },
    /// The street opened multiway and the projection of spec 10.2 fails at this 1-based step; the step
    /// after the last one names the final comparison against the decision.
    #[error("multiway street root not reproducible at step {step}")]
    ProjectionNotReproducing { step: u32 },
    /// Not a hero decision point (spec 2): hero is not to act, hero's cards are unknown, or the hand
    /// is not in a betting round.
    #[error("not a hero decision point")]
    NoDecision,
    /// A preflop decision; street roots exist only postflop.
    #[error("street roots exist only postflop")]
    Preflop,
    /// A genuine HU root (or the hand itself) that does not replay to the stored decision. `step` is
    /// 1-based over `history`; `0` names the root itself, before its first action.
    #[error("genuine HU root does not replay to the decision state at step {step}")]
    Inconsistent { step: u32 },
}

fn illegal(reason: impl Into<String>) -> RulesError { RulesError::IllegalAction { reason: reason.into() } }

/// The hand as `street` opened: the settled pots and both stacks before the street's first action.
///
/// The prefix keeps only the earlier streets' actions and only the cards those streets reached, so
/// [`simulate`] stops exactly at the street start — where `Round::committed` is all zero and the
/// settled pots are the root pot.
fn simulate_street_start(state: &HandState, street: Street) -> Result<Sim, RulesError> {
    let mut prefix = state.clone();
    prefix.actions.retain(|a| a.street < street);
    prefix.board.truncate(street.board_len());
    prefix.phase = HandPhase::Betting { street };
    simulate(&prefix)
}

/// [`Round::open`]'s preconditions, plus the chip bound the replay's sums rely on, as typed errors.
///
/// A [`StreetRootSnapshot`] is a wire type and can reach this crate from outside it (deserialized, or
/// built by a caller), while `Round`'s preconditions are always-on assertions — the same split as
/// `lifecycle`'s hand admission: everything that would abort the process is turned into a
/// [`RulesError`] here. The chip bound (`pot_root + dead_this_street` and both root stacks together at
/// most `u32::MAX`, summed in `u64` before it is compared) is what makes every later `u32` sum exact,
/// because no seat ever commits more than its root stack.
fn admit_snapshot(snap: &StreetRootSnapshot) -> Result<(), RulesError> {
    let invalid = |reason: String| RulesError::InvalidConfig { reason };
    if snap.street == Street::Preflop {
        return Err(invalid("a street root is postflop; preflop opens on the forced posts".into()));
    }
    for seat in [snap.oop, snap.ip] {
        if seat.0 >= 6 { return Err(invalid(format!("seat {} is outside 0..6", seat.0))); }
    }
    if snap.oop == snap.ip { return Err(invalid(format!("seat {} is both OOP and IP", snap.oop.0))); }
    if snap.bb_chips == 0 { return Err(invalid("the minimum bet is at least one chip".into())); }
    if snap.stack_oop_root == 0 || snap.stack_ip_root == 0 {
        return Err(invalid("a seat with no chips at the root cannot act on the street".into()));
    }
    let total = u64::from(snap.pot_root) + u64::from(snap.dead_this_street) + u64::from(snap.stack_oop_root) + u64::from(snap.stack_ip_root);
    if total > u64::from(u32::MAX) {
        return Err(invalid(format!("the root holds {total} chips, above the model's bound of {}", u32::MAX)));
    }
    Ok(())
}

/// [`replay_root`] with the failing step separated from the error, for [`street_root`]'s two labels.
fn replay_root_steps(snap: &StreetRootSnapshot) -> Result<Derived, (u32, RulesError)> {
    admit_snapshot(snap).map_err(|e| (0, e))?;
    let (o, i) = (usize::from(snap.oop.0), usize::from(snap.ip.0));
    let mut stacks = [0u32; 6];
    stacks[o] = snap.stack_oop_root;
    stacks[i] = snap.stack_ip_root;
    // Everyone else is out of this street entirely: the folded flag is what keeps them out of the
    // action order and out of `Round`'s live-seat assertions.
    let mut folded = [true; 6];
    folded[o] = false;
    folded[i] = false;
    let mut round = Round::open(snap.street, vec![snap.oop, snap.ip], stacks, folded, [false; 6], snap.bb_chips);
    for (k, (seat, action)) in snap.history.iter().enumerate() {
        let step = u32::try_from(k).expect("a street holds far fewer actions than u32::MAX") + 1;
        // An action once the street has closed finds no seat to act: `Round::apply` reports
        // `NotBetting`, carrying this step, rather than reopening a settled street.
        let (recorded, _) = round.apply(*seat, *action).map_err(|e| (step, e))?;
        if recorded != *action {
            return Err((step, illegal(format!("{action:?} is recorded as {recorded:?}"))));
        }
    }
    // Admission bounds `pot_root + dead_this_street` plus both root stacks by `u32::MAX`, and no seat
    // commits more than its root stack, so both sums are exact in `u32` (wide first, then narrowed).
    let root_pot = snap.pot_root + snap.dead_this_street;
    let betting = !round.closed();
    Ok(Derived {
        street: snap.street,
        to_act: if betting { round.to_act() } else { None },
        pot: root_pot + round.committed.iter().sum::<u32>(),
        committed_this_street: round.committed.to_vec(),
        stacks_remaining: round.stacks.to_vec(),
        folded: round.folded.to_vec(),
        all_in: (0..6).map(|j| !round.folded[j] && round.stacks[j] == 0).collect(),
        facing: round.facing,
        last_full_raise: round.last_full_raise,
        // The projected world's one settled pot: dead money is part of the root there (spec 10.2),
        // and the street's live commitments stay outside it, as everywhere else in this crate.
        pots: vec![Pot { amount: root_pot, eligible: vec![snap.oop.min(snap.ip), snap.oop.max(snap.ip)] }],
        legal: if betting { round.legal() } else { vec![] },
    })
}

/// Replays `history` from the snapshot as a HU street (spec 3.5); an illegal step is reported as
/// [`RulesError::IllegalAction`] naming its 1-based step, and a snapshot that is not a well-formed
/// root at all keeps [`admit_snapshot`]'s own error, because no step of the history was reached.
///
/// The result describes the street as the history leaves it. A history that closes the street yields
/// `to_act: None` and no legal actions, before street closure: no uncalled portion is returned and no
/// pot is layered, because the engine reads this at a decision point, where the street is still open.
/// The board is carried by the snapshot but takes no part in the replay.
pub fn replay_root(snap: &StreetRootSnapshot) -> Result<Derived, RulesError> {
    replay_root_steps(snap).map_err(|(step, e)| if step == 0 { e } else { illegal(format!("step {step}: {e}")) })
}

/// The quantities of spec 10.2 that the replay must reproduce, plus the legal action set.
///
/// `facing` and `last_full_raise` together are the minimum raise, and they also decide what reopens
/// the action later in the street, which is why both are compared and not just the legal set.
fn same_decision(a: &Derived, b: &Derived, oop: Seat, ip: Seat) -> bool {
    let (o, i) = (usize::from(oop.0), usize::from(ip.0));
    a.pot == b.pot
        && a.committed_this_street[o] == b.committed_this_street[o]
        && a.committed_this_street[i] == b.committed_this_street[i]
        && a.stacks_remaining[o] == b.stacks_remaining[o]
        && a.stacks_remaining[i] == b.stacks_remaining[i]
        && a.facing == b.facing
        && a.last_full_raise == b.last_full_raise
        && a.to_act == b.to_act
        && a.legal == b.legal
}

/// The street-root snapshot of the current hero decision, projected to HU when spec 10.2 admits it.
///
/// Returns [`RootError::NoDecision`] away from a hero decision point, [`RootError::Preflop`] preflop,
/// [`RootError::Multiway`] with three or more pot-eligible players (never an approximation, spec 6),
/// [`RootError::ProjectionNotReproducing`] when a multiway street root does not project, and
/// [`RootError::Inconsistent`] when the state or a genuine HU root does not replay to the stored
/// decision — an `EngineError` for the caller, never a root adjusted to fit.
pub fn street_root(state: &HandState) -> Result<StreetRootSnapshot, RootError> {
    if !is_decision_point(state) { return Err(RootError::NoDecision); }
    let d = &state.derived;
    let street = d.street;
    if street == Street::Preflop { return Err(RootError::Preflop); }
    // Ground truth. `is_decision_point` reads the stored `derived`, which an externally supplied state
    // can disagree with; requiring the hand to replay to exactly that decision is also what bounds
    // every chip sum below by the starting stacks (`lifecycle`'s admission).
    let sim = simulate(state).map_err(|_| RootError::Inconsistent { step: 0 })?;
    if sim.derived() != *d { return Err(RootError::Inconsistent { step: 0 }); }
    let order = postflop_order(state.button, &state.dealt);
    let eligible: Vec<Seat> = order.iter().copied().filter(|s| !d.folded[usize::from(s.0)]).collect();
    // A betting street has not closed, so at least two seats are still pot-eligible (`Sim::close`
    // ends the hand at one); a third makes the decision multiway.
    assert!(eligible.len() >= 2, "a betting street has at least two pot-eligible seats");
    if eligible.len() != 2 {
        return Err(RootError::Multiway { pot_eligible: u8::try_from(eligible.len()).expect("at most six dealt seats") });
    }
    let (oop, ip) = (eligible[0], eligible[1]);
    let start = simulate_street_start(state, street).map_err(|_| RootError::Inconsistent { step: 0 })?;
    let projected_from = u8::try_from(start.round.eligible_count()).expect("at most six dealt seats");
    // Dead money: this street's chips of every seat that is not one of the two survivors. A seat that
    // folded on an earlier street contributed nothing this street and already sits inside `pot_root`;
    // a seat that folded on this one keeps its chips in `Round::committed` until street closure, so
    // they are counted here exactly once.
    let dead: u32 = (0..6).filter(|j| *j != usize::from(oop.0) && *j != usize::from(ip.0)).map(|j| sim.round.committed[j]).sum();
    let snapshot = StreetRootSnapshot {
        street,
        board: state.board.clone(),
        oop,
        ip,
        pot_root: start.pots.iter().map(|p| p.amount).sum(),
        stack_oop_root: start.round.stacks[usize::from(oop.0)],
        stack_ip_root: start.round.stacks[usize::from(ip.0)],
        dead_this_street: dead,
        projected_from,
        history: state.actions.iter().filter(|a| a.street == street && (a.seat == oop || a.seat == ip)).map(|a| (a.seat, a.action)).collect(),
        bb_chips: state.config.bb_chips,
    };
    // A genuine HU root that does not replay is this crate's own inconsistency; a projected one that
    // does not replay is the spec 10.2 rejection, and the decision is `Unsupported` with equity only.
    let genuine = projected_from == 2;
    let fail = |step: u32| if genuine { RootError::Inconsistent { step } } else { RootError::ProjectionNotReproducing { step } };
    let replayed = replay_root_steps(&snapshot).map_err(|(step, _)| fail(step))?;
    if !same_decision(&replayed, d, oop, ip) {
        // Every step was legal, but the street the survivors alone played is not this decision.
        let last = u32::try_from(snapshot.history.len()).expect("a street holds far fewer actions than u32::MAX");
        return Err(fail(last + 1));
    }
    Ok(snapshot)
}
