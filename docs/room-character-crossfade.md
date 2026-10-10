# Bounded Room character crossfade

This slice extends the existing default-off Room character route. It does not
add an animation graph, blend tree, locomotion controller, root motion, IK,
retargeting, renderer/RHI API changes or animation data in the deterministic Frame.

## Authoring and compatibility

Schema 1 keeps its exact original fields and fixed 1x speed. Schema 2 keeps its
complete closed per-state speeds object. Both snap to the selected clip exactly
as before. Generated projects remain schema 1 until the user authors a change.

Schema 3 requires speeds and `crossfade_ticks`:

```json
{"schema":3,"player":"e_0123456789abcdef0123456789abcdef","searching":0,"carrying":1,"escaped":2,"speeds":{"searching":"1x","carrying":"2x","escaped":"1/2x"},"crossfade_ticks":12}
```

The duration is an integer in 0..120 simulation ticks. Null, missing, fractional,
negative, duplicate, string and out-of-range values are rejected. Schemas 1/2
reject this field. Zero is an explicit snap and carries no crossfade storage
charge. Changing the panel duration upgrades the candidate to schema 3; Apply,
Undo, Redo, paired Save and reopen use the existing transactional authoring path.

## Presentation contract

`PlaybackCursor` owns at most current and outgoing poses. At a consecutive-tick
state change, it freezes the previous visible pose and records the current tick
as the transition start. That tick has weight zero. Subsequent consecutive ticks
use `(tick - start) / crossfade_ticks`; at the endpoint the outgoing pose is
released. An interrupted blend freezes its current visible result and starts a
new bounded transition. The incoming clip always samples its original absolute
tick, authored rate and Loop phase, including zero-length clips.

`AnimatedModel::blend_poses` validates finite 0..1 weight and both opaque pose
owners before exact endpoint returns. Interior blends linearly interpolate local
translation/scale and use normalized shortest-path quaternion slerp. The model
rebuilds and validates local TRS, global hierarchy and skin palettes. Room also
checks finite deformed bounds before accepting an observation.

The native App and headless/export runtime observe initial state and each actual
simulation step. Render calls consume the exact observed pose and reject stale
source/tick mismatches; drawing does not advance the cursor. The editor uses the
same cursor on coherent immutable snapshots. When observed ticks are not
consecutive it snaps to the absolute target. It cannot reconstruct transitions
that happened in unavailable snapshots. A same-tick state correction also snaps.

Explicit local editor seek, including same-tick seek, start, stop, branch and
scene load replace a Room-only revision token. Host timeline epoch catches
observable remote seeks/edits. Native restart/checkpoint restore reset the
cursor. Document values, exact Arc asset identity, player entity generation,
tick rate and rest/play mode are part of source identity. Rewinds and source
changes snap, rather than blending unrelated history. No reset writes to the
simulation or recorded inputs/checkpoints.

## Memory and failure handling

Existing aggregate Room admission stays 64 MiB. Schema 1, schema 2 and schema 3
zero keep the previous two-pose working allowance. Only positive schema 3 adds
three pose buffers: the conservative five-buffer peak includes two retained
poses, sampled target, blend result and transactional outgoing replacement.
There is no unbounded history or transition queue. Validation failure leaves
cursor history unchanged, and budget checking happens before authored admission
or editor replacement.

## Validation status and intended gates

This source was newly reconstructed after an executor workspace replacement,
from explicitly untested candidate text on base
`737490399a0ccd0dfc29362535be0eeb30fa191e`. It is not recovered tested source.
At authoring time, compilation, test execution, GPU acceptance and hosted CI are
pending the coordinator's exclusive verification lease. Predecessor camera,
cubic-spline and playback-rate evidence is separate and is not crossfade proof.

Required source cases cover model endpoints/midpoint hierarchy/deformation,
quaternion shortest paths and foreign ownership; strict schema; interruption,
repeat draws, missing snapshots, rewind and all identity resets; zero duration,
zero-length clips; duration endpoints and authored-rate phase near `u64::MAX`;
stale render lookups; failed observations and scoped admission boundary accounting.
The CPU App case exercises actual authoritative inputs through pickup, interrupted
escape transition, completion and restart, with per-tick simulation byte parity
against a presentation-free simulation and non-default authored rates.
The actual editor case uses the production panel transaction, history, paired
save/reopen and paused production inputs. Its same-tick seek negative control
requires different viewport pixels with identical simulation bytes. Numeric
panel authoring is exercised through its production transaction API; no claim
of a simulated slider drag is made.

The exported runtime case requires exact source/export PNG and simulation parity
inside the established source-hidden, read-only namespace, compares positive
crossfade against schema-3 zero on the same run, and invokes a separate real App
midpoint/repeated-draw/completion/restart child. GPU acceptance requires
`ORR_REQUIRE_GPU=1`; adapter identity is printed. These are software GPU/App
harness claims when executed, not native-window or physical GPU coverage.
