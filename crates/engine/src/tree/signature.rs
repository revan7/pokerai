//! The §4.6 structural signature of an effective tree.
//!
//! The signature is the cache and snapshot key's structural component (§9.1, §10.4), so it must be
//! deterministic across processes and chip scales and must never depend on hero's cards, the board or
//! the realized chip amounts: it hashes the *nominal* tree (rules version, template id, root street,
//! menus, thresholds, wager cap) plus each inserted size as a reduced rational of the root pot.

use proto::{resolve_chip_path, Action, EffectiveTree, MenuSize};
use sha2::{Digest, Sha256};
use std::fmt::Write;

fn gcd(a: u64, b: u64) -> u64 { if b == 0 { a } else { gcd(b, a % b) } }

/// Every nominal float in the preimage — menu and donk coefficients, and the three thresholds — is
/// encoded losslessly as its IEEE-754 bit pattern in fixed-width lowercase hex (8 digits), with one
/// normalization: `-0.0` is encoded as `+0.0`.
///
/// Lossless because §4.6 keys the *nominal* coefficients, and the whole accepted domain of a
/// `TemplateSpec` reaches this function: any decimal-rounded encoding aliases distinct accepted
/// values (`0x3f000002` and `0x3f000003` both print as `0.500000` yet size a 10,000,000 pot to
/// 5,000,001 and 5,000,002 chips), which would let one cache or snapshot key serve a strategy solved
/// for a different tree. Fixed width matters too: eight digits can never be read as the `"a"` of an
/// all-in entry. Signed zero is normalized because `-0.0 == 0.0` is the same number and the
/// non-negative threshold check admits `-0.0`, so it must not split one tree into two keys; no other
/// value is touched, and NaN cannot reach a validated template.
fn float_bits(x: f32) -> String { format!("{:08x}", if x == 0.0 { 0.0f32.to_bits() } else { x.to_bits() }) }
fn size(m: &MenuSize) -> String { match m { MenuSize::Pot(x) => float_bits(*x), MenuSize::AllIn => "a".into() } }
fn sizes(v: &[MenuSize]) -> String { v.iter().map(size).collect::<Vec<_>>().join(",") }

/// The exact bytes `tree_signature` hashes. Kept separate so the frozen digest of `signature_is_frozen`
/// can be reproduced outside this process (the test quotes both the preimage and the digest).
///
/// `write!` into a `String` cannot fail (`impl fmt::Write for String` always returns `Ok`), so the
/// results are discarded rather than propagated: there is no error path to swallow here.
fn preimage(tree: &EffectiveTree, p: u32) -> String {
    // A zero root pot would reduce every inserted size to `1/0` and collide sizes that differ
    // (`73/100` and `50/100` both becoming `1/0`) inside a cache key. `materialize` refuses a zero
    // starting pot, so every tree that exists here was built at `p >= 1`: an infallible precondition,
    // asserted always rather than clamped or encoded away.
    assert!(p > 0, "tree_signature needs the root pot the tree was materialized at, got 0");
    let mut s = String::new();
    let _ = write!(s, "rules={}|template={}|root={:?}|", tree.rules_version, tree.template_id, tree.root_street);
    for (street, m) in &tree.menus {
        let donk = match &m.donk { None => "none".to_string(), Some(d) => format!("[{}]", sizes(d)) };
        let _ = write!(s, "{:?}:oop.bet=[{}],oop.raise=[{}],ip.bet=[{}],ip.raise=[{}],donk={};", street, sizes(&m.oop.bet), sizes(&m.oop.raise), sizes(&m.ip.bet), sizes(&m.ip.raise), donk);
    }
    let _ = write!(s, "|add={}|force={}|merge={}|cap={}|", float_bits(tree.add_allin_threshold), float_bits(tree.force_allin_threshold), float_bits(tree.merging_threshold), tree.wager_cap);
    for (chip, actor, action) in &tree.inserted {
        // Every inserted entry names the node it was inserted at by the chip path the materializer
        // walked, so it resolves by construction. An unresolvable path must not silently become the
        // root's empty path: that would give two structurally different trees one signature.
        let path = resolve_chip_path(&tree.materialized, chip)
            .unwrap_or_else(|| panic!("inserted chip path {chip:?} does not resolve against the tree's own materialized nodes"));
        let (kind, to) = match action { Action::Bet { to } => ("bet", *to), Action::Raise { to } => ("raise", *to), Action::AllIn { to } => ("allin", *to), Action::Fold => ("fold", 0), Action::Check => ("check", 0), Action::Call => ("call", 0) };
        let g = gcd(to as u64, p as u64).max(1);
        let _ = write!(s, "ins@{:?}:{}:{}:{}/{};", path, actor, kind, to as u64 / g, p as u64 / g);
    }
    s
}

