//! P3.T11 -- shared history-branch Bayesian updates (spec sections 8.4, 9.1 and 9.2; the section
//! 13.1 rows `replay_bayes_two_combos` (T3), `replay_off_tree_pseudo_harmonic` and
//! `replay_cross_actor_branches` (T6)).
//!
//! The three worked-example tests below are the task brief's, verbatim. The tests after them
//! cover the rest of the same section 13.1 rows and the brief's step 5 list: the same examples
//! with a rescale after every action and the rounded spec figures, the translated child
//! histories, zero-M children, frozen branches, the positive-output boundary, and the
//! out-of-domain inputs that are errors rather than clamped values.

fn close(a:f64,b:f64){assert!((a-b).abs()<1e-10,"{a} != {b}");}
fn two_combos()->Vec<core_replay::HistoryBranch>{
    let mut b=core_preflop::branches::initial(&[proto::Seat(0),proto::Seat(1)]);
    for s in &mut b[0].seats { s.mass.fill(0.0);s.mass[0]=1.;s.mass[1]=1.; }
    b
}
#[test]
fn replay_bayes_two_combos(){
    use core_preflop::branches::*;
    let mut b=two_combos(); let mut logs=vec![0.;6];
    for (pair,m) in [([0.8,0.2],0.5),([0.25,1.],0.4),([0.9,0.1],0.5)] {
        let mut p=vec![0.;1326]; p[..2].copy_from_slice(&pair);
        let old=b[0].q;
        b=vec![condition(&b[0],proto::Seat(0),&p,1.).unwrap()];
        close(b[0].q/old,m);
        if m==0.5 && pair[0]==0.9 {
            close(b[0].seats[0].mass[0],9.);close(b[0].seats[0].mass[1],1.);
        }
        rescale(&mut b,&mut logs);
    }
    close(b[0].q,0.1); close(logs[0],0.18_f64.ln());
    let r=marginal(&b,proto::Seat(0));close(r[0],1.);close(r[1],1./9.);
    close(r[0]/(r[0]+r[1]),0.9);close(r[1]/(r[0]+r[1]),0.1);
    assert_eq!(posterior(&b,proto::Seat(1),0),vec![1.]);
    let h=marginal(&b,proto::Seat(1));close(h[0],h[1]);
}

#[test]
fn replay_off_tree_pseudo_harmonic(){
    use core_preflop::branches::*;
    let v=proto::Seat(0);let b=two_combos();
    let f=81.0/173.0;let g=1.0-f;
    let likelihood=|a:f64,b:f64|{let mut p=vec![0.0;1326];p[0]=a;p[1]=b;p};
    let mut bs=vec![condition(&b[0],v,&likelihood(0.9,0.3),f).unwrap(),
                    condition(&b[0],v,&likelihood(0.1,0.5),g).unwrap()];
    close(bs[0].q,f*0.6);close(bs[1].q,g*0.3);
    close(bs[0].seats[0].mass[0],1.5);close(bs[0].seats[0].mass[1],0.5);
    close(bs[1].seats[0].mass[0],1.0/3.0);close(bs[1].seats[0].mass[1],5.0/3.0);
    let first=marginal(&bs,v);close(first[0],f*0.9+g*0.1);close(first[1],f*0.3+g*0.5);
    let mut logs=vec![0.0;6];rescale(&mut bs,&mut logs);
    let qa=bs[0].q;let qb=bs[1].q;
    bs[0]=condition(&bs[0],v,&likelihood(0.5,1.0),1.0).unwrap();
    bs[1]=condition(&bs[1],v,&likelihood(0.2,0.8),1.0).unwrap();
    close(bs[0].q/qa,0.625);close(bs[1].q/qb,0.7);
    let unscaled=[f*0.9*0.5+g*0.1*0.2,f*0.3+g*0.5*0.8];
    rescale(&mut bs,&mut logs);
    let r=marginal(&bs,v);close(r[0],unscaled[0]/unscaled[1]);close(r[1],1.0);
    close(logs[0],unscaled[1].ln());
    close(posterior(&bs,v,0)[0],f*0.9*0.5/unscaled[0]);
    close(posterior(&bs,v,1)[0],f*0.3/unscaled[1]);
}

#[test]
fn replay_cross_actor_branches(){
    use core_preflop::branches::*;
    let b=two_combos();let v=proto::Seat(0);let h=proto::Seat(1);
    let mut pa=vec![0.;1326];pa[..2].copy_from_slice(&[0.8,0.1]);
    let mut pb=vec![0.;1326];pb[..2].copy_from_slice(&[0.1,0.4]);
    let mut children=vec![condition(&b[0],v,&pa,0.6).unwrap(),condition(&b[0],v,&pb,0.4).unwrap()];
    close(children[0].q,0.27);close(children[1].q,0.10);
    close(marginal(&children,v)[0],0.52);close(marginal(&children,v)[1],0.22);
    close(posterior(&children,h,0)[0],0.27/0.37);
    children[0]=condition(&children[0],v,&vec![0.9;1326],1.).unwrap();
    children[1]=condition(&children[1],v,&vec![0.1;1326],1.).unwrap();
    close(children[0].q,0.243);close(children[1].q,0.010);
    close(marginal(&children,v)[0],0.436);close(marginal(&children,v)[1],0.070);
    close(posterior(&children,h,0)[0],0.243/0.253);
    assert_eq!(children[0].seats[1].mass,b[0].seats[1].mass);
}

// ---------------------------------------------------------------------------------------------
// Beyond the brief's verbatim tests: the rest of the section 13.1 rows and the brief's step 5.
// ---------------------------------------------------------------------------------------------

use core_replay::{
    cap_branches, condition, marginal, posterior, range_output, rescale, residual_reason, split_action, HistoryBranch, SeatMass,
};
use proto::{Action, ApproxReason, Seat};

/// Rounded spec figures (section 13.1 quotes them to 3-4 decimals).
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 5e-4, "{a} is not within 5e-4 of {b}");
}

/// A likelihood with `a` on combo 0, `b` on combo 1 and zero elsewhere (the two-combo support).
fn pair(a: f64, b: f64) -> Vec<f64> {
    let mut p = vec![0.0; 1326];
    p[0] = a;
    p[1] = b;
    p
}

/// Hero's next preflop node key in `branch`, derived from that branch's own translated history
/// with core-preflop's real key functions (P3.T13's `query_translated` does this against a live
/// store; the table here is fixed: 100 bb, no rake profile, no straddle, villain = Seat(0) in UTG,
/// hero = Seat(1) in HJ, one source unit = 100 chips).
fn next_key(branch: &HistoryBranch) -> String {
    let role = |s: Seat| if s == Seat(0) { proto::Position::Utg } else { proto::Position::Hj };
    core_preflop::node_key(&core_preflop::PreflopNodeKey {
        depth_bb: 100,
        rake_profile: "none".into(),
        straddle: false,
        history: branch.translated.iter().map(|(s, a)| (role(*s), core_preflop::to_source_step(a, 100))).collect(),
    })
}

/// T3's remaining clauses: an inserted observed bet conditions with its solved probability
/// (forcing its likelihood to 1 would leave the posterior unchanged, so that shortcut fails), and a
/// combo at relative reach 1e-30 stays positive through `condition`, `rescale` and the output.
#[test]
fn replay_bayes_inserted_bet_and_tiny_reach() {
    let v = Seat(0);
    let h = Seat(1);
    let b = two_combos();
    let share = |bs: &[HistoryBranch]| {
        let r = marginal(bs, v);
        r[0] / (r[0] + r[1])
    };
    let bet = condition(&b[0], v, &pair(0.9, 0.1), 1.).unwrap();
    let forced = condition(&b[0], v, &vec![1.0; 1326], 1.).unwrap();
    close(share(std::slice::from_ref(&bet)), 0.9);
    close(share(std::slice::from_ref(&forced)), 0.5);
    assert_eq!(forced.q, 1.0);
    assert_eq!(forced.seats[0].mass, b[0].seats[0].mass);

    let mut bs = vec![condition(&b[0], v, &pair(1e-30, 1.0), 1.).unwrap()];
    let mut logs = vec![0.0; 6];
    rescale(&mut bs, &mut logs);
    let r = marginal(&bs, v);
    close(r[1], 1.0);
    assert!(r[0] > 0.0);
    close(r[0] / 1e-30, 1.0);
    let out = range_output(&r);
    assert!(out.0[0] >= f32::MIN_POSITIVE);
    assert!((f64::from(out.0[0]) / 1e-30 - 1.0).abs() < 1e-6);
    assert_eq!(out.0[1], 1.0);
    assert!(out.0[2..].iter().all(|w| w.to_bits() == 0));

    let deeper = vec![condition(&b[0], v, &pair(1e-50, 1.0), 1.).unwrap()];
    let r = marginal(&deeper, v);
    assert!(r[0] > 0.0 && r[0] < f64::from(f32::MIN_POSITIVE));
    assert_eq!(range_output(&r).0[0], f32::MIN_POSITIVE);
    // The unacted seat is untouched by either update.
    assert_eq!(deeper[0].seats[1].mass, b[0].seats[1].mass);
    assert_eq!(posterior(&deeper, h, 0), vec![1.0]);
}

