"""Bring mamegui to the front, move the mouse to a point over its window,
hold still, and screenshot. No clicking.

Usage: hover_and_shot.py out.png X Y pid   (X/Y absolute screen coords)

Window lookup is copied from launch_check.py (match by exe path — the old
1.8.2 binary is also named mvui.exe).
"""
import ctypes
import sys
import time
from ctypes import wintypes

from PIL import ImageGrab

u32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32
ctypes.windll.shcore.SetProcessDpiAwareness(2)

EXE = r"C:\Users\11921\Desktop\mamepgui-rewrite\mamegui-rs\target\release\mvui.exe"
WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)

out, x, y, pid = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])


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
    if wpid.value == pid and u32.IsWindowVisible(h) and exe_of(pid) == EXE:
        # prefer a window with a title (skips the 6x6 helper)
        n = u32.GetWindowTextLengthW(h)
        if n > 0:
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

# restore if minimized (GetWindowRect then reports -32000 offsets)
if r.left < -20000:
    u32.ShowWindow(hwnd, 9)  # SW_RESTORE
    time.sleep(0.5)
    u32.GetWindowRect(hwnd, ctypes.byref(r))
    print(f"restored rect=({r.left},{r.top},{r.right},{r.bottom})")

HWND_TOPMOST, SWP_NOSIZE, SWP_NOMOVE, SWP_SHOWWINDOW = -1, 0x1, 0x2, 0x40
u32.SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW)

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
u32.SetCursorPos(x, y)
time.sleep(0.4)

img = ImageGrab.grab(all_screens=True)
img.save(out)
print(f"saved {out} {img.size}")
