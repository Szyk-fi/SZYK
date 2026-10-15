"""Generates the black-and-white SZYK and ROVE logos.

Letterforms are drawn here from simple geometry (a 60x60 cell, 14-unit
stems) instead of a font file so the SVG is self-contained and the 1bpp
boot bitmaps can be re-made without any font on the machine.

Outputs (run from this directory):
    szyk_logo.svg, rove_logo.svg                 boxed, used by the Slint UI
    szyk-console-wordmark.svg, rove-console-wordmark.svg   plain letters
    ../raw/szyk_logo.raw, ../raw/rove_logo.raw   400 px wide 1bpp boot logos
    pip install resvg_py Pillow
    python3 make_logos.py
"""

import io
import resvg_py
from PIL import Image

# Each glyph is a list of SVG shapes in a 60x60 cell, drawn filled.
GLYPHS = {
    "S": ['<path d="M0 0H60V14H14V23H60V60H0V46H46V37H0Z"/>'],
    "Z": ['<path d="M0 0H60V14L26 46H60V60H0V46L34 14H0Z"/>'],
    "Y": ['<path d="M0 0H17L30 22L43 0H60L37 36V60H23V36Z"/>'],
    "K": ['<path d="M0 0H14V22L40 0H60L30 28L60 60H41L18 36L14 40V60H0Z"/>'],
    "R": ['<path fill-rule="evenodd" d="M0 0H48Q60 0 60 12V26Q60 36 50 38L60 60H43L34 40H14V60H0ZM14 14V26H44Q46 26 46 24V16Q46 14 44 14Z"/>'],
    "O": ['<path fill-rule="evenodd" d="M12 0H48Q60 0 60 12V48Q60 60 48 60H12Q0 60 0 48V12Q0 0 12 0ZM16 14Q14 14 14 16V44Q14 46 16 46H44Q46 46 46 44V16Q46 14 44 14Z"/>'],
    "V": ['<path d="M0 0H17L30 38L43 0H60L40 60H20Z"/>'],
    "E": ['<path d="M0 0H60V14H14V23H52V37H14V46H60V60H0Z"/>'],
}

CELL, BOX, GAP, R = 60, 90, 8, 15


def wordmark(word, spacing=26):
    w = len(word) * CELL + (len(word) - 1) * spacing
    parts = []
    for i, ch in enumerate(word):
        parts.append(f'<g transform="translate({i * (CELL + spacing)} 0)">{"".join(GLYPHS[ch])}</g>')
    return w, CELL, "".join(parts)


def boxed(word):
    """Every letter in the same solid box, letter knocked out, in one ink."""
    w = len(word) * BOX + (len(word) - 1) * GAP
    off = (BOX - CELL) / 2
    parts, masks = [], []
    for i, ch in enumerate(word):
        x = i * (BOX + GAP)
        glyph = f'<g transform="translate({x + off} {off})">{"".join(GLYPHS[ch])}</g>'
        masks.append(f'<mask id="m{i}" maskUnits="userSpaceOnUse" x="{x}" y="0" width="{BOX}" height="{BOX}">'
                     f'<rect x="{x}" y="0" width="{BOX}" height="{BOX}" fill="#fff"/>'
                     f'<g fill="#000">{glyph}</g></mask>')
        parts.append(f'<rect x="{x}" y="0" width="{BOX}" height="{BOX}" rx="{R}" mask="url(#m{i})"/>')
    return w, BOX, "<defs>" + "".join(masks) + "</defs>" + "".join(parts)


def svg(w, h, body, pad=0):
    return (f'<?xml version="1.0" encoding="UTF-8"?>\n'
            f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="{-pad} {-pad} {w + 2 * pad} {h + 2 * pad}">'
            f'<g fill="#000">{body}</g></svg>\n')


def to_raw(svg_text, path, target_w=400, ss=4):
    data = resvg_py.svg_to_bytes(svg_string=svg_text, width=target_w * ss)
    hi = Image.open(io.BytesIO(bytes(data))).convert("RGBA")
    h = round(hi.size[1] / ss)
    img = hi.resize((target_w, h), Image.Resampling.BOX)
    px, bpr = img.load(), (target_w + 7) // 8
    out = bytearray(bpr * h)
    for y in range(h):
        for x in range(target_w):
            if px[x, y][3] > 127:
                out[y * bpr + x // 8] |= 1 << (7 - x % 8)
    open(path, "wb").write(out)
    print(f"wrote {path}: {target_w}x{h}, {len(out)} bytes")


for word, name in (("SZYK", "szyk"), ("ROVE", "rove")):
    s = svg(*boxed(word))
    open(f"{name}_logo.svg", "w").write(s)
    to_raw(s, f"../raw/{name}_logo.raw")
    open(f"{'szyk-console' if name == 'szyk' else 'rove-console'}-wordmark.svg", "w").write(svg(*wordmark(word)))
    # preview: white-on-black, what the boot screen shows
    prev = resvg_py.svg_to_bytes(svg_string=s.replace('fill="#000"', 'fill="#fff"').replace('stroke="#000"', 'stroke="#fff"').replace('<mask', '<mask').replace('fill="#fff"/><g fill="#fff">', 'fill="#fff"/><g fill="#000">'), width=800, background="#000000")
    open(f"/tmp/_prev_{name}.png", "wb").write(bytes(prev))
