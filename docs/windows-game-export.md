# Windows game export from the editor

## Workflow

The optional `windows-export` editor feature exposes `File > Export Windows
game…` for a local, saved Room project in Edit mode. It does not enable export
for the separate MeshNavigation game or make those scenes Room projects.

```powershell
cargo build --release --locked -p orr_editor --features windows-export
cargo build --release --locked -p orr_sample --features room-project --bin room_escape
target/release/orr_editor.exe --room-project E:/Games/MyRoom
```

Save the scene, model bindings and camera separately before exporting. Apply
and save camera drafts, or discard them explicitly. Unsaved presentation edits
block export instead of silently exporting an older on-disk sidecar. Finish
open proposals and transactions, and leave Play. In the export window:

1. Select the saved project containing the current scene.
2. Supply a known trusted, already-built Windows x86-64 `room_escape.exe`.
3. Use `Check runtime SHA-256` to measure it, or supply its expected hash.
4. Explicitly confirm `I trust this runtime executable`.
5. Enter an absolute, nonexistent output directory whose parent already exists.
6. Use `Export Windows game` and wait for the success or failure message.

Measuring a file never checks the trust box. A hash, PE header, declared revision,
or matching smoke output is not authentication, source attestation, or proof of
runtime capabilities. Do not use an executable supplied by an untrusted party.
Closing the export window does not cancel a started export. Work continues in a
background thread; the result is collected when the editor updates.

## Exported game

The bundle contains a fixed interactive `run-room-escape.cmd` launcher,
`bin/room_escape.exe`, an admitted `project/` closure and `orr.export.json`.
The launcher does not forward command-line arguments. For diagnostic options,
invoke the executable directly:

```powershell
E:/Games/MyRoomExport/bin/room_escape.exe --project E:/Games/MyRoomExport/project --headless --ticks 0
```

Moving the complete bundle preserves its relative layout. It does not require
the original project, package source directories, Cargo or the engine checkout
to find its project assets. The supplied runtime may still require its normal
Windows system/graphics dependencies. The exporter does not install or build
those dependencies and does not create an installer or signed release.

## Admission and publication

This Windows adapter reuses the existing export admission/profile authority
without editing the reserved Linux exporter files. Only the active package
closure, package manifests/licenses, selected scene and supported sidecars are
copied. Unreferenced project files, source checkouts and credentials are not
recursively copied. Package identity, asset hashes, entry schema and the compiled
runtime inventory must pass the existing project admission. Windows-specific
native path checks reject reparse components in each admitted closure path
before staging; admission may already have read that path. This is not a
read-isolation sandbox.

The editor supplies the host's fresh saved scene path and checksum. Both must
match the admitted Room project before staging starts. Scene Save As does not
rewrite the project's entry automatically: exporting a different current scene
is rejected rather than silently exporting the project's old entry.

A sibling temporary directory is populated with exclusive file creation. The
copied project is admitted again, then the copied executable runs directly
(without a shell) with `--headless --ticks 0`. Its stdout must exactly match
the staged project's expected output. The smoke has a ten-second deadline and
one-MiB limit per output stream. The runtime's startup precedes assignment to a
Windows Job Object, so this mechanism is not containment for hostile code.
Pipe readers are bounded even if a child retains an inherited pipe.

Source bytes, staged bytes, file inventory and hashes are checked again before
a native no-replace directory move publishes the result. Existing destinations
are never overwritten or repaired. Temporary-stage cleanup is best effort;
inspect leftovers after an interruption. Source/parent preflight and rechecks
detect ordinary changes, not hostile concurrent filesystem races or power-loss
durability. Do not edit the project concurrently with export.

`orr.export.json` names the Windows profile/target, initial checksum, relative
payload files, byte lengths and hashes. File mode `0` means no POSIX permissions
are asserted. Optional source revision is an operator declaration, not verified
provenance. The content digest uses the existing game domain plus the Windows
target and canonical payload. It is an inventory digest, not a signature.

## Command line

```powershell
cargo build --release --locked -p orr_sample --features project-export,room-project --bin orr_export_windows
target/release/orr_export_windows.exe --project E:/Games/MyRoom --runtime E:/Trusted/room_escape.exe --runtime-sha256 LOWERCASE_SHA256 --output E:/Games/MyRoomExport --trusted-runtime
```

The CLI selects the admitted Arena/Room profile, or Collect when compiled.
The editor integration deliberately accepts only Room. Default-off and other
platform builds retain their existing behavior; this is not a universal target
exporter. Remote CI results, local checks and development-branch integration
are separate states. Successful zero-tick admission/smoke is not evidence of
an interactive key-to-exit playthrough or complete engine/game production.
