# Create a static model PNG skin

In the model package importer, choose a cooked static `.model.json` source and enable static PNG skin authoring. Inspect its material catalog and select a used material. Choose a non-animated 8-bit RGBA PNG file, a new `.retexture.json` asset identity and a new package destination. Explicit distribution-rights acknowledgement is required before Import. Static PNG skin and static UV modes are exclusive.

The existing producer appends the decoded PNG as a new embedded image and points only the selected material to it. Materials sharing the old image retain their original image. Geometry, UVs, normals, transforms, colors and sampler settings remain unchanged, apart from new model identity and provenance. Source files and installed objects are not overwritten.

The inspected model source hash is checked before reading the PNG. The worker captures source and PNG bytes using existing bounded, link-free readers and decoder limits; provenance records those captured bytes. The PNG has no separate inspection hash, so a path snapshot is not a guarantee against a file changing in place. Revoke distribution consent or edit fields before submitting if the content is not ready.

Invalid source/PNG, unused material, malformed paths, exceeded budgets and occupied output destinations reject without publication. Form/context changes retire stale completion results. No new pre-import static model window is added; use the existing imported-package preview, install, assign through existing model bindings and save.

CPU tests cover decoded pixels, shared-image isolation and saved static-model consumption. Windows runtime, GPU pixels, physical play and video require separate verification.
