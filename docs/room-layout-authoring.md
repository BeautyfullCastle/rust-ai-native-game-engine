# Room player, key and exit placement

`room_layout` is an optional offline authoring example. It reads a saved,
admitted Room project and produces one operations file that moves its existing
PLAYER, KEY and EXIT actors together. Their GUIDs, Y coordinates, components,
names and model bindings are preserved. X/Z values use the engine's exact Q48.16
decimal parser.

Build and generate an operations file outside the project:

```powershell
cargo build --locked --release -p orr_remote --no-default-features --features room-project --example room_layout
.\target\release\examples\room_layout.exe --project C:\Games\MyRoom --player -2 -1 --key -1 -1 --exit 1 -1 --output C:\Games\layout.ops.json
```

All five options are required exactly once; coordinates are X then Z. The output
parent must already exist. Existing destinations and paths inside the project
are rejected. The example never applies edits, starts a server or saves the
project. Without `room-project` it fails before opening a project or output.

The three ordinary `orr_physics3d::Body.pos` patches are admitted together on a
private document through the existing atomic batch operation. Invalid coordinates
or initial player/wall contact reject the whole candidate. Admission does not
prove that a key or exit is reachable. Play the proposed layout to check that.

Open the same saved project in the editor with its ERP endpoint, then use the
existing verified proposal and save workflow:

```powershell
orr_editor --room-project C:\Games\MyRoom --erp 127.0.0.1:7777 --erp-dev
orr apply "place Room objectives" --ops-file C:\Games\layout.ops.json --idle 60 --check "entities.final == 7"
orr undo
orr redo
orr save --write
```

The seven-actor check applies to the stock Room starter only; use the actual
actor count for your project. The operations file is based on the saved project,
so save current editor edits before generating it. It carries no live-document
revision precondition. The live host's existing verification and admission remain
authoritative; regenerate after concurrent changes. Accepting all three patches
creates one undo entry. Scene save and model-sidecar save retain their existing
separate transactions.

## Verification

The CPU suite covers decimal/argument validation, saved-project preservation,
atomic edits, real ERP verification/adoption, Undo/Redo, save/reopen and acquiring
the moved key before reaching the moved exit:

```powershell
cargo test --locked --release -p orr_remote --no-default-features --features room-project --test room_layout -- --test-threads=1
```

The explicit GPU consumer invokes the actual built example and retains before/
after captures, generated operations and adapter evidence. Missing required
inputs fail the test; an ignored default run is not GPU acceptance:

```powershell
$env:ORR_REQUIRE_GPU = '1'
$env:ORR_ROOM_LAYOUT_BIN = (Resolve-Path .\target\release\examples\room_layout.exe).Path
$env:ORR_ROOM_LAYOUT_CAPTURE_DIR = 'C:\Evidence\new-room-layout-capture'
cargo test --locked --release -p orr_sample --features room-project --test room_layout_gpu -- --ignored --test-threads=1 --nocapture
```

The capture directory must be a new absolute path. Inspect the recorded adapter;
software-rendered output is not physical GPU coverage.

The production Room folder exporter currently supports Linux x86-64 hosts only.
This tool does not change that boundary. Local Windows CPU/GPU results cannot
establish source-hidden Linux export execution. Existing required hosted CI,
feature acceptance, review and development integration are separate statuses.
Neither this increment nor the original 22-issue roadmap count establishes full
editor creation, save, play and export completion for #94.
