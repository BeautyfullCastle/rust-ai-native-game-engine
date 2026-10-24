#!/usr/bin/env python3
"""Embed this optional adapter's exact engine/art and closed ES-module graph.

This is intentionally not a general JavaScript bundler. New imports require an
explicit graph update and regression tests, rather than silently shipping an
HTML file that still needs sibling modules.
"""
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


def bundle_game(root):
    """Keep the reviewed modules intact, removing only the exact entry imports."""
    game = (root / "game.js").read_text()
    if not game.startswith(GAME_IMPORTS):
        raise ValueError("Unexpected game import graph; update the explicit offline contract")
    sources = [(root / name).read_text() for name in MODULES]
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


def binary_script(element_id, path):
    encoded = base64.b64encode(path.read_bytes()).decode("ascii")
    return f'<script type="application/octet-stream" id="{element_id}">{encoded}</script>'


def package(root):
    html = (root / "index.html").read_text()
    css = (root / "style.css").read_text()
    js = bundle_game(root)
    if re.search(r"</style", css, re.I):
        raise ValueError("CSS would terminate its HTML style element")
    embedded = binary_script("wasm-core", root / "game.wasm")
    assets = root / "assets"
    metadata = json.loads((assets / "characters.json").read_text())
    encoded = json.dumps(metadata, ensure_ascii=False, separators=(",", ":")).replace("<", "\\u003c")
    embedded += f'<script type="application/json" id="character-atlas-meta">{encoded}</script>'
    embedded += binary_script("tower-atlas", assets / "units.png")
    embedded += binary_script("enemy-atlas", assets / "enemies.png")
    style_anchor = '<link rel="stylesheet" href="style.css">'
    script_anchor = '<script type="module" src="game.js"></script>'
    if html.count(style_anchor) != 1 or html.count(script_anchor) != 1:
        raise ValueError("Expected one stylesheet and one game module anchor")
    html = html.replace(style_anchor, "<style>" + css + "</style>")
    html = html.replace(script_anchor, embedded + '<script type="module" id="game-module">' + js + "</script>")
    output = root / "stormglass-foundry-offline.html"
    output.write_text(html)
    return output


if __name__ == "__main__":
    print(package(Path(sys.argv[1])))
