# Selected Room wall height verification — 2026-10-19

Base: `6a9d76698ef295b3661ad8917d4c8e2d5a09ed2a`. Verification ran on Linux on 2026-10-18 UTC. Independent source and workflow reviews approved the scoped change.

## Results

- **40 CPU tests passed:** six height, eight actor placement, fifteen footprint, three editor stamp and eight pure stamp tests. Commands used `cargo test -p orr_editor --lib --features room-authoring` with `room_authoring` and `room_wall_stamp::tests` filters, serially. Logs: `/tmp/room-wall-height-v5-{cpu,stamp}.log`.
- Default library check and minimal `--no-default-features --features room-authoring` check passed. Strict Clippy passed for the library and height, footprint and actor integration targets with `-D warnings`. Logs: `/tmp/room-wall-height-v6-{ui-build,default,minimal,clippy}.log`. The final UI-only fixture change was rebuilt and passed strict Clippy again in `/tmp/room-wall-height-v7-{ui-build,clippy}.log`.
- Four separate production EditorApp software-GPU cases passed:
  - Height: **1/1**, 15.88 s, five PNGs, `/workspace/shared/room-wall-height-ui-20261019-v2`.
  - Original grid-Off wall: **1/1**, 11.32 s, five PNGs, `/workspace/shared/height-wall-off-ui-20261019-v1`.
  - Snapped wall: **1/1**, 12.51 s, six PNGs, `/workspace/shared/height-wall-grid-ui-20261019-v1`.
  - Actor placement: **1/1**, 13.86 s, six PNGs, `/workspace/shared/height-actor-ui-20261019-v1`.

Each evidence directory contains `acceptance.json`, `run.log`, screenshots, `source-before.sha256` and `source-stability.log`. Height receipt records 26,243 changed image bytes, atomic invalid-height refusal, stale-read refusal, preserved identity/XZ/actors/models, Undo/Redo/Save/reopen and Play collision. Regression receipts retain wall staging/grid semantics and actor mutual exclusion, key pickup and exit completion.

## Failures retained and fixes reviewed

- Initial CPU run: 29 passed, two failed. Valid expansion failed because each individual physics patch is admitted before transaction completion. The reviewed fix expands half-height before moving center; shrinking moves center before reducing half-height. The fresh checksum comparison remains inside the authoritative transaction, with rollback and only the two Y fields patched. A reverse-shrink regression was added.
- Next runs recorded 30/2 and 31/1. The whole-world equality fixture incorrectly included the deliberately changed checksum; it now compares the complete entity payload after restoring only expected changed fields. Sequential edits also exposed idle-state timing assumptions. Tests now settle existing coherence/transaction predicates and explicitly Read before another Apply, without relaxing admission.
- The exact diagnostic shrink and one diagnostic full cohort subsequently passed. The earlier intermittent failure phase was **not conclusively reproduced**. Independently, review approved removing the fallible post-commit refresh: a committed edit reports success, invalidates cached Read state, preserves drafts and requires an explicit Read. This avoids reporting a refresh failure as a failed mutation. Earlier logs remain evidence rather than being relabeled as conclusively diagnosed.
- Integration compile E0308 was a test-only owned `String` passed where `&str` was required; the label was borrowed.
- First actual UI run could not find Read because the existing-wall section starts collapsed. The fixture now expands that section before navigation. No production change was needed.

Original logs/manifests are retained under `/tmp/room-wall-height-v1-*` through `v4-*`, the compile failure in `v5-ui-build.log`, and the first UI evidence directory `/workspace/shared/room-wall-height-ui-20261019-v1`.

## Frozen source

Final manifest `/tmp/room-wall-height-v7-source.sha256` has SHA256 `3c99dca776dd1e00444cae235718b7b2b7cd4ed3a3b443a44582a3e7c3994d76`. All five entries were unchanged through all final UI runs and rechecked afterward:

```text
9f024ae7ac6c4041e7d922cd067e3f76416016ab2835e7feb3c7390aab82f7d5  crates/orr_editor/src/room_authoring.rs
64eea948ee4f425dcd53a9fb9fe767d486b8e4ca6e9ad1eb180ad2efe0282f88  crates/orr_editor/src/room_authoring_panel.rs
3c3bbfc8106ded94c90a6f8dc1e2dea1cf2e0bf37429d7d38757e0336dd357e0  crates/orr_editor/src/room_authoring_panel/wall_height_tests.rs
1da28d3e7ff1f7c85ac6026826b886d4dfbefcc00eea3500af0ed599fe27778a  crates/orr_editor/tests/room_wall_height_ui.rs
4f0acd4b0bded2774c7bb7d8e9a804205d8beb772ac0fabeecc64acd680c75e4  docs/editor-room-wall-height.md
```

The UI used llvmpipe (LLVM 19.1.7, 256 bits) and synthetic input in owned fixtures. These results do not establish physical OS input, hardware GPU or Windows execution. No schema migration or user-project mutation was part of this verification.
