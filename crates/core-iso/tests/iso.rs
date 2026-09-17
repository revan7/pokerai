use core_iso::*;
use proto::*;
use serde::de::value::{Error as DeError, MapDeserializer, SeqDeserializer};
use serde::Deserialize;
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

/// `to_bits()` of every weight, so `-0.0` and `+0.0` compare unequal (spec section 2's tie-break
/// compares serialized `f32` bit patterns, and `Range1326`'s own `PartialEq` does not).
fn bits(r: &Range1326) -> Vec<u32> { r.0.iter().map(|w| w.to_bits()).collect() }

// R1 (fix round 1): apply_range must move every weight bit-for-bit, including -0.0.

#[test]
fn iso_apply_range_is_bit_exact() {
    let mut r = Range1326::zero();
    r.set(19, -0.0);
    r.set(1034, 1.0);
    assert_eq!(
        bits(&apply_range(&SuitPerm::IDENTITY, &r)),
        bits(&r),
        "identity must preserve bits exactly, including negative zero"
    );
    for p in ALL_PERMS {
        let p = SuitPerm(p);
        let round = apply_range(&inverse(&p), &apply_range(&p, &r));
        assert_eq!(bits(&round), bits(&r), "{p:?}: inverse round-trip must be bit-exact");
    }
}

#[test]
fn iso_tiebreak_negative_zero_regression() {
    // Reproduces the review finding exactly: board AsAh2c, range zero except r[19] = -0.0 and
    // r[1034] = 1.0. Permutation [0,3,1,2] and [0,3,2,1] both produce the same canonical board;
    // [0,3,2,1] is the bit-minimal serialization because it carries -0.0 (0x80000000) at its
    // slot rather than silently normalizing it to +0.0 (0x00000000).
    let mut r = Range1326::zero();
    r.set(19, -0.0);
    r.set(1034, 1.0);
    let (_, p) = canonicalize(&cards("AsAh2c"), &[&r]);
    assert_eq!(
        p,
        SuitPerm([0, 3, 2, 1]),
        "the bit-minimal permutation must win the tie-break, not the one that happens to zero out -0.0"
    );
}

// R2 (fix round 1): SuitPerm must validate the bijection invariant at deserialization and at
// every public operation boundary that consumes a caller-supplied permutation.

#[test]
fn iso_suit_perm_deserialize_round_trips_all_valid_permutations() {
    for p in ALL_PERMS {
        let de = SeqDeserializer::<_, DeError>::new(p.into_iter());
        let back: SuitPerm = SuitPerm::deserialize(de).unwrap();
        assert_eq!(back, SuitPerm(p));
    }
}

#[test]
fn iso_suit_perm_deserialize_rejects_duplicate_image() {
    let de = SeqDeserializer::<_, DeError>::new([0u8, 0, 2, 3].into_iter());
    assert!(SuitPerm::deserialize(de).is_err(), "duplicate image (not a bijection) must be rejected");
}

#[test]
fn iso_suit_perm_deserialize_rejects_out_of_range_image() {
    let de = SeqDeserializer::<_, DeError>::new([0u8, 1, 2, 4].into_iter());
    assert!(SuitPerm::deserialize(de).is_err(), "image 4 is outside 0..4 and must be rejected");
}

#[test]
#[should_panic(expected = "repeats an earlier suit's image")]
fn iso_inverse_panics_on_duplicate_image_perm() {
    inverse(&SuitPerm([0, 0, 2, 3]));
}

#[test]
#[should_panic(expected = "outside 0..4")]
fn iso_inverse_panics_on_out_of_range_image_perm() {
    inverse(&SuitPerm([0, 1, 2, 4]));
}

#[test]
#[should_panic(expected = "repeats an earlier suit's image")]
fn iso_apply_range_panics_on_duplicate_image_perm() {
    apply_range(&SuitPerm([0, 0, 2, 3]), &Range1326::zero());
}

// R3 (fix round 1): CanonicalBoard must validate length, card domain, uniqueness, and
// canonical-ness at deserialization; canonicalize/orbit_size_of must validate their board input.

#[test]
#[should_panic(expected = "duplicate card")]
fn iso_canonicalize_rejects_duplicate_cards() {
    canonicalize(&[Card(0), Card(0), Card(1)], &[]);
}

#[test]
#[should_panic(expected = "outside 0..52")]
fn iso_canonicalize_rejects_out_of_range_card_id() {
    canonicalize(&[Card(0), Card(1), Card(99)], &[]);
}

#[test]
#[should_panic(expected = "duplicate card")]
fn iso_orbit_size_of_rejects_duplicate_cards() {
    orbit_size_of(&[Card(0), Card(0), Card(1)]);
}

#[test]
fn iso_canonical_board_deserialize_rejects_empty_payload() {
    let pairs: Vec<(&str, Vec<&str>)> = vec![("cards", vec![])];
    let de = MapDeserializer::<_, DeError>::new(pairs.into_iter());
    assert!(CanonicalBoard::deserialize(de).is_err(), "0 cards is outside 3..=5");
}

#[test]
fn iso_canonical_board_deserialize_rejects_duplicate_cards() {
    let pairs: Vec<(&str, Vec<&str>)> = vec![("cards", vec!["As", "As", "2c"])];
    let de = MapDeserializer::<_, DeError>::new(pairs.into_iter());
    assert!(CanonicalBoard::deserialize(de).is_err(), "a repeated card must be rejected");
}

#[test]
fn iso_canonical_board_deserialize_rejects_noncanonical_payload() {
    // AsKd2c raw (dealt order 51, 45, 0) is a valid, duplicate-free 3-card board, but it is not
    // itself the canonical representative (the flop must be sorted ascending, and this rainbow
    // flop's minimal permutation relabels suits too).
    let pairs: Vec<(&str, Vec<&str>)> = vec![("cards", vec!["As", "Kd", "2c"])];
    let de = MapDeserializer::<_, DeError>::new(pairs.into_iter());
    assert!(
        CanonicalBoard::deserialize(de).is_err(),
        "a valid but non-canonical card sequence must be rejected, not silently re-canonicalized"
    );
}

#[test]
fn iso_canonical_board_deserialize_round_trips_canonical_payload() {
    let (canonical, _) = canonicalize(&cards("AsKd2c"), &[]);
    let codes: Vec<String> = canonical.cards().iter().map(|c| c.to_string()).collect();
    let code_refs: Vec<&str> = codes.iter().map(String::as_str).collect();
    let pairs: Vec<(&str, Vec<&str>)> = vec![("cards", code_refs)];
    let de = MapDeserializer::<_, DeError>::new(pairs.into_iter());
    let back: CanonicalBoard = CanonicalBoard::deserialize(de).unwrap();
    assert_eq!(back, canonical, "an already-canonical payload must round-trip");
}
