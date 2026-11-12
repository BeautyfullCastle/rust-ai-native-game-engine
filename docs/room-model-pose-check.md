# Check saved Room models before launching

Room creators can check saved static placements and sampled animations without
opening a window or selecting a GPU adapter:

```sh
room_escape --project ./my-room --headless --ticks 120 --check-model-poses
./deployed-room/run-room-escape --headless --ticks 120 --check-model-poses
```

The optional flag checks tick zero and every simulated tick (0 to 6000). It uses
the same CPU placement, local/world transform, selected clip and skinning
validation as the native Room renderer. Character builds retain their normal
playback cursor. Every checked pose must succeed; an error reports its tick and
returns a failing exit status. The final report gives the number of checked
frames and visible static/animated placements. Collected keys remain validated
but are excluded from visible counts.

Use the runtime built with your project's required capabilities. The exporter
still requires an explicitly trusted runtime and its SHA-256; this option does
not change that trust contract or the exporter's existing smoke check. An older
runtime without the option rejects it. Headless runs without the flag retain
their existing behavior. The check reads installed project/package data and
never saves or repairs authoring files.

Acceptance covers strict arguments, static visibility and stale GUID failures,
animated admission, and the exported launcher's option forwarding after hiding
the original project. These are CPU checks. They do not establish window,
GPU rendering, texture appearance, device audio or platform publication success.
