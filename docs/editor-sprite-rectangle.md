# Editor sprite rectangle authoring

Sprite Rectangle Authoring turns one PNG into a new sprite-only package by recording explicit pixel rectangles. It keeps the image as supplied and does not infer sprite bounds from its pixels.

1. Open Sprite Rectangle Authoring and choose one PNG. Use **Inspect** to view it, then add between 1 and 256 rectangles in the order they should appear. Each rectangle uses integer pixel coordinates and dimensions.
2. Review the ordered list and acknowledge the license. **Create** writes a new sprite-only package containing the PNG and rectangle metadata. Its animation clips start empty; the tool does not invent clips.
3. Open the package in the existing **Preview / Install** flow. Review the preview, then install it when ready.
4. In the existing Sprite panel, **Load** the installed package, **Assign** a sprite to the selected scene entity, then **Save bindings**. Saving the scene alone does not persist the sprite binding.

The authoring boundary is one PNG and an explicit ordered list of 1 to 256 integer pixel rectangles. It does not trim, resize, recolor, segment, or otherwise process image pixels. Creating the package does not automatically install it, load it into the Sprite panel, assign a sprite to an entity, or save the scene. Those remain separate user actions in the existing flows.

CPU evidence does not establish native GPU rendering, Windows behavior, or actual play, video, or export results. Those require separate platform and runtime evidence.
