# Optional input actions (bounded #106 slice)

`orr_input` is GPU-free, view-side Rust with no dependency on simulation, winit,
fonts, or renderer. A game chooses action names (`move_left`, `jump`, `interact`,
`pause`, etc.); its adapter converts held states or explicit tick samples into
its existing canonical input/commands. It does not modify replay formats.

## Contract

- Version 1 JSON, maximum 64 KiB read **before** parsing; 1–64 actions, ASCII
  identifiers of 1–64 bytes, 0–8 bindings/action (empty means unbound)
- Duplicate names or physical bindings (within or across actions) are rejected;
  multiple distinct bindings of one action OR together; opposing digital axes
  cancel. Unknown JSON fields/versions, invalid button indices are rejected
- OS repeats do not generate edges or resurrect cleared controls. Complete taps
  retain both pressed/released flags until `sample_tick`; multiple taps coalesce
- `state` is non-consuming. Call `sample_tick` only when the consumer actually
  accepts a tick, never merely because a render frame happened
- Focus/UI blocking clears held state and pending presses. Resume requires fresh
  presses. UI-consumed releases still release gameplay; consumed presses do not
  enter gameplay. Platform adapters should mark synthetic focus-gain presses as
  consumed. Gamepad disconnect removes that device's held buttons
- `replace_map` validates before replacing; invalid remaps preserve previous map
  and runtime state. Successful remaps clear held state and pending edges
- `load_file` rejects pre-existing special files, oversized files and direct
  symlinks before opening, then rechecks file metadata and bounds reads. Concurrent
  hostile path replacement is outside this local-filesystem boundary
- `save` encodes before writing; writer I/O failures can leave a partial writer.
  `save_new` uses same-directory staging, sync, and atomic hard-link publication,
  never overwrites a destination, and removes staging on failures. Unsupported
  filesystems fail explicitly. Rebinding can save to a new file then load it on
  the next launch; no automatic overwrite/background settings writes

## Support matrix

| Input | Core | Arena adapter |
|---|---|---|
| Physical keyboard A–Z/arrows/Space/Escape/Enter/Tab | yes | winit |
| Mouse buttons 0–7 | yes | winit (left/right/middle/back/forward/other 5–7) |
| Digital gamepad buttons 0–31 + session device ID | yes | unsupported, config rejected |
| Analog axes/deadzones, text/IME, touch | not implemented | not implemented |

## Real Arena consumer

Build `orr_sample --features input-actions` (composes with `sprites`). Without
this opt-in feature, the sample retains its existing key controls. Headless sim
crates acquire no input/GPU dependency.

```
cargo run --release -p orr_sample --features input-actions --bin arena -- --save-input-bindings my-bindings.json
# Edit the JSON: retain all seven Arena action names, change their bindings.
cargo run --release -p orr_sample --features input-actions --bin arena -- --input-bindings my-bindings.json
```

Default WASD/arrows move, Space holds fire, Escape quits. P toggles **local input
blocking only**; the bot/network simulation continues. The title reports input
blocking. Pausing/resuming/focus loss neutralizes gameplay until fresh presses.
`ArenaControls::set_ui_capture` and per-event `consumed` provide an explicit
integration boundary for future menus/text fields; this slice has no UI widget
or IME implementation. A consumed mouse click mapped to fire is tested.

Arena deliberately retains the Bridge's **latest held-state** contract and its
canonical `Keys::to_input()` conversion, including held fire every tick. It does
not use pressed edges as one-shot commands and does not promise delivery of a
short tap between Bridge updates. Core `sample_tick` behavior must not be
confused with tick acknowledgement from the threaded/network Bridge.

Tests cover the GPU-free contracts, malformed persistence, remapped Arena
canonical bytes, and identical real loopback-Bridge checksums. These are not
physical keyboard/mouse/gamepad, graphical menu, font, or Korean UI acceptance.
