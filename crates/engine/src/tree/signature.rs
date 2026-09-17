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
/// Nominal menu sizes are encoded with six decimals (§4.6 "nominal menus"), not bit-exactly: two
/// coefficients closer than 1e-6 therefore share a signature. Every §10.1 size is far apart, and a
/// cache lookup additionally compares the stored realized `materialized` list (§10.4), which two
/// different menus never share, so the encoding is not the only guard against a wrong hit.
fn size(m: &MenuSize) -> String { match m { MenuSize::Pot(x) => format!("{x:.6}"), MenuSize::AllIn => "a".into() } }
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
    let _ = write!(s, "|add={:.6}|force={:.6}|merge={:.6}|cap={}|", tree.add_allin_threshold, tree.force_allin_threshold, tree.merging_threshold, tree.wager_cap);
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
/// Panics when `p` is 0, or when an inserted entry's chip path does not resolve against `tree.materialized`
/// (both are engine bugs, not inputs: see `preimage`).
pub fn tree_signature(tree: &EffectiveTree, p: u32) -> String {
    hex::encode(Sha256::digest(preimage(tree, p).as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{build_tree_full, TemplateSelection};
    use proto::{Card, Seat, Street, StreetRootSnapshot};

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
    /// a deliberate `rules_version` decision. The digest below was reproduced independently, outside
    /// this crate, with `python -c "import hashlib; s='<the 388-byte preimage asserted below>';
    /// print(len(s)); print(hashlib.sha256(s.encode()).hexdigest())"`, which printed
    /// `388` and `1df260b5bfd770869008339d91d445b0bb8834160816527e312c1ca6c7ec898d`
    /// (the preimage is pure ASCII and carries no trailing newline).
    #[test]
    fn signature_is_frozen() {
        let t = inserted_73();
        assert_eq!(
            preimage(&t, 100),
            "rules=3|template=flop_fast_v1|root=Flop|Flop:oop.bet=[0.500000],oop.raise=[2.500000],ip.bet=[0.500000],ip.raise=[2.500000],donk=none;Turn:oop.bet=[0.500000],oop.raise=[2.500000],ip.bet=[0.500000],ip.raise=[2.500000],donk=[];River:oop.bet=[0.500000],oop.raise=[2.500000],ip.bet=[0.500000],ip.raise=[2.500000],donk=[];|add=1.000000|force=0.150000|merge=0.000000|cap=3|ins@[]:oop:bet:73/100;"
        );
        assert_eq!(preimage(&t, 100).len(), 388);
        assert_eq!(tree_signature(&t, 100), "1df260b5bfd770869008339d91d445b0bb8834160816527e312c1ca6c7ec898d");
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
