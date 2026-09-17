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
import warnings
from pathlib import Path

from pokerkit import Automation, Mode, NoLimitTexasHoldem

warnings.simplefilter("ignore")  # Mode.CASH_GAME warns on folds that face no wager; never recorded as legal

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


def legal_triple(state) -> dict:
    i = state.actor_index
    facing = max(state.bets)
    owed = facing - state.bets[i]
    fold = bool(state.can_fold()) and owed > 0
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
        state.deal_hole(h)
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
            state.deal_board("".join(cards))
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


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--count", type=int, default=200)
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    hands, dropped = generate(a.count, a.seed)
    a.out.mkdir(parents=True, exist_ok=True)
    for k, h in enumerate(hands, 1):
        h["id"] = f"h{k:04d}"
        (a.out / f"{h['id']}.json").write_text(json.dumps(h, separators=(",", ":")) + "\n", encoding="utf-8")
    print(f"wrote {len(hands)} hands to {a.out} ({dropped} seeds dropped)")


if __name__ == "__main__":
    main()
