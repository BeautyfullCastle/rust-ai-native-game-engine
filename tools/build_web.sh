#!/usr/bin/env sh
# Builds the browser client: crates/orr_web -> crates/orr_web/web/pkg (wasm + JS glue).
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock> --locked
# Serve crates/orr_web/web/ over http://localhost (WebTransport needs a secure context).
#
# Optional:
#   wasm-opt (binaryen, `apt install binaryen` or `npm i -g binaryen`; or WASM_OPT="node /path/to/wasm-opt")
#       runs on the result: -O3 by default, WASM_OPT_LEVEL=Oz for the smallest file.
#   WEB_SIMD=1   compile with wasm `simd128` (every current browser has it): about 1.4x faster
#       frame checksums and ECS passes, identical results (docs/wasm-bench.md). Off by default so
#       the build runs on browsers older than Safari 16.4 / Chrome 91 / Firefox 89.
#   WEB_GPU=0    skip the GPU view package (crates/orr_web_gpu -> web/pkg_gpu: orr_render on wgpu, WebGPU with a
#       WebGL2 fallback). The page then draws on a 2D canvas. The package is about 3 MB.
#   WEB_PROFILE  cargo profile (default `web`: opt-level 3, fat LTO, one codegen unit, no debug info;
#       `web-small` is the same with opt-level "s").
set -eu
cd "$(dirname "$0")/.."
PROFILE="${WEB_PROFILE:-web}"
if [ "${WEB_SIMD:-0}" = "1" ]; then
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+simd128"
  FEATURES="--enable-simd"
else
  FEATURES=""
fi
WO="${WASM_OPT:-}"
if [ -z "$WO" ] && command -v wasm-opt >/dev/null 2>&1; then WO=wasm-opt; fi

# build_pkg <cargo package> <wasm file stem> <output dir>
build_pkg() {
  cargo build -p "$1" --profile "$PROFILE" --target wasm32-unknown-unknown
  wasm-bindgen --target web --out-dir "$3" "target/wasm32-unknown-unknown/$PROFILE/$2.wasm"
  if [ -n "$WO" ]; then
    W="$3/${2}_bg.wasm"
    # shellcheck disable=SC2086
    $WO "-${WASM_OPT_LEVEL:-O3}" --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int --enable-mutable-globals $FEATURES "$W" -o "$W.opt"
    mv "$W.opt" "$W"
  fi
}

[ -n "$WO" ] || echo "wasm-opt not found: skipping (the .wasm files are about 25% larger)"
build_pkg orr_web orr_web crates/orr_web/web/pkg
ls -l crates/orr_web/web/pkg
if [ "${WEB_GPU:-1}" = "1" ]; then
  build_pkg orr_web_gpu orr_web_gpu crates/orr_web/web/pkg_gpu
  ls -l crates/orr_web/web/pkg_gpu
fi
