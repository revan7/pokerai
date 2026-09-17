"""Tests for `chart_ingest` (P3.T3): the deterministic chart ingestion and validation tool.

Covers `class_names`/`build`/`validate` as pure functions, `manifest_for_chart`'s exact
`BundleInfo` field parity with the Rust struct (Task 1), the CLI's four subcommands
(`fetch`/`build`/`validate`/`verify`), and every structural/numeric rule `validate` enforces
-- each exercised by an explicit malformed-row case so the rule fails without the check, per
the standing ruling that a test must fail when its guarded artifact/behavior is missing.

`fetch` is tested against a monkeypatched `chart_ingest.urlopen`, never a real network call
(this task ships the tool only; Task 4 performs the actual acquisition).
"""
from __future__ import annotations

import copy
import json
import re
from pathlib import Path

import pytest

import chart_ingest
from chart_ingest import (
    BUNDLE_INFO_FIELDS,
    bb_to_x1000,
    build,
    class_names,
    extract_links,
    find_anchor,
    manifest_for_chart,
    validate,
)

ROOT = Path(__file__).resolve().parents[2]


# --- exact brief scenario (Step 1) ---


def test_class_order_and_sibling_sum():
    assert class_names()[:3] == ["AA", "AKs", "AQs"]
    assert class_names()[13:15] == ["AKo", "KK"]
    t = {
        "bundle_id": "test",
        "depth_bb": 100,
        "rake_profile": "undocumented",
        "straddle": False,
        "nodes": [
            {
                "history": [],
                "actor": "UTG",
                "page": 2,
                "title": "UTG RFI",
                "actions": [{"step": "fold"}, {"step": "raise", "to_bb_x1000": 2500}],
                "legend": {"F": [1, 0], "R": [0, 1], "M": [0.5, 0.5]},
                "rows": [["F"] * 13 for _ in range(13)],
            }
        ],
    }
    t["nodes"][0]["rows"][0][0] = "M"
    e = build(t)
    assert e["nodes"][0]["weights"][0][0] == 0.5
    assert "evs" not in e["nodes"][0]
    validate(e)
    e["nodes"][0]["weights"][0][0] = 0.1
    with pytest.raises(ValueError, match="sum"):
        validate(e)


# --- class_names ---


def test_class_names_has_169_unique_entries():
    names = class_names()
    assert len(names) == 169
    assert len(set(names)) == 169


def test_class_names_last_entry_is_the_lowest_pair():
    assert class_names()[168] == "22"


# --- build: grid/legend structural errors ---


def _transcription_node(**overrides):
    node = {
        "history": [],
        "actor": "UTG",
        "page": 2,
        "title": "UTG RFI",
        "actions": [{"step": "fold"}, {"step": "raise", "to_bb_x1000": 2500}],
        "legend": {"F": [1, 0], "R": [0, 1]},
        "rows": [["F"] * 13 for _ in range(13)],
    }
    node.update(overrides)
    return node


def _transcription(**node_overrides):
    return {
        "bundle_id": "test",
        "depth_bb": 100,
        "rake_profile": "undocumented",
        "straddle": False,
        "license_note": "test fixture",
        "nodes": [_transcription_node(**node_overrides)],
    }


def test_build_rejects_wrong_grid_shape():
    t = _transcription(rows=[["F"] * 13 for _ in range(12)])
    with pytest.raises(ValueError, match="grid shape"):
        build(t)


def test_build_rejects_ragged_row():
    rows = [["F"] * 13 for _ in range(13)]
    rows[0] = ["F"] * 12
    t = _transcription(rows=rows)
    with pytest.raises(ValueError, match="grid shape"):
        build(t)


def test_build_rejects_legend_action_count_mismatch():
    t = _transcription(legend={"F": [1, 0, 0], "R": [0, 1, 0]})
    with pytest.raises(ValueError, match="legend action count"):
        build(t)


def test_build_rejects_missing_legend_code():
    rows = [["F"] * 13 for _ in range(13)]
    rows[3][7] = "Z"  # not in legend
    t = _transcription(rows=rows)
    with pytest.raises(ValueError, match="legend"):
        build(t)


def test_build_transposes_action_major_and_never_emits_evs():
    t = _transcription()
    e = build(t)
    node = e["nodes"][0]
    assert len(node["weights"]) == 2
    assert all(len(row) == 169 for row in node["weights"])
    assert "evs" not in node
    assert node["unreachable_classes"] == []


def test_build_preserves_explicit_unreachable_classes():
    rows = [["F"] * 13 for _ in range(13)]
    rows[12][12] = "Z"  # class 168 ("22"): the legend code below gives it zero weight everywhere
    t = _transcription(rows=rows, unreachable_classes=[168], legend={"F": [1, 0], "R": [0, 1], "Z": [0, 0]})
    e = build(t)
    assert e["nodes"][0]["unreachable_classes"] == [168]
    assert e["nodes"][0]["weights"][0][168] == 0
    assert e["nodes"][0]["weights"][1][168] == 0


def test_build_rejects_unreachable_class_with_nonzero_weight():
    rows = [["F"] * 13 for _ in range(13)]
    rows[12][12] = "R"  # class 168 ("22") given full weight on the raise action instead
    t = _transcription(rows=rows, unreachable_classes=[168])
    # class 168 (index 168 = row 12, col 12) sums to 0 on the fold action and 1 on raise --
    # declared unreachable but not exactly zero, so `build`'s trailing `validate` must reject it.
    with pytest.raises(ValueError, match="unreachable sum"):
        build(t)


