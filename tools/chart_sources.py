"""Freeze chart provenance, source hashes and the covered node inventory (plan 4 Task 17).

Reads the two committed chart bundles Plan 3 produced (`fixtures/charts/<name>.json` and
`<name>.manifest.json`) plus Plan 3 Task 4's acquisition record
(`fixtures/charts/sources.manifest.json`) and freezes them, byte-exact, into
`bench/spots/sources.json`. Task 20's spot generators and `engine::bench_support::
verify_source_lock` read that frozen lock before generating or trusting any bench spot, so a
chart that drifted after this freeze -- a re-transcription, a corrected cell, a different
source PDF -- is caught as a hash mismatch instead of silently feeding a stale chart into a
benchmark.

This module never fetches anything and never invents a value: every byte hashed here is already
committed (`chart_ingest.fetch` is the only network-capable surface in this tool family, and it
is not called from here). A depth whose acquisition record marks it anything other than
`"available"` (Plan 3's conditional-depth-200 rule) is recorded in `lock['unavailable']` with
the acquisition record's own stated reason -- never substituted with another publisher, another
depth, or a screenshot.

Task-17 fix round (I2/I3): generation is bound to the committed `#### Freeze: <bundle_id>` blocks
in `docs/data/chart-transcription.md`. Before any bundle is frozen, its Freeze block is parsed and
cross-checked against the acquisition record's audited source-PDF hash, the envelope sidecar
manifest's hash, the actual envelope bytes, and the transcription sidecar's own audited absent-row
inventory -- in every direction. A missing block or any mismatch raises `ValueError` rather than
silently freezing a drifted or unaudited bundle. The lock also now carries the audited source PDF
identity (URL/hash/byte count from the acquisition record, not the `SOURCES` constants) separately
from the envelope identity, and the audited absent-history inventory separately from both the
covered `nodes` list and the synthetic cross-bundle `missing` list.
"""
from __future__ import annotations

import hashlib
import json
import re
from pathlib import Path

# Matches `#### Freeze: <bundle_id>` immediately followed by a fenced ```json block (the same
# machine-readable shape `tools/tests/test_chart_ingest.py`'s frozen-checklist tests already parse
# and pin against the fixtures -- reimplemented here, in production code, because Task 17's
# generator must enforce the same binding at generation time, not only in Plan 3's tests).
_FREEZE_BLOCK_RE = re.compile(r"#### Freeze: (\S+)\n```json\n(.*?)\n```", re.DOTALL)

# The date this freeze was taken (task-17 brief); bumping it is a deliberate re-freeze, not an
# automatic "today" stamp, so the lock stays reproducible byte-for-byte across re-runs.
SNAPSHOT_DATE = "2026-09-10"

# Source acquisition metadata: the final resolved URL, page count and the fixed transcription
# envelope version (2) Plan 3 used for both bundles. Never an invented publisher "release" name
# -- the source's real version is its content sha256 (below) plus this envelope version.
SOURCES = {
    "pokercoaching_100": {
        "url": "https://poker-coaching.s3.amazonaws.com/tools/preflop-charts/online-6max-gto-charts.pdf",
        "pages": 6,
        "envelope_version": 2,
    },
    "rangeconverter_200": {
        "url": "https://rangeconverter.com/articles/poker-charts-6-max-200bb-no-limit-texas-holdem",
        "download": "https://rangeconverter.com/downloads/6-max-200bb-Poker-Charts-No-Limit-Texas-Holdem-Cash",
        "pages": 13,
        "envelope_version": 2,
    },
}

# The three synthetic fixture-only nodes no bundle covers (task-17 brief): neither chart is a
# raise-or-fold table that ever charts a limp for UTG or CO (both transcriptions' `absent`
# inventories record exactly this -- "no position other than the SB has a limp option" /
# "no grid covers it" -- for the equivalent UTG-limp row), and the 100bb PokerCoaching bundle
# has no verified complete versus-3-bet chart, so a BB cold-call response to a 3bet is absent
# too. Declared once, here, rather than discovered later by a missing-node crash deep inside a
# spot generator.
MISSING = ["UTG-limp", "CO-limp", "BB-cold-call-vs-3bet"]

