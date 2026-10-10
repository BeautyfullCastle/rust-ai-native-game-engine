#!/usr/bin/env bash
# Required positive-count RoomEscapeV1 acceptance. No GPU or isolation skip passes.
set -euo pipefail
mode=${1:?usage: check-room-project.sh contracts|gpu}
case "$mode" in
  contracts|gpu) ;;
  *) echo "Unknown mode: $mode" >&2; exit 2 ;;
esac

cd "$(dirname "${BASH_SOURCE[0]}")/.."
evidence_base=${ORR_ROOM_EVIDENCE:-target/room-project-evidence}
mkdir -p "$evidence_base"
# Keep each invocation separate, so cached captures cannot masquerade as new proof.
evidence=$(mktemp -d "$evidence_base/$mode.XXXXXXXX")
evidence=$(cd "$evidence" && pwd)
git rev-parse HEAD 'HEAD^{tree}' > "$evidence/source-revision.txt"
export CARGO_TERM_COLOR=never
cat > "$evidence/expected-inventories.json" <<'JSONROOM'
{
  "room-simulation": {
    "names": [
      "room_escape_game::tests::admission_rejects_hostile_hidden_fields_missing_roles_and_anonymous_entities",
      "room_escape_game::tests::canonical_setup_and_initial_only_admission",
      "room_escape_game::tests::hostile_samples_are_wholly_neutral",
      "room_escape_game::tests::key_then_exit_requires_separate_edges_and_won_freezes",
      "room_escape_game::tests::movement_bounds_each_segment_before_casting",
      "room_escape_game::tests::only_connected_slot_zero_controls_player",
      "room_escape_game::tests::rejects_noncanonical_and_unbounded_authoring_before_math",
      "room_escape_game::tests::snapshot_replay_and_initial_frame_restart_match_exact_bytes",
      "room_escape_game::tests::wall_blocks_key_interaction_and_exit_stays_locked",
      "room_escape_game::tests::walls_stop_motion_and_diagonal_is_normalized"
    ],
    "ignored_names": [],
    "expected_summary_rows": [
      [
        10,
        0,
        0
      ]
    ]
  },
  "runtime-input": {
    "names": [
      "room_app::tests::focus_synthetic_and_restart_withhold_keys",
      "room_app::tests::reset_inserts_neutral_tick_before_new_press",
      "room_app::tests::strict_options"
    ],
    "ignored_names": [],
    "expected_summary_rows": [
      [
        3,
        0,
        0
      ]
    ]
  },
  "room-view-contracts": {
    "names": [
      "room_view::tests::current_pose_and_collected_key_are_read_only",
      "room_view::tests::local_translation_rotates_with_body_and_scale_stays_local"
    ],
    "ignored_names": [],
    "expected_summary_rows": [
      [
        2,
        0,
        0
      ]
    ]
  },
  "project-admission": {
    "names": [
      "admission_rejects_player_transform_that_exceeds_bounds_only_after_movement",
      "admission_rejects_reused_model_aggregate_draw_budget_before_export_smoke",
      "authored_scene_rejects_initial_overlap_and_noncanonical_roles",
      "owned_real_model_offscreen_readback_is_nonblank_and_read_only",
      "owned_scene_and_models_survive_removal_of_every_source",
      "prepared_initial_frame_session_and_simulation_have_exact_parity",
      "project_manifest_rejects_unsupported_routes_and_escaping_model_paths",
      "sidecar_rejects_redirected_roots_unknown_guids_missing_packages_and_stale_identity"
    ],
    "ignored_names": [
      "owned_real_model_offscreen_readback_is_nonblank_and_read_only"
    ],
    "expected_summary_rows": [
      [
        7,
        0,
        1
      ]
    ]
  },
  "editor-workflow": {
    "names": [
      "authored_xyz_models_save_reopen_real_keyboard_win_restart_and_stop",
      "legacy_game_routing_remains_explicit_with_room_feature",
      "production_room_viewport_native_texture_has_imported_uv_model_contribution"
    ],
    "ignored_names": [
      "production_room_viewport_native_texture_has_imported_uv_model_contribution"
    ],
    "expected_summary_rows": [
      [
        2,
        0,
        1
      ]
    ]
  },
  "editor-model-admission": {
    "names": [
      "aggregate_assignment_rejection_preserves_cache_document_dirty_and_history",
      "last_binding_removal_and_forged_hints_preserve_last_good_state",
      "unknown_guid_open_rejection_preserves_document_cache_and_redo"
    ],
    "ignored_names": [],
    "expected_summary_rows": [
      [
        3,
        0,
        0
      ]
    ]
  },
  "sample-gpu-readback": {
    "names": [
      "owned_real_model_offscreen_readback_is_nonblank_and_read_only"
    ],
    "ignored_names": [
      "owned_real_model_offscreen_readback_is_nonblank_and_read_only"
    ],
    "expected_summary_rows": [
      [
        1,
        0,
        0
      ]
    ]
  },
  "editor-gpu-viewport": {
    "names": [
      "production_room_viewport_native_texture_has_imported_uv_model_contribution"
    ],
    "ignored_names": [
      "production_room_viewport_native_texture_has_imported_uv_model_contribution"
    ],
    "expected_summary_rows": [
      [
        1,
        0,
        0
      ]
    ]
  },
  "production-source-hidden-export": {
    "names": [
      "room_real_export_source_hidden_gpu_workflow"
    ],
    "ignored_names": [
      "room_real_export_source_hidden_gpu_workflow"
    ],
    "expected_summary_rows": [
      [
        1,
        0,
        0
      ]
    ]
  }
}
JSONROOM
cat > "$evidence/check-results.py" <<'PYROOMCHECK'
import json,pathlib,re,sys
inventory,lane,mode,log=sys.argv[1:]
expected=json.loads(pathlib.Path(inventory).read_text())[lane]
text=pathlib.Path(log).read_text()
assert not re.search(r'SKIP:|negative skipped|without capture|direct relocated execution used|source-hiding acceptance not requested|\bskipping\b',text,re.I),(lane,'skip/fallback diagnostic')
if mode in ('list','ignored'):
    actual=sorted(re.findall(r'^([A-Za-z0-9_:]+): test$',text,re.M))
    names=expected['names'] if mode=='list' else expected.get('ignored_names',[])
    assert actual==sorted(names),(lane,mode,actual)
