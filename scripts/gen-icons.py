#!/usr/bin/env python3
"""Generate Kōrero's app and tray icons from one geometric definition.

Kōrero 1.48 — the bubble-ō mark: a speech bubble holding an ō. Run from the
repo root:

    python3 scripts/gen-icons.py

Requires Pillow. Everything is drawn at 1024 px and downsampled, so the
output is deterministic and easy to tweak: change the geometry in
`bubble_o()` and re-run.

Outputs
  src-tauri/icons/   app icon in every size Tauri bundles (ICO largest-first:
                     Tauri builds the Windows window/taskbar icon from the
                     FIRST ico frame, so a 16 px first frame looks blurry)
  src-tauri/resources/tray_*.png   tray icons, 64 px. Files WITHOUT `_dark`
                     are for LIGHT taskbars; files WITH `_dark` are for DARK
                     taskbars. The bubble colour carries the state (blue idle,
                     red recording, amber transcribing) and the ō changes
                     shape too (ring, ring, progress arc), so state does not
                     rest on colour alone.

Contrast rule (checked by `icon_contrast_tests` in src-tauri/src/tray.rs):
every bubble is at least 3:1 against its taskbar (Windows 11 light ~#F3F3F3,
dark ~#202020) and every ō at least 3:1 against its bubble (WCAG 1.4.11,
non-text contrast). That is why light-taskbar transcribing uses a darker
amber with a white ō, and dark-taskbar transcribing a bright amber with a
dark ō.
"""

from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
RES = ROOT / "src-tauri" / "resources"

S = 1024  # master canvas
WHITE = (255, 255, 255, 255)
INK = (26, 26, 26, 255)
TILE = (21, 21, 23, 255)  # --color-logo-stroke / background-elev
KORERO_BLUE = (10, 132, 255, 255)  # --color-logo-primary

# (bubble colour, ō colour) per taskbar theme and state.
TRAY = {
    "light": {
        "idle": (KORERO_BLUE, WHITE),  # 3.3:1 on #F3F3F3, ō 3.7:1
        "recording": ((229, 72, 77, 255), WHITE),  # 3.5:1, ō 3.9:1
        "transcribing": ((184, 110, 0, 255), WHITE),  # 3.6:1, ō 4.0:1
    },
    "dark": {
        "idle": ((47, 143, 255, 255), WHITE),  # 5.0:1 on #202020, ō 3.2:1
        "recording": ((242, 85, 90, 255), WHITE),  # 4.8:1, ō 3.4:1
        "transcribing": ((255, 178, 36, 255), INK),  # 9.0:1, ō 9.7:1
    },
}


def bubble_o(fill, glyph, state="idle", box=(0, 0, S, S)):
    """Draw the bubble-ō into `box` (master units) on a transparent canvas.

    Geometry is authored on a 32-unit grid (the tray's own scale).
    """
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    x0, y0, x1, y1 = box
    u = (x1 - x0) / 32.0

    def P(x, y):
        return (x0 + x * u, y0 + y * u)

    # Bubble: rounded body plus a tail at bottom-left.
    d.rounded_rectangle((*P(2.5, 2.5), *P(29.5, 23.5)), radius=6 * u, fill=fill)
    d.polygon([P(15.5, 21.5), P(14.5, 23.5), P(8.3, 29.3), P(8.3, 21.5)], fill=fill)

    # ō: macron bar ...
    d.rounded_rectangle((*P(12, 5.4), *P(20, 7.7)), radius=1.15 * u, fill=glyph)
    # ... and the o as a ring, or a 280° progress arc while transcribing.
    cx, cy = P(16, 15.3)
    r_mid, w = 4.2 * u, 2.3 * u
    ring_box = (cx - r_mid - w / 2, cy - r_mid - w / 2, cx + r_mid + w / 2, cy + r_mid + w / 2)
    if state == "transcribing":
        import math

        start, end = -90, 190  # Pillow angles run clockwise from 3 o'clock
        d.arc(ring_box, start, end, fill=glyph, width=round(w))
        for a in (start, end):  # round caps
            ex = cx + r_mid * math.cos(math.radians(a))
            ey = cy + r_mid * math.sin(math.radians(a))
            d.ellipse((ex - w / 2, ey - w / 2, ex + w / 2, ey + w / 2), fill=glyph)
    else:
        d.ellipse(ring_box, outline=glyph, width=round(w))
    return im


def app_icon(px):
    """Dark rounded tile holding the Kōrero-blue bubble with a white ō."""
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    m = 40  # transparent margin, in line with platform icon grids
    ImageDraw.Draw(im).rounded_rectangle((m, m, S - m, S - m), radius=230, fill=TILE)
    # Small sizes get a bigger mark so it survives downsampling.
    pad = 170 if px >= 64 else 110
    # The bubble's tail makes the mark bottom-heavy; nudge it up to sit
    # optically centred in the tile.
    lift = 0.03 * (S - 2 * pad)
    im.alpha_composite(bubble_o(KORERO_BLUE, WHITE, box=(pad, pad - lift, S - pad, S - pad - lift)))
    return im.resize((px, px), Image.LANCZOS)


def tray(theme, state):
    fill, glyph = TRAY[theme][state]
    return bubble_o(fill, glyph, state).resize((64, 64), Image.LANCZOS)


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
    for state in ("idle", "recording", "transcribing"):
        tray("light", state).save(RES / f"tray_{state}.png")  # for light taskbars
        tray("dark", state).save(RES / f"tray_{state}_dark.png")  # for dark taskbars
    print("icons written")


if __name__ == "__main__":
    main()
