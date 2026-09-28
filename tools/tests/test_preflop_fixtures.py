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


# --- P3.T19: the replay and bet-translation engine goldens (spec section 13.3) -----------------
#
# `crates/engine/tests/golden/{replay_weights_golden,bet_translation_golden}.json` are written by
# `gen_preflop_fixtures.generate_goldens` from the oracles below; `crates/engine/tests/
# preflop_goldens.rs` compares the Rust implementation against them. These tests pin the oracles
# themselves: their indexing, a closed form of each headline figure computed a second way, the
# single f32 conversion, and byte-identical regeneration.

import hashlib
import math
import struct
from fractions import Fraction

GOLDENS = gpf.ENGINE_GOLDEN_DIR


def golden(name: str) -> dict:
    return json.loads((GOLDENS / name).read_text(encoding="utf-8"))


def is_f32(x: float) -> bool:
    return struct.unpack("<f", struct.pack("<f", x))[0] == x


def test_combo_class_expands_aa_aks_ako_to_6_4_12():
    counts = [0] * 169
    for hi in range(1, 52):
        for lo in range(hi):
            counts[gpf.combo_class(lo, hi)] += 1
    assert (counts[0], counts[1], counts[13]) == (6, 4, 12)  # AA, AKs, AKo
    assert sum(counts) == 1326 and all(n in (4, 6, 12) for n in counts)
    assert sorted(set(counts)) == [4, 6, 12] and counts.count(6) == 13 and counts.count(4) == 78


def test_combo_class_and_card_ids_follow_spec_section_4_1():
    # Card id = rank * 4 + suit, ranks 2..A = 0..12, suits c, d, h, s = 0..3.
    assert [gpf.card_id(t) for t in ("2c", "7d", "Kh", "As")] == [0, 21, 46, 51]
    assert gpf.BOARD == {46, 21, 0} == {gpf.card_id(t) for t in ("Kh", "7d", "2c")}
    assert gpf.combo_class(gpf.card_id("Ah"), gpf.card_id("As")) == 0  # AA, the diagonal
    assert gpf.combo_class(gpf.card_id("Ks"), gpf.card_id("As")) == 1  # AKs above it
    assert gpf.combo_class(gpf.card_id("Kd"), gpf.card_id("As")) == 13  # AKo below it
    assert gpf.combo_class(gpf.card_id("2c"), gpf.card_id("7s")) == 163  # 72o
    assert gpf.combo_index(gpf.card_id("2c"), gpf.card_id("2d")) == 0
    assert gpf.combo_index(gpf.card_id("As"), gpf.card_id("Ah")) == 1325
    assert len(gpf.vector_for(lambda c: c)) == 1326


def test_committed_goldens_exist():
    for name in gpf.GOLDEN_FILES:
        path = GOLDENS / name
        assert path.is_file(), f"required committed golden is missing: {path}"


def test_golden_regeneration_reproduces_committed_bytes_exactly(tmp_path: Path):
    gpf.generate_goldens(tmp_path)
    for name in gpf.GOLDEN_FILES:
        assert (tmp_path / name).read_bytes() == (GOLDENS / name).read_bytes(), f"{name} is not byte-identical"


def test_golden_sources_are_synthetic_embedded_and_hash_checked():
    replay = golden("replay_weights_golden.json")
    bet = golden("bet_translation_golden.json")
    sources = replay["input"]["sources"] + replay["offmenu"]["input"]["sources"] + bet["prominence"]["sources"]
    assert len(sources) == 3
    charts = gpf.ROOT / "fixtures" / "charts"
    for s in sources:
        m = s["manifest"]
        assert m["bundle_id"].startswith("golden_") and "Synthetic" in m["license_note"]
        assert m["sha256"] == hashlib.sha256(s["nodes_json"].encode("utf-8")).hexdigest()
        assert json.loads(s["nodes_json"])["bundle_id"] == m["bundle_id"]
        assert not list(charts.rglob(f"*{m['bundle_id']}*")), "golden sources are never added to charts"
    for g in (replay, bet):
        assert g["schema_version"] == 1 and g["synthetic"] is True


def test_replay_golden_q_and_log_reach_match_a_class_count_closed_form():
    # Independent of the per-combo walk: integrate each likelihood over classes by multiplicity.
    counts = [0] * 169
    for hi in range(1, 52):
        for lo in range(hi):
            counts[gpf.combo_class(lo, hi)] += 1
    f = gpf.f32
    m = [math.fsum(n * f(p(c)) for c, n in enumerate(counts)) / 1326 for p in gpf.UNIFORM_LINE_LIKELIHOODS]
    g = golden("replay_weights_golden.json")["expected"]
    assert abs(g["q"] - m[0] * m[1] * m[2]) < 1e-14
    # log_reach[S] is ln of the unblocked maximum of S's un-normalized marginal: each seat's own
    # likelihood peak (unblocked on Kh7d2c: AA, a class 0 combo, holds no board card) times the
    # other two seats' integrated likelihoods.
    assert abs(g["log_reach"][0] - math.log(f(0.8) * m[1] * m[2])) < 1e-12
    assert abs(g["log_reach"][1] - math.log(m[0] * 1.0 * m[2])) < 1e-12
    assert abs(g["log_reach"][2] - math.log(m[0] * m[1] * f(0.9))) < 1e-12
    assert g["log_reach"][3:] == [0.0, 0.0, 0.0]


