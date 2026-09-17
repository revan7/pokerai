import json
import pathlib

import gen_worker_fixtures as g

FIXTURES_DIR = pathlib.Path(__file__).resolve().parents[2] / "fixtures" / "worker"


def test_combo_index_and_cards():
    assert g.card_id("2c") == 0 and g.card_id("As") == 51 and g.card_id("Qs") == 43
    assert g.combo_index(0, 1) == 0 and g.combo_index(51, 50) == 1325
    assert g.combo_index(g.card_id("Ah"), g.card_id("Ad")) == g.combo_index(g.card_id("Ad"), g.card_id("Ah"))


def test_parse_range_counts():
    assert sum(1 for w in g.parse_range("AA") if w > 0) == 6
    assert sum(1 for w in g.parse_range("AKs") if w > 0) == 4
    assert sum(1 for w in g.parse_range("54o:0.25") if w > 0) == 12
    assert abs(sum(g.parse_range("54o:0.25")) - 3.0) < 1e-9
    assert sum(1 for w in g.parse_range("96s+") if w > 0) == 12          # 96s, 97s, 98s
    assert sum(1 for w in g.parse_range("QQ-88") if w > 0) == 30         # five pairs
    assert sum(1 for w in g.parse_range("98o-65o") if w > 0) == 48       # four offsuit classes
    # Recomputed from the strings themselves. R8 addendum A.1 publishes 646 / 804 for these two ranges, but the
    # strings as published expand to 634 / 720 under this grammar (CO_CALL_3BET 194 and BTN_3BET 138 match A.1
    # exactly, so the parser is right and A.1's two figures are not reproducible from its own strings).
    # The strings are the definition; `crates/bench/src/gen_spots.rs` freezes the same two constants.
    assert sum(1 for w in g.parse_range(g.BTN_OPEN) if w > 0) == 634
    assert sum(1 for w in g.parse_range(g.BB_DEFEND) if w > 0) == 720
    assert sum(1 for w in g.parse_range(g.CO_CALL_3BET) if w > 0) == 194
    assert sum(1 for w in g.parse_range(g.BTN_3BET) if w > 0) == 138


def test_river_two_combo_definition():
    lines = [json.loads(l) for l in g.river_two_combo_lines()]
    solve = lines[0]
    assert solve["type"] == "solve" and solve["id"] == "41" and solve["board"] == ["Qs", "Jd", "7h", "3c", "2d"]
    oop, ip = solve["oop_range"], solve["ip_range"]
    assert len(oop) == 1326 and len(ip) == 1326
    assert sum(1 for w in oop if w == 1.0) == 6 and sum(oop) == 6.0
    assert sum(1 for w in ip if w == 1.0) == 3 and sum(1 for w in ip if w == 0.25) == 12
    assert solve["tree"]["template_id"] == "river_oracle_v1" and len(solve["tree"]["materialized"]) == 3
    assert solve["history"] == [{"kind": "check"}] and solve["memory_limit_bytes"] == 10737418240
    assert lines[1] == {"type": "cancel", "id": "42", "target": "41"} and lines[2] == {"type": "shutdown", "id": "48"}


def test_write_all_with_stub_materializer(tmp_path):
    def stub(template, pot, eff, prefix):
        return {"tree": {"template_id": template, "materialized": []}, "history": [], "decision_path": []}
    written = g.write_all(tmp_path, materializer=stub)
    names = sorted(p.name for p in written)
    assert names == ["flop_best_so_far.jsonl", "flop_cancel.jsonl", "lock_river.jsonl", "materialization_cases.jsonl", "river_two_combo.jsonl"]
    cases = [json.loads(l) for l in (tmp_path / "materialization_cases.jsonl").read_text().splitlines()]
    #        7 section-10.1 templates x 5 points, 7 facing stacks, facing_350_full, cap1, cap3, insert_73, basic_turn_std
    assert len(cases) == 7 * 5 + 7 + 1 + 1 + 1 + 1 + 1 == 47
    lock = [json.loads(l) for l in (tmp_path / "lock_river.jsonl").read_text().splitlines()]
    assert lock[0]["type"] == "lock" and lock[1]["type"] == "solve" and lock[0]["spot"] == lock[1]["spot"]
    rows = lock[0]["locks"][0]["probs"]
    assert len(rows) == 1326 and all(len(r) == 2 for r in rows)
    assert sum(1 for r in rows if r == [0.0, 1.0]) == 3 and sum(1 for r in rows if r == [0.8, 0.2]) == 12


