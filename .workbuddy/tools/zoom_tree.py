from PIL import Image

SPECS = [
    (r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-036Z-62d9c688.png", "ref_top", (0, 0, 220, 120)),
    (r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png", "cur_top", (0, 0, 220, 120)),
    (r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png", "cur_parent_child", (0, 1120, 300, 1330)),
]
OUT = r"C:/Users/11921/Desktop/mamepgui-rewrite/.workbuddy/tools/"
for path, name, box in SPECS:
    im = Image.open(path).convert("RGB").crop(box)
    big = im.resize((im.width * 4, im.height * 4), Image.NEAREST)
    big.save(OUT + name + ".png")
    print(name, "->", big.size)
