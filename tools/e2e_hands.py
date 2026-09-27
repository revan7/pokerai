"""Define the fifty recorded end-to-end benchmark inputs as code (plan 4 Task 18).

`records()` is the single frozen inventory: 22 supported (numeric-EV release subset), 14 special
(exceptional-but-defined inputs -- straddle mapping, multiway/projection edge cases, missing preflop
nodes, hero-out-of-support combos) and 14 fault-injection records, always in that order and always
50 rows with `id` "001".."050" assigned in inventory order.

This is a benchmark input format, translated into proto's actual wire commands by Task 21 -- it does
not replace `proto` and never depends on its JSON field layout. Task 19 writes/verifies the fifty
fixture files and a manifest from this module; nothing here writes a file. `records()` does read the
frozen chart lock (`bench/spots/sources.json`) and the two chart envelopes it pins, to run the
generation-time hero-support guard (`verify_hero_support`); a failing guard fails generation for
review and never selects a replacement hand.

Every record is a complete, independent, explicit v1 dict (`version`, `config`, `button`, `dealt`,
`stacks`, `hero`, `hero_cards`, `events`, `fault`, `class`, `expected`, `id`) -- no record holds a
runtime reference to another one; `deepcopy` is used wherever a record starts from another's shape
so later mutation of one can never leak into another (see `test_records_are_deterministic_and_
independent_across_calls`).

Seats are numbered BTN=0, SB=1, BB=2, UTG=3, HJ=4, CO=5 (six dealt seats, `base()`'s fixed
preflop acting order below). Hero's cards are carried on the record for hero-combo equity and
terminal calculations only -- they never enter a public range, solve input or cache key (spec §2;
CLAUDE.md §6).

Fix round 1 (review I1-I5): the default hero is the audited constant below instead of the plan's
8h7h (zero support in PokerCoaching 100bb); the opening flop bet of the projection/multiway records
is one big blind (core-model's minimum bet); 021/022 require `UnconditionedPriorStreet` (SB's node
at prefix RFFC is absent, spec 9.3); every chart-derived record requires `RakeProfileMapped` (both
frozen manifests declare undocumented rake); straddle source depth is measured in straddles (spec
8.3), so 023/025 are 50 source bb mapped to the 100bb bundle with `DepthBucket`/`AsymmetricStacks`.
"""
from __future__ import annotations

import hashlib
import json
from copy import deepcopy
from fractions import Fraction
from pathlib import Path

from chart_ingest import CLASS_ORDER, RANKS, bounded_read, class_names
from chart_sources import history_key

REPO = Path(__file__).resolve().parents[1]

FAULTS = (
    "Oom", "Eof", "Malformed", "Oversized", "TreeMismatch",
    "BlockedCacheIo", "SlowAllocation", "NoIteration", "ClockJump", "SuspendResume",
)

# The fixed hero of every record that does not set its own (001-030, 037-050). Chosen once, at
# fix round 1 (I1), from an enumeration of the frozen charts -- never re-selected at runtime; the
# guard below only verifies it. The hero preflop decisions those records make at a present node
# are: BB call vs a BTN open (FFFRF) and vs a CO open (FFRFF) in both bundles, BTN cold-call vs a
# UTG open (RFF), the straddler's call as the virtual BB (RFFFF) and the UTG open (root), all three
# at 100bb.
# Enumerating all 169 classes, exactly two have positive weight at all seven: 77 and 66. 66 is the
# less mixed of the two, so it is the committed class (class index 112, A-2 row-major):
#
#   bundle              key     actor  action  66 weights (fold, call[, raise])
#   pokercoaching_100   FFFRF   BB     call    0.0, 1.0, 0.0
#   rangeconverter_200  FFFRF   BB     call    0.0, 0.5, 0.5   <- the one mixed cell (77: two)
#   pokercoaching_100   FFRFF   BB     call    0.0, 1.0, 0.0
#   rangeconverter_200  FFRFF   BB     call    0.0, 1.0, 0.0
#   pokercoaching_100   RFF     BTN    call    0.0, 1.0, 0.0
#   pokercoaching_100   RFFFF   BB     call    0.0, 1.0, 0.0
#   pokercoaching_100   (root)  UTG    raise   0.0, 1.0 (fold, raise)
#
# The cards are the first 66 combo in s-h-d-c suit order disjoint from every board card used
# (Kh 7d 2c, Jh 9h 6c, 8s 8d 3c, 4d, 2s): 6c is on the Jh9h6c board, so 6s6h.
DEFAULT_HERO_CLASS = "66"
DEFAULT_HERO_CARDS = "6s6h"