def test_replay_golden_ranges_are_f32_once_board_blocked_and_rescaled():
    g = golden("replay_weights_golden.json")["expected"]
    assert g["ranges"][3:] == [None, None, None]
    for r in g["ranges"][:3]:
        assert len(r) == 1326 and all(is_f32(x) for x in r) and max(r) == 1.0
        for (lo, hi), x in zip(gpf.COMBO_PAIRS, r):
            assert (x == 0.0) == (lo in gpf.BOARD or hi in gpf.BOARD)
    # The SB folded but keeps its posterior: fold likelihood 1 off class multiples of 3.
    sb = g["ranges"][1]
    for (lo, hi), x in zip(gpf.COMBO_PAIRS, sb):
        if lo not in gpf.BOARD and hi not in gpf.BOARD:
            assert x == (gpf.f32(0.25) if gpf.combo_class(lo, hi) % 3 == 0 else 1.0)
    assert g["folded"] == [1]


def test_offmenu_oracle_f_q_and_posteriors():
    fa = Fraction(1, 1) - Fraction(73, 100)
    fa = fa * Fraction(3, 2) / (Fraction(1, 2) * Fraction(173, 100))
    assert fa == Fraction(81, 173)
    g = golden("replay_weights_golden.json")["offmenu"]["expected"]
    assert abs(g["f"][0] - 81 / 173) < 1e-15 and abs(g["f"][1] - 92 / 173) < 1e-15
    total = sum(g["q"])
    assert all(abs(x - q / total) < 1e-15 for x, q in zip(g["bb_posterior"], g["q"]))
    for entry in g["btn_posterior"].values():
        assert abs(sum(entry["posterior"]) - 1.0) < 1e-12
    # AA (class 0, even) favours the small size; AKs and 72o (odd) the large one.
    assert g["btn_posterior"]["AA"]["posterior"][0] > 0.5 > g["btn_posterior"]["AKs"]["posterior"][0]
    assert g["btn_posterior"]["AKs"]["posterior"] == g["btn_posterior"]["72o"]["posterior"]
    assert [b["translated"][0][1] for b in g["branches"]] == [{"kind": "raise", "to": 90}, {"kind": "raise", "to": 140}]
    # The published BTN range is the un-normalized marginal scaled to maximum 1, converted once.
    peak = max(g["btn_marginal"])
    assert g["ranges"][0] == [gpf.f32(x / peak) for x in g["btn_marginal"]]
    assert abs(g["log_reach"][0] - math.log(peak)) < 1e-15
    assert all(is_f32(x) for r in g["ranges"][:3] for x in r)


def exact_interpolation(s: Fraction, menu: list) -> tuple:
    """Spec section 8.4's boundaries in exact rationals: returns (f per menu entry, d, clamped)."""
    sizes = sorted(set(menu))
    f = [Fraction(0)] * len(menu)
    first = lambda x: menu.index(x)  # noqa: E731 - the first menu entry of a size
    if s in sizes:
        f[first(s)] = Fraction(1)
        return f, Fraction(0), False
    if len(sizes) == 1 or s < sizes[0] or s > sizes[-1]:
        x = sizes[0] if s < sizes[0] else sizes[-1]
        f[first(x)] = Fraction(1)
        return f, abs(s - x), True
    a = max(x for x in sizes if x < s)
    b = min(x for x in sizes if x > s)
    fa = (b - s) * (1 + a) / ((b - a) * (1 + s))
    f[first(a)], f[first(b)] = fa, 1 - fa
    return f, min(abs(s - a), abs(s - b)), False


def test_bet_cases_match_an_exact_rational_oracle():
    q = lambda x: Fraction(str(x))  # noqa: E731 - the decimal the case states
    for case in gpf.bet_cases():
        f, d, clamped = exact_interpolation(q(case["s"]), [q(x) for x in case["menu"]])
        assert [float(x) for x in f] == case["f"], case["name"]
        assert float(d) == case["deviation"], case["name"]
        assert clamped == case["clamped"], case["name"]
    rows = golden("bet_translation_golden.json")["interpolation"]
    assert [r["name"] for r in rows] == [c["name"] for c in gpf.bet_cases()]


