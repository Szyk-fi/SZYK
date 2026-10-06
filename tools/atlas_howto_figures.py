#!/usr/bin/env python3
"""Turns the raw Atlas screenshots into the figures docs/atlas-howto uses.

    cargo run --example slint_home_live -- --render-atlas-howto /tmp/atlas_howto
    PORTAMAX_RENDER_APP=Atlas cargo run --example slint_home_live -- --render-instruments /tmp/atlas_home
    python3 tools/atlas_howto_figures.py /tmp/atlas_howto /tmp/atlas_home docs/atlas-howto

Each raw shot is the whole 1240x560 device frame. The guide mostly wants the
640x360 screen (that is where the text is), so every shot is also cropped to
it, and two figures get numbered callouts drawn on: the control map (frame)
and the Play view (screen). Callout positions are in the raw frame's pixels.
"""
import sys
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

SCREEN = (208, 92, 848, 452)  # the 640x360 display inside the 1240x560 frame
INK = (20, 24, 31)
CALLOUT = (255, 184, 48)
RING = (255, 255, 255)


def font(size):
    for path in ("/System/Library/Fonts/Helvetica.ttc", "/System/Library/Fonts/SFNS.ttf", "/Library/Fonts/Arial.ttf"):
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default()


def callout(draw, xy, label, r=15, size=19):
    x, y = xy
    draw.ellipse((x - r - 2, y - r - 2, x + r + 2, y + r + 2), fill=RING)
    draw.ellipse((x - r, y - r, x + r, y + r), fill=CALLOUT)
    f = font(size)
    w = draw.textlength(str(label), font=f)
    draw.text((x - w / 2, y - size / 2 - 2), str(label), fill=INK, font=f)


def main(shots, home, out):
    shots, home, out = Path(shots), Path(home), Path(out)
    out.mkdir(parents=True, exist_ok=True)

    # The screen of every shot, as-is.
    for p in sorted(shots.glob("*.png")):
        Image.open(p).convert("RGB").crop(SCREEN).save(out / f"screen-{p.name}")
    if (home / "home-instruments-console.png").exists():
        Image.open(home / "home-instruments-console.png").convert("RGB").crop(SCREEN).save(out / "screen-00-home.png")

    # Control map: numbered callouts on the whole device frame.
    frame = Image.open(shots / "01-play.png").convert("RGB")
    d = ImageDraw.Draw(frame)
    for label, xy in {
        1: (74, 38),     # L1
        2: (1166, 38),   # R1
        3: (56, 78),     # left hand sensor
        4: (1084, 78),   # right hand sensor
        5: (116, 146),   # D-pad (the four arrows)
        6: (116, 208),   # SELECT (centre of the D-pad)
        7: (116, 380),   # joystick
        8: (532, 100),   # screen
        9: (1035, 272),  # pads
        10: (186, 488),  # F1-F4
    }.items():
        callout(d, xy, label)
    frame.save(out / "fig-controls.png")

    # Play view: lettered callouts on the screen.
    screen = Image.open(out / "screen-01-play.png").convert("RGB")
    s = ImageDraw.Draw(screen)
    for label, xy in {
        "A": (9, 52),      # layer badge
        "B": (310, 51),    # hints
        "C": (9, 100),     # dials
        "D": (9, 190),     # pad map
        "E": (9, 270),     # stick / hands / status
        "F": (629, 89),    # preset name + category
        "G": (629, 135),   # macro bars
        "H": (629, 202),   # MORPH
        "I": (629, 238),   # spectrum
        "J": (629, 266),   # voices / cpu
        "K": (9, 341),     # F-button bar
    }.items():
        callout(s, xy, label, r=11, size=14)
    screen.save(out / "fig-playview.png")
    print("wrote", len(list(out.glob("*.png"))), "figures to", out)


if __name__ == "__main__":
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    main(*sys.argv[1:])
