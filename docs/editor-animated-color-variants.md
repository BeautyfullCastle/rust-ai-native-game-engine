# Create an animated color variant in the Editor

The cooked animated-model import screen can create a new color variant without running a separate CLI. The new package remains explicit: importing does not install it or change any character binding.

In an Editor built with `package-manager,room-animated-models`:

1. Open a Room project and **Content packages > Cooked animated model**.
2. Choose a self-contained `.animated.json` source, its license, a new package destination, name and version. Review the rights to redistribute it.
3. Enable **Create animated color variant**. Choose **Variant material index**, **Variant linear R/G/B (0..1)** and **Variant asset ID** such as `variants/blue.recolor.json`.
4. Click **Import cooked animated package**. The worker validates and creates a new immutable package.
5. Preview/add/verify it through the existing package workflow, select and use the animated asset, save bindings and reopen.

The option defaults off. Ordinary cooked animated import, static cooked import and GLB import keep their existing behavior. Color controls apply only to cooked animated sources in Animated mode with animation support. Changing source format or model kind clears the color-authoring state. Inputs are disabled while the worker runs; completion is associated with the submitted project/source and authoring state.

RGB values replace the selected used material's linear-light factor, rather than multiplying it or interpreting an sRGB color picker. Every primitive sharing that material receives the factor. Other materials, alpha, embedded images, samplers, geometry, nodes, skins, clips and motion remain unchanged. The existing renderer still combines color with texture and keeps its opaque rendering behavior.

The variant ID is a portable logical `.recolor.json` path, distinct from the input asset identity. Choose unique logical IDs for variants that coexist. The selected index must identify a material used by a primitive. The input must pass the complete animated loader, contain animation clips and use supported `#node=` primitive identities. Finite numeric RGB values must be within 0..1 before f32 conversion. Invalid fields or incompatible modes refuse creation.

The worker uses the existing animated recolor producer in memory after bounded source/license admission. There is no subprocess or temporary authoring input. The original file is preserved. Existing strict license acknowledgment, link/regular-file, input-stability, new destination, staging verification and publication rules remain authoritative. No installed package is mutated or overwritten. Keep trusted stable local directories; hostile concurrent filesystem replacement and power-loss durability remain outside the contract.

The package's asset hash and completion's **Asset SHA-256** refer to the actual produced cooked bytes. Embedded provenance separately binds the generated recipe and exact original input bytes; historical dependencies are not opened. The reader checks source/license stability during each bounded read. Authoring uses that admitted byte snapshot; it does not pin the original path until publication or reread it after transformation. A stale completion never selects an old result into the current project; inspect the destination after an uncertain worker result before retrying.

Existing clip mappings remain usable. Export uses the project's required character/lighting capabilities and trusted runtime. Focused acceptance checks actual color controls and worker submission, package preview/install/Use, saved binding identity/hash, preserved poses and deterministic headless native export. These CPU/UI-harness checks do not establish GPU pixels or actual window/input/audio playback. The locked computer remains locked; actual playtest video requires separate evidence.
