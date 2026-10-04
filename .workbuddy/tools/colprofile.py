from PIL import Image
im = Image.open(r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png").convert("RGB")
px = im.load()
for (y0, y1, note) in ((50, 73, "全部"), (800, 821, "child 0x0")):
    print("==", note)
    for x in range(30, 130, 2):
        col = [px[x, y] for y in range(y0, y1 + 1)]
        blue = sum(1 for r, g, b in col if b > 120 and b - r > 30 and g > r)
        dk = sum(1 for r, g, b in col if max(r, g, b) < 120)
        if blue or dk:
            sample = col[len(col) // 2]
            print(f"   x={x:4d} blue={blue:3d} dark={dk:3d} mid={sample}")
