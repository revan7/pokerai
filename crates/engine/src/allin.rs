//! Spec section 6's facing-an-all-in analytic fallback: no solve, one closed form.
//!
//! `EV(call) = equity * (W - R) - C`, where `C` is what hero must put in to call, `W` is the pot
//! that will actually be matched at the showdown and `R` the rake taken from it. Folding is
//! exactly 0 chips, because everything already in the pot is sunk (spec section 2), and a tie
//! (`EV(call) == 0`) goes to calling.
//!
//! The equity is hero's **actual combo** against the opponent's hero-conditioned public range:
//! hero's strategic range at the root never enters this calculation, which is what
//! `facing_allin_golden` pins by passing one and asserting nothing moves.

use crate::equity::{hero_combo_run, NoEquity};
use proto::{Action, ActionAdvice, Card, EquityMethod, Rake, Range1326, UnsupportedReason};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct AllInInput {
    pub hero: [Card; 2],
    pub board: Vec<Card>,
    /// The opponent's public range. Read only, and never conditioned on hero's cards in place.
    pub opp_public: Range1326,
    /// Hero's strategic range at the root. Accepted so callers can pass it, and deliberately
    /// unused: spec section 6's analytic fallback is conditioned on hero's ACTUAL combo only
    /// (pinned by `facing_allin_golden`).
    pub hero_public: Option<Range1326>,
    /// `C`: chips hero must add to call, already capped by hero's stack (so `C <= facing`).
    pub call_cost: u32,
    /// Chips in the pot before hero acts, including the all-in wager hero faces.
    pub pot: u32,
    /// The wager hero faces; anything above `call_cost` is uncalled and returned.
    pub facing: u32,
    pub rake: Rake,
    pub bb_chips: u32,
}

#[derive(Debug, Clone)]
pub struct AllInAnswer {
    pub equity: f32,
    pub method: EquityMethod,
    /// `W`: the matched pot at the showdown, after the uncalled excess goes back.
    pub w: u32,
    /// `R`: rake taken from `W`, in chips.
    pub r: f32,
    pub ev_call_chips: f32,
    pub actions: Vec<ActionAdvice>,
}

/// Spec section 2's coverage vocabulary for a run that produced no equity: only a range that
/// cannot reach a showdown is `InvalidRanges`; a stop is reported as the stop it was.
fn unsupported(e: NoEquity) -> UnsupportedReason {
    match e {
        NoEquity::InvalidRanges | NoEquity::NoHeroCombo => UnsupportedReason::InvalidRanges,
        NoEquity::BudgetExceeded => UnsupportedReason::DeadlineExceeded { stage: "facing-all-in hero-combo equity".to_string() },
        NoEquity::Cancelled => UnsupportedReason::EngineError { message: "facing-all-in hero-combo equity cancelled".to_string(), retryable: true },
        NoEquity::Malformed(what) => UnsupportedReason::EngineError { message: format!("facing-all-in hero-combo equity: {what}"), retryable: false },
    }
}

/// # Panics
/// Panics (in every build profile) on an input that breaks the identity `C <= facing <= pot`, on a
/// non-positive big blind (`ev_bb` divides by it) or on a matched pot outside the `u32` chip range.
/// These are caller bugs in the chip bookkeeping, not data conditions: a well-formed spot whose
/// ranges simply cannot meet at a showdown returns `Err(UnsupportedReason::InvalidRanges)`.
pub fn facing_allin(inp: &AllInInput, budget: Duration, cancel: &AtomicBool) -> Result<AllInAnswer, UnsupportedReason> {
    let _ = &inp.hero_public; // spec section 6: hero's strategic range never enters; only hero's actual combo does
    assert!(inp.bb_chips > 0, "facing_allin: bb_chips must be positive, ev_bb divides by it");
    assert!(inp.call_cost <= inp.facing, "facing_allin: call cost {} exceeds the {} chips faced", inp.call_cost, inp.facing);
    assert!(inp.facing <= inp.pot, "facing_allin: pot {} must already contain the {} chips faced", inp.pot, inp.facing);
    // W = pot - (facing - C) + C: the uncalled excess goes back, hero's call goes in. Widened to
    // u64 first (plan-1 standing ruling c) and checked before it narrows to the u32 chip count it
    // has to be; the asserts above are what make the subtraction impossible to underflow.
    let w64 = u64::from(inp.pot) + 2 * u64::from(inp.call_cost) - u64::from(inp.facing);
    assert!(w64 <= u64::from(u32::MAX), "facing_allin: matched pot {w64} is outside the u32 chip range");
    let w = w64 as u32;

    let (equity, method) = hero_combo_run(inp.hero, &inp.opp_public, &inp.board, budget, cancel).map_err(unsupported)?;

    // f64 throughout, narrowed exactly once per output. `core_eval::terminal_payoff` carries the
    // same discipline for the same reason (review S9): `w as f32` is already lossy above 2^24, so
    // narrowing first would throw away part of the pot before the rake was even subtracted. It is
    // not reused here because this answer reports `R` itself and subtracts `C` before narrowing.
    let w_chips = f64::from(w);
    let rake = match inp.rake {
        // `no_flop_no_drop` decides whether the caller rakes this pot at all, not how much.
        Rake::PotRake { rate, cap_mchips, .. } => (f64::from(rate) * w_chips).min(f64::from(cap_mchips) / 1000.0),
        Rake::TimeCharge => 0.0,
    };
    let ev = f64::from(equity) * (w_chips - rake) - f64::from(inp.call_cost);
    let ev_call_chips = ev as f32;
    let ev_bb = (ev / f64::from(inp.bb_chips)) as f32;
    assert!(ev_call_chips.is_finite() && ev_bb.is_finite(), "facing_allin: EV must be finite chips (spec section 2), got {ev_call_chips} chips / {ev_bb} bb");

    let call_wins = ev_call_chips >= 0.0; // spec section 6: EV(call) == 0 -> call
    let actions = vec![
        // Fold is 0 bb exactly: the chips already in the pot are sunk (spec section 2).
        ActionAdvice { action: Action::Fold, frequency: Some(if call_wins { 0.0 } else { 1.0 }), ev_bb: Some(0.0), unavailable: None, headline: !call_wins },
        ActionAdvice { action: Action::Call, frequency: Some(if call_wins { 1.0 } else { 0.0 }), ev_bb: Some(ev_bb), unavailable: None, headline: call_wins },
    ];
    Ok(AllInAnswer { equity, method, w, r: rake as f32, ev_call_chips, actions })
}
