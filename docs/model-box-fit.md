# Fit an imported model to a selected collider

In a local Yard3D or Room scene, open model bindings, select a body with a box or sphere
collider, choose the package and model, and click **Load model asset**. Click
**Fit rest model to box** or **Fit rest model to sphere**, then **Assign static model** or **Assign animated
model** and **Save model bindings** when the draft is ready.

Fit centers the geometry at the body's origin, resets the local quaternion to
identity, and chooses one uniform scale so the rest geometry fits the selected collider with
1% space. Sphere fitting encloses the entire rest bounding box inside the radius;
it is conservative for rounded meshes. Imported static node transforms are included; animated models use
skinned rest-pose bounds. Clip choice and playback mode remain unchanged.
The collider and physics body do not change. Animations may move outside their
fitted rest bounds during playback. The sphere option supports the default Room
player, key and exit colliders.

Fit changes only the Offset, Scale and Local quaternion fields. It creates no
model undo entry and does not write the sidecar; Assign, model Undo/Redo, and
Save retain their existing behavior. The model must be freshly verified against
the installed project, even after loading a candidate. Changed form identity,
package verification failure, Play/preview/remote mode or an unsupported shape
leaves the draft unchanged.

Boxes and spheres are supported; capsules are rejected. Empty or point geometry, invalid bounds, transforms
outside the existing authoring limits, and origins too distant for reliable
f32 fitting are rejected. Flat geometry can fit on its nonzero axes. These
checks concern presentation bounds; visual appearance still needs native
window/GPU review.

Focused acceptance covers arithmetic and precision limits, the actual egui
Fit/Assign/Save/reopen path, no physics mutation, unsupported/stale inputs,
retained animation clip and host sampling, and Room player settings saved into
a real export. Export acceptance hides the original project, authoring package
and input runtime before invoking the deployed launcher CPU pose check.
