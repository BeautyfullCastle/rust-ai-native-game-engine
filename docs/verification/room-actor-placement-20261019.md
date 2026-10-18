# Room actor ground placement verification — 2026-10-19

Local Linux verification against base `5589e0b02ed3d267252e2120644c886cd10f1007`.
After a successful current Read, viewport ground clicks stage Player, Key or Exit
XZ values. Explicit Apply commits the layout atomically. Actor and wall tools are
mutually exclusive: arming the other tool is disabled/refused while one is active,
not an implicit cancel-and-switch. Existing actor GUIDs, model bindings, camera,
renderer and schema remain unchanged. Independent source and workflow reviews
approved the scoped change.

## Passing checks

- **34 distinct CPU tests passed**: eight actor tests, fifteen footprint tests,
  three existing editor stamp regressions and eight pure stamp tests.
  Commands used `cargo test -p orr_editor --lib --features room-authoring` with
  selectors `room_authoring` and `room_wall_stamp::`, serially.
  Logs: `/tmp/room-actor-placement-v1-cpu.log` (26 passes) and
  `/tmp/room-actor-placement-v1-stamp.log` (8 passes).
- Default library and minimal `--no-default-features --features room-authoring`
  checks passed: `/tmp/room-actor-placement-v1-default.log` and
  `/tmp/room-actor-placement-v1-minimal.log`.
- Strict library and both integration-test Clippy passed with `--lib --test
  room_actor_placement_ui --test room_wall_footprint_ui --features room-authoring
  -- -D warnings`. Final log: `/tmp/room-actor-placement-v2-clippy.log`.
- Actor and wall integration builds passed. Actor's final fixture rebuild is
  `/tmp/room-actor-placement-v2-ui-build.log`; both original builds are recorded
  in `/tmp/room-actor-placement-v1-ui-build.log`.
- **Actor actual UI: 1 passed, 0 failed**, 14.76 seconds.
- **Wall Off regression UI: 1 passed, 0 failed**, 10.61 seconds.
- **Wall snapped regression UI: 1 passed, 0 failed**, 12.51 seconds.

## Separate actual UI evidence

Actor: `/workspace/shared/room-actor-placement-ui-20261019-v2/`, exact test
`actor_ground_placement_preserves_staging_and_completes_room`, with
`acceptance.json`, `run.log`, source stability records and six PNGs:
`01-overlap-refused.png`, `02-staged-roles.png`, `03-applied-saved.png`,
`04-reopened.png`, `05-key-collected.png`, `06-room-won.png`.
It verifies three role ground clicks, both directions of actor/wall mutual
exclusion, no authoring mutation during staging, atomic Player-wall overlap
refusal, Escape preserving selection, Apply/Undo/Redo/Save/Reopen and key pickup
followed by Exit/won gameplay. GUIDs, models and camera remain preserved.

Wall Off: `/workspace/shared/room-actor-wall-off-ui-20261019-v1/`, exact test
`two_corner_wall_stages_creates_persists_and_blocks_player`, with its separate
receipt/log/source proof and five PNGs.

Wall snapped: `/workspace/shared/room-actor-wall-grid-ui-20261019-v1/`, exact test
`snapped_wall_cancels_grid_changes_and_preserves_gameplay`, with its separate
receipt/log/source proof and six PNGs. These regressions retain staging,
Create/persistence/collision coverage through the changed shared panel and input
ownership paths.

## Review correction and retained failure

Review found that a cached checksum check before Apply could miss unpumped host
changes. A checked sibling of the existing layout API now compares the expected
checksum inside the authoritative transaction after `tx.begin` and before any
patch. Existing rollback handles mismatch; the original API remains intact.
The new regression mutates the host without pumping, then proves stale Apply is
refused without overwriting roles or changing the host checksum/full history.

The first actor UI fixture wrongly expected Player/Key trigger overlap rejection.
The existing validator permits that overlap. The test now puts the Player inside
the east wall while also changing Key, then checks the whole edit is refused
atomically. This was a test-only correction; no gameplay validator was changed.
Original failure retained at `/tmp/room-actor-placement-ui-v1.log` and
`/workspace/shared/room-actor-placement-ui-20261019-v1/`.
All production and CPU-test sources remained unchanged after the passing 34-test
cohort; only the integration fixture and explanatory documentation were refined.

## Frozen source evidence

All nine entries stayed unchanged across the three final UI runs and subsequent
checksum verification. Manifest `/tmp/room-actor-placement-v2-source.sha256`
has SHA-256 `29998a3e5639bff240844f5a9e2fb5c5ff21f3d07f0ee30d31f2587eeb777411`.

| Repository-relative file | SHA-256 |
| --- | --- |
| `crates/orr_editor/src/room_authoring_panel.rs` | `09a77ac2b4c72c49a500b0a39aa5a89901a7e4a4bee33350d3547ee107b80be4` |
| `crates/orr_editor/src/room_authoring_panel/actor_placement.rs` | `0b7e46d26e731fa6fc4e595743bc53cbc5f0e5d65a3442e9c598b831dc82d528` |
| `crates/orr_editor/src/room_authoring_panel/actor_placement/tests.rs` | `d263957c90ead5bc28196cbd7fe244b2376de455f0851189e8b9085390eed5f6` |
| `crates/orr_editor/src/room_authoring.rs` | `d7a8cd1de6dbfdd1367537b016eef7462ebe403644313f4bd2e97709e9bd1d37` |
| `crates/orr_editor/src/app.rs` | `65d6e387ede78f3f0ef9afb7df63f5c091e164f6666c5b2a0e6cb8d1038af136` |
| `crates/orr_editor/src/app/room_actor_placement.rs` | `5170cf8b836e96d8bc57d104e97c4940a82a9bae558ae903a363c79e1d203a87` |
| `crates/orr_editor/tests/room_actor_placement_ui.rs` | `df30e1192357745e90853ff77198e9dab51cca4334ef9c49b02bb85cec0d9267` |
| `crates/orr_editor/tests/room_wall_footprint_ui.rs` | `541680b345e4dd3c4b56ba5c18ca6846fd4f142bed71db102d639379778ce975` |
| `docs/editor-room-actor-placement.md` | `bf8ff2b606e62ee26541dfaff136c4dff0419b618cde49707c3dd27475162243` |

## Limits

The three UI runs used synthetic egui/AccessKit input and llvmpipe (LLVM 19.1.7,
256 bits) software Vulkan. They do not establish physical GPU, physical OS input,
native desktop-window or Windows execution acceptance. Remote CI/publication
remain separate gates. This report was added after source freezing;
`git diff --check` is clean.
