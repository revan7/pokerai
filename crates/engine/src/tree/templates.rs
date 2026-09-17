//! The solve-tree templates of spec section 10.1, plus the two test templates of section 13.2.

use proto::{MenuSize, PlayerMenus, SideMenu, Street};
use std::collections::BTreeMap;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq)]
pub struct TemplateSpec {
    pub id: &'static str,
    pub root_street: Street,
    pub menus: BTreeMap<Street, PlayerMenus>,
    pub add_allin_threshold: f32,
    pub force_allin_threshold: f32,
    pub merging_threshold: f32,
    pub wager_cap: u8,
}

fn side(bet: &[MenuSize], raise: &[MenuSize]) -> SideMenu { SideMenu { bet: bet.to_vec(), raise: raise.to_vec() } }
fn pm(bet: &[MenuSize], raise: &[MenuSize], donk: Option<Vec<MenuSize>>) -> PlayerMenus {
    PlayerMenus { oop: side(bet, raise), ip: side(bet, raise), donk }
}
const P: fn(f32) -> MenuSize = MenuSize::Pot;
const A: MenuSize = MenuSize::AllIn;

fn spec(id: &'static str, root: Street, streets: &[(Street, &[MenuSize])], raise: &[MenuSize], add: f32, force: f32, cap: u8) -> TemplateSpec {
    let mut menus = BTreeMap::new();
    for (s, bets) in streets {
        // Donk menus (§4.6): `None` on the root street, the explicit empty list on every later street.
        // A root-street `None` is legal and is never sent to the library: upstream ignores `turn_donk_sizes`
        // at a turn root because `prev_action` is `None` there, so no donk node exists to size. Only a LATER
        // street's `None` is structurally invalid input, and Tasks 8 and 13 reject exactly that (spec §13.2).
        let donk = if *s == root { None } else { Some(vec![]) };
        menus.insert(*s, pm(bets, raise, donk));
    }
    TemplateSpec { id, root_street: root, menus, add_allin_threshold: add, force_allin_threshold: force, merging_threshold: 0.0, wager_cap: cap }
}

fn build() -> Vec<TemplateSpec> {
    use Street::*;
    vec![
        spec("flop_fast_v1", Flop, &[(Flop, &[P(0.5)]), (Turn, &[P(0.5)]), (River, &[P(0.5)])], &[P(2.5)], 1.0, 0.15, 3),
        spec("flop_full_v1", Flop, &[(Flop, &[P(0.33), P(0.75)]), (Turn, &[P(0.33), P(0.75)]), (River, &[P(0.33), P(0.75)])], &[P(2.5)], 1.0, 0.15, 3),
        spec("flop_min_v1", Flop, &[(Flop, &[P(0.75)]), (Turn, &[P(0.75)]), (River, &[P(0.75)])], &[A], 1.5, 0.15, 1),
        spec("turn_std_v1", Turn, &[(Turn, &[P(0.33), P(0.75)]), (River, &[P(0.33), P(0.75)])], &[P(2.5)], 1.5, 0.15, 3),
        spec("turn_min_v1", Turn, &[(Turn, &[P(0.75)]), (River, &[P(0.75)])], &[A], 1.5, 0.15, 1),
        spec("river_std_v1", River, &[(River, &[P(0.33), P(0.75), A])], &[P(2.5)], 1.5, 0.0, 3),
        spec("river_min_v1", River, &[(River, &[P(0.75)])], &[A], 1.5, 0.0, 1),
        // test templates of §13.2 / §4.5
        spec("facing_test_v1", Flop, &[(Flop, &[P(1.0)]), (Turn, &[P(1.0)]), (River, &[P(1.0)])], &[P(2.5)], 1.0, 0.15, 3),
        TemplateSpec { id: "river_oracle_v1", root_street: River,
            menus: BTreeMap::from([(River, PlayerMenus { oop: side(&[], &[]), ip: side(&[P(1.0)], &[]), donk: None })]),
            add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1 },
    ]
}

/// Registry of the §10.1 production templates plus this plan's two §13.2 test templates.
/// `with_extra` is the single seam a downstream test harness uses to add its own templates
/// (plan 4 Task 8 registers `check_jam_test_v1`, `menu_round_test_v1`, `check_only_test_v1`);
/// it is compiled out of release builds, so the production registry is always exactly 9 ids.
pub struct Templates;

#[cfg(any(test, feature = "test-templates"))]
static EXTRA: std::sync::RwLock<Vec<&'static TemplateSpec>> = std::sync::RwLock::new(Vec::new());

