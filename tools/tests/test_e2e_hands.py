"""Inventory tests for the fifty recorded end-to-end benchmark inputs (plan 4 Task 18,
`tools/e2e_hands.py`). These pin the frozen record count, class layout and per-record schema, the
audited default hero and its chart support, the legality of every event history (replayed through
PokerKit), and the per-record coverage/lock mapping. Chart coverage is checked against the
already-frozen `bench/spots/sources.json` lock (Task 17) and the two chart envelopes whose sha256 it
pins; the fault/rejection reasons are checked against the exact strings the brief and the
orchestrator's fix-round rulings assign, never harvested from a live run.

Task 19 writes/verifies the fifty fixture files and a manifest from `records()`, and Task 21 replays
them through the real engine -- both depend on this schema staying exactly what is asserted here.
"""
import hashlib
import json
import os
import subprocess
import sys
from copy import deepcopy
from fractions import Fraction
from pathlib import Path

import pytest

import e2e_hands
from chart_ingest import class_names
from chart_sources import history_key
from e2e_hands import FAULTS, records, write_e2e, check_e2e
from gen_fixtures import replay_record

REPO = Path(__file__).resolve().parents[2]
ROOT = REPO / "fixtures/hands/e2e"
PC, RC = "pokercoaching_100", "rangeconverter_200"

SPECIAL_EXPECTATIONS = {
    # id: (class, coverage, unsupported reason or None)
    "023": ("straddle_mapping", "Approximate", None),
    "024": ("straddle_mapping", "Approximate", None),
    "025": ("straddle_mapping", "Approximate", None),
    "026": ("straddle_mapping", "Approximate", None),
    "027": ("projection_rejected", "Unsupported", "UnsupportedHistory"),
    "028": ("multiway", "Unsupported", "MultiwayEv"),
    "029": ("third_allin", "Unsupported", "MultiwayEv"),
    "030": ("multiway_side_pot", "Unsupported", "MultiwayEv"),
    "031": ("missing_preflop", "Unsupported", "MissingPreflopNode"),
    "032": ("missing_preflop", "Unsupported", "MissingPreflopNode"),
    "033": ("missing_preflop", "Unsupported", "MissingPreflopNode"),
    "034": ("hero_out_of_support", "Unsupported", "HeroComboOutOfSupport"),
    "035": ("hero_out_of_support", "Unsupported", "HeroComboOutOfSupport"),
    "036": ("hero_out_of_support", "Unsupported", "HeroComboOutOfSupport"),
}

FAULT_REASON = {
    "Oom": "EngineError", "Eof": "EngineError", "Malformed": "EngineError",
    "Oversized": "EngineError", "TreeMismatch": "EngineError",
    "BlockedCacheIo": "DeadlineExceeded", "SlowAllocation": "DeadlineExceeded",
    "NoIteration": "DeadlineExceeded", "ClockJump": "DeadlineExceeded",
    "SuspendResume": "DeadlineExceeded",
}

REQUIRED_KEYS = {
    "version", "config", "button", "dealt", "stacks", "hero", "hero_cards",
    "events", "fault", "class", "expected", "id",
}

# --- the frozen lock and the two chart envelopes it pins (read only, hash-checked) ---


def _lock() -> dict:
    return json.loads((REPO / "bench/spots/sources.json").read_text(encoding="utf-8"))


def _charts() -> dict:
    """`{bundle name: {history key: node}}` for both frozen envelopes, each verified against the
    lock's sha256 before it is trusted."""
    out = {}
    for bundle in _lock()["bundles"]:
        raw = (REPO / "fixtures/charts" / f"{bundle['name']}.json").read_bytes()
        assert hashlib.sha256(raw).hexdigest() == bundle["sha256"], bundle["name"]
        out[bundle["name"]] = {history_key(n["history"]): n for n in json.loads(raw)["nodes"]}
    return out


def _weight(node: dict, step: str, class_index: int) -> float:
    steps = [a["step"] for a in node["actions"]]
    return node["weights"][steps.index(step)][class_index]


# --- spec 8.3 depth rule, implemented here independently of e2e_hands as a cross-check ---


def _spec_bundle(record: dict, actor: int, eligible: list, lock: dict) -> str:
    """Spec 8.3: depth = min(actor stack, max other eligible stack) / unit, unit = straddle chips
    when a straddle is configured else bb_chips; nearest acquired depth, ties deeper; above the
    deepest acquired depth (200) clamps to it."""
    unit = record["config"]["straddle"] or record["config"]["bb_chips"]
    others = [record["stacks"][s] for s in eligible if s != actor]
    depth = Fraction(min(record["stacks"][actor], max(others)), unit)
    by_depth = {b["manifest"]["depth_bb"]: b["name"] for b in lock["bundles"]}
    acquired = sorted(by_depth)
    used = acquired[-1] if depth >= acquired[-1] else min(acquired, key=lambda d: (abs(d - depth), -d))
    return by_depth[used]


