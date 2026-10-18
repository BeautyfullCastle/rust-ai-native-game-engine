# Opt-in mesh hazards CLI

`orr_mesh_hazards` selects **MeshHazardsV1** explicitly. Building with the feature
alone does not register hazards in `orr_mesh` or the legacy MeshNavigation host.
The feature implies `mesh-mission`, so the legacy host keeps its mission-enabled
profile rather than the bare navigation-only build profile. This is a
bounded point-agent gameplay profile, not a physics controller, rendering feature,
or editor/export completion claim.

Build:

```sh
cargo build -p orr_remote --features mesh-hazards --bin orr_mesh_hazards
```

## Explicit scene upgrade

Start with an existing admitted mesh-navigation/mesh-mission `.scene.yaml`
(created, for example, using `orr_mesh new`; see
[mesh navigation authoring](mesh-navigation-authoring.md)). Use absolute paths:

```sh
orr_mesh_hazards upgrade \
  --scene /absolute/mission.scene.yaml \
  --output /absolute/mission-hazards.scene.yaml \
  --spheres '2,0,2,0.5;3,0,1,0.25'
orr_mesh_hazards inspect --scene /absolute/mission-hazards.scene.yaml
```

Each sphere is `X,Y,Z,R`, separated by semicolons; quote the argument in shells.
One to eight spheres are supported. Numbers use checked fixed-point parsing,
radii must be positive, and geometry must meet the runtime's checked bounds.
Unused slots are zero. The graph revision is taken from the admitted source pin,
not provided independently. Spheres may overlap a route without lying on the mesh.

Upgrade admits the source under the legacy registry, explicitly reopens it under
the hazard registry, validates the configuration, and verifies an exact frame
save/reopen before creating the output. Source bytes are unchanged. The output
must be a new `.scene.yaml`; existing files (including the source), symlinks,
symlink parents, parent traversal, and invalid scenes are rejected. This uses the
existing create-new/write/sync authoring convention, not crash-atomic publication:
a failed write may leave a partial new output and reports an error. Path checks
follow the existing check-then-open convention; they do not guarantee safety
against concurrent adversarial replacement of parent directories.

Already upgraded scenes are rejected as upgrade inputs. Runtime mission progress,
hazard loss, and failure fields cannot be authored; fresh scenes reopen at tick
zero with zero runtime state. Inspection reports raw fixed-point sphere values
(in units of 1/65536) and the admitted document checksum. It reads the saved
scene, not a running host's play snapshot.

## Local host and ERP authoring

```sh
orr_mesh_hazards serve --scene /absolute/mission-hazards.scene.yaml \
  --dev-no-auth --bind 127.0.0.1:7777
```

`--dev-no-auth` is mandatory and must be explicit. Bind addresses must be IPv4 or
IPv6 loopback; wildcard/public binds are rejected. This tool adds no token,
credential, or remote-access setup. Optional `--stop-after-ms 1000` runs a bounded
local smoke session (allowed range 1–600000).

Use the existing generic `orr` ERP client in another terminal:

```sh
orr --erp ws://127.0.0.1:7777 status
orr --erp ws://127.0.0.1:7777 schema MeshHazards
orr --erp ws://127.0.0.1:7777 get @MeshHazards
orr --erp ws://127.0.0.1:7777 propose 'resize first hazard' sset 'MeshHazards.spheres[0].radius=0.75'
# Inspect the returned proposal ID; verify and accept that ID when appropriate.
orr --erp ws://127.0.0.1:7777 proposals
orr --erp ws://127.0.0.1:7777 save
```

`save` prints YAML; `save --write` explicitly overwrites the host's configured
hazard file, so use it only when intending to keep the edit. For full workflows
see [the ERP agent guide](AGENTS.md). Admission validates every proposal/edit,
including graph pinning, active count, canonical unused entries, and zero runtime
state. Do not author `MeshHazardRun`; gameplay produces it. Edits while playing
are refused by this profile. Inspect live play state with the host's existing ERP
play-read mechanisms rather than reopening the saved document.

A swept hit rejects that entire movement tick, preserves the pre-tick position,
stops the agent, and records loss. The lowest authored sphere index touched anywhere during the attempted tick
is recorded, even if a higher-index sphere was touched earlier.
Hazards run before mission collection/exit; loss and win are terminal until reset.
This profile makes no performance claim.
