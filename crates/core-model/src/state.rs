//! The public hand-state API (spec 3.5, 4.3): the only way a caller starts a hand, records an action,
//! enters a board or reads the derived view.
//!
//! Every fallible function here — [`begin_hand`], [`apply_action`], [`set_board`] and
//! [`set_hero_cards`] — rebuilds `phase` and `derived` by replaying the whole hand through
//! [`simulate`], and returns [`Err`] rather than store a state that does not replay. That is what lets
//! [`derive`], [`settle_pots`] and [`abandon`] have no `Result` (spec 3.5): they *require* a state that
//! replays — one this crate produced — and panic on any other. A `HandState` that came from outside the
//! crate — deserialized, or built by a caller — must be admitted with [`crate::lifecycle::simulate`],
//! which returns a `Result`, before any of the three is called on it.

use proto::{Action, Card, CardParseError, Derived, HandConfig, HandPhase, HandState, Seat, Street, TakenAction};
use crate::error::RulesError;
use crate::lifecycle::simulate;
use crate::positions::validate_table;
use crate::settlement::Settlement;

/// The internal input to [`begin_hand`]: the engine-assigned `hand_id` plus the table, exactly as spec
/// 4.3 describes `HandState`. `stacks_start` is aligned with `dealt`.
///
/// Distinct from [`proto::BeginHand`], the id-free admission DTO of spec 5 step 2, whose field is named
/// `stacks`; the engine takes the DTO, assigns the id and converts. A module that glob-imports both
/// crates disambiguates with `use core_model::state::BeginHand;`.
#[derive(Clone, Debug, PartialEq)]
pub struct BeginHand { pub hand_id: u64, pub button: Seat, pub hero: Seat, pub dealt: Vec<Seat>, pub stacks_start: Vec<u32>, pub hero_cards: Option<[Card; 2]> }

/// Replays `state` and stores the phase and derived view the replay reached. An `Err` leaves the caller's
/// copy untouched, because every caller here refreshes a clone and only returns it on `Ok`.
fn refresh(state: &mut HandState) -> Result<(), RulesError> {
    let sim = simulate(state)?;
    state.phase = sim.phase;
    state.derived = sim.derived();
    Ok(())
}

/// Hero's two cards, when they are on record: distinct and valid ids.
///
/// The board is admitted against hero's cards by [`simulate`], but nothing there validates hero's own
/// ids, so the two admission points for hero cards ([`begin_hand`] and [`set_hero_cards`]) check them
/// here: an out-of-range id would otherwise reach `Display` and serde, which index the rank/suit tables.
fn admit_hero_cards(cards: [Card; 2]) -> Result<(), RulesError> {
    Card::checked(cards[0].0)?;
    Card::checked(cards[1].0)?;
    if cards[0] == cards[1] { return Err(CardParseError::Duplicate(cards[0]).into()); }
    Ok(())
}

/// Admits a new hand (spec 4.3, 5 step 2) and returns it at its first decision, preflop, with the forced
/// posts made.
///
/// This is the admission point for a caller-supplied table: the dealt-seat and blind rules are
/// [`validate_table`]'s, hero must be one of the dealt seats, and the starting stacks must be one per
/// dealt seat and non-zero. The aggregate bound that settlement relies on — the starting stacks sum to at
/// most `u32::MAX` — is not re-implemented here; it is [`simulate`]'s, reached through [`refresh`], so a
/// `HandState` this function returns always replays.
pub fn begin_hand(cfg: &HandConfig, begin: BeginHand) -> Result<HandState, RulesError> {
    validate_table(cfg, begin.button, &begin.dealt)?;
    let invalid = |reason: &str| RulesError::InvalidConfig { reason: reason.to_string() };
    if !begin.dealt.contains(&begin.hero) { return Err(invalid("hero is not a dealt seat")); }
    if begin.stacks_start.len() != begin.dealt.len() { return Err(invalid("one starting stack per dealt seat")); }
    if begin.stacks_start.iter().any(|s| *s == 0) { return Err(invalid("starting stacks must be positive")); }
    if let Some(cards) = begin.hero_cards { admit_hero_cards(cards)?; }
    let mut state = HandState {
        hand_id: begin.hand_id, hand_revision: 0, config: cfg.clone(), phase: HandPhase::Betting { street: Street::Preflop },
        button: begin.button, hero: begin.hero, hero_cards: begin.hero_cards, dealt: begin.dealt, stacks_start: begin.stacks_start,
        board: vec![], actions: vec![], derived: Derived::default(),
    };
    refresh(&mut state)?;
    Ok(state)
}

/// Spec 3.5 gives `derive` no `Result`, so it is only ever called on a state this crate built: the
/// four fallible entry points (`begin_hand`, `apply_action`, `set_board`, `set_hero_cards`) all refresh
/// through `simulate` and return `Err` instead of storing an inconsistent state, and `abandon` requires
/// the state it is handed to replay already. Validate any externally supplied `HandState` with
/// `lifecycle::simulate` first.
///
/// # Panics
/// Panics (in every build profile) if `state` does not replay.
pub fn derive(state: &HandState) -> Derived {
    simulate(state).expect("a HandState built by core-model replays consistently").derived()
}

/// Applies an action for `Derived.to_act`; the recorded action is the normalized one (Task 10).
///
/// The action is validated against a fresh replay, so an action that is not legal at the current
/// decision — and any call at all once the hand is awaiting a board, complete or abandoned — is an `Err`
/// and leaves `state` untouched.
pub fn apply_action(state: &HandState, action: Action) -> Result<HandState, RulesError> {
    let mut sim = simulate(state)?;
    let HandPhase::Betting { street } = sim.phase else { return Err(RulesError::NotBetting) };
    let seat = sim.round.to_act().ok_or(RulesError::NotBetting)?;
    let (recorded, paid) = sim.round.apply(seat, action)?;
    let mut next = state.clone();
    next.actions.push(TakenAction { seat, street, action: recorded, paid });
    refresh(&mut next)?;
    Ok(next)
}

