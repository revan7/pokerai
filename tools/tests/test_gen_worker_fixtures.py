import json
import gen_worker_fixtures as g


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
