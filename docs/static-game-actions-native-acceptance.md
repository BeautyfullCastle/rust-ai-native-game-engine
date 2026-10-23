# Private four-action native acceptance

This additive Linux acceptance fixture is not a third production game. It does
not modify the original two-game framebuffer oracles, generated-user acceptance,
or the pinned bootstrap engine. Neither private fixture binary nor its game alias
belongs in the production game registry or exported bundles.

The fixture gives Space, F, E and Q independent masks 1, 2, 4 and 8 and independent
per-consumed-tick counters. The Python validator recomputes every state transition
from the complete recorded inputs. It rejects numeric lookalikes, gaps, aliases,
missing scenarios, incorrect producers and a truncated fourth bit.

The standalone proof uses the production controller/player, an owned Xvfb server,
PID-bound window discovery, XTEST server input, actual presented consumed ticks,
XGetImage captures and an independent replay of the recorded ORRP bytes. Clear
proofs cover all four held actions and both axes, pause/focus loss, no automatic
resume, and the first naturally consumed neutral tick after explicit resume.

The editor proof uses the real EditorApp and production static-action router.
Read-only raw-key observations come from native winit/eframe events, never injected
RawInput. Full-tick states are explicitly derived by independently replaying the
actual local-host ORRP recording and bound to live snapshot checksums. The gate
checks every recorded input across released/reclaimed intervals, not just sampled
snapshots. A supported stop/save/open/play operation changes a wrapper scene
counter; this counter is not claimed to expose the editor's private internal epoch.

Independent framebuffer oracles inspect the fixture's four known marker positions
and colors. Receipt-supplied pixel counts or pass flags are not acceptance. XTEST
proves server keyboard delivery, not a physical human keyboard; the required CPU
Vulkan adapter is software graphics, not physical GPU proof. Source-hidden delivery
and packaged export remain false for this fixture and remain separately required
by the original delivery gates.

## Execution and evidence

The required `static-game-actions-native` job runs once on Ubuntu 22.04, not in a
new matrix. Its 70-minute job limit contains 64 minutes of explicit step caps and
6 minutes of overhead. The gate keeps two 900-second targeted compile limits,
180/240-second native limits and bounded helper/validator commands. This is an
operational limit, not a measured runtime guarantee. The job initializes evidence
before setup and always uploads success or partial failure artifacts. The existing
required checksum aggregate explicitly requires this job to succeed.

From a clean reviewed checkout, with hosted SHA/run/attempt environment variables
and native prerequisites provisioned:

```sh
cargo fetch --locked
python3 -B tools/static_game_actions_native_gate.py --artifacts /tmp/fresh-action-evidence
```

Cargo builds are locked/offline and exact package, target kind, source path,
release profile and executable hash are admitted from compiler messages. The
source tree is fenced before and after execution. Existing test binaries or stale
evidence directories are not accepted as a substitute for the current build.

Synthetic Python unit tests validate rejection behavior only. They do not establish
native execution, hosted CI success, hardware keyboard delivery, or physical GPU
coverage. Native acceptance is established only by the complete gate's current-run
artifacts and successful required CI check.
