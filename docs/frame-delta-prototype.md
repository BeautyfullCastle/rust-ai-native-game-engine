# Independent Frame delta prototype

This is the source prototype for NEXT-14-1 on the shared development baseline
`ef61e959306e71b186b9355f237b458392be073f`. It consists of the new
[`frame_delta.rs`](../crates/orr_remote/src/frame_delta.rs) module and its
[`focused integration tests`](../crates/orr_remote/tests/frame_delta.rs).
The test imports the module by path; the module is not exported by `orr_remote`
or called by the existing server or client.

ERP already sends complete serialized Frames with LZ4 compression. In
[`wire.rs`](../crates/orr_remote/src/wire.rs), `encode_frame_message` wraps
`Frame::to_bytes()` in the existing ORRS v1 envelope with metadata and an LZ4
size prefix. `decode_frame_message` checks the advertised uncompressed size
before decompression, with a 512 MiB ceiling. That envelope, the ORRF v2 Frame
format, existing client decoding, and all existing transport behavior remain
unchanged by this prototype.

## Records and baseline identity

`Encoder` and `Decoder` each retain at most one complete serialized Frame. A
`FrameStamp` contains five `u64` fields:

| Field | Meaning |
| --- | --- |
| `scope.stream_generation` | Distinguishes connections and producer resets. |
| `scope.play_epoch` | Distinguishes play sessions. |
| `scope.timeline_epoch` | Distinguishes seek, rollback, and other timeline discontinuities. |
| `tick` | The Frame's simulation tick. |
| `frame_checksum` | The existing complete Frame checksum. |

The caller assigns scope values. It must advance the appropriate epoch on a
discontinuity and must not reuse an identity for a different stream. These
labels and the existing checksum do not authenticate a sender. A future
transport dispatcher must authorize the current scope and full-reset response;
this module does not establish those permissions or negotiate capabilities.

`FrameRecord::Full` carries a target stamp and the complete original ORRF bytes.
It is also an explicit baseline reset: after successful validation it replaces
the old baseline, even when its scope or tick moves backward. This permits a
caller-authorized seek or reconnect reset. A rejected Full leaves the old
baseline intact.

`FrameRecord::Delta` names the exact base stamp and target stamp, the complete
target byte length, normalized prefix and suffix lengths, and replacement
bytes. The encoder considers a Delta only when scope values match and the
target tick is strictly greater than the base tick. Missing baseline, scope
changes, equal ticks, and backward ticks produce a Full. The caller should
still change its timeline epoch for a backward seek.

## Splice and reconstruction

The prototype uses one common-prefix/common-suffix splice. The two regions never
overlap. It compares normalized ORRF v2 bytes that omit the tick at offsets
8–15 and the final eight-byte checksum. This matters because both fields change
on a forward tick even when the simulation payload stays idle; comparing the
whole raw Frame would usually create an almost complete replacement.

All other bytes, including the ORRF magic, version, registry description,
allocator, component stores, singleton stores, and lists, remain in the
normalized sequence. Delta prefix and suffix offsets refer to that sequence;
`target_len` refers to the complete ORRF bytes and therefore includes the
omitted sixteen bytes. Reconstruction restores the target tick and checksum
from the stamp at their original positions. No field is removed from the
Frame delivered to the application.

The decoder checks scope, forward tick, and exact retained base identity. A
missing or stale base returns `DecodeError::NeedFull`; it does not guess a base,
apply a partial patch, or request a network retry. The caller must obtain an
explicit Full. Scope and non-forward-tick Delta refusals have distinct
`NeedFullReason` values.

Before reconstruction the decoder converts fixed-width lengths with checked
conversions, checks the configured full-frame cap and minimum ORRF header
length, rejects overlapping/out-of-range base regions, and verifies that the
prefix, replacement, and suffix have the declared normalized target length.
It then restores the full bytes and calls the existing
`Frame::from_bytes(registry, bytes)`. That decoder remains responsible for
schema, structural, and checksum validation. The decoded tick and checksum
must also equal the target stamp. Only then does the prototype replace or
clear its baseline. Any failure leaves the previously retained baseline
unchanged.

## Bounds and full fallback

Both constructors require an explicit `max_frame_bytes` and
`max_baseline_bytes`. They are independent limits, including zero. A successful
Frame whose serialized bytes fit the frame cap but exceed the retained
baseline cap is delivered and clears the cache. A subsequent sender record
will be Full, and a subsequent receiver Delta requires a Full reset. A zero
baseline cap disables retention for nonempty Frames. An encoder size failure
and a decoder refusal preserve the previous cache.

The retention limit bounds the serialized bytes retained in that codec's one
baseline. It is not a bound on the caller's input/output records, decoded ECS
storage, allocator overhead, or peak process memory. Full records may share
their `Arc<[u8]>` with the cache. Encoding first serializes the caller-owned
Frame, then checks its size; it does not provide a pre-serialization memory
limit. Encoding and decoding also allocate normalized and reconstructed
buffers. Those transient buffers are separate from retained baseline storage.
The decoder checks a declared target size before allocating its target
reconstruction; it does not control how an upstream caller allocated an
already-created `FrameRecord`.

The sender uses Delta only when its hypothetical uncompressed record cost is
strictly smaller than Full: 41 bytes plus full payload for Full, or 105 bytes
plus replacement payload for Delta. These estimates assume a one-byte tag,
five `u64` values per stamp, and three `u64` lengths for Delta, with replacement
length inferred from the remaining record. They are comparison constants for
this in-memory prototype. There is no implemented wire serializer, negotiated
capability, or measured LZ4 record size behind these numbers. High-change
Frames fall back to Full when that comparison favors Full; a high-change
classification alone is not a promise of a particular record kind.

## Focused validation and remaining work

The new test source uses actual `orr_ecs::Frame` bytes and the existing registry
decoder. It covers forward idle/sparse changes, insertion/removal, Full
fallback, scope changes and discontinuities, exact/stale/missing bases,
malformed lengths, corrupted payloads and stamp/schema mismatches, retention
boundaries, explicit reset, and frame-size refusal. Byte-for-byte reconstructed
Frames and original checksums are the correctness oracles. Existing test
assertions, golden checksums, and required CI gates are unchanged.

Cargo compilation and test execution are pending while the existing Windows
N6 lane remains held. Source parsing/formatting or static inspection does not
establish that these tests pass. An eventual authorized focused command is:

```sh
cargo test -p orr_remote --test frame_delta --release --locked --offline
```

That command must use a fresh execution contract and available local lane; it
has not been run as part of this source-only stage. Code review is assigned to
su under REVIEW-SU-1. The prototype does not require a renewed Haneul/Ddang
cross-review wait.

Transport/capability integration must separately define reset requests and
responses, scope assignment and stale Full handling, loss/reordering behavior,
bounded in-flight records, and a negotiated legacy Full fallback. Subsequent
measurement must compare existing Full+LZ4 and an agreed Delta representation
under fixed workload, registry, runner, cache, and lifecycle conditions. Encode
and decode CPU time, baseline and peak memory, transmitted bytes, idle and
high-change scenes, seek/rollback, and reconnect are separate observations.
No performance improvement, production readiness, cost ratio, or completed
#14 acceptance is claimed by this source prototype.