# One letter per action step (spec/`core_preflop::store`'s history-step vocabulary), concatenated
# in history order. The fixed 6-max seating order (UTG, HJ, CO, BTN, SB, BB) already determines
# which seat is acting at each position of a preflop-only history -- no street changes, no
# skipped seats, and a re-visited seat (e.g. SB facing a 4bet) still appears at its own, distinct
# depth -- so neither the seat name nor the raise amount adds information the letter sequence
# does not already carry. Verified against both committed bundles: every one of
# pokercoaching_100's 22 and rangeconverter_200's 35 node histories produces a distinct key
# (`test_bundle_node_keys_are_unique_within_each_bundle`). This also directly matches the
# "unopened folds to each opener" shorthand ('', 'F', 'FF', 'FFF', 'FFFF') since a fold's amount
# is always 0 anyway.
_STEP_LETTER = {"fold": "F", "check": "X", "call": "C", "raise": "R", "allin": "A"}


def history_key(history: list) -> str:
    """The canonical, hashable node-inventory key for one node's `history` list of
    `(position, step, amount)` triples (see `_STEP_LETTER` above for why position and amount are
    safely dropped). Raises `ValueError` on an unrecognized step rather than silently omitting a
    node from the inventory -- a chart step vocabulary change must be reflected in
    `_STEP_LETTER` before its nodes can be frozen.
    """
    letters = []
    for entry in history:
        _, step, _ = entry
        if step not in _STEP_LETTER:
            raise ValueError(f"unrecognized history step {step!r} in history {history!r}")
        letters.append(_STEP_LETTER[step])
    return "".join(letters)


def _acquisition_record(repo: Path) -> dict:
    """Plan 3 Task 4's full acquisition record, parsed once. `available_depths` below only
    exposes the `(status, note)` projection of this that Task 17's public interface promises;
    `freeze_sources` needs the full rows (final URL, hash, byte count, source file) to freeze the
    audited source PDF identity (I2)."""
    return json.loads((repo / "fixtures/charts/sources.manifest.json").read_text(encoding="utf-8"))


def available_depths(repo: Path) -> dict:
    """Plan 3 Task 4's acquisition record: `{bundle_id: (status, note)}` for every depth row in
    `fixtures/charts/sources.manifest.json`. Never re-derives status from anything else."""
    record = _acquisition_record(repo)
    return {row["bundle_id"]: (row["status"], row.get("note", "")) for row in record["depths"]}


def _freeze_blocks(repo: Path) -> dict:
    """Parses every `#### Freeze: <bundle_id>` heading + fenced ```json block from
    `docs/data/chart-transcription.md`. Raises if the document has no freeze block at all --
    generation never proceeds unaudited."""
    text = (repo / "docs/data/chart-transcription.md").read_text(encoding="utf-8")
    blocks = {m.group(1): json.loads(m.group(2)) for m in _FREEZE_BLOCK_RE.finditer(text)}
    if not blocks:
        raise ValueError(
            "no '#### Freeze: <bundle_id>' block found in docs/data/chart-transcription.md")
    return blocks


def _reject_unopened_bb_node(name: str, node_keys: list) -> None:
    """Domain assertion (task-17 fix round I3): if all five non-BB seats fold, the hand ends
    uncontested and BB wins without acting -- pokercoaching_100's own transcription inventory
    records this explicitly as an `absent` row ("No decision exists..."). A 5-fold-only history
    (`FFFFF`) must therefore never appear as a covered chart node in either bundle."""
    if "FFFFF" in node_keys:
        raise ValueError(
            f"{name}: FFFFF (all five non-BB seats folding) must never be a covered chart node "
            "-- BB has no unopened decision")


