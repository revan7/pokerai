"""Generate fixtures/preflop/synthetic_v2/*.json: a synthetic PokerData-shaped preflop
bundle (spec section 8.2) for P3.T2's `core_preflop::store` loader, plus the provider-shaped
sparse siblings (`node.json`, `range.json`) exercised by later ingestion tasks and the
lookup-path inventory (`spots.json`, `cases.json`).

Standard library only (json/hashlib/pathlib) -- no provider HTTP calls, no `.7z` decoding.
Deterministic: no randomness anywhere, so re-running this script reproduces the committed
bytes exactly (`tools/tests/test_preflop_fixtures.py` asserts this).

`nodes.json` is the dense, action-major, 169-wide normalized envelope `core_preflop::decode`
consumes; `manifest.json` is its `BundleInfo` sidecar, with `sha256` computed over `nodes.json`'s
exact committed bytes. `node.json`/`range.json` are the **provider-shaped** sparse wire layout
(R7 section 4) that a later ingestion task's key/spot parsers exercise -- not consumed by
`core_preflop::store` directly. Their one shared invariant with `nodes.json`, checked by the
Python test, is the sparse zero-weight EV: class 14 (KK) carries EV 2.31 with weight 0 in both
the dense and the sparse shapes.

P3.T19 adds `generate_goldens`: the two spec section 13.3 engine goldens
(`crates/engine/tests/golden/{replay_weights_golden,bet_translation_golden}.json`), written from
the independent oracles at the end of this module (standard library only: json, hashlib, math,
struct, fractions). `generate` itself, and the six synthetic_v2 files, are unchanged by it.
"""
from __future__ import annotations

import hashlib
import json
import math
import struct
from fractions import Fraction
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT_DIR = ROOT / "fixtures" / "preflop" / "synthetic_v2"

BUNDLE_ID = "synthetic_v2_100bb"
RAKE_PROFILE = "5% cap 0.5bb"
CLASS_ORDER = "A-2 row-major, section 4.1"


def write_json(path: Path, value: dict) -> None:
    # `Path.write_text`'s default `newline=None` applies universal-newline translation,
    # which turns every `\n` into `\r\n` on Windows -- silently changing the committed
    # bytes out from under `manifest_for`'s in-memory `sha256` (computed via `.encode()`
    # on an LF-only string, never touched by that translation). `write_bytes` writes the
    # exact bytes below, LF-only, on every platform, so the committed file's hash always
    # matches what `manifest_for` recorded.
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes((json.dumps(value, indent=2, allow_nan=False) + "\n").encode("utf-8"))


def make_node(history: list, actor: str, actions: list) -> dict:
    """One envelope node: dense action-major `weights`/`evs`, 169 classes wide. Every class
    defaults to full weight on action 0 (index-0 action, e.g. fold); class 1 (AKs) is split
    0.65/0.35 between actions 0 and 1 whenever a second action exists, with an EV of 1.84 for
    action 1 and an explicit EV of 2.31 for class 14 (KK) on action 1 *despite* KK carrying
    zero weight there -- the sparse-zero-weight-EV-retention case both this dense form and the
    provider-shaped `node.json`/`range.json` siblings must preserve. Class 168 is forced to
    zero weight on every action and declared unreachable.

    `committed_by_actor_sb` is derived by the Rust loader (`core_preflop::store::committed_before`)
    from the source posts and this `history`; this generator must not carry a second, divergent
    copy of it.
    """
    weights = [[0.0] * 169 for _ in actions]
    evs = [[None] * 169 for _ in actions]
    for c in range(169):
        weights[0][c] = 1.0
        evs[0][c] = 0.0
    if len(actions) > 1:
        weights[0][1], weights[1][1] = 0.65, 0.35  # class 1 = AKs
        evs[1][1] = 1.84
        evs[1][14] = 2.31  # class 14 = KK: explicit EV retained even with zero action weight.
    weights[0][168] = 0.0
    return {
        "history": history,
        "actor": actor,
        "actions": actions,
        "weights": weights,
        "evs": evs,
        "unreachable_classes": [168],
    }


def manifest_for(envelope: dict) -> dict:
    raw = (json.dumps(envelope, indent=2, allow_nan=False) + "\n").encode()
    return {
        "bundle_id": envelope["bundle_id"],
        "source": "PokerDataJson",
        "game": "nl",
        "version": 2,
        "depth_bb": 100,
        "depths": [100],
        "source_blinds": [0.5, 1.0],
        "rake_profile": RAKE_PROFILE,
        "rake": {"rate": 0.05, "cap_bb": 0.5, "no_flop_no_drop": True},
        "straddle": False,
        "ev_unit": "source_sb",
        "ev_reference": "decision_incremental_verified",
        "accuracy": "unverified",
        "license_note": "Synthetic test data; not vendor data; no V9 claim",
        "sha256": hashlib.sha256(raw).hexdigest(),
    }


def raise_action(to_bb_x1000: int) -> dict:
    return {"step": "raise", "to_bb_x1000": to_bb_x1000}


# --- the eight fixture-table rows (spec section 13.0's synthetic inventory) ---

UTG_RFI_HISTORY: list = []
HJ_VS_UTG_RFI_HISTORY = [["UTG", "raise", 2500]]
UTG_VS_HJ_3BET_HISTORY = [
    ["UTG", "raise", 2500],
    ["HJ", "raise", 8750],
    ["CO", "fold", 0],
    ["BTN", "fold", 0],
    ["SB", "fold", 0],
    ["BB", "fold", 0],
]
HJ_VS_UTG_4BET_HISTORY = UTG_VS_HJ_3BET_HISTORY + [["UTG", "raise", 22000]]
BB_SQUEEZE_HISTORY = [
    ["UTG", "raise", 2500],
    ["HJ", "call", 0],
    ["CO", "fold", 0],
    ["BTN", "fold", 0],
    ["SB", "fold", 0],
]
SB_LIMP_HISTORY = [
    ["UTG", "fold", 0],
    ["HJ", "fold", 0],
    ["CO", "fold", 0],
    ["BTN", "fold", 0],
]
BB_VS_SB_LIMP_HISTORY = SB_LIMP_HISTORY + [["SB", "call", 0]]

