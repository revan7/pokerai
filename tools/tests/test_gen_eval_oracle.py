import itertools
import random
import struct
from pathlib import Path

import pytest
from phevaluator import evaluate_cards

from gen_eval_oracle import FIVE_CARD_COUNT, write_7card_samples, five_card_prefix

FIXTURES = Path(__file__).resolve().parents[2] / "fixtures" / "eval"
SEVEN_CARD_RECORD_SIZE = 9
SEVEN_CARD_COUNT = 200_000
SEVEN_CARD_SEED = 20260910


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


def _five_card_fixture_path(fixtures_dir: Path) -> Path:
    """Locate the committed 5-card oracle under fixtures_dir, failing clearly if absent."""
    path = fixtures_dir / "phevaluator_5card.bin"
    if not path.exists():
        raise FileNotFoundError(f"missing required fixture: {path}")
    return path


def _seven_card_fixture_path(fixtures_dir: Path) -> Path:
    """Locate the committed 7-card oracle under fixtures_dir, failing clearly if absent."""
    path = fixtures_dir / "phevaluator_7card_200k.bin"
    if not path.exists():
        raise FileNotFoundError(f"missing required fixture: {path}")
    return path


def test_five_card_prefix_matches_committed_file():
    prefix = five_card_prefix(1000)
    assert len(prefix) == 2000
    combos = list(itertools.islice(itertools.combinations(range(52), 5), 1000))
    assert struct.unpack("<H", prefix[:2])[0] == evaluate_cards(*combos[0])

    committed = _five_card_fixture_path(FIXTURES)
    assert committed.stat().st_size == FIVE_CARD_COUNT * 2
    with committed.open("rb") as f:
        assert f.read(2000) == prefix


def test_seven_card_committed_fixture_matches_independent_construction():
    committed = _seven_card_fixture_path(FIXTURES)
    assert committed.stat().st_size == SEVEN_CARD_COUNT * SEVEN_CARD_RECORD_SIZE

    # Independently reconstruct the first records the same way write_7card_samples does:
    # a local random.Random(seed), rng.sample(range(52), 7) per record, then evaluate_cards.
    rng = random.Random(SEVEN_CARD_SEED)
    check_records = 200
    expected = bytearray()
    for _ in range(check_records):
        cards = rng.sample(range(52), 7)
        expected += bytes(cards) + struct.pack("<H", evaluate_cards(*cards))

    with committed.open("rb") as f:
        actual = f.read(len(expected))
    assert actual == bytes(expected)


def test_five_card_fixture_path_fails_clearly_when_artifact_missing(tmp_path):
    with pytest.raises(FileNotFoundError, match="phevaluator_5card.bin"):
        _five_card_fixture_path(tmp_path)


def test_seven_card_fixture_path_fails_clearly_when_artifact_missing(tmp_path):
    with pytest.raises(FileNotFoundError, match="phevaluator_7card_200k.bin"):
        _seven_card_fixture_path(tmp_path)