/// §4.6: sha256 over rules_version, template_id, root_street, nominal menus (explicit donk lists included),
/// thresholds, wager_cap and the inserted sizes as reduced rationals `to / P` at their ordinal paths. No raw chips.
///
/// Every nominal float — menu coefficient, donk coefficient, threshold — is hashed as its exact
/// IEEE-754 bit pattern (`float_bits`), so two nominally different trees can never share a key.
///
/// Panics when `p` is 0, or when an inserted entry's chip path does not resolve against `tree.materialized`
/// (both are engine bugs, not inputs: see `preimage`).
pub fn tree_signature(tree: &EffectiveTree, p: u32) -> String {
    hex::encode(Sha256::digest(preimage(tree, p).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{build_tree_full, materialize_at, TemplateSelection, TemplateSpec};
    use proto::{Card, PlayerMenus, Seat, SideMenu, Street, StreetRootSnapshot};
    use std::collections::BTreeMap;

    /// A one-street template whose single bet coefficient and two all-in thresholds are the test's
    /// own numbers. Built here instead of through `Templates::with_extra` so these tests cannot race
    /// the registry tests, which share this test binary.
    fn local(bet: f32, add: f32, force: f32) -> TemplateSpec {
        TemplateSpec {
            id: "signature_local_test",
            root_street: Street::River,
            menus: BTreeMap::from([(Street::River, PlayerMenus {
                oop: SideMenu { bet: vec![MenuSize::Pot(bet)], raise: vec![] },
                ip: SideMenu { bet: vec![], raise: vec![] },
                donk: None,
            })]),
            add_allin_threshold: add,
            force_allin_threshold: force,
            merging_threshold: 0.0,
            wager_cap: 1,
        }
    }

    fn snap(pot: u32, eff: u32, board: &str, history: Vec<(Seat, Action)>) -> StreetRootSnapshot {
        StreetRootSnapshot { street: Street::Flop, board: board.split(' ').map(|c| Card::parse(c).unwrap()).collect(),
            oop: Seat(2), ip: Seat(0), pot_root: pot, stack_oop_root: eff, stack_ip_root: eff, dead_this_street: 0, projected_from: 2, history, bb_chips: 2 }
    }
    fn tree(pot: u32, eff: u32, board: &str, history: Vec<(Seat, Action)>) -> EffectiveTree {
        let sel = TemplateSelection::from_history("flop_fast_v1", &history);
        build_tree_full(&snap(pot, eff, board, history), &sel).unwrap().tree
    }
    fn inserted_73() -> EffectiveTree { tree(100, 500, "Kh 7d 2c", vec![(Seat(2), Action::Bet { to: 73 })]) }

    /// The preimage and its digest are frozen: the cache and the replay snapshot store key on this
    /// string across processes and releases, so a change here invalidates stored payloads and must be
    /// a deliberate `rules_version` decision. Both were reproduced independently, outside this crate,
    /// by a Python script that derives the f32 bit patterns itself with `struct.pack('>f', x).hex()`
    /// rather than copying them from Rust, then prints the assembled string and
    /// `hashlib.sha256(s.encode()).hexdigest()`; it printed `len=388` and
    /// `sha256=6c6f8aedfc37825ac8f72a774b5a320f9b013775df603c47c55cd77d10db671e`
    /// (the preimage is pure ASCII and carries no trailing newline).
    ///
    /// Refrozen in fix round 1 for review finding R1 (lossless float encoding). The length is 388
    /// both before and after only because eight hex digits happen to be as wide as `0.500000` was;
    /// the digest changed, and no stored cache or snapshot key existed yet to migrate.
    #[test]
    fn signature_is_frozen() {
        let t = inserted_73();
        assert_eq!(
            preimage(&t, 100),
            "rules=3|template=flop_fast_v1|root=Flop|Flop:oop.bet=[3f000000],oop.raise=[40200000],ip.bet=[3f000000],ip.raise=[40200000],donk=none;Turn:oop.bet=[3f000000],oop.raise=[40200000],ip.bet=[3f000000],ip.raise=[40200000],donk=[];River:oop.bet=[3f000000],oop.raise=[40200000],ip.bet=[3f000000],ip.raise=[40200000],donk=[];|add=3f800000|force=3e19999a|merge=00000000|cap=3|ins@[]:oop:bet:73/100;"
        );
        assert_eq!(preimage(&t, 100).len(), 388);
        assert_eq!(tree_signature(&t, 100), "6c6f8aedfc37825ac8f72a774b5a320f9b013775df603c47c55cd77d10db671e");
        // the encoded values are the nominal template's own numbers, bit for bit
        assert_eq!((0.5f32.to_bits(), 2.5f32.to_bits(), 1.0f32.to_bits(), 0.15f32.to_bits(), 0.0f32.to_bits()), (0x3f00_0000, 0x4020_0000, 0x3f80_0000, 0x3e19_999a, 0));
    }

    /// R1 (review of P2.T4): §4.6 keys the *nominal* coefficients, so two menus that differ only by
    /// adjacent accepted `f32` values are two different nominal trees and must not share a signature —
    /// not even when they realize the same integer tree, which is the case a decimal-rounded encoding
    /// cannot distinguish. `0x3f000002` and `0x3f000003` both print as `0.500000` and both round to
    /// `Bet(50)` into a pot of 100, while at a pot of 10,000,000 they realize 5,000,001 and 5,000,002.
    #[test]
    fn adjacent_menu_coefficients_are_different_trees() {
        let (lo, hi) = (f32::from_bits(0x3f00_0002), f32::from_bits(0x3f00_0003));
        assert_ne!(lo.to_bits(), hi.to_bits());
        assert_eq!(format!("{lo:.6}"), format!("{hi:.6}"), "the pair is indistinguishable at six decimals");
        let small = |c: f32| materialize_at(&local(c, 0.0, 0.0), 100, 500, &[]).unwrap().tree;
        let (a, b) = (small(lo), small(hi));
        assert_eq!(a.materialized, b.materialized, "at this pot both realize the same integer tree");
        assert_ne!(tree_signature(&a, 100), tree_signature(&b, 100), "different nominal menus are different signatures");
        // the witness that they really are different trees: a pot large enough to separate them
        let big = |c: f32| materialize_at(&local(c, 0.0, 0.0), 10_000_000, 10_000_000, &[]).unwrap().tree;
        assert_eq!(big(lo).materialized[0].actions, vec![Action::Check, Action::Bet { to: 5_000_001 }]);
        assert_eq!(big(hi).materialized[0].actions, vec![Action::Check, Action::Bet { to: 5_000_002 }]);
    }

    /// R1: the same applies to the thresholds. `0x3e19999a` and `0x3e19999b` both print as `0.150000`
    /// and neither changes this small tree's realized menu, but they are different nominal trees.
    #[test]
    fn adjacent_thresholds_are_different_trees() {
        let (lo, hi) = (f32::from_bits(0x3e19_999a), f32::from_bits(0x3e19_999b));
        assert_eq!(format!("{lo:.6}"), format!("{hi:.6}"));
        let add = |t: f32| materialize_at(&local(0.5, t, 0.0), 100, 500, &[]).unwrap().tree;
        let force = |t: f32| materialize_at(&local(0.5, 0.0, t), 100, 500, &[]).unwrap().tree;
        for (what, a, b) in [("add", add(lo), add(hi)), ("force", force(lo), force(hi))] {
            assert_eq!(a.materialized, b.materialized, "{what}: the realized tree is the same either way");
            assert_ne!(tree_signature(&a, 100), tree_signature(&b, 100), "{what}: a different threshold is a different signature");
        }
    }

    /// R1's signed-zero policy: `-0.0` passes the non-negative threshold check, so it can reach a tree,
    /// and it is the same number as `+0.0` — the encoding normalizes it instead of signing the bits.
    #[test]
    fn negative_zero_thresholds_hash_as_positive_zero() {
        let plus = materialize_at(&local(0.5, 0.0, 0.0), 100, 500, &[]).unwrap().tree;
        let mut minus = materialize_at(&local(0.5, -0.0, -0.0), 100, 500, &[]).unwrap().tree;
        minus.merging_threshold = -0.0;
        assert!(minus.add_allin_threshold.is_sign_negative() && minus.force_allin_threshold.is_sign_negative() && minus.merging_threshold.is_sign_negative());
        assert_eq!(tree_signature(&plus, 100), tree_signature(&minus, 100));
    }

    /// §4.6: no raw chips and no board. The same nominal tree at a different chip scale, and over a
    /// different flop, is the same structural signature; hero's cards never reach the tree at all.
    #[test]
    fn signature_ignores_chip_scale_and_board() {
        let base = tree_signature(&inserted_73(), 100);
        assert_eq!(tree_signature(&tree(200, 1000, "Kh 7d 2c", vec![(Seat(2), Action::Bet { to: 146 })]), 200), base, "73/100 and 146/200 reduce to the same rational");
        assert_eq!(tree_signature(&tree(100, 500, "As Ad 5h", vec![(Seat(2), Action::Bet { to: 73 })]), 100), base, "the board is keyed separately (§10.4), never in the signature");
        assert_ne!(tree_signature(&tree(100, 500, "Kh 7d 2c", vec![(Seat(2), Action::Bet { to: 71 })]), 100), base, "a different inserted size is a different tree");
    }

    /// Every field of the preimage is discriminating: an otherwise identical tree that differs in
    /// exactly one of them must hash differently.
    #[test]
    fn every_signed_field_changes_the_signature() {
        let base = inserted_73();
        let sig = tree_signature(&base, 100);
        let cases: Vec<(&str, Box<dyn Fn(&mut EffectiveTree)>)> = vec![
            ("rules_version", Box::new(|t: &mut EffectiveTree| t.rules_version += 1)),
            ("template_id", Box::new(|t: &mut EffectiveTree| t.template_id = "flop_full_v1".into())),
            ("root_street", Box::new(|t: &mut EffectiveTree| t.root_street = Street::Turn)),
            ("oop.bet", Box::new(|t: &mut EffectiveTree| t.menus.get_mut(&Street::Flop).unwrap().oop.bet = vec![MenuSize::Pot(0.75)])),
            ("oop.raise", Box::new(|t: &mut EffectiveTree| t.menus.get_mut(&Street::Flop).unwrap().oop.raise = vec![MenuSize::Pot(3.0)])),
            ("ip.bet", Box::new(|t: &mut EffectiveTree| t.menus.get_mut(&Street::Flop).unwrap().ip.bet = vec![MenuSize::AllIn])),
            ("ip.raise", Box::new(|t: &mut EffectiveTree| t.menus.get_mut(&Street::Flop).unwrap().ip.raise = vec![])),
            ("donk none vs empty", Box::new(|t: &mut EffectiveTree| t.menus.get_mut(&Street::Flop).unwrap().donk = Some(vec![]))),
            ("donk sizes", Box::new(|t: &mut EffectiveTree| t.menus.get_mut(&Street::Turn).unwrap().donk = Some(vec![MenuSize::Pot(0.5)]))),
            ("a street's menus", Box::new(|t: &mut EffectiveTree| { t.menus.remove(&Street::River); })),
            ("add_allin_threshold", Box::new(|t: &mut EffectiveTree| t.add_allin_threshold = 1.5)),
            ("force_allin_threshold", Box::new(|t: &mut EffectiveTree| t.force_allin_threshold = 0.2)),
            ("merging_threshold", Box::new(|t: &mut EffectiveTree| t.merging_threshold = 0.1)),
            ("wager_cap", Box::new(|t: &mut EffectiveTree| t.wager_cap = 1)),
            ("inserted actor", Box::new(|t: &mut EffectiveTree| t.inserted[0].1 = "ip".into())),
            ("inserted kind", Box::new(|t: &mut EffectiveTree| t.inserted[0].2 = Action::Raise { to: 73 })),
            ("inserted size", Box::new(|t: &mut EffectiveTree| t.inserted[0].2 = Action::Bet { to: 74 })),
            ("inserted path", Box::new(|t: &mut EffectiveTree| t.inserted[0].0 = vec![Action::Bet { to: 73 }])),
            ("an extra insertion", Box::new(|t: &mut EffectiveTree| t.inserted.push((vec![], "ip".into(), Action::Bet { to: 73 })))),
        ];
        for (what, edit) in cases {
            let mut t = base.clone();
            edit(&mut t);
            assert_ne!(tree_signature(&t, 100), sig, "changing {what} must change the signature");
        }
        // control: the same tree twice is the same signature, and a clone of the base is unchanged
        assert_eq!(tree_signature(&base.clone(), 100), sig);
    }

    /// The two infallible preconditions are asserted, never worked around: a zero root pot would
    /// collide every inserted size onto `1/0`, and an unresolvable inserted path onto the root.
    #[test]
    fn preconditions_are_asserted() {
        let t = inserted_73();
        let zero_pot = std::panic::catch_unwind(|| tree_signature(&t, 0));
        assert!(zero_pot.is_err(), "a zero root pot must not silently produce a signature");
        let mut broken = inserted_73();
        broken.inserted[0].0 = vec![Action::Bet { to: 60 }];   // no such action at the root
        let unresolvable = std::panic::catch_unwind(move || tree_signature(&broken, 100));
        assert!(unresolvable.is_err(), "an unresolvable inserted path must not collapse onto the root path");
    }
}
