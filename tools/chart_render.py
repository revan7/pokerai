"""Dev-only PDF page renderer for chart transcription (plan 3 Task 5; a reported deviation
outside that task's Files list -- see `docs/data/chart-transcription.md`).

The PokerCoaching/RangeConverter grid pages are raster images and vector paths, not text, so
transcribing them cell by cell needs a rendered page image. This environment has no poppler,
so pages are rendered with `pypdfium2` (pinned in `tools/pyproject.toml`) and written as PNG
with a small standard-library encoder (no Pillow). Rendering is deterministic: the same PDF,
page, DPI and crop always produce byte-identical PNG files.

Rendered PNGs are scratch inspection artifacts only -- write them outside the repository (or
under the gitignored `.superpowers/`); they are never committed and never read by
`chart_ingest.py`, whose `build`/`validate`/`verify` stay standard-library only.

CLI: `python tools/chart_render.py PDF FIRST LAST --dpi N --out DIR [--crop L,B,R,T]`, where
`FIRST`/`LAST` are 1-based page numbers (inclusive) and `--crop` trims `L,B,R,T` PDF points
from the left/bottom/right/top page edges before rendering (pdfium's own crop convention).
"""
from __future__ import annotations

import argparse
import struct
import sys
import zlib
from pathlib import Path


def _chunk(kind: bytes, payload: bytes) -> bytes:
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)


def png_bytes(width: int, height: int, rgb: bytes) -> bytes:
    """An 8-bit truecolor (RGB, no alpha) PNG of `width` x `height` pixels from tightly
    packed row-major `rgb` bytes. Filter type 0 on every row and a fixed zlib level, so the
    output is a pure function of its inputs."""
    if width <= 0 or height <= 0 or len(rgb) != width * height * 3:
        raise ValueError(f"rgb buffer of {len(rgb)} bytes does not match {width}x{height}x3")
    row = width * 3
    raw = b"".join(b"\x00" + rgb[y * row:(y + 1) * row] for y in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + _chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
        + _chunk(b"IDAT", zlib.compress(raw, 9))
        + _chunk(b"IEND", b"")
    )


def render_page(pdf_path: Path, page_number: int, dpi: int, crop: tuple[float, float, float, float] = (0, 0, 0, 0)) -> tuple[int, int, bytes]:
    """Renders one 1-based page at `dpi` (optionally cropped, in PDF points) and returns
    `(width, height, tightly packed RGB bytes)`."""
    import pypdfium2 as pdfium

    pdf = pdfium.PdfDocument(str(pdf_path))
    try:
        if not 1 <= page_number <= len(pdf):
            raise ValueError(f"page {page_number} out of range 1..={len(pdf)}")
        page = pdf[page_number - 1]
        bitmap = page.render(scale=dpi / 72, crop=crop, rev_byteorder=True)
        width, height, stride, channels = bitmap.width, bitmap.height, bitmap.stride, bitmap.n_channels
        if channels != 3:
            raise ValueError(f"expected a 3-channel RGB bitmap, got {channels} channels")
        buf = bytes(bitmap.buffer)
        rgb = b"".join(buf[y * stride:y * stride + width * 3] for y in range(height))
        return width, height, rgb
    finally:
        pdf.close()


def render_pages(pdf_path: Path, first: int, last: int, dpi: int, out_dir: Path, crop: tuple[float, float, float, float] = (0, 0, 0, 0)) -> list[Path]:
    """Renders pages `first..=last` (1-based) to `out_dir/pageNN_<dpi>dpi[_crop...].png` and
    returns the written paths in page order."""
    if first > last:
        raise ValueError(f"first page {first} is after last page {last}")
    out_dir.mkdir(parents=True, exist_ok=True)
    suffix = "" if crop == (0, 0, 0, 0) else "_crop" + "-".join(f"{c:g}" for c in crop)
    written = []
    for n in range(first, last + 1):
        width, height, rgb = render_page(pdf_path, n, dpi, crop)
        path = out_dir / f"page{n:02d}_{dpi}dpi{suffix}.png"
        path.write_bytes(png_bytes(width, height, rgb))
        written.append(path)
    return written


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(prog="chart_render")
    parser.add_argument("pdf", type=Path)
    parser.add_argument("first", type=int)
    parser.add_argument("last", type=int)
    parser.add_argument("--dpi", type=int, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--crop", default="0,0,0,0", help="L,B,R,T points trimmed from each page edge")
    args = parser.parse_args(argv)
    try:
        crop = tuple(float(v) for v in args.crop.split(","))
        if len(crop) != 4:
            raise ValueError("--crop needs exactly four comma-separated numbers")
        for path in render_pages(args.pdf, args.first, args.last, args.dpi, args.out, crop):
            print(path)
    except (ValueError, OSError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
