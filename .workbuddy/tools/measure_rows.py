"""Row geometry of the folder dock: icon / label / branch positions, plus any
thin vertical guide lines."""
from PIL import Image
import sys

path = sys.argv[1]
x0, x1 = (int(v) for v in sys.argv[2].split(","))
y0, y1 = (int(v) for v in sys.argv[3].split(","))
scale = float(sys.argv[4]) if len(sys.argv) > 4 else 1.0

im = Image.open(path).convert("RGB")
px = im.load()


def is_blue(r, g, b):
    return b > 120 and b - r > 30 and g > r


rows = []
for y in range(y0, y1):
    nb = nd = 0
    for x in range(x0, x1):
        r, g, b = px[x, y]
        if is_blue(r, g, b):
            nb += 1
        elif max(r, g, b) < 140:
            nd += 1
    rows.append((y, nb, nd))

bands, cur = [], None
for y, nb, nd in rows:
    if nb + nd > 6:
        cur = [y, y] if cur is None else [cur[0], y]
    else:
        if cur and cur[1] - cur[0] > 8:
            bands.append(tuple(cur))
        cur = None
if cur and cur[1] - cur[0] > 8:
    bands.append(tuple(cur))

print(f"{len(bands)} bands in y={y0}..{y1}")
for (a, b) in bands:
    icon = [x for x in range(x0, x1)
            if sum(1 for y in range(a, b + 1) if is_blue(*px[x, y])) >= 8]
    if not icon:
        print(f"  y={a:4d}-{b:<4d} (no icon)")
        continue
    i0, i1 = icon[0], icon[-1]
    dark = [x for x in range(i1 + 2, x1)
            if any(max(px[x, y]) < 120 for y in range(a, b + 1))]
    left = [x for x in range(x0, i0 - 2)
            if any(max(px[x, y]) < 170 for y in range(a, b + 1))]
    print(f"  y={a:4d}-{b:<4d} icon={i0:4d}..{i1:<4d} label_x={dark[0] if dark else None} "
          f"branch_x={left[0] if left else None}")

# thin vertical guides: columns that are non-background over most rows
h = y1 - y0
guide = []
for x in range(x0, x1):
    n = sum(1 for y in range(y0, y1) if max(px[x, y]) < 220 and not is_blue(*px[x, y]))
    if n > h * 0.6:
        guide.append((x, n))
print("guide columns:", guide[:12])
