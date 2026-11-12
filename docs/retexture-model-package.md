# Package a model with a replacement PNG material

On Linux, build `orr_pkg` with the optional `model-authoring` feature. The
`create-retextured-model` command replaces the texture of one used material and
saves a complete installable static-model package:

```sh
cargo build --locked -p orr_package --features model-authoring --bin orr_pkg
orr_pkg create-retextured-model --recipe /work/props/paint.retexture.json \
  --asset-id variants/paint.retexture.json --license /work/props/LICENSE.txt \
  --name painted-prop --version 1.0.0 --engine '^0.0.1' \
  --output /work/new-painted-prop
```

Recipe example:

```json
{
  "format": "orr_static_material_texture", "version": 1,
  "source": "prop.model.json", "texture": "paint.png", "material": 0
}
```

Both input paths resolve relative to the recipe. The selected material must
exist and be used by at least one primitive. A new image is appended to the
cooked model and only that material's image reference changes. Other materials
sharing the original image keep it. UVs, geometry, transforms, base color,
filtering and wrap settings remain intact. Texture colour still multiplies the
material's base colour under the existing renderer; alpha does not introduce
transparency in the opaque renderer.

Use a static PNG that decodes to exact RGBA8 under the existing retexture
policy. Plain RGB/grayscale and APNG are rejected. No resizing, gamma conversion
or pixel painting is added. Width and height are at most 2048. The new image
counts toward the existing 32-image and 32-MiB decoded-image limits. The complete
recipe, cooked source and PNG share a 64-MiB input budget; recipe size is at most
16 KiB. Duplicate/unknown/missing recipe fields and invalid models are rejected.
The logical output asset ID ends in `.retexture.json`. Exact recipe, source
model and encoded PNG hashes identify provenance.

The output contains `model.model.json`, the byte-for-byte supplied `LICENSE.txt`
and a `models`-only `orr.package.json`. No original model or PNG directory is
needed after installation. Use existing `orr_pkg inspect`, `install` and `verify`
commands, then verify installed packages in a saved Room project's Content
packages panel. Choose the model, assign it and save model bindings through the
existing workflow. Creation does not install or assign automatically.

Publication reuses the existing synchronized, no-replace static-model package
publisher. Existing files, directories and links are never overwritten. Inputs
and output ancestry must be trusted, stable and symlink-free. License text must
be nonempty UTF-8 (at most 1 MiB); it is preserved, not interpreted. No external
transformer or shell is run. Unknown, duplicate and external-executable options
are rejected. Exit 0 means published (read any durability warning), 1 means
creation failed, and 2 means usage error. Unsupported platforms or feature-off
builds reject the command before opening inputs or output. The legacy
`static_model_retexture` example still uses the same shared implementation.

Focused checks cover the actual CLI, package installation and reloading the
embedded replacement pixels after hiding original input/package directories,
unchanged geometry/material settings, corrupted and linked PNGs, bad metadata,
and destination preservation. Native-window, GPU and exported-player appearance
remain separate acceptance checks.