/// The output boundary: 1e-30 stays a positive `f32`, anything positive below
/// `f32::MIN_POSITIVE` (1e-50, a would-be subnormal) becomes `f32::MIN_POSITIVE`, zero of either
/// sign becomes `+0.0`, and a rescaled maximum that drifted above 1 by rounding narrows to 1. The
/// result is a valid wire range, and the clamp keeps the hash seeing the supported combo.
#[test]
fn range_output_preserves_positive_reach() {
    let mut r = vec![0.0; 1326];
    r[0] = 1e-30;
    r[1] = 1e-50;
    r[2] = 0.0;
    r[3] = -0.0;
    r[4] = 1.0;
    r[5] = f64::from(f32::MIN_POSITIVE) / 2.0;
    r[6] = 1.0 + 1e-13;
    let out = range_output(&r);
    assert!(out.0[0] > f32::MIN_POSITIVE);
    assert!((f64::from(out.0[0]) / 1e-30 - 1.0).abs() < 1e-6);
    assert_eq!(out.0[1], f32::MIN_POSITIVE);
    assert_eq!(out.0[2].to_bits(), 0);
    assert_eq!(out.0[3].to_bits(), 0);
    assert_eq!(out.0[4], 1.0);
    assert_eq!(out.0[5], f32::MIN_POSITIVE);
    assert_eq!(out.0[6], 1.0);
    assert!(out.0[7..].iter().all(|w| w.to_bits() == 0));
    assert!(serde_json::to_string(&out).is_ok());
    let mut dropped = r.clone();
    dropped[1] = 0.0;
    assert_ne!(core_ranges::hash_scaled(&out), core_ranges::hash_scaled(&range_output(&dropped)));
}

/// `replay_off_tree_pseudo_harmonic` again, from P3.T10's interpolation choices (73 into 100
/// against sizes 50/100) and with every rounded section 13.1 figure asserted at 5e-4 next to the
/// exact hand-computed formula at 1e-10.
#[test]
fn replay_off_tree_pseudo_harmonic_spec_figures() {
    let v = Seat(0);
    let h = Seat(1);
    let b = two_combos();
    let menu = [Action::Bet { to: 50 }, Action::Bet { to: 100 }];
    let s = core_preflop::wager_fraction(73, 0, 0, 100).unwrap();
    let t = core_preflop::interpolate(s, &core_preflop::menu_fractions(&menu, 0, 0, 100).unwrap()).unwrap();
    assert_eq!(t.choices.iter().map(|c| c.0).collect::<Vec<_>>(), vec![0, 1]);
    let (fa, fb) = (t.choices[0].1, t.choices[1].1);
    close(fa, 81.0 / 173.0);
    close(fb, 92.0 / 173.0);
    near(fa, 0.468);
    near(fb, 0.532);
    close(t.deviation, 0.23);
    assert!(t.deviation > 0.10, "d = 0.23 is prominent");

    let likelihoods = [pair(0.9, 0.3), pair(0.1, 0.5)];
    let mut bs: Vec<HistoryBranch> = t
        .choices
        .iter()
        .map(|&(i, f)| {
            let mut child = condition(&b[0], v, &likelihoods[i], f).unwrap();
            child.translated.push((v, menu[i]));
            child
        })
        .collect();
    close(bs[0].q / fa, 0.6);
    close(bs[1].q / fb, 0.3);
    close(bs[0].q, 0.28092485549);
    close(bs[1].q, 0.15953757225);
    near(bs[0].q, 0.2809);
    near(bs[1].q, 0.1595);
    near(bs[1].seats[0].mass[0], 0.3333);
    near(bs[1].seats[0].mass[1], 1.6667);
    let first = marginal(&bs, v);
    close(first[0], 0.47456647399);
    close(first[1], 0.40635838150);
    near(first[0], 0.4746);
    near(first[1], 0.4064);
    let (pi0, pi1) = (posterior(&bs, v, 0), posterior(&bs, v, 1));
    close(pi0[0], fa * 0.9 / (fa * 0.9 + fb * 0.1));
    near(pi0[0], 0.888);
    near(pi1[0], 0.346);
    close(pi0.iter().sum(), 1.0);
    // Hero has not acted: masses copied into both children, posterior q_k / sum q for every combo.
    assert_eq!(bs[0].seats[1].mass, b[0].seats[1].mass);
    assert_eq!(bs[1].seats[1].mass, b[0].seats[1].mass);
    close(posterior(&bs, h, 0)[0], bs[0].q / (bs[0].q + bs[1].q));
    assert_eq!(posterior(&bs, h, 0), posterior(&bs, h, 1));

    let mut logs = vec![0.0; 6];
    rescale(&mut bs, &mut logs);
    close(logs[0], first[0].ln());
    close(logs[1], (bs[0].q + bs[1].q).ln());
    let (qa, qb) = (bs[0].q, bs[1].q);
    bs[0] = condition(&bs[0], v, &pair(0.5, 1.0), 1.0).unwrap();
    bs[1] = condition(&bs[1], v, &pair(0.2, 0.8), 1.0).unwrap();
    close(bs[0].q / qa, 0.625);
    close(bs[1].q / qb, 0.7);
    near(bs[0].q, 0.1756);
    near(bs[1].q, 0.1117);
    let unscaled: Vec<f64> = marginal(&bs, v)[..2].iter().map(|r| r * first[0]).collect();
    close(unscaled[0], fa * 0.9 * 0.5 + fb * 0.1 * 0.2);
    close(unscaled[1], fa * 0.3 * 1.0 + fb * 0.5 * 0.8);
    near(unscaled[0], 0.2213);
    near(unscaled[1], 0.3532);
    rescale(&mut bs, &mut logs);
    let out = range_output(&marginal(&bs, v));
    near(f64::from(out.0[0]), 0.6267);
    assert_eq!(out.0[1], 1.0);
    close(logs[0], unscaled[1].ln());
    near(logs[0], 0.3532_f64.ln());
    near(posterior(&bs, v, 0)[0], 0.952);
    near(posterior(&bs, v, 1)[0], 0.398);
    assert_eq!(bs[0].translated, vec![(v, Action::Bet { to: 50 })]);
    assert_eq!(bs[1].translated, vec![(v, Action::Bet { to: 100 })]);
}

