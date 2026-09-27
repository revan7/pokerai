"""Generate fixtures/hands/*.json: PokerKit hands for spec 13.1 `state_machine_pokerkit_fixtures`.

PokerKit is the oracle for pot totals, stacks, refunds and legal actions. Three normalizations are
applied (all verified on 2026-09-10) so the fixture states the same fact as core-model rather than a
differently sliced one:
  1. the minimum open over a straddle is 2S (PokerKit alone uses S + bb);
  2. at a fold-out the survivor's uncollected bet is split into the matched part (pot) and the refund;
  3. the pot list is folded into core-model's layering rule (`normalize_pots` below).
Hands where PokerKit's reopening rule diverges from spec 4.3 (a full all-in raise followed by a short
all-in raise) are dropped.
"""
from __future__ import annotations

import argparse
import json
import random
import sys
import warnings
from dataclasses import dataclass
from pathlib import Path

from pokerkit import Automation, Mode, NoLimitTexasHoldem

# PokerKit warns (UserWarning) whenever a card handed to `deal_hole`/`deal_board` isn't the exact
# next card in *its own* internal deck order. We deal from our own seeded, shuffled `DECK` instead
# (see `generate_hand` below), so every dealt card is still genuine, undealt, and distinct -- the
# warning is a known false positive for those two calls only. It is silenced narrowly, by category
# and message, only around the call that triggers it (`_deal` below); nothing else is affected, and
# no process-wide or pytest-wide filter is installed (see `test_gen_fixtures_warnings.py` for the guarantee).
_UNRECOMMENDED_DEAL_RE = r"A card being dealt .* is not recommended to be dealt\."

AUTOMATIONS = (
    Automation.ANTE_POSTING,
    Automation.BET_COLLECTION,
    Automation.BLIND_OR_STRADDLE_POSTING,
    Automation.CARD_BURNING,
    Automation.RUNOUT_COUNT_SELECTION,
)
# (sb, bb, straddle, min dealt seats, max dealt seats)
CONFIGS = [(1, 2, None, 3, 6), (1, 2, 4, 6, 6), (2, 5, 10, 6, 6), (2, 5, None, 3, 6)]
STACK_BB = [7, 8.5, 12.5, 20, 30, 50, 100, 150, 200]
FOLD_P, WAGER_P, ALLIN_P = 0.25, 0.30, 0.06
STREETS = ["preflop", "flop", "turn", "river"]
STREET_INDEX = {street: i for i, street in enumerate(STREETS)}
RANKS = "23456789TJQKA"
SUITS = "cdhs"
DECK = [r + s for r in RANKS for s in SUITS]


class SpecReopen:
    """Spec 4.3 cumulative reopening, tracked per street to detect the PokerKit divergence."""

    def __init__(self, n: int, initial_full: int, facing: int) -> None:
        self.n = n
        self.start_street(initial_full, facing)

    def start_street(self, min_bet: int, facing: int) -> None:
        self.facing_at_last: list[int | None] = [None] * self.n
        self.last_full = min_bet
        self.facing = facing

    def may_raise(self, i: int) -> bool:
        f = self.facing_at_last[i]
        return f is None or self.facing - f >= self.last_full

    def acted(self, i: int, to: int | None) -> None:
        if to is not None:
            inc = to - self.facing
            if inc >= self.last_full:
                self.last_full = inc
            self.facing = to
        self.facing_at_last[i] = self.facing


def _deal(fn, *args):
    """Call a PokerKit dealing method (`state.deal_hole` / `state.deal_board`), narrowly silencing
    only its expected "not recommended to be dealt" `UserWarning` (see the module docstring/comment
    above `_UNRECOMMENDED_DEAL_RE`) for the duration of this one call. Any other warning raised
    inside `fn` -- including a differently worded PokerKit warning -- still propagates normally.
    """
    with warnings.catch_warnings():
        warnings.filterwarnings("ignore", category=UserWarning, message=_UNRECOMMENDED_DEAL_RE)
        return fn(*args)


