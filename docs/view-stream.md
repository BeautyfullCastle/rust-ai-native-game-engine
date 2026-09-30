# Orrery view stream

A language-neutral description of what a view shows of a simulation, so a view
written in C#, C++, GDScript, or running on a console or in another engine, can
display an Orrery simulation and drive it **without linking Rust types or
reading a Rust `Frame`** (design doc decision 12).

The simulation and its `orr_view::Extractor` run on the host side. The stream
carries the extractor's output as bytes: one fixed-size record per drawable
entity with its transform at the previous and the current tick. The view
interpolates with its own clock.

- Format version: **1**. Everything is **little-endian**. Floats are IEEE 754 binary32.
- Crates: `orr_viewstream` (format, schema, producer), `orr_ffi` (C ABI,
  `crates/orr_ffi/include/orrery.h`), `orr_remote` (the `viewstream` topic).
- This is a view-boundary format: the sim stays integer-only. The conversion
  from `FP` to `f32` happens once, in the extractor, on the host.

Four ways to get the same bytes:

| Way | For | Entry point |
| --- | --- | --- |
| C ABI | a view in the same process (Unity plugin, Unreal module, GDExtension, console title) | `orr_host_open`, `orr_view_poll` |
| WebSocket | a view in another process, machine or devkit | `watch.subscribe {"topics":["viewstream"]}` |
| Plain TCP | scripts, tools, anything without a WebSocket library | same call, frames arrive as hex in JSON |
| Rust | `orr_bridge` users | `ViewStreamSource::pump(&mut bridge)` |

## Messages

| Message | Encoding | When |
| --- | --- | --- |
| `Schema` | JSON text | once per connection, before the first frame |
| `ViewFrame` | binary, type 1 | per published tick |
| `EventBatch` | binary, type 2 | events since the last batch |

### Binary preamble (every binary message)

| Offset | Size | Field |
| ---: | ---: | --- |
| 0 | 4 | magic `"OVS1"` (`4F 56 53 31`) |
| 4 | 2 | version (`u16`, 1). A reader refuses a larger one. |
| 6 | 1 | message type (1 = ViewFrame, 2 = EventBatch) |
| 7 | 1 | flags (ViewFrame) / 0 (EventBatch) |

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
single-machine play session has no rollback, so its events are all `verified`.

## Schema (JSON)

Sent once per connection, before the first frame, as compact JSON. Keys stay in
the order below, so even a `strstr` reader can find them. Example (shortened):

```json
{"format":"orrery.viewstream","version":1,"game":"PhysGame","build_id":"0x...","tick_rate":60,"player_count":2,
 "endian":"little",
 "fixed_point":{"type":"q48.16","raw":"i64","frac_bits":16,"note":"..."},
 "frame":{"magic":"OVS1","header_len":56,"record_len":48,"shapes":["circle","quad","capsule"],
          "interp_modes":["prediction","snapshot","none"],"flags":{"rolled_back":1,"discontinuity":2,"paused":4}},
 "kinds":[{"id":0,"name":"static","props":[]},
          {"id":1,"name":"dynamic","props":[{"name":"speed","type":"f32"}]},
          {"id":2,"name":"bar","props":[]},
          {"id":3,"name":"paddle","props":[{"name":"slot","type":"u32"}]}],
 "input":{"size":24,"fields":[{"name":"axis_x","offset":0,"size":8,"type":"fixed"},
                              {"name":"axis_y","offset":8,"size":8,"type":"fixed"},
                              {"name":"spin","offset":16,"size":4,"type":"i32"},
                              {"name":"buttons","offset":20,"size":4,"bits":{"shoot":1},"type":"flags"}]},
 "command":{"size":4},
 "events":[{"id":0,"name":"trigger","payload_size":12}]}
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

The hosts of the same tick share one encoded message, so two subscribers get
byte-identical frames (including `seq`).

## C ABI (`orr_ffi`)

Header: `crates/orr_ffi/include/orrery.h`. The library (`cdylib`: `.so`,
`.dylib`, `.dll`; `staticlib` for consoles) hosts **one game, chosen when it is
built**; `orr_ffi` ships the physics demo (`PhysGame`). To host another game,
copy the crate and replace the `spawn_phys_host` call in `open_host` with a
`LocalHost::spawn::<YourGame>` whose settings include its `view_stream` producer
(the extractor, the kinds, the schema). The header, the schema and the stream
are the same for every game.

```c
OrrHost* h = orr_host_open(NULL /* demo scene */, &cfg);   /* thread + paused session */
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
| `orr_schema_json(h, buf, cap)` | the schema, NUL-terminated; returns size needed |
| `orr_host_url(h, buf, cap)` | `ws://` URL if opened with `ORR_HOST_LISTEN` (for out-of-process views) |
| `orr_view_poll(h, buf, cap, &written)` / `orr_view_poll_ptr(h, &data, &len)` | newest unread frame (copy / zero-copy); `ORR_NO_FRAME` if nothing new |
| `orr_events_poll(h, buf, cap, &written)` | oldest queued event batch |
| `orr_set_input(h, player, bytes, len)`, `orr_send_command(...)` | the two view-to-sim writes |
| `orr_control(h, op, arg)` | `PLAY`, `PAUSE`, `STEP n`, `SEEK tick`, `SPEED permille`, `BRANCH`, `RESTART` |
| `orr_erp_call(h, request_json, out, cap, &needed)` | the full ERP method set in process (`world.query`, `scene.save`, ...), for a foreign editor |

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

Records are fixed-size and uncompressed. Not done yet, because the demo does
not need it: skipping static entities that did not change (a keyframe plus
deltas), quantized positions, lz4 (the ERP frame stream already uses lz4 for
full `Frame`s). A `max_fps` below the tick rate cuts the cost proportionally.

## Integrating an engine

What a Unity, Unreal or Godot view has to do, in order:

1. Load the library (P/Invoke, `FPlatformProcess::GetDllHandle`, GDExtension) and
   wrap the ten functions above; check `orr_abi_version`.
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
   reversible and `verified` as final.
6. Shapes are 2D primitives; a 3D engine maps them to its own meshes from the
   `kind`. Games with richer visuals add properties to their kinds.