impl Templates {
    fn base() -> &'static [TemplateSpec] { static T: OnceLock<Vec<TemplateSpec>> = OnceLock::new(); T.get_or_init(build) }
    /// Replaces the extra registrations with `extra` (pass `&[]` to clear). Each entry is leaked once so
    /// `get` can keep returning `&'static`; test harnesses call this a handful of times per process.
    #[cfg(any(test, feature = "test-templates"))]
    pub fn with_extra(extra: &[TemplateSpec]) {
        let leaked: Vec<&'static TemplateSpec> = extra.iter().cloned().map(|t| &*Box::leak(Box::new(t))).collect();
        *EXTRA.write().unwrap() = leaked;
    }
    #[cfg(any(test, feature = "test-templates"))]
    fn extra() -> Vec<&'static TemplateSpec> { EXTRA.read().unwrap().clone() }
    #[cfg(not(any(test, feature = "test-templates")))]
    fn extra() -> Vec<&'static TemplateSpec> { Vec::new() }
    /// Extra registrations shadow the base set, so a harness can also override a production template.
    pub fn get(id: &str) -> Option<&'static TemplateSpec> {
        Self::extra().into_iter().find(|t| t.id == id).or_else(|| Self::base().iter().find(|t| t.id == id))
    }
    /// The 9 templates compiled into this crate: §10.1's seven plus `facing_test_v1` and `river_oracle_v1`.
    /// Never affected by `with_extra`, so assertions on it cannot race a test harness.
    pub fn base_ids() -> Vec<&'static str> { Self::base().iter().map(|t| t.id).collect() }
    pub fn ids() -> Vec<&'static str> {
        let mut v = Self::base_ids();
        for t in Self::extra() { if !v.contains(&t.id) { v.push(t.id); } }
        v
    }
    /// The crash/timeout/TreeTooLarge retry template of §10.1 (`_min` of the same street); None for a `_min` or test template.
    pub fn min_variant(id: &str) -> Option<&'static str> {
        match id { "flop_fast_v1" | "flop_full_v1" => Some("flop_min_v1"), "turn_std_v1" => Some("turn_min_v1"), "river_std_v1" => Some("river_min_v1"), _ => None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{MenuSize, PlayerMenus, SideMenu, Street};
    #[test]
    fn river_std_matches_spec_10_1() {
        let t = Templates::get("river_std_v1").unwrap();
        let m = &t.menus[&Street::River];
        assert_eq!(m.oop.bet, vec![MenuSize::Pot(0.33), MenuSize::Pot(0.75), MenuSize::AllIn]);
        assert_eq!(m.oop.raise, vec![MenuSize::Pot(2.5)]);
        assert_eq!((t.add_allin_threshold, t.force_allin_threshold, t.wager_cap, t.merging_threshold), (1.5, 0.0, 3, 0.0));
        assert_eq!(Templates::get("flop_fast_v1").unwrap().menus[&Street::Turn].donk, Some(vec![]));
        assert_eq!(Templates::min_variant("turn_std_v1"), Some("turn_min_v1"));
        assert_eq!(Templates::min_variant("river_min_v1"), None);
        // the seven §10.1 ids are present by name, plus this plan's two §13.2 test templates.
        // `base_ids()` is unaffected by `with_extra`, so this assertion cannot race the test below.
        for id in ["flop_fast_v1", "flop_full_v1", "flop_min_v1", "turn_std_v1", "turn_min_v1", "river_std_v1", "river_min_v1"] {
            assert!(Templates::base_ids().contains(&id), "missing {id}");
        }
        assert_eq!(Templates::base_ids().len(), 9);
    }

    #[test]
    fn extra_templates_register_and_do_not_disturb_the_production_set() {
        // plan 4 Task 8 registers `check_jam_test_v1`, `menu_round_test_v1` and `check_only_test_v1` through this seam
        let extra = [TemplateSpec { id: "check_only_test_v1", root_street: Street::River,
            menus: std::collections::BTreeMap::from([(Street::River, PlayerMenus { oop: SideMenu { bet: vec![], raise: vec![] }, ip: SideMenu { bet: vec![], raise: vec![] }, donk: None })]),
            add_allin_threshold: 0.0, force_allin_threshold: 0.0, merging_threshold: 0.0, wager_cap: 1 }];
        Templates::with_extra(&extra);
        assert_eq!(Templates::get("check_only_test_v1").unwrap().wager_cap, 1);
        assert_eq!(Templates::get("river_std_v1").unwrap().wager_cap, 3);
        assert!(Templates::ids().contains(&"check_only_test_v1") && Templates::min_variant("check_only_test_v1").is_none());
        assert_eq!(Templates::base_ids().len(), 9, "the production registry is never enlarged by with_extra");
        Templates::with_extra(&[]);   // idempotent reset
        assert!(Templates::get("check_only_test_v1").is_none());
    }
}
