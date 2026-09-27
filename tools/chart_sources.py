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
"""
from __future__ import annotations

import hashlib
import json
from pathlib import Path

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


def available_depths(repo: Path) -> dict:
    """Plan 3 Task 4's acquisition record: `{bundle_id: (status, note)}` for every depth row in
    `fixtures/charts/sources.manifest.json`. Never re-derives status from anything else."""
    record = json.loads((repo / "fixtures/charts/sources.manifest.json").read_text(encoding="utf-8"))
    return {row["bundle_id"]: (row["status"], row.get("note", "")) for row in record["depths"]}


def freeze_sources(repo: Path) -> dict:
    """Builds the in-memory lock dict `write_sources` serializes. Bundle order follows `SOURCES`'
    declaration order (`pokercoaching_100` first), which is deterministic and matches the order
    the committed `sources.json` is written in.
    """
    statuses = available_depths(repo)
    result: dict = {
        "version": 1,
        "snapshot_date": SNAPSHOT_DATE,
        "missing": list(MISSING),
        "bundles": [],
        "unavailable": [],
    }
    for name, meta in SOURCES.items():
        status, note = statuses.get(name, ("unsupported", "absent from sources.manifest.json"))
        if status != "available":
            result["unavailable"].append({"name": name, "reason": note or status})
            continue
        raw = (repo / "fixtures/charts" / f"{name}.json").read_bytes()
        manifest = json.loads((repo / "fixtures/charts" / f"{name}.manifest.json").read_text(encoding="utf-8"))
        envelope = json.loads(raw.decode("utf-8"))
        result["bundles"].append({
            "name": name,
            "sha256": hashlib.sha256(raw).hexdigest(),
            "bytes": len(raw),
            "source": meta,
            "manifest": manifest,
            "nodes": [history_key(n["history"]) for n in envelope["nodes"]],
        })
    return result


def write_sources(repo: Path) -> None:
    """Writes `bench/spots/sources.json`, byte-exact and deterministic (`sort_keys=True`,
    `indent=2`, LF-only, trailing newline) so `gen_fixtures.py sources --check` can compare bytes
    exactly rather than re-parsing and re-comparing structurally.
    """
    raw = (json.dumps(freeze_sources(repo), sort_keys=True, indent=2) + "\n").encode("utf-8")
    (repo / "bench/spots/sources.json").write_bytes(raw)
