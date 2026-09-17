"""Deterministic chart ingestion and validation tool (spec section 8.2, plan 3 Task 3).

Converts a hand-written *transcription* (a chart's grids, read cell-by-cell from a published
PDF/HTML source, plus an audit inventory of every candidate node looked for) into the
normalized, dense, action-major `Envelope` JSON that `core_preflop::decode` consumes, and the
`BundleInfo` manifest sidecar that goes with it. Charts never carry EV data (spec: "Charts
always emit `ChartRounded` and omit `evs`") and never substitute a nearest hand or infer a
menu action from a legend color's name -- every action/probability comes from an explicit,
already-resolved source value.

CLI: `fetch URL OUTPUT`, `build TRANSCRIPTION OUTPUT MANIFEST`, `validate ENVELOPE`,
`verify TRANSCRIPTION OUTPUT MANIFEST`. `fetch` is the only network-capable surface, and it
downloads exactly the URL it is given -- this tool never guesses or synthesizes a source URL,
and this task (P3.T3) does not itself call `fetch` against any real provider (Task 4 performs
the actual acquisition).

Standard library only. `class_names`/`build`/`validate` are pure and deterministic: given the
same input, `build` always produces byte-identical output (`encoded`'s `json.dumps(...,
indent=2, allow_nan=False) + "\\n"`, matching `gen_preflop_fixtures.py`'s `write_json`).
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from decimal import Decimal, InvalidOperation
from html.parser import HTMLParser
from pathlib import Path
from urllib.request import Request, urlopen

RANKS = "AKQJT98765432"
CLASS_ORDER = "A-2 row-major, section 4.1"
POSITIONS = ("UTG", "HJ", "CO", "BTN", "SB", "BB")

# 64 MiB, matching `core_preflop::validate::MAX_BUNDLE_BYTES` (spec section 8.2) -- both a
# `fetch` download cap and a local-file read cap (`bounded_read`), so a caller reading either
# an untrusted downloaded byte stream or a file on disk never hands an unbounded amount of
# data to a parser before any size check has run (standing ruling: bounded reads before
# parsing). A module-level name, not an inlined literal, so tests can shrink it without
# allocating a real 64 MiB buffer.
MAX_BUNDLE_BYTES = 64 * 1024 * 1024

# The Rust `BundleInfo` struct's exact fifteen field names (`crates/core-preflop/src/envelope.rs`),
# in its declared order. `manifest_for_chart` below emits precisely this key set -- never more,
# never fewer -- so a manifest this tool writes always deserializes into `BundleInfo`, which has
# no serde defaults. `tools/tests/test_chart_ingest.py` cross-checks this list against the actual
# Rust source, so a field added on one side without the other fails the Python suite first.
BUNDLE_INFO_FIELDS = [
    "bundle_id", "source", "depth_bb", "depths", "source_blinds", "rake_profile",
    "rake", "straddle", "version", "game", "ev_unit", "ev_reference",
    "license_note", "accuracy", "sha256",
]


# --- class ordering (spec section 4.1: 13x13 A-to-2 row-major grid) ---


def class_names() -> list[str]:
    """169 class names in row-major order: diagonal pairs, above-diagonal suited, below-
    diagonal offsuit (rank-descending within each suited/offsuit name), matching `class_order
    = "A-2 row-major, section 4.1"` and the grid index `c = row*13 + col` that `build` below
    reads a transcription's `rows` with.
    """
    names = []
    for i, a in enumerate(RANKS):
        for j, b in enumerate(RANKS):
            if i == j:
                names.append(a + a)
            elif i < j:
                names.append(a + b + "s")
            else:
                names.append(b + a + "o")
    return names


# --- token/position/amount validity (Task 1's rules, mirrored for the Python validator) ---


def valid_step(step: str, amount: int | None) -> bool:
    """A raise must carry a resolved positive size; every other token carries no amount at
    all. Mirrors `core_preflop::validate::valid_step` exactly (never a second, divergent copy
    of the rule's *meaning* -- the two languages just can't share one function body).
    """
    if step == "raise":
        return amount is not None and amount > 0
    if step in ("fold", "check", "call", "allin"):
        return amount is None
    return False


def _history_amount(v: int) -> int | None:
    """`0` means "no amount" in a `(position, step, amount)` history triple, mirroring
    `core_preflop::store`'s identical convention."""
    return v if v != 0 else None


