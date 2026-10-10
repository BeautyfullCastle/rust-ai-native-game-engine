# Bounded authored Arena export folder

This optional tool packages one existing schema-2 Arena project and an explicitly
trusted, already-built Linux x86-64 runtime. It does not build game code, invoke
Cargo, download/install packages, or create a release/network build identity.
`#104` remains partial: [one offline Arena starter](new-arena-project.md) is now
available separately; other templates, targets, installers and general distribution
compliance remain separate work. [Saved Korean UI/font presets](authored-project-ui.md)
require `game-ui` in both the exporter and the supplied trusted runtime; a project-only
runtime rejects the declared UI even for the zero-tick compatibility check.

## Build and run

Build the tool/runtime from the source you intend to trust:

```sh
cargo build --release -p orr_sample --features project-export --bin arena --bin orr_export_arena
sha256sum target/release/arena
./target/release/orr_export_arena \
  --project assets/saved_arena_project \
  --runtime target/release/arena \
  --runtime-sha256 YOUR_MEASURED_64_LOWERCASE_HEX_SHA256 \
  --trusted-runtime \
  --source-revision YOUR_DECLARED_REVISION \
  --output /existing/parent/new-export
```

For a project declaring `entry.ui`, build both binaries with UI support instead:

```sh
cargo build --release -p orr_sample --features project-export,game-ui --bin arena --bin orr_export_arena
```

Then use that newly measured runtime and the UI-enabled project in the same export
command. See [the saved preset contract](authored-project-ui.md) for its metadata.

`--trusted-runtime` is an operator assertion: **you must already trust the supplied
executable**. Never use an unknown executable just because its hash matches a
string. The exporter executes its staged copy. SHA256 measures bytes; an ELF
header, supplied revision, filename or successful output is not an authentication,
source attestation, capability attestation or safety check. A program could
impersonate the expected output. The expected hash is mandatory; declared source
revision is optional, informational and not inferred from the exporter's checkout.
Only Linux x86-64 is supported. A compatible host loader/system libraries remain
required; ELF target screening does not establish ABI portability.

The command requires an existing nonsymlink parent and a new output leaf. It
rejects every existing destination, including an empty directory or dangling
symlink, and rejects output/source overlap. The final directory appears only after
all checks succeed. Run or relocate the resulting folder:

```sh
cd /some/unrelated/empty-directory
'/existing/parent/new-export/run-arena' --headless --ticks 0
'/existing/parent/new-export/run-arena' --headless --ticks 120 --hold right,fire
'/existing/parent/new-export/run-arena'
```

The fixed launcher derives its directory, quotes paths (including spaces), and
passes an absolute bundled project path. Launcher arguments retain the existing
Arena CLI rules. A display and graphics drivers are required for the windowed
route; the export acceptance suite uses headless runtime-compositor GPU readback.

## Exact output closure

- `bin/arena`: independent byte copy of the supplied runtime, output mode `0755`
- `run-arena`: fixed launcher, output mode `0755`
- `project/orr.project.json`: exact source bytes
- `project/orr.packages.lock.json`: exact bytes if present; valid absence stays absent
- The exact entry scene and optional sprite sidecar
- Every active package's installed `orr.package.json` and every locked file,
  preserving `.orr/packages/objects/<digest>/...`
- `orr.export.json`: deterministic content manifest, mode `0644`

Project payload files have normalized `0644` modes. Their bytes are not rewritten.
The package manager remains the only activation/version/digest authority:
`Project::list/verify/read_asset` and `PreparedRuntime` provide admission. The
exporter neither resolves a second dependency graph nor prunes assets by current
sprite usage. The fixture has **nine** exported project files: its package license
and currently unused `lantern_keeper.rgba` stay included; its root README does not.
For a UI-enabled project, the existing font package contributes its exact font,
`OFL.txt`, `COPYRIGHT.txt`, `font-manifest.json`, `corpus.txt` and package manifest.
The font remains OFL-1.1 with its separate notices; it is not regenerated or moved
outside the package closure. Neither the package metadata nor this exporter
establishes general distribution compliance.

Inactive objects, unrelated root files, package writer/staging files, source
repositories, build trees and caches are not recursively copied.

An asset explicitly declared by a package is runtime content, even if it contains
editor information or secrets. The exporter does not infer secret status, prune
authoritative files, or guarantee distribution/license compliance. Preserving
package license files does not collect executable/dependency notices or system
libraries. The workspace has license metadata; that is not a complete distribution
notice bundle.

