use postflop_solver::*;

#[test]
fn river_tree_root_menu_matches_r8() {
    let bet = BetSizeOptions { bet: vec![BetSize::PotRelative(0.33), BetSize::PotRelative(0.75), BetSize::AllIn], raise: vec![BetSize::PrevBetRelative(2.5)] };
    let cfg = TreeConfig {
        initial_state: BoardState::River, starting_pot: 100, effective_stack: 100, rake_rate: 0.0, rake_cap: 0.0,
        flop_bet_sizes: [bet.clone(), bet.clone()], turn_bet_sizes: [bet.clone(), bet.clone()], river_bet_sizes: [bet.clone(), bet],
        turn_donk_sizes: Some(DonkSizeOptions { donk: vec![] }), river_donk_sizes: Some(DonkSizeOptions { donk: vec![] }),
        add_allin_threshold: 1.5, force_allin_threshold: 0.0, merging_threshold: 0.0,
    };
    let tree = ActionTree::new(cfg).unwrap();
    assert_eq!(format!("{:?}", tree.available_actions()), "[Check, Bet(33), Bet(75), AllIn(100)]");
}
