#!/usr/bin/env bash
# Exact inventories and explicit mandatory CLI/GPU execution; never skip-pass.
set -euo pipefail
mode=${1:?usage: check-input-map-authoring.sh contracts|acceptance}
case "$mode" in contracts|acceptance) ;; *) exit 2 ;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
base=${ORR_INPUT_MAP_EVIDENCE_ROOT:-target/input-map-authoring-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
export CARGO_TERM_COLOR=never CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=1
printf '%s\n' "$evidence"
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
rustc -vV > "$evidence/rustc.txt"
sha256sum crates/orr_editor/{Cargo.toml,src/input_map_authoring.rs,src/bin/orr_input_map.rs,tests/input_map_authoring.rs} Cargo.lock docs/arena-input-map-authoring.md tools/check-input-map-authoring.sh .github/workflows/input-map-authoring.yml > "$evidence/source-before.sha256"
reserve() {
  python3 - <<'PY'
import shutil
assert min(shutil.disk_usage('.').free,shutil.disk_usage('/tmp').free)>=1024**3,'fixed1GiB reserve required'
PY
}
run() { local name=$1; shift; reserve; timeout --kill-after=30s 30m "$@" 2>&1 | tee "$evidence/$name.log"; reserve; }
audit() {
  python3 - "$evidence/$1.log" "$2" <<'PY'
import pathlib,re,sys
text=pathlib.Path(sys.argv[1]).read_text();name=sys.argv[2]
rows=re.findall(r'^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured;',text,re.M)
assert rows==[('ok','1','0','0','0')],rows
assert re.findall(r'^test (\S+) \.\.\. ok$',text,re.M)==[name]
assert not re.search(r'(?i)\bskip(?:s|ped|ping)?\b|GPU unavailable|no GPU adapter|fallback execution',text)
PY
}
inventory() {
  run inventory cargo test --release --locked -j1 -p orr_editor --no-default-features --features input-map-editor --test input_map_authoring -- --list
  run ignored-inventory cargo test --release --locked -j1 -p orr_editor --no-default-features --features input-map-editor --test input_map_authoring -- --ignored --list
  python3 - "$evidence" <<'PY'
import pathlib,re,sys
p=pathlib.Path(sys.argv[1]);ordinary={
'seven_action_widgets_save_reopen_and_exact_undo_redo',
'widget_conflicts_pending_drafts_and_empty_actions_preserve_contract',
'malformed_limits_and_failed_open_leave_document_and_redo_unchanged',
'symlinks_and_existing_destinations_are_rejected_without_replacement',
'authored_saved_actions_drive_production_controls_and_real_frame_trace'}
explicit={'authored_saved_map_is_accepted_by_actual_arena_cli','authored_input_map_widgets_have_real_gpu_pixels'}
for file,expected in [('inventory.log',ordinary|explicit),('ignored-inventory.log',explicit)]:
 actual=re.findall(r'^(\S+): test$',(p/file).read_text(),re.M)
 assert len(actual)==len(expected) and set(actual)==expected,(file,actual)
PY
}
case "$mode" in
contracts)
  inventory
  for name in seven_action_widgets_save_reopen_and_exact_undo_redo widget_conflicts_pending_drafts_and_empty_actions_preserve_contract malformed_limits_and_failed_open_leave_document_and_redo_unchanged symlinks_and_existing_destinations_are_rejected_without_replacement authored_saved_actions_drive_production_controls_and_real_frame_trace; do
    run "$name" cargo test --release --locked -j1 -p orr_editor --no-default-features --features input-map-editor --test input_map_authoring "$name" -- --exact --test-threads=1 --nocapture
    audit "$name" "$name"
  done
  # The standalone binary owns a local module and therefore keeps its empty test harness.
  run tool-harness cargo test --release --locked -j1 -p orr_editor --no-default-features --features input-map-editor --bin orr_input_map -- --list
  run default-inventory cargo test --release --locked -j1 -p orr_editor --no-default-features --test input_map_authoring -- --list
  python3 - "$evidence" <<'PY'