def _preflop_lookups(record: dict, lock: dict) -> list:
    """`[(seat, bundle, prefix key)]` for every preflop action of the record, in acting order."""
    out, letters, folded = [], "", set()
    for e in record["events"]:
        if e["type"] != "action" or e["street"] != "preflop":
            continue
        eligible = [s for s in record["dealt"] if s not in folded]
        out.append((e["seat"], _spec_bundle(record, e["seat"], eligible, lock), letters))
        letters += history_key([(None, e["action"]["kind"], None)])
        if e["action"]["kind"] == "fold":
            folded.add(e["seat"])
    return out


def _board_cards(record: dict) -> list:
    boards = [e["cards"] for e in record["events"] if e["type"] == "board"]
    final = boards[-1] if boards else ""
    return [final[i:i + 2] for i in range(0, len(final), 2)]


# --- inventory ---


def test_fifty_frozen_inputs():
    rows = records()
    assert len(rows) == 50
    assert [r["id"] for r in rows] == [f"{n:03}" for n in range(1, 51)]
    assert sum(r["expected"]["numeric_ev"] for r in rows) == 22
    assert [r["class"] for r in rows[:8]] == ["hu_flop_srp"] * 8
    assert [r["class"] for r in rows[8:14]] == ["hu_turn"] * 6
    assert [r["class"] for r in rows[14:20]] == ["hu_river"] * 6
    assert [r["class"] for r in rows[20:22]] == ["projection_admitted"] * 2
    assert all(r["version"] == 1 and r["events"] for r in rows)


def test_fault_records_are_the_last_fourteen():
    rows = records()
    assert [r["class"] for r in rows[36:]] == ["failure_injection"] * 14
    assert [r["fault"] for r in rows[36:46]] == list(FAULTS)


def test_every_record_has_the_full_v1_schema():
    for r in records():
        assert set(r.keys()) == REQUIRED_KEYS, r["id"]
        assert r["version"] == 1
        assert len(r["stacks"]) == len(r["dealt"]) == 6
        assert all(isinstance(s, int) for s in r["stacks"]), r["id"]
        assert isinstance(r["button"], int)
        assert r["config"]["bb_chips"] > 0 and r["config"]["sb_chips"] > 0
        assert isinstance(r["config"]["rake"]["cap_mchips"], int)


# --- I1: the audited default hero and the generation-time support guard ---

# The seven hero preflop decisions that records using the default hero make at a present node
# (fix round 1, I1 ruling): BB call vs a BTN open (FFFRF) and vs a CO open (FFRFF) in both
# bundles (001-020, 037-050), BTN cold-call vs UTG open (RFF, 022), the straddler's call as the
# virtual BB vs the virtual-UTG open (RFFFF, 023-026) and the UTG open (root, 027), all at 100bb.
HERO_REQUIREMENTS = [
    (PC, "FFFRF", "BB", "call"), (RC, "FFFRF", "BB", "call"),
    (PC, "FFRFF", "BB", "call"), (RC, "FFRFF", "BB", "call"),
    (PC, "RFF", "BTN", "call"), (PC, "RFFFF", "BB", "call"), (PC, "", "UTG", "raise"),
]
# The committed evidence for class 66 at each requirement above, in the same order.
HERO_66_WEIGHTS = [1.0, 0.5, 1.0, 1.0, 1.0, 1.0, 1.0]
HERO_BOARD_CARDS = {"Kh", "7d", "2c", "Jh", "9h", "6c", "8s", "8d", "3c", "4d", "2s"}


def test_default_hero_is_the_audited_supported_class():
    assert e2e_hands.DEFAULT_HERO_CLASS == "66"
    assert e2e_hands.DEFAULT_HERO_CARDS == "6s6h"
    assert e2e_hands.hand_class(e2e_hands.DEFAULT_HERO_CARDS) == "66"
    charts, names = _charts(), class_names()
    for bundle, key, actor, _ in HERO_REQUIREMENTS:
        assert charts[bundle][key]["actor"] == actor, (bundle, key)
    weights = {
        cls: [_weight(charts[b][k], step, ci) for b, k, _, step in HERO_REQUIREMENTS]
        for ci, cls in enumerate(names)
    }
    qualifying = [cls for cls in names if all(w > 0 for w in weights[cls])]
    # Only two classes have positive weight at all seven decisions; 66 is mixed at one cell
    # (RangeConverter FFFRF: call 0.5 / raise 0.5), 77 at two, so 66 is the committed choice.
    assert qualifying == ["77", "66"]
    assert weights["66"] == HERO_66_WEIGHTS
    assert sum(w != 1.0 for w in weights["77"]) == 2
    assert sum(w != 1.0 for w in weights["66"]) == 1


def test_hero_cards_are_two_distinct_cards_disjoint_from_every_board():
    all_boards = set()
    for r in records():
        hero = [r["hero_cards"][:2], r["hero_cards"][2:]]
        assert len(r["hero_cards"]) == 4 and hero[0] != hero[1], r["id"]
        board = _board_cards(r)
        assert not set(hero) & set(board), (r["id"], hero, board)
        all_boards |= set(board)
    assert all_boards == HERO_BOARD_CARDS
    assert not {"6s", "6h"} & HERO_BOARD_CARDS