/// T6 with a rescale after every action (section 13.1 figures), the translated child histories
/// kept distinct as raise A / raise B, and hero's next key following each branch's own history.
#[test]
fn replay_cross_actor_branches_rescaled() {
    let b = two_combos();
    let v = Seat(0);
    let h = Seat(1);
    let raises = [Action::Raise { to: 250 }, Action::Raise { to: 400 }];
    let mut children = vec![
        condition(&b[0], v, &pair(0.8, 0.1), 0.6).unwrap(),
        condition(&b[0], v, &pair(0.1, 0.4), 0.4).unwrap(),
    ];
    for (child, raise) in children.iter_mut().zip(raises) {
        assert!(child.translated.is_empty() && child.seats.iter().all(|s| s.node.is_none()));
        child.translated.push((v, raise));
        child.seats[1].node = Some(next_key(child));
    }
    let key = |to_bb_x1000: u32| {
        core_preflop::node_key(&core_preflop::PreflopNodeKey {
            depth_bb: 100,
            rake_profile: "none".into(),
            straddle: false,
            history: vec![(proto::Position::Utg, core_preflop::PreflopStep::Raise { to_bb_x1000 })],
        })
    };
    let (key_a, key_b) = (key(2500), key(4000));
    assert_ne!(key_a, key_b);
    assert_eq!(children[0].seats[1].node.as_deref(), Some(key_a.as_str()));
    assert_eq!(children[1].seats[1].node.as_deref(), Some(key_b.as_str()));

    close(children[0].seats[0].mass[0], 16.0 / 9.0);
    close(children[0].seats[0].mass[1], 2.0 / 9.0);
    close(children[1].seats[0].mass[0], 0.4);
    close(children[1].seats[0].mass[1], 1.6);
    near(children[0].seats[0].mass[0], 1.7778);
    near(children[0].seats[0].mass[1], 0.2222);
    assert_eq!(children[0].seats[1].mass, children[1].seats[1].mass);

    let mut logs = vec![0.0; 6];
    rescale(&mut children, &mut logs);
    close(logs[0], 0.52_f64.ln());
    close(logs[1], 0.37_f64.ln());
    let villain = range_output(&marginal(&children, v));
    assert_eq!(villain.0[0], 1.0);
    near(f64::from(villain.0[1]), 0.4231);
    close(marginal(&children, v)[1], 0.22 / 0.52);
    let hero = range_output(&marginal(&children, h));
    assert_eq!((hero.0[0], hero.0[1]), (1.0, 1.0));
    let (c1, c2) = (posterior(&children, v, 0), posterior(&children, v, 1));
    close(c1[0], 12.0 / 13.0);
    close(c2[0], 3.0 / 11.0);
    near(c1[0], 0.923);
    near(c1[1], 0.077);
    near(c2[0], 0.273);
    near(c2[1], 0.727);
    for combo in [0, 1] {
        let pi = posterior(&children, h, combo);
        close(pi[0], 0.27 / 0.37);
        near(pi[0], 0.7297);
        near(pi[1], 0.2703);
    }

    children[0] = condition(&children[0], v, &vec![0.9; 1326], 1.).unwrap();
    children[1] = condition(&children[1], v, &vec![0.1; 1326], 1.).unwrap();
    close(children[0].q, 0.243);
    close(children[1].q, 0.010);
    rescale(&mut children, &mut logs);
    close(logs[0], 0.436_f64.ln());
    close(logs[1], 0.253_f64.ln());
    close(marginal(&children, v)[1], 0.070 / 0.436);
    for combo in [0, 1] {
        let pi = posterior(&children, h, combo);
        close(pi[0], 0.243 / 0.253);
        near(pi[0], 0.9605);
        near(pi[1], 0.0395);
    }
    // Villain's masses keep their shape; hero's stay uniform and identical in both branches; the
    // later action advanced nothing, so each branch still names its own raise and hero's key.
    close(children[0].seats[0].mass[0] / children[0].seats[0].mass[1], 8.0);
    close(children[1].seats[0].mass[0] / children[1].seats[0].mass[1], 0.25);
    assert_eq!(children[0].seats[1].mass, children[1].seats[1].mass);
    assert_eq!(children[0].seats[1].mass[0], children[0].seats[1].mass[1]);
    assert_eq!(children[0].translated, vec![(v, raises[0])]);
    assert_eq!(children[1].translated, vec![(v, raises[1])]);
    assert_eq!(children[0].seats[1].node.as_deref(), Some(key_a.as_str()));
    assert_eq!(children[1].seats[1].node.as_deref(), Some(key_b.as_str()));
}

/// A child whose integrated likelihood is zero is not created, so it disappears for every seat;
/// both children zero is the zero-support case (no child at all, the input branch untouched).
#[test]
fn replay_zero_m_child_disappears_for_every_seat() {
    let b = two_combos();
    let v = Seat(0);
    let h = Seat(1);
    let mut off_support = vec![0.7; 1326];
    off_support[0] = 0.0;
    off_support[1] = 0.0;
    let children: Vec<HistoryBranch> =
        [condition(&b[0], v, &pair(0.8, 0.1), 0.6), condition(&b[0], v, &off_support, 0.4)].into_iter().flatten().collect();
    assert_eq!(children.len(), 1);
    close(children[0].q, 0.27);
    assert_eq!(posterior(&children, v, 0), vec![1.0]);
    assert_eq!(posterior(&children, h, 0), vec![1.0]);
    close(marginal(&children, h)[0], 0.27);
    close(marginal(&children, v)[1], 0.27 * (2.0 / 9.0));

    let none: Vec<HistoryBranch> =
        [condition(&b[0], v, &off_support, 0.6), condition(&b[0], v, &off_support, 0.4)].into_iter().flatten().collect();
    assert!(none.is_empty());
    assert!(condition(&b[0], v, &pair(0.8, 0.1), 0.0).is_none());
    assert!(condition(&b[0], Seat(4), &pair(0.8, 0.1), 1.0).is_none());
    assert_eq!(b[0].q, 1.0);
    assert_eq!(b[0].seats[0].mass, two_combos()[0].seats[0].mass);
}

/// Residual and stopped branches are frozen: `condition` returns them bit-identical whatever the
/// likelihood (even an impossible action), so they never add support. The seat-common rescale
/// still divides their masses by the same factor as every live branch and never touches `q`, so
/// no posterior moves.
#[test]
fn frozen_branches_are_not_conditioned_but_share_the_rescale() {
    let b = two_combos();
    let v = Seat(0);
    let h = Seat(1);
    let mut bs: Vec<HistoryBranch> = [0.5, 0.25, 0.25]
        .iter()
        .enumerate()
        .map(|(k, &f)| {
            let mut child = condition(&b[0], v, &vec![1.0; 1326], f).unwrap();
            child.id = k as u8;
            child
        })
        .collect();
    bs[1].residual = true;
    bs[2].stopped = Some("missing node UTG_2500".into());
    bs[0] = condition(&bs[0], v, &pair(0.8, 0.2), 1.0).unwrap();
    close(bs[0].q, 0.25);

    let impossible = vec![0.0; 1326];
    for k in [1, 2] {
        let same = condition(&bs[k], v, &impossible, 1.0).unwrap();
        assert_eq!(same.q.to_bits(), bs[k].q.to_bits());
        for (a, z) in same.seats.iter().zip(&bs[k].seats) {
            assert_eq!(a.mass, z.mass);
        }
    }
    assert!(condition(&bs[0], v, &impossible, 1.0).is_none());

    let q: Vec<f64> = bs.iter().map(|b| b.q).collect();
    let cells = [(v, 0), (v, 1), (h, 0), (h, 1)];
    let before: Vec<Vec<f64>> = cells.iter().map(|&(s, c)| posterior(&bs, s, c)).collect();
    let r = marginal(&bs, v);
    close(r[0], 0.25 * 1.6 + 0.5);
    close(r[1], 0.25 * 0.4 + 0.5);
    let residual_mass = bs[1].seats[0].mass.clone();
    let stopped_mass = bs[2].seats[0].mass.clone();
    let mut logs = vec![0.0; 6];
    rescale(&mut bs, &mut logs);
    assert_eq!(bs.iter().map(|b| b.q).collect::<Vec<_>>(), q);
    close(logs[0], 0.9_f64.ln());
    close(logs[1], 0.75_f64.ln()); // hero unacted: marginal = sum_k q_k = 0.25 + 0.25 + 0.25
    close(bs[1].seats[0].mass[0] * 0.9, residual_mass[0]);
    close(bs[2].seats[0].mass[1] * 0.9, stopped_mass[1]);
    let after: Vec<Vec<f64>> = cells.iter().map(|&(s, c)| posterior(&bs, s, c)).collect();
    for (x, y) in before.iter().flatten().zip(after.iter().flatten()) {
        close(*x, *y);
    }
}

// Out-of-domain inputs are errors (always-on assertions naming the index), never clamped.

