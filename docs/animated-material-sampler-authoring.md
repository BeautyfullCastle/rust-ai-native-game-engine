# Animated character sampler defaults

`animated_model_sampler` creates a reusable cooked animated model with new texture sampling defaults for one used material. It supplies nearest/linear filtering and independent S/T wrapping through the existing package import, character binding and native export route. No editor control or renderer change is introduced.

Build with the optional authoring features:

```sh
cargo build --locked -p orr_model --features animation,import --example animated_model_sampler
```

Place a self-contained animated source beneath a trusted recipe directory and write exactly these seven fields:

```json
{"format":"orr_animated_material_sampler","version":1,"source":"inputs/hero.animated.json","material":0,"linear_filter":false,"wrap_s":"repeat","wrap_t":"mirror"}
```

Publish to a new file on Linux x86_64:

```sh
target/debug/examples/animated_model_sampler --recipe /absolute/hero.sampler.json --asset-id variants/hero.sampler.json --output /absolute/hero.animated.json
```

`linear_filter=false` selects nearest sampling and `true` selects bilinear sampling. Each wrap is exactly `clamp`, `repeat` or `mirror`. These are the renderer's existing single-level controls, without mipmaps or anisotropy. Texture pixels and UVs are preserved rather than rewritten to imitate sampling. Rendering retains its existing opaque behavior.

The complete source must pass the animated loader, contain clips and use the supported `#node=` primitive identity convention. `material` is a zero-based material-table slot used by a primitive. All primitives sharing that slot receive the defaults, while other materials sharing an image stay unchanged. Only its three sampler fields and the new asset/primitive identity and provenance change. Existing image bytes, RGB/alpha factors, UVs, geometry, nodes, skins, clips and timing are retained. Equal-value and nested variants are valid; use distinct logical IDs for assets that must coexist.

Recipes reject unknown, duplicate, missing, null, positional or incorrectly typed fields. Booleans are not coerced; enum spelling is exact. Recipe size is at most 16 KiB, and recipe plus source at most 64 MiB. Existing decoded model limits and the independent 64 MiB cooked output limit apply. The result is validated, serialized and reloaded. Two ordered provenance entries bind the exact recipe and direct cooked source bytes; earlier dependencies are transitively hashed rather than resolved.

Portable source and identity paths end in `.animated.json` and `.sampler.json`. Filesystem checks reuse existing bounded regular-file, link-free ancestry, stable-input and new-only Linux publication helpers. The output directory must exist and the destination must be absent; lexical `.orr` store output rejects. A sibling stage is published with a no-replace hard link. Use trusted stable local directories: this is not a hostile concurrent filesystem sandbox or power-loss durability guarantee. No overwrite, installation, script or network action occurs. Keep original inputs for reproducible authoring.

Import the `.animated.json` with its explicit license through **Content packages > Cooked animated model**. Preview/add/verify, select and use the asset, save model bindings and reopen. Import alone does not install or assign. Existing clip mappings remain available. Export with required project character/lighting capabilities and a trusted runtime.

Focused tests cover the filter/wrap combinations, strict admission, bit/pose preservation, provenance, safe CLI files and Editor binding/export use. Headless checks inspect deployed sampler data and deterministic motion; they do not prove GPU pixels, actual window/input/audio or play video. Windows pure transformation and Linux filesystem acceptance are reported separately. The locked computer remains locked and historical Windows publication failures are not retried.
