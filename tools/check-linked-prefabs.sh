#!/usr/bin/env bash
# Required positive-count linked-prefab acceptance. Never converts skips to passes.
set -euo pipefail
mode=${1:?usage: check-linked-prefabs.sh contracts|gpu}
evidence=${ORR_LINKED_EVIDENCE:-target/linked-prefab-evidence}
mkdir -p "$evidence"
evidence=$(cd "$evidence" && pwd)
run_test() {
  local name=$1 expected=$2 ignored=$3; shift 3
  "$@" 2>&1 | tee "$evidence/$name.log"
  grep -Eq "test result: ok\\. ${expected} passed; 0 failed; ${ignored} ignored;" "$evidence/$name.log"
}
case "$mode" in
  contracts)
    cargo check --release --locked -p orr_reflect --no-default-features
    run_test parser-disabled 3 0 cargo test --release --locked -p orr_reflect --test linked_scene
    ORR_REQUIRE_LINKED_PARSER=1 run_test unified-consumer-boundary 2 0 cargo test --release --locked -p orr_remote --features collect-dodge,orr_reflect/linked-prefabs --test linked_prefab_boundary
    ORR_REQUIRE_LINKED_PARSER=1 run_test editor-unified-boundary 1 0 cargo test --release --locked -p orr_editor --features collect-dodge,orr_reflect/linked-prefabs --lib backend::linked_unified_tests::legacy_consumer_discovery_stays_legacy -- --exact
    run_test parser-enabled 13 0 cargo test --release --locked -p orr_reflect --features linked-prefabs --test linked_scene
    run_test edit-contracts 12 0 cargo test --release --locked -p orr_edit --features linked-prefabs,orr_sample/collect-dodge --test linked_prefab
    run_test legacy-fragments 6 0 cargo test --release --locked -p orr_edit --test fragment
    # Existing parser/document/proposal regressions also run with metadata enabled.
    cargo test --release --locked -p orr_reflect --features linked-prefabs
    cargo test --release --locked -p orr_edit --features linked-prefabs,orr_sample/collect-dodge
    run_test remote-contracts 6 0 cargo test --release --locked -p orr_remote --features collect-dodge,linked-prefabs --test linked_prefab
    run_test editor-widgets 6 1 cargo test --release --locked -p orr_editor --features linked-prefabs --test linked_prefab_ui
    cargo clippy --release --locked -p orr_edit -p orr_reflect -p orr_remote -p orr_sample -p orr_editor --features orr_editor/linked-prefabs,orr_sample/project-export --all-targets -- -D warnings
    ;;
  gpu)
    command -v bwrap
    bwrap --ro-bind / / -- /bin/true
    cargo build --release --locked -p orr_sample --features linked-prefabs,project-export --bin collect_dodge --bin orr_export_collect
    target=${CARGO_TARGET_DIR:-target}
    mkdir -p "$evidence/built-tools" "$evidence/captures"
    cp "$target/release/collect_dodge" "$evidence/built-tools/collect_dodge"
    cp "$target/release/orr_export_collect" "$evidence/built-tools/orr_export_collect"
    sha256sum "$evidence/built-tools/collect_dodge" "$evidence/built-tools/orr_export_collect" | tee "$evidence/built-tools.sha256"
    ORR_COLLECT_RUNTIME="$evidence/built-tools/collect_dodge" \
    ORR_COLLECT_EXPORTER="$evidence/built-tools/orr_export_collect" \
    ORR_LINKED_CAPTURES="$evidence/captures" \
    ORR_REQUIRE_GPU=1 \
      run_test editor-gpu-source-hidden-export 1 0 cargo test --release --locked -p orr_editor --features linked-prefabs --test linked_prefab_ui actual_editor_gpu_linked_prefabs_source_hidden_export -- --ignored --exact --nocapture
    ;;
  *) echo "Unknown mode: $mode" >&2; exit 2;;
esac
