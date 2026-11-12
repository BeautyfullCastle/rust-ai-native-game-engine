# Cooked animated model UV variants

`animated_model_uv` adjusts a saved animated model's texture coordinates without the original GLB. Use it after PNG texture replacement to fit or tile the new image, then reuse the existing package import, character binding and native export route. Raw glTF `KHR_texture_transform` import already exists and is unchanged.

```sh
cargo build --locked -p orr_model --features animation,import --example animated_model_uv
```

Place the self-contained cooked source beneath a trusted recipe directory. Write exactly these seven fields:

```json
{"format":"orr_animated_material_uv","version":1,"source":"inputs/hero.animated.json","material":0,"offset":[0.25,-0.5],"scale":[2.0,0.5],"quarter_turns":1}
```

On Linux x86_64, publish a new file:

```sh
target/debug/examples/animated_model_uv --recipe /absolute/hero.uv.json --asset-id variants/hero.uv.json --output /absolute/hero.animated.json
```

For each selected `(u,v)`, first scale to `(x,y)`, rotate around origin `(0,0)`, then add offset. Turns 0/1/2/3 produce `(x,y)`, `(-y,x)`, `(-x,-y)` and `(y,-x)` respectively. The example maps `(0.25,0.75)` to `(-0.125,0.0)`. Order matters. There is no clamp to 0..1: existing preserved wrap and filter settings address out-of-range coordinates.

Offset and scale contain exactly two finite values representable as finite f32; converted scales must remain strictly positive. Input numbers are admitted before f32 conversion. `quarter_turns` is an integer 0..3. Arithmetic uses f32 and rejects nonfinite intermediate results; final UV components must lie within inclusive ±65,536. Identity parameters (scale 1, zero turns, zero offsets of either sign) skip arithmetic and retain original signed-zero bits. No negative-scale mirroring, arbitrary angles, seam editing or UV unwrapping is added.

The complete source must pass animated-model validation, contain clips and use the supported `#node=` primitive identity convention. `material` selects a used zero-based material-table slot. Every primitive sharing that material has its UV0 changed; other materials sharing the same image retain their UVs. Image bytes, material factors/samplers, vertex positions/normals/joints/weights, topology, nodes, skins, clips and timing remain unchanged. The resulting asset receives new asset/primitive identity and two ordered provenance hashes binding exact recipe and direct cooked input. Historical dependencies are not resolved.

The recipe rejects unknown, duplicate, missing, null, positional or incorrectly typed fields. Recipe size is at most 16 KiB, recipe plus source at most 64 MiB; existing decoded limits and independent 64 MiB cooked output limit apply. The result is validated, serialized and reloaded. Portable source and identity paths end in `.animated.json` and `.uv.json`.

Filesystem handling reuses existing bounded regular-file, link-free ancestry, stable-input and new-only Linux publication helpers. Existing output directory and absent destination are required; lexical `.orr` package-store output rejects. A sibling stage is published with a no-replace hard link. Use trusted stable local directories: hostile concurrent filesystem replacement and power-loss durability are outside this contract. No overwrite, network, installation or scripts run. Keep inputs for reproducibility.

Import `.animated.json` with its explicit license through **Content packages > Cooked animated model**. Preview/add/verify, select and use, save model bindings and reopen. Import alone does not install or assign. Existing clip mappings remain available. Export with required character/lighting capabilities and a trusted runtime.

Focused acceptance checks numerical UV oracles, unchanged non-UV data/poses, strict admission, actual CLI files, Editor save/reopen and source-hidden headless export. CPU UV and pose checks do not prove rendered GPU appearance or actual window/input/audio/video. Windows pure transformation and Linux filesystem execution are separate. A locked computer is not unlocked; historical Windows publication failures are not retried.
