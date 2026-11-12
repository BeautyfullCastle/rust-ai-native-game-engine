# Create a static model UV variant

In the model package importer, choose a cooked static source and enable static UV authoring. Inspect its material catalog, select a used material and enter U/V offsets, positive U/V scales, a quarter-turn rotation and a `.uv.json` asset identity. Explicitly acknowledge distribution rights and import to a new package destination.

The existing static UV authoring operation scales UV0, rotates counterclockwise around UV origin in 90-degree steps, then adds the offset. All primitives using the selected material change; other UVs, positions, normals, transforms, materials and image pixels stay intact, apart from new identity and provenance. Original source and installed package objects are immutable.

The form validates the current inspected source and settings before Import. The producer rejects invalid numeric values, nonpositive scales, underflow, intermediate overflow and output UVs outside +/-65536 before publishing. Changed source hashes reject old selections. This tool does not add a pre-import static model preview window; inspect the resulting model through the existing imported-package preview.

Install the new package, assign the static model through existing bindings and save. Sampling beyond 0..1 follows existing material wrap settings; images and samplers are not changed. CPU tests verify coordinates and saved-model consumption. Windows runtime, GPU pixels, physical play and video require separate verification.
