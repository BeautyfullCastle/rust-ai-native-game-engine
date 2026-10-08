#!/usr/bin/env bash
# Required positive-count navigation gates. No zero-test/GPU/isolation fallback.
set -euo pipefail
mode=${1:?usage: check-navigation-playground.sh contracts|gpu-export}
case "$mode" in contracts|gpu-export) ;; *) exit 2 ;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
repo=$(pwd -P)
test "$(uname -s)" = Linux
test "$(uname -m)" = x86_64
export CARGO_TERM_COLOR=never PYTHONDONTWRITEBYTECODE=1
checker=tools/check_navigation_playground_gate.py
# A commit/tree can identify the executed source only if every source file is committed.
test -z "$(git status --porcelain --untracked-files=all)"
base=${ORR_NAVIGATION_EVIDENCE:-/tmp/orr-navigation-playground-evidence}
python3 - "$base" "$repo" <<'PY'
from pathlib import Path
import sys
base, repo = map(Path, sys.argv[1:])
assert base.is_absolute() and base == base.resolve(), 'canonical absolute evidence path required'
assert base.is_relative_to(Path('/tmp').resolve()) and base != Path('/tmp').resolve()
assert not base.is_relative_to(repo.parent), 'evidence must be outside hidden sources'
PY
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
printf 'Navigation evidence: %s\n' "$evidence"
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
cp tools/navigation-playground-inventory.json "$evidence/expected-inventory.json"
export NAVIGATION_SOURCE_SHA=$(git rev-parse HEAD)
export TMPDIR="$evidence/tmp"
mkdir "$TMPDIR"
run() {
 local name=$1; shift
 printf '%q ' "$@" > "$evidence/$name.command.txt"; printf '\n' >> "$evidence/$name.command.txt"
 timeout --kill-after=30s 40m "$@" 2>&1 | tee "$evidence/$name.log"
}
run_json() {
 local name=$1; shift
 printf '%q ' "$@" > "$evidence/$name.command.txt"; printf '\n' >> "$evidence/$name.command.txt"
 timeout --kill-after=30s 40m "$@" 2> >(tee "$evidence/$name.stderr.log" >&2) | tee "$evidence/$name.jsonl"
}
tests() {
 local name=$1; shift
 local command=() options=(--format=pretty --test-threads=1 --show-output)
 python3 "$checker" command "$name" "$@" > "$evidence/$name.command.nul"
 mapfile -d '' -t command < "$evidence/$name.command.nul"
 run "$name-list" "${command[@]}" --list
 python3 "$checker" log "$name" list "$evidence/$name-list.log"
 run "$name-ignored" "${command[@]}" --ignored --list
 python3 "$checker" log "$name" ignored "$evidence/$name-ignored.log"
 if [[ $name == readonly-export ]]; then options+=(--ignored); fi
 run "$name" "${command[@]}" "${options[@]}"
 python3 "$checker" log "$name" result "$evidence/$name.log"
}
finish() {
 git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision-after.txt"
 cmp "$evidence/source-revision.txt" "$evidence/source-revision-after.txt"
 test -z "$(git status --porcelain --untracked-files=all)"
 printf 'PASS %s: %s\n' "$mode" "$evidence"
}
run checker-fixtures python3 tools/test_navigation_playground_gate.py
if [[ $mode == contracts ]]; then
 for package in orr_sample orr_editor; do
  run "$package-default-graph" cargo tree --locked -p "$package" --no-default-features --edges normal --prefix none
  python3 "$checker" default-graph "$package" "$evidence/$package-default-graph.log"
 done
 for lane in project-admission app-cpu template pure-admission remote-owned legacy-remote package-manifest editor-default-off editor-enabled-guards editor-cpu; do
  tests "$lane"
 done
 run strict-clippy cargo clippy --release --locked --no-default-features \
  -p orr_games -p orr_package -p orr_sample -p orr_remote -p orr_editor -p orr_navigation_view \
  --features orr_games/navigation,orr_sample/navigation-project,orr_sample/project-create,orr_sample/project-export,orr_remote/navigation,orr_editor/navigation-project,orr_editor/project-create \
  --all-targets -- -D warnings
 finish
 exit 0
