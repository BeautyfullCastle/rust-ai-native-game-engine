# Bounded HDR and bloom (#101)

This is an opt-in view-layer increment of [#101](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/101), not completion of that issue. Probe sampling, reflection/IBL, probe authoring/baking and their missing-asset workflows remain separate work. No simulation state or golden checksum changes are intended.

## Availability and compatibility

The actual Yard3D editor main viewport exposes HDR in builds with `models` or `animated-models`. The default build keeps its optional model dependency boundary and legacy procedural rendering; its UI explains that HDR requires the models feature. Existing constructors and default settings remain HDR-off. The native texture registered with egui remains `Rgba8Unorm`; egui controls, text and other UI overlays are composed after scene postprocessing.

HDR requires actual `Rgba16Float` single-sample rendering, sampling/filtering and readback support within both adapter/downlevel capabilities and the enabled device's limits. No new optional device features, alternate view formats or persistent security/access settings are enabled. An unsupported adapter is an explicit error, never an RGBA8 texture relabeled HDR. The editor preserves the last working mode/frame when admission fails.

## Color contract

Procedural, static and skinned geometry and depth-tested scene lines share one scene-linear radiance contract in HDR mode. They bypass the legacy inline exposure, tone map and display encoding. Nonnegative HDR radiance saturates at 65504 before storage; material-domain clamps retain their existing meaning. Scene lines participate in bloom on the same threshold basis as geometry; egui overlays do not.

A single-sample full-resolution `Rgba16Float` scene target feeds half-resolution bright extraction, a bounded separable blur using two ping-pong `Rgba16Float` textures, then one final display pass. Half extents use ceiling division, including odd and one-pixel dimensions. The default bloom settings are threshold 1.0, strength 0.15, radius 4 and two horizontal/vertical iterations. Admission bounds threshold to 0..=65504, strength to 0..=8, half-resolution radius to 1..=8 and iterations to 1..=4. Nonfinite values are rejected, even while disabled. HDR with bloom disabled or zero strength still uses the final display pass. HDR disabled retains no HDR targets or HDR passes. Default bloom adds six fullscreen passes (one extraction, four blur passes and one final pass); the admitted maximum is ten. Bloom-off HDR adds only the final pass.

`Lighting.exposure` and `Lighting.tonemap` remain the only exposure/tone-map controls. The final pass applies exposure, ACES or a display-range clamp, then output conversion. sRGB render targets receive linear mapped values for hardware encoding; UNORM render targets receive one shader sRGB encoding. HDR background clear is scene-linear (RGB 0..=65504 and alpha 0..=1) and follows the same final transform. Legacy clear behavior is unchanged.

## Resource and admission contract

The postprocessor owns the full-resolution scene and exactly two half-resolution color textures. Color payload admission uses checked arithmetic, a 128 MiB ceiling, actual device texture-axis limits, and the peak retained-old plus prospective-new allocation during resizing. A resize can be rejected even when the replacement alone would fit, because the previous and next target sets overlap during replacement. The budget concerns these HDR color payloads, not total process/GPU residency, depth/shadow maps, model assets or driver overhead.

Same-size accepted frames reuse resources and retain generation. An accepted HDR resize or off transition drains prior GPU work before replacing or releasing targets; these transitions can stall, while steady-state frames do not wait. Resizing advances generation only after complete admission. Settings, target metadata, format support, resource budget and the entire mixed frame are validated before retained target/model-cache mutation, uniform upload or submission. An invalid late batch or bounds calculation cannot partially replace a working frame. Format-specific procedural/static/skinned pipelines are rebuilt coherently on accepted HDR mode changes.

No mip chains, temporal history, auto-exposure, multisampled HDR resolve, HDR-monitor signaling, PQ or HLG are provided. This is scene-referred floating-point rendering into an ordinary SDR display output.

## Verification boundary

Mandatory software-GPU readback checks are separate from native-window or physical-device validation. Exact adapter metadata, command outcomes, independent review and final source identity belong to the accompanying handoff evidence. Native window, physical GPU and Windows-local validation remain unverified unless explicitly recorded; no Windows access is required for this bounded implementation.

The existing `gpu3d` suite's 20 `VIEW_FORMATS` failures and four ignored measurements on the available software backend remain visible as baseline failures. HDR acceptance uses ordinary same-format texture views and must pass independently; a legacy failure or ignored case is never counted as new HDR success.

### Focused local checks

The RHI `hdr_texture` target requires `gpu-tests` and runs explicitly with all renderer and editor HDR suites in the required Linux `sample-build` job after Mesa provisioning. Native determinism jobs retain the RHI library and unrelated default targets without requiring a software adapter on macOS. The new HDR GPU tests intentionally fail if an adapter is unavailable; they have no ignored or silent-skip success path. Existing suites may retain their independent `ORR_REQUIRE_GPU=1` admission convention. Use the default supported backend unless separately verifying a particular API; forcing an unavailable Vulkan adapter is not evidence of a renderer defect.

```sh
cargo test --release --locked -p orr_rhi --features gpu-tests --lib --test hdr_texture
ORR_REQUIRE_GPU=1 cargo test --release --locked -p orr_render --features imported-scene,animation --lib --test hdr --test imported_scene --test models --test skinned_model
ORR_REQUIRE_GPU=1 cargo test --release --locked -p orr_editor --features animated-models --test yard3d_hdr
ORR_REQUIRE_GPU=1 cargo test --release --locked -p orr_editor --features models --test yard3d_hdr_static
```

`ORR_HDR_CAPTURE_DIR` on the renderer HDR suite writes exact little-endian binary16 scene bytes, separate display PPM files and JSON metadata. `ORR_YARD_CAPTURE_DIR` on editor HDR tests similarly saves native-viewport display captures and raw HDR evidence. A display image is not a visualization of untransformed HDR bytes; use the adjacent format/size metadata when decoding scene captures.
