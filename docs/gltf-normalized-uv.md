# Normalized integer UV0 import

The optional `orr_model/import` static importer and `animation,import` animated
importer accept glTF `TEXCOORD_0` accessors with these encodings:

- `FLOAT` (5126), `VEC2`, normalized omitted or false: existing behavior.
- `UNSIGNED_BYTE` (5121), `VEC2`, `normalized: true`: decode each value as `c/255`.
- `UNSIGNED_SHORT` (5123), `VEC2`, `normalized: true`: decode little-endian values
  as `c/65535`.

This adds a standard asset encoding, not UV animation or a new texture format.
Use the existing model import/package/Editor assignment paths. Installed raw GLB
assets and cooked models still use the existing consumers; no shader, renderer,
cooked schema, dependency, or simulation change is required.

## Bounds and compatibility

Integer UV accessors require explicit `normalized: true`. Missing, false or null
normalization, signed integers, unsigned int, incorrect vector type/count,
sparse accessors and additional UV sets remain unsupported. Float UV with
`normalized: true` is still rejected. This does not enable integer positions,
normals, additional skin influences or CUBICSPLINE.

The existing byte-range/stride/count budgets are reused. Each new integer UV
vertex element starts at a four-byte-aligned offset inside its buffer view;
its effective stride is a multiple of four. Thus byte VEC2 needs padding and
an explicit stride of at least four, while short VEC2 may be tightly packed.
The buffer view's absolute offset follows component-size alignment, not an
extra four-byte rule. Last-element trailing padding need not be present when
no following element requires it. Interleaved attributes retain their offsets.

Normalization occurs before the existing `KHR_texture_transform` base-color
UV bake, exactly once. Extension declaration/strict field rules and effective
UV0-only admission are unchanged. Cooking and reloading do not normalize or
transform UV coordinates a second time. Existing float UV conversion is
unchanged; quantization cannot reconstruct precision discarded by the author.

Both importers validate new normalized UVs in source primitives even if a mesh
is not selected for rendering. The new static normalized-UV source scan is capped at 250,000 vertex
occurrences across all source primitives, including repeated references, to
bound validation work. Static legacy float-only source validation is not
otherwise broadened. The animated importer's existing full-source checks,
source/expanded budgets and skin/animation validation remain in force. Source
asset digests and IDs may differ between a quantized file and its float twin;
that is not a change to geometry or animation semantics.

## Evidence boundaries

Focused CPU tests compare integer endpoints/interior values against independent
float twins, including interleaved layout, malformed/unused data and cook/reload.
Actual software offscreen Editor tests install packages, hide original inputs,
compare static and skinned viewport results with float twins, and reopen saved
bindings. Reported adapter/backend and explicit test results accompany each
publication. These are not physical-GPU, native-window or whole-glTF support
claims.

The encoding and alignment rules follow the official [glTF 2.0 specification,
accessors and meshes](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#meshes).
