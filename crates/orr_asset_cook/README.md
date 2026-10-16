# ORAM v1 asset cooker

`orr_asset_cook` is a std-only authoring CLI for the two opt-in asset formats in
[`asset-pipeline-v1.md`](../../docs/asset-pipeline-v1.md). It does not load assets
in a simulation, change Arena's sounds, inject runtime tables, bind a release,
or admit a replay. Those are separate consumers of its output.

## Reproduce the checked-in fixture

From the repository root:

```sh
cargo test -p orr_asset -p orr_asset_cook --locked
cargo run -p orr_asset_cook --locked -- cook \
  --index assets/fixture_v1/index.json --out assets/fixture_v1/cooked --check \
  --root a_0000000000001001 --root a_0000000000002001
cargo run -p orr_asset_cook --locked -- inspect --bundle assets/fixture_v1/cooked
```

For a new bundle, omit `--check` and choose a **new, nonexistent** output directory
whose parent already exists. An optional `--cache DIR` reuses validated imports. Cache and output directories
must not contain one another; this is checked through existing symlink aliases
before any cache directory is created.
`--check` always recomputes imports and compares every expected file plus the
exact output file set; it writes neither output nor cache. Cargo builds do not
silently regenerate assets. Source locations and whitespace can change
`report.json`, but do not change runtime bytes when semantics stay unchanged.

Output consists of:

- `sim.manifest.bin`, `view.manifest.bin`: canonical ORAM v1, numeric GUID order
- `objects/<sha256>.bin`: canonical little-endian payloads
- `generated_sim.rs`: compile-time validated immutable `MOTION_PROFILES` and the
  expected full `SIM_MANIFEST_SHA256`; requires `orr_asset` and `orr_fp`
- `inspect.json`: readable identity/metadata, reproducible and not identity input
- `report.json`: separately tracked source paths/raw hashes and cache keys

The fixed vector hashes in `assets/fixture_v1/vectors.json` were independently
computed with integer arithmetic, `struct.pack` and SHA-256, rather than copied
from the cooker. Tests compare them with cooker output. The checked-in generated
Rust is included by an integration test and re-encoded against the manifest.

## GUID lifecycle

```sh
# Start from an index with {"format":"orr.asset-index/1","entries":[]}.
orr_asset_cook register --index assets/index.json --out-index assets/next.json \
  --type sim.motion_profile --source source/motion.json
orr_asset_cook clone --index assets/index.json --out-index assets/next.json \
  --from a_0000000000001001 --source source/copied_motion.json
orr_asset_cook move --index assets/index.json --out-index assets/next.json \
  --id a_0000000000001001 --source source/renamed_motion.json
orr_asset_cook tombstone --index assets/index.json --out-index assets/next.json \
  --id a_0000000000001001 --root a_0000000000002001
```

Register/clone allocate nonzero OS-random u64 IDs and check both live IDs and
permanent tombstones for collisions. For reviewed fixtures, `--id a_...` can
provide an explicit canonical ID. Clone copies metadata to an independently
provided source path, not source files; move changes the index path, not the
filesystem. Neither derives identity from content or paths. All authoring commands
write a new index without replacing the old one. Keep it in the same directory,
or deliberately update every source path. A declared root cannot be tombstoned.
The tool does not search unrelated documents for references. Removing tombstones
by manually rewriting an index defeats its historical non-reuse guarantee.

## Strict bounded format

Index/source JSON rejects duplicate/unknown fields, unsupported versions/types,
non-UTF-8, trailing data, excess nesting, unexpected dependencies, and limits
before unbounded growth. Roots are explicit repeated `--root` options. Source
paths must be canonical portable relative paths under the index's directory;
absolute paths, traversal, nonportable components, aliases and escaping symlinks
are rejected. Sources are read only as explicitly listed by the index.

Limits: index plus source bytes at most 1 MiB, individual source 64 KiB, 1024
index records including tombstones, 64 live sim and 16 live view records, 128 KiB
manifest input, 64 KiB sim payloads and 2 MiB view payloads. PCM is exactly 48 kHz
mono, 1..48000 frames, period a multiple of four in 4..48000, peak 1..8192.
`triangle_decay_v1` uses only signed i64 arithmetic and truncates division toward
zero. Payload is an 8-byte rate/frame header followed by exact i16 LE samples.

Motion authoring is a JSON **string** with an optional sign, digits and at most
one decimal point, with at least one digit. Exponents and whitespace inside the
string are rejected. `FP::parse` supplies integer-only nearest rounding with ties
away from zero; the resulting raw value must be 1..1048576. No floating-point
parser or display/parse roundtrip is used.

## Cache and publication contract

Cache key input, in order: ASCII `orr.asset-cook.cache` plus NUL, cooker format
u32 LE (1), importer version u32 LE (1), type u32 LE, schema u32 LE (1), semantic
option count u32 LE (0), then the 32-byte SHA-256 of exact source bytes. There are
no v1 dependencies or semantic options. Format/importer changes require a version
bump. GUID and path are excluded because they belong to manifest assembly.

Cache entries carry magic `ORAC`, u32 version/type/schema, the full key, u64 LE
payload length, payload digest, and payload. Every hit validates exact length,
digest and schema. Corrupt/partial entries are recooked and repaired. Cache
filenames are internal hashes and cache reads cannot escape their directory.
Hashes detect corruption and identify content; they are not signatures or a
trusted-source guarantee.

All output is written to a new sibling staging directory, each file is flushed,
and every byte/object/table is checked before one directory rename publishes the
bundle. A create-new sibling lock serializes cooperating publishers. Existing
release output is rejected. This is atomic visibility, not a crash/power-loss
durability guarantee. A crash may leave a visible staging directory or lock;
there is no automatic GC. The std-only directory rename assumes callers control
the parent directory: hostile concurrent filesystem mutations are outside the
contract. Authoring index publication uses a create-if-absent hard link, so it
requires filesystem hard-link support. File reads also assume no hostile changes
between canonicalization and opening.

## Validation scope

Tests cover deterministic payload/manifests/generated source, clean/cold/warm
runs, input reorder/relocation/whitespace, GUID lifecycle, strict parser/path and
byte bounds, cache recovery, output preservation, and check/inspect corruption.
Linux focused tests can establish these vectors locally. Windows/macOS execution
and cross-platform equality require running the same vector tests on those
platforms; a Linux result alone does not establish those passes. Runtime replay,
audio-device quality and workspace-wide CI are outside this crate's test claim.
