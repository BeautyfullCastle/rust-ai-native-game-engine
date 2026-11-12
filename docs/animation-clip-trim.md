# Create an animation clip from a time range

The model-package importer can append a trimmed clip to a cooked animated model. Existing clip indices stay unchanged, so current Room mappings remain valid.

Choose Cooked animated model, select the `.animated.json` source and inspect it. Enable Create trimmed animation clip, choose a source clip, and enter start/end seconds, a unique new clip name and a new `.trim.json` asset identity. Preview animated variant prepares the same model payload used by import without creating a package or granting distribution consent. Review the license, choose a new package destination and explicitly acknowledge rights before importing.

Install the resulting package, assign its animated model, and choose the appended clip in the Room character settings. Apply and use the existing paired Save action. Both the saved model binding and character policy must be retained when deploying the Room.

The range must have finite, distinct representable sample times satisfying `0 <= start < end <= source duration`. Start and end are sampled using the existing runtime animation semantics; interior keys retain their values and move to time zero relative to the selected start. LINEAR translation/scale, shortest-path quaternion interpolation and STEP channels are supported. Short channels retain the runtime endpoint clamp behavior. The output duration is the selected interval, with exact endpoint samples. Playback looping or one-shot behavior remains a character/binding choice.

The operation appends one clip and preserves existing clip data, geometry, materials and rig. New asset identity and provenance identify the derived payload. Clip count, aggregate animation keys/bytes and cooked output limits still apply; precision loss that collapses sample times is rejected. There is no silent key merging, source overwrite or in-place package update.

Preview and import capture clip, range, name, identity and source hash. Source changes require inspection again, and edits retire old previews, including an edit followed by restoration. Color, PNG skin and trim authoring modes are mutually exclusive.

Automated acceptance compares trimmed poses with the original interval, checks strict malformed/budget rejection and legacy preservation, then exercises actual UI preview/import, installation, Room selection/paired Save and source-independent headless native playback. These CPU/filesystem checks do not establish Windows execution, GPU pixels or physical play coverage.