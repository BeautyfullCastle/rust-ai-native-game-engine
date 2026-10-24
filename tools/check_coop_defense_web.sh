#!/usr/bin/env bash
# Focused optional-web acceptance; logs and generated packages stay outside git.
# Native Canvas is a separate rendering oracle, never browser/hardware QA.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
OUT="${1:?Usage: tools/check_coop_defense_web.sh /absolute/output/directory}"
if [[ "$OUT" != /* ]]; then echo 'Use an absolute generated-output directory.' >&2; exit 2; fi
export PYTHONDONTWRITEBYTECODE=1
bash tools/build_coop_defense_web.sh "$OUT"
node tools/check_coop_defense_wasm.mjs "$OUT/game.wasm"
if [[ -n "${ORR_DEFENSE_PRIOR_WASM:-}" ]]; then
  node tools/check_coop_defense_presentation.mjs "$OUT/game.wasm" "$ORR_DEFENSE_PRIOR_WASM"
else
  node tools/check_coop_defense_presentation.mjs "$OUT/game.wasm"
fi
node tools/check_coop_defense_web_ui.mjs "$OUT"
node tools/check_coop_defense_gestures.mjs "$OUT"
python3 -B tools/test_pack_coop_defense_offline.py
python3 -B tools/check_coop_defense_offline.py "$OUT"
ORR_DEFENSE_WASM="$OUT/game.wasm" node --test crates/orr_coop_defense_web/web/tests/*.test.mjs
printf '%s\n' 'Optional web checks passed; mandatory Linux aggregate and independent review remain separate.'
