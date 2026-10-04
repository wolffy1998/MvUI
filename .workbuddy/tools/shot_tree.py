"""Grab the mamegui window (ours only — the old 1.8.2 build has the same exe name).

usage: shot_tree.py OUT.png [click_x,click_y ...] [hover_x,hover_y] [wheel:N]
                           [drag:x1,y1,x2,y2]
Clicks/hover/drag are window-relative pixels. The cursor is parked over the game
list unless `hover:` is given, so nothing is left hovered by accident.
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

from PIL import ImageGrab

OUT = sys.argv[1]
CLICKS = []
HOVER = None
WHEEL = 0
DRAG = None
RCLICK = None
for a in sys.argv[2:]:
    if a.startswith("wheel:"):
        WHEEL = int(a.split(":")[1])
    elif a.startswith("hover:"):
        HOVER = tuple(int(v) for v in a.split(":")[1].split(","))
    elif a.startswith("rclick:"):
        RCLICK = tuple(int(v) for v in a.split(":")[1].split(","))
    elif a.startswith("drag:"):
        DRAG = tuple(int(v) for v in a.split(":")[1].split(","))
    else:
        x, y = a.split(",")
        CLICKS.append((int(x), int(y)))

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u = ctypes.windll.user32
k = ctypes.windll.kernel32
DOWN, UP, WHEEL_FLAG = 0x0002, 0x0004, 0x0800
WANT = r"mamepgui-rewrite"          # the old 1.8.2 build runs under the same exe name
PARK = (1800, 1200)                # over the game list, away from the tree


def image_path(pid):
    h = k.OpenProcess(0x1000, False, pid)
    if not h:
        return ""
    buf = ctypes.create_unicode_buffer(1024)
    n = ctypes.c_ulong(1024)
    ok = k.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(n))
    k.CloseHandle(h)
    return buf.value if ok else ""


tl = subprocess.run(
    ["tasklist", "/FI", "IMAGENAME eq mvui.exe", "/FO", "CSV", "/NH"],
    capture_output=True, text=True, errors="replace",
).stdout
pids = {int(l.split('","')[1]) for l in tl.splitlines() if l.strip().startswith('"')}
mine = {p for p in pids if WANT in image_path(p)}
print("pids", sorted(pids), "ours", sorted(mine))
found = []


def cb(h, _):
    pid = ctypes.c_ulong()
    u.GetWindowThreadProcessId(h, ctypes.byref(pid))
    if pid.value in mine and u.IsWindowVisible(h):
        n = u.GetWindowTextLengthW(h)
        b = ctypes.create_unicode_buffer(n + 1)
        u.GetWindowTextW(h, b, n + 1)
        found.append((h, b.value))
    return True


u.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
titled = [f for f in found if f[1]]
if not titled:
    raise SystemExit("no mamegui window of ours")
hwnd = titled[0][0]

u.ShowWindow(hwnd, 3)          # maximize
time.sleep(2.0)
u.SetWindowPos(hwnd, -1, 0, 0, 0, 0, 3)   # topmost
u.SetForegroundWindow(hwnd)
time.sleep(0.4)
# NB: stay topmost until after the interactions and the capture, otherwise
# another window can steal the click.

r = wt.RECT()
u.GetWindowRect(hwnd, ctypes.byref(r))


def at(p):
    return r.left + p[0], r.top + p[1]


def click(p, hold=0.06):
    u.SetCursorPos(*at(p))
    time.sleep(0.25)
    u.mouse_event(DOWN, 0, 0, 0, 0)
    time.sleep(hold)
    u.mouse_event(UP, 0, 0, 0, 0)
    time.sleep(0.5)


for c in CLICKS:
    click(c)

if RCLICK:
    u.SetCursorPos(*at(RCLICK))
    time.sleep(0.3)
    u.mouse_event(0x0008, 0, 0, 0, 0)   # RIGHTDOWN
    time.sleep(0.06)
    u.mouse_event(0x0010, 0, 0, 0, 0)   # RIGHTUP
    time.sleep(0.7)

if DRAG:
    x1, y1, x2, y2 = DRAG
    u.SetCursorPos(*at((x1, y1)))
    time.sleep(0.3)
    u.mouse_event(DOWN, 0, 0, 0, 0)
    time.sleep(0.15)
    steps = 12
    for i in range(1, steps + 1):
        u.SetCursorPos(*at((x1 + (x2 - x1) * i // steps, y1 + (y2 - y1) * i // steps)))
        time.sleep(0.05)
    time.sleep(0.3)
    u.mouse_event(UP, 0, 0, 0, 0)
    time.sleep(0.6)

if WHEEL:
    u.SetCursorPos(*at((150, 500)))
    for _ in range(WHEEL):
        u.mouse_event(WHEEL_FLAG, 0, 0, -240, 0)
        time.sleep(0.05)
    time.sleep(0.8)

u.SetCursorPos(*at(HOVER if HOVER else PARK))
time.sleep(0.8)
img = ImageGrab.grab(all_screens=True).crop((r.left, r.top, r.right, r.bottom))
img.save(OUT)
u.SetWindowPos(hwnd, -2, 0, 0, 0, 0, 3)   # untopmost again
print("window", r.left, r.top, r.right - r.left, r.bottom - r.top, "->", OUT, img.size)
