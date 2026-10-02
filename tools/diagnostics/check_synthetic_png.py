"""Decode this task's synthetic PNGs; retain text-only integrity/color results."""
import hashlib
import json
from pathlib import Path
import struct
import sys
import zlib


def check(path):
    data = Path(path).read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Invalid PNG signature")
    offset, compressed, dimensions = 8, bytearray(), None
    while offset < len(data):
        length = struct.unpack(">I", data[offset:offset+4])[0]
        kind = data[offset+4:offset+8]
        content = data[offset+8:offset+8+length]
        crc = struct.unpack(">I", data[offset+8+length:offset+12+length])[0]
        if zlib.crc32(kind + content) != crc:
            raise ValueError("PNG chunk CRC mismatch")
        if kind == b"IHDR":
            width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", content)
            if (depth, color, compression, filtering, interlace) != (8, 2, 0, 0, 0):
                raise ValueError("Unexpected synthetic image format")
            dimensions = width, height
        if kind == b"IDAT":
            compressed.extend(content)
        offset += length + 12
        if kind == b"IEND":
            break
    if offset != len(data) or dimensions is None:
        raise ValueError("Unexpected trailing data or missing header")
    width, height = dimensions
    raw = zlib.decompress(compressed)
    stride = width * 3 + 1
    if len(raw) != height * stride or any(raw[y*stride] != 0 for y in range(height)):
        raise ValueError("Invalid unfiltered scanline byte count")
    colors = []
    for x, y, expected in [
        (width//4, height//4, (240, 20, 20)),
        (3*width//4, height//4, (20, 200, 20)),
        (width//4, 3*height//4, (20, 20, 240)),
        (3*width//4, 3*height//4, (240, 220, 20)),
    ]:
        start = y*stride + 1 + x*3
        actual = tuple(raw[start:start+3])
        if actual != expected:
            raise ValueError("Synthetic quadrant pixel mismatch")
        colors.append(list(actual))
    return {"file": Path(path).name, "width": width, "height": height,
            "sha256": hashlib.sha256(data).hexdigest(), "bytes": len(data),
            "png_crc_and_zlib_verified": True, "quadrant_colors": colors}


if __name__ == "__main__":
    print(json.dumps([check(path) for path in sys.argv[1:]], indent=2))
