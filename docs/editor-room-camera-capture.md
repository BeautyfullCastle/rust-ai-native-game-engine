# Capture the current Room viewport camera

With `room-project`, a saved local Room that already declares a camera sidecar
can capture its current viewport framing without manually copying camera fields.
Use right-drag to orbit, middle-drag to pan, and the wheel to zoom, then choose
**Stage viewport camera** in the inspector. Staging changes no camera document,
scene, simulation Frame, history or files. **Cancel staged viewport camera**
(or Escape) discards the candidate while leaving session navigation alone.

**Apply staged viewport camera** explicitly applies the captured framing in
memory through the existing camera document validator. It preserves perspective
FOV, or the effective orthographic scale after session zoom. Invalid or
unrepresentable views are rejected; values are not silently clamped. Saving
remains separate through **Save captured camera**.

**Undo captured camera** and **Redo captured camera** refer only to the most
recent capture-owned action. They are not general camera-history redo. While
that history is active, the usual camera form is hidden to prevent intervening
draft edits; choose **Finish captured camera history** to return to it. Other
camera authoring, scene/epoch/checksum changes, Play, dirty scene/model state or
active jobs invalidate the captured state. Apply rechecks the current context.
Capture requires clean, idle local Edit mode and is unavailable during remote
ERP authoring, transactions, previews, exports or package operations.

The existing camera sidecar schema, renderer, export and playback consumer are
unchanged. Reloaded projects and Play use the explicitly saved authored camera,
not a transient staged candidate. An opt-in `room_camera_capture_ui` test uses
synthetic egui orbit/pan/wheel input and software Vulkan to check Stage/Cancel,
unchanged framing after Apply, capture-specific Undo/Redo, Save/reopen and
Play/Stop. Its receipt reports the adapter and does not claim physical OS input
or hardware-GPU validation.
