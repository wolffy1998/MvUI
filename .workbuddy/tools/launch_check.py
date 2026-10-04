"""Check the freshly built app: find its window(s) by exe path and shoot it.

Usage: python launch_check.py <pid> <out.png>

Why "by exe path": more than one build can run at once (an older binary, or a
build from a different target dir), so matching on the process name alone can
grab the wrong window (it did before).
DPI awareness must be set first or GetWindowRect returns logical pixels while
ImageGrab returns physical ones and every crop is offset.

Pure ctypes — pywin32 is not installed in this env.
"""

import ctypes
import sys
from ctypes import wintypes

from PIL import ImageGrab

u32 = ctypes.windll.user32
k32 = ctypes.windll.kernel32
ctypes.windll.shcore.SetProcessDpiAwareness(2)

EXE = r"C:\Users\11921\Desktop\mamepgui-rewrite\mamegui-rs\target\release\mvui.exe"
WNDENUMPROC = ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)


def exe_of(pid: int) -> str:
    h = k32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
    if not h:
        return ""
    try:
        buf = ctypes.create_unicode_buffer(1024)
        size = wintypes.DWORD(1024)
        ok = k32.QueryFullProcessImageNameW(h, 0, buf, ctypes.byref(size))
        return buf.value if ok else ""
    finally:
        k32.CloseHandle(h)


def windows_of(pid: int):
    found = []

    def cb(hwnd, _):
        wpid = wintypes.DWORD()
        u32.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value != pid:
            return True
        if not u32.IsWindowVisible(hwnd):
            return True
        rect = wintypes.RECT()
        u32.GetWindowRect(hwnd, ctypes.byref(rect))
        length = u32.GetWindowTextLengthW(hwnd)
        buf = ctypes.create_unicode_buffer(length + 1)
        u32.GetWindowTextW(hwnd, buf, length + 1)
        found.append((hwnd, buf.value, (rect.left, rect.top, rect.right, rect.bottom)))
        return True

    u32.EnumWindows(WNDENUMPROC(cb), 0)
    return found


def main() -> None:
    pid = int(sys.argv[1])
    out = sys.argv[2]

    path = exe_of(pid)
    print("exe:", path)
    print("match:", "yes" if path.lower() == EXE.lower() else "NO — wrong process")

    wins = windows_of(pid)
    for hwnd, title, r in wins:
        print(f"hwnd={hwnd} title={title!r} size={r[2]-r[0]}x{r[3]-r[1]} rect={r}")
    if not wins:
        print("NO VISIBLE WINDOW — maybe the mame.exe picker dialog is up")
        return

    hwnd, _, r = wins[0]
    u32.SetWindowPos(hwnd, -1, r[0], r[1], r[2] - r[0], r[3] - r[1], 0x0040)  # TOPMOST
    u32.SetForegroundWindow(hwnd)
    u32.SetWindowPos(hwnd, -2, r[0], r[1], r[2] - r[0], r[3] - r[1], 0x0040)  # NOTOPMOST
    img = ImageGrab.grab(all_screens=True)
    img.crop(r).save(out)
    print("saved", out, img.size)


if __name__ == "__main__":
    main()
