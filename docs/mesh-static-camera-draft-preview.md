# Preview a static Mesh camera draft

In Mesh game camera, enable **Preview static camera draft** to inspect the validated draft before Apply. Preview is off by default and supports perspective and orthographic cameras. It is unavailable while **Follow mesh agent** is enabled.

Preview uses the draft camera for viewport presentation only. It does not change the underlying orbit, simulation Frame, history, dirty state or files. Valid camera-field edits update an enabled preview. Turn it off to return to the existing viewport camera. Apply, Undo/Redo and Save retain their explicit meanings and retire preview; reopening starts with preview off.

While preview is active, viewport framing, orbit, pan, zoom, selection, picking and capture cannot operate. Camera capture, reset and focus actions are also unavailable. Finish preview before manipulating the viewport. Input already held while preview owned it remains fenced through release, and residual wheel scrolling cannot become a delayed zoom.

Invalid camera values, Follow, stale scene/host or unavailable editing context retire preview. Correcting the context does not re-enable it automatically. Text editing alone does not cancel a valid preview.

Focused CPU checks compare actual selected cameras with existing validated camera calculations, exercise real controls and Apply/history/Save/reopening, and check conflicting-input admission and state preservation. They do not demonstrate GPU pixels, Windows execution, physical play or video.