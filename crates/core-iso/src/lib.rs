use proto::{combo_cards, combo_index, Card, Range1326, COMBOS};
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

/// A suit permutation: `perm.0[suit] = image suit`.
///
/// Deserializing a non-bijective or out-of-range tuple is rejected with a serde error naming
/// the offending suit/image. Direct tuple construction (`SuitPerm([...])`) is not itself gated
/// -- `ALL_PERMS` and test code rely on it -- but every public operation that *uses* a
/// caller-supplied permutation other than `apply` enforces the bijection invariant with an
/// always-on assertion (standing ruling: invariants are enforced with always-on asserts, not
/// `debug_assert!`); `apply` relies on `Card::new`'s own suit assert instead, since a single
/// application only needs its target suit in range, not the whole map to be a bijection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct SuitPerm(pub [u8; 4]);

impl SuitPerm {
    pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);
}

/// `Err` names the offending suit index and image when `images` is not a bijection on `0..4`.
fn validate_perm(images: &[u8; 4]) -> Result<(), String> {
    let mut seen = [false; 4];
    for (suit, &image) in images.iter().enumerate() {
        if image >= 4 {
            return Err(format!("SuitPerm: image {image} at suit {suit} is outside 0..4"));
        }
        if seen[image as usize] {
            return Err(format!(
                "SuitPerm: image {image} at suit {suit} repeats an earlier suit's image (not a bijection)"
            ));
        }
        seen[image as usize] = true;
    }
    Ok(())
}

/// Always-on assertion (standing ruling) that `p` is a bijection on `0..4`. See the `SuitPerm`
/// doc comment for which public operations call this.
fn assert_valid_perm(p: &SuitPerm) {
    if let Err(msg) = validate_perm(&p.0) {
        panic!("{msg}");
    }
}

impl<'de> Deserialize<'de> for SuitPerm {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<SuitPerm, D::Error> {
        let images = <[u8; 4]>::deserialize(d)?;
        validate_perm(&images).map_err(de::Error::custom)?;
        Ok(SuitPerm(images))
    }
}

/// All 24 suit permutations in lexicographic order.
pub const ALL_PERMS: [[u8; 4]; 24] = [
    [0, 1, 2, 3], [0, 1, 3, 2], [0, 2, 1, 3], [0, 2, 3, 1], [0, 3, 1, 2], [0, 3, 2, 1],
    [1, 0, 2, 3], [1, 0, 3, 2], [1, 2, 0, 3], [1, 2, 3, 0], [1, 3, 0, 2], [1, 3, 2, 0],
    [2, 0, 1, 3], [2, 0, 3, 1], [2, 1, 0, 3], [2, 1, 3, 0], [2, 3, 0, 1], [2, 3, 1, 0],
    [3, 0, 1, 2], [3, 0, 2, 1], [3, 1, 0, 2], [3, 1, 2, 0], [3, 2, 0, 1], [3, 2, 1, 0],
];

/// Canonical board: the flop sorted ascending by card id, then turn and river in dealt order.
///
/// Deserializing an out-of-shape (not 3..=5 cards), out-of-domain (card id `>= 52`),
/// duplicate-card, or non-canonical payload is rejected with a serde error, because a decoder
/// cannot silently re-canonicalize what it reads: the ranges that motivate the spec section 2
/// tie-break are not carried on the wire. `Serialize` stays derived (a `CanonicalBoard` is
/// always already valid by construction); `Deserialize` is written by hand to add this
/// validation, reusing `assert_valid_board` and `canonicalize` itself.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct CanonicalBoard { cards: Vec<Card> }

impl CanonicalBoard {
    pub fn cards(&self) -> &[Card] { &self.cards }
    pub fn flop(&self) -> &[Card] { &self.cards[..3] }
}

/// Always-on assertion (standing ruling) that `board` has 3 to 5 cards, every card id is `<52`,
/// and no card repeats -- named after the offending index so a caller can find the bad input.
/// Shared by `canonicalize`, `orbit_size_of`, and `CanonicalBoard`'s `Deserialize`.
fn assert_valid_board(board: &[Card]) {
    assert!(
        (3..=5).contains(&board.len()),
        "board: length {} is outside 3..=5", board.len()
    );
    for (i, c) in board.iter().enumerate() {
        assert!(c.0 < 52, "board: card {c:?} at index {i} has id {} outside 0..52", c.0);
    }
    for i in 0..board.len() {
        for j in (i + 1)..board.len() {
            assert!(
                board[i] != board[j],
                "board: duplicate card {:?} at indices {i} and {j}", board[i]
            );
        }
    }
}

#[derive(Deserialize)]
struct RawCanonicalBoard { cards: Vec<Card> }

