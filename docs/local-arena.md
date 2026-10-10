# Local Arena in the editor

Start an Arena editor without a separate remote host:

```sh
cargo run --release -p orr_editor -- --game arena
```

This opens `scenes/arena_blank.scene.yaml`. To open a saved Arena scene, add
`--scene <file>`. `--game physics`, or omitting `--game`, keeps the existing
PhysGame startup and `physics_demo.scene.yaml` default. An explicit `--game`
cannot be combined with `--connect`: an attached editor discovers its game
from the running host instead.

For Rust API callers, `HostSpec::Local` keeps its existing fields and PhysGame
behavior. The new `HostSpec::LocalGame` variant requires an additional arm in
external exhaustive matches. `cli::Args` also has a new public `game` field;
external struct literals must supply it, or use `Args::default()` and set the
desired fields.

The local Arena host uses seed 42, two player input slots, and 60 ticks per
second. It installs the Arena structured input and existing managed input
protocol before the editor connects. The editor remains an ERP client; scene
edits and play use the same host document, transaction history, and input
ownership rules as an attached editor.

## Create and edit players

In Edit mode, choose **Keyboard player slot** 0 or 1, then **+ Player**. The
player appears at the viewport center, with `Position` and `PlayerTag` only,
and becomes the selection. The chosen slot must be unused. Select the player
and edit `Position.pos` in the inspector or move it in the viewport. Creation
is one undo entry; a position gesture is another. Undo and Redo use the existing
ERP history.

Player creation checks fresh host state and a bounded `PlayerTag` query inside
its own transaction. It refuses a slot outside the host's configured player
count, non-finite or unrepresentable positions, duplicate occupied slots, and
an existing layout with duplicate or out-of-range player slots. A refusal
rolls back only a transaction that this operation opened. It does not roll
back another client's transaction.

These checks govern the editor's **+ Player** operation. They do not add a
general scene-import or ERP validation policy: an imported scene or a direct
ERP edit can still contain a layout that player creation refuses. Viewer and
proposal-preview scenes are read-only for this operation, and players cannot
be created during Play.

Save or Save As writes normal scene YAML. Reopen it with `--game arena`. Open
and Save As also update the local host's scene path, so Restart reopens that
file as Arena. Unsaved changes are lost on Restart, as with the existing local
PhysGame host.

## Play and input ownership

Press Play, select the desired keyboard player slot, and press **Take control**.
That explicit action focuses the viewport and claims the existing managed
input lease. Merely focusing the viewport or starting Play does not acquire
control. While control is active, use WASD or the arrow keys to move and hold
Space to fire. Escape or **Release control** releases it. Losing viewport
focus, pausing, seeking, or stopping also releases control; another explicit
Take control action is required to acquire it again.

Pause preserves the play session for inspection and stepping. Stop returns to
the authored Edit scene and retains the play recording for `verify.self` with
`last_play` and `recording_matches`. Play input does not rewrite the authored
scene. Remote replay Viewer and proposal-preview input restrictions are
unchanged.

## Validation scope

`crates/orr_editor/tests/local_arena.rs` exercises local game selection, player
creation and refusals, transaction ownership, position history, save/reopen/
restart, the egui player button, and managed movement/fire/recording. Existing
dependency checks keep simulation ownership outside the editor.

The existing `arena_native_window_smoke` also launches the editor with
`--game arena` and checks its own ERP framebuffer PNG in Edit and paused Play,
alongside its original remote Arena and local PhysGame assertions. Its native
window guards and required Linux CI policy remain in force. A platform guard
skip supplies no actual PNG evidence, and an egui harness test supplies no
native-window evidence. The framebuffer path captures the editor window; it
does not capture the operating system desktop.