# Reasons incurred by the first chart lookup of every record: both frozen chart bundles are
# charts (`ChartRounded`) whose manifests declare `rake_profile: undocumented` (`RakeProfileMapped`,
# spec 8.3 rake rule: any difference is labelled).
CHART_REASONS = ("ChartRounded", "RakeProfileMapped")


def chart_reasons(*extra: str) -> list:
    """A fresh required-reasons list: the chart reasons followed by `extra`, in order."""
    return [*CHART_REASONS, *extra]


def act(seat: int, street: str, kind: str, to: int | None = None) -> dict:
    action = {"kind": kind}
    if to is not None:
        action["to"] = to
    return {"type": "action", "seat": seat, "street": street, "action": action}


def board(cards: str) -> dict:
    return {"type": "board", "cards": cards}


def base(depth: int = 100, opener: int = 0, flop: str = "Kh7d2c") -> dict:
    """Common HU-vs-BB single-raised-pot preflop skeleton: BTN0 SB1 BB2 UTG3 HJ4 CO5 act in that
    order; `opener` raises to 250, BB (seat 2) always calls, everyone else folds. Depth is `depth`
    bb per seat, integer chips throughout (BB100). Hero is BB with `DEFAULT_HERO_CARDS`."""
    events = []
    for seat in (3, 4, 5, 0, 1, 2):
        if seat == opener:
            events.append(act(seat, "preflop", "raise", 250))
        elif seat == 2:
            events.append(act(seat, "preflop", "call"))
        else:
            events.append(act(seat, "preflop", "fold"))
    return {
        "version": 1,
        "config": {
            "sb_chips": 50, "bb_chips": 100, "straddle": None,
            "rake": {"rate": 0.05, "cap_mchips": 50000, "no_flop_no_drop": True},
            "flop_budget_s": 10, "threads": 16, "target_bp": 50,
        },
        "button": 0, "dealt": list(range(6)), "stacks": [depth * 100] * 6,
        "hero": 2, "hero_cards": DEFAULT_HERO_CARDS, "events": events + [board(flop)], "fault": None,
    }


def expect(row: dict, cls: str, numeric: bool, reasons: list, unsupported: str | None = None) -> dict:
    row["class"] = cls
    row["expected"] = {
        "numeric_ev": numeric,
        "coverage": "Approximate" if unsupported is None else "Unsupported",
        "required_reasons": reasons,
        "unsupported": unsupported,
        "flop_miss_accuracy": "raw_target_or_DeadlineBestSoFar" if cls == "hu_flop_srp" else None,
    }
    return row


def _projection_preflop() -> list:
    """UTG opens to 250, HJ/CO fold, BTN cold-calls, SB folds, BB calls; flop Kh7d2c. SB's node at
    prefix RFFC is absent from both frozen bundles, so preflop replay stops there (spec 9.3)."""
    return [act(3, "preflop", "raise", 250), act(4, "preflop", "fold"), act(5, "preflop", "fold"),
            act(0, "preflop", "call"), act(1, "preflop", "fold"), act(2, "preflop", "call"), board("Kh7d2c")]


