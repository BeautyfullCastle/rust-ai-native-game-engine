# Stormglass Windows content admission prerequisite

`orr_package` feature `stormglass-windows-admission` exposes
`stormglass_windows_admission::admit(runtime_bytes, trusted_sha256, scene_bytes)`.
It checks bounded byte snapshots, an exact SHA256 pin, x64 PE32+ application
headers with bounded file-backed sections/entry point, and the existing
`orr_games::coop_defense_scene::parse_config` scene contract. It returns borrowed
immutable snapshots, the parsed configuration, and SHA256/length records for
exactly `coop_defense.exe`, `game.scene.yaml`, and the repository's unchanged
`run-game.ps1` bytes. The feature adds no platform or process dependencies.

This is **not a folder publisher or an executable verifier**. Header admission
does not promise that Windows can load a binary or that its imports are present.
A digest is integrity, not authentication or a sandbox. Callers must establish
trust in both the executable and its expected digest before executing anything.
This API neither reads input paths nor writes, executes, or publishes anything;
its file records are not smoke or publication receipts.

A future Windows publisher still needs no-reparse reserved filesystem snapshots,
source/staging rechecks, nonexistent-output enforcement, bounded headless smoke
of the copied binary against production expected summary/checksum, and atomic
no-replace publication. Optional assets, audio banks, settings and helpers are
not admitted by this prerequisite. No existing Windows export game enum changes.

Focused validation:

```
cargo test --offline --locked -p orr_package --features stormglass-windows-admission stormglass_windows_admission
cargo clippy --offline --locked -p orr_package --all-targets --features stormglass-windows-admission -- -D warnings
```

Tests cover fixed byte/config/hash identity, invalid pins and changed bytes,
strict scene rejection, truncated headers, wrong platforms/DLLs, section bounds,
entry points and deterministic content records. One explicitly ignored test can
read an actual trusted binary and scene by setting `STORMGLASS_ADMISSION_RUNTIME`,
`STORMGLASS_ADMISSION_SCENE`, and `STORMGLASS_ADMISSION_SHA256` and selecting `real_local_runtime_content_admission
-- --ignored`. It never launches that binary. Publisher CLI/path/process failures,
relocation and GUI/hardware behavior remain outside this prerequisite's coverage.
