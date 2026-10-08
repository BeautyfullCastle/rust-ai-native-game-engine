# Saved Arena projects (partial issue #104)

The editor can open an existing, self-contained Arena project with its saved
scene and optional sprite bindings. This is a bounded project-opening slice of
[#104](https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/104),
not the complete project-management feature.

## Open the checked-in project

```sh
cargo run -p orr_editor --features sprites -- --project assets/saved_arena_project
```

There is no install step. Copy the complete directory, including hidden `.orr`
files, and the built editor can open that copy from any working directory:

```sh
/path/to/orr_editor --project /absolute/path/to/my-arena-project
```

The checked-in sample has two independent authored GUIDs, reversed idle/walk
bindings to make their independence visible, and a saved camera-follow target.
Play uses the existing Arena simulation. Use **Take control** and arrows/WASD to
move the first actor. **Stop** restores the edit scene and pre-Play camera.

## Project format

`orr.project.json` is strict schema 2:

```json
{
  "schema": 2,
  "engine": "^0.0.1",
  "entry": {
    "game": "arena",
    "scene": "arena.scene.yaml",
    "sprites": "arena.sprites.json"
  }
}
```

`sprites` is optional within the manifest. In this slice, all `--project`
opening requires the editor's `sprites` compile feature, including scene-only
projects. Package/sprite dependencies remain optional and default-off. A build
without that feature rejects `--project` explicitly.
Schema-1 project metadata remains valid for package tools, but it does not declare
an editor entry point. Unknown versions/fields and conflicting launch sources
are rejected. In particular, `--project` cannot be combined with `--game`,
`--scene`, `--connect`, or `--script`. Normal `--play-ticks` remains available.

The package lock and installed `.orr/packages/objects` are the only activation
and version authority. The manifest does not install, generate, select, or fetch
packages. Runtime loading verifies installed package identity, declared files,
content hashes, engine compatibility, and the actual compiled capabilities. It
never falls back to `assets/sprite_demo`, a package source directory, or an
embedded atlas. The sample keeps the original package license with its content.

Project admission validates bounded files and the declared scene, sprite
sidecar, clip references, stable entity GUIDs, and matching project/scene
identities before the host/window starts. A missing lock fails when sprite
bindings reference packages; a scene-only project with no active packages can
omit the lock, preserving the package API's empty-lock semantics. Tampered locks,
missing/tampered referenced packages, and missing/tampered atlases fail visibly.
Invalid paths, symlinks, special files such as FIFOs, oversized inputs, and
unsupported schemas are rejected.

Manifest entry paths are portable, project-root-relative paths without `.` or
`..` components. A nested sprite sidecar keeps its existing sidecar-relative
semantics: `..` is allowed only while every step remains inside the project and
the final scene/project paths match the manifest's scene and canonical root.
Admission bounds include 1 MiB each for manifest, lock, and sprite sidecar,
4 MiB and 20,000 entities for the scene, 4,096 sprite bindings, and eight unique
sprite documents. Existing package limits remain 128 packages, 4,096 declared
files per package, 64 MiB per file, and 256 MiB per package. Atlas decoding is
bounded to 2,048 pixels per axis and 16 MiB. This is bounded,
nonadversarial local-filesystem admission; it does not claim race-proof security
against an attacker replacing paths concurrently.

## Save and reopen

Scene editing/undo/save still belong to the simulation document. Sprite
assignments, their undo history, and camera-follow identity belong to the
presentation sidecar. Use scene **Save** and **Save bindings** separately. A
scene save does not silently save bindings; a bindings save does not modify the
scene or host history. Play does not rewrite either file. Close/reopen and
relocation recover the saved GUID-keyed bindings and follow target.

This slice adds no exporter, project generator, prefab GUI, joint scene/sidecar
save transaction, or general project browser. The acceptance evidence below is
real `EditorApp` execution and an offscreen composed egui/wgpu framebuffer, not
a claim of native-window automation or Windows verification.

## Reproducible verification

```sh
cargo test -p orr_editor --features sprites --test saved_project
ORR_REQUIRE_GPU=1 ORR_PROJECT_CAPTURE_DIR=/tmp/orr-project-captures \
  cargo test -p orr_editor --features sprites --test saved_project_gpu -- --nocapture
```

CPU acceptance copies the checked-in installed project, autoloads it in the real
`EditorApp`, edits a reflected position through the inspector, saves scene and
bindings separately, exercises Play/Take control/Stop, verifies recorded replay
checksums, closes/reopens, then removes the original temporary directory and
reopens a relocated copy in a separate process. CLI subprocess checks exercise
the actual `orr_editor` binary's early rejection path.

GPU acceptance uses `egui_kittest`'s `.wgpu().build_eframe` and `Harness::render`.
It reads the full composed output, including the sprite overlay, instead of
reading only the core viewport texture. The harness disables its test-only
forced bilinear filter so the production `NEAREST` sprite sampler is honored;
opaque texels are compared directly to the verified installed atlas. It checks real atlas colors, independent
actor frame pixels, changed walking pixels, camera-follow identity, identical
Stop/reopen pixels, and unchanged simulation/document checksums. Captures are:

- `project-01-initial-autoload.png`
- `project-02-independent-idle-frames.png`
- `project-03-walk-follow.png`
- `project-04-stopped.png`
- `project-05-relocated-reopen.png`

`ORR_REQUIRE_GPU=1` turns missing GPU support into failure. Use a fresh capture
directory per evidence run so earlier failed captures and logs remain available.
For stronger Linux relocation evidence, add `ORR_REQUIRE_PROJECT_ISOLATION=1`
to both commands. This requires `bwrap`: the subprocess mount namespace hides
the entire source repository (including original sample assets and the
checked-in project), retains only the relocated project outside it, and starts
from an empty working directory. The parent process never changes its cwd or
renames repository directories, so parallel tests remain isolated. Without this
flag, relocation still runs in an empty-cwd subprocess, but does not claim that
absolute source-repository paths were physically inaccessible.

## Optional saved UI preview

`--features project-ui` explicitly opts this editor into admission of the closed
`arena-korean-v1` preset and locked font reference in `entry.ui`. Merely
unifying the sample crate's `game-ui` feature does not enable editor support.
The inspector provides a read-only preset/font preview with the admitted bytes.
It neither authors UI nor overlays game widgets in the editor viewport. Existing
scene/sprite edits and saves retain project metadata, and editor Restart continues
to reload the saved scene. See [saved project UI](authored-project-ui.md).
