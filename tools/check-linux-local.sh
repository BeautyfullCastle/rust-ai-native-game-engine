#!/usr/bin/env bash
# Linux-local baseline; invocation logs and source identity belong outside git.
# This is not a substitute claim for cross-platform, GPU, or ignored UI tests.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
if [[ "$(uname -s)" != Linux ]]; then
  echo 'Linux is required for this validation entrypoint.' >&2
  exit 2
fi
export PYTHONDONTWRITEBYTECODE=1
export ORR_REQUIRE_C_COMPILER=1
set -x
rustc --version --verbose
cargo --version
python3 --version
# Existing repository-wide formatting debt is advisory, not a new build gate.
format_status=0
cargo fmt --all -- --check || format_status=$?
printf 'Advisory rustfmt exit status: %s\n' "$format_status"
python3 -B tools/check_sim_float_types.py
python3 -B tools/test_sim_float_guard.py
cargo build --locked --workspace --release
# Workspace regressions execute this freshly built example, not a cached CLI
# supplied by the caller. Cargo's artifact message supplies its real path.
assemble_artifacts=$(mktemp /tmp/orr-linux-assemble.XXXXXXXX.jsonl)
printf 'Fresh assemble build messages: %s\n' "$assemble_artifacts"
cargo build --locked --release -p orr_model --no-default-features --features import \
  --example static_model_assemble --message-format=json > "$assemble_artifacts"
ORR_STATIC_MODEL_ASSEMBLE_BIN=$(python3 - "$assemble_artifacts" <<'PYARTIFACT'
import json
import sys
paths = [record["executable"] for line in open(sys.argv[1])
         if (record := json.loads(line)).get("reason") == "compiler-artifact"
         and record.get("target", {}).get("name") == "static_model_assemble"
         and record.get("executable")]
assert len(paths) == 1, paths
print(paths[0])
PYARTIFACT
)
export ORR_STATIC_MODEL_ASSEMBLE_BIN
cargo test --locked --workspace --release
cargo clippy --locked --workspace --all-targets
# The optional changed action route is not enabled by workspace defaults.
cargo build --locked --release -p orr_static_starter --no-default-features --features standalone,static-game-actions --bins
# Compile the changed X11 oracle without pretending to execute a UI test.
cargo test --locked --release -p orr_static_starter --no-default-features --features standalone-native-acceptance --test standalone_x11 --no-run
cargo test --locked --release -p orr_input
# The contract is crate-gated on BOTH features; reject a false-green zero-test run.
action_contract_log=$(mktemp /tmp/orr-linux-action-contract.XXXXXXXX.log)
printf 'Multi-action contract results: %s\n' "$action_contract_log"
cargo test --locked --release -p orr_static_starter --no-default-features --features standalone,static-game-actions --test multi_action_contract 2>&1 | tee "$action_contract_log"
python3 - "$action_contract_log" <<'PYCOUNT'
import re
import sys
from pathlib import Path
counts = re.findall(r"test result: ok\. (\d+) passed; 0 failed;", Path(sys.argv[1]).read_text())
if len(counts) != 1 or int(counts[0]) <= 0:
    raise SystemExit("multi_action_contract must execute and pass tests")
print(f"Verified multi_action_contract: {counts[0]} tests passed")
PYCOUNT
cargo test --locked --release -p orr_editor --features static-game-actions --lib
for test in \
  tools/test_static_game_actions_observation_pacing.py \
  tools/test_static_game_standalone_focus_sync.py \
  tools/test_static_game_standalone_gate.py \
  tools/test_static_game_native_ui_settle_gate.py \
  tools/test_user_game_action_preview_gate.py \
  tools/test_user_game_own_lock_gate.py; do
  python3 -B "$test"
done
set +x
printf 'Advisory rustfmt exit status: %s (0 means passed).\n' "$format_status"
printf '%s\n' 'Linux-local build/test baseline passed. Report any additional native/UI checks separately.' \
  'Windows, macOS, Android, browser, hardware/GPU and ignored tests are not certified by this baseline.'