#[test]
#[should_panic(expected = "condition: likelihood[3] = 1.5 is not a probability in [0, 1]")]
fn condition_rejects_a_likelihood_above_one() {
    let mut p = pair(0.5, 0.5);
    p[3] = 1.5;
    condition(&two_combos()[0], Seat(0), &p, 1.0);
}

#[test]
#[should_panic(expected = "condition: likelihood[7] = NaN is not a probability in [0, 1]")]
fn condition_rejects_a_nan_likelihood() {
    let mut p = pair(0.5, 0.5);
    p[7] = f64::NAN;
    condition(&two_combos()[0], Seat(0), &p, 1.0);
}

#[test]
#[should_panic(expected = "condition: factor 1.2 is not a probability in [0, 1]")]
fn condition_rejects_a_factor_above_one() {
    condition(&two_combos()[0], Seat(0), &pair(0.5, 0.5), 1.2);
}

#[test]
#[should_panic(expected = "condition: likelihood has 2 entries, expected 1326")]
fn condition_rejects_a_short_likelihood() {
    condition(&two_combos()[0], Seat(0), &[0.5, 0.5], 1.0);
}

#[test]
#[should_panic(expected = "marginal: branch 0 seat Seat(0) mass[5] = NaN is not a finite non-negative value")]
fn rescale_rejects_a_nan_mass() {
    // R2 fix: the individual bad mass is now caught by `marginal`'s own per-branch validation
    // (called from `rescale`) before the aggregate marginal-entry check below it ever runs --
    // validating mass inputs before an aggregate check that could hide the invalid component.
    let mut b = two_combos();
    b[0].seats[0].mass[5] = f64::NAN;
    rescale(&mut b, &mut [0.0; 6]);
}

#[test]
#[should_panic(expected = "range_output: marginal[9] = 1.5 is not a finite weight in [0, 1]")]
fn range_output_rejects_a_weight_above_one() {
    let mut r = vec![0.0; 1326];
    r[9] = 1.5;
    range_output(&r);
}

#[test]
#[should_panic(expected = "range_output: marginal[2] = -0.001 is not a finite weight in [0, 1]")]
fn range_output_rejects_a_negative_weight() {
    let mut r = vec![0.0; 1326];
    r[2] = -1e-3;
    range_output(&r);
}

// ---------------------------------------------------------------------------------------------
// Fix round 1 (Codex review task-11-review.md, orchestrator ruling): R1 scaled accumulation so a
// representable positive support/marginal is never lost to underflow, decided from the mass and
// likelihood operands' positive/zero status rather than the computed sum, with a named assertion
// when a result truly cannot be represented; R2 branch-weight validation shared by every reader of
// a branch weight, including frozen branches and the public aggregation/rescale paths.
// ---------------------------------------------------------------------------------------------

/// R1: a single branch whose mass and likelihood are each representable on their own, but whose
/// exact product (`1e-300 * 1e-30 = 1e-330`) is smaller than the smallest representable positive
/// `f64` (`~4.94e-324`), is genuinely unrepresentable support, not impossible support: `condition`
/// must say so with a named assertion, never silently report `None`.
#[test]
#[should_panic(expected = "integrated support underflowed to 0 despite a positive mass and likelihood")]
fn positive_overlap_is_not_zero_support() {
    let mut b = core_preflop::branches::initial(&[Seat(0), Seat(1)]);
    b[0].seats[0].mass.fill(0.0);
    b[0].seats[0].mass[0] = 1e-300;
    let mut p = vec![0.0; 1326];
    p[0] = 1e-30;
    condition(&b[0], Seat(0), &p, 1.0);
}

/// R1 (the `condition` analogue of the four-branch marginal case below): four combos whose mass
/// and likelihood individually underflow to zero when multiplied in isolation, but whose exact sum
/// is the smallest representable positive `f64`, must come back positive, not `None`.
#[test]
fn condition_support_keeps_a_representable_positive_sum() {
    let tiny = f64::from_bits(1);
    let mut b = core_preflop::branches::initial(&[Seat(0), Seat(1)]);
    b[0].seats[0].mass.fill(0.0);
    for c in 0..4 {
        b[0].seats[0].mass[c] = 0.25;
    }
    let mut p = vec![0.0; 1326];
    for c in 0..4 {
        p[c] = tiny;
    }
    let child = condition(&b[0], Seat(0), &p, 1.0).expect("a representable positive support must not become None");
    assert!(
        child.q > 0.0 && child.q <= tiny * 2.0 && child.q >= tiny * 0.5,
        "child.q = {} should be within a factor of 2 of {tiny}",
        child.q
    );
}

/// R1 (review's four-branch example): four branches with `q = 0.25`, mass `1` on combo 0 and the
/// smallest representable positive `f64` on combo 1 -- the exact combo-1 marginal is that same
/// value (`sum_k 0.25 * tiny = tiny`), even though every individual `q_k * w_k[1]` term underflows
/// to zero on its own.
#[test]
fn marginal_keeps_a_representable_positive_sum() {
    let tiny = f64::from_bits(1);
    let v = Seat(0);
    let branches: Vec<HistoryBranch> = (0..4u8)
        .map(|id| {
            let mut br = core_preflop::branches::initial(&[Seat(0)]).remove(0);
            br.id = id;
            br.q = 0.25;
            br.seats[0].mass.fill(0.0);
            br.seats[0].mass[0] = 1.0;
            br.seats[0].mass[1] = tiny;
            br
        })
        .collect();
    let r = marginal(&branches, v);
    close(r[0], 1.0);
    assert!(
        r[1] > 0.0 && r[1] <= tiny * 2.0 && r[1] >= tiny * 0.5,
        "r[1] = {} (the exact combo-1 marginal, the smallest positive f64) must not be lost to underflow",
        r[1]
    );
}

/// R2: a frozen (residual) branch's `q` is validated too -- an invalid weight is rejected before
/// the frozen early return would otherwise hand it back unchanged.
#[test]
#[should_panic(expected = "condition: branch 0 weight q = 1.5 is not a finite value in [0, 1]")]
fn frozen_branch_rejects_out_of_domain_weight() {
    let mut b = two_combos();
    b[0].residual = true;
    b[0].q = 1.5;
    condition(&b[0], Seat(0), &vec![0.5; 1326], 1.0);
}

/// R2: `rescale` validates every branch's weight before using it, including one that would
/// otherwise look like it produces a valid unit marginal.
#[test]
#[should_panic(expected = "rescale: branch 0 weight q = 2.5 is not a finite value in [0, 1]")]
fn rescale_rejects_out_of_domain_weight() {
    let mut b = two_combos();
    b[0].q = 2.5;
    rescale(&mut b, &mut vec![0.0; 6]);
}

// ---------------------------------------------------------------------------------------------
// P3.T12 -- the branch cap, its persistent frozen residual, `residual_reason` and the test-only
// `split_action` (spec sections 8.4, 9.1, 9.2; section 13.1 row `replay_branch_cap_residual`).
// ---------------------------------------------------------------------------------------------

/// The task brief's worked example, verbatim: uniform likelihood .5 at all combos, three rounds of
/// splitting every current live branch across one observed wager (f = .6/.4) and capping. Round 3
/// overflows 8 live branches to 4 live + 1 residual (weights `.027,.018,.018,.018,.012,.012,.012,
/// .008`, residual `.044`, total `.125`); a later on-menu `.5` halves only the live total to
/// `.0405` (the residual is frozen, so it does not see the new likelihood); a fourth split then
/// overflows again, extending the same residual by `.0081` to `.0521`.
#[test]
fn replay_branch_cap_residual() {
    use core_preflop::branches::*;
    let mut b = two_combos();
    let actor = proto::Seat(0);
    let choices = vec![
        (proto::Action::Raise { to: 50 }, 0.6, vec![0.5; 1326]),
        (proto::Action::Raise { to: 100 }, 0.4, vec![0.5; 1326]),
    ];
    for _ in 0..3 {
        b = split_action(&b, actor, &choices);
        cap_branches(&mut b);
    }
    assert_eq!(b.iter().filter(|b| !b.residual).count(), 4);
    close(b.iter().find(|b| b.residual).unwrap().q, 0.044);
    close(b.iter().map(|b| b.q).sum(), 0.125);
    b = b.iter().filter_map(|b| condition(b, actor, &vec![0.5; 1326], 1.)).collect();
    close(b.iter().filter(|b| !b.residual).map(|b| b.q).sum(), 0.0405);
    close(b.iter().find(|b| b.residual).unwrap().q, 0.044);
    b = split_action(&b, actor, &choices);
    cap_branches(&mut b);
    close(b.iter().find(|b| b.residual).unwrap().q, 0.0521);
    assert_eq!(b.iter().filter(|b| b.residual).count(), 1);
}

