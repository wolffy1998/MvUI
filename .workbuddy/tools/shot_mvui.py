"""Screenshot the MvUI window reliably, even if it lands on another monitor.

Problems this solves, both hit in practice:
  * `shot_tree.py` maximized the window and cropped by its rect, but the window
    can sit on a secondary monitor — ImageGrab(all_screens=True) then returns
    the virtual desktop and the crop lands on whatever is really there.
  * so: force the window onto the primary monitor, un-maximize it to a known
    size, and only then grab.

usage: shot_mvui.py OUT.png
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

from PIL import ImageGrab

OUT = sys.argv[1]
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u = ctypes.windll.user32
k = ctypes.windll.kernel32
WANT = r"mamepgui-rewrite"  # the old 1.8.2 build has the same exe name
LEFT, TOP, WIDTH, HEIGHT = 60, 60, 1600, 1000

tl = subprocess.run(
    ["tasklist", "/FI", "IMAGENAME eq mvui.exe", "/FO", "CSV", "/NH"],
    capture_output=True, text=True, errors="replace",
).stdout
pids = {int(l.split('","')[1]) for l in tl.splitlines() if l.strip().startswith('"')}
mine = set()
for p in pids:
    h = k.OpenProcess(0x1000, False, p)
    if not h:
        continue
    try:
        buf = ctypes.create_unicode_buffer(1024)
        n = ctypes.c_ulong(1024)
        if k.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(n)) and WANT in buf.value:
            mine.add(p)
    finally:
        k.CloseHandle(h)
print("pids", sorted(pids), "ours", sorted(mine))
if not mine:
    raise SystemExit("mvui.exe not running")

found = []


def cb(h, _):
    pid = ctypes.c_ulong()
    u.GetWindowThreadProcessId(h, ctypes.byref(pid))
    if pid.value in mine and u.IsWindowVisible(h):
        n = u.GetWindowTextLengthW(h)
        b = ctypes.create_unicode_buffer(n + 1)
        u.GetWindowTextW(h, b, n + 1)
        if b.value == "MvUI":  # skip the mame.exe picker dialog
            found.append(h)
    return True


u.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
if not found:
    raise SystemExit("no MvUI window")
hwnd = found[0]

u.ShowWindow(hwnd, 1)  # SW_NORMAL — undo maximize
time.sleep(0.5)
u.SetWindowPos(hwnd, -1, LEFT, TOP, WIDTH, HEIGHT, 0x0040)  # TOPMOST on primary
time.sleep(0.6)
u.SetForegroundWindow(hwnd)
time.sleep(0.8)

r = wt.RECT()
u.GetWindowRect(hwnd, ctypes.byref(r))
print("rect", r.left, r.top, r.right - r.left, r.bottom - r.top)
img = ImageGrab.grab(all_screens=True).crop((r.left, r.top, r.right, r.bottom))
img.save(OUT)
u.SetWindowPos(hwnd, -2, 0, 0, 0, 0, 0x0040)  # NOTOPMOST
print("saved", OUT, img.size)
