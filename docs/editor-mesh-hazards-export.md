# Scene-only hazard game export

Build the editor and `orr_mesh_hazards_player` with the optional
`mesh-hazards-export` feature. This feature is separate from legacy mesh export.

1. Create or open a local **MeshHazardsV1** scene. Apply hazard and mission form
   changes, save the scene, resolve proposals and transactions, and remain in Edit.
2. Choose **File > Export hazard game…**.
3. Enter the absolute path to a self-built, QA-tested hazard player and an absolute
   output directory path that does not exist. These are literal filesystem paths,
   never shell commands. No file-picker dependency or shell invocation is used.
4. Check the explicit runtime trust checkbox, then choose **Export hazard game**.

The worker validates the saved scene and the executable's format/capability
marker, then publishes exactly three files: the scene, the profile manifest and
an x86-64 PE (Windows) or ELF (Linux) executable. A capability marker does not
establish executable trust. The editor does **not** execute the supplied player.
Only use a player built and independently tested from trusted source.

## Current limits

- Hazards must be enabled. A starter created without a sphere must have hazards
  authored and enabled before export. The exporter reports this unsupported case.
- Adjacent agent art, props, camera, lighting, HUD, and animation presentation
  sidecars are refused explicitly; they are not silently dropped or composed.
- Read-only/remote hosts, Play, preview, dirty scenes, unapplied hazard/mission
  drafts, transactions, proposals and conflicting authoring operations block export.
- A single bounded background job runs at a time. Save, Open, new-game creation,
  Play and Step are blocked while that job is outstanding. The UI remains responsive
  and continues polling even if its export window is closed.
- Cancellation cancels acceptance of the result. It cannot undo an already-published
  directory. Wait for the result and inspect the destination before retrying.

The request captures scene path, load epoch, authoritative checksum, runtime path
and file identity. These are checked before dispatch and again on completion;
worker-side bounded runtime hashes must match the bundle receipt. Changed inputs
or cancellation produce a retained-but-not-current result, not a success claim for
the current editor scene. The exporter independently validates bytes, paths,
sidecars and no-overwrite publication. This is not a filesystem sandbox against
adversarial concurrent directory replacement.

Model tests cover trust/blocker refusal without scene writes, busy/disconnected
worker handling, stale epoch/path/checksum/runtime rejection, and Play refusal.
Actual GUI export and relocated-player execution need separate integration evidence.