def test_records_that_use_the_default_hero_carry_the_constant():
    for r in records():
        if r["class"] in ("missing_preflop", "hero_out_of_support"):
            continue
        assert r["hero_cards"] == e2e_hands.DEFAULT_HERO_CARDS, r["id"]


def test_generation_runs_the_hero_support_guard(monkeypatch):
    # Generation itself fails for review when the fixed hero is out of support; the plan's
    # original 8h7h (87s) has zero BB-call weight at PokerCoaching FFFRF.
    monkeypatch.setattr(e2e_hands, "DEFAULT_HERO_CARDS", "8h7h")
    with pytest.raises(ValueError, match=r"record 001 .*pokercoaching_100.*'FFFRF'.*call"):
        records()


@pytest.mark.parametrize("record_id, key, action", [
    ("001", "FFFRF", "call"),   # BB call vs BTN open
    ("022", "RFF", "call"),     # BTN cold-call vs UTG open
    ("027", "", "raise"),       # UTG open
])
def test_guard_names_record_bundle_key_and_action_on_zero_support(record_id, key, action):
    row = next(r for r in records() if r["id"] == record_id)
    row["hero_cards"] = "8h7h"
    with pytest.raises(ValueError, match=rf"record {record_id} .*pokercoaching_100.*{key!r}.*{action}"):
        e2e_hands.verify_hero_support([row])


def test_guard_requires_zero_weight_for_the_out_of_support_class():
    rows = [r for r in records() if r["class"] == "hero_out_of_support"]
    assert [r["id"] for r in rows] == ["034", "035", "036"]
    assert all(r["hero_cards"] == "7c2d" for r in rows)
    e2e_hands.verify_hero_support(rows)
    for r in rows:
        r["hero_cards"] = "AhAd"
        with pytest.raises(ValueError, match=rf"record {r['id']} "):
            e2e_hands.verify_hero_support([r])


def test_guard_stops_at_the_first_absent_node():
    # 021 and 030 (UTG short-stacked): SB's node at RFFC is absent, so BB's call after it is
    # unconditioned (spec 9.3) and a zero-weight class (72o) is not rejected there.
    rows = {r["id"]: r for r in records()}
    for record_id in ("021", "030"):
        row = rows[record_id]
        row["hero_cards"] = "7c2d"
        e2e_hands.verify_hero_support([row])


def test_guard_accepts_all_fifty_records():
    e2e_hands.verify_hero_support(records())


# --- I2: every event history is legal, replayed through PokerKit ---

# The replay itself (PokerKit oracle, deal wrapper, legal-action triple, spec-4.3 reopening
# tracker) lives in `gen_fixtures.replay_record` (plan 4 Task 19) -- lifted from what used to be a
# private, test-only copy here, so there is exactly one PokerKit replay implementation.


def test_every_record_replays_legally_through_pokerkit():
    for r in records():
        trace = replay_record(r)
        assert trace.legal_at_every_step, r["id"]
        assert trace.state.status and trace.final_actor is not None, r["id"]
        assert trace.final_actor == r["hero"], (r["id"], trace.final_actor, r["hero"])


def test_opening_flop_bet_of_the_projection_records_is_one_big_blind():
    rows = {r["id"]: r for r in records()}
    for record_id in ("021", "022", "027", "030"):
        r = rows[record_id]
        first = next(e for e in r["events"] if e["type"] == "action" and e["street"] == "flop")
        assert first["action"] == {"kind": "bet", "to": r["config"]["bb_chips"]}, record_id
        assert r["config"]["bb_chips"] == 100


# id: (folded_this_street, dead_this_street, hero's cost to call, minimum raise-to)
PROJECTION_NUMBERS = {"021": (1, 0, 200, 500), "022": (1, 100, 200, 700)}


def test_admitted_projection_numbers_reproduce_in_pokerkit():
    rows = {r["id"]: r for r in records()}
    for record_id, (folded, dead, cost, min_to) in PROJECTION_NUMBERS.items():
        r = rows[record_id]
        trace = replay_record(r)
        state = trace.state
        flop_folds = [e for e in r["events"]
                      if e["type"] == "action" and e["street"] == "flop" and e["action"]["kind"] == "fold"]
        assert len(flop_folds) == folded, record_id
        # mid-street, a folded seat's uncollected flop wager is still in `bets`: dead money
        assert sum(b for b, live in zip(state.bets, state.statuses) if not live) == dead, record_id
        assert state.checking_or_calling_amount == cost, record_id
        assert state.min_completion_betting_or_raising_to_amount == min_to, record_id


def test_every_record_replays_legally_in_pokerkit():
    for row in records():
        trace = replay_record(row)
        assert trace.legal_at_every_step, row["id"]
        assert trace.final_actor == row["hero"] or row["expected"]["unsupported"], row["id"]


