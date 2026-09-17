use proto::{combo_cards, combo_index, Card, Range1326, COMBOS};
use serde::{Deserialize, Serialize};

/// A suit permutation: `perm.0[suit] = image suit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SuitPerm(pub [u8; 4]);

impl SuitPerm {
    pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);
}

/// All 24 suit permutations in lexicographic order.
pub const ALL_PERMS: [[u8; 4]; 24] = [
    [0, 1, 2, 3], [0, 1, 3, 2], [0, 2, 1, 3], [0, 2, 3, 1], [0, 3, 1, 2], [0, 3, 2, 1],
    [1, 0, 2, 3], [1, 0, 3, 2], [1, 2, 0, 3], [1, 2, 3, 0], [1, 3, 0, 2], [1, 3, 2, 0],
    [2, 0, 1, 3], [2, 0, 3, 1], [2, 1, 0, 3], [2, 1, 3, 0], [2, 3, 0, 1], [2, 3, 1, 0],
    [3, 0, 1, 2], [3, 0, 2, 1], [3, 1, 0, 2], [3, 1, 2, 0], [3, 2, 0, 1], [3, 2, 1, 0],
];

/// Canonical board: the flop sorted ascending by card id, then turn and river in dealt order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalBoard { cards: Vec<Card> }

impl CanonicalBoard {
    pub fn cards(&self) -> &[Card] { &self.cards }
    pub fn flop(&self) -> &[Card] { &self.cards[..3] }
}

pub fn apply(p: &SuitPerm, c: Card) -> Card { Card::new(c.rank(), p.0[c.suit() as usize]) }

pub fn inverse(p: &SuitPerm) -> SuitPerm {
    let mut inv = [0u8; 4];
    for (suit, image) in p.0.iter().enumerate() { inv[*image as usize] = suit as u8; }
    SuitPerm(inv)
}

/// Moves the weight of combo (a, b) to combo (p(a), p(b)).
pub fn apply_range(p: &SuitPerm, r: &Range1326) -> Range1326 {
    let mut out = Range1326::zero();
    for i in 0..COMBOS {
        let w = r.0[i];
        if w != 0.0 {
            let [a, b] = combo_cards(i as u16);
            out.set(combo_index(apply(p, a), apply(p, b)), w);
        }
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
    assert!((3..=5).contains(&board.len()), "canonicalize needs 3 to 5 board cards");
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
    (CanonicalBoard { cards: key.into_iter().map(Card).collect() }, chosen)
}

/// `24 / |stabilizer|` of the board (flop as a set, turn and river ordered).
pub fn orbit_size_of(board: &[Card]) -> u8 {
    let identity = board_key(&SuitPerm::IDENTITY, board);
    let stabilizer = ALL_PERMS.iter().filter(|p| board_key(&SuitPerm(**p), board) == identity).count();
    (24 / stabilizer) as u8
}

pub fn orbit_size(board: &CanonicalBoard) -> u8 { orbit_size_of(board.cards()) }
