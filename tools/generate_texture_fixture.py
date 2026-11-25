#!/usr/bin/env python3
"""Generate an embedded-texture GLB for CYM import and slicer acceptance."""

import argparse
import json
from pathlib import Path
import struct
import zlib


PALETTE = (
    (238, 223, 192), (215, 60, 38), (249, 154, 39), (246, 211, 91),
    (166, 104, 67), (130, 63, 99), (71, 97, 137), (25, 58, 95),
    (19, 122, 138), (124, 182, 162), (46, 143, 83), (147, 180, 64),
    (191, 221, 227), (237, 151, 168), (85, 78, 171), (46, 50, 54),
)


def png_chunk(kind, payload):
    return (struct.pack(">I", len(payload)) + kind + payload
            + struct.pack(">I", zlib.crc32(kind + payload)))


def atlas_png():
    # One distinguishable color in each of the 16 tiles. A textured face
    # samples the shared tiles in a different order at each UV seam.
    width = 64
    pixels = bytearray()
    for y in range(width):
        pixels.append(0)  # PNG scanline filter: none
        for x in range(width):
            pixels.extend(PALETTE[(y // 16) * 4 + x // 16])
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">2I5B", width, width, 8, 2, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(bytes(pixels)))
            + png_chunk(b"IEND", b""))


def make_glb(missing_uv=False, sides_by_slot=False):
    # Axis-aligned cuboid with different side lengths. glTF coordinates
    # are metre/Y-up. Each side has its own UV corners to exercise seams.
    x, y, z = 0.020, 0.024, 0.016
    corners = [(0, 0, 0), (x, 0, 0), (x, y, 0), (0, y, 0),
               (0, 0, z), (x, 0, z), (x, y, z), (0, y, z)]
    quads = [(3, 2, 1, 0), (4, 5, 6, 7), (0, 1, 5, 4),
             (7, 6, 2, 3), (4, 7, 3, 0), (1, 2, 6, 5)]
    uv = [(0, 0), (1, 0), (1, 1), (0, 1)]
    vertices, uvs, indices = [], [], []
    side_rows = (0, 1, 0, 0, 2, 3)
    for side, quad in enumerate(quads):
        start = len(vertices)
        vertices.extend(corners[corner] for corner in quad)
        if sides_by_slot:
            # Four source colours per wall, all from the same atlas row.
            # Dyadic subdivision follows the quarter-width colour boundaries.
            row = side_rows[side]
            uvs.extend((u, row / 4 + .0625 + v / 8) for u, v in uv)
        else:
            uvs.extend(uv[(corner + side) % 4] for corner in range(4))
        indices.extend(start + local for local in (0, 1, 2, 0, 2, 3))

    binary = bytearray()
    views = []

    def append(chunk, target=None):
        binary.extend(b"\0" * (-len(binary) % 4))
        views.append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(chunk)})
        if target:
            views[-1]["target"] = target
        binary.extend(chunk)
        return len(views) - 1

    positions_view = append(b"".join(struct.pack("<3f", *p) for p in vertices), 34962)
    uvs_view = append(b"".join(struct.pack("<2f", *point) for point in uvs), 34962)
    indices_view = append(struct.pack("<" + "H" * len(indices), *indices), 34963)
    image_view = append(atlas_png())
    attrs = {"POSITION": 0}
    if not missing_uv:
        attrs["TEXCOORD_0"] = 1
    document = {
        "asset": {"version": "2.0", "generator": "CYM texture acceptance fixture"},
        "scene": 0, "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0, "translation": [0.002, 0, 0.003]}],
        "meshes": [{"primitives": [{"attributes": attrs, "indices": 2, "material": 0}]}],
        "materials": [{"pbrMetallicRoughness": {"baseColorTexture": {"index": 0}}}],
        "samplers": [{"wrapS": 33071, "wrapT": 33071,
                      **({"magFilter": 9728, "minFilter": 9728} if sides_by_slot else {})}],
        "textures": [{"sampler": 0, "source": 0}],
        "images": [{"bufferView": image_view, "mimeType": "image/png"}],
        "buffers": [{"byteLength": len(binary)}], "bufferViews": views,
        "accessors": [
            {"bufferView": positions_view, "componentType": 5126, "count": len(vertices),
             "type": "VEC3", "min": [0, 0, 0], "max": [x, y, z]},
            {"bufferView": uvs_view, "componentType": 5126, "count": len(uvs), "type": "VEC2"},
            {"bufferView": indices_view, "componentType": 5123, "count": len(indices), "type": "SCALAR"},
        ],
    }
    metadata = json.dumps(document, separators=(",", ":")).encode()
    metadata += b" " * (-len(metadata) % 4)
    binary.extend(b"\0" * (-len(binary) % 4))
    chunks = (struct.pack("<I4s", len(metadata), b"JSON") + metadata
              + struct.pack("<I4s", len(binary), b"BIN\0") + binary)
    return struct.pack("<4sII", b"glTF", 2, 12 + len(chunks)) + chunks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--missing-uv", action="store_true")
    parser.add_argument("--sides-by-slot", action="store_true")
    args = parser.parse_args()
    args.output.write_bytes(make_glb(args.missing_uv, args.sides_by_slot))


if __name__ == "__main__":
    main()
