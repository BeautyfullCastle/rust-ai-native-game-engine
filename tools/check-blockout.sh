#!/usr/bin/env bash
# Mandatory bounded-box contracts and explicit GPU/export acceptance, never skip-pass.
set -euo pipefail
mode=${1:?usage: check-blockout.sh contracts|gpu}
case "$mode" in contracts|gpu) ;; *) exit 2 ;; esac
cd "$(dirname "${BASH_SOURCE[0]}")/.."
base=${ORR_BLOCKOUT_EVIDENCE:-target/blockout-evidence}
mkdir -p "$base"
evidence=$(mktemp -d "$base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
export CARGO_TERM_COLOR=never
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
run() { local name=$1; shift; timeout --kill-after=30s 30m "$@" 2>&1 | tee "$evidence/$name.log"; }
audit() {
  python3 - "$1" "$2" "$3" <<'PY'
import collections,pathlib,re,sys
suite,stage,path=sys.argv[1:]; text=pathlib.Path(path).read_text()
names={
 'cpu': [
 'six_named_faces_have_independent_outward_mesh_and_uv_oracle',
 'malformed_versions_duplicates_records_and_numeric_inputs_are_rejected',
 'invalid_edits_preserve_document_history_and_redo',
 'history_is_bounded_and_new_success_clears_redo',
 'save_reopen_retains_exact_bytes_and_recipe_hash',
 'immutable_revisions_are_idempotent_and_remain_loadable_for_binding_undo',
 'failed_save_changed_file_and_failed_bake_preserve_state_and_active_lock',
 'changed_lock_rejects_stale_bake_without_publishing_its_revision',
 'contained_regular_file_contract_rejects_escape_oversize_and_special_paths',
 'package_budget_rejects_revision_129_without_pruning_old_models',
 'recipe_mutation_inside_publish_callback_retains_active_lock',
 'preexisting_name_collision_never_overwrites_installed_content'],
 'deps':['the_editor_does_not_depend_on_the_simulation_crates','the_editor_sources_do_not_name_the_simulation_crates','terrain_authoring_stays_optional_and_simulation_integration_is_explicit','navigation_is_optional_and_independent_of_terrain_physics','saved_projects_reuse_optional_sprite_packages_without_new_core_dependencies'],
 'room':['aggregate_assignment_rejection_preserves_cache_document_dirty_and_history','last_binding_removal_and_forged_hints_preserve_last_good_state','unknown_guid_open_rejection_preserves_document_cache_and_redo'],
 'gpu':['authored_box_widgets_six_face_gpu_and_source_hidden_room_export'],
}[suite]
if stage in ['list','ignored']:
 expected=names if stage=='list' or suite=='gpu' else []
 actual=re.findall(r'^(\S+): test$',text,re.M)
 assert collections.Counter(actual)==collections.Counter(expected),(suite,stage,actual,expected)
else:
 expected=collections.Counter(names)
 if suite=='gpu':
  assert '\nsuccesses:\n' in text
  names_block=text.rsplit('\nsuccesses:\n',1)[1].split('\ntest result:',1)[0]
  actual=collections.Counter(line.strip() for line in names_block.splitlines() if line.strip())
 else:
  actual=collections.Counter(re.findall(r'^test (\S+) \.\.\. ok$',text,re.M))
 assert actual==expected,(suite,actual,expected)
 rows=re.findall(r'^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured; (\d+) filtered out;',text,re.M)
 assert rows==[('ok',str(len(names)),'0','0','0','0')],rows
PY
}
test_suite() {
  local name=$1; shift
  run "$name-list" "$@" --list
  audit "$name" list "$evidence/$name-list.log"
  run "$name-ignored" "$@" --ignored --list
  audit "$name" ignored "$evidence/$name-ignored.log"
  run "$name" "$@" --nocapture
  audit "$name" result "$evidence/$name.log"
}
case "$mode" in
 contracts)
  test_suite cpu cargo test --locked --release -p orr_editor --no-default-features --features models --test blockout --
  test_suite deps cargo test --locked --release -p orr_editor --no-default-features --test deps --
  test_suite room cargo test --locked --release -p orr_editor --no-default-features --features room-project --test room_model_admission --
  run default-off cargo check --locked --release -p orr_editor --no-default-features --all-targets
  run models-only-clippy cargo clippy --locked --release -p orr_editor --no-default-features --features models --all-targets -- -D warnings
  run room-strict-clippy cargo clippy --locked --release -p orr_editor --no-default-features --features room-project --all-targets -- -D warnings
  ;;
 gpu)
  test "$(uname -s)" = Linux
  test "$(uname -m)" = x86_64
  test "$(id -u)" -ne 0
  command -v bwrap
  run namespace-preflight bwrap --ro-bind / / -- /bin/true
  temporary=$(mktemp -d /tmp/orr-blockout-runtime.XXXXXXXX)
  mkdir "$temporary/tmp" "$temporary/xdg-runtime"
  chmod 700 "$temporary/xdg-runtime"
  export TMPDIR="$temporary/tmp" XDG_RUNTIME_DIR="$temporary/xdg-runtime"
  trap 'rm -rf -- "$temporary"' EXIT
  python3 - "$evidence" <<'PY'
