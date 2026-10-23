# Place Room actors in the viewport

Open **Room level authoring** and choose **Read Room layout** in a saved, clean
local Room. Choose **Place Player in viewport**, **Place Key in viewport**, or
**Place Exit in viewport**, then click the intended ground position. The picker
uses the camera that successfully rendered that viewport and the Y=0 plane. It
stages only that role's X/Z fields; actor vertical dimensions remain unchanged.

Review all three roles and choose **Apply Room layout**. This is the existing
single authoritative transaction, with normal Undo, Redo and Save. Player-wall overlaps or
otherwise invalid layouts are refused atomically. Reading and clicking do not
change the scene, model bindings, camera, GUIDs or history.

The draft stays pinned to the read scene and its revision. Scene changes,
playback, dirty or incoherent authoring, dialogs and content jobs cancel picking;
stale reads cannot authorize an Apply. Escape and focus loss cancel the gesture.
Actor placement and wall-footprint drawing cannot be armed together, and a final
or canceled viewport click cannot fall through into selection or other tools.

The software-Vulkan UI gate uses production editor controls with synthetic
pointer/accessibility events. It checks all three roles, overlap refusal,
Apply/Undo/Redo/Save/reopen, then live key pickup and exiting with a win. It does
not claim physical OS input or hardware-GPU verification.
