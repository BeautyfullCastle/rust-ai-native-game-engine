# Non-indexed glTF triangle import

The optional `orr_model` `import` feature accepts glTF/GLB TRIANGLES with an
omitted `indices` property. With `animation,import`, the same admission applies
to skinned meshes. This extends the previous indexed-only subset; it does not
add triangle strips/fans, missing normals, morphs, or new animation interpolation.

The [glTF contract](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes)
uses vertex-attribute order when indices are absent. Import generates the bounded
sequence `0..POSITION.count` in the existing cooked index vector. The vertex count
must be nonzero and divisible by three. No vertex welding, reordering, normal
calculation, skin reweighting, or schema conversion occurs. Explicit `null`,
invalid types, duplicate keys, and invalid indexed accessors still fail closed.

Use the existing static or animated import/package tools and Editor model loading
workflow; no extra CLI or dependency is required. Existing material, texture,
UV-transform, rig and attribute restrictions still apply. Import validates all
source primitive descriptors, including unused meshes; animated source-value and
per-instance skin checks remain in place. Existing per-primitive, source (where
already applicable), and expanded geometry budgets count generated indices before
allocation. Static import does not acquire an unrelated new source-aggregate cap.

## Acceptance

Focused fixtures expand every triangle corner's position, normal, UV and skin
attributes independently, then compare explicit sequential-index and omitted-index
twins through import/cook/reload and pose/deformation sampling. Source digests may
differ because the input files differ; geometry and existing runtime meaning must
not. Strict malformed/unused/instancing cases and existing indexed regressions
protect the admission boundary.

The explicit Editor consumer tests cover installed static and animated models,
actual viewport pixels, saved bindings and source-hidden reopening. GPU receipts
must identify the observed adapter/backend; software offscreen evidence is not
physical GPU, native-window or cross-platform certification. Test results are
reported separately from this contract; this document does not assert unrun tests.

The source and cooked formats, renderer, simulation, package capabilities and
Cargo feature graph are unchanged. This support is independent of the separately
held Editor scalar-prop shading candidate.
