"""Regression tests for P1.T16 review finding R1: `gen_fixtures` must never install a process-wide
or pytest-wide warnings filter, and its two known, verified PokerKit `UserWarning` classes (a
fold-legality query on an actor who owes nothing, and a manually dealt card that isn't PokerKit's
own "recommended" next card -- see `gen_fixtures._UNRECOMMENDED_DEAL_RE` and `legal_triple`) must be
silenced only narrowly, around the specific operation, leaving every other warning visible.
"""
import importlib
import warnings

import gen_fixtures


def test_import_does_not_alter_global_warning_filters():
    """Re-importing the module must not leave any new entry in `warnings.filters` behind."""
    before = list(warnings.filters)
    importlib.reload(gen_fixtures)
    after = list(warnings.filters)
    assert after == before, "importing gen_fixtures must not install a process-wide warnings filter"


def test_generate_does_not_alter_global_warning_filters():
    """`generate()` uses `warnings.catch_warnings()` internally (via `_deal`); each such context
    must restore the filters it found on entry, so the caller's warning policy is untouched."""
    before = list(warnings.filters)
    gen_fixtures.generate(5, 1)
    after = list(warnings.filters)
    assert after == before, "calling generate() must not leave any warnings filter installed"


def test_unrelated_warning_still_visible_after_generate():
    """A warning that has nothing to do with the two known PokerKit noise sources must still
    surface normally after generate() has run -- proving no blanket filter is left active."""
    gen_fixtures.generate(5, 1)
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        warnings.warn("unrelated diagnostic for the P1.T16 regression guard", UserWarning)
    assert len(caught) == 1
    assert "unrelated diagnostic for the P1.T16 regression guard" in str(caught[0].message)


def test_generate_emits_no_warnings():
    """A run large enough to exercise both known warning sources (a zero-owed fold query on most
    streets, and every dealt hole/board card) must not let either escape `generate()`."""
    with warnings.catch_warnings(record=True) as caught:
        warnings.simplefilter("always")
        gen_fixtures.generate(60, 1)
    assert caught == [], f"unexpected warnings: {[str(w.message) for w in caught]}"
