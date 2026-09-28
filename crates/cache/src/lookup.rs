//! Lookup-side match rules (spec section 10.4): whether a stored `CacheEntry` may stand in for
//! a live query -- equal full topology across every street, the exact-rational SPR delta bucket
//! search tolerance, and terminal rake-cap activation -- plus the exact rational ranking values
//! (`delta`, `max_dev`) a caller orders candidate entries by. `compare` never substitutes a
//! different canonical board (CLAUDE.md section 6 "no nearest-flop substitution"); board/range/
//! key identity is the caller's job (the `b - 1, b, b + 1` `spr_bucket` digest search), this
//! module only judges whether an already key-matched entry's *tree* still matches the query.
//!
//! Task 7 adds the lookup itself: the query and result types (`CacheQuery`, `CacheHit`,
//! `Lookup`), strict menu legality (`legal_menu`), candidate selection over the cells the
//! `cache-reader` thread read (`select`), and node reconstruction in the query's own chips and
//! suits (`reconstruct`, `map_rows`, `map_flags`). `Cache::lookup` (in `crate`) drives them. A
//! canonical key that is absent from the store is a `Miss` to be solved live: nothing here ever
//! searches another board, another tree signature or another range hash.

use crate::entry::sorted_by_path;

/// A command for the cache's bounded reader thread, the type `Cache::reader`'s channel carries.
/// `Cells` asks for the cells at up to three cell keys (the `b - 1, b, b + 1` probe; duplicate
/// keys are read once) and is answered on `reply` with the request's own `token`, so a reply that
/// arrives after its waiter gave up can never be taken for another request's. `Shutdown` stops the
/// thread (`Cache::shutdown`).
pub enum ReadCommand {
    Cells { token: u64, keys: [[u8; 32]; 3], reply: std::sync::mpsc::SyncSender<(u64, Vec<crate::storage::Cell>)> },
    Shutdown,
}

/// One live lookup (spec section 10.4 lookup): the canonical key and exact source inputs of the
/// query's street root (`engine::cache_bridge::key_and_source`), the query's own effective tree,
/// the requested node by ordinal path and its actor (`"oop"`/`"ip"`), the legal menu at the live
/// state, the query's accuracy target and the reasons already incurred before the cache was asked,
/// the inverse of the suit permutation that canonicalized the query (canonical suits -> the
/// query's suits), and the remaining delivery budget. `bb_chips` (in `source`), `target_bp`, the
/// actor and the requested path travel beside `key`, never in it; hero's cards appear nowhere in a
/// query.
#[derive(Clone)]
pub struct CacheQuery {
    pub key: crate::key::KeyFields,
    pub source: crate::entry::SourceInputs,
    pub tree: proto::EffectiveTree,
    pub requested: proto::OrdinalPath,
    pub actor: String,
    pub legal: Vec<proto::LegalAction>,
    pub target_bp: u16,
    pub reasons: Vec<proto::ApproxReason>,
    pub inverse_perm: core_iso::SuitPerm,
    pub budget: std::time::Duration,
}

/// A served cache hit: a `StreetSolution` rebuilt in the query's chips and suits (validated
/// against the query tree), the query tree it indexes, the entry's covered ordinal paths, the
/// coverage the hit discloses (input/model matching only, never a full-game GTO claim), the raw
/// stored accuracy, the disclosure notes (the realized menu at the requested node and the source
/// storage mode), the entry's storage mode and the query's tree signature.
#[derive(Clone, Debug, PartialEq)]
pub struct CacheHit {
    pub solution: proto::worker::StreetSolution,
    pub tree: proto::EffectiveTree,
    pub covered_paths: Vec<proto::OrdinalPath>,
    pub coverage: proto::Coverage,
    pub raw_exploitability_over_p: f64,
    pub notes: Vec<String>,
    pub source_mode: String,
    pub tree_signature: String,
}

/// The lookup outcome (spec section 3.5): `Exact` and `Approximate` pass the query's raw
/// accuracy target, `Provisional` does not (and carries every reason an accuracy pass would have
/// carried, possibly none); `Miss` covers everything else -- no candidate, a query-side mismatch,
/// a disabled or stopped cache, a full reader queue and a spent budget alike.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    Exact { hit: CacheHit },
    Approximate { hit: CacheHit, reasons: Vec<proto::ApproxReason> },
    Provisional { hit: CacheHit, reasons: Vec<proto::ApproxReason> },
    Miss,
}

