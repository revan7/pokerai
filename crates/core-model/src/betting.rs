use std::collections::VecDeque;
use proto::{Action, LegalAction, Seat, Street};
use crate::error::RulesError;

/// One betting street (spec 4.3). Per-seat arrays are indexed by `Seat.0`; undealt seats are marked folded.
///
/// Invariants established by [`Round::open`] and preserved by every method:
/// `facing` is the largest wager faced this street — the largest of the nominal posts and the accepted
/// wagers — and never decreases; a short post leaves its poster's commitment below it, but
/// `committed[i] <= facing` still holds for every seat; `committed[i] + stacks[i]` is the seat's starting
/// stack and never changes, which is this street's share of the conservation invariant; `pending` holds
/// live seats only, in action order.
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub street: Street,
    pub order: Vec<Seat>,
    pub committed: [u32; 6],
    pub stacks: [u32; 6],
    pub folded: [bool; 6],
    pub all_in: [bool; 6],
    pub facing: u32,
    pub last_full_raise: u32,
    /// The wager each seat was facing when it last acted; `None` until it acts. Drives cumulative reopening.
    pub facing_at_last: [Option<u32>; 6],
    pub pending: VecDeque<Seat>,
}

fn illegal(reason: impl Into<String>) -> RulesError { RulesError::IllegalAction { reason: reason.into() } }

impl Round {
    /// Opens a street; `order` is the action order of the seats at the street start. Folded and all-in
    /// seats may appear in it — the lifecycle caller passes the whole dealt-seat order for the street
    /// along with the current flags — and are filtered out of `pending` here, so they are never given a
    /// turn (review R1).
    ///
    /// Infallible, so the caller's contract is enforced with always-on assertions: `order` lists every
    /// unfolded seat that still has chips, each seat at most once; a seat with no chips left is all-in;
    /// and the minimum bet is at least one chip.
    pub fn open(street: Street, order: Vec<Seat>, stacks: [u32; 6], folded: [bool; 6], all_in: [bool; 6], min_bet: u32) -> Round {
        assert!(min_bet > 0, "the minimum bet is at least one chip");
        assert!(!order.is_empty(), "a street needs at least one seat in the action order");
        let mut listed = [false; 6];
        for seat in &order {
            let i = usize::from(seat.0);
            assert!(i < 6, "seat {i} is out of range 0..6");
            assert!(!listed[i], "seat {i} appears twice in the action order");
            listed[i] = true;
        }
        for (i, listed) in listed.iter().enumerate() {
            assert!(!(folded[i] && all_in[i]), "seat {i} cannot be both folded and all-in");
            assert!(folded[i] || all_in[i] || stacks[i] > 0, "seat {i} has no chips but is neither folded nor all-in");
            assert!(folded[i] || all_in[i] || *listed, "live seat {i} is missing from the action order");
        }
        let pending = order.iter().copied().filter(|s| !folded[usize::from(s.0)] && !all_in[usize::from(s.0)]).collect();
        Round { street, order, committed: [0; 6], stacks, folded, all_in, facing: 0, last_full_raise: min_bet, facing_at_last: [None; 6], pending }
    }

    /// Posts a blind or straddle without consuming the seat's turn. A post above the stack is all-in for
    /// the stack, but the *nominal* amount is still the wager everyone else faces: a short blind does not
    /// lower the bring-in, so the others enter for the full blind and a raise is measured from it
    /// (review R2). No chips are manufactured — the poster's commitment stays capped at its stack and the
    /// uncalled remainder is returned at settlement.
    pub fn post(&mut self, seat: Seat, amount: u32) {
        let i = usize::from(seat.0);
        let paid = amount.min(self.stacks[i]);
        self.stacks[i] -= paid;
        self.committed[i] += paid;
        self.facing = self.facing.max(amount).max(self.committed[i]);
        if self.stacks[i] == 0 { self.all_in[i] = true; self.pending.retain(|s| *s != seat); }
    }

    pub fn to_act(&self) -> Option<Seat> { self.pending.front().copied() }
    pub fn closed(&self) -> bool { self.pending.is_empty() }
    pub fn live(&self, i: usize) -> bool { !self.folded[i] && !self.all_in[i] }
    /// Pot-eligible seats: unfolded, all-in included (spec 2).
    pub fn eligible_count(&self) -> usize { self.folded.iter().filter(|f| !**f).count() }

    /// The seat's all-in wager. Chips only move from `stacks` to `committed`, so this is the seat's
    /// starting stack for the street and cannot exceed `u32`.
    pub fn all_in_to(&self, i: usize) -> u32 {
        self.committed[i].checked_add(self.stacks[i]).expect("a seat never holds more than its starting stack")
    }

    /// `facing + last_full_raise` in `u64`: both terms can be as large as a full stack, so the sum
    /// does not fit `u32` at deep stacks and is compared before it is narrowed.
    fn min_raise_to_wide(&self) -> u64 { u64::from(self.facing) + u64::from(self.last_full_raise) }

    /// The smallest legal raise-to, saturating at `u32::MAX`; a minimum above `u32::MAX` is simply
    /// unreachable, and [`Round::legal`] and [`Round::apply`] compare the exact `u64` value.
    pub fn min_raise_to(&self) -> u32 { u32::try_from(self.min_raise_to_wide()).unwrap_or(u32::MAX) }

