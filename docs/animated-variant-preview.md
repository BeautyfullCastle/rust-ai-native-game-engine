# Preview animated variants before package creation

The cooked animated-model import screen can prepare a color or PNG skin variant in memory and show it in an isolated **Animated variant preview** window. No package is created, installed, assigned or verified by this action.

In an Editor with `package-manager,room-animated-models`, open an idle editable local Room project. Choose **Cooked animated model**, enter the source and enable exactly one color or PNG skin operation. Complete its material, logical ID and RGB or PNG inputs. A material selected from source inspection keeps its original-source hash guard.

Click **Preview animated variant**. Preview needs no destination, package name/version, license file or redistribution acknowledgment. It does not grant rights to redistribute the model or PNG. **Cancel variant preview** retires a pending or accepted result; an already running worker finishes without publication or acceptance, and no second worker starts alongside it.

The preview window displays the produced asset ID and source/PNG/result hashes. Select a clip and enter **Variant preview seconds**, then click **Sample variant pose**. Time must be finite and within that clip. Invalid input leaves the last accepted pose intact. There is no autoplay and game time does not advance. Use the separate front/right/back/left, reset, drag-orbit and wheel-zoom controls to inspect the model. Closing the preview releases its accepted result and view resources.

The window uses the actual produced `AnimatedModel`, not a fabricated installed-package record. It has an isolated camera, texture and one-model GPU cache. Model and pose changes, close, stale form/project changes or blocked editor state retire old results and release preview resources. CPU pose controls remain available when no GPU render state is supplied, while drawing reports that a GPU is required.

Preparation reuses existing color/PNG producers and bounded regular-file/path checks. Source and PNG bytes are stable during each read and become admitted snapshots; paths are not pinned until publication. A catalog-selected source hash must still match before preparation. Complete animated validation, clips, selected used material, numeric/image budgets and output limits remain authoritative. The worker is read-only: no compiler, subprocess, temporary authoring file, license read, package lock or scene write.

Only one preparation worker runs at a time, interlocked with import and source inspection. Exact source/spec/PNG path/hash/project/generation changes retire its result, even if the form is closed. The preview does not automatically watch same-path external file mutations. Request a new preview after changing external files. Produced asset hashes identify captured result bytes; they do not establish that future import will use identical external inputs.

To publish the chosen variant, complete the normal destination/package/license inputs and explicitly import. Import rereads and validates the source and PNG and performs its normal verified staging. It remains separate from preview, with no automatic installation or binding changes. Existing installed-package preview remains a different workflow.

Focused tests verify producer bytes/hashes, stale/cancel fences, explicit pose controls, camera/resource reset and actual Color/PNG preview workers with unchanged files, Frame and consent. CPU/UI-harness/render submission checks do not prove actual GPU appearance or native window/input/audio/video. The connected computer remains locked and historical Windows publication failures are not retried. No real playtest video is claimed without separate execution.
