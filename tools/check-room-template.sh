#!/usr/bin/env bash
# Mandatory generated Room acceptance. Historical source results are not union results.
set -euo pipefail
mode=${1:?usage: check-room-template.sh contracts|gpu}
case "$mode" in contracts|gpu) ;; *) echo "Unknown mode: $mode" >&2; exit 2 ;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
test "$(uname -s)" = Linux
test "$(uname -m)" = x86_64
export CARGO_TERM_COLOR=never
base=${ORR_ROOM_TEMPLATE_EVIDENCE:-/tmp/orr-room-template-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
git diff --exit-code HEAD -- . >/dev/null
cp tools/room-template-inventories.json "$evidence/expected-inventories.json"
checker=tools/room_template_gate.py
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
    run_test creator-default normal cargo test --release --locked -j2 -p orr_sample --features project-create --lib project_create
    run_test creator-room normal cargo test --release --locked -j2 -p orr_sample --features project-create,room-project --lib project_create
    run_test creator-joint normal cargo test --release --locked -j2 -p orr_sample --features project-create,room-project,collect-ui --lib project_create
    run_test sample-disabled normal cargo test --release --locked -j2 -p orr_sample --features project-create --test new_room_project
    run_test sample-room normal cargo test --release --locked -j2 -p orr_sample --features project-create,room-project --test new_room_project
    run_test arena-room normal cargo test --release --locked -j2 -p orr_sample --features project-create,room-project --test new_arena_cli
    run_test arena-joint normal cargo test --release --locked -j2 -p orr_sample --features project-create,room-project,collect-ui --test new_arena_cli
    run_test editor-room normal cargo test --release --locked -j2 -p orr_editor --features project-create,room-project --test new_room_project
    run_command sample-clippy cargo clippy --release --locked -j2 -p orr_sample --features project-create,project-export,room-project --all-targets -- -D warnings
    run_command editor-clippy cargo clippy --release --locked -j2 -p orr_editor --features project-create,room-project --all-targets -- -D warnings
    run_command joint-clippy cargo clippy --release --locked -j2 -p orr_sample --features project-create,room-project,collect-ui --all-targets -- -D warnings
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
    timeout --kill-after=30s 30m cargo build --release --locked -j2 -p orr_sample --features project-create,project-export,room-project --bin orr_new_arena --bin room_escape --bin orr_export_room --message-format=json > "$evidence/production-tools.jsonl" 2> "$evidence/production-tools.stderr"
    python3 "$checker" pin "$evidence"
    python3 "$checker" verify-tools "$evidence"
    sha256sum --check "$evidence/tools.sha256"
    export ORR_ROOM_CREATOR="$evidence/tools/orr_new_arena"
    export ORR_ROOM_RUNTIME="$evidence/tools/room_escape"
    export ORR_ROOM_EXPORTER="$evidence/tools/orr_export_room"
    export ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/export"
    export ORR_ROOM_CAPTURE_DIR="$evidence/captures/editor"
    mkdir -p "$ORR_ROOM_EXPORT_CAPTURE_DIR" "$ORR_ROOM_CAPTURE_DIR"
    # The sample test verifies hidden source/original tools inside bwrap, isolated
    # HOME/XDG and empty cwd, read-only project/export, actual GPU and exact parity.
    run_test source-hidden-gpu ignored cargo test --release --locked -j2 -p orr_sample --features project-create,project-export,room-project --test new_room_project room::generated_room_real_export_source_hidden_gpu_workflow
    run_test editor-gpu ignored cargo test --release --locked -j2 -p orr_editor --features project-create,room-project --test new_room_project generated_room_gpu_uv_models
    python3 "$checker" captures "$evidence"
    python3 "$checker" verify-tools "$evidence"
    sha256sum --check "$evidence/tools.sha256"
    sha256sum "$evidence/captures/editor/"*.png "$evidence/export/captures/"*.png "$evidence/export/orr.export.json" > "$evidence/captures.sha256"
    ;;
esac
