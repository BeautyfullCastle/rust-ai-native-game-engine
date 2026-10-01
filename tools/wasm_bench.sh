#!/usr/bin/env sh
# Cross-target bench of the sim core (crates/orr_wasm_bench): native, wasm32-wasip1 under
# wasmtime, wasm32-unknown-unknown in headless Chromium. Prints RESULT lines per target
# (name ns/iter ns/unit iters checksum). Checksums of the same case and iteration count are
# equal on every target, but the runners calibrate iteration counts per target, so compare
# them with `cargo test -p orr_wasm_bench` (fixed counts), not by eye.
#
# Needs: rustup target add wasm32-wasip1 wasm32-unknown-unknown; wasmtime on PATH (optional);
#        wasm-bindgen-cli + Node + Playwright (optional, see tools/build_web.sh); wasm-opt (optional).
# Env: MS (ms per case, default 300), ONLY (case substring filter), WASM_PROFILE (cargo profile, default release).
set -eu
cd "$(dirname "$0")/.."
MS="${MS:-300}"
PROFILE="${WASM_PROFILE:-release}"
ONLY="${ONLY:-}"
pdir="$PROFILE"

echo "== native"
cargo build -p orr_wasm_bench --release --bin bench_runner
target/release/bench_runner --ms "$MS" $ONLY

if command -v wasmtime >/dev/null 2>&1 || [ -x "$HOME/.wasmtime/bin/wasmtime" ]; then
  WT="$(command -v wasmtime || echo "$HOME/.wasmtime/bin/wasmtime")"
  echo "== wasm32-wasip1 (wasmtime)"
  cargo build -p orr_wasm_bench --profile "$PROFILE" --target wasm32-wasip1 --bin bench_runner
  "$WT" run "target/wasm32-wasip1/$pdir/bench_runner.wasm" --ms "$MS" $ONLY
fi

if command -v wasm-bindgen >/dev/null 2>&1 && command -v node >/dev/null 2>&1; then
  echo "== wasm32-unknown-unknown (headless Chromium)"
  cargo build -p orr_wasm_bench --profile "$PROFILE" --target wasm32-unknown-unknown --lib
  wasm-bindgen --target web --out-dir crates/orr_wasm_bench/web/pkg "target/wasm32-unknown-unknown/$pdir/orr_wasm_bench.wasm"
  MS="$MS" ONLY="$ONLY" node tools/wasm_bench.cjs || echo "browser bench skipped (exit $?)"
fi
