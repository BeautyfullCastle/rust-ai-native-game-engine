# Use cooked animated variants on a Mesh agent

Build the editor with `mesh-animated-models`. In a local MeshNavigation scene with an admitted graph and agent, open **Mesh agent animation → Import animation source**. Supply a self-contained `.glb`, `.animated.json` or `.orrmodel.json`, a separate nonempty UTF-8 license, a new package directory, and its name/version. Review rights and click **Import animation package**.

Cooked inputs include the output of the existing animated color, PNG skin, sampler and UV authoring tools. They retain embedded geometry, images, material settings, nodes, skins and clips. The package preserves the exact admitted source bytes as `model.animated.json` and the license as `LICENSE.txt`; raw GLB packages keep `animated.glb`. The source limit remains 16 MiB and license limit 1 MiB. Existing model, pose, decoded-memory and sidecar limits still apply.

Recognized cooked suffixes select the typed animated-model decoder directly. They never fall back to raw import; arbitrary filenames containing cooked JSON are no longer auto-detected. Static, malformed or unsupported payloads are rejected. The raw GLB path still refuses external resources.

Cooked dependency entries describe history only. At most three unique records are accepted: a leading `$source`, an optional safe relative `.animated.json` source and optional `.png` source. Hash strings must be lowercase SHA-256. Historical files are never opened, fetched or independently verified; these entries do not prove recipe execution, licensing or ownership. Actual imported source, license and canonical embedded payload hashes retain their existing checks.

Import stages a candidate. Explicitly acknowledge use rights and **Load mesh animation**, select moving and optional idle clips, then **Apply mesh animation** and **Save mesh animation**. Apply/Undo/Redo remain presentation history; importing does not install packages or change the simulation. A saved sidecar can reopen without the original source, license or produced package.

Stable input identities, changed-byte checks, new-only publication, retained worker ownership and stale-context rejection are preserved. Use trusted, quiescent directories. A post-publication warning means output exists or may exist and requires inspection before any explicit retry.

Acceptance covers all four producer variants through the actual import worker, explicit Load/Apply/Save, hidden-source reopen, moving/idle poses and unchanged Frame checksums. Raw GLB and matrix-input regressions remain. The computer is locked; native window/GPU appearance and playtest video are unverified. CPU pose and UI harness results do not establish GPU rendering.
