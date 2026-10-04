"""Bring the mamegui window to the front, click a point, and report if anything changed.

Usage: clickat.py out.png X Y [clicks]
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

from PIL import Image, ImageChops, ImageGrab

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

user32 = ctypes.windll.user32
SWP_NOSIZE = 0x0001
SWP_NOMOVE = 0x0002
HWND_TOPMOST = -1
HWND_NOTOPMOST = -2

out = sys.argv[1]
x = int(sys.argv[2])
y = int(sys.argv[3])
clicks = int(sys.argv[4]) if len(sys.argv) > 4 else 1

tasklist = subprocess.run(
    ["tasklist", "/FI", "IMAGENAME eq mvui.exe", "/FO", "CSV", "/NH"],
    capture_output=True, text=True, errors="replace",
).stdout
pids = {int(l.split('","')[1]) for l in tasklist.splitlines() if l.strip().startswith('"')}

found = []


def cb(hwnd, _):
    pid = ctypes.c_ulong()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if pid.value in pids and user32.IsWindowVisible(hwnd):
        n = user32.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(n + 1)
        user32.GetWindowTextW(hwnd, buf, n + 1)
        found.append((hwnd, buf.value))
    return True


user32.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
hwnd = [f for f in found if f[1]][0][0]


def grab():
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return ImageGrab.grab(all_screens=True).crop((r.left, r.top, r.right, r.bottom)), (
        r.left, r.top, r.right, r.bottom,
    )


def focus():
    user32.ShowWindow(hwnd, 9)
    user32.SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE)
    user32.SetForegroundWindow(hwnd)
    time.sleep(0.3)
    user32.SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE)


focus()
before, box = grab()

for _ in range(clicks):
    user32.SetCursorPos(x, y)
    time.sleep(0.25)
    user32.mouse_event(0x0002, 0, 0, 0, 0)
    time.sleep(0.06)
    user32.mouse_event(0x0004, 0, 0, 0, 0)
    time.sleep(0.45)

time.sleep(0.6)
after, box2 = grab()
after.save(out)
d = ImageChops.difference(before, after)
print("box:", box, "->", box2)
print("changed:", d.getbbox() is not None, "bbox:", d.getbbox())
