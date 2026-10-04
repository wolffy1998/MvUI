"""Precise row measurement: for every text band, locate the folder icon (by its
characteristic blue) and the start of the label text."""
from PIL import Image

CUR = r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png"
REF = r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-036Z-62d9c688.png"


def is_icon_blue(r, g, b):
    return b > 120 and b - r > 30 and g > r


def is_dark(r, g, b):
    return r < 140 and g < 140 and b < 140


def measure(path, y_from=0, y_to=None):
    im = Image.open(path).convert("RGB")
    w, h = im.size
    y_to = y_to or h
    px = im.load()
    # row has content?
    rows = []
    for y in range(y_from, y_to):
        nb = nd = 0
        for x in range(w):
            r, g, b = px[x, y]
            if is_icon_blue(r, g, b):
                nb += 1
            elif is_dark(r, g, b):
                nd += 1
        rows.append((y, nb, nd))
    bands = []
    cur = None
    for y, nb, nd in rows:
        if nb + nd > 1:
            if cur is None:
                cur = [y, y]
            else:
                cur[1] = y
        else:
            if cur and cur[1] - cur[0] > 8:
                bands.append(tuple(cur))
            cur = None
    if cur and cur[1] - cur[0] > 8:
        bands.append(tuple(cur))

    out = []
    for (y0, y1) in bands:
        icon_cols = []
        dark_cols = []
        for x in range(w):
            ib = db = 0
            for y in range(y0, y1 + 1):
                r, g, b = px[x, y]
                if is_icon_blue(r, g, b):
                    ib += 1
                elif is_dark(r, g, b):
                    db += 1
            if ib:
                icon_cols.append(x)
            if db:
                dark_cols.append(x)
        icon_x = icon_cols[0] if icon_cols else None
        icon_r = icon_cols[-1] if icon_cols else None
        # text = first dark column right of the icon
        text_x = None
        if icon_r is not None:
            cands = [x for x in dark_cols if x > icon_r + 2]
            text_x = cands[0] if cands else None
        arrow_x = None
        if icon_x is not None:
            left = [x for x in dark_cols if x < icon_x - 2]
            arrow_x = left[0] if left else None
        out.append((y0, y1, icon_x, icon_r, text_x, arrow_x))
    return out


for tag, path in (("CUR", CUR), ("REF", REF)):
    print(f"===== {tag} {path.split(chr(92))[-1]}")
    for (y0, y1, ix, ir, tx, ax) in measure(path):
        print(f"  y={y0:4d}-{y1:<4d} icon={str(ix):>5}..{str(ir):<5} text={str(tx):>5} arrow={ax}")
