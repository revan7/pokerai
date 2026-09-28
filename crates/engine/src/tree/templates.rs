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
    ///
    /// The registrations are process-global (every test of the binary sees them), so an extra may add an id and never
    /// override a base one (final review M10): a harness that shadowed `river_std_v1` would change it for every other
    /// test. Asserted, before anything is registered.
    #[cfg(any(test, feature = "test-templates"))]
    pub fn with_extra(extra: &[TemplateSpec]) {
        for t in extra {
            assert!(Self::base().iter().all(|b| b.id != t.id), "Templates::with_extra: {:?} is a base template id; an extra never overrides one", t.id);
        }
        let leaked: Vec<&'static TemplateSpec> = extra.iter().cloned().map(|t| &*Box::leak(Box::new(t))).collect();
        *EXTRA.write().unwrap() = leaked;
    }
    #[cfg(any(test, feature = "test-templates"))]
    fn extra() -> Vec<&'static TemplateSpec> { EXTRA.read().unwrap().clone() }
    #[cfg(not(any(test, feature = "test-templates")))]
    fn extra() -> Vec<&'static TemplateSpec> { Vec::new() }
    /// A base template, or an extra registration (`with_extra`, which never shares an id with a base template).
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

/// The three §13.1 T4 templates. `add`/`force` are literal spec values, `merging` is 0 and
/// donk menus follow the §4.6 rule (`None` on the root street, explicit empty after it).
/// Registration goes through Plan 2 Task 2's `Templates::with_extra`, which REPLACES the whole
/// extra set on every call (it does not merge per id): this helper is therefore the single
/// `with_extra` caller of any test binary that uses it, called once behind a `std::sync::Once`
/// (`CacheRig::new`), and no other test of such a binary registers extra templates.
#[cfg(any(test, feature = "test-templates"))]
pub fn install_cache_test_templates() {
    use proto::MenuSize::{AllIn, Pot};
    use proto::Street::{Flop, River, Turn};
    let streets = [Flop, Turn, River];
    let build = |id: &'static str, bet: Vec<MenuSize>, raise: Vec<MenuSize>, add: f32, force: f32| {
        let mut menus = BTreeMap::new();
        for s in streets {
            menus.insert(s, PlayerMenus { oop: SideMenu { bet: bet.clone(), raise: raise.clone() }, ip: SideMenu { bet: bet.clone(), raise: raise.clone() }, donk: if s == Flop { None } else { Some(vec![]) } });
        }
        TemplateSpec { id, root_street: Flop, menus, add_allin_threshold: add, force_allin_threshold: force, merging_threshold: 0.0, wager_cap: 1 }
    };
    // 1. `a`-only bets, no ordinary raises: the §10.4 terminal rake-cap pair 500 / 504.
    // 2. empty menus everywhere: isolates SPR 5.00 / 5.08 / 5.11 with identical realized menus.
    // 3. single 0.33 bet, `a`-only raises: the specified `MenuRounded{2.0}` pair 100/500 vs 20/100.
    Templates::with_extra(&[
        build("check_jam_test_v1", vec![AllIn], vec![], 0.0, 0.0),
        build("check_only_test_v1", vec![], vec![], 0.0, 0.0),
        build("menu_round_test_v1", vec![Pot(0.33)], vec![AllIn], 1.5, 0.15),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{MenuSize, PlayerMenus, SideMenu, Street};

    /// `with_extra` replaces the whole process-global extra set, so the two tests of this binary that
    /// register extras (plan 2's registry test and plan 4 Task 8's) hold this lock for their whole body:
    /// neither can clear or replace the other's registrations between its own call and its assertions.
    static EXTRA_REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    fn registry_lock() -> std::sync::MutexGuard<'static, ()> { EXTRA_REGISTRY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) }

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
        let _registry = registry_lock();
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

    /// Plan 4 Task 8: the three spec 13.1 T4 test templates are registered through `with_extra` alone, with the
    /// exact spec values, and never enlarge the production registry.
    #[test]
    fn install_cache_test_templates_registers_the_three_t4_templates() {
        let _registry = registry_lock();
        install_cache_test_templates();
        assert_eq!(Templates::base_ids().len(), 9);
        for id in ["check_jam_test_v1", "check_only_test_v1", "menu_round_test_v1"] {
            assert!(Templates::get(id).is_some());
            assert!(Templates::ids().contains(&id));
            assert!(!Templates::base_ids().contains(&id), "{id} is a test template, never a production one");
            assert_eq!(Templates::min_variant(id), None);
        }
        let menu = Templates::get("menu_round_test_v1").unwrap();
        assert_eq!((menu.root_street, menu.add_allin_threshold, menu.force_allin_threshold, menu.merging_threshold, menu.wager_cap), (Street::Flop, 1.5, 0.15, 0.0, 1));
        for street in [Street::Flop, Street::Turn, Street::River] {
            let m = &menu.menus[&street];
            assert_eq!((&m.oop.bet[..], &m.oop.raise[..]), (&[MenuSize::Pot(0.33)][..], &[MenuSize::AllIn][..]));
            assert_eq!(m.ip, m.oop);
            assert_eq!(m.donk, if street == Street::Flop { None } else { Some(vec![]) });
        }
        let jam = Templates::get("check_jam_test_v1").unwrap();
        assert_eq!((jam.add_allin_threshold, jam.force_allin_threshold, jam.wager_cap), (0.0, 0.0, 1));
        assert_eq!((&jam.menus[&Street::Turn].oop.bet[..], jam.menus[&Street::Turn].oop.raise.len()), (&[MenuSize::AllIn][..], 0));
        let check = Templates::get("check_only_test_v1").unwrap();
        assert!(check.menus.values().all(|m| m.oop.bet.is_empty() && m.oop.raise.is_empty() && m.ip.bet.is_empty() && m.ip.raise.is_empty()));
        assert_eq!(check.menus.len(), 3);
    }

    /// Final review M10: the extra registrations are process-global, so an extra that overrode a production id would
    /// change that template for every other test of the binary. `with_extra` refuses one, before it registers anything.
    #[test]
    #[should_panic(expected = "Templates::with_extra: \"river_std_v1\" is a base template id")]
    fn an_extra_template_never_overrides_a_base_id() {
        let mut shadow = Templates::get("river_std_v1").unwrap().clone();
        shadow.wager_cap = 1;
        Templates::with_extra(&[shadow]);
    }
}
