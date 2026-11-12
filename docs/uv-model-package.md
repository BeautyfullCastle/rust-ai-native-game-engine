# Create an installable UV model variant

On Linux, the optional `model-authoring` command connects the existing cooked
UV transform directly to package creation. Build `orr_pkg` with that feature:

```sh
cargo build --locked -p orr_package --features model-authoring --bin orr_pkg
orr_pkg create-uv-model --recipe /work/props/tile.uv.json \
  --asset-id variants/tile.uv.json --license /work/props/LICENSE.txt \
  --name tiled-prop --version 1.0.0 --engine '^0.0.1' \
  --output /work/new-tiled-prop
```

A recipe selects one used material in a cooked static model:

```json
{
  "format": "orr_static_material_uv", "version": 1,
  "source": "prop.model.json", "material": 0,
  "scale": [2.0, 0.5], "quarter_turns": 1, "offset": [0.25, -0.5]
}
```

Source paths resolve relative to the recipe. UV0 is scaled, rotated around the
UV origin by zero through three counter-clockwise quarter turns, then offset.
Scale must be positive and finite; transformed values must be finite and within
±65536. Only primitives using the selected material change UVs. Geometry,
transforms, textures and sampler settings remain intact. Repeat or Mirror
samplers are needed for repeating textures; this command does not change them.

The existing strict recipe/model validation applies. Unknown, duplicate, missing
and wrong-type fields are rejected. The new logical asset ID ends in `.uv.json`.
Recipe and exact cooked source bytes become provenance dependencies; old source
images and model dependencies are not read again.

The output contains exactly `model.model.json`, `LICENSE.txt` and
`orr.package.json`, declaring the `models` capability. The user-supplied license
is copied byte-for-byte; the command does not determine asset rights. Then use
existing `orr_pkg inspect`, `install` and `verify` commands. In a saved Room
project, verify installed packages in Content packages, choose the model, assign
it and save model bindings through the existing workflow.

Publication reuses the static assembly package's bounded, synchronized staging
and no-replace directory rename. Existing files, directories and links are never
replaced. Use trusted, stable, symlink-free input/output ancestry. Inputs are
bounded: recipe 16 KiB, cooked input plus recipe 64 MiB, license 1 MiB. No child
executable or installer runs during creation. `--transformer`, executable paths,
unknown and duplicate flags are rejected.

Exit 0 means published; inspect any durability warning without blindly retrying.
Exit 1 means creation failed and 2 means usage error. The new command requires
Linux with `model-authoring`; other builds reject it before opening input/output.
The standalone `static_model_uv` example remains a compatibility interface to
the same transform.

Focused acceptance covers UV preservation and boundaries, CLI package creation,
installation and loading after hiding original recipe/model/package directories,
license/metadata errors, corrupt/link inputs and no-overwrite behavior. It does
not establish actual editor-window, GPU, shader or exported-player appearance.


