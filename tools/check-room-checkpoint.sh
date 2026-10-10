#!/usr/bin/env bash
# Positive-count checkpoint gates; GPU/export/syscall failures never become skips.
set -euo pipefail
mode=${1:?usage: check-room-checkpoint.sh contracts|gpu-export|syscall}
case "$mode" in contracts|gpu-export|syscall) ;; *) exit 2;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
test "$(uname -s)" = Linux
test "$(uname -m)" = x86_64
export CARGO_TERM_COLOR=never
base=${ORR_CHECKPOINT_EVIDENCE:-/tmp/orr-room-checkpoint-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
git diff --exit-code HEAD -- . >/dev/null
cp tools/room-checkpoint-inventories.json "$evidence/expected-inventories.json"
checker=tools/room_checkpoint_gate.py
run() { local name=$1; shift; timeout --kill-after=30s 40m "$@" 2>&1 | tee "$evidence/$name.log"; }
tests() {
 local name=$1 passed=$2 ignored=$3; shift 3
 local command=("$@") options=(--test-threads=1 --nocapture) exact=()
 if [[ $1 == cargo ]]; then command+=(--); fi
 if [[ $name == metadata-only-window-rejected || $name == app-gpu ]]; then exact+=(--exact); fi
 if [[ $name == app-gpu ]]; then options+=(--ignored); fi
 run "$name-list" "${command[@]}" "${exact[@]}" "${options[@]}" --list
 python3 "$checker" log "$name" list "$evidence/$name-list.log" "$passed" "$ignored"
 run "$name-ignored" "${command[@]}" "${exact[@]}" --ignored --list
 python3 "$checker" log "$name" ignored "$evidence/$name-ignored.log" "$passed" "$ignored"
 run "$name" "${command[@]}" "${exact[@]}" "${options[@]}"
 python3 "$checker" log "$name" result "$evidence/$name.log" "$passed" "$ignored"
}
features=room-checkpoint,project-create,project-export
if [[ $mode == contracts ]]; then
 run checker-fixtures python3 tools/test_room_checkpoint_gate.py
 run syscall-checker-fixtures python3 tools/test_room_checkpoint_syscall.py
 unset ORR_PACKAGE_FIFO_TEST_ROOT
 tests room-goldens 10 0 cargo test --release --locked -p orr_games --features room-escape --lib room_escape_game::tests::
 tests package-closure 36 0 cargo test --release --locked -p orr_package --lib
 tests default-off 2 0 cargo test --release --locked -p orr_sample --features room-project --lib room_project::checkpoint_tests::
 tests metadata-only-window-rejected 1 0 cargo test --release --locked -p orr_sample --features room-ui,project-create --lib room_project::checkpoint_tests::metadata_only_admission_preserves_legacy_scene_and_owns_checkpoint
 tests store 17 0 cargo test --release --locked -p orr_sample --features "$features" --lib room_checkpoint_store::tests::
 tests host-processes 8 2 cargo test --release --locked -p orr_sample --features "$features" --lib room_app::room_checkpoint_acceptance_tests::
 tests semantic-rules 1 0 cargo test --release --locked -p orr_sample --features "$features" --lib room_checkpoint::tests::
 tests admitted-profile 3 0 cargo test --release --locked -p orr_sample --features "$features" --lib room_project::checkpoint_tests::
 tests creator 1 0 cargo test --release --locked -p orr_sample --features "$features" --lib project_create::checkpoint_tests::
 tests editor-widgets 6 0 cargo test --release --locked -p orr_editor --features room-checkpoint,project-create --lib room_checkpoint_panel::tests::
 run strict-clippy cargo clippy --release --locked -p orr_sample -p orr_editor -p orr_package --features orr_sample/room-checkpoint,orr_sample/project-create,orr_sample/project-export,orr_editor/room-checkpoint,orr_editor/project-create --all-targets -- -D warnings
 # Compile the checkpoint path together with the retained optional editor features.
 run wide-optional-clippy cargo clippy --release --locked -j2 \
  -p orr_games -p orr_package -p orr_model_bindings -p orr_sample -p orr_remote -p orr_editor \
  --features orr_games/room-escape,orr_sample/room-checkpoint,orr_sample/project-export,orr_editor/room-checkpoint,orr_editor/project-create,orr_editor/animated-models,orr_editor/terrain-physics,orr_editor/navigation,orr_editor/irradiance-probes,orr_editor/collect-ui,orr_editor/linked-prefabs,orr_editor/image-reimport \
  --all-targets -- -D warnings
 exit 0
fi
test "$(id -u)" -ne 0
python3 - "$evidence" <<'PYPATHS'
import pathlib, sys
root = pathlib.Path(sys.argv[1]); hidden = pathlib.Path.cwd().resolve().parent
assert root.is_absolute() and root == root.resolve()
assert root.is_relative_to(pathlib.Path('/tmp').resolve()) and not root.is_relative_to(hidden)
PYPATHS
export ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan}
command -v bwrap
run namespace-preflight bwrap --ro-bind / / -- /bin/true
if [[ $mode == syscall ]]; then
 command -v strace
 run syscall-preflight strace -qq -o "$evidence/ptrace-preflight.trace" /bin/true
