# Terrace a terrain rectangle

Open a scene-owned local terrain in Edit mode and expand **Terrace terrain rectangle**. Choose inclusive minimum and maximum vertex indices, a base height and a positive step height. The preview reports how many vertices will change. **Apply terrace rectangle** commits one terrain Undo step; Save is separate.

Each selected height is rounded down to `base + n × step`, where n is an integer. For example, base 0 and step 1 turn heights 1.75 and -0.25 into 1 and -1. Exact fixed-point integer arithmetic preserves already aligned heights. Holes and vertices outside the rectangle remain unchanged. Existing triangles connect the terrace vertices; this does not add vertical walls or guarantee traversable steps.

Invalid text, nonpositive steps, reversed or out-of-bounds rectangles, oversized footprints and unrepresentable output reject the entire operation. The existing limit counts every selected vertex, including unchanged vertices. Read-only package terrain, stale scene attachments and Play mode cannot be edited. Repeating an already applied terrace is a no-op and preserves Undo/Redo state.

Undo/Redo uses the existing terrain history. Save and reopen retain the authored terrain. Rebuild navigation explicitly after saving changed terrain; an existing admitted graph is not silently replaced.

Focused CPU tests exercise arithmetic, transaction boundaries, UI controls, saved terrain and navigation consumption. Windows runtime, GPU rendering, physical play and video remain unverified.
