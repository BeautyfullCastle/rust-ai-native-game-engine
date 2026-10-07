# Animated-model bindings in the Arena editor

This optional feature assigns an installed animation asset and a clip index to
an existing Arena entity's persistent GUID, saves that assignment in a separate
local sidecar, and previews the actual skinned model on the editor's GPU device.
It is a **presentation authoring pane**, not a 3D gameplay editor. The existing
Arena viewport, 2D positions, collider picking, host scene format and deterministic
simulation are unchanged. The host Play/Stop timeline is separate from preview
playback. PhysGame and remote-host attachment do not enable this authoring pane.

## Build and install

```sh
cargo build --release -p orr_editor --features animated-models
# Combine independently with sprite authoring if wanted:
cargo build --release -p orr_editor --features animated-models,sprites
```

From the repository root, prepare a dedicated local project and open its Arena scene:

```sh
mkdir -p /tmp/orr-animation-project
cp scenes/arena_blank.scene.yaml /tmp/orr-animation-project/arena.yaml
cargo run --release -p orr_package --bin orr_pkg -- install /tmp/orr-animation-project --path assets/animation_demo
cargo run --release -p orr_editor --features animated-models -- --game arena --scene /tmp/orr-animation-project/arena.yaml
```

Use `/tmp/orr-animation-project/arena.animation.json` as the sidecar path,
`arena.yaml` as the scene hint and `.` as the package-project path.
The pane deliberately opens a **dedicated
animation-content project** with the compiled `animation` capability. A project
requiring other optional capabilities is rejected, even when some of those
features exist elsewhere in the workspace. Enabling this feature does not install
packages or execute package scripts.

The original fixture package is named `sample-animation`; its declared model is
`animated.glb`. Supported GLB/glTF imports use the existing bounded importer and
are cooked and reloaded before sharing an immutable model. Cooked animated model
files are also accepted. Import format limits remain those of `orr_model`.

## Authoring workflow

1. Open a local Arena scene using the normal editor workflow. Create/select an
   entity in the hierarchy so it has a real persistent scene GUID.
2. In the Inspector, click **Animated model authoring** to open the resizable
   authoring window. Expand **Animated model bindings (presentation only)**.
   Enter a local animation-sidecar filename, a scene path relative to that
   sidecar, and a package-project path relative to it. Paths are explicit user
   choices. Remote host scene paths are never local filesystem authority.
3. Click **Create animation bindings**, or **Open animation bindings** for an
   existing sidecar. A different scene, including a scene **Save As**, retains
   the sidecar and dirty state but disables assignment and preview. Save As does
   not silently retarget bindings.
4. Enter `sample-animation` and `animated.glb`, then **Load animated model**.
   Choose a clip from the imported clip list and **Once** or **Loop**, plus a
   preview speed. Clip indices are persisted; names are only labels and need not
   be unique or nonempty.
5. Click **Assign animated clip to selection**. A multiselection is one binding
   undo transaction. Assignment is separate from scene/host undo.
6. The labeled **3D animation preview** shows the first selected entity using
   `SkinnedModelRenderer`, an offscreen target, and an egui native texture on the
   eframe device. Its UNORM target uses the renderer’s existing explicit sRGB
   output encoding and needs no alternate texture view formats. Play, Pause, Stop and the time slider affect only that entity's
   transient player. Other bindings maintain independent players. Stop restores
   rest pose; seeking after Stop selects the assigned clip in a paused state.
7. Use **Undo animation binding** / **Redo animation binding** as needed, then
   **Save animation bindings**. Save the Arena scene separately if it changed.
   These are separate atomic file writes, not a joint atomic transaction.
8. Close and reopen the sidecar to restore assignments. Playback time and state
   are not saved. A dirty close requires explicit discard confirmation; cancelling
   it leaves the document and history intact.

The preview uses a fixed camera and diffuse skinning material support. There is
no 3D placement, collider creation, gameplay animation/root motion, skinned shadow
pass, PBR extension, retargeting, animation graph, or export in this slice.

## Identity, failures, and limits

- Every binding records its GUID, package name, declared asset path, verified
  package digest and source-file hash, clip index, mode and finite bounded speed.
- **Reload animated assets** explicitly rechecks installed content. Removed
  packages produce diagnostics and release old preview/player state. Reinstalling
  the same content restores the assignment on reload. Changed content produces a
  stale-binding error and requires explicit model/clip reassignment; it cannot
  silently reuse an index in a different model.
- Reload creates a new model identity and resets players. Deleted GUIDs remain
  visible as orphan bindings. A recycled frame handle never inherits a binding.
- Failed asset reload cannot continue drawing an old cached model. Failed saves
  preserve the prior destination and unsaved document/history. Invalid documents
  are rejected before becoming active.
- The sidecar is versioned and bounded; unknown fields/versions, invalid GUIDs,
  unsafe paths, invalid digests, nonfinite playback and invalid clip references
  are rejected. At most eight distinct models are held in the preview cache.
- A CPU-only UI can still author bindings and explicitly reports unavailable GPU
  preview. Headless egui interaction tests and required software-GPU readbacks do
  not constitute native-window, physical-device, or Windows acceptance.

## Focused validation commands

```sh
cargo test --release -p orr_editor --features animated-models --test animated_bindings --test animated_panel
ORR_REQUIRE_GPU=1 cargo test --release -p orr_editor --features animated-models --test animated_preview -- --test-threads=1
cargo test --release -p orr_editor --features animated-models,sprites --lib --test deps
cargo clippy --release -p orr_editor --all-targets --features animated-models,sprites -- -D warnings
```

Also regress default and sprites-only editor builds/tests and the existing
static/skinned renderer suites. #99 and #100 remain open for their wider 3D
creation and animation requirements. This feature's local source validation,
independent review, remote publication, CI and shared development integration
must be recorded separately.
