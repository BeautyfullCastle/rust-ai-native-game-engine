# Static model → mesh asset → saved navigation scene

These production tools reuse the existing model importer, canonical navigation
baker, scene admission/history and ERP host. They are not examples or an
alternative editor. **The mesh panel is not connected to `orr_editor` yet.**

## Build and author on Windows

```powershell
cargo build --release --locked -p orr_editor --features navigation --bin orr_mesh_cook
cargo build --release --locked -p orr_remote --features mesh-navigation --bin orr_mesh

$source = 'C:\Games\Level\floor.glb'
$asset = 'C:\Games\Level\floor.orrmesh'
$scene = 'C:\Games\Level\level.scene.yaml'

.\target\release\orr_mesh_cook.exe --source $source --output $asset --max-slope 1
.\target\release\orr_mesh.exe asset --asset $asset
```

`asset` prints exact decimal `first_face_points`, plus the graph revision and
counts. Choose start/goal on the intended connected surface; the first face's
distinct vertices give a small connected route. Set all three coordinates,
including the surface's Y, instead of assuming the model lies at Y=0:

```powershell
.\target\release\orr_mesh.exe new --asset $asset --output $scene --start '0,0,0' --goal '1,0,1' --vertical 0 --distance 0.03125
.\target\release\orr_mesh.exe inspect --scene $scene
.\target\release\orr_mesh.exe run --scene $scene --ticks 300
```

The coordinates above illustrate the syntax; use the inspected asset's actual
coordinates. `new` validates a complete route before creating a file. The scene
embeds its graph/settings and derives one real Frame-owned agent on admission.
`run` reopens that saved scene, advances the real simulation and prints raw Q16
position/goal, status, tick and checksum. It never rewrites the scene.

## Live authoring through the same ERP authority

```powershell
.\target\release\orr_mesh.exe serve --scene $scene --dev-no-auth --bind 127.0.0.1:7777
```

This host tool requires explicit **loopback development mode**; it provides no
new authentication mechanism or internet-facing service. It prints its real
listening URL. `--bind 127.0.0.1:0` chooses an available port, and optional
`--stop-after-ms 60000` bounds a session. Ctrl+C ends an unbounded session.

In another terminal, use the existing `orr` client, with the printed address:

```powershell
orr --erp ws://127.0.0.1:7777 status
orr --erp ws://127.0.0.1:7777 schema MeshScenePin
orr --erp ws://127.0.0.1:7777 apply 'mesh speed' sset MeshScenePin.distance_per_tick=0.25 --idle 32 --check 'entities.final == 1'
orr --erp ws://127.0.0.1:7777 undo
orr --erp ws://127.0.0.1:7777 redo
orr --erp ws://127.0.0.1:7777 save --write
orr --erp ws://127.0.0.1:7777 sim step 300
orr --erp ws://127.0.0.1:7777 sim stop
```

Edits rebake through admission and share one history. Invalid graph/spec changes
leave the document and history unchanged. Play operates on a copy; Stop restores
the authored document. This is the distinct compiled `MeshNavigation` host,
not the existing terrain-navigation game's schema.

## Import and numeric contract

- `.glb`/`.gltf` use the existing bounded, strict static importer and its
  contained dependency resolver. Its unsupported extensions/skins/animations
  remain unsupported. `.orrmodel` uses the existing cooked-model validator.
- All primitives count toward **1,024 source vertices / 512 source triangles**
  before downward/vertical/slope filtering; final canonical asset ≤8,192 bytes.
- Position and affine coefficient IEEE-f32 bits become Q16 by integer-only
  nearest rounding, ties away from zero. Affine dot products accumulate in
  checked i128 and round once. Nonfinite/overflow/quantized-singular transforms
  reject. The adapter is deterministic for the same model bits; newly imported
  glTF transform evaluation is not claimed newly cross-platform verified.
- Negative quantized determinants reverse face indices, preserving the existing
  model renderer's mirrored-front-face semantics. Only upward faces are retained;
  vertical/downward faces are not silently turned into walkable floors.
- Transformed coordinates are limited to +/-256. Degenerate geometry and
  unsupported canonical baker bounds reject rather than silently repairing it.
  See [static graph limits](static-mesh-navigation.md).
- This is a point-agent/open-sky graph: radius/headroom/step=0, no swept physics,
  collider clearance, crowd avoidance or full navmesh claim. Distance per tick
  advances an absolute route parameter, not a physical speed bound.
- The scene's world is its embedded walkable graph: world revision equals graph
  revision, not an independently checked collider-world identity or authentication.

Asset and scene outputs require absolute paths and existing ordinary parents.
They reject existing symlink/reparse/junction components and use `create_new`,
never overwrite. Reads/writes are bounded; failed writes can leave a new partial
file. **No hostile-parent isolation, full-file atomic publication, crash/power-loss
durability or publisher authentication is claimed.** Normal host Save deliberately
rewrites its authored scene through the existing save path.

## Actual acceptance and remaining editor work

Windows release/locked/offline final feature batch: **99 tests pass** =67
navigation/runtime+16 game/host/authoring+16 model/cooker. This total includes the
new tests; earlier runs are not added again. Scoped strict Clippy without extra
cfg/lint overrides, edition2021 formatting and fresh executable builds pass.

Actual GLB cooking produces a four-triangle, 480-byte graph. Relocated tools run
from a private directory after the original model and `.orrmesh` are hidden.
The embedded scene reopens/runs identically. A real WebSocket ERP verified edit,
Undo/Redo, Save and 300-tick play reaches the exact goal and agrees with the
offline admitted oracle, checksum **`0x58b73683a7e6b465`**. Four actual Windows
junction negatives reject source/output parents with all target hashes unchanged.
These are local authoring/host tests, not remote CI or game-export acceptance.

Original compile failure (ambiguous `Read`/`Write::by_ref`) and the separate
junction-harness relative-path invocation failure remain preserved. Source review
also caught and fixed finite float-shift overflow and mirrored winding before the
accepted run. Earlier temporary inactive editor cfg declarations are absent from
the final strict validation, not suppressed to pass a GUI feature.

The four reserved editor feature/module/dispatch/application hooks remain
untouched. The pending panel/game-adapter/widget work stays local and unvalidated;
ordinary `orr_editor` cannot open this new compiled game yet. Do not substitute
these CLI results for actual editor placement/save/reopen/Play, a native mesh
viewport, Windows export or source-hidden exported-game execution. #94 and whole
engine completion remain unverified; #2's original22 issues are separate.

Local reproducibility/evidence: `target/orrery-mesh-production-20261017/` contains
`verify.ps1`, v3 source/held manifests and test/lint/build logs, `pipeline.ps1`,
`pipeline-v3/receipt.json`, and `junctions-v3/receipt.json`. They are local evidence,
not hosted artifacts or a release package.