NODES = [
    make_node(UTG_RFI_HISTORY, "UTG", [{"step": "fold"}, raise_action(2500)]),
    make_node(HJ_VS_UTG_RFI_HISTORY, "HJ", [{"step": "fold"}, {"step": "call"}, raise_action(8750)]),
    make_node(UTG_VS_HJ_3BET_HISTORY, "UTG", [{"step": "fold"}, {"step": "call"}, raise_action(22000)]),
    make_node(HJ_VS_UTG_4BET_HISTORY, "HJ", [{"step": "fold"}, {"step": "call"}, {"step": "allin"}]),
    make_node(BB_SQUEEZE_HISTORY, "BB", [{"step": "fold"}, {"step": "call"}, raise_action(12000)]),
    make_node(SB_LIMP_HISTORY, "SB", [{"step": "fold"}, {"step": "call"}, raise_action(3000)]),
    make_node(BB_VS_SB_LIMP_HISTORY, "BB", [{"step": "check"}, raise_action(3500)]),
]

ABSENCES = [
    {"path": "CO_cold_call_vs_3bet", "note": "CO cold-call vs 3bet: no node recorded"},
    {"path": "SB_cold_call_vs_3bet", "note": "SB cold-call vs 3bet: no node recorded"},
    {"path": "UTG_open_limp", "note": "UTG open-limp: no node recorded"},
    {"path": "HJ_open_limp", "note": "HJ open-limp: no node recorded"},
    {"path": "CO_open_limp", "note": "CO open-limp: no node recorded"},
    {"path": "BTN_open_limp", "note": "BTN open-limp: no node recorded"},
]


def build_envelope() -> dict:
    return {
        "bundle_id": BUNDLE_ID,
        "depth_bb": 100,
        "rake_profile": RAKE_PROFILE,
        "straddle": False,
        "class_order": CLASS_ORDER,
        "nodes": NODES,
    }


def build_node_json() -> dict:
    """Provider-shaped sparse single-node record (R7 section 4), mirroring the dense
    "HJ vs UTG RFI" node above: only the `call` action's interesting classes are recorded,
    and class 14 (KK) is retained with EV 2.31 at weight 0 -- the shared invariant with
    `range.json` and with the dense `nodes.json` node it mirrors.
    """
    return {
        "synthetic": True,
        "game": "nl",
        "version": 2,
        "stack": 100,
        "actor": "HJ",
        "history": "UTG_60%",
        "actions": [
            {"step": "fold"},
            {"step": "call", "weights": {"AKs": 0.35, "KK": 0.0}, "evs": {"AKs": 1.84, "KK": 2.31}},
            raise_action(8750),
        ],
    }


def build_range_json() -> dict:
    return {
        "synthetic": True,
        "spot": "UTG_60%_HJ_Call",
        "actor": "HJ",
        "hand": "AKs",
        "freq": 0.35,
        "ev": 1.84,
        "combos": 61.9,
        "weights": {"AKs": 0.35, "KK": 0, "JJ": 1},
        "evs": {"AKs": 1.84, "KK": 2.31},
    }


def build_spots_json() -> dict:
    """Every table path (spec section 13.0's synthetic inventory) with its resolved action
    sizes, in bb. `Provider spot ending in an action identifies an action range: remove that
    last pair when deriving its decision-node key` -- the two open-limp/cold-call absences
    below end in an action for exactly that reason; the rest name the actor about to decide.
    """
    return {
        "synthetic": True,
        "spots": [
            {"path": "UTG_RFI", "actor": "UTG", "history": UTG_RFI_HISTORY, "resolved_sizes_bb": {"raise": 2.5}, "node": True},
            {"path": "HJ_vs_UTG_RFI", "actor": "HJ", "history": HJ_VS_UTG_RFI_HISTORY, "resolved_sizes_bb": {"raise": 8.75}, "node": True},
            {"path": "UTG_vs_HJ_3bet", "actor": "UTG", "history": UTG_VS_HJ_3BET_HISTORY, "resolved_sizes_bb": {"raise": 22.0}, "node": True},
            {"path": "HJ_vs_UTG_4bet", "actor": "HJ", "history": HJ_VS_UTG_4BET_HISTORY, "resolved_sizes_bb": {}, "node": True},
            {"path": "BB_squeeze", "actor": "BB", "history": BB_SQUEEZE_HISTORY, "resolved_sizes_bb": {"raise": 12.0}, "node": True},
            {"path": "SB_limp", "actor": "SB", "history": SB_LIMP_HISTORY, "resolved_sizes_bb": {"raise": 3.0}, "node": True},
            {"path": "BB_vs_SB_limp", "actor": "BB", "history": BB_VS_SB_LIMP_HISTORY, "resolved_sizes_bb": {"raise": 3.5}, "node": True},
            {
                "path": "explicit_absences",
                "actor": None,
                "history": [a["path"] for a in ABSENCES],
                "resolved_sizes_bb": {},
                "node": False,
            },
        ],
    }


def build_cases_json() -> dict:
    """Records all four `ev_reference` variants, the source posts (SB = 1, BB = 2 source-SB
    units, spec section 2's `sb = 0.5 bb` / `bb = 1 bb` pair), the full source stack at this
    bundle's depth (`source_stack_sb = 2 * depth_bb`), and the explicit-absences table row.
    """
    return {
        "synthetic": True,
        "ev_reference_variants": [
            "decision_incremental_verified",
            "net_hand_start_verified",
            "absolute_stack_verified",
            "unverified",
        ],
        "sb_committed_sb": 1,
        "bb_committed_sb": 2,
        "source_stack_sb": 200,
        "absences": ABSENCES,
    }


def generate(out_dir: Path) -> None:
    envelope = build_envelope()
    write_json(out_dir / "nodes.json", envelope)
    write_json(out_dir / "manifest.json", manifest_for(envelope))
    write_json(out_dir / "node.json", build_node_json())
    write_json(out_dir / "range.json", build_range_json())
    write_json(out_dir / "spots.json", build_spots_json())
    write_json(out_dir / "cases.json", build_cases_json())


