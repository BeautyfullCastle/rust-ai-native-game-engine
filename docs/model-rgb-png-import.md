# Opaque RGB PNG textures in model imports

Static and animated glTF imports can use an ordinary 8-bit RGB PNG without an
alpha channel or `tRNS` transparency chunk. This removes the need to re-export
an opaque texture merely to add an alpha channel. The importer copies each
stored RGB triplet and supplies alpha `255`; it does not change colour values,
apply gamma correction, resize the image, or introduce a new rendering mode.

The [glTF image contract](https://github.com/KhronosGroup/glTF/blob/main/specification/2.0/Specification.adoc)
defines PNG images without alpha or `tRNS` as fully opaque. This is a bounded
input extension, not a claim of complete PNG or glTF support.

## Use the existing import workflow

Use the RGB texture in a model satisfying the existing static or animated
import requirements. Existing embedded GLB images and permitted image resource
resolution use the same decoder. The original encoded image/model bytes remain
the source of provenance hashes; an RGB file and an equivalent RGBA file do not
have identical package or source hashes.

In Mesh Navigation, load the admitted static prop or animated package through
the existing inspector. Loading prepares a candidate. Explicitly Add/Update a
prop or Apply the animation, then Save. No automatic scene assignment or new
licensing/provenance exception is introduced. Saved embedded model data can be
reopened without the original package directory under the existing contract.

## Preserved boundaries

- Only original RGB, 8-bit, no-`tRNS` images gain the new opaque conversion.
  Grayscale, grayscale-alpha, opaque indexed PNG and opaque RGB16 remain outside
  this new path.
- Existing inputs that decode to RGBA8 keep their previous behavior, including
  RGBA16 stripped to eight bits and supported RGB/indexed images with `tRNS`.
  Transparency is retained in cooked pixels, never replaced by opaque alpha.
  The existing renderer's alpha policy is unchanged.
- Dimensions remain at most 2048 per axis. The final four-byte-per-pixel image
  is charged against the existing 32 MiB aggregate decoded-image limit before
  decoding. Source limits, APNG refusal, decoder limits and end-of-image checks
  remain unchanged. This does not add a stricter general PNG metadata validator.
- Every declared image is admitted, including an unused image. Selecting one
  valid material cannot hide malformed images elsewhere in the source.
- Animation, skin, geometry, UV, sampler, material and transform contracts are
  unchanged. The optional model `import` feature owns this conversion; animation
  imports reuse it. No runtime format, dependency or renderer change is needed.
- The separate `static_model_retexture` authoring example is unchanged and
  retains its own PNG input restrictions.

## Acceptance scope

Focused tests compare independent literal RGB/RGBA encodings, malformed and
over-budget inputs, legacy alpha/16-bit behavior, cooked reloads and static and
skinned model data. Actual offscreen Editor tests compare opaque RGBA twins with
RGB imports through installed packages, explicit application and Save, then
source-hidden fresh reopen at two animated poses. These are software-GPU
consumer checks, not native-window, hardware-GPU or general transparency proof.
