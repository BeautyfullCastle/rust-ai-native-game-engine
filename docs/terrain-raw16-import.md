# Import a RAW16 heightmap for terrain authoring

`orr_terrain_import` is optional offline authoring tooling. It converts one
explicitly described unsigned RAW16 heightmap into a new canonical `.orrt` file
for the existing local terrain Open and terrain-package workflows. No core,
editor or renderer change is required. It does not install anything, infer a
format from a filename, resample a map, add terrain collision/navigation, or
export a standalone terrain game. Current project exporters support their own
Arena/Collect/Room workflows; this command is not a general terrain-game exporter.

## A complete command

```sh
cargo run --locked -p orr_terrain_import -- \
  --input height.raw --output terrain.orrt --width 3 --depth 2 --endian le \
  --height-offset -1 --height-scale 1 --origin-x -1 --origin-z -1 \
  --spacing 1 --asset-id terrain/imported
```

Every value shown is required. Add `--flip-z` only when the source rows need
reversal. Output's parent must already exist, be caller-owned/trusted, and remain
stable. No `--force`, overwrite, recursive directory creation, install or stdin
mode exists. File paths may be OS-native; option/numeric/identity text is UTF-8.
Output's basename must be an ASCII `.orrt` filename of at most 100 bytes using
letters, digits, `-`, `_`, `.` and no Windows reserved device stem. Logical
`--asset-id` is distinct from the filename and follows the existing Terrain ID
rules, including a 256-byte maximum. Keep that ID stable when revising an asset.

For a tiny fixture, six little-endian samples 0, 1, 32767, 32768, 65534, 65535
have bytes `00 00 01 00 ff 7f 00 80 fe ff ff ff`. The command above produces
Q48.16 height raw integers `-65536, -65535, -32769, -32768, -2, 0`.

## Exact input and conversion contract

- Unsigned 16-bit samples only, with explicitly chosen `le` or `be` byte order
- Width and depth each 2..129; exact input length `2 * width * depth`, at most
  33,282 bytes. Headers, trailers, padding, sidecars and inferred square sizes
  are not accepted
- X is fastest in the row-major source. World coordinates are Y-up/XZ:
  `[origin_x + x * spacing, height, origin_z + z * spacing]`
- Without `--flip-z`, source row `z` is used. With it, source row `depth-1-z` is
  used. Columns never flip or transpose
- Decimal parameters use the existing integer-only `FP::parse` once: Q48.16,
  nearest rounding with ties away from zero. Its existing plain-decimal grammar
  permits an optional sign, decimal point and ASCII spaces; exponent notation,
  NaN and infinity are invalid. The normal `FP` display string is approximate,
  so do not derive a lossless import recipe from it
- Scale is a nonnegative full-range height difference, not a per-sample step.
  For each unsigned sample:
  `height_raw = offset_raw + floor(sample * scale_raw / 65535)`
  Checked integer arithmetic is used. Samples 0 and 65535 reach the exact parsed
  endpoints. Negative scale, nonpositive spacing, invalid identity/dimensions,
  coordinate overflow or endpoint overflow fail. Zero scale gives a flat map
- Every cell has `hole = false`. The terrain keeps existing `[a,c,d], [a,d,b]`
  triangles and piecewise planar sampling. It does not use bilinear interpolation

The result must also pass canonical load/cook/revision roundtrip and the existing
production mesh bridge. Every vertex must be exactly representable in f32 and
within +/-1,000,000; spacing must separately be exactly f32-representable for
editor Open. Large offsets/scales with fine Q48.16 steps can fail this display
requirement even when valid core Terrain data could hold them. The tool rejects
such inputs with a useful error rather than silently rounding heights to a
coarser display grid. Choose a representable range/step deliberately in your
source workflow. There is no implicit Unity/Unreal scale equivalence.

## Open or package the result

1. Put the output in the directory of an existing saved local Yard3D scene.
2. In the existing Terrain authoring panel, choose its scene-relative filename
   and Open terrain. Query, copy, edit, undo and save follow existing session
   behavior; importing does not itself change the scene or host Frame/history.
   If using nested directories, all scene-relative path segments must satisfy
   the existing portable path rules (at most 16 segments/240 total bytes).
3. To distribute it as content, author a normal package whose explicit `files`
   contains the `.orrt` path and whose `capabilities` contains `terrain_v1`.
   Pin the usual package/engine versions and use the existing package installer
   separately. This command never edits a manifest, lock or `.orr` store.
4. Open the installed terrain through the editor's package selection. Installed
   assets are read-only; use Copy to scene before editing. The existing consumer
   verifies installed bytes and declaration. Changing/removing original RAW16
   or package-source files does not mutate installed content.

