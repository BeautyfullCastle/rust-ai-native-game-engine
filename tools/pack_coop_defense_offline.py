#!/usr/bin/env python3
"""Embed this optional adapter's exact engine/art and closed ES-module graph.

This is intentionally not a general JavaScript bundler. New imports require an
explicit graph update and regression tests, rather than silently shipping an
HTML file that still needs sibling modules.
"""
import argparse
import hashlib
import io
import math
import struct
import zlib
from html.parser import HTMLParser
import os
import stat
import secrets
import subprocess
import tempfile
import base64
import json
from pathlib import Path
import re
import sys

GAME_IMPORTS = (
    "import { CombatPresentation } from './combat-presentation.mjs';\n"
    "import { StrictEventAdapter, applyStrictSnapshot } from './strict-event-adapter.mjs';\n"
)
MODULES = ("combat-presentation.mjs", "strict-event-adapter.mjs")


def bundle_game(root, files=None):
    """Keep the reviewed modules intact, removing only the exact entry imports."""
    read = lambda name: files[name].decode("utf-8") if files is not None else (root / name).read_text(encoding="utf-8")
    game = read("game.js")
    if not game.startswith(GAME_IMPORTS):
        raise ValueError("Unexpected game import graph; update the explicit offline contract")
    sources = [read(name) for name in MODULES]
    sources.append(game[len(GAME_IMPORTS):])
    for source in sources:
        # Module scope is shared; syntax checking catches duplicate bindings.
        # Disallow dependencies not represented by the closed graph above.
        # This closed-graph packer deliberately uses a conservative keyword
        # allowlist, not a partial JS parser. Check *every* occurrence, even in
        # comments/strings: false-positive refusal is safer than overlooking
        # comment-separated imports/reexports. Only literal import.meta and
        # declaration exports used by these reviewed sources are admitted.
        for keyword in re.finditer(r"\b(?:import|export)\b", source):
            tail = source[keyword.start():]
            if keyword.group() == "import" and re.match(r"import\.meta\b", tail):
                continue
            if keyword.group() == "export" and re.match(r"export (?:const|class|function)\b", tail):
                continue
            raise ValueError("Unbundled JavaScript dependency or unsupported module keyword")
        if re.search(r"</script", source, re.I):
            raise ValueError("JavaScript would terminate its HTML script element")
    return "\n".join(sources)


ASSETS = (*MODULES, "game.js", "index.html", "style.css", "game.wasm",
          "assets/characters.json", "assets/units.png", "assets/enemies.png")
MAX_ASSET = 64 * 1024 * 1024
MAX_SCENE = 65536
MAX_ANIMATION = 65536
MAX_IMAGES = 11
MAX_IMAGE = 8 * 1024 * 1024
MAX_IMAGES_TOTAL = 32 * 1024 * 1024
MAX_DIMENSION = 4096
MAX_FRAMES = 256


def strict_json(data):
    """Reject ambiguous JSON before any path lookup or image decoding."""
    def pairs(values):
        result = {}
        for key, value in values:
            if key in result:
                raise ValueError("Duplicate JSON key")
            result[key] = value
        return result
    def nonfinite(_):
        raise ValueError("Nonfinite JSON value")
    try:
        return json.loads(data.decode("utf-8", errors="strict"), object_pairs_hook=pairs,
                          parse_constant=nonfinite)
    except RecursionError as error:
        raise ValueError("Excessive JSON nesting") from error


def keys(value, required, optional=()):
    if not isinstance(value, dict) or not set(required) <= value.keys() or value.keys() - set(required) - set(optional):
        raise ValueError("Invalid animation object fields")


def number(value, low, high, positive=False):
    return (type(value) in (int, float) and math.isfinite(value)
            and low <= value <= high and (not positive or value > 0))


def pair(value, low, high, positive=False):
    return isinstance(value, list) and len(value) == 2 and all(number(v, low, high, positive) for v in value)