def test_build_maps_asymmetric_grid_cells_to_the_correct_classes():
    """R3 (P3.T3 fix round 1): the brief's own example mutates only the symmetric diagonal
    cell [0][0] ("AA"), which cannot distinguish correct row-major traversal from a transpose,
    nor a correct action-row order from a swapped one (both would still pass a diagonal-only,
    symmetric-value probe). This uses three distinguishable, nontrivial cells: [0][1] ("AKs")
    and [1][0] ("AKo") each carry a different two-action split, and an unrelated off-diagonal
    cell [2][5] ("Q9s") carries a third. See the P3.T3 fix-round-1 report for the executed
    demonstration that swapping row/column traversal, and separately swapping the action-row
    order, each makes this exact test fail.
    """
    rows = [["F"] * 13 for _ in range(13)]
    rows[0][1] = "X"
    rows[1][0] = "Y"
    rows[2][5] = "Z"
    legend = {"F": [1, 0], "X": [0.7, 0.3], "Y": [0.2, 0.8], "Z": [0.4, 0.6]}
    t = _transcription(rows=rows, legend=legend)
    e = build(t)
    weights = e["nodes"][0]["weights"]
    names = class_names()
    assert names[1] == "AKs"
    assert names[13] == "AKo"
    assert names[31] == "Q9s"
    assert (weights[0][1], weights[1][1]) == (0.7, 0.3), "AKs must carry [0][1]'s own values"
    assert (weights[0][13], weights[1][13]) == (0.2, 0.8), "AKo must carry [1][0]'s own values, not AKs's"
    assert (weights[0][31], weights[1][31]) == (0.4, 0.6), "Q9s must carry [2][5]'s own values"
    for c in range(169):
        if c in (1, 13, 31):
            continue
        assert weights[0][c] == 1.0 and weights[1][c] == 0.0, f"class {c} ({names[c]}) must stay at the default 'F' cell"


# --- BundleInfo field parity (Task 1) ---


def _rust_bundle_info_fields() -> list[str]:
    src = (ROOT / "crates" / "core-preflop" / "src" / "envelope.rs").read_text(encoding="utf-8")
    match = re.search(r"pub struct BundleInfo \{(.*?)\n\}", src, re.DOTALL)
    assert match, "BundleInfo struct not found in envelope.rs"
    return re.findall(r"pub (\w+):", match.group(1))


def test_bundle_info_fields_has_fifteen_entries():
    assert len(BUNDLE_INFO_FIELDS) == 15


def test_bundle_info_fields_matches_rust_struct():
    assert sorted(BUNDLE_INFO_FIELDS) == sorted(_rust_bundle_info_fields())


def test_manifest_for_chart_keys_match_bundle_info_fields():
    t = _transcription()
    e = build(t)
    manifest = manifest_for_chart(e, t)
    assert sorted(manifest) == sorted(BUNDLE_INFO_FIELDS)


def test_manifest_for_chart_forces_source_and_ev_reference():
    t = _transcription()
    e = build(t)
    manifest = manifest_for_chart(e, t)
    assert manifest["source"] == "ChartTranscription"
    assert manifest["ev_reference"] == "unverified"
    assert manifest["version"] == 2
    assert manifest["game"] == "nl"
    assert manifest["ev_unit"] == "source_sb"
    assert manifest["depths"] == [e["depth_bb"]]
    assert manifest["source_blinds"] == [0.5, 1.0]


def test_manifest_for_chart_accepts_undocumented_rake_profile_with_null_rake():
    t = _transcription()
    t["rake_profile"] = "undocumented"
    e = build(t)
    manifest = manifest_for_chart(e, t)
    assert manifest["rake"] is None
    assert manifest["rake_profile"] == "undocumented"


def test_manifest_for_chart_rejects_null_rake_with_documented_profile():
    t = _transcription()
    t["rake_profile"] = "5% cap 0.5bb"
    e = build(t)
    with pytest.raises(ValueError, match="undocumented"):
        manifest_for_chart(e, t)


def test_manifest_for_chart_sha256_matches_encoded_envelope():
    import hashlib

    t = _transcription()
    e = build(t)
    manifest = manifest_for_chart(e, t)
    raw = (json.dumps(e, indent=2, allow_nan=False) + "\n").encode("utf-8")
    assert manifest["sha256"] == hashlib.sha256(raw).hexdigest()


def test_manifest_for_chart_default_accuracy_is_unverified():
    t = _transcription()
    e = build(t)
    manifest = manifest_for_chart(e, t)
    assert manifest["accuracy"] == "unverified"


def test_manifest_for_chart_respects_explicit_accuracy():
    t = _transcription()
    t["accuracy"] = "verified-by-hand"
    e = build(t)
    manifest = manifest_for_chart(e, t)
    assert manifest["accuracy"] == "verified-by-hand"


# --- R1 (P3.T3 fix round 1): manifest/rake field types, mirroring RakeProfile/BundleInfo ---


def test_manifest_for_chart_accepts_a_well_typed_rake_object():
    t = _transcription()
    t["rake_profile"] = "5% cap 0.5bb"
    t["rake"] = {"rate": 0.05, "cap_bb": 0.5, "no_flop_no_drop": True}
    e = build(t)
    manifest = manifest_for_chart(e, t)
    assert manifest["rake"] == {"rate": 0.05, "cap_bb": 0.5, "no_flop_no_drop": True}


