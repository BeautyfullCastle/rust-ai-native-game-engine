# Orrery view stream

A language-neutral description of what a view shows of a simulation, so a view
written in C#, C++, GDScript, or running on a console or in another engine, can
display an Orrery simulation and drive it **without linking Rust types or
reading a Rust `Frame`** (design doc decision 12).

The simulation and its `orr_view::Extractor` run on the host side. The stream
carries the extractor's output as bytes: one fixed-size record per drawable
entity with its transform at the previous and the current tick. The view
interpolates with its own clock.

- Format version: **1** for 2D games, **2** adds the 3D frame (message type 3, below). Version 1
  messages are unchanged byte for byte; a version 1 reader refuses version 2 messages
  ("unsupported version") instead of misreading them. Everything is **little-endian**. Floats
  are IEEE 754 binary32.
- Crates: `orr_viewstream` (format, schema, producer), `orr_ffi` (C ABI,
  `crates/orr_ffi/include/orrery.h`), `orr_remote` (the `viewstream` topic).
- This is a view-boundary format: the sim stays integer-only. The conversion
  from `FP` to `f32` happens once, in the extractor, on the host.

Four ways to get the same bytes:

| Way | For | Entry point |
| --- | --- | --- |
| C ABI | a view in the same process (Unity plugin, Unreal module, GDExtension, console title) | `orr_host_open` (local PhysGame), `orr_yard3d_host_open_v1` (local Yard3D), `orr_client_open` (PhysGame multiplayer), `orr_view_poll` |
| WebSocket | a view in another process, machine or devkit | `watch.subscribe {"topics":["viewstream"]}` |
| Plain TCP | scripts, tools, anything without a WebSocket library | same call, frames arrive as hex in JSON |
| Rust | `orr_bridge` users | `ViewStreamSource::pump(&mut bridge)` |

The stream is the same whether the simulation is a local play session or a **multiplayer client**
that predicts and rolls back: see "Multiplayer (client sessions)" below. The C ABI
(`orr_client_open`) and a headless ERP host in client mode (`orr_remote_host --join`) open client
sessions; the ERP `viewstream` topic of a normal `orr_remote` host serves its single-peer play
session (no rollback, every event verified).

## Snapshot/checksum compatibility

The ORRF simulation snapshot version is independent of this view-stream format. The
ORRF v2 checksum change does not alter OVS1 record layouts or the C ABI version, but
simulation checksums returned by client sessions change and old/new simulation builds
must not share a room. See [Frame and replay compatibility](frame-compatibility.md) for
old snapshots, replay keyframes, build IDs, and upgrade requirements.

## Messages

| Message | Encoding | When |
| --- | --- | --- |
| `Schema` | JSON text | once per connection, before the first frame |
| `ViewFrame` | binary, type 1 | per published tick of a 2D game |
| `EventBatch` | binary, type 2 | events since the last batch (2D and 3D) |
| `ViewFrame3` | binary, type 3 (format version 2) | per published tick of a 3D game |

### Binary preamble (every binary message)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | magic `"OVS1"` (`4F 56 53 31`) |
| 4 | 2 | version (`u16`): 1 for types 1 and 2, 2 for type 3. A reader refuses a version it does not know. |
| 6 | 1 | message type (1 = ViewFrame, 2 = EventBatch, 3 = ViewFrame3) |
| 7 | 1 | flags (ViewFrame, ViewFrame3) / 0 (EventBatch) |

### ViewFrame (type 1)

Header, 56 bytes:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 8 | preamble (flags at offset 7, see below) |
| 8 | 8 | `tick` (`u64`): the predicted head tick the records show |
| 16 | 8 | `verified_tick` (`u64`): the newest fully confirmed tick (equals `tick` without prediction) |
| 24 | 8 | `seq` (`u64`): counts up by one per frame the producer built; a gap means frames were skipped |
| 32 | 8 | `rollback_from` (`u64`), 0 unless the rollback flag is set |
| 40 | 8 | `rollback_to` (`u64`), 0 unless the rollback flag is set |
| 48 | 4 | `entity_count` (`u32`) |
| 52 | 4 | `props_bytes` (`u32`): size of the property section |

Then `entity_count` records of 48 bytes, then `props_bytes` bytes of properties.
Total size = `56 + 48 * entity_count + props_bytes`.

