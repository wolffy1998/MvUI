# -*- coding: utf-8 -*-
# MvUI icon — homage to mamep.ico (italic gradient M + superscript mark).
# Variant A (active): transparent bg, big blue gradient italic M + peach heart.
# Variant B: modernized, rounded-square gradient bg + white M + cyan v.
from PIL import Image, ImageDraw, ImageFont, ImageFilter
import math

OUT = r"C:\Users\11921\Desktop\mamepgui-rewrite\icon-designs"
S = 512

def font(sz):
    try:
        return ImageFont.truetype(r"C:\Windows\Fonts\arialbi.ttf", sz)  # Arial Bold Italic
    except Exception:
        return ImageFont.truetype(r"C:\Windows\Fonts\arialbd.ttf", sz)

def grad_text(img, text, fnt, top, bottom, pos, stroke=0, stroke_fill=None):
    """draw text filled with a vertical gradient (+optional dark stroke)"""
    if stroke:
        d = ImageDraw.Draw(img)
        d.text(pos, text, font=fnt, fill=stroke_fill, stroke_width=stroke,
               stroke_fill=stroke_fill)
    # mask of the glyph interior ONLY (no stroke) so the navy outline survives
    tmp = Image.new("L", img.size, 0)
    td = ImageDraw.Draw(tmp)
    td.text(pos, text, font=fnt, fill=255)
    bb = tmp.getbbox()
    gh = bb[3] - bb[1] if bb else 1
    grad = Image.new("RGBA", img.size, (0, 0, 0, 0))
    gd = ImageDraw.Draw(grad)
    for y in range(bb[1], bb[3]):
        t = (y - bb[1]) / max(1, gh - 1)
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (img.size[0], y)], fill=c)
    img.paste(grad, (0, 0), tmp)

