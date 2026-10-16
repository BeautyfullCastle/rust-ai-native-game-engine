# orr_asset: first asset core

`no_std`, no allocator, filesystem, cooker, SHA implementation, audio, or runtime
asset injection. This implements the structural core of asset-pipeline-v1 only.
It does not change ORRF/ORRP or existing games.

## Identity and admission

- `AssetRef` is a transparent POD `u64`. Zero represents null, and authoring text
  roundtrips it, but live records, required typed references and lookup reject it.
  Every other `u64`, including `u64::MAX`, is valid. IDs are stable identities,
  not paths, hashes, array positions, or entity references.
- Text is exactly `a_` followed by 16 lowercase hexadecimal digits. Decimal,
  uppercase, alternate namespaces, and overlong forms are rejected.
- `Manifest::decode(bytes, expected_domain)` checks canonical ORAM v1 bytes.
  `Manifest::typed_ref::<MotionProfileV1>(id)` checks membership and type metadata.
  `TypedRef::from_entry` also supports cooker metadata, but does not authenticate
  it. Neither constructor proves that an artifact is trusted or a table matches
  a release. The bundle verifier must make those comparisons.
- `SimTable::new` borrows a sorted immutable record slice, rejecting null,
  duplicates, descending IDs, and over-budget counts. Resolve does no allocation,
  I/O, locking or hashing. Inserting a lower ID never changes stored references.
- `MotionProfileV1::new` is const and accepts FP raw values 1 through 1048576.
  Its canonical payload is exactly eight little-endian signed raw bytes.

## ORAM canonical encoding

All integer fields are little-endian, with no padding.

| Header offset | Field |
| --- | --- |
| 0 | `ORAM` (4 bytes) |
| 4 | manifest version `u32 = 1` |
| 8 | domain `u32`: 1 sim, 2 view |
| 12 | entry count `u32` |

Each entry is 56 bytes: GUID `u64`, type `u32`, schema `u32`, payload length
`u64`, and 32 opaque SHA-256 bytes. Entries must have strictly increasing numeric
GUIDs. Type 1/schema 1 belongs to sim and declares exactly 8 payload bytes.
Type 2/schema 1 belongs to view and declares 8 + 2*N bytes for N in 1..=48000.
Actual PCM header/sample validation belongs to the future view loader.
Unknown type/version/domain, noncanonical order, incomplete records, and trailing
bytes fail closed. Hash byte corruption cannot be detected structurally: a caller
must verify SHA-256 against the exact artifact and release binding.

Input is capped at 128 KiB before parsing. Counts are capped at 64 sim or 16 view
records before size arithmetic. Hence actual accepted v1 manifest sizes are at
most 3600/912 bytes, despite the larger general input cap. Declared payload totals
are checked with overflow-safe arithmetic against 64 KiB sim/2 MiB view. V1's
stricter per-type lengths and record caps currently make those aggregate maxima
unreachable (maximum valid totals are 512/1536128 bytes). These are admission
policies, not measured runtime/OS latency or peak-memory claims. Empty manifests
are canonical; a required-reference lookup in one still fails.

`encode_manifest` requires a correctly sized caller-owned buffer and canonical
record order. It validates everything before writing, so an error leaves the
buffer unchanged. `Manifest::as_bytes` returns the exact canonical bytes to hash
outside the core. Neither endianness nor native alignment is assumed by decoding.

## Boundary and verification

Only raw `AssetRef` belongs in Frame POD state; tables and typed references are
external read-only views. Supported simulation schemas are sealed in v1, so a
new type or version requires an explicit registry/codec design change. SHA-256,
JSON/source validation, file sizes, artifact integrity, release identity, replay
admission, and audio are still the caller's or later child implementation's work.

Focused checks:

```sh
cargo test -p orr_asset
CLIPPY_CONF_DIR="$PWD/tools/sim-float-guard" cargo clippy -p orr_asset --all-targets -- -D warnings -F clippy::disallowed_types
cargo check -p orr_asset --no-default-features --target wasm32-unknown-unknown
cargo tree -p orr_asset --edges normal,build
```

The wasm command requires the target to be installed. Native tests pin independent
sim/view wire vectors, full-u64 GUID handling, unaligned/truncated input, malformed
headers/entries, limits, transactional encoding, FP ranges, and stable lookup.
