"""Capture the MvUI window's own client area, immune to what covers it.

Screen grabs keep lying: the window drifts onto the secondary monitor, other
apps end up on top, and `ImageGrab(all_screens=True).crop(rect)` then returns
whatever is really there — which is how a background image "covered the whole
UI" in the first place when it never did.

`PrintWindow` with `PW_RENDERFULLCONTENT` asks the window to render itself into
a DC, so the result is the real content regardless of z-order or monitor. That
is the only trustworthy way to check what the app is drawing here.

usage: win_shot.py OUT.png
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys

from PIL import Image

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u = ctypes.windll.user32
g = ctypes.windll.gdi32
OUT = sys.argv[1]

tl = subprocess.run(
    ["tasklist", "/FI", "IMAGENAME eq mvui.exe", "/FO", "CSV", "/NH"],
    capture_output=True, text=True, errors="replace",
).stdout
pids = {int(l.split('","')[1]) for l in tl.splitlines() if l.strip().startswith('"')}
if not pids:
    raise SystemExit("mvui.exe not running")

target = []


def cb(h, _):
    pid = ctypes.c_ulong()
    u.GetWindowThreadProcessId(h, ctypes.byref(pid))
    if pid.value in pids and u.IsWindowVisible(h):
        n = u.GetWindowTextLengthW(h)
        b = ctypes.create_unicode_buffer(n + 1)
        u.GetWindowTextW(h, b, n + 1)
        if b.value == "MvUI":
            target.append(h)
    return True


u.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
if not target:
    raise SystemExit("no MvUI window")
hwnd = target[0]

r = wt.RECT()
u.GetWindowRect(hwnd, ctypes.byref(r))
w, h = r.right - r.left, r.bottom - r.top
print("window", r.left, r.top, w, h)

hdc = u.GetWindowDC(hwnd)
mdc = g.CreateCompatibleDC(hdc)
bmp = g.CreateCompatibleBitmap(hdc, w, h)
g.SelectObject(mdc, bmp)
# 2 = PW_RENDERFULLCONTENT
ok = u.PrintWindow(hwnd, mdc, 2)
print("PrintWindow ->", ok)

class BITMAPINFOHEADER(ctypes.Structure):
    _fields_ = [("biSize", wt.DWORD), ("biWidth", ctypes.c_long),
                ("biHeight", ctypes.c_long), ("biPlanes", wt.WORD),
                ("biBitCount", wt.WORD), ("biCompression", wt.DWORD),
                ("biSizeImage", wt.DWORD), ("biXPelsPerMeter", ctypes.c_long),
                ("biYPelsPerMeter", ctypes.c_long), ("biClrUsed", wt.DWORD),
                ("biClrImportant", wt.DWORD)]


bi = BITMAPINFOHEADER()
bi.biSize = ctypes.sizeof(BITMAPINFOHEADER)
bi.biWidth = w
bi.biHeight = -h  # negative → top-down rows
bi.biPlanes = 1
bi.biBitCount = 32
bi.biCompression = 0

buf = ctypes.create_string_buffer(w * h * 4)
g.GetDIBits(mdc, bmp, 0, h, buf, ctypes.byref(bi), 0)
img = Image.frombuffer("RGBA", (w, h), buf, "raw", "BGRA", 0, 1).convert("RGB")
img.save(OUT)
g.DeleteObject(bmp)
g.DeleteDC(mdc)
u.ReleaseDC(hwnd, hdc)
print("saved", OUT, img.size)
