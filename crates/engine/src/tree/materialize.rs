//! Materialization of one §10.1 template against a concrete street root: the betting skeleton of
//! spec §2, built by the pinned §4.6 rules (the `push_actions` / `create_next` pair of the vendored
//! solver), plus the §10.2 exact insertion of an observed wager that the template does not list.
//!
//! Chip arithmetic runs in `i64` (the vendored solver's `i32` widened), with `materialize`'s entry
//! guard keeping `starting_pot + 2 * eff` below 2^31 so every amount and pot narrows back to `u32`
//! without loss.

use super::templates::TemplateSpec;
use proto::{Action, MaterializedNode, MenuSize, OrdinalPath, Street, UnsupportedReason};

pub const OOP: usize = 0;
pub const IP: usize = 1;

pub fn actor_name(actor: usize) -> String { if actor == OOP { "oop".into() } else { "ip".into() } }
pub fn order_key(a: &Action) -> (u8, u32) {
    match a { Action::Fold => (0, 0), Action::Check => (1, 0), Action::Call => (2, 0), Action::Bet { to } | Action::Raise { to } => (3, *to), Action::AllIn { to } => (4, *to) }
}
fn unsupported(reason: impl Into<String>) -> UnsupportedReason { UnsupportedReason::UnsupportedHistory { reason: reason.into() } }
fn round(x: f64) -> i64 { x.round() as i64 }   // f64::round: half away from zero, as upstream
/// Narrows a menu amount to the wire's `u32`. Upstream's single `(f64).round() as i32` cast
/// saturates; splitting it into `round -> i64 -> u32` would instead wrap, so the saturation is
/// restored here. Every saturated value is above `max` and is clamped to the all-in immediately
/// after, exactly as upstream handles an oversized size.
fn chips(x: i64) -> u32 { x.clamp(0, u32::MAX as i64) as u32 }

#[derive(Clone, Copy, PartialEq, Debug)]
enum Prev { Root, Check, Chance, Wager }
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind { Opening, Donk, Facing }

/// Mirror of upstream `BuildTreeInfo` plus the street/actor bookkeeping of the betting skeleton.
///
/// `wagers_this_street` counts observed wagers as well as template ones and an observed prefix may
/// legally carry more wagers than `wager_cap` allows, so the counter is `usize`: a byte counter
/// overflows on a 256-wager prefix in debug and wraps in release, which reopens capped raises.
#[derive(Clone, Debug)]
struct Walk { street: Street, actor: usize, prev: Prev, wagers_this_street: usize, allin: bool, oop_call: bool, stack: [i64; 2], prev_amount: i64, matched: i64 }

struct Operands { to_call: i64, pot: i64, max: i64, min: i64 }
fn operands(w: &Walk, p: i64) -> Operands {
    let own = w.stack[w.actor];
    let opp = w.stack[w.actor ^ 1];
    let to_call = own - opp;
    let pot = p + 2 * (w.matched + to_call);
    let max = opp + w.prev_amount;
    let min = (w.prev_amount + to_call).clamp(1, max);
    Operands { to_call, pot, max, min }
}
fn kind(w: &Walk) -> Kind {
    match w.prev { Prev::Chance if w.oop_call => Kind::Donk, Prev::Root | Prev::Check | Prev::Chance => Kind::Opening, Prev::Wager => Kind::Facing }
}

