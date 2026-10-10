# Local content packages v1

`orr_package` is an optional, GPU-free filesystem/tooling crate. It does not enter
simulation state or force rendering, fonts, audio, an editor, or a network stack
into minimal simulation crates. Its `orr_pkg` binary manages offline content;
Cargo remains responsible for Rust code and compiled features.

## Contracts

A local package directory contains `orr.package.json`:

```json
{
  "schema": 1,
  "name": "sample-sprites",
  "version": "1.0.0",
  "engine": "^0.0.1",
  "capabilities": ["sprite"],
  "dependencies": {},
  "files": ["atlas.rgba", "sprites.json"]
}
```

- All contract objects reject unknown fields. No scripts, hooks, native plugins,
  executable discovery, downloads, or Cargo rewrites exist.
- Names/capabilities are 1–64 lowercase ASCII letters, digits, `_` or `-`.
- Package versions and dependency values are canonical exact semver strings,
  including optional prerelease/build metadata. Dependency ranges are rejected.
  Engine compatibility uses a semver requirement.
- `files` explicitly declares content. Unlisted source files are neither copied
  nor loaded, so installing a directory does not grant access to its other files.
- Paths are relative portable ASCII, at most 240 bytes, 16 components, and 100
  bytes per component. Components allow letters, digits, `_`, `-`, `.`; traversal,
  empty components, drive prefixes, backslashes, trailing dots, DOS device names,
  case-insensitive collisions, manifest replacement, and file/directory collisions
  are rejected. Every accessed path component is checked for symlinks. Inputs must
  be regular files, checked before opening so FIFOs/devices cannot block reads.
- Bounds: 1 MiB JSON, 4096 files/package, 64 MiB/file, 256 MiB/package and per
  installation transaction, 128 packages/dependencies, 128 capabilities/package.
  Oversized resulting locks are rejected before publication.

An optional, user-owned `orr.project.json` constrains the engine:

```json
{"schema": 1, "engine": "^0.0.1"}
```

The manager never changes it. Direct selections and the resolved graph live in
one `orr.packages.lock.json` with `schema`, `direct` (name → exact version), and
`packages` (name → `{manifest, digest, files}`). `files` maps each declared path
to its lowercase SHA-256. `digest` is SHA-256 of compact serde JSON encoding of
the tuple `["orr-package-v1", manifest, files]`. Manifest fields are serialized
in their documented Rust declaration order: `schema`, `name`, `version`, `engine`,
`capabilities`, `dependencies`, `files`. Sets and maps are sorted BTree collections.
JSON whitespace/source location is irrelevant. Engine requirements remain strings;
equivalent but differently written requirements intentionally have different
identities. The lock includes full manifests, so it can validate dependency edges,
engine constraints and capabilities without reading mutable package sources.

The lock is the sole activation authority. There is one version per package name.
Resolution detects missing packages, exact-version conflicts, cycles, unreachable
lock entries, invalid content identities and incompatible runtime inventories.

## Commands

```sh
cargo run -p orr_package --bin orr_pkg -- inspect ./my-art
cargo run -p orr_package --bin orr_pkg -- install ./my-game --path ./my-art --dependency-path ./dependency-art
cargo run -p orr_package --bin orr_pkg -- list ./my-game
cargo run -p orr_package --bin orr_pkg -- explain ./my-game my-art
cargo run -p orr_package --bin orr_pkg -- verify ./my-game
cargo run -p orr_package --bin orr_pkg -- remove ./my-game my-art
```

Each `--path` is an explicit direct selection; `--dependency-path` supplies a
candidate without selecting it directly. Only reachable candidates are activated.
Dependencies resolve from sources in that transaction or already active installed
packages. Supplying a
new version updates that direct selection if the complete graph remains valid.
Reinstalling the same active name/version with changed content fails: bump its
version. Package digests, not name/version alone, identify exact content.

`explain PROJECT NAME` reads the validated lock and reports the package's exact
version/digest, `direct` selection, declared `capabilities`, immediate `required_by`
dependents, and `selected_by` chains from every direct root that reaches it.
Each chain includes both endpoints; a direct selection includes `[NAME]`.
It reports one shortest chain per root, breaking equal-length ties by sorted
package name, rather than expanding every possible path. This explains why a
package remains active after removing its own direct selection: other roots may
still require it. Removing the last selecting root makes `explain` fail with
`missing package`. No lock, source, installed object or writer guard is changed.

