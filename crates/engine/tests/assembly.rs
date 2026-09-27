use engine::assemble::*;
use proto::worker::NodeStrategy;
use proto::*;

fn ctx() -> AssemblyCtx {
    let id = DecisionIdentity { hand_id: 1, hand_revision: 1, decision_id: 1, config_revision: 1, model_revision: 0 };
    AssemblyCtx { identity: id, legal: vec![LegalAction::Fold, LegalAction::Call { cost: 50 }, LegalAction::Raise { min_to: 100, max_to: 500 }], hero_combo: Some(combo_index(Card::parse("Ah").unwrap(), Card::parse("Ad").unwrap())), bb_chips: 10, equity: engine::equity::pending_summary(&[Seat(0)]) }
}
fn node(rows: &[(usize, [f32; 3], [f32; 3])]) -> NodeStrategy {
    let mut n = NodeStrategy { path: vec![Action::Bet { to: 50 }], actor: "ip".into(), actions: vec![Action::Fold, Action::Call, Action::Raise { to: 150 }], probs: vec![vec![0.0; 3]; 1326], ev_chips: vec![vec![0.0; 3]; 1326], available: vec![false; 1326] };
    for (c, p, e) in rows { n.probs[*c] = p.to_vec(); n.ev_chips[*c] = e.to_vec(); n.available[*c] = true; }
    n
}
fn adv(a: Action, f: Option<f32>, ev: Option<f32>, u: Option<Unavailable>) -> ActionAdvice { ActionAdvice { action: a, frequency: f, ev_bb: ev, unavailable: u, headline: false } }
fn who(actions: &[ActionAdvice]) -> Option<Action> { actions.iter().find(|a| a.headline).map(|a| a.action.clone()) }

#[test]
fn recommendation_assembly_golden() {
    let c = ctx();
    let hero = c.hero_combo.unwrap() as usize;
    let mut record = serde_json::Map::new();
    // (1) headline only with complete EVs: highest EV, fold = 0
    let n = node(&[(hero, [0.2, 0.3, 0.5], [0.0, 12.5, 30.0])]);
    let reach = vec![1.0f32; 1326];
    let rec = final_from_solution(&c, &n, &reach, Coverage::Exact, empty_assumptions("river_std_v1"));
    assert_eq!(who(&rec.actions), Some(Action::Raise { to: 150 }));
    assert!(rec.assumptions.notes.iter().any(|s| s == "headline: highest EV"));
    assert_eq!((rec.actions[1].ev_bb, rec.actions[0].ev_bb, rec.unresolved_mass), (Some(1.25), Some(0.0), 0.0));
    record.insert("complete_ev".into(), serde_json::to_value(&rec.actions).unwrap());
    // tie break: equal EV -> higher frequency, then earlier in menu order
    let mut t = vec![adv(Action::Fold, Some(0.2), Some(0.0), None), adv(Action::Call, Some(0.3), Some(3.0), None), adv(Action::Raise { to: 150 }, Some(0.5), Some(3.0), None)];
    assert_eq!(headline(&mut t, 0.0, HeadlineSource::Solved).as_deref(), Some("highest EV"));
    assert_eq!(who(&t), Some(Action::Raise { to: 150 }));
    t[1].frequency = Some(0.5);
    headline(&mut t, 0.0, HeadlineSource::Solved);
    assert_eq!(who(&t), Some(Action::Call));
    // (2) no EV headline when one action lacks EV; frequency headline wording per source
    let mut u = vec![adv(Action::Fold, Some(0.1), Some(0.0), None), adv(Action::Call, Some(0.6), None, Some(Unavailable::NoEvReference)), adv(Action::Raise { to: 150 }, Some(0.3), Some(9.0), None)];
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::Solved), None);
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::Chart).as_deref(), Some("highest-frequency chart action"));
    assert_eq!(who(&u), Some(Action::Call));
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::PokerDataUnverified).as_deref(), Some("highest-frequency source action, EV reference unverified"));
    u[1].unavailable = Some(Unavailable::BranchSupportIncomplete { covered_posterior: 0.2 });
    assert_eq!(headline(&mut u, 0.0, HeadlineSource::PokerDataUnverified).as_deref(), Some("highest-frequency action, EV incomplete"));
    // (3) no headline of any kind when unresolved_mass > 0
    let mut v = vec![adv(Action::Fold, Some(0.3), Some(0.0), None), adv(Action::Call, Some(0.5), Some(2.0), None)];
    assert_eq!(headline(&mut v, 0.2, HeadlineSource::Chart), None);
    assert!(v.iter().all(|a| !a.headline));
    // reason accumulation: never removed, Exact upgrades to Approximate, Unsupported keeps partial
    let acc = accumulate(Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] }, vec![ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 }]);
    assert!(matches!(&acc, Coverage::Approximate { reasons } if reasons.len() == 2));
    assert!(matches!(accumulate(acc.clone(), vec![]), Coverage::Approximate { .. }));
    assert!(matches!(accumulate(Coverage::Exact, vec![]), Coverage::Exact));
    assert!(matches!(coverage_for_solve(0.3, 100, 50, false, vec![]), Coverage::Exact));
    assert!(matches!(coverage_for_solve(1.9, 100, 50, true, vec![]), Coverage::Approximate { reasons } if reasons == vec![ApproxReason::DeadlineBestSoFar { reached_bp: 190, target_bp: 50 }]));
    let un = unsupported(&c, UnsupportedReason::MultiwayEv { pot_eligible: 3 }, vec![ApproxReason::ChartRounded], empty_assumptions(""));
    assert!(matches!(&un.coverage, Coverage::Unsupported { partial, .. } if partial == &vec![ApproxReason::ChartRounded]));
    assert!(un.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none()) && un.actions.len() == 3);
    // range_mix present under HeroComboOutOfSupport; frequencies absent
    let other = combo_index(Card::parse("Kh").unwrap(), Card::parse("Kd").unwrap()) as usize;
    let n2 = node(&[(other, [0.0, 1.0, 0.0], [0.0, 5.0, 0.0])]);
    let rec2 = final_from_solution(&c, &n2, &reach, Coverage::Exact, empty_assumptions("river_std_v1"));
    assert!(matches!(rec2.coverage, Coverage::Unsupported { reason: UnsupportedReason::HeroComboOutOfSupport, .. }));
    assert_eq!(rec2.range_mix.as_ref().unwrap()[1], (Action::Call, 1.0));
    assert!(rec2.actions.iter().all(|a| a.frequency.is_none() && matches!(a.unavailable, Some(Unavailable::HeroOutOfSupport))));
    // an Equity event after Final enriches it; Pending never replaces Ready
    let mut fin = rec.clone();
    let ready = EquitySummary { hero_combo_vs_each: vec![(Seat(0), EquityEstimate { value: Some(0.61), availability: Availability::Ready, method: Some(EquityMethod::Exact) })], hero_range_vs_each: vec![(Seat(0), EquityEstimate { value: None, availability: Availability::Pending, method: None })], per_pot_shares: vec![] };
    merge_equity(&mut fin, &ready);
    assert_eq!(fin.equity.hero_combo_vs_each[0].1.value, Some(0.61));
    merge_equity(&mut fin, &engine::equity::pending_summary(&[Seat(0)]));
    assert_eq!(fin.equity.hero_combo_vs_each[0].1.value, Some(0.61));
    record.insert("out_of_support".into(), serde_json::to_value(&rec2).unwrap());
    golden::check("recommendation_assembly_golden", &serde_json::Value::Object(record));
}

