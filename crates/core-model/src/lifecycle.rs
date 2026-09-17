//! Hand replay (spec 4.3): [`simulate`] rebuilds a hand from its recorded action list and board.
//!
//! [`simulate`] is the safe entry point for an externally supplied [`HandState`]: it admits the table,
//! replays every recorded action through [`Round`] and compares the normalized action and the chips
//! paid with what was recorded, so a state this crate did not build is an `Err` and never a panic.

use proto::{Action, Card, CardParseError, CompleteReason, Derived, HandPhase, HandState, Pot, Seat, Street};
use crate::betting::Round;
use crate::config::{initial_full_raise, posts};
use crate::error::RulesError;
use crate::positions::{postflop_order, preflop_order, ring, validate_table};
use crate::settlement::{check_conservation, layer_pots, refund_uncalled};

/// The replayed hand: the current (or last) betting round plus hand-level accounting.
#[derive(Clone, Debug)]
pub struct Sim {
    pub round: Round,
    pub phase: HandPhase,
    pub contributed: [u32; 6],
    pub pots: Vec<Pot>,
    /// One entry per street closure that returned an uncalled portion, in closure order.
    pub returned: Vec<(Seat, u32)>,
    pub start_total: u64,
}

fn seat_array<T: Copy>(state: &HandState, default: T, mut f: impl FnMut(usize) -> T) -> [T; 6] {
    let mut out = [default; 6];
    for (k, seat) in state.dealt.iter().enumerate() { out[seat.0 as usize] = f(k); }
    out
}

/// Hand admission (spec 4.3). [`Round::open`] and `settlement` are infallible and enforce their
/// preconditions with always-on assertions; this is where those preconditions are turned into typed
/// errors, so that an externally supplied state can never abort the process:
///
/// - the table shape and the blinds are [`validate_table`]'s (3..=6 distinct seats in 0..6, the button
///   among them, `0 < sb <= bb`, the straddle rules) — which is also what makes `bb_chips > 0` and the
///   preflop/postflop orders' indexing hold;
/// - one starting stack per dealt seat, each at least one chip, so no dealt seat opens a street with
///   no chips while it is neither folded nor all-in;
/// - the starting stacks sum to at most `u32::MAX` — `settlement`'s documented aggregate precondition,
///   which it does not re-check. The sum is taken in `u64` and compared there, never narrowed.
fn admit(state: &HandState) -> Result<(), RulesError> {
    let invalid = |reason: String| RulesError::InvalidConfig { reason };
    validate_table(&state.config, state.button, &state.dealt)?;
    if state.stacks_start.len() != state.dealt.len() {
        return Err(invalid(format!("{} starting stacks for {} dealt seats", state.stacks_start.len(), state.dealt.len())));
    }
    if let Some((k, _)) = state.stacks_start.iter().enumerate().find(|(_, s)| **s == 0) {
        return Err(invalid(format!("dealt seat {} starts with no chips", state.dealt[k].0)));
    }
    let total: u64 = state.stacks_start.iter().map(|s| u64::from(*s)).sum();
    if total > u64::from(u32::MAX) {
        return Err(invalid(format!("starting stacks sum to {total}, above the settlement bound of {}", u32::MAX)));
    }
    admit_board(state)
}

/// Board admission (spec 4.3): `HandState.board` is a street board — 0, 3, 4 or 5 cards — of valid,
/// distinct ids, none of them a card hero holds. Only the shape is checked here; that the board is the
/// one the replay actually reaches is checked against the replayed street in [`simulate`], because it
/// is not knowable before the actions are replayed.
fn admit_board(state: &HandState) -> Result<(), RulesError> {
    if !matches!(state.board.len(), 0 | 3 | 4 | 5) {
        return Err(RulesError::BadBoard { reason: format!("{} board cards; a street board is 0, 3, 4 or 5", state.board.len()) });
    }
    for (k, card) in state.board.iter().enumerate() {
        Card::checked(card.0)?;
        if state.board[..k].contains(card) { return Err(CardParseError::Duplicate(*card).into()); }
        if state.hero_cards.is_some_and(|hero| hero.contains(card)) {
            return Err(RulesError::BadBoard { reason: format!("board card {card} is one of hero's cards") });
        }
    }
    Ok(())
}

impl Sim {
    fn open_preflop(state: &HandState) -> Sim {
        let dealt = seat_array(state, false, |_| true);
        let stacks = seat_array(state, 0u32, |k| state.stacks_start[k]);
        let folded: [bool; 6] = std::array::from_fn(|i| !dealt[i]);
        let order = preflop_order(state.button, &state.dealt, state.config.straddle.is_some());
        let mut round = Round::open(Street::Preflop, order, stacks, folded, [false; 6], initial_full_raise(&state.config));
        let r = ring(state.button, &state.dealt);
        for (idx, chips) in posts(&state.config) { round.post(r[idx], chips); }
        Sim { round, phase: HandPhase::Betting { street: Street::Preflop }, contributed: [0; 6], pots: vec![], returned: vec![], start_total: state.stacks_start.iter().map(|s| u64::from(*s)).sum() }
    }

    fn check(&self) -> Result<(), RulesError> { check_conservation(&self.round.stacks, &self.round.committed, &self.pots, self.start_total) }

    /// A fold changes pot eligibility at once, not at the next closure (spec 4.3). `contributed` is
    /// untouched mid-street, so re-layering it under the new folded flags keeps every settled chip
    /// where it is — the folded seat's contributions stay in the pot as dead money — and only narrows
    /// the eligible sets, merging neighbouring layers that have become equal. The current street's
    /// live commitments stay out of the settled pots until [`Sim::close`].
    fn relayer_settled(&mut self) { self.pots = layer_pots(&self.contributed, &self.round.folded); }

