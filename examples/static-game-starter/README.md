# Stage-one static game starter

Two genuine external games use one immutable `StaticGameRegistry`, the single
`EditorGame::Registered` route and application-owned typed hosts/bridges. Adding
a third game changes this application, not an engine game enum. No VM, dynamic
library loader, discovery, generated art, tiles, model pipeline or packaging.

From the repository root, using a supported Rust toolchain:

```sh
cargo run -p orr_static_starter --bin editor -- delivery examples/static-game-starter/scenes/delivery.scene.yaml
cargo run -p orr_static_starter --bin editor -- switchboard examples/static-game-starter/scenes/switchboard.scene.yaml
cargo run -p orr_static_starter --bin player -- delivery examples/static-game-starter/scenes/delivery.scene.yaml
cargo run -p orr_static_starter --bin player -- delivery examples/static-game-starter/scenes/delivery.scene.yaml trace.json
cargo test -p orr_static_starter --test acceptance
```

The editor starts in edit mode. Inspect/drag the courier or change Delivery.goal;
Save As preserves this registration. Enter Play and use the editor's focused
keyboard-control mode: WASD/arrows moves one unit per tick. Space at the yellow
goal wins (green courier). Release then press Space to restart. The second game
uses a fixed-point selection input and a four-bit Board singleton instead of
courier motion: left/right selects a switch, Space toggles, matching target wins.
Holding directions repeats each tick; Space is edge-triggered. Default target 5
means switches zero and two. Edit Board.target before Play to change the puzzle.

The source `player` binary is deliberately HEADLESS, not a native
standalone game or packaged export. Both it and the editor host load exactly the
same scene through `document<G>`, initial-state admission, seed 42, tick rate 60
and explicit build identity. Acceptance tests drive non-idle inputs through ERP,
compare every sampled checksum with the source PlaySession, and verify replay,
seek, restart and rejection of mismatched contracts. They test editor lifecycle
logic without a window: they do not establish rendered frames, keyboard focus,
native GUI acceptance or packaged export. Those remain separate gates.

All simulation uses fixed-point/integer state. Views receive owned snapshots and
only read FrameView. Input arrives through the existing ERP managed-input path;
this integration supplies no alternate input queue or simulation authority.
Host auth, listener, work-status and scene services are retained. Unknown game
names fail closed. Admission rejects malformed runtime state; play debug edits
are disabled. Restart resets gameplay, while host restart reloads the saved
scene. Build IDs must change when gameplay or input layout changes.

Optional trace.json is a finite JSON array of steps, each with exactly `ticks`
and `input`. Example delivery win:

```json
[{"ticks":4,"input":{"dx":1,"dy":0,"deliver":0}},
 {"ticks":1,"input":{"dx":0,"dy":0,"deliver":1}}]
```

The complete trace is validated before running: maximum 64 KiB, 64 steps and
600 total ticks; positive integer tick counts; exact reflected fields; finite
held-key input domain only. No scripts or arbitrary commands. Without a trace,
the player runs 60 idle ticks. Acceptance launches the actual headless player binary for both games from a
scene-only temporary working directory and compares nonzero win traces and idle
checksums. The editor is exercised through its CPU library APIs, not launched as
a native window by these tests. This proves working-directory relocation, not source-hidden packaging.