/// Validates the template's own numbers before any of them reaches a rounding or a narrowing.
///
/// `TemplateSpec`'s fields are public and the registry establishes no invariant on a template a
/// caller builds itself, so this is the boundary: a menu coefficient must be inside the domain
/// `proto::MenuSize` enforces on both serde directions (positive and finite — a value outside it
/// could not even be serialized into the `EffectiveTree` the worker receives), the two all-in
/// thresholds must be finite and non-negative (upstream's own `ConfigError` rule, plus the
/// finiteness upstream's `< 0.0` test misses for NaN), and the merge threshold stays pinned to 0.0
/// because upstream's `merge_bet_actions` step is not mirrored here.
fn validate_template(t: &TemplateSpec) -> Result<(), UnsupportedReason> {
    let sizes = |label: &'static str, v: &[MenuSize], street: Street| -> Result<(), UnsupportedReason> {
        for s in v {
            if let MenuSize::Pot(x) = s {
                if !(x.is_finite() && *x > 0.0) {
                    return Err(unsupported(format!("template {} has an out-of-domain {label} menu size {x:e} on {street:?}: a menu size must be positive and finite", t.id)));
                }
            }
        }
        Ok(())
    };
    for (street, m) in &t.menus {
        sizes("oop bet", &m.oop.bet, *street)?;
        sizes("oop raise", &m.oop.raise, *street)?;
        sizes("ip bet", &m.ip.bet, *street)?;
        sizes("ip raise", &m.ip.raise, *street)?;
        if let Some(donk) = &m.donk { sizes("donk", donk, *street)?; }
    }
    for (label, x) in [("add_allin_threshold", t.add_allin_threshold), ("force_allin_threshold", t.force_allin_threshold)] {
        if !(x.is_finite() && x >= 0.0) {
            return Err(unsupported(format!("template {} has an out-of-domain {label} {x:e}: a threshold must be non-negative and finite", t.id)));
        }
    }
    if t.merging_threshold != 0.0 {
        return Err(unsupported(format!("template {} has merging_threshold {:e}, which is pinned to 0.0", t.id, t.merging_threshold)));
    }
    Ok(())
}

/// The menu of a node: built, clamped, forced, sorted and deduplicated exactly as upstream `push_actions` (§4.6).
fn menu(w: &Walk, t: &TemplateSpec, p: i64) -> Result<Vec<Action>, UnsupportedReason> {
    let o = operands(w, p);
    let street = t.menus.get(&w.street).ok_or_else(|| unsupported(format!("template {} has no menu for {:?}", t.id, w.street)))?;
    let side = if w.actor == OOP { &street.oop } else { &street.ip };
    let add = t.add_allin_threshold as f64;
    let force = t.force_allin_threshold as f64;
    // Infallible after `validate_template`, which every path into this function runs first.
    assert_eq!(t.merging_threshold, 0.0, "template {} reached menu() with an unpinned merging_threshold", t.id);
    let allin = Action::AllIn { to: chips(o.max) };
    let mut v: Vec<Action> = Vec::new();
    match kind(w) {
        Kind::Donk => {
            v.push(Action::Check);
            let donk = street.donk.as_ref().ok_or_else(|| unsupported("donk menu must be the explicit empty list"))?;
            if !donk.is_empty() { return Err(unsupported("donk sizes are pinned empty in phase 1")); }
            if o.max <= round(o.pot as f64 * add) { v.push(allin); }
        }
        Kind::Opening => {
            v.push(Action::Check);
            for b in &side.bet {
                match b { MenuSize::Pot(r) => v.push(Action::Bet { to: chips(round(o.pot as f64 * *r as f64)) }), MenuSize::AllIn => v.push(allin) }
            }
            if o.max <= round(o.pot as f64 * add) { v.push(allin); }
        }
        Kind::Facing => {
            v.push(Action::Fold);
            v.push(Action::Call);
            if !w.allin {
                for r in &side.raise {
                    match r { MenuSize::Pot(x) => v.push(Action::Raise { to: chips(round(w.prev_amount as f64 * *x as f64)) }), MenuSize::AllIn => v.push(allin) }
                }
                if o.max <= w.prev_amount + round(o.pot as f64 * add) { v.push(allin); }
            }
        }
    }
    for a in v.iter_mut() {
        let (amt, is_bet) = match *a { Action::Bet { to } => (to as i64, true), Action::Raise { to } => (to as i64, false), _ => continue };
        let c = amt.clamp(o.min, o.max);
        let thr = round((o.pot + 2 * (c - w.prev_amount)) as f64 * force);
        *a = if o.max <= c + thr { allin } else if is_bet { Action::Bet { to: chips(c) } } else { Action::Raise { to: chips(c) } };
    }
    v.sort_by_key(order_key);
    v.dedup();
    Ok(v)
}

