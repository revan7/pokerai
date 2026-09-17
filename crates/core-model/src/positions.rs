use proto::{HandConfig, Position, Seat};
use crate::error::RulesError;

/// Dealt seats clockwise from the seat after the button: SB first, BTN last.
pub fn ring(button: Seat, dealt: &[Seat]) -> Vec<Seat> {
    (1..=6u8).map(|k| Seat((button.0 + k) % 6)).filter(|s| dealt.contains(s)).collect()
}

/// Spec 4.3 dealt-seat rules; returns the ring on success.
pub fn validate_table(cfg: &HandConfig, button: Seat, dealt: &[Seat]) -> Result<Vec<Seat>, RulesError> {
    let invalid = |reason: &str| RulesError::InvalidConfig { reason: reason.to_string() };
    if dealt.len() == 2 { return Err(RulesError::FormatUnsupported { detail: "two dealt seats".into() }); }
    if !(3..=6).contains(&dealt.len()) { return Err(invalid(&format!("{} dealt seats; 3 to 6 are supported", dealt.len()))); }
    if dealt.iter().any(|s| s.0 >= 6) { return Err(invalid("seat ids must be 0..5")); }
    let mut sorted = dealt.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != dealt.len() { return Err(invalid("duplicate dealt seat")); }
    if !dealt.contains(&button) { return Err(invalid("the button is not a dealt seat")); }
    if cfg.sb_chips == 0 || cfg.bb_chips < cfg.sb_chips { return Err(invalid("blinds must satisfy 0 < sb <= bb")); }
    if let Some(s) = cfg.straddle {
        if dealt.len() != 6 { return Err(RulesError::FormatUnsupported { detail: "straddle requires six dealt seats".into() }); }
        if u64::from(s.amount_chips) < 2 * u64::from(cfg.bb_chips) { return Err(RulesError::FormatUnsupported { detail: "short straddle post".into() }); }
    }
    Ok(ring(button, dealt))
}

/// BTN, SB, BB, then the last `n - 3` names of UTG, HJ, CO clockwise (spec 4.3).
pub fn positions(button: Seat, dealt: &[Seat]) -> Vec<(Seat, Position)> {
    let r = ring(button, dealt);
    let n = r.len();
    debug_assert!((3..=6).contains(&n));
    let names = &[Position::Utg, Position::Hj, Position::Co][(6 - n)..];
    let mut out = vec![(r[0], Position::Sb), (r[1], Position::Bb)];
    for (k, seat) in r[2..n - 1].iter().enumerate() { out.push((*seat, names[k])); }
    out.push((r[n - 1], Position::Btn));
    out
}

pub fn position_of(button: Seat, dealt: &[Seat], seat: Seat) -> Option<Position> {
    positions(button, dealt).into_iter().find(|(s, _)| *s == seat).map(|(_, p)| p)
}

/// Preflop: the non-button, non-blind seats clockwise after BB, then BTN, SB, BB; with the straddle HJ, CO, BTN, SB, BB, UTG (spec 2, 4.3).
pub fn preflop_order(button: Seat, dealt: &[Seat], straddle: bool) -> Vec<Seat> {
    let r = ring(button, dealt);
    let n = r.len();
    let mut out = Vec::with_capacity(n);
    if straddle && n == 6 {
        out.extend_from_slice(&r[3..5]);
        out.push(r[5]); out.push(r[0]); out.push(r[1]); out.push(r[2]);
    } else {
        out.extend_from_slice(&r[2..n - 1]);
        out.push(r[n - 1]); out.push(r[0]); out.push(r[1]);
    }
    out
}

/// Postflop: SB, BB, then the others clockwise, BTN last.
pub fn postflop_order(button: Seat, dealt: &[Seat]) -> Vec<Seat> { ring(button, dealt) }
