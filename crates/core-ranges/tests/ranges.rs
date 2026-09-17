use core_ranges::*;
use proto::*;

fn weight_of(r: &Range1326, text: &str) -> f32 {
    let cards = text.as_bytes();
    let a: Card = std::str::from_utf8(&cards[0..2]).unwrap().parse().unwrap();
    let b: Card = std::str::from_utf8(&cards[2..4]).unwrap().parse().unwrap();
    r.get(combo_index(a, b))
}

#[test]
fn range_roundtrip_pio_strings() {
    let r = parse_range("AKs:0.5, 77+, A5o").unwrap();
    assert_eq!(weight_of(&r, "AsKs"), 0.5);
    assert_eq!(weight_of(&r, "AsKh"), 0.0);
    assert_eq!(weight_of(&r, "7c7d"), 1.0);
    assert_eq!(weight_of(&r, "AcAd"), 1.0);
    assert_eq!(weight_of(&r, "6c6d"), 0.0);
    assert_eq!(weight_of(&r, "Ah5c"), 1.0);
    assert_eq!(weight_of(&r, "Ah5h"), 0.0);
    assert_eq!(mass(&r), 4.0 * 0.5 + 8.0 * 6.0 + 12.0);
    let text = range_to_string(&r);
    assert_eq!(text, "AA,AKs:0.5,KK,QQ,JJ,TT,99,88,77,A5o", "169-class row-major order: row A holds AA then AKs; A5o is row 5, column A");
    assert_eq!(parse_range(&text).unwrap(), r);
    assert_eq!(mass(&parse_range("QQ-88").unwrap()), 30.0);
    assert_eq!(mass(&parse_range("A9s-A6s").unwrap()), 16.0);
    assert_eq!(mass(&parse_range("98o-65o").unwrap()), 48.0);
    assert_eq!(mass(&parse_range("T9s+").unwrap()), 20.0, "T9s, JTs, QJs, KQs, AKs");
    assert_eq!(mass(&parse_range("ATs+").unwrap()), 16.0, "ATs, AJs, AQs, AKs");
    assert_eq!(mass(&parse_range("AK").unwrap()), 16.0);
    assert_eq!(mass(&parse_range("AsKh:0.25").unwrap()), 0.25);
    assert_eq!(mass(&parse_range("22+").unwrap()), 78.0);
    assert_eq!(mass(&parse_range("AA, AA:0.5").unwrap()), 3.0, "later groups override");
    assert!(matches!(parse_range("88-QQ"), Err(RangeError::Syntax(_))), "dash ranges are written high to low");
    assert!(parse_range("KA").is_err());
    assert!(parse_range("AKx").is_err());
    assert!(parse_range("AA:1.5").is_err());
    assert!(parse_range("AA:-0.1").is_err());
    assert!(parse_range("AA:nan").is_err());
    assert!(parse_range("A9s-K6s").is_err());
    // R1 regression: text weights must be validated before narrowing to f32, not after
    assert!(matches!(parse_range("AA:1.00000001"), Err(RangeError::Weight(_))), "rounds to 1.0f32 but is out of domain as written");
    assert!(matches!(parse_range("AA:-1e-50"), Err(RangeError::Weight(_))), "rounds to -0.0f32 but is negative as written");
    assert_eq!(mass(&parse_range("AA:0").unwrap()), 0.0, "success control at 0");
    assert_eq!(mass(&parse_range("AA:0.5").unwrap()), 3.0, "success control at 0.5");
    assert_eq!(mass(&parse_range("AA:1").unwrap()), 6.0, "success control at 1");
    // a class with mixed weights prints per combo and round-trips
    let mut mixed = parse_range("AKs").unwrap();
    mixed.set(combo_index("As".parse().unwrap(), "Ks".parse().unwrap()), 0.37);
    let text = range_to_string(&mixed);
    assert!(text.contains("AsKs:0.37") || text.contains("KsAs:0.37"), "{text}");
    assert_eq!(parse_range(&text).unwrap(), mixed);
    assert_eq!(range_to_string(&Range1326::zero()), "");
}

#[test]
fn class_expansion_multiplicity() {
    let mut classes = [0f32; 169];
    classes[class_index(12, 12, false) as usize] = 1.0; // AA
    classes[class_index(12, 11, true) as usize] = 0.5;  // AKs
    classes[class_index(12, 11, false) as usize] = 0.25; // AKo
    let r = expand_169(&classes);
    assert_eq!(mass(&r), 6.0 + 4.0 * 0.5 + 12.0 * 0.25);
    assert_eq!(r.0.iter().filter(|w| **w == 1.0).count(), 6);
    assert_eq!(r.0.iter().filter(|w| **w == 0.5).count(), 4);
    assert_eq!(r.0.iter().filter(|w| **w == 0.25).count(), 12);
    assert_eq!(class_name(0), "AA");
    assert_eq!(class_name(1), "AKs");
    assert_eq!(class_name(13), "AKo");
    assert_eq!(class_name(168), "22");
    assert_eq!(class_name(class_index(8, 7, false)), "T9o");
    assert_eq!(mass(&parse_range("random").unwrap()), 1326.0);
    assert_eq!(mass(&expand_169(&[1.0; 169])), 1326.0);
}

// R2 regression: expand_169 must assert its precondition (finite, [0,1] weights) rather than
// silently propagating invalid values into the constructed Range1326.

#[test]
#[should_panic(expected = "class 5")]
fn expand_169_rejects_nan() {
    let mut classes = [0f32; 169];
    classes[5] = f32::NAN;
    expand_169(&classes);
}

#[test]
#[should_panic(expected = "class 5")]
fn expand_169_rejects_positive_infinity() {
    let mut classes = [0f32; 169];
    classes[5] = f32::INFINITY;
    expand_169(&classes);
}

#[test]
#[should_panic(expected = "class 5")]
fn expand_169_rejects_negative_infinity() {
    let mut classes = [0f32; 169];
    classes[5] = f32::NEG_INFINITY;
    expand_169(&classes);
}

#[test]
#[should_panic(expected = "class 5")]
fn expand_169_rejects_negative_weight() {
    let mut classes = [0f32; 169];
    classes[5] = -0.1;
    expand_169(&classes);
}

#[test]
#[should_panic(expected = "class 5")]
fn expand_169_rejects_weight_above_one() {
    let mut classes = [0f32; 169];
    classes[5] = 1.5;
    expand_169(&classes);
}