# --- Task 19 fix round 1 (I4): an independent accounting model compared with PokerKit ---
#
# `replay_record` keeps its own ledger (posts and straddle, wagers, street and hand commitments,
# stacks, collected and total pot, actor, dealt board) and asserts it equals PokerKit's state
# after the posts and after every event. The expectations below are derived by hand from each
# record's events, independently of both the ledger and PokerKit.


def _copied(record_id: str) -> dict:
    return deepcopy(next(r for r in records() if r["id"] == record_id))


def test_replay_compares_the_accounting_after_the_posts_and_every_event():
    for r in records():
        assert replay_record(r).states_compared == len(r["events"]) + 1, r["id"]


# Seats BTN0 SB1 BB2 UTG3 HJ4 CO5. "street" is the uncollected street commitment per seat.
_ZERO = {seat: 0 for seat in range(6)}
FINAL_ACCOUNTING = {
    # Straddle 1/2/4, 200 chips each: posts 1+2+4; HJ raises to 10, CO/BTN/SB/BB fold, the
    # straddler calls 10; collected 1 + 2 + 10 + 10 = 23; UTG (hero) first to act on the flop.
    "023": {"actor": 3, "stacks": {0: 200, 1: 199, 2: 198, 3: 190, 4: 190, 5: 200},
            "street": _ZERO, "collected": 23, "total": 23, "board": "Kh7d2c"},
    # Projection (paid): preflop collects 50 + 3 x 250 = 800; on the flop BB has 500 out, UTG's
    # 100 is dead (folded, still uncollected) and BTN (hero) faces 500 with 300 in.
    "022": {"actor": 0, "stacks": {0: 9450, 1: 9950, 2: 9250, 3: 9650, 4: 10000, 5: 10000},
            "street": {0: 300, 1: 0, 2: 500, 3: 100, 4: 0, 5: 0}, "collected": 800, "total": 1700,
            "board": "Kh7d2c"},
    # Third all-in: UTG (250 chips) all-in preflop, BTN and BB call, SB's 50 dead: 800.
    "029": {"actor": 2, "stacks": {0: 9750, 1: 9950, 2: 9750, 3: 0, 4: 10000, 5: 10000},
            "street": _ZERO, "collected": 800, "total": 800, "board": "Kh7d2c"},
    # Side pot with a short all-in: UTG (300) raises to 250 preflop (800 collected), then calls
    # BB's 100 flop bet all-in for 50 and BTN calls 100 (1050, nothing uncalled), turn 100/100
    # (1250); river dealt, BB to act.
    "030": {"actor": 2, "stacks": {0: 9550, 1: 9950, 2: 9550, 3: 0, 4: 10000, 5: 10000},
            "street": _ZERO, "collected": 1250, "total": 1250, "board": "Kh7d2c4d2s"},
}


@pytest.mark.parametrize("record_id", sorted(FINAL_ACCOUNTING))
def test_replay_accounting_matches_hand_derived_numbers(record_id):
    want = FINAL_ACCOUNTING[record_id]
    r = _copied(record_id)
    view = replay_record(r).ledger.view()
    start = {seat: r["stacks"][seat] for seat in range(6)}
    assert view["actor"] == want["actor"] == r["hero"]
    assert view["stacks"] == want["stacks"]
    assert view["street commitments"] == want["street"]
    assert view["hand commitments"] == {s: start[s] - want["stacks"][s] for s in range(6)}
    assert view["collected pot"] == want["collected"]
    assert view["total pot"] == want["total"] == want["collected"] + sum(want["street"].values())
    assert view["board"] == want["board"]
    assert sum(want["stacks"].values()) + want["total"] == sum(start.values())


def test_replay_returns_the_uncalled_wager_at_a_street_collection():
    # A private variant of 030 (never frozen): BTN folds to UTG's 50 all-in call of BB's 100 flop
    # bet, so 50 of BB's bet is uncalled and goes back to BB when the flop is collected.
    r = _copied("030")
    r["events"] = r["events"][:9] + [{"type": "action", "seat": 0, "street": "flop", "action": {"kind": "fold"}}]
    trace = replay_record(r)
    view = trace.ledger.view()
    assert trace.final_actor is None and view["actor"] is None
    assert view["stacks"] == {0: 9750, 1: 9950, 2: 9700, 3: 0, 4: 10000, 5: 10000}
    assert view["street commitments"] == _ZERO
    assert view["collected pot"] == view["total pot"] == 800 + 50 + 50


def test_replay_rejects_a_record_whose_all_in_amount_is_not_the_whole_stack():
    # UTG's short all-in call (030 event 8) states a post-event commitment of 50: its whole stack.
    for wrong in (49, 51):
        r = _copied("030")
        r["events"][8]["action"]["to"] = wrong
        with pytest.raises(AssertionError, match=r"record 030 event 8 "):
            replay_record(r)


