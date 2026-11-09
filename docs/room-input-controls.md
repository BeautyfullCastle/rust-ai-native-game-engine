# Author Room keyboard controls

`tools/create_room_controls.py` creates one new Room controls JSON sidecar using
Python 3.11+ and its standard library. It never opens or modifies a project
manifest. This is a command-line authoring helper, not GUI Save/reopen authoring.

## Create and attach

From the repository root, choose an existing Room project directory and a new
filename:

```sh
python3 tools/create_room_controls.py \
  --output /absolute/path/to/room/room.controls.json \
  --move-left j,arrow_left --move-right l,arrow_right \
  --move-forward i,arrow_up --move-backward k,arrow_down --interact o
```

Explicitly add `"controls": "room.controls.json"` to the `entry` object of that
project's `orr.project.json`. Keep the sidecar at the project root with a filename
distinct, ignoring case, from every other entry document. Existing files are not
replaced; to change a binding, create a different filename and explicitly update
the manifest reference. The tool prints these attachment instructions.

The Room runtime and exporter must be built with the optional `orr_sample`
`room-input-actions` feature in addition to their normal Room features. Consumers
without it reject an authored controls entry rather than silently ignoring it.
Projects without an entry keep the existing default controls.

## Contract

The output is `orr_input::ActionMap` version 1: exactly `move_left`, `move_right`,
`move_forward`, `move_backward`, and `interact`, each with 1–8 physical keyboard
bindings. Use exact lowercase names: `a`–`z`, `arrow_left`, `arrow_right`,
`arrow_up`, `arrow_down`, `space`, `enter`, or `tab`. `r` and `escape` are reserved
for restart and exit. No whitespace, logical/layout-dependent labels, modifiers,
mouse, gamepad, unknown keys, repeated options, or duplicate keys (even within
one action) are accepted. Aliases are sorted for deterministic bytes.

Runtime opposition cancels: left plus right yields zero X; forward plus backward
yields zero Z. Interact remains held while any of its keys is held. The helper
only authors the map; the Rust Room consumer owns input interpretation.

Arguments, bindings, filenames and output bytes are bounded. The parent directory
must already exist without symlink components. Existing destination files,
directories and symlinks are preserved. Publication uses a synced staging file
and an atomic no-replace hard link; filesystems without hard-link support fail
without an overwrite fallback. A cleanup failure is reported explicitly and may
leave a staging file; if already published, the error says so. Concurrent hostile
parent-directory replacement and power-loss durability are outside this local
filesystem contract.

## Acceptance

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools \
  -p test_create_room_controls.py -v
PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=tools python3 -c \
  'from test_create_room_controls import run_integration; run_integration("/trusted/binaries")'
```

The second command requires trusted, already-built `orr_new_arena`, `room_escape`
and `orr_export_room` executables with the applicable features. It creates an
isolated temporary Room project, runs the authoring CLI, explicitly updates the
fixture manifest, opens the project headlessly, exports, checks exact sidecar
bytes, and reopens through the exported launcher with matching CPU output and
43 model-pose frames checked. Its headless holds are semantic actions, not
physical keyboard events. It proves CPU admission/export/reopen, not native
keyboard handling, GPU rendering, source-hidden operation, or cross-platform
execution. No dependencies are installed and no Rust build is launched.
