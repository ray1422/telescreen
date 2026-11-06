"""Render the app icon (concept G, "CRT glass") to assets/macos/Telescreen.icns.

The shapes and the effects come from the "Icon concepts" artboard of the UI
mockup canvas. Each size has its own variant, as on the artboard: the CRT
filter from 128 px up, simple shapes at 32 px and 16 px, and scan lines about
2 px apart.

Usage (macOS; needs Google Chrome and iconutil). It renders with Chrome,
because the artboard effects were checked in Chrome:

    uv run python tools/make_icon.py

The script writes temporary files to a temporary directory only.
"""

import math
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUTPUT = ROOT / "assets" / "macos" / "Telescreen.icns"
CHROME = Path("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")

# The defaults of the artboard tweaks.
ABERRATION = 0.7
GLOW = 0.9
SCAN = 0.22
VIGNETTE = 0.65
BULGE = 10
CAMERA_EXPONENT = 3
CORNER_EXPONENT = 5
SPREAD = 1.7

# Apple icon grid: the body is 824 of 1024 px.
BODY = 824 / 1024


def f(v: float) -> str:
    return f"{round(v * 100) / 100:g}"


def sq(x: float, y: float, w: float, h: float, r: float, p: float) -> str:
    """A rounded rectangle with superellipse corners (G2 continuous)."""
    e = min(r * SPREAD, w / 2, h / 2)
    pts = []

    def corner(cx: float, cy: float, sx: int, sy: int, from_top: bool) -> None:
        for i in range(17):
            t = ((16 - i) if from_top else i) / 16 * math.pi / 2
            ux = math.cos(t) ** (2 / p)
            uy = math.sin(t) ** (2 / p)
            pts.append(f"{f(cx + sx * e * ux)} {f(cy + sy * e * uy)}")

    corner(x + w - e, y + e, 1, -1, True)
    corner(x + w - e, y + h - e, 1, 1, False)
    corner(x + e, y + h - e, -1, 1, True)
    corner(x + e, y + e, -1, -1, False)
    return "M" + "L".join(pts) + "Z"


def band(b: float) -> str:
    """The band bows toward the viewer, as a straight line on a curved tube."""
    return f"M-4 66 Q50 {f(44 - b * 0.4)} 104 16 L104 32 Q50 {f(62 - b * 0.4)} -4 82Z"


N, CAM = CORNER_EXPONENT, CAMERA_EXPONENT
BG = sq(0, 0, 100, 100, 22, N)
HEAD = sq(14, 12, 72, 56, 40, CAM)
VISOR = sq(22, 19, 56, 42, 40, CAM)
DOOR = sq(30, 24.5, 40, 30, 40, CAM)
BASE = sq(32, 70, 36, 11, 40, CAM)
HEAD32 = sq(10, 8, 80, 62, 40, CAM)
VISOR32 = sq(19, 16, 62, 46, 40, CAM)
BASE32 = sq(30, 73, 40, 12, 40, CAM)
HEAD16 = sq(4, 6, 92, 72, 40, CAM)
VISOR16 = sq(15, 15, 70, 52, 40, CAM)
BAND = band(BULGE)
BAND16 = "M-4 70 Q50 44 104 14 L104 40 Q50 66 -4 94Z"

