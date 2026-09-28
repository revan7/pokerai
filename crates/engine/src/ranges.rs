//! The root-range source (spec §9): the public ranges both players hold at a street root.
//!
//! `RangeSource::ranges_at_root` is the only root-range provider (cross-plan M15, M16; orchestrator interface request
//! g). Plan 3 Task 18 implements it for the replay-backed `crate::replay_bridge::ReplayRanges`, which `Engine::new`
//! installs into `EngineCore.range_source` (`EngineCore::install_replay_ranges`) and which reuses this module's two
//! validators; this module ships `ExplicitRanges`, used by the tests and by `bench`. Hero's cards never enter a public
//! range (spec §2): the board is the only thing blocked here.

use crate::snapshots::SnapshotProvenance;
use core_ranges::{block_public, mass, range_to_string};
use proto::{combo_cards, ApproxReason, ComboIndex, HandState, Range1326, Seat, Street, StreetRootSnapshot, UnsupportedReason};

pub struct RootRanges {
    pub oop: Range1326,
    pub ip: Range1326,
    pub reasons: Vec<ApproxReason>,
    /// `(seat, range text, mass)` for OOP then IP, of the public (board-blocked) ranges handed to the solve.
    pub ranges_used: Vec<(Seat, String, f32)>,
    /// Plan 3 Task 18 (fix round 1, ruling 18-I3; spec 9.3): the provenance of every snapshot a prior street was
    /// conditioned through to reach these ranges, in street order (`core_replay::ReplayOutput::snapshots_used`), which
    /// `serve` discloses in the current result's assumptions. Empty for a source that replays nothing (`ExplicitRanges`).
    pub snapshots_used: Vec<(Street, SnapshotProvenance)>,
}