@pytest.mark.parametrize(
    "rake",
    [
        {},  # missing all three required fields
        {"rate": 0.05, "cap_bb": 0.5},  # missing no_flop_no_drop
        {"rate": 1.0, "cap_bb": 0.5, "no_flop_no_drop": True},  # rate domain is half-open at 1
        {"rate": -0.01, "cap_bb": 0.5, "no_flop_no_drop": True},  # rate must be >= 0
        {"rate": "0.05", "cap_bb": 0.5, "no_flop_no_drop": True},  # rate must be a number, not a string
        {"rate": 0.05, "cap_bb": -1.0, "no_flop_no_drop": True},  # cap_bb must be >= 0
        {"rate": 0.05, "cap_bb": 0.5, "no_flop_no_drop": 1},  # no_flop_no_drop must be a bool, not an int
        {"rate": True, "cap_bb": 0.5, "no_flop_no_drop": True},  # rate must be a number, not a bool
    ],
    ids=["empty", "missing_field", "rate_domain_high", "rate_domain_low", "rate_string", "cap_negative", "nfnd_int", "rate_bool"],
)
def test_manifest_for_chart_rejects_malformed_rake_object(rake):
    t = _transcription()
    t["rake_profile"] = "5% cap 0.5bb"
    t["rake"] = rake
    e = build(t)
    with pytest.raises(ValueError, match="rake"):
        manifest_for_chart(e, t)


def test_manifest_for_chart_rejects_non_string_license_note():
    t = _transcription()
    t["license_note"] = 12345
    e = build(t)
    with pytest.raises(ValueError, match="license_note"):
        manifest_for_chart(e, t)


def test_manifest_for_chart_rejects_non_string_accuracy():
    t = _transcription()
    t["accuracy"] = 1
    e = build(t)
    with pytest.raises(ValueError, match="accuracy"):
        manifest_for_chart(e, t)


# --- validate: exhaustive malformed-row parametrization ---


def _valid_envelope() -> dict:
    weights = [[0.0] * 169, [0.0] * 169]
    for c in range(169):
        weights[0][c] = 1.0
    return {
        "bundle_id": "test",
        "depth_bb": 100,
        "rake_profile": "undocumented",
        "straddle": False,
        "class_order": "A-2 row-major, section 4.1",
        "nodes": [
            {
                "history": [],
                "actor": "UTG",
                "actions": [{"step": "fold"}, {"step": "raise", "to_bb_x1000": 2500}],
                "weights": weights,
                "unreachable_classes": [],
            }
        ],
    }


def test_valid_envelope_passes():
    validate(_valid_envelope())


def _mut_bad_actor(e):
    e["nodes"][0]["actor"] = "ZZ"
    return e


def _mut_bad_history_position(e):
    e["nodes"][0]["history"] = [["ZZ", "fold", 0]]
    return e


def _mut_raise_missing_amount_in_history(e):
    e["nodes"][0]["history"] = [["UTG", "raise", 0]]
    return e


def _mut_action_raise_missing_amount(e):
    e["nodes"][0]["actions"] = [{"step": "fold"}, {"step": "raise"}]
    return e


def _mut_action_fold_with_amount(e):
    e["nodes"][0]["actions"] = [{"step": "fold", "to_bb_x1000": 100}, {"step": "raise", "to_bb_x1000": 2500}]
    return e


def _mut_action_unknown_step(e):
    e["nodes"][0]["actions"] = [{"step": "bogus"}, {"step": "raise", "to_bb_x1000": 2500}]
    return e


def _mut_duplicate_action_kind(e):
    e["nodes"][0]["actions"] = [{"step": "fold"}, {"step": "fold"}]
    return e


def _mut_duplicate_unreachable_class(e):
    e["nodes"][0]["unreachable_classes"] = [5, 5]
    return e


def _mut_unreachable_class_out_of_range(e):
    e["nodes"][0]["unreachable_classes"] = [169]
    return e


def _mut_duplicate_node_history(e):
    e["nodes"].append(copy.deepcopy(e["nodes"][0]))
    return e


def _mut_empty_menu(e):
    e["nodes"][0]["actions"] = []
    e["nodes"][0]["weights"] = []
    return e


def _mut_actions_weights_shape_mismatch(e):
    e["nodes"][0]["weights"] = [e["nodes"][0]["weights"][0]]
    return e


def _mut_row_not_169_wide(e):
    e["nodes"][0]["weights"][0] = e["nodes"][0]["weights"][0][:-1]
    return e


def _mut_probability_out_of_bounds(e):
    e["nodes"][0]["weights"][0][0] = 1.5
    return e


def _mut_sibling_sum_wrong(e):
    e["nodes"][0]["weights"][0][0] = 0.1
    return e


def _mut_unreachable_sum_nonzero(e):
    e["nodes"][0]["unreachable_classes"] = [0]
    return e


def _mut_evs_forbidden(e):
    e["nodes"][0]["evs"] = [[None] * 169, [None] * 169]
    return e


def _mut_class_order_wrong(e):
    e["class_order"] = "some other order"
    return e


def _mut_depth_bb_zero(e):
    e["depth_bb"] = 0
    return e


