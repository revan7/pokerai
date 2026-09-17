import itertools
import struct
from pathlib import Path

from phevaluator import evaluate_cards

from gen_eval_oracle import write_7card_samples, five_card_prefix

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures" / "eval"


def test_phevaluator_encoding_matches_spec():
    assert evaluate_cards("As", "Ks", "Qs", "Js", "Ts") == 1
    assert evaluate_cards(51, 47, 43, 39, 35) == 1, "id = rank*4 + suit with c,d,h,s = 0..3"
    assert evaluate_cards("7h", "5d", "4c", "3s", "2h") == 7462
    assert evaluate_cards(0, 1, 2, 3, 4) < evaluate_cards(0, 1, 2, 4, 5), "quads beat a full house (lower is stronger)"


def test_seven_card_sample_format(tmp_path):
    out = tmp_path / "s.bin"
    write_7card_samples(out, 10, 20260910)
    data = out.read_bytes()
    assert len(data) == 90
    for k in range(10):
        rec = data[9 * k: 9 * k + 9]
        cards = list(rec[:7])
        assert len(set(cards)) == 7 and all(0 <= c < 52 for c in cards)
        assert struct.unpack("<H", rec[7:])[0] == evaluate_cards(*cards)
    assert write_7card_samples(tmp_path / "t.bin", 10, 20260910) is None
    assert (tmp_path / "t.bin").read_bytes() == data, "deterministic"


def test_five_card_prefix_matches_committed_file():
    prefix = five_card_prefix(1000)
    assert len(prefix) == 2000
    combos = list(itertools.islice(itertools.combinations(range(52), 5), 1000))
    assert struct.unpack("<H", prefix[:2])[0] == evaluate_cards(*combos[0])
    committed = FIXTURES / "phevaluator_5card.bin"
    if committed.exists():
        assert committed.stat().st_size == 2_598_960 * 2
        with committed.open("rb") as f:
            assert f.read(2000) == prefix
