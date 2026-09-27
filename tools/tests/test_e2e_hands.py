"""Inventory tests for the fifty recorded end-to-end benchmark inputs (plan 4 Task 18,
`tools/e2e_hands.py`). These pin the frozen record count, class layout and per-record schema; they
never re-derive chart coverage, rake mapping or fault semantics themselves -- coverage is checked
against the already-frozen `bench/spots/sources.json` lock (Task 17), and the fault/rejection
reasons are checked against the exact strings the brief assigns, never harvested from a live run.

Task 19 writes/verifies the fifty fixture files and a manifest from `records()`, and Task 21 replays
them through the real engine -- both depend on this schema staying exactly what is asserted here.
"""
import json
from pathlib import Path

import pytest

from chart_sources import history_key
from e2e_hands import FAULTS, records

REPO = Path(__file__).resolve().parents[2]

# Classes whose only claimed coverage support is a direct, single-decision chart node (the BB's
# closing action against a single open, or the straddler's closing action against a single open) --
# these are exactly the classes for which a node-membership check against the frozen lock is
# meaningful. Multiway/projection/missing-node/out-of-support classes are deliberately excluded:
# their coverage or rejection is audited by other means (Task 17's `missing` list and absent-history
# audit), never by a plain node-membership check.
CHART_NODE_CLASSES = {"hu_flop_srp", "hu_turn", "hu_river", "straddle_mapping"}

SPECIAL_EXPECTATIONS = {
    # id: (class, coverage, unsupported reason or None)
    "023": ("straddle_mapping", "Approximate", None),
    "024": ("straddle_mapping", "Approximate", None),
    "025": ("straddle_mapping", "Approximate", None),
    "026": ("straddle_mapping", "Approximate", None),
    "027": ("projection_rejected", "Unsupported", "UnsupportedHistory"),
    "028": ("multiway", "Unsupported", "MultiwayEv"),
    "029": ("third_allin", "Unsupported", "MultiwayEv"),
    "030": ("multiway_side_pot", "Unsupported", "MultiwayEv"),
    "031": ("missing_preflop", "Unsupported", "MissingPreflopNode"),
    "032": ("missing_preflop", "Unsupported", "MissingPreflopNode"),
    "033": ("missing_preflop", "Unsupported", "MissingPreflopNode"),
    "034": ("hero_out_of_support", "Unsupported", "HeroComboOutOfSupport"),
    "035": ("hero_out_of_support", "Unsupported", "HeroComboOutOfSupport"),
    "036": ("hero_out_of_support", "Unsupported", "HeroComboOutOfSupport"),
}

FAULT_REASON = {
    "Oom": "EngineError", "Eof": "EngineError", "Malformed": "EngineError",
    "Oversized": "EngineError", "TreeMismatch": "EngineError",
    "BlockedCacheIo": "DeadlineExceeded", "SlowAllocation": "DeadlineExceeded",
    "NoIteration": "DeadlineExceeded", "ClockJump": "DeadlineExceeded",
    "SuspendResume": "DeadlineExceeded",
}

REQUIRED_KEYS = {
    "version", "config", "button", "dealt", "stacks", "hero", "hero_cards",
    "events", "fault", "class", "expected", "id",
}


def test_fifty_frozen_inputs():
    rows = records()
    assert len(rows) == 50
    assert [r["id"] for r in rows] == [f"{n:03}" for n in range(1, 51)]
    assert sum(r["expected"]["numeric_ev"] for r in rows) == 22
    assert [r["class"] for r in rows[:8]] == ["hu_flop_srp"] * 8
    assert [r["class"] for r in rows[8:14]] == ["hu_turn"] * 6
    assert [r["class"] for r in rows[14:20]] == ["hu_river"] * 6
    assert [r["class"] for r in rows[20:22]] == ["projection_admitted"] * 2
    assert all(r["version"] == 1 and r["events"] for r in rows)


def test_fault_records_are_the_last_fourteen():
    rows = records()
    assert [r["class"] for r in rows[36:]] == ["failure_injection"] * 14
    assert [r["fault"] for r in rows[36:46]] == list(FAULTS)


