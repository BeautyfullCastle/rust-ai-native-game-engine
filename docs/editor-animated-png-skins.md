# Create an animated PNG skin in the Editor

The cooked animated-model import screen can create a new character skin from a local PNG. It produces a new package without changing the source model, original PNG, installed assets or character bindings automatically.

In an Editor built with `package-manager,room-animated-models`:

1. Open a Room project and **Content packages > Cooked animated model**.
2. Select a self-contained `.animated.json` source, license, new destination, package name and version.
3. Enable **Create animated PNG skin**. Select **Variant PNG file**, **Variant material index** and a portable logical **Variant asset ID**, such as `variants/hero.retexture.json`.
4. Review redistribution rights for the model, PNG and license; confirm the rights checkbox. Click **Import cooked animated package**.
5. Preview/add/verify the new package, select and use its animated asset, save model bindings and reopen.

The original-import mode is the default. PNG skin and RGB color creation are mutually exclusive, and both are restricted to cooked animated sources in Animated mode with animation support. Switching source format or model kind clears authoring activation. Inputs are disabled during the worker. The immutable submission includes PNG path, selected material and logical identity, and stale results cannot select a package into a changed project/source/authoring state.

The PNG is decoded and appended as a new embedded RGBA8 image. Only the selected used material's image reference changes. All primitives sharing that material receive the skin; other materials sharing the old image retain it. Existing images, RGB/alpha factors, filter/wrap settings, UVs, geometry, joints/weights, skins, clips and motion remain intact. Existing color multiplication and opaque rendering behavior still apply.

The complete source must pass animated-model validation, contain clips and use supported `#node=` primitive identities. The zero-based material index must be used by a primitive. The new logical identity ends in `.retexture.json` and must differ from the input asset identity. Use unique IDs for variants that coexist.

PNG admission reuses the existing model decoder: inputs must expand/strip to exact RGBA8, at most 2048 pixels per dimension; APNG, plain RGB without alpha and grayscale reject. RGBA16 and indexed alpha follow the decoder's existing policy. Exact PNG bytes are hashed even when ancillary metadata is discarded. Recipe plus source plus PNG must fit 64 MiB, and the resulting model must fit existing image/table/decoded limits and the independent 64 MiB cooked-output budget. Repeated skins retain earlier images; this is not texture-memory optimization.

The worker uses the existing `animated_retexture` producer in memory, with no compiler, subprocess or temporary authoring file. Existing license acknowledgment, bounded regular-file reads, link checks, new-destination staging verification and publication guards remain authoritative. Input stability is checked during each bounded read; the admitted source/license/PNG snapshots are used afterward, without pinning original paths until publication. Use trusted stable local directories. Hostile concurrent filesystem mutation and power-loss durability remain outside the contract.

The reported **Asset SHA-256** and saved binding hash refer to the produced cooked bytes. Embedded provenance binds the generated recipe, original cooked source and PNG snapshots; historical dependencies are not opened. After publication, the model includes its pixels and does not require the source PNG. Keep author inputs and rights records separately for reproducibility. Import does not install or assign automatically; inspect a destination after an uncertain worker result before retrying.

Existing Room clip mappings remain available. Export requires project character/lighting capabilities and a trusted runtime. Focused tests cover actual PNG controls and worker submission, package installation/Use, binding save/reopen, embedded pixels, preserved poses and source-hidden headless native execution. UI-harness/CPU checks do not prove GPU appearance or actual window/input/audio/video. The locked computer remains locked; historical Windows publication failures are not retried.