def supported_records() -> list:
    """The 22 designated supported records (numeric-EV release subset): 8 `hu_flop_srp`, 6
    `hu_turn`, 6 `hu_river`, 2 `projection_admitted`. Exact action inputs, not a random sample --
    IDs are assigned later, in `records()`, by inventory order."""
    out = []
    for depth in (100, 200):
        for opener in (0, 5):
            for bet in (275, 401):
                r = base(depth, opener)
                r["events"] += [act(2, "flop", "check"), act(opener, "flop", "bet", bet)]
                out.append(expect(r, "hu_flop_srp", True, chart_reasons()))
    for depth, opener, flop in [(100, 0, "Kh7d2c"), (100, 5, "Jh9h6c"), (100, 0, "8s8d3c"),
                                (200, 0, "Kh7d2c"), (200, 5, "Jh9h6c"), (200, 0, "8s8d3c")]:
        r = base(depth, opener, flop)
        r["events"] += [act(2, "flop", "check"), act(opener, "flop", "bet", 275), act(2, "flop", "call"),
                        board(flop + "4d"), act(2, "turn", "check"), act(opener, "turn", "bet", 550)]
        out.append(expect(r, "hu_turn", True, chart_reasons("UnconditionedPriorStreet")))
    for index, (depth, opener, flop) in enumerate([(100, 0, "Kh7d2c"), (100, 5, "Jh9h6c"), (200, 0, "8s8d3c"),
                                                  (200, 5, "Kh7d2c"), (100, 0, "Jh9h6c"), (200, 0, "8s8d3c")]):
        r = base(depth, opener, flop)
        r["events"] += [act(2, "flop", "check"), act(opener, "flop", "bet", 275), act(2, "flop", "call"),
            board(flop + "4d"), act(2, "turn", "check"), act(opener, "turn", "check"), board(flop + "4d2s"),
            act(2, "river", "check"), act(opener, "river", "allin" if index >= 4 else "bet",
                                   depth * 100 - 525 if index >= 4 else 550)]
        out.append(expect(r, "hu_river", True, chart_reasons("UnconditionedPriorStreet")))
    # Projection records: the opening flop bet is one big blind (100, core-model's minimum bet);
    # the later flop raises are legal: to 300 (minimum 200) and to 500 (exactly the minimum
    # re-raise, 300 + the last full raise of 200). Unpaid (021): UTG folds 0 dead, hero
    # BB faces 300 (call 200, min raise to 500). Paid (022): UTG calls 100 then folds (100 dead),
    # hero BTN faces 500 with 300 in (call 200, min raise to 700).
    for paid in (False, True):
        r = base()
        r["events"] = _projection_preflop() + [act(2, "flop", "bet", 100)]
        if paid:
            r["hero"] = 0
            r["events"] += [act(3, "flop", "call"), act(0, "flop", "raise", 300),
                            act(2, "flop", "raise", 500), act(3, "flop", "fold")]
        else:
            r["events"] += [act(3, "flop", "fold"), act(0, "flop", "raise", 300)]
        out.append(expect(r, "projection_admitted", True,
                          chart_reasons("MultiwayStreetRoot", "UnconditionedPriorStreet")))
    return out


