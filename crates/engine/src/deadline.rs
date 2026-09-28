//! Absolute deadlines of spec section 7, in integer milliseconds on the engine's `Clock`.
//!
//! Every deadline is absolute, measured from the request's monotonic admission time `t0_ms`: the first-attempt
//! (street) deadline, the final delivery, and the watchdog's fire time `final delivery - 100 ms`. The worker never
//! sees an absolute time; it receives a relative `deadline_ms` computed at send time from whatever is left, less the
//! delivery and pipe margins (no cross-process timestamps, §7).

use proto::Street;

/// §7: time kept between the worker's deadline and the absolute limit it was computed from, for delivery.
pub const DELIVERY_MARGIN_MS: u64 = 100;
/// §7: time kept for the result line's pipe transfer.
pub const PIPE_MARGIN_MS: u64 = 50;
/// §7: the watchdog emits `Final` this long before the final delivery deadline.
pub const WATCHDOG_LEAD_MS: u64 = 100;

/// §4.2 / §13.3: the per-session flop budget is 1..=30 seconds, default 10. The range is `Engine::set_config`'s own
/// (`engine::FLOP_BUDGET_RANGE`, plan 2 Task 29), so this and the validation it performs cannot disagree.
pub fn flop_budget_valid(flop_budget_s: u8) -> bool {
    crate::engine::FLOP_BUDGET_RANGE.contains(&flop_budget_s)
}

/// §7 street budgets: river 2 s, turn 6 s, flop `flop_budget_s`.
pub fn street_budget_ms(street: Street, flop_budget_s: u8) -> u64 {
    match street {
        Street::River => 2_000,
        Street::Turn => 6_000,
        Street::Flop => u64::from(flop_budget_s) * 1_000,
        Street::Preflop => 0,
    }
}

/// §7 final delivery: 15 s for river and turn, `5 s + flop_budget_s` for flop decisions.
pub fn final_delivery_ms(street: Street, flop_budget_s: u8) -> u64 {
    match street {
        Street::Flop => 5_000 + u64::from(flop_budget_s) * 1_000,
        _ => 15_000,
    }
}

/// §7 worker extraction margin: river/turn 0.2 s, flop 0.6 s.
pub fn extraction_margin_ms(street: Street) -> u32 {
    if street == Street::Flop { 600 } else { 200 }
}

/// One request's absolute deadlines, all on the engine's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deadlines { pub t0_ms: u64, pub street_deadline_ms: u64, pub final_delivery_ms: u64, pub extraction_margin_ms: u32 }

impl Deadlines {
    /// The deadlines of a request admitted at `t0_ms` for a decision on `street`. `flop_budget_s` only matters on the
    /// flop (its validity, `1..=30`, is `set_config`'s to enforce).
    pub fn for_request(t0_ms: u64, street: Street, flop_budget_s: u8) -> Self {
        let at = |offset_ms: u64| {
            t0_ms.checked_add(offset_ms).unwrap_or_else(|| panic!("deadline t0_ms {t0_ms} + {offset_ms} ms overflows u64"))
        };
        Self {
            t0_ms,
            street_deadline_ms: at(street_budget_ms(street, flop_budget_s)),
            final_delivery_ms: at(final_delivery_ms(street, flop_budget_s)),
            extraction_margin_ms: extraction_margin_ms(street),
        }
    }

    /// `deadline_ms = remaining - delivery_margin - pipe_margin` at send time, where `remaining = until_ms - now_ms`;
    /// `None` when no iteration could fit (the result is not above the extraction margin, or `until_ms` has passed).
    ///
    /// `until_ms` is the street deadline (first attempt) or the final delivery (a retry): no phase is ever given time
    /// past the final delivery, and asking for it is a caller bug.
    pub fn worker_deadline_ms(&self, now_ms: u64, until_ms: u64) -> Option<u32> {
        assert!(
            until_ms <= self.final_delivery_ms,
            "worker deadline asked until {until_ms} ms, past the final delivery at {} ms",
            self.final_delivery_ms
        );
        let d = until_ms.saturating_sub(now_ms).saturating_sub(DELIVERY_MARGIN_MS + PIPE_MARGIN_MS);
        if d <= u64::from(self.extraction_margin_ms) {
            return None;
        }
        // Narrowed only after the check in the wide type: a relative deadline beyond u32 milliseconds (49 days) cannot
        // come from a §7 budget, so it is a broken `Deadlines`, never a value to clamp.
        Some(u32::try_from(d).unwrap_or_else(|_| panic!("worker deadline {d} ms does not fit the u32 wire field")))
    }

    /// §7: the watchdog fires at `final delivery - 100 ms`.
    pub fn watchdog_fire_ms(&self) -> u64 {
        let fire = self.final_delivery_ms.checked_sub(WATCHDOG_LEAD_MS);
        match fire {
            Some(fire) if fire >= self.t0_ms => fire,
            _ => panic!(
                "watchdog fire time (final delivery {} ms - {WATCHDOG_LEAD_MS} ms) falls before t0 {} ms",
                self.final_delivery_ms, self.t0_ms
            ),
        }
    }
}

