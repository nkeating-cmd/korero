#!/usr/bin/env python3
"""Generate Kōrero's app and tray icons from one geometric definition.

Kōrero 1.45.2 — the refined macron-K mark. Run from the repo root:

    python3 scripts/gen-icons.py

Requires Pillow. Everything is drawn at 1024 px and downsampled, so the
output is deterministic and easy to tweak: change the geometry in `glyph()`
and re-run.

Outputs
  src-tauri/icons/   app icon in every size Tauri bundles (ICO largest-first:
                     Tauri builds the Windows window/taskbar icon from the
                     FIRST ico frame, so a 16 px first frame looks blurry)
  src-tauri/resources/tray_*.png   tray icons, 64 px. Files WITHOUT `_dark`
                     hold a dark glyph (for light taskbars); files WITH
                     `_dark` hold a light glyph (for dark taskbars).
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
RES = ROOT / "src-tauri" / "resources"

S = 1024  # master canvas
WHITE = (255, 255, 255, 255)
GLYPH_DARK = (52, 52, 56, 255)  # dark glyph for light taskbars
KORERO_BLUE = (10, 132, 255, 255)  # --color-logo-primary
TILE = (21, 21, 23, 255)  # --color-logo-stroke / background-elev
RECORDING = (255, 95, 95, 255)
TRANSCRIBING = (77, 163, 255, 255)


def glyph(size, fg, macron=None, box=(0, 0, S, S)):
    """Draw the macron-K into `box` (in master units) on a transparent canvas."""
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    x0, y0, x1, y1 = box
    u = (x1 - x0) / 512.0  # geometry below is authored on a 512 grid

    def P(x, y):
        return (x0 + x * u, y0 + y * u)

    d.rounded_rectangle((*P(96, 40), *P(416, 104)), radius=32 * u, fill=macron or fg)
    d.rectangle((*P(96, 152), *P(186, 472)), fill=fg)
    d.polygon(
        [P(186, 312), P(370, 152), P(470, 152), P(250, 340), P(470, 472), P(360, 472), P(186, 360)],
        fill=fg,
    )
    return im


def with_dot(im, colour):
    """Add a state dot bottom-right, cut out from the glyph with a clear ring."""
    cx, cy, r, gap = 784, 784, 208, 68
    ring = Image.new("L", (S, S), 0)
    ImageDraw.Draw(ring).ellipse((cx - r - gap, cy - r - gap, cx + r + gap, cy + r + gap), fill=255)
    alpha = im.getchannel("A")
    alpha.paste(0, mask=ring)
    im.putalpha(alpha)
    ImageDraw.Draw(im).ellipse((cx - r, cy - r, cx + r, cy + r), fill=colour)
    return im


def app_icon(px):
    """Dark rounded tile, white K, Kōrero-blue macron."""
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    m = 40  # transparent margin, in line with platform icon grids
    ImageDraw.Draw(im).rounded_rectangle((m, m, S - m, S - m), radius=230, fill=TILE)
    # Small sizes get a bigger glyph so it survives downsampling.
    pad = 230 if px >= 64 else 170
    im.alpha_composite(glyph(px, WHITE, macron=KORERO_BLUE, box=(pad, pad, S - pad, S - pad)))
    return im.resize((px, px), Image.LANCZOS)


def tray(fg, dot):
    im = glyph(S, fg)
    if dot:
        im = with_dot(im, dot)
    return im.resize((64, 64), Image.LANCZOS)


def write_ico(path, frames):
    """Write PNG-compressed ICO frames in the order given (largest first)."""
    import io
    import struct

    blobs = []
    for f in frames:
        buf = io.BytesIO()
        f.save(buf, format="PNG")
        blobs.append(buf.getvalue())
    header = struct.pack("<HHH", 0, 1, len(frames))
    offset = 6 + 16 * len(frames)
    entries = b""
    for f, blob in zip(frames, blobs):
        w, h = f.size
        entries += struct.pack("<BBBBHHII", w % 256, h % 256, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
    path.write_bytes(header + entries + b"".join(blobs))


def main():
    # --- app icons -----------------------------------------------------
    sizes = {
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "icon.png": 512,
        "icon-1024.png": 1024,
        "logo.png": 1024,
        "StoreLogo.png": 50,
    }
    for n in (30, 44, 71, 89, 107, 142, 150, 284, 310):
        sizes[f"Square{n}x{n}Logo.png"] = n
    for name, px in sizes.items():
        app_icon(px).save(ICONS / name)

    # ICO: write frames largest-first (see module docstring).
    # Pillow's own ICO writer sorts frames smallest-first, which reintroduces
    # the blurry-taskbar bug, so the container is written by hand.
    write_ico(ICONS / "icon.ico", [app_icon(px) for px in (256, 128, 64, 48, 32, 16)])
    app_icon(1024).save(ICONS / "icon.icns", format="ICNS")

    # --- tray icons ----------------------------------------------------
    for state, dot in (("idle", None), ("recording", RECORDING), ("transcribing", TRANSCRIBING)):
        tray(GLYPH_DARK, dot).save(RES / f"tray_{state}.png")  # for light taskbars
        tray(WHITE, dot).save(RES / f"tray_{state}_dark.png")  # for dark taskbars
    print("icons written")


if __name__ == "__main__":
    main()