# --- build: transcription (grid + legend) -> dense action-major envelope ---


def build(t: dict) -> dict:
    """Converts one transcription dict into the normalized `Envelope` dict Task 1's
    `core_preflop::decode` accepts. No nearest-hand substitution; no EV key; never infers a
    menu action from its legend color's name -- every cell's action-probability vector comes
    only from `legend[code]`, an explicit lookup.
    """
    out = {k: t[k] for k in ("bundle_id", "depth_bb", "rake_profile", "straddle")}
    out["class_order"] = CLASS_ORDER
    out["nodes"] = []
    for src in t["nodes"]:
        rows = src["rows"]
        if len(rows) != 13 or any(len(row) != 13 for row in rows):
            raise ValueError(f"grid shape: expected 13x13, got {len(rows)} rows")
        legend = src["legend"]
        actions = src["actions"]
        cells = []
        for row in rows:
            for code in row:
                if code not in legend:
                    raise ValueError(f"legend missing code {code!r} for node {src.get('history')!r}")
                cells.append(legend[code])
        if any(len(v) != len(actions) for v in cells):
            raise ValueError(f"legend action count mismatch for node {src.get('history')!r}")
        out["nodes"].append({
            "history": src["history"],
            "actor": src["actor"],
            "actions": actions,
            "weights": [list(col) for col in zip(*cells)],
            "unreachable_classes": src.get("unreachable_classes", []),
        })
    validate(out)
    return out


# --- validate: every structural/numeric rule the Rust loader also enforces ---


def _class_sums(node: dict) -> list[float]:
    return [math.fsum(col) for col in zip(*node["weights"])]


def _validate_node(index: int, n: dict, seen_keys: set[str]) -> None:
    def bad(msg: str) -> ValueError:
        return ValueError(f"node {index} (history {n.get('history')!r}): {msg}")

    actor = n.get("actor")
    if actor not in POSITIONS:
        raise bad(f"invalid actor {actor!r}")

    history = n.get("history")
    if history is None:
        raise bad("missing history")
    key = json.dumps(history, separators=(",", ":"))
    if key in seen_keys:
        raise bad("duplicate node history")
    seen_keys.add(key)

    actions = n.get("actions")
    if not actions:
        # Without this, `zip(*[])` below would yield nothing and a zero-action node would
        # pass this validator while the Rust loader rejects it (empty menu).
        raise bad("empty menu")

    for entry in history:
        if len(entry) != 3:
            raise bad(f"malformed history entry {entry!r}")
        pos, step, amount = entry
        if pos not in POSITIONS or not valid_step(step, _history_amount(amount)):
            raise bad(f"invalid history entry {entry!r}")

    menu: set[tuple[str, int | None]] = set()
    for a in actions:
        step = a.get("step")
        amount = a.get("to_bb_x1000")
        if not valid_step(step, amount):
            raise bad(f"invalid action token {(step, amount)!r}")
        token = (step, amount)
        if token in menu:
            raise bad(f"duplicate action kind and amount {token!r}")
        menu.add(token)

    unreachable = n.get("unreachable_classes", [])
    if len(set(unreachable)) != len(unreachable):
        raise bad("duplicate unreachable class")
    if any(c < 0 or c >= 169 for c in unreachable):
        raise bad("unreachable class out of range")

    weights = n.get("weights")
    if weights is None or len(weights) != len(actions):
        raise bad("actions shape")
    if any(len(row) != 169 for row in weights):
        raise bad("169 shape")

    for c, col in enumerate(zip(*weights)):
        if any(not math.isfinite(x) or not 0 <= x <= 1 for x in col):
            raise bad(f"probability bound at class {c}")
        s = math.fsum(col)
        if c in unreachable:
            if s != 0:
                raise bad(f"unreachable sum at class {c}: {s}")
        elif abs(s - 1) > 1e-3:
            raise bad(f"class {c} sum {s}")

    if "evs" in n:
        raise bad("chart EV forbidden")


def validate(e: dict) -> None:
    """Every structural and numeric rule spec section 8.2 states (shape, token/position/
    amount validity, uniqueness, per-class sibling sum, unreachable-exact-zero, no chart EV).
    The Rust loader (`core_preflop::validate`) remains the final boundary validator -- this is
    a second, independent implementation of the same rules, not a substitute for it.
    """
    if e.get("class_order") != CLASS_ORDER:
        raise ValueError(f"class order {e.get('class_order')!r} must be {CLASS_ORDER!r}")
    if not e.get("depth_bb"):
        raise ValueError("depth_bb must be nonzero")
    seen_keys: set[str] = set()
    for index, n in enumerate(e["nodes"]):
        _validate_node(index, n, seen_keys)


