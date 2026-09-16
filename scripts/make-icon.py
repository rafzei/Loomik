"""Create the app's simple record-button icon using Python's standard library."""
import pathlib
import struct
import zlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
ICONSET = ROOT / "target" / "AppIcon.iconset"
ICONSET.mkdir(parents=True, exist_ok=True)


def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def render(size):
    rows = bytearray()
    for y in range(size):
        rows.append(0)
        for x in range(size):
            px, py = (x + 0.5) / size, (y + 0.5) / size
            qx, qy = max(abs(px - 0.5) - 0.28, 0), max(abs(py - 0.5) - 0.28, 0)
            edge = (qx * qx + qy * qy) ** 0.5 - 0.17
            alpha = round(max(0, min(1, 0.5 - edge * size)) * 255)
            distance = ((px - 0.5) ** 2 + (py - 0.5) ** 2) ** 0.5
            ring = max(0, min(1, (0.018 - abs(distance - 0.278)) * size + 0.5))
            dot = max(0, min(1, (0.178 - distance) * size + 0.5))
            orange = max(ring, dot)
            rows.extend([round(38 * (1 - orange) + 239 * orange),
                         round(39 * (1 - orange) + 75 * orange),
                         round(43 * (1 - orange) + 43 * orange), alpha])
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b""))


for logical in (16, 32, 128, 256, 512):
    for factor in (1, 2):
        suffix = "@2x" if factor == 2 else ""
        (ICONSET / f"icon_{logical}x{logical}{suffix}.png").write_bytes(render(logical * factor))
# Modern macOS ICNS stores PNG representations in typed blocks.
blocks = bytearray()
for kind, size in ((b"icp4", 16), (b"icp5", 32), (b"icp6", 64),
                   (b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)):
    png = render(size)
    blocks.extend(kind + struct.pack(">I", len(png) + 8) + png)
(ROOT / "packaging" / "AppIcon.icns").write_bytes(b"icns" + struct.pack(">I", len(blocks) + 8) + blocks)