import pathlib,re,sys
p=pathlib.Path(sys.argv[1])
for name in ['tool-harness','default-inventory']:
 text=(p/(name+'.log')).read_text();assert not re.findall(r'^(\S+): test$',text,re.M);assert '0 tests, 0 benchmarks' in text
PY
  for name in bins_without_test_harnesses_have_no_tests_or_local_modules the_testless_bin_guard_catches_attributes_modules_and_generated_sources; do
    run "$name" cargo test --release --locked -j1 -p orr_tui --test deps "$name" -- --exact --nocapture
    audit "$name" "$name"
  done
  run default-check cargo check --release --locked -j1 -p orr_editor --no-default-features --all-targets
  run strict-default cargo clippy --release --locked -j1 -p orr_editor --no-default-features --all-targets -- -D warnings
  run strict-feature cargo clippy --release --locked -j1 -p orr_editor --no-default-features --features input-map-editor --all-targets -- -D warnings
  ;;
acceptance)
  test "$(uname -s)" = Linux
  inventory
  mkdir "$evidence/captures" "$evidence/tools"
  reserve
  timeout --kill-after=30s 30m cargo build --release --locked -j1 -p orr_sample --no-default-features --features input-actions --bin arena --message-format=json > "$evidence/arena-producer.jsonl" 2> "$evidence/arena-producer.stderr"
  reserve
  python3 - "$evidence" <<'PY'
import pathlib,json,hashlib,shutil,sys
p=pathlib.Path(sys.argv[1]);rows=[json.loads(s) for s in (p/'arena-producer.jsonl').read_text().splitlines() if s.strip()]
assert [r for r in rows if r.get('reason')=='build-finished']==[{'reason':'build-finished','success':True}]
found=[r for r in rows if r.get('reason')=='compiler-artifact' and r.get('executable') and r['manifest_path']==str(pathlib.Path('crates/orr_sample/Cargo.toml').resolve()) and r['target']['name']=='arena' and r['target']['kind']==['bin']]
assert len(found)==1;row=found[0];assert row['features']==['input-actions'] and row['profile']['test'] is False
source=pathlib.Path(row['executable']).resolve();copy=p/'tools/arena';shutil.copy2(source,copy)
h=lambda q:hashlib.sha256(q.read_bytes()).hexdigest();assert h(source)==h(copy)
(p/'arena-artifact.json').write_text(json.dumps({'producer':row,'original':str(source),'frozen':str(copy),'sha256':h(copy)},indent=2)+'\n')
PY
  export ORR_INPUT_MAP_ARENA_BIN="$evidence/tools/arena" ORR_INPUT_MAP_EVIDENCE="$evidence/captures" ORR_REQUIRE_GPU=1
  for name in authored_saved_map_is_accepted_by_actual_arena_cli authored_input_map_widgets_have_real_gpu_pixels; do
    run "$name" cargo test --release --locked -j1 -p orr_editor --no-default-features --features input-map-editor --test input_map_authoring "$name" -- --ignored --exact --test-threads=1 --nocapture
    audit "$name" "$name"
  done
  python3 - "$evidence" <<'PY'
import pathlib,json,hashlib,sys,struct
p=pathlib.Path(sys.argv[1]);a=json.loads((p/'arena-artifact.json').read_text());h=lambda q:hashlib.sha256(q.read_bytes()).hexdigest()
assert h(pathlib.Path(a['frozen']))==a['sha256']
expected={'arena-loader.stdout','arena-loader.stderr','authored-map.json','input-map-default.png','input-map-authored.png'}
assert {q.name for q in (p/'captures').iterdir()}==expected
records={q.name:{'bytes':q.stat().st_size,'sha256':h(q)} for q in (p/'captures').iterdir()}
for n in ['input-map-default.png','input-map-authored.png']:
 data=(p/'captures'/n).read_bytes();assert data[:8]==b'\x89PNG\r\n\x1a\n';assert struct.unpack('>II',data[16:24])==(900,760)
assert records['input-map-default.png']['sha256']!=records['input-map-authored.png']['sha256']
(p/'capture-manifest.json').write_text(json.dumps(records,indent=2)+'\n')
PY
  ;;
esac
sha256sum -c "$evidence/source-before.sha256" > "$evidence/source-after-check.txt"