/// Strict legality of a served menu at the live state: every action must be offered by `legal`
/// exactly -- the same kind, a wager size inside its `[min_to, max_to]`, an all-in to exactly the
/// legal amount. Nothing is mapped or rounded and no probability is moved between actions: a menu
/// with any illegal entry is illegal as a whole.
pub fn legal_menu(actions: &[proto::Action], legal: &[proto::LegalAction]) -> bool {
    use proto::{Action as A, LegalAction as L};
    actions.iter().all(|a| {
        legal.iter().any(|l| match (a, l) {
            (A::Fold, L::Fold) | (A::Check, L::Check) | (A::Call, L::Call { .. }) => true,
            (A::Bet { to }, L::Bet { min_to, max_to }) | (A::Raise { to }, L::Raise { min_to, max_to }) => (*min_to..=*max_to).contains(to),
            (A::AllIn { to: a }, L::AllIn { to: b }) => a == b,
            _ => false,
        })
    })
}

/// Moves every combo row from its canonical-suit index to the index of its image under `inverse`
/// (canonical suits -> the query's suits). Used for probability rows and signed EV rows alike; the
/// rows are moved whole, never validated or renormalized here. `rows` must have 1326 rows.
pub fn map_rows(rows: &[Vec<f32>], inverse: &core_iso::SuitPerm) -> Vec<Vec<f32>> {
    let mut out = vec![Vec::new(); 1326];
    for hi in 1_u8..52 {
        for lo in 0_u8..hi {
            let a = core_iso::apply(inverse, proto::Card(lo)).0;
            let b = core_iso::apply(inverse, proto::Card(hi)).0;
            let (x, y) = (a.min(b) as usize, a.max(b) as usize);
            let source = hi as usize * (hi as usize - 1) / 2 + lo as usize;
            out[y * (y - 1) / 2 + x] = rows[source].clone();
        }
    }
    out
}

/// `map_rows` for the availability mask, through the identical source/destination indices.
pub fn map_flags(flags: &[bool], inverse: &core_iso::SuitPerm) -> Vec<bool> {
    let mut out = vec![false; 1326];
    for hi in 1_u8..52 {
        for lo in 0_u8..hi {
            let a = core_iso::apply(inverse, proto::Card(lo)).0;
            let b = core_iso::apply(inverse, proto::Card(hi)).0;
            let (x, y) = (a.min(b) as usize, a.max(b) as usize);
            out[y * (y - 1) / 2 + x] = flags[hi as usize * (hi as usize - 1) / 2 + lo as usize];
        }
    }
    out
}

/// The canonical payload digest of an entry (`created`/`last_hit` excluded): the identity the
/// writer's index and `Cache::touch` name an entry by, and `select`'s final tie-break. It is
/// `quota::entry_digest` itself, so a touch can never name an entry by a different digest than the
/// one the writer indexed it under.
pub(crate) fn payload_digest(e: &crate::entry::CacheEntry) -> Vec<u8> {
    crate::quota::entry_digest(e)
}

