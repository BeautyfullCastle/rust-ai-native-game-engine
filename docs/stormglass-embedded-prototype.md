# Stormglass embedded-scene prototype

This opt-in experiment compiles one authored YAML scene into a dedicated native
binary. It uses the existing strict 64 KiB parser, deterministic game/session,
headless runner and interactive presentation. The ordinary `coop_defense` binary
still accepts its original file-based scene options and default configuration.

Build with `cargo build --locked --offline -p orr_sample --features
coop-defense-embedded-prototype --bin stormglass_embedded` (add `--release` for
the optimized profile). The exact input is
`crates/orr_sample/assets/stormglass-embedded.scene.yaml`, included in the thin
binary with `include_str!`. Edit that file and rebuild to change the game.
No generated shell, extraction, appended payload or executable patching is used.

Run `stormglass_embedded` for the native player, or
`stormglass_embedded --headless --ticks 60` for CPU-only verification.
`--scene` is rejected before opening a scene file in either mode. Both modes
parse the identical embedded text before creating a session/window/GPU.
Malformed or oversized source fails closed. This prototype uses the existing
vector presentation and does not promise optional external audio assets.

Acceptance: focused tests reject external overrides and invalid scenes, retain
all six authored settings, match ordinary scene-file sessions for 420 ticks,
and detect a changed reactor value. Windows evidence must additionally compare
original and changed scene builds with independently derived exact runtime
summaries/checksums, retain hashes and sizes, distinguish warm unchanged from
scene-only incremental timing, and prove no-replace publication using continuously
held artifact/source handles after confirmed bounded process/job cleanup.

This is not the final export format or a production publisher. The isolated
Windows evidence harness is separate from the existing directory publisher,
whose known failure is unchanged. Headless acceptance does not validate native
input, a locked desktop, GPU presentation, hostile directory races, portability,
or a clean-machine deployment. The native executable still depends on Windows,
MSVC/UCRT and graphics system libraries. Linux-local mandatory checks and
independent exact-source review remain necessary before integration.
