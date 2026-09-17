//! Spec 3.6 V1: the FLOP-FAST comparator of R8 section 3.3 (GNU, AVX2, 24 threads: 6.2 s to 0.5 %).
//! `#[ignore]` so the ordinary smoke run stays fast; the V1 step runs it explicitly with `--ignored`.
use postflop_solver::*;
use std::time::Instant;

#[test]
#[ignore]
fn flop_fast_time_to_target() {
    let bet = BetSizeOptions::try_from(("52%", "2.5x")).unwrap();
    let cfg = TreeConfig {
        initial_state: BoardState::Flop, starting_pot: 180, effective_stack: 910, rake_rate: 0.0, rake_cap: 0.0,
        flop_bet_sizes: [bet.clone(), bet.clone()], turn_bet_sizes: [bet.clone(), bet.clone()], river_bet_sizes: [bet.clone(), bet],
        turn_donk_sizes: None, river_donk_sizes: None,
        add_allin_threshold: 1.0, force_allin_threshold: 0.15, merging_threshold: 0.1,
    };
    let cards = CardConfig {
        range: ["66+,A8s+,A5s-A4s,AJo+,K9s+,KQo,QTs+,JTs,96s+,85s+,75s+,65s,54s".parse().unwrap(),
                "QQ-22,AQs-A2s,ATo+,K5s+,KJo+,Q8s+,J8s+,T7s+,96s+,86s+,75s+,64s+,53s+".parse().unwrap()],
        flop: flop_from_str("QsJh2h").unwrap(), turn: NOT_DEALT, river: NOT_DEALT,
    };
    let mut game = PostFlopGame::with_config(cards, ActionTree::new(cfg).unwrap()).unwrap();
    game.allocate_memory(false);
    let target = 180.0 * 0.005;                       // R8's 0.5 % of the pot, in chips
    let t0 = Instant::now();
    let (mut iters, mut expl) = (0u32, f32::INFINITY);
    while iters < 1000 {
        solve_step(&game, iters);
        iters += 1;
        if iters % 10 == 0 {
            expl = compute_exploitability(&game);
            if expl <= target { break; }
        }
    }
    let secs = t0.elapsed().as_secs_f64();
    println!("V1_FLOP_FAST secs={secs:.3} iterations={iters} exploitability_chips={expl:.4} \
              memory_bytes={}", game.memory_usage().0);
    assert!(expl <= target, "did not reach 0.5 % in 1000 iterations (reached {expl:.4} chips)");
}
