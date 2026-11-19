"""Draws the two app icons as multi-size .ico files: a plain gray rounded tile (own design, not the Rust logo)
with one flat glyph: a folder (Rust File Explorer) or three bars (Rust Task Manager). Minimal: no lines, no shine.
Usage: python tools/make_icons.py [light|graphite]
  -> installer/wfiles.ico, installer/wintask.ico (+ copies in app/assets, fm/assets), design/build/icons-v3.png"""
import os
import shutil
import sys

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.join(HERE, "..")
S = 1024  # draw large, downscale for clean small sizes
SIZES = [(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)]
SHADES = {
    # name: (tile top, tile bottom, glyph)
    "graphite": ((86, 90, 97, 255), (60, 63, 69, 255), (238, 239, 241, 255)),
    "light": ((236, 237, 239, 255), (214, 216, 220, 255), (72, 76, 83, 255)),
}


def tile(top, bot):
    grad = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        f = y / (S - 1)
        gd.line([(0, y), (S, y)], fill=tuple(int(top[i] + (bot[i] - top[i]) * f) for i in range(4)))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([48, 48, S - 48, S - 48], radius=230, fill=255)
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    im.paste(grad, (0, 0), mask)
    return im


def explorer(shade):
    top, bot, ink = SHADES[shade]
    im = tile(top, bot)
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([250, 300, 500, 420], radius=44, fill=ink)   # tab
    d.rounded_rectangle([250, 370, 774, 724], radius=64, fill=ink)   # body
    return im


def taskman(shade):
    top, bot, ink = SHADES[shade]
    im = tile(top, bot)
    d = ImageDraw.Draw(im)
    base = 740
    for i, h in enumerate([220, 430, 310]):
        x = 262 + i * 190
        d.rounded_rectangle([x, base - h, x + 120, base], radius=60, fill=ink)
    return im


def sheet():
    """Preview: both shades, each at 256 / 48 / 32 / 16 px, on light and dark backgrounds."""
    cell, pad = 300, 20
    out = Image.new("RGBA", (cell * 4, cell * 2), (0, 0, 0, 255))
    for row, bg in enumerate([(243, 243, 243, 255), (32, 32, 32, 255)]):
        for col, (shade, fn) in enumerate([(s, f) for s in SHADES for f in (explorer, taskman)]):
            x0, y0 = col * cell, row * cell
            ImageDraw.Draw(out).rectangle([x0, y0, x0 + cell, y0 + cell], fill=bg)
            big = fn(shade).resize((180, 180), Image.LANCZOS)
            out.paste(big, (x0 + pad, y0 + pad), big)
            x = x0 + pad
            for sz in (48, 32, 16):
                sm = fn(shade).resize((sz, sz), Image.LANCZOS)
                out.paste(sm, (x, y0 + 220), sm)
                x += sz + 16
    return out


shade = sys.argv[1] if len(sys.argv) > 1 else "light"
for name, fn in [("wfiles", explorer), ("wintask", taskman)]:
    small = fn(shade).resize((256, 256), Image.LANCZOS)
    out = os.path.join(SRC, "installer", f"{name}.ico")
    small.save(out, sizes=SIZES)
    pkg = "fm" if name == "wfiles" else "app"
    shutil.copy(out, os.path.join(SRC, pkg, "assets", f"{name}.ico"))
    small.save(os.path.join(SRC, "installer", f"{name}-preview.png"))
    print(out, os.path.getsize(out))
prev = os.path.join(SRC, "design", "build", "icons-v3.png")
sheet().save(prev)
print(prev)
