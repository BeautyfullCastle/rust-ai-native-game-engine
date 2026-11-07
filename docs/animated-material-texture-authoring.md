# Animated character PNG texture variants

`animated_model_retexture` produces a new self-contained cooked animated model with a PNG assigned to one used material. Character motion and existing material color factors remain intact. This supplies reusable character skins through the existing package import, binding and native export workflow.

```sh
cargo build --locked -p orr_model --features animation,import --example animated_model_retexture
```

Put the cooked source and PNG below a trusted recipe directory. Write exactly these five fields:

```json
{"format":"orr_animated_material_texture","version":1,"source":"inputs/hero.animated.json","texture":"textures/blue.png","material":0}
```

On Linux x86_64, publish a new file:

```sh
target/debug/examples/animated_model_retexture --recipe /absolute/blue.retexture.json --asset-id variants/blue.retexture.json --output /absolute/blue.animated.json
```

The complete source must pass animated-model validation, contain at least one clip, and use the supported `#node=` primitive identity convention. `material` names a material used by a primitive. Every primitive sharing that material receives the new image. The decoded RGBA8 image is appended; original images and materials sharing an old image stay intact. Only the selected image reference, new asset/primitive identity and provenance change. RGB/alpha factors, samplers, UVs, geometry, nodes, skins, clips and timing remain unchanged. Existing color multiplication and opaque rendering semantics still apply.

PNG admission reuses the static model author's bounded decoder: input must decode to exact RGBA8 after expansion and 16-bit stripping; no APNG, at most 2048 per dimension, with complete decoder finishing. RGBA, indexed alpha and 16-bit alpha follow the existing decoder. RGB-only and grayscale reject without an implicit alpha fallback. Decoder policy can discard malformed ancillary metadata; exact raw PNG bytes are still hashed. Recipe plus cooked source plus PNG must fit 64 MiB, and the appended image plus existing images must fit 32 MiB decoded bytes and 32 images. The cooked output independently must fit 64 MiB.

The strict recipe rejects unknown/duplicate/missing fields and invalid types. Portable logical paths and bounded regular input files are required. Recipe/source/texture bytes are checked again before returning. Three ordered provenance entries bind exact recipe, source and PNG bytes; historical dependencies are not resolved. The output is validated, serialized and reloaded. Keep author inputs separately for reproducibility.

Publication reuses the existing Linux authoring boundary: trusted stable link-free parents, existing output directory, absent destination and no-replace hard link. Existing files and lexical `.orr` package-store outputs reject. No overwrite, installation, network or script execution occurs. Hostile concurrent filesystem mutation and power-loss durability are outside the contract. Known Windows publication failures are not retried; the pure transformation API remains separate from filesystem publication.

Import `.animated.json` with its explicit license through **Content packages > Cooked animated model**. Preview/add/verify, select and use the asset, save bindings and reopen. Import alone does not install or assign. Existing Room clip mappings remain usable. Export with the project's required character/lighting capabilities and a trusted runtime.

Focused acceptance checks selected pixels, preserved images/materials/poses, strict invalid input, actual CLI publication, Editor import/use/save/reopen and source-hidden native export. CPU pixels and headless runtime results do not establish GPU appearance or real window/input/audio playback. A locked computer is not unlocked or bypassed; actual playtest video needs separate evidence.
