#!/usr/bin/env python3
"""Independent scalar acceptance of current-CLI assembly GPU captures.

No Rust renderer/math calls and no receipt-derived oracle or thresholds. Run with
an absolute captures directory, or import check_gpu(path) from the acceptance gate.
"""
import hashlib
import json
import math
from pathlib import Path
import sys

SIZE = 384
EPSILON = 3
CONTROL_MIN = 80
POINTS = ((-.65, -.4), (.65, -.4), (-.25, .4))
COLORS = ((240, 60, 40), (50, 220, 80), (45, 90, 230), (220, 180, 35))
CONTROLS = ("missing_part", "wrong_material_offset", "wrong_image_offset", "double_source_transform", "reversed_order", "missing_mirror_handling", "ignored_placement", "wrong_normals")


def _fnv(data):
    value = 0xcbf29ce484222325
    for byte in data:
        value = ((value ^ byte) * 0x100000001b3) & ((1 << 64) - 1)
    return f"{value:016x}"


def _geometry(part, node):
    # Forward-transform explicit fixture vertices. This intentionally differs from
    # Rust's hand-composed inverse affine raster oracle.
    sy = -.7 if node == 0 else .7
    source = ((.7, -.25), (.3, .9)) if part == 1 else ((.9, .2), (0., .8))
    offset = (-.12, sy) if part == 1 else (.12, sy)
    placement = (((0., -.9), (.8, 0.)), ((-1., 0.), (0., .8)), ((0., 1.), (-.75, 0.)))[part]
    shift = ((-2.5, .2), (0., -.1), (2.5, .15))[part]
    result = []
    for point in POINTS:
        p = [sum(source[r][k] * point[k] for k in range(2)) + offset[r] for r in range(2)]
        result.append([sum(placement[r][k] * p[k] for k in range(2)) + shift[r] for r in range(2)])
    # Normal covector transforms sequentially, rather than using Rust's inverse
    # transpose of the independently hand-composed matrix.
    normal = [.36, .48, .8]
    for matrix, z in ((source, 1.3 if part == 1 else .6), (placement, (1.2, .7, .9)[part])):
        a, b = matrix[0]
        c, d = matrix[1]
        det = a * d - b * c
        normal = [(d * normal[0] - c * normal[1]) / det, (-b * normal[0] + a * normal[1]) / det, normal[2] / z]
    ny = normal[1] / math.sqrt(sum(v * v for v in normal))
    rgb = COLORS[(2 if part == 1 else 0) + node]
    factors = (.8, .6, .9) if node == 0 else (.5, .9, .7)
    color = []
    for encoded, factor, ground, sky in zip(rgb, factors, (.1, .2, .3), (.9, .7, .5)):
        srgb = encoded / 255.
        linear = srgb / 12.92 if srgb <= .04045 else ((srgb + .055) / 1.055) ** 2.4
        value = linear * factor * (ground + (sky - ground) * (ny * .5 + .5)) * .9
        encoded = value * 12.92 if value <= .0031308 else 1.055 * value ** (1 / 2.4) - .055
        color.append(math.floor(encoded * 255 + .5))
    return result, tuple(color + [255])


def _weights(p, triangle):
    a, b, c = triangle
    def cross(u, v):
        return u[0] * v[1] - u[1] * v[0]
    def sub(u, v):
        return (u[0] - v[0], u[1] - v[1])
    denominator = cross(sub(b, a), sub(c, a))
    return (cross(sub(b, p), sub(c, p)) / denominator,
            cross(sub(c, p), sub(a, p)) / denominator,
            cross(sub(a, p), sub(b, p)) / denominator)


def oracle():
    triangles = [_geometry(p, n) for p in range(3) for n in range(2)]
    counts = [0] * 6
    background = 0
    checked = []
    for y in range(SIZE):
        for x in range(SIZE):
            p = ((x + .5 - SIZE / 2) / 48., (SIZE / 2 - y - .5) / 48.)
            near, expected = False, None
            for index, (triangle, color) in enumerate(triangles):
                weights = _weights(p, triangle)
                near |= min(weights) >= -.08
                if min(weights) > .08:
                    assert expected is None, "fixture geometry overlaps"
                    expected = color
                    counts[index] += 1
            if expected is None and not near:
                expected = (0, 0, 0, 255)
                background += 1
            if expected is not None:
                checked.append(((y * SIZE + x) * 4, expected))
    assert counts == [363, 347, 390, 391, 390, 363] and background == 141514, (counts, background)
    return checked, counts, background


