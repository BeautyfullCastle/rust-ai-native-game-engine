# Create an animated material UV variant

In the model package importer, choose a cooked animated source and enable UV variant authoring. Inspect the material catalog and select a used material. Set U/V offsets, positive U/V scales and a quarter-turn rotation, then choose a new `.uv.json` asset identity. UV mode is exclusive with other authoring modes.

The operation scales UV0 coordinates, rotates them counterclockwise around UV origin in 90-degree steps, then adds the offset. It changes every primitive using the selected material. Other UVs, vertex positions and normals, rig, clips, materials and image pixels remain unchanged, apart from the new model identity and provenance.

Preview prepares the result without publishing. Import requires distribution-rights acknowledgement, a current inspected source hash and a new package destination. Settings or source changes retire the preview. Nonfinite or out-of-range values, nonpositive scales, scale underflow or transformed UV coordinates outside +/-65536 reject before publication.

Install the new package, assign the animated model through existing bindings and save. UVs beyond 0..1 use the model's existing texture wrap policy; this tool does not change samplers or repack an atlas. CPU tests verify coordinate results and saved-model consumption. Windows runtime, GPU pixels, physical play and video need separate verification.