/// Rebuilds the requested node set in the query's chips and suits (spec section 10.4 lookup step
/// 4). Every exported node takes the query's materialized actions and chip path at the identical
/// ordinal position; probability and EV rows and availability flags move under
/// `q.inverse_perm`; EV is `ev_over_P * P_query`, never re-rounded from the entry. The rebuilt
/// solution is validated against the query tree and the requested node's menu must be strictly
/// legal (`legal_menu`). Returns `None` for any query-side mismatch: the caller turns that into
/// `Lookup::Miss` without deleting the source file.
pub fn reconstruct(e: &crate::entry::CacheEntry, q: &CacheQuery) -> Option<CacheHit> {
    let p = q.source.pot as f32;
    let mut nodes = Vec::with_capacity(e.nodes.len());
    for cached in &e.nodes {
        // `map_rows`/`map_flags` index 1326 rows; a decoded entry always has them (validated).
        if cached.probs.len() != 1326 || cached.ev_over_P.len() != 1326 || cached.available.len() != 1326 {
            return None;
        }
        let m = q.tree.materialized.iter().find(|m| m.path == cached.path)?;
        if m.actor != cached.actor {
            return None;
        }
        nodes.push(proto::worker::NodeStrategy {
            path: crate::entry::chip_path(&q.tree.materialized, &cached.path)?,
            actor: cached.actor.clone(),
            actions: m.actions.clone(),
            probs: map_rows(&cached.probs, &q.inverse_perm),
            ev_chips: map_rows(&cached.ev_over_P, &q.inverse_perm).into_iter().map(|row| row.into_iter().map(|v| v * p).collect()).collect(),
            available: map_flags(&cached.available, &q.inverse_perm),
        });
    }
    let requested = e.nodes.iter().position(|n| n.path == q.requested)?;
    if nodes[requested].actor != q.actor {
        return None;
    }
    let covered_paths = e.covered_paths.clone();
    let solution = proto::worker::StreetSolution {
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        requested: u32::try_from(requested).ok()?,
        exploitability_chips: (e.exploitability_over_P * q.source.pot as f64) as f32,
        iterations: e.iterations,
        memory_bytes: e.memory_bytes,
        mode: e.mode.clone(),
        locks_applied: e.locks_applied,
        export: e.export.clone(),
    };
    proto::worker::validate_solution(&solution, &q.tree.materialized).ok()?;
    if !legal_menu(&solution.nodes[requested].actions, &q.legal) {
        return None;
    }
    let realized = solution.nodes[requested].actions.iter().filter_map(|a| action_to(a).map(|to| format!("{to}"))).collect::<Vec<_>>().join(", ");
    Some(CacheHit {
        tree: q.tree.clone(),
        covered_paths,
        coverage: proto::Coverage::Exact,
        raw_exploitability_over_p: e.exploitability_over_P,
        notes: vec![format!("cache realized menu at the requested node: [{realized}]"), format!("cache source storage mode: {}", e.mode)],
        source_mode: e.mode.clone(),
        tree_signature: q.key.tree_signature.clone(),
        solution,
    })
}

/// Spec 10.4: over the cells read at buckets `b - 1`, `b`, `b + 1`, keeps every entry that
/// re-validates, shares the query's key in every field but the SPR bucket (`at_bucket(0)`),
/// covers the requested ordinal path with the query's actor, and passes `compare`; then ranks the
/// survivors by `Comparison::rank` (SPR delta, then menu deviation), then raw exploitability, then
/// payload digest. An entry above the query's accuracy target stays a survivor (it is labelled
/// `Provisional` afterwards): accuracy never reorders ahead of closeness. An excluded candidate is
/// simply not served; nothing here deletes anything.
pub fn select(cells: Vec<crate::storage::Cell>, q: &CacheQuery) -> Option<(crate::entry::CacheEntry, Comparison)> {
    let identity = q.key.at_bucket(0);
    let eff = q.source.stack_oop.min(q.source.stack_ip);
    let mut candidates = Vec::new();
    for cell in cells {
        for e in cell.entries {
            if crate::entry::validate_entry(&e).is_err() || e.key.at_bucket(0) != identity {
                continue;
            }
            if !e.covered_paths.contains(&q.requested) || !e.nodes.iter().any(|n| n.path == q.requested && n.actor == q.actor) {
                continue;
            }
            let Some(c) = compare(&e, &q.tree, q.source.pot, eff, q.source.cap_mchips) else { continue };
            candidates.push((e, c));
        }
    }
    candidates.sort_by(|(ea, ca), (eb, cb)| ca.rank(cb).then(ea.exploitability_over_P.total_cmp(&eb.exploitability_over_P)));
    // The payload-digest tie-break, computed once per candidate and only among the leading group
    // tied on both earlier keys (the digest serializes a whole entry).
    let (first, first_comparison) = candidates.first()?;
    let tied = candidates
        .iter()
        .take_while(|(e, c)| c.rank(first_comparison).is_eq() && e.exploitability_over_P.total_cmp(&first.exploitability_over_P).is_eq())
        .count();
    let best = if tied > 1 { (0..tied).min_by_key(|&i| payload_digest(&candidates[i].0))? } else { 0 };
    Some(candidates.swap_remove(best))
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
