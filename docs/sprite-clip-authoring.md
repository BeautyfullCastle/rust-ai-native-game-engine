# Optional sprite clip authoring

The bounded core/history/Linux-writer and panel widget suite has passed focused local acceptance, including actual package/PNG admission and an omitted-Apply negative/restored control. The additional authored-package Play/GPU/export acceptance sources are prepared but remain uncompiled and unexecuted until separately reviewed validation. This is not yet a complete production acceptance claim.
Build the explicit `orr_editor/sprite-clip-editor` feature to expose `orr_sprite_clip PROJECT PACKAGE DOCUMENT CLIP NEW_VERSION`. Default editor/runtime behavior is unchanged.

The standalone tool opens an already installed dependency-free sprite package. It uses the compiled sprite capability, project verification and hash-checked asset reads, then the existing PNG decoder. It never installs a package or changes the project lock. Package scope is one sprite document, its PNG atlas, LICENSE.txt and optional same-stem RGBA bytes.

Edit ordered frames, positive integer millisecond durations and Loop/Once. Apply commits a valid draft atomically; failed drafts retain the previous document. Undo/Redo is bounded to32 entries. Save As requires a strictly newer stable version and a new absolute destination. Atlas, region IDs, other clips and license bytes are preserved. Installing the new version and assigning it in a saved scene remain explicit separate steps.

Filesystem publication is currently Linux-only. Unsupported platforms fail closed; no overwriting rename fallback is used. The Linux writer rejects unsafe ancestry, symlinks, traversal and existing destinations; private staging and atomic no-replace publication protect other users. Malicious same-user/root mutation is outside its guarantee. Interrupted writes may retain a private stage; failures after publication report `published=true`. Never delete a destination or leftover stage automatically to retry.

Registration reuses the locked rustix1.1.5 dependency with Linux fs/process features and existing sha2. Writer fault hooks compile only into library tests. Widget integration tests import the public panel and use real pointer/key actions.

Required before completion: exact feature-off and enabled inventories, writer rejection controls, real widget/save-new-version, omitted-Apply negative and restored positive, actual PNG admission, install/Assign/save/reopen, authoritative Frame and Loop/Once traces, source-hidden export and GPU captures, strict/default gates and dedicated hosted CI. Placeholder atlas fixtures prove byte preservation only. Static review is not execution proof.

## Explicit workflow and acceptance boundary

Build with `cargo build --release --locked -p orr_editor --features sprite-clip-editor --bin orr_sprite_clip`, then run `orr_sprite_clip PROJECT sample-sprites sprites.json walk 1.0.1` against a verified installed package. The window displays authoring controls; it does not currently provide an atlas-image preview. Save As publishes a new package directory without changing the project's active lock. Explicitly install that directory, assign its clip through Sprite bindings, save/reopen the sidecar, and validate Play/Frame behavior before export.

The prepared dedicated gate `tools/check-sprite-clip-authoring.sh` requires exact ordinary and ignored inventories and explicitly runs every mandatory GPU acceptance. Standalone offscreen GPU tests cover the actual eframe App controls, selected values and save/error outcomes. Actual atlas-region pixels are checked in the composed EditorApp framebuffer and original/exported Arena runtime captures, not inferred from controls-only screenshots. Software GPU results identify their adapter and do not imply hardware or native-window coverage.

Prepared export tests require pinned external runtime/exporter hashes, read-only exported trees, namespace-hidden source paths and before/after byte/mode/lock inventories. They must preserve original source files and rejection diagnostics. No child invocation builds, installs software or downloads dependencies. Unsupported writer platforms remain fail closed. Hosted CI and full local gates must pass before feature completion or publication.