def validation_report(e: dict) -> dict:
    """Per-node 169 class sums plus the aggregate minimum/maximum across every node -- the
    report `validate`'s CLI prints (spec: "prints every node's 169 class sums and aggregate
    minimum/maximum"). Callers must run `validate(e)` first; this does not itself re-check
    shape and will raise a confusing error (or silently report nonsense) on a malformed input.
    """
    node_reports = []
    all_sums: list[float] = []
    for index, n in enumerate(e["nodes"]):
        sums = _class_sums(n)
        all_sums.extend(sums)
        node_reports.append({"node": index, "history": n["history"], "sums": sums})
    return {
        "nodes": node_reports,
        "aggregate_min": min(all_sums) if all_sums else None,
        "aggregate_max": max(all_sums) if all_sums else None,
    }


# --- manifest (BundleInfo) construction ---


def encoded(value: dict) -> bytes:
    """LF-only, deterministic JSON bytes (`indent=2`, `allow_nan=False`, trailing newline) --
    the exact byte shape `write_json` in `gen_preflop_fixtures.py` produces, so a committed
    envelope/manifest pair's hash is stable across platforms and re-runs.
    """
    return (json.dumps(value, indent=2, allow_nan=False) + "\n").encode("utf-8")


def manifest_for_chart(envelope: dict, transcription: dict) -> dict:
    """The `BundleInfo` manifest for a chart-sourced envelope: `source = "ChartTranscription"`
    and `ev_reference = "unverified"` are always forced, never taken from the transcription.
    `rake` is `None` unless the transcription documents one -- a chart that does not publish
    its rake is `null`, never silently assigned the PokerData profile as fact.
    """
    raw = encoded(envelope)
    manifest = {
        "bundle_id": envelope["bundle_id"],
        "source": "ChartTranscription",
        "depth_bb": envelope["depth_bb"],
        "depths": [envelope["depth_bb"]],
        "source_blinds": [0.5, 1.0],
        "rake_profile": envelope["rake_profile"],
        "rake": transcription.get("rake"),
        "straddle": envelope["straddle"],
        "version": 2,
        "game": "nl",
        "ev_unit": "source_sb",
        "ev_reference": "unverified",
        "license_note": transcription["license_note"],
        "accuracy": transcription.get("accuracy", "unverified"),
        "sha256": hashlib.sha256(raw).hexdigest(),
    }
    if manifest["rake"] is None and manifest["rake_profile"] != "undocumented":
        raise ValueError("a chart without a published rake must use rake_profile 'undocumented'")
    if sorted(manifest) != sorted(BUNDLE_INFO_FIELDS):
        raise ValueError("manifest keys do not match the Rust BundleInfo field set")
    return manifest


# --- bounded reads (standing ruling: bounded reads before parsing) ---


def bounded_read(path: Path) -> bytes:
    """Reads at most `MAX_BUNDLE_BYTES + 1` bytes from `path`, regardless of its reported
    size, and rejects anything over `MAX_BUNDLE_BYTES` before the caller ever parses it."""
    with open(path, "rb") as f:
        data = f.read(MAX_BUNDLE_BYTES + 1)
    if len(data) > MAX_BUNDLE_BYTES:
        raise ValueError(f"{path} exceeds the 64 MiB limit")
    return data


def load_json(path: Path) -> dict:
    return json.loads(bounded_read(path).decode("utf-8"))


# --- fetch: the tool's only network-capable surface ---


