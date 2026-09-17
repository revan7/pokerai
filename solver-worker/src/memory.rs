use proto::worker::WorkerError;

pub const GIB: u64 = 1 << 30;

#[derive(Debug, Clone, Copy)]
pub struct Admission { pub compressed: bool, pub estimate_bytes: u64, pub mode: &'static str }

/// §10.3: f32 when the f32 estimate <= 2 GiB, else i16 when the i16 estimate <= 8 GiB, else tree_too_large;
/// then refuse when `estimate * 1.25 > memory_limit_bytes`.
pub fn admit(f32_bytes: u64, i16_bytes: u64, memory_limit_bytes: u64) -> Result<Admission, WorkerError> {
    let too_large = |estimate: u64, msg: &str| WorkerError { code: "tree_too_large".into(), message: msg.into(), retryable: false, estimate_bytes: Some(estimate) };
    let a = if f32_bytes <= 2 * GIB { Admission { compressed: false, estimate_bytes: f32_bytes, mode: "f32" } }
        else if i16_bytes <= 8 * GIB { Admission { compressed: true, estimate_bytes: i16_bytes, mode: "i16" } }
        else { return Err(too_large(i16_bytes, "i16 estimate above 8 GiB")); };
    // R3 (fix round 1): exact `estimate * 1.25` via integer ceiling, not floor division -- a plain
    // `estimate / 4` silently drops the fractional quarter-byte and can admit an estimate whose
    // true headroom requirement is a few bytes above the limit. `div_ceil` stays safely inside
    // u64: `a.estimate_bytes` is already capped at 8 GiB above, so the ceiling quarter is at most
    // 2 GiB and the sum is at most 10 GiB, far below u64::MAX. Never clamped, saturated, narrowed,
    // or used to switch storage mode -- a rejection here is final.
    if a.estimate_bytes + a.estimate_bytes.div_ceil(4) > memory_limit_bytes {
        return Err(too_large(a.estimate_bytes, "estimate * 1.25 above memory_limit_bytes"));
    }
    Ok(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn admission_rule_of_section_10_3() {
        assert_eq!(admit(GIB, GIB / 2, 10 * GIB).unwrap().mode, "f32");
        assert_eq!(admit(2 * GIB, GIB, 10 * GIB).unwrap().mode, "f32");           // inclusive
        let i16 = admit(3 * GIB, GIB + GIB / 2, 10 * GIB).unwrap();
        assert!(i16.compressed && i16.estimate_bytes == GIB + GIB / 2);
        let e = admit(20 * GIB, 10 * GIB, 10 * GIB).unwrap_err();
        assert_eq!((e.code.as_str(), e.retryable, e.estimate_bytes), ("tree_too_large", false, Some(10 * GIB)));
        let e = admit(GIB, GIB / 2, GIB).unwrap_err();                              // headroom: 1.25 GiB > 1 GiB
        assert_eq!((e.code.as_str(), e.estimate_bytes), ("tree_too_large", Some(GIB)));
    }

    // R3 (fix round 1): floor-based `estimate / 4` silently drops the fractional quarter-byte, so
    // an estimate not divisible by 4 was wrongly admitted right at the boundary. Each of the three
    // nonzero residues gets a rejection at the (wrong) floor-sum boundary and an acceptance at the
    // true (ceiling) boundary, one step higher.
    #[test]
    fn memory_headroom_uses_exact_ceiling_not_floor() {
        // residue 1: estimate = GIB + 1 (f32 mode, <= 2 GiB).
        let estimate = GIB + 1;
        let floor_sum = estimate + estimate / 4; // = 1_342_177_281; old code admitted here.
        let ceil_sum = estimate + estimate.div_ceil(4); // = 1_342_177_282; the exact 1.25x bound.
        assert_eq!((estimate, floor_sum, ceil_sum), (1_073_741_825, 1_342_177_281, 1_342_177_282));
        let e = admit(estimate, estimate, floor_sum).unwrap_err(); // exact*1.25 = 1_342_177_281.25 > limit
        assert_eq!((e.code.as_str(), e.estimate_bytes), ("tree_too_large", Some(estimate)));
        admit(estimate, estimate, ceil_sum).unwrap(); // limit == exact ceiling: admitted, not strictly over

        // residue 2: estimate = GIB + 2.
        let estimate = GIB + 2;
        let floor_sum = estimate + estimate / 4;
        let ceil_sum = estimate + estimate.div_ceil(4);
        assert_eq!((floor_sum, ceil_sum), (1_342_177_282, 1_342_177_283));
        admit(estimate, estimate, floor_sum).unwrap_err();
        admit(estimate, estimate, ceil_sum).unwrap();

        // residue 3: estimate = GIB + 3.
        let estimate = GIB + 3;
        let floor_sum = estimate + estimate / 4;
        let ceil_sum = estimate + estimate.div_ceil(4);
        assert_eq!((floor_sum, ceil_sum), (1_342_177_283, 1_342_177_284));
        admit(estimate, estimate, floor_sum).unwrap_err();
        admit(estimate, estimate, ceil_sum).unwrap();

        // residue 0 (divisible by 4): floor and ceiling agree, exact equality is still admitted.
        admit(GIB, GIB / 2, GIB + GIB / 4).unwrap();
    }

    // R3: the i16 8 GiB cap is itself inclusive, and its exact 1.25x headroom (10 GiB, the
    // engine's own default `memory_limit_bytes`, spec §10.3) must admit, not just approximately fit.
    #[test]
    fn memory_admits_i16_at_exact_8gib_boundary() {
        let a = admit(20 * GIB, 8 * GIB, 10 * GIB).unwrap();
        assert!(a.compressed && a.mode == "i16" && a.estimate_bytes == 8 * GIB);
    }

    // R3: wide-input safety -- an absurd estimate must be rejected (not overflow/panic), and the
    // headroom arithmetic on an admitted, capped estimate must never overflow even against a
    // maximal memory_limit_bytes.
    #[test]
    fn memory_admit_handles_u64_max_without_overflow() {
        let e = admit(u64::MAX, u64::MAX, u64::MAX).unwrap_err();
        assert_eq!((e.code.as_str(), e.retryable, e.estimate_bytes), ("tree_too_large", false, Some(u64::MAX)));
        let a = admit(GIB, GIB / 2, u64::MAX).unwrap(); // tiny estimate against a maximal limit: no overflow
        assert_eq!(a.mode, "f32");
    }
}
