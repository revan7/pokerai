use proto::{class_combos, class_of, combo_cards, combo_index, Card, ComboIndex, Range1326, CLASSES, COMBOS};
use crate::RangeError;

const RANKS: &[u8; 13] = b"23456789TJQKA";

fn rank_of(c: char) -> Result<u8, RangeError> {
    RANKS.iter().position(|r| *r as char == c.to_ascii_uppercase()).map(|p| p as u8).ok_or_else(|| RangeError::Syntax(format!("unknown rank {c:?}")))
}

/// 169-class index for ranks `hi >= lo` (2 = 0 .. A = 12).
pub fn class_index(hi: u8, lo: u8, suited: bool) -> u8 {
    let (row_hi, row_lo) = (12 - hi, 12 - lo);
    if hi == lo { row_hi * 13 + row_hi } else if suited { row_hi * 13 + row_lo } else { row_lo * 13 + row_hi }
}

/// "AA", "AKs", "AKo" in the spec's grid order.
pub fn class_name(class: u8) -> String {
    let (i, j) = ((class / 13) as usize, (class % 13) as usize);
    let (ri, rj) = (RANKS[12 - i] as char, RANKS[12 - j] as char);
    if i == j { format!("{ri}{ri}") } else if i < j { format!("{ri}{rj}s") } else { format!("{rj}{ri}o") }
}

#[derive(Clone, Copy, PartialEq)]
enum Suits { Suited, Offsuit, Both }

/// Parses "AK", "AKs", "AKo", "AA" into (hi, lo, suitedness); ranks must be written high first.
fn parse_shape(body: &str) -> Result<(u8, u8, Suits), RangeError> {
    let chars: Vec<char> = body.chars().collect();
    let (a, b, suffix) = match chars.as_slice() {
        [a, b] => (*a, *b, None),
        [a, b, s] => (*a, *b, Some(s.to_ascii_lowercase())),
        _ => return Err(RangeError::Syntax(format!("cannot parse {body:?}"))),
    };
    let (hi, lo) = (rank_of(a)?, rank_of(b)?);
    if hi < lo { return Err(RangeError::Syntax(format!("{body:?}: write the higher rank first"))); }
    let suits = match suffix {
        None => Suits::Both,
        Some('s') if hi != lo => Suits::Suited,
        Some('o') if hi != lo => Suits::Offsuit,
        Some(other) => return Err(RangeError::Syntax(format!("{body:?}: unexpected suffix {other:?}"))),
    };
    Ok((hi, lo, suits))
}

fn classes_of(hi: u8, lo: u8, suits: Suits) -> Vec<u8> {
    if hi == lo { return vec![class_index(hi, lo, false)]; }
    match suits {
        Suits::Suited => vec![class_index(hi, lo, true)],
        Suits::Offsuit => vec![class_index(hi, lo, false)],
        Suits::Both => vec![class_index(hi, lo, true), class_index(hi, lo, false)],
    }
}

/// Expands one group body into combos.
///
/// Token precedence, in this order: `random`; an explicit four-character combo (`AsKh`, detected by
/// suit characters at positions 2 and 4, which is why `AKs+` and `A9s-A6s` never reach that branch);
/// a `+` suffix; a `-` range; a plain shape (`AA`, `AKs`, `AKo`, `AK`).
fn expand_body(body: &str) -> Result<Vec<ComboIndex>, RangeError> {
    if body.eq_ignore_ascii_case("random") { return Ok((0..COMBOS as u16).collect()); }
    let chars: Vec<char> = body.chars().collect();
    if chars.len() == 4 && "cdhs".contains(chars[1].to_ascii_lowercase()) && "cdhs".contains(chars[3].to_ascii_lowercase()) {
        let a: Card = format!("{}{}", chars[0], chars[1]).parse().map_err(|e| RangeError::Syntax(format!("{e}")))?;
        let b: Card = format!("{}{}", chars[2], chars[3]).parse().map_err(|e| RangeError::Syntax(format!("{e}")))?;
        if a == b { return Err(RangeError::Syntax(format!("{body:?}: duplicate card"))); }
        return Ok(vec![combo_index(a, b)]);
    }
    let classes: Vec<u8> = if let Some(base) = body.strip_suffix('+') {
        let (hi, lo, suits) = parse_shape(base)?;
        if hi == lo { (hi..=12).flat_map(|r| classes_of(r, r, suits)).collect() }
        else if hi == lo + 1 { (lo..12).flat_map(|l| classes_of(l + 1, l, suits)).collect() }
        else { (lo..hi).flat_map(|l| classes_of(hi, l, suits)).collect() }
    } else if let Some((first, second)) = body.split_once('-') {
        let (h1, l1, s1) = parse_shape(first)?;
        let (h2, l2, s2) = parse_shape(second)?;
        if s1 != s2 { return Err(RangeError::Syntax(format!("{body:?}: mixed suitedness"))); }
        if h1 == l1 && h2 == l2 {
            if h1 < h2 { return Err(RangeError::Syntax(format!("{body:?}: dash ranges are written high to low"))); }
            (h2..=h1).flat_map(|r| classes_of(r, r, s1)).collect()
        } else if h1 == h2 {
            if l1 < l2 { return Err(RangeError::Syntax(format!("{body:?}: dash ranges are written high to low"))); }
            (l2..=l1).flat_map(|l| classes_of(h1, l, s1)).collect()
        } else if h1 - l1 == h2 - l2 {
            if h1 < h2 { return Err(RangeError::Syntax(format!("{body:?}: dash ranges are written high to low"))); }
            (h2..=h1).flat_map(|h| classes_of(h, h - (h1 - l1), s1)).collect()
        } else {
            return Err(RangeError::Syntax(format!("{body:?}: endpoints must share the high rank or the gap")));
        }
    } else {
        let (hi, lo, suits) = parse_shape(body)?;
        classes_of(hi, lo, suits)
    };
    Ok(classes.into_iter().flat_map(class_combos).collect())
}

