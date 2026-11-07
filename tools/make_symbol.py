"""Write the custom SF Symbol of the Control Center button (spec UI-10, UI-17).

The symbol is the "menu-open" status glyph of tools/make_icon.py: the shell,
the full lens, and the stand. An SF Symbol must be filled outlines with no
strokes, so this script outlines the 1.4 stroke of the stand and joins all
the shapes into one outline.

Usage:

    uv run --with shapely python tools/make_symbol.py

Output: crates/telescreen-ui-macos/control/Assets.xcassets/telescreen.display.symbolset.
scripts/make-macos-control.sh compiles the catalog with actool (Xcode only).
"""

import json
import re
import sys
from pathlib import Path

from shapely.geometry import Point, Polygon
from shapely.geometry.polygon import orient
from shapely.ops import unary_union

sys.path.insert(0, str(Path(__file__).resolve().parent))
import make_icon  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
NAME = "telescreen.display"
OUTPUT = ROOT / "crates" / "telescreen-ui-macos" / "control" / "Assets.xcassets" / f"{NAME}.symbolset"

# The guides of the SF Symbols template. actool needs the baseline and the
# capline of each scale (S, M, L), also when only Regular-M has a drawing.
# The glyph is 16 units high; it goes 1.25 cap heights high, centered on the
# cap height of M.
GUIDES = {"S": (696.0, 625.54), "M": (1126.0, 1055.54), "L": (1556.0, 1485.54)}
BASELINE, CAPLINE = GUIDES["M"]
SCALE = 1.25 * (BASELINE - CAPLINE) / 16
LEFT = 1391.3


def polygons(d: str) -> list[Polygon]:
    """The subpaths of an SVG path that has only M, L, and Z commands."""
    out = []
    for sub in re.findall(r"M[^M]*", d):
        nums = [float(v) for v in re.findall(r"-?\d+(?:\.\d+)?", sub)]
        out.append(Polygon(list(zip(nums[0::2], nums[1::2]))).buffer(0))
    return out


def glyph():
    outer, visor = polygons(make_icon.GLYPH_SHELL)
    shell = outer.difference(visor)
    lens = Point(9, 6.6).buffer(2.5, quad_segs=32)
    (stand,) = polygons(make_icon.GLYPH_CRADLE)
    stand = stand.buffer(0.7, quad_segs=16)
    return unary_union([shell, lens, stand])


def path(shape) -> str:
    top = (BASELINE + CAPLINE) / 2 - 8 * SCALE

    def ring(coords) -> str:
        pts = [f"{LEFT + x * SCALE:.2f} {top + y * SCALE:.2f}" for x, y in list(coords)[:-1]]
        return "M" + " L".join(pts) + " Z"

    parts = []
    for poly in getattr(shape, "geoms", [shape]):
        # Outer rings and holes turn in opposite directions (nonzero fill).
        poly = orient(poly, sign=-1.0)
        parts.append(ring(poly.exterior.coords))
        parts.extend(ring(hole.coords) for hole in poly.interiors)
    return " ".join(parts)


def svg(d: str) -> str:
    right = LEFT + 18 * SCALE
    guides = "".join(
        f'  <path id="Baseline-{k}" d="M289.88 {b:g} L3010.12 {b:g}"/>\n'
        f'  <path id="Capline-{k}" d="M289.88 {c:g} L3010.12 {c:g}"/>\n'
        for k, (b, c) in GUIDES.items()
    )
    return f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<!-- Made by tools/make_symbol.py. Do not edit. -->
<svg version="1.1" xmlns="http://www.w3.org/2000/svg" width="3300" height="2200">
 <g id="Notes">
  <text id="template-version" x="0" y="0">Template v.2.0</text>
 </g>
 <g id="Guides">
{guides}  <path id="left-margin-Regular-M" d="M{LEFT:g} 1030 L{LEFT:g} 1150"/>
  <path id="right-margin-Regular-M" d="M{right:.2f} 1030 L{right:.2f} 1150"/>
 </g>
 <g id="Symbols">
  <g id="Regular-M">
   <path d="{d}"/>
  </g>
 </g>
</svg>
"""


def main() -> int:
    OUTPUT.mkdir(parents=True, exist_ok=True)
    (OUTPUT / f"{NAME}.svg").write_text(svg(path(glyph())))
    contents = {
        "info": {"author": "xcode", "version": 1},
        "symbols": [{"filename": f"{NAME}.svg", "idiom": "universal"}],
    }
    (OUTPUT / "Contents.json").write_text(json.dumps(contents, indent=2) + "\n")
    catalog = OUTPUT.parent / "Contents.json"
    catalog.write_text(json.dumps({"info": {"author": "xcode", "version": 1}}, indent=2) + "\n")
    print(f"make_symbol: wrote {OUTPUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