@pytest.mark.parametrize("record_id, index, cards, dup", [
    ("009", 10, "Kh7d2c6s", "6s"),   # the turn repeats one of hero's hole cards
    ("009", 6, "KhKh2c", "Kh"),      # the flop repeats its own card
], ids=["hero-card", "board-card"])
def test_replay_rejects_a_record_whose_board_repeats_a_dealt_card(record_id, index, cards, dup):
    r = _copied(record_id)
    r["events"][index]["cards"] = cards
    with pytest.raises(AssertionError, match=rf"record {record_id} event {index} .*{dup} is already dealt"):
        replay_record(r)


def _diverge_after(monkeypatch, event: dict, corrupt) -> None:
    """Make the model diverge from PokerKit right after `event` is applied (`corrupt(ledger, j)`,
    `j` = the event seat's player index): the next comparison must reject the replay."""
    import gen_fixtures

    apply = gen_fixtures.Ledger.apply

    def diverging(ledger, e, where):
        apply(ledger, e, where)
        if e is event:
            corrupt(ledger, ledger.ring.index(e["seat"]) if "seat" in e else None)

    monkeypatch.setattr(gen_fixtures.Ledger, "apply", diverging)


def _one_chip_back_to_the_stack(ledger, j):
    ledger.bets[j] -= 1
    ledger.committed[j] -= 1
    ledger.stacks[j] += 1


def _one_chip_more_collected(ledger, j):
    ledger.collected += 1


def _another_turn_card(ledger, j):
    ledger.board = ledger.board[:-2] + "4c"


@pytest.mark.parametrize("record_id, index, corrupt, field", [
    ("030", 8, _one_chip_back_to_the_stack, "stacks"),        # a short all-in call's amount
    ("030", 9, _one_chip_more_collected, "collected pot"),    # the flop collection
    ("009", 10, _another_turn_card, "board"),                 # a dealt board
], ids=["post-event-amount", "street-collection", "board"])
def test_replay_rejects_a_model_that_diverges_from_pokerkit(monkeypatch, record_id, index, corrupt, field):
    r = _copied(record_id)
    _diverge_after(monkeypatch, r["events"][index], corrupt)
    with pytest.raises(AssertionError, match=rf"record {record_id} event {index} .*: {field} differ"):
        replay_record(r)


# --- I3: per-record lock mapping for every supported record and the four straddle records ---

BTN_OPEN = ["", "F", "FF", "FFF", "FFFR", "FFFRF"]      # UTG/HJ/CO fold, BTN opens, SB folds, BB calls
CO_OPEN = ["", "F", "FF", "FFR", "FFRF", "FFRFF"]       # UTG/HJ fold, CO opens, BTN/SB fold, BB calls
UTG_OPEN_BTN_CALL = ["", "R", "RF", "RFF"]              # UTG opens, HJ/CO fold, BTN calls; SB next
VIRTUAL_UTG_OPEN = ["", "R", "RF", "RFF", "RFFF", "RFFFF"]  # straddle: HJ (virtual UTG) opens, straddler calls

LOCK_TABLE = {
    # id: (bundle, covered prefixes every villain/hero lookup needs, first missing prefix or None)
    "001": (PC, BTN_OPEN, None), "002": (PC, BTN_OPEN, None),
    "003": (PC, CO_OPEN, None), "004": (PC, CO_OPEN, None),
    "005": (RC, BTN_OPEN, None), "006": (RC, BTN_OPEN, None),
    "007": (RC, CO_OPEN, None), "008": (RC, CO_OPEN, None),
    "009": (PC, BTN_OPEN, None), "010": (PC, CO_OPEN, None), "011": (PC, BTN_OPEN, None),
    "012": (RC, BTN_OPEN, None), "013": (RC, CO_OPEN, None), "014": (RC, BTN_OPEN, None),
    "015": (PC, BTN_OPEN, None), "016": (PC, CO_OPEN, None), "017": (RC, BTN_OPEN, None),
    "018": (RC, CO_OPEN, None), "019": (PC, BTN_OPEN, None), "020": (RC, BTN_OPEN, None),
    "021": (PC, UTG_OPEN_BTN_CALL, "RFFC"), "022": (PC, UTG_OPEN_BTN_CALL, "RFFC"),
    "023": (PC, VIRTUAL_UTG_OPEN, None), "024": (PC, VIRTUAL_UTG_OPEN, None),
    "025": (PC, VIRTUAL_UTG_OPEN, None), "026": (PC, VIRTUAL_UTG_OPEN, None),
}


def test_every_supported_and_straddle_record_has_an_audited_lock_mapping():
    lock = _lock()
    nodes = {b["name"]: set(b["nodes"]) for b in lock["bundles"]}
    rows = {r["id"]: r for r in records()}
    assert set(LOCK_TABLE) == {f"{n:03}" for n in range(1, 27)}
    for record_id, (bundle, covered, missing) in LOCK_TABLE.items():
        r = rows[record_id]
        lookups = _preflop_lookups(r, lock)
        # every lookup up to (and including) the first missing one uses the spec-8.3 bundle
        needed = lookups[:len(covered) + (1 if missing else 0)]
        assert {b for _, b, _ in needed} == {bundle}, (record_id, needed)
        keys = [k for _, _, k in lookups]
        assert keys[:len(covered)] == covered, (record_id, keys)
        if missing is None:
            assert len(keys) == len(covered), (record_id, keys)
        else:
            assert keys[len(covered)] == missing, (record_id, keys)
        # against the frozen lock, in both directions
        assert all(k in nodes[bundle] for k in covered), (record_id, bundle, covered)
        assert missing is None or missing not in nodes[bundle], (record_id, bundle, missing)
        assert "ChartRounded" in r["expected"]["required_reasons"], record_id


