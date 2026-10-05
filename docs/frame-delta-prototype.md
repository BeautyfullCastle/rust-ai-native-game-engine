# Negotiated Frame delta transport

NEXT-14-1 introduced the independent codec in source
`3d3d2aed88c1777c4f9f8ce52fef6796f1dae261` on shared EF. NEXT-14-2 exports
[`frame_delta.rs`](../crates/orr_remote/src/frame_delta.rs) and connects it to
the ERP WebSocket server and `RemoteBridge` through an explicit opt-in policy.
The original [codec tests](../crates/orr_remote/tests/frame_delta.rs) retain
their path import. New [transport tests](../crates/orr_remote/tests/frame_delta_transport.rs)
exercise the negotiated envelope, transaction refusal, legacy compatibility,
independent subscribers, and backward seek restoration.

ERP already sends complete serialized Frames with LZ4 compression. In
[`wire.rs`](../crates/orr_remote/src/wire.rs), `encode_frame_message` wraps
`Frame::to_bytes()` in the existing ORRS v1 envelope with metadata and an LZ4
size prefix. `decode_frame_message` checks the advertised uncompressed size
before decompression, with a 512 MiB ceiling. That envelope, the ORRF v2 Frame
format, existing client decoding, and all existing transport behavior remain
unchanged for clients that do not negotiate the new capability. The new ORRS
v2 envelope is a separate tagged Full/Delta format with smaller explicit caps.
The capability version is `frame_codec: 1`; this is distinct from envelope
version 2 and the existing ORRF v2 Frame format.

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
labels and the existing checksum do not authenticate a sender. In the
negotiated transport, stream generation is the acknowledged subscription,
play epoch is the host play epoch, and timeline epoch is the codec reset
generation. The client validates these identities before committing a record.

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

The convenience `Encoder::encode` uses Delta only when its uncompressed record cost is
strictly smaller than Full: 41 bytes plus full payload for Full, or 105 bytes
plus replacement payload for Delta. These estimates assume a one-byte tag,
five `u64` values per stamp, and three `u64` lengths for Delta, with replacement
length inferred from the remaining record. They are comparison constants for
the independent codec. The negotiated server instead prepares both candidates
and compares the complete actual compressed envelope sizes, including JSON
metadata. It chooses Delta only when its successful encoded message is
strictly smaller than Full, or when Full cannot fit the negotiated message
cap and Delta can. High-change classification alone does not promise a
particular record kind or a measured performance improvement.

## Capability, admission, and reset

`rpc.discover` advertises `features.frame_codec: [1]`. Negotiation is restricted
to network WebSocket subscriptions with exactly `frames`, `events`, and
`notes`, `source: "sim"`, and `view_delivery: 1`. TCP, local adapters, view
streams, editor preview, and the existing unnegotiated path use their existing
formats. Existing public `RemoteConfig` fields are unchanged.

Use `RemoteBridge::connect_with_frame_codec` or the additive
`connect_transport_with_frame_codec` with a `FrameCodecPolicy`. `Prefer`
falls back when discovery does not advertise the capability, before activating
a codec subscription. `Require` refuses that host. A malformed or unexpected
acknowledgement is an error, not permission to downgrade an active stream.
The reset timeout defaults to ten seconds and must be positive and at most
sixty seconds.

The subscription request carries version 1 and explicit frame, baseline, and
message byte caps. The server admits the acknowledgement to its bounded queue
before installing the subscription. The acknowledgement names the accepted
caps, new subscription, sequence `"0"`, and reset generation `"1"`. Identities
are canonical decimal strings so every `u64` remains exact in JSON. The first
record is Full with sequence 1 and generation 1; subsequent record sequences
strictly increase within that subscription.

The new envelope has a 12-byte ORRS header, bounded JSON metadata, and a
tagged LZ4 record with an advertised uncompressed size. Reserved bytes,
unknown versions/tags, oversized metadata or decompression prefixes, malformed
lengths, mismatched base/target identities, and trailing record bytes are
rejected. The negotiated maxima are 8 MiB per frame, 8 MiB per retained
baseline, 16 MiB per message, and 64 KiB of JSON metadata. Frame and baseline
caps remain independent. Server subscriptions reserve their accepted
baseline capacity against a 64 MiB aggregate cap; admission releases the
capacity when that subscription is replaced or removed.

`Encoder::prepare` and `Decoder::prepare_decode` leave the current baseline
unchanged. The server commits its prepared baseline, sequence, and delivery
cut only after exact byte admission succeeds. Text records and binary records
in the new mode share a 64 MiB retained queue budget, with RAII charge
reclamation on dequeue or failure. This is a retained-byte limit, not a
process memory ceiling or a bound on pre-serialization allocation.

On a host discontinuity, `watch.frame_codec_reset` and the following Full
enter the FIFO as one atomic queue item. The announcement names the next
generation, next record sequence, scope, and delivery cut. Sequence never
restarts within a subscription. A refused enqueue preserves the old encoder
and counters. The client requires the announced Full and validates its outer
identity and delivery stamp before atomically publishing the decoded frame
and mailbox state. Rejection preserves the previous displayed snapshot.

If reconstruction needs an unavailable baseline, the client makes one
correlated `watch.subscribe` reset request. It discards records from the old
subscription while awaiting the new acknowledgement and Full. The new
subscription starts at sequence 1/generation 1, clears staged delivery state,
and retains the previous displayed snapshot until a valid replacement is
published. Missing or invalid reset acknowledgement/Full becomes an explicit
failure by the finite deadline; it does not start a retry loop.

## Focused validation and remaining work

The new test source uses actual `orr_ecs::Frame` bytes and the existing registry
decoder. It covers forward idle/sparse changes, insertion/removal, Full
fallback, scope changes and discontinuities, exact/stale/missing bases,
malformed lengths, corrupted payloads and stamp/schema mismatches, retention
boundaries, explicit reset, and frame-size refusal. Byte-for-byte reconstructed
Frames and original checksums are the correctness oracles. Existing test
assertions, golden checksums, and required CI gates are unchanged.

The fixed NEXT-14-1 codec test binary ran on native ARM CI with
16 passed, 0 failed, and 0 ignored. That result belongs to source 3d3; it does
not establish success for the later transport changes. A separate local run of
the new transport integration target passed 11 tests with no failures or ignored
tests. It exercised negotiated WebSocket reconstruction and host checksums,
independent subscribers, backward seek, legacy Full+LZ4, old-host Prefer/Require,
independent limits, zero retention, strict ACK identity, and rejected commits.
That run used the type-corrected working source. A subsequent test-only wrapper
attribute change and the affected unit, existing delivery, and strict Clippy
checks remain pending. The transport-focused command is:

```sh
cargo test -p orr_remote --test frame_delta_transport --release --locked --offline
```

That command must use a fresh execution contract and available local lane.
Code review is assigned to
su under REVIEW-SU-1. The prototype does not require a renewed Haneul/Ddang
cross-review wait.

Subsequent measurement must compare existing Full+LZ4 and the Delta representation
under fixed workload, registry, runner, cache, and lifecycle conditions. Encode
and decode CPU time, baseline and peak memory, transmitted bytes, idle and
high-change scenes, seek/rollback, and reconnect are separate observations.
No performance improvement, production readiness, cost ratio, or completed
#14 acceptance is claimed by source inspection or the original codec CI.