def special_records(supported: list) -> list:
    """The 14 special (exceptional-but-defined) records: 4 straddle mapping, 1 projection
    rejection, 1 multiway root, 1 third-all-in multiway, 1 multiway side pot, 3 missing preflop
    nodes, 3 hero-out-of-support combos. Each is a complete copy with explicit events, never an
    expectation-only sample."""
    out = []
    for sb, bb, straddle in ((1, 2, 4), (2, 5, 10)):
        for depth in (100, 200):
            r = base(depth)
            r["hero"] = 3
            r["stacks"] = [depth * bb] * 6
            r["config"].update(sb_chips=sb, bb_chips=bb, straddle=straddle)
            r["config"]["rake"]["cap_mchips"] = bb * 500
            r["events"] = [act(4, "preflop", "raise", straddle * 5 // 2),
                act(5, "preflop", "fold"), act(0, "preflop", "fold"), act(1, "preflop", "fold"),
                act(2, "preflop", "fold"), act(3, "preflop", "call"), board("Kh7d2c")]
            # Spec 8.3: source bb = the straddle, so `depth` physical bb is depth * bb / straddle
            # source bb: 50 (023, 025) or 100 (024, 026). Both map to the 100bb bundle; the 50 ->
            # 100 mapping is labelled `DepthBucket` and `AsymmetricStacks` (any difference labels).
            mapped = [] if depth * bb == 100 * straddle else ["DepthBucket", "AsymmetricStacks"]
            out.append(expect(r, "straddle_mapping", False, chart_reasons("StraddleMapped", *mapped)))
    pre = _projection_preflop()
    r = base()
    r["hero"] = 3
    r["events"] = deepcopy(pre) + [act(2, "flop", "bet", 100),
        act(3, "flop", "call"), act(0, "flop", "raise", 300), act(2, "flop", "fold")]
    out.append(expect(r, "projection_rejected", False, chart_reasons(), "UnsupportedHistory"))
    r = base()
    r["events"] = deepcopy(pre)
    out.append(expect(r, "multiway", False, chart_reasons(), "MultiwayEv"))
    r = deepcopy(r)
    r["stacks"][3] = 250
    r["events"][0] = act(3, "preflop", "allin", 250)
    out.append(expect(r, "third_allin", False, chart_reasons(), "MultiwayEv"))
    # UTG (300 behind before the hand) has 50 left on the flop: a short all-in call of the 100 bet.
    r = base()
    r["stacks"][3] = 300
    r["events"] = deepcopy(pre) + [
        act(2, "flop", "bet", 100), act(3, "flop", "allin", 50), act(0, "flop", "call"),
        board("Kh7d2c4d"), act(2, "turn", "bet", 100), act(0, "turn", "call"), board("Kh7d2c4d2s")]
    out.append(expect(r, "multiway_side_pot", False, chart_reasons(), "MultiwayEv"))
    missing = [(4, [act(3, "preflop", "call")]),
        (0, [act(3, "preflop", "fold"), act(4, "preflop", "fold"), act(5, "preflop", "call")]),
        (4, [act(3, "preflop", "fold"), act(4, "preflop", "raise", 250), act(5, "preflop", "raise", 850),
            act(0, "preflop", "fold"), act(1, "preflop", "call"), act(2, "preflop", "fold")])]
    for hero, events in missing:
        r = base()
        r["hero"] = hero
        r["hero_cards"] = "AhAd"
        r["events"] = events
        out.append(expect(r, "missing_preflop", False, chart_reasons(), "MissingPreflopNode"))
    for n, depth in enumerate((100, 200, 100)):
        r = base(depth)
        r["hero"] = 0
        r["hero_cards"] = "7c2d"
        r["events"] += [act(2, "flop", "check")]
        if n == 2:
            r["events"] += [act(0, "flop", "check"), board("Kh7d2c4d"), act(2, "turn", "check")]
        out.append(expect(r, "hero_out_of_support", False, chart_reasons(), "HeroComboOutOfSupport"))
    assert len(out) == 14
    return out


def fault_records(supported: list) -> list:
    """The 14 fault-injection records: the 10 named faults (`FAULTS`) applied to the default
    supported baseline (record 001, `hu_flop_srp`), then 4 budget-specific variants applied to
    other supported records. Task 22 owns the fault semantics themselves; this only names, per
    record, which fault is injected and the coverage reason it forces."""
    out = []
    for fault in FAULTS:
        r = deepcopy(supported[0])
        r["fault"] = fault
        reason = "EngineError" if fault in FAULTS[:5] else "DeadlineExceeded"
        out.append(expect(r, "failure_injection", False, chart_reasons(), reason))
    for index, fault, budget in ((8, "Oom", 10), (14, "Eof", 10), (8, "NoIteration", 10), (0, "SlowAllocation", 10)):
        r = deepcopy(supported[index])
        r["fault"] = fault
        r["config"]["flop_budget_s"] = budget
        out.append(expect(r, "failure_injection", False, chart_reasons(),
                          "EngineError" if fault in ("Oom", "Eof") else "DeadlineExceeded"))
    return out


# --- generation-time hero-support guard (fix round 1, I1) ---

# Physical position by offset from the button, and the spec 8.3 lookup-only virtual roles under a
# straddle (HJ->UTG, CO->HJ, BTN->CO, SB->BTN, BB->SB, straddler->BB).
_POSITIONS = ("BTN", "SB", "BB", "UTG", "HJ", "CO")
_STRADDLE_ROLES = {"HJ": "UTG", "CO": "HJ", "BTN": "CO", "SB": "BTN", "BB": "SB", "UTG": "BB"}


def hand_class(cards: str) -> str:
    """The 169-class name of two cards written as e.g. "6s6h" ("66"), "AhKh" ("AKs"), "7c2d"
    ("72o"), in `chart_ingest.class_names()`'s naming."""
    (r1, s1), (r2, s2) = (cards[0], cards[1]), (cards[2], cards[3])
    if r1 == r2:
        return r1 + r2
    hi, lo = sorted((r1, r2), key=RANKS.index)
    return hi + lo + ("s" if s1 == s2 else "o")


def nearest_depth(depth: Fraction, acquired: list) -> int:
    """Spec 8.3: the nearest acquired depth, ties to the deeper one; above the deepest acquired
    depth clamps to it."""
    ordered = sorted(acquired)
    if depth >= ordered[-1]:
        return ordered[-1]
    return min(ordered, key=lambda d: (abs(d - depth), -d))


def lookup_depth(record: dict, actor: int, eligible: list) -> Fraction:
    """Spec 8.3 per-prefix depth: min(actor's start stack, max over the other eligible seats'
    start stacks) / unit, unit = straddle chips when a straddle is configured else bb_chips."""
    unit = record["config"]["straddle"] or record["config"]["bb_chips"]
    others = [record["stacks"][s] for s in eligible if s != actor]
    return Fraction(min(record["stacks"][actor], max(others)), unit)


def load_frozen_charts(repo: Path = REPO) -> dict:
    """`{bundle name: {"depth": depth_bb, "nodes": set of covered keys from the lock, "chart":
    {history key: node}}}` for every bundle of the frozen lock. Each envelope's sha256 must equal
    the lock's, its class order must be the section-4.1 order, and every key the lock lists as
    covered must be a node of the envelope; anything else raises `ValueError`."""
    lock = json.loads(bounded_read(repo / "bench/spots/sources.json").decode("utf-8"))
    out = {}
    for bundle in lock["bundles"]:
        name = bundle["name"]
        raw = bounded_read(repo / "fixtures/charts" / f"{name}.json")
        if hashlib.sha256(raw).hexdigest() != bundle["sha256"]:
            raise ValueError(f"chart {name} does not match the sha256 frozen in bench/spots/sources.json")
        envelope = json.loads(raw.decode("utf-8"))
        if envelope["class_order"] != CLASS_ORDER:
            raise ValueError(f"chart {name} has class order {envelope['class_order']!r}, not {CLASS_ORDER!r}")
        chart = {history_key(n["history"]): n for n in envelope["nodes"]}
        nodes = set(bundle["nodes"])
        if not nodes <= set(chart):
            raise ValueError(f"lock lists {sorted(nodes - set(chart))} as covered but chart {name} lacks them")
        out[name] = {"depth": bundle["manifest"]["depth_bb"], "nodes": nodes, "chart": chart}
    return out


def _menu_index(node: dict, action: dict, unit: int) -> int | None:
    """Index of the node's menu action matching an observed preflop action: same step, and for a
    raise the same size within half a chip (spec 8.3 sizes rule); `None` when off the menu."""
    for i, a in enumerate(node["actions"]):
        if a["step"] != action["kind"]:
            continue
        if a["step"] == "raise" and abs(action["to"] * 1000 - a["to_bb_x1000"] * unit) > 500:
            continue
        return i
    return None


def hero_preflop_lookups(record: dict, frozen: dict) -> list:
    """Replay the record's preflop actions against the frozen charts, bundle chosen per prefix by
    the spec 8.3 depth rule, and return `[(bundle, key, action label, hero class weight)]` for each
    of hero's own actions at a present node. Stops at the first absent node (spec 9.3: the branch
    stops for the rest of the preflop street, so hero's range is unconditioned after it)."""
    cfg = record["config"]
    unit = cfg["straddle"] or cfg["bb_chips"]
    by_depth = {v["depth"]: name for name, v in frozen.items()}
    class_index = class_names().index(hand_class(record["hero_cards"]))
    letters, folded, out = "", set(), []
    for e in record["events"]:
        if e["type"] != "action" or e["street"] != "preflop":
            continue
        seat, action = e["seat"], e["action"]
        eligible = [s for s in record["dealt"] if s not in folded]
        bundle = by_depth[nearest_depth(lookup_depth(record, seat, eligible), list(by_depth))]
        if letters not in frozen[bundle]["nodes"]:
            break
        node = frozen[bundle]["chart"][letters]
        role = _POSITIONS[(seat - record["button"]) % 6]
        if cfg["straddle"]:
            role = _STRADDLE_ROLES[role]
        if node["actor"] != role:
            raise ValueError(f"record {record['id']}: {bundle} node {letters!r} belongs to {node['actor']}, "
                             f"but seat {seat} acts there as {role}")
        if seat == record["hero"]:
            label = action["kind"] + (f" to {action['to']}" if "to" in action else "")
            index = _menu_index(node, action, unit)
            if index is None:
                raise ValueError(f"record {record['id']} ({record['class']}): hero's {label} is not on the "
                                 f"menu of {bundle} node {letters!r} ({role})")
            out.append((bundle, letters, f"{role} {label}", node["weights"][index][class_index]))
        letters += history_key([(None, action["kind"], None)])
        if action["kind"] == "fold":
            folded.add(seat)
    return out


def verify_hero_support(rows: list, repo: Path = REPO) -> None:
    """Generation guard (fix round 1, I1): for every record, replay hero's own preflop actions
    against the frozen charts. A record that promises support raises `ValueError` naming the record
    id, bundle, key and action at the first zero class weight; a `hero_out_of_support` record must
    instead have a zero weight at one of hero's actions. Never selects a replacement hand."""
    frozen = load_frozen_charts(repo)
    for r in rows:
        lookups = hero_preflop_lookups(r, frozen)
        cards = f"{r['hero_cards']} ({hand_class(r['hero_cards'])})"
        if r["class"] == "hero_out_of_support":
            if not any(weight == 0 for *_, weight in lookups):
                raise ValueError(f"record {r['id']} ({r['class']}): hero {cards} has positive weight at every "
                                 f"preflop decision {[(b, k, a) for b, k, a, _ in lookups]}; expected zero support")
            continue
        for bundle, key, label, weight in lookups:
            if weight <= 0:
                raise ValueError(f"record {r['id']} ({r['class']}): hero {cards} has zero weight at {bundle} "
                                 f"node {key!r} for {label}")


def records() -> list:
    """The frozen fifty: 22 supported + 14 special + 14 fault-injection, in that fixed order, with
    `id` "001".."050" assigned by inventory position, verified by `verify_hero_support` before
    they are returned. Every call returns fresh, independent dicts."""
    supported = supported_records()
    rows = supported + special_records(supported) + fault_records(supported)
    for i, row in enumerate(rows, 1):
        row["id"] = f"{i:03}"
    assert len(rows) == 50
    verify_hero_support(rows)
    return rows
