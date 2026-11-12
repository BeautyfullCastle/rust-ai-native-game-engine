# CoopDefense presentation contract v1

## Boundary and reuse

The core already provides generational ECS entities, deterministic
`SimEvent<EventKey(tick, system_index, seq)>`, Session predicted/verified/canceled
reconciliation, a bounded bridge with `ViewResync`, and event-keyed particle and
audio lifetimes. Sprite clips and skeletal animation are optional view facilities.
This change adds no competing engine bus, simulation clock or gameplay rule.
The optional browser view provides local clip/effect/audio presentation only; it
does not replace native renderer, audio or Session facilities.

The missing layer was the optional CoopDefense sample: its cell-backed unit and
projectile pools lacked logical lifetimes, its events omitted combat attribution,
and its local WASM wrapper discarded `Simulation::step` events. An ECS pool slot
is not an actor: moving a unit swaps values between cell slots.

## Simulation contract

`DefenseUnit.id` and `DefenseProjectile.id/source_unit_id` are deterministic,
nonzero lifetime IDs. Allocator counters and restart `generation` live in the
Frame, so snapshot restore/replay reproduces them exactly. Zero means absent.
Counters fail explicitly on exhaustion; they never wrap into an old identity.
Enemies retain their existing ID allocator, now checked against overflow.
Movement preserves unit identity. Merge preserves the destination unit ID and
explicitly consumes `other_unit_id`; increasing its tier does not create an actor.
Projectiles retain source identity after the source moves or is consumed.

`DefenseEvent.event_kind()` validates the closed semantic tag. Events distinguish
summon, move, merge, enemy spawn, actual projectile release, actual damage, death,
leak, target-lost projectile expiration, wave, finish, and restart. Hit carries
actual HP removed, HP afterward, source unit, projectile, and victim IDs. Splash
emits one attributed hit per victim. Death follows the killing hit. Leak never
emits death. Positions describe the event, not a later guessed snapshot position.
Windup remains unsupported because simulation has no preattack command/state.

Per-tick production has a structural bound from fixed actor/projectile pools,
checked before emission. Existing EventKey and Session reconciliation are reused.
Native rollback clients must continue through Session/Bridge; never replace their
predicted/verified/canceled semantics with the local WASM adapter.

## Additive WASM contract

Existing array prefixes remain unchanged. Units append `id` at index 7;
projectiles append `id` and `source_unit_id` at indices 6 and 7. Enemy IDs already
occupy index 0. This changes the game Frame schema/checksum intentionally, not
its gameplay results; old binary Frame/replay compatibility is not claimed.

`snapshot.presentation` contains version 1, source, generation, tick, delivery,
status, dropped, fullResync, windup, and events. Source is a host-instance serial,
changed by `defense_init`. Generation is the actual Frame restart generation.
Tick is the actual post-step engine tick and does not reset with gameplay tick.
All u64 source/tick values are decimal strings. Module replacement also requires a
new consumer session; the serial is scoped to that module instance.

Events expose the existing key as tick/system/seq, a typed kind string, generation,
unitId, otherUnitId, enemyId, projectileId, board, role, value, hpAfter, x, y.
Event tick equals the post-step engine tick (first tick is 1). `value` retains old
action values; for release it is nominal damage, for hit it is actual HP removed.
Board means source board for release/expiration and affected board/lane otherwise.
Fields irrelevant to a kind are zero. All local events are final; batch `status`
is delivery health, not a second prediction protocol.

Delivery is `latest_tick`, not a reliable queue. Up to 1024 events are retained
until the next step. Repeated snapshots return identical events; deduplicate on
source/generation/tick/system/seq. Read after each step for every effect. On a tick
gap, backward tick, source/generation change, or overflow, discard transient
history and rebuild persistent visuals from the snapshot; do not infer missed
attacks or deaths. Overflow discards the whole batch, sets status `overflow`,
fullResync true, and reports the dropped count. A complete subsequent tick clears
that status. Initial attachment adopts the snapshot as a baseline.

## Strict optional consumer

`web/strict-event-adapter.mjs` validates the complete batch before delivery to
`web/combat-presentation.mjs`. Actor keys are `(source,generation,type,id)`; event
keys are `(source,generation,tick,system,seq)`. u64 values stay decimal strings
and are compared with BigInt. Identical repeats are no-ops. Conflicting repeats,
invalid attribution and malformed lifecycle ordering reject atomically, clear
transients, then silently baseline the next valid snapshot. There is no legacy
fallback; the old heuristic adapter exists only in comparison tests.

Release, splash hit, death, leak and expiration use exact producer attribution.
Born-and-killed-in-one-tick actors receive event-local cues, not invented live
snapshot entities. The renderer's revision/time is presentation bookkeeping only.
Pause/hidden/ready freeze it; draw never steps Rust. Active forest art remains a
single-image procedural fallback. Test-only authored clips are not a new roster.
The unrelated v9 historical map/input fixture is not integrated.

## Acceptance

Focused Rust tests cover stable IDs, consumed merge identity, target-lost
expiration, splash attribution, leak/death separation, exact key/payload replay,
restart, duplicate input, and bounded whole-batch overflow. The actual-WASM test
checks the ABI and compares every legacy gameplay field to the prior WASM for
1200 parity ticks plus a complete AFK loss. The revised checksum fixture must
match the independent native run. The optional web gate builds the candidate WASM, tests strict consumption,
4,000-tick duplicate/invariance replay, paused native-Canvas output, mock-DOM
input, and byte-exact offline ES-module packaging. Module graph changes fail
closed. `bash tools/check-linux-local.sh` and independent exact-source review
remain mandatory. Focused success is not browser/acoustic/hardware validation,
original-game fidelity, whole-workspace acceptance or merge readiness.
