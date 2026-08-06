#!/usr/bin/env python3
"""Generate WinKnife's application icon.

Committed as a script rather than only as binaries so the icon is reproducible
and reviewable: anybody can see what the pixels are supposed to be, change a
colour and re-run, instead of having to trust a blob that arrived in a commit.

Pure standard library — `zlib` and `struct` are enough to write a PNG, and a
Vista-or-later `.ico` is a container of PNGs. No image library, no download.

    python scripts/make-icons.py

The mark: a dark rounded square, and a blade. It reads at 16 px, which is the
only size that is genuinely hard, and it is not a gradient-and-glow logo — this
app's whole positioning is that it is not trying to look alarming.
"""

from __future__ import annotations

import math
import struct
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"

# The app's own surfaces, so the icon belongs to the same product as the window.
BACKGROUND = (18, 22, 29, 255)      # surface-0
BLADE = (232, 234, 240, 255)        # ink
EDGE = (122, 162, 247, 255)         # accent
HANDLE = (75, 92, 116, 255)         # surface-3


def rounded_square_alpha(size: int, x: float, y: float) -> float:
    """Coverage of a rounded square at a point, antialiased at the corners."""
    radius = size * 0.22
    half = size / 2
    dx = abs(x - half) - (half - radius)
    dy = abs(y - half) - (half - radius)
    dx = max(dx, 0.0)
    dy = max(dy, 0.0)
    distance = math.hypot(dx, dy)
    return clamp01(radius - distance + 0.5)


def clamp01(value: float) -> float:
    return 0.0 if value < 0 else 1.0 if value > 1 else value


# Where the blade's spine runs, as a line through normalised coordinates: it
# rises from the lower left to the upper right, which is how a knife is held.
SPINE_AT_ZERO = 0.70
SPINE_SLOPE = -0.42
# The blade starts here and tapers to a point here.
BLADE_START = 0.26
BLADE_TIP = 0.88
# How deep the blade is at its widest, right at the handle.
BLADE_DEPTH = 0.24


def spine_at(u: float) -> float:
    return SPINE_AT_ZERO + SPINE_SLOPE * u


def blade_alpha(size: int, x: float, y: float) -> tuple[float, float, float]:
    """Coverage of the blade, its cutting edge and the handle.

    A wedge: a straight spine along the top, and a belly below it that is
    deepest at the handle and tapers to nothing at the tip. Written in
    normalised coordinates so every size renders the same drawing rather than a
    differently-proportioned one.
    """
    u = x / size
    v = y / size
    feather = 1.4 / size

    spine = spine_at(u)
    # Depth tapers linearly to zero at the tip, which is what makes it a blade
    # rather than a stripe.
    taper = clamp01((BLADE_TIP - u) / (BLADE_TIP - BLADE_START))
    depth = BLADE_DEPTH * taper * taper ** 0.4
    belly = spine + depth

    below_spine = clamp01((v - spine + feather) / (2 * feather))
    above_belly = clamp01((belly - v + feather) / (2 * feather))
    within = clamp01((u - BLADE_START + feather) / (2 * feather)) * clamp01(
        (BLADE_TIP - u + feather) / (2 * feather)
    )
    body = below_spine * above_belly * within

    # The cutting edge: a bright band along the belly.
    edge = clamp01(1.0 - abs(v - belly) / (2.0 * feather)) * within * below_spine

    # The handle sits on the same axis, overlapping the blade's base so the two
    # read as one object.
    hu = clamp01((u - 0.09 + feather) / (2 * feather)) * clamp01(
        (BLADE_START + 0.05 - u + feather) / (2 * feather)
    )
    handle_centre = spine_at(u) + 0.11
    hv = clamp01((0.085 - abs(v - handle_centre) + feather) / (2 * feather))
    handle = hu * hv

    return body, edge, handle


def blend(base: tuple[int, int, int, int], over: tuple[int, int, int, int], alpha: float):
    a = clamp01(alpha)
    return tuple(round(base[i] * (1 - a) + over[i] * a) for i in range(4))


def render(size: int) -> bytes:
    """Raw RGBA rows for one square icon."""
    rows = bytearray()
    for py in range(size):
        rows.append(0)  # PNG filter type 0 for this scanline
        for px in range(size):
            x = px + 0.5
            y = py + 0.5
            coverage = rounded_square_alpha(size, x, y)
            pixel = (0, 0, 0, 0)
            pixel = blend(pixel, BACKGROUND, coverage)

            body, edge, handle = blade_alpha(size, x, y)
            pixel = blend(pixel, HANDLE, handle * coverage)
            pixel = blend(pixel, BLADE, body * coverage)
            pixel = blend(pixel, EDGE, edge * coverage)
            rows.extend(pixel)
    return bytes(rows)


def png(size: int) -> bytes:
    """A minimal RGBA PNG."""

    def chunk(kind: bytes, payload: bytes) -> bytes:
        return (
            struct.pack(">I", len(payload))
            + kind
            + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
        )

    header = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(render(size), 9))
        + chunk(b"IEND", b"")
    )


def ico(sizes: list[int]) -> bytes:
    """A Vista-or-later `.ico`: a directory of embedded PNGs."""
    images = [(size, png(size)) for size in sizes]
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    directory = b""
    body = b""
    for size, data in images:
        directory += struct.pack(
            "<BBBBHHII",
            0 if size >= 256 else size,
            0 if size >= 256 else size,
            0,
            0,
            1,
            32,
            len(data),
            offset,
        )
        body += data
        offset += len(data)
    return header + directory + body


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)

    # The names `tauri.conf.json` and the Windows Store manifest ask for.
    square = {
        "32x32.png": 32,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }
    for name, size in square.items():
        (ICONS / name).write_bytes(png(size))
        print(f"  {name} ({size}x{size})")

    (ICONS / "icon.ico").write_bytes(ico([16, 32, 48, 64, 128, 256]))
    print("  icon.ico (16, 32, 48, 64, 128, 256)")
    print(f"wrote {len(square) + 1} files to {ICONS}")


if __name__ == "__main__":
    main()
