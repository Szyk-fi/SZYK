"""One-time conversion: mx1_logo_white.svg -> ../raw/mx1_logo.raw.

Renders the vector logo at a fixed target size with resvg (`pip install
resvg_py`), then thresholds on alpha (>127 = lit) and packs it 1bpp,
MSB-first per row -- the same raw-bitmap convention
src/spleen_fonts.rs's fonts use, for the same reason: this has to blit
on real firmware exactly the way it does here, no runtime SVG
rasterizer or alpha blending in the loop on an STM32N6.

Re-run this if the source artwork or target size changes:
    pip install resvg_py Pillow
    python3 convert_logo.py
"""

import resvg_py
from PIL import Image

TARGET_WIDTH = 400  # resvg preserves aspect ratio -- actual output may be a pixel or two narrower


def main():
    data = resvg_py.svg_to_bytes(svg_path="mx1_logo_white.svg", width=TARGET_WIDTH)
    img = Image.open(__import__("io").BytesIO(bytes(data))).convert("RGBA")
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
