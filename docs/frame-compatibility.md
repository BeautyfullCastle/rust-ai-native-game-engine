# Frame checksum and replay compatibility

Status: ORRF v2 passed the native workspace and wasm standard/SIMD golden slices,
2026-10-02. The reflection fixture, browser-target build, and strict Clippy checks also
passed. See [progress](progress.md) for exact coverage and skips.
This is not a claim that every supported platform or browser runtime has been exercised.

## What changes in ORRF v2

`Frame::to_bytes()` retains the ORRF v1 field layout, with the header version changed from
`1` to `2` and a new checksum value in the final 8-byte little-endian trailer.

`Frame::checksum()` now equals `xxh3_64(bytes[..bytes.len() - 8])` for the bytes returned
by `Frame::to_bytes()`. The hashed bytes include the magic and version, tick, schema names
and element sizes, entity allocator, component stores, singletons, and list pools. Collection
counts, individual list lengths, and ordered free lists are included. The streaming checksum
does not allocate a serialized buffer. Derived sparse indexes and allocation capacities are
not serialized state.

This fixes omissions in v1: distinct component/list boundaries or list free orders could
hash identically even though they represented different states or produced different next
allocations. It also covers zero-sized list lengths, which have no element payload bytes.
The existing xxh3 hash is a deterministic integrity checksum, not cryptographic authentication.

A local old/new ECS comparison over a 128-frame workload confirmed identical schema/state
payload bytes, with only the 4-byte version field and 8-byte checksum trailer changing.
That workload exercised entity reuse, two same-sized component stores, singleton updates,
live/freed lists, and zero-sized lists. This is focused evidence, not an all-game or
cross-platform result.

The format remains limited to supported little-endian targets; POD payloads are stored as
raw bytes, and big-endian builds are rejected.

## Compatibility by boundary

| Boundary | Behavior after this change |
| --- | --- |
| ORRF frame snapshots | `Frame::from_bytes` accepts version 2 and explicitly rejects version 1 with `FrameDecodeError::UnsupportedVersion(1)` |
| `.orrp` replay container | Writer remains version 3; parser still recognizes container versions 1–3. Container parsing alone does not establish frame/checksum compatibility |
| Old replay keyframes | Embedded ORRF v1 frames fail when restored, for example `ReplayError::BadKeyframe` from `ReplayReader::seek`, whose source is `UnsupportedVersion(1)` |
| Old recorded checksums | Resimulating unchanged gameplay still produces the new Frame checksum. Old Frame-checksum expectations are incompatible and verification reports mismatches unless an earlier build-id check rejects the replay |
| Late-join/correction/ERP full-frame payloads | They carry serialized Frames, so ORRF v1 payloads cannot be loaded by v2 frame readers. ERP's outer ORRS envelope remains version 1 |
| Multiplayer checksum agreement | Old and new checksum implementations are incompatible. All peers and an authoritative server must use compatible builds |
| View stream and C ABI layouts | This checksum change does not change OVS1 view-stream layouts or the C ABI version. A returned simulation checksum, such as `orr_confirmed_checksum`, uses the new Frame checksum |
| Strict YAML scenes | This change does not alter the scene text format; rebaking produces a new-format Frame |

The same-named versions are independent: ORRF v2 is a simulation snapshot, `.orrp` v3 is a
replay container, ORRS v1 is an ERP frame envelope, and view-stream v2 adds 3D view records.
Do not infer compatibility of one from the version of another.

## Golden migration scope

The v2 migration updates 21 Frame-derived expected values in seven test files: ECS core,
session arena, 2D physics, 3D physics, sample games, reflection scene baking, and wasm benchmark
checksums. The operation sequences and assertions were retained; raw FP arithmetic and the
capsule-query hash were not changed. All 21 migrated values matched in the native workspace
and standard wasm reruns, including the separately run reflection bake fixture. The configured
SIMD slice and reflection fixture also passed; this is not a full-workspace SIMD run.
Platform coverage is tracked in [progress](progress.md). Updating an expectation alone is
not a passing validation result.

## Upgrade and release requirements

1. Preserve any old replay/snapshot needed for diagnosis together with its matching old build.
   There is no automatic v1 snapshot converter in this change. Do not relabel the header or
   overwrite recorded checksum values and claim the old recording was verified.
2. Rebuild scenes from their source and record new replays using the v2 build. Recovering an
   old binary-only state would require a separately designed and validated migration tool.
3. Give incompatible engine/game builds distinct nonzero build IDs and deploy compatible
   clients and servers together. Built-in arena/physics clients (native and browser), server
   presets, and the default ERP host now use `orr_sim::frame_build_id`, binding their game ID
   to the public `orr_ecs::FRAME_FORMAT_VERSION`. The compatibility patch and rejection tests
   are implemented; their execution status is tracked in [progress](progress.md).
   Explicit custom IDs are unchanged: callers must use the helper or bump incompatible IDs
   themselves. The helper preserves `frame_build_id(0) == 0`, the deliberate untracked
   wildcard, and produces a nonzero result for a nonzero game ID. It does not detect changes
   to game code within one format version; the underlying game/build ID must still change.
   For an explicit browser override, pass an exact decimal or `0x` hexadecimal string. The
   page forwards text without converting it to a JavaScript Number; the wasm options also
   accept safe nonnegative numeric values and reject unsafe numbers before connecting.
   Omitting the override selects the versioned built-in default.
4. Rebaseline only Frame-dependent golden expectations after proving the semantic checksum
   change. Record the reason in the eventual commit message. FP-only arithmetic goldens need
   not change for this fix. Historical golden values in older progress entries remain history.
5. Re-run affected snapshot, replay, rollback, late-join/correction, ERP, C ABI, and game tests,
   then the supported-platform determinism jobs. Compare checksum/serialization/tick costs
   before and after; a local focused pass is not a cross-platform result.

Implementation references: `crates/orr_ecs/src/frame.rs`, `codec.rs`, `entity.rs`, `store.rs`,
`list.rs`; regression coverage in `crates/orr_ecs/tests/checksum.rs`; replay restore in
`crates/orr_session/src/replay.rs`; ERP envelope in `crates/orr_remote/src/wire.rs`.
