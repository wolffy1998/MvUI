"""Click inside the MvUI window, DPI-correctly, and capture the result.

Why this file exists
--------------------
Two earlier attempts both failed, for reasons worth not repeating:

* `SetCursorPos` + `mouse_event` — the calling process is per-monitor-DPI-aware
  (see the `SetProcessDpiAwareness` call), so those coordinates are *physical*
  pixels while the values people tend to read off a screenshot are *logical*
  ones. On this machine (4K at 200 %) the two differ by exactly 2x, so every
  click landed in the wrong place.
* `PostMessage(WM_LBUTTONDOWN)` — egui reads the pointer through winit, which on
  Windows comes from real input, not from posted window messages. The click is
  delivered and nothing happens.

What works is `SendInput` with `MOUSEEVENTF_ABSOLUTE`, which takes normalised
0..65535 virtual-screen coordinates and is DPI-unaware by design. Client
coordinates are converted with `ClientToScreen`, so the caller can think in the
same pixels `PrintWindow` hands back.

Usage
-----
    python ui_click.py shot out.png              # just capture
    python ui_click.py click 166 34 out.png      # click client (166,34), capture
    python ui_click.py click 166 34 --esc        # ... then press Escape
"""

import ctypes
import ctypes.wintypes as wt
import subprocess
import sys
import time

from PIL import Image

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u = ctypes.windll.user32
g = ctypes.windll.gdi32

MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_ABSOLUTE = 0x8000
MOUSEEVENTF_WHEEL = 0x0800
KEYEVENTF_KEYUP = 0x0002

INPUT_MOUSE = 0
INPUT_KEYBOARD = 1
VK_ESCAPE = 0x1B


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wt.LONG),
        ("dy", wt.LONG),
        ("mouseData", wt.DWORD),
        ("dwFlags", wt.DWORD),
        ("time", wt.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class KEYBDINPUT(ctypes.Structure):
    _fields_ = [
        ("wVk", wt.WORD),
        ("wScan", wt.WORD),
        ("dwFlags", wt.DWORD),
        ("time", wt.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class _INPUTUNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("ki", KEYBDINPUT)]


class INPUT(ctypes.Structure):
    _anonymous_ = ("u",)
    _fields_ = [("type", wt.DWORD), ("u", _INPUTUNION)]


def _send(flags, data=0):
    mi = MOUSEINPUT(data, data, 0, flags, 0, None)
    inp = INPUT(type=INPUT_MOUSE)
    inp.mi = mi
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def _send_key(vk, up=False):
    ki = KEYBDINPUT(vk, 0, KEYEVENTF_KEYUP if up else 0, 0, None)
    inp = INPUT(type=INPUT_KEYBOARD)
    inp.ki = ki
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def virtual():
    return (
        u.GetSystemMetrics(76),
        u.GetSystemMetrics(77),
        max(1, u.GetSystemMetrics(78) - 1),
        max(1, u.GetSystemMetrics(79) - 1),
    )


def find_window(title="MvUI", exe="mvui.exe"):
    out = subprocess.run(
        ["tasklist", "/FI", f"IMAGENAME eq {exe}", "/FO", "CSV", "/NH"],
        capture_output=True, text=True, errors="replace",
    ).stdout
    pids = {
        int(l.split('","')[1])
        for l in out.splitlines()
        if l.strip().startswith('"')
    }
    found = {}

    def cb(h, _):
        pid = ctypes.c_ulong()
        u.GetWindowThreadProcessId(h, ctypes.byref(pid))
        if pid.value in pids and u.IsWindowVisible(h):
            n = u.GetWindowTextLengthW(h)
            b = ctypes.create_unicode_buffer(n + 1)
            u.GetWindowTextW(h, b, n + 1)
            if b.value == title:
                found["h"] = h
        return True

    u.EnumWindows(ctypes.WINFUNCTYPE(ctypes.c_bool, wt.HWND, wt.LPARAM)(cb), 0)
    if "h" not in found:
        raise SystemExit(f"no visible window titled {title!r} for {exe}")
    return found["h"]


def client_size(hwnd):
    r = wt.RECT()
    u.GetClientRect(hwnd, ctypes.byref(r))
    return r.right - r.left, r.bottom - r.top


def focus(hwnd):
    u.ShowWindow(hwnd, 9)  # SW_RESTORE
    time.sleep(0.4)
    u.SetForegroundWindow(hwnd)
    time.sleep(0.6)
    u.BringWindowToTop(hwnd)
    time.sleep(0.3)


def move(hwnd, cx, cy):
    """Client pixels -> normalised absolute input. Returns the screen point."""
    org = wt.POINT(0, 0)
    u.ClientToScreen(hwnd, ctypes.byref(org))
    sx, sy = org.x + cx, org.y + cy
    vx, vy, vw, vh = virtual()
    nx = int((sx - vx) * 65535 / vw)
    ny = int((sy - vy) * 65535 / vh)
    _send(MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE, 0)
    # dx/dy travel in the same struct, so build it explicitly rather than via
    # the `data` shortcut above
    mi = MOUSEINPUT(nx, ny, 0, MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_MOVE, 0, None)
    inp = INPUT(type=INPUT_MOUSE)
    inp.mi = mi
    u.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))
    return sx, sy


