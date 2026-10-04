"""Refined: the folder icon is a solid 16x16 blue block, so its columns carry many
blue pixels; ClearType fringes on the glyphs only give a few. Split on that."""
from PIL import Image

CUR = r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png"


def is_icon_blue(r, g, b):
    return b > 120 and b - r > 30 and g > r


def measure(path, y0, y1, note):
    im = Image.open(path).convert("RGB")
    w, _ = im.size
    px = im.load()
    blue_cnt = [0] * w
    dark_cols = []
    for x in range(w):
        for y in range(y0, y1 + 1):
            r, g, b = px[x, y]
            if is_icon_blue(r, g, b):
                blue_cnt[x] += 1
    ic = [x for x in range(w) if blue_cnt[x] >= 6]
    icon = (ic[0], ic[-1]) if ic else None
    # text columns: strongly dark, to the right of the icon
    x_start = (icon[1] + 3) if icon else 0
    for x in range(x_start, w):
        hits = 0
        for y in range(y0, y1 + 1):
            r, g, b = px[x, y]
            if max(r, g, b) < 120:
                hits += 1
        if hits:
            dark_cols.append(x)
    text = dark_cols[0] if dark_cols else None
    # arrow: strongly dark columns left of the icon
    arrow = None
    for x in range(0, (icon[0] - 2) if icon else 0):
        hits = 0
        for y in range(y0, y1 + 1):
            r, g, b = px[x, y]
            if max(r, g, b) < 170:
                hits += 1
        if hits:
            arrow = x
            break
    print(f"{note:22} icon={icon} text_x={text} arrow_x={arrow}")


print("--- CUR level 1 rows")
measure(CUR, 50, 73, "全部")
measure(CUR, 188, 210, "Dump状态(arrow)")
measure(CUR, 732, 753, "显示方向(arrow)")
measure(CUR, 766, 787, "分辨率(arrow,v)")
print("--- CUR children of 分辨率")
measure(CUR, 800, 821, "0 x 0 (H)")
measure(CUR, 902, 923, "64 x 8 (H)")
measure(CUR, 1038, 1059, "120 x 9 (H)")
