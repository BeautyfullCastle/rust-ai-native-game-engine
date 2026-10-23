#!/usr/bin/env python3
"""Source-bound #274 CPU acceptance in the existing sample CI job, no new matrix.

Reuse the reviewed process owner, source fencing and bounded logs from #265.
Actual App-to-MCP is explicitly executed, not counted from an ignored default
sweep. No Cargo dependency, listener policy or retry is added by this gate.
"""
import json
import os
from pathlib import Path
import re
import shutil
import sys

import unsaved_close_gate as source
import static_model_extract_gate as runner
import png_sequence_clips_gate as shared

require = shared.require
APP = "app::work_status_tests::"
SUBPROCESS = APP + "actual_app_work_status_through_real_erp_and_mcp_all_versions"
# Exact inventories are frozen with the tests below; never inferred at runtime.
INVENTORIES = {'mailbox': ['work_status::tests::absence_before_publication_and_drop_never_mean_clean',
             'work_status::tests::fixed_schema_and_full_typed_inventory_preserve_domain_semantics',
             'work_status::tests::disabled_and_absent_domains_are_not_labeled_clean',
             'work_status::tests::reads_do_not_refresh_and_stale_blockers_do_not_establish_current_work',
             'work_status::tests::submillisecond_age_never_rounds_stale_to_fresh_and_time_cannot_go_backwards',
             'work_status::tests::fresh_positive_blocker_survives_incomplete_other_coverage',
             'work_status::tests::authoritative_host_blocker_is_current_even_without_usable_local_data',
             'work_status::tests::unknown_and_pending_operations_never_infer_idle',
             'work_status::tests::invalid_duplicate_missing_and_unsupported_rows_fail_closed',
             'work_status::tests::record_overflow_is_not_truncated_into_a_clean_inventory',
             'work_status::tests::contention_is_nonblocking_and_failed_publication_preserves_age_and_sequence',
             'work_status::tests::owner_drop_invalidates_even_while_mailbox_is_locked',
             'work_status::tests::service_drop_disables_publisher_and_new_pair_is_independent',
             'work_status::tests::identity_and_both_observation_counters_fail_closed_without_wrapping',
             'work_status::tests::poisoned_mailbox_fails_closed',
             'work_status::tests::discovery_has_no_identity_or_domains_and_tracks_owner_liveness',
             'work_status::tests::save_or_rollback_host_values_never_refresh_or_clear_local_work',
             'work_status::tests::largest_live_counters_are_decimal_strings_then_exhaust_without_reuse',
             'work_status::tests::strict_parameter_validation_has_fixed_non_reflective_errors',
             'work_status::tests::hard_wire_budget_and_no_loss_or_mutation_claims',
             'work_status::tests::response_validation_accepts_canonical_lifecycle_and_all_finite_reasons',
             'work_status::tests::response_validation_rejects_extra_nested_metadata_before_forwarding',
             'work_status::tests::response_validation_rejects_arbitrary_strings_arrays_and_wrong_types',
             'work_status::tests::response_validation_rejects_malformed_counters_checksums_and_observations',
             'work_status::tests::response_validation_rejects_false_freshness_but_allows_stale_truncation',
             'work_status::tests::response_validation_rejects_semantically_contradictory_assessments'],
 'host': ['editor_work_status_preserves_exhaustive_config_and_old_entrypoints',
          'editor_work_status_denies_nonread_caps_before_params_or_unpaired_state',
          'paired::editor_work_status_owner_lifecycle_and_discovery_are_not_permission',
          'paired::editor_work_status_finite_inventory_incomplete_coverage_and_operations',
          'paired::editor_work_status_reads_never_refresh_clean_or_dirty_snapshot_age',
          'paired::editor_work_status_host_is_authoritative_across_edit_play_save_and_rollback',
          'paired::editor_work_status_duplicate_reads_preserve_drafts_history_disk_and_tx_owner',
          'paired::editor_work_status_authenticated_ws_tcp_and_local_have_the_same_contract',
          'paired::editor_work_status_rejects_nonobject_extra_or_invalid_age_without_echoing_secrets',
          'paired::editor_work_status_does_not_acquire_or_wait_for_screenshot_capture',
          'paired::editor_work_status_arena_sibling_preserves_first_ready_input_setup',
          'paired::editor_work_status_collect_sibling_preserves_first_ready_input_setup_and_admission'],
 'mcp': ['static_listing_is_scene_group_read_only_not_proof_of_a_live_owner',
         'every_status_is_success_and_preserves_the_erp_payload_in_all_versions',
         'canonical_decimal_identities_remain_exact_above_javascript_integer_precision',
         'hostile_success_payloads_fail_locally_without_echoing_private_data_or_retrying',
         'valid_age_bounds_are_forwarded_once_without_extra_work',
         'absent_and_null_outer_arguments_keep_legacy_empty_object_normalization',
         'local_bad_arguments_are_sanitized_and_never_connect_to_erp',
         'non_object_outer_arguments_are_protocol_errors_without_erp_calls',
         'error_metadata_is_bounded_and_cannot_contaminate_later_results',
         'unreviewed_error_kind_is_not_promoted_to_structured_metadata',
         'disconnect_timeout_and_protocol_loss_do_not_invent_status_or_retry',
         'connection_failure_has_no_erp_code_or_automatic_second_attempt',
         'real_authenticated_read_only_socket_reports_unpaired_without_mutating_scene',
         'real_non_read_capabilities_cannot_learn_even_an_absent_owner',
         'real_erp_rejects_non_object_or_invalid_status_params_without_echoing_them',
         'genuine_erp_method_not_found_is_preserved_as_an_old_host_error',
         'reported_freshness_bound_must_match_this_call_not_a_looser_remote_default'],
 'app': ['app::work_status_tests::actual_app_work_status_requires_first_frame_and_has_fixed_inventory',
         'app::work_status_tests::actual_app_work_status_poll_is_passive_and_does_not_refresh_snapshot',
         'app::work_status_tests::actual_app_work_status_current_host_is_not_replaced_by_older_editor_frame',
         'app::work_status_tests::actual_app_work_status_one_event_callback_and_early_return_publish',
         'app::work_status_tests::actual_app_work_status_restart_changes_binding_and_requires_new_publication',
         'app::work_status_tests::actual_app_work_status_unknown_features_are_not_silently_clean',
         'app::work_status_tests::actual_app_work_status_sprite_sidecar_survives_scene_save_and_blocked_closes',
         'app::work_status_tests::actual_app_work_status_collect_document_and_preview_are_independent',
         'app::work_status_tests::actual_app_work_status_observation_never_invokes_close_only_predicates',
         'app::work_status_tests::actual_app_work_status_missing_file_close_observations_remain_historical',
         'app::work_status_tests::actual_app_work_status_loaded_tilemap_is_unknown_without_frame_serialization'],
 'operations': ['editor::work_status_operation_tests::work_status_ordinary_reads_are_idle_but_lost_mutations_are_uncertain']}


