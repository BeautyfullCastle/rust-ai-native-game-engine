#!/usr/bin/env bash
# Focused optional-web acceptance; logs and generated packages stay outside git.
# Native Canvas is a separate rendering oracle, never browser/hardware QA.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT="${1:?Usage: tools/check_coop_defense_web.sh /absolute/output/directory}"
if [[ "$OUT" != /* ]]; then echo 'Use an absolute generated-output directory.' >&2; exit 2; fi
export PYTHONDONTWRITEBYTECODE=1
bash tools/build_coop_defense_web.sh "$OUT"
# Generate the complete consumed-input oracle with native Rust; WASM must
# match every tick through two authored victories, terminal idle and restart.
# Keep this evidence beside generated artifacts, never as a committed run log.
cargo run --locked --release -p orr_games --features coop-defense-scene \
  --example coop_defense_campaign_trace > "$OUT/native-campaign-trace.json"
node tools/check_coop_defense_wasm.mjs "$OUT/game.wasm" "$OUT/native-campaign-trace.json"
if [[ -n "${ORR_DEFENSE_PRIOR_WASM:-}" ]]; then
  node tools/check_coop_defense_presentation.mjs "$OUT/game.wasm" "$ORR_DEFENSE_PRIOR_WASM"
else
  node tools/check_coop_defense_presentation.mjs "$OUT/game.wasm"
fi
node tools/check_coop_defense_web_ui.mjs "$OUT"
node tools/check_coop_defense_gestures.mjs "$OUT"
ORR_DEFENSE_WASM="$OUT/game.wasm" python3 -B tools/test_pack_coop_defense_offline.py
python3 -B tools/check_coop_defense_offline.py "$OUT"
ORR_DEFENSE_WASM="$OUT/game.wasm" ORR_DEFENSE_CAMPAIGN_TRACE="$OUT/native-campaign-trace.json" node --test crates/orr_coop_defense_web/web/tests/*.test.mjs
printf '%s\n' 'Optional web checks passed; mandatory Linux aggregate and independent review remain separate.'