else:
    assert mode=='result',mode
    rows=[list(map(int,row)) for row in re.findall(r'^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored;',text,re.M)]
    assert text.count('test result:')==len(expected['expected_summary_rows']),(lane,'extra/malformed/missing summary')
    assert rows==expected['expected_summary_rows'],(lane,rows)
PYROOMCHECK
audit() {
  python3 "$evidence/check-results.py" "$evidence/expected-inventories.json" "$1" "$2" "$3"
}

run_command() {
  local name=$1; shift
  timeout --kill-after=30s 30m "$@" 2>&1 | tee "$evidence/$name.log"
}
run_test() {
  local name=$1 expected=$2 ignored=$3; shift 3
  local separated=0 only_ignored=0 argument
  local separator=()
  for argument in "$@"; do
    [[ "$argument" != -- ]] || separated=1
    [[ "$argument" != --ignored ]] || only_ignored=1
  done
  [[ $separated == 1 ]] || separator=(--)
  run_command "$name-list" "$@" "${separator[@]}" --list
  audit "$name" list "$evidence/$name-list.log"
  if [[ $only_ignored == 1 ]]; then
    audit "$name" ignored "$evidence/$name-list.log"
  else
    run_command "$name-ignored" "$@" "${separator[@]}" --ignored --list
    audit "$name" ignored "$evidence/$name-ignored.log"
  fi
  run_command "$name" "$@"
  audit "$name" result "$evidence/$name.log"
}


case "$mode" in
  contracts)
    run_test room-simulation 10 0 cargo test --release --locked -p orr_games --features room-escape --lib room_escape_game::tests::
    run_test runtime-input 3 0 cargo test --release --locked -p orr_sample --features room-project --lib room_app::tests::
    run_test room-view-contracts 2 0 cargo test --release --locked -p orr_sample --features room-project --lib room_view::tests::
    # Enable export here too, so invalid model admission must fail before its smoke run.
    run_test project-admission 7 1 cargo test --release --locked -p orr_sample --features room-project,project-export --test room_project
    run_test editor-workflow 2 1 cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow
    run_test editor-model-admission 3 0 cargo test --release --locked -p orr_editor --features room-project --test room_model_admission
    # Exercise unified optional features without changing any default feature or lint.
    run_command optional-features-clippy cargo clippy --release --locked \
      -p orr_games -p orr_package -p orr_model_bindings -p orr_sample -p orr_remote -p orr_editor \
      --features orr_games/room-escape,orr_sample/room-project,orr_sample/project-export,orr_editor/room-project,orr_editor/animated-models,orr_editor/terrain-physics,orr_editor/navigation,orr_editor/irradiance-probes,orr_editor/collect-ui,orr_editor/linked-prefabs,orr_editor/image-reimport \
      --all-targets -- -D warnings
    ;;
  gpu)
    test "$(uname -s)" = Linux
    test "$(uname -m)" = x86_64
    test "$(id -u)" -ne 0
    rustc -vV | tee "$evidence/rustc.txt"
    grep -Fx 'host: x86_64-unknown-linux-gnu' "$evidence/rustc.txt"
    command -v bwrap
    run_command namespace-preflight bwrap --ro-bind / / -- /bin/true
    # A fixture under the checkout cannot prove that its sources were hidden.
    temporary=$(mktemp -d /tmp/orr-room-project-runtime.XXXXXXXX)
    mkdir "$temporary/tmp" "$temporary/xdg-runtime"
    chmod 700 "$temporary/xdg-runtime"
    export TMPDIR="$temporary/tmp" XDG_RUNTIME_DIR="$temporary/xdg-runtime"
    trap 'rm -rf -- "$temporary"' EXIT
    python3 - "$evidence" <<'PYROOMPATHS'