# --- R1 (P3.T3 fix round 1): exact JSON scalar type/domain rules mirroring the Rust schema ---


def _mut_depth_bb_negative(e):
    e["depth_bb"] = -1
    return e


def _mut_depth_bb_too_large(e):
    e["depth_bb"] = 65536
    return e


def _mut_depth_bb_float(e):
    e["depth_bb"] = 100.0  # a Rust u16 field must never accept a JSON float, even a whole one
    return e


def _mut_straddle_string(e):
    e["straddle"] = "false"  # a JSON string, not the Rust bool the field actually is
    return e


def _mut_straddle_int(e):
    e["straddle"] = 0  # bool is never a number -- Python's `0 == False` must not paper over this
    return e


def _mut_bundle_id_wrong_type(e):
    e["bundle_id"] = 123
    return e


def _mut_raise_amount_float(e):
    e["nodes"][0]["actions"][1]["to_bb_x1000"] = 2500.5
    return e


def _mut_raise_amount_overflow_u32(e):
    e["nodes"][0]["actions"][1]["to_bb_x1000"] = 4294967296  # u32::MAX + 1
    return e


def _mut_raise_amount_bool(e):
    e["nodes"][0]["actions"][1]["to_bb_x1000"] = True
    return e


def _mut_action_unknown_field(e):
    e["nodes"][0]["actions"][0]["note"] = "not a field Rust's EnvelopeAction has"
    return e


def _mut_action_label_wrong_type(e):
    e["nodes"][0]["actions"][0]["label"] = 123
    return e


def _mut_history_amount_float(e):
    e["nodes"][0]["history"] = [["UTG", "raise", 2500.0]]
    return e


def _mut_history_amount_negative(e):
    e["nodes"][0]["history"] = [["UTG", "raise", -5]]
    return e


def _mut_unreachable_class_float(e):
    e["nodes"][0]["unreachable_classes"] = [5.0]
    return e


def _mut_unreachable_class_bool(e):
    e["nodes"][0]["unreachable_classes"] = [True]
    return e


def _mut_weight_boolean(e):
    e["nodes"][0]["weights"][0][0] = True  # numerically 1 (a valid-looking probability), but not a JSON number
    return e


MALFORMED_CASES = [
    (_mut_bad_actor, "actor"),
    (_mut_bad_history_position, "history"),
    (_mut_raise_missing_amount_in_history, "history"),
    (_mut_action_raise_missing_amount, "action"),
    (_mut_action_fold_with_amount, "action"),
    (_mut_action_unknown_step, "action"),
    (_mut_duplicate_action_kind, "duplicate action"),
    (_mut_duplicate_unreachable_class, "duplicate unreachable"),
    (_mut_unreachable_class_out_of_range, "range"),
    (_mut_duplicate_node_history, "duplicate node"),
    (_mut_empty_menu, "empty menu"),
    (_mut_actions_weights_shape_mismatch, "shape"),
    (_mut_row_not_169_wide, "169"),
    (_mut_probability_out_of_bounds, "bound"),
    (_mut_sibling_sum_wrong, "sum"),
    (_mut_unreachable_sum_nonzero, "unreachable sum"),
    (_mut_evs_forbidden, "EV"),
    (_mut_class_order_wrong, "class order"),
    (_mut_depth_bb_zero, "depth"),
    (_mut_depth_bb_negative, "depth"),
    (_mut_depth_bb_too_large, "depth"),
    (_mut_depth_bb_float, "depth"),
    (_mut_straddle_string, "straddle"),
    (_mut_straddle_int, "straddle"),
    (_mut_bundle_id_wrong_type, "bundle_id"),
    (_mut_raise_amount_float, "action"),
    (_mut_raise_amount_overflow_u32, "action"),
    (_mut_raise_amount_bool, "action"),
    (_mut_action_unknown_field, "unknown"),
    (_mut_action_label_wrong_type, "label"),
    (_mut_history_amount_float, "history"),
    (_mut_history_amount_negative, "history"),
    (_mut_unreachable_class_float, "integer"),
    (_mut_unreachable_class_bool, "integer"),
    (_mut_weight_boolean, "bound"),
]


@pytest.mark.parametrize("mutate,match", MALFORMED_CASES, ids=[m.__name__ for m, _ in MALFORMED_CASES])
def test_validate_rejects_malformed_rows(mutate, match):
    envelope = mutate(copy.deepcopy(_valid_envelope()))
    with pytest.raises(ValueError, match=match):
        validate(envelope)


# --- N1 (P3.T3 fix round 2, minor): boundary-value ACCEPTANCE regressions. Fix round 1 added
# rejection tests for out-of-domain and immediately-out-of-range values (e.g. depth_bb 0 and
# 65536, to_bb_x1000 4294967296); it never separately confirmed that the domain's own inclusive
# endpoints (depth_bb 1 and 65535, to_bb_x1000 1 and u32::MAX, weight 0.0 and 1.0) actually
# validate successfully, as opposed to merely being adjacent to a rejected value. ---


@pytest.mark.parametrize("depth_bb", [1, 65535], ids=["min", "max"])
def test_validate_accepts_depth_bb_boundary_values(depth_bb):
    envelope = copy.deepcopy(_valid_envelope())
    envelope["depth_bb"] = depth_bb
    validate(envelope)  # must not raise