def legal_triple(state) -> dict:
    i = state.actor_index
    facing = max(state.bets)
    owed = facing - state.bets[i]
    # `state.can_fold()` itself emits a `UserWarning` ("There is no reason for this player to
    # fold.") when `owed == 0` (Mode.CASH_GAME). Test `owed > 0` first so the call -- and the
    # warning -- never happens for a fold that would be illegal anyway.
    fold = owed > 0 and bool(state.can_fold())
    cc = {"cost": min(owed, state.stacks[i])} if state.can_check_or_call() else None
    assert cc is None or cc["cost"] == state.checking_or_calling_amount
    rz = None
    if state.can_complete_bet_or_raise_to():
        rz = {"min_to": state.min_completion_betting_or_raising_to_amount, "max_to": state.max_completion_betting_or_raising_to_amount}
    return {"fold": fold, "check_or_call": cc, "raise": rz}


def spec_may_aggress(state, reopen: SpecReopen, i: int) -> bool:
    facing = max(state.bets)
    if state.stacks[i] + state.bets[i] <= facing or not reopen.may_raise(i):
        return False
    return any(j != i and state.statuses[j] and state.stacks[j] + state.bets[j] > facing for j in range(len(state.stacks)))


# --- Plan 4 Task 19: PokerKit adapter for the fifty recorded e2e hands (`tools/e2e_hands.py`) ---


def _board_cards_from_events(events: list) -> list[str]:
    """The final dealt board of an event history (0, 3, 4 or 5 cards), split into two-character
    cards. `events` may be a record's full history (`e2e_hands.records()`'s `events` field) or any
    prefix of it that still contains every `board` event dealt so far."""
    boards = [e["cards"] for e in events if e["type"] == "board"]
    final = boards[-1] if boards else ""
    return [final[i:i + 2] for i in range(0, len(final), 2)]


class Ledger:
    """Independent chip-and-board accounting for one e2e record (Task 19 fix round 1, I4), kept by
    player index in PokerKit's order (`ring`: SB first, button last). It reads only the record
    and hold'em's rules, never PokerKit's state; `replay_record` asserts that `view()` equals
    PokerKit's state after the posts and after every event.

    Rules: SB and BB post, and a configured straddle is posted by the next seat (a short stack
    posts what it has). A check pays nothing, a call pays min(owed, stack) -- a short all-in call
    included -- a bet or raise to `to` pays `to` minus the seat's street commitment, and an
    all-in pays the whole stack. A seat needs to act while it is live, has chips behind, and
    either owes chips or has not acted this street (posts are not actions) and still faces an
    opponent who could put in more than it has. When no seat needs to act the street closes: the
    wager above the second-largest street commitment (folded seats' dead chips count) goes back
    to its owner and every other street commitment is collected into the pot. A board event must
    add cards not already dealt (hero's hole cards or the board) and opens the next street,
    first to act the first seat after the button. A fold that leaves one live seat ends the hand,
    which an e2e history -- played to a decision -- never does, so it is rejected."""

    def __init__(self, record: dict, ring: list[int]) -> None:
        cfg = record["config"]
        n = len(ring)
        assert n >= 3, f"record {record['id']}: the ledger covers 3-6 dealt seats, not {n}"
        self.ring = ring
        self.stacks = [record["stacks"][s] for s in ring]
        self.bets = [0] * n          # street commitments, uncollected
        self.committed = [0] * n     # hand commitments, net of returned wagers
        self.live = [True] * n
        self.acted = [False] * n
        self.collected = 0
        self.board = ""
        self.dealt = {record["hero_cards"][:2], record["hero_cards"][2:]}
        posts = [cfg["sb_chips"], cfg["bb_chips"]] + ([cfg["straddle"]] if cfg["straddle"] else [])
        for j, amount in enumerate(posts):
            self._pay(j, min(amount, self.stacks[j]))
        self.actor = self._next_after(len(posts) - 1)

    def _pay(self, j: int, amount: int) -> None:
        self.stacks[j] -= amount
        self.bets[j] += amount
        self.committed[j] += amount

    def _needs_to_act(self, j: int) -> bool:
        if not self.live[j] or self.stacks[j] == 0:
            return False
        if self.bets[j] < max(self.bets):
            return True
        return not self.acted[j] and any(
            self.live[k] and self.bets[k] + self.stacks[k] > self.bets[j] for k in range(len(self.ring)) if k != j)

    def _next_after(self, j: int) -> int | None:
        n = len(self.ring)
        return next((k % n for k in range(j + 1, j + 1 + n) if self._needs_to_act(k % n)), None)

    def _collect(self) -> None:
        cutoff = sorted(self.bets)[-2]
        for j, bet in enumerate(self.bets):
            if bet > cutoff:
                self.stacks[j] += bet - cutoff
                self.committed[j] -= bet - cutoff
                self.bets[j] = cutoff
        self.collected += sum(self.bets)
        self.bets = [0] * len(self.ring)

    def apply(self, e: dict, where: str) -> None:
        """Apply one record event (`where` names the record and event in any rejection)."""
        if e["type"] == "board":
            for card in (e["cards"][i:i + 2] for i in range(len(self.board), len(e["cards"]), 2)):
                assert card not in self.dealt, f"{where}: board card {card} is already dealt (hero's hole cards or the board)"
                self.dealt.add(card)
            self.board = e["cards"]
            self.acted = [False] * len(self.ring)
            self.actor = self._next_after(len(self.ring) - 1)
            return
        j = self.ring.index(e["seat"])
        kind, to = e["action"]["kind"], e["action"].get("to")
        if kind == "fold":
            self.live[j] = False
            assert sum(self.live) > 1, f"{where}: a fold-out ends the hand, but an e2e history is played to a decision"
        elif kind == "call":
            self._pay(j, min(max(self.bets) - self.bets[j], self.stacks[j]))
        elif kind in ("bet", "raise"):
            self._pay(j, to - self.bets[j])
        elif kind == "allin":
            self._pay(j, self.stacks[j])
        self.acted[j] = True
        self.actor = self._next_after(j)
        if self.actor is None:
            self._collect()

    def view(self) -> dict:
        """What `replay_record` compares with PokerKit, per-seat values keyed by seat."""
        return _view(
            actor=None if self.actor is None else self.ring[self.actor],
            ring=self.ring, live=self.live, stacks=self.stacks, bets=self.bets, committed=self.committed,
            collected=self.collected, board=self.board)