/// Replaces the full board (3, 4 or 5 cards); legal only in `AwaitingBoard`; the cards already on record must be kept.
///
/// The whole board is passed, not the new cards alone, because that is what `HandState` stores; the
/// `starts_with` check is what keeps a street already on record from being rewritten here (that is undo's
/// job). A hand that ended before the board was dealt — folded out, or an all-in runout, which spec 4.3
/// leaves in `Complete` — is not awaiting a board and is refused with [`RulesError::NotAwaitingBoard`].
pub fn set_board(state: &HandState, cards: &[Card]) -> Result<HandState, RulesError> {
    let HandPhase::AwaitingBoard { street } = state.phase else { return Err(RulesError::NotAwaitingBoard) };
    let bad = |reason: String| RulesError::BadBoard { reason };
    if cards.len() != street.board_len() { return Err(bad(format!("{street:?} needs {} cards, got {}", street.board_len(), cards.len()))); }
    if !cards.starts_with(&state.board) { return Err(bad("the earlier streets' cards differ from the board on record; use undo".into())); }
    for (i, c) in cards.iter().enumerate() {
        Card::checked(c.0)?;
        if cards[..i].contains(c) { return Err(bad(format!("duplicate card {c}"))); }
        if state.hero_cards.is_some_and(|h| h.contains(c)) { return Err(bad(format!("{c} is one of hero's cards"))); }
    }
    let mut next = state.clone();
    next.board = cards.to_vec();
    refresh(&mut next)?;
    Ok(next)
}

/// Records hero's two cards. They never enter a public range, a solve input or a cache key (spec 2); the
/// hand's money and phase do not depend on them, so this only narrows what a later board may contain.
pub fn set_hero_cards(state: &HandState, cards: [Card; 2]) -> Result<HandState, RulesError> {
    admit_hero_cards(cards)?;
    if let Some(c) = cards.iter().find(|c| state.board.contains(c)) { return Err(RulesError::BadBoard { reason: format!("{c} is on the board") }); }
    let mut next = state.clone();
    next.hero_cards = Some(cards);
    refresh(&mut next)?;
    Ok(next)
}

/// Settled pots and every refund made so far in the hand, in order.
/// Same precondition as [`derive`]: only for states this crate built.
///
/// # Panics
/// Panics (in every build profile) if `state` does not replay.
pub fn settle_pots(state: &HandState) -> Settlement {
    let sim = simulate(state).expect("a HandState built by core-model replays consistently");
    Settlement { pots: sim.pots, returned: sim.returned }
}

/// Spec section 2: hero to act, two hero cards on record, at least two legal actions.
pub fn is_decision_point(state: &HandState) -> bool {
    matches!(state.phase, HandPhase::Betting { .. }) && state.derived.to_act == Some(state.hero) && state.hero_cards.is_some() && state.derived.legal.len() >= 2
}

/// Abandons the hand (spec 4.3): the history is kept, no seat is to act and no action is legal again.
///
/// Same precondition as [`derive`], which it calls: only for a state that replays.
///
/// # Panics
/// Panics (in every build profile) if `state` does not replay.
pub fn abandon(state: &HandState) -> HandState {
    let mut next = state.clone();
    next.phase = HandPhase::Abandoned;
    next.derived = derive(&next);
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Rake;

    fn cfg() -> HandConfig {
        HandConfig { config_revision: 1, sb_chips: 1, bb_chips: 2, straddle: None, rake: Rake::TimeCharge, chip_label: "$1".into() }
    }
    fn table(hero_cards: Option<[Card; 2]>) -> BeginHand {
        BeginHand { hand_id: 1, button: Seat(5), hero: Seat(5), dealt: vec![Seat(5), Seat(0), Seat(1)], stacks_start: vec![200, 200, 200], hero_cards }
    }

    /// A `Card` is a public tuple struct, so an out-of-range id is constructible; `Display` indexes the
    /// rank/suit tables, so admission must reject the id before any error text can format it.
    #[test]
    fn hero_cards_are_admitted_before_an_error_can_format_them() {
        let err = begin_hand(&cfg(), table(Some([Card(60), Card(0)]))).unwrap_err();
        assert_eq!(err, RulesError::from(CardParseError::Id(60)));
        assert_eq!(err.to_string(), "card id 60 is outside 0..52");
        assert_eq!(begin_hand(&cfg(), table(Some([Card(0), Card(0)]))).unwrap_err(), RulesError::from(CardParseError::Duplicate(Card(0))));
        let s = begin_hand(&cfg(), table(None)).unwrap();
        assert_eq!(set_hero_cards(&s, [Card(52), Card(1)]).unwrap_err(), RulesError::from(CardParseError::Id(52)));
        assert_eq!(set_hero_cards(&s, [Card(1), Card(1)]).unwrap_err(), RulesError::from(CardParseError::Duplicate(Card(1))));
    }

    #[test]
    fn set_board_admits_card_ids_before_it_reports_a_duplicate() {
        let s = begin_hand(&cfg(), table(None)).unwrap();
        let s = apply_action(&s, Action::Call).unwrap();
        let s = apply_action(&s, Action::Call).unwrap();
        let s = apply_action(&s, Action::Check).unwrap();
        assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Flop });
        assert_eq!(set_board(&s, &[Card(60), Card(60), Card(0)]).unwrap_err(), RulesError::from(CardParseError::Id(60)));
    }
}
