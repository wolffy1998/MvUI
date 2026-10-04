# -*- coding: utf-8 -*-
# probe: render heart_layer with ow=0 in isolation
import importlib.util, sys

src = open("make_mvui.py", encoding="utf-8").read()
head = src.split("NAVY = (16, 42, 82, 255)")[0] + "NAVY = (16, 42, 82, 255)\n"
ns = {}
exec(head, ns)
h = ns["heart_layer"](150, (255, 205, 216), (255, 148, 170),
                      (16, 42, 82, 255), 0, (255, 235, 240, 150))
h.save("_heart_ow0.png")
print("heart saved", h.size)
