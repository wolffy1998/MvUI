# -*- coding: utf-8 -*-
# Generate icon designs for CandyCab and RetroCab.
# Output: 512 png, multi-size .ico, glyph png, one contact-sheet preview.
from PIL import Image, ImageDraw, ImageFont, ImageFilter

OUT = r"C:\Users\11921\Desktop\mamepgui-rewrite\icon-designs"
S = 512  # canvas

def font(sz, bold=True):
    name = "arialbd.ttf" if bold else "arial.ttf"
    try:
        return ImageFont.truetype(r"C:\Windows\Fonts" + "\\" + name, sz)
    except Exception:
        return ImageFont.load_default()

def rounded_bg(d, top, bottom, radius=96):
    # vertical gradient clipped to a rounded square
    grad = Image.new("RGB", (S, S), top)
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        t = y / (S - 1)
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3))
        gd.line([(0, y), (S, y)], fill=c)
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([16, 16, S - 16, S - 16], radius=radius, fill=255)
    base = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    base.paste(grad, (0, 0), mask)
    d._image.alpha_composite(base)

# ---------------------------------------------------------------- CandyCab
def candy_cab(glyph=False):
    """Cute candy-color cabinet on cream background."""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    if not glyph:
        rounded_bg(d, (255, 246, 233), (255, 214, 228))  # cream -> soft pink
        # ground shadow
        sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        ImageDraw.Draw(sh).ellipse([130, 408, 382, 452], fill=(90, 40, 60, 70))
        img.alpha_composite(sh.filter(ImageFilter.GaussianBlur(10)))

    W = (58, 46, 57, 255)        # outline dark cocoa
    PINK = (255, 92, 138, 255)   # marquee
    TEAL = (78, 205, 196, 255)   # control panel
    CREAM = (255, 252, 245, 255) # cabinet body
    SCREEN = (23, 63, 95, 255)   # deep blue screen
    MINT = (123, 224, 173, 255)  # sprite
    YEL = (255, 209, 102, 255)
    RED = (255, 82, 82, 255)

    # cabinet body
    d.rounded_rectangle([166, 108, 346, 402], 26, fill=CREAM, outline=W, width=7)
    # marquee
    d.rounded_rectangle([158, 96, 354, 168], 20, fill=PINK, outline=W, width=7)
    t = "CC"
    f = font(46)
    tb = d.textbbox((0, 0), t, font=f)
    d.text((256 - (tb[2] - tb[0]) / 2 - tb[0], 132 - (tb[3] - tb[1]) / 2 - tb[1]),
           t, font=f, fill=(255, 255, 255, 255))
    # screen
    d.rounded_rectangle([186, 186, 326, 296], 14, fill=SCREEN, outline=W, width=6)
    # 8-bit invader on screen (pixel map)
    inv = ["00100100", "00100100", "01111110", "11011011",
           "11111111", "10111101", "10100101", "00100100"]
    px, ox, oy = 11, 212, 202
    for r, row in enumerate(inv):
        for c, ch in enumerate(row):
            if ch == "1":
                d.rectangle([ox + c * px, oy + r * px, ox + c * px + px - 1, oy + r * px + px - 1],
                            fill=MINT)
    # control panel
    d.rounded_rectangle([154, 302, 358, 348], 16, fill=TEAL, outline=W, width=7)
    # joystick (stick + red ball)
    d.rounded_rectangle([224, 296, 236, 322], 5, fill=(120, 120, 130, 255))
    d.ellipse([216, 278, 244, 306], fill=RED, outline=W, width=5)
    # buttons
    for i, col in enumerate([YEL, PINK, (91, 158, 250, 255)]):
        cx = 278 + i * 30
        d.ellipse([cx, 312, cx + 22, 334], fill=col, outline=W, width=4)
    # coin slot
    d.rounded_rectangle([238, 362, 274, 386], 6, fill=(240, 231, 219, 255), outline=W, width=5)
    d.rectangle([251, 368, 261, 380], fill=W)
    return img

