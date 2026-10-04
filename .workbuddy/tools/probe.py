"""Resize + activate the mamegui window, screenshot, click a list row, screenshot again.

Usage: probe.py outdir [w] [h]
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

from PIL import ImageGrab

# match GetWindowRect coordinates to ImageGrab's physical pixels
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

user32 = ctypes.windll.user32

outdir = sys.argv[1]
w = int(sys.argv[2]) if len(sys.argv) > 2 else 1600
h = int(sys.argv[3]) if len(sys.argv) > 3 else 900

tasklist = subprocess.run(
    ["tasklist", "/FI", "IMAGENAME eq mamegui.exe", "/FO", "CSV", "/NH"],
    capture_output=True, text=True, errors="replace",
).stdout
pids = {int(l.split('","')[1]) for l in tasklist.splitlines() if l.strip().startswith('"')}
print("pids:", pids)

found = []


def cb(hwnd, _):
    pid = ctypes.c_ulong()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if pid.value in pids and user32.IsWindowVisible(hwnd):
        n = user32.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        r = wt.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        found.append((hwnd, buf.value, (r.left, r.top, r.right, r.bottom)))
    return True


user32.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
win = [f for f in found if f[1]]
if not win:
    print("NO WINDOW")
    sys.exit(2)
hwnd, title, rect = max(win, key=lambda f: (f[2][2]-f[2][0])*(f[2][3]-f[2][1]))
print("window:", hex(hwnd), title, rect)

user32.ShowWindow(hwnd, 9)
user32.MoveWindow(hwnd, 60, 60, w, h, True)
time.sleep(0.6)
r0 = wt.RECT()
user32.GetWindowRect(hwnd, ctypes.byref(r0))
print("post-move rect:", (r0.left, r0.top, r0.right, r0.bottom), "zoomed:", user32.IsZoomed(hwnd))
user32.SetForegroundWindow(hwnd)
time.sleep(1.5)

r = wt.RECT()
user32.GetWindowRect(hwnd, ctypes.byref(r))
box = (r.left, r.top, r.right, r.bottom)
ImageGrab.grab(all_screens=True).crop(box).save(f"{outdir}/before.png")
print("before saved", box)

# click a row in the middle of the game list (right half of the window, a bit down)
cx = r.left + int(w * 0.62)
cy = r.top + 240
user32.SetCursorPos(cx, cy)
time.sleep(0.3)
user32.mouse_event(0x0002, 0, 0, 0, 0)  # LEFTDOWN
time.sleep(0.05)
user32.mouse_event(0x0004, 0, 0, 0, 0)  # LEFTUP
time.sleep(1.2)

ImageGrab.grab(all_screens=True).crop(box).save(f"{outdir}/after.png")
print("after saved")
