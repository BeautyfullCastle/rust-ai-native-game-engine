# Create a Room with authored lighting

`orr_new_lit_room` creates a new complete Linux Room project with a supplied light document already declared. The existing Room starter normally omits lighting, requiring manual manifest work. This path retains its scene, models, camera, verified CC0 package and deterministic initial Frame, adding the light before publication.

Build:

```sh
cargo build --locked -p orr_sample --features project-create,room-lighting --bin orr_new_lit_room
```

Provide a valid Room lighting document, for example:

```json
{"version":3,"point_light":{"position":[0,4,0],"color":[1,0.9,0.7],"intensity":3,"range":12},"follow_player":[1,2,-1],"activation":"key_missing"}
```

Then create a new destination under an existing directory:

```sh
target/debug/orr_new_lit_room --output /absolute/new-game --seed my-game --lighting /absolute/light.json
```

The seed uses the existing bounded ASCII starter contract. The tool copies validated light values, not a live reference to the input file. Fixed, carried, conditional and disabled documents use the existing strict versions1–3. The input is a bounded regular nonsymlink file; malformed, oversized and special inputs reject. No source script, compiler or downloaded content executes during creation. The template ID is `room-escape-lit-3d-v1`.

Open with a `room-lighting` enabled editor:

```sh
orr_editor --room-project /absolute/new-game
```

Room lighting controls can Apply, Undo/Redo, Save and reopen the generated sidecar without editing the manifest. Run with a `room-lighting` enabled `room_escape --project /absolute/new-game`. Export with the existing lighting-enabled `orr_export_room` and a trusted lighting-capable runtime. Consumers still require explicit lighting consent; a feature being compiled does not authorize every caller. The lighting source and creator are unnecessary once the complete project or exported bundle exists.

The supplied light is the existing single unshadowed imported-model light. This does not add GPU features, character animation, progress identity or gameplay rules. `key_missing` lights turn off on actual key pickup; Edit/Stop still previews the authored light.

Creation first uses the existing Room generator in an owned temporary directory. Only its unpublished manifest, lighting sidecar and README change. The complete saved project is admitted and its bounded byte closure rechecked before atomic Linux no-replace publication. Existing files, directories and symlinks at the output are refused, including a competing destination created just before publication. Failures clean only the owned stage. This assumes trusted local directories; parent inode replacement, hostile filesystem races and power-loss durability are not covered.

Focused acceptance checks reproducible creation, unchanged base gameplay/assets, explicit consumer consent, invalid paths/lights, interruption, tampering, competing publication, bounded FIFO-safe input, actual CLI options, Editor Apply/Save/reopen/pickup and a source-hidden exported headless game. These are CPU checks. Windows is an explicit unsupported CLI target here; its fallback is compile-checked, not positive creation. Existing Windows access failures remain unresolved. Native-window, GPU pixels, audio-device and video playtests require separate verification; a locked computer is not unlocked or bypassed.
