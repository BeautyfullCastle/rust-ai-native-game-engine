#!/usr/bin/env bash
# Mandatory focused acceptance. Ignored GPU tests are explicitly run, never passes by omission.
set -euo pipefail
mode=${1:-contracts}
evidence=${ORR_UI_DUPLICATE_EVIDENCE:-target/ui-subtree-duplicate-evidence}
mkdir -p "$evidence"
evidence=$(cd "$evidence" && pwd)
export CARGO_TERM_COLOR=never
sha256sum crates/orr_editor/src/collect_ui_panel.rs crates/orr_editor/tests/ui_subtree_duplicate.rs docs/ui-subtree-duplication.md .github/workflows/ui-subtree-duplicate.yml tools/check-ui-subtree-duplicate.sh > "$evidence/source-$mode.sha256"
git rev-parse HEAD > "$evidence/commit-$mode.txt"
rustc -vV > "$evidence/rustc-$mode.txt"
run() { local name=$1; shift; "$@" 2>&1 | tee "$evidence/$name.log"; }
audit() {
  python3 - "$evidence/$1.log" "$2" "$3" <<'PY'
import pathlib,re,sys
text=pathlib.Path(sys.argv[1]).read_text()
rows=re.findall(r'^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored;',text,re.M)
assert rows==[('ok',sys.argv[2],'0',sys.argv[3])],rows
assert not re.search(r'(?im)^\s*(?:skip(?:ped)?\b|.*\bskipping\b)',text),'skip is not acceptance'
PY
}
inventory() {
  run inventory cargo test --release --locked -p orr_editor --no-default-features --features collect-ui --test ui_subtree_duplicate -- --list
  python3 - "$evidence/inventory.log" <<'PY'
import pathlib,re,sys
expected='''gpu_saved_duplicate_source_hidden_readonly_export_preserves_frames
widgets_copy_root_selection_parents_new_child_to_copy
widgets_duplicate_noncontiguous_subtree_preserve_fields_and_single_undo
widgets_invalid_depth_geometry_font_and_byte_candidates_fail_atomically
widgets_leaf_at_depth_four_and_every_screen_preserve_external_parent
widgets_no_selection_and_play_are_readonly
widgets_node_limit_failure_preserves_document_selection_and_history
widgets_repeated_duplicates_are_independent_and_reopen_exactly'''.splitlines()
actual=re.findall(r'^([^\s]+): test$',pathlib.Path(sys.argv[1]).read_text(),re.M)
assert sorted(actual)==sorted(expected),(actual,expected)
PY
}
case "$mode" in
  contracts)
    inventory
    run widgets cargo test --release --locked -p orr_editor --no-default-features --features collect-ui --test ui_subtree_duplicate -- --test-threads=1
    audit widgets 7 1
    run existing-panel cargo test --release --locked -p orr_editor --no-default-features --features collect-ui --lib collect_ui_panel::tests -- --test-threads=1
    audit existing-panel 6 0
    run byte-bound cargo test --release --locked -p orr_sample --no-default-features --features collect-ui --lib authored_ui::tests::rejects_size_version_syntax_and_trailing_data -- --exact
    audit byte-bound 1 0
    run default-check cargo check --release --locked -p orr_editor --no-default-features
    run strict-default cargo clippy --release --locked -p orr_editor --no-default-features --all-targets -- -D warnings
    run strict-collect cargo clippy --release --locked -p orr_editor --no-default-features --features collect-ui --all-targets -- -D warnings
    ;;
  gpu)
    test "$(uname -s)" = Linux
    test "$(uname -m)" = x86_64
    command -v bwrap
    bwrap --ro-bind / / -- /bin/true
    run producers cargo build --release --locked -p orr_sample --no-default-features --features project-create,project-export,collect-ui,collect-sprites,collect-progress --bin orr_new_arena --bin collect_dodge --bin orr_export_collect
    target=$(cd "${CARGO_TARGET_DIR:-target}" && pwd)
    mkdir -p "$evidence/built-tools" "$evidence/captures"
    for name in orr_new_arena collect_dodge orr_export_collect; do cp "$target/release/$name" "$evidence/built-tools/$name"; done
    sha256sum "$evidence/built-tools/"* > "$evidence/producer-binaries.sha256"
    ORR_NEW_ARENA_BIN="$evidence/built-tools/orr_new_arena" \
    ORR_COLLECT_RUNTIME="$evidence/built-tools/collect_dodge" \
    ORR_COLLECT_EXPORTER="$evidence/built-tools/orr_export_collect" \
    ORR_UI_DUPLICATE_CAPTURES="$evidence/captures" \
    ORR_REQUIRE_GPU=1 \
      run gpu-export cargo test --release --locked -p orr_editor --no-default-features --features collect-ui --test ui_subtree_duplicate gpu_saved_duplicate_source_hidden_readonly_export_preserves_frames -- --ignored --exact --nocapture
    audit gpu-export 1 0
    ;;
  *) echo "Unknown mode: $mode" >&2; exit 2 ;;
esac
