# Animated characters in hazard games

Build the editor and `orr_mesh_hazards_player` with both `mesh-hazards-export`
and `mesh-animated-models`. The animation feature remains opt-in. A build without
it rejects animated bundles rather than dropping their character artwork.

In a saved local MeshHazardsV1 scene, use **Mesh agent animation** to load a
verified animation package, choose its clip and presentation settings, Apply,
and **Save mesh animation**. The existing bounded embedded snapshot retains the
model, package provenance and license text. A fresh editor can reopen it without
the original package. Animation follows authoritative simulation ticks and does
not alter navigation, hazards, mission state or replay checksums.

Use **Export hazard game** with a trusted, matching player and a new output
directory. Animation-bearing bundles use schema 7 and require the exact
`game.scene.yaml.mesh-animation.json` record. Camera, lighting, HUD and unshaded
props remain optional. A saved static agent-art sidecar and animation cannot be
exported together in this profile. Resolve that conflict explicitly; export
never silently omits an authored attachment. Older schemas 1–6 are unchanged.

The player requires a separate native animation capability in addition to its
hazard capability and the capabilities of any other included attachments.
Capability markers and integrity hashes are not signatures or executable trust.
The exporter validates the original filename and graph revision before rebinding
only the exported animation copy to `game.scene.yaml`. Source bytes and optional
attachment presence stay pinned throughout export. Closed inventory, resource
limits, no-overwrite publication and invalid/stale-data rejection remain in force.

The relocated player uses the same saved clip, settings and tick-driven playback
as Editor Play. Stop/Restart restores the original animation state. The existing
`--verify-headless --ticks N` mode checks admitted animation and restart parity
without a native window. This is not evidence of rendered pixels, physical input,
Windows execution or hardware GPU performance. Interpolation formats, animation
events, gameplay collision and CUBICSPLINE support are outside this change.

Focused acceptance must distinguish authoring/history/save/reopen, bundle
admission, actual source-hidden relocated executable behavior, animation pose
consumption and simulation parity. No result is claimed merely by this guide.
