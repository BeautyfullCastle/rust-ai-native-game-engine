# Export a Room puzzle

Create a starter and attach the arrow/Space preset using the commands in
[README.md](README.md). Export the resulting saved project with a known trusted
Linux x86-64 runtime built with `room-project` support. This copies the authored
scene, referenced models/camera, controls and required package content into a
new relocatable folder. Authoring backups are not shipped.

```sh
RUNTIME=target/release/room_escape
SHA=$(sha256sum "$RUNTIME" | cut -d' ' -f1)
target/release/orr_export_room --project '/path/to/my-puzzle' \
  --runtime "$RUNTIME" --runtime-sha256 "$SHA" \
  --output '/path/to/new-bundle' --trusted-runtime
'/path/to/new-bundle/run-room-escape'
```

The destination must not exist. Review the executable's build/source evidence
before using `--trusted-runtime`; calculating a hash does not establish trust.
The export does not build, download, install or publish anything. Keep the
source project separately for editing. Move the entire bundle, not just its
launcher. Native execution requires the runtime's host libraries and graphics
support; the folder is not a universal cross-platform installer.

For a CPU check without a window:

```sh
'/path/to/new-bundle/run-room-escape' --headless --ticks 42 \
  --hold left,right,up,down,interact --check-model-poses
```

`--hold` supplies constant semantic actions, not physical key events. The three
starter solutions have multiple steps and are checked separately against the
exported project with `orr_playtest_room` and their schema-2 key routes. They
are not scripted through the graphical launcher.

## Repeatable acceptance

With trusted already-built creator, controls attachment, exporter, playtest and
runtime executables in one directory, run from the repository:

```sh
python3 -B tools/test_room_grid_exports.py --binary-dir target/release \
  --output-dir /path/to/new-acceptance-directory
```

An explicit `--runtime /path/to/known-runtime` can select a separately retained
runtime. The harness never rebuilds. It preserves all generated projects,
export folders and command receipts, reserves space for three runtime copies
and an exporter stage, and refuses to cross a 1 GiB free-space floor.

It checks all three puzzle victories, failed shortcuts and repeats against
relocated exported projects, exact manifest/file hashes and modes, controls
preservation, backup exclusion, existing-output refusal, and unchanged inputs.
The original source and bundle paths are renamed away before relocation tests.
Actual relocated launchers must reproduce the source runtime's 42-tick CPU
output and model-pose checks. This does not establish hardware keyboard,
window/GPU, other-platform, or full-repository validation.