def run(label, command):
    return source.run(label, command)


def build(label, package, name, kind, extra):
    command = ["cargo", "build" if kind == "bin" else "test", "--locked", "--release", "-p", package]
    command += ["--lib"] if kind == "lib" else ["--" + kind, name]
    command += extra
    if kind != "bin":
        command += ["--no-run"]
    command += ["--message-format=json"]
    output = run("build-" + label, command)
    messages = [shared.load_json(line) for line in output.splitlines() if line.startswith("{")]
    require(messages and messages[-1] == {"reason": "build-finished", "success": True}, "Cargo did not finish successfully")
    require(sum(m.get("reason") == "build-finished" for m in messages) == 1, "ambiguous Cargo completion")
    artifacts = [m for m in messages if m.get("reason") == "compiler-artifact" and m.get("executable")
                 and m.get("target", {}).get("name") == name and m["target"].get("kind") == [kind]
                 and Path(m.get("manifest_path", "")).resolve() == runner.repo / "crates" / package / "Cargo.toml"]
    require(len(artifacts) == 1, "expected exactly one source-bound producer")
    artifact = artifacts[0]
    require(artifact.get("profile", {}).get("opt_level") == "3", "release producer required")
    require(artifact["profile"].get("test") == (kind != "bin"), "wrong producer profile")
    expected_features = source.FEATURES if package == "orr_editor" else (["collect-dodge", "default", "sample-host"] if package == "orr_remote" else [])
    require(sorted(artifact.get("features", [])) == expected_features, "wrong feature closure")
    original = Path(artifact["executable"])
    require(original.is_absolute() and original == original.resolve() and not original.is_symlink(), "noncanonical producer")
    require(original.parent == runner.target_dir / "release" / "deps" if kind != "bin" else original.parent == runner.target_dir / "release", "producer outside expected target")
    shared.ordinary_file(original, 256 * 1024**2)
    copy = runner.evidence / label
    with original.open("rb") as src, copy.open("xb") as dst:
        shutil.copyfileobj(src, dst, 1024**2)
    shutil.copymode(original, copy)
    require(runner.digest(original) == runner.digest(copy), "producer copy differs")
    runner.report["producers"][label] = {"producer":artifact,"file":copy.name,"sha256":runner.digest(copy),"bytes":copy.stat().st_size}
    runner.record()
    return str(copy)


