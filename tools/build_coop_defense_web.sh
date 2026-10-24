#!/usr/bin/env bash
# Same deterministic Rust DefenseGame, with a browser-only presentation layer.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-dist/coop-defense-web}"
TARGET="${CARGO_TARGET_DIR:-target}"
cargo build --locked -p orr_coop_defense_web --target wasm32-unknown-unknown --profile web
mkdir -p "$OUT"
WEB="crates/orr_coop_defense_web/web"
cp "$WEB/index.html" "$WEB/style.css" "$WEB/game.js" "$WEB/combat-presentation.mjs" "$WEB/strict-event-adapter.mjs" "$OUT/"
if [ -d "$WEB/assets" ]; then
  cp -R "$WEB/assets" "$OUT/"
else
  cp "$WEB/units.png" "$WEB/units.json" "$OUT/"
fi
cp "$TARGET/wasm32-unknown-unknown/web/orr_coop_defense_web.wasm" "$OUT/game.wasm"
python3 tools/pack_coop_defense_offline.py "$OUT"
printf 'Serve %s with an HTTP static server; open index.html. No account or relay required.\n' "$OUT"
