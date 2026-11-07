# Place a fixed Room light from the viewport

A `room-lighting` editor can stage a fixed point light's horizontal position by clicking the Room viewport. This replaces coordinate guessing while keeping the existing explicit Apply/Undo/Save workflow.

Open the complete saved Room project and expand **Room lighting**. Enable a valid point light and turn off **Carry light with player**. Set the desired **Position Y** height, then choose **Pick fixed light X/Z** and click the viewport. The ray intersects the world **Y=0 work plane**; its X/Z coordinates become the lighting draft while the authored height, RGB, intensity, range and activation stay unchanged. This is plane placement, not a collision-surface, wall or model picker.

The work area is X/Z within -32..32 world units. Parallel or backward rays, off-viewport input, invalid camera/size/data and points beyond that area leave the draft unchanged. The tool uses the currently displayed camera, viewport offset and physical pixel scale for perspective and orthographic views. It does not reset the camera or change gameplay/colliders.

Inspect the staged coordinates and use **Apply lighting** to preview. Undo/Redo and **Save lighting** retain their usual independent meaning. Reopen restores the saved position, and existing Room runtime/export paths consume the same lighting document. A click alone changes no applied lighting, history, Frame or files.

**Cancel light pick**, Escape or lost focus leave existing lighting drafts intact. Play, replaced/stale sources, changed scene/draft or conflicting modal/authoring operations invalidate pending picking. The tool owns its viewport gesture through completion/cancellation so the same click cannot also select or move an entity or orbit the camera. Other authoring tools retain their established ownership; picking does not bypass them.

This requires a coherent editable local Room, a valid enabled fixed-light draft and current source identity. Carried lights remain on their existing offset workflow. No new light/shadow/renderer feature is added.

Focused CPU acceptance covers finite bounded ray projection, both camera projection modes, stale/cancel/focus/gesture ownership, real panel activation, synthetic viewport input through the production adapter with offset/high-DPI coordinates, draft-only behavior, Apply/Undo/Redo/Save/reopen and resolved CPU light settings. Actual native-window input, GPU pixels, audio-device behavior and video playtests require separate verification. A locked computer is not unlocked or bypassed, and known Windows file-publication failures are preserved without retry.
