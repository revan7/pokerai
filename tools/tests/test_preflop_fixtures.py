"""Tests for `gen_preflop_fixtures`: the committed `fixtures/preflop/synthetic_v2/*.json`
files must exist (standing ruling: a test must fail when a required committed artifact is
missing, never skip), regenerating them must reproduce the committed bytes exactly
(determinism), and the dense (`nodes.json`) and provider-shaped sparse (`node.json`/
`range.json`) forms must agree on the one invariant they share: class 14 (KK) carries EV
2.31 at weight 0.
"""
import json
from pathlib import Path

import gen_preflop_fixtures as gpf

FIXTURES = gpf.ROOT / "fixtures" / "preflop" / "synthetic_v2"
FILES = ["manifest.json", "nodes.json", "node.json", "range.json", "spots.json", "cases.json"]


def test_committed_fixtures_exist():
    for name in FILES:
        path = FIXTURES / name
        assert path.is_file(), f"required committed fixture is missing: {path}"


def test_regeneration_reproduces_committed_bytes_exactly(tmp_path: Path):
    gpf.generate(tmp_path)
    for name in FILES:
        committed = (FIXTURES / name).read_bytes()
        regenerated = (tmp_path / name).read_bytes()
        assert regenerated == committed, f"{name} is not byte-identical on regeneration"


def test_generation_is_deterministic_across_two_runs(tmp_path: Path):
    out_a = tmp_path / "a"
    out_b = tmp_path / "b"
    gpf.generate(out_a)
    gpf.generate(out_b)
    for name in FILES:
        assert (out_a / name).read_bytes() == (out_b / name).read_bytes()


def test_manifest_sha256_matches_committed_nodes_json():
    manifest = json.loads((FIXTURES / "manifest.json").read_text(encoding="utf-8"))
    raw = (FIXTURES / "nodes.json").read_bytes()
    import hashlib

    assert manifest["sha256"] == hashlib.sha256(raw).hexdigest()


def test_kk_zero_weight_ev_shared_invariant_dense_and_sparse():
    """Class 14 (KK) carries EV 2.31 at weight 0 in both the dense envelope
    (`nodes.json`'s "HJ vs UTG RFI" node, action index 1 = call) and the provider-shaped
    sparse siblings `node.json`/`range.json`."""
    nodes = json.loads((FIXTURES / "nodes.json").read_text(encoding="utf-8"))
    hj_vs_utg_rfi = nodes["nodes"][1]
    assert hj_vs_utg_rfi["actor"] == "HJ"
    assert hj_vs_utg_rfi["weights"][1][14] == 0.0
    assert hj_vs_utg_rfi["evs"][1][14] == 2.31

    node_json = json.loads((FIXTURES / "node.json").read_text(encoding="utf-8"))
    call_action = next(a for a in node_json["actions"] if a["step"] == "call")
    assert call_action["weights"]["KK"] == 0.0
    assert call_action["evs"]["KK"] == 2.31

    range_json = json.loads((FIXTURES / "range.json").read_text(encoding="utf-8"))
    assert range_json["weights"]["KK"] == 0
    assert range_json["evs"]["KK"] == 2.31


def test_nodes_json_has_seven_nodes_and_class_168_unreachable():
    nodes = json.loads((FIXTURES / "nodes.json").read_text(encoding="utf-8"))
    assert len(nodes["nodes"]) == 7
    for node in nodes["nodes"]:
        assert node["unreachable_classes"] == [168]
        assert all(row[168] == 0.0 for row in node["weights"])


def test_cases_json_records_posts_and_absences():
    cases = json.loads((FIXTURES / "cases.json").read_text(encoding="utf-8"))
    assert cases["sb_committed_sb"] == 1
    assert cases["bb_committed_sb"] == 2
    assert cases["source_stack_sb"] == 200
    assert len(cases["ev_reference_variants"]) == 4
    assert len(cases["absences"]) == 6


def test_spots_json_lists_all_eight_table_paths():
    spots = json.loads((FIXTURES / "spots.json").read_text(encoding="utf-8"))["spots"]
    assert len(spots) == 8
    assert sum(1 for s in spots if s["node"]) == 7
