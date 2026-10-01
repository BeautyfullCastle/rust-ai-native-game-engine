#!/usr/bin/env sh
# Builds the browser client: crates/orr_web -> crates/orr_web/web/pkg (wasm + JS glue).
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version <the wasm-bindgen version in Cargo.lock> --locked
# Serve crates/orr_web/web/ over http://localhost (WebTransport needs a secure context).
set -eu
cd "$(dirname "$0")/.."
cargo build -p orr_web --release --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir crates/orr_web/web/pkg target/wasm32-unknown-unknown/release/orr_web.wasm
ls -l crates/orr_web/web/pkg