# =============================================================================================
# P3.T19: the replay and bet-translation engine goldens (spec section 13.3).
#
# `generate_goldens` writes `crates/engine/tests/golden/replay_weights_golden.json` and
# `bet_translation_golden.json`, which `crates/engine/tests/preflop_goldens.rs` compares the Rust
# implementation against. Every expected value below is computed here, from the spec's formulas,
# with this module's own card/combo/class indexing and scalar arithmetic -- never from the Rust
# implementation under test. Every source is synthetic, embedded byte for byte in the golden's
# input (manifest + exact envelope text), and never written to the chart directories.
#
# One modelling rule applies throughout: a likelihood enters the oracles at the precision its
# source actually carries. Envelope probabilities are `f32` once admitted (spec section 8.2's
# source envelope; plan 3's type-consistency check: "envelope probabilities f32"), so each class
# likelihood is rounded once with `f32()` before any arithmetic; everything after that is double.
# Without it the oracle would measure the source's storage precision (0.8 is 0.800000011920929 as
# an f32, a 1.5e-8 relative difference) rather than replay arithmetic, far above the 1e-10 the
# goldens hold `q` and `log_reach` to.
# =============================================================================================

ENGINE_GOLDEN_DIR = ROOT / "crates" / "engine" / "tests" / "golden"
GOLDEN_FILES = ["replay_weights_golden.json", "bet_translation_golden.json"]
GOLDEN_SCHEMA_VERSION = 1

RANKS = "23456789TJQKA"
SUITS = "cdhs"


def f32(x: float) -> float:
    """`x` rounded once to the nearest IEEE-754 single, returned as the double it equals."""
    return struct.unpack("<f", struct.pack("<f", x))[0]


def f32_display(x: float) -> str:
    """The shortest decimal that reads back as the same single: how a single prints in a note."""
    for digits in range(1, 10):
        text = f"{x:.{digits}g}"
        if f32(float(text)) == f32(x):
            return text
    raise ValueError(f"{x} has no short single representation")


def card_id(text: str) -> int:
    """Spec section 4.1: rank index * 4 + suit index, ranks 2..A = 0..12, suits c, d, h, s = 0..3."""
    return RANKS.index(text[0]) * 4 + SUITS.index(text[1])


def combo_index(a: int, b: int) -> int:
    """Spec section 4.1: `hi * (hi - 1) / 2 + lo` for card ids `lo < hi`."""
    lo, hi = min(a, b), max(a, b)
    return hi * (hi - 1) // 2 + lo


def combo_class(lo, hi):
    r1, r2 = 12-lo//4, 12-hi//4
    high, low = min(r1,r2), max(r1,r2)
    if high == low: return high*13+low
    return high*13+low if lo%4 == hi%4 else low*13+high


def vector_for(fn):
    return [fn(combo_class(lo,hi)) for hi in range(1,52) for lo in range(hi)]


COMBO_PAIRS = [(lo, hi) for hi in range(1, 52) for lo in range(hi)]
BOARD = {46, 21, 0}  # Kh7d2c: rank indices 11,5,0; suits h,d,c.


# --- the synthetic class likelihoods (plan 3 Task 19 Step 3) ---

def btn_raise(c: int) -> float:
    return .8 if c % 2 == 0 else .2


def sb_fold(c: int) -> float:
    return .25 if c % 3 == 0 else 1.0


def bb_call(c: int) -> float:
    return .9 if c % 5 == 0 else .1


def btn_raise_large(c: int) -> float:
    """The off-menu line's second menu size: P(B | class)."""
    return .1 if c % 2 == 0 else .5


UNIFORM_LINE_LIKELIHOODS = (btn_raise, sb_fold, bb_call)


def source_vector(fn):
    """A class likelihood expanded to 1326 combos at the source's own (f32) precision."""
    return vector_for(lambda c: f32(fn(c)))


# --- the two replay oracles (plan 3 Task 19 Step 3, with the f32 source precision above) ---

def replay_golden_expected():
    q=1.0
    masses=[[1.0]*1326 for _ in range(3)]
    logs=[0.0]*6
    likelihoods=[source_vector(p) for p in UNIFORM_LINE_LIKELIHOODS]
    for actor,p in enumerate(likelihoods):
        m=math.fsum(w*x for w,x in zip(masses[actor],p))/math.fsum(masses[actor])
        q*=m
        masses[actor]=[w*x/m for w,x in zip(masses[actor],p)]
        for seat in range(3):
            removed=max(q*w for w in masses[seat])
            masses[seat]=[w/removed for w in masses[seat]]
            logs[seat]+=math.log(removed)
    board=BOARD # Kh7d2c: rank indices 11,5,0; suits h,d,c.
    pairs=[(lo,hi) for hi in range(1,52) for lo in range(hi)]
    ranges=[]
    for seat in range(3):
        r=[q*w if lo not in board and hi not in board else 0.0
           for w,(lo,hi) in zip(masses[seat],pairs)]
        removed=max(r); logs[seat]+=math.log(removed)
        ranges.append([w/removed for w in r])
    return {"ranges":ranges+[None]*3,"log_reach":logs,"q":q}


def replay_golden_offmenu_expected():
    """Second three-seat line: BTN's observed raise sits between the source menu's two sizes
    (pot fractions 0.5 and 1.0 at the source parent), so section 8.4 splits the initial branch
    in two. Written against the formulas, not against the Rust kernel. Beyond the brief's
    sketch it also returns the SB's and BB's un-normalized marginals, from which the golden's
    full ranges and log_reach follow."""
    A, B, s = 0.5, 1.0, 0.73                      # pot fractions at the parent
    fA = (B - s) * (1 + A) / ((B - A) * (1 + s))  # 81/173
    fB = 1 - fA
    pA = source_vector(btn_raise)          # P(A | class)
    pB = source_vector(btn_raise_large)    # P(B | class)
    branches = []
    for f, p, label in ((fA, pA, "A"), (fB, pB, "B")):
        w = [1.0] * 1326
        m = math.fsum(x * y for x, y in zip(w, p)) / math.fsum(w)
        branches.append({"q": f * m, "mass": [x * y / m for x, y in zip(w, p)], "translated": label})
    # SB folds in both branches with the same class-varying likelihood.
    fold = source_vector(sb_fold)
    sb = [[1.0] * 1326, [1.0] * 1326]
    for k, b in enumerate(branches):
        m = math.fsum(x * y for x, y in zip(sb[k], fold)) / math.fsum(sb[k])
        b["q"] *= m
        sb[k] = [x * y / m for x, y in zip(sb[k], fold)]
    total = math.fsum(b["q"] for b in branches)
    btn_marginal = [math.fsum(b["q"] * b["mass"][c] for b in branches) for c in range(1326)]
    # Cross-seat evidence: BB has not acted, so its posterior is q_k / sum_j q_j
    # for every combo -- the section 8.4 property the uniform line cannot exercise.
    bb_posterior = [b["q"] / total for b in branches]
    btn_posterior = [[b["q"] * b["mass"][c] / btn_marginal[c] if btn_marginal[c] > 0 else 0.0
                      for b in branches] for c in range(1326)]
    sb_marginal = [math.fsum(b["q"] * sb[k][c] for k, b in enumerate(branches)) for c in range(1326)]
    bb_marginal = [total] * 1326  # the BB's masses are still uniform in both branches
    return {"f": [fA, fB], "q": [b["q"] for b in branches],
            "btn_marginal": btn_marginal, "btn_posterior": btn_posterior,
            "bb_posterior": bb_posterior,
            "translated": [b["translated"] for b in branches],
            "sb_marginal": sb_marginal, "bb_marginal": bb_marginal}


