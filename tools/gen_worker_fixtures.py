#!/usr/bin/env python
"""Generates fixtures/worker/*.jsonl from the definitions of spec sections 4.5 and 13.2.

Card id = rank * 4 + suit (ranks 2..A = 0..12, suits c,d,h,s = 0..3); combo index = hi*(hi-1)/2 + lo (spec 4.1).
The flop trees come from `bench materialize` (engine materializer); the river oracle tree is written by hand
exactly as the section 4.5 wire example.
"""
import json
import pathlib
import subprocess
import sys

RANKS = "23456789TJQKA"
SUITS = "cdhs"
GIB = 1024 ** 3
SPOT_RIVER = "3f9c0a7d2b1e4c6f8a9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f"
SPOT_FLOP = "5a1c9e3b7d2f4a6c8e0b1d3f5a7c9e2b4d6f8a0c1e3b5d7f9a2c4e6b8d0f1a3c"
SPOT_LOCK = "9d0b2c4e6f8a1b3c5d7e9f0a2b4c6d8e0f1a3b5c7d9e1f3f9c0a7d2b1e4c6f8a"
# R8 addendum A.1 range strings, copied verbatim; the same four constants are frozen in crates/bench/src/gen_spots.rs.
BTN_OPEN = "22+,A2s+,K2s+,Q2s+,J3s+,T6s+,96s+,86s+,75s+,65s,54s,43s,A2o+,K7o+,Q8o+,J8o+,T8o+,98o"
BB_DEFEND = "JJ-22,AJs-A2s,K2s+,Q2s+,J2s+,T2s+,92s+,84s+,74s+,63s+,53s+,43s,32s,AJo-A2o,K5o+,Q7o+,J8o+,T8o+,98o,97o,87o,76o"
CO_CALL_3BET = "QQ-22,AKs-ATs,A5s-A4s,KQs-KTs,QJs-QTs,JTs,J9s,T9s,T8s,98s,87s,76s,65s,54s,AQo-AJo,KQo,KJo"
BTN_3BET = "TT+,AJs+,A5s-A2s,KJs+,QJs,JTs,T9s,76s,65s,54s,AQo+,KQo,KJo"
BASIC_OOP = "66+,A8s+,A5s-A4s,AJo+,K9s+,KQo,QTs+,JTs,96s+,85s+,75s+,65s,54s"
BASIC_IP = "QQ-22,AQs-A2s,ATo+,K5s+,KJo+,Q8s+,J8s+,T7s+,96s+,86s+,75s+,64s+,53s+"


def card_id(s):
    return RANKS.index(s[0]) * 4 + SUITS.index(s[1])


def combo_index(a, b):
    lo, hi = sorted((a, b))
    return hi * (hi - 1) // 2 + lo


def class_combos(r1, r2, suited):
    """Card-id pairs of a 169-class: r1 == r2 pair; suited True/False/None (None = both)."""
    out = []
    if r1 == r2:
        return [(r1 * 4 + s, r1 * 4 + t) for s in range(4) for t in range(s + 1, 4)]
    for s in range(4):
        for t in range(4):
            if suited is True and s != t or suited is False and s == t:
                continue
            out.append((r1 * 4 + s, r2 * 4 + t))
    return out


def parse_token(tok):
    """One Pio group -> list of (r1, r2, suited); grammar of the postflop-solver Range doc comment."""
    if len(tok) == 4 and tok[1] in SUITS and tok[3] in SUITS:          # specific combo "AsKs"
        return [("specific", card_id(tok[:2]), card_id(tok[2:]))]
    plus = tok.endswith("+")
    body = tok[:-1] if plus else tok
    if "-" in body:
        hi, lo = body.split("-")
        r1, r2 = RANKS.index(hi[0]), RANKS.index(hi[1])
        l1, l2 = RANKS.index(lo[0]), RANKS.index(lo[1])
        suited = None if len(hi) == 2 else hi[2] == "s"
        if r1 == r2:                                                    # QQ-88
            return [(r, r, None) for r in range(l2, r1 + 1)]
        if r1 == l1:                                                    # A9s-A6s
            return [(r1, r, suited) for r in range(l2, r2 + 1)]
        return [(r1 - k, r2 - k, suited) for k in range(0, r1 - l1 + 1)]  # 98o-65o
    r1, r2 = RANKS.index(body[0]), RANKS.index(body[1])
    suited = None if len(body) == 2 else body[2] == "s"
    if not plus:
        return [(r1, r2, suited)]
    if r1 == r2:
        return [(r, r, None) for r in range(r1, 13)]
    return [(r1, r, suited) for r in range(r2, r1)]


