# Base-color texture transforms on animated glTF

With existing optional `orr_model` features `animation,import`, animated glTF and
GLB now accept declared `KHR_texture_transform` on base-color textures. This is
new support beyond the earlier explicitly static-only importer, not animated UV
keyframes or `KHR_animation_pointer` support.

The existing static helper applies scale, origin-centered counter-clockwise
rotation in radians, then offset to UV0 once during import. Extension `texCoord`
overrides the base selector; only effective set zero is supported. See the
[official Khronos extension](https://github.com/KhronosGroup/glTF/tree/main/extensions/2.0/Khronos/KHR_texture_transform).

Strict declarations, object shapes, duplicates, finite source/results and the
existing UV bound remain checked. All meshes are checked, including unused or
nonselected geometry; zero scale cannot hide invalid source UV. Shared accessors
are evaluated independently for each primitive's material. Identity preserves
coordinate bits. Cooked reload does not apply the transform again.

Rig, weights, nodes, animation channels, positions/normals and sampled deformation
remain unchanged. No new dependency, shader, cooked schema or simulation change
is introduced. Existing package/editor animated-content paths consume the baked
UVs through their normal immutable model admission. This does not add UV1,
normal-map transforms, general glTF extensions, PBR authoring or rights checking.

Focused acceptance uses independent UV arithmetic, original non-UV data and pose
comparisons, strict negatives, cooking/reload and actual textured skinned consumer
checks. Software offscreen rendering is not native-window or physical-GPU proof.
