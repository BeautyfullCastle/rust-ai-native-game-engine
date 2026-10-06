#!/usr/bin/env python3
"""Original CC0 fixture authoring script. Uses only Python's standard library.

No downloaded model, copied asset, image-generation service or third-party
geometry is used. glTF and GLB carry identical geometry, rig, clips and PNG.
"""
import base64
import json
import math
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parent
blob = bytearray()
views = []
accessors = []


def view(data, target=None):
    while len(blob) % 4:
        blob.append(0)
    i = len(views)
    entry = {"buffer": 0, "byteOffset": len(blob), "byteLength": len(data)}
    if target is not None:
        entry["target"] = target
    views.append(entry)
    blob.extend(data)
    return i


def accessor(values, component, kind, target=None, minimum=None, maximum=None):
    fmt = {5121: "B", 5123: "H", 5126: "f"}[component]
    flat = [number for value in values for number in (value if isinstance(value, (tuple, list)) else [value])]
    entry = {"bufferView": view(struct.pack("<" + fmt * len(flat), *flat), target),
             "componentType": component, "count": len(values), "type": kind}
    if minimum is not None:
        entry["min"] = minimum
        entry["max"] = maximum
    accessors.append(entry)
    return len(accessors) - 1


positions = [(-.28, 0, 0), (.42, 0, 0), (-.22, 1, 0), (.34, 1, 0),
             (-.35, 2, 0), (.27, 2, 0), (-.18, 3, 0), (.46, 3, 0)]
pos = accessor(positions, 5126, "VEC3", 34962, [-.35, 0, 0], [.46, 3, 0])
normal = accessor([(0, 0, 1)] * 8, 5126, "VEC3", 34962)
uv = accessor([(side, 1 - row / 3) for row in range(4) for side in (0, 1)], 5126, "VEC2", 34962)
joints = accessor([(0, 1, 0, 0)] * 8, 5121, "VEC4", 34962)
weights = accessor([weight for pair in [(1, 0), (.8, .2), (.3, .7), (0, 1)]
                    for weight in [(*pair, 0, 0)] * 2], 5126, "VEC4", 34962)
triangles = [v for row in range(3) for v in [row*2, row*2+1, row*2+2, row*2+1, row*2+3, row*2+2]]
indices = accessor(triangles, 5123, "SCALAR", 34963)


def inverse_translation(x, y):
    return (1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, -x, -y, 0, 1)


inverse_bind = accessor([inverse_translation(.3, .2), inverse_translation(.3, 1.7)], 5126, "MAT4")
times = accessor([0, 1, 2], 5126, "SCALAR", minimum=[0], maximum=[2])
rotations = accessor([(0, 0, 0, 1), (0, 0, .5, math.sqrt(.75)), (0, 0, 0, 1)], 5126, "VEC4")
pulse_times = accessor([0, .5, 1], 5126, "SCALAR", minimum=[0], maximum=[1])
translations = accessor([(.3, .2, 0), (.55, .4, 0), (.3, .2, 0)], 5126, "VEC3")
scales = accessor([(1, 1, 1), (1.15, .8, 1), (1, 1, 1)], 5126, "VEC3")


def chunk(kind, payload):
    return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xffffffff)


# Original 4x4 asymmetrical orange/cyan/violet/cream checker, opaque RGBA8.
colors = [(245, 113, 40, 255), (35, 190, 213, 255), (133, 85, 219, 255), (250, 231, 166, 255)]
pattern = [[0, 0, 1, 1], [0, 3, 1, 2], [2, 2, 3, 3], [2, 1, 3, 0]]
raw = b"".join(b"\0" + bytes(c for index in row for c in colors[index]) for row in pattern)
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
image_view = view(png)
source = {
    "asset": {"version": "2.0", "generator": "Orrery original animated_strip_generate.py", "copyright": "CC0-1.0; original Orrery test fixture"},
    "buffers": [{"byteLength": len(blob)}], "bufferViews": views, "accessors": accessors,
    "images": [{"bufferView": image_view, "mimeType": "image/png"}],
    "samplers": [{"minFilter": 9728, "magFilter": 9728, "wrapS": 33071, "wrapT": 33071}],
    "textures": [{"source": 0, "sampler": 0}],
    "materials": [{"name": "original asymmetric checker", "pbrMetallicRoughness": {"baseColorTexture": {"index": 0}, "metallicFactor": 0, "roughnessFactor": 1}}],
    "meshes": [{"name": "asymmetric strip", "primitives": [{"attributes": {"POSITION": pos, "NORMAL": normal, "TEXCOORD_0": uv, "JOINTS_0": joints, "WEIGHTS_0": weights}, "indices": indices, "material": 0}]}],
    "nodes": [
        {"name": "transformed nonjoint ancestor", "translation": [.3, .2, 0], "children": [1]},
        {"name": "lower joint", "children": [2]},
        {"name": "upper joint", "translation": [0, 1.5, 0]},
        {"name": "mesh transform deliberately ignored by skinning", "mesh": 0, "skin": 0, "translation": [7, 0, 0]},
    ],
    "skins": [{"name": "two-joint original rig", "joints": [1, 2], "skeleton": 0, "inverseBindMatrices": inverse_bind}],
    "animations": [
        {"name": "bend", "samplers": [{"input": times, "output": rotations, "interpolation": "LINEAR"}],
         "channels": [{"sampler": 0, "target": {"node": 2, "path": "rotation"}}]},
        {"name": "pulse", "samplers": [{"input": pulse_times, "output": translations, "interpolation": "STEP"}, {"input": pulse_times, "output": scales, "interpolation": "LINEAR"}],
         "channels": [{"sampler": 0, "target": {"node": 0, "path": "translation"}}, {"sampler": 1, "target": {"node": 1, "path": "scale"}}]},
    ], "scenes": [{"nodes": [0, 3]}], "scene": 0,
}
json_bytes = json.dumps(source, separators=(",", ":"), ensure_ascii=True).encode()
json_bytes += b" " * ((-len(json_bytes)) % 4)
bin_bytes = bytes(blob) + b"\0" * ((-len(blob)) % 4)
glb = struct.pack("<III", 0x46546c67, 2, 12 + 8 + len(json_bytes) + 8 + len(bin_bytes))
glb += struct.pack("<II", len(json_bytes), 0x4e4f534a) + json_bytes

glb += struct.pack("<II", len(bin_bytes), 0x004e4942) + bin_bytes
(ROOT / "animated_strip.glb").write_bytes(glb)
source["buffers"][0]["uri"] = "data:application/octet-stream;base64," + base64.b64encode(blob).decode()
(ROOT / "animated_strip.gltf").write_text(json.dumps(source, indent=2) + "\n")
print(f"Wrote animated_strip.gltf and animated_strip.glb ({len(glb)} GLB bytes)")