# --- synthetic golden sources, hands and wire shapes ---

SHORT_HANDED_FOLDS = [["UTG", "fold", 0], ["HJ", "fold", 0], ["CO", "fold", 0]]  # 3 dealt seats


def golden_node(history: list, actor: str, actions: list, likelihoods: dict, complement: int) -> dict:
    """One synthetic envelope node: action `i` of `likelihoods` takes `likelihoods[i](class)`,
    action `complement` the rest of each class row, so every row sums to 1 (spec section 8.2)."""
    weights = [[0.0] * 169 for _ in actions]
    for c in range(169):
        listed = [likelihoods[i](c) for i in sorted(likelihoods)]
        for i in likelihoods:
            weights[i][c] = likelihoods[i](c)
        weights[complement][c] = round(1.0 - math.fsum(listed), 12)
    return {"history": history, "actor": actor, "actions": actions, "weights": weights, "unreachable_classes": []}


def golden_source(bundle_id: str, nodes: list) -> dict:
    """A synthetic 100 bb bundle embedded in a golden: the manifest and the envelope's exact bytes
    (compact JSON), hashed as the loader hashes a bundle on disk."""
    envelope = {
        "bundle_id": bundle_id,
        "depth_bb": 100,
        "rake_profile": RAKE_PROFILE,
        "straddle": False,
        "class_order": CLASS_ORDER,
        "nodes": nodes,
    }
    text = json.dumps(envelope, separators=(",", ":"), allow_nan=False)
    manifest = {
        "bundle_id": bundle_id,
        "source": "PokerDataJson",
        "game": "nl",
        "version": 2,
        "depth_bb": 100,
        "depths": [100],
        "source_blinds": [0.5, 1.0],
        "rake_profile": RAKE_PROFILE,
        "rake": {"rate": 0.05, "cap_bb": 0.5, "no_flop_no_drop": True},
        "straddle": False,
        "ev_unit": "source_sb",
        "ev_reference": "unverified",
        "accuracy": "unverified",
        "license_note": "Synthetic golden test data (plan 3 Task 19); not vendor data; never a chart",
        "sha256": hashlib.sha256(text.encode("utf-8")).hexdigest(),
    }
    return {"manifest": manifest, "nodes_json": text}


def hand_config(bb_chips: int) -> dict:
    """A `proto::HandConfig` at `bb_chips` per big blind with the standard 5% / 0.5 bb rake, which
    is exactly the golden sources' own profile (no RakeProfileMapped reason)."""
    return {
        "config_revision": 1,
        "sb_chips": bb_chips // 2,
        "bb_chips": bb_chips,
        "straddle": None,
        "rake": {"kind": "pot_rake", "rate": 0.05, "cap_mchips": bb_chips * 500, "no_flop_no_drop": True},
        "chip_label": "$1",
    }


def act(seat: int, kind: str, to: int | None = None) -> dict:
    action = {"kind": kind} if to is None else {"kind": kind, "to": to}
    return {"seat": seat, "action": action}


def chips(to_bb_x1000: int, unit: int) -> int:
    """A source size in chips at `unit` chips per bb, rounded half up (spec section 8.4)."""
    return (to_bb_x1000 * unit + 500) // 1000


def f32_vector(xs: list) -> list:
    return [f32(x) for x in xs]


def normalized(xs: list) -> list:
    peak = max(xs)
    return f32_vector([x / peak for x in xs])


def bet_translation_reason(seat: int, s: float, mapped: list, deviation: Fraction) -> dict:
    """Spec section 8.4's disclosure of one translated wager, `prominent = d > 0.10`: `deviation` is
    the exact `d` (a `Fraction`), compared exactly with 1/10 as `prominence_cases` does (plan-3 final
    review F-M2), and written as a float for display only."""
    if not isinstance(deviation, Fraction):
        raise TypeError(f"the deviation must be the exact Fraction, not {type(deviation).__name__}")
    return {"kind": "BetTranslation", "street": "preflop", "seat": seat, "observed_pct": s,
            "mapped": [[size, f] for size, f in mapped], "deviation": float(deviation),
            "prominent": deviation > Fraction(1, 10)}


