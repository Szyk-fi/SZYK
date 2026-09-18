import sys, re

def parse_bdf(path):
    glyphs = {}  # codepoint -> list of int rows (raw hex value per row)
    ascent = None
    descent = None
    bbw = bbh = None
    with open(path) as f:
        lines = f.read().splitlines()
    i = 0
    cur_enc = None
    cur_bbx = None
    cur_rows = None
    in_bitmap = False
    while i < len(lines):
        line = lines[i]
        if line.startswith("FONT_ASCENT"):
            ascent = int(line.split()[1])
        elif line.startswith("FONT_DESCENT"):
            descent = int(line.split()[1])
        elif line.startswith("FONTBOUNDINGBOX"):
            parts = line.split()
            bbw, bbh = int(parts[1]), int(parts[2])
        elif line.startswith("STARTCHAR"):
            cur_enc = None
            cur_bbx = None
            cur_rows = []
        elif line.startswith("ENCODING"):
            cur_enc = int(line.split()[1])
        elif line.startswith("BBX"):
            parts = line.split()
            cur_bbx = tuple(int(x) for x in parts[1:5])
        elif line.startswith("BITMAP"):
            in_bitmap = True
        elif line.startswith("ENDCHAR"):
            in_bitmap = False
            if cur_enc is not None and cur_bbx is not None:
                glyphs[cur_enc] = (cur_bbx, cur_rows)
        elif in_bitmap:
            cur_rows.append(int(line.strip(), 16) if line.strip() else 0)
        i += 1
    return glyphs, ascent, descent, bbw, bbh

def build_raw(glyphs, bbw, bbh, char_w, char_h, lo, hi):
    n = hi - lo + 1
    image_width = n * char_w
    bytes_per_row = (image_width + 7) // 8
    out = bytearray(bytes_per_row * char_h)

    for gi, cp in enumerate(range(lo, hi + 1)):
        entry = glyphs.get(cp)
        if entry is None:
            continue
        (gbbw, gbbh, gxoff, gyoff), rows = entry
        # Assume glyph BBX matches font bounding box (true for Spleen's
        # fixed-width design -- every glyph fills the full cell).
        for r in range(min(len(rows), char_h)):
            row_val = rows[r]
            row_bytes = (gbbw + 7) // 8
            for b in range(char_w):
                bit_pos_in_glyph = row_bytes * 8 - 1 - b  # MSB-first within glyph's own byte span
                pixel = (row_val >> bit_pos_in_glyph) & 1 if b < gbbw else 0
                if pixel:
                    x = gi * char_w + b
                    byte_i = r * bytes_per_row + (x // 8)
                    bit_i = 7 - (x % 8)
                    out[byte_i] |= (1 << bit_i)
    return bytes(out), image_width, char_h

def main():
    path, char_w, char_h, out_raw = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    glyphs, ascent, descent, bbw, bbh = parse_bdf(path)
    lo, hi = 0x20, 0x7E
    data, w, h = build_raw(glyphs, bbw, bbh, char_w, char_h, lo, hi)
    with open(out_raw, "wb") as f:
        f.write(data)
    print(f"{out_raw}: {w}x{h} px, {len(data)} bytes, ascent={ascent} descent={descent} bbw={bbw} bbh={bbh}")

if __name__ == "__main__":
    main()
