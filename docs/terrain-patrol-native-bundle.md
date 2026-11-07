# Native TerrainPatrol bundles

Build the editor with `--features terrain-patrol-export`. Create a local
NavigationYard3D scene, build its terrain route, apply the waypoint patrol and save
the scene. The existing Navigation export panel captures that saved state; later
edits are excluded. Unsaved terrain or route drafts must be resolved first.

Build a matching Linux x86-64 runtime:

```sh
cargo build --release -p orr_navigation_view --features terrain-patrol-bundle --bin navigation_player --bin navigation_export
```

In the editor, enter the runtime path, its lowercase SHA-256 and a new output
directory. Explicitly trust the supplied executable, then choose **Export saved
Navigation**. Verification executes that executable. Changing its path or hash
revokes the UI trust selection. Wait for the worker to finish and join before
closing or starting another export. The result belongs to the captured source.

The same exporter is available as a CLI:

```sh
navigation_export --scene game.scene.yaml --runtime /absolute/navigation_player --runtime-sha256 HEX --trusted-runtime --output NEW_DIRECTORY
```

The closed folder includes the saved scene, pinned terrain, runtime, fixed launcher
and manifest. Move the complete folder and run:

```sh
./run-navigation --ticks 200 --restart-check
```

This is a read-only headless point-agent player. Its JSON reports the patrol
configuration and progress together with the deterministic checksum and restart
result. The bundle opens from its own files; original authoring files and supplied
runtime can be removed. Restart and subsequent simulation use admitted Frame data.

`NavigationYard3DPatrolV1` is a separate compiled profile, including when patrol is
disabled. It requires the patrol scene and bundle capabilities in the exact probe
reply. A legacy point-only runtime cannot publish this profile. Recomputing a
manifest hash does not permit swapping profiles or initial state. Existing point
profile bytes, capabilities and output remain unchanged when the feature is off.
The surface bundle format remains unsupported for patrol.

Existing source snapshots, byte limits, confined paths, file inventory, runtime
identity checks and new-directory publication remain in force. A source change
before publication fails; partial private staging may be retained for inspection.
No automatic retry or overwrite occurs. Hashes establish integrity, not trust or
verified source provenance.

Acceptance covers actual export UI, worker ownership, source changes, real old
runtime rejection, relocation, original-file removal, two native 200-tick runs and
independent Frame/restart agreement. The connected desktop is locked: Windows new
UI/filesystem checks and GPU/window/physical playtest/video are unverified.