/// Observed action -> tree action at this node (§10.2 exact insertion, §4.6 all-in normalization).
///
/// `facing_allin` is the node's `Walk::allin`: §4.6 leaves only fold and call against an all-in,
/// and the pinned `ActionTree::add_line_recursive` refuses a "Bet action after all-in", so an
/// observed wager there is rejected here instead of being normalized and inserted into a tree the
/// worker could not realize.
fn map_observed(k: Kind, o: &Operands, obs: &Action, facing_allin: bool) -> Result<Action, UnsupportedReason> {
    match obs {
        Action::Fold | Action::Check | Action::Call => Ok(*obs),
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => {
            if facing_allin { return Err(unsupported(format!("wager to {to} faces an all-in, where only fold and call are legal"))); }
            let to = *to as i64;
            if to >= o.max { return Ok(Action::AllIn { to: chips(o.max) }); }   // equal to max is forced; above max is the deeper stack's uncontestable excess
            if matches!(obs, Action::AllIn { .. }) { return Err(unsupported(format!("all-in to {to} below the effective maximum {}", o.max))); }
            if to < o.min { return Err(unsupported(format!("wager to {to} below the tree minimum {}", o.min))); }
            Ok(match k { Kind::Facing => Action::Raise { to: to as u32 }, _ => Action::Bet { to: to as u32 } })
        }
    }
}

struct Child { walk: Walk, terminal_pot: Option<u32> }
fn next_street(n: &mut Walk) {
    n.street = match n.street { Street::Flop => Street::Turn, Street::Turn => Street::River, s => s };
    n.actor = OOP; n.prev = Prev::Chance; n.wagers_this_street = 0;
}
/// Mirror of upstream `create_next` plus the terminal classification of §2 (fold, called wager, showdown).
fn child(w: &Walk, a: &Action, p: i64) -> Child {
    let o = operands(w, p);
    let mut n = w.clone();
    let river = w.street == Street::River;
    match a {
        Action::Fold => Child { walk: n, terminal_pot: Some(chips(p + 2 * w.matched)) },
        Action::Check => {
            n.oop_call = false; n.prev = Prev::Check;
            if w.actor == OOP { n.actor = IP; return Child { walk: n, terminal_pot: None }; }
            if river { return Child { walk: n, terminal_pot: Some(chips(p + 2 * w.matched)) }; }
            next_street(&mut n); Child { walk: n, terminal_pot: None }
        }
        Action::Call => {
            n.matched += o.to_call; n.stack[w.actor] = w.stack[w.actor ^ 1]; n.prev_amount = 0; n.oop_call = w.actor == OOP; n.prev = Prev::Chance;
            // the called amount is already in `n.matched`; read it before `n` moves into `Child`
            let called_pot = chips(p + 2 * n.matched);
            if river || w.allin { return Child { walk: n, terminal_pot: Some(called_pot) }; }
            next_street(&mut n); Child { walk: n, terminal_pot: None }
        }
        Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => {
            let amt = *to as i64;
            n.matched += o.to_call;
            n.stack[w.actor] -= amt - w.prev_amount + o.to_call;
            n.prev_amount = amt;
            n.allin = matches!(a, Action::AllIn { .. });
            if !n.allin { n.wagers_this_street += 1; }
            n.actor = w.actor ^ 1; n.prev = Prev::Wager;
            Child { walk: n, terminal_pot: None }
        }
    }
}