Generic `Project::read_asset` is a content API: a missing `terrain_v1` declaration
is rejected by the terrain editor consumer, not inferred from a file extension
by the generic reader. The package tests and editor tests distinguish these gates.

## Filesystem safety and status

Input and output ancestry must both be trusted, stable and free of symlinks.
Traversal (`..`), nonregular input files, wrong input lengths, symlinks and
preexisting destinations (including dangling links and aliases) are rejected.
Any output path inside a case-insensitive `.orr` component is rejected, even
outside the conventional package-store subdirectory. No ownership or directory
permission is changed or automatically established; caller ownership/trust is a
precondition, not something the tool can portably prove.

The importer checks regular-file metadata before and after opening and reads at
most the expected size plus one byte. On Unix it also compares device/inode;
on other platforms standard-library length/time checks are only consistency
checks. Retained bytes form the import snapshot. A concurrently modified file is
not guaranteed to represent one atomic original version; do not modify it while
importing. Metadata rechecks cannot defeat hostile same-user/root replacement.
In particular, replacement with a FIFO between the metadata check and open may
block before opened-file metadata can be checked. Neither input nor output path
walking is a capability-safe traversal primitive. Do not run this CLI against
untrusted writable/shared directories or under an adversarial path-replacement
threat model.

Publication creates one unique same-directory stage with `create_new` (up to 32
name attempts), writes and syncs it, rereads exact canonical bytes through the
owned handle, closes it and hard-links it to the new destination. Hard-link
creation is no-replace: a destination created by another writer just before
commit wins and remains untouched. Filesystems without hard-link support fail;
there is no replace/copy fallback. Only the owned stage is removed. Unix parent
sync follows link/cleanup; other platforms report unsupported directory sync
rather than promising durability. File/parent sync is not a hardware power-loss
guarantee. Stage permissions follow the process umask/default ACL.

- Exit 0: published, or help displayed
- Exit 2: usage/admission or prepublication error; this invocation did not publish
  a destination. Cleanup failure can leave its named owned stage, reported in
  the error; unrelated files and stages are never deleted
- Exit 3: destination has already been published, but parent durability, stage
  cleanup or success-output delivery is uncertain. A cleanup warning may leave
  one extra named stage. The destination is never rolled back or overwritten

After exit 3, inspect the reported file/warning; do not blindly retry expecting
replacement. On platforms without the implemented parent-directory sync (for
example Windows), a successful import deliberately returns 3 with exact bytes
and a durability warning. A process crash before link may also leave an owned
stage; automatic cleanup of other runs' stages is intentionally unsupported.

## Validation lanes and limits

Default-on tests in this optional crate cover integer/endian/orientation and
canonical conversion, actual CLI processes, filesystem negatives/races, injected
write/sync/readback/link/cleanup faults, and install/reopen/read-asset/tamper.
No GPU or editor dependency enters its default normal dependency closure.

```sh
cargo test --locked -p orr_terrain_import
cargo clippy --locked -p orr_terrain_import --all-targets -- -D warnings
cargo test --locked -p orr_terrain_import --features editor-acceptance --test editor
```

The editor feature uses unchanged production TerrainPanel/TerrainSession methods
and a real local Yard host. It tests local/package Open, query, copy-before-edit,
edit/undo/save/reopen, dirty/precision/declaration rejection and host state
preservation. It is API-level acceptance, not a native-window/widget interaction
or GPU proof. Building it requires the existing editor dependency cache/toolchain.

```sh
mkdir captures
ORR_RAW16_REQUIRE_SOFTWARE=1 ORR_RAW16_CAPTURE_DIR="$PWD/captures" \
  cargo test --locked -p orr_terrain_import --features gpu-acceptance \
    --test render -- --ignored --exact \
    raw16_package_reload_oblique_side_and_orientation_controls_render --nocapture
```

The ignored, explicitly selected GPU case fails if no adapter is available,
reports its actual identity/software status, and requires a fresh capture
directory. It runs importer -> canonical cook -> verified installed package ->
unchanged ModelRenderer, comparing oblique/side baseline with deliberately
flipped/transposed controls and query/vertex anchors. Side-camera anchors have an independent fixed-height oracle and a +2-height
control that translates the entire imported terrain and both markers together.
This checks fixed absolute anchor heights and full-import vertical translation;
isolated marker displacement over an identical terrain background is outside
this fixture's scope. It retains ten original PNGs plus ten RGBA readbacks;
inspect the images separately. This establishes
only the tested fixture/cameras, not hardware-wide quality or performance.
Existing ordinary workspace/native CI can discover the default tests; unchanged
CI is not evidence that these separate opt-in editor/GPU lanes ran.
