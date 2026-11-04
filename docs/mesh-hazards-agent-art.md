# Static character art for hazard games

The optional `mesh-hazards-export` profile carries authored static character
art from a saved MeshHazardsV1 scene into its native player. Art changes only
presentation: the navigation agent, hazards, mission, inputs and replay remain
authoritative simulation state. This is not animated character support or prop
collision authoring.

## Author and deliver

Open a local hazard scene and use the static agent art controls in the Inspector.
Choose a verified static-model package, its model and license, acknowledge usage
rights, and explicitly load/apply the artwork. Existing appearance and transform
controls, Undo/Redo and **Save agent art** apply to the same embedded snapshot.
Save or discard pending art edits before Play or export. Source packages are not
needed after the embedded sidecar has been saved.

Use **Export hazard game** with a self-built player and a new output directory.
The existing explicit runtime-trust checkbox is still required. Optional saved
camera settings can accompany the character. The exporter validates the original
scene filename and graph revision, then changes only the exported filename binding
to `game.scene.yaml`. It does not repair a stale graph binding or modify source
files. Package provenance, license text and appearance remain embedded.

Scene-only bundles retain schema 1; camera-only bundles retain schema 2.
Art-bearing schema 3 requires `game.scene.yaml.mesh-art.json` and optionally the
camera sidecar. File lengths and SHA-256 digests are part of the closed inventory.
Unknown attachments, props, lighting, HUD and animation remain unsupported and
are rejected rather than omitted. The player must advertise its separate
`ORR_MESH_HAZARDS_ART_V1` native capability, and the camera capability when needed.
Capability metadata and hashes are not signatures or executable trust.

## Headless verification

The same native player can verify a relocated bundle without opening a window:

```sh
./orr_mesh_hazards_player --bundle /absolute/relocated-game \
  --verify-headless --ticks 50
```

This uses ordinary bundle and presentation admission, verifies the executing
binary, advances the real paused host, and checks Stop/Restart parity. Its JSON
receipt identifies the tick, simulation checksum, agent position and loaded art.
It does not render pixels or establish native-window or physical-input support.
Screenshot/window-size options cannot be combined with headless verification.
Normal launch continues to use the interactive player.

## Boundaries and acceptance

Artwork uses the existing strict model/provenance decoder and resource limits.
Source presence and exact bytes are pinned across asynchronous export, including
the absence of optional files. New output must not exist; published output is
retained if a later check fails, so inspect it before retrying. Filesystem checks
assume trusted, quiescent directories and are not a sandbox against hostile
concurrent filesystem changes.

Focused acceptance covers source binding and schema rejection, old bundle
compatibility, runtime capabilities, history/save/reopen, source-hidden relocated
execution, loaded appearance and unchanged simulation. Offscreen software GPU,
headless executable and native-window results are separate evidence categories.
No test or platform is claimed passed by this document alone.
