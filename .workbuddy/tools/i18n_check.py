"""Launch mamegui with a given language, open the Options window, screenshot.

usage: i18n_check.py <lang> <outdir>

The menu-bar item positions differ per language, so the Options entry is found
by clustering the dark text runs of the menu row instead of hard-coding x.
"""
import ctypes
import ctypes.wintypes as wt
import pathlib
import subprocess
import sys
import time

try:
    ctypes.windll.shcore.SetProcessDpiAwareness(2)
except Exception:
    ctypes.windll.user32.SetProcessDPIAware()

u = ctypes.windll.user32
from PIL import Image, ImageGrab

APP = pathlib.Path(r'C:/Users/11921/Desktop/mamepgui-rewrite/mamegui-rs/target/release')
INI = APP / '.mamepgui/mamepgui.ini'


def set_lang(lang: str):
    txt = INI.read_text(encoding='utf-8')
    lines = []
    found = False
    for ln in txt.splitlines():
        if ln.startswith('language='):
            lines.append(f'language={lang}')
            found = True
        else:
            lines.append(ln)
    if not found:
        lines.insert(1, f'language={lang}')
    INI.write_text('\n'.join(lines) + '\n', encoding='utf-8')


def find_window():
    tl = subprocess.run(["tasklist", "/FI", "IMAGENAME eq mamegui.exe", "/FO", "CSV", "/NH"],
                        capture_output=True, text=True, errors="replace").stdout
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


def shot(rect, path):
    for _ in range(4):
        try:
            ImageGrab.grab(all_screens=True).crop(rect).save(path)
            return True
        except Exception:
            time.sleep(1.5)
    return False


def main():
    lang, outdir = sys.argv[1], pathlib.Path(sys.argv[2])
    set_lang(lang)
    subprocess.Popen([str(APP / 'mamegui.exe')], cwd=str(APP),
                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     # detach: otherwise the launching shell tears the GUI down
                     # as soon as this script exits
                     creationflags=0x00000008 | 0x00000200, close_fds=True)
    time.sleep(14)
    hwnd = find_window()
    u.ShowWindow(hwnd, 3)          # maximise
    time.sleep(2.5)
    u.SetWindowPos(hwnd, -1, 0, 0, 0, 0, 3)
    u.SetForegroundWindow(hwnd)
    time.sleep(0.3)
    u.SetWindowPos(hwnd, -2, 0, 0, 0, 0, 3)
    time.sleep(0.8)
    r = wt.RECT()
    u.GetWindowRect(hwnd, ctypes.byref(r))
    rect = (r.left, r.top, r.right, r.bottom)
    pt = wt.POINT(0, 0)
    u.ClientToScreen(hwnd, ctypes.byref(pt))
    ox, oy = pt.x, pt.y
    shot(rect, outdir / f'main_{lang}.png')

    # locate the menu bar items: dark text runs on the top strip
    img = Image.open(outdir / f'main_{lang}.png').convert('L')
    px = img.load()
    W, H = img.size
    band = range(70, 105)          # window-relative rows of the menu bar
    cols = []
    for x in range(0, 900):
        if any(px[x, y] < 140 for y in band):
            cols.append(x)
    groups = []
    for c in cols:
        if groups and c - groups[-1][-1] <= 20:
            groups[-1].append(c)
        else:
            groups.append([c])
    # the window border shows up as a narrow run at x≈0 — not a menu item
    groups = [g for g in groups if (g[-1] - g[0]) >= 20]
    print('menu clusters:', [(g[0], g[-1]) for g in groups])
    if len(groups) < 3:
        print('could not find the Options menu')
        return
    g = groups[2] if len(groups) > 2 else groups[-1]   # File / View / Options / Help
    menu_x = (g[0] + g[-1]) // 2
    # window-relative (screenshot) -> client coords, via the measured frame size
    border_x = ox - r.left
    border_y = oy - r.top
    cx = menu_x - border_x
    cy = 88 - border_y
    print('menu x(w)', menu_x, 'border', border_x, border_y, '-> client', cx, cy)
    u.SetCursorPos(ox + cx, oy + cy)
    time.sleep(0.3)
    u.mouse_event(2, 0, 0, 0, 0)
    time.sleep(0.08)
    u.mouse_event(4, 0, 0, 0, 0)
    time.sleep(0.9)
    shot(rect, outdir / f'menu_{lang}.png')
    # second item of the Options menu = Default Game Options (the dialog)
    u.SetCursorPos(ox + cx + 60, oy + cy + 86)
    time.sleep(0.3)
    u.mouse_event(2, 0, 0, 0, 0)
    time.sleep(0.08)
    u.mouse_event(4, 0, 0, 0, 0)
    time.sleep(1.8)
    shot(rect, outdir / f'options_{lang}.png')
    subprocess.run(["taskkill", "/F", "/IM", "mamegui.exe"],
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    print('done', lang)


main()