@pytest.mark.parametrize("amount", [1, 4294967295], ids=["min", "u32_max"])
def test_validate_accepts_raise_amount_boundary_values(amount):
    """`to_bb_x1000` boundary values where the token rule allows an amount at all (`raise`)."""
    envelope = copy.deepcopy(_valid_envelope())
    envelope["nodes"][0]["actions"][1]["to_bb_x1000"] = amount
    validate(envelope)  # must not raise


def test_validate_accepts_weight_boundary_values_zero_and_one():
    envelope = copy.deepcopy(_valid_envelope())
    # The closed interval `[0, 1]`'s two inclusive endpoints, explicit and asserted here rather
    # than only implicitly present (as every other passing test's default weights already are).
    envelope["nodes"][0]["weights"][0][0] = 1.0
    envelope["nodes"][0]["weights"][1][0] = 0.0
    validate(envelope)  # must not raise


def test_build_accepts_weight_boundary_values_through_a_transcription_grid():
    """The same two boundary values, reached through `build`'s legend lookup (a transcription
    grid cell), not only by constructing an already-built envelope dict directly."""
    legend = {"F": [1.0, 0.0], "R": [0.0, 1.0]}
    t = _transcription(legend=legend)
    e = build(t)  # must not raise
    assert e["nodes"][0]["weights"][0][0] == 1.0
    assert e["nodes"][0]["weights"][1][0] == 0.0


# The immediately-out-of-range rejections (depth 0/65536, amount 4294967296) are already
# covered by MALFORMED_CASES above (`_mut_depth_bb_zero`, `_mut_depth_bb_too_large`,
# `_mut_raise_amount_overflow_u32`) -- not duplicated here.


# --- R4 (P3.T3 fix round 1): class-specific validation errors carry both index and name ---


def test_validate_sibling_sum_error_names_both_class_index_and_hand_name():
    envelope = _mut_sibling_sum_wrong(copy.deepcopy(_valid_envelope()))
    with pytest.raises(ValueError, match=r"class 0 sum .*\bAA\b"):
        validate(envelope)


def test_validate_unreachable_sum_error_names_both_class_index_and_hand_name():
    envelope = _mut_unreachable_sum_nonzero(copy.deepcopy(_valid_envelope()))
    with pytest.raises(ValueError, match=r"unreachable sum at class 0.*\bAA\b"):
        validate(envelope)


def test_validate_probability_bound_error_names_both_class_index_and_hand_name():
    envelope = _mut_probability_out_of_bounds(copy.deepcopy(_valid_envelope()))
    with pytest.raises(ValueError, match=r"probability bound at class 0.*\bAA\b"):
        validate(envelope)


# --- CLI: build / validate / verify round trip ---


def test_cli_build_writes_envelope_and_manifest(tmp_path):
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"

    rc = chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 0
    envelope = json.loads(output_path.read_text(encoding="utf-8"))
    assert envelope == build(t)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert manifest == manifest_for_chart(build(t), t)
    # LF-only, deterministic, trailing newline (matches gen_preflop_fixtures.py's write_json).
    assert output_path.read_bytes().endswith(b"\n")
    assert b"\r\n" not in output_path.read_bytes()


def test_cli_validate_success_prints_report(tmp_path, capsys):
    envelope_path = tmp_path / "envelope.json"
    envelope_path.write_text(json.dumps(_valid_envelope()), encoding="utf-8")
    rc = chart_ingest.main(["validate", str(envelope_path)])
    assert rc == 0
    lines = [line for line in capsys.readouterr().out.splitlines() if line.strip()]
    assert len(lines) == 2  # one node report line + one aggregate line
    node_report = json.loads(lines[0])
    assert len(node_report["sums"]) == 169
    aggregate = json.loads(lines[1])
    assert aggregate["aggregate_min"] == pytest.approx(1.0)
    assert aggregate["aggregate_max"] == pytest.approx(1.0)


def test_cli_validate_failure_exits_nonzero(tmp_path, capsys):
    bad = _mut_sibling_sum_wrong(_valid_envelope())
    envelope_path = tmp_path / "envelope.json"
    envelope_path.write_text(json.dumps(bad), encoding="utf-8")
    rc = chart_ingest.main(["validate", str(envelope_path)])
    assert rc == 1
    err = capsys.readouterr().err
    assert "sum" in err


def test_cli_verify_succeeds_on_untouched_build(tmp_path, capsys):
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    t["inventory"] = [{"title": "UTG RFI", "status": "covered", "history": [], "page": 2, "reason": "2.5bb"}]
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    assert chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)]) == 0

    rc = chart_ingest.main(["verify", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 0
    out = capsys.readouterr().out
    assert "verified 1 nodes, 169 classes" in out


def test_cli_verify_detects_corrupted_output_bytes(tmp_path, capsys):
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    t["inventory"] = [{"title": "UTG RFI", "status": "covered", "history": [], "page": 2, "reason": "2.5bb"}]
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)])

    envelope = json.loads(output_path.read_text(encoding="utf-8"))
    envelope["nodes"][0]["weights"][0][0] = 0.4  # class 0 = AA
    envelope["nodes"][0]["weights"][1][0] = 0.6  # keep the sibling sum valid so only bytes differ
    output_path.write_text(json.dumps(envelope, indent=2) + "\n", encoding="utf-8")

    rc = chart_ingest.main(["verify", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 1
    err = capsys.readouterr().err
    assert "node 0" in err
    assert "AA" in err


def test_cli_verify_detects_inventory_envelope_key_mismatch(tmp_path, capsys):
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    t["inventory"] = []  # the built node is never listed as covered
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)])

    rc = chart_ingest.main(["verify", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 1
    err = capsys.readouterr().err
    assert "mismatch" in err


# --- R2 (P3.T3 fix round 1): every inventory row is validated, including absent ones ---


def _built_transcription_and_paths(tmp_path, inventory):
    t = _transcription()
    t["inventory"] = inventory
    transcription_path = tmp_path / "transcription.json"
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    assert chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)]) == 0
    return transcription_path, output_path, manifest_path