def click(hwnd, cx, cy, settle=0.9):
    sx, sy = move(hwnd, cx, cy)
    time.sleep(0.35)
    _send(MOUSEEVENTF_LEFTDOWN)
    time.sleep(0.07)
    _send(MOUSEEVENTF_LEFTUP)
    time.sleep(settle)
    got = wt.POINT()
    u.GetCursorPos(ctypes.byref(got))
    return (sx, sy), (got.x, got.y)


def hover(hwnd, cx, cy, settle=0.6):
    move(hwnd, cx, cy)
    time.sleep(settle)


def escape():
    _send_key(VK_ESCAPE)
    time.sleep(0.1)
    _send_key(VK_ESCAPE, up=True)
    time.sleep(0.4)


def shot(hwnd, path):
    r = wt.RECT()
    u.GetClientRect(hwnd, ctypes.byref(r))
    w, h = r.right, r.bottom
    hdc = u.GetDC(hwnd)
    mem = g.CreateCompatibleDC(hdc)
    bmp = g.CreateCompatibleBitmap(hdc, w, h)
    g.SelectObject(mem, bmp)
    u.PrintWindow(hwnd, mem, 2)  # PW_RENDERFULLCONTENT

    class BITMAPINFOHEADER(ctypes.Structure):
        _fields_ = [
            ("biSize", wt.DWORD), ("biWidth", ctypes.c_long),
            ("biHeight", ctypes.c_long), ("biPlanes", wt.WORD),
            ("biBitCount", wt.WORD), ("biCompression", wt.DWORD),
            ("biSizeImage", wt.DWORD), ("biXPelsPerMeter", ctypes.c_long),
            ("biYPelsPerMeter", ctypes.c_long), ("biClrUsed", wt.DWORD),
            ("biClrImportant", wt.DWORD),
        ]

    hdr = BITMAPINFOHEADER()
    hdr.biSize = ctypes.sizeof(hdr)
    hdr.biWidth, hdr.biHeight = w, -h
    hdr.biPlanes, hdr.biBitCount = 1, 32
    buf = ctypes.create_string_buffer(w * h * 4)
    g.GetDIBits(mem, bmp, 0, h, buf, ctypes.byref(hdr), 0)
    img = Image.frombuffer("RGBA", (w, h), buf, "raw", "BGRA", 0, 1).convert("RGB")
    img.save(path)
    g.DeleteObject(bmp)
    g.DeleteDC(mem)
    u.ReleaseDC(hwnd, hdc)
    return img.size


def main(argv):
    hwnd = find_window()
    focus(hwnd)
    w, h = client_size(hwnd)
    vx, vy, vw, vh = virtual()
    print(f"client {w}x{h}  virtual origin({vx},{vy}) size({vw}x{vh})")

    out = None
    if "--shot" in argv:
        i = argv.index("--shot")
        out = argv[i + 1]
        argv = argv[:i]

    i = 0
    while i < len(argv):
        verb = argv[i]
        if verb == "click":
            cx, cy = int(argv[i + 1]), int(argv[i + 2])
            want, got = click(hwnd, cx, cy)
            print(f"click client({cx},{cy}) -> screen{want}  cursor{got}")
            i += 3
        elif verb == "hover":
            cx, cy = int(argv[i + 1]), int(argv[i + 2])
            hover(hwnd, cx, cy)
            print(f"hover client({cx},{cy})")
            i += 3
        elif verb == "esc":
            escape()
            print("escape")
            i += 1
        else:
            raise SystemExit(f"unknown verb {verb!r}")

    if out:
        time.sleep(0.5)
        print("saved", out, shot(hwnd, out))


if __name__ == "__main__":
    main(sys.argv[1:])
