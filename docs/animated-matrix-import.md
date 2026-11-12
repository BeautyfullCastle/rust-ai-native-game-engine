# Static matrix nodes in animated glTF imports

The optional `orr_model` `animation,import` path accepts a bounded subset of
static matrix-transform nodes in animated glTF/GLB assets. This connects to the
existing Mesh **Import animation source** workflow: provide the self-contained
GLB, license and new package destination, inspect the staged candidate, then
explicitly Apply and Save. Importing never automatically changes the scene.

[Khronos glTF §3.5.3](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#transformations)
permits a node matrix that decomposes into translation, rotation and scale, but
forbids a matrix on any animation-channel target. Every clip and node is checked,
including inactive nodes and clips that are not selected for preview.

## Supported transform boundary

A matrix must pass the existing finite, affine, coefficient-range and
nonsingularity checks. It must preserve orientation and reconstruct as positive
scales and a proper rotation in the existing `Trs` representation. Translation
is copied exactly. Reflections, singular/projective transforms, matrix-plus-TRS
and animation-target matrices remain unsupported. Explicit TRS inputs retain
their existing behavior, including rejection of negative or zero scales.

The signs originally chosen by an author cannot be recovered uniquely from a
matrix: two negative axes may equal a proper 180-degree rotation with positive
scales. Such an equivalent matrix is not a reflection and may be admitted.

Decomposition uses f64 intermediate arithmetic. Each extracted scale must fit
the existing f32 interval `1e-6..=1e6`. Normalized column dot products must be at
most `8 * f32::EPSILON` in absolute value. After conversion to f32, the actual
runtime `Trs::matrix` result must reproduce each linear component within
`32 * f32::EPSILON * source_column_length`. Translation and affine-row checks
are exact. Quaternion sign and signed zero are canonicalized after conversion.
Both numerical checks must pass; small roundoff within these stated bounds is
allowed, while excess shear is rejected rather than repaired. Large translation
cannot hide linear error. No clamping or Gram–Schmidt correction is performed.

Normalization happens once at import. Cooked reload uses the unchanged animated
model format; no new runtime transform type, interpolation mode, renderer,
package capability or dependency is introduced. Existing source provenance and
license requirements remain in force. This is broader source admission, not a
change to simulation positions or the positive-scale animation runtime.

## Acceptance

Focused tests compare independently authored matrix/TRS twins, near-180-degree
and mixed-axis rotations, hierarchy/skin deformation at multiple times, numeric
boundaries, whole-source invalid cases, cook/reload and legacy TRS behavior.
The Editor consumer uses actual raw-import widgets and animation-only packages,
explicit Apply/Undo/Redo/Save, then hides original sources and reopens the saved
scene. Two distinct pose images are compared with the explicit-TRS twin.
Software offscreen evidence does not certify native windows, physical input,
hardware GPUs or other platforms. General mirrored/sheared transform support,
Arena 3D presentation and animated matrix channels remain outside this slice.