def build_replay_weights_golden() -> dict:
    # The uniform line: 1/2 chips, 100 bb, BTN = 0, SB = 1, BB = 2 (hero, holding As Ad, which
    # never enter any public range); BTN raises to the source's 2.5 bb, SB folds, BB calls; flop
    # Kh7d2c; the replay's target is the flop root.
    raise_to = chips(2500, 2)
    uniform = golden_source("golden_replay_uniform_100bb", [
        golden_node(SHORT_HANDED_FOLDS, "BTN", [{"step": "fold"}, raise_action(2500)], {1: btn_raise}, 0),
        golden_node(SHORT_HANDED_FOLDS + [["BTN", "raise", 2500]], "SB", [{"step": "fold"}, {"step": "call"}], {0: sb_fold}, 1),
        golden_node(SHORT_HANDED_FOLDS + [["BTN", "raise", 2500], ["SB", "fold", 0]], "BB", [{"step": "fold"}, {"step": "call"}], {1: bb_call}, 0),
    ])
    oracle = replay_golden_expected()
    ranges = [None if r is None else f32_vector(r) for r in oracle["ranges"]]
    translated = [[0, {"kind": "raise", "to": raise_to}], [1, {"kind": "fold"}], [2, {"kind": "call"}]]
    uniform_line = {
        "input": {
            "hand_id": 1,
            "config": hand_config(2),
            "button": 0,
            "hero": 2,
            "hero_cards": "AsAd",
            "dealt": [0, 1, 2],
            "stacks_start": [200, 200, 200],
            "actions": [act(0, "raise", raise_to), act(1, "fold"), act(2, "call")],
            "board": "Kh7d2c",
            "board_cards": [card_id(t) for t in ("Kh", "7d", "2c")],
            "likelihoods": {
                "btn_raise": "0.8 if class % 2 == 0 else 0.2 (fold takes the rest)",
                "sb_fold": "0.25 if class % 3 == 0 else 1.0 (call takes the rest)",
                "bb_call": "0.9 if class % 5 == 0 else 0.1 (fold takes the rest)",
                "precision": "each class likelihood as the f32 the source envelope carries",
            },
            "sources": [uniform],
        },
        "expected": {
            "ranges": ranges,
            "log_reach": oracle["log_reach"],
            "q": oracle["q"],
            "branches": [{"q": oracle["q"], "translated": translated}],
            "folded": [1],
            "reasons": [{"kind": "ShortHandedMapped", "dealt": 3}],
        },
    }

    # The off-menu line: 20/40 chips, 100 bb, the same seats; BTN raises to 113 chips (s = 0.73 at
    # the source parent: (113 - 40) / (60 + 40)), between the source menu's 2.25 bb (A = 0.5) and
    # 3.5 bb (B = 1.0); SB folds; the BB is to act preflop.
    unit = 40
    small, large = 2250, 3500
    offmenu_source = golden_source("golden_replay_offmenu_100bb", [
        golden_node(SHORT_HANDED_FOLDS, "BTN", [{"step": "fold"}, raise_action(small), raise_action(large)],
                    {1: btn_raise, 2: btn_raise_large}, 0),
        golden_node(SHORT_HANDED_FOLDS + [["BTN", "raise", small]], "SB", [{"step": "fold"}, {"step": "call"}], {0: sb_fold}, 1),
        golden_node(SHORT_HANDED_FOLDS + [["BTN", "raise", large]], "SB", [{"step": "fold"}, {"step": "call"}], {0: sb_fold}, 1),
    ])
    s = Fraction(113 * 1000 - 1000 * unit, 2500 * unit)
    menu = [Fraction(small * unit - 1000 * unit, 2500 * unit), Fraction(large * unit - 1000 * unit, 2500 * unit)]
    assert (s, menu) == (Fraction(73, 100), [Fraction(1, 2), Fraction(1)])
    off = replay_golden_offmenu_expected()
    sizes = {"A": chips(small, unit), "B": chips(large, unit)}
    named = {}
    for name, cards in (("AA", "AsAh"), ("AKs", "AsKs"), ("72o", "7s2c")):
        idx = combo_index(card_id(cards[:2]), card_id(cards[2:]))
        lo, hi = COMBO_PAIRS[idx]
        named[name] = {"cards": cards, "combo": idx, "class": combo_class(lo, hi), "posterior": off["btn_posterior"][idx]}
    marginals = [off["btn_marginal"], off["sb_marginal"], off["bb_marginal"]]
    offmenu_line = {
        "input": {
            "hand_id": 2,
            "config": hand_config(unit),
            "button": 0,
            "hero": 2,
            "hero_cards": None,
            "dealt": [0, 1, 2],
            "stacks_start": [100 * unit] * 3,
            "actions": [act(0, "raise", 113), act(1, "fold")],
            "board": "",
            "board_cards": [],
            "likelihoods": {
                "btn_raise_small": "0.8 if class % 2 == 0 else 0.2 (P(A | class), menu size 2.25 bb)",
                "btn_raise_large": "0.1 if class % 2 == 0 else 0.5 (P(B | class), menu size 3.5 bb)",
                "sb_fold": "0.25 if class % 3 == 0 else 1.0 after either size (call takes the rest)",
                "precision": "each class likelihood as the f32 the source envelope carries",
            },
            "pot_fractions": {"s": float(s), "menu": [float(x) for x in menu]},
            "sources": [offmenu_source],
        },
        "expected": {
            "f": off["f"],
            "q": off["q"],
            "branches": [
                {"q": q, "translated": [[0, {"kind": "raise", "to": sizes[label]}], [1, {"kind": "fold"}]]}
                for q, label in zip(off["q"], off["translated"])
            ],
            "ranges": [normalized(m) for m in marginals] + [None] * 3,
            "log_reach": [math.log(max(m)) for m in marginals] + [0.0] * 3,
            "btn_marginal": off["btn_marginal"],
            "btn_posterior": named,
            "bb_posterior": off["bb_posterior"],
            "reasons": [
                {"kind": "ShortHandedMapped", "dealt": 3},
                bet_translation_reason(0, float(s), [(float(menu[0]), off["f"][0]), (float(menu[1]), off["f"][1])],
                                       min(abs(s - x) for x in menu)),
            ],
        },
    }
    return {
        "schema_version": GOLDEN_SCHEMA_VERSION,
        "synthetic": True,
        "description": "Spec section 13.3 replay_weights_golden: two three-seat preflop replays through "
                       "synthetic class-varying sources; expected 1326 vectors, q, translated histories, "
                       "log_reach and the off-menu split's posteriors. Oracles: tools/gen_preflop_fixtures.py.",
        "tolerances": {"range": 1e-6, "log_reach": 1e-10, "q": 1e-10, "f": 1e-12, "posterior": 1e-10, "reason": 1e-6},
        "input": uniform_line["input"],
        "expected": uniform_line["expected"],
        "offmenu": offmenu_line,
    }


# --- bet translation (plan 3 Task 19 Step 4) ---

def bet_cases():
    return [
        {"name":"below","s":.2,"menu":[.5,1.],"f":[1.,0.],"deviation":.3,"clamped":True},
        {"name":"between","s":.73,"menu":[.5,1.],"f":[81/173,92/173],"deviation":.23,"clamped":False},
        {"name":"above_no_jam","s":1.5,"menu":[.5,1.],"f":[0.,1.],"deviation":.5,"clamped":True},
        {"name":"above_with_jam","s":1.5,"menu":[1.,2.],"f":[.4,.6],"deviation":.5,"clamped":False},
        {"name":"single","s":.73,"menu":[.5],"f":[1.],"deviation":.23,"clamped":True},
        {"name":"equal","s":.73,"menu":[.5,.5],"f":[1.,0.],"deviation":.23,"clamped":True},
    ]


