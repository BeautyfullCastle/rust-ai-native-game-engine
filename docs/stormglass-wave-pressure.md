# Native Stormglass wave pressure

After Start, the native HUD subtitle shows **INCOMING** (enemies not yet spawned
in this wave) and **ON ROUTE** (currently active enemies across both lanes).
The title screen retains its introduction. Victory and defeat show **CAMPAIGN
CLEARED** and **CORE LOST**, rather than suggesting more enemies will arrive.

Counts come only from the displayed simulation Frame: authored `WaveCounts`
pair count times two, minus `DefenseRun.spawned`, and the active enemy pool.
Dead/leaked inactive slots are excluded. Zero incoming does not mean the wave
has ended while enemies remain on route. This is neither a spawn countdown nor
a promise about when the next wave starts. The display adds no input target or
simulation/economy rule and does not mutate the Frame.

Pause keeps the same snapshot counts. Rewound/replaced snapshots and Restart
recompute immediately without a cached counter or wall clock. Invalid wave/phase
snapshots show unavailable status; excess spawn counters cannot underflow.

Focused CPU acceptance exercises authored/default counts, both lanes/inactive
slots, a real simulation spawn and Restart, terminal status, pause/rewind and
egui text output at 1200×940, 750×900 and 1800×800. It checks header geometry,
no-command output and unchanged source Frame tick/checksum. Existing native HUD
command tests and optional-controls builds remain covered. These are CPU text
shapes and simulation checks, not a raster screenshot, native window/GPU/device
playtest or video. Earlier blocked window tools are not retried.

```powershell
cargo test --locked -p orr_sample --features coop-defense --lib wave_pressure_tests:: -- --test-threads=1
cargo test --locked -p orr_sample --features coop-defense-controls --lib wave_pressure_tests:: -- --test-threads=1
```