    /// What the seat still owes to match the highest wager (capped at its stack by the caller).
    fn owed(&self, i: usize) -> u32 {
        assert!(self.facing >= self.committed[i], "facing is the largest wager faced this street");
        self.facing - self.committed[i]
    }

    /// Spec 4.3: the seat can put more chips in, the action is (re)opened for it (cumulative rule) and some opponent can still respond.
    pub fn may_aggress(&self, i: usize) -> bool {
        if self.folded[i] || self.all_in_to(i) <= self.facing { return false; }
        let reopened = match self.facing_at_last[i] {
            None => true,
            Some(f) => {
                assert!(self.facing >= f, "facing never decreases within a street");
                self.facing - f >= self.last_full_raise
            }
        };
        let responder = (0..6).any(|j| j != i && !self.folded[j] && self.all_in_to(j) > self.facing);
        reopened && responder
    }

    /// The legal intervals for the seat to act, in the order Fold, Check, Call, Bet|Raise, AllIn.
    pub fn legal(&self) -> Vec<LegalAction> {
        let Some(seat) = self.to_act() else { return vec![] };
        let i = usize::from(seat.0);
        let owed = self.owed(i);
        let mut v = Vec::with_capacity(4);
        if owed > 0 { v.push(LegalAction::Fold); v.push(LegalAction::Call { cost: owed.min(self.stacks[i]) }); } else { v.push(LegalAction::Check); }
        if self.may_aggress(i) {
            let max_to = self.all_in_to(i);
            let min_wide = self.min_raise_to_wide();
            if min_wide <= u64::from(max_to) {
                let min_to = u32::try_from(min_wide).expect("checked against max_to, which is a u32");
                if self.facing == 0 { v.push(LegalAction::Bet { min_to, max_to }); } else { v.push(LegalAction::Raise { min_to, max_to }); }
            }
            v.push(LegalAction::AllIn { to: max_to });
        }
        v
    }

    /// Applies an action for `seat`; returns the recorded (normalized) action and the chips paid.
    pub fn apply(&mut self, seat: Seat, action: Action) -> Result<(Action, u32), RulesError> {
        let actor = self.to_act().ok_or(RulesError::NotBetting)?;
        if actor != seat { return Err(illegal(format!("seat {} is not to act (seat {} is)", seat.0, actor.0))); }
        let i = usize::from(seat.0);
        let owed = self.owed(i);
        let max_to = self.all_in_to(i);
        // A wager to the whole stack is recorded as an all-in; an all-in that only matches the
        // highest wager is recorded as a call.
        let action = match action {
            Action::AllIn { to } if to == max_to && to <= self.facing => Action::Call,
            Action::Bet { to } | Action::Raise { to } if to == max_to => Action::AllIn { to },
            a => a,
        };
        match action {
            Action::Fold => {
                if owed == 0 { return Err(illegal("fold with no wager to fold to")); }
                self.folded[i] = true;
                self.facing_at_last[i] = Some(self.facing);
                self.pending.pop_front();
                Ok((Action::Fold, 0))
            }
            Action::Check => {
                if owed != 0 { return Err(illegal(format!("check while facing {owed}"))); }
                self.facing_at_last[i] = Some(self.facing);
                self.pending.pop_front();
                Ok((Action::Check, 0))
            }
            Action::Call => {
                if owed == 0 { return Err(illegal("call with nothing owed")); }
                let paid = owed.min(self.stacks[i]);
                self.stacks[i] -= paid;
                self.committed[i] += paid;
                if self.stacks[i] == 0 { self.all_in[i] = true; }
                self.facing_at_last[i] = Some(self.facing);
                self.pending.pop_front();
                Ok((Action::Call, paid))
            }
            Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => {
                let is_all_in = matches!(action, Action::AllIn { .. });
                if matches!(action, Action::Bet { .. }) && self.facing != 0 { return Err(illegal("bet while a wager is pending; use raise")); }
                if matches!(action, Action::Raise { .. }) && self.facing == 0 { return Err(illegal("raise with no wager pending; use bet")); }
                if !self.may_aggress(i) { return Err(illegal("betting is not (re)opened for this seat")); }
                if is_all_in && to != max_to { return Err(illegal(format!("all-in must be to {max_to}, got {to}"))); }
                // Compared in u64: the minimum raise-to can exceed `u32` at deep stacks.
                if !is_all_in && (u64::from(to) < self.min_raise_to_wide() || to > max_to) {
                    return Err(illegal(format!("wager to {to} outside [{}, {max_to}]", self.min_raise_to_wide())));
                }
                assert!(to > self.facing, "an accepted wager always exceeds the wager it faces");
                let inc = to - self.facing;
                let paid = to - self.committed[i];
                self.stacks[i] -= paid;
                self.committed[i] = to;
                if self.stacks[i] == 0 { self.all_in[i] = true; }
                if inc >= self.last_full_raise { self.last_full_raise = inc; }
                self.facing = to;
                self.facing_at_last[i] = Some(to);
                let pos = self.order.iter().position(|s| *s == seat).expect("the actor is in the street order");
                let n = self.order.len();
                self.pending = (1..n).map(|k| self.order[(pos + k) % n]).filter(|s| self.live(usize::from(s.0))).collect();
                Ok((action, paid))
            }
        }
    }
}
