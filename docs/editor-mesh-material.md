# Mesh material authoring

The optional Mesh Navigation material helper applies a finite linear RGBA multiplier to every material in an immutable cooked model. An identity multiplier preserves the original cooked bytes. Other values pass through the existing orr_model::StaticModel validator after multiplication.

Tinting changes only material base-color factors. It preserves geometry, transforms, material/image references, textures, and dependency provenance. The operation does not modify the source package or scene, change the navigation graph, or claim a physics or licensing effect.

Derived model allocation is capped at 8 MiB of decoded image, vertex, and index data before cloning. The helper is private editor infrastructure; it adds no scene schema, package schema, or runtime API.

Use **Art tint R/G/B/A** in **Mesh agent art**, or **Prop tint R/G/B/A** for the selected prop, then explicitly Apply or Update and Save. Private Undo/Redo, scene reopening, viewport movement, and exported Play retain the saved multipliers. The original embedded model and its provenance digests stay unchanged; the derived render model is cached. The current opaque renderer ignores alpha, so this does not add transparency or a general material/shader graph, PBR editor, or texture painting.

Tint values use the optional private `tint` sidecar field; identity is omitted for legacy compatibility. Any explicitly serialized tint, even identity, requires the dedicated `.orrmat` / `ORR_MESH_MATERIAL_V1` player capability. Export rejects older players rather than silently dropping the authored appearance. No extra bundle file or source package is needed.
