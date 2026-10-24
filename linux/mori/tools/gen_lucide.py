#!/usr/bin/env python3
"""Turns Lucide SVG icons into path operations for mori_icons.cc.

Mori draws Lucide icons using generated paths. Chromium's Skia is
built without SVG path parsing, so this converts each icon once into
absolute move/line/cubic/quad/arc/close/circle/ellipse operations.

  gen_lucide.py <lucide icons dir> <name>... > mori_lucide_icons.inc
"""
import math
import re
import sys
import xml.etree.ElementTree as ET

NUM = re.compile(r"[-+]?(?:\d*\.\d+|\d+\.?)(?:[eE][-+]?\d+)?")


class Scanner:
    """Reads SVG path data, including arc flags written without separators
    ("a2 2 0 0022 17" is flags 0, 0 then x 22)."""

    def __init__(self, d):
        self.d, self.i = d, 0

    def skip(self):
        while self.i < len(self.d) and self.d[self.i] in " ,\t\n":
            self.i += 1

    def at_command(self):
        self.skip()
        return self.i < len(self.d) and self.d[self.i].isalpha()

    def done(self):
        self.skip()
        return self.i >= len(self.d)

    def command(self):
        self.skip()
        c = self.d[self.i]
        self.i += 1
        return c

    def num(self):
        self.skip()
        m = NUM.match(self.d, self.i)
        if not m:
            raise ValueError(f"bad path data at {self.d[self.i:self.i+10]!r}")
        self.i = m.end()
        return float(m.group())

    def flag(self):
        self.skip()
        c = self.d[self.i]
        self.i += 1
        return float(c)


def path_ops(d):
    ops = []
    sc = Scanner(d)
    cx = cy = sx = sy = 0.0
    last_c2 = None  # last cubic control point (for S)
    last_q = None   # last quad control point (for T)
    cmd = None
    num = sc.num

    while not sc.done():
        if sc.at_command():
            cmd = sc.command()
        rel = cmd.islower()
        c = cmd.upper()
        ox, oy = (cx, cy) if rel else (0.0, 0.0)
        if c == "M":
            x, y = num() + ox, num() + oy
            ops.append(("M", x, y))
            cx, cy, sx, sy = x, y, x, y
            cmd = "l" if rel else "L"
            last_c2 = last_q = None
        elif c == "L":
            x, y = num() + ox, num() + oy
            ops.append(("L", x, y))
            cx, cy = x, y
            last_c2 = last_q = None
        elif c == "H":
            x = num() + (cx if rel else 0.0)
            ops.append(("L", x, cy))
            cx = x
            last_c2 = last_q = None
        elif c == "V":
            y = num() + (cy if rel else 0.0)
            ops.append(("L", cx, y))
            cy = y
            last_c2 = last_q = None
        elif c == "C":
            x1, y1 = num() + ox, num() + oy
            x2, y2 = num() + ox, num() + oy
            x, y = num() + ox, num() + oy
            ops.append(("C", x1, y1, x2, y2, x, y))
            last_c2, last_q = (x2, y2), None
            cx, cy = x, y
        elif c == "S":
            x1, y1 = (2 * cx - last_c2[0], 2 * cy - last_c2[1]) if last_c2 else (cx, cy)
            x2, y2 = num() + ox, num() + oy
            x, y = num() + ox, num() + oy
            ops.append(("C", x1, y1, x2, y2, x, y))
            last_c2, last_q = (x2, y2), None
            cx, cy = x, y
        elif c == "Q":
            x1, y1 = num() + ox, num() + oy
            x, y = num() + ox, num() + oy
            ops.append(("Q", x1, y1, x, y))
            last_q, last_c2 = (x1, y1), None
            cx, cy = x, y
        elif c == "T":
            x1, y1 = (2 * cx - last_q[0], 2 * cy - last_q[1]) if last_q else (cx, cy)
            x, y = num() + ox, num() + oy
            ops.append(("Q", x1, y1, x, y))
            last_q, last_c2 = (x1, y1), None
            cx, cy = x, y
        elif c == "A":
            rx, ry, rot = num(), num(), num()
            large, sweep = sc.flag(), sc.flag()
            x, y = num() + ox, num() + oy
            ops.append(("A", rx, ry, rot, large, sweep, x, y))
            cx, cy = x, y
            last_c2 = last_q = None
        elif c == "Z":
            ops.append(("Z",))
            cx, cy = sx, sy
            last_c2 = last_q = None
        else:
            raise ValueError(f"unsupported command {cmd}")
    return ops


def f(v, default=0.0):
    return float(v) if v is not None else default


def element_ops(el):
    tag = el.tag.split("}")[-1]
    a = el.attrib
    if tag == "path":
        return path_ops(a["d"])
    if tag == "line":
        return [("M", f(a.get("x1")), f(a.get("y1"))), ("L", f(a.get("x2")), f(a.get("y2")))]
    if tag in ("polyline", "polygon"):
        pts = [float(n) for n in NUM.findall(a["points"])]
        ops = [("M", pts[0], pts[1])] + [("L", pts[k], pts[k + 1]) for k in range(2, len(pts), 2)]
        return ops + ([("Z",)] if tag == "polygon" else [])
    if tag == "circle":
        return [("O", f(a.get("cx")), f(a.get("cy")), f(a.get("r")))]
    if tag == "ellipse":
        return [("E", f(a.get("cx")), f(a.get("cy")), f(a.get("rx")), f(a.get("ry")))]
    if tag == "rect":
        x, y = f(a.get("x")), f(a.get("y"))
        w, h = f(a["width"]), f(a["height"])
        rx = f(a.get("rx"), f(a.get("ry")))
        ry = f(a.get("ry"), rx)
        return [("R", x, y, w, h, rx, ry)]
    raise ValueError(f"unsupported element {tag}")


def literal(v):
    text = f"{v:g}"
    return (text if "." in text or "e" in text else text + ".0") + "f"


def main():
    folder, names = sys.argv[1], sys.argv[2:]
    print("// Generated by tools/gen_lucide.py from Lucide (ISC licence); do not edit.")
    for name in names:
        root = ET.parse(f"{folder}/{name}.svg").getroot()
        ops = []
        for el in root:
            ops += element_ops(el)
        ident = "k" + "".join(p.capitalize() for p in name.split("-"))
        print(f"constexpr IconOp {ident}Ops[] = {{")
        for op in ops:
            args = ", ".join(literal(v) for v in op[1:])
            print(f"    {{'{op[0]}', {{{args}}}}},")
        print("};")
    # Each icon also gets a Chromium VectorIcon named "mori::<name>", so Mori
    # code can pass it wherever Chromium takes an icon; its placeholder path
    # is never drawn (PaintReplacementIcon draws the Lucide one).
    for name in names:
        ident = "k" + "".join(p.capitalize() for p in name.split("-"))
        print(f'constexpr gfx::VectorIcon kMori{ident[1:]}Icon(kPlaceholderReps, 1, "mori::{name}");')
    print("constexpr LucideIconData kLucideIcons[] = {")
    for name in names:
        ident = "k" + "".join(p.capitalize() for p in name.split("-"))
        print(f'    {{"{name}", {ident}Ops, &kMori{ident[1:]}Icon}},')
    print("};")


if __name__ == "__main__":
    main()