fi
# No privileged root or bypass: the real namespace must work before building.
test "$(id -u)" -ne 0
command -v bwrap
export ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1
# Vulkan is the CI preference, not proof of the selected backend. Each actual
# acceptance independently requires a CPU/software adapter; none may skip.
export WGPU_BACKEND=${WGPU_BACKEND:-vulkan} LIBGL_ALWAYS_SOFTWARE=1
export XDG_RUNTIME_DIR="$evidence/xdg-runtime"
mkdir -m 700 "$XDG_RUNTIME_DIR"
export XDG_CACHE_HOME="$evidence/xdg-cache" MESA_SHADER_CACHE_DIR="$evidence/mesa-shader-cache"
mkdir -m 700 "$XDG_CACHE_HOME" "$MESA_SHADER_CACHE_DIR"
printf '%s\n' "WGPU_BACKEND_HINT=$WGPU_BACKEND" "VK_DRIVER_FILES=${VK_DRIVER_FILES:-unset}" \
 "ORR_REQUIRE_GPU=$ORR_REQUIRE_GPU" "ORR_REQUIRE_PROJECT_ISOLATION=$ORR_REQUIRE_PROJECT_ISOLATION" > "$evidence/gpu-environment.txt"
run namespace-preflight bwrap --die-with-parent --ro-bind / / --bind "$evidence" "$evidence" \
 --tmpfs "$repo" -- /bin/sh -c 'test ! -e "$1/Cargo.toml" && touch "$2/namespace-write-probe"' sh "$repo" "$evidence"
features=navigation-project,project-create,project-export
run_json production-build cargo build --release --locked --no-default-features -p orr_sample --features "$features" \
 --bin navigation_playground --bin orr_export_navigation --bin orr_new_navigation --message-format=json
# Capture each producer before a later test build can overwrite named bin paths.
python3 "$checker" pin "$evidence" "$repo" production
run_json app-build cargo test --release --locked --no-default-features -p orr_sample --features "$features" --lib --no-run --message-format=json
python3 "$checker" pin "$evidence" "$repo" app
run_json editor-build cargo test --release --locked --no-default-features -p orr_editor --features navigation-project,project-create --test navigation_project_editor --no-run --message-format=json
python3 "$checker" pin "$evidence" "$repo" editor
run_json export-build cargo test --release --locked --no-default-features -p orr_sample --features "$features" --test navigation_export --no-run --message-format=json
python3 "$checker" pin "$evidence" "$repo" export
sha256sum "$evidence/tools/"* | tee "$evidence/tools.sha256"
run creator-help "$evidence/tools/orr_new_navigation" --help
run creator-a "$evidence/tools/orr_new_navigation" --output "$evidence/created-project-a" --seed required-ci
run creator-b "$evidence/tools/orr_new_navigation" --output "$evidence/created-project-b" --seed required-ci
python3 "$checker" creator "$evidence"
export NAVIGATION_PLAYGROUND_CAPTURE_DIR="$evidence/app-captures"
export NAVIGATION_PROJECT_CAPTURE_DIR="$evidence/editor-captures"
export NAVIGATION_EXPORT_CAPTURE_DIR="$evidence/export-captures"
export ORR_NAVIGATION_RUNTIME="$evidence/tools/navigation_playground"
export ORR_NAVIGATION_EXPORTER="$evidence/tools/orr_export_navigation"
tests app-gpu "$evidence/tools/app-tests"
tests editor-gpu "$evidence/tools/editor-tests"
tests readonly-export "$evidence/tools/export-tests"
python3 "$checker" captures "$evidence"
sha256sum --check "$evidence/tools.sha256" | tee "$evidence/tools-rechecked.log"
finish