/// `residual_reason` recomputes the share from current weights every call, never caches the
/// original cap's figure: `None` before any cap has produced a residual, `35.2%` right after the
/// cap that built it (`100 * .044 / .125`), and `52.0710059%` (52.1 rounded) after the later
/// on-menu action halves only the live total, matching the brief's prose for the same sequence.
#[test]
fn residual_reason_reports_the_recomputed_share() {
    let actor = Seat(0);
    let hero = Seat(1);
    let mut b = two_combos();
    let choices = vec![(Action::Raise { to: 50 }, 0.6, vec![0.5; 1326]), (Action::Raise { to: 100 }, 0.4, vec![0.5; 1326])];
    assert!(residual_reason(&b, hero).is_none(), "no residual exists before the first overflowing cap");
    for _ in 0..3 {
        b = split_action(&b, actor, &choices);
        cap_branches(&mut b);
    }
    match residual_reason(&b, hero).expect("cap_branches created a residual") {
        ApproxReason::BranchResidual { seat, residual_mass_pct, cause } => {
            assert_eq!(seat, hero);
            assert_eq!(cause, "cap");
            assert!((residual_mass_pct - 35.2).abs() < 0.05, "{residual_mass_pct} != 35.2");
        }
        other => panic!("expected ApproxReason::BranchResidual, got {other:?}"),
    }
    b = b.iter().filter_map(|b| condition(b, actor, &vec![0.5; 1326], 1.)).collect();
    match residual_reason(&b, hero).expect("the residual survives a live-only conditioning step") {
        ApproxReason::BranchResidual { residual_mass_pct, .. } => {
            assert!((residual_mass_pct - 52.0710059).abs() < 0.001, "{residual_mass_pct} != 52.0710059");
        }
        other => panic!("expected ApproxReason::BranchResidual, got {other:?}"),
    }
}

/// `split_action` assigns each parent's children in choice order (child A before child B),
/// stamps `parent`/`split_by`/the appended `translated` entry, and copies a residual or stopped
/// branch through completely unchanged (bit-identical `q` and masses, its own id kept) whatever
/// the menu -- it is never conditioned, matching `condition`'s own frozen-branch rule.
#[test]
fn split_action_wires_parent_split_by_translated_and_freezes_residual_and_stopped() {
    let b = two_combos();
    let actor = Seat(0);
    let choices = vec![(Action::Bet { to: 50 }, 0.6, vec![0.5; 1326]), (Action::Bet { to: 100 }, 0.4, vec![0.5; 1326])];
    let mut bs = split_action(&b, actor, &choices);
    assert_eq!(bs.len(), 2);
    assert_eq!(bs[0].parent, Some(0));
    assert_eq!(bs[1].parent, Some(0));
    assert_eq!(bs[0].split_by, Some(actor));
    assert_eq!(bs[1].split_by, Some(actor));
    assert_eq!(bs[0].translated, vec![(actor, Action::Bet { to: 50 })]);
    assert_eq!(bs[1].translated, vec![(actor, Action::Bet { to: 100 })]);
    assert!(bs[0].id < bs[1].id, "child A (first choice) keeps a lower id than child B");

    bs[0].residual = true;
    bs[1].stopped = Some("missing node X".into());
    let (before_q, before_mass) = (bs[0].q, bs[0].seats[0].mass.clone());
    let (before_stopped_q, before_stopped_mass) = (bs[1].q, bs[1].seats[0].mass.clone());
    let out = split_action(&bs, actor, &choices);
    assert_eq!(out.len(), 2, "a residual and a stopped branch are copied through, never expanded");
    assert_eq!(out[0].id, bs[0].id);
    assert!(out[0].residual);
    assert_eq!(out[0].q, before_q);
    assert_eq!(out[0].seats[0].mass, before_mass);
    assert_eq!(out[1].id, bs[1].id);
    assert_eq!(out[1].stopped, Some("missing node X".into()));
    assert_eq!(out[1].q, before_stopped_q);
    assert_eq!(out[1].seats[0].mass, before_stopped_mass);
}

/// A child whose id counter would exceed `u8::MAX` triggers compaction in creation order (never a
/// silent id collision left in the output), and child order (A before B) survives the compaction.
/// The input generation is mapped first (254 -> 0) and the children follow it (1, 2), so both
/// name their consumed parent as 0 -- an id no output branch holds (fix round 1, R2: the earlier
/// expectation `[0, 1]` came with parent `Some(0)`, i.e. child A named itself as its parent).
#[test]
fn split_action_compacts_ids_past_u8_max() {
    let mut b = two_combos();
    b[0].id = 254;
    let actor = Seat(0);
    let choices = vec![(Action::Raise { to: 50 }, 0.6, vec![0.5; 1326]), (Action::Raise { to: 100 }, 0.4, vec![0.5; 1326])];
    let out = split_action(&b, actor, &choices);
    assert_eq!(out.len(), 2);
    let ids: Vec<u8> = out.iter().map(|c| c.id).collect();
    assert_eq!(ids, vec![1, 2], "254/255 would otherwise collide; compaction must yield unique, ordered ids after the input generation");
    assert_eq!(out.iter().map(|c| c.parent).collect::<Vec<_>>(), vec![Some(0), Some(0)]);
    assert_eq!(out[0].translated.last(), Some(&(actor, Action::Raise { to: 50 })));
    assert_eq!(out[1].translated.last(), Some(&(actor, Action::Raise { to: 100 })));
}

/// The audit `cap_branches` itself must satisfy: every seat's public marginal is unchanged by the
/// cap, combo by combo, at 1e-12 -- including when the merged-away branches have different (not
/// uniform) mass shapes, so the residual that absorbs them is not flat either.
#[test]
fn cap_branches_preserves_marginals_with_nonuniform_masses() {
    let seat = Seat(0);
    let mut bs: Vec<HistoryBranch> = (0..6u8)
        .map(|id| {
            let mut mass = vec![0.0; 1326];
            mass[0] = 1.0 + f64::from(id);
            mass[1] = 2.0 + f64::from(id) * 0.5;
            mass[2] = if id % 2 == 0 { 3.0 } else { 0.0 };
            HistoryBranch {
                id,
                parent: None,
                split_by: None,
                translated: vec![],
                q: 0.30 - f64::from(id) * 0.04,
                residual: false,
                stopped: None,
                seats: vec![SeatMass { seat, node: None, mass }],
            }
        })
        .collect();
    let before = marginal(&bs, seat);
    cap_branches(&mut bs);
    assert_eq!(bs.iter().filter(|b| !b.residual).count(), 4);
    assert_eq!(bs.iter().filter(|b| b.residual).count(), 1);
    let after = marginal(&bs, seat);
    for (c, (a, z)) in before.iter().zip(&after).enumerate() {
        assert!((a - z).abs() <= 1e-12, "combo {c}: marginal moved from {a} to {z} across cap_branches");
    }
    let r = bs.iter().find(|b| b.residual).unwrap();
    assert_ne!(r.seats[0].mass[0], r.seats[0].mass[1], "the merged residual keeps a non-flat shape, not a uniform average");
}

