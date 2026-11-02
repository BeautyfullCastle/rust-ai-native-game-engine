# Navigation export from the editor (optional Linux x86-64)

Build `orr_editor` with `--features navigation-export`. This default-off option
adds the **Navigation export** bottom tab for a local NavigationYard3D scene.
The supplied player must separately be built with `authored-bundle`.

1. Save the terrain, Build route, and Save the scene.
2. Enter an already-built runtime path, its expected lowercase SHA256, and a new
   output path. Explicitly trust that executable: verification runs its code.
3. Click **Export saved Navigation**. No Save, Build route or runtime compilation
   is performed automatically.

The button captures bounded saved scene/terrain bytes and requires a coherent,
idle local Edit state: no unsaved scene or terrain, route draft mismatch, stale
pin, proposal, gesture, transaction, control, preview or ERP listener. A bounded
local authoritative Frame read verifies full initial-state equality; source
capture may briefly wait for that local response. SHA256 and a user revision are
integrity/owner records, not executable authentication or license verification.

The worker uses the same typed Rust publisher as the CLI, rechecks the captured
source at start and before publication, executes bounded trusted-runtime smoke,
and refuses existing output. Unrelated editing may continue. Completion names
the **captured scene/checksum**, not later edits. Hiding the tab does not abandon
the job: close is blocked until terminal completion and join. A failed or lost
worker result is treated conservatively as uncertain; **Inspect previous
destination** acknowledges inspection without deleting or automatically retrying.
Changing the runtime path or hash clears its trust checkbox.

The output is the [five-file Navigation bundle](navigation-bundle.md). Its launcher
runs headlessly on the tested Linux host with its system/shared-library
requirements. Point-agent radius/headroom/step clearance remain zero and terrain
is at most 17×17. This does not add a window, keyboard gameplay, GPU capture,
other-host portability or general navigation support.

Focused acceptance covers authoritative saved-state guards, queued source edits,
worker ownership/uncertainty, actual widget dispatch and real bundle relocation
with authoring inputs hidden, empty working directory, 60 ticks and restart.
Legacy CLI behavior remains supported. The frozen original candidate is untouched;
its exact 83-path inventory exclusion remains unconfirmed.