def animation_names(manifest):
    if not isinstance(manifest, dict) or not 1 <= len(manifest) <= 11:
        raise ValueError("Animation manifest must contain 1..11 roles")
    names = set()
    for role, definition in manifest.items():
        if not re.fullmatch(r"(?:unit:[0-5]|enemy:[0-4])", role):
            raise ValueError("Invalid animation role")
        keys(definition, ("image", "clips"), ("anchor", "pivot", "logicalSize"))
        name = definition["image"]
        if (not isinstance(name, str) or len(name) > 128 or not name.endswith(".png")
                or any(not re.fullmatch(r"[A-Za-z0-9_-][A-Za-z0-9_.-]*", part) for part in name.split("/"))):
            raise ValueError("Animation image must be a bounded relative PNG path")
        names.add(name)
        for field, low, high, positive in (("anchor", -16, 16, False), ("pivot", -4096, 4096, False), ("logicalSize", 0, 4096, True)):
            if field in definition and not pair(definition[field], low, high, positive):
                raise ValueError("Invalid animation " + field)
        clips = definition["clips"]
        if not isinstance(clips, dict) or not 1 <= len(clips) <= 5 or clips.keys() - {"idle", "move", "attack", "hit", "death"}:
            raise ValueError("Invalid animation clips")
        for clip_name, clip in clips.items():
            timing = ("windupMs", "releaseMs", "recoveryMs") if clip_name == "attack" else ()
            keys(clip, ("frames",), ("fps", "loop", "pivots", *timing))
            frames = clip["frames"]
            if not isinstance(frames, list) or not 1 <= len(frames) <= MAX_FRAMES:
                raise ValueError("Animation clip requires 1..256 frames")
            for rect in frames:
                if (not isinstance(rect, list) or len(rect) != 4
                        or not all(number(v, 0, MAX_DIMENSION) for v in rect) or rect[2] <= 0 or rect[3] <= 0):
                    raise ValueError("Invalid animation frame rectangle")
            if "fps" in clip and not number(clip["fps"], 1, 120):
                raise ValueError("Invalid animation fps")
            if "loop" in clip and type(clip["loop"]) is not bool:
                raise ValueError("Invalid animation loop")
            if "pivots" in clip and (not isinstance(clip["pivots"], list) or len(clip["pivots"]) != len(frames)
                                    or not all(pair(p, -4096, 4096) for p in clip["pivots"])):
                raise ValueError("Invalid animation pivots")
            if any(field in clip and not number(clip[field], 1, 2000) for field in timing):
                raise ValueError("Invalid animation attack timing")
    return sorted(names)


def validate_png(data):
    """Bound before trusted Pillow CRC verification AND full pixel decoding."""
    if not 0 < len(data) <= MAX_IMAGE or data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("Invalid PNG signature or byte limit")
    position, chunks = 8, []
    while position < len(data):
        if len(data) - position < 12:
            raise ValueError("Truncated PNG chunk")
        length, kind = struct.unpack_from(">I4s", data, position)
        end = position + 12 + length
        if end > len(data) or not re.fullmatch(b"[A-Za-z]{4}", kind):
            raise ValueError("Invalid PNG chunk")
        if zlib.crc32(data[position + 4:end - 4]) != struct.unpack_from(">I", data, end - 4)[0]:
            raise ValueError("Invalid PNG CRC")
        chunks.append(kind)
        if kind in (b"acTL", b"fcTL", b"fdAT"):
            raise ValueError("Animated PNG is not a sprite atlas")
        position = end
        if kind == b"IEND":
            if length or position != len(data):
                raise ValueError("Invalid PNG end or trailing bytes")
            break
    if not chunks or chunks[0] != b"IHDR" or chunks.count(b"IHDR") != 1 or chunks[-1] != b"IEND" or b"IDAT" not in chunks:
        raise ValueError("Incomplete PNG")
    if struct.unpack_from(">I", data, 8)[0] != 13:
        raise ValueError("Invalid PNG IHDR length")
    if any(kind[0] & 32 == 0 and kind not in (b"IHDR", b"PLTE", b"IDAT", b"IEND") for kind in chunks):
        raise ValueError("Unknown critical PNG chunk")
    width, height = struct.unpack_from(">II", data, 16)
    if not 1 <= width <= MAX_DIMENSION or not 1 <= height <= MAX_DIMENSION:
        raise ValueError("PNG dimensions exceed 4096x4096")
    # No optional dependency import on the historical no-animation path.
    from PIL import Image
    with Image.open(io.BytesIO(data)) as image:
        if image.format != "PNG" or image.size != (width, height) or image.n_frames != 1:
            raise ValueError("Invalid PNG image")
        image.verify()
    with Image.open(io.BytesIO(data)) as image:
        image.load()  # verify() alone does not decompress IDAT/pixels.
    return width, height