import pathlib,sys,tempfile
hidden=pathlib.Path.cwd().resolve().parent
assert not pathlib.Path(tempfile.gettempdir()).resolve().is_relative_to(hidden)
PY
  timeout --kill-after=30s 30m cargo build --locked --release -p orr_sample --no-default-features --features room-project,project-export --bin room_escape --bin orr_export_room --message-format=json > "$evidence/production-tools.jsonl" 2> "$evidence/production-tools.stderr"
  mkdir "$evidence/built-tools" "$evidence/captures"
  python3 - "$evidence" <<'PY'
import hashlib,json,pathlib,shutil,sys
root=pathlib.Path(sys.argv[1]); manifest=(pathlib.Path.cwd()/'crates/orr_sample/Cargo.toml').resolve()
rows=[json.loads(line) for line in (root/'production-tools.jsonl').read_text().splitlines() if line.strip()]
assert [r for r in rows if r.get('reason')=='build-finished']==[{'reason':'build-finished','success':True}]
records=[]
for name in ('room_escape','orr_export_room'):
 found=[r for r in rows if r.get('reason')=='compiler-artifact' and pathlib.Path(r['manifest_path']).resolve()==manifest and r['target']['name']==name and r['target']['kind']==['bin'] and r['profile']['test'] is False and r.get('executable')]
 assert len(found)==1,(name,found)
 row=found[0];assert set(row['features'])=={'project','project-export','room-project','sprites'}
 source=pathlib.Path(row['executable']).resolve(strict=True); copy=root/'built-tools'/name
 shutil.copy2(source,copy)
 sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
 assert sha(source)==sha(copy)
 records.append({'source':str(source),'copy':str(copy),'sha256':sha(copy),'producer':row})
(root/'production-tools-artifacts.json').write_text(json.dumps(records,indent=2)+'\n')
PY
  export ORR_REQUIRE_GPU=1 ORR_REQUIRE_PROJECT_ISOLATION=1
  export ORR_ROOM_RUNTIME="$evidence/built-tools/room_escape" ORR_ROOM_EXPORTER="$evidence/built-tools/orr_export_room"
  export ORR_BLOCKOUT_CAPTURE_DIR="$evidence/captures"
  test=authored_box_widgets_six_face_gpu_and_source_hidden_room_export
  run gpu-list cargo test --locked --release -p orr_editor --no-default-features --features room-project --test blockout_gpu -- --list
  audit gpu list "$evidence/gpu-list.log"
  run gpu-ignored cargo test --locked --release -p orr_editor --no-default-features --features room-project --test blockout_gpu -- --ignored --list
  audit gpu ignored "$evidence/gpu-ignored.log"
  run gpu cargo test --locked --release -p orr_editor --no-default-features --features room-project --test blockout_gpu -- --ignored --exact "$test" --show-output
  audit gpu result "$evidence/gpu.log"
  python3 - "$evidence/captures" <<'PY'
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[1]); inventory=json.loads((root/'capture-inventory.json').read_text())
actual={str(p.relative_to(root)):[p.stat().st_size,hashlib.sha256(p.read_bytes()).hexdigest()] for p in root.rglob('*') if p.is_file() and p.name!='capture-inventory.json'}
assert actual==inventory
views=['positive-x','negative-x','positive-y','negative-y','positive-z','negative-z']
prefixes=['editor-original','independent-original','editor-uv-only','editor-model-undo','editor-model-redo','editor-material-only','independent-final','editor-two-revision-undo','editor-two-revision-redo','editor-bindings-reopened','editor-project-reopened']
pngs={f'{prefix}-{view}.png' for prefix in prefixes for view in views}
pngs.update(['editor-original-composed.png','editor-uv-only-composed.png'])
pngs.update(f'runtime-{stage}-{suffix}.png' for stage in ['initial','moved','collision','interacted'] for suffix in ['source','export'])
assert len(pngs)==76
assert {name for name in inventory if name.endswith('.png')}==pngs
assert {name for name in inventory if name.endswith('.frame')}=={f'prepared-project-probe-{stage}.frame' for stage in ['initial','moved','collision','interacted']}
for name in ['acceptance-source.json','production-receipt.json','orr.export.json','editor-original-composed.png','editor-uv-only-composed.png']:
 assert name in inventory and inventory[name][0]>0,name
for stage in ['initial','moved','collision','interacted']:
 for suffix in ['source','export']:
  assert f'runtime-{stage}-{suffix}.png' in inventory
 assert f'prepared-project-probe-{stage}.frame' in inventory
PY
  ;;
esac
printf 'Blockout %s passed; evidence: %s\n' "$mode" "$evidence"
