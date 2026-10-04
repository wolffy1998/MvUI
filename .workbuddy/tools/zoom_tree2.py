from PIL import Image

REF = r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-036Z-62d9c688.png"
CUR = r"C:\Users\11921\.workbuddy\clipboard-images\clipboard-2026-10-03T10-51-25-039Z-85ea253d.png"
OUT = r"C:/Users/11921/Desktop/mamepgui-rewrite/.workbuddy/tools/"

SPECS = [
    (CUR, "cur_arrow", (0, 95, 240, 230), 4),      # 未拥有 / 家用机 / Dump状态 / 存档
    (REF, "ref_arrow", (0, 95, 240, 230), 4),
    (CUR, "cur_res", (0, 1150, 300, 1290), 4),     # 分辨率 + first children
    (REF, "ref_star", (0, 0, 240, 60), 5),
]
for path, name, box, scale in SPECS:
    im = Image.open(path).convert("RGB").crop(box)
    im.resize((im.width * scale, im.height * scale), Image.NEAREST).save(OUT + name + ".png")
    print(name, im.size)