def parse_range(text):
    vec = [0.0] * 1326
    for group in (g.strip() for g in text.split(",") if g.strip()):
        tok, weight = (group.split(":") + ["1"])[:2]
        w = float(weight)
        for entry in parse_token(tok):
            if entry[0] == "specific":
                vec[combo_index(entry[1], entry[2])] = w
                continue
            r1, r2, suited = entry
            for a, b in class_combos(r1, r2, suited):
                vec[combo_index(a, b)] = w
    return vec


def block(vec, board):
    dead = {card_id(c) for c in board}
    out = list(vec)
    for hi in range(1, 52):
        for lo in range(hi):
            if hi in dead or lo in dead:
                out[combo_index(lo, hi)] = 0.0
    return out


def action(kind, to=None):
    return {"kind": kind} if to is None else {"kind": kind, "to": to}


RIVER_ORACLE_TREE = {
    "rules_version": 3, "template_id": "river_oracle_v1", "root_street": "river",
    "menus": {"river": {"oop": {"bet": [], "raise": []}, "ip": {"bet": [1.0], "raise": []}}},
    "add_allin_threshold": 0.0, "force_allin_threshold": 0.0, "merging_threshold": 0.0, "wager_cap": 1, "inserted": [],
    "materialized": [
        {"path": [], "street": "river", "actor": "oop", "actions": [action("check")], "terminal_pots": [None]},
        {"path": [0], "street": "river", "actor": "ip", "actions": [action("check"), action("allin", 100)], "terminal_pots": [100, None]},
        {"path": [0, 1], "street": "river", "actor": "oop", "actions": [action("fold"), action("call")], "terminal_pots": [100, 300]},
    ],
}
RIVER_BOARD = ["Qs", "Jd", "7h", "3c", "2d"]


def solve_line(id_, spot, board, oop, ip, pot, stack, tree, history, target_bp, deadline_ms, margin_ms, rake=(0.0, 0)):
    return json.dumps({"type": "solve", "id": id_, "spot": spot, "board": board, "oop_range": block(oop, board), "ip_range": block(ip, board),
                       "pot": pot, "stack_oop": stack, "stack_ip": stack, "rake_rate": rake[0], "rake_cap_mchips": rake[1], "tree": tree,
                       "history": history, "target_bp": target_bp, "deadline_ms": deadline_ms, "extraction_margin_ms": margin_ms,
                       "memory_limit_bytes": 10 * GIB, "background": False}, separators=(",", ":"))


def river_two_combo_lines():
    yield solve_line("41", SPOT_RIVER, RIVER_BOARD, parse_range("AA"), parse_range("QQ,54o:0.25"), 100, 100, RIVER_ORACLE_TREE, [action("check")], 10, 1500, 200)
    yield json.dumps({"type": "cancel", "id": "42", "target": "41"}, separators=(",", ":"))
    yield json.dumps({"type": "shutdown", "id": "48"}, separators=(",", ":"))


def lock_river_lines():
    """IP's river node locked to bet QQ 100% and 54o 20% (ev_convention_non_root_payoffs); OOP holds AA and 66."""
    rows = [[0.0, 0.0] for _ in range(1326)]
    for i, w in enumerate(block(parse_range("QQ"), RIVER_BOARD)):
        if w > 0:
            rows[i] = [0.0, 1.0]
    for i, w in enumerate(block(parse_range("54o"), RIVER_BOARD)):
        if w > 0:
            rows[i] = [0.8, 0.2]
    yield json.dumps({"type": "lock", "id": "47", "spot": SPOT_LOCK, "locks": [{"path": [action("check")], "actor": "ip", "probs": rows}]}, separators=(",", ":"))
    yield solve_line("51", SPOT_LOCK, RIVER_BOARD, parse_range("AA,66"), parse_range("QQ,54o:0.25"), 100, 100, RIVER_ORACLE_TREE, [action("check")], 10, 1500, 200)
    yield json.dumps({"type": "shutdown", "id": "52"}, separators=(",", ":"))


