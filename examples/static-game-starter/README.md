# Static game starter

Two genuine external games use one immutable `StaticGameRegistry`, the single
`EditorGame::Registered` route and application-owned typed hosts/bridges. Adding
a third game changes this application, not an engine game enum. No VM, dynamic
library loader, discovery, generated art, tiles or model pipeline. The original
headless/editor paths remain; concrete native players and one-target packaging
are described below.

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
only read FrameView. In editor play, input arrives through the existing ERP
managed-input path; this integration supplies no alternate input queue or simulation authority.
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

## Required hosted native-window acceptance

The separate `native-acceptance` feature selects the `native_window` integration
test. The existing hosted Linux gate builds a SHA-bound test executable, checks
its exact one-test inventory, and runs it under Xvfb with Mesa software Vulkan.
Both delivery and switchboard must produce successful receipts and four actual
EditorApp framebuffer PNGs (edit, won, neutral, reopened). Missing renderer,
display, source/run provenance, either game, input-release acknowledgment or
expected pixels is a failure, not a skipped success.

The fixture clicks actual viewport/Play/Take-control controls and supplies queued
egui raw key/focus events. Its non-rendering harness only locates toolbar widgets;
all evidence PNGs come from native eframe screenshot events. It checks separate
Escape/focus release, one neutral tick, Space restart, reopened identity and no
automatic re-claim. Compiled source SHA, executable hash, exact admitted contracts,
scene hashes, typed states and image hashes accompany the captures.

This proves the native App path on a virtual X11 display using software Vulkan.
It does not prove physical OS keyboard/focus delivery, a physical GPU, a native
standalone player, source-hidden deployment, or packaged export. Local compilation
and lint alone do not establish this acceptance; exact published-SHA hosted
receipts must pass the independent validator.

## Concrete native games and one-target bundles

`delivery-player` and `switchboard-player` are separate concrete windowed game
executables. They reuse the engine's `InProc` bridge, `PlaySession`, input map and
window renderer; they do not host the editor or its UI. Build these without the
package's default `editor` feature:

```sh
cargo run -p orr_static_starter --no-default-features --features standalone --bin delivery-player -- --scene examples/static-game-starter/scenes/delivery.scene.yaml
cargo run -p orr_static_starter --no-default-features --features standalone --bin switchboard-player -- --scene examples/static-game-starter/scenes/switchboard.scene.yaml
```

Both start paused. Enter explicitly starts/resumes; WASD/arrows move/select and
Space acts or restarts after a win. Escape or focus loss clears held input and
pauses without secretly advancing the simulation. Focus regain never resumes.
In live play, the first naturally consumed tick after explicit resume is neutral;
a fresh, non-repeat key press after that boundary is needed. Replay viewing
consumes the recorded inputs unchanged. Synthetic/repeated presses cannot
re-arm controls. The window title reports pause/win/control instructions.

`--record /absolute/new.orrp` saves the actual input recording on normal window
close. `--replay /absolute/existing.orrp` opens a read-only viewer. These options
are mutually exclusive. `--observe /absolute/new.ndjson` is a bounded, read-only
post-presentation diagnostic stream, not an input/control interface. Output files
must be new and outside the executable/bundle directory. Recordings are limited
to 4 MiB and 36,000 ticks. Admission requires canonical current engine-writer v3
ORRP, exact game/build/seed/rate/player/input layout, the admitted initial frame,
finite typed inputs, and independently verified keyframes/checksums. No command
or debug edits are accepted. Legacy/alternative ORRP encodings remain outside
this starter's policy, without changing the engine's raw replay format. Full
scene/input descriptors are bound by the admitted compiled game and bundle, not
embedded or authenticated by raw ORRP alone.

The Linux x86-64 `bundle` feature packages one already-trusted concrete runtime,
one admitted scene, a fixed launcher and a deterministic receipt. The selected
runtime must be compiled with `ORR_STATIC_STANDALONE_BUILD_SHA` set to the exact
reviewed 40-hex source revision. `--provenance` fails when that stamp is absent or
malformed; `--identity --scene PATH` queries full admission without a window.
Inputs must be regular single-link files; copy a Cargo-produced hard-linked
executable into a fresh staging path before packaging. The packager requires matching source stamp and executable SHA-256, then runs
bounded identity/provenance checks before no-clobber publication. A supplied
hash/stamp is a mix-up guard, not authentication of arbitrary executable code.

```sh
cargo run -p orr_static_starter --no-default-features --features bundle --bin bundle -- delivery --runtime /absolute/delivery-player --runtime-sha256 HEX64 --scene /absolute/delivery.scene.yaml --output /absolute/new-bundle --source-sha HEX40 --trusted-runtime
/absolute/new-bundle/run-game --record /absolute/new-recording.orrp
```

This is one Linux x86-64 target, not a fully static or universal Linux export.
The system ELF loader, glibc, display server and compatible graphics driver are
runtime prerequisites. A relocated source-hidden launch and actual X11 input /
framebuffer evidence are separate required native gates. CPU controller tests,
compilation or identity queries alone do not establish successful presentation,
source-hidden delivery, physical keyboard hardware or physical GPU acceptance.