def _covered_row(**overrides):
    row = {"title": "UTG RFI", "status": "covered", "history": [], "page": 2, "reason": "2.5bb open"}
    row.update(overrides)
    return row


def test_cli_verify_rejects_inventory_row_missing_required_fields(tmp_path, capsys):
    paths = _built_transcription_and_paths(tmp_path, [{"status": "covered"}])
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 1
    assert "title" in capsys.readouterr().err


def test_cli_verify_rejects_invalid_inventory_status(tmp_path, capsys):
    paths = _built_transcription_and_paths(tmp_path, [_covered_row(status="maybe")])
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 1
    assert "status" in capsys.readouterr().err


def test_cli_verify_rejects_inventory_row_with_bad_page_type(tmp_path, capsys):
    paths = _built_transcription_and_paths(tmp_path, [_covered_row(page="two")])
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 1
    assert "page" in capsys.readouterr().err


def test_cli_verify_rejects_inventory_row_with_empty_reason(tmp_path, capsys):
    paths = _built_transcription_and_paths(tmp_path, [_covered_row(reason="   ")])
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 1
    assert "reason" in capsys.readouterr().err


def test_cli_verify_rejects_inventory_row_with_malformed_history_position(tmp_path, capsys):
    paths = _built_transcription_and_paths(tmp_path, [_covered_row(history=[["ZZ", "fold", 0]])])
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 1
    assert "history" in capsys.readouterr().err


def test_cli_verify_validates_absent_rows_too_not_only_covered_ones(tmp_path, capsys):
    """The covered row matches the built node exactly (so the covered/envelope key check
    alone would pass); the absent row's malformed history must still be caught."""
    inventory = [_covered_row(), {"title": "CO cold call", "status": "absent", "history": [["ZZ", "fold", 0]], "page": None, "reason": "no grid shown"}]
    paths = _built_transcription_and_paths(tmp_path, inventory)
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 1
    assert "history" in capsys.readouterr().err


def test_cli_verify_accepts_a_well_formed_absent_row(tmp_path, capsys):
    inventory = [_covered_row(), {"title": "CO cold call", "status": "absent", "history": [["CO", "call", 0]], "page": None, "reason": "no grid shown for this line"}]
    paths = _built_transcription_and_paths(tmp_path, inventory)
    rc = chart_ingest.main(["verify", *[str(p) for p in paths]])
    assert rc == 0, capsys.readouterr().err


# --- R4 (P3.T3 fix round 1): verify diagnostics never leak a traceback on malformed input ---