def _assert_frame(data, checked, label):
    assert len(data) == SIZE * SIZE * 4, label
    failures = [(offset // 4, tuple(data[offset:offset + 4]), expected)
                for offset, expected in checked
                if max(abs(a - b) for a, b in zip(data[offset:offset + 4], expected)) > EPSILON]
    assert not failures, (label, "independent geometry/color/normal mismatch", len(failures), failures[:4])


def check_gpu(path):
    path = Path(path)
    receipt = json.loads((path / "receipt.json").read_text())
    assert receipt["schema"] == 1 and receipt["status"] == "passed"
    assert (receipt["width"], receipt["height"]) == (SIZE, SIZE)
    adapter = json.loads((path / "adapter.json").read_text())
    assert adapter == receipt["adapter"]
    assert adapter["backend"] == "Vulkan" and adapter["software"] is True and adapter["name"]
    def records(value):
        if isinstance(value, dict):
            if "file" in value and "fnv1a64" in value:
                name = value["file"]
                assert Path(name).name == name
                data = (path / name).read_bytes()
                assert len(data) == value["bytes"] and _fnv(data) == value["fnv1a64"], name
            for child in value.values():
                records(child)
        elif isinstance(value, list):
            for child in value:
                records(child)
    records(receipt)
    assets = receipt["assets"]
    assert assets["source_hidden_after_install"] and assets["cooked_reload_before_render"]
    assert [assets[k] for k in ("installed_files", "parts", "unique_sources", "primitives", "materials", "images")] == [1, 3, 2, 6, 4, 4]
    sources = [json.loads((path / f"source-{i}.model.json").read_text()) for i in range(2)]
    cooked = json.loads((path / "assembly.model.json").read_text())
    assert sources[0]["asset_id"] == sources[1]["asset_id"] == "collision/source.glb"
    assert sources[0]["primitives"][0]["id"] == sources[1]["primitives"][0]["id"]
    assert cooked["images"] == sources[0]["images"] + sources[1]["images"]
    assert len(cooked["materials"]) == 4 and len(cooked["primitives"]) == 6
    for i in range(4):
        expected = dict(sources[i // 2]["materials"][i % 2])
        expected["image"] += (i // 2) * 2
        assert cooked["materials"][i] == expected
    assert len({p["id"] for p in cooked["primitives"]}) == 6
    for i, primitive in enumerate(cooked["primitives"]):
        source = 1 if i // 2 == 1 else 0
        old = sources[source]["primitives"][i % 2]
        assert primitive["material"] == source * 2 + i % 2
        assert primitive["vertices"] == old["vertices"] and primitive["indices"] == old["indices"]
    deps = {d["uri"]: d["sha256"] for d in cooked["dependencies"]}
    assert deps == {"$source": hashlib.sha256((path / "assembly.recipe.json").read_bytes()).hexdigest(),
                    **{f"inputs/source-{i}.model.json": hashlib.sha256((path / f"source-{i}.model.json").read_bytes()).hexdigest() for i in range(2)}}
    checked, counts, background = oracle()
    differences = {}
    for renderer in ("model", "imported_scene"):
        actual = (path / f"{renderer}.rgba").read_bytes()
        _assert_frame(actual, checked, renderer)
        report = receipt["coverage"][renderer]
        assert [v for pair in report["interior_per_part_material"] for v in pair] == counts
        assert report["background"] == background
        for control in CONTROLS:
            label = f"{renderer}_{control}"
            wrong = (path / f"{label}.rgba").read_bytes()
            assert len(wrong) == len(actual)
            diff = sum(max(abs(a - b) for a, b in zip(actual[i:i+4], wrong[i:i+4])) > EPSILON for i in range(0, len(actual), 4))
            assert diff > CONTROL_MIN, (label, diff)
            assert receipt["control_pixel_differences"][label] == diff
            try:
                _assert_frame(wrong, checked, label)
            except AssertionError:
                pass
            else:
                raise AssertionError((label, "negative control passed independent oracle"))
            differences[label] = diff
    return {"status": "passed", "adapter": adapter, "interior_per_part_material": counts,
            "background": background, "independent_checked_pixels": len(checked), "control_pixel_differences": differences}


if __name__ == "__main__":
    print(json.dumps(check_gpu(sys.argv[1]), indent=2))
