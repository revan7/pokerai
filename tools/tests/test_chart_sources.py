"""Provenance-freeze tests for plan 4 Task 17 (`tools/chart_sources.py`).

`freeze_sources` must derive `bench/spots/sources.json` only from already-committed bytes: the
two chart bundles' envelope JSON, their `*.manifest.json` sidecars, Plan 3 Task 4's acquisition
record (`fixtures/charts/sources.manifest.json`), and (fix round, I2/I3) the committed
`#### Freeze: <bundle_id>` blocks in `docs/data/chart-transcription.md` -- never a fresh fetch,
never a hand-typed hash. These tests pin that against the real, committed fixtures (never a
synthetic stand-in for the passing-case assertions), so a drift in any of those files fails here
first, before any bench spot generator (Task 20) could run against a stale chart. The rejection
tests below build a small, isolated copy of just the files `freeze_sources` reads (never the real
committed files) and corrupt exactly one fact at a time, proving the comparison itself actually
rejects a mismatch rather than merely that nothing happens to disagree today.
"""
import json
import re
import shutil
from pathlib import Path

import pytest

from chart_sources import (
    MISSING,
    _FREEZE_BLOCK_RE,
    _reject_unopened_bb_node,
    available_depths,
    freeze_sources,
    history_key,
    write_sources,
)

REPO = Path(__file__).resolve().parents[2]

# The files `freeze_sources` actually reads, relative to the repo root -- copied into an isolated
# temp repo for the rejection tests below (never the whole 7.6MB `fixtures/charts` tree, which
# also carries the large source PDFs freeze_sources never opens).
_REQUIRED_REL_PATHS = [
    "fixtures/charts/sources.manifest.json",
    "fixtures/charts/pokercoaching_100.json",
    "fixtures/charts/pokercoaching_100.manifest.json",
    "fixtures/charts/rangeconverter_200.json",
    "fixtures/charts/rangeconverter_200.manifest.json",
    "fixtures/charts/transcription/pokercoaching_100.json",
    "fixtures/charts/transcription/rangeconverter_200.json",
    "docs/data/chart-transcription.md",
]


def _temp_repo(tmp_path: Path) -> Path:
    fake_repo = tmp_path / "repo"
    for rel in _REQUIRED_REL_PATHS:
        dst = fake_repo / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_bytes((REPO / rel).read_bytes())
    return fake_repo


def _replace_freeze_block(doc_path: Path, bundle_id: str, **overrides) -> None:
    """Loads the Freeze block for `bundle_id` out of `doc_path`, merges `overrides` into it, and
    rewrites just that fenced block in place -- used to corrupt exactly one recorded fact
    (envelope_sha256, source_sha256, ...) without touching anything else in the document."""
    text = doc_path.read_text(encoding="utf-8")
    match = next(m for m in _FREEZE_BLOCK_RE.finditer(text) if m.group(1) == bundle_id)
    frozen = json.loads(match.group(2))
    frozen.update(overrides)
    start, end = match.span(2)
    doc_path.write_text(text[:start] + json.dumps(frozen, indent=2) + text[end:], encoding="utf-8")


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
    # write_sources always writes to <repo>/bench/spots/sources.json; exercise it against a
    # throwaway copy of the repo's relevant subtree so this test never touches the real file.
    fake_repo = tmp_path / "repo"
    for rel in ("fixtures/charts", "docs/data", "bench/spots"):
        shutil.copytree(REPO / rel, fake_repo / rel)
    write_sources(fake_repo)
    written = (fake_repo / "bench/spots/sources.json").read_bytes()
    expected = (json.dumps(freeze_sources(fake_repo), sort_keys=True, indent=2) + "\n").encode("utf-8")
    assert written == expected
    assert written.endswith(b"\n")


def test_lock_is_byte_deterministic_across_runs():
    first = json.dumps(freeze_sources(REPO), sort_keys=True, indent=2)
    second = json.dumps(freeze_sources(REPO), sort_keys=True, indent=2)
    assert first == second


def test_committed_lock_matches_the_generator():
    """A pytest-native equivalent of `gen_fixtures.py sources --check` (task-17 review's
    verification blocker: the review's environment could not invoke `python` at all to run that
    CLI check). Runs unconditionally as part of the normal `pytest tools` gate, so a drifted
    committed `bench/spots/sources.json` fails the gate itself, not only an optional CLI step."""
    committed = (REPO / "bench/spots/sources.json").read_bytes()
    expected = (json.dumps(freeze_sources(REPO), sort_keys=True, indent=2) + "\n").encode("utf-8")
    assert committed == expected


# --- I2: the frozen lock carries the audited source PDF identity, separately from the envelope
# identity, sourced from the acquisition record's full rows rather than the SOURCES constants ---


def test_freeze_sources_freezes_the_audited_source_pdf_identity():
    record = json.loads((REPO / "fixtures/charts/sources.manifest.json").read_text(encoding="utf-8"))
    rows = {row["bundle_id"]: row for row in record["depths"]}
    lock = freeze_sources(REPO)
    for bundle in lock["bundles"]:
        row = rows[bundle["name"]]
        assert bundle["source_pdf"] == {
            "url": row["final_url"],
            "sha256": row["sha256"],
            "bytes": row["bytes"],
            "source_file": row["source_file"],
        }
        # the source PDF hash must never collide with the envelope's own (transcription) hash --
        # these are two distinct artifacts and I2 exists precisely because the old lock conflated
        # them.
        assert bundle["source_pdf"]["sha256"] != bundle["sha256"]