def bench_materialize(template, pot, eff, prefix):
    """Runs `bench materialize`; prefix is a list of [actor, action] pairs (actor 'oop'|'ip')."""
    items = ",".join(f"{a}:{x['kind']}" + (f":{x['to']}" if "to" in x else "") for a, x in prefix)
    cmd = ["cargo", "run", "-q", "--release", "-p", "bench", "--", "materialize", "--template", template, "--pot", str(pot), "--eff", str(eff), "--prefix", items]
    out = subprocess.run(cmd, check=True, capture_output=True, text=True, cwd=pathlib.Path(__file__).resolve().parents[1]).stdout
    return json.loads(out)


FLOP_BOARD = ["Qs", "Jh", "2h"]


def flop_lines(materializer):
    m = materializer("flop_fast_v1", 180, 910, [])
    oop, ip = parse_range(BASIC_OOP), parse_range(BASIC_IP)
    cancel = [solve_line("43", SPOT_FLOP, FLOP_BOARD, oop, ip, 180, 910, m["tree"], m["history"], 50, 30000, 600),
              json.dumps({"type": "cancel", "id": "44", "target": "43"}, separators=(",", ":"))]
    best = [solve_line("45", SPOT_FLOP, FLOP_BOARD, oop, ip, 180, 910, m["tree"], m["history"], 1, 2000, 600)]
    return cancel, best


# All seven section-10.1 templates receive the section-13.2 five-point sweep.
TEMPLATES = ["flop_fast_v1", "flop_min_v1", "flop_full_v1",
             "turn_std_v1", "turn_min_v1", "river_std_v1", "river_min_v1"]


def materialization_cases(materializer):
    cases = []
    for t in TEMPLATES:
        for pot, eff in [(100, 100), (100, 150), (180, 910), (100, 149), (100, 151)]:
            cases.append((f"{t}_{pot}_{eff}", t, pot, eff, []))
    for eff in [350, 400, 401, 340, 341, 240, 100]:
        cases.append((f"facing_{eff}", "facing_test_v1", 100, eff, [["oop", action("bet", 100)]]))
    cases.append(("facing_350_full", "facing_test_v1", 100, 350, []))
    cases.append(("cap1_two_wagers", "flop_min_v1", 100, 500, [["oop", action("bet", 40)], ["ip", action("raise", 120)]]))
    cases.append(("cap3_three_wagers", "turn_std_v1", 100, 1000, [["oop", action("bet", 33)], ["ip", action("raise", 83)], ["oop", action("raise", 208)]]))
    cases.append(("insert_73", "flop_fast_v1", 100, 500, [["oop", action("bet", 73)]]))
    cases.append(("basic_turn_std", "turn_std_v1", 200, 900, []))      # the pinned example spot of Task 15
    for case, t, pot, eff, prefix in cases:
        m = materializer(t, pot, eff, prefix)
        yield json.dumps({"case": case, "template_id": t, "pot": pot, "eff": eff, "prefix": prefix, "tree": m["tree"], "history": m["history"], "decision_path": m["decision_path"]}, separators=(",", ":"))


def write_all(out_dir, materializer=bench_materialize):
    out_dir = pathlib.Path(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    cancel, best = flop_lines(materializer)
    files = {"river_two_combo.jsonl": list(river_two_combo_lines()), "lock_river.jsonl": list(lock_river_lines()),
             "flop_cancel.jsonl": cancel, "flop_best_so_far.jsonl": best, "materialization_cases.jsonl": list(materialization_cases(materializer))}
    written = []
    for name, lines in files.items():
        p = out_dir / name
        p.write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
        written.append(p)
    return written


if __name__ == "__main__":
    target = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[1] / "fixtures" / "worker"
    for p in write_all(target):
        print(f"wrote {p} ({p.stat().st_size} bytes)")
