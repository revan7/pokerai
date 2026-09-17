//! Shared test fixtures for `crates/cache/tests/entry.rs` (plan 4 task 2 brief, steps 5/5a).

/// A single-populated-combo pair of 1326-row matrices plus an `available` mask, for tests that
/// only need one legal combo to exercise `normalize`/`validate_solution` shape checks without
/// building a full range. `actions` is the row width (the node's menu size).
pub fn rows(actions: usize) -> (Vec<Vec<f32>>, Vec<Vec<f32>>, Vec<bool>) {
    let mut p = vec![vec![0.0; actions]; 1326];
    let mut ev = p.clone();
    let mut a = vec![false; 1326];
    // AhAd index: Card(50), Card(49); fixture board Kh7d2c does not block them.
    let i = 50 * 49 / 2 + 49;
    a[i] = true;
    p[i].fill(1.0 / actions as f32);
    for j in 0..actions {
        ev[i][j] = j as f32 * 10.0;
    }
    (p, ev, a)
}

/// A complete, valid `CacheEntry`: a check/jam skeleton with six check-line nodes across three
/// streets (flop/turn/river), each with its own jam response node, of which only the four flop
/// nodes are exported into the entry (root street is flop). Storage tests (task 8+) import this
/// helper via `mod support` rather than depend on a later task to build one.
pub fn entry() -> cache::entry::CacheEntry {
    use cache::entry::{CacheEntry, SourceInputs};
    use cache::key::{KeyFields, Model, RakeKey, Rational};
    use proto::{Action, EffectiveTree, MaterializedNode, MenuSize, PlayerMenus, SideMenu, Street};

    let original = vec![proto::Card(46), proto::Card(21), proto::Card(0)];
    let r = core_ranges::parse_range("AA").unwrap();
    let (_, perm) = core_iso::canonicalize(&original, &[&r, &r]);
    let mut board = original.iter().map(|c| core_iso::apply(&perm, *c)).collect::<Vec<_>>();
    board.sort_by_key(|c| c.0);
    let r = core_iso::apply_range(&perm, &r);
    let hash = core_ranges::hash_scaled(&r);

    let mut materialized = Vec::new();
    for n in 0..6 {
        let street = [Street::Flop, Street::Turn, Street::River][n / 2];
        let actor = if n % 2 == 0 { "oop" } else { "ip" };
        let other = if n % 2 == 0 { "ip" } else { "oop" };
        materialized.push(MaterializedNode {
            path: vec![0; n],
            street,
            actor: actor.into(),
            actions: vec![Action::Check, Action::AllIn { to: 500 }],
            terminal_pots: vec![if n == 5 { Some(100) } else { None }, None],
        });
        let mut facing = vec![0; n];
        facing.push(1);
        materialized.push(MaterializedNode {
            path: facing,
            street,
            actor: other.into(),
            actions: vec![Action::Fold, Action::Call],
            terminal_pots: vec![Some(100), Some(1100)],
        });
    }
    materialized.sort_by(|a, b| a.path.cmp(&b.path));

    let side = SideMenu { bet: vec![MenuSize::AllIn], raise: vec![MenuSize::AllIn] };
    // review m12 / Plan 2 `spec()`: donk is None on the root street, an explicit empty list after it.
    let menus = [Street::Flop, Street::Turn, Street::River]
        .into_iter()
        .map(|street| {
            (
                street,
                PlayerMenus { oop: side.clone(), ip: side.clone(), donk: if street == Street::Flop { None } else { Some(vec![]) } },
            )
        })
        .collect();
    let tree = EffectiveTree {
        rules_version: 3,
        template_id: "check_jam_test_v1".into(),
        root_street: Street::Flop,
        menus,
        add_allin_threshold: 0.0,
        force_allin_threshold: 0.0,
        merging_threshold: 0.0,
        wager_cap: 1,
        inserted: vec![],
        materialized,
    };

    let nodes = tree
        .materialized
        .iter()
        .filter(|n| n.street == Street::Flop)
        .enumerate()
        .map(|(k, n)| {
            let (mut probs, mut ev_chips, mut available) = rows(2);
            for i in 0..1326 {
                available[i] = r.0[i] > 0.0;
                probs[i] = if available[i] { vec![0.5, 0.5] } else { vec![0.0, 0.0] };
                ev_chips[i] = if available[i] { vec![0.0, 10.0] } else { vec![0.0, 0.0] };
            }
            if matches!(n.actions[0], Action::Fold) {
                for row in &mut ev_chips {
                    row[0] = 0.0;
                }
            }
            for (i, row) in ev_chips.iter_mut().enumerate() {
                if available[i] {
                    row[1] += k as f32;
                }
            }
            proto::worker::NodeStrategy {
                path: cache::entry::chip_path(&tree.materialized, &n.path).unwrap(),
                actor: n.actor.clone(),
                actions: n.actions.clone(),
                probs,
                ev_chips,
                available,
            }
        })
        .collect::<Vec<_>>();

    let solution = proto::worker::StreetSolution {
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        requested: 0,
        exploitability_chips: 0.4,
        iterations: 100,
        memory_bytes: 1024,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
    };
    let nodes = cache::entry::normalize(&solution, &tree, 100).unwrap();

    let fractions = tree
        .materialized
        .iter()
        .map(|n| {
            n.actions
                .iter()
                .map(|a| match a {
                    Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => Some(Rational::new(*to as u64, 100).unwrap()),
                    _ => None,
                })
                .collect()
        })
        .collect();

    CacheEntry {
        key: KeyFields {
            schema_version: 3,
            solver_commit: proto::worker::SOLVER_COMMIT.into(),
            adapter_version: 1,
            rules_version: 3,
            canonical_board: board,
            root_street: Street::Flop,
            spr_bucket: 81,
            tree_signature: "check_jam_test_v1".into(),
            rake: RakeKey::new(0.05, Rational::new(5000, 100_000).unwrap(), 1).unwrap(),
            range_hash_oop: hash,
            range_hash_ip: hash,
            model: Model::Baseline,
        },
        source: SourceInputs {
            pot: 100,
            stack_oop: 500,
            stack_ip: 500,
            spr: Rational::new(5, 1).unwrap(),
            bb_chips: 2,
            quantum_over_p: Rational::new(1, 100).unwrap(),
            cap_mchips: 5000,
            ranges: [r.clone(), r],
        },
        tree,
        fractions,
        covered_paths: nodes.iter().map(|n| n.path.clone()).collect(),
        nodes,
        exploitability_over_P: 0.004,
        target_bp: 50,
        iterations: 100,
        elapsed_ms: 10,
        memory_bytes: 1024,
        mode: "f32".into(),
        locks_applied: 0,
        export: "street".into(),
        reasons: vec![],
        created: 1,
        last_hit: 1,
    }
}