/// A cap that overflows into an *existing* residual (a second overflow event) keeps absorbing
/// mass rather than creating a second one, and the marginal-preservation audit still holds.
#[test]
fn cap_branches_merges_a_second_overflow_into_the_existing_residual() {
    let seat = Seat(0);
    let mut bs: Vec<HistoryBranch> = (0..7u8)
        .map(|id| {
            let mut mass = vec![0.0; 1326];
            mass[0] = 1.0 + f64::from(id);
            mass[1] = 3.0 - f64::from(id) * 0.2;
            HistoryBranch {
                id,
                parent: None,
                split_by: None,
                translated: vec![],
                q: 0.20 - f64::from(id) * 0.02,
                residual: false,
                stopped: None,
                seats: vec![SeatMass { seat, node: None, mass }],
            }
        })
        .collect();
    let before1 = marginal(&bs, seat);
    cap_branches(&mut bs);
    assert_eq!(bs.iter().filter(|b| b.residual).count(), 1);
    assert_eq!(bs.iter().filter(|b| !b.residual).count(), 4);
    let after1 = marginal(&bs, seat);
    for (c, (a, z)) in before1.iter().zip(&after1).enumerate() {
        assert!((a - z).abs() <= 1e-12, "combo {c}: marginal moved from {a} to {z} across the first cap");
    }

    // Add three more low-weight live branches (fresh ids) and cap again: the existing residual
    // must absorb them, never spawn a second one, and the (now larger) marginal is still
    // preserved exactly across this second cap.
    for (k, id) in (10u8..13).enumerate() {
        let mut mass = vec![0.0; 1326];
        mass[0] = 0.5 + k as f64;
        mass[1] = 1.0;
        bs.push(HistoryBranch {
            id,
            parent: None,
            split_by: None,
            translated: vec![],
            q: 0.01 + (k as f64) * 0.001,
            residual: false,
            stopped: None,
            seats: vec![SeatMass { seat, node: None, mass }],
        });
    }
    let before2 = marginal(&bs, seat);
    cap_branches(&mut bs);
    assert_eq!(bs.iter().filter(|b| b.residual).count(), 1, "a second overflow extends the one residual, never a second");
    assert_eq!(bs.iter().filter(|b| !b.residual).count(), 4);
    let after2 = marginal(&bs, seat);
    for (c, (a, z)) in before2.iter().zip(&after2).enumerate() {
        assert!((a - z).abs() <= 1e-12, "combo {c}: marginal moved from {a} to {z} across the second cap");
    }
}

// ---------------------------------------------------------------------------------------------
// P3.T12 fix round 1 (task-12-review.md R1-R3, orchestrator rulings): creation-ordered id
// compaction after a u8 wraparound, a collision-free old-to-new parent remap built from the input
// generation, and an underflow-safe residual merge (normalized weights) that keeps a representable
// positive support alive.
// ---------------------------------------------------------------------------------------------

/// A two-seat branch with uniform masses (weight 1 on all 1326 combos for both seats) and the given
/// identity and weight, tagged with a one-entry translated history `(Seat(1), Raise{to: tag})` so
/// each input history can be traced through `split_action` and `cap_branches`.
fn tagged(id: u8, parent: Option<u8>, q: f64, tag: u32) -> HistoryBranch {
    HistoryBranch {
        id,
        parent,
        split_by: None,
        translated: vec![(Seat(1), Action::Raise { to: tag })],
        q,
        residual: false,
        stopped: None,
        seats: [Seat(0), Seat(1)].iter().map(|&seat| SeatMass { seat, node: None, mass: vec![1.0; 1326] }).collect(),
    }
}

/// `tagged`, marked as the frozen residual (no history of its own).
fn tagged_residual(id: u8, parent: Option<u8>, q: f64) -> HistoryBranch {
    let mut r = tagged(id, parent, q, 0);
    r.residual = true;
    r.translated.clear();
    r
}

/// `tagged`, marked as stopped on the current street.
fn tagged_stopped(id: u8, parent: Option<u8>, q: f64, tag: u32) -> HistoryBranch {
    let mut s = tagged(id, parent, q, tag);
    s.stopped = Some(format!("missing node K{tag}"));
    s
}

/// A two-choice common menu with `f = .5` each and likelihood 1 at every combo: `M = 1`, so every
/// child has exactly half its parent's `q` and its parent's masses.
fn even_split() -> Vec<(Action, f64, Vec<f64>)> {
    vec![(Action::Raise { to: 50 }, 0.5, vec![1.0; 1326]), (Action::Raise { to: 100 }, 0.5, vec![1.0; 1326])]
}

/// R1 (review reproduction plus a residual): live ids 251/252 (`q = .2`) split into four children
/// of `q = .1`, alongside stopped ids 253/254 (`q = .1`) and a residual. The child id counter wraps
/// past `u8::MAX`, so ids are compacted -- and compaction must keep creation order: the input
/// generation is renumbered in its original order first (7, 251, 252, 253, 254 -> 0..4) and every
/// new child sorts after every older retained branch (5..8). All six non-residual weights tie at
/// `.1`, so the cap keeps the two older stopped branches and the earliest two children (251's A
/// then B) and merges 252's two children into the existing residual.
#[test]
fn cap_after_id_wraparound_keeps_older_stopped_ties() {
    let actor = Seat(0);
    let input = vec![
        tagged(251, None, 0.2, 251),
        tagged(252, None, 0.2, 252),
        tagged_stopped(253, Some(240), 0.1, 253),
        tagged_stopped(254, Some(241), 0.1, 254),
        tagged_residual(7, Some(2), 0.05),
    ];
    let mut out = split_action(&input, actor, &even_split());
    assert_eq!(out.len(), 7);
    for b in out.iter().filter(|b| !b.residual) {
        assert_eq!(b.q, 0.1, "every non-residual weight ties at .1");
    }
    assert_eq!(
        out.iter().map(|b| b.id).collect::<Vec<_>>(),
        vec![5, 6, 7, 8, 3, 4, 0],
        "input generation 0..4 in creation order, children 5..8 after it"
    );
    assert_eq!(
        out.iter().map(|b| b.parent).collect::<Vec<_>>(),
        vec![Some(1), Some(1), Some(2), Some(2), None, None, None],
        "children name their parent's compacted id; references outside the input generation (240, 241, 2) resolve to None, never to an output branch"
    );
    let retained_max = out.iter().filter(|b| b.split_by.is_none()).map(|b| b.id).max().unwrap();
    assert!(
        out.iter().filter(|b| b.split_by.is_some()).all(|b| b.id > retained_max),
        "every new child sorts after every older retained branch"
    );

    cap_branches(&mut out);
    let survivors: Vec<(u8, Vec<(Seat, Action)>, Option<String>)> =
        out.iter().filter(|b| !b.residual).map(|b| (b.id, b.translated.clone(), b.stopped.clone())).collect();
    assert_eq!(
        survivors,
        vec![
            (3, vec![(Seat(1), Action::Raise { to: 253 })], Some("missing node K253".to_string())),
            (4, vec![(Seat(1), Action::Raise { to: 254 })], Some("missing node K254".to_string())),
            (5, vec![(Seat(1), Action::Raise { to: 251 }), (actor, Action::Raise { to: 50 })], None),
            (6, vec![(Seat(1), Action::Raise { to: 251 }), (actor, Action::Raise { to: 100 })], None),
        ],
        "the two older stopped branches and the earliest two children survive the equal-q tie"
    );
    let residuals: Vec<&HistoryBranch> = out.iter().filter(|b| b.residual).collect();
    assert_eq!(residuals.len(), 1);
    assert_eq!(residuals[0].id, 0, "the existing residual is extended, never replaced");
    close(residuals[0].q, 0.05 + 0.1 + 0.1);
    close(out.iter().map(|b| b.q).sum(), 0.65);
}