import os,pathlib,sys,tempfile
hidden=pathlib.Path.cwd().resolve().parent
temporary=pathlib.Path(tempfile.gettempdir()).resolve()
assert not temporary.is_relative_to(hidden)
pathlib.Path(sys.argv[1],'isolation-roots.json').write_text(__import__('json').dumps({'hidden_workspace':str(hidden),'temporary':str(temporary)},indent=2)+'\n')
PYROOMPATHS
    # Read exact artifact identities from Cargo, then pin both tools immediately.
    timeout --kill-after=30s 30m cargo build --release --locked -p orr_sample --features room-project,project-export --bin room_escape --bin orr_export_room --message-format=json > "$evidence/production-tools.jsonl" 2> "$evidence/production-tools.stderr"
    mkdir -p "$evidence/built-tools" "$evidence/captures/editor"
    python3 - "$evidence" <<'PYROOMTOOLS'
import hashlib,json,pathlib,shutil,sys
root=pathlib.Path(sys.argv[1]);manifest=(pathlib.Path.cwd()/'crates/orr_sample/Cargo.toml').resolve()
rows=[json.loads(line) for line in (root/'production-tools.jsonl').read_text().splitlines() if line.strip()]
finished=[r for r in rows if r.get('reason')=='build-finished']
assert len(finished)==1 and finished[0]['success'] is True
expected={'default','project','project-export','room-project','sprites'}
records=[]
for name in ('room_escape','orr_export_room'):
    found=[r for r in rows if r.get('reason')=='compiler-artifact' and pathlib.Path(r['manifest_path']).resolve()==manifest and r['target']['name']==name and r['target']['kind']==['bin'] and r['profile']['test'] is False and r.get('executable')]
    assert len(found)==1,(name,found)
    item=found[0]
    assert set(item['features'])==expected,(name,item['features'])
    source=pathlib.Path(item['executable']).resolve(strict=True);copy=root/'built-tools'/name
    assert source.is_file() and not copy.exists()
    shutil.copy2(source,copy)
    digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    sha=digest(source);assert digest(copy)==sha
    records.append({'name':name,'source':str(source),'copy':str(copy),'sha256':sha,'manifest':str(manifest),'package_id':item['package_id'],'target':item['target'],'profile':item['profile'],'features':sorted(item['features']),'requested_features':['room-project','project-export']})
(root/'production-tools-artifacts.json').write_text(json.dumps(records,indent=2)+'\n')
PYROOMTOOLS
    sha256sum "$evidence/built-tools/room_escape" "$evidence/built-tools/orr_export_room" | tee "$evidence/built-tools.sha256"
    ORR_REQUIRE_GPU=1 \
      run_test sample-gpu-readback 1 0 cargo test --release --locked -p orr_sample --features room-project,project-export --test room_project owned_real_model_offscreen_readback_is_nonblank_and_read_only -- --ignored --exact --nocapture
    ORR_ROOM_CAPTURE_DIR="$evidence/captures/editor" \
    ORR_REQUIRE_GPU=1 \
      run_test editor-gpu-viewport 1 0 cargo test --release --locked -p orr_editor --features room-project --test room_project_workflow production_room_viewport_native_texture_has_imported_uv_model_contribution -- --ignored --exact --nocapture
    # The test copies these exact tools out of the workspace, hides source and
    # original executables, then runs the real exporter and relocated launcher.
    ORR_ROOM_RUNTIME="$evidence/built-tools/room_escape" \
    ORR_ROOM_EXPORTER="$evidence/built-tools/orr_export_room" \
    ORR_ROOM_EXPORT_CAPTURE_DIR="$evidence/export" \
    ORR_REQUIRE_GPU=1 \
    ORR_REQUIRE_PROJECT_ISOLATION=1 \
      run_test production-source-hidden-export 1 0 cargo test --release --locked -p orr_sample --features room-project,project-export --test room_export room_real_export_source_hidden_gpu_workflow -- --ignored --exact --nocapture
    # Require persisted proof as well as test counts; missing captures fail CI.
    for capture in room-editor-bound room-editor-models-offscreen; do
      test -s "$evidence/captures/editor/$capture.png"
    done
    for capture in initial-source initial-export moved-source moved-export interacted-source interacted-export no-models; do
      test -s "$evidence/export/captures/$capture.png"
    done
    test -s "$evidence/export/orr.export.json"
    sha256sum "$evidence/captures/editor/"*.png "$evidence/export/captures/"*.png "$evidence/export/orr.export.json" | tee "$evidence/captures.sha256"
    ;;
esac
