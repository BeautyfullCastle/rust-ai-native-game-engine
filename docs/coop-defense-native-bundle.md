# Native Stormglass bundle

`orr_pkg pack-coop-native` delivers one trusted Linux x86-64 player and an authored
scene as a relocatable folder. It needs neither the source checkout nor a Rust,
Node or WASM toolchain when played. The host still needs compatible Linux shared
libraries, display/graphics drivers, `/bin/sh` and `/usr/bin/sha256sum`.

## Build and package

```sh
cargo build --locked --release -p orr_package --features coop-native-authoring --bin orr_pkg
cargo build --locked --release -p orr_sample --features coop-defense-run-save --bin coop_defense
sha256sum target/release/coop_defense

target/release/orr_pkg pack-coop-native \
  --runtime /absolute/trusted/coop_defense \
  --runtime-sha256 THE_EXACT_LOWERCASE_SHA256 \
  --scene /absolute/authored.scene.yaml \
  --output /existing/parent/new-game --trusted-runtime

/existing/parent/new-game/run-game
/existing/parent/new-game/run-game --headless --ticks 420
/existing/parent/new-game/run-game --run-directory /absolute/private-saves
```

Use `create-coop-native-scene` to author the scene. The output must not exist.
Move or copy the entire four-file directory, preserving executable permissions.
The runtime and scene are directly at its root, beside `run-game` and
`bundle.json`. No engine registry, asset download or compilation happens.
Optional assets already embedded in the supplied executable travel with it;
external audio banks are not packaged or accepted by this launcher. Packaging does
not collect third-party license notices or grant redistribution rights; supply
the notices required for your chosen executable before sharing a release.

## Trust and verification

`--trusted-runtime` explicitly permits executing the supplied native code. Only
use a player built or obtained through a source you trust. SHA-256, executable
permissions and ELF64 x86-64 checks catch mix-ups; they do not authenticate code
or establish a sandbox. The publisher limits runtime input to 256 MiB and scene
to 64 KiB, rejects links and unsafe ancestry, and requires stable trusted paths.

The scene goes through the existing strict native parser. The copied runtime
runs against the copied scene for 60 neutral ticks with a cleared environment,
15-second deadline and 64 KiB combined output limit. Its exact summary and frame
checksum must match the publisher's production simulation. Owned child processes
are terminated before publication. All staged names, bytes, identities and modes
are rechecked, then a descriptor-relative no-replace rename publishes the bundle.

The receipt records exact runtime/scene hashes, the admission build ID and smoke
result. It is integrity and bounded compatibility evidence, not source attestation,
a full campaign win, native-window/GPU coverage or a signature. The launcher checks
its fixed runtime/scene hashes before execution. An owner who changes the launcher
can bypass that check; this is not protection from a hostile bundle owner.

Failed staging directories are retained for inspection. If publication succeeds
but durability is uncertain, the command warns that the output may already exist.
Do not blindly retry, overwrite, or remove it.

## Save boundary

Saving is optional and requires a runtime built with `coop-defense-run-save`.
Create an owner-controlled mode-0700 directory outside the bundle and scene/source
folders, then pass its absolute path explicitly. No default save directory is
created. The launcher rejects bundle-local saves; the native store independently
checks ownership, privacy, no-follow paths and forbidden roots. Headless playback
cannot use `--run-directory`. Save Copy is explicit, immutable and bound to the
exact executable; retain that binary when keeping saves. Existing native save
limits and non-migration rules remain unchanged.

The launcher accepts only interactive play, optional external run-directory, or
`--headless` with optional decimal ticks 0..36000. It rejects arbitrary arguments,
scene overrides, capture, audio and conflicting/duplicate flags.

## Reproducible acceptance

Run native bundle unit/CLI tests, the Python launcher tests, strict package Clippy,
feature-off checks and the repository Linux aggregate. Explicit real-binary
acceptance requires a fresh exact-source player; absence fails rather than skips:

```sh
ORR_NATIVE_BUNDLE_RUNTIME_BIN=/absolute/exact-source/coop_defense \
  cargo test --locked --release -p orr_package --features coop-native-authoring \
  --test native_bundle_cli \
  relocated_native_bundle_runs_without_original_inputs_and_enforces_save_boundary \
  -- --ignored --exact
python -m unittest tools/test_stormglass_native_bundle_launcher.py
```

It verifies copied execution, relocation with original inputs hidden, matching
420-tick output, unchanged bundle bytes, no implicit saves and the production
bundle-root save refusal. Original-input hiding is not an OS sandbox or proof that
every source path is inaccessible. Interactive save/load, hardware and other OSes
retain their separate acceptance requirements.