def test_every_record_has_the_full_v1_schema():
    for r in records():
        assert set(r.keys()) == REQUIRED_KEYS, r["id"]
        assert r["version"] == 1
        assert len(r["stacks"]) == len(r["dealt"]) == 6
        assert all(isinstance(s, int) for s in r["stacks"]), r["id"]
        assert isinstance(r["button"], int)
        assert r["config"]["bb_chips"] > 0 and r["config"]["sb_chips"] > 0
        assert isinstance(r["config"]["rake"]["cap_mchips"], int)


def _bundle_nodes_by_depth() -> dict:
    """The frozen chart lock's covered-node inventory (Task 17), keyed by `depth_bb`. Read only --
    never re-derives coverage; a record either names a history the lock already lists as covered,
    or it does not."""
    lock = json.loads((REPO / "bench/spots/sources.json").read_text(encoding="utf-8"))
    return {b["manifest"]["depth_bb"]: set(b["nodes"]) for b in lock["bundles"]}


def _preflop_prefix_key(record: dict) -> str:
    """The chart-node history key (`chart_sources.history_key`, the same function that built the
    frozen lock) for the decision a chart-covered record's *last* preflop action represents: the
    F/C/R/A letters of every preflop action before it, in acting order. The final action itself is
    the decision being looked up, not part of the node it is looked up against."""
    preflop = [
        (None, e["action"]["kind"], None)
        for e in record["events"]
        if e["type"] == "action" and e["street"] == "preflop"
    ]
    return history_key(preflop[:-1])


def test_chart_covered_records_reference_a_node_the_frozen_lock_covers():
    nodes_by_depth = _bundle_nodes_by_depth()
    rows = records()
    checked = 0
    for r in rows:
        if r["class"] not in CHART_NODE_CLASSES:
            continue
        assert "ChartRounded" in r["expected"]["required_reasons"], r["id"]
        depth_bb = r["stacks"][0] // r["config"]["bb_chips"]
        key = _preflop_prefix_key(r)
        assert key in nodes_by_depth[depth_bb], (r["id"], r["class"], depth_bb, key)
        checked += 1
    # 8 hu_flop_srp + 6 hu_turn + 6 hu_river + 4 straddle_mapping
    assert checked == 24


def test_special_records_name_their_exact_coverage_label_or_rejection_reason():
    rows = {r["id"]: r for r in records()}
    assert set(SPECIAL_EXPECTATIONS) == {f"{n:03}" for n in range(23, 37)}
    for id_, (cls, coverage, unsupported) in SPECIAL_EXPECTATIONS.items():
        r = rows[id_]
        assert r["class"] == cls, id_
        assert r["expected"]["coverage"] == coverage, id_
        assert r["expected"]["unsupported"] == unsupported, id_
        assert r["expected"]["numeric_ev"] is False, id_


def test_fault_records_name_their_injected_fault_and_reason():
    rows = records()
    fault_rows = rows[36:]
    assert len(fault_rows) == 14
    for r in fault_rows:
        assert r["class"] == "failure_injection"
        assert r["fault"] in FAULTS, r["id"]
        assert r["expected"]["coverage"] == "Unsupported", r["id"]
        assert r["expected"]["unsupported"] == FAULT_REASON[r["fault"]], r["id"]
        assert r["expected"]["required_reasons"] == ["ChartRounded"], r["id"]
    # the four budget-variant records (047-050) reuse an existing supported record's events and
    # additionally pin flop_budget_s to the fault runner's default budget
    for r in fault_rows[10:]:
        assert r["config"]["flop_budget_s"] == 10, r["id"]


def test_records_are_deterministic_and_independent_across_calls():
    first = records()
    second = records()
    assert first == second
    # mutating one call's result must never affect another (no shared nested state)
    first[0]["events"].append({"type": "action", "seat": 0, "street": "flop", "action": {"kind": "fold"}})
    assert second[0]["events"] != first[0]["events"]
