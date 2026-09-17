use proto::HandConfig;

/// Normalized posts `(sb/S, bb/S, 1)` reported under `StraddleMapped` (spec 8.3).
pub fn straddle_posts(cfg: &HandConfig) -> Option<[f32; 3]> {
    cfg.straddle.map(|s| { let unit = s.amount_chips as f32; [cfg.sb_chips as f32 / unit, cfg.bb_chips as f32 / unit, 1.0] })
}

/// The first full raise on the preflop street is one big blind, or one straddle when posted.
pub fn initial_full_raise(cfg: &HandConfig) -> u32 { cfg.straddle.map(|s| s.amount_chips).unwrap_or(cfg.bb_chips) }

/// Forced posts as (ring index, chips): SB, BB and the straddle.
pub fn posts(cfg: &HandConfig) -> Vec<(usize, u32)> {
    let mut v = vec![(0, cfg.sb_chips), (1, cfg.bb_chips)];
    if let Some(s) = cfg.straddle { v.push((2, s.amount_chips)); }
    v
}