def fetch(url: str, output: Path) -> None:
    """Downloads exactly `url` (never a guessed or synthesized alternative) to `output`,
    bounded to `MAX_BUNDLE_BYTES`, and refuses HTML masquerading as a PDF (a publisher's
    download page returning an error/redirect page instead of the actual file). Prints the
    final redirected URL, byte count and SHA-256 -- the acquisition record a later task copies
    into `docs/data/chart-transcription.md`.
    """
    request = Request(url, headers={"User-Agent": "PokerAI-chart-ingest/1"})
    with urlopen(request, timeout=30) as response:
        data = response.read(MAX_BUNDLE_BYTES + 1)
        final_url = response.url
    if len(data) > MAX_BUNDLE_BYTES:
        raise ValueError("64 MiB limit exceeded while fetching")
    if output.suffix.lower() == ".pdf" and not data.startswith(b"%PDF-"):
        raise ValueError("publisher returned non-PDF; inspect download page links")
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_bytes(data)
    print(json.dumps({"url": final_url, "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data)}))


# --- HTML anchor lookup (a fetched download page's link text -> href) ---


class _AnchorCollector(HTMLParser):
    """Collects `(visible text, href)` for every `<a href=...>...</a>` in a page. Flat text
    only (no nested-tag reconstruction) -- all a chart source's download-page link needs.
    Never fetches anything itself; operates on HTML already retrieved by `fetch`.
    """

    def __init__(self) -> None:
        super().__init__()
        self._current_href: str | None = None
        self._current_text: list[str] = []
        self.links: list[tuple[str, str]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag != "a":
            return
        href = dict(attrs).get("href")
        if href is not None:
            self._current_href = href
            self._current_text = []

    def handle_data(self, data: str) -> None:
        if self._current_href is not None:
            self._current_text.append(data)

    def handle_endtag(self, tag: str) -> None:
        if tag == "a" and self._current_href is not None:
            self.links.append(("".join(self._current_text).strip(), self._current_href))
            self._current_href = None
            self._current_text = []


def extract_links(html: str) -> list[tuple[str, str]]:
    parser = _AnchorCollector()
    parser.feed(html)
    return parser.links


def find_anchor(html: str, text: str) -> str | None:
    """The `href` of the first anchor whose visible text contains `text` (exact, case-
    sensitive substring match -- never a fuzzy or color-based match). `None` if absent. Used
    to follow a chart source's article page to its actual download link, e.g. the anchor whose
    text is exactly "6 max 200bb 500z GTO Ranges" (spec section 8.2's acquisition step).
    """
    for anchor_text, href in extract_links(html):
        if text in anchor_text:
            return href
    return None


# --- bb -> to_bb_x1000 exact conversion (decimal, never float, arithmetic) ---


def bb_to_x1000(bb: str | int | float) -> int:
    """Exact bb -> `to_bb_x1000` conversion (spec section 8.2's action-size unit) via
    `decimal.Decimal`, so a resolved size read off a chart (e.g. `"8.75"`) converts exactly --
    unlike `float(bb) * 1000`, which can drift for a value that is not exact in binary. Rejects
    a non-terminating/invalid decimal and a non-positive result, matching `valid_step`'s
    requirement that a raise always carries a resolved positive size.
    """
    try:
        value = Decimal(str(bb)) * 1000
    except InvalidOperation as exc:
        raise ValueError(f"bb size {bb!r} is not a valid decimal number") from exc
    to_bb_x1000 = int(value)
    if value != to_bb_x1000 or to_bb_x1000 <= 0:
        raise ValueError(f"bb size {bb!r} does not convert to a positive integer to_bb_x1000 value")
    return to_bb_x1000


# --- verify: rebuild from the transcription and compare exactly ---


def _first_mismatch(built: dict, disk: dict) -> tuple[int | None, int | None]:
    """The first `(node_index, class_index)` at which `built` and `disk` diverge, walking
    node-then-class so a byte-level mismatch can be reported at the granularity `verify`'s
    exact-exit-diagnostic requirement calls for ("its nonzero exit includes the node and class
    name"). `class_index` is `None` when the divergence is not localizable to one class (e.g.
    a node-count or action-count difference).
    """
    built_nodes = built.get("nodes", [])
    disk_nodes = disk.get("nodes", [])
    for i, (bn, dn) in enumerate(zip(built_nodes, disk_nodes)):
        if bn == dn:
            continue
        b_weights, d_weights = bn.get("weights", []), dn.get("weights", [])
        if len(b_weights) == len(d_weights):
            for c in range(169):
                b_col = [row[c] for row in b_weights] if all(len(row) > c for row in b_weights) else None
                d_col = [row[c] for row in d_weights] if all(len(row) > c for row in d_weights) else None
                if b_col != d_col:
                    return i, c
        return i, None
    if len(built_nodes) != len(disk_nodes):
        return min(len(built_nodes), len(disk_nodes)), None
    return None, None


def verify(transcription_path: Path, output_path: Path, manifest_path: Path) -> str:
    """Rebuilds the envelope and manifest from `transcription_path` and compares them exactly
    against what is already on disk at `output_path`/`manifest_path`, then compares the
    transcription's `covered` inventory keys against the rebuilt envelope's actual node keys.
    Raises `ValueError` (never repairs anything) on any mismatch; returns a one-line success
    summary otherwise.
    """
    t = load_json(transcription_path)
    envelope = build(t)
    manifest = manifest_for_chart(envelope, t)

    disk_output_bytes = bounded_read(output_path)
    rebuilt_output_bytes = encoded(envelope)
    if rebuilt_output_bytes != disk_output_bytes:
        node_idx, class_idx = _first_mismatch(envelope, json.loads(disk_output_bytes.decode("utf-8")))
        class_name = class_names()[class_idx] if class_idx is not None else "?"
        raise ValueError(f"{output_path}: rebuilt envelope differs from disk at node {node_idx}, class {class_name}")

    disk_manifest = load_json(manifest_path)
    if manifest != disk_manifest:
        mismatched = sorted(k for k in BUNDLE_INFO_FIELDS if manifest.get(k) != disk_manifest.get(k))
        raise ValueError(
            f"{manifest_path}: manifest mismatch in fields {mismatched} "
            f"(rebuilt sha256 {manifest['sha256']} vs on-disk {disk_manifest.get('sha256')})"
        )

    covered_keys = {
        json.dumps(row["history"], separators=(",", ":"))
        for row in t.get("inventory", [])
        if row.get("status") == "covered"
    }
    envelope_keys = {json.dumps(n["history"], separators=(",", ":")) for n in envelope["nodes"]}
    if covered_keys != envelope_keys:
        raise ValueError(
            "inventory/envelope key mismatch: "
            f"covered-only {sorted(covered_keys - envelope_keys)}, "
            f"envelope-only {sorted(envelope_keys - covered_keys)}"
        )

    total_classes = len(envelope["nodes"]) * 169
    return f"verified {len(envelope['nodes'])} nodes, {total_classes} classes"


# --- CLI wiring ---


def _cmd_build(transcription_path: Path, output_path: Path, manifest_path: Path) -> None:
    t = load_json(transcription_path)
    envelope = build(t)
    manifest = manifest_for_chart(envelope, t)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_bytes(encoded(envelope))
    manifest_path.parent.mkdir(parents=True, exist_ok=True)
    manifest_path.write_bytes(encoded(manifest))


def _cmd_validate(envelope_path: Path) -> None:
    envelope = load_json(envelope_path)
    validate(envelope)
    report = validation_report(envelope)
    for node_report in report["nodes"]:
        print(json.dumps(node_report))
    print(json.dumps({"aggregate_min": report["aggregate_min"], "aggregate_max": report["aggregate_max"]}))


def _cmd_verify(transcription_path: Path, output_path: Path, manifest_path: Path) -> None:
    print(verify(transcription_path, output_path, manifest_path))


def _build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="chart_ingest")
    sub = parser.add_subparsers(dest="command", required=True)

    p_fetch = sub.add_parser("fetch", help="download a public source URL to a local file")
    p_fetch.add_argument("url")
    p_fetch.add_argument("output", type=Path)

    p_build = sub.add_parser("build", help="convert a transcription into an envelope + manifest")
    p_build.add_argument("transcription", type=Path)
    p_build.add_argument("output", type=Path)
    p_build.add_argument("manifest", type=Path)

    p_validate = sub.add_parser("validate", help="validate an envelope and print its class-sum report")
    p_validate.add_argument("envelope", type=Path)

    p_verify = sub.add_parser("verify", help="rebuild from a transcription and compare exactly to disk")
    p_verify.add_argument("transcription", type=Path)
    p_verify.add_argument("output", type=Path)
    p_verify.add_argument("manifest", type=Path)

    return parser


def main(argv: list[str] | None = None) -> int:
    args = _build_arg_parser().parse_args(argv)
    try:
        if args.command == "fetch":
            fetch(args.url, args.output)
        elif args.command == "build":
            _cmd_build(args.transcription, args.output, args.manifest)
        elif args.command == "validate":
            _cmd_validate(args.envelope)
        elif args.command == "verify":
            _cmd_verify(args.transcription, args.output, args.manifest)
    except (ValueError, OSError, json.JSONDecodeError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