def glyph_layer(text, fnt, stroke, stroke_fill, top, bottom, box):
    """render text (outline + gradient interior + top bevel) fitted into box"""
    W = 1400
    sm = Image.new("L", (W, W), 0)   # glyph incl. outline
    ImageDraw.Draw(sm).text((250, 250), text, font=fnt, fill=255,
                            stroke_width=stroke, stroke_fill=255)
    im = Image.new("L", (W, W), 0)   # interior only
    ImageDraw.Draw(im).text((250, 250), text, font=fnt, fill=255)
    bb = sm.getbbox()
    stroke_m = sm.crop(bb)
    inner_m = im.crop(bb)            # same crop box -> stays aligned
    lw, lh = stroke_m.size
    layer = Image.new("RGBA", (lw, lh), (0, 0, 0, 0))
    layer.paste(stroke_fill, (0, 0), stroke_m)
    grad = Image.new("RGBA", (lw, lh), (0, 0, 0, 0))
    gd = ImageDraw.Draw(grad)
    for y in range(lh):
        t = y / max(1, lh - 1)
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (lw, y)], fill=c)
    layer.paste(grad, (0, 0), inner_m)
    # top bevel highlight (upper 35% of the glyph) — blur the MASK only,
    # never the composited RGBA (transparent-black edges darken the colour)
    hl = Image.new("RGBA", (lw, lh), (0, 0, 0, 0))
    hlm = inner_m.crop((0, 0, lw, max(1, int(lh * 0.35))))
    hlm = hlm.filter(ImageFilter.GaussianBlur(6))
    hl.paste((212, 236, 252, 115), (0, 0), hlm)
    layer = Image.alpha_composite(layer, hl)
    bl, bt, br, bm = box
    bw, bh = br - bl, bm - bt
    sc = min(bw / lw, bh / lh)
    nw, nh = int(lw * sc), int(lh * sc)
    layer = layer.resize((nw, nh), Image.LANCZOS)
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    out.paste(layer, (bl + (bw - nw) // 2, bt + (bh - nh) // 2), layer)
    return out

def heart_layer(w, top, bottom, outline_col, ow, highlight_col):
    """peach heart, supersampled 4x, navy outline, gradient + lobe highlight.
    Returns a 512-canvas layer with the heart centred at (cx, cy)."""
    ss = 4  # supersample factor
    pad = int(w * 0.9)
    CW = (w + 2 * pad) * ss
    mask = Image.new("L", (CW, CW), 0)
    pts = []
    for i in range(180):
        t = 2 * math.pi * i / 180
        x = 16 * math.sin(t) ** 3
        y = (13 * math.cos(t) - 5 * math.cos(2 * t)
             - 2 * math.cos(3 * t) - math.cos(4 * t))
        pts.append((CW / 2 + x * w * ss / 32.0, CW / 2 - y * w * ss / 30.0))
    ImageDraw.Draw(mask).polygon(pts, fill=255)
    bb = mask.getbbox()
    # recentre: move bbox centre to canvas centre, target width w*ss
    bw = bb[2] - bb[0]
    bh = bb[3] - bb[1]
    k = (w * ss) / bw
    cx, cy = (bb[0] + bb[2]) / 2, (bb[1] + bb[3]) / 2
    pts2 = [((px - cx) * k + CW / 2, (py - cy) * k + CW / 2) for px, py in pts]
    mask = Image.new("L", (CW, CW), 0)
    ImageDraw.Draw(mask).polygon(pts2, fill=255)
    lw, lh = CW, CW
    layer = Image.new("RGBA", (lw, lh), (0, 0, 0, 0))
    if ow > 0:
        # MaxFilter(1) crashes PIL 12.3.0/py3.14 natively, so only dilate
        # (and paste the outline colour) when an outline is actually wanted
        outer = mask.filter(ImageFilter.MaxFilter(2 * ow * ss + 1))
        layer.paste(outline_col, (0, 0), outer)
    b2 = mask.getbbox()
    grad = Image.new("RGBA", (lw, lh), (0, 0, 0, 0))
    gd = ImageDraw.Draw(grad)
    gh = b2[3] - b2[1]
    for y in range(b2[1], b2[3]):
        t = (y - b2[1]) / max(1, gh - 1)
        c = tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (lw, y)], fill=c)
    layer.paste(grad, (0, 0), mask)
    # optional specular highlight on the upper-left lobe — blur the MASK only
    if highlight_col:
        hm = Image.new("L", (lw, lh), 0)
        hd = ImageDraw.Draw(hm)
        hx, hy = CW / 2 - w * ss * 0.17, CW / 2 - w * ss * 0.18
        hd.ellipse([hx - w * ss * 0.15, hy - w * ss * 0.11,
                    hx + w * ss * 0.15, hy + w * ss * 0.15], fill=255)
        hm = hm.filter(ImageFilter.GaussianBlur(7 * ss))
        hi = Image.new("RGBA", (lw, lh), (0, 0, 0, 0))
        hi.paste(highlight_col[:3] + (255,), (0, 0), hm)
        layer = Image.alpha_composite(layer, hi)
    layer = layer.resize((lw // ss, lh // ss), Image.LANCZOS)
    bb3 = layer.getbbox()
    return layer.crop(bb3)

NAVY = (16, 42, 82, 255)

def variant_a():
    """active: transparent bg, big blue gradient M + peach-blossom heart"""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    m = glyph_layer("M", font(460), 18, NAVY,
                    (74, 170, 238), (10, 60, 145),
                    (96, 66, 488, 452))
    heart = heart_layer(178, (255, 205, 216), (255, 148, 170), NAVY, 0,
                        None)
    hx, hy = 46, 28   # heart top-left position (heart w≈150,h≈136+outline)
    # combined drop shadow
    sil = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    sil.alpha_composite(m)
    tmp = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    tmp.paste(heart, (hx, hy), heart)
    sil.alpha_composite(tmp)
    sh = sil.split()[3].point(lambda p: 95 if p > 10 else 0)
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    shadow.paste((18, 38, 76, 255), (0, 0), sh)
    img.alpha_composite(ImageChops_offset(shadow.filter(ImageFilter.GaussianBlur(9)), 9, 11))
    img.alpha_composite(m)
    img.alpha_composite(tmp)
    return img

def ImageChops_offset(im, dx, dy):
    from PIL import ImageChops
    return ImageChops.offset(im, dx, dy)

def variant_b():
    """modernized: rounded-square blue gradient bg, white M, cyan v"""
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    bg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    gd = ImageDraw.Draw(bg)
    for y in range(S):
        t = y / (S - 1)
        gd.line([(0, y), (S, y)],
                fill=(int(30 + 25 * t), int(58 + 40 * t), int(138 + 60 * t), 255))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([16, 16, S - 16, S - 16], 110, fill=255)
    img.paste(bg, (0, 0), mask)
    hl = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(hl).ellipse([-80, -120, 380, 240], fill=(255, 255, 255, 40))
    img.alpha_composite(hl.filter(ImageFilter.GaussianBlur(40)))
    f = font(300)
    sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(sh).text((122, 122), "M", font=f, fill=(0, 20, 60, 130))
    img.alpha_composite(sh.filter(ImageFilter.GaussianBlur(12)))
    d = ImageDraw.Draw(img)
    d.text((138, 112), "M", font=f, fill=(255, 255, 255, 255))
    fv = font(140)
    vg = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(vg).text((82, 86), "v", font=fv, fill=(103, 232, 249, 255))
    img.alpha_composite(vg.filter(ImageFilter.GaussianBlur(10)).point(lambda p: int(p * 0.8)))
    d.text((82, 86), "v", font=fv, fill=(103, 232, 249, 255))
    return img

def fitted(img, fill=0.95):
    """crop to content and scale so the artwork fills `fill` of the canvas —
    small taskbar sizes read much bigger than the padded 512 art"""
    bb = img.getbbox()
    c = img.crop(bb)
    side = int(S * fill)
    sc = side / max(c.size)
    nw, nh = max(1, int(c.width * sc)), max(1, int(c.height * sc))
    out = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    r = c.resize((nw, nh), Image.LANCZOS)
    out.paste(r, ((S - nw) // 2, (S - nh) // 2), r)
    return out

def save(name, img):
    p = OUT + "\\" + name
    img.save(p + "_512.png")
    fitted(img).resize((256, 256), Image.LANCZOS).save(p + "_256.png")
    fitted(img).save(p + ".ico", sizes=[(256, 256), (128, 128), (64, 64), (48, 48),
                                        (32, 32), (16, 16)])

a = variant_a()
b = variant_b()
save("MvUI_classic", a)
save("MvUI_modern", b)

# contact sheet vs reference
sheet = Image.new("RGB", (1060, 420), (245, 245, 248))
sd = ImageDraw.Draw(sheet)
f24 = ImageFont.truetype(r"C:\Windows\Fonts\arialbd.ttf", 26)
ref = Image.open(OUT + r"\ref_mamep.png").convert("RGBA").resize((160, 160), Image.NEAREST)
cells = [(30, ref, "mamep.ico (ref)"), (330, a.resize((160, 160), Image.LANCZOS), "MvUI classic"),
         (630, b.resize((160, 160), Image.LANCZOS), "MvUI modern")]
for x, im, t in cells:
    sheet.paste(im, (x, 60), im)
    tb = sd.textbbox((0, 0), t, font=f24)
    sd.text((x + 80 - (tb[2] - tb[0]) / 2, 26), t, font=f24, fill=(40, 40, 40))
for i, sz in enumerate([64, 48, 32, 16]):
    sheet.paste(a.resize((sz, sz), Image.LANCZOS), (330 + i * 90, 260), a.resize((sz, sz), Image.LANCZOS))
    sheet.paste(b.resize((sz, sz), Image.LANCZOS), (630 + i * 90, 260), b.resize((sz, sz), Image.LANCZOS))
sd.text((30, 300), "64 / 48 / 32 / 16 px:", font=ImageFont.truetype(r"C:\Windows\Fonts\arial.ttf", 20), fill=(90, 90, 90))
sheet.save(OUT + r"\preview_mvui.png")
print("done")
