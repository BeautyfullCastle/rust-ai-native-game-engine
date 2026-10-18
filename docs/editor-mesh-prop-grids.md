# Mesh prop level grids

The optional MeshNavigation editor supports up to 128 cosmetic static props per scene. Copies with the same admitted appearance share one immutable GPU model even after saving and reopening. The existing eight-asset viewport limit is unchanged: props may use at most six distinct appearances, retaining slots for the graph and agent art.

## Build a level

1. Open a local admitted MeshNavigation scene in Edit mode. Import a licensed prop package or load a reusable prop asset, then explicitly **Add prop** or **Update selected prop**.
2. Select the applied prop. Finish or discard material, transform, prefab, and viewport drafts first.
3. Open **Mesh props → Prop grid**. Set **Prop grid columns**, **Prop grid rows**, **Prop grid spacing X**, and **Prop grid spacing Z**.
4. Choose **Stamp selected prop grid**. The selected prop remains cell `[0,0]`; all other cells receive fresh nonzero IDs and unique bounded names. A 4×3 stamp therefore adds eleven copies and leaves twelve total when starting with one prop.
5. The entire stamp is one **Undo prop edit** / **Redo prop edit** entry. It does not write files or alter the simulation Frame. Use **Save props** or **File → Save mesh workspace** explicitly, then reopen and Play.
6. Export with a compatible current mesh player. A scene with more than six props requires the player's structural `.orrdns` / `ORR_MESH_DENSE_PROPS_V1` capability. An older player is refused before a bundle is published; scenes with six or fewer props retain their existing compatibility requirements.

## Admission and identity

- Grid dimensions are integers from 1 through 16, with at least two cells. Both spacings must be finite, positive, and at most 1024. Final count may not exceed 128; overflow, invalid transforms, duplicate identity/name failures, or any budget failure reject the entire stamp without partial edits.
- Each copy preserves Y, scale, rotation, tint, UV/pivot, material-slot overrides, facing, original source model, and license/provenance. Only X/Z placement and the new prop ID/name differ.
- Appearance identity is cached from the validated cooked-model hash and tint/UV/pivot/material-slot settings. Graph revision, instance transforms, facing, and license metadata do not cause redundant GPU assets. Interning shares only the returned render-model Arc; it does not replace another prop's source model or license record.
- Identity is conservative: differently ordered material settings or signed-zero encodings can count as different appearances even if their rendered pixels would match. Such inputs may overcount the bounded asset budget; they are never silently merged as a different appearance.
- The saved prop schema and bundle manifest are unchanged. The existing 16 MiB sidecar limit remains. Prop admission also bounds primitive-instance draws to 3584; the existing composed-renderer draw, shadow, memory, and total-instance checks still apply to the complete frame.
- More than six different appearances are refused even when count is below 128. This is bounded level dressing, not unlimited model streaming. Cosmetic props do not become collision bodies or navigation obstacles, and the point-agent simulation is unchanged.

## Verification scope

CPU coverage includes the 128/129 instance boundary, appearance interning after reopen, distinct-appearance and draw limits, cached identity invalidation, atomic grid/history behavior, and new/legacy player admission. The full editor acceptance extends the reusable-prop workflow with a twelve-prop grid, source-hidden fresh reopen, two static GPU cache groups, Play/mission replay, explicit old-player export refusal, and compatible source-free export/native launch. Actual results and original failed attempts are recorded separately in PR205; neither this document nor an individual passing feature declares the whole engine complete.

See [reusable mesh prop assets](editor-mesh-prop-prefabs.md) for asset capture/load and rights acknowledgement. This feature stays within the existing optional `mesh-navigation` / `mesh-export` boundary; it adds no simulation, public model, or runtime prefab schema.