def _verify_against_freeze_block(repo: Path, name: str, row: dict, manifest: dict, envelope: dict,
                                  envelope_hash: str, freeze_blocks: dict) -> list:
    """Binds generation to the committed Freeze block for `name` (task-17 fix round I3): validates
    it against the acquisition row's audited PDF hash (I2), the envelope sidecar manifest's hash,
    the actual envelope bytes' own hash, and the envelope's node histories, then cross-checks the
    block's audited absent histories against the transcription sidecar's `absent` inventory rows.
    Returns the audited absent-history keys (letter-encoded, `history_key`) so the caller can
    freeze them separately from covered `nodes` and the synthetic `missing` list. Raises
    `ValueError` on any mismatch or a missing block -- generation never proceeds past an
    unaudited or drifted bundle.
    """
    frozen = freeze_blocks.get(name)
    if frozen is None:
        raise ValueError(
            f"{name}: no '#### Freeze: {name}' block in docs/data/chart-transcription.md -- "
            "generation refuses to run against an unaudited bundle")
    if frozen["bundle_id"] != name:
        raise ValueError(f"{name}: Freeze block bundle_id {frozen['bundle_id']!r} != {name!r}")
    if frozen["depth_bb"] != row.get("depth_bb"):
        raise ValueError(
            f"{name}: Freeze block depth_bb {frozen['depth_bb']!r} disagrees with "
            f"sources.manifest.json's {row.get('depth_bb')!r}")
    if frozen["source_sha256"] != row["sha256"]:
        raise ValueError(
            f"{name}: Freeze block source_sha256 {frozen['source_sha256']!r} disagrees with "
            f"sources.manifest.json's audited PDF hash {row['sha256']!r}")
    if manifest["sha256"] != envelope_hash:
        raise ValueError(
            f"{name}: {name}.manifest.json's sha256 {manifest['sha256']!r} does not match the "
            f"actual envelope bytes ({envelope_hash!r})")
    if frozen["envelope_sha256"] != manifest["sha256"]:
        raise ValueError(
            f"{name}: Freeze block envelope_sha256 {frozen['envelope_sha256']!r} disagrees with "
            f"{name}.manifest.json's sha256 {manifest['sha256']!r}")

    actual_covered = sorted(json.dumps(n["history"]) for n in envelope["nodes"])
    frozen_covered = sorted(json.dumps(h) for h in frozen["covered_histories"])
    if actual_covered != frozen_covered:
        raise ValueError(
            f"{name}: the committed envelope's node histories drifted from the Freeze block's "
            "covered_histories")

    transcription = json.loads(
        (repo / "fixtures/charts/transcription" / f"{name}.json").read_text(encoding="utf-8"))
    actual_absent = {json.dumps(r["history"]) for r in transcription["inventory"] if r["status"] == "absent"}
    frozen_absent = {json.dumps(h) for h in frozen["absent_histories"]}
    if actual_absent != frozen_absent:
        raise ValueError(
            f"{name}: the transcription's audited absent histories drifted from the Freeze "
            "block's absent_histories")
    if frozen_absent & set(frozen_covered):
        raise ValueError(f"{name}: a Freeze block history is claimed both covered and absent")

    return [history_key(json.loads(h)) for h in sorted(frozen_absent)]


def freeze_sources(repo: Path) -> dict:
    """Builds the in-memory lock dict `write_sources` serializes. Bundle order follows `SOURCES`'
    declaration order (`pokercoaching_100` first), which is deterministic and matches the order
    the committed `sources.json` is written in.
    """
    record = _acquisition_record(repo)
    rows = {row["bundle_id"]: row for row in record["depths"]}
    freeze_blocks = _freeze_blocks(repo)
    result: dict = {
        "version": 1,
        "snapshot_date": SNAPSHOT_DATE,
        "missing": list(MISSING),
        "bundles": [],
        "unavailable": [],
    }
    for name, meta in SOURCES.items():
        row = rows.get(name)
        if row is None or row["status"] != "available":
            reason = (row.get("note") or row["status"]) if row else "absent from sources.manifest.json"
            result["unavailable"].append({"name": name, "reason": reason})
            continue
        raw = (repo / "fixtures/charts" / f"{name}.json").read_bytes()
        manifest = json.loads((repo / "fixtures/charts" / f"{name}.manifest.json").read_text(encoding="utf-8"))
        envelope = json.loads(raw.decode("utf-8"))
        envelope_hash = hashlib.sha256(raw).hexdigest()
        absent = _verify_against_freeze_block(repo, name, row, manifest, envelope, envelope_hash, freeze_blocks)
        node_keys = [history_key(n["history"]) for n in envelope["nodes"]]
        _reject_unopened_bb_node(name, node_keys)
        result["bundles"].append({
            "name": name,
            "sha256": envelope_hash,
            "bytes": len(raw),
            "source": meta,
            "source_pdf": {
                "url": row["final_url"],
                "sha256": row["sha256"],
                "bytes": row["bytes"],
                "source_file": row["source_file"],
            },
            "manifest": manifest,
            "nodes": node_keys,
            "absent": absent,
        })
    return result


def write_sources(repo: Path) -> None:
    """Writes `bench/spots/sources.json`, byte-exact and deterministic (`sort_keys=True`,
    `indent=2`, LF-only, trailing newline) so `gen_fixtures.py sources --check` can compare bytes
    exactly rather than re-parsing and re-comparing structurally.
    """
    raw = (json.dumps(freeze_sources(repo), sort_keys=True, indent=2) + "\n").encode("utf-8")
    (repo / "bench/spots/sources.json").write_bytes(raw)
