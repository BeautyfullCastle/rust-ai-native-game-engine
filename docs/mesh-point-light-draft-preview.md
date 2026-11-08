# Preview a Mesh point-light draft

In Mesh game lighting, enable **Preview point light draft** to see the current validated point-light position, color, intensity and range before applying it. Preview is off by default and starts off again when reopening the project. Turning it off restores the applied lighting. A validated disabled point-light draft previews no point light.

The toggle changes only viewport presentation. It does not apply the draft, mark a clean form dirty, change the simulation Frame or history, or write files. Other lighting and HDR settings continue using their applied values. Use **Apply game lighting**, lighting Undo/Redo and **Save game lighting** as before; these actions retire the preview.

Valid field edits and viewport X/Z picking update an enabled preview. Invalid values, a stale scene or host, leaving editable context, focus loss or conflicting authoring/export/capture work turn it off. Correcting the context does not re-enable it automatically. Text editing alone does not cancel it.

Automated acceptance checks the selected renderer settings, real controls, draft-only behavior, lifecycle invalidation, Apply/history/Save/reopening and the existing viewport-picker input fencing. These CPU and filesystem checks do not demonstrate GPU pixels, a Windows run, physical play or video.