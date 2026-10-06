# Yard3D scene authoring and host-timeline skeletal playback

This is the main editor viewport backed by the existing Yard3D ERP host and
`RemoteBridge<Yard3D>`. It is not the separate Arena animation-preview pane.
The editor never owns a second simulation/document.

## Open and edit

```sh
cargo run --release -p orr_editor --features models -- \
  --game yard3d --scene scenes/yard3d_authoring.scene.yaml
```

Without `models`, Yard3D procedural rendering, picking, reflected inspection,
scene editing and the host timeline still work. Model import/package dependencies
remain optional. `animated-models` adds host-timeline skeletal playback in the
main Yard3D viewport and preserves the older independent Arena preview pane.

The supplied fixture contains ground and two dynamic boxes. Rain is disabled and
the body limit is 128. `+ 3D Box` creates a dynamic box through `world.spawn` and
selects its persistent GUID. Hierarchy and viewport selections share one selected
entity. Picking finds the nearest oriented collider bounding proxy; it is not
mesh-accurate picking, and sphere/capsule proxies are conservative boxes.

Right-drag orbits, middle-drag pans, the wheel zooms. Open **Exact 3D transform**
to enter decimal XYZ and the entire unit quaternion (x/y/z/w); **Apply XYZ +
rotation** sends one host transaction. A rejected quaternion rolls back the
position as well. Existing reflected fields remain available. Scene undo/redo
uses the host's document history. Vec2 selection nudge and 2D batch movement are
not offered for Yard3D.

## Assign a verified static model

Install a package into a project using the existing package manager. For example,
with a scene copied into a disposable project directory:

```sh
mkdir -p /tmp/yard-project
cp scenes/yard3d_authoring.scene.yaml /tmp/yard-project/yard.scene.yaml
cargo run --release -p orr_package --bin orr_pkg -- install /tmp/yard-project \
  --path assets/imported_scene_demo
```

Open the copied scene in that project.
In **Static model binding**, set **Project** to the project directory relative
to the scene directory (`.` when the scene is at the project root), then choose
**Create model bindings**. Set **Package** to `sample-imported-scene` and **Asset**
to `foreground.glb` or `background.glb`. Select a box and click **Assign static
model**. The assigned mesh replaces that body's decorative proxy in the main
viewport; its collider and physics are unchanged. Selection still highlights the
collider proxy.

Local model offset, positive scale, and unit quaternion are presentation-only.
The final model placement is body pose × local offset/rotation/scale × imported
node transform. Body position/rotation are read from the latest host snapshot;
no displayed float is written back to physics. Imported and procedural geometry
share one single-sample color/depth target, so nearer opaque geometry occludes
farther geometry regardless of batch order.

Only declared, verified installed package files are accepted. Package digest,
source hash, path containment and model validation are checked before assignment.
Changed content is diagnosed rather than silently changing a saved binding.
An invalid replacement or explicit reload retains the last valid document/cache.
**Reload verified models** explicitly checks installed content again.

## Assign skeletal content and use the host timeline

Build with `--features animated-models` and install `assets/animation_demo`
with the same package command. The package is `sample-animation`, asset
`animated.glb`; it contains the independent `bend` (index 0) and `pulse`
(index 1) clips. Choose the animated kind, clip index and Once/Loop policy,
then assign to a persistent body GUID. Static and animated assets can coexist
in the same Yard project and sidecar. Yard's loader permits only compiled
model/animation capabilities; the Arena preview retains its narrower policy.

Animation uses exactly the immutable host snapshot that supplied the Body
transforms: absolute `tick / tick_rate` at fixed 1x. Host speed already changes
tick progression and is not applied again. Rolling history does not move the
animation origin. Paused repeated frames therefore use identical poses;
Step, backward Seek and restart sample directly from the requested tick.
Edit and Stop use rest pose, while paused Play at tick 0 and Seek 0 use the
selected clip's time-zero pose. Once holds its endpoint and Loop wraps.
Invalid clock/range/pose input rejects the frame before changing visible output.

For a skinned asset the external placement is Body pose × local TRS. The
sampled hierarchy is applied once by the existing skinning renderer; the
mesh-node transform is not applied a second time. Instances sharing one
immutable asset use one renderer/batch but have independent owned sampled poses.
Procedural, static and skinned batches share one depth attachment and clear.
The complete mixed CPU frame is preflighted before target resizing or GPU cache
changes. Model bytes and floating-point poses remain outside authoritative ECS.

## Persistence, playback and boundaries

- The sidecar is `<scene path>.models.json`, written as schema version 2 with
  explicit `static` or `animated` kinds, package/path/digest/source-hash identity,
  local TRS and persistent scene GUIDs. Animated bindings also persist a clip
  index and `once`/`loop` policy. Version-1 static sidecars remain readable and
  migrate on save. Recyclable ECS handles, GPU handles and elapsed playback
  time are never persisted
- **Save model bindings**, **Undo model**, and **Redo model** affect only the
  sidecar. The ordinary scene Save/Undo/Redo affect only the host scene. Save
  both; a successful scene save does not mean the sidecar was saved
- Existing sidecars are loaded on local scene open. A failed sidecar open does
  not replace the current valid binding state. Dirty sidecars block replacement
  until saved or explicitly discarded
- At most 256 bound entities and eight distinct verified model assets are
  presented. Orphan GUID bindings do not attach to a new entity reusing an old
  handle
- Model binding edits are disabled during Play. Play/Pause/Step/Seek/Stop are the
  actual Yard3D host timeline. The bound mesh follows authoritative physics;
  Stop restores authored poses. This is not a controlled 3D character game
- A remote host's scene path never authorizes local package or sidecar access
- Proposal preview is currently not rendered in 3D; the viewport explicitly
  displays the live host snapshot and labels that limitation

The composed path is bounded opaque diffuse, single-sample, with no shadows.
It does not claim full glTF PBR, imported-mesh colliders, skinned shadows,
root motion, a character controller, IK, retargeting, an animation graph,
prefab/export or scene/sidecar atomic project save. Standalone
renderers retain their existing behavior. Native-window, physical-GPU and
Windows-local acceptance remain separate from software-GPU readback tests.

## Focused verification

Use the repository toolchain and serialized release test lane. GPU tests must set
`ORR_REQUIRE_GPU=1`; an unavailable adapter is a failure, not accepted evidence.
The focused suites are `orr_remote::yard3d_authoring`,
`orr_editor::{yard3d_authoring,model_bindings,yard3d_models,yard3d_animated}` and renderer
`imported_scene` plus standalone `gpu3d` regression. `ORR_YARD_CAPTURE_DIR`
optionally records the main viewport's PPM readbacks.
