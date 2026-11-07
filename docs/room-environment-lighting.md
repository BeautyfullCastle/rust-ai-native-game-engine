# Room environment lighting

Open a local Room project and expand **Room lighting**. Enable **Environment lighting enabled**, then open **Environment lighting controls** to set the sun direction, linear RGB color and intensity, sky and ground colors, and ambient strength. **Reset environment defaults** stages the original Room values.

These controls edit a draft. Use the existing Apply, Undo/Redo and Save actions to apply or persist the complete lighting document. Turning the point light off preserves the environment; its follow-player and activation policies affect only the point light. Resetting the complete lighting document intentionally clears all authored lighting.

Environment values are presentation data. They do not modify the simulation Frame, collider or game rules. Direction components are bounded to -1..1 and the direction must have a squared length greater than 1e-24. Linear RGB components are 0..1; intensity and ambient strength are 0..8. Nonfinite and invalid values are rejected before changing applied lighting.

Saved documents with environment values use strict version 4 JSON. Existing version 1–3 documents retain their original serialization and default Room appearance. Disabling the optional environment returns to that appearance. The 4096-byte document limit still applies.

The saved document is consumed by the editor and shared native Room renderer, including recreated windows and captures. Environment values change sun and hemisphere inputs; the current Room shadow, exposure and tone-mapping policies remain in effect. Existing point-light settings remain independent.

Acceptance checks the codec and legacy bytes, staging/history/save/reopen, prepared project consumption and CPU lighting inputs. The connected desktop is locked: actual native window interaction, GPU appearance, physical device playback and playtest video remain unverified. CPU tests do not establish those results.