def _view(*, actor, ring, live, stacks, bets, committed, collected, board) -> dict:
    def by_seat(values) -> dict:
        return dict(zip(ring, values))

    return {
        "actor": actor,
        "live": by_seat(bool(v) for v in live),
        "stacks": by_seat(stacks),
        "street commitments": by_seat(bets),
        "hand commitments": by_seat(committed),
        "collected pot": collected,
        "total pot": collected + sum(bets),
        "board": board,
    }


def _pokerkit_view(state, ring: list[int]) -> dict:
    """PokerKit's side of the comparison: `bets` are the uncollected street commitments, the
    negated `payoffs` are what each player has put in net of returned wagers (no chips are pushed
    before a decision), `pot_amounts` are the collected pots, `total_pot_amount` adds the
    uncollected wagers, and board index 0 is the board."""
    view = _view(
        actor=None if state.actor_index is None else ring[state.actor_index],
        ring=ring, live=state.statuses, stacks=state.stacks, bets=state.bets,
        committed=[-p for p in state.payoffs], collected=sum(state.pot_amounts),
        board="".join(repr(c) for c in state.get_board_cards(0)))
    view["total pot"] = state.total_pot_amount
    return view


def _compare(state, ledger: Ledger, where: str) -> int:
    """Assert the ledger equals PokerKit's state, naming `where` and the first differing field;
    returns 1, the number of states compared."""
    actual = _pokerkit_view(state, ledger.ring)
    for field, value in ledger.view().items():
        assert actual[field] == value, f"{where}: {field} differ -- PokerKit {actual[field]!r}, model {value!r}"
    return 1


@dataclass
class Trace:
    """The result of replaying one e2e record's complete event history through PokerKit: the
    final `pokerkit` state, its seating ring (SB first, button last, the same order PokerKit's
    `actor_index` indexes into), whether every step compared legal (see `replay_record`), the
    seat to act when the replay stops (`None` at a terminal street -- an all-in runout or
    showdown), the independent `Ledger` at that point, and how many states were compared with
    PokerKit (the posts plus one per event)."""

    state: object
    ring: list[int]
    legal_at_every_step: bool
    final_actor: int | None
    ledger: Ledger
    states_compared: int


