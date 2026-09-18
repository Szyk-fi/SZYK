import sys, zlib, struct

def write_png(path, width, height, gray_rows):
    def chunk(tag, data):
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xffffffff)
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0)  # grayscale, 8-bit
    raw = bytearray()
    for row in gray_rows:
        raw.append(0)  # filter type: none
        raw.extend(row)
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig)
        f.write(chunk(b"IHDR", ihdr))
        f.write(chunk(b"IDAT", idat))
        f.write(chunk(b"IEND", b""))

def main():
    raw_path, char_w, char_h, out_png = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
    with open(raw_path, "rb") as f:
        data = f.read()
    bytes_per_row = len(data) // char_h
    width = bytes_per_row * 8
    rows = []
    for r in range(char_h):
        row_bytes = data[r*bytes_per_row:(r+1)*bytes_per_row]
        gray = bytearray(width)
        for byte_i, byte in enumerate(row_bytes):
            for bit in range(8):
                x = byte_i * 8 + bit
                pixel = (byte >> (7 - bit)) & 1
                gray[x] = 255 if pixel else 0
        rows.append(gray)
    write_png(out_png, width, char_h, rows)
    print(f"wrote {out_png}: {width}x{char_h}")

if __name__ == "__main__":
    main()
