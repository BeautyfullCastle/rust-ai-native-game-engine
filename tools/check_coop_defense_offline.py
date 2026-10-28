#!/usr/bin/env python3
"""Verify the offline package embeds byte-identical engine/art and exact metadata."""
import base64
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import sys
import subprocess
import tempfile

from pack_coop_defense_offline import (bundle_game, validate_scene, validate_animations, strict_json,
    MAX_ANIMATION, MAX_IMAGE, MAX_IMAGES_TOTAL, snapshot_file)


class Package(HTMLParser):
    def __init__(self):
        super().__init__()
        self.scripts = {}
        self.current = None
        self.external = []
        self.ids = set()

    def handle_starttag(self, tag, attrs):
        ids = [value for key, value in attrs if key == "id"]
        assert len(ids) <= 1 and not any(value in self.ids for value in ids), "duplicate embedded asset ID"
        self.ids.update(ids)
        attrs = dict(attrs)
        if tag == "script":
            if "src" in attrs:
                self.external.append(attrs["src"])
            self.current = attrs.get("id")
            if self.current:
                assert self.current not in self.scripts, "duplicate embedded asset ID"
                self.scripts[self.current] = ""
        if tag == "link" and attrs.get("rel") == "stylesheet":
            self.external.append(attrs.get("href"))

    def handle_endtag(self, tag):
        if tag == "script":
            self.current = None

    def handle_data(self, data):
        if self.current:
            self.scripts[self.current] += data


root = Path(sys.argv[1])
html = Path(sys.argv[2]) if len(sys.argv) > 2 else root / "stormglass-foundry-offline.html"
parser = Package()
parser.feed(html.read_text())
assert not parser.external, "offline package depends on external CSS or scripts"
for element_id, source in {
    "wasm-core": root / "game.wasm",
    "tower-atlas": root / "assets/units.png",
    "enemy-atlas": root / "assets/enemies.png",
}.items():
    assert base64.b64decode(parser.scripts[element_id], validate=True) == source.read_bytes(), element_id
assert parser.scripts["game-module"] == bundle_game(root), "offline ES modules differ from the exact served sources"
with tempfile.NamedTemporaryFile(mode="w", suffix=".mjs") as module:
    module.write(parser.scripts["game-module"]); module.flush()
    subprocess.run(["node", "--check", module.name], check=True)
if "authored-scene" in parser.scripts or "authored-scene-receipt" in parser.scripts:
    encoded_scene = parser.scripts["authored-scene"]
    scene = base64.b64decode(encoded_scene, validate=True)
    assert base64.b64encode(scene).decode("ascii") == encoded_scene, "noncanonical authored scene base64"
    receipt = json.loads(parser.scripts["authored-scene-receipt"])
    assert receipt == validate_scene(scene, base64.b64decode(parser.scripts["wasm-core"], validate=True)), "authored receipt differs from actual engine admission"
animation_ids = {key for key in parser.ids if key.startswith("authored-animation-")}
if animation_ids:
    assert "combat-animation-manifest" not in parser.ids, "conflicting served animation source"
    def binary(element_id, limit):
        encoded = parser.scripts[element_id]
        assert len(encoded) <= 4 * ((limit + 2) // 3), "oversized animation base64"
        data = base64.b64decode(encoded, validate=True)
        assert len(data) <= limit and base64.b64encode(data).decode("ascii") == encoded, "noncanonical animation base64"
        return data
    animation = binary("authored-animation-manifest", MAX_ANIMATION)
    receipt_text = parser.scripts["authored-animation-receipt"]
    assert len(receipt_text) <= 8192, "oversized animation receipt"
    receipt = strict_json(receipt_text.encode("utf-8"))
    assert isinstance(receipt, dict) and isinstance(receipt.get("images"), list) and len(receipt["images"]) <= 11, "invalid animation receipt"
    images = {}
    expected_ids = {"authored-animation-manifest", "authored-animation-receipt"}
    total = 0
    for i, item in enumerate(receipt["images"]):
        assert isinstance(item, dict) and item.get("id") == "authored-animation-image-" + str(i), "invalid animation image ID"
        name = item["name"]
        assert isinstance(name, str) and name not in images, "duplicate animation image name"
        images[name] = binary(item["id"], MAX_IMAGE)
        total += len(images[name])
        assert total <= MAX_IMAGES_TOTAL, "animation total image bytes"
        expected_ids.add(item["id"])
    assert animation_ids == expected_ids, "unexpected animation payload IDs"
    assert receipt == validate_animations(animation, images, snapshot_file(root / "combat-presentation.mjs", 64 * 1024 * 1024)), "authored animation receipt differs from validated payload"
    for name, data in images.items():
        assert data == snapshot_file(root / "assets" / name, MAX_IMAGE), "animation PNG differs from exact served source"
metadata = json.loads(parser.scripts["character-atlas-meta"])
assert metadata == json.loads((root / "assets/characters.json").read_text())
assert len(metadata["heroes"]) == 6 and len(metadata["enemies"]) == 4
print(json.dumps({"passed": True, "kind": "offline engine/art/module integrity and JS syntax, not browser QA", "bytes": html.stat().st_size, "sha256": hashlib.sha256(html.read_bytes()).hexdigest()}, indent=2))
