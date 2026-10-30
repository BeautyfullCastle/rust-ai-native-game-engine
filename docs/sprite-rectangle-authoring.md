# Irregular atlas rectangle authoring

This optional Python 3 tool creates an installable sprite content package from an
existing PNG atlas, explicit rectangles and named clips. It fills the irregular
atlas workflow; use `png_grid_package` for full regular grids and
`png_sequence_package` for separate PNG frames instead.

```sh
python3 tools/create_sprite_package.py \
  --packer /trusted/bin/png_sprite_package \
  --png hero.png --license LICENSE.txt \
  --name hero --version 1.0.0 --out new-hero-package \
  --atlas-size 128 64 \
  --region 10 3 2 24 30 --region 20 41 5 29 28 \
  --clip idle loop 10:200 \
  --clip attack once 10:80,20:120,10:80
```

Each `--region` takes `ID X Y WIDTH HEIGHT`, in top-left atlas pixels. Each
`--clip` takes `ID MODE REGION:DURATION_MS,...`; mode is `loop` or `once`.
Quote names containing spaces. Explicit IDs, reference order, repeated references
and integer millisecond durations are preserved. No grid inference, trimming,
rotation, resampling, pivot generation or image re-encoding occurs.

## One authoritative format and validator

The tool emits existing `orr_sprite` version-1 JSON, not a new recipe/schema.
Its atlas image name is always `atlas.png`. The supplied trusted local
`png_sprite_package` executable must already be built with `project-create`.
The wrapper uses fixed process arguments, never a shell, PATH search, download
or implicit build. Selecting the executable authorizes running local code; this
is not a sandbox for untrusted binaries. No third-party Python dependencies,
Cargo/core/runtime changes or mandatory feature are introduced.

Python parses bounded decimal arguments and serializes the document. Semantic
admission remains exclusively in Rust `SpriteDocument::from_json` and the
existing packer's PNG decoder: duplicate IDs, out-of-bounds rectangles, missing
references, invalid modes and zero/overflowing durations are Rust errors.

Allocation guards cap raw arguments and emitted UTF-8 JSON at 4 MiB, regions at
4096, clips at 256, references per clip at 4096 and total references at 16384.
Integer tokens are at most 21 characters. These are authoring resource guards,
not a second schema validator. The packer's existing limits also apply: PNG
16 MiB, nonempty UTF-8 license 64 KiB, atlas axes 1–2048 and decoded RGBA 16 MiB.

## Output and failure behavior

The output directory's parent must exist and the destination must not exist.
The Rust packer validates before publication and creates exactly `sprites.json`,
`atlas.png`, `LICENSE.txt` and `orr.package.json`; original PNG/license bytes are
preserved. Existing directories, files and symlink destinations are not replaced.
The wrapper deletes only its private temporary JSON directory on ordinary exit,
never the package destination, and propagates packer failures (signals become
exit 1). CLI syntax errors use exit 2; wrapper input/launch errors use exit 1.

Directories, including the selected system temporary directory (`TMPDIR`), must
remain trusted and quiescent and have no symlink ancestors. The packer may clean
up only its newly claimed output after a reported write failure. Existing packer symlink/path
checks are not protection against malicious concurrent path replacement.
Publication is not atomic or crash-durable; interruption may leave partial
output. Inspect it yourself or choose a new path—there is no overwrite, resume,
automatic retry or wrapper destination cleanup. See
[png-sprite-package.md](png-sprite-package.md) for the underlying contract.

## Verification and next use

Run isolated unit tests (mock process invocation):

```sh
PYTHONPATH=tools python3 -m unittest test_create_sprite_package.UnitTests -v
```

Run actual integration separately with a verified trusted binary:

```sh
ORR_RECTANGLE_PACKER=/verified/bin/png_sprite_package PYTHONPATH=tools \
  python3 -m unittest test_create_sprite_package.RealPackerTests -v
```

Integration checks actual Rust admission, invalid documents, unchanged existing
outputs and preserved source bytes. Missing the environment variable skips those
checks and is **not** integration success. Install the resulting content package
through the existing package manager and assign its regions/clips through existing
sprite bindings. This tool does not install packages, edit a scene, launch a host
or prove rendering/playback acceptance.
