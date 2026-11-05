# Stormglass: readonly live observation

The optional registered Stormglass host exposes its **same session** through the
existing language-neutral OVS1 viewstream. This complements the typed Rust bridge
and the separate WASM snapshot; it does not create another simulation or permit
runtime state edits. Focused CPU acceptance has passed; final-candidate verification is tracked in [#94](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/94#issuecomment-6649941508).

## Subscribe and decode

After ordinary ERP authentication and discovery, a client with `read` capability
can send:

```json
{"method":"watch.subscribe","params":{"topics":["viewstream"],"source":"sim","max_fps":30},"id":1}
```

Use `source:"sim"` for the play session, or `source:"view"` for play while active
and the authored preview in edit mode. `watch.viewstream.schema` describes the
same `CoopDefense` build, 60 Hz, one input player, reflected input layout and
four-byte command encoding. Two boards do not mean two external input players.
The transport then carries OVS bytes, or `watch.viewstream` hex on plain TCP.
See the existing [wire format](view-stream.md); this change adds no RPC or format
version. The registered listener remains opt-in and retains token/capability rules.

Dispatch by message type. Type 1 is a complete current-state frame. Type 2 is an
event batch: schema kind 0, `defense_event_opaque`, has 48-byte opaque payloads.
Ignore these events for this observation contract; they are not action receipts.
Frames may be paced, skipped or superseded. Replace the observed state rather
than assuming every intermediate transition or event was delivered.

## Schema-described state

All semantic properties are exact `u32` words. Decode property order from the
schema, not a compiled game struct or assumed byte offset.

- `run`: `playing`, `won`, `lost`, `run_tick`, `generation`, `last_sequence`,
  `wave`, `last_wave`, `reactor_hp`, `max_reactor_hp`, `gold_board_0/1`,
  `investment_board_0/1`, `upgrades_board_0/1`, `board_cols`, `board_rows`,
  `checksum_lo`, `checksum_hi`.
- `cell.empty`, `cell.rivet`, `cell.mortar`, `cell.prism`, `cell.condenser`,
  `cell.haste`, `cell.miner`: `board`, `cell`, `unit_id`, `tier`, `cooldown`.
  Every frame contains all 24 cells. Empty cells expose zero actor/tier/cooldown,
  even when the simulation pool retains retired values.
- `enemy.normal`, `enemy.fast`, `enemy.armored`, `enemy.horde`, `enemy.boss`:
  `enemy_id`, `lane`, `progress`, `hp`, `max_hp`, `slow_ticks`. Only active enemies
  appear, at most 64. There are no projectile-state records or cost advice.

Reconstruct the checksum with integer operations: `lo | (u64(hi) << 32)`.
The run record uses reserved ID `0x00000000ffffffff`; it is metadata, not an
editable ECS entity. Other record IDs identify fixed ECS pool slots. Logical
unit/enemy IDs identify actors within the current game generation and established
session/timeline, not globally. A merge or slot reuse must not revive a retired
actor. Simple Y-up markers with `prev == cur` and no interpolation aid inspection;
they do not reproduce native artwork, animation or framebuffer rendering.

## Observe outcomes without replacing receipts

`playing` means the game phase, including when the host is paused. OVS tick is the
engine tick; `run_tick` resets on Restart and can stop at a terminal outcome.
Generation may repeat after a new session or rewind. OVS sequence counts produced
frames, not commands or per-client delivery.

Use `editor.gameplay.status` for command consumption and neutral completion.
Then inspect current gold, cells, enemies or terminal flags for the visible
outcome: a consumed action can fail to purchase or merge. `last_sequence` is
corroboration, not a receipt. OVS does not carry editor binding/play/timeline epoch
fences and cannot promise atomic command-to-frame matching. Matching tick/checksum
supports content comparison only within an independently established unchanged
session. Rebaseline and rediscover after lifecycle changes or reconnect.

## Acceptance boundary

The original CPU ERP tests failed in both edit/view and active Play/sim because
the host had no configured stream; that evidence remains preserved. CPU acceptance
decodes schema/wire only, retains readonly authorization and authored source/history,
and exercises action versus no-op, economy, move/merge, pause/step, restart and
natural terminal outcomes. Producer tests separately check schema/identity, inactive normalization,
exact checksums, bounded records, deterministic projection and source immutability.
No socket, physical input, native-window, browser, GPU/device or full-workspace
acceptance follows from these CPU tests.