    /// Spec 4.3: a street closes when no pot-eligible player still owes a response. Beyond the two
    /// cases [`Round`] reports (nobody left in `pending`, everyone but one folded), no response is
    /// possible once at most one pot-eligible seat still has chips — nobody can bet into it and it
    /// cannot be raised. It is then still a decision point only while that seat owes chips to the
    /// outstanding wager (call or fold); a pending Check is not a decision, so the street is closed.
    fn no_response_possible(&self) -> bool {
        let r = &self.round;
        let mut with_chips = (0..6).filter(|i| !r.folded[*i] && r.stacks[*i] > 0);
        match (with_chips.next(), with_chips.next()) {
            (_, Some(_)) => false,
            (None, _) => true,
            (Some(i), None) => r.committed[i] >= r.facing,
        }
    }

    /// Settles the street if it is closed. Called at every point the round can change — after the
    /// forced posts, after each replayed action and after a street is opened — so the returned
    /// snapshot is never left inside a street that has no decision remaining.
    fn settle_if_closed(&mut self) -> Result<(), RulesError> {
        if self.round.eligible_count() == 1 || self.round.closed() || self.no_response_possible() { self.close()?; }
        Ok(())
    }

    /// Street closure (spec 4.3): refund the uncalled portion, settle the pots, choose the next phase.
    fn close(&mut self) -> Result<(), RulesError> {
        if let Some(r) = refund_uncalled(&mut self.round.committed, &mut self.round.stacks) {
            self.round.all_in[r.0 .0 as usize] = false;
            self.returned.push(r);
        }
        self.check()?;
        for i in 0..6 { self.contributed[i] += self.round.committed[i]; self.round.committed[i] = 0; }
        self.pots = layer_pots(&self.contributed, &self.round.folded);
        self.check()?;
        let eligible = self.round.eligible_count();
        let with_chips = (0..6).filter(|i| !self.round.folded[*i] && self.round.stacks[*i] > 0).count();
        self.phase = if eligible == 1 { HandPhase::Complete { reason: CompleteReason::FoldedOut } }
            else if with_chips < 2 { HandPhase::Complete { reason: CompleteReason::AllInRunout } }
            else if let Some(next) = self.round.street.next() { HandPhase::AwaitingBoard { street: next } }
            else { HandPhase::Complete { reason: CompleteReason::ShowdownReached } };
        self.round.pending.clear();
        Ok(())
    }

    fn open_street(&mut self, state: &HandState, street: Street) {
        let order = postflop_order(state.button, &state.dealt);
        self.round = Round::open(street, order, self.round.stacks, self.round.folded, self.round.all_in, state.config.bb_chips);
        self.phase = HandPhase::Betting { street };
    }

    pub fn derived(&self) -> Derived {
        let r = &self.round;
        let betting = matches!(self.phase, HandPhase::Betting { .. });
        let street = match self.phase { HandPhase::Betting { street } | HandPhase::AwaitingBoard { street } => street, _ => r.street };
        Derived {
            street,
            to_act: if betting { r.to_act() } else { None },
            pot: self.pots.iter().map(|p| p.amount).sum::<u32>() + r.committed.iter().sum::<u32>(),
            committed_this_street: r.committed.to_vec(),
            stacks_remaining: r.stacks.to_vec(),
            folded: r.folded.to_vec(),
            all_in: (0..6).map(|i| !r.folded[i] && r.stacks[i] == 0).collect(),
            facing: r.facing,
            last_full_raise: r.last_full_raise,
            pots: self.pots.clone(),
            legal: if betting { r.legal() } else { vec![] },
        }
    }
}

/// Replays `state.actions` and `state.board` from the forced posts. Fails only on a state that this crate did not build.
pub fn simulate(state: &HandState) -> Result<Sim, RulesError> {
    admit(state)?;
    let mut sim = Sim::open_preflop(state);
    sim.check()?;
    sim.settle_if_closed()?;
    let mut k = 0;
    loop {
        match sim.phase {
            HandPhase::Betting { street } => {
                if k >= state.actions.len() { break; }
                let a = state.actions[k];
                k += 1;
                if a.street != street {
                    return Err(RulesError::IllegalAction { reason: format!("action {k} is on {:?} but the hand is on {:?}", a.street, street) });
                }
                let (recorded, paid) = sim.round.apply(a.seat, a.action)?;
                if recorded != a.action || paid != a.paid {
                    return Err(RulesError::IllegalAction { reason: format!("action {k} recorded as {:?}/{} replays as {:?}/{}", a.action, a.paid, recorded, paid) });
                }
                if recorded == Action::Fold { sim.relayer_settled(); }
                sim.check()?;
                sim.settle_if_closed()?;
            }
            HandPhase::AwaitingBoard { street } => {
                if state.board.len() >= street.board_len() { sim.open_street(state, street); sim.settle_if_closed()?; } else { break; }
            }
            HandPhase::Complete { .. } | HandPhase::Abandoned => break,
        }
    }
    if k < state.actions.len() { return Err(RulesError::NotBetting); }
    // The board must be exactly the one the replay reached: `round.street` is the street being bet, or
    // the last street bet once the hand is awaiting a board or complete. A shorter board would leave a
    // street unopened, a longer one would carry cards from a street this history never reached.
    let reached = sim.round.street.board_len();
    if state.board.len() != reached {
        return Err(RulesError::BadBoard {
            reason: format!("{} board cards recorded, but the replay reached {:?}, whose board is {reached} cards", state.board.len(), sim.round.street),
        });
    }
    if matches!(state.phase, HandPhase::Abandoned) { sim.phase = HandPhase::Abandoned; }
    Ok(sim)
}
