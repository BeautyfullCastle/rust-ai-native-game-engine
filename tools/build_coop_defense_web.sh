#!/usr/bin/env bash
# Same deterministic Rust DefenseGame, with a browser-only presentation layer.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-dist/coop-defense-web}"
TARGET="${CARGO_TARGET_DIR:-target}"
cargo build --locked -p orr_coop_defense_web --target wasm32-unknown-unknown --profile web
mkdir -p "$OUT"
cp crates/orr_coop_defense_web/web/* "$OUT/"
cp "$TARGET/wasm32-unknown-unknown/web/orr_coop_defense_web.wasm" "$OUT/game.wasm"
python3 tools/pack_coop_defense_offline.py "$OUT"
printf 'Serve %s with an HTTP static server; open index.html. No account or relay required.\n' "$OUT"