/// R2 shared check: `split_action` over a stopped branch (id 20, stale parent 12), the live
/// parents `live`, whose children overflow the u8 id space, and a residual (id 5, stale parent 1).
/// The old-to-new id map is built from the whole input generation (sorted ids -> 0, 1, 2, ...)
/// before any id is narrowed; children take the ids after it, and every child's `parent` is its
/// parent's mapped id -- a slot no output branch holds, since the parent was consumed by the split
/// -- so no branch is its own parent and no child names a sibling, cousin or frozen branch.
/// References outside the input generation (the frozen branches' stale 12 and 1) become `None`,
/// never an unrelated output branch. Frozen branches keep their flags, weight and masses.
fn check_collision_free_parent_remap(live: &[u8]) {
    let actor = Seat(0);
    let mut input = vec![tagged_stopped(20, Some(12), 0.1, 20)];
    input.extend(live.iter().map(|&id| tagged(id, None, 0.2, u32::from(id))));
    input.push(tagged_residual(5, Some(1), 0.05));
    let out = split_action(&input, actor, &even_split());

    let mut old: Vec<u8> = input.iter().map(|b| b.id).collect();
    old.sort_unstable();
    let rank = |id: u8| u8::try_from(old.iter().position(|&o| o == id).unwrap()).unwrap();
    let base = u8::try_from(old.len()).unwrap();

    let ids: Vec<u8> = out.iter().map(|b| b.id).collect();
    let mut unique = ids.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), ids.len(), "live {live:?}: output ids {ids:?} collide");
    for b in &out {
        assert_ne!(b.parent, Some(b.id), "live {live:?}: branch {} is its own parent", b.id);
    }

    let menu = even_split();
    let children: Vec<&HistoryBranch> = out.iter().filter(|b| b.split_by.is_some()).collect();
    assert_eq!(children.len(), 2 * live.len());
    for (k, child) in children.iter().enumerate() {
        let parent_old = live[k / 2];
        assert_eq!(child.id, base + u8::try_from(k).unwrap(), "live {live:?}: child {k} id");
        assert_eq!(child.parent, Some(rank(parent_old)), "live {live:?}: child {k} parent");
        assert!(!ids.contains(&rank(parent_old)), "live {live:?}: child {k}'s parent id resolves to an output branch");
        assert_eq!(child.translated, vec![(Seat(1), Action::Raise { to: u32::from(parent_old) }), (actor, menu[k % 2].0)]);
    }
    for (a, z) in children.iter().zip(children.iter().skip(1)) {
        assert_eq!(a.parent == z.parent, a.translated[0] == z.translated[0], "live {live:?}: siblings share one parent id, cousins never do");
    }

    let stopped = out.iter().find(|b| b.stopped.is_some()).expect("the stopped branch is copied through");
    assert_eq!((stopped.id, stopped.parent, stopped.q), (rank(20), None, 0.1));
    assert_eq!(stopped.translated, vec![(Seat(1), Action::Raise { to: 20 })]);
    assert!(stopped.seats.iter().all(|s| s.mass == vec![1.0; 1326]));
    let residual = out.iter().find(|b| b.residual).expect("the residual is copied through");
    assert_eq!((residual.id, residual.parent, residual.q), (rank(5), None, 0.05));
    assert!(residual.seats.iter().all(|s| s.mass == vec![1.0; 1326]));
}

/// R2 seeded at 254: one live parent at 254 (children would take 255 and 256) and two live
/// parents 253/254.
#[test]
fn split_action_remaps_parents_collision_free_seeded_at_254() {
    check_collision_free_parent_remap(&[254]);
    check_collision_free_parent_remap(&[253, 254]);
}

/// R2 seeded at 255: the review's reproduction (one live parent at 255, whose two children used to
/// saturate to 255 and compact to a self-parent and a sibling alias) plus two live parents 254/255.
#[test]
fn split_action_remaps_parents_collision_free_seeded_at_255() {
    check_collision_free_parent_remap(&[255]);
    check_collision_free_parent_remap(&[254, 255]);
}

/// The old-to-new id map must be collision-free, so the input generation's ids must be distinct.
#[test]
#[should_panic(expected = "split_action: branch id 3 appears more than once in the input generation")]
fn split_action_rejects_duplicate_input_ids() {
    let input = vec![tagged(3, None, 0.2, 1), tagged_stopped(3, None, 0.1, 2)];
    split_action(&input, Seat(0), &even_split());
}

/// R3 (review reproduction): six branches, three seats each, every seat's masses totalling 1326.
/// The four retained branches (`q = .1`) have no combo-0 mass; the two overflow branches have the
/// smallest positive `f64` weight and the only combo-0 support (mass .5), so the pre-cap combo-0
/// marginal is the representable `5e-324`. The merge must use normalized weights so neither
/// `q * mass` product rounds to zero first: the residual keeps mass .5 on combo 0 (1.5 on combo 1,
/// 1 elsewhere) for every seat, every marginal is unchanged, combo 0 stays positive and the output
/// boundary still reports it as `f32::MIN_POSITIVE`.
#[test]
fn cap_branches_keeps_support_that_exists_only_in_overflow() {
    let tiny = f64::from_bits(1);
    let seats = [Seat(0), Seat(1), Seat(2)];
    let branch = |id: u8, q: f64, m0: f64, m1: f64| {
        let mut mass = vec![1.0; 1326];
        mass[0] = m0;
        mass[1] = m1;
        HistoryBranch {
            id,
            parent: None,
            split_by: None,
            translated: vec![],
            q,
            residual: false,
            stopped: None,
            seats: seats.iter().map(|&seat| SeatMass { seat, node: None, mass: mass.clone() }).collect(),
        }
    };
    let mut bs: Vec<HistoryBranch> = (0..4u8).map(|id| branch(id, 0.1, 0.0, 2.0)).collect();
    bs.push(branch(4, tiny, 0.5, 1.5));
    bs.push(branch(5, tiny, 0.5, 1.5));
    for b in &bs {
        for s in &b.seats {
            assert_eq!(s.mass.iter().sum::<f64>(), 1326.0, "equal per-seat totals");
        }
    }
    let before: Vec<Vec<f64>> = seats.iter().map(|&s| marginal(&bs, s)).collect();
    for r in &before {
        assert_eq!(r[0], tiny, "the pre-cap combo-0 marginal is the representable 5e-324");
    }

    cap_branches(&mut bs);
    assert_eq!(bs.iter().filter(|b| !b.residual).count(), 4);
    let residual = bs.iter().find(|b| b.residual).expect("the overflow forms the residual");
    assert_eq!(residual.q, 2.0 * tiny);
    for (k, &seat) in seats.iter().enumerate() {
        let s = &residual.seats[k];
        assert_eq!(s.seat, seat);
        assert_eq!(s.mass[0], 0.5, "seat {seat:?}: residual combo-0 mass");
        assert_eq!(s.mass[1], 1.5, "seat {seat:?}: residual combo-1 mass");
        assert!(s.mass[2..].iter().all(|&w| w == 1.0), "seat {seat:?}: residual masses elsewhere");
        let after = marginal(&bs, seat);
        assert!(after[0] > 0.0, "seat {seat:?}: combo 0 lost its only (overflow) support");
        assert_eq!(after[0], before[k][0], "seat {seat:?}: combo-0 marginal");
        for (c, (a, z)) in before[k].iter().zip(&after).enumerate() {
            assert!((a - z).abs() <= 1e-12, "seat {seat:?} combo {c}: marginal moved from {a} to {z}");
        }
        assert_eq!(range_output(&after).0[0], f32::MIN_POSITIVE, "seat {seat:?}: positive reach stays positive at the output");
    }
}

/// R3's rejection side: a merged mass whose exact weighted average is below the smallest positive
/// `f64` (weight `2^-1074` merged into a residual of weight .5, mass .2: 0.4 of the smallest
/// positive value) is unrepresentable support, not zero support -- rejected with a named
/// assertion, as `condition` and `marginal` do, never left as a silent zero.
#[test]
#[should_panic(expected = "cap_branches: residual seat Seat(0) mass[0] underflowed to 0 despite a positive branch weight and mass on some merged branch")]
fn cap_branches_rejects_an_unrepresentable_merged_support() {
    let tiny = f64::from_bits(1);
    let branch = |id: u8, q: f64, m0: f64| {
        let mut mass = vec![1.0; 1326];
        mass[0] = m0;
        HistoryBranch {
            id,
            parent: None,
            split_by: None,
            translated: vec![],
            q,
            residual: false,
            stopped: None,
            seats: vec![SeatMass { seat: Seat(0), node: None, mass }],
        }
    };
    let mut bs: Vec<HistoryBranch> = (0..4u8).map(|id| branch(id, 0.1, 0.0)).collect();
    bs.push(branch(4, tiny, 0.2));
    let mut residual = branch(5, 0.5, 0.0);
    residual.residual = true;
    bs.push(residual);
    cap_branches(&mut bs);
}

