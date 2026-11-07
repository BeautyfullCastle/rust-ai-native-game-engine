# Animated model material color variants

The optional `animated_model_recolor` authoring tool creates a new cooked animated model with one used material's linear RGB factor replaced. It connects native character appearance editing to the existing cooked-animation package importer without changing animation or runtime shaders.

Build with `animation,import`:

```sh
cargo build --locked -p orr_model --features animation,import --example animated_model_recolor
```

Place a self-contained cooked source at a contained logical path such as `inputs/hero.animated.json`. Write a recipe:

```json
{"format":"orr_animated_material_color","version":1,"source":"inputs/hero.animated.json","linear_rgb":[0.2,0.7,0.4],"material":0}
```

The filesystem command uses the existing Linux x86_64 authoring boundary. Then run:

```sh
target/debug/examples/animated_model_recolor --recipe /absolute/blue.recolor.json --asset-id variants/blue.recolor.json --output /absolute/blue.animated.json
```

RGB values are linear-light factors in0..1, replacing the old factor rather than multiplying it or converting from sRGB. The existing renderer combines this factor with the texture. Alpha, embedded images, sampler settings, geometry, transforms, hierarchy, skin data, clips and animation timing remain unchanged. Choose an existing material used by a primitive. The source must pass complete animated-model validation, contain clips, and use the existing `#node=` primitive identity convention emitted by native animation import/authoring. The result receives a new authored identity and provenance hashes, is cooked and re-admitted before use. Provenance entries are not extra files to resolve or execute.

Import the resulting `.animated.json` and your explicit license file through **Content packages > Cooked animated model** in a `package-manager,room-animated-models` editor. Import does not install or assign. Preview/add/verify the new package, select and use its animated asset, save bindings and reopen. Existing Room character mappings continue selecting the retained clips. Keep consumer character/lighting capabilities enabled where the project declares them; export through the existing trusted-runtime path.

The recipe is strict, bounded data: unknown, duplicate, missing, null or positional fields reject. Channel ranges are checked as f64 before conversion so rounding cannot rescue an invalid value. Source and output path checks, stable input snapshots and new-only publication reuse the existing authoring boundary. Existing input/output data is preserved; use a new filename for a revision. Use trusted local directories; hostile filesystem mutation and power-loss durability are outside the contract. No scripts, network sources or source-history dependency resolution run.

Focused CPU acceptance checks exact data/pose preservation, strict invalid-input behavior, command/file boundaries and authoring-to-installed Room binding/export usage. Shader code is unchanged; CPU color and pose checks do not establish rendered pixel appearance. Windows pure transformation, Linux filesystem/CLI execution and exported headless gameplay are reported separately. Known Windows publication failures are not retried, and a locked computer is not unlocked or bypassed. Native-window/GPU/audio/video verification requires separate evidence.