Frame flags (byte 7):

| Bit | Name | Meaning |
| ---: | --- | --- |
| 0 | `rolled_back` | a rollback corrected the past since the previous frame sent; `rollback_from..rollback_to` is the range resimulated. Positions of those ticks changed: an interpolating view should smooth the correction (`orr_view` does this with an error offset that decays). |
| 1 | `discontinuity` | the timeline jumped (seek, branch, new session): show this frame as it is, reset interpolation and smoothing. `prev` equals `cur` in every record. |
| 2 | `paused` | the simulation is paused: the tick does not advance by itself. |
| 3 | `events_reset` | presentation event history has a gap. Clear all pending predicted sound/VFX handles and event-key tables before ingesting this full baseline. Also carries `discontinuity`, with `prev == cur`. |

#### Entity record (48 bytes)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 8 | `id` (`u64`): `entity index \| version << 32`. Stable for the life of the entity; a reused index gets a new version. |
| 8 | 2 | `kind` (`u16`): game-defined, see the schema's `kinds` |
| 10 | 1 | `shape`: 0 circle, 1 quad, 2 capsule |
| 11 | 1 | `mode`: interpolation mode, 0 prediction, 1 snapshot, 2 none |
| 12 | 4 | `size` (`f32`): radius (circle), half width (quad), half segment length (capsule) |
| 16 | 4 | `half_y` (`f32`): quad half height (0 = square, use `size`), capsule radius |
| 20 | 4 | `rgba` (4 x `u8`) |
| 24 | 12 | `prev`: x, y, rotation (3 x `f32`) at tick `tick - 1` as the sim knows it now |
| 36 | 12 | `cur`: x, y, rotation (3 x `f32`) at tick `tick` |

World units are the game's, y up, rotation in radians counter-clockwise. The
view computes `pos = prev + (cur - prev) * alpha` with its own `alpha` in
`0..1` (rotation along the shorter way round), exactly like `orr_view`'s
`Transform2::lerp`. A `mode` of 2 (none) means the entity does not move
(`prev == cur`); a view shows mode 1 (snapshot) like mode 0 unless it buffers
confirmed frames itself (this stream carries the predicted transforms only).

Shapes: a **capsule** is a segment along the entity's local x axis (half length
`size`) grown by a radius (`half_y`). The extractor already folds any offset of
the segment into `rot`.

#### Property section

Custom per-entity properties, declared per kind in the schema (`kinds[].props`,
each a 32-bit word: `f32`, `u32` or `i32`). For each entity, in record order,
its kind's words follow, concatenated with no padding. A reader that does not
know the kinds can skip the section with `props_bytes`. The physics demo has
`speed` (f32) on `dynamic` and `slot` (u32) on `paddle`.

#### Spawn and despawn

The stream has no spawn or despawn messages: they are **derived from id
sets**. An id in this frame that was not in the previous frame is a spawn; an
id that disappeared is a despawn. Ids include the version, so a reused entity
index is a despawn plus a spawn, never a teleport. After a `discontinuity`
frame (seek, new session), diff against the new frame only. After a
`rolled_back` frame, diff as usual: entities the rollback removed despawn,
entities it created spawn.

### ViewFrame3 (type 3, format version 2)

For 3D games (`orr_physics3d`, anything with positions in 3 dimensions). The **header is the
ViewFrame header** (56 bytes, same fields, flags and meaning; only the version is 2 and the type
is 3). Then `entity_count` records of **88 bytes**, then `props_bytes` bytes of properties, as for
2D. Total size = `56 + 88 * entity_count + props_bytes`. Spawn, despawn, discontinuity and
rollback handling are exactly the ones described for 2D.

#### 3D entity record (88 bytes)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 8 | `id` (`u64`): `entity index \| version << 32` |
| 8 | 2 | `kind` (`u16`): game-defined, see the schema's `kinds` |
| 10 | 1 | `shape`: 0 sphere, 1 box, 2 capsule, 3 plane |
| 11 | 1 | `mode`: interpolation mode, 0 prediction, 1 snapshot, 2 none (as in 2D) |
| 12 | 12 | `size`: 3 x `f32`, see below |
| 24 | 4 | `rgba` (4 x `u8`), linear RGB, straight alpha |
| 28 | 1 | `roughness` (`u8`, 255 = 1.0) |
| 29 | 1 | `metallic` (`u8`, 255 = 1.0) |
| 30 | 1 | `style_flags`: bit 0 = checker pattern (for ground planes) |
| 31 | 1 | reserved, 0 |
| 32 | 28 | `prev`: position x, y, z (3 x `f32`) then orientation quaternion x, y, z, w (4 x `f32`) at tick `tick - 1` as the sim knows it now |
| 60 | 28 | `cur`: the same at tick `tick` |

