"""Screenshot the mamegui window, cropping to its client area.

Usage: python winshot.py out.png [procname] [settle_seconds]
"""
import ctypes
import sys
import time

from PIL import ImageGrab

user32 = ctypes.windll.user32

# match GetWindowRect coordinates to ImageGrab's physical pixels
try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    user32.SetProcessDPIAware()

out = sys.argv[1] if len(sys.argv) > 1 else "win.png"
proc = sys.argv[2] if len(sys.argv) > 2 else "mamegui.exe"
settle = float(sys.argv[3]) if len(sys.argv) > 3 else 3.0

import subprocess

tasklist = subprocess.run(
    ["tasklist", "/FI", f"IMAGENAME eq {proc}", "/FO", "CSV", "/NH"],
    capture_output=True,
    text=True,
    errors="replace",
).stdout
pids = set()
for line in tasklist.splitlines():
    line = line.strip()
    if line.startswith('"'):
        pids.add(int(line.split('","')[1]))
print("pids:", pids)

found = []


def enum_proc(hwnd, _lparam):
    pid = ctypes.c_ulong()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    if pid.value in pids and user32.IsWindowVisible(hwnd):
        length = user32.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(length + 1)
        user32.GetWindowTextW(hwnd, buf, length + 1)
        cls = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, cls, 256)
        rect = ctypes.wintypes.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(rect))
        found.append((hwnd, buf.value, cls.value, (rect.left, rect.top, rect.right, rect.bottom)))
    return True


import ctypes.wintypes  # noqa: E402  (needs to exist before enum_proc uses RECT)

user32.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.wintypes.HWND, ctypes.wintypes.LPARAM)(enum_proc), 0)
print("windows:", found)

if not found:
    print("NO WINDOW FOUND")
    sys.exit(2)

hwnd, title, cls, rect = max(found, key=lambda f: (f[3][2] - f[3][0]) * (f[3][3] - f[3][1]))
print("chosen:", hex(hwnd), title, cls, rect)
user32.ShowWindow(hwnd, 9)  # SW_RESTORE
user32.SetForegroundWindow(hwnd)
time.sleep(settle)
img = ImageGrab.grab(all_screens=True)
img.save(out)
print("saved", out, img.size)