fi
# Cargo JSON selects executables from this invocation, never stale target paths.
run production-build cargo build --release --locked -p orr_sample --features "$features" --bin room_escape --bin orr_export_room --bin orr_new_arena --message-format=json
run app-harness-build cargo test --release --locked -p orr_sample --features "$features" --lib --no-run --message-format=json
mkdir -p "$evidence/tools"
python3 - "$evidence" <<'PY'
import json, pathlib, shutil, os
root=pathlib.Path(__import__('sys').argv[1])
for logfile, expected in [('production-build',{'room_escape','orr_export_room','orr_new_arena'}),('app-harness-build',{'orr_sample'})]:
 found={}
 success=False
 for line in (root/(logfile+'.log')).read_text().splitlines():
  if not line.startswith('{'): continue
  event=json.loads(line)
  if event.get('reason')=='build-finished': success=event.get('success') is True
  if event.get('reason')!='compiler-artifact' or not event.get('executable'): continue
  target=event.get('target',{}); name=target.get('name')
  if name not in expected: continue
  assert ('lib' in target.get('kind',[]) and event.get('profile',{}).get('test')) if name=='orr_sample' else 'bin' in target.get('kind',[])
  assert name not in found
  found[name]=pathlib.Path(event['executable']).resolve(strict=True)
 assert success and set(found)==expected, (success,found,expected)
 for name,source in found.items():
  dest=root/'tools'/('app-tests' if name=='orr_sample' else name)
  assert source.is_file() and os.access(source,os.X_OK)
  shutil.copyfile(source,dest); dest.chmod(0o555)
PY
sha256sum "$evidence/tools/"* | tee "$evidence/tools.sha256"
if [[ $mode == gpu-export ]]; then
 ORR_REQUIRE_GPU=1 ORR_ROOM_CHECKPOINT_CAPTURE_DIR="$evidence/captures" \
 tests app-gpu 1 0 "$evidence/tools/app-tests" room_app::room_checkpoint_acceptance_tests::checkpoint_gpu_actual_app_acquire_resume_and_win_overlay
 python3 "$checker" captures "$evidence/captures"
fi
extra=(); if [[ $mode == syscall ]]; then extra+=(--syscall); fi
run readonly-export python3 tools/check-room-checkpoint-export.py --creator "$evidence/tools/orr_new_arena" --runtime "$evidence/tools/room_escape" --exporter "$evidence/tools/orr_export_room" --app-tests "$evidence/tools/app-tests" --workspace "$(pwd)/.." --evidence "$evidence/export" "${extra[@]}"
python3 "$checker" export "$evidence/export" "$mode"
echo "PASS $mode: $evidence"
