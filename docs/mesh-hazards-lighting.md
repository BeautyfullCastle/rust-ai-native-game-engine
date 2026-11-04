# Authored lighting in hazard games

With `mesh-hazards-export`, a local MeshHazardsV1 game can keep its authored
lighting through Edit, Play, export and the relocated native player. Lighting
changes presentation only; movement, hazards, missions and simulation checksums
remain unchanged.

## Author and deliver

Open **Mesh game lighting** in the Inspector. Set sun direction, color,
intensity, sky/ground ambient lighting, shadows, exposure and tonemapping.
Optionally enable **Saved HDR post-processing** and bloom. Choose **Apply game
lighting**, inspect the result, then **Save game lighting**. Undo/Redo changes
this lighting draft independently of the simulation. Save, undo or explicitly
discard pending lighting edits before Play, opening another scene or export.
Malformed or stale saved lighting blocks Play/export rather than silently
falling back to default lighting.

Use **File > Export hazard game…**, select a player you built and tested, choose
a new output directory, and confirm runtime trust. Existing camera and static
character artwork may accompany lighting. The original scene filename and graph
revision are validated before only the exported filename binding is changed to
`game.scene.yaml`. Source files are never rewritten. Move the entire bundle to
run without the original authoring directory.

## Bundle and rendering contract

A lighting-bearing bundle uses schema 4 with required
`game.scene.yaml.mesh-lighting.json` and optional camera/art files. The closed
manifest records lengths and SHA-256 digests. The executable must advertise
`ORR_MESH_HAZARDS_LIGHTING_V1` in `.orrhlit` as well as the base capability and
any required camera/art capabilities. These markers and hashes establish
compatibility and identity, not executable trust. Light-free bundles retain
schema 1, 2 or 3 and their previous contracts.

The existing bounded lighting decoder, graph/scene binding, byte checks and
post-processing renderer are reused. On unsupported HDR hardware, the player
shows an explicit warning and renders without HDR for that request. Authored
settings and bundle files remain unchanged; ordinary lighting still applies.
Unknown attachments, props, HUD and animation remain rejected.

The exporter pins lighting presence and exact bytes across asynchronous work,
rechecks before publication and completion, and never overwrites an output.
A stale completion retains already published output but does not report it as
current. File checks assume trusted, quiescent directories, not hostile
concurrent ancestor replacement.

## Verification

`orr_mesh_hazards_player --bundle /absolute/game --verify-headless --ticks 50`
validates the executing binary and loaded lighting, advances the admitted game,
and checks restart parity. Its receipt includes the lighting digest. It does
not render pixels or establish window/input acceptance.

Focused tests cover authoring history, profile/source pins, strict schema and
capability rejection, relocated execution, simulation parity, software-GPU
lighting and explicit HDR fallback. Native-window, physical-input, Windows and
hardware-GPU results must be recorded separately. This document alone claims
none of those checks passed.