Explanation is **lock metadata only**: it does not read or verify installed bytes
(use `verify` for that). Listed capability names are requirements, not proof that
code was compiled. Like the other management commands, CLI capability validation
is deferred to the consuming host. `Project::explain` uses the same lock admission
as `list`, so a host opened with its actual inventory still rejects unsupported
capabilities, invalid identities and invalid dependency graphs.

`remove` drops a direct selection and recomputes reachability. A package required
by another direct selection remains installed transitively. Removing that last
root makes it inactive. Removal never deletes source files, scene files, Cargo
files, or stored objects. Reinstalling unchanged sources restores identical
content identity. There is no garbage collector in v1.

Commands print JSON results; failures exit 1 without replacing the old active
lock. CLI success means content/engine validation only. The CLI always reports
that compiled-capability validation is deferred to the consuming application;
there is no `--capabilities` switch that can claim a feature was compiled.

## Application loading

```rust,no_run
use orr_package::{Project, Runtime};
let mut runtime = Runtime::content_only();
// Only add this in the actual sprite-enabled host build.
#[cfg(feature = "sprites")]
runtime.capabilities.insert("sprite".into());
let project = Project::open("./my-game", runtime)?;
let document = project.read_asset("sample-sprites", "sprites.json")?;
let pixels = project.read_asset("sample-sprites", "atlas.rgba")?;
# Ok::<(), orr_package::Error>(())
```

The host must truthfully construct its inventory from compiled features. Package
or project JSON cannot enable code. `Project::open_for_install` enables metadata-
only management, and deliberately refuses asset reads. `Project::open` enforces
the actual inventory when reading/validating the lock. `read_asset` returns owned
bytes only after checking the requested declared file's hash; it never returns an
unchecked filesystem path. A removed package gives a clear `missing package`
error. The caller decodes/validates sprite JSON and RGBA metadata before rendering.
Installed asset edits cause an integrity error. `verify` checks every file and
stored manifest; `read_asset` checks the complete locked graph and requested bytes.

## Transaction and trust boundary

Objects live at `.orr/packages/objects/<digest>/`. Each object is built in a
unique sibling staging directory and renamed into place only when complete.
Existing objects are verified and never overwritten. The old active lock remains
until all objects are ready. A synced temporary lock in the project directory is
published by one atomic rename/replacement. Direct choices and the graph therefore
cannot tear across two manifest files. A failed install may leave unreferenced
objects but cannot activate a partially installed package.

A `create_new` `.orr/packages/writer.lock` serializes manager mutations. Normal
errors release it; a killed process may leave a stale guard. After confirming no
writer remains, the user may remove that guard and retry. Automatic lock stealing
is intentionally absent. Readers see either old or new lock data. Process-
interruption atomicity is covered by this design; filesystem-specific power-loss
durability of directory metadata is not claimed.

The project directory and source tree must not be concurrently modified by an
adversarial process. Component checks reject existing symlinks, but are not an
OS-level sandbox against malicious filesystem races. Object immutability is a
manager guarantee, not a permissions lock; verification detects later byte edits.
SHA-256 establishes integrity against the recorded lock, not publisher identity:
a malicious party able to rewrite both lock and assets remains trusted. No signed
registry/authentication claim is made. Arbitrary content is never executed.

Windows implementation/execution is deferred. Portable path rules are applied
on Linux, but Windows rename, junction/reparse-point and filesystem behavior are
not validated and must not be counted as Windows acceptance evidence.

## Focused checks

`cargo test -p orr_package` covers install/list/verified load/remove/reinstall,
source isolation, repeatable offline digest, exact versions, engine requirements,
missing dependencies, conflicts, cycles, actual host capabilities, lock retention
on failure, stale writer guards, tampering, path collisions, hooks rejection and
Unix symlink rejection. FIFO asset/manifest rejection runs in a timeout-bounded
subprocess, so a regression cannot hang the test suite. Additional cases cover
numeric/path/graph bounds, forged lock identities/maps, unreachable packages,
ancestor symlinks, reserved manifest subtrees, and competing writers. Thread-local,
test-only failpoints exercise post-staging and pre-lock-publication failures;
these hooks are absent from production builds. `cargo clippy -p orr_package --all-targets -- -D warnings`
checks the standalone CPU-only crate. End-to-end rendered/playable-scene evidence
belongs to the sample integration, not these filesystem unit tests.