def tests(label, binary, prefix, names, ignored=False):
    require(names and len(names) == len(set(names)), "empty/duplicate required tests")
    listing = run(label + "-inventory", [binary, prefix, "--list"] + (["--ignored"] if ignored else []))
    listed = re.findall(r"^([A-Za-z0-9_:]+): test$", listing, re.M)
    # Normal App inventory also includes the separately required ignored case.
    expected_listing = list(names) + ([SUBPROCESS] if label == "app" else [])
    require(sorted(listed) == sorted(expected_listing), "required test inventory changed")
    all_names = re.findall(r"^([A-Za-z0-9_:]+): test$", run(label + "-full-inventory", [binary,"--list"]), re.M)
    command = [binary,prefix,"--test-threads=1","--nocapture","--color=never"]
    command += ["--ignored"] if ignored else (["--skip",SUBPROCESS] if label == "app" else [])
    output = run(label + "-run",command)
    filtered = len(all_names) - len(names)
    require(re.findall(r"; (\d+) filtered out;",output) == [str(filtered)], "unexpected filtered count")
    runner.result(output.replace(f"; {filtered} filtered out;", "; 0 filtered out;"),names)
    runner.report["coverage"][label] = {"passed":len(names),"failed":0,"ignored":0,"names":names}
    runner.record()


def check():
    remote = build("remote-unit", "orr_remote", "orr_remote", "lib", ["--features","collect-dodge"])
    host = build("remote-host", "orr_remote", "editor_work_status", "test", ["--features","collect-dodge"])
    mcp = build("mcp-protocol", "orr_mcp", "editor_work_status", "test", [])
    editor = build("editor-app", "orr_editor", "orr_editor", "lib", ["--no-default-features","--features",source.REQUESTED])
    executable = build("orr_mcp", "orr_mcp", "orr_mcp", "bin", [])
    os.environ["ORR_WORK_STATUS_MCP_BIN"] = executable
    for label,binary,prefix in [("mailbox",remote,"work_status::tests::"),("host",host,"editor_work_status_"),
                                ("mcp",mcp,""),("app",editor,APP),("operations",editor,"editor::work_status_operation_tests::")]:
        tests(label,binary,prefix,INVENTORIES[label])
    tests("app-mcp",editor,SUBPROCESS,[SUBPROCESS],True)
    run("strict-transport", ["cargo","clippy","--locked","--release","-p","orr_remote","-p","orr_mcp","-p","orr_cli","--lib","--bins","--","-D","warnings"])
    run("remote-feature-off", ["cargo","check","--locked","--release","-p","orr_remote","--no-default-features","--lib"])
    runner.report["coverage"]["scope"] = "CPU actual App/ERP/MCP and authenticated socket tests; contention decomposed; no physical OS window/GPU evidence"
    runner.record()


def main():
    error = None
    try:
        source.initialize(Path(__file__).resolve().parents[1],Path(os.environ["ORR_WORK_STATUS_EVIDENCE"]),"editor-work-status")
        check()
        shared.source_fence()
    except BaseException as failure:
        error = failure
    finally:
        if runner.evidence is not None and runner.evidence.is_dir():
            runner.finish(error)
        elif error is not None:
            raise error


if __name__ == "__main__":
    main()