def debug_action(a: dict) -> str:
    """An action as the Rust notes print it (`{:?}` of `proto::Action`)."""
    names = {"fold": "Fold", "check": "Check", "call": "Call", "bet": "Bet", "raise": "Raise", "allin": "AllIn"}
    return names[a["kind"]] if "to" not in a else f"{names[a['kind']]} {{ to: {a['to']} }}"


def menu_rank(a: dict) -> tuple:
    """Spec section 8.4's canonical menu order: fold, check, call, wagers ascending, all-in."""
    order = {"fold": 0, "check": 1, "call": 2, "bet": 3, "raise": 3, "allin": 4}
    return (order[a["kind"]], a.get("to", 0))


def legalize_expected(source: list, legal: list) -> dict:
    """Spec section 8.4's legality after mapping for one row, written against the rule: a source
    action legal at the node owns its destination; a raise below the minimum moves to the smallest
    legal source raise, else to call; a wager above the stack moves to all-in; a destination's
    probability is the sum; it keeps its own EV only when a legal source owns it, and a destination
    created by a move has no EV and `MovedProbability{from}`; every move is noted."""
    interval = next(((la["min_to"], la["max_to"]) for la in legal if la["kind"] in ("bet", "raise")), None)
    allin = next((la["to"] for la in legal if la["kind"] == "all_in"), None)

    def is_legal(a: dict) -> bool:
        for la in legal:
            if a["kind"] == la["kind"] and a["kind"] in ("fold", "check", "call"):
                return True
            if a["kind"] == la["kind"] and a["kind"] in ("bet", "raise") and la["min_to"] <= a["to"] <= la["max_to"]:
                return True
            if a["kind"] == "allin" and la["kind"] == "all_in" and a["to"] == la["to"]:
                return True
        return False

    legal_source = [is_legal(r["action"]) for r in source]
    targets = []
    for r, ok in zip(source, legal_source):
        a = r["action"]
        if ok:
            targets.append(a)
        elif a["kind"] == "raise" and a["to"] < interval[0]:
            raises = [x["action"] for x, y in zip(source, legal_source) if y and x["action"]["kind"] == "raise"]
            targets.append(min(raises, key=lambda x: x["to"]) if raises else {"kind": "call"})
        elif a["kind"] in ("bet", "raise") and a["to"] > interval[1]:
            targets.append({"kind": "allin", "to": allin})
        else:
            raise ValueError(f"no legal destination for {a}")
    menu = sorted({json.dumps(t, sort_keys=True): t for t in targets}.values(), key=menu_rank)
    actions, notes = [], []
    for dest in menu:
        rows = [(r, ok) for r, ok, t in zip(source, legal_source, targets) if t == dest]
        owners = [r for r, ok in rows if ok and r["action"] == dest]
        moved = [r for r, ok in rows if r["action"] != dest]
        ev = owners[0]["ev_chips"] if owners and all(o["ev_chips"] == owners[0]["ev_chips"] for o in owners) else None
        actions.append({
            "action": dest,
            "probability": math.fsum(r["probability"] for r, _ in rows),
            "ev_chips": ev,
            "unavailable": None if owners else {"kind": "MovedProbability", "from": moved[0]["action"]},
        })
    for r, t in zip(source, targets):
        if r["action"] != t:
            notes.append(f"Moved {debug_action(r['action'])} probability {f32_display(r['probability'])} to {debug_action(t)}")
    return {"actions": actions, "notes": notes}


def legal_move_cases() -> list:
    def row(action: dict, probability: float, ev: float) -> dict:
        return {"action": action, "probability": probability, "ev_chips": ev}

    facing = [{"kind": "fold"}, {"kind": "call", "cost": 2}, {"kind": "raise", "min_to": 10, "max_to": 100}, {"kind": "all_in", "to": 100}]
    opening = [{"kind": "check"}, {"kind": "bet", "min_to": 2, "max_to": 100}, {"kind": "all_in", "to": 100}]
    cases = [
        ("below_min_to_smallest_legal_raise", facing,
         [row({"kind": "call"}, .5, 2.0), row({"kind": "raise", "to": 7}, .2, 10.0), row({"kind": "raise", "to": 12}, .3, 3.0)]),
        ("below_min_to_call", facing,
         [row({"kind": "fold"}, .3, 0.0), row({"kind": "call"}, .5, 2.0), row({"kind": "raise", "to": 7}, .2, 10.0)]),
        ("above_stack_creates_allin", opening,
         [row({"kind": "check"}, .6, 1.0), row({"kind": "bet", "to": 120}, .4, 5.0)]),
        ("above_stack_joins_source_allin", opening,
         [row({"kind": "check"}, .5, 1.0), row({"kind": "bet", "to": 120}, .3, 5.0), row({"kind": "allin", "to": 100}, .2, 4.0)]),
    ]
    return [{"name": name, "legal": legal, "source": source, "expected": legalize_expected(source, legal)}
            for name, legal, source in cases]


HEADLINE_LABELS = {
    "Chart": "highest-frequency chart action",
    "PokerDataUnverified": "highest-frequency source action, EV reference unverified",
}


def headline_source(source: str, ev_reference: str) -> str:
    """The engine's source mapping for the headline wording (spec section 4.4)."""
    if source == "ChartTranscription":
        return "Chart"
    return "PokerDataUnverified" if ev_reference == "unverified" else "Solved"