def test_prominence_cases_sit_on_and_just_above_the_boundary():
    cases = golden("bet_translation_golden.json")["prominence"]["cases"]
    assert [c["expected_prominent"] for c in cases] == [False, True]
    exact = [c["deviation_exact"] for c in cases]
    assert Fraction(exact[0]) == Fraction(1, 10) and Fraction(exact[1]) > Fraction(1, 10)
    for c in cases:
        (reason,) = c["expected"]["reasons"]
        assert reason["kind"] == "BetTranslation" and reason["prominent"] == c["expected_prominent"]


def test_bet_translation_reason_decides_prominence_from_the_exact_deviation():
    """Plan-3 final review F-M2: the oracle's `BetTranslation` takes `prominent` from an exact
    `Fraction` comparison (`d > 1/10`), as `prominence_cases` does, and keeps the float `deviation`
    for display only. A deviation just above 1/10 whose float rounds to the literal 0.1 is still
    prominent; exactly 1/10 is not; and a float deviation is refused, so no caller compares floats."""
    just_above = Fraction(1, 10) + Fraction(1, 10**20)
    assert float(just_above) == 0.1
    reason = gpf.bet_translation_reason(2, 1.6, [(1.5, 1.0)], just_above)
    assert reason["prominent"] is True and reason["deviation"] == 0.1
    exact = gpf.bet_translation_reason(2, 1.6, [(1.5, 1.0)], Fraction(16, 10) - Fraction(15, 10))
    assert exact["prominent"] is False and exact["deviation"] == 0.1 and isinstance(exact["deviation"], float)
    try:
        gpf.bet_translation_reason(2, 1.6, [(1.5, 1.0)], 1.6 - 1.5)
    except TypeError:
        pass
    else:
        raise AssertionError("a float deviation must be refused")


def test_legal_move_and_assembly_rows_conserve_mass():
    g = golden("bet_translation_golden.json")
    for case in g["legal_moves"]:
        into = math.fsum(r["probability"] for r in case["source"])
        out = math.fsum(a["probability"] for a in case["expected"]["actions"])
        assert abs(into - 1.0) < 1e-12 and abs(out - into) < 1e-12, case["name"]
    for case in g["assembly"]:
        e = case["expected"]
        if e["unsupported"] is None:
            known = math.fsum(a["frequency"] for a in e["actions"])
            assert abs(known + e["unresolved_mass"] - 1.0) < 1e-12, case["name"]
        if e["unresolved_mass"] > 0:
            assert e["headline"] is None and not any(a["headline"] for a in e["actions"]), case["name"]
    labels = {c["name"]: c["expected"]["headline"] for c in g["assembly"]}
    assert labels["t7_residual_no_headline"] is None
    assert labels["t7_branch_support_incomplete"] == "highest-frequency action, EV incomplete"
    assert labels["chart_frequency"] == "highest-frequency chart action"
    assert labels["unverified_source_frequency"] == "highest-frequency source action, EV reference unverified"
    assert labels["complete_ev"] == "highest EV"


def residual_reasons(case: dict) -> list:
    return [(r["cause"], r["residual_mass_pct"]) for r in case["expected"]["reasons"] if r["kind"] == "BranchResidual"]


def test_branch_residual_causes_follow_spec_8_4():
    """Ruling 19-I1: a `BranchResidual` names only a cause actually incurred, in spec section
    8.4's vocabulary -- `cap` for the persistent cap residual's share, `missing node <key>` for a
    positive-posterior branch whose key has no node -- one reason per distinct cause."""
    cases = {c["name"]: c for c in golden("bet_translation_golden.json")["assembly"]}
    # T7's residual row: both live branches have their node; only the cap residual has no strategy.
    t7 = cases["t7_residual_no_headline"]
    live = [b for b in t7["branches"] if not b["residual"] and b["stopped"] is None]
    nodes = {n["branch_id"]: n for n in t7["nodes"]}
    assert len(live) == 2 and all(nodes[b["id"]]["node"] is not None for b in live)
    assert [nodes[b["id"]]["key"] for b in live] == ["golden:after raise A", "golden:after raise B"]
    assert [b["q"] for b in t7["branches"] if b["residual"]] == [0.05]
    ((cause, pct),) = residual_reasons(t7)
    assert cause == "cap" and abs(pct - 5.0) < 1e-9
    # A node actually missing keeps its missing-node cause, naming that branch's own key.
    missing = cases["missing_node_partial"]
    ((cause, pct),) = residual_reasons(missing)
    absent = [n["key"] for n in missing["nodes"] if n["node"] is None]
    assert cause == f"missing node {absent[0]}" and abs(pct - 40.0) < 1e-9
    # Both causes at once: one reason each, with shares summing to the unresolved mass.
    both = cases["cap_beside_missing_node"]
    assert [c for c, _ in residual_reasons(both)] == ["cap", "missing node golden:k-missing"]
    assert abs(math.fsum(p for _, p in residual_reasons(both)) - 100 * both["expected"]["unresolved_mass"]) < 1e-9
