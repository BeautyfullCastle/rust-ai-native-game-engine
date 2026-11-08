# Create an animated material sampler variant

In the model package importer, choose a cooked animated source and enable the sampler variant explicitly. Inspect its material catalog, select a used material, then choose linear or nearest filtering and Clamp, Repeat or Mirror for each texture axis, and give the result a new asset identity and package destination. Other variant modes are mutually exclusive.

Preview prepares the new animated model without publishing or installing it. Changing the source, selected material or sampler settings retires the preview. Import uses the same validated authoring operation and the inspected source hash, with the existing explicit license acknowledgement and package publication checks.

The new model changes only the selected material's three sampler defaults, plus its new asset identity and provenance. Images, other materials, geometry, skins and animation clips remain intact. Original inputs and installed package objects remain unchanged. Install the new package, assign its animated model and save through the existing model binding workflow.

Sampling defaults affect texture addressing and filtering; they do not resample image pixels or alter UV coordinates. Existing rendering policies still apply. CPU tests verify authored data and save/use paths. Windows runtime, GPU pixels, physical play and video need separate verification.