def assemble_expected(case: dict) -> dict:
    """Hero's current decision over the case's branches, written against spec section 8.4's node
    translation and assembly and section 4.4's headline, for masses that are uniform except on the
    listed zero combos and node rows that are the same for every combo."""
    c, bb_chips = case["hero_combo"], case["bb_chips"]
    branches = case["branches"]
    by_id = {n["branch_id"]: n for n in case["nodes"]}
    node = [by_id.get(b["id"], {}).get("node") for b in branches]
    has = [node[k] is not None and not b["residual"] and b["stopped"] is None for k, b in enumerate(branches)]
    w = [0.0 if c in b["zero_combos"] else 1.0 for b in branches]
    r = math.fsum(b["q"] * w[k] for k, b in enumerate(branches))
    pi = [b["q"] * w[k] / r if r > 0 else 0.0 for k, b in enumerate(branches)]
    unresolved = math.fsum(pi[k] for k in range(len(branches)) if not has[k])
    positive = [k for k in range(len(branches)) if pi[k] > 0]
    out = {"actions": [], "unresolved_mass": unresolved, "range_mix": None, "reasons": [], "notes": [],
           "unsupported": None, "headline": None}
    heaviest = lambda ks: max(ks, key=lambda k: (branches[k]["q"], -branches[k]["id"]))  # noqa: E731
    if not any(has) or (positive and all(not has[k] for k in positive)):
        k = heaviest(positive or range(len(branches)))
        out["unsupported"] = {"kind": "MissingPreflopNode", "key": by_id[branches[k]["id"]]["key"]}
        return out
    probs = [None if n is None else [p / math.fsum(n["probs"]) for p in n["probs"]] for n in node]
    menu = sorted({json.dumps(a, sort_keys=True): a for k in range(len(branches)) if has[k] for a in node[k]["actions"]}.values(),
                  key=menu_rank)
    mass = [b["q"] * (1326 - len(b["zero_combos"])) for b in branches]
    covered = math.fsum(mass[k] for k in range(len(branches)) if has[k])
    uncovered = math.fsum(mass[k] for k in range(len(branches)) if not has[k])
    out["range_mix"] = [[a, math.fsum(mass[k] * probs[k][node[k]["actions"].index(a)] for k in range(len(branches))
                                      if has[k] and a in node[k]["actions"]) / covered] for a in menu]
    contributing = [node[k] for k in range(len(branches)) if has[k]]
    if any(n["source"] == "ChartTranscription" for n in contributing):
        out["reasons"].append({"kind": "ChartRounded"})
    if any(n["source"] == "PokerDataJson" and n["ev_reference"] == "unverified" for n in contributing):
        out["reasons"].append({"kind": "EvReferenceUnverified"})
    excluded = uncovered / (covered + uncovered)
    if excluded > 0:
        out["notes"].append(f"range mix excludes {100 * excluded:.1f}% of hero's public range mass (no strategy)")
    if not positive:
        out["unsupported"] = {"kind": "HeroComboOutOfSupport"}
        out["actions"] = [{"action": a, "frequency": None, "ev_bb": None, "unavailable": {"kind": "HeroOutOfSupport"},
                           "headline": False} for a in menu]
    else:
        same = len({node[k]["ev_reference"] for k in positive if has[k]}) == 1
        for a in menu:
            present = [k for k in positive if has[k] and a in node[k]["actions"]]
            with_ev = [k for k in present if node[k]["evs"][node[k]["actions"].index(a)] is not None]
            freq = math.fsum(pi[k] * probs[k][node[k]["actions"].index(a)] for k in present)
            ev, why = None, None
            if not present:
                why = {"kind": "NotInMenu"}
            elif len(with_ev) == len(positive) and same:
                ev = math.fsum(pi[k] * node[k]["evs"][node[k]["actions"].index(a)] for k in positive) / bb_chips
            elif len(present) != len(positive):
                why = {"kind": "BranchSupportIncomplete", "covered_posterior": math.fsum(pi[k] for k in with_ev)}
            elif all(node[k]["source"] == "ChartTranscription" for k in present):
                why = {"kind": "ChartNoEv"}
            else:
                why = {"kind": "NoEvReference"}
            out["actions"].append({"action": a, "frequency": freq, "ev_bb": ev, "unavailable": why, "headline": False})
    if unresolved > 0:
        # Spec section 8.4's causes, only those actually incurred (section 2; ruling 19-I1): the cap
        # residual's own share is "cap"; the share of positive branches whose key has no node (stopped,
        # or live without one) is "missing node <key>", the heaviest such branch's key.
        capped = [k for k in positive if not has[k] and branches[k]["residual"]]
        missing = [k for k in positive if not has[k] and not branches[k]["residual"]]
        if capped:
            out["reasons"].append({"kind": "BranchResidual", "seat": case["hero"],
                                   "residual_mass_pct": 100 * math.fsum(pi[k] for k in capped), "cause": "cap"})
        if missing:
            keyed = [k for k in missing if by_id.get(branches[k]["id"], {}).get("key")]
            key = by_id[branches[heaviest(keyed)]["id"]]["key"] if keyed else "no retained key"
            out["reasons"].append({"kind": "BranchResidual", "seat": case["hero"],
                                   "residual_mass_pct": 100 * math.fsum(pi[k] for k in missing), "cause": f"missing node {key}"})
        out["notes"].append(f"{100 * unresolved:.1f}% of the posterior has no strategy")
    # Section 4.4's headline, only ever with nothing unresolved.
    acts = out["actions"]
    if acts and unresolved == 0:
        if all(x["ev_bb"] is not None for x in acts):
            best = max(range(len(acts)), key=lambda i: (acts[i]["ev_bb"], acts[i]["frequency"], -i))
            acts[best]["headline"], out["headline"] = True, "highest EV"
        elif all(x["frequency"] is not None for x in acts):
            incomplete = any((x["unavailable"] or {}).get("kind") == "BranchSupportIncomplete" for x in acts)
            kinds = {headline_source(n["source"], n["ev_reference"]) for n in contributing}
            label = "highest-frequency action, EV incomplete" if incomplete else HEADLINE_LABELS.get(kinds.pop())
            if label is not None:
                best = max(range(len(acts)), key=lambda i: (acts[i]["frequency"], -i))
                acts[best]["headline"], out["headline"] = True, label
    return out


