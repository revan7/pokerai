"""Define the fifty recorded end-to-end benchmark inputs as code (plan 4 Task 18).

`records()` is the single frozen inventory: 22 supported (numeric-EV release subset), 14 special
(exceptional-but-defined inputs -- straddle mapping, multiway/projection edge cases, missing preflop
nodes, hero-out-of-support combos) and 14 fault-injection records, always in that order and always
50 rows with `id` "001".."050" assigned in inventory order.

This is a benchmark input format, translated into proto's actual wire commands by Task 21 -- it does
not replace `proto` and never depends on its JSON field layout. Task 19 writes/verifies the fifty
fixture files and a manifest from this module; nothing here writes a file.

Every record is a complete, independent, explicit v1 dict (`version`, `config`, `button`, `dealt`,
`stacks`, `hero`, `hero_cards`, `events`, `fault`, `class`, `expected`, `id`) -- no record holds a
runtime reference to another one; `deepcopy` is used wherever a record starts from another's shape
so later mutation of one can never leak into another (see `test_records_are_deterministic_and_
independent_across_calls`).

Seats are numbered BTN=0, SB=1, BB=2, UTG=3, HJ=4, CO=5 (six dealt seats, `base()`'s fixed
preflop acting order below). Hero's cards are carried on the record for hero-combo equity and
terminal calculations only -- they never enter a public range, solve input or cache key (spec §2;
CLAUDE.md §6).
"""
from __future__ import annotations

from copy import deepcopy

FAULTS = (
    "Oom", "Eof", "Malformed", "Oversized", "TreeMismatch",
    "BlockedCacheIo", "SlowAllocation", "NoIteration", "ClockJump", "SuspendResume",
)


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
    bb per seat, integer chips throughout (BB100)."""
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
        "hero": 2, "hero_cards": "8h7h", "events": events + [board(flop)], "fault": None,
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
                out.append(expect(r, "hu_flop_srp", True, ["ChartRounded"]))
    for depth, opener, flop in [(100, 0, "Kh7d2c"), (100, 5, "Jh9h6c"), (100, 0, "8s8d3c"),
                                (200, 0, "Kh7d2c"), (200, 5, "Jh9h6c"), (200, 0, "8s8d3c")]:
        r = base(depth, opener, flop)
        r["events"] += [act(2, "flop", "check"), act(opener, "flop", "bet", 275), act(2, "flop", "call"),
                        board(flop + "4d"), act(2, "turn", "check"), act(opener, "turn", "bet", 550)]
        out.append(expect(r, "hu_turn", True, ["ChartRounded", "UnconditionedPriorStreet"]))
    for index, (depth, opener, flop) in enumerate([(100, 0, "Kh7d2c"), (100, 5, "Jh9h6c"), (200, 0, "8s8d3c"),
                                                  (200, 5, "Kh7d2c"), (100, 0, "Jh9h6c"), (200, 0, "8s8d3c")]):
        r = base(depth, opener, flop)
        r["events"] += [act(2, "flop", "check"), act(opener, "flop", "bet", 275), act(2, "flop", "call"),
            board(flop + "4d"), act(2, "turn", "check"), act(opener, "turn", "check"), board(flop + "4d2s"),
            act(2, "river", "check"), act(opener, "river", "allin" if index >= 4 else "bet",
                                   depth * 100 - 525 if index >= 4 else 550)]
        out.append(expect(r, "hu_river", True, ["ChartRounded", "UnconditionedPriorStreet"]))
    for paid in (False, True):
        r = base()
        r["events"] = [act(3, "preflop", "raise", 250), act(4, "preflop", "fold"), act(5, "preflop", "fold"),
            act(0, "preflop", "call"), act(1, "preflop", "fold"), act(2, "preflop", "call"), board("Kh7d2c"),
            act(2, "flop", "bet", 50)]
        if paid:
            r["hero"] = 0
            r["events"] += [act(3, "flop", "call"), act(0, "flop", "raise", 150),
                            act(2, "flop", "raise", 250), act(3, "flop", "fold")]
        else:
            r["events"] += [act(3, "flop", "fold"), act(0, "flop", "raise", 150)]
        out.append(expect(r, "projection_admitted", True, ["ChartRounded", "MultiwayStreetRoot"]))
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
            out.append(expect(r, "straddle_mapping", False, ["ChartRounded", "StraddleMapped"]))
    pre = deepcopy(supported[20]["events"][:7])
    r = base()
    r["hero"] = 3
    r["events"] = deepcopy(pre) + [act(2, "flop", "bet", 50),
        act(3, "flop", "call"), act(0, "flop", "raise", 150), act(2, "flop", "fold")]
    out.append(expect(r, "projection_rejected", False, ["ChartRounded"], "UnsupportedHistory"))
    r = base()
    r["events"] = deepcopy(pre)
    out.append(expect(r, "multiway", False, ["ChartRounded"], "MultiwayEv"))
    r = deepcopy(r)
    r["stacks"][3] = 250
    r["events"][0] = act(3, "preflop", "allin", 250)
    out.append(expect(r, "third_allin", False, ["ChartRounded"], "MultiwayEv"))
    r = base()
    r["stacks"][3] = 300
    r["events"] = deepcopy(pre) + [
        act(2, "flop", "bet", 50), act(3, "flop", "allin", 50), act(0, "flop", "call"),
        board("Kh7d2c4d"), act(2, "turn", "bet", 100), act(0, "turn", "call"), board("Kh7d2c4d2s")]
    out.append(expect(r, "multiway_side_pot", False, ["ChartRounded"], "MultiwayEv"))
    missing = [(4, [act(3, "preflop", "call")]),
        (0, [act(3, "preflop", "fold"), act(4, "preflop", "fold"), act(5, "preflop", "call")]),
        (4, [act(3, "preflop", "fold"), act(4, "preflop", "raise", 250), act(5, "preflop", "raise", 850),
            act(0, "preflop", "fold"), act(1, "preflop", "call"), act(2, "preflop", "fold")])]
    for hero, events in missing:
        r = base()
        r["hero"] = hero
        r["hero_cards"] = "AhAd"
        r["events"] = events
        out.append(expect(r, "missing_preflop", False, ["ChartRounded"], "MissingPreflopNode"))
    for n, depth in enumerate((100, 200, 100)):
        r = base(depth)
        r["hero"] = 0
        r["hero_cards"] = "7c2d"
        r["events"] += [act(2, "flop", "check")]
        if n == 2:
            r["events"] += [act(0, "flop", "check"), board("Kh7d2c4d"), act(2, "turn", "check")]
        out.append(expect(r, "hero_out_of_support", False, ["ChartRounded"], "HeroComboOutOfSupport"))
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
        out.append(expect(r, "failure_injection", False, ["ChartRounded"], reason))
    for index, fault, budget in ((8, "Oom", 10), (14, "Eof", 10), (8, "NoIteration", 10), (0, "SlowAllocation", 10)):
        r = deepcopy(supported[index])
        r["fault"] = fault
        r["config"]["flop_budget_s"] = budget
        out.append(expect(r, "failure_injection", False, ["ChartRounded"],
                          "EngineError" if fault in ("Oom", "Eof") else "DeadlineExceeded"))
    return out


def records() -> list:
    """The frozen fifty: 22 supported + 14 special + 14 fault-injection, in that fixed order, with
    `id` "001".."050" assigned by inventory position. Every call returns fresh, independent dicts."""
    supported = supported_records()
    rows = supported + special_records(supported) + fault_records(supported)
    for i, row in enumerate(rows, 1):
        row["id"] = f"{i:03}"
    assert len(rows) == 50
    return rows