def test_freeze_sources_rejects_a_freeze_block_source_hash_mismatch(tmp_path):
    fake_repo = _temp_repo(tmp_path)
    manifest_path = fake_repo / "fixtures/charts/sources.manifest.json"
    record = json.loads(manifest_path.read_text(encoding="utf-8"))
    for row in record["depths"]:
        if row["bundle_id"] == "pokercoaching_100":
            row["sha256"] = "0" * 64
    manifest_path.write_text(json.dumps(record), encoding="utf-8")

    with pytest.raises(ValueError, match="source_sha256"):
        freeze_sources(fake_repo)


def test_freeze_sources_rejects_an_envelope_manifest_hash_mismatch(tmp_path):
    fake_repo = _temp_repo(tmp_path)
    manifest_path = fake_repo / "fixtures/charts/pokercoaching_100.manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["sha256"] = "0" * 64
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

    with pytest.raises(ValueError, match="manifest.json's sha256"):
        freeze_sources(fake_repo)


# --- I3: generation is bound to the committed Freeze blocks; audited absences are carried in the
# lock, separately from covered nodes and the synthetic `missing` list ---


def test_freeze_sources_rejects_a_missing_freeze_block(tmp_path):
    fake_repo = _temp_repo(tmp_path)
    doc_path = fake_repo / "docs/data/chart-transcription.md"
    text = doc_path.read_text(encoding="utf-8")
    doc_path.write_text(text.replace("#### Freeze: pokercoaching_100", "#### NotAFreeze: pokercoaching_100"),
                         encoding="utf-8")

    with pytest.raises(ValueError, match="no '#### Freeze: pokercoaching_100' block"):
        freeze_sources(fake_repo)


def test_freeze_sources_rejects_a_freeze_block_bound_to_the_wrong_depth(tmp_path):
    fake_repo = _temp_repo(tmp_path)
    _replace_freeze_block(fake_repo / "docs/data/chart-transcription.md", "pokercoaching_100", depth_bb=200)

    with pytest.raises(ValueError, match="depth_bb"):
        freeze_sources(fake_repo)


def test_freeze_sources_rejects_an_absent_history_promoted_to_a_covered_node(tmp_path):
    """A history the transcription audit recorded as `absent` (never a real chart decision) must
    never silently become a real covered node without the Freeze block being updated to match --
    this is the "audited absences are recorded, not dropped" invariant the fix-round ruling names,
    tested from the promotion side rather than only the omission side."""
    fake_repo = _temp_repo(tmp_path)
    env_path = fake_repo / "fixtures/charts/pokercoaching_100.json"
    envelope = json.loads(env_path.read_text(encoding="utf-8"))
    promoted_history = [
        ["UTG", "fold", 0], ["HJ", "fold", 0], ["CO", "fold", 0], ["BTN", "fold", 0], ["SB", "fold", 0],
    ]
    envelope["nodes"].append({"history": promoted_history, "actor": None, "actions": [], "weights": []})
    new_raw = json.dumps(envelope).encode("utf-8")
    env_path.write_bytes(new_raw)
    import hashlib
    new_hash = hashlib.sha256(new_raw).hexdigest()

    # keep the manifest sidecar and the Freeze block's envelope_sha256 consistent with the new
    # bytes, so the *only* thing that disagrees is covered_histories -- isolating exactly the
    # invariant this test targets, not an incidental hash mismatch.
    manifest_path = fake_repo / "fixtures/charts/pokercoaching_100.manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    manifest["sha256"] = new_hash
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    _replace_freeze_block(fake_repo / "docs/data/chart-transcription.md", "pokercoaching_100",
                           envelope_sha256=new_hash)

    with pytest.raises(ValueError, match="covered_histories"):
        freeze_sources(fake_repo)


def test_freeze_sources_preserves_audited_absent_histories_without_inventing_them():
    """Preserve PokerCoaching's data-derived `FFFFF` absence (its Freeze block records it as an
    audited absent history) but never invent a matching RangeConverter absence -- its Freeze block
    never mentions `FFFFF` at all, covered or absent, and I3 forbids fabricating that row."""
    lock = freeze_sources(REPO)
    by_name = {b["name"]: b for b in lock["bundles"]}
    assert "FFFFF" in by_name["pokercoaching_100"]["absent"]
    assert "FFFFF" not in by_name["rangeconverter_200"]["absent"]
    # the audited absent list is disjoint from the synthetic cross-bundle `missing` list and from
    # the bundle's own covered `nodes` -- three genuinely distinct concepts, never merged.
    for bundle in lock["bundles"]:
        assert not (set(bundle["absent"]) & set(bundle["nodes"]))
        assert not (set(bundle["absent"]) & set(lock["missing"]))


def test_reject_unopened_bb_node_rejects_fffff_as_covered():
    with pytest.raises(ValueError, match="FFFFF"):
        _reject_unopened_bb_node("pokercoaching_100", ["", "F", "FF", "FFF", "FFFF", "FFFFF"])
    _reject_unopened_bb_node("pokercoaching_100", ["", "F", "FF", "FFF", "FFFF"])  # does not raise
