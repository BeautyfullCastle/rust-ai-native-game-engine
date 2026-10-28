#!/usr/bin/env python3
"""Embed this optional adapter's exact engine/art and closed ES-module graph.

This is intentionally not a general JavaScript bundler. New imports require an
explicit graph update and regression tests, rather than silently shipping an
HTML file that still needs sibling modules.
"""
import argparse
import hashlib
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


def package(root, scene=None, output=None):
    root = Path(root)
    explicit_output = output is not None
    if scene is not None and output is None:
        raise ValueError("Authored scene requires explicit new --output")
    output = Path(output) if output is not None else root / "stormglass-foundry-offline.html"
    if scene is not None or explicit_output:
        if os.path.lexists(output):
            raise FileExistsError("Output already exists; choose a new destination")
    files = {name: snapshot_file(root / name, MAX_ASSET) for name in ASSETS}
    scene_bytes = snapshot_file(Path(scene), MAX_SCENE) if scene is not None else None
    receipt = validate_scene(scene_bytes, files["game.wasm"]) if scene_bytes is not None else None
    html = files["index.html"].decode("utf-8")
    css = files["style.css"].decode("utf-8")
    js = bundle_game(root, files)
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
    style_anchor = '<link rel="stylesheet" href="style.css">'
    script_anchor = '<script type="module" src="game.js"></script>'
    if html.count(style_anchor) != 1 or html.count(script_anchor) != 1:
        raise ValueError("Expected one stylesheet and one game module anchor")
    html = html.replace(style_anchor, "<style>" + css + "</style>")
    html = html.replace(script_anchor, embedded + '<script type="module" id="game-module">' + js + "</script>")
    # Default command retains its historical replace behavior; authored deliveries never do.
    if scene is None and not explicit_output:
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
    args = parser.parse_args()
    print(package(args.build_dir, args.scene, args.output))