// --- Beyond the golden: the rest of the assembly surface ------------------------------------------

fn rows(pairs: &[(usize, &[f32])], width: usize) -> Vec<Vec<f32>> {
    let mut m = vec![vec![0.0; width]; 1326];
    for (c, p) in pairs { m[*c] = p.to_vec(); }
    m
}
fn tree_node(actor: &str, actions: Vec<Action>, probs: Vec<Vec<f32>>) -> NodeStrategy {
    let available = probs.iter().map(|r| r.iter().any(|x| *x > 0.0)).collect();
    let ev_chips = vec![vec![0.0; actions.len()]; 1326];
    NodeStrategy { path: vec![], actor: actor.into(), actions, probs, ev_chips, available }
}

#[test]
fn hero_reach_multiplies_only_hero_actions_on_the_path() {
    let (a, b) = (5usize, 9usize);
    let cb = || vec![Action::Check, Action::Bet { to: 50 }];
    // [] villain, [0] hero (the path goes through its bet), [0,1] villain, [0,1,2] hero (requested),
    // [1] a hero node off the path that must not be applied.
    let nodes = vec![
        tree_node("oop", cb(), rows(&[(a, &[0.5, 0.5]), (b, &[1.0, 0.0])], 2)),
        tree_node("ip", cb(), rows(&[(a, &[0.25, 0.75]), (b, &[0.6, 0.4])], 2)),
        tree_node("oop", vec![Action::Fold, Action::Call, Action::Raise { to: 150 }], rows(&[(a, &[0.0, 0.0, 1.0]), (b, &[0.5, 0.0, 0.5])], 3)),
        tree_node("ip", vec![Action::Fold, Action::Call], rows(&[(a, &[0.5, 0.5]), (b, &[0.5, 0.5])], 2)),
        tree_node("ip", cb(), rows(&[(a, &[1.0, 0.0]), (b, &[1.0, 0.0])], 2)),
    ];
    let paths: Vec<OrdinalPath> = vec![vec![], vec![0], vec![0, 1], vec![0, 1, 2], vec![1]];
    let mut public = Range1326::zero();
    public.set(a as ComboIndex, 1.0);
    public.set(b as ComboIndex, 0.5);
    let reach = hero_reach(&nodes, &paths, 3, &public, "ip");
    assert_eq!(reach.len(), 1326);
    assert_eq!((reach[a], reach[b]), (0.75, 0.2));
    assert!(reach.iter().enumerate().all(|(c, r)| c == a || c == b || *r == 0.0));
}