World units are the game's, **right handed, y up**. The quaternion is a unit quaternion
`x*i + y*j + z*k + w` that rotates local to world. The view computes
`pos = prev + (cur - prev) * alpha` and `rot = slerp(prev, cur, alpha)` along the **shortest arc**
(negate one quaternion when their dot product is negative; `q` and `-q` are the same rotation),
exactly like `orr_view`'s `Transform3::lerp`. Rollback smoothing works like 2D with a position
offset plus a correction quaternion (`rot_offset * rot`, decayed with `slerp(identity, rot_offset, keep)`).

| Shape | `size[0]` | `size[1]` | `size[2]` |
| --- | --- | --- | --- |
| sphere | radius | 0 | 0 |
| box | half extent along local x | half y | half z |
| capsule | radius | half length of the segment (local y axis) | 0 |
| plane | half extent x | 0 | half extent z (a horizontal rectangle in the entity's local xz plane, normal local +y) |

A capsule is the segment from `(0, -size[1], 0)` to `(0, size[1], 0)` in local space, grown by the
radius. The `kind` and the properties work as in 2D. The schema of a 3D game has `"version": 2`
and a `frame3d` object (`record_len` 88, `message_type` 3, the shape names and style flags) in
place of `frame`. Events use the same `EventBatch` (version 1) as 2D games.

Golden bytes (a capsule with a checker style, `orr_viewstream/tests/format.rs`
`golden_bytes_of_a_tiny_frame3`) pin the layout. `tools/viewstream_client.py` decodes it
(`decode_frame3`, or `decode_message` for either kind). The producer is
`orr_viewstream::ViewStreamSource3` with an `orr_view::Extractor3`.

### EventBatch (type 2)

Sim events with the three states of the bridge (design doc section 5). Events
are **queued, never replaced** by newer ones, unlike frames.

Header, 16 bytes: preamble (8, flags 0), `count` (`u32`), 4 reserved bytes.
Then `count` records, each starting on an 8-byte boundary:

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 8 | `tick` (`u64`): the tick that emitted the event |
| 8 | 4 | `system` (`u32`): index of the emitting system |
| 12 | 4 | `seq` (`u32`): sequence within system and tick |
| 16 | 1 | `state`: 0 predicted, 1 verified, 2 canceled |
| 17 | 1 | reserved |
| 18 | 2 | `event_type` (`u16`): game-defined, see the schema's `events` |
| 20 | 4 | `payload_len` (`u32`) |
| 24 | n | payload (the game's event bytes), zero-padded to a multiple of 8 |

`(tick, system, seq)` is the event's **deterministic key**: the same across a
rollback resimulation. A `verified` or `canceled` record matches the
`predicted` record with the same key. `predicted` events may be canceled by a
later rollback (make reactions reversible: sounds, particles); `verified`
events are final and announced once; `canceled` carries no payload. A
single-machine play session has no rollback, so its events are all `verified`. A client
session (below) sends `predicted` first, then `verified` or `canceled` with the same key. The
physics demo emits a `shot` event (type 1) when a paddle fires.

## Schema (JSON)

Sent once per connection, before the first frame, as compact JSON. Keys stay in
the order below, so even a `strstr` reader can find them. Example (shortened):

```json
{"format":"orrery.viewstream","version":1,"game":"PhysGame","build_id":"0x...","tick_rate":60,"player_count":2,
 "endian":"little",
 "fixed_point":{"type":"q48.16","raw":"i64","frac_bits":16,"note":"..."},
 "frame":{"magic":"OVS1","header_len":56,"record_len":48,"shapes":["circle","quad","capsule"],
          "interp_modes":["prediction","snapshot","none"],"flags":{"rolled_back":1,"discontinuity":2,"paused":4,"events_reset":8}},
 "kinds":[{"id":0,"name":"static","props":[]},
          {"id":1,"name":"dynamic","props":[{"name":"speed","type":"f32"}]},
          {"id":2,"name":"bar","props":[]},
          {"id":3,"name":"paddle","props":[{"name":"slot","type":"u32"}]}],
 "input":{"size":24,"fields":[{"name":"axis_x","offset":0,"size":8,"type":"fixed"},
                              {"name":"axis_y","offset":8,"size":8,"type":"fixed"},
                              {"name":"spin","offset":16,"size":4,"type":"i32"},
                              {"name":"buttons","offset":20,"size":4,"bits":{"shoot":1},"type":"flags"}]},
 "command":{"size":4},
 "events":[{"id":0,"name":"trigger","payload_size":12},{"id":1,"name":"shot","payload_size":12}]}
```

- `build_id`: hosts with the same build id simulate identically.
- `input`: the byte layout of the game's input type, from its `orr_reflect`
  descriptor, so the offsets are the real ones. A view sets an input by writing
  exactly `input.size` bytes. Field types: `u8`..`u64`, `i8`..`i64`, `bool`,
  `fixed` (an `i64` holding value x 65536), `fixed32` (an `i32`, same scale),
  `fixed_vec2`/`fixed_vec3` (2 or 3 `fixed`), `entity` (`u32` index, `u32`
  version), `enum` (`values`), `flags` (`bits`), `array` (`len`, `elem`),
  `struct` (`fields`). Unknown types are `opaque`.
- `command.size`: size of the game's command encoding (0 = none).
- `kinds`, `events`: the game's vocabulary.

## Multiplayer (client sessions)

A view can play a game on an `orr_server` room through the C ABI: `orr_client_open` joins as a
relay client (QUIC, or plain WebSocket; the same simulated latency, jitter and loss as the Rust
samples' `--sim-*` options, for testing), runs the prediction and rollback of `orr_session` on
the host thread, and feeds this stream from the Rust bridge's snapshots (`ViewStreamSource` over
`Threaded<PhysGame>`), exactly as `orr_sample --connect` does for its own window. What differs
from a local host:

- **`rolled_back`** is set on the first frame after a rollback, with `rollback_from..rollback_to`
  (the ticks that were resimulated, at most the prediction window). `tick` is the predicted head,
  `verified_tick` the newest tick every player's input is confirmed for; `tick - verified_tick` is
  the prediction depth. Positions of the resimulated ticks may have changed: blend the correction
  over about 100 ms (`orr_tui` does: it keeps where each entity was drawn and fades the difference
  out; positions only).
- **Events** arrive as `predicted` (the head simulated them from predicted inputs), then, with
  the same `(tick, system, seq)` key, `verified` (the confirmed inputs produced them) or
  `canceled` (a rollback found they do not happen). A view starts a sound or particle on
  `predicted` and takes it back on `canceled`. Events that only exist after a rollback arrive as
  new `predicted` records. In an uninterrupted stream, each settlement follows its `predicted`.
  A presentation overflow instead emits `events_reset`: clear all speculative handles and rebuild
  from that full frame. Missed transient effects are not replayed; late settlements belonging to
  the discarded baseline are suppressed. `verified` is final simulation state, not durable delivery.
- **Inputs**: `orr_set_input` works for the joined slot only (the server chose it; see the
  status). Commands go with the next submitted tick. Timeline controls (play, pause, step, seek,
  speed), `orr_erp_call` and listening sockets belong to a local host and return `ORR_ERR_ARG`.
- **Session status** is not part of the stream (the binary layout, and so the format version 1,
  is unchanged): `orr_session_status` fills a struct with the state (`CONNECTING`, `PLAYING`,
  `DISCONNECTED`, `FAILED`), the joined slot and player count, smoothed round trip time, input
  delay in ticks, head and verified tick, rollbacks, ticks resimulated, the range of the latest
  rollback, desyncs reported by the room, stalls and repeated inputs. A rollbacks-per-second or
  depth readout is computed by the view from it and from the frames. Joining returns at once
  (`CONNECTING`; the room starts when every player is in), or waits with `ORR_CLIENT_WAIT`; until
  it plays, polls say `ORR_NO_FRAME` and the schema `ORR_ERR_NOT_READY`. A lost connection turns
  the state to `DISCONNECTED` (the last frames stay readable); a refused or failed join to
  `FAILED` (the next view call returns `ORR_ERR_HOST` with the reason in `orr_last_error`).
- **Agreement**: `orr_confirmed_checksum(h, tick, &found, &sum)` gives the checksum of the
  confirmed state at a checkpoint tick (a multiple of the room's checksum interval, 30). It is
  the value the client reports to the server; two peers that agree on a tick have the same
  state there. A desync turns on `ORR_STATUS_DESYNC`.

### Client mode of the ERP host (`orr_remote_host --join`)

The same client session, out of process. `orr_remote_host --join HOST:PORT [--fingerprint HEX |
--insecure] [--ws] [--room N] [--slot N] [--sim-latency MS] [--sim-jitter MS] [--sim-loss P]
[--sim-seed N] [--connect-timeout S]` makes the host a relay client instead of a play-session
owner (the code is shared with the C ABI: `orr_sample::relay_view::RelayView`, so the bytes are
the same). It starts serving ERP once the room has started. Over the ERP socket (WebSocket or
plain TCP):

- `watch.subscribe {"topics":["viewstream"]}` streams the client's frames (with `rolled_back` and
  the range) and events (`predicted`, then `verified` or `canceled`), exactly as above. `max_fps`
  caps the frames per subscriber (a frame built while throttled is replaced by a newer one, so use
  a high cap, like 1000, when every rollback flag must be seen); event batches are never held
  back. `activity` also works; other topics answer `not_in_client_mode`.
- `session.status` (and `sim.state`, which answers the same in client mode) returns
  `{mode:"client", state, playing, slot, player_count, rtt_ms, input_delay, desyncs, head_tick,
  verified_tick, rollbacks, resim_ticks, last_rollback_from, last_rollback_to, stall_episodes,
  stalled_ms, repeats, confirmed:{tick, checksum}}`, the fields of `orr_session_status`.
- `sim.checksum {tick?}` returns the confirmed checksum of a checkpoint tick (default the newest),
  like `orr_confirmed_checksum`.
- `sim.input {player, input}` and `sim.command {player, command}` work for the joined slot only
  (another slot is `invalid params`).
- Scene edit, proposal, history, `sim.start/stop/play/pause/step/seek/speed` and the like are
  refused with `not_in_client_mode`; `rpc.discover` and `activity.list` work.

`orr_tui --connect ws://host:port` detects such a host (`sim.state` says `mode: "client"`), does
not try to start a session, polls `session.status` for the status line (slot, RTT, delay,
rollbacks) and smooths rollbacks from the frame flags like in `--server` mode. With `--headless
--ticks N --check-tick T` it prints the same `RESULT client ... checkpoint=T checksum=0x..` line.

`TickInputs` carries per-player flags for the game (`PlayerFlags`): `predicted` (this tick's input
of that player was not confirmed when the tick was simulated: a repeat of the last confirmed one)
and `disconnected` (the server confirmed the tick with nobody in the slot). `predicted` is a hint
for view-side logic only: a rule that depends on it would make peers differ. `disconnected` comes
from the server's confirmed bundle, so it is the same on every peer, and a prediction that
assumed the old value triggers a rollback like a wrong input.

## Driving the simulation

Views write the simulation in two ways only (the bridge rule): **input** (the
held input of a player, sampled once per tick) and **commands** (one-offs).
Timeline controls (play, pause, step, seek, speed) are the editor's play
controls. Over ERP these are `sim.input {player, input: <hex>}`,
`sim.command {player, command: <hex>}`, `sim.play`, `sim.pause`,
`sim.step {n}`, `sim.seek {tick}`, `sim.speed {permille}`; the C ABI wraps them
(`orr_set_input`, `orr_send_command`, `orr_control`).

## Socket handshake (ERP)

ERP is JSON-RPC 2.0 over a WebSocket (text messages) or newline-delimited JSON
on plain TCP, same port (`orr_remote_host`, or an `orr_ffi` host opened with
`ORR_HOST_LISTEN`). See `rpc.discover` for the method table.

1. Connect, authenticate (`?token=` in the URL or an `auth` message; a dev-mode
   loopback host needs none).
2. `{"jsonrpc":"2.0","id":1,"method":"watch.subscribe","params":{"topics":["viewstream"],"max_fps":60,"source":"sim"}}`
   - `max_fps` (default 60): at most this many frames a second per subscriber;
     a frame built while throttled goes out a moment later. A slow reader skips
     frames (never events) instead of making the host queue grow.
   - `source`: `sim` (default: the play session; nothing in edit mode) or `view`
     (the play frame while playing, else the scene's preview frame).
3. The response `{"result":{"topics":["viewstream"]}}` is followed by one text
   notification `{"method":"watch.viewstream.schema","params":<Schema>}`.
4. Then, per published tick:
   - **WebSocket**: one **binary** message per ViewFrame / EventBatch, exactly
     the bytes above (no extra framing).
   - **Plain TCP**: a text line `{"method":"watch.viewstream","params":{"encoding":"hex","data":"4f565331..."}}`;
     decode the hex to get the same bytes.
5. The view drives the sim with the calls above on the same connection.
   `watch.unsubscribe {"topics":["viewstream"]}` stops it.

`tools/viewstream_client.py` is a standard-library Python client of this
handshake (TCP, hex) that decodes frames; the test suite runs it against a host
and checks it reads the same records as the Rust decoder.

`crates/orr_tui` (`orr_tui`) is a terminal view that uses nothing but this
document: it depends on `orr_viewstream` without its `producer` feature (the
decoder, no simulation crates; `tests/deps.rs` enforces it), reads the stream
with a tiny WebSocket/TCP client (`--connect ws://host:port [--token t]`) or
through the C ABI loaded at run time (`--ffi [--lib path]`, calling the
functions of `orrery.h` through `extern "C"` declarations, feature `ffi`), and
builds input bytes from the schema's input layout. `--server HOST:PORT [--fingerprint HEX |
--insecure] [--sim-latency MS --sim-jitter MS --sim-loss PCT]` joins a multiplayer game
through the C ABI (`orr_client_open`): the status line shows slot, round trip time, input delay,
rollbacks per second and the depth of the last one, event counts by state (predicted, verified,
canceled and the transitions), and a rollback correction fades out over 0.1 s instead of
snapping. `--headless --server ...` plays a scripted player for `--ticks` and prints
`RESULT client slot=.. players=.. ticks=.. rolled_back_frames=.. max_depth=.. rtt_ms=.. predicted=..
verified=.. canceled=.. checkpoint=300 checksum=0x..`; every player of a room prints the same
`checkpoint=` and `checksum=` parts (the confirmed state at that verified tick). `--headless --frames N
--dump file` prints `RESULT entities=.. frames=.. fnv=0x..`, the same line the
C client and the Rust bridge produce for the same scenario.

The Yard3D socket TUI is a separate #13 follow-up; this C ABI change does not
add that consumer. Its scope is an XZ observer through
`orr_tui --connect ws://HOST:PORT`: `p`/space play or pause the timeline, `s`
steps once, `r` refits the view, and `q` quits. Gameplay input is unsupported,
including initial neutral input, held movement and fire; the 2D input encoder
must not produce YardInput. Interactive terminal validation remains pending
for that follow-up. 3D headless operation is unsupported, and rejection can
occur after a socket or session handshake. A dynamically loaded Yard3D FFI
TUI, Yard3D relay multiplayer, arbitrary scene loading and a new GPU renderer
remain separate work.

The hosts of the same tick share one encoded message, so two subscribers get
byte-identical frames (including `seq`).

## C ABI (`orr_ffi`)

Header: `crates/orr_ffi/include/orrery.h`. The library (`cdylib`: `.so`,
`.dylib`, `.dll`; `staticlib` for consoles) provides explicit built-in game
entries. `orr_host_open(scene_path, cfg)` remains the physics demo (`PhysGame`),
and `orr_client_open` remains its relay client. The additive
`orr_yard3d_host_open_v1(max_view_version, cfg)` opens a local Yard3D scene.
To compile in another game, copy the crate and replace a game factory with a
`LocalHost::spawn::<YourGame>` whose settings include its `view_stream` producer
(the extractor, the kinds, the schema). The header, the schema and the stream
are the same for every game.

```c
OrrHost* h = orr_host_open(NULL /* demo scene */, &cfg);   /* thread + paused session */
/* or, to play a multiplayer game: OrrClientConfig cc = {sizeof cc, ORR_CLIENT_WAIT, ...}; h = orr_client_open(&cc); */
size_t n = orr_schema_json(h, NULL, 0);                    /* size query */
char* schema = malloc(n); orr_schema_json(h, schema, n);   /* kinds, input layout */
orr_set_input(h, 0, input_bytes, 24);                      /* layout from the schema */
orr_control(h, ORR_CTL_STEP, 1);                           /* or ORR_CTL_PLAY */
const uint8_t* frame; size_t len;
if (orr_view_poll_ptr(h, &frame, &len) == ORR_OK) { /* decode, interpolate, draw */ }
orr_host_close(h);
```

| Function | Purpose |
| --- | --- |
| `orr_abi_version`, `orr_last_error` | version check; text of the last error on this thread |
| `orr_host_open(scene_yaml_or_NULL, cfg)` / `orr_host_close` | start / stop a host thread with a paused play session |
| `orr_yard3d_host_open_v1(max_view_version, cfg)` | deterministic built-in Yard3D local host; requires a reader supporting view format 2 |
| `orr_schema_json(h, buf, cap)` | the schema, NUL-terminated; returns size needed |
| `orr_host_url(h, buf, cap)` | `ws://` URL if opened with `ORR_HOST_LISTEN` (for out-of-process views) |
| `orr_view_poll(h, buf, cap, &written)` / `orr_view_poll_ptr(h, &data, &len)` | newest unread frame (copy / zero-copy); `ORR_NO_FRAME` if nothing new |
| `orr_events_poll(h, buf, cap, &written)` | oldest queued event batch |
| `orr_set_input(h, player, bytes, len)`, `orr_send_command(...)` | the two view-to-sim writes |
| `orr_control(h, op, arg)` | `PLAY`, `PAUSE`, `STEP n`, `SEEK tick`, `SPEED permille`, `BRANCH`, `RESTART` |
| `orr_erp_call(h, request_json, out, cap, &needed)` | the full ERP method set in process (`world.query`, `scene.save`, ...), for a foreign editor |
| `orr_client_open(cfg)` | play on an `orr_server` room (ABI 2): `OrrClientConfig` has the server address, transport, certificate fingerprint, room, slot, simulated latency/jitter/loss, timeout |
| `orr_session_status(h, &status)` | `OrrSessionStatus` of either kind of handle (state, slot, RTT, input delay, rollbacks, desyncs, ...) |
| `orr_confirmed_checksum(h, tick, &found, &sum)` | checksum of the confirmed state at a checkpoint tick (client sessions) |

### Built-in Yard3D through the C ABI

`orr_yard3d_host_open_v1(2, cfg)` creates the existing nonempty deterministic
Yard3D scene (24 initial raining bodies plus yard fixtures, default rain of
6 bodies/second, max 2500 entities, layout seed `0x5EED_CAFE`, session seed 42,
two players, 60 Hz, local build ID zero) and starts a
play session, paused unless `ORR_HOST_RUN` is set. `cfg` has the same optional listen/run settings as
`orr_host_open`; there is no scene-path argument. The handle's schema is
`game: "Yard3D"`, `version: 2`, with `frame3d` describing
message type 3 and 88-byte entity records. Existing copy and pointer polling
calls return those opaque bytes, including the usual flags and properties.

`max_view_version` is the highest view format the caller can decode. A value
below 2 returns NULL with an explanation from `orr_last_error` before a host
thread or listening socket starts. A higher maximum still selects this
entry's version 2 stream. The `_v1` suffix versions this entry's C contract;
it does not mean view format 1. Existing ABI version 2, structure layouts,
symbols and PhysGame behavior are preserved. When loading a library at run
time, resolve the new symbol explicitly: an older ABI2 library may lack it,
which is an unsupported feature, never a reason to call the PhysGame entry
and interpret its records as 3D.

Build the held input from the returned schema: YardInput is exactly 32 bytes,
with buttons at offset 0, zero padding at 4, three signed 32-bit origin
coordinates in centimetres at 8, and three signed 32-bit ray-direction
components scaled by 1000 at 20. Button bits are shoot/spawn box/spawn ball/
spawn capsule (1/2/4/8). `orr_set_input` uses that raw layout for slots 0 and 1;
the generic 24-byte PhysInput layout does not apply. The no-op command is
4 bytes. Timeline controls and ERP calls operate on the same local play
session. Seek replays its recorded history; this entry does not add a Yard3D
relay client or genuine late-input network rollback.

The dedicated external C consumer is `tests/c/yard3d_view_client.c`, run by
`cargo test -p orr_ffi --release --test yard3d_c_client` with
`ORR_REQUIRE_C_COMPILER=1` to require a real C compiler and shared library.
Header parity, existing `c_client` and `client_session` tests continue to
cover the original ABI and 2D consumers.

A host opened with `ORR_HOST_LISTEN` also serves ERP on a loopback port (for out-of-process
views and tools) **without authentication**: any local process can drive it. Use it for
development and devkits; `orr_remote_host --token ...` is the authenticated server.

Rules: every call returns a code (`ORR_OK` 0, `ORR_NO_FRAME` 1, negative =
error) and never panics across the boundary; every pointer is null-checked; a
buffer that is too small gets the size needed and nothing is written or
consumed. A handle is internally locked (calls from several threads run one
after another); `orr_host_close` must not race another call on the same handle.
A pointer from `orr_view_poll_ptr` is valid until the next `orr_view_poll*` on
the handle or close.

## Size

Measured (`cargo test -p orr_sample --release --test viewstream -- --nocapture bytes_per_tick`):

| Scene | Entities | Bytes per tick | At 60 Hz |
| --- | ---: | ---: | ---: |
| physics demo at start | 49 | 2 576 (56 header + 49 x 48 + 168 properties) | 155 KB/s |
| 1000 bodies + walls and paddles | 1 010 | 52 544 | 3.1 MB/s |
| 3D: 1000 bodies | 1 000 | 88 056 (56 header + 1000 x 88) | 5.3 MB/s |

Records are fixed-size and uncompressed. Not done yet, because the demo does
not need it: skipping static entities that did not change (a keyframe plus
deltas), quantized positions, lz4 (the ERP frame stream already uses lz4 for
full `Frame`s). A `max_fps` below the tick rate cuts the cost proportionally.

## Integrating an engine

What a Unity, Unreal or Godot view has to do, in order:

1. Load the library (P/Invoke, `FPlatformProcess::GetDllHandle`, GDExtension) and
   wrap the functions above; check `orr_abi_version` (2 for client sessions).
2. Read the schema once: entity kinds (to pick a prefab, mesh or material per
   kind) and the input layout (to build the input bytes).
3. Each render frame: `orr_view_poll_ptr`; if a frame came, diff its ids
   against the last one (spawn, despawn), keep `prev`/`cur` per entity and the
   time the frame arrived; draw at `alpha = (now - arrival) * tick_rate`
   (clamped to 0..1). On `discontinuity` reset; on `rolled_back` blend the
   correction over ~100 ms instead of snapping.
4. Each input sample (once per sim tick is enough): write the input bytes and
   `orr_set_input`.
5. Drain `orr_events_poll` for sounds and effects; treat `predicted` as
   reversible and `verified` as final. On frame flag `events_reset` (8), clear
   pending speculative effects and event-key history before accepting new events.
   Older clients that ignore unknown flags must be upgraded to support recovery;
   this flag does not alter the binary header or format version.
6. 2D shapes are circle, quad and capsule. A 3D game streams `ViewFrame3` (sphere, box, capsule,
   plane with position and quaternion): map them to the engine's meshes (or use `kind` to pick a
   prefab), draw with the engine's own lighting and slerp the rotation. Games with richer
   visuals add properties to their kinds.


## Presentation delivery recovery

For `InProc`/`Threaded`, view notifications now have a configurable bounded
mailbox and an explicit snapshot-based reset. See [bounded view recovery](view-recovery.md)
for the exact cursor, lifecycle-diagnostic, and durability contract. Recovery is
presentation-only; the authoritative simulation continues ticking. `RemoteBridge`
and the editor's ERP `LocalHost` path are not covered by the bounded mailbox.
The flag above signals a bridge-level reset, not network packet-loss recovery.
The binary frame carries no diagnostic count; Rust `ViewResync` exposes counts.
