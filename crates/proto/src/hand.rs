use serde::{Deserialize, Deserializer, Serialize, Serializer};
use crate::cards::Card;
use crate::game::HandConfig;
use crate::range::Range1326;
use crate::tree::EffectiveTree;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Seat(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Position { Btn, Sb, Bb, Utg, Hj, Co }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum Street { Preflop, Flop, Turn, River }

impl Street {
    pub fn next(self) -> Option<Street> {
        match self { Street::Preflop => Some(Street::Flop), Street::Flop => Some(Street::Turn), Street::Turn => Some(Street::River), Street::River => None }
    }
    pub fn board_len(self) -> usize {
        match self { Street::Preflop => 0, Street::Flop => 3, Street::Turn => 4, Street::River => 5 }
    }
    pub fn index(self) -> usize { self as usize }
}

/// `to` = the actor's total contribution on this street.
///
/// `Serialize`/`Deserialize` are hand-written (Task: P4.T2-followup) rather than derived with
/// `#[serde(tag = "kind", rename_all = "lowercase")]` directly: serde's internally-tagged enum
/// representation deserializes by buffering the object's content via `Deserializer::deserialize_any`
/// (to find the tag before knowing which variant to decode), and bincode 1.3.3 -- the pinned cache
/// storage format, spec 10.4 -- unconditionally refuses `deserialize_any`
/// (`ErrorKind::DeserializeAnyNotSupported`, regardless of the visitor). Any `CacheEntry`
/// containing an `Action` (every `MaterializedNode.actions` does) therefore could never round-trip
/// through the cache. The fix keeps the exact tagged-map JSON form via a private `ActionJson`
/// mirror that still carries the removed `#[serde(...)]` attributes (so human-readable output is
/// byte-for-byte unchanged, including error behavior), and uses a second private `ActionBincode`
/// mirror -- an ordinary, attribute-free derive -- for every non-human-readable format, which
/// bincode encodes as a plain variant index plus fields, needing no self-description at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(tag = "kind", rename_all = "lowercase"))]
pub enum Action { Fold, Check, Call, Bet { to: u32 }, Raise { to: u32 }, AllIn { to: u32 } }

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ActionJson { Fold, Check, Call, Bet { to: u32 }, Raise { to: u32 }, AllIn { to: u32 } }

#[derive(Serialize, Deserialize)]
enum ActionBincode { Fold, Check, Call, Bet { to: u32 }, Raise { to: u32 }, AllIn { to: u32 } }

impl From<Action> for ActionJson {
    fn from(a: Action) -> Self {
        match a {
            Action::Fold => ActionJson::Fold,
            Action::Check => ActionJson::Check,
            Action::Call => ActionJson::Call,
            Action::Bet { to } => ActionJson::Bet { to },
            Action::Raise { to } => ActionJson::Raise { to },
            Action::AllIn { to } => ActionJson::AllIn { to },
        }
    }
}
impl From<ActionJson> for Action {
    fn from(a: ActionJson) -> Self {
        match a {
            ActionJson::Fold => Action::Fold,
            ActionJson::Check => Action::Check,
            ActionJson::Call => Action::Call,
            ActionJson::Bet { to } => Action::Bet { to },
            ActionJson::Raise { to } => Action::Raise { to },
            ActionJson::AllIn { to } => Action::AllIn { to },
        }
    }
}
impl From<Action> for ActionBincode {
    fn from(a: Action) -> Self {
        match a {
            Action::Fold => ActionBincode::Fold,
            Action::Check => ActionBincode::Check,
            Action::Call => ActionBincode::Call,
            Action::Bet { to } => ActionBincode::Bet { to },
            Action::Raise { to } => ActionBincode::Raise { to },
            Action::AllIn { to } => ActionBincode::AllIn { to },
        }
    }
}
impl From<ActionBincode> for Action {
    fn from(a: ActionBincode) -> Self {
        match a {
            ActionBincode::Fold => Action::Fold,
            ActionBincode::Check => Action::Check,
            ActionBincode::Call => Action::Call,
            ActionBincode::Bet { to } => Action::Bet { to },
            ActionBincode::Raise { to } => Action::Raise { to },
            ActionBincode::AllIn { to } => Action::AllIn { to },
        }
    }
}

impl Serialize for Action {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        if s.is_human_readable() { ActionJson::from(*self).serialize(s) } else { ActionBincode::from(*self).serialize(s) }
    }
}
impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Action, D::Error> {
        if d.is_human_readable() { ActionJson::deserialize(d).map(Action::from) } else { ActionBincode::deserialize(d).map(Action::from) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct TakenAction { pub seat: Seat, pub street: Street, pub action: Action, pub paid: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum CompleteReason { FoldedOut, AllInRunout, ShowdownReached }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum HandPhase {
    Betting { street: Street },
    AwaitingBoard { street: Street },
    Complete { reason: CompleteReason },
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Pot { pub amount: u32, pub eligible: Vec<Seat> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub enum LegalAction { Fold, Check, Call { cost: u32 }, Bet { min_to: u32, max_to: u32 }, Raise { min_to: u32, max_to: u32 }, AllIn { to: u32 } }

/// Per-seat vectors are indexed by `Seat.0` (length 6); an undealt seat is `folded = true`, `all_in = false`, zeros elsewhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct Derived {
    pub street: Street,
    pub to_act: Option<Seat>,
    pub pot: u32,
    pub committed_this_street: Vec<u32>,
    pub stacks_remaining: Vec<u32>,
    pub folded: Vec<bool>,
    pub all_in: Vec<bool>,
    pub facing: u32,
    pub last_full_raise: u32,
    pub pots: Vec<Pot>,
    pub legal: Vec<LegalAction>,
}

impl Default for Derived {
    fn default() -> Self {
        Derived { street: Street::Preflop, to_act: None, pot: 0, committed_this_street: vec![0; 6], stacks_remaining: vec![0; 6], folded: vec![true; 6], all_in: vec![false; 6], facing: 0, last_full_raise: 0, pots: vec![], legal: vec![] }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct HandState {
    pub hand_id: u64,
    pub hand_revision: u32,
    pub config: HandConfig,
    pub phase: HandPhase,
    pub button: Seat,
    pub hero: Seat,
    pub hero_cards: Option<[Card; 2]>,
    pub dealt: Vec<Seat>,
    pub stacks_start: Vec<u32>,
    pub board: Vec<Card>,
    pub actions: Vec<TakenAction>,
    pub derived: Derived,
}

/// Financial snapshot only (spec section 2); no ranges. `bb_chips` is carried so that `replay_root` knows the minimum bet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct StreetRootSnapshot {
    pub street: Street,
    pub board: Vec<Card>,
    pub oop: Seat,
    pub ip: Seat,
    pub pot_root: u32,
    pub stack_oop_root: u32,
    pub stack_ip_root: u32,
    pub dead_this_street: u32,
    pub projected_from: u8,
    pub history: Vec<(Seat, Action)>,
    pub bb_chips: u32,
}

/// Solve request sent to the engine/worker: financial root, both players' public ranges
/// and the effective tree to solve against (spec section 2). Hero's cards never appear here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct SolveInput {
    pub root: StreetRootSnapshot,
    pub ranges: [Range1326; 2],
    pub tree: EffectiveTree,
    pub target_bp: u16,
}

/// Admission DTO of spec section 5 step 2: no `hand_id` (the engine assigns it),
/// `stacks` in `dealt` order. `core_model::BeginHand` is the internal input that carries the id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
pub struct BeginHand {
    pub button: Seat,
    pub hero: Seat,
    pub dealt: Vec<Seat>,
    pub stacks: Vec<u32>,
    #[serde(default)]
    pub hero_cards: Option<[Card; 2]>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn action_wire_tags() {
        assert_eq!(serde_json::to_string(&Action::Check).unwrap(), r#"{"kind":"check"}"#);
        assert_eq!(serde_json::to_string(&Action::AllIn { to: 100 }).unwrap(), r#"{"kind":"allin","to":100}"#);
        assert_eq!(serde_json::from_str::<Action>(r#"{"kind":"raise","to":250}"#).unwrap(), Action::Raise { to: 250 });
        assert_eq!(serde_json::to_string(&Street::River).unwrap(), r#""river""#);
        assert_eq!(serde_json::to_string(&Position::Utg).unwrap(), r#""UTG""#);
        assert_eq!(Street::Preflop.next(), Some(Street::Flop));
        assert_eq!(Street::River.next(), None);
        assert_eq!(Street::Turn.board_len(), 4);
        let phase = HandPhase::Complete { reason: CompleteReason::AllInRunout };
        let back: HandPhase = serde_json::from_str(&serde_json::to_string(&phase).unwrap()).unwrap();
        assert_eq!(back, phase);
        let d = Derived::default();
        assert_eq!(d.stacks_remaining.len(), 6);
        assert!(d.folded.iter().all(|f| *f));
    }

    #[test]
    fn begin_hand_dto_is_id_free_and_roundtrips() {
        let dto = BeginHand { button: Seat(5), hero: Seat(0), dealt: vec![Seat(5), Seat(0), Seat(1)], stacks: vec![200, 100, 150], hero_cards: None };
        let text = serde_json::to_string(&dto).unwrap();
        assert!(!text.contains("hand_id"), "the engine assigns hand_id (spec 4.3): {text}");
        assert!(text.contains(r#""stacks":[200,100,150]"#), "{text}");
        assert_eq!(serde_json::from_str::<BeginHand>(&text).unwrap(), dto);

        // Control: the unmodified, fully valid payload deserializes successfully.
        let mut value = serde_json::to_value(&dto).unwrap();
        assert_eq!(serde_json::from_value::<BeginHand>(value.clone()).unwrap(), dto);

        // Unknown-field guard: injecting an extra field into an otherwise-valid
        // payload must be rejected, and the error must name the unknown field.
        // (A renamed-required-field case would fail deserialization even without
        // the guard, so it cannot prove the guard is load-bearing; see review R1.)
        value["hand_id"] = serde_json::json!(42);
        let err = serde_json::from_value::<BeginHand>(value)
            .expect_err("BeginHand must reject an unknown `hand_id` field")
            .to_string();
        assert!(err.contains("hand_id"), "error should mention the unknown field `hand_id`: {err}");
    }
}