impl<'de> Deserialize<'de> for CanonicalBoard {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<CanonicalBoard, D::Error> {
        let raw = RawCanonicalBoard::deserialize(d)?;
        if !(3..=5).contains(&raw.cards.len()) {
            return Err(de::Error::custom(format!(
                "CanonicalBoard: length {} is outside 3..=5", raw.cards.len()
            )));
        }
        for (i, c) in raw.cards.iter().enumerate() {
            if c.0 >= 52 {
                return Err(de::Error::custom(format!(
                    "CanonicalBoard: card {c:?} at index {i} has id {} outside 0..52", c.0
                )));
            }
        }
        for i in 0..raw.cards.len() {
            for j in (i + 1)..raw.cards.len() {
                if raw.cards[i] == raw.cards[j] {
                    return Err(de::Error::custom(format!(
                        "CanonicalBoard: duplicate card {:?} at indices {i} and {j}", raw.cards[i]
                    )));
                }
            }
        }
        let (canonical, _) = canonicalize(&raw.cards, &[]);
        if canonical.cards != raw.cards {
            return Err(de::Error::custom(
                "CanonicalBoard: payload is not already in canonical form",
            ));
        }
        Ok(CanonicalBoard { cards: raw.cards })
    }
}

pub fn apply(p: &SuitPerm, c: Card) -> Card { Card::new(c.rank(), p.0[c.suit() as usize]) }

pub fn inverse(p: &SuitPerm) -> SuitPerm {
    assert_valid_perm(p);
    let mut inv = [0u8; 4];
    for (suit, image) in p.0.iter().enumerate() { inv[*image as usize] = suit as u8; }
    SuitPerm(inv)
}

/// Moves the weight of combo (a, b) to combo (p(a), p(b)), bit-for-bit -- including `-0.0`,
/// which `Range1326` accepts as a valid weight and which the spec section 2 tie-break compares
/// by bit pattern (`f32::to_bits`). Silently normalizing `-0.0` to `+0.0` here would let a
/// different, non-bit-minimal permutation win a tie (see `iso_tiebreak_negative_zero_regression`
/// in `tests/iso.rs`), so every weight is transferred unconditionally rather than skipped when
/// `w != 0.0` (which is also true of `-0.0`).
pub fn apply_range(p: &SuitPerm, r: &Range1326) -> Range1326 {
    assert_valid_perm(p);
    let mut out = Range1326::zero();
    for i in 0..COMBOS {
        let [a, b] = combo_cards(i as u16);
        out.set(combo_index(apply(p, a), apply(p, b)), r.0[i]);
    }
    out
}

fn board_key(p: &SuitPerm, board: &[Card]) -> Vec<u8> {
    let mut key: Vec<u8> = board[..3].iter().map(|c| apply(p, *c).0).collect();
    key.sort_unstable();
    key.extend(board[3..].iter().map(|c| apply(p, *c).0));
    key
}

/// The `(oop, ip)` tuple serialized as f32 bit patterns in combo order (spec section 2 tie-break).
fn serialized(p: &SuitPerm, ranges: &[&Range1326]) -> Vec<u32> {
    ranges.iter().flat_map(|r| apply_range(p, r).0.into_iter().map(f32::to_bits).collect::<Vec<u32>>()).collect()
}

/// Spec section 2: minimal board key over the 24 permutations; ties broken by the minimal serialized ranges, then the minimal permutation.
pub fn canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm) {
    assert_valid_board(board);
    let mut best_key: Option<Vec<u8>> = None;
    let mut candidates: Vec<SuitPerm> = Vec::new();
    for p in ALL_PERMS {
        let perm = SuitPerm(p);
        let key = board_key(&perm, board);
        match &best_key {
            Some(k) if key > *k => {}
            Some(k) if key == *k => candidates.push(perm),
            _ => { best_key = Some(key); candidates = vec![perm]; }
        }
    }
    let key = best_key.expect("at least one permutation");
    let mut chosen = candidates[0];
    if candidates.len() > 1 && !ranges.is_empty() {
        let mut best = serialized(&chosen, ranges);
        for p in &candidates[1..] {
            let ser = serialized(p, ranges);
            if ser < best { best = ser; chosen = *p; }
        }
    }
    assert_valid_perm(&chosen);
    (CanonicalBoard { cards: key.into_iter().map(Card).collect() }, chosen)
}

/// `24 / |stabilizer|` of the board (flop as a set, turn and river ordered).
pub fn orbit_size_of(board: &[Card]) -> u8 {
    assert_valid_board(board);
    let identity = board_key(&SuitPerm::IDENTITY, board);
    let stabilizer = ALL_PERMS.iter().filter(|p| board_key(&SuitPerm(**p), board) == identity).count();
    (24 / stabilizer) as u8
}

pub fn orbit_size(board: &CanonicalBoard) -> u8 { orbit_size_of(board.cards()) }
