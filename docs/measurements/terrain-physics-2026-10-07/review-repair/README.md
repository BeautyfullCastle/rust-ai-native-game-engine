# Independent-review repair and final proof

Original implementation: `96f6e966981da1f4978485f29dbecd5b0d5f93ee`.
Its 190-test proof and all original failures remain in the parent directory.
Passing those tests did not replace independent review or authorize publication.

## Findings and corrections

1. **Support lost before the last substep could be forgotten.** Support history
   now spans the incoming cache and every refresh, cleared only by endpoint
   reacquisition. Regressions cover 1, 2 and 8 substeps, cached-only loss and
   reacquisition.
2. **Tiny-gravity free flight could sleep without support.** Under nonzero
   gravity, sleeping islands must have current near contacts carrying positive
   normal impulse and a path to an immovable anchor opposing gravity. A free
   cluster, sideways/ceiling contact or distant speculative terrain contact does
   not ground an island. Zero-gravity free sleep and the no-provider path remain
   unchanged. Ordinary convex contact skin remains `contact_margin`; refreshed
   terrain uses `linear_slop`. The falling-stack regression preserves this
   compatibility without changing either solver target algorithm.
3. **A restored sleeper could bypass validation at the idle shortcut.** A new
   read-only preflight reconstructs current ordinary/provider contacts and
   matches complete cached pair/feature keys before that shortcut. Contact
   connectivity is rebuilt rather than trusted from saved island IDs. Invalid
   sleepers reject byte-atomically; legitimate transitive stacks remain idle and
   byte-identical. Explicit/orphan wakes and zero gravity are preserved. Both
   public adapter validation and the solver entry enforce this boundary.
4. **Same-SHA source changes left the panel label stale.** The label now follows
   every current scene pin while the mesh stays cached by revision. A production
   editor/egui regression verifies the new label and retained model Arc.

Preflight is bounded but repeated at defensive API boundaries. No performance
improvement is claimed. The provider contract documents the additional read-only
call containing sleeping poses.

## Preserved intermediate failures

- `terrain-physics-review-solver.log`: one new test incorrectly required a
  separating free-flight pair to remain in contact on every tick. It now requires
  observed initial contact, no sleeping on every tick, and center-of-mass fall.
- `terrain-physics-review-falling-stack.log`: the strict `linear_slop` predicate
  rejected legitimate legacy convex collision-skin support. The falling heights
  3 and 3.03 remain in the regression; only convex support classification uses
  its pre-existing `contact_margin`.
- `terrain-physics-restored-preflight-adapter.log`: the new disconnected-cluster
  fixture initially reused terrain-only collision masks. The fixture now enables
  actual sphere–sphere collision; the valid-stack test also asserts separation
  and a genuine shared island before testing malformed island labels.

## Final affected proof

All commands use the guarded release build, two jobs, incremental disabled.

| Focused target | Observed result | Log |
| --- | --- | --- |
| physics3d library, static contacts, existing behavior, golden, session | 85 passed (20 + 28 + 26 + 10 + 1) | `terrain-physics-restored-preflight-solver.log` |
| terrain adapter all tests | 45 passed (12 assets + 1 unchanged terrain golden + 32 behavior) | `terrain-physics-restored-adapter-final.log` |
| terrain host | 5 passed | `terrain-physics-restored-host.log` |
| editor architecture, CPU/egui and mandatory viewport GPU | 7 passed (3 + 3 + 1) | `terrain-physics-restored-editor.log` |

Total affected rerun: **142 passing tests**. The 66 unchanged edit/ECS/game/bridge
checks from the original proof remain applicable, for 208 distinct focused tests
in the combined matrix. This is not a workspace-wide or multi-platform pass.

Strict Clippy (`-D warnings`) is repeated for the final solver, adapter, remote
host library/test/binary and editor library/CPU/GPU/dependency tests; see the
corresponding `terrain-physics-restored-*-clippy.log` files.

All original convex golden constants and the separate new terrain golden
constants remain unchanged throughout the repairs.

## Final visual evidence and limits

The final built GPU test is also executed directly to save
[final captures](final-captures/01-admitted.png),
[Play](final-captures/02-play-solid-and-hole.png), and
[Stop](final-captures/03-stop-restores-scene.png). It uses llvmpipe software
rendering, with no skipped test. Exact pixel restoration remains asserted.
Native Windows, physical GPU and native-window operation remain unverified.

Exact repaired-commit approval is a separate independent-review gate. Nothing
in this report authorizes publishing or closing the broader #102 collision work.
