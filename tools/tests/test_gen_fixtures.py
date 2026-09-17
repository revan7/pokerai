import collections
import json

from gen_fixtures import CONFIGS, generate


def conserved(hand: dict) -> bool:
    total = sum(hand["stacks_start"])
    for step in hand["steps"]:
        a = step["after"]
        pots = sum(p["amount"] for p in a["pots"])
        if sum(a["stacks"]) + sum(a["committed"]) + pots != total:
            return False
        if a["pot_total"] != sum(a["committed"]) + pots:
            return False
    return True


def test_generated_set_covers_spec_classes():
    hands, dropped = generate(200, 1)
    assert len(hands) == 200
    assert dropped <= 10, "the PokerKit reopening divergence must stay rare"
    assert sum(1 for h in hands if h["config"]["straddle_chips"]) >= 80
    assert sum(1 for h in hands if h["stats"]["allin_players"] >= 2) >= 50
    assert sum(1 for h in hands if h["stats"]["allin_players"] >= 3) >= 15
    assert sum(1 for h in hands if h["stats"]["short_allins"] >= 1) >= 25
    assert sum(1 for h in hands if h["returned"]) >= 100
    # `max_pots` counts pot layers after `normalize_pots`, so it counts *genuine* side pots
    # (distinct eligible sets), not PokerKit's raw contribution levels. 20 is a floor, not the
    # measured count: 65 hands have two or more all-in players and 25 have three or more.
    assert sum(1 for h in hands if h["stats"]["max_pots"] >= 2) >= 20
    finals = collections.Counter(h["steps"][-1]["after"]["final"] for h in hands)
    assert finals["showdown_reached"] >= 15
    assert finals["folded_out"] >= 40
    assert finals["all_in_runout"] >= 40
    assert {len(h["dealt"]) for h in hands} == {3, 4, 5, 6}
    seen = {(h["config"]["sb_chips"], h["config"]["bb_chips"], h["config"]["straddle_chips"]) for h in hands}
    assert seen == {(sb, bb, s) for sb, bb, s, _, _ in CONFIGS}
    assert all(conserved(h) for h in hands)
    assert all(h["steps"][-1]["after"]["phase"] == "complete" for h in hands)


def test_generation_is_deterministic():
    assert json.dumps(generate(5, 1)[0]) == json.dumps(generate(5, 1)[0])


def test_straddle_min_raise_is_twice_the_straddle():
    hands, _ = generate(40, 1)
    checked = 0
    for h in hands:
        s = h["config"]["straddle_chips"]
        if not s:
            continue
        first = h["steps"][0]
        assert first["kind"] == "action"
        assert first["seat"] == h["dealt"][3], "HJ opens over the straddle"
        # the legal set the opener faced is the one PokerKit reported before the first action; reconstruct it from the config
        after = first["after"]
        if first["action"]["kind"] in ("call", "fold") and after["to_act"] is not None:
            rz = after["legal"]["raise"]
            assert rz is None or rz["min_to"] == 2 * s
            checked += 1
    assert checked >= 5


def test_fold_is_legal_only_when_facing_a_wager():
    hands, _ = generate(60, 1)
    for h in hands:
        for step in h["steps"]:
            a = step["after"]
            if a["legal"] is None:
                continue
            idx = h["dealt"].index(a["to_act"])
            facing = max(a["committed"])
            assert a["legal"]["fold"] == (facing > a["committed"][idx])
            cc = a["legal"]["check_or_call"]
            assert cc is not None and cc["cost"] == min(facing - a["committed"][idx], a["stacks"][idx])


def test_pot_layers_follow_core_model_merging():
    """The fixture uses core-model's layering rule, not PokerKit's raw levels."""
    hands, _ = generate(60, 1)
    merged_cases = 0
    for h in hands:
        for step in h["steps"]:
            pots = step["after"]["pots"]
            assert all(p["eligible"] for p in pots), "an empty eligible set must be merged away"
            for a, b in zip(pots, pots[1:]):
                assert a["eligible"] != b["eligible"], f"adjacent equal eligibility must be merged: {pots}"
            merged_cases += 1
    assert merged_cases > 0