## Transaction and resource bounds

The source manifest/optional lock are pinned before validation. Admission bounds
all listed files before whole-package verification; exact scene/sidecar identity,
GUIDs, clips, atlas formats and every active capability/digest use the existing
project runtime validators. Source file identity and hashes are rechecked after
copying and again before publication, including ordinary writes through hardlink
aliases. Input hardlinks are accepted (normal Cargo output can have multiple
links); output files are fresh inodes. Input permissions are never changed.

Limits are 8,192 project files, 256 MiB total project bytes, 64 MiB per package
asset, 1 MiB each for project/package manifests, lock and sidecar, 4 MiB for the
scene, and 512 MiB for the runtime. Export-relative paths have explicit bounded
length/components; existing portable package/entry path rules still apply.
Manifest output is capped at 4 MiB. Symlink ancestors, symlink files and special
files (including FIFOs) are rejected before opening.

A private uniquely created sibling stage uses create-new file writes. The staged
project is readmitted/baked. The copied runtime is streamed and hash-checked again before execution, then
invoked from an empty cwd using
`--project <stage/project> --headless --ticks 0`; stdout must exactly match the
shared initial checksum and entity state. The smoke uses a cleared environment. Execution is bounded to 10 seconds and
4 MiB combined stdout/stderr; failure, malformed output, timeout, bad loader or
mismatch prevents publication. The owned process group is terminated on every
terminal path before its direct child is reaped, including redirected descendants. This proves admission of this particular project
on this host, not general binary capability or trust. Staged payload hashes/modes
and the complete allowlist are checked again after execution.

Publication uses `rustix::fs::renameat_with(..., RenameFlags::NOREPLACE)` (Linux
`renameat2`). Unsupported kernel/filesystem behavior fails closed: there is no
`std::fs::rename` fallback. A concurrent destination creator cannot be replaced.
Ordinary failures remove only this invocation's stage; previous exports and
unrelated stages remain untouched. Crashes can leave an uncommitted stage.
These checks address ordinary source changes, not a hostile concurrent writer,
security sandbox or power-loss durability guarantee.

## Deterministic manifest and identity

`orr.export.json` schema 1 contains `payload` and `content_digest`. Payload fields
are serialized as compact JSON in declared struct order; package names use sorted
maps and file entries sort by relative path. The file list records role, normalized
mode, byte length and lowercase SHA256. It excludes the manifest itself. The
manifest ends with one newline. It has no timestamps, absolute paths or stage names.

The content digest is SHA256 of the bytes
`orrery.arena.export.content.v1\0` followed by compact serialized `payload` (no
newline). It covers exporter version, fixed profile, entry metadata, active package
identities, all payload file identities, measured runtime bytes/hash, explicitly
declared provenance and admitted initial checksum. Identical bytes/tool version/
declarations produce identical manifests across output directories. This does not
claim reproducible native compilation. The digest is **not** the Arena game/network
build ID, and the existing authored route still rejects network `--build-id`.

## Verification boundaries

Focused tests cover exact inclusion, source-byte preservation, independent output
inodes, absence of a scene-only lock, deterministic manifests, existing/concurrent
destinations, transaction failures, source/stage mutation, special files and bounds,
binary target/hash screening, exact smoke output and timeout/output limits. Mocked
smoke unit tests exercise transaction failures; separate integration tests execute
the actual built Arena binary.

Mandatory integration mode relocates an actual generated export, hides the whole
workspace and original project/runtime, and executes from an empty cwd. It compares
initial/movement/fire/relaunch checksums and reads back the real runtime compositor:

```sh
ORR_REQUIRE_PROJECT_ISOLATION=1 ORR_EXPORT_HIDE_ROOT=/absolute/workspace/root \
  ORR_REQUIRE_GPU=1 \
  cargo test --release -p orr_sample --features project-export --test project_export
```

Set `ORR_EXPORT_HIDE_ROOT` to the existing workspace root containing the checkout
and any sibling checkouts/build sources; without it, the test hides only the repository.

Exact-head independent review and integrator-owned required CI/development-union
verification are separate gates. Native-window interaction, physical-GPU hardware,
Windows/macOS export/local execution, system-library bundling, general deployment
and full-workspace success are not implied by these focused tests.

Primary filesystem references:
- https://docs.rs/rustix/1.1.5/rustix/fs/fn.renameat_with.html
- https://man7.org/linux/man-pages/man2/rename.2.html
