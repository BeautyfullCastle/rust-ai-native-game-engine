# Offline WAV to Collect SFX package

This deliberately narrow, dependency-free Cargo example converts one user-supplied
WAV and license into a new package source directory. It does not install a package,
modify a project, bind an event, preview or play audio, or export a game. Installing
the result proves metadata/content integrity only, not playback or host capability.

```sh
cargo run -p orr_asset_cook --example wav_audio_package -- \
  --wav /trusted/source/collect.wav \
  --asset-id a_0000000000000042 \
  --name collect-sfx --version 1.0.0 \
  --license /trusted/source/LICENSE.txt \
  --out /trusted/output/collect-sfx
```

All six options are required exactly once. `--help` prints usage. Assign the
nonzero asset ID explicitly and retain it across revisions: it is a stable
reference, not a content hash. Its spelling is `a_` plus 16 lowercase hex digits.
Package names contain 1–64 lowercase ASCII letters, digits, hyphens or underscores.
Versions use canonical SemVer (including optional prerelease/build identifiers),
with a 256-byte tool limit. No network or additional dependencies are needed.

## Accepted input

- RIFF/WAVE PCM16 little-endian, mono, 48000 Hz
- 1–48000 frames, every sample in the inclusive range −8192 through 8192
- No resampling, clipping, normalization, stereo mixing or compressed/float decoding
- At most 1 MiB of WAV bytes, including ancillary chunks
- One explicit nonempty UTF-8 license file, at most 64 KiB, copied byte-for-byte;
  supplying it does not establish that you have permission to redistribute the WAV

The parser checks exact RIFF length, bounded chunks and padding, one format chunk
and one data chunk (either order), and consistent block alignment/byte rate. Unsupported or
malformed inputs fail without creating output. Source files are never modified.

## Output contract

The package contains exactly four files:

- `orr.package.json`: schema 1, supplied name/version, engine `^0.0.1`, capability
  `collect-audio`, no dependencies; its explicit files list names the three below
  and does not list its own metadata
- `LICENSE.txt`: exact supplied license bytes
- `cooked/view.manifest.bin`: public ORAM codec, Domain::View, one nonzero ID,
  type 2 / schema 1
- `cooked/objects/<sha256>.bin`: SHA-256 of the exact existing Impact PCM payload
  (u32 little-endian rate, u32 frame count, then i16 samples)

Identical input bytes/options yield identical output bytes, regardless of paths.
The existing cooker Impact payload validator is run before publication. This is a
package source, not a full general-purpose cooker bundle: it intentionally has no
simulation manifest or generated simulation code.

## Filesystem and failure boundary

Use trusted directories that no other process changes during this command. All
existing path components are checked for symlinks, including ancestors and dangling
links; lexical `..` is rejected. Source leaves must be regular files before they
are opened, so static FIFOs/devices/directories are rejected without reading. File
size is checked before open, after open, and with a bounded read. These portable
standard-library checks do not provide race-proof `openat`/`O_NOFOLLOW` isolation:
a concurrent hostile replacement could still change an input or block an open.

The output parent must already exist. Checked publication exclusively creates a
new output directory; existing files, directories and links are never deliberately
overwritten. The directory is visible while files are written, so consumers must
wait for successful command exit. Files use create-new writes and are synced.
Ordinary write failures remove the newly claimed directory; cleanup failure is
reported explicitly. A killed process or power failure can leave a partial output;
inspect it yourself and use a different output path for a retry. This is not a
transactional install or a crash-durable directory publication protocol.

## Checks

```sh
cargo test -p orr_asset_cook --example wav_audio_package
cargo clippy -p orr_asset_cook --example wav_audio_package -- -D warnings
```

The example's unit tests cover parser boundaries, deterministic public-codec
output, metadata, source preservation, no-clobber, validation failure, bounded
reads, cleanup, and static symlink rejection. See the separate verification guide
for cross-crate package installation checks; these are content-only checks.
