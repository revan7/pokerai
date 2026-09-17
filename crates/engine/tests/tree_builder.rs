use engine::tree::{build_effective_tree, build_tree_full, materialize_at, node_at, resolve_chip_path, tree_signature, TemplateSelection, Templates};
use proto::worker::{validate_solution, NodeStrategy, StreetSolution};
use proto::{Action, MaterializedNode, Seat, Street, StreetRootSnapshot, UnsupportedReason};

fn snap(pot: u32, oop_stack: u32, ip_stack: u32, history: Vec<(Seat, Action)>) -> StreetRootSnapshot {
    StreetRootSnapshot { street: Street::Flop, board: vec![proto::Card::parse("Kh").unwrap(), proto::Card::parse("7d").unwrap(), proto::Card::parse("2c").unwrap()],
        oop: Seat(2), ip: Seat(0), pot_root: pot, stack_oop_root: oop_stack, stack_ip_root: ip_stack, dead_this_street: 0, projected_from: 2, history, bb_chips: 2 }
}
fn bet(to: u32) -> Action { Action::Bet { to } }

#[test]
fn tree_builder_golden() {
    // in-tree detection: 50 into 100 equals the flop_fast_v1 menu size; nothing inserted
    let t = build_tree_full(&snap(100, 500, 700, vec![(Seat(2), bet(50))]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(50))])).unwrap();
    assert!(t.tree.inserted.is_empty());
    assert_eq!((t.pot, t.eff, &t.history[..], &t.decision_path[..]), (100, 500, &[bet(50)][..], &[1u8][..]));
    // insertion alongside menus, duplicates merged, requested node after 73
    let sel = TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(73))]);
    let a = build_tree_full(&snap(100, 500, 700, vec![(Seat(2), bet(73))]), &sel).unwrap();
    assert_eq!(node_at(&a.tree.materialized, &[]).unwrap().actions, vec![Action::Check, bet(50), bet(73)]);
    assert_eq!(a.tree.inserted, vec![(vec![], "oop".to_string(), bet(73))]);
    assert_eq!(a.decision_path, vec![2]);
    // duplicates merged (1): an observed size that lands exactly on a menu size inserts nothing and leaves one entry
    let dup = build_tree_full(&snap(100, 500, 700, vec![(Seat(2), bet(50))]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(50))])).unwrap();
    let root: &MaterializedNode = node_at(&dup.tree.materialized, &[]).unwrap();
    assert!(dup.tree.inserted.is_empty());
    assert_eq!(root.actions, vec![Action::Check, bet(50)]);
    assert_eq!(root.actions.iter().filter(|x| **x == bet(50)).count(), 1);
    // duplicates merged (2): at eff 340 facing_test_v1's menu raise 250 is force-collapsed onto the all-in that the
    // add-threshold also lists; the clamp-force-dedupe order leaves exactly one AllIn(340) and no Raise
    let coll = build_tree_full(&snap(100, 340, 340, vec![(Seat(2), bet(100))]), &TemplateSelection::from_history("facing_test_v1", &[(Seat(2), bet(100))])).unwrap();
    let ip = node_at(&coll.tree.materialized, &[1]).unwrap();
    assert_eq!(ip.actions, vec![Action::Fold, Action::Call, Action::AllIn { to: 340 }]);
    assert_eq!(ip.actions.iter().filter(|x| matches!(x, Action::AllIn { .. })).count(), 1);
    // signature stability across chip scales: 100/500 + 73 versus 200/1000 + 146 (same reduced rational 73/100)
    let b = build_tree_full(&snap(200, 1000, 1400, vec![(Seat(2), bet(146))]), &TemplateSelection::from_history("flop_fast_v1", &[(Seat(2), bet(146))])).unwrap();
    assert_eq!(tree_signature(&a.tree, 100), tree_signature(&b.tree, 200));
    assert_ne!(tree_signature(&a.tree, 100), tree_signature(&t.tree, 100));
    // The materialized lists agree node by node with doubled chips: same node count, same paths,
    // streets, actors, menu shape, action kinds and terminal-versus-continuation markers, and every
    // amount within one chip of the doubled one. §4.6 pins `round` to half away from zero, which is
    // not scale-invariant, so exact doubling is unattainable and is not asserted: `round(2.5 * 73)`
    // is 183 where `round(2.5 * 146)` is 365 (183 * 2 = 366), and the same happens at the 2.5x raise
    // off the menu bet (313 * 2 = 626 versus 625) and at the all-ins those raises lead to
    // (187 * 2 = 374 versus 375). A one-chip wager difference moves a matched terminal pot by two
    // chips (1452 versus 1450), because a pot counts both players' contributions.
    assert_eq!(a.tree.materialized.len(), b.tree.materialized.len());
    let mut scaled_exactly = 0;
    let mut off_by_a_chip = 0;
    for (x, y) in a.tree.materialized.iter().zip(&b.tree.materialized) {
        assert_eq!((&x.path, x.street, &x.actor), (&y.path, y.street, &y.actor));
        assert_eq!(x.terminal_pots.len(), y.terminal_pots.len(), "at {:?}", x.path);
        for (p, q) in x.terminal_pots.iter().zip(&y.terminal_pots) {
            match (p, q) {
                (Some(u), Some(v)) => assert!((*u as i64 * 2 - *v as i64).abs() <= 2, "terminal pot at {:?}: {u} * 2 versus {v}", x.path),
                // a continuation must stay a continuation and a terminal a terminal
                _ => assert_eq!(p, q, "terminal marker at {:?}", x.path),
            }
        }
        assert_eq!(x.actions.len(), y.actions.len(), "at {:?}", x.path);
        for (p, q) in x.actions.iter().zip(&y.actions) {
            match (p, q) {
                (Action::Bet { to: u }, Action::Bet { to: v }) | (Action::Raise { to: u }, Action::Raise { to: v }) | (Action::AllIn { to: u }, Action::AllIn { to: v }) => {
                    let d = *u as i64 * 2 - *v as i64;
                    assert!(d.abs() <= 1, "amount at {:?}: {p:?} doubles to {} but the scaled tree has {q:?}", x.path, u * 2);
                    if d == 0 { scaled_exactly += 1 } else { off_by_a_chip += 1 }
                }
                _ => assert_eq!(p, q, "action kind at {:?}", x.path),
            }
        }
    }
    // the tolerance above is the exception, not the rule: the overwhelming majority of amounts double exactly
    assert_eq!((scaled_exactly, off_by_a_chip), (118, 23));
    // facing-node all-in of §4.6 through the snapshot path: eff 350 and 400 list it, 401 does not
    let ip_menu = |eff: u32| { let s = snap(100, eff, eff, vec![(Seat(2), bet(100))]); node_at(&build_effective_tree(&s, &TemplateSelection::from_history("facing_test_v1", &s.history)).unwrap().materialized, &[1]).unwrap().actions.clone() };
    assert!(ip_menu(350).contains(&Action::AllIn { to: 350 }) && ip_menu(400).contains(&Action::AllIn { to: 400 }));
    assert!(!ip_menu(401).iter().any(|x| matches!(x, Action::AllIn { .. })));
    // chip paths of a worker result resolve to ordinal paths; an unresolvable path is "invalid solution"
    let zero = |n: usize| vec![vec![0.0f32; n]; 1326];
    let node = |path: Vec<Action>, actor: &str, actions: Vec<Action>| NodeStrategy { path, actor: actor.into(), probs: zero(actions.len()), ev_chips: zero(actions.len()), actions, available: vec![false; 1326] };
    let sol = StreetSolution { nodes: vec![node(vec![bet(73)], "ip", vec![Action::Fold, Action::Call, Action::Raise { to: 183 }]), node(vec![], "oop", vec![Action::Check, bet(50), bet(73)])],
        requested: 0, exploitability_chips: 0.1, iterations: 10, memory_bytes: 1, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![vec![bet(73)], vec![]] };
    assert_eq!(validate_solution(&sol, &a.tree.materialized).unwrap(), vec![vec![2u8], vec![]]);
    assert_eq!(resolve_chip_path(&a.tree.materialized, &[bet(60)]), None);
    let mut bad = sol.clone(); bad.nodes[0].path = vec![bet(60)]; bad.covered_paths[0] = vec![bet(60)];
    assert!(validate_solution(&bad, &a.tree.materialized).is_err());
    // UnsupportedHistory: a raise below the minimum, a wrong seat, a wrong root street. Each case is
    // pinned by its reason as well as its variant, so one cause cannot stand in for another (the
    // snapshot's history is what is walked here: these selections carry no history of their own).
    let low_raise = build_tree_full(&snap(100, 500, 500, vec![(Seat(2), bet(30)), (Seat(0), Action::Raise { to: 40 })]), &TemplateSelection::from_history("flop_fast_v1", &[]));
    assert!(matches!(&low_raise, Err(UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("below the tree minimum 60")), "{low_raise:?}");
    let wrong_seat = build_tree_full(&snap(100, 500, 500, vec![(Seat(4), bet(30))]), &TemplateSelection::from_history("flop_fast_v1", &[]));
    assert!(matches!(&wrong_seat, Err(UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("seat 4 is not a street-root player")), "{wrong_seat:?}");
    let wrong_street = build_tree_full(&snap(100, 500, 500, vec![]), &TemplateSelection::from_history("turn_std_v1", &[]));
    assert!(matches!(&wrong_street, Err(UnsupportedReason::EngineError { message, .. }) if message.contains("rooted at Turn")), "{wrong_street:?}");
    // materialize_at for bench/tools and the golden file
    let m = materialize_at(Templates::get("river_oracle_v1").unwrap(), 100, 100, &[(0, Action::Check)]).unwrap();
    golden::check("tree_builder_golden", &serde_json::json!({ "river_oracle": m.tree.materialized, "flop_fast_73": a.tree.materialized }));
}

mod golden {
    /// The golden file is a committed artifact, so a missing one fails the test instead of being
    /// recorded silently on the spot (standing ruling on required artifacts): a fresh checkout that
    /// lost the fixture would otherwise "pass" against whatever the code produces that day. Recording
    /// is explicit, `UPDATE_GOLDEN=1`, and the recorded file is hand-checked before it is committed.
    pub fn check(name: &str, value: &serde_json::Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.json"));
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("golden {name} is missing at {}: {e}; record it with UPDATE_GOLDEN=1 and hand-check it before committing", path.display()));
        let stored: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(&stored, value, "golden {name} differs; rerun with UPDATE_GOLDEN=1 after hand-checking");
    }
}
