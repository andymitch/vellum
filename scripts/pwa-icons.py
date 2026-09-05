#!/usr/bin/env python3
"""Regenerate the web app's icons in public/ from the app icon (#221/#222).

One-off tool, run by hand when src-tauri/icons/icon.png changes:

    pip install pillow && python3 scripts/pwa-icons.py

The app icon is a rounded tile on a transparent background, which is wrong for
two of the four sizes we need:

  - iOS applies its own squircle mask to apple-touch-icon, so transparent
    corners come out black. It needs a full-bleed square.
  - A `maskable` icon may be cropped to a circle, so the glyph has to sit well
    inside a safe zone rather than filling the tile.

Both are built by isolating the dark glyph and centring it on the tile's own
colour. favicon/pwa-192/pwa-512 keep the rounded tile as-is.
"""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src-tauri" / "icons" / "icon.png"
OUT = ROOT / "public"

src = Image.open(SOURCE).convert("RGBA")
w, h = src.size
# The tile's own colour, sampled well inside its rounded edge.
cream = src.getpixel((w // 2, int(h * 0.12)))

flat = Image.new("RGBA", (w, h), cream)
flat.alpha_composite(src)
# Anything far from the tile colour is glyph; its bounding box is what gets
# placed, so the safe-zone fraction below means what it says.
glyph = flat.crop(flat.convert("L").point(lambda v: 255 if v < 128 else 0).getbbox())


def centred(size: int, fraction: float) -> Image.Image:
    """The glyph on a full-bleed square, sized to `fraction` of the edge."""
    canvas = Image.new("RGBA", (size, size), cream)
    scale = (size * fraction) / max(glyph.size)
    scaled = glyph.resize(
        (max(1, round(glyph.width * scale)), max(1, round(glyph.height * scale))),
        Image.LANCZOS,
    )
    canvas.paste(scaled, ((size - scaled.width) // 2, (size - scaled.height) // 2), scaled)
    return canvas


src.resize((64, 64), Image.LANCZOS).save(OUT / "favicon.png")
src.resize((192, 192), Image.LANCZOS).save(OUT / "pwa-192.png")
src.resize((512, 512), Image.LANCZOS).save(OUT / "pwa-512.png")
centred(180, 0.55).save(OUT / "apple-touch-icon.png")
centred(512, 0.45).save(OUT / "pwa-maskable-512.png")
print(f"wrote 5 icons to {OUT}")
