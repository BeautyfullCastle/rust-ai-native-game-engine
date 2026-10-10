# Authored RoomEscapeV1 projects

This is the first **closed flat-room game profile**, not a general 3D character
controller or completion of the full room-escape roadmap in #94.

## Optional layers

- `orr_games/room-escape`: fixed-point simulation only, no GPU, files or clocks
- `orr_sample/room-project`: strict scene/model admission, immutable assets,
  standalone `room_escape` player and shared 3D presentation
- `orr_editor/room-project`: managed keyboard control, the existing 3D scene/model
  editing widgets and `--room-project DIR`
- `orr_sample/project-export,room-project`: `orr_export_room`, Linux x86-64 only
- `orr_model_bindings`: GPU-free existing model descriptors and verified static/
  optional animated loaders. Editor assignment/history/save remains in editor

Default engine/editor dependencies do not enable this profile. Cargo unification
of animation, sprites or UI does not grant this consuming route those features.
The existing `orr_package` lock remains the content authority; no hooks, remote
registry, automatic build/download or credentials are introduced.

## Game contract

One player, one key and one locked exit; up to 64 axis-aligned wall boxes and one
optional floor. The player is a radius-0.4 sphere at Y=0.5. World X/Z movement is
bounded to [-16,16] and each X-then-Z segment is bounded **before** a sphere cast.
Requested diagonal movement is normalized. Wall-only query layers exclude the
player, floor and interaction markers; a skin gap stops the actor before contact.
Walls cover the player sphere's vertical band. Initial overlap/contact is rejected.

This query-driven profile has no gravity, jumping, stairs, slopes, moving bodies,
pushing, physics solver stepping or general depenetration. Floor and key/exit are
not physics triggers. An explicit fresh Interact press uses bounded fixed-point
proximity and wall line-of-sight. Collecting the key and winning at the exit need
separate presses. Won state freezes gameplay; marker entities remain stable.

All key, win and previous-button state belongs to the Frame. Scene-index GUIDs
remain presentation identities, never recycled bare entity indices. Standalone
Restart reconstructs the retained admitted initial Frame and clears held input.
Editor Restart room stops the play copy and starts a new play copy of its current
unchanged document; it does not restart the host or reread the scene from disk.

## Project entry

A schema-2 `orr.project.json` entry uses `game: room-escape-v1`, an explicit scene
path and `models: room.models.json`. The first profile requires the model sidecar
at the project root. Its existing model-document v1/v2 `project` hint must be `.`
and its `scene` hint must exactly equal the entry scene. Sprite/UI/progress entry
fields are rejected, including when those capabilities are compiled elsewhere.

The scene contains only the registered Body, Collider and RoomActor components,
canonical PhysicsState and an initial empty RoomRun singleton. Admission checks
complete numeric/canonical fields before query arithmetic. Existing reflected
scene baking initializes hidden physics storage; game setup never replaces an
authored scene on open. Unknown roles, dynamic bodies, wrong component sets,
linked prefabs and invalid initial state are rejected.

The active package lock is fully verified before and after preparing all assets.
Static bindings use the existing imported/cooked model format, UVs, material data,
package/source hashes and GUID keys. There are at most eight assets and 68 model
bindings, with a 64 MiB aggregate decoded-content budget and the existing renderer
work limits. Initial and reachable player-composed placements are CPU-admitted
before startup/export smoke. Model transforms never change collision geometry.
Scene/model saves and undo remain separate transactions.

## Run and export

```sh
cargo run --release -p orr_editor --features room-project -- --room-project DIR
cargo run --release -p orr_sample --features room-project --bin room_escape -- --project DIR
cargo run --release -p orr_sample --features room-project --bin room_escape -- \
  --project DIR --headless --ticks 180 --hold right --capture NEW.png
```

Window controls: WASD/arrows, E interact, R restart, Escape quit; mouse controls
only change the camera. Editor uses Take control and E, with a Restart room button.
Focus loss/synthetic events neutralize held controls. Headless capture requires a
working GPU adapter and creates a new PNG without replacing an existing file.

```sh
cargo run --release -p orr_sample --features room-project,project-export \
  --bin orr_export_room -- --project DIR --runtime BUILT_ROOM_BINARY \
  --runtime-sha256 EXPECTED_SHA256 --output NEW_DIR --trusted-runtime
```

The supplied prebuilt binary must already be trusted. Its SHA and bounded zero-tick
smoke do not establish trust. Export reuses the existing captured-byte closure,
no-replace staging and failure-preservation protocol. It includes exact entry,
model sidecar, full active package closure/notices and runtime; no editor/source
cache copy. `run-room-escape` starts it from any working directory. The bundle is
not an installer, a system-library bundle or a separate network/game-code identity.

## Verification and remaining work

Tests separate sim/admission, production editor controls, offscreen GPU and actual
copied-binary/export isolation. Opt-in GPU tests require ORR_REQUIRE_GPU=1 and must
be explicitly executed; an ignored default test is not success. Exact-head required
CI and shared development integration are separate from focused local passes.

Full #94 room acceptance still includes a skinned character, material overrides,
authored HUD/audio, checkpoints/door restoration, wider lighting/probe/post settings
and richer 3D authoring. This profile does not claim those. It is separate from the
original #2 denominator of 22 follow-ups.

Plan: https://github.com/BeautyfullCastle/rust-ai-native-game-engine/issues/104#issuecomment-6051552622