#[test]
fn reasons_are_deduplicated_and_never_removed() {
    let merged = accumulate(Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] }, vec![ApproxReason::ChartRounded, ApproxReason::EvReferenceUnverified, ApproxReason::ChartRounded]);
    assert_eq!(merged, Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded, ApproxReason::EvReferenceUnverified] });
    // a later solve at target never removes an inherited reason
    assert_eq!(coverage_for_solve(0.3, 100, 50, false, vec![ApproxReason::ChartRounded]), Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] });
    // Unsupported keeps its accumulated reasons in `partial`, deduplicated
    let un = unsupported(&ctx(), UnsupportedReason::InvalidRanges, vec![ApproxReason::ChartRounded, ApproxReason::ChartRounded], empty_assumptions(""));
    assert_eq!(un.coverage, Coverage::Unsupported { reason: UnsupportedReason::InvalidRanges, partial: vec![ApproxReason::ChartRounded] });
    assert_eq!(accumulate(un.coverage, vec![ApproxReason::UnconditionedCurrentStreet]), Coverage::Unsupported { reason: UnsupportedReason::InvalidRanges, partial: vec![ApproxReason::ChartRounded, ApproxReason::UnconditionedCurrentStreet] });
}

#[test]
fn coverage_for_solve_compares_raw_chips_at_the_boundary() {
    // exactly at target: 0.5 chips of a 100-chip pot is 50 bp
    assert_eq!(coverage_for_solve(0.5, 100, 50, false, vec![]), Coverage::Exact);
    // just above target, `ok` terminal: not Exact, and no reason is invented
    assert_eq!(coverage_for_solve(0.5001, 100, 50, false, vec![]), Coverage::Approximate { reasons: vec![] });
    // just above target at the deadline: 50.01 bp is displayed as 50 bp, the comparison stays raw
    assert_eq!(coverage_for_solve(0.5001, 100, 50, true, vec![]), Coverage::Approximate { reasons: vec![ApproxReason::DeadlineBestSoFar { reached_bp: 50, target_bp: 50 }] });
}

#[test]
fn fast_carries_the_legal_menu_and_nothing_numeric() {
    let f = fast(&ctx(), Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] }, empty_assumptions("river_std_v1"));
    assert_eq!(f.phase, Phase::Fast);
    assert_eq!(f.legal, ctx().legal);
    assert_eq!(f.actions.iter().map(|a| a.action).collect::<Vec<_>>(), vec![Action::Fold, Action::Call, Action::Raise { to: 100 }]);
    assert!(f.actions.iter().all(|a| a.frequency.is_none() && a.ev_bb.is_none() && !a.headline && a.unavailable == Some(Unavailable::Pending)));
    assert_eq!((f.range_mix.as_ref(), f.unresolved_mass), (None, 0.0));
    assert_eq!(f.equity, engine::equity::pending_summary(&[Seat(0)]));
    assert_eq!(f.coverage, Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] });
}

#[test]
fn map_to_legal_uses_the_intervals_and_the_real_all_in() {
    let legal = [LegalAction::Check, LegalAction::Bet { min_to: 20, max_to: 480 }, LegalAction::AllIn { to: 480 }];
    assert_eq!(map_to_legal(&Action::Check, &legal), Some(Action::Check));
    assert_eq!(map_to_legal(&Action::Bet { to: 20 }, &legal), Some(Action::Bet { to: 20 }));
    assert_eq!(map_to_legal(&Action::Bet { to: 480 }, &legal), Some(Action::Bet { to: 480 }));
    assert_eq!(map_to_legal(&Action::Bet { to: 19 }, &legal), None);
    assert_eq!(map_to_legal(&Action::Bet { to: 481 }, &legal), None);
    assert_eq!(map_to_legal(&Action::AllIn { to: 470 }, &legal), Some(Action::AllIn { to: 480 }));
    assert_eq!(map_to_legal(&Action::Fold, &legal), None);
    assert_eq!(map_to_legal(&Action::Call, &legal), None);
    assert_eq!(map_to_legal(&Action::Raise { to: 100 }, &legal), None);
}

