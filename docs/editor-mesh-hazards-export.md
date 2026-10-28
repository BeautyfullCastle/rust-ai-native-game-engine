# Hazard game export with an authored camera

Build the editor and `orr_mesh_hazards_player` with `mesh-hazards-export`.
This opt-in route is separate from legacy mesh export.

1. Create or open a local **MeshHazardsV1** scene. Apply hazard and mission
   changes, enable hazards, save the scene and remain in Edit mode.
2. Optionally author a perspective or orthographic view in **Mesh game camera**.
   Capture or Apply the camera, then **Save game camera** separately from the
   scene. Save or discard camera drafts before Play or export.
3. Choose **File > Export hazard game…** (or the Export guide's settings button).
4. Enter the absolute path to a self-built, QA-tested hazard player and a new
   absolute output directory. Paths are literal filesystem paths, not commands.
5. Confirm runtime trust and choose **Export hazard game**.

## Bundle contract

Camera-free exports retain schema 1 and exactly three files: `game.scene.yaml`,
`orr.mesh-hazards-bundle.json`, and the x86-64 PE/ELF hazard player. Camera-bearing
exports use schema 2 and exactly one additional file:
`game.scene.yaml.mesh-camera.json`. The manifest records its exact filename,
length and SHA-256. The existing camera decoder validates the source scene name,
graph revision, projection and follow configuration; only the exported copy is
rebound to `game.scene.yaml`. The source sidecar is never rewritten by export.
Moving the whole bundle does not require the original source directory.

Both versions require the native `.orrhaz` player capability. Schema 2 additionally
requires `.orrhcam` with `ORR_MESH_HAZARDS_CAMERA_V1`. Markers are bounded native
section-format checks, **not executable trust**. Supply a player you built and
independently tested from trusted source. The editor copies it but never runs it.

The player uses the same authored camera, projection, follow and mesh picking
path as the editor. Play uses the saved view; Restart restores it. Camera settings
are presentation-only and do not change simulation checksums or camera-free defaults.

## Safety and workflow limits

- Hazards must be enabled. Art, props, lighting, HUD, animation and unknown
  adjacent attachments remain explicitly rejected, not silently omitted.
- Read-only/remote hosts, Play, preview, dirty scenes or cameras, unapplied
  drafts, proposals, transactions and conflicting authoring work block export.
- One background job runs at a time. Camera editing, Save, Open, creation, Play
  and Step are blocked while it is pending. Closing the export window does not
  stop polling. Cancellation only cancels result acceptance: published output
  is retained, so inspect it before retrying.
- Camera presence and original source bytes are captured at request time and
  checked at worker admission, before publication and on completion. Addition,
  removal or replacement invalidates the result. Scene/runtime identities and
  hashes are also checked; existing output is never overwritten.
- Files are bounded and regular; symlinks, FIFOs, case aliases, malformed camera
  data and incompatible runtimes fail closed. Path checks are not a sandbox
  against adversarial concurrent ancestor-directory replacement.

## Acceptance boundary

Reproducible tests cover camera editing/history/save/reopen, projection and
synthetic picking, Player state transitions, simulation parity, four-file
relocation, capability checks and malformed/stale-input rejection. Headless
EditorApp/PlayerApp tests exercise application logic, not GPU output or a launched
native executable. Actual rendering requires a GPU lane; normal exported-player
launch requires a native window. Record those checks separately rather than
inferring them from headless success.