def test_cli_verify_reports_class_index_and_name_together(tmp_path, capsys):
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    t["inventory"] = [_covered_row()]
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)])

    envelope = json.loads(output_path.read_text(encoding="utf-8"))
    envelope["nodes"][0]["weights"][0][0] = 0.4
    envelope["nodes"][0]["weights"][1][0] = 0.6
    output_path.write_text(json.dumps(envelope, indent=2) + "\n", encoding="utf-8")

    rc = chart_ingest.main(["verify", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 1
    err = capsys.readouterr().err
    assert "node 0" in err
    assert "class 0" in err
    assert "AA" in err


def test_cli_verify_handles_a_malformed_stored_node_without_a_traceback(tmp_path, capsys):
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    t["inventory"] = [_covered_row()]
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)])

    envelope = json.loads(output_path.read_text(encoding="utf-8"))
    envelope["nodes"] = [None]  # structurally invalid stored output -- not just a value mismatch
    output_path.write_text(json.dumps(envelope, indent=2) + "\n", encoding="utf-8")

    rc = chart_ingest.main(["verify", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 1
    err = capsys.readouterr().err
    assert err.startswith("error:"), f"main() must report a clean typed error, not a traceback: {err!r}"
    assert "Traceback" not in err


def test_cli_verify_rejects_on_disk_manifest_with_straddle_as_integer_not_boolean(tmp_path, capsys):
    """R1: dict equality alone would let a stored `straddle: 0` silently pass as `False`
    (Python's `0 == False`); `verify` must type-check the on-disk manifest before comparing."""
    t = _transcription()
    transcription_path = tmp_path / "transcription.json"
    t["inventory"] = [_covered_row()]
    transcription_path.write_text(json.dumps(t), encoding="utf-8")
    output_path = tmp_path / "out.json"
    manifest_path = tmp_path / "out.manifest.json"
    chart_ingest.main(["build", str(transcription_path), str(output_path), str(manifest_path)])

    manifest_text = manifest_path.read_text(encoding="utf-8")
    assert '"straddle": false' in manifest_text
    manifest_path.write_text(manifest_text.replace('"straddle": false', '"straddle": 0'), encoding="utf-8")

    rc = chart_ingest.main(["verify", str(transcription_path), str(output_path), str(manifest_path)])
    assert rc == 1
    assert "straddle" in capsys.readouterr().err


# --- fetch: monkeypatched urlopen, no real network access ---


class _FakeResponse:
    def __init__(self, data: bytes, url: str):
        self._data = data
        self.url = url

    def read(self, n: int = -1) -> bytes:
        return self._data if n < 0 else self._data[:n]

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False


def test_fetch_writes_bytes_and_reports_final_url(tmp_path, monkeypatch, capsys):
    monkeypatch.setattr(chart_ingest, "urlopen", lambda req, timeout=30: _FakeResponse(b"hello world", "https://example.com/final"))
    output = tmp_path / "out.txt"
    chart_ingest.fetch("https://example.com/start", output)
    assert output.read_bytes() == b"hello world"
    payload = json.loads(capsys.readouterr().out)
    assert payload["url"] == "https://example.com/final"
    assert payload["bytes"] == len(b"hello world")
    import hashlib

    assert payload["sha256"] == hashlib.sha256(b"hello world").hexdigest()


def test_fetch_rejects_html_returned_as_pdf(tmp_path, monkeypatch):
    monkeypatch.setattr(chart_ingest, "urlopen", lambda req, timeout=30: _FakeResponse(b"<html>not found</html>", "https://example.com/final.pdf"))
    output = tmp_path / "out.pdf"
    with pytest.raises(ValueError, match="non-PDF"):
        chart_ingest.fetch("https://example.com/start", output)
    assert not output.exists()


def test_fetch_accepts_pdf_magic_bytes(tmp_path, monkeypatch):
    monkeypatch.setattr(chart_ingest, "urlopen", lambda req, timeout=30: _FakeResponse(b"%PDF-1.4 minimal", "https://example.com/final.pdf"))
    output = tmp_path / "out.pdf"
    chart_ingest.fetch("https://example.com/start", output)
    assert output.read_bytes() == b"%PDF-1.4 minimal"


def test_fetch_rejects_oversized_download(tmp_path, monkeypatch):
    monkeypatch.setattr(chart_ingest, "MAX_BUNDLE_BYTES", 10)
    monkeypatch.setattr(chart_ingest, "urlopen", lambda req, timeout=30: _FakeResponse(b"x" * 20, "https://example.com/final"))
    output = tmp_path / "out.bin"
    with pytest.raises(ValueError, match="64 MiB"):
        chart_ingest.fetch("https://example.com/start", output)
    assert not output.exists()


# --- bounded_read / load_json ---


def test_bounded_read_rejects_oversized_file(tmp_path, monkeypatch):
    monkeypatch.setattr(chart_ingest, "MAX_BUNDLE_BYTES", 10)
    path = tmp_path / "big.json"
    path.write_bytes(b"x" * 20)
    with pytest.raises(ValueError, match="64 MiB"):
        chart_ingest.bounded_read(path)


def test_load_json_round_trips_a_small_file(tmp_path):
    path = tmp_path / "small.json"
    path.write_text(json.dumps({"a": 1}), encoding="utf-8")
    assert chart_ingest.load_json(path) == {"a": 1}


# --- extract_links / find_anchor (html.parser) ---


SAMPLE_HTML = """
<html><body>
<a href="/page-a">Some other link</a>
<a href="/downloads/gto">6 max 200bb 500z GTO Ranges</a>
</body></html>
"""


def test_extract_links_returns_text_and_href_pairs():
    links = extract_links(SAMPLE_HTML)
    assert ("Some other link", "/page-a") in links
    assert ("6 max 200bb 500z GTO Ranges", "/downloads/gto") in links


def test_find_anchor_returns_href_for_matching_text():
    assert find_anchor(SAMPLE_HTML, "6 max 200bb 500z GTO Ranges") == "/downloads/gto"


def test_find_anchor_returns_none_when_absent():
    assert find_anchor(SAMPLE_HTML, "nonexistent link text") is None


# --- bb_to_x1000 (decimal-exact bb -> to_bb_x1000 conversion) ---


def test_bb_to_x1000_exact_decimal_conversion():
    assert bb_to_x1000("8.75") == 8750
    assert bb_to_x1000(22) == 22000
    assert bb_to_x1000("0.1") == 100


def test_bb_to_x1000_rejects_non_positive():
    with pytest.raises(ValueError):
        bb_to_x1000(0)
    with pytest.raises(ValueError):
        bb_to_x1000(-2.5)


def test_bb_to_x1000_rejects_invalid_decimal():
    with pytest.raises(ValueError):
        bb_to_x1000("1/3")


# --- sources.manifest.json availability (P3.T4): which chart depths this build actually has ---


def load_availability():
    return json.loads((ROOT / "fixtures/charts/sources.manifest.json").read_text(encoding="utf-8"))


def available_depths():
    return {d["depth_bb"]: d for d in load_availability()["depths"] if d["status"] == "available"}


def assert_committed_bytes_match(path, expected_bytes, expected_sha256):
    """The one integrity check every manifest-tracked committed artifact (a depth's PDF, or a
    provenance HTML) is verified against: `path` must be a regular file, its actual byte count
    must equal `expected_bytes` exactly, and `hashlib.sha256` over its actual bytes must equal
    `expected_sha256` exactly. Raises `AssertionError` (never returns a bool, never skips) on
    a missing file, a wrong length, or a content change that keeps the same length -- the
    review's explicit requirement that a listed source whose actual bytes differ from its
    manifest entry fails this test, rather than the previous `len(sha256) == 64 and bytes > 0`
    check, which read no file at all and could not detect drift, truncation or replacement.
    """
    import hashlib

    assert path.is_file(), f"{path} is missing or not a regular file"
    raw = path.read_bytes()
    assert len(raw) == expected_bytes, (
        f"{path}: manifest says {expected_bytes} bytes, on-disk file has {len(raw)}"
    )
    actual_sha256 = hashlib.sha256(raw).hexdigest()
    assert actual_sha256 == expected_sha256, (
        f"{path}: manifest sha256 {expected_sha256} does not match on-disk sha256 {actual_sha256}"
    )


def test_sources_manifest_shape():
    m = load_availability()
    assert m["schema_version"] == 1
    assert [d["depth_bb"] for d in m["depths"]] == [100, 200]
    for d in m["depths"]:
        assert d["status"] in ("available", "unsupported")
        if d["status"] == "available":
            assert len(d["sha256"]) == 64 and d["bytes"] > 0
        else:
            assert d["source_file"] is None and d["sha256"] is None
            assert d["note"].startswith(f"depth {d['depth_bb']} unsupported")
    assert available_depths(), "no chart depth acquired; the release has no range source"


def test_sources_manifest_depth_files_match_their_recorded_bytes_and_hash():
    """The actual integrity gate (R2): every `"available"` depth's committed `source_file`
    must have exactly the recorded byte count and SHA-256, read from disk, not merely a
    64-character hex string and a positive declared size."""
    m = load_availability()
    checked = 0
    for d in m["depths"]:
        if d["status"] != "available":
            continue
        assert_committed_bytes_match(ROOT / d["source_file"], d["bytes"], d["sha256"])
        checked += 1
    assert checked >= 1


def test_sources_manifest_provenance_artifacts_present_and_integrity_checked():
    """R3: the redacted `rangeconverter_200.html` provenance HTML is not a chart source (it is
    not any depth's `source_file`), but it must still be integrity-pinned in the manifest, with
    its committed (post-redaction) bytes/hash kept separate from its original fetched
    bytes/hash so the redaction itself stays auditable."""
    m = load_availability()
    artifacts = m["provenance_artifacts"]
    assert artifacts, "no provenance_artifacts entry; the redacted article HTML is unpinned"
    required_fields = {
        "artifact_id",
        "path",
        "fetched_url",
        "fetched_utc",
        "fetched_bytes",
        "fetched_sha256",
        "committed_bytes",
        "committed_sha256",
        "redaction_note",
    }
    for artifact in artifacts:
        assert required_fields <= set(artifact.keys())
        assert len(artifact["fetched_sha256"]) == 64
        assert len(artifact["committed_sha256"]) == 64
        assert_committed_bytes_match(
            ROOT / artifact["path"], artifact["committed_bytes"], artifact["committed_sha256"]
        )


def test_rangeconverter_article_provenance_record_matches_the_acquisition_report():
    """Pins the one provenance artifact's exact recorded values (not just its shape) against
    what P3.T4's acquisition actually reported: the `fetch` at Step 3a returned 16584 bytes,
    and the two token redactions dropped it to 16432 committed bytes -- a real difference, not
    a placeholder pair of equal numbers."""
    m = load_availability()
    article = next(
        a for a in m["provenance_artifacts"] if a["artifact_id"] == "rangeconverter_200_article"
    )
    assert article["fetched_bytes"] == 16584
    assert article["fetched_sha256"] == (
        "33b9007cf6318b347a5ade89d6e2ba099b126dc86207a45dbd24f9afeda3a76d"
    )
    assert article["committed_bytes"] == 16432
    assert article["committed_sha256"] == (
        "f9cba1810f1486b320934f15a07be59ab62d28d860f39fa04586d667b7506c7c"
    )
    assert article["fetched_bytes"] != article["committed_bytes"]
    assert article["fetched_sha256"] != article["committed_sha256"]


def test_assert_committed_bytes_match_fails_on_missing_file(tmp_path):
    with pytest.raises(AssertionError, match="missing"):
        assert_committed_bytes_match(tmp_path / "does_not_exist.bin", 4, "0" * 64)


def test_assert_committed_bytes_match_fails_on_length_mismatch(tmp_path):
    import hashlib

    path = tmp_path / "recorded.bin"
    path.write_bytes(b"1234")
    wrong_length = 5
    real_sha256 = hashlib.sha256(b"1234").hexdigest()
    with pytest.raises(AssertionError, match="bytes"):
        assert_committed_bytes_match(path, wrong_length, real_sha256)


def test_assert_committed_bytes_match_fails_on_hash_mismatch_same_length(tmp_path):
    import hashlib

    path = tmp_path / "tampered.bin"
    path.write_bytes(b"1234")  # same length as the "recorded" content below, different bytes
    recorded_sha256 = hashlib.sha256(b"5678").hexdigest()
    with pytest.raises(AssertionError, match="sha256"):
        assert_committed_bytes_match(path, 4, recorded_sha256)


def test_assert_committed_bytes_match_passes_on_a_genuine_match(tmp_path):
    import hashlib

    path = tmp_path / "good.bin"
    path.write_bytes(b"exact-bytes")
    assert_committed_bytes_match(path, len(b"exact-bytes"), hashlib.sha256(b"exact-bytes").hexdigest())
