import sys, time
from PIL import ImageGrab

out = sys.argv[1] if len(sys.argv) > 1 else "shot.png"
delay = float(sys.argv[2]) if len(sys.argv) > 2 else 0.0
if delay:
    time.sleep(delay)
img = ImageGrab.grab(all_screens=True)
img.save(out)
print("saved", out, img.size)
