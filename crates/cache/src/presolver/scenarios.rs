//! Spec section 10.5: the 24 pre-solver scenarios (4 tier-1 SRP at 100bb, 4 tier-2 SRP + 4
//! tier-2 3-bet at 100bb, and the same twelve lines repeated at 200bb as tier-3), plus the
//! 1,755-class canonical-flop enumeration order that the task-14 queue walks against them.

use proto::{Card, Position, Range1326};

/// One pre-solver job template: a stack depth and a preflop line (spec section 10.5). In the
/// 3-bet cases `caller` denotes the original opener who calls the 3-bet; the opponent who put
/// in the 3-bet is `three_bettor`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Scenario {
    pub tier: u8,
    pub depth_bb: u16,
    pub opener: Position,
    pub caller: Position,
    pub three_bettor: Option<Position>,
}

impl Scenario {
    /// Deterministic, stable across runs: it keys `queue.json` items.
    pub fn id(&self) -> String {
        match self.three_bettor {
            None => format!(
                "t{}-{}bb-{:?}-open-{:?}-call",
                self.tier, self.depth_bb, self.opener, self.caller
            ),
            Some(b) => format!(
                "t{}-{}bb-{:?}-open-{:?}-3bet-{:?}-call",
                self.tier, self.depth_bb, self.opener, b, self.caller
            ),
        }
    }
}

/// The 24 scenarios in tier order (spec section 10.5): 4 tier-1 SRP, then 4 tier-2 SRP + 4
/// tier-2 3-bet (all at 100bb), then the same twelve opener/caller/three-bettor lines again at
/// 200bb as tier-3.
pub fn scenarios() -> Vec<Scenario> {
    use Position::*;
    let lines = [
        (Btn, Bb, None), (Co, Bb, None), (Hj, Bb, None), (Utg, Bb, None),
        (Sb, Bb, None), (Btn, Sb, None), (Co, Btn, None), (Hj, Btn, None),
        (Btn, Btn, Some(Bb)), (Co, Co, Some(Btn)), (Btn, Btn, Some(Sb)), (Hj, Hj, Some(Btn)),
    ];
    let mut out = Vec::new();
    for (i, (opener, caller, three_bettor)) in lines.iter().cloned().enumerate() {
        out.push(Scenario { tier: if i < 4 { 1 } else { 2 }, depth_bb: 100, opener, caller, three_bettor });
    }
    for (opener, caller, three_bettor) in lines {
        out.push(Scenario { tier: 3, depth_bb: 200, opener, caller, three_bettor });
    }
    out
}

/// Frozen count of canonical flop classes (spec section 10.4): 1,755 classes represent all
/// 22,100 raw flops. Frozen so the cheap unit test never pays for the enumeration (review m5).
pub const CANONICAL_FLOP_COUNT: usize = 1755;

/// All `C(52, 3) = 22,100` raw flops, ascending by card id, no repeats.
pub fn raw_flops() -> impl Iterator<Item = [Card; 3]> {
    (0_u8..50).flat_map(|a| {
        ((a + 1)..51).flat_map(move |b| ((b + 1)..52).map(move |c| [Card(a), Card(b), Card(c)]))
    })
}

/// The 1,755 canonical flop classes, ordered descending by orbit size (24 before 12 before 4)
/// and then by ascending canonical card ids. Computed at most once per process: 22,100
/// canonicalizations are minutes in a debug build, so `task-14`'s queue and this module's own
/// exhaustive test both pay for it only once via `OnceLock`.
///
/// Board-only canonical representative enumeration uses zero ranges (the canonical board is
/// unaffected by the range tie-break); this never hashes or solves with those zero ranges. At
/// actual job preparation, canonicalize again using the replayed public ranges for the
/// stabilizer tie-break -- never `flop_full_v1`.
pub fn canonical_flops_ordered() -> &'static [Vec<Card>] {
    static ORDER: std::sync::OnceLock<Vec<Vec<Card>>> = std::sync::OnceLock::new();
    ORDER
        .get_or_init(|| {
            let zero = Range1326::zero();
            let mut unique = std::collections::BTreeMap::new();
            for raw in raw_flops() {
                let (canonical, perm) = core_iso::canonicalize(&raw, &[&zero, &zero]);
                let mut cards = raw.iter().map(|c| core_iso::apply(&perm, *c)).collect::<Vec<_>>();
                cards.sort_by_key(|c| c.0);
                let key = cards.iter().map(|c| c.0).collect::<Vec<_>>();
                unique.entry(key).or_insert((core_iso::orbit_size(&canonical), cards));
            }
            let mut rows = unique.into_iter().collect::<Vec<_>>();
            rows.sort_by(|(a, (oa, _)), (b, (ob, _))| ob.cmp(oa).then(a.cmp(b)));
            rows.into_iter().map(|(_, (_, cards))| cards).collect()
        })
        .as_slice()
}
