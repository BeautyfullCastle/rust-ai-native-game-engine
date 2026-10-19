# WAV audio package import verification

This acceptance slice covers offline import and installed package integrity. It
uses the real `wav_audio_package` example and existing `orr_pkg` executable,
including the CLI's metadata-only `Project::open_for_install` path. It does not
supply a pretend compiled `collect-audio` capability or use `read_asset` to bypass
consumer admission. Missing-capability asset loading remains the real consuming
host’s responsibility and is not exercised through metadata-only installation.

## Reproduce

Build and check the example and package CLI:

```sh
cargo fmt -p orr_asset_cook --check
cargo test --locked -p orr_asset_cook --example wav_audio_package
cargo test --locked -p orr_asset -p orr_asset_cook
cargo clippy --locked -p orr_asset_cook --all-targets -- -D warnings
cargo build --locked -p orr_asset_cook --example wav_audio_package
cargo build --locked -p orr_package --bin orr_pkg
python tools/check-wav-audio-package.py --evidence target/wav-import-acceptance --importer target/debug/examples/wav_audio_package --package-cli target/debug/orr_pkg
```

The evidence directory must not exist. On Windows append `.exe` to executable
paths. If using a custom Cargo target directory, pass the corresponding binary
paths. The harness preserves command logs and writes its receipt only after all
checks pass; a failure must not be described as completed acceptance.

## What the real CLI harness checks

- Generates an original 480-frame mono PCM16 48 kHz waveform locally and includes
  explicit permission to use, copy, modify and distribute it for any purpose
- Imports the WAV with explicit asset ID, name, version, license and new output
  directory; compares complete package bytes after source/output relocation
- Independently reconstructs the exact cooked PCM bytes and ORAM v1 View
  manifest, including stable asset ID, type, schema, length and SHA-256 digest
- Checks the exact package file inventory and unchanged license bytes
- Rejects malformed WAV, existing output, source directory/file collisions,
  missing/unknown/duplicate flags without changing source or existing outputs
- Runs actual `orr_pkg inspect` and `install` in an isolated schema-1 project;
  checks installed lock file hashes against independent SHA-256 computation
- Relocates generated input WAV/license trees and package sources away from their
  original paths (retaining them as evidence), then runs
  real `list`, `explain`, whole-project `verify` and scoped `verify --package`
- Confirms installed immutable snapshot bytes and project metadata are unchanged
- Rejects changed package contents at the same name/version without changing the
  project; rejects a tampered copy of the installed payload without repairing it
- Rejects a static FIFO source without blocking on Unix; Windows records this
  platform-specific check as not applicable

`explain` is lock-metadata inspection, not installed-byte verification. The
explicit `verify` operations establish installed-byte integrity. Integrity is
not authenticity; input and output directories must be trusted and must not be
concurrently modified by another process.

## CI and scope limits

The additive `WAV audio package import acceptance` workflow runs on Linux and
Windows, retains logs on failure, tests the example and existing asset/cooker
contracts, applies strict Clippy, and executes the real CLI harness. Existing
mandatory workflows and golden checksums are unchanged. Local results do not
stand in for hosted CI or Windows execution.

Audio preview, playback and export require a compatible consuming host, including
the audio integration represented by PR #153. That host is absent from this
import branch. No audio device, listener experience, editor preview, exported
application, or end-to-end audio workflow is verified by this acceptance slice.

## Local acceptance result

Linux acceptance passed on 2026-10-19: 21 example tests (14 parser and seven
package/CLI tests), 13 existing asset tests, 28 existing cooker tests and one
compile-fail asset documentation test (62 unit/integration cases plus one
compile-fail documentation case in total). Formatting and all-target cooker Clippy
with warnings denied passed. The real CLI harness passed all checks above.

An initial formatting check failed on the newly authored example; formatting was
applied and the check rerun successfully. No existing test, golden, or mandatory
CI was weakened. Windows and hosted CI results remain separate requirements.
