# Offline PNG + Sprite JSON package authoring

`png_sprite_package` is a deliberately narrow, opt-in example. Give it an existing
PNG atlas, an existing `orr_sprite` JSON document and an explicit license file. It
validates them with the existing public sprite parser and atlas decoder, then
creates a **new** content-package directory. It does not generate regions or clips,
reimport or replace an existing package, install into a project, edit a scene or
binding, launch a host, create a GPU, or demonstrate rendered animation.

## Run

From the repository root, with an existing trusted output parent:

```sh
cargo run -p orr_sample --features project-create --example png_sprite_package -- \
  --png assets/sprite_demo/lantern_keeper.png \
  --json assets/sprite_demo/sprites.json \
  --license assets/sprite_demo/LICENSE.txt \
  --name my-sprites --version 1.0.0 --out /tmp/my-new-sprite-package
```

All six options are required exactly once. `--help` alone prints usage. Paths are
OS-native arguments; names and versions must be UTF-8. Running without
`project-create` exits unsuccessfully with a feature-required message **before
argument processing or any source/output file access**, including for `--help`.
There are no new dependencies, Cargo features, public library APIs or runtime
routes.

The package name is 1–64 lowercase ASCII letters, digits, `-` or `_`. The version
must be canonical SemVer 2.0.0 with exactly three non-leading-zero unsigned-u64
core fields, optional prerelease/build identifiers, and at most 256 bytes.
Prerelease numeric identifiers cannot have leading zeroes; build identifiers may.

The source PNG path may have a different filename: the destination PNG is named
by the JSON's `atlas.image`. That name must be a single portable ASCII leaf,
1–255 bytes, containing only letters, digits, `-`, `_` and `.`, with a nonempty
stem and a case-insensitive `.png` extension. Absolute, nested, traversal, drive,
UNC, alternate-data-stream and other separator-bearing names are rejected, as
are trailing dots/spaces and Windows device stems (`CON`, `PRN`, `AUX`, `NUL`,
`COM1`–`COM9`, `LPT1`–`LPT9`, case-insensitively, also before extra extensions).
Case-insensitive collisions with `sprites.json`, `LICENSE.txt` and
`orr.package.json` are rejected. No JSON path is used to locate an input: the
explicit `--png` is the only atlas source.

## Validation and limits

These are independent limits, inclusive at the stated cap:

| Input/result | Maximum |
| --- | --- |
| Sprite JSON | 4 MiB, UTF-8 |
| Encoded PNG | 16 MiB |
| License | 64 KiB, non-whitespace UTF-8 text |
| Atlas axes | 2048 × 2048; each axis at least 1 |
| Decoded RGBA | 16 MiB (width × height × 4) |

MiB means 1,048,576 bytes. Regular-file metadata is checked before opening and
again on the open handle. Reads stop at the file-specific cap plus one sentinel
byte, so growth cannot turn them into unbounded reads.

`orr_sprite::SpriteDocument::from_json` supplies the existing format/version,
dimension, region, clip, duration and collection validation. The source JSON is
not normalized or regenerated. `orr_sample::project_sprites::decode_atlas` checks
PNG signature/header, exact document dimensions, decode success and final pixel
count, and rejects animated PNG (use sprite clips instead). It uses the existing
PNG 0.17 decoder with `EXPAND | STRIP_16`: palette and applicable transparency
are expanded, low-bit-depth grayscale is expanded, and 16-bit channels are
reduced to 8-bit. RGB and grayscale output are converted to RGBA with opaque
alpha when absent; explicit alpha is retained. Thus validation is not an
original-color-depth fidelity promise. Stored PNG bytes are still unchanged.

The 16 MiB RGBA cap is **not** a process-memory budget. Encoded inputs, parsed
JSON objects, PNG decoder metadata/internal buffers, its decoded pixel buffer,
and the returned RGBA buffer can coexist. JSON collection limits are checked by
the existing parser after deserialization. This example does not impose a new
PNG decoder allocator, a total-allocation limit, CPU/time limit, or strict
whole-file/trailing-byte consumption policy. Acceptance is the existing
consumer decoder's acceptance; this is not a hostile-input sandbox or a new
image format validator.

## Output and byte preservation

The directory contains exactly these four regular files:

- `sprites.json`: original JSON bytes, including whitespace and ordering
- The exact `atlas.image` filename: original PNG bytes
- `LICENSE.txt`: original supplied license bytes
- `orr.package.json`: generated metadata

Metadata has `schema: 1`, the requested `name` and `version`, `engine: "^0.0.1"`,
`capabilities: ["sprite"]`, `dependencies: {}`, and `files` containing
`sprites.json`, the PNG leaf and `LICENSE.txt`. The manifest is not itself listed
as payload. No cooked RGBA duplicate, hashes or unrelated assets are generated.
The success report records dimensions and decoded RGBA length, not a claim that
a game preview, renderer or playback was exercised. Installation and runtime
consumption are separate explicit workflows; see the verification guide for
actual-consumer test coverage.

## Filesystem trust and failure behavior

All input and output directories must be trusted and remain unchanged by other
processes throughout the operation. Lexical `..` and static symlinks in path
components are rejected. Inputs must be regular files; static FIFOs/devices and
other special files are rejected before opening. Output must not exist, even as
an empty directory or symlink, and its parent must already exist.

All input validation completes before creating output. `create_dir` claims a new
directory without replacing an existing path, and each file uses `create_new`.
On a reported write error, cleanup attempts to remove only that claimed output
under the trusted-directory assumption; cleanup failure is reported explicitly.

These are ordinary path checks and opens, **not race-proof** descriptor-relative
no-follow operations. A malicious concurrent path replacement can invalidate the
checks or cleanup assumptions. The output directory is visible before all files
are written. A crash, kill or power loss may leave a partial directory, and
per-file `sync_all` does not establish directory-entry crash durability. Inspect
and remove such partial output yourself before retrying, or choose a new path.
The command never treats an existing output as resumable or overwritable.
