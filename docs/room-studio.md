# Room studio presentation

A ready-to-run CC0 [Key Courier example](../examples/room_studio/README.md) is
provided with its immutable assets, lockfile, bronze key material and closer camera.

`room_escape --project /absolute/project --studio` opts into a presentation-only
look. Omit `--studio` for the unchanged legacy room renderer. The option works
with both a window and `--headless --capture /absolute/new.png`; it changes no
project files, package declarations, camera, actor, collider, input, or checksum.
Existing authored camera and optional point-light settings remain authoritative.

The preset applies the existing renderer facilities:

- Shared directional shadow map for procedural, static and skinned geometry
- Scene-linear RGBA16F, a single ACES/display transform, and restrained bloom
- Warm directional light against cool hemisphere ambient
- Slate checker floor with world-space grout, teal walls, clean brass/stone trim
  and wall-mounted ribs and emissive inset panels
- Amber key and mint exit floor inlays; collected-key inlays disappear immediately

The floor and trim are decorative. They do not add collision obstacles. The
existing optional point light is still unshadowed. This is not ray tracing, GI,
physically based path tracing, or a new shader implementation. See
[shared shadows](imported-scene.md#bounded-shared-directional-shadows) and [HDR/bloom](hdr-bloom.md) for implementation
limits. HDR capability/budget failures are explicit errors rather than silently
falling back. Run without `--studio` on unsupported devices.

The look is runtime-session-only. It is not serialized into project/export
manifests, automatically enabled by an asset package, or applied to the editor.
An exported runtime supporting this version can receive the same `--studio`
option explicitly. Runtime restart retains the chosen look.

## Verification

The CPU test `room_studio::tests::studio_presentation_is_read_only_bounded_and_key_aware`
checks immutable frame bytes, bounded decorative geometry, and key removal.
The ignored GPU test
`room_view::tests::studio_gpu_shadows_hdr_bloom_and_legacy_capture` requires
`ORR_ROOM_STUDIO_PROJECT` and a new `ORR_ROOM_STUDIO_CAPTURE_DIR`. It writes real
1024x768 before/after/no-bloom/no-shadows captures, checks actual shadow draw calls and map
allocation, checks RGBA16F allocation, verifies bloom and directional shadows change rendered pixels,
and verifies that rendering never changes simulation bytes. A missing GPU fails
the test; no generated artwork is used as rendering evidence.
