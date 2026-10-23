# Author a cooked static-model color variant

The optional GPU-free `static_model_recolor` example replaces the linear RGB
base-color factor of one used material in an existing version-1 cooked static
model. It adds no runtime format, shader, import feature, or editor tool.

```json
{
  "format": "orr_static_material_color",
  "version": 1,
  "source": "inputs/chair.model.json",
  "material": 0,
  "linear_rgb": [0.125, 0.5, 0.875]
}
```

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_recolor -- \
  --recipe /existing/trusted/chair-blue.recolor.json \
  --asset-id variants/chair-blue.recolor.json \
  --output /existing/trusted/chair-blue.model.json
```

All three unique option/value pairs are required, in any order. `--help` or `-h`
works alone. Without `import`, the executable exits 2 with a feature diagnostic.
There is no force, stdin, network, install, or scene-edit option.

## Linear factor, not a display color

`linear_rgb` replaces the selected factor's three RGB channels. It does not
multiply them by their old values. This follows the linear base-color-factor
convention described in the [Khronos glTF material specification](https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#materials).
The existing renderer still multiplies decoded texture color by the resulting
factor and applies its existing lighting. A black texel therefore stays black;
this is not a texture replacement, hue shift, or recoloring of every material.
Values are linear-light numbers, not sRGB byte values or CSS hex colors. This
example performs no automatic sRGB-to-linear conversion.

`material` is a zero-based material-table index, not a primitive or image index.
It must exist and be used by at least one primitive. Every primitive sharing the
selected material gets the same new factor. Materials sharing its image remain
unchanged unless they are the selected material.

## Strict admission and preservation

Exactly five required recipe fields are accepted. Duplicate/unknown fields,
missing/null values, positional arrays, string/boolean coercions, negative or
fractional indices, unsupported formats/versions, and a recipe over 16 KiB
reject. `linear_rgb` must contain exactly three finite numbers in [0,1]. Bounds
are checked as f64 before conversion to f32, so a negative number rounding to
f32 negative zero and a value above one rounding to f32 one still reject.

The whole cooked source must pass the existing loader before slot selection.
Invalid unrelated geometry, materials, images, transforms, or provenance cannot
be hidden by choosing another material. Existing cooked model size limits apply.

Only the selected material's RGB factor and the variant's identity/provenance
change. Alpha retains its exact f32 bits, including signed zero. All unowned
float bits survive the cooked roundtrip: positions, normals, UVs, transforms,
other material factors and alpha. Indices, primitive order, material assignments,
samplers, images and unused table entries are preserved without deduplication.
Mirrored/nonuniform transforms and shared material/image users are supported.
The current renderer remains opaque; preserving alpha does not add blending.
Same-RGB recipes and variants of variants are supported.

The output asset ID is the logical `.recolor.json` ID, independent of its output
filename. Primitive IDs retain their original `#node=` suffix beneath that new
namespace. Choose distinct IDs for separately bound variants. Exactly two
ordered dependencies bind the bytes used:

1. `$source`: SHA-256 of the exact recipe bytes, including whitespace
2. The recipe's `source` path: SHA-256 of the exact cooked input bytes

Prior provenance is transitively bound by the cooked-source hash. It is not
copied into a conflicting dependency list. The result is validated, serialized,
freshly reloaded and checked for bit-exact preservation before publication.

## Paths and publication

Logical source paths and asset IDs are normalized portable ASCII relative paths,
at most 240 bytes, ending in `.model.json` and `.recolor.json` respectively.
Allowed segment characters are letters, digits, `-`, `_` and `.`; segments cannot
be empty, hidden, dot/traversal segments, Windows reserved device basenames, or
end in a dot. Spaces, backslashes, URI syntax and percent escapes reject.

Inputs resolve relative to the recipe's parent, never the working directory.
Use existing trusted, stable, link-free parents. Symlinked inputs, ancestry and
destinations, nonregular inputs, raw parent traversal and nonlocal Windows
prefixes reject. Outputs require a portable basename and existing parent;
package-store `.orr` destinations are excluded. Existing output files,
directories, symlinks and hardlink aliases are never replaced. Inputs remain
unchanged on both success and failure. Publication uses a staged file and
no-replace hard link. These checks are not a sandbox against hostile concurrent
ancestor replacement. Exit 0 means help or publication; read any cleanup warning.
Exit 2 means error without publication.

## Packaging for the existing editor

The CLI writes `.model.json`. The existing editor package browser recognizes
`.orrmodel`; copy the exact cooked bytes to that suffix and list the `.orrmodel`
file in the public package manifest before install/preview. No conversion or
recooking is needed. Retain the recipe logical ID inside those bytes.

## Acceptance

The additive `Static model recolor` workflow has required independent CPU Linux,
CPU Windows, software Vulkan GPU and editor-preview lanes. Each builds and pins
its own CLI and test producers in a fresh target, checks source/producer hashes,
requires the exact test inventory, and retains logs, receipts and captures even
on failure. Its aggregate fails when any lane fails or is skipped. Existing
static-model/retexture gates and aggregate checks remain unchanged.

CPU fixtures cover strict parsing, complete source admission, byte/slot limits,
bitwise preservation, nested identities and filesystem refusal cases. The actual
CLI also feeds a package installed through the public manifest/project path;
verification, source hiding, distinct persistent bindings and A/B/A reloads are
required. The GPU fixture checks independent analytic pixels, shared textured
materials, mirrored transforms, multiple variants and deliberate wrong-result
controls. Editor acceptance exercises installed variant preview pixels, orbit,
zoom/reset, read-only behavior, missing/corrupt cache refusal and binding save/
reopen. These lanes do not claim production export acceptance.

Run a hosted-style gate from a clean checkout with Python bytecode disabled and
an absolute new evidence directory outside that checkout:

```sh
PYTHONDONTWRITEBYTECODE=1 ORR_STATIC_RECOLOR_EVIDENCE=/new/evidence/recolor-cpu \
  python -B tools/static_model_recolor_gate.py cpu
```

Use `gpu` or `ui` on Linux with Mesa Lavapipe installed for the other routes.