def replay_record(record: dict) -> Trace:
    """Plan 1's PokerKit adapter, extended by Task 19: replays one `e2e_hands.records()` row's
    complete event history through PokerKit (the plan-1 oracle for legal actions, pots, stacks and
    refunds) and, alongside it, through the independent `Ledger`, asserting after the posts and
    after every event that both agree on the actor, live seats, stacks, street and hand
    commitments, collected and total pot, and dealt board (fix round 1, I4). Straddle and
    projection records are replayed as real, legal sequences first -- no record gets a pass
    because it is "just a projection" or "just a straddle" case.

    Fails loudly (`AssertionError`, naming the record id and the event index, or "posts" before
    event 0) on any illegal action (a wager outside PokerKit's `[min_to, max_to)`, a check facing
    a bet, an all-in that is not the whole stack, a fold with nothing owed, the wrong actor or
    street, a board dealt mid-street or repeating a dealt card), on a fold-out, on any
    PokerKit/spec-4.3 reopening divergence and on any ledger/PokerKit difference -- so
    `legal_at_every_step` is `True` on every `Trace` this function actually returns; it never
    returns a `Trace` for an illegal replay.
    """
    cfg = record["config"]
    button = record["button"]
    ring = [s for s in ((button + k) % 6 for k in range(1, 7)) if s in record["dealt"]]
    n = len(ring)
    straddle = cfg["straddle"]
    blinds = [cfg["sb_chips"], cfg["bb_chips"]] + ([straddle] if straddle else [])
    blinds += [0] * (n - len(blinds))
    state = NoLimitTexasHoldem.create_state(
        AUTOMATIONS, False, 0, blinds, cfg["bb_chips"], [record["stacks"][s] for s in ring], n,
        mode=Mode.CASH_GAME)
    hero_index = ring.index(record["hero"])
    used = {record["hero_cards"][:2], record["hero_cards"][2:]} | set(_board_cards_from_events(record["events"]))
    spare = [c for c in DECK if c not in used]
    for i in range(n):
        _deal(state.deal_hole, record["hero_cards"] if i == hero_index else spare.pop() + spare.pop())
    assert "".join(repr(c) for c in state.hole_cards[hero_index]) == record["hero_cards"], record["id"]
    if straddle:
        # the first full raise over a straddle is one straddle (min open 2S), as in generate_hand
        state.completion_betting_or_raising_amount = straddle
    reopen = SpecReopen(n, straddle or cfg["bb_chips"], max(state.bets))
    ledger = Ledger(record, ring)
    compared = _compare(state, ledger, f"record {record['id']} posts (before event 0)")
    board_so_far = ""
    for k, e in enumerate(record["events"]):
        where = f"record {record['id']} event {k} {e}"
        if e["type"] == "board":
            assert state.actor_index is None and state.can_deal_board(), where
            assert e["cards"].startswith(board_so_far), where
            new = e["cards"][len(board_so_far):]
            assert len(new) == (6 if not board_so_far else 2), where
            ledger.apply(e, where)  # before PokerKit, which would silently deal a repeated card
            _deal(state.deal_board, new)
            reopen.start_street(cfg["bb_chips"], 0)
            board_so_far = e["cards"]
            compared += _compare(state, ledger, where)
            continue
        i = state.actor_index
        assert i is not None and ring[i] == e["seat"], where
        assert state.street_index == STREET_INDEX[e["street"]], where
        legal = legal_triple(state)
        assert (legal["raise"] is not None) == spec_may_aggress(state, reopen, i), f"reopening divergence at {where}"
        kind, to = e["action"]["kind"], e["action"].get("to")
        facing = max(state.bets)
        aggression = None
        if kind == "fold":
            assert to is None and legal["fold"], f"illegal fold at {where}: {legal}"
            state.fold()
        elif kind == "check":
            assert to is None and legal["check_or_call"] == {"cost": 0}, f"illegal check at {where}: {legal}"
            state.check_or_call()
        elif kind == "call":
            cc = legal["check_or_call"]
            assert to is None and cc is not None and cc["cost"] > 0, f"illegal call at {where}: {legal}"
            state.check_or_call()
        elif kind in ("bet", "raise"):
            assert (facing == 0) == (kind == "bet"), f"{kind} while facing {facing} at {where}"
            rz = legal["raise"]
            assert rz is not None and rz["min_to"] <= to < rz["max_to"], f"illegal {kind} to {to} at {where}: {legal}"
            state.complete_bet_or_raise_to(to)
            aggression = to
        elif kind == "allin":
            assert to == state.bets[i] + state.stacks[i], f"all-in to {to} is not the whole stack at {where}"
            if to > facing:
                assert legal["raise"] is not None and legal["raise"]["max_to"] == to, f"illegal all-in at {where}: {legal}"
                state.complete_bet_or_raise_to(to)
                aggression = to
            else:
                cc = legal["check_or_call"]
                assert cc is not None and cc["cost"] == state.stacks[i], f"illegal all-in call at {where}: {legal}"
                state.check_or_call()
        else:
            raise AssertionError(f"unknown action kind at {where}")
        reopen.acted(i, aggression)
        ledger.apply(e, where)  # after the legality checks above, so theirs is the first failure
        compared += _compare(state, ledger, where)
    final_actor = ring[state.actor_index] if state.actor_index is not None else None
    return Trace(state=state, ring=ring, legal_at_every_step=True, final_actor=final_actor,
                 ledger=ledger, states_compared=compared)


