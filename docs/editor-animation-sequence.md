# Create a two-clip animation sequence

In the model package importer, choose a cooked animated source and enable sequence authoring. Inspect its clip catalog and choose the first and second clips from that current source. Set the transition duration, a new clip name and a new `.sequence.json` asset identity. Sequence mode is exclusive with the other authoring modes.

Preview creates the variant in memory without publishing it. Import requires explicit distribution-rights acknowledgement and a new package destination. Changing inputs retires the preview, and a source hash change rejects old inspected data before publication.

The existing sequence authoring API appends one clip; original clip indices, geometry, materials, skins and images stay intact. It plays the first clip, transitions from its final pose to the second clip's initial pose, then plays the second clip. Both clips must use LINEAR channels with the same node/property targets. Incompatible pairs reject before publication. Transition duration must be finite and positive, at most 60 seconds. No overlap, automatic locomotion state machine or root-motion extraction is added.

Install the new package and assign the appended clip through existing animation bindings, then save. CPU tests inspect the saved model and sample its poses, including the transition. GPU rendering, Windows runtime, physical play and video require separate verification.
