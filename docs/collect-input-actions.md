# Optional Collect keyboard action maps

The standalone `collect_dodge` player accepts an optional keyboard map when built
with `orr_sample/input-actions`. It uses the existing version-1 `orr_input`
format and introduces no dependency, simulation schema, project manifest field,
editor control change or Arena control change.

Without a map, the defaults and window-title hint are unchanged: WASD/arrows
move, Space restarts and Escape quits. `collect_view::title(frame)` and
`collect_app::run_window(project)` retain their default behavior. Custom window
maps display all six admitted actions and their actual physical bindings in the
title; an empty binding list is shown as `Unbound`.

## Print, edit and launch

```sh
cargo build --locked -p orr_sample --features collect-dodge,input-actions --bin collect_dodge
# This command prints JSON to stdout; it neither opens a project nor writes a file.
target/debug/collect_dodge --print-default-input-map
# Optional POSIX shell capture: noclobber protects an existing destination.
( set -C; target/debug/collect_dodge --print-default-input-map > collect-input.json )
target/debug/collect_dodge --project assets/collect_dodge_project --input-bindings collect-input.json
```

`--print-default-input-map` must be used alone. `--input-bindings FILE` requires a
project and is window-only. Both options explicitly fail when `input-actions` is
not compiled; they never silently fall back to defaults. Enabling `collect-ui`
already enables `input-actions`, as it did before this change.

For example, replace the map with I/J/K/L movement, Enter restart and Q quit:

```json
{
  "version": 1,
  "actions": [
    { "name": "left", "bindings": [{ "device": "Keyboard", "key": "j" }] },
    { "name": "right", "bindings": [{ "device": "Keyboard", "key": "l" }] },
    { "name": "up", "bindings": [{ "device": "Keyboard", "key": "i" }] },
    { "name": "down", "bindings": [{ "device": "Keyboard", "key": "k" }] },
    { "name": "restart", "bindings": [{ "device": "Keyboard", "key": "enter" }] },
    { "name": "quit", "bindings": [{ "device": "Keyboard", "key": "q" }] }
  ]
}
```

Bindings use physical key identities, not typed characters or text-layout input.
Supported keys are A–Z, the four arrows, Space, Escape, Enter and Tab. Their JSON
names are lower snake case, such as `a`, `arrow_left` and `escape`.

The file must contain exactly `left`, `right`, `up`, `down`, `restart` and `quit`.
Order is irrelevant. Each action may have zero to eight bindings; empty lists
explicitly unbind an action. A physical key may appear only once across the whole
map. Mouse/gamepad bindings, unknown fields, duplicate actions, unknown actions,
unsupported keys, invalid versions and malformed JSON are rejected. This player
has no mouse/gamepad movement adapter.

The shared bounded loader admits only regular files at most 64 KiB, rejects
direct symlinks and special files, and bounds bytes again while reading. It does
not promise an adversarial concurrent filesystem-replacement sandbox. The
entire map is loaded and validated before opening the project, creating a
simulation host, opening a window or starting GPU work. There is no partial map
application or automatic save. Reopening the same file restores the same map;
there is no hot reload. Maps are supplied as explicit CLI files and are not
silently added to project packages or exporter launchers.

## Input lifecycle contract

The adapter changes physical key lookup only. The existing canonical
`CollectInput` and local Bridge remain the simulation boundary.

- Movement ORs aliases for one direction; opposite directions cancel per axis
- A fresh physical press of any restart binding queues one restart, including a
  second distinct restart binding pressed while the first remains held. This is
  a per-physical-press queue, not an ORed action-held transition
- Up to eight restart presses await delivery; additional presses coalesce at the
  cap. A press/release between simulation ticks is retained
- Only an advanced authoritative simulation tick acknowledges a restart. Queued
  presses are separated by a simulation-observed neutral sample; zero-tick
  render updates consume neither a restart nor its neutral rearm
- Once the queue is empty, any still-held restart alias supplies held state;
  holding or repeating it does not restart the game every tick
- Focus loss clears held controls and undelivered restarts. Synthetic presses,
  repeat-only presses and held keys crossing focus/capture boundaries do not
  become fresh gameplay presses; release and a fresh press are required
- UI capture blocks new gameplay input without cancelling a previously admitted
  restart. Existing authored Play/Restart buttons retain their acknowledged
  restart behavior independently of physical key mapping
- Quit retains the existing system-control policy: a focused, non-synthetic
  press exits even during gameplay UI capture. Repeated quit presses are also
  exit requests. Unfocused or synthetic quit presses do not exit
- Keys omitted from a replacement map lose their old roles, including Space and
  Escape. The normal OS close request remains available if quit is unbound

Headless `--hold right,up,restart` supplies semantic simulation input. It does
not emulate keys, and combining it or any headless run with `--input-bindings`
fails explicitly. A headless result or software-GPU screenshot is not evidence
of native window keyboard input.

## Verification scopes

The new focused tests exercise the exact production `Keys::keyboard_event`
policy used by the native event handler, then deliver canonical inputs through
real `InProc<CollectDodgeV1, PlayHost<_>>` Bridge sessions. They compare default
and remapped input/tick/Frame-checksum/run-state traces through collect, win,
subtick restart, neutral separation and quit policy. They also cover obsolete
keys, independent aliases, two restart aliases, the queue cap, repeats, focus,
synthetic input, capture and reloading unchanged file bytes.

The CLI integration launches the built executable, validates actual stdout and
exit status, tests feature-disabled and headless refusals, proves malformed map
admission precedes project opening, and checks that admission leaves map and
project bytes unchanged. These tests intentionally do not open a native window.
The existing title API/read-only mapping tests verify custom and default hints.

```sh
cargo test --locked -p orr_sample --features collect-dodge,input-actions --lib collect_app::input_actions::tests
cargo test --locked -p orr_sample --features collect-dodge,input-actions --lib collect_app::input_runtime_tests
cargo test --locked -p orr_sample --features collect-dodge,input-actions --lib collect_app::tests
cargo test --locked -p orr_sample --features collect-dodge,input-actions --lib collect_view::
cargo test --locked -p orr_sample --features collect-dodge,input-actions --test collect_input_cli
cargo test --locked -p orr_sample --features collect-dodge --test collect_input_cli
cargo test --locked -p orr_sample --features collect-ui --lib collect_app::tests
```

Commands are verification instructions, not a claim that they passed on every
platform. The dedicated workflow records nonzero exact test inventories and
keeps build, lint, CLI and test logs. The adapter inventory is 16 on Linux and 15
on Windows (one FIFO rejection test is Unix-only); runtime input tests are eight
on either. CLI inventory is five with mapping enabled and three without it.
Existing Collect runtime policy tests are nine without authored UI and eleven
with it; the view scope has two tests.

Native-window acceptance remains a separate scope: open an authored project,
use remapped movement to collect and win, restart with the remapped key, confirm
old keys are inert, check focus/UI capture and verify remapped quit. Close and
relaunch with the same file, compare source bytes, and verify the custom title.
This requires a real native display and physical/native event route. Do not
label the automated Bridge/CLI checks or headless rendering as that result.

Related contracts: [authored Collect player](collect-dodge-authored.md),
[authored UI](collect-authored-ui.md), [shared input format](../crates/orr_input/README.md).