def normalize_pots(state, ring) -> list[dict]:
    """PokerKit's raw pot decomposition, folded into core-model's layering rule (Task 11).

    `core_model::settlement::layer_pots` merges a contribution layer into the previous one when the
    eligible set is equal, or when it is empty (a folded seat's chips are dead money). PokerKit
    reports the raw levels, so a fold-out like 100 / 2 (folded) / 100 is two PokerKit pots and one
    core-model pot of 202. The totals are identical either way; this only aligns the slicing, so the
    fixture asserts one layering rule instead of two.
    """
    out: list[dict] = []
    for p in state.pots:
        entry = {"amount": p.amount, "eligible": sorted(ring[j] for j in p.player_indices)}
        if out and (not entry["eligible"] or out[-1]["eligible"] == entry["eligible"]):
            out[-1]["amount"] += entry["amount"]
        else:
            out.append(entry)
    return out


def snapshot(state, ring, phase: str, street: str, pots_override=None, stacks_override=None) -> dict:
    n = len(ring)
    pots = pots_override if pots_override is not None else normalize_pots(state, ring)
    stacks = stacks_override if stacks_override is not None else list(state.stacks)
    committed = [0] * n if pots_override is not None else list(state.bets)
    return {
        "phase": phase,
        "street": street,
        "to_act": ring[state.actor_index] if state.actor_index is not None else None,
        "pot_total": sum(p["amount"] for p in pots) + sum(committed),
        "committed": committed,
        "stacks": stacks,
        "folded": [not s for s in state.statuses],
        "all_in": [bool(state.statuses[j]) and stacks[j] == 0 for j in range(n)],
        "pots": pots,
        "legal": legal_triple(state) if state.actor_index is not None else None,
    }


