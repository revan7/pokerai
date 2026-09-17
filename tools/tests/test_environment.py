"""The oracles are pinned: a different version silently changes every generated fixture."""
import sys
from importlib import metadata


def test_python_is_312_or_newer():
    assert sys.version_info[:2] >= (3, 12), sys.version


def test_oracle_versions_are_pinned():
    assert metadata.version("pokerkit") == "0.7.5"
    assert metadata.version("phevaluator") == "0.6.0"


def test_oracles_import_and_expose_the_entry_points_the_generators_use():
    from phevaluator import evaluate_cards
    from pokerkit import Automation, Mode, NoLimitTexasHoldem

    assert evaluate_cards("As", "Ks", "Qs", "Js", "Ts") == 1, "lower is stronger; 1 is the royal flush"
    assert hasattr(NoLimitTexasHoldem, "create_state")
    assert Mode.CASH_GAME is not None
    assert Automation.BET_COLLECTION is not None
