#!/usr/bin/env bash
# Mandatory Room and authored-camera acceptance. Historical source results are not union results.
set -euo pipefail
mode=${1:?usage: check-room-project.sh contracts|gpu}
case "$mode" in contracts|gpu) ;; *) echo "Unknown mode: $mode" >&2; exit 2 ;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
test "$(uname -s)" = Linux
test "$(uname -m)" = x86_64
export CARGO_TERM_COLOR=never
base=${ORR_ROOM_EVIDENCE:-/tmp/orr-room-project-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
git diff --exit-code HEAD -- . >/dev/null
cp tools/room-project-inventories.json "$evidence/expected-inventories.json"
checker=tools/room_project_gate.py
run_command() {
  local lane=$1; shift
  timeout --kill-after=30s 30m "$@" 2>&1 | tee "$evidence/$lane.log"
}
run_test() {
  local lane=$1 execution=$2; shift 2
  local options=(--test-threads=1 --nocapture)
  if [[ "$execution" == ignored ]]; then options+=(--ignored --exact); fi
  run_command "$lane-list" "$@" -- --list "${options[@]}"
  python3 "$checker" log "$lane" list "$evidence/$lane-list.log"
  run_command "$lane-ignored" "$@" -- --ignored --list --test-threads=1
  python3 "$checker" log "$lane" ignored "$evidence/$lane-ignored.log"
  run_command "$lane" "$@" -- "${options[@]}"
  python3 "$checker" log "$lane" result "$evidence/$lane.log"
}
case "$mode" in
  contracts)
    run_command checker-fixtures python3 tools/test_room_project_gate.py
    run_command generated-checker-fixtures python3 tools/test_room_template_gate.py
    run_test room-simulation normal cargo test --release --locked -p orr_games --features room-escape --lib room_escape_game::tests::
    run_test runtime-input normal cargo test --release --locked -p orr_sample --features room-project --lib room_app::tests::
    run_test room-view-contracts normal cargo test --release --locked -p orr_sample --features room-project --lib room_view::tests::
    run_test project-admission normal cargo test --release --locked -p orr_sample --features room-project,project-export --test room_project
    run_test editor-workflow normal cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow
    run_test camera-schema normal cargo test --release --locked -p orr_sample --features room-project --lib room_camera::tests::
    run_test camera-schema-unified normal cargo test --release --locked -p orr_sample -p orr_editor --features orr_sample/room-project,orr_editor/room-project --lib room_camera::tests::
    run_test camera-editor-panel normal cargo test --release --locked -p orr_editor --features room-project --lib room_camera_panel::tests::
    run_test generated-room-camera normal cargo test --release --locked -p orr_sample --features room-project,project-create,project-export --test new_room_project
    run_test editor-model-admission normal cargo test --release --locked -p orr_editor --features room-project --test room_model_admission
    # Exercise unified optional features without changing any default feature or lint.
    run_command optional-features-clippy cargo clippy --release --locked \
      -p orr_games -p orr_package -p orr_model_bindings -p orr_sample -p orr_remote -p orr_editor \
      --features orr_games/room-escape,orr_sample/room-project,orr_sample/project-export,orr_editor/room-project,orr_editor/animated-models,orr_editor/terrain-physics,orr_editor/navigation,orr_editor/irradiance-probes,orr_editor/collect-ui,orr_editor/linked-prefabs,orr_editor/image-reimport \
      --all-targets -- -D warnings
    ;;
  gpu)
    test "$(id -u)" -ne 0
    rustc -vV | tee "$evidence/rustc.txt"
    grep -Fx 'host: x86_64-unknown-linux-gnu' "$evidence/rustc.txt"
    command -v bwrap
    run_command namespace-preflight bwrap --ro-bind / / -- /bin/true
    python3 - "$evidence" <<'PYPATHS'
import pathlib,sys
root=pathlib.Path(sys.argv[1]); hidden=pathlib.Path.cwd().resolve().parent
assert root.is_absolute() and root == root.resolve()
assert root.is_relative_to(pathlib.Path('/tmp').resolve())
assert not root.is_relative_to(hidden), 'tools/evidence must survive full source hiding'
PYPATHS
    mkdir "$evidence/temporary" "$evidence/xdg-runtime"
    chmod 700 "$evidence/xdg-runtime"
    export TMPDIR="$evidence/temporary" XDG_RUNTIME_DIR="$evidence/xdg-runtime"
    export ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1
    export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
    # Pin the exact compiler-JSON production artifacts before any other Cargo command.
    timeout --kill-after=30s 30m cargo build --release --locked -j2 -p orr_sample --features project-create,project-export,room-project --bin orr_new_arena --bin room_escape --bin orr_export_room --message-format=json > "$evidence/production-tools.jsonl" 2> "$evidence/production-tools.stderr"
    python3 "$checker" pin "$evidence"
    python3 "$checker" verify-tools "$evidence"
    sha256sum --check "$evidence/tools.sha256"
    mkdir -p "$evidence/captures/legacy-editor" "$evidence/captures/camera-editor" "$evidence/export" "$evidence/camera-export"
    ORR_REQUIRE_GPU="1" \
      run_test sample-gpu-readback ignored cargo test --release --locked -p orr_sample --features room-project,project-export --test room_project owned_real_model_offscreen_readback_is_nonblank_and_read_only
    ORR_REQUIRE_GPU="1" ORR_ROOM_CAPTURE_DIR="$evidence/captures/legacy-editor" \
      run_test editor-gpu-viewport ignored cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow production_room_viewport_native_texture_has_imported_uv_model_contribution
    ORR_REQUIRE_GPU="1" ORR_REQUIRE_PROJECT_ISOLATION="1" ORR_ROOM_RUNTIME="$evidence/tools/room_escape" ORR_ROOM_EXPORTER="$evidence/tools/orr_export_room" ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/export" \
      run_test production-source-hidden-export ignored cargo test --release --locked -p orr_sample --features room-project,project-export --test room_export room_real_export_source_hidden_gpu_workflow
    ORR_REQUIRE_GPU="1" ORR_ROOM_CAPTURE_DIR="$evidence/captures/camera-editor" \
      run_test camera-editor-gpu ignored cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow authored_camera_viewport_projection_picking_and_invalid_preservation
    ORR_REQUIRE_GPU="1" ORR_REQUIRE_PROJECT_ISOLATION="1" ORR_ROOM_RUNTIME="$evidence/tools/room_escape" ORR_ROOM_EXPORTER="$evidence/tools/orr_export_room" ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/camera-export" ORR_ROOM_CREATOR="$evidence/tools/orr_new_arena" \
      run_test camera-generated-source-hidden-export ignored cargo test --release --locked -p orr_sample --features room-project,project-create,project-export --test new_room_project room::generated_room_real_export_source_hidden_gpu_workflow
    python3 "$checker" captures "$evidence"
    python3 "$checker" verify-tools "$evidence"
    sha256sum --check "$evidence/tools.sha256"
    find "$evidence/captures" "$evidence/export" "$evidence/camera-export" -type f -print0 | sort -z | xargs -0 sha256sum > "$evidence/captures.sha256"
    ;;
esac
