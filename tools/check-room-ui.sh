#!/usr/bin/env bash
# Mandatory authored Room UI acceptance. Historical source results are not union results.
set -euo pipefail
mode=${1:?usage: check-room-ui.sh contracts|gpu}
case "$mode" in contracts|gpu) ;; *) echo "Unknown mode: $mode" >&2; exit 2 ;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
test "$(uname -s)" = Linux
test "$(uname -m)" = x86_64
export CARGO_TERM_COLOR=never
base=${ORR_ROOM_UI_EVIDENCE:-/tmp/orr-room-ui-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
git diff --exit-code HEAD -- . >/dev/null
cp tools/room-ui-inventories.json "$evidence/expected-inventories.json"
checker=tools/room_ui_gate.py
run_command() {
  local lane=$1; shift
  timeout --kill-after=30s 30m "$@" 2>&1 | tee "$evidence/$lane.log"
}
run_test() {
  local lane=$1 execution=$2; shift 2
  local options=(--test-threads=1 --nocapture)
  if [[ "$execution" == ignored ]]; then options+=(--ignored --exact); fi
  if [[ "$execution" == exact ]]; then options+=(--exact); fi
  run_command "$lane-list" "$@" -- --list "${options[@]}"
  python3 "$checker" log "$lane" list "$evidence/$lane-list.log"
  run_command "$lane-ignored" "$@" -- --ignored --list --test-threads=1
  python3 "$checker" log "$lane" ignored "$evidence/$lane-ignored.log"
  run_command "$lane" "$@" -- "${options[@]}"
  python3 "$checker" log "$lane" result "$evidence/$lane.log"
}
case "$mode" in
  contracts)
    run_command checker-fixtures python3 tools/test_room_ui_gate.py
    run_test room-ui-schema normal cargo test --release --locked -j2 -p orr_sample --features room-ui --lib authored_ui::room_profile_tests::
    run_test room-ui-widgets normal cargo test --release --locked -j2 -p orr_sample --features room-ui --lib collect_ui::room_tests::
    run_test room-ui-input-scale normal cargo test --release --locked -j2 -p orr_sample --features room-ui --lib room_app::room_pointer_capture_tests::
    run_test room-ui-native-app normal cargo test --release --locked -j2 -p orr_sample --features room-ui,project-create --lib room_app::room_ui_app_tests::
    run_test room-ui-admission normal cargo test --release --locked -j2 -p orr_sample --features room-ui,project-create --test room_ui_project
    run_test room-ui-editor-widgets normal cargo test --release --locked -j2 -p orr_editor --features room-ui,project-create --lib collect_ui_panel::room_panel_tests::
    run_test room-ui-save-transition-producer exact cargo test --release --locked -j2 -p orr_remote --test activity scene_save_path_transition_flag_is_exact_in_list_and_watch
    run_test room-ui-editor-source-lifecycle normal cargo test --release --locked -j2 -p orr_editor --features room-ui,project-create --test room_ui_lifecycle
    run_test collect-schema-compatible normal cargo test --release --locked -j2 -p orr_sample --features collect-ui,room-ui --lib authored_ui::tests::
    run_test collect-widgets-compatible normal cargo test --release --locked -j2 -p orr_sample --features collect-ui,room-ui --lib collect_ui::tests::
    run_test collect-editor-compatible normal cargo test --release --locked -j2 -p orr_editor --features collect-ui,room-ui --lib collect_ui_panel::tests::
    run_command optional-features-clippy cargo clippy --release --locked -j2 \
      -p orr_games -p orr_package -p orr_model_bindings -p orr_sample -p orr_remote -p orr_editor \
      --features orr_games/room-escape,orr_sample/room-ui,orr_sample/project-export,orr_editor/room-ui,orr_editor/project-create,orr_editor/animated-models,orr_editor/terrain-physics,orr_editor/navigation,orr_editor/irradiance-probes,orr_editor/collect-ui,orr_editor/linked-prefabs,orr_editor/image-reimport \
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
root=pathlib.Path(sys.argv[1]); hidden=pathlib.Path.cwd().resolve()
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
    timeout --kill-after=30s 30m cargo build --release --locked -j2 -p orr_sample --features project-create,project-export,room-ui --bin orr_new_arena --bin room_escape --bin orr_export_room --message-format=json > "$evidence/production-tools.jsonl" 2> "$evidence/production-tools.stderr"
    python3 "$checker" pin "$evidence"
    python3 "$checker" verify-tools "$evidence"
    sha256sum --check "$evidence/tools.sha256"
    export ORR_ROOM_CREATOR="$evidence/tools/orr_new_arena"
    export ORR_ROOM_RUNTIME="$evidence/tools/room_escape"
    export ORR_ROOM_EXPORTER="$evidence/tools/orr_export_room"
    export ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/export"
    export ORR_ROOM_UI_CAPTURE_DIR="$evidence/captures/room-ui"
    mkdir -p "$ORR_ROOM_EXPORT_CAPTURE_DIR" "$ORR_ROOM_UI_CAPTURE_DIR"
    run_test room-ui-composited-gpu ignored cargo test --release --locked -j2 -p orr_sample --features room-ui,project-create --lib room_app::room_ui_gpu_tests::room_ui_composited_title_play_key_win_and_projection_preserve_frame
    run_test room-ui-source-hidden-export ignored cargo test --release --locked -j2 -p orr_sample --features room-ui,project-create,project-export --test new_room_project room::generated_room_ui_real_export_source_hidden_gpu_workflow
    python3 "$checker" captures "$evidence"
    python3 "$checker" verify-tools "$evidence"
    sha256sum --check "$evidence/tools.sha256"
    sha256sum "$evidence/captures/room-ui/"*.png "$evidence/export/captures/"*.png "$evidence/export/orr.export.json" "$evidence/export/authored-project/room.ui.json" > "$evidence/captures.sha256"
    ;;
esac