/// The only root-range provider (§9). Plan 3 implements it for a replay-backed type and installs that into
/// `EngineCore.range_source`; this plan ships the explicit-ranges implementation used by the tests and by `bench`.
/// Every implementation returns public ranges only (hero's cards never blocked or removed) and answers
/// `InvalidRanges` for empty or jointly incompatible ranges (spec §13 failure table).
pub trait RangeSource: Send {
    fn ranges_at_root(&self, state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason>;
}

/// Fixed public ranges for OOP and IP, as given (for example by a bench spot). `None` on either side is
/// `InvalidRanges`.
pub struct ExplicitRanges {
    pub oop: Option<Range1326>,
    pub ip: Option<Range1326>,
}

impl RangeSource for ExplicitRanges {
    /// Validates both ranges as given, blocks the board (never hero's cards) and requires support on both sides and at
    /// least one jointly compatible holding; anything else is `InvalidRanges` (spec §13 failure table).
    fn ranges_at_root(&self, _state: &HandState, root: &StreetRootSnapshot) -> Result<RootRanges, UnsupportedReason> {
        let (Some(mut oop), Some(mut ip)) = (self.oop.clone(), self.ip.clone()) else { return Err(UnsupportedReason::InvalidRanges) };
        if !weights_valid(&oop) || !weights_valid(&ip) {
            return Err(UnsupportedReason::InvalidRanges);
        }
        block_public(&mut oop, &root.board);
        block_public(&mut ip, &root.board);
        if mass(&oop) <= 0.0 || mass(&ip) <= 0.0 || !jointly_compatible(&oop, &ip) {
            return Err(UnsupportedReason::InvalidRanges);
        }
        let used = vec![(root.oop, range_to_string(&oop), mass(&oop)), (root.ip, range_to_string(&ip), mass(&ip))];
        Ok(RootRanges { oop, ip, reasons: vec![], ranges_used: used, snapshots_used: vec![] })
    }
}

/// Every weight is a finite frequency in `[0, 1]` (the domain the worker's range adapter accepts). Checked on the range
/// as given, before blocking: a malformed weight is rejected even on a combo the board would remove. Crate-visible: the
/// replay range source validates with it too (plan-2 carry P2T27R).
pub(crate) fn weights_valid(r: &Range1326) -> bool {
    r.0.iter().all(|w| w.is_finite() && (0.0..=1.0).contains(w))
}

/// Whether some OOP combo and some IP combo, both with positive weight, share no card. For an OOP combo `{a, b}` the
/// clashing IP combos are those holding `a` or `b`; by inclusion-exclusion the compatible count is
/// `support - with[a] - with[b] + [ip holds {a, b}]`, compared here without subtraction so it cannot underflow.
/// Crate-visible: the replay range source validates with it too (plan-2 carry P2T27R).
pub(crate) fn jointly_compatible(oop: &Range1326, ip: &Range1326) -> bool {
    let mut support = 0u32;
    let mut with = [0u32; 52];
    for (i, w) in ip.0.iter().enumerate() {
        if *w > 0.0 {
            let [a, b] = combo_cards(i as ComboIndex);
            support += 1;
            with[usize::from(a.0)] += 1;
            with[usize::from(b.0)] += 1;
        }
    }
    oop.0.iter().enumerate().any(|(i, w)| {
        if *w <= 0.0 {
            return false;
        }
        let [a, b] = combo_cards(i as ComboIndex);
        support + u32::from(ip.0[i] > 0.0) > with[usize::from(a.0)] + with[usize::from(b.0)]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Card, Street};
    fn root() -> StreetRootSnapshot {
        StreetRootSnapshot { street: Street::River, board: ["Kh", "7d", "2c", "4d", "9s"].iter().map(|s| Card::parse(s).unwrap()).collect(),
            oop: Seat(2), ip: Seat(0), pot_root: 100, stack_oop_root: 500, stack_ip_root: 500, dead_this_street: 0, projected_from: 2, history: vec![], bb_chips: 2 }
    }
    /// The same six-handed $1/$2-style hand, built through the shared hand builder (`crate::testing::hand`,
    /// `testing.rs:816`; P2T27-M1).
    fn hand_with(hero_cards: Option<[Card; 2]>) -> HandState {
        crate::testing::hand(&(0..6).map(|i| (Seat(i), 1000)).collect::<Vec<_>>(), Seat(0), Seat(2), hero_cards)
    }
    fn state() -> HandState { hand_with(None) }
    #[test]
    fn explicit_ranges_block_the_board_and_reject_empty_support() {
        let full = Range1326([1.0; 1326]);
        let r = ExplicitRanges { oop: Some(full.clone()), ip: Some(full) }.ranges_at_root(&state(), &root()).unwrap();
        // 5 board cards remove 5 * 51 - C(5,2) = 245 combos from each side
        assert_eq!(r.oop.0.iter().filter(|w| **w > 0.0).count(), 1326 - 245);
        assert_eq!(r.ranges_used.len(), 2);
        assert!(r.reasons.is_empty());
        assert!(matches!(ExplicitRanges { oop: None, ip: None }.ranges_at_root(&state(), &root()), Err(UnsupportedReason::InvalidRanges)));
        // a range made entirely of board blockers has no support left
        let mut only_board = Range1326([0.0; 1326]);
        only_board.0[proto::combo_index(Card::parse("Kh").unwrap(), Card::parse("7d").unwrap()) as usize] = 1.0;
        assert!(matches!(ExplicitRanges { oop: Some(only_board), ip: Some(Range1326([1.0; 1326])) }.ranges_at_root(&state(), &root()), Err(UnsupportedReason::InvalidRanges)));
    }

    /// Hero's cards never enter a public range (CLAUDE.md section 6): with hero holding AsAh, every combo that touches
    /// no board card keeps its weight on both sides, hero's own combo included, and `ranges_used` reports the public
    /// (board-blocked) ranges in (oop, ip) order.
    #[test]
    fn hero_cards_are_never_blocked_and_ranges_used_reports_the_public_ranges() {
        let hero = [Card::parse("As").unwrap(), Card::parse("Ah").unwrap()];
        let board = root().board;
        let full = Range1326([1.0; 1326]);
        let r = ExplicitRanges { oop: Some(full.clone()), ip: Some(full) }.ranges_at_root(&hand_with(Some(hero)), &root()).unwrap();
        for i in 0..proto::COMBOS {
            let [a, b] = proto::combo_cards(i as proto::ComboIndex);
            let expected = if board.contains(&a) || board.contains(&b) { 0.0 } else { 1.0 };
            assert_eq!((r.oop.0[i], r.ip.0[i]), (expected, expected), "combo {a}{b}");
        }
        assert_eq!(r.oop.0[proto::combo_index(hero[0], hero[1]) as usize], 1.0, "hero's own combo stays in the public range");
        let used: Vec<(Seat, String, f32)> = r.ranges_used.clone();
        assert_eq!(used, vec![(Seat(2), range_to_string(&r.oop), 1081.0), (Seat(0), range_to_string(&r.ip), 1081.0)]);
    }

    /// Weights are validated as given, before blocking (a NaN, an infinity, a negative or a weight above 1 is never a
    /// range, even on a combo the board would remove), and two ranges with no jointly compatible holdings are
    /// `InvalidRanges` (spec §13 failure table), even though each side on its own has support.
    #[test]
    fn malformed_weights_and_jointly_incompatible_ranges_are_invalid() {
        let c = |s: &str| Card::parse(s).unwrap();
        let full = || Range1326([1.0; 1326]);
        let one = |a: &str, b: &str| { let mut r = Range1326([0.0; 1326]); r.0[proto::combo_index(c(a), c(b)) as usize] = 1.0; r };
        let at_root = |oop: Range1326, ip: Range1326| ExplicitRanges { oop: Some(oop), ip: Some(ip) }.ranges_at_root(&state(), &root());
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.5, 1.5] {
            for combo in [("As", "Ks"), ("Kh", "7d")] {
                let mut r = full();
                r.0[proto::combo_index(c(combo.0), c(combo.1)) as usize] = bad;
                assert!(matches!(at_root(r.clone(), full()), Err(UnsupportedReason::InvalidRanges)), "oop weight {bad} on {combo:?}");
                assert!(matches!(at_root(full(), r), Err(UnsupportedReason::InvalidRanges)), "ip weight {bad} on {combo:?}");
            }
        }
        // both sides hold only combos sharing the As, or the very same combo: no jointly compatible holding
        assert!(matches!(at_root(one("As", "Ah"), one("As", "Ks")), Err(UnsupportedReason::InvalidRanges)));
        assert!(matches!(at_root(one("As", "Ah"), one("As", "Ah")), Err(UnsupportedReason::InvalidRanges)));
        // IP holding OOP's very combo plus one disjoint combo is compatible (the shared combo holds both of OOP's cards
        // and is counted once, not twice, among the clashing IP combos)
        let mut shared_and_disjoint = one("As", "Ah");
        shared_and_disjoint.0[proto::combo_index(c("Ks"), c("Kc")) as usize] = 1.0;
        assert!(at_root(one("As", "Ah"), shared_and_disjoint).is_ok());
        // a single disjoint pair is enough, and a partial weight in [0, 1] is a valid range
        let mut partial = one("Qs", "Qh");
        partial.0[proto::combo_index(c("Qs"), c("Qh")) as usize] = 0.25;
        let ok = at_root(one("As", "Ah"), partial).unwrap();
        assert_eq!((ok.ranges_used[0].2, ok.ranges_used[1].2), (1.0, 0.25));
    }
}