def validate_animations(data, images, sampler=None):
    if not 0 < len(data) <= MAX_ANIMATION:
        raise ValueError("Animation manifest exceeds 64 KiB or is empty")
    manifest = strict_json(data)
    names = animation_names(manifest)
    if set(names) != images.keys() or len(images) > MAX_IMAGES or sum(map(len, images.values())) > MAX_IMAGES_TOTAL:
        raise ValueError("Animation image inventory or total byte limit")
    dimensions = {name: validate_png(images[name]) for name in names}
    for definition in manifest.values():
        width, height = dimensions[definition["image"]]
        if any(rect[0] + rect[2] > width or rect[1] + rect[3] > height
               for clip in definition["clips"].values() for rect in clip["frames"]):
            raise ValueError("Animation frame outside PNG")
    # Exercise the existing FPS sampler, never convert the author's format.
    with tempfile.TemporaryDirectory(prefix="orr-animation-") as directory:
        path = Path(directory) / "manifest.json"
        path.write_bytes(data)
        sampler_path = Path(directory) / "combat-presentation.mjs"
        if sampler is None:
            sampler = snapshot_file(Path(__file__).resolve().parents[1] / "crates/orr_coop_defense_web/web/combat-presentation.mjs", MAX_ASSET)
        sampler_path.write_bytes(sampler)
        result = subprocess.run(["node", str(Path(__file__).with_name("validate_coop_defense_animation.mjs").resolve()), str(path), str(sampler_path)],
                                shell=False, check=True, capture_output=True, text=True, timeout=15)
    sampled = strict_json(result.stdout.encode("utf-8"))
    digest = hashlib.sha256(data).hexdigest()
    if sampled != {"version": 1, "manifestBytes": len(data), "manifestSha256": digest}:
        raise ValueError("Invalid animation sampler receipt")
    return {**sampled, "images": [{"name": name, "bytes": len(images[name]),
            "sha256": hashlib.sha256(images[name]).hexdigest(), "width": dimensions[name][0],
            "height": dimensions[name][1], "id": "authored-animation-image-" + str(i)} for i, name in enumerate(names)]}


class EmbeddedIds(HTMLParser):
    def __init__(self):
        super().__init__()
        self.ids = set()
    def handle_starttag(self, tag, attrs):
        ids = [value for key, value in attrs if key == "id"]
        if len(ids) > 1 or any(value in self.ids for value in ids):
            raise ValueError("Duplicate HTML ID")
        self.ids.update(ids)


def check_reserved_ids(html):
    parsed = EmbeddedIds()
    parsed.feed(html)
    reserved = {"wasm-core", "tower-atlas", "enemy-atlas", "character-atlas-meta", "game-module", "authored-scene", "authored-scene-receipt"}
    if parsed.ids & reserved or any(value.startswith("authored-animation-") for value in parsed.ids):
        raise ValueError("Reserved embedded asset ID already exists")
    if "combat-animation-manifest" in parsed.ids:
        # Never ship a dangling served manifest/src in a supposedly offline file.
        raise ValueError("Use --animations instead of the served animation hook for offline delivery")


def open_directory(path):
    """Walk without following symlinks, holding descriptors across renames."""
    path = Path(os.path.abspath(path))
    fd = os.open(path.anchor, os.O_RDONLY | os.O_DIRECTORY)
    try:
        for part in path.parts[1:]:
            child = os.open(part, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW, dir_fd=fd)
            os.close(fd)
            fd = child
        return fd
    except BaseException:
        os.close(fd)
        raise


def snapshot_file(path, limit):
    path = Path(path)
    directory = open_directory(path.parent)
    try:
        fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
        with os.fdopen(fd, "rb") as stream:
            before = os.fstat(stream.fileno())
            if not stat.S_ISREG(before.st_mode):
                raise ValueError("Input must be a regular file")
            if before.st_size > limit:
                raise ValueError("Input exceeds byte limit")
            data = stream.read(limit + 1)
            after = os.fstat(stream.fileno())
            if len(data) > limit or (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (after.st_size, after.st_mtime_ns, after.st_ctime_ns):
                raise ValueError("Input changed during bounded snapshot")
            return data
    finally:
        os.close(directory)


def validate_scene(scene, wasm):
    """Only for a trusted locally built engine; this is not a WASM sandbox."""
    scene.decode("utf-8", errors="strict")
    if len(scene) > MAX_SCENE:
        raise ValueError("Scene exceeds 64 KiB")
    with tempfile.TemporaryDirectory(prefix="orr-scene-") as directory:
        root = Path(directory)
        (root / "scene").write_bytes(scene)
        (root / "engine.wasm").write_bytes(wasm)
        result = subprocess.run(
            ["node", str(Path(__file__).with_name("validate_coop_defense_scene.mjs").resolve()),
             str(root / "engine.wasm"), str(root / "scene")],
            shell=False, check=True, capture_output=True, text=True, timeout=15)
    receipt = json.loads(result.stdout)
    expected = {"version": 1, "sceneSha256": hashlib.sha256(scene).hexdigest(),
                "sceneBytes": len(scene), "wasmSha256": hashlib.sha256(wasm).hexdigest()}
    if not isinstance(receipt, dict) or any(receipt.get(k) != v for k, v in expected.items()) or not re.fullmatch(r"[0-9a-f]{16}", str(receipt.get("initialChecksum", ""))):
        raise ValueError("Invalid validator receipt")
    return {**expected, "initialChecksum": receipt["initialChecksum"]}


def binary_script(element_id, data):
    encoded = base64.b64encode(data).decode("ascii")
    return f'<script type="application/octet-stream" id="{element_id}">{encoded}</script>'


def publish_new(output, html):
    """Atomic hard-link publication never replaces an existing destination."""
    directory = open_directory(output.parent)
    name = None
    try:
        # Use an exclusive sibling via the held directory, not a re-resolved path.
        name = ".orr-offline-" + secrets.token_hex(16) + ".tmp"
        fd = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory)
        with os.fdopen(fd, "wb") as stream:
            stream.write(html.encode("utf-8"))
            stream.flush()
            os.fsync(stream.fileno())
        os.link(name, output.name, src_dir_fd=directory, dst_dir_fd=directory, follow_symlinks=False)
    finally:
        if name is not None:
            os.unlink(name, dir_fd=directory)
        os.close(directory)


