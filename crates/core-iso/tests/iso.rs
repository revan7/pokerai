use core_iso::*;
use proto::*;
use std::collections::HashMap;

fn cards(text: &str) -> Vec<Card> { text.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect() }

/// Every raw flop grouped by canonical class, with the raw count per class.
fn classes() -> HashMap<CanonicalBoard, usize> {
    let mut map: HashMap<CanonicalBoard, usize> = HashMap::new();
    for a in 0..52u8 { for b in (a + 1)..52 { for c in (b + 1)..52 {
        let (cb, _) = canonicalize(&[Card(a), Card(b), Card(c)], &[]);
        *map.entry(cb).or_default() += 1;
    } } }
    map
}

#[test]
fn iso_class_count_1755() {
    assert_eq!(classes().len(), 1755);
    let (a, _) = canonicalize(&cards("AsKd2c"), &[]);
    let (b, _) = canonicalize(&cards("2cAsKd"), &[]);
    assert_eq!(a, b, "the flop is an unordered set");
}

#[test]
fn iso_orbit_sizes() {
    let map = classes();
    let (mut trips, mut paired, mut distinct, mut total) = (0usize, 0usize, 0usize, 0usize);
    for (cb, raw) in &map {
        let orbit = orbit_size(cb) as usize;
        assert!(matches!(orbit, 4 | 12 | 24), "{cb:?}: orbit {orbit}");
        assert_eq!(*raw, orbit, "{cb:?}: raw flops in the class equal the orbit size");
        total += orbit;
        let ranks: Vec<u8> = cb.cards().iter().map(|c| c.rank()).collect();
        let distinct_ranks = { let mut r = ranks.clone(); r.sort(); r.dedup(); r.len() };
        match distinct_ranks { 1 => trips += orbit, 2 => paired += orbit, _ => distinct += orbit }
    }
    assert_eq!((trips, paired, distinct, total), (52, 3744, 18304, 22100));
    assert_eq!(orbit_size_of(&cards("AsKsQs")), 4, "monotone");
    assert_eq!(orbit_size_of(&cards("AsKs2c")), 12, "two-tone");
    assert_eq!(orbit_size_of(&cards("AsKd2c")), 24, "rainbow");
    assert_eq!(orbit_size_of(&cards("AsAh2c")), 12, "paired");
    assert_eq!(orbit_size_of(&cards("AsAhAd")), 4, "trips");
}

#[test]
fn iso_stabilizer_tiebreak() {
    let r1 = Range1326::from_fn(|i| ((i as u32 * 7919) % 101) as f32 / 100.0);
    let r2 = Range1326::from_fn(|i| ((i as u32 * 104729 + 7) % 89) as f32 / 88.0);
    for board in [cards("AsAh2c"), cards("AsKsQs"), cards("AsAh2c7d"), cards("KsKhKd")] {
        let (cb, p) = canonicalize(&board, &[&r1, &r2]);
        let key1 = apply_range(&p, &r1);
        let key2 = apply_range(&p, &r2);
        for tau in ALL_PERMS {
            let tau = SuitPerm(tau);
            let permuted: Vec<Card> = board.iter().map(|c| apply(&tau, *c)).collect();
            let (cb2, p2) = canonicalize(&permuted, &[&apply_range(&tau, &r1), &apply_range(&tau, &r2)]);
            assert_eq!(cb2, cb, "{board:?} under {tau:?}");
            assert_eq!(apply_range(&p2, &apply_range(&tau, &r1)), key1, "{board:?} under {tau:?}: oop range key");
            assert_eq!(apply_range(&p2, &apply_range(&tau, &r2)), key2, "{board:?} under {tau:?}: ip range key");
        }
    }
    let (a, _) = canonicalize(&cards("AhKd2c7s7h"), &[]);
    let (b, _) = canonicalize(&cards("AhKd2c7h7s"), &[]);
    assert_ne!(a, b, "turn and river keep their dealt order");
    // AhKd2c is rainbow: its stabilizer is trivial, so the two turns are genuinely different
    // boards (verified keys [0,45,50,23] and [0,45,50,22]).
    let (a, _) = canonicalize(&cards("AhKd2c7s"), &[]);
    let (b, _) = canonicalize(&cards("AhKd2c7h"), &[]);
    assert_ne!(a, b, "a rainbow flop fixes every suit, so the turn suit survives canonicalization");
    // AsAh2c has the non-trivial stabilizer (s <-> h), so its two turns share one class
    // (verified: both canonicalize to the key [0,49,50,21]).
    let (a, _) = canonicalize(&cards("AsAh2c7s"), &[]);
    let (b, _) = canonicalize(&cards("AsAh2c7h"), &[]);
    assert_eq!(a, b, "the turn is canonicalized within the flop's stabilizer");
    // core-ranges is core-iso's declared dependency (spec 3.2): exercise it on a real Pio range.
    let r3 = core_ranges::parse_range("AA,KK,AKs:0.5,54o").unwrap();
    for p in ALL_PERMS {
        let p = SuitPerm(p);
        assert_eq!(apply_range(&inverse(&p), &apply_range(&p, &r1)), r1);
        assert_eq!(apply_range(&inverse(&p), &apply_range(&p, &r3)), r3);
        for c in Card::all() { assert_eq!(apply(&inverse(&p), apply(&p, c)), c); }
    }
    let (_, p) = canonicalize(&cards("AsKd2c"), &[]);
    let (_, q) = canonicalize(&cards("AsKd2c"), &[&r1, &r2]);
    assert_eq!(p, q, "a rainbow flop has a trivial stabilizer: the ranges cannot change the permutation");
    assert_eq!(canonicalize(&cards("AsAh2c"), &[]).1, canonicalize(&cards("AsAh2c"), &[]).1, "deterministic");
}
