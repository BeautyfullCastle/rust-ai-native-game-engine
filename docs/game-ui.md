# Optional native Korean game UI

The `orr_sample` `game-ui` feature uses egui/egui-winit/egui-wgpu 0.36.2,
independently of `orr_editor`. This optional feature requires Rust 1.95;
validation uses 1.97.1. Default builds retain the workspace MSRV and existing
window/input behavior. `game-ui` enables `input-actions` for capture-safe controls.

Install the licensed font package, then opt in at runtime:

```sh
cargo run -p orr_package --bin orr_pkg -- install /path/to/project --path assets/game_ui_font
cargo run -p orr_sample --bin arena --features game-ui -- --game-ui-project /path/to/project
```

See `orr_pkg --help` for package CLI syntax. The font is read from the installed,
hash-verified `korean-game-ui` package, never from runtime system fonts. A missing,
modified, malformed, or insufficient font fails startup. The package is SIL OFL,
not MIT; see its license, provenance, deterministic subset script and font notes.

The Korean title, HUD, menu, continue, restart and quit buttons are real widgets.
Menus stop only local controls: simulation and network continue, including on the
title screen. A visible notice states this. The HUD shows tick, verified tick and
rollback count. The menu button opens the menu; the configured pause binding also
opens it during gameplay. In a menu the pause binding is captured; use Continue
to resume. Continue uses a button and does not synthesize a key.
Captured input is neutralized immediately, release/focus events are preserved,
and resuming requires fresh presses. Input bindings remain configurable.

Restart is a fresh local-loopback bridge using the original configuration; it
resets view interpolation, audio voices, controls, sprite playback and camera
history. Relay restart is disabled both in the UI and action dispatch. Repeated
local restarts are supported. Existing `run` callers without a restart factory
also receive no restart capability.

The overlay draws on the acquired scene view before the same present, following
shape/sprite draws. It accepts only mesh primitives and submits through the RHI.
Skipped acquisition retains ordered texture epochs, including free/reuse across
frames. The queue is capped at 256 epochs and 64 MiB texture payload; exceeding
these limits reports an error rather than silently losing updates. Tests use a single-view GPU
target, not OffscreenTarget's alternate-format views. Window resize and scale
changes feed the winit integration; zero-size surfaces keep their last usable size.

Scope excludes text entry/IME, localization catalogs, accessibility certification,
gamepads, simulation pause, relay restart, browser/mobile UI and an editor wrapper.
Headless widget/GPU tests do not establish native-window execution.
