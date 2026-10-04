"""Simulate a header drag on mamegui: press at (x0,y0), move in steps to
(x1,y1), screenshot mid-drag (ghost should float with the pointer), release,
screenshot again (column should have moved).

Usage: drag_shot.py pid x0 y0 x1 y1 mid.png final.png
"""
import ctypes
import sys
import time
from ctypes import wintypes

from PIL import ImageGrab

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32

pid, x0, y0, x1, y1, mid_png, final_png = (
    int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]),
    int(sys.argv[4]), int(sys.argv[5]), sys.argv[6], sys.argv[7],
)

EXE = r"C:\Users\11921\Desktop\mamepgui-rewrite\mamegui-rs\target\release\mvui.exe"
WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)


def exe_of(p: int) -> str:
    h = k32.OpenProcess(0x1000, False, p)
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wintypes.DWORD(1024)
        ok = k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size))
        return buf.value if ok else ""
    finally:
        k32.CloseHandle(h)


hwnd = None


def cb(h, _):
    global hwnd
    wpid = wintypes.DWORD()
    u32.GetWindowThreadProcessId(h, ctypes.byref(wpid))
    if wpid.value == pid and u32.IsWindowVisible(h) and u32.GetWindowTextLengthW(h) > 0:
        hwnd = h
        return False
    return True


u32.EnumWindows(WNDENUMPROC(cb), 0)
if hwnd is None:
    print("no window")
    sys.exit(1)

r = wintypes.RECT()
u32.GetWindowRect(hwnd, ctypes.byref(r))
print(f"hwnd={hwnd} rect=({r.left},{r.top},{r.right},{r.bottom})")
if r.left < -20000:
    u32.ShowWindow(hwnd, 9)
    time.sleep(0.5)
    u32.GetWindowRect(hwnd, ctypes.byref(r))
    print(f"restored rect=({r.left},{r.top},{r.right},{r.bottom})")

# foreground
fg = u32.GetForegroundWindow()
t_fg = u32.GetWindowThreadProcessId(fg, None)
t_me = k32.GetCurrentThreadId()
ati = u32.AttachThreadInput
ati.argtypes = [wintypes.DWORD, wintypes.DWORD, wintypes.BOOL]
if t_fg and t_fg != t_me:
    ati(t_fg, t_me, True)
u32.SetForegroundWindow(hwnd)
if t_fg and t_fg != t_me:
    ati(t_fg, t_me, False)
time.sleep(0.3)

MOUSEEVENTF_MOVE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP = 0x0001, 0x0002, 0x0004


# SendInput via a proper struct
class MOUSEINPUT(ctypes.Structure):
    _fields_ = [("dx", ctypes.c_long), ("dy", ctypes.c_long),
                ("mouseData", ctypes.c_ulong), ("dwFlags", ctypes.c_ulong),
                ("time", ctypes.c_ulong), ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong))]


class _INPUTunion(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ulong), ("union", _INPUTunion)]


def send(flags, dx=0, dy=0):
    mi = MOUSEINPUT(dx, dy, 0, flags, 0, None)
    inp = INPUT(0, _INPUTunion(mi=mi))
    u32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))


def move_to(x, y, steps=8):
    u32.SetCursorPos(x, y)
    return


# press
u32.SetCursorPos(x0, y0)
time.sleep(0.15)
send(MOUSEEVENTF_LEFTDOWN)
time.sleep(0.15)

# drag in steps so egui sees intermediate pointer positions
steps = 10
for i in range(1, steps + 1):
    xi = x0 + (x1 - x0) * i // steps
    yi = y0 + (y1 - y0) * i // steps
    u32.SetCursorPos(xi, yi)
    time.sleep(0.04)

# mid-drag screenshot (button still down)
time.sleep(0.25)
ImageGrab.grab(all_screens=True).save(mid_png)
print(f"saved {mid_png}")

# release
send(MOUSEEVENTF_LEFTUP)
time.sleep(0.35)
ImageGrab.grab(all_screens=True).save(final_png)
print(f"saved {final_png}")
