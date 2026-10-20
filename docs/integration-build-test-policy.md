# Integration-only build/test policy

The user's 2026-10-20 instruction replaces per-push and per-PR-update build/test
runs. Batch related implementation and parallel contributions before a shared
local build; rerun only checks affected by fixes. Do not treat unrun checks as
passing. Existing mandatory platform matrices, tests, artifacts and required
aggregates remain unchanged.

## Explicit integration boundary

Keep feature and stacked PRs draft during implementation. Ordinary pushes,
PR creation, synchronization, retargeting, and tag publication do not start these
five build/test workflows in this development-branch snapshot. There is no post-merge duplicate push run.

When a reviewed candidate is ready to integrate into
`su/engine-correctness-hardening` (or separately authorized `main` integration):

1. Combine the related changes and resolve conflicts. Freeze and record the PR
   head and target base SHAs. Target the actual integration branch, not another
   feature branch. Do not merge main or PR1 without separate authorization.
2. Change the draft integration PR to ready for review. The `ready_for_review`
   event triggers all the build/test workflows against the PR merge candidate.
   Verify that the expected workflows actually started; absence is not success.
3. Require all mandatory checks and review for that exact candidate. Record the
   workflow run IDs, tested merge SHA, head SHA and base SHA. Preserve failures.
4. Immediately before merging, verify unchanged head/base and successful required
   checks. Any change invalidates the candidate: return to draft, batch the next
   candidate, then mark ready again. A ready PR does NOT automatically retest on
   push or base update. Never reuse the previous candidate's success.

`workflow_dispatch` is a recovery/manual integration path, not a per-edit build.
Dispatch the frozen integration ref itself, not a different ref through a checkout
input. Verify the resulting run's SHA and all required workflows. Dispatch
availability requires the workflow on the default branch; do not bypass missing
registration by merging main. Use the draft-to-ready integration PR path instead.
A conflicting PR must be resolved before it can trigger a valid PR run.

This is scheduling, not removal of validation or changes to branch protection.
Existing queued runs are not cancelled by this patch, and other branches retain
their old triggers until they incorporate this change. No cancellation or setting
change is implied by publishing this patch. Newly restored unpublished workflows
must adopt the same policy before publishing their feature branches.

Reference: [GitHub workflow events](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows).

## Branch-specific scope

This backport starts from development commit
`87d54e185c7425abc7eb94a4ac85532241f871a3` and changes only the triggers of
`arena-agent-trials.yml`, `blockout-authoring.yml`, `determinism.yml`,
`input-map-authoring.yml`, and `ui-subtree-duplicate.yml`. Their job bodies,
matrices, commands, permissions, artifacts and required gates are unchanged.

The separately published stacked PR #253 applies this policy to 47 workflows
on its feature stack. This development backport does not import those additional
workflows or their unintegrated production dependencies, and does not accept that
stack. Future integrations must preserve the policy for newly added workflows
and derive the complete required job inventory from the frozen combined source.
Five workflows does not mean five jobs: expand all matrices and include required
aggregates. Local trigger fixtures do not replace hosted acceptance.
