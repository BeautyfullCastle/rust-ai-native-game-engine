# Same-head targeted CI retry policy

This offline planner validates an operator-normalized snapshot. It never calls GitHub, retries a job, edits a workflow, declares a check passed, or permits a merge. Existing required CI coverage is unchanged. Exit 0 means only that a plan was produced; exit 2 means blocked. Maximum three total attempts is this planner's conservative retry budget, not GitHub's service limit.

## Use

Run `python tools/ci_targeted_retry.py snapshot.json --expected-head FULL_CURRENT_PR_HEAD --evidence-root IMMUTABLE_ARCHIVE_DIRECTORY`. Capture stdout into a new file, never over original evidence. Run fixtures with `python -m unittest discover -s tests -v`.

The tests include a complete synthetic input example in `RetryTests.setUp`. The macOS observation JSON is historical evidence, not a normalized executable plan or a claim that the old workflow completed.

## Input contract

Schema 1 has `run` (`id`, `head_sha`, `status`, `run_attempt`), `required_jobs`, `jobs`, `inventory_complete`, `upload_inventory_complete`, `transient_failures`, `failure_evidence`, `latest_job_ids`, `failure_kinds`, and `artifacts`.

Each job has a unique stable `key` (workflow YAML job ID plus fully expanded matrix identity), GitHub `id`, `run_id`, actual `run_attempt`, `head_sha`, `status`, `conclusion`, direct `needs` keys, and `uploads`. Expand matrix dependencies to all matching producer keys. Do not use GitHub display names alone as identity. Include every job, not only failed jobs. The independently reviewed workflow inventory supplies `required_jobs` and the graph; API job responses do not supply the graph. Completeness booleans are operator attestations, not discoverable proof produced by this tool.

Uploads have a resolved `name_template` with optional `{attempt}` and `{run_id}` substitutions and explicit `overwrite: false`. Resolve every other expression and all possible uploads first. An empty upload list means the reviewed job has no uploads, not unknown. Attempt-qualified names are preferred but not mandatory. Repeated existing names cause a conservative manual-review block, not a claim that GitHub necessarily rejects names across attempts. Never resolve a collision using overwrite or delete original proof.

Each failure evidence entry is keyed by logical job and contains `job_id`, actual `run_attempt`, `head_sha`, `run_id`, archive-relative `path`, and file `sha256`. Preserve original logs for every failure, including aggregate failures. Existing artifacts require `id`, `name`, `run_id`, `head_sha`, `producer_attempt`, `archive_path`, and `archive_sha256`. Archive the original bytes before retry. `producer_attempt` is normalized provenance verified from producer logs/manifests or attempt-specific responses; do not invent it from the workflow's latest attempt. Unknown or unavailable artifact provenance requires investigation, not overwrite.

`latest_job_ids` maps every logical key to the latest observed GitHub job ID, established from complete latest and attempt-specific job responses. A stale ID blocks planning. This independent normalization must not simply copy an old snapshot.

`failure_kinds` explicitly classifies every failed job as `transient` or `dependency_aggregate`. Only reviewed dependency-only aggregators with `aggregate_only: true` may use the latter. An assertion or independent step failure in a downstream job is not an aggregate failure.

`transient_failures` classifies all transient failures as `artifact_dns` or `runner_service_unavailable`. A dependent aggregate failure is not a new infrastructure diagnosis. Assertions, compilation errors, timeouts with unknown cause, cancellations, unknown conclusions, and unexplained skipped jobs do not qualify. Classification is reviewed evidence, not keyword-only matching.

## Execution and post-retry contract

Refresh PR head, run status, and artifact inventory immediately before a separately authorized retry. A changed head invalidates this plan. Re-run the failed jobs and their transitive dependent jobs, not the successful independent OS jobs. A job reruns from the beginning: steps skipped after an upload failure still need to execute and pass. The planner's `observed_census` preserves each actual attempt; the proposed attempt is not a result.

Afterward collect complete latest jobs and attempt-specific history, retain original logs/artifact IDs/digests, and record each logical job's actual result and producing attempt. A successful attempt-1 Linux job remains attempt 1 even when the macOS rerun is attempt 2. Pending, missing, cancelled, skipped required jobs or missing required step/evidence manifests block integration. Verify checkout tree and required artifact contents separately; head equality alone is not checkout proof. Do not infer success from the overall workflow badge. No merge decision is made here.

## Actual macOS case and exact source check

Historical job 119227889203, run 39601803643, head 0c405a60d68459e393993f2153de3a5848d34dbb failed at artifact upload with CreateArtifact ENOTFOUND (2026-10-21 13:43:23 UTC). Later FFI/Clippy/RAW16/hotpatch work remained skipped. The observation still contained other running jobs. It is not eligible for a terminal-run plan and cannot validate newer db54a6b67d6830407209d4c8e89ed9e8799adb2e.

Exact db54 tree 5c0f978c04b8e75c2cf4a03fac33f369acd1a200 determinism.yml has native-assemble, cargo-timings-native, room-core and raw16-native artifact names without attempt suffixes. This alone does not prove a collision. Inspect actual IDs and archive all original proof before any separately authorized rerun; this change does not modify those workflows.

## Official references

GitHub documents reruns retaining the original event SHA/ref, failed-only reruns, and reviewing previous attempts: https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs

The job rerun REST operation includes dependent jobs: https://docs.github.com/en/rest/actions/workflow-runs#re-run-a-job-from-a-workflow-run

Upload artifact overwrite deletes the earlier named artifact before replacing it. Do not use that option to bypass preservation: https://github.com/actions/upload-artifact#overwriting-an-artifact
