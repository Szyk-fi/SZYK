"""One-time conversion: mx1_logo_white.svg -> ../raw/mx1_logo.raw.

Renders the vector logo with resvg (`pip install resvg_py`) at a much
higher resolution than the target (supersampling), downsamples with a
proper box filter, *then* thresholds on alpha (>127 = lit) and packs
it 1bpp, MSB-first per row -- the same raw-bitmap convention
src/spleen_fonts.rs's fonts use, for the same reason: this has to blit
on real firmware exactly the way it does here, no runtime SVG
rasterizer or alpha blending in the loop on an STM32N6.

The supersample-then-threshold step matters: resvg already
anti-aliases at whatever resolution it renders, but thresholding
straight off a render *at* the target size throws that gradient
information away per-pixel, leaving visibly jagged diagonals/curves at
1bpp. Rendering at SUPERSAMPLE_FACTOR x the target and box-filtering
down first lets each final pixel's lit/unlit call be based on real
coverage (how much of that pixel's area the glyph actually fills)
instead of one sample point -- the standard supersample-then-threshold
technique for sharp binary output, same idea `spleen_fonts.rs`'s own
hand-drawn 1bpp glyphs get "for free" from being authored at their
exact target resolution to begin with.

Re-run this if the source artwork or target size changes:
    pip install resvg_py Pillow
    python3 convert_logo.py
"""

import resvg_py
from PIL import Image

TARGET_WIDTH = 400  # resvg preserves aspect ratio -- actual output may be a pixel or two narrower
SUPERSAMPLE_FACTOR = 4


def main():
    data = resvg_py.svg_to_bytes(svg_path="mx1_logo_white.svg", width=TARGET_WIDTH * SUPERSAMPLE_FACTOR)
    hi_res = Image.open(__import__("io").BytesIO(bytes(data))).convert("RGBA")
    hi_w, hi_h = hi_res.size
    target_h = round(hi_h / SUPERSAMPLE_FACTOR)
    img = hi_res.resize((TARGET_WIDTH, target_h), Image.Resampling.BOX)
    w, h = img.size
    px = img.load()

    bytes_per_row = (w + 7) // 8
    out = bytearray(bytes_per_row * h)
    for y in range(h):
        for x in range(w):
            _, _, _, a = px[x, y]
            if a > 127:
                byte_i = y * bytes_per_row + (x // 8)
                bit_i = 7 - (x % 8)
                out[byte_i] |= 1 << bit_i

    with open("../raw/mx1_logo.raw", "wb") as f:
        f.write(out)
    print(f"wrote ../raw/mx1_logo.raw: {w}x{h}, {len(out)} bytes (update LOGO_WIDTH/LOGO_HEIGHT in src/startup_logo.rs to match)")


if __name__ == "__main__":
    main()