DEFS = f"""
<filter id="crt" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">
  <feColorMatrix in="SourceGraphic" type="matrix" values="1 0 0 0 0  0 0 0 0 0  0 0 0 0 0  0 0 0 1 0" result="r"/>
  <feOffset in="r" dx="{ABERRATION}" dy="0" result="rs"/>
  <feColorMatrix in="SourceGraphic" type="matrix" values="0 0 0 0 0  0 1 0 0 0  0 0 0 0 0  0 0 0 1 0" result="g"/>
  <feColorMatrix in="SourceGraphic" type="matrix" values="0 0 0 0 0  0 0 0 0 0  0 0 1 0 0  0 0 0 1 0" result="b"/>
  <feOffset in="b" dx="{-ABERRATION}" dy="0" result="bs"/>
  <feBlend in="rs" in2="g" mode="screen" result="rg"/>
  <feBlend in="rg" in2="bs" mode="screen" result="rgb"/>
  <feGaussianBlur in="SourceGraphic" stdDeviation="{GLOW}" result="bl"/>
  <feComponentTransfer in="bl" result="halo"><feFuncA type="linear" slope="0.7"/></feComponentTransfer>
  <feMerge><feMergeNode in="halo"/><feMergeNode in="rgb"/></feMerge>
</filter>
<filter id="rimGlow" x="-20%" y="-20%" width="140%" height="140%"><feGaussianBlur stdDeviation="1.1"/></filter>
<radialGradient id="phosphor" cx="50%" cy="46%" r="62%">
  <stop offset="0%" stop-color="#2f3d37"/><stop offset="70%" stop-color="#18201d"/><stop offset="100%" stop-color="#0a0d0c"/>
</radialGradient>
<radialGradient id="vignette" cx="50%" cy="48%" r="60%">
  <stop offset="55%" stop-color="#000000" stop-opacity="0"/><stop offset="100%" stop-color="#000000" stop-opacity="{VIGNETTE}"/>
</radialGradient>
<linearGradient id="reflect" x1="0" y1="0" x2="0" y2="1">
  <stop offset="0%" stop-color="#ffffff" stop-opacity="0.22"/><stop offset="100%" stop-color="#ffffff" stop-opacity="0"/>
</linearGradient>
<linearGradient id="shell" x1="0.2" y1="0" x2="0.8" y2="1">
  <stop offset="0%" stop-color="#f7f3ea"/><stop offset="60%" stop-color="#ddd6c6"/><stop offset="100%" stop-color="#b3ab98"/>
</linearGradient>
<radialGradient id="visor" cx="45%" cy="30%" r="80%">
  <stop offset="0%" stop-color="#1d2a27"/><stop offset="100%" stop-color="#040606"/>
</radialGradient>
<radialGradient id="lens" cx="38%" cy="35%" r="70%">
  <stop offset="0%" stop-color="#6f86ff"/><stop offset="45%" stop-color="#2a3a9c"/><stop offset="100%" stop-color="#070b22"/>
</radialGradient>
<clipPath id="clip"><path d="{BG}"/></clipPath>
"""


def scan_pattern(px: int, opacity: float) -> str:
    """Scan lines about 2 px apart; the large icons use the 320 px spacing."""
    period = max(1.25, 200 / (px * BODY))
    return (
        f'<pattern id="scan" width="{period:g}" height="{period:g}" patternUnits="userSpaceOnUse">'
        f'<rect width="{period:g}" height="{period / 2:g}" fill="#000000" opacity="{opacity:g}"/></pattern>'
    )


