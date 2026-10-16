# Remember the player's firing control

Linux standalone Arena can remember **Space** or **left mouse** for firing in a
saved project with the `arena-korean-v1` UI. Build with the default-off
`player-settings` feature; it includes `project` and `game-ui` and inherits the
UI's Rust 1.95 requirement. The checked-in saved Arena fixture has no UI until
its font package and `entry.ui` are installed/saved as described in
[Saved Korean game UI](authored-project-ui.md).

```sh
cargo run --release -p orr_sample --bin arena --features player-settings -- \
  --project /absolute/path/to/ui-enabled-project --bridge inproc
```

Open the title/menu settings, select Space or left mouse, then **Apply**. Apply
saves the preference and installs the complete validated input map. **Cancel**
keeps the previous setting. **Reset** changes the draft to Space; Apply is still
required to save it. Changing the setting clears held input, and returning to
play requires a fresh press. Restart retains the active setting; closing and
relaunching loads the saved setting.

An explicit input file remains a read-only session override:

```sh
cargo run --release -p orr_sample --bin arena --features player-settings -- \
  --project /absolute/path/to/ui-enabled-project \
  --input-bindings /absolute/path/to/custom-bindings.json
```

The selector shows that this session uses an external binding file. This route
neither reads nor writes the persistent profile and never imports or rewrites
the override file.

## Profile location and scope

All compatible Arena projects and relocated exports intentionally share the
`arena-controls-v1` player preference. It is not keyed to the project directory,
package digest, export digest or executable location.

- Absolute `$XDG_CONFIG_HOME`: `orrery/arena-controls-v1/settings.json` underneath it
- Otherwise absolute `$HOME`: `.config/orrery/arena-controls-v1/settings.json`
- For a managed installation or a test, `--player-settings-dir /absolute/external/directory`

Empty or relative XDG configuration falls back to an absolute HOME. If no safe
location exists, gameplay uses defaults with a persistence-unavailable notice.
A missing profile loads defaults without creating directories or files. Nothing
is stored under the project, `.orr`, installed package objects or exported
bundle. An explicit directory requires `--project` and, for interactive use, the
authored Korean UI. A read-only game installation is supported when the external
profile directory is writable.

`--headless`, `--capture` and the exporter's smoke run bypass settings discovery
and writes completely. Their explicit settings-directory argument is ignored,
even if it names an invalid or unavailable path. They continue to work with a
cleared environment and an empty working directory.

## Format and failure handling

The first supported format is a bounded JSON object:

```json
{"schema":1,"profile":"arena-controls-v1","fire_binding":"left_mouse"}
```

The other firing value is `"space"`. Unknown fields, duplicate fields, nulls,
invalid values, unsafe file types and oversized data are refused. Schema 1 is
the first format; there is no legacy save migration. This profile contains no
simulation frame, score, checkpoint, inventory, replay, entity handle or asset
path. Such game-progress features require separate identity and format designs.

Apply uses an exclusive nonblocking writer lock, a same-directory temporary
file, synchronization and atomic primary-file replacement. The previous valid
primary is retained as `settings.json.bak`; a malformed primary can recover from
that valid backup with a visible notice. Unknown future schema/profile data is
preserved read-only rather than downgraded. A busy or unwritable profile leaves
the current controls active and reports the problem.

Failure before primary publication leaves the previous setting active. A
failure after publication is reported as durability uncertain: the new setting
is active, and the application does not blindly retry or claim that nothing
was saved. These guarantees concern the tested Linux process/filesystem
behavior; they do not establish power-loss durability on every filesystem.

## Focused verification

No test uses a real user's configuration. Child processes clear their
environment and use temporary external directories.

```sh
# Feature guard and manifest-level default/project/game-ui/editor boundaries
cargo test --release -p orr_sample --test player_settings_cli

# Fresh-process store/input-adapter lifecycle and real CLI bypass checks
cargo test --release -p orr_sample --features player-settings \
  --test player_settings_lifecycle --test player_settings_cli --test player_settings_ui

# Mandatory Linux source hiding and software-GPU capture; replace workspace root
ORR_REQUIRE_PROJECT_ISOLATION=1 ORR_EXPORT_HIDE_ROOT=/absolute/workspace/root \
ORR_REQUIRE_GPU=1 \
cargo test --release -p orr_sample --features player-settings,project-export \
  --test player_settings_lifecycle --test player_settings_cli --test player_settings_ui
```

To retain the actual settings panel images, create an empty directory and set
`ORR_PLAYER_SETTINGS_CAPTURE_DIR` to its absolute path. The UI test clicks the
real left-mouse selector and Apply button, then captures the default and saved
Korean panels over the admitted project scene. Files are created without
overwriting existing captures.

The lifecycle harness opens the admitted Korean project and uses the production
settings session, store and input adapter in separate processes. It checks effective mouse/Space
binding after relaunch and exits immediately after commit without relying on
Rust destructors. The export proof relocates a generated bundle, makes it
read-only, hides the original workspace/project/runtime with bubblewrap when
required, persists the shared profile externally, and checks every bundle and
source-project file digest. It also launches the actual copied runtime through
`run-arena` twice in headless mode.

These harness checks are distinct from actual widget dispatch and restart tests
in the App module. The headless/capture CLI tests prove those paths do not access
the profile; they do not prove native-window input. Optional source-hiding or GPU
checks print an explicit diagnostic when unavailable; set the mandatory flags
above for acceptance. Full inherited CI, exact-head review, native-device testing,
Windows/macOS support and broader game saves remain separate gates.
