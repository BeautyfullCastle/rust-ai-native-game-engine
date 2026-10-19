# Offline static-model assembly

`static_model_assemble` produces one ordinary cooked static model from 1–32 keyed
placements of already-cooked static models. It is an optional authoring example,
not a runtime feature, importer, scene editor, mesh baker, or schema change.

```sh
cargo run --locked -p orr_model --features import --example static_model_assemble -- \
  --recipe assemblies/chair.assemble.json \
  --asset-id assemblies/chair.assemble.json \
  --output chair.model.json
```

The output must not exist. Exit 0 means help or successful publication; exit 2
means an error. Without `import`, the executable always fails with exit 2.
The recipe's physical parent anchors source paths; logical asset identity does
not change filesystem resolution. Sources and output require trusted stable,
link-free local ancestry. No overwrite, network access, install, stdin, or force
option is supported. Existing package APIs may install the resulting cooked
file; the assembly CLI does not change package state.

## Recipe

```json
{
  "format": "orr_static_model_assemble",
  "version": 1,
  "parts": [
    {
      "key": "seat",
      "source": "inputs/seat.model.json",
      "placement": [[1,0,0,0],[0,1,0,0],[0,0,1,0],[0,2,0,1]]
    }
  ]
}
```

All fields are required. Unknown/duplicate fields, positional object aliases,
nulls, duplicate part keys and case-only source aliases are rejected. Keys are
1–64 ASCII alphanumeric, hyphen or underscore characters, beginning with an
alphanumeric character; uniqueness is case-insensitive. Logical paths are
normalized portable relative ASCII paths, at most 240 bytes. Sources end in
`.model.json`; recipe identity ends in `.assemble.json`.

Matrices are column-major: the final column contains translation. Each output
primitive uses `placement × source_transform`; source-world transforms already
include source ancestors. Singular, non-affine, nonfinite and out-of-bounds
matrices are errors, as are invalid composed transforms and world vertices.
Identity placement preserves source transform bits, including signed zeros.
No geometry, normal, UV, winding, image bytes, filter or wrap modes are changed.

Unique source paths are admitted fully in first-use order, including unused
materials and images. Each source's entire material/image tables are appended
once, even if identical bytes appear under another source path. Every material
image reference and primitive material reference is checked and offset to its
new table. Repeated placement of the same path reuses those tables. Primitive
order is part order then source order. IDs are the recipe identity followed by
`#node=part:<key>` and the original `#node=...` suffix, so equal original identities
from different sources cannot collide across parts.

The only output dependencies are `$source` (SHA-256 of exact recipe bytes) and
each directly consumed cooked source (SHA-256 of its exact bytes, in first-use
order). Historical dependencies are validated but never resolved or copied.
Recipes are limited to 16 KiB; recipe plus unique source bytes to 64 MiB.
Existing cooked model collection, decoded image, geometry, transform and output
serialization limits all still apply to the aggregate. Failure never publishes
a partial output or replaces an existing file.

## Verification

The additive workflow runs CPU acceptance on Linux and Windows and required
software-Vulkan rendering on Linux. It leaves existing mandatory workflows
unchanged. CPU tests independently construct matrices and table-offset oracles,
exercise the real CLI and keep the Unix socket tests mandatory on hosted Linux.
GPU acceptance installs cooked assembly output using existing public package
APIs, hides authoring sources, reloads installed bytes and compares actual pixels
against independent geometry/material oracles and incorrect-order controls.
Source inventories, retained producer hashes, logs and failure receipts are
preserved. Local model-only results cannot certify hosted GPU acceptance; a
sandbox denial creating a Unix socket is a blocked test, never a passing one.

Placement matrices are cooked affine authoring data, not glTF node TRS; glTF
import rules remain unchanged. Cooked sources use the existing
`StaticModel::from_bytes` semantics, while the recipe and part records require
strict objects. Mirrored determinants use the existing renderer's handling;
assembly does not reverse indices. No export, UI or linked-prefab integration
is claimed by this feature's acceptance.
