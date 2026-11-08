# Create a two-part static model assembly

In the model package importer, choose a cooked static `.model.json` source and enable two-part assembly. Inspect the source and explicitly use that inspected source for assembly. Set each part's XYZ offset, positive XYZ scales and quarter-turn Y rotation, then choose a new `.assemble.json` identity and package destination. Acknowledge distribution rights before Import.

This bounded tool creates two transformed copies of the same model. It does not combine different sources or licenses. Each part applies scale, then right-handed Y rotation, then translation, outside the source primitive transform. A positive 90-degree yaw maps a scaled point `(x,y,z)` to `(z,y,-x)` before translation. Image and material tables are shared once; primitives receive distinct part identities. Original files, UVs, geometry data and sampler settings are preserved.

The form admits only current explicitly chosen source data. Source/context changes or restarting inspection retire old choices and pending results. Invalid numeric input, nonpositive or ill-conditioned placements, invalid identities and resource-limit failures reject without publication. Existing producer validation checks composed geometry and normal matrices.

Install the new immutable package, preview it through the existing package browser, assign it through existing static model bindings and save. The assembly changes visual model data; it does not create colliders or navigation geometry. No new pre-import static model window is added.

CPU tests compare placement math, normal matrices, source-table reuse and saved Room model consumption. Windows runtime, GPU pixels, physical play, native export and video need separate verification.
