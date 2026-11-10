# Copy a recorded Room regression route

Build the existing optional `room-authoring` editor feature. In a local Room
project, save the scene, enter Play, perform a route, then **Stop**. Open
**File > Room level authoring… > Recorded Room regression route**.

1. Choose **Prepare last Room route**. This checks the host's existing stopped
   recording against the current saved project. It does not start recording,
   advance gameplay, save the project or write a script file.
2. Choose **Copy Room route JSON**. Paste the clipboard text into a new JSON
   script file, then run `orr_playtest_room --project PROJECT --script FILE --json`
   with the existing `room-playtest` feature. Prepare is consumed by Copy;
   **Cancel Room route** discards the prepared text without changing the game.

This bridge supports static-model projects without authored controls, with their
scene directly inside the project root. Projects with authored controls use the
separate [schema-2 key-script workflow](room-playtest.md); recording cannot recover
which physical keys produced a logical input. Nested-scene and animated projects
are not supported by this bridge. No new feature, dependency, recorder, automatic
file writer or clipboard access happens before an explicit action.

The copied schema-1 script groups consecutive identical logical inputs and uses
key/win states observed at each input-run endpoint by replaying the actual
simulation. Intermediate state transitions inside a run are not separate script
assertions; checksum verification still checks every recorded tick. Held interactions and
release ticks are preserved. A failing route can also be exported: observed false
victory is a regression expectation, not a claim that the puzzle is solvable.
Editing geometry later may intentionally make the saved assertions fail.

Admission requires 1–6000 contiguous input ticks, at most 256 input runs, a
recording no larger than 256 KiB and a declared decoded body no larger than 16 MiB.
The game/build/seed/player count/input format must match. The initial recorded
Frame must equal the admitted scene Frame; every tick checksum and the stop
receipt must match. Missing or malformed input, simulation commands and debug
commands anywhere in the recording are rejected. The original recording remains
untouched on success or failure. This proves simulation identity, not that the
recording contained the original YAML text or captured hardware key events.

Prepare checks canonical saved/current documents, including names and GUIDs.
Copy rechecks the exact captured source bytes, saved project admission, editor
path/load epoch/checksum and original stopped recording. Closing the panel,
restarting Play, changing the scene or losing eligibility invalidates prepared
output. Return to idle Edit and prepare again. No automatic retry is performed.

Validation is CPU replay and headless widget behavior. It is not native-window,
physical-input, GPU, browser, cross-platform or real-model creation acceptance.
Existing aggregate failures and required integration checks remain separate.
