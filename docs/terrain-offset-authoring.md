# Offset a terrain rectangle

In a coherent local Yard3D Edit scene, open or create a local terrain and expand
**Terrain authoring → Offset terrain rectangle**. Enter inclusive vertex X/Z
bounds and a signed decimal **Offset height delta**, then select **Apply offset
rectangle**. A positive delta raises the existing shape; a negative delta lowers
it. Unlike Flatten, height differences within the rectangle remain unchanged.
Each click applies the delta again.

The complete rectangle must fit, with at most 4096 vertices. There is no clipping,
falloff or smoothing at its boundary. Hole flags, identity, origin, spacing and
outside vertices remain unchanged. Checked fixed-point addition rejects the
whole operation if any result overflows. Existing display precision and renderer
capacity checks still apply; an inadmissible result changes nothing.

Each successful nonzero offset is one Terrain Undo/Redo step. Zero is a no-op;
no-op or rejected edits preserve redo. Save terrain is separate. Reopen consumes
the same canonical .orrt bytes in existing terrain queries and rendering. This
presentation-only editing session does not install collision or change the
scene's authoritative simulation/history. Package terrains remain read-only;
Play mode and stale scene contexts refuse editing.