def test_a_missing_prior_prefix_requires_unconditioned_prior_street():
    rows = {r["id"]: r for r in records()}
    for record_id, (_, _, missing) in LOCK_TABLE.items():
        if missing is not None:
            assert "UnconditionedPriorStreet" in rows[record_id]["expected"]["required_reasons"], record_id
    assert rows["021"]["expected"]["required_reasons"] == [
        "ChartRounded", "RakeProfileMapped", "MultiwayStreetRoot", "UnconditionedPriorStreet"]
    assert rows["022"]["expected"]["required_reasons"] == rows["021"]["expected"]["required_reasons"]


# --- I4: undocumented rake in both manifests -> RakeProfileMapped wherever ChartRounded ---


def test_rake_profile_mapped_accompanies_every_chart_rounded():
    for bundle in _lock()["bundles"]:
        assert bundle["manifest"]["rake_profile"] == "undocumented", bundle["name"]
        assert bundle["manifest"]["rake"] is None, bundle["name"]
    rows = records()
    for r in rows:
        reasons = r["expected"]["required_reasons"]
        assert "ChartRounded" in reasons, r["id"]
        assert "RakeProfileMapped" in reasons, r["id"]
        assert len(set(reasons)) == len(reasons), r["id"]
    assert sum("RakeProfileMapped" in r["expected"]["required_reasons"] for r in rows) == 50


# --- I5: straddle source depth uses the straddle as the unit ---

# id: (source depth in straddle units, required mapping reasons beyond StraddleMapped, posts)
STRADDLE_DEPTHS = {
    "023": (50, ["DepthBucket", "AsymmetricStacks"], (Fraction(1, 4), Fraction(1, 2), 1)),
    "024": (100, [], (Fraction(1, 4), Fraction(1, 2), 1)),
    "025": (50, ["DepthBucket", "AsymmetricStacks"], (Fraction(1, 5), Fraction(1, 2), 1)),
    "026": (100, [], (Fraction(1, 5), Fraction(1, 2), 1)),
}


def test_straddle_records_use_the_straddle_unit_for_source_depth():
    lock = _lock()
    rows = {r["id"]: r for r in records()}
    for record_id, (depth, mapping, posts) in STRADDLE_DEPTHS.items():
        r = rows[record_id]
        cfg = r["config"]
        straddle = cfg["straddle"]
        assert all(Fraction(s, straddle) == depth for s in r["stacks"]), record_id
        assert (Fraction(cfg["sb_chips"], straddle), Fraction(cfg["bb_chips"], straddle), 1) == posts, record_id
        assert {b for _, b, _ in _preflop_lookups(r, lock)} == {PC}, record_id
        assert r["expected"]["required_reasons"] == ["ChartRounded", "RakeProfileMapped", "StraddleMapped", *mapping], record_id


@pytest.mark.parametrize("depth, used", [
    (Fraction(50), 100), (Fraction(100), 100), (Fraction(149), 100), (Fraction(150), 200),
    (Fraction(151), 200), (Fraction(200), 200), (Fraction(300), 200), (Fraction(3), 100),
])
def test_nearest_acquired_depth_ties_deeper_and_clamps(depth, used):
    assert e2e_hands.nearest_depth(depth, [100, 200]) == used


# --- special and fault expectations ---


def test_special_records_name_their_exact_coverage_label_or_rejection_reason():
    rows = {r["id"]: r for r in records()}
    assert set(SPECIAL_EXPECTATIONS) == {f"{n:03}" for n in range(23, 37)}
    for id_, (cls, coverage, unsupported) in SPECIAL_EXPECTATIONS.items():
        r = rows[id_]
        assert r["class"] == cls, id_
        assert r["expected"]["coverage"] == coverage, id_
        assert r["expected"]["unsupported"] == unsupported, id_
        assert r["expected"]["numeric_ev"] is False, id_


def test_fault_records_name_their_injected_fault_and_reason():
    rows = records()
    fault_rows = rows[36:]
    assert len(fault_rows) == 14
    for r in fault_rows:
        assert r["class"] == "failure_injection"
        assert r["fault"] in FAULTS, r["id"]
        assert r["expected"]["coverage"] == "Unsupported", r["id"]
        assert r["expected"]["unsupported"] == FAULT_REASON[r["fault"]], r["id"]
        assert r["expected"]["required_reasons"] == ["ChartRounded", "RakeProfileMapped"], r["id"]
        assert r["hero_cards"] == e2e_hands.DEFAULT_HERO_CARDS, r["id"]
    # the four budget-variant records (047-050) reuse an existing supported record's events and
    # additionally pin flop_budget_s to the fault runner's default budget
    for r in fault_rows[10:]:
        assert r["config"]["flop_budget_s"] == 10, r["id"]


