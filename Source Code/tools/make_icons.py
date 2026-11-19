"""Draws the two app icons (Task Manager: three bars on the accent square; Files: a folder) as multi-size .ico files.
Usage: python tools/make_icons.py  -> installer/wintask.ico, installer/wfiles.ico"""
import os

from PIL import Image, ImageDraw

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "installer")
S = 256
SIZES = [(16, 16), (20, 20), (24, 24), (32, 32), (40, 40), (48, 48), (64, 64), (128, 128), (256, 256)]


def taskman():
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle([8, 8, S - 8, S - 8], radius=56, fill=(0, 95, 184, 255))
    bar_w, base = 38, S - 62
    for i, h in enumerate([70, 140, 100]):
        x = 52 + i * 58
        d.rounded_rectangle([x, base - h, x + bar_w, base], radius=10, fill=(247, 250, 255, 255))
    return im


def files():
    im = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    # back of the folder with its tab
    d.rounded_rectangle([16, 44, 120, 100], radius=18, fill=(230, 160, 30, 255))
    d.rounded_rectangle([16, 68, S - 16, S - 36], radius=22, fill=(230, 160, 30, 255))
    # front panel
    d.rounded_rectangle([16, 92, S - 16, S - 36], radius=22, fill=(252, 196, 64, 255))
    # accent strip = "fast" mark
    d.rounded_rectangle([52, 140, S - 52, 156], radius=8, fill=(0, 95, 184, 255))
    return im


for name, im in [("wintask", taskman()), ("wfiles", files())]:
    path = os.path.join(OUT, f"{name}.ico")
    im.save(path, sizes=SIZES)
    print(path, os.path.getsize(path))