def assembly_cases() -> list:
    fold, call = {"kind": "fold"}, {"kind": "call"}

    def raise_to(to: int) -> dict:
        return {"kind": "raise", "to": to}

    def branch(id_: int, q: float, residual: bool = False, zero: list | None = None, stopped: str | None = None) -> dict:
        return {"id": id_, "q": q, "residual": residual, "stopped": stopped, "zero_combos": zero or []}

    def at(branch_id: int, key: str, actions: list | None = None, probs: list | None = None, evs: list | None = None,
           source: str = "PokerDataJson", ev_reference: str = "decision_incremental_verified") -> dict:
        node = None if actions is None else {"actions": actions, "probs": probs, "evs": evs, "source": source,
                                            "ev_reference": ev_reference}
        return {"branch_id": branch_id, "key": key, "node": node}

    hero_combo = combo_index(card_id("Qs"), card_id("Js"))
    t7_a = dict(actions=[fold, call, raise_to(60)], probs=[.3, .3, .4], evs=[0.0, 2.0, 10.0])
    t7_b = dict(actions=[fold, call], probs=[.4, .6], evs=[0.0, -1.0])
    menu = [fold, call, raise_to(25)]
    cases = [
        # T7 (spec section 13.1): posterior 0.2 / 0.8, a raise present only in the 0.2 branch.
        ("t7_branch_support_incomplete", [branch(0, .2), branch(1, .8)],
         [at(0, "golden:after raise A", **t7_a), at(1, "golden:after raise B", **t7_b)]),
        ("chart_frequency", [branch(0, 1.0)],
         [at(0, "golden:chart", menu, [.5, .2, .3], [None, None, None], "ChartTranscription", "unverified")]),
        ("unverified_source_frequency", [branch(0, 1.0)],
         [at(0, "golden:unverified", menu, [.2, .5, .3], [None, None, None], "PokerDataJson", "unverified")]),
        ("complete_ev", [branch(0, .2), branch(1, .8)],
         [at(0, "golden:after raise A", [fold, call], [.3, .7], [0.0, 4.0]),
          at(1, "golden:after raise B", [fold, call], [.6, .4], [0.0, -2.0])]),
        # T7's residual: a residual branch holds 0.05 of hero's posterior.
        ("t7_residual_no_headline", [branch(0, .19), branch(1, .76), branch(2, .05, residual=True)],
         [at(0, "golden:after raise A", **t7_a), at(1, "golden:after raise B", **t7_b)]),
        ("missing_node_everywhere", [branch(0, .6), branch(1, .4)],
         [at(0, "golden:after raise A"), at(1, "golden:after raise B")]),
        ("hero_out_of_support", [branch(0, 1.0, zero=[hero_combo])],
         [at(0, "golden:out of support", menu, [.5, .3, .2], [0.0, 1.0, 3.0])]),
        # A node actually missing (ruling 19-I1): branch B's lookup found no node for hero, so its
        # share is "missing node <B's key>".
        ("missing_node_partial", [branch(0, .6), branch(1, .4)],
         [at(0, "golden:after raise A", **t7_b), at(1, "golden:after raise B")]),
        # Both causes at once: the cap residual (0.5), a branch stopped on a missing node (0.1) and a
        # live branch with hero's node (0.4): one reason per cause, shares 50% and 10%.
        ("cap_beside_missing_node",
         [branch(0, .5, residual=True), branch(1, .1, stopped="missing node golden:k-missing"), branch(2, .4)],
         [at(1, "golden:k-missing"), at(2, "golden:k-live", **t7_b)]),
    ]
    out = []
    for name, branches, nodes in cases:
        case = {"name": name, "hero": 0, "hero_combo": hero_combo, "bb_chips": 10, "branches": branches, "nodes": nodes}
        case["expected"] = assemble_expected(case)
        out.append(case)
    return out


def prominence_cases() -> dict:
    """BetTranslation prominence at its boundary through the replay: six seats, 5/10 chips, 100 bb;
    UTG, HJ, CO, BTN and SB limp, and the BB raises off the source menu, whose only size is 2 bb.
    At the BB's source parent the pot is 6 bb and there is nothing to call, so a raise to `to`
    chips has pot fraction `(to - 10) / 60` and the menu size `(20 - 10) / 60`: a raise to 26 lies
    exactly 0.10 above it (in exact arithmetic and in doubles alike), a raise to 27 just above."""
    unit, size = 10, 2000
    limp = [["UTG", "call", 0], ["HJ", "call", 0], ["CO", "call", 0], ["BTN", "call", 0], ["SB", "call", 0]]
    half = lambda c: .5  # noqa: E731 - combo-independent
    nodes = [golden_node(limp[:i], limp[i][0], [{"step": "fold"}, {"step": "call"}], {1: half}, 0) for i in range(5)]
    nodes.append(golden_node(limp, "BB", [{"step": "check"}, raise_action(size)], {1: half}, 0))
    source = golden_source("golden_prominence_100bb", nodes)
    a = Fraction(size * unit - 1000 * unit, 6000 * unit)
    cases = []
    for name, to in (("deviation_exactly_0.10", 26), ("deviation_above_0.10", 27)):
        s = Fraction(to * 1000 - 1000 * unit, 6000 * unit)
        d = s - a
        cases.append({
            "name": name,
            "input": {
                "hand_id": 3,
                "config": hand_config(unit),
                "button": 0,
                "hero": 2,
                "hero_cards": None,
                "dealt": [0, 1, 2, 3, 4, 5],
                "stacks_start": [100 * unit] * 6,
                "actions": [act(3, "call"), act(4, "call"), act(5, "call"), act(0, "call"), act(1, "call"), act(2, "raise", to)],
                "board": "",
                "board_cards": [],
            },
            "deviation_exact": f"{d.numerator}/{d.denominator}",
            "expected_prominent": d > Fraction(1, 10),
            "expected": {"reasons": [bet_translation_reason(2, float(s), [(float(a), 1.0)], d)]},
        })
    return {"sources": [source], "cases": cases}


def build_bet_translation_golden() -> dict:
    return {
        "schema_version": GOLDEN_SCHEMA_VERSION,
        "synthetic": True,
        "description": "Spec section 13.3 bet_translation_golden: section 8.4's interpolation boundaries, "
                       "BetTranslation prominence at d = 0.10, legality after mapping, and the branch-supported "
                       "assembly with its section 4.4 headline. Oracles: tools/gen_preflop_fixtures.py.",
        "tolerances": {"f": 1e-12, "deviation": 1e-12, "probability": 1e-6, "reason": 1e-6},
        "interpolation": bet_cases(),
        "prominence": prominence_cases(),
        "legal_moves": legal_move_cases(),
        "assembly": assembly_cases(),
    }


def generate_goldens(out_dir: Path) -> None:
    write_json(out_dir / "replay_weights_golden.json", build_replay_weights_golden())
    write_json(out_dir / "bet_translation_golden.json", build_bet_translation_golden())


def main() -> None:
    generate(OUT_DIR)
    print(f"wrote 6 synthetic preflop fixtures to {OUT_DIR}")
    generate_goldens(ENGINE_GOLDEN_DIR)
    print(f"wrote {len(GOLDEN_FILES)} engine goldens to {ENGINE_GOLDEN_DIR}")


if __name__ == "__main__":
    main()