def art(px: int) -> str:
    """The content of the 0..100 square for an icon of `px` pixels."""
    glass = (
        f'<path d="{BG}" fill="url(#scan)"/>'
        f'<path d="{BG}" fill="url(#vignette)"/>'
        '<ellipse cx="36" cy="20" rx="30" ry="12" fill="url(#reflect)" transform="rotate(-18 36 20)"/>'
    )
    if px >= 256:
        return (
            f'<path d="{BG}" fill="url(#phosphor)"/>'
            '<g filter="url(#crt)">'
            f'<path d="{BAND}" fill="#d6402b"/>'
            '<ellipse cx="50" cy="82.5" rx="21" ry="3" fill="#000000" opacity="0.45"/>'
            f'<path d="{BASE}" fill="url(#shell)"/>'
            '<rect x="40" y="66" width="20" height="5" fill="#7d7768"/>'
            f'<path d="{HEAD}" fill="url(#shell)"/>'
            f'<path d="{VISOR}" fill="url(#visor)"/>'
            f'<path d="{DOOR}" fill="#0f2421" opacity="0.9"/>'
            '<circle cx="50" cy="40" r="9.5" fill="#22282c"/>'
            '<circle cx="50" cy="40" r="7" fill="url(#lens)"/>'
            '<circle cx="47.4" cy="37.4" r="1.7" fill="#ffffff" opacity="0.85"/>'
            '<circle cx="66" cy="27.5" r="1.2" fill="#7be0a0"/>'
            '<rect x="47" y="75" width="6" height="1.2" rx="0.6" fill="#4a4d50"/>'
            "</g>"
            f'<path d="{VISOR}" fill="none" stroke="#8fe6cf" stroke-width="1.4" opacity="0.55" filter="url(#rimGlow)"/>'
            + glass
        )
    if px >= 128:
        return (
            f'<path d="{BG}" fill="url(#phosphor)"/>'
            '<g filter="url(#crt)">'
            f'<path d="{BAND}" fill="#d6402b"/>'
            '<ellipse cx="50" cy="82.5" rx="21" ry="3" fill="#000000" opacity="0.45"/>'
            f'<path d="{BASE}" fill="url(#shell)"/>'
            '<rect x="40" y="66" width="20" height="5" fill="#7d7768"/>'
            f'<path d="{HEAD}" fill="url(#shell)"/>'
            f'<path d="{VISOR}" fill="url(#visor)"/>'
            f'<path d="{DOOR}" fill="#0f2421" opacity="0.9"/>'
            '<circle cx="50" cy="40" r="9.5" fill="#22282c"/>'
            '<circle cx="50" cy="40" r="7" fill="url(#lens)"/>'
            '<circle cx="47.4" cy="37.4" r="1.7" fill="#ffffff" opacity="0.85"/>'
            "</g>"
            f'<path d="{VISOR}" fill="none" stroke="#8fe6cf" stroke-width="1.6" opacity="0.5" filter="url(#rimGlow)"/>'
            + glass
        )
    if px >= 64:
        return (
            f'<path d="{BG}" fill="url(#phosphor)"/>'
            f'<path d="{BAND}" fill="#d6402b"/>'
            f'<path d="{BASE}" fill="url(#shell)"/>'
            f'<path d="{HEAD}" fill="url(#shell)"/>'
            f'<path d="{VISOR}" fill="url(#visor)"/>'
            '<circle cx="50" cy="40" r="8.5" fill="url(#lens)"/>' + glass
        )
    if px >= 32:
        return (
            f'<path d="{BG}" fill="#1a221f"/>'
            f'<path d="{BAND}" fill="#d6402b"/>'
            f'<path d="{BASE32}" fill="#e6dfcf"/>'
            f'<path d="{HEAD32}" fill="#e6dfcf"/>'
            f'<path d="{VISOR32}" fill="#060908"/>'
            '<circle cx="50" cy="39" r="10" fill="#3a4fc0"/>'
            f'<path d="{BG}" fill="url(#scan)"/>'
        )
    return (
        f'<path d="{BG}" fill="#1a221f"/>'
        f'<path d="{BAND16}" fill="#d6402b"/>'
        f'<path d="{HEAD16}" fill="#e6dfcf"/>'
        f'<path d="{VISOR16}" fill="#060908"/>'
        f'<path d="{BG}" fill="url(#scan)"/>'
    )


def svg(px: int) -> str:
    # The 0..100 art is the body of the Apple grid; the rest is transparent.
    side = 100 / BODY
    origin = -(side - 100) / 2
    scan = SCAN if px >= 64 else SCAN * 0.6
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{px}" height="{px}" '
        f'viewBox="{origin:g} {origin:g} {side:g} {side:g}">'
        f"<defs>{DEFS}{scan_pattern(px, scan)}</defs>"
        f'<g clip-path="url(#clip)">{art(px)}</g></svg>'
    )


# iconutil names and pixel sizes.
ICONSET = [
    ("icon_16x16", 16), ("icon_16x16@2x", 32),
    ("icon_32x32", 32), ("icon_32x32@2x", 64),
    ("icon_128x128", 128), ("icon_128x128@2x", 256),
    ("icon_256x256", 256), ("icon_256x256@2x", 512),
    ("icon_512x512", 512), ("icon_512x512@2x", 1024),
]


def render(svg_path: Path, png_path: Path, px: int) -> None:
    subprocess.run(
        [
            str(CHROME), "--headless", "--disable-gpu", "--hide-scrollbars",
            "--force-device-scale-factor=1", "--default-background-color=00000000",
            f"--window-size={px},{px}", f"--screenshot={png_path}", svg_path.as_uri(),
        ],
        check=True, capture_output=True,
    )


def main() -> int:
    if not CHROME.exists() or shutil.which("iconutil") is None:
        print("make_icon: needs Google Chrome and iconutil (macOS)", file=sys.stderr)
        return 1
    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)
        iconset = work / "Telescreen.iconset"
        iconset.mkdir()
        for name, px in ICONSET:
            source = work / f"{name}.svg"
            source.write_text(svg(px))
            render(source, iconset / f"{name}.png", px)
        OUTPUT.parent.mkdir(parents=True, exist_ok=True)
        subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(OUTPUT)], check=True)
    print(f"make_icon: wrote {OUTPUT.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
