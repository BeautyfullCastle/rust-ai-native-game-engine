# Rust Scene authoring for the web game

Build the optional Linux tool with `cargo build --release -p orr_package --features coop-web-authoring --bin orr_pkg`.

```sh
orr_pkg create-coop-scene --wasm path/to/trusted/game.wasm --output new.scene.yaml \
  --starting-gold 150 --reactor-hp 30 --last-wave 10
```

This replaces the user-facing `tools/create_coop_defense_scene.py` invocation without Python. **Trusted Node on PATH remains required.** The supplied WASM must be a trusted locally built CoopDefense adapter: validation executes it, and is not a malicious-code sandbox or a WASM memory/resource guarantee. The command never builds, fetches, or invokes arbitrary user-specified scripts/executables.

Gold is 0–10000 (default 150), reactor HP 1–1000 (default 30), last wave 1–10 (default 10). Optional `--spawn-interval-ticks 35..350` writes the existing WavePacing singleton; omission preserves the original engine default and exact legacy YAML bytes. This is authoring, not a new simulation rule or a renderer.

The tool snapshots at most 64 MiB of WASM, runs the exact repository scene verifier in a private directory, and requires the actual adapter to admit the generated scene and produce a tick 0 snapshot. A strict receipt binds version 1, scene byte count/SHA256, WASM SHA256, and the adapter's 16-digit lowercase initial checksum. Structural YAML checks or a checksum computed by a different native build cannot substitute for this gate. The same immutable WASM is verified and its original file is rechecked before publication. The fixed Node supervisor clears NODE_OPTIONS/NODE_PATH, caps output and time, and fails closed on errors; it is shared with animation authoring without weakening either verified-state boundary.

`--output` must name a new file in an existing directory. Symlink ancestry, traversal, overlong paths and duplicate flags are refused. Integers accept ASCII decimal, signs, surrounding whitespace and digit-separating underscores. Unicode decimal digits accepted by Python are deliberately refused; argument/path bounds and duplicate-option refusal are additional safety restrictions. Existing files, directories and links are never overwritten. Publication uses a synced private sibling and exclusive hard link; failure may retain that sibling for inspection. An error after publication is reported as published with uncertain durability/cleanup, never as safe to retry or roll back. This assumes trusted, stable directory ancestry, not hostile concurrent replacement.

Legacy Python remains as a compatibility oracle. Focused tests compare actual old/new YAML and receipts with a real built WASM, exercise admission and output preservation, then feed Rust output to the unchanged offline packer and checker. These checks are CPU/Node packaging evidence, not browser/GPU or a played-game result. Rust HTML packaging, asset copying and portable recipes are now connected through the [official Rust workflow](rust-coop-workflow.md). The legacy Python tools remain test oracles, not required production steps.
