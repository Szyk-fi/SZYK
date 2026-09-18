"""One-time conversion: szyk_logo.svg -> ../raw/szyk_logo.raw.

Same pipeline as convert_logo.py (MX1) -- see its docstring for the
full explanation of why this is a pre-thresholded 1bpp raw bitmap
instead of an SVG rendered live. Re-run if the source artwork or
target size changes:
    pip install resvg_py Pillow
    python3 convert_szyk_logo.py
"""

import resvg_py
from PIL import Image

TARGET_WIDTH = 400  # resvg preserves aspect ratio -- actual output may be a pixel or two narrower


def main():
    data = resvg_py.svg_to_bytes(svg_path="szyk_logo.svg", width=TARGET_WIDTH)
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

    with open("../raw/szyk_logo.raw", "wb") as f:
        f.write(out)
    print(f"wrote ../raw/szyk_logo.raw: {w}x{h}, {len(out)} bytes (update SZYK's width/height in src/startup_logo.rs to match)")


if __name__ == "__main__":
    main()