def generate_hand(seed: int) -> dict | None:
    rng = random.Random(seed)
    sb, bb, straddle, lo, hi = rng.choice(CONFIGS)
    n = rng.randint(lo, hi)
    button = rng.randrange(6)
    others = [s for s in range(6) if s != button]
    dealt_set = set(rng.sample(others, n - 1)) | {button}
    ring = [s for s in ((button + k) % 6 for k in range(1, 7)) if s in dealt_set]  # SB, BB, ..., BTN
    stacks_start = [int(rng.choice(STACK_BB) * bb) for _ in range(n)]
    blinds = [sb, bb] + ([straddle] if straddle else []) + [0] * (n - 2 - (1 if straddle else 0))
    state = NoLimitTexasHoldem.create_state(AUTOMATIONS, False, 0, blinds, bb, stacks_start, n, mode=Mode.CASH_GAME)
    deck = DECK[:]
    rng.shuffle(deck)
    hole = [deck.pop() + deck.pop() for _ in range(n)]
    for h in hole:
        _deal(state.deal_hole, h)
    if straddle:
        # standard rule: the first full raise over a straddle is one straddle (min open 2S); must be set after hole dealing
        state.completion_betting_or_raising_amount = straddle
    reopen = SpecReopen(n, straddle or bb, max(state.bets))
    hero = rng.choice(ring)
    steps: list[dict] = []
    returned: list[list[int]] = []
    street_idx = 0
    n_allin = 0
    short_allin = 0

    while state.status:
        if state.actor_index is not None:
            i = state.actor_index
            legal = legal_triple(state)
            if (legal["raise"] is not None) != spec_may_aggress(state, reopen, i):
                return None  # documented PokerKit divergence: drop the hand
            facing = max(state.bets)
            r = rng.random()
            to = None
            if legal["raise"] is not None and r < ALLIN_P:
                to = legal["raise"]["max_to"]
                action = {"kind": "allin", "to": to}
            elif legal["raise"] is not None and r < ALLIN_P + WAGER_P:
                mn, mx = legal["raise"]["min_to"], legal["raise"]["max_to"]
                to = mn if rng.random() < 0.5 else rng.randint(mn, mx)
                action = {"kind": "allin", "to": to} if to == mx else {"kind": "bet" if facing == 0 else "raise", "to": to}
            elif legal["fold"] and r < ALLIN_P + WAGER_P + FOLD_P:
                action = {"kind": "fold"}
            else:
                action = {"kind": "check" if legal["check_or_call"]["cost"] == 0 else "call"}
            bets_before, stacks_before = list(state.bets), list(state.stacks)
            street = STREETS[street_idx]
            if action["kind"] == "fold":
                state.fold()
            elif action["kind"] in ("check", "call"):
                state.check_or_call()
            else:
                if action["kind"] == "allin":
                    n_allin += 1
                    if to < facing + reopen.last_full:
                        short_allin += 1
                state.complete_bet_or_raise_to(to)
            reopen.acted(i, to)
            step = {"kind": "action", "seat": ring[i], "street": street, "action": action}
            if state.actor_index is not None:
                step["after"] = snapshot(state, ring, "betting", street)
                steps.append(step)
                continue
            # the street closed with this action
            if sum(state.statuses) == 1:
                w = state.statuses.index(True)
                matched = max((bets_before[j] for j in range(n) if j != w), default=0)
                refund = bets_before[w] - matched
                stacks = list(state.stacks)
                stacks[w] += refund
                pots = [{"amount": sum(p.amount for p in state.pots) + matched, "eligible": [ring[w]]}]
                if refund > 0:
                    returned.append([ring[w], refund])
                step["after"] = snapshot(state, ring, "complete", street, pots_override=pots, stacks_override=stacks)
                step["after"]["final"] = "folded_out"
                steps.append(step)
                break
            after_pay = list(stacks_before)
            if action["kind"] == "call":
                after_pay[i] -= min(max(bets_before) - bets_before[i], stacks_before[i])
            elif action["kind"] in ("bet", "raise", "allin"):
                after_pay[i] -= to - bets_before[i]
            for j in range(n):
                if state.stacks[j] > after_pay[j]:
                    returned.append([ring[j], state.stacks[j] - after_pay[j]])
            with_chips = sum(1 for j in range(n) if state.statuses[j] and state.stacks[j] > 0)
            if with_chips < 2:
                step["after"] = snapshot(state, ring, "complete", street)
                step["after"]["final"] = "all_in_runout"
                steps.append(step)
                break
            if street_idx == 3:
                step["after"] = snapshot(state, ring, "complete", street)
                step["after"]["final"] = "showdown_reached"
                steps.append(step)
                break
            step["after"] = snapshot(state, ring, "awaiting_board", STREETS[street_idx + 1])
            steps.append(step)
        elif state.can_deal_board():
            count = 3 if street_idx == 0 else 1
            cards = [deck.pop() for _ in range(count)]
            _deal(state.deal_board, "".join(cards))
            street_idx += 1
            reopen.start_street(bb, 0)
            steps.append({"kind": "board", "cards": cards, "after": snapshot(state, ring, "betting", STREETS[street_idx])})
        else:
            raise RuntimeError(f"unexpected PokerKit state at seed {seed}")

    return {
        "id": f"seed{seed}",
        "seed": seed,
        "config": {"sb_chips": sb, "bb_chips": bb, "straddle_chips": straddle},
        "button": button,
        "hero": hero,
        "dealt": ring,
        "stacks_start": stacks_start,
        "hole_cards": hole,
        "steps": steps,
        "returned": returned,
        "stats": {
            "allins": n_allin,
            "short_allins": short_allin,
            "max_pots": max(len(s["after"]["pots"]) for s in steps),
            "allin_players": sum(steps[-1]["after"]["all_in"]),
        },
    }


