# Create a Room project on Windows

The `orr_new_room` production CLI creates a new, admitted flat-room project
using the existing Room simulation, content package manager, static model
bindings and camera. It creates seven actors and two model bindings. The created
directory opens in the existing Room-enabled editor and runtime.

```powershell
cargo build --locked --release -p orr_sample --no-default-features --features project-create,room-project --bin orr_new_room
.\target\release\orr_new_room.exe --output 'C:\Games\New Room Ω' --seed room-01
orr_editor --room-project 'C:\Games\New Room Ω'
room_escape --project 'C:\Games\New Room Ω'
```

The older `room_create_windows` example remains as a compatibility entry point;
it delegates to the same creator implementation. To build and run that example
explicitly:

```powershell
cargo build --locked --release -p orr_sample --no-default-features --features project-create,room-project --example room_create_windows
.\target\release\examples\room_create_windows.exe --output C:\Games\NewRoom --seed room-01
```

Both `--output` and `--seed` are required exactly once. `--help` prints usage on
the supported configuration. The parent must exist and the destination must be
absent. Ordinary absolute drive paths support spaces and Unicode. UNC, device,
user-supplied verbatim, drive-relative, traversal, alternate-stream and reserved
device paths are rejected. Parent directories must be real non-reparse
directories. Seeds contain 1–128 ASCII letters, digits, dots, underscores or
hyphens; they namespace authored GUIDs and do not create a game/progress identity.

The same seed and tool version reproduce project bytes. Different seeds change
the authored GUID namespace while preserving the canonical initial Frame
checksum. The complete CC0 `sample-imported-scene` package, license and inert
`generate.py` are installed through `Project.install`; no script, runtime,
download or build is invoked during creation. Model bindings use actual installed
package identities. The source project, user settings and engine source are not
copied into the output.

Creation writes a unique staging directory under the chosen parent, verifies
the complete file/byte closure and production `PreparedProject` admission, then
moves the project subtree using Windows `MoveFileExW` with flags zero. Existing
files and directories are preserved, and there is no replacement or cross-volume
copy fallback. The Windows API documents directory moves on the same drive and
the replacement flag separately; this example does not claim crash atomicity or
power-loss durability. [Microsoft API contract](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-movefileexw)

Checks address ordinary filesystem changes, not hostile concurrent parent-path
replacement. Reparse attributes are rejected using Windows metadata, including
when opening staged files. Files use Windows default access control; Unix mode
0600/0644 semantics are not claimed. [Windows file attributes](https://learn.microsoft.com/en-us/windows/win32/fileio/file-attribute-constants)

## Verify the creator

```powershell
cargo test --locked --release -p orr_sample --no-default-features --features project-create,room-project --test room_creator_cli -- --test-threads=1
cargo test --locked --release -p orr_sample --no-default-features --features project-create,room-project --test room_create_windows -- --test-threads=1
$env:ORR_WINDOWS_ROOM_CREATOR = (Resolve-Path .\target\release\examples\room_create_windows.exe).Path
New-Item -ItemType Directory C:\Evidence\NewRoomCreation
$env:ORR_WIN_CREATE_EVIDENCE = 'C:\Evidence\NewRoomCreation'
cargo test --locked --release -p orr_sample --no-default-features --features project-create,room-project --test room_create_windows -- --ignored --exact actual_windows_creator_cli_persists_an_admitted_project --test-threads=1 --nocapture
```

The explicit CLI check requires the selected executable and a fresh, existing,
empty evidence directory; missing inputs fail it. Normal CPU tests verify
reproducibility, package/model/camera admission, failed creation preservation,
native no-replace publication, input paths, separate key/exit interactions with
actual movement, and edit/Undo/Redo/save/reopen. An ignored default CLI case is
not counted as executed CLI acceptance. The explicit check also writes
`room-session-oracle.json` beside, not inside, the project. It asserts key
collection followed by a separate exit interaction and records Frame checksums
at ticks 0, 20, 21, 62 and 63 for comparison with the same actual editor inputs.
Native editor and renderer checks must also be run explicitly before claiming
their acceptance.

This Windows command is separate from the existing Linux-only public creator.
The shared Linux creator, CLI, Cargo and library files retain their ownership;
this increment adds no public library API, dependency, schema or default feature.
The command and compatibility example fail before opening a project on other
targets or without both features.
The Room profile has no new HUD, checkpoint, skinning or progress support here.

The production folder exporter remains Linux x86-64-only. Creating and playing a
project on Windows does not establish Windows export or source-hidden execution.
Full #94 creation→save→play→export acceptance remains separate, as do current-head
mandatory CI, independent review and development integration. Original #2's
22-item denominator remains unchanged.
