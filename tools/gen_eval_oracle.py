"""Generate fixtures/eval oracles with phevaluator (spec 13.0, 13.1).

phevaluator_5card.bin: 2,598,960 little-endian u16 ranks, one per 5-card combination in
itertools.combinations(range(52), 5) order. phevaluator_7card_<n>.bin: records of 9 bytes,
seven card ids (rank*4 + suit) as sampled, then the u16 LE rank. Lower rank = stronger hand.
"""
from __future__ import annotations

import argparse
import itertools
import random
import struct
from pathlib import Path

from phevaluator import evaluate_cards

FIVE_CARD_COUNT = 2_598_960


def five_card_prefix(count: int) -> bytes:
    out = bytearray()
    for combo in itertools.islice(itertools.combinations(range(52), 5), count):
        out += struct.pack("<H", evaluate_cards(*combo))
    return bytes(out)


def write_5card(out: Path) -> None:
    with out.open("wb") as f:
        for combo in itertools.combinations(range(52), 5):
            f.write(struct.pack("<H", evaluate_cards(*combo)))


def write_7card_samples(out: Path, count: int, seed: int) -> None:
    rng = random.Random(seed)
    with out.open("wb") as f:
        for _ in range(count):
            cards = rng.sample(range(52), 7)
            f.write(bytes(cards) + struct.pack("<H", evaluate_cards(*cards)))


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out-dir", type=Path, default=Path("fixtures/eval"))
    ap.add_argument("--samples", type=int, default=200_000)
    ap.add_argument("--samples-name", default="phevaluator_7card_200k.bin")
    ap.add_argument("--seed", type=int, default=20260910)
    ap.add_argument("--skip-5card", action="store_true")
    a = ap.parse_args()
    a.out_dir.mkdir(parents=True, exist_ok=True)
    if not a.skip_5card:
        write_5card(a.out_dir / "phevaluator_5card.bin")
        print(f"wrote {FIVE_CARD_COUNT} five-card ranks")
    write_7card_samples(a.out_dir / a.samples_name, a.samples, a.seed)
    print(f"wrote {a.samples} seven-card samples to {a.samples_name}")


if __name__ == "__main__":
    main()
