"""Synthesise a real mouse drag inside the mamegui window.

usage: drag.py <x1> <y1> <x2> <y2> [steps]

Coordinates are *window-client* logical points (egui points); the window's
client origin is derived from GetClientRect/ClientToScreen so DPI scaling and
the title bar are handled for us.
"""
import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u = ctypes.windll.user32


def find_window():
    tl = subprocess.run(
        ["tasklist", "/FI", "IMAGENAME eq mamegui.exe", "/FO", "CSV", "/NH"],
        capture_output=True, text=True, errors="replace",
    ).stdout
    pids = {int(l.split('","')[1]) for l in tl.splitlines() if l.strip().startswith('"')}
    found = []

    def cb(h, _):
        pid = ctypes.c_ulong()
        u.GetWindowThreadProcessId(h, ctypes.byref(pid))
        if pid.value in pids and u.IsWindowVisible(h):
            n = u.GetWindowTextLengthW(h)
            b = ctypes.create_unicode_buffer(n + 1)
            u.GetWindowTextW(h, b, n + 1)
            found.append((h, b.value))
        return True

    u.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
    return [f for f in found if f[1]][0][0]


def client_origin(hwnd):
    pt = wt.POINT(0, 0)
    u.ClientToScreen(hwnd, ctypes.byref(pt))
    return pt.x, pt.y


def main():
    x1, y1, x2, y2 = (int(a) for a in sys.argv[1:5])
    steps = int(sys.argv[5]) if len(sys.argv) > 5 else 20
    hwnd = find_window()
    u.ShowWindow(hwnd, 9)
    u.SetWindowPos(hwnd, -1, 0, 0, 0, 0, 3)
    u.SetForegroundWindow(hwnd)
    time.sleep(0.3)
    u.SetWindowPos(hwnd, -2, 0, 0, 0, 0, 3)
    time.sleep(0.4)
    ox, oy = client_origin(hwnd)

    def move(x, y):
        # SendInput via mouse_event in *screen* pixels; the process is
        # DPI-aware so these are physical pixels, matching ClientToScreen.
        u.SetCursorPos(ox + x, oy + y)
        time.sleep(0.02)

    print(f"client origin {ox},{oy}; drag ({x1},{y1}) -> ({x2},{y2})")
    move(x1, y1)
    time.sleep(0.35)
    u.mouse_event(0x0002, 0, 0, 0, 0)  # LEFTDOWN
    time.sleep(0.2)
    for k in range(1, steps + 1):
        move(x1 + (x2 - x1) * k // steps, y1 + (y2 - y1) * k // steps)
    time.sleep(0.45)
    u.mouse_event(0x0004, 0, 0, 0, 0)  # LEFTUP
    time.sleep(0.9)
    print("done")


main()
