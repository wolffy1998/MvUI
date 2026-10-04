"""Measure the folder-tree row layout of a screenshot: for each text band, report
the leftmost dark pixel (arrow / icon outline) and the horizontal extent of the
blue folder icon.  Pure PIL, no numpy."""
from PIL import Image
import sys

PATHS = [
    ("IMG1(ref)", r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-036Z-62d9c688.png"),
    ("IMG2(cur)", r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png"),
]


def scan(tag, path):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    px = im.load()
    print(f"--- {tag}  size={w}x{h}")
    dark_cols = [0] * w
    blue_cols = [0] * w
    rowdark = [0] * h
    rowblue = [0] * h
    for y in range(h):
        for x in range(w):
            r, g, b = px[x, y]
            if r < 150 and g < 150 and b < 150:
                dark_cols[x] += 1
                rowdark[y] += 1
            if b > r + 25 and b > 120 and g > r:
                blue_cols[x] += 1
                rowblue[y] += 1
    bands = []
    inb = False
    for y in range(h):
        has = rowdark[y] > 2 or rowblue[y] > 2
        if has and not inb:
            start = y
            inb = True
        elif not has and inb:
            bands.append((start, y))
            inb = False
    if inb:
        bands.append((start, h))
    print(f"  {len(bands)} bands")
    sel = bands[:5] + bands[-26:]
    for (y0, y1) in sel:
        dc = [x for x in range(w) if any(px[x, yy][0] < 150 and px[x, yy][1] < 150 and px[x, yy][2] < 150 for yy in range(y0, y1))]
        bc = [x for x in range(w) if any(px[x, yy][2] > px[x, yy][0] + 25 and px[x, yy][2] > 120 and px[x, yy][1] > px[x, yy][0] for yy in range(y0, y1))]
        fd = dc[0] if dc else -1
        b0 = bc[0] if bc else -1
        b1 = bc[-1] if bc else -1
        print(f"   y={y0:4d}-{y1:<4d} firstDark={fd:4d}  blueIcon={b0:4d}..{b1:<4d}")
    print()


for tag, p in PATHS:
    scan(tag, p)