# ---------------------------------------------------------------- RetroCab
def retro_cab(glyph=False):
    """Retro synthwave cabinet: dark bg, neon cyan outline, magenta accents."""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    if not glyph:
        rounded_bg(d, (15, 15, 45), (58, 29, 110))  # navy -> purple
        # synthwave floor grid, clipped to the rounded background
        gl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
        grid = ImageDraw.Draw(gl)
        for i in range(7):
            y = 420 + i * 14
            a = max(20, 110 - i * 16)
            grid.line([(40, y), (S - 40, y)], fill=(244, 114, 182, a), width=3)
        for i in range(-4, 5):
            x = 256 + i * 60
            grid.line([(256 + i * 26, 408), (x + i * 34, S - 30)],
                      fill=(244, 114, 182, 60), width=3)
        mask = Image.new("L", (S, S), 0)
        ImageDraw.Draw(mask).rounded_rectangle([16, 16, S - 16, S - 16], 96, fill=255)
        img.paste(Image.new("RGBA", (S, S), (0, 0, 0, 0)), (0, 0),
                  Image.composite(gl.split()[3], Image.new("L", (S, S), 0), mask))

    CYAN = (34, 211, 238, 255)
    MAG = (244, 114, 182, 255)
    PUR = (168, 85, 247, 255)
    DARK = (17, 24, 39, 255)

    body = [(172, 118), (340, 118), (340, 300), (366, 312), (366, 396),
            (146, 396), (146, 312), (172, 300)]
    # neon glow: draw thick blurred copy
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    gd = ImageDraw.Draw(glow)
    gd.polygon(body, fill=(34, 211, 238, 90))
    gd.rounded_rectangle([160, 92, 352, 160], 12, fill=(168, 85, 247, 90))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(14)))
    # cabinet body
    d.polygon(body, fill=DARK, outline=CYAN, width=5)
    # marquee
    d.rounded_rectangle([160, 92, 352, 160], 12, fill=PUR, outline=CYAN, width=5)
    t = "RC"
    f = font(44)
    tb = d.textbbox((0, 0), t, font=f)
    d.text((256 - (tb[2] - tb[0]) / 2 - tb[0], 126 - (tb[3] - tb[1]) / 2 - tb[1]),
           t, font=f, fill=(34, 211, 238, 255))
    # screen with scanlines + magenta invader
    d.rectangle([186, 176, 326, 286], fill=(11, 17, 32, 255), outline=CYAN, width=4)
    for y in range(184, 282, 8):
        d.line([(190, y), (322, y)], fill=(30, 41, 59, 255), width=2)
    inv = ["00100100", "00100100", "01111110", "11011011",
           "11111111", "10111101", "10100101", "00100100"]
    px, ox, oy = 10, 216, 202
    for r, row in enumerate(inv):
        for c, ch in enumerate(row):
            if ch == "1":
                d.rectangle([ox + c * px, oy + r * px, ox + c * px + px - 1, oy + r * px + px - 1],
                            fill=MAG)
    # panel + controls
    d.rectangle([154, 300, 358, 344], fill=(30, 41, 59, 255), outline=CYAN, width=4)
    d.rounded_rectangle([226, 292, 238, 318], 4, fill=(148, 163, 184, 255))
    d.ellipse([218, 274, 246, 302], fill=MAG, outline=(255, 255, 255, 255), width=3)
    for i, col in enumerate([CYAN, (255, 209, 102, 255), (134, 239, 172, 255)]):
        cx = 276 + i * 30
        d.ellipse([cx, 310, cx + 22, 332], fill=col)
    # coin slot
    d.rectangle([240, 358, 272, 382], outline=CYAN, width=4)
    d.rectangle([250, 366, 262, 376], fill=CYAN)
    return img

def glyphize(img, colorize):
    """mono-color silhouette glyph for in-app / small use"""
    a = img.split()[3]
    g = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    solid = Image.new("RGBA", (S, S), colorize)
    g.paste(solid, (0, 0), a)
    return g

def save_set(name, img, glyph):
    p = OUT + "\\" + name
    img.save(p + "_512.png")
    img.resize((256, 256), Image.LANCZOS).save(p + "_256.png")
    img.save(p + ".ico", sizes=[(256, 256), (128, 128), (64, 64), (48, 48), (32, 32), (16, 16)])
    glyph.save(p + "_glyph_512.png")
    glyph.resize((64, 64), Image.LANCZOS).save(p + "_glyph_64.png")

cc = candy_cab()
rc = retro_cab()
cc_g = glyphize(candy_cab(glyph=True), (255, 92, 138, 255))
rc_g = glyphize(retro_cab(glyph=True), (34, 211, 238, 255))
save_set("CandyCab", cc, cc_g)
save_set("RetroCab", rc, rc_g)

# ---- contact sheet preview ------------------------------------------------
sheet = Image.new("RGB", (1060, 700), (250, 250, 250))
sd = ImageDraw.Draw(sheet)
f24 = font(30); f14 = font(18)
def label(x, txt):
    tb = sd.textbbox((0, 0), txt, font=f24)
    sd.text((x + 160 - (tb[2] - tb[0]) / 2, 28), txt, font=f24, fill=(40, 40, 40))
def row(x, y, img, name):
    label(x, name)
    for i, sz in enumerate([256, 64, 32, 16]):
        im = img.resize((sz, sz), Image.LANCZOS)
        sheet.paste(im, (x + (160 - sz) // 2 + i * 5, y), im if sz < 256 else None)
        if sz < 256:
            pass
    # paste 256 with alpha
    sheet.paste(img.resize((256, 256), Image.LANCZOS), (x + 2, y), img.resize((256, 256), Image.LANCZOS))
# simpler: draw manually
def preview_cell(x, img, name):
    tb = sd.textbbox((0, 0), name, font=f24)
    sd.text((x + 128 - (tb[2] - tb[0]) / 2, 30), name, font=f24, fill=(40, 40, 40))
    big = img.resize((256, 256), Image.LANCZOS)
    sheet.paste(big, (x, 80), big)
    xx = x
    for sz in (64, 48, 32, 16):
        sm = img.resize((sz, sz), Image.LANCZOS)
        sheet.paste(sm, (xx, 360), sm)
        xx += sz + 18
    sd.text((x, 400), "64 / 48 / 32 / 16 px", font=f14, fill=(110, 110, 110))
    g = img.resize((48, 48), Image.LANCZOS)
preview_cell(30, cc, "CandyCab")
preview_cell(560, rc, "RetroCab")
sd.text((30, 470), "design: CandyCab = candy-color cabinet, cream/pink/teal", font=f14, fill=(80, 80, 80))
sd.text((30, 498), "design: RetroCab = synthwave neon cabinet, navy/purple + cyan/magenta", font=f14, fill=(80, 80, 80))
sd.text((30, 540), "glyph (mono) versions:", font=f14, fill=(80, 80, 80))
sheet.paste(cc_g.resize((96, 96), Image.LANCZOS), (260, 528), cc_g.resize((96, 96), Image.LANCZOS))
sheet.paste(rc_g.resize((96, 96), Image.LANCZOS), (790, 528), rc_g.resize((96, 96), Image.LANCZOS))
sheet.save(OUT + r"\preview.png")
print("done")