/// With normalized weights a merge whose every weight is zero has no average to take (`0 / 0`);
/// the zero-weight branches contribute nothing to any marginal, so the residual keeps its own
/// masses and weight 0 rather than turning into NaN.
#[test]
fn cap_branches_merges_zero_weight_overflow_without_nan() {
    let seat = Seat(0);
    let mut bs: Vec<HistoryBranch> = [0.3, 0.25, 0.2, 0.15, 0.0, 0.0]
        .iter()
        .enumerate()
        .map(|(id, &q)| {
            let mut mass = vec![0.0; 1326];
            mass[0] = 1.0 + id as f64;
            mass[1] = 7.0 - id as f64;
            HistoryBranch {
                id: u8::try_from(id).unwrap(),
                parent: None,
                split_by: None,
                translated: vec![],
                q,
                residual: false,
                stopped: None,
                seats: vec![SeatMass { seat, node: None, mass }],
            }
        })
        .collect();
    let before = marginal(&bs, seat);
    cap_branches(&mut bs);
    let r = bs.iter().find(|b| b.residual).expect("the zero-weight overflow forms the residual");
    assert_eq!(r.id, 4);
    assert_eq!(r.q, 0.0);
    assert_eq!((r.seats[0].mass[0], r.seats[0].mass[1]), (5.0, 3.0), "the first merged branch's masses are kept");
    assert_eq!(marginal(&bs, seat), before);
}

/// Guard for the merge's overflow fallback (green before and after the fix): masses far above the
/// power-of-two scale used against underflow (here `1e300`) still merge to their finite weighted
/// average instead of overflowing to infinity.
#[test]
fn cap_branches_merges_huge_masses_without_overflow() {
    let seat = Seat(0);
    let mut bs: Vec<HistoryBranch> = (0..6u8)
        .map(|id| {
            let mut mass = vec![0.0; 1326];
            mass[0] = 1e300;
            mass[1] = 1e300 * (1.0 + f64::from(id));
            HistoryBranch {
                id,
                parent: None,
                split_by: None,
                translated: vec![],
                q: 0.3 - f64::from(id) * 0.05,
                residual: false,
                stopped: None,
                seats: vec![SeatMass { seat, node: None, mass }],
            }
        })
        .collect();
    cap_branches(&mut bs);
    let r = bs.iter().find(|b| b.residual).expect("the overflow forms the residual");
    let (q4, q5) = (0.3 - 4.0 * 0.05, 0.3 - 5.0 * 0.05);
    let expected1 = (q4 * 5e300 + q5 * 6e300) / (q4 + q5);
    assert!(r.seats[0].mass[0].is_finite() && (r.seats[0].mass[0] / 1e300 - 1.0).abs() < 1e-12, "{}", r.seats[0].mass[0]);
    assert!(r.seats[0].mass[1].is_finite() && (r.seats[0].mass[1] / expected1 - 1.0).abs() < 1e-12, "{}", r.seats[0].mass[1]);
}

// ---------------------------------------------------------------------------------------------
// P3.T12 fix round 2 (task-12-rereview-1.md N1, ruling 12-N1): at the last subnormal unit the
// correctly rounded residual mass can make the residual's own marginal product `q_R' * mass`
// round to zero although the pre-cap marginal was the representable `5e-324`. The merge raises
// such a mass by one representable step, so the cap never accepts a state that the next
// `marginal`/`rescale` rejects.
// ---------------------------------------------------------------------------------------------

/// A one-seat branch with weight `q`, combo-0 mass `m0`, combo-1 mass `1 - m0` and mass 1 on
/// every other combo, so every branch built here has the same per-seat total (the equal-total
/// invariant).
fn last_unit_branch(id: u8, q: f64, m0: f64) -> HistoryBranch {
    let mut mass = vec![1.0; 1326];
    mass[0] = m0;
    mass[1] = 1.0 - m0;
    HistoryBranch {
        id,
        parent: None,
        split_by: None,
        translated: vec![],
        q,
        residual: false,
        stopped: None,
        seats: vec![SeatMass { seat: Seat(0), node: None, mass }],
    }
}

/// N1 shared check: the pre-cap combo-0 marginal is the representable `5e-324` and its only support
/// is merged by the cap. The cap must accept, give the residual combo-0 mass `expected_mass` (one
/// step above the correctly rounded average, which alone would round `q_R' * mass` to zero), keep
/// every marginal (combo 0 bit-equal and positive), and leave a list that `rescale` accepts with
/// combo 0 still positive and reported as `f32::MIN_POSITIVE` at the output boundary.
fn check_last_unit_cap(mut bs: Vec<HistoryBranch>, residual_id: u8, expected_mass: f64) {
    let seat = Seat(0);
    let before = marginal(&bs, seat);
    assert_eq!(before[0], f64::from_bits(1), "the pre-cap combo-0 marginal is the representable 5e-324");

    cap_branches(&mut bs);
    assert_eq!(bs.iter().filter(|b| !b.residual).count(), 4);
    let residual = bs.iter().find(|b| b.residual).expect("the cap leaves one residual");
    assert_eq!(residual.id, residual_id);
    let after = marginal(&bs, seat);
    assert!(after[0] > 0.0, "combo 0 lost its only (merged) support across the cap");
    assert_eq!(after[0], before[0], "combo-0 marginal");
    assert_eq!(residual.seats[0].mass[0], expected_mass, "residual combo-0 mass");
    for (c, (a, z)) in before.iter().zip(&after).enumerate() {
        assert!((a - z).abs() <= 1e-12, "combo {c}: marginal moved from {a} to {z} across cap_branches");
    }

    let mut logs = vec![0.0; 6];
    rescale(&mut bs, &mut logs);
    assert!(logs[0].is_finite(), "log_reach {}", logs[0]);
    let rescaled = marginal(&bs, seat);
    assert!(rescaled[0] > 0.0, "combo 0 lost its support in the rescale after the cap");
    assert_eq!(range_output(&rescaled).0[0], f32::MIN_POSITIVE, "positive reach stays positive at the output");
}

/// N1 (the re-review's reproduction): four live branches (`q = .1`, combo-0 mass 0), one overflow
/// branch (`q = 2^-1074`, combo-0 mass .55) and an existing residual (`q = .4`, combo-0 mass 0).
/// The correctly rounded average is `1.375 * 2^-1074 -> 2^-1074`, and `.4 * 2^-1074` rounds to 0;
/// one step up (`2 * 2^-1074`, the old pairwise formula's value) keeps `.8 * 2^-1074 -> 5e-324`.
#[test]
fn cap_branches_keeps_the_residual_marginal_positive_at_the_last_subnormal_unit() {
    let mut bs: Vec<HistoryBranch> = (0..4u8).map(|id| last_unit_branch(id, 0.1, 0.0)).collect();
    bs.push(last_unit_branch(4, f64::from_bits(1), 0.55));
    let mut residual = last_unit_branch(5, 0.4, 0.0);
    residual.residual = true;
    bs.push(residual);
    check_last_unit_cap(bs, 5, f64::from_bits(2));
}

/// N1 grid case with the residual **created** by the cap: four live branches (`q = .1`, combo-0
/// mass 0); the first overflow branch (`q = .095`, combo-0 mass 0) becomes the residual, and two
/// more overflow branches (`q = 2^-1074` each, combo-0 masses .3 and .21) merge into it. The
/// pre-cap combo-0 marginal is `.51 * 2^-1074 -> 5e-324`; the correctly rounded average
/// `5.37 * 2^-1074 -> 5 * 2^-1074` gives `.475 * 2^-1074 -> 0`, so the mass steps to
/// `6 * 2^-1074` (`.57 * 2^-1074 -> 5e-324`).
#[test]
fn cap_branches_keeps_a_created_residual_marginal_positive_at_the_last_subnormal_unit() {
    let mut bs: Vec<HistoryBranch> = (0..4u8).map(|id| last_unit_branch(id, 0.1, 0.0)).collect();
    bs.push(last_unit_branch(4, 0.095, 0.0));
    bs.push(last_unit_branch(5, f64::from_bits(1), 0.3));
    bs.push(last_unit_branch(6, f64::from_bits(1), 0.21));
    check_last_unit_cap(bs, 4, f64::from_bits(6));
}