def test_records_are_deterministic_and_independent_across_calls():
    first = records()
    second = records()
    assert first == second
    # mutating one call's result must never affect another (no shared nested state)
    first[0]["events"].append({"type": "action", "seat": 0, "street": "flop", "action": {"kind": "fold"}})
    assert second[0]["events"] != first[0]["events"]
    first[0]["expected"]["required_reasons"].append("X")
    assert "X" not in second[0]["expected"]["required_reasons"]
    assert "X" not in first[1]["expected"]["required_reasons"]


# --- Task 19: freezing the fifty fixture files + manifest ---
#
# Fix round 1 (review I1-I3): every frozen file, manifest included, comes from one pure serializer
# (`e2e_hands.canonical_json_bytes`: UTF-8, LF, one trailing newline) through
# `e2e_hands.e2e_files()`. Exactly one test reads the committed directory, and only reads it; every
# test that writes or tampers works on its own `tmp_path` freeze, so no test can repair, rewrite or
# corrupt `fixtures/hands/e2e` (or race the Rust loader reading it).

FROZEN_NAMES = [f"{n:03}.json" for n in range(1, 51)] + ["manifest.json"]
SUPPORTED_IDS = [f"{n:03}" for n in range(1, 23)]
TOOLS = REPO / "tools"


def _snapshot(root: Path) -> dict:
    return {p.name: p.read_bytes() for p in sorted(root.iterdir())}


def test_committed_e2e_freeze_matches_in_memory_regeneration():
    """The one committed-artifact test, read-only: the exact inventory, then every frozen file
    byte for byte (fresh-checkout LF bytes) against in-memory regeneration. A missing file, a
    stray file or a CR byte fails; nothing is written, so a failure is never repaired here."""
    assert sorted(p.name for p in ROOT.iterdir()) == FROZEN_NAMES
    expected = e2e_hands.e2e_files()
    assert list(expected) == FROZEN_NAMES
    for name in FROZEN_NAMES:
        raw = (ROOT / name).read_bytes()
        assert b"\r" not in raw, f"{name} contains CR bytes; the freeze is LF"
        assert raw == expected[name], f"{name} differs from in-memory regeneration"
    manifest = json.loads((ROOT / "manifest.json").read_bytes())
    assert len(manifest["sha256"]) == 50
    assert manifest["supported_ids"] == SUPPORTED_IDS
    assert manifest["supported_baseline"] == manifest["supported_ids"]
    check_e2e(ROOT)  # the CLI's check agrees, and it only reads


def test_e2e_files_are_canonical_lf_utf8_json():
    files = e2e_hands.e2e_files()
    assert list(files) == FROZEN_NAMES
    for name, raw in files.items():
        assert raw.endswith(b"\n") and not raw.endswith(b"\n\n"), name
        assert b"\r" not in raw, name
        text = raw.decode("utf-8")
        assert text == json.dumps(json.loads(text), sort_keys=True, indent=2) + "\n", name
        assert e2e_hands.canonical_json_bytes(json.loads(raw)) == raw, name
    for row, name in zip(records(), FROZEN_NAMES):
        assert json.loads(files[name]) == row, name
    assert json.loads(files["manifest.json"]) == {
        "version": 1,
        "synthetic": True,
        "supported_ids": SUPPORTED_IDS,
        "supported_baseline": SUPPORTED_IDS,
        "sha256": {name: hashlib.sha256(files[name]).hexdigest() for name in FROZEN_NAMES[:50]},
    }


_WRITE_IN_A_FRESH_PROCESS = (
    "import sys\nfrom pathlib import Path\nimport e2e_hands\ne2e_hands.write_e2e(Path(sys.argv[1]))\n"
)


def test_generated_bytes_are_stable(tmp_path):
    """Brief Step 1, on private directories: repeated generation -- over its own output in this
    process, and in a fresh interpreter with a different hash seed -- writes exactly the LF bytes
    of in-memory regeneration (a text-mode write would put CR bytes into the manifest on Windows)."""
    expected = e2e_hands.e2e_files()
    here, fresh = tmp_path / "here", tmp_path / "fresh"
    write_e2e(here)
    assert _snapshot(here) == expected
    write_e2e(here)
    assert _snapshot(here) == expected
    env = {**os.environ, "PYTHONHASHSEED": "20260927"}
    subprocess.run([sys.executable, "-W", "error", "-c", _WRITE_IN_A_FRESH_PROCESS, str(fresh)],
                   cwd=TOOLS, env=env, check=True)
    assert _snapshot(fresh) == expected
    manifest = json.loads((here / "manifest.json").read_bytes())
    assert len(manifest["sha256"]) == 50
    assert manifest["supported_ids"] == SUPPORTED_IDS
    assert manifest["supported_baseline"] == manifest["supported_ids"]


