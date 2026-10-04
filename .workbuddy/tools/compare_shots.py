"""Decide whether the two clipboard screenshots come from the same rendering:
align them by the 8 px vertical offset and diff the overlapping area."""
from PIL import Image, ImageChops

A = Image.open(r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-036Z-62d9c688.png").convert("RGB")
B = Image.open(r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png").convert("RGB")
print("A", A.size, "B", B.size)

w = min(A.width, B.width)
for dy in (0, 8, -8):
    ax0, ay0 = 0, max(0, dy)
    bx0, by0 = 0, max(0, -dy)
    h = min(A.height - ay0, B.height - by0)
    a = A.crop((ax0, ay0, ax0 + w, ay0 + h))
    b = B.crop((bx0, by0, bx0 + w, by0 + h))
    diff = ImageChops.difference(a, b)
    bbox = diff.getbbox()
    if bbox is None:
        print(f"dy={dy:3d}: IDENTICAL")
        continue
    stat = diff.convert("L").getextrema()
    # count pixels differing by more than 8
    px = diff.convert("L").load()
    n = 0
    for y in range(0, h, 2):
        for x in range(0, w, 2):
            if px[x, y] > 8:
                n += 1
    print(f"dy={dy:3d}: bbox={bbox} extrema={stat} sampled_diff_px={n}")