def package(root, scene=None, output=None, animations=None):
    root = Path(root)
    explicit_output = output is not None
    if (scene is not None or animations is not None) and output is None:
        raise ValueError("Authored scene/animations require explicit new --output")
    output = Path(output) if output is not None else root / "stormglass-foundry-offline.html"
    if scene is not None or animations is not None or explicit_output:
        if os.path.lexists(output):
            raise FileExistsError("Output already exists; choose a new destination")
    files = {name: snapshot_file(root / name, MAX_ASSET) for name in ASSETS}
    js = bundle_game(root, files)
    scene_bytes = snapshot_file(Path(scene), MAX_SCENE) if scene is not None else None
    receipt = validate_scene(scene_bytes, files["game.wasm"]) if scene_bytes is not None else None
    animation_bytes, animation_images, animation_receipt = None, {}, None
    if animations is not None:
        animation_bytes = snapshot_file(Path(animations), MAX_ANIMATION)
        names = animation_names(strict_json(animation_bytes))
        total = 0
        for name in names:
            # Fixed atlases were snapshotted already; reuse those exact bytes.
            image = files.get("assets/" + name)
            if image is None:
                image = snapshot_file(root / "assets" / name, MAX_IMAGE)
            total += len(image)
            if len(image) > MAX_IMAGE or total > MAX_IMAGES_TOTAL:
                raise ValueError("Animation image byte limit")
            animation_images[name] = image
        animation_receipt = validate_animations(animation_bytes, animation_images, files["combat-presentation.mjs"])
    html = files["index.html"].decode("utf-8")
    check_reserved_ids(html)
    css = files["style.css"].decode("utf-8")
    if re.search(r"</style", css, re.I):
        raise ValueError("CSS would terminate its HTML style element")
    embedded = binary_script("wasm-core", files["game.wasm"])
    metadata = json.loads(files["assets/characters.json"].decode("utf-8"))
    encoded = json.dumps(metadata, ensure_ascii=False, separators=(",", ":")).replace("<", "\\u003c")
    embedded += f'<script type="application/json" id="character-atlas-meta">{encoded}</script>'
    embedded += binary_script("tower-atlas", files["assets/units.png"])
    embedded += binary_script("enemy-atlas", files["assets/enemies.png"])
    if receipt is not None:
        embedded += binary_script("authored-scene", scene_bytes)
        encoded = json.dumps(receipt, separators=(",", ":")).replace("<", "\\u003c")
        embedded += f'<script type="application/json" id="authored-scene-receipt">{encoded}</script>'
    if animation_receipt is not None:
        embedded += binary_script("authored-animation-manifest", animation_bytes)
        encoded = json.dumps(animation_receipt, separators=(",", ":")).replace("<", "\\u003c")
        embedded += f'<script type="application/json" id="authored-animation-receipt">{encoded}</script>'
        for image in animation_receipt["images"]:
            embedded += binary_script(image["id"], animation_images[image["name"]])
    style_anchor = '<link rel="stylesheet" href="style.css">'
    script_anchor = '<script type="module" src="game.js"></script>'
    if html.count(style_anchor) != 1 or html.count(script_anchor) != 1:
        raise ValueError("Expected one stylesheet and one game module anchor")
    html = html.replace(style_anchor, "<style>" + css + "</style>")
    html = html.replace(script_anchor, embedded + '<script type="module" id="game-module">' + js + "</script>")
    # Default command retains its historical replace behavior; authored deliveries never do.
    if scene is None and animations is None and not explicit_output:
        if output.is_symlink():
            raise ValueError("Output cannot be a symlink")
        output.write_text(html, encoding="utf-8")
    else:
        publish_new(output, html)
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("build_dir", type=Path)
    parser.add_argument("--scene", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--animations", type=Path)
    args = parser.parse_args()
    print(package(args.build_dir, args.scene, args.output, args.animations))
