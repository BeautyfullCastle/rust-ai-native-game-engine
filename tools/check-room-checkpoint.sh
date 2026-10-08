#!/usr/bin/env bash
# Positive-count checkpoint gates; GPU/export/syscall failures never become skips.
set -euo pipefail
mode=${1:?usage: check-room-checkpoint.sh contracts|gpu-export|syscall}
case "$mode" in contracts|gpu-export|syscall) ;; *) exit 2;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
base=${ORR_CHECKPOINT_EVIDENCE:-target/room-checkpoint-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD > "$evidence/source-revision.txt"
run() { local name=$1; shift; timeout --kill-after=30s 40m "$@" 2>&1 | tee "$evidence/$name.log"; }
tests() {
 local name=$1 passed=$2 ignored=$3; shift 3
 run "$name" "$@"
 grep -Eq "^test result: ok\\. ${passed} passed; 0 failed; ${ignored} ignored;" "$evidence/$name.log" || { echo "Missing positive test count for $name" >&2; exit 1; }
}
features=room-checkpoint,project-create,project-export
if [[ $mode == contracts ]]; then
 run syscall-checker-fixtures python3 tools/test_room_checkpoint_syscall.py
 tests room-goldens 10 0 cargo test --release --locked -p orr_games --features room-escape --lib room_escape_game::tests::
 tests package-closure 34 0 cargo test --release --locked -p orr_package --lib
 tests default-off 2 0 cargo test --release --locked -p orr_sample --features room-project --lib room_project::checkpoint_tests::
 tests metadata-only-window-rejected 1 0 cargo test --release --locked -p orr_sample --features room-ui,project-create --lib room_project::checkpoint_tests::metadata_only_admission_preserves_legacy_scene_and_owns_checkpoint -- --exact
 tests store 17 0 cargo test --release --locked -p orr_sample --features "$features" --lib room_checkpoint_store::tests::
 tests host-processes 8 2 cargo test --release --locked -p orr_sample --features "$features" --lib room_app::room_checkpoint_acceptance_tests:: -- --nocapture
 tests semantic-rules 1 0 cargo test --release --locked -p orr_sample --features "$features" --lib room_checkpoint::tests::
 tests admitted-profile 3 0 cargo test --release --locked -p orr_sample --features "$features" --lib room_project::checkpoint_tests::
 tests creator 1 0 cargo test --release --locked -p orr_sample --features "$features" --lib project_create::checkpoint_tests::
 tests editor-widgets 6 0 cargo test --release --locked -p orr_editor --features room-checkpoint,project-create --lib room_checkpoint_panel::tests::
 run strict-clippy cargo clippy --release --locked -p orr_sample -p orr_editor -p orr_package --features orr_sample/room-checkpoint,orr_sample/project-create,orr_sample/project-export,orr_editor/room-checkpoint,orr_editor/project-create --all-targets -- -D warnings
 exit 0
fi
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
 tests app-gpu 1 0 "$evidence/tools/app-tests" --ignored --exact room_app::room_checkpoint_acceptance_tests::checkpoint_gpu_actual_app_acquire_resume_and_win_overlay --nocapture
 for phase in chooser resumed-unlocked won; do for size in 1024x768 480x800; do test -s "$evidence/captures/checkpoint-$phase-$size.png"; done; done
fi
extra=(); if [[ $mode == syscall ]]; then extra+=(--syscall); fi
run readonly-export python3 tools/check-room-checkpoint-export.py --creator "$evidence/tools/orr_new_arena" --runtime "$evidence/tools/room_escape" --exporter "$evidence/tools/orr_export_room" --app-tests "$evidence/tools/app-tests" --workspace "$(pwd)/.." --evidence "$evidence/export" "${extra[@]}"
test -s "$evidence/export/result.json"
echo "PASS $mode: $evidence"
