//! Lookup-side match rules (spec section 10.4): whether a stored `CacheEntry` may stand in for
//! a live query -- equal full topology across every street, the exact-rational SPR delta bucket
//! search tolerance, and terminal rake-cap activation -- plus the exact rational ranking values
//! (`delta`, `max_dev`) a caller orders candidate entries by. `compare` never substitutes a
//! different canonical board (CLAUDE.md section 6 "no nearest-flop substitution"); board/range/
//! key identity is the caller's job (the `b - 1, b, b + 1` `spr_bucket` digest search), this
//! module only judges whether an already key-matched entry's *tree* still matches the query.

use crate::entry::sorted_by_path;

/// A command for the cache's bounded reader thread, the type `Cache::reader`'s channel carries
/// (plan 4 task 6's `crate::lookup::ReadCommand`). Task 6 leaves `Cache::reader` as `None` and
/// needs only `Shutdown`, which `Cache::shutdown` sends; task 7 adds the `Cells` read request here
/// and installs the reader thread (`Cache::start_reader`) that consumes both.
pub enum ReadCommand {
    Shutdown,
}

/// The exact-rational comparison of a candidate `CacheEntry` against a live query: `delta` is
/// the SPR relative difference, `max_dev` the maximum pot-fraction deviation across every wager
/// menu entry that survived the topology check (spec section 10.4 `delta`/`dev`). The `f64`
/// fields are display/threshold values recomputed from the `_num`/`_den` pairs; `rank` orders
/// candidates on the exact rational cross-products so two entries whose `f64` values happen to
/// round the same way are still ordered correctly (spec: "order by delta, maximum deviation").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    pub delta: f64,
    pub max_dev: f64,
    pub delta_num: u128,
    pub delta_den: u128,
    pub menu_num: u128,
    pub menu_den: u128,
}

impl Comparison {
    /// Orders `self` before `other` when `self` is the closer match: smaller SPR delta first,
    /// then smaller maximum menu deviation. Cross-multiplies rather than comparing the `f64`
    /// display fields, so the ordering is exact even when `delta`/`max_dev` were rounded the
    /// same by `f64` division.
    pub fn rank(&self, other: &Self) -> std::cmp::Ordering {
        (self.delta_num * other.delta_den)
            .cmp(&(other.delta_num * self.delta_den))
            .then((self.menu_num * other.menu_den).cmp(&(other.menu_num * self.menu_den)))
    }
}

/// The chip amount a wager action names, or `None` for an action with no size (`Fold`, `Check`,
/// `Call`). Used to compute the pot-fraction deviation between an entry's menu entry and the
/// query's corresponding one.
pub fn action_to(a: &proto::Action) -> Option<u32> {
    match a {
        proto::Action::Bet { to } | proto::Action::Raise { to } | proto::Action::AllIn { to } => Some(*to),
        _ => None,
    }
}

/// Whether a terminal's rake-cap activation agrees between an entry's chip pot `pe` (under its
/// own absolute cap `ce`, thousandths of a chip) and the query's chip pot `pq` (under its own
/// absolute cap `cq`), at the shared normalized rake `rate` both were solved under (spec section
/// 10.4: "topology + terminal rake-cap predicate"). `rate` already agreed between entry and
/// query before `compare` is called (it is part of the normalized `RakeKey`, matched by the
/// caller's key digest search); only the raw chip amounts and raw caps are compared here,
/// because the cap is a threshold on the *absolute* rake charged (`rate * pot >= cap / 1000`),
/// which the normalized `cap_over_p` fraction alone cannot re-derive at query time.
pub fn cap_agrees(rate: f64, ce: u32, pe: u32, cq: u32, pq: u32) -> bool {
    (rate * pe as f64 >= ce as f64 / 1000.0) == (rate * pq as f64 >= cq as f64 / 1000.0)
}

/// Whether cached entry `e` may stand in for a live query against effective tree `t`, at query
/// pot `p` (chips), query effective stack `eff` (chips, `min(stack_oop, stack_ip)`) and query
/// absolute rake cap `cap` (`cap_mchips`). `None` if `p` or `eff` is zero, if the SPR relative
/// delta exceeds 0.02, if the two trees do not have equal full topology on every street (path,
/// actor, street, action-kind menu shape, and terminal-vs-continuation classification at every
/// action -- a rake-cap boundary that flips a terminal's classification counts as a topology
/// change even with identical kinds/paths/actors/streets), or if the maximum pot-fraction wager
/// deviation exceeds 0.05. All arithmetic is exact `u128` (no float comparison of SPRs or
/// deviations); `pot + stacks < 2^31` (spec section 2) keeps every cross-product used here well
/// inside `u128`.
pub fn compare(e: &crate::entry::CacheEntry, t: &proto::EffectiveTree, p: u32, eff: u32, cap: u32) -> Option<Comparison> {
    if p == 0 || eff == 0 {
        return None;
    }
    let pe = e.source.pot as u128;
    let pq = p as u128;
    let ee = e.source.stack_oop.min(e.source.stack_ip) as u128;
    // spec 10.4: delta = abs(SPR_query - SPR_entry) / SPR_entry, SPR_query = eff/p, SPR_entry =
    // ee/pe; multiplying through by pe*p*ee gives the exact-integer form compared below.
    let difference = (eff as u128 * pe).abs_diff(ee * pq);
    if 100 * difference > 2 * ee * pq {
        return None;
    }
    // Both lists must already be ordinal-path sorted (`validate_entry` enforces this for the
    // entry side; the tree materializer does for the query side) before a positional zip is a
    // total, order-independent comparison of the two trees.
    if e.tree.materialized.len() != t.materialized.len() || !sorted_by_path(&e.tree.materialized) || !sorted_by_path(&t.materialized) {
        return None;
    }
    let mut maximum = 0_u128;
    for (a, b) in e.tree.materialized.iter().zip(&t.materialized) {
        if a.path != b.path
            || a.actor != b.actor
            || a.street != b.street
            || a.actions.len() != b.actions.len()
            || a.terminal_pots.len() != a.actions.len()
            || b.terminal_pots.len() != b.actions.len()
        {
            return None;
        }
        for i in 0..a.actions.len() {
            if std::mem::discriminant(&a.actions[i]) != std::mem::discriminant(&b.actions[i]) {
                return None;
            }
            match (a.terminal_pots[i], b.terminal_pots[i]) {
                (None, None) => {}
                (Some(x), Some(y)) if cap_agrees(e.key.rake.clone().rate() as f64, e.source.cap_mchips, x, cap, y) => {}
                _ => return None,
            }
            if let (Some(x), Some(y)) = (action_to(&a.actions[i]), action_to(&b.actions[i])) {
                // spec 10.4: dev = abs(to_query/P_query - to_entry/P_entry); multiplying through
                // by pe*pq gives the exact-integer form accumulated as `maximum` below.
                maximum = maximum.max((y as u128 * pe).abs_diff(x as u128 * pq));
            }
        }
    }
    if 100 * maximum > 5 * pe * pq {
        return None;
    }
    Some(Comparison {
        delta: difference as f64 / (ee * pq) as f64,
        max_dev: maximum as f64 / (pe * pq) as f64,
        delta_num: difference,
        delta_den: ee * pq,
        menu_num: maximum,
        menu_den: pe * pq,
    })
}
