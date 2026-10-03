# Focused validation record

Run on 2026-10-03, Linux x86_64, Rust/Cargo 1.99.0. Base: local
`ba301295b2d81935decff6eb984a55d41a74c939` (reviewed asset-core tree).
All build outputs used `/tmp/orrery-cooker-target`; no workspace-wide, graphics,
FFI, audio or replay regression builds were run for this work.

Passed after the object-only parser and cache/output isolation review fixes:

- `cargo tree -p orr_asset_cook --locked --offline`: only asset/fp, serde/JSON,
  SHA-256 and OS randomness utility dependencies; no audio/graphics/session edge
- `cargo check -p orr_asset_cook --locked --offline`
- `cargo test -p orr_asset -p orr_asset_cook --locked --offline`: 13 core tests,
  28 cooker tests, one compile-fail core doc test; zero failures
- `cargo clippy -p orr_asset_cook --all-targets --locked --offline -- -D warnings`
- `cargo fmt -p orr_asset_cook --check`
- `git diff --check`
- CLI `cook --check` on `assets/fixture_v1/cooked` with both declared roots
- CLI `inspect` on that bundle; independent Python reconstruction matched both
  payloads, both manifests, and all four SHA-256 fixture vectors exactly

Cooker test groups: nine authoring, two CLI, two checked-in/generated Rust,
fifteen filesystem/pipeline tests. The latter cover cache corruption/partial
entries, cold/warm and source reorder/relocation, strict output comparison,
failed publication preserving old bytes, source escape/aliases, and cache/output
isolation before directory creation (including symlink/missing-prefix/`..` and
conservative case/trailing-dot/space aliases).

Windows/macOS/wasm/device execution, workspace CI, actual audio playback,
release binding, asset fixture replay and runtime performance remain unverified
by this change. Unix symlink tests are platform-gated. The case-alias regression
runs on Linux too, but is not evidence of a Windows execution pass.

No default-game golden, ORRF/ORRP format, existing production API, or CI workflow
is changed. This record does not claim issue #24 end-to-end completion.
