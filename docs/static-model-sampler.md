# Reusable cooked static-model sampler defaults

The optional GPU-free `static_model_sampler` authoring example changes one used
material's cooked sampler defaults. Existing Mesh per-instance sampler editing
already works. This feature supplies reusable asset defaults for the existing
Room package Preview → Use model → Save model bindings → fresh reopen workflow.
It does not add an editor control, model schema, runtime, renderer or core feature.

```json
{
  "format": "orr_static_material_sampler",
  "version": 1,
  "source": "inputs/chair.model.json",
  "material": 0,
  "linear_filter": false,
  "wrap_s": "repeat",
  "wrap_t": "mirror"
}
```

```sh
cargo run --locked --release -p orr_model --features import \
  --example static_model_sampler -- \
  --recipe /existing/trusted/chair.sampler.json \
  --asset-id variants/chair.sampler.json \
  --output /existing/trusted/chair.model.json
```

The three unique option/value pairs are required in any order. `--help` or `-h`
works alone. The existing `import` feature enables authoring; without it the
executable exits 2 with a feature diagnostic. No new dependency or feature is
introduced. No overwrite, force, network, install or scene-edit option exists.

## Exact recipe and bounded admission

Exactly the seven keys shown above are required. Duplicate/unknown/missing keys,
nulls, positional arrays, coerced boolean/string values, unsupported versions,
negative/fractional indices and wrap values outside the exact lowercase strings
`clamp`, `repeat`, `mirror` reject. `linear_filter` is a JSON boolean. `material`
is a zero-based cooked material-table index, and must be used by a primitive.
Every primitive sharing it receives the defaults; other materials sharing its
image remain unchanged. The recipe is at most 16 KiB and recipe plus source bytes
are at most 64 MiB. Existing cooked model table/resource bounds also apply.
The complete cooked source passes the unchanged loader before selecting a slot.

`linear_filter=false` selects nearest sampling; `true` selects bilinear sampling.
S and T wrap independently. These are the renderer's existing single-level
sampling controls; this feature does not provide mipmaps or anisotropy, and does
not claim to reproduce every glTF minification-filter mode. Texture bytes and UVs
are not changed to simulate wrap or filtering.

Only the selected material's three sampler fields and the asset's identity and
provenance change. Geometry, UVs, transforms, all material factors and alpha,
other samplers, image bytes, unused table entries and primitive order remain
bit-exact, including signed-zero float bits. Alpha remains stored, while existing
rendering remains opaque. Retexture → UV → sampler composition is covered by a
validated roundtrip test. Equal-sampler variants and nested variants are allowed.

The output identity is the logical `.sampler.json` ID. Each primitive keeps its
original `#node=` suffix below that namespace. Use distinct logical IDs for assets
that must coexist; changing only a physical output filename is insufficient.
Exactly two ordered dependencies bind the input bytes:

1. `$source`: SHA-256 of the exact recipe bytes, including whitespace
2. The recipe source path: SHA-256 of the exact cooked source bytes

Earlier provenance remains transitively bound through that cooked source hash.
The output is validated, serialized and reloaded before no-replace publication.

## Paths, output and editor use

Source and logical asset ID are portable ASCII relative paths, at most 240 bytes,
with `.model.json` and `.sampler.json` suffixes. Segments permit letters, digits,
`-`, `_`, `.`; empty/hidden/dot/traversal segments, trailing dots, Windows device
names, spaces, backslashes, URI syntax and percent escapes reject. Resolution is
anchored at the recipe parent. Use existing trusted, stable, link-free parents;
nonregular inputs, symlink ancestry, raw parent traversal and nonlocal Windows
prefixes are refused. This is not a sandbox against hostile concurrent ancestor
replacement. Existing outputs and hardlink aliases are not replaced; staging is
published with a no-replace hard link. `.orr` package-store destinations reject.
Exit 0 means help or publication; exit 2 means refusal without publication.

The CLI emits cooked `.model.json`. Copy those exact bytes to a package's
`.orrmodel` asset entries for the existing editor browser, declare them in its
manifest and install using the existing package workflow. Preview works without
selection and does not alter project files, frame/history, bindings or the main
editor camera. Use model requires the intended selected actor; Save model bindings
persists the asset identity, source hash, package digest and local transform.
Fresh reopen loads installed cooked bytes after authoring/package sources are
hidden. Corrupt or removed installed bytes are refused, including after caching.

## Mandatory acceptance and evidence

`.github/workflows/static-model-sampler.yml` requires independent CPU Linux,
CPU Windows, software Vulkan GPU and actual EditorApp UI lanes. Its aggregate
fails unless every matrix lane succeeds. All lanes retain logs, receipts and
partial failure evidence with `always()`, including hidden package files.

`tools/static_model_sampler_gate.py cpu|gpu|ui` requires a clean committed source,
`PYTHONDONTWRITEBYTECODE=1`, unoptimized Python and a fresh absolute
`ORR_STATIC_SAMPLER_EVIDENCE` directory outside the checkout. It freezes the complete
source inventory, records exact Git head/tree, compiles into a private fresh
target, pins executable hashes, and checks unchanged source/head/tree afterward.
`GITHUB_SHA`, when present, must match the tested checkout. Source changes or
missing producers prevent success. Historical logs do not substitute for execution.

The reviewed census is 22 distinct CPU test declarations across platforms:
Linux executes 20 regular tests plus one explicitly invoked pinned-CLI test;
macOS has 19 plus one; Windows has 18 plus one. Unix-only path tests and Linux-only
non-UTF-8 filesystem coverage are not claimed on Windows. The separate bindings
consumer executes one mandatory ignored test explicitly. The 18 sampler
combinations are assertions inside one test, not 18 additional harness tests.
Full model/bindings regression suites and affected strict Clippy checks remain
mandatory. The model test inventory and ignored inventory are separately checked;
zero-test runs, skipped GPU tests and failed-as-success logs cannot certify a lane.

The GPU lane executes one mandatory test: three independent S/T wrap-pair cases,
45 real frames and 108 artifacts. Independent scalar sampling and geometry checks
cover nearest/bilinear, below/inside/above-range UVs, reflected transforms, shared
material users, an unaffected shared-image material, ordered and reversed
multi-asset batches, opaque output, and wrong-filter/axes/wrap/material/transform
negative controls. The checker recomputes pixel differences, exact coverage,
provenance and artifact hashes; control thresholds are fixed before execution.

The UI lane executes one mandatory software-GPU test and retains 19 PNGs plus
CLI logs and source-hidden project/package evidence. The checker decodes PNG
pixels with bounded CRC/decompression validation, recomputes wrap/filter/orbit/
zoom differences, requires exact reset and fresh-reopen viewport pixels, and
checks cooked preservation, sidecar identity and installed-package hashes.
Twelve independent Python selftests cover census, fail-closed execution,
source/inventory mutation, JSON/PNG corruption, sampler arithmetic, cooked
negative controls, workflow routing/retention and optimized-Python refusal.

Hosted gates compile release (`opt-level=3`) producers with release debug info
explicitly disabled to fit the existing 128 MiB pin limit. Local debug0 test
receipts are useful development evidence, but do not claim those fresh hosted
release producers, Windows execution or all required CI lanes passed. Actual
observed evidence and exact source hashes must accompany publication/review;
required CI still governs merge readiness.
