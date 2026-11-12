#!/usr/bin/env bash
# Same deterministic Rust DefenseGame, with a browser-only presentation layer.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT="${1:-dist/coop-defense-web}"
TARGET="${CARGO_TARGET_DIR:-target}"
# Completed deliveries are never silently overwritten. Refuse before Cargo or
# generated-file writes, and choose a fresh directory for the next delivery.
if [[ -e "$OUT/stormglass-foundry-offline.html" || -L "$OUT/stormglass-foundry-offline.html" ]]; then
  printf 'Offline HTML already exists; choose a fresh output directory: %s\n' "$OUT" >&2
  exit 1
fi
if [[ -L "$OUT/game.wasm" || ( -e "$OUT/game.wasm" && ! -f "$OUT/game.wasm" ) ]]; then
  printf 'Generated game.wasm must not be a symlink or nonregular object.\n' >&2
  exit 1
fi
cargo build --locked -p orr_package --features coop-web-authoring --release --bin orr_pkg
cargo build --locked -p orr_coop_defense_web --target wasm32-unknown-unknown --profile web
"$TARGET/release/orr_pkg" copy-coop-web-assets \
  --source-dir "$PWD/crates/orr_coop_defense_web/web" --output "$OUT" \
  --wasm "$TARGET/wasm32-unknown-unknown/web/orr_coop_defense_web.wasm"
"$TARGET/release/orr_pkg" pack-coop-offline --build-dir "$OUT" \
  --output "$OUT/stormglass-foundry-offline.html"
printf 'Serve %s with an HTTP static server; open index.html. No account or relay required.\n' "$OUT"