#[test]
fn a_tree_action_outside_the_legal_intervals_keeps_its_frequency_without_ev() {
    let c = ctx();
    let hero = c.hero_combo.unwrap() as usize;
    let mut n = node(&[(hero, [0.2, 0.3, 0.5], [0.0, 12.5, 30.0])]);
    n.actions[2] = Action::Raise { to: 600 };
    let rec = final_from_solution(&c, &n, &vec![1.0f32; 1326], Coverage::Exact, empty_assumptions("river_std_v1"));
    assert_eq!(rec.actions[2], ActionAdvice { action: Action::Raise { to: 600 }, frequency: Some(0.5), ev_bb: None, unavailable: Some(Unavailable::NotEvaluated), headline: false });
    assert!(rec.actions.iter().all(|a| !a.headline), "no EV headline over an incomplete EV set, no frequency headline for a solve");
    assert!(rec.assumptions.notes.iter().any(|s| s.contains("Raise { to: 600 }")), "{:?}", rec.assumptions.notes);
}

#[test]
fn no_range_mix_when_hero_range_has_no_reach_at_the_node() {
    let c = ctx();
    let other = combo_index(Card::parse("Kh").unwrap(), Card::parse("Kd").unwrap()) as usize;
    let n = node(&[(other, [0.0, 1.0, 0.0], [0.0, 5.0, 0.0])]);
    let rec = final_from_solution(&c, &n, &vec![0.0f32; 1326], Coverage::Approximate { reasons: vec![ApproxReason::ChartRounded] }, empty_assumptions("river_std_v1"));
    assert_eq!(rec.coverage, Coverage::Unsupported { reason: UnsupportedReason::HeroComboOutOfSupport, partial: vec![ApproxReason::ChartRounded] });
    assert_eq!(rec.range_mix, None);
    assert!(rec.assumptions.notes.iter().any(|s| s.contains("no reach")), "{:?}", rec.assumptions.notes);
}

#[test]
fn equity_merge_never_unsettles_an_estimate() {
    let c = ctx();
    let mut rec = unsupported(&c, UnsupportedReason::InvalidRanges, vec![], empty_assumptions(""));
    let est = |availability: Availability, value: Option<f32>| EquityEstimate { method: value.map(|_| EquityMethod::Exact), value, availability };
    let gone = Availability::Unavailable { reason: "equity budget exceeded".into() };
    let summary = |combo: EquityEstimate, range: EquityEstimate| EquitySummary { hero_combo_vs_each: vec![(Seat(0), combo)], hero_range_vs_each: vec![(Seat(0), range)], per_pot_shares: vec![] };
    merge_equity(&mut rec, &summary(est(Availability::Ready, Some(0.4)), est(gone.clone(), None)));
    // neither a later Pending nor a later Unavailable replaces Ready; Pending never resets Unavailable
    merge_equity(&mut rec, &summary(est(gone.clone(), None), est(Availability::Pending, None)));
    assert_eq!(rec.equity.hero_combo_vs_each, vec![(Seat(0), est(Availability::Ready, Some(0.4)))]);
    assert_eq!(rec.equity.hero_range_vs_each, vec![(Seat(0), est(gone.clone(), None))]);
    // a seat not shown yet is added
    merge_equity(&mut rec, &EquitySummary { hero_combo_vs_each: vec![(Seat(3), est(Availability::Ready, Some(0.7)))], hero_range_vs_each: vec![], per_pot_shares: vec![] });
    assert_eq!(rec.equity.hero_combo_vs_each.len(), 2);
}

#[test]
#[should_panic(expected = "action 1")]
fn headline_refuses_a_non_finite_ev() {
    let mut t = vec![adv(Action::Fold, Some(0.5), Some(0.0), None), adv(Action::Call, Some(0.5), Some(f32::NAN), None)];
    headline(&mut t, 0.0, HeadlineSource::Solved);
}

mod golden {
    /// The golden file is a committed artifact (same rule as `tree_builder.rs`): a missing file fails the test
    /// instead of being recorded silently on the spot, recording is explicit (`UPDATE_GOLDEN=1`, then a hand
    /// check before committing), and a mismatch names the case that differs.
    pub fn check(name: &str, value: &serde_json::Value) {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden").join(format!("{name}.json"));
        if std::env::var("UPDATE_GOLDEN").is_ok() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, serde_json::to_string_pretty(value).unwrap()).unwrap();
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("golden {name} is missing at {}: {e}; record it with UPDATE_GOLDEN=1 and hand-check it before committing", path.display()));
        let stored: serde_json::Value = serde_json::from_str(&text).unwrap();
        if let (Some(s), Some(v)) = (stored.as_object(), value.as_object()) {
            for case in s.keys().chain(v.keys()) {
                assert_eq!(s.get(case), v.get(case), "golden {name}: case {case:?} differs (stored left, produced right)");
            }
        }
        assert_eq!(&stored, value, "golden {name} differs");
    }
}
