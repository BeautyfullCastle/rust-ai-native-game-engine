#!/usr/bin/env bash
# Required positive-count RoomEscapeV1 acceptance. No GPU or isolation skip passes.
set -euo pipefail
mode=${1:?usage: check-room-project.sh contracts|gpu}
case "$mode" in
  contracts|gpu) ;;
  *) echo "Unknown mode: $mode" >&2; exit 2 ;;
esac

cd "$(dirname "${BASH_SOURCE[0]}")/.."
evidence_base=${ORR_ROOM_EVIDENCE:-target/room-project-evidence}
mkdir -p "$evidence_base"
# Keep each invocation separate, so cached captures cannot masquerade as new proof.
evidence=$(mktemp -d "$evidence_base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD > "$evidence/source-revision.txt"

run_command() {
  local name=$1; shift
  timeout --kill-after=30s 30m "$@" 2>&1 | tee "$evidence/$name.log"
}
run_test() {
  local name=$1 expected=$2 ignored=$3; shift 3
  run_command "$name" "$@"
  if ! grep -Eq "^test result: ok\\. ${expected} passed; 0 failed; ${ignored} ignored;" "$evidence/$name.log"; then
    echo "Expected $expected passing and $ignored ignored tests in $name; zero tests or skips are not acceptance" >&2
    exit 1
  fi
}

case "$mode" in
  contracts)
    run_test room-simulation 10 0 cargo test --release --locked -p orr_games --features room-escape --lib room_escape_game::tests::
    run_test runtime-input 3 0 cargo test --release --locked -p orr_sample --features room-project --lib room_app::tests::
    run_test room-view-contracts 2 0 cargo test --release --locked -p orr_sample --features room-project --lib room_view::tests::
    # Enable export here too, so invalid model admission must fail before its smoke run.
    run_test project-admission 10 1 cargo test --release --locked -p orr_sample --features room-project,project-export --test room_project
    run_test editor-workflow 4 2 cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow
    run_test camera-schema 9 0 cargo test --release --locked -p orr_sample --features room-project --lib room_camera::tests::
    # Build the schema tests with editor's arbitrary_precision dependency too.
    run_test camera-schema-unified 9 0 cargo test --release --locked -p orr_sample -p orr_editor --features orr_sample/room-project,orr_editor/room-project --lib room_camera::tests::
    run_test camera-editor-panel 9 0 cargo test --release --locked -p orr_editor --features room-project --lib room_camera_panel::tests::
    run_test generated-room-camera 3 1 cargo test --release --locked -p orr_sample --features room-project,project-create,project-export --test new_room_project
    run_test editor-model-admission 3 0 cargo test --release --locked -p orr_editor --features room-project --test room_model_admission
    # Explicit Room HUD consumers; cross-profile/default routes stay closed.
    run_test room-ui-schema 2 0 cargo test --release --locked -p orr_sample --features room-ui --lib authored_ui::room_profile_tests::
    run_test room-ui-widgets 2 0 cargo test --release --locked -p orr_sample --features room-ui --lib collect_ui::room_tests::
    run_test room-ui-input-scale 1 0 cargo test --release --locked -p orr_sample --features room-ui --lib room_app::room_pointer_capture_tests::
    run_test room-ui-native-app 1 0 cargo test --release --locked -p orr_sample --features room-ui,project-create --lib room_app::room_ui_app_tests::
    run_test room-ui-admission 3 0 cargo test --release --locked -p orr_sample --features room-ui,project-create --test room_ui_project
    run_test room-ui-editor-widgets 1 0 cargo test --release --locked -p orr_editor --features room-ui,project-create --lib collect_ui_panel::room_panel_tests::
    run_test room-ui-editor-source-lifecycle 2 0 cargo test --release --locked -p orr_editor --features room-ui,project-create --test room_ui_lifecycle
    # Exercise unified optional features without changing any default feature or lint.
    run_command optional-features-clippy cargo clippy --release --locked \
      -p orr_games -p orr_package -p orr_model_bindings -p orr_sample -p orr_remote -p orr_editor \
      --features orr_games/room-escape,orr_sample/room-project,orr_sample/project-export,orr_editor/room-project,orr_editor/room-ui,orr_editor/project-create,orr_sample/room-ui,orr_editor/animated-models,orr_editor/terrain-physics,orr_editor/navigation,orr_editor/irradiance-probes,orr_editor/collect-ui,orr_editor/linked-prefabs,orr_editor/image-reimport \
      --all-targets -- -D warnings
    ;;
  gpu)
    command -v bwrap
    run_command namespace-preflight bwrap --ro-bind / / -- /bin/true
    run_command production-tools cargo build --release --locked -p orr_sample --features room-project,project-export,project-create --bin room_escape --bin orr_export_room --bin orr_new_arena
    target=${CARGO_TARGET_DIR:-target}
    mkdir -p "$evidence/built-tools" "$evidence/captures/editor"
    cp "$target/release/room_escape" "$evidence/built-tools/room_escape"
    cp "$target/release/orr_export_room" "$evidence/built-tools/orr_export_room"
    cp "$target/release/orr_new_arena" "$evidence/built-tools/orr_new_arena"
    sha256sum "$evidence/built-tools/room_escape" "$evidence/built-tools/orr_export_room" "$evidence/built-tools/orr_new_arena" | tee "$evidence/built-tools.sha256"
    ORR_REQUIRE_GPU=1 \
      run_test sample-gpu-readback 1 0 cargo test --release --locked -p orr_sample --features room-project,project-export --test room_project owned_real_model_offscreen_readback_is_nonblank_and_read_only -- --ignored --exact --nocapture
    ORR_ROOM_CAPTURE_DIR="$evidence/captures/editor" \
    ORR_REQUIRE_GPU=1 \
      run_test editor-gpu-viewport 1 0 cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow production_room_viewport_native_texture_has_imported_uv_model_contribution -- --ignored --exact --nocapture
    # The test copies these exact tools out of the workspace, hides source and
    # original executables, then runs the real exporter and relocated launcher.
    ORR_ROOM_RUNTIME="$evidence/built-tools/room_escape" \
    ORR_ROOM_EXPORTER="$evidence/built-tools/orr_export_room" \
    ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/export" \
    ORR_REQUIRE_GPU=1 \
    ORR_REQUIRE_PROJECT_ISOLATION=1 \
      run_test production-source-hidden-export 1 0 cargo test --release --locked -p orr_sample --features room-project,project-export --test room_export room_real_export_source_hidden_gpu_workflow -- --ignored --exact --nocapture
    ORR_ROOM_CAPTURE_DIR="$evidence/captures/editor" ORR_REQUIRE_GPU=1 \
      run_test camera-editor-gpu 1 0 cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow authored_camera_viewport_projection_picking_and_invalid_preservation -- --ignored --exact --nocapture
    ORR_ROOM_CREATOR="$evidence/built-tools/orr_new_arena" \
    ORR_ROOM_RUNTIME="$evidence/built-tools/room_escape" \
    ORR_ROOM_EXPORTER="$evidence/built-tools/orr_export_room" \
    ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/camera-export" \
    ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1 \
      run_test camera-generated-source-hidden-export 1 0 cargo test --release --locked -p orr_sample --features room-project,project-create,project-export --test new_room_project room::generated_room_real_export_source_hidden_gpu_workflow -- --ignored --exact --nocapture
    # Build a separate explicit UI-capable consumer; do not overwrite the
    # original no-UI production tool proof or remove any earlier gate.
    run_command room-ui-production-tools cargo build --release --locked -p orr_sample --features room-ui,project-export,project-create --bin room_escape --bin orr_export_room --bin orr_new_arena
    mkdir -p "$evidence/ui-built-tools"
    cp "$target/release/room_escape" "$evidence/ui-built-tools/room_escape"
    cp "$target/release/orr_export_room" "$evidence/ui-built-tools/orr_export_room"
    cp "$target/release/orr_new_arena" "$evidence/ui-built-tools/orr_new_arena"
    sha256sum "$evidence/ui-built-tools/"* | tee "$evidence/ui-built-tools.sha256"
    ORR_ROOM_UI_CAPTURE_DIR="$evidence/captures/room-ui" ORR_REQUIRE_GPU=1 \
      run_test room-ui-composited-gpu 1 0 cargo test --release --locked -p orr_sample --features room-ui,project-create --lib room_app::room_ui_gpu_tests::room_ui_composited_title_play_key_win_and_projection_preserve_frame -- --ignored --exact --nocapture
    ORR_ROOM_CREATOR="$evidence/ui-built-tools/orr_new_arena" \
    ORR_ROOM_RUNTIME="$evidence/ui-built-tools/room_escape" \
    ORR_ROOM_EXPORTER="$evidence/ui-built-tools/orr_export_room" \
    ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/ui-export" \
    ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1 \
      run_test room-ui-source-hidden-export 1 0 cargo test --release --locked -p orr_sample --features room-ui,project-create,project-export --test new_room_project room::generated_room_ui_real_export_source_hidden_gpu_workflow -- --ignored --exact --nocapture
    for size in 1024x768 480x800; do
      for state in title playing key won; do test -s "$evidence/captures/room-ui/$state-$size.png"; done
    done
    for capture in initial-source initial-export moved-source moved-export interacted-source interacted-export no-models; do
      test -s "$evidence/ui-export/captures/$capture.png"
    done
    test -s "$evidence/ui-export/orr.export.json"
    test -s "$evidence/ui-export/authored-project/room.ui.json"
    sha256sum "$evidence/captures/room-ui/"*.png "$evidence/ui-export/captures/"*.png "$evidence/ui-export/orr.export.json" "$evidence/ui-export/authored-project/room.ui.json" | tee "$evidence/room-ui-captures.sha256"
    for capture in authored-camera-landscape authored-camera-invalid-retained authored-camera-manual authored-camera-reset authored-camera-projection-error authored-camera-portrait authored-camera-resize-return; do
      test -s "$evidence/captures/editor/$capture.png"
    done
    # Require persisted proof as well as test counts; missing captures fail CI.
    for capture in room-editor-bound room-editor-models-offscreen; do
      test -s "$evidence/captures/editor/$capture.png"
    done
    for capture in initial-source initial-export moved-source moved-export interacted-source interacted-export no-models; do
      test -s "$evidence/export/captures/$capture.png"
    done
    test -s "$evidence/export/orr.export.json"
    for capture in initial-source initial-export moved-source moved-export interacted-source interacted-export no-models; do
      test -s "$evidence/camera-export/captures/$capture.png"
    done
    test -s "$evidence/camera-export/orr.export.json"
    test -s "$evidence/camera-export/authored-project/room.camera.json"
    sha256sum "$evidence/captures/editor/"*.png "$evidence/export/captures/"*.png "$evidence/export/orr.export.json" "$evidence/camera-export/captures/"*.png "$evidence/camera-export/orr.export.json" "$evidence/camera-export/authored-project/room.camera.json" | tee "$evidence/captures.sha256"
    ;;
esac
