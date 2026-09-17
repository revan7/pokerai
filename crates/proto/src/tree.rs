use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use crate::hand::{Action, Street};

pub type ChipPath = Vec<Action>;
pub type OrdinalPath = Vec<u8>;
pub const RULES_VERSION: u16 = 3;

/// One menu entry: a pot fraction (bets, donks) or a multiple of the facing wager (raises),
/// or "a" = all-in only (spec 4.6, 10.1). The same type is used for bets, raises and donks so
/// that a bet menu can carry `a` (`river_std_v1` is "0.33, 0.75 + a").
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuSize { Pot(f32), AllIn }

impl Serialize for MenuSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self { MenuSize::Pot(x) => s.serialize_f32(*x), MenuSize::AllIn => s.serialize_str("a") }
    }
}

struct MenuSizeVisitor;

impl<'de> Visitor<'de> for MenuSizeVisitor {
    type Value = MenuSize;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("a positive menu size or \"a\"") }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<MenuSize, E> {
        if v.is_finite() && v > 0.0 { Ok(MenuSize::Pot(v as f32)) } else { Err(E::custom("a menu size must be positive and finite")) }
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<MenuSize, E> { self.visit_f64(v as f64) }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<MenuSize, E> { self.visit_f64(v as f64) }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<MenuSize, E> {
        if v == "a" { Ok(MenuSize::AllIn) } else { Err(E::custom(format!("unknown menu size {v:?}"))) }
    }
}

impl<'de> Deserialize<'de> for MenuSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<MenuSize, D::Error> { d.deserialize_any(MenuSizeVisitor) }
}

/// One player's menu on one street.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SideMenu { pub bet: Vec<MenuSize>, pub raise: Vec<MenuSize> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerMenus {
    pub oop: SideMenu,
    pub ip: SideMenu,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub donk: Option<Vec<MenuSize>>,
}

/// One action node of the betting skeleton (spec section 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterializedNode {
    pub path: OrdinalPath,
    pub street: Street,
    pub actor: String,
    pub actions: Vec<Action>,
    pub terminal_pots: Vec<Option<u32>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveTree {
    pub rules_version: u16,
    pub template_id: String,
    pub root_street: Street,
    pub menus: BTreeMap<Street, PlayerMenus>,
    pub add_allin_threshold: f32,
    pub force_allin_threshold: f32,
    pub merging_threshold: f32,
    pub wager_cap: u8,
    pub inserted: Vec<(ChipPath, String, Action)>,
    pub materialized: Vec<MaterializedNode>,
}

/// Resolves a wire chip path into the ordinal path of a materialized decision node (spec section 2).
pub fn resolve_chip_path(materialized: &[MaterializedNode], path: &[Action]) -> Option<OrdinalPath> {
    let index: HashMap<&[u8], &MaterializedNode> = materialized.iter().map(|n| (n.path.as_slice(), n)).collect();
    let mut node = *index.get(&[][..])?;
    let mut ordinal: OrdinalPath = Vec::with_capacity(path.len());
    for (k, action) in path.iter().enumerate() {
        let i = node.actions.iter().position(|a| a == action)?;
        ordinal.push(i as u8);
        if k + 1 == path.len() { break; }
        if node.terminal_pots.get(i)?.is_some() { return None; }
        node = *index.get(ordinal.as_slice())?;
    }
    if !index.contains_key(ordinal.as_slice()) { return None; }
    Some(ordinal)
}

#[cfg(test)]
mod tests {
    use super::*;
    pub const RIVER_ORACLE_TREE: &str = r#"{"rules_version":3,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]},{"path":[0],"street":"river","actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"terminal_pots":[100,null]},{"path":[0,1],"street":"river","actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"terminal_pots":[100,300]}]}"#;

    #[test]
    fn wire_tree_parses_and_paths_resolve() {
        let tree: EffectiveTree = serde_json::from_str(RIVER_ORACLE_TREE).unwrap();
        assert_eq!(tree.rules_version, RULES_VERSION);
        assert_eq!(tree.menus[&Street::River].ip.bet, vec![MenuSize::Pot(1.0)]);
        assert_eq!(tree.menus[&Street::River].donk, None, "donk is a field of PlayerMenus, not of SideMenu");
        let back: EffectiveTree = serde_json::from_str(&serde_json::to_string(&tree).unwrap()).unwrap();
        assert_eq!(back, tree);
        let m = &tree.materialized;
        assert_eq!(resolve_chip_path(m, &[]), Some(vec![]));
        assert_eq!(resolve_chip_path(m, &[Action::Check]), Some(vec![0]));
        assert_eq!(resolve_chip_path(m, &[Action::Check, Action::AllIn { to: 100 }]), Some(vec![0, 1]));
        assert_eq!(resolve_chip_path(m, &[Action::Check, Action::AllIn { to: 100 }, Action::Call]), None, "terminal child is not a node");
        assert_eq!(resolve_chip_path(m, &[Action::Check, Action::Check]), None);
        assert_eq!(resolve_chip_path(m, &[Action::Bet { to: 50 }]), None);
        let sizes: Vec<MenuSize> = serde_json::from_str(r#"[2.5, "a"]"#).unwrap();
        assert_eq!(sizes, vec![MenuSize::Pot(2.5), MenuSize::AllIn]);
        assert_eq!(serde_json::to_string(&sizes).unwrap(), r#"[2.5,"a"]"#);
        assert!(serde_json::from_str::<Vec<MenuSize>>(r#"["b"]"#).is_err());
        assert!(serde_json::from_str::<EffectiveTree>(&RIVER_ORACLE_TREE.replace(r#""wager_cap":1"#, r#""wager_cap":1,"extra":1"#)).is_err());
    }

    /// Spec 10.1 `river_std_v1` ("0.33, 0.75 + a") and the 13.1 T4 template with an `a`-only bet menu.
    #[test]
    fn bet_menus_carry_all_in_entries() {
        let menus: PlayerMenus = serde_json::from_str(r#"{"oop":{"bet":[0.33,0.75,"a"],"raise":[2.5]},"ip":{"bet":[0.33,0.75,"a"],"raise":[2.5]},"donk":[]}"#).unwrap();
        assert_eq!(menus.oop.bet, vec![MenuSize::Pot(0.33), MenuSize::Pot(0.75), MenuSize::AllIn]);
        assert_eq!(menus.ip.raise, vec![MenuSize::Pot(2.5)]);
        assert_eq!(menus.donk, Some(vec![]), "turn and river donk menus are the explicit empty list");
        assert_eq!(serde_json::to_string(&menus).unwrap(), r#"{"oop":{"bet":[0.33,0.75,"a"],"raise":[2.5]},"ip":{"bet":[0.33,0.75,"a"],"raise":[2.5]},"donk":[]}"#);
        let jam_only: PlayerMenus = serde_json::from_str(r#"{"oop":{"bet":["a"],"raise":["a"]},"ip":{"bet":["a"],"raise":["a"]}}"#).unwrap();
        assert_eq!(jam_only.oop.bet, vec![MenuSize::AllIn]);
        assert_eq!(jam_only.donk, None, "absent on the root street");
        assert!(serde_json::from_str::<PlayerMenus>(r#"{"oop":{"bet":[0.0],"raise":[]},"ip":{"bet":[],"raise":[]}}"#).is_err(), "a pot fraction must be positive");
    }
}