/// §7: a retry is admitted only if `remaining >= p95(_min template) + margins`, the margins being the delivery,
/// pipe and extraction margins. Until a measured bench matrix exists (plan 4 Task 21) the caller passes the street
/// budget as the p95 proxy. Pure: the same inputs always give the same answer. A requirement that overflows `u64`
/// can never fit.
pub fn retry_admitted(now_ms: u64, final_delivery_ms: u64, p95_ms: u64, extraction_margin_ms: u32) -> bool {
    let remaining = final_delivery_ms.saturating_sub(now_ms);
    let required = p95_ms
        .checked_add(DELIVERY_MARGIN_MS + PIPE_MARGIN_MS)
        .and_then(|r| r.checked_add(u64::from(extraction_margin_ms)));
    required.is_some_and(|required| remaining >= required)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_margins_and_retry_admission_of_section_7() {
        let river = Deadlines::for_request(0, Street::River, 10);
        assert_eq!((river.street_deadline_ms, river.final_delivery_ms, river.extraction_margin_ms), (2_000, 15_000, 200));
        assert_eq!(river.watchdog_fire_ms(), 14_900);
        let turn = Deadlines::for_request(1_000, Street::Turn, 10);
        assert_eq!((turn.street_deadline_ms, turn.final_delivery_ms), (7_000, 16_000));
        // the flop budget stretches both the first-attempt deadline and the final delivery, and only on the flop
        let flop = Deadlines::for_request(0, Street::Flop, 30);
        assert_eq!((flop.street_deadline_ms, flop.final_delivery_ms, flop.extraction_margin_ms), (30_000, 35_000, 600));
        assert_eq!(Deadlines::for_request(0, Street::Flop, 10).final_delivery_ms, 15_000);
        // deadline_ms = remaining - 100 - 50 at send time; None when not even the extraction margin fits
        assert_eq!(river.worker_deadline_ms(500, 2_000), Some(1_350));
        assert_eq!(river.worker_deadline_ms(0, 15_000), Some(14_850));
        assert_eq!(river.worker_deadline_ms(1_800, 2_000), None);        // 50 <= 200
        assert_eq!(river.worker_deadline_ms(3_000, 2_000), None);        // already past
        // a retry is admitted only when the p95 of the retry template plus every margin still fits
        assert!(retry_admitted(6_200, 15_000, 6_000, 200));               // 8_800 >= 6_350
        assert!(!retry_admitted(9_000, 15_000, 6_000, 200));              // 6_000 <  6_350
    }

    /// The boundaries of admission are exact: `remaining == p95 + margins` is admitted, one millisecond less is not,
    /// and a requirement too large for `u64` is refused rather than wrapped into a small one.
    #[test]
    fn retry_admission_boundaries_and_overflow() {
        assert!(retry_admitted(8_650, 15_000, 6_000, 200));               // 6_350 >= 6_350
        assert!(!retry_admitted(8_651, 15_000, 6_000, 200));              // 6_349 <  6_350
        assert!(!retry_admitted(20_000, 15_000, 0, 200));                 // past the final delivery
        assert!(!retry_admitted(0, u64::MAX, u64::MAX, 200));             // u64::MAX + 350 does not wrap to 349
        assert!(!retry_admitted(0, u64::MAX, u64::MAX - 150, u32::MAX));  // nor does the extraction margin's addition
    }

    /// The worker deadline is `None` exactly up to the extraction margin: 1 ms above it is the first that fits.
    #[test]
    fn worker_deadline_boundary_at_the_extraction_margin() {
        let river = Deadlines::for_request(0, Street::River, 10);
        assert_eq!(river.worker_deadline_ms(1_650, 2_000), None);         // 200 <= 200
        assert_eq!(river.worker_deadline_ms(1_649, 2_000), Some(201));
        let flop = Deadlines::for_request(0, Street::Flop, 10);
        assert_eq!(flop.worker_deadline_ms(9_250, 10_000), None);         // 600 <= 600
        assert_eq!(flop.worker_deadline_ms(250, 10_000), Some(9_600));
    }

    #[test]
    #[should_panic(expected = "past the final delivery")]
    fn worker_deadline_past_the_final_delivery_is_a_caller_bug() {
        Deadlines::for_request(0, Street::River, 10).worker_deadline_ms(0, 15_001);
    }

    #[test]
    #[should_panic(expected = "does not fit the u32 wire field")]
    fn worker_deadline_beyond_u32_is_refused_not_truncated() {
        let d = Deadlines { t0_ms: 0, street_deadline_ms: 0, final_delivery_ms: u64::MAX, extraction_margin_ms: 200 };
        d.worker_deadline_ms(0, u64::MAX);
    }

    #[test]
    #[should_panic(expected = "falls before t0")]
    fn watchdog_fire_before_t0_is_refused() {
        Deadlines { t0_ms: 1_000, street_deadline_ms: 1_000, final_delivery_ms: 1_050, extraction_margin_ms: 200 }.watchdog_fire_ms();
    }

    #[test]
    #[should_panic(expected = "falls before t0")]
    fn watchdog_fire_below_zero_is_refused() {
        Deadlines { t0_ms: 0, street_deadline_ms: 0, final_delivery_ms: 50, extraction_margin_ms: 200 }.watchdog_fire_ms();
    }

    #[test]
    #[should_panic(expected = "overflows u64")]
    fn deadlines_that_overflow_the_clock_are_refused() {
        Deadlines::for_request(u64::MAX - 1_000, Street::River, 10);
    }

    /// Preflop never reaches the solver: no street budget, the 15 s final delivery; the flop budget moves nothing
    /// off the flop.
    #[test]
    fn preflop_and_off_flop_budgets() {
        assert_eq!(street_budget_ms(Street::Preflop, 30), 0);
        assert_eq!(final_delivery_ms(Street::Preflop, 30), 15_000);
        assert_eq!((street_budget_ms(Street::River, 30), street_budget_ms(Street::Turn, 30)), (2_000, 6_000));
        assert_eq!((final_delivery_ms(Street::River, 30), final_delivery_ms(Street::Turn, 30)), (15_000, 15_000));
        assert_eq!((extraction_margin_ms(Street::River), extraction_margin_ms(Street::Turn), extraction_margin_ms(Street::Flop)), (200, 200, 600));
    }
}
