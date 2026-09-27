"""Provenance-freeze tests for plan 4 Task 17 (`tools/chart_sources.py`).

`freeze_sources` must derive `bench/spots/sources.json` only from already-committed bytes: the
two chart bundles' envelope JSON, their `*.manifest.json` sidecars, and Plan 3 Task 4's
acquisition record (`fixtures/charts/sources.manifest.json`) -- never a fresh fetch, never a
hand-typed hash. These tests pin that against the real, committed fixtures (never a synthetic
stand-in), so a drift in any of those files fails here first, before any bench spot generator
(Task 20) could run against a stale chart.
"""
from pathlib import Path

from chart_sources import MISSING, available_depths, freeze_sources, history_key, write_sources

REPO = Path(__file__).resolve().parents[2]


def test_sources_cover_every_required_node():
    lock = freeze_sources(REPO)
    names = [b["name"] for b in lock["bundles"]]
    # depth 200 is present only when Plan 3's acquisition record marks it `available`
    assert names[0] == "pokercoaching_100"
    assert set(names) | {b["name"] for b in lock["unavailable"]} == {
        "pokercoaching_100", "rangeconverter_200"}
    assert lock["version"] == 1 and lock["snapshot_date"] == "2026-09-10"
    assert all(len(b["sha256"]) == 64 for b in lock["bundles"])
    assert all(b["reason"] for b in lock["unavailable"])

    # Unopened-fold-to-each-opener prefixes for the five seats that can actually RFI (UTG, HJ,
    # CO, BTN, SB). BB never gets a sixth "unopened" node here: if SB also folds the hand ends
    # uncontested and BB wins without a decision -- pokercoaching_100's own transcription
    # inventory records this explicitly as an `absent` row ("BB RFI (folded to the big blind)":
    # "No decision exists: when all five other seats fold the hand ends and the BB wins the
    # blinds; no page shows a BB first-in chart", fixtures/charts/transcription/
    # pokercoaching_100.json). A hypothetical sixth all-fold prefix is therefore never a covered
    # node in either bundle, so it is deliberately excluded from `required` below.
    required = {"", "F", "FF", "FFF", "FFFF"}
    for bundle in lock["bundles"]:
        histories = set(bundle["nodes"])
        assert required <= histories, sorted(required - histories)

    # the three synthetic absences are declared, not discovered later
    assert lock["missing"] == MISSING


def test_history_key_matches_the_required_fold_prefixes():
    assert history_key([]) == ""
    assert history_key([["UTG", "fold", 0]]) == "F"
    assert history_key([["UTG", "fold", 0], ["HJ", "fold", 0]]) == "FF"
    assert history_key([["UTG", "raise", 2500], ["HJ", "fold", 0]]) == "RF"
    assert history_key([
        ["UTG", "fold", 0], ["HJ", "fold", 0], ["CO", "fold", 0], ["BTN", "fold", 0],
        ["SB", "raise", 3000], ["BB", "raise", 10500],
    ]) == "FFFFRR"


def test_history_key_rejects_an_unknown_step():
    try:
        history_key([["UTG", "limp", 0]])
    except ValueError:
        pass
    else:
        raise AssertionError("history_key must reject an unrecognized step")


def test_bundle_node_keys_are_unique_within_each_bundle():
    lock = freeze_sources(REPO)
    for bundle in lock["bundles"]:
        assert len(bundle["nodes"]) == len(set(bundle["nodes"])), bundle["name"]


def test_available_depths_reads_the_acquisition_record():
    statuses = available_depths(REPO)
    assert statuses["pokercoaching_100"][0] == "available"
    assert statuses["rangeconverter_200"][0] == "available"


def test_write_sources_matches_freeze_sources_bytes(tmp_path):
    import json
    import shutil

    # write_sources always writes to <repo>/bench/spots/sources.json; exercise it against a
    # throwaway copy of the repo's relevant subtree so this test never touches the real file.
    fake_repo = tmp_path / "repo"
    for rel in ("fixtures/charts", "bench/spots"):
        shutil.copytree(REPO / rel, fake_repo / rel)
    write_sources(fake_repo)
    written = (fake_repo / "bench/spots/sources.json").read_bytes()
    expected = (json.dumps(freeze_sources(fake_repo), sort_keys=True, indent=2) + "\n").encode("utf-8")
    assert written == expected
    assert written.endswith(b"\n")


def test_lock_is_byte_deterministic_across_runs():
    import json

    first = json.dumps(freeze_sources(REPO), sort_keys=True, indent=2)
    second = json.dumps(freeze_sources(REPO), sort_keys=True, indent=2)
    assert first == second
