# Integration-only build/test policy

The user's 2026-10-20 instruction replaces per-push and per-PR-update build/test
runs. Batch related implementation and parallel contributions before a shared
local build; rerun only checks affected by fixes. Do not treat unrun checks as
passing. Existing mandatory platform matrices, tests, artifacts and required
aggregates remain unchanged.

## Explicit integration boundary

Keep feature and stacked PRs draft during implementation. Ordinary pushes,
PR creation, synchronization, retargeting, and tag publication do not start these
47 build/test workflows. There is no post-merge duplicate push run.

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

## Historical branch scopes and combined candidate

The development backport started from commit
`87d54e185c7425abc7eb94a4ac85532241f871a3`. Its scheduling change covered only
`arena-agent-trials.yml`, `blockout-authoring.yml`, `determinism.yml`,
`input-map-authoring.yml`, and `ui-subtree-duplicate.yml`, the five workflows
present in that snapshot. That backport did not import or accept the additional
workflows and production dependencies from the separately published 47-workflow
feature stack in PR #253. Its scheduling patch preserved job bodies, matrices,
commands, permissions, artifacts and required gates.

This combined candidate retains the authoring stack's complete 47-workflow
policy and expanded regression assertions. It also incorporates the subsequent
browser-runner fix from the backport branch: the existing mandatory browser job
uses `ubuntu-24.04` instead of the moving `ubuntu-latest` label. No browser checks
or required aggregate gates are removed. This local combination alone is not
evidence of hosted acceptance or authorization to merge main or PR1.

Derive the complete required job inventory from the frozen combined source:
47 workflows does not mean 47 jobs. Expand every matrix and include required
aggregates. Local trigger fixtures do not replace hosted acceptance. Future
integrations must preserve this policy for newly added workflows.
