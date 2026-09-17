use serde::de::{self, Visitor};
use serde::ser::Error as _;
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
        match self {
            // A constructed `Pot` can be built outside `Deserialize` (the type is public), so the
            // same positive-finite domain is re-checked here: emitting an invalid value as JSON
            // `null` (serde's behavior for a non-finite f32) would silently corrupt the wire form.
            MenuSize::Pot(x) if x.is_finite() && *x > 0.0 => s.serialize_f32(*x),
            MenuSize::Pot(x) => Err(S::Error::custom(format!("a menu size must be positive and finite, got {x:e}"))),
            MenuSize::AllIn => s.serialize_str("a"),
        }
    }
}

struct MenuSizeVisitor;

impl<'de> Visitor<'de> for MenuSizeVisitor {
    type Value = MenuSize;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("a positive menu size or \"a\"") }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<MenuSize, E> {
        if !(v.is_finite() && v > 0.0) {
            return Err(E::custom("a menu size must be positive and finite"));
        }
        // The wire value is f64 but the stored value is f32 (spec 4.6): a magnitude that overflows
        // f32 to infinity, or underflows to zero, must be rejected rather than silently narrowed.
        // A positive f64 that narrows to a positive f32 subnormal is preserved as-is.
        let narrowed = v as f32;
        if narrowed.is_finite() && narrowed > 0.0 {
            Ok(MenuSize::Pot(narrowed))
        } else {
            Err(E::custom("a menu size must be positive and finite after narrowing to f32"))
        }
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

/// `MenuSize`'s untagged mixed-payload wire form (`0.33` / `"a"`) cannot be derived by ts-rs;
/// this hand-written impl supplies the same union the hand-written `Serialize`/`Deserialize`
/// impls above produce. `output_path` only needs to be `Some(_)` so `Registry::visit` does not
/// skip this type -- `bindings.rs` never reads the path itself. `decl` is overridden directly
/// (ts-rs 12.0.1's default `decl` panics rather than composing `name`/`inline`); `WithoutGenerics`
/// and `OptionInnerType` are the non-generic, non-`Option` defaults per the `TS` trait's own docs.
#[cfg(feature = "typescript")]
impl ts_rs::TS for MenuSize {
    type WithoutGenerics = MenuSize;
    type OptionInnerType = Self;
    fn name(_cfg: &ts_rs::Config) -> String { "MenuSize".into() }
    fn inline(_cfg: &ts_rs::Config) -> String { r#"number | "a""#.into() }
    fn decl(_cfg: &ts_rs::Config) -> String { r#"type MenuSize = number | "a";"#.into() }
    fn output_path() -> Option<std::path::PathBuf> { Some(std::path::PathBuf::from("bindings/MenuSize.ts")) }
}

/// One player's menu on one street.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct SideMenu { pub bet: Vec<MenuSize>, pub raise: Vec<MenuSize> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct PlayerMenus {
    pub oop: SideMenu,
    pub ip: SideMenu,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub donk: Option<Vec<MenuSize>>,
}

/// One action node of the betting skeleton (spec section 2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct MaterializedNode {
    pub path: OrdinalPath,
    pub street: Street,
    pub actor: String,
    pub actions: Vec<Action>,
    pub terminal_pots: Vec<Option<u32>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct EffectiveTree {
    pub rules_version: u16,
    pub template_id: String,
    pub root_street: Street,
    pub menus: BTreeMap<Street, PlayerMenus>,
    // Spec 4.6: the tree materializer reads these at every opening/facing boundary. Each is a
    // non-negative finite magnitude, validated wide before narrowing and again on serialize
    // (review S1), so a `null` or an `inf` can never reach the materializer.
    #[serde(with = "crate::numeric::non_negative")]
    #[cfg_attr(feature = "typescript", ts(as = "f32"))]
    pub add_allin_threshold: f32,
    #[serde(with = "crate::numeric::non_negative")]
    #[cfg_attr(feature = "typescript", ts(as = "f32"))]
    pub force_allin_threshold: f32,
    #[serde(with = "crate::numeric::non_negative")]
    #[cfg_attr(feature = "typescript", ts(as = "f32"))]
    pub merging_threshold: f32,
    pub wager_cap: u8,
    pub inserted: Vec<(ChipPath, String, Action)>,
    pub materialized: Vec<MaterializedNode>,
}

/// Ordinal path -> materialized node, for repeated path resolution against one tree.
pub type MaterializedIndex<'a> = HashMap<&'a [u8], &'a MaterializedNode>;

/// Builds the ordinal-path index of a materialized tree **once**, for a caller that is about to
/// resolve many paths against it (review S2).
///
/// `resolve_chip_path` builds this itself on every call, which is Theta(materialized) per path; a
/// validator that resolves one path per exported node (up to `worker::MAX_EXPORTED_NODES` of them)
/// must hoist it instead, or the whole validation is quadratic in the tree.
pub fn index_materialized(materialized: &[MaterializedNode]) -> MaterializedIndex<'_> {
    materialized.iter().map(|n| (n.path.as_slice(), n)).collect()
}

/// Resolves a wire chip path into the ordinal path of a materialized decision node (spec section 2),
/// against an index built once by [`index_materialized`].
///
/// A path resolves only if, for every edge including the last: the action exists in the current
/// node's menu, the action's index is a representable ordinal (`u8`), its `terminal_pots` marker
/// exists and is `None` (a continuation, never a terminal child), and a materialized node exists
/// at the resulting ordinal path.
pub fn resolve_chip_path_indexed(index: &MaterializedIndex<'_>, path: &[Action]) -> Option<OrdinalPath> {
    let mut node = *index.get(&[][..])?;
    let mut ordinal: OrdinalPath = Vec::with_capacity(path.len());
    for action in path {
        let i = node.actions.iter().position(|a| a == action)?;
        // Reject before narrowing: an index that does not fit in u8 must never wrap onto another
        // action's ordinal (e.g. 256 -> 0), which could alias an unrelated existing node.
        let idx = u8::try_from(i).ok()?;
        // The marker must exist and name a continuation on every edge, including the final one:
        // a terminal edge (Some(pot)) never names a decision node, no matter what else is materialized.
        if node.terminal_pots.get(i)?.is_some() { return None; }
        ordinal.push(idx);
        node = *index.get(ordinal.as_slice())?;
    }
    Some(ordinal)
}

/// [`resolve_chip_path_indexed`] for a caller resolving a single path: builds the index, resolves,
/// throws it away. Unchanged in signature and in answers; a caller resolving many paths against the
/// same tree wants [`index_materialized`] instead.
pub fn resolve_chip_path(materialized: &[MaterializedNode], path: &[Action]) -> Option<OrdinalPath> {
    resolve_chip_path_indexed(&index_materialized(materialized), path)
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

    /// R1: the wire form must reject an f64 that overflows or underflows f32, and `Serialize`
    /// must refuse to emit an invalid constructed `Pot` value instead of silently writing `null`.
    #[test]
    fn menu_size_domain_is_validated_both_directions() {
        assert!(serde_json::from_str::<MenuSize>("1e39").is_err(), "f64->f32 overflow to infinity must be rejected");
        assert!(serde_json::from_str::<MenuSize>("1e-50").is_err(), "f64->f32 underflow to zero must be rejected");

        // valid numeric and "a" values still round trip
        assert_eq!(serde_json::from_str::<MenuSize>("2.5").unwrap(), MenuSize::Pot(2.5));
        assert_eq!(serde_json::from_str::<MenuSize>(r#""a""#).unwrap(), MenuSize::AllIn);
        assert_eq!(serde_json::to_string(&MenuSize::Pot(2.5)).unwrap(), "2.5");
        assert_eq!(serde_json::to_string(&MenuSize::AllIn).unwrap(), r#""a""#);

        // a positive representable subnormal f32 must survive, not collapse to zero
        let subnormal: MenuSize = serde_json::from_str("1e-40").unwrap();
        match subnormal {
            MenuSize::Pot(x) => {
                assert!(x > 0.0 && x.is_finite(), "subnormal must remain positive and finite, got {x:e}");
                assert!(x < 1.18e-38, "expected a subnormal magnitude, got {x:e}");
                assert!(serde_json::to_string(&MenuSize::Pot(x)).is_ok(), "a valid subnormal must still serialize");
            }
            MenuSize::AllIn => panic!("expected Pot for a numeric input"),
        }

        // invalid constructed values must be rejected on serialize, not emitted as `null`
        assert!(serde_json::to_string(&MenuSize::Pot(0.0)).is_err(), "zero must be rejected on serialize");
        assert!(serde_json::to_string(&MenuSize::Pot(-1.0)).is_err(), "negative must be rejected on serialize");
        assert!(serde_json::to_string(&MenuSize::Pot(f32::NAN)).is_err(), "NaN must be rejected on serialize");
        assert!(serde_json::to_string(&MenuSize::Pot(f32::INFINITY)).is_err(), "infinity must be rejected on serialize");
        assert!(serde_json::to_string(&MenuSize::Pot(f32::NEG_INFINITY)).is_err(), "negative infinity must be rejected on serialize");
    }

    /// R2: `resolve_chip_path` must not narrow an action index that does not fit in `u8`,
    /// and must not let that overflow alias an existing node at a wrapped-around ordinal.
    #[test]
    fn resolve_chip_path_rejects_unrepresentable_ordinal_and_accepts_255() {
        let root = MaterializedNode {
            path: vec![],
            street: Street::River,
            actor: "oop".to_string(),
            actions: (0u32..257).map(|to| Action::Bet { to }).collect(),
            terminal_pots: vec![None; 257],
        };
        // an existing node at ordinal [0] that a `256 as u8 -> 0` wraparound could wrongly alias
        let alias_target = MaterializedNode {
            path: vec![0],
            street: Street::River,
            actor: "ip".to_string(),
            actions: vec![Action::Check],
            terminal_pots: vec![None],
        };
        // the node actually addressed by ordinal 255 (the positive control)
        let at_255 = MaterializedNode {
            path: vec![255],
            street: Street::River,
            actor: "ip".to_string(),
            actions: vec![Action::Check],
            terminal_pots: vec![None],
        };
        let materialized = vec![root, alias_target, at_255];

        assert_eq!(
            resolve_chip_path(&materialized, &[Action::Bet { to: 256 }]),
            None,
            "index 256 is unrepresentable as u8 and must not alias node [0]"
        );
        assert_eq!(
            resolve_chip_path(&materialized, &[Action::Bet { to: 255 }]),
            Some(vec![255]),
            "index 255 is representable and must resolve normally"
        );
    }

    /// R3: every edge, including the last, must have a `None` (continuation) marker before its
    /// child is accepted; a terminal final edge or a missing final marker must not resolve.
    #[test]
    fn resolve_chip_path_rejects_terminal_final_edge_and_missing_marker() {
        let root_terminal = MaterializedNode {
            path: vec![],
            street: Street::River,
            actor: "oop".to_string(),
            actions: vec![Action::Check],
            terminal_pots: vec![Some(100)], // Check is terminal at the root
        };
        // a stray materialized child at [0], even though the edge into it is terminal
        let stray_child = MaterializedNode {
            path: vec![0],
            street: Street::River,
            actor: "ip".to_string(),
            actions: vec![Action::Check],
            terminal_pots: vec![None],
        };
        let materialized = vec![root_terminal, stray_child];
        assert_eq!(
            resolve_chip_path(&materialized, &[Action::Check]),
            None,
            "a terminal final edge must not resolve to a stray materialized child"
        );

        // missing final marker: the node's terminal_pots vector is shorter than its actions vector
        let root_missing_marker = MaterializedNode {
            path: vec![],
            street: Street::River,
            actor: "oop".to_string(),
            actions: vec![Action::Check],
            terminal_pots: vec![],
        };
        let materialized_missing = vec![root_missing_marker];
        assert_eq!(
            resolve_chip_path(&materialized_missing, &[Action::Check]),
            None,
            "a missing terminal marker on the final edge must not resolve"
        );

        // controls kept: valid final continuation (`[Check, AllIn]` -> `Some([0, 1])`) and the
        // empty root path (`[]` -> `Some(vec![])`) are asserted in `wire_tree_parses_and_paths_resolve`.
    }
}