@pytest.fixture
def freeze(tmp_path) -> Path:
    """A private, freshly written freeze for tests that tamper with it."""
    root = tmp_path / "e2e"
    write_e2e(root)
    return root


def _rewrite_manifest(root: Path, edit) -> None:
    manifest = json.loads((root / "manifest.json").read_bytes())
    edit(manifest)
    (root / "manifest.json").write_bytes(e2e_hands.canonical_json_bytes(manifest))


def _assert_check_fails(root: Path, pattern: str) -> None:
    before = _snapshot(root)
    with pytest.raises(SystemExit, match=pattern):
        check_e2e(root)
    assert _snapshot(root) == before, "check_e2e must never write"


def test_check_e2e_passes_on_a_fresh_freeze_and_never_writes(freeze):
    before = _snapshot(freeze)
    check_e2e(freeze)
    assert _snapshot(freeze) == before


def test_check_e2e_flags_a_tampered_hand_file(freeze):
    path = freeze / "001.json"
    path.write_bytes(path.read_bytes() + b" ")
    _assert_check_fails(freeze, r"001\.json differs from the generator")


def test_check_e2e_flags_a_crlf_hand_file(freeze):
    path = freeze / "007.json"
    path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
    _assert_check_fails(freeze, r"007\.json differs from the generator \(it contains CR bytes; the freeze is LF\)")


def test_check_e2e_flags_a_missing_hand_file(freeze):
    (freeze / "002.json").unlink()
    _assert_check_fails(freeze, r"002\.json is missing")


def test_check_e2e_flags_a_missing_manifest(freeze):
    (freeze / "manifest.json").unlink()
    _assert_check_fails(freeze, r"manifest\.json is missing")


def test_check_e2e_flags_a_file_outside_the_freeze(freeze):
    (freeze / "051.json").write_bytes(b"{}\n")
    _assert_check_fails(freeze, r"051\.json is not part of the freeze")


@pytest.mark.parametrize("edit, pattern", [
    (lambda m: m["sha256"].pop("017.json"),
     r"manifest\.json: sha256 lacks 1 of 50 entries: \['017\.json'\]"),
    (lambda m: m["sha256"].clear(),
     r"manifest\.json: sha256 lacks 50 of 50 entries: \['001\.json', '002\.json', .*'050\.json'\]"),
    (lambda m: m["sha256"].update({"051.json": "0" * 64}),
     r"manifest\.json: sha256 has unexpected entries \['051\.json'\]"),
    (lambda m: m["sha256"].update({"001.json": "0" * 64}),
     r"manifest\.json: sha256\['001\.json'\] is '0{64}', expected [0-9a-f]{64}"),
    (lambda m: m["supported_ids"].pop(),
     r"manifest\.json: supported_ids is \['001', .*'021'\], expected \['001', .*'022'\]"),
    (lambda m: m.update(supported_baseline=SUPPORTED_IDS[:-1] + ["023"]),
     r"manifest\.json: supported_baseline is \[.*'023'\], expected \[.*'022'\]"),
    (lambda m: m.update(version=2), r"manifest\.json: version is 2, expected 1"),
    (lambda m: m.update(version=True), r"manifest\.json: version is True, expected 1"),
    (lambda m: m.update(synthetic=False), r"manifest\.json: synthetic is False, expected True"),
    (lambda m: m.pop("supported_baseline"), r"manifest\.json: supported_baseline is missing"),
    (lambda m: m.update(note="x"), r"manifest\.json: unexpected keys \['note'\]"),
], ids=["missing-hash", "empty-hash-map", "extra-hash", "mismatched-digest", "altered-supported-ids",
        "altered-baseline-ids", "version", "version-bool", "synthetic", "missing-baseline", "extra-key"])
def test_check_e2e_flags_manifest_content(freeze, edit, pattern):
    _rewrite_manifest(freeze, edit)
    _assert_check_fails(freeze, pattern)


@pytest.mark.parametrize("drift, pattern", [
    (lambda raw: raw.replace(b"\n", b"\r\n"), r"\(it contains CR bytes; the freeze is LF\)"),
    (lambda raw: raw[:-1], ""),
    (lambda raw: raw + b"\n", ""),
    (lambda raw: (json.dumps(json.loads(raw), sort_keys=True, indent=4) + "\n").encode(), ""),
    (lambda raw: (json.dumps(dict(reversed(json.loads(raw).items())), indent=2) + "\n").encode(), ""),
], ids=["crlf", "no-trailing-newline", "extra-newline", "indent-4", "key-order"])
def test_check_e2e_flags_manifest_only_byte_drift(freeze, drift, pattern):
    path = freeze / "manifest.json"
    raw = path.read_bytes()
    drifted = drift(raw)
    assert drifted != raw and json.loads(drifted) == json.loads(raw)  # same content, other bytes
    path.write_bytes(drifted)
    _assert_check_fails(freeze, r"manifest\.json: bytes differ from the generator although its content matches ?" + pattern)
