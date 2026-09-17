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
"""
from __future__ import annotations

import hashlib
import json
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


def main() -> None:
    generate(OUT_DIR)
    print(f"wrote 6 synthetic preflop fixtures to {OUT_DIR}")


if __name__ == "__main__":
    main()