/// Parses a weight token as `f64` and validates finiteness and `[0, 1]` on that wider value
/// before narrowing to `f32` (never clamping), so out-of-domain text such as `"1.00000001"`
/// or `"-1e-50"` cannot slip through by rounding into range during narrowing. The narrowed
/// `f32` value is re-checked as a safety net.
fn parse_weight(text: &str) -> Result<f32, RangeError> {
    let text = text.trim();
    let w64: f64 = text.parse().map_err(|_| RangeError::Weight(format!("cannot parse weight {text:?}")))?;
    if !w64.is_finite() || !(0.0..=1.0).contains(&w64) {
        return Err(RangeError::Weight(format!("weight {text} is outside [0, 1]")));
    }
    // `+ 0.0` maps `-0.0` to `+0.0` and is the identity on every other in-domain value. `-0.0`
    // passes the `[0, 1]` check above (`-0.0 == 0.0`) and would otherwise survive into the
    // bit-exact layers, where `hash_scaled` hashes `0x80000000` differently from `0x00000000`: a
    // range and its own `range_to_string` round trip would be numerically identical yet produce
    // different cache keys (review S5). Normalizing here, at the one text ingestion boundary,
    // leaves `hash_scaled`, `apply_range` and `canonicalize` bit-exact as ruled by T20/T21.
    let w = (w64 as f32) + 0.0;
    if !w.is_finite() || !(0.0..=1.0).contains(&w) {
        return Err(RangeError::Weight(format!("weight {text} is outside [0, 1]")));
    }
    Ok(w)
}

/// Pio-style range string to `Range1326`; later groups override earlier ones.
pub fn parse_range(text: &str) -> Result<Range1326, RangeError> {
    let mut r = Range1326::zero();
    for token in text.split(|c: char| c == ',' || c.is_whitespace()).map(str::trim).filter(|t| !t.is_empty()) {
        let (body, weight) = match token.split_once(':') { Some((b, w)) => (b.trim(), parse_weight(w)?), None => (token, 1.0) };
        for combo in expand_body(body)? { r.set(combo, weight); }
    }
    Ok(r)
}

/// Deterministic text form in 169-class order; zero-weight combos are omitted.
pub fn range_to_string(r: &Range1326) -> String {
    let mut tokens: Vec<String> = Vec::new();
    for class in 0..CLASSES as u8 {
        let combos = class_combos(class);
        let first = r.get(combos[0]);
        if combos.iter().all(|c| r.get(*c) == first) {
            if first > 0.0 { tokens.push(if first == 1.0 { class_name(class) } else { format!("{}:{}", class_name(class), first) }); }
        } else {
            for c in combos {
                let w = r.get(c);
                if w > 0.0 {
                    let [lo, hi] = combo_cards(c);
                    tokens.push(if w == 1.0 { format!("{hi}{lo}") } else { format!("{hi}{lo}:{w}") });
                }
            }
        }
    }
    tokens.join(",")
}

/// Every combo of a class receives the class value (pair 6, suited 4, offsuit 12 combos).
///
/// # Panics
///
/// Panics if any of the 169 class weights is not finite or lies outside `[0.0, 1.0]`, naming
/// the offending class index in the message. This is an always-on precondition check (not
/// debug-only, never a silent clamp): `expand_169` stays infallible for valid input, but callers
/// must supply already-validated class weights, since `Range1326::from_fn` performs no
/// validation of its own.
pub fn expand_169(classes: &[f32; CLASSES]) -> Range1326 {
    for (i, w) in classes.iter().enumerate() {
        assert!(w.is_finite() && (0.0..=1.0).contains(w), "expand_169: class {i} weight {w} is not finite in [0, 1]");
    }
    Range1326::from_fn(|i| classes[class_of(i) as usize])
}