pub struct MaterializeInput<'a> { pub template: &'a TemplateSpec, pub starting_pot: u32, pub eff: u32, pub prefix: &'a [(usize, Action)] }
#[derive(Debug, Clone, Default)]
pub struct Materialized { pub nodes: Vec<MaterializedNode>, pub inserted: Vec<(Vec<Action>, String, Action)>, pub history: Vec<Action>, pub decision_path: OrdinalPath }

pub fn materialize(inp: &MaterializeInput) -> Result<Materialized, UnsupportedReason> {
    if inp.starting_pot == 0 || inp.eff == 0 { return Err(unsupported("zero pot or zero effective stack")); }
    if (inp.starting_pot as u64) + 2 * (inp.eff as u64) >= (1u64 << 31) { return Err(unsupported("pot + stacks exceed 2^31")); }
    validate_template(inp.template)?;
    let p = inp.starting_pot as i64;
    let root = Walk { street: inp.template.root_street, actor: OOP, prev: Prev::Root, wagers_this_street: 0, allin: false, oop_call: false, stack: [inp.eff as i64; 2], prev_amount: 0, matched: 0 };
    let mut out = Materialized::default();
    let mut found = false;
    walk(inp, p, &root, &mut Vec::new(), &mut Vec::new(), true, &mut out, &mut found)?;
    if !found { return Err(unsupported("prefix closes the street before hero's decision")); }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn walk(inp: &MaterializeInput, p: i64, w: &Walk, path: &mut OrdinalPath, chip: &mut Vec<Action>, on_prefix: bool, out: &mut Materialized, found: &mut bool) -> Result<(), UnsupportedReason> {
    let t = inp.template;
    let mut menu = menu(w, t, p)?;
    let mut prefix_action: Option<Action> = None;
    if on_prefix {
        if path.len() < inp.prefix.len() {
            let (actor, obs) = &inp.prefix[path.len()];
            if *actor != w.actor { return Err(unsupported(format!("prefix actor mismatch at step {}", path.len()))); }
            let mapped = map_observed(kind(w), &operands(w, p), obs, w.allin)?;
            if !menu.contains(&mapped) {
                if matches!(mapped, Action::Fold | Action::Check | Action::Call) { return Err(unsupported(format!("{mapped:?} not available at step {}", path.len()))); }
                menu.push(mapped);
                menu.sort_by_key(order_key);
                menu.dedup();
                out.inserted.push((chip.clone(), actor_name(w.actor), mapped));
            }
            prefix_action = Some(mapped);
        } else {
            out.decision_path = path.clone();
            *found = true;
        }
    }
    // wager cap (§4.6): observed prefix actions are never removed
    if w.wagers_this_street >= usize::from(t.wager_cap) {
        menu.retain(|a| !matches!(a, Action::Bet { .. } | Action::Raise { .. }) || prefix_action.as_ref() == Some(a));
    }
    let children: Vec<Child> = menu.iter().map(|a| child(w, a, p)).collect();
    out.nodes.push(MaterializedNode { path: path.clone(), street: w.street, actor: actor_name(w.actor), actions: menu.clone(), terminal_pots: children.iter().map(|c| c.terminal_pot).collect() });
    for (i, (a, c)) in menu.iter().zip(children.iter()).enumerate() {
        if c.terminal_pot.is_some() { continue; }
        let child_on_prefix = prefix_action.as_ref() == Some(a);
        // An ordinal that does not fit `u8` would alias another child's path (`proto::resolve_chip_path`
        // rejects exactly that on the wire), so it is refused here rather than narrowed.
        let idx = u8::try_from(i).map_err(|_| unsupported(format!("menu index {i} is not a representable ordinal")))?;
        path.push(idx);
        chip.push(*a);
        if child_on_prefix { out.history.push(*a); }
        walk(inp, p, &c.walk, path, chip, child_on_prefix, out, found)?;
        path.pop();
        chip.pop();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::templates::Templates;
    use proto::{Action, Street};

    fn mat(template: &str, p: u32, eff: u32, prefix: &[(usize, Action)]) -> Materialized {
        materialize(&MaterializeInput { template: Templates::get(template).unwrap(), starting_pot: p, eff, prefix }).unwrap()
    }
    fn node<'a>(m: &'a Materialized, path: &[u8]) -> &'a MaterializedNode { m.nodes.iter().find(|n| n.path == path).unwrap() }
    fn bet(to: u32) -> Action { Action::Bet { to } }
    fn raise(to: u32) -> Action { Action::Raise { to } }
    fn allin(to: u32) -> Action { Action::AllIn { to } }

    #[test]
    fn river_oracle_wire_example() {
        let m = mat("river_oracle_v1", 100, 100, &[(0, Action::Check)]);
        assert_eq!(m.history, vec![Action::Check]);
        assert_eq!(m.decision_path, vec![0]);
        assert_eq!(m.inserted.len(), 0);
        let r = node(&m, &[]);
        assert_eq!((r.actor.as_str(), &r.actions[..], &r.terminal_pots[..]), ("oop", &[Action::Check][..], &[None][..]));
        let ip = node(&m, &[0]);
        assert_eq!((&ip.actions[..], &ip.terminal_pots[..]), (&[Action::Check, allin(100)][..], &[Some(100), None][..]));
        let oop = node(&m, &[0, 1]);
        assert_eq!((oop.actor.as_str(), &oop.actions[..], &oop.terminal_pots[..]), ("oop", &[Action::Fold, Action::Call][..], &[Some(100), Some(300)][..]));
        assert_eq!(m.nodes.len(), 3);
    }

    #[test]
    fn opening_allin_and_force_boundaries() {
        // turn_std_v1 (no `a` entry), add 1.5: P=100, stack 150 lists AllIn(150) (150 <= 150); 151 does not
        assert!(node(&mat("turn_std_v1", 100, 150, &[]), &[]).actions.contains(&allin(150)));
        assert!(!node(&mat("turn_std_v1", 100, 151, &[]), &[]).actions.iter().any(|a| matches!(a, Action::AllIn { .. })));
        // river_std_v1 at P=100, stacks 100 reproduces the measured RIVER root menu (its `a` entry and the threshold both list the all-in)
        assert_eq!(node(&mat("river_std_v1", 100, 100, &[]), &[]).actions, vec![Action::Check, bet(33), bet(75), allin(100)]);
        // flop_fast_v1 opening bet 0.5 at force 0.15: stack 80 forces (80 <= 50 + 30), 81 does not
        assert_eq!(node(&mat("flop_fast_v1", 100, 80, &[]), &[]).actions, vec![Action::Check, allin(80)]);
        assert_eq!(node(&mat("flop_fast_v1", 100, 81, &[]), &[]).actions, vec![Action::Check, bet(50), allin(81)]);
    }

    #[test]
    fn facing_boundaries_of_section_4_6() {
        let prefix = [(0usize, bet(100))];
        let ip = |eff: u32| node(&mat("facing_test_v1", 100, eff, &prefix), &[1]).actions.clone();
        assert_eq!(ip(350), vec![Action::Fold, Action::Call, raise(250), allin(350)]);
        assert_eq!(ip(400), vec![Action::Fold, Action::Call, raise(250), allin(400)]);
        assert_eq!(ip(401), vec![Action::Fold, Action::Call, raise(250)]);
        assert_eq!(ip(340), vec![Action::Fold, Action::Call, allin(340)]);
        assert_eq!(ip(341), vec![Action::Fold, Action::Call, raise(250), allin(341)]);
        assert_eq!(ip(240), vec![Action::Fold, Action::Call, allin(240)]);
        // stack 100: OOP's bet becomes the all-in; IP's node is Fold, Call only
        let m = mat("facing_test_v1", 100, 100, &prefix);
        assert_eq!(m.history, vec![allin(100)]);
        assert_eq!(node(&m, &[1]).actions, vec![Action::Fold, Action::Call]);
    }

    #[test]
    fn cross_street_operand_and_donk_pinning() {
        let m = mat("facing_test_v1", 100, 350, &[]);
        // after OOP Bet(100), IP Call: turn root is an opening node with matched = 100, pot = 300, stacks 250/250
        let turn = node(&m, &[1, 1]);
        assert_eq!((turn.street, turn.actor.as_str(), &turn.actions[..]), (Street::Turn, "oop", &[Action::Check, allin(250)][..]));
        // after OOP Check, IP Bet(100), OOP Call: a donk node with the same operand
        let donk = node(&m, &[0, 1, 1]);
        assert_eq!((donk.street, &donk.actions[..]), (Street::Turn, &[Action::Check, allin(250)][..]));
        // after turn Check, Check the river keeps matched = 100, pot = 300
        let river = node(&m, &[1, 1, 0, 0]);
        assert_eq!((river.street, &river.actions[..]), (Street::River, &[Action::Check, allin(250)][..]));
        // terminal markers: IP fold after the flop bet closes at the matched pot 100; a call is a continuation
        let ip = node(&m, &[1]);
        assert_eq!(ip.terminal_pots[0], Some(100));
        assert_eq!(ip.terminal_pots[1], None);
    }

    #[test]
    fn exact_insertion_and_wager_cap() {
        // villain (oop) bets 73 into 100 with the 0.5 menu: both 50 and 73 are present, 73 inserted, requested node is IP's after 73
        let m = mat("flop_fast_v1", 100, 500, &[(0, bet(73))]);
        assert_eq!(node(&m, &[]).actions, vec![Action::Check, bet(50), bet(73)]);
        assert_eq!(m.inserted, vec![(vec![], "oop".to_string(), bet(73))]);
        assert_eq!(m.decision_path, vec![2]);
        assert_eq!(m.history, vec![bet(73)]);
        // cap 1 (flop_min_v1): after one wager only fold/call/all-in remain; a two-wager prefix is kept intact
        let m1 = mat("flop_min_v1", 100, 500, &[(0, bet(40)), (1, raise(120))]);
        assert_eq!(node(&m1, &[1]).actions, vec![Action::Fold, Action::Call, raise(120), allin(500)]);
        assert_eq!(node(&m1, &[1, 2]).actions, vec![Action::Fold, Action::Call, allin(500)]);
        assert_eq!(m1.decision_path, vec![1, 2]);
        // a wager below the tree minimum is UnsupportedHistory
        let e = materialize(&MaterializeInput { template: Templates::get("flop_fast_v1").unwrap(), starting_pot: 100, eff: 500, prefix: &[(0, bet(30)), (1, raise(40))] });
        assert!(matches!(e, Err(proto::UnsupportedReason::UnsupportedHistory { .. })));
    }

    /// A local river template: OOP opens with `bets`, IP has no sizes, no thresholds fire.
    /// Built here instead of through `Templates::with_extra` so these tests cannot race the
    /// registry tests in `templates.rs`, which share this test binary.
    fn local(bets: Vec<proto::MenuSize>) -> super::TemplateSpec {
        super::TemplateSpec {
            id: "materialize_local_test",
            root_street: Street::River,
            menus: std::collections::BTreeMap::from([(Street::River, proto::PlayerMenus {
                oop: proto::SideMenu { bet: bets, raise: vec![] },
                ip: proto::SideMenu { bet: vec![], raise: vec![] },
                donk: None,
            })]),
            add_allin_threshold: 0.0,
            force_allin_threshold: 0.0,
            merging_threshold: 0.0,
            wager_cap: 3,
        }
    }

    /// Upstream computes a menu amount with one saturating `(f64).round() as i32`; narrowing an
    /// `i64` to `u32` instead wraps, and a wrapped amount can land back inside `[min, max]` and
    /// materialize as an ordinary bet where the rule says all-in. Pot 1e9 with ratio 4.694967296
    /// is such a case: the true amount is just above 2^32, the wrapped one is about 4e8 < max 5e8.
    #[test]
    fn oversized_menu_amount_saturates_into_the_all_in() {
        let t = local(vec![proto::MenuSize::Pot(4.694967296)]);
        let m = materialize(&MaterializeInput { template: &t, starting_pot: 1_000_000_000, eff: 500_000_000, prefix: &[] }).unwrap();
        assert_eq!(node(&m, &[]).actions, vec![Action::Check, allin(500_000_000)]);
        // control: the same pot with a ratio inside the domain keeps its exact bet
        let t = local(vec![proto::MenuSize::Pot(0.25)]);
        let m = materialize(&MaterializeInput { template: &t, starting_pot: 1_000_000_000, eff: 500_000_000, prefix: &[] }).unwrap();
        assert_eq!(node(&m, &[]).actions, vec![Action::Check, bet(250_000_000)]);
    }

    /// Both sides of the ordinal domain: 256 menu entries address ordinals 0..=255 and materialize;
    /// 257 would need ordinal 256, which cannot be an `OrdinalPath` byte and is refused, not wrapped.
    #[test]
    fn menu_wider_than_the_ordinal_domain_is_refused() {
        // round(100_000 * i/1000) = 100 * i, so every size is distinct and survives dedup
        let sizes = |n: u32| (1..=n).map(|i| proto::MenuSize::Pot(i as f32 / 1000.0)).collect::<Vec<_>>();
        let t = local(sizes(255));
        let m = materialize(&MaterializeInput { template: &t, starting_pot: 100_000, eff: 1_000_000, prefix: &[] }).unwrap();
        let root = node(&m, &[]);
        assert_eq!((root.actions.len(), root.actions[255]), (256, bet(25_500)));
        assert!(m.nodes.iter().any(|n| n.path == [255u8]), "ordinal 255 is representable and is materialized");
        let t = local(sizes(256));
        let e = materialize(&MaterializeInput { template: &t, starting_pot: 100_000, eff: 1_000_000, prefix: &[] });
        assert!(matches!(&e, Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("256")), "got {e:?}");
    }

    /// R1: an observed prefix may legally exceed the wager cap, so the counter that enforces the
    /// cap must be wider than the number of wagers a prefix can carry. With `river_std_v1` at pot
    /// 100 and stacks 1_000 the alternating wagers 1, 2, ..., n are each exactly the tree minimum
    /// at their step and stay below the effective stack, so a 256-wager prefix is legal input: a
    /// `u8` counter panics on it in debug and wraps in release, reoffering `Raise(640)` at the
    /// decision node although the cap of three was passed 253 wagers earlier.
    #[test]
    fn observed_prefix_wider_than_a_byte_counter_still_keeps_the_cap() {
        let prefix = |n: u32| (1..=n).map(|k| ((k as usize - 1) & 1, if k == 1 { bet(1) } else { raise(k) })).collect::<Vec<_>>();
        for n in [255u32, 256] {
            let p = prefix(n);
            let m = mat("river_std_v1", 100, 1_000, &p);
            assert_eq!(m.history.len(), n as usize, "n = {n}");
            assert_eq!((m.history[0], *m.history.last().unwrap()), (bet(1), raise(n)), "n = {n}");
            assert_eq!(m.decision_path.len(), n as usize, "n = {n}");
            // past the cap the decision node offers no wager but the all-in
            assert_eq!(node(&m, &m.decision_path).actions, vec![Action::Fold, Action::Call, allin(1_000)], "n = {n}");
        }
    }

    /// R2: `TemplateSpec`'s fields are public, so a caller can hand the materializer a menu size or
    /// threshold the wire form would refuse (`proto::MenuSize` admits only positive finite pot
    /// fractions in both serde directions; upstream requires a non-negative threshold). Those
    /// numbers are validated before any rounding or narrowing, instead of being turned into a
    /// plausible tree: `Pot(-1.0)` and `Pot(NaN)` used to give `Bet(1)`, `Pot(INFINITY)` an all-in.
    #[test]
    fn invalid_template_numbers_are_refused_before_any_rounding() {
        let run = |t: &super::TemplateSpec| materialize(&MaterializeInput { template: t, starting_pot: 100, eff: 500, prefix: &[] });
        for x in [-1.0f32, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -0.0] {
            let e = run(&local(vec![proto::MenuSize::Pot(x)]));
            assert!(matches!(&e, Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("menu size")), "{x:e} -> {e:?}");
        }
        for bad in [-0.1f32, f32::NAN, f32::INFINITY] {
            let mut t = local(vec![proto::MenuSize::Pot(0.5)]);
            t.add_allin_threshold = bad;
            let e = run(&t);
            assert!(matches!(&e, Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("add_allin_threshold")), "{bad:e} -> {e:?}");
            let mut t = local(vec![proto::MenuSize::Pot(0.5)]);
            t.force_allin_threshold = bad;
            let e = run(&t);
            assert!(matches!(&e, Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("force_allin_threshold")), "{bad:e} -> {e:?}");
        }
        // the pinned merge threshold is still refused when it is not exactly zero
        let mut t = local(vec![proto::MenuSize::Pot(0.5)]);
        t.merging_threshold = 0.1;
        assert!(matches!(&run(&t), Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("merging_threshold")), "{:?}", run(&t));
        // controls: a valid template of the same shape still materializes, and the large positive
        // size of `oversized_menu_amount_saturates_into_the_all_in` is not caught by this gate
        let m = run(&local(vec![proto::MenuSize::Pot(0.5)])).unwrap();
        assert_eq!(node(&m, &[]).actions, vec![Action::Check, bet(50)]);
        assert!(run(&local(vec![proto::MenuSize::Pot(4.694967296)])).is_ok());
        assert!(run(&local(vec![proto::MenuSize::AllIn])).is_ok());
    }

    /// R3: §4.6 allows only fold and call against an all-in, and the pinned
    /// `ActionTree::add_line_recursive` independently refuses a "Bet action after all-in", so an
    /// observed wager there must fail locally rather than be inserted into a tree the worker
    /// cannot realize. `AllIn(100)` and `Bet(200)` are the cases that normalization would have
    /// turned into a second all-in at the same amount.
    #[test]
    fn a_wager_facing_an_all_in_is_refused_but_the_all_in_prefix_still_resolves() {
        for second in [bet(100), raise(100), allin(100), bet(200), raise(50)] {
            let e = materialize(&MaterializeInput { template: Templates::get("river_std_v1").unwrap(), starting_pot: 100, eff: 100, prefix: &[(0, allin(100)), (1, second)] });
            assert!(matches!(&e, Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("all-in")), "{second:?} -> {e:?}");
        }
        // retained: the single all-in prefix reaches the opponent's fold/call decision
        let m = mat("river_std_v1", 100, 100, &[(0, allin(100))]);
        assert_eq!(m.history, vec![allin(100)]);
        assert_eq!(node(&m, &m.decision_path).actions, vec![Action::Fold, Action::Call]);
        // and a call facing the all-in is still admitted as an action: it is refused only because
        // it closes the hand, which is a different error than the wager rejection above
        let e = materialize(&MaterializeInput { template: Templates::get("river_std_v1").unwrap(), starting_pot: 100, eff: 100, prefix: &[(0, allin(100)), (1, Action::Call)] });
        assert!(matches!(&e, Err(proto::UnsupportedReason::UnsupportedHistory { reason }) if reason.contains("closes the street")), "got {e:?}");
    }
}