def generate(count: int, first_seed: int = 1) -> tuple[list[dict], int]:
    hands, dropped, seed = [], 0, first_seed
    while len(hands) < count:
        h = generate_hand(seed)
        seed += 1
        if h is None:
            dropped += 1
            continue
        hands.append(h)
    return hands, dropped


def _run_hands(a: argparse.Namespace) -> None:
    hands, dropped = generate(a.count, a.seed)
    a.out.mkdir(parents=True, exist_ok=True)
    for k, h in enumerate(hands, 1):
        h["id"] = f"h{k:04d}"
        (a.out / f"{h['id']}.json").write_text(json.dumps(h, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"wrote {len(hands)} hands to {a.out} ({dropped} seeds dropped)")


def _run_sources(repo: Path, check: bool) -> None:
    import chart_sources

    if check:
        raw = (json.dumps(chart_sources.freeze_sources(repo), sort_keys=True, indent=2) + "\n").encode("utf-8")
        existing = (repo / "bench/spots/sources.json").read_bytes()
        if raw != existing:
            raise SystemExit("bench/spots/sources.json differs from the generator")
        print("bench/spots/sources.json matches the generator")
    else:
        chart_sources.write_sources(repo)
        print(f"wrote {repo / 'bench/spots/sources.json'}")


def _run_e2e(repo: Path, check: bool) -> None:
    import e2e_hands

    root = repo / "fixtures/hands/e2e"
    if check:
        e2e_hands.check_e2e(root)
        print(f"{root} matches the generator")
    else:
        e2e_hands.write_e2e(root)
        print(f"wrote fixtures/hands/e2e (50 records + manifest) to {root}")


# The pre-subcommand-table flat CLI (plan 1), still the documented regeneration command in
# docs/superpowers/plans/2026-09-10-plan-1-foundation.md:3637,3645,5318 ("tools/gen_fixtures.py
# --out fixtures/hands"). Task 17's fix round (M1) keeps it working as a compatibility alias for
# `hands` rather than silently breaking the published command: these are exactly its flags, so if
# the first token is one of them (never a subcommand name), `hands` is implied.
_LEGACY_HANDS_FLAGS = {"--out", "--count", "--seed"}


def _alias_legacy_flat_hands_command(argv: list[str]) -> list[str]:
    if argv and argv[0] in _LEGACY_HANDS_FLAGS:
        return ["hands", *argv]
    return argv


def main(argv: list[str] | None = None) -> int:
    # The subcommand table (plan 4 Task 17); `sources` is added here and extended by Task 19
    # with `e2e` (see that task's brief for the shared shape this mirrors).
    repo = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(prog="gen_fixtures", description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    p_hands = sub.add_parser("hands", help="generate PokerKit hand fixtures")
    p_hands.add_argument("--out", type=Path, required=True)
    p_hands.add_argument("--count", type=int, default=200)
    p_hands.add_argument("--seed", type=int, default=1)

    p_sources = sub.add_parser("sources", help="freeze chart provenance and node inventory")
    p_sources.add_argument(
        "--check", action="store_true",
        help="regenerate bench/spots/sources.json in memory and compare bytes instead of writing",
    )

    p_e2e = sub.add_parser("e2e", help="freeze the fifty recorded e2e hand fixtures + manifest")
    p_e2e.add_argument(
        "--check", action="store_true",
        help="regenerate fixtures/hands/e2e in memory and compare bytes instead of writing",
    )

    raw_argv = sys.argv[1:] if argv is None else argv
    args = parser.parse_args(_alias_legacy_flat_hands_command(raw_argv))
    if args.command == "sources":
        _run_sources(repo, args.check)
        return 0
    if args.command == "e2e":
        _run_e2e(repo, args.check)
        return 0
    _run_hands(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
