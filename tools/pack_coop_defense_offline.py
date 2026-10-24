#!/usr/bin/env python3
"""Package our static game into one offline HTML, with identical Rust WASM bytes."""
import base64
import pathlib
import sys
root = pathlib.Path(sys.argv[1])
html = (root / 'index.html').read_text()
css = (root / 'style.css').read_text()
js = (root / 'game.js').read_text()
wasm = base64.b64encode((root / 'game.wasm').read_bytes()).decode('ascii')
html = html.replace('<link rel="stylesheet" href="style.css">', '<style>' + css + '</style>')
atlas_path = root / 'units.png'
atlas = '<script type="application/octet-stream" id="tower-atlas">' + base64.b64encode(atlas_path.read_bytes()).decode('ascii') + '</script>' if atlas_path.exists() else ''
html = html.replace('<script type="module" src="game.js"></script>', atlas + '<script type="application/octet-stream" id="wasm-core">' + wasm + '</script><script type="module">' + js + '</script>')
(root / 'stormglass-foundry-offline.html').write_text(html)
print(root / 'stormglass-foundry-offline.html')
