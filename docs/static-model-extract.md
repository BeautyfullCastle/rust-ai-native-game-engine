# Extract a reusable static prop from a GLB

This optional, GPU-free authoring tool extracts one source-node subtree into the
existing `orr_static_model` version-1 cooked format. Use it to separate a prop
from a larger supported static scene before explicitly packaging and assigning
it. This is a bounded part of #99/#94, not a general modeling editor.

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_extract -- \
  --input scene.glb --asset-id models/scene.glb --list
cargo run --locked --release -p orr_model --features import \
  --example static_model_extract -- \
  --input scene.glb --asset-id models/scene.glb --node 3 \
  --output /existing/trusted/directory/chair.model.json
```

Listing returns JSON records with the original numeric `index`, optional `name`
and subtree `primitive_count`, sorted by index. Only nodes reachable in the
source's default scene with nonempty imported geometry are listed. Names may be
missing or duplicated; select by index. The logical `--asset-id` is the normalized
source-relative identity used by the importer, independent of the machine's
input path and output filename. It is required in both modes. The default scene
is glTF's explicit `scene`, or scene zero when omitted.

Extraction requires exactly the four option/value pairs shown above; listing
requires `--input`, `--asset-id` and the standalone `--list` flag. Options may be
reordered. `--help` is supported alone. The existing `import` feature enables the
tool; a feature-disabled executable fails clearly. No Cargo dependency, runtime
format, importer behavior or public runtime API changes.

## Selection and preservation

The entire supported source is admitted by the unchanged production static
importer before selection. The tool never makes unsupported source acceptable by
dropping its unwanted branch. A missing node, a node outside the imported default
scene, or a subtree without imported primitives is an error.

Selected primitives retain their original traversal order, stable source-slot
IDs, vertices, triangle indices, normals, UVs and fully composed source-world
transforms. Ancestor transforms remain effective even when their geometry is
omitted. Mirrored and nonuniform transforms retain the existing renderer's
normal/winding behavior. There is no centering, pivot rewrite, hierarchy editing,
vertex welding, material optimization or generated collision.

Complete material and image tables remain unchanged, including unused entries,
so all indices keep their original meaning. Extraction therefore need not reduce
texture memory or output size. The logical source identity and original exact
GLB SHA-256 dependency remain. Packaging the new cooked asset establishes its
own file hash through existing consumers; this differs from the embedded source
provenance hash. Keep the original licensing notices when authoring a package.

The result must pass `StaticModel::new`, bounded cooked serialization, and a
fresh cooked reload with identical source data before publication. Output is a
self-contained cooked JSON model with decoded images. It is not another GLB and
has no hierarchy for a second node-subtree extraction.

## Supported source and limits

Accept only GLB 2 with exactly one aligned JSON chunk and one aligned embedded
BIN chunk. Exactly one buffer without a URI is allowed; images must use embedded
buffer views. External paths, data URIs, unknown/additional chunks, animation,
skins and unsupported source fields/extensions are rejected. All existing static
importer restrictions apply, including its supported diffuse material subset.
This does not add the separately held UV-extension or animation importer work.

Existing limits remain: 64 MiB source/cooked file, 4 MiB source JSON, 1,024 nodes,
64 graph depth, 1,024 rendered primitives, 250,000 expanded vertices, 750,000
indices, 256 materials, 32 images and 32 MiB decoded image bytes. Full-source
admission still applies even if the chosen subtree is small. Cooked JSON can be
larger than the source GLB and must independently fit the output bound. Selection
uses bounded node masks; unwanted geometry is not cloned into the result.

## Filesystem contract

Input and output ancestry must be trusted and stable. Input is a bounded regular
file; symlink ancestry, nonregular files and parent traversal are rejected. Path
components ending in a dot/space and nonlocal Windows network/device prefixes
are rejected before metadata access; drive-relative paths are unsupported.
Opened-handle identity/metadata and bounded reads are checked. These portable
checks are not a capability sandbox against hostile concurrent same-user/root
directory replacement or in-place mutation; a path replacement can race open,
including substituting a FIFO. There is no stdin, URI resolver or network-fetch API. Filesystem paths can
resolve onto mounted storage; callers must supply trusted local directories.

The output parent must already exist. Its portable ASCII basename must end in
`.model.json`, be at most 100 bytes and avoid reserved Windows device names.
An ancestor lexically named `.orr` (case-insensitive) is rejected. This is not a
package-store capability sandbox: trusted callers must not target an installed
store through alternate directory names, short-name aliases or mounts. An
existing destination, source alias
or dangling symlink is never replaced; there is no `--force` option.

A uniquely created sibling stage is written, synced and read back, then
hard-linked to the absent destination. Hard-link support is required; there is
no copy/replace fallback. Cleanup applies only to the owned stage. Before
publication, failure leaves no newly published destination. After publication,
cleanup/directory-sync failures produce warnings and retain the destination.
Non-Unix directory sync is unavailable through this implementation. No hardware
power-loss durability guarantee is claimed.

Exit 0 means help, successful listing or publication (check warnings). Exit 2
means error and no destination published by that operation. Listing writes only
to stdout and does not create files.

## Existing consumer path

Include the new cooked file and licensing notices in a new package or explicitly
authored immutable version. Install it through the existing package workflow,
then Load/Assign the model through the existing editor and save model bindings.
The production model loader already accepts cooked static models, and existing
Room runtime/export routes consume them. The tool does not install, activate,
relink, overwrite, mutate a package lock, change a scene or automatically change
collision. Assignment of a different package/asset is an explicit separate step.

## Verification boundary

Model CPU tests cover independent source fixtures, default-scene/subtree
selection, source-slot/material/UV/transform preservation, exact digests,
malformed/unsupported/full-source admission, CLI paths and failure preservation.
Bindings acceptance separately exercises installation, production loading,
save/reopen and corruption detection. Software-device tests separately check
selected and omitted geometry against independent transform/UV/material oracles.
Room export acceptance uses the real authoring executable and copied production
runtime/exporter with source-hidden isolation, original/export pixel and state
parity. Ignored device tests must be invoked explicitly; inventory or CPU passes
alone do not establish GPU, editor, export, native-window or physical-device
acceptance. Local validation and mandatory exact-head hosted CI remain distinct.

```sh
cargo test --locked --offline --release -p orr_model --features import \
  --test static_model_extract
cargo clippy --locked --offline --release -p orr_model --features import \
  --all-targets -- -D warnings
```
