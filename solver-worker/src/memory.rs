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
    if a.estimate_bytes + a.estimate_bytes / 4 > memory_limit_bytes { return Err(too_large(a.estimate_bytes, "estimate * 1.25 above memory_limit_bytes")); }
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
}
