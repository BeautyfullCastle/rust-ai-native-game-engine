# Static-prop scalar shading in the Editor

Build `orr_editor` with the optional, default-off `mesh-prop-shading` feature.
It enables existing scalar metallic/roughness rendering for explicitly selected
static-prop material slots. It does not change agent materials, cooked models,
textures, shaders, lights or deterministic simulation.

In **Mesh props → Scalar material shading**, choose a used material slot and
check **Use scalar shading for prop slot**. Enter finite metallic and roughness
values in `0..=1`. Fields are drafts: **Update selected prop** or **Add prop**
applies one prop-history edit; **Undo prop edit** / **Redo prop edit** restore it.
**Save props** persists the applied document. Drafts, Play, stale scene contexts
and pending edits retain existing editing/save restrictions. Zero roughness is
stored exactly; the existing renderer uses its documented evaluation floor.

The optional `shading` record lives on each prop in the existing scene-local
`.mesh-props.json`, not the shared agent art record. Its closed version-1 object
contains a nonempty bounded list of `{material, metallic, roughness}` overrides.
Only used existing material indices are accepted. Omitted slots remain legacy
diffuse, and omitting the entire record preserves legacy serialized bytes and
appearance. Present null/empty, unknown fields/versions, duplicate slots and
invalid numeric values fail admission. Signed zero is canonicalized; slot order
has no meaning. Builds without this feature reject authored shading rather than
silently displaying diffuse materials.

Props sharing a model but using different scalar tables have separate validated
appearance/cache identities. The existing six-appearance prop budget includes
these differences; identical appearances can still share uploaded resources.
Duplicate and grid stamping preserve shading. Source geometry, model and license
digests remain unchanged. Loading an unshaded prefab is only a candidate until
explicit Apply; existing applied props are not changed by loading.

**This first slice is Editor authoring and Play preview only.** Shaded props are
explicitly rejected by mesh export and bundle-open admission, even in a build
with the feature enabled. Local [prop prefabs](editor-mesh-prop-prefabs.md)
preserve scalar settings when this feature is enabled; feature-off readers reject
those records. Existing unshaded export/prefabs remain supported. Standalone player/export support needs a later capability and consumer
validation slice; no new runtime capability is advertised here.

Focused acceptance covers strict data/feature boundaries, real prop controls,
Apply/Undo/Redo/Save/source-hidden reopen, simultaneous different appearances,
actual software-GPU viewport results, cache rejection atomicity, and export
refusal. Software offscreen evidence is not native-window or physical-GPU proof,
and scalar controls are not a claim of complete PBR or new glTF material support.
