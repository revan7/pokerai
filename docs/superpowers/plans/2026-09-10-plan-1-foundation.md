# Plan 1: Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the Cargo workspace and the five foundation crates (`proto`, `core-model`, `core-ranges`, `core-iso`, `core-eval`) with every §13.1 test for them green, plus the Python `tools/` oracles that generate `fixtures/hands` (PokerKit) and `fixtures/eval` (phevaluator).

**Architecture:** `proto` is the single source of truth for every serde type of spec §4.1-4.6 (cards, config, hand state, ranges, recommendation, effective tree, worker wire messages and `validate_solution`); it depends only on `serde`/`thiserror`/`sha2`. `core-model` replays a `HandState` from its action list through one betting-round engine (`betting::Round`) shared by the full hand lifecycle (`lifecycle::simulate`) and by the HU street-root replay (`street_root::replay_root`), so the §10.2 projection check compares two runs of the same code. `core-ranges`, `core-iso` and `core-eval` are pure libraries over `proto::Range1326`; `core-eval` wraps the b-inary evaluator behind a small `Evaluator` trait and implements exact and Monte Carlo joint-disjoint equity in-house (the `pokers` crate only accepts integer-percent weights, so it cannot represent `Range1326`; it is not linked).

**Tech Stack:** Rust 2021 on `stable-x86_64-pc-windows-msvc` (1.95; the MSVC C++ tools are installed, the user's global default stays GNU), `serde 1.0` + `serde_json 1.0`, `thiserror 2.0`, `sha2 0.11`, `holdem-hand-evaluator` (git, commit `d7b2a5bba4015f96855d5cd66d21f781d3cbcf9b`, MIT), Python 3.12 with `pokerkit==0.7.5`, `phevaluator==0.6.0`, `pytest`.

**Spec:** `docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md` (revision 5), sections 2, 3.2, 3.5, 3.7, 4.1-4.6, 10.2, 13.0, 13.1. Decisions: `docs/design/2026-09-10-design-outline.md` §0b. Library facts: `docs/research/R3-libraries.md`.

## Global Constraints

- Crate, type, function, test and reason names are the spec's, verbatim (`proto`, `core-model`, `core-ranges`, `core-iso`, `core-eval`; `Card`, `Range1326`, `StreetRootSnapshot`, `validate_solution`, `hash_scaled`, `canonicalize`, `rank7`, `equity`, ...).
- Dependency direction strictly downward (§3.2): `proto` <- `core-*`. `core-iso` and `core-eval` depend on `proto` and `core-ranges`; `core-model` and `core-ranges` depend on `proto` only. No crate depends on `solver-worker`.
- Every crate of this plan is `license = "MIT OR Apache-2.0"`, `edition = "2021"`.
- Card id = `rank_index*4 + suit_index`, ranks `2..A = 0..12`, suits `c,d,h,s = 0..3` (§4.1; identical to phevaluator's and the b-inary evaluator's encoding, verified). `ComboIndex = hi*(hi-1)/2 + lo` for `lo < hi`, `0..1326`. 169-class order: 13×13 grid row-major from A down to 2; `i == j` pair, `i < j` suited, `i > j` offsuit; `class = i*13 + j`.
- Money: every wager is `u32` chips; `cap_mchips: u32` is thousandths of a chip; EV is `f32` chips (§2). Conservation invariant after every action, refund and settlement: `sum(stacks_remaining) + live_commitments + sum(unawarded pot chips) + rake_collected(0) = sum(stacks_start)` (§4.3).
- Positions (§2): clockwise from the button BTN, SB, BB, UTG, HJ, CO; preflop order UTG, HJ, CO, BTN, SB, BB; with the UTG straddle HJ, CO, BTN, SB, BB, UTG; postflop SB, BB, UTG, HJ, CO, BTN skipping folded and all-in seats; OOP = the pot-eligible player earliest in postflop order. Dealt seats 3..=6 per §4.3; two dealt seats are `FormatUnsupported{detail: "two dealt seats"}`; the straddle requires six dealt seats and `amount_chips >= 2*bb_chips`.
- Cumulative reopening (§4.3): a player who already acted may raise again only if the total raise since their last action is at least one full raise (`Derived.last_full_raise`); several short all-ins accumulate. Minimum raise-to = facing + `last_full_raise`; `last_full_raise` starts at the big blind (the straddle amount when posted) and is updated only by full raises.
- Street closure and completion (§4.3): a street closes when every pot-eligible player has matched, is all-in or folded and nobody owes a response; uncalled portions are returned, pots layered; `Complete{FoldedOut}` with one pot-eligible player, `Complete{AllInRunout}` when fewer than two pot-eligible players still have chips, `Complete{ShowdownReached}` after river closure otherwise, else `AwaitingBoard{next}`.
- Range hash (§2): after suit canonicalization and board blocking (caller's job), every weight is divided (one IEEE `f32` division) by the range's maximum weight and the 1326 `f32` bit patterns are hashed with sha256; an all-zero range hashes 1326 zero patterns.
- Canonical board (§2): the flop as an unordered set under the 24 suit permutations; turn and river appended in dealt order; among permutations giving the same canonical board, the one producing the lexicographically minimal serialized `(oop, ip)` range tuple, then the lexicographically minimal permutation.
- Worker wire (§4.5): UTF-8 JSON Lines, `#[serde(tag = "type")]`, lowercase tags, unknown fields rejected, ids as decimal strings; limits request line <= 1 MiB, result line <= 16 MiB, <= 100,000 exported nodes; matrix validation: `probs` and `ev_chips` exactly `[1326][actions.len()]`, finite, every probability in `[0, 1]`, available rows sum to `1 +- 1e-3`, unavailable rows all zero, `requested < nodes.len()`, `covered_paths[k] == nodes[k].path`, every chip path resolves against the materialized tree.
- Build flags (§3.7): `.cargo/config.toml` sets `rustflags = ["-C", "target-feature=+avx2"]` for `x86_64-pc-windows-gnu` and `x86_64-pc-windows-msvc`.
- Tests: `cargo test --workspace` green after every task; exhaustive suites behind `--features exhaustive`; Python tests with `pytest`. Commits: one per task, `feat(<crate>): ...` / `test(<crate>): ...` / `chore: ...`, trailer `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.

## Verified environment and library facts (2026-09-10)

- `rust-toolchain.toml` with `channel = "stable-x86_64-pc-windows-msvc"` is honoured by rustup on this machine; a scratch crate with the git evaluator dependency, `sha2 0.11`, `serde` internally tagged enums with `deny_unknown_fields` and `+avx2` builds and passes tests under MSVC in 6.5 s.
- `holdem-hand-evaluator`: `Hand::new()`, `Hand::from_slice(&[usize])`, `add_card(usize)`, `evaluate() -> u16` (higher is stronger; category = `rank >> 12`, 8 = straight flush), 5-7 cards only, card ids `0 = 2c .. 51 = As`. Not on crates.io: git dependency pinned to `d7b2a5bba4015f96855d5cd66d21f781d3cbcf9b` (2022-06-02, MIT).
- `pokers 0.10.0` (MIT) stores combo weights as `u8` percent (`Combo(u8, u8, u8)`, `QQ@50`); it cannot carry `Range1326` weights and is therefore not used. `rs_poker 5.1.0` stays the named contingency for the evaluator only (§3.2).
- crates.io versions: `serde 1.0.229`, `serde_json 1.0.151`, `thiserror 2.0.20`, `sha2 0.11.0`. PyPI: `pokerkit 0.7.5` (pure Python), `phevaluator 0.6.0` (`cp312-win_amd64` wheel), `pytest 9.1.1`. phevaluator ranks: lower is stronger, `1` = royal flush, `7462` = worst high card.
- PokerKit facts used by `tools/gen_fixtures.py` (probed): player index 0 posts the SB, the last index is the button; `raw_blinds_or_straddles=(1, 2, 4, 0, 0, 0)` makes index 2 the straddler and index 3 (HJ) the preflop opener; PokerKit's minimum open over a straddle is `S + min_bet` (6 at 1/2/4), so the generator sets `state.completion_betting_or_raising_amount = S` **after hole dealing** to obtain the standard `2S` (verified: min raise-to 8, then 12 after a raise to 8); PokerKit implements the cumulative short-all-in reopening rule (raise to 10, all-ins 14 and 17: the raiser cannot re-raise; 15 and 19: the raiser can, minimum 27); it returns uncalled portions at bet collection; at a fold-out it leaves the survivor's whole street bet uncollected in `bets` (the generator normalizes this, Task 13); `Mode.CASH_GAME` lets `can_fold()` return true when no wager is pending (the generator records fold as legal only when facing a wager). The one PokerKit divergence from the spec rule (a full all-in raise followed by a short all-in raise lets a player who acted in between re-raise) is detected by the generator and the hand is dropped (1 of 201 seeds).

---

## File structure

| Path | Responsibility |
|---|---|
| `Cargo.toml` | Workspace: `members = ["crates/*"]`, shared `[workspace.package]` and `[workspace.dependencies]`, profiles |
| `rust-toolchain.toml` | Pins `stable-x86_64-pc-windows-msvc` for this repo only |
| `.cargo/config.toml` | `+avx2` rustflags for both Windows targets (§3.7) |
| `.gitignore` | `target/`, `tools/.venv/`, Python caches, the optional 10M-sample oracle |
| `crates/proto/src/lib.rs` | Module list, re-exports, `PROTO_VERSION = 3` |
| `crates/proto/src/cards.rs` | `Card`, parsing/display/serde, `ComboIndex`, `combo_index`, `combo_cards`, `class_of`, `class_combos` |
| `crates/proto/src/game.rs` | `GameConfig`, `HandConfig`, `UtgStraddle`, `Rake`, `SeatConfig`, `SeatTag`, `QuickFact`, `SolverPrefs` (§4.2) |
| `crates/proto/src/hand.rs` | `Seat`, `Position`, `Street`, `Action`, `TakenAction`, `HandPhase`, `CompleteReason`, `HandState`, `Derived`, `Pot`, `LegalAction`, `StreetRootSnapshot`, `SolveInput` (§4.3) |
| `crates/proto/src/range.rs` | `Range1326` with manual serde (exactly 1326 finite weights in `[0, 1]`) |
| `crates/proto/src/recommendation.rs` | `DecisionIdentity`, `Coverage`, `ApproxReason`, `UnsupportedReason`, `Unavailable`, `ActionAdvice`, `Availability`, `EquityMethod`, `EquityEstimate`, `EquitySummary`, `PotShares`, `Assumptions`, `ExperimentalHu`, `ExploitAdvice`, `Phase`, `Recommendation`, `RecommendationEvent` (§4.4, §11) |
| `crates/proto/src/tree.rs` | `EffectiveTree`, `PlayerMenus`, `Menu`, `RaiseSize`, `ChipPath`, `OrdinalPath`, `MaterializedNode`, `resolve_chip_path` (§4.6, §2) |
| `crates/proto/src/worker.rs` | `EngineMessage`, `WorkerMessage`, `SolveRequest`, `ReadyInfo`, `NodeLock`, `AckStatus`, `Stage`, `ResultStatus`, `WorkerError`, `StreetSolution`, `NodeStrategy`, limits, `validate_solution`, `validate_locks` (§4.5) |
| `crates/proto/tests/wire_examples.rs` | §4.5 wire example round-trips, unknown tag/field rejection |
| `crates/proto/tests/validate_solution.rs` | §4.5 matrix validation tests |
| `crates/core-model/src/lib.rs` | Module list and re-exports of the §3.5 interface |
| `crates/core-model/src/error.rs` | `RulesError` (thiserror) |
| `crates/core-model/src/cards.rs` | `parse_card`, `parse_hand`, `parse_cards` (thin wrappers over `proto::Card`) |
| `crates/core-model/src/positions.rs` | Ring, position names for 3-6 dealt seats, preflop/postflop orders incl. the straddle, `validate_table` |
| `crates/core-model/src/config.rs` | `straddle_posts`, `initial_full_raise` |
| `crates/core-model/src/betting.rs` | `Round`: one betting street (legal actions, min raise, cumulative reopening, pending responses) |
| `crates/core-model/src/settlement.rs` | `refund_uncalled`, `layer_pots`, `Settlement`, `check_conservation` |
| `crates/core-model/src/lifecycle.rs` | `simulate`: replays `HandState.actions` + board through `Round`, closes streets, settles, assigns `HandPhase`, builds `Derived` |
| `crates/core-model/src/state.rs` | `BeginHand`, `begin_hand`, `apply_action`, `set_board`, `set_hero_cards`, `derive`, `settle_pots`, `is_decision_point`, `abandon` |
| `crates/core-model/src/street_root.rs` | `RootError`, `street_root`, `replay_root`, the §10.2 projection rule |
| `crates/core-model/tests/*.rs` | Every §13.1 core-model row (one file per row group) |
| `crates/core-ranges/src/lib.rs` | `RangeError`, `parse_range`, `range_to_string`, `expand_169`, `class_name`, `block_public`, `hero_conditioned`, `mass`, `hash_scaled` |
| `crates/core-ranges/src/parse.rs` | Pio grammar tokenizer and class expansion |
| `crates/core-ranges/tests/ranges.rs` | §13.1 core-ranges rows |
| `crates/core-iso/src/lib.rs` | `SuitPerm`, `CanonicalBoard`, `ALL_PERMS`, `apply`, `apply_range`, `inverse`, `canonicalize`, `orbit_size` |
| `crates/core-iso/tests/iso.rs` | §13.1 core-iso rows |
| `crates/core-eval/src/lib.rs` | Re-exports; `rank7`, `rank5`, `rank` |
| `crates/core-eval/src/evaluator.rs` | `Evaluator` trait, `BinaryEvaluator` |
| `crates/core-eval/src/equity.rs` | Request/result types, `equity`, `exact_cost`, exact enumeration, `per_combo_equity`, `terminal_payoff` |
| `crates/core-eval/src/mc.rs` | `Xoshiro256` PRNG, joint disjoint sampling, Monte Carlo |
| `crates/core-eval/tests/{oracle,equity}.rs` | §13.1 core-eval rows |
| `tools/pyproject.toml`, `tools/requirements.txt`, `tools/tests/conftest.py` | Python project metadata, pinned dependencies, pytest path setup |
| `tools/gen_fixtures.py`, `tools/tests/test_gen_fixtures.py` | 200 PokerKit hands -> `fixtures/hands/h0001..h0200.json` |
| `tools/gen_eval_oracle.py`, `tools/tests/test_gen_eval_oracle.py` | phevaluator oracle -> `fixtures/eval/phevaluator_5card.bin`, `fixtures/eval/phevaluator_7card_200k.bin` |
| `fixtures/hands/*.json`, `fixtures/eval/*.bin` | Committed fixtures (§13.0) |

---

### Task 1: Workspace skeleton

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, `.gitignore`, `crates/proto/Cargo.toml`, `crates/proto/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: the workspace every later crate joins by creating a directory under `crates/`; `proto::PROTO_VERSION: u16 = 3`.

- [ ] **Step 1: Write the workspace files**

`Cargo.toml`:
```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT OR Apache-2.0"
rust-version = "1.95"

[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror = "2.0"
sha2 = "0.11"
holdem-hand-evaluator = { git = "https://github.com/b-inary/holdem-hand-evaluator", rev = "d7b2a5bba4015f96855d5cd66d21f781d3cbcf9b" }

[profile.dev]
opt-level = 1

[profile.dev.package.holdem-hand-evaluator]
opt-level = 3

[profile.release]
opt-level = 3
```

`rust-toolchain.toml`:
```toml
[toolchain]
channel = "stable-x86_64-pc-windows-msvc"
```

`.cargo/config.toml`:
```toml
[target.x86_64-pc-windows-gnu]
rustflags = ["-C", "target-feature=+avx2"]

[target.x86_64-pc-windows-msvc]
rustflags = ["-C", "target-feature=+avx2"]
```

`.gitignore`:
```
/target/
**/*.rs.bk
tools/.venv/
__pycache__/
.pytest_cache/
fixtures/eval/phevaluator_7card_10m.bin
```

`crates/proto/Cargo.toml`:
```toml
[package]
name = "proto"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Shared serde types of the PokerAI assistant (spec section 4)"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
```

`crates/proto/src/lib.rs`:
```rust
//! Single source of truth for every serde type shared by the UI, the engine and the worker.

/// Wire protocol version reported in `ready` (spec section 4.5).
pub const PROTO_VERSION: u16 = 3;

#[cfg(test)]
mod tests {
    #[test]
    fn proto_version_is_three() {
        assert_eq!(super::PROTO_VERSION, 3);
        assert!(cfg!(target_feature = "avx2"), "avx2 must be enabled by .cargo/config.toml");
    }
}
```

- [ ] **Step 2: Build and test**

Run: `rustup show active-toolchain` then `cargo test --workspace`
Expected: the active toolchain line names `stable-x86_64-pc-windows-msvc (overridden by ... rust-toolchain.toml)`; 1 test passes.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml .cargo/config.toml .gitignore crates/proto
git commit -m "chore: workspace skeleton" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: `proto` cards and combos

**Files:**
- Create: `crates/proto/src/cards.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Produces: `Card(pub u8)` with `Card::new(rank: u8, suit: u8)`, `rank()`, `suit()`, `Card::all()`, `FromStr`, `Display` ("As"), serde as a two-character string; `CardParseError`; `ComboIndex = u16`; `COMBOS = 1326`; `CLASSES = 169`; `combo_index(a: Card, b: Card) -> ComboIndex`; `combo_cards(i: ComboIndex) -> [Card; 2]` (lo, hi); `class_of(i: ComboIndex) -> u8`; `class_combos(class: u8) -> Vec<ComboIndex>`.

- [ ] **Step 1: Write the failing tests** (inside `cards.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_ids_match_spec() {
        assert_eq!("2c".parse::<Card>().unwrap(), Card(0));
        assert_eq!("As".parse::<Card>().unwrap(), Card(51));
        assert_eq!("Td".parse::<Card>().unwrap(), Card(8 * 4 + 1));
        assert_eq!(Card(51).to_string(), "As");
        assert!("1s".parse::<Card>().is_err());
        assert!("Ax".parse::<Card>().is_err());
        assert!("Ass".parse::<Card>().is_err());
        assert_eq!(serde_json::to_string(&Card(43)).unwrap(), "\"Qs\"");
        assert_eq!(serde_json::from_str::<Card>("\"Qs\"").unwrap(), Card(43));
        assert!(serde_json::from_str::<Card>("\"Zz\"").is_err());
    }

    #[test]
    fn combo_index_bijection() {
        let mut seen = vec![false; COMBOS];
        for hi in 1..52u8 {
            for lo in 0..hi {
                let i = combo_index(Card(hi), Card(lo));
                assert_eq!(i, combo_index(Card(lo), Card(hi)));
                assert_eq!(combo_cards(i), [Card(lo), Card(hi)]);
                assert!(!seen[i as usize]);
                seen[i as usize] = true;
            }
        }
        assert!(seen.iter().all(|s| *s));
    }

    #[test]
    fn class_order_spec() {
        let aa = combo_index("As".parse().unwrap(), "Ah".parse().unwrap());
        let aks = combo_index("As".parse().unwrap(), "Ks".parse().unwrap());
        let ako = combo_index("As".parse().unwrap(), "Kh".parse().unwrap());
        let s22 = combo_index("2c".parse().unwrap(), "2d".parse().unwrap());
        assert_eq!(class_of(aa), 0);
        assert_eq!(class_of(aks), 1);
        assert_eq!(class_of(ako), 13);
        assert_eq!(class_of(s22), 168);
        let mut counts = [0usize; CLASSES];
        for i in 0..COMBOS as u16 { counts[class_of(i) as usize] += 1; }
        for c in 0..CLASSES as u8 {
            let (i, j) = (c / 13, c % 13);
            let expected = if i == j { 6 } else if i < j { 4 } else { 12 };
            assert_eq!(counts[c as usize], expected, "class {c}");
            assert_eq!(class_combos(c).len(), expected);
            assert!(class_combos(c).iter().all(|k| class_of(*k) == c));
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto cards`
Expected: compile error (module missing).

- [ ] **Step 3: Implement `cards.rs`**

```rust
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;

/// id = rank_index*4 + suit_index; ranks 2..A = 0..12; suits c,d,h,s = 0..3 (spec 4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Card(pub u8);

pub const RANK_CHARS: &[u8; 13] = b"23456789TJQKA";
pub const SUIT_CHARS: &[u8; 4] = b"cdhs";
pub const COMBOS: usize = 1326;
pub const CLASSES: usize = 169;
pub type ComboIndex = u16;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CardParseError {
    #[error("card text {0:?} must be exactly two characters")]
    Length(String),
    #[error("unknown rank character {0:?}")]
    Rank(char),
    #[error("unknown suit character {0:?}")]
    Suit(char),
    #[error("card id {0} is outside 0..52")]
    Id(u8),
    #[error("duplicate card {0}")]
    Duplicate(Card),
}

impl Card {
    pub fn new(rank: u8, suit: u8) -> Card {
        debug_assert!(rank < 13 && suit < 4);
        Card(rank * 4 + suit)
    }
    pub fn rank(self) -> u8 { self.0 / 4 }
    pub fn suit(self) -> u8 { self.0 % 4 }
    pub fn all() -> impl Iterator<Item = Card> { (0..52u8).map(Card) }
    pub fn checked(id: u8) -> Result<Card, CardParseError> {
        if id < 52 { Ok(Card(id)) } else { Err(CardParseError::Id(id)) }
    }
}

impl FromStr for Card {
    type Err = CardParseError;
    fn from_str(s: &str) -> Result<Card, CardParseError> {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() != 2 { return Err(CardParseError::Length(s.to_string())); }
        let r = chars[0].to_ascii_uppercase();
        let rank = RANK_CHARS.iter().position(|c| *c as char == r).ok_or(CardParseError::Rank(chars[0]))?;
        let su = chars[1].to_ascii_lowercase();
        let suit = SUIT_CHARS.iter().position(|c| *c as char == su).ok_or(CardParseError::Suit(chars[1]))?;
        Ok(Card::new(rank as u8, suit as u8))
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", RANK_CHARS[self.rank() as usize] as char, SUIT_CHARS[self.suit() as usize] as char)
    }
}

impl Serialize for Card {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> { s.serialize_str(&self.to_string()) }
}

impl<'de> Deserialize<'de> for Card {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Card, D::Error> {
        let text = String::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// `idx = hi*(hi-1)/2 + lo` for card ids `lo < hi` (spec 4.1).
pub fn combo_index(a: Card, b: Card) -> ComboIndex {
    debug_assert!(a != b);
    let (lo, hi) = if a.0 < b.0 { (a.0 as u16, b.0 as u16) } else { (b.0 as u16, a.0 as u16) };
    hi * (hi - 1) / 2 + lo
}

/// Inverse of `combo_index`: returns `[lo, hi]`.
pub fn combo_cards(i: ComboIndex) -> [Card; 2] {
    debug_assert!((i as usize) < COMBOS);
    let mut hi: u16 = 1;
    while (hi + 1) * hi / 2 <= i { hi += 1; }
    let lo = i - hi * (hi - 1) / 2;
    [Card(lo as u8), Card(hi as u8)]
}

/// 169-class of a combo: row-major grid from A down to 2, `i == j` pair, `i < j` suited, `i > j` offsuit.
pub fn class_of(i: ComboIndex) -> u8 {
    let [lo, hi] = combo_cards(i);
    let (r_hi, r_lo) = (hi.rank().max(lo.rank()), hi.rank().min(lo.rank()));
    let (row_hi, row_lo) = (12 - r_hi, 12 - r_lo); // row_hi <= row_lo
    if r_hi == r_lo { row_hi * 13 + row_hi }
    else if hi.suit() == lo.suit() { row_hi * 13 + row_lo }
    else { row_lo * 13 + row_hi }
}

/// Every combo of a class, ascending by combo index.
pub fn class_combos(class: u8) -> Vec<ComboIndex> {
    (0..COMBOS as u16).filter(|i| class_of(*i) == class).collect()
}
```

Add to `lib.rs`: `pub mod cards; pub use cards::*;`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p proto`
Expected: 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): cards, combo index and 169-class order" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 3: `proto` game config and hand-state types

**Files:**
- Create: `crates/proto/src/game.rs`, `crates/proto/src/hand.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Consumes: `Card` (Task 2).
- Produces (all `Serialize + Deserialize + Clone + Debug + PartialEq`): `GameConfig`, `HandConfig` (+ `HandConfig::from_game(&GameConfig)`), `UtgStraddle { amount_chips: u32 }`, `Rake::{PotRake { rate: f32, cap_mchips: u32, no_flop_no_drop: bool }, TimeCharge}`, `SeatConfig`, `SeatTag`, `QuickFact`, `SolverPrefs { threads: u8, target_bp: u16, flop_budget_s: u8 }` (`Default` = 16/50/10); `Seat(pub u8)`, `Position::{Btn, Sb, Bb, Utg, Hj, Co}` (serde "BTN".."CO"), `Street::{Preflop, Flop, Turn, River}` (serde lowercase) with `Street::next(self) -> Option<Street>`, `Street::board_len(self) -> usize`, `Street::index(self) -> usize`; `Action::{Fold, Check, Call, Bet { to }, Raise { to }, AllIn { to }}` (serde `tag = "kind"`, lowercase: `{"kind":"allin","to":100}`); `TakenAction { seat, street, action, paid }`; `HandPhase::{Betting { street }, AwaitingBoard { street }, Complete { reason }, Abandoned}`; `CompleteReason::{FoldedOut, AllInRunout, ShowdownReached}`; `HandState`; `Derived` (`Default`); `Pot { amount: u32, eligible: Vec<Seat> }`; `LegalAction::{Fold, Check, Call { cost }, Bet { min_to, max_to }, Raise { min_to, max_to }, AllIn { to }}`; `StreetRootSnapshot` (spec fields plus `bb_chips: u32`, needed by `replay_root` for the minimum bet; the worker ignores it).

- [ ] **Step 1: Write the failing test** (`crates/proto/src/hand.rs`, bottom)

```rust
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
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto hand`
Expected: compile error.

- [ ] **Step 3: Implement `game.rs`**

```rust
use serde::{Deserialize, Serialize};
use crate::hand::Seat;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UtgStraddle { pub amount_chips: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Rake {
    PotRake { rate: f32, cap_mchips: u32, no_flop_no_drop: bool },
    TimeCharge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeatTag { Unknown, Nit, Tag, LoosePassive, CallingStation, Lag, Maniac }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickFact { NeverFoldsRiver, RarelyBluffs, LimpsALot, Over3bets, FoldsToPressure }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeatConfig { pub seat: Seat, pub tag: Option<SeatTag>, pub facts: Vec<QuickFact> }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolverPrefs { pub threads: u8, pub target_bp: u16, pub flop_budget_s: u8 }

impl Default for SolverPrefs {
    fn default() -> Self { SolverPrefs { threads: 16, target_bp: 50, flop_budget_s: 10 } }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameConfig {
    pub config_revision: u32,
    pub chip_label: String,
    pub sb_chips: u32,
    pub bb_chips: u32,
    pub straddle: Option<UtgStraddle>,
    pub rake: Rake,
    pub seats: Vec<SeatConfig>,
    pub solver: SolverPrefs,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandConfig {
    pub config_revision: u32,
    pub sb_chips: u32,
    pub bb_chips: u32,
    pub straddle: Option<UtgStraddle>,
    pub rake: Rake,
    pub chip_label: String,
}

impl HandConfig {
    pub fn from_game(g: &GameConfig) -> HandConfig {
        HandConfig { config_revision: g.config_revision, sb_chips: g.sb_chips, bb_chips: g.bb_chips, straddle: g.straddle, rake: g.rake, chip_label: g.chip_label.clone() }
    }
}
```

- [ ] **Step 4: Implement `hand.rs`**

```rust
use serde::{Deserialize, Serialize};
use crate::cards::Card;
use crate::game::HandConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seat(pub u8);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Position { Btn, Sb, Bb, Utg, Hj, Co }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Action { Fold, Check, Call, Bet { to: u32 }, Raise { to: u32 }, AllIn { to: u32 } }

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TakenAction { pub seat: Seat, pub street: Street, pub action: Action, pub paid: u32 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompleteReason { FoldedOut, AllInRunout, ShowdownReached }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum HandPhase {
    Betting { street: Street },
    AwaitingBoard { street: Street },
    Complete { reason: CompleteReason },
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pot { pub amount: u32, pub eligible: Vec<Seat> }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LegalAction { Fold, Check, Call { cost: u32 }, Bet { min_to: u32, max_to: u32 }, Raise { min_to: u32, max_to: u32 }, AllIn { to: u32 } }

/// Per-seat vectors are indexed by `Seat.0` (length 6); an undealt seat is `folded = true`, `all_in = false`, zeros elsewhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
```

Add to `lib.rs`: `pub mod game; pub mod hand; pub use game::*; pub use hand::*;`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p proto`
Expected: 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): game config and hand state types" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: `proto::Range1326`

**Files:**
- Create: `crates/proto/src/range.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Produces: `Range1326(pub [f32; 1326])` with `Clone`, `PartialEq`, custom `Debug`, serde as a JSON array of exactly 1326 finite numbers in `[0, 1]` (rejected otherwise at deserialization); `Range1326::zero()`, `Range1326::uniform()`, `Range1326::from_fn(impl FnMut(ComboIndex) -> f32)`, `get(i)`, `set(i, w)`.

- [ ] **Step 1: Write the failing test** (bottom of `range.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_serde_validates_shape_and_domain() {
        let r = Range1326::from_fn(|i| if i % 7 == 0 { 0.5 } else { 0.0 });
        let text = serde_json::to_string(&r).unwrap();
        let back: Range1326 = serde_json::from_str(&text).unwrap();
        assert_eq!(back, r);
        let short = serde_json::to_string(&vec![0.0f32; 1325]).unwrap();
        assert!(serde_json::from_str::<Range1326>(&short).is_err());
        let long = serde_json::to_string(&vec![0.0f32; 1327]).unwrap();
        assert!(serde_json::from_str::<Range1326>(&long).is_err());
        let mut v = vec![0.0f32; 1326]; v[3] = 1.5;
        assert!(serde_json::from_str::<Range1326>(&serde_json::to_string(&v).unwrap()).is_err());
        v[3] = -0.1;
        assert!(serde_json::from_str::<Range1326>(&serde_json::to_string(&v).unwrap()).is_err());
        assert!(serde_json::from_str::<Range1326>("[null]").is_err());
        assert_eq!(Range1326::uniform().0.iter().sum::<f32>(), 1326.0);
        assert_eq!(format!("{:?}", Range1326::zero()), "Range1326(support=0, mass=0)");
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto range`
Expected: compile error.

- [ ] **Step 3: Implement `range.rs`**

```rust
use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use crate::cards::{ComboIndex, COMBOS};

/// Weights in `[0, 1]` indexed by combo index (spec 4.1).
#[derive(Clone, PartialEq)]
pub struct Range1326(pub [f32; COMBOS]);

impl Range1326 {
    pub fn zero() -> Range1326 { Range1326([0.0; COMBOS]) }
    pub fn uniform() -> Range1326 { Range1326([1.0; COMBOS]) }
    pub fn from_fn(mut f: impl FnMut(ComboIndex) -> f32) -> Range1326 {
        let mut r = Range1326::zero();
        for i in 0..COMBOS { r.0[i] = f(i as ComboIndex); }
        r
    }
    pub fn get(&self, i: ComboIndex) -> f32 { self.0[i as usize] }
    pub fn set(&mut self, i: ComboIndex, w: f32) { self.0[i as usize] = w; }
}

impl fmt::Debug for Range1326 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let support = self.0.iter().filter(|w| **w > 0.0).count();
        let mass: f64 = self.0.iter().map(|w| *w as f64).sum();
        write!(f, "Range1326(support={support}, mass={mass})")
    }
}

impl Serialize for Range1326 {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(COMBOS))?;
        for w in self.0.iter() { seq.serialize_element(w)?; }
        seq.end()
    }
}

struct RangeVisitor;

impl<'de> Visitor<'de> for RangeVisitor {
    type Value = Range1326;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("an array of exactly 1326 finite numbers in [0, 1]") }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Range1326, A::Error> {
        let mut out = [0f32; COMBOS];
        for (i, slot) in out.iter_mut().enumerate() {
            let w: f32 = seq.next_element()?.ok_or_else(|| de::Error::invalid_length(i, &self))?;
            if !w.is_finite() || !(0.0..=1.0).contains(&w) {
                return Err(de::Error::custom(format!("weight {w} at combo {i} is outside [0, 1]")));
            }
            *slot = w;
        }
        if seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom("range has more than 1326 entries"));
        }
        Ok(Range1326(out))
    }
}

impl<'de> Deserialize<'de> for Range1326 {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Range1326, D::Error> { d.deserialize_seq(RangeVisitor) }
}
```

Add to `lib.rs`: `pub mod range; pub use range::*;`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p proto`
Expected: 6 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): Range1326 with validated serde" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: `proto` recommendation, coverage and event types

**Files:**
- Create: `crates/proto/src/recommendation.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Consumes: `Seat`, `Street`, `Action`, `LegalAction` (Task 3).
- Produces: every type of spec §4.4 verbatim plus `ExploitAdvice` (§11) and `Phase`: `DecisionIdentity`, `Coverage`, `ApproxReason`, `UnsupportedReason`, `Unavailable`, `ActionAdvice`, `Availability`, `EquityMethod`, `EquityEstimate`, `EquitySummary`, `PotShares` (+ `POT_SHARES_POPULATION`), `Assumptions`, `ExperimentalHu` (+ `EXPERIMENTAL_NOTE`), `ExploitAdvice`, `Phase`, `Recommendation`, `RecommendationEvent`. All enums with payloads use `#[serde(tag = "kind")]`; reason names are the spec's.

- [ ] **Step 1: Write the failing test** (bottom of `recommendation.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reasons_and_events_roundtrip() {
        let cov = Coverage::Approximate { reasons: vec![
            ApproxReason::BetTranslation { street: Street::Flop, seat: Seat(2), observed_pct: 0.73, mapped: vec![(0.5, 0.468), (1.0, 0.532)], deviation: 0.23, prominent: true },
            ApproxReason::MultiwayStreetRoot { folded_this_street: 1, dead_this_street: 50 },
            ApproxReason::ChartRounded,
        ] };
        let text = serde_json::to_string(&cov).unwrap();
        assert!(text.contains(r#""kind":"BetTranslation""#));
        assert!(text.contains(r#""kind":"ChartRounded""#));
        let back: Coverage = serde_json::from_str(&text).unwrap();
        assert_eq!(back, cov);
        let uns = Coverage::Unsupported { reason: UnsupportedReason::UnsupportedHistory { reason: "multiway street root not reproducible at step 1".into() }, partial: vec![] };
        assert_eq!(serde_json::from_str::<Coverage>(&serde_json::to_string(&uns).unwrap()).unwrap(), uns);
        let id = DecisionIdentity { hand_id: 1, hand_revision: 7, decision_id: 3, config_revision: 1, model_revision: 0 };
        let ev = RecommendationEvent::NoDecision { identity: id.clone(), reason: "hero all-in".into() };
        assert_eq!(serde_json::from_str::<RecommendationEvent>(&serde_json::to_string(&ev).unwrap()).unwrap(), ev);
        let est = EquityEstimate { value: Some(0.25), availability: Availability::Ready, method: Some(EquityMethod::MonteCarlo { samples: 100_000, std_err: 0.0014 }) };
        assert_eq!(serde_json::from_str::<EquityEstimate>(&serde_json::to_string(&est).unwrap()).unwrap(), est);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto recommendation`
Expected: compile error.

- [ ] **Step 3: Implement `recommendation.rs`**

```rust
use serde::{Deserialize, Serialize};
use crate::hand::{Action, LegalAction, Seat, Street};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DecisionIdentity { pub hand_id: u64, pub hand_revision: u32, pub decision_id: u64, pub config_revision: u32, pub model_revision: u32 }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ApproxReason {
    BetTranslation { street: Street, seat: Seat, observed_pct: f32, mapped: Vec<(f32, f32)>, deviation: f32, prominent: bool },
    DepthBucket { seat: Seat, actual_bb: f32, used_bb: u16, prominent: bool },
    AsymmetricStacks { stacks_bb: Vec<f32> },
    RakeProfileMapped { actual: String, used: String },
    StraddleMapped { posts: [f32; 3] },
    ShortHandedMapped { dealt: u8 },
    DeadlineBestSoFar { reached_bp: u16, target_bp: u16 },
    ChartRounded,
    EvReferenceUnverified,
    UnconditionedPriorStreet { street: Street, seat: Seat, cause: String },
    UnconditionedCurrentStreet,
    MultiwayStreetRoot { folded_this_street: u8, dead_this_street: u32 },
    SprBucketed { actual: f32, used: f32 },
    MenuRounded { max_delta_pct: f32 },
    BranchResidual { seat: Seat, residual_mass_pct: f32, cause: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum UnsupportedReason {
    MultiwayEv { pot_eligible: u8 },
    MissingPreflopNode { key: String },
    HeroComboOutOfSupport,
    EngineError { message: String, retryable: bool },
    TreeTooLarge { estimate_bytes: u64 },
    DeadlineExceeded { stage: String },
    InvalidRanges,
    UnsupportedHistory { reason: String },
    FormatUnsupported { detail: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Coverage {
    Exact,
    Approximate { reasons: Vec<ApproxReason> },
    Unsupported { reason: UnsupportedReason, partial: Vec<ApproxReason> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Unavailable {
    NotInMenu, NoEvReference, HeroOutOfSupport, MovedProbability { from: Action }, ChartNoEv, NotEvaluated, Pending,
    BranchSupportIncomplete { covered_posterior: f32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionAdvice { pub action: Action, pub frequency: Option<f32>, pub ev_bb: Option<f32>, pub unavailable: Option<Unavailable>, pub headline: bool }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Availability { Ready, Pending, Unavailable { reason: String } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum EquityMethod { Exact, MonteCarlo { samples: u32, std_err: f32 } }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquityEstimate { pub value: Option<f32>, pub availability: Availability, pub method: Option<EquityMethod> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PotShares { pub pot_index: u8, pub population: String, pub shares: Vec<(Seat, EquityEstimate)> }

pub const POT_SHARES_POPULATION: &str = "hero combo fixed; opponents jointly sampled, disjoint, from hero-conditioned public ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct EquitySummary { pub hero_combo_vs_each: Vec<(Seat, EquityEstimate)>, pub hero_range_vs_each: Vec<(Seat, EquityEstimate)>, pub per_pot_shares: Vec<PotShares> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assumptions {
    pub ranges_used: Vec<(Seat, String, f32)>, pub tree_signature: String, pub template_id: String,
    pub source: String, pub source_accuracy: String, pub source_granularity: String,
    pub target_bp: u16, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub cache: String,
    pub translations: Vec<ApproxReason>, pub mappings: Vec<ApproxReason>, pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExperimentalHu {
    pub opponent: Seat, pub hero_role: String, pub pot: u32, pub stack: u32, pub template_id: String,
    pub ranges_used: [(Seat, String, f32); 2], pub actions: Vec<ActionAdvice>, pub reached_bp: Option<u16>, pub elapsed_ms: u32, pub note: String,
}

pub const EXPERIMENTAL_NOTE: &str = "experimental, not solved: synthetic root, empty history, unconditioned ranges";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExploitAdvice { pub alpha: f32, pub model_revision: u32, pub model_summary: String, pub gto_action: Action, pub exploit_action: Action, pub ev_delta_bb: Option<f32>, pub locked_nodes: u16 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase { Fast, Provisional, Final }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub identity: DecisionIdentity, pub phase: Phase, pub coverage: Coverage, pub legal: Vec<LegalAction>,
    pub actions: Vec<ActionAdvice>, pub unresolved_mass: f32, pub range_mix: Option<Vec<(Action, f32)>>,
    pub equity: EquitySummary, pub assumptions: Assumptions, pub experimental: Option<ExperimentalHu>, pub exploit: Option<ExploitAdvice>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum RecommendationEvent {
    Fast(Recommendation),
    Equity { identity: DecisionIdentity, equity: EquitySummary },
    Progress { identity: DecisionIdentity, stage: String, iterations: u32, exploitability_pct: Option<f32>, elapsed_ms: u32 },
    Provisional(Recommendation),
    Final(Recommendation),
    NoDecision { identity: DecisionIdentity, reason: String },
}
```

Add to `lib.rs`: `pub mod recommendation; pub use recommendation::*;`.

- [ ] **Step 4: Run tests**

Run: `cargo test -p proto`
Expected: 7 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): recommendation, coverage and event types" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 6: `proto` effective tree, materialized nodes and chip-path resolution

**Files:**
- Create: `crates/proto/src/tree.rs`
- Modify: `crates/proto/src/lib.rs`

**Interfaces:**
- Consumes: `Action`, `Street` (Task 3).
- Produces: `ChipPath = Vec<Action>`, `OrdinalPath = Vec<u8>`, `RaiseSize::{Mult(f32), AllInOnly}` (wire: number or `"a"`), `Menu { bet: Vec<f32>, raise: Vec<RaiseSize> }`, `PlayerMenus { oop: Menu, ip: Menu, donk: Option<Vec<f32>> }` (`donk` absent on the root street, `Some(vec![])` on later streets in phase 1; the worker of plan 2 enforces that), `MaterializedNode { path: OrdinalPath, street: Street, actor: String, actions: Vec<Action>, terminal_pots: Vec<Option<u32>> }`, `EffectiveTree` (spec fields verbatim, `inserted: Vec<(ChipPath, String, Action)>`), `RULES_VERSION: u16 = 3`, `resolve_chip_path(materialized: &[MaterializedNode], path: &[Action]) -> Option<OrdinalPath>` (spec §2: a chip path resolves only if every action exists in its node's menu in order, no step continues past a terminal child, and the final path names a materialized node).

- [ ] **Step 1: Write the failing test** (bottom of `tree.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    pub const RIVER_ORACLE_TREE: &str = r#"{"rules_version":3,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]},{"path":[0],"street":"river","actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"terminal_pots":[100,null]},{"path":[0,1],"street":"river","actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"terminal_pots":[100,300]}]}"#;

    #[test]
    fn wire_tree_parses_and_paths_resolve() {
        let tree: EffectiveTree = serde_json::from_str(RIVER_ORACLE_TREE).unwrap();
        assert_eq!(tree.rules_version, RULES_VERSION);
        assert_eq!(tree.menus[&Street::River].ip.bet, vec![1.0]);
        assert_eq!(tree.menus[&Street::River].oop.donk, None);
        let back: EffectiveTree = serde_json::from_str(&serde_json::to_string(&tree).unwrap()).unwrap();
        assert_eq!(back, tree);
        let m = &tree.materialized;
        assert_eq!(resolve_chip_path(m, &[]), Some(vec![]));
        assert_eq!(resolve_chip_path(m, &[Action::Check]), Some(vec![0]));
        assert_eq!(resolve_chip_path(m, &[Action::Check, Action::AllIn { to: 100 }]), Some(vec![0, 1]));
        assert_eq!(resolve_chip_path(m, &[Action::Check, Action::AllIn { to: 100 }, Action::Call]), None, "terminal child is not a node");
        assert_eq!(resolve_chip_path(m, &[Action::Check, Action::Check]), None);
        assert_eq!(resolve_chip_path(m, &[Action::Bet { to: 50 }]), None);
        let sizes: Vec<RaiseSize> = serde_json::from_str(r#"[2.5, "a"]"#).unwrap();
        assert_eq!(sizes, vec![RaiseSize::Mult(2.5), RaiseSize::AllInOnly]);
        assert_eq!(serde_json::to_string(&sizes).unwrap(), r#"[2.5,"a"]"#);
        assert!(serde_json::from_str::<Vec<RaiseSize>>(r#"["b"]"#).is_err());
        assert!(serde_json::from_str::<EffectiveTree>(&RIVER_ORACLE_TREE.replace(r#""wager_cap":1"#, r#""wager_cap":1,"extra":1"#)).is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto tree`
Expected: compile error.

- [ ] **Step 3: Implement `tree.rs`**

```rust
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use crate::hand::{Action, Street};

pub type ChipPath = Vec<Action>;
pub type OrdinalPath = Vec<u8>;
pub const RULES_VERSION: u16 = 3;

/// Raise size: a multiple of the facing wager, or "a" = all-in only (spec 10.1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RaiseSize { Mult(f32), AllInOnly }

impl Serialize for RaiseSize {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self { RaiseSize::Mult(x) => s.serialize_f32(*x), RaiseSize::AllInOnly => s.serialize_str("a") }
    }
}

struct RaiseSizeVisitor;

impl<'de> Visitor<'de> for RaiseSizeVisitor {
    type Value = RaiseSize;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result { f.write_str("a positive raise multiple or \"a\"") }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<RaiseSize, E> {
        if v.is_finite() && v > 0.0 { Ok(RaiseSize::Mult(v as f32)) } else { Err(E::custom("raise multiple must be positive and finite")) }
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<RaiseSize, E> { self.visit_f64(v as f64) }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<RaiseSize, E> { self.visit_f64(v as f64) }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<RaiseSize, E> {
        if v == "a" { Ok(RaiseSize::AllInOnly) } else { Err(E::custom(format!("unknown raise size {v:?}"))) }
    }
}

impl<'de> Deserialize<'de> for RaiseSize {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<RaiseSize, D::Error> { d.deserialize_any(RaiseSizeVisitor) }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct Menu { pub bet: Vec<f32>, pub raise: Vec<RaiseSize> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerMenus {
    pub oop: Menu,
    pub ip: Menu,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub donk: Option<Vec<f32>>,
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
```

Add to `lib.rs`: `pub mod tree; pub use tree::*;` and, after `HandState` exists, `SolveInput` in `hand.rs`:

```rust
use crate::range::Range1326;
use crate::tree::EffectiveTree;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolveInput { pub root: StreetRootSnapshot, pub ranges: [Range1326; 2], pub tree: EffectiveTree, pub target_bp: u16 }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p proto`
Expected: 8 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): effective tree, materialized nodes, chip path resolution" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 7: `proto::worker` wire messages

**Files:**
- Create: `crates/proto/src/worker.rs`, `crates/proto/tests/wire_examples.rs`
- Modify: `crates/proto/src/lib.rs` (`pub mod worker;`)

**Interfaces:**
- Consumes: `Card`, `Action`, `Range1326`, `EffectiveTree`, `MaterializedNode`, `OrdinalPath`, `resolve_chip_path`.
- Produces: `REQUEST_LINE_MAX = 1 << 20`, `RESULT_LINE_MAX = 16 << 20`, `MAX_EXPORTED_NODES = 100_000`, `FAILURE_CODES`; `SolveRequest` (16 spec fields, `deny_unknown_fields`), `NodeLock { path: Vec<Action>, actor: String, probs: Vec<Vec<f32>> }`, `EngineMessage::{Solve(SolveRequest), Lock { id, spot, locks }, Cancel { id, target }, Shutdown { id }}`, `ReadyInfo`, `AckStatus::{Accepted, Staged, Rejected, AlreadyFinished, UnknownTarget}` (snake_case), `Stage::{Building, Solving, Extracting}`, `ResultStatus::{Ok, BestSoFar, Cancelled, Error}` (snake_case), `WorkerError { code, message, retryable, estimate_bytes: Option<u64> }`, `NodeStrategy`, `StreetSolution`, `WorkerMessage::{Ready(ReadyInfo), Ack { id, status, reason: Option<String>, replaced: Option<bool> }, Progress { id, stage, iterations, exploitability_chips: Option<f32>, elapsed_ms, memory_bytes }, Result { id, status, elapsed_ms, solution: Option<StreetSolution>, error: Option<WorkerError> }}`. All tagged `type`, lowercase, `deny_unknown_fields`; optional fields omitted when `None` except `exploitability_chips`, which is always present (`null` until measured).

- [ ] **Step 1: Write the failing tests** (`crates/proto/tests/wire_examples.rs`)

```rust
use proto::worker::*;
use proto::*;

/// The two-combo river fixture of spec 4.5: OOP = the six AA combos at 1.0; IP = the three legal QQ combos on Qs Jd 7h 3c 2d at 1.0 and the twelve 54o combos at 0.25.
pub fn river_two_combo_ranges() -> (Range1326, Range1326) {
    let card = |s: &str| s.parse::<Card>().unwrap();
    let mut oop = Range1326::zero();
    for c in class_combos(0) { oop.set(c, 1.0); } // AA
    let mut ip = Range1326::zero();
    let queens = ["Qc", "Qd", "Qh"];
    for a in 0..3 { for b in (a + 1)..3 { ip.set(combo_index(card(queens[a]), card(queens[b])), 1.0); } }
    for i in 0..1326u16 {
        let [lo, hi] = combo_cards(i);
        let ranks = (lo.rank(), hi.rank());
        if (ranks == (2, 3) || ranks == (3, 2)) && lo.suit() != hi.suit() { ip.set(i, 0.25); }
    }
    (oop, ip)
}

const READY: &str = r#"{"type":"ready","proto_version":3,"solver_commit":"9d1509fe5077d019825f833eed04b16d342dfda1","adapter_version":1,"threads":16,"build_features":["avx2"],"cpu_features":["avx2","fma"],"capabilities":["solve","lock","cancel","street_export","i16"]}"#;

#[test]
fn ready_ack_cancel_shutdown_roundtrip_exactly() {
    let m: WorkerMessage = serde_json::from_str(READY).unwrap();
    assert_eq!(serde_json::to_string(&m).unwrap(), READY);
    for line in [r#"{"type":"ack","id":"41","status":"accepted"}"#, r#"{"type":"ack","id":"42","status":"already_finished"}"#, r#"{"type":"ack","id":"47","status":"staged","replaced":false}"#, r#"{"type":"ack","id":"9","status":"rejected","reason":"busy"}"#] {
        let m: WorkerMessage = serde_json::from_str(line).unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), line);
    }
    for line in [r#"{"type":"cancel","id":"42","target":"41"}"#, r#"{"type":"shutdown","id":"48"}"#] {
        let m: EngineMessage = serde_json::from_str(line).unwrap();
        assert_eq!(serde_json::to_string(&m).unwrap(), line);
    }
    let p: WorkerMessage = serde_json::from_str(r#"{"type":"progress","id":"41","stage":"building","iterations":0,"exploitability_chips":null,"elapsed_ms":3,"memory_bytes":331776}"#).unwrap();
    assert!(matches!(p, WorkerMessage::Progress { exploitability_chips: None, stage: Stage::Building, .. }));
    assert!(serde_json::to_string(&p).unwrap().contains(r#""exploitability_chips":null"#));
    let e: WorkerMessage = serde_json::from_str(r#"{"type":"result","id":"49","status":"error","elapsed_ms":2,"error":{"code":"tree_too_large","message":"f32 estimate above limit","retryable":false,"estimate_bytes":9126805504}}"#).unwrap();
    match e { WorkerMessage::Result { status: ResultStatus::Error, error: Some(err), solution: None, .. } => assert_eq!(err.estimate_bytes, Some(9126805504)), other => panic!("{other:?}") }
}

#[test]
fn solve_and_result_roundtrip_with_full_vectors() {
    let (oop, ip) = river_two_combo_ranges();
    let tree: EffectiveTree = serde_json::from_str(r#"{"rules_version":3,"template_id":"river_oracle_v1","root_street":"river","menus":{"river":{"oop":{"bet":[],"raise":[]},"ip":{"bet":[1.0],"raise":[]}}},"add_allin_threshold":0.0,"force_allin_threshold":0.0,"merging_threshold":0.0,"wager_cap":1,"inserted":[],"materialized":[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]},{"path":[0],"street":"river","actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"terminal_pots":[100,null]},{"path":[0,1],"street":"river","actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"terminal_pots":[100,300]}]}"#).unwrap();
    let board: Vec<Card> = ["Qs", "Jd", "7h", "3c", "2d"].iter().map(|s| s.parse().unwrap()).collect();
    let solve = EngineMessage::Solve(SolveRequest { id: "41".into(), spot: "3f9c".into(), board, oop_range: oop, ip_range: ip, pot: 100, stack_oop: 100, stack_ip: 100, rake_rate: 0.0, rake_cap_mchips: 0, tree: tree.clone(), history: vec![Action::Check], target_bp: 10, deadline_ms: 1500, extraction_margin_ms: 200, memory_limit_bytes: 10737418240, background: false });
    let line = serde_json::to_string(&solve).unwrap();
    assert!(line.starts_with(r#"{"type":"solve","id":"41","spot":"3f9c","board":["Qs","Jd","7h","3c","2d"],"oop_range":[1"#) || line.starts_with(r#"{"type":"solve","id":"41","spot":"3f9c","board":["Qs","Jd","7h","3c","2d"],"oop_range":[0"#));
    assert!(line.len() < REQUEST_LINE_MAX);
    let back: EngineMessage = serde_json::from_str(&line).unwrap();
    assert_eq!(back, solve);
    let node = |path: Vec<Action>, actor: &str, actions: Vec<Action>| NodeStrategy { path, actor: actor.into(), actions: actions.clone(), probs: vec![vec![1.0 / actions.len() as f32; actions.len()]; 1326], ev_chips: vec![vec![0.0; actions.len()]; 1326], available: vec![true; 1326] };
    let sol = StreetSolution { nodes: vec![node(vec![Action::Check], "ip", vec![Action::Check, Action::AllIn { to: 100 }]), node(vec![Action::Check, Action::AllIn { to: 100 }], "oop", vec![Action::Fold, Action::Call])], requested: 0, exploitability_chips: 0.09, iterations: 50, memory_bytes: 6914048, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![vec![Action::Check], vec![Action::Check, Action::AllIn { to: 100 }]] };
    let result = WorkerMessage::Result { id: "41".into(), status: ResultStatus::Ok, elapsed_ms: 12, solution: Some(sol), error: None };
    let line = serde_json::to_string(&result).unwrap();
    assert!(line.starts_with(r#"{"type":"result","id":"41","status":"ok","elapsed_ms":12,"solution":{"nodes":[{"path":[{"kind":"check"}],"actor":"ip""#));
    assert!(!line.contains(r#""error""#));
    let back: WorkerMessage = serde_json::from_str(&line).unwrap();
    assert_eq!(back, result);
}

#[test]
fn structurally_invalid_lines_are_rejected() {
    assert!(serde_json::from_str::<EngineMessage>(r#"{"type":"nope","id":"1"}"#).is_err());
    assert!(serde_json::from_str::<EngineMessage>(r#"{"type":"cancel","id":"1","target":"2","extra":true}"#).is_err());
    assert!(serde_json::from_str::<WorkerMessage>(r#"{"type":"ack","id":"1","status":"maybe"}"#).is_err());
    assert!(serde_json::from_str::<WorkerMessage>(&READY.replace(r#""threads":16"#, r#""threads":16,"bunching":true"#)).is_err());
    let short = format!(r#"{{"type":"lock","id":"1","spot":"x","locks":[{{"path":[],"actor":"oop","probs":{}}}]}}"#, serde_json::to_string(&vec![vec![1.0f32]; 1325]).unwrap());
    let parsed: EngineMessage = serde_json::from_str(&short).unwrap();
    assert!(matches!(parsed, EngineMessage::Lock { .. }), "shape errors of lock matrices are validate_locks' job, not serde's");
    assert!(serde_json::from_str::<WorkerMessage>(r#"{"type":"progress","id":"1","stage":"solving","iterations":1,"exploitability_chips":1e999,"elapsed_ms":1,"memory_bytes":1}"#).is_err(), "non-finite numbers are rejected");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto --test wire_examples`
Expected: compile error (`proto::worker` missing).

- [ ] **Step 3: Implement `worker.rs`** (types only; `validate_solution` comes in Task 8)

```rust
//! Worker protocol (spec 4.5): UTF-8 JSON Lines, `type`-tagged, lowercase tags, unknown fields rejected.
use serde::{Deserialize, Serialize};
use crate::cards::Card;
use crate::hand::Action;
use crate::range::Range1326;
use crate::tree::EffectiveTree;

pub const REQUEST_LINE_MAX: usize = 1 << 20;
pub const RESULT_LINE_MAX: usize = 16 << 20;
pub const MAX_EXPORTED_NODES: usize = 100_000;
pub const FAILURE_CODES: [&str; 7] = ["invalid_request", "tree_mismatch", "tree_too_large", "out_of_memory", "lock_mismatch", "no_iteration", "internal"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolveRequest {
    pub id: String, pub spot: String, pub board: Vec<Card>, pub oop_range: Range1326, pub ip_range: Range1326,
    pub pot: u32, pub stack_oop: u32, pub stack_ip: u32, pub rake_rate: f32, pub rake_cap_mchips: u32,
    pub tree: EffectiveTree, pub history: Vec<Action>, pub target_bp: u16, pub deadline_ms: u32,
    pub extraction_margin_ms: u32, pub memory_limit_bytes: u64, pub background: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeLock { pub path: Vec<Action>, pub actor: String, pub probs: Vec<Vec<f32>> }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum EngineMessage {
    Solve(SolveRequest),
    Lock { id: String, spot: String, locks: Vec<NodeLock> },
    Cancel { id: String, target: String },
    Shutdown { id: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadyInfo {
    pub proto_version: u16, pub solver_commit: String, pub adapter_version: u16, pub threads: u8,
    pub build_features: Vec<String>, pub cpu_features: Vec<String>, pub capabilities: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AckStatus { Accepted, Staged, Rejected, AlreadyFinished, UnknownTarget }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage { Building, Solving, Extracting }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultStatus { Ok, BestSoFar, Cancelled, Error }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerError {
    pub code: String, pub message: String, pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeStrategy {
    pub path: Vec<Action>, pub actor: String, pub actions: Vec<Action>,
    pub probs: Vec<Vec<f32>>, pub ev_chips: Vec<Vec<f32>>, pub available: Vec<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreetSolution {
    pub nodes: Vec<NodeStrategy>, pub requested: u32, pub exploitability_chips: f32, pub iterations: u32,
    pub memory_bytes: u64, pub mode: String, pub locks_applied: u16, pub export: String, pub covered_paths: Vec<Vec<Action>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum WorkerMessage {
    Ready(ReadyInfo),
    Ack {
        id: String, status: AckStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")] reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")] replaced: Option<bool>,
    },
    Progress { id: String, stage: Stage, iterations: u32, exploitability_chips: Option<f32>, elapsed_ms: u32, memory_bytes: u64 },
    Result {
        id: String, status: ResultStatus, elapsed_ms: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")] solution: Option<StreetSolution>,
        #[serde(default, skip_serializing_if = "Option::is_none")] error: Option<WorkerError>,
    },
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p proto`
Expected: all pass (8 unit + 3 wire tests).

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): worker wire messages with spec 4.5 examples" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 8: `proto::worker::validate_solution` and `validate_locks`

**Files:**
- Modify: `crates/proto/src/worker.rs`
- Create: `crates/proto/tests/validate_solution.rs`

**Interfaces:**
- Produces: `validate_solution(sol: &StreetSolution, materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>` (returns the resolved ordinal path of every node in `nodes` order; the error text names the first violation), `validate_locks(locks: &[NodeLock], materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>`. Rules (§4.5): `probs` and `ev_chips` exactly `[1326][actions.len()]`, all finite; every probability in `[0, 1]`; available rows sum to `1 +- 1e-3`; unavailable rows all zero in both matrices; `requested < nodes.len()`; `nodes.len() <= MAX_EXPORTED_NODES`; `covered_paths[k] == nodes[k].path`; every path resolves and its node's `actor` and `actions` equal the materialized node's; `mode` in {`f32`, `i16`}; `export` in {`street`, `truncated`}; `exploitability_chips` finite and `>= 0`. Locks: entries in `[0, 1]`, rows all-zero (free combo) or summing to `1 +- 1e-3`.

- [ ] **Step 1: Write the failing tests** (`crates/proto/tests/validate_solution.rs`)

```rust
use proto::worker::*;
use proto::*;

fn tree() -> Vec<MaterializedNode> {
    serde_json::from_str(r#"[{"path":[],"street":"river","actor":"oop","actions":[{"kind":"check"}],"terminal_pots":[null]},{"path":[0],"street":"river","actor":"ip","actions":[{"kind":"check"},{"kind":"allin","to":100}],"terminal_pots":[100,null]},{"path":[0,1],"street":"river","actor":"oop","actions":[{"kind":"fold"},{"kind":"call"}],"terminal_pots":[100,300]}]"#).unwrap()
}

fn node(path: Vec<Action>, actor: &str, actions: Vec<Action>) -> NodeStrategy {
    let a = actions.len();
    let mut probs = vec![vec![1.0 / a as f32; a]; 1326];
    let mut ev = vec![vec![1.5; a]; 1326];
    let mut available = vec![true; 1326];
    for c in (0..1326).step_by(5) { available[c] = false; probs[c] = vec![0.0; a]; ev[c] = vec![0.0; a]; }
    NodeStrategy { path, actor: actor.into(), actions, probs, ev_chips: ev, available }
}

fn solution() -> StreetSolution {
    StreetSolution { nodes: vec![node(vec![Action::Check], "ip", vec![Action::Check, Action::AllIn { to: 100 }]), node(vec![Action::Check, Action::AllIn { to: 100 }], "oop", vec![Action::Fold, Action::Call])], requested: 0, exploitability_chips: 0.09, iterations: 50, memory_bytes: 1, mode: "f32".into(), locks_applied: 0, export: "street".into(), covered_paths: vec![vec![Action::Check], vec![Action::Check, Action::AllIn { to: 100 }]] }
}

#[test]
fn valid_solution_resolves_ordinal_paths() {
    assert_eq!(validate_solution(&solution(), &tree()).unwrap(), vec![vec![0u8], vec![0, 1]]);
}

#[test]
fn validate_solution_rejects_negative_and_above_one() {
    let mut s = solution();
    s.nodes[0].probs[7] = vec![-0.1, 1.1]; // sums to 1 but leaves [0, 1]
    let err = validate_solution(&s, &tree()).unwrap_err();
    assert!(err.contains("outside [0, 1]"), "{err}");
    let mut s = solution();
    s.nodes[1].probs[8] = vec![1.1, -0.1];
    assert!(validate_solution(&s, &tree()).is_err());
}

#[test]
fn validate_solution_row_rules() {
    let mut s = solution();
    s.nodes[0].probs[3] = vec![0.2, 0.2];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("sums to"));
    let mut s = solution();
    s.nodes[0].probs[0] = vec![0.5, 0.5]; // combo 0 is unavailable
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("unavailable"));
    let mut s = solution();
    s.nodes[0].ev_chips[0] = vec![0.0, 1.0];
    assert!(validate_solution(&s, &tree()).is_err());
    let mut s = solution();
    s.nodes[0].ev_chips[9][1] = f32::NAN;
    assert!(validate_solution(&s, &tree()).is_err());
    let mut s = solution();
    s.nodes[0].probs.pop();
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("1326"));
    let mut s = solution();
    s.nodes[0].probs[4] = vec![1.0];
    assert!(validate_solution(&s, &tree()).is_err());
}

#[test]
fn validate_solution_structure_rules() {
    let mut s = solution();
    s.requested = 2;
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("requested"));
    let mut s = solution();
    s.covered_paths[1] = vec![Action::Check];
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("covered_paths"));
    let mut s = solution();
    s.nodes[1].path = vec![Action::Check, Action::Bet { to: 50 }];
    s.covered_paths[1] = s.nodes[1].path.clone();
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("resolve"));
    let mut s = solution();
    s.nodes[0].actor = "oop".into();
    assert!(validate_solution(&s, &tree()).unwrap_err().contains("actor"));
    let mut s = solution();
    s.nodes[1].actions = vec![Action::Fold, Action::Check];
    assert!(validate_solution(&s, &tree()).is_err());
    let mut s = solution();
    s.mode = "f64".into();
    assert!(validate_solution(&s, &tree()).is_err());
}

#[test]
fn validate_locks_rules() {
    let mut probs = vec![vec![0.0, 0.0]; 1326];
    probs[10] = vec![0.3, 0.7];
    let lock = NodeLock { path: vec![Action::Check, Action::AllIn { to: 100 }], actor: "oop".into(), probs };
    assert_eq!(validate_locks(&[lock.clone()], &tree()).unwrap(), vec![vec![0u8, 1]]);
    let mut bad = lock.clone(); bad.probs[11] = vec![-0.1, 1.1];
    assert!(validate_locks(&[bad], &tree()).unwrap_err().contains("outside [0, 1]"));
    let mut bad = lock.clone(); bad.probs[11] = vec![0.2, 0.2];
    assert!(validate_locks(&[bad], &tree()).is_err());
    let mut bad = lock.clone(); bad.actor = "ip".into();
    assert!(validate_locks(&[bad], &tree()).is_err());
    let mut bad = lock; bad.probs.truncate(1325);
    assert!(validate_locks(&[bad], &tree()).is_err());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p proto --test validate_solution`
Expected: compile error (functions missing).

- [ ] **Step 3: Implement the validators** (append to `worker.rs`)

```rust
use crate::cards::COMBOS;
use crate::tree::{resolve_chip_path, MaterializedNode, OrdinalPath};

const ROW_TOLERANCE: f32 = 1e-3;

fn resolve_node<'a>(materialized: &'a [MaterializedNode], what: &str, k: usize, path: &[Action], actor: &str) -> Result<(&'a MaterializedNode, OrdinalPath), String> {
    let ordinal = resolve_chip_path(materialized, path).ok_or_else(|| format!("{what} {k}: chip path does not resolve against the materialized tree"))?;
    let node = materialized.iter().find(|m| m.path == ordinal).expect("resolved paths are materialized");
    if node.actor != actor { return Err(format!("{what} {k}: actor {actor:?} differs from the materialized actor {:?}", node.actor)); }
    Ok((node, ordinal))
}

fn check_row(what: &str, k: usize, combo: usize, row: &[f32], width: usize, allow_all_zero: bool, must_be_zero: bool) -> Result<(), String> {
    if row.len() != width { return Err(format!("{what} {k} combo {combo}: row has {} entries, expected {width}", row.len())); }
    let mut sum = 0.0f32;
    for x in row {
        if !x.is_finite() { return Err(format!("{what} {k} combo {combo}: non-finite entry")); }
        if *x < 0.0 || *x > 1.0 { return Err(format!("{what} {k} combo {combo}: probability {x} outside [0, 1]")); }
        sum += *x;
    }
    if must_be_zero {
        if sum != 0.0 { return Err(format!("{what} {k} combo {combo}: unavailable combo has a non-zero probability row")); }
        return Ok(());
    }
    if allow_all_zero && sum == 0.0 { return Ok(()); }
    if (sum - 1.0).abs() > ROW_TOLERANCE { return Err(format!("{what} {k} combo {combo}: probability row sums to {sum}, expected 1")); }
    Ok(())
}

/// Spec 4.5 matrix and structure validation; returns the ordinal path of every node.
pub fn validate_solution(sol: &StreetSolution, materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String> {
    if sol.nodes.is_empty() { return Err("solution has no nodes".into()); }
    if sol.nodes.len() > MAX_EXPORTED_NODES { return Err(format!("{} nodes exceed the export limit {MAX_EXPORTED_NODES}", sol.nodes.len())); }
    if sol.requested as usize >= sol.nodes.len() { return Err(format!("requested {} is not below nodes.len() {}", sol.requested, sol.nodes.len())); }
    if sol.covered_paths.len() != sol.nodes.len() { return Err(format!("covered_paths has {} entries for {} nodes", sol.covered_paths.len(), sol.nodes.len())); }
    if !sol.exploitability_chips.is_finite() || sol.exploitability_chips < 0.0 { return Err("exploitability_chips must be finite and non-negative".into()); }
    if sol.mode != "f32" && sol.mode != "i16" { return Err(format!("unknown mode {:?}", sol.mode)); }
    if sol.export != "street" && sol.export != "truncated" { return Err(format!("unknown export {:?}", sol.export)); }
    let mut out = Vec::with_capacity(sol.nodes.len());
    for (k, node) in sol.nodes.iter().enumerate() {
        if sol.covered_paths[k] != node.path { return Err(format!("covered_paths[{k}] differs from nodes[{k}].path")); }
        let (m, ordinal) = resolve_node(materialized, "node", k, &node.path, &node.actor)?;
        if m.actions != node.actions { return Err(format!("node {k}: actions differ from the materialized menu")); }
        let width = node.actions.len();
        if node.probs.len() != COMBOS || node.ev_chips.len() != COMBOS || node.available.len() != COMBOS {
            return Err(format!("node {k}: matrices must have exactly 1326 rows"));
        }
        for c in 0..COMBOS {
            check_row("node", k, c, &node.probs[c], width, false, !node.available[c])?;
            let ev = &node.ev_chips[c];
            if ev.len() != width { return Err(format!("node {k} combo {c}: ev row has {} entries, expected {width}", ev.len())); }
            if ev.iter().any(|x| !x.is_finite()) { return Err(format!("node {k} combo {c}: non-finite ev")); }
            if !node.available[c] && ev.iter().any(|x| *x != 0.0) { return Err(format!("node {k} combo {c}: unavailable combo has a non-zero ev row")); }
        }
        out.push(ordinal);
    }
    Ok(out)
}

/// Lock validation (spec 4.5): every entry in [0, 1]; a row is all zero (the free-combo sentinel) or sums to 1 +- 1e-3.
pub fn validate_locks(locks: &[NodeLock], materialized: &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String> {
    let mut out = Vec::with_capacity(locks.len());
    for (k, lock) in locks.iter().enumerate() {
        let (m, ordinal) = resolve_node(materialized, "lock", k, &lock.path, &lock.actor)?;
        if lock.probs.len() != COMBOS { return Err(format!("lock {k}: probs must have exactly 1326 rows")); }
        for (c, row) in lock.probs.iter().enumerate() { check_row("lock", k, c, row, m.actions.len(), true, false)?; }
        out.push(ordinal);
    }
    Ok(out)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all proto tests pass (unit, wire, validation).

- [ ] **Step 5: Commit**

```bash
git add crates/proto
git commit -m "feat(proto): validate_solution and validate_locks per spec 4.5" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 9: `core-model` skeleton: errors, card parsing, positions, config helpers

**Files:**
- Create: `crates/core-model/Cargo.toml`, `crates/core-model/src/lib.rs`, `crates/core-model/src/error.rs`, `crates/core-model/src/cards.rs`, `crates/core-model/src/positions.rs`, `crates/core-model/src/config.rs`, `crates/core-model/tests/positions.rs`

**Interfaces:**
- Consumes: `proto::{Card, CardParseError, HandConfig, Position, Seat, UtgStraddle}`.
- Produces: `RulesError::{FormatUnsupported { detail }, InvalidConfig { reason }, IllegalAction { reason }, NotBetting, NotAwaitingBoard, BadBoard { reason }, Conservation { expected, actual }, Card(CardParseError)}`; `parse_card(&str) -> Result<Card, RulesError>`, `parse_hand(&str) -> Result<[Card; 2], RulesError>`, `parse_cards(&str) -> Result<Vec<Card>, RulesError>` (accepts "AsKd", "As Kd", "As,Kd"; rejects duplicates), `cards_to_string(&[Card]) -> String`; `ring(button, dealt) -> Vec<Seat>` (SB, BB, ..., BTN last), `validate_table(cfg, button, dealt) -> Result<Vec<Seat>, RulesError>`, `positions(button, dealt) -> Vec<(Seat, Position)>`, `position_of(button, dealt, seat) -> Option<Position>`, `preflop_order(button, dealt, straddle: bool) -> Vec<Seat>`, `postflop_order(button, dealt) -> Vec<Seat>`; `straddle_posts(&HandConfig) -> Option<[f32; 3]>`, `initial_full_raise(&HandConfig) -> u32`, `posts(&HandConfig) -> Vec<(usize, u32)>` (ring index, chips: SB, BB, straddle).

- [ ] **Step 1: Write the failing tests** (`crates/core-model/tests/positions.rs`)

```rust
use core_model::*;
use proto::*;

fn cfg(sb: u32, bb: u32, straddle: Option<u32>) -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: sb, bb_chips: bb, straddle: straddle.map(|a| UtgStraddle { amount_chips: a }), rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }

#[test]
fn card_parser_roundtrip() {
    let hand = parse_hand("AsKd").unwrap();
    assert_eq!(cards_to_string(&hand), "AsKd");
    let board = parse_cards("Qs Jd 7h 3c 2d").unwrap();
    assert_eq!(board.len(), 5);
    assert_eq!(cards_to_string(&board), "QsJd7h3c2d");
    assert_eq!(parse_cards("QsJd7h3c2d").unwrap(), board);
    assert_eq!(parse_card(" Td ").unwrap(), Card(33));
    assert!(matches!(parse_hand("AsAs"), Err(RulesError::Card(CardParseError::Duplicate(_)))));
    assert!(parse_hand("As").is_err());
    assert!(parse_hand("AsKdQc").is_err());
    assert!(parse_cards("1s").is_err());
    assert!(parse_cards("AsK").is_err());
}

#[test]
fn straddle_action_order_utg() {
    let dealt = seats(&[0, 1, 2, 3, 4, 5]);
    let btn = Seat(5);
    assert_eq!(positions(btn, &dealt), vec![(Seat(0), Position::Sb), (Seat(1), Position::Bb), (Seat(2), Position::Utg), (Seat(3), Position::Hj), (Seat(4), Position::Co), (Seat(5), Position::Btn)]);
    assert_eq!(preflop_order(btn, &dealt, false), seats(&[2, 3, 4, 5, 0, 1]));
    assert_eq!(preflop_order(btn, &dealt, true), seats(&[3, 4, 5, 0, 1, 2]), "HJ, CO, BTN, SB, BB, UTG");
    assert_eq!(postflop_order(btn, &dealt), seats(&[0, 1, 2, 3, 4, 5]), "SB, BB, UTG, HJ, CO, BTN");
    assert_eq!(straddle_posts(&cfg(1, 2, Some(4))), Some([0.25, 0.5, 1.0]));
    assert_eq!(straddle_posts(&cfg(2, 5, Some(10))), Some([0.2, 0.5, 1.0]));
    assert_eq!(straddle_posts(&cfg(1, 2, None)), None);
    assert_eq!(initial_full_raise(&cfg(1, 2, Some(4))), 4);
    assert_eq!(initial_full_raise(&cfg(2, 5, None)), 5);
    assert_eq!(posts(&cfg(1, 2, Some(4))), vec![(0, 1), (1, 2), (2, 4)]);
    // button elsewhere: seat 2 is the button, so seat 3 posts the SB
    assert_eq!(preflop_order(Seat(2), &dealt, true), seats(&[0, 1, 2, 3, 4, 5]));
}

#[test]
fn dealt_seats_3_to_6() {
    let c = cfg(1, 2, None);
    let three = seats(&[5, 0, 1]);
    assert_eq!(validate_table(&c, Seat(5), &three).unwrap(), seats(&[0, 1, 5]));
    assert_eq!(positions(Seat(5), &three), vec![(Seat(0), Position::Sb), (Seat(1), Position::Bb), (Seat(5), Position::Btn)]);
    let pre = preflop_order(Seat(5), &three, false);
    assert_eq!(pre, seats(&[5, 0, 1]));
    assert_eq!(postflop_order(Seat(5), &three), seats(&[0, 1, 5]));
    let four = seats(&[5, 0, 1, 2]);
    assert_eq!(position_of(Seat(5), &four, Seat(2)), Some(Position::Co));
    assert_eq!(preflop_order(Seat(5), &four, false), seats(&[2, 5, 0, 1]));
    assert_eq!(postflop_order(Seat(5), &four), seats(&[0, 1, 2, 5]));
    let five = seats(&[5, 0, 1, 2, 3]);
    assert_eq!(position_of(Seat(5), &five, Seat(2)), Some(Position::Hj));
    assert_eq!(position_of(Seat(5), &five, Seat(3)), Some(Position::Co));
    assert_eq!(preflop_order(Seat(5), &five, false), seats(&[2, 3, 5, 0, 1]));
    let six = seats(&[0, 1, 2, 3, 4, 5]);
    assert_eq!(preflop_order(Seat(5), &six, false), seats(&[2, 3, 4, 5, 0, 1]));
    assert!(matches!(validate_table(&c, Seat(5), &seats(&[5, 0])), Err(RulesError::FormatUnsupported { detail }) if detail == "two dealt seats"));
    assert!(matches!(validate_table(&cfg(1, 2, Some(4)), Seat(5), &five), Err(RulesError::FormatUnsupported { .. })));
    assert!(matches!(validate_table(&cfg(1, 2, Some(3)), Seat(5), &six), Err(RulesError::FormatUnsupported { detail }) if detail == "short straddle post"));
    assert!(validate_table(&c, Seat(4), &three).is_err(), "button must be dealt");
    assert!(validate_table(&c, Seat(5), &seats(&[5, 0, 0])).is_err(), "duplicate seat");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-model`
Expected: error "package core-model not found".

- [ ] **Step 3: Create the crate**

`crates/core-model/Cargo.toml`:
```toml
[package]
name = "core-model"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Hand lifecycle, positions, legal actions, settlement and street roots (spec sections 2, 4.3, 10.2)"

[dependencies]
proto = { path = "../proto" }
serde = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
serde_json = { workspace = true }
```

`crates/core-model/src/lib.rs`:
```rust
pub mod cards;
pub mod config;
pub mod error;
pub mod positions;

pub use cards::{cards_to_string, parse_card, parse_cards, parse_hand};
pub use config::{initial_full_raise, posts, straddle_posts};
pub use error::RulesError;
pub use positions::{position_of, positions, postflop_order, preflop_order, ring, validate_table};
```

`crates/core-model/src/error.rs`:
```rust
use proto::CardParseError;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RulesError {
    #[error("format unsupported: {detail}")]
    FormatUnsupported { detail: String },
    #[error("invalid config: {reason}")]
    InvalidConfig { reason: String },
    #[error("illegal action: {reason}")]
    IllegalAction { reason: String },
    #[error("no betting round is open")]
    NotBetting,
    #[error("the hand is not awaiting a board")]
    NotAwaitingBoard,
    #[error("bad board: {reason}")]
    BadBoard { reason: String },
    #[error("chip conservation violated: expected {expected}, found {actual}")]
    Conservation { expected: u64, actual: u64 },
    #[error(transparent)]
    Card(#[from] CardParseError),
}
```

`crates/core-model/src/cards.rs`:
```rust
use proto::{Card, CardParseError};
use crate::error::RulesError;

pub fn parse_card(text: &str) -> Result<Card, RulesError> { Ok(text.trim().parse::<Card>()?) }

/// "AsKd", "As Kd" and "As,Kd" are accepted; duplicates are rejected.
pub fn parse_cards(text: &str) -> Result<Vec<Card>, RulesError> {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    if chars.len() % 2 != 0 { return Err(CardParseError::Length(text.to_string()).into()); }
    let mut out = Vec::with_capacity(chars.len() / 2);
    for pair in chars.chunks(2) {
        let card: Card = format!("{}{}", pair[0], pair[1]).parse()?;
        if out.contains(&card) { return Err(CardParseError::Duplicate(card).into()); }
        out.push(card);
    }
    Ok(out)
}

pub fn parse_hand(text: &str) -> Result<[Card; 2], RulesError> {
    let v = parse_cards(text)?;
    if v.len() != 2 { return Err(CardParseError::Length(text.to_string()).into()); }
    Ok([v[0], v[1]])
}

pub fn cards_to_string(cards: &[Card]) -> String { cards.iter().map(|c| c.to_string()).collect() }
```

`crates/core-model/src/positions.rs`:
```rust
use proto::{HandConfig, Position, Seat};
use crate::error::RulesError;

/// Dealt seats clockwise from the seat after the button: SB first, BTN last.
pub fn ring(button: Seat, dealt: &[Seat]) -> Vec<Seat> {
    (1..=6u8).map(|k| Seat((button.0 + k) % 6)).filter(|s| dealt.contains(s)).collect()
}

/// Spec 4.3 dealt-seat rules; returns the ring on success.
pub fn validate_table(cfg: &HandConfig, button: Seat, dealt: &[Seat]) -> Result<Vec<Seat>, RulesError> {
    let invalid = |reason: &str| RulesError::InvalidConfig { reason: reason.to_string() };
    if dealt.len() == 2 { return Err(RulesError::FormatUnsupported { detail: "two dealt seats".into() }); }
    if !(3..=6).contains(&dealt.len()) { return Err(invalid(&format!("{} dealt seats; 3 to 6 are supported", dealt.len()))); }
    if dealt.iter().any(|s| s.0 >= 6) { return Err(invalid("seat ids must be 0..5")); }
    let mut sorted = dealt.to_vec();
    sorted.sort();
    sorted.dedup();
    if sorted.len() != dealt.len() { return Err(invalid("duplicate dealt seat")); }
    if !dealt.contains(&button) { return Err(invalid("the button is not a dealt seat")); }
    if cfg.sb_chips == 0 || cfg.bb_chips < cfg.sb_chips { return Err(invalid("blinds must satisfy 0 < sb <= bb")); }
    if let Some(s) = cfg.straddle {
        if dealt.len() != 6 { return Err(RulesError::FormatUnsupported { detail: "straddle requires six dealt seats".into() }); }
        if s.amount_chips < 2 * cfg.bb_chips { return Err(RulesError::FormatUnsupported { detail: "short straddle post".into() }); }
    }
    Ok(ring(button, dealt))
}

/// BTN, SB, BB, then the last `n - 3` names of UTG, HJ, CO clockwise (spec 4.3).
pub fn positions(button: Seat, dealt: &[Seat]) -> Vec<(Seat, Position)> {
    let r = ring(button, dealt);
    let n = r.len();
    debug_assert!((3..=6).contains(&n));
    let names = &[Position::Utg, Position::Hj, Position::Co][(6 - n)..];
    let mut out = vec![(r[0], Position::Sb), (r[1], Position::Bb)];
    for (k, seat) in r[2..n - 1].iter().enumerate() { out.push((*seat, names[k])); }
    out.push((r[n - 1], Position::Btn));
    out
}

pub fn position_of(button: Seat, dealt: &[Seat], seat: Seat) -> Option<Position> {
    positions(button, dealt).into_iter().find(|(s, _)| *s == seat).map(|(_, p)| p)
}

/// Preflop: the non-button, non-blind seats clockwise after BB, then BTN, SB, BB; with the straddle HJ, CO, BTN, SB, BB, UTG (spec 2, 4.3).
pub fn preflop_order(button: Seat, dealt: &[Seat], straddle: bool) -> Vec<Seat> {
    let r = ring(button, dealt);
    let n = r.len();
    let mut out = Vec::with_capacity(n);
    if straddle && n == 6 {
        out.extend_from_slice(&r[3..5]);
        out.push(r[5]); out.push(r[0]); out.push(r[1]); out.push(r[2]);
    } else {
        out.extend_from_slice(&r[2..n - 1]);
        out.push(r[n - 1]); out.push(r[0]); out.push(r[1]);
    }
    out
}

/// Postflop: SB, BB, then the others clockwise, BTN last.
pub fn postflop_order(button: Seat, dealt: &[Seat]) -> Vec<Seat> { ring(button, dealt) }
```

`crates/core-model/src/config.rs`:
```rust
use proto::HandConfig;

/// Normalized posts `(sb/S, bb/S, 1)` reported under `StraddleMapped` (spec 8.3).
pub fn straddle_posts(cfg: &HandConfig) -> Option<[f32; 3]> {
    cfg.straddle.map(|s| { let unit = s.amount_chips as f32; [cfg.sb_chips as f32 / unit, cfg.bb_chips as f32 / unit, 1.0] })
}

/// The first full raise on the preflop street is one big blind, or one straddle when posted.
pub fn initial_full_raise(cfg: &HandConfig) -> u32 { cfg.straddle.map(|s| s.amount_chips).unwrap_or(cfg.bb_chips) }

/// Forced posts as (ring index, chips): SB, BB and the straddle.
pub fn posts(cfg: &HandConfig) -> Vec<(usize, u32)> {
    let mut v = vec![(0, cfg.sb_chips), (1, cfg.bb_chips)];
    if let Some(s) = cfg.straddle { v.push((2, s.amount_chips)); }
    v
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: proto tests plus 3 core-model tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/core-model
git commit -m "feat(core-model): errors, card parsing, positions and dealt-seat rules" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 10: `core-model::betting::Round` (one betting street)

**Files:**
- Create: `crates/core-model/src/betting.rs`, `crates/core-model/tests/betting.rs`
- Modify: `crates/core-model/src/lib.rs` (`pub mod betting;`)

**Interfaces:**
- Consumes: `proto::{Action, LegalAction, Seat, Street}`, `RulesError`.
- Produces: `Round { street, order: Vec<Seat>, committed: [u32; 6], stacks: [u32; 6], folded: [bool; 6], all_in: [bool; 6], facing: u32, last_full_raise: u32, facing_at_last: [Option<u32>; 6], pending: VecDeque<Seat> }` with `Round::open(street, order, stacks, folded, all_in, min_bet)`, `post(seat, amount)`, `to_act() -> Option<Seat>`, `closed() -> bool`, `live(i) -> bool`, `eligible_count() -> usize`, `all_in_to(i) -> u32`, `min_raise_to() -> u32`, `may_aggress(i) -> bool`, `legal() -> Vec<LegalAction>` (order Fold, Check, Call, Bet|Raise, AllIn), `apply(seat, action) -> Result<(Action, u32), RulesError>` (returns the normalized action and the chips paid). Normalization: a Bet/Raise whose `to` equals the all-in amount is recorded as `AllIn`; an `AllIn` that does not exceed the facing wager is recorded as `Call`. Per-seat arrays are indexed by `Seat.0`; undealt seats are passed as folded.

- [ ] **Step 1: Write the failing tests** (`crates/core-model/tests/betting.rs`)

```rust
use core_model::betting::Round;
use proto::*;

/// Six seats, button 5: preflop order UTG(2), HJ(3), CO(4), BTN(5), SB(0), BB(1); blinds 1/2.
fn preflop(stacks: [u32; 6]) -> Round {
    let mut r = Round::open(Street::Preflop, vec![Seat(2), Seat(3), Seat(4), Seat(5), Seat(0), Seat(1)], stacks, [false; 6], [false; 6], 2);
    r.post(Seat(0), 1);
    r.post(Seat(1), 2);
    r
}
fn has_raise(legal: &[LegalAction]) -> Option<(u32, u32)> {
    legal.iter().find_map(|l| match l { LegalAction::Raise { min_to, max_to } | LegalAction::Bet { min_to, max_to } => Some((*min_to, *max_to)), _ => None })
}
fn has_allin(legal: &[LegalAction]) -> bool { legal.iter().any(|l| matches!(l, LegalAction::AllIn { .. })) }

#[test]
fn min_raise_and_short_allin_no_reopen() {
    let mut r = preflop([200, 200, 200, 14, 200, 200]);
    assert_eq!(r.to_act(), Some(Seat(2)));
    assert_eq!(has_raise(&r.legal()), Some((4, 200)));
    r.apply(Seat(2), Action::Raise { to: 10 }).unwrap();
    assert_eq!(r.last_full_raise, 8);
    assert_eq!(r.to_act(), Some(Seat(3)));
    let legal = r.legal();
    assert_eq!(legal, vec![LegalAction::Fold, LegalAction::Call { cost: 10 }, LegalAction::AllIn { to: 14 }], "14 < min raise-to 18: all-in only");
    assert_eq!(r.apply(Seat(3), Action::AllIn { to: 14 }).unwrap(), (Action::AllIn { to: 14 }, 14));
    assert_eq!(r.last_full_raise, 8, "a short all-in never becomes the full raise");
    assert_eq!(has_raise(&r.legal()), Some((22, 200)), "CO faces 14 with a full raise of 8");
    for seat in [4, 5, 0] { r.apply(Seat(seat), Action::Fold).unwrap(); }
    r.apply(Seat(1), Action::Call).unwrap();
    assert_eq!(r.to_act(), Some(Seat(2)), "the raiser owes a response to the short all-in");
    let legal = r.legal();
    assert_eq!(legal, vec![LegalAction::Fold, LegalAction::Call { cost: 4 }], "a single short all-in does not reopen");
    assert!(matches!(r.apply(Seat(2), Action::Raise { to: 30 }), Err(core_model::RulesError::IllegalAction { .. })));
    r.apply(Seat(2), Action::Call).unwrap();
    assert!(r.closed());
}

#[test]
fn cumulative_short_allins_reopen() {
    for (shorts, reopens) in [((14, 17), false), ((15, 19), true)] {
        let mut r = preflop([200, 200, 200, shorts.0, shorts.1, 200]);
        r.apply(Seat(2), Action::Raise { to: 10 }).unwrap();
        r.apply(Seat(3), Action::AllIn { to: shorts.0 }).unwrap();
        r.apply(Seat(4), Action::AllIn { to: shorts.1 }).unwrap();
        assert_eq!(has_raise(&r.legal()), Some((shorts.1 + 8, 200)), "BTN: facing + last full raise");
        r.apply(Seat(5), Action::Call).unwrap();
        r.apply(Seat(0), Action::Fold).unwrap();
        r.apply(Seat(1), Action::Fold).unwrap();
        assert_eq!(r.to_act(), Some(Seat(2)));
        let legal = r.legal();
        assert_eq!(has_raise(&legal).is_some(), reopens, "cumulative {} vs full raise 8", shorts.1 - 10);
        assert_eq!(has_allin(&legal), reopens);
        if reopens { assert_eq!(has_raise(&legal), Some((27, 200))); }
    }
}

#[test]
fn straddle_min_raise_and_normalization() {
    let mut r = Round::open(Street::Preflop, vec![Seat(3), Seat(4), Seat(5), Seat(0), Seat(1), Seat(2)], [200; 6], [false; 6], [false; 6], 4);
    r.post(Seat(0), 1); r.post(Seat(1), 2); r.post(Seat(2), 4);
    assert_eq!(r.to_act(), Some(Seat(3)));
    assert_eq!(has_raise(&r.legal()), Some((8, 200)), "minimum open over a straddle is 2S");
    r.apply(Seat(3), Action::Raise { to: 8 }).unwrap();
    assert_eq!(has_raise(&r.legal()), Some((12, 200)));
    assert_eq!(r.apply(Seat(4), Action::Raise { to: 200 }).unwrap(), (Action::AllIn { to: 200 }, 200), "a raise to the stack is recorded as all-in");
    assert_eq!(r.legal(), vec![LegalAction::Fold, LegalAction::Call { cost: 200 }], "covered seats cannot raise");
    assert_eq!(r.apply(Seat(5), Action::AllIn { to: 200 }).unwrap(), (Action::Call, 200), "an all-in that only matches is a call");
    assert!(matches!(r.apply(Seat(0), Action::Check), Err(_)));
    assert!(matches!(r.apply(Seat(1), Action::Fold), Err(_)), "seat 0 is to act");
    let mut flop = Round::open(Street::Flop, vec![Seat(0), Seat(1)], [100, 50, 0, 0, 0, 0], [false, false, true, true, true, true], [false; 6], 2);
    assert_eq!(flop.legal(), vec![LegalAction::Check, LegalAction::Bet { min_to: 2, max_to: 100 }, LegalAction::AllIn { to: 100 }]);
    assert!(flop.apply(Seat(0), Action::Raise { to: 10 }).is_err(), "no wager pending: bet, not raise");
    flop.apply(Seat(0), Action::Bet { to: 10 }).unwrap();
    assert!(flop.apply(Seat(1), Action::Bet { to: 30 }).is_err(), "wager pending: raise, not bet");
    flop.apply(Seat(1), Action::Raise { to: 30 }).unwrap();
    assert_eq!(flop.last_full_raise, 20);
    assert_eq!(has_raise(&flop.legal()), Some((50, 100)));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-model --test betting`
Expected: compile error (`betting` missing).

- [ ] **Step 3: Implement `betting.rs`**

```rust
use std::collections::VecDeque;
use proto::{Action, LegalAction, Seat, Street};
use crate::error::RulesError;

/// One betting street. Per-seat arrays are indexed by `Seat.0`; undealt seats are marked folded.
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    pub street: Street,
    pub order: Vec<Seat>,
    pub committed: [u32; 6],
    pub stacks: [u32; 6],
    pub folded: [bool; 6],
    pub all_in: [bool; 6],
    pub facing: u32,
    pub last_full_raise: u32,
    pub facing_at_last: [Option<u32>; 6],
    pub pending: VecDeque<Seat>,
}

fn illegal(reason: impl Into<String>) -> RulesError { RulesError::IllegalAction { reason: reason.into() } }

impl Round {
    /// Opens a street; `order` is the action order among the seats present at the street start.
    pub fn open(street: Street, order: Vec<Seat>, stacks: [u32; 6], folded: [bool; 6], all_in: [bool; 6], min_bet: u32) -> Round {
        let pending = order.iter().copied().filter(|s| !folded[s.0 as usize] && !all_in[s.0 as usize]).collect();
        Round { street, order, committed: [0; 6], stacks, folded, all_in, facing: 0, last_full_raise: min_bet, facing_at_last: [None; 6], pending }
    }

    /// Posts a blind or straddle without consuming the seat's turn; a post above the stack is all-in for the stack.
    pub fn post(&mut self, seat: Seat, amount: u32) {
        let i = seat.0 as usize;
        let paid = amount.min(self.stacks[i]);
        self.stacks[i] -= paid;
        self.committed[i] += paid;
        self.facing = self.facing.max(self.committed[i]);
        if self.stacks[i] == 0 { self.all_in[i] = true; self.pending.retain(|s| *s != seat); }
    }

    pub fn to_act(&self) -> Option<Seat> { self.pending.front().copied() }
    pub fn closed(&self) -> bool { self.pending.is_empty() }
    pub fn live(&self, i: usize) -> bool { !self.folded[i] && !self.all_in[i] }
    pub fn eligible_count(&self) -> usize { self.folded.iter().filter(|f| !**f).count() }
    pub fn all_in_to(&self, i: usize) -> u32 { self.committed[i] + self.stacks[i] }
    pub fn min_raise_to(&self) -> u32 { self.facing + self.last_full_raise }

    /// Spec 4.3: the seat can put more chips in, the action is (re)opened for it (cumulative rule) and some opponent can still respond.
    pub fn may_aggress(&self, i: usize) -> bool {
        if self.folded[i] || self.all_in_to(i) <= self.facing { return false; }
        let reopened = match self.facing_at_last[i] { None => true, Some(f) => self.facing - f >= self.last_full_raise };
        let responder = (0..6).any(|j| j != i && !self.folded[j] && self.all_in_to(j) > self.facing);
        reopened && responder
    }

    pub fn legal(&self) -> Vec<LegalAction> {
        let Some(seat) = self.to_act() else { return vec![] };
        let i = seat.0 as usize;
        let owed = self.facing - self.committed[i];
        let mut v = Vec::with_capacity(4);
        if owed > 0 { v.push(LegalAction::Fold); v.push(LegalAction::Call { cost: owed.min(self.stacks[i]) }); } else { v.push(LegalAction::Check); }
        if self.may_aggress(i) {
            let (min_to, max_to) = (self.min_raise_to(), self.all_in_to(i));
            if min_to <= max_to {
                if self.facing == 0 { v.push(LegalAction::Bet { min_to, max_to }); } else { v.push(LegalAction::Raise { min_to, max_to }); }
            }
            v.push(LegalAction::AllIn { to: max_to });
        }
        v
    }

    /// Applies an action for `seat`; returns the recorded (normalized) action and the chips paid.
    pub fn apply(&mut self, seat: Seat, action: Action) -> Result<(Action, u32), RulesError> {
        let actor = self.to_act().ok_or(RulesError::NotBetting)?;
        if actor != seat { return Err(illegal(format!("seat {} is not to act (seat {} is)", seat.0, actor.0))); }
        let i = seat.0 as usize;
        let owed = self.facing - self.committed[i];
        let max_to = self.all_in_to(i);
        let action = match action {
            Action::AllIn { to } if to == max_to && to <= self.facing => Action::Call,
            Action::Bet { to } | Action::Raise { to } if to == max_to => Action::AllIn { to },
            a => a,
        };
        match action {
            Action::Fold => {
                if owed == 0 { return Err(illegal("fold with no wager to fold to")); }
                self.folded[i] = true;
                self.facing_at_last[i] = Some(self.facing);
                self.pending.pop_front();
                Ok((Action::Fold, 0))
            }
            Action::Check => {
                if owed != 0 { return Err(illegal(format!("check while facing {owed}"))); }
                self.facing_at_last[i] = Some(self.facing);
                self.pending.pop_front();
                Ok((Action::Check, 0))
            }
            Action::Call => {
                if owed == 0 { return Err(illegal("call with nothing owed")); }
                let paid = owed.min(self.stacks[i]);
                self.stacks[i] -= paid;
                self.committed[i] += paid;
                if self.stacks[i] == 0 { self.all_in[i] = true; }
                self.facing_at_last[i] = Some(self.facing);
                self.pending.pop_front();
                Ok((Action::Call, paid))
            }
            Action::Bet { to } | Action::Raise { to } | Action::AllIn { to } => {
                let is_all_in = matches!(action, Action::AllIn { .. });
                if matches!(action, Action::Bet { .. }) && self.facing != 0 { return Err(illegal("bet while a wager is pending; use raise")); }
                if matches!(action, Action::Raise { .. }) && self.facing == 0 { return Err(illegal("raise with no wager pending; use bet")); }
                if !self.may_aggress(i) { return Err(illegal("betting is not (re)opened for this seat")); }
                if is_all_in && to != max_to { return Err(illegal(format!("all-in must be to {max_to}, got {to}"))); }
                if !is_all_in && (to < self.min_raise_to() || to > max_to) {
                    return Err(illegal(format!("wager to {to} outside [{}, {max_to}]", self.min_raise_to())));
                }
                let inc = to - self.facing;
                let paid = to - self.committed[i];
                self.stacks[i] -= paid;
                self.committed[i] = to;
                if self.stacks[i] == 0 { self.all_in[i] = true; }
                if inc >= self.last_full_raise { self.last_full_raise = inc; }
                self.facing = to;
                self.facing_at_last[i] = Some(to);
                let pos = self.order.iter().position(|s| *s == seat).expect("the actor is in the street order");
                let n = self.order.len();
                self.pending = (1..n).map(|k| self.order[(pos + k) % n]).filter(|s| self.live(s.0 as usize)).collect();
                Ok((action, paid))
            }
        }
    }
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (3 new betting tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core-model
git commit -m "feat(core-model): betting round with cumulative reopening and legal actions" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 11: `core-model` hand lifecycle, settlement and the public state API

**Files:**
- Create: `crates/core-model/src/settlement.rs`, `crates/core-model/src/lifecycle.rs`, `crates/core-model/src/state.rs`, `crates/core-model/tests/lifecycle.rs`
- Modify: `crates/core-model/src/lib.rs`

**Interfaces:**
- Consumes: `Round` (Task 10), positions and config helpers (Task 9), `proto::{HandState, HandPhase, CompleteReason, Derived, Pot, TakenAction, ...}`.
- Produces: `settlement::{Settlement { pots: Vec<Pot>, returned: Vec<(Seat, u32)> }, refund_uncalled(committed: &mut [u32; 6], stacks: &mut [u32; 6]) -> Option<(Seat, u32)>, layer_pots(contributed: &[u32; 6], folded: &[bool; 6]) -> Vec<Pot>, check_conservation(stacks, live, pots, stacks_start_total) -> Result<(), RulesError>}`; `lifecycle::{Sim { round, phase, contributed, pots, returned, start_total }, simulate(&HandState) -> Result<Sim, RulesError>, Sim::derived(&self) -> Derived}`; `state::{BeginHand { hand_id, button, hero, dealt, stacks_start, hero_cards }, begin_hand(&HandConfig, BeginHand) -> Result<HandState, RulesError>, apply_action(&HandState, Action) -> Result<HandState, RulesError>, set_board(&HandState, &[Card]) -> Result<HandState, RulesError>, set_hero_cards(&HandState, [Card; 2]) -> Result<HandState, RulesError>, derive(&HandState) -> Derived, settle_pots(&HandState) -> Settlement, is_decision_point(&HandState) -> bool, abandon(&HandState) -> HandState}`. `HandState.stacks_start` is aligned with `HandState.dealt`; `Derived` vectors are indexed by `Seat.0`. `Derived.all_in[s] = !folded[s] && stacks_remaining[s] == 0`. In `Complete`, `Derived.street` is the last betting street; in `AwaitingBoard{s}` it is `s`.

- [ ] **Step 1: Write the failing tests** (`crates/core-model/tests/lifecycle.rs`)

```rust
use core_model::settlement::{layer_pots, refund_uncalled};
use core_model::*;
use proto::*;

fn cfg(sb: u32, bb: u32, straddle: Option<u32>) -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: sb, bb_chips: bb, straddle: straddle.map(|a| UtgStraddle { amount_chips: a }), rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }
fn begin(cfg: &HandConfig, button: u8, dealt: &[u8], stacks: &[u32]) -> HandState {
    begin_hand(cfg, BeginHand { hand_id: 1, button: Seat(button), hero: Seat(dealt[0]), dealt: seats(dealt), stacks_start: stacks.to_vec(), hero_cards: None }).unwrap()
}
fn total(d: &Derived) -> u32 { d.stacks_remaining.iter().sum::<u32>() + d.committed_this_street.iter().sum::<u32>() + d.pots.iter().map(|p| p.amount).sum::<u32>() }
fn play(state: HandState, action: Action, start_total: u32) -> HandState {
    let next = apply_action(&state, action).unwrap();
    assert_eq!(total(&next.derived), start_total, "conservation after {action:?}");
    next
}
fn pot(amount: u32, eligible: &[u8]) -> Pot { Pot { amount, eligible: seats(eligible) } }

#[test]
fn side_pot_three_allins() {
    // dealt BTN(5)=200, SB(0)=50, BB(1)=100; blinds 1/2; preflop order BTN, SB, BB
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1], &[200, 50, 100]);
    assert_eq!(total(&s.derived), 350);
    let s = play(s, Action::AllIn { to: 200 }, 350);
    assert_eq!(s.derived.legal, vec![LegalAction::Fold, LegalAction::Call { cost: 49 }]);
    let s = play(s, Action::Call, 350);
    assert_eq!(s.phase, HandPhase::Betting { street: Street::Preflop });
    // the invariant between refund and settlement, on the closing committed amounts
    let mut committed = [50, 100, 0, 0, 0, 200];
    let mut stacks = [0, 0, 0, 0, 0, 0];
    assert_eq!(refund_uncalled(&mut committed, &mut stacks), Some((Seat(5), 100)));
    assert_eq!(stacks[5], 100);
    assert_eq!(stacks.iter().sum::<u32>() + committed.iter().sum::<u32>(), 350, "after the refund: stacks 100, live 250");
    let pots = layer_pots(&committed, &[false, false, true, true, true, false]);
    assert_eq!(pots, vec![pot(150, &[0, 1, 5]), pot(100, &[1, 5])]);
    assert_eq!(stacks.iter().sum::<u32>() + pots.iter().map(|p| p.amount).sum::<u32>(), 350);
    // the state machine does the same at closure
    let s = play(s, Action::Call, 350);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(s.derived.pots, vec![pot(150, &[0, 1, 5]), pot(100, &[1, 5])]);
    assert_eq!(s.derived.stacks_remaining, vec![0, 0, 0, 0, 0, 100]);
    assert_eq!(s.derived.all_in, vec![true, true, false, false, false, false]);
    assert_eq!(settle_pots(&s).returned, vec![(Seat(5), 100)]);
    assert_eq!(s.derived.pot, 250);
    assert!(matches!(set_board(&s, &parse_cards("AsKd2c").unwrap()), Err(RulesError::NotAwaitingBoard)));
}

#[test]
fn side_pot_two_contested() {
    // dealt BTN(5)=200, SB(0)=50, BB(1)=100, CO(2)=200; preflop order CO, BTN, SB, BB
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1, 2], &[200, 50, 100, 200]);
    let s = play(s, Action::AllIn { to: 200 }, 550);
    let s = play(s, Action::Call, 550);
    let s = play(s, Action::Call, 550);
    let s = play(s, Action::Call, 550);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(settle_pots(&s).returned, vec![]);
    assert_eq!(s.derived.pots, vec![pot(200, &[0, 1, 2, 5]), pot(150, &[1, 2, 5]), pot(200, &[2, 5])]);
    assert_eq!(s.derived.pots.iter().map(|p| p.amount).sum::<u32>(), 550);
}

#[test]
fn allin_runout_single_survivor() {
    // BTN(5)=300 folds, SB(0)=100 shoves, BB(1)=300 calls
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1], &[300, 100, 300]);
    let s = play(s, Action::Fold, 700);
    let s = play(s, Action::AllIn { to: 100 }, 700);
    let s = play(s, Action::Call, 700);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(settle_pots(&s).returned, vec![], "no refund: the call matched exactly");
    assert_eq!(s.derived.pots, vec![pot(200, &[0, 1])]);
    assert_eq!(s.derived.stacks_remaining[1], 200, "the survivor keeps 200 chips and the hand is still complete");
    // the caller shorter: BTN shoves 300, SB calls for 100, BB folds
    let s = begin(&cfg(1, 2, None), 5, &[5, 0, 1], &[300, 100, 300]);
    let s = play(s, Action::AllIn { to: 300 }, 700);
    let s = play(s, Action::Call, 700);
    assert_eq!(s.derived.all_in[0], true);
    let s = play(s, Action::Fold, 700);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::AllInRunout });
    assert_eq!(settle_pots(&s).returned, vec![(Seat(5), 200)]);
    assert_eq!(s.derived.pots, vec![pot(202, &[0, 5])]);
    assert_eq!(s.derived.stacks_remaining, vec![0, 298, 0, 0, 0, 200]);
}

#[test]
fn lifecycle_streets_and_board() {
    let c = cfg(1, 2, Some(4));
    let s = begin(&c, 5, &[0, 1, 2, 3, 4, 5], &[200; 6]);
    assert_eq!(s.derived.to_act, Some(Seat(3)), "HJ opens over the straddle");
    assert_eq!(s.derived.pot, 7);
    assert_eq!(s.derived.committed_this_street, vec![1, 2, 4, 0, 0, 0]);
    assert!(!is_decision_point(&s), "hero (seat 0) is not to act");
    let mut s = s;
    for _ in 0..5 { s = play(s, Action::Fold, 1200); }
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::FoldedOut });
    assert_eq!(s.derived.pots, vec![pot(5, &[2])]);
    assert_eq!(s.derived.stacks_remaining[2], 198);
    assert_eq!(settle_pots(&s).returned, vec![(Seat(2), 2)]);
    assert!(apply_action(&s, Action::Check).is_err());
    // three-way limped pot to the flop
    let c = cfg(1, 2, None);
    let s = begin(&c, 5, &[5, 0, 1], &[200; 3]);
    let s = play(s, Action::Call, 600);
    let s = play(s, Action::Call, 600);
    assert_eq!(s.derived.legal, vec![LegalAction::Check, LegalAction::Raise { min_to: 4, max_to: 200 }, LegalAction::AllIn { to: 200 }], "BB option");
    let s = play(s, Action::Check, 600);
    assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Flop });
    assert_eq!(s.derived.street, Street::Flop);
    assert_eq!(s.derived.to_act, None);
    assert!(s.derived.legal.is_empty());
    assert_eq!(s.derived.pots, vec![pot(6, &[0, 1, 5])]);
    assert!(matches!(set_board(&s, &parse_cards("AsKd").unwrap()), Err(RulesError::BadBoard { .. })));
    assert!(matches!(set_board(&s, &parse_cards("AsAs2c").unwrap_or_default()), Err(RulesError::BadBoard { .. })));
    let s = set_board(&s, &parse_cards("AsKd2c").unwrap()).unwrap();
    assert_eq!(s.phase, HandPhase::Betting { street: Street::Flop });
    assert_eq!(s.derived.to_act, Some(Seat(0)), "SB acts first postflop");
    assert_eq!(s.derived.last_full_raise, 2);
    let s = set_hero_cards(&s, parse_hand("QhQd").unwrap()).unwrap();
    assert!(!is_decision_point(&s), "SB is to act; hero is the button (dealt[0])");
    assert!(set_hero_cards(&s, parse_hand("AsQd").unwrap()).is_err(), "board card");
    let s = play(s, Action::Bet { to: 10 }, 600);
    let s = play(s, Action::Fold, 600);
    assert!(is_decision_point(&s), "hero (BTN) faces the bet with two cards on record");
    let s = play(s, Action::Call, 600);
    assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Turn });
    assert!(set_board(&s, &parse_cards("AsKd2c").unwrap()).is_err(), "turn needs four cards");
    assert!(set_board(&s, &parse_cards("AsKd3c7h").unwrap()).is_err(), "the flop on record must be kept");
    let s = set_board(&s, &parse_cards("AsKd2c7h").unwrap()).unwrap();
    let s = play(s, Action::Check, 600);
    let s = play(s, Action::Check, 600);
    let s = set_board(&s, &parse_cards("AsKd2c7h7d").unwrap()).unwrap();
    let s = play(s, Action::Check, 600);
    let s = play(s, Action::Check, 600);
    assert_eq!(s.phase, HandPhase::Complete { reason: CompleteReason::ShowdownReached });
    assert_eq!(s.derived.pots, vec![pot(26, &[0, 5])]);
    let a = abandon(&s);
    assert_eq!(a.phase, HandPhase::Abandoned);
    assert_eq!(derive(&a).to_act, None);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-model --test lifecycle`
Expected: compile error.

- [ ] **Step 3: Implement `settlement.rs`**

```rust
use proto::{Pot, Seat};
use crate::error::RulesError;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Settlement { pub pots: Vec<Pot>, pub returned: Vec<(Seat, u32)> }

/// Returns the uncalled portion of the highest street contribution to its owner (spec 4.3): a transfer from live commitment back to the stack.
pub fn refund_uncalled(committed: &mut [u32; 6], stacks: &mut [u32; 6]) -> Option<(Seat, u32)> {
    let top = (0..6).max_by_key(|i| committed[*i])?;
    let top_amount = committed[top];
    let second = (0..6).filter(|i| *i != top).map(|i| committed[i]).max().unwrap_or(0);
    if top_amount > second {
        let refund = top_amount - second;
        committed[top] = second;
        stacks[top] += refund;
        Some((Seat(top as u8), refund))
    } else {
        None
    }
}

/// Layers total contributions into main and side pots by contribution level; folded seats' chips are dead money.
/// Adjacent layers with the same eligible set are merged.
pub fn layer_pots(contributed: &[u32; 6], folded: &[bool; 6]) -> Vec<Pot> {
    let mut levels: Vec<u32> = contributed.iter().copied().filter(|c| *c > 0).collect();
    levels.sort_unstable();
    levels.dedup();
    let mut pots: Vec<Pot> = Vec::new();
    let mut prev = 0u32;
    for level in levels {
        let amount: u32 = contributed.iter().map(|c| (*c).min(level) - (*c).min(prev)).sum();
        let eligible: Vec<Seat> = (0..6).filter(|i| !folded[*i] && contributed[*i] >= level).map(|i| Seat(i as u8)).collect();
        prev = level;
        if amount == 0 { continue; }
        match pots.last_mut() {
            Some(last) if last.eligible == eligible || eligible.is_empty() => last.amount += amount,
            _ => pots.push(Pot { amount, eligible }),
        }
    }
    pots
}

/// Spec 4.3: `sum(stacks) + live commitments + unawarded pots + rake (0) == sum(stacks_start)`.
pub fn check_conservation(stacks: &[u32; 6], live: &[u32; 6], pots: &[Pot], stacks_start_total: u64) -> Result<(), RulesError> {
    let actual = stacks.iter().map(|s| *s as u64).sum::<u64>() + live.iter().map(|c| *c as u64).sum::<u64>() + pots.iter().map(|p| p.amount as u64).sum::<u64>();
    if actual == stacks_start_total { Ok(()) } else { Err(RulesError::Conservation { expected: stacks_start_total, actual }) }
}
```

- [ ] **Step 4: Implement `lifecycle.rs`**

```rust
use proto::{CompleteReason, Derived, HandPhase, HandState, Pot, Seat, Street};
use crate::betting::Round;
use crate::config::{initial_full_raise, posts};
use crate::error::RulesError;
use crate::positions::{postflop_order, preflop_order, ring};
use crate::settlement::{check_conservation, layer_pots, refund_uncalled};

/// The replayed hand: the current (or last) betting round plus hand-level accounting.
#[derive(Clone, Debug)]
pub struct Sim {
    pub round: Round,
    pub phase: HandPhase,
    pub contributed: [u32; 6],
    pub pots: Vec<Pot>,
    pub returned: Vec<(Seat, u32)>,
    pub start_total: u64,
}

fn seat_array<T: Copy>(state: &HandState, default: T, mut f: impl FnMut(usize) -> T) -> [T; 6] {
    let mut out = [default; 6];
    for (k, seat) in state.dealt.iter().enumerate() { out[seat.0 as usize] = f(k); }
    out
}

impl Sim {
    fn open_preflop(state: &HandState) -> Sim {
        let dealt = seat_array(state, false, |_| true);
        let stacks = seat_array(state, 0u32, |k| state.stacks_start[k]);
        let folded: [bool; 6] = std::array::from_fn(|i| !dealt[i]);
        let order = preflop_order(state.button, &state.dealt, state.config.straddle.is_some());
        let mut round = Round::open(Street::Preflop, order, stacks, folded, [false; 6], initial_full_raise(&state.config));
        let r = ring(state.button, &state.dealt);
        for (idx, chips) in posts(&state.config) { round.post(r[idx], chips); }
        Sim { round, phase: HandPhase::Betting { street: Street::Preflop }, contributed: [0; 6], pots: vec![], returned: vec![], start_total: state.stacks_start.iter().map(|s| *s as u64).sum() }
    }

    fn check(&self) -> Result<(), RulesError> { check_conservation(&self.round.stacks, &self.round.committed, &self.pots, self.start_total) }

    /// Street closure (spec 4.3): refund the uncalled portion, settle the pots, choose the next phase.
    fn close(&mut self) -> Result<(), RulesError> {
        if let Some(r) = refund_uncalled(&mut self.round.committed, &mut self.round.stacks) {
            self.round.all_in[r.0 .0 as usize] = false;
            self.returned.push(r);
        }
        self.check()?;
        for i in 0..6 { self.contributed[i] += self.round.committed[i]; self.round.committed[i] = 0; }
        self.pots = layer_pots(&self.contributed, &self.round.folded);
        self.check()?;
        let eligible = self.round.eligible_count();
        let with_chips = (0..6).filter(|i| !self.round.folded[*i] && self.round.stacks[*i] > 0).count();
        self.phase = if eligible == 1 { HandPhase::Complete { reason: CompleteReason::FoldedOut } }
            else if with_chips < 2 { HandPhase::Complete { reason: CompleteReason::AllInRunout } }
            else if let Some(next) = self.round.street.next() { HandPhase::AwaitingBoard { street: next } }
            else { HandPhase::Complete { reason: CompleteReason::ShowdownReached } };
        self.round.pending.clear();
        Ok(())
    }

    fn open_street(&mut self, state: &HandState, street: Street) {
        let order = postflop_order(state.button, &state.dealt);
        self.round = Round::open(street, order, self.round.stacks, self.round.folded, self.round.all_in, state.config.bb_chips);
        self.phase = HandPhase::Betting { street };
    }

    pub fn derived(&self) -> Derived {
        let r = &self.round;
        let betting = matches!(self.phase, HandPhase::Betting { .. });
        let street = match self.phase { HandPhase::Betting { street } | HandPhase::AwaitingBoard { street } => street, _ => r.street };
        Derived {
            street,
            to_act: if betting { r.to_act() } else { None },
            pot: self.pots.iter().map(|p| p.amount).sum::<u32>() + r.committed.iter().sum::<u32>(),
            committed_this_street: r.committed.to_vec(),
            stacks_remaining: r.stacks.to_vec(),
            folded: r.folded.to_vec(),
            all_in: (0..6).map(|i| !r.folded[i] && r.stacks[i] == 0).collect(),
            facing: r.facing,
            last_full_raise: r.last_full_raise,
            pots: self.pots.clone(),
            legal: if betting { r.legal() } else { vec![] },
        }
    }
}

/// Replays `state.actions` and `state.board` from the forced posts. Fails only on a state that this crate did not build.
pub fn simulate(state: &HandState) -> Result<Sim, RulesError> {
    let mut sim = Sim::open_preflop(state);
    sim.check()?;
    let mut k = 0;
    loop {
        match sim.phase {
            HandPhase::Betting { street } => {
                if k >= state.actions.len() { break; }
                let a = state.actions[k];
                k += 1;
                if a.street != street {
                    return Err(RulesError::IllegalAction { reason: format!("action {k} is on {:?} but the hand is on {:?}", a.street, street) });
                }
                let (recorded, paid) = sim.round.apply(a.seat, a.action)?;
                if recorded != a.action || paid != a.paid {
                    return Err(RulesError::IllegalAction { reason: format!("action {k} recorded as {:?}/{} replays as {:?}/{}", a.action, a.paid, recorded, paid) });
                }
                sim.check()?;
                if sim.round.eligible_count() == 1 || sim.round.closed() { sim.close()?; }
            }
            HandPhase::AwaitingBoard { street } => {
                if state.board.len() >= street.board_len() { sim.open_street(state, street); } else { break; }
            }
            HandPhase::Complete { .. } | HandPhase::Abandoned => break,
        }
    }
    if k < state.actions.len() { return Err(RulesError::NotBetting); }
    if matches!(state.phase, HandPhase::Abandoned) { sim.phase = HandPhase::Abandoned; }
    Ok(sim)
}
```

- [ ] **Step 5: Implement `state.rs`**

```rust
use proto::{Action, Card, CardParseError, Derived, HandConfig, HandPhase, HandState, Seat, Street, TakenAction};
use crate::error::RulesError;
use crate::lifecycle::simulate;
use crate::positions::validate_table;
use crate::settlement::Settlement;

#[derive(Clone, Debug, PartialEq)]
pub struct BeginHand { pub hand_id: u64, pub button: Seat, pub hero: Seat, pub dealt: Vec<Seat>, pub stacks_start: Vec<u32>, pub hero_cards: Option<[Card; 2]> }

fn refresh(state: &mut HandState) -> Result<(), RulesError> {
    let sim = simulate(state)?;
    state.phase = sim.phase;
    state.derived = sim.derived();
    Ok(())
}

pub fn begin_hand(cfg: &HandConfig, begin: BeginHand) -> Result<HandState, RulesError> {
    validate_table(cfg, begin.button, &begin.dealt)?;
    let invalid = |reason: &str| RulesError::InvalidConfig { reason: reason.to_string() };
    if !begin.dealt.contains(&begin.hero) { return Err(invalid("hero is not a dealt seat")); }
    if begin.stacks_start.len() != begin.dealt.len() { return Err(invalid("one starting stack per dealt seat")); }
    if begin.stacks_start.iter().any(|s| *s == 0) { return Err(invalid("starting stacks must be positive")); }
    if let Some([a, b]) = begin.hero_cards { if a == b { return Err(CardParseError::Duplicate(a).into()); } }
    let mut state = HandState {
        hand_id: begin.hand_id, hand_revision: 0, config: cfg.clone(), phase: HandPhase::Betting { street: Street::Preflop },
        button: begin.button, hero: begin.hero, hero_cards: begin.hero_cards, dealt: begin.dealt, stacks_start: begin.stacks_start,
        board: vec![], actions: vec![], derived: Derived::default(),
    };
    refresh(&mut state)?;
    Ok(state)
}

pub fn derive(state: &HandState) -> Derived {
    simulate(state).expect("a HandState built by core-model replays consistently").derived()
}

/// Applies an action for `Derived.to_act`; the recorded action is the normalized one (Task 10).
pub fn apply_action(state: &HandState, action: Action) -> Result<HandState, RulesError> {
    let mut sim = simulate(state)?;
    let HandPhase::Betting { street } = sim.phase else { return Err(RulesError::NotBetting) };
    let seat = sim.round.to_act().ok_or(RulesError::NotBetting)?;
    let (recorded, paid) = sim.round.apply(seat, action)?;
    let mut next = state.clone();
    next.actions.push(TakenAction { seat, street, action: recorded, paid });
    refresh(&mut next)?;
    Ok(next)
}

/// Replaces the full board (3, 4 or 5 cards); legal only in `AwaitingBoard`; the cards already on record must be kept.
pub fn set_board(state: &HandState, cards: &[Card]) -> Result<HandState, RulesError> {
    let HandPhase::AwaitingBoard { street } = state.phase else { return Err(RulesError::NotAwaitingBoard) };
    let bad = |reason: String| RulesError::BadBoard { reason };
    if cards.len() != street.board_len() { return Err(bad(format!("{street:?} needs {} cards, got {}", street.board_len(), cards.len()))); }
    if !cards.starts_with(&state.board) { return Err(bad("the earlier streets' cards differ from the board on record; use undo".into())); }
    for (i, c) in cards.iter().enumerate() {
        if cards[..i].contains(c) { return Err(bad(format!("duplicate card {c}"))); }
        if state.hero_cards.map_or(false, |h| h.contains(c)) { return Err(bad(format!("{c} is one of hero's cards"))); }
    }
    let mut next = state.clone();
    next.board = cards.to_vec();
    refresh(&mut next)?;
    Ok(next)
}

pub fn set_hero_cards(state: &HandState, cards: [Card; 2]) -> Result<HandState, RulesError> {
    if cards[0] == cards[1] { return Err(CardParseError::Duplicate(cards[0]).into()); }
    if let Some(c) = cards.iter().find(|c| state.board.contains(c)) { return Err(RulesError::BadBoard { reason: format!("{c} is on the board") }); }
    let mut next = state.clone();
    next.hero_cards = Some(cards);
    Ok(next)
}

/// Settled pots and every refund made so far in the hand, in order.
pub fn settle_pots(state: &HandState) -> Settlement {
    let sim = simulate(state).expect("a HandState built by core-model replays consistently");
    Settlement { pots: sim.pots, returned: sim.returned }
}

/// Spec section 2: hero to act, two hero cards on record, at least two legal actions.
pub fn is_decision_point(state: &HandState) -> bool {
    matches!(state.phase, HandPhase::Betting { .. }) && state.derived.to_act == Some(state.hero) && state.hero_cards.is_some() && state.derived.legal.len() >= 2
}

pub fn abandon(state: &HandState) -> HandState {
    let mut next = state.clone();
    next.phase = HandPhase::Abandoned;
    next.derived = derive(&next);
    next
}
```

Update `lib.rs`:
```rust
pub mod betting;
pub mod cards;
pub mod config;
pub mod error;
pub mod lifecycle;
pub mod positions;
pub mod settlement;
pub mod state;

pub use cards::{cards_to_string, parse_card, parse_cards, parse_hand};
pub use config::{initial_full_raise, posts, straddle_posts};
pub use error::RulesError;
pub use positions::{position_of, positions, postflop_order, preflop_order, ring, validate_table};
pub use settlement::Settlement;
pub use state::{abandon, apply_action, begin_hand, derive, is_decision_point, set_board, set_hero_cards, settle_pots, BeginHand};
```

- [ ] **Step 6: Run tests**

Run: `cargo test --workspace`
Expected: all pass (4 new lifecycle tests). If `lifecycle_streets_and_board` fails on `parse_cards("AsAs2c").unwrap_or_default()`, keep the assertion: `unwrap_or_default()` yields an empty board and `set_board` must reject it with `BadBoard` (wrong length).

- [ ] **Step 7: Commit**

```bash
git add crates/core-model
git commit -m "feat(core-model): hand lifecycle, settlement, conservation invariant and state API" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 12: `core-model` street root, `replay_root` and the §10.2 projection rule

**Files:**
- Create: `crates/core-model/src/street_root.rs`, `crates/core-model/tests/street_root.rs`
- Modify: `crates/core-model/src/lib.rs` (`pub mod street_root; pub use street_root::{replay_root, street_root, RootError};`)

**Interfaces:**
- Consumes: `Round`, `lifecycle::simulate`, `postflop_order`, `is_decision_point`, `proto::{StreetRootSnapshot, Derived, ...}`.
- Produces: `RootError::{Multiway { pot_eligible: u8 }, ProjectionNotReproducing { step: u32 }, NoDecision, Preflop, Inconsistent { step: u32 }}` (the spec's three variants plus `Preflop` for a preflop decision and `Inconsistent` for a genuine HU root that does not replay, which the engine maps to `EngineError`); `street_root(&HandState) -> Result<StreetRootSnapshot, RootError>`; `replay_root(&StreetRootSnapshot) -> Result<Derived, RulesError>` (replays `history` from the root as a HU street; the error names the failing 1-based step). Projection (§10.2): remove the players who folded on this street, drop their actions from `history`, add their current-street contributions to the root as `dead_this_street`, keep the survivors' actions in order; admitted only if the replay is legal at every step and reproduces the decision's total pot, both survivors' current-street contributions and remaining stacks, facing amount, `last_full_raise`, actor and legal action set.

- [ ] **Step 1: Write the failing tests** (`crates/core-model/tests/street_root.rs`)

```rust
use core_model::*;
use proto::*;

fn cfg(sb: u32, bb: u32) -> HandConfig {
    HandConfig { config_revision: 1, sb_chips: sb, bb_chips: bb, straddle: None, rake: Rake::TimeCharge, chip_label: "$1".into() }
}
fn seats(ids: &[u8]) -> Vec<Seat> { ids.iter().map(|i| Seat(*i)).collect() }
fn begin(cfg: &HandConfig, dealt: &[u8], stacks: &[u32]) -> HandState {
    begin_hand(cfg, BeginHand { hand_id: 1, button: Seat(5), hero: Seat(dealt[0]), dealt: seats(dealt), stacks_start: stacks.to_vec(), hero_cards: None }).unwrap()
}
/// The same state seen by `hero_seat` holding two cards.
fn seen_by(state: &HandState, hero_seat: u8) -> HandState {
    let mut s = state.clone();
    s.hero = Seat(hero_seat);
    s.hero_cards = Some(parse_hand("QhQd").unwrap());
    s
}
fn act(state: HandState, action: Action) -> HandState { apply_action(&state, action).unwrap() }
fn flop(state: HandState) -> HandState { set_board(&state, &parse_cards("Ks7c2s").unwrap()).unwrap() }

/// Blinds 25/50, BTN folds, SB completes, BB checks: flop root pot 100, stacks 500/500, OOP = SB(0), IP = BB(1).
fn hu_flop_root() -> HandState {
    let s = begin(&cfg(25, 50), &[5, 0, 1], &[600, 550, 550]);
    let s = act(s, Action::Fold);
    let s = act(s, Action::Call);
    let s = act(s, Action::Check);
    flop(s)
}

#[test]
fn street_root_reconstruction() {
    let s = hu_flop_root();
    let root = street_root(&seen_by(&s, 0)).unwrap();
    assert_eq!((root.pot_root, root.stack_oop_root, root.stack_ip_root, root.projected_from, root.dead_this_street), (100, 500, 500, 2, 0));
    assert_eq!((root.oop, root.ip), (Seat(0), Seat(1)));
    assert!(root.history.is_empty());
    let after_bet = act(s.clone(), Action::Bet { to: 50 });
    assert_eq!((after_bet.derived.pot, after_bet.derived.stacks_remaining[0], after_bet.derived.stacks_remaining[1]), (150, 450, 500));
    let snap = street_root(&seen_by(&after_bet, 1)).unwrap();
    assert_eq!((snap.pot_root, snap.stack_oop_root, snap.stack_ip_root), (100, 500, 500), "the root, never the current pot");
    assert_eq!(snap.history, vec![(Seat(0), Action::Bet { to: 50 })]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!((replayed.pot, replayed.to_act, replayed.facing), (150, Some(Seat(1)), 50));
    assert_eq!(replayed.legal, after_bet.derived.legal);
    let after_raise = act(after_bet, Action::Raise { to: 150 });
    assert_eq!((after_raise.derived.pot, after_raise.derived.stacks_remaining[0], after_raise.derived.stacks_remaining[1]), (300, 450, 350));
    assert!(after_raise.derived.legal.contains(&LegalAction::Call { cost: 100 }));
    let snap = street_root(&seen_by(&after_raise, 0)).unwrap();
    assert_eq!((snap.pot_root, snap.stack_oop_root, snap.stack_ip_root), (100, 500, 500));
    assert_eq!(snap.history.len(), 2);
    assert_eq!(replay_root(&snap).unwrap().pot, 300);
    let after_call = act(after_raise, Action::Call);
    assert_eq!((after_call.derived.pot, after_call.derived.stacks_remaining[0], after_call.derived.stacks_remaining[1]), (400, 350, 350));
    assert_eq!(after_call.phase, HandPhase::AwaitingBoard { street: Street::Turn });
    let turn = set_board(&after_call, &parse_cards("Ks7c2s9d").unwrap()).unwrap();
    let snap = street_root(&seen_by(&turn, 0)).unwrap();
    assert_eq!((snap.street, snap.pot_root, snap.stack_oop_root, snap.stack_ip_root), (Street::Turn, 400, 350, 350));
    // check-prefix: the actor changes at an unchanged pot
    let checked = act(hu_flop_root(), Action::Check);
    let snap = street_root(&seen_by(&checked, 1)).unwrap();
    assert_eq!(snap.history, vec![(Seat(0), Action::Check)]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!((replayed.pot, replayed.to_act), (100, Some(Seat(1))));
    assert_eq!(replayed.legal, checked.derived.legal);
    // hero all-in: no decision; the opponent facing the jam has one
    let jammed = act(hu_flop_root(), Action::AllIn { to: 500 });
    assert_eq!(street_root(&seen_by(&jammed, 0)), Err(RootError::NoDecision));
    let snap = street_root(&seen_by(&jammed, 1)).unwrap();
    assert_eq!(snap.history, vec![(Seat(0), Action::AllIn { to: 500 })]);
    assert_eq!(replay_root(&snap).unwrap().legal, vec![LegalAction::Fold, LegalAction::Call { cost: 500 }]);
    // preflop decisions have no street root
    let pre = begin(&cfg(25, 50), &[5, 0, 1], &[600, 550, 550]);
    assert_eq!(street_root(&seen_by(&pre, 5)), Err(RootError::Preflop));
    // a third player all-in preflop keeps the flop multiway although the side pot is HU
    let s = begin(&cfg(25, 50), &[5, 0, 1], &[100, 550, 550]);
    let s = act(s, Action::AllIn { to: 100 });
    let s = act(s, Action::Call);
    let s = act(s, Action::Call);
    assert_eq!(s.phase, HandPhase::AwaitingBoard { street: Street::Flop });
    let s = flop(s);
    assert_eq!(s.derived.to_act, Some(Seat(0)));
    assert_eq!(street_root(&seen_by(&s, 0)), Err(RootError::Multiway { pot_eligible: 3 }));
}

/// Postflop order A = SB(0), B = BB(1), C = BTN(5); every seat holds 1,000 at the flop root (limped pot of 6).
fn three_way_flop() -> HandState {
    let s = begin(&cfg(1, 2), &[5, 0, 1], &[1002, 1002, 1002]);
    let s = act(s, Action::Call);
    let s = act(s, Action::Call);
    let s = act(s, Action::Check);
    let s = flop(s);
    assert_eq!(s.derived.stacks_remaining, vec![1000, 1000, 0, 0, 0, 1000]);
    assert_eq!(s.derived.pot, 6);
    s
}

#[test]
fn multiway_root_projection() {
    // case 1: A bets 50, B folds, C raises to 150, A to act -> admitted, dead 0
    let s = act(act(act(three_way_flop(), Action::Bet { to: 50 }), Action::Fold), Action::Raise { to: 150 });
    let real = seen_by(&s, 0);
    let snap = street_root(&real).unwrap();
    assert_eq!((snap.projected_from, snap.dead_this_street, snap.pot_root), (3, 0, 6));
    assert_eq!(snap.history, vec![(Seat(0), Action::Bet { to: 50 }), (Seat(5), Action::Raise { to: 150 })]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!(replayed.pot, real.derived.pot);
    assert_eq!(replayed.legal, real.derived.legal);
    assert_eq!(replayed.legal, vec![LegalAction::Fold, LegalAction::Call { cost: 100 }, LegalAction::Raise { min_to: 250, max_to: 1000 }, LegalAction::AllIn { to: 1000 }]);
    // case 2: A bets 50, B calls, C raises to 150, A raises to 250, B folds, C to act -> admitted, dead 50, min re-raise 350
    let s = three_way_flop();
    let s = act(s, Action::Bet { to: 50 });
    let s = act(s, Action::Call);
    let s = act(s, Action::Raise { to: 150 });
    let s = act(s, Action::Raise { to: 250 });
    let s = act(s, Action::Fold);
    let real = seen_by(&s, 5);
    assert_eq!(real.derived.to_act, Some(Seat(5)));
    let snap = street_root(&real).unwrap();
    assert_eq!((snap.projected_from, snap.dead_this_street), (3, 50));
    assert_eq!(snap.history, vec![(Seat(0), Action::Bet { to: 50 }), (Seat(5), Action::Raise { to: 150 }), (Seat(0), Action::Raise { to: 250 })]);
    let replayed = replay_root(&snap).unwrap();
    assert_eq!(replayed.pot, real.derived.pot);
    assert_eq!(replayed.pot, 6 + 50 + 250 + 150);
    assert_eq!(replayed.facing, 250);
    assert_eq!(replayed.legal, real.derived.legal);
    assert!(replayed.legal.contains(&LegalAction::Call { cost: 100 }));
    assert!(replayed.legal.contains(&LegalAction::Raise { min_to: 350, max_to: 1000 }));
    // case 3: A bets 50, B calls, C raises to 150, A folds, B to act -> a call at an unbet root is illegal at step 1
    let s = three_way_flop();
    let s = act(s, Action::Bet { to: 50 });
    let s = act(s, Action::Call);
    let s = act(s, Action::Raise { to: 150 });
    let s = act(s, Action::Fold);
    let real = seen_by(&s, 1);
    assert_eq!(real.derived.to_act, Some(Seat(1)));
    assert_eq!(street_root(&real), Err(RootError::ProjectionNotReproducing { step: 1 }));
    // an out-of-turn sequence is rejected by the replay, never projected
    let mut tampered = s.clone();
    tampered.actions.swap(0, 1);
    assert!(core_model::lifecycle::simulate(&tampered).is_err());
    assert!(matches!(apply_action(&three_way_flop(), Action::Call), Err(RulesError::IllegalAction { .. })));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-model --test street_root`
Expected: compile error (`street_root`, `RootError` missing).

- [ ] **Step 3: Implement `street_root.rs`**

```rust
use proto::{Action, Derived, HandPhase, HandState, Pot, Seat, Street, StreetRootSnapshot, TakenAction};
use crate::betting::Round;
use crate::error::RulesError;
use crate::lifecycle::{simulate, Sim};
use crate::positions::postflop_order;
use crate::state::is_decision_point;

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RootError {
    #[error("{pot_eligible} pot-eligible players at the decision point")]
    Multiway { pot_eligible: u8 },
    #[error("multiway street root not reproducible at step {step}")]
    ProjectionNotReproducing { step: u32 },
    #[error("not a hero decision point")]
    NoDecision,
    #[error("street roots exist only postflop")]
    Preflop,
    #[error("genuine HU root does not replay to the decision state at step {step}")]
    Inconsistent { step: u32 },
}

/// The hand as it stood when `street` opened: stacks and settled pots at the street root.
fn simulate_street_start(state: &HandState, street: Street) -> Result<Sim, RulesError> {
    let mut prefix = state.clone();
    prefix.actions.retain(|a| a.street < street);
    prefix.board.truncate(street.board_len());
    prefix.phase = HandPhase::Betting { street };
    simulate(&prefix)
}

fn replay_root_steps(snap: &StreetRootSnapshot) -> Result<Derived, (u32, RulesError)> {
    let (o, i) = (snap.oop.0 as usize, snap.ip.0 as usize);
    let mut stacks = [0u32; 6];
    stacks[o] = snap.stack_oop_root;
    stacks[i] = snap.stack_ip_root;
    let mut folded = [true; 6];
    folded[o] = false;
    folded[i] = false;
    let mut round = Round::open(snap.street, vec![snap.oop, snap.ip], stacks, folded, [false; 6], snap.bb_chips);
    for (k, (seat, action)) in snap.history.iter().enumerate() {
        let step = k as u32 + 1;
        let (recorded, _) = round.apply(*seat, *action).map_err(|e| (step, e))?;
        if recorded != *action {
            return Err((step, RulesError::IllegalAction { reason: format!("step {step}: {action:?} replays as {recorded:?}") }));
        }
        if round.closed() && k + 1 < snap.history.len() { return Err((step + 1, RulesError::NotBetting)); }
    }
    let root_pot = snap.pot_root + snap.dead_this_street;
    let betting = !round.closed();
    Ok(Derived {
        street: snap.street,
        to_act: if betting { round.to_act() } else { None },
        pot: root_pot + round.committed.iter().sum::<u32>(),
        committed_this_street: round.committed.to_vec(),
        stacks_remaining: round.stacks.to_vec(),
        folded: round.folded.to_vec(),
        all_in: (0..6).map(|j| !round.folded[j] && round.stacks[j] == 0).collect(),
        facing: round.facing,
        last_full_raise: round.last_full_raise,
        pots: vec![Pot { amount: root_pot, eligible: vec![snap.oop.min(snap.ip), snap.oop.max(snap.ip)] }],
        legal: if betting { round.legal() } else { vec![] },
    })
}

/// Replays `history` from the snapshot as a HU street (spec 3.5); the error names the failing 1-based step.
pub fn replay_root(snap: &StreetRootSnapshot) -> Result<Derived, RulesError> {
    replay_root_steps(snap).map_err(|(step, e)| RulesError::IllegalAction { reason: format!("step {step}: {e}") })
}

/// The quantities of spec 10.2 that the replay must reproduce, plus the legal action set.
fn same_decision(a: &Derived, b: &Derived, oop: Seat, ip: Seat) -> bool {
    let (o, i) = (oop.0 as usize, ip.0 as usize);
    a.pot == b.pot
        && a.committed_this_street[o] == b.committed_this_street[o] && a.committed_this_street[i] == b.committed_this_street[i]
        && a.stacks_remaining[o] == b.stacks_remaining[o] && a.stacks_remaining[i] == b.stacks_remaining[i]
        && a.facing == b.facing && a.last_full_raise == b.last_full_raise && a.to_act == b.to_act && a.legal == b.legal
}

/// Spec 3.5 / 10.2: the financial street-root snapshot of the current hero decision, projected to HU when admissible.
pub fn street_root(state: &HandState) -> Result<StreetRootSnapshot, RootError> {
    if !is_decision_point(state) { return Err(RootError::NoDecision); }
    let d = &state.derived;
    let street = d.street;
    if street == Street::Preflop { return Err(RootError::Preflop); }
    let order = postflop_order(state.button, &state.dealt);
    let eligible: Vec<Seat> = order.iter().copied().filter(|s| !d.folded[s.0 as usize]).collect();
    if eligible.len() != 2 { return Err(RootError::Multiway { pot_eligible: eligible.len() as u8 }); }
    let (oop, ip) = (eligible[0], eligible[1]);
    let start = simulate_street_start(state, street).map_err(|_| RootError::Inconsistent { step: 0 })?;
    let n0 = start.round.eligible_count();
    let street_actions: Vec<&TakenAction> = state.actions.iter().filter(|a| a.street == street).collect();
    let dead: u32 = street_actions.iter().filter(|a| a.seat != oop && a.seat != ip).map(|a| a.paid).sum();
    let history: Vec<(Seat, Action)> = street_actions.iter().filter(|a| a.seat == oop || a.seat == ip).map(|a| (a.seat, a.action)).collect();
    let snapshot = StreetRootSnapshot {
        street,
        board: state.board.clone(),
        oop, ip,
        pot_root: start.pots.iter().map(|p| p.amount).sum(),
        stack_oop_root: start.round.stacks[oop.0 as usize],
        stack_ip_root: start.round.stacks[ip.0 as usize],
        dead_this_street: dead,
        projected_from: n0 as u8,
        history,
        bb_chips: state.config.bb_chips,
    };
    let genuine = n0 == 2;
    let fail = |step: u32| if genuine { RootError::Inconsistent { step } } else { RootError::ProjectionNotReproducing { step } };
    let replayed = replay_root_steps(&snapshot).map_err(|(step, _)| fail(step))?;
    if !same_decision(&replayed, d, oop, ip) { return Err(fail(snapshot.history.len() as u32 + 1)); }
    Ok(snapshot)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (2 new tests).

- [ ] **Step 5: Commit**

```bash
git add crates/core-model
git commit -m "feat(core-model): street root snapshot, replay_root and the multiway projection rule" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 13: `tools/` Python project and `gen_fixtures.py` (200 PokerKit hands)

**Files:**
- Create: `tools/pyproject.toml`, `tools/requirements.txt`, `tools/tests/conftest.py`, `tools/gen_fixtures.py`, `tools/tests/test_gen_fixtures.py`, `fixtures/hands/h0001.json` .. `fixtures/hands/h0200.json`

**Interfaces:**
- Consumes: PokerKit 0.7.5 (`NoLimitTexasHoldem.create_state`, `deal_hole`, `deal_board`, `fold`, `check_or_call`, `complete_bet_or_raise_to`, `pots`, `bets`, `stacks`, `statuses`, `actor_index`, `can_*`, `min/max_completion_betting_or_raising_to_amount`, `checking_or_calling_amount`, `completion_betting_or_raising_amount`).
- Produces: `generate(count, first_seed) -> (hands, dropped)` and one JSON file per hand with this schema (arrays "per dealt seat" are in `dealt` order = clockwise from the SB, button last, which is PokerKit's player order):

```
{"id":"h0001","seed":1,
 "config":{"sb_chips":1,"bb_chips":2,"straddle_chips":4|null},
 "button":5,"hero":1,"dealt":[3,4,5,0,1,2],"stacks_start":[40,17,300,14,200,200],
 "hole_cards":["JhAc",...],                          per dealt seat
 "steps":[
   {"kind":"action","seat":0,"street":"preflop","action":{"kind":"raise","to":8},"after":SNAP},
   {"kind":"board","cards":["Ah","Kd","2c"],"after":SNAP}],
 "returned":[[seat,amount],...],                     every refund in hand order
 "stats":{"allins":n,"short_allins":n,"max_pots":n,"allin_players":n}}
SNAP = {"phase":"betting"|"awaiting_board"|"complete","street":"preflop"|"flop"|"turn"|"river",
        "to_act":seat|null,"pot_total":n,"committed":[per dealt seat],"stacks":[..],"folded":[..],"all_in":[..],
        "pots":[{"amount":n,"eligible":[seats ascending]}],
        "legal":{"fold":bool,"check_or_call":{"cost":n}|null,"raise":{"min_to":n,"max_to":n}|null}|null,
        "final":"folded_out"|"all_in_runout"|"showdown_reached"   (complete only)}
```
`action.kind` is `fold|check|call|bet|raise|allin` with `to` = the actor's total street contribution (the same encoding as `proto::Action`). `phase`/`street` follow `core-model`: `awaiting_board` names the next street, `complete` keeps the closing street. At a fold-out the survivor's uncollected bet is normalized: the matched part joins the single pot and the rest is a refund (PokerKit leaves the whole bet in `bets`).

- [ ] **Step 1: Create the Python project files**

`tools/pyproject.toml`:
```toml
[project]
name = "pokerai-tools"
version = "0.1.0"
description = "Dev-only oracles and fixture generators for the PokerAI assistant (never a runtime dependency)"
requires-python = ">=3.12"
dependencies = ["pokerkit==0.7.5", "phevaluator==0.6.0", "pytest>=9.0"]

[tool.pytest.ini_options]
testpaths = ["tests"]
```

`tools/requirements.txt`:
```
pokerkit==0.7.5
phevaluator==0.6.0
pytest>=9.0
```

`tools/tests/conftest.py`:
```python
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
```

Create the environment (once):
```
python -m venv tools/.venv
tools/.venv/Scripts/python -m pip install -r tools/requirements.txt
```

- [ ] **Step 2: Write the failing tests** (`tools/tests/test_gen_fixtures.py`)

```python
import collections
import json

from gen_fixtures import CONFIGS, generate


def conserved(hand: dict) -> bool:
    total = sum(hand["stacks_start"])
    for step in hand["steps"]:
        a = step["after"]
        pots = sum(p["amount"] for p in a["pots"])
        if sum(a["stacks"]) + sum(a["committed"]) + pots != total:
            return False
        if a["pot_total"] != sum(a["committed"]) + pots:
            return False
    return True


def test_generated_set_covers_spec_classes():
    hands, dropped = generate(200, 1)
    assert len(hands) == 200
    assert dropped <= 10, "the PokerKit reopening divergence must stay rare"
    assert sum(1 for h in hands if h["config"]["straddle_chips"]) >= 80
    assert sum(1 for h in hands if h["stats"]["allin_players"] >= 2) >= 50
    assert sum(1 for h in hands if h["stats"]["allin_players"] >= 3) >= 15
    assert sum(1 for h in hands if h["stats"]["short_allins"] >= 1) >= 25
    assert sum(1 for h in hands if h["returned"]) >= 100
    assert sum(1 for h in hands if h["stats"]["max_pots"] >= 2) >= 60
    finals = collections.Counter(h["steps"][-1]["after"]["final"] for h in hands)
    assert finals["showdown_reached"] >= 15
    assert finals["folded_out"] >= 40
    assert finals["all_in_runout"] >= 40
    assert {len(h["dealt"]) for h in hands} == {3, 4, 5, 6}
    seen = {(h["config"]["sb_chips"], h["config"]["bb_chips"], h["config"]["straddle_chips"]) for h in hands}
    assert seen == {(sb, bb, s) for sb, bb, s, _, _ in CONFIGS}
    assert all(conserved(h) for h in hands)
    assert all(h["steps"][-1]["after"]["phase"] == "complete" for h in hands)


def test_generation_is_deterministic():
    assert json.dumps(generate(5, 1)[0]) == json.dumps(generate(5, 1)[0])


def test_straddle_min_raise_is_twice_the_straddle():
    hands, _ = generate(40, 1)
    checked = 0
    for h in hands:
        s = h["config"]["straddle_chips"]
        if not s:
            continue
        first = h["steps"][0]
        assert first["kind"] == "action"
        assert first["seat"] == h["dealt"][3], "HJ opens over the straddle"
        # the legal set the opener faced is the one PokerKit reported before the first action; reconstruct it from the config
        after = first["after"]
        if first["action"]["kind"] in ("call", "fold") and after["to_act"] is not None:
            rz = after["legal"]["raise"]
            assert rz is None or rz["min_to"] == 2 * s
            checked += 1
    assert checked >= 5


def test_fold_is_legal_only_when_facing_a_wager():
    hands, _ = generate(60, 1)
    for h in hands:
        for step in h["steps"]:
            a = step["after"]
            if a["legal"] is None:
                continue
            idx = h["dealt"].index(a["to_act"])
            facing = max(a["committed"])
            assert a["legal"]["fold"] == (facing > a["committed"][idx])
            cc = a["legal"]["check_or_call"]
            assert cc is not None and cc["cost"] == min(facing - a["committed"][idx], a["stacks"][idx])
```

- [ ] **Step 3: Run to verify failure**

Run: `tools/.venv/Scripts/python -m pytest tools -q`
Expected: `ModuleNotFoundError: No module named 'gen_fixtures'`.

- [ ] **Step 4: Write `tools/gen_fixtures.py`**

```python
"""Generate fixtures/hands/*.json: PokerKit hands for spec 13.1 `state_machine_pokerkit_fixtures`.

PokerKit is the oracle for pots, stacks, refunds and legal actions. Two normalizations are applied
(both verified on 2026-09-10): the minimum open over a straddle is 2S (PokerKit alone uses S + bb),
and at a fold-out the survivor's uncollected bet is split into the matched part (pot) and the refund.
Hands where PokerKit's reopening rule diverges from spec 4.3 (a full all-in raise followed by a short
all-in raise) are dropped.
"""
from __future__ import annotations

import argparse
import json
import random
import warnings
from pathlib import Path

from pokerkit import Automation, Mode, NoLimitTexasHoldem

warnings.simplefilter("ignore")  # Mode.CASH_GAME warns on folds that face no wager; never recorded as legal

AUTOMATIONS = (
    Automation.ANTE_POSTING,
    Automation.BET_COLLECTION,
    Automation.BLIND_OR_STRADDLE_POSTING,
    Automation.CARD_BURNING,
    Automation.RUNOUT_COUNT_SELECTION,
)
# (sb, bb, straddle, min dealt seats, max dealt seats)
CONFIGS = [(1, 2, None, 3, 6), (1, 2, 4, 6, 6), (2, 5, 10, 6, 6), (2, 5, None, 3, 6)]
STACK_BB = [7, 8.5, 12.5, 20, 30, 50, 100, 150, 200]
FOLD_P, WAGER_P, ALLIN_P = 0.25, 0.30, 0.06
STREETS = ["preflop", "flop", "turn", "river"]
RANKS = "23456789TJQKA"
SUITS = "cdhs"
DECK = [r + s for r in RANKS for s in SUITS]


class SpecReopen:
    """Spec 4.3 cumulative reopening, tracked per street to detect the PokerKit divergence."""

    def __init__(self, n: int, initial_full: int, facing: int) -> None:
        self.n = n
        self.start_street(initial_full, facing)

    def start_street(self, min_bet: int, facing: int) -> None:
        self.facing_at_last: list[int | None] = [None] * self.n
        self.last_full = min_bet
        self.facing = facing

    def may_raise(self, i: int) -> bool:
        f = self.facing_at_last[i]
        return f is None or self.facing - f >= self.last_full

    def acted(self, i: int, to: int | None) -> None:
        if to is not None:
            inc = to - self.facing
            if inc >= self.last_full:
                self.last_full = inc
            self.facing = to
        self.facing_at_last[i] = self.facing


def legal_triple(state) -> dict:
    i = state.actor_index
    facing = max(state.bets)
    owed = facing - state.bets[i]
    fold = bool(state.can_fold()) and owed > 0
    cc = {"cost": min(owed, state.stacks[i])} if state.can_check_or_call() else None
    assert cc is None or cc["cost"] == state.checking_or_calling_amount
    rz = None
    if state.can_complete_bet_or_raise_to():
        rz = {"min_to": state.min_completion_betting_or_raising_to_amount, "max_to": state.max_completion_betting_or_raising_to_amount}
    return {"fold": fold, "check_or_call": cc, "raise": rz}


def spec_may_aggress(state, reopen: SpecReopen, i: int) -> bool:
    facing = max(state.bets)
    if state.stacks[i] + state.bets[i] <= facing or not reopen.may_raise(i):
        return False
    return any(j != i and state.statuses[j] and state.stacks[j] + state.bets[j] > facing for j in range(len(state.stacks)))


def snapshot(state, ring, phase: str, street: str, pots_override=None, stacks_override=None) -> dict:
    n = len(ring)
    pots = pots_override if pots_override is not None else [
        {"amount": p.amount, "eligible": sorted(ring[j] for j in p.player_indices)} for p in state.pots
    ]
    stacks = stacks_override if stacks_override is not None else list(state.stacks)
    committed = [0] * n if pots_override is not None else list(state.bets)
    return {
        "phase": phase,
        "street": street,
        "to_act": ring[state.actor_index] if state.actor_index is not None else None,
        "pot_total": sum(p["amount"] for p in pots) + sum(committed),
        "committed": committed,
        "stacks": stacks,
        "folded": [not s for s in state.statuses],
        "all_in": [bool(state.statuses[j]) and stacks[j] == 0 for j in range(n)],
        "pots": pots,
        "legal": legal_triple(state) if state.actor_index is not None else None,
    }


def generate_hand(seed: int) -> dict | None:
    rng = random.Random(seed)
    sb, bb, straddle, lo, hi = rng.choice(CONFIGS)
    n = rng.randint(lo, hi)
    button = rng.randrange(6)
    others = [s for s in range(6) if s != button]
    dealt_set = set(rng.sample(others, n - 1)) | {button}
    ring = [s for s in ((button + k) % 6 for k in range(1, 7)) if s in dealt_set]  # SB, BB, ..., BTN
    stacks_start = [int(rng.choice(STACK_BB) * bb) for _ in range(n)]
    blinds = [sb, bb] + ([straddle] if straddle else []) + [0] * (n - 2 - (1 if straddle else 0))
    state = NoLimitTexasHoldem.create_state(AUTOMATIONS, False, 0, blinds, bb, stacks_start, n, mode=Mode.CASH_GAME)
    deck = DECK[:]
    rng.shuffle(deck)
    hole = [deck.pop() + deck.pop() for _ in range(n)]
    for h in hole:
        state.deal_hole(h)
    if straddle:
        # standard rule: the first full raise over a straddle is one straddle (min open 2S); must be set after hole dealing
        state.completion_betting_or_raising_amount = straddle
    reopen = SpecReopen(n, straddle or bb, max(state.bets))
    hero = rng.choice(ring)
    steps: list[dict] = []
    returned: list[list[int]] = []
    street_idx = 0
    n_allin = 0
    short_allin = 0

    while state.status:
        if state.actor_index is not None:
            i = state.actor_index
            legal = legal_triple(state)
            if (legal["raise"] is not None) != spec_may_aggress(state, reopen, i):
                return None  # documented PokerKit divergence: drop the hand
            facing = max(state.bets)
            r = rng.random()
            to = None
            if legal["raise"] is not None and r < ALLIN_P:
                to = legal["raise"]["max_to"]
                action = {"kind": "allin", "to": to}
            elif legal["raise"] is not None and r < ALLIN_P + WAGER_P:
                mn, mx = legal["raise"]["min_to"], legal["raise"]["max_to"]
                to = mn if rng.random() < 0.5 else rng.randint(mn, mx)
                action = {"kind": "allin", "to": to} if to == mx else {"kind": "bet" if facing == 0 else "raise", "to": to}
            elif legal["fold"] and r < ALLIN_P + WAGER_P + FOLD_P:
                action = {"kind": "fold"}
            else:
                action = {"kind": "check" if legal["check_or_call"]["cost"] == 0 else "call"}
            bets_before, stacks_before = list(state.bets), list(state.stacks)
            street = STREETS[street_idx]
            if action["kind"] == "fold":
                state.fold()
            elif action["kind"] in ("check", "call"):
                state.check_or_call()
            else:
                if action["kind"] == "allin":
                    n_allin += 1
                    if to < facing + reopen.last_full:
                        short_allin += 1
                state.complete_bet_or_raise_to(to)
            reopen.acted(i, to)
            step = {"kind": "action", "seat": ring[i], "street": street, "action": action}
            if state.actor_index is not None:
                step["after"] = snapshot(state, ring, "betting", street)
                steps.append(step)
                continue
            # the street closed with this action
            if sum(state.statuses) == 1:
                w = state.statuses.index(True)
                matched = max((bets_before[j] for j in range(n) if j != w), default=0)
                refund = bets_before[w] - matched
                stacks = list(state.stacks)
                stacks[w] += refund
                pots = [{"amount": sum(p.amount for p in state.pots) + matched, "eligible": [ring[w]]}]
                if refund > 0:
                    returned.append([ring[w], refund])
                step["after"] = snapshot(state, ring, "complete", street, pots_override=pots, stacks_override=stacks)
                step["after"]["final"] = "folded_out"
                steps.append(step)
                break
            after_pay = list(stacks_before)
            if action["kind"] == "call":
                after_pay[i] -= min(max(bets_before) - bets_before[i], stacks_before[i])
            elif action["kind"] in ("bet", "raise", "allin"):
                after_pay[i] -= to - bets_before[i]
            for j in range(n):
                if state.stacks[j] > after_pay[j]:
                    returned.append([ring[j], state.stacks[j] - after_pay[j]])
            with_chips = sum(1 for j in range(n) if state.statuses[j] and state.stacks[j] > 0)
            if with_chips < 2:
                step["after"] = snapshot(state, ring, "complete", street)
                step["after"]["final"] = "all_in_runout"
                steps.append(step)
                break
            if street_idx == 3:
                step["after"] = snapshot(state, ring, "complete", street)
                step["after"]["final"] = "showdown_reached"
                steps.append(step)
                break
            step["after"] = snapshot(state, ring, "awaiting_board", STREETS[street_idx + 1])
            steps.append(step)
        elif state.can_deal_board():
            count = 3 if street_idx == 0 else 1
            cards = [deck.pop() for _ in range(count)]
            state.deal_board("".join(cards))
            street_idx += 1
            reopen.start_street(bb, 0)
            steps.append({"kind": "board", "cards": cards, "after": snapshot(state, ring, "betting", STREETS[street_idx])})
        else:
            raise RuntimeError(f"unexpected PokerKit state at seed {seed}")

    return {
        "id": f"seed{seed}",
        "seed": seed,
        "config": {"sb_chips": sb, "bb_chips": bb, "straddle_chips": straddle},
        "button": button,
        "hero": hero,
        "dealt": ring,
        "stacks_start": stacks_start,
        "hole_cards": hole,
        "steps": steps,
        "returned": returned,
        "stats": {
            "allins": n_allin,
            "short_allins": short_allin,
            "max_pots": max(len(s["after"]["pots"]) for s in steps),
            "allin_players": sum(steps[-1]["after"]["all_in"]),
        },
    }


def generate(count: int, first_seed: int = 1) -> tuple[list[dict], int]:
    hands, dropped, seed = [], 0, first_seed
    while len(hands) < count:
        h = generate_hand(seed)
        seed += 1
        if h is None:
            dropped += 1
            continue
        hands.append(h)
    return hands, dropped


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    hands, dropped = generate(a.count, a.seed)
    a.out.mkdir(parents=True, exist_ok=True)
    for k, h in enumerate(hands, 1):
        h["id"] = f"h{k:04d}"
        (a.out / f"{h['id']}.json").write_text(json.dumps(h, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"wrote {len(hands)} hands to {a.out} ({dropped} seeds dropped)")


if __name__ == "__main__":
    main()
```

- [ ] **Step 5: Run the tests and generate the fixtures**

Run: `tools/.venv/Scripts/python -m pytest tools -q`
Expected: 4 passed (the run of 2026-09-10 with these constants gives 95 straddle hands, 65 with two all-in players, 25 with three, 36 short all-ins, 138 refunds, 86 side pots, 23 showdowns, 56 fold-outs, 121 runouts, 1 dropped seed).

Run: `tools/.venv/Scripts/python tools/gen_fixtures.py --out fixtures/hands`
Expected: `wrote 200 hands to fixtures/hands (1 seeds dropped)`; 200 files, about 1.2 MB in total.

- [ ] **Step 6: Commit**

```bash
git add tools fixtures/hands
git commit -m "feat(tools): PokerKit hand fixture generator and 200 fixtures" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 14: `core-model` replays the PokerKit fixtures

**Files:**
- Create: `crates/core-model/tests/pokerkit_fixtures.rs`

**Interfaces:**
- Consumes: `fixtures/hands/*.json` (Task 13 schema), `begin_hand`, `apply_action`, `set_board`, `settle_pots`, `parse_hand`, `parse_cards`.
- Produces: the test `state_machine_pokerkit_fixtures` (§13.1, V14).

- [ ] **Step 1: Write the test**

```rust
use core_model::*;
use proto::*;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
struct Fixture { id: String, config: FixCfg, button: u8, hero: u8, dealt: Vec<u8>, stacks_start: Vec<u32>, hole_cards: Vec<String>, steps: Vec<Step>, returned: Vec<(u8, u32)> }
#[derive(Deserialize)]
struct FixCfg { sb_chips: u32, bb_chips: u32, straddle_chips: Option<u32> }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Step { Action { seat: u8, street: String, action: Action, after: Snap }, Board { cards: Vec<String>, after: Snap } }
#[derive(Deserialize)]
struct Snap { phase: String, street: String, to_act: Option<u8>, pot_total: u32, committed: Vec<u32>, stacks: Vec<u32>, folded: Vec<bool>, all_in: Vec<bool>, pots: Vec<FixPot>, legal: Option<Legal>, #[serde(rename = "final")] final_reason: Option<String> }
#[derive(Deserialize)]
struct FixPot { amount: u32, eligible: Vec<u8> }
#[derive(Deserialize)]
struct Legal { fold: bool, check_or_call: Option<Cost>, raise: Option<MinMax> }
#[derive(Deserialize)]
struct Cost { cost: u32 }
#[derive(Deserialize)]
struct MinMax { min_to: u32, max_to: u32 }

fn street(name: &str) -> Street {
    match name { "preflop" => Street::Preflop, "flop" => Street::Flop, "turn" => Street::Turn, "river" => Street::River, other => panic!("street {other}") }
}

fn legal_triple(legal: &[LegalAction]) -> (bool, Option<u32>, Option<(u32, u32)>) {
    let fold = legal.contains(&LegalAction::Fold);
    let cc = legal.iter().find_map(|l| match l { LegalAction::Check => Some(0), LegalAction::Call { cost } => Some(*cost), _ => None });
    let mut raise = legal.iter().find_map(|l| match l { LegalAction::Bet { min_to, max_to } | LegalAction::Raise { min_to, max_to } => Some((*min_to, *max_to)), _ => None });
    if raise.is_none() { raise = legal.iter().find_map(|l| match l { LegalAction::AllIn { to } => Some((*to, *to)), _ => None }); }
    (fold, cc, raise)
}

fn check_snapshot(id: &str, k: usize, state: &HandState, snap: &Snap) {
    let d = &state.derived;
    let ctx = format!("{id} step {k}");
    let phase = match snap.phase.as_str() {
        "betting" => HandPhase::Betting { street: street(&snap.street) },
        "awaiting_board" => HandPhase::AwaitingBoard { street: street(&snap.street) },
        "complete" => HandPhase::Complete { reason: match snap.final_reason.as_deref() { Some("folded_out") => CompleteReason::FoldedOut, Some("all_in_runout") => CompleteReason::AllInRunout, Some("showdown_reached") => CompleteReason::ShowdownReached, other => panic!("{ctx}: final {other:?}") } },
        other => panic!("{ctx}: phase {other}"),
    };
    assert_eq!(state.phase, phase, "{ctx}: phase");
    assert_eq!(d.street, street(&snap.street), "{ctx}: street");
    assert_eq!(d.to_act, snap.to_act.map(Seat), "{ctx}: to_act");
    assert_eq!(d.pot, snap.pot_total, "{ctx}: pot");
    for (idx, seat) in state.dealt.iter().enumerate() {
        let s = seat.0 as usize;
        assert_eq!(d.committed_this_street[s], snap.committed[idx], "{ctx}: committed seat {s}");
        assert_eq!(d.stacks_remaining[s], snap.stacks[idx], "{ctx}: stack seat {s}");
        assert_eq!(d.folded[s], snap.folded[idx], "{ctx}: folded seat {s}");
        assert_eq!(d.all_in[s], snap.all_in[idx], "{ctx}: all_in seat {s}");
    }
    let pots: Vec<Pot> = snap.pots.iter().map(|p| Pot { amount: p.amount, eligible: p.eligible.iter().map(|s| Seat(*s)).collect() }).collect();
    assert_eq!(d.pots, pots, "{ctx}: pots");
    match &snap.legal {
        None => assert!(d.legal.is_empty(), "{ctx}: legal should be empty"),
        Some(l) => {
            let expected = (l.fold, l.check_or_call.as_ref().map(|c| c.cost), l.raise.as_ref().map(|r| (r.min_to, r.max_to)));
            assert_eq!(legal_triple(&d.legal), expected, "{ctx}: legal {:?}", d.legal);
        }
    }
}

#[test]
fn state_machine_pokerkit_fixtures() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hands");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).expect("fixtures/hands exists (Task 13)").map(|e| e.unwrap().path()).filter(|p| p.extension().map_or(false, |e| e == "json")).collect();
    files.sort();
    assert_eq!(files.len(), 200, "200 PokerKit hands");
    for file in files {
        let fx: Fixture = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let cfg = HandConfig { config_revision: 1, sb_chips: fx.config.sb_chips, bb_chips: fx.config.bb_chips, straddle: fx.config.straddle_chips.map(|a| UtgStraddle { amount_chips: a }), rake: Rake::TimeCharge, chip_label: "$1".into() };
        let hero_idx = fx.dealt.iter().position(|s| *s == fx.hero).unwrap();
        let mut state = begin_hand(&cfg, BeginHand { hand_id: 1, button: Seat(fx.button), hero: Seat(fx.hero), dealt: fx.dealt.iter().map(|s| Seat(*s)).collect(), stacks_start: fx.stacks_start.clone(), hero_cards: Some(parse_hand(&fx.hole_cards[hero_idx]).unwrap()) }).unwrap();
        let mut board: Vec<Card> = vec![];
        for (k, step) in fx.steps.iter().enumerate() {
            match step {
                Step::Action { seat, street: st, action, after } => {
                    assert_eq!(state.derived.to_act, Some(Seat(*seat)), "{} step {k}: actor", fx.id);
                    state = apply_action(&state, *action).unwrap_or_else(|e| panic!("{} step {k}: {action:?} rejected: {e}", fx.id));
                    let taken = state.actions.last().unwrap();
                    assert_eq!((taken.seat, taken.street, taken.action), (Seat(*seat), street(st), *action), "{} step {k}: recorded action", fx.id);
                    check_snapshot(&fx.id, k, &state, after);
                }
                Step::Board { cards, after } => {
                    for c in cards { board.push(c.parse().unwrap()); }
                    state = set_board(&state, &board).unwrap_or_else(|e| panic!("{} step {k}: board rejected: {e}", fx.id));
                    check_snapshot(&fx.id, k, &state, after);
                }
            }
        }
        assert!(matches!(state.phase, HandPhase::Complete { .. }), "{}: every fixture ends complete", fx.id);
        let returned: Vec<(Seat, u32)> = fx.returned.iter().map(|(s, a)| (Seat(*s), *a)).collect();
        assert_eq!(settle_pots(&state).returned, returned, "{}: refunds", fx.id);
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p core-model --test pokerkit_fixtures`
Expected: PASS (200/200). A failure names the fixture id and step; the fixture is the oracle: fix `core-model`, never the fixture (the only intentional differences are the two documented normalizations of Task 13, both already expressed in the fixture data).

- [ ] **Step 3: Commit**

```bash
git add crates/core-model/tests/pokerkit_fixtures.rs
git commit -m "test(core-model): replay the 200 PokerKit fixtures" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 15: `core-ranges` Pio range strings and 169-class expansion

**Files:**
- Create: `crates/core-ranges/Cargo.toml`, `crates/core-ranges/src/lib.rs`, `crates/core-ranges/src/parse.rs`, `crates/core-ranges/tests/ranges.rs`

**Interfaces:**
- Consumes: `proto::{Card, ComboIndex, Range1326, class_combos, class_of, combo_cards, combo_index, COMBOS, CLASSES}`.
- Produces: `RangeError::{Syntax(String), Weight(String)}`; `parse_range(&str) -> Result<Range1326, RangeError>` (grammar: comma- or whitespace-separated groups, each `body[:weight]`, weight `f32` in `[0, 1]`, later groups override earlier ones; bodies `random`, `AsKh`, `AA`, `AKs`, `AKo`, `AK`, `TT+`, `ATs+`, `T9o+` (connectors climb: T9o, JTo, QJo, KQo, AKo), `QQ-88`, `A9s-A6s`, `98o-65o`; dash ranges high-to-low only, ranks written high first); `range_to_string(&Range1326) -> String` (class tokens in 169-class order with `:w` when `w != 1`, per-combo tokens for classes with mixed weights, zero-weight combos omitted); `expand_169(&[f32; 169]) -> Range1326`; `class_name(class: u8) -> String`; `class_index(hi: u8, lo: u8, suited: bool) -> u8`.

- [ ] **Step 1: Write the failing tests** (`crates/core-ranges/tests/ranges.rs`)

```rust
use core_ranges::*;
use proto::*;

fn weight_of(r: &Range1326, text: &str) -> f32 {
    let cards = text.as_bytes();
    let a: Card = std::str::from_utf8(&cards[0..2]).unwrap().parse().unwrap();
    let b: Card = std::str::from_utf8(&cards[2..4]).unwrap().parse().unwrap();
    r.get(combo_index(a, b))
}

#[test]
fn range_roundtrip_pio_strings() {
    let r = parse_range("AKs:0.5, 77+, A5o").unwrap();
    assert_eq!(weight_of(&r, "AsKs"), 0.5);
    assert_eq!(weight_of(&r, "AsKh"), 0.0);
    assert_eq!(weight_of(&r, "7c7d"), 1.0);
    assert_eq!(weight_of(&r, "AcAd"), 1.0);
    assert_eq!(weight_of(&r, "6c6d"), 0.0);
    assert_eq!(weight_of(&r, "Ah5c"), 1.0);
    assert_eq!(weight_of(&r, "Ah5h"), 0.0);
    assert_eq!(mass(&r), 4.0 * 0.5 + 8.0 * 6.0 + 12.0);
    let text = range_to_string(&r);
    assert_eq!(text, "AA,AKs:0.5,KK,QQ,JJ,TT,99,88,77,A5o", "169-class row-major order: row A holds AA then AKs; A5o is row 5, column A");
    assert_eq!(parse_range(&text).unwrap(), r);
    assert_eq!(mass(&parse_range("QQ-88").unwrap()), 30.0);
    assert_eq!(mass(&parse_range("A9s-A6s").unwrap()), 16.0);
    assert_eq!(mass(&parse_range("98o-65o").unwrap()), 48.0);
    assert_eq!(mass(&parse_range("T9s+").unwrap()), 20.0, "T9s, JTs, QJs, KQs, AKs");
    assert_eq!(mass(&parse_range("ATs+").unwrap()), 16.0, "ATs, AJs, AQs, AKs");
    assert_eq!(mass(&parse_range("AK").unwrap()), 16.0);
    assert_eq!(mass(&parse_range("AsKh:0.25").unwrap()), 0.25);
    assert_eq!(mass(&parse_range("22+").unwrap()), 78.0);
    assert_eq!(mass(&parse_range("AA, AA:0.5").unwrap()), 3.0, "later groups override");
    assert!(matches!(parse_range("88-QQ"), Err(RangeError::Syntax(_))), "dash ranges are written high to low");
    assert!(parse_range("KA").is_err());
    assert!(parse_range("AKx").is_err());
    assert!(parse_range("AA:1.5").is_err());
    assert!(parse_range("AA:-0.1").is_err());
    assert!(parse_range("AA:nan").is_err());
    assert!(parse_range("A9s-K6s").is_err());
    // a class with mixed weights prints per combo and round-trips
    let mut mixed = parse_range("AKs").unwrap();
    mixed.set(combo_index("As".parse().unwrap(), "Ks".parse().unwrap()), 0.37);
    let text = range_to_string(&mixed);
    assert!(text.contains("AsKs:0.37") || text.contains("KsAs:0.37"), "{text}");
    assert_eq!(parse_range(&text).unwrap(), mixed);
    assert_eq!(range_to_string(&Range1326::zero()), "");
}

#[test]
fn class_expansion_multiplicity() {
    let mut classes = [0f32; 169];
    classes[class_index(12, 12, false) as usize] = 1.0; // AA
    classes[class_index(12, 11, true) as usize] = 0.5;  // AKs
    classes[class_index(12, 11, false) as usize] = 0.25; // AKo
    let r = expand_169(&classes);
    assert_eq!(mass(&r), 6.0 + 4.0 * 0.5 + 12.0 * 0.25);
    assert_eq!(r.0.iter().filter(|w| **w == 1.0).count(), 6);
    assert_eq!(r.0.iter().filter(|w| **w == 0.5).count(), 4);
    assert_eq!(r.0.iter().filter(|w| **w == 0.25).count(), 12);
    assert_eq!(class_name(0), "AA");
    assert_eq!(class_name(1), "AKs");
    assert_eq!(class_name(13), "AKo");
    assert_eq!(class_name(168), "22");
    assert_eq!(class_name(class_index(8, 7, false)), "T9o");
    assert_eq!(mass(&parse_range("random").unwrap()), 1326.0);
    assert_eq!(mass(&expand_169(&[1.0; 169])), 1326.0);
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-ranges`
Expected: "package core-ranges not found".

- [ ] **Step 3: Create the crate**

`crates/core-ranges/Cargo.toml`:
```toml
[package]
name = "core-ranges"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Pio-style range strings, 169-class expansion, blocking, hero conditioning and range hashing"

[dependencies]
proto = { path = "../proto" }
sha2 = { workspace = true }
thiserror = { workspace = true }
```

`crates/core-ranges/src/lib.rs`:
```rust
mod parse;

pub use parse::{class_index, class_name, expand_169, parse_range, range_to_string};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("range syntax: {0}")]
    Syntax(String),
    #[error("range weight: {0}")]
    Weight(String),
}

/// Total weight (accumulated in f64).
pub fn mass(r: &proto::Range1326) -> f32 { r.0.iter().map(|w| *w as f64).sum::<f64>() as f32 }
```

`crates/core-ranges/src/parse.rs`:
```rust
use proto::{class_combos, class_of, combo_cards, combo_index, Card, ComboIndex, Range1326, CLASSES, COMBOS};
use crate::RangeError;

const RANKS: &[u8; 13] = b"23456789TJQKA";

fn rank_of(c: char) -> Result<u8, RangeError> {
    RANKS.iter().position(|r| *r as char == c.to_ascii_uppercase()).map(|p| p as u8).ok_or_else(|| RangeError::Syntax(format!("unknown rank {c:?}")))
}

/// 169-class index for ranks `hi >= lo` (2 = 0 .. A = 12).
pub fn class_index(hi: u8, lo: u8, suited: bool) -> u8 {
    let (row_hi, row_lo) = (12 - hi, 12 - lo);
    if hi == lo { row_hi * 13 + row_hi } else if suited { row_hi * 13 + row_lo } else { row_lo * 13 + row_hi }
}

/// "AA", "AKs", "AKo" in the spec's grid order.
pub fn class_name(class: u8) -> String {
    let (i, j) = ((class / 13) as usize, (class % 13) as usize);
    let (ri, rj) = (RANKS[12 - i] as char, RANKS[12 - j] as char);
    if i == j { format!("{ri}{ri}") } else if i < j { format!("{ri}{rj}s") } else { format!("{rj}{ri}o") }
}

#[derive(Clone, Copy, PartialEq)]
enum Suits { Suited, Offsuit, Both }

/// Parses "AK", "AKs", "AKo", "AA" into (hi, lo, suitedness); ranks must be written high first.
fn parse_shape(body: &str) -> Result<(u8, u8, Suits), RangeError> {
    let chars: Vec<char> = body.chars().collect();
    let (a, b, suffix) = match chars.as_slice() {
        [a, b] => (*a, *b, None),
        [a, b, s] => (*a, *b, Some(s.to_ascii_lowercase())),
        _ => return Err(RangeError::Syntax(format!("cannot parse {body:?}"))),
    };
    let (hi, lo) = (rank_of(a)?, rank_of(b)?);
    if hi < lo { return Err(RangeError::Syntax(format!("{body:?}: write the higher rank first"))); }
    let suits = match suffix {
        None => Suits::Both,
        Some('s') if hi != lo => Suits::Suited,
        Some('o') if hi != lo => Suits::Offsuit,
        Some(other) => return Err(RangeError::Syntax(format!("{body:?}: unexpected suffix {other:?}"))),
    };
    Ok((hi, lo, suits))
}

fn classes_of(hi: u8, lo: u8, suits: Suits) -> Vec<u8> {
    if hi == lo { return vec![class_index(hi, lo, false)]; }
    match suits {
        Suits::Suited => vec![class_index(hi, lo, true)],
        Suits::Offsuit => vec![class_index(hi, lo, false)],
        Suits::Both => vec![class_index(hi, lo, true), class_index(hi, lo, false)],
    }
}

/// Expands one group body into combos.
fn expand_body(body: &str) -> Result<Vec<ComboIndex>, RangeError> {
    if body.eq_ignore_ascii_case("random") { return Ok((0..COMBOS as u16).collect()); }
    let chars: Vec<char> = body.chars().collect();
    if chars.len() == 4 && "cdhs".contains(chars[1].to_ascii_lowercase()) && "cdhs".contains(chars[3].to_ascii_lowercase()) {
        let a: Card = format!("{}{}", chars[0], chars[1]).parse().map_err(|e| RangeError::Syntax(format!("{e}")))?;
        let b: Card = format!("{}{}", chars[2], chars[3]).parse().map_err(|e| RangeError::Syntax(format!("{e}")))?;
        if a == b { return Err(RangeError::Syntax(format!("{body:?}: duplicate card"))); }
        return Ok(vec![combo_index(a, b)]);
    }
    let classes: Vec<u8> = if let Some(base) = body.strip_suffix('+') {
        let (hi, lo, suits) = parse_shape(base)?;
        if hi == lo { (hi..=12).flat_map(|r| classes_of(r, r, suits)).collect() }
        else if hi == lo + 1 { (lo..12).flat_map(|l| classes_of(l + 1, l, suits)).collect() }
        else { (lo..hi).flat_map(|l| classes_of(hi, l, suits)).collect() }
    } else if let Some((first, second)) = body.split_once('-') {
        let (h1, l1, s1) = parse_shape(first)?;
        let (h2, l2, s2) = parse_shape(second)?;
        if s1 != s2 { return Err(RangeError::Syntax(format!("{body:?}: mixed suitedness"))); }
        if h1 == l1 && h2 == l2 {
            if h1 < h2 { return Err(RangeError::Syntax(format!("{body:?}: dash ranges are written high to low"))); }
            (h2..=h1).flat_map(|r| classes_of(r, r, s1)).collect()
        } else if h1 == h2 {
            if l1 < l2 { return Err(RangeError::Syntax(format!("{body:?}: dash ranges are written high to low"))); }
            (l2..=l1).flat_map(|l| classes_of(h1, l, s1)).collect()
        } else if h1 - l1 == h2 - l2 {
            if h1 < h2 { return Err(RangeError::Syntax(format!("{body:?}: dash ranges are written high to low"))); }
            (h2..=h1).flat_map(|h| classes_of(h, h - (h1 - l1), s1)).collect()
        } else {
            return Err(RangeError::Syntax(format!("{body:?}: endpoints must share the high rank or the gap")));
        }
    } else {
        let (hi, lo, suits) = parse_shape(body)?;
        classes_of(hi, lo, suits)
    };
    Ok(classes.into_iter().flat_map(class_combos).collect())
}

fn parse_weight(text: &str) -> Result<f32, RangeError> {
    let w: f32 = text.trim().parse().map_err(|_| RangeError::Weight(format!("cannot parse weight {text:?}")))?;
    if !w.is_finite() || !(0.0..=1.0).contains(&w) { return Err(RangeError::Weight(format!("weight {text} is outside [0, 1]"))); }
    Ok(w)
}

/// Pio-style range string to `Range1326`; later groups override earlier ones.
pub fn parse_range(text: &str) -> Result<Range1326, RangeError> {
    let mut r = Range1326::zero();
    for token in text.split(|c: char| c == ',' || c.is_whitespace()).map(str::trim).filter(|t| !t.is_empty()) {
        let (body, weight) = match token.split_once(':') { Some((b, w)) => (b.trim(), parse_weight(w)?), None => (token, 1.0) };
        for combo in expand_body(body)? { r.set(combo, weight); }
    }
    Ok(r)
}

/// Deterministic text form in 169-class order; zero-weight combos are omitted.
pub fn range_to_string(r: &Range1326) -> String {
    let mut tokens: Vec<String> = Vec::new();
    for class in 0..CLASSES as u8 {
        let combos = class_combos(class);
        let first = r.get(combos[0]);
        if combos.iter().all(|c| r.get(*c) == first) {
            if first > 0.0 { tokens.push(if first == 1.0 { class_name(class) } else { format!("{}:{}", class_name(class), first) }); }
        } else {
            for c in combos {
                let w = r.get(c);
                if w > 0.0 {
                    let [lo, hi] = combo_cards(c);
                    tokens.push(if w == 1.0 { format!("{hi}{lo}") } else { format!("{hi}{lo}:{w}") });
                }
            }
        }
    }
    tokens.join(",")
}

/// Every combo of a class receives the class value (pair 6, suited 4, offsuit 12 combos).
pub fn expand_169(classes: &[f32; CLASSES]) -> Range1326 {
    Range1326::from_fn(|i| classes[class_of(i) as usize])
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (2 new).

- [ ] **Step 5: Commit**

```bash
git add crates/core-ranges
git commit -m "feat(core-ranges): Pio range parser, printer and 169-class expansion" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 16: `core-ranges` blocking, hero conditioning and `hash_scaled`

**Files:**
- Modify: `crates/core-ranges/src/lib.rs`, `crates/core-ranges/tests/ranges.rs`

**Interfaces:**
- Produces: `block_public(&mut Range1326, board: &[Card])` (zeroes combos containing a board card), `hero_conditioned(&Range1326, hero: [Card; 2]) -> Range1326` (copy with combos sharing a hero card zeroed; the input is untouched), `hash_scaled(&Range1326) -> [u8; 32]` (§2: each weight divided by the maximum weight, 1326 `f32` little-endian bit patterns hashed with sha256; an all-zero range hashes 1326 zero patterns).

- [ ] **Step 1: Write the failing tests** (append to `crates/core-ranges/tests/ranges.rs`)

```rust
fn cards(text: &str) -> Vec<Card> { text.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect() }

#[test]
fn public_blocking_board_only() {
    let mut r = Range1326::uniform();
    block_public(&mut r, &cards("AsKd2c"));
    assert_eq!(mass(&r), 1326.0 - 3.0 * 51.0 + 3.0, "combos containing a board card are zero; the three board pairs were counted twice");
    assert_eq!(weight_of(&r, "AsQh"), 0.0);
    assert_eq!(weight_of(&r, "QhQd"), 1.0, "hero's own cards keep their weight in a public range");
    assert_eq!(weight_of(&r, "KdKc"), 0.0);
}

#[test]
fn hero_conditioned_copy() {
    let mut public = Range1326::uniform();
    block_public(&mut public, &cards("AsKd2c"));
    let hero = hero_conditioned(&public, ["Qh".parse().unwrap(), "Qd".parse().unwrap()]);
    assert_eq!(weight_of(&hero, "QhJh"), 0.0);
    assert_eq!(weight_of(&hero, "QdQc"), 0.0);
    assert_eq!(weight_of(&hero, "JhJd"), 1.0);
    assert_eq!(weight_of(&public, "QhJh"), 1.0, "the public range is untouched");
    assert!(mass(&hero) < mass(&public));
}

#[test]
fn range_hash_scale_invariant() {
    let r = Range1326::from_fn(|i| ((i % 17) as f32 + 1.0) / 100.0); // max 0.17 <= 0.25, min 0.01 >= 2^-100
    let scaled = |k: f32| Range1326::from_fn(|i| r.get(i) * k);
    assert_eq!(hash_scaled(&r), hash_scaled(&scaled(0.5)));
    assert_eq!(hash_scaled(&r), hash_scaled(&scaled(4.0)));
    let _maybe_different = hash_scaled(&scaled(0.37)); // not asserted either way: an unequal hash is simply a miss
    let mut changed = r.clone();
    changed.set(5, changed.get(5) + 0.01);
    assert_ne!(hash_scaled(&changed), hash_scaled(&r), "a changed normalized bit pattern alters the hash");
    let one = |w: f32| Range1326::from_fn(|i| if i == 100 { w } else { 0.0 });
    assert_eq!(hash_scaled(&one(0.2)), hash_scaled(&one(0.9)), "a single supported combo normalizes to 1.0");
    assert_ne!(hash_scaled(&one(0.2)), hash_scaled(&Range1326::zero()));
    assert_eq!(hash_scaled(&Range1326::zero()), hash_scaled(&Range1326::zero()));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-ranges`
Expected: compile error (`block_public` etc. missing).

- [ ] **Step 3: Implement** (append to `lib.rs`)

```rust
use proto::{combo_cards, Card, Range1326, COMBOS};
use sha2::{Digest, Sha256};

/// Public blocking: combos containing a board card get weight 0 (never hero's cards; spec section 2).
pub fn block_public(r: &mut Range1326, board: &[Card]) {
    for i in 0..COMBOS {
        let [a, b] = combo_cards(i as u16);
        if board.contains(&a) || board.contains(&b) { r.0[i] = 0.0; }
    }
}

/// Hero-conditioned copy for equity and terminal calculations only.
pub fn hero_conditioned(r: &Range1326, hero: [Card; 2]) -> Range1326 {
    let mut out = r.clone();
    block_public(&mut out, &hero);
    out
}

/// Spec section 2 range hash: weights divided by the maximum (one f32 division each), bit patterns hashed with sha256.
pub fn hash_scaled(r: &Range1326) -> [u8; 32] {
    let max = r.0.iter().copied().fold(0.0f32, f32::max);
    let mut h = Sha256::new();
    for w in r.0.iter() {
        let v: f32 = if max > 0.0 { *w / max } else { 0.0 };
        h.update(v.to_le_bytes());
    }
    h.finalize().into()
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (3 new).

- [ ] **Step 5: Commit**

```bash
git add crates/core-ranges
git commit -m "feat(core-ranges): public blocking, hero-conditioned copies and hash_scaled" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 17: `core-iso` suit permutations and canonical boards

**Files:**
- Create: `crates/core-iso/Cargo.toml`, `crates/core-iso/src/lib.rs`, `crates/core-iso/tests/iso.rs`

**Interfaces:**
- Consumes: `proto::{Card, Range1326, combo_cards, combo_index, COMBOS}`; `core_ranges::parse_range` (tests only).
- Produces: `SuitPerm(pub [u8; 4])` (`perm.0[suit] = image suit`; `SuitPerm::IDENTITY`; derives `Ord`, serde), `ALL_PERMS: [[u8; 4]; 24]` in lexicographic order, `CanonicalBoard` (`cards(&self) -> &[Card]`, `flop(&self) -> &[Card]`; derives `Hash`, `Ord`, serde), `apply(&SuitPerm, Card) -> Card`, `inverse(&SuitPerm) -> SuitPerm`, `apply_range(&SuitPerm, &Range1326) -> Range1326`, `canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm)` (§2: the flop as an unordered set, turn and river in dealt order, minimal card-id key over the 24 permutations; ties broken by the lexicographically minimal serialized ranges as `f32` bit patterns in combo order, then the lexicographically minimal permutation), `orbit_size(&CanonicalBoard) -> u8` and `orbit_size_of(&[Card]) -> u8` (`24 / |stabilizer|`).

- [ ] **Step 1: Write the failing tests** (`crates/core-iso/tests/iso.rs`)

```rust
use core_iso::*;
use proto::*;
use std::collections::HashMap;

fn cards(text: &str) -> Vec<Card> { text.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect() }

/// Every raw flop grouped by canonical class, with the raw count per class.
fn classes() -> HashMap<CanonicalBoard, usize> {
    let mut map: HashMap<CanonicalBoard, usize> = HashMap::new();
    for a in 0..52u8 { for b in (a + 1)..52 { for c in (b + 1)..52 {
        let (cb, _) = canonicalize(&[Card(a), Card(b), Card(c)], &[]);
        *map.entry(cb).or_default() += 1;
    } } }
    map
}

#[test]
fn iso_class_count_1755() {
    assert_eq!(classes().len(), 1755);
    let (a, _) = canonicalize(&cards("AsKd2c"), &[]);
    let (b, _) = canonicalize(&cards("2cAsKd"), &[]);
    assert_eq!(a, b, "the flop is an unordered set");
}

#[test]
fn iso_orbit_sizes() {
    let map = classes();
    let (mut trips, mut paired, mut distinct, mut total) = (0usize, 0usize, 0usize, 0usize);
    for (cb, raw) in &map {
        let orbit = orbit_size(cb) as usize;
        assert!(matches!(orbit, 4 | 12 | 24), "{cb:?}: orbit {orbit}");
        assert_eq!(*raw, orbit, "{cb:?}: raw flops in the class equal the orbit size");
        total += orbit;
        let ranks: Vec<u8> = cb.cards().iter().map(|c| c.rank()).collect();
        let distinct_ranks = { let mut r = ranks.clone(); r.sort(); r.dedup(); r.len() };
        match distinct_ranks { 1 => trips += orbit, 2 => paired += orbit, _ => distinct += orbit }
    }
    assert_eq!((trips, paired, distinct, total), (52, 3744, 18304, 22100));
    assert_eq!(orbit_size_of(&cards("AsKsQs")), 4, "monotone");
    assert_eq!(orbit_size_of(&cards("AsKs2c")), 12, "two-tone");
    assert_eq!(orbit_size_of(&cards("AsKd2c")), 24, "rainbow");
    assert_eq!(orbit_size_of(&cards("AsAh2c")), 12, "paired");
    assert_eq!(orbit_size_of(&cards("AsAhAd")), 4, "trips");
}

#[test]
fn iso_stabilizer_tiebreak() {
    let r1 = Range1326::from_fn(|i| ((i as u32 * 7919) % 101) as f32 / 100.0);
    let r2 = Range1326::from_fn(|i| ((i as u32 * 104729 + 7) % 89) as f32 / 88.0);
    for board in [cards("AsAh2c"), cards("AsKsQs"), cards("AsAh2c7d"), cards("KsKhKd")] {
        let (cb, p) = canonicalize(&board, &[&r1, &r2]);
        let key1 = apply_range(&p, &r1);
        let key2 = apply_range(&p, &r2);
        for tau in ALL_PERMS {
            let tau = SuitPerm(tau);
            let permuted: Vec<Card> = board.iter().map(|c| apply(&tau, *c)).collect();
            let (cb2, p2) = canonicalize(&permuted, &[&apply_range(&tau, &r1), &apply_range(&tau, &r2)]);
            assert_eq!(cb2, cb, "{board:?} under {tau:?}");
            assert_eq!(apply_range(&p2, &apply_range(&tau, &r1)), key1, "{board:?} under {tau:?}: oop range key");
            assert_eq!(apply_range(&p2, &apply_range(&tau, &r2)), key2, "{board:?} under {tau:?}: ip range key");
        }
    }
    let (a, _) = canonicalize(&cards("AhKd2c7s7h"), &[]);
    let (b, _) = canonicalize(&cards("AhKd2c7h7s"), &[]);
    assert_ne!(a, b, "turn and river keep their dealt order");
    let (a, _) = canonicalize(&cards("AhKd2c7s"), &[]);
    let (b, _) = canonicalize(&cards("AhKd2c7h"), &[]);
    assert_eq!(a, b, "the turn is canonicalized within the flop's stabilizer");
    for p in ALL_PERMS {
        let p = SuitPerm(p);
        assert_eq!(apply_range(&inverse(&p), &apply_range(&p, &r1)), r1);
        for c in Card::all() { assert_eq!(apply(&inverse(&p), apply(&p, c)), c); }
    }
    let (_, p) = canonicalize(&cards("AsKd2c"), &[]);
    let (_, q) = canonicalize(&cards("AsKd2c"), &[&r1, &r2]);
    assert_eq!(p, q, "a rainbow flop has a trivial stabilizer: the ranges cannot change the permutation");
    assert_eq!(canonicalize(&cards("AsAh2c"), &[]).1, canonicalize(&cards("AsAh2c"), &[]).1, "deterministic");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-iso`
Expected: "package core-iso not found".

- [ ] **Step 3: Create the crate**

`crates/core-iso/Cargo.toml`:
```toml
[package]
name = "core-iso"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "Board canonicalization under suit permutations with the stabilizer tie-break (spec section 2)"

[dependencies]
proto = { path = "../proto" }
core-ranges = { path = "../core-ranges" }
serde = { workspace = true }
```

`crates/core-iso/src/lib.rs`:
```rust
use proto::{combo_cards, combo_index, Card, Range1326, COMBOS};
use serde::{Deserialize, Serialize};

/// A suit permutation: `perm.0[suit] = image suit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SuitPerm(pub [u8; 4]);

impl SuitPerm {
    pub const IDENTITY: SuitPerm = SuitPerm([0, 1, 2, 3]);
}

/// All 24 suit permutations in lexicographic order.
pub const ALL_PERMS: [[u8; 4]; 24] = [
    [0, 1, 2, 3], [0, 1, 3, 2], [0, 2, 1, 3], [0, 2, 3, 1], [0, 3, 1, 2], [0, 3, 2, 1],
    [1, 0, 2, 3], [1, 0, 3, 2], [1, 2, 0, 3], [1, 2, 3, 0], [1, 3, 0, 2], [1, 3, 2, 0],
    [2, 0, 1, 3], [2, 0, 3, 1], [2, 1, 0, 3], [2, 1, 3, 0], [2, 3, 0, 1], [2, 3, 1, 0],
    [3, 0, 1, 2], [3, 0, 2, 1], [3, 1, 0, 2], [3, 1, 2, 0], [3, 2, 0, 1], [3, 2, 1, 0],
];

/// Canonical board: the flop sorted ascending by card id, then turn and river in dealt order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CanonicalBoard { cards: Vec<Card> }

impl CanonicalBoard {
    pub fn cards(&self) -> &[Card] { &self.cards }
    pub fn flop(&self) -> &[Card] { &self.cards[..3] }
}

pub fn apply(p: &SuitPerm, c: Card) -> Card { Card::new(c.rank(), p.0[c.suit() as usize]) }

pub fn inverse(p: &SuitPerm) -> SuitPerm {
    let mut inv = [0u8; 4];
    for (suit, image) in p.0.iter().enumerate() { inv[*image as usize] = suit as u8; }
    SuitPerm(inv)
}

/// Moves the weight of combo (a, b) to combo (p(a), p(b)).
pub fn apply_range(p: &SuitPerm, r: &Range1326) -> Range1326 {
    let mut out = Range1326::zero();
    for i in 0..COMBOS {
        let w = r.0[i];
        if w != 0.0 {
            let [a, b] = combo_cards(i as u16);
            out.set(combo_index(apply(p, a), apply(p, b)), w);
        }
    }
    out
}

fn board_key(p: &SuitPerm, board: &[Card]) -> Vec<u8> {
    let mut key: Vec<u8> = board[..3].iter().map(|c| apply(p, *c).0).collect();
    key.sort_unstable();
    key.extend(board[3..].iter().map(|c| apply(p, *c).0));
    key
}

/// The `(oop, ip)` tuple serialized as f32 bit patterns in combo order (spec section 2 tie-break).
fn serialized(p: &SuitPerm, ranges: &[&Range1326]) -> Vec<u32> {
    ranges.iter().flat_map(|r| apply_range(p, r).0.into_iter().map(f32::to_bits).collect::<Vec<u32>>()).collect()
}

/// Spec section 2: minimal board key over the 24 permutations; ties broken by the minimal serialized ranges, then the minimal permutation.
pub fn canonicalize(board: &[Card], ranges: &[&Range1326]) -> (CanonicalBoard, SuitPerm) {
    assert!((3..=5).contains(&board.len()), "canonicalize needs 3 to 5 board cards");
    let mut best_key: Option<Vec<u8>> = None;
    let mut candidates: Vec<SuitPerm> = Vec::new();
    for p in ALL_PERMS {
        let perm = SuitPerm(p);
        let key = board_key(&perm, board);
        match &best_key {
            Some(k) if key > *k => {}
            Some(k) if key == *k => candidates.push(perm),
            _ => { best_key = Some(key); candidates = vec![perm]; }
        }
    }
    let key = best_key.expect("at least one permutation");
    let mut chosen = candidates[0];
    if candidates.len() > 1 && !ranges.is_empty() {
        let mut best = serialized(&chosen, ranges);
        for p in &candidates[1..] {
            let ser = serialized(p, ranges);
            if ser < best { best = ser; chosen = *p; }
        }
    }
    (CanonicalBoard { cards: key.into_iter().map(Card).collect() }, chosen)
}

/// `24 / |stabilizer|` of the board (flop as a set, turn and river ordered).
pub fn orbit_size_of(board: &[Card]) -> u8 {
    let identity = board_key(&SuitPerm::IDENTITY, board);
    let stabilizer = ALL_PERMS.iter().filter(|p| board_key(&SuitPerm(**p), board) == identity).count();
    (24 / stabilizer) as u8
}

pub fn orbit_size(board: &CanonicalBoard) -> u8 { orbit_size_of(board.cards()) }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (3 new; `iso_class_count_1755` and `iso_orbit_sizes` each canonicalize 22,100 flops with 24 permutations, about one second in the `dev` profile).

- [ ] **Step 5: Commit**

```bash
git add crates/core-iso
git commit -m "feat(core-iso): suit permutations, canonical boards and orbit sizes" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 18: `tools/gen_eval_oracle.py` and the phevaluator fixtures

**Files:**
- Create: `tools/gen_eval_oracle.py`, `tools/tests/test_gen_eval_oracle.py`, `fixtures/eval/phevaluator_5card.bin`, `fixtures/eval/phevaluator_7card_200k.bin`

**Interfaces:**
- Consumes: `phevaluator.evaluate_cards(*cards) -> int` (lower is stronger, 1..7462; accepts ids `rank*4 + suit`, the spec's encoding).
- Produces: `fixtures/eval/phevaluator_5card.bin` = 2,598,960 little-endian `u16` ranks, one per 5-card combination in `itertools.combinations(range(52), 5)` order (ascending card ids); `fixtures/eval/phevaluator_7card_200k.bin` = 200,000 records of 9 bytes: 7 card ids (as sampled, unsorted) then the `u16` LE rank; seed 20260910. `write_5card(out)`, `write_7card_samples(out, count, seed)`.

- [ ] **Step 1: Write the failing tests** (`tools/tests/test_gen_eval_oracle.py`)

```python
import itertools
import struct
from pathlib import Path

from phevaluator import evaluate_cards

from gen_eval_oracle import write_7card_samples, five_card_prefix

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures" / "eval"


def test_phevaluator_encoding_matches_spec():
    assert evaluate_cards("As", "Ks", "Qs", "Js", "Ts") == 1
    assert evaluate_cards(51, 47, 43, 39, 35) == 1, "id = rank*4 + suit with c,d,h,s = 0..3"
    assert evaluate_cards("7h", "5d", "4c", "3s", "2h") == 7462
    assert evaluate_cards(0, 1, 2, 3, 4) < evaluate_cards(0, 1, 2, 4, 5), "quads beat a full house (lower is stronger)"


def test_seven_card_sample_format(tmp_path):
    out = tmp_path / "s.bin"
    write_7card_samples(out, 10, 20260910)
    data = out.read_bytes()
    assert len(data) == 90
    for k in range(10):
        rec = data[9 * k: 9 * k + 9]
        cards = list(rec[:7])
        assert len(set(cards)) == 7 and all(0 <= c < 52 for c in cards)
        assert struct.unpack("<H", rec[7:])[0] == evaluate_cards(*cards)
    assert write_7card_samples(tmp_path / "t.bin", 10, 20260910) is None
    assert (tmp_path / "t.bin").read_bytes() == data, "deterministic"


def test_five_card_prefix_matches_committed_file():
    prefix = five_card_prefix(1000)
    assert len(prefix) == 2000
    combos = list(itertools.islice(itertools.combinations(range(52), 5), 1000))
    assert struct.unpack("<H", prefix[:2])[0] == evaluate_cards(*combos[0])
    committed = FIXTURES / "phevaluator_5card.bin"
    if committed.exists():
        assert committed.stat().st_size == 2_598_960 * 2
        with committed.open("rb") as f:
            assert f.read(2000) == prefix
```

- [ ] **Step 2: Run to verify failure**

Run: `tools/.venv/Scripts/python -m pytest tools/tests/test_gen_eval_oracle.py -q`
Expected: `ModuleNotFoundError: No module named 'gen_eval_oracle'`.

- [ ] **Step 3: Write `tools/gen_eval_oracle.py`**

```python
"""Generate fixtures/eval oracles with phevaluator (spec 13.0, 13.1).

phevaluator_5card.bin: 2,598,960 little-endian u16 ranks, one per 5-card combination in
itertools.combinations(range(52), 5) order. phevaluator_7card_<n>.bin: records of 9 bytes,
seven card ids (rank*4 + suit) as sampled, then the u16 LE rank. Lower rank = stronger hand.
"""
from __future__ import annotations

import argparse
import itertools
import random
import struct
from pathlib import Path

from phevaluator import evaluate_cards

FIVE_CARD_COUNT = 2_598_960


def five_card_prefix(count: int) -> bytes:
    out = bytearray()
    for combo in itertools.islice(itertools.combinations(range(52), 5), count):
        out += struct.pack("<H", evaluate_cards(*combo))
    return bytes(out)


def write_5card(out: Path) -> None:
    with out.open("wb") as f:
        for combo in itertools.combinations(range(52), 5):
            f.write(struct.pack("<H", evaluate_cards(*combo)))


def write_7card_samples(out: Path, count: int, seed: int) -> None:
    rng = random.Random(seed)
    with out.open("wb") as f:
        for _ in range(count):
            cards = rng.sample(range(52), 7)
            f.write(bytes(cards) + struct.pack("<H", evaluate_cards(*cards)))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out-dir", type=Path, default=Path("fixtures/eval"))
    ap.add_argument("--samples", type=int, default=200_000)
    ap.add_argument("--samples-name", default="phevaluator_7card_200k.bin")
    ap.add_argument("--seed", type=int, default=20260910)
    ap.add_argument("--skip-5card", action="store_true")
    a = ap.parse_args()
    a.out_dir.mkdir(parents=True, exist_ok=True)
    if not a.skip_5card:
        write_5card(a.out_dir / "phevaluator_5card.bin")
        print(f"wrote {FIVE_CARD_COUNT} five-card ranks")
    write_7card_samples(a.out_dir / a.samples_name, a.samples, a.seed)
    print(f"wrote {a.samples} seven-card samples to {a.samples_name}")


if __name__ == "__main__":
    main()
```

- [ ] **Step 4: Run the tests and generate the fixtures**

Run: `tools/.venv/Scripts/python -m pytest tools -q`
Expected: all pass (the committed-file comparison is skipped until the file exists).

Run: `tools/.venv/Scripts/python tools/gen_eval_oracle.py --out-dir fixtures/eval`
Expected: `wrote 2598960 five-card ranks` (about 10-30 s) and `wrote 200000 seven-card samples to phevaluator_7card_200k.bin`; file sizes 5,197,920 and 1,800,000 bytes.

Run again: `tools/.venv/Scripts/python -m pytest tools -q`
Expected: all pass, including the prefix comparison against the committed file.

- [ ] **Step 5: Commit**

```bash
git add tools/gen_eval_oracle.py tools/tests/test_gen_eval_oracle.py fixtures/eval
git commit -m "feat(tools): phevaluator oracle generator and eval fixtures" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 19: `core-eval` evaluator trait, b-inary backend and oracle tests

**Files:**
- Create: `crates/core-eval/Cargo.toml`, `crates/core-eval/src/lib.rs`, `crates/core-eval/src/evaluator.rs`, `crates/core-eval/tests/oracle.rs`

**Interfaces:**
- Consumes: `holdem_hand_evaluator::Hand`, `proto::Card`, `fixtures/eval/*.bin` (Task 18).
- Produces: `trait Evaluator: Send + Sync { type Partial: Clone + Send + Sync; fn partial(&self, cards: &[Card]) -> Self::Partial; fn rank_with(&self, partial: &Self::Partial, extra: &[Card]) -> u16; fn rank(&self, cards: &[Card]) -> u16 }` (ranks are "higher is stronger"; 5 to 7 cards), `BinaryEvaluator` (the b-inary backend; `rs_poker` is the named contingency and is not implemented), `rank7(&[Card; 7]) -> u16`, `rank5(&[Card; 5]) -> u16`, `rank(&[Card]) -> u16`. Feature `exhaustive` gates the 10,000,000-sample test.

- [ ] **Step 1: Write the failing tests** (`crates/core-eval/tests/oracle.rs`)

```rust
use core_eval::*;
use proto::Card;
use std::path::PathBuf;

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/eval").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run tools/gen_eval_oracle.py)", path.display()))
}

/// phevaluator rank (1..7462) -> our rank, built from the full 5-card oracle; panics on any inconsistency.
fn oracle_map() -> Vec<u16> {
    let data = fixture("phevaluator_5card.bin");
    assert_eq!(data.len(), 2_598_960 * 2);
    let mut map: Vec<Option<u16>> = vec![None; 7463];
    let mut k = 0usize;
    for a in 0..52u8 { for b in (a + 1)..52 { for c in (b + 1)..52 { for d in (c + 1)..52 { for e in (d + 1)..52 {
        let oracle = u16::from_le_bytes([data[2 * k], data[2 * k + 1]]) as usize;
        k += 1;
        let ours = rank5(&[Card(a), Card(b), Card(c), Card(d), Card(e)]);
        match map[oracle] {
            None => map[oracle] = Some(ours),
            Some(prev) => assert_eq!(prev, ours, "phevaluator rank {oracle} maps to two different ranks"),
        }
    } } } } }
    assert_eq!(k, 2_598_960);
    (0..=7462).map(|o| if o == 0 { 0 } else { map[o].expect("every phevaluator rank occurs") }).collect()
}

#[test]
fn eval_vs_phevaluator_full_5card() {
    let map = oracle_map();
    for o in 1..7462 {
        assert!(map[o] > map[o + 1], "rank-order equivalence: phevaluator {o} (stronger) must map above {} ", o + 1);
    }
    assert_eq!(rank5(&["As", "Ks", "Qs", "Js", "Ts"].map(|s| s.parse().unwrap())), map[1]);
    assert_eq!(rank7(&["As", "Ks", "Qs", "Js", "Ts", "2c", "3d"].map(|s| s.parse().unwrap())), map[1]);
}

fn check_samples(name: &str, expected_records: usize) {
    let map = oracle_map();
    let data = fixture(name);
    assert_eq!(data.len(), expected_records * 9, "{name}: 9-byte records");
    for rec in data.chunks_exact(9) {
        let cards: [Card; 7] = std::array::from_fn(|i| Card(rec[i]));
        let oracle = u16::from_le_bytes([rec[7], rec[8]]) as usize;
        assert_eq!(rank7(&cards), map[oracle], "{cards:?}");
    }
}

#[test]
fn eval_vs_phevaluator_random_7card() { check_samples("phevaluator_7card_200k.bin", 200_000); }

/// Generate the input with `tools/.venv/Scripts/python tools/gen_eval_oracle.py --skip-5card --samples 10000000 --samples-name phevaluator_7card_10m.bin` (gitignored).
#[cfg(feature = "exhaustive")]
#[test]
fn eval_vs_phevaluator_random_7card_exhaustive() { check_samples("phevaluator_7card_10m.bin", 10_000_000); }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-eval`
Expected: "package core-eval not found".

- [ ] **Step 3: Create the crate**

`crates/core-eval/Cargo.toml`:
```toml
[package]
name = "core-eval"
version.workspace = true
edition.workspace = true
license.workspace = true
rust-version.workspace = true
description = "7-card evaluator and weighted range-vs-range equity (exact and Monte Carlo)"

[features]
exhaustive = []

[dependencies]
proto = { path = "../proto" }
core-ranges = { path = "../core-ranges" }
holdem-hand-evaluator = { workspace = true }
```

`crates/core-eval/src/evaluator.rs`:
```rust
use holdem_hand_evaluator::Hand;
use proto::Card;

/// Hand-rank backend: ranks are "higher is stronger"; 5 to 7 cards in total.
pub trait Evaluator: Send + Sync {
    type Partial: Clone + Send + Sync;
    /// Pre-combines 0 to 5 cards (a board) for repeated evaluation.
    fn partial(&self, cards: &[Card]) -> Self::Partial;
    /// Rank of `partial` plus `extra` (5 to 7 cards in total).
    fn rank_with(&self, partial: &Self::Partial, extra: &[Card]) -> u16;
    fn rank(&self, cards: &[Card]) -> u16 { self.rank_with(&self.partial(&[]), cards) }
}

/// b-inary/holdem-hand-evaluator (MIT); `rs_poker` is the named contingency only if V1 fails.
#[derive(Clone, Copy, Debug, Default)]
pub struct BinaryEvaluator;

impl Evaluator for BinaryEvaluator {
    type Partial = Hand;
    fn partial(&self, cards: &[Card]) -> Hand {
        let ids: Vec<usize> = cards.iter().map(|c| c.0 as usize).collect();
        Hand::from_slice(&ids)
    }
    fn rank_with(&self, partial: &Hand, extra: &[Card]) -> u16 {
        let mut hand = *partial;
        for c in extra { hand = hand.add_card(c.0 as usize); }
        debug_assert!((5..=7).contains(&hand.len()), "evaluate needs 5 to 7 cards");
        hand.evaluate()
    }
}
```

`crates/core-eval/src/lib.rs`:
```rust
pub mod evaluator;

pub use evaluator::{BinaryEvaluator, Evaluator};
use proto::Card;

pub fn rank(cards: &[Card]) -> u16 { BinaryEvaluator.rank(cards) }
pub fn rank5(cards: &[Card; 5]) -> u16 { BinaryEvaluator.rank(cards) }
pub fn rank7(cards: &[Card; 7]) -> u16 { BinaryEvaluator.rank(cards) }
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (2 new; each builds the 2.6M-hand map, about a second with the evaluator at `opt-level = 3`).

Run: `cargo test -p core-eval --features exhaustive`
Expected: the exhaustive test fails with the "run tools/gen_eval_oracle.py" message until the 10M file is generated; with the file present it passes (about 30 s).

- [ ] **Step 5: Commit**

```bash
git add crates/core-eval
git commit -m "feat(core-eval): evaluator trait with the b-inary backend and phevaluator oracle tests" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
### Task 20: `core-eval` exact equity, per-combo equity and terminal payoffs

**Files:**
- Create: `crates/core-eval/src/equity.rs`, `crates/core-eval/tests/equity.rs`
- Modify: `crates/core-eval/src/lib.rs`

**Interfaces:**
- Consumes: `Evaluator`/`BinaryEvaluator` (Task 19), `proto::{Card, ComboIndex, EquityMethod, Rake, Range1326, Seat, combo_cards, COMBOS}`, `core_ranges::{parse_range, block_public}` (tests).
- Produces: `PlayerRange { seat: Seat, range: Range1326 }` (a fixed hero combo is a range with one supported combo), `EquityMode::{Exact, MonteCarlo { seed: u64, max_samples: u32 }}`, `PotEligibility { pot_index: u8, eligible: Vec<Seat> }`, `EquityRequest { board: Vec<Card>, players: Vec<PlayerRange>, mode: EquityMode, pots: Vec<PotEligibility> }` (+ `EquityRequest::single_pot(board, players, mode)`; empty `pots` = one pot with every player eligible), `EquityStatus::{Ready, Cancelled, BudgetExceeded, InvalidRanges}`, `EquityShare { pot_index, seat, value: f32, std_err: f32 }`, `EquityResult { status, method: Option<EquityMethod>, shares: Vec<EquityShare>, samples: u64, elapsed: Duration }`, `equity(&EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult`, `exact_cost(&EquityRequest) -> u64` (product of support sizes × runouts; the engine applies the §7 rule `<= 2 * 10^7`), `per_combo_equity(hero: &Range1326, villain: &Range1326, board: &[Card]) -> [f32; 1326]` (exact, hero combo fixed against the villain's disjoint weighted combos, 0 for unsupported hero combos), `terminal_payoff(equity: f32, pot: u32, rake: &Rake) -> f32` = `equity * (pot - min(rate * pot, cap))`, 0 rake for `TimeCharge`. Joint weighting: every disjoint tuple of combos has weight = the product of its weights; runouts are uniform; a tie splits the pot equally among the tied eligible players. Cancel flag and clock are checked every 4096 evaluations.

- [ ] **Step 1: Write the failing tests** (`crates/core-eval/tests/equity.rs`)

```rust
use core_eval::*;
use core_ranges::{block_public, parse_range};
use proto::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn cards(text: &str) -> Vec<Card> { text.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().parse().unwrap()).collect() }
fn combo(text: &str) -> ComboIndex { let c = cards(text); combo_index(c[0], c[1]) }
fn player(seat: u8, text: &str, board: &[Card]) -> PlayerRange {
    let mut range = parse_range(text).unwrap();
    block_public(&mut range, board);
    PlayerRange { seat: Seat(seat), range }
}
fn no_cancel() -> AtomicBool { AtomicBool::new(false) }
fn share(res: &EquityResult, seat: u8) -> f32 { res.shares.iter().find(|s| s.seat == Seat(seat) && s.pot_index == 0).unwrap().value }

#[test]
fn terminal_payoff_equity_times_pot() {
    let board = cards("QsJd7h3c2d");
    let oop = player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board).range;
    let ip = player(1, "QQ,JJ,77,AKo,54o,T9s", &board).range;
    let eq_oop = per_combo_equity(&oop, &ip, &board);
    let eq_ip = per_combo_equity(&ip, &oop, &board);
    let unraked = Rake::TimeCharge;
    let raked = Rake::PotRake { rate: 0.05, cap_mchips: 5000, no_flop_no_drop: false };
    for (range, eq) in [(&oop, &eq_oop), (&ip, &eq_ip)] {
        for i in 0..1326u16 {
            if range.get(i) == 0.0 { assert_eq!(eq[i as usize], 0.0); continue; }
            let e = eq[i as usize];
            assert!((0.0..=1.0).contains(&e));
            assert!((terminal_payoff(e, 100, &unraked) - e * 100.0).abs() < 1e-4);
            assert!((terminal_payoff(e, 100, &raked) - e * 95.0).abs() < 1e-4, "5% of 100 capped at 5 chips");
        }
    }
    let aces = eq_oop[combo("AcAd") as usize];
    assert!(aces > 0.0 && aces < 1.0, "AcAd loses to the sets and beats the rest: {aces}");
    let aa = per_combo_equity(&parse_range("AA").unwrap(), &parse_range("54o").unwrap(), &board);
    assert_eq!(aa[combo("AcAd") as usize], 1.0, "aces beat 54o on Q J 7 3 2");
    let set = per_combo_equity(&parse_range("QQ").unwrap(), &parse_range("AA").unwrap(), &board);
    assert_eq!(set[combo("QcQd") as usize], 1.0);
    // swapping seats leaves every value unchanged
    let a = EquityRequest::single_pot(board.clone(), vec![player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board), player(1, "QQ,JJ,77,AKo,54o,T9s", &board)], EquityMode::Exact);
    let b = EquityRequest::single_pot(board.clone(), vec![player(1, "QQ,JJ,77,AKo,54o,T9s", &board), player(0, "AA,KK,QQ,JJ,TT,AKs,AQs,54s", &board)], EquityMode::Exact);
    let ra = equity(&a, Duration::from_secs(10), &no_cancel());
    let rb = equity(&b, Duration::from_secs(10), &no_cancel());
    assert_eq!(ra.status, EquityStatus::Ready);
    assert_eq!(ra.method, Some(EquityMethod::Exact));
    assert!((share(&ra, 0) - share(&rb, 0)).abs() < 1e-6);
    assert!((share(&ra, 0) + share(&ra, 1) - 1.0).abs() < 1e-6);
}

#[test]
fn equity_budget_respected() {
    let board = cards("Kh7d2c");
    let req = EquityRequest::single_pot(board.clone(), vec![player(0, "random", &board), player(1, "random", &board)], EquityMode::Exact);
    assert!(exact_cost(&req) > 20_000_000);
    let budget = Duration::from_millis(100);
    let start = Instant::now();
    let res = equity(&req, budget, &no_cancel());
    let took = start.elapsed();
    assert_eq!(res.status, EquityStatus::BudgetExceeded);
    assert!(took < budget + Duration::from_millis(50), "stopped {took:?} after a {budget:?} budget");
    assert!(res.shares.is_empty());
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let setter = std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(30)); flag.store(true, Ordering::Relaxed); });
    let start = Instant::now();
    let res = equity(&req, Duration::from_secs(10), &cancel);
    let took = start.elapsed();
    setter.join().unwrap();
    assert_eq!(res.status, EquityStatus::Cancelled);
    assert!(took < Duration::from_millis(80), "cancelled {took:?} after a 30 ms flag");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-eval --test equity`
Expected: compile error.

- [ ] **Step 3: Implement `equity.rs`**

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use holdem_hand_evaluator::Hand;
use proto::{combo_cards, Card, ComboIndex, EquityMethod, Rake, Range1326, Seat, COMBOS};
use crate::evaluator::{BinaryEvaluator, Evaluator};

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRange { pub seat: Seat, pub range: Range1326 }

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EquityMode { Exact, MonteCarlo { seed: u64, max_samples: u32 } }

#[derive(Clone, Debug, PartialEq)]
pub struct PotEligibility { pub pot_index: u8, pub eligible: Vec<Seat> }

#[derive(Clone, Debug, PartialEq)]
pub struct EquityRequest { pub board: Vec<Card>, pub players: Vec<PlayerRange>, pub mode: EquityMode, pub pots: Vec<PotEligibility> }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquityStatus { Ready, Cancelled, BudgetExceeded, InvalidRanges }

#[derive(Clone, Debug, PartialEq)]
pub struct EquityShare { pub pot_index: u8, pub seat: Seat, pub value: f32, pub std_err: f32 }

#[derive(Clone, Debug, PartialEq)]
pub struct EquityResult { pub status: EquityStatus, pub method: Option<EquityMethod>, pub shares: Vec<EquityShare>, pub samples: u64, pub elapsed: Duration }

impl EquityRequest {
    pub fn single_pot(board: Vec<Card>, players: Vec<PlayerRange>, mode: EquityMode) -> EquityRequest { EquityRequest { board, players, mode, pots: vec![] } }
    pub(crate) fn pot_list(&self) -> Vec<PotEligibility> {
        if self.pots.is_empty() { vec![PotEligibility { pot_index: 0, eligible: self.players.iter().map(|p| p.seat).collect() }] } else { self.pots.clone() }
    }
    pub(crate) fn board_used(&self) -> [bool; 52] { let mut used = [false; 52]; for c in &self.board { used[c.0 as usize] = true; } used }
}

pub(crate) type Support = Vec<(ComboIndex, [Card; 2], f64)>;

/// Supported combos of a range (weight > 0, not blocked by the board).
pub(crate) fn support(range: &Range1326, board: &[Card]) -> Support {
    (0..COMBOS as u16).filter_map(|i| {
        let w = range.get(i);
        if w <= 0.0 { return None; }
        let cards = combo_cards(i);
        if board.contains(&cards[0]) || board.contains(&cards[1]) { return None; }
        Some((i, cards, w as f64))
    }).collect()
}

fn choose(n: u64, k: u64) -> u64 { if k > n { return 0; } (0..k).fold(1u64, |r, i| r * (n - i) / (i + 1)) }

/// Upper bound on the evaluations of an exact enumeration: product of support sizes times runouts.
pub fn exact_cost(req: &EquityRequest) -> u64 {
    let tuples: u64 = req.players.iter().map(|p| support(&p.range, &req.board).len() as u64).product();
    let remaining = (52 - req.board.len() - 2 * req.players.len()) as u64;
    tuples * choose(remaining, (5 - req.board.len()) as u64)
}

/// `equity * (pot - rake)`; rake = min(rate * pot, cap) for pot rake and 0 for a time charge (spec section 6).
pub fn terminal_payoff(equity: f32, pot: u32, rake: &Rake) -> f32 {
    let pot = pot as f32;
    let r = match rake { Rake::PotRake { rate, cap_mchips, .. } => (rate * pot).min(*cap_mchips as f32 / 1000.0), Rake::TimeCharge => 0.0 };
    equity * (pot - r)
}

pub(crate) struct Deadline<'a> { start: Instant, budget: Duration, cancel: &'a AtomicBool, pub evals: u64 }

impl<'a> Deadline<'a> {
    pub fn new(budget: Duration, cancel: &'a AtomicBool) -> Self { Deadline { start: Instant::now(), budget, cancel, evals: 0 } }
    /// Counts one evaluation; every 4096 evaluations checks the cancel flag and the clock.
    pub fn tick(&mut self) -> Option<EquityStatus> {
        self.evals += 1;
        if self.evals % 4096 == 0 {
            if self.cancel.load(Ordering::Relaxed) { return Some(EquityStatus::Cancelled); }
            if self.start.elapsed() > self.budget { return Some(EquityStatus::BudgetExceeded); }
        }
        None
    }
    pub fn elapsed(&self) -> Duration { self.start.elapsed() }
}

/// Per-pot win accumulation: `acc[pot][player]`, `total[pot]`.
pub(crate) struct Tally { acc: Vec<Vec<f64>>, total: Vec<f64>, eligible_idx: Vec<Vec<usize>> }

impl Tally {
    pub fn new(req: &EquityRequest, pots: &[PotEligibility]) -> Tally {
        let n = req.players.len();
        let eligible_idx = pots.iter().map(|p| p.eligible.iter().map(|s| req.players.iter().position(|q| q.seat == *s).expect("every eligible seat is a player")).collect()).collect();
        Tally { acc: vec![vec![0.0; n]; pots.len()], total: vec![0.0; pots.len()], eligible_idx }
    }
    /// Awards one showdown of weight `w`: per pot the best eligible rank wins; ties split equally.
    pub fn award(&mut self, ranks: &[u16], w: f64) {
        for (k, idx) in self.eligible_idx.iter().enumerate() {
            let best = idx.iter().map(|i| ranks[*i]).max().expect("a pot has an eligible player");
            let winners: Vec<usize> = idx.iter().copied().filter(|i| ranks[*i] == best).collect();
            let each = w / winners.len() as f64;
            for i in winners { self.acc[k][i] += each; }
            self.total[k] += w;
        }
    }
    pub fn empty(&self) -> bool { self.total.iter().all(|t| *t == 0.0) }
    pub fn shares(&self, req: &EquityRequest, pots: &[PotEligibility], samples: Option<f64>) -> Vec<EquityShare> {
        let mut out = Vec::new();
        for (k, pot) in pots.iter().enumerate() {
            for i in &self.eligible_idx[k] {
                let value = (self.acc[k][*i] / self.total[k]) as f32;
                let std_err = samples.map(|n| ((value as f64 * (1.0 - value as f64)) / n).sqrt() as f32).unwrap_or(0.0);
                out.push(EquityShare { pot_index: pot.pot_index, seat: req.players[*i].seat, value, std_err });
            }
        }
        out
    }
}

pub(crate) fn result(status: EquityStatus, method: Option<EquityMethod>, shares: Vec<EquityShare>, samples: u64, elapsed: Duration) -> EquityResult {
    EquityResult { status, method, shares, samples, elapsed }
}

struct ExactRun<'a> {
    req: &'a EquityRequest,
    supports: Vec<Support>,
    used: [bool; 52],
    chosen: Vec<[Card; 2]>,
    ranks: Vec<u16>,
    tally: Tally,
    deadline: Deadline<'a>,
}

impl<'a> ExactRun<'a> {
    fn assign(&mut self, player: usize, weight: f64) -> Option<EquityStatus> {
        if player == self.req.players.len() { return self.runouts(weight); }
        for idx in 0..self.supports[player].len() {
            let (_, cards, w) = self.supports[player][idx];
            let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
            if self.used[a] || self.used[b] { continue; }
            self.used[a] = true; self.used[b] = true;
            self.chosen.push(cards);
            let stop = self.assign(player + 1, weight * w);
            self.chosen.pop();
            self.used[a] = false; self.used[b] = false;
            if stop.is_some() { return stop; }
        }
        None
    }

    /// Enumerates every runout for the chosen holes and awards each one with `weight`.
    fn runouts(&mut self, weight: f64) -> Option<EquityStatus> {
        let deck: Vec<Card> = (0..52u8).map(Card).filter(|c| !self.used[c.0 as usize]).collect();
        let k = 5 - self.req.board.len();
        let mut idx: Vec<usize> = (0..k).collect();
        let mut board_cards = self.req.board.clone();
        loop {
            board_cards.truncate(self.req.board.len());
            board_cards.extend(idx.iter().map(|i| deck[*i]));
            let full: Hand = BinaryEvaluator.partial(&board_cards);
            for (i, hole) in self.chosen.iter().enumerate() {
                self.ranks[i] = BinaryEvaluator.rank_with(&full, hole);
                if let Some(s) = self.deadline.tick() { return Some(s); }
            }
            self.tally.award(&self.ranks, weight);
            if k == 0 { return None; }
            let mut j = k;
            loop {
                if j == 0 { return None; }
                j -= 1;
                if idx[j] < deck.len() - k + j {
                    idx[j] += 1;
                    for t in (j + 1)..k { idx[t] = idx[t - 1] + 1; }
                    break;
                }
            }
        }
    }
}

pub(crate) fn exact(req: &EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult {
    let pots = req.pot_list();
    let deadline = Deadline::new(budget, cancel);
    let supports: Vec<Support> = req.players.iter().map(|p| support(&p.range, &req.board)).collect();
    if supports.iter().any(|s| s.is_empty()) { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }
    let n = req.players.len();
    let mut run = ExactRun { req, supports, used: req.board_used(), chosen: Vec::with_capacity(n), ranks: vec![0; n], tally: Tally::new(req, &pots), deadline };
    let stop = run.assign(0, 1.0);
    let elapsed = run.deadline.elapsed();
    match stop {
        Some(status) => result(status, None, vec![], run.deadline.evals, elapsed),
        None if run.tally.empty() => result(EquityStatus::InvalidRanges, None, vec![], run.deadline.evals, elapsed),
        None => result(EquityStatus::Ready, Some(EquityMethod::Exact), run.tally.shares(req, &pots, None), run.deadline.evals, elapsed),
    }
}

/// Spec 3.5 entry point: exact enumeration or time-bounded Monte Carlo per `req.mode`.
pub fn equity(req: &EquityRequest, budget: Duration, cancel: &AtomicBool) -> EquityResult {
    match req.mode {
        EquityMode::Exact => exact(req, budget, cancel),
        EquityMode::MonteCarlo { seed, max_samples } => crate::mc::monte_carlo(req, seed, max_samples, budget, cancel),
    }
}

/// Exact equity of every supported hero combo against the villain's disjoint weighted combos over all runouts (river terminal use).
pub fn per_combo_equity(hero: &Range1326, villain: &Range1326, board: &[Card]) -> [f32; COMBOS] {
    let mut out = [0f32; COMBOS];
    let never = AtomicBool::new(false);
    for (i, _, _) in support(hero, board) {
        let one = Range1326::from_fn(|j| if j == i { 1.0 } else { 0.0 });
        let req = EquityRequest::single_pot(board.to_vec(), vec![PlayerRange { seat: Seat(0), range: one }, PlayerRange { seat: Seat(1), range: villain.clone() }], EquityMode::Exact);
        let res = exact(&req, Duration::from_secs(3600), &never);
        if res.status == EquityStatus::Ready { out[i as usize] = res.shares.iter().find(|s| s.seat == Seat(0)).map(|s| s.value).unwrap_or(0.0); }
    }
    out
}
```

Add to `lib.rs`: `pub mod equity; pub mod mc; pub use equity::*;` and create a placeholder-free `mc.rs` for this task with only the function the dispatcher needs, replaced in Task 21:

```rust
//! Monte Carlo equity (Task 21); this task ships exact enumeration only.
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use crate::equity::{result, EquityRequest, EquityResult, EquityStatus};

pub fn monte_carlo(_req: &EquityRequest, _seed: u64, _max_samples: u32, _budget: Duration, _cancel: &AtomicBool) -> EquityResult {
    result(EquityStatus::BudgetExceeded, None, vec![], 0, Duration::ZERO)
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (2 new; `terminal_payoff_equity_times_pot` enumerates about 60 hero combos × 60 villain combos on the river, instant).

- [ ] **Step 5: Commit**

```bash
git add crates/core-eval
git commit -m "feat(core-eval): exact joint-disjoint equity, per-combo equity and terminal payoffs" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 21: `core-eval` Monte Carlo with joint disjoint sampling

**Files:**
- Modify: `crates/core-eval/src/mc.rs`, `crates/core-eval/tests/equity.rs`

**Interfaces:**
- Produces: `mc::Xoshiro256 { seed(u64), next_u64(), next_f64(), below(n) }` (deterministic, dependency-free), `sample_joint_holes(req: &EquityRequest, seed: u64, count: usize) -> Vec<Vec<[Card; 2]>>` (test support: `count` accepted joint draws), `mc::monte_carlo(req, seed, max_samples, budget, cancel) -> EquityResult`. Semantics: each player's combo is drawn proportionally to its weight from its support, the draw is rejected when any two combos overlap (so accepted tuples follow the product-weight law of Task 20), the runout is a uniform partial shuffle of the remaining deck; `std_err = sqrt(v * (1 - v) / samples)` per share; `method = MonteCarlo { samples, std_err: max over shares }`. Status: `Ready` when `max_samples` is reached or the budget stops the loop with at least one sample; `Cancelled` when the flag stops it (shares present when samples > 0); `BudgetExceeded` only when no sample completed; `InvalidRanges` when no disjoint assignment exists (bounded DFS over the supports, or 10,000,000 rejections without an accepted sample).

- [ ] **Step 1: Write the failing tests** (append to `crates/core-eval/tests/equity.rs`)

```rust
#[test]
fn equity_mc_within_standard_error() {
    let heroes = ["AsKs", "7h7d", "QdJd", "9c8c", "AhQh"];
    let villains = ["QQ+,AKs", "22+,A2s+,KTs+", "JJ-77,AQo,KJs", "random"];
    let boards = ["Jh9h6c", "Kd7d2c", "8s8d3c", "AcQs5h", "Ts6s2d"];
    let mut checked = 0;
    for k in 0..20 {
        let board = cards(boards[k % 5]);
        let hero = heroes[k % 5];
        let villain = villains[k % 4];
        if board.iter().any(|c| cards(hero).contains(c)) { continue; }
        let players = || vec![player(0, hero, &board), player(1, villain, &board)];
        let exact = equity(&EquityRequest::single_pot(board.clone(), players(), EquityMode::Exact), Duration::from_secs(30), &no_cancel());
        assert_eq!(exact.status, EquityStatus::Ready, "spot {k}");
        let mc_mode = EquityMode::MonteCarlo { seed: 7 + k as u64, max_samples: 100_000 };
        let mc = equity(&EquityRequest::single_pot(board.clone(), players(), mc_mode), Duration::from_secs(30), &no_cancel());
        assert_eq!(mc.status, EquityStatus::Ready, "spot {k}");
        assert_eq!(mc.samples, 100_000);
        let (e, m) = (share(&exact, 0), mc.shares.iter().find(|s| s.seat == Seat(0)).unwrap());
        let bound = ((m.value as f64) * (1.0 - m.value as f64) / 100_000.0).sqrt() as f32;
        assert!((m.std_err - bound).abs() < 1e-9, "spot {k}: reported std_err {} vs binomial bound {bound}", m.std_err);
        assert!((m.value - e).abs() <= 4.0 * m.std_err, "spot {k} ({hero} vs {villain} on {}): mc {} exact {e} std_err {}", boards[k % 5], m.value, m.std_err);
        match mc.method { Some(EquityMethod::MonteCarlo { samples: 100_000, std_err }) => assert!(std_err >= m.std_err), other => panic!("{other:?}") }
        checked += 1;
    }
    assert!(checked >= 18);
}

#[test]
fn equity_joint_disjoint_sampling() {
    let board = cards("Kh7d2c");
    let req = EquityRequest::single_pot(board.clone(), vec![player(0, "AKs,AQs,KQs", &board), player(1, "AKs,AQs,KQs", &board), player(2, "AKs,AQs,KQs,AA", &board)], EquityMode::MonteCarlo { seed: 1, max_samples: 1000 });
    let draws = sample_joint_holes(&req, 1, 20_000);
    assert_eq!(draws.len(), 20_000);
    for holes in &draws {
        let mut all: Vec<Card> = holes.iter().flatten().copied().collect();
        all.extend(board.iter().copied());
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "samples never share a card");
    }
    let mut conflicting = EquityRequest::single_pot(board.clone(), vec![player(0, "AsKs", &board), player(1, "AsKs", &board)], EquityMode::Exact);
    assert_eq!(equity(&conflicting, Duration::from_secs(1), &no_cancel()).status, EquityStatus::InvalidRanges);
    conflicting.mode = EquityMode::MonteCarlo { seed: 1, max_samples: 1000 };
    assert_eq!(equity(&conflicting, Duration::from_secs(1), &no_cancel()).status, EquityStatus::InvalidRanges);
    let hero_blocks = EquityRequest::single_pot(board.clone(), vec![player(0, "AsKs", &board), player(1, "AsQd,AsJd,KsTc", &board)], EquityMode::MonteCarlo { seed: 1, max_samples: 1000 });
    assert_eq!(equity(&hero_blocks, Duration::from_secs(1), &no_cancel()).status, EquityStatus::InvalidRanges);
    // per-pot shares sum to 1 per pot: main pot with three players, side pot between seats 1 and 2
    let river = cards("Kh7d2c9s4h");
    let mut multi = EquityRequest::single_pot(river.clone(), vec![player(0, "QQ+,AK", &river), player(1, "22+,A2s+", &river), player(2, "random", &river)], EquityMode::Exact);
    multi.pots = vec![PotEligibility { pot_index: 0, eligible: vec![Seat(0), Seat(1), Seat(2)] }, PotEligibility { pot_index: 1, eligible: vec![Seat(1), Seat(2)] }];
    for mode in [EquityMode::Exact, EquityMode::MonteCarlo { seed: 3, max_samples: 50_000 }] {
        multi.mode = mode;
        let res = equity(&multi, Duration::from_secs(60), &no_cancel());
        assert_eq!(res.status, EquityStatus::Ready, "{mode:?}");
        for pot in 0..2u8 {
            let sum: f32 = res.shares.iter().filter(|s| s.pot_index == pot).map(|s| s.value).sum();
            assert!((sum - 1.0).abs() < 1e-5, "{mode:?} pot {pot} sums to {sum}");
            assert_eq!(res.shares.iter().filter(|s| s.pot_index == pot).count(), if pot == 0 { 3 } else { 2 });
        }
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p core-eval --test equity`
Expected: `sample_joint_holes` not found; the MC test fails against the placeholder.

- [ ] **Step 3: Implement `mc.rs`**

```rust
//! Monte Carlo equity with joint disjoint sampling (spec 3.5, 13.1).
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use holdem_hand_evaluator::Hand;
use proto::{Card, EquityMethod};
use crate::equity::{result, support, Deadline, EquityRequest, EquityResult, EquityStatus, Support, Tally};
use crate::evaluator::{BinaryEvaluator, Evaluator};

/// xoshiro256** seeded through splitmix64; deterministic and dependency-free.
pub struct Xoshiro256 { s: [u64; 4] }

impl Xoshiro256 {
    pub fn seed(seed: u64) -> Xoshiro256 {
        let mut z = seed;
        let mut s = [0u64; 4];
        for slot in s.iter_mut() {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            *slot = x ^ (x >> 31);
        }
        Xoshiro256 { s }
    }
    pub fn next_u64(&mut self) -> u64 {
        let out = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        out
    }
    pub fn next_f64(&mut self) -> f64 { (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64) }
    pub fn below(&mut self, n: usize) -> usize { ((self.next_u64() as u128 * n as u128) >> 64) as usize }
}

struct Sampler { supports: Vec<Support>, cumulative: Vec<Vec<f64>> }

impl Sampler {
    fn new(supports: Vec<Support>) -> Sampler {
        let cumulative = supports.iter().map(|s| { let mut acc = 0.0; s.iter().map(|(_, _, w)| { acc += w; acc }).collect() }).collect();
        Sampler { supports, cumulative }
    }
    fn draw_one(&self, player: usize, rng: &mut Xoshiro256) -> [Card; 2] {
        let cum = &self.cumulative[player];
        let u = rng.next_f64() * cum[cum.len() - 1];
        let idx = cum.partition_point(|c| *c <= u).min(cum.len() - 1);
        self.supports[player][idx].1
    }
    /// One joint draw; `None` when two players' combos overlap (rejection).
    fn draw(&self, rng: &mut Xoshiro256, board_used: &[bool; 52]) -> Option<(Vec<[Card; 2]>, [bool; 52])> {
        let mut used = *board_used;
        let mut holes = Vec::with_capacity(self.supports.len());
        for p in 0..self.supports.len() {
            let cards = self.draw_one(p, rng);
            let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
            if used[a] || used[b] { return None; }
            used[a] = true; used[b] = true;
            holes.push(cards);
        }
        Some((holes, used))
    }
}

/// Bounded DFS: does any disjoint assignment exist? A search that exceeds the step cap counts as compatible.
fn compatible(supports: &[Support], used: &mut [bool; 52], player: usize, steps: &mut u64) -> bool {
    if player == supports.len() { return true; }
    for (_, cards, _) in &supports[player] {
        *steps += 1;
        if *steps > 5_000_000 { return true; }
        let (a, b) = (cards[0].0 as usize, cards[1].0 as usize);
        if used[a] || used[b] { continue; }
        used[a] = true; used[b] = true;
        let ok = compatible(supports, used, player + 1, steps);
        used[a] = false; used[b] = false;
        if ok { return true; }
    }
    false
}

/// Test support: `count` accepted joint draws for the request's players.
pub fn sample_joint_holes(req: &EquityRequest, seed: u64, count: usize) -> Vec<Vec<[Card; 2]>> {
    let sampler = Sampler::new(req.players.iter().map(|p| support(&p.range, &req.board)).collect());
    let mut rng = Xoshiro256::seed(seed);
    let board_used = req.board_used();
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        if let Some((holes, _)) = sampler.draw(&mut rng, &board_used) { out.push(holes); }
    }
    out
}

pub fn monte_carlo(req: &EquityRequest, seed: u64, max_samples: u32, budget: Duration, cancel: &AtomicBool) -> EquityResult {
    let pots = req.pot_list();
    let mut deadline = Deadline::new(budget, cancel);
    let supports: Vec<Support> = req.players.iter().map(|p| support(&p.range, &req.board)).collect();
    let board_used = req.board_used();
    if supports.iter().any(|s| s.is_empty()) { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }
    let mut probe = board_used;
    if !compatible(&supports, &mut probe, 0, &mut 0) { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }
    let sampler = Sampler::new(supports);
    let mut rng = Xoshiro256::seed(seed);
    let mut tally = Tally::new(req, &pots);
    let n = req.players.len();
    let k = 5 - req.board.len();
    let deck_base: Vec<Card> = (0..52u8).map(Card).filter(|c| !board_used[c.0 as usize]).collect();
    let mut ranks = vec![0u16; n];
    let mut board_cards = req.board.clone();
    let mut samples = 0u64;
    let mut rejections = 0u64;
    let mut stop: Option<EquityStatus> = None;
    while samples < max_samples as u64 {
        if let Some(s) = deadline.tick() { stop = Some(s); break; }
        let Some((holes, used)) = sampler.draw(&mut rng, &board_used) else {
            rejections += 1;
            if samples == 0 && rejections > 10_000_000 { return result(EquityStatus::InvalidRanges, None, vec![], 0, deadline.elapsed()); }
            continue;
        };
        let mut deck: Vec<Card> = deck_base.iter().copied().filter(|c| !used[c.0 as usize]).collect();
        for j in 0..k { let r = j + rng.below(deck.len() - j); deck.swap(j, r); }
        board_cards.truncate(req.board.len());
        board_cards.extend_from_slice(&deck[..k]);
        let full: Hand = BinaryEvaluator.partial(&board_cards);
        for (i, hole) in holes.iter().enumerate() {
            ranks[i] = BinaryEvaluator.rank_with(&full, hole);
            if let Some(s) = deadline.tick() { stop = Some(s); }
        }
        tally.award(&ranks, 1.0);
        samples += 1;
        if stop.is_some() { break; }
    }
    let elapsed = deadline.elapsed();
    if samples == 0 { return result(stop.unwrap_or(EquityStatus::BudgetExceeded), None, vec![], 0, elapsed); }
    let shares = tally.shares(req, &pots, Some(samples as f64));
    let std_err = shares.iter().map(|s| s.std_err).fold(0.0f32, f32::max);
    let status = match stop { Some(EquityStatus::Cancelled) => EquityStatus::Cancelled, _ => EquityStatus::Ready };
    result(status, Some(EquityMethod::MonteCarlo { samples: samples as u32, std_err }), shares, samples, elapsed)
}
```

Add `pub use mc::{sample_joint_holes, Xoshiro256};` to `lib.rs`.

- [ ] **Step 4: Run tests**

Run: `cargo test --workspace`
Expected: all pass (2 new; the MC test draws 20 × 100,000 samples, a few seconds in the `dev` profile).

- [ ] **Step 5: Commit**

```bash
git add crates/core-eval
git commit -m "feat(core-eval): Monte Carlo equity with joint disjoint sampling and standard errors" -m "Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---
## Running everything

- `cargo test --workspace` (green after every task; about 15 s in the `dev` profile once the fixtures exist).
- `cargo test -p core-eval --features exhaustive` after `tools/.venv/Scripts/python tools/gen_eval_oracle.py --skip-5card --samples 10000000 --samples-name phevaluator_7card_10m.bin` (the file is gitignored; `bench oracle` of plan 2 wraps this).
- `tools/.venv/Scripts/python -m pytest tools -q` (Python oracles and generators).
- Regenerating fixtures is deterministic: `tools/.venv/Scripts/python tools/gen_fixtures.py --out fixtures/hands` and `tools/.venv/Scripts/python tools/gen_eval_oracle.py --out-dir fixtures/eval` reproduce the committed bytes.

## Interfaces handed to plans 2-5

| Item | Defined in | Signature |
|---|---|---|
| `proto::{Card, ComboIndex, combo_index, combo_cards, class_of, class_combos}` | Task 2 | `combo_index(a: Card, b: Card) -> ComboIndex`, `combo_cards(i) -> [Card; 2]` |
| `proto::Range1326` | Task 4 | `zero() / uniform() / from_fn / get / set`; serde array of 1326 |
| `proto::{GameConfig, HandConfig, Rake, SolverPrefs, SeatConfig, SeatTag, QuickFact}` | Task 3 | spec §4.2 |
| `proto::{Seat, Position, Street, Action, TakenAction, HandPhase, CompleteReason, HandState, Derived, Pot, LegalAction, StreetRootSnapshot, SolveInput}` | Tasks 3, 6 | spec §4.3 (+ `StreetRootSnapshot.bb_chips`) |
| `proto::{DecisionIdentity, Coverage, ApproxReason, UnsupportedReason, Unavailable, ActionAdvice, Availability, EquityMethod, EquityEstimate, EquitySummary, PotShares, Assumptions, ExperimentalHu, ExploitAdvice, Phase, Recommendation, RecommendationEvent}` | Task 5 | spec §4.4, §11 |
| `proto::{EffectiveTree, PlayerMenus, Menu, RaiseSize, MaterializedNode, ChipPath, OrdinalPath, RULES_VERSION, resolve_chip_path}` | Task 6 | `resolve_chip_path(&[MaterializedNode], &[Action]) -> Option<OrdinalPath>` |
| `proto::worker::{EngineMessage, WorkerMessage, SolveRequest, ReadyInfo, NodeLock, AckStatus, Stage, ResultStatus, WorkerError, StreetSolution, NodeStrategy, REQUEST_LINE_MAX, RESULT_LINE_MAX, MAX_EXPORTED_NODES, FAILURE_CODES}` | Task 7 | spec §4.5 |
| `proto::worker::{validate_solution, validate_locks}` | Task 8 | `(&StreetSolution, &[MaterializedNode]) -> Result<Vec<OrdinalPath>, String>` |
| `core_model::{RulesError, parse_card, parse_hand, parse_cards, cards_to_string}` | Task 9 | |
| `core_model::{ring, validate_table, positions, position_of, preflop_order, postflop_order, straddle_posts, initial_full_raise, posts}` | Task 9 | `preflop_order(button: Seat, dealt: &[Seat], straddle: bool) -> Vec<Seat>` |
| `core_model::betting::Round` | Task 10 | one betting street |
| `core_model::{BeginHand, begin_hand, apply_action, set_board, set_hero_cards, derive, settle_pots, is_decision_point, abandon, Settlement}` | Task 11 | spec §3.5 |
| `core_model::{RootError, street_root, replay_root}` | Task 12 | spec §3.5, §10.2 |
| `core_ranges::{RangeError, parse_range, range_to_string, expand_169, class_name, class_index, block_public, hero_conditioned, mass, hash_scaled}` | Tasks 15-16 | spec §3.5 |
| `core_iso::{SuitPerm, ALL_PERMS, CanonicalBoard, apply, inverse, apply_range, canonicalize, orbit_size, orbit_size_of}` | Task 17 | spec §3.5 |
| `core_eval::{Evaluator, BinaryEvaluator, rank, rank5, rank7}` | Task 19 | |
| `core_eval::{PlayerRange, EquityMode, PotEligibility, EquityRequest, EquityStatus, EquityShare, EquityResult, equity, exact_cost, per_combo_equity, terminal_payoff, sample_joint_holes, Xoshiro256}` | Tasks 20-21 | `equity(&EquityRequest, Duration, &AtomicBool) -> EquityResult` |

The engine (plan 2) chooses `EquityMode::Exact` when `exact_cost(&req) <= 20_000_000` and Monte Carlo otherwise (spec §7), and converts `EquityResult` into `EquityEstimate`/`PotShares`.

## Self-review

### 1. Spec coverage

| Spec section / requirement | Tasks |
|---|---|
| §2 decision point, NoDecision conditions | 11 (`is_decision_point`), 12 (`RootError::NoDecision`) |
| §2 street root, pot-eligible counting, HU spot, OOP/IP | 12 |
| §2 positions, preflop/postflop order, UTG straddle order | 9 |
| §2 money units (`u32` chips, `cap_mchips`), raise-to amounts | 3, 10 |
| §2 EV convention at terminals (`equity * pot`, rake at terminals) | 20 (`terminal_payoff`) |
| §2 coverage labels and reason names | 5 |
| §2 public range vs hero-conditioned copies | 16 |
| §2 canonical board with the stabilizer tie-break | 17 |
| §2 range hash `hash_scaled` | 16 |
| §2 materialized tree, ordinal and chip paths | 6 |
| §2 decision identity | 5 |
| §3.2 crate set, dependency direction, licenses | 1, 9, 15, 17, 19 |
| §3.5 `core-model` interface (`parse_card`, `parse_hand`, `begin_hand`, `apply_action`, `set_board`, `derive`, `street_root`, `replay_root`, `settle_pots`) | 9, 11, 12 |
| §3.5 `core-ranges` interface | 15, 16 |
| §3.5 `core-iso` interface | 17 |
| §3.5 `core-eval` interface (`rank7`, `equity` with budget and cancel) | 19, 20, 21 |
| §3.6 toolchain pin (MSVC), §3.7 `+avx2` rustflags | 1 |
| §4.1 cards, combo index, 169-class order | 2 |
| §4.2 `GameConfig`, `HandConfig`, `SolverPrefs` defaults | 3 |
| §4.3 types (`Action`, `TakenAction`, `HandPhase`, `HandState`, `Derived`, `StreetRootSnapshot`, `SolveInput`) | 3, 6 |
| §4.3 dealt-seat rules, two dealt seats `FormatUnsupported`, straddle requirements | 9 |
| §4.3 lifecycle, street closure, `FoldedOut`/`AllInRunout`/`ShowdownReached`, `set_board` rules | 11 |
| §4.3 settlement, uncalled returns, side pots, conservation invariant | 11 |
| §4.3 cumulative reopening | 10 |
| §4.4 recommendation, events, headline data types | 5 |
| §4.5 wire schema, tags, limits, examples, optional fields | 7 |
| §4.5 matrix validation incl. `[0, 1]` bound, lock rows, chip-path resolution | 8 |
| §4.6 `EffectiveTree`, `MaterializedNode`, donk menus as an explicit empty list | 6 |
| §10.2 projection rule and its three worked cases | 12 |
| §13.0 `fixtures/hands`, `fixtures/eval` | 13, 18 |
| §13.1 `state_machine_pokerkit_fixtures` | 13, 14 |
| §13.1 `straddle_action_order_utg`, `dealt_seats_3_to_6`, `card_parser_roundtrip` | 9 |
| §13.1 `min_raise_and_short_allin_no_reopen`, `cumulative_short_allins_reopen` | 10 |
| §13.1 `side_pot_three_allins`, `side_pot_two_contested`, `allin_runout_single_survivor` | 11 |
| §13.1 `street_root_reconstruction`, `multiway_root_projection` | 12 |
| §13.1 `range_roundtrip_pio_strings`, `class_expansion_multiplicity` | 15 |
| §13.1 `public_blocking_board_only`, `hero_conditioned_copy`, `range_hash_scale_invariant` | 16 |
| §13.1 `iso_class_count_1755`, `iso_orbit_sizes`, `iso_stabilizer_tiebreak` | 17 |
| §13.1 `eval_vs_phevaluator_full_5card`, `eval_vs_phevaluator_random_7card` (+ exhaustive) | 18, 19 |
| §13.1 `terminal_payoff_equity_times_pot`, `equity_budget_respected` | 20 |
| §13.1 `equity_mc_within_standard_error`, `equity_joint_disjoint_sampling` | 21 |
| proto validator tests (negative entries, entries above 1, row sums, unavailable rows, `requested`, `covered_paths`, resolution) | 8 |

Gaps and deliberate deviations (all small, all named so later plans can rely on them):
1. `StreetRootSnapshot` carries an extra `bb_chips: u32` (the minimum bet that `replay_root` needs to reproduce the legal set of an unopened street). The worker ignores it.
2. `RootError` has the spec's `Multiway` (with a `pot_eligible` payload), `ProjectionNotReproducing { step }` and `NoDecision`, plus `Preflop` (called on a preflop decision) and `Inconsistent { step }` (a genuine HU root that does not replay; the engine maps it to `EngineError`, spec §10.2).
3. `EquityMode::MonteCarlo` carries `max_samples` next to `seed`; the §7 exact-versus-MC decision (`pairs * runouts <= 2 * 10^7`) is the engine's, using `exact_cost`.
4. Weighted equity is implemented in-house on the evaluator trait instead of through `pokers` (its weights are `u8` percents and cannot represent `Range1326`); spec §3.2 names `pokers` as the mechanism, the interface of §3.5 is unchanged.
5. `fixtures/worker/*.jsonl` and `tools/gen_worker_fixtures.py` belong to plan 2; the two-combo river ranges they use are defined by `river_two_combo_ranges()` in `crates/proto/tests/wire_examples.rs`.
6. The 10,000,000-sample 7-card check runs only with `--features exhaustive` against a locally generated, gitignored file.
7. The PokerKit generator drops hands where PokerKit's reopening rule diverges from spec §4.3 (verified: 1 seed in 201) and applies two documented normalizations (straddle minimum open, fold-out bet split).
8. `ts-rs` bindings are plan 5's (per the series brief).
9. `Derived` per-seat vectors are indexed by `Seat.0` with undealt seats marked folded; `HandState.stacks_start` is aligned with `HandState.dealt`.

### 2. Placeholder scan

Searched the plan for `TBD`, `TODO`, `implement later`, `fill in`, `add validation`, `handle edge cases`, `similar to Task`, `write tests for`: none. Every code step contains the full code. The only transient artifact is the Task 20 `mc.rs` dispatcher (a complete, compiling function that Task 21 replaces; `cargo test --workspace` is green in between because no test of Task 20 exercises Monte Carlo).

### 3. Type consistency

- `combo_index` / `combo_cards` / `class_combos` / `class_of` (Task 2) are used unchanged in Tasks 15, 16, 17, 20 and 21.
- `Range1326::{zero, uniform, from_fn, get, set}` (Task 4) are the only accessors used by later tasks; `r.0[i]` direct indexing appears only inside `core-ranges` and `core-iso`.
- `Round::{open, post, apply, legal, to_act, closed, live, eligible_count, all_in_to, min_raise_to, may_aggress}` (Task 10) are the calls made by `lifecycle.rs` (Task 11) and `street_root.rs` (Task 12); `Sim::{round, phase, pots, returned}` (Task 11) are read by Task 12.
- `resolve_chip_path` (Task 6) is what `validate_solution`/`validate_locks` (Task 8) call; both validators return `Result<Vec<OrdinalPath>, String>`, the signature plan 4 assumes.
- `support`, `Deadline`, `Tally`, `result`, `Support` are `pub(crate)` in `equity.rs` (Task 20) and imported by `mc.rs` (Task 21) under the same names; `BinaryEvaluator::{partial, rank_with}` (Task 19) are used by both.
- `CanonicalBoard::cards()`, `orbit_size(&CanonicalBoard) -> u8`, `apply(&SuitPerm, Card)`, `apply_range(&SuitPerm, &Range1326)`, `inverse(&SuitPerm)` (Task 17), `parse_range(&str) -> Result<Range1326, RangeError>`, `block_public(&mut Range1326, &[Card])`, `hash_scaled(&Range1326) -> [u8; 32]` (Tasks 15-16) match the signatures plan 4 lists as assumptions.
- Test helper names are local to each test file (`cfg`, `seats`, `begin`, `play`, `pot`, `cards`, `player`, `share`) and are redefined wherever used.
