# Authored HUD in hazard games

With `mesh-hazards-export`, local MeshHazardsV1 games can carry an optional
authored navigation HUD through Edit, Play, export and the relocated player.
This presentation-only HUD does not change simulation state or checksums.

## Author and save

Open **Mesh game HUD** in the Inspector. Set the title, stopped/moving/arrived
messages, optional tick display, font size and colors. Choose **Apply game HUD**,
inspect the result, then **Save game HUD**. Undo/Redo changes the independent HUD
draft. Save or explicitly discard pending edits before Play, scene changes or
export. Missing HUD retains the existing game interface; a saved disabled HUD
remains disabled. Invalid or stale saved HUD blocks Play/export instead of
silently disappearing. Saving the scene alone does not save this sidecar.

These status messages describe the navigation agent, not victory or defeat.
The built-in hazard status and restart controls remain authoritative. In hazard
viewports the authored HUD is clipped below the hazard status and above any HDR
fallback warning; very small viewports may have no room for the authored HUD.

## Deliver

Use **File > Export hazard game…**, choose a self-built, tested player and a new
output directory, and confirm runtime trust. The exporter never launches that
player. Optional camera, static agent artwork and lighting can accompany HUD.
The entire bundle can be relocated without the original scene or sidecars.

A HUD-bearing bundle uses schema 5 with required
`game.scene.yaml.mesh-hud.json` and optional camera/art/lighting records. The
closed manifest pins lengths and SHA-256 digests. The runtime requires the
separate `ORR_MESH_HAZARDS_HUD_V1` marker in `.orrhhud`, plus the base marker and
any required camera/art/lighting markers. Markers and hashes are compatibility
and identity checks, not executable trust or signatures. HUD-free schemas 1–4
retain their prior contracts. Props, animation and unknown attachments remain
unsupported and are rejected rather than omitted.

The existing bounded HUD decoder validates the original scene filename and
graph revision. Only the exported filename binding changes; source files remain
untouched. HUD presence and bytes are checked at request capture, worker
admission, before publication and on completion. Output is never overwritten.
Loading rechecks the bundle after binding the HUD. These checks assume trusted,
quiescent directories, not hostile concurrent ancestor replacement.

## Verification boundaries

`orr_mesh_hazards_player --bundle /absolute/game --verify-headless --ticks 50`
checks the executing binary and loaded HUD, advances the game and checks HUD
and simulation restart parity. Its receipt includes `hud_sha256` and
`hud_restart_parity`. It does not render a window or verify physical input.

Focused tests cover profile isolation, draft/history/save, closed manifest and
capability checks, tampering, relocated execution and software-GL HUD pixels.
Native-window, Windows and hardware-GPU acceptance remain separate. This guide
does not itself claim those checks passed.
