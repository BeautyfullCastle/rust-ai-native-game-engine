#!/usr/bin/env python3
"""Author the original CC0 imported-scene fixtures (Python standard library).

This is an offline authoring tool, never run by the demo or package installer.
The viewer imports the checked-in GLBs from installed immutable package objects.
"""
import json
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parent


def png_chunk(kind, payload):
    return (struct.pack(">I", len(payload)) + kind + payload
            + struct.pack(">I", zlib.crc32(kind + payload) & 0xffffffff))


def texture(colors):
    # An original 8x8 grid makes orientation and texture sampling visible.
    raw = b"".join(b"\0" + bytes(c for x in range(8)
                    for c in colors[0 if x == 0 or y == 0 else 1 + (x // 4 + y // 4) % 2])
                   for y in range(8))
    return (b"\x89PNG\r\n\x1a\n"
            + png_chunk(b"IHDR", struct.pack(">IIBBBBB", 8, 8, 8, 6, 0, 0, 0))
            + png_chunk(b"IDAT", zlib.compress(raw, 9)) + png_chunk(b"IEND", b""))


def write_model(name, boxes, colors):
    blob, views, accessors = bytearray(), [], []

    def view(data, target=None):
        blob.extend(b"\0" * ((-len(blob)) % 4))
        result = len(views)
        entry = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
        if target:
            entry["target"] = target
        views.append(entry)
        blob.extend(data)
        return result

    def accessor(values, component, kind, target, bounds=False):
        flat = [n for value in values for n in (value if isinstance(value, tuple) else [value])]
        fmt = "f" if component == 5126 else "H"
        item = {"bufferView": view(struct.pack("<" + fmt * len(flat), *flat), target),
                "componentType": component, "count": len(values), "type": kind}
        if bounds:
            item["min"] = [min(p[i] for p in values) for i in range(3)]
            item["max"] = [max(p[i] for p in values) for i in range(3)]
        accessors.append(item)
        return len(accessors) - 1

    positions, normals, uvs, indices = [], [], [], []
    for lo, hi in boxes:
        x0, y0, z0 = lo
        x1, y1, z1 = hi
        faces = [
            ((0, 0, 1), [(x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)]),
            ((0, 0, -1), [(x1, y0, z0), (x0, y0, z0), (x0, y1, z0), (x1, y1, z0)]),
            ((1, 0, 0), [(x1, y0, z1), (x1, y0, z0), (x1, y1, z0), (x1, y1, z1)]),
            ((-1, 0, 0), [(x0, y0, z0), (x0, y0, z1), (x0, y1, z1), (x0, y1, z0)]),
            ((0, 1, 0), [(x0, y1, z1), (x1, y1, z1), (x1, y1, z0), (x0, y1, z0)]),
            ((0, -1, 0), [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1)]),
        ]
        for normal, points in faces:
            first = len(positions)
            positions.extend(points)
            normals.extend([normal] * 4)
            uvs.extend([(0, 1), (1, 1), (1, 0), (0, 0)])
            indices.extend(first + i for i in (0, 1, 2, 0, 2, 3))
    pos = accessor(positions, 5126, "VEC3", 34962, True)
    normal = accessor(normals, 5126, "VEC3", 34962)
    uv = accessor(uvs, 5126, "VEC2", 34962)
    index = accessor(indices, 5123, "SCALAR", 34963)
    image = view(texture(colors))
    source = {
        "asset": {"version": "2.0", "generator": "Orrery imported_scene_demo/generate.py",
                  "copyright": "CC0-1.0; original Orrery imported-scene fixture"},
        "buffers": [{"byteLength": len(blob)}], "bufferViews": views, "accessors": accessors,
        "images": [{"bufferView": image, "mimeType": "image/png"}],
        "samplers": [{"minFilter": 9728, "magFilter": 9728, "wrapS": 33071, "wrapT": 33071}],
        "textures": [{"source": 0, "sampler": 0}],
        "materials": [{"name": name + " original grid", "pbrMetallicRoughness": {
            "baseColorTexture": {"index": 0}, "metallicFactor": 0, "roughnessFactor": 1}}],
        "meshes": [{"name": name, "primitives": [{"attributes": {
            "POSITION": pos, "NORMAL": normal, "TEXCOORD_0": uv}, "indices": index, "material": 0}]}],
        "nodes": [{"name": name, "mesh": 0}], "scenes": [{"nodes": [0]}], "scene": 0,
    }
    encoded = json.dumps(source, separators=(",", ":")).encode()
    encoded += b" " * ((-len(encoded)) % 4)
    blob.extend(b"\0" * ((-len(blob)) % 4))
    result = struct.pack("<III", 0x46546c67, 2, 28 + len(encoded) + len(blob))
    result += struct.pack("<II", len(encoded), 0x4e4f534a) + encoded
    result += struct.pack("<II", len(blob), 0x004e4942) + blob
    (ROOT / (name + ".glb")).write_bytes(result)
    print(f"{name}.glb: {len(result)} bytes, {len(positions)} vertices, {len(indices) // 3} triangles")


write_model("background", [((-3.0, -0.25, -1.1), (3.0, 3.7, -0.8)),
                           ((-3.0, -0.45, -1.1), (3.0, -0.25, 1.4))],
            [(65, 73, 94, 255), (143, 155, 178, 255), (164, 174, 195, 255)])
write_model("foreground", [((-1.3, 0.45, 0.4), (1.05, 0.95, 0.9)),
                           ((-1.65, -0.25, 0.25), (-1.1, 1.75, 0.85))],
            [(19, 87, 83, 255), (77, 176, 157, 255), (126, 212, 180, 255)])