def test_lock_fixture_oop_is_qq_and_66_with_signed_ev_witnesses():
    """Fix round 1, R1: spec section 13.2 `ev_convention_non_root_payoffs`
    (docs/superpowers/specs/2026-09-10-pokerai-assistant-design.md around line 734) requires OOP's
    lock-fixture range to be QQ and 66 so the locked call EVs are opposite-signed. The brief itself
    supplied AA (a plan defect, not an implementer deviation); the spec outranks the plan.

    OOP's QQ is blocked to the three non-board queens (Qc, Qd, Qh) -- the same three cards IP's
    locked QQ portion uses -- so any OOP QQ combo has zero compatible mass against IP's locked QQ
    and full compatible mass against IP's locked 54o, winning the entire compatible range:
    `equity * 300 - 100 = 1.0 * 300 - 100 = +200`. OOP's 66 is unblocked, so it faces IP's full
    locked range (QQ mass 3 + 54o mass 0.6 = 3.6 compatible), loses the QQ portion and wins only
    the 54o portion: `equity * 300 - 100 = (0.6 / 3.6) * 300 - 100 = -50`.
    """
    lines = [json.loads(l) for l in g.lock_river_lines()]
    lock, solve = lines[0], lines[1]
    oop, ip = solve["oop_range"], solve["ip_range"]
    rows = lock["locks"][0]["probs"]

    qq_cards = ["Qc", "Qd", "Qh"]  # the three non-board queens
    qq_combos = [g.combo_index(g.card_id(a), g.card_id(b)) for i, a in enumerate(qq_cards) for b in qq_cards[i + 1:]]
    six_cards = ["6c", "6d", "6h", "6s"]
    six_combos = [g.combo_index(g.card_id(a), g.card_id(b)) for i, a in enumerate(six_cards) for b in six_cards[i + 1:]]
    assert len(qq_combos) == 3 and len(six_combos) == 6
    assert sum(1 for w in oop if w == 1.0) == 9 and sum(oop) == 9.0
    assert all(oop[i] == 1.0 for i in qq_combos + six_combos)

    fives, fours = ["5c", "5d", "5h", "5s"], ["4c", "4d", "4h", "4s"]

    def qq_bet_mass(hero_a, hero_b):
        hero = {hero_a, hero_b}
        return sum(ip[g.combo_index(g.card_id(a), g.card_id(b))] * rows[g.combo_index(g.card_id(a), g.card_id(b))][1]
                   for i, a in enumerate(qq_cards) for b in qq_cards[i + 1:] if not ({a, b} & hero))

    def offsuit_54_bet_mass(hero_a, hero_b):
        hero = {hero_a, hero_b}
        return sum(ip[g.combo_index(g.card_id(a), g.card_id(b))] * rows[g.combo_index(g.card_id(a), g.card_id(b))][1]
                   for a in fives for b in fours if a[1] != b[1] and not ({a, b} & hero))

    def ev_call(hero_a, hero_b, hero_beats_qq):
        qq_mass, o54_mass = qq_bet_mass(hero_a, hero_b), offsuit_54_bet_mass(hero_a, hero_b)
        compatible = qq_mass + o54_mass
        winning = o54_mass + (qq_mass if hero_beats_qq else 0.0)
        return (winning / compatible) * 300 - 100

    assert abs(ev_call("Qc", "Qd", hero_beats_qq=True) - 200.0) < 1e-9      # OOP's QQ (trips): +200
    assert abs(ev_call("6c", "6d", hero_beats_qq=False) - (-50.0)) < 1e-9  # OOP's 66 (pair): -50


def test_committed_worker_fixtures_have_the_required_message_inventory():
    """Fix round 1, R2: every committed fixture under `fixtures/worker/` must be required by a
    test, opened by its real repository path -- never a `tmp_path` stand-in -- so deleting any one
    of the five fails this test outright rather than being silently unnoticed."""
    def load(name):
        data = (FIXTURES_DIR / f"{name}.jsonl").read_bytes()  # FileNotFoundError if absent: never skipped
        assert b"\r" not in data, f"{name}.jsonl must be LF-only"
        assert data.endswith(b"\n"), f"{name}.jsonl must end with a final newline"
        return [json.loads(l) for l in data.decode("utf-8").splitlines()]

    river = load("river_two_combo")
    assert [l["type"] for l in river] == ["solve", "cancel", "shutdown"]
    assert river[0]["id"] == "41" and river[1] == {"type": "cancel", "id": "42", "target": "41"}
    assert river[2] == {"type": "shutdown", "id": "48"}

    cancel = load("flop_cancel")
    assert [l["type"] for l in cancel] == ["solve", "cancel"]
    assert cancel[0]["id"] == "43" and cancel[1] == {"type": "cancel", "id": "44", "target": "43"}

    best = load("flop_best_so_far")
    assert [l["type"] for l in best] == ["solve"] and best[0]["id"] == "45"

    lock = load("lock_river")
    assert [l["type"] for l in lock] == ["lock", "solve", "shutdown"]
    assert lock[0]["id"] == "47" and lock[1]["id"] == "51" and lock[2] == {"type": "shutdown", "id": "52"}
    assert lock[0]["spot"] == lock[1]["spot"]

    cases = load("materialization_cases")
    assert len(cases) == 47
    names = [c["case"] for c in cases]
    assert len(set(names)) == 47

    def stub(template, pot, eff, prefix):
        return {"tree": {}, "history": [], "decision_path": []}
    expected_names = [json.loads(l)["case"] for l in g.materialization_cases(stub)]
    assert names == expected_names, "committed case order must match the generator's case order"


def test_generator_output_matches_committed_bytes_for_independently_generatable_fixtures():
    """Fix round 1, R2: `river_two_combo_lines()` and `lock_river_lines()` need no `bench`
    subprocess, so their current output can be compared byte-for-byte against the committed
    fixture bytes with no stub/tmp_path indirection -- a generator-only change (e.g. a range
    string edited without regenerating) fails this test even though the brief's own
    count/structure assertions above would still pass."""
    river_expected = ("\n".join(g.river_two_combo_lines()) + "\n").encode("utf-8")
    assert (FIXTURES_DIR / "river_two_combo.jsonl").read_bytes() == river_expected

    lock_expected = ("\n".join(g.lock_river_lines()) + "\n").encode("utf-8")
    assert (FIXTURES_DIR / "lock_river.jsonl").read_bytes() == lock_expected
